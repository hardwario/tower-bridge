//! The hub thread: receives readings and device state from the acquisition
//! thread and fans them out to the database, alert engine, MQTT broker and
//! the front end (through its `Observer`). A single consumer keeps ordering
//! simple.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, select, tick};
use tower_bridge_core::acquisition::AcqEvent;
use tower_bridge_core::alerts::{AlertEngine, AlertEvent};
use tower_bridge_core::gps::GpsInfo;
use tower_bridge_core::model::{DeviceState, Metric, Reading, altitude};
use tower_bridge_core::mqtt::{self, ClientConfig, LinkEvent, MqttCommand, Publisher};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_core::store::Store;
use tracing::{error, info, warn};

use crate::service::Mode;
use crate::settings::Settings;
use crate::state::{MqttState, ServiceState};

const RETENTION_EVERY: Duration = Duration::from_secs(3600);
const DEVICE_LOST_AFTER: Duration = Duration::from_secs(60);

pub enum HubMsg {
    /// Apply initial settings (MQTT, retention) once the service is set up.
    Init,
    SettingsChanged {
        old: Arc<Settings>,
        new: Arc<Settings>,
    },
    Shutdown,
}

/// When to say that the bridge went away and came back.
#[derive(Debug, Default)]
struct Presence {
    /// Since when the bridge is gone (after it had worked).
    lost_since: Option<Instant>,
    /// The time to report it ran out (notified or not).
    lost_reported: bool,
    /// The "disconnected" notification was shown.
    lost_shown: bool,
    was_ready: bool,
}

impl Presence {
    /// A new device state; returns whether to announce the bridge is back.
    fn device(&mut self, d: &DeviceState, now: Instant) -> bool {
        match d {
            DeviceState::Ready { .. } => {
                let back = self.lost_shown;
                *self = Presence {
                    was_ready: true,
                    ..Presence::default()
                };
                back
            }
            // The bridge is there and only the sensors are missing: not a
            // disconnect. A shown "disconnected" waits for the sensors.
            DeviceState::NoModule { .. } => {
                self.lost_since = None;
                self.lost_reported = self.lost_shown;
                false
            }
            DeviceState::Searching | DeviceState::Error { .. } => {
                if self.was_ready && self.lost_since.is_none() && !self.lost_reported {
                    self.lost_since = Some(now);
                }
                false
            }
        }
    }

    /// Returns whether to announce that the bridge is gone (only when
    /// `notify` is on; "back" is then announced too, and only then).
    fn tick(&mut self, now: Instant, notify: bool) -> bool {
        match self.lost_since {
            Some(since)
                if !self.lost_reported && now.duration_since(since) >= DEVICE_LOST_AFTER =>
            {
                self.lost_reported = true;
                self.lost_shown = notify;
                notify
            }
            _ => false,
        }
    }
}

/// The connection to the user's MQTT broker.
struct MqttClient {
    config: ClientConfig,
    prefix: String,
    address: String,
    publisher: Publisher,
    /// News of this connection only (an old one's cannot get mixed in).
    events: Receiver<LinkEvent>,
    connected: bool,
}

struct Hub {
    state: Arc<ServiceState>,
    store: Option<Store>,
    engine: AlertEngine,
    /// The MQTT broker connection, replaced whenever its settings change.
    mqtt: Option<MqttClient>,
    /// Where the broker delivers relay commands.
    mqtt_commands: Sender<MqttCommand>,
    last_retention: Option<Instant>,
    presence: Presence,
}

pub fn spawn(
    state: Arc<ServiceState>,
    acq_rx: Receiver<AcqEvent>,
    hub_rx: Receiver<HubMsg>,
    store: Option<Store>,
    mqtt_commands: Sender<MqttCommand>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("hub".into())
        .spawn(move || {
            let engine = AlertEngine::new(state.settings().alerts.clone());
            let mut hub = Hub {
                state,
                store,
                engine,
                mqtt: None,
                mqtt_commands,
                last_retention: None,
                presence: Presence::default(),
            };
            // A panic must not silently end storage, alerts and MQTT while
            // the front end looks alive.
            let mut panics = 0u32;
            loop {
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // After a panic: connect MQTT again (its client went with
                    // the panic) and refresh what the front end shows. Not
                    // forever, should that panic itself.
                    if (1..=3).contains(&panics) {
                        hub.on_init();
                    }
                    hub.run(&acq_rx, &hub_rx);
                }));
                if run.is_ok() {
                    break;
                }
                panics += 1;
                tracing::error!("hub thread panicked; restarting");
                hub.mqtt = None;
                hub.set_mqtt_state(MqttState::Disabled);
                std::thread::sleep(Duration::from_secs(1));
            }
        })
}

impl Hub {
    fn run(&mut self, acq_rx: &Receiver<AcqEvent>, hub_rx: &Receiver<HubMsg>) {
        let ticker = tick(Duration::from_secs(5));
        loop {
            let link_events = self
                .mqtt
                .as_ref()
                .map_or_else(crossbeam_channel::never, |e| e.events.clone());
            select! {
                recv(link_events) -> ev => {
                    if let Ok(ev) = ev {
                        self.on_link_event(ev);
                    }
                },
                recv(acq_rx) -> ev => match ev {
                    Ok(AcqEvent::Reading(r)) => self.on_reading(r),
                    Ok(AcqEvent::Device(d)) => self.on_device(d),
                    Ok(AcqEvent::Relay(r)) => self.on_relay(r),
                    Ok(AcqEvent::Nfc(n)) => self.on_nfc(n),
                    Ok(AcqEvent::Gps(g)) => self.on_gps(g),
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
        // A goodbye skips the broker's last will: say "offline" ourselves and
        // give the client a moment to send it.
        if self.mqtt.as_ref().is_some_and(|e| e.connected) {
            self.stop_mqtt();
            std::thread::sleep(Duration::from_millis(300));
        }
        info!("hub stopped");
    }

    fn refresh(&self) {
        self.state.observer().refresh(&self.state);
    }

    fn on_init(&mut self) {
        let settings = self.state.settings();
        self.apply_mqtt(&settings);
        self.run_retention(&settings);
        self.refresh();
    }

    fn on_reading(&mut self, mut r: Reading) {
        let settings = self.state.settings();
        r.altitude = r
            .values
            .pressure
            .map(|p| altitude(p, settings.sea_level_pressure_pa));

        if let Some(store) = self.store.as_mut()
            && let Err(e) = store.insert(&r)
        {
            error!("cannot store reading: {e}");
        }

        let events = self.engine.evaluate(r.ts, r.channel, &r.values);
        for ev in &events {
            self.on_alert(ev);
        }
        // Also while alerts stay active: their values follow the readings.
        self.publish_active_alerts();

        if let Some(publisher) = self.publisher()
            && let Err(e) = publisher.publish_reading(&r)
        {
            warn!("MQTT publish failed: {e}");
        }

        self.state.set_latest(r.clone());
        self.refresh();
        self.state.observer().reading(&self.state, &r);
    }

    fn on_alert(&mut self, ev: &AlertEvent) {
        if let Some(store) = self.store.as_mut()
            && let Err(e) = store.log_alert(ev)
        {
            error!("cannot log alert: {e}");
        }
        self.state.observer().alert(&self.state, ev);
    }

    /// Store and announce the active alerts when they changed.
    fn publish_active_alerts(&self) {
        let active = self.engine.active();
        if *self.state.alerts.read() == active {
            return;
        }
        *self.state.alerts.write() = active.clone();
        self.state.observer().alerts_active(&self.state, &active);
    }

    fn on_device(&mut self, d: DeviceState) {
        let back = self.presence.device(&d, Instant::now());
        if let DeviceState::Ready { sources, .. } = &d {
            // A metric no longer measured on a channel (a sensor gone) has no
            // current value, alert or MQTT topic any more.
            let measured: Vec<(Metric, u8)> =
                sources.iter().map(|s| (s.metric, s.channel)).collect();
            self.state
                .latest
                .write()
                .retain(|r| measured.iter().any(|(_, c)| *c == r.channel));
            if self.engine.retain_sources(&measured) {
                self.publish_active_alerts();
            }
            if let Some(publisher) = self.publisher()
                && let Err(e) = publisher.retain_sources(&measured)
            {
                warn!("MQTT publish failed: {e}");
            }
        }
        *self.state.device.write() = d.clone();
        if back {
            self.state.observer().bridge_back(&self.state);
        }
        self.refresh();
        self.state.observer().device(&self.state, &d);
    }

    fn on_relay(&mut self, relay: Option<RelayInfo>) {
        *self.state.relay.write() = relay;
        self.publish_relay();
        self.state.observer().relay(&self.state, relay.as_ref());
    }

    fn on_nfc(&mut self, nfc: Option<NfcInfo>) {
        *self.state.nfc.write() = nfc.clone();
        self.state.observer().nfc(&self.state, nfc.as_ref());
    }

    fn on_gps(&mut self, gps: Option<GpsInfo>) {
        *self.state.gps.write() = gps.clone();
        self.publish_gps();
        self.state.observer().gps(&self.state, gps.as_ref());
    }

    fn publish_gps(&mut self) {
        let gps = self.state.gps.read().clone();
        if let Some(publisher) = self.publisher()
            && let Err(e) = publisher.publish_gps(gps.as_ref())
        {
            warn!("MQTT publish failed: {e}");
        }
    }

    fn publish_relay(&mut self) {
        let relay = *self.state.relay.read();
        if let Some(publisher) = self.publisher()
            && let Err(e) = publisher.publish_relay(relay.as_ref())
        {
            warn!("MQTT publish failed: {e}");
        }
    }

    /// Where readings go now: the MQTT broker, while connected.
    fn publisher(&mut self) -> Option<&mut Publisher> {
        match &mut self.mqtt {
            Some(c) if c.connected => Some(&mut c.publisher),
            _ => None,
        }
    }

    /// Everything current, after (re)connecting or a prefix change: the
    /// broker may have lost the retained values.
    fn republish(&mut self) {
        let latest = self.state.latest.read().clone();
        if let Some(publisher) = self.publisher() {
            let mut result = publisher.publish_status(true);
            for r in &latest {
                result = result.and(publisher.publish_reading(r));
            }
            if let Err(e) = result {
                warn!("MQTT publish failed: {e}");
            }
        }
        self.publish_relay();
        self.publish_gps();
    }

    fn on_link_event(&mut self, ev: LinkEvent) {
        let Some(client) = self.mqtt.as_mut() else {
            return;
        };
        let address = client.address.clone();
        match ev {
            LinkEvent::Connected => {
                client.connected = true;
                self.set_mqtt_state(MqttState::Connected { address });
                self.republish();
            }
            LinkEvent::Disconnected(error) => {
                client.connected = false;
                self.set_mqtt_state(MqttState::Connecting {
                    address,
                    error: Some(error),
                });
            }
        }
    }

    /// Disconnect from the MQTT broker, saying goodbye.
    fn stop_mqtt(&mut self) {
        if let Some(mut e) = self.mqtt.take()
            && e.connected
            && let Err(err) = e.publisher.publish_status(false)
        {
            warn!("MQTT publish failed: {err}");
        }
    }

    fn start_mqtt(&mut self, config: ClientConfig, prefix: &str) {
        let address = format!("{}:{}", config.host, config.port);
        let (tx, events) = crossbeam_channel::unbounded();
        match mqtt::connect(&config, prefix, self.mqtt_commands.clone(), tx) {
            Ok(publisher) => {
                self.mqtt = Some(MqttClient {
                    config,
                    prefix: mqtt::sanitize_prefix(prefix),
                    address: address.clone(),
                    publisher,
                    events,
                    connected: false,
                });
                self.set_mqtt_state(MqttState::Connecting {
                    address,
                    error: None,
                });
            }
            Err(e) => {
                warn!("{e}");
                self.set_mqtt_state(MqttState::Error {
                    message: e.to_string(),
                });
            }
        }
    }

    fn on_tick(&mut self) {
        let settings = self.state.settings();
        if self
            .presence
            .tick(Instant::now(), settings.notify_device_lost)
        {
            self.state.observer().bridge_lost(&self.state);
        }
        if self
            .last_retention
            .is_none_or(|t| t.elapsed() >= RETENTION_EVERY)
        {
            self.run_retention(&settings);
        }
        // Readings that stop coming send no event: summaries notice them
        // getting old here.
        self.refresh();
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
            *self.state.alerts.write() = active.clone();
            self.state.observer().alerts_active(&self.state, &active);
        }
        if old.retention_days != new.retention_days {
            self.run_retention(new);
        }
        if old.mqtt != new.mqtt {
            self.apply_mqtt(new);
        }
        if old.sea_level_pressure_pa != new.sea_level_pressure_pa {
            // Recompute the altitude of the latest readings.
            let latest = {
                let mut guard = self.state.latest.write();
                for r in guard.iter_mut() {
                    r.altitude = r
                        .values
                        .pressure
                        .map(|p| altitude(p, new.sea_level_pressure_pa));
                }
                guard.clone()
            };
            for r in latest {
                self.state.observer().reading(&self.state, &r);
            }
        }
        self.refresh();
    }

    fn set_mqtt_state(&self, s: MqttState) {
        *self.state.mqtt.write() = s.clone();
        self.refresh();
        self.state.observer().mqtt(&self.state, &s);
    }

    fn apply_mqtt(&mut self, settings: &Settings) {
        let m = &settings.mqtt;
        if !m.enabled || self.state.mode != Mode::Full {
            self.stop_mqtt();
            self.set_mqtt_state(MqttState::Disabled);
            return;
        }
        let Some(config) = m.client_config() else {
            self.stop_mqtt();
            // Front ends translate this code.
            self.set_mqtt_state(MqttState::Error {
                message: "no_host".into(),
            });
            return;
        };
        let prefix = mqtt::sanitize_prefix(&m.topic_prefix);
        if self
            .mqtt
            .as_ref()
            .is_some_and(|c| c.config == config && c.prefix == prefix)
        {
            return;
        }
        // A prefix change moves the topics: clear the old ones.
        if let Some(c) = self.mqtt.as_mut()
            && c.connected
            && c.prefix != prefix
            && let Err(e) = c.publisher.clear_all()
        {
            warn!("MQTT publish failed: {e}");
        }
        self.stop_mqtt();
        self.start_mqtt(config, &prefix);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tower_bridge_core::model::{Backend, DeviceErrorKind, DeviceInfo};

    use super::*;

    fn info() -> DeviceInfo {
        DeviceInfo {
            backend: Backend::Hid,
            serial: "SN".into(),
            path: "p".into(),
            chip: None,
            bridges_connected: 1,
        }
    }

    fn ready() -> DeviceState {
        DeviceState::Ready {
            device: info(),
            sources: Vec::new(),
        }
    }

    fn gone() -> DeviceState {
        DeviceState::Error {
            kind: DeviceErrorKind::NotFound,
            message: String::new(),
        }
    }

    #[test]
    fn a_lost_bridge_is_reported_after_a_minute_and_its_return_too() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        assert!(!p.device(&ready(), t0));
        assert!(!p.device(&gone(), t0));
        assert!(!p.tick(t0 + Duration::from_secs(30), true), "too early");
        assert!(p.tick(t0 + DEVICE_LOST_AFTER, true));
        assert!(!p.tick(t0 + DEVICE_LOST_AFTER * 2, true), "once");
        assert!(p.device(&ready(), t0 + DEVICE_LOST_AFTER * 3), "back");
    }

    #[test]
    fn the_return_is_silent_when_the_loss_was_not_announced() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.device(&ready(), t0);
        p.device(&gone(), t0);
        assert!(!p.tick(t0 + DEVICE_LOST_AFTER, false), "notifications off");
        assert!(!p.device(&ready(), t0 + DEVICE_LOST_AFTER * 2));
    }

    #[test]
    fn missing_sensors_are_not_a_usb_disconnect() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.device(&ready(), t0);
        p.device(&DeviceState::NoModule { device: info() }, t0);
        assert!(!p.tick(t0 + DEVICE_LOST_AFTER * 2, true));
        assert!(!p.device(&ready(), t0 + DEVICE_LOST_AFTER * 3));
    }

    #[test]
    fn a_short_blip_is_not_reported() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.device(&ready(), t0);
        p.device(&gone(), t0);
        p.device(&ready(), t0 + Duration::from_secs(5));
        assert!(!p.tick(t0 + DEVICE_LOST_AFTER * 2, true));
    }

    #[test]
    fn nothing_is_reported_before_the_bridge_ever_worked() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.device(&gone(), t0);
        assert!(!p.tick(t0 + DEVICE_LOST_AFTER * 2, true));
    }
}
