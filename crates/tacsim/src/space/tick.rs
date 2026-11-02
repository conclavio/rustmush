//! **World Tick Consumer & Interdiction Promotion Seam (`SF.R1`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §6.1, `D-35-3`, `D-35-5`,
//! `Q-35-4(1)`, `docs/design/35a-space-flight-review.md` §8.3).
//!
//! Connects [`LocalSpaceBubble`] to `mush-sim`'s [`CarrierJourney`] and
//! [`ActivationBook`] on the coarse world tick ([`Registrar::tick`]):
//! 1. When an under-way carrier crosses an interdicted leg boundary (`LegDef::interdiction > 0`)
//!    and `mush-sim` raises [`PromotionTrigger::Interception`] on the waypoint domain,
//!    [`space_tick`] promotes the [`CarrierJourney`] to [`JourneyState::Promoted`]
//!    (which pauses the carrier's leg timer while keeping its remaining leg duration intact)
//!    and opens a [`LocalSpaceBubble`] populated from canonical `UnitSpec` hulls.
//! 2. On each subsequent tick fire while the bubble is open, [`space_tick`] steps the
//!    bubble on **1:1 wall time** ([`LocalSpaceBubble::step_wall_ms_at`]) using the
//!    adaptive step cadence (`5,000 ms` cruise / `1,000 ms` combat / `500 ms` ordnance in flight)
//!    and stamps the live W-clock `now.at` without scaling physics by the world IC rate.
//! 3. When the encounter clears (all hostile contacts disabled/derelict or explicitly
//!    cleared, and no ordnance in flight), [`space_tick`] closes the bubble into
//!    [`SpaceBubbleBook::completed`], resumes the [`CarrierJourney`] to
//!    [`JourneyState::Cruising`], and stands down the domain activation.

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
    instantiate_stock_space_unit, record_space_combat_to_black_box, space_ship_from_tactical_unit,
};
use crate::field_ops::{OperationalState, SalvageClaimBook};
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
}

/// Summary record retained in [`SpaceBubbleBook::completed`] when an interdiction bubble clears.
#[derive(Clone, Debug)]
pub struct CompletedInterdictionBubble {
    /// Carrier hull root object id.
    pub hull: ObjId,
    /// Promoted waypoint domain id that stood down on clear.
    pub domain: DomainId,
    /// Bubble identifier.
    pub bubble_id: BubbleId,
    /// W-clock IC timestamp (seconds) when the bubble was opened.
    pub opened_ic_s: i64,
    /// W-clock IC timestamp (seconds) when the bubble cleared and the leg resumed.
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

/// World-state registry of active and completed space interdiction bubbles (`SF.R1`).
#[derive(Clone, Debug, Default)]
pub struct SpaceBubbleBook {
    /// Active interdiction bubbles keyed by carrier hull root [`ObjId`].
    pub by_hull: BTreeMap<ObjId, ActiveInterdictionBubble>,
    /// Completed interdiction bubbles keyed by carrier hull root [`ObjId`].
    pub completed: BTreeMap<ObjId, CompletedInterdictionBubble>,
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

/// Register [`SpaceBubbleBook`] and the [`SPACE_TICK`] consumer on `reg`.
///
/// # Errors
///
/// Propagates any duplicate-registration error from [`Registrar::tick`].
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(SpaceBubbleBook::default());
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
    step_active_bubbles(ctx, &now);
    promote_due_interdictions(ctx, &now);
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

    let mut resumed_domains = Vec::new();
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        for hull in cleared_hulls {
            let Some(active) = book.by_hull.remove(&hull) else {
                continue;
            };
            let domain = active.domain;
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
            resumed_domains.push((hull, domain));
        }
    }

    for (hull, domain) in resumed_domains {
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
            combat_reports.push((hull, active.carrier_ship_id.clone(), events, carrier_state));
        }
    }

    if !combat_reports.is_empty()
        && let Some(salvage_book) = ctx.w.module_state_mut::<SalvageClaimBook>()
    {
        for (hull, carrier_ship_id, events, carrier_state) in combat_reports {
            let _ = record_space_combat_to_black_box(
                salvage_book,
                hull,
                &carrier_ship_id,
                None,
                &events,
                carrier_state,
                now.at,
            );
        }
    }
}

fn promote_due_interdictions(ctx: &mut Ctx<'_>, now: &ClockStamp) {
    let Some(journey_book) = ctx.w.module_state::<JourneyBook>().cloned() else {
        return;
    };
    let already_open: Vec<ObjId> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|b| b.by_hull.keys().copied().collect())
        .unwrap_or_default();

    let mut to_promote: Vec<(ObjId, DomainId, u32)> = Vec::new();
    for (&hull, journey) in &journey_book.journeys {
        if already_open.contains(&hull) {
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
        let Some(active) = build_interdiction_bubble(ctx.w, hull, domain, cursor, now.at) else {
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
    w: &World,
    hull: ObjId,
    domain: DomainId,
    leg_cursor: u32,
    ic_now_s: i64,
) -> Option<ActiveInterdictionBubble> {
    let carrier_ship_id = format!("hull.{}", hull.0);
    let interdictor_ship_id = format!("picket.{}.{}", domain.0, leg_cursor);

    let carrier_ship = build_carrier_ship_state(w, hull, &carrier_ship_id)?;
    let interdictor_obj = ObjId(900_000_u32.saturating_add(u32::try_from(domain.0).unwrap_or(0)));
    let (interdictor_unit, _) =
        instantiate_stock_space_unit(interdictor_obj, "tacsim.unit.dray").ok()?;
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
    let site_id = format!("domain.{}", domain.0);
    let mut bubble =
        LocalSpaceBubble::new(bubble_id, site_id, "Waypoint Interdiction Bubble", ic_now_s);
    bubble.set_party_relation(CARRIER_PARTY, INTERDICTOR_PARTY, SpaceRelation::Hostile);
    bubble.insert_ship(carrier_ship);
    bubble.insert_ship(interdictor_ship);

    let initial_snapshot = bubble.snapshot();
    let tape = BubbleInputTape::new(&bubble);

    Some(ActiveInterdictionBubble {
        hull,
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
    w: &World,
    hull: ObjId,
    carrier_ship_id: &str,
) -> Option<SpaceShipState> {
    let callsign = crate::port::coop::name_of(w, hull);
    if let Some(unit) = mush_tactics::module::unit_of(w, hull) {
        return space_ship_from_tactical_unit(
            unit,
            carrier_ship_id,
            callsign,
            CARRIER_PARTY,
            [0, 0, 0],
            [25_000, 0, 0],
        )
        .ok();
    }
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
}
