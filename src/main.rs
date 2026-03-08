mod charts;
mod config;
mod count;
mod languages;
mod stats;
mod util;
mod year_month;

use anyhow::{Context, bail};
use chrono::NaiveDate;
use clap::{Parser, command};
use config::Config;
use console::style;
use globset::{GlobBuilder, GlobMatcher};
use std::cmp;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use util::PathExt;
use walkdir::WalkDir;

#[derive(Debug, Clone, serde::Serialize)]
struct RepoParsedConfig {
    ignore: bool,
    skip_languages: Vec<tokei::LanguageType>,
    min_lines: u32,
    from: Option<NaiveDate>,
    archived: bool,
}

impl RepoParsedConfig {
    fn default() -> Self {
        Self { ignore: false, skip_languages: Vec::new(), min_lines: 1, from: None, archived: false }
    }

    pub fn merge(mut self, other: &Self) -> Self {
        self.skip_languages.extend_from_slice(&other.skip_languages);
        Self {
            ignore: self.ignore || other.ignore,
            skip_languages: self.skip_languages,
            min_lines: cmp::max(self.min_lines, other.min_lines),
            from: util::merge_options(self.from, other.from, cmp::max),
            archived: self.archived || other.archived,
        }
    }
}

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

Multiple patterns can match a repository; settings are merged (ignore/archived are OR'd, min_lines takes the max, skip_languages are combined)."#;

#[derive(Debug, clap::Parser)]
#[command(
    version,
    about = "Count lines of code across Git repositories over time",
    long_about = "Scans a directory tree for Git repositories and counts lines of code over their \
    history. The output is formatted in an interactive HTML report."
)]
struct Args {
    #[arg(
        help = "Base directory in which to search for repositories",
        required_unless_present = "languages"
    )]
    base_dir: Option<PathBuf>,

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

    #[arg(long, help = "Print the resolved per-repository configuration and exit")]
    show_resolved_config: bool,

    #[arg(long, help = "Print the list of supported languages and exit")]
    languages: bool,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();
    if args.languages {
        print_language_list();
        return Ok(());
    }
    let base_dir = args.base_dir.expect("base dir should be present if 'languages' was not specified");
    let parsed_config = match args.config {
        Some(config_file) => {
            let file = fs::read_to_string(&config_file)
                .with_context(|| format!("cannot read file \"{}\"", config_file.display()))?;
            parse_config(&file).with_context(|| format!("cannot parse file {}", config_file.display()))?
        }
        None => Vec::new(),
    };
    let repos = collect_repositories(&base_dir);
    if repos.is_empty() {
        bail!("No Git repositories found in {}", base_dir.display());
    }
    let repos_with_config =
        repos.iter().map(|repo| (repo.to_owned(), configure_repo(repo, &parsed_config))).collect();
    if args.show_resolved_config {
        println!("{}", toml::to_string(&repos_with_config).expect("resolved config should be serializable"));
        return Ok(());
    }
    let start = Instant::now();
    let (stats, min_month, max_month) =
        count::get_stats_from_repos(&base_dir, &repos_with_config, args.suppress_progress)?;
    let html_file = charts::write_output(&args.output_dir, &base_dir, &stats, min_month, max_month)?;
    let time = style(format!("{:.2}s", start.elapsed().as_secs_f32())).blue();
    let url = format!("file://{}", html_file.canonicalize().expect("valid path").to_str_or_panic());
    eprintln!("🏁 Counted {count} repositories in {time}. 🔗: {url}", count = repos_with_config.len());
    Ok(())
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

fn parse_config(file_contents: &str) -> anyhow::Result<Vec<(GlobMatcher, RepoParsedConfig)>> {
    let config: Config = toml::from_str(file_contents).with_context(|| "cannot parse TOML")?;
    let mut parsed_config = Vec::new();
    for (unparsed_pattern, repo_config) in config {
        let pattern = GlobBuilder::new(&unparsed_pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("cannot parse GLOB pattern \"{unparsed_pattern}\""))?
            .compile_matcher();
        let skip_languages = parse_skip_languages(&repo_config.skip_languages)
            .with_context(|| format!("cannot parse rules for \"{unparsed_pattern}\""))?;
        parsed_config.push((
            pattern,
            RepoParsedConfig {
                ignore: repo_config.ignore,
                skip_languages,
                min_lines: repo_config.min_lines,
                from: repo_config.from_time,
                archived: repo_config.archived,
            },
        ));
    }
    Ok(parsed_config)
}

/// Finds all Git repositories recursively.
fn collect_repositories(base_dir: &Path) -> Vec<PathBuf> {
    // The iterator is created only for its side effects
    let mut repos = Vec::new();
    WalkDir::new(base_dir)
        .into_iter()
        .filter_entry(|e| {
            if is_git_repo(e.path()) {
                let rel_path = pathdiff::diff_paths(e.path(), base_dir).unwrap();
                repos.push(rel_path);
                false // do not recurse inside repositories
            } else {
                true
            }
        })
        .for_each(|_| ()); // force iterator consumption
    repos
}

fn configure_repo(repo: &Path, config: &[(GlobMatcher, RepoParsedConfig)]) -> RepoParsedConfig {
    let (_, applicable_configs): (Vec<_>, Vec<_>) =
        config.iter().filter(|&(pattern, _)| pattern.is_match(repo)).cloned().unzip();
    applicable_configs.iter().fold(RepoParsedConfig::default(), RepoParsedConfig::merge)
}

fn parse_skip_languages(string_args: &Vec<String>) -> anyhow::Result<Vec<tokei::LanguageType>> {
    let mut parsed_languages = Vec::new();
    for arg in string_args {
        match tokei::LanguageType::from_name(arg) {
            Some(language) => parsed_languages.push(language),
            None => bail!(
                "unknown language \"{arg}\" (for a list of languages use \"{} --languages\")",
                command!().get_name()
            ),
        }
    }
    Ok(parsed_languages)
}

/// Checks whether the supplied path is a Git repo with at least one commit
fn is_git_repo(path: &Path) -> bool {
    match git2::Repository::open(path) {
        Ok(repo) => repo.head().is_ok(),
        Err(_) => false,
    }
}

/// The display name of a repository is in most cases its path relative to the base directory.
/// However, when there is only one repository in the base path itself, the specific path is just
/// the empty path, making reports confusing. In those cases we pick the last component of the base
/// path, which is the name of the directory of the repository itself.
pub fn display_name(base_path: &Path, path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() { PathBuf::from(base_path.file_name().unwrap()) } else { path.to_owned() }
}
