//! Human-readable values for the tray and notifications.

use tower_bridge_core::model::{Metric, celsius_to_fahrenheit};

use std::sync::OnceLock;

use crate::i18n::Lang;
use crate::settings::{TemperatureUnit, TimeFormat};

/// Use the language's decimal separator.
fn localize(s: String, lang: Lang) -> String {
    if lang.decimal_comma() {
        s.replace('.', ",")
    } else {
        s
    }
}

/// Convert an SI value to its display unit.
pub fn display_value(metric: Metric, v: f64, unit: TemperatureUnit) -> f64 {
    match metric {
        Metric::Temperature => match unit {
            TemperatureUnit::Celsius => v,
            TemperatureUnit::Fahrenheit => celsius_to_fahrenheit(v),
        },
        Metric::Pressure => v / 100.0,
        _ => v,
    }
}

pub fn unit_label(metric: Metric, unit: TemperatureUnit) -> &'static str {
    match metric {
        Metric::Temperature => match unit {
            TemperatureUnit::Celsius => "°C",
            TemperatureUnit::Fahrenheit => "°F",
        },
        Metric::Humidity => "%",
        Metric::Illuminance => "lx",
        Metric::Pressure => "hPa",
    }
}

fn decimals(metric: Metric) -> usize {
    match metric {
        Metric::Temperature | Metric::Humidity | Metric::Pressure => 1,
        Metric::Illuminance => 0,
    }
}

/// "23.4 °C", "45.2 %", "320 lx", "1013.2 hPa" (decimal comma where usual)
pub fn full(metric: Metric, v: f64, unit: TemperatureUnit, lang: Lang) -> String {
    let d = display_value(metric, v, unit);
    localize(
        format!("{:.*} {}", decimals(metric), d, unit_label(metric, unit)),
        lang,
    )
}

/// Compact form for the macOS menu bar / Linux indicator label.
pub fn short(metric: Metric, v: f64, unit: TemperatureUnit, lang: Lang) -> String {
    let d = display_value(metric, v, unit);
    let s = match metric {
        Metric::Temperature => format!("{d:.1}°"),
        Metric::Humidity => format!("{d:.0} %"),
        Metric::Illuminance => format!("{d:.0} lx"),
        Metric::Pressure => format!("{d:.0} hPa"),
    };
    localize(s, lang)
}

/// At most four characters for the Windows tray tile.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn tile(metric: Metric, v: f64, unit: TemperatureUnit, lang: Lang) -> String {
    let d = display_value(metric, v, unit);
    let s = if d.abs() >= 9_999.5 {
        format!("{:.0}k", d / 1000.0)
    } else if metric == Metric::Illuminance && d.abs() >= 999.5 {
        format!("{:.1}k", d / 1000.0)
    } else {
        format!("{d:.0}")
    };
    localize(s, lang)
}

/// English regions whose usual clock is 12-hour. Like the dashboard, English
/// follows the system region and falls back to British (24-hour) usage.
fn english_uses_12h() -> bool {
    static TWELVE: OnceLock<bool> = OnceLock::new();
    *TWELVE.get_or_init(|| {
        let Some(locale) = sys_locale::get_locale() else {
            return false;
        };
        let mut parts = locale.split(['-', '_', '.']);
        if !parts.next().is_some_and(|l| l.eq_ignore_ascii_case("en")) {
            return false;
        }
        parts
            .find(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
            .is_none_or(|r| {
                ["US", "CA", "AU", "NZ", "PH", "IN"].contains(&r.to_ascii_uppercase().as_str())
            })
    })
}

/// Clock time with seconds for the tray: "14:05:09" or "2:05:09 PM".
pub fn clock(t: chrono::DateTime<chrono::Local>, pref: TimeFormat, lang: Lang) -> String {
    let twelve = match pref {
        TimeFormat::H24 => false,
        TimeFormat::H12 => true,
        TimeFormat::Auto => lang == Lang::En && english_uses_12h(),
    };
    t.format(if twelve { "%-I:%M:%S %p" } else { "%H:%M:%S" })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let c = TemperatureUnit::Celsius;
        let en = Lang::En;
        assert_eq!(full(Metric::Temperature, 23.456, c, en), "23.5 °C");
        assert_eq!(
            full(Metric::Temperature, 20.0, TemperatureUnit::Fahrenheit, en),
            "68.0 °F"
        );
        assert_eq!(full(Metric::Pressure, 101_325.0, c, en), "1013.2 hPa");
        assert_eq!(short(Metric::Humidity, 45.4, c, en), "45 %");
        assert_eq!(tile(Metric::Temperature, -4.4, c, en), "-4");
        assert_eq!(tile(Metric::Illuminance, 1234.0, c, en), "1.2k");
        assert_eq!(tile(Metric::Illuminance, 25_000.0, c, en), "25k");
        assert_eq!(tile(Metric::Pressure, 101_325.0, c, en), "1013");
    }

    #[test]
    fn decimal_comma() {
        let c = TemperatureUnit::Celsius;
        assert_eq!(full(Metric::Temperature, 23.456, c, Lang::Cs), "23,5 °C");
        assert_eq!(short(Metric::Temperature, 23.456, c, Lang::De), "23,5°");
        assert_eq!(tile(Metric::Illuminance, 1234.0, c, Lang::Pl), "1,2k");
    }

    #[test]
    fn clock_follows_the_time_format() {
        use chrono::TimeZone;
        let t = chrono::Local
            .with_ymd_and_hms(2026, 10, 7, 14, 5, 9)
            .single()
            .unwrap_or_default();
        assert_eq!(clock(t, TimeFormat::H24, Lang::En), "14:05:09");
        assert_eq!(clock(t, TimeFormat::H12, Lang::Cs), "2:05:09 PM");
        assert_eq!(clock(t, TimeFormat::Auto, Lang::De), "14:05:09");
    }
}
