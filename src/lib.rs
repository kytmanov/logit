pub mod alias;
pub mod atomic;
pub mod calendar;
pub mod cli;
pub mod clock;
pub mod config;
pub mod domain;
pub mod dryrun;
pub mod error;
pub mod format;
pub mod jira;
pub mod mcp;
pub mod paths;
pub mod profile;
pub mod secrets;
pub mod service;
pub mod stats;
pub mod style;
pub mod tempo;
pub mod time_parse;
pub mod ui;

use std::ffi::OsString;
use std::process::ExitCode;

use crate::cli::parse_cli;
use crate::clock::{SystemClock, today_in_profile};
use crate::config::{delete_alias, load_config, save_config};
use crate::domain::{
    Alias, AliasCommand, CacheCommand, CommandInput, ConfigCommand, DomainCommand, LogInput,
    McpCommand, StatRangeInput,
};
use crate::error::AppError;
use crate::jira::{HttpJiraClient, JiraClient};
use crate::profile::{resolve_profile, resolve_profile_name};
use crate::secrets::{FileSecretStore, SecretStore};
use crate::service::doctor::collect_doctor_info;
use crate::service::stats::get_stats;
use crate::service::types::{GetStatsRequest, ProfileRef, RequestScope, StatsCacheMode};
use crate::style::Style;
use crate::tempo::HttpTempoClient;
use crate::time_parse::build_worklog_draft;
use crate::ui::{
    render_alias_delete, render_alias_set, render_aliases, render_cache_clear, render_doctor,
    render_error, render_log_success, render_stats, run_setup,
};
use tracing::{debug, info};

const HELP_SUFFIX: &str = "\
Usage: {bin} <COMMAND> [OPTIONS]\n\
\n\
Logging time:\n\
  {bin} <ISSUE> <DURATION>            log duration to issue (e.g. TK-1234 1h 30m)\n\
  {bin} <DURATION> <ISSUE>            duration first (e.g. 8h TK-1234)\n\
  {bin} <ALIAS> [DURATION]            log via alias (uses alias default duration)\n\
  {bin} <DATE> <START> - <DATE> <END> <ISSUE>\n\
                                      log a time range\n\
  Options: --date <YYYY-MM-DD>, -m <message>, --dry-run, --force\n\
\n\
Commands:\n\
  setup        configure Tempo and Jira access\n\
  stat [WHEN]  worklog summary (today|week|last week|<month>|<year>|<YYYY-MM-DD>)\n\
               add --details to include individual worklog rows\n\
  alias        manage aliases (list, set, delete)\n\
  mcp          install MCP client config (claude|codex|opencode)\n\
  cache clear  clear calendar cache\n\
  doctor       show paths, schema, active profile\n\
  config       path | edit\n\
\n\
Global options:\n\
  --profile <name>     use a non-default profile\n\
  --config-dir <path>  override config dir\n\
  --data-dir <path>    override data dir\n\
  --cache-dir <path>   override cache dir\n\
  --verbose            include extra detail rows\n\
  -h, --help           print help\n\
\n\
Environment:\n\
  NO_COLOR / LOGIT_NO_COLOR   disable ANSI color\n\
  LOGIT_ASCII=1               force ASCII glyphs\n\
  LOGIT_LOG=debug             tracing log level on stderr\n";

pub fn run<I>(argv: I) -> ExitCode
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = argv.into_iter();
    let bin = args
        .next()
        .and_then(|value| value.into_string().ok())
        .and_then(|value| {
            value
                .rsplit(std::path::MAIN_SEPARATOR)
                .next()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from("logit"));

    let cli_args: Vec<String> = args.filter_map(|value| value.into_string().ok()).collect();

    if cli_args.is_empty() {
        print_help(&bin);
        return ExitCode::SUCCESS;
    }

    if cli_args.len() == 1 && (cli_args[0] == "-h" || cli_args[0] == "--help") {
        print_help(&bin);
        return ExitCode::SUCCESS;
    }

    match parse_cli(cli_args) {
        Ok(parsed) => {
            init_logging(parsed.verbose);
            let verbose = parsed.verbose;
            match dispatch(parsed.command, verbose) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    let style = Style::for_stderr();
                    eprintln!("{}", render_error(&error, &style));
                    ExitCode::from(error.exit_code())
                }
            }
        }
        Err(error) => {
            let style = Style::for_stderr();
            eprintln!("{}", render_error(&error, &style));
            print_help(&bin);
            ExitCode::from(error.exit_code())
        }
    }
}

pub fn dispatch(command: DomainCommand, verbose: bool) -> Result<(), AppError> {
    let clock = SystemClock;
    let jira = HttpJiraClient::default();
    let tempo = HttpTempoClient::default();

    match command {
        DomainCommand::Setup(input) => run_setup(input),
        DomainCommand::Log(input) => run_log_command(input, &clock, &jira, &tempo, verbose),
        DomainCommand::Stat(input) => run_stat_command(input, verbose),
        DomainCommand::Alias(input) => run_alias_command(input),
        DomainCommand::Mcp(command) => run_mcp_command(command),
        DomainCommand::Cache(input) => run_cache_command(input),
        DomainCommand::Doctor(input) => run_doctor_command(input),
        DomainCommand::Config(input) => run_config_command(input),
    }
}

fn run_mcp_command(command: McpCommand) -> Result<(), AppError> {
    match command {
        McpCommand::Install(input) => {
            let outcome = crate::mcp::install::install_target(input)?;
            println!("{outcome}");
            Ok(())
        }
    }
}

pub fn run_log_command<C, J, T>(
    input: LogInput,
    clock: &C,
    jira: &J,
    tempo: &T,
    verbose: bool,
) -> Result<(), AppError>
where
    C: crate::clock::Clock,
    J: crate::jira::JiraClient,
    T: crate::tempo::TempoClient,
{
    let dirs = crate::paths::resolve_dirs(&input.paths)?;
    let config = load_config(&dirs)?;
    let profile_name = resolve_profile_name(&config, &input.profile)?.to_owned();
    let profile = resolve_profile(&config, &input.profile)?;
    let resolved = crate::alias::resolve_log_target(profile, &input)?;
    let draft = build_worklog_draft(&resolved, profile, clock)?;

    let style = Style::for_stdout();
    let today = today_in_profile(profile);

    if resolved.dry_run {
        println!(
            "{}",
            crate::dryrun::render_draft(&draft, &profile_name, profile, today, &style, verbose)
        );
        return Ok(());
    }

    let store = FileSecretStore::new(dirs.clone())?;
    let secrets = store
        .load_profile(&profile_name)?
        .ok_or_else(|| AppError::config("missing secrets; run `logit setup`"))?;

    let account_id = profile
        .account_id
        .clone()
        .ok_or_else(|| AppError::config("missing account_id; run `logit setup`"))?;
    let issue_id = jira.resolve_issue_id(
        &profile.jira_url,
        &profile.email,
        &secrets.jira_token,
        &draft.issue_key,
    )?;
    let existing = tempo.list_worklogs(
        &secrets.tempo_token,
        &account_id,
        draft.start.date(),
        draft.end.date(),
    )?;
    if !resolved.force
        && let Some(worklog) = existing.iter().find(|worklog| {
            worklog.issue_id.as_deref() == Some(issue_id.as_str())
                && worklog.start == draft.start
                && worklog.end == draft.end
        })
    {
        return Err(AppError::conflict(format!(
            "duplicate worklog detected: existing worklog ID {}. Re-run with --force to override",
            worklog.worklog_id
        )));
    }
    let boundary = tempo.to_boundary_draft(issue_id, account_id, &draft);
    let mut result = tempo.create_worklog(&secrets.tempo_token, profile, &boundary)?;
    result.issue_key = draft.issue_key.clone();

    info!(category = "worklog", issue_key = %draft.issue_key, worklog_id = %result.worklog_id, "worklog created");
    println!(
        "{}",
        render_log_success(&result, profile, today, &style, verbose)
    );
    Ok(())
}

fn run_stat_command(input: StatRangeInput, verbose: bool) -> Result<(), AppError> {
    let output = get_stats(GetStatsRequest {
        scope: RequestScope {
            profile: ProfileRef::Named(input.profile.clone()),
            paths: input.paths.clone(),
        },
        selector: input.selector.clone(),
        details: input.details,
        cache_mode: if input.no_calendar {
            StatsCacheMode::NoCalendar
        } else if input.refresh_calendar {
            StatsCacheMode::Refresh
        } else {
            StatsCacheMode::UseCache
        },
    })
    .map_err(AppError::from)?;

    let dirs = crate::paths::resolve_dirs(&input.paths)?;
    let config = load_config(&dirs)?;
    let profile_name = output
        .meta
        .profile_used
        .clone()
        .unwrap_or_else(|| String::from("default"));
    let profile = resolve_profile(&config, &profile_name)?;
    let style = Style::for_stdout();
    let today = today_in_profile(profile);
    println!(
        "{}",
        render_stats(
            &output.data.report,
            &output.data.selector,
            profile,
            &profile_name,
            output.data.start,
            output.data.end,
            today,
            output.data.details,
            &style,
            verbose,
        )
    );
    Ok(())
}

fn run_alias_command(command: AliasCommand) -> Result<(), AppError> {
    match command {
        AliasCommand::Set(input) => {
            let dirs = crate::paths::resolve_dirs(&input.paths)?;
            let mut config = load_config(&dirs)?;
            let profile_name = resolve_profile_name(&config, &input.profile)?.to_owned();
            crate::alias::validate_alias_name(&input.name)?;
            if !crate::time_parse::is_issue_key(&input.issue_key) {
                return Err(AppError::validation(format!(
                    "invalid issue key for alias: {}",
                    input.issue_key
                )));
            }
            if input.validate_remote {
                let profile_view = resolve_profile(&config, &input.profile)?;
                let store = FileSecretStore::new(dirs.clone())?;
                let secrets = store
                    .load_profile(&profile_name)?
                    .ok_or_else(|| AppError::config("missing secrets; run `logit setup`"))?;
                let jira = HttpJiraClient::default();
                jira.resolve_issue_id(
                    &profile_view.jira_url,
                    &profile_view.email,
                    &secrets.jira_token,
                    &input.issue_key,
                )?;
            }
            let profile = config
                .profiles
                .get_mut(&profile_name)
                .ok_or_else(|| AppError::config(format!("unknown profile: {}", input.profile)))?;
            let previous = profile.aliases.insert(
                input.name.clone(),
                Alias {
                    key: input.issue_key.clone(),
                    default_duration: input.default_duration,
                    default_message: input.default_message.clone(),
                },
            );
            save_config(&dirs, &config)?;
            let style = Style::for_stdout();
            println!(
                "{}",
                render_alias_set(&input.name, &input.issue_key, previous.as_ref(), &style)
            );
            Ok(())
        }
        AliasCommand::List(input) => {
            let dirs = crate::paths::resolve_dirs(&input.paths)?;
            let config = load_config(&dirs)?;
            let profile_name = resolve_profile_name(&config, &input.profile)?.to_owned();
            let profile = resolve_profile(&config, &input.profile)?;
            let style = Style::for_stdout();
            println!("{}", render_aliases(profile, &profile_name, &style));
            Ok(())
        }
        AliasCommand::Delete(input) => {
            let dirs = crate::paths::resolve_dirs(&input.paths)?;
            let mut config = load_config(&dirs)?;
            let profile_name = resolve_profile_name(&config, &input.profile)?.to_owned();
            delete_alias(&mut config, &profile_name, &input.name)?;
            save_config(&dirs, &config)?;
            let style = Style::for_stdout();
            println!("{}", render_alias_delete(&input.name, &style));
            Ok(())
        }
    }
}

fn run_cache_command(command: CacheCommand) -> Result<(), AppError> {
    match command {
        CacheCommand::Clear(input) => {
            let dirs = crate::paths::resolve_dirs(&input.paths)?;
            let config = load_config(&dirs)?;
            let profile_name = resolve_profile_name(&config, &input.profile)?;
            let path = dirs.calendar_file(profile_name);
            if path.exists() {
                std::fs::remove_file(&path).map_err(|error| {
                    AppError::config(format!("remove {}: {error}", path.display()))
                })?;
            }
            let style = Style::for_stdout();
            println!("{}", render_cache_clear(profile_name, &style));
            Ok(())
        }
    }
}

fn run_doctor_command(input: CommandInput) -> Result<(), AppError> {
    let style = Style::for_stdout();
    let doctor = collect_doctor_info(&RequestScope {
        profile: ProfileRef::Named(input.profile.clone()),
        paths: input.paths.clone(),
    })
    .map_err(AppError::from)?;
    let active_profile = doctor
        .data
        .active_profile
        .as_deref()
        .zip(doctor.data.profile_timezone.as_deref())
        .map(|(name, tz)| {
            (
                name,
                crate::domain::Profile {
                    jira_url: String::new(),
                    email: String::new(),
                    account_id: None,
                    tz: tz.to_owned(),
                    work_hours: crate::domain::WorkHours {
                        start: String::new(),
                        end: String::new(),
                    },
                    working_days: Vec::new(),
                    time_format: crate::domain::TimeFormat::TwentyFourHour,
                    aliases: std::collections::BTreeMap::new(),
                },
            )
        });
    let active_ref = active_profile
        .as_ref()
        .map(|(name, profile)| (*name, profile));
    println!(
        "{}",
        render_doctor(
            std::path::Path::new(&doctor.data.config.path),
            std::path::Path::new(&doctor.data.data.path),
            std::path::Path::new(&doctor.data.cache.path),
            doctor.data.schema_version,
            doctor.data.supported_schema_version,
            active_ref,
            &style,
        )
    );
    Ok(())
}

fn run_config_command(command: ConfigCommand) -> Result<(), AppError> {
    match command {
        ConfigCommand::Path(input) => {
            let dirs = crate::paths::resolve_dirs(&input.paths)?;
            println!("{}", dirs.config_file().display());
            Ok(())
        }
        ConfigCommand::Edit(input) => {
            let dirs = crate::paths::resolve_dirs(&input.paths)?;
            let path = dirs.config_file();

            if !path.exists() {
                return Err(AppError::config("run `logit setup`"));
            }

            let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
                if cfg!(windows) {
                    String::from("notepad")
                } else {
                    String::from("vi")
                }
            });

            let status = std::process::Command::new(&editor)
                .arg(&path)
                .status()
                .map_err(|error| AppError::config(format!("launch {editor}: {error}")))?;
            if !status.success() {
                return Err(AppError::config(format!(
                    "editor exited with status {status}"
                )));
            }

            match load_config(&dirs) {
                Ok(_) => Ok(()),
                Err(error) => {
                    let invalid_path = path.with_extension("toml.invalid");
                    std::fs::copy(&path, &invalid_path).map_err(|copy_error| {
                        AppError::config(format!(
                            "{}; also failed to preserve invalid file at {}: {copy_error}",
                            error,
                            invalid_path.display()
                        ))
                    })?;
                    Err(AppError::config(format!(
                        "{}; preserved invalid edit at {}",
                        error,
                        invalid_path.display()
                    )))
                }
            }
        }
    }
}

fn print_help(bin: &str) {
    print!("{}", HELP_SUFFIX.replace("{bin}", bin));
}

fn init_logging(_verbose: bool) {
    use tracing_subscriber::EnvFilter;
    let filter = std::env::var("LOGIT_LOG").unwrap_or_else(|_| String::from("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&filter).unwrap_or_else(|_| EnvFilter::new("warn")))
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
    debug!("logging initialized");
}
