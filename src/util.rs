use chrono::{DateTime, Datelike, Utc};
use std::fmt::{Display, Formatter};
use std::time::{Duration, UNIX_EPOCH};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct YearMonth {
    pub year: i32,
    pub month: u8,
}

impl YearMonth {
    pub fn from_datetime(datetime: DateTime<Utc>) -> Self {
        Self {
            year: datetime.year(),
            month: datetime.month() as u8,
        }
    }
}

impl Display for YearMonth {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.year, self.month)
    }
}

pub fn datetime_from_epoch_seconds(seconds: i64) -> DateTime<Utc> {
    let epoch: DateTime<Utc> = UNIX_EPOCH.into();
    epoch + Duration::from_secs(seconds as u64)
}
