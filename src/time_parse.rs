use chrono::{LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

use crate::clock::Clock;
use crate::domain::{LogInput, LogKind, Profile, WorklogDraft};
use crate::error::AppError;

pub fn is_issue_key(token: &str) -> bool {
    let mut parts = token.split('-');
    let Some(prefix) = parts.next() else {
        return false;
    };
    let Some(number) = parts.next() else {
        return false;
    };
    if parts.next().is_some() || prefix.is_empty() || number.is_empty() {
        return false;
    }

    prefix.chars().enumerate().all(|(index, ch)| match index {
        0 => ch.is_ascii_uppercase(),
        _ => ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_',
    }) && number.chars().all(|ch| ch.is_ascii_digit())
}

pub fn parse_duration_tokens(tokens: &[String]) -> Result<(u32, usize), AppError> {
    let mut consumed = 0;
    let mut seconds = 0_u32;

    for token in tokens {
        let Some(unit) = token.chars().last() else {
            break;
        };
        let value = &token[..token.len().saturating_sub(1)];
        if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
            break;
        }

        let amount: u32 = value
            .parse()
            .map_err(|_| AppError::validation(format!("invalid duration: {token}")))?;
        if amount == 0 {
            return Err(AppError::validation("duration must be greater than zero"));
        }

        match unit {
            'h' => seconds = seconds.saturating_add(amount.saturating_mul(3600)),
            'm' => seconds = seconds.saturating_add(amount.saturating_mul(60)),
            _ => break,
        }

        consumed += 1;
    }

    if consumed == 0 {
        return Err(AppError::validation("missing duration"));
    }

    Ok((seconds, consumed))
}

pub fn build_worklog_draft<C: Clock>(
    input: &LogInput,
    profile: &Profile,
    clock: &C,
) -> Result<WorklogDraft, AppError> {
    match &input.kind {
        LogKind::Duration { seconds, date } => {
            build_duration_draft(input, profile, clock, *seconds, *date)
        }
        LogKind::Period { start, end } => build_period_draft(input, profile, *start, *end),
    }
}

pub fn parse_period_tokens(
    tokens: &[String],
) -> Result<(NaiveDateTime, NaiveDateTime, String), AppError> {
    let Some(separator) = tokens.iter().position(|token| token == "-") else {
        return Err(AppError::validation("missing period separator '-'"));
    };
    if separator < 2 || separator + 2 >= tokens.len() {
        return Err(AppError::validation("invalid period format"));
    }

    let issue = tokens
        .last()
        .cloned()
        .ok_or_else(|| AppError::validation("missing issue key or alias"))?;

    let start = parse_date_time(&tokens[..separator])?;
    let end = parse_date_time(&tokens[separator + 1..tokens.len() - 1])?;
    if end <= start {
        return Err(AppError::validation("end time must be after start time"));
    }
    if start.date() != end.date() {
        return Err(AppError::validation(
            "cross-midnight periods are not supported",
        ));
    }

    Ok((start, end, issue))
}

pub fn parse_date_override(value: &str) -> Result<NaiveDate, AppError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(value, "%m/%d/%Y"))
        .map_err(|_| AppError::validation(format!("invalid date: {value}")))
}

fn build_duration_draft<C: Clock>(
    input: &LogInput,
    profile: &Profile,
    clock: &C,
    seconds: Option<u32>,
    date: Option<NaiveDate>,
) -> Result<WorklogDraft, AppError> {
    let duration_seconds = seconds.ok_or_else(|| {
        AppError::validation(format!(
            "duration required for alias '{}'",
            input.issue_token
        ))
    })?;

    let end = if let Some(date) = date {
        let end_time = NaiveTime::parse_from_str(&profile.work_hours.end, "%H:%M")
            .map_err(|_| AppError::config("invalid work_hours.end in config"))?;
        NaiveDateTime::new(date, end_time)
    } else {
        clock.now()
    };
    let start = end - chrono::Duration::seconds(i64::from(duration_seconds));

    Ok(WorklogDraft {
        issue_key: input.issue_token.clone(),
        start,
        end,
        duration_seconds,
        timezone: profile.tz.clone(),
        description: normalize_description(input.description.as_deref()),
    })
}

fn build_period_draft(
    input: &LogInput,
    profile: &Profile,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<WorklogDraft, AppError> {
    validate_profile_local_datetime(profile, start)?;
    validate_profile_local_datetime(profile, end)?;
    let seconds = (end - start).num_seconds();
    if seconds <= 0 {
        return Err(AppError::validation("end time must be after start time"));
    }

    Ok(WorklogDraft {
        issue_key: input.issue_token.clone(),
        start,
        end,
        duration_seconds: seconds as u32,
        timezone: profile.tz.clone(),
        description: normalize_description(input.description.as_deref()),
    })
}

fn normalize_description(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn validate_profile_local_datetime(
    profile: &Profile,
    value: NaiveDateTime,
) -> Result<(), AppError> {
    let Some(timezone) = crate::domain::parse_timezone(&profile.tz) else {
        if crate::domain::parse_fixed_offset(&profile.tz).is_some() {
            return Ok(());
        }
        return Err(AppError::config(format!(
            "invalid timezone: {}",
            profile.tz
        )));
    };
    match timezone.from_local_datetime(&value) {
        LocalResult::Single(_) => Ok(()),
        LocalResult::Ambiguous(_, _) => Err(AppError::validation(format!(
            "ambiguous local time in timezone {}: {}",
            profile.tz, value
        ))),
        LocalResult::None => Err(AppError::validation(format!(
            "nonexistent local time in timezone {}: {}",
            profile.tz, value
        ))),
    }
}

fn parse_date_time(tokens: &[String]) -> Result<NaiveDateTime, AppError> {
    let Some(date_token) = tokens.first() else {
        return Err(AppError::validation("missing date token"));
    };
    let date = NaiveDate::parse_from_str(date_token, "%m/%d/%Y")
        .map_err(|_| AppError::validation(format!("invalid date: {date_token}")))?;
    let time = parse_time_tokens(&tokens[1..])?;
    Ok(NaiveDateTime::new(date, time))
}

fn parse_time_tokens(tokens: &[String]) -> Result<NaiveTime, AppError> {
    if tokens.is_empty() {
        return Err(AppError::validation("missing time token"));
    }

    let meridiem = tokens
        .last()
        .and_then(|token| match token.to_ascii_lowercase().as_str() {
            "am" => Some(false),
            "pm" => Some(true),
            _ => None,
        });

    let time_tokens = if meridiem.is_some() {
        &tokens[..tokens.len() - 1]
    } else {
        tokens
    };

    let (hour, minute) = match time_tokens {
        [compact]
            if compact.chars().all(|ch| ch.is_ascii_digit())
                && (compact.len() == 3 || compact.len() == 4) =>
        {
            let split = compact.len() - 2;
            let hour: u32 = compact[..split]
                .parse()
                .map_err(|_| AppError::validation(format!("invalid time: {compact}")))?;
            let minute: u32 = compact[split..]
                .parse()
                .map_err(|_| AppError::validation(format!("invalid time: {compact}")))?;
            (hour, minute)
        }
        [hour, minute]
            if hour.chars().all(|ch| ch.is_ascii_digit())
                && minute.chars().all(|ch| ch.is_ascii_digit()) =>
        {
            let hour: u32 = hour
                .parse()
                .map_err(|_| AppError::validation(format!("invalid time: {hour} {minute}")))?;
            let minute: u32 = minute
                .parse()
                .map_err(|_| AppError::validation(format!("invalid time: {hour} {minute}")))?;
            (hour, minute)
        }
        _ => return Err(AppError::validation("invalid time format")),
    };

    let hour = match meridiem {
        Some(false) => {
            if hour == 12 {
                0
            } else {
                hour
            }
        }
        Some(true) => {
            if hour == 12 {
                12
            } else {
                hour + 12
            }
        }
        None => hour,
    };

    NaiveTime::from_hms_opt(hour, minute, 0)
        .ok_or_else(|| AppError::validation("invalid time value"))
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveDateTime};

    use super::*;
    use crate::clock::FixedClock;
    use crate::config::default_profile;
    use crate::domain::{LogInput, PathOverrides};

    #[test]
    fn parses_duration_sequences() {
        let tokens = vec![
            String::from("8h"),
            String::from("15m"),
            String::from("TK-1"),
        ];
        let (seconds, consumed) = parse_duration_tokens(&tokens).expect("duration parses");

        assert_eq!(seconds, 8 * 3600 + 15 * 60);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn parses_compact_and_spaced_period_times() {
        let compact = vec![
            String::from("04/01/2026"),
            String::from("812"),
            String::from("-"),
            String::from("04/01/2026"),
            String::from("1700"),
            String::from("TK-1234"),
        ];
        let spaced = vec![
            String::from("04/01/2026"),
            String::from("8"),
            String::from("12"),
            String::from("am"),
            String::from("-"),
            String::from("04/01/2026"),
            String::from("5"),
            String::from("00"),
            String::from("pm"),
            String::from("TK-1234"),
        ];

        let (start_compact, end_compact, _) =
            parse_period_tokens(&compact).expect("compact parses");
        let (start_spaced, end_spaced, _) = parse_period_tokens(&spaced).expect("spaced parses");

        assert_eq!(start_compact, start_spaced);
        assert_eq!(end_compact, end_spaced);
    }

    #[test]
    fn builds_duration_draft_from_clock() {
        let profile = default_profile("UTC");
        let input = LogInput {
            profile: String::from("default"),
            paths: PathOverrides::default(),
            issue_token: String::from("TK-1"),
            description: None,
            dry_run: false,
            force: false,
            kind: LogKind::Duration {
                seconds: Some(3600),
                date: None,
            },
        };
        let clock = FixedClock::new(
            NaiveDate::from_ymd_opt(2026, 4, 1)
                .unwrap()
                .and_hms_opt(17, 0, 0)
                .unwrap(),
        );

        let draft = build_worklog_draft(&input, &profile, &clock).expect("draft builds");

        assert_eq!(
            draft.start,
            NaiveDateTime::parse_from_str("2026-04-01 16:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
        assert_eq!(
            draft.end,
            NaiveDateTime::parse_from_str("2026-04-01 17:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
    }

    #[test]
    fn rejects_dst_gap_period_for_profile_timezone() {
        let profile = default_profile("America/New_York");
        let input = LogInput {
            profile: String::from("default"),
            paths: PathOverrides::default(),
            issue_token: String::from("TK-1"),
            description: None,
            dry_run: false,
            force: false,
            kind: LogKind::Period {
                start: NaiveDate::from_ymd_opt(2026, 3, 8)
                    .unwrap()
                    .and_hms_opt(2, 30, 0)
                    .unwrap(),
                end: NaiveDate::from_ymd_opt(2026, 3, 8)
                    .unwrap()
                    .and_hms_opt(3, 30, 0)
                    .unwrap(),
            },
        };

        let error = build_worklog_draft(
            &input,
            &profile,
            &FixedClock::new(
                NaiveDate::from_ymd_opt(2026, 4, 1)
                    .unwrap()
                    .and_hms_opt(17, 0, 0)
                    .unwrap(),
            ),
        )
        .expect_err("dst gap rejected");

        assert!(error.to_string().contains("nonexistent local time"));
    }
}
