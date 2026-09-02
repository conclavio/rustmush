//! Object identity ([`ObjId`]), object kinds, and the core [`Obj`] record.
//!
//! Every entity in the game — rooms, exits, things, players, NPCs — is one
//! [`Obj`] identified by a dense, monotonically allocated [`ObjId`] (the
//! classic MUSH "dbref"). Ids are **never recycled**: `@destroy` marks
//! [`crate::flags::Flags::GOING`] and a later sweep leaves a permanent
//! tombstone, so a stale `#123` can never silently point at a new object.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::attr::AttrMap;
use crate::behavior::{BehaviorBinding, BehaviorId};
use crate::flags::{Flags, Powers, Role};
use crate::lock::{LockExpr, LockType};

/// Classic dbref. Monotonic, never recycled.
///
/// Displays and parses as `#123`; serializes as a plain `u32` (the `#` form
/// is a human/UI affordance, not a wire format).
///
/// ```
/// use mush_core::obj::ObjId;
/// let id: ObjId = "#42".parse().unwrap();
/// assert_eq!(id, ObjId(42));
/// assert_eq!(id.to_string(), "#42");
/// ```
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjId(pub u32);

/// Room `#0` — Limbo, the fallback location for homeless objects.
pub const LIMBO: ObjId = ObjId(0);

/// Player `#1` — God, the superuser. `#1` outranks every flag-derived role.
pub const GOD: ObjId = ObjId(1);

impl fmt::Display for ObjId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

impl fmt::Debug for ObjId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Error produced when parsing an [`ObjId`] from its `#123` display form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ObjIdParseError {
    /// The input did not start with `#`.
    #[error("object id must start with '#'")]
    MissingHash,
    /// The part after `#` was not a valid non-negative number.
    #[error("invalid object number after '#': {0:?}")]
    BadNumber(String),
}

impl FromStr for ObjId {
    type Err = ObjIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.strip_prefix('#').ok_or(ObjIdParseError::MissingHash)?;
        rest.parse::<u32>()
            .map(ObjId)
            .map_err(|_| ObjIdParseError::BadNumber(rest.to_string()))
    }
}

/// What an object fundamentally is. Fixed at creation.
///
/// `Npc` is a first-class kind, not a flagged `Player`: it shares the
/// [`is_actor`](ObjKind::is_actor) capability with players (movement, speech,
/// comsys) but has no login credentials; its agent configuration lives in
/// `agent:*` attributes consumed by `mush-npc`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjKind {
    /// A location. `location` is `None`; holds `contents` and `exits`.
    Room,
    /// A passage. `location` is the source room; `destination` the far side.
    Exit,
    /// An ordinary object.
    Thing,
    /// A logged-in-able character. Players own themselves.
    Player,
    /// An LLM- or script-driven character with no login credentials.
    Npc,
}

impl ObjKind {
    /// Players and NPCs are *actors*: they can move, speak, hear, and appear
    /// in the player index for `*name` lookup.
    #[must_use]
    pub fn is_actor(self) -> bool {
        matches!(self, ObjKind::Player | ObjKind::Npc)
    }
}

/// One game object. All mutation goes through [`crate::world::World`]
/// methods, which maintain the inverse indexes (`contents`, `exits`, the
/// player index, zone membership) and the dirty set — handlers never poke
/// fields directly even though they are `pub` for read access.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Obj {
    /// This object's dbref.
    pub id: ObjId,
    /// Fixed kind (see [`ObjKind`]).
    pub kind: ObjKind,
    /// Display name.
    pub name: String,
    /// Alternate names: players ("Nick" for "Nicholas") and exits ("n" for
    /// "north"). Actor aliases are indexed for lookup.
    pub aliases: Vec<SmolStr>,
    /// Owning object. Players own themselves.
    pub owner: ObjId,
    /// Where this object is. `None` for rooms; for exits, the source room.
    pub location: Option<ObjId>,
    /// Exits only: where the exit leads.
    pub destination: Option<ObjId>,
    /// Where the object returns when its container is destroyed (or on
    /// `home`). Defaults to the creation location for actors.
    pub home: Option<ObjId>,
    /// Zone Master Object this object belongs to, if any.
    pub zone: Option<ObjId>,
    /// `@parent` chain for attribute inheritance (prototype objects).
    pub parent: Option<ObjId>,
    /// Maintained inverse of `location` (things/actors held here).
    pub contents: Vec<ObjId>,
    /// Rooms: maintained inverse of exit `location` (exits leading out).
    pub exits: Vec<ObjId>,
    /// Core bit flags.
    pub flags: Flags,
    /// Rhost-style granular powers granted to this object. Wizard+ passes
    /// every power check implicitly (see `mush_core::perm::has_power`), so
    /// grants matter mostly for non-staff. Defaults to empty.
    #[serde(default)]
    pub powers: Powers,
    /// Module-defined marker flags, e.g. `"wod:kindred"`.
    pub markers: BTreeSet<SmolStr>,
    /// Lock expressions by lock type.
    pub locks: BTreeMap<LockType, LockExpr>,
    /// Attribute map (case-insensitive names).
    pub attrs: AttrMap,
    /// Durable, typed behavior bindings (see [`crate::behavior`]). Empty for
    /// almost every object — heavy behavior state lives in the owning module's
    /// namespace, not here — and omitted from the persisted JSON when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub behaviors: BTreeMap<BehaviorId, BehaviorBinding>,
    /// Unix seconds at creation.
    pub created: i64,
    /// Unix seconds of last mutation (maintained by `World` mutators).
    pub modified: i64,
}

impl Obj {
    /// A fresh object with empty collections and no links.
    ///
    /// This is the raw constructor used by [`crate::world::World::create`];
    /// it does **not** wire up any indexes.
    #[must_use]
    pub fn new(id: ObjId, kind: ObjKind, name: impl Into<String>, owner: ObjId, now: i64) -> Obj {
        Obj {
            id,
            kind,
            name: name.into(),
            aliases: Vec::new(),
            owner,
            location: None,
            destination: None,
            home: None,
            zone: None,
            parent: None,
            contents: Vec::new(),
            exits: Vec::new(),
            flags: Flags::empty(),
            powers: Powers::empty(),
            markers: BTreeSet::new(),
            locks: BTreeMap::new(),
            attrs: AttrMap::new(),
            behaviors: BTreeMap::new(),
            created: now,
            modified: now,
        }
    }

    /// Staff role derived from this object's id and flags
    /// (see [`Role::of`]).
    #[must_use]
    pub fn role(&self) -> Role {
        Role::of(self.id, self.flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_display_and_parse_round_trip() {
        for n in [0u32, 1, 123, u32::MAX] {
            let id = ObjId(n);
            let s = id.to_string();
            assert_eq!(s, format!("#{n}"));
            assert_eq!(s.parse::<ObjId>().unwrap(), id);
        }
    }

    #[test]
    fn id_parse_errors() {
        assert_eq!("123".parse::<ObjId>(), Err(ObjIdParseError::MissingHash));
        assert_eq!(
            "#12x".parse::<ObjId>(),
            Err(ObjIdParseError::BadNumber("12x".into()))
        );
        assert_eq!(
            "#-1".parse::<ObjId>(),
            Err(ObjIdParseError::BadNumber("-1".into()))
        );
        assert_eq!(
            "#".parse::<ObjId>(),
            Err(ObjIdParseError::BadNumber(String::new()))
        );
    }

    #[test]
    fn id_serde_is_plain_u32() {
        let json = serde_json::to_string(&ObjId(42)).unwrap();
        assert_eq!(json, "42");
        let back: ObjId = serde_json::from_str("42").unwrap();
        assert_eq!(back, ObjId(42));
    }

    #[test]
    fn kind_is_actor() {
        assert!(ObjKind::Player.is_actor());
        assert!(ObjKind::Npc.is_actor());
        assert!(!ObjKind::Room.is_actor());
        assert!(!ObjKind::Exit.is_actor());
        assert!(!ObjKind::Thing.is_actor());
    }

    #[test]
    fn obj_serde_round_trip() {
        let mut o = Obj::new(ObjId(7), ObjKind::Thing, "widget", GOD, 100);
        o.markers.insert(SmolStr::new("wod:relic"));
        o.locks.insert(LockType::Basic, LockExpr::Is(GOD));
        let json = serde_json::to_string(&o).unwrap();
        let back: Obj = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }
}
