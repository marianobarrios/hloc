use crate::Asset;
use crate::stats::{CodeStats, GlobalStats, LanguageStats};
use crate::util::YearMonth;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::{fs, io};

pub fn write_output(output_dir: &Path, stats: &GlobalStats) -> PathBuf {
    let by_repo_data = get_by_repo_chart(stats);
    let by_lang_data = get_by_language_chart(stats);

    match fs::remove_dir_all(output_dir) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => (),
        Err(err) => panic!("{}", err),
    }
    fs::create_dir(output_dir).unwrap();

    copy_file(output_dir, "chart.html");
    copy_file(output_dir, "chart.js");

    let by_repo_data = serde_json::to_string(&by_repo_data).unwrap();
    let by_lang_data = serde_json::to_string(&by_lang_data).unwrap();
    fs::write(
        output_dir.join("data.js"),
        format!("by_repo_data = {by_repo_data}; by_lang_data = {by_lang_data}"),
    )
    .unwrap();
    output_dir.join("chart.html")
}

fn copy_file<P: AsRef<Path>>(output_dir: P, file_name: &str) {
    let chart_html = Asset::get(file_name).unwrap();
    fs::write(output_dir.as_ref().join(file_name), chart_html.data).unwrap();
}

fn get_by_language_chart(global_stats: &GlobalStats) -> serde_json::Value {
    // pre-process data, grouting by language and filling gaps in commits
    let (min_month, max_month) = get_extreme_months(global_stats);
    let mut month_stats = BTreeMap::new();
    for month in gen_month_range(min_month, max_month) {
        month_stats.insert(month, CodeStats::zero());
        for historic_stats in global_stats.repositories.values() {
            let floor = historic_stats
                .snapshots
                .range(..=month)
                .last()
                .map(|(_, v)| v)
                .cloned()
                .unwrap_or(CodeStats::zero());
            *month_stats.get_mut(&month).unwrap() += floor;
        }
    }

    let mut rows = Vec::new();
    let languages = get_all_languages(global_stats);

    // header row
    let mut headers = vec![json!("Month")];
    for language in languages.iter() {
        headers.push(json!(language));
    }
    rows.push(json!(headers));

    // month rows
    for (month, stats) in month_stats {
        let mut row = vec![json!(month.to_string())];
        for language in languages.iter() {
            let lang_stats = stats.languages.get(language).unwrap_or(&LanguageStats::zero()).clone();
            row.push(json!(lang_stats.line_count));
        }
        rows.push(json!(row));
    }

    json!(rows)
}

fn get_by_repo_chart(global_stats: &GlobalStats) -> serde_json::Value {
    // pre-process data, grouting by repository and filling gaps in commits
    let (min_month, max_month) = get_extreme_months(global_stats);

    let mut month_stats = BTreeMap::new();
    for month in gen_month_range(min_month, max_month) {
        let mut repo_stats = BTreeMap::new();
        for repo in global_stats.repositories.keys() {
            repo_stats.insert(repo.clone(), 0);
        }
        month_stats.insert(month, repo_stats);
        for (repo, historic_stats) in global_stats.repositories.iter() {
            let floor = historic_stats
                .snapshots
                .range(..=month)
                .last()
                .map(|(_, v)| v)
                .cloned()
                .unwrap_or(CodeStats::zero());
            for stats in floor.languages.values() {
                *month_stats.get_mut(&month).unwrap().get_mut(repo).unwrap() += stats.line_count;
            }
        }
    }

    let mut rows = Vec::new();
    let repos = get_sorted_repos(global_stats);

    // header row
    let mut headers = vec![json!("Month")];
    for repo in repos.iter() {
        headers.push(json!(repo));
    }
    rows.push(json!(headers));

    // month rows
    for (month, stats) in month_stats {
        let mut row = vec![json!(month.to_string())];
        for repo in repos.iter() {
            let lang_stats = stats.get(repo).copied().unwrap_or(0);
            row.push(json!(lang_stats));
        }
        rows.push(json!(row));
    }

    json!(rows)
}

fn gen_month_range(from: YearMonth, to: YearMonth) -> Vec<YearMonth> {
    let mut months = Vec::new();
    for year in from.year..=to.year {
        let min_month = if year == from.year { from.month } else { 1 };
        let max_month = if year == to.year { to.month } else { 12 };
        for month in min_month..=max_month {
            months.push(YearMonth { year, month });
        }
    }
    months
}

fn get_extreme_months(global_stats: &GlobalStats) -> (YearMonth, YearMonth) {
    let months: Vec<_> =
        global_stats.repositories.values().flat_map(|s| s.snapshots.iter()).map(|s| s.0).cloned().collect();
    let min = months.iter().min().unwrap();
    let max = months.iter().max().unwrap();
    (*min, *max)
}

/// Returns all languages present in the stats, sorted by decreasing popularity (using last commit)
fn get_all_languages(global_stats: &GlobalStats) -> Vec<tokei::LanguageType> {
    let mut language_map = HashMap::new();
    for historic_stats in global_stats.repositories.values() {
        let last_commit = historic_stats.snapshots.values().last().unwrap();
        for (&language, lang_stats) in last_commit.languages.iter() {
            let count = language_map.entry(language).or_insert(0);
            *count += lang_stats.line_count;
        }
    }
    let mut languages: Vec<_> = language_map.keys().cloned().collect();
    languages.sort_by(|a, b| language_map[a].cmp(&language_map[b]));
    languages
}

/// Returns the repositories present in the stats, sorted by decreasing size (using last commit)
fn get_sorted_repos(global_stats: &GlobalStats) -> Vec<String> {
    let mut repo_map = HashMap::new();
    for (repo, historic_stats) in global_stats.repositories.iter() {
        let last_commit = historic_stats.snapshots.values().last().unwrap();
        for lang_stats in last_commit.languages.values() {
            let count = repo_map.entry(repo.clone()).or_insert(0);
            *count += lang_stats.line_count;
        }
    }
    let mut repos: Vec<_> = repo_map.keys().cloned().collect();
    repos.sort_by(|a, b| repo_map[a].cmp(&repo_map[b]));
    repos
}
