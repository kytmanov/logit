use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::AppError;

pub trait JiraClient {
    fn validate_credentials(
        &self,
        jira_url: &str,
        email: &str,
        token: &str,
    ) -> Result<String, AppError>;
    fn resolve_issue_id(
        &self,
        jira_url: &str,
        email: &str,
        token: &str,
        issue_key: &str,
    ) -> Result<String, AppError>;
    fn resolve_issue_key(
        &self,
        jira_url: &str,
        email: &str,
        token: &str,
        issue_id: &str,
    ) -> Result<String, AppError>;
}

pub struct HttpJiraClient {
    client: Client,
    issue_cache: Mutex<HashMap<String, String>>,
}

impl std::fmt::Debug for HttpJiraClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpJiraClient").finish_non_exhaustive()
    }
}

impl Default for HttpJiraClient {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("jira reqwest client"),
            issue_cache: Mutex::new(HashMap::new()),
        }
    }
}

impl JiraClient for HttpJiraClient {
    fn validate_credentials(
        &self,
        jira_url: &str,
        email: &str,
        token: &str,
    ) -> Result<String, AppError> {
        let url = format!("{}/rest/api/3/myself", jira_url.trim_end_matches('/'));
        let response = self
            .client
            .get(url)
            .basic_auth(email, Some(token))
            .send()
            .map_err(|error| {
                AppError::network(format!("jira credentials request failed: {error}"))
            })?;

        match response.status().as_u16() {
            200 => {
                let body: JiraMyself = response
                    .json()
                    .map_err(|error| AppError::network(format!("parse jira response: {error}")))?;
                Ok(body.account_id)
            }
            401 | 403 => Err(AppError::auth("Jira credentials rejected")),
            status => Err(AppError::network(format!(
                "jira myself returned HTTP {status}"
            ))),
        }
    }

    fn resolve_issue_id(
        &self,
        jira_url: &str,
        email: &str,
        token: &str,
        issue_key: &str,
    ) -> Result<String, AppError> {
        let cache_key = issue_id_cache_key(jira_url, issue_key);
        if let Some(value) = self
            .issue_cache
            .lock()
            .expect("jira issue cache")
            .get(&cache_key)
            .cloned()
        {
            return Ok(value);
        }

        let url = format!(
            "{}/rest/api/3/issue/{}?fields=summary",
            jira_url.trim_end_matches('/'),
            issue_key
        );
        let response = self
            .client
            .get(url)
            .basic_auth(email, Some(token))
            .send()
            .map_err(|error| AppError::network(format!("jira issue lookup failed: {error}")))?;

        match response.status().as_u16() {
            200 => {
                let body: JiraIssueIdLookup = response.json().map_err(|error| {
                    AppError::network(format!("parse jira issue response: {error}"))
                })?;
                let mut cache = self.issue_cache.lock().expect("jira issue cache");
                cache.insert(cache_key, body.id.clone());
                cache.insert(issue_key_cache_key(jira_url, &body.id), body.key);
                Ok(body.id)
            }
            404 => Err(AppError::not_found(format!(
                "unknown issue key or alias '{issue_key}'"
            ))),
            401 | 403 => Err(AppError::auth("Jira credentials rejected")),
            status => Err(AppError::network(format!(
                "jira issue lookup returned HTTP {status}"
            ))),
        }
    }

    fn resolve_issue_key(
        &self,
        jira_url: &str,
        email: &str,
        token: &str,
        issue_id: &str,
    ) -> Result<String, AppError> {
        let cache_key = issue_key_cache_key(jira_url, issue_id);
        if let Some(value) = self
            .issue_cache
            .lock()
            .expect("jira issue cache")
            .get(&cache_key)
            .cloned()
        {
            return Ok(value);
        }

        let url = format!(
            "{}/rest/api/3/issue/{}?fields=key",
            jira_url.trim_end_matches('/'),
            issue_id
        );
        let response = self
            .client
            .get(url)
            .basic_auth(email, Some(token))
            .send()
            .map_err(|error| AppError::network(format!("jira issue lookup failed: {error}")))?;

        match response.status().as_u16() {
            200 => {
                let body: JiraIssueKeyLookup = response.json().map_err(|error| {
                    AppError::network(format!("parse jira issue response: {error}"))
                })?;
                let mut cache = self.issue_cache.lock().expect("jira issue cache");
                cache.insert(cache_key, body.key.clone());
                cache.insert(issue_id_cache_key(jira_url, &body.key), body.id);
                Ok(body.key)
            }
            404 => Err(AppError::not_found(format!(
                "unknown issue id '{issue_id}'"
            ))),
            401 | 403 => Err(AppError::auth("Jira credentials rejected")),
            status => Err(AppError::network(format!(
                "jira issue lookup returned HTTP {status}"
            ))),
        }
    }
}

fn issue_id_cache_key(jira_url: &str, issue_key: &str) -> String {
    format!("id::{}::{issue_key}", jira_url.trim_end_matches('/'))
}

fn issue_key_cache_key(jira_url: &str, issue_id: &str) -> String {
    format!("key::{}::{issue_id}", jira_url.trim_end_matches('/'))
}

#[derive(Debug, Deserialize)]
struct JiraMyself {
    #[serde(rename = "accountId")]
    account_id: String,
}

#[derive(Debug, Deserialize)]
struct JiraIssueIdLookup {
    id: String,
    key: String,
}

#[derive(Debug, Deserialize)]
struct JiraIssueKeyLookup {
    id: String,
    key: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_credentials_with_mock_jira() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("GET", "/rest/api/3/myself")
            .with_status(200)
            .with_body(r#"{"accountId":"acct-1"}"#)
            .create();

        let client = HttpJiraClient::default();
        let account_id = client
            .validate_credentials(&server.url(), "user@example.com", "jira-token")
            .expect("jira validates");

        assert_eq!(account_id, "acct-1");
    }

    #[test]
    fn resolves_issue_id_with_mock_jira() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("GET", "/rest/api/3/issue/TK-1")
            .match_query(mockito::Matcher::UrlEncoded(
                "fields".into(),
                "summary".into(),
            ))
            .with_status(200)
            .with_body(r#"{"id":"10001","key":"TK-1"}"#)
            .create();

        let client = HttpJiraClient::default();
        let issue_id = client
            .resolve_issue_id(&server.url(), "user@example.com", "jira-token", "TK-1")
            .expect("issue resolves");

        assert_eq!(issue_id, "10001");
    }

    #[test]
    fn resolves_issue_key_with_mock_jira() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("GET", "/rest/api/3/issue/10001")
            .match_query(mockito::Matcher::UrlEncoded("fields".into(), "key".into()))
            .with_status(200)
            .with_body(r#"{"id":"10001","key":"TK-1"}"#)
            .create();

        let client = HttpJiraClient::default();
        let issue_key = client
            .resolve_issue_key(&server.url(), "user@example.com", "jira-token", "10001")
            .expect("issue key resolves");

        assert_eq!(issue_key, "TK-1");
    }
}
