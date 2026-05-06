use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};

use crate::domain::TimeFormat;

pub fn format_duration(seconds: u32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    match (hours, minutes) {
        (0, 0) => String::from("0m"),
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

pub fn format_time(time: NaiveTime, format: &TimeFormat) -> String {
    match format {
        TimeFormat::TwentyFourHour => time.format("%H:%M").to_string(),
        TimeFormat::AmPm => time.format("%-I:%M %p").to_string(),
    }
}

pub fn format_datetime_short(
    datetime: NaiveDateTime,
    today: NaiveDate,
    format: &TimeFormat,
) -> String {
    format!(
        "{} {}",
        format_date_short(datetime.date(), today),
        format_time(datetime.time(), format)
    )
}

pub fn format_date_short(date: NaiveDate, today: NaiveDate) -> String {
    let weekday = match date.weekday() {
        chrono::Weekday::Mon => "Mon",
        chrono::Weekday::Tue => "Tue",
        chrono::Weekday::Wed => "Wed",
        chrono::Weekday::Thu => "Thu",
        chrono::Weekday::Fri => "Fri",
        chrono::Weekday::Sat => "Sat",
        chrono::Weekday::Sun => "Sun",
    };
    let month = month_short(date.month());
    if date.year() == today.year() {
        format!("{weekday} {month} {}", date.day())
    } else {
        format!("{weekday} {month} {} {}", date.day(), date.year())
    }
}

pub fn format_date_iso(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

pub fn format_range_header(label: &str, start: NaiveDate, end: NaiveDate) -> String {
    match label {
        "today" => format!("Today, {}", format_long_date(start)),
        "week" => format!(
            "This week — {} to {}",
            format_long_date(start),
            format_long_date(end)
        ),
        "last week" => format!(
            "Last week — {} to {}",
            format_long_date(start),
            format_long_date(end)
        ),
        other if other.len() == 4 && other.chars().all(|ch| ch.is_ascii_digit()) => {
            format!("Year {other}")
        }
        other if other.len() == 7 && other.as_bytes().get(4) == Some(&b'-') => {
            let parts: Vec<&str> = other.split('-').collect();
            if parts.len() == 2 {
                let year = parts[0];
                let month = parts[1]
                    .parse::<u32>()
                    .ok()
                    .map(month_long)
                    .unwrap_or(parts[1]);
                format!("{month} {year}")
            } else {
                other.to_owned()
            }
        }
        other => other.to_owned(),
    }
}

fn format_long_date(date: NaiveDate) -> String {
    format!(
        "{} {} {}",
        month_short(date.month()),
        date.day(),
        date.year()
    )
}

fn month_short(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "?",
    }
}

fn month_long(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "?",
    }
}

pub fn progress_bar(filled: u32, expected: u32, width: usize, unicode: bool) -> String {
    let (full, empty) = if unicode { ('█', '░') } else { ('#', '.') };
    if expected == 0 {
        return std::iter::repeat_n(empty, width).collect();
    }
    let ratio = (filled as f64) / (expected as f64);
    let filled_cells = (ratio * width as f64).round().min(width as f64) as usize;
    let mut bar = String::with_capacity(width * 4);
    for _ in 0..filled_cells {
        bar.push(full);
    }
    for _ in filled_cells..width {
        bar.push(empty);
    }
    bar
}

pub fn percent(filled: u32, expected: u32) -> Option<u32> {
    if expected == 0 {
        return None;
    }
    Some(filled.saturating_mul(100) / expected)
}

pub fn weekday_short(date: NaiveDate) -> &'static str {
    match date.weekday() {
        chrono::Weekday::Mon => "Mon",
        chrono::Weekday::Tue => "Tue",
        chrono::Weekday::Wed => "Wed",
        chrono::Weekday::Thu => "Thu",
        chrono::Weekday::Fri => "Fri",
        chrono::Weekday::Sat => "Sat",
        chrono::Weekday::Sun => "Sun",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formats_compactly() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(5400), "1h 30m");
    }

    #[test]
    fn time_formats_24h_and_ampm() {
        let t = NaiveTime::from_hms_opt(14, 30, 0).unwrap();
        assert_eq!(format_time(t, &TimeFormat::TwentyFourHour), "14:30");
        assert_eq!(format_time(t, &TimeFormat::AmPm), "2:30 PM");
    }

    #[test]
    fn date_omits_year_when_current() {
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        assert_eq!(format_date_short(date, today), "Wed Apr 1");
        let last_year = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();
        assert_eq!(format_date_short(last_year, today), "Mon Apr 1 2024");
    }

    #[test]
    fn progress_bar_full_and_partial() {
        assert_eq!(progress_bar(8, 8, 4, false), "####");
        assert_eq!(progress_bar(4, 8, 4, false), "##..");
        assert_eq!(progress_bar(0, 0, 4, false), "....");
        assert_eq!(progress_bar(8, 8, 4, true), "████");
    }

    #[test]
    fn percent_handles_zero_expected() {
        assert_eq!(percent(0, 0), None);
        assert_eq!(percent(4, 8), Some(50));
    }
}
