mod charts;
mod config;
mod count;
mod git;
mod history_trie;
mod languages;
mod stats;
mod time_period;
mod util;

use crate::git::CommitId;
use crate::time_period::TimePeriod;
use anyhow::{Context, bail};
use chrono::{DateTime, Local, NaiveDate};
use clap::Parser;
use config::{Config, RepoConfig};
use console::style;
use git2::Sort;
use globset::{GlobBuilder, GlobMatcher};
use rayon::iter::IntoParallelRefIterator;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use time_period::{YearMonth, YearQuarter, YearWeek};
use tracing::{debug, info, trace};
use util::PathExt;
use walkdir::WalkDir;

const MAX_PERIODS: usize = 200;

#[derive(Debug, clap::Parser)]
#[command(
    version,
    about = "Count lines of code across Git repositories over time",
    long_about = "Scans a directory tree for Git repositories and counts lines of code over their \
    history. The output is formatted in an interactive HTML report."
)]
struct Args {
    #[arg(
        help = "Directories in which to search for repositories",
        required_unless_present = "languages",
        num_args = 1..
    )]
    repo_dirs: Vec<PathBuf>,

    #[arg(short, long, action, help = "Do not print progress to stderr")]
    suppress_progress: bool,

    #[arg(
        long,
        value_name = "LEVEL",
        help = "Enable logging at the given severity threshold (info, debug, trace); implies --suppress-progress"
    )]
    log: Option<LogLevel>,

    #[arg(
        short,
        long,
        value_name = "DIRECTORY",
        default_value = "out",
        help = "Directory to write the HTML report to"
    )]
    output_dir: PathBuf,

    #[arg(
        short,
        long,
        value_name = "CONFIG_FILE",
        help = "TOML file controlling which repositories to include and additional configuration",
        long_help = config::HELP
    )]
    config: Option<PathBuf>,

    #[arg(
        short,
        long,
        value_name = "PERIOD",
        default_value = "auto",
        help = "Time granularity for sampling commits: auto (default), month, quarter, or week"
    )]
    period: PeriodArg,

    #[arg(
        short,
        long,
        value_name = "N",
        default_value_t = default_parallelism(),
        help = "Number of parallel threads (default: number of CPUs)"
    )]
    threads: usize,

    #[arg(short, long, action, help = "Do not try to detect forks to avoid double counting")]
    no_fork_detection: bool,

    #[arg(long, help = "Print the resolved per-repository configuration and exit")]
    show_resolved_config: bool,

    #[arg(long, help = "Print the list of supported languages and exit")]
    languages: bool,
}

/// Log severity threshold for the `--log` option.
#[derive(Debug, Copy, Clone, PartialEq, Eq, clap::ValueEnum)]
enum LogLevel {
    Info,
    Debug,
    Trace,
}

impl From<LogLevel> for tracing::Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Trace => tracing::Level::TRACE,
        }
    }
}

/// Controls the frequency of history sampling.
#[derive(Debug, Copy, Clone, PartialEq, Eq, clap::ValueEnum)]
pub enum PeriodArg {
    /// Pick the finest granularity that keeps the chart under 200 periods.
    Auto,
    Week,
    Month,
    Quarter,
}

impl Display for PeriodArg {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            PeriodArg::Week => "weekly",
            PeriodArg::Month => "monthly",
            PeriodArg::Quarter => "quarterly",
            PeriodArg::Auto => "auto",
        };
        write!(f, "{label}")
    }
}

fn default_parallelism() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let filter = match args.log {
        Some(level) => {
            let level: tracing::Level = level.into();
            // tokei logs too many warnings
            tracing_subscriber::EnvFilter::new(format!("{level},tokei=error"))
        }
        None => tracing_subscriber::EnvFilter::from_default_env(),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    debug!("parsed arguments: {args:#?}");

    rayon::ThreadPoolBuilder::new()
        .num_threads(args.threads)
        .build_global()
        .expect("failed to build thread pool");
    debug!("using {} parallel thread(s)", args.threads);

    if args.languages {
        print_language_list();
        return Ok(());
    }
    let parsed_config = match args.config {
        Some(config_file) => {
            let file = fs::read_to_string(&config_file)
                .with_context(|| format!("cannot read file \"{}\"", config_file.display()))?;
            parse_config(&file).with_context(|| format!("cannot parse file {}", config_file.display()))?
        }
        None => Vec::new(),
    };

    let repo_paths = collect_repositories(&args.repo_dirs);
    if repo_paths.is_empty() {
        let dirs =
            args.repo_dirs.iter().map(|d| format!("\"{}\"", d.display())).collect::<Vec<_>>().join(", ");
        bail!("No Git repositories found in {dirs}");
    }
    info!("found {} repositories", repo_paths.len());
    for repo in &repo_paths {
        trace!("{}", repo.display());
    }
    let repos: HashMap<_, _> =
        repo_paths.iter().map(|repo| (repo.to_owned(), configure_repo(repo, &parsed_config))).collect();
    if args.show_resolved_config {
        println!("{}", toml::to_string(&repos).expect("resolved config should be serializable"));
        return Ok(());
    }

    let repos: HashMap<_, _> = repos.into_iter().filter(|(_, config)| !config.ignore).collect();
    let excluded = repo_paths.len() - repos.len();
    if excluded > 0 {
        info!(
            "excluded {} {} (marked as ignored in config)",
            excluded,
            if excluded == 1 { "repository" } else { "repositories" }
        );
    }
    let detect_forks = !args.no_fork_detection;
    let no_progress = args.suppress_progress || args.log.is_some();

    let resolved_period = match args.period {
        PeriodArg::Auto => {
            let chosen_period = choose_period_automatically(&repos);
            info!("using {chosen_period} sampling (auto-selected)");
            chosen_period
        }
        explicit_period => {
            debug!("using {explicit_period} sampling (explicit)");
            explicit_period
        }
    };

    let base_dir = util::longest_common_subpath(&args.repo_dirs);

    let start = Instant::now();
    let chart_path = match resolved_period {
        PeriodArg::Auto => unreachable!(),
        PeriodArg::Week => {
            calculate_stats::<YearWeek>(&repos, &base_dir, detect_forks, no_progress, &args.output_dir)?
        }
        PeriodArg::Month => {
            calculate_stats::<YearMonth>(&repos, &base_dir, detect_forks, no_progress, &args.output_dir)?
        }
        PeriodArg::Quarter => {
            calculate_stats::<YearQuarter>(&repos, &base_dir, detect_forks, no_progress, &args.output_dir)?
        }
    };
    let time = style(format!("{:.2}s", start.elapsed().as_secs_f32())).blue();

    let url = format!("file://{}", chart_path.canonicalize().expect("valid path").to_str_or_panic());
    eprintln!("🏁 Counted {count} repositories in {time}. 🔗: {url}", count = repo_paths.len());
    Ok(())
}

fn choose_period_automatically(filtered_repos: &HashMap<PathBuf, RepoConfig>) -> PeriodArg {
    let earliest = find_earliest_commit_date(filtered_repos);
    debug!("auto period selection: earliest commit date {earliest}");
    let today = Local::now().date_naive();
    if period_count::<YearWeek>(earliest, today) <= MAX_PERIODS {
        PeriodArg::Week
    } else if period_count::<YearMonth>(earliest, today) <= MAX_PERIODS {
        PeriodArg::Month
    } else {
        PeriodArg::Quarter
    }
}

fn print_language_list() {
    let languages = tokei::LanguageType::list();
    let name_width = languages.iter().map(|(l, _)| l.name().len()).max().unwrap();
    let header = format!("{:<name_width$}  Extensions", "Language");
    println!("{}", style(header).bold());
    for &(language, extensions) in languages {
        println!(
            "{name:<name_width$}  {extensions}",
            name = language.name(),
            extensions = extensions.join(", ")
        );
    }
}

fn parse_config(file_contents: &str) -> anyhow::Result<Vec<(GlobMatcher, RepoConfig)>> {
    let config: Config = toml::from_str(file_contents)?;
    let mut parsed_config = Vec::new();
    for (unparsed_pattern, repo_config) in config {
        let pattern = GlobBuilder::new(&unparsed_pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("cannot parse GLOB pattern \"{unparsed_pattern}\""))?
            .compile_matcher();
        parsed_config.push((pattern, repo_config));
    }
    Ok(parsed_config)
}

/// Finds all Git repositories recursively under each of the given base directories.
/// Returns absolute paths.
fn collect_repositories(base_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    for base_dir in base_dirs {
        // The iterator is created only for its side effects
        WalkDir::new(base_dir)
            .into_iter()
            .filter_entry(|e| {
                if is_git_repo(e.path()) {
                    repos.push(e.path().to_owned());
                    false // do not recurse inside repositories
                } else {
                    true
                }
            })
            .for_each(|_| ()); // force iterator consumption
    }
    repos
}

fn configure_repo(repo: &Path, config: &[(GlobMatcher, RepoConfig)]) -> RepoConfig {
    config
        .iter()
        .filter(|(pattern, _)| pattern.is_match(repo))
        .map(|(_, repo_config)| repo_config)
        .fold(RepoConfig::default(), RepoConfig::merge)
}

/// Checks whether the supplied path is a Git repo with at least one commit
fn is_git_repo(path: &Path) -> bool {
    match git2::Repository::open(path) {
        Ok(repo) => repo.head().is_ok(),
        Err(_) => false,
    }
}

/// The display name of a repository is its path relative to the common ancestor of the base
/// directories. However, when there is only one repository and it is the base itself, the relative
/// path would be empty, making reports confusing. In that case we use the last component of the
/// base path, which is the name of the directory of the repository itself.
#[must_use]
pub fn display_name(base_path: &Path, path: &Path) -> PathBuf {
    let rel = pathdiff::diff_paths(path, base_path).unwrap_or_else(|| path.to_owned());
    if rel.as_os_str().is_empty() { PathBuf::from(base_path.file_name().unwrap()) } else { rel }
}

/// Returns the date of the oldest commit across all non-ignored repositories, or the specified
/// `from_time` setting.
fn find_earliest_commit_date(repos: &HashMap<PathBuf, RepoConfig>) -> NaiveDate {
    repos
        .par_iter()
        .map(|(repo_path, config)| match config.from_time {
            Some(time) => time,
            None => earliest_commit_date(repo_path),
        })
        .min()
        .unwrap()
}

fn earliest_commit_date(repo_path: &Path) -> NaiveDate {
    let repo = git2::Repository::open(repo_path.to_str_or_panic()).unwrap();
    let mut revwalk = repo.revwalk().unwrap();
    revwalk.simplify_first_parent().unwrap();
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();
    revwalk.push_head().unwrap();
    let first_commit_id = CommitId::from(revwalk.next().unwrap().unwrap());
    let first_commit = first_commit_id.into_object(&repo);
    let earliest_datetime =
        DateTime::from_timestamp(first_commit.time().seconds(), 0).expect("valid epoch seconds");
    earliest_datetime.date_naive()
}

/// Counts how many periods of type `P` fall between `start` and `today` (inclusive).
fn period_count<P: TimePeriod>(start: NaiveDate, today: NaiveDate) -> usize {
    P::from_datelike(start).iter_to(P::from_datelike(today)).count()
}

fn calculate_stats<P: TimePeriod>(
    repos: &HashMap<PathBuf, RepoConfig>,
    base_dir: &Path,
    detect_forks: bool,
    suppress_progress: bool,
    output_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let stats = count::get_stats_from_repos::<P>(base_dir, repos, detect_forks, suppress_progress);
    let chart_path = charts::write_output(output_dir, base_dir, &stats)?;
    info!("report written to {}", chart_path.display());
    Ok(chart_path)
}
