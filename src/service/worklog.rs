use crate::alias::resolve_log_target;
use crate::clock::Clock;
use crate::domain::{LogInput, LogKind};
use crate::jira::JiraClient;
use crate::secrets::{FileSecretStore, SecretStore};
use crate::service::types::{
    LogTimeRequest, LogTimeResult, PreviewLogRequest, PreviewLogResult, ServiceError, ServiceMeta,
    ServiceOutput, resolve_scope,
};
use crate::tempo::TempoClient;
use crate::time_parse::build_worklog_draft;

pub fn preview_log_time<C: Clock>(
    clock: &C,
    request: PreviewLogRequest,
) -> Result<ServiceOutput<PreviewLogResult>, ServiceError> {
    let resolved = resolve_scope(&request.scope)?;
    let alias_used = (!crate::time_parse::is_issue_key(&request.issue_or_alias))
        .then(|| request.issue_or_alias.clone());
    let input = LogInput {
        profile: resolved.profile_name.clone(),
        paths: request.scope.paths.clone(),
        issue_token: request.issue_or_alias,
        description: request.message,
        dry_run: true,
        force: request.force,
        kind: LogKind::Duration {
            seconds: request.duration_seconds,
            date: request.date,
        },
    };
    let log_input = resolve_log_target(&resolved.profile, &input).map_err(ServiceError::from)?;
    let draft =
        build_worklog_draft(&log_input, &resolved.profile, clock).map_err(ServiceError::from)?;

    Ok(ServiceOutput {
        meta: ServiceMeta {
            profile_used: Some(resolved.profile_name),
            degraded: false,
            warnings: Vec::new(),
        },
        data: PreviewLogResult {
            issue_key: draft.issue_key.clone(),
            draft,
            alias_used,
        },
    })
}

pub fn log_time<C: Clock, J: JiraClient, T: TempoClient>(
    clock: &C,
    jira: &J,
    tempo: &T,
    request: LogTimeRequest,
) -> Result<ServiceOutput<LogTimeResult>, ServiceError> {
    let resolved = resolve_scope(&request.scope)?;
    let input = LogInput {
        profile: resolved.profile_name.clone(),
        paths: request.scope.paths.clone(),
        issue_token: request.issue_or_alias,
        description: request.message,
        dry_run: false,
        force: request.force,
        kind: LogKind::Duration {
            seconds: request.duration_seconds,
            date: request.date,
        },
    };
    let log_input = resolve_log_target(&resolved.profile, &input).map_err(ServiceError::from)?;
    let draft =
        build_worklog_draft(&log_input, &resolved.profile, clock).map_err(ServiceError::from)?;
    let store = FileSecretStore::new(resolved.dirs.clone()).map_err(ServiceError::from)?;
    let secrets = store
        .load_profile(&resolved.profile_name)
        .map_err(ServiceError::from)?
        .ok_or_else(|| ServiceError {
            code: "missing_secrets",
            category: "config",
            message: String::from("missing secrets; run `logit setup`"),
            remediation: Some(String::from("Run `logit setup` before logging work.")),
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
                "Run `logit setup` again to store your Jira account id.",
            )),
            retryable: false,
        })?;

    let issue_id = jira
        .resolve_issue_id(
            &resolved.profile.jira_url,
            &resolved.profile.email,
            &secrets.jira_token,
            &draft.issue_key,
        )
        .map_err(ServiceError::from)?;
    let existing = tempo
        .list_worklogs(
            &secrets.tempo_token,
            &account_id,
            draft.start.date(),
            draft.end.date(),
        )
        .map_err(ServiceError::from)?;
    if !request.force
        && let Some(worklog) = existing.iter().find(|worklog| {
            worklog.issue_id.as_deref() == Some(issue_id.as_str())
                && worklog.start == draft.start
                && worklog.end == draft.end
        })
    {
        return Err(ServiceError {
            code: "duplicate_worklog",
            category: "conflict",
            message: format!(
                "duplicate worklog detected: existing worklog ID {}. Re-run with --force to override",
                worklog.worklog_id
            ),
            remediation: Some(String::from(
                "Review the existing worklog or repeat the request with force if the duplicate is intentional.",
            )),
            retryable: false,
        });
    }

    let boundary = tempo.to_boundary_draft(issue_id, account_id, &draft);
    let mut result = tempo
        .create_worklog(&secrets.tempo_token, &resolved.profile, &boundary)
        .map_err(ServiceError::from)?;
    result.issue_key = draft.issue_key.clone();

    Ok(ServiceOutput {
        meta: ServiceMeta {
            profile_used: Some(resolved.profile_name),
            degraded: false,
            warnings: Vec::new(),
        },
        data: LogTimeResult { worklog: result },
    })
}
