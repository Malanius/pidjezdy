use std::io::Read;
use std::time::Duration;

use flate2::read::GzDecoder;
use pidjezdy_core::departure::Departure;
use reqwest::blocking::Client;
use reqwest::header::ACCEPT_ENCODING;
use thiserror::Error;
use url::Url;

use crate::request::DepartureBoardRequest;
use crate::response::{PidResponseError, parse_response};

pub const DEFAULT_ENDPOINT: &str = "https://data.pid.cz/departures/data.php";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

pub struct PidClient {
    http: Client,
    endpoint: Url,
}

impl PidClient {
    /// Create a client for PID's public departure-board endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client or built-in endpoint cannot be
    /// constructed.
    pub fn new() -> Result<Self, PidClientError> {
        Self::with_endpoint(DEFAULT_ENDPOINT)
    }

    /// Create a client with a custom endpoint, primarily for deterministic
    /// integration tests.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid endpoint or HTTP client setup failure.
    pub fn with_endpoint(endpoint: &str) -> Result<Self, PidClientError> {
        let http = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(PidClientError::BuildClient)?;
        let endpoint = Url::parse(endpoint).map_err(PidClientError::InvalidEndpoint)?;
        Ok(Self { http, endpoint })
    }

    /// Fetch and normalize departures without retries.
    ///
    /// # Errors
    ///
    /// Returns transport, HTTP status, compression, or PID response errors.
    pub fn fetch(&self, request: &DepartureBoardRequest) -> Result<Vec<Departure>, PidClientError> {
        let response = self
            .http
            .get(request.url(&self.endpoint))
            // PID sometimes compresses this response despite an identity
            // request. reqwest's gzip feature stays disabled so we can inspect
            // the bytes instead of trusting an inconsistent header.
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .map_err(PidClientError::Send)?;
        let status = response.status();
        let body = response.bytes().map_err(PidClientError::ReadBody)?;
        let decoded = decode_body(&body)?;

        if !status.is_success() {
            return Err(PidClientError::HttpStatus {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&decoded).into_owned(),
            });
        }

        parse_response(&decoded).map_err(PidClientError::Response)
    }
}

impl Default for PidClient {
    fn default() -> Self {
        Self::new().expect("the built-in PID endpoint and HTTP client must be valid")
    }
}

fn decode_body(body: &[u8]) -> Result<Vec<u8>, PidClientError> {
    if body.starts_with(&GZIP_MAGIC) {
        let mut decoded = Vec::new();
        GzDecoder::new(body)
            .read_to_end(&mut decoded)
            .map_err(PidClientError::Decompress)?;
        Ok(decoded)
    } else {
        Ok(body.to_vec())
    }
}

#[derive(Debug, Error)]
pub enum PidClientError {
    #[error("invalid PID endpoint: {0}")]
    InvalidEndpoint(url::ParseError),
    #[error("could not build HTTP client: {0}")]
    BuildClient(reqwest::Error),
    #[error("PID request failed: {0}")]
    Send(reqwest::Error),
    #[error("could not read PID response: {0}")]
    ReadBody(reqwest::Error),
    #[error("PID returned HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("could not decompress PID response: {0}")]
    Decompress(std::io::Error),
    #[error(transparent)]
    Response(PidResponseError),
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::thread;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    const DEPARTURES: &[u8] = include_bytes!("../tests/fixtures/departures.json");
    const ERROR: &[u8] = include_bytes!("../tests/fixtures/error.json");

    #[test]
    fn decodes_plain_and_gzip_bodies_by_magic_bytes() {
        assert_eq!(decode_body(DEPARTURES).unwrap(), DEPARTURES);

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(DEPARTURES).unwrap();
        let compressed = encoder.finish().unwrap();
        assert_eq!(decode_body(&compressed).unwrap(), DEPARTURES);
    }

    #[test]
    fn rejects_invalid_gzip_bodies() {
        assert!(matches!(
            decode_body(&[GZIP_MAGIC[0], GZIP_MAGIC[1], 0, 1, 2]),
            Err(PidClientError::Decompress(_))
        ));
    }

    #[test]
    fn fetch_handles_a_misleading_gzip_header() {
        let endpoint = serve_once(200, &[b"Content-Encoding: gzip"], DEPARTURES);
        let client = PidClient::with_endpoint(&endpoint).unwrap();
        let request = DepartureBoardRequest::new(120, 20, vec![vec!["U100Z1P".into()]]).unwrap();

        let departures = client.fetch(&request).unwrap();
        assert_eq!(departures.len(), 2);
    }

    #[test]
    fn fetch_handles_gzip_bytes_without_relying_on_the_header() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(DEPARTURES).unwrap();
        let compressed = encoder.finish().unwrap();
        let endpoint = serve_once(200, &[], &compressed);
        let client = PidClient::with_endpoint(&endpoint).unwrap();
        let request = DepartureBoardRequest::new(120, 20, vec![vec!["U100Z1P".into()]]).unwrap();

        let departures = client.fetch(&request).unwrap();
        assert_eq!(departures.len(), 2);
    }

    #[test]
    fn fetch_reports_unsuccessful_http_status() {
        let endpoint = serve_once(503, &[], b"temporarily unavailable");
        let client = PidClient::with_endpoint(&endpoint).unwrap();
        let request = DepartureBoardRequest::new(120, 20, vec![vec!["U100Z1P".into()]]).unwrap();

        assert!(matches!(
            client.fetch(&request),
            Err(PidClientError::HttpStatus { status: 503, .. })
        ));
    }

    #[test]
    fn fetch_reports_structured_api_errors_from_successful_http_responses() {
        let endpoint = serve_once(200, &[], ERROR);
        let client = PidClient::with_endpoint(&endpoint).unwrap();
        let request = DepartureBoardRequest::new(120, 20, vec![vec!["U100Z1P".into()]]).unwrap();

        assert!(matches!(
            client.fetch(&request),
            Err(PidClientError::Response(PidResponseError::Api {
                status: 400,
                ..
            }))
        ));
    }

    fn serve_once(status: u16, headers: &[&[u8]], body: &[u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let headers = headers
            .iter()
            .map(|header| header.to_vec())
            .collect::<Vec<_>>();
        let body = body.to_vec();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n",
                body.len()
            )
            .unwrap();
            for header in headers {
                stream.write_all(&header).unwrap();
                stream.write_all(b"\r\n").unwrap();
            }
            stream.write_all(b"Connection: close\r\n\r\n").unwrap();
            stream.write_all(&body).unwrap();
        });
        format!("http://{address}/data.php")
    }
}
