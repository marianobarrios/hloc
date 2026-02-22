use crate::util::YearMonth;
use std::collections::{BTreeMap, HashMap};
use std::ops::AddAssign;
use std::path::Path;
use tokei::Config;

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
    pub languages: HashMap<tokei::LanguageType, LanguageStats>,
}

/// Statistics of a single repository and language at a single point in time
#[derive(Debug, Clone)]
pub struct LanguageStats {
    pub line_count: usize,
    pub children: HashMap<tokei::LanguageType, usize>,
}

impl CodeStats {
    pub fn zero() -> Self {
        Self { languages: HashMap::new() }
    }

    pub fn generate(path: &Path) -> Self {
        let mut tokei_languages = tokei::Languages::new();

        // as we are counting on a clean clone, we don't need to ignore local files
        let config = Config { no_ignore: Some(true), ..Default::default() };

        tokei_languages.get_statistics(&[path], &[], &config);
        Self::from_tokei_stats(&tokei_languages)
    }

    fn from_tokei_stats(tokei_languages: &tokei::Languages) -> Self {
        let mut languages = HashMap::new();
        for (&language_type, tokei_lang) in tokei_languages {
            let mut children = HashMap::new();
            for (&language_type, reports) in tokei_lang.children.iter() {
                let line_stats = reports.iter().map(|l| l.stats.code).sum();
                children.insert(language_type, line_stats);
            }
            let language_stats = LanguageStats { line_count: tokei_lang.code, children };
            languages.insert(language_type, language_stats);
        }
        Self { languages }
    }
}

impl AddAssign for CodeStats {
    fn add_assign(&mut self, rhs: Self) {
        for (lang, stats) in rhs.languages {
            let value = self.languages.entry(lang).or_insert(LanguageStats::zero());
            *value += stats;
        }
    }
}

impl LanguageStats {
    pub fn zero() -> Self {
        Self { line_count: 0, children: HashMap::new() }
    }
}

impl AddAssign for LanguageStats {
    fn add_assign(&mut self, rhs: Self) {
        self.line_count += rhs.line_count;
        for (lang, stats) in rhs.children {
            let value = self.children.entry(lang).or_insert(0);
            *value += stats;
        }
    }
}
