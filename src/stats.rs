use crate::time_period::TimePeriod;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// Statistics across repositories and time
#[derive(Debug)]
pub struct Stats<P: TimePeriod> {
    pub from: P,
    pub to: P,
    pub repositories: HashMap<PathBuf, HistoricStats<P>>,
}

/// Statistics of a single repository across time
#[derive(Debug)]
pub struct HistoricStats<P: TimePeriod> {
    pub periods: BTreeMap<P, CodeStats>,
}

/// Statistics of a single repository at a single point in time
#[derive(Debug, Clone, Default)]
pub struct CodeStats {
    pub languages: HashMap<tokei::LanguageType, usize>,
}
