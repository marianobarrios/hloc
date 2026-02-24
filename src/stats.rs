use crate::util::YearMonth;
use std::collections::{BTreeMap, HashMap};
use std::ops::AddAssign;

/// Statistics across repositories and time
#[derive(Debug, Clone)]
pub struct GlobalStats {
    pub repositories: HashMap<String, HistoricStats>,
}

/// Statistics of a single repository across time
#[derive(Debug, Clone)]
pub struct HistoricStats {
    pub snapshots: BTreeMap<YearMonth, CodeStats>,
}

/// Statistics of a single repository at a single point in time
#[derive(Debug, Clone)]
pub struct CodeStats {
    pub languages: HashMap<tokei::LanguageType, usize>,
}

impl CodeStats {
    pub fn zero() -> Self {
        Self { languages: HashMap::new() }
    }
}

impl AddAssign for CodeStats {
    fn add_assign(&mut self, rhs: Self) {
        for (lang, stats) in rhs.languages {
            let value = self.languages.entry(lang).or_insert(0);
            *value += stats;
        }
    }
}
