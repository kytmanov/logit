use crate::domain::{
    AliasCommand, AliasDeleteInput, AliasSetInput, CacheCommand, CommandInput, ConfigCommand,
    DomainCommand, LogDateSpec, LogInput, LogKind, McpCommand, McpInstallInput, McpInstallTarget,
    ParsedCli, PathOverrides, ProfileSource, SetupInput, StatRangeInput, StatSelector,
};
use crate::error::AppError;
use crate::time_parse::{
    is_issue_key, parse_date_override, parse_duration_tokens, parse_log_date_spec,
    parse_period_tokens,
};

pub fn parse_cli(args: Vec<String>) -> Result<ParsedCli, AppError> {
    let mut state = ParseState::new(args);
    let verbose = state.take_flag("--verbose") || truthy_env("LOGIT_VERBOSE");

    let paths = PathOverrides {
        config_dir: state.take_option_value("--config-dir")?.map(Into::into),
        data_dir: state.take_option_value("--data-dir")?.map(Into::into),
        cache_dir: state.take_option_value("--cache-dir")?.map(Into::into),
    };
    let (profile, profile_source) = match state.take_option_value("--profile")? {
        Some(profile) => (profile, ProfileSource::Flag),
        None => match std::env::var("LOGIT_PROFILE") {
            Ok(profile) => (profile, ProfileSource::Env),
            Err(_) => (String::from("default"), ProfileSource::Default),
        },
    };

    let Some(command) = state.next() else {
        return Err(AppError::validation("missing command"));
    };

    let command = match command.as_str() {
        "setup" => DomainCommand::Setup(SetupInput { profile, paths }),
        "stat" => DomainCommand::Stat(parse_stat(profile, paths, &mut state)?),
        "alias" => DomainCommand::Alias(parse_alias(profile, paths, &mut state)?),
        "doctor" => DomainCommand::Doctor(CommandInput { profile, paths }),
        "config" => DomainCommand::Config(parse_config(profile, paths, &mut state)?),
        "cache" => DomainCommand::Cache(parse_cache(profile, paths, &mut state)?),
        "mcp" => DomainCommand::Mcp(parse_mcp(profile, profile_source, paths, &mut state)?),
        other => parse_log(profile, paths, other.to_owned(), &mut state)?,
    };

    if let Some(trailing) = state.peek() {
        return Err(AppError::validation(format!(
            "unexpected trailing argument: {trailing}"
        )));
    }

    Ok(ParsedCli { command, verbose })
}

fn parse_log(
    profile: String,
    paths: PathOverrides,
    first_token: String,
    state: &mut ParseState,
) -> Result<DomainCommand, AppError> {
    let mut tokens = vec![first_token];
    while let Some(next) = state.next() {
        tokens.push(next);
    }

    let dry_run = extract_flag(&mut tokens, "--dry-run") || truthy_env("LOGIT_DRY_RUN");
    let force = extract_flag(&mut tokens, "--force");
    let date = extract_option_value(&mut tokens, "--date")?
        .map(|value| parse_date_override(&value).map(LogDateSpec::Absolute))
        .transpose()?;
    let description = extract_message(&mut tokens)?;

    if let Some(trailing_date) = trailing_log_date(&tokens)? {
        if date.is_some() {
            return Err(AppError::validation(
                "cannot use both --date and a trailing date argument",
            ));
        }
        tokens.pop();
        return parse_log_tokens(
            profile,
            paths,
            tokens,
            Some(trailing_date),
            description,
            dry_run,
            force,
        );
    }

    parse_log_tokens(profile, paths, tokens, date, description, dry_run, force)
}

fn parse_log_tokens(
    profile: String,
    paths: PathOverrides,
    tokens: Vec<String>,
    date: Option<LogDateSpec>,
    description: Option<String>,
    dry_run: bool,
    force: bool,
) -> Result<DomainCommand, AppError> {
    if tokens.iter().any(|token| token == "-") {
        let (start, end, issue_token) = parse_period_tokens(&tokens)?;
        return Ok(DomainCommand::Log(LogInput {
            profile,
            paths,
            issue_token,
            description,
            dry_run,
            force,
            kind: LogKind::Period { start, end },
        }));
    }

    if tokens.len() == 1 {
        if is_issue_key(&tokens[0]) {
            return Err(AppError::validation("missing duration after issue key"));
        }

        return Ok(DomainCommand::Log(LogInput {
            profile,
            paths,
            issue_token: tokens[0].clone(),
            description,
            dry_run,
            force,
            kind: LogKind::Duration {
                seconds: None,
                date,
            },
        }));
    }

    if let Ok((seconds, consumed)) = parse_duration_tokens(&tokens[1..]) {
        if consumed != tokens.len() - 1 {
            return Err(AppError::validation("unexpected token after duration"));
        }

        return Ok(DomainCommand::Log(LogInput {
            profile,
            paths,
            issue_token: tokens[0].clone(),
            description,
            dry_run,
            force,
            kind: LogKind::Duration {
                seconds: Some(seconds),
                date,
            },
        }));
    }

    if is_issue_key(&tokens[0]) {
        return Err(AppError::validation("missing duration after issue key"));
    }

    let (seconds, consumed) = parse_duration_tokens(&tokens)?;
    if consumed == tokens.len() {
        return Err(AppError::validation(
            "missing issue key or alias after duration",
        ));
    }
    if consumed + 1 != tokens.len() {
        return Err(AppError::validation(
            "unexpected trailing argument after issue key",
        ));
    }

    Ok(DomainCommand::Log(LogInput {
        profile,
        paths,
        issue_token: tokens[consumed].clone(),
        description,
        dry_run,
        force,
        kind: LogKind::Duration {
            seconds: Some(seconds),
            date,
        },
    }))
}

fn trailing_log_date(tokens: &[String]) -> Result<Option<LogDateSpec>, AppError> {
    let Some(last) = tokens.last() else {
        return Ok(None);
    };
    if tokens.len() < 2 {
        return Ok(None);
    }
    if tokens.len() == 2 && parse_duration_tokens(&tokens[..1]).is_ok() {
        return Ok(None);
    }
    match parse_log_date_spec(last) {
        Ok(date) => Ok(Some(date)),
        Err(error) if error.to_string().contains("invalid date:") => Ok(None),
        Err(error) => Err(error),
    }
}

fn parse_stat(
    profile: String,
    paths: PathOverrides,
    state: &mut ParseState,
) -> Result<StatRangeInput, AppError> {
    let details = state.take_flag("--details");
    let refresh_calendar = state.take_flag("--refresh-calendar");
    let no_calendar = state.take_flag("--no-calendar");

    let selector = match state.next() {
        None => StatSelector::Today,
        Some(value) if value.eq_ignore_ascii_case("today") => StatSelector::Today,
        Some(value) if value.eq_ignore_ascii_case("yesterday") => StatSelector::Yesterday,
        Some(value) if value.eq_ignore_ascii_case("week") => StatSelector::Week,
        Some(value) if value.eq_ignore_ascii_case("last") => {
            let Some(next) = state.next() else {
                return Err(AppError::validation("expected 'week' after 'last'"));
            };
            if next.eq_ignore_ascii_case("week") {
                StatSelector::LastWeek
            } else {
                return Err(AppError::validation(format!(
                    "unknown stat selector: last {next}"
                )));
            }
        }
        Some(value) if value.len() == 4 && value.chars().all(|ch| ch.is_ascii_digit()) => {
            StatSelector::Year(value.parse().expect("digit-checked"))
        }
        Some(value) => {
            if let Ok(date) = parse_date_override(&value) {
                StatSelector::Date(date)
            } else {
                let month = parse_month(&value)?;
                if let Some(next) = state.peek() {
                    if next.len() == 4 && next.chars().all(|ch| ch.is_ascii_digit()) {
                        let year: i32 = state
                            .next()
                            .expect("peeked")
                            .parse()
                            .expect("digit-checked");
                        StatSelector::Month { month, year }
                    } else {
                        StatSelector::Month { month, year: 0 }
                    }
                } else {
                    StatSelector::Month { month, year: 0 }
                }
            }
        }
    };

    Ok(StatRangeInput {
        profile,
        paths,
        selector,
        details,
        refresh_calendar,
        no_calendar,
    })
}

fn parse_alias(
    profile: String,
    paths: PathOverrides,
    state: &mut ParseState,
) -> Result<AliasCommand, AppError> {
    let Some(first) = state.next() else {
        return Err(AppError::validation("missing alias subcommand or name"));
    };

    if first == "list" && state.peek().is_none() {
        return Ok(AliasCommand::List(CommandInput { profile, paths }));
    }

    if first == "delete" && state.remaining_len() == 1 {
        let Some(name) = state.next() else {
            return Err(AppError::validation("missing alias name for delete"));
        };
        return Ok(AliasCommand::Delete(AliasDeleteInput {
            profile,
            paths,
            name,
        }));
    }

    let Some(issue_key) = state.next() else {
        return Err(AppError::validation("missing issue key for alias"));
    };

    let default_duration = state
        .take_option_value("--default-duration")?
        .map(|value| parse_duration_minutes(&value))
        .transpose()?;
    let default_message = if let Some(message) = state.take_option_value("-m")? {
        Some(message)
    } else {
        state.take_option_value("--message")?
    };
    let validate_remote = !state.take_flag("--no-validate");

    Ok(AliasCommand::Set(AliasSetInput {
        profile,
        paths,
        name: first,
        issue_key,
        default_duration,
        default_message,
        validate_remote,
    }))
}

fn parse_config(
    profile: String,
    paths: PathOverrides,
    state: &mut ParseState,
) -> Result<ConfigCommand, AppError> {
    match state.next().as_deref() {
        Some("path") => Ok(ConfigCommand::Path(CommandInput { profile, paths })),
        Some("edit") => Ok(ConfigCommand::Edit(CommandInput { profile, paths })),
        Some(other) => Err(AppError::validation(format!(
            "unknown config subcommand: {other}"
        ))),
        None => Err(AppError::validation("missing config subcommand")),
    }
}

fn parse_cache(
    profile: String,
    paths: PathOverrides,
    state: &mut ParseState,
) -> Result<CacheCommand, AppError> {
    match state.next().as_deref() {
        Some("clear") => Ok(CacheCommand::Clear(CommandInput { profile, paths })),
        Some(other) => Err(AppError::validation(format!(
            "unknown cache subcommand: {other}"
        ))),
        None => Err(AppError::validation("missing cache subcommand")),
    }
}

fn parse_mcp(
    profile: String,
    profile_source: ProfileSource,
    paths: PathOverrides,
    state: &mut ParseState,
) -> Result<McpCommand, AppError> {
    let enable_write_tools = state.take_flag("--enable-write-tools");

    match state.next().as_deref() {
        Some("install") => match state.next().as_deref() {
            Some("claude") => Ok(McpCommand::Install(McpInstallInput {
                target: McpInstallTarget::Claude,
                profile,
                profile_source,
                enable_write_tools,
                paths,
            })),
            Some("codex") => Ok(McpCommand::Install(McpInstallInput {
                target: McpInstallTarget::Codex,
                profile,
                profile_source,
                enable_write_tools,
                paths,
            })),
            Some("opencode") => Ok(McpCommand::Install(McpInstallInput {
                target: McpInstallTarget::OpenCode,
                profile,
                profile_source,
                enable_write_tools,
                paths,
            })),
            Some(other) => Err(AppError::validation(format!(
                "unknown mcp install target: {other}"
            ))),
            None => Err(AppError::validation("missing mcp install target")),
        },
        Some(other) => Err(AppError::validation(format!(
            "unknown mcp subcommand: {other}"
        ))),
        None => Err(AppError::validation("missing mcp subcommand")),
    }
}

fn parse_month(value: &str) -> Result<u32, AppError> {
    match value.to_ascii_lowercase().as_str() {
        "january" => Ok(1),
        "february" => Ok(2),
        "march" => Ok(3),
        "april" => Ok(4),
        "may" => Ok(5),
        "june" => Ok(6),
        "july" => Ok(7),
        "august" => Ok(8),
        "september" => Ok(9),
        "october" => Ok(10),
        "november" => Ok(11),
        "december" => Ok(12),
        other => Err(AppError::validation(format!(
            "unknown stat selector: {other}"
        ))),
    }
}

fn parse_duration_minutes(value: &str) -> Result<u32, AppError> {
    if let Some(number) = value.strip_suffix('m') {
        let minutes: u32 = number
            .parse()
            .map_err(|_| AppError::validation(format!("invalid duration: {value}")))?;
        return Ok(minutes * 60);
    }
    if let Some(number) = value.strip_suffix('h') {
        let hours: u32 = number
            .parse()
            .map_err(|_| AppError::validation(format!("invalid duration: {value}")))?;
        return Ok(hours * 3600);
    }

    Err(AppError::validation(format!("invalid duration: {value}")))
}

fn truthy_env(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn extract_flag(tokens: &mut Vec<String>, flag: &str) -> bool {
    if let Some(position) = tokens.iter().position(|token| token == flag) {
        tokens.remove(position);
        true
    } else {
        false
    }
}

fn extract_option_value(tokens: &mut Vec<String>, flag: &str) -> Result<Option<String>, AppError> {
    let Some(position) = tokens.iter().position(|token| token == flag) else {
        return Ok(None);
    };
    tokens.remove(position);
    if position >= tokens.len() {
        return Err(AppError::validation(format!("missing value for {flag}")));
    }
    if tokens[position].starts_with('-') {
        return Err(AppError::validation(format!("missing value for {flag}")));
    }
    Ok(Some(tokens.remove(position)))
}

fn extract_message(tokens: &mut Vec<String>) -> Result<Option<String>, AppError> {
    if let Some(message) = extract_option_value(tokens, "-m")? {
        return Ok(Some(message));
    }
    if let Some(message) = extract_option_value(tokens, "--message")? {
        return Ok(Some(message));
    }
    Ok(None)
}

struct ParseState {
    args: Vec<String>,
    index: usize,
}

impl ParseState {
    fn new(args: Vec<String>) -> Self {
        Self { args, index: 0 }
    }

    fn next(&mut self) -> Option<String> {
        let value = self.args.get(self.index).cloned();
        if value.is_some() {
            self.index += 1;
        }
        value
    }

    fn peek(&self) -> Option<&str> {
        self.args.get(self.index).map(String::as_str)
    }

    fn remaining_len(&self) -> usize {
        self.args.len().saturating_sub(self.index)
    }

    fn take_flag(&mut self, flag: &str) -> bool {
        if let Some(position) = self.args[self.index..]
            .iter()
            .position(|value| value == flag)
        {
            self.args.remove(self.index + position);
            true
        } else {
            false
        }
    }

    fn take_option_value(&mut self, flag: &str) -> Result<Option<String>, AppError> {
        let Some(relative) = self.args[self.index..]
            .iter()
            .position(|value| value == flag)
        else {
            return Ok(None);
        };
        let absolute = self.index + relative;
        self.args.remove(absolute);
        if absolute >= self.args.len() {
            return Err(AppError::validation(format!("missing value for {flag}")));
        }
        if self.args[absolute].starts_with('-') {
            return Err(AppError::validation(format!("missing value for {flag}")));
        }
        Ok(Some(self.args.remove(absolute)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        DomainCommand, LogDateSpec, McpCommand, McpInstallTarget, ProfileSource, RelativeLogDate,
        StatSelector,
    };

    #[test]
    fn parses_setup_with_profile() {
        let parsed = parse_cli(vec![
            String::from("--verbose"),
            String::from("--profile"),
            String::from("work"),
            String::from("setup"),
        ])
        .expect("setup parses");

        match parsed.command {
            DomainCommand::Setup(input) => {
                assert_eq!(input.profile, "work");
                assert!(parsed.verbose);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_stat_flags_and_week_selector() {
        let parsed = parse_cli(vec![
            String::from("stat"),
            String::from("--refresh-calendar"),
            String::from("week"),
        ])
        .expect("stat parses");

        match parsed.command {
            DomainCommand::Stat(input) => {
                assert!(input.refresh_calendar);
                assert_eq!(input.selector, StatSelector::Week);
                assert!(!input.details);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_stat_date_with_details() {
        let parsed = parse_cli(vec![
            String::from("stat"),
            String::from("--details"),
            String::from("2026-04-01"),
        ])
        .expect("stat date parses");

        match parsed.command {
            DomainCommand::Stat(input) => {
                assert!(input.details);
                assert_eq!(
                    input.selector,
                    StatSelector::Date(parse_date_override("2026-04-01").unwrap())
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_stat_yesterday() {
        let parsed = parse_cli(vec![String::from("stat"), String::from("yesterday")])
            .expect("stat yesterday parses");

        match parsed.command {
            DomainCommand::Stat(input) => {
                assert_eq!(input.selector, StatSelector::Yesterday);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_alias_set_with_defaults() {
        let parsed = parse_cli(vec![
            String::from("alias"),
            String::from("standup"),
            String::from("TC-3"),
            String::from("--default-duration"),
            String::from("30m"),
            String::from("-m"),
            String::from("daily standup"),
        ])
        .expect("alias parses");

        match parsed.command {
            DomainCommand::Alias(AliasCommand::Set(input)) => {
                assert_eq!(input.name, "standup");
                assert_eq!(input.default_duration, Some(1800));
                assert_eq!(input.default_message.as_deref(), Some("daily standup"));
                assert!(input.validate_remote);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_issue_first_duration_log() {
        let parsed = parse_cli(vec![
            String::from("TK-1234"),
            String::from("8h"),
            String::from("15m"),
        ])
        .expect("issue-first log parses");

        match parsed.command {
            DomainCommand::Log(input) => {
                assert_eq!(input.issue_token, "TK-1234");
                assert_eq!(
                    input.kind,
                    LogKind::Duration {
                        seconds: Some(8 * 3600 + 15 * 60),
                        date: None
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_alias_first_duration_log() {
        let parsed = parse_cli(vec![String::from("standup"), String::from("30m")])
            .expect("alias-first log parses");

        match parsed.command {
            DomainCommand::Log(input) => {
                assert_eq!(input.issue_token, "standup");
                assert_eq!(
                    input.kind,
                    LogKind::Duration {
                        seconds: Some(1800),
                        date: None
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_single_token_alias_candidate() {
        let parsed = parse_cli(vec![String::from("standup")]).expect("single-token alias parses");

        match parsed.command {
            DomainCommand::Log(input) => {
                assert_eq!(input.issue_token, "standup");
                assert_eq!(
                    input.kind,
                    LogKind::Duration {
                        seconds: None,
                        date: None
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_duration_first_log_with_date_and_message() {
        let parsed = parse_cli(vec![
            String::from("8h"),
            String::from("TK-1234"),
            String::from("--date"),
            String::from("2026-04-01"),
            String::from("--dry-run"),
            String::from("-m"),
            String::from("fix flaky test"),
        ])
        .expect("duration-first log parses");

        match parsed.command {
            DomainCommand::Log(input) => {
                assert_eq!(input.issue_token, "TK-1234");
                assert!(input.dry_run);
                assert_eq!(input.description.as_deref(), Some("fix flaky test"));
                assert_eq!(
                    input.kind,
                    LogKind::Duration {
                        seconds: Some(8 * 3600),
                        date: Some(LogDateSpec::Absolute(
                            parse_date_override("2026-04-01").unwrap(),
                        )),
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_period_log() {
        let parsed = parse_cli(vec![
            String::from("04/01/2026"),
            String::from("812"),
            String::from("-"),
            String::from("04/01/2026"),
            String::from("1700"),
            String::from("TK-1234"),
        ])
        .expect("period log parses");

        match parsed.command {
            DomainCommand::Log(input) => match input.kind {
                LogKind::Period { start, end } => {
                    assert_eq!(input.issue_token, "TK-1234");
                    assert_eq!(start.date(), parse_date_override("04/01/2026").unwrap());
                    assert_eq!(end.date(), parse_date_override("04/01/2026").unwrap());
                }
                other => panic!("unexpected log kind: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_duration_first_log_with_trailing_absolute_date() {
        let parsed = parse_cli(vec![
            String::from("3h"),
            String::from("TK-1234"),
            String::from("2026-05-11"),
        ])
        .expect("duration-first trailing date parses");

        match parsed.command {
            DomainCommand::Log(input) => {
                assert_eq!(input.issue_token, "TK-1234");
                assert_eq!(
                    input.kind,
                    LogKind::Duration {
                        seconds: Some(3 * 3600),
                        date: Some(LogDateSpec::Absolute(
                            parse_date_override("2026-05-11").unwrap(),
                        )),
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_issue_first_log_with_trailing_relative_date() {
        let parsed = parse_cli(vec![
            String::from("TK-1234"),
            String::from("3h"),
            String::from("yesterday"),
        ])
        .expect("issue-first trailing relative date parses");

        match parsed.command {
            DomainCommand::Log(input) => {
                assert_eq!(input.issue_token, "TK-1234");
                assert_eq!(
                    input.kind,
                    LogKind::Duration {
                        seconds: Some(3 * 3600),
                        date: Some(LogDateSpec::Relative(RelativeLogDate::Yesterday)),
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_alias_log_with_trailing_relative_date() {
        let parsed = parse_cli(vec![String::from("standup"), String::from("yesterday")])
            .expect("alias trailing relative date parses");

        match parsed.command {
            DomainCommand::Log(input) => {
                assert_eq!(input.issue_token, "standup");
                assert_eq!(
                    input.kind,
                    LogKind::Duration {
                        seconds: None,
                        date: Some(LogDateSpec::Relative(RelativeLogDate::Yesterday)),
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn rejects_mixed_flag_and_trailing_dates() {
        let error = parse_cli(vec![
            String::from("3h"),
            String::from("TK-1234"),
            String::from("yesterday"),
            String::from("--date"),
            String::from("2026-05-11"),
        ])
        .expect_err("mixed dates rejected");

        assert!(
            error
                .to_string()
                .contains("cannot use both --date and a trailing date argument")
        );
    }

    #[test]
    fn rejects_bare_issue_key_without_duration() {
        let error = parse_cli(vec![String::from("TK-1234")]).expect_err("bare issue key rejected");
        assert!(
            error
                .to_string()
                .contains("missing duration after issue key")
        );
    }

    #[test]
    fn rejects_flag_like_profile_value() {
        let error = parse_cli(vec![
            String::from("--profile"),
            String::from("--config-dir"),
            String::from("stat"),
        ])
        .expect_err("flag-like profile value rejected");

        assert!(error.to_string().contains("missing value for --profile"));
    }

    #[test]
    fn rejects_flag_like_date_value() {
        let error = parse_cli(vec![
            String::from("1h"),
            String::from("TK-1234"),
            String::from("--date"),
            String::from("--force"),
        ])
        .expect_err("flag-like date value rejected");

        assert!(error.to_string().contains("missing value for --date"));
    }

    #[test]
    fn rejects_flag_like_alias_message_value() {
        let error = parse_cli(vec![
            String::from("alias"),
            String::from("standup"),
            String::from("TC-3"),
            String::from("-m"),
            String::from("--no-validate"),
        ])
        .expect_err("flag-like message value rejected");

        assert!(error.to_string().contains("missing value for -m"));
    }

    #[test]
    fn parses_mcp_install_claude() {
        let parsed = parse_cli(vec![
            String::from("mcp"),
            String::from("install"),
            String::from("claude"),
        ])
        .expect("mcp install claude parses");

        match parsed.command {
            DomainCommand::Mcp(McpCommand::Install(input)) => {
                assert_eq!(input.target, McpInstallTarget::Claude);
                assert_eq!(input.profile, "default");
                assert_eq!(input.profile_source, ProfileSource::Default);
                assert!(!input.enable_write_tools);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_mcp_install_with_profile_flag() {
        let parsed = parse_cli(vec![
            String::from("--profile"),
            String::from("work"),
            String::from("mcp"),
            String::from("install"),
            String::from("codex"),
        ])
        .expect("mcp install codex parses");

        match parsed.command {
            DomainCommand::Mcp(McpCommand::Install(input)) => {
                assert_eq!(input.target, McpInstallTarget::Codex);
                assert_eq!(input.profile, "work");
                assert_eq!(input.profile_source, ProfileSource::Flag);
                assert!(!input.enable_write_tools);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_mcp_install_with_enable_write_tools_flag() {
        let parsed = parse_cli(vec![
            String::from("mcp"),
            String::from("--enable-write-tools"),
            String::from("install"),
            String::from("opencode"),
        ])
        .expect("mcp install opencode parses");

        match parsed.command {
            DomainCommand::Mcp(McpCommand::Install(input)) => {
                assert_eq!(input.target, McpInstallTarget::OpenCode);
                assert!(input.enable_write_tools);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_mcp_install_target() {
        let error = parse_cli(vec![
            String::from("mcp"),
            String::from("install"),
            String::from("cursor"),
        ])
        .expect_err("unknown mcp target rejected");

        assert!(
            error
                .to_string()
                .contains("unknown mcp install target: cursor")
        );
    }
}
