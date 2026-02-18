use std::collections::{BTreeMap, HashMap};
use std::ops::AddAssign;
use std::path::Path;
use tokei::Config;
use crate::util::YearMonth;

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
    pub line_stats: LineStats,
    pub file_count: usize,
    pub children: HashMap<tokei::LanguageType, LineStats>,
}

#[derive(Debug, Copy, Clone)]
pub struct LineStats {
    pub blanks: usize,
    pub code: usize,
    pub comments: usize,
}

impl AddAssign for HistoricStats {
    fn add_assign(&mut self, rhs: Self) {
        for (month, stats) in rhs.snapshots {
            let value = self.snapshots.entry(month).or_insert(CodeStats::zero());
            *value += stats;
        }
    }
}

impl CodeStats {
    pub fn zero() -> Self {
        Self {
            languages: HashMap::new(),
        }
    }

    pub fn generate(path: &Path) -> Self {
        let mut tokei_languages = tokei::Languages::new();
        tokei_languages.get_statistics(&[path], &[], &Config::default());
        Self::from_tokei_stats(&tokei_languages)
    }

    fn from_tokei_stats(tokei_languages: &tokei::Languages) -> Self {
        let mut languages = HashMap::new();
        for (&language_type, tokei_lang) in tokei_languages {
            let mut children = HashMap::new();
            for (&language_type, reports) in tokei_lang.children.iter() {
                let line_stats = LineStats {
                    blanks: reports.iter().map(|l| l.stats.blanks).sum(),
                    code: reports.iter().map(|l| l.stats.code).sum(),
                    comments: reports.iter().map(|l| l.stats.comments).sum(),
                };
                children.insert(language_type, line_stats);
            }
            let language_stats = LanguageStats {
                line_stats: LineStats {
                    blanks: tokei_lang.blanks,
                    code: tokei_lang.code,
                    comments: tokei_lang.comments,
                },
                file_count: tokei_lang.reports.len(),
                children,
            };
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
        Self {
            line_stats: LineStats::zero(),
            file_count: 0,
            children: HashMap::new(),
        }
    }
}

impl AddAssign for LanguageStats {
    fn add_assign(&mut self, rhs: Self) {
        self.line_stats += rhs.line_stats;
        self.file_count += rhs.file_count;
        for (lang, stats) in rhs.children {
            let value = self.children.entry(lang).or_insert(LineStats::zero());
            *value += stats;
        }
    }
}

impl LineStats {
    pub fn zero() -> Self {
        Self {
            blanks: 0,
            code: 0,
            comments: 0,
        }
    }
}

impl AddAssign for LineStats {
    fn add_assign(&mut self, rhs: Self) {
        self.blanks += rhs.blanks;
        self.code += rhs.code;
        self.comments += rhs.comments;
    }
}
