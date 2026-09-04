use chrono::{DateTime, FixedOffset};
use pidjezdy_core::departure::{Departure, Vehicle};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PidResponseError {
    #[error("invalid PID response JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("PID API returned status {status}: {message}{details}")]
    Api {
        status: u16,
        message: String,
        details: ApiDetails,
    },
}

#[derive(Debug)]
pub struct ApiDetails(Option<String>);

impl std::fmt::Display for ApiDetails {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self
            .0
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            Some(text) => write!(formatter, " ({text})"),
            None => Ok(()),
        }
    }
}

/// Parse and normalize a successful PID response, flattening response groups.
///
/// Group order is deliberately discarded because the endpoint does not
/// reliably preserve request order. Callers associate records by `stop_id`.
///
/// # Errors
///
/// Returns malformed JSON, schema/timestamp errors, or a structured API error.
pub fn parse_response(input: &[u8]) -> Result<Vec<Departure>, PidResponseError> {
    match serde_json::from_slice::<ApiResponse>(input)? {
        ApiResponse::Departures(groups) => Ok(groups
            .into_iter()
            .flatten()
            .map(ApiDeparture::normalize)
            .collect()),
        ApiResponse::Error(error) => Err(PidResponseError::Api {
            status: error.status,
            message: error.message,
            details: ApiDetails(error.info),
        }),
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ApiResponse {
    Departures(Vec<Vec<ApiDeparture>>),
    Error(ApiError),
}

#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(rename = "error_message")]
    message: String,
    #[serde(rename = "error_status")]
    status: u16,
    #[serde(default, rename = "error_info")]
    info: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiDeparture {
    departure: ApiDepartureTime,
    stop: ApiStop,
    route: ApiRoute,
    trip: ApiTrip,
    #[serde(default)]
    vehicle: Option<ApiVehicle>,
}

impl ApiDeparture {
    fn normalize(self) -> Departure {
        Departure {
            trip_id: self.trip.id,
            line: self.route.short_name,
            headsign: self.trip.headsign,
            stop_id: self.stop.id,
            platform_code: non_empty(self.stop.platform_code),
            scheduled_at: self.departure.timestamp_scheduled.to_utc(),
            predicted_at: self
                .departure
                .timestamp_predicted
                .map(|timestamp| timestamp.to_utc()),
            delay_seconds: self.departure.delay_seconds,
            is_cancelled: self.trip.is_canceled,
            vehicle: self
                .vehicle
                .map_or_else(Vehicle::default, ApiVehicle::normalize),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApiDepartureTime {
    timestamp_scheduled: DateTime<FixedOffset>,
    #[serde(default)]
    timestamp_predicted: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    delay_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ApiStop {
    id: String,
    #[serde(default)]
    platform_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiRoute {
    short_name: String,
}

#[derive(Debug, Deserialize)]
struct ApiTrip {
    id: String,
    headsign: String,
    #[serde(default)]
    is_canceled: bool,
}

#[derive(Debug, Deserialize)]
struct ApiVehicle {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    is_wheelchair_accessible: Option<bool>,
    #[serde(default)]
    is_air_conditioned: Option<bool>,
    #[serde(default)]
    has_charger: Option<bool>,
}

impl ApiVehicle {
    fn normalize(self) -> Vehicle {
        Vehicle {
            id: non_empty(self.id),
            is_wheelchair_accessible: self.is_wheelchair_accessible,
            is_air_conditioned: self.is_air_conditioned,
            has_charger: self.has_charger,
        }
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPARTURES: &[u8] = include_bytes!("../tests/fixtures/departures.json");
    const ERROR: &[u8] = include_bytes!("../tests/fixtures/error.json");

    #[test]
    fn parses_flattens_and_normalizes_departures() {
        let departures = parse_response(DEPARTURES).unwrap();
        assert_eq!(departures.len(), 2);

        let tracked = &departures[0];
        assert_eq!(tracked.trip_id, "158_100_260901");
        assert_eq!(tracked.line, "158");
        assert_eq!(tracked.headsign, "Centrum");
        assert_eq!(tracked.stop_id, "U100Z1P");
        assert_eq!(tracked.platform_code.as_deref(), Some("A"));
        assert_eq!(
            tracked.scheduled_at.to_rfc3339(),
            "2026-09-04T10:10:00+00:00"
        );
        assert_eq!(
            tracked.predicted_at.unwrap().to_rfc3339(),
            "2026-09-04T10:12:00+00:00"
        );
        assert_eq!(tracked.delay_seconds, Some(120));
        assert_eq!(tracked.vehicle.id.as_deref(), Some("vehicle-1"));
        assert_eq!(tracked.vehicle.is_air_conditioned, Some(true));

        let untracked = &departures[1];
        assert_eq!(untracked.predicted_at, None);
        assert_eq!(untracked.delay_seconds, None);
        assert_eq!(untracked.platform_code, None);
        assert_eq!(untracked.vehicle, Vehicle::default());
    }

    #[test]
    fn reports_structured_api_errors() {
        let error = parse_response(ERROR).unwrap_err();
        assert!(matches!(
            error,
            PidResponseError::Api {
                status: 400,
                ref message,
                ..
            } if message == "Bad request"
        ));
        assert!(error.to_string().contains("Invalid value"));
    }

    #[test]
    fn reports_malformed_json_and_timestamps() {
        assert!(matches!(
            parse_response(b"not json"),
            Err(PidResponseError::Json(_))
        ));
        let invalid_time = DEPARTURES
            .windows(b"2026-09-04T12:10:00+02:00".len())
            .position(|window| window == b"2026-09-04T12:10:00+02:00")
            .unwrap();
        let mut body = DEPARTURES.to_vec();
        body.splice(
            invalid_time..invalid_time + b"2026-09-04T12:10:00+02:00".len(),
            b"invalid-time".iter().copied(),
        );
        assert!(matches!(
            parse_response(&body),
            Err(PidResponseError::Json(_))
        ));
    }
}
