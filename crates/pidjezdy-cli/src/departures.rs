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
    pub(crate) cancelled: Vec<SelectedDeparture>,
    pub(crate) warnings: Vec<DepartureWarning>,
}

#[derive(Debug, Error)]
pub(crate) enum DepartureWarning {
    #[error(transparent)]
    CacheWrite(#[from] CacheWriteError),
    #[error(
        "{boarding_point_name:?} hit the {count}-departure API limit, covering only the next {covered_minutes} of {requested_minutes} requested minutes; matching departures beyond that are not visible"
    )]
    ApiLimitTruncated {
        boarding_point_name: String,
        count: usize,
        covered_minutes: i64,
        requested_minutes: u32,
    },
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
    let (departures, data_updated_at, stale, warnings) = match sources.live {
        Ok(departures) => {
            let warnings = if departures.is_empty() {
                Vec::new()
            } else {
                (sources.write_cache)(&departures)
                    .err()
                    .map(DepartureWarning::from)
                    .into_iter()
                    .collect()
            };
            (departures, generated_at, false, warnings)
        }
        Err(live) => {
            let cached = (sources.read_cache)().map_err(|cache| {
                DepartureQueryError::Unavailable(DepartureUnavailable { live, cache })
            })?;
            (cached.departures, cached.fetched_at, true, Vec::new())
        }
    };
    let selection = select_departures(
        config,
        &departures,
        generated_at,
        SelectionOptions {
            limit,
            include_unreachable: false,
        },
    );
    let mut warnings = warnings;
    if !stale && selection.departures.len() < limit {
        warnings.extend(api_limit_warnings(config, &departures, generated_at));
    }
    Ok(DepartureQuery {
        generated_at,
        data_updated_at,
        stale,
        departures: selection.departures,
        cancelled: selection.cancelled,
        warnings,
    })
}

fn api_limit_warnings(
    config: &Config,
    departures: &[Departure],
    generated_at: DateTime<Utc>,
) -> Vec<DepartureWarning> {
    config
        .boarding_points
        .iter()
        .filter_map(|point| {
            let point_departures = departures.iter().filter(|departure| {
                point
                    .stop_ids
                    .iter()
                    .any(|stop_id| stop_id == &departure.stop_id)
            });
            let mut count = 0;
            let mut latest = None;
            for departure in point_departures {
                count += 1;
                latest = Some(
                    latest.map_or(departure.effective_at(), |current: DateTime<Utc>| {
                        current.max(departure.effective_at())
                    }),
                );
            }
            (count != 0 && count == config.fetch.api_limit).then(|| {
                DepartureWarning::ApiLimitTruncated {
                    boarding_point_name: point.name.clone(),
                    count,
                    covered_minutes: latest
                        .map_or(0, |latest| (latest - generated_at).num_minutes().max(0)),
                    requested_minutes: config.fetch.minutes_after,
                }
            })
        })
        .collect()
}

pub(crate) fn configured_request(
    config: &Config,
) -> Result<DepartureBoardRequest, PidRequestError> {
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
        departure_at("trip-1", "158", "Centre", 10)
    }

    fn departure_at(trip_id: &str, line: &str, headsign: &str, minutes: i64) -> Departure {
        departure_at_stop(trip_id, line, headsign, "U100Z1P", minutes)
    }

    fn departure_at_stop(
        trip_id: &str,
        line: &str,
        headsign: &str,
        stop_id: &str,
        minutes: i64,
    ) -> Departure {
        Departure {
            trip_id: trip_id.into(),
            line: line.into(),
            headsign: headsign.into(),
            stop_id: stop_id.into(),
            platform_code: Some("A".into()),
            scheduled_at: now() + TimeDelta::minutes(minutes),
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

    fn capped_config() -> Config {
        let mut config = fallback_config();
        config.fetch.api_limit = 3;
        config
    }

    fn short_capped_board() -> Vec<Departure> {
        vec![
            departure_at("matching", "158", "Centre", 10),
            departure_at("other-1", "900", "Elsewhere", 20),
            departure_at("other-2", "901", "Elsewhere", 27),
        ]
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
        assert!(result.warnings.is_empty());
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
            result.warnings.as_slice(),
            [DepartureWarning::CacheWrite(
                CacheWriteError::DirectoryUnavailable
            )]
        ));
    }

    #[test]
    fn warns_when_a_live_group_hits_the_api_limit_and_selection_is_short() {
        let result = finish_query_with(
            &capped_config(),
            3,
            now(),
            QuerySources {
                live: Ok(short_capped_board()),
                write_cache: |_: &[Departure]| Ok(()),
                read_cache: || unreachable!("a successful live request must not read the cache"),
            },
        )
        .unwrap();

        assert_eq!(result.departures.len(), 1);
        assert!(matches!(
            result.warnings.as_slice(),
            [DepartureWarning::ApiLimitTruncated {
                boarding_point_name,
                count: 3,
                covered_minutes: 27,
                requested_minutes: 120,
            }] if boarding_point_name == "Nearby stop"
        ));
        assert_eq!(
            result.warnings[0].to_string(),
            concat!(
                "\"Nearby stop\" hit the 3-departure API limit, covering only the next ",
                "27 of 120 requested minutes; matching departures beyond that are not visible"
            )
        );
    }

    #[test]
    fn warns_for_each_capped_boarding_point() {
        let config = Config::from_toml(
            r#"
                [fetch]
                api_limit = 2

                [[boarding_points]]
                name = "First"
                stop_ids = ["U1"]
                walking_minutes = 1

                [[boarding_points.routes]]
                line = "1"
                headsign = "Centre"

                [[boarding_points]]
                name = "Second"
                stop_ids = ["U2"]
                walking_minutes = 1

                [[boarding_points.routes]]
                line = "2"
                headsign = "Station"
            "#,
        )
        .unwrap();
        let departures = vec![
            departure_at_stop("matching", "1", "Centre", "U1", 10),
            departure_at_stop("first-other", "900", "Elsewhere", "U1", 25),
            departure_at_stop("second-other-1", "901", "Elsewhere", "U2", 15),
            departure_at_stop("second-other-2", "902", "Elsewhere", "U2", 35),
        ];
        let result = finish_query_with(
            &config,
            3,
            now(),
            QuerySources {
                live: Ok(departures),
                write_cache: |_: &[Departure]| Ok(()),
                read_cache: || unreachable!("a successful live request must not read the cache"),
            },
        )
        .unwrap();

        assert_eq!(result.warnings.len(), 2);
        assert!(matches!(
            &result.warnings[0],
            DepartureWarning::ApiLimitTruncated {
                boarding_point_name,
                count: 2,
                covered_minutes: 25,
                ..
            } if boarding_point_name == "First"
        ));
        assert!(matches!(
            &result.warnings[1],
            DepartureWarning::ApiLimitTruncated {
                boarding_point_name,
                count: 2,
                covered_minutes: 35,
                ..
            } if boarding_point_name == "Second"
        ));
    }

    #[test]
    fn does_not_warn_when_the_requested_selection_is_full() {
        let departures = vec![
            departure_at("matching-1", "158", "Centre", 10),
            departure_at("matching-2", "158", "Centre", 20),
            departure_at("matching-3", "158", "Centre", 27),
        ];
        let result = finish_query_with(
            &capped_config(),
            3,
            now(),
            QuerySources {
                live: Ok(departures),
                write_cache: |_: &[Departure]| Ok(()),
                read_cache: || unreachable!("a successful live request must not read the cache"),
            },
        )
        .unwrap();

        assert_eq!(result.departures.len(), 3);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn does_not_report_api_truncation_from_a_stale_cache() {
        let result = finish_query_with(
            &capped_config(),
            3,
            now(),
            QuerySources {
                live: Err(live_error()),
                write_cache: |_: &[Departure]| {
                    unreachable!("a failed live request must not update the cache")
                },
                read_cache: || {
                    Ok(CacheSnapshot {
                        fetched_at: now() - TimeDelta::minutes(3),
                        departures: short_capped_board(),
                    })
                },
            },
        )
        .unwrap();

        assert!(result.stale);
        assert_eq!(result.departures.len(), 1);
        assert!(result.warnings.is_empty());
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
        assert!(result.warnings.is_empty());
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
