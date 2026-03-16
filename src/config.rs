use crate::util;
use chrono::NaiveDate;
use clap::command;
use std::cmp;
use std::collections::HashMap;

/// Key is a glob pattern
pub type Config = HashMap<String, RepoConfig>;

/// The main configuration structure representing the TOML file
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RepoConfig {
    #[serde(default)]
    pub ignore: bool,

    #[serde(default, deserialize_with = "deserialize_languages")]
    pub skip_languages: Vec<tokei::LanguageType>,

    #[serde(default = "RepoConfig::default_min_lines")]
    pub min_lines: u32,

    #[serde(default)]
    pub from_time: Option<NaiveDate>,

    #[serde(default)]
    pub archived: bool,

    /// An option is used (instead of relying on the integer default of zero) for merge to work:
    /// The minimum priority is preserved during merging, but excluding Nones (merging None and 10
    /// results in 10, not 0).
    #[serde(default)]
    pub fork_priority: Option<i32>,
}

impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            ignore: false,
            skip_languages: vec![],
            min_lines: 1,
            from_time: None,
            archived: false,
            fork_priority: None,
        }
    }
}

impl RepoConfig {
    fn default_min_lines() -> u32 {
        1
    }

    pub fn merge(mut self, other: &Self) -> Self {
        self.skip_languages.extend_from_slice(&other.skip_languages);
        Self {
            ignore: self.ignore || other.ignore,
            skip_languages: self.skip_languages,
            min_lines: cmp::max(self.min_lines, other.min_lines),
            from_time: util::merge_options(self.from_time, other.from_time, cmp::max),
            archived: self.archived || other.archived,
            fork_priority: util::merge_options(self.fork_priority, other.fork_priority, cmp::min),
        }
    }
}

fn deserialize_languages<'de, D>(deserializer: D) -> Result<Vec<tokei::LanguageType>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let strings: Vec<String> = serde::Deserialize::deserialize(deserializer)?;
    strings
        .iter()
        .map(|s| {
            tokei::LanguageType::from_name(s).ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "unknown language \"{s}\" (use {} --languages for a list of supported languages)",
                    command!().get_name()
                ))
            })
        })
        .collect()
}
