//! What the service reports to its front end: the desktop app turns it into
//! webview events, the tray and notifications; the CLI into its terminal UI
//! or log lines. Every method has an empty default.
//!
//! The methods run on the service's threads, after the state was updated;
//! they must return quickly and must not call back into the service and wait
//! for it.

use tower_bridge_core::alerts::{ActiveAlert, AlertEvent};
use tower_bridge_core::gps::GpsInfo;
use tower_bridge_core::model::{DeviceState, Reading};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;

use crate::settings::Settings;
use crate::state::{ApiState, MqttState, ServiceState};

#[allow(unused_variables)]
pub trait Observer: Send + Sync + 'static {
    /// Something a summary shows may have changed (device, readings,
    /// alerts, MQTT); also every 5 s, so stale readings get noticed.
    fn refresh(&self, state: &ServiceState) {}
    fn reading(&self, state: &ServiceState, reading: &Reading) {}
    fn device(&self, state: &ServiceState, device: &DeviceState) {}
    fn relay(&self, state: &ServiceState, relay: Option<&RelayInfo>) {}
    fn nfc(&self, state: &ServiceState, nfc: Option<&NfcInfo>) {}
    fn gps(&self, state: &ServiceState, gps: Option<&GpsInfo>) {}
    /// An alert started or ended; `event.notify` says whether to tell the
    /// user (not within the repeat interval).
    fn alert(&self, state: &ServiceState, event: &AlertEvent) {}
    fn alerts_active(&self, state: &ServiceState, active: &[ActiveAlert]) {}
    fn mqtt(&self, state: &ServiceState, mqtt: &MqttState) {}
    fn api(&self, state: &ServiceState, api: &ApiState) {}
    /// The bridge has been gone for a minute (only with
    /// `notify_device_lost` on).
    fn bridge_lost(&self, state: &ServiceState) {}
    /// It is back after `bridge_lost` was reported.
    fn bridge_back(&self, state: &ServiceState) {}
    fn settings_changed(&self, state: &ServiceState, old: &Settings, new: &Settings) {}
}

/// Reports to nobody.
pub struct Silent;

impl Observer for Silent {}
