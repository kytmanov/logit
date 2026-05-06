use crate::calendar::{CalendarContext, cached_calendar_context, calendar_context};
use crate::jira::{HttpJiraClient, JiraClient};
use crate::secrets::{FileSecretStore, SecretStore};
use crate::service::types::{
    GetStatsRequest, GetStatsResult, ServiceError, ServiceMeta, ServiceOutput, ServiceWarning,
    StatsCacheMode, resolve_scope,
};
use crate::stats::{build_stat_report, select_range};
use crate::tempo::{HttpTempoClient, TempoClient};

pub fn get_stats(request: GetStatsRequest) -> Result<ServiceOutput<GetStatsResult>, ServiceError> {
    if matches!(request.selector, crate::domain::StatSelector::Year(_)) && request.details {
        return Err(ServiceError {
            code: "details_not_supported",
            category: "validation",
            message: String::from("--details is not supported for year stats"),
            remediation: Some(String::from(
                "Use month, week, or a single date with details enabled.",
            )),
            retryable: false,
        });
    }

    let resolved = resolve_scope(&request.scope)?;
    let store = FileSecretStore::new(resolved.dirs.clone()).map_err(ServiceError::from)?;
    let secrets = store
        .load_profile(&resolved.profile_name)
        .map_err(ServiceError::from)?
        .ok_or_else(|| ServiceError {
            code: "missing_secrets",
            category: "config",
            message: String::from("missing secrets; run `logit setup`"),
            remediation: Some(String::from(
                "Run `logit setup` for this profile before requesting stats.",
            )),
            retryable: false,
        })?;
    let account_id = resolved
        .profile
        .account_id
        .clone()
        .ok_or_else(|| ServiceError {
            code: "missing_account_id",
            category: "config",
            message: String::from("missing account_id; run `logit setup`"),
            remediation: Some(String::from(
                "Run `logit setup` again to store the Jira account id for this profile.",
            )),
            retryable: false,
        })?;

    let (label, start, end) =
        select_range(&request.selector, &resolved.profile).map_err(ServiceError::from)?;
    let tempo = HttpTempoClient::default();
    let jira = HttpJiraClient::default();
    let worklogs = tempo
        .list_worklogs(&secrets.tempo_token, &account_id, start, end)
        .map_err(ServiceError::from)?;
    let worklogs = hydrate_issue_keys(
        &jira,
        &resolved.profile.jira_url,
        &resolved.profile.email,
        &secrets.jira_token,
        worklogs,
    )
    .map_err(ServiceError::from)?;
    let calendar = match request.cache_mode {
        StatsCacheMode::UseCache => calendar_context(
            &resolved.dirs,
            &resolved.profile_name,
            &secrets.tempo_token,
            &account_id,
            &resolved.profile,
            start,
            end,
            false,
            false,
        )
        .map_err(ServiceError::from)?,
        StatsCacheMode::NoCalendar => CalendarContext::configured(&resolved.profile, start, end)
            .map_err(ServiceError::from)?,
        StatsCacheMode::Refresh => calendar_context(
            &resolved.dirs,
            &resolved.profile_name,
            &secrets.tempo_token,
            &account_id,
            &resolved.profile,
            start,
            end,
            false,
            true,
        )
        .map_err(ServiceError::from)?,
        StatsCacheMode::NoWrite => cached_calendar_context(
            &resolved.dirs,
            &resolved.profile_name,
            &resolved.profile,
            start,
            end,
        )
        .map_err(ServiceError::from)?
        .unwrap_or(
            CalendarContext::configured(&resolved.profile, start, end)
                .map_err(ServiceError::from)?,
        ),
    };

    let report = build_stat_report(&resolved.profile, &label, start, end, worklogs, &calendar)
        .map_err(ServiceError::from)?;
    let mut warnings = Vec::new();
    let degraded = !matches!(
        request.cache_mode,
        StatsCacheMode::Refresh | StatsCacheMode::UseCache
    );
    match request.cache_mode {
        StatsCacheMode::NoWrite => warnings.push(ServiceWarning {
            code: "no_write_cache_mode",
            message: String::from(
                "Stats used configured workday or existing cache only; no calendar refresh was written.",
            ),
        }),
        StatsCacheMode::NoCalendar => warnings.push(ServiceWarning {
            code: "no_calendar_mode",
            message: String::from(
                "Stats used configured workday only; Tempo schedule lookup was skipped.",
            ),
        }),
        StatsCacheMode::UseCache | StatsCacheMode::Refresh => {}
    }

    Ok(ServiceOutput {
        meta: ServiceMeta {
            profile_used: Some(resolved.profile_name),
            degraded,
            warnings,
        },
        data: GetStatsResult {
            selector: request.selector,
            start,
            end,
            details: request.details,
            report,
        },
    })
}

fn hydrate_issue_keys<J: JiraClient>(
    jira: &J,
    jira_url: &str,
    email: &str,
    token: &str,
    worklogs: Vec<crate::domain::WorklogResult>,
) -> Result<Vec<crate::domain::WorklogResult>, crate::error::AppError> {
    worklogs
        .into_iter()
        .map(|mut worklog| {
            if !crate::time_parse::is_issue_key(&worklog.issue_key)
                && let Some(issue_id) = worklog.issue_id.as_deref()
            {
                worklog.issue_key = jira.resolve_issue_key(jira_url, email, token, issue_id)?;
            }
            Ok(worklog)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::WorklogResult;
    use crate::domain::{PathOverrides, StatSelector};
    use crate::error::AppError;
    use crate::jira::JiraClient;
    use crate::service::types::{ProfileRef, RequestScope};
    use chrono::NaiveDateTime;

    #[derive(Debug)]
    struct StubJira;

    impl JiraClient for StubJira {
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
            _issue_key: &str,
        ) -> Result<String, AppError> {
            unreachable!()
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

    #[test]
    fn rejects_year_details_requests() {
        let error = get_stats(GetStatsRequest {
            scope: RequestScope {
                profile: ProfileRef::Active,
                paths: PathOverrides::default(),
            },
            selector: StatSelector::Year(2026),
            details: true,
            cache_mode: StatsCacheMode::UseCache,
        })
        .expect_err("year details should be rejected");

        assert_eq!(error.code, "details_not_supported");
        assert_eq!(error.category, "validation");
    }

    #[test]
    fn hydrates_numeric_issue_keys_from_jira() {
        let worklogs = vec![WorklogResult {
            worklog_id: String::from("w1"),
            issue_key: String::from("1641146"),
            issue_id: Some(String::from("1641146")),
            start: NaiveDateTime::parse_from_str("2026-04-01 09:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap(),
            end: NaiveDateTime::parse_from_str("2026-04-01 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            duration_seconds: 3600,
            tempo_url: String::new(),
            description: None,
        }];

        let hydrated = hydrate_issue_keys(
            &StubJira,
            "https://example.atlassian.net",
            "user@example.com",
            "jira-token",
            worklogs,
        )
        .expect("issue keys hydrate");

        assert_eq!(hydrated[0].issue_key, "TC-1641146");
    }

    #[test]
    fn named_default_profile_stays_named_default() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dirs = crate::paths::AppDirs {
            config: temp.path().join("config"),
            data: temp.path().join("data"),
            cache: temp.path().join("cache"),
        };
        let mut config = crate::config::default_config("UTC");
        config.profiles.insert(
            String::from("work"),
            crate::config::default_profile("Europe/Berlin"),
        );
        config.active = String::from("work");
        crate::config::save_config(&dirs, &config).expect("save config");

        let resolved = crate::service::types::resolve_scope(&RequestScope {
            profile: ProfileRef::Named(String::from("default")),
            paths: crate::domain::PathOverrides {
                config_dir: Some(dirs.config),
                data_dir: Some(dirs.data),
                cache_dir: Some(dirs.cache),
            },
        })
        .expect("resolve scope");

        assert_eq!(resolved.profile_name, "default");
        assert_eq!(resolved.profile.tz, "UTC");
    }
}
