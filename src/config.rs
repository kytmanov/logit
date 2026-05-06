use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::NaiveTime;
use toml_edit::{DocumentMut, Item, TableLike};

use crate::atomic::atomic_write;
use crate::domain::{Alias, Config, Profile, TimeFormat, WeekdayName, WorkHours};
use crate::error::AppError;
use crate::paths::AppDirs;

pub const SCHEMA_VERSION: u32 = 1;

pub fn load_config(dirs: &AppDirs) -> Result<Config, AppError> {
    let path = dirs.config_file();
    if !path.exists() {
        return Err(AppError::config("run `logit setup`"));
    }
    let raw = fs::read_to_string(&path)
        .map_err(|error| AppError::config(format!("read {}: {error}", path.display())))?;

    let mut document = raw
        .parse::<DocumentMut>()
        .map_err(|error| AppError::config(format!("parse {}: {error}", path.display())))?;
    backfill_schema_version(&mut document);

    let config: Config = toml_edit::de::from_document(document)
        .map_err(|error| AppError::config(format!("parse {}: {error}", path.display())))?;

    validate_schema(&config)?;
    validate_profiles(&config)?;
    Ok(config)
}

pub fn save_config(dirs: &AppDirs, config: &Config) -> Result<(), AppError> {
    validate_schema(config)?;
    validate_profiles(config)?;

    let path = dirs.config_file();
    ensure_parent(&path)?;
    let mut document = toml_edit::ser::to_document(config)
        .map_err(|error| AppError::config(format!("serialize config: {error}")))?;
    sort_config_tables(&mut document);

    let serialized = match fs::read_to_string(&path) {
        Ok(raw) => match raw.parse::<DocumentMut>() {
            Ok(mut existing) => {
                merge_table_like(existing.as_table_mut(), document.as_table());
                sort_config_tables(&mut existing);
                existing.to_string()
            }
            Err(_) => document.to_string(),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => document.to_string(),
        Err(error) => {
            return Err(AppError::config(format!(
                "read {}: {error}",
                path.display()
            )));
        }
    };

    atomic_write(&path, serialized.as_bytes())
}

pub fn default_config(timezone: &str) -> Config {
    let mut profiles = BTreeMap::new();
    profiles.insert(String::from("default"), default_profile(timezone));

    Config {
        schema_version: SCHEMA_VERSION,
        active: String::from("default"),
        profiles,
    }
}

pub fn default_profile(timezone: &str) -> Profile {
    Profile {
        jira_url: String::new(),
        email: String::new(),
        account_id: None,
        tz: timezone.to_owned(),
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
        aliases: BTreeMap::<String, Alias>::new(),
    }
}

pub fn active_profile_name(config: &Config) -> &str {
    &config.active
}

pub fn upsert_profile(config: &mut Config, name: String, profile: Profile) {
    config.profiles.insert(name, profile);
}

pub fn delete_alias(config: &mut Config, profile: &str, alias: &str) -> Result<(), AppError> {
    let profile = config
        .profiles
        .get_mut(profile)
        .ok_or_else(|| AppError::config(format!("unknown profile: {profile}")))?;
    if profile.aliases.remove(alias).is_none() {
        return Err(AppError::not_found(format!("unknown alias '{alias}'")));
    }
    Ok(())
}

pub fn ensure_parent(path: &Path) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::config(format!("create dir {}: {error}", parent.display()))
        })?;
    }

    Ok(())
}

fn validate_schema(config: &Config) -> Result<(), AppError> {
    if config.schema_version > SCHEMA_VERSION {
        return Err(AppError::config(format!(
            "config written by a newer logit (schema v{}, this binary supports v{}); upgrade with a newer release",
            config.schema_version, SCHEMA_VERSION
        )));
    }

    Ok(())
}

fn validate_profiles(config: &Config) -> Result<(), AppError> {
    for (name, profile) in &config.profiles {
        if !crate::domain::is_valid_timezone(&profile.tz) {
            return Err(AppError::config(format!(
                "invalid timezone '{0}' in profile '{name}'",
                profile.tz
            )));
        }
        validate_work_hours(name, &profile.work_hours)?;
        if profile.working_days.is_empty() {
            return Err(AppError::config(format!(
                "profile '{name}' must define at least one working day"
            )));
        }
    }

    Ok(())
}

fn validate_work_hours(profile_name: &str, work_hours: &WorkHours) -> Result<(), AppError> {
    let start = NaiveTime::parse_from_str(&work_hours.start, "%H:%M").map_err(|_| {
        AppError::config(format!(
            "invalid work_hours.start '{}' in profile '{profile_name}'",
            work_hours.start
        ))
    })?;
    let end = NaiveTime::parse_from_str(&work_hours.end, "%H:%M").map_err(|_| {
        AppError::config(format!(
            "invalid work_hours.end '{}' in profile '{profile_name}'",
            work_hours.end
        ))
    })?;
    if end <= start {
        return Err(AppError::config(format!(
            "work_hours.end must be after work_hours.start in profile '{profile_name}'"
        )));
    }

    Ok(())
}

fn backfill_schema_version(document: &mut DocumentMut) {
    if !document.as_table().contains_key("schema_version") {
        document.as_table_mut().insert(
            "schema_version",
            toml_edit::value(i64::from(SCHEMA_VERSION)),
        );
    }
}

fn merge_item(existing: &mut Item, updated: &Item) {
    if existing.is_table_like() && updated.is_table_like() {
        let existing_table = existing
            .as_table_like_mut()
            .expect("table-like item stays table-like");
        let updated_table = updated
            .as_table_like()
            .expect("table-like item stays table-like");
        merge_table_like(existing_table, updated_table);
    } else {
        *existing = updated.clone();
    }
}

fn merge_table_like(existing: &mut dyn TableLike, updated: &dyn TableLike) {
    let existing_keys: Vec<String> = existing.iter().map(|(key, _)| key.to_owned()).collect();
    for key in existing_keys {
        if !updated.contains_key(&key) {
            existing.remove(&key);
        }
    }

    let updated_keys: Vec<String> = updated.iter().map(|(key, _)| key.to_owned()).collect();
    for key in updated_keys {
        let updated_item = updated.get(&key).expect("iter key exists");
        if existing.contains_key(&key) {
            let existing_item = existing.get_mut(&key).expect("existing key exists");
            merge_item(existing_item, updated_item);
        } else {
            existing.insert(&key, updated_item.clone());
        }
    }
}

fn sort_config_tables(document: &mut DocumentMut) {
    let Some(profiles_item) = document.as_table_mut().get_mut("profiles") else {
        return;
    };
    let Some(profiles_table) = profiles_item.as_table_like_mut() else {
        return;
    };

    profiles_table.sort_values();

    let profile_names: Vec<String> = profiles_table
        .iter()
        .map(|(key, _)| key.to_owned())
        .collect();
    for profile_name in profile_names {
        let Some(profile_item) = profiles_table.get_mut(&profile_name) else {
            continue;
        };
        let Some(profile_table) = profile_item.as_table_like_mut() else {
            continue;
        };
        let Some(aliases_item) = profile_table.get_mut("aliases") else {
            continue;
        };
        let Some(aliases_table) = aliases_item.as_table_like_mut() else {
            continue;
        };
        aliases_table.sort_values();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Alias;
    use crate::paths::AppDirs;

    fn temp_dirs() -> (tempfile::TempDir, AppDirs) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        (
            temp,
            AppDirs {
                config: root.join("config"),
                data: root.join("data"),
                cache: root.join("cache"),
            },
        )
    }

    #[test]
    fn saves_and_loads_profile_config() {
        let (_temp, dirs) = temp_dirs();
        let config = default_config("UTC");

        save_config(&dirs, &config).expect("config saves");
        let loaded = load_config(&dirs).expect("config loads");

        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.active, "default");
        assert_eq!(loaded.profiles["default"].tz, "UTC");
    }

    #[test]
    fn default_profile_has_expected_workday_defaults() {
        let profile = default_profile("UTC");

        assert_eq!(profile.work_hours.start, "09:00");
        assert_eq!(profile.work_hours.end, "17:00");
        assert_eq!(profile.working_days.len(), 5);
    }

    #[test]
    fn rejects_newer_schema_version() {
        let (_temp, dirs) = temp_dirs();
        let mut config = default_config("UTC");
        config.schema_version = SCHEMA_VERSION + 1;

        let error = save_config(&dirs, &config).expect_err("schema version rejected");

        assert!(
            error
                .to_string()
                .contains("config written by a newer logit")
        );
    }

    #[test]
    fn rejects_invalid_profile_timezone() {
        let (_temp, dirs) = temp_dirs();
        let mut config = default_config("UTC");
        config.profiles.get_mut("default").unwrap().tz = String::from("Mars/Olympus");

        let error = save_config(&dirs, &config).expect_err("bad timezone rejected");

        assert!(error.to_string().contains("invalid timezone"));
    }

    #[test]
    fn accepts_fixed_offset_profile_timezone() {
        let (_temp, dirs) = temp_dirs();
        let mut config = default_config("UTC");
        config.profiles.get_mut("default").unwrap().tz = String::from("-07:00");

        save_config(&dirs, &config).expect("offset timezone accepted");
        let loaded = load_config(&dirs).expect("config loads");

        assert_eq!(loaded.profiles["default"].tz, "-07:00");
    }

    #[test]
    fn load_config_treats_missing_schema_version_as_v1() {
        let (_temp, dirs) = temp_dirs();
        let path = dirs.config_file();
        ensure_parent(&path).expect("config parent");
        fs::write(
            &path,
            r#"active = "default"

[profiles.default]
jira_url = "https://example.atlassian.net"
email = "user@example.com"
account_id = "acct-1"
tz = "UTC"
working_days = ["Mon", "Tue", "Wed", "Thu", "Fri"]
time_format = "TwentyFourHour"

[profiles.default.work_hours]
start = "09:00"
end = "17:00"

[profiles.default.aliases]
"#,
        )
        .expect("config fixture");

        let loaded = load_config(&dirs).expect("config loads");

        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn save_config_preserves_existing_comments() {
        let (_temp, dirs) = temp_dirs();
        let path = dirs.config_file();
        ensure_parent(&path).expect("config parent");
        fs::write(
            &path,
            r#"schema_version = 1
active = "default"

[profiles.default]
# weekly retro
jira_url = "https://example.atlassian.net"
email = "user@example.com"
account_id = "acct-1"
tz = "UTC"
working_days = ["Mon", "Tue", "Wed", "Thu", "Fri"]
time_format = "TwentyFourHour"

[profiles.default.work_hours]
start = "09:00"
end = "17:00"

[profiles.default.aliases]
retro = { key = "TC-1" }
"#,
        )
        .expect("config fixture");

        let mut loaded = load_config(&dirs).expect("config loads");
        loaded.profiles.get_mut("default").unwrap().aliases.insert(
            String::from("standup"),
            Alias {
                key: String::from("TC-2"),
                default_duration: Some(1800),
                default_message: Some(String::from("daily")),
            },
        );

        save_config(&dirs, &loaded).expect("config saves");

        let saved = fs::read_to_string(path).expect("saved config");
        assert!(saved.contains("# weekly retro"));
    }

    #[test]
    fn save_config_serializes_aliases_in_alphabetical_order() {
        let (_temp, dirs) = temp_dirs();
        let mut config = default_config("UTC");
        let aliases = &mut config.profiles.get_mut("default").unwrap().aliases;
        aliases.insert(
            String::from("zebra"),
            Alias {
                key: String::from("TC-9"),
                default_duration: None,
                default_message: None,
            },
        );
        aliases.insert(
            String::from("alpha"),
            Alias {
                key: String::from("TC-1"),
                default_duration: None,
                default_message: None,
            },
        );
        aliases.insert(
            String::from("mango"),
            Alias {
                key: String::from("TC-5"),
                default_duration: None,
                default_message: None,
            },
        );

        save_config(&dirs, &config).expect("config saves");

        let saved = fs::read_to_string(dirs.config_file()).expect("saved config");
        let alpha = saved.find("alpha").expect("alpha in config");
        let mango = saved.find("mango").expect("mango in config");
        let zebra = saved.find("zebra").expect("zebra in config");
        assert!(alpha < mango && mango < zebra);
    }
}
