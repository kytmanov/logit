use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandInput {
    pub profile: String,
    pub paths: PathOverrides,
}

impl Default for CommandInput {
    fn default() -> Self {
        Self {
            profile: String::from("default"),
            paths: PathOverrides::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathOverrides {
    pub config_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParsedCli {
    pub command: DomainCommand,
    pub verbose: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProfileSource {
    Flag,
    Env,
    Default,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DomainCommand {
    Setup(SetupInput),
    Log(LogInput),
    Stat(StatRangeInput),
    Alias(AliasCommand),
    Doctor(CommandInput),
    Config(ConfigCommand),
    Cache(CacheCommand),
    Mcp(McpCommand),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum McpCommand {
    Install(McpInstallInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpInstallInput {
    pub target: McpInstallTarget,
    pub profile: String,
    pub profile_source: ProfileSource,
    pub enable_write_tools: bool,
    pub paths: PathOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum McpInstallTarget {
    Claude,
    Codex,
    OpenCode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupInput {
    pub profile: String,
    pub paths: PathOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupValues {
    pub profile: String,
    pub jira_url: String,
    pub email: String,
    pub tempo_token: String,
    pub jira_token: String,
    pub tz: String,
    pub work_hours: WorkHours,
    pub working_days: Vec<WeekdayName>,
    pub time_format: TimeFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AliasCommand {
    Set(AliasSetInput),
    List(CommandInput),
    Delete(AliasDeleteInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AliasSetInput {
    pub profile: String,
    pub paths: PathOverrides,
    pub name: String,
    pub issue_key: String,
    pub default_duration: Option<u32>,
    pub default_message: Option<String>,
    pub validate_remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AliasDeleteInput {
    pub profile: String,
    pub paths: PathOverrides,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigCommand {
    Path(CommandInput),
    Edit(CommandInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CacheCommand {
    Clear(CommandInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub schema_version: u32,
    pub active: String,
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub jira_url: String,
    pub email: String,
    pub account_id: Option<String>,
    pub tz: String,
    pub work_hours: WorkHours,
    pub working_days: Vec<WeekdayName>,
    pub time_format: TimeFormat,
    pub aliases: BTreeMap<String, Alias>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Alias {
    pub key: String,
    pub default_duration: Option<u32>,
    pub default_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkHours {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WeekdayName {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimeFormat {
    TwentyFourHour,
    AmPm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogInput {
    pub profile: String,
    pub paths: PathOverrides,
    pub issue_token: String,
    pub description: Option<String>,
    pub dry_run: bool,
    pub force: bool,
    pub kind: LogKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RelativeLogDate {
    Today,
    Yesterday,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LogDateSpec {
    Absolute(NaiveDate),
    Relative(RelativeLogDate),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LogKind {
    Duration {
        seconds: Option<u32>,
        date: Option<LogDateSpec>,
    },
    Period {
        start: NaiveDateTime,
        end: NaiveDateTime,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorklogDraft {
    pub issue_key: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub duration_seconds: u32,
    pub timezone: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorklogBoundaryDraft {
    pub issue_id: String,
    pub author_account_id: String,
    pub start_date: NaiveDate,
    pub start_time: NaiveTime,
    pub time_spent_seconds: u32,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorklogResult {
    pub worklog_id: String,
    pub issue_key: String,
    pub issue_id: Option<String>,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub duration_seconds: u32,
    pub tempo_url: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatRangeInput {
    pub profile: String,
    pub paths: PathOverrides,
    pub selector: StatSelector,
    pub details: bool,
    pub refresh_calendar: bool,
    pub no_calendar: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StatSelector {
    Today,
    Yesterday,
    Date(NaiveDate),
    Month { month: u32, year: i32 },
    Year(i32),
    Week,
    LastWeek,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatReport {
    pub label: String,
    pub rows: Vec<StatRow>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatRow {
    pub date: NaiveDate,
    pub source: ExpectedTimeSource,
    pub worklogs: Vec<WorklogLine>,
    pub expected_seconds: u32,
    pub filled_seconds: u32,
    pub holiday_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorklogLine {
    pub issue_key: String,
    pub duration_seconds: u32,
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExpectedTimeSource {
    TempoWorkSchedule,
    ConfiguredWorkday,
    StaleCache { days_old: u32 },
}

pub fn parse_timezone(name: &str) -> Option<Tz> {
    name.parse().ok()
}

pub fn parse_fixed_offset(name: &str) -> Option<FixedOffset> {
    let bytes = name.as_bytes();
    if bytes.len() != 6 || !matches!(bytes[0], b'+' | b'-') || bytes[3] != b':' {
        return None;
    }

    let hours: i32 = name[1..3].parse().ok()?;
    let minutes: i32 = name[4..6].parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }

    let seconds = hours * 3600 + minutes * 60;
    let seconds = if bytes[0] == b'-' { -seconds } else { seconds };
    FixedOffset::east_opt(seconds)
}

pub fn is_valid_timezone(name: &str) -> bool {
    parse_timezone(name).is_some() || parse_fixed_offset(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_domain_types_to_json() {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            String::from("default"),
            Profile {
                jira_url: String::from("https://example.atlassian.net"),
                email: String::from("user@example.com"),
                account_id: Some(String::from("acct-1")),
                tz: String::from("UTC"),
                work_hours: WorkHours {
                    start: String::from("09:00"),
                    end: String::from("17:00"),
                },
                working_days: vec![WeekdayName::Mon, WeekdayName::Tue, WeekdayName::Wed],
                time_format: TimeFormat::TwentyFourHour,
                aliases: BTreeMap::new(),
            },
        );

        let parsed = ParsedCli {
            command: DomainCommand::Setup(SetupInput {
                profile: String::from("default"),
                paths: PathOverrides::default(),
            }),
            verbose: false,
        };

        let config = Config {
            schema_version: 1,
            active: String::from("default"),
            profiles,
        };

        let parsed_json = serde_json::to_string(&parsed).expect("parsed cli serializes");
        let config_json = serde_json::to_string(&config).expect("config serializes");

        assert!(parsed_json.contains("Setup"));
        assert!(config_json.contains("schema_version"));
        assert!(config_json.contains("default"));
    }

    #[test]
    fn parses_known_timezone() {
        assert_eq!(parse_timezone("UTC"), Some(chrono_tz::UTC));
    }

    #[test]
    fn accepts_fixed_offset_timezone() {
        assert!(is_valid_timezone("-07:00"));
        assert!(parse_fixed_offset("+05:30").is_some());
    }
}
