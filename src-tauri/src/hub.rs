//! The hub thread: receives readings and device state from the acquisition
//! thread and fans them out to the database, alert engine, MQTT broker, tray
//! and dashboard. A single consumer keeps ordering simple.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, select, tick};
use tauri::{AppHandle, Emitter, Manager};
use tower_bridge_core::acquisition::AcqEvent;
use tower_bridge_core::alerts::{AlertEngine, AlertEvent, AlertKind, Bound};
use tower_bridge_core::model::{DeviceState, Reading, altitude};
use tower_bridge_core::mqtt::{self, BrokerConfig, MqttCommand, Publisher};
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_core::store::Store;
use tracing::{error, info, warn};

use crate::format;
use crate::i18n::{Lang, fill};
use crate::notify;
use crate::settings::{MqttSettings, Settings};
use crate::state::{AppState, MqttState};

const RETENTION_EVERY: Duration = Duration::from_secs(3600);
const DEVICE_LOST_AFTER: Duration = Duration::from_secs(60);

pub enum HubMsg {
    /// Apply initial settings (MQTT, tray) once the app is set up.
    Init,
    SettingsChanged {
        old: Arc<Settings>,
        new: Arc<Settings>,
    },
    Shutdown,
}

/// Identity of a running broker listener.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ListenerKey {
    port: u16,
    allow_lan: bool,
    username: String,
    password: String,
}

impl From<&MqttSettings> for ListenerKey {
    fn from(m: &MqttSettings) -> Self {
        let (port, allow_lan, username, password) = m.listener_key();
        Self {
            port,
            allow_lan,
            username: username.into(),
            password: password.into(),
        }
    }
}

struct Hub {
    app: AppHandle,
    store: Option<Store>,
    engine: AlertEngine,
    broker: Option<(ListenerKey, Publisher)>,
    /// Where the broker delivers relay commands.
    mqtt_commands: Sender<MqttCommand>,
    publishing: bool,
    last_retention: Option<Instant>,
    lost_since: Option<Instant>,
    lost_notified: bool,
    was_ready: bool,
}

pub fn spawn(
    app: AppHandle,
    acq_rx: Receiver<AcqEvent>,
    hub_rx: Receiver<HubMsg>,
    store: Option<Store>,
    settings: Arc<Settings>,
    mqtt_commands: Sender<MqttCommand>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("hub".into())
        .spawn(move || {
            let mut hub = Hub {
                app,
                store,
                engine: AlertEngine::new(settings.alerts.clone()),
                broker: None,
                mqtt_commands,
                publishing: false,
                last_retention: None,
                lost_since: None,
                lost_notified: false,
                was_ready: false,
            };
            hub.run(acq_rx, hub_rx);
        })
}

impl Hub {
    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    fn run(&mut self, acq_rx: Receiver<AcqEvent>, hub_rx: Receiver<HubMsg>) {
        let ticker = tick(Duration::from_secs(5));
        loop {
            select! {
                recv(acq_rx) -> ev => match ev {
                    Ok(AcqEvent::Reading(r)) => self.on_reading(r),
                    Ok(AcqEvent::Device(d)) => self.on_device(d),
                    Ok(AcqEvent::Relay(r)) => self.on_relay(r),
                    Err(_) => break,
                },
                recv(hub_rx) -> msg => match msg {
                    Ok(HubMsg::Init) => self.on_init(),
                    Ok(HubMsg::SettingsChanged { old, new }) => self.on_settings(&old, &new),
                    Ok(HubMsg::Shutdown) | Err(_) => break,
                },
                recv(ticker) -> _ => self.on_tick(),
            }
        }
        info!("hub stopped");
    }

    fn refresh_tray(&self) {
        let state = self.state();
        let settings = state.settings();
        let device = state.device.read().clone();
        let latest = state.latest.read().clone();
        let alerts = state.alerts.read().clone();
        let mqtt = state.mqtt.read().clone();
        state
            .tray
            .update(&device, latest.as_ref(), &settings, &alerts, &mqtt);
    }

    fn on_init(&mut self) {
        let settings = self.state().settings();
        self.apply_mqtt(&settings);
        self.run_retention(&settings);
        self.refresh_tray();
    }

    fn on_reading(&mut self, mut r: Reading) {
        let settings = self.state().settings();
        r.altitude = r
            .values
            .pressure
            .map(|p| altitude(p, settings.sea_level_pressure_pa));

        if let Some(store) = self.store.as_mut()
            && let Err(e) = store.insert(&r)
        {
            error!("cannot store reading: {e}");
        }

        let events = self.engine.evaluate(r.ts, &r.values);
        if !events.is_empty() {
            for ev in &events {
                self.on_alert(ev, &settings);
            }
            let active = self.engine.active();
            *self.state().alerts.write() = active.clone();
            let _ = self.app.emit("alerts-active", &active);
        }

        if self.publishing
            && let Some((_, publisher)) = self.broker.as_mut()
            && let Err(e) = publisher.publish_reading(&r)
        {
            warn!("MQTT publish failed: {e}");
        }

        *self.state().latest.write() = Some(r.clone());
        self.refresh_tray();
        let _ = self.app.emit("reading", &r);
    }

    fn on_alert(&mut self, ev: &AlertEvent, settings: &Settings) {
        if let Some(store) = self.store.as_mut()
            && let Err(e) = store.log_alert(ev)
        {
            error!("cannot log alert: {e}");
        }
        let _ = self.app.emit("alert", ev);
        if !ev.notify {
            return;
        }
        let unit = settings.temperature_unit;
        let lang = Lang::effective(settings.language);
        let tr = lang.tr();
        let value = format::full(ev.metric, ev.value, unit, lang);
        let limit = format::full(ev.metric, ev.threshold, unit, lang);
        let vars = [("value", value.as_str()), ("limit", limit.as_str())];
        let (title, body) = match (ev.kind, ev.bound) {
            (AlertKind::Triggered, Bound::High) => {
                (tr.high(ev.metric).to_string(), fill(tr.above_limit, &vars))
            }
            (AlertKind::Triggered, Bound::Low) => {
                (tr.low(ev.metric).to_string(), fill(tr.below_limit, &vars))
            }
            (AlertKind::Recovered, _) => (
                fill(tr.back_to_normal, &[("metric", tr.metric(ev.metric))]),
                fill(tr.within_limits, &vars),
            ),
        };
        notify::send(&self.app, &title, &body);
    }

    fn on_device(&mut self, d: DeviceState) {
        let ready = d.is_ready();
        if ready {
            if self.lost_notified {
                let tr = self.lang().tr();
                notify::send(&self.app, tr.reconnected_title, tr.reconnected_body);
            }
            self.lost_since = None;
            self.lost_notified = false;
            self.was_ready = true;
        } else if self.was_ready && self.lost_since.is_none() {
            self.lost_since = Some(Instant::now());
        }
        *self.state().device.write() = d.clone();
        self.refresh_tray();
        let _ = self.app.emit("device-state", &d);
    }

    fn on_relay(&mut self, relay: Option<RelayInfo>) {
        *self.state().relay.write() = relay;
        self.publish_relay();
        let _ = self.app.emit("relay-state", relay);
    }

    fn publish_relay(&mut self) {
        let relay = *self.state().relay.read();
        if self.publishing
            && let Some((_, publisher)) = self.broker.as_mut()
            && let Err(e) = publisher.publish_relay(relay.as_ref())
        {
            warn!("MQTT publish failed: {e}");
        }
    }

    fn on_tick(&mut self) {
        let settings = self.state().settings();
        if let Some(since) = self.lost_since
            && !self.lost_notified
            && since.elapsed() >= DEVICE_LOST_AFTER
        {
            self.lost_notified = true;
            if settings.notify_device_lost {
                let tr = Lang::effective(settings.language).tr();
                notify::send(&self.app, tr.lost_title, tr.lost_body);
            }
        }
        if self
            .last_retention
            .is_none_or(|t| t.elapsed() >= RETENTION_EVERY)
        {
            self.run_retention(&settings);
        }
    }

    fn run_retention(&mut self, settings: &Settings) {
        self.last_retention = Some(Instant::now());
        let Some(store) = self.store.as_mut() else {
            return;
        };
        let cutoff = tower_bridge_core::acquisition::unix_ms()
            - i64::from(settings.retention_days) * 86_400_000;
        match store.purge_older_than(cutoff) {
            Ok(0) => {}
            Ok(n) => info!("retention: deleted {n} old readings"),
            Err(e) => warn!("retention failed: {e}"),
        }
    }

    fn on_settings(&mut self, old: &Settings, new: &Settings) {
        if old.alerts != new.alerts {
            self.engine.set_rules(new.alerts.clone());
            let active = self.engine.active();
            *self.state().alerts.write() = active.clone();
            let _ = self.app.emit("alerts-active", &active);
        }
        if old.retention_days != new.retention_days {
            self.run_retention(new);
        }
        if old.mqtt != new.mqtt {
            self.apply_mqtt(new);
        }
        if old.sea_level_pressure_pa != new.sea_level_pressure_pa {
            // Recompute altitude of the latest reading for the UI.
            let latest = {
                let state = self.state();
                let mut guard = state.latest.write();
                if let Some(r) = guard.as_mut() {
                    r.altitude = r
                        .values
                        .pressure
                        .map(|p| altitude(p, new.sea_level_pressure_pa));
                }
                guard.clone()
            };
            if let Some(r) = latest {
                let _ = self.app.emit("reading", &r);
            }
        }
        self.refresh_tray();
    }

    fn lang(&self) -> Lang {
        Lang::effective(self.state().settings().language)
    }

    fn set_mqtt_state(&self, s: MqttState) {
        *self.state().mqtt.write() = s.clone();
        self.refresh_tray();
        let _ = self.app.emit("mqtt-state", &s);
    }

    fn apply_mqtt(&mut self, settings: &Settings) {
        let m = &settings.mqtt;
        let key = ListenerKey::from(m);
        match (&mut self.broker, m.enabled) {
            (None, false) => {
                self.publishing = false;
                self.set_mqtt_state(MqttState::Disabled);
            }
            (None, true) => {
                let bind = if m.allow_lan {
                    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
                } else {
                    IpAddr::V4(Ipv4Addr::LOCALHOST)
                };
                let credentials =
                    (!m.username.is_empty()).then(|| (m.username.clone(), m.password.clone()));
                let cfg = BrokerConfig {
                    bind,
                    port: m.port,
                    credentials,
                };
                match mqtt::start(&cfg, &m.topic_prefix, self.mqtt_commands.clone()) {
                    Ok(publisher) => {
                        self.broker = Some((key, publisher));
                        self.publishing = true;
                        self.set_mqtt_state(MqttState::Running {
                            address: format!("{bind}:{}", m.port),
                        });
                        let latest = self.state().latest.read().clone();
                        if let (Some(r), Some((_, p))) = (latest, self.broker.as_mut()) {
                            let _ = p.publish_reading(&r);
                        }
                        if self.state().relay.read().is_some() {
                            self.publish_relay();
                        }
                    }
                    Err(e) => {
                        warn!("{e}");
                        self.publishing = false;
                        self.set_mqtt_state(MqttState::Error {
                            message: e.to_string(),
                        });
                    }
                }
            }
            (Some((running, publisher)), true) if *running == key => {
                publisher.set_prefix(&m.topic_prefix);
                self.publishing = true;
                if self.state().relay.read().is_some() {
                    self.publish_relay();
                }
                let bind = if m.allow_lan { "0.0.0.0" } else { "127.0.0.1" };
                self.set_mqtt_state(MqttState::Running {
                    address: format!("{bind}:{}", m.port),
                });
            }
            (Some(_), enabled) => {
                self.publishing = false;
                // Codes are translated by the dashboard.
                let reason = if enabled {
                    "apply_settings"
                } else {
                    "stop_broker"
                };
                self.set_mqtt_state(MqttState::RestartRequired {
                    reason: reason.into(),
                });
            }
        }
    }
}
