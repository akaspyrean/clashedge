//! Minimal HTTP/1.1 client codec over a Windows named pipe.
//!
//! Mihomo creates the named-pipe server when configured with
//! `external-controller-pipe`; this module only connects as a client. Mihomo
//! does not authenticate API requests received through that pipe, so callers
//! must not treat an Authorization header as pipe access control.

use std::fmt;
use std::io;

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::{StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const PIPE_BUSY_ERROR: Option<i32> = Some(231); // Win32 ERROR_PIPE_BUSY

/// HTTP response returned by [`request`]. Header names are lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PipeHttpResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl PipeHttpResponse {
    pub(crate) fn status(&self) -> StatusCode {
        // The codec only admits valid HTTP status codes.
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::BAD_GATEWAY)
    }

    pub(crate) async fn json<T: DeserializeOwned>(self) -> Result<T, PipeHttpError> {
        serde_json::from_slice(&self.body).map_err(PipeHttpError::Json)
    }

    /// Decode the body as text; used by codec tests only for now.
    #[cfg(test)]
    pub(crate) async fn text(self) -> Result<String, PipeHttpError> {
        Ok(String::from_utf8_lossy(&self.body).into_owned())
    }
}

/// Errors are intentionally small and contain no request credentials or body.
#[derive(Debug)]
pub(crate) enum PipeHttpError {
    Io(io::Error),
    InvalidRequest(&'static str),
    Protocol(&'static str),
    Url(String),
    Json(serde_json::Error),
    InvalidHeader,
    Timeout,
}

impl fmt::Display for PipeHttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "named-pipe HTTP I/O failed: {e}"),
            Self::InvalidRequest(message) => write!(f, "invalid named-pipe HTTP request: {message}"),
            Self::Protocol(message) => write!(f, "invalid named-pipe HTTP response: {message}"),
            Self::Url(message) => write!(f, "invalid named-pipe HTTP URL: {message}"),
            Self::Json(error) => write!(f, "named-pipe HTTP JSON error: {error}"),
            Self::InvalidHeader => f.write_str("named-pipe HTTP header cannot be represented as text"),
            Self::Timeout => f.write_str("named-pipe HTTP request timed out"),
        }
    }
}

impl std::error::Error for PipeHttpError {}

impl From<io::Error> for PipeHttpError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<PipeHttpError> for crate::util::error::Error {
    fn from(error: PipeHttpError) -> Self {
        Self::Other(error.to_string())
    }
}

/// Small reqwest-shaped facade for the controller's existing call pattern.
#[derive(Clone)]
pub(crate) struct PipeHttpClient {
    pipe_name: std::sync::Arc<dyn Fn() -> String + Send + Sync>,
}

impl PipeHttpClient {
    /// Fixed-name client; production code uses [`Self::new_dynamic`] so a rotated
    /// secret is picked up without rebuilding the client.
    #[cfg(test)]
    pub(crate) fn new(pipe_name: impl Into<String>) -> Self {
        let pipe_name = pipe_name.into();
        Self::new_dynamic(move || pipe_name.clone())
    }

    pub(crate) fn new_dynamic<F>(pipe_name: F) -> Self
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        Self { pipe_name: std::sync::Arc::new(pipe_name) }
    }

    pub(crate) fn get<U: IntoPipeUrl>(&self, url: U) -> PipeRequestBuilder {
        self.request("GET", url)
    }

    pub(crate) fn patch<U: IntoPipeUrl>(&self, url: U) -> PipeRequestBuilder {
        self.request("PATCH", url)
    }

    pub(crate) fn put<U: IntoPipeUrl>(&self, url: U) -> PipeRequestBuilder {
        self.request("PUT", url)
    }

    pub(crate) fn delete<U: IntoPipeUrl>(&self, url: U) -> PipeRequestBuilder {
        self.request("DELETE", url)
    }

    fn request<U: IntoPipeUrl>(&self, method: &str, url: U) -> PipeRequestBuilder {
        PipeRequestBuilder {
            client: self.clone(),
            method: method.to_string(),
            url: url.into_pipe_url(),
            headers: HeaderMap::new(),
            body: Vec::new(),
            error: None,
        }
    }
}

/// URL input accepted by `PipeHttpClient`: parsed reqwest URLs and string forms.
pub(crate) trait IntoPipeUrl {
    fn into_pipe_url(self) -> Result<Url, PipeHttpError>;
}

impl IntoPipeUrl for Url {
    fn into_pipe_url(self) -> Result<Url, PipeHttpError> {
        validate_pipe_url(self)
    }
}

impl IntoPipeUrl for &Url {
    fn into_pipe_url(self) -> Result<Url, PipeHttpError> {
        validate_pipe_url(self.clone())
    }
}

impl IntoPipeUrl for &str {
    fn into_pipe_url(self) -> Result<Url, PipeHttpError> {
        let url = Url::parse(self).map_err(|error| PipeHttpError::Url(error.to_string()))?;
        validate_pipe_url(url)
    }
}

impl IntoPipeUrl for String {
    fn into_pipe_url(self) -> Result<Url, PipeHttpError> {
        self.as_str().into_pipe_url()
    }
}

impl IntoPipeUrl for &String {
    fn into_pipe_url(self) -> Result<Url, PipeHttpError> {
        self.as_str().into_pipe_url()
    }
}

fn validate_pipe_url(url: Url) -> Result<Url, PipeHttpError> {
    if url.scheme() != "http" {
        return Err(PipeHttpError::InvalidRequest("named-pipe controller URLs must use http"));
    }
    Ok(url)
}

pub(crate) struct PipeRequestBuilder {
    client: PipeHttpClient,
    method: String,
    url: Result<Url, PipeHttpError>,
    headers: HeaderMap,
    body: Vec<u8>,
    error: Option<PipeHttpError>,
}

impl PipeRequestBuilder {
    pub(crate) fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    pub(crate) fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        if !self.headers.contains_key(CONTENT_TYPE) {
            self.headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }
        match serde_json::to_vec(value) {
            Ok(body) => self.body = body,
            Err(error) => self.error = Some(PipeHttpError::Json(error)),
        }
        self
    }

    pub(crate) async fn send(self) -> Result<PipeHttpResponse, PipeHttpError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let url = self.url?;
        let mut path = url.path().to_string();
        if path.is_empty() {
            path.push('/');
        }
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        let headers = self.headers.iter().map(|(name, value)| {
            let value = value.to_str().map_err(|_| PipeHttpError::InvalidHeader)?;
            Ok((name.as_str().to_string(), value.to_string()))
        }).collect::<Result<Vec<_>, PipeHttpError>>()?;
        let pipe_name = (self.client.pipe_name)();
        let operation = request(&pipe_name, &self.method, &path, &headers, &self.body);
        tokio::time::timeout(std::time::Duration::from_secs(10), operation)
            .await
            .map_err(|_| PipeHttpError::Timeout)?
    }
}

/// Send one HTTP/1.1 request to Mihomo's named-pipe API and read its response.
///
/// Each call opens a fresh pipe client and sends `Connection: close`; callers
/// can safely issue requests concurrently without sharing HTTP framing state.
/// `headers` are additional end-to-end headers such as Authorization. Framing
/// and hop-by-hop headers are generated here and must not be supplied by caller.
#[cfg(windows)]
pub(crate) async fn request(
    pipe_name: &str,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<PipeHttpResponse, PipeHttpError> {
    let request = encode_request(method, path, headers, body)?;
    let mut pipe = connect_pipe(pipe_name).await?;
    pipe.write_all(&request).await?;
    pipe.flush().await?;

    let mut reader = BufReader::new(pipe);
    read_response(&mut reader, method).await
}

#[cfg(not(windows))]
pub(crate) async fn request(
    _pipe_name: &str,
    _method: &str,
    _path: &str,
    _headers: &[(String, String)],
    _body: &[u8],
) -> Result<PipeHttpResponse, PipeHttpError> {
    Err(PipeHttpError::InvalidRequest("named-pipe HTTP is only available on Windows"))
}

#[cfg(windows)]
async fn connect_pipe(
    pipe_name: &str,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, PipeHttpError> {
    use tokio::net::windows::named_pipe::ClientOptions;

    if !pipe_name.starts_with(r"\\.\pipe\") || pipe_name.chars().any(|c| matches!(c, '\r' | '\n' | '\0')) {
        return Err(PipeHttpError::InvalidRequest("pipe name must be a local \\\\.\\pipe\\ path"));
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match ClientOptions::new().open(pipe_name) {
            Ok(client) => return Ok(client),
            Err(error) if error.raw_os_error() == PIPE_BUSY_ERROR => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(PipeHttpError::Io(error));
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(error) => return Err(PipeHttpError::Io(error)),
        }
    }
}

fn encode_request(
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<Vec<u8>, PipeHttpError> {
    validate_request(method, path, headers, body.len())?;
    let mut request = Vec::with_capacity(256 + body.len());
    request.extend_from_slice(method.as_bytes());
    request.push(b' ');
    request.extend_from_slice(path.as_bytes());
    request.extend_from_slice(b" HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    request.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    let header_bytes = headers.iter().try_fold(request.len(), |length, (name, value)| {
        length
            .checked_add(name.len())
            .and_then(|length| length.checked_add(value.len()))
            .and_then(|length| length.checked_add(4)) // ": " + CRLF
    }).and_then(|length| length.checked_add(2)); // End of headers
    if header_bytes.is_none_or(|length| length > MAX_HEADER_BYTES) {
        return Err(PipeHttpError::InvalidRequest("request line and headers exceed size limit"));
    }
    for (name, value) in headers {
        request.extend_from_slice(name.as_bytes());
        request.extend_from_slice(b": ");
        request.extend_from_slice(value.as_bytes());
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(body);
    Ok(request)
}

fn validate_request(
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body_len: usize,
) -> Result<(), PipeHttpError> {
    if method.is_empty() || !method.bytes().all(is_token_byte) {
        return Err(PipeHttpError::InvalidRequest("method must be an HTTP token"));
    }
    if !path.starts_with('/') || path.len() > MAX_HEADER_BYTES || path.bytes().any(|b| b <= b' ' || b == b'\x7f') {
        return Err(PipeHttpError::InvalidRequest("path must be a bounded origin-form URL path"));
    }
    if body_len > MAX_BODY_BYTES {
        return Err(PipeHttpError::InvalidRequest("request body exceeds size limit"));
    }
    for (name, value) in headers {
        if name.is_empty() || !name.bytes().all(is_token_byte) {
            return Err(PipeHttpError::InvalidRequest("header name is invalid"));
        }
        if value.bytes().any(|b| (b < b' ' && b != b'\t') || b == b'\x7f') {
            return Err(PipeHttpError::InvalidRequest("header value contains a forbidden byte"));
        }
        if matches!(name.to_ascii_lowercase().as_str(), "host" | "content-length" | "transfer-encoding" | "connection") {
            return Err(PipeHttpError::InvalidRequest("framing headers are managed by the client"));
        }
    }
    Ok(())
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

async fn read_response<R: AsyncRead + Unpin>(
    reader: &mut R,
    request_method: &str,
) -> Result<PipeHttpResponse, PipeHttpError> {
    let mut buffered = BufReader::new(reader);
    let mut interim_count = 0usize;
    loop {
        let mut header_bytes = 0usize;
        let status_line = read_header_line(&mut buffered, &mut header_bytes).await?;
        let status = parse_status_line(&status_line)?;
        let headers = read_headers(&mut buffered, &mut header_bytes).await?;
        // Ignore interim 1xx responses except protocol upgrade, which this REST
        // client does not support.
        if (100..200).contains(&status) {
            if status == 101 {
                return Err(PipeHttpError::Protocol("protocol upgrade is unsupported"));
            }
            interim_count += 1;
            if interim_count > 8 {
                return Err(PipeHttpError::Protocol("too many interim responses"));
            }
            continue;
        }
        let body = if request_method.eq_ignore_ascii_case("HEAD")
            || matches!(status, 204 | 205 | 304)
        {
            Vec::new()
        } else {
            read_body(&mut buffered, &headers).await?
        };
        return Ok(PipeHttpResponse { status, headers, body });
    }
}

async fn read_header_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    total: &mut usize,
) -> Result<String, PipeHttpError> {
    let remaining = MAX_HEADER_BYTES.saturating_sub(*total);
    let mut line = read_bounded_line_bytes(reader, remaining)
        .await?
        .ok_or(PipeHttpError::Protocol("unexpected EOF in headers"))?;
    *total = total.saturating_add(line.len());
    if !line.ends_with(b"\r\n") {
        return Err(PipeHttpError::Protocol("HTTP lines must end with CRLF"));
    }
    line.truncate(line.len() - 2);
    String::from_utf8(line).map_err(|_| PipeHttpError::Protocol("HTTP header line is not valid UTF-8"))
}

async fn read_bounded_line_bytes<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<Option<Vec<u8>>, PipeHttpError> {
    let mut line = Vec::new();
    loop {
        let (consumed, found_newline) = {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                return Ok((!line.is_empty()).then_some(line));
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(available.len(), |index| index + 1);
            if line.len().checked_add(consumed).is_none_or(|length| length > max) {
                return Err(PipeHttpError::Protocol("HTTP line exceeds size limit"));
            }
            line.extend_from_slice(&available[..consumed]);
            (consumed, newline.is_some())
        };
        reader.consume(consumed);
        if found_newline {
            return Ok(Some(line));
        }
    }
}

fn parse_status_line(line: &str) -> Result<u16, PipeHttpError> {
    let mut parts = line.split_ascii_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(version), Some(status), _) if version == "HTTP/1.0" || version == "HTTP/1.1" => {
            if status.len() != 3 || !status.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(PipeHttpError::Protocol("status code must contain three digits"));
            }
            let code = status.parse::<u16>().map_err(|_| PipeHttpError::Protocol("invalid status code"))?;
            if !(100..=599).contains(&code) {
                return Err(PipeHttpError::Protocol("status code is out of range"));
            }
            Ok(code)
        }
        _ => Err(PipeHttpError::Protocol("invalid HTTP status line")),
    }
}

async fn read_headers<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    total: &mut usize,
) -> Result<Vec<(String, String)>, PipeHttpError> {
    let mut headers = Vec::new();
    loop {
        let line = read_header_line(reader, total).await?;
        if line.is_empty() {
            return Ok(headers);
        }
        let (name, value) = line.split_once(':').ok_or(PipeHttpError::Protocol("malformed header"))?;
        if name.is_empty() || !name.bytes().all(is_token_byte) {
            return Err(PipeHttpError::Protocol("invalid header name"));
        }
        if value.bytes().any(|b| (b < b' ' && b != b'\t') || b == b'\x7f') {
            return Err(PipeHttpError::Protocol("invalid header value"));
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }
}

async fn read_body<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    headers: &[(String, String)],
) -> Result<Vec<u8>, PipeHttpError> {
    let transfer_encoding: Vec<&str> = headers
        .iter()
        .filter(|(name, _)| name == "transfer-encoding")
        .flat_map(|(_, value)| value.split(','))
        .map(str::trim)
        .collect();
    if !transfer_encoding.is_empty() {
        if transfer_encoding.len() != 1 || !transfer_encoding[0].eq_ignore_ascii_case("chunked") {
            return Err(PipeHttpError::Protocol("unsupported transfer encoding"));
        }
        return read_chunked(reader).await;
    }

    let lengths: Vec<&str> = headers
        .iter()
        .filter(|(name, _)| name == "content-length")
        .map(|(_, value)| value.as_str())
        .collect();
    if lengths.is_empty() {
        return read_to_eof_bounded(reader, MAX_BODY_BYTES).await;
    }
    let length = lengths[0]
        .parse::<usize>()
        .map_err(|_| PipeHttpError::Protocol("invalid Content-Length"))?;
    if length > MAX_BODY_BYTES || lengths.iter().any(|value| value.parse::<usize>().ok() != Some(length)) {
        return Err(PipeHttpError::Protocol("Content-Length is invalid or exceeds size limit"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await.map_err(|_| PipeHttpError::Protocol("truncated Content-Length body"))?;
    Ok(body)
}

async fn read_to_eof_bounded<R: AsyncBufRead + Unpin>(reader: &mut R, max: usize) -> Result<Vec<u8>, PipeHttpError> {
    let mut body = Vec::new();
    loop {
        let count = {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                return Ok(body);
            }
            let remaining = max.saturating_sub(body.len());
            if available.len() > remaining {
                return Err(PipeHttpError::Protocol("close-delimited body exceeds size limit"));
            }
            body.extend_from_slice(available);
            available.len()
        };
        reader.consume(count);
    }
}

async fn read_chunked<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, PipeHttpError> {
    let mut body = Vec::new();
    loop {
        let line = read_bounded_line(reader, MAX_HEADER_BYTES).await?;
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| PipeHttpError::Protocol("invalid chunk size"))?;
        if size == 0 {
            let mut trailer_bytes = 0usize;
            loop {
                let trailer = read_header_line(reader, &mut trailer_bytes).await?;
                if trailer.is_empty() {
                    return Ok(body);
                }
                if !trailer.contains(':') {
                    return Err(PipeHttpError::Protocol("malformed chunk trailer"));
                }
            }
        }
        if body.len().checked_add(size).is_none_or(|n| n > MAX_BODY_BYTES) {
            return Err(PipeHttpError::Protocol("chunked body exceeds size limit"));
        }
        let old_len = body.len();
        body.resize(old_len + size, 0);
        reader.read_exact(&mut body[old_len..]).await.map_err(|_| PipeHttpError::Protocol("truncated chunk"))?;
        let mut crlf = [0; 2];
        reader.read_exact(&mut crlf).await.map_err(|_| PipeHttpError::Protocol("missing chunk terminator"))?;
        if crlf != *b"\r\n" {
            return Err(PipeHttpError::Protocol("invalid chunk terminator"));
        }
    }
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<String, PipeHttpError> {
    let mut line = read_bounded_line_bytes(reader, max)
        .await?
        .ok_or(PipeHttpError::Protocol("unexpected EOF in HTTP line"))?;
    if !line.ends_with(b"\r\n") {
        return Err(PipeHttpError::Protocol("HTTP lines must end with CRLF"));
    }
    line.truncate(line.len() - 2);
    String::from_utf8(line).map_err(|_| PipeHttpError::Protocol("HTTP line is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn dynamic_pipe_name_is_shared_and_resolved_on_demand() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let dynamic_calls = calls.clone();
        let client = PipeHttpClient::new_dynamic(move || {
            let call = dynamic_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            format!(r"\\.\pipe\mihomo-{call}")
        });
        let clone = client.clone();
        assert_eq!((client.pipe_name)(), r"\\.\pipe\mihomo-0");
        assert_eq!((clone.pipe_name)(), r"\\.\pipe\mihomo-1");
    }

    #[test]
    fn wrapper_accepts_url_and_string_and_builds_json_request() {
        let client = PipeHttpClient::new(r"\\.\pipe\mihomo");
        let parsed = Url::parse("http://127.0.0.1:9090/configs?force=true").unwrap();
        let builder = client
            .patch(&parsed)
            .headers(HeaderMap::new())
            .json(&serde_json::json!({ "mode": "rule" }));
        let url = builder.url.as_ref().unwrap();
        assert_eq!(url.path(), "/configs");
        assert_eq!(url.query(), Some("force=true"));
        assert_eq!(builder.method, "PATCH");
        assert_eq!(builder.headers.get(CONTENT_TYPE).unwrap(), "application/json");
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&builder.body).unwrap()["mode"], "rule");

        let get = client.get("http://127.0.0.1:9090/version");
        assert_eq!(get.method, "GET");
        assert_eq!(get.url.unwrap().path(), "/version");
    }

    #[tokio::test]
    async fn response_wrapper_exposes_status_json_and_text() {
        let json = PipeHttpResponse {
            status: 200,
            headers: Vec::new(),
            body: br#"{"version":"test"}"#.to_vec(),
        };
        assert!(json.status().is_success());
        let value: serde_json::Value = json.json().await.unwrap();
        assert_eq!(value["version"], "test");

        let text = PipeHttpResponse { status: 404, headers: Vec::new(), body: b"missing".to_vec() };
        assert_eq!(text.status(), StatusCode::NOT_FOUND);
        assert_eq!(text.text().await.unwrap(), "missing");
    }

    #[test]
    fn encodes_request_line_headers_and_body() {
        let request = encode_request(
            "PATCH",
            "/configs?force=true",
            &[("Authorization".into(), "Bearer test".into()), ("Content-Type".into(), "application/json".into())],
            br#"{"mode":"rule"}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            b"PATCH /configs?force=true HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 15\r\nAuthorization: Bearer test\r\nContent-Type: application/json\r\n\r\n{\"mode\":\"rule\"}"
        );
    }

    #[test]
    fn request_line_and_all_headers_share_one_size_limit() {
        let long_path = format!("/{}", "a".repeat(MAX_HEADER_BYTES - 1));
        assert!(validate_request("GET", &long_path, &[], 0).is_ok());
        assert!(encode_request("GET", &long_path, &[], b"").is_err());

        let large_header = vec![("X-Large".to_string(), "v".repeat(MAX_HEADER_BYTES))];
        assert!(encode_request("GET", "/", &large_header, b"").is_err());
    }

    #[test]
    fn validates_request_framing_and_header_injection() {
        assert!(validate_request("PATCH", "/configs", &[], 3).is_ok());
        assert!(validate_request("GET\r\nX: y", "/version", &[], 0).is_err());
        assert!(validate_request("GET", "http://localhost/", &[], 0).is_err());
        assert!(validate_request("GET", "/", &[("Content-Length".into(), "9".into())], 0).is_err());
        assert!(validate_request("GET", "/", &[("X-Test".into(), "ok\r\nInjected: yes".into())], 0).is_err());
    }

    async fn parse(raw: &[u8], method: &str) -> Result<PipeHttpResponse, PipeHttpError> {
        let (mut writer, mut reader) = tokio::io::duplex(128 * 1024);
        writer.write_all(raw).await.unwrap();
        drop(writer);
        read_response(&mut reader, method).await
    }

    #[tokio::test]
    async fn decodes_content_length_and_interim_response() {
        let response = parse(
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: application/json\r\n\r\nhello",
            "GET",
        )
        .await
        .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        assert!(response.headers.iter().any(|(name, value)| name == "content-type" && value == "application/json"));
    }

    #[tokio::test]
    async fn decodes_chunked_response_with_extensions_and_trailers() {
        let response = parse(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;foo=bar\r\ntest\r\n3\r\ning\r\n0\r\nX-Checksum: ok\r\n\r\n",
            "GET",
        )
        .await
        .unwrap();
        assert_eq!(response.body, b"testing");
    }

    #[tokio::test]
    async fn reads_close_delimited_response_until_eof() {
        let response = parse(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nbody without framing", "GET")
            .await
            .unwrap();
        assert_eq!(response.body, b"body without framing");
    }

    #[tokio::test]
    async fn bounds_close_delimited_body_and_http_lines() {
        let (mut writer, mut reader) = tokio::io::duplex(16);
        writer.write_all(b"12345").await.unwrap();
        drop(writer);
        assert!(read_to_eof_bounded(&mut reader, 4).await.is_err());

        let (mut writer, mut reader) = tokio::io::duplex(16);
        writer.write_all(b"12345\r\n").await.unwrap();
        drop(writer);
        assert!(read_bounded_line_bytes(&mut reader, 4).await.is_err());
    }

    #[tokio::test]
    async fn rejects_truncated_and_oversized_header_responses() {
        assert!(parse(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nxy", "GET").await.is_err());
        assert!(parse(b"HTTP/1.1 101 Switching Protocols\r\n\r\n", "GET").await.is_err());

        let oversized_status = vec![b'x'; MAX_HEADER_BYTES + 1];
        assert!(parse(&oversized_status, "GET").await.is_err());
    }

    #[tokio::test]
    async fn rejects_non_three_digit_status_and_handles_reset_content() {
        assert!(parse(b"HTTP/1.1 0200 OK\r\nContent-Length: 0\r\n\r\n", "GET")
            .await
            .is_err());
        let response = parse(b"HTTP/1.1 205 Reset Content\r\n\r\n", "GET")
            .await
            .unwrap();
        assert!(response.body.is_empty());
    }

    #[tokio::test]
    async fn head_and_no_content_responses_have_no_body() {
        let response = parse(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n", "HEAD")
            .await
            .unwrap();
        assert!(response.body.is_empty());
        let response = parse(b"HTTP/1.1 204 No Content\r\n\r\n", "GET")
            .await
            .unwrap();
        assert!(response.body.is_empty());
    }
}
