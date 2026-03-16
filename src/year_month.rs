use chrono::Datelike;
use std::fmt::{Display, Formatter};
use std::iter::FusedIterator;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct YearMonth {
    pub year: i32,
    pub month: u32,
}

impl YearMonth {
    pub fn from_datelike<T: Datelike>(datelike: T) -> Self {
        Self { year: datelike.year(), month: datelike.month() }
    }

    /// Creates an _inclusive_ iterator
    pub fn iter_to(self, end: YearMonth) -> YearMonthInclusiveIter {
        YearMonthInclusiveIter { current: self, last: end }
    }

    pub fn inc(&mut self) {
        if self.month == 12 {
            self.year += 1;
            self.month = 1;
        } else {
            self.month += 1;
        }
    }
}

impl Display for YearMonth {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{:02}", self.year, self.month)
    }
}

pub struct YearMonthInclusiveIter {
    current: YearMonth,
    last: YearMonth,
}

impl Iterator for YearMonthInclusiveIter {
    type Item = YearMonth;

    fn next(&mut self) -> Option<Self::Item> {
        // iterator is inclusive
        if self.current > self.last {
            return None;
        }
        let result = self.current;
        self.current.inc();
        Some(result)
    }
}

impl FusedIterator for YearMonthInclusiveIter {}
