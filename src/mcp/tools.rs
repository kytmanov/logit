use std::path::PathBuf;

use chrono::NaiveDate;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::clock::Clock;
use crate::domain::{PathOverrides, StatSelector};
use crate::error::AppError;
use crate::jira::JiraClient;
use crate::service::alias::list_aliases;
use crate::service::config::config_path;
use crate::service::doctor::collect_doctor_info;
use crate::service::stats::get_stats;
use crate::service::types::{
    ConfigPathRequest, GetStatsRequest, ListAliasesRequest, LogTimeRequest, PreviewLogRequest,
    ProfileRef, RequestScope, ServiceError, StatsCacheMode,
};
use crate::service::worklog::{log_time, preview_log_time};
use crate::tempo::TempoClient;
use crate::time_parse::parse_date_override;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallOutput {
    pub structured_content: Value,
    pub content_text: String,
    pub is_error: bool,
}

impl ToolCallOutput {
    pub fn into_result(self) -> Value {
        let mut result = Map::new();
        result.insert(
            String::from("content"),
            Value::Array(vec![json!({
                "type": "text",
                "text": self.content_text,
            })]),
        );
        result.insert(String::from("structuredContent"), self.structured_content);
        if self.is_error {
            result.insert(String::from("isError"), Value::Bool(true));
        }
        Value::Object(result)
    }
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default = "empty_object")]
    arguments: Value,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ScopeArgs {
    profile: Option<String>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
}

impl ScopeArgs {
    fn into_scope(self, default_scope: &RequestScope) -> RequestScope {
        RequestScope {
            profile: self
                .profile
                .map(ProfileRef::Named)
                .unwrap_or_else(|| default_scope.profile.clone()),
            paths: PathOverrides {
                config_dir: self
                    .config_dir
                    .or_else(|| default_scope.paths.config_dir.clone()),
                data_dir: self
                    .data_dir
                    .or_else(|| default_scope.paths.data_dir.clone()),
                cache_dir: self
                    .cache_dir
                    .or_else(|| default_scope.paths.cache_dir.clone()),
            },
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DoctorArgs {
    #[serde(flatten)]
    scope: ScopeArgs,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ConfigPathArgs {
    #[serde(flatten)]
    scope: ScopeArgs,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ListAliasesArgs {
    #[serde(flatten)]
    scope: ScopeArgs,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GetStatsArgs {
    #[serde(flatten)]
    scope: ScopeArgs,
    when: Option<String>,
    details: bool,
    cache_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewLogArgs {
    #[serde(flatten)]
    scope: ScopeArgs,
    issue_or_alias: String,
    duration_seconds: Option<u32>,
    date: Option<NaiveDate>,
    message: Option<String>,
    #[serde(default)]
    force: bool,
}

pub fn tool_definitions(enable_write_tools: bool) -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "doctor",
            "description": "Inspect local config, data, cache, and active profile state.",
            "inputSchema": scope_only_schema(),
        }),
        json!({
            "name": "config_path",
            "description": "Return the resolved config.toml path.",
            "inputSchema": scope_only_schema(),
        }),
        json!({
            "name": "list_aliases",
            "description": "List aliases for the resolved profile.",
            "inputSchema": scope_only_schema(),
        }),
        json!({
            "name": "get_stats",
            "description": "Return worklog stats for today, a date, a week, a month, or a year.",
            "inputSchema": get_stats_schema(),
        }),
        json!({
            "name": "preview_log_time",
            "description": "Preview a worklog draft without creating a Tempo worklog.",
            "inputSchema": preview_log_schema(),
        }),
    ];

    if enable_write_tools {
        tools.push(json!({
            "name": "log_time",
            "description": "Create a Tempo worklog using the same inputs as preview_log_time.",
            "inputSchema": preview_log_schema(),
        }));
    }

    tools
}

pub fn parse_tool_call_params(value: Value) -> Result<(String, Value), AppError> {
    let params: ToolCallParams = serde_json::from_value(value)
        .map_err(|error| AppError::validation(format!("invalid tools/call params: {error}")))?;
    Ok((params.name, params.arguments))
}

pub fn call_tool<C: Clock, J: JiraClient, T: TempoClient>(
    name: &str,
    arguments: Value,
    default_scope: &RequestScope,
    clock: &C,
    jira: &J,
    tempo: &T,
    enable_write_tools: bool,
) -> Result<ToolCallOutput, AppError> {
    let arguments = normalize_arguments(arguments)?;

    match name {
        "doctor" => {
            let args: DoctorArgs = parse_arguments(arguments)?;
            let output = collect_doctor_info(&args.scope.into_scope(default_scope));
            Ok(match output {
                Ok(output) => tool_success(output),
                Err(error) => tool_service_error(error),
            })
        }
        "config_path" => {
            let args: ConfigPathArgs = parse_arguments(arguments)?;
            let output = config_path(ConfigPathRequest {
                scope: args.scope.into_scope(default_scope),
            });
            Ok(match output {
                Ok(output) => tool_success(output),
                Err(error) => tool_service_error(error),
            })
        }
        "list_aliases" => {
            let args: ListAliasesArgs = parse_arguments(arguments)?;
            let output = list_aliases(ListAliasesRequest {
                scope: args.scope.into_scope(default_scope),
            });
            Ok(match output {
                Ok(output) => tool_success(output),
                Err(error) => tool_service_error(error),
            })
        }
        "get_stats" => {
            let args: GetStatsArgs = parse_arguments(arguments)?;
            let output = get_stats(GetStatsRequest {
                scope: args.scope.into_scope(default_scope),
                selector: parse_stat_selector(args.when)?,
                details: args.details,
                cache_mode: parse_cache_mode(args.cache_mode)?,
            });
            Ok(match output {
                Ok(output) => tool_success(output),
                Err(error) => tool_service_error(error),
            })
        }
        "preview_log_time" => {
            let args: PreviewLogArgs = parse_arguments(arguments)?;
            let output = preview_log_time(
                clock,
                PreviewLogRequest {
                    scope: args.scope.into_scope(default_scope),
                    issue_or_alias: args.issue_or_alias,
                    duration_seconds: args.duration_seconds,
                    date: args.date,
                    message: args.message,
                    force: args.force,
                },
            );
            Ok(match output {
                Ok(output) => tool_success(output),
                Err(error) => tool_service_error(error),
            })
        }
        "log_time" => {
            if !enable_write_tools {
                return Ok(tool_disabled_error());
            }

            let args: PreviewLogArgs = parse_arguments(arguments)?;
            let output = log_time(
                clock,
                jira,
                tempo,
                LogTimeRequest {
                    scope: args.scope.into_scope(default_scope),
                    issue_or_alias: args.issue_or_alias,
                    duration_seconds: args.duration_seconds,
                    date: args.date,
                    message: args.message,
                    force: args.force,
                },
            );
            Ok(match output {
                Ok(output) => tool_success(output),
                Err(error) => tool_service_error(error),
            })
        }
        other => Err(AppError::validation(format!("unknown tool: {other}"))),
    }
}

fn parse_arguments<T: DeserializeOwned>(arguments: Value) -> Result<T, AppError> {
    serde_json::from_value(arguments)
        .map_err(|error| AppError::validation(format!("invalid tool arguments: {error}")))
}

fn normalize_arguments(arguments: Value) -> Result<Value, AppError> {
    match arguments {
        Value::Null => Ok(empty_object()),
        Value::Object(_) => Ok(arguments),
        _ => Err(AppError::validation("tool arguments must be an object")),
    }
}

fn tool_success<T: Serialize>(value: T) -> ToolCallOutput {
    let structured_content = serde_json::to_value(&value).expect("tool result serializes");
    ToolCallOutput {
        content_text: serde_json::to_string_pretty(&value).expect("tool result text"),
        structured_content,
        is_error: false,
    }
}

fn tool_service_error(error: crate::service::types::ServiceError) -> ToolCallOutput {
    let structured_content = json!({ "error": error });
    ToolCallOutput {
        content_text: serde_json::to_string_pretty(&structured_content).expect("tool error text"),
        structured_content,
        is_error: true,
    }
}

fn tool_disabled_error() -> ToolCallOutput {
    tool_service_error(ServiceError {
        code: "tool_disabled",
        category: "config",
        message: String::from("log_time is disabled; restart logit-mcp with --enable-write-tools"),
        remediation: Some(String::from(
            "Enable write tools explicitly before using mutating MCP operations.",
        )),
        retryable: false,
    })
}

fn parse_stat_selector(when: Option<String>) -> Result<StatSelector, AppError> {
    let Some(when) = when else {
        return Ok(StatSelector::Today);
    };
    let trimmed = when.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("today") {
        return Ok(StatSelector::Today);
    }
    if trimmed.eq_ignore_ascii_case("yesterday") {
        return Ok(StatSelector::Yesterday);
    }
    if trimmed.eq_ignore_ascii_case("week") {
        return Ok(StatSelector::Week);
    }
    if trimmed.len() == 4 && trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(StatSelector::Year(
            trimmed.parse().expect("digit-checked year selector"),
        ));
    }
    if let Ok(date) = parse_date_override(trimmed) {
        return Ok(StatSelector::Date(date));
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() == 2 && parts[0].eq_ignore_ascii_case("last") {
        if parts[1].eq_ignore_ascii_case("week") {
            return Ok(StatSelector::LastWeek);
        }
        return Err(AppError::validation(format!(
            "unknown stat selector: last {}",
            parts[1]
        )));
    }

    match parts.as_slice() {
        [month] => Ok(StatSelector::Month {
            month: parse_month(month)?,
            year: 0,
        }),
        [month, year] if year.len() == 4 && year.chars().all(|ch| ch.is_ascii_digit()) => {
            Ok(StatSelector::Month {
                month: parse_month(month)?,
                year: year.parse().expect("digit-checked month year"),
            })
        }
        _ => Err(AppError::validation(format!(
            "unknown stat selector: {trimmed}"
        ))),
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

fn parse_cache_mode(value: Option<String>) -> Result<StatsCacheMode, AppError> {
    match value.as_deref() {
        None => Ok(StatsCacheMode::NoWrite),
        Some("no_write") | Some("no-write") => Ok(StatsCacheMode::NoWrite),
        Some("use_cache") | Some("use-cache") => Ok(StatsCacheMode::UseCache),
        Some("refresh") => Ok(StatsCacheMode::Refresh),
        Some("no_calendar") | Some("no-calendar") => Ok(StatsCacheMode::NoCalendar),
        Some(other) => Err(AppError::validation(format!("unknown cache_mode: {other}"))),
    }
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

fn scope_only_schema() -> Value {
    object_schema(scope_properties(), &[])
}

fn get_stats_schema() -> Value {
    let mut properties = scope_properties();
    properties.push((
        "when",
        json!({
            "type": "string",
            "description": "today, week, last week, YYYY-MM-DD, a month name, or a year",
        }),
    ));
    properties.push((
        "details",
        json!({
            "type": "boolean",
            "description": "Include individual worklog rows when supported.",
        }),
    ));
    properties.push((
        "cache_mode",
        json!({
            "type": "string",
            "enum": ["no_write", "use_cache", "refresh", "no_calendar"],
            "description": "Calendar lookup mode. Defaults to no_write for MCP.",
        }),
    ));
    object_schema(properties, &[])
}

fn preview_log_schema() -> Value {
    let mut properties = scope_properties();
    properties.push((
        "issue_or_alias",
        json!({
            "type": "string",
            "description": "Issue key or alias to resolve.",
        }),
    ));
    properties.push((
        "duration_seconds",
        json!({
            "type": "integer",
            "minimum": 1,
            "description": "Explicit duration in seconds. Optional when an alias defines a default duration.",
        }),
    ));
    properties.push((
        "date",
        json!({
            "type": "string",
            "format": "date",
            "description": "Optional worklog date in YYYY-MM-DD format.",
        }),
    ));
    properties.push((
        "message",
        json!({
            "type": "string",
            "description": "Optional worklog description.",
        }),
    ));
    properties.push((
        "force",
        json!({
            "type": "boolean",
            "description": "Keep duplicate checking semantics aligned with the CLI preview.",
        }),
    ));
    object_schema(properties, &["issue_or_alias"])
}

fn object_schema(properties: Vec<(&'static str, Value)>, required: &[&str]) -> Value {
    let mut property_map = Map::new();
    for (name, value) in properties {
        property_map.insert(String::from(name), value);
    }

    json!({
        "type": "object",
        "properties": property_map,
        "required": required,
        "additionalProperties": false,
    })
}

fn scope_properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "profile",
            json!({
                "type": "string",
                "description": "Exact profile name. Omit to use the active or process default profile.",
            }),
        ),
        (
            "config_dir",
            json!({
                "type": "string",
                "description": "Optional config directory override.",
            }),
        ),
        (
            "data_dir",
            json!({
                "type": "string",
                "description": "Optional data directory override.",
            }),
        ),
        (
            "cache_dir",
            json!({
                "type": "string",
                "description": "Optional cache directory override.",
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_last_week_case_insensitively() {
        assert_eq!(
            parse_stat_selector(Some(String::from("LAST WEEK"))).expect("selector parses"),
            StatSelector::LastWeek
        );
    }

    #[test]
    fn parses_yesterday_case_insensitively() {
        assert_eq!(
            parse_stat_selector(Some(String::from("YESTERDAY"))).expect("selector parses"),
            StatSelector::Yesterday
        );
    }

    #[test]
    fn mcp_stats_default_to_no_write_cache_mode() {
        assert_eq!(
            parse_cache_mode(None).expect("default cache mode"),
            StatsCacheMode::NoWrite
        );
    }

    #[test]
    fn log_time_tool_is_only_listed_when_enabled() {
        let disabled = tool_definitions(false);
        let enabled = tool_definitions(true);

        assert!(!disabled.iter().any(|tool| tool["name"] == "log_time"));
        assert!(enabled.iter().any(|tool| tool["name"] == "log_time"));
    }
}
