use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use chrono::NaiveDate;

use crate::domain::{Profile, WorklogBoundaryDraft, WorklogDraft, WorklogResult};
use crate::error::AppError;

pub trait TempoClient {
    fn validate_token(&self, tempo_token: &str) -> Result<(), AppError>;
    fn to_boundary_draft(
        &self,
        issue_id: String,
        author_account_id: String,
        draft: &WorklogDraft,
    ) -> WorklogBoundaryDraft;
    fn create_worklog(
        &self,
        tempo_token: &str,
        profile: &Profile,
        draft: &WorklogBoundaryDraft,
    ) -> Result<WorklogResult, AppError>;
    fn list_worklogs(
        &self,
        tempo_token: &str,
        account_id: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<WorklogResult>, AppError>;
}

#[derive(Debug, Clone)]
pub struct HttpTempoClient {
    client: Client,
    base_url: String,
}

impl HttpTempoClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("tempo reqwest client"),
            base_url: base_url.into(),
        }
    }

    fn list_url(
        &self,
        account_id: &str,
        from: NaiveDate,
        to: NaiveDate,
        offset: usize,
        limit: usize,
    ) -> String {
        format!(
            "{}/4/worklogs/user/{}?from={}&to={}&offset={offset}&limit={limit}",
            self.base_url.trim_end_matches('/'),
            account_id,
            from.format("%Y-%m-%d"),
            to.format("%Y-%m-%d")
        )
    }

    fn send_list_request(
        &self,
        tempo_token: &str,
        account_id: &str,
        from: NaiveDate,
        to: NaiveDate,
        offset: usize,
        limit: usize,
    ) -> Result<reqwest::blocking::Response, AppError> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let response = self
                .client
                .get(self.list_url(account_id, from, to, offset, limit))
                .bearer_auth(tempo_token)
                .send()
                .map_err(|error| AppError::network(format!("tempo list failed: {error}")))?;

            let status = response.status().as_u16();
            if status == 401 || status == 403 {
                return Err(AppError::auth("Tempo token rejected"));
            }
            if !matches!(status, 429 | 500..=599) || attempts >= 3 {
                return Ok(response);
            }

            let delay_ms = (1_u64 << attempts.min(5)) * 25 + (offset % 17) as u64;
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
    }

    fn parse_worklog(item: TempoWorklogItem) -> Result<WorklogResult, AppError> {
        let start_date =
            chrono::NaiveDate::parse_from_str(&item.start_date, "%Y-%m-%d").map_err(|error| {
                AppError::network(format!(
                    "parse tempo start_date '{}': {error}",
                    item.start_date
                ))
            })?;
        let start_time =
            chrono::NaiveTime::parse_from_str(&item.start_time, "%H:%M:%S").map_err(|error| {
                AppError::network(format!(
                    "parse tempo start_time '{}': {error}",
                    item.start_time
                ))
            })?;
        let start = start_date.and_time(start_time);
        let issue_key = item.issue.issue_key();

        Ok(WorklogResult {
            worklog_id: item.tempo_worklog_id.to_string(),
            issue_key,
            issue_id: item.issue.id.map(|value| value.to_string()),
            start,
            end: start + chrono::Duration::seconds(i64::from(item.time_spent_seconds)),
            duration_seconds: item.time_spent_seconds,
            tempo_url: String::new(),
            description: item.description,
        })
    }
}

impl Default for HttpTempoClient {
    fn default() -> Self {
        let base_url = std::env::var("LOGIT_TEMPO_BASE_URL")
            .unwrap_or_else(|_| String::from("https://api.tempo.io"));
        Self::new(base_url)
    }
}

impl TempoClient for HttpTempoClient {
    fn validate_token(&self, tempo_token: &str) -> Result<(), AppError> {
        let response = self
            .client
            .get(format!(
                "{}/4/worklogs?limit=1",
                self.base_url.trim_end_matches('/')
            ))
            .bearer_auth(tempo_token)
            .send()
            .map_err(|error| {
                AppError::network(format!("tempo token validation failed: {error}"))
            })?;

        match response.status().as_u16() {
            200 => Ok(()),
            401 | 403 => Err(AppError::auth("Tempo token rejected")),
            status => Err(AppError::network(format!(
                "tempo validation returned HTTP {status}"
            ))),
        }
    }

    fn to_boundary_draft(
        &self,
        issue_id: String,
        author_account_id: String,
        draft: &WorklogDraft,
    ) -> WorklogBoundaryDraft {
        WorklogBoundaryDraft {
            issue_id,
            issue_key: draft.issue_key.clone(),
            author_account_id,
            start_date: draft.start.date(),
            start_time: draft.start.time(),
            time_spent_seconds: draft.duration_seconds,
            description: draft.description.clone(),
        }
    }

    fn create_worklog(
        &self,
        tempo_token: &str,
        profile: &Profile,
        draft: &WorklogBoundaryDraft,
    ) -> Result<WorklogResult, AppError> {
        let response = self
            .client
            .post(format!(
                "{}/4/worklogs",
                self.base_url.trim_end_matches('/')
            ))
            .bearer_auth(tempo_token)
            .json(&TempoCreateWorklogRequest::from(draft))
            .send()
            .map_err(|error| AppError::network(format!("tempo create failed: {error}")))?;

        match response.status().as_u16() {
            200 | 201 => {
                let body: TempoCreateWorklogResponse = response.json().map_err(|error| {
                    AppError::network(format!("parse tempo create response: {error}"))
                })?;
                let worklog_id = body.tempo_worklog_id.to_string();
                let start = draft.start_date.and_time(draft.start_time);
                let end = start + chrono::Duration::seconds(i64::from(draft.time_spent_seconds));
                Ok(WorklogResult {
                    worklog_id: worklog_id.clone(),
                    issue_key: draft.issue_key.clone(),
                    issue_id: Some(draft.issue_id.clone()),
                    start,
                    end,
                    duration_seconds: draft.time_spent_seconds,
                    tempo_url: format!("{}/tempo/worklog/{}", profile.jira_url, worklog_id),
                    description: draft.description.clone(),
                })
            }
            401 | 403 => Err(AppError::auth("Tempo token rejected")),
            409 => Err(AppError::conflict("duplicate worklog detected")),
            status => Err(AppError::network(format!(
                "tempo create returned HTTP {status}"
            ))),
        }
    }

    fn list_worklogs(
        &self,
        tempo_token: &str,
        account_id: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<WorklogResult>, AppError> {
        let mut offset = 0_usize;
        let limit = 1000_usize;
        let mut all_results = Vec::new();

        loop {
            let response =
                self.send_list_request(tempo_token, account_id, from, to, offset, limit)?;

            match response.status().as_u16() {
                200 => {
                    let body: TempoListResponse = response.json().map_err(|error| {
                        AppError::network(format!("parse tempo list response: {error}"))
                    })?;
                    let page_len = body.results.len();
                    for item in body.results {
                        all_results.push(Self::parse_worklog(item)?);
                    }
                    if page_len < limit {
                        return Ok(all_results);
                    }
                    offset += limit;
                    if offset >= limit * 20 {
                        return Err(AppError::network("tempo list exceeded pagination cap"));
                    }
                }
                status => {
                    return Err(AppError::network(format!(
                        "tempo list returned HTTP {status}"
                    )));
                }
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct TempoCreateWorklogRequest {
    #[serde(rename = "issueId")]
    issue_id: String,
    #[serde(rename = "authorAccountId")]
    author_account_id: String,
    #[serde(rename = "startDate")]
    start_date: String,
    #[serde(rename = "startTime")]
    start_time: String,
    #[serde(rename = "timeSpentSeconds")]
    time_spent_seconds: u32,
    description: Option<String>,
    attributes: Vec<String>,
}

impl From<&WorklogBoundaryDraft> for TempoCreateWorklogRequest {
    fn from(value: &WorklogBoundaryDraft) -> Self {
        Self {
            issue_id: value.issue_id.clone(),
            author_account_id: value.author_account_id.clone(),
            start_date: value.start_date.format("%Y-%m-%d").to_string(),
            start_time: value.start_time.format("%H:%M:%S").to_string(),
            time_spent_seconds: value.time_spent_seconds,
            description: value.description.clone(),
            attributes: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct TempoCreateWorklogResponse {
    #[serde(rename = "tempoWorklogId")]
    tempo_worklog_id: u64,
}

#[derive(Debug, Deserialize, Default)]
struct TempoListResponse {
    #[serde(default)]
    results: Vec<TempoWorklogItem>,
}

#[derive(Debug, Deserialize)]
struct TempoWorklogItem {
    #[serde(rename = "tempoWorklogId")]
    tempo_worklog_id: u64,
    issue: TempoIssueRef,
    #[serde(rename = "startDate")]
    start_date: String,
    #[serde(rename = "startTime")]
    start_time: String,
    #[serde(rename = "timeSpentSeconds")]
    time_spent_seconds: u32,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TempoIssueRef {
    #[serde(default)]
    key: Option<String>,
    #[serde(rename = "self", default)]
    self_url: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    id: Option<u64>,
}

impl TempoIssueRef {
    fn issue_key(&self) -> String {
        if let Some(key) = &self.key
            && crate::time_parse::is_issue_key(key)
        {
            return key.clone();
        }

        if let Some(self_url) = &self.self_url {
            let path = self_url.split('?').next().unwrap_or(self_url);
            if let Some(key) = path.rsplit('/').next()
                && crate::time_parse::is_issue_key(key)
            {
                return key.to_owned();
            }
        }

        self.id
            .map(|value| value.to_string())
            .unwrap_or_else(|| String::from("unknown-issue"))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveTime};

    use super::*;
    use crate::config::default_profile;

    #[test]
    fn validates_tempo_token_with_mock_server() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("GET", "/4/worklogs")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "1".into()))
            .with_status(200)
            .create();
        let client = HttpTempoClient::new(server.url());

        client
            .validate_token("tempo-token")
            .expect("tempo token validates");
    }

    #[test]
    fn creates_worklog_with_mock_server() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("POST", "/4/worklogs")
            .match_header(
                "content-type",
                mockito::Matcher::Regex("application/json".into()),
            )
            .with_status(201)
            .with_body(r#"{"tempoWorklogId":9001}"#)
            .create();

        let client = HttpTempoClient::new(server.url());
        let profile = default_profile("UTC");
        let result = client
            .create_worklog(
                "tempo-token",
                &profile,
                &WorklogBoundaryDraft {
                    issue_id: String::from("10001"),
                    issue_key: String::from("TK-1"),
                    author_account_id: String::from("acct-1"),
                    start_date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                    start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    time_spent_seconds: 3600,
                    description: Some(String::from("fix flaky test")),
                },
            )
            .expect("tempo create succeeds");

        assert_eq!(result.worklog_id, "9001");
        assert_eq!(result.issue_key, "TK-1");
        assert_eq!(result.duration_seconds, 3600);
    }

    #[test]
    fn lists_worklogs_with_mock_server() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("GET", "/4/worklogs/user/acct-1")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("from".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("to".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("offset".into(), "0".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "1000".into()),
            ]))
            .with_status(200)
            .with_body(r#"{"self":"https://api.tempo.io/4/worklogs/user/acct-1?offset=0&limit=1000","metadata":{"count":1,"offset":0,"limit":1000},"results":[{"tempoWorklogId":9001,"issue":{"self":"https://example.atlassian.net/rest/api/3/issue/TK-1","id":10001},"startDate":"2026-04-01","startTime":"09:00:00","timeSpentSeconds":3600,"description":"work"}]}"#)
            .create();

        let client = HttpTempoClient::new(server.url());
        let worklogs = client
            .list_worklogs(
                "tempo-token",
                "acct-1",
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            )
            .expect("tempo list succeeds");

        assert_eq!(worklogs.len(), 1);
        assert_eq!(worklogs[0].issue_key, "TK-1");
    }

    #[test]
    fn paginates_worklog_list_until_short_page() {
        let mut server = mockito::Server::new();
        let first_page = format!(
            "{{\"results\":[{}]}}",
            (0..1000)
                .map(|index| format!(
                    "{{\"tempoWorklogId\":{},\"issue\":{{\"self\":\"https://example.atlassian.net/rest/api/3/issue/TK-{}\",\"id\":{}}},\"startDate\":\"2026-04-01\",\"startTime\":\"09:00:00\",\"timeSpentSeconds\":60,\"description\":null}}",
                    index,
                    index,
                    index
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        let second_page = r#"{"self":"https://api.tempo.io/4/worklogs/user/acct-1?offset=1000&limit=1000","metadata":{"count":1,"offset":1000,"limit":1000},"results":[{"tempoWorklogId":1000,"issue":{"self":"https://example.atlassian.net/rest/api/3/issue/TK-1000","id":11000},"startDate":"2026-04-01","startTime":"10:00:00","timeSpentSeconds":120,"description":"follow-up"}]}"#;

        let _page_one = server
            .mock("GET", "/4/worklogs/user/acct-1")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("from".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("to".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("offset".into(), "0".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "1000".into()),
            ]))
            .with_status(200)
            .with_body(first_page)
            .create();
        let _page_two = server
            .mock("GET", "/4/worklogs/user/acct-1")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("from".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("to".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("offset".into(), "1000".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "1000".into()),
            ]))
            .with_status(200)
            .with_body(second_page)
            .create();

        let client = HttpTempoClient::new(server.url());
        let worklogs = client
            .list_worklogs(
                "tempo-token",
                "acct-1",
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            )
            .expect("tempo list paginates");

        assert_eq!(worklogs.len(), 1001);
        assert_eq!(worklogs.last().unwrap().issue_key, "TK-1000");
    }

    #[test]
    fn list_worklogs_keeps_numeric_issue_id_when_key_missing() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("GET", "/4/worklogs/user/acct-1")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("from".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("to".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("offset".into(), "0".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "1000".into()),
            ]))
            .with_status(200)
            .with_body(r#"{"results":[{"tempoWorklogId":9001,"issue":{"self":"https://example.atlassian.net/rest/api/3/issue/1641146","id":1641146},"startDate":"2026-04-01","startTime":"09:00:00","timeSpentSeconds":3600,"description":"work"}]}"#)
            .create();

        let client = HttpTempoClient::new(server.url());
        let worklogs = client
            .list_worklogs(
                "tempo-token",
                "acct-1",
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            )
            .expect("tempo list succeeds");

        assert_eq!(worklogs[0].issue_key, "1641146");
        assert_eq!(worklogs[0].issue_id.as_deref(), Some("1641146"));
    }

    #[test]
    fn retries_rate_limited_worklog_list() {
        let mut server = mockito::Server::new();
        let _retry = server
            .mock("GET", "/4/worklogs/user/acct-1")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("from".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("to".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("offset".into(), "0".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "1000".into()),
            ]))
            .with_status(429)
            .expect(2)
            .create();
        let _success = server
            .mock("GET", "/4/worklogs/user/acct-1")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("from".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("to".into(), "2026-04-01".into()),
                mockito::Matcher::UrlEncoded("offset".into(), "0".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "1000".into()),
            ]))
            .with_status(200)
            .with_body(r#"{"results":[]}"#)
            .create();

        let client = HttpTempoClient::new(server.url());
        let worklogs = client
            .list_worklogs(
                "tempo-token",
                "acct-1",
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            )
            .expect("tempo list retries and succeeds");

        assert!(worklogs.is_empty());
    }
}
