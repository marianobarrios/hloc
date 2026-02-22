use std::ffi::OsStr;
use chrono::{DateTime, Datelike, Utc};
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, UNIX_EPOCH};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct YearMonth {
    pub year: i32,
    pub month: u8,
}

impl YearMonth {
    pub fn from_datetime(datetime: DateTime<Utc>) -> Self {
        Self { year: datetime.year(), month: datetime.month() as u8 }
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

pub trait MutexExt<T>  {
    fn lock_or_panic(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_panic(&self) -> MutexGuard<'_, T> {
        self.lock().expect("lock should not be poisoned")
    }
}

pub trait OsStrExt  {
    fn to_str_or_panic(&self) -> &str;
}

impl OsStrExt for OsStr {
    fn to_str_or_panic(&self) -> &str {
        self.to_str().expect("path should be valid UTF-8")
    }
}

pub trait PathExt  {
    fn to_str_or_panic(&self) -> &str;
}

impl PathExt for Path {
    fn to_str_or_panic(&self) -> &str {
        self.to_str().expect("path should be valid UTF-8")
    }
}