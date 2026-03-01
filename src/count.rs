use crate::languages;
use crate::stats::{CodeStats, GlobalStats, HistoricStats};
use crate::util::{MutexExt, OsStrExt, PathExt, YearMonth, datetime_from_epoch_seconds};
use crate::{RepoParsedConfig, util};
use console::style;
use git2::{Commit, ObjectType, Sort, TreeWalkMode, TreeWalkResult};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

pub fn get_stats_from_repos(
    base_path: &str,
    repos_with_config: &HashMap<PathBuf, RepoParsedConfig>,
    suppress_progress: bool,
) -> (GlobalStats, YearMonth, YearMonth) {
    let mut stats = get_stats_in_repos_impl(base_path, repos_with_config, suppress_progress);
    let (min_month, max_month) = fill_gaps(&mut stats);
    (stats, min_month, max_month)
}

fn get_stats_in_repos_impl(
    base_path: &str,
    repos_with_config: &HashMap<PathBuf, RepoParsedConfig>,
    suppress_progress: bool,
) -> GlobalStats {
    let filtered_repos: HashMap<_, _> =
        repos_with_config.iter().filter(|&(_, config)| !config.ignore).collect();

    let total_steps = filtered_repos.len();
    let max_step_width = format!("{}", total_steps).len();

    // pre-calculate all display names to know in advance which is the longest one
    let display_names: HashMap<_, _> =
        filtered_repos.keys().map(|p| (p, display_name(base_path, p))).collect();
    let display_name_len = display_names.values().map(|s| s.len()).max().unwrap();

    let multi_progress = MultiProgress::new();
    let counter = AtomicUsize::new(1);
    let total_stats = Mutex::new(HashMap::new());
    filtered_repos.par_iter().for_each(|(&path, &config)| {
        let display_name = &display_names[&path];
        let bar = if !suppress_progress {
            Some(create_progress_bar(&multi_progress, display_name, display_name_len))
        } else {
            None
        };
        let start = SystemTime::now();
        let stats = get_historic_stats(path, &config.skip_languages, |perc, msg| {
            if let Some(bar) = &bar {
                bar.set_position((perc * 100.0) as u64);
                bar.set_message(msg.to_owned());
            }
        });
        if let Some(bar) = &bar {
            bar.finish_and_clear();
            let step = counter.fetch_add(1, Ordering::Relaxed);
            let counter = style(format!("[{step:max_step_width$}/{total_steps}]")).dim();
            let time = style(format!("{time:7.3}s", time = start.elapsed().unwrap().as_secs_f32())).blue();
            bar.println(format!(
                "{check} {display_name:display_name_len$} {counter} {time}",
                check = style("✔").green(),
            ));
        }

        let mut total_stats = total_stats.lock_or_panic();
        total_stats.insert(display_name.clone(), stats);
    });
    let total_stats = total_stats.lock_or_panic();
    GlobalStats { repositories: total_stats.clone() }
}

fn display_name(base_path: &str, path: &Path) -> String {
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

fn get_historic_stats<F>(
    git_repo_path: &Path,
    skip_languages: &[tokei::LanguageType],
    update_reporter: F,
) -> HistoricStats
where
    F: Fn(f32, &str),
{
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

fn get_stats_from_samples<F>(
    repo: &git2::Repository,
    samples: &BTreeMap<YearMonth, Commit>,
    skip_languages: &[tokei::LanguageType],
    update_reporter: F,
) -> HistoricStats
where
    F: Fn(f32, &str),
{
    let mut snapshots = BTreeMap::new();
    let total = samples.len();

    // relying on the fact that Git oid are stable across commits if the file is identical
    // to avoid counting lines in the same file more than once
    let mut cache: HashMap<git2::Oid, Option<(tokei::LanguageType, usize)>> = HashMap::new();

    for (i, (&date, commit)) in samples.iter().enumerate() {
        snapshots.insert(date, get_stats_from_commit(repo, commit, skip_languages, &mut cache));
        let progress = (i + 1) as f32 / total as f32;
        update_reporter(progress, &format!("counting {}", date));
    }
    HistoricStats { snapshots }
}

fn get_stats_from_commit(
    repo: &git2::Repository,
    commit: &Commit,
    skip_languages: &[tokei::LanguageType],
    cache: &mut HashMap<git2::Oid, Option<(tokei::LanguageType, usize)>>,
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
    if let Some(lang) = languages::detect_language(file_name, file_content)
        && !skip_languages.contains(&lang)
    {
        let stats = lang.parse_from_slice(file_content, &tokei::Config::default());
        Some((lang, stats.code))
    } else {
        None
    }
}

fn fill_gaps(stats: &mut GlobalStats) -> (YearMonth, YearMonth) {
    let (min_month, max_month) = get_extreme_months(stats);
    for historic_stats in stats.repositories.values_mut() {
        for month in util::gen_month_range(min_month, max_month) {
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

fn get_extreme_months(global_stats: &GlobalStats) -> (YearMonth, YearMonth) {
    let months: Vec<_> =
        global_stats.repositories.values().flat_map(|s| s.snapshots.iter()).map(|s| s.0).cloned().collect();
    let min = months.iter().min().unwrap();
    let max = months.iter().max().unwrap();
    (*min, *max)
}
