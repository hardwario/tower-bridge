//! Update notices. Once a day the app asks GitHub for the newest published
//! release and, when it is newer than this version, says so: a tray item, a
//! banner on the Dashboard and one notification per version. Nothing is
//! downloaded or installed; the user takes the new version from its release
//! page (the user chose this over in-app installs, 2026-10-23).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

use crate::i18n::fill;
use crate::notify;
use crate::state::AppState;

const REPOSITORY: &str = "https://github.com/hardwario/tower-bridge";
/// The newest release that is neither a draft nor a pre-release.
const LATEST: &str = "https://api.github.com/repos/hardwario/tower-bridge/releases/latest";
/// After the start: at login the network may not be up yet.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const INTERVAL: Duration = Duration::from_secs(24 * 3600);
/// After a failed check: offline, or GitHub's limit of 60 requests an hour
/// per address reached (many computers behind one address).
const RETRY: Duration = Duration::from_secs(3600);
/// The schedule follows the wall clock, read this often: the monotonic clock
/// stands still while a Mac sleeps, so a day of it can take weeks.
const POLL: Duration = Duration::from_secs(600);
const TIMEOUT: Duration = Duration::from_secs(30);
/// The version the last notification was about, in the data folder.
const NOTIFIED_FILE: &str = "update-notified.txt";

/// What the app knows about newer versions, for the dashboard
/// (`update-state` events) and the tray.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct UpdateState {
    /// A check is under way.
    pub checking: bool,
    /// The newest release, when it is newer than this version.
    pub available: Option<Release>,
    /// When the last check ended (Unix ms).
    pub last_check: Option<i64>,
    /// Why the last check failed; a release found before stays known.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Release {
    /// Without the tag's "v": "1.0.2".
    pub version: String,
    /// Its page on GitHub, with the downloads and the changes.
    pub url: String,
}

/// The update checker: state and a thread asking GitHub.
#[derive(Clone)]
pub struct Updates {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<UpdateState>,
    /// Wakes the thread for a check now.
    wake: Sender<()>,
}

impl Updates {
    pub fn start(app: &AppHandle, data_dir: PathBuf) -> Self {
        // One pending request is enough: more clicks on "Check now" during a
        // check add nothing.
        let (wake, rx) = crossbeam_channel::bounded(1);
        let updates = Self {
            inner: Arc::new(Inner {
                state: Mutex::default(),
                wake,
            }),
        };
        let checker = Checker {
            app: app.clone(),
            updates: updates.clone(),
            current: app.package_info().version.clone(),
            // A local file server in place of GitHub, for testing.
            url: std::env::var("TOWER_BRIDGE_UPDATE_URL").unwrap_or_else(|_| LATEST.into()),
            notified: data_dir.join(NOTIFIED_FILE),
        };
        if let Err(e) = std::thread::Builder::new()
            .name("updates".into())
            .spawn(move || checker.run(rx))
        {
            warn!("cannot start the update check: {e}");
        }
        updates
    }

    pub fn state(&self) -> UpdateState {
        self.inner.state.lock().clone()
    }

    /// Check now, also while the automatic checks are off ("Check now").
    pub fn check_now(&self) {
        let _ = self.inner.wake.try_send(());
    }

    /// The automatic checks were switched on or off in the settings. Off
    /// forgets what is known, so the tray item and the banner go away.
    pub fn set_enabled(&self, app: &AppHandle, enabled: bool) {
        if enabled {
            self.check_now();
        } else {
            self.publish(app, UpdateState::default());
        }
    }

    fn publish(&self, app: &AppHandle, state: UpdateState) {
        *self.inner.state.lock() = state.clone();
        let _ = app.emit("update-state", &state);
        if let Some(app_state) = app.try_state::<AppState>() {
            app_state.tray.set_update(
                state.available.map(|r| r.version),
                crate::i18n::current(app),
            );
        }
    }
}

/// Open the release page of the newer version.
pub fn open_release(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let url = app
        .try_state::<Updates>()
        .and_then(|u| u.state().available)
        .map(|r| r.url)
        .ok_or("no newer version known")?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

struct Checker {
    app: AppHandle,
    updates: Updates,
    current: Version,
    url: String,
    notified: PathBuf,
}

impl Checker {
    fn run(self, wake: Receiver<()>) {
        let mut due = SystemTime::now() + FIRST_CHECK;
        loop {
            // While off, only "Check now" or switching the checks on wakes it.
            let wait = if self.enabled() {
                due.duration_since(SystemTime::now())
                    .unwrap_or_default()
                    .min(POLL)
            } else {
                POLL
            };
            let asked = match wake.recv_timeout(wait) {
                Ok(()) => true,
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if !asked && (!self.enabled() || SystemTime::now() < due) {
                continue;
            }

            let mut state = self.updates.state();
            state.checking = true;
            self.updates.publish(&self.app, state);
            let result = fetch(&self.url, &self.current);
            due = SystemTime::now() + if result.is_ok() { INTERVAL } else { RETRY };
            let mut state = self.updates.state();
            state.checking = false;
            // Switched off during the check: the result is not wanted.
            if !asked && !self.enabled() {
                self.updates.publish(&self.app, state);
                continue;
            }
            state.last_check = Some(chrono::Utc::now().timestamp_millis());
            match result {
                Ok(available) => {
                    state.available = available;
                    state.error = None;
                }
                Err(e) => {
                    warn!("update check failed: {e}");
                    state.error = Some(e);
                }
            }
            if let Some(release) = &state.available {
                self.notify_once(release, !asked);
            }
            self.updates.publish(&self.app, state);
        }
    }

    fn enabled(&self) -> bool {
        self.app
            .try_state::<AppState>()
            .is_some_and(|s| s.settings().check_for_updates)
    }

    /// One notification per version, also across restarts. A version found
    /// by "Check now" counts as told, since the user is looking at it.
    fn notify_once(&self, release: &Release, notify: bool) {
        let told = std::fs::read_to_string(&self.notified).unwrap_or_default();
        if told.trim() == release.version {
            return;
        }
        info!("TOWER Bridge {} is available", release.version);
        if let Err(e) = std::fs::write(&self.notified, &release.version) {
            warn!("cannot write {}: {e}", self.notified.display());
        }
        if notify {
            let tr = crate::i18n::current(&self.app).tr();
            notify::send(
                &self.app,
                &fill(tr.update_title, &[("version", &release.version)]),
                tr.update_body,
            );
        }
    }
}

/// Ask for the newest release; it, when it is newer than `current`.
fn fetch(url: &str, current: &Version) -> Result<Option<Release>, String> {
    let body = agent()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let latest: Latest = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    newer(&latest, current)
}

/// The part of GitHub's release that matters here.
#[derive(Debug, Deserialize)]
struct Latest {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// The release, when it is newer than `current`. Its page is built from the
/// checked tag rather than taken from the answer, so the app never opens
/// an address it got from the network.
fn newer(latest: &Latest, current: &Version) -> Result<Option<Release>, String> {
    if latest.draft || latest.prerelease {
        return Ok(None);
    }
    let tag = &latest.tag_name;
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map_err(|e| format!("unexpected release tag {tag:?}: {e}"))?;
    Ok((version > *current).then(|| Release {
        version: version.to_string(),
        url: format!("{REPOSITORY}/releases/tag/{tag}"),
    }))
}

/// An HTTPS client trusting the system's root certificates, as MQTT does
/// (a company's own CA included), with the TLS provider already linked in.
fn agent() -> ureq::Agent {
    use ureq::tls::{Certificate, RootCerts, TlsConfig, TlsProvider};
    let found = rustls_native_certs::load_native_certs();
    let roots: Vec<Certificate<'static>> = found
        .certs
        .iter()
        .map(|c| Certificate::from_der(c.as_ref()).to_owned())
        .collect();
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::from(roots))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(TIMEOUT))
        // GitHub's API wants one; nothing about the computer or the version.
        .user_agent("tower-bridge")
        .build()
        .new_agent()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latest(tag: &str) -> Latest {
        Latest {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
        }
    }

    fn current() -> Version {
        Version::new(1, 0, 1)
    }

    #[test]
    fn a_newer_release_is_found_with_its_page() {
        let found = newer(&latest("v1.0.2"), &current());
        assert_eq!(
            found,
            Ok(Some(Release {
                version: "1.0.2".into(),
                url: "https://github.com/hardwario/tower-bridge/releases/tag/v1.0.2".into(),
            }))
        );
        assert_eq!(
            newer(&latest("1.1.0"), &current()).map(|r| r.map(|r| r.version)),
            Ok(Some("1.1.0".into()))
        );
    }

    #[test]
    fn the_same_or_an_older_release_is_not_an_update() {
        assert_eq!(newer(&latest("v1.0.1"), &current()), Ok(None));
        assert_eq!(newer(&latest("v1.0.0"), &current()), Ok(None));
        // A development build of the next version is newer than its release candidate.
        let dev = Version::parse("1.1.0-rc.1").ok();
        assert!(dev.is_some_and(|v| newer(&latest("v1.0.9"), &v) == Ok(None)));
    }

    #[test]
    fn drafts_and_pre_releases_are_ignored() {
        let mut pre = latest("v2.0.0-beta.1");
        pre.prerelease = true;
        assert_eq!(newer(&pre, &current()), Ok(None));
        let mut draft = latest("v2.0.0");
        draft.draft = true;
        assert_eq!(newer(&draft, &current()), Ok(None));
    }

    #[test]
    fn a_tag_that_is_no_version_is_an_error() {
        assert!(newer(&latest("nightly"), &current()).is_err());
        // Nothing but a version can reach the page address.
        assert!(newer(&latest("v1.0.2/../../evil"), &current()).is_err());
    }

    /// The real request, TLS and system certificates included:
    /// `cargo test -p tower-bridge -- --ignored asks_github`.
    #[test]
    #[ignore = "needs the network"]
    fn asks_github() {
        let found = fetch(LATEST, &Version::new(0, 0, 1));
        assert!(
            matches!(&found, Ok(Some(r)) if r.url.starts_with(REPOSITORY)),
            "{found:?}"
        );
    }

    #[test]
    fn github_answers_are_read() {
        let json = r#"{"url":"x","html_url":"https://github.com/hardwario/tower-bridge/releases/tag/v1.0.2",
            "tag_name":"v1.0.2","name":"v1.0.2","draft":false,"prerelease":false,"assets":[]}"#;
        let parsed: Result<Latest, _> = serde_json::from_str(json);
        assert!(parsed.is_ok_and(|l| l.tag_name == "v1.0.2" && !l.draft && !l.prerelease));
    }
}
