use std::fs;

use predicates::prelude::*;
use tempfile::TempDir;

fn write_config_fixture(
    temp: &TempDir,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    let cache_dir = temp.path().join("cache");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::create_dir_all(&data_dir).expect("data dir");
    fs::create_dir_all(&cache_dir).expect("cache dir");
    fs::write(
        config_dir.join("config.toml"),
        r#"schema_version = 1
active = "default"

[profiles.default]
jira_url = "https://example.atlassian.net"
email = "user@example.com"
account_id = "acct-1"
tz = "America/Los_Angeles"
time_format = "TwentyFourHour"
working_days = ["Mon", "Tue", "Wed", "Thu", "Fri"]

[profiles.default.work_hours]
start = "09:00"
end = "17:00"

[profiles.default.aliases.standup]
key = "TK-42"
default_duration = 1800
default_message = "daily standup"
"#,
    )
    .expect("config fixture");

    (config_dir, data_dir, cache_dir)
}

#[test]
fn dry_run_accepts_trailing_absolute_date_end_to_end() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (config_dir, data_dir, cache_dir) = write_config_fixture(&temp);

    let mut cmd = assert_cmd::Command::cargo_bin("logit").expect("binary");
    cmd.arg("--config-dir")
        .arg(&config_dir)
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("--verbose")
        .arg("3h")
        .arg("TK-1234")
        .arg("2026-05-11")
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry-run"))
        .stdout(predicate::str::contains("TK-1234"))
        .stdout(predicate::str::contains("Mon May 11"))
        .stdout(predicate::str::contains("14:00 → 17:00"));
}

#[test]
fn dry_run_accepts_trailing_yesterday_alias_end_to_end() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (config_dir, data_dir, cache_dir) = write_config_fixture(&temp);

    let mut cmd = assert_cmd::Command::cargo_bin("logit").expect("binary");
    cmd.env("TZ", "UTC")
        .arg("--config-dir")
        .arg(&config_dir)
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("standup")
        .arg("yesterday")
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("TK-42"))
        .stdout(predicate::str::contains("daily standup"))
        .stdout(predicate::str::contains("16:30 → 17:00"));
}

#[test]
fn dry_run_accepts_compact_duration_alias_end_to_end() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (config_dir, data_dir, cache_dir) = write_config_fixture(&temp);

    let mut cmd = assert_cmd::Command::cargo_bin("logit").expect("binary");
    cmd.arg("--config-dir")
        .arg(&config_dir)
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("1h15m")
        .arg("standup")
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("TK-42"))
        .stdout(predicate::str::contains("1h 15m"))
        .stdout(predicate::str::contains("Time"))
        .stdout(predicate::str::contains("daily standup"));
}

#[test]
fn log_rejects_mixed_flag_and_trailing_date_end_to_end() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (config_dir, data_dir, cache_dir) = write_config_fixture(&temp);

    let mut cmd = assert_cmd::Command::cargo_bin("logit").expect("binary");
    cmd.arg("--config-dir")
        .arg(&config_dir)
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("3h")
        .arg("TK-1234")
        .arg("yesterday")
        .arg("--date")
        .arg("2026-05-11")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "cannot use both --date and a trailing date argument",
        ));
}
