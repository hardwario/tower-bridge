//! Human-readable values for the tray and notifications.

use tower_bridge_core::model::{Metric, celsius_to_fahrenheit};

use std::sync::OnceLock;

use crate::i18n::Lang;
use crate::settings::{TemperatureUnit, TimeFormat};

/// The decimal separator of displayed numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decimal {
    Point,
    Comma,
}

/// The language's usual separator.
impl From<Lang> for Decimal {
    fn from(lang: Lang) -> Self {
        if lang.decimal_comma() {
            Decimal::Comma
        } else {
            Decimal::Point
        }
    }
}

fn localize(s: String, dec: impl Into<Decimal>) -> String {
    match dec.into() {
        Decimal::Comma => s.replace('.', ","),
        Decimal::Point => s,
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
        Metric::Co2 => "ppm",
        Metric::Voc => "ppb",
    }
}

fn decimals(metric: Metric) -> usize {
    match metric {
        Metric::Temperature | Metric::Humidity | Metric::Pressure => 1,
        Metric::Illuminance | Metric::Co2 | Metric::Voc => 0,
    }
}

/// "-0.0" rounds a small negative number; it reads as "0.0".
fn no_negative_zero(s: String) -> String {
    match s.strip_prefix('-') {
        Some(rest)
            if rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .all(|c| c == '0' || c == '.') =>
        {
            rest.to_string()
        }
        _ => s,
    }
}

/// "23.4 °C", "45.2 %", "320 lx", "1013.2 hPa" (decimal comma where usual)
pub fn full(metric: Metric, v: f64, unit: TemperatureUnit, dec: impl Into<Decimal>) -> String {
    let d = display_value(metric, v, unit);
    localize(
        no_negative_zero(format!(
            "{:.*} {}",
            decimals(metric),
            d,
            unit_label(metric, unit)
        )),
        dec,
    )
}

/// Compact form for the macOS menu bar / Linux indicator label.
pub fn short(metric: Metric, v: f64, unit: TemperatureUnit, dec: impl Into<Decimal>) -> String {
    let d = display_value(metric, v, unit);
    let s = match metric {
        Metric::Temperature => format!("{d:.1}°"),
        Metric::Humidity => format!("{d:.0} %"),
        Metric::Illuminance => format!("{d:.0} lx"),
        Metric::Pressure => format!("{d:.0} hPa"),
        Metric::Co2 => format!("{d:.0} ppm"),
        Metric::Voc => format!("{d:.0} ppb"),
    };
    localize(no_negative_zero(s), dec)
}

/// At most four characters for the Windows tray tile.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn tile(metric: Metric, v: f64, unit: TemperatureUnit, dec: impl Into<Decimal>) -> String {
    let d = display_value(metric, v, unit);
    let k = d / 1000.0;
    // "9.9k" but "10k": one decimal only while it stays below ten.
    let s = if d.abs() >= 9_999.5
        || (metric == Metric::Illuminance && (k * 10.0).round().abs() >= 100.0)
    {
        format!("{k:.0}k")
    } else if metric == Metric::Illuminance && d.abs() >= 999.5 {
        format!("{k:.1}k")
    } else {
        format!("{d:.0}")
    };
    localize(no_negative_zero(s), dec)
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
    fn a_tile_never_takes_more_than_four_characters() {
        let c = TemperatureUnit::Celsius;
        for metric in Metric::ALL {
            let mut v = -60.0;
            while v < 200_000.0 {
                let t = tile(metric, v, c, Lang::En);
                assert!(t.chars().count() <= 4, "{metric:?} {v} gives {t:?}");
                v += if v < 2000.0 { 0.37 } else { 9.7 };
            }
        }
        assert_eq!(tile(Metric::Illuminance, 9960.0, c, Lang::En), "10k");
        assert_eq!(tile(Metric::Illuminance, 9940.0, c, Lang::En), "9.9k");
    }

    #[test]
    fn small_negative_values_read_as_zero() {
        let c = TemperatureUnit::Celsius;
        assert_eq!(full(Metric::Temperature, -0.04, c, Lang::En), "0.0 °C");
        assert_eq!(short(Metric::Temperature, -0.04, c, Lang::Cs), "0,0°");
        assert_eq!(tile(Metric::Temperature, -0.4, c, Lang::En), "0");
        assert_eq!(tile(Metric::Temperature, -4.4, c, Lang::En), "-4");
        assert_eq!(full(Metric::Temperature, -0.06, c, Lang::En), "-0.1 °C");
    }

    #[test]
    fn decimal_comma() {
        let c = TemperatureUnit::Celsius;
        assert_eq!(full(Metric::Temperature, 23.456, c, Lang::Cs), "23,5 °C");
        assert_eq!(short(Metric::Temperature, 23.456, c, Lang::De), "23,5°");
        assert_eq!(tile(Metric::Illuminance, 1234.0, c, Lang::Pl), "1,2k");
    }

    #[test]
    fn a_chosen_separator_beats_the_language() {
        let c = TemperatureUnit::Celsius;
        assert_eq!(
            full(Metric::Temperature, 23.456, c, Decimal::Point),
            "23.5 °C"
        );
        assert_eq!(
            short(Metric::Temperature, 23.456, c, Decimal::Comma),
            "23,5°"
        );
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
