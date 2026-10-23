//! Alert evaluation.
//!
//! An alert fires on the rising edge of its condition. Repeating alerts can
//! fire again while the condition remains true, but never more often than the
//! configured cooldown. One-shot alerts disable themselves after firing.

use std::collections::BTreeMap;

use chrono::{DateTime, Local};

use crate::model::{Alert, AlertEvent, MarketId, MarketMetrics, Quote, SessionAlert, SessionId};

#[derive(Debug, Default)]
pub struct Evaluation {
    pub events: Vec<AlertEvent>,
    pub changed: bool,
}

pub fn reset_conditions(alerts: &mut [Alert]) {
    for alert in alerts {
        alert.last_condition = false;
    }
}

pub fn evaluate(
    alerts: &mut [Alert],
    quotes: &BTreeMap<MarketId, Quote>,
    metrics: &MarketMetrics,
    now: DateTime<Local>,
) -> Evaluation {
    let mut result = Evaluation::default();
    for alert in alerts.iter_mut() {
        if !alert.enabled {
            continue;
        }
        let Some(value) = alert.value(quotes, metrics) else {
            continue;
        };
        alert.last_value = Some(value);
        let met = alert.direction.matches(value, alert.threshold);
        let rising_edge = met && !alert.last_condition;
        let repeat_due = met && alert.repeat && alert.last_condition;
        let cooldown_ok = alert.cooldown_ready(now);

        if met && (rising_edge || repeat_due) && cooldown_ok {
            let message = alert.trigger_message(value);
            let event = AlertEvent::new(alert, message, value);
            alert.last_triggered = Some(now);
            alert.last_triggered_value = Some(value);
            alert.trigger_count = alert.trigger_count.saturating_add(1);
            alert.last_condition = true;
            if !alert.repeat {
                alert.enabled = false;
            }
            result.events.push(event);
            result.changed = true;
        } else {
            if alert.last_condition != met {
                result.changed = true;
            }
            alert.last_condition = met;
        }
    }
    result
}

/// Evaluate open/close alerts for one session after a state transition.
/// Returns the events together with each alert's notification/flash settings.
pub fn evaluate_session(
    alerts: &mut [SessionAlert],
    session: SessionId,
    opened: bool,
) -> Vec<(AlertEvent, bool, bool)> {
    let mut events = Vec::new();
    for alert in alerts.iter_mut() {
        if !alert.enabled || alert.session != session || !alert.watches(opened) {
            continue;
        }
        let event = AlertEvent::new_session(alert, opened);
        alert.last_triggered = Some(chrono::Local::now());
        alert.trigger_count = alert.trigger_count.saturating_add(1);
        events.push((event, alert.notify, alert.flash));
    }
    events
}

pub fn active_count(alerts: &[Alert]) -> usize {
    alerts.iter().filter(|a| a.enabled).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AlertMetric, AlertScope, Direction, FxRates};

    fn quote(market: MarketId, price: f64, prev: f64) -> Quote {
        let mut q = Quote::new(market, price);
        q.prev_close = prev;
        q
    }

    fn quotes(price: f64) -> BTreeMap<MarketId, Quote> {
        let mut map = BTreeMap::new();
        map.insert(MarketId::London, quote(MarketId::London, price, 4_000.0));
        map
    }

    #[test]
    fn price_alert_fires_on_crossing_and_disables() {
        let mut alerts = vec![Alert::new(
            "突破",
            AlertScope::Market {
                market: MarketId::London,
            },
            AlertMetric::Price,
            Direction::Above,
            4_050.0,
        )];
        let metrics = MarketMetrics::default();
        let now = Local::now();
        let first = evaluate(&mut alerts, &quotes(4_040.0), &metrics, now);
        assert!(first.events.is_empty());
        let second = evaluate(&mut alerts, &quotes(4_060.0), &metrics, now);
        assert_eq!(second.events.len(), 1);
        assert!(!alerts[0].enabled);
        let third = evaluate(&mut alerts, &quotes(4_070.0), &metrics, now);
        assert!(third.events.is_empty());
    }

    #[test]
    fn repeat_alert_respects_cooldown() {
        let mut alert = Alert::new(
            "持续",
            AlertScope::Market {
                market: MarketId::London,
            },
            AlertMetric::Price,
            Direction::Above,
            4_050.0,
        );
        alert.repeat = true;
        alert.cooldown_secs = 60;
        let mut alerts = vec![alert];
        let metrics = MarketMetrics::default();
        let now = Local::now();
        assert_eq!(
            evaluate(&mut alerts, &quotes(4_060.0), &metrics, now)
                .events
                .len(),
            1
        );
        assert_eq!(
            evaluate(&mut alerts, &quotes(4_070.0), &metrics, now)
                .events
                .len(),
            0
        );
        assert_eq!(
            evaluate(
                &mut alerts,
                &quotes(4_080.0),
                &metrics,
                now + chrono::Duration::seconds(61)
            )
            .events
            .len(),
            1
        );
    }

    #[test]
    fn repeat_alert_cooldown_applies_to_new_crossings() {
        let mut alert = Alert::new(
            "震荡",
            AlertScope::Market {
                market: MarketId::London,
            },
            AlertMetric::Price,
            Direction::Above,
            4_050.0,
        );
        alert.repeat = true;
        alert.cooldown_secs = 300;
        let mut alerts = vec![alert];
        let metrics = MarketMetrics::default();
        let now = Local::now();
        assert_eq!(
            evaluate(&mut alerts, &quotes(4_060.0), &metrics, now)
                .events
                .len(),
            1
        );
        assert_eq!(
            evaluate(
                &mut alerts,
                &quotes(4_040.0),
                &metrics,
                now + chrono::Duration::seconds(10)
            )
            .events
            .len(),
            0
        );
        assert_eq!(
            evaluate(
                &mut alerts,
                &quotes(4_060.0),
                &metrics,
                now + chrono::Duration::seconds(20)
            )
            .events
            .len(),
            0
        );
        assert_eq!(
            evaluate(
                &mut alerts,
                &quotes(4_060.0),
                &metrics,
                now + chrono::Duration::seconds(301)
            )
            .events
            .len(),
            1
        );
    }

    #[test]
    fn session_alerts_only_fire_for_selected_events() {
        let mut open_alert = SessionAlert::new(SessionId::NewYork, true, false);
        open_alert.id = "open".into();
        let mut close_alert = SessionAlert::new(SessionId::NewYork, false, true);
        close_alert.id = "close".into();
        let mut other = SessionAlert::new(SessionId::London, true, true);
        other.id = "other".into();
        let mut alerts = vec![open_alert, close_alert, other];
        let opened = evaluate_session(&mut alerts, SessionId::NewYork, true);
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].0.alert_id, "open");
        let closed = evaluate_session(&mut alerts, SessionId::NewYork, false);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].0.alert_id, "close");
        assert_eq!(alerts[0].trigger_count, 1);
    }

    #[test]
    fn premium_alert_uses_derived_metrics() {
        let mut alerts = vec![Alert::new(
            "溢价",
            AlertScope::SgePremium,
            AlertMetric::Price,
            Direction::Above,
            1.2,
        )];
        let fx = FxRates {
            usd_cny: 6.7,
            usd_jpy: 150.0,
            usd_inr: 83.0,
            updated_at: Local::now(),
            source: "test".into(),
        };
        let mut map = BTreeMap::new();
        map.insert(MarketId::London, quote(MarketId::London, 4_000.0, 3_900.0));
        map.insert(MarketId::Shanghai, quote(MarketId::Shanghai, 900.0, 880.0));
        let metrics = MarketMetrics::derive(&map, Some(&fx));
        let result = evaluate(&mut alerts, &map, &metrics, Local::now());
        assert_eq!(result.events.len(), 1);
    }
}
