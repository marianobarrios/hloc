use chrono::NaiveDate;
use std::collections::HashMap;

fn default_min_lines() -> u32 {
    1
}

/// Key is a glob pattern
pub type Config = HashMap<String, RepoConfig>;

/// The main configuration structure representing the TOML file
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RepoConfig {
    #[serde(default)]
    pub ignore: bool,

    #[serde(default)]
    pub skip_languages: Vec<String>,

    #[serde(default = "default_min_lines")]
    pub min_lines: u32,

    #[serde(default)]
    pub from_time: Option<NaiveDate>,

    #[serde(default)]
    pub archived: bool,
}
