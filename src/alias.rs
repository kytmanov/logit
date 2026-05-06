use crate::domain::{LogInput, LogKind, Profile};
use crate::error::AppError;

pub fn validate_alias_name(name: &str) -> Result<(), AppError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(AppError::validation("alias name must not be empty"));
    };
    if !first.is_ascii_lowercase() {
        return Err(AppError::validation(format!(
            "invalid alias name '{}': must start with a lowercase ASCII letter",
            name
        )));
    }
    if name.len() > 64
        || !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    {
        return Err(AppError::validation(format!(
            "invalid alias name '{}': use [a-z][a-z0-9_-]{{0,63}}",
            name
        )));
    }
    Ok(())
}

pub fn resolve_log_target(profile: &Profile, input: &LogInput) -> Result<LogInput, AppError> {
    if crate::time_parse::is_issue_key(&input.issue_token) {
        return Ok(input.clone());
    }

    let Some(alias) = profile.aliases.get(&input.issue_token) else {
        let suggestion = did_you_mean(profile, &input.issue_token)
            .map(|value| format!("\ndid you mean '{value}'?"))
            .unwrap_or_default();
        return Err(AppError::not_found(format!(
            "unknown issue key or alias '{}'{}",
            input.issue_token, suggestion
        )));
    };

    let mut resolved = input.clone();
    if let LogKind::Duration { seconds, .. } = &resolved.kind
        && seconds.is_none()
        && alias.default_duration.is_none()
    {
        return Err(AppError::validation(format!(
            "duration required for alias '{}'",
            input.issue_token
        )));
    }
    resolved.issue_token = alias.key.clone();
    if resolved.description.is_none() {
        resolved.description = alias.default_message.clone();
    }
    if let LogKind::Duration { seconds, .. } = &mut resolved.kind
        && seconds.is_none()
    {
        *seconds = alias.default_duration;
    }

    Ok(resolved)
}

fn did_you_mean(profile: &Profile, token: &str) -> Option<String> {
    profile
        .aliases
        .keys()
        .filter_map(|candidate| {
            let distance = strsim::levenshtein(candidate, token);
            (distance <= 2).then_some((distance, candidate.clone()))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::default_profile;
    use crate::domain::{Alias, LogInput, LogKind, PathOverrides};

    use super::*;

    #[test]
    fn resolves_alias_and_applies_defaults() {
        let mut profile = default_profile("UTC");
        profile.aliases.insert(
            String::from("standup"),
            Alias {
                key: String::from("TC-3"),
                default_duration: Some(1800),
                default_message: Some(String::from("daily standup")),
            },
        );

        let resolved = resolve_log_target(
            &profile,
            &LogInput {
                profile: String::from("default"),
                paths: PathOverrides::default(),
                issue_token: String::from("standup"),
                description: None,
                dry_run: false,
                force: false,
                kind: LogKind::Duration {
                    seconds: None,
                    date: None,
                },
            },
        )
        .expect("alias resolves");

        assert_eq!(resolved.issue_token, "TC-3");
        assert_eq!(resolved.description.as_deref(), Some("daily standup"));
        assert_eq!(
            resolved.kind,
            LogKind::Duration {
                seconds: Some(1800),
                date: None
            }
        );
    }

    #[test]
    fn suggests_close_alias_name() {
        let mut profile = default_profile("UTC");
        profile.aliases = BTreeMap::from([(
            String::from("scrm"),
            Alias {
                key: String::from("TC-1"),
                default_duration: None,
                default_message: None,
            },
        )]);

        let error = resolve_log_target(
            &profile,
            &LogInput {
                profile: String::from("default"),
                paths: PathOverrides::default(),
                issue_token: String::from("scrum"),
                description: None,
                dry_run: false,
                force: false,
                kind: LogKind::Duration {
                    seconds: Some(3600),
                    date: None,
                },
            },
        )
        .expect_err("unknown alias rejected");

        assert!(error.to_string().contains("did you mean 'scrm'?"));
    }
}
