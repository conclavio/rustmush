//! **A hull's logistics and its walkable interior** (doc 18 cut IC2; doc 18
//! §4.2 "The Dray definition" and its schema additions (2), D-18-28 as replaced
//! by R7, D-18-29, D-18-45).
//!
//! Two optional blocks a [`super::UnitSpec`] may carry, read by tacsim's port
//! code only (the passage planner, the capacity function, the hull template of
//! cut IC4) and **never** by `bind()`, so declaring them moves no
//! `DefinitionRef`:
//!
//! - [`LogisticsDef`] — the dry mass, the stowage compartments as doc 06 D3
//!   measure maps (`si.mass_kg`, `si.volume_l`: a later measure is a data key on
//!   both sides), the berths and their occupant allowance, the payload rail and
//!   the drive. The drive's law is an externally tagged enum (`{"coast": {…}}`)
//!   so the astronomy engine's flip-and-burn law (post-slice cut P-A) is a new
//!   variant and not a new field. One drive class flies one law across the whole
//!   fielded catalog ([`super::Catalog::drive_for_class`], Q-18-5, D-18-45).
//!   The drive is optional since doc 20 cut SM.3a: a stowage-only unit (doc 20
//!   cut SM.3a's terrain vehicle) declares no drive and never plans a passage
//!   (doc 20 §4.1 "stowage-only").
//! - [`InteriorDef`] — the rooms the hull template instantiates (18a §3): one
//!   walkable room that is the space object, one store room that backs the hold
//!   vault and that no exit reaches, the exits between the walkable rooms with
//!   both directions' names, and which rooms show which compartment's gauge.
//!   Room text rides the template (lead ruling R6), held to the atlas's own
//!   bounds by value so no atlas edge is needed. Since doc 20 cut SM.3a (§4.3,
//!   D-20-7) an interior may also declare vehicle **bays** ([`BayDef`]): an
//!   exitless room that holds rigs, not people, its one mush-sim compartment and
//!   the hull-local pose the rigs leave by ([`EgressDef`], definition data
//!   only until cut SM.3b).
//!
//! Every number is an integer; every refusal is a sentence naming the field and
//! the value, under the entry's id (`DefinitionError::Value`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mush_core::CanonicalId;
use serde::{Deserialize, Serialize};

use super::{DefinitionError, MAX_LABEL_BYTES};

/// The doc 06 D3 measure id for mass, kilograms (doc 18 §4.3).
pub const MASS_KG: &str = "si.mass_kg";
/// The doc 06 D3 measure id for packed volume, litres (doc 18 §4.3).
pub const VOLUME_L: &str = "si.volume_l";

/// The heaviest dry hull a definition may declare, tonnes.
pub const MAX_DRY_MASS_T: u32 = 1_000_000;
/// The most stowage compartments one hull may declare.
pub const MAX_COMPARTMENTS: usize = 8;
/// The most stowage tags one compartment may accept, and the most measures it
/// may limit.
pub const MAX_COMPARTMENT_KEYS: usize = 8;
/// The most berths one hull may declare (doc 18 §4.2: the interior's Members
/// cap is 16).
pub const MAX_BERTHS: u32 = 16;
/// The largest per-occupant allowance, kilograms.
pub const MAX_OCCUPANT_ALLOWANCE_KG: u32 = 1_000;
/// The fastest coast a drive law may author, km/s: the speed of light.
pub const MAX_COAST_KMS: u32 = 299_792;
/// The largest burn fraction, permille of departure mass.
pub const MAX_BURN_PERMILLE: u32 = 1_000;

/// The fewest rooms an interior may declare: the space and the store.
pub const MIN_INTERIOR_ROOMS: usize = 2;
/// The most rooms an interior may declare (doc 18 §4.2: the Members cap, 16).
pub const MAX_INTERIOR_ROOMS: usize = 16;
/// The most exits an interior may declare.
pub const MAX_INTERIOR_EXITS: usize = 32;
/// The longest `short_desc` an interior room may carry, bytes.
pub const MAX_INTERIOR_SHORT_DESC_BYTES: usize = 256;
/// The longest `desc` an interior room may carry, bytes.
pub const MAX_INTERIOR_DESC_BYTES: usize = 4 * 1024;
/// The most details one interior room may carry — the atlas's `MAX_DETAILS`,
/// stated by value (`crates/mush-atlas/src/apply.rs`).
pub const MAX_INTERIOR_DETAILS: usize = 32;
/// The longest detail name, bytes — the atlas's `check_details` rule by value.
pub const MAX_DETAIL_NAME_BYTES: usize = 64;
/// The longest detail text, bytes — the atlas's `MAX_DETAIL_BYTES` by value.
pub const MAX_DETAIL_TEXT_BYTES: usize = 2 * 1024;

/// **A hull's logistics** (doc 18 §4.2; D-18-28 as replaced by R7, D-18-29).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogisticsDef {
    /// The empty hull, tonnes; `1..=1_000_000`. The burn's departure mass is
    /// this plus the payload (doc 18 §4.8).
    pub dry_mass_t: u32,
    /// The stowage compartments, by id; `1..=8` of them.
    pub stowage: BTreeMap<CanonicalId, StowageDef>,
    /// How many people the hull carries; `1..=16`.
    pub berths: u32,
    /// What one occupant weighs against the payload, kilograms; `1..=1_000`.
    pub occupant_allowance_kg: u32,
    /// The payload rail and `route_duration`'s laden denominator, kilograms; at
    /// or above the stowage mass limits plus every berth's allowance, so it
    /// never refuses on its own (doc 18 §4.2).
    pub payload_ceiling_kg: u64,
    /// The drive: its class and the law the class flies under. `None` for a
    /// stowage-only unit (doc 20 cut SM.3a's terrain vehicle, §4.1), which
    /// never plans a passage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive: Option<DriveDef>,
}

/// **One stowage compartment**: the stowage tags it accepts and doc 06 D3's
/// measure map of its limits (doc 18 §4.2, §4.3).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StowageDef {
    /// The stowage tags a good must carry to go in (`stow.bulk`); `1..=8`.
    pub accepts: BTreeSet<CanonicalId>,
    /// Measure id → limit; `1..=8` entries, every value positive, and
    /// `si.mass_kg` always among them.
    pub limits: BTreeMap<CanonicalId, u64>,
}

impl StowageDef {
    /// Re-run the shared bounded tag/measure rules for a real compartment.
    pub fn check(&self, entry: &str, id: &CanonicalId) -> Result<(), DefinitionError> {
        if self.accepts.is_empty() || self.accepts.len() > MAX_COMPARTMENT_KEYS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "compartment `{id}` accepts 1 to {MAX_COMPARTMENT_KEYS} stowage tags; {} \
                     is not",
                    self.accepts.len()
                ),
            ));
        }
        if self.limits.is_empty() || self.limits.len() > MAX_COMPARTMENT_KEYS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "compartment `{id}` limits 1 to {MAX_COMPARTMENT_KEYS} measures; {} is not",
                    self.limits.len()
                ),
            ));
        }
        if let Some((measure, _)) = self.limits.iter().find(|(_, limit)| **limit == 0) {
            return Err(DefinitionError::value(
                entry,
                format!("compartment `{id}` limits `{measure}` to 0; a limit is positive"),
            ));
        }
        // Payload is mass: every compartment says what it weighs.
        if !self
            .limits
            .keys()
            .any(|measure| measure.as_str() == MASS_KG)
        {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "compartment `{id}` does not limit {MASS_KG}; every compartment says what \
                     it weighs"
                ),
            ));
        }
        Ok(())
    }
}

/// **A hull's drive**: its class, and the law that class flies under.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveDef {
    /// The drive class a route's `drive_class` names (`coast_hauler`).
    pub class: CanonicalId,
    /// The law (lead ruling R7: it lives here because the definition decoder
    /// needs it; the passage planner of cut IC9 is built against this type).
    pub law: DriveLaw,
}

/// **A drive law** — externally tagged on the wire, `{"coast": {…}}` or
/// `{"flip": {…}}` (D-18-45; doc 19 cut A.2, D-19-8).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DriveLaw {
    /// Coast at a fixed unladen cruise; the burn is a fraction of departure
    /// mass per departure, distance-free doctrine (doc 18 §4.8).
    Coast {
        /// The unladen cruise, km/s; `1..=299_792`.
        kms: u32,
        /// The burn per departure, permille of `dry_mass_t + payload_t`;
        /// `1..=1_000`.
        burn_permille: u32,
    },
    /// **Flip and burn** (doc 19 §4.1): constant thrust, accelerate to the
    /// midpoint, flip, decelerate — `t = 2√(d / a)` per leg — with the burn by
    /// the rocket equation over the passage's `Δv = Σ a·t`. Thrust-limited, so a
    /// lighter hull is faster: `accel_mg` is the acceleration at the rated
    /// maximum departure mass (`dry_mass_t + payload_ceiling_kg`).
    Flip {
        /// The acceleration at the rated maximum departure mass, milli-g;
        /// `1..=MAX_ACCEL_MG`.
        accel_mg: u32,
        /// The exhaust velocity, km/s; `1..=MAX_EXHAUST_KMS`.
        exhaust_kms: u32,
    },
}

/// The hardest a drive may push at its rated mass, milli-g (ten gravities).
pub const MAX_ACCEL_MG: u32 = 10_000;
/// The fastest exhaust a drive may author, km/s (light).
pub const MAX_EXHAUST_KMS: u32 = 299_792;

impl DriveLaw {
    /// **The law's word** (doc 18 cut IC9, lead ruling L20): what a passage plan
    /// records as `PassagePlan.law` (`coast`, `flip`); the drive class itself is
    /// pinned by the ticket's `unit_definition`.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            DriveLaw::Coast { .. } => "coast",
            DriveLaw::Flip { .. } => "flip",
        }
    }
}

impl fmt::Display for DriveLaw {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriveLaw::Coast { kms, burn_permille } => {
                write!(f, "coast {{kms {kms}, burn_permille {burn_permille}}}")
            }
            DriveLaw::Flip {
                accel_mg,
                exhaust_kms,
            } => write!(f, "flip {{accel_mg {accel_mg}, exhaust_kms {exhaust_kms}}}"),
        }
    }
}

impl LogisticsDef {
    /// The stowage mass limits plus every berth's allowance, kilograms — the
    /// floor the payload rail may not go below. `None` on overflow.
    #[must_use]
    pub fn payload_floor_kg(&self) -> Option<u64> {
        let mass = CanonicalId::new(MASS_KG).ok()?;
        let mut sum: u64 = 0;
        for def in self.stowage.values() {
            sum = sum.checked_add(def.limits.get(&mass).copied().unwrap_or(0))?;
        }
        let occupants =
            u64::from(self.berths).checked_mul(u64::from(self.occupant_allowance_kg))?;
        sum.checked_add(occupants)
    }

    /// Re-run every bound (doc 18 §4.2; lead ruling R15).
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry, the field and the value.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        if self.dry_mass_t == 0 || self.dry_mass_t > MAX_DRY_MASS_T {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "logistics.dry_mass_t is 1 to {MAX_DRY_MASS_T} t; {} is not",
                    self.dry_mass_t
                ),
            ));
        }
        if self.stowage.is_empty() || self.stowage.len() > MAX_COMPARTMENTS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "logistics.stowage declares 1 to {MAX_COMPARTMENTS} compartments; {} is not",
                    self.stowage.len()
                ),
            ));
        }
        if self.berths == 0 || self.berths > MAX_BERTHS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "logistics.berths is 1 to {MAX_BERTHS}; {} is not",
                    self.berths
                ),
            ));
        }
        if self.occupant_allowance_kg == 0 || self.occupant_allowance_kg > MAX_OCCUPANT_ALLOWANCE_KG
        {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "logistics.occupant_allowance_kg is 1 to {MAX_OCCUPANT_ALLOWANCE_KG}; {} is not",
                    self.occupant_allowance_kg
                ),
            ));
        }
        for (id, def) in &self.stowage {
            def.check(entry, id)?;
        }
        // Doc 20 cut SM.3a: a stowage-only unit declares no drive, so it has no
        // law to bound.
        if let Some(drive) = &self.drive {
            match drive.law {
                DriveLaw::Coast { kms, burn_permille } => {
                    if kms == 0 || kms > MAX_COAST_KMS {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "drive `{}` coasts at 1 to {MAX_COAST_KMS} km/s; {kms} is not",
                                drive.class
                            ),
                        ));
                    }
                    if burn_permille == 0 || burn_permille > MAX_BURN_PERMILLE {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "drive `{}` burns 1 to {MAX_BURN_PERMILLE} permille of its departure \
                                 mass; {burn_permille} is not",
                                drive.class
                            ),
                        ));
                    }
                }
                DriveLaw::Flip {
                    accel_mg,
                    exhaust_kms,
                } => {
                    if accel_mg == 0 || accel_mg > MAX_ACCEL_MG {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "drive `{}` flips at 1 to {MAX_ACCEL_MG} milli-g; {accel_mg} is not",
                                drive.class
                            ),
                        ));
                    }
                    if exhaust_kms == 0 || exhaust_kms > MAX_EXHAUST_KMS {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "drive `{}` exhausts at 1 to {MAX_EXHAUST_KMS} km/s; {exhaust_kms} is not",
                                drive.class
                            ),
                        ));
                    }
                }
            }
        }
        // Doc 18 §4.2: "a rail at or above the stowage maxima plus occupants".
        let Some(floor) = self.payload_floor_kg() else {
            return Err(DefinitionError::value(
                entry,
                "the stowage mass limits plus the occupants overflow",
            ));
        };
        if self.payload_ceiling_kg < floor {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "logistics.payload_ceiling_kg {} is below the stowage mass limits plus the \
                     occupants, {floor}",
                    self.payload_ceiling_kg
                ),
            ));
        }
        Ok(())
    }
}

/// **A hull's walkable interior** (doc 18 §4.2; 18a §3; lead ruling R6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteriorDef {
    /// The walkable room that is the space object (the Airlock).
    pub space: CanonicalId,
    /// The store room: no exits, the hold vault's backing.
    pub store: CanonicalId,
    /// The occupancy slot, whose maximum is the hull's berths.
    pub berths_slot: BerthsSlotDef,
    /// The rooms, `2..=16`, in authored order.
    pub rooms: Vec<InteriorRoomDef>,
    /// The exits between walkable rooms, `0..=32`.
    pub exits: Vec<InteriorExitDef>,
    /// Stowage compartment id → the walkable rooms that show its gauge (18a §3).
    pub gauges: BTreeMap<CanonicalId, Vec<CanonicalId>>,
    /// **The vehicle bays** (doc 20 cut SM.3a; §4.3, D-20-7), `0..=MAX_BAYS`:
    /// each names one [`InteriorRoomKind::Bay`] room, its compartment and its
    /// egress. Empty for every hull but the Dray-P, and serialised only when
    /// declared, so no earlier definition's digest moves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bays: Vec<BayDef>,
}

/// The most vehicle bays one interior may declare (doc 20 cut SM.3a).
pub const MAX_BAYS: usize = 2;
/// The most rigs one bay compartment may hold (doc 20 cut SM.3a).
pub const MAX_BAY_ITEMS: u32 = 4;
/// The farthest a bay's egress may sit from the hull's origin on either axis,
/// centimetres (100 m).
pub const MAX_EGRESS_OFFSET_CM: i64 = 10_000;

/// **One vehicle bay** (doc 20 cut SM.3a; §4.3, D-20-7): the bay room, the
/// mush-sim compartment its rigs are placed in, and where they leave by.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayDef {
    /// The [`InteriorRoomKind::Bay`] room the rigs stand in.
    pub room: CanonicalId,
    /// The compartment every rig of the bay shares (doc 20: "a compartment
    /// `maximum_items 2`").
    pub slot: BaySlotDef,
    /// The pose the rigs leave the hull by.
    pub egress: EgressDef,
}

/// **A bay's compartment**: the mush-sim slot id and how many rigs it holds.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaySlotDef {
    /// The slot id (`rigs`); never the berths slot's.
    pub id: CanonicalId,
    /// The rig cap, `1..=MAX_BAY_ITEMS`.
    pub maximum_items: u32,
}

/// **A bay's egress** (doc 20 §4.1 "at the ramp pose"): a hull-local offset —
/// x starboard, y forward, both centimetres — and a heading relative to the
/// hull's own, milli-degrees. Definition data only in cut SM.3a; cut SM.3b turns
/// it into a site pose (no mush-sim egress writer exists yet).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EgressDef {
    /// What the egress is called (`stern ramp`).
    pub label: String,
    /// Starboard of the hull's origin, centimetres.
    pub x_cm: i64,
    /// Forward of the hull's origin, centimetres.
    pub y_cm: i64,
    /// Relative to the hull's heading, milli-degrees, `0..360_000`.
    pub heading_mdeg: u32,
}

/// **The berths slot**: the mush-sim compartment that caps occupancy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BerthsSlotDef {
    /// The slot id (`berths`).
    pub id: CanonicalId,
    /// The occupancy cap; equal to `logistics.berths`.
    pub maximum_items: u32,
}

/// **One interior room** — its text is 18a §3's (lead ruling R6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteriorRoomDef {
    /// The template-local id.
    pub id: CanonicalId,
    /// The room's name.
    pub name: String,
    /// Walkable, or the store.
    pub kind: InteriorRoomKind,
    /// The one-line description, `0..=256` bytes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub short_desc: String,
    /// The room description, `0..=4_096` bytes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub desc: String,
    /// The details, name → text, under the atlas's own rules.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
}

/// What kind of interior room.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteriorRoomKind {
    /// A room a person walks in.
    Walkable,
    /// The store: no exits, nobody walks in.
    Store,
    /// **A vehicle bay** (doc 20 cut SM.3a): no exits, reached by the ramp,
    /// holds rigs, not people.
    Bay,
}

/// **One exit pair**: `names[0]` leaves `from` for `to`, `names[1]` comes back.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteriorExitDef {
    /// The room `names[0]` leaves.
    pub from: CanonicalId,
    /// The room `names[1]` leaves.
    pub to: CanonicalId,
    /// The two exits' names, `;`-separated aliases after the display name.
    pub names: Vec<String>,
}

/// The aliases of one exit name: every `;`-separated segment after the first,
/// trimmed and lower-cased.
fn aliases(name: &str) -> Vec<String> {
    name.split(';')
        .skip(1)
        .map(|alias| alias.trim().to_ascii_lowercase())
        .filter(|alias| !alias.is_empty())
        .collect()
}

impl InteriorDef {
    /// The room `id`, when declared.
    #[must_use]
    pub fn room(&self, id: &CanonicalId) -> Option<&InteriorRoomDef> {
        self.rooms.iter().find(|room| &room.id == id)
    }

    /// **The first vehicle bay** (doc 20 cut SM.3a), when the interior declares
    /// one.
    #[must_use]
    pub fn bay(&self) -> Option<&BayDef> {
        self.bays.first()
    }

    fn is_walkable(&self, id: &CanonicalId) -> bool {
        self.room(id)
            .is_some_and(|room| room.kind == InteriorRoomKind::Walkable)
    }

    /// Refuse what the hull template could not build (doc 18 §4.2, 18a §3).
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and what is wrong.
    pub fn check(&self, entry: &str, logistics: &LogisticsDef) -> Result<(), DefinitionError> {
        let refuse = |detail: String| Err(DefinitionError::value(entry, detail));
        if self.rooms.len() < MIN_INTERIOR_ROOMS || self.rooms.len() > MAX_INTERIOR_ROOMS {
            return refuse(format!(
                "an interior has {MIN_INTERIOR_ROOMS} to {MAX_INTERIOR_ROOMS} rooms; {} is not",
                self.rooms.len()
            ));
        }
        let mut ids: BTreeSet<&CanonicalId> = BTreeSet::new();
        let mut store: Option<&CanonicalId> = None;
        for room in &self.rooms {
            if !ids.insert(&room.id) {
                return refuse(format!("interior room `{}` is declared twice", room.id));
            }
            if room.name.trim().is_empty() || room.name.len() > MAX_LABEL_BYTES {
                return refuse(format!(
                    "interior room `{}`'s name is 1 to {MAX_LABEL_BYTES} bytes; {} is not",
                    room.id,
                    room.name.len()
                ));
            }
            if room.short_desc.len() > MAX_INTERIOR_SHORT_DESC_BYTES {
                return refuse(format!(
                    "interior room `{}`'s short_desc is at most {MAX_INTERIOR_SHORT_DESC_BYTES} \
                     bytes; {} is not",
                    room.id,
                    room.short_desc.len()
                ));
            }
            if room.desc.len() > MAX_INTERIOR_DESC_BYTES {
                return refuse(format!(
                    "interior room `{}`'s desc is at most {MAX_INTERIOR_DESC_BYTES} bytes; {} is \
                     not",
                    room.id,
                    room.desc.len()
                ));
            }
            if room.details.len() > MAX_INTERIOR_DETAILS {
                return refuse(format!(
                    "interior room `{}` carries at most {MAX_INTERIOR_DETAILS} details; {} is not",
                    room.id,
                    room.details.len()
                ));
            }
            for (name, text) in &room.details {
                if name.trim().is_empty()
                    || name.len() > MAX_DETAIL_NAME_BYTES
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b' ')
                {
                    return refuse(format!(
                        "interior room `{}`'s detail name {name:?} is not a short plain name",
                        room.id
                    ));
                }
                if text.trim().is_empty() || text.len() > MAX_DETAIL_TEXT_BYTES {
                    return refuse(format!(
                        "interior room `{}`'s detail {name:?} is empty or over \
                         {MAX_DETAIL_TEXT_BYTES} bytes",
                        room.id
                    ));
                }
            }
            if room.kind == InteriorRoomKind::Store {
                if let Some(first) = store {
                    return refuse(format!(
                        "an interior has one store room; `{first}` and `{}` are both",
                        room.id
                    ));
                }
                store = Some(&room.id);
            }
        }
        if !self.is_walkable(&self.space) {
            return refuse(format!(
                "interior.space `{}` is not a walkable room",
                self.space
            ));
        }
        if !self
            .room(&self.store)
            .is_some_and(|room| room.kind == InteriorRoomKind::Store)
        {
            return refuse(format!(
                "interior.store `{}` is not a store room",
                self.store
            ));
        }
        if self.exits.len() > MAX_INTERIOR_EXITS {
            return refuse(format!(
                "an interior has at most {MAX_INTERIOR_EXITS} exits; {} is not",
                self.exits.len()
            ));
        }
        let mut pairs: BTreeSet<(&CanonicalId, &CanonicalId)> = BTreeSet::new();
        let mut leaving: BTreeMap<&CanonicalId, BTreeSet<String>> = BTreeMap::new();
        for exit in &self.exits {
            let (from, to) = (&exit.from, &exit.to);
            // The store is reached by panels and hoses, never by an exit (18a
            // §3.1 "Hold Stowage"): no person can become vault custody.
            if from == &self.store || to == &self.store {
                return refuse(format!(
                    "the exit `{from}` -> `{to}` touches the store `{}`; a store has no exits",
                    self.store
                ));
            }
            // Doc 20 cut SM.3a: a bay is reached by the ramp, never by an exit,
            // so no person walks in among the rigs.
            if let Some(bay) = [from, to].into_iter().find(|end| {
                self.room(end)
                    .is_some_and(|room| room.kind == InteriorRoomKind::Bay)
            }) {
                return refuse(format!(
                    "the exit `{from}` -> `{to}` touches the bay `{bay}`; a bay has no exits"
                ));
            }
            for end in [from, to] {
                if !self.is_walkable(end) {
                    return refuse(format!(
                        "the exit `{from}` -> `{to}` names `{end}`, which is not a walkable room"
                    ));
                }
            }
            if from == to {
                return refuse(format!(
                    "the exit `{from}` -> `{to}` joins a room to itself"
                ));
            }
            if exit.names.len() != 2 {
                return refuse(format!(
                    "the exit `{from}` -> `{to}` carries two names, one each way; {} is not",
                    exit.names.len()
                ));
            }
            if exit.names.iter().any(|name| {
                name.split(';')
                    .next()
                    .is_none_or(|display| display.trim().is_empty())
            }) {
                return refuse(format!("the exit `{from}` -> `{to}` has an empty name"));
            }
            let pair = if from < to { (from, to) } else { (to, from) };
            if !pairs.insert(pair) {
                return refuse(format!("two exits join `{}` and `{}`", pair.0, pair.1));
            }
            // `names[0]` leaves `from`, `names[1]` leaves `to`.
            for (room, name) in [(from, exit.names.first()), (to, exit.names.get(1))] {
                let seen = leaving.entry(room).or_default();
                for alias in name.map(|n| aliases(n)).unwrap_or_default() {
                    if !seen.insert(alias.clone()) {
                        return refuse(format!("the alias `{alias}` leaves `{room}` twice"));
                    }
                }
            }
        }
        // Every walkable room is reachable from the space over the exits.
        let mut reached: BTreeSet<&CanonicalId> = BTreeSet::new();
        let mut frontier: Vec<&CanonicalId> = vec![&self.space];
        while let Some(room) = frontier.pop() {
            if !reached.insert(room) {
                continue;
            }
            for exit in &self.exits {
                if &exit.from == room {
                    frontier.push(&exit.to);
                } else if &exit.to == room {
                    frontier.push(&exit.from);
                }
            }
        }
        if let Some(lost) = self
            .rooms
            .iter()
            .find(|room| room.kind == InteriorRoomKind::Walkable && !reached.contains(&room.id))
        {
            return refuse(format!(
                "interior room `{}` cannot be reached from `{}`",
                lost.id, self.space
            ));
        }
        if self.berths_slot.maximum_items != logistics.berths {
            return refuse(format!(
                "berths_slot.maximum_items is {} and logistics.berths is {}; they agree",
                self.berths_slot.maximum_items, logistics.berths
            ));
        }
        for (gauge, rooms) in &self.gauges {
            if !logistics.stowage.contains_key(gauge) {
                return refuse(format!(
                    "the gauge `{gauge}` names no logistics.stowage compartment"
                ));
            }
            if let Some(room) = rooms.iter().find(|room| !self.is_walkable(room)) {
                return refuse(format!(
                    "the gauge `{gauge}` shows in `{room}`, which is not a walkable room"
                ));
            }
        }
        self.check_bays(entry)
    }

    /// **The bays** (doc 20 cut SM.3a; §4.3, D-20-7): what the applier's step 6b
    /// could not lay up, refused at the definition.
    fn check_bays(&self, entry: &str) -> Result<(), DefinitionError> {
        let refuse = |detail: String| Err(DefinitionError::value(entry, detail));
        if self.bays.len() > MAX_BAYS {
            return refuse(format!(
                "an interior has at most {MAX_BAYS} bays; {} is not",
                self.bays.len()
            ));
        }
        for end in [&self.space, &self.store] {
            if self.bays.iter().any(|bay| &bay.room == end) {
                return refuse(format!(
                    "the bay `{end}` is the interior's space or store; a bay is its own room"
                ));
            }
        }
        let mut named: BTreeSet<&CanonicalId> = BTreeSet::new();
        for bay in &self.bays {
            if !self
                .room(&bay.room)
                .is_some_and(|room| room.kind == InteriorRoomKind::Bay)
            {
                return refuse(format!("the bay `{}` names no bay room", bay.room));
            }
            if !named.insert(&bay.room) {
                return refuse(format!("the bay room `{}` is named by two bays", bay.room));
            }
            if bay.slot.maximum_items == 0 || bay.slot.maximum_items > MAX_BAY_ITEMS {
                return refuse(format!(
                    "the bay `{}` holds 1 to {MAX_BAY_ITEMS} rigs; {} is not",
                    bay.room, bay.slot.maximum_items
                ));
            }
            if bay.slot.id == self.berths_slot.id {
                return refuse(format!(
                    "the bay `{}`'s slot `{}` is the berths slot's; a bay has its own",
                    bay.room, bay.slot.id
                ));
            }
            let label = &bay.egress.label;
            if label.trim().is_empty() || label.len() > MAX_LABEL_BYTES {
                return refuse(format!(
                    "the bay `{}`'s egress label is 1 to {MAX_LABEL_BYTES} bytes; {} is not",
                    bay.room,
                    label.len()
                ));
            }
            for (axis, offset) in [("x_cm", bay.egress.x_cm), ("y_cm", bay.egress.y_cm)] {
                if offset.unsigned_abs() > MAX_EGRESS_OFFSET_CM.unsigned_abs() {
                    return refuse(format!(
                        "the bay `{}`'s egress {axis} is within {MAX_EGRESS_OFFSET_CM} cm of the \
                         hull; {offset} is not",
                        bay.room
                    ));
                }
            }
            if bay.egress.heading_mdeg >= 360_000 {
                return refuse(format!(
                    "the bay `{}`'s egress heading_mdeg is below 360000; {} is not",
                    bay.room, bay.egress.heading_mdeg
                ));
            }
        }
        if let Some(room) = self
            .rooms
            .iter()
            .find(|room| room.kind == InteriorRoomKind::Bay && !named.contains(&room.id))
        {
            return refuse(format!("the bay room `{}` is named by no bay", room.id));
        }
        Ok(())
    }
}
