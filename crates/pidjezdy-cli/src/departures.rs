use chrono::{DateTime, Utc};
use pidjezdy_core::config::Config;
use pidjezdy_core::selection::{SelectedDeparture, SelectionOptions, select_departures};
use pidjezdy_pid::{DepartureBoardRequest, PidClient, PidClientError, PidRequestError};
use thiserror::Error;

#[derive(Debug)]
pub(crate) struct DepartureQuery {
    pub(crate) generated_at: DateTime<Utc>,
    pub(crate) departures: Vec<SelectedDeparture>,
}

#[derive(Debug, Error)]
pub(crate) enum DepartureQueryError {
    #[error("could not build PID departure request: {0}")]
    Request(#[from] PidRequestError),
    #[error("could not create PID client: {0}")]
    CreateClient(#[source] PidClientError),
    #[error("could not fetch PID departures: {0}")]
    Fetch(#[source] PidClientError),
}

/// Fetch, filter, rank, and limit departures using validated configuration.
///
/// # Errors
///
/// Returns an error if a PID request or client cannot be constructed, or if
/// fetching and decoding the provider response fails.
pub(crate) fn query_departures(
    config: &Config,
    limit: usize,
) -> Result<DepartureQuery, DepartureQueryError> {
    let request = configured_request(config)?;
    let client = PidClient::new().map_err(DepartureQueryError::CreateClient)?;
    let departures = client.fetch(&request).map_err(DepartureQueryError::Fetch)?;
    let generated_at = Utc::now();
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
        departures,
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
    use url::Url;

    use super::*;

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
}
