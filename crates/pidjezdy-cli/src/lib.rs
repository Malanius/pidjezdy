use std::env;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use directories::ProjectDirs;
use pidjezdy_core::config::{Config, ConfigError, MAX_DISPLAY_DEPARTURES};
use pidjezdy_pid::DEFAULT_ENDPOINT;
use thiserror::Error;

mod cache;
mod departures;
mod output;

pub use departures::{DepartureQueryError, DepartureUnavailable};
use departures::{configured_request, query_departures};
pub use output::{OutputError, distinct_causes};
use output::{OutputFormat, write_departures, write_json_error};

const CONFIG_ENV: &str = "PIDJEZDY_CONFIG";
const ENDPOINT_ENV: &str = "PIDJEZDY_ENDPOINT";
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
    /// Inspect and manage cached departures.
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
    /// Generate a completion script for a supported shell.
    Completions {
        /// Shell whose completion script should be generated.
        shell: Shell,
    },
}

fn parse_departure_limit(value: &str) -> Result<usize, String> {
    let error = || format!("limit must be an integer between 1 and {MAX_DISPLAY_DEPARTURES}");
    let limit = value.parse::<usize>().map_err(|_| error())?;
    if (1..=MAX_DISPLAY_DEPARTURES).contains(&limit) {
        Ok(limit)
    } else {
        Err(error())
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

#[derive(Debug, Subcommand)]
enum CacheCommand {
    /// Print the resolved departure cache path.
    Path,
    /// Remove the departure cache if it exists.
    Clear,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("could not determine the platform configuration directory")]
    ConfigDirectoryUnavailable,
    #[error("configuration already exists at {0}")]
    ConfigAlreadyExists(PathBuf),
    #[error("could not create configuration directory {path}")]
    CreateConfigDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not create configuration {path}")]
    CreateConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read configuration {path}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid configuration {path}")]
    InvalidConfig {
        path: PathBuf,
        #[source]
        source: ConfigError,
    },
    #[error("could not determine the platform cache directory")]
    CacheDirectoryUnavailable,
    #[error("could not remove departure cache {path}")]
    RemoveCache {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Departures(#[from] DepartureQueryError),
    #[error(transparent)]
    Output(#[from] OutputError),
    #[error("could not write command diagnostics")]
    WriteDiagnostics(#[source] std::io::Error),
}

impl AppError {
    fn json_kind(&self) -> &'static str {
        match self {
            Self::ConfigDirectoryUnavailable => "config_directory_unavailable",
            Self::ConfigAlreadyExists(_) => "config_already_exists",
            Self::CreateConfigDirectory { .. } => "config_directory_create_failed",
            Self::CreateConfig { .. } => "config_create_failed",
            Self::ReadConfig { .. } => "config_unreadable",
            Self::InvalidConfig { .. } => "config_invalid",
            Self::CacheDirectoryUnavailable => "cache_directory_unavailable",
            Self::RemoveCache { .. } => "cache_remove_failed",
            Self::Departures(DepartureQueryError::Request(_)) => "request_invalid",
            Self::Departures(DepartureQueryError::Unavailable(_)) => "departures_unavailable",
            Self::Output(_) => "output_failed",
            Self::WriteDiagnostics(_) => "diagnostics_write_failed",
        }
    }

    fn can_report_as_json(&self) -> bool {
        !matches!(self, Self::Output(OutputError::Write(_)))
    }
}

#[derive(Debug)]
pub struct CommandFailure {
    error: AppError,
    json_reported: bool,
}

impl CommandFailure {
    #[must_use]
    pub fn json_reported(&self) -> bool {
        self.json_reported
    }
}

impl fmt::Display for CommandFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for CommandFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.error)
    }
}

/// Run the command using process arguments and environment.
///
/// # Errors
///
/// Returns configuration path, filesystem, parsing, or validation failures.
pub fn run_from_env() -> Result<(), CommandFailure> {
    let cli = Cli::parse();
    let env_path = env::var_os(CONFIG_ENV).map(PathBuf::from);
    let env_endpoint = env::var(ENDPOINT_ENV).ok();
    let mut stdout = std::io::stdout();
    let styled_text = stdout.is_terminal()
        && env::var_os("NO_COLOR").is_none()
        && env::var_os("TERM").is_none_or(|term| term != "dumb");
    run(
        cli,
        env_path.as_deref(),
        env_endpoint.as_deref(),
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

fn resolve_endpoint(environment: Option<&str>) -> &str {
    environment
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty())
        .unwrap_or(DEFAULT_ENDPOINT)
}

fn run(
    cli: Cli,
    env_path: Option<&Path>,
    env_endpoint: Option<&str>,
    styled_text: bool,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<(), CommandFailure> {
    let json_errors = matches!(
        &cli.command,
        Command::Departures {
            format: OutputFormat::Json,
            ..
        }
    );
    let result = run_command(
        cli,
        env_path,
        resolve_endpoint(env_endpoint),
        styled_text,
        output,
        diagnostics,
    );
    match result {
        Ok(()) => Ok(()),
        Err(error) if json_errors && error.can_report_as_json() => {
            write_json_error(output, chrono::Utc::now(), &error).map_err(|error| {
                CommandFailure {
                    error: error.into(),
                    json_reported: false,
                }
            })?;
            Err(CommandFailure {
                error,
                json_reported: true,
            })
        }
        Err(error) => Err(CommandFailure {
            error,
            json_reported: false,
        }),
    }
}

fn run_command(
    cli: Cli,
    env_path: Option<&Path>,
    endpoint: &str,
    styled_text: bool,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<(), AppError> {
    match cli.command {
        Command::Departures { limit, format } => {
            let path = resolve_config_path(cli.config.as_deref(), env_path)?;
            let config = load_config(&path)?;
            let query = query_departures(
                &config,
                limit.unwrap_or(config.display.max_departures),
                endpoint,
            )?;
            for warning in &query.warnings {
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
                    writeln!(output, "{}", path.display()).map_err(OutputError::from)?;
                    Ok(())
                }
                ConfigCommand::Init => init_config(&path, output),
                ConfigCommand::Check => {
                    let config = load_config(&path)?;
                    configured_request(&config).map_err(DepartureQueryError::from)?;
                    writeln!(output, "configuration is valid: {}", path.display())
                        .map_err(OutputError::from)?;
                    Ok(())
                }
            }
        }
        Command::Cache { command } => {
            let path = cache::default_cache_path().ok_or(AppError::CacheDirectoryUnavailable)?;
            match command {
                CacheCommand::Path => {
                    writeln!(output, "{}", path.display()).map_err(OutputError::from)?;
                    Ok(())
                }
                CacheCommand::Clear => clear_cache(&path, output),
            }
        }
        Command::Completions { shell } => {
            let mut command = Cli::command();
            let mut script = Vec::new();
            generate(shell, &mut command, "pidjezdy", &mut script);
            output.write_all(&script).map_err(OutputError::from)?;
            Ok(())
        }
    }
}

fn clear_cache(path: &Path, output: &mut impl Write) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => writeln!(output, "removed {}", path.display()).map_err(OutputError::from)?,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            writeln!(output, "no cache file at {}", path.display()).map_err(OutputError::from)?;
        }
        Err(source) => {
            return Err(AppError::RemoveCache {
                path: path.to_owned(),
                source,
            });
        }
    }
    Ok(())
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
    writeln!(output, "created {}", path.display()).map_err(OutputError::from)?;
    Ok(())
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
    use pidjezdy_pid::MAX_API_LIMIT;
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

        run(cli, None, None, false, &mut output, &mut diagnostics).unwrap();

        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("configuration is valid")
        );
    }

    #[test]
    fn config_check_rejects_limits_above_the_pid_api_cap() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let invalid_limit = MAX_API_LIMIT + 1;
        fs::write(
            &path,
            format!(
                r#"
                [fetch]
                api_limit = {invalid_limit}

                [[boarding_points]]
                name = "Test stop"
                stop_ids = ["U100Z1P"]
                walking_minutes = 1

                [[boarding_points.routes]]
                line = "123"
                headsign = "Test destination"
                "#
            ),
        )
        .unwrap();
        let cli = Cli::try_parse_from([
            OsString::from("pidjezdy"),
            OsString::from("--config"),
            path.into_os_string(),
            OsString::from("config"),
            OsString::from("check"),
        ])
        .unwrap();
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();

        let failure = run(cli, None, None, false, &mut output, &mut diagnostics).unwrap_err();
        let causes = std::iter::successors(
            Some(&failure as &(dyn std::error::Error + 'static)),
            |error| error.source(),
        )
        .map(ToString::to_string)
        .collect::<Vec<_>>();

        assert!(output.is_empty());
        assert!(diagnostics.is_empty());
        let expected =
            format!("PID API limit must be between 1 and {MAX_API_LIMIT}, got {invalid_limit}");
        assert!(causes.iter().any(|cause| cause == &expected), "{causes:?}");
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
    fn json_departure_failures_emit_a_versioned_error_document() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "not = [valid").unwrap();
        let cli = Cli::try_parse_from([
            OsString::from("pidjezdy"),
            OsString::from("--config"),
            path.into_os_string(),
            OsString::from("departures"),
            OsString::from("--format"),
            OsString::from("json"),
        ])
        .unwrap();
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();

        let failure = run(cli, None, None, false, &mut output, &mut diagnostics).unwrap_err();

        assert!(failure.json_reported());
        assert!(diagnostics.is_empty());
        let document: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(document["schema_version"], output::JSON_SCHEMA_VERSION);
        assert_eq!(document["error"]["kind"], "config_invalid");
        assert!(
            document["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with("invalid configuration ")
        );
        assert_eq!(document["error"]["causes"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn text_departure_failures_do_not_write_an_error_document() {
        let cli = Cli::try_parse_from([
            "pidjezdy",
            "--config",
            "missing.toml",
            "departures",
            "--format",
            "text",
        ])
        .unwrap();
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();

        let failure = run(cli, None, None, false, &mut output, &mut diagnostics).unwrap_err();

        assert!(!failure.json_reported());
        assert!(output.is_empty());
        assert!(diagnostics.is_empty());
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

        run(cli, None, None, false, &mut output, &mut diagnostics).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("pidjezdy"));
        assert!(output.contains("departures"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn output_errors_preserve_the_io_error_source() {
        let error = AppError::from(OutputError::Write(std::io::Error::other("closed output")));
        let chain = std::iter::successors(
            Some(&error as &(dyn std::error::Error + 'static)),
            |error| error.source(),
        )
        .map(ToString::to_string)
        .collect::<Vec<_>>();

        assert_eq!(chain, ["could not write command output", "closed output"]);
    }

    #[test]
    fn endpoint_environment_uses_trimmed_nonempty_overrides() {
        assert_eq!(resolve_endpoint(None), DEFAULT_ENDPOINT);
        assert_eq!(resolve_endpoint(Some("")), DEFAULT_ENDPOINT);
        assert_eq!(resolve_endpoint(Some(" \t\n")), DEFAULT_ENDPOINT);
        assert_eq!(
            resolve_endpoint(Some("  http://127.0.0.1:1234/departures\t")),
            "http://127.0.0.1:1234/departures"
        );
    }

    #[test]
    fn only_stdout_write_failures_prevent_json_error_reporting() {
        let serialization = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let serialization = AppError::from(OutputError::Json(serialization));
        let write = AppError::from(OutputError::Write(std::io::Error::other("closed output")));

        assert!(serialization.can_report_as_json());
        assert!(!write.can_report_as_json());
    }
}
