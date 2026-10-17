//! Persistent user settings (`settings.json` in the data directory).

use std::path::Path;

use serde::{Deserialize, Serialize};
use tower_bridge_core::alerts::AlertRules;
use tower_bridge_core::bus::discovery::BackendPref;
use tower_bridge_core::climate::ChannelMode;
use tower_bridge_core::model::{Metric, STANDARD_SEA_LEVEL_PA};
use tracing::warn;

/// 2: chart colours differ per metric by default.
pub const SETTINGS_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemperatureUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

/// Colour scheme of the dashboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

/// User interface language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LanguagePref {
    #[default]
    System,
    En,
    Cs,
    Sk,
    De,
    Pl,
}

/// How dates are written; `Auto` follows the language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateFormat {
    #[default]
    Auto,
    /// 2026-10-17
    Iso,
    /// 17.10.2026
    Dmy,
    /// 17/10/2026
    #[serde(alias = "dmySlash")]
    DmySlash,
    /// 10/17/2026
    Mdy,
}

/// 12- or 24-hour clock; `Auto` follows the language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeFormat {
    #[default]
    Auto,
    H24,
    H12,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct MqttSettings {
    pub enabled: bool,
    pub port: u16,
    /// Listen on all interfaces instead of localhost only.
    pub allow_lan: bool,
    /// Topics are `<prefix>/temperature`, `<prefix>/humidity`, …
    pub topic_prefix: String,
    pub username: String,
    pub password: String,
    /// Accept Relay Module commands on `<prefix>/relay/set` and `/pulse`.
    pub allow_control: bool,
}

impl Default for MqttSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 1883,
            allow_lan: false,
            topic_prefix: tower_bridge_core::mqtt::DEFAULT_PREFIX.into(),
            username: String::new(),
            password: String::new(),
            allow_control: false,
        }
    }
}

/// Line colour of a chart, from the dashboard's palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartColor {
    #[default]
    Blue,
    Green,
    Pink,
    Gold,
    Violet,
    Teal,
    Orange,
    Graphite,
}

/// How one metric's charts look on the dashboard and in the history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ChartStyle {
    pub color: ChartColor,
    /// Horizontal lines at the minimum, average and maximum of the window.
    pub show_min: bool,
    pub show_avg: bool,
    pub show_max: bool,
}

impl Default for ChartStyle {
    fn default() -> Self {
        Self {
            color: ChartColor::Blue,
            show_min: false,
            show_avg: true,
            show_max: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartStyles {
    pub temperature: ChartStyle,
    pub humidity: ChartStyle,
    pub illuminance: ChartStyle,
    pub pressure: ChartStyle,
}

impl Default for ChartStyles {
    /// A different colour per metric from the start.
    fn default() -> Self {
        let colored = |color| ChartStyle {
            color,
            ..ChartStyle::default()
        };
        Self {
            temperature: colored(ChartColor::Orange),
            humidity: colored(ChartColor::Blue),
            illuminance: colored(ChartColor::Gold),
            pressure: colored(ChartColor::Violet),
        }
    }
}

impl ChartStyles {
    /// Version 1 defaulted every chart to blue; colours picked by the user stay.
    fn upgrade_from_v1(&mut self) {
        let defaults = Self::default();
        for (style, default) in [
            (&mut self.temperature, defaults.temperature.color),
            (&mut self.humidity, defaults.humidity.color),
            (&mut self.illuminance, defaults.illuminance.color),
            (&mut self.pressure, defaults.pressure.color),
        ] {
            if style.color == ChartColor::Blue {
                style.color = default;
            }
        }
    }
}

/// Relay Module options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RelaySettings {
    /// Length of a timed pulse (dashboard, default for API and MQTT).
    pub pulse_duration_ms: u64,
}

impl Default for RelaySettings {
    fn default() -> Self {
        Self {
            pulse_duration_ms: tower_bridge_core::relay::DEFAULT_PULSE.as_millis() as u64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ApiSettings {
    /// Run the local HTTP server (REST, metrics, MCP).
    pub enabled: bool,
    pub port: u16,
    /// Listen on all interfaces; the access token is then required.
    pub allow_lan: bool,
    /// Serve the MCP endpoint at /mcp.
    pub mcp: bool,
    /// Let API and MCP clients change alert limits and blink the LED.
    pub allow_control: bool,
    pub token: String,
}

impl Default for ApiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8180,
            allow_lan: false,
            mcp: false,
            allow_control: false,
            token: String::new(),
        }
    }
}

/// Random access token (48 hex characters).
pub fn generate_token() -> String {
    let mut buf = [0u8; 24];
    if getrandom::fill(&mut buf).is_err() {
        // Extremely unlikely; fall back to a time-seeded value.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (nanos >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(97);
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

impl MqttSettings {
    /// Settings that require a broker restart when changed.
    pub fn listener_key(&self) -> (u16, bool, &str, &str) {
        (self.port, self.allow_lan, &self.username, &self.password)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Settings {
    pub version: u32,
    pub theme: ThemePref,
    pub language: LanguagePref,
    pub date_format: DateFormat,
    pub time_format: TimeFormat,
    pub temperature_unit: TemperatureUnit,
    pub sample_interval_s: u64,
    /// Value shown next to the tray icon; `None` shows the icon only.
    pub tray_metric: Option<Metric>,
    pub retention_days: u32,
    pub sea_level_pressure_pa: f64,
    pub i2c_channel: ChannelMode,
    pub backend: BackendPref,
    pub alerts: AlertRules,
    pub notify_device_lost: bool,
    pub mqtt: MqttSettings,
    pub api: ApiSettings,
    pub relay: RelaySettings,
    pub charts: ChartStyles,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            theme: ThemePref::System,
            language: LanguagePref::System,
            date_format: DateFormat::Auto,
            time_format: TimeFormat::Auto,
            temperature_unit: TemperatureUnit::Celsius,
            sample_interval_s: 10,
            tray_metric: Some(Metric::Temperature),
            retention_days: 90,
            sea_level_pressure_pa: STANDARD_SEA_LEVEL_PA,
            i2c_channel: ChannelMode::Auto,
            backend: BackendPref::Auto,
            alerts: AlertRules::default(),
            notify_device_lost: true,
            mqtt: MqttSettings::default(),
            api: ApiSettings::default(),
            relay: RelaySettings::default(),
            charts: ChartStyles::default(),
        }
    }
}

impl Settings {
    /// Clamp values into supported ranges.
    pub fn validated(mut self) -> Self {
        self.version = SETTINGS_VERSION;
        self.sample_interval_s = self.sample_interval_s.clamp(2, 3600);
        self.retention_days = self.retention_days.clamp(1, 3650);
        if !(80_000.0..=110_000.0).contains(&self.sea_level_pressure_pa) {
            self.sea_level_pressure_pa = STANDARD_SEA_LEVEL_PA;
        }
        if self.mqtt.port == 0 {
            self.mqtt.port = 1883;
        }
        self.mqtt.topic_prefix = tower_bridge_core::mqtt::sanitize_prefix(&self.mqtt.topic_prefix);
        if self.api.port == 0 {
            self.api.port = 8180;
        }
        {
            use tower_bridge_core::relay::{MAX_PULSE, MIN_PULSE};
            self.relay.pulse_duration_ms = self
                .relay
                .pulse_duration_ms
                .clamp(MIN_PULSE.as_millis() as u64, MAX_PULSE.as_millis() as u64);
        }
        self.api.token = self.api.token.trim().to_string();
        if self.api.token.len() < 16 {
            self.api.token = generate_token();
        }
        for metric in Metric::ALL {
            let rule = match metric {
                Metric::Temperature => &mut self.alerts.temperature,
                Metric::Humidity => &mut self.alerts.humidity,
                Metric::Illuminance => &mut self.alerts.illuminance,
                Metric::Pressure => &mut self.alerts.pressure,
            };
            rule.hysteresis = rule.hysteresis.max(0.0);
            if let (Some(min), Some(max)) = (rule.min, rule.max)
                && min > max
            {
                rule.min = Some(max);
                rule.max = Some(min);
            }
        }
        self
    }

    /// Load settings; a corrupt file is moved aside and defaults are used.
    pub fn load(path: &Path) -> (Self, bool) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return (Self::default(), true),
        };
        let parsed = serde_json::from_str::<serde_json::Value>(&text).and_then(|mut v| {
            snake_case_keys(&mut v);
            serde_json::from_value::<Settings>(v)
        });
        match parsed {
            Ok(mut s) => {
                if s.version < 2 {
                    s.charts.upgrade_from_v1();
                }
                (s.validated(), false)
            }
            Err(e) => {
                warn!("settings file is invalid ({e}); using defaults");
                let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                let backup = path.with_extension(format!("json.bak-{stamp}"));
                let _ = std::fs::rename(path, backup);
                (Self::default(), false)
            }
        }
    }

    /// Save atomically (write a temporary file, then rename).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }
}

/// Settings files from earlier builds used camelCase keys.
fn snake_case_keys(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            let old = std::mem::take(map);
            for (k, mut child) in old {
                snake_case_keys(&mut child);
                let mut key = String::with_capacity(k.len() + 4);
                for c in k.chars() {
                    if c.is_ascii_uppercase() {
                        key.push('_');
                        key.push(c.to_ascii_lowercase());
                    } else {
                        key.push(c);
                    }
                }
                map.insert(key, child);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(snake_case_keys),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_json_uses_defaults() -> Result<(), serde_json::Error> {
        let s: Settings =
            serde_json::from_str(r#"{"sample_interval_s": 30, "mqtt": {"enabled": true}}"#)?;
        assert_eq!(s.sample_interval_s, 30);
        assert!(s.mqtt.enabled);
        assert_eq!(s.mqtt.port, 1883);
        assert_eq!(s.retention_days, 90);
        Ok(())
    }

    #[test]
    fn loads_camel_case_files_from_earlier_builds() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("tb-settings-camel-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"sampleIntervalS": 30, "dateFormat": "dmySlash", "backend": "i2c-dev",
                "mqtt": {"enabled": true, "topicPrefix": "home/tower", "allowLan": true},
                "alerts": {"temperature": {"enabled": true, "max": 25, "cooldownS": 600}}}"#,
        )?;
        let (s, first_run) = Settings::load(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!first_run);
        assert_eq!(s.sample_interval_s, 30);
        assert_eq!(s.date_format, DateFormat::DmySlash);
        assert_eq!(s.backend, BackendPref::I2cDev);
        assert_eq!(s.mqtt.topic_prefix, "home/tower");
        assert!(s.mqtt.allow_lan);
        assert_eq!(s.alerts.temperature.cooldown_s, 600);
        Ok(())
    }

    #[test]
    fn version_1_blue_charts_get_the_new_defaults() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("tb-settings-v1-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"version": 1, "charts": {"temperature": {"color": "blue"},
                "humidity": {"color": "blue"}, "illuminance": {"color": "green"},
                "pressure": {"color": "blue"}}}"#,
        )?;
        let (s, _) = Settings::load(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(s.charts.temperature.color, ChartColor::Orange);
        assert_eq!(s.charts.humidity.color, ChartColor::Blue);
        assert_eq!(
            s.charts.illuminance.color,
            ChartColor::Green,
            "picked by the user"
        );
        assert_eq!(s.charts.pressure.color, ChartColor::Violet);
        assert_eq!(s.version, SETTINGS_VERSION);
        Ok(())
    }

    #[test]
    fn network_services_are_off_by_default() {
        // Also what an older settings file without these sections loads as.
        for s in [
            Settings::default().validated(),
            Settings::load(std::path::Path::new("/nonexistent/settings.json")).0,
        ] {
            assert!(!s.mqtt.enabled, "MQTT broker");
            assert!(!s.mqtt.allow_lan);
            assert!(!s.api.enabled, "REST API server");
            assert!(!s.api.mcp, "MCP server");
            assert!(!s.api.allow_lan);
            assert!(!s.api.allow_control);
        }
        let partial: Settings =
            serde_json::from_str(r#"{"api": {"enabled": true}}"#).unwrap_or_default();
        assert!(
            partial.api.enabled && !partial.api.mcp,
            "enabling the API does not turn on MCP"
        );
    }

    #[test]
    fn token_is_generated_once() {
        let s = Settings::default().validated();
        assert_eq!(s.api.token.len(), 48);
        let again = s.clone().validated();
        assert_eq!(again.api.token, s.api.token);
        assert_ne!(Settings::default().validated().api.token, s.api.token);
    }

    #[test]
    fn validation_clamps() {
        let mut s = Settings {
            sample_interval_s: 0,
            ..Settings::default()
        };
        s.alerts.temperature.min = Some(30.0);
        s.alerts.temperature.max = Some(10.0);
        let s = s.validated();
        assert_eq!(s.sample_interval_s, 2);
        assert_eq!(s.alerts.temperature.min, Some(10.0));
    }

    #[test]
    fn save_and_load_roundtrip() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("tb-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let s = Settings {
            temperature_unit: TemperatureUnit::Fahrenheit,
            ..Settings::default()
        }
        .validated();
        s.save(&path)?;
        let (loaded, first_run) = Settings::load(&path);
        assert!(!first_run);
        assert_eq!(loaded, s);
        std::fs::write(&path, "{not json")?;
        let (loaded, _) = Settings::load(&path);
        assert_eq!(loaded.api.port, 8180);
        assert_eq!(
            loaded.sample_interval_s,
            Settings::default().sample_interval_s
        );
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
