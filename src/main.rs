mod charts;
mod stats;
mod util;

use charts::write_output;
use clap::Parser;
use console::style;
use git2::build::CheckoutBuilder;
use git2::{Commit, ErrorCode, Sort};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::{debug, warn};
use rayon::prelude::*;
use rust_embed::Embed;
use stats::{CodeStats, GlobalStats, HistoricStats};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;
use util::{YearMonth, datetime_from_epoch_seconds};
use walkdir::WalkDir;
use util::MutexExt;
use util::OsStrExt;
use util::PathExt;

#[derive(Embed)]
#[folder = "templates"]
struct Asset;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    base_dir: String,

    #[arg(short, long, action)]
    suppress_progress: bool,

    #[arg(short, long, default_value = "out")]
    output_dir: PathBuf,

    #[arg(long)]
    skip_language: Vec<tokei::LanguageType>,
}

fn main() {
    env_logger::init();
    let args = Args::parse();
    let repos = collect_repositories(&args.base_dir);
    let start = SystemTime::now();
    let stats = get_historic_stats_in_repos(&args.base_dir, &repos, args.suppress_progress);
    let html_file = write_output(&args.output_dir, &stats, &args.skip_language);
    let time = style(format!("{:.2}s", start.elapsed().unwrap().as_secs_f32())).blue();
    let url = format!("file://{}", html_file.canonicalize().unwrap().to_str().expect("valid utf-8"));
    eprintln!("🏁 Counted {count} repositories in {time}. 🔗: {url}", count = repos.len());
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
) -> GlobalStats {
    let repositories = Mutex::new(HashMap::new());
    let multi_progress = MultiProgress::new();
    let counter = AtomicUsize::new(0);
    let total_steps = repo_paths.len();
    let max_step_width = format!("{}", total_steps).len();
    repo_paths.par_iter().for_each(|path| {
        let display_name = display_name(base_path, path);
        let bar = create_progress_bar(&multi_progress, &display_name);
        if !suppress_progress {
            bar.set_message("cloning");
        }
        let start = SystemTime::now();
        let stats = get_historic_stats(path, |perc, month| {
            if !suppress_progress {
                bar.set_position((perc * 100.0) as u64);
                bar.set_message(format!("counting {}", month));
            }
        });
        if !suppress_progress {
            bar.finish_and_clear();
            let step = counter.fetch_add(1, Ordering::Relaxed);
            let counter = style(format!("[{step1:max_step_width$}/{total_steps}]", step1 = step + 1)).dim();
            let time = style(format!("{time:7.2}s", time = start.elapsed().unwrap().as_secs_f32())).blue();
            bar.println(format!("{check} {display_name:45} {counter} {time}", check = style("✔").green(),));
        }

        let mut repositories = repositories.lock_or_panic();
        repositories.insert(display_name, stats);
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

fn create_progress_bar(multi_progress: &MultiProgress, display_name: &str) -> ProgressBar {
    let bar = multi_progress.add(ProgressBar::new(100));
    bar.set_prefix(display_name.to_owned());
    bar.set_style(
        ProgressStyle::with_template("{spinner:.green} {prefix:45} [{bar:45.cyan/blue}] {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar
}

fn get_historic_stats<F: Fn(f32, YearMonth)>(git_repo_path: &Path, update_reporter: F) -> HistoricStats {
    // Using a temporary directory for cloning the Git repository
    // A named directory (as opposed to an unnamed one or a simply fetching blobs from Git) is
    // needed because the library used for line counting, tokei, needs it.
    // `tempfile` will remove the directory when it's dropped at the end of this function.
    // An abnormal program termination will rely on the cleaning mechanism of the operating system.
    // Portability note: `tempfile` uses `tempfs` in Linux, which lives in memory. In MacOS it uses
    // the normal disk, which may be slightly slower but save some memory.
    let tmp_dir = tempfile::tempdir().unwrap();

    // cloning the repository (as opposed to something else like using a worktree or operating
    // directly) allows for 100% not touching it, even working without write permissions.
    debug!("cloning repo in {}", tmp_dir.path().to_str_or_panic());
    let repo = git2::Repository::clone(git_repo_path.to_str_or_panic(), tmp_dir.path()).unwrap();

    // inspecting all commit would be too slow and pointless for a slow-moving metric like lines of
    // code, taking the last commit of each period of time, currently the month.
    let samples: BTreeMap<YearMonth, Commit> = sample_commits(&repo);

    // actually count the lines
    get_stats_from_samples(&repo, &samples, update_reporter)
}

fn sample_commits(repo: &git2::Repository) -> BTreeMap<YearMonth, Commit<'_>> {
    let mut samples = BTreeMap::new();
    let mut revwalk = repo.revwalk().unwrap();

    // Only traverse the default branch
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

fn get_stats_from_samples<F: Fn(f32, YearMonth)>(
    repo: &git2::Repository,
    samples: &BTreeMap<YearMonth, Commit>,
    update_reporter: F,
) -> HistoricStats {
    let mut snapshots = BTreeMap::new();
    let total = samples.len();
    for (i, (&date, commit)) in samples.iter().enumerate() {
        debug!("checking out tree for commit {:?}", commit.id());

        match repo.checkout_tree(commit.tree().unwrap().as_object(), Some(CheckoutBuilder::new().force())) {
            Ok(_) => (),
            Err(err) if err.code() == ErrorCode::NotFound => {
                warn!("tree not found for commit {}, skipping", commit.id())
            }
            Err(err) => panic!("{err}"),
        };

        // count the lines for this commit
        let stats = CodeStats::generate(repo.workdir().unwrap());

        snapshots.insert(date, stats);

        let progress = (i + 1) as f32 / total as f32;
        update_reporter(progress, date);
    }
    HistoricStats { snapshots }
}
