use std::collections::BTreeMap;
use std::fs;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::atomic::atomic_write;
use crate::domain::{ExpectedTimeSource, Profile};
use crate::error::AppError;
use crate::paths::AppDirs;

#[derive(Debug, Clone)]
pub struct CalendarContext {
    days: BTreeMap<NaiveDate, CalendarDay>,
    notes: Vec<String>,
}

impl CalendarContext {
    pub fn configured(
        profile: &Profile,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Self, AppError> {
        let mut days = BTreeMap::new();
        let mut current = start;
        while current <= end {
            days.insert(
                current,
                CalendarDay {
                    expected_seconds: configured_expected_seconds(profile, current)?,
                    source: ExpectedTimeSource::ConfiguredWorkday,
                    holiday_name: None,
                },
            );
            current += Duration::days(1);
        }

        Ok(Self {
            days,
            notes: Vec::new(),
        })
    }

    pub fn day(&self, date: NaiveDate) -> Option<&CalendarDay> {
        self.days.get(&date)
    }

    pub fn notes(&self) -> &[String] {
        &self.notes
    }
}

#[derive(Debug, Clone)]
pub struct CalendarDay {
    pub expected_seconds: u32,
    pub source: ExpectedTimeSource,
    pub holiday_name: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn calendar_context(
    dirs: &AppDirs,
    profile_name: &str,
    tempo_token: &str,
    account_id: &str,
    profile: &Profile,
    start: NaiveDate,
    end: NaiveDate,
    no_calendar: bool,
    refresh: bool,
) -> Result<CalendarContext, AppError> {
    if no_calendar {
        return CalendarContext::configured(profile, start, end);
    }

    let client = HttpCalendarClient::default();
    let cache_path = dirs.calendar_file(profile_name);
    let now = Utc::now();

    if !refresh
        && let Some(cache) = load_cache(&cache_path)?
        && now.signed_duration_since(cache.fetched_at) < Duration::days(7)
    {
        return cache_to_context(profile, start, end, cache, false);
    }

    match client.fetch_schedule(tempo_token, account_id, start, end) {
        Ok(schedule) => {
            let cache = CalendarCache {
                fetched_at: now,
                scope_warning_at: None,
                schedule,
            };
            save_cache(&cache_path, &cache)?;
            cache_to_context(profile, start, end, cache, false)
        }
        Err(CalendarFetchError::NotAvailable) => CalendarContext::configured(profile, start, end),
        Err(CalendarFetchError::Unauthorized) => {
            let mut context = CalendarContext::configured(profile, start, end)?;
            let mut cache = load_cache(&cache_path)?.unwrap_or_else(|| CalendarCache {
                fetched_at: now,
                scope_warning_at: None,
                schedule: Vec::new(),
            });
            let should_warn = cache
                .scope_warning_at
                .map(|previous| now.signed_duration_since(previous) >= Duration::days(7))
                .unwrap_or(true);
            if should_warn {
                context.notes.push(String::from(
                    "Tempo calendar scope missing; regenerate the token with schedule access",
                ));
                cache.scope_warning_at = Some(now);
                save_cache(&cache_path, &cache)?;
            }
            Ok(context)
        }
        Err(CalendarFetchError::Network(error)) => {
            if let Some(cache) = load_cache(&cache_path)? {
                return cache_to_context(profile, start, end, cache, true);
            }
            Err(error)
        }
    }
}

pub fn cached_calendar_context(
    dirs: &AppDirs,
    profile_name: &str,
    profile: &Profile,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Option<CalendarContext>, AppError> {
    let cache_path = dirs.calendar_file(profile_name);
    let Some(cache) = load_cache(&cache_path)? else {
        return Ok(None);
    };
    let stale = Utc::now().signed_duration_since(cache.fetched_at) >= Duration::days(7);
    Ok(Some(cache_to_context(profile, start, end, cache, stale)?))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalendarCache {
    fetched_at: DateTime<Utc>,
    scope_warning_at: Option<DateTime<Utc>>,
    schedule: Vec<ScheduleDay>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScheduleDay {
    date: NaiveDate,
    required_seconds: u32,
    holiday_name: Option<String>,
}

#[derive(Debug)]
enum CalendarFetchError {
    NotAvailable,
    Unauthorized,
    Network(AppError),
}

fn cache_to_context(
    profile: &Profile,
    start: NaiveDate,
    end: NaiveDate,
    cache: CalendarCache,
    stale: bool,
) -> Result<CalendarContext, AppError> {
    let mut schedule_by_date = BTreeMap::new();
    for day in cache.schedule {
        schedule_by_date.insert(day.date, day);
    }

    let mut days = BTreeMap::new();
    let mut notes = Vec::new();
    let mut current = start;
    while current <= end {
        let entry = schedule_by_date.get(&current).cloned();
        let configured = configured_expected_seconds(profile, current)?;
        let (expected_seconds, holiday_name) = entry
            .map(|day| (day.required_seconds, day.holiday_name))
            .unwrap_or((configured, None));
        let source = if stale {
            let days_old = Utc::now()
                .signed_duration_since(cache.fetched_at)
                .num_days()
                .max(0) as u32;
            ExpectedTimeSource::StaleCache { days_old }
        } else {
            ExpectedTimeSource::TempoWorkSchedule
        };
        if !stale && expected_seconds != configured {
            notes.push(format!(
                "Tempo schedule reports {}s for {}; configured workday is {}s. Stats use Tempo.",
                expected_seconds, current, configured
            ));
        }
        days.insert(
            current,
            CalendarDay {
                expected_seconds,
                source,
                holiday_name,
            },
        );
        current += Duration::days(1);
    }

    notes.sort();
    notes.dedup();
    Ok(CalendarContext { days, notes })
}

fn load_cache(path: &std::path::Path) -> Result<Option<CalendarCache>, AppError> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| AppError::config(format!("read {}: {error}", path.display())))?;
    let cache = serde_json::from_str(&raw)
        .map_err(|error| AppError::config(format!("parse {}: {error}", path.display())))?;
    Ok(Some(cache))
}

fn save_cache(path: &std::path::Path, cache: &CalendarCache) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::config(format!("create dir {}: {error}", parent.display()))
        })?;
    }
    let payload = serde_json::to_vec(cache)
        .map_err(|error| AppError::config(format!("serialize calendar cache: {error}")))?;
    atomic_write(path, &payload)
}

fn configured_expected_seconds(profile: &Profile, date: NaiveDate) -> Result<u32, AppError> {
    let start = chrono::NaiveTime::parse_from_str(&profile.work_hours.start, "%H:%M")
        .map_err(|_| AppError::config("invalid work_hours.start in config"))?;
    let end = chrono::NaiveTime::parse_from_str(&profile.work_hours.end, "%H:%M")
        .map_err(|_| AppError::config("invalid work_hours.end in config"))?;
    let is_working_day = profile.working_days.iter().any(|weekday| {
        matches!(
            (weekday, date.weekday()),
            (crate::domain::WeekdayName::Mon, chrono::Weekday::Mon)
                | (crate::domain::WeekdayName::Tue, chrono::Weekday::Tue)
                | (crate::domain::WeekdayName::Wed, chrono::Weekday::Wed)
                | (crate::domain::WeekdayName::Thu, chrono::Weekday::Thu)
                | (crate::domain::WeekdayName::Fri, chrono::Weekday::Fri)
                | (crate::domain::WeekdayName::Sat, chrono::Weekday::Sat)
                | (crate::domain::WeekdayName::Sun, chrono::Weekday::Sun)
        )
    });
    if !is_working_day {
        return Ok(0);
    }

    Ok((end - start).num_seconds().max(0) as u32)
}

#[derive(Debug, Clone)]
struct HttpCalendarClient {
    client: Client,
    base_url: String,
}

impl Default for HttpCalendarClient {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("calendar reqwest client"),
            base_url: String::from("https://api.tempo.io"),
        }
    }
}

impl HttpCalendarClient {
    fn fetch_schedule(
        &self,
        tempo_token: &str,
        account_id: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<ScheduleDay>, CalendarFetchError> {
        let schedule_response = self
            .client
            .get(format!(
                "{}/4/user-schedule/{}?from={}&to={}",
                self.base_url.trim_end_matches('/'),
                account_id,
                start.format("%Y-%m-%d"),
                end.format("%Y-%m-%d")
            ))
            .bearer_auth(tempo_token)
            .send()
            .map_err(|error| {
                CalendarFetchError::Network(AppError::network(format!(
                    "tempo schedule fetch failed: {error}"
                )))
            })?;

        match schedule_response.status().as_u16() {
            200 => {
                let schedule_body: DayScheduleResults =
                    schedule_response.json().map_err(|error| {
                        CalendarFetchError::Network(AppError::network(format!(
                            "parse tempo schedule response: {error}"
                        )))
                    })?;
                Ok(schedule_body
                    .results
                    .into_iter()
                    .map(|day| ScheduleDay {
                        date: day.date,
                        required_seconds: day.required_seconds as u32,
                        holiday_name: day.holiday.map(|holiday| holiday.name),
                    })
                    .collect())
            }
            404 => Err(CalendarFetchError::NotAvailable),
            401 | 403 => Err(CalendarFetchError::Unauthorized),
            status => Err(CalendarFetchError::Network(AppError::network(format!(
                "tempo schedule returned HTTP {status}"
            )))),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DayScheduleResults {
    results: Vec<DaySchedule>,
}

#[derive(Debug, Deserialize)]
struct DaySchedule {
    date: NaiveDate,
    #[serde(rename = "requiredSeconds")]
    required_seconds: i64,
    holiday: Option<Holiday>,
}

#[derive(Debug, Deserialize)]
struct Holiday {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_profile;

    #[test]
    fn configured_context_marks_non_working_days_zero() {
        let profile = default_profile("UTC");
        let context = CalendarContext::configured(
            &profile,
            NaiveDate::from_ymd_opt(2026, 4, 4).unwrap(),
            NaiveDate::from_ymd_opt(2026, 4, 4).unwrap(),
        )
        .expect("context builds");

        assert_eq!(
            context
                .day(NaiveDate::from_ymd_opt(2026, 4, 4).unwrap())
                .unwrap()
                .expected_seconds,
            0
        );
    }

    #[test]
    fn stale_cache_marks_rows_stale() {
        let profile = default_profile("UTC");
        let cache = CalendarCache {
            fetched_at: Utc::now() - Duration::days(3),
            scope_warning_at: None,
            schedule: vec![ScheduleDay {
                date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                required_seconds: 14400,
                holiday_name: Some(String::from("Holiday")),
            }],
        };

        let context = cache_to_context(
            &profile,
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            cache,
            true,
        )
        .expect("context builds");

        assert!(matches!(
            context
                .day(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap())
                .unwrap()
                .source,
            ExpectedTimeSource::StaleCache { .. }
        ));
    }
}
