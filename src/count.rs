use crate::RepoParsedConfig;
use crate::languages;
use crate::stats::{CodeStats, GlobalStats, HistoricStats};
use crate::util::{MutexExt, PathExt, datetime_from_epoch_seconds};
use crate::year_month::YearMonth;
use anyhow::Context;
use console::style;
use git2::{ObjectType, Sort, TreeWalkMode, TreeWalkResult};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use linked_hash_set::LinkedHashSet;
use rayon::prelude::*;
use std::cmp;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// relying on the fact that Git oid are stable across commits if the file is identical
// to avoid counting lines in the same file more than once
type StatsCache = HashMap<git2::Oid, Option<(tokei::LanguageType, usize)>>;

pub fn get_stats_from_repos(
    base_path: &Path,
    repos_with_config: &HashMap<PathBuf, RepoParsedConfig>,
    suppress_progress: bool,
) -> anyhow::Result<(GlobalStats, YearMonth, YearMonth)> {
    let mut stats = get_stats_in_repos_impl(base_path, repos_with_config, suppress_progress)?;
    let (min_month, max_month) = fill_gaps(&mut stats, repos_with_config);
    Ok((stats, min_month, max_month))
}

fn get_stats_in_repos_impl(
    base_path: &Path,
    repos_with_config: &HashMap<PathBuf, RepoParsedConfig>,
    suppress_progress: bool,
) -> anyhow::Result<GlobalStats> {
    let filtered_repos: HashMap<_, _> =
        repos_with_config.iter().filter(|&(_, config)| !config.ignore).collect();

    let total_repos = filtered_repos.len();
    let max_step_width = format!("{total_repos}").len();

    let finished_repos = AtomicUsize::new(0);
    let total_stats = Mutex::new(HashMap::new());

    // The set of the repositories that are currently being count, used to display. It is a linked
    // set to preserve insertion order, in turn to make the list as stable as possible.
    let currently_counting = Mutex::new(LinkedHashSet::new());

    let bar = create_progress_bar(suppress_progress);

    // inspecting all commit would be too slow and pointless for a slow-moving metric like lines of
    // code, taking the last commit of each period of time, currently the month.
    bar.set_position(1);
    bar.set_message("sampling commits");
    let mut samples: HashMap<PathBuf, BTreeMap<YearMonth, git2::Oid>> = HashMap::new();
    for (&repo_path, &repo_config) in &filtered_repos {
        let repo = git2::Repository::open(base_path.join(repo_path).to_str_or_panic())
            .with_context(|| format!("cannot open Git repository at {}", repo_path.display()))?;

        let repo_samples: BTreeMap<YearMonth, git2::Oid> = sample_commits(&repo, repo_config);
        samples.insert(repo_path.clone(), repo_samples);
    }
    let total_samples: usize = samples.values().map(|x| x.len()).sum();
    bar.set_length(total_samples as u64);

    // The first level of concurrent is by repository
    filtered_repos.par_iter().for_each(|(&path, &config)| {
        add_current_repo(&mut currently_counting.lock_or_panic(), &bar, path);

        let stats = get_stats_from_samples(base_path, path, &samples[path], &config.skip_languages, {
            let bar = bar.clone();
            move || bar.inc(1)
        });

        total_stats.lock_or_panic().insert(path.to_owned(), stats);

        let finished_repos = finished_repos.fetch_add(1, Ordering::SeqCst) + 1;
        remove_current_repo(&mut currently_counting.lock_or_panic(), &bar, path);

        let counter = style(format!("[{finished_repos:max_step_width$}/{total_repos}]")).dim();
        bar.println(format!("{counter} {}", path.display()));
    });

    bar.finish_and_clear();

    let total_stats = total_stats.lock_or_panic();
    Ok(GlobalStats { repositories: total_stats.clone() })
}

fn add_current_repo<'r>(currently_counting: &mut LinkedHashSet<&'r Path>, bar: &ProgressBar, name: &'r Path) {
    currently_counting.insert(name);
    bar.set_message(list_of_current(currently_counting));
}

fn remove_current_repo(currently_counting: &mut LinkedHashSet<&Path>, bar: &ProgressBar, name: &Path) {
    currently_counting.remove(name);
    bar.set_message(list_of_current(currently_counting));
}

fn list_of_current(currently_counting: &LinkedHashSet<&Path>) -> String {
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

fn sample_commits(repo: &git2::Repository, config: &RepoParsedConfig) -> BTreeMap<YearMonth, git2::Oid> {
    let mut samples = BTreeMap::new();
    let mut revwalk = repo.revwalk().unwrap();

    // Only traverse the original branch
    revwalk.simplify_first_parent().unwrap();

    // The default format is reversed chronological, reversing again for pure chronological
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).unwrap();

    revwalk.push_head().unwrap();
    for commit_oid in revwalk {
        let commit_oid = commit_oid.unwrap();
        let commit = repo.find_commit(commit_oid).unwrap();
        let time = datetime_from_epoch_seconds(commit.time().seconds());
        let date_naive = time.date_naive();

        if let Some(from) = config.from
            && date_naive < from
        {
            continue;
        }

        if let Some(to) = config.to
            && date_naive > to
        {
            // the iteration is chronological, once we are past the top date, it's over
            break;
        }

        // as we are iterating in chronological order, the last commit for the period will stay
        // in the map
        samples.insert(YearMonth::from_datelike(time), commit_oid);
    }
    samples
}

fn get_stats_from_samples<F>(
    base_path: &Path,
    repo_path: &Path,
    samples: &BTreeMap<YearMonth, git2::Oid>,
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
        for (&date, &commit_oid) in samples {
            s.spawn({
                let snapshots = snapshots.clone();
                let cache = cache.clone();
                let update_reporter = update_reporter.clone();
                move |_| {
                    let stats =
                        get_stats_from_commit(base_path, repo_path, commit_oid, skip_languages, &cache);
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
    commit_oid: git2::Oid,
    skip_languages: &[tokei::LanguageType],
    cache: &Mutex<StatsCache>,
) -> CodeStats {
    // Opening the repository independently for each commit is the most natural way to access
    // a Git repository concurrently in Rust (read only).
    let repo = git2::Repository::open(base_path.join(repo_path)).unwrap();

    let commit = repo.find_commit(commit_oid).unwrap();
    let tree = commit.tree().unwrap();
    let mut languages = HashMap::new();
    tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        // only process files, not other object types
        if let Some(ObjectType::Blob) = entry.kind() {
            let blob_oid = entry.id();
            let file_name = Path::new(entry.name().unwrap());
            let result = count_lines(&repo, blob_oid, file_name, skip_languages, cache);

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
    blob_oid: git2::Oid,
    file_name: &Path,
    skip_languages: &[tokei::LanguageType],
    cache: &Mutex<StatsCache>,
) -> Option<(tokei::LanguageType, usize)> {
    if let Some(existing) = cache.lock_or_panic().get(&blob_oid).copied() {
        existing
    } else {
        let stats = count_lines_impl(repo, blob_oid, file_name, skip_languages);
        cache.lock_or_panic().insert(blob_oid, stats);
        stats
    }
}

fn count_lines_impl(
    repo: &git2::Repository,
    blob_oid: git2::Oid,
    file_name: &Path,
    skip_languages: &[tokei::LanguageType],
) -> Option<(tokei::LanguageType, usize)> {
    if let Some(lang) = languages::detect_language(repo, blob_oid, file_name)
        && !skip_languages.contains(&lang)
    {
        // this is the most expensive step with respect to Git, postponing it until it's really needed
        let blob = repo.find_blob(blob_oid).unwrap();

        // actual count
        let stats = lang.parse_from_slice(blob.content(), &tokei::Config::default());

        Some((lang, stats.code))
    } else {
        None
    }
}

fn fill_gaps(
    stats: &mut GlobalStats,
    configs: &HashMap<PathBuf, RepoParsedConfig>,
) -> (YearMonth, YearMonth) {
    let (min_month, max_month) = get_extreme_months(stats).expect("there should be at least one month");
    for (repo, historic_stats) in &mut stats.repositories {
        // Normally, this function will fill gaps at the end of the series with the last known
        // value, assuming a stale repository. However, a "to" date cut can create the same effect.
        // To prevent this to happen, be careful and iterate only until the appropriate date
        // Note: This is not necessary with the "from" date because data is always propagated from
        // the past to the future, and not the other way around.
        let effective_max_month = match configs[repo].to {
            Some(to) => cmp::min(max_month, YearMonth::from_datelike(to)),
            None => max_month,
        };

        for month in min_month.iter_to(effective_max_month) {
            let floor = historic_stats
                .snapshots
                .range(..=month)
                .last()
                .map(|(_, v)| v)
                .cloned()
                .unwrap_or(CodeStats::zero());
            historic_stats.snapshots.entry(month).or_insert(floor);
        }
    }
    (min_month, max_month)
}

fn get_extreme_months(global_stats: &GlobalStats) -> Option<(YearMonth, YearMonth)> {
    let months: Vec<_> =
        global_stats.repositories.values().flat_map(|s| s.snapshots.iter()).map(|s| s.0).copied().collect();
    if months.is_empty() {
        None
    } else {
        let min = months.iter().min().unwrap();
        let max = months.iter().max().unwrap();
        Some((*min, *max))
    }
}
