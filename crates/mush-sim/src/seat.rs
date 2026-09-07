//! E7 cut 10 B1 — the **two** `mush-sim` export seams, and nothing else
//! (10b §3.10 OPEN 1, taken as the owner directed).
//!
//! Cut 10's code lives in `mush-campaign`; its *mechanical* half — the billet
//! write, the narrowed grant, the activation request and the unclean-loss
//! trigger — lives here, because `handoff.rs` is a **private** module and
//! `ModuleActionState` is **owner-scoped**, so no crate outside `mush-sim` can
//! reach either the K6 request path or the `control_leases` family. The whole
//! surface `mush-campaign` reaches across the one new edge is:
//!
//! * [`request_seat_activation`] — one seam over the three mechanical seat
//!   transitions ([`SeatRequest`]: activate, release, revoke), each of which
//!   ends in the SAME `control_handoff_request` `station.rs`'s
//!   `submit_station_action` builds. Cut 10 creates no lease by any other path.
//! * [`channel_lease_holder`] — the "is this channel under a live lease, and
//!   whose" read the K6 gate's leg (2) needs (08j S1: `checked_request_handoff`
//!   never asks, because requesting over a live lease is the ordinary path).
//!
//! **Recorded deviation — how `channel_lease_holder` reads the lease.**
//! `Ctx::typed_state` is scoped to the *registering module*, so a
//! `mush-campaign` command context cannot read `sim`'s `control_leases` family,
//! and the only widening the kernel offers (`Registrar::delegate_state_families`)
//! is **whole-owner** and would hand `mush-campaign` WRITE authority over K's
//! lease family — which §4.1 and this cut's rules forbid outright. So the seam
//! answers from a bounded world-owned mirror ([`ControlLeaseView`]) that
//! `mush-sim` refreshes from its OWN owner-scoped contexts (its `sim.journey`
//! Coarse tick and the `Disconnected` hook). The mirror is the message; the
//! **authoritative** leg-2 refusal is `HandoffProvider::project_station_offer`'s
//! own holder branch, which reads live state at resolution — so a race can only
//! ever make the command's refusal *late*, never make an eviction *succeed*.
//!
//! **The K6 gate, as 10b §3.10 states it after 08j S1** — two policy refusals
//! plus one kernel refusal:
//!
//! 1. [`SeatError::StationHeld`] — the target station is vacant or held by the
//!    grant's own declared fallback; a live human operator is **never** evicted.
//!    Cut 10's own policy; nothing in the tree performed it before.
//! 2. [`SeatError::ChannelLeased`] — no channel in the seat's set is under a
//!    live lease held by anyone but that fallback. Cut 10's own policy.
//! 3. `ControlLeaseError::BeforeBoundary` — the only kernel-answered leg.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{
    AuthoritativeSubmit, ChannelId, ControlLeaseKey, ControlLeaseState, Ctx, Deferred,
    IdempotencyKeyText, ObjId, PerceivedRef, PrincipalId, StateFamilyHandle, World,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::commands::{apply_focus_bump_and_refresh, console_room, prepare_focus_bump};
use crate::data::{
    HostAssignments, MAX_GRANTED_CHANNELS, SpatialState, StationAssignment, StationId,
};
use crate::focus::{AccountOracle, ActiveFocus, FocusState, ReachProof};

/// Maximum lease records one mirror refresh visits (D-LBK-5). One page of K's
/// own `MAX_STATE_FAMILY_SCAN_RECORDS`; a deployment with more live leases than
/// this refreshes the rest on the next fire.
pub const MAX_LEASE_VIEW_RECORDS: usize = 1_024;

/// The world-owned mirror of the `control_leases` family: `(unit, channel) ->
/// the current holder`. **Never persisted, never authority** — it is the read
/// [`channel_lease_holder`] answers so a crate outside `mush-sim` can ask the
/// K6 gate's leg-2 question at all. Refreshed from `mush-sim`'s own
/// owner-scoped contexts; empty until the first refresh, which is fail-closed
/// for a *reader* asking "is anyone holding this" only in the sense that an
/// empty mirror says "nobody" — which is why the authoritative refusal lives in
/// the offer projection and this is the message.
#[derive(Clone, Default)]
pub struct ControlLeaseView {
    handle: Option<StateFamilyHandle<ControlLeaseKey, ControlLeaseState>>,
    holders: BTreeMap<(ObjId, ChannelId), PrincipalId>,
    /// Whether the last refresh completed (`false` = the family scan was
    /// unavailable or hit its bound; a caller that must fail closed asks this).
    complete: bool,
}

impl std::fmt::Debug for ControlLeaseView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlLeaseView")
            .field("wired", &self.handle.is_some())
            .field("holders", &self.holders.len())
            .field("complete", &self.complete)
            .finish()
    }
}

impl ControlLeaseView {
    /// Install the family handle at registration (the handle is a capability
    /// minted only while composing the owning module).
    #[must_use]
    pub fn new(handle: StateFamilyHandle<ControlLeaseKey, ControlLeaseState>) -> Self {
        Self {
            handle: Some(handle),
            holders: BTreeMap::new(),
            complete: false,
        }
    }

    /// Whether the last refresh saw the whole family.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// Note one `(unit, channel) -> holder` row. This is the refresh's own
    /// writer, exported so a test can seed the mirror the K6 gate's leg (2)
    /// reads without driving a whole scheduled apply. It is a MIRROR, never
    /// authority: writing here changes no lease and grants nothing.
    pub fn note_holder(&mut self, unit: ObjId, channel: ChannelId, holder: PrincipalId) {
        self.holders.insert((unit, channel), holder);
    }

    /// How many live lease holders the mirror carries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.holders.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.holders.is_empty()
    }
}

/// **Seam 2 of 2.** The principal currently holding `unit`'s `channel` lease, or
/// `None` when the channel is vacant. Reads the world-owned mirror; see the
/// module head for why it is not a direct family read.
#[must_use]
pub fn channel_lease_holder(w: &World, unit: ObjId, channel: &ChannelId) -> Option<PrincipalId> {
    w.module_state::<ControlLeaseView>()
        .and_then(|view| view.holders.get(&(unit, channel.clone())).cloned())
}

/// Refresh the mirror from `mush-sim`'s own owner-scoped typed-state reader.
/// Bounded on records **visited** ([`MAX_LEASE_VIEW_RECORDS`]); a partial pass
/// marks the mirror incomplete rather than publishing a half view as whole.
pub(crate) fn refresh_control_lease_view(ctx: &mut Ctx<'_>) {
    let Some(handle) = ctx
        .w
        .module_state::<ControlLeaseView>()
        .and_then(|view| view.handle.clone())
    else {
        return;
    };
    let mut holders = BTreeMap::new();
    let mut complete = false;
    let mut after: Option<SmolStr> = None;
    let mut visited = 0usize;
    while let Ok(page) = ctx.typed_state.scan_family(
        &handle,
        after.as_ref().map(SmolStr::as_str),
        MAX_LEASE_VIEW_RECORDS.min(256),
    ) {
        for entry in page.entries() {
            let lease = entry.value();
            visited += 1;
            if let Some(holder) = lease.holder() {
                holders.insert(
                    (lease.key().subject(), lease.key().channel().clone()),
                    holder.clone(),
                );
            }
        }
        match page.next_after() {
            Some(next) if visited < MAX_LEASE_VIEW_RECORDS => after = Some(next.clone()),
            Some(_) => break,
            None => {
                complete = true;
                break;
            }
        }
    }
    if let Some(view) = ctx.w.module_state_mut::<ControlLeaseView>() {
        view.holders = holders;
        view.complete = complete;
    }
}

/// The three mechanical seat transitions [`request_seat_activation`] performs.
/// One seam, not three `pub fn`s: the owner fence is "exactly two exported
/// seams", and all three end in the same K6 `control_handoff_request`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SeatRequest {
    /// `+seat/activate`: seat `npc` on `unit`/`station` with the narrowed
    /// channel set, bind the calling account's ONE foreground focus to it, and
    /// request the control lease at a safe boundary under `seat.activate`.
    Activate {
        /// The operated unit.
        unit: ObjId,
        /// Its station.
        station: StationId,
        /// The in-world actor the seat drives — a named NPC, never the account.
        npc: ObjId,
        /// The seat's channel set (intersected with the station's own).
        granted: BTreeSet<ChannelId>,
        /// The grant's declared fallback controller. Leg (1) admits a station
        /// held by exactly this actor and nobody else.
        fallback: Option<ObjId>,
    },
    /// `+seat/leave`: the driver relinquishes. The NPC keeps its billet (that
    /// is its station — AI control resumes), the narrowing is cleared, and the
    /// station's declared fallback is fired at a safe boundary.
    Release {
        /// The operated unit.
        unit: ObjId,
        /// Its station.
        station: StationId,
        /// The seated actor.
        npc: ObjId,
    },
    /// `@seat/revoke`, and the `Disconnected` hook's unclean loss. Identical to
    /// [`SeatRequest::Release`] except that it is not the driver's choice, so it
    /// never requires the driver's own account to be resolvable.
    Revoke {
        /// The operated unit.
        unit: ObjId,
        /// Its station.
        station: StationId,
        /// The seated actor.
        npc: ObjId,
    },
}

impl SeatRequest {
    /// The `(unit, station)` this request acts on.
    #[must_use]
    pub fn target(&self) -> (ObjId, &StationId) {
        match self {
            SeatRequest::Activate { unit, station, .. }
            | SeatRequest::Release { unit, station, .. }
            | SeatRequest::Revoke { unit, station, .. } => (*unit, station),
        }
    }
}

/// What a successful [`request_seat_activation`] did — enough for the caller to
/// render a receipt and to key its own idempotency, and **no lease epoch**: the
/// grant becomes `Active { epoch }` only from what `checked_apply_handoff`
/// later produces, observed by a pull.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeatTicket {
    /// The unit's control generation after the bump this request applied.
    pub control_generation: u64,
    /// The channels the seat actually grants (the intersection).
    pub channels: BTreeSet<ChannelId>,
    /// Whether a `control_handoff_request` was staged.
    pub staged: bool,
}

/// Why a seat request refused. Every variant is rendered by the CALLER, so the
/// two refusals a viewer may not distinguish stay byte-identical there.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SeatError {
    /// The unit has no such station (or no station topology at all).
    #[error("no such station")]
    NoSuchStation,
    /// The seat's channel set is empty after intersecting the station's own.
    #[error("the seat grants no channel that station carries")]
    NoChannel,
    /// The seat's narrowed set is over [`MAX_GRANTED_CHANNELS`].
    #[error("the seat grants too many channels")]
    TooManyChannels,
    /// **K6 gate leg (1).** The station is held by somebody who is not the
    /// grant's declared fallback. E7 ships no eviction path.
    #[error("that station is held")]
    StationHeld,
    /// **K6 gate leg (2).** One of the seat's channels is under a live lease
    /// held by somebody who is not the grant's declared fallback.
    #[error("that control channel is leased")]
    ChannelLeased {
        /// The conflicting channel.
        channel: ChannelId,
    },
    /// The acting account is not mapped (the oracle answered `None`) — fail
    /// closed, never a default.
    #[error("account unmapped")]
    UnmappedAccount,
    /// The seated actor cannot reach the station's console.
    #[error("out of reach")]
    OutOfReach,
    /// The unit's control-generation counter is exhausted (E5a S2: arithmetic
    /// overflow **pages**, never stalls).
    #[error("control generation exhausted")]
    CounterExhausted,
    /// The station is not seated by that actor (release/revoke).
    #[error("that seat is not held by that actor")]
    NotSeated,
    /// The module is composed without the K6 handoff actions.
    #[error("station control is unavailable")]
    Unavailable,
}

fn oracle_account(ctx: &Ctx<'_>, who: ObjId) -> Option<mush_core::AccountId> {
    ctx.w
        .module_state::<AccountOracle>()
        .and_then(|oracle| oracle.account_of(who))
}

/// **Seam 1 of 2.** Perform one mechanical seat transition.
///
/// Every fallible check runs **before** the first mutation: an over-cap set, a
/// held station, a leased channel, an unmapped account, an unreachable console
/// or an exhausted control-generation counter all refuse with the world
/// untouched. Past the first write there is no `?`, no `unwrap`, no `expect`
/// and no panicking index.
pub fn request_seat_activation(
    ctx: &mut Ctx<'_>,
    request: &SeatRequest,
) -> Result<SeatTicket, SeatError> {
    match request {
        SeatRequest::Activate {
            unit,
            station,
            npc,
            granted,
            fallback,
        } => activate(ctx, *unit, station, *npc, granted, *fallback),
        SeatRequest::Release { unit, station, npc }
        | SeatRequest::Revoke { unit, station, npc } => stand_down(ctx, *unit, station, *npc),
    }
}

/// The effective channel set for a seat: the station's own set intersected with
/// the requested narrowing.
fn effective_channels(
    ctx: &Ctx<'_>,
    unit: ObjId,
    station: &StationId,
    granted: &BTreeSet<ChannelId>,
) -> Result<BTreeSet<ChannelId>, SeatError> {
    let spec = crate::commands::st(ctx)
        .stations
        .get(&unit)
        .and_then(|hs| hs.stations.get(station))
        .ok_or(SeatError::NoSuchStation)?;
    if granted.len() > MAX_GRANTED_CHANNELS {
        return Err(SeatError::TooManyChannels);
    }
    let effective: BTreeSet<ChannelId> = spec.channels.intersection(granted).cloned().collect();
    if effective.is_empty() {
        return Err(SeatError::NoChannel);
    }
    Ok(effective)
}

fn activate(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    station: &StationId,
    npc: ObjId,
    granted: &BTreeSet<ChannelId>,
    fallback: Option<ObjId>,
) -> Result<SeatTicket, SeatError> {
    // ---- every fallible check, before any mutation ----
    let channels = effective_channels(ctx, unit, station, granted)?;
    // Leg (1): vacant, or held by the grant's OWN declared fallback. A live
    // human operator is never evicted, and cut 10 reaches no eviction path.
    if let Some(seat) = crate::commands::st(ctx)
        .assignments
        .get(&unit)
        .and_then(|ha| ha.seats.get(station))
        && Some(seat.actor) != fallback
        && seat.actor != npc
    {
        return Err(SeatError::StationHeld);
    }
    // Leg (2): no channel in the seat's set is under a live lease held by
    // anyone but that fallback. Cut 10's own policy — the kernel's
    // `checked_request_handoff` never compares `requested_by` to `holder`.
    let permitted: Vec<PrincipalId> = fallback
        .into_iter()
        .chain(std::iter::once(npc))
        .map(|id| PrincipalId::Object { id })
        .collect();
    for channel in &channels {
        if let Some(holder) = channel_lease_holder(ctx.w, unit, channel)
            && !permitted.contains(&holder)
        {
            return Err(SeatError::ChannelLeased {
                channel: channel.clone(),
            });
        }
    }
    // The account whose ONE foreground focus this seat consumes (T12/Q20A).
    let account = oracle_account(ctx, ctx.actor).ok_or(SeatError::UnmappedAccount)?;
    // Reach is proven for the DRIVING actor — the NPC at the console, not the
    // absent player's character (T13: the character never moves).
    let room = console_room(ctx, unit, station).ok_or(SeatError::OutOfReach)?;
    let reach = ReachProof::Console { room };
    if !reach.is_reachable(ctx.w, npc) {
        return Err(SeatError::OutOfReach);
    }
    // Preflight the K6 request action BEFORE any mutation: a composition
    // without it must refuse with the world untouched, never seat a billet and
    // then find it has no way to ask for the lease.
    if crate::handoff::request_action_ref(ctx.reg).is_none() {
        return Err(SeatError::Unavailable);
    }
    let prior_unit = ctx
        .w
        .module_state::<FocusState>()
        .and_then(|state| state.focus_of_account(account))
        .map(|focus| (focus.unit, focus.station.clone(), focus.actor));
    let new_gen = prepare_focus_bump(ctx, unit).map_err(|_| SeatError::CounterExhausted)?;
    let prior_gen = match &prior_unit {
        Some((prior, _, _)) if *prior != unit => Some((
            *prior,
            prepare_focus_bump(ctx, *prior).map_err(|_| SeatError::CounterExhausted)?,
        )),
        _ => None,
    };
    let acquired_at = ctx.w.world_ic_now();
    let now = ctx.w.now();

    // ---- apply: no fallible work past here ----
    seat_billet(ctx, unit, station, npc, now, Some(channels.clone()));
    apply_focus_bump_and_refresh(ctx, unit, new_gen, npc);
    let focus = ActiveFocus {
        account,
        actor: npc,
        unit,
        station: station.clone(),
        acquired_at,
        epoch: new_gen,
        reach,
        proxy: true,
    };
    let released = match ctx.w.module_state_mut::<FocusState>() {
        Some(state) => state.claim(focus),
        None => Vec::new(),
    };
    if let Some((prior, generation)) = prior_gen {
        apply_focus_bump_and_refresh(ctx, prior, generation, ctx.actor);
    }
    for prior in &released {
        if prior.actor != npc {
            // A displaced PROXY focus takes its narrowing with it: the station
            // it surrendered goes back to its declared controller on the
            // STATION's own channels, never on the seat's narrower set. Without
            // this, taking a second seat left the first station permanently
            // narrowed with nobody driving it.
            if prior.proxy {
                let now = ctx.w.now();
                seat_billet(ctx, prior.unit, &prior.station, prior.actor, now, None);
            }
            crate::links::refresh_actor_links(ctx.w, prior.actor);
        }
    }
    // T12: entering proxy mode hands off the player's OWN station — the
    // displaced focus's declared fallback fires on cut 5's path, exactly as a
    // `+focus` switch does.
    for prior in &released {
        if prior.actor != npc {
            crate::station::fire_station_fallback_as(ctx, prior.actor, prior.unit, &prior.station);
        }
    }
    // The K6 request, submitted AS THE NPC: the seat's own billet grant and the
    // proxy focus project cut 5's create/relief offer, so this is the identical
    // `control_handoff_request` `@station/handoff` stages — cut 10 opens no
    // second lease-creation path.
    let staged = submit_seat_handoff(ctx, npc, unit, station);
    Ok(SeatTicket {
        control_generation: new_gen,
        channels,
        staged,
    })
}

fn stand_down(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    station: &StationId,
    npc: ObjId,
) -> Result<SeatTicket, SeatError> {
    // The service interval, captured BEFORE the focus is relinquished (the
    // applied marker `sim.station.served` answers from).
    let served = ctx
        .w
        .module_state::<FocusState>()
        .and_then(|state| state.focus_of_actor(npc))
        .filter(|focus| focus.unit == unit && &focus.station == station)
        .map(|focus| focus.acquired_at.at);
    if crate::commands::st(ctx)
        .assignments
        .get(&unit)
        .and_then(|ha| ha.seats.get(station))
        .is_none_or(|seat| seat.actor != npc)
    {
        return Err(SeatError::NotSeated);
    }
    let generation = prepare_focus_bump(ctx, unit).map_err(|_| SeatError::CounterExhausted)?;
    let now = ctx.w.now();
    // ---- apply: no fallible work past here ----
    // The narrowing is cleared: the NPC keeps its billet (that IS its station)
    // and AI control resumes over the station's own full channel set.
    seat_billet(ctx, unit, station, npc, now, None);
    let account = ctx
        .w
        .module_state::<FocusState>()
        .and_then(|state| state.focus_of_actor(npc))
        .filter(|focus| focus.unit == unit && &focus.station == station && focus.proxy)
        .map(|focus| focus.account);
    if let Some(account) = account
        && let Some(state) = ctx.w.module_state_mut::<FocusState>()
    {
        state.relinquish(account);
    }
    // Bump THEN republish (cut 5's order, verify-fold C1): every queued human,
    // NPC, pathfinder or LLM intent captured under the old grant now fails
    // `World::channel_grant_is_current`.
    apply_focus_bump_and_refresh(ctx, unit, generation, npc);
    // The applied marker: service is recorded only when it actually ENDED, so a
    // settlement gated on `sim.station.served` cannot settle from a sortie that
    // merely started (08c 6.2).
    if let Some(from) = served {
        let to = ctx.w.world_ic_now().at;
        record_service(
            ctx,
            StationService {
                unit,
                station: station.clone(),
                actor: npc,
                from,
                to,
            },
        );
    }
    // The seat returns to its declared fallback at a SAFE BOUNDARY through the
    // same request path — never mid-tick, never a direct lease write.
    let staged = crate::station::fire_station_fallback_as(ctx, npc, unit, station);
    Ok(SeatTicket {
        control_generation: generation,
        channels: BTreeSet::new(),
        staged,
    })
}

/// Write one seat on `unit`'s `assignment:<unit>` document, rebuild the derived
/// indexes and persist. Infallible by construction (the station was proven to
/// exist by the caller's preflight).
fn seat_billet(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    station: &StationId,
    actor: ObjId,
    now: i64,
    granted: Option<BTreeSet<ChannelId>>,
) {
    let record = {
        let state = match ctx.w.module_state_mut::<SpatialState>() {
            Some(state) => state,
            None => return,
        };
        let seats = state
            .assignments
            .entry(unit)
            .or_insert_with(|| HostAssignments {
                host: unit,
                seats: BTreeMap::new(),
            });
        seats.seats.insert(
            station.clone(),
            StationAssignment {
                actor,
                assigned_at: now,
                granted,
            },
        );
        let record = seats.clone();
        state.derived = crate::graph::rebuild(state);
        record
    };
    crate::data::persist_assignments(ctx.out, &record);
}

/// Stage the seat's `control_handoff_request` **as the seated NPC**.
///
/// `Ctx::submit_authoritative` submits as `ctx.actor` (here: the absent
/// player's own character), and the offer that authorizes a station handoff is
/// projected for the *driving* actor. So the seam builds the same deferred
/// [`AuthoritativeSubmit`] with `actor = npc` — the engine keys its one-submit-
/// per-drain slot by actor, so the seat's request and the displaced station's
/// fallback fire are two different actors and neither voids the other.
fn submit_seat_handoff(ctx: &mut Ctx<'_>, npc: ObjId, unit: ObjId, station: &StationId) -> bool {
    let Some(action) = crate::handoff::request_action_ref(ctx.reg) else {
        return false;
    };
    // The 08g §10 C12 census stays at NINE: this is `station.rs`'s own
    // `actor_grant_epoch` — `mush-sim`'s single `command_subjects_of` read for a
    // handoff subject revision — called a second time, not a tenth consumer.
    let Some(revision) = crate::station::actor_grant_epoch(ctx, npc, unit, station.as_str()) else {
        return false;
    };
    let discriminator = format!("seat.activate/{}/{}/{}", unit.0, station.as_str(), npc.0);
    let Ok(key) = IdempotencyKeyText::parse_canonical(&discriminator) else {
        return false;
    };
    ctx.out.defer(Deferred::AuthoritativeSubmit(Box::new(
        AuthoritativeSubmit {
            actor: npc,
            action,
            subject: PerceivedRef::KnownObject { id: unit, revision },
            targets: Vec::new(),
            parameters: serde_json::json!({}),
            key,
        },
    )));
    true
}

/// **E7c10 B6 — the tree's FIRST `HookPoint::Disconnected` hook, and cut 5's
/// deferred unclean-loss PRODUCTION trigger** (10b §3.10 OPEN 3, closing the
/// deferral 10b:955 tracked).
///
/// Cut 5 shipped the unclean-loss relief as a provider-level test only
/// (`vf_c1_b_unclean_loss_system_principal_seats`): no module registered a
/// `Disconnected` hook, so a proxy seat — or an ordinary billet — stayed alive
/// through a disconnect, which T11 forbids in terms.
///
/// It fires from `Inbound::SessionGone` after `mush-session`'s `linkdead_grace`
/// (300 s by default) expires, with `ctx.actor` the departing character, a live
/// owner-scoped `typed_state` and a full `&mut Ctx`. It does exactly what a
/// clean `+focus/release` does, in the same order and through the same code:
///
/// 1. relinquish the departing account's foreground focus — the driven actor's
///    focus, whether the account was driving its own billet or a proxy seat;
/// 2. **bump then republish** (cut 5's order, verify-fold C1), so every queued
///    human, NPC, pathfinder or LLM intent captured under the old grant fails
///    `World::channel_grant_is_current` immediately;
/// 3. clear a proxy seat's narrowing, so the station's declared controller
///    resumes on the station's own channels rather than the seat's;
/// 4. fire the released station's declared fallback **at a safe boundary**,
///    through the same `control_handoff_request` path — never a direct lease
///    write, never mid-tick.
///
/// **Recorded, honestly:** the relief is staged under the *departing actor's*
/// surviving billet grant (`departed_fallback_station`), which is the shape that
/// works end to end through the real ingress. The fuller fork-(a)
/// scheduler-principal/no-witness ingress cut 5 recorded stays exactly where cut
/// 5 left it — exercised at the provider level by `vf_c1_b` — because
/// `Ctx::submit_authoritative` mints an `Object` principal and there is no
/// world-side seam that mints a scoped one.
pub(crate) struct SeatDisconnectHook;

impl mush_core::EventHook for SeatDisconnectHook {
    fn disconnected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        // The mirror is refreshed from `mush-sim`'s own owner-scoped context,
        // which this hook is, so a seat check made right after a disconnect
        // reads the lease as it actually stands.
        refresh_control_lease_view(ctx);
        let Some(account) = oracle_account(ctx, actor) else {
            return;
        };
        // Read the focus WITHOUT taking it, so every fallible step runs before
        // the first mutation (cut 5's invariant: a release whose witness bump
        // would fail must leave the focus intact rather than a focus-less seat
        // still publishing a live grant).
        let Some(focus) = ctx
            .w
            .module_state::<FocusState>()
            .and_then(|state| state.focus_of_account(account))
            .cloned()
        else {
            return;
        };
        let Ok(generation) = crate::commands::prepare_focus_bump(ctx, focus.unit) else {
            // The counter is exhausted: page rather than stall (E5a S2), with
            // NOTHING mutated — the driver keeps the seat until staff repair it,
            // which is the safe direction for an authority witness.
            tracing::error!(
                target: "sim",
                subject = %format!("cgen:{}", focus.unit.0),
                runbook = %format!(
                    "unit #{} could not bump its control generation while releasing a \
                     disconnected driver's seat, so the seat was NOT released. Repair the \
                     counter, then release it with `@station/fallback #{}/{}=<who>` and a \
                     staff handoff",
                    focus.unit.0, focus.unit.0, focus.station
                ),
                "sim: a disconnected seat could not bump its unit's control generation"
            );
            return;
        };
        // ---- apply: no fallible work past here ----
        if let Some(state) = ctx.w.module_state_mut::<FocusState>() {
            state.relinquish(account);
        }
        if focus.proxy {
            // A proxy seat's narrowing goes with the driver: the declared
            // controller resumes on the STATION's own channels, not the seat's.
            let now = ctx.w.now();
            seat_billet(ctx, focus.unit, &focus.station, focus.actor, now, None);
        }
        crate::commands::apply_focus_bump_and_refresh(ctx, focus.unit, generation, focus.actor);
        // The service the departing driver actually completed, as an applied
        // marker (the same one a clean release writes).
        let to = ctx.w.world_ic_now().at;
        record_service(
            ctx,
            StationService {
                unit: focus.unit,
                station: focus.station.clone(),
                actor: focus.actor,
                from: focus.acquired_at.at,
                to,
            },
        );
        crate::station::fire_station_fallback_as(ctx, focus.actor, focus.unit, &focus.station);
    }
}

// ---- E7c10 B8: `sim.station.served`, the cut's first `sim.*` fact ----

/// One **applied marker** of completed station service — never a status
/// (08c §6.2). It is written when a driver actually leaves the seat (a clean
/// release, a revocation or a disconnect), so a settlement gated on
/// `sim.station.served` can never settle from a sortie that merely *started*.
///
/// Rule 1: **exempt** — workflow, legacy-post-image, future family
/// `sim.station_service`. It takes no part in authority or prepared mechanics:
/// nothing in the control path reads it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StationService {
    /// The unit served.
    pub unit: ObjId,
    /// Its station.
    pub station: StationId,
    /// The in-world actor that served it (an NPC for a proxy seat).
    pub actor: ObjId,
    /// When service began (world IC seconds).
    pub from: i64,
    /// When it ended.
    pub to: i64,
}

/// Load-refusing bound on the service book (D-LBK-5): a bounded world-owned
/// mirror, one row per `(unit, station, actor)` — the latest completed interval.
pub const MAX_STATION_SERVICE_ROWS: usize = 4096;

/// The world-owned mirror of the `sim.station_service` records the `sim.*` fact
/// provider answers from. Hydrated at boot; bounded by
/// [`MAX_STATION_SERVICE_ROWS`].
#[derive(Clone, Debug, Default)]
pub struct ServiceBook(BTreeMap<(ObjId, StationId, ObjId), StationService>);

impl ServiceBook {
    /// The latest completed service interval for one `(unit, station, actor)`.
    #[must_use]
    pub fn get(&self, unit: ObjId, station: &StationId, actor: ObjId) -> Option<&StationService> {
        self.0.get(&(unit, station.clone(), actor))
    }

    /// Record one completed interval. Returns `false` (without inserting) when a
    /// NEW row would exceed [`MAX_STATION_SERVICE_ROWS`].
    #[must_use]
    pub fn record(&mut self, service: StationService) -> bool {
        let key = (service.unit, service.station.clone(), service.actor);
        if !self.0.contains_key(&key) && self.0.len() >= MAX_STATION_SERVICE_ROWS {
            return false;
        }
        self.0.insert(key, service);
        true
    }

    /// How many rows the book carries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The `sim.station_service` document key.
#[must_use]
pub fn service_key(unit: ObjId, station: &StationId, actor: ObjId) -> SmolStr {
    SmolStr::new(format!(
        "station_service:{}:{}:{}",
        unit.0,
        station.as_str(),
        actor.0
    ))
}

/// Write one completed service interval: the RAM mirror and its durable exempt
/// record, refusing past the cap before either is touched.
fn record_service(ctx: &mut Ctx<'_>, service: StationService) {
    let key = service_key(service.unit, &service.station, service.actor);
    let value = match serde_json::to_value(&service) {
        Ok(value) => value,
        Err(_) => return,
    };
    let accepted = ctx
        .w
        .module_state_mut::<ServiceBook>()
        .is_some_and(|book| book.record(service));
    if !accepted {
        return;
    }
    ctx.out
        .defer(mush_core::Deferred::Persist(mush_core::ModuleOp {
            ns: SmolStr::new_static(crate::NS),
            op: mush_core::ModuleWrite::JsonSet { key, value },
        }));
}

/// **`mush-sim`'s one `FactProvider` namespace** (D-E7-30: five crates, one
/// namespace each; `with_facts` PANICS on a duplicate). It answers exactly one
/// kind today — `sim.station.served`, keyed `"<unit>/<station>/<actor>"` — from
/// the applied marker above, and `None` (unknown, and therefore unsatisfied) for
/// everything else.
#[derive(Debug, Default)]
pub struct SimFactProvider;

impl mush_economy::FactProvider for SimFactProvider {
    fn provider(&self) -> &str {
        crate::NS
    }

    fn resolve(
        &self,
        w: &World,
        kind: &str,
        id: &str,
        _revision: Option<u64>,
    ) -> Option<mush_economy::FactState> {
        if kind != "sim.station.served" {
            return None;
        }
        let mut parts = id.split('/');
        let unit = ObjId(parts.next()?.trim_start_matches('#').parse::<u32>().ok()?);
        let station = StationId::new(parts.next()?)?;
        let actor = ObjId(parts.next()?.trim_start_matches('#').parse::<u32>().ok()?);
        if parts.next().is_some() {
            return None;
        }
        let service = w
            .module_state::<ServiceBook>()?
            .get(unit, &station, actor)?;
        Some(mush_economy::FactState {
            satisfied: service.to >= service.from,
            actor: Some(service.actor),
            at: service.to,
        })
    }
}
