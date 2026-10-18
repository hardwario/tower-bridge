//! MQTT: a client connection to the user's broker (Mosquitto, Home
//! Assistant, ...) and the publisher that feeds it.
//!
//! Readings go out as retained QoS 0 messages under a configurable prefix
//! (default `tower`); the client reconnects by itself. The last level is the
//! I2C channel of the sensor, so sensors on both channels can be told apart:
//!
//! ```text
//! tower/temperature/0    °C
//! tower/humidity/0       %RH
//! tower/illuminance/0    lux
//! tower/pressure/0       Pa
//! tower/altitude/0       m
//! tower/co2/0            ppm (TOWER CO2 Module)
//! tower/voc/0            ppb TVOC (TOWER VOC-LP Tag)
//! tower/relay/0          true | false | null (unknown); Relay Module, when present
//! tower/gps/0/latitude   degrees (WGS 84); also longitude, altitude (m) and
//!                        satellites; GPS Module, when present
//! tower/status           online | offline (also the client's last will)
//! ```
//!
//! Commands for the Relay Module are accepted on (messages with the retain
//! flag are ignored, so a stored command never fires later):
//!
//! ```text
//! tower/relay/0/set      true | false
//! tower/relay/0/pulse    empty, a duration in ms, true | false, or
//!                        {"state": false, "duration_ms": 2000}
//! ```

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use tracing::{info, warn};

use crate::gps::GpsInfo;
use crate::model::{Metric, Reading};
use crate::relay::{RelayInfo, parse_state, pulse_from_ms};

pub const DEFAULT_PREFIX: &str = "tower";

#[derive(Debug, thiserror::Error)]
pub enum MqttError {
    #[error("MQTT client failed: {0}")]
    Link(String),
    #[error("cannot start the MQTT thread: {0}")]
    Spawn(#[from] std::io::Error),
}

/// A command received over MQTT, for the relay on `channel`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MqttCommand {
    SetRelay {
        channel: u8,
        state: bool,
    },
    /// Missing fields take the app's defaults.
    PulseRelay {
        channel: u8,
        state: Option<bool>,
        duration: Option<Duration>,
    },
}

impl MqttCommand {
    pub fn channel(&self) -> u8 {
        match self {
            MqttCommand::SetRelay { channel, .. } | MqttCommand::PulseRelay { channel, .. } => {
                *channel
            }
        }
    }
}

/// Parse a message on `<prefix>/relay/<channel>/set` or
/// `<prefix>/relay/<channel>/pulse`. `None` for other topics.
pub fn parse_command(
    prefix: &str,
    topic: &str,
    payload: &[u8],
) -> Option<Result<MqttCommand, String>> {
    let rest = topic.strip_prefix(prefix)?.strip_prefix("/relay/")?;
    let (channel, action) = rest.split_once('/')?;
    let text = String::from_utf8_lossy(payload);
    let text = text.trim();
    let word = text.trim_matches('"');
    let Ok(channel) = channel.parse::<u8>() else {
        return Some(Err(format!("{channel:?} is not an I2C channel number")));
    };
    Some(match action {
        "set" => parse_state(word)
            .map(|state| MqttCommand::SetRelay { channel, state })
            .ok_or_else(|| format!("expected true or false, got {text:?}")),
        "pulse" => parse_pulse(channel, text),
        _ => return None,
    })
}

fn parse_pulse(channel: u8, text: &str) -> Result<MqttCommand, String> {
    let pulse = |state, ms: Option<f64>| {
        let duration = match ms {
            Some(ms) => Some(pulse_from_ms(ms).ok_or_else(|| {
                "the duration must be a positive number of milliseconds".to_string()
            })?),
            None => None,
        };
        Ok(MqttCommand::PulseRelay {
            channel,
            state,
            duration,
        })
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
    let invalid =
        || format!("expected a duration in ms, true, false or a JSON object, got {text:?}");
    let json: serde_json::Value = serde_json::from_str(text).map_err(|_| invalid())?;
    // An object with only known fields: a typo must not pulse the relay.
    let Some(fields) = json.as_object() else {
        return Err(invalid());
    };
    if let Some(unknown) = fields
        .keys()
        .find(|k| !matches!(k.as_str(), "state" | "duration_ms"))
    {
        return Err(format!(
            "unknown field {unknown:?}; use state and duration_ms"
        ));
    }
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
    format!("{prefix}/relay/+/+")
}

/// What carries the app's messages (a seam for tests).
trait Link: Send {
    fn publish(&mut self, topic: String, payload: String) -> Result<(), MqttError>;
}

/// Hand a message on a command topic to `commands`. Brokers replay retained
/// messages to every new subscription (after a reconnect, too), so only live
/// messages count: a stored command must never fire.
fn on_message(
    prefix: &str,
    topic: &str,
    payload: &[u8],
    retain: bool,
    commands: &Sender<MqttCommand>,
) {
    if retain {
        if parse_command(prefix, topic, &[]).is_some() {
            warn!("ignored retained MQTT command on {topic}");
        }
        return;
    }
    match parse_command(prefix, topic, payload) {
        Some(Ok(cmd)) => {
            let _ = commands.send(cmd);
        }
        Some(Err(e)) => warn!("ignored MQTT message on {topic}: {e}"),
        None => {}
    }
}

/// The user's broker (Mosquitto, Home Assistant, ...) the app publishes to
/// as a client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientConfig {
    pub host: String,
    pub port: u16,
    /// TLS, checked against the system's root certificates.
    pub tls: bool,
    pub credentials: Option<(String, String)>,
    pub client_id: String,
}

/// News from the connection to the broker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkEvent {
    Connected,
    /// The connection failed or dropped; it is tried again by itself.
    Disconnected(String),
}

/// Longest wait between two connection attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// How long a dropped link may take to send what is queued (the clears,
/// `offline`) and its disconnect.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// The system's root certificates for TLS. rumqttc's own default panics when
/// one certificate cannot be read; here it is skipped and logged.
fn tls_config() -> rumqttc::TlsConfiguration {
    let found = rustls_native_certs::load_native_certs();
    for e in &found.errors {
        warn!("a system certificate could not be loaded: {e}");
    }
    let mut roots = rustls::RootCertStore::empty();
    let (_, ignored) = roots.add_parsable_certificates(found.certs);
    if ignored > 0 {
        warn!("{ignored} system certificates were not usable");
    }
    rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth()
        .into()
}

/// The client connection to the broker.
struct ClientLink {
    client: rumqttc::Client,
    /// Since when the link is being dropped.
    stopping: Arc<Mutex<Option<Instant>>>,
}

impl Link for ClientLink {
    // Never blocks: while the broker is away, a full queue drops the value
    // (the next reading, or the republish on reconnect, brings it back).
    fn publish(&mut self, topic: String, payload: String) -> Result<(), MqttError> {
        self.client
            .try_publish(topic, rumqttc::QoS::AtMostOnce, true, payload.into_bytes())
            .map_err(|e| MqttError::Link(e.to_string()))
    }
}

impl Drop for ClientLink {
    fn drop(&mut self) {
        if let Ok(mut s) = self.stopping.lock() {
            *s = Some(Instant::now());
        }
        // Queued behind the last messages, which go out first; the thread
        // ends when the disconnect is sent (or after `STOP_GRACE`).
        let _ = self.client.try_disconnect();
    }
}

/// Connect to the broker on a background thread, which reconnects by itself,
/// and return a publisher. Commands go to `commands`, connection news
/// to `events`. The broker shows `<prefix>/status` as `offline` when the app
/// goes away without saying goodbye (the last will).
pub fn connect(
    cfg: &ClientConfig,
    prefix: &str,
    commands: Sender<MqttCommand>,
    events: Sender<LinkEvent>,
) -> Result<Publisher, MqttError> {
    let prefix = sanitize_prefix(prefix);
    let mut options = rumqttc::MqttOptions::new(cfg.client_id.clone(), cfg.host.clone(), cfg.port);
    options.set_keep_alive(Duration::from_secs(30));
    options.set_clean_session(true);
    options.set_last_will(rumqttc::LastWill::new(
        status_topic(&prefix),
        "offline",
        rumqttc::QoS::AtLeastOnce,
        true,
    ));
    if let Some((user, password)) = &cfg.credentials {
        options.set_credentials(user.clone(), password.clone());
    }
    if cfg.tls {
        options.set_transport(rumqttc::Transport::tls_with_config(tls_config()));
    }
    let (client, mut connection) = rumqttc::Client::new(options, 256);
    let stopping: Arc<Mutex<Option<Instant>>> = Arc::default();

    let thread_client = client.clone();
    let thread_prefix = prefix.clone();
    let thread_stopping = stopping.clone();
    let address = format!("{}:{}", cfg.host, cfg.port);
    std::thread::Builder::new()
        .name("mqtt-client".into())
        .spawn(move || {
            let mut backoff = Duration::from_secs(1);
            let stopping = || thread_stopping.lock().ok().and_then(|s| *s);
            for notification in connection.iter() {
                if stopping().is_some_and(|t| t.elapsed() > STOP_GRACE) {
                    break;
                }
                match notification {
                    // Everything queued before it went out.
                    Ok(rumqttc::Event::Outgoing(rumqttc::Outgoing::Disconnect)) => break,
                    Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                        backoff = Duration::from_secs(1);
                        // A clean session forgets subscriptions: subscribe on
                        // every connect.
                        let _ = thread_client.try_subscribe(
                            command_filter(&thread_prefix),
                            rumqttc::QoS::AtMostOnce,
                        );
                        info!("connected to the MQTT broker at {address}");
                        let _ = events.send(LinkEvent::Connected);
                    }
                    Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
                        on_message(&thread_prefix, &p.topic, &p.payload, p.retain, &commands);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        if stopping().is_some() {
                            break;
                        }
                        warn!("MQTT broker at {address}: {e}");
                        let _ = events.send(LinkEvent::Disconnected(e.to_string()));
                        // Wait before the next attempt, ready to stop meanwhile.
                        let until = Instant::now() + backoff;
                        while Instant::now() < until && stopping().is_none() {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                }
            }
        })?;
    Ok(Publisher::new(
        Box::new(ClientLink { client, stopping }),
        prefix,
    ))
}

fn status_topic(prefix: &str) -> String {
    format!("{prefix}/status")
}

/// Publishes readings to the broker.
pub struct Publisher {
    link: Box<dyn Link>,
    prefix: String,
    /// Channel of the relay topic published last, to clear it when the
    /// relay goes away or moves.
    relay_channel: Option<u8>,
    /// Topics holding a retained value, to clear them when they go stale.
    published: HashSet<String>,
}

impl Publisher {
    fn new(link: Box<dyn Link>, prefix: String) -> Self {
        Self {
            link,
            prefix,
            relay_channel: None,
            published: HashSet::new(),
        }
    }

    fn publish(&mut self, topic: String, payload: String) -> Result<(), MqttError> {
        if payload.is_empty() {
            self.published.remove(&topic);
        } else {
            self.published.insert(topic.clone());
        }
        self.link.publish(topic, payload)
    }

    /// `<prefix>/status`: `online` while the app publishes, `offline` when
    /// it stops (and the last will, if the app vanishes).
    pub fn publish_status(&mut self, online: bool) -> Result<(), MqttError> {
        let payload = if online { "online" } else { "offline" };
        self.publish(status_topic(&self.prefix), payload.into())
    }

    /// Clear every retained value this publisher left on the broker.
    pub fn clear_all(&mut self) -> Result<(), MqttError> {
        let all: Vec<String> = self.published.iter().cloned().collect();
        for topic in all {
            self.publish(topic, String::new())?;
        }
        Ok(())
    }

    /// Clear the retained values of metrics no longer measured on their
    /// channel (a sensor or a whole module gone).
    pub fn retain_sources(&mut self, measured: &[(Metric, u8)]) -> Result<(), MqttError> {
        let stem = format!("{}/", self.prefix);
        let stale: Vec<String> = self
            .published
            .iter()
            .filter(|topic| {
                let Some(rest) = topic.strip_prefix(&stem) else {
                    return false;
                };
                let mut parts = rest.split('/');
                let (Some(name), Some(channel), None) = (parts.next(), parts.next(), parts.next())
                else {
                    return false;
                };
                let Ok(channel) = channel.parse::<u8>() else {
                    return false;
                };
                let metric = if name == "altitude" {
                    Some(Metric::Pressure)
                } else {
                    Metric::ALL.into_iter().find(|m| m.as_str() == name)
                };
                metric.is_some_and(|m| !measured.contains(&(m, channel)))
            })
            .cloned()
            .collect();
        for topic in stale {
            self.publish(topic, String::new())?;
        }
        Ok(())
    }

    /// Relay state as a retained message: `true`, `false` or `null` when
    /// unknown; `None` (no relay) clears the topic.
    pub fn publish_relay(&mut self, relay: Option<&RelayInfo>) -> Result<(), MqttError> {
        let channel = relay.map(|r| r.channel);
        if let Some(old) = self.relay_channel
            && channel != Some(old)
        {
            self.publish(format!("{}/relay/{old}", self.prefix), String::new())?;
        }
        self.relay_channel = channel;
        match relay {
            Some(r) => {
                let payload = r.state.map_or("null".to_string(), |s| s.to_string());
                self.publish(format!("{}/relay/{}", self.prefix, r.channel), payload)
            }
            None => Ok(()),
        }
    }

    /// The GPS position as retained plain numbers under
    /// `<prefix>/gps/<channel>/`: latitude, longitude, altitude (m) and
    /// satellites. Without a fix the position keeps its last value; `None`
    /// (no module) clears them all.
    pub fn publish_gps(&mut self, gps: Option<&GpsInfo>) -> Result<(), MqttError> {
        let stem = format!("{}/gps/", self.prefix);
        let Some(g) = gps else {
            let stale: Vec<String> = self
                .published
                .iter()
                .filter(|t| t.starts_with(&stem))
                .cloned()
                .collect();
            for topic in stale {
                self.publish(topic, String::new())?;
            }
            return Ok(());
        };
        // The module moved to another channel: its old topics go.
        let mine = format!("{stem}{}/", g.channel);
        let moved: Vec<String> = self
            .published
            .iter()
            .filter(|t| t.starts_with(&stem) && !t.starts_with(&mine))
            .cloned()
            .collect();
        for topic in moved {
            self.publish(topic, String::new())?;
        }
        let values = [
            ("latitude", g.latitude.map(|v| format!("{v:.7}"))),
            ("longitude", g.longitude.map(|v| format!("{v:.7}"))),
            ("altitude", g.altitude.map(|v| format!("{v:.1}"))),
            ("satellites", Some(g.satellites.to_string())),
        ];
        for (name, payload) in values {
            if let Some(p) = payload {
                self.publish(format!("{stem}{}/{name}", g.channel), p)?;
            }
        }
        Ok(())
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

/// Topic/payload pairs for a reading of one channel (missing values are
/// skipped, since an empty retained payload would clear the topic).
pub fn topics(prefix: &str, r: &Reading) -> Vec<(String, String)> {
    let v = &r.values;
    [
        ("temperature", v.temperature.map(|t| format!("{t:.2}"))),
        ("humidity", v.humidity.map(|h| format!("{h:.1}"))),
        ("illuminance", v.illuminance.map(|l| format!("{l:.1}"))),
        ("pressure", v.pressure.map(|p| format!("{p:.2}"))),
        ("altitude", r.altitude.map(|a| format!("{a:.1}"))),
        ("co2", v.co2.map(|c| format!("{c:.0}"))),
        ("voc", v.voc.map(|c| format!("{c:.0}"))),
    ]
    .into_iter()
    .filter_map(|(name, payload)| payload.map(|p| (format!("{prefix}/{name}/{}", r.channel), p)))
    .collect()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;
    use crate::model::Values;

    #[test]
    fn topic_layout() {
        let r = Reading {
            ts: 0,
            device: "x".into(),
            channel: 1,
            values: Values {
                temperature: Some(23.456),
                humidity: Some(45.21),
                illuminance: None,
                pressure: Some(98_765.25),
                co2: None,
                voc: None,
            },
            altitude: Some(312.44),
        };
        assert_eq!(
            topics("tower", &r),
            vec![
                ("tower/temperature/1".to_string(), "23.46".to_string()),
                ("tower/humidity/1".to_string(), "45.2".to_string()),
                ("tower/pressure/1".to_string(), "98765.25".to_string()),
                ("tower/altitude/1".to_string(), "312.4".to_string()),
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

    /// A link that records what it is asked to do.
    #[derive(Clone, Default)]
    struct FakeLink(Arc<Mutex<Vec<String>>>);

    impl FakeLink {
        fn take(&self) -> Vec<String> {
            self.0
                .lock()
                .map(|mut v| std::mem::take(&mut *v))
                .unwrap_or_default()
        }
    }

    impl Link for FakeLink {
        fn publish(&mut self, topic: String, payload: String) -> Result<(), MqttError> {
            if let Ok(mut v) = self.0.lock() {
                v.push(format!("{topic}={payload}"));
            }
            Ok(())
        }
    }

    fn fake(prefix: &str) -> (Publisher, FakeLink) {
        let link = FakeLink::default();
        let publisher = Publisher::new(Box::new(link.clone()), prefix.into());
        (publisher, link)
    }

    fn reading(channel: u8) -> Reading {
        Reading {
            ts: 0,
            device: "x".into(),
            channel,
            values: Values {
                pressure: Some(100_000.0),
                ..Default::default()
            },
            altitude: Some(110.0),
        }
    }

    #[test]
    fn the_publisher_keeps_track_of_its_topics() -> Result<(), MqttError> {
        let (mut p, link) = fake("t");
        p.publish_status(true)?;
        p.publish_reading(&reading(1))?;
        assert_eq!(
            link.take(),
            vec![
                "t/status=online",
                "t/pressure/1=100000.00",
                "t/altitude/1=110.0"
            ]
        );
        // Channel 1 lost its barometer (pressure and altitude go), not its
        // thermometer.
        p.retain_sources(&[(Metric::Temperature, 1)])?;
        let mut cleared = link.take();
        cleared.sort();
        assert_eq!(cleared, vec!["t/altitude/1=", "t/pressure/1="]);
        p.retain_sources(&[(Metric::Temperature, 1)])?;
        assert!(link.take().is_empty(), "cleared once");
        p.publish_reading(&reading(0))?;
        link.take();
        p.clear_all()?;
        let mut cleared = link.take();
        cleared.sort();
        assert_eq!(cleared, vec!["t/altitude/0=", "t/pressure/0=", "t/status="]);
        Ok(())
    }

    #[test]
    fn a_relay_moving_to_another_channel_clears_the_old_topic() -> Result<(), MqttError> {
        let (mut p, link) = fake("t");
        let relay = |channel, state| RelayInfo {
            channel,
            address: 0x3B,
            state,
            pulse_until: None,
        };
        p.publish_relay(Some(&relay(0, None)))?;
        p.publish_relay(Some(&relay(1, Some(true))))?;
        p.publish_relay(None)?;
        assert_eq!(
            link.take(),
            vec![
                "t/relay/0=null",
                "t/relay/0=",
                "t/relay/1=true",
                "t/relay/1="
            ]
        );
        Ok(())
    }

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

    fn read_packet(stream: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
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

    /// What the test broker saw.
    #[derive(Default)]
    struct Seen {
        /// The CONNECT packet's body.
        connect: Vec<u8>,
        /// Topic, payload and retain flag of every PUBLISH.
        published: Vec<(String, String, bool)>,
        /// The client said goodbye (DISCONNECT).
        disconnected: bool,
    }

    /// A one-client MQTT 3.1.1 broker for the tests: it answers CONNECT,
    /// SUBSCRIBE and PINGREQ, records PUBLISHes, and sends `send` messages
    /// (topic, payload, retain) to the client once it has subscribed.
    struct TestBroker {
        port: u16,
        seen: Arc<Mutex<Seen>>,
        send: crossbeam_channel::Sender<(String, String, bool)>,
    }

    fn test_broker() -> std::io::Result<TestBroker> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (send, outbox) = crossbeam_channel::unbounded::<(String, String, bool)>();
        let record = seen.clone();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let Ok(writer) = stream.try_clone() else {
                return;
            };
            let writer = Arc::new(Mutex::new(writer));
            let write = |w: &Arc<Mutex<TcpStream>>, packet: Vec<u8>| {
                if let Ok(mut w) = w.lock() {
                    let _ = w.write_all(&packet);
                }
            };
            let mut subscribed = false;
            while let Ok((kind, body)) = read_packet(&mut stream) {
                match kind & 0xF0 {
                    0x10 => {
                        if let Ok(mut s) = record.lock() {
                            s.connect = body;
                        }
                        write(&writer, vec![0x20, 2, 0, 0]);
                    }
                    0x30 => {
                        let tlen = usize::from(u16::from_be_bytes([body[0], body[1]]));
                        let topic = String::from_utf8_lossy(&body[2..2 + tlen]).into_owned();
                        let qos = (kind >> 1) & 3;
                        let start = 2 + tlen + if qos > 0 { 2 } else { 0 };
                        let payload = String::from_utf8_lossy(&body[start..]).into_owned();
                        if let Ok(mut s) = record.lock() {
                            s.published.push((topic, payload, kind & 1 == 1));
                        }
                        if qos == 1 {
                            write(&writer, vec![0x40, 2, body[2 + tlen], body[3 + tlen]]);
                        }
                    }
                    0x80 => {
                        write(&writer, vec![0x90, 3, body[0], body[1], 0]);
                        if !subscribed {
                            subscribed = true;
                            let w = writer.clone();
                            let outbox = outbox.clone();
                            std::thread::spawn(move || {
                                for (topic, payload, retain) in outbox {
                                    let mut packet = mqtt_str(&topic);
                                    packet.extend_from_slice(payload.as_bytes());
                                    write(&w, mqtt_packet(0x30 | u8::from(retain), &packet));
                                }
                            });
                        }
                    }
                    0xA0 => write(&writer, vec![0xB0, 2, body[0], body[1]]),
                    0xC0 => write(&writer, vec![0xD0, 0]),
                    0xE0 => {
                        if let Ok(mut s) = record.lock() {
                            s.disconnected = true;
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });
        Ok(TestBroker { port, seen, send })
    }

    fn client(
        port: u16,
        prefix: &str,
    ) -> Result<
        (
            Publisher,
            crossbeam_channel::Receiver<MqttCommand>,
            crossbeam_channel::Receiver<LinkEvent>,
        ),
        MqttError,
    > {
        let (commands, received) = crossbeam_channel::unbounded();
        let (events_tx, events) = crossbeam_channel::unbounded();
        let publisher = connect(
            &ClientConfig {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                credentials: Some(("ha".into(), "secret".into())),
                client_id: "tower-bridge-test".into(),
            },
            prefix,
            commands,
            events_tx,
        )?;
        Ok((publisher, received, events))
    }

    #[test]
    fn the_client_publishes_retained_values_with_a_last_will()
    -> Result<(), Box<dyn std::error::Error>> {
        let broker = test_broker()?;
        let (mut publisher, _received, events) = client(broker.port, "ext")?;
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5))?,
            LinkEvent::Connected
        );
        publisher.publish_status(true)?;
        publisher.publish_reading(&reading(0))?;
        std::thread::sleep(Duration::from_millis(300));
        let seen = broker.seen.lock().map_err(|_| "lock")?;
        assert_eq!(
            seen.published,
            vec![
                ("ext/status".into(), "online".into(), true),
                ("ext/pressure/0".into(), "100000.00".into(), true),
                ("ext/altitude/0".into(), "110.0".into(), true),
            ]
        );
        // CONNECT: protocol, level 4, flags, keepalive, client id, will, login.
        let c = &seen.connect;
        assert_eq!(&c[..7], b"\x00\x04MQTT\x04");
        let flags = c[7];
        assert_eq!(flags & 0xC0, 0xC0, "username and password");
        assert_eq!(flags & 0x24, 0x24, "a retained last will");
        let text = String::from_utf8_lossy(c);
        assert!(text.contains("tower-bridge-test"));
        assert!(text.contains("ext/status") && text.contains("offline"));
        Ok(())
    }

    #[test]
    fn commands_from_the_broker_arrive_but_retained_ones_do_not()
    -> Result<(), Box<dyn std::error::Error>> {
        let broker = test_broker()?;
        let (_publisher, received, events) = client(broker.port, "ext")?;
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5))?,
            LinkEvent::Connected
        );
        // As a broker replays stored messages to a new subscription.
        broker
            .send
            .send(("ext/relay/0/set".into(), "true".into(), true))?;
        broker
            .send
            .send(("ext/relay/0/set".into(), "false".into(), false))?;
        assert_eq!(
            received.recv_timeout(Duration::from_secs(3))?,
            MqttCommand::SetRelay {
                channel: 0,
                state: false
            }
        );
        assert!(received.recv_timeout(Duration::from_millis(300)).is_err());
        Ok(())
    }

    #[test]
    fn an_unreachable_broker_is_reported_and_retried() -> Result<(), Box<dyn std::error::Error>> {
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let (client, _received, events) = client(port, "ext")?;
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(5))?,
            LinkEvent::Disconnected(_)
        ));
        // Retried after a pause, not in a tight loop.
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(5))?,
            LinkEvent::Disconnected(_)
        ));
        assert!(events.try_recv().is_err());
        drop(client);
        Ok(())
    }

    #[test]
    fn parses_relay_commands() {
        let cmd = |topic: &str, payload: &str| parse_command("tower", topic, payload.as_bytes());
        assert_eq!(cmd("tower/temperature/0", "1"), None);
        assert_eq!(cmd("other/relay/0/set", "true"), None);
        assert_eq!(cmd("tower/relay/0", "true"), None, "the state topic");
        assert_eq!(
            cmd("tower/relay/0/set", " False\n"),
            Some(Ok(MqttCommand::SetRelay {
                channel: 0,
                state: false
            }))
        );
        assert_eq!(
            cmd("tower/relay/1/set", "\"true\""),
            Some(Ok(MqttCommand::SetRelay {
                channel: 1,
                state: true
            }))
        );
        assert!(matches!(cmd("tower/relay/0/set", "on"), Some(Err(_))));
        assert!(matches!(cmd("tower/relay/x/set", "true"), Some(Err(_))));
        assert_eq!(
            cmd("tower/relay/0/pulse", ""),
            Some(Ok(MqttCommand::PulseRelay {
                channel: 0,
                state: None,
                duration: None
            }))
        );
        assert_eq!(
            cmd("tower/relay/0/pulse", "1500"),
            Some(Ok(MqttCommand::PulseRelay {
                channel: 0,
                state: None,
                duration: Some(Duration::from_millis(1500))
            }))
        );
        assert_eq!(
            cmd(
                "tower/relay/0/pulse",
                r#"{"state": true, "duration_ms": 3000}"#
            ),
            Some(Ok(MqttCommand::PulseRelay {
                channel: 0,
                state: Some(true),
                duration: Some(Duration::from_secs(3))
            }))
        );
        assert!(matches!(cmd("tower/relay/0/pulse", "-5"), Some(Err(_))));
        for junk in [
            "null",
            "[]",
            "\"foo\"",
            r#"{"duration": 3000}"#,
            r#"{"durationMs": 3000}"#,
        ] {
            assert!(
                matches!(cmd("tower/relay/0/pulse", junk), Some(Err(_))),
                "{junk} must not pulse"
            );
        }
        assert!(matches!(
            cmd("tower/relay/0/pulse", r#"{"state": "on"}"#),
            Some(Err(_))
        ));
    }

    #[test]
    fn stopping_sends_every_clear_and_then_disconnects() -> Result<(), Box<dyn std::error::Error>> {
        let broker = test_broker()?;
        let (mut publisher, _received, events) = client(broker.port, "old")?;
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5))?,
            LinkEvent::Connected
        );
        publisher.publish_status(true)?;
        for channel in 0..2 {
            publisher.publish_reading(&reading(channel))?;
        }
        std::thread::sleep(Duration::from_millis(200));
        // A prefix change: clear everything, say offline, go.
        publisher.clear_all()?;
        publisher.publish_status(false)?;
        drop(publisher);
        std::thread::sleep(Duration::from_millis(500));
        let seen = broker.seen.lock().map_err(|_| "lock")?;
        for topic in [
            "old/pressure/0",
            "old/altitude/0",
            "old/pressure/1",
            "old/altitude/1",
        ] {
            assert!(
                seen.published
                    .iter()
                    .any(|(t, p, _)| t == topic && p.is_empty()),
                "{topic} cleared: {:?}",
                seen.published
            );
        }
        assert_eq!(seen.published.last().map(|p| p.1.as_str()), Some("offline"));
        assert!(seen.disconnected, "a clean disconnect");
        Ok(())
    }
}
