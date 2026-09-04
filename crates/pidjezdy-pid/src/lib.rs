//! Adapter for PID's undocumented public departure-board endpoint.

mod client;
mod request;
mod response;

pub use client::{DEFAULT_ENDPOINT, PidClient, PidClientError};
pub use request::{DepartureBoardRequest, PidRequestError};
pub use response::{PidResponseError, parse_response};
