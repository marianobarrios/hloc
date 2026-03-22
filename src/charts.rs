use crate::stats::Stats;
use crate::time_period::TimePeriod;
use crate::util::PathExt;
use crate::{display_name, util};
use anyhow::Context;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::{fs, io};

const CHART_HTML: &[u8] = include_bytes!("../templates/chart.html");
const CHART_JS: &[u8] = include_bytes!("../templates/chart.js");
const CHART_CSS: &[u8] = include_bytes!("../templates/chart.css");

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

    write_template(output_dir, "chart.html", CHART_HTML)?;
    write_template(output_dir, "chart.js", CHART_JS)?;
    write_template(output_dir, "chart.css", CHART_CSS)?;

    let period_label = P::axis_label();
    let data_file = output_dir.join("data.js");
    fs::write(
        &data_file,
        format!("by_repo_data = {by_repo_data};\nby_lang_data = {by_lang_data};\nperiod_label = \"{period_label}\";\n"),
    )
    .with_context(|| format!("cannot write file {}", data_file.display()))?;
    Ok(output_dir.join("chart.html"))
}

fn write_template(output_dir: &Path, file_name: &str, contents: &[u8]) -> anyhow::Result<()> {
    let file = output_dir.join(file_name);
    fs::write(&file, contents).with_context(|| format!("cannot write file {}", file.display()))?;
    Ok(())
}

fn get_by_repo_chart<P: TimePeriod>(base_dir: &Path, stats: &Stats<P>) -> serde_json::Value {
    let x_labels: Vec<_> = stats.from.iter_to(stats.to).map(|p| p.to_string()).collect();
    let dataset: Vec<_> = get_sorted_repos(stats)
        .iter()
        .map(|repo| {
            let historic_stats = &stats.repositories[repo];
            // For archived repositories, `fill_gaps` only fills up to the last commit, not
            // `stats.to`. This means `period_data` may be shorter than `x_labels`, which is
            // intentional: Chart.js aligns data points to the first N labels, so the line
            // ends at the last commit and the tail of the x-axis is left empty.
            let period_data: Vec<_> =
                historic_stats.periods.values().map(|period_stats| period_stats.total_lines()).collect();
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
fn get_sorted_languages<P: TimePeriod>(global_stats: &Stats<P>) -> Vec<tokei::LanguageType> {
    let mut language_map: HashMap<tokei::LanguageType, usize> = HashMap::new();
    for historic_stats in global_stats.repositories.values() {
        let last_commit =
            historic_stats.periods.values().last().expect("repository should have at least one commit");
        for (language, line_count) in &last_commit.languages {
            *language_map.entry(*language).or_insert(0) += line_count;
        }
    }
    let mut languages: Vec<_> = language_map.into_iter().collect();
    languages.sort_by_key(|&(_, count)| count);
    languages.into_iter().map(|(lang, _)| lang).collect()
}

/// Returns the repositories present in the stats, sorted by increasing size (using last commit)
fn get_sorted_repos<P: TimePeriod>(global_stats: &Stats<P>) -> Vec<PathBuf> {
    let mut repos: Vec<_> = global_stats
        .repositories
        .iter()
        .map(|(repo, historic_stats)| {
            let last_commit =
                historic_stats.periods.values().last().expect("repository should have at least one commit");
            (repo.clone(), last_commit.total_lines())
        })
        .collect();
    repos.sort_by_key(|&(_, total)| total);
    repos.into_iter().map(|(repo, _)| repo).collect()
}
