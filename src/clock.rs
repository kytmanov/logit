use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

use crate::domain::{Profile, parse_fixed_offset, parse_timezone};

pub trait Clock {
    fn now(&self) -> NaiveDateTime;
}

pub fn today_in_profile(profile: &Profile) -> NaiveDate {
    let utc_naive = Utc::now().naive_utc();
    if let Some(tz) = parse_timezone(&profile.tz) {
        return tz.from_utc_datetime(&utc_naive).date_naive();
    }
    if let Some(offset) = parse_fixed_offset(&profile.tz) {
        return offset.from_utc_datetime(&utc_naive).date_naive();
    }
    Local.from_utc_datetime(&utc_naive).date_naive()
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> NaiveDateTime {
        Local::now().naive_local()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    now: NaiveDateTime,
}

impl FixedClock {
    pub fn new(now: NaiveDateTime) -> Self {
        Self { now }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> NaiveDateTime {
        self.now
    }
}
