//! The dashboard drawn into a test terminal and read back as text, and its
//! keys driven directly.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tower_bridge_core::model::{
    Backend, DeviceInfo, DeviceState, Metric, ModuleKind, Reading, SensorSource, Values,
};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_service::MqttState;
use tower_bridge_service::settings::TemperatureUnit;

use super::app::{Action, App, Span};
use super::ui;
use crate::access::Snapshot;

fn source(metric: Metric, channel: u8, module: ModuleKind) -> SensorSource {
    SensorSource {
        metric,
        channel,
        module,
        sensor: "SHT30".into(),
        address: 0x45,
    }
}

fn snapshot() -> Snapshot {
    let now = tower_bridge_core::acquisition::unix_ms();
    Snapshot {
        device: DeviceState::Ready {
            device: DeviceInfo {
                backend: Backend::Hid,
                serial: "ft260".into(),
                path: "sim".into(),
                chip: None,
                bridges_connected: 1,
            },
            sources: vec![
                source(Metric::Temperature, 0, ModuleKind::ClimateModuleR2),
                source(Metric::Humidity, 0, ModuleKind::ClimateModuleR2),
                source(Metric::Temperature, 1, ModuleKind::TemperatureTag),
            ],
        },
        latest: vec![
            Reading {
                ts: now,
                device: "ft260".into(),
                channel: 0,
                values: Values {
                    temperature: Some(23.44),
                    humidity: Some(41.2),
                    ..Values::default()
                },
                altitude: None,
            },
            Reading {
                ts: now,
                device: "ft260".into(),
                channel: 1,
                values: Values {
                    temperature: Some(19.9),
                    ..Values::default()
                },
                altitude: None,
            },
        ],
        relay: Some(RelayInfo {
            channel: 0,
            address: 0x3B,
            state: Some(true),
            pulse_until: None,
        }),
        nfc: Some(NfcInfo {
            channel: 0,
            address: 0x08,
            part: "NT3H2111".into(),
            url: Some("https://www.hardwario.com".into()),
            formatted: true,
        }),
        gps: None,
        active_alerts: Vec::new(),
        mqtt: Some(MqttState::Connected {
            address: "broker:1883".into(),
        }),
        sample_interval_ms: 20_000,
        unit: TemperatureUnit::Celsius,
    }
}

fn render(app: &App, w: u16, h: u16) -> String {
    // The test backend cannot fail.
    let Ok(mut terminal) = Terminal::new(TestBackend::new(w, h));
    let _ = terminal.draw(|f| {
        ui::draw(
            f,
            app,
            &["12:00:00  INFO MQTT connected to broker:1883".into()],
        )
    });
    let buf = terminal.backend().buffer().clone();
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn shows_values_devices_and_keys() {
    let mut app = App::new("this terminal".into(), TemperatureUnit::Celsius);
    app.set_snapshot(snapshot());
    for (w, h) in [(80, 24), (120, 40)] {
        let screen = render(&app, w, h);
        for text in [
            "TOWER Bridge",
            "connected over USB",
            "broker:1883",
            "I²C 0",
            "Climate Module R2",
            "Temperature",
            "23.4 °C",
            "41.2 %",
            "19.9 °C",
            "TRUE",
            "hardwario.com",
            "MQTT connected",
            "quit",
        ] {
            assert!(screen.contains(text), "{w}x{h} lacks {text:?}:\n{screen}");
        }
    }
}

#[test]
fn a_small_terminal_asks_for_room() {
    let app = App::new("this terminal".into(), TemperatureUnit::Celsius);
    assert!(render(&app, 50, 12).contains("Make the terminal at least"));
}

#[test]
fn keys() {
    let mut app = App::new("x".into(), TemperatureUnit::Celsius);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('t'))),
        Action::None,
        "no relay yet"
    );
    app.set_snapshot(snapshot());
    assert_eq!(app.handle_key(key(KeyCode::Char('t'))), Action::Relay(true));
    assert_eq!(
        app.handle_key(key(KeyCode::Char('f'))),
        Action::Relay(false)
    );
    assert_eq!(app.handle_key(key(KeyCode::Char('p'))), Action::Pulse);
    let mut release = key(KeyCode::Char('q'));
    release.kind = KeyEventKind::Release;
    assert_eq!(
        app.handle_key(release),
        Action::None,
        "releases are ignored"
    );

    assert_eq!(app.selected_metric(), Some(Metric::Temperature));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.selected_metric(), Some(Metric::Humidity));
    for _ in 0..5 {
        app.handle_key(key(KeyCode::Down));
    }
    assert_eq!(app.selected, 2, "stops at the last row");
    assert_eq!(app.handle_key(key(KeyCode::Char('2'))), Action::Span);
    assert_eq!(app.span, Span::Day);
    assert_eq!(
        app.handle_key(key(KeyCode::Char('2'))),
        Action::None,
        "already"
    );

    // Typing a URL takes every key, q included.
    assert_eq!(app.handle_key(key(KeyCode::Char('n'))), Action::None);
    for c in "q.io".chars() {
        assert_eq!(app.handle_key(key(KeyCode::Char(c))), Action::None);
    }
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        Action::WriteNfc("q.io".into())
    );
    assert_eq!(app.input, None);

    app.busy = true;
    assert_eq!(
        app.handle_key(key(KeyCode::Char('i'))),
        Action::None,
        "one action at a time"
    );
    assert_eq!(app.handle_key(key(KeyCode::Char('q'))), Action::Quit);
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Action::Quit
    );
}
