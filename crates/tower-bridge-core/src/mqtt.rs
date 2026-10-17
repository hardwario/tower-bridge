//! Embedded MQTT broker (rumqttd) and the publisher that feeds it.
//!
//! The broker runs in-process; readings are injected through a local link
//! as retained QoS 0 messages under a configurable prefix (default `tower`):
//!
//! ```text
//! tower/temperature    °C
//! tower/humidity       %RH
//! tower/illuminance    lux
//! tower/pressure       Pa
//! tower/altitude       m
//! tower/relay          true | false | null (unknown); Relay Module, when present
//! ```
//!
//! Commands for the Relay Module are accepted on (never retained-forwarded):
//!
//! ```text
//! tower/relay/set      true | false
//! tower/relay/pulse    empty, a duration in ms, true | false, or
//!                      {"state": false, "duration_ms": 2000}
//! ```
//!
//! rumqttd cannot be stopped once started; the app restarts itself to stop it.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use crossbeam_channel::Sender;
use rumqttd::local::LinkTx;
use rumqttd::protocol::{Packet, Publish};
use rumqttd::{Broker, Config, ConnectionSettings, Notification, RouterConfig, ServerSettings};
use tracing::{info, warn};

use crate::model::Reading;
use crate::relay::{RelayInfo, parse_state, pulse_from_ms};

pub const LOCAL_CLIENT_ID: &str = "tower-bridge";
pub const DEFAULT_PREFIX: &str = "tower";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerConfig {
    pub bind: IpAddr,
    pub port: u16,
    /// Require these credentials from clients when set.
    pub credentials: Option<(String, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum MqttError {
    #[error("cannot listen on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    #[error("broker link failed: {0}")]
    Link(String),
    #[error("cannot start broker thread: {0}")]
    Spawn(#[from] std::io::Error),
}

fn build_config(cfg: &BrokerConfig) -> Config {
    let connections = ConnectionSettings {
        connection_timeout_ms: 60_000,
        max_payload_size: 20_480,
        max_inflight_count: 100,
        auth: cfg.credentials.as_ref().map(|(u, p)| {
            let mut m = HashMap::new();
            m.insert(u.clone(), p.clone());
            m
        }),
        external_auth: None,
        dynamic_filters: true,
    };
    let server = ServerSettings {
        name: "v4-1".into(),
        listen: SocketAddr::new(cfg.bind, cfg.port),
        tls: None,
        next_connection_delay_ms: 1,
        connections,
    };
    let mut v4 = HashMap::new();
    v4.insert("1".to_string(), server);
    Config {
        id: 0,
        router: RouterConfig {
            max_connections: 64,
            max_outgoing_packet_count: 200,
            max_segment_size: 102_400,
            max_segment_count: 10,
            custom_segment: None,
            initialized_filters: None,
            shared_subscriptions_strategy: Default::default(),
        },
        v4: Some(v4),
        v5: None,
        ws: None,
        cluster: None,
        console: None,
        bridge: None,
        prometheus: None,
        metrics: None,
    }
}

/// A command received over MQTT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MqttCommand {
    SetRelay(bool),
    /// Missing fields take the app's defaults.
    PulseRelay {
        state: Option<bool>,
        duration: Option<Duration>,
    },
}

/// Parse a message on `<prefix>/relay/set` or `<prefix>/relay/pulse`.
/// `None` for other topics.
pub fn parse_command(
    prefix: &str,
    topic: &str,
    payload: &[u8],
) -> Option<Result<MqttCommand, String>> {
    let rest = topic.strip_prefix(prefix)?.strip_prefix("/relay/")?;
    let text = String::from_utf8_lossy(payload);
    let text = text.trim();
    let word = text.trim_matches('"');
    Some(match rest {
        "set" => parse_state(word)
            .map(MqttCommand::SetRelay)
            .ok_or_else(|| format!("expected true or false, got {text:?}")),
        "pulse" => parse_pulse(text),
        _ => return None,
    })
}

fn parse_pulse(text: &str) -> Result<MqttCommand, String> {
    let pulse = |state, ms: Option<f64>| {
        let duration = match ms {
            Some(ms) => Some(pulse_from_ms(ms).ok_or_else(|| {
                "the duration must be a positive number of milliseconds".to_string()
            })?),
            None => None,
        };
        Ok(MqttCommand::PulseRelay { state, duration })
    };
    if text.is_empty() {
        return pulse(None, None);
    }
    if let Some(state) = parse_state(text.trim_matches('"')) {
        return pulse(Some(state), None);
    }
    if let Ok(ms) = text.parse::<f64>() {
        return pulse(None, Some(ms));
    }
    let json: serde_json::Value = serde_json::from_str(text).map_err(|_| {
        format!("expected a duration in ms, true, false or a JSON object, got {text:?}")
    })?;
    let state = match json.get("state") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(
            v.as_bool()
                .ok_or_else(|| "state must be true or false".to_string())?,
        ),
    };
    let ms = match json.get("duration_ms") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(
            v.as_f64()
                .ok_or_else(|| "duration_ms must be a number".to_string())?,
        ),
    };
    pulse(state, ms)
}

fn command_filter(prefix: &str) -> String {
    format!("{prefix}/relay/+")
}

/// Start the broker on background threads and return a publisher. Valid
/// commands are sent to `commands`; checking whether they are allowed is up
/// to the receiver.
pub fn start(
    cfg: &BrokerConfig,
    prefix: &str,
    commands: Sender<MqttCommand>,
) -> Result<Publisher, MqttError> {
    let addr = SocketAddr::new(cfg.bind, cfg.port);
    // rumqttd only logs bind failures from its server thread, so check first.
    drop(TcpListener::bind(addr).map_err(|source| MqttError::Bind { addr, source })?);

    let mut broker = Broker::new(build_config(cfg));
    let (mut link_tx, mut link_rx) = broker
        .link(LOCAL_CLIENT_ID)
        .map_err(|e| MqttError::Link(e.to_string()))?;
    let prefix = sanitize_prefix(prefix);
    link_tx
        .subscribe(command_filter(&prefix))
        .map_err(|e| MqttError::Link(e.to_string()))?;
    let shared_prefix = Arc::new(Mutex::new(prefix.clone()));

    std::thread::Builder::new()
        .name("mqtt-broker".into())
        .spawn(move || {
            if let Err(e) = broker.start() {
                warn!("MQTT broker stopped: {e}");
            }
        })?;
    // Receive commands (and keep the local link drained).
    let link_prefix = shared_prefix.clone();
    std::thread::Builder::new()
        .name("mqtt-link".into())
        .spawn(move || {
            while let Ok(notification) = link_rx.recv() {
                let Some(Notification::Forward(forward)) = notification else {
                    continue;
                };
                let topic = String::from_utf8_lossy(&forward.publish.topic).into_owned();
                let prefix = link_prefix.lock().map(|p| p.clone()).unwrap_or_default();
                match parse_command(&prefix, &topic, &forward.publish.payload) {
                    Some(Ok(cmd)) => {
                        let _ = commands.send(cmd);
                    }
                    Some(Err(e)) => warn!("ignored MQTT message on {topic}: {e}"),
                    None => {}
                }
            }
        })?;

    info!("MQTT broker listening on {addr}");
    Ok(Publisher {
        tx: link_tx,
        prefix,
        shared_prefix,
    })
}

/// Publishes readings into the embedded broker.
pub struct Publisher {
    tx: LinkTx,
    prefix: String,
    /// Read by the command receiver.
    shared_prefix: Arc<Mutex<String>>,
}

impl Publisher {
    pub fn set_prefix(&mut self, prefix: &str) {
        let prefix = sanitize_prefix(prefix);
        if prefix == self.prefix {
            return;
        }
        let _ = self.tx.unsubscribe(command_filter(&self.prefix));
        if let Err(e) = self.tx.subscribe(command_filter(&prefix)) {
            warn!("cannot subscribe to MQTT commands: {e}");
        }
        if let Ok(mut shared) = self.shared_prefix.lock() {
            shared.clone_from(&prefix);
        }
        self.prefix = prefix;
    }

    fn publish(&mut self, topic: String, payload: String) -> Result<(), MqttError> {
        let publish = Publish::new(Bytes::from(topic), Bytes::from(payload), true);
        pollster::block_on(self.tx.send(Packet::Publish(publish, None)))
            .map(|_| ())
            .map_err(|e| MqttError::Link(e.to_string()))
    }

    /// Relay state as a retained message: `true`, `false` or `null` when
    /// unknown; `None` (no relay) clears the topic.
    pub fn publish_relay(&mut self, relay: Option<&RelayInfo>) -> Result<(), MqttError> {
        let payload = relay
            .map(|r| r.state.map_or("null".to_string(), |s| s.to_string()))
            .unwrap_or_default();
        self.publish(format!("{}/relay", self.prefix), payload)
    }

    pub fn publish_reading(&mut self, reading: &Reading) -> Result<(), MqttError> {
        for (topic, payload) in topics(&self.prefix, reading) {
            self.publish(topic, payload)?;
        }
        Ok(())
    }
}

/// Make a topic prefix safe: no wildcards or spaces, no empty levels, no
/// leading/trailing slash. Multi-level prefixes (`home/office/tower`) are fine.
pub fn sanitize_prefix(prefix: &str) -> String {
    let levels: Vec<String> = prefix
        .split('/')
        .map(|level| {
            level
                .trim()
                .chars()
                .map(|c| match c {
                    '+' | '#' | ' ' => '-',
                    c => c,
                })
                .collect::<String>()
        })
        .filter(|level| !level.is_empty())
        .collect();
    if levels.is_empty() {
        DEFAULT_PREFIX.into()
    } else {
        levels.join("/")
    }
}

/// Topic/payload pairs for a reading (missing values are skipped, since an
/// empty retained payload would clear the topic).
pub fn topics(prefix: &str, r: &Reading) -> Vec<(String, String)> {
    let v = &r.values;
    [
        ("temperature", v.temperature.map(|t| format!("{t:.2}"))),
        ("humidity", v.humidity.map(|h| format!("{h:.1}"))),
        ("illuminance", v.illuminance.map(|l| format!("{l:.1}"))),
        ("pressure", v.pressure.map(|p| format!("{p:.2}"))),
        ("altitude", r.altitude.map(|a| format!("{a:.1}"))),
    ]
    .into_iter()
    .filter_map(|(name, payload)| payload.map(|p| (format!("{prefix}/{name}"), p)))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Values;

    #[test]
    fn topic_layout() {
        let r = Reading {
            ts: 0,
            device: "x".into(),
            values: Values {
                temperature: Some(23.456),
                humidity: Some(45.21),
                illuminance: None,
                pressure: Some(98_765.25),
            },
            altitude: Some(312.44),
        };
        assert_eq!(
            topics("tower", &r),
            vec![
                ("tower/temperature".to_string(), "23.46".to_string()),
                ("tower/humidity".to_string(), "45.2".to_string()),
                ("tower/pressure".to_string(), "98765.25".to_string()),
                ("tower/altitude".to_string(), "312.4".to_string()),
            ]
        );
    }

    #[test]
    fn prefix_sanitizing() {
        assert_eq!(sanitize_prefix("tower"), "tower");
        assert_eq!(
            sanitize_prefix(" /home//office/ tower #1/ "),
            "home/office/tower--1"
        );
        assert_eq!(sanitize_prefix("  "), "tower");
        assert_eq!(sanitize_prefix("a/+/b"), "a/-/b");
    }

    #[test]
    fn port_in_use_is_reported() -> Result<(), std::io::Error> {
        let l = TcpListener::bind("127.0.0.1:0")?;
        let port = l.local_addr()?.port();
        let cfg = BrokerConfig {
            bind: "127.0.0.1".parse().map_err(std::io::Error::other)?,
            port,
            credentials: None,
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        assert!(matches!(start(&cfg, "t", tx), Err(MqttError::Bind { .. })));
        Ok(())
    }

    /// Minimal MQTT 3.1.1 client over TCP for the end-to-end test.
    fn mqtt_packet(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![kind];
        let mut len = body.len();
        loop {
            let mut b = (len % 128) as u8;
            len /= 128;
            if len > 0 {
                b |= 0x80;
            }
            out.push(b);
            if len == 0 {
                break;
            }
        }
        out.extend_from_slice(body);
        out
    }

    fn mqtt_str(s: &str) -> Vec<u8> {
        let mut v = (s.len() as u16).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v
    }

    fn read_packet(stream: &mut std::net::TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
        use std::io::Read;
        let mut h = [0u8; 1];
        stream.read_exact(&mut h)?;
        let (mut len, mut mult) = (0usize, 1usize);
        loop {
            let mut b = [0u8; 1];
            stream.read_exact(&mut b)?;
            len += usize::from(b[0] & 0x7F) * mult;
            mult *= 128;
            if b[0] & 0x80 == 0 {
                break;
            }
        }
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body)?;
        Ok((h[0], body))
    }

    #[test]
    fn retained_readings_reach_a_late_subscriber() -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        use std::time::Duration;

        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let cfg = BrokerConfig {
            bind: "127.0.0.1".parse()?,
            port,
            credentials: None,
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut publisher = start(&cfg, "test", tx)?;
        let reading = Reading {
            ts: 0,
            device: "x".into(),
            values: Values {
                temperature: Some(21.5),
                humidity: Some(40.0),
                illuminance: Some(100.0),
                pressure: Some(100_000.0),
            },
            altitude: Some(110.0),
        };
        // Wait for the listener thread, publish before anyone subscribes.
        std::thread::sleep(Duration::from_millis(300));
        publisher.publish_reading(&reading)?;
        std::thread::sleep(Duration::from_millis(200));

        let mut s = std::net::TcpStream::connect(("127.0.0.1", port))?;
        s.set_read_timeout(Some(Duration::from_secs(3)))?;
        let mut connect = mqtt_str("MQTT");
        connect.extend_from_slice(&[4, 0x02, 0, 30]);
        connect.extend(mqtt_str("late-subscriber"));
        s.write_all(&mqtt_packet(0x10, &connect))?;
        let (kind, body) = read_packet(&mut s)?;
        assert_eq!((kind, body), (0x20, vec![0, 0]), "CONNACK");

        let mut sub = vec![0, 1];
        sub.extend(mqtt_str("test/#"));
        sub.push(0);
        s.write_all(&mqtt_packet(0x82, &sub))?;

        let mut topics = std::collections::HashMap::new();
        while topics.len() < 5 {
            let (kind, body) = read_packet(&mut s)?;
            if kind & 0xF0 != 0x30 {
                continue;
            }
            assert_eq!(kind & 0x01, 1, "retain flag set");
            let tlen = usize::from(u16::from_be_bytes([body[0], body[1]]));
            let topic = String::from_utf8(body[2..2 + tlen].to_vec())?;
            let payload = String::from_utf8(body[2 + tlen..].to_vec())?;
            topics.insert(topic, payload);
        }
        assert_eq!(
            topics.get("test/temperature").map(String::as_str),
            Some("21.50")
        );
        assert_eq!(
            topics.get("test/pressure").map(String::as_str),
            Some("100000.00")
        );
        Ok(())
    }

    #[test]
    fn parses_relay_commands() {
        let cmd = |topic: &str, payload: &str| parse_command("tower", topic, payload.as_bytes());
        assert_eq!(cmd("tower/temperature", "1"), None);
        assert_eq!(cmd("other/relay/set", "true"), None);
        assert_eq!(
            cmd("tower/relay/set", " False\n"),
            Some(Ok(MqttCommand::SetRelay(false)))
        );
        assert_eq!(
            cmd("tower/relay/set", "\"true\""),
            Some(Ok(MqttCommand::SetRelay(true)))
        );
        assert!(matches!(cmd("tower/relay/set", "on"), Some(Err(_))));
        assert_eq!(
            cmd("tower/relay/pulse", ""),
            Some(Ok(MqttCommand::PulseRelay {
                state: None,
                duration: None
            }))
        );
        assert_eq!(
            cmd("tower/relay/pulse", "1500"),
            Some(Ok(MqttCommand::PulseRelay {
                state: None,
                duration: Some(Duration::from_millis(1500))
            }))
        );
        assert_eq!(
            cmd(
                "tower/relay/pulse",
                r#"{"state": true, "duration_ms": 3000}"#
            ),
            Some(Ok(MqttCommand::PulseRelay {
                state: Some(true),
                duration: Some(Duration::from_secs(3))
            }))
        );
        assert!(matches!(cmd("tower/relay/pulse", "-5"), Some(Err(_))));
        assert!(matches!(
            cmd("tower/relay/pulse", r#"{"state": "on"}"#),
            Some(Err(_))
        ));
    }

    #[test]
    fn relay_commands_arrive_from_a_client() -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;

        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let cfg = BrokerConfig {
            bind: "127.0.0.1".parse()?,
            port,
            credentials: None,
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut publisher = start(&cfg, "home/tower", tx)?;
        std::thread::sleep(Duration::from_millis(300));
        // A changed prefix moves the command subscription.
        publisher.set_prefix("test");
        std::thread::sleep(Duration::from_millis(100));

        let mut s = std::net::TcpStream::connect(("127.0.0.1", port))?;
        s.set_read_timeout(Some(Duration::from_secs(3)))?;
        let mut connect = mqtt_str("MQTT");
        connect.extend_from_slice(&[4, 0x02, 0, 30]);
        connect.extend(mqtt_str("controller"));
        s.write_all(&mqtt_packet(0x10, &connect))?;
        read_packet(&mut s)?;
        for (topic, payload) in [
            ("home/tower/relay/set", "false"),
            ("test/relay/set", "true"),
        ] {
            let mut publish = mqtt_str(topic);
            publish.extend_from_slice(payload.as_bytes());
            s.write_all(&mqtt_packet(0x30, &publish))?;
        }
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3))?,
            MqttCommand::SetRelay(true)
        );
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        Ok(())
    }
}
