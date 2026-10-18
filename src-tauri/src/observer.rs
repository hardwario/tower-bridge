//! The service's events for the desktop: webview events for the dashboard,
//! the tray, and native notifications.

use tauri::{AppHandle, Emitter};
use tower_bridge_core::alerts::{ActiveAlert, AlertEvent, AlertKind, Bound};
use tower_bridge_core::gps::GpsInfo;
use tower_bridge_core::model::{DeviceState, Reading, source_label};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_service::{ApiState, MqttState, Observer, ServiceState, Settings};

use crate::format::{self, SettingsExt};
use crate::i18n::{Lang, fill};
use crate::notify;
use crate::tray::TrayController;

pub struct AppObserver {
    pub app: AppHandle,
    pub tray: TrayController,
}

impl Observer for AppObserver {
    fn refresh(&self, state: &ServiceState) {
        let settings = state.settings();
        let device = state.device.read().clone();
        let latest = state.latest.read().clone();
        let alerts = state.alerts.read().clone();
        let mqtt = state.mqtt.read().clone();
        self.tray
            .update(&device, &latest, &settings, &alerts, &mqtt);
    }

    fn reading(&self, _: &ServiceState, reading: &Reading) {
        let _ = self.app.emit("reading", reading);
    }

    fn device(&self, _: &ServiceState, device: &DeviceState) {
        let _ = self.app.emit("device-state", device);
    }

    fn relay(&self, _: &ServiceState, relay: Option<&RelayInfo>) {
        let _ = self.app.emit("relay-state", relay);
    }

    fn nfc(&self, _: &ServiceState, nfc: Option<&NfcInfo>) {
        let _ = self.app.emit("nfc-state", nfc);
    }

    fn gps(&self, _: &ServiceState, gps: Option<&GpsInfo>) {
        let _ = self.app.emit("gps-state", gps);
    }

    fn alert(&self, state: &ServiceState, ev: &AlertEvent) {
        let _ = self.app.emit("alert", ev);
        if !ev.notify {
            return;
        }
        let settings = state.settings();
        let unit = settings.temperature_unit;
        let lang = Lang::effective(settings.language);
        let tr = lang.tr();
        let dec = settings.decimal(lang);
        let value = format::full(ev.metric, ev.value, unit, dec);
        let limit = format::full(ev.metric, ev.threshold, unit, dec);
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
        // With the metric measured on several channels, say which one.
        let device = state.device.read().clone();
        let title = match source_label(device.sources(), ev.metric, ev.channel, tr.channel_n) {
            Some(source) => format!("{title} · {source}"),
            None => title,
        };
        notify::send(&self.app, &title, &body);
    }

    fn alerts_active(&self, _: &ServiceState, active: &[ActiveAlert]) {
        let _ = self.app.emit("alerts-active", active);
    }

    fn mqtt(&self, _: &ServiceState, mqtt: &MqttState) {
        let _ = self.app.emit("mqtt-state", mqtt);
    }

    fn api(&self, _: &ServiceState, api: &ApiState) {
        let _ = self.app.emit("api-state", api);
    }

    fn bridge_lost(&self, state: &ServiceState) {
        let tr = Lang::effective(state.settings().language).tr();
        notify::send(&self.app, tr.lost_title, tr.lost_body);
    }

    fn bridge_back(&self, state: &ServiceState) {
        let tr = Lang::effective(state.settings().language).tr();
        notify::send(&self.app, tr.reconnected_title, tr.reconnected_body);
    }

    fn settings_changed(&self, _: &ServiceState, old: &Settings, new: &Settings) {
        if old.theme != new.theme {
            crate::window::apply_theme(&self.app, new.theme);
        }
        let _ = self.app.emit("settings-changed", new);
    }
}
