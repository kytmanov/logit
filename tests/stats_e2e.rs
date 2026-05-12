use std::fs;

use predicates::prelude::*;

fn write_profile_fixture(
    temp: &tempfile::TempDir,
    jira_url: &str,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    let cache_dir = temp.path().join("cache");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::create_dir_all(&data_dir).expect("data dir");
    fs::create_dir_all(&cache_dir).expect("cache dir");
    fs::write(
        config_dir.join("config.toml"),
        format!(
            r#"schema_version = 1
active = "default"

[profiles.default]
jira_url = "{jira_url}"
email = "user@example.com"
account_id = "acct-1"
tz = "UTC"
time_format = "TwentyFourHour"
working_days = ["Mon", "Tue", "Wed", "Thu", "Fri"]

[profiles.default.work_hours]
start = "09:00"
end = "17:00"

[profiles.default.aliases]
"#
        ),
    )
    .expect("config fixture");
    fs::write(
        data_dir.join("secrets.toml"),
        "[profiles.default]\ntempo_token = \"tempo-token\"\njira_token = \"jira-token\"\n",
    )
    .expect("secrets fixture");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            data_dir.join("secrets.toml"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("chmod 600");
    }

    (config_dir, data_dir, cache_dir)
}

#[test]
fn stat_yesterday_runs_end_to_end_against_mock_servers() {
    let mut tempo = mockito::Server::new();
    let mut jira = mockito::Server::new();
    let _tempo_mock = tempo
        .mock("GET", "/4/worklogs/user/acct-1")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("from".into(), "2026-05-11".into()),
            mockito::Matcher::UrlEncoded("to".into(), "2026-05-11".into()),
            mockito::Matcher::UrlEncoded("offset".into(), "0".into()),
            mockito::Matcher::UrlEncoded("limit".into(), "1000".into()),
        ]))
        .with_status(200)
        .with_body(
            r#"{"results":[{"tempoWorklogId":9001,"issue":{"self":"https://example.atlassian.net/rest/api/3/issue/1641146","id":1641146},"startDate":"2026-05-11","startTime":"09:00:00","timeSpentSeconds":3600,"description":"work"}]}"#,
        )
        .create();
    let _jira_mock = jira
        .mock("GET", "/rest/api/3/issue/1641146")
        .match_query(mockito::Matcher::UrlEncoded("fields".into(), "key".into()))
        .with_status(200)
        .with_body(r#"{"id":"1641146","key":"TK-1641146"}"#)
        .create();

    let temp = tempfile::tempdir().expect("tempdir");
    let (config_dir, data_dir, cache_dir) = write_profile_fixture(&temp, &jira.url());

    let mut cmd = assert_cmd::Command::cargo_bin("logit").expect("binary");
    cmd.env("LOGIT_TEMPO_BASE_URL", tempo.url())
        .env("TZ", "UTC")
        .arg("--config-dir")
        .arg(&config_dir)
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("stat")
        .arg("yesterday")
        .arg("--no-calendar")
        .assert()
        .success()
        .stdout(predicate::str::contains("Yesterday"))
        .stdout(predicate::str::contains("Mon May 11"))
        .stdout(predicate::str::contains("TK-1641146"))
        .stdout(predicate::str::contains("1h"));
}
