//! Persistent user settings (`settings.json` in the data directory).

use std::path::Path;

use serde::{Deserialize, Serialize};
use tower_bridge_core::alerts::AlertRules;
use tower_bridge_core::bus::discovery::BackendPref;
use tower_bridge_core::climate::ChannelMode;
use tower_bridge_core::model::{Metric, STANDARD_SEA_LEVEL_PA};
use tracing::warn;

/// 2: chart colours differ per metric by default.
pub const SETTINGS_VERSION: u32 = 6;

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

/// Decimal separator of displayed numbers; `Auto` follows the language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecimalSeparator {
    #[default]
    Auto,
    Point,
    Comma,
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

/// Publishing to the user's MQTT broker (Mosquitto, Home Assistant, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct MqttSettings {
    pub enabled: bool,
    /// The broker's host name or IP address.
    pub host: String,
    pub port: u16,
    /// TLS, checked against the system's root certificates.
    pub tls: bool,
    pub username: String,
    pub password: String,
    /// Chosen once (`tower-bridge-` and 8 hex digits) when empty.
    pub client_id: String,
    /// Topics are `<prefix>/temperature/<channel>`, …
    pub topic_prefix: String,
    /// Accept Relay Module commands on `<prefix>/relay/<channel>/set` and `/pulse`.
    pub allow_control: bool,
}

impl Default for MqttSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 1883,
            tls: false,
            username: String::new(),
            password: String::new(),
            client_id: String::new(),
            topic_prefix: tower_bridge_core::mqtt::DEFAULT_PREFIX.into(),
            allow_control: false,
        }
    }
}

impl MqttSettings {
    /// How to connect, once a host is set.
    pub fn client_config(&self) -> Option<tower_bridge_core::mqtt::ClientConfig> {
        (!self.host.is_empty()).then(|| tower_bridge_core::mqtt::ClientConfig {
            host: self.host.clone(),
            port: self.port,
            tls: self.tls,
            credentials: (!self.username.is_empty())
                .then(|| (self.username.clone(), self.password.clone())),
            client_id: self.client_id.clone(),
        })
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

impl ChartColor {
    /// In the picker's order.
    pub const ALL: [ChartColor; 8] = [
        ChartColor::Blue,
        ChartColor::Green,
        ChartColor::Pink,
        ChartColor::Gold,
        ChartColor::Violet,
        ChartColor::Teal,
        ChartColor::Orange,
        ChartColor::Graphite,
    ];

    /// The first of `preferred` (then of the palette) not in `taken`, so a
    /// default never repeats a colour another channel already shows.
    fn unused(taken: &[ChartColor], preferred: &[ChartColor]) -> ChartColor {
        preferred
            .iter()
            .chain(Self::ALL.iter())
            .copied()
            .find(|c| !taken.contains(c))
            .unwrap_or_default()
    }
}

/// I2C channels with their own line colour.
pub const CHART_CHANNELS: usize = 2;

/// How one metric's charts look on the dashboard and in the history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ChartStyle {
    /// Line colour of each I2C channel (index = channel).
    pub colors: Vec<ChartColor>,
    /// Horizontal lines at the minimum, average and maximum of the window.
    pub show_min: bool,
    pub show_avg: bool,
    pub show_max: bool,
    /// I2C channels whose lines the user hid; never all of them.
    pub hidden_channels: Vec<u8>,
    /// Settings version 2 and older had one colour, now channel 0's.
    #[serde(skip_serializing)]
    color: Option<ChartColor>,
}

impl Default for ChartStyle {
    /// Without colours: [`ChartStyles`] fills in each metric's own.
    fn default() -> Self {
        Self {
            colors: Vec::new(),
            show_min: false,
            show_avg: true,
            show_max: false,
            hidden_channels: Vec::new(),
            color: None,
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
    pub co2: ChartStyle,
    pub voc: ChartStyle,
}

impl Default for ChartStyles {
    /// A different colour per metric from the start, and a contrasting one
    /// (checked for colour blindness in both themes) for the second channel.
    fn default() -> Self {
        let colored = |a, b| ChartStyle {
            colors: vec![a, b],
            ..ChartStyle::default()
        };
        Self {
            temperature: colored(ChartColor::Orange, ChartColor::Teal),
            humidity: colored(ChartColor::Blue, ChartColor::Pink),
            illuminance: colored(ChartColor::Gold, ChartColor::Violet),
            pressure: colored(ChartColor::Violet, ChartColor::Green),
            co2: colored(ChartColor::Green, ChartColor::Pink),
            voc: colored(ChartColor::Teal, ChartColor::Orange),
        }
    }
}

impl ChartStyles {
    fn each(&mut self) -> [(&mut ChartStyle, ChartStyle); 6] {
        let d = Self::default();
        [
            (&mut self.temperature, d.temperature),
            (&mut self.humidity, d.humidity),
            (&mut self.illuminance, d.illuminance),
            (&mut self.pressure, d.pressure),
            (&mut self.co2, d.co2),
            (&mut self.voc, d.voc),
        ]
    }

    /// Bring a file of an earlier version up to date. Version 1 defaulted
    /// every chart to blue (colours picked by the user stay); version 2 had
    /// one colour per metric, which becomes channel 0's.
    fn upgrade(&mut self, version: u32) {
        for (style, default) in self.each() {
            if let Some(mut color) = style.color.take() {
                if version < 2 && color == ChartColor::Blue {
                    color = default.colors[0];
                }
                let second = ChartColor::unused(&[color], &[default.colors[1], default.colors[0]]);
                style.colors = vec![color, second];
            }
        }
    }

    /// A colour for every channel (added defaults differ from the others),
    /// and at least one line left to see.
    fn fill_colors(&mut self) {
        for (style, default) in self.each() {
            // Taken over by `upgrade`; never kept around.
            style.color = None;
            while style.colors.len() < CHART_CHANNELS {
                let i = style.colors.len();
                let color = ChartColor::unused(&style.colors, &[default.colors[i]]);
                style.colors.push(color);
            }
            style.colors.truncate(CHART_CHANNELS);
            let hidden = &mut style.hidden_channels;
            hidden.retain(|c| usize::from(*c) < CHART_CHANNELS);
            hidden.sort_unstable();
            hidden.dedup();
            if hidden.len() == CHART_CHANNELS {
                hidden.clear();
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Settings {
    pub version: u32,
    pub theme: ThemePref,
    pub language: LanguagePref,
    pub date_format: DateFormat,
    pub time_format: TimeFormat,
    pub decimal_separator: DecimalSeparator,
    pub temperature_unit: TemperatureUnit,
    pub sample_interval_ms: u64,
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
            decimal_separator: DecimalSeparator::Auto,
            temperature_unit: TemperatureUnit::Celsius,
            sample_interval_ms: 20_000,
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
        self.sample_interval_ms = self.sample_interval_ms.clamp(2_000, 3_600_000);
        self.retention_days = self.retention_days.clamp(1, 3650);
        if !(80_000.0..=110_000.0).contains(&self.sea_level_pressure_pa) {
            self.sea_level_pressure_pa = STANDARD_SEA_LEVEL_PA;
        }
        let mqtt = &mut self.mqtt;
        mqtt.topic_prefix = tower_bridge_core::mqtt::sanitize_prefix(&mqtt.topic_prefix);
        mqtt.host = mqtt.host.trim().to_string();
        if mqtt.port == 0 {
            mqtt.port = if mqtt.tls { 8883 } else { 1883 };
        }
        mqtt.client_id = mqtt.client_id.trim().to_string();
        if mqtt.client_id.is_empty() {
            // Brokers may limit client IDs to 23 characters.
            mqtt.client_id = format!("tower-bridge-{}", &generate_token()[..8]);
        }
        if self.api.port == 0 {
            self.api.port = 8180;
        }
        self.charts.fill_colors();
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
            let rule = self.alerts.get_mut(metric);
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

    /// Load settings. Values that do not fit (for example written by a newer
    /// build) fall back to their defaults one by one, keeping the rest; a file
    /// that is not JSON at all is moved aside. Returns whether this is the
    /// first run (no file yet).
    pub fn load(path: &Path) -> (Self, bool) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default().validated(), true);
            }
            Err(e) => {
                warn!("cannot read the settings ({e}); using defaults");
                backup(path, false);
                return (Self::default().validated(), false);
            }
        };
        match Self::parse(&text) {
            Parsed::Complete(s) => (s, false),
            Parsed::Partial(s, e) => {
                warn!("some settings are invalid ({e}); keeping the valid ones");
                backup(path, true);
                (s, false)
            }
            Parsed::NotJson(e) => {
                warn!("settings file is not valid JSON ({e}); using defaults");
                backup(path, false);
                (Self::default().validated(), false)
            }
        }
    }

    /// The settings in the file and what is wrong with it, if anything: a
    /// file that is not JSON, or values that did not fit and read as
    /// defaults. A missing file is not wrong. The file is not touched.
    pub fn read_checked(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (Self::default().validated(), None)
            }
            Err(e) => (Self::default().validated(), Some(e.to_string())),
            Ok(text) => match Self::parse(&text) {
                Parsed::Complete(s) => (s, None),
                Parsed::Partial(s, why) => (s, Some(why)),
                Parsed::NotJson(why) => (
                    Self::default().validated(),
                    Some(format!("not JSON: {why}")),
                ),
            },
        }
    }

    /// The settings in the file, without touching it: for another process
    /// (the `--mcp-stdio` bridge) next to the running app.
    pub fn read(path: &Path) -> Self {
        match std::fs::read_to_string(path).map(|text| Self::parse(&text)) {
            Ok(Parsed::Complete(s) | Parsed::Partial(s, _)) => s,
            _ => Self::default().validated(),
        }
    }

    fn parse(text: &str) -> Parsed {
        let mut v = match serde_json::from_str::<serde_json::Value>(text) {
            Ok(v) => v,
            Err(e) => return Parsed::NotJson(e.to_string()),
        };
        snake_case_keys(&mut v);
        // Files from before versioning are version 1.
        if let Some(map) = v.as_object_mut() {
            map.entry("version").or_insert(serde_json::json!(1));
        }
        if v["version"].as_u64().is_some_and(|n| n < 4) {
            upgrade_mqtt_v3(&mut v);
        }
        if v["version"].as_u64().is_some_and(|n| n < 6) {
            upgrade_ms_v5(&mut v);
        }
        let (mut s, problem) = match serde_json::from_value::<Settings>(v.clone()) {
            Ok(s) => (s, None),
            Err(e) => (lenient(&v), Some(e.to_string())),
        };
        if s.version < 3 {
            s.charts.upgrade(s.version);
        }
        // The default went from 10 s to 20 s; a chosen interval stays.
        if s.version < 5 && s.sample_interval_ms == 10_000 {
            s.sample_interval_ms = 20_000;
        }
        match problem {
            None => Parsed::Complete(s.validated()),
            Some(e) => Parsed::Partial(s.validated(), e),
        }
    }

    /// Save atomically (write a temporary file, flush it to the disk, then
    /// rename), readable by the user only: it holds the MQTT password and the
    /// API token. Each save has its own temporary file, so concurrent saves
    /// cannot mix.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let n = SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("json.tmp-{}-{n}", std::process::id()));
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(text.as_bytes())?;
        // Without it, a power cut right after the rename can leave an empty
        // file, and the next start would fall back to the defaults.
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    }
}

/// What a settings file held.
enum Parsed {
    Complete(Settings),
    /// Some values did not fit and are defaults now (the error says why).
    Partial(Settings, String),
    NotJson(String),
}

/// Version 5 and older kept durations in seconds (`sample_interval_s`,
/// `alerts.<metric>.cooldown_s`); every duration in JSON is milliseconds now
/// (`_ms`).
fn upgrade_ms_v5(v: &mut serde_json::Value) {
    let Some(map) = v.as_object_mut() else {
        return;
    };
    let ms = |s: &serde_json::Value| {
        s.as_u64()
            .map(|s| serde_json::json!(s.saturating_mul(1000)))
    };
    if let Some(s) = map.remove("sample_interval_s")
        && let Some(ms) = ms(&s)
    {
        map.insert("sample_interval_ms".into(), ms);
    }
    if let Some(alerts) = map
        .get_mut("alerts")
        .and_then(serde_json::Value::as_object_mut)
    {
        for rule in alerts
            .values_mut()
            .filter_map(serde_json::Value::as_object_mut)
        {
            if let Some(s) = rule.remove("cooldown_s")
                && let Some(ms) = ms(&s)
            {
                rule.insert("cooldown_ms".into(), ms);
            }
        }
    }
}

/// Version 3 and older could run a built-in MQTT broker (`port`,
/// `allow_lan`, `username`, `password` were its listener) or connect to an
/// external one (`mode: "external"` with an `external` object). Only the
/// external client is left: its settings move up, and a file that used the
/// built-in broker turns MQTT off until a broker is set.
fn upgrade_mqtt_v3(v: &mut serde_json::Value) {
    let Some(m) = v.get_mut("mqtt").and_then(serde_json::Value::as_object_mut) else {
        return;
    };
    let external = m.get("mode").and_then(serde_json::Value::as_str) == Some("external");
    let client = m.remove("external");
    for key in ["mode", "port", "allow_lan", "username", "password"] {
        m.remove(key);
    }
    // The client fields stay either way (a host typed in, the client ID).
    if let Some(serde_json::Value::Object(fields)) = client {
        m.extend(fields);
    }
    if !external {
        if m.get("enabled") == Some(&serde_json::Value::Bool(true)) {
            warn!("the built-in MQTT broker was removed; MQTT is off until a broker is set");
        }
        m.insert("enabled".into(), serde_json::Value::Bool(false));
    }
}

/// Keep a copy of a settings file that cannot be used as it is: moved aside,
/// or copied when the valid part is kept and saved over it.
fn backup(path: &Path, copy: bool) {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let target = path.with_extension(format!("json.bak-{stamp}"));
    let _ = if copy {
        std::fs::copy(path, &target).map(|_| ())
    } else {
        std::fs::rename(path, &target)
    };
}

/// The defaults with every value from `v` that fits, looking two levels deep
/// (a bad chart colour costs only that chart's style).
fn lenient(v: &serde_json::Value) -> Settings {
    let mut root = serde_json::to_value(Settings::default()).unwrap_or_default();
    merge_valid(&mut root, &mut Vec::new(), v, 2);
    serde_json::from_value(root).unwrap_or_default()
}

fn merge_valid(
    root: &mut serde_json::Value,
    path: &mut Vec<String>,
    incoming: &serde_json::Value,
    depth: usize,
) {
    let Some(map) = incoming.as_object() else {
        return;
    };
    for (key, value) in map {
        path.push(key.clone());
        let mut candidate = root.clone();
        set_at(&mut candidate, path, value.clone());
        if serde_json::from_value::<Settings>(candidate.clone()).is_ok() {
            *root = candidate;
        } else if depth > 0 && value.is_object() {
            merge_valid(root, path, value, depth - 1);
        } else {
            warn!("setting {} is invalid; using the default", path.join("."));
        }
        path.pop();
    }
}

fn set_at(v: &mut serde_json::Value, path: &[String], new: serde_json::Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut node = v;
    for key in parents {
        if !node.get(key).is_some_and(serde_json::Value::is_object) {
            node[key] = serde_json::json!({});
        }
        node = &mut node[key];
    }
    node[last] = new;
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
            serde_json::from_str(r#"{"sample_interval_ms": 30000, "mqtt": {"enabled": true}}"#)?;
        assert_eq!(s.sample_interval_ms, 30_000);
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
        assert_eq!(s.sample_interval_ms, 30_000);
        assert_eq!(s.date_format, DateFormat::DmySlash);
        assert_eq!(s.backend, BackendPref::I2cDev);
        assert_eq!(s.mqtt.topic_prefix, "home/tower");
        assert!(!s.mqtt.enabled, "it used the removed built-in broker");
        assert_eq!(s.alerts.temperature.cooldown_ms, 600_000);
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
        assert_eq!(s.charts.temperature.colors[0], ChartColor::Orange);
        assert_eq!(s.charts.humidity.colors[0], ChartColor::Blue);
        assert_eq!(
            s.charts.illuminance.colors[0],
            ChartColor::Green,
            "picked by the user"
        );
        assert_eq!(s.charts.pressure.colors[0], ChartColor::Violet);
        assert_eq!(s.version, SETTINGS_VERSION);
        Ok(())
    }

    #[test]
    fn version_2_colour_becomes_channel_0s() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("tb-settings-v2-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"version": 2, "charts": {"temperature": {"color": "pink", "show_max": true}}}"#,
        )?;
        let (s, _) = Settings::load(&path);
        let _ = std::fs::remove_dir_all(&dir);
        let t = &s.charts.temperature;
        assert_eq!(t.colors, vec![ChartColor::Pink, ChartColor::Teal]);
        assert!(t.show_max);
        assert_eq!(
            s.charts.humidity.colors,
            vec![ChartColor::Blue, ChartColor::Pink]
        );
        let saved = serde_json::to_value(&s).map_err(std::io::Error::other)?;
        assert!(saved["charts"]["temperature"].get("color").is_none());
        assert_eq!(saved["charts"]["temperature"]["colors"][1], "teal");
        Ok(())
    }

    #[test]
    fn a_second_channel_never_repeats_the_first_ones_colour() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("tb-settings-v2b-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("settings.json");
        // Humidity's channel 1 default is pink, which the user picked for it.
        std::fs::write(
            &path,
            r#"{"version": 2, "charts": {"humidity": {"color": "pink"},
                "pressure": {"colors": ["green"]}}}"#,
        )?;
        let (s, _) = Settings::load(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            s.charts.humidity.colors,
            vec![ChartColor::Pink, ChartColor::Blue]
        );
        assert_eq!(
            s.charts.pressure.colors,
            vec![ChartColor::Green, ChartColor::Blue],
            "the default green is taken; the first free colour"
        );
        Ok(())
    }

    /// Load `json` from a fresh settings file.
    fn load_json(name: &str, json: &str) -> std::io::Result<(Settings, std::path::PathBuf)> {
        let dir = std::env::temp_dir().join(format!("tb-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("settings.json");
        std::fs::write(&path, json)?;
        Ok((Settings::load(&path).0, dir))
    }

    #[test]
    fn a_file_without_a_version_is_upgraded_like_version_1() -> std::io::Result<()> {
        let (s, dir) = load_json(
            "noversion",
            r#"{"charts": {"temperature": {"color": "pink"}}}"#,
        )?;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            s.charts.temperature.colors,
            vec![ChartColor::Pink, ChartColor::Teal]
        );
        assert_eq!(s.version, SETTINGS_VERSION);
        assert!(
            s.charts.temperature.color.is_none(),
            "the old field is not kept"
        );
        Ok(())
    }

    #[test]
    fn a_partial_chart_style_keeps_the_metric_colours() -> std::io::Result<()> {
        let (s, dir) = load_json(
            "partial",
            r#"{"version": 3, "charts": {"temperature": {"show_max": true}}}"#,
        )?;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            s.charts.temperature.colors,
            vec![ChartColor::Orange, ChartColor::Teal]
        );
        assert!(s.charts.temperature.show_max);
        Ok(())
    }

    #[test]
    fn an_unknown_value_keeps_the_other_settings() -> std::io::Result<()> {
        // As written by a newer build: an unknown colour and language.
        let (s, dir) = load_json(
            "unknown",
            r#"{"version": 3, "language": "fi", "retention_days": 30,
                "api": {"token": "0123456789abcdef0123456789abcdef", "port": 9000},
                "charts": {"humidity": {"colors": ["ultraviolet", "pink"]},
                           "pressure": {"colors": ["teal", "gold"]}}}"#,
        )?;
        let backups = std::fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .count();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(s.retention_days, 30);
        assert_eq!(s.api.token, "0123456789abcdef0123456789abcdef");
        assert_eq!(s.api.port, 9000);
        assert_eq!(
            s.language,
            LanguagePref::System,
            "the unknown one falls back"
        );
        assert_eq!(
            s.charts.pressure.colors,
            vec![ChartColor::Teal, ChartColor::Gold]
        );
        assert_eq!(
            s.charts.humidity.colors,
            vec![ChartColor::Blue, ChartColor::Pink]
        );
        assert_eq!(backups, 1, "the original is kept");
        Ok(())
    }

    #[test]
    fn a_file_that_is_not_json_is_moved_aside() -> std::io::Result<()> {
        let (s, dir) = load_json("garbage", "{not json")?;
        let names: Vec<String> = std::fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(names.iter().all(|n| n.contains(".bak-")), "{names:?}");
        assert_eq!(s.api.token.len(), 48, "defaults are validated");
        Ok(())
    }

    #[test]
    fn the_first_run_has_a_token() {
        let path = std::env::temp_dir().join(format!(
            "tb-settings-none-{}/settings.json",
            std::process::id()
        ));
        let (s, first) = Settings::load(&path);
        assert!(first);
        assert_eq!(s.api.token.len(), 48);
    }

    #[test]
    fn camel_case_keys_become_snake_case() {
        let mut v = serde_json::json!({"i2cChannel": "auto", "mqtt": {"allowLan": true, "topic_prefix": "x"},
            "list": [{"pulseDurationMs": 1}]});
        snake_case_keys(&mut v);
        assert_eq!(v["i2c_channel"], "auto");
        assert_eq!(v["mqtt"]["allow_lan"], true);
        assert_eq!(v["mqtt"]["topic_prefix"], "x");
        assert_eq!(v["list"][0]["pulse_duration_ms"], 1);
    }

    #[test]
    fn the_broker_client_gets_an_id_once() {
        let s = Settings::default().validated();
        let id = s.mqtt.client_id.clone();
        assert!(id.starts_with("tower-bridge-") && id.len() <= 23, "{id}");
        assert_eq!(s.clone().validated().mqtt.client_id, id);
        assert!(s.mqtt.client_config().is_none(), "no host yet");
        let mut t = s;
        t.mqtt.host = " broker.local ".into();
        t.mqtt.tls = true;
        t.mqtt.port = 0;
        t.mqtt.username = "ha".into();
        let t = t.validated();
        let cfg = t.mqtt.client_config();
        assert_eq!(
            cfg.map(|c| (c.host, c.port, c.tls, c.credentials.map(|(u, _)| u))),
            Some(("broker.local".into(), 8883, true, Some("ha".into())))
        );
    }

    #[test]
    fn version_3_external_broker_settings_move_up() -> std::io::Result<()> {
        let (s, dir) = load_json(
            "v3ext",
            r#"{"version": 3, "mqtt": {"enabled": true, "mode": "external", "port": 1999,
                "allow_lan": true, "username": "listener", "topic_prefix": "home",
                "external": {"host": "ha.local", "port": 8883, "tls": true,
                             "username": "ha", "password": "pw", "client_id": "tb-1"}}}"#,
        )?;
        let _ = std::fs::remove_dir_all(&dir);
        let m = &s.mqtt;
        assert!(m.enabled);
        assert_eq!(
            (
                m.host.as_str(),
                m.port,
                m.tls,
                m.username.as_str(),
                m.client_id.as_str()
            ),
            ("ha.local", 8883, true, "ha", "tb-1")
        );
        assert_eq!(m.topic_prefix, "home");
        Ok(())
    }

    #[test]
    fn the_removed_built_in_broker_turns_mqtt_off() -> std::io::Result<()> {
        let (s, dir) = load_json(
            "v3builtin",
            r#"{"version": 3, "mqtt": {"enabled": true, "mode": "builtin", "port": 1999,
                "username": "listener", "password": "x", "allow_control": true,
                "external": {"host": "nas.local", "port": 1883, "client_id": "tower-bridge-0badf00d"}}}"#,
        )?;
        let _ = std::fs::remove_dir_all(&dir);
        let m = &s.mqtt;
        assert!(!m.enabled);
        assert_eq!(
            (m.host.as_str(), m.port, m.username.as_str()),
            ("nas.local", 1883, ""),
            "the broker's login, not the built-in listener's"
        );
        assert_eq!(m.client_id, "tower-bridge-0badf00d");
        assert!(m.allow_control, "unrelated settings stay");
        Ok(())
    }

    #[test]
    fn defaults_are_consistent() {
        let d = ChartStyles::default();
        for style in [&d.temperature, &d.humidity, &d.illuminance, &d.pressure] {
            assert_eq!(style.colors.len(), CHART_CHANNELS);
            assert_ne!(style.colors[0], style.colors[1]);
        }
        let once = Settings::default().validated();
        let twice = once.clone().validated();
        assert_eq!(once, twice, "validation is idempotent");
    }

    #[test]
    fn the_old_default_interval_becomes_the_new_one() -> std::io::Result<()> {
        let (s, dir) = load_json("v4interval", r#"{"version": 4, "sample_interval_s": 10}"#)?;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(s.sample_interval_ms, 20_000);
        let (s, dir) = load_json("v4interval30", r#"{"version": 4, "sample_interval_s": 30}"#)?;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(s.sample_interval_ms, 30_000, "chosen by the user");
        let (s, dir) = load_json("v5interval", r#"{"version": 5, "sample_interval_s": 10}"#)?;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(s.sample_interval_ms, 10_000, "chosen after the change");
        Ok(())
    }

    #[test]
    fn version_5_durations_become_milliseconds() -> std::io::Result<()> {
        let (s, dir) = load_json(
            "v5ms",
            r#"{"version": 5, "sample_interval_s": 60,
                "alerts": {"co2": {"enabled": true, "max": 1000, "cooldown_s": 1800}}}"#,
        )?;
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(s.sample_interval_ms, 60_000);
        assert_eq!(s.alerts.co2.cooldown_ms, 1_800_000);
        assert_eq!(s.alerts.co2.max, Some(1000.0), "the rest of the rule stays");
        assert_eq!(s.version, SETTINGS_VERSION);
        Ok(())
    }

    #[test]
    fn a_chart_keeps_at_least_one_line() {
        let mut s = Settings::default();
        s.charts.temperature.hidden_channels = vec![1, 1, 7];
        s.charts.humidity.hidden_channels = vec![1, 0];
        let s = s.validated();
        assert_eq!(s.charts.temperature.hidden_channels, vec![1]);
        assert!(s.charts.humidity.hidden_channels.is_empty());
    }

    #[test]
    fn network_services_are_off_by_default() {
        // Also what an older settings file without these sections loads as.
        for s in [
            Settings::default().validated(),
            Settings::load(std::path::Path::new("/nonexistent/settings.json")).0,
        ] {
            assert!(!s.mqtt.enabled, "MQTT");
            assert!(!s.mqtt.allow_control);
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
            sample_interval_ms: 0,
            ..Settings::default()
        };
        s.alerts.temperature.min = Some(30.0);
        s.alerts.temperature.max = Some(10.0);
        let s = s.validated();
        assert_eq!(s.sample_interval_ms, 2_000);
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)?.permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "secrets readable by the user only");
        }
        let (loaded, first_run) = Settings::load(&path);
        assert!(!first_run);
        assert_eq!(loaded, s);
        std::fs::write(&path, "{not json")?;
        let (loaded, _) = Settings::load(&path);
        assert_eq!(loaded.api.port, 8180);
        assert_eq!(
            loaded.sample_interval_ms,
            Settings::default().sample_interval_ms
        );
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
