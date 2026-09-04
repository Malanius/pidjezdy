use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{BoardingPoint, Config};
use crate::departure::Departure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionOptions {
    pub limit: usize,
    pub include_unreachable: bool,
}

impl SelectionOptions {
    #[must_use]
    pub const fn from_config(config: &Config) -> Self {
        Self {
            limit: config.display.max_departures,
            include_unreachable: false,
        }
    }
}

/// A departure enriched with the configured journey to its boarding point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedDeparture {
    pub departure: Departure,
    pub boarding_point_name: String,
    pub walking_minutes: u32,
    pub safety_buffer_minutes: u32,
    pub departs_in_seconds: i64,
    pub leave_in_seconds: i64,
    pub reachable: bool,
}

impl SelectedDeparture {
    #[must_use]
    pub fn departs_in_minutes(&self) -> i64 {
        floor_minutes(self.departs_in_seconds)
    }

    #[must_use]
    pub fn leave_in_minutes(&self) -> i64 {
        floor_minutes(self.leave_in_seconds)
    }
}

#[derive(Debug)]
struct Candidate {
    selected: SelectedDeparture,
    configuration_order: usize,
}

/// Match, rank, deduplicate, and limit provider-independent departures.
#[must_use]
pub fn select_departures(
    config: &Config,
    departures: &[Departure],
    now: DateTime<Utc>,
    options: SelectionOptions,
) -> Vec<SelectedDeparture> {
    let candidates = departures
        .iter()
        .filter(|departure| !departure.is_cancelled)
        .filter_map(|departure| matching_point(config, departure).map(|entry| (departure, entry)))
        .map(|(departure, (point, configuration_order))| {
            candidate(departure, point, configuration_order, now)
        })
        .filter(|candidate| options.include_unreachable || candidate.selected.reachable)
        .collect::<Vec<_>>();

    let mut deduplicated: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        let trip_id = candidate.selected.departure.trip_id.trim();
        if !trip_id.is_empty()
            && let Some(index) = deduplicated
                .iter()
                .position(|existing| existing.selected.departure.trip_id.trim() == trip_id)
        {
            if better_boarding_point(&candidate, &deduplicated[index]) {
                deduplicated[index] = candidate;
            }
            continue;
        }
        deduplicated.push(candidate);
    }

    deduplicated.sort_by(|left, right| {
        left.selected
            .departure
            .effective_at()
            .cmp(&right.selected.departure.effective_at())
            .then_with(|| {
                right
                    .selected
                    .leave_in_seconds
                    .cmp(&left.selected.leave_in_seconds)
            })
            .then_with(|| left.configuration_order.cmp(&right.configuration_order))
    });

    deduplicated
        .into_iter()
        .take(options.limit)
        .map(|candidate| candidate.selected)
        .collect()
}

fn matching_point<'a>(
    config: &'a Config,
    departure: &Departure,
) -> Option<(&'a BoardingPoint, usize)> {
    config
        .boarding_points
        .iter()
        .enumerate()
        .find(|(_, point)| {
            point
                .stop_ids
                .iter()
                .any(|stop_id| stop_id.trim() == departure.stop_id.trim())
                && point.routes.iter().any(|route| {
                    route.line.trim() == departure.line.trim()
                        && route.headsign.trim() == departure.headsign.trim()
                })
        })
        .map(|(index, point)| (point, index))
}

fn candidate(
    departure: &Departure,
    point: &BoardingPoint,
    configuration_order: usize,
    now: DateTime<Utc>,
) -> Candidate {
    let departs_in_seconds = (departure.effective_at() - now).num_seconds();
    let journey_seconds =
        (i64::from(point.walking_minutes) + i64::from(point.safety_buffer_minutes)) * 60;
    let leave_in_seconds = departs_in_seconds - journey_seconds;

    Candidate {
        selected: SelectedDeparture {
            departure: departure.clone(),
            boarding_point_name: point.name.clone(),
            walking_minutes: point.walking_minutes,
            safety_buffer_minutes: point.safety_buffer_minutes,
            departs_in_seconds,
            leave_in_seconds,
            reachable: leave_in_seconds >= 0,
        },
        configuration_order,
    }
}

fn better_boarding_point(candidate: &Candidate, incumbent: &Candidate) -> bool {
    candidate.selected.leave_in_seconds > incumbent.selected.leave_in_seconds
        || (candidate.selected.leave_in_seconds == incumbent.selected.leave_in_seconds
            && (candidate.selected.departure.effective_at()
                < incumbent.selected.departure.effective_at()
                || (candidate.selected.departure.effective_at()
                    == incumbent.selected.departure.effective_at()
                    && candidate.configuration_order < incumbent.configuration_order)))
}

const fn floor_minutes(seconds: i64) -> i64 {
    seconds.div_euclid(60)
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::config::{BoardingPoint, DisplayConfig, FetchConfig, RouteFilter};
    use crate::departure::Vehicle;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-04T12:00:00+02:00")
            .unwrap()
            .to_utc()
    }

    fn point(name: &str, stop_id: &str, walking_minutes: u32) -> BoardingPoint {
        BoardingPoint {
            name: name.into(),
            stop_ids: vec![stop_id.into()],
            walking_minutes,
            safety_buffer_minutes: 2,
            routes: vec![RouteFilter {
                line: "158".into(),
                headsign: "Letňany".into(),
            }],
        }
    }

    fn config(points: Vec<BoardingPoint>) -> Config {
        Config {
            display: DisplayConfig { max_departures: 3 },
            fetch: FetchConfig::default(),
            boarding_points: points,
        }
    }

    fn departure(trip_id: &str, stop_id: &str, minutes_after_now: i64) -> Departure {
        Departure {
            trip_id: trip_id.into(),
            line: "158".into(),
            headsign: "Letňany".into(),
            stop_id: stop_id.into(),
            platform_code: Some("A".into()),
            scheduled_at: now() + TimeDelta::minutes(minutes_after_now),
            predicted_at: None,
            delay_seconds: None,
            is_cancelled: false,
            vehicle: Vehicle::default(),
        }
    }

    fn options(limit: usize) -> SelectionOptions {
        SelectionOptions {
            limit,
            include_unreachable: false,
        }
    }

    #[test]
    fn exact_leave_boundary_is_reachable() {
        let selected = select_departures(
            &config(vec![point("Near", "U1", 4)]),
            &[departure("trip", "U1", 6)],
            now(),
            options(3),
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].leave_in_seconds, 0);
        assert!(selected[0].reachable);
    }

    #[test]
    fn filters_unreachable_cancelled_and_unconfigured_departures() {
        let config = config(vec![point("Near", "U1", 4)]);
        let mut cancelled = departure("cancelled", "U1", 10);
        cancelled.is_cancelled = true;
        let mut wrong_direction = departure("wrong-direction", "U1", 10);
        wrong_direction.headsign = "Elsewhere".into();
        let departures = [
            departure("too-soon", "U1", 5),
            cancelled,
            wrong_direction,
            departure("wrong-stop", "U2", 10),
            departure("wanted", "U1", 10),
        ];

        let selected = select_departures(&config, &departures, now(), options(3));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].departure.trip_id, "wanted");
    }

    #[test]
    fn can_include_unreachable_departures_for_diagnostics() {
        let selected = select_departures(
            &config(vec![point("Near", "U1", 4)]),
            &[departure("too-soon", "U1", 5)],
            now(),
            SelectionOptions {
                limit: 3,
                include_unreachable: true,
            },
        );
        assert_eq!(selected.len(), 1);
        assert!(!selected[0].reachable);
        assert_eq!(selected[0].leave_in_seconds, -60);
    }

    #[test]
    fn prediction_controls_reachability() {
        let mut delayed = departure("delayed", "U1", 5);
        delayed.predicted_at = Some(now() + TimeDelta::minutes(8));

        let selected = select_departures(
            &config(vec![point("Near", "U1", 4)]),
            &[delayed],
            now(),
            options(3),
        );
        assert_eq!(selected[0].departs_in_seconds, 8 * 60);
        assert_eq!(selected[0].leave_in_seconds, 2 * 60);
    }

    #[test]
    fn duplicate_trip_uses_boarding_point_with_most_slack() {
        let config = config(vec![point("Near", "U1", 4), point("Far", "U2", 8)]);
        let departures = [
            departure("same-trip", "U2", 12),
            departure("same-trip", "U1", 10),
        ];

        let selected = select_departures(&config, &departures, now(), options(3));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].boarding_point_name, "Near");
        assert_eq!(selected[0].leave_in_seconds, 4 * 60);
    }

    #[test]
    fn duplicate_tie_prefers_earlier_departure_then_configuration_order() {
        let configured = config(vec![point("First", "U1", 4), point("Second", "U2", 6)]);
        let departures = [
            departure("same-trip", "U2", 12),
            departure("same-trip", "U1", 10),
        ];

        let selected = select_departures(&configured, &departures, now(), options(3));
        assert_eq!(selected[0].boarding_point_name, "First");

        let equal_points = config(vec![point("First", "U1", 4), point("Second", "U2", 4)]);
        let same_time = [
            departure("same-trip", "U2", 12),
            departure("same-trip", "U1", 12),
        ];
        let selected = select_departures(&equal_points, &same_time, now(), options(3));
        assert_eq!(selected[0].boarding_point_name, "First");
    }

    #[test]
    fn empty_trip_ids_are_not_deduplicated() {
        let selected = select_departures(
            &config(vec![point("Near", "U1", 1)]),
            &[departure("", "U1", 10), departure("", "U1", 11)],
            now(),
            options(3),
        );
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn sorts_chronologically_then_applies_limit() {
        let selected = select_departures(
            &config(vec![point("Near", "U1", 1)]),
            &[
                departure("third", "U1", 30),
                departure("first", "U1", 10),
                departure("second", "U1", 20),
            ],
            now(),
            options(2),
        );
        assert_eq!(
            selected
                .iter()
                .map(|item| item.departure.trip_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn rounds_minutes_down_conservatively() {
        let mut selected = SelectedDeparture {
            departure: departure("trip", "U1", 10),
            boarding_point_name: "Near".into(),
            walking_minutes: 1,
            safety_buffer_minutes: 0,
            departs_in_seconds: 119,
            leave_in_seconds: 59,
            reachable: true,
        };
        assert_eq!(selected.departs_in_minutes(), 1);
        assert_eq!(selected.leave_in_minutes(), 0);

        selected.leave_in_seconds = -1;
        assert_eq!(selected.leave_in_minutes(), -1);
    }
}
