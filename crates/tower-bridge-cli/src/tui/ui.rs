//! Drawing the dashboard.
//!
//! ```text
//!  TOWER Bridge  ● connected over USB   interval 20 s   MQTT ● connected       12:55:37
//! ┌ Values ──────────────────────┐┌ Temperature · last hour ──────────────────────┐
//! │ I²C 0  TOWER Climate Module R2││                                    I²C 0 I²C 1 │
//! │▸Temperature  24.2 °C ▂▃▅▆▇    ││  braille lines, one per channel                │
//! └──────────────────────────────┘└────────────────────────────────────────────────┘
//! ┌ Relay ───────┐┌ GPS ─────────┐┌ NFC ──────────┐
//! ┌ Log ───────────────────────────────────────────────────────────────────────────┐
//!  ↑↓ value  1 2 3 span  t f relay  p pulse  i identify  r rescan  n NFC  q quit
//! ```

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span as TextSpan};
use ratatui::widgets::{Axis, Block, Chart, Dataset, GraphType, LegendPosition, Paragraph, Wrap};
use tower_bridge_core::model::{DeviceState, Metric};
use tower_bridge_core::store::History;
use tower_bridge_service::MqttState;

use super::app::{App, Span};
use crate::output;

/// The line colour of each I²C channel.
const CHANNEL: [Color; 2] = [Color::Cyan, Color::Magenta];
/// Focus and selection: the HARDWARIO red, as near as terminals get.
const ACCENT: Color = Color::LightRed;
const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 16;

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn block(title: &str) -> Block<'_> {
    Block::bordered()
        .title(format!(" {title} "))
        .border_style(dim())
        .title_style(Style::new().fg(Color::Gray))
}

pub fn draw(frame: &mut Frame, app: &App, log: &[String]) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        frame.render_widget(
            Paragraph::new(format!(
                "Make the terminal at least {MIN_WIDTH}×{MIN_HEIGHT}."
            )),
            area,
        );
        return;
    }
    let devices = 4;
    let [header, body, devices_area, log_area, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(devices),
        Constraint::Length(6),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_header(frame, app, header);
    let values_width = if area.width >= 110 { 46 } else { 40 };
    let [values, chart] =
        Layout::horizontal([Constraint::Length(values_width), Constraint::Min(20)]).areas(body);
    draw_values(frame, app, values);
    draw_chart(frame, app, chart);
    draw_devices(frame, app, devices_area);
    draw_log(frame, app, log, log_area);
    draw_footer(frame, app, footer);
}

fn dot(ok: Option<bool>) -> TextSpan<'static> {
    let color = match ok {
        Some(true) => Color::Green,
        Some(false) => Color::Red,
        None => Color::Yellow,
    };
    TextSpan::styled("● ", Style::new().fg(color))
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let bar = Style::new().bg(Color::Gray).fg(Color::Black);
    let mut spans = vec![TextSpan::styled(
        " TOWER Bridge  ",
        bar.add_modifier(Modifier::BOLD),
    )];
    match (&app.snap, &app.error) {
        (_, Some(e)) => {
            spans.push(dot(Some(false)));
            spans.push(TextSpan::raw(e.clone()));
        }
        (Some(snap), None) => {
            let ok = match &snap.device {
                DeviceState::Ready { .. } => Some(true),
                DeviceState::Searching => None,
                _ => Some(false),
            };
            spans.push(dot(ok));
            spans.push(TextSpan::raw(output::device(&snap.device)));
            spans.push(TextSpan::raw(format!(
                "   interval {} s",
                snap.sample_interval_ms / 1000
            )));
            if let Some(m) = &snap.mqtt {
                let (ok, text) = match m {
                    MqttState::Disabled => (None, "off".to_string()),
                    MqttState::Connecting { .. } => (None, "connecting".into()),
                    MqttState::Connected { address } => (Some(true), address.clone()),
                    MqttState::Error { .. } => (Some(false), "error".into()),
                };
                if !matches!(m, MqttState::Disabled) {
                    spans.push(TextSpan::raw("   MQTT "));
                    spans.push(dot(ok));
                    spans.push(TextSpan::raw(text));
                }
            }
        }
        (None, None) => spans.push(TextSpan::raw("starting…")),
    }
    // Where the data comes from, when there is room for it.
    let via = format!("   via {}", app.via);
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if used + via.chars().count() <= usize::from(area.width) {
        spans.push(TextSpan::raw(via));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(bar), area);
}

/// Up to `n` recent values of `metric` on `channel`, from the history.
fn recent(history: Option<&History>, metric: Metric, channel: u8, n: usize) -> Vec<f64> {
    let Some(series) = history.and_then(|h| h.series(metric, channel)) else {
        return Vec::new();
    };
    let values: Vec<f64> = series.avg.iter().flatten().copied().collect();
    values[values.len().saturating_sub(n)..].to_vec()
}

/// A small bar chart of values, "▁▂▃▅▇".
fn sparkline(values: &[f64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let (lo, hi) = values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    values
        .iter()
        .map(|v| {
            let i = if hi > lo {
                ((v - lo) / (hi - lo) * 7.0).round() as usize
            } else {
                3
            };
            BARS[i.min(7)]
        })
        .collect()
}

fn draw_values(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();
    let Some(snap) = &app.snap else {
        let text = app.error.as_deref().unwrap_or("Looking for the bridge…");
        frame.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: true })
                .block(block("Values")),
            area,
        );
        return;
    };
    let sources = snap.device.sources();
    let mut current = None;
    // The line of the selected row, to keep it in view.
    let mut selected_line = 0;
    let width = usize::from(area.width.saturating_sub(2));
    for (i, (channel, metric)) in app.rows().into_iter().enumerate() {
        if current != Some(channel) {
            current = Some(channel);
            // The modules, wrapped under the channel rather than cut off.
            let label = format!("I²C {channel}  ");
            let indent = label.chars().count();
            let mut rows: Vec<String> = vec![String::new()];
            for name in output::modules_short(sources, channel) {
                let row = rows.last_mut().map(|r| r.chars().count()).unwrap_or(0);
                if row > 0 && indent + row + 2 + name.chars().count() > width {
                    rows.push(String::new());
                }
                if let Some(r) = rows.last_mut() {
                    if !r.is_empty() {
                        r.push_str(", ");
                    }
                    r.push_str(name);
                }
            }
            for (k, row) in rows.into_iter().enumerate() {
                let head = if k == 0 {
                    label.clone()
                } else {
                    " ".repeat(indent)
                };
                lines.push(Line::from(vec![
                    TextSpan::styled(head, Style::new().fg(CHANNEL[usize::from(channel) % 2])),
                    TextSpan::styled(row, dim()),
                ]));
            }
        }
        let selected = i == app.selected;
        if selected {
            selected_line = lines.len();
        }
        let value = snap
            .latest
            .iter()
            .find(|r| r.channel == channel)
            .and_then(|r| r.values.get(metric));
        let text = match value {
            Some(v) => output::value(metric, Some(v), app.unit),
            None => "…".into(),
        };
        let marker = if selected { "▸" } else { " " };
        let name = Style::new().fg(if selected { ACCENT } else { Color::Reset });
        lines.push(Line::from(vec![
            TextSpan::styled(marker, Style::new().fg(ACCENT)),
            TextSpan::styled(format!("{:<12}", output::metric_name(metric)), name),
            TextSpan::styled(
                format!("{text:>12}"),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            TextSpan::raw("  "),
            TextSpan::styled(
                sparkline(&recent(app.history.as_ref(), metric, channel, 10)),
                Style::new().fg(CHANNEL[usize::from(channel) % 2]),
            ),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::styled("No sensors.", dim()));
    }
    // Keep the selected row in view in a short window.
    let inner = area.height.saturating_sub(2) as usize;
    let skip = (selected_line + 1)
        .saturating_sub(inner)
        .min(lines.len().saturating_sub(inner));
    let lines: Vec<Line> = lines.into_iter().skip(skip).collect();
    frame.render_widget(Paragraph::new(lines).block(block("Values")), area);
}

/// The lines of the chart: per channel, its runs of values between gaps
/// (the history has a `null` where readings stopped), so a night without
/// readings is no straight line.
/// Points (time in s, value) without a gap.
type Run = Vec<(f64, f64)>;

pub(super) fn segments(app: &App, metric: Metric, from: f64) -> Vec<(u8, Vec<Run>)> {
    let mut lines: Vec<(u8, Vec<Run>)> = Vec::new();
    if let Some(h) = &app.history {
        for channel in &h.channels {
            let Some(series) = h.series(metric, *channel) else {
                continue;
            };
            let mut runs: Vec<Run> = vec![Vec::new()];
            for (t, v) in h.t.iter().zip(&series.avg) {
                match v {
                    Some(v) if *t >= from => {
                        if let Some(run) = runs.last_mut() {
                            run.push((*t, output::display(metric, *v, app.unit)));
                        }
                    }
                    _ => {
                        if runs.last().is_some_and(|r| !r.is_empty()) {
                            runs.push(Vec::new());
                        }
                    }
                }
            }
            lines.push((*channel, runs));
        }
    }
    // The latest value continues the last run (the history is reloaded only
    // every half minute).
    if let Some(snap) = &app.snap {
        for r in &snap.latest {
            let Some(v) = r.values.get(metric) else {
                continue;
            };
            let point = (r.ts as f64 / 1000.0, output::display(metric, v, app.unit));
            match lines.iter_mut().find(|(c, _)| *c == r.channel) {
                Some((_, runs)) => {
                    if let Some(run) = runs.last_mut()
                        && run.last().is_none_or(|p| p.0 < point.0)
                    {
                        run.push(point);
                    }
                }
                None => lines.push((r.channel, vec![vec![point]])),
            }
        }
    }
    for (_, runs) in &mut lines {
        runs.retain(|r| !r.is_empty());
    }
    lines.retain(|(_, runs)| !runs.is_empty());
    lines.sort_by_key(|(c, _)| *c);
    lines
}

fn draw_chart(frame: &mut Frame, app: &App, area: Rect) {
    let Some(metric) = app.selected_metric() else {
        frame.render_widget(block("Chart"), area);
        return;
    };
    let unit = output::unit_label(metric, app.unit);
    let title = format!(
        "{} ({unit}) · {}",
        output::metric_name(metric),
        app.span.label()
    );
    let now = tower_bridge_core::acquisition::unix_ms() as f64 / 1000.0;
    let from = now - app.span.ms() as f64 / 1000.0;
    let lines = segments(app, metric, from);
    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new("No values yet.")
                .style(dim())
                .block(block(&title)),
            area,
        );
        return;
    }
    let (lo, hi) = lines
        .iter()
        .flat_map(|(_, runs)| runs.iter().flatten().map(|(_, v)| *v))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
    let pad = ((hi - lo) * 0.1).max(match metric {
        Metric::Temperature | Metric::Humidity | Metric::Pressure => 0.5,
        _ => 5.0,
    });
    let (lo, hi) = (lo - pad, hi + pad);
    let fmt = |v: f64| format!("{v:.*}", output::decimals(metric));
    // One line per run; the channel is named once, for the legend.
    let datasets: Vec<Dataset> = lines
        .iter()
        .flat_map(|(channel, runs)| {
            runs.iter().enumerate().map(move |(i, run)| {
                let set = Dataset::default()
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::new().fg(CHANNEL[usize::from(*channel) % 2]))
                    .data(run);
                if i == 0 {
                    set.name(format!("I²C {channel}"))
                } else {
                    set
                }
            })
        })
        .collect();
    // Relative times stay short at any span and width.
    let (start, middle) = match app.span {
        Span::Hour => ("-60 min", "-30 min"),
        Span::Day => ("-24 h", "-12 h"),
        Span::Week => ("-7 d", "-3.5 d"),
    };
    let x_labels = if area.width >= 50 {
        vec![
            TextSpan::raw(start),
            TextSpan::raw(middle),
            TextSpan::raw("now"),
        ]
    } else {
        vec![TextSpan::raw(start), TextSpan::raw("now")]
    };
    let chart = Chart::new(datasets)
        .block(block(&title))
        // Top left: the newest values are at the right.
        .legend_position(Some(LegendPosition::TopLeft))
        .x_axis(
            Axis::default()
                .style(dim())
                .bounds([from, now])
                .labels(x_labels),
        )
        .y_axis(Axis::default().style(dim()).bounds([lo, hi]).labels(vec![
            TextSpan::raw(fmt(lo)),
            TextSpan::raw(fmt((lo + hi) / 2.0)),
            TextSpan::raw(fmt(hi)),
        ]));
    frame.render_widget(chart, area);
}

fn draw_devices(frame: &mut Frame, app: &App, area: Rect) {
    let [relay, gps, nfc] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    let snap = app.snap.as_ref();
    let relay_lines = match snap.and_then(|s| s.relay) {
        Some(r) => {
            let color = match r.state {
                Some(true) => Color::Green,
                Some(false) => Color::Red,
                None => Color::DarkGray,
            };
            let mut first = vec![
                TextSpan::styled("● ", Style::new().fg(color)),
                TextSpan::styled(
                    output::relay_state(r.state),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(until) = r.pulse_until {
                first.push(TextSpan::styled(
                    format!("  pulse until {}", output::time(until)),
                    dim(),
                ));
            }
            vec![
                Line::from(first),
                Line::styled("t TRUE  f FALSE  p pulse", dim()),
            ]
        }
        None => vec![Line::styled("Not connected", dim())],
    };
    frame.render_widget(Paragraph::new(relay_lines).block(block("Relay")), relay);
    let gps_lines = match snap.and_then(|s| s.gps.as_ref()) {
        Some(g) => match (g.fix, g.latitude, g.longitude) {
            (true, Some(lat), Some(lon)) => vec![
                Line::raw(format!("{lat:.5}, {lon:.5}")),
                Line::styled(
                    format!(
                        "{}{} satellites",
                        g.accuracy.map_or(String::new(), |a| format!("±{a:.0} m, ")),
                        g.satellites
                    ),
                    dim(),
                ),
            ],
            _ => vec![Line::styled(
                format!("Searching ({} satellites)", g.satellites),
                dim(),
            )],
        },
        None => vec![Line::styled("Not connected", dim())],
    };
    frame.render_widget(Paragraph::new(gps_lines).block(block("GPS")), gps);
    let nfc_lines = match snap.and_then(|s| s.nfc.as_ref()) {
        Some(n) => vec![
            Line::raw(n.url.clone().unwrap_or_else(|| "No URL".into())),
            Line::styled("n write a URL", dim()),
        ],
        None => vec![Line::styled("Not connected", dim())],
    };
    frame.render_widget(Paragraph::new(nfc_lines).block(block("NFC")), nfc);
}

fn draw_log(frame: &mut Frame, app: &App, log: &[String], area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    if let Some(snap) = &app.snap {
        for a in &snap.active_alerts {
            let bound = match a.bound {
                tower_bridge_core::alerts::Bound::High => "above",
                tower_bridge_core::alerts::Bound::Low => "below",
            };
            lines.push(Line::styled(
                format!(
                    "Alert: {} on I²C {} {bound} {}: {}",
                    output::metric_name(a.metric),
                    a.channel,
                    output::value(a.metric, Some(a.threshold), app.unit),
                    output::value(a.metric, Some(a.value), app.unit)
                ),
                Style::new().fg(Color::Red),
            ));
        }
    }
    let room = (area.height.saturating_sub(2) as usize).saturating_sub(lines.len());
    for l in &log[log.len().saturating_sub(room)..] {
        let style = if l.contains(" WARN ") || l.contains(" ERROR ") {
            Style::new().fg(Color::Yellow)
        } else {
            dim()
        };
        lines.push(Line::styled(l.clone(), style));
    }
    // Where the data comes from, also when the header has no room for it.
    let title = if app.via.starts_with("this") {
        "Log".to_string()
    } else {
        format!("Log · data from {}", app.via)
    };
    frame.render_widget(Paragraph::new(lines).block(block(&title)), area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let key = Style::new().bg(Color::DarkGray).fg(Color::White);
    let width = usize::from(area.width);
    let line = if let Some(text) = &app.input {
        // The end of a long URL and the cursor stay in view.
        let room = width.saturating_sub(34).max(10);
        let count = text.chars().count();
        let shown: String = if count > room {
            let tail: String = text.chars().skip(count - room + 1).collect();
            format!("…{tail}")
        } else {
            text.clone()
        };
        Line::from(vec![
            TextSpan::styled(" NFC URL ", key),
            TextSpan::raw(format!(" {shown}▏")),
            TextSpan::styled("  Enter write  Esc cancel", dim()),
        ])
    } else if let Some(flash) = app
        .flash
        .as_ref()
        .filter(|f| app.busy || f.at.elapsed().as_secs() < 5)
    {
        Line::styled(
            format!(" {}", flash.text),
            Style::new().fg(if flash.ok { Color::Green } else { Color::Red }),
        )
    } else {
        let mut chips: Vec<(&str, &str)> = vec![("↑↓", "value"), ("1 2 3", "span")];
        if app.has_relay() {
            chips.push(("t f p", "relay"));
        }
        chips.extend([("i", "identify"), ("r", "rescan")]);
        if app.snap.as_ref().is_some_and(|s| s.nfc.is_some()) {
            chips.push(("n", "NFC"));
        }
        chips.push(("q", "quit"));
        // In a narrow terminal the keys alone, except for quitting.
        let full: usize = chips
            .iter()
            .map(|(k, w)| k.chars().count() + w.chars().count() + 4)
            .sum();
        let mut spans = Vec::new();
        for (k, what) in chips {
            spans.push(TextSpan::styled(format!(" {k} "), key));
            if full <= width || k == "q" {
                spans.push(TextSpan::raw(format!(" {what} ")));
            } else {
                spans.push(TextSpan::raw(" "));
            }
        }
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line), area);
}
