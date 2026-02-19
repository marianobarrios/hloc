mod charts;
mod stats;
mod util;

use crate::util::{YearMonth, datetime_from_epoch_seconds};
use charts::{get_by_language_chart, get_by_repo_chart};
use clap::Parser;
use console::style;
use git2::build::CheckoutBuilder;
use git2::{Commit, Sort};
use stats::{CodeStats, GlobalStats, HistoricStats};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::time::SystemTime;
use std::{fs, io, thread};
use walkdir::WalkDir;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    base_dir: String,

    /// Number of times to greet
    #[arg(short, long, default_value_t = 1)]
    count: u8,
}

fn main() {
    let args = Args::parse();

    let repos = collect_repositories(&args.base_dir);
    let stats = get_historic_stats_in_repos(&args.base_dir, &repos);

    let by_repo_data = get_by_repo_chart(&stats);
    fs::write(
        "by_repo_data.js",
        format!(
            "by_repo_data = {}",
            serde_json::to_string(&by_repo_data).unwrap()
        ),
    )
    .unwrap();

    let by_lang_data = get_by_language_chart(&stats);
    fs::write(
        "by_lang_data.js",
        format!(
            "by_lang_data = {}",
            serde_json::to_string(&by_lang_data).unwrap()
        ),
    )
    .unwrap();
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

fn get_historic_stats_in_repos<P1: AsRef<Path> + Sync, P2: AsRef<Path> + Sync + Send>(
    base_path: &P1,
    repo_paths: &[P2],
) -> GlobalStats {
    // TODO: Parallelize?

    let mut repositories = HashMap::new();
    for path in repo_paths {
        let suffix = path.as_ref().strip_prefix(base_path).unwrap();
        let (tx, rx) = mpsc::channel();

        let start = SystemTime::now();
        let join_handle = {
            let path = path.as_ref().to_owned();
            thread::spawn(|| get_historic_stats(path, tx))
        };
        for completed_month in rx.iter() {
            print!("\r  {:-100} {:7}", suffix.display(), completed_month);
            io::stdout().flush().unwrap();
        }
        let stats = join_handle.join().unwrap();
        println!(
            "\r{check} {msg:-100}{time:7.2}s",
            check = style("✔").green(),
            msg = suffix.display(),
            time = start.elapsed().unwrap().as_secs_f32()
        );
        repositories.insert(suffix.to_str().unwrap().to_owned(), stats);
    }
    GlobalStats { repositories }
}

fn get_historic_stats<P: AsRef<Path>>(git_repo_path: P, tx: Sender<YearMonth>) -> HistoricStats {
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
    let repo =
        git2::Repository::clone(git_repo_path.as_ref().to_str().unwrap(), tmp_dir.path()).unwrap();

    // inspecting all commit would be too slow and pointless for a slow-moving metric like lines of
    // code, taking the last commit of each period of time, currently the month.
    let samples: BTreeMap<YearMonth, Commit> = sample_commits(&repo);

    // actually count the lines
    get_stats_from_samples(&repo, &samples, tx)
}

fn sample_commits(repo: &git2::Repository) -> BTreeMap<YearMonth, Commit<'_>> {
    let mut samples = BTreeMap::new();
    let mut revwalk = repo.revwalk().unwrap();
    revwalk.set_sorting(Sort::TOPOLOGICAL).unwrap();
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
    tx: Sender<YearMonth>,
) -> HistoricStats {
    let mut snapshots = BTreeMap::new();
    for (&date, commit) in samples.iter() {
        let tree = commit.tree().unwrap();
        repo.checkout_tree(tree.as_object(), Some(CheckoutBuilder::new().force()))
            .unwrap();

        // count the lines for this commit
        let stats = CodeStats::generate(repo.workdir().unwrap());

        snapshots.insert(date, stats);
        tx.send(date).unwrap();
    }
    HistoricStats { snapshots }
}
