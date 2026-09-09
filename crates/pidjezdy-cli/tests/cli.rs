use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use pidjezdy_pid::MAX_API_LIMIT;
use serde_json::json;

const DEPARTURES: &[u8] = include_bytes!("../../pidjezdy-pid/fixtures/departures.json");
const ERROR: &[u8] = include_bytes!("../../pidjezdy-pid/fixtures/error.json");
const SERVER_TIMEOUT: Duration = Duration::from_secs(5);

struct FixtureServer {
    endpoint: String,
    worker: JoinHandle<()>,
}

impl FixtureServer {
    fn finish(self) {
        if let Err(payload) = self.worker.join() {
            resume_unwind(payload);
        }
    }
}

fn serve_once(body: Vec<u8>) -> FixtureServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let mut stream = accept_before_timeout(&listener);
        stream.set_read_timeout(Some(SERVER_TIMEOUT)).unwrap();
        stream.set_write_timeout(Some(SERVER_TIMEOUT)).unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let mut chunk = [0_u8; 1024];
            let length = stream.read(&mut chunk).unwrap();
            assert!(length > 0, "client closed before completing HTTP headers");
            request.extend_from_slice(&chunk[..length]);
            assert!(request.len() <= 8192, "request headers exceeded test limit");
        }
        let request = String::from_utf8_lossy(&request);
        assert!(request.starts_with("GET /departures?"), "{request}");
        assert!(request.contains("stopIds%5B%5D="), "{request}");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("user-agent: pidjezdy/"),
            "{request}"
        );

        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    });
    FixtureServer {
        endpoint: format!("http://{address}/departures"),
        worker,
    }
}

fn accept_before_timeout(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + SERVER_TIMEOUT;

    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                panic!("CLI did not connect to fixture server within {SERVER_TIMEOUT:?}");
            }
            Err(error) => panic!("fixture server failed to accept connection: {error}"),
        }
    }
}

fn current_departures() -> Vec<u8> {
    let mut document: serde_json::Value = serde_json::from_slice(DEPARTURES).unwrap();
    let now = Utc::now();
    document[0][0]["departure"]["timestamp_scheduled"] =
        json!((now + TimeDelta::minutes(10)).to_rfc3339());
    document[0][0]["departure"]["timestamp_predicted"] =
        json!((now + TimeDelta::minutes(12)).to_rfc3339());
    document[1][0]["departure"]["timestamp_scheduled"] =
        json!((now + TimeDelta::minutes(20)).to_rfc3339());
    serde_json::to_vec(&document).unwrap()
}

fn current_cancelled_departures() -> Vec<u8> {
    let mut document: serde_json::Value = serde_json::from_slice(&current_departures()).unwrap();
    document[0][0]["trip"]["is_canceled"] = json!(true);
    serde_json::to_vec(&document).unwrap()
}

fn capped_departures() -> Vec<u8> {
    let document: serde_json::Value = serde_json::from_slice(DEPARTURES).unwrap();
    let template = document[0][0].clone();
    let now = Utc::now();
    let mut group = Vec::new();
    for (index, line, minutes) in [(0, "158", 10), (1, "900", 20), (2, "901", 28)] {
        let mut departure = template.clone();
        departure["departure"]["timestamp_scheduled"] =
            json!((now + TimeDelta::minutes(minutes)).to_rfc3339());
        departure["departure"]["timestamp_predicted"] = serde_json::Value::Null;
        departure["departure"]["delay_seconds"] = serde_json::Value::Null;
        departure["route"]["short_name"] = json!(line);
        departure["trip"]["id"] = json!(format!("trip-{index}"));
        departure["trip"]["headsign"] = json!(if line == "158" {
            "Centrum"
        } else {
            "Elsewhere"
        });
        group.push(departure);
    }
    serde_json::to_vec(&json!([group])).unwrap()
}

fn write_config(root: &Path) -> PathBuf {
    write_config_with_walking_time(root, 4)
}

fn write_config_with_walking_time(root: &Path, walking_minutes: u32) -> PathBuf {
    write_config_with(root, walking_minutes, MAX_API_LIMIT)
}

fn write_config_with_api_limit(root: &Path, api_limit: usize) -> PathBuf {
    write_config_with(root, 4, api_limit)
}

fn write_config_with(root: &Path, walking_minutes: u32, api_limit: usize) -> PathBuf {
    let path = root.join("config.toml");
    fs::write(
        &path,
        format!(
            r#"
            [display]
            max_departures = 3

            [fetch]
            minutes_after = 120
            api_limit = {api_limit}

            [[boarding_points]]
            name = "Nearby stop"
            stop_ids = ["U100Z1P"]
            walking_minutes = {walking_minutes}
            safety_buffer_minutes = 2

            [[boarding_points.routes]]
            line = "158"
            headsign = "Centrum"
        "#
        ),
    )
    .unwrap();
    path
}

fn command(root: &Path, config: &Path, endpoint: &str, arguments: &[&str]) -> Output {
    let mut command = isolated_command(root);
    command
        .env("PIDJEZDY_CONFIG", config)
        .env("PIDJEZDY_ENDPOINT", endpoint)
        .args(arguments);
    command.output().unwrap()
}

/// Variables the operating system needs in the child process itself. Clearing
/// them breaks socket setup on Windows, where Winsock resolves system
/// libraries through `SystemRoot`, so the CLI cannot reach a fixture server.
#[cfg(windows)]
const PLATFORM_ENVIRONMENT: &[&str] = &[
    "COMSPEC",
    "PATH",
    "PATHEXT",
    "ProgramData",
    "SystemDrive",
    "SystemRoot",
    "TEMP",
    "TMP",
    "windir",
];
#[cfg(not(windows))]
const PLATFORM_ENVIRONMENT: &[&str] = &[];

/// The isolated cache file, pinned with `PIDJEZDY_CACHE`. On Windows the
/// platform directory ignores `HOME` and `XDG_CACHE_HOME` entirely; on macOS
/// `HOME` places it, but under `Library/Caches` rather than the XDG layout.
/// Pinning the file keeps one expected path on every platform.
fn cache_path(root: &Path) -> PathBuf {
    root.join("cache").join("pidjezdy").join("departures.json")
}

fn isolated_command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pidjezdy"));
    command.env_clear();
    for name in PLATFORM_ENVIRONMENT {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("HOME", root)
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("PIDJEZDY_CACHE", cache_path(root));
    command
}

fn disconnect_once() -> FixtureServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || drop(accept_before_timeout(&listener)));
    FixtureServer {
        endpoint: format!("http://{address}/departures"),
        worker,
    }
}

#[test]
fn fixture_server_finish_preserves_worker_panic() {
    let server = FixtureServer {
        endpoint: String::new(),
        worker: thread::spawn(|| panic!("fixture worker failed")),
    };

    let payload = std::panic::catch_unwind(AssertUnwindSafe(|| server.finish())).unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"fixture worker failed")
    );
}

#[test]
fn json_happy_path_exercises_the_complete_binary() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(directory.path());
    let server = serve_once(current_departures());

    let output = command(
        directory.path(),
        &config,
        &server.endpoint,
        &["departures", "--format", "json"],
    );
    server.finish();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["schema_version"], 2);
    assert_eq!(document["stale"], false);
    assert_eq!(document["generated_at"], document["data_updated_at"]);
    assert_eq!(document["departures"][0]["departure"]["line"], "158");
    assert_eq!(
        document["departures"][0]["departure"]["headsign"],
        "Centrum"
    );
    assert_eq!(
        document["departures"][0]["boarding_point_name"],
        "Nearby stop"
    );
    assert_eq!(document["cancelled"], json!([]));
}

#[test]
fn json_all_cancelled_result_is_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(directory.path());
    let server = serve_once(current_cancelled_departures());

    let output = command(
        directory.path(),
        &config,
        &server.endpoint,
        &["departures", "--format", "json"],
    );
    server.finish();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["departures"], json!([]));
    assert_eq!(document["cancelled"][0]["departure"]["line"], "158");
    assert_eq!(document["cancelled"][0]["departure"]["is_cancelled"], true);
}

#[test]
fn text_happy_path_is_aligned_and_unstyled_when_piped() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(directory.path());
    let server = serve_once(current_departures());

    let output = command(
        directory.path(),
        &config,
        &server.endpoint,
        &["departures", "--format", "text"],
    );
    server.finish();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains('\u{1b}'));
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("158  Centrum"), "{text}");
    assert!(lines[0].contains("leave by "), "{text}");
    assert!(lines[0].contains(" · in "), "{text}");
    assert!(lines[1].contains("Nearby stop · platform A"), "{text}");
    assert!(lines[1].contains("+2 late"), "{text}");
    assert!(lines[1].contains("departs "), "{text}");
    assert!(lines[1].contains(" · in "), "{text}");
}

#[test]
fn short_capped_response_warns_on_stderr() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config_with_api_limit(directory.path(), 3);
    let server = serve_once(capped_departures());

    let output = command(
        directory.path(),
        &config,
        &server.endpoint,
        &["departures", "--format", "json"],
    );
    server.finish();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["departures"].as_array().unwrap().len(), 1);
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    assert!(
        diagnostics.starts_with(
            "pidjezdy: warning: \"Nearby stop\" hit the 3-departure API limit, covering only the next "
        ),
        "{diagnostics}"
    );
    assert!(
        diagnostics.ends_with(
            " of 120 requested minutes; matching departures beyond that are not visible\n"
        ),
        "{diagnostics}"
    );
}

#[test]
fn structured_api_error_is_a_json_failure_document() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(directory.path());
    let server = serve_once(ERROR.to_vec());

    let output = command(
        directory.path(),
        &config,
        &server.endpoint,
        &["departures", "--format", "json"],
    );
    server.finish();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["schema_version"], 2);
    assert_eq!(document["error"]["kind"], "departures_unavailable");
    let causes = document["error"]["causes"].as_array().unwrap();
    assert!(causes.iter().any(|cause| {
        cause
            .as_str()
            .is_some_and(|cause| cause.contains("PID API returned status 400: Bad request"))
    }));
}

#[test]
fn network_failure_reuses_and_reselects_a_compatible_cache() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(directory.path());
    let server = serve_once(current_departures());
    let fresh = command(
        directory.path(),
        &config,
        &server.endpoint,
        &["departures", "--format", "json"],
    );
    server.finish();
    assert!(fresh.status.success());
    let fresh: serde_json::Value = serde_json::from_slice(&fresh.stdout).unwrap();

    let config = write_config_with_walking_time(directory.path(), 5);
    let disconnect = disconnect_once();
    let stale = command(
        directory.path(),
        &config,
        &disconnect.endpoint,
        &["departures", "--format", "json"],
    );
    disconnect.finish();

    assert!(
        stale.status.success(),
        "{}",
        String::from_utf8_lossy(&stale.stderr)
    );
    let stale: serde_json::Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(stale["stale"], true);
    assert_eq!(stale["departures"][0]["departure"]["line"], "158");
    assert!(
        stale["departures"][0]["leave_in_seconds"].as_i64().unwrap()
            < fresh["departures"][0]["leave_in_seconds"].as_i64().unwrap() - 50
    );

    let disconnect = disconnect_once();
    let stale_text = command(
        directory.path(),
        &config,
        &disconnect.endpoint,
        &["departures", "--format", "text"],
    );
    disconnect.finish();
    assert!(stale_text.status.success());
    assert!(
        String::from_utf8(stale_text.stdout)
            .unwrap()
            .starts_with("STALE · last updated ")
    );
}

#[test]
fn empty_success_does_not_destroy_the_network_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(directory.path());
    let initial_server = serve_once(current_departures());
    let initial = command(
        directory.path(),
        &config,
        &initial_server.endpoint,
        &["departures", "--format", "json"],
    );
    initial_server.finish();
    assert!(initial.status.success());

    let empty_server = serve_once(b"[]".to_vec());
    let empty = command(
        directory.path(),
        &config,
        &empty_server.endpoint,
        &["departures", "--format", "json"],
    );
    empty_server.finish();
    assert!(empty.status.success());
    let empty: serde_json::Value = serde_json::from_slice(&empty.stdout).unwrap();
    assert_eq!(empty["stale"], false);
    assert!(empty["departures"].as_array().unwrap().is_empty());

    let disconnect = disconnect_once();
    let fallback = command(
        directory.path(),
        &config,
        &disconnect.endpoint,
        &["departures", "--format", "json"],
    );
    disconnect.finish();
    assert!(
        fallback.status.success(),
        "{}",
        String::from_utf8_lossy(&fallback.stderr)
    );
    let fallback: serde_json::Value = serde_json::from_slice(&fallback.stdout).unwrap();
    assert_eq!(fallback["stale"], true);
    assert_eq!(fallback["departures"][0]["departure"]["line"], "158");
}

#[test]
fn network_and_cache_failure_reports_both_causes() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(directory.path());

    let disconnect = disconnect_once();
    let output = command(
        directory.path(),
        &config,
        &disconnect.endpoint,
        &["departures", "--format", "json"],
    );
    disconnect.finish();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["error"]["kind"], "departures_unavailable");
    let causes = document["error"]["causes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(causes.contains("PID request failed"), "{causes}");
    assert!(causes.contains("cached fallback unavailable"), "{causes}");
}

#[test]
fn invalid_configuration_lists_validation_errors() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("invalid.toml");
    fs::write(
        &config,
        "[display]\nmax_departures = 0\n[fetch]\nminutes_after = 0\n",
    )
    .unwrap();

    let output = command(
        directory.path(),
        &config,
        "http://127.0.0.1:1/departures",
        &["departures"],
    );

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    assert!(
        diagnostics.contains("invalid configuration"),
        "{diagnostics}"
    );
    assert!(diagnostics.contains("display.max_departures must be between 1 and 20"));
    assert!(diagnostics.contains("fetch.minutes_after must be greater than 0"));
    assert!(diagnostics.contains("at least one boarding point is required"));
}

#[test]
fn config_check_rejects_a_limit_above_the_pid_api_cap() {
    let directory = tempfile::tempdir().unwrap();
    let invalid_limit = MAX_API_LIMIT + 1;
    let config = write_config_with_api_limit(directory.path(), invalid_limit);
    let output = isolated_command(directory.path())
        .arg("--config")
        .arg(config)
        .args(["config", "check"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    assert!(
        diagnostics.contains("could not build PID departure request"),
        "{diagnostics}"
    );
    assert!(
        diagnostics.contains(&format!(
            "PID API limit must be between 1 and {MAX_API_LIMIT}, got {invalid_limit}"
        )),
        "{diagnostics}"
    );
}

#[test]
fn cache_path_does_not_require_valid_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("invalid.toml");
    fs::write(&config, "not = [valid").unwrap();
    let output = isolated_command(directory.path())
        .arg("--config")
        .arg(config)
        .args(["cache", "path"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n", cache_path(directory.path()).display())
    );
}

#[test]
fn cache_clear_removes_a_snapshot_and_accepts_an_absent_cache() {
    let directory = tempfile::tempdir().unwrap();
    let cache = cache_path(directory.path());
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, "cached departures").unwrap();

    let removed = isolated_command(directory.path())
        .args(["cache", "clear"])
        .output()
        .unwrap();
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(removed.stderr.is_empty());
    assert_eq!(
        String::from_utf8(removed.stdout).unwrap(),
        format!("removed {}\n", cache.display())
    );
    assert!(!cache.exists());

    let absent = isolated_command(directory.path())
        .args(["cache", "clear"])
        .output()
        .unwrap();
    assert!(
        absent.status.success(),
        "{}",
        String::from_utf8_lossy(&absent.stderr)
    );
    assert!(absent.stderr.is_empty());
    assert_eq!(
        String::from_utf8(absent.stdout).unwrap(),
        format!("no cache file at {}\n", cache.display())
    );
}

#[test]
fn explicit_config_beats_environment_config() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(directory.path());
    let server = serve_once(current_departures());
    let missing = directory.path().join("missing.toml");
    let mut command = isolated_command(directory.path());
    let output = command
        .env("PIDJEZDY_CONFIG", missing)
        .env("PIDJEZDY_ENDPOINT", &server.endpoint)
        .arg("--config")
        .arg(&config)
        .args(["departures", "--format", "json"])
        .output()
        .unwrap();
    server.finish();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn environment_config_beats_platform_default() {
    let directory = tempfile::tempdir().unwrap();
    let config = write_config(directory.path());
    let default = directory.path().join("config/pidjezdy/config.toml");
    fs::create_dir_all(default.parent().unwrap()).unwrap();
    fs::write(&default, "invalid = [toml").unwrap();
    let server = serve_once(current_departures());

    let output = command(
        directory.path(),
        &config,
        &server.endpoint,
        &["departures", "--format", "json"],
    );
    server.finish();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
