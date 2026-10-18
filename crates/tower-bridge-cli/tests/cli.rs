//! The command line's contract, without hardware: help, usage errors and
//! exit codes, and the commands against the simulated bridge in a data folder
//! of their own.

use std::process::{Child, Command as Process, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::Command;
use predicates::prelude::*;

/// The CLI with its own data folder, untouched by the developer's shell.
fn cli(data: &tempfile::TempDir) -> Command {
    let mut cmd = Command::cargo_bin("tower-bridge-cli").unwrap_or_else(|e| panic!("{e}"));
    cmd.env_remove("TOWER_BRIDGE_DATA_DIR")
        .env_remove("TOWER_BRIDGE_SIMULATE")
        .env_remove("RUST_LOG")
        .env("NO_COLOR", "1")
        .arg("--data-dir")
        .arg(data.path());
    cmd
}

fn data() -> tempfile::TempDir {
    tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn help_lists_the_commands() {
    let d = data();
    let help = cli(&d).arg("--help").assert().success();
    let out = String::from_utf8_lossy(&help.get_output().stdout).to_string();
    for word in [
        "[COMMAND]",
        "dashboard",
        "run",
        "status",
        "watch",
        "relay",
        "gps",
        "nfc",
        "identify",
        "rescan",
        "export",
        "alerts",
        "config",
        "mcp",
        "toolbox",
        "Exit codes",
    ] {
        assert!(out.contains(word), "help lacks {word:?}");
    }
    cli(&d)
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn usage_errors_exit_2() {
    let d = data();
    cli(&d).arg("frobnicate").assert().code(2);
    cli(&d)
        .args(["relay", "pulse", "--ms", "10"])
        .assert()
        .code(2);
    cli(&d)
        .args(["export", "--last", "7d", "--from", "2026-10-01"])
        .assert()
        .code(2);
    cli(&d)
        .args(["--color", "rainbow", "status"])
        .assert()
        .code(2);
    // No terminal here: no dashboard.
    cli(&d)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("needs a terminal"));
    cli(&d).arg("dashboard").assert().code(2);
    cli(&d)
        .args(["nfc", "write", &"x".repeat(300)])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("240"));
}

#[test]
fn status_relay_and_nfc_on_the_simulated_bridge() {
    let d = data();
    cli(&d)
        .args(["--simulate", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TOWER Climate Module R2"))
        .stdout(predicate::str::contains("Temperature"));
    let json = cli(&d)
        .args(["--simulate", "--json", "status"])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&json.get_output().stdout).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(v["status"]["device"]["status"], "ready");
    assert_eq!(v["via"], "this process");
    assert!(
        v["status"]["latest"]
            .as_array()
            .is_some_and(|l| l.len() == 2),
        "a reading per channel"
    );

    cli(&d)
        .args(["--simulate", "relay", "true"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TRUE"));
    cli(&d)
        .args(["--simulate", "nfc", "write", "hardwario.com"])
        .assert()
        .success()
        .stdout(predicate::str::contains("https://hardwario.com"));
    cli(&d).args(["--simulate", "identify"]).assert().success();
}

#[test]
fn watch_prints_ndjson() {
    let d = data();
    let out = cli(&d)
        .args(["--simulate", "--json", "watch", "--count", "2"])
        .timeout(Duration::from_secs(60))
        .assert()
        .success();
    let readings: Vec<serde_json::Value> = String::from_utf8_lossy(&out.get_output().stdout)
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|v: &serde_json::Value| v["type"] == "reading")
        .collect();
    assert_eq!(readings.len(), 2);
    assert!(readings[0]["temperature"].is_number());
}

#[test]
fn config_reads_and_checks_values() {
    let d = data();
    cli(&d)
        .args(["config", "set", "sample_interval_ms", "30000"])
        .assert()
        .success();
    cli(&d)
        .args(["config", "get", "sample_interval_ms"])
        .assert()
        .success()
        .stdout("30000\n");
    cli(&d)
        .args(["config", "set", "mqtt.host", "broker.local"])
        .assert()
        .success();
    cli(&d)
        .args(["config", "get", "mqtt.host"])
        .assert()
        .success()
        .stdout("broker.local\n");
    cli(&d)
        .args(["config", "set", "mqtt.nope", "1"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("mqtt.host"));
    cli(&d)
        .args(["config", "set", "sample_interval_ms", "soon"])
        .assert()
        .code(2);
    // The token is a secret.
    cli(&d)
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"(set)\""));
}

#[test]
fn export_without_history_is_not_found() {
    let d = data();
    cli(&d).arg("export").assert().code(4);
    cli(&d).args(["export", "--last", "soon"]).assert().code(2);
}

/// Stops the background service when the test ends, also on failure.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_running_service_holds_the_bridge() {
    let d = data();
    let bin = assert_cmd::cargo::cargo_bin("tower-bridge-cli");
    let child = Process::new(&bin)
        .env_remove("TOWER_BRIDGE_DATA_DIR")
        .arg("--data-dir")
        .arg(d.path())
        .args(["--simulate", "run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"));
    let _running = Running(child);
    // A command started before the service took its lock would open the
    // bridge itself: wait for the lock.
    let lock = d.path().join("simulated").join("tower-bridge.lock");
    let deadline = Instant::now() + Duration::from_secs(20);
    while tower_bridge_service::lock::InstanceLock::holder(&lock).is_none() {
        assert!(Instant::now() < deadline, "the service never started");
        std::thread::sleep(Duration::from_millis(100));
    }
    // Its API server is off by default: commands cannot reach it.
    cli(&d)
        .args(["--simulate", "status"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("API server"));
    cli(&d).args(["--simulate", "run"]).assert().code(3);
    cli(&d)
        .args(["--simulate", "config", "set", "mqtt.port", "1884"])
        .assert()
        .code(3);
}

#[test]
fn the_simulation_variable_takes_1_as_the_app_does() {
    let d = data();
    cli(&d)
        .env("TOWER_BRIDGE_SIMULATE", "1")
        .args(["--json", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"ready\""));
    cli(&d).args(["watch", "--count", "0"]).assert().code(2);
}

#[test]
fn config_keeps_text_text_and_refuses_damaged_files() {
    let d = data();
    // Digits stay a password, and it is not echoed.
    cli(&d)
        .args(["config", "set", "mqtt.password", "123456"])
        .assert()
        .success()
        .stdout(predicate::str::contains("123456").not());
    cli(&d)
        .args(["config", "get", "mqtt.password"])
        .assert()
        .success()
        .stdout("123456\n");
    cli(&d)
        .args(["config", "set", "mqtt", "{}"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("mqtt.host"));
    // A file with an error is not overwritten with defaults.
    let file = d.path().join("settings.json");
    std::fs::write(&file, "{ \"mqtt\": { \"host\": \"x\", } }").unwrap_or_else(|e| panic!("{e}"));
    cli(&d)
        .args(["config", "set", "mqtt.host", "y"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("problems"));
    assert!(
        std::fs::read_to_string(&file)
            .unwrap_or_default()
            .contains("\"x\", }"),
        "untouched"
    );
}
