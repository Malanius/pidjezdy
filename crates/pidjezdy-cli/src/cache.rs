use std::fs;
use std::io::Write as _;
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
    #[error("could not flush departure cache: {0}")]
    Flush(#[source] std::io::Error),
    #[error("could not sync departure cache: {0}")]
    Sync(#[source] std::io::Error),
    #[error("could not atomically replace cache {path}: {source}")]
    Persist {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub(crate) enum CacheReadError {
    #[error("could not determine the platform cache directory")]
    DirectoryUnavailable,
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
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
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
    temporary
        .write_all(b"\n")
        .and_then(|()| temporary.flush())
        .map_err(CacheWriteError::Flush)?;
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

fn read_snapshot_from(path: &Path, config: &Config) -> Result<CacheSnapshot, CacheReadError> {
    let input = fs::read(path).map_err(|source| CacheReadError::Read {
        path: path.to_owned(),
        source,
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

#[cfg(test)]
mod tests {
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
}
