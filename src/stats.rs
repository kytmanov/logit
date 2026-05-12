use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};

use crate::calendar::CalendarContext;
use crate::clock::today_in_profile;
use crate::domain::{
    ExpectedTimeSource, Profile, StatRangeInput, StatReport, StatRow, StatSelector, WorklogLine,
    WorklogResult,
};
use crate::error::AppError;

pub fn select_range(
    selector: &StatSelector,
    profile: &Profile,
) -> Result<(String, NaiveDate, NaiveDate), AppError> {
    let today = today_in_profile(profile);
    match selector {
        StatSelector::Today => Ok((String::from("today"), today, today)),
        StatSelector::Yesterday => {
            let yesterday = today - Duration::days(1);
            Ok((String::from("yesterday"), yesterday, yesterday))
        }
        StatSelector::Date(date) => Ok((date.format("%Y-%m-%d").to_string(), *date, *date)),
        StatSelector::Week => {
            let start = start_of_week(today);
            Ok((String::from("week"), start, start + Duration::days(6)))
        }
        StatSelector::LastWeek => {
            let end = start_of_week(today) - Duration::days(1);
            let start = end - Duration::days(6);
            Ok((String::from("last week"), start, end))
        }
        StatSelector::Year(year) => {
            let start = NaiveDate::from_ymd_opt(*year, 1, 1)
                .ok_or_else(|| AppError::validation(format!("invalid year: {year}")))?;
            let end = NaiveDate::from_ymd_opt(*year, 12, 31)
                .ok_or_else(|| AppError::validation(format!("invalid year: {year}")))?;
            Ok((year.to_string(), start, end))
        }
        StatSelector::Month { month, year } => {
            let year = if *year == 0 { today.year() } else { *year };
            let start = NaiveDate::from_ymd_opt(year, *month, 1).ok_or_else(|| {
                AppError::validation(format!("invalid month selector: {month}/{year}"))
            })?;
            let next_month = if *month == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
            } else {
                NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
            };
            let end = next_month - Duration::days(1);
            Ok((format!("{:04}-{:02}", year, month), start, end))
        }
    }
}

pub fn build_stat_report(
    _profile: &Profile,
    label: &str,
    start: NaiveDate,
    end: NaiveDate,
    worklogs: Vec<WorklogResult>,
    calendar: &CalendarContext,
) -> Result<StatReport, AppError> {
    let mut rows = Vec::new();
    let mut current = start;
    while current <= end {
        let day_worklogs: Vec<&WorklogResult> = worklogs
            .iter()
            .filter(|worklog| worklog.start.date() == current)
            .collect();
        let filled_seconds: u32 = day_worklogs
            .iter()
            .map(|worklog| worklog.duration_seconds)
            .sum();
        let calendar_day = calendar
            .day(current)
            .ok_or_else(|| AppError::config(format!("missing calendar day for {current}")))?;
        let expected_seconds = if calendar_day.expected_seconds > 0 || filled_seconds > 0 {
            calendar_day.expected_seconds
        } else {
            0
        };
        let source = calendar_day.source.clone();

        let mut sorted_worklogs = day_worklogs;
        sorted_worklogs.sort_by_key(|worklog| worklog.start);
        rows.push(StatRow {
            date: current,
            source,
            worklogs: sorted_worklogs
                .into_iter()
                .map(|worklog| WorklogLine {
                    issue_key: worklog.issue_key.clone(),
                    duration_seconds: worklog.duration_seconds,
                    start: worklog.start.time(),
                    end: worklog.end.time(),
                    description: worklog.description.clone(),
                })
                .collect(),
            expected_seconds,
            filled_seconds,
            holiday_name: calendar_day.holiday_name.clone(),
        });

        current += Duration::days(1);
    }

    Ok(StatReport {
        label: label.to_owned(),
        rows,
        notes: calendar.notes().to_vec(),
    })
}

pub fn build_empty_report(input: &StatRangeInput, label: &str) -> Result<StatReport, AppError> {
    let source = if input.no_calendar {
        ExpectedTimeSource::ConfiguredWorkday
    } else {
        ExpectedTimeSource::TempoWorkSchedule
    };

    Ok(StatReport {
        label: label.to_owned(),
        rows: vec![StatRow {
            date: Local::now().date_naive(),
            source,
            worklogs: Vec::new(),
            expected_seconds: 0,
            filled_seconds: 0,
            holiday_name: None,
        }],
        notes: Vec::new(),
    })
}

fn start_of_week(day: NaiveDate) -> NaiveDate {
    let offset = match day.weekday() {
        Weekday::Mon => 0,
        Weekday::Tue => 1,
        Weekday::Wed => 2,
        Weekday::Thu => 3,
        Weekday::Fri => 4,
        Weekday::Sat => 5,
        Weekday::Sun => 6,
    };
    day - Duration::days(offset)
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveDateTime};

    use crate::calendar::CalendarContext;
    use crate::config::default_profile;
    use crate::domain::{ExpectedTimeSource, StatSelector};

    use super::*;

    #[test]
    fn selects_week_range() {
        let profile = default_profile("UTC");
        let (label, start, end) =
            select_range(&StatSelector::Week, &profile).expect("range selects");

        assert_eq!(label, "week");
        assert_eq!((end - start).num_days(), 6);
    }

    #[test]
    fn selects_yesterday_range() {
        let profile = default_profile("UTC");
        let (label, start, end) =
            select_range(&StatSelector::Yesterday, &profile).expect("range selects");

        assert_eq!(label, "yesterday");
        assert_eq!(start, end);
        assert_eq!(end, today_in_profile(&profile) - Duration::days(1));
    }

    #[test]
    fn builds_stat_report_from_worklogs() {
        let profile = default_profile("UTC");
        let worklogs = vec![WorklogResult {
            worklog_id: String::from("w1"),
            issue_key: String::from("TK-1"),
            issue_id: Some(String::from("10001")),
            start: NaiveDateTime::parse_from_str("2026-04-01 09:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap(),
            end: NaiveDateTime::parse_from_str("2026-04-01 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            duration_seconds: 3600,
            tempo_url: String::new(),
            description: None,
        }];

        let report = build_stat_report(
            &profile,
            "week",
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            worklogs,
            &CalendarContext::configured(
                &profile,
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            )
            .unwrap(),
        )
        .expect("report builds");

        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].filled_seconds, 3600);
        assert_eq!(report.rows[0].source, ExpectedTimeSource::ConfiguredWorkday);
    }
}
