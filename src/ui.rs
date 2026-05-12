use chrono::{Datelike, NaiveDate, NaiveDateTime};
use comfy_table::presets::NOTHING;
use comfy_table::{Cell, CellAlignment, ContentArrangement, Table};
use inquire::{MultiSelect, Password, Select, Text};

use crate::config::{default_config, load_config, save_config};
use crate::domain::{
    Alias, ExpectedTimeSource, Profile, SetupInput, SetupValues, StatReport, StatRow, StatSelector,
    TimeFormat, WeekdayName, WorklogDraft, WorklogResult,
};
use crate::error::AppError;
use crate::format::{
    format_date_iso, format_date_short, format_datetime_short, format_duration, format_time,
    percent, progress_bar, weekday_short,
};
use crate::jira::JiraClient;
use crate::paths::{AppDirs, resolve_dirs};
use crate::secrets::{FileSecretStore, ProfileSecrets, SecretStore};
use crate::style::Style;
use crate::tempo::TempoClient;

pub fn run_setup(input: SetupInput) -> Result<(), AppError> {
    let jira = crate::jira::HttpJiraClient::default();
    let tempo = crate::tempo::HttpTempoClient::default();
    let timezone = iana_time_zone::get_timezone().unwrap_or_else(|_| String::from("UTC"));
    let values = prompt_setup_values(&input.profile, &timezone)?;
    run_setup_with_clients(input, values, &jira, &tempo)
}

pub fn run_setup_with_clients<J: JiraClient, T: TempoClient>(
    input: SetupInput,
    values: SetupValues,
    jira: &J,
    tempo: &T,
) -> Result<(), AppError> {
    let dirs = resolve_dirs(&input.paths)?;
    let existing_config = std::fs::read(dirs.config_file()).ok();
    let existing_secrets = std::fs::read(dirs.secrets_file()).ok();

    if !crate::domain::is_valid_timezone(&values.tz) {
        return Err(AppError::validation(format!(
            "invalid timezone: {}",
            values.tz
        )));
    }

    tempo.validate_token(&values.tempo_token)?;
    let account_id =
        jira.validate_credentials(&values.jira_url, &values.email, &values.jira_token)?;

    let mut config = if dirs.config_file().exists() {
        load_config(&dirs)?
    } else {
        default_config(&values.tz)
    };

    config.active = values.profile.clone();
    config.profiles.insert(
        values.profile.clone(),
        Profile {
            jira_url: values.jira_url.clone(),
            email: values.email.clone(),
            account_id: Some(account_id),
            tz: values.tz.clone(),
            work_hours: values.work_hours.clone(),
            working_days: values.working_days.clone(),
            time_format: values.time_format.clone(),
            aliases: config
                .profiles
                .get(&values.profile)
                .map(|profile| profile.aliases.clone())
                .unwrap_or_default(),
        },
    );

    let store = FileSecretStore::new(dirs.clone())?;
    if let Err(error) = store.save_profile(
        &values.profile,
        &ProfileSecrets {
            tempo_token: values.tempo_token.clone(),
            jira_token: values.jira_token.clone(),
        },
    ) {
        cleanup_setup_artifacts(
            &dirs,
            existing_config.as_deref(),
            existing_secrets.as_deref(),
        )?;
        return Err(error);
    }
    if let Err(error) = save_config(&dirs, &config) {
        cleanup_setup_artifacts(
            &dirs,
            existing_config.as_deref(),
            existing_secrets.as_deref(),
        )?;
        return Err(error);
    }

    let style = Style::for_stdout();
    println!("{}", render_setup_complete(&values, &dirs, &style));
    Ok(())
}

pub fn render_setup_complete(values: &SetupValues, dirs: &AppDirs, style: &Style) -> String {
    let header = format!(
        "{} {} {} profile \"{}\"",
        style.green(style.check()),
        style.bold("Setup complete"),
        style.dim(style.dot()),
        values.profile
    );
    let working_days: Vec<String> = values
        .working_days
        .iter()
        .map(|day| weekday_label(day).to_string())
        .collect();
    let rows: Vec<(&str, String)> = vec![
        ("Jira", values.jira_url.clone()),
        ("Email", values.email.clone()),
        ("Timezone", values.tz.clone()),
        (
            "Hours",
            format!(
                "{} {} {}",
                values.work_hours.start,
                style.arrow(),
                values.work_hours.end
            ),
        ),
        ("Days", working_days.join(", ")),
        (
            "Format",
            match values.time_format {
                TimeFormat::TwentyFourHour => "24h".to_string(),
                TimeFormat::AmPm => "AM/PM".to_string(),
            },
        ),
    ];
    let mut out = render_card(style, &header, &rows);
    out.push('\n');
    out.push_str(&format!(
        "  {} {} stored at {}",
        style.yellow(style.warn()),
        style.bold("Secrets"),
        style.dim(&dirs.secrets_file().display().to_string())
    ));
    out.push('\n');
    out.push_str(&format!(
        "    {}",
        style.dim("Keep this file out of dotfile sync.")
    ));
    out
}

fn weekday_label(day: &WeekdayName) -> &'static str {
    match day {
        WeekdayName::Mon => "Mon",
        WeekdayName::Tue => "Tue",
        WeekdayName::Wed => "Wed",
        WeekdayName::Thu => "Thu",
        WeekdayName::Fri => "Fri",
        WeekdayName::Sat => "Sat",
        WeekdayName::Sun => "Sun",
    }
}

fn cleanup_setup_artifacts(
    dirs: &crate::paths::AppDirs,
    config_before: Option<&[u8]>,
    secrets_before: Option<&[u8]>,
) -> Result<(), AppError> {
    restore_or_remove(&dirs.config_file(), config_before)?;
    restore_or_remove(&dirs.secrets_file(), secrets_before)?;
    Ok(())
}

fn restore_or_remove(path: &std::path::Path, previous: Option<&[u8]>) -> Result<(), AppError> {
    match previous {
        Some(bytes) => crate::atomic::atomic_write(path, bytes),
        None if path.exists() => std::fs::remove_file(path)
            .map_err(|error| AppError::config(format!("cleanup {}: {error}", path.display()))),
        None => Ok(()),
    }
}

pub fn default_setup_values(profile: &str, timezone: &str) -> SetupValues {
    SetupValues {
        profile: profile.to_owned(),
        jira_url: String::new(),
        email: String::new(),
        tempo_token: String::new(),
        jira_token: String::new(),
        tz: timezone.to_owned(),
        work_hours: crate::domain::WorkHours {
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
    }
}

pub fn setup_values_from_env(profile: &str, timezone: &str) -> SetupValues {
    let mut values = default_setup_values(profile, timezone);
    values.jira_url = std::env::var("LOGIT_SETUP_JIRA_URL")
        .unwrap_or_else(|_| String::from("https://example.atlassian.net"));
    values.email = std::env::var("LOGIT_SETUP_JIRA_EMAIL")
        .unwrap_or_else(|_| String::from("user@example.com"));
    values.tempo_token = std::env::var("LOGIT_SETUP_TEMPO_TOKEN")
        .unwrap_or_else(|_| String::from("tempo-token-placeholder"));
    values.jira_token = std::env::var("LOGIT_SETUP_JIRA_TOKEN")
        .unwrap_or_else(|_| String::from("jira-token-placeholder"));
    values
}

fn prompt_setup_values(profile: &str, timezone: &str) -> Result<SetupValues, AppError> {
    let env_values = setup_values_from_env(profile, timezone);
    if std::env::var_os("LOGIT_SETUP_JIRA_URL").is_some()
        || std::env::var_os("LOGIT_SETUP_JIRA_EMAIL").is_some()
        || std::env::var_os("LOGIT_SETUP_TEMPO_TOKEN").is_some()
        || std::env::var_os("LOGIT_SETUP_JIRA_TOKEN").is_some()
    {
        return Ok(env_values);
    }

    let jira_url = Text::new("Jira base URL")
        .with_placeholder("https://your-company.atlassian.net")
        .prompt()
        .map_err(|error| AppError::config(format!("setup canceled: {error}")))?;
    let email = Text::new("Jira account email")
        .with_placeholder("name@example.com")
        .prompt()
        .map_err(|error| AppError::config(format!("setup canceled: {error}")))?;
    let tempo_token = Password::new("Tempo API token")
        .without_confirmation()
        .with_display_toggle_enabled()
        .prompt()
        .map_err(|error| AppError::config(format!("setup canceled: {error}")))?;
    let jira_token = Password::new("Jira API token")
        .without_confirmation()
        .with_display_toggle_enabled()
        .prompt()
        .map_err(|error| AppError::config(format!("setup canceled: {error}")))?;
    let tz = Text::new("Time zone")
        .with_default(timezone)
        .prompt()
        .map_err(|error| AppError::config(format!("setup canceled: {error}")))?;
    let work_start = Text::new("Workday start (HH:MM)")
        .with_default(&env_values.work_hours.start)
        .prompt()
        .map_err(|error| AppError::config(format!("setup canceled: {error}")))?;
    let work_end = Text::new("Workday end (HH:MM)")
        .with_default(&env_values.work_hours.end)
        .prompt()
        .map_err(|error| AppError::config(format!("setup canceled: {error}")))?;
    let weekday_options = vec!["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let working_days = MultiSelect::new("Working days", weekday_options.clone())
        .with_default(&[0, 1, 2, 3, 4])
        .without_filtering()
        .prompt()
        .map_err(|error| AppError::config(format!("setup canceled: {error}")))?;
    let time_format = Select::new("Time format", vec!["24h", "AM/PM"])
        .without_filtering()
        .prompt()
        .map_err(|error| AppError::config(format!("setup canceled: {error}")))?;

    Ok(SetupValues {
        profile: profile.to_owned(),
        jira_url,
        email,
        tempo_token,
        jira_token,
        tz,
        work_hours: crate::domain::WorkHours {
            start: work_start,
            end: work_end,
        },
        working_days: working_days
            .into_iter()
            .map(|value| match value {
                "Mon" => WeekdayName::Mon,
                "Tue" => WeekdayName::Tue,
                "Wed" => WeekdayName::Wed,
                "Thu" => WeekdayName::Thu,
                "Fri" => WeekdayName::Fri,
                "Sat" => WeekdayName::Sat,
                _ => WeekdayName::Sun,
            })
            .collect(),
        time_format: if time_format == "24h" {
            TimeFormat::TwentyFourHour
        } else {
            TimeFormat::AmPm
        },
    })
}

pub fn render_log_success(
    result: &WorklogResult,
    profile: &Profile,
    today: NaiveDate,
    style: &Style,
    verbose: bool,
) -> String {
    let header = format!(
        "{} {} {} {} {} {}",
        style.green(style.check()),
        style.bold("Logged"),
        style.bold_cyan(&result.issue_key),
        style.dim(style.dot()),
        format_duration(result.duration_seconds),
        style.dim(""),
    )
    .trim_end()
    .to_owned();

    let rows = log_detail_rows(
        result.start,
        result.end,
        result.description.as_deref(),
        Some((&result.worklog_id, &result.tempo_url)),
        profile,
        today,
        style,
        verbose,
    );
    render_card(style, &header, &rows)
}

pub fn render_dry_run(
    draft: &WorklogDraft,
    profile_name: &str,
    profile: &Profile,
    today: NaiveDate,
    style: &Style,
    verbose: bool,
) -> String {
    let header = format!(
        "{} {} {} {} {} {} {}",
        style.yellow(style.dry_run_glyph()),
        style.bold("Dry-run"),
        style.bold_cyan(&draft.issue_key),
        style.dim(style.dot()),
        format_duration(draft.duration_seconds),
        style.dim(style.dot()),
        style.dim(&format!("profile {profile_name}, tz {}", draft.timezone)),
    );

    let rows = log_detail_rows(
        draft.start,
        draft.end,
        draft.description.as_deref(),
        None,
        profile,
        today,
        style,
        verbose,
    );
    render_card(style, &header, &rows)
}

#[allow(clippy::too_many_arguments)]
fn log_detail_rows(
    start: NaiveDateTime,
    end: NaiveDateTime,
    description: Option<&str>,
    worklog: Option<(&str, &str)>,
    profile: &Profile,
    today: NaiveDate,
    style: &Style,
    verbose: bool,
) -> Vec<(&'static str, String)> {
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    rows.push(("Date", format_date_short(start.date(), today)));
    let time_label = if start.date() == end.date() {
        format!(
            "{} {} {}",
            format_time(start.time(), &profile.time_format),
            style.arrow(),
            format_time(end.time(), &profile.time_format)
        )
    } else {
        format!(
            "{} {} {}",
            format_datetime_short(start, today, &profile.time_format),
            style.arrow(),
            format_datetime_short(end, today, &profile.time_format)
        )
    };
    rows.push(("Time", time_label));
    if let Some(message) = description {
        rows.push(("Message", message.to_owned()));
    }
    if let Some((id, url)) = worklog
        && verbose
    {
        rows.push(("Worklog", format!("#{id}  {}", style.dim(url))));
    } else if let Some((_, url)) = worklog {
        rows.push(("Worklog", url.to_owned()));
    }
    rows
}

pub fn render_card(style: &Style, header: &str, rows: &[(&str, String)]) -> String {
    let mut out = String::new();
    out.push_str(header);
    if !rows.is_empty() {
        let label_width = rows
            .iter()
            .map(|(label, _)| label.chars().count())
            .max()
            .unwrap_or(0);
        for (label, value) in rows {
            out.push('\n');
            let padding = " ".repeat(label_width.saturating_sub(label.chars().count()));
            out.push_str(&format!("  {}{}  {}", style.dim(label), padding, value));
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn render_stats(
    report: &StatReport,
    selector: &StatSelector,
    profile: &Profile,
    profile_name: &str,
    start: NaiveDate,
    end: NaiveDate,
    today: NaiveDate,
    details: bool,
    style: &Style,
    verbose: bool,
) -> String {
    match selector {
        StatSelector::Today => {
            render_stats_day(report, profile, profile_name, today, style, "Today")
        }
        StatSelector::Yesterday => {
            render_stats_day(report, profile, profile_name, today, style, "Yesterday")
        }
        StatSelector::Date(_) => {
            render_stats_day(report, profile, profile_name, today, style, "Day")
        }
        StatSelector::Week => render_stats_table(
            report,
            profile,
            profile_name,
            today,
            style,
            ViewMode::Week,
            details,
            verbose,
        ),
        StatSelector::LastWeek => render_stats_table(
            report,
            profile,
            profile_name,
            today,
            style,
            ViewMode::LastWeek,
            details,
            verbose,
        ),
        StatSelector::Month { .. } => render_stats_table(
            report,
            profile,
            profile_name,
            today,
            style,
            ViewMode::Month,
            details,
            verbose,
        ),
        StatSelector::Year(_) => render_stats_year(report, profile_name, start, end, style),
    }
}

#[derive(Clone, Copy)]
enum ViewMode {
    Week,
    LastWeek,
    Month,
}

fn render_stats_day(
    report: &StatReport,
    profile: &Profile,
    profile_name: &str,
    today: NaiveDate,
    style: &Style,
    title: &str,
) -> String {
    let row = report.rows.first();
    let date = row.map(|r| r.date).unwrap_or(today);
    let header = format!(
        "{} {} {}",
        style.bold(&format!("{title} {}", style.dim(style.dot()))),
        style.bold_cyan(&format_date_short(date, today)),
        style.dim(&format!("profile {profile_name}")),
    );
    let mut out = header;

    if let Some(row) = row {
        if let Some(holiday) = &row.holiday_name {
            out.push('\n');
            out.push_str(&format!(
                "  {} Holiday: {holiday}",
                style.yellow(style.warn()),
            ));
        }
        if row.worklogs.is_empty() {
            out.push('\n');
            out.push_str(&format!("  {}", style.dim("No worklogs yet today.")));
        } else {
            let mut table = base_table();
            table.set_header(header_cells(
                ["Time", "Issue", "Duration", "Message"],
                style,
            ));
            for line in &row.worklogs {
                let time = format!(
                    "{} {} {}",
                    format_time(line.start, &profile.time_format),
                    style.arrow(),
                    format_time(line.end, &profile.time_format)
                );
                let message = line
                    .description
                    .clone()
                    .unwrap_or_else(|| style.dash().to_owned());
                table.add_row(vec![
                    Cell::new(time),
                    Cell::new(style.bold_cyan(&line.issue_key)),
                    Cell::new(format_duration(line.duration_seconds))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(message),
                ]);
            }
            out.push('\n');
            out.push_str(&indent_block(&table.to_string(), 2));
        }
        out.push('\n');
        out.push_str(&render_total_line(
            row.filled_seconds,
            row.expected_seconds,
            style,
        ));
    }

    append_footer(&mut out, report, style);
    out
}

#[allow(clippy::too_many_arguments)]
fn render_stats_table(
    report: &StatReport,
    profile: &Profile,
    profile_name: &str,
    today: NaiveDate,
    style: &Style,
    mode: ViewMode,
    details: bool,
    verbose: bool,
) -> String {
    let label = match mode {
        ViewMode::Week => "This week",
        ViewMode::LastWeek => "Last week",
        ViewMode::Month => "Month",
    };
    let range_text = report_range(report, today);
    let header = format!(
        "{} {} {} {} {}",
        style.bold(label),
        style.dim(style.dot()),
        style.bold_cyan(&range_text),
        style.dim(style.dot()),
        style.dim(&format!("profile {profile_name}")),
    );
    let mut out = header;

    let mut table = base_table();
    table.set_header(header_cells(
        ["Date", "Filled", "Expected", "Progress", "Issues"],
        style,
    ));
    let mut visible_rows = 0;
    for row in &report.rows {
        if let Some(holiday) = &row.holiday_name {
            table.add_row(vec![
                Cell::new(format_date_short(row.date, today)),
                Cell::new(style.yellow(&format!("Holiday: {holiday}"))),
                Cell::new(""),
                Cell::new(""),
                Cell::new(""),
            ]);
            visible_rows += 1;
            continue;
        }
        if row.expected_seconds == 0 && row.filled_seconds == 0 && !verbose {
            continue;
        }

        let bar = progress_bar(row.filled_seconds, row.expected_seconds, 8, style.unicode);
        let pct = percent(row.filled_seconds, row.expected_seconds)
            .map(|value| format!("{value:>3}%"))
            .unwrap_or_else(|| style.dim("  —").to_owned());
        let bar_cell = format!(
            "{} {}",
            colorize_bar(&bar, row.filled_seconds, row.expected_seconds, style),
            pct
        );
        let filled_text = if row.filled_seconds == 0 {
            style.dim(style.dash()).to_owned()
        } else {
            format_duration(row.filled_seconds)
        };
        let expected_text = if row.expected_seconds == 0 {
            style.dim("off").to_owned()
        } else {
            format_duration(row.expected_seconds)
        };
        let issues_text = format_issue_summary(row, style);

        table.add_row(vec![
            Cell::new(format_date_short(row.date, today)),
            Cell::new(filled_text).set_alignment(CellAlignment::Right),
            Cell::new(expected_text).set_alignment(CellAlignment::Right),
            Cell::new(bar_cell),
            Cell::new(issues_text),
        ]);
        visible_rows += 1;
    }
    if visible_rows == 0 {
        out.push('\n');
        out.push_str(&format!("  {}", style.dim("No worklogs in this range.")));
    } else {
        out.push('\n');
        out.push_str(&indent_block(&table.to_string(), 2));
    }

    let total_filled: u32 = report.rows.iter().map(|row| row.filled_seconds).sum();
    let total_expected: u32 = report.rows.iter().map(|row| row.expected_seconds).sum();
    out.push('\n');
    out.push_str(&render_total_line(total_filled, total_expected, style));

    if details {
        append_detail_sections(&mut out, &report.rows, profile, today, style);
    }

    append_footer(&mut out, report, style);
    out
}

fn append_detail_sections(
    out: &mut String,
    rows: &[StatRow],
    profile: &Profile,
    today: NaiveDate,
    style: &Style,
) {
    for row in rows {
        if row.worklogs.is_empty() {
            continue;
        }
        out.push('\n');
        out.push('\n');
        out.push_str(&format!(
            "  {}",
            style.bold(&format_date_short(row.date, today))
        ));
        for worklog in &row.worklogs {
            out.push('\n');
            out.push_str(&format!(
                "    {}  {}  {}{}",
                style.dim(&format!(
                    "{} {} {}",
                    format_time(worklog.start, &profile.time_format),
                    style.arrow(),
                    format_time(worklog.end, &profile.time_format)
                )),
                style.bold_cyan(&worklog.issue_key),
                format_duration(worklog.duration_seconds),
                worklog
                    .description
                    .as_deref()
                    .map(|message| format!("  {}", style.dim(message)))
                    .unwrap_or_default(),
            ));
        }
    }
}

fn render_stats_year(
    report: &StatReport,
    profile_name: &str,
    start: NaiveDate,
    end: NaiveDate,
    style: &Style,
) -> String {
    let header = format!(
        "{} {} {} {} {}",
        style.bold("Year"),
        style.dim(style.dot()),
        style.bold_cyan(&format!("{}", start.year())),
        style.dim(style.dot()),
        style.dim(&format!("profile {profile_name}")),
    );
    let mut out = header;

    let mut by_month: std::collections::BTreeMap<u32, (u32, u32)> =
        std::collections::BTreeMap::new();
    for row in &report.rows {
        let entry = by_month.entry(row.date.month()).or_insert((0, 0));
        entry.0 += row.filled_seconds;
        entry.1 += row.expected_seconds;
    }

    let mut table = base_table();
    table.set_header(header_cells(
        ["Month", "Filled", "Expected", "Progress"],
        style,
    ));
    for month in 1..=12u32 {
        let (filled, expected) = *by_month.get(&month).unwrap_or(&(0, 0));
        let bar = progress_bar(filled, expected, 10, style.unicode);
        let pct = percent(filled, expected)
            .map(|value| format!("{value:>3}%"))
            .unwrap_or_else(|| style.dim("  —").to_owned());
        let bar_cell = format!("{} {}", colorize_bar(&bar, filled, expected, style), pct);
        let filled_text = if filled == 0 {
            style.dim(style.dash()).to_owned()
        } else {
            format_duration(filled)
        };
        let expected_text = if expected == 0 {
            style.dim(style.dash()).to_owned()
        } else {
            format_duration(expected)
        };
        let _ = end;
        table.add_row(vec![
            Cell::new(month_full(month)),
            Cell::new(filled_text).set_alignment(CellAlignment::Right),
            Cell::new(expected_text).set_alignment(CellAlignment::Right),
            Cell::new(bar_cell),
        ]);
    }
    out.push('\n');
    out.push_str(&indent_block(&table.to_string(), 2));

    let total_filled: u32 = report.rows.iter().map(|row| row.filled_seconds).sum();
    let total_expected: u32 = report.rows.iter().map(|row| row.expected_seconds).sum();
    out.push('\n');
    out.push_str(&render_total_line(total_filled, total_expected, style));
    append_footer(&mut out, report, style);
    out
}

fn render_total_line(filled: u32, expected: u32, style: &Style) -> String {
    let bar = progress_bar(filled, expected, 10, style.unicode);
    let pct = percent(filled, expected)
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| String::from("—"));
    let delta = i64::from(filled) - i64::from(expected);
    let delta_label = if expected == 0 {
        String::new()
    } else if delta == 0 {
        format!(" {} on target", style.green(style.check()))
    } else if delta < 0 {
        format!(
            " {} {} remaining",
            style.yellow(style.warn()),
            format_duration(delta.unsigned_abs() as u32)
        )
    } else {
        format!(" {} {} over", style.dim("·"), format_duration(delta as u32))
    };

    format!(
        "  {}  {} / {}  {} {}{}",
        style.bold("Total"),
        format_duration(filled),
        format_duration(expected),
        colorize_bar(&bar, filled, expected, style),
        pct,
        delta_label,
    )
}

fn append_footer(out: &mut String, report: &StatReport, style: &Style) {
    let mut sources: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for row in &report.rows {
        sources.insert(source_label(&row.source));
    }
    if !sources.is_empty() {
        out.push('\n');
        out.push('\n');
        let joined = sources.into_iter().collect::<Vec<_>>().join(", ");
        out.push_str(&format!(
            "  {} {}",
            style.dim("Source:"),
            style.dim(&joined)
        ));
    }
    for note in &report.notes {
        out.push('\n');
        out.push_str(&format!(
            "  {} {}",
            style.yellow(style.warn()),
            style.dim(note)
        ));
    }
}

fn source_label(source: &ExpectedTimeSource) -> String {
    match source {
        ExpectedTimeSource::TempoWorkSchedule => "Tempo work schedule".to_owned(),
        ExpectedTimeSource::ConfiguredWorkday => "configured workday".to_owned(),
        ExpectedTimeSource::StaleCache { days_old } => {
            format!("cached calendar ({days_old}d old)")
        }
    }
}

fn report_range(report: &StatReport, today: NaiveDate) -> String {
    let Some(first) = report.rows.first() else {
        return String::from("(empty)");
    };
    let Some(last) = report.rows.last() else {
        return String::from("(empty)");
    };
    if first.date == last.date {
        format_date_short(first.date, today)
    } else {
        format!(
            "{} – {}",
            format_date_short(first.date, today),
            format_date_short(last.date, today)
        )
    }
}

fn format_issue_summary(row: &StatRow, style: &Style) -> String {
    if row.worklogs.is_empty() {
        return style.dim(style.dash()).to_owned();
    }
    let mut totals: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for line in &row.worklogs {
        if !totals.contains_key(&line.issue_key) {
            order.push(line.issue_key.clone());
        }
        *totals.entry(line.issue_key.clone()).or_insert(0) += line.duration_seconds;
    }
    let parts: Vec<String> = order
        .into_iter()
        .map(|key| {
            let secs = totals.get(&key).copied().unwrap_or(0);
            format!("{} {}", style.cyan(&key), style.dim(&format_duration(secs)))
        })
        .collect();
    parts.join(", ")
}

fn colorize_bar(bar: &str, filled: u32, expected: u32, style: &Style) -> String {
    if expected == 0 {
        return style.dim(bar).to_owned();
    }
    let pct = percent(filled, expected).unwrap_or(0);
    if pct >= 100 {
        style.green(bar).to_owned()
    } else if pct >= 50 {
        style.yellow(bar).to_owned()
    } else {
        style.red(bar).to_owned()
    }
}

fn base_table() -> Table {
    let mut table = Table::new();
    table.load_preset(NOTHING);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table
}

fn header_cells<const N: usize>(labels: [&'static str; N], style: &Style) -> Vec<Cell> {
    labels
        .iter()
        .map(|label| Cell::new(style.dim(label)))
        .collect()
}

fn indent_block(text: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    text.lines()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn month_full(month: u32) -> &'static str {
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

pub fn render_doctor(
    config: &std::path::Path,
    data: &std::path::Path,
    cache: &std::path::Path,
    schema_version: Option<u32>,
    supported_schema_version: u32,
    active_profile: Option<(&str, &Profile)>,
    style: &Style,
) -> String {
    let detect = |path: &std::path::Path| -> (&'static str, bool) {
        let mut current = Some(path);
        while let Some(candidate) = current {
            if candidate.file_name().is_some_and(|name| {
                name == "dotfiles" || name == ".dotfiles" || name == "dotfiles.git"
            }) {
                return ("dotfile-sync detected", true);
            }
            if candidate.join(".git").exists() {
                return ("dotfile-sync detected", true);
            }
            current = candidate.parent();
        }
        ("local-only", false)
    };

    let header = format!(
        "{} {}",
        style.bold("logit doctor"),
        style.dim("· environment summary"),
    );

    let schema_status = match schema_version {
        Some(version) if version == supported_schema_version => format!(
            "v{version} {} {}",
            style.dim(style.dot()),
            style.green(&format!("{} ok", style.check())),
        ),
        Some(version) => format!(
            "v{version} {} {}",
            style.dim(style.dot()),
            style.red(&format!(
                "{} mismatch (supported v{supported_schema_version})",
                style.cross()
            )),
        ),
        None => style.dim("unknown").to_owned(),
    };

    let mut rows: Vec<(&str, String)> = vec![("Schema", schema_status)];
    for (label, path) in [("Config", config), ("Data", data), ("Cache", cache)] {
        let (text, warn) = detect(path);
        let glyph = if warn {
            format!(" {}", style.yellow(style.warn()))
        } else {
            String::new()
        };
        rows.push((
            label,
            format!("{}\n      {}{}", path.display(), style.dim(text), glyph),
        ));
    }
    if let Some((name, profile)) = active_profile {
        rows.push((
            "Profile",
            format!(
                "{} {} {}",
                name,
                style.dim(style.dot()),
                style.dim(&profile.tz)
            ),
        ));
    }

    render_card(style, &header, &rows)
}

pub fn render_aliases(profile: &Profile, profile_name: &str, style: &Style) -> String {
    let header = format!(
        "{} {} {}",
        style.bold("Aliases"),
        style.dim(style.dot()),
        style.dim(&format!("profile {profile_name}")),
    );
    if profile.aliases.is_empty() {
        let mut out = header;
        out.push('\n');
        out.push_str(&format!("  {}", style.dim("No aliases yet.")));
        out.push('\n');
        out.push_str(&format!(
            "  {} {}",
            style.dim("Add one with"),
            style.dim("logit alias <name> <ISSUE-KEY>"),
        ));
        return out;
    }

    let mut table = base_table();
    table.set_header(header_cells(
        ["Name", "Issue", "Duration", "Default message"],
        style,
    ));
    for (name, alias) in &profile.aliases {
        let duration = alias
            .default_duration
            .map(format_duration)
            .unwrap_or_else(|| style.dash().to_owned());
        let message = alias
            .default_message
            .clone()
            .unwrap_or_else(|| style.dash().to_owned());
        table.add_row(vec![
            Cell::new(name),
            Cell::new(style.bold_cyan(&alias.key)),
            Cell::new(duration),
            Cell::new(message),
        ]);
    }
    let mut out = header;
    out.push('\n');
    out.push_str(&indent_block(&table.to_string(), 2));
    out
}

pub fn render_alias_set(
    name: &str,
    issue_key: &str,
    previous: Option<&Alias>,
    style: &Style,
) -> String {
    match previous {
        Some(prev) => format!(
            "{} {} {} {} {} {} {}",
            style.green(style.check()),
            style.bold("Replaced"),
            style.bold(name),
            style.dim(":"),
            style.dim(&prev.key),
            style.arrow(),
            style.bold_cyan(issue_key)
        ),
        None => format!(
            "{} {} {} {} {}",
            style.green(style.check()),
            style.bold("Added"),
            style.bold(name),
            style.arrow(),
            style.bold_cyan(issue_key)
        ),
    }
}

pub fn render_alias_delete(name: &str, style: &Style) -> String {
    format!(
        "{} {} alias {}",
        style.green(style.check()),
        style.bold("Deleted"),
        style.bold(name)
    )
}

pub fn render_cache_clear(profile_name: &str, style: &Style) -> String {
    format!(
        "{} {} calendar cache for profile {}",
        style.green(style.check()),
        style.bold("Cleared"),
        style.bold(profile_name)
    )
}

pub fn render_error(error: &AppError, style: &Style) -> String {
    format!(
        "{} {} {}",
        style.red(style.cross()),
        style.bold(&format!("{}:", error_label(error))),
        error.message
    )
}

fn error_label(error: &AppError) -> &'static str {
    match error.category {
        "validation" => "Invalid input",
        "config" => "Config error",
        "auth" => "Auth error",
        "network" => "Network error",
        "not_found" => "Not found",
        "conflict" => "Conflict",
        _ => "Error",
    }
}

pub fn date_label(date: NaiveDate) -> String {
    format_date_iso(date)
}

#[allow(dead_code)]
pub fn render_stat_row(row: &StatRow) -> String {
    let _ = weekday_short(row.date);
    format!(
        "{} filled={} expected={}",
        format_date_iso(row.date),
        format_duration(row.filled_seconds),
        format_duration(row.expected_seconds),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ExpectedTimeSource, PathOverrides, WorkHours, WorklogLine};
    use crate::error::AppError;
    use crate::jira::JiraClient;
    use crate::tempo::TempoClient;

    #[derive(Debug)]
    struct TestJira;

    impl JiraClient for TestJira {
        fn validate_credentials(
            &self,
            _jira_url: &str,
            _email: &str,
            _token: &str,
        ) -> Result<String, AppError> {
            Ok(String::from("acct-1"))
        }

        fn resolve_issue_id(
            &self,
            _jira_url: &str,
            _email: &str,
            _token: &str,
            issue_key: &str,
        ) -> Result<String, AppError> {
            Ok(format!("issue-{issue_key}"))
        }

        fn resolve_issue_key(
            &self,
            _jira_url: &str,
            _email: &str,
            _token: &str,
            issue_id: &str,
        ) -> Result<String, AppError> {
            Ok(format!("TC-{issue_id}"))
        }
    }

    #[derive(Debug)]
    struct TestTempo;

    impl TempoClient for TestTempo {
        fn validate_token(&self, _tempo_token: &str) -> Result<(), AppError> {
            Ok(())
        }

        fn to_boundary_draft(
            &self,
            issue_id: String,
            author_account_id: String,
            draft: &crate::domain::WorklogDraft,
        ) -> crate::domain::WorklogBoundaryDraft {
            crate::domain::WorklogBoundaryDraft {
                issue_id,
                author_account_id,
                start_date: draft.start.date(),
                start_time: draft.start.time(),
                time_spent_seconds: draft.duration_seconds,
                description: draft.description.clone(),
            }
        }

        fn create_worklog(
            &self,
            _tempo_token: &str,
            profile: &crate::domain::Profile,
            draft: &crate::domain::WorklogBoundaryDraft,
        ) -> Result<crate::domain::WorklogResult, AppError> {
            let start = draft.start_date.and_time(draft.start_time);
            let end = start + chrono::Duration::seconds(i64::from(draft.time_spent_seconds));
            Ok(crate::domain::WorklogResult {
                worklog_id: String::from("worklog-1"),
                issue_key: draft.issue_id.clone(),
                issue_id: Some(draft.issue_id.clone()),
                start,
                end,
                duration_seconds: draft.time_spent_seconds,
                tempo_url: format!("{}/tempo/worklog/1", profile.jira_url),
                description: draft.description.clone(),
            })
        }

        fn list_worklogs(
            &self,
            _tempo_token: &str,
            _account_id: &str,
            _from: NaiveDate,
            _to: NaiveDate,
        ) -> Result<Vec<crate::domain::WorklogResult>, AppError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn setup_defaults_include_editable_timezone() {
        let values = default_setup_values("default", "UTC");

        assert_eq!(values.profile, "default");
        assert_eq!(values.tz, "UTC");
        assert_eq!(values.working_days.len(), 5);
    }

    #[test]
    fn renders_stats_with_source_label() {
        let profile = crate::config::default_profile("UTC");
        let style = Style::plain();
        let report = StatReport {
            label: String::from("week"),
            rows: vec![StatRow {
                date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                source: ExpectedTimeSource::ConfiguredWorkday,
                worklogs: vec![WorklogLine {
                    issue_key: String::from("TK-1"),
                    duration_seconds: 3600,
                    start: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    end: chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                    description: None,
                }],
                expected_seconds: 28_800,
                filled_seconds: 3_600,
                holiday_name: None,
            }],
            notes: vec![String::from("a note")],
        };

        let text = render_stats(
            &report,
            &StatSelector::Week,
            &profile,
            "default",
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            false,
            &style,
            false,
        );

        assert!(text.contains("configured workday"));
        assert!(text.contains("a note"));
    }

    #[test]
    fn renders_last_week_label() {
        let profile = crate::config::default_profile("UTC");
        let style = Style::plain();
        let report = StatReport {
            label: String::from("last week"),
            rows: vec![StatRow {
                date: NaiveDate::from_ymd_opt(2026, 3, 24).unwrap(),
                source: ExpectedTimeSource::ConfiguredWorkday,
                worklogs: Vec::new(),
                expected_seconds: 28_800,
                filled_seconds: 0,
                holiday_name: None,
            }],
            notes: Vec::new(),
        };

        let text = render_stats(
            &report,
            &StatSelector::LastWeek,
            &profile,
            "default",
            NaiveDate::from_ymd_opt(2026, 3, 24).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 30).unwrap(),
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            false,
            &style,
            false,
        );

        assert!(text.contains("Last week"));
        assert!(!text.contains("This week"));
    }

    #[test]
    fn render_doctor_marks_dotfile_like_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("dotfiles");
        std::fs::create_dir_all(root.join("logit")).expect("dotfiles tree");

        let style = Style::plain();
        let text = render_doctor(
            &root.join("logit"),
            &temp.path().join("local-data"),
            &temp.path().join("local-cache"),
            Some(1),
            1,
            None,
            &style,
        );

        assert!(text.contains("dotfile-sync detected"));
        assert!(text.contains("v1"));
    }

    #[test]
    fn setup_with_clients_persists_profile_and_secrets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = PathOverrides {
            config_dir: Some(temp.path().join("config")),
            data_dir: Some(temp.path().join("data")),
            cache_dir: Some(temp.path().join("cache")),
        };

        run_setup_with_clients(
            SetupInput {
                profile: String::from("default"),
                paths: paths.clone(),
            },
            SetupValues {
                profile: String::from("default"),
                jira_url: String::from("https://example.atlassian.net"),
                email: String::from("user@example.com"),
                tempo_token: String::from("tempo-token"),
                jira_token: String::from("jira-token"),
                tz: String::from("UTC"),
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
            },
            &TestJira,
            &TestTempo,
        )
        .expect("setup succeeds");

        let dirs = crate::paths::resolve_dirs(&paths).expect("dirs");
        let config = load_config(&dirs).expect("config loads");
        let store = FileSecretStore::new(dirs).expect("store");
        let secrets = store
            .load_profile("default")
            .expect("load secrets")
            .expect("secrets");

        assert_eq!(
            config.profiles["default"].account_id.as_deref(),
            Some("acct-1")
        );
        assert_eq!(secrets.tempo_token, "tempo-token");
    }
}
