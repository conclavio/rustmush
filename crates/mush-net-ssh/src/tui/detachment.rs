//! DT.1b: bounded own-roster display and exact server-generated command prefills.
use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Slot {
    pub slot: u16,
    pub callsign: String,
    pub pilot: Option<String>,
    pub controller: String,
    pub order_prefill: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Order {
    pub sequence: String,
    pub from: String,
    pub objective: String,
    pub response: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Detachment {
    pub id: String,
    pub revision: String,
    pub term: String,
    pub leader_slot: Option<u16>,
    pub slots: Vec<Slot>,
    pub all_order_prefill: String,
    pub my_order: Option<Order>,
    pub wilco_command: Option<String>,
    pub negative_command: Option<String>,
}

fn counter(value: &str) -> bool {
    value.len() <= 20 && value.parse::<u64>().is_ok_and(|n| n.to_string() == value)
}

fn label(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}

pub(super) fn decode(value: &Value) -> Option<Detachment> {
    for key in [
        "leader_slot",
        "my_order",
        "wilco_command",
        "negative_command",
    ] {
        value.get(key)?;
    }
    // Bound allocation before deserializing an externally supplied collection.
    let slots = value.get("slots")?.as_array()?;
    if slots.is_empty() || slots.len() > 6 {
        return None;
    }
    for slot in slots {
        slot.get("pilot")?;
    }
    if let Some(order) = value.get("my_order").filter(|order| !order.is_null()) {
        order.get("response")?;
    }
    let d: Detachment = serde_json::from_value(value.clone()).ok()?;
    if !counter(&d.id) || !counter(&d.revision) || !counter(&d.term) {
        return None;
    }
    let prefix = format!("ORDER DT:{}:{}:{} ", d.id, d.revision, d.term);
    if d.all_order_prefill != format!("{prefix}ALL ") {
        return None;
    }
    let mut seen = BTreeSet::new();
    for slot in &d.slots {
        if !(1..=6).contains(&slot.slot)
            || !seen.insert(slot.slot)
            || !label(&slot.callsign, 32)
            || slot.pilot.as_ref().is_some_and(|name| !label(name, 96))
            || !matches!(slot.controller.as_str(), "player" | "npc" | "automated")
            || slot.order_prefill != format!("{prefix}{} ", slot.slot)
        {
            return None;
        }
    }
    if d.leader_slot.is_some_and(|slot| !seen.contains(&slot)) {
        return None;
    }
    if let Some(order) = &d.my_order {
        if !counter(&order.sequence)
            || !label(&order.from, 96)
            || !label(&order.objective, 256)
            || order
                .response
                .as_ref()
                .is_some_and(|r| !matches!(r.as_str(), "wilco" | "negative"))
        {
            return None;
        }
        let reply = |verb| format!("{verb} DT:{}:{} {}", d.id, d.revision, order.sequence);
        if d.wilco_command.as_deref() != Some(reply("WILCO").as_str())
            || d.negative_command.as_deref() != Some(reply("NEGATIVE").as_str())
        {
            return None;
        }
    } else if d.wilco_command.is_some() || d.negative_command.is_some() {
        return None;
    }
    Some(d)
}

impl Detachment {
    pub fn prefill(&self, slot: u16) -> Option<String> {
        self.slots
            .iter()
            .find(|member| member.slot == slot)
            .map(|member| member.order_prefill.clone())
    }

    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "Leader: {}",
            self.leader_slot
                .map_or_else(|| "none".into(), |slot| format!("slot {slot}"))
        )];
        let mut slots: Vec<_> = self.slots.iter().collect();
        slots.sort_by_key(|slot| slot.slot);
        for slot in slots {
            lines.push(format!(
                "{} {} — {}{}",
                slot.slot,
                slot.callsign,
                slot.pilot.as_deref().unwrap_or(&slot.controller),
                if Some(slot.slot) == self.leader_slot {
                    " (leader)"
                } else {
                    ""
                }
            ));
        }
        if let Some(order) = &self.my_order {
            lines.push(format!("Order from {}: {}", order.from, order.objective));
            lines.push(format!(
                "Response: {}",
                order
                    .response
                    .as_ref()
                    .map_or_else(|| "not acknowledged".into(), |r| r.to_uppercase())
            ));
        }
        // SSH has a text composer: show exact commands and preserve its normal
        // explicit Enter submission, as the browser's prefill buttons do.
        lines.push(self.all_order_prefill.clone());
        if let Some(command) = &self.wilco_command {
            lines.push(command.clone());
        }
        if let Some(command) = &self.negative_command {
            lines.push(command.clone());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn fixture() -> Value {
        serde_json::json!({"id":"18446744073709551615","revision":"2","term":"3","leader_slot":2,
            "slots":[{"slot":2,"callsign":"RB","pilot":"Aster","controller":"player","order_prefill":"ORDER DT:18446744073709551615:2:3 2 "}],
            "all_order_prefill":"ORDER DT:18446744073709551615:2:3 ALL ",
            "my_order":{"sequence":"9","from":"Aster","objective":"hold","response":null},
            "wilco_command":"WILCO DT:18446744073709551615:2 9",
            "negative_command":"NEGATIVE DT:18446744073709551615:2 9"})
    }

    #[test]
    fn exact_versions_and_recipient_commands_survive_projection() {
        let d = decode(&fixture()).expect("valid detachment");
        assert_eq!(
            d.prefill(2).as_deref(),
            Some("ORDER DT:18446744073709551615:2:3 2 ")
        );
        assert_eq!(d.prefill(1), None);
        assert!(
            d.lines()
                .iter()
                .any(|line| line == "Order from Aster: hold")
        );
        assert!(
            d.lines()
                .iter()
                .any(|line| line == "Response: not acknowledged")
        );
    }

    #[test]
    fn malformed_prefills_and_duplicate_slots_fail_closed() {
        for (key, value) in [
            ("id", serde_json::json!("01")),
            ("term", serde_json::json!("18446744073709551616")),
            ("leader_slot", serde_json::json!(6)),
            (
                "all_order_prefill",
                serde_json::json!("ORDER ALL \nsay injected"),
            ),
            (
                "wilco_command",
                serde_json::json!("WILCO DT:18446744073709551615:2 8"),
            ),
        ] {
            let mut frame = fixture();
            frame[key] = value;
            assert!(decode(&frame).is_none(), "{key}");
        }
        let mut frame = fixture();
        let duplicate = frame["slots"][0].clone();
        if let Some(slots) = frame["slots"].as_array_mut() {
            slots.push(duplicate);
        }
        assert!(decode(&frame).is_none());
        frame = fixture();
        frame["slots"][0]["controller"] = serde_json::json!("unknown");
        assert!(decode(&frame).is_none());
    }
}
