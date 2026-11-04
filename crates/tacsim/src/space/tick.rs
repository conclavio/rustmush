//! **World Tick Consumer, Interdiction Promotion & Unit Custody Seam (`SF.R1`, `SF.R2`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §6.1, `D-35-1`, `D-35-3`, `D-35-5`,
//! `D-35-12`, `Q-35-4(1)`, `docs/design/35a-space-flight-review.md` §8.3).
//!
//! Connects [`LocalSpaceBubble`] to `mush-sim`'s [`CarrierJourney`] and [`ActivationBook`]
//! and `mush-tactics`'s [`mush_tactics::custody`] on the coarse world tick ([`Registrar::tick`]):
//! 1. When an under-way carrier crosses an interdicted leg boundary (`LegDef::interdiction > 0`)
//!    and `mush-sim` raises [`PromotionTrigger::Interception`] on the waypoint domain,
//!    [`space_tick`] promotes the [`CarrierJourney`] to [`JourneyState::Promoted`]
//!    (which pauses the carrier's leg timer while keeping its remaining leg duration intact),
//!    checks out the carrier's [`mush_tactics::TacticalUnit`] via [`check_out_space_unit`],
//!    and opens a [`LocalSpaceBubble`] populated from canonical `UnitSpec` hulls.
//! 2. On each subsequent tick fire while the bubble is open, [`space_tick`] steps the
//!    bubble on **1:1 wall time** ([`LocalSpaceBubble::step_wall_ms_at`]) using the
//!    adaptive step cadence (`5,000 ms` cruise / `1,000 ms` combat / `500 ms` ordnance in flight),
//!    stamps the live W-clock `now.at` without scaling physics by the world IC rate, and appends
//!    combat hits and disablement events to [`SalvageClaimBook`]'s tamper-evident black box.
//! 3. When the encounter clears (all hostile contacts disabled/derelict or explicitly
//!    cleared, and no ordnance in flight), [`space_tick`] checks the carrier's [`mush_tactics::TacticalUnit`]
//!    back in via [`check_in_space_unit`] (persisting all section/facet/component/heat mutations
//!    to `UnitStore`), opens a 48 real-hour (`172,800 s`) salvage contest window in
//!    [`SalvageClaimBook`] for any disabled derelict hull (`D-35-12`), closes the bubble into
//!    [`SpaceBubbleBook::completed`], resumes the [`CarrierJourney`] to
//!    [`JourneyState::Cruising`], and stands down the domain activation.
//! 4. If the carrier hull itself is disabled into an open-epoch derelict (`D-35-12`),
//!    [`space_tick`] checks its [`mush_tactics::TacticalUnit`] back in (`Immobile` / `Disabled` /
//!    `Wreck`, `FieldEpochDisposition::RemainOpen`), records its [`SpaceDerelictRecord`],
//!    opens its 48 real-hour [`CustodyTitleStatus::PendingSalvageClaim`] in [`SalvageClaimBook`],
//!    and leaves the carrier paused at the waypoint domain with its epoch open.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar, TickCadence};
use mush_core::{ClockStamp, Ctx, ModuleWrite, ObjId, SceneId, World};
use mush_sim::tick::{OrderChannel, OrderValue};
use mush_sim::{
    ActivationBook, CarrierJourney, DomainId, JourneyBook, JourneyState, PromotionTrigger,
    SpatialState, promote_carrier_journey, resume_carrier_journey, stand_down_domain,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::bubble::{
    BubbleId, BubbleInputCommand, BubbleInputTape, BubbleSnapshot, DERELICT_WINDOW_S, EmconPosture,
    HullStatus, LocalSpaceBubble, SpaceShipState, admitted_party,
};
use super::custody::{
    check_in_space_unit, check_out_space_unit, instantiate_stock_space_unit,
    record_space_combat_to_black_box, space_ship_from_tactical_unit,
};
use crate::arena::party::{Mark, PartyId, Relation};
use crate::field_ops::{
    CustodyTitleStatus, FieldUnitAssessment, OperationalState, RecoveryOpId, SalvageClaimBook,
};
use crate::store::TacsimStore;

/// Coarse tick tag registered by [`register`].
pub const SPACE_TICK: &str = "tacsim.space";

/// Coarse tick period (`1 s` real wall cadence).
pub const SPACE_TICK_PERIOD: Duration = Duration::from_secs(1);

/// Adaptive 1:1 wall-time step when no hostile contact is operational and no ordnance is in flight (`5,000 ms`).
pub const CRUISE_STEP_WALL_MS: u32 = 5_000;

/// Adaptive 1:1 wall-time step when an operational hostile contact is in the bubble (`1,000 ms`).
pub const COMBAT_STEP_WALL_MS: u32 = 1_000;

/// Adaptive 1:1 wall-time step when a torpedo or slug is in flight (`500 ms`).
pub const ORDNANCE_STEP_WALL_MS: u32 = 500;

/// Party identifier for the promoted carrier's side inside an interdiction bubble.
pub const CARRIER_PARTY: PartyId = admitted_party(0);

/// Party identifier for the interdicting hostile picket inside an interdiction bubble.
pub const INTERDICTOR_PARTY: PartyId = admitted_party(1);

/// Default standoff distance (`18,000 m`) at which an interdicting picket opens the bubble.
pub const INTERDICTION_STANDOFF_M: i64 = 18_000;

/// Maximum number of snapshots retained per active bubble (`SF.R10`, `35b` §5 #9).
pub const MAX_BUBBLE_SNAPSHOTS: usize = 6;

/// Maximum number of input tape entries retained per active bubble before rolling the checkpoint (`SF.R10`, `35b` §5 #9).
pub const MAX_BUBBLE_TAPE_ENTRIES: usize = 64;

/// Maximum number of completed bubble summary records retained in [`SpaceBubbleBook::completed`] (`SF.R10`, `35b` §5 #9).
pub const MAX_COMPLETED_BUBBLES: usize = 8;

/// Maximum 1:1 wall-clock catch-up step (`30,000 ms`, `30 s`) applied on a single late tick fire (`SF.R17`).
pub const MAX_BUBBLE_TICK_STEP_MS: u32 = 30_000;

/// Maximum wall-clock duration (`60 s`) a carrier may hold a resolved bubble open for derelict salvage (`SF.R17`).
pub const SALVAGE_HOLD_TIMEOUT_WALL_S: i64 = 60;

const fn default_carrier_party() -> PartyId {
    CARRIER_PARTY
}

/// Per-carrier entry inside a domain-keyed [`ActiveInterdictionBubble`] (`SF.R12`, `SF.R18`, `D-35-5`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BubbleCarrierEntry {
    /// Carrier hull root object id.
    pub hull: ObjId,
    /// Carrier ship id inside [`ActiveInterdictionBubble::bubble`].
    pub ship_id: String,
    /// Arena party identifier for this carrier (`SF.R18`, `Q-36-48`).
    #[serde(default = "default_carrier_party", with = "super::bubble::party_wire")]
    pub party: PartyId,
    /// 1-based leg cursor at which this carrier was promoted.
    pub leg_cursor: u32,
    /// Departure IC timestamp of the [`CarrierJourney`] when promoted.
    pub departed_at_ic_s: i64,
    /// Last `propellant_kg` synced into [`HullPropellantLedger`] (`SF.R11`).
    pub last_synced_propellant_kg: u32,
    /// Whether this carrier is actively executing a `+helm/approach` burn toward a derelict (`SF.R9`).
    #[serde(default)]
    pub approaching_derelict: bool,
    /// Whether this carrier is holding the bubble open while docking with / towing a derelict (`SF.R9`).
    #[serde(default)]
    pub holding_for_salvage: bool,
    /// Wall-clock deadline (`World::now()`) after which `holding_for_salvage` expires (`SF.R17`).
    #[serde(default)]
    pub holding_for_salvage_until_wall_s: Option<i64>,
}

/// Active interdiction bubble bound to a promoted waypoint [`DomainId`] (`SF.R12`, `D-35-5`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveInterdictionBubble {
    /// Primary carrier hull root object id.
    pub hull: ObjId,
    /// Hostile interdictor hull object id.
    pub interdictor_hull: ObjId,
    /// Promoted waypoint domain id.
    pub domain: DomainId,
    /// Primary carrier ship id inside [`Self::bubble`].
    pub carrier_ship_id: String,
    /// Hostile interdictor ship id inside [`Self::bubble`].
    pub interdictor_ship_id: String,
    /// 1-based leg cursor at which the primary carrier was promoted.
    pub leg_cursor: u32,
    /// W-clock IC timestamp (seconds) when the bubble was opened.
    pub opened_ic_s: i64,
    /// Number of 1:1 wall-clock steps executed since promotion.
    pub steps: u32,
    /// Whether the encounter has been explicitly marked cleared (e.g. picket waved off).
    pub cleared: bool,
    /// Live deterministic space bubble.
    pub bubble: LocalSpaceBubble,
    /// Recorded input tape for deterministic replay (`§6.3`).
    pub tape: BubbleInputTape,
    /// Periodic snapshots captured at bubble creation and after each tick step (`§6.3`).
    pub snapshots: Vec<BubbleSnapshot>,
    /// Multi-party Kirkwood Customs Picket state when this bubble is a Kirkwood picket encounter (`SF.R6`, `D-35-11`).
    pub picket: Option<super::picket::KirkwoodPicketState>,
    /// Whether the carrier is actively executing a `+helm/approach` burn toward a disabled derelict inside this bubble (`SF.R9`).
    #[serde(default)]
    pub approaching_derelict: bool,
    /// Whether the bubble is held open while the carrier docks with / attaches a tow line to a disabled derelict (`SF.R9`).
    #[serde(default)]
    pub holding_for_salvage: bool,
    /// Engine wall-clock timestamp (seconds, `World::now()`) of the last tick fire (`SF.R10`, `D-35-3`).
    #[serde(default)]
    pub last_stepped_wall_s: i64,
    /// Last carrier `propellant_kg` synced into [`HullPropellantLedger`] (`SF.R11`, `D-35-6`).
    #[serde(default)]
    pub last_synced_propellant_kg: u32,
    /// All promoted carrier hulls participating in this domain bubble (`SF.R12`, `D-35-5`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub carriers: BTreeMap<ObjId, BubbleCarrierEntry>,
    /// Mapping from ship id in [`LocalSpaceBubble::ships`] to real world [`ObjId`] (`SF.R12`).
    #[serde(default)]
    pub ship_hulls: BTreeMap<String, ObjId>,
    /// Persistent Rhai memory bytes for a non-picket interdictor (`INTERDICTOR_PATROL_RHAI`, `SF.R12`).
    #[serde(default)]
    pub interdictor_memory: Vec<u8>,
    /// Think sequence counter for a non-picket interdictor (`SF.R12`).
    #[serde(default)]
    pub interdictor_think_seq: u64,
    /// Sealed mind attached to the non-picket interdictor (`SF.R14`, `D-35-17`).
    #[serde(default, with = "super::picket::compact_mind_seal")]
    pub interdictor_seal: Option<crate::arena::mind::MindSeal>,
}

impl ActiveInterdictionBubble {
    /// Return whether `hull` is a promoted carrier participating in this domain bubble (`SF.R12`).
    #[must_use]
    pub fn contains_carrier(&self, hull: ObjId) -> bool {
        self.hull == hull || self.carriers.contains_key(&hull)
    }

    /// Return the ship id in [`Self::bubble`] for carrier `hull` (`SF.R12`).
    #[must_use]
    pub fn carrier_ship_id_for(&self, hull: ObjId) -> &str {
        if let Some(entry) = self.carriers.get(&hull) {
            entry.ship_id.as_str()
        } else {
            self.carrier_ship_id.as_str()
        }
    }

    /// Return the promoted leg cursor for carrier `hull` (`SF.R12`).
    #[must_use]
    pub fn carrier_leg_cursor_for(&self, hull: ObjId) -> u32 {
        self.carriers
            .get(&hull)
            .map_or(self.leg_cursor, |e| e.leg_cursor)
    }

    /// Return the arena [`PartyId`] for carrier `hull` (`SF.R18`).
    #[must_use]
    pub fn carrier_party_for(&self, hull: ObjId) -> PartyId {
        if let Some(entry) = self.carriers.get(&hull) {
            return entry.party;
        }
        self.bubble
            .ships
            .get(self.carrier_ship_id_for(hull))
            .map_or(CARRIER_PARTY, |s| s.party)
    }

    /// Allocate the next available arena [`PartyId`] for a secondary carrier joining this bubble (`SF.R18`).
    #[must_use]
    pub fn next_available_carrier_party(&self) -> PartyId {
        let used: std::collections::BTreeSet<PartyId> =
            self.bubble.ships.values().map(|s| s.party).collect();
        for idx in 3..=25usize {
            if let Some(p) = PartyId::nth(idx)
                && !used.contains(&p)
            {
                return p;
            }
        }
        admitted_party(3)
    }

    /// Return all carrier `(hull, ship_id, party)` tuples for picket stepping (`SF.R18`).
    #[must_use]
    pub fn carrier_picket_entries(&self) -> Vec<(ObjId, String, PartyId)> {
        if self.carriers.is_empty() {
            vec![(
                self.hull,
                self.carrier_ship_id.clone(),
                self.carrier_party_for(self.hull),
            )]
        } else {
            self.carriers
                .values()
                .map(|c| (c.hull, c.ship_id.clone(), c.party))
                .collect()
        }
    }

    /// Resolve the real world [`ObjId`] for `ship_id` inside this bubble (`SF.R12`).
    #[must_use]
    pub fn hull_for_ship_id(&self, ship_id: &str) -> Option<ObjId> {
        if let Some(&h) = self.ship_hulls.get(ship_id) {
            return Some(h);
        }
        if ship_id == self.carrier_ship_id {
            return Some(self.hull);
        }
        if ship_id == self.interdictor_ship_id {
            return Some(self.interdictor_hull);
        }
        if let Some(picket) = &self.picket {
            if ship_id == picket.corvette_ship_id {
                return Some(picket.corvette_hull);
            }
            if ship_id == picket.raider_ship_id {
                return Some(picket.raider_hull);
            }
        }
        for entry in self.carriers.values() {
            if entry.ship_id == ship_id {
                return Some(entry.hull);
            }
        }
        ship_id
            .strip_prefix("hull.")
            .or_else(|| ship_id.strip_prefix("ship."))
            .and_then(|s| s.parse::<u32>().ok())
            .map(ObjId)
    }

    /// Return all carrier `(hull, ship_id, leg_cursor, departed_at_ic_s)` entries in this bubble (`SF.R12`).
    #[must_use]
    pub fn all_carrier_hulls(&self) -> Vec<(ObjId, String, u32, i64)> {
        if self.carriers.is_empty() {
            vec![(
                self.hull,
                self.carrier_ship_id.clone(),
                self.leg_cursor,
                self.opened_ic_s,
            )]
        } else {
            self.carriers
                .values()
                .map(|c| (c.hull, c.ship_id.clone(), c.leg_cursor, c.departed_at_ic_s))
                .collect()
        }
    }

    /// Set `approaching_derelict` for `hull` (`SF.R12`).
    pub fn set_approaching_derelict_for(&mut self, hull: ObjId, val: bool) {
        if let Some(c) = self.carriers.get_mut(&hull) {
            c.approaching_derelict = val;
        }
        if hull == self.hull || self.carriers.is_empty() {
            self.approaching_derelict = val;
        } else {
            self.approaching_derelict = self.carriers.values().any(|c| c.approaching_derelict);
        }
    }

    /// Set `holding_for_salvage` for `hull` (`SF.R12`).
    pub fn set_holding_for_salvage_for(&mut self, hull: ObjId, val: bool) {
        self.set_holding_for_salvage_for_with_deadline(hull, val, None);
    }

    /// Set `holding_for_salvage` and its optional wall-clock expiry deadline for `hull` (`SF.R17`).
    pub fn set_holding_for_salvage_for_with_deadline(
        &mut self,
        hull: ObjId,
        val: bool,
        until_wall_s: Option<i64>,
    ) {
        if let Some(c) = self.carriers.get_mut(&hull) {
            c.holding_for_salvage = val;
            c.holding_for_salvage_until_wall_s = if val { until_wall_s } else { None };
        }
        if hull == self.hull || self.carriers.is_empty() {
            self.holding_for_salvage = val;
        } else {
            self.holding_for_salvage = self.carriers.values().any(|c| c.holding_for_salvage);
        }
    }

    /// Return whether combat and customs in this bubble have cleared, independent of per-carrier
    /// derelict salvage holds (`SF.R17`, `SF.R18`).
    #[must_use]
    pub fn is_combat_clear(&self) -> bool {
        if self.cleared {
            return true;
        }
        if !self.bubble.torpedoes.is_empty() || !self.bubble.railgun_slugs.is_empty() {
            return false;
        }
        if let Some(picket) = &self.picket {
            let corvette_operational = self
                .bubble
                .ships
                .get(&picket.corvette_ship_id)
                .is_some_and(|s| s.status == HullStatus::Operational);
            if corvette_operational && !picket.is_customs_cleared() {
                return false;
            }
        }
        let carrier_parties: Vec<PartyId> = if self.carriers.is_empty() {
            vec![CARRIER_PARTY]
        } else {
            self.carriers.values().map(|c| c.party).collect()
        };
        !self.bubble.ships.values().any(|ship| {
            ship.status == HullStatus::Operational
                && carrier_parties
                    .iter()
                    .any(|&cp| self.bubble.party_relation(cp, ship.party) == Relation::Hostile)
        })
    }

    /// Compute the adaptive 1:1 wall-clock cadence period (`500 ms`, `1,000 ms`, or `5,000 ms`)
    /// from the bubble's live ordnance and hostile posture (`Q-35-4(1)`, `SF.R10`, `SF.R18`).
    #[must_use]
    pub fn adaptive_step_wall_ms(&self) -> u32 {
        if !self.bubble.torpedoes.is_empty() || !self.bubble.railgun_slugs.is_empty() {
            return ORDNANCE_STEP_WALL_MS;
        }
        let carrier_parties: Vec<PartyId> = if self.carriers.is_empty() {
            vec![CARRIER_PARTY]
        } else {
            self.carriers.values().map(|c| c.party).collect()
        };
        let has_operational_hostile = self.bubble.ships.values().any(|ship| {
            ship.status == HullStatus::Operational
                && carrier_parties
                    .iter()
                    .any(|&cp| self.bubble.party_relation(cp, ship.party) == Relation::Hostile)
        });
        if has_operational_hostile || self.picket.is_some() || self.approaching_derelict {
            COMBAT_STEP_WALL_MS
        } else {
            CRUISE_STEP_WALL_MS
        }
    }

    /// Push a snapshot while capping [`Self::snapshots`] at [`MAX_BUBBLE_SNAPSHOTS`] (`SF.R10`, `35b` §5 #9).
    pub fn push_bounded_snapshot(&mut self) {
        if self.snapshots.len() >= MAX_BUBBLE_SNAPSHOTS {
            let remove_idx = if self.snapshots.len() > 1 { 1 } else { 0 };
            self.snapshots.remove(remove_idx);
        }
        self.snapshots.push(self.bubble.snapshot());
    }

    /// Bound [`Self::tape`] so a long heave-to or `+helm/roll` loop cannot grow the persisted
    /// document without bound (`SF.R10`, `35b` §5 #9).
    pub fn compact_tape_if_needed(&mut self) {
        if self.tape.entries.len() > MAX_BUBBLE_TAPE_ENTRIES {
            self.tape = BubbleInputTape::new(&self.bubble);
        }
    }

    /// Return whether the encounter is clear for carrier `hull` to resume its [`CarrierJourney`] (`SF.R18`).
    #[must_use]
    pub fn is_carrier_clear_to_resume(&self, hull: ObjId) -> bool {
        let ship_id = self.carrier_ship_id_for(hull);
        let carrier_party = self.carrier_party_for(hull);
        let carrier_operational = self
            .bubble
            .ships
            .get(ship_id)
            .is_some_and(|ship| ship.status == HullStatus::Operational);
        if !carrier_operational {
            return false;
        }
        if let Some(entry) = self.carriers.get(&hull) {
            if entry.approaching_derelict || entry.holding_for_salvage {
                return false;
            }
        } else if self.approaching_derelict || self.holding_for_salvage {
            return false;
        }
        if self.cleared {
            return true;
        }
        if !self.bubble.torpedoes.is_empty() || !self.bubble.railgun_slugs.is_empty() {
            return false;
        }
        if let Some(picket) = &self.picket {
            let corvette_operational = self
                .bubble
                .ships
                .get(&picket.corvette_ship_id)
                .is_some_and(|s| s.status == HullStatus::Operational);
            if corvette_operational && !picket.is_carrier_customs_cleared(hull) {
                return false;
            }
        }
        !self.bubble.ships.values().any(|ship| {
            ship.status == HullStatus::Operational
                && self.bubble.party_relation(carrier_party, ship.party) == Relation::Hostile
        })
    }

    /// Return whether the encounter is clear for all carriers in the bubble to resume their [`CarrierJourney`] (`SF.R18`).
    #[must_use]
    pub fn is_clear_to_resume(&self) -> bool {
        if self.carriers.is_empty() {
            self.is_carrier_clear_to_resume(self.hull)
        } else {
            self.carriers
                .keys()
                .all(|&ch| self.is_carrier_clear_to_resume(ch))
        }
    }

    /// Return whether any carrier in this bubble has been disabled into an open-epoch derelict
    /// (`D-35-12`, `SF.R18`) and all in-flight ordnance has resolved.
    #[must_use]
    pub fn is_carrier_disabled_and_settled(&self) -> bool {
        if !self.bubble.torpedoes.is_empty() || !self.bubble.railgun_slugs.is_empty() {
            return false;
        }
        if self.carriers.is_empty() {
            self.bubble
                .ships
                .get(&self.carrier_ship_id)
                .is_some_and(|ship| matches!(ship.status, HullStatus::DisabledDerelict { .. }))
        } else {
            self.carriers.values().any(|c| {
                self.bubble
                    .ships
                    .get(&c.ship_id)
                    .is_some_and(|ship| matches!(ship.status, HullStatus::DisabledDerelict { .. }))
            })
        }
    }
}

/// Summary record retained in [`SpaceBubbleBook::completed`] when an interdiction bubble closes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedInterdictionBubble {
    /// Carrier hull root object id.
    pub hull: ObjId,
    /// Promoted waypoint domain id.
    pub domain: DomainId,
    /// Bubble identifier.
    pub bubble_id: BubbleId,
    /// W-clock IC timestamp (seconds) when the bubble was opened.
    pub opened_ic_s: i64,
    /// W-clock IC timestamp (seconds) when the bubble closed.
    pub closed_ic_s: i64,
    /// Total 1:1 wall-clock milliseconds simulated inside the bubble.
    pub wall_elapsed_ms: u64,
    /// Number of 1:1 wall-clock tick steps executed while the bubble was active.
    pub steps: u32,
    /// Complete input tape recorded across the bubble's lifetime.
    pub tape: BubbleInputTape,
    /// Snapshots captured across the bubble's lifetime.
    pub snapshots: Vec<BubbleSnapshot>,
    /// Final Kirkwood Customs Picket state when the bubble closed (`SF.R6`, `D-35-11`).
    pub picket: Option<super::picket::KirkwoodPicketState>,
}

impl CompletedInterdictionBubble {
    /// Construct a bounded summary record when `active` closes, pruning historical snapshots and
    /// tape entries so closed bubbles do not bloat `tacsim:space:state` (`SF.R10`, `35b` §5 #9).
    #[must_use]
    pub fn from_closed(active: &ActiveInterdictionBubble, closed_ic_s: i64) -> Self {
        Self {
            hull: active.hull,
            domain: active.domain,
            bubble_id: active.bubble.id.clone(),
            opened_ic_s: active.opened_ic_s,
            closed_ic_s,
            wall_elapsed_ms: active.bubble.wall_elapsed_ms,
            steps: active.steps,
            tape: BubbleInputTape::new(&active.bubble),
            snapshots: Vec::new(),
            picket: None,
        }
    }
}

/// Open-epoch derelict hull record persisted when a ship's drive or hull is disabled
/// in a space bubble (`SF.R2`, `D-35-12`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceDerelictRecord {
    /// Disabled hull object id.
    pub hull: ObjId,
    /// Waypoint domain id where the hull was disabled.
    pub domain: DomainId,
    /// W-clock IC timestamp (seconds) when the hull was disabled.
    pub disabled_at_ic_s: i64,
    /// Whether the hull's doc 23 epoch remains open (`D-35-12`, `FO-12`).
    pub epoch_open: bool,
    /// Remaining life-support window in real seconds (`172,800 s` at disablement).
    pub life_support_remaining_s: u32,
    /// Remaining 48 real-hour salvage contest window in real seconds (`172,800 s` at disablement).
    pub salvage_contest_window_s: u32,
    /// Post-combat `TacticalUnit` assessment (`Immobile`, `Disabled`, `Wreck`), if bound.
    pub assessment: Option<FieldUnitAssessment>,
    /// Active 48 real-hour Registry salvage claim status in [`SalvageClaimBook`].
    pub salvage_title_status: Option<CustodyTitleStatus>,
}

/// Guidance attitude mode for a ship in Tier-1 free flight (`SF.R4`, `35a` §5 #5).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreeFlightGuidanceMode {
    /// Manual attitude (`+helm/heading`).
    #[default]
    Manual,
    /// Nose aligned with heliocentric velocity vector ($+\vec{v}$).
    Prograde,
    /// Nose aligned opposite heliocentric velocity vector ($-\vec{v}$).
    Retrograde,
    /// Nose aligned toward the Sun ($-\vec{r}$).
    RadialIn,
    /// Nose aligned away from the Sun ($+\vec{r}$).
    RadialOut,
    /// Nose aligned along orbital angular momentum ($+\vec{r} \times \vec{v}$).
    Normal,
    /// Nose aligned opposite orbital angular momentum ($-\vec{r} \times \vec{v}$).
    AntiNormal,
    /// Intercept target body; flip/brake test uses target-relative velocity ($\vec{v}_{\text{rel}}$) (`35a` §5 #5).
    Intercept,
    /// Match velocity with target body using target-relative velocity ($\vec{v}_{\text{rel}}$) (`35a` §5 #5).
    MatchVelocity,
}

impl FreeFlightGuidanceMode {
    /// Parse a guidance mode token (`manual`, `prograde`, `retrograde`, `radial_in`, `radial_out`,
    /// `normal`, `anti_normal`, `intercept`, `match_velocity` / `match`).
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "manual" | "off" => Some(Self::Manual),
            "prograde" | "pro" => Some(Self::Prograde),
            "retrograde" | "retro" => Some(Self::Retrograde),
            "radial_in" | "radialin" => Some(Self::RadialIn),
            "radial_out" | "radialout" => Some(Self::RadialOut),
            "normal" | "norm" => Some(Self::Normal),
            "anti_normal" | "antinormal" => Some(Self::AntiNormal),
            "intercept" | "target_prograde" => Some(Self::Intercept),
            "match_velocity" | "match" | "target_retrograde" => Some(Self::MatchVelocity),
            _ => None,
        }
    }

    /// Canonical code string for status display.
    #[must_use]
    pub const fn code_str(self) -> &'static str {
        match self {
            Self::Manual => "MANUAL",
            Self::Prograde => "PROGRADE",
            Self::Retrograde => "RETROGRADE",
            Self::RadialIn => "RADIAL_IN",
            Self::RadialOut => "RADIAL_OUT",
            Self::Normal => "NORMAL",
            Self::AntiNormal => "ANTI_NORMAL",
            Self::Intercept => "INTERCEPT",
            Self::MatchVelocity => "MATCH_VEL",
        }
    }
}

/// Solar gravitational parameter $\mu_{\odot} = 132{,}712{,}440{,}018\text{ km}^3/\text{s}^2$.
pub const SOLAR_MU_KM3_S2: f64 = 132_712_440_018.0;

/// Tier-1 interplanetary free-flight state for a carrier hull that has decoupled from its
/// corridor route (`+helm/decouple`, `SF.R4`, `D-35-5`, `D-35-6`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceFreeFlightRecord {
    /// Carrier hull root [`ObjId`].
    pub hull: ObjId,
    /// Origin place id of the leg the hull decoupled from.
    pub origin_place: mush_core::CanonicalId,
    /// Target place id of the leg (or autopilot target) the hull is flying toward.
    pub target_place: mush_core::CanonicalId,
    /// Heliocentric position in kilometres (`[x_km, y_km, z_km]`).
    pub pos_km: [i64; 3],
    /// Heliocentric velocity in metres per second (`[vx_mps, vy_mps, vz_mps]`).
    pub vel_mps: [i64; 3],
    /// Commanded acceleration vector in millimetres per second squared (`[ax_mmps2, ay_mmps2, az_mmps2]`).
    pub thrust_mmps2: [i64; 3],
    /// Remaining timed-burn duration in IC seconds; when this reaches `0`, thrust resets to `[0, 0, 0]`
    /// so a timed burn never latches (`35a` §5 #4).
    pub burn_remaining_ic_s: Option<u32>,
    /// Active guidance mode (`35a` §5 #5).
    pub guidance: FreeFlightGuidanceMode,
    /// Whether `Intercept` / `MatchVelocity` has flipped to braking against the target-relative velocity.
    pub braking_flipped: bool,
    /// W-clock IC timestamp (seconds) when the hull decoupled.
    pub decoupled_at_ic_s: i64,
    /// W-clock IC timestamp (seconds) of the last integration step.
    pub last_ic_s: i64,
    /// Optional orbital cluster arrival deadline (IC seconds) when flying to a cluster via `+helm/autopilot <cluster_id>` (`SF.R9`, `D-35-5`).
    #[serde(default)]
    pub cluster_arrival_ic_s: Option<i64>,
    /// Prepaid reaction mass (tonnes) for the unflown portion of the corridor passage the hull decoupled from (`SF.R11`, `D-35-6`).
    #[serde(default)]
    pub prepaid_unflown_t: u64,
}

impl SpaceFreeFlightRecord {
    /// Advance the free-flight state by `dt_ic_s` IC seconds under solar gravity
    /// ($\vec{a}_{\odot} = -\mu_{\odot} \vec{r} / |\vec{r}|^3$) and any active timed burn
    /// (`35a` §5 #4, #5).
    ///
    /// - Timed burns (`burn_remaining_ic_s = Some(rem)`) apply thrust for `min(dt_ic_s, rem)`
    ///   seconds and cut off (`thrust_mmps2 = [0, 0, 0]`, `burn_remaining_ic_s = None`) once
    ///   `rem` expires so thrust never latches.
    /// - `Intercept` and `MatchVelocity` evaluate their flip/braking condition against
    ///   `target_vel_mps` ($\vec{v}_{\text{rel}} = \vec{v}_{\text{ship}} - \vec{v}_{\text{target}}$)
    ///   rather than heliocentric speed, so a ship co-moving with its target body at zero
    ///   relative speed does not flip (`35a` §5 #5).
    pub fn step_ic_s(&mut self, dt_ic_s: i64, target_vel_mps: Option<[i64; 3]>) {
        if dt_ic_s <= 0 {
            return;
        }
        // Target-relative flip check (`35a` §5 #5): only flip when relative speed to the target
        // exceeds 5 m/s (`v_rel_sq > 25`).
        if matches!(
            self.guidance,
            FreeFlightGuidanceMode::Intercept | FreeFlightGuidanceMode::MatchVelocity
        ) {
            let tv = target_vel_mps.unwrap_or([0, 0, 0]);
            let rvx = i128::from(self.vel_mps[0]) - i128::from(tv[0]);
            let rvy = i128::from(self.vel_mps[1]) - i128::from(tv[1]);
            let rvz = i128::from(self.vel_mps[2]) - i128::from(tv[2]);
            let v_rel_sq = rvx * rvx + rvy * rvy + rvz * rvz;
            self.braking_flipped = v_rel_sq > 25;
        } else {
            self.braking_flipped = false;
        }

        // Timed burn cutoff (`35a` §5 #4, `35b` §3 #3): apply thrust only for the remaining burn duration,
        // and clear thrust whenever a timed burn finishes (including when `burn_s == dt_ic_s`).
        let had_timed_burn = self.burn_remaining_ic_s.is_some();
        let burn_s = match self.burn_remaining_ic_s {
            Some(rem) => {
                let rem_i64 = i64::from(rem);
                if dt_ic_s >= rem_i64 {
                    self.burn_remaining_ic_s = None;
                    rem_i64
                } else {
                    self.burn_remaining_ic_s = u32::try_from(rem_i64 - dt_ic_s).ok();
                    dt_ic_s
                }
            }
            None => dt_ic_s,
        };

        // Solar gravity acceleration in mm/s² (`a_sun = -mu_sun * r / |r|^3`).
        let rx = self.pos_km[0] as f64;
        let ry = self.pos_km[1] as f64;
        let rz = self.pos_km[2] as f64;
        let r_sq_km2 = rx * rx + ry * ry + rz * rz;
        let (gx_mmps2, gy_mmps2, gz_mmps2) = if r_sq_km2 > 1.0e6 {
            let r_km = libm::sqrt(r_sq_km2);
            let r_cubed = r_sq_km2 * r_km;
            // `SOLAR_MU_KM3_S2 / r_cubed` has units km/s² = 10^6 mm/s².
            let scale_mmps2_per_km = -SOLAR_MU_KM3_S2 * 1.0e6 / r_cubed;
            (
                (rx * scale_mmps2_per_km).round() as i64,
                (ry * scale_mmps2_per_km).round() as i64,
                (rz * scale_mmps2_per_km).round() as i64,
            )
        } else {
            (0, 0, 0)
        };

        for axis in 0..3 {
            let v0_mps = i128::from(self.vel_mps[axis]);
            let dv_burn_mps = (i128::from(self.thrust_mmps2[axis]) * i128::from(burn_s)) / 1_000;
            let dv_grav_mps =
                (i128::from([gx_mmps2, gy_mmps2, gz_mmps2][axis]) * i128::from(dt_ic_s)) / 1_000;
            let v1_mps = v0_mps + dv_burn_mps + dv_grav_mps;
            let avg_v_mps = (v0_mps + v1_mps) / 2;
            let dpos_km = (avg_v_mps * i128::from(dt_ic_s)) / 1_000;
            self.pos_km[axis] = (i128::from(self.pos_km[axis]) + dpos_km)
                .clamp(i128::from(i64::MIN / 2), i128::from(i64::MAX / 2))
                as i64;
            self.vel_mps[axis] =
                v1_mps.clamp(i128::from(i64::MIN / 2), i128::from(i64::MAX / 2)) as i64;
        }

        if had_timed_burn && self.burn_remaining_ic_s.is_none() {
            self.thrust_mmps2 = [0, 0, 0];
        }
        self.last_ic_s = self.last_ic_s.saturating_add(dt_ic_s);
    }
}

/// Live reaction-mass ledger for a hull in free flight or an interdiction bubble (`SF.R4`, `D-35-6`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HullPropellantLedger {
    /// Sub-tonne kilograms burned across maneuvers (`0..999`); every 1,000 kg crosses into a
    /// whole-tonne `tacsim_port_burn` debit against `stow.propellant`.
    pub fractional_burned_kg: u64,
    /// Cumulative kilograms burned in free-flight and tactical maneuvers.
    pub total_burned_kg: u64,
    /// Last observed vault `stow.propellant` pool quantity (tonnes) when `unlanded_burn_t` was updated.
    pub last_vault_tank_t: u64,
    /// Whole tonnes queued for `tacsim_port_burn` that have not yet been reflected in `last_vault_tank_t`.
    pub unlanded_burn_t: u64,
}

impl HullPropellantLedger {
    /// Compute effective live propellant in kilograms given the hull vault's current
    /// `stow.propellant` quantity in tonnes (`vault_tank_t`).
    #[must_use]
    pub fn effective_live_kg(&self, vault_tank_t: u64) -> u64 {
        let landed_t = self.last_vault_tank_t.saturating_sub(vault_tank_t);
        let unlanded_t = self.unlanded_burn_t.saturating_sub(landed_t);
        let whole_t_after_debits = vault_tank_t.saturating_sub(unlanded_t);
        if self.fractional_burned_kg == 0 {
            whole_t_after_debits.saturating_mul(1_000)
        } else {
            whole_t_after_debits
                .saturating_mul(1_000)
                .saturating_add(1_000_u64.saturating_sub(self.fractional_burned_kg))
        }
    }

    /// Record `burn_kg` of reaction mass spent against a hull whose vault currently holds
    /// `vault_tank_t` tonnes of `stow.propellant`. Returns the number of whole tonnes that
    /// must be debited from the integer-tonne vault via `tacsim_port_burn` so `vault_tank_t`
    /// always equals `floor(live_kg / 1_000)`.
    pub fn record_burn_kg(&mut self, vault_tank_t: u64, burn_kg: u64) -> u64 {
        if self.last_vault_tank_t > vault_tank_t {
            let landed_t = self.last_vault_tank_t - vault_tank_t;
            self.unlanded_burn_t = self.unlanded_burn_t.saturating_sub(landed_t);
        }
        self.last_vault_tank_t = vault_tank_t;
        if burn_kg == 0 {
            return 0;
        }
        self.total_burned_kg = self.total_burned_kg.saturating_add(burn_kg);
        let prev_ceil = self.fractional_burned_kg.div_ceil(1_000);
        let combined = self.fractional_burned_kg.saturating_add(burn_kg);
        let new_ceil = combined.div_ceil(1_000);
        let whole_tonnes = new_ceil.saturating_sub(prev_ceil);
        self.fractional_burned_kg = combined % 1_000;
        self.unlanded_burn_t = self.unlanded_burn_t.saturating_add(whole_tonnes);
        whole_tonnes
    }

    /// Record a whole-tonne passage burn (`burn_t` tonnes) queued for `tacsim_port_burn`.
    pub fn record_passage_burn_t(&mut self, vault_tank_t: u64, burn_t: u64) {
        if self.last_vault_tank_t > vault_tank_t {
            let landed_t = self.last_vault_tank_t - vault_tank_t;
            self.unlanded_burn_t = self.unlanded_burn_t.saturating_sub(landed_t);
        }
        self.last_vault_tank_t = vault_tank_t;
        self.total_burned_kg = self
            .total_burned_kg
            .saturating_add(burn_t.saturating_mul(1_000));
        self.unlanded_burn_t = self.unlanded_burn_t.saturating_add(burn_t);
    }
}

/// Linkdead grace window in real wall seconds (`2 minutes`) before queued return-plan activation (`SF.R13`, `D-35-7`).
pub const SPACE_LINKDEAD_GRACE_WALL_S: i64 = 120;

/// Linkdead grace-window and queued return-plan state for a carrier hull (`SF.R13`, `D-35-7`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceLinkdeadState {
    /// Carrier hull root [`ObjId`].
    pub hull: ObjId,
    /// Real wall epoch seconds (`w.now()`) when the crew went linkdead.
    pub grace_started_wall_s: i64,
    /// World IC epoch seconds when the crew went linkdead.
    pub grace_started_ic_s: i64,
    /// Whether active point-defense (`pdc_auto`) is held armed during the grace window (`D-35-7`).
    pub pdc_held: bool,
    /// Whether the linkdead grace window has expired into full disconnect (`D-35-7`).
    pub grace_expired: bool,
    /// Nearest open port queued for the return plan (`D-35-7`).
    pub queued_return_port: Option<String>,
}

/// Pending bubble input queued by a `+tac/*` or `+helm/*` switch for `BubbleInputProvider` (`SF.R21`, `35b` §3b).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingBubbleInput {
    /// Monotonic sequence number.
    pub seq: u64,
    /// Submitting actor [`ObjId`].
    pub actor: ObjId,
    /// Carrier hull [`ObjId`].
    pub hull: ObjId,
    /// Switch name (`"beam"`, `"lock"`, `"torp"`, `"railgun"`, `"pdc"`, `"heading"`, `"thrust"`, etc.).
    pub switch: String,
    /// World IC timestamp.
    pub at_ic: i64,
    /// Optional [`BubbleInputCommand`] staged by the command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<BubbleInputCommand>,
    /// Optional collector drone sortie excursions `(drone_obj, slot, site)` staged by `+mine/drones` (`SF.R22`, `D-35-13`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drone_excursions: Vec<(ObjId, u8, mush_core::CanonicalId)>,
}

/// World-state registry of active and completed space interdiction bubbles,
/// open-epoch derelicts, Tier-1 free-flight records, live propellant ledgers,
/// orbital cluster mining sessions, and 6-DOF docking sessions (`SF.R1`–`SF.R21`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceBubbleBook {
    /// Active interdiction bubbles keyed by waypoint [`DomainId`] (`SF.R12`, `D-35-5`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub by_domain: BTreeMap<DomainId, ActiveInterdictionBubble>,
    /// Carrier legs `(hull, leg_cursor)` that have already cleared an interdiction bubble (`SF.R12`, `SF.R18`).
    #[serde(default)]
    pub cleared_interdiction_legs: std::collections::BTreeSet<(ObjId, u32)>,
    /// Completed interdiction bubbles keyed by carrier hull root [`ObjId`].
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub completed: BTreeMap<ObjId, CompletedInterdictionBubble>,
    /// Open-epoch derelict hulls keyed by hull [`ObjId`] (`D-35-12`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub derelicts: BTreeMap<ObjId, SpaceDerelictRecord>,
    /// Tier-1 interplanetary free-flight records keyed by carrier hull root [`ObjId`] (`SF.R4`, `D-35-5`, `D-35-6`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub free_flights: BTreeMap<ObjId, SpaceFreeFlightRecord>,
    /// Live reaction-mass ledgers keyed by carrier hull root [`ObjId`] (`SF.R4`, `D-35-6`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub propellant_ledgers: BTreeMap<ObjId, HullPropellantLedger>,
    /// Server-authenticated in-flight reaction-mass burns queued for `tacsim_port_burn` (`SF.R4`, `D-35-6`).
    #[serde(default)]
    pub pending_space_burns: Vec<crate::port::burn::PendingSpaceBurn>,
    /// Monotonic sequence allocator for [`Self::pending_space_burns`].
    #[serde(default)]
    pub next_space_burn_seq: u32,
    /// Laid-up orbital cluster sites keyed by authored cluster [`mush_core::CanonicalId`] (`SF.R5`, `SF.R22`, `D-35-13`, `D-35-17`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub cluster_sites: BTreeMap<mush_core::CanonicalId, super::mining::ClusterSiteState>,
    /// Active orbital cluster mining sessions keyed by carrier hull [`ObjId`] (`SF.R5`, `D-35-13`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub cluster_sessions: BTreeMap<ObjId, super::mining::ActiveClusterMiningSession>,
    /// Pending licensed player claim stake/contest orders in orbital clusters (`SF.R5`, `D-35-13`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub cluster_tenure_orders: BTreeMap<ObjId, super::mining::ClusterTenureOrder>,
    /// Server-authenticated space mining ore draws queued for `tacsim_site_work` (`SF.R5`, `D-35-13`).
    #[serde(default)]
    pub pending_space_mine_draws: Vec<super::mining::PendingSpaceMineDraw>,
    /// Completed space mining ore draws (`SF.R5`, `D-35-13`).
    #[serde(default)]
    pub completed_space_mine_draws: Vec<super::mining::PendingSpaceMineDraw>,
    /// Monotonic sequence allocator for [`Self::pending_space_mine_draws`].
    #[serde(default)]
    pub next_space_mine_draw_seq: u64,
    /// Waypoint domains explicitly configured as 3-party Kirkwood Customs Pickets (`SF.R6`, `D-35-11`).
    #[serde(default)]
    pub kirkwood_picket_domains: std::collections::BTreeSet<DomainId>,
    /// Active 6-DOF docking sessions keyed by carrier hull [`ObjId`] (`SF.R7`, `Q-35-5`, `D-35-14`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub docking_sessions: BTreeMap<ObjId, super::docking::DockingSession>,
    /// Hard-docked derelict hulls keyed by tug carrier hull [`ObjId`] (`SF.R7`, `D-35-12`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub clamped_derelicts: BTreeMap<ObjId, ObjId>,
    /// Active salvage tow-line records keyed by tug carrier hull [`ObjId`] (`SF.R7`, `D-35-12`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub active_tows: BTreeMap<ObjId, super::docking::SalvageTowRecord>,
    /// Monotonic sequence allocator for [`Self::docking_sessions`].
    #[serde(default)]
    pub next_docking_session_seq: u64,
    /// Monotonic allocator for space salvage [`RecoveryOpId`]s.
    #[serde(default)]
    pub next_recovery_op_id: u64,
    /// Linkdead grace-window and queued return-plan state keyed by carrier hull [`ObjId`] (`SF.R13`, `D-35-7`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub linkdead_hulls: BTreeMap<ObjId, SpaceLinkdeadState>,
    /// Pinned tenure, title holder, and witnesses recorded at 6-DOF derelict collar clamp, keyed by derelict hull [`ObjId`] (`SF.R19`, `Q-36-46`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub derelict_pins: BTreeMap<ObjId, crate::surface::claim::TenurePin>,
    /// Live salvage custody rows for towed space derelicts, keyed by derelict hull [`ObjId`] (`SF.R19`, `Q-36-46`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub space_custody: BTreeMap<ObjId, crate::surface::salvage::SalvageCustody>,
    /// Derelict hulls whose Freeport Registry salvage recovery bounty has already been paid (`SF.R19`, `35b` §5 #5).
    #[serde(default)]
    pub paid_salvage_bounties: std::collections::BTreeSet<ObjId>,
    /// Origin blocks `(cluster_id, letter)` from which a carrier hull extracted `strategic_ore` (`SF.R23`, `35b` §5 #15).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub mined_strategic_blocks:
        BTreeMap<ObjId, std::collections::BTreeSet<(mush_core::CanonicalId, char)>>,
    /// Queued radio hails awaiting one-way light-lag (`d / c`) delivery (`SF.R23`, `D-35-10`).
    #[serde(default)]
    pub pending_light_lag_hails: Vec<super::picket::PendingLightLagHail>,
    /// Bubble domains quarantined at boot due to an unreadable `tacsim:space:bubble:{domain}` document (`SF.R21`, `35c` §6.3).
    #[serde(default, skip)]
    pub quarantined_bubbles: std::collections::BTreeSet<DomainId>,
    /// Hulls quarantined at boot due to an unreadable per-hull space document (`SF.R21`, `35c` §6.3).
    #[serde(default, skip)]
    pub quarantined_hulls: std::collections::BTreeSet<ObjId>,
}

impl SpaceBubbleBook {
    /// Return a copy of `self` with the five per-entity maps (`by_domain`, `free_flights`,
    /// `derelicts`, `docking_sessions`, `propellant_ledgers`) cleared, for auxiliary store diffing (`SF.R21`).
    #[must_use]
    pub fn without_per_entity_maps(&self) -> Self {
        let mut aux = self.clone();
        aux.by_domain.clear();
        aux.free_flights.clear();
        aux.derelicts.clear();
        aux.docking_sessions.clear();
        aux.propellant_ledgers.clear();
        aux.quarantined_bubbles.clear();
        aux.quarantined_hulls.clear();
        aux
    }

    /// Borrow the linkdead state for `hull`, if any (`SF.R13`, `D-35-7`).
    #[must_use]
    pub fn linkdead_state(&self, hull: ObjId) -> Option<&SpaceLinkdeadState> {
        self.linkdead_hulls.get(&hull)
    }

    /// Borrow the active interdiction bubble for `hull`, if any (`SF.R12`).
    #[must_use]
    pub fn active_for_hull(&self, hull: ObjId) -> Option<&ActiveInterdictionBubble> {
        self.by_domain
            .values()
            .find(|active| active.contains_carrier(hull))
    }

    /// Mutably borrow the active interdiction bubble for `hull`, if any (`SF.R12`).
    pub fn active_for_hull_mut(&mut self, hull: ObjId) -> Option<&mut ActiveInterdictionBubble> {
        self.by_domain
            .values_mut()
            .find(|active| active.contains_carrier(hull))
    }

    /// Borrow the active interdiction bubble for `domain`, if any (`SF.R12`).
    #[must_use]
    pub fn active_for_domain(&self, domain: DomainId) -> Option<&ActiveInterdictionBubble> {
        self.by_domain.get(&domain)
    }

    /// Mutably borrow the active interdiction bubble for `domain`, if any (`SF.R12`).
    pub fn active_for_domain_mut(
        &mut self,
        domain: DomainId,
    ) -> Option<&mut ActiveInterdictionBubble> {
        self.by_domain.get_mut(&domain)
    }

    /// Remove and return the active interdiction bubble containing `hull`, or detach `hull` from
    /// a multi-carrier domain bubble (`SF.R12`, `SF.R18`).
    pub fn remove_carrier_from_active(&mut self, hull: ObjId) -> Option<ActiveInterdictionBubble> {
        let domain = self
            .by_domain
            .iter()
            .find_map(|(&dom, a)| a.contains_carrier(hull).then_some(dom))?;
        let mut active = self.by_domain.remove(&domain)?;
        if active.carriers.len() > 1 {
            let mut snapshot = active.clone();
            if let Some(removed) = active.carriers.remove(&hull) {
                snapshot.hull = hull;
                snapshot.carrier_ship_id = removed.ship_id.clone();
                snapshot.leg_cursor = removed.leg_cursor;
                active.bubble.ships.remove(&removed.ship_id);
                active.ship_hulls.remove(&removed.ship_id);
            }
            if let Some(picket) = active.picket.as_mut() {
                picket.carrier_inspection_status.remove(&hull);
                picket.carrier_inspection_outcome.remove(&hull);
            }
            if active.hull == hull
                && let Some(first) = active.carriers.values().next().cloned()
            {
                active.hull = first.hull;
                active.carrier_ship_id = first.ship_id;
                active.leg_cursor = first.leg_cursor;
            }
            self.by_domain.insert(domain, active);
            Some(snapshot)
        } else {
            Some(active)
        }
    }

    /// Borrow the most recent completed interdiction bubble for `hull`, if any.
    #[must_use]
    pub fn completed_for_hull(&self, hull: ObjId) -> Option<&CompletedInterdictionBubble> {
        self.completed.get(&hull)
    }

    /// Borrow the open-epoch derelict record for `hull`, if any (`D-35-12`).
    #[must_use]
    pub fn derelict_for_hull(&self, hull: ObjId) -> Option<&SpaceDerelictRecord> {
        self.derelicts.get(&hull)
    }

    /// Borrow the active Tier-1 free-flight record for `hull`, if any (`SF.R4`, `D-35-5`).
    #[must_use]
    pub fn free_flight_for_hull(&self, hull: ObjId) -> Option<&SpaceFreeFlightRecord> {
        self.free_flights.get(&hull)
    }

    /// Mutably borrow the active Tier-1 free-flight record for `hull`, if any (`SF.R4`, `D-35-5`).
    pub fn free_flight_for_hull_mut(&mut self, hull: ObjId) -> Option<&mut SpaceFreeFlightRecord> {
        self.free_flights.get_mut(&hull)
    }

    /// Borrow the live propellant ledger for `hull` (`SF.R4`, `D-35-6`).
    #[must_use]
    pub fn propellant_ledger(&self, hull: ObjId) -> Option<&HullPropellantLedger> {
        self.propellant_ledgers.get(&hull)
    }

    /// Mutably borrow or insert the live propellant ledger for `hull` (`SF.R4`, `D-35-6`).
    pub fn propellant_ledger_mut(&mut self, hull: ObjId) -> &mut HullPropellantLedger {
        self.propellant_ledgers.entry(hull).or_default()
    }

    /// Allocate the next monotonic sequence number for a [`crate::port::burn::PendingSpaceBurn`].
    pub fn alloc_space_burn_seq(&mut self) -> u32 {
        self.next_space_burn_seq = self.next_space_burn_seq.saturating_add(1);
        1_000_000_u32.saturating_add(self.next_space_burn_seq)
    }

    /// Return whether `hull` is currently stepped out of the live world tick for a 6-DOF
    /// [`super::docking::DockingSession`] (`Q-35-5`, `D-35-14`).
    #[must_use]
    pub fn is_hull_paused_for_docking(&self, hull: ObjId) -> bool {
        self.docking_sessions
            .get(&hull)
            .is_some_and(|s| s.paused_from_world_tick)
    }

    /// Allocate a unique docking session ID (`"dock.<hull>.<seq>"`).
    pub fn alloc_docking_session_id(&mut self, hull: ObjId) -> String {
        self.next_docking_session_seq = self.next_docking_session_seq.saturating_add(1);
        format!("dock.{}.{}", hull.0, self.next_docking_session_seq)
    }

    pub(crate) fn alloc_recovery_op_id(&mut self) -> Option<RecoveryOpId> {
        self.next_recovery_op_id = self.next_recovery_op_id.saturating_add(1);
        RecoveryOpId::new(self.next_recovery_op_id).ok()
    }
}

/// Apply an authoritative [`BubbleInputCommand`] to the active interdiction bubble for `hull`,
/// recording the command on its deterministic [`BubbleInputTape`], appending a snapshot, and
/// logging any resulting combat impacts to both hulls' black-box flight recorders in
/// [`SalvageClaimBook`] (`SF.R2`, `SF.R3`, `D-35-1`, `D-35-12`).
pub fn apply_bubble_command(
    w: &mut World,
    hull: ObjId,
    cmd: BubbleInputCommand,
) -> Vec<crate::space::bubble::SpaceCombatEvent> {
    let at_ic = w.world_ic_now().at;
    let Some(book) = w.module_state_mut::<SpaceBubbleBook>() else {
        return Vec::new();
    };
    let Some(active) = book.active_for_hull_mut(hull) else {
        return Vec::new();
    };
    let events = active.tape.record_and_apply(&mut active.bubble, cmd);
    active.compact_tape_if_needed();
    if events.is_empty() {
        return events;
    }
    let carrier_ship_id = active.carrier_ship_id_for(hull).to_owned();
    let carrier_state = active
        .bubble
        .ships
        .get(&carrier_ship_id)
        .map(|s| match s.status {
            HullStatus::Operational => OperationalState::Operational,
            HullStatus::DisabledDerelict { .. } => OperationalState::Disabled,
        })
        .unwrap_or(OperationalState::Operational);
    let interdictor_state = active
        .bubble
        .ships
        .get(&active.interdictor_ship_id)
        .map(|s| match s.status {
            HullStatus::Operational => OperationalState::Operational,
            HullStatus::DisabledDerelict { .. } => OperationalState::Disabled,
        })
        .unwrap_or(OperationalState::Operational);
    let interdictor_hull = active.interdictor_hull;
    let interdictor_ship_id = active.interdictor_ship_id.clone();

    if let Some(salvage_book) = w.module_state_mut::<SalvageClaimBook>() {
        let _ = record_space_combat_to_black_box(
            salvage_book,
            hull,
            &carrier_ship_id,
            Some(interdictor_hull),
            &events,
            carrier_state,
            at_ic,
        );
        let _ = record_space_combat_to_black_box(
            salvage_book,
            interdictor_hull,
            &interdictor_ship_id,
            Some(hull),
            &events,
            interdictor_state,
            at_ic,
        );
    }
    events
}

/// Mark the active interdiction bubble for `hull` as cleared (disabling any remaining
/// hostile interdictors and clearing in-flight ordnance) so the next [`SPACE_TICK`] fire
/// closes the bubble and resumes the carrier's [`CarrierJourney`].
pub fn clear_interdiction_bubble(w: &mut World, hull: ObjId) -> bool {
    let Some(book) = w.module_state_mut::<SpaceBubbleBook>() else {
        return false;
    };
    let Some(active) = book.active_for_hull_mut(hull) else {
        return false;
    };
    active.cleared = true;
    active.bubble.torpedoes.clear();
    active.bubble.railgun_slugs.clear();
    let carrier_parties: std::collections::BTreeSet<PartyId> = if active.carriers.is_empty() {
        std::collections::BTreeSet::from([CARRIER_PARTY])
    } else {
        active.carriers.values().map(|c| c.party).collect()
    };
    for ship in active.bubble.ships.values_mut() {
        if !carrier_parties.contains(&ship.party) && ship.status == HullStatus::Operational {
            ship.status = HullStatus::DisabledDerelict {
                epoch_open: true,
                life_support_remaining_s: DERELICT_WINDOW_S,
                salvage_contest_window_s: DERELICT_WINDOW_S,
            };
            ship.thrust_mps2 = [0, 0, 0];
        }
    }
    true
}

/// Store document key under which auxiliary [`SpacePersistedState`] is persisted in the `"tacsim"` namespace (`SF.R8`, `SF.R21`, `D-35-12`).
pub const SPACE_STATE_KEY: &str = "tacsim:space:state";

/// Prefix shared by all space persistence keys (`tacsim:space:*`).
pub const SPACE_KEY_PREFIX: &str = "tacsim:space:";

/// Per-domain active bubble store key prefix (`tacsim:space:bubble:{domain}`, `SF.R21`, `35c` §6.2).
pub const SPACE_BUBBLE_PREFIX: &str = "tacsim:space:bubble:";

/// Per-hull free-flight store key prefix (`tacsim:space:flight:{hull}`, `SF.R21`, `35c` §6.2).
pub const SPACE_FLIGHT_PREFIX: &str = "tacsim:space:flight:";

/// Per-hull derelict store key prefix (`tacsim:space:derelict:{hull}`, `SF.R21`, `35c` §6.2).
pub const SPACE_DERELICT_PREFIX: &str = "tacsim:space:derelict:";

/// Per-hull docking session store key prefix (`tacsim:space:docking:{hull}`, `SF.R21`, `35c` §6.2).
pub const SPACE_DOCKING_PREFIX: &str = "tacsim:space:docking:";

/// Per-hull propellant ledger store key prefix (`tacsim:space:ledger:{hull}`, `SF.R21`, `35c` §6.2).
pub const SPACE_LEDGER_PREFIX: &str = "tacsim:space:ledger:";

/// Schema format version for [`SpacePersistedState`] and per-entity space documents.
pub const SPACE_STATE_FORMAT: u16 = 1;

/// Format the per-domain store key for an active bubble (`tacsim:space:bubble:{domain}`).
#[must_use]
pub fn space_bubble_key(domain: DomainId) -> String {
    format!("{SPACE_BUBBLE_PREFIX}{}", domain.0)
}

/// Format the per-hull store key for a free-flight record (`tacsim:space:flight:{hull}`).
#[must_use]
pub fn space_flight_key(hull: ObjId) -> String {
    format!("{SPACE_FLIGHT_PREFIX}{}", hull.0)
}

/// Format the per-hull store key for a derelict record (`tacsim:space:derelict:{hull}`).
#[must_use]
pub fn space_derelict_key(hull: ObjId) -> String {
    format!("{SPACE_DERELICT_PREFIX}{}", hull.0)
}

/// Format the per-hull store key for a docking session (`tacsim:space:docking:{hull}`).
#[must_use]
pub fn space_docking_key(hull: ObjId) -> String {
    format!("{SPACE_DOCKING_PREFIX}{}", hull.0)
}

/// Format the per-hull store key for a propellant ledger (`tacsim:space:ledger:{hull}`).
#[must_use]
pub fn space_ledger_key(hull: ObjId) -> String {
    format!("{SPACE_LEDGER_PREFIX}{}", hull.0)
}

/// Per-domain active bubble store document (`tacsim:space:bubble:{domain}`, `SF.R21`, `35c` §6.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedBubbleDoc {
    /// Schema format version ([`SPACE_STATE_FORMAT`]).
    pub format: u16,
    /// Domain identity matching the key suffix.
    pub domain: DomainId,
    /// Persisted [`ActiveInterdictionBubble`].
    pub bubble: ActiveInterdictionBubble,
}

/// Per-hull free-flight store document (`tacsim:space:flight:{hull}`, `SF.R21`, `35c` §6.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedFlightDoc {
    /// Schema format version ([`SPACE_STATE_FORMAT`]).
    pub format: u16,
    /// Hull identity matching the key suffix.
    pub hull: ObjId,
    /// Persisted [`SpaceFreeFlightRecord`].
    pub flight: SpaceFreeFlightRecord,
}

/// Per-hull derelict store document (`tacsim:space:derelict:{hull}`, `SF.R21`, `35c` §6.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedDerelictDoc {
    /// Schema format version ([`SPACE_STATE_FORMAT`]).
    pub format: u16,
    /// Hull identity matching the key suffix.
    pub hull: ObjId,
    /// Persisted [`SpaceDerelictRecord`].
    pub derelict: SpaceDerelictRecord,
}

/// Per-hull docking session store document (`tacsim:space:docking:{hull}`, `SF.R21`, `35c` §6.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedDockingDoc {
    /// Schema format version ([`SPACE_STATE_FORMAT`]).
    pub format: u16,
    /// Hull identity matching the key suffix.
    pub hull: ObjId,
    /// Persisted [`super::docking::DockingSession`].
    pub session: super::docking::DockingSession,
}

/// Per-hull propellant ledger store document (`tacsim:space:ledger:{hull}`, `SF.R21`, `35c` §6.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedLedgerDoc {
    /// Schema format version ([`SPACE_STATE_FORMAT`]).
    pub format: u16,
    /// Hull identity matching the key suffix.
    pub hull: ObjId,
    /// Persisted [`HullPropellantLedger`].
    pub ledger: HullPropellantLedger,
}

/// Durable store snapshot of [`SpaceBubbleBook`], [`SalvageClaimBook`], and `mush-sim`
/// [`mush_sim::tick::StandingOrderBook`] (`SF.R8`, `SF.R21`, `D-35-12`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpacePersistedState {
    /// Schema format version ([`SPACE_STATE_FORMAT`]).
    pub format: u16,
    /// Persisted [`SpaceBubbleBook`] state.
    pub book: SpaceBubbleBook,
    /// Persisted [`SalvageClaimBook`] state (48-hour salvage claims, recovery records, black-box tapes).
    pub salvage: SalvageClaimBook,
    /// Persisted `mush-sim` [`mush_sim::tick::StandingOrderBook`] (`SF.R21`, `D-35-9`).
    #[serde(default)]
    pub standing_orders: mush_sim::tick::StandingOrderBook,
}

impl SpacePersistedState {
    /// Construct an empty state at [`SPACE_STATE_FORMAT`].
    #[must_use]
    pub fn empty() -> Self {
        Self {
            format: SPACE_STATE_FORMAT,
            book: SpaceBubbleBook::default(),
            salvage: SalvageClaimBook::new(),
            standing_orders: mush_sim::tick::StandingOrderBook::default(),
        }
    }
}

/// Decode and validate one `tacsim:space:*` KV entry into `store` (`SF.R8`, `SF.R10`, `SF.R21`, `35c` §6.2).
///
/// Any malformed, format-mismatched, or key-vs-inner-identity-mismatched document quarantines
/// space state (`store.space_state = None; store.space_state_held = true`) so it fails closed
/// and is never overwritten.
pub fn load_space_kv_entry(store: &mut TacsimStore, key: &str, value: &serde_json::Value) {
    if store.space_state_held && store.space_state.is_none() {
        return;
    }
    store.space_state_held = true;
    let mut state = store
        .space_state
        .take()
        .unwrap_or_else(SpacePersistedState::empty);

    let ok = if key == SPACE_STATE_KEY {
        match serde_json::from_value::<SpacePersistedState>(value.clone()) {
            Ok(aux) if aux.format == SPACE_STATE_FORMAT => {
                let mut merged_book = aux.book;
                for (dom, b) in state.book.by_domain {
                    merged_book.by_domain.insert(dom, b);
                }
                for (h, f) in state.book.free_flights {
                    merged_book.free_flights.insert(h, f);
                }
                for (h, d) in state.book.derelicts {
                    merged_book.derelicts.insert(h, d);
                }
                for (h, s) in state.book.docking_sessions {
                    merged_book.docking_sessions.insert(h, s);
                }
                for (h, l) in state.book.propellant_ledgers {
                    merged_book.propellant_ledgers.insert(h, l);
                }
                merged_book.quarantined_bubbles = state.book.quarantined_bubbles;
                merged_book.quarantined_hulls = state.book.quarantined_hulls;
                state.book = merged_book;
                state.salvage = aux.salvage;
                state.standing_orders = aux.standing_orders;
                true
            }
            _ => false,
        }
    } else if let Some(dom_str) = key.strip_prefix(SPACE_BUBBLE_PREFIX) {
        match (
            dom_str.parse::<u64>().ok().map(DomainId),
            serde_json::from_value::<PersistedBubbleDoc>(value.clone()),
        ) {
            (Some(dom), Ok(doc))
                if doc.format == SPACE_STATE_FORMAT
                    && doc.domain == dom
                    && doc.bubble.domain == dom =>
            {
                state.book.by_domain.insert(dom, doc.bubble);
                true
            }
            (Some(dom), _) => {
                state.book.quarantined_bubbles.insert(dom);
                tracing::warn!(%key, domain = dom.0, "tacsim: space bubble document is unreadable; domain quarantined");
                true
            }
            _ => false,
        }
    } else if let Some(hull_str) = key.strip_prefix(SPACE_FLIGHT_PREFIX) {
        match (
            hull_str.parse::<u32>().ok().map(ObjId),
            serde_json::from_value::<PersistedFlightDoc>(value.clone()),
        ) {
            (Some(hull), Ok(doc))
                if doc.format == SPACE_STATE_FORMAT
                    && doc.hull == hull
                    && doc.flight.hull == hull =>
            {
                state.book.free_flights.insert(hull, doc.flight);
                true
            }
            (Some(hull), _) => {
                state.book.quarantined_hulls.insert(hull);
                tracing::warn!(%key, hull = hull.0, "tacsim: space flight document is unreadable; hull quarantined");
                true
            }
            _ => false,
        }
    } else if let Some(hull_str) = key.strip_prefix(SPACE_DERELICT_PREFIX) {
        match (
            hull_str.parse::<u32>().ok().map(ObjId),
            serde_json::from_value::<PersistedDerelictDoc>(value.clone()),
        ) {
            (Some(hull), Ok(doc))
                if doc.format == SPACE_STATE_FORMAT
                    && doc.hull == hull
                    && doc.derelict.hull == hull =>
            {
                state.book.derelicts.insert(hull, doc.derelict);
                true
            }
            (Some(hull), _) => {
                state.book.quarantined_hulls.insert(hull);
                tracing::warn!(%key, hull = hull.0, "tacsim: space derelict document is unreadable; hull quarantined");
                true
            }
            _ => false,
        }
    } else if let Some(hull_str) = key.strip_prefix(SPACE_DOCKING_PREFIX) {
        match (
            hull_str.parse::<u32>().ok().map(ObjId),
            serde_json::from_value::<PersistedDockingDoc>(value.clone()),
        ) {
            (Some(hull), Ok(doc))
                if doc.format == SPACE_STATE_FORMAT
                    && doc.hull == hull
                    && doc.session.hull_id == hull =>
            {
                state.book.docking_sessions.insert(hull, doc.session);
                true
            }
            (Some(hull), _) => {
                state.book.quarantined_hulls.insert(hull);
                tracing::warn!(%key, hull = hull.0, "tacsim: space docking document is unreadable; hull quarantined");
                true
            }
            _ => false,
        }
    } else if let Some(hull_str) = key.strip_prefix(SPACE_LEDGER_PREFIX) {
        match (
            hull_str.parse::<u32>().ok().map(ObjId),
            serde_json::from_value::<PersistedLedgerDoc>(value.clone()),
        ) {
            (Some(hull), Ok(doc)) if doc.format == SPACE_STATE_FORMAT && doc.hull == hull => {
                state.book.propellant_ledgers.insert(hull, doc.ledger);
                true
            }
            (Some(hull), _) => {
                state.book.quarantined_hulls.insert(hull);
                tracing::warn!(%key, hull = hull.0, "tacsim: space ledger document is unreadable; hull quarantined");
                true
            }
            _ => false,
        }
    } else {
        false
    };

    if ok {
        store.space_state = Some(state);
    } else {
        store.space_state = None;
        tracing::warn!(%key, "tacsim: space document is unreadable; held for staff repair");
    }
}

/// Persist the current [`SpaceBubbleBook`], [`SalvageClaimBook`], and `mush-sim`
/// [`mush_sim::tick::StandingOrderBook`] into [`TacsimStore`], emitting dirty-only per-entity
/// [`ModuleWrite::JsonSet`] / [`ModuleWrite::Del`] operations (`tacsim:space:bubble:{domain}`,
/// `tacsim:space:flight:{hull}`, `tacsim:space:derelict:{hull}`, `tacsim:space:docking:{hull}`,
/// `tacsim:space:ledger:{hull}`, and auxiliary `tacsim:space:state`) (`SF.R8`, `SF.R21`, `35c` §6.2).
pub fn persist_space_state(ctx: &mut Ctx<'_>) {
    if ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::is_space_state_held)
    {
        return;
    }
    let Some(book) = ctx.w.module_state::<SpaceBubbleBook>().cloned() else {
        return;
    };

    let salvage = ctx
        .w
        .module_state::<SalvageClaimBook>()
        .cloned()
        .unwrap_or_else(SalvageClaimBook::new);
    let standing_orders = mush_sim::tick::standing_orders_snapshot(ctx.w);
    let snapshot = SpacePersistedState {
        format: SPACE_STATE_FORMAT,
        book,
        salvage,
        standing_orders,
    };
    let prev = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::space_state)
        .cloned();
    let unchanged = match &prev {
        Some(p) => *p == snapshot,
        None => {
            snapshot.book == SpaceBubbleBook::default()
                && snapshot.salvage == SalvageClaimBook::new()
                && snapshot.standing_orders == mush_sim::tick::StandingOrderBook::default()
        }
    };
    if unchanged {
        return;
    }

    // 1. Per-domain active bubbles (`tacsim:space:bubble:{domain}`).
    for (&domain, active) in &snapshot.book.by_domain {
        if prev.as_ref().and_then(|p| p.book.by_domain.get(&domain)) != Some(active) {
            let doc = PersistedBubbleDoc {
                format: SPACE_STATE_FORMAT,
                domain,
                bubble: active.clone(),
            };
            if let Ok(value) = serde_json::to_value(&doc) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::JsonSet {
                        key: SmolStr::new(space_bubble_key(domain)),
                        value,
                    },
                );
            }
        }
    }
    if let Some(p) = &prev {
        for &domain in p.book.by_domain.keys() {
            if !snapshot.book.by_domain.contains_key(&domain) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::Del {
                        key: SmolStr::new(space_bubble_key(domain)),
                    },
                );
            }
        }
    }

    // 2. Per-hull free-flight records (`tacsim:space:flight:{hull}`).
    for (&hull, flight) in &snapshot.book.free_flights {
        if prev.as_ref().and_then(|p| p.book.free_flights.get(&hull)) != Some(flight) {
            let doc = PersistedFlightDoc {
                format: SPACE_STATE_FORMAT,
                hull,
                flight: flight.clone(),
            };
            if let Ok(value) = serde_json::to_value(&doc) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::JsonSet {
                        key: SmolStr::new(space_flight_key(hull)),
                        value,
                    },
                );
            }
        }
    }
    if let Some(p) = &prev {
        for &hull in p.book.free_flights.keys() {
            if !snapshot.book.free_flights.contains_key(&hull) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::Del {
                        key: SmolStr::new(space_flight_key(hull)),
                    },
                );
            }
        }
    }

    // 3. Per-hull derelict records (`tacsim:space:derelict:{hull}`).
    for (&hull, derelict) in &snapshot.book.derelicts {
        if prev.as_ref().and_then(|p| p.book.derelicts.get(&hull)) != Some(derelict) {
            let doc = PersistedDerelictDoc {
                format: SPACE_STATE_FORMAT,
                hull,
                derelict: derelict.clone(),
            };
            if let Ok(value) = serde_json::to_value(&doc) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::JsonSet {
                        key: SmolStr::new(space_derelict_key(hull)),
                        value,
                    },
                );
            }
        }
    }
    if let Some(p) = &prev {
        for &hull in p.book.derelicts.keys() {
            if !snapshot.book.derelicts.contains_key(&hull) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::Del {
                        key: SmolStr::new(space_derelict_key(hull)),
                    },
                );
            }
        }
    }

    // 4. Per-hull docking sessions (`tacsim:space:docking:{hull}`).
    for (&hull, session) in &snapshot.book.docking_sessions {
        if prev
            .as_ref()
            .and_then(|p| p.book.docking_sessions.get(&hull))
            != Some(session)
        {
            let doc = PersistedDockingDoc {
                format: SPACE_STATE_FORMAT,
                hull,
                session: session.clone(),
            };
            if let Ok(value) = serde_json::to_value(&doc) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::JsonSet {
                        key: SmolStr::new(space_docking_key(hull)),
                        value,
                    },
                );
            }
        }
    }
    if let Some(p) = &prev {
        for &hull in p.book.docking_sessions.keys() {
            if !snapshot.book.docking_sessions.contains_key(&hull) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::Del {
                        key: SmolStr::new(space_docking_key(hull)),
                    },
                );
            }
        }
    }

    // 5. Per-hull propellant ledgers (`tacsim:space:ledger:{hull}`).
    for (&hull, ledger) in &snapshot.book.propellant_ledgers {
        if prev
            .as_ref()
            .and_then(|p| p.book.propellant_ledgers.get(&hull))
            != Some(ledger)
        {
            let doc = PersistedLedgerDoc {
                format: SPACE_STATE_FORMAT,
                hull,
                ledger: ledger.clone(),
            };
            if let Ok(value) = serde_json::to_value(&doc) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::JsonSet {
                        key: SmolStr::new(space_ledger_key(hull)),
                        value,
                    },
                );
            }
        }
    }
    if let Some(p) = &prev {
        for &hull in p.book.propellant_ledgers.keys() {
            if !snapshot.book.propellant_ledgers.contains_key(&hull) {
                crate::store::push_op(
                    ctx.out,
                    ModuleWrite::Del {
                        key: SmolStr::new(space_ledger_key(hull)),
                    },
                );
            }
        }
    }

    // 6. Auxiliary space state (`tacsim:space:state`), written only when auxiliary fields change.
    let aux_now = SpacePersistedState {
        format: SPACE_STATE_FORMAT,
        book: snapshot.book.without_per_entity_maps(),
        salvage: snapshot.salvage.clone(),
        standing_orders: snapshot.standing_orders.clone(),
    };
    let aux_changed = match &prev {
        Some(p) => {
            let aux_prev = SpacePersistedState {
                format: p.format,
                book: p.book.without_per_entity_maps(),
                salvage: p.salvage.clone(),
                standing_orders: p.standing_orders.clone(),
            };
            aux_prev != aux_now
        }
        None => {
            aux_now.book != SpaceBubbleBook::default()
                || aux_now.salvage != SalvageClaimBook::new()
                || aux_now.standing_orders != mush_sim::tick::StandingOrderBook::default()
        }
    };
    if aux_changed {
        match serde_json::to_value(&aux_now) {
            Ok(value) => crate::store::push_op(
                ctx.out,
                ModuleWrite::JsonSet {
                    key: SmolStr::new_static(SPACE_STATE_KEY),
                    value,
                },
            ),
            Err(error) => {
                tracing::error!(%error, "tacsim: space state did not encode");
                return;
            }
        }
    }

    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.space_state = Some(snapshot);
        store.space_state_held = true;
    }
}

/// Hydrate [`SpaceBubbleBook`], [`SalvageClaimBook`], `mush-sim` [`mush_sim::tick::StandingOrderBook`],
/// and [`TacsimStore::space_state`] from the `"tacsim"` boot key-value documents during
/// `TacsimRuleset::on_world_loaded` (`SF.R8`, `SF.R21`, `D-35-12`).
pub fn boot_read(ctx: &mut Ctx<'_>, kv: &[(SmolStr, serde_json::Value)]) {
    let mut has_space_keys = false;
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.space_state = None;
        store.space_state_held = false;
        for (key, value) in kv {
            if key.as_str() == SPACE_STATE_KEY || key.as_str().starts_with(SPACE_KEY_PREFIX) {
                has_space_keys = true;
                load_space_kv_entry(store, key.as_str(), value);
            }
        }
    }
    if !has_space_keys {
        return;
    }
    let loaded = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|s| s.space_state.clone());
    let Some(mut state) = loaded else {
        return;
    };
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        crate::minds::content::ensure_space_mind_sources_installed(store);
    }
    for active in state.book.by_domain.values_mut() {
        if let Some(picket) = active.picket.as_mut() {
            picket.corvette_seal = super::picket::space_mind_seal_of(ctx.w, picket.corvette_hull)
                .or_else(|| {
                    super::picket::build_space_mind_seal(
                        picket.corvette_hull,
                        crate::arena::mind::SortieId::ic(picket.corvette_hull, 1),
                        "kirkwood_picket",
                        crate::minds::content::kirkwood_picket_digest().0,
                    )
                });
            picket.raider_seal = super::picket::space_mind_seal_of(ctx.w, picket.raider_hull)
                .or_else(|| {
                    super::picket::build_space_mind_seal(
                        picket.raider_hull,
                        crate::arena::mind::SortieId::ic(picket.raider_hull, 1),
                        "belt_raider",
                        crate::minds::content::belt_raider_digest().0,
                    )
                });
        } else {
            active.interdictor_seal =
                super::picket::space_mind_seal_of(ctx.w, active.interdictor_hull).or_else(|| {
                    super::picket::build_space_mind_seal(
                        active.interdictor_hull,
                        crate::arena::mind::SortieId::ic(active.interdictor_hull, 1),
                        "interdictor_patrol",
                        crate::minds::content::interdictor_patrol_digest().0,
                    )
                });
        }
    }
    for sess in state.book.cluster_sessions.values_mut() {
        for d in &mut sess.active_drones {
            d.mind_seal =
                super::picket::space_mind_seal_of(ctx.w, d.excursion_id.tv).or_else(|| {
                    super::picket::build_space_mind_seal(
                        d.excursion_id.tv,
                        crate::arena::mind::SortieId::ic(
                            d.excursion_id.tv,
                            d.excursion_id.launch_ic,
                        ),
                        "drone_mining",
                        crate::minds::content::drone_mining_digest().0,
                    )
                });
        }
    }
    mush_sim::tick::restore_standing_orders(ctx.w, state.standing_orders.clone());
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        *book = state.book.clone();
    }
    if let Some(salvage) = ctx.w.module_state_mut::<SalvageClaimBook>() {
        *salvage = state.salvage.clone();
    }
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.space_state = Some(state);
    }
}

/// Authoritative action id for bubble input tape staging (`SF.R21`, `35b` §3b).
pub const BUBBLE_INPUT_ACTION: &str = "tacsim_space_bubble_input";

/// Authoritative outcome id for [`BUBBLE_INPUT_ACTION`] (`SF.R21`, `35b` §3b).
pub const BUBBLE_INPUT_OUTCOME: &str = "tacsim_space_bubble_input_outcome";

/// Typed state family id for staged bubble input tapes (`SF.R21`, `35b` §3b).
pub const BUBBLE_TAPE_FAMILY: &str = "tacsim.space_bubble_tape";

/// Staged authoritative bubble-input ledger for a hull in `tacsim.space_bubble_tape` (`SF.R21`, `35b` §3b).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedBubbleTapeLedger {
    /// Carrier hull [`ObjId`].
    pub hull: ObjId,
    /// Last staged sequence number (`> 0`).
    pub last_seq: u64,
    /// Bounded tail of staged bubble commands (`<= MAX_BUBBLE_TAPE_ENTRIES`).
    #[serde(default)]
    pub recent: Vec<StagedBubbleTapeEntry>,
}

impl StagedBubbleTapeLedger {
    /// Validate record invariants for the `tacsim.space_bubble_tape` state family.
    pub fn check(&self) -> Result<(), String> {
        if self.hull.0 == 0 || self.last_seq == 0 {
            return Err(
                "a staged bubble tape ledger names a hull and a positive sequence".to_owned(),
            );
        }
        if self.recent.len() > MAX_BUBBLE_TAPE_ENTRIES {
            return Err("a staged bubble tape ledger exceeds MAX_BUBBLE_TAPE_ENTRIES".to_owned());
        }
        Ok(())
    }
}

/// One authoritative bubble input staged through `BubbleInputProvider` (`SF.R21`, `35b` §3b).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedBubbleTapeEntry {
    /// Monotonic sequence number on the hull's bubble input ledger.
    pub seq: u64,
    /// Submitting actor [`ObjId`].
    pub actor: ObjId,
    /// Carrier hull [`ObjId`].
    pub hull: ObjId,
    /// Switch name (`"beam"`, `"lock"`, `"torp"`, `"railgun"`, `"pdc"`, `"heading"`, `"thrust"`, etc.).
    pub switch: String,
    /// World IC timestamp when staged.
    pub at_ic: i64,
    /// Optional [`BubbleInputCommand`] staged by the command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<BubbleInputCommand>,
}

/// World-state handles for [`BUBBLE_INPUT_ACTION`] and [`BUBBLE_TAPE_FAMILY`] (`SF.R21`, `35b` §3b).
#[derive(Clone)]
pub struct BubbleInputHandles {
    /// Registered `tacsim_space_bubble_input` action reference.
    pub action: Option<mush_core::DefinitionRef>,
    /// Typed state family handle (`tacsim.space_bubble_tape`).
    pub tapes: mush_core::StateFamilyHandle<ObjId, StagedBubbleTapeLedger>,
    /// Server-authenticated bubble inputs queued by `+tac/*` and `+helm/*` for `BubbleInputProvider` (`SF.R21`, `35b` §3b).
    pub pending_bubble_inputs: Vec<PendingBubbleInput>,
    /// Monotonic sequence allocator for [`Self::pending_bubble_inputs`] (`SF.R21`, `35b` §3b).
    pub next_bubble_input_seq: u64,
    /// Transient tracking of actors with an un-drained `tacsim_port_burn` submission.
    pub pending_burn_submit_actors: std::collections::BTreeSet<ObjId>,
}

/// Outcome descriptor for [`BUBBLE_INPUT_OUTCOME`] (`SF.R21`, `35b` §3b).
pub(crate) fn outcome_descriptor() -> Result<mush_core::DefinitionDescriptor, ModError> {
    crate::port_tick::descriptor(
        "outcome",
        BUBBLE_INPUT_OUTCOME,
        serde_json::json!({"format":1,"fields":["actor","hull","seq","switch"]}),
    )
}

/// Action descriptor for [`BUBBLE_INPUT_ACTION`] (`SF.R21`, `35b` §3b).
pub(crate) fn action_descriptor(
    outcomes: &BTreeMap<String, mush_core::DefinitionRef>,
) -> Result<mush_core::DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(BUBBLE_INPUT_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the space bubble input outcome did not register"))?;
    crate::port_tick::descriptor(
        "action",
        BUBBLE_INPUT_ACTION,
        serde_json::json!({"format":1,"subject":"self_actor","targets":0,"family":BUBBLE_TAPE_FAMILY}),
    )
    .map(|d| d.with_references(vec![outcome]))
}

/// Register the [`BUBBLE_TAPE_FAMILY`] state family, [`BubbleInputProvider`], and [`BubbleInputHandles`] (`SF.R21`, `SF.R22`).
pub(crate) fn register_bubble_input(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, mush_core::DefinitionRef>,
    outcomes: &BTreeMap<String, mush_core::DefinitionRef>,
    port_families: &crate::port::families::PortFamilies,
) -> Result<(), ModError> {
    use crate::port::families::family_descriptor;
    let tapes = reg.state_family::<ObjId, StagedBubbleTapeLedger, _, _, _, _>(
        family_descriptor(
            BUBBLE_TAPE_FAMILY,
            "tacsim.space_bubble_tape_key.v1",
            "tacsim.space_bubble_tape",
        )?,
        |key: &ObjId| Ok(key.0.to_string()),
        |value: &serde_json::Value| {
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())
        },
        |value: &StagedBubbleTapeLedger| value.check(),
        |value: &StagedBubbleTapeLedger| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let action = if let Some(action) = actions.get(BUBBLE_INPUT_ACTION).cloned() {
        let outcome = outcomes
            .get(BUBBLE_INPUT_OUTCOME)
            .cloned()
            .ok_or_else(|| ModError::msg("the space bubble input outcome did not register"))?;
        reg.action_provider(
            action.clone(),
            BubbleInputProvider {
                action: action.clone(),
                outcome,
                tapes: tapes.clone(),
                excursions: port_families.excursions.clone(),
            },
        )?;
        Some(action)
    } else {
        None
    };
    reg.state(BubbleInputHandles {
        action,
        tapes,
        pending_bubble_inputs: Vec::new(),
        next_bubble_input_seq: 0,
        pending_burn_submit_actors: std::collections::BTreeSet::new(),
    });
    Ok(())
}

/// Queue a [`PendingBubbleInput`] and submit [`BUBBLE_INPUT_ACTION`] via `Ctx::submit_authoritative` (`SF.R21`, `35b` §3b).
pub fn submit_bubble_input(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    switch: &str,
    cmd: Option<BubbleInputCommand>,
) -> Option<u64> {
    if !ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .is_some_and(|b| b.active_for_hull(hull).is_some())
    {
        return None;
    }
    submit_bubble_input_with_drones(ctx, hull, switch, cmd, Vec::new())
}

/// Submit collector drone sortie [`crate::surface::excursion::RigRecord`]s into `tacsim.excursion`
/// via [`BUBBLE_INPUT_ACTION`]'s `prepare_authoritative` (`SF.R22`, `D-35-13`).
pub fn submit_drone_excursions(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    drone_excursions: Vec<(ObjId, u8, mush_core::CanonicalId)>,
) -> Option<u64> {
    if drone_excursions.is_empty() {
        return None;
    }
    submit_bubble_input_with_drones(ctx, hull, "drones_launch", None, drone_excursions)
}

fn submit_bubble_input_with_drones(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    switch: &str,
    cmd: Option<BubbleInputCommand>,
    drone_excursions: Vec<(ObjId, u8, mush_core::CanonicalId)>,
) -> Option<u64> {
    let at_ic = ctx.w.world_ic_now().at;
    let actor = ctx.actor;
    let (seq, can_submit, action) = {
        let handles = ctx.w.module_state_mut::<BubbleInputHandles>()?;
        handles.next_bubble_input_seq = handles.next_bubble_input_seq.saturating_add(1);
        let seq = handles.next_bubble_input_seq;
        handles.pending_bubble_inputs.push(PendingBubbleInput {
            seq,
            actor,
            hull,
            switch: switch.to_owned(),
            at_ic,
            cmd,
            drone_excursions,
        });
        if handles.pending_bubble_inputs.len() > 64 {
            let excess = handles.pending_bubble_inputs.len() - 64;
            handles.pending_bubble_inputs.drain(0..excess);
        }
        let can_submit = !ctx.out.authoritative_pending(actor)
            && !handles.pending_burn_submit_actors.contains(&actor);
        (seq, can_submit, handles.action.clone())
    };
    if can_submit
        && let Some(action) = action
        && let Ok(key) = mush_core::IdempotencyKeyText::parse_canonical(format!(
            "tacsim.space.input/{}/{}/{}",
            hull.0, seq, switch
        ))
    {
        ctx.submit_authoritative(
            action,
            mush_core::PerceivedRef::SelfActor,
            Vec::new(),
            serde_json::json!({
                "hull": hull.0,
                "seq": seq,
                "switch": switch,
            }),
            key,
        );
    }
    Some(seq)
}

/// Authoritative action provider for [`BUBBLE_INPUT_ACTION`] (`SF.R21`, `SF.R22`, `35b` §3b).
pub(crate) struct BubbleInputProvider {
    pub action: mush_core::DefinitionRef,
    pub outcome: mush_core::DefinitionRef,
    pub tapes: mush_core::StateFamilyHandle<ObjId, StagedBubbleTapeLedger>,
    pub excursions: mush_core::StateFamilyHandle<
        crate::surface::excursion::RigKey,
        crate::surface::excursion::RigRecord,
    >,
}

impl mush_core::ActionProvider for BubbleInputProvider {
    fn validate_invocation(
        &self,
        invocation: &mush_core::ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        let invalid = || {
            crate::port_tick::port_error(
                mush_core::ActionErrorCode::InvalidRequest,
                "a space bubble input is self-scoped and names hull, seq, and switch",
            )
        };
        if invocation.subject() != &mush_core::PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
        {
            return Err(invalid());
        }
        let params = invocation.parameters().as_object().ok_or_else(invalid)?;
        let _hull = params
            .get("hull")
            .and_then(serde_json::Value::as_u64)
            .and_then(|h| u32::try_from(h).ok())
            .filter(|&h| h > 0)
            .ok_or_else(invalid)?;
        let _seq = params
            .get("seq")
            .and_then(serde_json::Value::as_u64)
            .filter(|&s| s > 0)
            .ok_or_else(invalid)?;
        let _switch = params
            .get("switch")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid)?;
        Ok(())
    }

    fn project_offer(
        &self,
        action: &mush_core::DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, mush_core::ActionError> {
        if !crate::surface::act::self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w = &engine.world;
        let Some(handles) = w.module_state::<BubbleInputHandles>() else {
            return Ok(None);
        };
        let mut pending_map = serde_json::Map::new();
        for entry in handles
            .pending_bubble_inputs
            .iter()
            .filter(|e| e.actor == query.actor())
        {
            if let Ok(val) = serde_json::to_value(entry) {
                pending_map.insert(entry.seq.to_string(), val);
            }
        }
        if pending_map.is_empty() {
            return Ok(None);
        }
        let parameters = vec![
            crate::surface::act::parameter(
                "hull",
                true,
                mush_core::ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(u32::MAX),
                },
            )?,
            crate::surface::act::parameter(
                "seq",
                true,
                mush_core::ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::MAX,
                },
            )?,
            crate::surface::act::parameter(
                "switch",
                true,
                mush_core::ProjectedParameterKind::Text { maximum_bytes: 32 },
            )?,
        ];
        crate::surface::act::offer_draft(
            action,
            engine,
            serde_json::json!({ "pending": pending_map }),
            parameters,
            (
                "tacsim-space-bubble-input",
                "Stage space bubble input",
                "Stage an authenticated +helm or +tac bubble command onto tacsim.space_bubble_tape.",
            ),
        )
        .map(Some)
    }

    fn prepare_authoritative(
        &self,
        invocation: &mush_core::ActionInvocation,
        txn: &mut mush_core::ActionTxn<'_>,
    ) -> Result<mush_core::AuthoritativeActionOutput, mush_core::ActionError> {
        self.validate_invocation(invocation)?;
        let invalid = || {
            crate::port_tick::port_error(
                mush_core::ActionErrorCode::InvalidRequest,
                "that space bubble input is not on file",
            )
        };
        let params = invocation.parameters().as_object().ok_or_else(invalid)?;
        let hull = ObjId(
            params
                .get("hull")
                .and_then(serde_json::Value::as_u64)
                .and_then(|h| u32::try_from(h).ok())
                .ok_or_else(invalid)?,
        );
        let seq = params
            .get("seq")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(invalid)?;
        let switch = params
            .get("switch")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(invalid)?;
        let pending_val = params
            .get("pending")
            .and_then(serde_json::Value::as_object)
            .and_then(|m| m.get(&seq.to_string()))
            .ok_or_else(invalid)?;
        let pending: PendingBubbleInput =
            serde_json::from_value(pending_val.clone()).map_err(|_| invalid())?;
        if pending.actor != invocation.actor() || pending.hull != hull || pending.switch != switch {
            return Err(crate::port_tick::port_error(
                mush_core::ActionErrorCode::Unauthorized,
                "that space bubble input belongs to another actor or hull",
            ));
        }
        let prior = txn
            .get(&self.tapes, &hull)
            .map_err(|_| {
                crate::port_tick::port_error(
                    mush_core::ActionErrorCode::Integrity,
                    "the staged bubble tape family cannot be read",
                )
            })?
            .cloned();
        let mut recent = prior.as_ref().map(|l| l.recent.clone()).unwrap_or_default();
        recent.push(StagedBubbleTapeEntry {
            seq,
            actor: pending.actor,
            hull,
            switch: pending.switch.clone(),
            at_ic: pending.at_ic,
            cmd: pending.cmd,
        });
        if recent.len() > MAX_BUBBLE_TAPE_ENTRIES {
            let excess = recent.len() - MAX_BUBBLE_TAPE_ENTRIES;
            recent.drain(0..excess);
        }
        let ledger = StagedBubbleTapeLedger {
            hull,
            last_seq: seq,
            recent,
        };
        let staged = if prior.is_some() {
            txn.replace(&self.tapes, &hull, ledger)
        } else {
            txn.insert(&self.tapes, &hull, ledger)
        };
        staged.map_err(|_| {
            crate::port_tick::port_error(
                mush_core::ActionErrorCode::Integrity,
                "the staged bubble tape ledger did not stage",
            )
        })?;
        for (drone_obj, slot, site) in &pending.drone_excursions {
            let key = crate::surface::excursion::RigKey(*drone_obj);
            let prior_rig = txn
                .get(&self.excursions, &key)
                .map_err(|_| {
                    crate::port_tick::port_error(
                        mush_core::ActionErrorCode::Integrity,
                        "the excursion family cannot be read",
                    )
                })?
                .cloned();
            let ordinal = prior_rig
                .as_ref()
                .map_or(1, |r| r.ordinal.saturating_add(1));
            let rig_record = crate::surface::excursion::RigRecord {
                field_crew: None,
                field_work: None,
                schema: crate::surface::excursion::EXCURSION_SCHEMA,
                tv: *drone_obj,
                ordinal,
                charge: crate::surface::excursion::Charge {
                    value_cu: 1_000,
                    at_ic: pending.at_ic,
                    in_bay: false,
                },
                act: None,
                excursion: Some(crate::surface::excursion::Excursion {
                    id: crate::surface::excursion::ExcursionId {
                        tv: *drone_obj,
                        launch_ic: pending.at_ic,
                    },
                    operator: pending.actor,
                    hull,
                    slot: (*slot).clamp(1, 16),
                    site: site.clone(),
                }),
                last: None,
                run: None,
                queued: None,
                plan: None,
                plans: 0,
                recovery: None,
                sortie: None,
                claim: None,
                bale: None,
            };
            let rig_staged = if prior_rig.is_some() {
                txn.replace(&self.excursions, &key, rig_record)
            } else {
                txn.insert(&self.excursions, &key, rig_record)
            };
            rig_staged.map_err(|_| {
                crate::port_tick::port_error(
                    mush_core::ActionErrorCode::Integrity,
                    "the collector drone excursion record did not stage",
                )
            })?;
        }
        let outcome = mush_core::RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({
                "actor": pending.actor.0,
                "hull": hull.0,
                "seq": seq,
                "switch": pending.switch,
            }),
        )
        .map_err(|_| {
            crate::port_tick::port_error(
                mush_core::ActionErrorCode::BoundsExceeded,
                "the space bubble input outcome exceeds canonical bounds",
            )
        })?;
        Ok(mush_core::AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// Re-acquire `DeployedMark::field` custody for any active space bubbles after `field::boot::phase`
/// runs during engine startup (`SF.R10`, `35b` §5 #6).
pub fn reacquire_custody_after_boot(ctx: &mut Ctx<'_>) {
    let active_hulls: Vec<(ObjId, DomainId, u32)> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            let mut out = Vec::new();
            for (&domain, active) in &b.by_domain {
                for (chull, _, cleg, _) in active.all_carrier_hulls() {
                    out.push((chull, domain, cleg));
                }
                out.push((active.interdictor_hull, domain, active.leg_cursor));
                if let Some(picket) = &active.picket {
                    out.push((picket.corvette_hull, domain, active.leg_cursor));
                    if picket.raider_hull != active.interdictor_hull {
                        out.push((picket.raider_hull, domain, active.leg_cursor));
                    }
                }
            }
            out
        })
        .unwrap_or_default();
    for (hull, domain, leg_cursor) in active_hulls {
        if mush_tactics::module::unit_of(ctx.w, hull).is_some()
            && !mush_sim::deployed::is_deployed(ctx.w, hull)
        {
            let mark =
                mush_sim::DeployedMark::field(format!("domain.{}", domain.0), leg_cursor.max(1));
            let _ = mush_tactics::custody::check_out(ctx, hull, mark);
        }
    }
}

struct SpacePresenceHook;

impl mush_core::EventHook for SpacePresenceHook {
    fn connected(&self, ctx: &mut Ctx<'_>, player: ObjId) {
        handle_crew_revived_or_connected(ctx, player);
    }

    fn revived(&self, ctx: &mut Ctx<'_>, player: ObjId) {
        handle_crew_revived_or_connected(ctx, player);
    }

    fn linkdead(&self, ctx: &mut Ctx<'_>, player: ObjId) {
        handle_crew_linkdead(ctx, player);
    }

    fn disconnected(&self, ctx: &mut Ctx<'_>, player: ObjId) {
        handle_crew_disconnected(ctx, player);
    }
}

fn any_other_live_crew_aboard(w: &World, hull: ObjId, excluding: ObjId) -> bool {
    super::commands::all_aboard_crew_for_hull(w, hull)
        .into_iter()
        .any(|c| c != excluding && w.is_connected(c) && w.has_live_conns(c))
}

fn handle_crew_linkdead(ctx: &mut Ctx<'_>, player: ObjId) {
    let Some(hull) = super::commands::hull_aboard_for_actor(ctx.w, player) else {
        return;
    };
    if any_other_live_crew_aboard(ctx.w, hull, player) {
        return;
    }
    let ic_now_s = ctx.w.world_ic_now().at;
    let wall_now_s = ctx.w.now();
    let queued_return_port = super::commands::nearest_return_port_for_hull(ctx.w, hull)
        .or_else(|| Some("ceres_freeport".to_owned()));
    let has_journey = ctx
        .w
        .module_state::<JourneyBook>()
        .is_some_and(|jb| jb.get(hull).is_some());

    let mut affected = false;
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        if let Some(active) = book.active_for_hull_mut(hull) {
            let ship_id = active.carrier_ship_id_for(hull).to_owned();
            if let Some(ship) = active.bubble.ships.get_mut(&ship_id) {
                ship.crew_linkdead = true;
                ship.thrust_mps2 = [0, 0, 0];
                ship.pdc_auto = true;
                affected = true;
            }
        }
        if let Some(ff) = book.free_flights.get_mut(&hull) {
            ff.thrust_mmps2 = [0, 0, 0];
            ff.burn_remaining_ic_s = None;
            affected = true;
        }
        if let Some(cs) = book.cluster_sessions.get_mut(&hull) {
            cs.approaching_rock = None;
            cs.ship_vel_mmps = [0, 0, 0];
            affected = true;
        }
        if affected || has_journey {
            book.linkdead_hulls.insert(
                hull,
                SpaceLinkdeadState {
                    hull,
                    grace_started_wall_s: wall_now_s,
                    grace_started_ic_s: ic_now_s,
                    pdc_held: true,
                    grace_expired: false,
                    queued_return_port: queued_return_port.clone(),
                },
            );
            affected = true;
        }
    }
    if affected {
        let port_label = queued_return_port.as_deref().unwrap_or("nearest port");
        let line = format!(
            "[SPACE] Bridge crew linkdead; main drive cut to ballistic drift, point-defense held AUTO for {}s grace window, return plan queued for {}.",
            SPACE_LINKDEAD_GRACE_WALL_S, port_label
        );
        crate::port::voyage::narrate_hull(ctx, hull, &line);
    }
}

fn handle_crew_disconnected(ctx: &mut Ctx<'_>, player: ObjId) {
    let Some(hull) = super::commands::hull_aboard_for_actor(ctx.w, player) else {
        return;
    };
    if any_other_live_crew_aboard(ctx.w, hull, player) {
        return;
    }
    expire_linkdead_grace_for_hull(ctx, hull);
}

fn expire_linkdead_grace_for_hull(ctx: &mut Ctx<'_>, hull: ObjId) {
    let mut newly_expired_port: Option<String> = None;
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        if let Some(active) = book.active_for_hull_mut(hull) {
            let ship_id = active.carrier_ship_id_for(hull).to_owned();
            if let Some(ship) = active.bubble.ships.get_mut(&ship_id) {
                ship.crew_linkdead = true;
                ship.thrust_mps2 = [0, 0, 0];
            }
        }
        if let Some(ff) = book.free_flights.get_mut(&hull) {
            ff.thrust_mmps2 = [0, 0, 0];
            ff.burn_remaining_ic_s = None;
            ff.guidance = FreeFlightGuidanceMode::Retrograde;
        }
        if let Some(cs) = book.cluster_sessions.get_mut(&hull) {
            cs.approaching_rock = None;
            cs.ship_vel_mmps = [0, 0, 0];
        }
        if let Some(state) = book.linkdead_hulls.get_mut(&hull)
            && !state.grace_expired
        {
            state.grace_expired = true;
            newly_expired_port = Some(
                state
                    .queued_return_port
                    .clone()
                    .unwrap_or_else(|| "ceres".to_owned()),
            );
        }
    }
    if let Some(port) = newly_expired_port {
        let _ = super::commands::execute_linkdead_return_journey(ctx, hull, &port);
        let line = format!(
            "[SPACE] Linkdead grace window expired; hull holding ballistic life-support reserve with queued return plan to {}.",
            port
        );
        crate::port::voyage::narrate_hull(ctx, hull, &line);
    }
}

fn handle_crew_revived_or_connected(ctx: &mut Ctx<'_>, player: ObjId) {
    let Some(hull) = super::commands::hull_aboard_for_actor(ctx.w, player) else {
        return;
    };
    let mut restored = false;
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        if book.linkdead_hulls.remove(&hull).is_some() {
            restored = true;
        }
        if let Some(active) = book.active_for_hull_mut(hull) {
            let ship_id = active.carrier_ship_id_for(hull).to_owned();
            if let Some(ship) = active.bubble.ships.get_mut(&ship_id)
                && ship.crew_linkdead
            {
                ship.crew_linkdead = false;
                restored = true;
            }
        }
    }
    super::commands::push_space_panels(ctx, hull);
    if restored {
        crate::port::voyage::narrate_hull(
            ctx,
            hull,
            "[SPACE] Bridge link restored; manual helm and tactical authority resumed.",
        );
    }
}

fn step_linkdead_grace_windows(ctx: &mut Ctx<'_>) {
    if ctx.w.world_timeline().is_frozen() {
        return;
    }
    let wall_now_s = ctx.w.now();
    let due_hulls: Vec<ObjId> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            b.linkdead_hulls
                .values()
                .filter(|s| {
                    !s.grace_expired
                        && s.grace_started_wall_s > 0
                        && wall_now_s - s.grace_started_wall_s >= SPACE_LINKDEAD_GRACE_WALL_S
                })
                .map(|s| s.hull)
                .collect()
        })
        .unwrap_or_default();
    for hull in due_hulls {
        expire_linkdead_grace_for_hull(ctx, hull);
    }
}

/// Register [`SpaceBubbleBook`], [`SalvageClaimBook`], `SpacePresenceHook`, and the [`SPACE_TICK`] consumer on `reg`.
///
/// # Errors
///
/// Propagates any duplicate-registration error from [`Registrar::tick`].
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(SpaceBubbleBook::default());
    reg.state(SalvageClaimBook::new());
    reg.hook(mush_core::HookPoint::Connected, Arc::new(SpacePresenceHook));
    reg.hook(mush_core::HookPoint::Linkdead, Arc::new(SpacePresenceHook));
    reg.hook(
        mush_core::HookPoint::Disconnected,
        Arc::new(SpacePresenceHook),
    );
    reg.tick(
        SPACE_TICK,
        TickCadence::Coarse(SPACE_TICK_PERIOD),
        Arc::new(space_tick),
    )?;
    Ok(())
}

/// Coarse tick callback for [`SPACE_TICK`].
pub fn space_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    if !ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(|s| s.enabled() && !s.is_space_state_held())
    {
        return;
    }
    let now = ctx.w.world_ic_now();
    step_linkdead_grace_windows(ctx);
    super::picket::step_pending_light_lag_hails(ctx);
    expire_stale_docking_sessions(ctx, &now);
    step_derelict_records(ctx);
    clear_resolved_bubbles(ctx, &now);
    settle_disabled_carrier_bubbles(ctx, &now);
    step_active_bubbles(ctx, &now);
    step_free_flight_records(ctx, &now);
    super::mining::step_clusters_on_tick(ctx, now.clone());
    promote_due_interdictions(ctx, &now);
    step_delivered_tows_at_port(ctx, &now);
    crate::surface::claim::tick(ctx);
    prune_space_book_collections(ctx);
    persist_space_state(ctx);
}

fn prune_space_book_collections(ctx: &mut Ctx<'_>) {
    let active_journey_hulls: std::collections::BTreeSet<ObjId> = ctx
        .w
        .module_state::<JourneyBook>()
        .map(|jb| jb.journeys.keys().copied().collect())
        .unwrap_or_default();
    let existing_hulls: std::collections::BTreeSet<ObjId> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            b.propellant_ledgers
                .keys()
                .copied()
                .filter(|&h| ctx.w.get(h).is_some())
                .collect()
        })
        .unwrap_or_default();
    let burn_hulls: Vec<ObjId> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            let mut hs: Vec<ObjId> = b.pending_space_burns.iter().map(|p| p.hull).collect();
            hs.sort_unstable();
            hs.dedup();
            hs
        })
        .unwrap_or_default();
    let mut tank_by_hull: BTreeMap<ObjId, u64> = BTreeMap::new();
    for h in burn_hulls {
        if let Some(view) = crate::port::ship::hull_view(ctx.w, h)
            && let Ok(basis) = crate::port::voyage::basis(ctx, &view, 1)
        {
            tank_by_hull.insert(h, basis.tank);
        }
    }

    let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() else {
        return;
    };
    let mut landed_hulls = std::collections::BTreeSet::new();
    for (&hull, &vault_tank_t) in &tank_by_hull {
        if let Some(ledger) = book.propellant_ledgers.get_mut(&hull) {
            if ledger.last_vault_tank_t > vault_tank_t {
                let landed_t = ledger.last_vault_tank_t - vault_tank_t;
                ledger.unlanded_burn_t = ledger.unlanded_burn_t.saturating_sub(landed_t);
                ledger.last_vault_tank_t = vault_tank_t;
            }
            if ledger.unlanded_burn_t == 0 {
                landed_hulls.insert(hull);
            }
        }
    }
    if !landed_hulls.is_empty() {
        book.pending_space_burns.retain(|b| {
            !(landed_hulls.contains(&b.hull) && b.seized_strategic_ore_t == 0 && b.crew_days == 0)
        });
    }
    if book.pending_space_burns.len() > 16 {
        let excess = book.pending_space_burns.len() - 16;
        book.pending_space_burns.drain(0..excess);
    }

    let clamped_set: std::collections::BTreeSet<ObjId> =
        book.clamped_derelicts.values().copied().collect();
    let towed_set: std::collections::BTreeSet<ObjId> =
        book.active_tows.values().map(|t| t.derelict_hull).collect();
    book.derelicts.retain(|&hull, d| {
        clamped_set.contains(&hull)
            || towed_set.contains(&hull)
            || (d.epoch_open && (d.life_support_remaining_s > 0 || d.salvage_contest_window_s > 0))
    });

    let active_bubble_hulls: std::collections::BTreeSet<ObjId> = book
        .by_domain
        .values()
        .flat_map(|a| a.all_carrier_hulls().into_iter().map(|(ch, _, _, _)| ch))
        .collect();
    book.cleared_interdiction_legs.retain(|&(hull, _)| {
        active_journey_hulls.contains(&hull) || active_bubble_hulls.contains(&hull)
    });

    book.propellant_ledgers
        .retain(|hull, _| existing_hulls.contains(hull));
    while book.propellant_ledgers.len() > 64 {
        let Some(&oldest) = book.propellant_ledgers.keys().next() else {
            break;
        };
        book.propellant_ledgers.remove(&oldest);
    }
    while book.completed.len() > MAX_COMPLETED_BUBBLES {
        let Some(&oldest) = book.completed.keys().next() else {
            break;
        };
        book.completed.remove(&oldest);
    }
    let remaining_burn_actors: std::collections::BTreeSet<ObjId> =
        book.pending_space_burns.iter().map(|b| b.actor).collect();
    if let Some(handles) = ctx.w.module_state_mut::<BubbleInputHandles>() {
        handles
            .pending_burn_submit_actors
            .retain(|a| remaining_burn_actors.contains(a));
    }
}

/// Complete port delivery for any active tow on `tug_hull` once `tug_hull` is berthed at a port
/// (`SF.R19`, `Q-36-46`, `D-35-12`).
///
/// Docks `derelict_hull` at the tug's port domain so `berth_of(w, derelict_hull)` resolves to
/// `Berth::Docked` for bay repair (`+workshop`) and claim inspection (`+salvage/inspect`,
/// `+salvage/contest`, `+salvage/report`), logs `BlackBoxFlightEntry::EpochClosedInBay`, and
/// settles the one-time `1,200 cr` salvage recovery bounty on port delivery.
pub fn deliver_active_tow_at_port(ctx: &mut Ctx<'_>, tug_hull: ObjId, now: &ClockStamp) {
    let Some(tow) = ctx.w.module_state_mut::<SpaceBubbleBook>().and_then(|b| {
        let tow = b.active_tows.remove(&tug_hull)?;
        b.clamped_derelicts.remove(&tug_hull);
        Some(tow)
    }) else {
        return;
    };
    let derelict_hull = tow.derelict_hull;
    let _ = mush_tactics::custody::check_in(ctx, derelict_hull, None);
    if let Some((port_domain, port_pose)) = mush_sim::pose_of(ctx.w, tug_hull) {
        let _ = mush_sim::dock_carrier_at_destination(
            ctx,
            derelict_hull,
            mush_sim::SpatialParent::Direct {
                domain: port_domain,
                pose: port_pose.clone(),
            },
            now,
        );
        let _ = mush_sim::place(ctx, derelict_hull, port_domain, port_pose);
        let base_obj = resolve_nearest_port_base_obj(ctx.w, port_domain, tug_hull);
        let op_id = ctx.w.module_state::<SpaceBubbleBook>().and_then(|b| {
            b.space_custody
                .get(&derelict_hull)
                .and_then(|c| c.operation)
        });
        if let Some(salvage_book) = ctx.w.module_state_mut::<SalvageClaimBook>() {
            let _ = salvage_book.append_black_box_entry(
                derelict_hull,
                crate::field_ops::BlackBoxFlightEntry::EpochClosedInBay {
                    operation_id: op_id,
                    base: base_obj,
                    at_ic: now.at,
                },
            );
        }
    }
    if let Some(pilot) = crate::port::coop::helm_holder(ctx.w, tug_hull).or_else(|| {
        super::commands::all_aboard_crew_for_hull(ctx.w, tug_hull)
            .into_iter()
            .next()
    }) {
        let _ =
            super::picket::settle_salvage_recovery_payout_from_registry(ctx, pilot, derelict_hull);
    }
}

fn step_delivered_tows_at_port(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    let docked_tugs: Vec<ObjId> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            b.active_tows
                .keys()
                .copied()
                .filter(|&tug_hull| {
                    matches!(
                        crate::port::ship::berth_of(ctx.w, tug_hull),
                        crate::port::ship::Berth::Docked { .. }
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    for tug_hull in docked_tugs {
        deliver_active_tow_at_port(ctx, tug_hull, now);
    }
}

fn expire_stale_docking_sessions(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    if ctx.w.world_timeline().is_frozen() {
        return;
    }
    let wall_now_s = ctx.w.now();
    let expired: Vec<ObjId> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            b.docking_sessions
                .iter()
                .filter(|(_, s)| {
                    if s.started_wall_s > 0 && wall_now_s >= s.started_wall_s {
                        wall_now_s - s.started_wall_s
                            >= super::docking::DOCKING_SESSION_TIMEOUT_WALL_S
                    } else {
                        now.at.saturating_sub(s.started_ic_s)
                            >= super::docking::DOCKING_SESSION_TIMEOUT_WALL_S * 3_600
                    }
                })
                .map(|(&hull, _)| hull)
                .collect()
        })
        .unwrap_or_default();
    if expired.is_empty() {
        return;
    }
    for hull in expired {
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
            book.docking_sessions.remove(&hull);
        }
        let _ = super::commands::debit_hull_propellant_kg(ctx, hull, 15);
        crate::port::voyage::narrate_hull(
            ctx,
            hull,
            "[DOCKING] Session wall deadline (60s) expired; collar approach aborted with 15 kg RCS spent and hull unpaused.",
        );
    }
}

fn step_derelict_records(ctx: &mut Ctx<'_>) {
    if ctx.w.world_timeline().is_frozen() {
        return;
    }
    let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() else {
        return;
    };
    let step_s = u32::try_from(SPACE_TICK_PERIOD.as_secs())
        .unwrap_or(1)
        .max(1);
    for derelict in book.derelicts.values_mut() {
        if derelict.epoch_open {
            derelict.life_support_remaining_s =
                derelict.life_support_remaining_s.saturating_sub(step_s);
            derelict.salvage_contest_window_s =
                derelict.salvage_contest_window_s.saturating_sub(step_s);
        }
    }
}

/// Resolve the real world owner or charterer [`ObjId`] of `hull` for salvage claim filing (`SF.R12`, `SF.R19`, `35b` §5 #7).
#[must_use]
pub(crate) fn resolve_hull_prior_owner(w: &World, hull: ObjId, recoverer_hull: ObjId) -> ObjId {
    if let Some(mush_org::PrincipalId::Object(holder)) = w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().asset(hull))
        .map(|a| a.title_owner.clone())
        && holder != hull
        && holder != recoverer_hull
    {
        return holder;
    }
    if let Some(pilot) = crate::port::coop::helm_holder(w, hull)
        && pilot != recoverer_hull
    {
        return pilot;
    }
    if let Some(leader) = crate::field::raiders::leader_of(w, hull)
        && leader != recoverer_hull
    {
        return leader;
    }
    if let Some(pilot) =
        w.module_state::<crate::store::TacsimStore>()
            .and_then(|s| match s.voyage_state(hull) {
                crate::port::voyage::VoyageState::Present(v) => {
                    v.departed.as_ref().map(|d| d.pilot)
                }
                crate::port::voyage::VoyageState::Absent
                | crate::port::voyage::VoyageState::Unreadable => None,
            })
        && pilot != recoverer_hull
    {
        return pilot;
    }
    if let Some(owner) = w.get(hull).map(|o| o.owner)
        && owner != hull
        && owner != recoverer_hull
    {
        return owner;
    }
    if mush_core::GOD != recoverer_hull {
        mush_core::GOD
    } else {
        ObjId(2)
    }
}

/// Resolve a real world base/port [`ObjId`] for salvage claim filing at `domain` (`SF.R12`, `SF.R19`, `35b` §5 #7).
#[must_use]
pub(crate) fn resolve_nearest_port_base_obj(
    w: &World,
    domain: DomainId,
    recoverer_hull: ObjId,
) -> ObjId {
    for (_, map) in crate::port::voyage::lane_ports(w) {
        if let Some(host_raw) = map.and_then(|m| m.host) {
            let host = ObjId(host_raw);
            if w.get(host).is_some() && host != recoverer_hull {
                return host;
            }
        }
    }
    if let Some(host) = w
        .module_state::<SpatialState>()
        .and_then(|s| s.domains.get(&domain))
        .map(|d| d.host)
        && w.get(host).is_some()
        && host != recoverer_hull
    {
        return host;
    }
    if mush_core::GOD != recoverer_hull {
        mush_core::GOD
    } else {
        ObjId(2)
    }
}

fn clear_resolved_bubbles(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    let wall_now_s = ctx.w.now();
    let mut detached_carriers: Vec<(ObjId, Option<SpaceShipState>)> = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        let paused_hulls: std::collections::BTreeSet<ObjId> = book
            .docking_sessions
            .iter()
            .filter(|(_, s)| s.paused_from_world_tick)
            .map(|(&h, _)| h)
            .collect();
        let clamped_hulls: std::collections::BTreeSet<ObjId> =
            book.clamped_derelicts.keys().copied().collect();
        for active in book.by_domain.values_mut() {
            for entry in active.carriers.values_mut() {
                if entry.holding_for_salvage
                    && !clamped_hulls.contains(&entry.hull)
                    && entry
                        .holding_for_salvage_until_wall_s
                        .is_some_and(|until| wall_now_s >= until)
                {
                    entry.holding_for_salvage = false;
                    entry.holding_for_salvage_until_wall_s = None;
                    entry.approaching_derelict = false;
                }
            }
            if !active.carriers.is_empty() {
                active.holding_for_salvage =
                    active.carriers.values().any(|c| c.holding_for_salvage);
                active.approaching_derelict =
                    active.carriers.values().any(|c| c.approaching_derelict);
            }
            // Per-carrier clearance and salvage hold (`SF.R17`, `SF.R18`): when multiple carriers
            // share the bubble, any carrier that is clear to resume and not paused for docking
            // detaches and resumes while remaining carriers continue in the bubble.
            if active.carriers.len() > 1 {
                let ready_hulls: Vec<ObjId> = active
                    .carriers
                    .keys()
                    .copied()
                    .filter(|&ch| {
                        !paused_hulls.contains(&ch) && active.is_carrier_clear_to_resume(ch)
                    })
                    .collect();
                let all_ready = ready_hulls.len() == active.carriers.len();
                if !all_ready && !ready_hulls.is_empty() {
                    for ch in ready_hulls {
                        if active.carriers.len() <= 1 {
                            break;
                        }
                        if let Some(entry) = active.carriers.remove(&ch) {
                            book.cleared_interdiction_legs
                                .insert((ch, entry.leg_cursor));
                            let cship = active.bubble.ships.remove(&entry.ship_id);
                            active.ship_hulls.remove(&entry.ship_id);
                            if let Some(picket) = active.picket.as_mut() {
                                picket.carrier_inspection_status.remove(&ch);
                                picket.carrier_inspection_outcome.remove(&ch);
                            }
                            if active.hull == ch
                                && let Some(next_entry) = active.carriers.values().next()
                            {
                                active.hull = next_entry.hull;
                                active.carrier_ship_id = next_entry.ship_id.clone();
                                active.leg_cursor = next_entry.leg_cursor;
                                active.approaching_derelict = next_entry.approaching_derelict;
                                active.holding_for_salvage = next_entry.holding_for_salvage;
                            }
                            let mut completed =
                                CompletedInterdictionBubble::from_closed(active, now.at);
                            completed.hull = ch;
                            book.completed.insert(ch, completed);
                            detached_carriers.push((ch, cship));
                        }
                    }
                }
            }
        }
    }
    for (ch, cship) in detached_carriers {
        mush_sim::tick::clear_unit_standing_orders(ctx.w, ch);
        if let Some(ship) = cship {
            let _ = check_in_space_unit(ctx, ch, &ship, now.at);
        }
        let _ = resume_carrier_journey(ctx, ch, now);
        if let Some(new_cursor) = ctx
            .w
            .module_state::<JourneyBook>()
            .and_then(|jb| jb.get(ch))
            .map(CarrierJourney::cursor)
            && let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
        {
            book.cleared_interdiction_legs.insert((ch, new_cursor));
        }
        let line =
            "[SPACE] Interdiction contact cleared; local bubble closed and transit burn resumed.";
        crate::port::voyage::narrate_hull(ctx, ch, line);
    }

    let Some(book) = ctx.w.module_state::<SpaceBubbleBook>() else {
        return;
    };
    let cleared_domains: Vec<DomainId> = book
        .by_domain
        .iter()
        .filter(|(_, active)| {
            let any_paused = active
                .all_carrier_hulls()
                .iter()
                .any(|(ch, _, _, _)| book.is_hull_paused_for_docking(*ch));
            !any_paused && active.is_clear_to_resume()
        })
        .map(|(&domain, _)| domain)
        .collect();
    if cleared_domains.is_empty() {
        return;
    }

    let mut to_finalize = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        for domain in cleared_domains {
            let Some(active) = book.by_domain.remove(&domain) else {
                continue;
            };
            let carrier_entries: Vec<(ObjId, Option<SpaceShipState>, u32, i64)> = active
                .all_carrier_hulls()
                .into_iter()
                .map(|(ch, sid, cleg, dep_ic)| {
                    book.cleared_interdiction_legs.insert((ch, cleg));
                    (ch, active.bubble.ships.get(&sid).cloned(), cleg, dep_ic)
                })
                .collect();
            let mut npc_contacts: Vec<(ObjId, SpaceShipState)> = Vec::new();
            if let Some(iship) = active.bubble.ships.get(&active.interdictor_ship_id) {
                npc_contacts.push((active.interdictor_hull, iship.clone()));
            }
            if let Some(picket) = &active.picket {
                if picket.corvette_hull != active.interdictor_hull
                    && let Some(cship) = active.bubble.ships.get(&picket.corvette_ship_id)
                {
                    npc_contacts.push((picket.corvette_hull, cship.clone()));
                }
                if picket.raider_hull != active.interdictor_hull
                    && let Some(rship) = active.bubble.ships.get(&picket.raider_ship_id)
                {
                    npc_contacts.push((picket.raider_hull, rship.clone()));
                }
            }
            for &(ch, _, _, _) in &carrier_entries {
                let mut completed = CompletedInterdictionBubble::from_closed(&active, now.at);
                completed.hull = ch;
                if book.completed.len() >= MAX_COMPLETED_BUBBLES
                    && !book.completed.contains_key(&ch)
                    && let Some(&oldest_hull) = book.completed.keys().next()
                {
                    book.completed.remove(&oldest_hull);
                }
                book.completed.insert(ch, completed);
            }
            if book.pending_space_burns.len() > 16 {
                let excess = book.pending_space_burns.len() - 16;
                book.pending_space_burns.drain(0..excess);
            }
            to_finalize.push((active.hull, domain, carrier_entries, npc_contacts));
        }
    }

    for (_primary_hull, domain, carrier_entries, npc_contacts) in to_finalize {
        for (ch, cship, _, _) in &carrier_entries {
            mush_sim::tick::clear_unit_standing_orders(ctx.w, *ch);
            if let Some(ship) = cship {
                let _ = check_in_space_unit(ctx, *ch, ship, now.at);
            }
        }

        for (contact_hull, iship) in npc_contacts {
            let assessment = check_in_space_unit(ctx, contact_hull, &iship, now.at).ok();
            if let HullStatus::DisabledDerelict {
                epoch_open,
                life_support_remaining_s,
                salvage_contest_window_s,
            } = iship.status
                && let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
            {
                let existing_status = book
                    .derelicts
                    .get(&contact_hull)
                    .and_then(|d| d.salvage_title_status.clone());
                book.derelicts.insert(
                    contact_hull,
                    SpaceDerelictRecord {
                        hull: contact_hull,
                        domain,
                        disabled_at_ic_s: now.at,
                        epoch_open,
                        life_support_remaining_s,
                        salvage_contest_window_s,
                        assessment,
                        salvage_title_status: existing_status,
                    },
                );
            }
            destroy_unclaimed_npc_hull(ctx, contact_hull);
        }

        for (ch, _, _, _) in &carrier_entries {
            let _ = resume_carrier_journey(ctx, *ch, now);
            if let Some(new_cursor) = ctx
                .w
                .module_state::<JourneyBook>()
                .and_then(|jb| jb.get(*ch))
                .map(CarrierJourney::cursor)
                && let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
            {
                book.cleared_interdiction_legs.insert((*ch, new_cursor));
            }
            let line = "[SPACE] Interdiction contact cleared; local bubble closed and transit burn resumed.";
            crate::port::voyage::narrate_hull(ctx, *ch, line);
        }
        let any_other_promoted = ctx.w.module_state::<JourneyBook>().is_some_and(|jb| {
            jb.journeys
                .values()
                .any(|j| matches!(j.state(), JourneyState::Promoted { domain: d } if *d == domain))
        });
        if !any_other_promoted {
            let _ = stand_down_domain(ctx, domain, now.clone());
        }
    }
}

fn settle_disabled_carrier_bubbles(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    // 1. Per-carrier disabled settlement in multi-carrier bubbles (`SF.R18`): if some (not all)
    //    carriers in a bubble are disabled into `DisabledDerelict` and ordnance has resolved,
    //    detach and settle each disabled carrier while leaving operational carriers in the bubble.
    let mut partial_settled: Vec<(ObjId, ObjId, DomainId, Option<SpaceShipState>)> = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        for (&domain, active) in &mut book.by_domain {
            if !active.bubble.torpedoes.is_empty() || !active.bubble.railgun_slugs.is_empty() {
                continue;
            }
            if active.carriers.len() <= 1 {
                continue;
            }
            let disabled_hulls: Vec<ObjId> =
                active
                    .carriers
                    .values()
                    .filter(|c| {
                        active.bubble.ships.get(&c.ship_id).is_some_and(|s| {
                            matches!(s.status, HullStatus::DisabledDerelict { .. })
                        })
                    })
                    .map(|c| c.hull)
                    .collect();
            if disabled_hulls.is_empty() || disabled_hulls.len() == active.carriers.len() {
                continue;
            }
            for ch in disabled_hulls {
                if active.carriers.len() <= 1 {
                    break;
                }
                if let Some(entry) = active.carriers.remove(&ch) {
                    let cship = active.bubble.ships.get(&entry.ship_id).cloned();
                    if let Some(picket) = active.picket.as_mut() {
                        picket.carrier_inspection_status.remove(&ch);
                        picket.carrier_inspection_outcome.remove(&ch);
                    }
                    if active.hull == ch
                        && let Some(next_entry) = active.carriers.values().next()
                    {
                        active.hull = next_entry.hull;
                        active.carrier_ship_id = next_entry.ship_id.clone();
                        active.leg_cursor = next_entry.leg_cursor;
                        active.approaching_derelict = next_entry.approaching_derelict;
                        active.holding_for_salvage = next_entry.holding_for_salvage;
                    }
                    let mut completed = CompletedInterdictionBubble::from_closed(active, now.at);
                    completed.hull = ch;
                    book.completed.insert(ch, completed);
                    partial_settled.push((ch, active.interdictor_hull, domain, cship));
                }
            }
        }
    }

    for (hull, _interdictor_hull, domain, carrier_ship) in partial_settled {
        mush_sim::tick::clear_unit_standing_orders(ctx.w, hull);
        if let Some(ship) = carrier_ship {
            let assessment = check_in_space_unit(ctx, hull, &ship, now.at).ok();
            let (epoch_open, life_support_remaining_s, salvage_contest_window_s) = match ship.status
            {
                HullStatus::DisabledDerelict {
                    epoch_open,
                    life_support_remaining_s,
                    salvage_contest_window_s,
                } => (
                    epoch_open,
                    life_support_remaining_s,
                    salvage_contest_window_s,
                ),
                HullStatus::Operational => (true, DERELICT_WINDOW_S, DERELICT_WINDOW_S),
            };
            if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                let existing_status = book
                    .derelicts
                    .get(&hull)
                    .and_then(|d| d.salvage_title_status.clone());
                book.derelicts.insert(
                    hull,
                    SpaceDerelictRecord {
                        hull,
                        domain,
                        disabled_at_ic_s: now.at,
                        epoch_open,
                        life_support_remaining_s,
                        salvage_contest_window_s,
                        assessment,
                        salvage_title_status: existing_status,
                    },
                );
            }
        }
        let _ = mush_sim::fail_carrier_journey(
            ctx,
            hull,
            "carrier hull disabled into an open-epoch derelict",
        );
        let line = "[SPACE] Drive offline; hull adrift as an open-epoch derelict with a 48-hour Registry salvage window.";
        crate::port::voyage::narrate_hull(ctx, hull, line);
    }

    let Some(book) = ctx.w.module_state::<SpaceBubbleBook>() else {
        return;
    };
    let disabled_domains: Vec<DomainId> = book
        .by_domain
        .iter()
        .filter(|(_, active)| active.is_carrier_disabled_and_settled())
        .map(|(&domain, _)| domain)
        .collect();
    if disabled_domains.is_empty() {
        return;
    }

    let mut settled = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        for domain in disabled_domains {
            let Some(active) = book.by_domain.remove(&domain) else {
                continue;
            };
            let interdictor_hull = active.interdictor_hull;
            let carrier_ships: Vec<(ObjId, Option<SpaceShipState>)> = active
                .all_carrier_hulls()
                .into_iter()
                .map(|(ch, sid, _, _)| (ch, active.bubble.ships.get(&sid).cloned()))
                .collect();
            let mut npc_contacts: Vec<(ObjId, SpaceShipState)> = Vec::new();
            if let Some(iship) = active.bubble.ships.get(&active.interdictor_ship_id) {
                npc_contacts.push((active.interdictor_hull, iship.clone()));
            }
            if let Some(picket) = &active.picket {
                if picket.corvette_hull != active.interdictor_hull
                    && let Some(cship) = active.bubble.ships.get(&picket.corvette_ship_id)
                {
                    npc_contacts.push((picket.corvette_hull, cship.clone()));
                }
                if picket.raider_hull != active.interdictor_hull
                    && let Some(rship) = active.bubble.ships.get(&picket.raider_ship_id)
                {
                    npc_contacts.push((picket.raider_hull, rship.clone()));
                }
            }
            for &(ch, _) in &carrier_ships {
                let mut completed = CompletedInterdictionBubble::from_closed(&active, now.at);
                completed.hull = ch;
                if book.completed.len() >= MAX_COMPLETED_BUBBLES
                    && !book.completed.contains_key(&ch)
                    && let Some(&oldest_hull) = book.completed.keys().next()
                {
                    book.completed.remove(&oldest_hull);
                }
                book.completed.insert(ch, completed);
            }
            if book.pending_space_burns.len() > 16 {
                let excess = book.pending_space_burns.len() - 16;
                book.pending_space_burns.drain(0..excess);
            }
            settled.push((interdictor_hull, domain, carrier_ships, npc_contacts));
        }
    }

    for (_interdictor_hull, domain, carrier_ships, npc_contacts) in settled {
        for (npc_hull, npc_ship) in npc_contacts {
            let assessment = check_in_space_unit(ctx, npc_hull, &npc_ship, now.at).ok();
            if let HullStatus::DisabledDerelict {
                epoch_open,
                life_support_remaining_s,
                salvage_contest_window_s,
            } = npc_ship.status
                && let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
            {
                let existing_status = book
                    .derelicts
                    .get(&npc_hull)
                    .and_then(|d| d.salvage_title_status.clone());
                book.derelicts.insert(
                    npc_hull,
                    SpaceDerelictRecord {
                        hull: npc_hull,
                        domain,
                        disabled_at_ic_s: now.at,
                        epoch_open,
                        life_support_remaining_s,
                        salvage_contest_window_s,
                        assessment,
                        salvage_title_status: existing_status,
                    },
                );
            }
            destroy_unclaimed_npc_hull(ctx, npc_hull);
        }
        for (hull, carrier_ship) in carrier_ships {
            mush_sim::tick::clear_unit_standing_orders(ctx.w, hull);
            let Some(ship) = carrier_ship else {
                continue;
            };
            let assessment = check_in_space_unit(ctx, hull, &ship, now.at).ok();
            let (epoch_open, life_support_remaining_s, salvage_contest_window_s) = match ship.status
            {
                HullStatus::DisabledDerelict {
                    epoch_open,
                    life_support_remaining_s,
                    salvage_contest_window_s,
                } => (
                    epoch_open,
                    life_support_remaining_s,
                    salvage_contest_window_s,
                ),
                HullStatus::Operational => (true, DERELICT_WINDOW_S, DERELICT_WINDOW_S),
            };
            if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                let existing_status = book
                    .derelicts
                    .get(&hull)
                    .and_then(|d| d.salvage_title_status.clone());
                book.derelicts.insert(
                    hull,
                    SpaceDerelictRecord {
                        hull,
                        domain,
                        disabled_at_ic_s: now.at,
                        epoch_open,
                        life_support_remaining_s,
                        salvage_contest_window_s,
                        assessment,
                        salvage_title_status: existing_status,
                    },
                );
            }
            let _ = mush_sim::fail_carrier_journey(
                ctx,
                hull,
                "carrier hull disabled into an open-epoch derelict",
            );
            let line = "[SPACE] Drive offline; hull adrift as an open-epoch derelict with a 48-hour Registry salvage window.";
            crate::port::voyage::narrate_hull(ctx, hull, line);
        }
        let any_other_promoted = ctx.w.module_state::<JourneyBook>().is_some_and(|jb| {
            jb.journeys
                .values()
                .any(|j| matches!(j.state(), JourneyState::Promoted { domain: d } if *d == domain))
        });
        if !any_other_promoted {
            let _ = stand_down_domain(ctx, domain, now.clone());
        }
    }
}

fn step_active_bubbles(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    let frozen = ctx.w.world_timeline().is_frozen();
    let wall_now_s = ctx.w.now();
    let mut combat_reports = Vec::new();
    let mut ran_picket_hulls = Vec::new();
    let standing_orders = mush_sim::tick::standing_orders_snapshot(ctx.w);

    let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() else {
        return;
    };
    if book.by_domain.is_empty() {
        return;
    }

    let paused_hulls: std::collections::BTreeSet<ObjId> = book
        .docking_sessions
        .iter()
        .filter(|(_, s)| s.paused_from_world_tick)
        .map(|(&h, _)| h)
        .collect();

    for active in book.by_domain.values_mut() {
        let hull = active.hull;
        active.bubble.outage_frozen = frozen;
        if frozen
            || active
                .all_carrier_hulls()
                .iter()
                .any(|(ch, _, _, _)| paused_hulls.contains(ch))
        {
            continue;
        }
        let cadence_ms = active.adaptive_step_wall_ms();
        let dt_wall_ms = if active.last_stepped_wall_s > 0 {
            if wall_now_s <= active.last_stepped_wall_s {
                continue;
            }
            let elapsed_wall_s = wall_now_s - active.last_stepped_wall_s;
            if cadence_ms == CRUISE_STEP_WALL_MS
                && elapsed_wall_s * 1_000 < i64::from(CRUISE_STEP_WALL_MS)
            {
                continue;
            }
            u32::try_from(elapsed_wall_s.saturating_mul(1_000))
                .unwrap_or(MAX_BUBBLE_TICK_STEP_MS)
                .min(MAX_BUBBLE_TICK_STEP_MS)
        } else {
            1_000
        };
        active.last_stepped_wall_s = wall_now_s;
        for (chull, cship_id, _, _) in active.all_carrier_hulls() {
            let is_linkdead = active
                .bubble
                .ships
                .get(&cship_id)
                .is_some_and(|s| s.crew_linkdead);
            if is_linkdead {
                continue;
            }
            if let Some(order) = standing_orders.get(chull, OrderChannel::Posture)
                && let OrderValue::Emcon { level } = order.value()
            {
                let posture = match level {
                    0 => EmconPosture::Ghost,
                    1 => EmconPosture::Passive,
                    2 => EmconPosture::Standard,
                    _ => EmconPosture::ActiveCombat,
                };
                if active
                    .bubble
                    .ships
                    .get(&cship_id)
                    .is_some_and(|s| s.emcon != posture)
                {
                    let _ = active.tape.record_and_apply(
                        &mut active.bubble,
                        BubbleInputCommand::SetEmcon {
                            ship_id: cship_id.clone(),
                            posture,
                        },
                    );
                }
            }
            if let Some(order) = standing_orders.get(chull, OrderChannel::Thrust)
                && let OrderValue::Scalar { value } = order.value()
                && *value <= 0
                && active
                    .bubble
                    .ships
                    .get(&cship_id)
                    .is_some_and(|s| s.thrust_mmps2 != [0, 0, 0])
            {
                let _ = active.tape.record_and_apply(
                    &mut active.bubble,
                    BubbleInputCommand::CommandBurnMmps2 {
                        ship_id: cship_id.clone(),
                        thrust_mmps2: [0, 0, 0],
                        force_into_reserve: false,
                    },
                );
            }
        }
        let mut events = active.tape.record_and_apply(
            &mut active.bubble,
            BubbleInputCommand::StepWall {
                dt_wall_ms,
                ic_now_s: now.at,
            },
        );
        let carrier_picket_entries = active.carrier_picket_entries();
        if let Some(picket) = active.picket.as_mut() {
            let picket_events = super::picket::step_kirkwood_picket_for_carriers(
                &mut active.bubble,
                &mut active.tape,
                picket,
                &active.carrier_ship_id,
                &carrier_picket_entries,
            );
            events.extend(picket_events);
            for (chull, _, _) in &carrier_picket_entries {
                if picket.inspection_status_for(*chull)
                    == super::picket::PicketInspectionStatus::RanThePicket
                    && picket.inspection_outcome_for(*chull).is_none()
                {
                    ran_picket_hulls.push(*chull);
                }
            }
        } else {
            let picket_events = step_interdictor_picket(
                &mut active.bubble,
                &mut active.tape,
                active.interdictor_hull,
                &active.interdictor_ship_id,
                &active.carrier_ship_id,
                active.interdictor_seal.as_ref(),
                &mut active.interdictor_think_seq,
                &mut active.interdictor_memory,
                &mut active.cleared,
            );
            events.extend(picket_events);
        }
        if active.approaching_derelict || active.carriers.values().any(|c| c.approaching_derelict) {
            step_derelict_approach_in_bubble(active);
        }
        active.push_bounded_snapshot();
        active.compact_tape_if_needed();
        active.steps = active.steps.saturating_add(1);
        if !events.is_empty() {
            let carrier_state = active
                .bubble
                .ships
                .get(&active.carrier_ship_id)
                .map(|s| match s.status {
                    HullStatus::Operational => OperationalState::Operational,
                    HullStatus::DisabledDerelict { .. } => OperationalState::Disabled,
                })
                .unwrap_or(OperationalState::Operational);
            let interdictor_state = active
                .bubble
                .ships
                .get(&active.interdictor_ship_id)
                .map(|s| match s.status {
                    HullStatus::Operational => OperationalState::Operational,
                    HullStatus::DisabledDerelict { .. } => OperationalState::Disabled,
                })
                .unwrap_or(OperationalState::Operational);
            combat_reports.push((
                hull,
                active.interdictor_hull,
                active.carrier_ship_id.clone(),
                active.interdictor_ship_id.clone(),
                events,
                carrier_state,
                interdictor_state,
            ));
        }
    }

    let burned_hulls: Vec<ObjId> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            let mut out = Vec::new();
            for active in b.by_domain.values() {
                if active.carriers.is_empty() {
                    let cur_kg = active
                        .bubble
                        .ships
                        .get(&active.carrier_ship_id)
                        .map_or(0, |s| s.propellant_kg);
                    if active.last_synced_propellant_kg > cur_kg {
                        out.push(active.hull);
                    }
                } else {
                    for entry in active.carriers.values() {
                        let cur_kg = active
                            .bubble
                            .ships
                            .get(&entry.ship_id)
                            .map_or(0, |s| s.propellant_kg);
                        if entry.last_synced_propellant_kg > cur_kg {
                            out.push(entry.hull);
                        }
                    }
                }
            }
            out
        })
        .unwrap_or_default();
    for hull in burned_hulls {
        super::commands::sync_bubble_propellant_to_ledger(ctx, hull);
    }

    for hull in ran_picket_hulls {
        let front_id = super::picket::record_ran_the_picket_incident(ctx, hull, hull).map(|f| f.0);
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
            && let Some(active) = book.active_for_hull_mut(hull)
            && let Some(picket) = active.picket.as_mut()
        {
            picket.set_inspection_outcome_for(
                hull,
                super::picket::PicketInspectionOutcome::RanThePicket { front_id },
            );
        }
    }

    if !combat_reports.is_empty()
        && let Some(salvage_book) = ctx.w.module_state_mut::<SalvageClaimBook>()
    {
        for (
            hull,
            interdictor_hull,
            carrier_ship_id,
            interdictor_ship_id,
            events,
            carrier_state,
            interdictor_state,
        ) in combat_reports
        {
            let _ = record_space_combat_to_black_box(
                salvage_book,
                hull,
                &carrier_ship_id,
                Some(interdictor_hull),
                &events,
                carrier_state,
                now.at,
            );
            let _ = record_space_combat_to_black_box(
                salvage_book,
                interdictor_hull,
                &interdictor_ship_id,
                Some(hull),
                &events,
                interdictor_state,
                now.at,
            );
        }
    }

    let stepped_carrier_hulls: Vec<ObjId> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            let mut out = Vec::new();
            for active in b.by_domain.values() {
                for (chull, _, _, _) in active.all_carrier_hulls() {
                    if !out.contains(&chull) {
                        out.push(chull);
                    }
                }
            }
            out
        })
        .unwrap_or_default();
    for chull in stepped_carrier_hulls {
        super::commands::push_space_panels(ctx, chull);
    }
}

fn step_free_flight_records(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    if ctx.w.world_timeline().is_frozen() {
        return;
    }
    let (hulls, paused_hulls): (
        Vec<(ObjId, mush_core::CanonicalId)>,
        std::collections::BTreeSet<ObjId>,
    ) = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            let paused: std::collections::BTreeSet<ObjId> = b
                .docking_sessions
                .iter()
                .filter(|(_, s)| s.paused_from_world_tick)
                .map(|(&h, _)| h)
                .collect();
            let active = b
                .free_flights
                .iter()
                .filter(|(hull, _)| !paused.contains(hull))
                .map(|(&hull, rec)| (hull, rec.target_place.clone()))
                .collect();
            (active, paused)
        })
        .unwrap_or_default();
    if !paused_hulls.is_empty()
        && let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
    {
        for hull in paused_hulls {
            if let Some(rec) = book.free_flights.get_mut(&hull) {
                let dt_paused = now.at.saturating_sub(rec.last_ic_s).max(0);
                rec.last_ic_s = rec.last_ic_s.max(now.at);
                if let Some(arr_ic) = &mut rec.cluster_arrival_ic_s {
                    *arr_ic = arr_ic.saturating_add(dt_paused);
                }
            }
        }
    }
    if hulls.is_empty() {
        return;
    }

    let mut target_vels: BTreeMap<ObjId, Option<[i64; 3]>> = BTreeMap::new();
    let mut target_positions: BTreeMap<ObjId, Option<[i64; 3]>> = BTreeMap::new();
    let mut prev_target_positions: BTreeMap<ObjId, Option<[i64; 3]>> = BTreeMap::new();
    let prev_ic_by_hull: BTreeMap<ObjId, i64> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            b.free_flights
                .iter()
                .map(|(&h, r)| (h, r.last_ic_s))
                .collect()
        })
        .unwrap_or_default();
    if let Some(store) = ctx.w.module_state::<TacsimStore>()
        && let Ok(lane) = mush_core::CanonicalId::new(crate::definitions::content::LANE_PACK)
        && let Some(version) = store.applied_in(ctx.w, &lane)
        && let Ok((sky, pack)) = crate::port::voyage::lane_sky_of(ctx.w, version)
    {
        for (hull, target_place) in &hulls {
            if let Ok(body) = crate::port::passage::body_or_place_of(&sky, pack, target_place) {
                let vel = sky
                    .velocity_mps(&body, now.at)
                    .map(|[vx, vy, vz]| [i64::from(vx), i64::from(vy), i64::from(vz)]);
                target_vels.insert(*hull, vel);
                let pos = sky.position_km(&body, now.at).map(|p| [p.x, p.y, p.z]);
                target_positions.insert(*hull, pos);
                let prev_ic = prev_ic_by_hull.get(hull).copied().unwrap_or(now.at);
                let prev_pos = sky.position_km(&body, prev_ic).map(|p| [p.x, p.y, p.z]);
                prev_target_positions.insert(*hull, prev_pos);
            }
        }
    }

    let is_cluster_by_hull: BTreeMap<ObjId, bool> = hulls
        .iter()
        .map(|(h, tp)| {
            (
                *h,
                super::mining::lookup_orbital_cluster(ctx.w, tp.as_str()).is_some(),
            )
        })
        .collect();
    let mut arrived_bodies: Vec<(ObjId, String)> = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        for (hull, _) in &hulls {
            let tvel = target_vels.get(hull).copied().flatten();
            let tpos = target_positions.get(hull).copied().flatten();
            let prev_tpos = prev_target_positions.get(hull).copied().flatten();
            if let Some(rec) = book.free_flights.get_mut(hull) {
                let dt_ic_s = now.at.saturating_sub(rec.last_ic_s);
                let was_co_located = prev_tpos.is_some_and(|p| rec.pos_km == p);
                if dt_ic_s > 0 {
                    rec.step_ic_s(dt_ic_s, tvel);
                    if was_co_located
                        && let (Some(pos), Some(vel)) = (tpos, tvel)
                        && super::picket::relative_speed_3d_mmps(rec.vel_mps, vel)
                            .saturating_mul(dt_ic_s.max(0))
                            < 500
                    {
                        rec.pos_km = pos;
                    }
                }
                if !is_cluster_by_hull.get(hull).copied().unwrap_or(false)
                    && rec
                        .cluster_arrival_ic_s
                        .is_some_and(|arr_ic| now.at >= arr_ic)
                {
                    if let Some(pos) = tpos {
                        rec.pos_km = pos;
                    }
                    if let Some(vel) = tvel {
                        rec.vel_mps = vel;
                    }
                    rec.cluster_arrival_ic_s = None;
                    arrived_bodies.push((*hull, rec.target_place.as_str().to_owned()));
                }
            }
        }
    }
    for (hull, body_name) in arrived_bodies {
        let line = format!(
            "[SPACE] Arrived in orbit at `{body_name}`; bridge canopy tracking local system."
        );
        crate::port::voyage::narrate_hull(ctx, hull, &line);
    }

    // Promote any free-flight hulls that have reached their destination orbital cluster (`SF.R9`, `SF.R22`, `D-35-5`, `D-35-17`):
    let arrived_clusters: Vec<(ObjId, super::mining::ResolvedOrbitalCluster)> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            b.free_flights
                .iter()
                .filter_map(|(&hull, rec)| {
                    let cluster =
                        super::mining::lookup_orbital_cluster(ctx.w, rec.target_place.as_str())?;
                    let arrived = rec
                        .cluster_arrival_ic_s
                        .is_some_and(|arr_ic| now.at >= arr_ic);
                    arrived.then_some((hull, cluster))
                })
                .collect()
        })
        .unwrap_or_default();
    for (hull, cluster) in arrived_clusters {
        let _ = super::mining::ensure_cluster_laid_up(ctx, &cluster, hull);
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
            book.free_flights.remove(&hull);
            let existing_drones = book
                .cluster_sessions
                .get(&hull)
                .map(|s| s.drone_things.clone())
                .unwrap_or_default();
            let mut session =
                super::mining::ActiveClusterMiningSession::new(hull, cluster.id.clone(), now.at);
            session.drone_things = existing_drones;
            book.cluster_sessions.insert(hull, session);
        }
        let line = format!(
            "[SPACE] Arrived at {} ({}); orbital cluster site promoted.",
            cluster.display_name(),
            cluster.as_str(),
        );
        crate::port::voyage::narrate_hull(ctx, hull, &line);
    }
    for (hull, _) in hulls {
        super::commands::push_space_panels(ctx, hull);
    }
}

#[allow(clippy::too_many_arguments)]
fn step_interdictor_picket(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    interdictor_hull: ObjId,
    interdictor_ship_id: &str,
    carrier_ship_id: &str,
    interdictor_seal: Option<&crate::arena::mind::MindSeal>,
    think_seq: &mut u64,
    memory: &mut Vec<u8>,
    cleared: &mut bool,
) -> Vec<super::bubble::SpaceCombatEvent> {
    use crate::arena::{IntentKind, MotorGoal};

    let mut emitted = Vec::new();
    let Some(iship) = bubble.ships.get(interdictor_ship_id) else {
        return emitted;
    };
    if iship.status != HullStatus::Operational {
        return emitted;
    }
    let carrier_operational = bubble
        .ships
        .get(carrier_ship_id)
        .is_some_and(|s| s.status == HullStatus::Operational);
    if !carrier_operational {
        return emitted;
    }
    if iship.emcon == super::bubble::EmconPosture::Ghost {
        emitted.extend(tape.record_and_apply(
            bubble,
            BubbleInputCommand::SetEmcon {
                ship_id: interdictor_ship_id.to_owned(),
                posture: super::bubble::EmconPosture::Standard,
            },
        ));
    }
    let Some((res, cmap)) = super::picket::run_space_rhai_think_with_seal(
        bubble,
        interdictor_ship_id,
        interdictor_hull,
        interdictor_seal,
        crate::minds::content::interdictor_patrol_digest().0,
        None,
        *think_seq,
        memory,
    ) else {
        return emitted;
    };
    if res.fault.is_some() {
        return emitted;
    }
    *think_seq = think_seq.saturating_add(1);
    *memory = res.memory;
    if res
        .trace
        .iter()
        .any(|t| t == "INTERDICTOR_STAND_DOWN_CLEARED")
    {
        *cleared = true;
        let carrier_parties: Vec<PartyId> = bubble
            .ships
            .values()
            .filter(|s| s.party != INTERDICTOR_PARTY)
            .map(|s| s.party)
            .collect();
        for cparty in carrier_parties {
            bubble.set_party_relation(cparty, INTERDICTOR_PARTY, None);
        }
        emitted.extend(tape.record_and_apply(
            bubble,
            BubbleInputCommand::CommandBurn {
                ship_id: interdictor_ship_id.to_owned(),
                thrust_mps2: [0, 0, 0],
                force_into_reserve: false,
            },
        ));
        return emitted;
    }

    let mut locked_ship = bubble
        .ships
        .get(interdictor_ship_id)
        .and_then(|s| s.locked_target_id.clone())
        .unwrap_or_else(|| carrier_ship_id.to_owned());

    if bubble
        .ships
        .get(interdictor_ship_id)
        .is_some_and(|s| s.heat_permille > 650)
    {
        emitted.extend(tape.record_and_apply(
            bubble,
            BubbleInputCommand::BlowdownCoolant {
                ship_id: interdictor_ship_id.to_owned(),
            },
        ));
    }

    for intent in &res.intents {
        match intent {
            IntentKind::Emcon { on } => {
                let target_posture = if *on {
                    super::bubble::EmconPosture::ActiveCombat
                } else {
                    super::bubble::EmconPosture::Standard
                };
                if bubble
                    .ships
                    .get(interdictor_ship_id)
                    .is_some_and(|s| s.emcon != target_posture)
                {
                    emitted.extend(tape.record_and_apply(
                        bubble,
                        BubbleInputCommand::SetEmcon {
                            ship_id: interdictor_ship_id.to_owned(),
                            posture: target_posture,
                        },
                    ));
                }
            }
            IntentKind::Lock { contact } => {
                if let Some(target_ship_id) = cmap.get(contact).cloned() {
                    locked_ship = target_ship_id.clone();
                    if bubble.ships.get(interdictor_ship_id).is_some_and(|s| {
                        s.locked_target_id.as_deref() != Some(target_ship_id.as_str())
                    }) {
                        emitted.extend(tape.record_and_apply(
                            bubble,
                            BubbleInputCommand::SetLockedTarget {
                                ship_id: interdictor_ship_id.to_owned(),
                                target_id: Some(target_ship_id),
                            },
                        ));
                    }
                }
            }
            IntentKind::Goal(MotorGoal::Hold) => {
                if bubble
                    .ships
                    .get(interdictor_ship_id)
                    .is_some_and(|s| s.thrust_mps2 != [0, 0, 0])
                {
                    emitted.extend(tape.record_and_apply(
                        bubble,
                        BubbleInputCommand::CommandBurn {
                            ship_id: interdictor_ship_id.to_owned(),
                            thrust_mps2: [0, 0, 0],
                            force_into_reserve: false,
                        },
                    ));
                }
            }
            IntentKind::Speed { cms } if *cms > 0 => {
                if bubble
                    .ships
                    .get(interdictor_ship_id)
                    .is_some_and(|s| s.thrust_mps2 == [0, 0, 0])
                {
                    emitted.extend(tape.record_and_apply(
                        bubble,
                        BubbleInputCommand::CommandBurn {
                            ship_id: interdictor_ship_id.to_owned(),
                            thrust_mps2: [-10, 0, 0],
                            force_into_reserve: false,
                        },
                    ));
                }
            }
            IntentKind::Fire { weapon, .. } => {
                if weapon.starts_with("torp") {
                    if bubble
                        .ships
                        .get(&locked_ship)
                        .is_some_and(|s| s.emcon == super::bubble::EmconPosture::ActiveCombat)
                        && bubble.ships.get(interdictor_ship_id).is_some_and(|s| {
                            s.torpedo_tubes.iter().any(|t| t.magazine_remaining == 8)
                        })
                    {
                        emitted.extend(tape.record_and_apply(
                            bubble,
                            BubbleInputCommand::FireTorpedoSalvo {
                                shooter_id: interdictor_ship_id.to_owned(),
                                target_id: locked_ship.clone(),
                                count: 1,
                            },
                        ));
                    }
                } else if weapon.starts_with("laser") {
                    emitted.extend(tape.record_and_apply(
                        bubble,
                        BubbleInputCommand::FireOffensiveLasers {
                            shooter_id: interdictor_ship_id.to_owned(),
                            target_id: locked_ship.clone(),
                        },
                    ));
                }
            }
            _ => {}
        }
    }
    emitted
}

fn step_derelict_approach_in_bubble(active: &mut ActiveInterdictionBubble) {
    let approaching_ship_id = active
        .carriers
        .values()
        .find(|c| c.approaching_derelict)
        .map_or_else(|| active.carrier_ship_id.clone(), |c| c.ship_id.clone());
    let derelict_pos_vel = active
        .bubble
        .ships
        .values()
        .find(|s| {
            s.id != approaching_ship_id && matches!(s.status, HullStatus::DisabledDerelict { .. })
        })
        .map(|s| (s.pos_m, s.vel_mmps));
    let Some((d_pos, d_vel)) = derelict_pos_vel else {
        active.approaching_derelict = false;
        for c in active.carriers.values_mut() {
            c.approaching_derelict = false;
        }
        return;
    };
    let Some(carrier) = active.bubble.ships.get_mut(&approaching_ship_id) else {
        active.approaching_derelict = false;
        return;
    };
    carrier.thrust_mps2 = [0, 0, 0];
    let dist_m = super::picket::distance_3d_m(carrier.pos_m, d_pos);
    let step_m = 6_000_i64;
    if dist_m <= 250 {
        carrier.pos_m = [d_pos[0] - 120, d_pos[1], d_pos[2]];
        carrier.vel_mmps = [d_vel[0] + 2_000, d_vel[1], d_vel[2]];
        active.approaching_derelict = false;
        for c in active.carriers.values_mut() {
            c.approaching_derelict = false;
        }
    } else {
        let new_dist = (dist_m - step_m).max(120);
        let dx = i128::from(carrier.pos_m[0]) - i128::from(d_pos[0]);
        let dy = i128::from(carrier.pos_m[1]) - i128::from(d_pos[1]);
        let dz = i128::from(carrier.pos_m[2]) - i128::from(d_pos[2]);
        let d = i128::from(dist_m.max(1));
        carrier.pos_m = [
            d_pos[0] + ((dx * i128::from(new_dist)) / d) as i64,
            d_pos[1] + ((dy * i128::from(new_dist)) / d) as i64,
            d_pos[2] + ((dz * i128::from(new_dist)) / d) as i64,
        ];
        carrier.vel_mmps = [d_vel[0] + 2_000, d_vel[1], d_vel[2]];
        if new_dist <= 250 {
            active.approaching_derelict = false;
            for c in active.carriers.values_mut() {
                c.approaching_derelict = false;
            }
        }
    }
}

fn promote_due_interdictions(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    let Some(journey_book) = ctx.w.module_state::<JourneyBook>().cloned() else {
        return;
    };
    let (already_open, already_derelict, in_free_flight, cleared_legs) = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            let mut open = Vec::new();
            for active in b.by_domain.values() {
                for (ch, _, _, _) in active.all_carrier_hulls() {
                    open.push(ch);
                }
            }
            (
                open,
                b.derelicts.keys().copied().collect::<Vec<ObjId>>(),
                b.free_flights.keys().copied().collect::<Vec<ObjId>>(),
                b.cleared_interdiction_legs.clone(),
            )
        })
        .unwrap_or_default();

    let mut to_promote: Vec<(ObjId, DomainId, u32, i64)> = Vec::new();
    for (&hull, journey) in &journey_book.journeys {
        let cursor = journey.cursor();
        let dep_at = journey.departed().at;
        if already_open.contains(&hull)
            || already_derelict.contains(&hull)
            || in_free_flight.contains(&hull)
            || cleared_legs.contains(&(hull, cursor))
        {
            continue;
        }
        if let Some(domain) = interdicted_domain_for_journey(ctx.w, journey) {
            to_promote.push((hull, domain, cursor, dep_at));
        }
    }

    if to_promote.is_empty() {
        return;
    }
    to_promote.sort_by_key(|(hull, domain, _, _)| {
        (
            !super::picket::should_open_kirkwood_picket(ctx.w, *hull, *domain),
            hull.0,
        )
    });

    for (hull, domain, cursor, dep_at) in to_promote {
        if !promote_carrier_journey(ctx, hull, domain, now) {
            continue;
        }
        let domain_has_active = ctx
            .w
            .module_state::<SpaceBubbleBook>()
            .is_some_and(|b| b.by_domain.contains_key(&domain));
        if domain_has_active {
            let carrier_ship_id = format!("hull.{}", hull.0);
            let site_id = format!("domain.{}", domain.0);
            let carrier_party = ctx
                .w
                .module_state::<SpaceBubbleBook>()
                .and_then(|b| b.by_domain.get(&domain))
                .map_or(
                    CARRIER_PARTY,
                    ActiveInterdictionBubble::next_available_carrier_party,
                );
            let Some(mut carrier_ship) = build_carrier_ship_state(
                ctx,
                hull,
                &site_id,
                cursor,
                &carrier_ship_id,
                carrier_party,
            ) else {
                continue;
            };
            let initial_prop_kg = carrier_ship.propellant_kg;
            let mut is_kirkwood_picket = false;
            if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>()
                && let Some(active) = book.by_domain.get_mut(&domain)
            {
                let offset_idx = i64::try_from(active.carriers.len().max(1)).unwrap_or(1);
                carrier_ship.pos_m = [0, 200 * offset_idx, 0];
                is_kirkwood_picket = active.picket.is_some();
                for existing in active.carriers.values() {
                    active.bubble.set_party_relation(
                        carrier_party,
                        existing.party,
                        Some(Mark::Friendly),
                    );
                }
                if let Some(ref mut picket) = active.picket {
                    active.bubble.set_party_relation(
                        carrier_party,
                        super::picket::BELT_RAIDER_PARTY,
                        Some(Mark::Hostile),
                    );
                    active.bubble.set_party_relation(
                        carrier_party,
                        super::picket::CUSTOMS_CORVETTE_PARTY,
                        None,
                    );
                    picket
                        .carrier_inspection_status
                        .insert(hull, super::picket::PicketInspectionStatus::AwaitingHeaveTo);
                } else {
                    active.bubble.set_party_relation(
                        carrier_party,
                        INTERDICTOR_PARTY,
                        Some(Mark::Hostile),
                    );
                }
                active.carriers.insert(
                    hull,
                    BubbleCarrierEntry {
                        hull,
                        ship_id: carrier_ship_id.clone(),
                        party: carrier_party,
                        leg_cursor: cursor,
                        departed_at_ic_s: dep_at,
                        last_synced_propellant_kg: initial_prop_kg,
                        approaching_derelict: false,
                        holding_for_salvage: false,
                        holding_for_salvage_until_wall_s: None,
                    },
                );
                active.ship_hulls.insert(carrier_ship_id, hull);
                active.bubble.insert_ship(carrier_ship);
            }
            let line = "[SPACE] Interdiction contact detected at waypoint boundary; joined active domain space bubble on 1:1 wall clock.";
            crate::port::voyage::narrate_hull(ctx, hull, line);
            super::commands::push_space_panels(ctx, hull);
            if is_kirkwood_picket {
                let _ = super::picket::emit_light_lagged_picket_hail(
                    ctx,
                    hull,
                    super::picket::CORVETTE_STANDOFF_M,
                );
            }
            continue;
        }

        let Some(active) = build_interdiction_bubble(ctx, hull, domain, cursor, dep_at, now.at)
        else {
            continue;
        };
        let is_kirkwood_picket = active.picket.is_some();
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
            book.by_domain.insert(domain, active);
        }
        let line = "[SPACE] Interdiction contact detected at waypoint boundary; local space bubble opened on 1:1 wall clock.";
        crate::port::voyage::narrate_hull(ctx, hull, line);
        super::commands::push_space_panels(ctx, hull);
        if is_kirkwood_picket {
            let _ = super::picket::emit_light_lagged_picket_hail(
                ctx,
                hull,
                super::picket::CORVETTE_STANDOFF_M,
            );
        }
    }
}

fn interdicted_domain_for_journey(w: &World, journey: &CarrierJourney) -> Option<DomainId> {
    if let JourneyState::Promoted { domain } = *journey.state() {
        return Some(domain);
    }
    let cursor = journey.cursor();
    let leg_count = u32::try_from(journey.route().legs().len()).ok()?;
    if cursor == 0 || cursor >= leg_count {
        return None;
    }
    let crossed_leg = journey
        .route()
        .legs()
        .get(usize::try_from(cursor.saturating_sub(1)).ok()?)?;
    if !crossed_leg.is_interdicted() {
        return None;
    }
    let waypoint = crossed_leg.to();
    let domain = w
        .module_state::<SpatialState>()
        .and_then(|s| s.domain_of_host(waypoint))
        .map(|rec| rec.id)?;
    let active_interception = w
        .module_state::<ActivationBook>()
        .and_then(|ab| ab.activations.get(&domain))
        .is_some_and(|act| {
            !act.is_queued() && matches!(act.trigger(), PromotionTrigger::Interception)
        });
    active_interception.then_some(domain)
}

fn build_interdiction_bubble(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    domain: DomainId,
    leg_cursor: u32,
    departed_at_ic_s: i64,
    ic_now_s: i64,
) -> Option<ActiveInterdictionBubble> {
    let carrier_ship_id = format!("hull.{}", hull.0);
    let site_id = format!("domain.{}", domain.0);

    let mut carrier_ship = build_carrier_ship_state(
        ctx,
        hull,
        &site_id,
        leg_cursor,
        &carrier_ship_id,
        CARRIER_PARTY,
    )?;
    let open_kirkwood = super::picket::should_open_kirkwood_picket(ctx.w, hull, domain);
    if !open_kirkwood {
        // Flip-and-burn braking attitude as the carrier decelerates into the waypoint (`D-35-1`).
        carrier_ship.attitude_deg = (180, 0, 0);
    }

    let bubble_id = format!("bubble.interdiction.{}.{}", domain.0, leg_cursor);
    let mut bubble = LocalSpaceBubble::new(
        bubble_id,
        site_id.clone(),
        if open_kirkwood {
            "Kirkwood Customs Picket Bubble"
        } else {
            "Waypoint Interdiction Bubble"
        },
        ic_now_s,
    );
    bubble.insert_ship(carrier_ship);

    let mut ship_hulls = BTreeMap::new();
    ship_hulls.insert(carrier_ship_id.clone(), hull);

    let (interdictor_hull, interdictor_ship_id, mut picket, interdictor_seal) = if open_kirkwood {
        let picket_state = super::picket::spawn_kirkwood_picket_ships_in_world(
            ctx,
            &mut bubble,
            domain,
            leg_cursor,
            &carrier_ship_id,
        )?;
        ship_hulls.insert(
            picket_state.corvette_ship_id.clone(),
            picket_state.corvette_hull,
        );
        ship_hulls.insert(
            picket_state.raider_ship_id.clone(),
            picket_state.raider_hull,
        );
        let raider_hull = picket_state.raider_hull;
        let raider_ship_id = picket_state.raider_ship_id.clone();
        (raider_hull, raider_ship_id, Some(picket_state), None)
    } else {
        let cast = super::picket::picket_cast_book()?;
        let interdictor_ship_id = format!("picket.{}.{}", domain.0, leg_cursor);
        let interdictor_commander = reuse_or_create_npc_commander(
            ctx,
            &cast.interdictor.commander,
            cast.interdictor.organization.as_deref(),
        )?;
        let interdictor_hull =
            reuse_or_create_npc_hull(ctx, &cast.interdictor.hull_name, interdictor_commander)?;
        super::custody::create_stock_space_unit_in_world(
            ctx,
            interdictor_hull,
            &cast.interdictor.unit,
        )
        .ok()?;
        super::custody::ensure_space_hull_titled(ctx, interdictor_hull, interdictor_commander);
        let interdictor_seal = super::picket::seal_space_unit_in_world(
            ctx,
            interdictor_hull,
            crate::arena::mind::SortieId::ic(interdictor_hull, ic_now_s.max(1)),
            &cast.interdictor.doctrine,
            crate::minds::content::interdictor_patrol_digest().0,
        );
        let mut interdictor_ship = check_out_space_unit(
            ctx,
            interdictor_hull,
            &site_id,
            leg_cursor,
            &interdictor_ship_id,
            &cast.interdictor.hull_name,
            INTERDICTOR_PARTY,
            cast.interdictor.pos_m,
            cast.interdictor.vel_mmps,
        )
        .ok()?;
        super::custody::apply_space_unit_overlay(
            &mut interdictor_ship,
            &super::custody::space_unit_overlay_for_id(&cast.interdictor.overlay),
        );
        interdictor_ship.attitude_deg = (0, 0, 0);
        interdictor_ship.emcon = super::bubble::EmconPosture::Ghost;
        interdictor_ship.heat_permille = 0;
        interdictor_ship.thrust_mps2 = [0, 0, 0];
        bubble.set_party_relation(CARRIER_PARTY, INTERDICTOR_PARTY, Some(Mark::Hostile));
        bubble.insert_ship(interdictor_ship);
        ship_hulls.insert(interdictor_ship_id.clone(), interdictor_hull);
        (
            interdictor_hull,
            interdictor_ship_id,
            None,
            interdictor_seal,
        )
    };
    if let Some(ref mut p) = picket {
        p.carrier_inspection_status
            .insert(hull, super::picket::PicketInspectionStatus::AwaitingHeaveTo);
    }

    let initial_prop_kg = bubble
        .ships
        .get(&carrier_ship_id)
        .map_or(0, |s| s.propellant_kg);
    let initial_snapshot = bubble.snapshot();
    let tape = BubbleInputTape::new(&bubble);

    let mut carriers = BTreeMap::new();
    carriers.insert(
        hull,
        BubbleCarrierEntry {
            hull,
            ship_id: carrier_ship_id.clone(),
            party: CARRIER_PARTY,
            leg_cursor,
            departed_at_ic_s,
            last_synced_propellant_kg: initial_prop_kg,
            approaching_derelict: false,
            holding_for_salvage: false,
            holding_for_salvage_until_wall_s: None,
        },
    );

    Some(ActiveInterdictionBubble {
        hull,
        interdictor_hull,
        domain,
        carrier_ship_id,
        interdictor_ship_id,
        leg_cursor,
        opened_ic_s: ic_now_s,
        steps: 0,
        cleared: false,
        bubble,
        tape,
        snapshots: vec![initial_snapshot],
        picket,
        approaching_derelict: false,
        holding_for_salvage: false,
        last_stepped_wall_s: ctx.w.now(),
        last_synced_propellant_kg: initial_prop_kg,
        carriers,
        ship_hulls,
        interdictor_memory: Vec::new(),
        interdictor_think_seq: 0,
        interdictor_seal,
    })
}

fn build_carrier_ship_state(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    site_id: &str,
    leg_cursor: u32,
    carrier_ship_id: &str,
    party: PartyId,
) -> Option<SpaceShipState> {
    let callsign = crate::port::coop::name_of(ctx.w, hull);
    let mut ship = if mush_tactics::module::unit_of(ctx.w, hull).is_some() {
        if let Ok(ship) = check_out_space_unit(
            ctx,
            hull,
            site_id,
            leg_cursor,
            carrier_ship_id,
            &callsign,
            party,
            [0, 0, 0],
            [25_000, 0, 0],
        ) {
            Some(ship)
        } else if let Some(unit) = mush_tactics::module::unit_of(ctx.w, hull) {
            space_ship_from_tactical_unit(
                unit,
                carrier_ship_id,
                callsign,
                party,
                [0, 0, 0],
                [25_000, 0, 0],
            )
            .ok()
        } else {
            None
        }
    } else {
        let (unit, _) = instantiate_stock_space_unit(hull, "tacsim.unit.dray").ok()?;
        space_ship_from_tactical_unit(
            &unit,
            carrier_ship_id,
            callsign,
            party,
            [0, 0, 0],
            [25_000, 0, 0],
        )
        .ok()
    }?;

    // Hydrate `propellant_kg`, `max_accel_mps2`, `delta_v_mps`, and `return_reserve_dv_mps`
    // from the hull's live `stow.propellant` pool and `HullPropellantLedger` (`SF.R11`, `D-35-6`).
    if let Some(view) = crate::port::ship::hull_view(ctx.w, hull) {
        let souls = view
            .airlock()
            .map_or(1, |a| crate::port::voyage::occupants(ctx.w, a).max(1));
        if let Ok(basis) = crate::port::voyage::basis(ctx, &view, souls) {
            let live_kg = ctx
                .w
                .module_state::<SpaceBubbleBook>()
                .and_then(|b| b.propellant_ledger(hull))
                .map_or_else(
                    || basis.tank.saturating_mul(1_000),
                    |l| l.effective_live_kg(basis.tank),
                );
            let non_prop_payload = basis
                .payload
                .saturating_sub(basis.tank.saturating_mul(1_000));
            let effective_payload = non_prop_payload.saturating_add(live_kg);
            ship.non_prop_payload_kg = non_prop_payload;
            ship.propellant_kg =
                u32::try_from(live_kg.min(u64::from(u32::MAX))).unwrap_or(u32::MAX);
            ship.recompute_drive_kinematics();
            if let Some(accel) = crate::port::passage::flip_accel_mps2(
                &basis.spec.drive,
                basis.spec.dry_kg,
                effective_payload,
            ) {
                ship.max_accel_mmps2 = ((accel * 1_000.0).round() as i64).max(1);
                if ship.slow_pct > 0 && ship.slow_pct < 100 {
                    ship.max_accel_mmps2 = (ship
                        .max_accel_mmps2
                        .saturating_mul(i64::from(100_u8.saturating_sub(ship.slow_pct)))
                        / 100)
                        .max(1);
                }
                ship.max_accel_mps2 = ship
                    .max_accel_mmps2
                    .saturating_add(999)
                    .div_euclid(1_000)
                    .max(1);
            }
            ship.delta_v_mps = i64::from(
                crate::port::passage::remaining_dv_mps_for_propellant(
                    &basis.spec.drive,
                    basis.spec.dry_kg,
                    effective_payload,
                    live_kg,
                )
                .unwrap_or(0),
            );
            if let Some(reserve_dv) = super::commands::compute_carrier_bubble_return_reserve_dv(
                ctx.w,
                hull,
                &basis,
                live_kg,
                ctx.w.world_ic_now().at,
            ) {
                ship.return_reserve_dv_mps = i64::from(reserve_dv);
            }
        }
    }
    let is_linkdead = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .is_some_and(|b| b.linkdead_hulls.contains_key(&hull))
        || (!super::commands::all_aboard_crew_for_hull(ctx.w, hull).is_empty()
            && !any_other_live_crew_aboard(ctx.w, hull, ObjId(0)));
    if is_linkdead {
        ship.crew_linkdead = true;
        ship.thrust_mps2 = [0, 0, 0];
        ship.pdc_auto = true;
    }
    Some(ship)
}

/// Resolve an existing NPC commander by `name` (owned by `GOD`) or create one, optionally
/// registering `faction` in `mush-faction` (`SF.R20`).
pub(crate) fn reuse_or_create_npc_commander(
    ctx: &mut Ctx<'_>,
    name: &str,
    faction: Option<&str>,
) -> Option<ObjId> {
    if let Some(existing) = ctx
        .w
        .iter()
        .find(|o| {
            o.kind == mush_core::ObjKind::Npc
                && !o.flags.contains(mush_core::Flags::GOING)
                && o.owner == mush_core::GOD
                && o.name.as_str() == name
        })
        .map(|o| o.id)
    {
        return Some(existing);
    }
    let npc = ctx
        .w
        .create(mush_core::ObjKind::Npc, name, mush_core::GOD, None)
        .ok()?;
    if let Some(org) = faction
        && ctx.w.module_state::<mush_faction::FactionState>().is_some()
    {
        let _ = mush_faction::register_content_faction(ctx, org, npc);
    }
    Some(npc)
}

/// Reuse an idle NPC space hull (`ObjKind::Thing` owned by `owner` with `name`) or create a fresh
/// one (`SF.R20`, `35b` §5 #11).
pub(crate) fn reuse_or_create_npc_hull(
    ctx: &mut Ctx<'_>,
    name: &str,
    owner: ObjId,
) -> Option<ObjId> {
    let existing = ctx
        .w
        .iter()
        .find(|o| {
            if o.kind != mush_core::ObjKind::Thing
                || o.flags.contains(mush_core::Flags::GOING)
                || o.owner != owner
                || o.name.as_str() != name
                || mush_tactics::custody::marker_of(ctx.w, o.id).is_some()
            {
                return false;
            }
            let in_space_custody = ctx.w.module_state::<SpaceBubbleBook>().is_some_and(|b| {
                b.by_domain.values().any(|a| {
                    a.interdictor_hull == o.id
                        || a.picket
                            .as_ref()
                            .is_some_and(|p| p.corvette_hull == o.id || p.raider_hull == o.id)
                        || a.ship_hulls.values().any(|&h| h == o.id)
                }) || b.clamped_derelicts.values().any(|&dh| dh == o.id)
                    || b.active_tows.values().any(|t| t.derelict_hull == o.id)
                    || b.space_custody.contains_key(&o.id)
            });
            !in_space_custody
        })
        .map(|o| o.id);
    if let Some(hull) = existing {
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
            book.derelicts.remove(&hull);
        }
        return Some(hull);
    }
    ctx.w
        .create(mush_core::ObjKind::Thing, name, owner, None)
        .ok()
}

/// Destroy an unclaimed NPC picket/raider hull (`Thing` + `TacticalUnit` + `EconomyStore` asset)
/// when its bubble closes without a clamp, tow, or salvage claim (`SF.R20`, `35b` §5 #11).
pub(crate) fn destroy_unclaimed_npc_hull(ctx: &mut Ctx<'_>, npc_hull: ObjId) {
    let is_npc_hull = ctx.w.get(npc_hull).is_some_and(|o| {
        o.kind == mush_core::ObjKind::Thing
            && !o.flags.contains(mush_core::Flags::GOING)
            && ctx
                .w
                .get(o.owner)
                .is_some_and(|owner| owner.kind == mush_core::ObjKind::Npc)
    });
    if !is_npc_hull {
        return;
    }
    let retained = ctx.w.module_state::<SpaceBubbleBook>().is_some_and(|b| {
        b.by_domain.values().any(|a| {
            a.interdictor_hull == npc_hull
                || a.picket
                    .as_ref()
                    .is_some_and(|p| p.corvette_hull == npc_hull || p.raider_hull == npc_hull)
                || a.ship_hulls.values().any(|&h| h == npc_hull)
        }) || b.clamped_derelicts.values().any(|&dh| dh == npc_hull)
            || b.active_tows.values().any(|t| t.derelict_hull == npc_hull)
            || b.space_custody.contains_key(&npc_hull)
    });
    if retained {
        return;
    }
    let _ = mush_tactics::custody::check_in(ctx, npc_hull, None);
    let _ = mush_sim::deployed::clear(ctx.w, npc_hull);
    if ctx.w.destroy(npc_hull).is_ok() {
        ctx.emit_world_event(&mush_core::WorldEvent::ObjDestroyed { obj: npc_hull });
    }
}
