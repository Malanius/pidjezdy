//! Adapter for PID's undocumented public departure-board endpoint.

mod response;

pub use response::{PidResponseError, parse_response};
