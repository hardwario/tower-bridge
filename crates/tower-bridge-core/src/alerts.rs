//! Threshold alerts with hysteresis and notification cooldown.
//!
//! All values are SI units (°C, %RH, lux, Pa). The engine is pure: it turns
//! readings into events; the caller decides how to notify. A rule applies to
//! its metric on every I2C channel, and each channel is tracked on its own.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::{Metric, Values};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct AlertRule {
    pub enabled: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Distance back inside the limit needed to clear an alert.
    pub hysteresis: f64,
    /// Minimum seconds between two notifications for the same limit.
    pub cooldown_s: u64,
    pub notify_recovery: bool,
}

impl Default for AlertRule {
    fn default() -> Self {
        Self {
            enabled: false,
            min: None,
            max: None,
            hysteresis: 0.0,
            cooldown_s: 900,
            notify_recovery: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct AlertRules {
    pub temperature: AlertRule,
    pub humidity: AlertRule,
    pub illuminance: AlertRule,
    pub pressure: AlertRule,
    pub co2: AlertRule,
    pub voc: AlertRule,
}

impl Default for AlertRules {
    fn default() -> Self {
        Self {
            temperature: AlertRule {
                min: Some(18.0),
                max: Some(28.0),
                hysteresis: 0.5,
                ..AlertRule::default()
            },
            humidity: AlertRule {
                min: Some(30.0),
                max: Some(60.0),
                hysteresis: 2.0,
                ..AlertRule::default()
            },
            illuminance: AlertRule {
                hysteresis: 10.0,
                ..AlertRule::default()
            },
            pressure: AlertRule {
                hysteresis: 100.0,
                ..AlertRule::default()
            },
            // Above about 1000 ppm a room wants airing.
            co2: AlertRule {
                max: Some(1200.0),
                hysteresis: 50.0,
                ..AlertRule::default()
            },
            voc: AlertRule {
                max: Some(500.0),
                hysteresis: 25.0,
                ..AlertRule::default()
            },
        }
    }
}

impl AlertRules {
    pub fn get(&self, metric: Metric) -> &AlertRule {
        match metric {
            Metric::Temperature => &self.temperature,
            Metric::Humidity => &self.humidity,
            Metric::Illuminance => &self.illuminance,
            Metric::Pressure => &self.pressure,
            Metric::Co2 => &self.co2,
            Metric::Voc => &self.voc,
        }
    }

    pub fn get_mut(&mut self, metric: Metric) -> &mut AlertRule {
        match metric {
            Metric::Temperature => &mut self.temperature,
            Metric::Humidity => &mut self.humidity,
            Metric::Illuminance => &mut self.illuminance,
            Metric::Pressure => &mut self.pressure,
            Metric::Co2 => &mut self.co2,
            Metric::Voc => &mut self.voc,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bound {
    Low,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    Triggered,
    Recovered,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AlertEvent {
    pub ts: i64,
    pub metric: Metric,
    /// I2C channel of the sensor.
    pub channel: u8,
    pub bound: Bound,
    pub kind: AlertKind,
    pub value: f64,
    pub threshold: f64,
    /// Whether a notification should be shown (false when suppressed by the
    /// cooldown, or for a recovery that is not notified).
    pub notify: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ActiveAlert {
    pub metric: Metric,
    pub channel: u8,
    pub bound: Bound,
    pub since: i64,
    pub threshold: f64,
    pub value: f64,
}

#[derive(Clone, Debug, Default)]
struct BoundState {
    active: bool,
    since: i64,
    value: f64,
    last_notified: Option<i64>,
    notified_episode: bool,
}

#[derive(Debug, Default)]
pub struct AlertEngine {
    rules: AlertRules,
    state: HashMap<(Metric, u8, Bound), BoundState>,
}

impl AlertEngine {
    pub fn new(rules: AlertRules) -> Self {
        Self {
            rules,
            state: HashMap::new(),
        }
    }

    pub fn rules(&self) -> &AlertRules {
        &self.rules
    }

    /// Replace the rules. Active alerts stay, and the next values are
    /// checked against the new limits: an edit neither repeats a
    /// notification nor forgets when the last one was sent.
    pub fn set_rules(&mut self, rules: AlertRules) {
        // A rule switched off or a limit removed ends its alerts at once:
        // with no readings coming (the bridge unplugged) they would stay.
        self.state.retain(|(metric, _, bound), _| {
            let rule = rules.get(*metric);
            rule.enabled
                && match bound {
                    Bound::Low => rule.min.is_some(),
                    Bound::High => rule.max.is_some(),
                }
        });
        self.rules = rules;
    }

    /// Forget alerts of metrics no longer measured on their channel (the
    /// sensor is gone); they could never end otherwise. Returns whether any
    /// was active.
    pub fn retain_sources(&mut self, measured: &[(Metric, u8)]) -> bool {
        let before = self.active().len();
        self.state
            .retain(|(m, c, _), _| measured.contains(&(*m, *c)));
        self.active().len() != before
    }

    /// Check the values measured on `channel`.
    pub fn evaluate(&mut self, ts: i64, channel: u8, values: &Values) -> Vec<AlertEvent> {
        let mut events = Vec::new();
        for metric in Metric::ALL {
            let rule = self.rules.get(metric).clone();
            for bound in [Bound::Low, Bound::High] {
                let limit = match bound {
                    Bound::Low => rule.min,
                    Bound::High => rule.max,
                };
                let (true, Some(limit)) = (rule.enabled, limit) else {
                    self.state
                        .retain(|(m, _, b), _| (*m, *b) != (metric, bound));
                    continue;
                };
                let Some(value) = values.get(metric) else {
                    continue;
                };
                let st = self.state.entry((metric, channel, bound)).or_default();
                let hyst = rule.hysteresis.max(0.0);
                let (beyond, back_inside) = match bound {
                    Bound::High => (value > limit, value <= limit - hyst),
                    Bound::Low => (value < limit, value >= limit + hyst),
                };
                st.value = value;
                if !st.active && beyond {
                    let cooldown_ms = i64::try_from(rule.cooldown_s)
                        .unwrap_or(i64::MAX)
                        .saturating_mul(1000);
                    let notify = st
                        .last_notified
                        .is_none_or(|t| ts.saturating_sub(t) >= cooldown_ms);
                    st.active = true;
                    st.since = ts;
                    st.notified_episode = notify;
                    if notify {
                        st.last_notified = Some(ts);
                    }
                    events.push(AlertEvent {
                        ts,
                        metric,
                        channel,
                        bound,
                        kind: AlertKind::Triggered,
                        value,
                        threshold: limit,
                        notify,
                    });
                } else if st.active && back_inside {
                    st.active = false;
                    events.push(AlertEvent {
                        ts,
                        metric,
                        channel,
                        bound,
                        kind: AlertKind::Recovered,
                        value,
                        threshold: limit,
                        notify: rule.notify_recovery && st.notified_episode,
                    });
                    st.notified_episode = false;
                }
            }
        }
        events
    }

    pub fn active(&self) -> Vec<ActiveAlert> {
        let mut out: Vec<ActiveAlert> = self
            .state
            .iter()
            .filter(|(_, s)| s.active)
            .filter_map(|((metric, channel, bound), s)| {
                let rule = self.rules.get(*metric);
                let threshold = match bound {
                    Bound::Low => rule.min,
                    Bound::High => rule.max,
                }?;
                Some(ActiveAlert {
                    metric: *metric,
                    channel: *channel,
                    bound: *bound,
                    since: s.since,
                    threshold,
                    value: s.value,
                })
            })
            .collect();
        out.sort_by_key(|a| (a.metric as u8, a.channel, a.bound as u8));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(v: Option<f64>) -> Values {
        Values {
            temperature: v,
            ..Values::default()
        }
    }

    fn engine() -> AlertEngine {
        AlertEngine::new(AlertRules {
            temperature: AlertRule {
                enabled: true,
                min: Some(18.0),
                max: Some(25.0),
                hysteresis: 1.0,
                cooldown_s: 600,
                notify_recovery: true,
            },
            ..AlertRules::default()
        })
    }

    fn kinds(ev: &[AlertEvent]) -> Vec<(Bound, AlertKind, bool)> {
        ev.iter().map(|e| (e.bound, e.kind, e.notify)).collect()
    }

    #[test]
    fn trigger_and_recover_with_hysteresis() {
        let mut e = engine();
        assert!(e.evaluate(0, 0, &temp(Some(24.0))).is_empty());
        let ev = e.evaluate(1_000, 0, &temp(Some(25.5)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Triggered, true)]);
        assert_eq!(e.active().len(), 1);
        // Inside the limit but within hysteresis: still active.
        assert!(e.evaluate(2_000, 0, &temp(Some(24.5))).is_empty());
        let ev = e.evaluate(3_000, 0, &temp(Some(23.9)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Recovered, true)]);
        assert!(e.active().is_empty());
    }

    #[test]
    fn flapping_is_rate_limited_by_cooldown() {
        let mut e = engine();
        let ev = e.evaluate(0, 0, &temp(Some(26.0)));
        assert!(ev[0].notify);
        let ev = e.evaluate(10_000, 0, &temp(Some(23.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Recovered, true)]);
        // Re-trigger within the 600 s cooldown: event but no notification,
        // and its recovery is not notified either.
        let ev = e.evaluate(20_000, 0, &temp(Some(26.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Triggered, false)]);
        let ev = e.evaluate(30_000, 0, &temp(Some(23.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Recovered, false)]);
        // After the cooldown it notifies again.
        let ev = e.evaluate(700_000, 0, &temp(Some(26.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Triggered, true)]);
    }

    #[test]
    fn low_bound() {
        let mut e = engine();
        let ev = e.evaluate(0, 0, &temp(Some(17.0)));
        assert_eq!(kinds(&ev), vec![(Bound::Low, AlertKind::Triggered, true)]);
        assert!(e.evaluate(1, 0, &temp(Some(18.5))).is_empty());
        let ev = e.evaluate(2, 0, &temp(Some(19.0)));
        assert_eq!(kinds(&ev), vec![(Bound::Low, AlertKind::Recovered, true)]);
    }

    #[test]
    fn missing_values_keep_state() {
        let mut e = engine();
        e.evaluate(0, 0, &temp(Some(30.0)));
        assert!(e.evaluate(1, 0, &temp(None)).is_empty());
        assert_eq!(e.active().len(), 1);
    }

    #[test]
    fn a_rule_edit_keeps_the_alert_and_checks_the_new_limits() {
        let mut e = engine();
        e.evaluate(0, 0, &temp(Some(30.0)));
        // Only the repeat interval changes: no second notification.
        let mut rules = e.rules().clone();
        rules.temperature.cooldown_s += 60;
        e.set_rules(rules.clone());
        assert!(e.evaluate(1, 0, &temp(Some(30.0))).is_empty());
        assert_eq!(e.active().len(), 1);
        // A limit above the value ends it, as the value falling would.
        rules.temperature.max = Some(35.0);
        e.set_rules(rules);
        let ev = e.evaluate(2, 0, &temp(Some(30.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Recovered, true)]);
        assert!(e.active().is_empty());
    }

    #[test]
    fn disabling_clears_state() {
        let mut e = engine();
        e.evaluate(0, 0, &temp(Some(30.0)));
        let mut rules = e.rules().clone();
        rules.temperature.enabled = false;
        e.set_rules(rules);
        assert!(e.active().is_empty(), "at once, not at the next reading");
        assert!(e.evaluate(1, 0, &temp(Some(30.0))).is_empty());
        assert!(e.active().is_empty());
    }

    #[test]
    fn channels_are_tracked_separately() {
        let mut e = engine();
        let ev = e.evaluate(0, 1, &temp(Some(30.0)));
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].channel, 1);
        assert!(e.evaluate(1, 0, &temp(Some(20.0))).is_empty());
        let active = e.active();
        assert_eq!(active.len(), 1);
        assert_eq!((active[0].channel, active[0].bound), (1, Bound::High));
        let ev = e.evaluate(2, 0, &temp(Some(30.0)));
        assert_eq!(ev.len(), 1, "channel 0 alerts on its own");
        assert_eq!(e.active().len(), 2);
    }

    #[test]
    fn alerts_of_vanished_sensors_are_cleared() {
        let mut e = engine();
        e.evaluate(0, 1, &temp(Some(30.0)));
        e.evaluate(0, 0, &temp(Some(30.0)));
        assert_eq!(e.active().len(), 2);
        // Channel 1 lost its thermometer but kept other sensors.
        let measured = [
            (Metric::Temperature, 0),
            (Metric::Humidity, 0),
            (Metric::Humidity, 1),
        ];
        assert!(e.retain_sources(&measured));
        let active = e.active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].channel, 0);
        assert!(!e.retain_sources(&measured), "nothing more to drop");
    }

    #[test]
    fn huge_cooldown_never_renotifies() {
        let mut e = engine();
        let mut rules = e.rules().clone();
        rules.temperature.cooldown_s = u64::MAX;
        e.set_rules(rules);
        assert!(e.evaluate(1_000, 0, &temp(Some(30.0)))[0].notify);
        e.evaluate(2_000, 0, &temp(Some(20.0)));
        let ev = e.evaluate(3_000, 0, &temp(Some(30.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Triggered, false)]);
    }

    #[test]
    fn no_recovery_notification_when_disabled() {
        let mut e = engine();
        let mut rules = e.rules().clone();
        rules.temperature.notify_recovery = false;
        e.set_rules(rules);
        e.evaluate(0, 0, &temp(Some(30.0)));
        let ev = e.evaluate(1, 0, &temp(Some(20.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Recovered, false)]);
    }
}
