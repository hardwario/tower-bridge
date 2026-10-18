//! Tauri commands used by the dashboard.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tower_bridge_core::acquisition::{AcqError, RescanOutcome};
use tower_bridge_core::alerts::ActiveAlert;
use tower_bridge_core::gps::GpsInfo;
use tower_bridge_core::model::{DeviceState, Reading};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;
use tower_bridge_core::store::{self, AlertLogEntry, CsvOptions, History, HistoryQuery};

use crate::settings::{Settings, TemperatureUnit};
use crate::state::{ApiState, AppState, MqttState};

#[derive(Debug, Serialize)]
pub struct CmdError {
    /// Machine-readable reason the dashboard can translate (e.g. `notConnected`).
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    message: String,
}

impl<E: std::fmt::Display> From<E> for CmdError {
    fn from(e: E) -> Self {
        Self {
            code: None,
            message: e.to_string(),
        }
    }
}

fn acq_error(e: AcqError) -> CmdError {
    CmdError {
        code: Some(match e {
            AcqError::NotConnected => "not_connected",
            AcqError::NoRelay => "no_relay",
            AcqError::NoNfc => "no_nfc",
            AcqError::Nfc(_) => "nfc",
            AcqError::Bus(_) => "bus",
        }),
        message: e.to_string(),
    }
}

type CmdResult<T> = Result<T, CmdError>;

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AppInfo {
    version: String,
    data_dir: PathBuf,
    portable: bool,
    simulated: bool,
    os: &'static str,
    /// Supported language of the operating system (for "System" language).
    system_language: crate::i18n::Lang,
    /// Executable for MCP clients that launch `tower-bridge --mcp-stdio`.
    exe_path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Snapshot {
    device: DeviceState,
    relay: Option<RelayInfo>,
    nfc: Option<NfcInfo>,
    gps: Option<GpsInfo>,
    /// The latest reading of each I2C channel.
    latest: Vec<Reading>,
    settings: Settings,
    open_at_login: bool,
    mqtt: MqttState,
    api: ApiState,
    active_alerts: Vec<ActiveAlert>,
    app: AppInfo,
}

#[tauri::command]
pub fn get_snapshot(app: AppHandle, state: State<'_, AppState>) -> Snapshot {
    Snapshot {
        device: state.device.read().clone(),
        relay: *state.relay.read(),
        nfc: state.nfc.read().clone(),
        gps: state.gps.read().clone(),
        latest: state.latest.read().clone(),
        settings: (*state.settings()).clone(),
        open_at_login: crate::autostart::is_enabled(&app),
        mqtt: state.mqtt.read().clone(),
        api: state.api.read().clone(),
        active_alerts: state.alerts.read().clone(),
        app: AppInfo {
            version: app.package_info().version.to_string(),
            data_dir: state.paths.data_dir.clone(),
            portable: state.paths.portable,
            simulated: state.simulated,
            os: std::env::consts::OS,
            system_language: crate::i18n::Lang::system(),
            exe_path: std::env::var("APPIMAGE").ok().or_else(|| {
                std::env::current_exe()
                    .ok()
                    .map(|p| p.display().to_string())
            }),
        },
    }
}

/// Time span of the stored readings, for panning through history.
#[derive(Serialize)]
pub struct DataRange {
    first: i64,
    last: i64,
}

#[tauri::command]
pub async fn get_data_range(app: AppHandle) -> CmdResult<Option<DataRange>> {
    let db = app.state::<AppState>().paths.db_file();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = store::open_reader(&db)?;
        store::data_range(&conn)
    })
    .await?
    .map(|r| r.map(|(first, last)| DataRange { first, last }))
    .map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_history(
    app: AppHandle,
    from: i64,
    to: i64,
    max_points: Option<usize>,
) -> CmdResult<History> {
    let state = app.state::<AppState>();
    let db = state.paths.db_file();
    let interval_ms = i64::try_from(state.settings().sample_interval_ms).unwrap_or(i64::MAX);
    let query = HistoryQuery {
        from,
        to,
        max_points: max_points.unwrap_or(1000).clamp(10, 5000),
        sample_interval_ms: interval_ms,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let conn = store::open_reader(&db)?;
        store::history(&conn, &query)
    })
    .await?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_alert_log(app: AppHandle, limit: Option<usize>) -> CmdResult<Vec<AlertLogEntry>> {
    let db = app.state::<AppState>().paths.db_file();
    let limit = limit.unwrap_or(50).min(1000);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = store::open_reader(&db)?;
        store::alert_log(&conn, limit)
    })
    .await?
    .map_err(Into::into)
}

#[tauri::command]
pub fn save_settings(app: AppHandle, settings: Settings) -> CmdResult<Settings> {
    let saved = crate::actions::apply_settings(&app, settings)?;
    Ok((*saved).clone())
}

#[tauri::command]
pub fn set_open_at_login(app: AppHandle, enabled: bool) -> CmdResult<bool> {
    crate::autostart::set(&app, enabled).map_err(|message| CmdError {
        code: None,
        message,
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExportResult {
    path: PathBuf,
    rows: u64,
}

#[derive(Clone, Serialize)]
struct ExportProgress {
    done: u64,
    total: u64,
}

#[tauri::command]
pub async fn export_csv(app: AppHandle, from: i64, to: i64) -> CmdResult<Option<ExportResult>> {
    let state = app.state::<AppState>();
    let db = state.paths.db_file();
    let settings = state.settings();
    let name = format!(
        "tower-bridge-{}.csv",
        chrono::Local::now().format("%Y%m%d-%H%M")
    );
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<Option<ExportResult>> {
        let Some(path) = handle
            .dialog()
            .file()
            .set_title(crate::i18n::current(&handle).tr().export_title)
            .set_file_name(name)
            .add_filter("CSV", &["csv"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = path.into_path()?;
        let conn = store::open_reader(&db)?;
        let opts = CsvOptions {
            fahrenheit: settings.temperature_unit == TemperatureUnit::Fahrenheit,
            sea_level_pa: settings.sea_level_pressure_pa,
        };
        let rows = store::export_to_file(&conn, &path, from, to, opts, |done, total| {
            let _ = handle.emit("export-progress", ExportProgress { done, total });
            true
        })?;
        Ok(Some(ExportResult { path, rows }))
    })
    .await?
}

/// Excel number format for the time column when the UI sends none or an odd one.
const DEFAULT_XLSX_DATETIME: &str = "yyyy-mm-dd hh:mm:ss";

/// Accept a short, single-section Excel number format.
fn xlsx_datetime_format(f: Option<String>) -> String {
    f.filter(|f| {
        !f.is_empty() && f.len() <= 64 && !f.contains(';') && !f.chars().any(char::is_control)
    })
    .unwrap_or_else(|| DEFAULT_XLSX_DATETIME.into())
}

/// Export readings in `[from, to)` as an Excel workbook (localised headers,
/// timestamps in the user's date and time format).
#[tauri::command(rename_all = "snake_case")]
pub async fn export_xlsx(
    app: AppHandle,
    from: i64,
    to: i64,
    datetime_format: Option<String>,
) -> CmdResult<Option<ExportResult>> {
    let state = app.state::<AppState>();
    let db = state.paths.db_file();
    let settings = state.settings();
    let name = format!(
        "tower-bridge-{}.xlsx",
        chrono::Local::now().format("%Y%m%d-%H%M")
    );
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<Option<ExportResult>> {
        let tr = crate::i18n::current(&handle).tr();
        let Some(path) = handle
            .dialog()
            .file()
            .set_title(tr.export_title)
            .set_file_name(name)
            .add_filter("Excel", &["xlsx"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = path.into_path()?;
        let conn = store::open_reader(&db)?;
        let fahrenheit = settings.temperature_unit == TemperatureUnit::Fahrenheit;
        let opts = CsvOptions {
            fahrenheit,
            sea_level_pa: settings.sea_level_pressure_pa,
        };
        use tower_bridge_core::model::Metric;
        let labels = store::XlsxLabels {
            datetime_format: xlsx_datetime_format(datetime_format),
            sheet: tr.xlsx_sheet.into(),
            time: tr.xlsx_time.into(),
            device: tr.xlsx_device.into(),
            channel: tr.xlsx_channel.into(),
            temperature: format!(
                "{} ({})",
                tr.metric(Metric::Temperature),
                if fahrenheit { "°F" } else { "°C" }
            ),
            humidity: format!("{} (%)", tr.metric(Metric::Humidity)),
            illuminance: format!("{} (lx)", tr.metric(Metric::Illuminance)),
            pressure: format!("{} (hPa)", tr.metric(Metric::Pressure)),
            altitude: format!("{} (m)", tr.altitude),
            co2: format!("{} (ppm)", tr.metric(Metric::Co2)),
            voc: format!("{} (ppb)", tr.metric(Metric::Voc)),
        };
        let rows =
            store::export_xlsx_to_file(&conn, &path, from, to, opts, &labels, |done, total| {
                let _ = handle.emit("export-progress", ExportProgress { done, total });
                true
            })?;
        Ok(Some(ExportResult { path, rows }))
    })
    .await?
}

#[tauri::command]
pub fn send_test_notification(app: AppHandle) {
    let tr = crate::i18n::current(&app).tr();
    crate::notify::send(&app, "TOWER Bridge", tr.test_body);
}

/// Blink the bridge LED; resolves when the blinking has finished.
#[tauri::command]
pub async fn identify_device(app: AppHandle) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || crate::actions::identify(&app))
        .await?
        .map_err(acq_error)
}

/// Switch the Relay Module to `true` or `false`; resolves with its new state.
#[tauri::command]
pub async fn set_relay(app: AppHandle, state: bool) -> CmdResult<RelayInfo> {
    tauri::async_runtime::spawn_blocking(move || crate::actions::set_relay(&app, state))
        .await?
        .map_err(acq_error)
}

/// Open a position on OpenStreetMap in the system browser (the URL is built
/// here: the dashboard cannot open arbitrary addresses).
#[tauri::command]
pub fn open_map(app: AppHandle, latitude: f64, longitude: f64) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err(CmdError::from(format!(
            "no position: {latitude}, {longitude}"
        )));
    }
    let url = format!(
        "https://www.openstreetmap.org/?mlat={latitude:.6}&mlon={longitude:.6}#map=17/{latitude:.6}/{longitude:.6}"
    );
    app.opener().open_url(url, None::<&str>)?;
    Ok(())
}

/// Write a URL to the NFC Tag; resolves with what the tag holds afterwards.
#[tauri::command]
pub async fn write_nfc_url(app: AppHandle, url: String) -> CmdResult<NfcInfo> {
    tauri::async_runtime::spawn_blocking(move || crate::actions::write_nfc_url(&app, url))
        .await?
        .map_err(acq_error)
}

/// Timed relay pulse (defaults: opposite state, configured length);
/// resolves once the relay has switched over.
#[tauri::command(rename_all = "snake_case")]
pub async fn pulse_relay(
    app: AppHandle,
    state: Option<bool>,
    duration_ms: Option<f64>,
) -> CmdResult<RelayInfo> {
    let duration = duration_ms.and_then(tower_bridge_core::relay::pulse_from_ms);
    tauri::async_runtime::spawn_blocking(move || crate::actions::pulse_relay(&app, state, duration))
        .await?
        .map_err(acq_error)
}

/// Detect the Climate Module again; resolves with what was found.
#[tauri::command]
pub async fn rescan_device(app: AppHandle) -> CmdResult<RescanOutcome> {
    tauri::async_runtime::spawn_blocking(move || crate::actions::rescan(&app))
        .await?
        .map_err(acq_error)
}

#[tauri::command]
pub fn open_data_folder(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(state.paths.data_dir.display().to_string(), None::<&str>)?;
    Ok(())
}

#[tauri::command]
pub fn reveal_file(app: AppHandle, path: PathBuf) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(path)?;
    Ok(())
}

/// External pages the dashboard may open (fixed allowlist, no arbitrary URLs).
pub const LINKS: &[(&str, &str)] = &[
    ("docs", "https://docs.hardwario.com/tower/"),
    ("store", "https://www.hardwario.store/"),
    ("academy", "https://hardwario.academy/"),
];

/// Open one of [`LINKS`] in the system browser.
#[tauri::command]
pub fn open_link(app: AppHandle, target: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let url = LINKS
        .iter()
        .find(|(key, _)| *key == target)
        .map(|(_, url)| *url)
        .ok_or_else(|| CmdError::from(format!("unknown link: {target}")))?;
    app.opener().open_url(url, None::<&str>)?;
    Ok(())
}
