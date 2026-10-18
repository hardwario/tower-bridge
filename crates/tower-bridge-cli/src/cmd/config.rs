//! `config`: the settings.json the app and the CLI share, by dotted keys.

use anyhow::Result;
use serde_json::Value;
use tower_bridge_service::lock::InstanceLock;
use tower_bridge_service::settings::Settings;

use super::Out;
use crate::access::Ctx;
use crate::cli::ConfigAction;
use crate::{exit, output};

/// Keys whose values are secrets: shown as "(set)" by `show`.
const SECRETS: &[&str] = &["mqtt.password", "api.token"];

/// Values made up while there is no settings file (a new token on every
/// read): shown as such until the service first starts and stores them.
const GENERATED: &[&str] = &["api.token", "mqtt.client_id"];

/// The settings as JSON, with a warning on stderr when the file has problems.
fn current(file: &std::path::Path) -> Result<(Value, Option<String>)> {
    let (settings, problem) = Settings::read_checked(file);
    if let Some(problem) = &problem {
        eprintln!(
            "warning: {} has problems, some values are defaults: {problem}",
            file.display()
        );
    }
    let mut v = serde_json::to_value(settings)?;
    if !file.exists() {
        for key in GENERATED {
            if let Some(slot) = lookup_mut(&mut v, key) {
                *slot = Value::String("(made at the first start)".into());
            }
        }
    }
    Ok((v, problem))
}

pub fn config(ctx: &Ctx, out: Out, action: Option<ConfigAction>) -> Result<()> {
    let paths = ctx.paths()?;
    let file = paths.settings_file();
    match action.unwrap_or(ConfigAction::Show) {
        ConfigAction::Path => println!("{}", file.display()),
        ConfigAction::Show => {
            let (mut v, _) = current(&file)?;
            for key in SECRETS {
                if let Some(secret) = lookup_mut(&mut v, key)
                    && secret
                        .as_str()
                        .is_some_and(|s| !s.is_empty() && !s.starts_with('('))
                {
                    *secret = Value::String("(set)".into());
                }
            }
            if out.json {
                output::json(&v)?;
            } else {
                println!("{}", serde_json::to_string_pretty(&v)?);
            }
        }
        ConfigAction::Get { key } => {
            let (v, _) = current(&file)?;
            let value = lookup(&v, &key).ok_or_else(|| unknown(&v, &key))?;
            match value {
                Value::String(s) if !out.json => println!("{s}"),
                other => println!("{}", serde_json::to_string_pretty(other)?),
            }
        }
        ConfigAction::Set { key, value } => {
            if let Some(holder) = InstanceLock::holder(&paths.lock_file()) {
                return Err(crate::failure(
                    exit::BUSY,
                    format!("{holder} is running and would overwrite the change"),
                    Some("change it there (in the app: Settings), or stop it first"),
                ));
            }
            let (settings, problem) = Settings::read_checked(&file);
            // Saving would write the defaults over what could not be read.
            if let Some(problem) = problem {
                return Err(crate::failure(
                    exit::ERROR,
                    format!("{} has problems: {problem}", file.display()),
                    Some(
                        "fix the file, or start the app or `tower-bridge-cli run` once: it keeps a backup and starts from defaults",
                    ),
                ));
            }
            let mut v = serde_json::to_value(&settings)?;
            let unknown_key = unknown(&v, &key);
            let slot = lookup_mut(&mut v, &key).ok_or(unknown_key)?;
            *slot = match slot {
                Value::Object(group) => {
                    let keys: Vec<String> = group.keys().map(|k| format!("{key}.{k}")).collect();
                    return Err(crate::failure(
                        exit::USAGE,
                        format!("{key} is a group of settings"),
                        Some(&format!("set them one by one: {}", keys.join(", "))),
                    ));
                }
                // Text stays text: a password of digits is no number.
                Value::String(_) => match serde_json::from_str::<Value>(&value) {
                    Ok(Value::String(s)) => Value::String(s),
                    _ => Value::String(value),
                },
                // Elsewhere JSON (true, 20000, null), else the text, which
                // the type check below then explains.
                _ => serde_json::from_str(&value).unwrap_or(Value::String(value)),
            };
            let new: Settings = serde_json::from_value(v).map_err(|e| {
                crate::failure(
                    exit::USAGE,
                    format!("{key}: {e}"),
                    Some("see `config get` for the current value and its type"),
                )
            })?;
            let new = new.validated();
            new.save(&file)?;
            let stored = serde_json::to_value(&new)?;
            let shown = if SECRETS.contains(&key.as_str()) {
                Value::String("(set)".into())
            } else {
                lookup(&stored, &key).cloned().unwrap_or(Value::Null)
            };
            if out.json {
                output::json(&shown)?;
            } else {
                println!("{key} = {shown}");
            }
        }
    }
    Ok(())
}

fn lookup<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.').try_fold(v, |v, part| v.get(part))
}

fn lookup_mut<'a>(v: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    key.split('.').try_fold(v, |v, part| v.get_mut(part))
}

/// An unknown key, with the keys that exist where it went wrong.
fn unknown(v: &Value, key: &str) -> anyhow::Error {
    let mut at = v;
    let mut prefix = String::new();
    for part in key.split('.') {
        match at.get(part) {
            Some(next) if next.is_object() => {
                at = next;
                prefix = format!("{prefix}{part}.");
            }
            _ => break,
        }
    }
    let keys: Vec<String> = at
        .as_object()
        .map(|m| m.keys().map(|k| format!("{prefix}{k}")).collect())
        .unwrap_or_default();
    crate::failure(
        exit::USAGE,
        format!("no setting {key:?}"),
        Some(&format!("settings here: {}", keys.join(", "))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_keys() {
        let v = serde_json::to_value(Settings::default()).unwrap_or_default();
        assert_eq!(lookup(&v, "mqtt.port"), Some(&Value::from(1883)));
        assert!(lookup(&v, "alerts.temperature.max").is_some());
        assert!(lookup(&v, "mqtt.nope").is_none());
        let e = unknown(&v, "mqtt.nope").to_string();
        assert_eq!(e, "no setting \"mqtt.nope\"");
    }
}
