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
use mush_sim::{
    ActivationBook, CarrierJourney, DomainId, JourneyBook, JourneyState, PromotionTrigger,
    SpatialState, promote_carrier_journey, resume_carrier_journey, stand_down_domain,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::bubble::{
    BubbleId, BubbleInputCommand, BubbleInputTape, BubbleSnapshot, DERELICT_WINDOW_S, HullStatus,
    LocalSpaceBubble, SpacePartyId, SpaceRelation, SpaceShipState,
};
use super::custody::{
    check_in_space_unit, check_out_space_unit, instantiate_stock_space_unit,
    record_space_combat_to_black_box, register_derelict_salvage_in_book,
    space_ship_from_tactical_unit,
};
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
pub const CARRIER_PARTY: SpacePartyId = SpacePartyId(1);

/// Party identifier for the interdicting hostile picket inside an interdiction bubble.
pub const INTERDICTOR_PARTY: SpacePartyId = SpacePartyId(2);

/// Default standoff distance (`18,000 m`) at which an interdicting picket opens the bubble.
pub const INTERDICTION_STANDOFF_M: i64 = 18_000;

/// Maximum number of snapshots retained per active bubble (`SF.R10`, `35b` §5 #9).
pub const MAX_BUBBLE_SNAPSHOTS: usize = 6;

/// Maximum number of input tape entries retained per active bubble before rolling the checkpoint (`SF.R10`, `35b` §5 #9).
pub const MAX_BUBBLE_TAPE_ENTRIES: usize = 64;

/// Maximum number of completed bubble summary records retained in [`SpaceBubbleBook::completed`] (`SF.R10`, `35b` §5 #9).
pub const MAX_COMPLETED_BUBBLES: usize = 8;

/// Per-carrier entry inside a domain-keyed [`ActiveInterdictionBubble`] (`SF.R12`, `D-35-5`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BubbleCarrierEntry {
    /// Carrier hull root object id.
    pub hull: ObjId,
    /// Carrier ship id inside [`ActiveInterdictionBubble::bubble`].
    pub ship_id: String,
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
        if let Some(c) = self.carriers.get_mut(&hull) {
            c.holding_for_salvage = val;
        }
        if hull == self.hull || self.carriers.is_empty() {
            self.holding_for_salvage = val;
        } else {
            self.holding_for_salvage = self.carriers.values().any(|c| c.holding_for_salvage);
        }
    }
    /// Compute the adaptive 1:1 wall-clock cadence period (`500 ms`, `1,000 ms`, or `5,000 ms`)
    /// from the bubble's live ordnance and hostile posture (`Q-35-4(1)`, `SF.R10`).
    #[must_use]
    pub fn adaptive_step_wall_ms(&self) -> u32 {
        if !self.bubble.torpedoes.is_empty() || !self.bubble.railgun_slugs.is_empty() {
            return ORDNANCE_STEP_WALL_MS;
        }
        let has_operational_hostile = self.bubble.ships.values().any(|ship| {
            ship.status == HullStatus::Operational
                && self.bubble.party_relation(CARRIER_PARTY, ship.party) == SpaceRelation::Hostile
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

    /// Return whether the encounter is clear for the carrier to resume its [`CarrierJourney`].
    #[must_use]
    pub fn is_clear_to_resume(&self) -> bool {
        let carrier_operational = self
            .bubble
            .ships
            .get(&self.carrier_ship_id)
            .is_some_and(|ship| ship.status == HullStatus::Operational);
        if !carrier_operational {
            return false;
        }
        if self.approaching_derelict
            || self.holding_for_salvage
            || self
                .carriers
                .values()
                .any(|c| c.approaching_derelict || c.holding_for_salvage)
        {
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
            if corvette_operational && !picket.is_customs_cleared() {
                return false;
            }
        }
        !self.bubble.ships.values().any(|ship| {
            ship.status == HullStatus::Operational
                && self.bubble.party_relation(CARRIER_PARTY, ship.party) == SpaceRelation::Hostile
        })
    }

    /// Return whether the carrier itself has been disabled into an open-epoch derelict
    /// (`D-35-12`) and all in-flight ordnance has resolved.
    #[must_use]
    pub fn is_carrier_disabled_and_settled(&self) -> bool {
        if !self.bubble.torpedoes.is_empty() || !self.bubble.railgun_slugs.is_empty() {
            return false;
        }
        self.bubble
            .ships
            .get(&self.carrier_ship_id)
            .is_some_and(|ship| matches!(ship.status, HullStatus::DisabledDerelict { .. }))
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

/// World-state registry of active and completed space interdiction bubbles,
/// open-epoch derelicts, validated `mush-sim` standing orders, Tier-1 free-flight records,
/// live propellant ledgers, orbital cluster mining sessions, and 6-DOF docking sessions (`SF.R1`–`SF.R8`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceBubbleBook {
    /// Active interdiction bubbles keyed by waypoint [`DomainId`] (`SF.R12`, `D-35-5`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub by_domain: BTreeMap<DomainId, ActiveInterdictionBubble>,
    /// Carrier legs `(hull, departed_at_ic_s, leg_cursor)` that have already cleared an interdiction bubble (`SF.R12`).
    #[serde(default)]
    pub cleared_interdiction_legs: std::collections::BTreeSet<(ObjId, i64, u32)>,
    /// Completed interdiction bubbles keyed by carrier hull root [`ObjId`].
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub completed: BTreeMap<ObjId, CompletedInterdictionBubble>,
    /// Open-epoch derelict hulls keyed by hull [`ObjId`] (`D-35-12`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub derelicts: BTreeMap<ObjId, SpaceDerelictRecord>,
    /// Validated `mush-sim` continuous standing orders (`Heading`, `Thrust`, `Speed`, `Posture`)
    /// keyed by `(hull, channel)` (`SF.R3`, `D-35-9`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub standing_orders: BTreeMap<mush_sim::tick::StandingOrderKey, mush_sim::tick::StandingOrder>,
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
    /// Laid-up orbital cluster sites (`ceres_halo`, `kirkwood_gap_3_1`, `pallas_inclined_belt`, `saturn_b_ring_edge`) (`SF.R5`, `D-35-13`).
    #[serde(default, with = "super::bubble::vec_tuple_map")]
    pub cluster_sites: BTreeMap<super::mining::OrbitalClusterId, super::mining::ClusterSiteState>,
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
    /// Monotonic revision counter for `mush-sim` standing orders (`SF.R3`).
    #[serde(default)]
    pub next_order_revision: u64,
    /// Monotonic allocator for space salvage [`RecoveryOpId`]s.
    #[serde(default)]
    pub next_recovery_op_id: u64,
}

impl SpaceBubbleBook {
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
    /// a multi-carrier domain bubble (`SF.R12`).
    pub fn remove_carrier_from_active(&mut self, hull: ObjId) -> Option<ActiveInterdictionBubble> {
        let domain = self
            .by_domain
            .iter()
            .find_map(|(&dom, a)| a.contains_carrier(hull).then_some(dom))?;
        let mut active = self.by_domain.remove(&domain)?;
        if active.carriers.len() > 1 {
            if let Some(removed) = active.carriers.remove(&hull) {
                active.bubble.ships.remove(&removed.ship_id);
                active.ship_hulls.remove(&removed.ship_id);
            }
            if active.hull == hull
                && let Some(first) = active.carriers.values().next().cloned()
            {
                active.hull = first.hull;
                active.carrier_ship_id = first.ship_id;
                active.leg_cursor = first.leg_cursor;
            }
            let snapshot = active.clone();
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

    /// Borrow the current validated [`mush_sim::tick::StandingOrder`] for `(unit, channel)` (`D-35-9`).
    #[must_use]
    pub fn standing_order(
        &self,
        unit: ObjId,
        channel: mush_sim::tick::OrderChannel,
    ) -> Option<&mush_sim::tick::StandingOrder> {
        self.standing_orders
            .get(&mush_sim::tick::StandingOrderKey { unit, channel })
    }

    /// Allocate the next monotonic standing-order revision and record a validated
    /// [`mush_sim::tick::StandingOrder`] on `(unit, channel)` (`SF.R3`, `D-35-9`).
    ///
    /// # Errors
    ///
    /// Returns [`mush_sim::tick::TickRecordError`] if the order value or stamp fails
    /// `mush-sim`'s [`mush_sim::tick::StandingOrder::new`] invariant checks.
    #[allow(clippy::too_many_arguments)]
    pub fn record_standing_order(
        &mut self,
        unit: ObjId,
        channel: mush_sim::tick::OrderChannel,
        value: mush_sim::tick::OrderValue,
        by: ObjId,
        domain: DomainId,
        generation: u64,
        tick: i64,
    ) -> Result<mush_sim::tick::StandingOrder, mush_sim::tick::TickRecordError> {
        let next_rev = self.next_order_revision.saturating_add(1);
        let stamp = ClockStamp {
            clock: mush_core::ClockRef::DomainTick {
                owner: mush_core::OwnerId::new("sim")
                    .map_err(|_| mush_sim::tick::TickRecordError::NotDomainTick)?,
                id: mush_core::CanonicalId::new(format!("domain_{}", domain.0))
                    .map_err(|_| mush_sim::tick::TickRecordError::NotDomainTick)?,
                generation,
            },
            at: tick,
        };
        let order = mush_sim::tick::StandingOrder::new(unit, channel, value, stamp, by, next_rev)?;
        self.next_order_revision = next_rev;
        self.standing_orders.insert(order.key(), order.clone());
        Ok(order)
    }

    fn alloc_recovery_op_id(&mut self) -> Option<RecoveryOpId> {
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
    for ship in active.bubble.ships.values_mut() {
        if ship.party != CARRIER_PARTY && ship.status == HullStatus::Operational {
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

/// Store document key under which [`SpacePersistedState`] is persisted in the `"tacsim"` namespace (`SF.R8`, `D-35-12`).
pub const SPACE_STATE_KEY: &str = "tacsim:space:state";

/// Schema format version for [`SpacePersistedState`].
pub const SPACE_STATE_FORMAT: u16 = 1;

/// Durable store snapshot of [`SpaceBubbleBook`] and [`SalvageClaimBook`] (`SF.R8`, `D-35-12`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpacePersistedState {
    /// Schema format version ([`SPACE_STATE_FORMAT`]).
    pub format: u16,
    /// Persisted [`SpaceBubbleBook`] state.
    pub book: SpaceBubbleBook,
    /// Persisted [`SalvageClaimBook`] state (48-hour salvage claims, recovery records, black-box tapes).
    pub salvage: SalvageClaimBook,
}

/// Persist the current [`SpaceBubbleBook`] and [`SalvageClaimBook`] into [`TacsimStore`] and
/// emit a [`ModuleWrite::JsonSet`] document under [`SPACE_STATE_KEY`] (`SF.R8`, `D-35-12`).
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
    let snapshot = SpacePersistedState {
        format: SPACE_STATE_FORMAT,
        book,
        salvage,
    };
    let unchanged = match ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::space_state)
    {
        Some(prev) => *prev == snapshot,
        None => snapshot.book == SpaceBubbleBook::default(),
    };
    if unchanged {
        return;
    }
    match serde_json::to_value(&snapshot) {
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
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.space_state = Some(snapshot);
        store.space_state_held = true;
    }
}

/// Hydrate [`SpaceBubbleBook`], [`SalvageClaimBook`], and [`TacsimStore::space_state`] from the
/// `"tacsim"` boot key-value documents during `TacsimRuleset::on_world_loaded` (`SF.R8`, `D-35-12`).
pub fn boot_read(ctx: &mut Ctx<'_>, kv: &[(SmolStr, serde_json::Value)]) {
    for (key, value) in kv {
        if key.as_str() != SPACE_STATE_KEY {
            continue;
        }
        if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
            store.space_state_held = true;
        }
        match serde_json::from_value::<SpacePersistedState>(value.clone()) {
            Ok(state) if state.format == SPACE_STATE_FORMAT => {
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
            Ok(state) => {
                if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
                    store.space_state = None;
                }
                tracing::warn!(
                    format = state.format,
                    "tacsim: tacsim:space:state has unsupported format; held for staff repair"
                );
            }
            Err(error) => {
                if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
                    store.space_state = None;
                }
                tracing::warn!(
                    %error,
                    "tacsim: tacsim:space:state did not decode; held for staff repair"
                );
            }
        }
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

/// Register [`SpaceBubbleBook`], [`SalvageClaimBook`], and the [`SPACE_TICK`] consumer on `reg`.
///
/// # Errors
///
/// Propagates any duplicate-registration error from [`Registrar::tick`].
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(SpaceBubbleBook::default());
    reg.state(SalvageClaimBook::new());
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
    expire_stale_docking_sessions(ctx, &now);
    step_derelict_records(ctx);
    clear_resolved_bubbles(ctx, &now);
    settle_disabled_carrier_bubbles(ctx, &now);
    step_active_bubbles(ctx, &now);
    step_free_flight_records(ctx, &now);
    super::mining::step_clusters_on_tick(ctx, now.clone());
    promote_due_interdictions(ctx, &now);
    persist_space_state(ctx);
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
        let vault_tank_t = crate::port::ship::hull_view(ctx.w, hull)
            .and_then(|v| crate::port::ship::hold_vault(ctx.w, &v))
            .and_then(|vid| {
                let good = mush_core::CanonicalId::new("propellant").ok()?;
                crate::port_tick::read_pool(ctx, mush_inventory::record::EndpointRef(vid), &good)
                    .ok()
            })
            .unwrap_or(0);
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
            book.docking_sessions.remove(&hull);
            let _ = book
                .propellant_ledger_mut(hull)
                .record_burn_kg(vault_tank_t, 15);
        }
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

fn real_now_secs(w: &World, _now: &ClockStamp) -> i64 {
    let wall_s = w.now();
    if wall_s > 0 { wall_s } else { 1 }
}

/// Resolve the real world owner or charterer [`ObjId`] of `hull` for salvage claim filing (`SF.R12`, `35b` §5 #7).
#[must_use]
pub(crate) fn resolve_hull_prior_owner(w: &World, hull: ObjId, recoverer_hull: ObjId) -> ObjId {
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

/// Resolve a real world base/port [`ObjId`] for salvage claim filing at `domain` (`SF.R12`, `35b` §5 #7).
#[must_use]
pub(crate) fn resolve_nearest_port_base_obj(
    w: &World,
    domain: DomainId,
    recoverer_hull: ObjId,
) -> ObjId {
    if let Some(host) = w
        .module_state::<SpatialState>()
        .and_then(|s| s.domains.get(&domain))
        .map(|d| d.host)
        && w.get(host).is_some()
        && host != recoverer_hull
    {
        return host;
    }
    for (_, map) in crate::port::voyage::lane_ports(w) {
        if let Some(host_raw) = map.and_then(|m| m.host) {
            let host = ObjId(host_raw);
            if w.get(host).is_some() && host != recoverer_hull {
                return host;
            }
        }
    }
    if mush_core::GOD != recoverer_hull {
        mush_core::GOD
    } else {
        ObjId(2)
    }
}

fn open_salvage_claim_for_derelict(
    w: &mut World,
    derelict_obj: ObjId,
    recoverer_hull: ObjId,
    prior_owner_obj: ObjId,
    port_base_obj: ObjId,
    at_ic: i64,
    filed_at_real_s: i64,
) -> Option<CustodyTitleStatus> {
    let op_id = w
        .module_state_mut::<SpaceBubbleBook>()?
        .alloc_recovery_op_id()?;
    let salvage_book = w.module_state_mut::<SalvageClaimBook>()?;
    register_derelict_salvage_in_book(
        salvage_book,
        op_id,
        derelict_obj,
        recoverer_hull,
        recoverer_hull,
        prior_owner_obj,
        port_base_obj,
        at_ic,
        filed_at_real_s,
    )
    .ok()
}

fn clear_resolved_bubbles(ctx: &mut Ctx<'_>, now: &ClockStamp) {
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
                    book.cleared_interdiction_legs.insert((ch, dep_ic, cleg));
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

    let filed_real_s = real_now_secs(ctx.w, now);
    for (primary_hull, domain, carrier_entries, npc_contacts) in to_finalize {
        for (ch, cship, _, _) in &carrier_entries {
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
            {
                let prior_owner = resolve_hull_prior_owner(ctx.w, contact_hull, primary_hull);
                let base_obj = resolve_nearest_port_base_obj(ctx.w, domain, primary_hull);
                let title_status = open_salvage_claim_for_derelict(
                    ctx.w,
                    contact_hull,
                    primary_hull,
                    prior_owner,
                    base_obj,
                    now.at,
                    filed_real_s,
                );
                if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
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
                            salvage_title_status: title_status,
                        },
                    );
                }
            }
        }

        for (ch, _, _, _) in &carrier_entries {
            let _ = resume_carrier_journey(ctx, *ch, now);
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
            let hull = active.hull;
            let carrier_ship = active.bubble.ships.get(&active.carrier_ship_id).cloned();
            let interdictor_hull = active.interdictor_hull;
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
            let completed = CompletedInterdictionBubble::from_closed(&active, now.at);
            if book.completed.len() >= MAX_COMPLETED_BUBBLES
                && !book.completed.contains_key(&hull)
                && let Some(&oldest_hull) = book.completed.keys().next()
            {
                book.completed.remove(&oldest_hull);
            }
            book.completed.insert(hull, completed);
            if book.pending_space_burns.len() > 16 {
                let excess = book.pending_space_burns.len() - 16;
                book.pending_space_burns.drain(0..excess);
            }
            settled.push((hull, interdictor_hull, domain, carrier_ship, npc_contacts));
        }
    }

    let filed_real_s = real_now_secs(ctx.w, now);
    for (hull, interdictor_hull, domain, carrier_ship, npc_contacts) in settled {
        for (npc_hull, npc_ship) in npc_contacts {
            let _ = check_in_space_unit(ctx, npc_hull, &npc_ship, now.at);
        }
        let Some(ship) = carrier_ship else {
            continue;
        };
        let assessment = check_in_space_unit(ctx, hull, &ship, now.at).ok();
        let (epoch_open, life_support_remaining_s, salvage_contest_window_s) = match ship.status {
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
        let prior_owner = resolve_hull_prior_owner(ctx.w, hull, interdictor_hull);
        let base_obj = resolve_nearest_port_base_obj(ctx.w, domain, interdictor_hull);
        let title_status = open_salvage_claim_for_derelict(
            ctx.w,
            hull,
            interdictor_hull,
            prior_owner,
            base_obj,
            now.at,
            filed_real_s,
        );
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
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
                    salvage_title_status: title_status,
                },
            );
        }
        let _ = mush_sim::fail_carrier_journey(
            ctx,
            hull,
            "carrier hull disabled into an open-epoch derelict",
        );
        let any_other_promoted = ctx.w.module_state::<JourneyBook>().is_some_and(|jb| {
            jb.journeys
                .values()
                .any(|j| matches!(j.state(), JourneyState::Promoted { domain: d } if *d == domain))
        });
        if !any_other_promoted {
            let _ = stand_down_domain(ctx, domain, now.clone());
        }
        let line = "[SPACE] Drive offline; hull adrift as an open-epoch derelict with a 48-hour Registry salvage window.";
        crate::port::voyage::narrate_hull(ctx, hull, line);
    }
}

fn step_active_bubbles(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    let frozen = ctx.w.world_timeline().is_frozen();
    let wall_now_s = ctx.w.now();
    let mut combat_reports = Vec::new();
    let mut ran_picket_hulls = Vec::new();

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
        let elapsed_wall_s =
            if active.last_stepped_wall_s > 0 && wall_now_s > active.last_stepped_wall_s {
                wall_now_s - active.last_stepped_wall_s
            } else {
                1
            };
        if cadence_ms == CRUISE_STEP_WALL_MS
            && active.last_stepped_wall_s > 0
            && wall_now_s > active.last_stepped_wall_s
            && elapsed_wall_s * 1_000 < i64::from(CRUISE_STEP_WALL_MS)
        {
            continue;
        }
        active.last_stepped_wall_s = wall_now_s;
        let dt_wall_ms = if (1..=10).contains(&elapsed_wall_s) {
            u32::try_from(elapsed_wall_s * 1_000).unwrap_or(1_000)
        } else {
            1_000
        };
        let mut events = active.tape.record_and_apply(
            &mut active.bubble,
            BubbleInputCommand::StepWall {
                dt_wall_ms,
                ic_now_s: now.at,
            },
        );
        if let Some(picket) = active.picket.as_mut() {
            let picket_events = super::picket::step_kirkwood_picket(
                &mut active.bubble,
                &mut active.tape,
                picket,
                &active.carrier_ship_id,
            );
            events.extend(picket_events);
            if picket.inspection_status == super::picket::PicketInspectionStatus::RanThePicket
                && picket.inspection_outcome.is_none()
            {
                ran_picket_hulls.push(hull);
            }
        } else {
            let picket_events = step_interdictor_picket(
                &mut active.bubble,
                &mut active.tape,
                active.interdictor_hull,
                &active.interdictor_ship_id,
                &active.carrier_ship_id,
                &mut active.interdictor_think_seq,
                &mut active.interdictor_memory,
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
            picket.inspection_outcome =
                Some(super::picket::PicketInspectionOutcome::RanThePicket { front_id });
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
                rec.last_ic_s = rec.last_ic_s.max(now.at);
            }
        }
    }
    if hulls.is_empty() {
        return;
    }

    let mut target_vels: BTreeMap<ObjId, Option<[i64; 3]>> = BTreeMap::new();
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
            }
        }
    }

    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        for (hull, _) in hulls {
            let tvel = target_vels.get(&hull).copied().flatten();
            if let Some(rec) = book.free_flights.get_mut(&hull) {
                let dt_ic_s = now.at.saturating_sub(rec.last_ic_s);
                if dt_ic_s > 0 {
                    rec.step_ic_s(dt_ic_s, tvel);
                }
            }
        }
    }

    // Promote any free-flight hulls that have reached their destination orbital cluster (`SF.R9`, `D-35-5`):
    let arrived_clusters: Vec<(ObjId, super::mining::OrbitalClusterId)> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            b.free_flights
                .iter()
                .filter_map(|(&hull, rec)| {
                    let cluster =
                        super::mining::OrbitalClusterId::parse(rec.target_place.as_str())?;
                    let arrived = rec
                        .cluster_arrival_ic_s
                        .is_some_and(|arr_ic| now.at >= arr_ic);
                    arrived.then_some((hull, cluster))
                })
                .collect()
        })
        .unwrap_or_default();
    for (hull, cluster) in arrived_clusters {
        let _ = super::mining::ensure_cluster_laid_up(ctx, cluster, hull);
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
            book.free_flights.remove(&hull);
            let existing_drones = book
                .cluster_sessions
                .get(&hull)
                .map(|s| s.drone_things.clone())
                .unwrap_or_default();
            let mut session = super::mining::ActiveClusterMiningSession::new(hull, cluster, now.at);
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
}

fn step_interdictor_picket(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    interdictor_hull: ObjId,
    interdictor_ship_id: &str,
    carrier_ship_id: &str,
    think_seq: &mut u64,
    memory: &mut Vec<u8>,
) -> Vec<super::bubble::SpaceCombatEvent> {
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
    if let Some((res, _)) = super::picket::run_space_rhai_think(
        bubble,
        interdictor_ship_id,
        interdictor_hull,
        super::picket::INTERDICTOR_PATROL_RHAI,
        *think_seq,
        memory,
    ) && res.fault.is_none()
    {
        *think_seq = think_seq.saturating_add(1);
        *memory = res.memory;
        if res
            .trace
            .iter()
            .any(|t| t == "INTERDICTOR_STAND_DOWN_CLEARED")
        {
            bubble.set_party_relation(CARRIER_PARTY, INTERDICTOR_PARTY, SpaceRelation::Neutral);
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
    }
    if bubble
        .ships
        .get(interdictor_ship_id)
        .is_some_and(|s| s.locked_target_id.as_deref() != Some(carrier_ship_id))
    {
        emitted.extend(tape.record_and_apply(
            bubble,
            BubbleInputCommand::SetLockedTarget {
                ship_id: interdictor_ship_id.to_owned(),
                target_id: Some(carrier_ship_id.to_owned()),
            },
        ));
    }
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
    emitted.extend(tape.record_and_apply(
        bubble,
        BubbleInputCommand::FireOffensiveLasers {
            shooter_id: interdictor_ship_id.to_owned(),
            target_id: carrier_ship_id.to_owned(),
        },
    ));
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
            || cleared_legs.contains(&(hull, dep_at, cursor))
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
            let Some(mut carrier_ship) =
                build_carrier_ship_state(ctx, hull, &site_id, cursor, &carrier_ship_id)
            else {
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
                active.carriers.insert(
                    hull,
                    BubbleCarrierEntry {
                        hull,
                        ship_id: carrier_ship_id.clone(),
                        leg_cursor: cursor,
                        departed_at_ic_s: dep_at,
                        last_synced_propellant_kg: initial_prop_kg,
                        approaching_derelict: false,
                        holding_for_salvage: false,
                    },
                );
                active.ship_hulls.insert(carrier_ship_id, hull);
                active.bubble.insert_ship(carrier_ship);
            }
            let line = "[SPACE] Interdiction contact detected at waypoint boundary; joined active domain space bubble on 1:1 wall clock.";
            crate::port::voyage::narrate_hull(ctx, hull, line);
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

    let carrier_ship = build_carrier_ship_state(ctx, hull, &site_id, leg_cursor, &carrier_ship_id)?;
    let open_kirkwood = super::picket::should_open_kirkwood_picket(ctx.w, hull, domain);

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

    let (interdictor_hull, interdictor_ship_id, picket) = if open_kirkwood {
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
        (raider_hull, raider_ship_id, Some(picket_state))
    } else {
        let interdictor_ship_id = format!("picket.{}.{}", domain.0, leg_cursor);
        let interdictor_hull = ctx
            .w
            .create(
                mush_core::ObjKind::Thing,
                "Interdictor Picket",
                mush_core::GOD,
                None,
            )
            .ok()?;
        super::custody::create_stock_space_unit_in_world(
            ctx,
            interdictor_hull,
            crate::definitions::content::CORVETTE,
        )
        .ok()?;
        let mut interdictor_ship = check_out_space_unit(
            ctx,
            interdictor_hull,
            &site_id,
            leg_cursor,
            &interdictor_ship_id,
            "Interdictor Picket",
            INTERDICTOR_PARTY,
            [INTERDICTION_STANDOFF_M, 0, 0],
            [-20_000, 0, 0],
        )
        .ok()?;
        interdictor_ship.attitude_deg = (180, 0, 0);
        interdictor_ship.emcon = super::bubble::EmconPosture::Ghost;
        interdictor_ship.heat_permille = 0;
        interdictor_ship.thrust_mps2 = [0, 0, 0];
        if let Some(def_laser) = interdictor_ship.lasers.get_mut(1) {
            def_laser.mode = super::weapons::LaserMode::Defense;
        }
        bubble.set_party_relation(CARRIER_PARTY, INTERDICTOR_PARTY, SpaceRelation::Hostile);
        bubble.insert_ship(interdictor_ship);
        ship_hulls.insert(interdictor_ship_id.clone(), interdictor_hull);
        (interdictor_hull, interdictor_ship_id, None)
    };

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
            leg_cursor,
            departed_at_ic_s,
            last_synced_propellant_kg: initial_prop_kg,
            approaching_derelict: false,
            holding_for_salvage: false,
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
    })
}

fn build_carrier_ship_state(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    site_id: &str,
    leg_cursor: u32,
    carrier_ship_id: &str,
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
            CARRIER_PARTY,
            [0, 0, 0],
            [25_000, 0, 0],
        ) {
            Some(ship)
        } else if let Some(unit) = mush_tactics::module::unit_of(ctx.w, hull) {
            space_ship_from_tactical_unit(
                unit,
                carrier_ship_id,
                callsign,
                CARRIER_PARTY,
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
            CARRIER_PARTY,
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
            if let Some(accel) = crate::port::passage::flip_accel_mps2(
                &basis.spec.drive,
                basis.spec.dry_kg,
                effective_payload,
            ) {
                ship.max_accel_mps2 = (accel.ceil() as i64).max(1);
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
    Some(ship)
}
