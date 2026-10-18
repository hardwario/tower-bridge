//! The terminal dashboard: live values with charts, the relay, GPS, NFC,
//! alerts and the log, with the keys listed in the footer.
//!
//! Without another instance, the dashboard runs the whole service, as the
//! desktop app does (recording, alerts, MQTT, the API server). Next to a
//! running app or `run`, it shows that instance through its API server.
//!
//! The view never waits: snapshots, history and actions come from worker
//! threads over channels; the loop drains them, draws and polls the keys.

mod app;
mod ui;

use std::collections::VecDeque;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use ratatui::crossterm::event::{self, Event as TermEvent};
use tower_bridge_core::acquisition::unix_ms;
use tower_bridge_core::store::History;
use tower_bridge_service::lock::InstanceLock;
use tower_bridge_service::{Mode, MqttState};
use tracing::{info, warn};

use crate::access::{self, Access, Ctx, Event, Snapshot};
use crate::{exit, output};
use app::{Action, App};

/// What the workers bring back.
enum Update {
    Snapshot(Result<Snapshot, String>),
    /// The running instance shown has stopped.
    Gone(String),
    /// The history of a span.
    History(app::Span, History),
    Done(Result<String, String>),
}

/// The log lines the service writes, for the log pane (stderr would tear
/// the screen).
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<VecDeque<String>>>);

impl LogBuffer {
    const KEEP: usize = 500;

    fn lines(&self) -> Vec<String> {
        self.0.lock().iter().cloned().collect()
    }
}

pub struct LogWriter(LogBuffer, Vec<u8>);

impl std::io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.1.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.1);
        let mut lines = self.0.0.lock();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            lines.push_back(line.to_string());
        }
        while lines.len() > LogBuffer::KEEP {
            lines.pop_front();
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogWriter;
    fn make_writer(&'a self) -> LogWriter {
        LogWriter(self.clone(), Vec::new())
    }
}

/// "12:55:37" in the log pane.
struct LocalTime;

impl tracing_subscriber::fmt::time::FormatTime for LocalTime {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%H:%M:%S"))
    }
}

pub fn run(ctx: &Ctx, verbose: u8) -> Result<()> {
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return Err(crate::failure(
            exit::USAGE,
            "the dashboard needs a terminal",
            Some("use `status`, `watch` or `run` without one"),
        ));
    }
    let log = LogBuffer::default();
    let level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .with_writer(log.clone())
        .with_ansi(false)
        .with_target(false)
        .with_timer(LocalTime)
        .try_init();

    // Opened before the terminal is taken over: a failure prints normally.
    let (observer, events) = access::events();
    let access = Arc::new(Access::open(ctx, Mode::Full, observer)?);
    let unit = ctx.settings()?.temperature_unit;
    let lock_file = ctx.paths()?.lock_file();
    let mut app = App::new(access.describe(), unit);
    // SIGTERM, and SIGHUP when the terminal goes away; in raw mode Ctrl+C
    // arrives as a key.
    crate::stop::install();

    install_panic_hook();
    let result = take_terminal().and_then(|mut terminal| {
        let result = event_loop(
            &mut terminal,
            &mut app,
            &access,
            ctx,
            &lock_file,
            &events,
            &log,
        );
        let _ = terminal.show_cursor();
        // Dropped, the terminal prints to stderr when it cannot show the
        // cursor, and printing to a terminal that is gone panics.
        std::mem::forget(terminal);
        result
    });
    // Never panics, also on a terminal that is gone (a closed window, a
    // dropped SSH session): the service must still be stopped below.
    let _ = ratatui::try_restore();
    if matches!(*access, Access::Local(_)) {
        let _ = writeln!(std::io::stderr(), "Stopping…");
    }
    access.close();
    result
}

/// The alternate screen in raw mode, without ratatui's panic hook (see
/// `install_panic_hook`).
fn take_terminal() -> Result<ratatui::DefaultTerminal> {
    use ratatui::crossterm::{execute, terminal};
    terminal::enable_raw_mode()?;
    execute!(std::io::stdout(), terminal::EnterAlternateScreen)?;
    Ok(ratatui::Terminal::new(
        ratatui::backend::CrosstermBackend::new(std::io::stdout()),
    )?)
}

/// Restore the terminal only for a panic of the dashboard itself. The hub
/// and the acquisition thread catch their panics and carry on: theirs go
/// to the log pane instead of tearing the screen down under a running
/// dashboard.
fn install_panic_hook() {
    let main = std::thread::current().id();
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == main {
            let _ = ratatui::try_restore();
            default(info);
        } else {
            let thread = std::thread::current();
            tracing::error!("{} panicked: {info}", thread.name().unwrap_or("a thread"));
        }
    }));
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        warn!("cannot start a worker: {e}");
    }
}

/// Keys on a thread of their own: reading a terminal that has gone away
/// may never return, and the loop must still see the quit flag.
fn read_keys(tx: Sender<std::io::Result<TermEvent>>) {
    spawn("keys", move || {
        loop {
            let ev = event::read();
            let failed = ev.is_err();
            if tx.send(ev).is_err() || failed {
                return;
            }
        }
    });
}

/// Snapshots every second, for as long as the dashboard runs.
fn poll_snapshots(access: Arc<Access>, lock_file: PathBuf, tx: Sender<Update>, wake: Receiver<()>) {
    spawn("snapshots", move || {
        loop {
            let update = match access.snapshot() {
                Ok(snap) => Update::Snapshot(Ok(snap)),
                // The instance shown has stopped: its values are gone too.
                Err(_)
                    if matches!(*access, Access::Remote(_))
                        && InstanceLock::holder(&lock_file).is_none() =>
                {
                    Update::Gone(format!(
                        "{} has stopped; quit and start the dashboard again to run the service here",
                        access.describe()
                    ))
                }
                Err(e) => Update::Snapshot(Err(match &*access {
                    Access::Remote(_) => format!("no answer from {}", access.describe()),
                    Access::Local(_) => format!("{e:#}"),
                })),
            };
            if tx.send(update).is_err() {
                return;
            }
            // A report from the service asks for a new one sooner.
            match wake.recv_timeout(Duration::from_secs(1)) {
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                _ => while wake.try_recv().is_ok() {},
            }
        }
    });
}

fn load_history(access: &Arc<Access>, ctx: &Ctx, span: app::Span, tx: &Sender<Update>) {
    let (access, ctx, tx) = (access.clone(), ctx.clone(), tx.clone());
    spawn("history", move || {
        let now = unix_ms();
        match access.history(&ctx, now - span.ms(), now, 300) {
            Ok(h) => {
                let _ = tx.send(Update::History(span, h));
            }
            Err(e) => warn!("cannot load the history: {e:#}"),
        }
    });
}

fn act(access: &Arc<Access>, action: Action, tx: &Sender<Update>) {
    let (access, tx) = (access.clone(), tx.clone());
    spawn("action", move || {
        let result = match action {
            Action::Relay(state) => access
                .set_relay(state)
                .map(|r| format!("Relay switched to {}.", output::relay_state(r.state))),
            Action::Pulse => access.pulse_relay(None, None).map(|r| {
                format!(
                    "Relay pulsed to {} until {}.",
                    output::relay_state(r.state),
                    r.pulse_until.map_or("-".into(), output::time)
                )
            }),
            Action::Identify => access
                .identify()
                .map(|()| "The LED on the bridge blinked.".into()),
            Action::Rescan => access.rescan().map(|()| "Looked for modules again.".into()),
            Action::WriteNfc(url) => {
                let url = tower_bridge_core::nfc::with_scheme(&url);
                access
                    .write_nfc(&url)
                    .map(|n| format!("Written to the NFC Tag: {}", n.url.unwrap_or_default()))
            }
            Action::None | Action::Quit | Action::Span => return,
        };
        let _ = tx.send(Update::Done(result.map_err(|e| format!("{e:#}"))));
    });
}

#[allow(clippy::too_many_arguments)]
fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    access: &Arc<Access>,
    ctx: &Ctx,
    lock_file: &std::path::Path,
    events: &Receiver<Event>,
    log: &LogBuffer,
) -> Result<()> {
    let (tx, updates) = crossbeam_channel::unbounded();
    let (wake_tx, wake) = crossbeam_channel::bounded(1);
    let (keys_tx, keys) = crossbeam_channel::unbounded();
    read_keys(keys_tx);
    poll_snapshots(access.clone(), lock_file.to_path_buf(), tx.clone(), wake);
    load_history(access, ctx, app.span, &tx);
    let mut history_at = Instant::now();
    let unit = app.unit;
    loop {
        // The service's reports (when it runs here): news for the log, and
        // a fresh snapshot.
        while let Ok(ev) = events.try_recv() {
            match &ev {
                Event::Alert(a) => match a.kind {
                    tower_bridge_core::alerts::AlertKind::Triggered => {
                        warn!("alert: {}", output::alert(a, unit))
                    }
                    tower_bridge_core::alerts::AlertKind::Recovered => {
                        info!("alert over: {}", output::alert(a, unit))
                    }
                },
                Event::Device(d) => info!("bridge {}", output::device(d)),
                Event::Mqtt(m) => match m {
                    MqttState::Connected { address } => info!("MQTT connected to {address}"),
                    MqttState::Connecting {
                        address,
                        error: Some(e),
                    } => warn!("MQTT {address}: {e}"),
                    MqttState::Error { message } => warn!("MQTT: {message}"),
                    _ => {}
                },
                _ => {}
            }
            let _ = wake_tx.try_send(());
        }
        while let Ok(update) = updates.try_recv() {
            match update {
                Update::Snapshot(Ok(s)) => app.set_snapshot(s),
                Update::Snapshot(Err(e)) => app.error = Some(e),
                Update::Gone(e) => {
                    app.snap = None;
                    app.error = Some(e);
                }
                // A slower reply for a span no longer shown is dropped.
                Update::History(span, h) if span == app.span => app.history = Some(h),
                Update::History(..) => {}
                Update::Done(r) => {
                    app.busy = false;
                    match r {
                        Ok(text) => app.flash(text, true),
                        Err(e) => app.flash(e, false),
                    }
                    let _ = wake_tx.try_send(());
                }
            }
        }
        // New points for the chart: a long span changes slowly.
        let every = if app.span == app::Span::Hour { 30 } else { 120 };
        if history_at.elapsed() > Duration::from_secs(every) {
            history_at = Instant::now();
            load_history(access, ctx, app.span, &tx);
        }
        let lines = log.lines();
        terminal.draw(|f| ui::draw(f, app, &lines))?;
        if crate::stop::requested() {
            return Ok(());
        }
        match keys.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(TermEvent::Key(key))) => match app.handle_key(key) {
                Action::None => {}
                Action::Quit => return Ok(()),
                Action::Span => {
                    history_at = Instant::now();
                    load_history(access, ctx, app.span, &tx);
                }
                action => {
                    app.busy = true;
                    app.flash("Working…", true);
                    act(access, action, &tx);
                }
            },
            Ok(Ok(_)) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            // The terminal is gone.
            Ok(Err(e)) => return Err(e.into()),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
