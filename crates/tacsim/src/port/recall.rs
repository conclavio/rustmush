//! **The recall** (doc 18 cut IC13; doc 18 §4.1 "Losses, dormancy and release",
//! D-18-23, D-18-25; lead rulings L3, L4, L5, L12, L15, L16 of the IC13 brief).
//!
//! A chartered Dray goes back to the Co-op three ways — the pilot hands her back
//! (`+coop/release`), staff revoke the charter (`@coop/revoke <pilot>=<why>`), or a
//! week passes without a departure (the port tick's **recall arm**) — and all three
//! run one path, under one rule (L16): **the hull is docked at Ceres, her cargo hold
//! is empty, and nothing is in flight on her.**
//!
//! The recall has **two halves** (L4):
//!
//! - **ending the charter**: the enrolment is written `Revoked` with its cause
//!   ([`EndCause`]) **first**, so the observer's IC6 fold ignores the lease's
//!   Cancelled event, then the lease ends through `mush_economy::end_lease`;
//! - **returning the hull** (`return_hull`): the pilot's sailing plan is held, anyone
//!   aboard is put ashore at the gallery, the hold vault is **re-homed to the Trust**
//!   (never closed: a closed vault keeps its backing index and the hull could never be
//!   chartered again, L5) with the remaining fuel and stores forfeit inside it (owner
//!   item O1), and the hull's laid-up name comes back (L15, owner item O5).
//!
//! When the lease is already terminal — a staff `@contract/cancel` folded by the IC6
//! observer, a `Retrying` lease end that completed, a refunded buyout whose re-lease
//! failed — only the second half runs, from the tick's **Return** arm. That closes
//! IC6's residual "a staff-revoked hull stays out of the pool until IC13". The next
//! `+coop/enrol` charters a returned hull and re-owns its hold vault to the new pilot.
//!
//! **Dormancy reads durable departures only** (L3): the latest burned ticket (scanned
//! down from `Voyage.last_seq`), the Done departs of the pilot's plan and the
//! enrolment's `leased_at` — never a RAM stamp; a read error never recalls.
//!
//! No step here submits anything authoritative and none swaps the actor:
//! `end_lease`, `rehome_vault`, `put_orders`, `set_name` and the placements are
//! normal-lane writes (L12).

use mush_core::{CmdError, Ctx, ObjId, ObjKind, World};
use mush_economy::{EconomyStore, LeaseEnd, LeaseKey, VaultId};
use mush_inventory::record::EndpointRef;
use mush_org::{OrgScopeId, PrincipalId};
use smol_str::SmolStr;

use crate::definitions::Consumable;
use crate::port::board::group_thousands;
use crate::port::buyout::{ALREADY_GRADUATED, BUYOUT_PENDING, Busy, in_flight};
use crate::port::coop::{
    BOOKS_UNREADABLE, Books, CHARTER_NOT_IN_FORCE, COUNTERSIGN, EndCause, Enrolment,
    EnrolmentStatus, MAX_REVOKE_WHY_BYTES, NO_CHARTER, account_books, live_charter, office_of,
    own_enrolment, port_name, programme_port, say, trust_lease_on,
};
use crate::port::executor::HELD_LINE;
use crate::port::families::TicketId;
use crate::port::orders::{OrderState, OrdersState, PlanState, What, orders_state};
use crate::port::ship::{Berth, berth_of, gallery_line, hull_view, put_ashore};
use crate::port::voyage::{VoyageState, ticket_at};
use crate::port_tick::{PortTickState, Step};
use crate::store::{TacsimStore, put_enrolment, put_orders};

/// The staff verb (doc 18 §4.1, 18a §4).
pub const COOP_STAFF_VERB: &str = "@coop";

/// `@coop`'s help (cut IC13).
pub const COOP_STAFF_HELP: &str = "@coop/revoke <pilot>=<why>  (staff) end a First Captain's \
     charter now and take the Dray back into the Co-op's pool: the same recall a week without a \
     departure makes (the hull docked at Ceres, her cargo hold empty, nothing in flight). \
     Attributed: the reason is recorded and told to the pilot.";

/// Seconds in a real day.
const DAY_SECS: i64 = 86_400;

/// The burned tickets the dormancy clock scans down from `Voyage.last_seq` (L3).
pub const DORMANCY_TICKET_SCAN: u32 = 16;

/// A blocked recall is re-evaluated every this many fires (cut IC13).
pub const RECALL_RECHECK_FIRES: u64 = 24;

/// Under way: `{name}`, `{port}`.
pub const RECALL_UNDER_WAY: &str =
    "The {name} is under way; she goes back to the Co-op docked at {port}.";
/// Docked elsewhere, or anywhere else: `{name}`, `{port}`.
pub const RECALL_NOT_AT_PORT: &str = "The {name} goes back to the Co-op docked at {port}.";
/// Cargo aboard: `{name}`.
pub const RECALL_CARGO: &str =
    "The {name} still carries cargo; she goes back to the Co-op with an empty hold.";
/// A hold pool that would not read: `{name}`.
pub const RECALL_HOLD_DARK: &str =
    "The {name}'s hold cannot be read right now; nothing changes hands.";
/// The economy frozen, the Trust or the vault unresolved: `{name}`.
pub const RECALL_ECONOMY: &str =
    "The Registry cannot take the {name} back right now; nothing changes hands.";
/// The plan's hold when a hull goes back (lead 2i): `{name}`.
pub const HELD_RECALLED: &str = "the {name} went back to the Co-op.";
/// The plan's hold on a dormant hull that cannot be recalled where she is: `{name}`,
/// `{port}`.
pub const HELD_DORMANT: &str = "a week without a departure: the Co-op holds the {name} until she is docked at {port} with an empty hold.";
/// To each person put ashore: `{name}`.
pub const PUT_ASHORE: &str =
    "The Co-op's dockhands see you ashore: the {name} is the Co-op's again.";
/// The dormancy notice: `{name}` (the pilot's name for her), `{mark}`.
pub const DORMANT_NOTICE: &str = "The Co-op has taken back the {name} ({mark}): a week without a departure. The stake stays yours.";
/// The revocation notice: `{name}`, `{mark}`, `{why}`.
pub const REVOKED_NOTICE: &str = "The Co-op has revoked your charter of the {name} ({mark}): {why}";
/// The return arm completing an ended charter: `{name}`, `{mark}`.
pub const RETURNED_NOTICE: &str = "The Co-op has taken the {name} ({mark}) back into its pool.";
/// `+coop/release` with an argument.
pub const RELEASE_USAGE: &str = "Usage: +coop/release";
/// Away from the desk: `{office}`, `{port}`.
pub const RELEASE_WHERE: &str = "A charter is handed back at the {office} in {port}.";
/// The release: `{name}`, `{mark}`, `{countersign}`.
pub const RELEASED: &str = "You hand the {name}'s papers back across the counter: the Co-op takes back {mark}, and the stake stays yours. {countersign} countersigns.";
/// What stays aboard: `{tank}`, `{locker}`.
pub const FORFEIT_LINE: &str =
    "The {tank} t of reaction mass and {locker} crew-days aboard stay with her.";
/// The release whose lease end is retrying: `{mark}`.
pub const RELEASE_RETRYING: &str =
    "The Registry has your papers; the Co-op takes {mark} back as soon as its books close.";
/// `@coop/revoke` without its two sides.
pub const REVOKE_USAGE: &str = "Usage: @coop/revoke <pilot>=<why>";
/// A reason out of bounds.
pub const REVOKE_WHY_BOUNDS: &str = "A revocation gives its reason in 1 to 200 bytes.";
/// No player by that name or number.
pub const NO_SUCH_PILOT: &str = "No such pilot.";
/// A pilot with no live charter: `{pilot}`.
pub const NO_COOP_CHARTER: &str = "{pilot} holds no Co-op charter.";
/// The prefix of a blocked revocation's line.
pub const NOT_REVOKED: &str = "Not revoked: ";
/// The revocation: `{name}`, `{mark}`, `{pilot}`, `{why}`.
pub const REVOKED_REPLY: &str = "Charter of the {name} ({mark}) revoked from {pilot}: {why}. The Dray is back in the Co-op's pool.";
/// The revocation whose lease end is retrying: `{name}`, `{mark}`, `{pilot}`, `{why}`.
pub const REVOKED_RETRYING: &str = "Charter of the {name} ({mark}) revoked from {pilot}: {why}. The lease end is retrying; the Dray returns when it completes.";
/// **A rig in the bay** (doc 20 cut SM.3a; reviewer F1): `{name}`. The pilot's own
/// `+coop/release` refuses with it (the pilot hands the rigs back first); a revoke,
/// the dormancy recall and the return arm repossess the rigs instead
/// (`recall_block_clearing`).
pub const RECALL_BAY: &str =
    "The {name} still carries a rig in her bay; she goes back to the Co-op with an empty bay.";
/// **The page a bay block raises** on the recall arm (doc 20 cut SM.3a; reviewer F1:
/// the hold-and-revoke advice cannot clear a rig, so the page says what does).
pub const BAY_PAGE: &str = "tacsim: the hull's charter is ending but a rig in its bay cannot go back to Occator (its hopper is not empty, or its lease record cannot be read, or its lease will not end, or the hull's unit is bound to a definition that is not a hull (not fielded, or fielded with no interior or no logistics) so its bay cannot be read, or a rig is out on a site or mid-act (its tacsim.excursion record holds an act or an open excursion)); staff empty the hopper (staff may take from a hopper vault), repair the tacsim:tvlease: record, or bind the hull's unit back to the Dray or the Dray-P; a rig out on a site holds until its pilot recovers it (+tv/<n> recover) or its act finishes; the recall retries";

/// Why a hull goes back (maps to [`EndCause`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecallCause {
    /// `+coop/release`.
    Released,
    /// The tick's dormancy recall.
    Dormant,
    /// `@coop/revoke`.
    Revoked {
        /// The staff member.
        by: ObjId,
        /// Their reason.
        why: String,
    },
}

impl RecallCause {
    fn end_cause(&self) -> EndCause {
        match self {
            RecallCause::Released => EndCause::Released,
            RecallCause::Dormant => EndCause::Dormant,
            RecallCause::Revoked { by, why } => EndCause::Revoked {
                by: *by,
                why: why.clone(),
            },
        }
    }
}

/// What blocks a recall: each carries its player/staff-facing line beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecallBlock {
    /// On a journey.
    UnderWay,
    /// Docked at another port, or anywhere else.
    NotAtPort,
    /// A clearance filed, or the voyage unreadable.
    Cleared,
    /// A queued step at the counter, or the queue unreadable.
    Inflight,
    /// Goods still moving through the hold.
    GoodsMoving,
    /// Cargo aboard.
    Cargo,
    /// A hold pool that would not read.
    Unreadable,
    /// The economy frozen, the Trust or the vault unresolved.
    Economy,
    /// **A rig in the hull's bay** (doc 20 cut SM.3a; reviewer F1). Checked last,
    /// so a `Bay` block means every other recall check passed.
    Bay,
}

/// What a recall did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecallDone {
    /// Reaction mass left in the hold (forfeit with it), ledger units.
    pub tank: u64,
    /// Crew-days left in the hold (forfeit with it).
    pub locker: u64,
    /// The people put ashore.
    pub ashore: Vec<ObjId>,
    /// The lease end is retrying in the economy sweep: the return half has not run
    /// (the tick's Return arm finishes it once the lease is terminal).
    pub lease_retrying: bool,
}

/// `{name}` and `{mark}` filled from an enrolment.
fn named(template: &str, e: &Enrolment) -> String {
    template
        .replace("{name}", &e.name)
        .replace("{mark}", &e.mark)
}

/// The hull's hold, as the inventory addresses it (the enrolment's vault).
fn hold_of(e: &Enrolment) -> EndpointRef {
    EndpointRef(VaultId(e.vault))
}

/// **The read-only checks, in this order** (none writes): the berth at the
/// programme port, nothing in flight, an empty cargo pool, the economy, and —
/// since doc 20 cut SM.3a, last (check 7) — an empty vehicle bay.
pub(crate) fn recall_block(ctx: &Ctx<'_>, e: &Enrolment) -> Option<(RecallBlock, String)> {
    let w: &World = ctx.w;
    let economy_block = || {
        tracing::error!(
            hull = e.hull.0,
            pilot = e.pilot.0,
            "tacsim: a recall cannot resolve the economy, the Trust or the hold vault"
        );
        Some((RecallBlock::Economy, named(RECALL_ECONOMY, e)))
    };
    let Some(port) = programme_port(w) else {
        return economy_block();
    };
    let port_label = port_name(port.def);
    // 1. Docked at the programme port.
    match berth_of(w, e.hull) {
        Berth::UnderWay => {
            return Some((
                RecallBlock::UnderWay,
                named(RECALL_UNDER_WAY, e).replace("{port}", &port_label),
            ));
        }
        Berth::Docked { map, .. } if map.entry == port.entry => {}
        Berth::Docked { .. } | Berth::Elsewhere => {
            return Some((
                RecallBlock::NotAtPort,
                named(RECALL_NOT_AT_PORT, e).replace("{port}", &port_label),
            ));
        }
    }
    // 2–4. Nothing in flight. A filed hull is never recalled, so there is never a
    // pending to clear and `last_seq` is untouched (this refusal wins over lead 2f's
    // "clear the Voyage pending").
    if let Some((busy, line)) = in_flight(w, e) {
        let block = match busy {
            Busy::Cleared => RecallBlock::Cleared,
            Busy::Inflight => RecallBlock::Inflight,
            Busy::Goods => RecallBlock::GoodsMoving,
        };
        return Some((block, line));
    }
    // 5. The cargo pool: every goods-table row that is no consumable.
    let Some(goods) = w.module_state::<TacsimStore>().and_then(TacsimStore::goods) else {
        return economy_block();
    };
    for good in goods.goods.iter().filter(|g| g.consumable.is_none()) {
        match crate::port_tick::read_pool(ctx, hold_of(e), &good.id) {
            Ok(0) => {}
            Ok(_) => return Some((RecallBlock::Cargo, named(RECALL_CARGO, e))),
            Err(error) => {
                tracing::warn!(hull = e.hull.0, good = %good.id, ?error, "tacsim: a recall could not read the hold");
                return Some((RecallBlock::Unreadable, named(RECALL_HOLD_DARK, e)));
            }
        }
    }
    // 6. The economy.
    let Some(economy) = w.module_state::<EconomyStore>() else {
        return economy_block();
    };
    if economy.is_frozen()
        || port.trust().is_none()
        || port.trust_scope().is_none()
        || economy.snapshot().vault(VaultId(e.vault)).is_none()
    {
        return economy_block();
    }
    // 7. Doc 20 cut SM.3a (reviewer F1): the bay is empty — no placement `Interior`
    // in the hull's bay room, and no live lease record naming the hull (reviewer
    // finding correctness/F2: a record whose rig is out of the bay still holds the
    // hull, so no charter ends with a live rig lease). Last, so a `Bay` block means
    // everything else passed.
    //
    // **An unreadable view fails closed** (the SM.3a pre-commit round, correctness
    // F5): `hull_view` is `None` when the hull's live unit is bound to a definition
    // that is not a hull — one the fielded catalog does not hold, or one it fields
    // with no interior or no logistics (a staff `@unit` re-create, say) — and then
    // nothing here can see the bay, so the hull is treated as carrying a rig. The
    // pilot's `+coop/release` hears the rigs' dark line; a revoke, the dormancy
    // recall and the return arm reach `recall_block_clearing`, whose repossession
    // refuses on the same missing view, so the hull is held and staff are paged
    // ([`BAY_PAGE`]), exactly as for a hopper that is not empty.
    let Some(view) = hull_view(w, e.hull) else {
        tracing::error!(
            hull = e.hull.0,
            pilot = e.pilot.0,
            "tacsim: a recall cannot read the hull's view, so it cannot see the bay; the hull is held"
        );
        return Some((RecallBlock::Bay, named(crate::port::tv::TV_DARK, e)));
    };
    if crate::port::tv::bay_slot_word(w, &view).is_some() {
        return Some((RecallBlock::Bay, named(RECALL_BAY, e)));
    }
    None
}

/// **`recall_block`, clearing the bay first** (doc 20 cut SM.3a; reviewer F1,
/// decisions D14 and D16): on a `Bay` block every rig in the bay is repossessed
/// through [`crate::port::tv::repossess`] and the checks run again — `None` now
/// unless a rig remains; when the repossession refuses, its reason is logged and
/// the `Bay` block stands. The three exits that end a charter **without the pilot**
/// use it (`@coop/revoke`, the dormancy recall, [`return_hull`]); the pilot's own
/// `+coop/release` and the buyout keep the plain block.
pub(crate) fn recall_block_clearing(
    ctx: &mut Ctx<'_>,
    e: &Enrolment,
) -> Option<(RecallBlock, String)> {
    match recall_block(ctx, e) {
        Some((RecallBlock::Bay, line)) => match crate::port::tv::repossess(ctx, e.hull) {
            Ok(_) => recall_block(ctx, e),
            Err(why) => {
                tracing::error!(
                    hull = e.hull.0,
                    pilot = e.pilot.0,
                    %why,
                    "tacsim: a charter's end could not repossess the rigs in its bay"
                );
                Some((RecallBlock::Bay, line))
            }
        },
        other => other,
    }
}

/// The consumables left in the hold: (reaction mass, crew-days). A read error
/// counts as zero (the reply and the log only).
fn consumables(ctx: &Ctx<'_>, e: &Enrolment) -> (u64, u64) {
    let Some(goods) = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
    else {
        return (0, 0);
    };
    let mut tank = 0_u64;
    let mut locker = 0_u64;
    for good in &goods.goods {
        let Some(kind) = good.consumable else {
            continue;
        };
        let units = crate::port_tick::read_pool(ctx, hold_of(e), &good.id).unwrap_or(0);
        match kind {
            Consumable::Propellant => tank = tank.saturating_add(units),
            Consumable::Provisions => locker = locker.saturating_add(units),
        }
    }
    (tank, locker)
}

/// **Hold the pilot's sailing plan** (lead 2i): when the hull's plan reads, its
/// author is `e.pilot` and it has a cursor, and it is not already stopped with this
/// reason — and, when `running_only`, it is `Running` — a stopped hold at the cursor,
/// one `HELD_LINE` log line, the cursor derived and one checked `put_orders`; when
/// `tell`, the author hears the line once (the executor's stopped holds are told; a
/// recall that went through tells its own notice instead). `true` when it wrote; a
/// failed write is logged and the caller proceeds.
fn hold_plan(
    ctx: &mut Ctx<'_>,
    e: &Enrolment,
    reason: &str,
    running_only: bool,
    tell: bool,
) -> bool {
    let mut doc = match orders_state(ctx.w, e.hull) {
        OrdersState::Present(doc) if doc.author == e.pilot => doc.clone(),
        OrdersState::Present(_) | OrdersState::Absent | OrdersState::Unreadable => return false,
    };
    let Some(cursor) = doc.cursor else {
        return false;
    };
    let stopped_so = doc.state == PlanState::Held
        && doc
            .hold
            .as_ref()
            .is_some_and(|note| !note.retry && note.reason == reason);
    if stopped_so || (running_only && doc.state != PlanState::Running) {
        return false;
    }
    let now = ctx.w.now();
    doc.stop_at(cursor, reason, now);
    let line = HELD_LINE
        .replace("{n}", &cursor.to_string())
        .replace("{reason}", reason);
    doc.push_log(now, ctx.w.world_ic_now().at, cursor, &line);
    doc.derive_cursor();
    match put_orders(ctx, &doc) {
        Ok(()) => {
            if tell {
                crate::port_tick::notify(ctx, e.pilot, &crate::port::orders::clamp_line(&line));
            }
            true
        }
        Err(error) => {
            tracing::error!(hull = e.hull.0, %error, "tacsim: a recall could not hold the sailing plan; the recall proceeds");
            false
        }
    }
}

/// **The return half, unchecked** (`recall_block` has passed): the plan held, people
/// ashore, the vault re-homed to the Trust, the laid-up name restored. Nothing in it
/// refuses; each failure is logged and the rest proceeds.
fn return_unchecked(
    ctx: &mut Ctx<'_>,
    e: &Enrolment,
    trust: PrincipalId,
    scope: OrgScopeId,
) -> RecallDone {
    // 1. The plan (lead 2i): the document stays readable and its author stays; a Held
    // plan is never offered by the order arm, and the next tenant's `+ship/order`
    // replaces another author's plan.
    hold_plan(ctx, e, &named(HELD_RECALLED, e), false, false);
    // 2. People aboard (lead 2g), put ashore at the programme port's gallery.
    let airlock = hull_view(ctx.w, e.hull).and_then(|view| view.airlock());
    let gallery = programme_port(ctx.w).and_then(|port| port.map.presentation.map(ObjId));
    let mut ashore = Vec::new();
    match (airlock, gallery) {
        (Some(airlock), Some(gallery)) => {
            for person in crate::port::voyage::souls_aboard(ctx.w, airlock) {
                let name = e.name.clone();
                let landed = put_ashore(
                    ctx,
                    person,
                    e.hull,
                    gallery,
                    "put ashore by the Co-op",
                    |ctx: &mut Ctx<'_>| {
                        ctx.out
                            .sys_to_actor(person, "tacsim", PUT_ASHORE.replace("{name}", &name));
                        gallery_line(ctx, person, gallery, &name);
                    },
                );
                match landed {
                    Ok(()) => ashore.push(person),
                    Err(error) => tracing::error!(
                        hull = e.hull.0,
                        person = person.0,
                        %error,
                        "tacsim: a recall could not put a person ashore; they stay aboard and \
                         the recall proceeds; staff move them by hand"
                    ),
                }
            }
        }
        _ => tracing::error!(
            hull = e.hull.0,
            "tacsim: a recall found no airlock or gallery; nobody was put ashore"
        ),
    }
    // 3. The vault (L5): re-homed to the Trust with the consumables forfeit inside it;
    // never closed.
    let (tank, locker) = consumables(ctx, e);
    let hold_name = SmolStr::new(format!("{} hold", e.mark));
    // The Trust is `Organization(programme org)` by construction (`ProgrammePort::trust`).
    if mush_economy::rehome_vault(ctx, VaultId(e.vault), trust, Some(scope), hold_name) {
        tracing::info!(
            hull = e.hull.0,
            pilot = e.pilot.0,
            vault = e.vault,
            tank,
            locker,
            cause = ?e.ended,
            "tacsim: a recalled hull's hold vault is the Trust's; the fuel and stores in it are forfeit with it"
        );
    } else {
        tracing::error!(
            hull = e.hull.0,
            vault = e.vault,
            "tacsim: a recall could not re-home the hold vault; staff re-own it to the Trust by hand"
        );
    }
    // 4. The laid-up name (L15); the mark is untouched.
    if let Err(error) = ctx.w.set_name(e.hull, &e.laid_up_name) {
        tracing::error!(hull = e.hull.0, %error, "tacsim: a recall could not restore the laid-up name");
    }
    RecallDone {
        tank,
        locker,
        ashore,
        lease_retrying: false,
    }
}

/// The Trust and its root scope.
fn trust_of(w: &World) -> Option<(PrincipalId, OrgScopeId)> {
    let port = programme_port(w)?;
    Some((port.trust()?, port.trust_scope()?))
}

/// **The return half alone** (no live Trust lease on the hull; the enrolment already
/// `Revoked`): `recall_block`, then the return.
///
/// # Errors
///
/// The block and its line.
pub(crate) fn return_hull(
    ctx: &mut Ctx<'_>,
    e: &Enrolment,
) -> Result<RecallDone, (RecallBlock, String)> {
    // Doc 20 cut SM.3a: the return arm and `end_revoked_lease` repossess the bay's
    // rigs before the hull goes back.
    if let Some(block) = recall_block_clearing(ctx, e) {
        return Err(block);
    }
    let Some((trust, scope)) = trust_of(ctx.w) else {
        return Err((RecallBlock::Economy, named(RECALL_ECONOMY, e)));
    };
    Ok(return_unchecked(ctx, e, trust, scope))
}

/// **End the charter and return the hull, in one step** (an `Active` enrolment with a
/// live charter): `recall_block`; the enrolment `Revoked` with its cause **first**;
/// `end_lease`; then the return half — or, on `Retrying`, stop there (the Return arm
/// finishes it). The cause's notice is sent either way (the verb's own reply tells a
/// releasing pilot).
///
/// # Errors
///
/// The block and its line; nothing was written.
pub(crate) fn recall(
    ctx: &mut Ctx<'_>,
    e: &Enrolment,
    cause: RecallCause,
) -> Result<RecallDone, (RecallBlock, String)> {
    if let Some(block) = recall_block(ctx, e) {
        return Err(block);
    }
    let Some((trust, scope)) = trust_of(ctx.w) else {
        return Err((RecallBlock::Economy, named(RECALL_ECONOMY, e)));
    };
    let revoked = Enrolment {
        status: EnrolmentStatus::Revoked,
        ended_at: Some(ctx.w.now()),
        ended: Some(cause.end_cause()),
        ..e.clone()
    };
    put_enrolment(ctx, &revoked);
    let key = LeaseKey {
        lessor: trust.clone(),
        premises: e.hull,
    };
    let ended = mush_economy::end_lease(ctx, mush_economy::ContractId(e.lease), &key);
    let done = match recall_outcome(&ended) {
        RecallOutcome::Return => return_unchecked(ctx, &revoked, trust, scope),
        RecallOutcome::Retry => {
            match &ended {
                Err(error) => {
                    tracing::error!(lease = e.lease, %error, "tacsim: a recall's lease did not end; the recall arm retries it");
                }
                Ok(_) => tracing::error!(
                    lease = e.lease,
                    hull = e.hull.0,
                    "tacsim: a recall's lease end is retrying; the recall arm returns the hull \
                     once the lease is terminal; `@contract/show #<lease>`"
                ),
            }
            RecallDone {
                lease_retrying: true,
                ..RecallDone::default()
            }
        }
    };
    let notice = match &cause {
        RecallCause::Released => None,
        RecallCause::Dormant => Some(named(DORMANT_NOTICE, e)),
        RecallCause::Revoked { why, .. } => Some(named(REVOKED_NOTICE, e).replace("{why}", why)),
    };
    if let Some(line) = notice {
        crate::port_tick::notify(ctx, e.pilot, &line);
    }
    Ok(done)
}

/// What a recall does after its `end_lease` (doc 18 cut IC14 row 32: the decision
/// split out of [`recall`] so it is unit-tested without a world).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecallOutcome {
    /// The lease is over: the return half runs now.
    Return,
    /// `Retrying`, or `end_lease` refused: the enrolment is already `Revoked`
    /// (written first) and the lease still live, so the recall arm finishes it —
    /// the Return arm once the lease is terminal, or (cut IC14 row 4) its
    /// revoked-with-a-live-lease retry of `end_lease`.
    Retry,
}

/// **The recall's answer to its `end_lease`** (pure; doc 18 cut IC14 row 32).
#[must_use]
pub(crate) fn recall_outcome<E>(ended: &Result<LeaseEnd, E>) -> RecallOutcome {
    match ended {
        Ok(LeaseEnd::Ended | LeaseEnd::AlreadyEnded) => RecallOutcome::Return,
        Ok(LeaseEnd::Retrying) | Err(_) => RecallOutcome::Retry,
    }
}

/// The asker's own `Active` enrolment with a live charter, or the refusal
/// (`+coop/buyout`'s check 1, shared by `+coop/release`, and since doc 20 cut SM.3a
/// by `+tv`, `+tv/lease` and `+coop/refit`).
pub(crate) fn chartered(ctx: &Ctx<'_>) -> Result<Enrolment, String> {
    let w: &World = ctx.w;
    let actor = ctx.actor;
    let own = own_enrolment(w, actor);
    let e = match account_books(w, actor, own.as_ref()) {
        Books::Absent => return Err(NO_CHARTER.to_owned()),
        Books::Unreadable => {
            tracing::error!(
                "tacsim: +coop/release found an unreadable enrolment for the asker's account; \
                 staff repair the tacsim:coop: document"
            );
            return Err(BOOKS_UNREADABLE.to_owned());
        }
        Books::Present(e) if e.pilot != actor => return Err(NO_CHARTER.to_owned()),
        Books::Present(e) => e.clone(),
    };
    match e.status {
        EnrolmentStatus::Revoked => {
            return Err(named("Your charter of the {name} ({mark}) has ended.", &e));
        }
        EnrolmentStatus::Graduated => return Err(named(ALREADY_GRADUATED, &e)),
        EnrolmentStatus::BuyoutPending => return Err(named(BUYOUT_PENDING, &e)),
        EnrolmentStatus::Active => {}
    }
    if live_charter(w, actor).is_none_or(|charter| charter.contract.0 != e.lease) {
        return Err(named(CHARTER_NOT_IN_FORCE, &e));
    }
    Ok(e)
}

/// **`+coop/release`** (doc 18 §4.1): the pilot's voluntary recall, under the same
/// rule as dormancy (L16); the stake is untouched (no post anywhere in the recall).
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn release_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx.args.raw().trim().is_empty() {
        return Err(CmdError::msg(RELEASE_USAGE));
    }
    let e = chartered(ctx).map_err(CmdError::msg)?;
    if let Some((_, line)) = recall_block(ctx, &e) {
        return Err(CmdError::msg(line));
    }
    let Some(port) = programme_port(ctx.w) else {
        return Err(CmdError::msg(crate::port::coop::NOT_OPEN_HERE));
    };
    let (office, office_name, port_label) = office_of(ctx.w, &port);
    let here = ctx.w.get(ctx.actor).and_then(|o| o.location);
    if office.is_none() || here != office {
        return Err(CmdError::msg(
            RELEASE_WHERE
                .replace("{office}", &office_name)
                .replace("{port}", &port_label),
        ));
    }
    let done = recall(ctx, &e, RecallCause::Released).map_err(|(_, line)| CmdError::msg(line))?;
    if done.lease_retrying {
        say(ctx, RELEASE_RETRYING.replace("{mark}", &e.mark));
        return Ok(());
    }
    say(
        ctx,
        named(RELEASED, &e).replace("{countersign}", COUNTERSIGN),
    );
    if done.tank > 0 || done.locker > 0 {
        say(
            ctx,
            FORFEIT_LINE
                .replace("{tank}", &group_thousands(done.tank))
                .replace("{locker}", &group_thousands(done.locker)),
        );
    }
    Ok(())
}

/// The player `text` names, globally (reviewer F2: GOD is never standing with the
/// pilot, so `match_object`'s near-only bare name is never used): `#N`, else one
/// leading `*` stripped and `World::resolve_player`; either way a `Player`.
fn resolve_pilot(w: &World, text: &str) -> Option<ObjId> {
    let text = text.trim();
    let found = match text.strip_prefix('#') {
        Some(number) => number.parse::<u32>().ok().map(ObjId),
        None => w.resolve_player(text.strip_prefix('*').unwrap_or(text)),
    }?;
    w.get(found)
        .is_some_and(|o| o.kind == ObjKind::Player)
        .then_some(found)
}

/// **`@coop/revoke <pilot>=<why>`** (doc 18 §4.1, 18a §4; staff, attributed; lead
/// rulings L9, L16): the same recall a week without a departure makes, now.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn revoke_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    crate::commands::require_staff(ctx)?;
    if programme_port(ctx.w).is_none() {
        return Err(CmdError::msg(crate::port::coop::NOT_OPEN_HERE));
    }
    let lhs = ctx.args.lhs().trim().to_owned();
    let Some(rhs) = ctx.args.rhs().map(str::to_owned) else {
        return Err(CmdError::msg(REVOKE_USAGE));
    };
    if !ctx.switches.has("revoke") || lhs.is_empty() {
        return Err(CmdError::msg(REVOKE_USAGE));
    }
    let why = rhs.trim().to_owned();
    if why.is_empty() || why.len() > MAX_REVOKE_WHY_BYTES {
        return Err(CmdError::msg(REVOKE_WHY_BOUNDS));
    }
    let Some(pilot) = resolve_pilot(ctx.w, &lhs) else {
        return Err(CmdError::msg(NO_SUCH_PILOT));
    };
    let pilot_name = crate::port::coop::name_of(ctx.w, pilot);
    let e = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| store.enrolment_of(pilot))
        .filter(|e| e.status == EnrolmentStatus::Active)
        .filter(|e| live_charter(ctx.w, pilot).is_some_and(|c| c.contract.0 == e.lease))
        .cloned();
    let Some(e) = e else {
        return Err(CmdError::msg(
            NO_COOP_CHARTER.replace("{pilot}", &pilot_name),
        ));
    };
    // Doc 20 cut SM.3a: a revoke repossesses the rigs in the bay first; a refusal
    // after a repossession wrote anything pins the deferred queue, so RAM and the
    // store agree on the rigs already back in the shed.
    if let Some((block, line)) = recall_block_clearing(ctx, &e) {
        if block == RecallBlock::Bay {
            ctx.out.pin_deferred();
        }
        return Err(CmdError::msg(format!("{NOT_REVOKED}{line}")));
    }
    let by = ctx.actor;
    let done = recall(
        ctx,
        &e,
        RecallCause::Revoked {
            by,
            why: why.clone(),
        },
    )
    .map_err(|(_, line)| CmdError::msg(format!("{NOT_REVOKED}{line}")))?;
    tracing::warn!(
        staff = by.0,
        pilot = pilot.0,
        hull = e.hull.0,
        why = %why,
        "tacsim: @coop/revoke ended a First Captain's charter"
    );
    let reply = if done.lease_retrying {
        REVOKED_RETRYING
    } else {
        REVOKED_REPLY
    };
    say(
        ctx,
        named(reply, &e)
            .replace("{pilot}", &pilot_name)
            .replace("{why}", &why),
    );
    Ok(())
}

// ---- dormancy --------------------------------------------------------------------

/// **Whether a charter has gone idle** (L3; pure): the lease is at least `days` real
/// days old at `now`, and there is no departure since `now − D` — no burn at or after
/// `cutoff_ic` (the IC second of `now − D`) and no reach at or after `now − D`
/// (`>=` counts as a departure). An overflowing `D` never recalls.
#[must_use]
pub(crate) fn idle(
    now: i64,
    leased_at: i64,
    days: u32,
    last_burn_ic: Option<i64>,
    cutoff_ic: i64,
    last_reach_utc: Option<i64>,
) -> bool {
    let Some(span) = i64::from(days).checked_mul(DAY_SECS) else {
        return false;
    };
    let Some(cutoff) = now.checked_sub(span) else {
        return false;
    };
    if now.saturating_sub(leased_at) < span {
        return false;
    }
    let burned = last_burn_ic.is_some_and(|ic| ic >= cutoff_ic);
    let reached = last_reach_utc.is_some_and(|utc| utc >= cutoff);
    !(burned || reached)
}

/// **The dormancy clock** (lead rulings 2f/2i, L3): the latest burned ticket and the
/// plan's Done departs, against `leased_at`. `Err(())` on any read error — fail safe:
/// never recall.
pub(crate) fn dormant(ctx: &Ctx<'_>, e: &Enrolment, days: u32) -> Result<bool, ()> {
    let w: &World = ctx.w;
    let now = w.now();
    let span = i64::from(days).checked_mul(DAY_SECS).ok_or(())?;
    if now.saturating_sub(e.leased_at) < span {
        return Ok(false);
    }
    let cutoff = now.checked_sub(span).ok_or(())?;
    let cutoff_ic = crate::port::when::ic_at_real(w, cutoff);
    let store = w.module_state::<TacsimStore>().ok_or(())?;
    // 1. The latest burned ticket, scanned down from `last_seq`.
    let last_burn_ic = match store.voyage_state(e.hull) {
        VoyageState::Unreadable => return Err(()),
        VoyageState::Absent => None,
        VoyageState::Present(voyage) => {
            let floor = voyage
                .last_seq
                .saturating_add(1)
                .saturating_sub(DORMANCY_TICKET_SCAN)
                .max(1);
            let mut found = None;
            let mut seq = voyage.last_seq;
            while seq >= floor && seq > 0 {
                if let Some(ticket) = ticket_at(ctx, TicketId { hull: e.hull, seq })? {
                    found = Some(ticket.burned_at);
                    break;
                }
                seq = seq.saturating_sub(1);
            }
            found
        }
    };
    // 2. The plan's Done departs, authored by the pilot.
    let last_reach_utc = match orders_state(w, e.hull) {
        OrdersState::Unreadable => return Err(()),
        OrdersState::Absent => None,
        OrdersState::Present(doc) if doc.author != e.pilot => None,
        OrdersState::Present(doc) => doc
            .orders
            .iter()
            .filter(|order| order.state == OrderState::Done)
            .filter(|order| matches!(order.what, What::Depart { .. }))
            .filter_map(|order| order.reached.as_ref().map(|reach| reach.utc))
            .max(),
    };
    Ok(idle(
        now,
        e.leased_at,
        days,
        last_burn_ic,
        cutoff_ic,
        last_reach_utc,
    ))
}

// ---- the recall arm on the port tick ------------------------------------------------

/// The programme's dormancy days.
fn dormancy_days(w: &World) -> Option<u32> {
    programme_port(w).map(|port| port.programme.dormancy_real_days)
}

/// Whether `e` is the hull's governing enrolment (L4: an old tenant never shadows the
/// current one).
fn governs(store: &TacsimStore, e: &Enrolment) -> bool {
    store
        .enrolment_for_hull(e.hull)
        .is_some_and(|g| g.account == e.account)
}

/// **A charter ended whose hull is not back**: `Revoked`, the hull's governing
/// enrolment, no live Trust lease on the hull, and the stowage vault still the pilot's.
fn awaiting_return(w: &World, store: &TacsimStore, e: &Enrolment) -> bool {
    if e.status != EnrolmentStatus::Revoked || !governs(store, e) {
        return false;
    }
    let Some(trust) = programme_port(w).and_then(|port| port.trust()) else {
        return false;
    };
    if trust_lease_on(w, &trust, e.hull).is_some() {
        return false;
    }
    let stowage = hull_view(w, e.hull).and_then(|view| view.stowage());
    stowage.is_some_and(|stowage| {
        w.module_state::<EconomyStore>().is_some_and(|economy| {
            economy
                .snapshot()
                .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
                .is_some_and(|v| v.owner == PrincipalId::Object(e.pilot))
        })
    })
}

/// **A Revoked charter whose own lease is still live** (doc 18 cut IC14 row 4; lead
/// ruling L10): the state a recall whose `end_lease` returned `Err` leaves — the
/// enrolment `Revoked`, the hull's governing enrolment, and the Trust's live lease on
/// the hull **is** the enrolment's own `e.lease`. The recall arm retries `end_lease`
/// for it under the backoff until the lease ends, then runs the return half.
fn revoked_with_live_lease(w: &World, store: &TacsimStore, e: &Enrolment) -> bool {
    if e.status != EnrolmentStatus::Revoked || !governs(store, e) {
        return false;
    }
    let Some(trust) = programme_port(w).and_then(|port| port.trust()) else {
        return false;
    };
    trust_lease_on(w, &trust, e.hull).is_some_and(|lease| lease.id.0 == e.lease)
}

/// **Whether `e` waits on a re-charter that is due this fire** (doc 18 cut IC14 row
/// 12; lead ruling L11): the governing enrolment, `relet_pending` (BuyoutPending, its
/// buyout refunded, its old lease terminal), and the recorded attempt's fire reached —
/// or no attempt recorded yet (first sight: the observer made the first).
fn relet_due(w: &World, store: &TacsimStore, state: &PortTickState, e: &Enrolment) -> bool {
    governs(store, e)
        && crate::port::buyout::relet_pending(w, e)
        && state
            .relet_attempts
            .get(&e.hull)
            .is_none_or(|(_, due)| state.fires >= *due)
}

/// Whether `e` is a live charter of its hull (`Active`, the live charter naming its
/// lease on its hull).
fn live(w: &World, e: &Enrolment) -> bool {
    e.status == EnrolmentStatus::Active
        && live_charter(w, e.pilot).is_some_and(|c| c.contract.0 == e.lease && c.hull == e.hull)
}

/// Page once per hull per boot (the recall arm's pages share one set).
fn page_once(state: &mut PortTickState, hull: ObjId) -> bool {
    state.recall_warned.insert(hull.0)
}

/// **The governing enrolment of each hull row** (doc 18 cut IC14 row 18), in row order:
/// one per hull that has one, so the recall arm visits at most the pool's rows however
/// many enrolments a world has accumulated.
fn governing_rows<'a>(store: &'a TacsimStore, hulls: &[ObjId]) -> Vec<&'a Enrolment> {
    hulls
        .iter()
        .filter_map(|hull| store.enrolment_for_hull(*hull))
        .collect()
}

/// **The recall agents this fire** (agent = the hull, principal = the enrolment's
/// pilot): every **Dormant** charter and every hull **awaiting its return**, hulls in
/// their backoff and hulls with a pending voyage skipped. The pending-voyage skip
/// keeps the hull out of the launch arm's way (the walk refuses a duplicate agent),
/// so a filed hull never reaches `recall_block`'s `Cleared` refusal here: the page
/// lead 2f asks for ("`@tacsim/port clear #<hull>` first") is the launch arm's own
/// held-launch page (`launch_hold`), and this arm adds no second one. A burned ticket
/// is a departure (L3), and an unburned filing lapses after `LAUNCH_GRACE_FIRES`.
///
/// Doc 18 cut IC14 row 18: the walk is over the programme port's map **hull rows**
/// (bounded by the pool of eight), each read for its governing enrolment, never the
/// whole enrolment history. Cut IC14 rows 4 and 12 add two more due states: a
/// Revoked charter whose own lease is still live, and a refunded buyout whose
/// re-charter is due.
pub(crate) fn recall_agents(ctx: &Ctx<'_>, state: &mut PortTickState) -> Vec<(ObjId, ObjId)> {
    let w: &World = ctx.w;
    let fires = state.fires;
    state
        .recall_backoff
        .retain(|_, at| fires.saturating_sub(*at) < RECALL_RECHECK_FIRES);
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    let Some(days) = dormancy_days(w) else {
        return Vec::new();
    };
    let hulls: Vec<ObjId> = programme_port(w)
        .map(|port| port.map.hulls.iter().map(|row| ObjId(row.hull)).collect())
        .unwrap_or_default();
    // Doc 18 cut IC14 row 12: the re-charter bookkeeping is kept to hulls whose
    // governing enrolment still waits on one (bounded by the pool).
    state.relet_attempts.retain(|hull, _| {
        store
            .enrolment_for_hull(*hull)
            .is_some_and(|e| crate::port::buyout::relet_pending(w, e))
    });
    let mut agents = Vec::new();
    let mut dark: Vec<ObjId> = Vec::new();
    for e in governing_rows(store, &hulls) {
        // Doc 18 cut IC14 row 11: an unreadable sailing plan is skipped as a filed one
        // is (fail closed; the port tick pages it once).
        if state.recall_backoff.contains_key(&e.hull.0)
            || crate::port::voyage::voyage_blocks(w, e.hull)
        {
            continue;
        }
        let due = if live(w, e) {
            match dormant(ctx, e, days) {
                Ok(due) => due,
                Err(()) => {
                    dark.push(e.hull);
                    false
                }
            }
        } else {
            awaiting_return(w, store, e)
                || revoked_with_live_lease(w, store, e)
                || relet_due(w, store, state, e)
        };
        if due {
            agents.push((e.hull, e.pilot));
        }
    }
    for hull in dark {
        if page_once(state, hull) {
            tracing::error!(
                hull = hull.0,
                "tacsim: a chartered hull's dormancy cannot be read (an unreadable voyage, ticket \
                 or sailing plan); it is never recalled while the read fails; staff repair the \
                 document"
            );
        }
    }
    agents
}

/// **The recall step for one hull** (no submission, no swap): the governing enrolment
/// re-read; a **Dormant** charter is recalled, or — blocked — its plan held once
/// ([`HELD_DORMANT`]), staff paged once and the hull backed off; a hull **awaiting its
/// return** is returned, or — blocked — the lapsed author's plan held once
/// ([`HELD_RECALLED`]), paged once and backed off.
pub(crate) fn recall_step(ctx: &mut Ctx<'_>, state: &mut PortTickState, hull: ObjId) -> Step {
    if crate::port_tick::gone(ctx.w, hull) {
        return Step::Refused;
    }
    let Some(days) = dormancy_days(ctx.w) else {
        return Step::Idle;
    };
    let Some(e) = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| store.enrolment_for_hull(hull))
        .cloned()
    else {
        return Step::Idle;
    };
    if live(ctx.w, &e) {
        if dormant(ctx, &e, days) != Ok(true) {
            return Step::Idle;
        }
        // Doc 20 cut SM.3a: the dormancy recall repossesses the bay's rigs first.
        let blocked = match recall_block_clearing(ctx, &e) {
            None => match recall(ctx, &e, RecallCause::Dormant) {
                Ok(_) => return Step::Worked,
                Err(block) => block,
            },
            Some(block) => block,
        };
        let port = programme_port(ctx.w).map_or_else(String::new, |port| port_name(port.def));
        let reason = named(HELD_DORMANT, &e).replace("{port}", &port);
        let wrote = hold_plan(ctx, &e, &reason, true, true);
        if page_once(state, hull) {
            if blocked.0 == RecallBlock::Bay {
                tracing::error!(hull = hull.0, pilot = e.pilot.0, block = ?blocked.0, "{BAY_PAGE}");
            } else {
                tracing::error!(
                    hull = hull.0,
                    pilot = e.pilot.0,
                    block = ?blocked.0,
                    "tacsim: the hull is dormant but cannot be recalled where it is; get it docked \
                     at Ceres with an empty hold, or `@coop/revoke` once it is"
                );
            }
        }
        state.recall_backoff.insert(hull.0, state.fires);
        return if wrote { Step::Worked } else { Step::Idle };
    }
    // ---- doc 18 cut IC14 row 4 (lead ruling L10): a Revoked charter's live lease ----
    let lease_live = ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(|store| revoked_with_live_lease(ctx.w, store, &e));
    if lease_live {
        return end_revoked_lease(ctx, state, &e);
    }
    // ---- doc 18 cut IC14 row 12 (lead ruling L11): a refunded buyout's re-charter ----
    let relet = ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(|store| relet_due(ctx.w, store, state, &e));
    if relet {
        return relet_step(ctx, state, &e);
    }
    let awaiting = ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(|store| awaiting_return(ctx.w, store, &e));
    if !awaiting {
        return Step::Idle;
    }
    match return_hull(ctx, &e) {
        Ok(_) => {
            crate::port_tick::notify(ctx, e.pilot, &named(RETURNED_NOTICE, &e));
            Step::Worked
        }
        Err((block, _)) => {
            let wrote = hold_plan(ctx, &e, &named(HELD_RECALLED, &e), true, true);
            if page_once(state, hull) {
                if block == RecallBlock::Bay {
                    tracing::error!(hull = hull.0, pilot = e.pilot.0, ?block, "{BAY_PAGE}");
                } else {
                    tracing::error!(
                        hull = hull.0,
                        pilot = e.pilot.0,
                        ?block,
                        "tacsim: a revoked hull is not back in the pool: it is not docked at Ceres \
                         with an empty hold; staff move its cargo or hull by hand"
                    );
                }
            }
            state.recall_backoff.insert(hull.0, state.fires);
            if wrote { Step::Worked } else { Step::Idle }
        }
    }
}

/// **End a Revoked charter's still-live lease, then return the hull** (doc 18 cut IC14
/// row 4; lead ruling L10): `end_lease` again — `Ended | AlreadyEnded` runs the return
/// half (the pilot told [`RETURNED_NOTICE`] on success); `Retrying` backs off with
/// nothing paged; `Err` pages once and backs off. Every due fire retries, so a later
/// success converges.
fn end_revoked_lease(ctx: &mut Ctx<'_>, state: &mut PortTickState, e: &Enrolment) -> Step {
    let Some((trust, _)) = trust_of(ctx.w) else {
        state.recall_backoff.insert(e.hull.0, state.fires);
        return Step::Idle;
    };
    let key = LeaseKey {
        lessor: trust,
        premises: e.hull,
    };
    match mush_economy::end_lease(ctx, mush_economy::ContractId(e.lease), &key) {
        Ok(LeaseEnd::Ended | LeaseEnd::AlreadyEnded) => match return_hull(ctx, e) {
            Ok(_) => {
                crate::port_tick::notify(ctx, e.pilot, &named(RETURNED_NOTICE, e));
                Step::Worked
            }
            Err((block, _)) => {
                // The lease is over now; the ordinary Return arm takes it from here.
                tracing::info!(
                    hull = e.hull.0,
                    ?block,
                    "tacsim: a revoked charter's lease ended; the hull returns once it may"
                );
                state.recall_backoff.insert(e.hull.0, state.fires);
                Step::Worked
            }
        },
        Ok(LeaseEnd::Retrying) => {
            state.recall_backoff.insert(e.hull.0, state.fires);
            Step::Idle
        }
        Err(error) => {
            if page_once(state, e.hull) {
                tracing::error!(
                    hull = e.hull.0,
                    lease = e.lease,
                    %error,
                    "tacsim: a revoked charter's lease will not end; the pilot still holds it; \
                     `@contract/show #<lease>`"
                );
            }
            state.recall_backoff.insert(e.hull.0, state.fires);
            Step::Idle
        }
    }
}

/// **One re-charter of a refunded buyout** (doc 18 cut IC14 row 12; lead ruling L11;
/// proposed Q-18-71). First sight (no attempt recorded: the observer made the first
/// and it failed): record attempt 1, due [`crate::port::buyout::relet_backoff`]`(1)`
/// fires on, and page once. Each due retry runs
/// [`crate::port::buyout::relet`] with its attempt number — the last
/// ([`crate::port::buyout::MAX_RELET_ATTEMPTS`]) gives up exactly as cut IC13 built it
/// — and on another failure records the attempt, due after the doubling backoff.
fn relet_step(ctx: &mut Ctx<'_>, state: &mut PortTickState, e: &Enrolment) -> Step {
    let Some(&(made, _)) = state.relet_attempts.get(&e.hull) else {
        state.relet_attempts.insert(
            e.hull,
            (
                1,
                state
                    .fires
                    .saturating_add(crate::port::buyout::relet_backoff(1)),
            ),
        );
        if page_once(state, e.hull) {
            tracing::error!(
                hull = e.hull.0,
                pilot = e.pilot.0,
                buyout = e.buyout,
                "tacsim: a refunded buyout's re-charter failed; retrying under a backoff; \
                 `@contract/show #<buyout>`"
            );
        }
        return Step::Idle;
    };
    let attempt = made.saturating_add(1);
    let give_up = attempt >= crate::port::buyout::MAX_RELET_ATTEMPTS;
    if crate::port::buyout::relet(ctx, e, attempt, give_up) || give_up {
        state.relet_attempts.remove(&e.hull);
    } else {
        state.relet_attempts.insert(
            e.hull,
            (
                attempt,
                state
                    .fires
                    .saturating_add(crate::port::buyout::relet_backoff(attempt)),
            ),
        );
    }
    Step::Worked
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: i64 = 7 * DAY_SECS;

    /// Doc 18 cut IC14 row 18: twenty lifetime enrolments over eight hulls — twelve
    /// ended, eight governing — give the recall arm eight rows, each the hull's latest.
    #[test]
    fn ic14_the_recall_walk_visits_one_row_per_hull() {
        let mut store = TacsimStore::default();
        let base = |n: u32| Enrolment {
            schema: crate::port::coop::ENROLMENT_SCHEMA,
            account: mush_core::AccountId::mint(&format!("pilot{n}"), i64::from(n) + 1)
                .expect("an account"),
            pilot: ObjId(1_000 + n),
            programme: mush_core::CanonicalId::new("coop.first_captain").expect("id"),
            pack: mush_core::CanonicalId::new("tacsim.pack.lane").expect("id"),
            entry: mush_core::CanonicalId::new("ceres_port").expect("id"),
            hull: ObjId(500 + n % 8),
            mark: format!("FPC-D-040{}", n % 8 + 1),
            laid_up_name: "Slack Water".to_owned(),
            name: "Salt Moth".to_owned(),
            lease: u64::from(n) + 7,
            vault: 3,
            credit_account: 11,
            stake_minor: 2_500_000,
            share_permille: 250,
            buyout_minor: 9_000_000,
            enrolled_at: 1_000,
            status: EnrolmentStatus::Active,
            leased_at: 1_000,
            buyout: None,
            ended_at: None,
            ended: None,
            sales: 0,
            margin_minor: 0,
            shares_minor: 0,
        };
        for n in 0..20_u32 {
            let mut e = base(n);
            if n < 12 {
                e.status = EnrolmentStatus::Revoked;
                e.ended_at = Some(2_000 + i64::from(n));
                e.ended = Some(EndCause::LeaseEnded);
            }
            store.enrolments.insert(e.account, e);
        }
        let hulls: Vec<ObjId> = (0..8).map(|n| ObjId(500 + n)).collect();
        let rows = governing_rows(&store, &hulls);
        assert_eq!(rows.len(), 8, "one row per hull");
        for (row, hull) in rows.iter().zip(&hulls) {
            assert_eq!(row.hull, *hull);
            assert_eq!(row.status, EnrolmentStatus::Active, "the governing one");
        }
    }

    /// Doc 18 cut IC14 row 32: the recall's answer to its `end_lease` — an ended lease
    /// returns the hull now; `Retrying` and an `Err` leave the return to the recall arm
    /// (`lease_retrying: true`, the enrolment already written Revoked first).
    #[test]
    fn ic14_a_retrying_or_refused_lease_end_leaves_the_return_to_the_arm() {
        assert_eq!(
            recall_outcome::<()>(&Ok(LeaseEnd::Ended)),
            RecallOutcome::Return
        );
        assert_eq!(
            recall_outcome::<()>(&Ok(LeaseEnd::AlreadyEnded)),
            RecallOutcome::Return
        );
        assert_eq!(
            recall_outcome::<()>(&Ok(LeaseEnd::Retrying)),
            RecallOutcome::Retry
        );
        assert_eq!(recall_outcome(&Err("refused")), RecallOutcome::Retry);
    }

    /// Doc 18 cut IC13 (L3): dormancy is judged from durable departures only.
    #[test]
    fn ic13_dormancy_is_judged_from_durable_departures() {
        let leased = 1_000_000;
        let now = leased + D + 3_600;
        let cutoff = now - D;
        let cutoff_ic = 50_000_000;
        // The lease is younger than D: never idle, whatever else.
        assert!(!idle(leased + D - 1, leased, 7, None, cutoff_ic, None));
        // Old enough, no departure at all: idle.
        assert!(idle(now, leased, 7, None, cutoff_ic, None));
        // A burn after the cutoff keeps the charter.
        assert!(!idle(now, leased, 7, Some(cutoff_ic + 1), cutoff_ic, None));
        // A reach after the cutoff keeps the charter.
        assert!(!idle(now, leased, 7, None, cutoff_ic, Some(cutoff + 1)));
        // Both older than the cutoff: idle.
        assert!(idle(
            now,
            leased,
            7,
            Some(cutoff_ic - 1),
            cutoff_ic,
            Some(cutoff - 1)
        ));
        // The exact boundary counts as a departure (`>=`).
        assert!(!idle(now, leased, 7, Some(cutoff_ic), cutoff_ic, None));
        assert!(!idle(now, leased, 7, None, cutoff_ic, Some(cutoff)));
        // The lease exactly D old is old enough.
        assert!(idle(leased + D, leased, 7, None, cutoff_ic, None));
    }
}
