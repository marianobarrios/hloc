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
use crate::util::datetime_from_epoch_seconds;
use anyhow::{Context, bail};
use chrono::{Local, NaiveDate};
use clap::Parser;
use config::{Config, RepoConfig};
use console::style;
use git2::Sort;
use globset::{GlobBuilder, GlobMatcher};
use log::{debug, info};
use rayon::iter::IntoParallelRefIterator;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use time_period::{YearMonth, YearQuarter, YearWeek};
use util::PathExt;
use walkdir::WalkDir;

const MAX_PERIODS: usize = 200;

const CONFIG_HELP: &str = r#"Path to a TOML configuration file.

The file is a map of Unix glob patterns to repository settings:

  ["**/*"]
  min_lines = 5000
  skip_languages = ["Xml", "Json"]

  ["**/some-repo"]
  ignore = true

Available settings per pattern:
  ignore          (bool)        Exclude matching repositories entirely [default: false]
  skip_languages  ([string])    Languages to exclude from the line count [default: none]
  min_lines       (integer)     Minimum lines of code required for a repository to appear in the report [default: 1]
  from_time       (date)        Only count commits from this date onward (YYYY-MM-DD) [default: none]
  archived        (bool)        Treat matching repositories as archived. Archived repositories are assumed to finish at the last commit, as opposed to propagating until the current date [default: false]
  fork_priority   (integer)     Priority used during fork detection. When two repositories share commit history, the one with the lower value is treated as the original and keeps the shared commits; the other has those commits removed. Ties are broken alphabetically. [default: 0]

Multiple patterns can match a repository; settings are merged (ignore/archived are OR'd, min_lines takes the max, skip_languages are combined, fork_priority takes the min)."#;

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
        long_help = CONFIG_HELP
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

    #[arg(short, long, action, help = "Do not try to detect forks to avoid double counting")]
    no_fork_detection: bool,

    #[arg(long, help = "Print the resolved per-repository configuration and exit")]
    show_resolved_config: bool,

    #[arg(long, help = "Print the list of supported languages and exit")]
    languages: bool,
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

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let args = Args::parse();
    debug!("parsed arguments: {args:#?}");

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
    let repos: HashMap<_, _> =
        repo_paths.iter().map(|repo| (repo.to_owned(), configure_repo(repo, &parsed_config))).collect();
    if args.show_resolved_config {
        println!("{}", toml::to_string(&repos).expect("resolved config should be serializable"));
        return Ok(());
    }

    let repos = repos.into_iter().filter(|(_, config)| !config.ignore).collect();
    let base_dir = util::longest_common_subpath(&args.repo_dirs);
    let detect_forks = !args.no_fork_detection;

    let resolved_period = match args.period {
        PeriodArg::Auto => {
            let chosen = choose_period_automatically(&repos, &base_dir);
            info!("chosen period: {chosen:?}");
            chosen
        }
        explicit => explicit,
    };

    let start = Instant::now();
    let chart_path = match resolved_period {
        PeriodArg::Auto => unreachable!(),
        PeriodArg::Week => calculate_stats::<YearWeek>(
            &repos,
            &base_dir,
            detect_forks,
            args.suppress_progress,
            &args.output_dir,
        )?,
        PeriodArg::Month => calculate_stats::<YearMonth>(
            &repos,
            &base_dir,
            detect_forks,
            args.suppress_progress,
            &args.output_dir,
        )?,
        PeriodArg::Quarter => calculate_stats::<YearQuarter>(
            &repos,
            &base_dir,
            detect_forks,
            args.suppress_progress,
            &args.output_dir,
        )?,
    };
    let time = style(format!("{:.2}s", start.elapsed().as_secs_f32())).blue();

    let url = format!("file://{}", chart_path.canonicalize().expect("valid path").to_str_or_panic());
    eprintln!("🏁 Counted {count} repositories in {time}. 🔗: {url}", count = repo_paths.len());
    Ok(())
}

fn choose_period_automatically(filtered_repos: &HashMap<PathBuf, RepoConfig>, base_dir: &Path) -> PeriodArg {
    let earliest = find_earliest_commit_date(base_dir, filtered_repos);
    info!("doing automatic period selection, earliest commit date: {earliest}");
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
pub fn display_name(base_path: &Path, path: &Path) -> PathBuf {
    let rel = pathdiff::diff_paths(path, base_path).unwrap_or_else(|| path.to_owned());
    if rel.as_os_str().is_empty() { PathBuf::from(base_path.file_name().unwrap()) } else { rel }
}

/// Returns the date of the oldest commit across all non-ignored repositories, or the specified
/// `from_time` setting.
fn find_earliest_commit_date(base_path: &Path, repos: &HashMap<PathBuf, RepoConfig>) -> NaiveDate {
    repos
        .par_iter()
        .map(|(repo_path, config)| match config.from_time {
            Some(time) => time,
            None => earliest_commit_date(base_path, repo_path),
        })
        .min()
        .unwrap()
}

fn earliest_commit_date(base_path: &Path, repo_path: &PathBuf) -> NaiveDate {
    let repo = git2::Repository::open(base_path.join(repo_path).to_str_or_panic()).unwrap();
    let mut revwalk = repo.revwalk().unwrap();
    revwalk.simplify_first_parent().unwrap();
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();
    revwalk.push_head().unwrap();
    let first_commit = CommitId::from_oid(revwalk.next().unwrap().unwrap());
    datetime_from_epoch_seconds(first_commit.to_object(&repo).time().seconds()).date_naive()
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
) -> Result<PathBuf, anyhow::Error> {
    let (stats, min, max) =
        count::get_stats_from_repos::<P>(base_dir, repos, detect_forks, suppress_progress)?;
    let chart_path = charts::write_output(output_dir, base_dir, &stats, min, max)?;
    Ok(chart_path)
}
