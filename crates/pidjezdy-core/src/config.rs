use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_MAX_DEPARTURES: usize = 3;
const DEFAULT_MINUTES_AFTER: u32 = 120;
const DEFAULT_API_LIMIT: usize = 20;
const DEFAULT_SAFETY_BUFFER_MINUTES: u32 = 2;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub display: DisplayConfig,
    pub fetch: FetchConfig,
    pub boarding_points: Vec<BoardingPoint>,
}

impl Config {
    /// Parse and validate a TOML configuration document.
    ///
    /// # Errors
    ///
    /// Returns a syntax error for invalid TOML or all semantic validation
    /// errors found in a syntactically valid document.
    pub fn from_toml(input: &str) -> Result<Self, ConfigError> {
        let mut config: Self = toml::from_str(input)?;
        config.normalize();
        config.validate_normalized()?;
        Ok(config)
    }

    /// Normalize user-provided text in place.
    ///
    /// [`Self::from_toml`] performs this automatically. Callers that construct
    /// a `Config` directly should normalize it before passing it to departure
    /// selection.
    pub fn normalize(&mut self) {
        for quota in &mut self.display.route_quotas {
            quota.line = quota.line.trim().to_owned();
            quota.headsign = quota.headsign.trim().to_owned();
        }
        for point in &mut self.boarding_points {
            point.name = point.name.trim().to_owned();
            for stop_id in &mut point.stop_ids {
                *stop_id = stop_id.trim().to_owned();
            }
            for route in &mut point.routes {
                route.line = route.line.trim().to_owned();
                route.headsign = route.headsign.trim().to_owned();
            }
        }
    }

    /// Validate configuration invariants.
    ///
    /// # Errors
    ///
    /// Returns every detected semantic error in declaration order.
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut normalized = self.clone();
        normalized.normalize();
        normalized.validate_normalized()
    }

    fn validate_normalized(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();

        if !(1..=20).contains(&self.display.max_departures) {
            errors.push("display.max_departures must be between 1 and 20".into());
        }
        if self.fetch.minutes_after == 0 {
            errors.push("fetch.minutes_after must be greater than 0".into());
        }
        if !(1..=20).contains(&self.fetch.api_limit) {
            errors.push("fetch.api_limit must be between 1 and 20".into());
        }
        if self.boarding_points.is_empty() {
            errors.push("at least one boarding point is required".into());
        }

        let configured_routes = validate_boarding_points(&self.boarding_points, &mut errors);

        validate_route_quotas(&self.display, &configured_routes, &mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(errors))
        }
    }
}

fn validate_boarding_points<'a>(
    boarding_points: &'a [BoardingPoint],
    errors: &mut Vec<String>,
) -> HashSet<(&'a str, &'a str)> {
    let mut all_stop_ids = HashSet::new();
    let mut configured_routes = HashSet::new();
    for (point_index, point) in boarding_points.iter().enumerate() {
        let prefix = format!("boarding_points[{point_index}]");
        if point.name.is_empty() {
            errors.push(format!("{prefix}.name must not be empty"));
        }
        if point.walking_minutes == 0 {
            errors.push(format!("{prefix}.walking_minutes must be greater than 0"));
        }
        if point.stop_ids.is_empty() {
            errors.push(format!("{prefix}.stop_ids must not be empty"));
        }
        for (stop_index, stop_id) in point.stop_ids.iter().enumerate() {
            if stop_id.is_empty() {
                errors.push(format!("{prefix}.stop_ids[{stop_index}] must not be empty"));
            } else if !all_stop_ids.insert(stop_id.as_str()) {
                errors.push(format!("stop ID {stop_id:?} is configured more than once"));
            }
        }
        if point.routes.is_empty() {
            errors.push(format!("{prefix}.routes must not be empty"));
        }
        let mut routes = HashSet::new();
        for (route_index, route) in point.routes.iter().enumerate() {
            let route_prefix = format!("{prefix}.routes[{route_index}]");
            let complete = !route.line.is_empty() && !route.headsign.is_empty();
            if route.line.is_empty() {
                errors.push(format!("{route_prefix}.line must not be empty"));
            }
            if route.headsign.is_empty() {
                errors.push(format!("{route_prefix}.headsign must not be empty"));
            }
            if complete {
                let key = (route.line.as_str(), route.headsign.as_str());
                if !routes.insert(key) {
                    errors.push(format!(
                        "{route_prefix} duplicates line {:?} toward {:?}",
                        key.0, key.1
                    ));
                }
                configured_routes.insert(key);
            }
        }
    }
    configured_routes
}

fn validate_route_quotas(
    display: &DisplayConfig,
    configured_routes: &HashSet<(&str, &str)>,
    errors: &mut Vec<String>,
) {
    let mut quota_routes = HashSet::new();
    let mut minimum_total = 0usize;
    for (quota_index, quota) in display.route_quotas.iter().enumerate() {
        let prefix = format!("display.route_quotas[{quota_index}]");
        let key = (quota.line.as_str(), quota.headsign.as_str());
        if key.0.is_empty() {
            errors.push(format!("{prefix}.line must not be empty"));
        }
        if key.1.is_empty() {
            errors.push(format!("{prefix}.headsign must not be empty"));
        }
        if (1..=20).contains(&quota.minimum_departures) {
            minimum_total = minimum_total.saturating_add(quota.minimum_departures);
        } else {
            errors.push(format!(
                "{prefix}.minimum_departures must be between 1 and 20"
            ));
        }
        if !key.0.is_empty() && !key.1.is_empty() {
            if !quota_routes.insert(key) {
                errors.push(format!(
                    "{prefix} duplicates line {:?} toward {:?}",
                    key.0, key.1
                ));
            }
            if !configured_routes.contains(&key) {
                errors.push(format!(
                    "{prefix} does not match a configured boarding-point route"
                ));
            }
        }
    }
    if minimum_total > display.max_departures {
        errors.push(format!(
            "display route minimums total {minimum_total}, exceeding display.max_departures {}",
            display.max_departures
        ));
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayConfig {
    pub max_departures: usize,
    pub route_quotas: Vec<RouteQuota>,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            max_departures: DEFAULT_MAX_DEPARTURES,
            route_quotas: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteQuota {
    pub line: String,
    pub headsign: String,
    pub minimum_departures: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct FetchConfig {
    pub minutes_after: u32,
    pub api_limit: usize,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            minutes_after: DEFAULT_MINUTES_AFTER,
            api_limit: DEFAULT_API_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoardingPoint {
    pub name: String,
    pub stop_ids: Vec<String>,
    pub walking_minutes: u32,
    #[serde(default = "default_safety_buffer_minutes")]
    pub safety_buffer_minutes: u32,
    pub routes: Vec<RouteFilter>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteFilter {
    pub line: String,
    pub headsign: String,
}

const fn default_safety_buffer_minutes() -> u32 {
    DEFAULT_SAFETY_BUFFER_MINUTES
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error(transparent)]
    Validation(#[from] ValidationErrors),
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("configuration is invalid:\n{}", format_validation_errors(.0))]
pub struct ValidationErrors(pub Vec<String>);

fn format_validation_errors(errors: &[String]) -> String {
    errors
        .iter()
        .map(|error| format!("- {error}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
        [[boarding_points]]
        name = "Nearby stop"
        stop_ids = ["U123Z1P"]
        walking_minutes = 4

        [[boarding_points.routes]]
        line = "158"
        headsign = "Centrum"
    "#;

    #[test]
    fn parses_defaults() {
        let config = Config::from_toml(VALID).unwrap();
        assert_eq!(config.display.max_departures, 3);
        assert!(config.display.route_quotas.is_empty());
        assert_eq!(config.fetch.minutes_after, 120);
        assert_eq!(config.fetch.api_limit, 20);
        assert_eq!(config.boarding_points[0].safety_buffer_minutes, 2);
    }

    #[test]
    fn normalizes_configured_text_before_validation() {
        let config = Config::from_toml(
            r#"
                [display]
                max_departures = 2

                [[display.route_quotas]]
                line = " 158 "
                headsign = " Centrum "
                minimum_departures = 2

                [[boarding_points]]
                name = " Nearby stop "
                stop_ids = [" U123Z1P "]
                walking_minutes = 4

                [[boarding_points.routes]]
                line = " 158 "
                headsign = " Centrum "
            "#,
        )
        .unwrap();

        let point = &config.boarding_points[0];
        assert_eq!(point.name, "Nearby stop");
        assert_eq!(point.stop_ids, ["U123Z1P"]);
        assert_eq!(point.routes[0].line, "158");
        assert_eq!(point.routes[0].headsign, "Centrum");
        assert_eq!(config.display.route_quotas[0].line, "158");
        assert_eq!(config.display.route_quotas[0].headsign, "Centrum");
    }

    #[test]
    fn programmatic_configs_can_enforce_the_normalized_invariant() {
        let mut config = Config {
            display: DisplayConfig::default(),
            fetch: FetchConfig::default(),
            boarding_points: vec![BoardingPoint {
                name: " Nearby stop ".into(),
                stop_ids: vec![" U123Z1P ".into()],
                walking_minutes: 4,
                safety_buffer_minutes: 2,
                routes: vec![RouteFilter {
                    line: " 158 ".into(),
                    headsign: " Centrum ".into(),
                }],
            }],
        };

        config.normalize();
        config.validate().unwrap();

        assert_eq!(config.boarding_points[0].name, "Nearby stop");
        assert_eq!(config.boarding_points[0].stop_ids, ["U123Z1P"]);
        assert_eq!(config.boarding_points[0].routes[0].line, "158");
        assert_eq!(config.boarding_points[0].routes[0].headsign, "Centrum");
    }

    #[test]
    fn rejects_unknown_fields_at_every_config_layer() {
        let cases = [
            (
                format!("misspelled = true\n{VALID}"),
                "expected one of `display`, `fetch`, `boarding_points`",
            ),
            (
                format!("[display]\nmisspelled = true\n{VALID}"),
                "expected `max_departures` or `route_quotas`",
            ),
            (
                format!("[fetch]\nmisspelled = true\n{VALID}"),
                "expected `minutes_after` or `api_limit`",
            ),
            (
                format!(
                    r#"
                        [display]
                        max_departures = 2

                        [[display.route_quotas]]
                        line = "158"
                        headsign = "Centrum"
                        minimum_departures = 2
                        misspelled = true

                        {VALID}
                    "#
                ),
                "expected one of `line`, `headsign`, `minimum_departures`",
            ),
            (
                r#"
                    [[boarding_points]]
                    name = "Nearby stop"
                    stop_ids = ["U123Z1P"]
                    walking_minutes = 4
                    misspelled = true

                    [[boarding_points.routes]]
                    line = "158"
                    headsign = "Centrum"
                "#
                .to_owned(),
                concat!(
                    "expected one of `name`, `stop_ids`, `walking_minutes`, ",
                    "`safety_buffer_minutes`, `routes`"
                ),
            ),
            (
                format!("{VALID}\nmisspelled = true"),
                "expected `line` or `headsign`",
            ),
        ];

        for (input, expected_fields) in cases {
            let message = Config::from_toml(&input).unwrap_err().to_string();
            assert!(message.contains("unknown field `misspelled`"), "{message}");
            assert!(message.contains(expected_fields), "{message}");
        }
    }

    #[test]
    fn reports_multiple_validation_errors() {
        let config = Config {
            display: DisplayConfig {
                max_departures: 0,
                route_quotas: Vec::new(),
            },
            fetch: FetchConfig {
                minutes_after: 0,
                api_limit: 21,
            },
            boarding_points: Vec::new(),
        };
        let errors = config.validate().unwrap_err();
        assert_eq!(errors.0.len(), 4);
    }

    #[test]
    fn rejects_duplicate_stop_ids_and_routes() {
        let input = r#"
            [[boarding_points]]
            name = "One"
            stop_ids = [" U1 ", "U1"]
            walking_minutes = 1

            [[boarding_points.routes]]
            line = "1"
            headsign = "There"

            [[boarding_points.routes]]
            line = "1"
            headsign = "There"
        "#;
        let error = Config::from_toml(input).unwrap_err().to_string();
        assert!(error.contains("configured more than once"));
        assert!(error.contains("stop ID \"U1\""));
        assert!(!error.contains("stop ID \" U1 \""));
        assert!(error.contains("duplicates line"));
    }

    #[test]
    fn parses_valid_route_quotas() {
        let input = format!(
            r#"
                [display]
                max_departures = 4

                [[display.route_quotas]]
                line = "158"
                headsign = "Centrum"
                minimum_departures = 2

                {VALID}
            "#
        );
        let config = Config::from_toml(&input).unwrap();
        assert_eq!(config.display.route_quotas.len(), 1);
        assert_eq!(config.display.route_quotas[0].minimum_departures, 2);
    }

    #[test]
    fn validates_route_quotas() {
        let input = format!(
            r#"
                [display]
                max_departures = 2

                [[display.route_quotas]]
                line = "158"
                headsign = "Centrum"
                minimum_departures = 2

                [[display.route_quotas]]
                line = "158"
                headsign = "Centrum"
                minimum_departures = 2

                [[display.route_quotas]]
                line = "999"
                headsign = "Nowhere"
                minimum_departures = 0

                {VALID}
            "#
        );
        let error = Config::from_toml(&input).unwrap_err().to_string();
        assert!(error.contains("duplicates line"));
        assert!(error.contains("must be between 1 and 20"));
        assert!(error.contains("does not match a configured boarding-point route"));
        assert!(error.contains("route minimums total 4"));
    }
}
