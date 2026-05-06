use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};

use logit::domain::{
    Alias, ExpectedTimeSource, Profile, SetupValues, StatRangeInput, StatReport, StatRow,
    StatSelector, TimeFormat, WeekdayName, WorkHours, WorklogDraft, WorklogLine, WorklogResult,
};
use logit::error::AppError;
use logit::paths::AppDirs;
use logit::style::Style;
use logit::ui::{
    render_alias_delete, render_alias_set, render_aliases, render_cache_clear, render_doctor,
    render_dry_run, render_error, render_log_success, render_setup_complete, render_stats,
};

fn fixture_today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
}

fn fixture_profile() -> Profile {
    Profile {
        jira_url: String::from("https://example.atlassian.net"),
        email: String::from("user@example.com"),
        account_id: Some(String::from("acct-1")),
        tz: String::from("Europe/Berlin"),
        work_hours: WorkHours {
            start: String::from("09:00"),
            end: String::from("17:00"),
        },
        working_days: vec![
            WeekdayName::Mon,
            WeekdayName::Tue,
            WeekdayName::Wed,
            WeekdayName::Thu,
            WeekdayName::Fri,
        ],
        time_format: TimeFormat::TwentyFourHour,
        aliases: BTreeMap::new(),
    }
}

fn worklog_result(
    issue: &str,
    hours: u32,
    start_hour: u32,
    message: Option<&str>,
) -> WorklogResult {
    let start = NaiveDateTime::new(
        fixture_today(),
        NaiveTime::from_hms_opt(start_hour, 0, 0).unwrap(),
    );
    let end = start + chrono::Duration::hours(hours as i64);
    WorklogResult {
        worklog_id: String::from("42"),
        issue_key: issue.to_string(),
        issue_id: Some(String::from("10001")),
        start,
        end,
        duration_seconds: hours * 3600,
        tempo_url: format!("https://example.atlassian.net/tempo/worklog/{}", "42"),
        description: message.map(String::from),
    }
}

#[test]
fn snapshot_log_success() {
    let result = worklog_result("TK-1234", 1, 9, Some("fix flaky test"));
    let style = Style::plain();
    let text = render_log_success(&result, &fixture_profile(), fixture_today(), &style, false);
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_dry_run() {
    let draft = WorklogDraft {
        issue_key: String::from("TK-1234"),
        start: NaiveDateTime::new(fixture_today(), NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
        end: NaiveDateTime::new(fixture_today(), NaiveTime::from_hms_opt(10, 30, 0).unwrap()),
        duration_seconds: 5400,
        timezone: String::from("Europe/Berlin"),
        description: Some(String::from("plan rework")),
    };
    let style = Style::plain();
    let text = render_dry_run(
        &draft,
        "work",
        &fixture_profile(),
        fixture_today(),
        &style,
        false,
    );
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_setup_complete() {
    let values = SetupValues {
        profile: String::from("work"),
        jira_url: String::from("https://example.atlassian.net"),
        email: String::from("user@example.com"),
        tempo_token: String::from("tempo-token"),
        jira_token: String::from("jira-token"),
        tz: String::from("Europe/Berlin"),
        work_hours: WorkHours {
            start: String::from("09:00"),
            end: String::from("17:00"),
        },
        working_days: vec![
            WeekdayName::Mon,
            WeekdayName::Tue,
            WeekdayName::Wed,
            WeekdayName::Thu,
            WeekdayName::Fri,
        ],
        time_format: TimeFormat::TwentyFourHour,
    };
    let dirs = AppDirs {
        config: std::path::PathBuf::from("/home/user/.config/logit"),
        data: std::path::PathBuf::from("/home/user/.local/share/logit"),
        cache: std::path::PathBuf::from("/home/user/.cache/logit"),
    };
    let style = Style::plain();
    let text = render_setup_complete(&values, &dirs, &style);
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_doctor_with_dotfile_sync() {
    let style = Style::plain();
    let text = render_doctor(
        std::path::Path::new("/home/user/.dotfiles/logit/config.toml"),
        std::path::Path::new("/home/user/.local/share/logit"),
        std::path::Path::new("/home/user/.cache/logit"),
        Some(1),
        1,
        Some(("work", &fixture_profile())),
        &style,
    );
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_doctor_schema_mismatch() {
    let style = Style::plain();
    let text = render_doctor(
        std::path::Path::new("/home/user/.config/logit/config.toml"),
        std::path::Path::new("/home/user/.local/share/logit"),
        std::path::Path::new("/home/user/.cache/logit"),
        Some(1),
        2,
        None,
        &style,
    );
    insta::assert_snapshot!(text);
}

fn week_report() -> StatReport {
    let mut rows = Vec::new();
    for offset in 0..7i64 {
        let date = NaiveDate::from_ymd_opt(2026, 3, 30).unwrap() + chrono::Duration::days(offset);
        let weekday = date.weekday();
        let expected = if matches!(weekday, chrono::Weekday::Sat | chrono::Weekday::Sun) {
            0
        } else {
            8 * 3600
        };
        let (filled, worklogs) = match offset {
            0 => (
                8 * 3600,
                vec![
                    WorklogLine {
                        issue_key: String::from("TK-1"),
                        duration_seconds: 6 * 3600,
                        start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                        end: NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                        description: Some(String::from("api rework")),
                    },
                    WorklogLine {
                        issue_key: String::from("TK-2"),
                        duration_seconds: 2 * 3600,
                        start: NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                        end: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                        description: None,
                    },
                ],
            ),
            1 => (
                4 * 3600 + 30 * 60,
                vec![WorklogLine {
                    issue_key: String::from("TK-3"),
                    duration_seconds: 4 * 3600 + 30 * 60,
                    start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    end: NaiveTime::from_hms_opt(13, 30, 0).unwrap(),
                    description: None,
                }],
            ),
            2 => (0, Vec::new()),
            3 => (0, Vec::new()),
            4 => (
                8 * 3600,
                vec![WorklogLine {
                    issue_key: String::from("TK-1"),
                    duration_seconds: 8 * 3600,
                    start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    end: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                    description: None,
                }],
            ),
            _ => (0, Vec::new()),
        };
        let holiday_name = if offset == 3 {
            Some(String::from("Founders Day"))
        } else {
            None
        };
        let source = if offset < 4 {
            ExpectedTimeSource::TempoWorkSchedule
        } else {
            ExpectedTimeSource::ConfiguredWorkday
        };
        rows.push(StatRow {
            date,
            source,
            worklogs,
            expected_seconds: if holiday_name.is_some() { 0 } else { expected },
            filled_seconds: filled,
            holiday_name,
        });
    }
    StatReport {
        label: String::from("week"),
        rows,
        notes: Vec::new(),
    }
}

#[test]
fn snapshot_stats_week() {
    let report = week_report();
    let style = Style::plain();
    let text = render_stats(
        &report,
        &StatSelector::Week,
        &fixture_profile(),
        "work",
        NaiveDate::from_ymd_opt(2026, 3, 30).unwrap(),
        NaiveDate::from_ymd_opt(2026, 4, 5).unwrap(),
        fixture_today(),
        false,
        &style,
        false,
    );
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_stats_today_card() {
    let report = StatReport {
        label: String::from("today"),
        rows: vec![StatRow {
            date: fixture_today(),
            source: ExpectedTimeSource::TempoWorkSchedule,
            worklogs: vec![
                WorklogLine {
                    issue_key: String::from("TK-1"),
                    duration_seconds: 3600,
                    start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    end: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                    description: Some(String::from("standup")),
                },
                WorklogLine {
                    issue_key: String::from("TK-9"),
                    duration_seconds: 7200,
                    start: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                    end: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
                    description: None,
                },
            ],
            expected_seconds: 8 * 3600,
            filled_seconds: 3 * 3600,
            holiday_name: None,
        }],
        notes: Vec::new(),
    };
    let style = Style::plain();
    let text = render_stats(
        &report,
        &StatSelector::Today,
        &fixture_profile(),
        "work",
        fixture_today(),
        fixture_today(),
        fixture_today(),
        false,
        &style,
        false,
    );
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_stats_today_empty() {
    let report = StatReport {
        label: String::from("today"),
        rows: vec![StatRow {
            date: fixture_today(),
            source: ExpectedTimeSource::ConfiguredWorkday,
            worklogs: Vec::new(),
            expected_seconds: 8 * 3600,
            filled_seconds: 0,
            holiday_name: None,
        }],
        notes: Vec::new(),
    };
    let style = Style::plain();
    let text = render_stats(
        &report,
        &StatSelector::Today,
        &fixture_profile(),
        "work",
        fixture_today(),
        fixture_today(),
        fixture_today(),
        false,
        &style,
        false,
    );
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_stats_year() {
    let mut rows = Vec::new();
    let mut current = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
    while current <= end {
        let weekday = current.weekday();
        let expected = if matches!(weekday, chrono::Weekday::Sat | chrono::Weekday::Sun) {
            0
        } else {
            8 * 3600
        };
        let filled = if current.month() <= 4 { expected } else { 0 };
        rows.push(StatRow {
            date: current,
            source: ExpectedTimeSource::ConfiguredWorkday,
            worklogs: Vec::new(),
            expected_seconds: expected,
            filled_seconds: filled,
            holiday_name: None,
        });
        current += chrono::Duration::days(1);
    }
    let report = StatReport {
        label: String::from("2026"),
        rows,
        notes: Vec::new(),
    };
    let style = Style::plain();
    let text = render_stats(
        &report,
        &StatSelector::Year(2026),
        &fixture_profile(),
        "work",
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        end,
        fixture_today(),
        false,
        &style,
        false,
    );
    insta::assert_snapshot!(text);
}

#[test]
fn stat_week_details_includes_worklog_lines() {
    let report = week_report();
    let style = Style::plain();
    let text = render_stats(
        &report,
        &StatSelector::Week,
        &fixture_profile(),
        "work",
        NaiveDate::from_ymd_opt(2026, 3, 30).unwrap(),
        NaiveDate::from_ymd_opt(2026, 4, 5).unwrap(),
        fixture_today(),
        true,
        &style,
        false,
    );

    assert!(text.contains("09:00 -> 15:00  TK-1  6h  api rework"));
    assert!(text.contains("15:00 -> 17:00  TK-2  2h"));
}

#[test]
fn snapshot_aliases_populated() {
    let mut profile = fixture_profile();
    profile.aliases.insert(
        String::from("standup"),
        Alias {
            key: String::from("TC-3"),
            default_duration: Some(1800),
            default_message: Some(String::from("daily standup")),
        },
    );
    profile.aliases.insert(
        String::from("review"),
        Alias {
            key: String::from("TK-9"),
            default_duration: Some(3600),
            default_message: None,
        },
    );
    let style = Style::plain();
    let text = render_aliases(&profile, "work", &style);
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_aliases_empty() {
    let style = Style::plain();
    let text = render_aliases(&fixture_profile(), "work", &style);
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_alias_set_replace() {
    let style = Style::plain();
    let prev = Alias {
        key: String::from("TK-old"),
        default_duration: None,
        default_message: None,
    };
    let text = render_alias_set("standup", "TC-3", Some(&prev), &style);
    insta::assert_snapshot!(text);
}

#[test]
fn snapshot_alias_delete_and_cache_clear() {
    let style = Style::plain();
    insta::assert_snapshot!(render_alias_delete("standup", &style));
    insta::assert_snapshot!(render_cache_clear("work", &style));
}

#[test]
fn snapshot_errors() {
    let style = Style::plain();
    insta::assert_snapshot!(render_error(
        &AppError::validation("missing duration after issue key"),
        &style
    ));
    insta::assert_snapshot!(render_error(
        &AppError::auth("Tempo token rejected"),
        &style
    ));
    insta::assert_snapshot!(render_error(
        &AppError::not_found("unknown issue key 'TK-999'"),
        &style
    ));
}

// Use unused import to keep clippy quiet on platforms without StatRangeInput usage.
#[allow(dead_code)]
fn _phantom_use(_: StatRangeInput) {}
