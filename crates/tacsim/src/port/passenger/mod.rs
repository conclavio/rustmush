//! Conversation-discovered passengers: authored intent, exact fare agreements,
//! real ship occupancy, and delivery through the existing economy contracts.
//!
//! A traveller is a persistent NPC. Its authored role can be reused by content
//! packs; a completed journey never respawns the person or refills their purse.

mod commands;
pub mod content;
pub(crate) mod economy;
pub(crate) mod facts;
mod movement;
pub(crate) mod store;

use mush_core::{CanonicalId, Flags, GOD, ObjId, ObjKind, World};
use serde::{Deserialize, Serialize};

/// Source-owned, staff-only configuration. Negotiation limits never enter prompts.
pub const TRAVELLER_ATTR: &str = "passage:traveller";
pub const SCHEMA: u16 = 1;

/// Reusable authoring template. Identity is pack/entry, never a character name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TravellerDef {
    pub schema: u16,
    pub key: String,
    pub origin: String,
    pub destination: String,
    pub opening_fare_minor: i64,
    pub max_fare_minor: i64,
    pub funding_minor: i64,
}

impl TravellerDef {
    pub fn check(&self) -> Result<(), String> {
        if self.schema != SCHEMA
            || self.key.is_empty()
            || self.key.len() > 160
            || !self
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
            || self.origin == self.destination
            || CanonicalId::new(&self.origin).is_err()
            || CanonicalId::new(&self.destination).is_err()
            || self.opening_fare_minor <= 0
            || self.opening_fare_minor > self.max_fare_minor
            || self.max_fare_minor > self.funding_minor
            || self.funding_minor > 100_000_000
        {
            return Err(
                "Invalid passenger template: use distinct ports and positive bounded fares.".into(),
            );
        }
        Ok(())
    }
}

/// An authored live traveller. The source attribute is not player-writable.
pub(crate) fn traveller(w: &World, npc: ObjId) -> Option<TravellerDef> {
    let object = w.get(npc)?;
    if object.kind != ObjKind::Npc || object.flags.contains(Flags::GOING) {
        return None;
    }
    let attr = object.attrs.get(TRAVELLER_ATTR)?;
    if attr.owner != GOD {
        return None;
    }
    let def: TravellerDef = serde_json::from_value(attr.as_json()?.clone()).ok()?;
    def.check().ok()?;
    Some(def)
}

pub(crate) use commands::register;
pub(crate) use movement::{agents, step, tick, transition_allowed};
pub(crate) use movement::{departure_refusal, voyage_event};
