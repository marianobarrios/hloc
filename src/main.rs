mod charts;
mod config;
mod count;
mod languages;
mod stats;
mod util;

use anyhow::{Context, bail};
use clap::Parser;
use config::Config;
use console::style;
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
}

impl RepoParsedConfig {
    fn default() -> Self {
        Self { ignore: false, skip_languages: Vec::new() }
    }

    pub fn merge(mut self, other: &Self) -> Self {
        self.skip_languages.extend_from_slice(&other.skip_languages);
        Self { ignore: self.ignore || other.ignore, skip_languages: self.skip_languages }
    }
}

#[derive(Debug, clap::Parser)]
#[command(version, about, long_about = None)]
struct Args {
    base_dir: String,

    #[arg(short, long, action)]
    suppress_progress: bool,

    #[arg(short, long, value_name = "DIRECTORY", default_value = "out")]
    output_dir: PathBuf,

    #[arg(short, long, value_name = "CONFIG_FILE")]
    config: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();
    let parsed_config = match args.config {
        Some(config_file) => {
            let file = fs::read_to_string(&config_file)
                .with_context(|| format!("cannot read file {config_file:?}"))?;
            parse_config(&file).with_context(|| format!("cannot parse file {config_file:?}"))?
        }
        None => HashMap::new(),
    };
    let repos = collect_repositories(&args.base_dir);
    if repos.is_empty() {
        bail!("No Git repositories found in {}", args.base_dir);
    }
    let repos_with_config = apply_config(&repos, &parsed_config);
    let start = Instant::now();
    let (stats, min_month, max_month) =
        count::get_stats_from_repos(&args.base_dir, &repos_with_config, args.suppress_progress)?;
    let html_file = charts::write_output(&args.output_dir, &stats, min_month, max_month)?;
    let time = style(format!("{:.2}s", start.elapsed().as_secs_f32())).blue();
    let url = format!("file://{}", html_file.canonicalize().expect("valid path").to_str_or_panic());
    eprintln!("🏁 Counted {count} repositories in {time}. 🔗: {url}", count = repos_with_config.len());
    Ok(())
}

fn parse_config(file_contents: &str) -> anyhow::Result<HashMap<glob::Pattern, RepoParsedConfig>> {
    let config: Config = serde_yaml_ng::from_str(file_contents).with_context(|| "cannot parse YAML")?;
    let mut parsed_config: HashMap<glob::Pattern, RepoParsedConfig> = HashMap::new();
    for (unparsed_pattern, repo_config) in config.repositories.into_iter() {
        let pattern = glob::Pattern::new(&unparsed_pattern)
            .with_context(|| format!("cannot parse GLOB pattern \"{unparsed_pattern}\""))?;
        let skip_languages = match parse_skip_language(&repo_config.skip_languages) {
            Ok(languages) => languages,
            Err(err) => {
                eprintln!("Cannot parse language: {}", err);
                process::exit(1);
            }
        };
        parsed_config.insert(pattern, RepoParsedConfig { ignore: repo_config.ignore, skip_languages });
    }
    Ok(parsed_config)
}

/// Finds all Git repositories recursively.
fn collect_repositories<P: AsRef<Path>>(path: P) -> Vec<PathBuf> {
    // The iterator is created only for its side effects
    let mut repos = Vec::new();
    WalkDir::new(path)
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
    repos
}

fn apply_config(
    repos: &[PathBuf],
    config: &HashMap<glob::Pattern, RepoParsedConfig>,
) -> HashMap<PathBuf, RepoParsedConfig> {
    repos
        .iter()
        .map(|repo| {
            let (_, configs): (Vec<_>, Vec<_>) =
                config.iter().filter(|&(pattern, _)| pattern.matches_path(repo.as_path())).unzip();
            let merged_config =
                configs.into_iter().fold(RepoParsedConfig::default(), RepoParsedConfig::merge);
            (repo.to_owned(), merged_config)
        })
        .collect()
}

fn parse_skip_language(string_args: &Vec<String>) -> Result<Vec<tokei::LanguageType>, String> {
    let mut parsed_languages = Vec::new();
    for arg in string_args {
        match tokei::LanguageType::from_name(arg) {
            Some(language) => parsed_languages.push(language),
            None => return Err(format!("unknown language {}", arg).to_owned()),
        }
    }
    Ok(parsed_languages)
}

/// Checks whether the supplied path is a Git repo with at least one commit
fn is_git_repo<P: AsRef<Path>>(path: P) -> bool {
    match git2::Repository::open(path) {
        Ok(repo) => repo.head().is_ok(),
        Err(_) => false,
    }
}
