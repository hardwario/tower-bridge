//! HTTP server lifecycle: runs on its own thread with a Tokio runtime and
//! stops gracefully, so it can be restarted when settings change.

use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use tracing::{info, warn};

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
                        Ok(l) => l,
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
        assert_eq!(v["temperature"], 23.456);
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

        let r = get(&server, "/api/v1/status", &[])?;
        assert_eq!(json_body(&r)["device"]["status"], "ready");

        let r = get(&server, "/metrics", &[])?;
        let text = String::from_utf8_lossy(&r.body);
        assert!(
            text.contains("tower_bridge_temperature_celsius{device=\"SN1\"} 23.456"),
            "{text}"
        );
        assert!(
            text.contains("tower_bridge_alert_active{metric=\"temperature\",bound=\"high\"} 1")
        );

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
        assert_eq!(v["result"]["structuredContent"]["humidity_pct"], 41.3);

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
}
