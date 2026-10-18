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

pub fn config(ctx: &Ctx, out: Out, action: Option<ConfigAction>) -> Result<()> {
    let paths = ctx.paths()?;
    let file = paths.settings_file();
    match action.unwrap_or(ConfigAction::Show) {
        ConfigAction::Path => println!("{}", file.display()),
        ConfigAction::Show => {
            let mut v = serde_json::to_value(Settings::read(&file))?;
            for key in SECRETS {
                if let Some(secret) = lookup_mut(&mut v, key)
                    && secret.as_str().is_some_and(|s| !s.is_empty())
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
            let v = serde_json::to_value(Settings::read(&file))?;
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
                    Some("change it there (the app's Settings), or stop it first"),
                ));
            }
            let settings = Settings::read(&file);
            let mut v = serde_json::to_value(&settings)?;
            let unknown_key = unknown(&v, &key);
            let slot = lookup_mut(&mut v, &key).ok_or(unknown_key)?;
            // JSON where it parses (true, 20000, null, "x"), else the text.
            *slot = serde_json::from_str(&value).unwrap_or(Value::String(value));
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
            let shown = lookup(&stored, &key).cloned().unwrap_or(Value::Null);
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
