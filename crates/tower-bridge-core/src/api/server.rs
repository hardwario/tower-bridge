//! HTTP server lifecycle: runs on its own thread with a Tokio runtime and
//! stops gracefully, so it can be restarted when settings change.

use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};
use tracing::{debug, info, warn};

use super::ApiBackend;
use super::mcp::McpServer;
use super::rest::{Ctx, router};

pub struct ApiServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl ApiServer {
    /// Bind `addr` and start serving. Bind errors are returned immediately.
    pub fn start(addr: SocketAddr, backend: Arc<dyn ApiBackend>) -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let local = listener.local_addr()?;
        let ctx = Ctx {
            mcp: Arc::new(McpServer::new(backend.clone())),
            backend,
            lan: !local.ip().is_loopback(),
        };
        let app = router(ctx);
        let (tx, rx) = oneshot::channel::<()>();
        let join = std::thread::Builder::new()
            .name("api-server".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        warn!("API server runtime failed: {e}");
                        return;
                    }
                };
                runtime.block_on(async move {
                    let listener = match tokio::net::TcpListener::from_std(listener) {
                        Ok(l) => Guarded {
                            listener: l,
                            slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
                        },
                        Err(e) => {
                            warn!("API server listener failed: {e}");
                            return;
                        }
                    };
                    let shutdown = async {
                        let _ = rx.await;
                    };
                    if let Err(e) = axum::serve(listener, app)
                        .with_graceful_shutdown(shutdown)
                        .await
                    {
                        warn!("API server stopped: {e}");
                    }
                });
            })?;
        info!("API server listening on http://{local}");
        Ok(Self {
            addr: local,
            shutdown: Some(tx),
            join: Some(join),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop accepting connections and wait (bounded) for the thread to end.
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !join.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if join.is_finished() {
                let _ = join.join();
            }
            info!("API server on {} stopped", self.addr);
        }
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Connections served at once; more wait in the kernel's backlog instead of
/// taking file descriptors the rest of the app needs (the database, the
/// bridge, MQTT).
const MAX_CONNECTIONS: usize = 64;
/// A connection without traffic either way for this long is closed: an idle
/// keep-alive, or headers trickling in. Hyper keeps reading while a request
/// is answered, so this must be longer than any answer takes.
const IDLE_TIMEOUT: Duration = Duration::from_secs(if cfg!(test) { 3 } else { 30 });

/// The listener with a cap on connections and an idle timeout on each.
struct Guarded {
    listener: tokio::net::TcpListener,
    slots: Arc<Semaphore>,
}

impl axum::serve::Listener for Guarded {
    type Io = GuardedStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let Ok(permit) = self.slots.clone().acquire_owned().await else {
                // Never closed; wait as for a failed accept.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            match self.listener.accept().await {
                Ok((stream, addr)) => {
                    let idle = Box::pin(tokio::time::sleep(IDLE_TIMEOUT));
                    return (
                        GuardedStream {
                            stream,
                            idle,
                            _permit: permit,
                        },
                        addr,
                    );
                }
                Err(e) => {
                    // Out of file descriptors and the like: wait, retry.
                    debug!("API server accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

/// A connection holding its slot; reading fails once it was idle too long.
struct GuardedStream {
    stream: tokio::net::TcpStream,
    idle: Pin<Box<tokio::time::Sleep>>,
    _permit: OwnedSemaphorePermit,
}

impl AsyncRead for GuardedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.stream).poll_read(cx, buf) {
            Poll::Ready(r) => {
                let deadline = tokio::time::Instant::now() + IDLE_TIMEOUT;
                this.idle.as_mut().reset(deadline);
                Poll::Ready(r)
            }
            Poll::Pending => match this.idle.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Err(std::io::ErrorKind::TimedOut.into())),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

impl AsyncWrite for GuardedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = &mut *self;
        let r = Pin::new(&mut this.stream).poll_write(cx, buf);
        if matches!(r, Poll::Ready(Ok(n)) if n > 0) {
            // A slow client downloading a large answer is not idle.
            let deadline = tokio::time::Instant::now() + IDLE_TIMEOUT;
            this.idle.as_mut().reset(deadline);
        }
        r
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::*;
    use crate::api::Policy;
    use crate::api::http_client::{HttpResponse, request};
    use crate::api::testing::FakeBackend;

    fn start(policy: Policy) -> Result<(ApiServer, Arc<FakeBackend>), std::io::Error> {
        let backend = Arc::new(FakeBackend::new(policy));
        let server = ApiServer::start(
            "127.0.0.1:0".parse().map_err(std::io::Error::other)?,
            backend.clone(),
        )?;
        Ok((server, backend))
    }

    fn get(
        server: &ApiServer,
        path: &str,
        headers: &[(&str, &str)],
    ) -> std::io::Result<HttpResponse> {
        request(
            server.local_addr(),
            "GET",
            path,
            headers,
            b"",
            Duration::from_secs(5),
        )
    }

    fn json_body(r: &HttpResponse) -> Value {
        serde_json::from_slice(&r.body).unwrap_or(Value::Null)
    }

    fn open_policy() -> Policy {
        Policy {
            token: "tok".into(),
            allow_control: false,
            mcp: true,
        }
    }

    #[test]
    fn rest_read_endpoints() -> std::io::Result<()> {
        let (server, _) = start(open_policy())?;
        let r = get(&server, "/api/v1/readings/latest", &[])?;
        assert_eq!(r.status, 200);
        let v = json_body(&r);
        assert_eq!(v["readings"][0]["temperature"], 23.456);
        assert_eq!(v["readings"][0]["channel"], 0);
        assert_eq!(v["stale"], false);

        let r = get(&server, "/api/v1/readings?hours=2&max_points=10", &[])?;
        assert_eq!(r.status, 200);
        assert_eq!(json_body(&r)["bucket_ms"], 60_000);

        let r = get(
            &server,
            "/api/v1/readings?from=2026-10-17T10:00:00Z&to=2026-10-17T09:00:00Z",
            &[],
        )?;
        assert_eq!(r.status, 400);

        let r = get(&server, "/api/v1/readings.csv?hours=1", &[])?;
        assert_eq!(r.status, 200);
        assert!(
            r.header("content-type")
                .is_some_and(|c| c.starts_with("text/csv"))
        );
        assert!(r.body.starts_with(b"timestamp,unix_ms"), "streamed in full");

        let r = get(&server, "/api/v1/status", &[])?;
        assert_eq!(json_body(&r)["device"]["status"], "ready");

        let r = get(&server, "/metrics", &[])?;
        let text = String::from_utf8_lossy(&r.body);
        assert!(
            text.contains("tower_bridge_temperature_celsius{device=\"SN1\",channel=\"0\"} 23.456"),
            "{text}"
        );
        assert!(text.contains(
            "tower_bridge_alert_active{metric=\"temperature\",channel=\"0\",bound=\"high\"} 1"
        ));

        assert_eq!(get(&server, "/api/v1/openapi.json", &[])?.status, 200);
        assert_eq!(get(&server, "/nope", &[])?.status, 404);
        Ok(())
    }

    #[test]
    fn control_requires_opt_in() -> std::io::Result<()> {
        let (server, backend) = start(open_policy())?;
        let addr = server.local_addr();
        let patch = br#"{"enabled": true, "max": 26.5}"#;
        let ct = [("Content-Type", "application/json")];
        let r = request(
            addr,
            "PATCH",
            "/api/v1/alerts/temperature",
            &ct,
            patch,
            Duration::from_secs(5),
        )?;
        assert_eq!(r.status, 403);

        if let Ok(mut p) = backend.policy.lock() {
            p.allow_control = true;
        }
        let r = request(
            addr,
            "PATCH",
            "/api/v1/alerts/temperature",
            &ct,
            patch,
            Duration::from_secs(5),
        )?;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        assert_eq!(json_body(&r)["max"], 26.5);
        assert_eq!(json_body(&r)["min"], 18.0, "unspecified fields are kept");

        let r = request(
            addr,
            "PATCH",
            "/api/v1/alerts/temperature",
            &ct,
            br#"{"min": 30, "max": 20}"#,
            Duration::from_secs(5),
        )?;
        assert_eq!(r.status, 400);
        let r = request(
            addr,
            "PATCH",
            "/api/v1/alerts/wind",
            &ct,
            patch,
            Duration::from_secs(5),
        )?;
        assert_eq!(r.status, 404);
        let r = request(
            addr,
            "POST",
            "/api/v1/device/identify",
            &[],
            b"",
            Duration::from_secs(5),
        )?;
        assert_eq!(r.status, 200);
        assert_eq!(backend.identified.lock().map(|n| *n).unwrap_or(0), 1);
        Ok(())
    }

    #[test]
    fn a_slow_rescan_does_not_block_other_requests() -> std::io::Result<()> {
        let (server, backend) = start(open_policy())?;
        if let Ok(mut p) = backend.policy.lock() {
            p.allow_control = true;
        }
        if let Ok(mut d) = backend.rescan_time.lock() {
            *d = Duration::from_millis(1500);
        }
        let addr = server.local_addr();
        let slow = std::thread::spawn(move || {
            request(
                addr,
                "POST",
                "/api/v1/device/rescan",
                &[],
                b"",
                Duration::from_secs(5),
            )
        });
        std::thread::sleep(Duration::from_millis(150));
        let started = std::time::Instant::now();
        assert_eq!(get(&server, "/api/v1/status", &[])?.status, 200);
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "status waited {:?} for the rescan",
            started.elapsed()
        );
        let r = slow
            .join()
            .map_err(|_| std::io::Error::other("rescan thread"))??;
        assert_eq!(r.status, 200);
        Ok(())
    }

    #[test]
    fn malformed_requests_get_json_errors() -> std::io::Result<()> {
        let (server, backend) = start(open_policy())?;
        if let Ok(mut p) = backend.policy.lock() {
            p.allow_control = true;
        }
        let addr = server.local_addr();
        // No content type on a JSON body.
        let r = request(
            addr,
            "PUT",
            "/api/v1/relay",
            &[],
            br#"{"state":true}"#,
            Duration::from_secs(5),
        )?;
        assert_eq!(r.status, 415);
        assert!(json_body(&r)["error"]["code"].is_string());
        let r = get(&server, "/api/v1/readings?max_points=abc", &[])?;
        assert_eq!(r.status, 400);
        assert!(json_body(&r)["error"]["message"].is_string());
        // Extreme times: an error or data, never a crash.
        let r = get(&server, "/api/v1/readings?to=-9223372036854775808", &[])?;
        assert_eq!(r.status, 400);
        let r = get(&server, "/api/v1/readings?from=-9223372036854775808", &[])?;
        assert_eq!(r.status, 200);
        Ok(())
    }

    #[test]
    fn nfc_endpoints() -> std::io::Result<()> {
        let (server, backend) = start(open_policy())?;
        let addr = server.local_addr();
        let ct = [("Content-Type", "application/json")];
        let put = |body: &[u8]| {
            request(
                addr,
                "PUT",
                "/api/v1/nfc",
                &ct,
                body,
                Duration::from_secs(5),
            )
        };

        let r = get(&server, "/api/v1/nfc", &[])?;
        assert_eq!(r.status, 200);
        assert_eq!(json_body(&r)["url"], Value::Null);
        let status = json_body(&get(&server, "/api/v1/status", &[])?);
        assert_eq!(status["nfc"]["address"], 0x08);
        assert_eq!(status["mqtt"]["status"], "disabled");

        // Writing needs "Allow changes", like the relay.
        assert_eq!(put(br#"{"url":"hardwario.com"}"#)?.status, 403);
        if let Ok(mut p) = backend.policy.lock() {
            p.allow_control = true;
        }
        let r = put(br#"{"url":" hardwario.com "}"#)?;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        assert_eq!(
            json_body(&r)["url"],
            "https://hardwario.com",
            "the scheme is added"
        );
        assert_eq!(put(br#"{"url":""}"#)?.status, 400);
        assert_eq!(put(br#"{}"#)?.status, 400);

        if let Ok(mut n) = backend.nfc.lock() {
            *n = None;
        }
        assert_eq!(get(&server, "/api/v1/nfc", &[])?.status, 404);
        assert_eq!(put(br#"{"url":"x.io"}"#)?.status, 404);
        Ok(())
    }

    #[test]
    fn relay_endpoints() -> std::io::Result<()> {
        let (server, backend) = start(open_policy())?;
        let addr = server.local_addr();
        let ct = [("Content-Type", "application/json")];
        let send = |method: &str, path: &str, body: &[u8]| {
            request(addr, method, path, &ct, body, Duration::from_secs(5))
        };

        let r = get(&server, "/api/v1/relay", &[])?;
        assert_eq!(r.status, 200);
        assert_eq!(json_body(&r)["state"], Value::Null);
        assert_eq!(
            json_body(&get(&server, "/api/v1/status", &[])?)["relay"]["address"],
            0x3B
        );
        let metrics = String::from_utf8_lossy(&get(&server, "/metrics", &[])?.body).into_owned();
        assert!(
            !metrics.contains("tower_bridge_relay_state"),
            "an unknown state is not exported"
        );

        // Switching needs "Allow changes".
        assert_eq!(
            send("PUT", "/api/v1/relay", br#"{"state":true}"#)?.status,
            403
        );
        if let Ok(mut p) = backend.policy.lock() {
            p.allow_control = true;
        }
        let r = send("PUT", "/api/v1/relay", br#"{"state":true}"#)?;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        assert_eq!(json_body(&r)["state"], true);
        assert_eq!(
            send("PUT", "/api/v1/relay", br#"{"state":"on"}"#)?.status,
            400
        );
        assert_eq!(send("PUT", "/api/v1/relay", b"{}")?.status, 400);

        let r = send("POST", "/api/v1/relay/pulse", b"")?;
        assert_eq!(r.status, 200);
        assert!(json_body(&r)["pulse_until"].is_number());
        assert_eq!(
            backend.pulsed.lock().ok().and_then(|p| *p),
            Some((None, None))
        );
        let r = send(
            "POST",
            "/api/v1/relay/pulse",
            br#"{"state":false,"duration_ms":1500}"#,
        )?;
        assert_eq!(r.status, 200);
        assert_eq!(
            backend.pulsed.lock().ok().and_then(|p| *p),
            Some((Some(false), Some(Duration::from_millis(1500))))
        );
        assert_eq!(
            send("POST", "/api/v1/relay/pulse", br#"{"duration_ms":-1}"#)?.status,
            400
        );

        if let Ok(mut r) = backend.relay.lock() {
            *r = None;
        }
        assert_eq!(get(&server, "/api/v1/relay", &[])?.status, 404);
        assert_eq!(
            send("PUT", "/api/v1/relay", br#"{"state":true}"#)?.status,
            404
        );
        Ok(())
    }

    #[test]
    fn rejects_foreign_host_and_origin() -> std::io::Result<()> {
        let (server, _) = start(open_policy())?;
        let r = get(&server, "/api/v1/status", &[("Host", "attacker.example")])?;
        assert_eq!(r.status, 403);
        let r = get(
            &server,
            "/api/v1/status",
            &[("Origin", "https://attacker.example")],
        )?;
        assert_eq!(r.status, 403);
        let r = get(
            &server,
            "/api/v1/status",
            &[("Authorization", "Bearer wrong")],
        )?;
        assert_eq!(r.status, 401);
        let r = get(
            &server,
            "/api/v1/status",
            &[
                ("Host", "attacker.example"),
                ("Authorization", "Bearer tok"),
            ],
        )?;
        assert_eq!(r.status, 200, "a valid token is accepted from anywhere");
        Ok(())
    }

    fn mcp_post(
        server: &ApiServer,
        body: &Value,
        headers: &[(&str, &str)],
    ) -> std::io::Result<HttpResponse> {
        let mut h = vec![
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
        ];
        h.extend_from_slice(headers);
        request(
            server.local_addr(),
            "POST",
            "/mcp",
            &h,
            body.to_string().as_bytes(),
            Duration::from_secs(5),
        )
    }

    #[test]
    fn mcp_batches_are_bounded() -> std::io::Result<()> {
        let (server, _) = start(open_policy())?;
        let ping = |id: u32| json!({"jsonrpc":"2.0","id":id,"method":"ping"});
        let legacy = [("MCP-Protocol-Version", "2025-03-26")];
        let r = mcp_post(&server, &json!([ping(1), ping(2)]), &legacy)?;
        assert_eq!(r.status, 200);
        assert_eq!(json_body(&r).as_array().map(Vec::len), Some(2));
        let r = mcp_post(&server, &json!([]), &legacy)?;
        assert_eq!(r.status, 400, "an empty batch is invalid");
        let many: Vec<Value> = (0..33).map(ping).collect();
        let r = mcp_post(&server, &Value::Array(many), &legacy)?;
        assert_eq!(r.status, 400, "a batch too long to queue on the bridge");
        Ok(())
    }

    #[test]
    fn mcp_over_http() -> std::io::Result<()> {
        let (server, backend) = start(open_policy())?;
        // Legacy client.
        let r = mcp_post(
            &server,
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
            &[],
        )?;
        assert_eq!(r.status, 200);
        assert_eq!(json_body(&r)["result"]["protocolVersion"], "2025-06-18");
        let r = mcp_post(
            &server,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            &[("MCP-Protocol-Version", "2025-06-18")],
        )?;
        assert_eq!(r.status, 202);

        // Modern client.
        let call = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"get_current_readings","arguments":{},
            "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}});
        let headers = [
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", "get_current_readings"),
        ];
        let r = mcp_post(&server, &call, &headers)?;
        assert_eq!(r.status, 200);
        let v = json_body(&r);
        assert_eq!(v["result"]["resultType"], "complete");
        assert_eq!(
            v["result"]["structuredContent"]["readings"][0]["humidity_pct"],
            41.3
        );

        let r = mcp_post(&server, &call, &headers[..2])?;
        assert_eq!(r.status, 400, "missing Mcp-Name");
        assert_eq!(json_body(&r)["error"]["code"], mcp_codes::HEADER_MISMATCH);

        let missing = json!({"jsonrpc":"2.0","id":3,"method":"prompts/list","params":{
            "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}});
        let r = mcp_post(
            &server,
            &missing,
            &[
                ("MCP-Protocol-Version", "2026-07-28"),
                ("Mcp-Method", "prompts/list"),
            ],
        )?;
        assert_eq!(r.status, 404);

        assert_eq!(get(&server, "/mcp", &[])?.status, 405);

        // Disabling MCP hides the endpoint.
        if let Ok(mut p) = backend.policy.lock() {
            p.mcp = false;
        }
        assert_eq!(mcp_post(&server, &call, &headers)?.status, 404);
        Ok(())
    }

    #[test]
    fn lan_mode_requires_token() -> std::io::Result<()> {
        let backend = Arc::new(FakeBackend::new(open_policy()));
        let server =
            match ApiServer::start("0.0.0.0:0".parse().map_err(std::io::Error::other)?, backend) {
                Ok(s) => s,
                Err(_) => return Ok(()), // no permission to bind all interfaces in this environment
            };
        let addr: SocketAddr = format!("127.0.0.1:{}", server.local_addr().port())
            .parse()
            .map_err(std::io::Error::other)?;
        let r = request(
            addr,
            "GET",
            "/api/v1/status",
            &[],
            b"",
            Duration::from_secs(5),
        )?;
        assert_eq!(r.status, 401);
        let r = request(
            addr,
            "GET",
            "/api/v1/status",
            &[("Authorization", "Bearer tok")],
            b"",
            Duration::from_secs(5),
        )?;
        assert_eq!(r.status, 200);
        Ok(())
    }

    #[test]
    fn port_in_use() -> std::io::Result<()> {
        let taken = std::net::TcpListener::bind("127.0.0.1:0")?;
        let backend = Arc::new(FakeBackend::new(open_policy()));
        assert!(ApiServer::start(taken.local_addr()?, backend).is_err());
        Ok(())
    }

    mod mcp_codes {
        pub use crate::api::mcp::HEADER_MISMATCH;
    }

    #[test]
    fn idle_connections_are_closed_and_requests_still_served() -> std::io::Result<()> {
        use std::io::{Read, Write};
        let (mut server, _) = start(Policy {
            token: "tok".into(),
            allow_control: false,
            mcp: false,
        })?;
        let addr = server.local_addr();
        // Says nothing: closed after the idle timeout.
        let mut idle = std::net::TcpStream::connect(addr)?;
        idle.set_read_timeout(Some(Duration::from_secs(8)))?;
        let started = std::time::Instant::now();
        let mut buf = [0u8; 64];
        let n = idle.read(&mut buf).unwrap_or(0);
        assert_eq!(n, 0, "closed without a response");
        assert!(started.elapsed() < Duration::from_secs(6));
        // A real request is answered.
        let mut client = std::net::TcpStream::connect(addr)?;
        client.write_all(b"GET /api/v1/status HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer tok\r\nConnection: close\r\n\r\n")?;
        let mut response = String::new();
        client.read_to_string(&mut response)?;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        server.stop();
        Ok(())
    }
}
