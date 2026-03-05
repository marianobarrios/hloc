use chrono::NaiveDate;
use std::collections::HashMap;

/// The main configuration structure representing the YAML file
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Config {
    /// Glob pattern
    pub repositories: HashMap<String, RepoConfig>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RepoConfig {
    #[serde(default)]
    pub ignore: bool,

    #[serde(default)]
    pub skip_languages: Vec<String>,

    #[serde(default)]
    pub from_time: Option<NaiveDate>,

    #[serde(default)]
    pub to_time: Option<NaiveDate>,
}
