use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A normalized departure independent of the upstream provider response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Departure {
    pub trip_id: String,
    pub line: String,
    pub headsign: String,
    pub stop_id: String,
    pub platform_code: Option<String>,
    pub scheduled_at: DateTime<Utc>,
    pub predicted_at: Option<DateTime<Utc>>,
    pub delay_seconds: Option<i64>,
    pub is_cancelled: bool,
    pub vehicle: Vehicle,
}

impl Departure {
    #[must_use]
    pub fn effective_at(&self) -> DateTime<Utc> {
        self.predicted_at.unwrap_or(self.scheduled_at)
    }
}

/// Vehicle information may be absent when a trip is not tracked in real time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vehicle {
    pub id: Option<String>,
    pub is_wheelchair_accessible: Option<bool>,
    pub is_air_conditioned: Option<bool>,
    pub has_charger: Option<bool>,
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    fn departure() -> Departure {
        let scheduled_at = DateTime::parse_from_rfc3339("2026-09-04T12:00:00+02:00")
            .unwrap()
            .to_utc();
        Departure {
            trip_id: "trip-1".into(),
            line: "158".into(),
            headsign: "Letňany".into(),
            stop_id: "U1".into(),
            platform_code: Some("A".into()),
            scheduled_at,
            predicted_at: None,
            delay_seconds: None,
            is_cancelled: false,
            vehicle: Vehicle::default(),
        }
    }

    #[test]
    fn scheduled_time_is_effective_without_prediction() {
        let departure = departure();
        assert_eq!(departure.effective_at(), departure.scheduled_at);
    }

    #[test]
    fn prediction_is_effective_when_present() {
        let mut departure = departure();
        departure.predicted_at = Some(departure.scheduled_at + TimeDelta::minutes(3));
        assert_eq!(departure.effective_at(), departure.predicted_at.unwrap());
    }
}
