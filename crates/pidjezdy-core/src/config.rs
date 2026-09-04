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
        let config: Self = toml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration invariants.
    ///
    /// # Errors
    ///
    /// Returns every detected semantic error in declaration order.
    pub fn validate(&self) -> Result<(), ValidationErrors> {
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

        let mut all_stop_ids = HashSet::new();
        for (point_index, point) in self.boarding_points.iter().enumerate() {
            let prefix = format!("boarding_points[{point_index}]");
            if point.name.trim().is_empty() {
                errors.push(format!("{prefix}.name must not be empty"));
            }
            if point.walking_minutes == 0 {
                errors.push(format!("{prefix}.walking_minutes must be greater than 0"));
            }
            if point.stop_ids.is_empty() {
                errors.push(format!("{prefix}.stop_ids must not be empty"));
            }
            for (stop_index, stop_id) in point.stop_ids.iter().enumerate() {
                let trimmed = stop_id.trim();
                if trimmed.is_empty() {
                    errors.push(format!("{prefix}.stop_ids[{stop_index}] must not be empty"));
                } else if !all_stop_ids.insert(trimmed.to_owned()) {
                    errors.push(format!("stop ID {trimmed:?} is configured more than once"));
                }
            }
            if point.routes.is_empty() {
                errors.push(format!("{prefix}.routes must not be empty"));
            }
            let mut routes = HashSet::new();
            for (route_index, route) in point.routes.iter().enumerate() {
                let route_prefix = format!("{prefix}.routes[{route_index}]");
                if route.line.trim().is_empty() {
                    errors.push(format!("{route_prefix}.line must not be empty"));
                }
                if route.headsign.trim().is_empty() {
                    errors.push(format!("{route_prefix}.headsign must not be empty"));
                }
                let key = (route.line.trim(), route.headsign.trim());
                if !key.0.is_empty() && !key.1.is_empty() && !routes.insert(key) {
                    errors.push(format!(
                        "{route_prefix} duplicates line {:?} toward {:?}",
                        key.0, key.1
                    ));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(errors))
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayConfig {
    pub max_departures: usize,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            max_departures: DEFAULT_MAX_DEPARTURES,
        }
    }
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
        assert_eq!(config.fetch.minutes_after, 120);
        assert_eq!(config.fetch.api_limit, 20);
        assert_eq!(config.boarding_points[0].safety_buffer_minutes, 2);
    }

    #[test]
    fn rejects_unknown_fields() {
        let error = Config::from_toml(&format!("{VALID}\nmisspelled = true")).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn reports_multiple_validation_errors() {
        let config = Config {
            display: DisplayConfig { max_departures: 0 },
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
            stop_ids = ["U1", "U1"]
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
        assert!(error.contains("duplicates line"));
    }
}
