use chrono::{DateTime, Utc};
use pidjezdy_core::config::Config;
use pidjezdy_core::departure::Departure;
use pidjezdy_core::selection::{SelectedDeparture, SelectionOptions, select_departures};
use pidjezdy_pid::{DepartureBoardRequest, PidClient, PidClientError, PidRequestError};
use thiserror::Error;

use crate::cache::{CacheReadError, CacheSnapshot, CacheWriteError, read_snapshot, write_snapshot};

#[derive(Debug)]
pub(crate) struct DepartureQuery {
    pub(crate) generated_at: DateTime<Utc>,
    pub(crate) data_updated_at: DateTime<Utc>,
    pub(crate) stale: bool,
    pub(crate) departures: Vec<SelectedDeparture>,
    pub(crate) cache_warning: Option<CacheWriteError>,
}

struct QuerySources<WriteCache, ReadCache> {
    live: Result<Vec<Departure>, LiveDepartureError>,
    write_cache: WriteCache,
    read_cache: ReadCache,
}

#[derive(Debug, Error)]
pub(super) enum LiveDepartureError {
    #[error("could not create PID client")]
    CreateClient(#[source] PidClientError),
    #[error("could not fetch PID departures")]
    Fetch(#[source] PidClientError),
}

#[derive(Debug, Error)]
pub enum DepartureQueryError {
    #[error("could not build PID departure request")]
    Request(#[from] PidRequestError),
    #[error("departures unavailable")]
    Unavailable(#[source] DepartureUnavailable),
}

#[derive(Debug, Error)]
#[error("cached fallback unavailable: {cache}")]
pub struct DepartureUnavailable {
    #[source]
    live: LiveDepartureError,
    cache: CacheReadError,
}

/// Fetch, filter, rank, and limit departures using validated configuration.
///
/// # Errors
///
/// Returns an error if a PID request cannot be constructed, or when both the
/// live query and its compatible cached fallback are unavailable.
pub(crate) fn query_departures(
    config: &Config,
    limit: usize,
    endpoint: &str,
) -> Result<DepartureQuery, DepartureQueryError> {
    let request = configured_request(config)?;
    let live = PidClient::with_endpoint(endpoint)
        .map_err(LiveDepartureError::CreateClient)
        .and_then(|client| client.fetch(&request).map_err(LiveDepartureError::Fetch));
    let generated_at = Utc::now();
    finish_query_with(
        config,
        limit,
        generated_at,
        QuerySources {
            live,
            write_cache: |departures: &[Departure]| {
                write_snapshot(&request, generated_at, departures)
            },
            read_cache: || read_snapshot(&request),
        },
    )
}

fn finish_query_with<WriteCache, ReadCache>(
    config: &Config,
    limit: usize,
    generated_at: DateTime<Utc>,
    sources: QuerySources<WriteCache, ReadCache>,
) -> Result<DepartureQuery, DepartureQueryError>
where
    WriteCache: FnOnce(&[Departure]) -> Result<(), CacheWriteError>,
    ReadCache: FnOnce() -> Result<CacheSnapshot, CacheReadError>,
{
    let (departures, data_updated_at, stale, cache_warning) = match sources.live {
        Ok(departures) => {
            let warning = if departures.is_empty() {
                None
            } else {
                (sources.write_cache)(&departures).err()
            };
            (departures, generated_at, false, warning)
        }
        Err(live) => {
            let cached = (sources.read_cache)().map_err(|cache| {
                DepartureQueryError::Unavailable(DepartureUnavailable { live, cache })
            })?;
            (cached.departures, cached.fetched_at, true, None)
        }
    };
    let departures = select_departures(
        config,
        &departures,
        generated_at,
        SelectionOptions {
            limit,
            include_unreachable: false,
        },
    );
    Ok(DepartureQuery {
        generated_at,
        data_updated_at,
        stale,
        departures,
        cache_warning,
    })
}

fn configured_request(config: &Config) -> Result<DepartureBoardRequest, PidRequestError> {
    let stop_groups = config
        .boarding_points
        .iter()
        .map(|point| point.stop_ids.clone())
        .collect();
    DepartureBoardRequest::new(
        config.fetch.minutes_after,
        config.fetch.api_limit,
        stop_groups,
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::TimeDelta;
    use pidjezdy_core::departure::{Departure, Vehicle};
    use url::Url;

    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-04T12:00:00+02:00")
            .unwrap()
            .to_utc()
    }

    fn fallback_config() -> Config {
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

    fn departure() -> Departure {
        Departure {
            trip_id: "trip-1".into(),
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

    fn live_error() -> LiveDepartureError {
        let error = PidClient::with_endpoint("not a URL").err().unwrap();
        LiveDepartureError::CreateClient(error)
    }

    #[test]
    fn builds_one_independently_limited_group_per_boarding_point() {
        let config = Config::from_toml(
            r#"
                [fetch]
                minutes_after = 90
                api_limit = 7

                [[boarding_points]]
                name = "First"
                stop_ids = [" U100Z1P ", "U100Z2P"]
                walking_minutes = 4

                [[boarding_points.routes]]
                line = "1"
                headsign = "Centre"

                [[boarding_points]]
                name = "Second"
                stop_ids = ["U200Z1P"]
                walking_minutes = 6

                [[boarding_points.routes]]
                line = "2"
                headsign = "Station"
            "#,
        )
        .unwrap();
        let endpoint = Url::parse("https://example.test/data.php").unwrap();
        let url = configured_request(&config).unwrap().url(&endpoint);
        let pairs = url.query_pairs().collect::<Vec<_>>();

        assert_eq!(pairs[0], ("minutesAfter".into(), "90".into()));
        assert_eq!(pairs[1], ("limit".into(), "7".into()));
        assert_eq!(
            pairs[2],
            ("stopIds[]".into(), r#"{"0":["U100Z1P","U100Z2P"]}"#.into())
        );
        assert_eq!(
            pairs[3],
            ("stopIds[]".into(), r#"{"0":["U200Z1P"]}"#.into())
        );
    }

    #[test]
    fn presentation_changes_do_not_change_the_cache_request() {
        let original = fallback_config();
        let mut changed = original.clone();
        changed.display.max_departures = 7;
        changed.boarding_points[0].name = "Renamed stop".into();
        changed.boarding_points[0].walking_minutes = 9;
        changed.boarding_points[0].safety_buffer_minutes = 4;
        changed.boarding_points[0].routes[0].headsign = "Different direction".into();

        assert_eq!(
            configured_request(&original).unwrap(),
            configured_request(&changed).unwrap()
        );
    }

    #[test]
    fn reselects_cached_departures_at_the_current_time_and_marks_them_stale() {
        let configured = fallback_config();
        let cached_at = now() - TimeDelta::minutes(3);
        let result = finish_query_with(
            &configured,
            3,
            now(),
            QuerySources {
                live: Err(live_error()),
                write_cache: |_: &[Departure]| {
                    unreachable!("a failed live request must not update the cache")
                },
                read_cache: || {
                    Ok(CacheSnapshot {
                        fetched_at: cached_at,
                        departures: vec![departure()],
                    })
                },
            },
        )
        .unwrap();

        assert!(result.stale);
        assert_eq!(result.data_updated_at, cached_at);
        assert_eq!(result.departures.len(), 1);
        assert_eq!(result.departures[0].leave_in_seconds, 4 * 60);
        assert!(result.cache_warning.is_none());
    }

    #[test]
    fn cache_write_failure_does_not_hide_fresh_departures() {
        let configured = fallback_config();
        let result = finish_query_with(
            &configured,
            3,
            now(),
            QuerySources {
                live: Ok(vec![departure()]),
                write_cache: |_: &[Departure]| Err(CacheWriteError::DirectoryUnavailable),
                read_cache: || unreachable!("a successful live request must not read the cache"),
            },
        )
        .unwrap();

        assert!(!result.stale);
        assert_eq!(result.data_updated_at, now());
        assert!(matches!(
            result.cache_warning,
            Some(CacheWriteError::DirectoryUnavailable)
        ));
    }

    #[test]
    fn empty_live_result_does_not_overwrite_the_cache() {
        let result = finish_query_with(
            &fallback_config(),
            3,
            now(),
            QuerySources {
                live: Ok(Vec::new()),
                write_cache: |_: &[Departure]| {
                    unreachable!("an empty live result must not overwrite the cache")
                },
                read_cache: || unreachable!("a successful live request must not read the cache"),
            },
        )
        .unwrap();

        assert!(!result.stale);
        assert_eq!(result.data_updated_at, now());
        assert!(result.departures.is_empty());
        assert!(result.cache_warning.is_none());
    }

    #[test]
    fn reports_both_live_and_cache_failures() {
        let error = finish_query_with(
            &fallback_config(),
            3,
            now(),
            QuerySources {
                live: Err(live_error()),
                write_cache: |_: &[Departure]| {
                    unreachable!("a failed live request must not update the cache")
                },
                read_cache: || Err(CacheReadError::RequestMismatch),
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            DepartureQueryError::Unavailable(DepartureUnavailable {
                cache: CacheReadError::RequestMismatch,
                ..
            })
        ));
    }

    #[test]
    fn describes_a_missing_fallback_without_a_filesystem_error() {
        let missing = PathBuf::from("/cache/departures.json");
        let error = finish_query_with(
            &fallback_config(),
            3,
            now(),
            QuerySources {
                live: Err(live_error()),
                write_cache: |_: &[Departure]| {
                    unreachable!("a failed live request must not update the cache")
                },
                read_cache: || {
                    Err(CacheReadError::NotFound {
                        path: missing.clone(),
                    })
                },
            },
        )
        .unwrap_err();

        let chain = std::iter::successors(
            Some(&error as &(dyn std::error::Error + 'static)),
            |error| error.source(),
        )
        .map(ToString::to_string)
        .collect::<Vec<_>>();

        assert_eq!(chain[0], "departures unavailable");
        assert!(
            chain[1].contains("cached fallback unavailable: no cached departures available yet")
        );
        assert_eq!(chain[2], "could not create PID client");
        assert_eq!(chain[3], "invalid PID endpoint");
        assert!(chain.len() > 4);
        assert!(!chain.join("\n").contains("os error"));
    }
}
