//! HTTP(S) announces. Built on hyper so that only `Host` and the profile's
//! headers are sent, in that order (reqwest always adds `Accept: */*`).

use std::io::Read;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Limited};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use thiserror::Error;

use crate::bencode::{self, Value};

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnounceResponse {
    pub interval: u32,
    /// `complete - 1`: we do not count ourselves.
    pub seeders: u32,
    pub leechers: u32,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum AnnounceError {
    #[error("refusé par le tracker : {0}")]
    Failure(String),
    #[error("erreur réseau : {0}")]
    Network(String),
    #[error("le tracker a répondu HTTP {0}")]
    Status(u16),
    #[error("réponse du tracker invalide : {0}")]
    Invalid(String),
    #[error("le tracker n'a pas répondu à temps")]
    Timeout,
}

pub fn announce_url(tracker: &str, query: &str) -> String {
    let separator = if tracker.contains('?') { '&' } else { '?' };
    format!("{tracker}{separator}{query}")
}

pub fn parse_response(body: &[u8]) -> Result<AnnounceResponse, AnnounceError> {
    let root = bencode::decode(body).map_err(|e| AnnounceError::Invalid(e.to_string()))?;
    if let Some(reason) = root.get("failure reason").and_then(Value::as_bytes) {
        return Err(AnnounceError::Failure(String::from_utf8_lossy(reason).into_owned()));
    }
    let interval = root
        .get("interval")
        .and_then(Value::as_int)
        .filter(|&i| i > 0)
        .ok_or_else(|| AnnounceError::Invalid("intervalle absent".into()))?;
    let count = |key: &str| {
        let n = root.get(key).and_then(Value::as_int).unwrap_or(0).clamp(0, i64::from(u32::MAX));
        u32::try_from(n).unwrap_or(u32::MAX)
    };
    Ok(AnnounceResponse {
        interval: u32::try_from(interval).unwrap_or(u32::MAX),
        seeders: count("complete").saturating_sub(1),
        leechers: count("incomplete"),
    })
}

#[derive(Clone)]
pub struct HttpClient {
    client: Client<HttpsConnector<HttpConnector>, Empty<Bytes>>,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    pub fn new() -> Self {
        // Ignore the error: a provider may already be installed by another client.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        let client = Client::builder(TokioExecutor::new()).http1_title_case_headers(true).build(https);
        Self { client }
    }

    pub async fn announce(&self, url: &str, headers: &[(String, String)]) -> Result<AnnounceResponse, AnnounceError> {
        let body = tokio::time::timeout(TIMEOUT, self.fetch(url, headers)).await.map_err(|_| AnnounceError::Timeout)??;
        parse_response(&body)
    }

    async fn fetch(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>, AnnounceError> {
        let uri: http::Uri = url.parse().map_err(|e| AnnounceError::Invalid(format!("URL invalide : {e}")))?;
        let host = uri
            .authority()
            .map(|a| a.as_str().to_owned())
            .ok_or_else(|| AnnounceError::Invalid("URL sans hôte".into()))?;
        let mut request = http::Request::get(uri).header(http::header::HOST, host);
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let request = request.body(Empty::new()).map_err(|e| AnnounceError::Invalid(e.to_string()))?;
        let response = self.client.request(request).await.map_err(|e| AnnounceError::Network(error_chain(&e)))?;
        let status = response.status();
        let gzip = response
            .headers()
            .get(http::header::CONTENT_ENCODING)
            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"gzip"));
        let body = Limited::new(response.into_body(), MAX_BODY)
            .collect()
            .await
            .map_err(|e| AnnounceError::Network(e.to_string()))?
            .to_bytes();
        let body = if gzip { gunzip(&body)? } else { body.to_vec() };
        if !status.is_success() {
            return Err(match parse_response(&body) {
                Err(failure @ AnnounceError::Failure(_)) => failure,
                _ => AnnounceError::Status(status.as_u16()),
            });
        }
        Ok(body)
    }
}

fn gunzip(data: &[u8]) -> Result<Vec<u8>, AnnounceError> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data)
        .take(MAX_BODY as u64)
        .read_to_end(&mut out)
        .map_err(|e| AnnounceError::Invalid(format!("gzip invalide : {e}")))?;
    Ok(out)
}

fn error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(" : ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tracker_response;
    use std::io::Write;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    /// Serves one raw HTTP response; yields the base URL and the raw request received.
    async fn serve_once(head: &str, body: Vec<u8>) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut response =
            format!("{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        response.extend(body);
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            let _ = tx.send(String::from_utf8_lossy(&request).into_owned());
            socket.write_all(&response).await.unwrap();
        });
        (base, rx)
    }

    fn qbit_headers() -> Vec<(String, String)> {
        vec![
            ("User-Agent".into(), "qBittorrent/5.2.4".into()),
            ("Accept-Encoding".into(), "gzip".into()),
            ("Connection".into(), "close".into()),
        ]
    }

    #[tokio::test]
    async fn sends_exactly_host_then_profile_headers_and_raw_query() {
        let (base, request) = serve_once("HTTP/1.1 200 OK", tracker_response(1800, 5, 3)).await;
        let url = announce_url(&format!("{base}/announce"), "info_hash=%12%ab&peer_id=-qB(x)!*~");
        let response = HttpClient::new().announce(&url, &qbit_headers()).await.unwrap();
        assert_eq!(response, AnnounceResponse { interval: 1800, seeders: 4, leechers: 3 });
        let host = base.trim_start_matches("http://");
        assert_eq!(
            request.await.unwrap(),
            format!(
                "GET /announce?info_hash=%12%ab&peer_id=-qB(x)!*~ HTTP/1.1\r\nHost: {host}\r\nUser-Agent: qBittorrent/5.2.4\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
            )
        );
    }

    #[tokio::test]
    async fn gzip_body_is_decoded() {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tracker_response(900, 1, 2)).unwrap();
        let (base, _) = serve_once("HTTP/1.1 200 OK\r\nContent-Encoding: gzip", gz.finish().unwrap()).await;
        let r = HttpClient::new().announce(&format!("{base}/a"), &[]).await.unwrap();
        assert_eq!(r, AnnounceResponse { interval: 900, seeders: 0, leechers: 2 });
    }

    #[tokio::test]
    async fn failure_reason_is_reported() {
        let (base, _) = serve_once("HTTP/1.1 200 OK", b"d14:failure reason17:torrent not founde".to_vec()).await;
        let err = HttpClient::new().announce(&format!("{base}/a"), &[]).await.unwrap_err();
        assert_eq!(err, AnnounceError::Failure("torrent not found".into()));
    }

    #[tokio::test]
    async fn html_body_is_invalid() {
        let (base, _) = serve_once("HTTP/1.1 200 OK", b"<html>blocked</html>".to_vec()).await;
        let err = HttpClient::new().announce(&format!("{base}/a"), &[]).await.unwrap_err();
        assert!(matches!(err, AnnounceError::Invalid(_)), "{err:?}");
    }

    #[tokio::test]
    async fn error_status_is_reported_unless_body_has_a_failure_reason() {
        let (base, _) = serve_once("HTTP/1.1 503 Service Unavailable", b"busy".to_vec()).await;
        assert_eq!(HttpClient::new().announce(&format!("{base}/a"), &[]).await, Err(AnnounceError::Status(503)));
        let (base, _) = serve_once("HTTP/1.1 400 Bad Request", b"d14:failure reason4:nopee".to_vec()).await;
        assert_eq!(
            HttpClient::new().announce(&format!("{base}/a"), &[]).await,
            Err(AnnounceError::Failure("nope".into()))
        );
    }

    #[tokio::test]
    async fn refused_connection_is_a_network_error() {
        let port = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let err = HttpClient::new().announce(&format!("http://127.0.0.1:{port}/a"), &[]).await.unwrap_err();
        assert!(matches!(err, AnnounceError::Network(_)), "{err:?}");
    }

    #[test]
    fn parse_response_edge_cases() {
        assert_eq!(parse_response(&tracker_response(60, 0, 0)).unwrap().seeders, 0);
        assert!(matches!(parse_response(b"d8:completei1ee"), Err(AnnounceError::Invalid(_))));
        assert!(matches!(parse_response(b"d8:intervali0ee"), Err(AnnounceError::Invalid(_))));
        assert!(matches!(parse_response(b"i5e"), Err(AnnounceError::Invalid(_))));
    }

    #[test]
    fn announce_url_appends_with_the_right_separator() {
        assert_eq!(announce_url("http://t/announce", "a=1"), "http://t/announce?a=1");
        assert_eq!(announce_url("http://t/announce.php?passkey=x", "a=1"), "http://t/announce.php?passkey=x&a=1");
    }
}
