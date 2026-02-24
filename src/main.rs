mod charts;
mod languages;
mod stats;
mod util;

use crate::languages::detect_language;
use charts::write_output;
use clap::Parser;
use console::style;
use git2::{Commit, ObjectType, Oid, Sort, TreeWalkMode, TreeWalkResult};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;
use rust_embed::Embed;
use stats::{CodeStats, GlobalStats, HistoricStats};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;
use util::MutexExt;
use util::OsStrExt;
use util::PathExt;
use util::{YearMonth, datetime_from_epoch_seconds};
use walkdir::WalkDir;

#[derive(Embed)]
#[folder = "templates"]
struct Asset;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    base_dir: String,

    #[arg(short, long, action)]
    suppress_progress: bool,

    #[arg(short, long, value_name = "DIRECTORY", default_value = "out")]
    output_dir: PathBuf,

    #[arg(short('l'), long, value_name = "LANGUAGE")]
    skip_language: Vec<String>,
}

fn main() {
    env_logger::init();
    let args = Args::parse();
    let skip_languages = match parse_skip_language(&args.skip_language) {
        Ok(languages) => languages,
        Err(err) => {
            eprintln!("Cannot parse language: {}", err);
            process::exit(1);
        }
    };
    let repos = collect_repositories(&args.base_dir);
    let start = SystemTime::now();
    let stats = get_historic_stats_in_repos(&args.base_dir, &repos, args.suppress_progress, &skip_languages);
    let html_file = write_output(&args.output_dir, &stats);
    let time = style(format!("{:.2}s", start.elapsed().unwrap().as_secs_f32())).blue();
    let url = format!("file://{}", html_file.canonicalize().unwrap().to_str_or_panic());
    eprintln!("🏁 Counted {count} repositories in {time}. 🔗: {url}", count = repos.len());
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

/// Checks whether the supplied path is a Git repo with at least one commit
fn is_git_repo<P: AsRef<Path>>(path: P) -> bool {
    match git2::Repository::open(path) {
        Ok(repo) => repo.head().is_ok(),
        Err(_) => false,
    }
}

fn get_historic_stats_in_repos(
    base_path: &str,
    repo_paths: &[PathBuf],
    suppress_progress: bool,
    skip_languages: &[tokei::LanguageType],
) -> GlobalStats {
    let repositories = Mutex::new(HashMap::new());
    let multi_progress = MultiProgress::new();
    let counter = AtomicUsize::new(1);
    let total_steps = repo_paths.len();

    // pre-calculate all display names to know in advance which is the longest one
    let display_names: HashMap<_, _> =
        repo_paths.iter().map(|p| (p.as_path(), display_name(base_path, p))).collect();
    let display_name_len = display_names.values().map(|s| s.len()).max().unwrap();

    let max_step_width = format!("{}", total_steps).len();
    repo_paths.par_iter().for_each(|path| {
        let display_name = &display_names[path.as_path()];
        let bar = create_progress_bar(&multi_progress, &display_name, display_name_len);
        let start = SystemTime::now();
        let stats = get_historic_stats(path, skip_languages, |perc, msg| {
            if !suppress_progress {
                bar.set_position((perc * 100.0) as u64);
                bar.set_message(msg.to_owned());
            }
        });
        if !suppress_progress {
            bar.finish_and_clear();
            let step = counter.fetch_add(1, Ordering::Relaxed);
            let counter = style(format!("[{step:max_step_width$}/{total_steps}]")).dim();
            let time = style(format!("{time:7.3}s", time = start.elapsed().unwrap().as_secs_f32())).blue();
            bar.println(format!(
                "{check} {display_name:display_name_len$} {counter} {time}",
                check = style("✔").green(),
            ));
        }

        let mut repositories = repositories.lock_or_panic();
        repositories.insert(display_name.clone(), stats);
    });
    let repositories = repositories.lock_or_panic();
    GlobalStats { repositories: repositories.clone() }
}

fn display_name(base_path: &str, path: &PathBuf) -> String {
    if path == base_path {
        path.file_name().unwrap().to_str_or_panic().to_owned()
    } else {
        path.strip_prefix(base_path).unwrap().to_str_or_panic().to_owned()
    }
}

fn create_progress_bar(
    multi_progress: &MultiProgress,
    display_name: &str,
    display_name_len: usize,
) -> ProgressBar {
    let bar = multi_progress.add(ProgressBar::new(100));
    bar.set_prefix(display_name.to_owned());
    let template = "{spinner:.green} {prefix:PREFIX_LENGTH} [{bar:40.cyan/blue}] {msg}"
        .replace("PREFIX_LENGTH", &display_name_len.to_string());
    bar.set_style(ProgressStyle::with_template(&template).unwrap().progress_chars("=> "));
    bar
}

fn get_historic_stats<F: Fn(f32, &str)>(
    git_repo_path: &Path,
    skip_languages: &[tokei::LanguageType],
    update_reporter: F,
) -> HistoricStats {
    update_reporter(0.0, "preparing");
    let repo = git2::Repository::open(git_repo_path.to_str_or_panic()).unwrap();

    // inspecting all commit would be too slow and pointless for a slow-moving metric like lines of
    // code, taking the last commit of each period of time, currently the month.
    let samples: BTreeMap<YearMonth, Commit> = sample_commits(&repo);

    // actually count the lines
    get_stats_from_samples(&repo, &samples, skip_languages, update_reporter)
}

fn sample_commits(repo: &git2::Repository) -> BTreeMap<YearMonth, Commit<'_>> {
    let mut samples = BTreeMap::new();
    let mut revwalk = repo.revwalk().unwrap();

    // Only traverse the original branch
    revwalk.simplify_first_parent().unwrap();

    // The default format is reversed chronological, reversing again for pure chronological
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();

    revwalk.push_head().unwrap();
    for oid in revwalk {
        let oid = oid.unwrap();
        let commit = repo.find_commit(oid).unwrap();
        let time = datetime_from_epoch_seconds(commit.time().seconds());

        // as we are iterating in chronological order, the last commit for the period will stay
        // in the map
        samples.insert(YearMonth::from_datetime(time), commit);
    }
    samples
}

fn get_stats_from_samples<F: Fn(f32, &str)>(
    repo: &git2::Repository,
    samples: &BTreeMap<YearMonth, Commit>,
    skip_languages: &[tokei::LanguageType],
    update_reporter: F,
) -> HistoricStats {
    let mut snapshots = BTreeMap::new();
    let total = samples.len();

    // relying on the fact that Git oid are stable across commits if the file is identical
    // to avoid counting lines in the same file more than once
    let mut cache: HashMap<Oid, Option<(tokei::LanguageType, usize)>> = HashMap::new();

    for (i, (&date, commit)) in samples.iter().enumerate() {
        snapshots.insert(date, get_status_from_commit(repo, commit, skip_languages, &mut cache));
        let progress = (i + 1) as f32 / total as f32;
        update_reporter(progress, &format!("counting {}", date));
    }
    HistoricStats { snapshots }
}

fn get_status_from_commit(
    repo: &git2::Repository,
    commit: &Commit,
    skip_languages: &[tokei::LanguageType],
    cache: &mut HashMap<Oid, Option<(tokei::LanguageType, usize)>>,
) -> CodeStats {
    let tree = commit.tree().unwrap();
    let mut languages = HashMap::new();
    tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        // only process files, not other object types
        if let Some(ObjectType::Blob) = entry.kind() {
            let blob = repo.find_blob(entry.id()).unwrap();
            let result = cache.entry(blob.id()).or_insert_with(|| {
                count_lines_in_file(entry.name().unwrap(), blob.content(), skip_languages)
            });

            if let Some((language, lines)) = *result {
                *languages.entry(language).or_insert(0) += lines;
            }
        }
        TreeWalkResult::Ok
    })
    .unwrap();
    CodeStats { languages }
}

fn count_lines_in_file(
    file_name: &str,
    file_content: &[u8],
    skip_languages: &[tokei::LanguageType],
) -> Option<(tokei::LanguageType, usize)> {
    if let Some(language) = detect_language(file_name, file_content)
        && !skip_languages.contains(&language)
    {
        let stats = language.parse_from_slice(file_content, &tokei::Config::default());
        Some((language, stats.code))
    } else {
        None
    }
}
