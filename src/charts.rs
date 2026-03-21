use crate::stats::Stats;
use crate::time_period::TimePeriod;
use crate::util::PathExt;
use crate::{display_name, util};
use anyhow::Context;
use rust_embed::Embed;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::{fs, io};

#[derive(Embed)]
#[folder = "templates"]
struct Asset;

pub fn write_output<P: TimePeriod>(
    output_dir: &Path,
    base_dir: &Path,
    stats: &Stats<P>,
) -> anyhow::Result<PathBuf> {
    let by_repo_data = get_by_repo_chart(base_dir, stats);
    let by_lang_data = get_by_lang_chart(stats);

    match fs::remove_dir_all(output_dir) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => (),
        Err(err) => panic!("{}", err),
    }
    fs::create_dir(output_dir)
        .with_context(|| format!("cannot create directory {}", output_dir.display()))?;

    copy_file_from_embedded(output_dir, "chart.html")?;
    copy_file_from_embedded(output_dir, "chart.js")?;
    copy_file_from_embedded(output_dir, "chart.css")?;

    let period_label = P::axis_label();
    let data_file = output_dir.join("data.js");
    fs::write(
        &data_file,
        format!("by_repo_data = {by_repo_data};\nby_lang_data = {by_lang_data};\nperiod_label = \"{period_label}\";\n"),
    )
    .with_context(|| format!("cannot write file {}", data_file.display()))?;
    Ok(output_dir.join("chart.html"))
}

fn copy_file_from_embedded(output_dir: &Path, file_name: &str) -> anyhow::Result<()> {
    let chart_html = Asset::get(file_name).unwrap();
    let file = output_dir.join(file_name);
    fs::write(&file, chart_html.data).with_context(|| format!("cannot write file {}", file.display()))?;
    Ok(())
}

fn get_by_repo_chart<P: TimePeriod>(base_dir: &Path, stats: &Stats<P>) -> serde_json::Value {
    let x_labels: Vec<_> = stats.from.iter_to(stats.to).map(|p| p.to_string()).collect();
    let dataset: Vec<_> = get_sorted_repos(stats)
        .iter()
        .map(|repo| {
            let historic_stats = &stats.repositories[repo];
            let period_data: Vec<_> = historic_stats
                .periods
                .values()
                .map(|period_stats| period_stats.languages.values().sum::<usize>())
                .collect();
            let label = util::truncate_beginning(display_name(base_dir, repo).to_str_or_panic(), 35, "...");
            json!({
                "label": label,
                "data": period_data,
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

fn get_by_lang_chart<P: TimePeriod>(stats: &Stats<P>) -> serde_json::Value {
    let x_labels: Vec<_> = stats.from.iter_to(stats.to).map(|p| p.to_string()).collect();

    let all_languages = get_sorted_languages(stats);

    let mut per_lang_data = BTreeMap::new();
    for lang in &all_languages {
        let mut period_data = BTreeMap::new();
        for repo_stats in stats.repositories.values() {
            for (&period, period_stats) in &repo_stats.periods {
                let lang_stats = period_stats.languages.get(lang).unwrap_or(&0);
                *period_data.entry(period).or_insert(0) += lang_stats;
            }
        }
        per_lang_data.insert(lang, period_data);
    }

    let dataset: Vec<_> = all_languages
        .iter()
        .map(|lang| {
            let period_data: Vec<_> = per_lang_data[lang].values().collect();
            json!({
                "label": lang,
                "data": period_data,
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

/// Returns all languages present in the stats, sorted by increasing popularity (using last commit)
fn get_sorted_languages<P>(global_stats: &Stats<P>) -> Vec<tokei::LanguageType> {
    let mut language_map = HashMap::new();
    for historic_stats in global_stats.repositories.values() {
        let last_commit =
            historic_stats.periods.values().last().expect("repository should have at least one commit");
        for (language, line_count) in &last_commit.languages {
            *language_map.entry(*language).or_insert(0) += line_count;
        }
    }
    let mut languages: Vec<_> = language_map.keys().copied().collect();
    languages.sort_by(|a, b| language_map[a].cmp(&language_map[b]));
    languages
}

/// Returns the repositories present in the stats, sorted by increasing size (using last commit)
fn get_sorted_repos<P>(global_stats: &Stats<P>) -> Vec<PathBuf> {
    let mut repo_map = HashMap::new();
    for (repo, historic_stats) in &global_stats.repositories {
        let last_commit =
            historic_stats.periods.values().last().expect("repository should have at least one commit");
        let total: usize = last_commit.languages.values().sum();
        repo_map.insert(repo.clone(), total);
    }
    let mut repos: Vec<_> = repo_map.keys().cloned().collect();
    repos.sort_by(|a, b| repo_map[a].cmp(&repo_map[b]));
    repos
}
