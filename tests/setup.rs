use logit::domain::{PathOverrides, SetupInput, SetupValues, TimeFormat, WeekdayName, WorkHours};
use logit::error::AppError;
use logit::jira::JiraClient;
use logit::tempo::TempoClient;
use logit::ui::run_setup_with_clients;

struct RejectingTempo;

impl TempoClient for RejectingTempo {
    fn validate_token(&self, _tempo_token: &str) -> Result<(), AppError> {
        Err(AppError::auth("Tempo token rejected"))
    }

    fn to_boundary_draft(
        &self,
        _issue_id: String,
        _author_account_id: String,
        _draft: &logit::domain::WorklogDraft,
    ) -> logit::domain::WorklogBoundaryDraft {
        unreachable!()
    }

    fn create_worklog(
        &self,
        _tempo_token: &str,
        _profile: &logit::domain::Profile,
        _draft: &logit::domain::WorklogBoundaryDraft,
    ) -> Result<logit::domain::WorklogResult, AppError> {
        unreachable!()
    }

    fn list_worklogs(
        &self,
        _tempo_token: &str,
        _account_id: &str,
        _from: chrono::NaiveDate,
        _to: chrono::NaiveDate,
    ) -> Result<Vec<logit::domain::WorklogResult>, AppError> {
        unreachable!()
    }
}

struct PassingJira;

impl JiraClient for PassingJira {
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
        _issue_key: &str,
    ) -> Result<String, AppError> {
        Ok(String::from("10001"))
    }

    fn resolve_issue_key(
        &self,
        _jira_url: &str,
        _email: &str,
        _token: &str,
        _issue_id: &str,
    ) -> Result<String, AppError> {
        Ok(String::from("TK-1"))
    }
}

#[test]
fn setup_failure_persists_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = PathOverrides {
        config_dir: Some(temp.path().join("config")),
        data_dir: Some(temp.path().join("data")),
        cache_dir: Some(temp.path().join("cache")),
    };

    let error = run_setup_with_clients(
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
        &PassingJira,
        &RejectingTempo,
    )
    .expect_err("setup should fail");

    assert!(error.to_string().contains("Tempo token rejected"));
    let dirs = logit::paths::resolve_dirs(&paths).expect("dirs");
    assert!(!dirs.config_file().exists());
    assert!(!dirs.secrets_file().exists());
}
