use std::env;
use std::fs::{self, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use directories::ProjectDirs;
use pidjezdy_core::config::{Config, ConfigError};
use pidjezdy_pid::{PidClientError, PidRequestError};
use thiserror::Error;

mod cache;
mod departures;
mod output;

use departures::{DepartureQueryError, query_departures};
use output::{OutputError, OutputFormat, write_departures};

const CONFIG_ENV: &str = "PIDJEZDY_CONFIG";
const CONFIG_FILE: &str = "config.toml";

pub const DEFAULT_CONFIG_TEMPLATE: &str = r#"# pidjezdy configuration

[display]
max_departures = 3

# Reserve space for important routes before filling the remaining result slots.
[[display.route_quotas]]
line = "123"
headsign = "City centre"
minimum_departures = 2

[fetch]
minutes_after = 120
api_limit = 20

[[boarding_points]]
name = "Nearby stop"
stop_ids = ["U123Z1P"]
walking_minutes = 4
safety_buffer_minutes = 2

[[boarding_points.routes]]
line = "123"
headsign = "City centre"
"#;

#[derive(Debug, Parser)]
#[command(name = "pidjezdy", version, about)]
pub struct Cli {
    /// Use this configuration file instead of the platform default.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show the next reachable configured departures.
    Departures {
        /// Apply a hard result limit, allocating quota slots fairly when constrained.
        #[arg(long, value_parser = parse_departure_limit)]
        limit: Option<usize>,
        /// Select human-readable or machine-readable output.
        #[arg(long, value_enum, default_value_t)]
        format: OutputFormat,
    },
    /// Inspect and manage user configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Generate a completion script for a supported shell.
    Completions {
        /// Shell whose completion script should be generated.
        shell: Shell,
    },
}

fn parse_departure_limit(value: &str) -> Result<usize, String> {
    const ERROR: &str = "limit must be an integer between 1 and 20";
    let limit = value.parse::<usize>().map_err(|_| ERROR.to_owned())?;
    if (1..=20).contains(&limit) {
        Ok(limit)
    } else {
        Err(ERROR.to_owned())
    }
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Print the resolved configuration path.
    Path,
    /// Create a commented starter configuration.
    Init,
    /// Parse and validate the active configuration.
    Check,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("could not determine the platform configuration directory")]
    ConfigDirectoryUnavailable,
    #[error("configuration already exists at {0}")]
    ConfigAlreadyExists(PathBuf),
    #[error("could not create configuration directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not create configuration {path}: {source}")]
    CreateConfig {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not read configuration {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid configuration {path}: {source}")]
    InvalidConfig { path: PathBuf, source: ConfigError },
    #[error("could not build PID departure request: {0}")]
    DepartureRequest(#[source] PidRequestError),
    #[error("could not create PID client: {source}; cached fallback unavailable: {cache}")]
    CreatePidClient {
        source: PidClientError,
        cache: String,
    },
    #[error("could not fetch PID departures: {source}; cached fallback unavailable: {cache}")]
    FetchDepartures {
        source: PidClientError,
        cache: String,
    },
    #[error("could not serialize JSON output: {0}")]
    SerializeOutput(#[source] serde_json::Error),
    #[error("could not write command output: {0}")]
    WriteOutput(#[source] std::io::Error),
    #[error("could not write command diagnostics: {0}")]
    WriteDiagnostics(#[source] std::io::Error),
}

impl From<DepartureQueryError> for AppError {
    fn from(error: DepartureQueryError) -> Self {
        match error {
            DepartureQueryError::Request(source) => Self::DepartureRequest(source),
            DepartureQueryError::Unavailable { live, cache } => match live {
                departures::LiveDepartureError::CreateClient(source) => Self::CreatePidClient {
                    source,
                    cache: cache.to_string(),
                },
                departures::LiveDepartureError::Fetch(source) => Self::FetchDepartures {
                    source,
                    cache: cache.to_string(),
                },
            },
        }
    }
}

impl From<OutputError> for AppError {
    fn from(error: OutputError) -> Self {
        match error {
            OutputError::Json(source) => Self::SerializeOutput(source),
            OutputError::Write(source) => Self::WriteOutput(source),
        }
    }
}

/// Run the command using process arguments and environment.
///
/// # Errors
///
/// Returns configuration path, filesystem, parsing, or validation failures.
pub fn run_from_env() -> Result<(), AppError> {
    let cli = Cli::parse();
    let env_path = env::var_os(CONFIG_ENV).map(PathBuf::from);
    let mut stdout = std::io::stdout();
    let styled_text = stdout.is_terminal()
        && env::var_os("NO_COLOR").is_none()
        && env::var_os("TERM").is_none_or(|term| term != "dumb");
    run(
        cli,
        env_path.as_deref(),
        styled_text,
        &mut stdout,
        &mut std::io::stderr(),
    )
}

/// Resolve the configuration path without reading it.
///
/// Explicit CLI configuration wins over the environment, which wins over the
/// platform-standard directory.
///
/// # Errors
///
/// Returns an error when the platform has no discoverable configuration path.
pub fn resolve_config_path(
    explicit: Option<&Path>,
    environment: Option<&Path>,
) -> Result<PathBuf, AppError> {
    if let Some(path) = explicit.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(path.to_owned());
    }
    if let Some(path) = environment.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(path.to_owned());
    }
    ProjectDirs::from("", "", "pidjezdy")
        .map(|dirs| dirs.config_dir().join(CONFIG_FILE))
        .ok_or(AppError::ConfigDirectoryUnavailable)
}

fn run(
    cli: Cli,
    env_path: Option<&Path>,
    styled_text: bool,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<(), AppError> {
    match cli.command {
        Command::Departures { limit, format } => {
            let path = resolve_config_path(cli.config.as_deref(), env_path)?;
            let config = load_config(&path)?;
            let query = query_departures(&config, limit.unwrap_or(config.display.max_departures))?;
            if let Some(warning) = &query.cache_warning {
                writeln!(diagnostics, "pidjezdy: warning: {warning}")
                    .map_err(AppError::WriteDiagnostics)?;
            }
            write_departures(output, format, &query, styled_text)?;
            Ok(())
        }
        Command::Config { command } => {
            let path = resolve_config_path(cli.config.as_deref(), env_path)?;
            match command {
                ConfigCommand::Path => {
                    writeln!(output, "{}", path.display()).map_err(AppError::WriteOutput)
                }
                ConfigCommand::Init => init_config(&path, output),
                ConfigCommand::Check => {
                    load_config(&path)?;
                    writeln!(output, "configuration is valid: {}", path.display())
                        .map_err(AppError::WriteOutput)
                }
            }
        }
        Command::Completions { shell } => {
            let mut command = Cli::command();
            let mut script = Vec::new();
            generate(shell, &mut command, "pidjezdy", &mut script);
            output.write_all(&script).map_err(AppError::WriteOutput)
        }
    }
}

fn init_config(path: &Path, output: &mut impl Write) -> Result<(), AppError> {
    if path.exists() {
        return Err(AppError::ConfigAlreadyExists(path.to_owned()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| AppError::CreateConfigDirectory {
            path: parent.to_owned(),
            source,
        })?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| AppError::CreateConfig {
            path: path.to_owned(),
            source,
        })?;
    file.write_all(DEFAULT_CONFIG_TEMPLATE.as_bytes())
        .map_err(|source| AppError::CreateConfig {
            path: path.to_owned(),
            source,
        })?;
    writeln!(output, "created {}", path.display()).map_err(AppError::WriteOutput)
}

fn load_config(path: &Path) -> Result<Config, AppError> {
    let input = fs::read_to_string(path).map_err(|source| AppError::ReadConfig {
        path: path.to_owned(),
        source,
    })?;
    Config::from_toml(&input).map_err(|source| AppError::InvalidConfig {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn explicit_path_wins_over_environment() {
        let explicit = Path::new("explicit.toml");
        let environment = Path::new("environment.toml");
        assert_eq!(
            resolve_config_path(Some(explicit), Some(environment)).unwrap(),
            explicit
        );
    }

    #[test]
    fn environment_wins_over_platform_default() {
        let environment = Path::new("environment.toml");
        assert_eq!(
            resolve_config_path(None, Some(environment)).unwrap(),
            environment
        );
    }

    #[test]
    fn empty_explicit_path_falls_back_to_environment() {
        let environment = Path::new("environment.toml");
        assert_eq!(
            resolve_config_path(Some(Path::new("")), Some(environment)).unwrap(),
            environment
        );
    }

    #[test]
    fn empty_environment_path_falls_back_to_platform_default() {
        assert_eq!(
            resolve_config_path(None, Some(Path::new(""))).unwrap(),
            resolve_config_path(None, None).unwrap()
        );
    }

    #[test]
    fn init_creates_valid_config_and_refuses_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/config.toml");
        let mut output = Vec::new();

        init_config(&path, &mut output).unwrap();
        load_config(&path).unwrap();
        let original = fs::read_to_string(&path).unwrap();

        assert!(matches!(
            init_config(&path, &mut output),
            Err(AppError::ConfigAlreadyExists(existing)) if existing == path
        ));
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    fn config_check_reports_success() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, DEFAULT_CONFIG_TEMPLATE).unwrap();
        let cli = Cli::try_parse_from([
            OsString::from("pidjezdy"),
            OsString::from("--config"),
            path.clone().into_os_string(),
            OsString::from("config"),
            OsString::from("check"),
        ])
        .unwrap();
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();

        run(cli, None, false, &mut output, &mut diagnostics).unwrap();

        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("configuration is valid")
        );
    }

    #[test]
    fn departures_accepts_bounded_limit_and_json_format() {
        let cli =
            Cli::try_parse_from(["pidjezdy", "departures", "--limit", "2", "--format", "json"])
                .unwrap();

        assert!(matches!(
            cli.command,
            Command::Departures {
                limit: Some(2),
                format: OutputFormat::Json
            }
        ));
    }

    #[test]
    fn departures_rejects_limits_outside_the_supported_range() {
        for limit in ["0", "21", "not-a-number"] {
            let error = Cli::try_parse_from(["pidjezdy", "departures", "--limit", limit])
                .unwrap_err()
                .to_string();
            assert!(error.contains("limit must be an integer between 1 and 20"));
        }
    }

    #[test]
    fn completions_generates_a_shell_script_without_configuration() {
        let cli = Cli::try_parse_from(["pidjezdy", "completions", "bash"]).unwrap();
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();

        run(cli, None, false, &mut output, &mut diagnostics).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("pidjezdy"));
        assert!(output.contains("departures"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn output_errors_preserve_the_io_error_source() {
        let error = AppError::WriteOutput(std::io::Error::other("closed output"));
        assert!(std::error::Error::source(&error).is_some());
    }
}
