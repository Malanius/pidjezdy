use std::cmp::Reverse;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{BoardingPoint, Config, RouteQuota};
use crate::departure::Departure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionOptions {
    pub limit: usize,
    pub include_unreachable: bool,
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

impl Candidate {
    fn deduplication_key(&self) -> (Reverse<i64>, DateTime<Utc>, usize) {
        (
            Reverse(self.selected.leave_in_seconds),
            self.selected.departure.effective_at(),
            self.configuration_order,
        )
    }

    fn ranking_key(&self) -> (DateTime<Utc>, Reverse<i64>, usize) {
        (
            self.selected.departure.effective_at(),
            Reverse(self.selected.leave_in_seconds),
            self.configuration_order,
        )
    }
}

/// Match, rank, deduplicate, and limit provider-independent departures.
///
/// Configuration and departure text is expected to have been normalized at
/// its parsing or provider boundary.
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
        let trip_id = &candidate.selected.departure.trip_id;
        if !trip_id.is_empty()
            && let Some(index) = deduplicated
                .iter()
                .position(|existing| existing.selected.departure.trip_id.eq(trip_id))
        {
            if candidate.deduplication_key() < deduplicated[index].deduplication_key() {
                deduplicated[index] = candidate;
            }
            continue;
        }
        deduplicated.push(candidate);
    }

    deduplicated.sort_by_key(Candidate::ranking_key);

    let selected = select_candidate_indices(config, &deduplicated, options.limit);
    deduplicated
        .into_iter()
        .zip(selected)
        .filter_map(|(candidate, keep)| keep.then_some(candidate.selected))
        .collect()
}

fn select_candidate_indices(config: &Config, candidates: &[Candidate], limit: usize) -> Vec<bool> {
    let mut selected = vec![false; candidates.len()];
    let mut selected_count = 0;
    let rounds = config
        .display
        .route_quotas
        .iter()
        .map(|quota| quota.minimum_departures)
        .max()
        .unwrap_or(0);

    for round in 0..rounds {
        let mut round_candidates = config
            .display
            .route_quotas
            .iter()
            .filter(|quota| round < quota.minimum_departures)
            .filter_map(|quota| {
                candidates
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| quota_matches(quota, &candidate.selected.departure))
                    .nth(round)
                    .map(|(index, _)| index)
            })
            .collect::<Vec<_>>();
        round_candidates.sort_unstable();
        round_candidates.dedup();

        for index in round_candidates {
            if selected_count == limit {
                return selected;
            }
            if !selected[index] {
                selected[index] = true;
                selected_count += 1;
            }
        }
    }

    for is_selected in &mut selected {
        if selected_count == limit {
            break;
        }
        if !*is_selected {
            *is_selected = true;
            selected_count += 1;
        }
    }
    selected
}

fn quota_matches(quota: &RouteQuota, departure: &Departure) -> bool {
    quota.line == departure.line && quota.headsign == departure.headsign
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
                .any(|stop_id| stop_id == &departure.stop_id)
                && point.routes.iter().any(|route| {
                    route.line == departure.line && route.headsign == departure.headsign
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

const fn floor_minutes(seconds: i64) -> i64 {
    seconds.div_euclid(60)
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::config::{BoardingPoint, DisplayConfig, FetchConfig, RouteFilter, RouteQuota};
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
            display: DisplayConfig {
                max_departures: 3,
                route_quotas: Vec::new(),
            },
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

    fn departure_for(
        trip_id: &str,
        line: &str,
        headsign: &str,
        minutes_after_now: i64,
    ) -> Departure {
        let mut departure = departure(trip_id, "U1", minutes_after_now);
        departure.line = line.into();
        departure.headsign = headsign.into();
        departure
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
    fn reserves_configured_minimums_before_filling_the_global_limit() {
        let mut configured = config(vec![BoardingPoint {
            name: "Near".into(),
            stop_ids: vec!["U1".into()],
            walking_minutes: 1,
            safety_buffer_minutes: 0,
            routes: vec![
                RouteFilter {
                    line: "158".into(),
                    headsign: "Metro".into(),
                },
                RouteFilter {
                    line: "195".into(),
                    headsign: "Town".into(),
                },
            ],
        }]);
        configured.display.max_departures = 4;
        configured.display.route_quotas = vec![RouteQuota {
            line: "195".into(),
            headsign: "Town".into(),
            minimum_departures: 2,
        }];
        let departures = [
            departure_for("158-1", "158", "Metro", 10),
            departure_for("158-2", "158", "Metro", 11),
            departure_for("158-3", "158", "Metro", 12),
            departure_for("195-1", "195", "Town", 20),
            departure_for("195-2", "195", "Town", 21),
        ];

        let selected = select_departures(&configured, &departures, now(), options(4));
        assert_eq!(
            selected
                .iter()
                .map(|item| item.departure.trip_id.as_str())
                .collect::<Vec<_>>(),
            ["158-1", "158-2", "195-1", "195-2"]
        );
    }

    #[test]
    fn hard_limit_allocates_quota_slots_fairly_across_routes() {
        let mut configured = config(vec![BoardingPoint {
            name: "Near".into(),
            stop_ids: vec!["U1".into()],
            walking_minutes: 1,
            safety_buffer_minutes: 0,
            routes: vec![
                RouteFilter {
                    line: "158".into(),
                    headsign: "Metro".into(),
                },
                RouteFilter {
                    line: "195".into(),
                    headsign: "Town".into(),
                },
                RouteFilter {
                    line: "201".into(),
                    headsign: "Station".into(),
                },
            ],
        }]);
        configured.display.route_quotas = [("158", "Metro"), ("195", "Town"), ("201", "Station")]
            .into_iter()
            .map(|(line, headsign)| RouteQuota {
                line: line.into(),
                headsign: headsign.into(),
                minimum_departures: 2,
            })
            .collect();
        let departures = [
            departure_for("158-1", "158", "Metro", 10),
            departure_for("158-2", "158", "Metro", 11),
            departure_for("195-1", "195", "Town", 20),
            departure_for("195-2", "195", "Town", 21),
            departure_for("201-1", "201", "Station", 30),
            departure_for("201-2", "201", "Station", 31),
        ];

        let selected = select_departures(&configured, &departures, now(), options(3));
        assert_eq!(
            selected
                .iter()
                .map(|item| item.departure.trip_id.as_str())
                .collect::<Vec<_>>(),
            ["158-1", "195-1", "201-1"]
        );
    }

    #[test]
    fn quota_shortfall_spills_into_chronological_fill() {
        let mut configured = quota_config(4);
        let departures = [
            departure_for("158-1", "158", "Metro", 10),
            departure_for("unreserved", "100", "Local", 11),
            departure_for("195-1", "195", "Town", 20),
            departure_for("195-2", "195", "Town", 21),
        ];

        configured.display.route_quotas = vec![quota("158", "Metro", 2), quota("195", "Town", 2)];
        let selected = select_departures(&configured, &departures, now(), options(4));

        assert_eq!(
            trip_ids(&selected),
            ["158-1", "unreserved", "195-1", "195-2"]
        );
    }

    #[test]
    fn hard_limit_smaller_than_quota_count_uses_earliest_routes() {
        let mut configured = quota_config(3);
        configured.display.route_quotas = vec![
            quota("158", "Metro", 1),
            quota("195", "Town", 1),
            quota("201", "Station", 1),
        ];
        let departures = [
            departure_for("158-1", "158", "Metro", 10),
            departure_for("195-1", "195", "Town", 20),
            departure_for("201-1", "201", "Station", 30),
        ];

        let selected = select_departures(&configured, &departures, now(), options(2));

        assert_eq!(trip_ids(&selected), ["158-1", "195-1"]);
    }

    #[test]
    fn empty_quota_route_does_not_block_generic_fill() {
        let mut configured = quota_config(3);
        configured.display.route_quotas =
            vec![quota("158", "Metro", 2), quota("201", "Station", 1)];
        let departures = [
            departure_for("158-1", "158", "Metro", 10),
            departure_for("unreserved", "100", "Local", 11),
            departure_for("158-2", "158", "Metro", 12),
        ];

        let selected = select_departures(&configured, &departures, now(), options(3));

        assert_eq!(trip_ids(&selected), ["158-1", "unreserved", "158-2"]);
    }

    fn quota_config(max_departures: usize) -> Config {
        Config {
            display: DisplayConfig {
                max_departures,
                route_quotas: Vec::new(),
            },
            fetch: FetchConfig::default(),
            boarding_points: vec![BoardingPoint {
                name: "Near".into(),
                stop_ids: vec!["U1".into()],
                walking_minutes: 1,
                safety_buffer_minutes: 0,
                routes: [
                    ("100", "Local"),
                    ("158", "Metro"),
                    ("195", "Town"),
                    ("201", "Station"),
                ]
                .into_iter()
                .map(|(line, headsign)| RouteFilter {
                    line: line.into(),
                    headsign: headsign.into(),
                })
                .collect(),
            }],
        }
    }

    fn quota(line: &str, headsign: &str, minimum_departures: usize) -> RouteQuota {
        RouteQuota {
            line: line.into(),
            headsign: headsign.into(),
            minimum_departures,
        }
    }

    fn trip_ids(departures: &[SelectedDeparture]) -> Vec<&str> {
        departures
            .iter()
            .map(|departure| departure.departure.trip_id.as_str())
            .collect()
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
