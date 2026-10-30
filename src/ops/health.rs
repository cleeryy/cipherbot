//! Tiny HTTP/1.1 health + metrics server (no axum, no extra dependency).
//!
//! Routes: `/healthz` (JSON), `/metrics` (Prometheus text). Anything else 404.
//! A failed bind only logs a warning: the bot must keep running.

use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::bot::SharedState;

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Start the health server, or `None` when disabled / unbindable.
pub fn spawn(state: SharedState) -> Option<JoinHandle<()>> {
    if !state.config.health.enabled {
        tracing::info!("Health server disabled by configuration");
        return None;
    }

    let addr = state.config.health.addr.clone();
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!("Health server could not bind {addr}: {error}");
            return None;
        }
    };

    if let Err(error) = listener.set_nonblocking(true) {
        tracing::warn!("Health server could not configure socket: {error}");
        return None;
    }

    let listener = match TcpListener::from_std(listener) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!("Health server could not adopt socket: {error}");
            return None;
        }
    };

    tracing::info!("Health server listening on http://{addr}/healthz");
    Some(tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _peer)) => {
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(error) = serve_connection(state, stream).await {
                            tracing::debug!("Health connection error: {error}");
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!("Health accept failed: {error}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }))
}

async fn serve_connection(state: SharedState, mut stream: TcpStream) -> std::io::Result<()> {
    let request = match tokio::time::timeout(READ_TIMEOUT, read_request(&mut stream)).await {
        Ok(Ok(request)) => request,
        Ok(Err(error)) => {
            let _ = write_response(&mut stream, 400, "text/plain", "bad request").await;
            return Err(error);
        }
        Err(_) => {
            let _ = write_response(&mut stream, 408, "text/plain", "timeout").await;
            return Ok(());
        }
    };

    let (method, path) = parse_request_line(&request);
    let (status, content_type, body) = if method != "GET" {
        (405, "text/plain", "method not allowed".to_owned())
    } else {
        let uptime = state.started_at.elapsed().as_secs();
        let db_ok = state.db.health_check();
        respond_for(path, uptime, &state.metrics, db_ok)
    };

    write_response(&mut stream, status, content_type, &body).await
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if buffer.len() >= MAX_REQUEST_BYTES {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn parse_request_line(request: &str) -> (&str, &str) {
    let first_line = request.lines().next().unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/");
    (method, path)
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        _ => "Not Found",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

/// Pure routing function, unit-testable without a socket.
///
/// Returns `(status, content_type, body)`.
pub fn respond_for(
    path: &str,
    uptime_secs: u64,
    metrics: &crate::ops::metrics::Metrics,
    db_ok: bool,
) -> (u16, &'static str, String) {
    match path {
        "/healthz" => {
            let status = if db_ok { "ok" } else { "degraded" };
            let body = serde_json::json!({
                "status": status,
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_s": uptime_secs,
                "guilds": metrics.guilds(),
                "db_ok": db_ok,
            })
            .to_string();
            (200, "application/json", body)
        }
        "/metrics" => (
            200,
            "text/plain; version=0.0.4",
            metrics.render_prometheus(uptime_secs, db_ok),
        ),
        _ => (404, "text/plain", "not found".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthz_returns_json() {
        let metrics = crate::ops::metrics::Metrics::default();
        metrics.set_guilds(4);
        let (status, content_type, body) = respond_for("/healthz", 12, &metrics, true);
        assert_eq!(status, 200);
        assert_eq!(content_type, "application/json");
        assert!(body.contains("\"status\":\"ok\""));
        assert!(body.contains("\"uptime_s\":12"));
        assert!(body.contains("\"guilds\":4"));
        assert!(body.contains("\"db_ok\":true"));
    }

    #[test]
    fn healthz_reports_degraded() {
        let metrics = crate::ops::metrics::Metrics::default();
        let (_, _, body) = respond_for("/healthz", 0, &metrics, false);
        assert!(body.contains("\"status\":\"degraded\""));
        assert!(body.contains("\"db_ok\":false"));
    }

    #[test]
    fn metrics_path_returns_prometheus() {
        let metrics = crate::ops::metrics::Metrics::default();
        metrics.inc_commands();
        let (status, content_type, body) = respond_for("/metrics", 5, &metrics, true);
        assert_eq!(status, 200);
        assert!(content_type.starts_with("text/plain"));
        assert!(body.contains("cipherbot_uptime_seconds 5"));
        assert!(body.contains("cipherbot_commands_total 1"));
    }

    #[test]
    fn unknown_path_is_404() {
        let metrics = crate::ops::metrics::Metrics::default();
        let (status, _, body) = respond_for("/nope", 0, &metrics, true);
        assert_eq!(status, 404);
        assert_eq!(body, "not found");
    }

    #[test]
    fn parses_request_line() {
        let (method, path) = parse_request_line("GET /healthz?x=1 HTTP/1.1\r\nHost: a\r\n\r\n");
        assert_eq!(method, "GET");
        assert_eq!(path, "/healthz");

        let (method, path) = parse_request_line("");
        assert_eq!(method, "");
        assert_eq!(path, "/");
    }
}
