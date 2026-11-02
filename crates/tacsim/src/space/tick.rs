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
use mush_core::{ClockStamp, Ctx, ObjId, SceneId, World};
use mush_sim::{
    ActivationBook, CarrierJourney, DomainId, JourneyBook, JourneyState, PromotionTrigger,
    SpatialState, promote_carrier_journey, resume_carrier_journey, stand_down_domain,
};

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

/// Active interdiction bubble bound to a promoted [`CarrierJourney`].
#[derive(Clone, Debug)]
pub struct ActiveInterdictionBubble {
    /// Carrier hull root object id.
    pub hull: ObjId,
    /// Hostile interdictor hull object id.
    pub interdictor_hull: ObjId,
    /// Promoted waypoint domain id.
    pub domain: DomainId,
    /// Carrier ship id inside [`Self::bubble`].
    pub carrier_ship_id: String,
    /// Hostile interdictor ship id inside [`Self::bubble`].
    pub interdictor_ship_id: String,
    /// 1-based leg cursor at which the carrier was promoted.
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
}

impl ActiveInterdictionBubble {
    /// Compute the adaptive 1:1 wall-clock step duration (`500 ms`, `1,000 ms`, or `5,000 ms`)
    /// from the bubble's live ordnance and hostile posture (`Q-35-4(1)`).
    #[must_use]
    pub fn adaptive_step_wall_ms(&self) -> u32 {
        if !self.bubble.torpedoes.is_empty() || !self.bubble.railgun_slugs.is_empty() {
            return ORDNANCE_STEP_WALL_MS;
        }
        let has_operational_hostile = self.bubble.ships.values().any(|ship| {
            ship.status == HullStatus::Operational
                && self.bubble.party_relation(CARRIER_PARTY, ship.party) == SpaceRelation::Hostile
        });
        if has_operational_hostile {
            COMBAT_STEP_WALL_MS
        } else {
            CRUISE_STEP_WALL_MS
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
        if self.cleared {
            return true;
        }
        if !self.bubble.torpedoes.is_empty() || !self.bubble.railgun_slugs.is_empty() {
            return false;
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
#[derive(Clone, Debug)]
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
}

/// Open-epoch derelict hull record persisted when a ship's drive or hull is disabled
/// in a space bubble (`SF.R2`, `D-35-12`).
#[derive(Clone, Debug)]
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
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

        // Timed burn cutoff (`35a` §5 #4): apply thrust only for the remaining burn duration.
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

        if self.burn_remaining_ic_s.is_none() && burn_s < dt_ic_s {
            self.thrust_mmps2 = [0, 0, 0];
        } else if self.burn_remaining_ic_s.is_none() && burn_s == dt_ic_s {
            // If this was a timed burn that finished exactly on `dt_ic_s`, clear thrust.
            // (Continuous burns via `+helm/thrust` leave `burn_remaining_ic_s = None` from the start,
            // whereas `+helm/burn` always sets `burn_remaining_ic_s = Some(dur)`.)
        }
        self.last_ic_s = self.last_ic_s.saturating_add(dt_ic_s);
    }
}

/// Live reaction-mass ledger for a hull in free flight or an interdiction bubble (`SF.R4`, `D-35-6`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
/// and live propellant ledgers (`SF.R1`–`SF.R4`).
#[derive(Clone, Debug, Default)]
pub struct SpaceBubbleBook {
    /// Active interdiction bubbles keyed by carrier hull root [`ObjId`].
    pub by_hull: BTreeMap<ObjId, ActiveInterdictionBubble>,
    /// Completed interdiction bubbles keyed by carrier hull root [`ObjId`].
    pub completed: BTreeMap<ObjId, CompletedInterdictionBubble>,
    /// Open-epoch derelict hulls keyed by hull [`ObjId`] (`D-35-12`).
    pub derelicts: BTreeMap<ObjId, SpaceDerelictRecord>,
    /// Validated `mush-sim` continuous standing orders (`Heading`, `Thrust`, `Speed`, `Posture`)
    /// keyed by `(hull, channel)` (`SF.R3`, `D-35-9`).
    pub standing_orders: BTreeMap<mush_sim::tick::StandingOrderKey, mush_sim::tick::StandingOrder>,
    /// Tier-1 interplanetary free-flight records keyed by carrier hull root [`ObjId`] (`SF.R4`, `D-35-5`, `D-35-6`).
    pub free_flights: BTreeMap<ObjId, SpaceFreeFlightRecord>,
    /// Live reaction-mass ledgers keyed by carrier hull root [`ObjId`] (`SF.R4`, `D-35-6`).
    pub propellant_ledgers: BTreeMap<ObjId, HullPropellantLedger>,
    /// Server-authenticated in-flight reaction-mass burns queued for `tacsim_port_burn` (`SF.R4`, `D-35-6`).
    pub pending_space_burns: Vec<crate::port::burn::PendingSpaceBurn>,
    /// Monotonic sequence allocator for [`Self::pending_space_burns`].
    pub next_space_burn_seq: u32,
    /// Monotonic revision counter for `mush-sim` standing orders (`SF.R3`).
    pub next_order_revision: u64,
    /// Monotonic allocator for space salvage [`RecoveryOpId`]s.
    pub next_recovery_op_id: u64,
}

impl SpaceBubbleBook {
    /// Borrow the active interdiction bubble for `hull`, if any.
    #[must_use]
    pub fn active_for_hull(&self, hull: ObjId) -> Option<&ActiveInterdictionBubble> {
        self.by_hull.get(&hull)
    }

    /// Mutably borrow the active interdiction bubble for `hull`, if any.
    pub fn active_for_hull_mut(&mut self, hull: ObjId) -> Option<&mut ActiveInterdictionBubble> {
        self.by_hull.get_mut(&hull)
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
    let Some(active) = book.by_hull.get_mut(&hull) else {
        return Vec::new();
    };
    let events = active.tape.record_and_apply(&mut active.bubble, cmd);
    active.snapshots.push(active.bubble.snapshot());
    if events.is_empty() {
        return events;
    }
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
    let interdictor_hull = active.interdictor_hull;
    let carrier_ship_id = active.carrier_ship_id.clone();
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
    let Some(active) = book.by_hull.get_mut(&hull) else {
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
        .is_some_and(TacsimStore::enabled)
    {
        return;
    }
    let now = ctx.w.world_ic_now();
    clear_resolved_bubbles(ctx, &now);
    settle_disabled_carrier_bubbles(ctx, &now);
    step_active_bubbles(ctx, &now);
    step_free_flight_records(ctx, &now);
    promote_due_interdictions(ctx, &now);
}

fn real_now_secs(w: &World, now: &ClockStamp) -> i64 {
    let wall_ms = w.now();
    if wall_ms > 0 {
        (wall_ms / 1_000).max(1)
    } else {
        (now.at / 24).max(1)
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
    let cleared_hulls: Vec<ObjId> = book
        .by_hull
        .iter()
        .filter(|(_, active)| active.is_clear_to_resume())
        .map(|(&hull, _)| hull)
        .collect();
    if cleared_hulls.is_empty() {
        return;
    }

    let mut to_finalize = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        for hull in cleared_hulls {
            let Some(active) = book.by_hull.remove(&hull) else {
                continue;
            };
            let carrier_ship = active.bubble.ships.get(&active.carrier_ship_id).cloned();
            let interdictor_ship = active
                .bubble
                .ships
                .get(&active.interdictor_ship_id)
                .cloned();
            let domain = active.domain;
            let interdictor_hull = active.interdictor_hull;
            let completed = CompletedInterdictionBubble {
                hull,
                domain,
                bubble_id: active.bubble.id.clone(),
                opened_ic_s: active.opened_ic_s,
                closed_ic_s: now.at,
                wall_elapsed_ms: active.bubble.wall_elapsed_ms,
                steps: active.steps,
                tape: active.tape,
                snapshots: active.snapshots,
            };
            book.completed.insert(hull, completed);
            to_finalize.push((
                hull,
                interdictor_hull,
                domain,
                carrier_ship,
                interdictor_ship,
            ));
        }
    }

    let filed_real_s = real_now_secs(ctx.w, now);
    for (hull, interdictor_hull, domain, carrier_ship, interdictor_ship) in to_finalize {
        if let Some(ship) = &carrier_ship {
            let _ = check_in_space_unit(ctx, hull, ship, now.at);
        }

        if let Some(iship) = &interdictor_ship
            && let HullStatus::DisabledDerelict {
                epoch_open,
                life_support_remaining_s,
                salvage_contest_window_s,
            } = iship.status
        {
            let prior_owner = ObjId(interdictor_hull.0.saturating_add(1_000));
            let base_obj = ObjId(domain.0.try_into().unwrap_or(1u32).max(1));
            let title_status = open_salvage_claim_for_derelict(
                ctx.w,
                interdictor_hull,
                hull,
                prior_owner,
                base_obj,
                now.at,
                filed_real_s,
            );
            if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                book.derelicts.insert(
                    interdictor_hull,
                    SpaceDerelictRecord {
                        hull: interdictor_hull,
                        domain,
                        disabled_at_ic_s: now.at,
                        epoch_open,
                        life_support_remaining_s,
                        salvage_contest_window_s,
                        assessment: None,
                        salvage_title_status: title_status,
                    },
                );
            }
        }

        let _ = resume_carrier_journey(ctx, hull, now);
        let any_other_promoted = ctx.w.module_state::<JourneyBook>().is_some_and(|jb| {
            jb.journeys.values().any(|j| {
                j.root() != hull
                    && matches!(j.state(), JourneyState::Promoted { domain: d } if *d == domain)
            })
        });
        if !any_other_promoted {
            let _ = stand_down_domain(ctx, domain, now.clone());
        }
        let line =
            "[SPACE] Interdiction contact cleared; local bubble closed and transit burn resumed.";
        crate::port::voyage::narrate_hull(ctx, hull, line);
    }
}

fn settle_disabled_carrier_bubbles(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    let Some(book) = ctx.w.module_state::<SpaceBubbleBook>() else {
        return;
    };
    let disabled_hulls: Vec<ObjId> = book
        .by_hull
        .iter()
        .filter(|(_, active)| active.is_carrier_disabled_and_settled())
        .map(|(&hull, _)| hull)
        .collect();
    if disabled_hulls.is_empty() {
        return;
    }

    let mut settled = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        for hull in disabled_hulls {
            let Some(active) = book.by_hull.remove(&hull) else {
                continue;
            };
            let carrier_ship = active.bubble.ships.get(&active.carrier_ship_id).cloned();
            let domain = active.domain;
            let interdictor_hull = active.interdictor_hull;
            let completed = CompletedInterdictionBubble {
                hull,
                domain,
                bubble_id: active.bubble.id.clone(),
                opened_ic_s: active.opened_ic_s,
                closed_ic_s: now.at,
                wall_elapsed_ms: active.bubble.wall_elapsed_ms,
                steps: active.steps,
                tape: active.tape,
                snapshots: active.snapshots,
            };
            book.completed.insert(hull, completed);
            settled.push((hull, interdictor_hull, domain, carrier_ship));
        }
    }

    let filed_real_s = real_now_secs(ctx.w, now);
    for (hull, interdictor_hull, domain, carrier_ship) in settled {
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
        let base_obj = ObjId(domain.0.try_into().unwrap_or(1u32).max(1));
        let title_status = open_salvage_claim_for_derelict(
            ctx.w,
            hull,
            interdictor_hull,
            hull,
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
        let line = "[SPACE] Drive offline; hull adrift as an open-epoch derelict with a 48-hour Registry salvage window.";
        crate::port::voyage::narrate_hull(ctx, hull, line);
    }
}

fn step_active_bubbles(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    let frozen = ctx.w.world_timeline().is_frozen();
    let mut combat_reports = Vec::new();

    let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() else {
        return;
    };
    if book.by_hull.is_empty() {
        return;
    }

    for (&hull, active) in &mut book.by_hull {
        active.bubble.outage_frozen = frozen;
        if frozen {
            continue;
        }
        let dt_wall_ms = active.adaptive_step_wall_ms();
        let events = active.tape.record_and_apply(
            &mut active.bubble,
            BubbleInputCommand::StepWall {
                dt_wall_ms,
                ic_now_s: now.at,
            },
        );
        active.snapshots.push(active.bubble.snapshot());
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
    let hulls: Vec<(ObjId, mush_core::CanonicalId)> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| {
            b.free_flights
                .iter()
                .map(|(&hull, rec)| (hull, rec.target_place.clone()))
                .collect()
        })
        .unwrap_or_default();
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
}

fn promote_due_interdictions(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    let Some(journey_book) = ctx.w.module_state::<JourneyBook>().cloned() else {
        return;
    };
    let (already_open, already_derelict, in_free_flight): (Vec<ObjId>, Vec<ObjId>, Vec<ObjId>) =
        ctx.w
            .module_state::<SpaceBubbleBook>()
            .map(|b| {
                (
                    b.by_hull.keys().copied().collect(),
                    b.derelicts.keys().copied().collect(),
                    b.free_flights.keys().copied().collect(),
                )
            })
            .unwrap_or_default();

    let mut to_promote: Vec<(ObjId, DomainId, u32)> = Vec::new();
    for (&hull, journey) in &journey_book.journeys {
        if already_open.contains(&hull)
            || already_derelict.contains(&hull)
            || in_free_flight.contains(&hull)
        {
            continue;
        }
        if let Some(domain) = interdicted_domain_for_journey(ctx.w, journey) {
            to_promote.push((hull, domain, journey.cursor()));
        }
    }

    if to_promote.is_empty() {
        return;
    }

    for (hull, domain, cursor) in to_promote {
        if !promote_carrier_journey(ctx, hull, domain, now) {
            continue;
        }
        let Some(active) = build_interdiction_bubble(ctx, hull, domain, cursor, now.at) else {
            continue;
        };
        if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
            book.by_hull.insert(hull, active);
        }
        let line = "[SPACE] Interdiction contact detected at waypoint boundary; local space bubble opened on 1:1 wall clock.";
        crate::port::voyage::narrate_hull(ctx, hull, line);
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
    ic_now_s: i64,
) -> Option<ActiveInterdictionBubble> {
    let carrier_ship_id = format!("hull.{}", hull.0);
    let interdictor_ship_id = format!("picket.{}.{}", domain.0, leg_cursor);
    let site_id = format!("domain.{}", domain.0);

    let carrier_ship = build_carrier_ship_state(ctx, hull, &site_id, leg_cursor, &carrier_ship_id)?;
    let interdictor_hull = ObjId(
        900_000_u32
            .saturating_add(u32::try_from(domain.0).unwrap_or(0))
            .max(1),
    );
    let (interdictor_unit, _) =
        instantiate_stock_space_unit(interdictor_hull, "tacsim.unit.dray").ok()?;
    let mut interdictor_ship = space_ship_from_tactical_unit(
        &interdictor_unit,
        &interdictor_ship_id,
        "Interdictor Picket",
        INTERDICTOR_PARTY,
        [INTERDICTION_STANDOFF_M, 0, 0],
        [-20_000, 0, 0],
    )
    .ok()?;
    interdictor_ship.attitude_deg = (180, 0, 0);

    let bubble_id = format!("bubble.interdiction.{}.{}", hull.0, leg_cursor);
    let mut bubble =
        LocalSpaceBubble::new(bubble_id, site_id, "Waypoint Interdiction Bubble", ic_now_s);
    bubble.set_party_relation(CARRIER_PARTY, INTERDICTOR_PARTY, SpaceRelation::Hostile);
    bubble.insert_ship(carrier_ship);
    bubble.insert_ship(interdictor_ship);

    let initial_snapshot = bubble.snapshot();
    let tape = BubbleInputTape::new(&bubble);

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

    // Hydrate `propellant_kg` and `delta_v_mps` from the hull's live `stow.propellant` pool (`D-35-6`).
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
            ship.propellant_kg =
                u32::try_from(live_kg.min(u64::from(u32::MAX))).unwrap_or(u32::MAX);
            ship.delta_v_mps = i64::from(
                crate::port::passage::remaining_dv_mps_for_propellant(
                    &basis.spec.drive,
                    basis.spec.dry_kg,
                    non_prop_payload,
                    live_kg,
                )
                .unwrap_or(0),
            );
        }
    }
    Some(ship)
}
