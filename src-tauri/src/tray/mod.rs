//! System tray / menu bar icon.

pub mod render;

use std::sync::Arc;

use parking_lot::Mutex;
use tauri::AppHandle;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tower_bridge_core::alerts::ActiveAlert;
use tower_bridge_core::model::{DeviceErrorKind, DeviceState, Metric, Reading};
use tracing::warn;

use crate::format;
use crate::i18n::{Lang, fill};
use crate::settings::{Settings, TemperatureUnit};
use crate::state::MqttState;

const TRAY_ID: &str = "main";

#[allow(dead_code)]
mod icons {
    pub static MAC_ON: &[u8] = include_bytes!("../../icons/tray/mac-template.png");
    pub static MAC_OFF: &[u8] = include_bytes!("../../icons/tray/mac-template-off.png");
    pub static TILES: &[(u32, &[u8], &[u8])] = &[
        (
            16,
            include_bytes!("../../icons/tray/tile-16.png"),
            include_bytes!("../../icons/tray/tile-off-16.png"),
        ),
        (
            20,
            include_bytes!("../../icons/tray/tile-20.png"),
            include_bytes!("../../icons/tray/tile-off-20.png"),
        ),
        (
            24,
            include_bytes!("../../icons/tray/tile-24.png"),
            include_bytes!("../../icons/tray/tile-off-24.png"),
        ),
        (
            32,
            include_bytes!("../../icons/tray/tile-32.png"),
            include_bytes!("../../icons/tray/tile-off-32.png"),
        ),
        (
            40,
            include_bytes!("../../icons/tray/tile-40.png"),
            include_bytes!("../../icons/tray/tile-off-40.png"),
        ),
        (
            48,
            include_bytes!("../../icons/tray/tile-48.png"),
            include_bytes!("../../icons/tray/tile-off-48.png"),
        ),
        (
            64,
            include_bytes!("../../icons/tray/tile-64.png"),
            include_bytes!("../../icons/tray/tile-off-64.png"),
        ),
    ];

    /// Smallest bundled tile at least `size` pixels wide.
    pub fn tile(size: u32, on: bool) -> &'static [u8] {
        let entry = TILES
            .iter()
            .find(|(s, _, _)| *s >= size)
            .unwrap_or(&TILES[TILES.len() - 1]);
        if on { entry.1 } else { entry.2 }
    }
}

/// What the icon currently shows, to skip redundant updates.
#[derive(Clone, Debug, PartialEq, Default)]
struct Shown {
    icon: String,
    title: Option<String>,
    tooltip: String,
    /// Language of the static menu labels.
    lang: Option<Lang>,
    mqtt: Option<(bool, String)>,
}

struct Inner {
    tray: TrayIcon,
    temperature: MenuItem<tauri::Wry>,
    humidity: MenuItem<tauri::Wry>,
    illuminance: MenuItem<tauri::Wry>,
    pressure: MenuItem<tauri::Wry>,
    status: MenuItem<tauri::Wry>,
    open: MenuItem<tauri::Wry>,
    open_at_login: CheckMenuItem<tauri::Wry>,
    mqtt: CheckMenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
    #[allow(dead_code)]
    renderer: Option<render::TileRenderer>,
    shown: Mutex<Shown>,
}

#[derive(Clone)]
pub struct TrayController {
    inner: Arc<Inner>,
}

fn image(bytes: &[u8]) -> Option<Image<'static>> {
    Image::from_bytes(bytes).ok().map(|i| i.to_owned())
}

impl TrayController {
    pub fn build(app: &AppHandle) -> tauri::Result<Self> {
        let none = None::<&str>;
        let temperature = MenuItem::with_id(app, "temperature", "Temperature: -", false, none)?;
        let humidity = MenuItem::with_id(app, "humidity", "Humidity: -", false, none)?;
        let illuminance = MenuItem::with_id(app, "illuminance", "Illuminance: -", false, none)?;
        let pressure = MenuItem::with_id(app, "pressure", "Pressure: -", false, none)?;
        let status = MenuItem::with_id(app, "status", "Searching for TOWER Bridge…", false, none)?;
        let open = MenuItem::with_id(app, "open", "Open Dashboard…", true, none)?;
        let open_at_login =
            CheckMenuItem::with_id(app, "open_at_login", "Open at Login", true, false, none)?;
        let mqtt = CheckMenuItem::with_id(app, "mqtt", "MQTT Broker", true, false, none)?;
        let quit = MenuItem::with_id(app, "quit", "Quit TOWER Bridge", true, none)?;
        let menu = Menu::with_items(
            app,
            &[
                &open,
                &PredefinedMenuItem::separator(app)?,
                &temperature,
                &humidity,
                &illuminance,
                &pressure,
                &status,
                &PredefinedMenuItem::separator(app)?,
                &open_at_login,
                &mqtt,
                &PredefinedMenuItem::separator(app)?,
                &quit,
            ],
        )?;

        let mut builder = TrayIconBuilder::with_id(TRAY_ID)
            .menu(&menu)
            .tooltip("TOWER Bridge")
            .on_menu_event(on_menu_event)
            .on_tray_icon_event(on_tray_icon_event);
        #[cfg(target_os = "macos")]
        {
            if let Some(icon) = image(icons::MAC_OFF) {
                builder = builder.icon(icon);
            }
            builder = builder.icon_as_template(true).show_menu_on_left_click(true);
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(icon) = image(icons::tile(32, false)) {
                builder = builder.icon(icon);
            }
        }
        #[cfg(windows)]
        {
            builder = builder.show_menu_on_left_click(false);
        }
        let tray = builder.build(app)?;

        Ok(Self {
            inner: Arc::new(Inner {
                tray,
                temperature,
                humidity,
                illuminance,
                pressure,
                status,
                open,
                open_at_login,
                mqtt,
                quit,
                renderer: render::TileRenderer::new(),
                shown: Mutex::new(Shown::default()),
            }),
        })
    }

    pub fn set_open_at_login(&self, enabled: bool) {
        let _ = self.inner.open_at_login.set_checked(enabled);
    }

    pub fn update(
        &self,
        device: &DeviceState,
        latest: &[Reading],
        settings: &Settings,
        alerts: &[ActiveAlert],
        mqtt: &MqttState,
    ) {
        let i = &self.inner;
        let lang = Lang::effective(settings.language);
        let tr = lang.tr();
        let ready = device.is_ready();
        let unit = settings.temperature_unit;
        let dec = settings.decimal(lang);
        let readings: &[Reading] = if ready { latest } else { &[] };
        // The first channel's value goes into the menu bar.
        let value = |m: Metric| readings.iter().find_map(|r| r.values.get(m));

        for (metric, item) in [
            (Metric::Temperature, &i.temperature),
            (Metric::Humidity, &i.humidity),
            (Metric::Illuminance, &i.illuminance),
            (Metric::Pressure, &i.pressure),
        ] {
            let _ = item.set_text(metric_line(metric, readings, alerts, unit, lang, dec));
        }
        let newest = readings.iter().max_by_key(|r| r.ts);
        let status = status_text(device, newest, settings, lang);
        let _ = i.status.set_text(&status);

        let summary: Vec<String> = Metric::ALL
            .iter()
            .filter_map(|m| joined(*m, readings, unit, dec))
            .collect();
        let tooltip = if summary.is_empty() {
            format!("TOWER Bridge\n{status}")
        } else {
            format!("TOWER Bridge\n{}", summary.join(" · "))
        };
        let tray_value = settings.tray_metric.and_then(|m| value(m).map(|v| (m, v)));
        let title = tray_value.map(|(m, v)| format::short(m, v, unit, dec));
        let mqtt_item = match mqtt {
            MqttState::Disabled => (false, "MQTT".to_string()),
            MqttState::Error { .. } => (false, format!("MQTT ({})", tr.mqtt_error)),
            MqttState::Connected { address } => (true, format!("MQTT ({address})")),
            MqttState::Connecting { .. } => (true, format!("MQTT ({})", tr.mqtt_connecting)),
        };

        let (icon_key, icon) = self.icon_for(ready, tray_value, settings, lang);
        let next = Shown {
            icon: icon_key,
            title: if cfg!(windows) { None } else { title },
            tooltip,
            lang: Some(lang),
            mqtt: Some(mqtt_item.clone()),
        };
        let mut shown = i.shown.lock();
        if shown.lang != next.lang {
            let _ = i.open.set_text(tr.open_dashboard);
            let _ = i.open_at_login.set_text(tr.open_at_login);
            let _ = i.quit.set_text(tr.quit);
        }
        if shown.mqtt != next.mqtt {
            let _ = i.mqtt.set_checked(mqtt_item.0);
            let _ = i.mqtt.set_text(&mqtt_item.1);
        }
        if shown.icon != next.icon {
            let r = if cfg!(target_os = "macos") {
                i.tray.set_icon_with_as_template(icon, true)
            } else {
                i.tray.set_icon(icon)
            };
            if let Err(e) = r {
                warn!("tray icon update failed: {e}");
            }
        }
        if shown.title != next.title {
            let _ = i.tray.set_title(next.title.as_deref());
        }
        if shown.tooltip != next.tooltip && !cfg!(target_os = "linux") {
            let _ = i.tray.set_tooltip(Some(&next.tooltip));
        }
        *shown = next;
    }

    #[cfg(target_os = "macos")]
    fn icon_for(
        &self,
        ready: bool,
        _value: Option<(Metric, f64)>,
        _settings: &Settings,
        _lang: Lang,
    ) -> (String, Option<Image<'static>>) {
        let bytes = if ready { icons::MAC_ON } else { icons::MAC_OFF };
        (format!("mac-{ready}"), image(bytes))
    }

    #[cfg(windows)]
    fn icon_for(
        &self,
        ready: bool,
        value: Option<(Metric, f64)>,
        settings: &Settings,
        lang: Lang,
    ) -> (String, Option<Image<'static>>) {
        let scale = self
            .inner
            .tray
            .app_handle()
            .primary_monitor()
            .ok()
            .flatten()
            .map_or(1.0, |m| m.scale_factor());
        let size = (16.0 * scale).round().clamp(16.0, 64.0) as u32;
        match (value, self.inner.renderer.as_ref()) {
            (Some((m, v)), Some(renderer)) => {
                let text = format::tile(m, v, settings.temperature_unit, settings.decimal(lang));
                let key = format!("tile-{text}-{size}");
                let rgba = renderer.render(&text, size, render::BRAND_RED);
                (key, Some(Image::new_owned(rgba, size, size)))
            }
            _ => (
                format!("static-{ready}-{size}"),
                image(icons::tile(size, ready)),
            ),
        }
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    fn icon_for(
        &self,
        ready: bool,
        _value: Option<(Metric, f64)>,
        _settings: &Settings,
        _lang: Lang,
    ) -> (String, Option<Image<'static>>) {
        (format!("static-{ready}"), image(icons::tile(32, ready)))
    }
}

/// The values of `metric` on each channel, in channel order: "23.4 °C / 19.5 °C".
fn joined(
    metric: Metric,
    readings: &[Reading],
    unit: TemperatureUnit,
    dec: format::Decimal,
) -> Option<String> {
    let list: Vec<String> = readings
        .iter()
        .filter_map(|r| r.values.get(metric))
        .map(|v| format::full(metric, v, unit, dec))
        .collect();
    (!list.is_empty()).then(|| list.join(" / "))
}

/// A menu line: "Temperature: 23.4 °C / 19.5 °C", with ⚠ while a shown
/// channel is beyond a limit.
fn metric_line(
    metric: Metric,
    readings: &[Reading],
    alerts: &[ActiveAlert],
    unit: TemperatureUnit,
    lang: Lang,
    dec: format::Decimal,
) -> String {
    let label = lang.tr().metric(metric);
    match joined(metric, readings, unit, dec) {
        Some(text) => {
            let alerting = alerts
                .iter()
                .any(|a| a.metric == metric && readings.iter().any(|r| r.channel == a.channel));
            let warn = if alerting { "  ⚠" } else { "" };
            format!("{label}: {text}{warn}")
        }
        None => format!("{label}: -"),
    }
}

fn status_text(
    device: &DeviceState,
    reading: Option<&Reading>,
    settings: &Settings,
    lang: Lang,
) -> String {
    let tr = lang.tr();
    // Which one is read is a guess: more important than the time.
    if device.device().is_some_and(|d| d.bridges_connected > 1) {
        return tr.several_bridges.into();
    }
    match device {
        DeviceState::Ready { .. } => reading
            .and_then(|r| chrono::DateTime::from_timestamp_millis(r.ts))
            .map(|t| {
                let time =
                    format::clock(t.with_timezone(&chrono::Local), settings.time_format, lang);
                fill(tr.updated, &[("time", &time)])
            })
            .unwrap_or_else(|| tr.measuring.into()),
        DeviceState::Searching => tr.searching.into(),
        DeviceState::NoModule { .. } => tr.no_module.into(),
        DeviceState::Error { kind, .. } => match kind {
            DeviceErrorKind::NotFound => tr.not_connected,
            DeviceErrorKind::PermissionDenied => tr.no_permission,
            DeviceErrorKind::I2cDevNotLoaded => tr.i2c_dev_missing,
            DeviceErrorKind::Busy => tr.busy,
            DeviceErrorKind::Failed => tr.bridge_error,
        }
        .into(),
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "open" => crate::window::show_dashboard(app),
        "open_at_login" => crate::autostart::toggle(app),
        "mqtt" => crate::actions::toggle_mqtt(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

fn on_tray_icon_event(tray: &TrayIcon, event: TrayIconEvent) {
    if cfg!(windows)
        && let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
    {
        crate::window::show_dashboard(tray.app_handle());
    }
}

#[cfg(test)]
mod tests {
    use tower_bridge_core::alerts::Bound;
    use tower_bridge_core::model::{Backend, DeviceInfo, Values};

    use super::*;

    fn reading(channel: u8, temperature: Option<f64>) -> Reading {
        Reading {
            ts: 1_792_245_600_000,
            device: "SN".into(),
            channel,
            values: Values {
                temperature,
                ..Values::default()
            },
            altitude: None,
        }
    }

    fn alert(channel: u8) -> ActiveAlert {
        ActiveAlert {
            metric: Metric::Temperature,
            channel,
            bound: Bound::High,
            since: 0,
            threshold: 25.0,
            value: 30.0,
        }
    }

    #[test]
    fn menu_lines_list_every_channel() {
        let c = TemperatureUnit::Celsius;
        let readings = [reading(0, Some(23.44)), reading(1, Some(19.5))];
        assert_eq!(
            metric_line(
                Metric::Temperature,
                &readings,
                &[],
                c,
                Lang::En,
                Lang::En.into()
            ),
            "Temperature: 23.4 °C / 19.5 °C"
        );
        assert_eq!(
            metric_line(
                Metric::Temperature,
                &readings,
                &[],
                c,
                Lang::Cs,
                Lang::Cs.into()
            ),
            "Teplota: 23,4 °C / 19,5 °C"
        );
        assert_eq!(
            metric_line(
                Metric::Humidity,
                &readings,
                &[],
                c,
                Lang::En,
                Lang::En.into()
            ),
            "Humidity: -"
        );
        // A channel without the value is left out, not shown as a gap.
        let partial = [reading(0, None), reading(1, Some(19.5))];
        assert_eq!(
            metric_line(
                Metric::Temperature,
                &partial,
                &[],
                c,
                Lang::En,
                Lang::En.into()
            ),
            "Temperature: 19.5 °C"
        );
    }

    #[test]
    fn the_warning_is_only_for_channels_shown() {
        let c = TemperatureUnit::Celsius;
        let readings = [reading(0, Some(23.0))];
        assert!(
            metric_line(
                Metric::Temperature,
                &readings,
                &[alert(0)],
                c,
                Lang::En,
                Lang::En.into()
            )
            .ends_with('⚠')
        );
        assert!(
            !metric_line(
                Metric::Temperature,
                &readings,
                &[alert(1)],
                c,
                Lang::En,
                Lang::En.into()
            )
            .contains('⚠')
        );
    }

    #[test]
    fn status_lines() {
        let settings = Settings::default();
        let info = DeviceInfo {
            backend: Backend::Hid,
            serial: "SN".into(),
            path: "p".into(),
            chip: None,
            bridges_connected: 1,
        };
        let ready = DeviceState::Ready {
            device: info.clone(),
            sources: Vec::new(),
        };
        assert_eq!(
            status_text(&ready, None, &settings, Lang::En),
            Lang::En.tr().measuring
        );
        assert!(
            status_text(&ready, Some(&reading(0, Some(1.0))), &settings, Lang::En).contains(':')
        );
        assert_eq!(
            status_text(
                &DeviceState::NoModule {
                    device: info.clone()
                },
                None,
                &settings,
                Lang::En
            ),
            "No sensors found"
        );
        let several = DeviceState::Ready {
            device: DeviceInfo {
                bridges_connected: 2,
                ..info
            },
            sources: Vec::new(),
        };
        assert_eq!(
            status_text(&several, Some(&reading(0, Some(1.0))), &settings, Lang::En),
            Lang::En.tr().several_bridges,
            "the warning wins over the time"
        );
        let busy = DeviceState::Error {
            kind: DeviceErrorKind::Busy,
            message: String::new(),
        };
        assert_eq!(
            status_text(&busy, None, &settings, Lang::En),
            Lang::En.tr().busy
        );
    }
}
