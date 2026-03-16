use crate::config::RepoConfig;
use crate::display_name;
use crate::git::{BlobId, CommitId};
use crate::history_trie::HistoryTrie;
use crate::languages;
use crate::stats::{CodeStats, GlobalStats, HistoricStats};
use crate::util::{MutexExt, PathExt, datetime_from_epoch_seconds};
use crate::year_month::YearMonth;
use anyhow::Context;
use chrono::Utc;
use console::style;
use git2::{ObjectType, Sort, TreeWalkMode, TreeWalkResult};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use linked_hash_set::LinkedHashSet;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// relying on the fact that Git oid are stable across commits if the file is identical
// to avoid counting lines in the same file more than once
type StatsCache = HashMap<BlobId, Option<(tokei::LanguageType, usize)>>;

pub fn get_stats_from_repos(
    base_path: &Path,
    repos_with_config: &HashMap<PathBuf, RepoConfig>,
    detect_forks: bool,
    suppress_progress: bool,
) -> anyhow::Result<(GlobalStats, YearMonth, YearMonth)> {
    // count
    let mut stats = get_stats_in_repos_impl(base_path, repos_with_config, detect_forks, suppress_progress)?;

    // post-processing
    let min_month = stats
        .repositories
        .values()
        .flat_map(|s| s.snapshots.keys().copied())
        .min()
        .expect("there should be at least one month");
    let this_month = YearMonth::from_datelike(Utc::now());
    fill_gaps(&mut stats, repos_with_config, min_month, this_month);
    remove_min_lines_repos(&mut stats, repos_with_config);

    Ok((stats, min_month, this_month))
}

fn get_stats_in_repos_impl(
    base_path: &Path,
    repos_with_config: &HashMap<PathBuf, RepoConfig>,
    detect_forks: bool,
    suppress_progress: bool,
) -> anyhow::Result<GlobalStats> {
    let filtered_repos: HashMap<_, _> =
        repos_with_config.iter().filter(|&(_, config)| !config.ignore).collect();

    let total_repos = filtered_repos.len();
    let max_step_width = format!("{total_repos}").len();

    let finished_repos = AtomicUsize::new(0);
    let total_stats = Mutex::new(HashMap::new());

    // The set of the repositories that are currently being counted, used to display. It is a linked
    // set to preserve insertion order, in turn to make the list as stable as possible.
    let currently_counting = Mutex::new(LinkedHashSet::new());

    let bar = create_progress_bar(suppress_progress);

    // inspecting all commit would be too slow and pointless for a slow-moving metric like lines of
    // code, taking the last commit of each period of time, currently the month.
    bar.set_position(1);
    bar.set_message("sampling commits");
    let mut samples = sample_all_commits(base_path, &filtered_repos);

    if detect_forks {
        let priorities: HashMap<_, _> = repos_with_config
            .iter()
            .map(|(repo, conf)| (repo.clone(), conf.fork_priority.unwrap_or(0)))
            .collect();
        remove_commits_from_forks(&mut samples, &priorities);
    }

    let total_samples: usize = samples.values().map(|x| x.len()).sum();
    bar.set_length(total_samples as u64);

    // The first level of concurrency is by repository
    filtered_repos.par_iter().for_each(|(&path, &config)| {
        let display_name = display_name(base_path, path);

        add_current_repo(&mut currently_counting.lock_or_panic(), &bar, &display_name);

        let stats = get_stats_from_samples(base_path, path, &samples[path], &config.skip_languages, {
            let bar = bar.clone();
            move || bar.inc(1)
        });

        total_stats.lock_or_panic().insert(path.to_owned(), stats);

        let finished_repos = finished_repos.fetch_add(1, Ordering::SeqCst) + 1;
        remove_current_repo(&mut currently_counting.lock_or_panic(), &bar, &display_name);

        let counter = style(format!("[{finished_repos:max_step_width$}/{total_repos}]")).dim();
        bar.println(format!("{counter} {}", display_name.display()));
    });

    bar.finish_and_clear();

    let total_stats = total_stats.lock_or_panic();
    Ok(GlobalStats { repositories: total_stats.clone() })
}

/// Detects forks of project to avoid double counting.
/// Forked project share identical histories until the forking point. Those commits have identical
/// IDs and can be identified. This function detects such shared histories, removes them from all
/// involved repositories except one (chosen alphabetically).
fn remove_commits_from_forks(
    samples: &mut HashMap<PathBuf, BTreeMap<YearMonth, CommitId>>,
    priorities: &HashMap<PathBuf, i32>,
) {
    let mut history_trie = HistoryTrie::default();
    for (repo, commit_map) in samples.iter() {
        let commits: Vec<_> = commit_map.values().copied().collect();
        let priority = priorities[repo];
        history_trie.insert(repo, priority, &commits).unwrap();
    }

    let result = history_trie.get_all_sequences();

    for (repo, repo_samples) in samples.iter_mut() {
        let remaining_commits: HashSet<CommitId> = HashSet::from_iter(result[repo].clone().into_iter());
        repo_samples.retain(|_, commit| remaining_commits.contains(commit));
    }
}

fn sample_all_commits(
    base_path: &Path,
    filtered_repos: &HashMap<&PathBuf, &RepoConfig>,
) -> HashMap<PathBuf, BTreeMap<YearMonth, CommitId>> {
    let samples = Mutex::new(HashMap::new());
    filtered_repos.par_iter().for_each(|(&repo_path, &repo_config)| {
        let repo = git2::Repository::open(base_path.join(repo_path).to_str_or_panic())
            .with_context(|| format!("cannot open Git repository at {}", repo_path.display()))
            .unwrap();

        let repo_samples: BTreeMap<YearMonth, CommitId> = sample_commits(&repo, repo_config);
        samples.lock_or_panic().insert(repo_path.clone(), repo_samples);
    });
    samples.lock_or_panic().clone()
}

fn add_current_repo(currently_counting: &mut LinkedHashSet<PathBuf>, bar: &ProgressBar, name: &Path) {
    currently_counting.insert(name.to_owned());
    bar.set_message(list_of_current(currently_counting));
}

fn remove_current_repo(currently_counting: &mut LinkedHashSet<PathBuf>, bar: &ProgressBar, name: &Path) {
    currently_counting.remove(name);
    bar.set_message(list_of_current(currently_counting));
}

fn list_of_current(currently_counting: &LinkedHashSet<PathBuf>) -> String {
    currently_counting.iter().map(|p| p.to_str_or_panic()).collect::<Vec<_>>().join(", ")
}

fn create_progress_bar(suppress: bool) -> ProgressBar {
    // using a placeholder length, to be replaced by the actual number of commits to count
    let bar = ProgressBar::new(100);
    let template = "[{bar:45.cyan/blue}] {msg}";
    bar.set_style(ProgressStyle::with_template(template).unwrap().progress_chars("=> "));
    bar.set_draw_target(if suppress { ProgressDrawTarget::hidden() } else { ProgressDrawTarget::stderr() });
    bar
}

fn sample_commits(repo: &git2::Repository, config: &RepoConfig) -> BTreeMap<YearMonth, CommitId> {
    let mut samples = BTreeMap::new();
    let mut revwalk = repo.revwalk().unwrap();

    // Only traverse the original branch
    revwalk.simplify_first_parent().unwrap();

    // The default format is reversed chronological, reversing again for pure chronological
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();

    revwalk.push_head().unwrap();
    for oid in revwalk {
        let commit_id = CommitId::from_oid(oid.unwrap());
        let commit = commit_id.to_object(repo);
        let time = datetime_from_epoch_seconds(commit.time().seconds());
        let date_naive = time.date_naive();

        if let Some(from) = config.from_time
            && date_naive < from
        {
            continue;
        }

        // as we are iterating in chronological order, the last commit for the period will stay
        // in the map
        samples.insert(YearMonth::from_datelike(time), commit_id);
    }
    samples
}

fn get_stats_from_samples<F>(
    base_path: &Path,
    repo_path: &Path,
    samples: &BTreeMap<YearMonth, CommitId>,
    skip_languages: &[tokei::LanguageType],
    update_reporter: F,
) -> HistoricStats
where
    F: Fn() + Send + Sync,
{
    let snapshots = Arc::new(Mutex::new(BTreeMap::new()));
    let cache: Arc<Mutex<StatsCache>> = Arc::new(Mutex::new(HashMap::new()));
    let update_reporter = Arc::new(update_reporter);

    // The second level of concurrency (after parallelizing by repository) is by commit. This is
    // necessary for when a couple of repositories are much bigger than the rest, or when simply
    // analyzing only one.
    rayon::scope(|s| {
        for (&date, &commit_id) in samples {
            s.spawn({
                let snapshots = snapshots.clone();
                let cache = cache.clone();
                let update_reporter = update_reporter.clone();
                move |_| {
                    let stats =
                        get_stats_from_commit(base_path, repo_path, commit_id, skip_languages, &cache);
                    snapshots.lock_or_panic().insert(date, stats);

                    // a commit was finished, ping the reported to increase the progress
                    update_reporter();
                }
            });
        }
    });
    HistoricStats { snapshots: snapshots.lock_or_panic().clone() }
}

fn get_stats_from_commit(
    base_path: &Path,
    repo_path: &Path,
    commit_id: CommitId,
    skip_languages: &[tokei::LanguageType],
    cache: &Mutex<StatsCache>,
) -> CodeStats {
    // Opening the repository independently for each commit is the most natural way to access
    // a Git repository concurrently in Rust (read only).
    let repo = git2::Repository::open(base_path.join(repo_path)).unwrap();

    let commit = commit_id.to_object(&repo);
    let tree = commit.tree().unwrap();
    let mut languages = HashMap::new();
    tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        // only process files, not other object types
        if let Some(ObjectType::Blob) = entry.kind() {
            let blob_id = BlobId::from_oid(entry.id());
            let file_name = Path::new(entry.name().unwrap());
            let result = count_lines(&repo, blob_id, file_name, skip_languages, cache);

            // merge result with the global count
            if let Some((language, lines)) = result {
                *languages.entry(language).or_insert(0) += lines;
            }
        }
        TreeWalkResult::Ok
    })
    .unwrap();
    CodeStats { languages }
}

fn count_lines(
    repo: &git2::Repository,
    blob_id: BlobId,
    file_name: &Path,
    skip_languages: &[tokei::LanguageType],
    cache: &Mutex<StatsCache>,
) -> Option<(tokei::LanguageType, usize)> {
    if let Some(existing) = cache.lock_or_panic().get(&blob_id).copied() {
        existing
    } else {
        let stats = count_lines_impl(repo, blob_id, file_name, skip_languages);
        cache.lock_or_panic().insert(blob_id, stats);
        stats
    }
}

fn count_lines_impl(
    repo: &git2::Repository,
    blob_id: BlobId,
    file_name: &Path,
    skip_languages: &[tokei::LanguageType],
) -> Option<(tokei::LanguageType, usize)> {
    if let Some(lang) = languages::detect_language(repo, blob_id, file_name)
        && !skip_languages.contains(&lang)
    {
        // this is the most expensive step with respect to Git, postponing it until it's really needed
        let blob = blob_id.to_object(repo);

        // actual count
        let stats = lang.parse_from_slice(blob.content(), &tokei::Config::default());

        Some((lang, stats.code))
    } else {
        None
    }
}

fn fill_gaps(
    stats: &mut GlobalStats,
    configs: &HashMap<PathBuf, RepoConfig>,
    min_month: YearMonth,
    this_month: YearMonth,
) {
    for (repo, historic_stats) in &mut stats.repositories {
        // Normally, this function will fill gaps at the end of the series until the present time
        // with the last known value, assuming a stale repository. However, if the repository is
        // marked as "archived" we take the last commit as the end.
        let max_month = if configs[repo].archived {
            *historic_stats.snapshots.keys().max().expect("there should be at least one commit")
        } else {
            this_month
        };

        for month in min_month.iter_to(max_month) {
            let floor =
                historic_stats.snapshots.range(..=month).last().map(|(_, v)| v).cloned().unwrap_or_default();
            historic_stats.snapshots.entry(month).or_insert(floor);
        }
    }
}

fn remove_min_lines_repos(stats: &mut GlobalStats, repos_with_config: &HashMap<PathBuf, RepoConfig>) {
    let mut empty_repos = Vec::new();
    for (repo, historic_stats) in &stats.repositories {
        let config = &repos_with_config[repo];
        if historic_stats
            .snapshots
            .values()
            .all(|code_stats| code_stats.languages.values().all(|&s| s < config.min_lines as usize))
        {
            empty_repos.push(repo.clone());
        }
    }
    for repo in empty_repos {
        stats.repositories.remove(&repo);
    }
}
