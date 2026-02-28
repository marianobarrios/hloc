use crate::stats::GlobalStats;
use crate::util::YearMonth;
use crate::{Asset, util};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::{fs, io};

pub fn write_output(
    output_dir: &Path,
    stats: &GlobalStats,
    min_month: YearMonth,
    max_month: YearMonth,
) -> PathBuf {
    let by_repo_data = get_by_repo_chart(stats, min_month, max_month);
    let by_lang_data = get_by_lang_chart(stats, min_month, max_month);

    match fs::remove_dir_all(output_dir) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => (),
        Err(err) => panic!("{}", err),
    }
    fs::create_dir(output_dir).unwrap();

    copy_file(output_dir, "chart.html");
    copy_file(output_dir, "chart.js");
    copy_file(output_dir, "chart.css");

    let by_repo_data = serde_json::to_string(&by_repo_data).unwrap();
    let by_lang_data = serde_json::to_string(&by_lang_data).unwrap();
    fs::write(
        output_dir.join("data.js"),
        format!("by_repo_data = {by_repo_data};\nby_lang_data = {by_lang_data};\n"),
    )
    .unwrap();
    output_dir.join("chart.html")
}

fn copy_file<P: AsRef<Path>>(output_dir: P, file_name: &str) {
    let chart_html = Asset::get(file_name).unwrap();
    fs::write(output_dir.as_ref().join(file_name), chart_html.data).unwrap();
}

fn get_by_repo_chart(stats: &GlobalStats, min_month: YearMonth, max_month: YearMonth) -> serde_json::Value {
    let x_labels: Vec<_> =
        util::gen_month_range(min_month, max_month).iter().map(|m| m.to_string()).collect();
    let dataset: Vec<_> = get_sorted_repos(stats)
        .iter()
        .map(|repo| {
            let historic_stats = &stats.repositories[repo];
            let monthly_data: Vec<_> = historic_stats
                .snapshots
                .values()
                .map(|month_stats| month_stats.languages.values().sum::<usize>())
                .collect();
            json!({
                "label": repo,
                "data": monthly_data,
                "borderWidth": 1,
                "fill": true,
            })
        })
        .collect();
    json!({
        "labels": x_labels,
        "datasets": dataset,
    })
}

fn get_by_lang_chart(stats: &GlobalStats, min_month: YearMonth, max_month: YearMonth) -> serde_json::Value {
    let x_labels: Vec<_> =
        util::gen_month_range(min_month, max_month).iter().map(|m| m.to_string()).collect();

    let all_languages = get_sorted_languages(stats);

    let mut per_lang_data = BTreeMap::new();
    for lang in all_languages.iter() {
        let mut monthly_data = BTreeMap::new();
        for repo_stats in stats.repositories.values() {
            for (&month, monthly_stats) in repo_stats.snapshots.iter() {
                let lang_stats = monthly_stats.languages.get(&lang).unwrap_or(&0);
                *monthly_data.entry(month).or_insert(0) += lang_stats;
            }
        }
        per_lang_data.insert(lang, monthly_data);
    }

    let dataset: Vec<_> = all_languages
        .iter()
        .map(|lang| {
            let monthly_data: Vec<_> = per_lang_data[lang].values().collect();
            json!({
                "label": lang,
                "data": monthly_data,
                "borderWidth": 1,
                "fill": true,
            })
        })
        .collect();
    json!({
        "labels": x_labels,
        "datasets": dataset,
    })
}

/// Returns all languages present in the stats, sorted by decreasing popularity (using last commit)
fn get_sorted_languages(global_stats: &GlobalStats) -> Vec<tokei::LanguageType> {
    let mut language_map = HashMap::new();
    for historic_stats in global_stats.repositories.values() {
        let last_commit = historic_stats.snapshots.values().last().unwrap();
        for (language, line_count) in last_commit.languages.iter() {
            *language_map.entry(*language).or_insert(0) += line_count;
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
        for line_count in last_commit.languages.values() {
            *repo_map.entry(repo.clone()).or_insert(0) += line_count;
        }
    }
    let mut repos: Vec<_> = repo_map.keys().cloned().collect();
    repos.sort_by(|a, b| repo_map[a].cmp(&repo_map[b]));
    repos
}
