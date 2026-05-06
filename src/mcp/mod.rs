pub mod install;
pub mod server;
pub mod tools;

use std::ffi::OsString;
use std::process::ExitCode;

use crate::clock::SystemClock;
use crate::domain::PathOverrides;
use crate::error::AppError;
use crate::jira::HttpJiraClient;
use crate::service::types::{ProfileRef, RequestScope};
use crate::tempo::HttpTempoClient;
use crate::ui::render_error;

const HELP_SUFFIX: &str = "\
Usage: {bin} [OPTIONS]\n\
\n\
Run the logit MCP server over stdio.\n\
\n\
Options:\n\
  --profile <name>     default profile for tool calls\n\
  --config-dir <path>  override config dir\n\
  --data-dir <path>    override data dir\n\
  --cache-dir <path>   override cache dir\n\
  --enable-write-tools expose mutating MCP tools like log_time\n\
  -h, --help           print help\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub default_scope: RequestScope,
    pub enable_write_tools: bool,
}

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
        .unwrap_or_else(|| String::from("logit-mcp"));

    let cli_args: Vec<String> = args.filter_map(|value| value.into_string().ok()).collect();

    if cli_args.len() == 1 && (cli_args[0] == "-h" || cli_args[0] == "--help") {
        print_help(&bin);
        return ExitCode::SUCCESS;
    }

    match parse_args(cli_args) {
        Ok(runtime) => {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let jira = HttpJiraClient::default();
            let tempo = HttpTempoClient::default();
            match server::serve_stdio(
                stdin.lock(),
                stdout.lock(),
                runtime,
                SystemClock,
                &jira,
                &tempo,
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    let style = crate::style::Style::for_stderr();
                    eprintln!("{}", render_error(&error, &style));
                    ExitCode::from(error.exit_code())
                }
            }
        }
        Err(error) => {
            let style = crate::style::Style::for_stderr();
            eprintln!("{}", render_error(&error, &style));
            print_help(&bin);
            ExitCode::from(error.exit_code())
        }
    }
}

fn parse_args(args: Vec<String>) -> Result<RuntimeConfig, AppError> {
    let mut profile = ProfileRef::Active;
    let mut paths = PathOverrides::default();
    let mut enable_write_tools = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--enable-write-tools" => {
                enable_write_tools = true;
                index += 1;
            }
            "--profile" => {
                let value = option_value(&args, index, "--profile")?;
                profile = ProfileRef::Named(value.clone());
                index += 2;
            }
            "--config-dir" => {
                let value = option_value(&args, index, "--config-dir")?;
                paths.config_dir = Some(value.into());
                index += 2;
            }
            "--data-dir" => {
                let value = option_value(&args, index, "--data-dir")?;
                paths.data_dir = Some(value.into());
                index += 2;
            }
            "--cache-dir" => {
                let value = option_value(&args, index, "--cache-dir")?;
                paths.cache_dir = Some(value.into());
                index += 2;
            }
            other => {
                return Err(AppError::validation(format!("unknown argument: {other}")));
            }
        }
    }

    Ok(RuntimeConfig {
        default_scope: RequestScope { profile, paths },
        enable_write_tools,
    })
}

fn option_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a String, AppError> {
    let Some(value) = args.get(index + 1) else {
        return Err(AppError::validation(format!("missing value for {flag}")));
    };
    if value.starts_with('-') {
        return Err(AppError::validation(format!("missing value for {flag}")));
    }
    Ok(value)
}

fn print_help(bin: &str) {
    print!("{}", HELP_SUFFIX.replace("{bin}", bin));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_flag_like_profile_value() {
        let error = parse_args(vec![String::from("--profile"), String::from("--help")])
            .expect_err("flag-like profile rejected");

        assert!(error.to_string().contains("missing value for --profile"));
    }

    #[test]
    fn rejects_flag_like_config_dir_value() {
        let error = parse_args(vec![
            String::from("--config-dir"),
            String::from("--cache-dir"),
        ])
        .expect_err("flag-like config-dir rejected");

        assert!(error.to_string().contains("missing value for --config-dir"));
    }

    #[test]
    fn parses_enable_write_tools_flag() {
        let runtime = parse_args(vec![String::from("--enable-write-tools")])
            .expect("write-tools flag parses");

        assert!(runtime.enable_write_tools);
        assert_eq!(runtime.default_scope.profile, ProfileRef::Active);
    }
}
