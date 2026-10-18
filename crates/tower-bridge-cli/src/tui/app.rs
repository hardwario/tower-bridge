//! The dashboard's state and keys, without any terminal: what the view
//! draws and the tests drive.

use std::time::Instant;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tower_bridge_core::model::Metric;
use tower_bridge_core::store::History;
use tower_bridge_service::settings::TemperatureUnit;

use crate::access::Snapshot;

/// The time span of the chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Hour,
    Day,
    Week,
}

impl Span {
    pub fn ms(self) -> i64 {
        match self {
            Span::Hour => 3_600_000,
            Span::Day => 86_400_000,
            Span::Week => 7 * 86_400_000,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Span::Hour => "last hour",
            Span::Day => "last 24 hours",
            Span::Week => "last 7 days",
        }
    }
}

/// What a key asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Relay(bool),
    Pulse,
    Identify,
    Rescan,
    WriteNfc(String),
    /// The chart span changed: load its history.
    Span,
}

/// A short message in the footer: what an action did.
#[derive(Clone, Debug)]
pub struct Flash {
    pub text: String,
    pub ok: bool,
    pub at: Instant,
}

pub struct App {
    /// Where the data comes from: this terminal's service, or which instance.
    pub via: String,
    pub unit: TemperatureUnit,
    pub snap: Option<Snapshot>,
    /// Why the last snapshot failed (a running instance went away).
    pub error: Option<String>,
    pub history: Option<History>,
    pub span: Span,
    /// The selected row of `rows()`, whose chart is shown.
    pub selected: usize,
    pub flash: Option<Flash>,
    /// The NFC URL being typed.
    pub input: Option<String>,
    /// An action runs; others wait.
    pub busy: bool,
}

impl App {
    pub fn new(via: String, unit: TemperatureUnit) -> Self {
        Self {
            via,
            unit,
            snap: None,
            error: None,
            history: None,
            span: Span::Hour,
            selected: 0,
            flash: None,
            input: None,
            busy: false,
        }
    }

    /// The measured values, by channel and in the usual metric order.
    pub fn rows(&self) -> Vec<(u8, Metric)> {
        let Some(snap) = &self.snap else {
            return Vec::new();
        };
        let sources = snap.device.sources();
        let mut channels: Vec<u8> = sources.iter().map(|s| s.channel).collect();
        channels.sort_unstable();
        channels.dedup();
        channels
            .into_iter()
            .flat_map(|c| {
                Metric::ALL
                    .into_iter()
                    .filter(move |m| sources.iter().any(|s| s.channel == c && s.metric == *m))
                    .map(move |m| (c, m))
            })
            .collect()
    }

    /// The metric whose chart is shown.
    pub fn selected_metric(&self) -> Option<Metric> {
        self.rows().get(self.selected).map(|(_, m)| *m)
    }

    pub fn set_snapshot(&mut self, snap: Snapshot) {
        self.unit = snap.unit;
        self.snap = Some(snap);
        self.error = None;
        let rows = self.rows().len();
        if rows > 0 && self.selected >= rows {
            self.selected = rows - 1;
        }
    }

    pub fn flash(&mut self, text: impl Into<String>, ok: bool) {
        self.flash = Some(Flash {
            text: text.into(),
            ok,
            at: Instant::now(),
        });
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        // Windows reports key releases too.
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        if let Some(text) = &mut self.input {
            match key.code {
                KeyCode::Enter => {
                    let url = text.trim().to_string();
                    self.input = None;
                    if !url.is_empty() {
                        return Action::WriteNfc(url);
                    }
                }
                KeyCode::Esc => self.input = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) => text.push(c),
                _ => {}
            }
            return Action::None;
        }
        let rows = self.rows().len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::F(10) => return Action::Quit,
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < rows {
                    self.selected += 1;
                }
            }
            KeyCode::Char(c @ ('1' | '2' | '3')) => {
                let span = match c {
                    '1' => Span::Hour,
                    '2' => Span::Day,
                    _ => Span::Week,
                };
                if span != self.span {
                    self.span = span;
                    return Action::Span;
                }
            }
            _ if self.busy => {}
            KeyCode::Char('t') if self.has_relay() => return Action::Relay(true),
            KeyCode::Char('f') if self.has_relay() => return Action::Relay(false),
            KeyCode::Char('p') if self.has_relay() => return Action::Pulse,
            KeyCode::Char('i') => return Action::Identify,
            KeyCode::Char('r') => return Action::Rescan,
            KeyCode::Char('n') if self.snap.as_ref().is_some_and(|s| s.nfc.is_some()) => {
                self.input = Some(String::new());
            }
            _ => {}
        }
        Action::None
    }

    pub fn has_relay(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.relay.is_some())
    }
}
