use chrono::{Datelike, NaiveDate, Utc, Weekday};
use std::fmt::{Debug, Display, Formatter};
use std::iter::FusedIterator;

/// A discrete time period used as a snapshot key in the time series.
pub trait TimePeriod: Ord + Copy + Display + Debug + Send + Sync {
    /// Creates a period from any chrono date-like value.
    fn from_datelike<T: Datelike>(d: T) -> Self;

    /// Advances to the next period in-place.
    fn inc(&mut self);

    /// Human-readable label for the x-axis (e.g. "Week", "Month", "Quarter").
    fn axis_label() -> &'static str;

    /// Returns the period that contains the current instant.
    fn current() -> Self {
        Self::from_datelike(Utc::now())
    }

    /// Returns an inclusive iterator from `self` to `end`.
    fn iter_to(self, end: Self) -> PeriodIter<Self> {
        PeriodIter { current: self, last: end }
    }
}

/// Inclusive iterator over consecutive time periods.
pub struct PeriodIter<P> {
    current: P,
    last: P,
}

impl<P: TimePeriod> Iterator for PeriodIter<P> {
    type Item = P;

    fn next(&mut self) -> Option<P> {
        if self.current > self.last {
            return None;
        }
        let result = self.current;
        self.current.inc();
        Some(result)
    }
}

impl<P: TimePeriod> FusedIterator for PeriodIter<P> {}

// ---------------------------------------------------------------------------
// YearMonth
// ---------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct YearMonth {
    pub year: i32,
    /// ISO month number (1–12).
    pub month: u32,
}

impl TimePeriod for YearMonth {
    fn from_datelike<T: Datelike>(datelike: T) -> Self {
        Self { year: datelike.year(), month: datelike.month() }
    }

    fn axis_label() -> &'static str {
        "Month"
    }

    fn inc(&mut self) {
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

// ---------------------------------------------------------------------------
// YearQuarter
// ---------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct YearQuarter {
    pub year: i32,
    /// Quarter number (1–4).
    pub quarter: u32,
}

impl TimePeriod for YearQuarter {
    fn from_datelike<T: Datelike>(datelike: T) -> Self {
        Self { year: datelike.year(), quarter: (datelike.month() - 1) / 3 + 1 }
    }

    fn axis_label() -> &'static str {
        "Quarter"
    }

    fn inc(&mut self) {
        if self.quarter == 4 {
            self.year += 1;
            self.quarter = 1;
        } else {
            self.quarter += 1;
        }
    }
}

impl Display for YearQuarter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-Q{}", self.year, self.quarter)
    }
}

// ---------------------------------------------------------------------------
// YearWeek (ISO 8601)
// ---------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct YearWeek {
    pub year: i32,
    /// ISO week number (1–53).
    pub week: u32,
}

fn weeks_in_year(year: i32) -> u32 {
    if NaiveDate::from_isoywd_opt(year, 53, Weekday::Mon).is_some() { 53 } else { 52 }
}

impl TimePeriod for YearWeek {
    fn from_datelike<T: Datelike>(datelike: T) -> Self {
        let date = NaiveDate::from_ymd_opt(datelike.year(), datelike.month(), datelike.day()).unwrap();
        let iso = date.iso_week();
        Self { year: iso.year(), week: iso.week() }
    }

    fn axis_label() -> &'static str {
        "Week"
    }

    fn inc(&mut self) {
        if self.week < weeks_in_year(self.year) {
            self.week += 1;
        } else {
            self.year += 1;
            self.week = 1;
        }
    }
}

impl Display for YearWeek {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-W{:02}", self.year, self.week)
    }
}
