use std::io::{BufRead, Write};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::clock::Clock;
use crate::error::AppError;
use crate::jira::JiraClient;
use crate::mcp::RuntimeConfig;
use crate::mcp::tools::{call_tool, parse_tool_call_params, tool_definitions};
use crate::tempo::TempoClient;

const JSONRPC_VERSION: &str = "2.0";
const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";
const ERROR_PARSE: i64 = -32700;
const ERROR_INVALID_REQUEST: i64 = -32600;
const ERROR_METHOD_NOT_FOUND: i64 = -32601;
const ERROR_INVALID_PARAMS: i64 = -32602;
const ERROR_NOT_INITIALIZED: i64 = -32002;

pub fn serve_stdio<R: BufRead, W: Write, C: Clock, J: JiraClient, T: TempoClient>(
    reader: R,
    mut writer: W,
    runtime: RuntimeConfig,
    clock: C,
    jira: &J,
    tempo: &T,
) -> Result<(), AppError> {
    let mut server = Server::new(runtime, clock, jira, tempo);

    for line in reader.lines() {
        let line = line.map_err(|error| AppError::config(format!("read stdin: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line) {
            serde_json::to_writer(&mut writer, &response)
                .map_err(|error| AppError::config(format!("write stdout: {error}")))?;
            writer
                .write_all(b"\n")
                .map_err(|error| AppError::config(format!("write stdout: {error}")))?;
            writer
                .flush()
                .map_err(|error| AppError::config(format!("flush stdout: {error}")))?;
        }
    }

    Ok(())
}

struct Server<'a, C, J, T> {
    runtime: RuntimeConfig,
    clock: C,
    jira: &'a J,
    tempo: &'a T,
    initialized: bool,
    protocol_version: String,
}

impl<'a, C: Clock, J: JiraClient, T: TempoClient> Server<'a, C, J, T> {
    fn new(runtime: RuntimeConfig, clock: C, jira: &'a J, tempo: &'a T) -> Self {
        Self {
            runtime,
            clock,
            jira,
            tempo,
            initialized: false,
            protocol_version: String::from(DEFAULT_PROTOCOL_VERSION),
        }
    }

    fn handle_line(&mut self, line: &str) -> Option<Value> {
        match serde_json::from_str::<Value>(line) {
            Ok(value) => self.handle_value(value),
            Err(error) => {
                Some(self.error_response(Value::Null, ERROR_PARSE, format!("parse error: {error}")))
            }
        }
    }

    fn handle_value(&mut self, value: Value) -> Option<Value> {
        let Value::Object(request) = value else {
            return Some(self.error_response(
                Value::Null,
                ERROR_INVALID_REQUEST,
                "request must be a JSON object",
            ));
        };

        let has_id = request.contains_key("id");
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            return has_id.then(|| {
                self.error_response(id, ERROR_INVALID_REQUEST, "missing JSON-RPC method")
            });
        };
        let params = request.get("params").cloned().unwrap_or_else(empty_object);

        match method {
            "initialize" => Some(self.handle_initialize(id, params)),
            "initialized" => {
                self.initialized = true;
                None
            }
            "ping" => has_id.then(|| self.success_response(id, empty_object())),
            "tools/list" => has_id.then(|| self.handle_tools_list(id, params)),
            "tools/call" => has_id.then(|| self.handle_tools_call(id, params)),
            _ => has_id.then(|| {
                self.error_response(
                    id,
                    ERROR_METHOD_NOT_FOUND,
                    format!("method not found: {method}"),
                )
            }),
        }
    }

    fn handle_initialize(&mut self, id: Value, params: Value) -> Value {
        match parse_initialize_params(params) {
            Ok(params) => {
                self.initialized = true;
                self.protocol_version = params
                    .protocol_version
                    .unwrap_or_else(|| String::from(DEFAULT_PROTOCOL_VERSION));
                self.success_response(
                    id,
                    json!({
                        "protocolVersion": self.protocol_version,
                        "capabilities": {
                            "tools": {
                                "listChanged": false,
                            }
                        },
                        "serverInfo": {
                            "name": "logit-mcp",
                            "version": env!("CARGO_PKG_VERSION"),
                        }
                    }),
                )
            }
            Err(error) => self.error_response(id, ERROR_INVALID_PARAMS, error.message),
        }
    }

    fn handle_tools_list(&mut self, id: Value, params: Value) -> Value {
        if !self.initialized {
            return self.error_response(id, ERROR_NOT_INITIALIZED, "server not initialized");
        }
        if !params.is_object() {
            return self.error_response(
                id,
                ERROR_INVALID_PARAMS,
                "tools/list params must be an object",
            );
        }

        self.success_response(
            id,
            json!({ "tools": tool_definitions(self.runtime.enable_write_tools) }),
        )
    }

    fn handle_tools_call(&mut self, id: Value, params: Value) -> Value {
        if !self.initialized {
            return self.error_response(id, ERROR_NOT_INITIALIZED, "server not initialized");
        }

        match parse_tool_call_params(params).and_then(|(name, arguments)| {
            call_tool(
                &name,
                arguments,
                &self.runtime.default_scope,
                &self.clock,
                self.jira,
                self.tempo,
                self.runtime.enable_write_tools,
            )
        }) {
            Ok(result) => self.success_response(id, result.into_result()),
            Err(error) => self.error_response(id, ERROR_INVALID_PARAMS, error.message),
        }
    }

    fn success_response(&self, id: Value, result: Value) -> Value {
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "result": result,
        })
    }

    fn error_response(&self, id: Value, code: i64, message: impl Into<String>) -> Value {
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "error": {
                "code": code,
                "message": message.into(),
            },
        })
    }
}

#[derive(Debug, Deserialize)]
struct InitializeParams {
    #[serde(default, rename = "protocolVersion")]
    protocol_version: Option<String>,
}

fn parse_initialize_params(params: Value) -> Result<InitializeParams, AppError> {
    serde_json::from_value(params)
        .map_err(|error| AppError::validation(format!("invalid initialize params: {error}")))
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::NaiveDateTime;

    use super::*;
    use crate::clock::FixedClock;
    use crate::config::{default_config, save_config};
    use crate::domain::{Alias, PathOverrides, WorklogBoundaryDraft, WorklogResult};
    use crate::error::AppError;
    use crate::jira::JiraClient;
    use crate::secrets::{FileSecretStore, ProfileSecrets, SecretStore};
    use crate::service::types::{ProfileRef, RequestScope};
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
            unreachable!()
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

    #[derive(Debug, Default)]
    struct TestTempo {
        existing: Vec<WorklogResult>,
    }

    impl TempoClient for TestTempo {
        fn validate_token(&self, _tempo_token: &str) -> Result<(), AppError> {
            unreachable!()
        }

        fn to_boundary_draft(
            &self,
            issue_id: String,
            author_account_id: String,
            draft: &crate::domain::WorklogDraft,
        ) -> WorklogBoundaryDraft {
            WorklogBoundaryDraft {
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
            draft: &WorklogBoundaryDraft,
        ) -> Result<WorklogResult, AppError> {
            let start = draft.start_date.and_time(draft.start_time);
            let end = start + chrono::Duration::seconds(i64::from(draft.time_spent_seconds));
            Ok(WorklogResult {
                worklog_id: String::from("worklog-1"),
                issue_key: draft.issue_id.clone(),
                issue_id: Some(draft.issue_id.clone()),
                start,
                end,
                duration_seconds: draft.time_spent_seconds,
                tempo_url: format!("{}/tempo/worklog/worklog-1", profile.jira_url),
                description: draft.description.clone(),
            })
        }

        fn list_worklogs(
            &self,
            _tempo_token: &str,
            _account_id: &str,
            _from: chrono::NaiveDate,
            _to: chrono::NaiveDate,
        ) -> Result<Vec<WorklogResult>, AppError> {
            Ok(self.existing.clone())
        }
    }

    fn temp_scope() -> (tempfile::TempDir, RequestScope) {
        let temp = tempfile::tempdir().expect("tempdir");
        let scope = RequestScope {
            profile: ProfileRef::Active,
            paths: PathOverrides {
                config_dir: Some(temp.path().join("config")),
                data_dir: Some(temp.path().join("data")),
                cache_dir: Some(temp.path().join("cache")),
            },
        };
        (temp, scope)
    }

    fn runtime(scope: RequestScope, enable_write_tools: bool) -> RuntimeConfig {
        RuntimeConfig {
            default_scope: scope,
            enable_write_tools,
        }
    }

    fn call(
        server: &mut Server<'_, FixedClock, TestJira, TestTempo>,
        request: Value,
    ) -> Option<Value> {
        server.handle_line(&request.to_string())
    }

    fn initialize(server: &mut Server<'_, FixedClock, TestJira, TestTempo>) -> Value {
        call(
            server,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26"
                }
            }),
        )
        .expect("initialize response")
    }

    #[test]
    fn initialize_advertises_tools_capability() {
        let (_temp, scope) = temp_scope();
        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, false), clock, &jira, &tempo);

        let response = initialize(&mut server);

        assert_eq!(response["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(response["result"]["serverInfo"]["name"], "logit-mcp");
        assert_eq!(
            response["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
    }

    #[test]
    fn tools_list_returns_expected_read_only_tools() {
        let (_temp, scope) = temp_scope();
        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, false), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }),
        )
        .expect("tools/list response");
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tool array present");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();

        assert_eq!(
            names,
            vec![
                "doctor",
                "config_path",
                "list_aliases",
                "get_stats",
                "preview_log_time"
            ]
        );
    }

    #[test]
    fn tools_list_includes_log_time_when_write_tools_enabled() {
        let (_temp, scope) = temp_scope();
        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, true), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/list",
                "params": {}
            }),
        )
        .expect("tools/list response");
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tool array present");

        assert!(tools.iter().any(|tool| tool["name"] == "log_time"));
    }

    #[test]
    fn ping_returns_empty_result() {
        let (_temp, scope) = temp_scope();
        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, false), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "ping",
                "params": {}
            }),
        )
        .expect("ping response");

        assert_eq!(response["result"], json!({}));
    }

    #[test]
    fn config_path_tool_uses_resolved_scope() {
        let (_temp, scope) = temp_scope();
        let expected_path = scope
            .paths
            .config_dir
            .clone()
            .expect("config dir")
            .join("config.toml")
            .display()
            .to_string();
        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, false), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "config_path",
                    "arguments": {}
                }
            }),
        )
        .expect("tools/call response");

        assert_eq!(
            response["result"]["structuredContent"]["data"]["config_path"],
            expected_path
        );
        assert_eq!(response["result"].get("isError"), None);
    }

    #[test]
    fn preview_log_time_resolves_alias_defaults() {
        let (_temp, scope) = temp_scope();
        let dirs = crate::paths::resolve_dirs(&scope.paths).expect("dirs");
        let mut config = default_config("UTC");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .aliases = BTreeMap::from([(
            String::from("standup"),
            Alias {
                key: String::from("TC-3"),
                default_duration: Some(1800),
                default_message: Some(String::from("daily standup")),
            },
        )]);
        save_config(&dirs, &config).expect("save config");

        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, false), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "preview_log_time",
                    "arguments": {
                        "issue_or_alias": "standup",
                        "date": "2026-04-01"
                    }
                }
            }),
        )
        .expect("preview response");

        assert_eq!(
            response["result"]["structuredContent"]["data"]["issue_key"],
            "TC-3"
        );
        assert_eq!(
            response["result"]["structuredContent"]["data"]["alias_used"],
            "standup"
        );
        assert_eq!(
            response["result"]["structuredContent"]["data"]["draft"]["start"],
            "2026-04-01T16:30:00"
        );
        assert_eq!(
            response["result"]["structuredContent"]["data"]["draft"]["description"],
            "daily standup"
        );
    }

    #[test]
    fn tool_service_errors_return_is_error_results() {
        let (_temp, scope) = temp_scope();
        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, false), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "get_stats",
                    "arguments": {
                        "when": "2026",
                        "details": true
                    }
                }
            }),
        )
        .expect("error response");

        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            response["result"]["structuredContent"]["error"]["code"],
            "details_not_supported"
        );
    }

    #[test]
    fn tools_call_accepts_top_level_meta_field() {
        let (_temp, scope) = temp_scope();
        let expected_path = scope
            .paths
            .config_dir
            .clone()
            .expect("config dir")
            .join("config.toml")
            .display()
            .to_string();
        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, false), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": {
                    "name": "config_path",
                    "arguments": {},
                    "_meta": {
                        "progressToken": 1
                    }
                }
            }),
        )
        .expect("tools/call response");

        assert_eq!(
            response["result"]["structuredContent"]["data"]["config_path"],
            expected_path
        );
        assert_eq!(response["error"], Value::Null);
    }

    #[test]
    fn disabled_log_time_returns_tool_disabled_error() {
        let (_temp, scope) = temp_scope();
        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, false), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 8,
                "method": "tools/call",
                "params": {
                    "name": "log_time",
                    "arguments": {
                        "issue_or_alias": "TC-3",
                        "duration_seconds": 1800
                    }
                }
            }),
        )
        .expect("log_time response");

        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            response["result"]["structuredContent"]["error"]["code"],
            "tool_disabled"
        );
    }

    #[test]
    fn log_time_creates_worklog_when_enabled() {
        let (_temp, scope) = temp_scope();
        let dirs = crate::paths::resolve_dirs(&scope.paths).expect("dirs");
        let mut config = default_config("UTC");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .jira_url = String::from("https://example.atlassian.net");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .email = String::from("user@example.com");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .account_id = Some(String::from("acct-1"));
        save_config(&dirs, &config).expect("save config");
        FileSecretStore::new(dirs)
            .expect("store")
            .save_profile(
                "default",
                &ProfileSecrets {
                    tempo_token: String::from("tempo-token"),
                    jira_token: String::from("jira-token"),
                },
            )
            .expect("save secrets");

        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, true), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 10,
                "method": "tools/call",
                "params": {
                    "name": "log_time",
                    "arguments": {
                        "issue_or_alias": "TC-3",
                        "duration_seconds": 1800,
                        "message": "daily standup"
                    }
                }
            }),
        )
        .expect("log_time response");

        assert_eq!(response["result"].get("isError"), None);
        assert_eq!(
            response["result"]["structuredContent"]["data"]["worklog"]["issue_key"],
            "TC-3"
        );
        assert_eq!(
            response["result"]["structuredContent"]["data"]["worklog"]["worklog_id"],
            "worklog-1"
        );
    }

    #[test]
    fn log_time_reports_duplicate_worklog_conflict() {
        let (_temp, scope) = temp_scope();
        let dirs = crate::paths::resolve_dirs(&scope.paths).expect("dirs");
        let mut config = default_config("UTC");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .jira_url = String::from("https://example.atlassian.net");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .email = String::from("user@example.com");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .account_id = Some(String::from("acct-1"));
        save_config(&dirs, &config).expect("save config");
        FileSecretStore::new(dirs)
            .expect("store")
            .save_profile(
                "default",
                &ProfileSecrets {
                    tempo_token: String::from("tempo-token"),
                    jira_token: String::from("jira-token"),
                },
            )
            .expect("save secrets");

        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo {
            existing: vec![WorklogResult {
                worklog_id: String::from("existing-1"),
                issue_key: String::from("TC-3"),
                issue_id: Some(String::from("issue-TC-3")),
                start: NaiveDateTime::parse_from_str("2026-04-01T11:30:00", "%Y-%m-%dT%H:%M:%S")
                    .expect("start"),
                end: NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                    .expect("end"),
                duration_seconds: 1800,
                tempo_url: String::new(),
                description: Some(String::from("daily standup")),
            }],
        };
        let mut server = Server::new(runtime(scope, true), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "tools/call",
                "params": {
                    "name": "log_time",
                    "arguments": {
                        "issue_or_alias": "TC-3",
                        "duration_seconds": 1800,
                        "message": "daily standup"
                    }
                }
            }),
        )
        .expect("log_time response");

        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            response["result"]["structuredContent"]["error"]["code"],
            "duplicate_worklog"
        );
    }

    #[test]
    fn log_time_reports_missing_account_id() {
        let (_temp, scope) = temp_scope();
        let dirs = crate::paths::resolve_dirs(&scope.paths).expect("dirs");
        let mut config = default_config("UTC");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .jira_url = String::from("https://example.atlassian.net");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .email = String::from("user@example.com");
        save_config(&dirs, &config).expect("save config");
        FileSecretStore::new(dirs)
            .expect("store")
            .save_profile(
                "default",
                &ProfileSecrets {
                    tempo_token: String::from("tempo-token"),
                    jira_token: String::from("jira-token"),
                },
            )
            .expect("save secrets");

        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, true), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 12,
                "method": "tools/call",
                "params": {
                    "name": "log_time",
                    "arguments": {
                        "issue_or_alias": "TC-3",
                        "duration_seconds": 1800
                    }
                }
            }),
        )
        .expect("log_time response");

        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            response["result"]["structuredContent"]["error"]["code"],
            "missing_account_id"
        );
    }

    #[test]
    fn log_time_reports_missing_secrets() {
        let (_temp, scope) = temp_scope();
        let dirs = crate::paths::resolve_dirs(&scope.paths).expect("dirs");
        let mut config = default_config("UTC");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .jira_url = String::from("https://example.atlassian.net");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .email = String::from("user@example.com");
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .account_id = Some(String::from("acct-1"));
        save_config(&dirs, &config).expect("save config");

        let clock = FixedClock::new(
            NaiveDateTime::parse_from_str("2026-04-01T12:00:00", "%Y-%m-%dT%H:%M:%S")
                .expect("fixed clock"),
        );
        let jira = TestJira;
        let tempo = TestTempo::default();
        let mut server = Server::new(runtime(scope, true), clock, &jira, &tempo);
        initialize(&mut server);

        let response = call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": 13,
                "method": "tools/call",
                "params": {
                    "name": "log_time",
                    "arguments": {
                        "issue_or_alias": "TC-3",
                        "duration_seconds": 1800
                    }
                }
            }),
        )
        .expect("log_time response");

        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            response["result"]["structuredContent"]["error"]["code"],
            "missing_secrets"
        );
    }
}
