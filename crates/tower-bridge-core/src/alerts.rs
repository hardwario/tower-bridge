//! Threshold alerts with hysteresis and notification cooldown.
//!
//! All values are SI units (°C, %RH, lux, Pa). The engine is pure: it turns
//! readings into events; the caller decides how to notify.

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
    state: HashMap<(Metric, Bound), BoundState>,
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

    /// Replace the rules; state is kept only for metrics whose rule is unchanged.
    pub fn set_rules(&mut self, rules: AlertRules) {
        for metric in Metric::ALL {
            if self.rules.get(metric) != rules.get(metric) {
                self.state.retain(|(m, _), _| *m != metric);
            }
        }
        self.rules = rules;
    }

    pub fn evaluate(&mut self, ts: i64, values: &Values) -> Vec<AlertEvent> {
        let mut events = Vec::new();
        for metric in Metric::ALL {
            let rule = self.rules.get(metric).clone();
            for bound in [Bound::Low, Bound::High] {
                let limit = match bound {
                    Bound::Low => rule.min,
                    Bound::High => rule.max,
                };
                let (true, Some(limit)) = (rule.enabled, limit) else {
                    self.state.remove(&(metric, bound));
                    continue;
                };
                let Some(value) = values.get(metric) else {
                    continue;
                };
                let st = self.state.entry((metric, bound)).or_default();
                let hyst = rule.hysteresis.max(0.0);
                let (beyond, back_inside) = match bound {
                    Bound::High => (value > limit, value <= limit - hyst),
                    Bound::Low => (value < limit, value >= limit + hyst),
                };
                st.value = value;
                if !st.active && beyond {
                    let cooldown_ms = (rule.cooldown_s as i64).saturating_mul(1000);
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
            .filter_map(|((metric, bound), s)| {
                let rule = self.rules.get(*metric);
                let threshold = match bound {
                    Bound::Low => rule.min,
                    Bound::High => rule.max,
                }?;
                Some(ActiveAlert {
                    metric: *metric,
                    bound: *bound,
                    since: s.since,
                    threshold,
                    value: s.value,
                })
            })
            .collect();
        out.sort_by_key(|a| (a.metric as u8, a.bound as u8));
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
        assert!(e.evaluate(0, &temp(Some(24.0))).is_empty());
        let ev = e.evaluate(1_000, &temp(Some(25.5)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Triggered, true)]);
        assert_eq!(e.active().len(), 1);
        // Inside the limit but within hysteresis: still active.
        assert!(e.evaluate(2_000, &temp(Some(24.5))).is_empty());
        let ev = e.evaluate(3_000, &temp(Some(23.9)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Recovered, true)]);
        assert!(e.active().is_empty());
    }

    #[test]
    fn flapping_is_rate_limited_by_cooldown() {
        let mut e = engine();
        let ev = e.evaluate(0, &temp(Some(26.0)));
        assert!(ev[0].notify);
        let ev = e.evaluate(10_000, &temp(Some(23.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Recovered, true)]);
        // Re-trigger within the 600 s cooldown: event but no notification,
        // and its recovery is not notified either.
        let ev = e.evaluate(20_000, &temp(Some(26.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Triggered, false)]);
        let ev = e.evaluate(30_000, &temp(Some(23.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Recovered, false)]);
        // After the cooldown it notifies again.
        let ev = e.evaluate(700_000, &temp(Some(26.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Triggered, true)]);
    }

    #[test]
    fn low_bound() {
        let mut e = engine();
        let ev = e.evaluate(0, &temp(Some(17.0)));
        assert_eq!(kinds(&ev), vec![(Bound::Low, AlertKind::Triggered, true)]);
        assert!(e.evaluate(1, &temp(Some(18.5))).is_empty());
        let ev = e.evaluate(2, &temp(Some(19.0)));
        assert_eq!(kinds(&ev), vec![(Bound::Low, AlertKind::Recovered, true)]);
    }

    #[test]
    fn missing_values_keep_state() {
        let mut e = engine();
        e.evaluate(0, &temp(Some(30.0)));
        assert!(e.evaluate(1, &temp(None)).is_empty());
        assert_eq!(e.active().len(), 1);
    }

    #[test]
    fn rule_edit_resets_only_that_metric() {
        let mut e = engine();
        e.evaluate(0, &temp(Some(30.0)));
        let mut rules = e.rules().clone();
        rules.humidity.enabled = true;
        e.set_rules(rules.clone());
        assert_eq!(
            e.active().len(),
            1,
            "unchanged temperature rule keeps state"
        );
        rules.temperature.max = Some(35.0);
        e.set_rules(rules);
        assert!(e.active().is_empty());
        assert!(e.evaluate(1, &temp(Some(30.0))).is_empty());
    }

    #[test]
    fn disabling_clears_state() {
        let mut e = engine();
        e.evaluate(0, &temp(Some(30.0)));
        let mut rules = e.rules().clone();
        rules.temperature.enabled = false;
        e.set_rules(rules);
        assert!(e.evaluate(1, &temp(Some(30.0))).is_empty());
        assert!(e.active().is_empty());
    }

    #[test]
    fn no_recovery_notification_when_disabled() {
        let mut e = engine();
        let mut rules = e.rules().clone();
        rules.temperature.notify_recovery = false;
        e.set_rules(rules);
        e.evaluate(0, &temp(Some(30.0)));
        let ev = e.evaluate(1, &temp(Some(20.0)));
        assert_eq!(kinds(&ev), vec![(Bound::High, AlertKind::Recovered, false)]);
    }
}
