//! MCP over stdio, for clients that launch local processes (e.g. Claude
//! Desktop). The process forwards requests to the running app's `/mcp`
//! endpoint; discovery and the legacy handshake are answered locally so
//! the client can start even while the app is closed, and tool calls then
//! explain how to fix it.

use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::http_client::request;
use super::mcp::{self, LEGACY_VERSIONS, META_VERSION, McpServer, RpcError};
use super::{ApiBackend, ApiError, ApiInfo, Policy};
use crate::alerts::{ActiveAlert, AlertRule, AlertRules};
use crate::model::{DeviceState, Metric, Reading};
use crate::relay::RelayInfo;
use crate::store::{AlertLogEntry, History};

const FORWARD_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the running app's API can be reached, re-read for every request.
#[derive(Clone, Debug, Default)]
pub struct StdioTarget {
    /// `None` when the API server is disabled.
    pub addr: Option<SocketAddr>,
    pub token: String,
    pub allow_control: bool,
}

/// Answers local requests while the app is unreachable.
struct OfflineBackend {
    info: ApiInfo,
    allow_control: bool,
}

impl OfflineBackend {
    fn unavailable<T>() -> Result<T, ApiError> {
        Err(ApiError::Unavailable(
            "TOWER Bridge is not reachable".into(),
        ))
    }
}

impl ApiBackend for OfflineBackend {
    fn info(&self) -> ApiInfo {
        self.info.clone()
    }
    fn policy(&self) -> Policy {
        Policy {
            token: String::new(),
            allow_control: self.allow_control,
            mcp: true,
        }
    }
    fn device(&self) -> DeviceState {
        DeviceState::Searching
    }
    fn latest(&self) -> Option<Reading> {
        None
    }
    fn sample_interval_s(&self) -> u64 {
        0
    }
    fn history(&self, _: i64, _: i64, _: usize) -> Result<History, ApiError> {
        Self::unavailable()
    }
    fn csv(&self, _: i64, _: i64) -> Result<Vec<u8>, ApiError> {
        Self::unavailable()
    }
    fn alert_rules(&self) -> AlertRules {
        AlertRules::default()
    }
    fn active_alerts(&self) -> Vec<ActiveAlert> {
        Vec::new()
    }
    fn alert_log(&self, _: usize) -> Result<Vec<AlertLogEntry>, ApiError> {
        Self::unavailable()
    }
    fn set_alert_rule(&self, _: Metric, _: AlertRule) -> Result<AlertRules, ApiError> {
        Self::unavailable()
    }
    fn identify(&self) -> Result<(), ApiError> {
        Self::unavailable()
    }
    fn rescan(&self) -> Result<(), ApiError> {
        Self::unavailable()
    }
    fn relay(&self) -> Option<RelayInfo> {
        None
    }
    fn set_relay(&self, _: bool) -> Result<RelayInfo, ApiError> {
        Self::unavailable()
    }
    fn pulse_relay(&self, _: Option<bool>, _: Option<Duration>) -> Result<RelayInfo, ApiError> {
        Self::unavailable()
    }
}

/// Serve MCP on this process's stdin/stdout until stdin closes.
pub fn run(info: ApiInfo, target: impl Fn() -> StdioTarget) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(info, target, stdin.lock(), stdout.lock())
}

/// Newline-delimited JSON-RPC loop over any reader/writer.
pub fn serve<R: BufRead, W: Write>(
    info: ApiInfo,
    target: impl Fn() -> StdioTarget,
    reader: R,
    mut writer: W,
) -> std::io::Result<()> {
    let mut legacy_version = LEGACY_VERSIONS[1].to_string();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&info, &target(), &msg, &mut legacy_version),
            Err(e) => Some(mcp::error_response(
                Value::Null,
                &RpcError::new(mcp::PARSE_ERROR, format!("parse error: {e}")),
            )),
        };
        if let Some(reply) = reply {
            writeln!(writer, "{reply}")?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn handle(
    info: &ApiInfo,
    target: &StdioTarget,
    msg: &Value,
    legacy_version: &mut String,
) -> Option<Value> {
    let local = McpServer::new(Arc::new(OfflineBackend {
        info: info.clone(),
        allow_control: target.allow_control,
    }));
    let method = msg.get("method").and_then(Value::as_str)?;
    let id = msg.get("id").cloned()?;

    if matches!(method, "initialize" | "server/discover" | "ping") {
        let reply = local.handle(msg).body;
        if method == "initialize"
            && let Some(v) = reply
                .as_ref()
                .and_then(|r| r["result"]["protocolVersion"].as_str())
        {
            *legacy_version = v.to_string();
        }
        return reply;
    }

    let failure = match forward(target, msg, legacy_version) {
        Ok(reply) => return reply,
        Err(reason) => reason,
    };
    match method {
        // Still let the client see the tools while the app is closed.
        "tools/list" => local.handle(msg).body,
        "tools/call" => {
            let mut result =
                json!({ "content": [{ "type": "text", "text": failure }], "isError": true });
            let modern = msg
                .get("params")
                .and_then(|p| p.get("_meta"))
                .and_then(|m| m.get(META_VERSION))
                .is_some();
            if modern {
                result["resultType"] = json!("complete");
            }
            Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
        }
        _ => Some(mcp::error_response(
            id,
            &RpcError::new(mcp::INTERNAL_ERROR, failure),
        )),
    }
}

/// POST a message to the app; `Err` carries a user-facing explanation.
fn forward(
    target: &StdioTarget,
    msg: &Value,
    legacy_version: &str,
) -> Result<Option<Value>, String> {
    let Some(addr) = target.addr else {
        return Err("The TOWER Bridge API server is turned off. Turn on Settings → API & MCP in the TOWER Bridge app.".into());
    };
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let version = params
        .get("_meta")
        .and_then(|m| m.get(META_VERSION))
        .and_then(Value::as_str)
        .unwrap_or(legacy_version)
        .to_string();
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| n.bytes().all(|b| b.is_ascii_graphic()))
        .map(str::to_string);
    let auth = format!("Bearer {}", target.token);
    let mut headers: Vec<(&str, &str)> = vec![
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
        ("MCP-Protocol-Version", &version),
        ("Mcp-Method", method),
    ];
    if let Some(n) = &name {
        headers.push(("Mcp-Name", n));
    }
    if !target.token.is_empty() {
        headers.push(("Authorization", &auth));
    }
    let body = msg.to_string();
    let resp = request(addr, "POST", "/mcp", &headers, body.as_bytes(), FORWARD_TIMEOUT).map_err(|_| {
        "The TOWER Bridge app is not running (or its API server is unreachable). Start TOWER Bridge and try again."
            .to_string()
    })?;
    match resp.status {
        202 => Ok(None),
        401 | 403 => Err("TOWER Bridge rejected the request (access token mismatch). Restart the MCP client to reload the settings.".into()),
        _ => match serde_json::from_slice::<Value>(&resp.body) {
            Ok(v) if v.get("jsonrpc").is_some() => Ok(Some(v)),
            _ if resp.status == 404 => Err("The MCP server is turned off in TOWER Bridge → Settings → API & MCP.".into()),
            _ => Err(format!("TOWER Bridge returned HTTP {}", resp.status)),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::api::ApiServer;
    use crate::api::testing::FakeBackend;

    fn info() -> ApiInfo {
        ApiInfo {
            name: "tower-bridge".into(),
            version: "0.0.0-test".into(),
        }
    }

    fn run_lines(target: StdioTarget, input: &[Value]) -> std::io::Result<Vec<Value>> {
        let text: String = input.iter().map(|v| format!("{v}\n")).collect();
        let mut out = Vec::new();
        serve(info(), || target.clone(), Cursor::new(text), &mut out)?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect())
    }

    #[test]
    fn forwards_to_running_app() -> std::io::Result<()> {
        let backend = Arc::new(FakeBackend::new(Policy {
            token: "tok".into(),
            allow_control: false,
            mcp: true,
        }));
        let server = ApiServer::start(
            "127.0.0.1:0".parse().map_err(std::io::Error::other)?,
            backend,
        )?;
        let target = StdioTarget {
            addr: Some(server.local_addr()),
            token: "tok".into(),
            allow_control: false,
        };
        let out = run_lines(
            target,
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_current_readings","arguments":{}}}),
            ],
        )?;
        assert_eq!(out.len(), 3, "the notification gets no reply");
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-03-26");
        assert!(
            out[1]["result"]["tools"]
                .as_array()
                .is_some_and(|t| t.len() == 4)
        );
        assert_eq!(
            out[2]["result"]["structuredContent"]["temperature_c"],
            23.46
        );
        Ok(())
    }

    #[test]
    fn explains_when_app_is_not_running() -> std::io::Result<()> {
        let port = std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port();
        let target = StdioTarget {
            addr: Some(SocketAddr::from(([127, 0, 0, 1], port))),
            token: String::new(),
            allow_control: true,
        };
        let out = run_lines(
            target,
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_history","arguments":{}}}),
            ],
        )?;
        assert!(
            out[0]["result"]["tools"]
                .as_array()
                .is_some_and(|t| t.len() == 8)
        );
        assert_eq!(out[1]["result"]["isError"], true);
        assert!(
            out[1]["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|t| t.contains("not running"))
        );

        let disabled = run_lines(
            StdioTarget::default(),
            &[
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_alerts","arguments":{}}}),
            ],
        )?;
        assert!(
            disabled[0]["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|t| t.contains("turned off"))
        );
        Ok(())
    }

    #[test]
    fn modern_discover_is_local() -> std::io::Result<()> {
        let out = run_lines(
            StdioTarget::default(),
            &[
                json!({"jsonrpc":"2.0","id":"d","method":"server/discover","params":{"_meta":{
                "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                "io.modelcontextprotocol/clientCapabilities":{}}}}),
            ],
        )?;
        assert_eq!(out[0]["result"]["resultType"], "complete");
        assert!(
            out[0]["result"]["supportedVersions"]
                .as_array()
                .is_some_and(|v| v.len() >= 2)
        );
        Ok(())
    }
}
