//! Text output: values with their units, product names, times and colours,
//! and JSON.

use std::io::{IsTerminal, Write};

use chrono::{Local, TimeZone};
use serde::Serialize;
use tower_bridge_core::model::{
    Backend, DeviceErrorKind, DeviceState, Metric, ModuleKind, SensorSource, celsius_to_fahrenheit,
};
use tower_bridge_service::settings::TemperatureUnit;

use crate::cli::ColorMode;

/// Colours for text on stdout.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub color: bool,
}

impl Style {
    pub fn new(mode: ColorMode) -> Self {
        let color = match mode {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => {
                std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
            }
        };
        Self { color }
    }

    fn paint(self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    pub fn bold(self, s: &str) -> String {
        self.paint("1", s)
    }
    pub fn dim(self, s: &str) -> String {
        self.paint("2", s)
    }
    pub fn good(self, s: &str) -> String {
        self.paint("32", s)
    }
    pub fn bad(self, s: &str) -> String {
        self.paint("31", s)
    }
    pub fn warn(self, s: &str) -> String {
        self.paint("33", s)
    }
}

/// Print `value` as JSON on one line (pretty in a terminal).
pub fn json(value: &impl Serialize) -> anyhow::Result<()> {
    let text = if std::io::stdout().is_terminal() {
        serde_json::to_string_pretty(value)?
    } else {
        serde_json::to_string(value)?
    };
    let mut out = std::io::stdout().lock();
    writeln!(out, "{text}")?;
    Ok(())
}

pub fn metric_name(metric: Metric) -> &'static str {
    match metric {
        Metric::Temperature => "Temperature",
        Metric::Humidity => "Humidity",
        Metric::Illuminance => "Illuminance",
        Metric::Pressure => "Pressure",
        Metric::Co2 => "CO₂",
        Metric::Voc => "VOC",
    }
}

/// An SI value in the unit it is shown in.
pub fn display(metric: Metric, v: f64, unit: TemperatureUnit) -> f64 {
    match metric {
        Metric::Temperature if unit == TemperatureUnit::Fahrenheit => celsius_to_fahrenheit(v),
        Metric::Pressure => v / 100.0,
        _ => v,
    }
}

pub fn unit_label(metric: Metric, unit: TemperatureUnit) -> &'static str {
    match metric {
        Metric::Temperature if unit == TemperatureUnit::Fahrenheit => "°F",
        Metric::Temperature => "°C",
        Metric::Humidity => "%",
        Metric::Illuminance => "lx",
        Metric::Pressure => "hPa",
        Metric::Co2 => "ppm",
        Metric::Voc => "ppb",
    }
}

pub fn decimals(metric: Metric) -> usize {
    match metric {
        Metric::Temperature | Metric::Humidity | Metric::Pressure => 1,
        Metric::Illuminance | Metric::Co2 | Metric::Voc => 0,
    }
}

/// The number alone, as the dashboard shows it: "23.4", never "-0.0".
pub fn number(metric: Metric, v: f64, unit: TemperatureUnit) -> String {
    let s = format!("{:.*}", decimals(metric), display(metric, v, unit));
    match s.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => rest.to_string(),
        _ => s,
    }
}

/// "23.4 °C", "1013.2 hPa"; "-" without a value.
pub fn value(metric: Metric, v: Option<f64>, unit: TemperatureUnit) -> String {
    match v {
        Some(v) => format!("{} {}", number(metric, v, unit), unit_label(metric, unit)),
        None => "-".into(),
    }
}

/// The full product name: "TOWER Climate Module R2", "TOWER Humidity Tag".
pub fn product(kind: ModuleKind) -> String {
    format!("TOWER {}", kind.name())
}

/// The modules on `channel`, A to Z.
pub fn modules_on(sources: &[SensorSource], channel: u8) -> Vec<String> {
    let mut names: Vec<String> = tower_bridge_core::model::modules(sources)
        .into_iter()
        .filter(|(c, _)| *c == channel)
        .map(|(_, m)| product(m))
        .collect();
    names.sort();
    names
}

/// The modules on `channel` by their short names ("Climate Module R2"), A
/// to Z, for tight places.
pub fn modules_short(sources: &[SensorSource], channel: u8) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = tower_bridge_core::model::modules(sources)
        .into_iter()
        .filter(|(c, _)| *c == channel)
        .map(|(_, m)| m.name())
        .collect();
    names.sort_unstable();
    names
}

/// "10:05:22" today, "2026-10-17 10:05" before.
pub fn time(ts: i64) -> String {
    let Some(t) = Local.timestamp_millis_opt(ts).single() else {
        return ts.to_string();
    };
    if t.date_naive() == Local::now().date_naive() {
        t.format("%H:%M:%S").to_string()
    } else {
        t.format("%Y-%m-%d %H:%M").to_string()
    }
}

/// "Temperature on I²C channel 0 above 24.0 °C: 24.3 °C".
pub fn alert(ev: &tower_bridge_core::alerts::AlertEvent, unit: TemperatureUnit) -> String {
    use tower_bridge_core::alerts::{AlertKind, Bound};
    let value = value(ev.metric, Some(ev.value), unit);
    let limit = value_of(ev.metric, ev.threshold, unit);
    let what = format!("{} on I²C channel {}", metric_name(ev.metric), ev.channel);
    match (ev.kind, ev.bound) {
        (AlertKind::Triggered, Bound::High) => format!("{what} above {limit}: {value}"),
        (AlertKind::Triggered, Bound::Low) => format!("{what} below {limit}: {value}"),
        (AlertKind::Recovered, _) => format!("{what} back within its limits: {value}"),
    }
}

fn value_of(metric: Metric, v: f64, unit: TemperatureUnit) -> String {
    value(metric, Some(v), unit)
}

pub fn relay_state(state: Option<bool>) -> &'static str {
    match state {
        Some(true) => "TRUE",
        Some(false) => "FALSE",
        None => "unknown",
    }
}

/// What the bridge is doing, in a few words.
pub fn device(d: &DeviceState) -> String {
    match d {
        DeviceState::Searching => "looking for the bridge".into(),
        DeviceState::NoModule { .. } => "connected, no sensors found".into(),
        DeviceState::Ready { device, .. } => match device.backend {
            Backend::Hid => "connected over USB".into(),
            Backend::I2cDev => "connected through Linux i2c-dev".into(),
        },
        DeviceState::Error { kind, message } => match kind {
            DeviceErrorKind::NotFound => "not connected".into(),
            DeviceErrorKind::Busy => "in use by another program".into(),
            DeviceErrorKind::PermissionDenied => format!("no permission ({message})"),
            DeviceErrorKind::I2cDevNotLoaded => "the i2c-dev kernel module is not loaded".into(),
            DeviceErrorKind::Failed => message.clone(),
        },
    }
}

/// A short span such as "30s", "15m", "24h", "7d", "2w" in milliseconds.
pub fn parse_span(s: &str) -> Option<i64> {
    let s = s.trim();
    let digits = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (n, unit) = s.split_at(digits);
    let n: i64 = n.parse().ok().filter(|n| *n > 0)?;
    let ms = match unit.trim() {
        "s" => 1_000,
        "m" | "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 7 * 86_400_000,
        _ => return None,
    };
    n.checked_mul(ms)
}

/// A point in time: a date (local midnight), a local date and time, RFC
/// 3339, or Unix milliseconds.
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(ms) = tower_bridge_core::api::parse_time(s) {
        return Some(ms);
    }
    let local = |naive: chrono::NaiveDateTime| {
        Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|t| t.timestamp_millis())
    };
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return local(d.and_hms_opt(0, 0, 0)?);
    }
    [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ]
    .iter()
    .find_map(|f| chrono::NaiveDateTime::parse_from_str(s, f).ok())
    .and_then(local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_as_on_the_dashboard() {
        let c = TemperatureUnit::Celsius;
        assert_eq!(value(Metric::Temperature, Some(23.44), c), "23.4 °C");
        assert_eq!(
            value(Metric::Temperature, Some(-0.04), c),
            "0.0 °C",
            "no negative zero"
        );
        assert_eq!(
            value(Metric::Temperature, Some(20.0), TemperatureUnit::Fahrenheit),
            "68.0 °F"
        );
        assert_eq!(value(Metric::Pressure, Some(101_325.0), c), "1013.2 hPa");
        assert_eq!(value(Metric::Illuminance, Some(312.4), c), "312 lx");
        assert_eq!(value(Metric::Co2, None, c), "-");
        assert_eq!(product(ModuleKind::BarometerTag), "TOWER Barometer Tag");
    }

    #[test]
    fn spans_and_times() {
        assert_eq!(parse_span("7d"), Some(7 * 86_400_000));
        assert_eq!(parse_span("30m"), Some(1_800_000));
        assert_eq!(parse_span("24 h"), Some(86_400_000));
        assert_eq!(parse_span("0h"), None);
        assert_eq!(parse_span("7"), None);
        assert_eq!(parse_span("7y"), None);
        assert_eq!(parse_time("1792310500005"), Some(1_792_310_500_005));
        assert_eq!(parse_time("2026-10-17T08:00:00Z"), Some(1_792_224_000_000));
        let midnight = parse_time("2026-10-17");
        assert!(midnight.is_some());
        assert_eq!(parse_time("2026-10-17T00:00"), midnight);
        assert_eq!(parse_time("tomorrow"), None);
    }
}
