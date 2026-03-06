use chrono::{DateTime, Datelike, Utc};
use std::ffi::OsStr;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, UNIX_EPOCH};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct YearMonth {
    pub year: i32,
    pub month: u32,
}

impl YearMonth {
    pub fn from_datelike<T: Datelike>(datelike: T) -> Self {
        Self { year: datelike.year(), month: datelike.month() }
    }
}

impl Display for YearMonth {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{:02}", self.year, self.month)
    }
}

pub fn datetime_from_epoch_seconds(seconds: i64) -> DateTime<Utc> {
    let epoch: DateTime<Utc> = UNIX_EPOCH.into();
    epoch + Duration::from_secs(seconds as u64)
}

pub trait MutexExt<T> {
    fn lock_or_panic(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_panic(&self) -> MutexGuard<'_, T> {
        self.lock().expect("lock should not be poisoned")
    }
}

pub trait OsStrExt {
    fn to_str_or_panic(&self) -> &str;
}

impl OsStrExt for OsStr {
    fn to_str_or_panic(&self) -> &str {
        self.to_str().expect("path should be valid UTF-8")
    }
}

pub trait PathExt {
    fn to_str_or_panic(&self) -> &str;
}

impl PathExt for Path {
    fn to_str_or_panic(&self) -> &str {
        self.to_str().expect("path should be valid UTF-8")
    }
}

pub fn gen_month_range(from: YearMonth, to: YearMonth) -> Vec<YearMonth> {
    let mut months = Vec::new();
    for year in from.year..=to.year {
        let min_month = if year == from.year { from.month } else { 1 };
        let max_month = if year == to.year { to.month } else { 12 };
        for month in min_month..=max_month {
            months.push(YearMonth { year, month });
        }
    }
    months
}

pub fn merge_options<T>(a: Option<T>, b: Option<T>, merge_fn: fn(T, T) -> T) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(merge_fn(a, b)),
        (opt_a, opt_b) => opt_a.or(opt_b),
    }
}
