//! `run`: the service without a window, and `mcp`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use parking_lot::Mutex;
use tower_bridge_core::alerts::{AlertEvent, AlertKind};
use tower_bridge_core::api::ApiInfo;
use tower_bridge_core::model::DeviceState;
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_service::{
    ApiState, Mode, MqttState, Observer, Service, ServiceConfig, ServiceState, StartError,
};
use tracing::{info, warn};

use super::watch::stop_signal;
use crate::access::Ctx;
use crate::{exit, init_logging, output};

/// The service's events as log lines on stderr (the journal under systemd).
#[derive(Default)]
struct LogObserver {
    /// What was said last, so a repeated state is not logged again.
    device: Mutex<Option<String>>,
}

impl Observer for LogObserver {
    fn device(&self, _: &ServiceState, d: &DeviceState) {
        let text = match d {
            DeviceState::Ready { sources, .. } => {
                let mut channels: Vec<u8> = sources.iter().map(|s| s.channel).collect();
                channels.sort_unstable();
                channels.dedup();
                let modules: Vec<String> = channels
                    .iter()
                    .map(|c| {
                        format!(
                            "I²C channel {c}: {}",
                            output::modules_on(sources, *c).join(", ")
                        )
                    })
                    .collect();
                format!("bridge {}; {}", output::device(d), modules.join("; "))
            }
            other => format!("bridge {}", output::device(other)),
        };
        let mut last = self.device.lock();
        if last.as_deref() != Some(text.as_str()) {
            info!("{text}");
            *last = Some(text);
        }
    }

    fn relay(&self, _: &ServiceState, r: Option<&RelayInfo>) {
        if let Some(r) = r {
            info!(
                "relay {} (I²C channel {}, 0x{:02X})",
                output::relay_state(r.state),
                r.channel,
                r.address
            );
        }
    }

    fn alert(&self, state: &ServiceState, ev: &AlertEvent) {
        let text = output::alert(ev, state.settings().temperature_unit);
        match ev.kind {
            AlertKind::Triggered => warn!("alert: {text}"),
            AlertKind::Recovered => info!("alert over: {text}"),
        }
    }

    fn mqtt(&self, _: &ServiceState, m: &MqttState) {
        match m {
            MqttState::Disabled => info!("MQTT off"),
            MqttState::Connecting {
                address,
                error: None,
            } => info!("MQTT connecting to {address}"),
            MqttState::Connecting {
                address,
                error: Some(e),
            } => warn!("MQTT connecting to {address} again: {e}"),
            MqttState::Connected { address } => info!("MQTT connected to {address}"),
            MqttState::Error { message } if message == "no_host" => {
                warn!("MQTT is on, but no broker is set (mqtt.host)")
            }
            MqttState::Error { message } => warn!("MQTT: {message}"),
        }
    }

    fn api(&self, state: &ServiceState, a: &ApiState) {
        match a {
            ApiState::Disabled => {}
            ApiState::Running { address, url } => {
                let s = state.settings();
                info!(
                    "API server on {address} ({url}{}{})",
                    if s.api.mcp { ", MCP on" } else { "" },
                    if s.api.allow_control {
                        ", changes allowed"
                    } else {
                        ""
                    }
                );
            }
            ApiState::Error { port, message } => {
                warn!("API server cannot listen on port {port}: {message}")
            }
        }
    }

    fn bridge_lost(&self, _: &ServiceState) {
        warn!("the TOWER Bridge Module has been gone for a minute");
    }

    fn bridge_back(&self, _: &ServiceState) {
        info!("the TOWER Bridge Module is back");
    }
}

pub fn service(ctx: &Ctx, verbose: u8) -> Result<()> {
    init_logging(verbose, true);
    let stop = stop_signal();
    let service = match Service::start(ServiceConfig {
        paths: ctx.paths()?,
        mode: Mode::Full,
        simulated: ctx.simulate,
        program: "tower-bridge-cli".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        observer: Arc::new(LogObserver::default()),
    }) {
        Ok(s) => s,
        Err(StartError::AlreadyRunning(holder, dir)) => {
            return Err(crate::failure(
                exit::BUSY,
                format!("{holder} is already running with {}", dir.display()),
                Some("stop it first, or give this one its own --data-dir"),
            ));
        }
        Err(e) => return Err(e.into()),
    };
    let _ = stop.recv();
    info!("stopping");
    service.shutdown(Duration::from_secs(5));
    Ok(())
}

pub fn mcp(ctx: &Ctx) -> Result<()> {
    let info = ApiInfo {
        name: "tower-bridge-cli".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    tower_bridge_service::api::mcp_stdio(&ctx.paths()?, info).map_err(|e| anyhow::anyhow!(e))
}
