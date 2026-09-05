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
const MAX_BODY_BYTES: u64 = 1024 * 1024;
const MAX_ERROR_BODY_BYTES: u64 = 1024;
const USER_AGENT: &str = concat!(
    "pidjezdy/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Malanius/pidjezdy)"
);

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
            .user_agent(USER_AGENT)
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
        let mut response = self
            .http
            .get(request.url(&self.endpoint))
            // PID sometimes compresses this response despite an identity
            // request. reqwest's gzip feature stays disabled so we can inspect
            // the bytes instead of trusting an inconsistent header.
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .map_err(|error| PidClientError::Send(error.without_url()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(PidClientError::HttpStatus {
                status: status.as_u16(),
                body: error_body_preview(&mut response),
            });
        }

        let body = read_at_most(&mut response, MAX_BODY_BYTES)
            .map_err(PidClientError::ReadBody)?
            .ok_or(PidClientError::ResponseTooLarge {
                limit: MAX_BODY_BYTES,
            })?;
        let decoded = decode_body(&body)?;

        parse_response(&decoded).map_err(PidClientError::Response)
    }
}

fn decode_body(body: &[u8]) -> Result<Vec<u8>, PidClientError> {
    if body.starts_with(&GZIP_MAGIC) {
        read_at_most(GzDecoder::new(body), MAX_BODY_BYTES)
            .map_err(PidClientError::Decompress)?
            .ok_or(PidClientError::ResponseTooLarge {
                limit: MAX_BODY_BYTES,
            })
    } else {
        Ok(body.to_vec())
    }
}

fn error_body_preview(reader: impl Read) -> String {
    let Ok((raw, raw_truncated)) = read_prefix(reader, MAX_ERROR_BODY_BYTES) else {
        return "unreadable response body".to_owned();
    };
    let (body, decoded_truncated) = if raw.starts_with(&GZIP_MAGIC) {
        read_prefix(GzDecoder::new(raw.as_slice()), MAX_ERROR_BODY_BYTES).unwrap_or((raw, false))
    } else {
        (raw, false)
    };
    let cleaned = String::from_utf8_lossy(&body)
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut preview = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.is_empty() {
        return "empty response body".to_owned();
    }
    if raw_truncated || decoded_truncated {
        preview.push('…');
    }
    preview
}

fn read_prefix(reader: impl Read, limit: u64) -> Result<(Vec<u8>, bool), std::io::Error> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let truncated = u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit;
    bytes.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok((bytes, truncated))
}

// Kept identical to the CLI cache helper so the domain core remains I/O-free.
fn read_at_most(reader: impl Read, limit: u64) -> Result<Option<Vec<u8>>, std::io::Error> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok((u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= limit).then_some(bytes))
}

#[derive(Debug, Error)]
pub enum PidClientError {
    #[error("invalid PID endpoint: {0}")]
    InvalidEndpoint(url::ParseError),
    #[error("could not build HTTP client: {0}")]
    BuildClient(reqwest::Error),
    #[error("PID request failed: {0}")]
    Send(#[source] reqwest::Error),
    #[error("could not read PID response: {0}")]
    ReadBody(std::io::Error),
    #[error("PID response exceeded the {limit}-byte body limit")]
    ResponseTooLarge { limit: u64 },
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
    use std::sync::mpsc::{self, Receiver};
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
    fn rejects_oversized_raw_and_decompressed_bodies() {
        let oversized = vec![b' '; usize::try_from(MAX_BODY_BYTES).unwrap() + 1];
        assert!(
            read_at_most(oversized.as_slice(), MAX_BODY_BYTES)
                .unwrap()
                .is_none()
        );

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&oversized).unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(matches!(
            decode_body(&compressed),
            Err(PidClientError::ResponseTooLarge {
                limit: MAX_BODY_BYTES
            })
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
    fn requests_identify_pidjezdy_with_a_versioned_user_agent() {
        let (endpoint, request_bytes) = serve_once_and_capture(200, &[], DEPARTURES);
        let client = PidClient::with_endpoint(&endpoint).unwrap();
        let request = DepartureBoardRequest::new(120, 20, vec![vec!["U100Z1P".into()]]).unwrap();

        client.fetch(&request).unwrap();

        let request = String::from_utf8(request_bytes.recv().unwrap()).unwrap();
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("user-agent: {USER_AGENT}").to_ascii_lowercase())
        );
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

        let corrupt_gzip = [GZIP_MAGIC[0], GZIP_MAGIC[1], 0, 1, 2];
        let endpoint = serve_once(503, &[], &corrupt_gzip);
        let client = PidClient::with_endpoint(&endpoint).unwrap();
        assert!(matches!(
            client.fetch(&request),
            Err(PidClientError::HttpStatus { status: 503, .. })
        ));

        let oversized = vec![b'x'; usize::try_from(MAX_ERROR_BODY_BYTES).unwrap() + 1];
        let endpoint = serve_once(503, &[], &oversized);
        let client = PidClient::with_endpoint(&endpoint).unwrap();
        let error = client.fetch(&request).unwrap_err();
        assert!(matches!(
            &error,
            PidClientError::HttpStatus { status: 503, .. }
        ));
        assert!(error.to_string().ends_with('…'));
    }

    #[test]
    fn transport_errors_do_not_expose_the_request_url() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/data.php", listener.local_addr().unwrap());
        drop(listener);
        let client = PidClient::with_endpoint(&endpoint).unwrap();
        let request =
            DepartureBoardRequest::new(120, 20, vec![vec!["PRIVATE_STOP_ID".into()]]).unwrap();

        let error = client.fetch(&request).unwrap_err();
        assert!(matches!(&error, PidClientError::Send(_)));
        let mut chain = Some(&error as &(dyn std::error::Error + 'static));
        let mut depth = 0;
        while let Some(cause) = chain {
            let message = cause.to_string();
            assert!(!message.contains("PRIVATE_STOP_ID"), "{message}");
            assert!(!message.contains(&endpoint), "{message}");
            depth += 1;
            chain = cause.source();
        }
        assert!(
            depth > 1,
            "the transport source chain must remain available"
        );
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
        serve_once_and_capture(status, headers, body).0
    }

    fn serve_once_and_capture(
        status: u16,
        headers: &[&[u8]],
        body: &[u8],
    ) -> (String, Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, request_receiver) = mpsc::channel();
        let headers = headers
            .iter()
            .map(|header| header.to_vec())
            .collect::<Vec<_>>();
        let body = body.to_vec();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let request_length = stream.read(&mut request).unwrap();
            let _ = request_sender.send(request[..request_length].to_vec());
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
        (format!("http://{address}/data.php"), request_receiver)
    }
}
