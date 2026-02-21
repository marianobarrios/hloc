mod charts;
mod stats;
mod util;
use log::{debug, warn};

use crate::charts::write_output;
use crate::util::{YearMonth, datetime_from_epoch_seconds};
use clap::Parser;
use console::style;
use git2::build::CheckoutBuilder;
use git2::{Commit, ErrorCode, Sort};
use indicatif::{ProgressBar, ProgressStyle};
use rust_embed::Embed;
use stats::{CodeStats, GlobalStats, HistoricStats};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

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
}

fn main() {
    env_logger::init();
    let args = Args::parse();
    let repos = collect_repositories(&args.base_dir);
    let start = SystemTime::now();
    let stats = get_historic_stats_in_repos(&args.base_dir, &repos, args.suppress_progress);
    let html_file = write_output(&args.output_dir, &stats);
    println!(
        "Counted {} repositories in {:.2}s. Output: file://{}",
        repos.len(),
        start.elapsed().unwrap().as_secs_f32(),
        html_file.canonicalize().unwrap().to_str().unwrap()
    );
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
    // TODO: Parallelize?

    let mut repositories = HashMap::new();
    for (i, path) in repo_paths.iter().enumerate() {
        let suffix = if path == base_path {
            path.file_name().unwrap().to_str().unwrap()
        } else {
            path.strip_prefix(base_path).unwrap().to_str().unwrap()
        };
        let (tx, rx) = mpsc::channel();

        let bar = ProgressBar::new(100);
        bar.set_prefix(format_prefix(i, repo_paths.len(), suffix, false));
        bar.set_style(
            ProgressStyle::with_template("{spinner:.green} {prefix:45} [{bar:45.cyan/blue}] {msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        bar.enable_steady_tick(Duration::from_millis(100));

        if !suppress_progress {
            bar.set_message("cloning");
        }
        let start = SystemTime::now();
        let join_handle = {
            let path = path.to_owned();
            thread::spawn(move || get_historic_stats(&path, tx))
        };
        if !suppress_progress {
            for (percentage, completed_month) in rx.iter() {
                bar.set_position((percentage * 100.0) as u64);
                bar.set_message(format!("counting {}", completed_month));
            }
            bar.finish_and_clear();
        }
        let stats = join_handle.join().unwrap();
        if !suppress_progress {
            eprintln!(
                "{check} {prefix:101} {time:.2}s",
                check = style("✔").green(),
                prefix = format_prefix(i, repo_paths.len(), suffix, true),
                time = start.elapsed().unwrap().as_secs_f32()
            );
        }
        repositories.insert(suffix.to_owned(), stats);
    }
    GlobalStats { repositories }
}

fn format_prefix(step: usize, total_steps: usize, suffix: &str, dim_prefix: bool) -> String {
    let step1 = step + 1;
    let max_step_width = format!("{}", total_steps).len();
    let mut prefix = style(format!("[{step1:max_step_width$}/{total_steps}]"));
    if dim_prefix {
        prefix = prefix.dim();
    }
    format!("{prefix} {suffix}")
}

fn get_historic_stats(git_repo_path: &Path, tx: Sender<(f32, YearMonth)>) -> HistoricStats {
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
    debug!("cloning repo in {}", tmp_dir.path().to_str().unwrap());
    let repo = git2::Repository::clone(git_repo_path.to_str().unwrap(), tmp_dir.path()).unwrap();

    // inspecting all commit would be too slow and pointless for a slow-moving metric like lines of
    // code, taking the last commit of each period of time, currently the month.
    let samples: BTreeMap<YearMonth, Commit> = sample_commits(&repo);

    // actually count the lines
    get_stats_from_samples(&repo, &samples, tx)
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

fn get_stats_from_samples(
    repo: &git2::Repository,
    samples: &BTreeMap<YearMonth, Commit>,
    tx: Sender<(f32, YearMonth)>,
) -> HistoricStats {
    let mut snapshots = BTreeMap::new();
    let total = samples.len();
    for (i, (&date, commit)) in samples.iter().enumerate() {
        let tree = commit.tree().unwrap();

        debug!("checking out tree {:?}", tree.as_object());

        match repo.checkout_tree(tree.as_object(), Some(CheckoutBuilder::new().force())) {
            Ok(_) => (),
            Err(err) if err.code() == ErrorCode::NotFound => {
                warn!("tree not found for commit {}, skipping", commit.id())
            },
            Err(err) => panic!("{err}")
        };

        // count the lines for this commit
        let stats = CodeStats::generate(repo.workdir().unwrap());

        snapshots.insert(date, stats);

        let progress = (i + 1) as f32 / total as f32;
        tx.send((progress, date)).unwrap();
    }
    HistoricStats { snapshots }
}
