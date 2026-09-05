use std::fs;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use pidjezdy_core::config::Config;
use pidjezdy_core::departure::Departure;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;

const CACHE_FILE: &str = "departures.json";
const CACHE_VERSION: u8 = 1;
const MAX_CACHE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheFile {
    version: u8,
    fetched_at: DateTime<Utc>,
    config: Config,
    departures: Vec<Departure>,
}

#[derive(Debug)]
pub(crate) struct CacheSnapshot {
    pub(crate) fetched_at: DateTime<Utc>,
    pub(crate) departures: Vec<Departure>,
}

#[derive(Debug, Error)]
pub(crate) enum CacheWriteError {
    #[error("could not determine the platform cache directory")]
    DirectoryUnavailable,
    #[error("could not create cache directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not create a temporary cache file in {path}: {source}")]
    CreateTemporary {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not serialize departure cache: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("could not write departure cache: {0}")]
    Write(#[source] std::io::Error),
    #[error("could not flush departure cache: {0}")]
    Flush(#[source] std::io::Error),
    #[error("could not sync departure cache: {0}")]
    Sync(#[source] std::io::Error),
    #[error("could not inspect temporary departure cache: {0}")]
    Inspect(#[source] std::io::Error),
    #[error("could not atomically replace cache {path}: {source}")]
    Persist {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("departure cache exceeded the {limit}-byte size limit")]
    TooLarge { limit: u64 },
}

#[derive(Debug, Error)]
pub(crate) enum CacheReadError {
    #[error("could not determine the platform cache directory")]
    DirectoryUnavailable,
    #[error("no cached departures available yet at {path}")]
    NotFound { path: PathBuf },
    #[error("could not read departure cache {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not deserialize departure cache: {0}")]
    Deserialize(#[source] serde_json::Error),
    #[error("departure cache uses unsupported version {0}")]
    UnsupportedVersion(u8),
    #[error("departure cache belongs to a different configuration")]
    ConfigMismatch,
    #[error("departure cache exceeded the {limit}-byte size limit")]
    TooLarge { limit: u64 },
}

pub(crate) fn write_snapshot(
    config: &Config,
    fetched_at: DateTime<Utc>,
    departures: &[Departure],
) -> Result<(), CacheWriteError> {
    let path = default_cache_path().ok_or(CacheWriteError::DirectoryUnavailable)?;
    write_snapshot_to(&path, config, fetched_at, departures)
}

pub(crate) fn read_snapshot(config: &Config) -> Result<CacheSnapshot, CacheReadError> {
    let path = default_cache_path().ok_or(CacheReadError::DirectoryUnavailable)?;
    read_snapshot_from(&path, config)
}

fn default_cache_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "pidjezdy").map(|dirs| dirs.cache_dir().join(CACHE_FILE))
}

fn write_snapshot_to(
    path: &Path,
    config: &Config,
    fetched_at: DateTime<Utc>,
    departures: &[Departure],
) -> Result<(), CacheWriteError> {
    write_snapshot_to_with_limit(path, config, fetched_at, departures, MAX_CACHE_BYTES)
}

fn write_snapshot_to_with_limit(
    path: &Path,
    config: &Config,
    fetched_at: DateTime<Utc>,
    departures: &[Departure],
    max_bytes: u64,
) -> Result<(), CacheWriteError> {
    let directory = cache_directory(path);
    fs::create_dir_all(directory).map_err(|source| CacheWriteError::CreateDirectory {
        path: directory.to_owned(),
        source,
    })?;
    let mut temporary =
        NamedTempFile::new_in(directory).map_err(|source| CacheWriteError::CreateTemporary {
            path: directory.to_owned(),
            source,
        })?;
    serde_json::to_writer(
        &mut temporary,
        &CacheFile {
            version: CACHE_VERSION,
            fetched_at,
            config: config.clone(),
            departures: departures.to_vec(),
        },
    )
    .map_err(CacheWriteError::Serialize)?;
    temporary.write_all(b"\n").map_err(CacheWriteError::Write)?;
    temporary.flush().map_err(CacheWriteError::Flush)?;
    if temporary
        .as_file()
        .metadata()
        .map_err(CacheWriteError::Inspect)?
        .len()
        > max_bytes
    {
        return Err(CacheWriteError::TooLarge { limit: max_bytes });
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(CacheWriteError::Sync)?;
    temporary
        .persist(path)
        .map_err(|error| CacheWriteError::Persist {
            path: path.to_owned(),
            source: error.error,
        })?;
    Ok(())
}

fn cache_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn read_snapshot_from(path: &Path, config: &Config) -> Result<CacheSnapshot, CacheReadError> {
    let file = fs::File::open(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            CacheReadError::NotFound {
                path: path.to_owned(),
            }
        } else {
            CacheReadError::Read {
                path: path.to_owned(),
                source,
            }
        }
    })?;
    let input = read_at_most(file, MAX_CACHE_BYTES)
        .map_err(|source| CacheReadError::Read {
            path: path.to_owned(),
            source,
        })?
        .ok_or(CacheReadError::TooLarge {
            limit: MAX_CACHE_BYTES,
        })?;
    let cached: CacheFile = serde_json::from_slice(&input).map_err(CacheReadError::Deserialize)?;
    if cached.version != CACHE_VERSION {
        return Err(CacheReadError::UnsupportedVersion(cached.version));
    }
    if cached.config != *config {
        return Err(CacheReadError::ConfigMismatch);
    }
    Ok(CacheSnapshot {
        fetched_at: cached.fetched_at,
        departures: cached.departures,
    })
}

// Kept identical to the PID client helper so the domain core remains I/O-free.
fn read_at_most(reader: impl Read, limit: u64) -> Result<Option<Vec<u8>>, std::io::Error> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok((u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= limit).then_some(bytes))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use chrono::TimeDelta;
    use pidjezdy_core::departure::Vehicle;

    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-04T12:00:00+02:00")
            .unwrap()
            .to_utc()
    }

    fn config() -> Config {
        Config::from_toml(
            r#"
                [[boarding_points]]
                name = "Nearby stop"
                stop_ids = ["U100Z1P"]
                walking_minutes = 4

                [[boarding_points.routes]]
                line = "158"
                headsign = "Centre"
            "#,
        )
        .unwrap()
    }

    fn departure(trip_id: &str) -> Departure {
        Departure {
            trip_id: trip_id.into(),
            line: "158".into(),
            headsign: "Centre".into(),
            stop_id: "U100Z1P".into(),
            platform_code: Some("A".into()),
            scheduled_at: now() + TimeDelta::minutes(10),
            predicted_at: None,
            delay_seconds: None,
            is_cancelled: false,
            vehicle: Vehicle::default(),
        }
    }

    #[test]
    fn atomically_replaces_a_complete_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/departures.json");

        write_snapshot_to(&path, &config(), now(), &[departure("old")]).unwrap();
        write_snapshot_to(
            &path,
            &config(),
            now() + TimeDelta::minutes(1),
            &[departure("new")],
        )
        .unwrap();

        let cached: CacheFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(cached.version, CACHE_VERSION);
        assert_eq!(cached.fetched_at, now() + TimeDelta::minutes(1));
        assert_eq!(cached.config, config());
        assert_eq!(cached.departures[0].trip_id, "new");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn reads_only_matching_supported_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("departures.json");
        let configured = config();
        write_snapshot_to(&path, &configured, now(), &[departure("cached")]).unwrap();

        let cached = read_snapshot_from(&path, &configured).unwrap();
        assert_eq!(cached.fetched_at, now());
        assert_eq!(cached.departures[0].trip_id, "cached");

        let mut other_config = configured.clone();
        other_config.boarding_points[0].walking_minutes = 5;
        assert!(matches!(
            read_snapshot_from(&path, &other_config),
            Err(CacheReadError::ConfigMismatch)
        ));

        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document["version"] = 2.into();
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(matches!(
            read_snapshot_from(&path, &configured),
            Err(CacheReadError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn rejects_oversized_cache_files_before_deserialization() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("departures.json");
        fs::write(
            &path,
            vec![b' '; usize::try_from(MAX_CACHE_BYTES).unwrap() + 1],
        )
        .unwrap();

        assert!(matches!(
            read_snapshot_from(&path, &config()),
            Err(CacheReadError::TooLarge {
                limit: MAX_CACHE_BYTES
            })
        ));
    }

    #[test]
    fn reports_a_missing_cache_as_an_expected_empty_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("departures.json");

        let error = read_snapshot_from(&path, &config()).unwrap_err();

        assert!(matches!(
            &error,
            CacheReadError::NotFound { path: missing } if missing == &path
        ));
        assert_eq!(
            error.to_string(),
            format!("no cached departures available yet at {}", path.display())
        );
    }

    #[test]
    fn rejects_snapshots_that_exceed_the_write_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("departures.json");

        let error = write_snapshot_to_with_limit(&path, &config(), now(), &[departure("large")], 1)
            .unwrap_err();

        assert!(matches!(error, CacheWriteError::TooLarge { limit: 1 }));
        assert!(!path.exists());
    }

    #[test]
    fn reports_failure_to_persist_the_temporary_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("existing-directory");
        fs::create_dir(&destination).unwrap();

        let error =
            write_snapshot_to(&destination, &config(), now(), &[departure("cached")]).unwrap_err();

        assert!(matches!(
            error,
            CacheWriteError::Persist { path, .. } if path == destination
        ));
    }

    #[test]
    fn distinguishes_write_and_flush_diagnostics() {
        let write = CacheWriteError::Write(std::io::Error::other("disk full"));
        let flush = CacheWriteError::Flush(std::io::Error::other("device unavailable"));

        assert_eq!(
            write.to_string(),
            "could not write departure cache: disk full"
        );
        assert_eq!(
            flush.to_string(),
            "could not flush departure cache: device unavailable"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn platform_cache_wrappers_resolve_and_round_trip() {
        const CHILD_ROOT: &str = "PIDJEZDY_CACHE_TEST_ROOT";

        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let expected_path = PathBuf::from(root).join("pidjezdy").join(CACHE_FILE);
            assert_eq!(
                default_cache_path().as_deref(),
                Some(expected_path.as_path())
            );

            write_snapshot(&config(), now(), &[departure("platform-cache")]).unwrap();
            let cached = read_snapshot(&config()).unwrap();
            assert_eq!(cached.fetched_at, now());
            assert_eq!(cached.departures[0].trip_id, "platform-cache");
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("cache::tests::platform_cache_wrappers_resolve_and_round_trip")
            .env(CHILD_ROOT, directory.path())
            .env("XDG_CACHE_HOME", directory.path())
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "child test failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn treats_an_empty_relative_parent_as_the_current_directory() {
        assert_eq!(
            cache_directory(Path::new("departures.json")),
            Path::new(".")
        );
        assert_eq!(
            cache_directory(Path::new("cache/departures.json")),
            Path::new("cache")
        );
    }
}
