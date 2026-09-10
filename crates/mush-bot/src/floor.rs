//! Cooperative dispatch floor. Expiry revokes; only an acknowledgment releases.
use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub speaking_window_ms: u64,
    pub lease_ms: u64,
    pub revoke_timeout_ms: u64,
    pub priority_wait_ms: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            speaking_window_ms: 1800,
            lease_ms: 5000,
            revoke_timeout_ms: 5000,
            priority_wait_ms: 10_000,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !(1..=60_000).contains(&self.speaking_window_ms)
            || !(100..=60_000).contains(&self.lease_ms)
            || !(100..=60_000).contains(&self.revoke_timeout_ms)
            || !(1..=60_000).contains(&self.priority_wait_ms)
        {
            return Err("invalid speaking-floor policy version or timing limits".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
struct Intent {
    id: u64,
    directed: bool,
    at: u64,
}
#[derive(Clone, Debug, Serialize)]
struct Lease {
    actor: String,
    id: u64,
    at: u64,
    revoking_at: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    Grant(String, u64),
    Revoke(String, u64),
}
pub struct Floor {
    settings: Settings,
    queue: BTreeMap<String, Intent>,
    active: Option<Lease>,
    last_actor: Option<String>,
    quiet_until: u64,
}
impl Floor {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            queue: BTreeMap::new(),
            active: None,
            last_actor: None,
            quiet_until: 0,
        }
    }
    pub fn offer(&mut self, actor: &str, id: u64, directed: bool, now: u64) -> Result<()> {
        if id == 0 {
            return Err("speech intent ID must be positive".into());
        }
        if self.active.as_ref().is_some_and(|a| a.actor == actor) || self.queue.contains_key(actor)
        {
            return Err("actor offered more than one outstanding speech intent".into());
        }
        if self.queue.len() >= 16 {
            return Err("speaking floor queue is full".into());
        }
        self.queue.insert(
            actor.into(),
            Intent {
                id,
                directed,
                at: now,
            },
        );
        Ok(())
    }
    pub fn complete(&mut self, actor: &str, id: u64, now: u64) {
        if self.queue.get(actor).is_some_and(|i| i.id == id) {
            self.queue.remove(actor);
        }
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.actor == actor && a.id == id)
        {
            self.active = None;
            self.last_actor = Some(actor.into());
            // Cancellation can race a sent acknowledgment; conservatively hold the window.
            self.quiet_until = now.saturating_add(self.settings.speaking_window_ms);
        }
    }
    pub fn tick(&mut self, now: u64) -> Result<Option<Control>> {
        if let Some(lease) = &mut self.active {
            if let Some(at) = lease.revoking_at {
                if now.saturating_sub(at) >= self.settings.revoke_timeout_ms {
                    return Err(format!("{} did not acknowledge speaking lease revocation; cast stopped without reassigning the floor", lease.actor).into());
                }
            } else if now.saturating_sub(lease.at) >= self.settings.lease_ms {
                lease.revoking_at = Some(now);
                return Ok(Some(Control::Revoke(lease.actor.clone(), lease.id)));
            }
            return Ok(None);
        }
        if now < self.quiet_until {
            return Ok(None);
        }
        let selected = self
            .queue
            .iter()
            .min_by_key(|(actor, i)| {
                // Aging outranks address priority, then prevent consecutive monopoly.
                let aged = now.saturating_sub(i.at) >= self.settings.priority_wait_ms;
                (
                    !aged,
                    if aged { i.at } else { 0 },
                    self.queue.len() > 1 && self.last_actor.as_ref() == Some(*actor),
                    !i.directed,
                    i.at,
                    *actor,
                )
            })
            .map(|(actor, _)| actor.clone());
        if let Some(actor) = selected {
            let intent = self.queue.remove(&actor).unwrap();
            self.active = Some(Lease {
                actor: actor.clone(),
                id: intent.id,
                at: now,
                revoking_at: None,
            });
            return Ok(Some(Control::Grant(actor, intent.id)));
        }
        Ok(None)
    }
    pub fn snapshot(&self) -> Value {
        json!({"policy":self.settings,"queue":self.queue,"active":self.active,"quiet_until_elapsed_ms":self.quiet_until,"last_actor":self.last_actor})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_grant_is_never_reassigned_without_ack() {
        let mut f = Floor::new(Settings::default());
        f.offer("a", 1, false, 0).unwrap();
        f.offer("b", 1, true, 0).unwrap();
        assert_eq!(f.tick(0).unwrap(), Some(Control::Grant("b".into(), 1)));
        assert_eq!(f.tick(5000).unwrap(), Some(Control::Revoke("b".into(), 1)));
        f.complete("b", 2, 5100); // stale/unrelated acknowledgment cannot unlock the floor.
        assert_eq!(f.tick(6000).unwrap(), None);
        assert!(f.tick(10_000).is_err());
        f.complete("b", 1, 10_001);
        assert_eq!(f.tick(11_800).unwrap(), None);
        assert_eq!(f.tick(11_801).unwrap(), Some(Control::Grant("a".into(), 1)));
    }
    #[test]
    fn priority_aging_and_alternation_allow_other_speakers() {
        let mut f = Floor::new(Settings {
            speaking_window_ms: 1,
            priority_wait_ms: 100,
            ..Default::default()
        });
        f.offer("a", 1, true, 0).unwrap();
        f.offer("b", 1, false, 0).unwrap();
        assert_eq!(f.tick(0).unwrap(), Some(Control::Grant("a".into(), 1)));
        f.complete("a", 1, 1);
        f.offer("a", 2, true, 1).unwrap();
        assert_eq!(f.tick(2).unwrap(), Some(Control::Grant("b".into(), 1)));
        f.complete("b", 1, 3);
        f.offer("b", 2, true, 200).unwrap();
        assert_eq!(f.tick(200).unwrap(), Some(Control::Grant("a".into(), 2)));
    }
    #[test]
    fn oldest_aged_waiter_outranks_other_aged_direct_addresses() {
        let mut f = Floor::new(Settings::default());
        f.offer("quiet", 1, false, 0).unwrap();
        f.offer("addressed", 1, true, 100).unwrap();
        assert_eq!(
            f.tick(20_000).unwrap(),
            Some(Control::Grant("quiet".into(), 1))
        );
    }
}
