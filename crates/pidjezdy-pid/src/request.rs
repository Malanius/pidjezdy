use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DepartureBoardRequest {
    minutes_after: u32,
    limit: u32,
    stop_groups: Vec<Vec<String>>,
}

impl DepartureBoardRequest {
    /// Create a validated departure-board request.
    ///
    /// Each stop group becomes a separate repeated `stopIds[]` query value so
    /// the endpoint applies its result limit independently to every group.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty time window, a limit outside the observed
    /// API range, empty groups, or blank stop IDs.
    pub fn new(
        minutes_after: u32,
        limit: usize,
        mut stop_groups: Vec<Vec<String>>,
    ) -> Result<Self, PidRequestError> {
        if minutes_after == 0 {
            return Err(PidRequestError::EmptyTimeWindow);
        }
        if !(1..=20).contains(&limit) {
            return Err(PidRequestError::InvalidLimit(limit));
        }
        let limit = u32::try_from(limit).map_err(|_| PidRequestError::InvalidLimit(limit))?;
        if stop_groups.is_empty() {
            return Err(PidRequestError::NoStopGroups);
        }
        for (group_index, group) in stop_groups.iter_mut().enumerate() {
            if group.is_empty() {
                return Err(PidRequestError::EmptyStopGroup(group_index));
            }
            for (stop_index, stop_id) in group.iter_mut().enumerate() {
                *stop_id = stop_id.trim().to_owned();
                if stop_id.is_empty() {
                    return Err(PidRequestError::EmptyStopId {
                        group: group_index,
                        stop: stop_index,
                    });
                }
            }
        }

        Ok(Self {
            minutes_after,
            limit,
            stop_groups,
        })
    }

    #[must_use]
    pub fn url(&self, endpoint: &Url) -> Url {
        let mut url = endpoint.clone();
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("minutesAfter", &self.minutes_after.to_string());
            query.append_pair("limit", &self.limit.to_string());
            for group in &self.stop_groups {
                query.append_pair("stopIds[]", &json!({ "0": group }).to_string());
            }
        }
        url
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum PidRequestError {
    #[error("minutes_after must be greater than 0")]
    EmptyTimeWindow,
    #[error("limit must be between 1 and 20, got {0}")]
    InvalidLimit(usize),
    #[error("at least one stop group is required")]
    NoStopGroups,
    #[error("stop group {0} must not be empty")]
    EmptyStopGroup(usize),
    #[error("stop ID at group {group}, index {stop} must not be empty")]
    EmptyStopId { group: usize, stop: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_repeated_encoded_stop_groups() {
        let request = DepartureBoardRequest::new(
            120,
            20,
            vec![
                vec![" U100Z1P ".into()],
                vec!["U200Z2P".into(), "U200Z3P".into()],
            ],
        )
        .unwrap();
        let endpoint = Url::parse("https://example.test/data.php").unwrap();
        let url = request.url(&endpoint);
        let pairs = url.query_pairs().collect::<Vec<_>>();

        assert_eq!(pairs[0], ("minutesAfter".into(), "120".into()));
        assert_eq!(pairs[1], ("limit".into(), "20".into()));
        assert_eq!(
            pairs[2],
            ("stopIds[]".into(), r#"{"0":["U100Z1P"]}"#.into())
        );
        assert_eq!(
            pairs[3],
            ("stopIds[]".into(), r#"{"0":["U200Z2P","U200Z3P"]}"#.into())
        );
    }

    #[test]
    fn rejects_values_outside_the_observed_contract() {
        assert_eq!(
            DepartureBoardRequest::new(0, 20, vec![vec!["U1".into()]]),
            Err(PidRequestError::EmptyTimeWindow)
        );
        assert_eq!(
            DepartureBoardRequest::new(120, 21, vec![vec!["U1".into()]]),
            Err(PidRequestError::InvalidLimit(21))
        );
        assert_eq!(
            DepartureBoardRequest::new(120, 20, Vec::new()),
            Err(PidRequestError::NoStopGroups)
        );
        assert_eq!(
            DepartureBoardRequest::new(120, 20, vec![Vec::new()]),
            Err(PidRequestError::EmptyStopGroup(0))
        );
        assert_eq!(
            DepartureBoardRequest::new(120, 20, vec![vec![" ".into()]]),
            Err(PidRequestError::EmptyStopId { group: 0, stop: 0 })
        );
    }

    #[test]
    fn request_round_trips_as_a_cache_key() {
        let request = DepartureBoardRequest::new(
            120,
            20,
            vec![vec![" U100Z1P ".into()], vec!["U200Z2P".into()]],
        )
        .unwrap();

        let encoded = serde_json::to_vec(&request).unwrap();
        let decoded = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(request, decoded);
    }
}
