use crate::year_month::YearMonth;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// Statistics across repositories and time
#[derive(Debug)]
pub struct Stats {
    pub repositories: HashMap<PathBuf, HistoricStats>,
}

/// Statistics of a single repository across time
#[derive(Debug)]
pub struct HistoricStats {
    pub periods: BTreeMap<YearMonth, CodeStats>,
}

/// Statistics of a single repository at a single point in time
#[derive(Debug, Clone, Default)]
pub struct CodeStats {
    pub languages: HashMap<tokei::LanguageType, usize>,
}
