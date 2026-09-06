use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::{self, JoinHandle};

use chrono::{TimeDelta, Utc};
use serde_json::json;

const DEPARTURES: &[u8] = include_bytes!("../../pidjezdy-pid/fixtures/departures.json");

struct FixtureServer {
    endpoint: String,
    worker: JoinHandle<()>,
}

impl FixtureServer {
    fn finish(self) {
        self.worker.join().unwrap();
    }
}

fn serve_once(body: Vec<u8>) -> FixtureServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 8192];
        let length = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..length]);
        assert!(request.starts_with("GET /departures?"), "{request}");
        assert!(request.contains("stopIds%5B%5D="), "{request}");

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

fn write_config(root: &Path) -> PathBuf {
    let path = root.join("config.toml");
    fs::write(
        &path,
        r#"
            [display]
            max_departures = 3

            [fetch]
            minutes_after = 120
            api_limit = 20

            [[boarding_points]]
            name = "Nearby stop"
            stop_ids = ["U100Z1P"]
            walking_minutes = 4
            safety_buffer_minutes = 2

            [[boarding_points.routes]]
            line = "158"
            headsign = "Centrum"
        "#,
    )
    .unwrap();
    path
}

fn command(root: &Path, config: &Path, endpoint: &str, arguments: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pidjezdy"));
    command
        .env_clear()
        .env("HOME", root)
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("PIDJEZDY_CONFIG", config)
        .env("PIDJEZDY_ENDPOINT", endpoint)
        .args(arguments);
    command.output().unwrap()
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
    assert_eq!(document["schema_version"], 1);
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
    assert!(lines[0].contains("leave in "), "{text}");
    assert!(lines[1].contains("Nearby stop · platform A"), "{text}");
    assert!(lines[1].contains("departs in "), "{text}");
}
