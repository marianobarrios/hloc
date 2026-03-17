use crate::util;
use chrono::NaiveDate;
use clap::command;
use std::cmp;
use std::collections::HashMap;

pub const HELP: &str = r#"Path to a TOML configuration file.

The file is a map of Unix glob patterns to repository settings:

  ["**/*"]
  min_lines = 5000
  skip_languages = ["Xml", "Json"]

  ["**/some-repo"]
  ignore = true

Available settings per pattern:
  ignore          (bool)        Exclude matching repositories entirely [default: false]
  skip_languages  ([string])    Languages to exclude from the line count [default: none]
  min_lines       (integer)     Minimum lines of code required for a repository to appear in the report [default: 1]
  from_time       (date)        Only count commits from this date onward (YYYY-MM-DD) [default: none]
  archived        (bool)        Treat matching repositories as archived. Archived repositories are assumed to finish at the last commit, as opposed to propagating until the current date [default: false]
  fork_priority   (integer)     Priority used during fork detection. When two repositories share commit history, the one with the lower value is treated as the original and keeps the shared commits; the other has those commits removed. Ties are broken alphabetically. [default: 0]

Multiple patterns can match a repository; settings are merged (ignore/archived are OR'd, min_lines takes the max, skip_languages are combined, fork_priority takes the min)."#;

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
