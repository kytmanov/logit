use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

use crate::domain::{Profile, parse_fixed_offset, parse_timezone};

pub trait Clock {
    fn now_utc(&self) -> DateTime<Utc>;
}

pub fn today_in_profile(profile: &Profile) -> NaiveDate {
    let utc_naive = Utc::now().naive_utc();
    profile_local_datetime(profile, utc_naive).date()
}

pub fn today_in_profile_at<C: Clock>(clock: &C, profile: &Profile) -> NaiveDate {
    profile_local_datetime(profile, clock.now_utc().naive_utc()).date()
}

pub fn current_profile_datetime<C: Clock>(clock: &C, profile: &Profile) -> NaiveDateTime {
    profile_local_datetime(profile, clock.now_utc().naive_utc())
}

fn profile_local_datetime(profile: &Profile, utc_naive: NaiveDateTime) -> NaiveDateTime {
    if let Some(tz) = parse_timezone(&profile.tz) {
        return tz.from_utc_datetime(&utc_naive).naive_local();
    }
    if let Some(offset) = parse_fixed_offset(&profile.tz) {
        return offset.from_utc_datetime(&utc_naive).naive_local();
    }
    Local.from_utc_datetime(&utc_naive).naive_local()
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> DateTime<Utc> {
        Utc::now()
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
    fn now_utc(&self) -> DateTime<Utc> {
        DateTime::from_naive_utc_and_offset(self.now, Utc)
    }
}
