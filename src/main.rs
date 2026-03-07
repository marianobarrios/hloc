mod charts;
mod config;
mod count;
mod languages;
mod stats;
mod util;
mod year_month;

use anyhow::{Context, bail};
use chrono::NaiveDate;
use clap::Parser;
use config::Config;
use console::style;
use globset::{GlobBuilder, GlobMatcher};
use std::cmp;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::{fs, process};
use util::PathExt;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
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

#[derive(Debug, clap::Parser)]
#[command(version, about, long_about = None)]
struct Args {
    base_dir: PathBuf,

    #[arg(short, long, action)]
    suppress_progress: bool,

    #[arg(short, long, value_name = "DIRECTORY", default_value = "out")]
    output_dir: PathBuf,

    #[arg(short, long, value_name = "CONFIG_FILE")]
    config: Option<PathBuf>,

    #[arg(long, help = "Show resolved configuration and exit")]
    show_resolved_config: bool,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();
    let parsed_config = match args.config {
        Some(config_file) => {
            let file = fs::read_to_string(&config_file)
                .with_context(|| format!("cannot read file {}", config_file.display()))?;
            parse_config(&file).with_context(|| format!("cannot parse file {}", config_file.display()))?
        }
        None => Vec::new(),
    };
    let repos = collect_repositories(&args.base_dir);
    if repos.is_empty() {
        bail!("No Git repositories found in {}", args.base_dir.display());
    }
    let repos_with_config = apply_config(&repos, &parsed_config);
    if args.show_resolved_config {
        println!("{repos_with_config:#?}");
        return Ok(());
    }
    let start = Instant::now();
    let (stats, min_month, max_month) =
        count::get_stats_from_repos(&args.base_dir, &repos_with_config, args.suppress_progress)?;
    let html_file = charts::write_output(&args.output_dir, &stats, min_month, max_month)?;
    let time = style(format!("{:.2}s", start.elapsed().as_secs_f32())).blue();
    let url = format!("file://{}", html_file.canonicalize().expect("valid path").to_str_or_panic());
    eprintln!("🏁 Counted {count} repositories in {time}. 🔗: {url}", count = repos_with_config.len());
    Ok(())
}

fn parse_config(file_contents: &str) -> anyhow::Result<Vec<(GlobMatcher, RepoParsedConfig)>> {
    let config: Config = toml::from_str(file_contents).with_context(|| "cannot parse TOML")?;
    let mut parsed_config: Vec<(GlobMatcher, RepoParsedConfig)> = Vec::new();
    for (unparsed_pattern, repo_config) in config.repositories {
        let pattern = GlobBuilder::new(&unparsed_pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("cannot parse GLOB pattern \"{unparsed_pattern}\""))?
            .compile_matcher();
        let skip_languages = match parse_skip_language(&repo_config.skip_languages) {
            Ok(languages) => languages,
            Err(err) => {
                eprintln!("Cannot parse language: {err}");
                process::exit(1);
            }
        };
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

fn apply_config(
    repos: &[PathBuf],
    config: &[(GlobMatcher, RepoParsedConfig)],
) -> HashMap<PathBuf, RepoParsedConfig> {
    repos
        .iter()
        .map(|repo| {
            let applicable_config: Vec<_> =
                config.iter().filter(|&(pattern, _)| pattern.is_match(repo)).collect();
            let (_, applicable_config): (Vec<_>, Vec<_>) = applicable_config.into_iter().cloned().unzip();
            let merged_config =
                applicable_config.iter().fold(RepoParsedConfig::default(), RepoParsedConfig::merge);
            (repo.to_owned(), merged_config)
        })
        .collect()
}

fn parse_skip_language(string_args: &Vec<String>) -> Result<Vec<tokei::LanguageType>, String> {
    let mut parsed_languages = Vec::new();
    for arg in string_args {
        match tokei::LanguageType::from_name(arg) {
            Some(language) => parsed_languages.push(language),
            None => return Err(format!("unknown language {arg}").to_owned()),
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
