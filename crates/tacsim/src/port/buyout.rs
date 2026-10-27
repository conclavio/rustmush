//! **The buyout** (doc 18 cut IC13; doc 18 §4.1 "The buyout", D-18-21, D-18-23,
//! D-18-25; lead rulings L1, L2, L7, L10, L11, L14 of the IC13 brief).
//!
//! `+coop/buyout` at the Registry & Co-op desk turns a First Captain's charter into
//! title, through the economy's own contracts and nothing else: **the economy's
//! contracts are the record, and the enrolment document follows them.**
//!
//! - **The contract** is the market's asset-purchase shape (`buyout_spec`, the
//!   `Representation::Asset` arm of `mush_market`'s `purchase_spec`): issued by the
//!   pilot (so the Trust's issuer cap is untouched), `TitleHeld{hull, Trust}` and a
//!   real-clock `Before` window ([`BUYOUT_WINDOW_SECS`], L11), the enrolment's frozen
//!   `buyout_minor` escrowed from the pilot's account, one `Exact` payout to the
//!   Trust's programme account, and one `TitleTransfer` Trust → pilot.
//! - **The step order** (lead ruling L1, which wins over doc 18 §4.1's 3-4-5): offer
//!   → the pilot's escrow → the enrolment `BuyoutPending` → `end_lease` → the Trust's
//!   acceptance through the party-cap source. `cancel_contract` refuses an `Active`
//!   contract with deliverables, so the Trust accepts **last**: at `end_lease` the
//!   contract is still `Offered` and the verb can still cancel its own offer. Every
//!   cancel goes through `cancel_own`, which treats `cancel_contract`'s `Ok(false)`
//!   (the refund unwind retrying in the sweep) as not done.
//! - **The settlement** is the economy sweep's, never this verb's: the observer
//!   (`CoopObserver`, `coop.rs`) hands every terminal state of the buyout or of the
//!   lease to `settle_buyout`, which reads both contracts from the snapshot (L7)
//!   and graduates on `Settled`, re-charters the pilot on `Cancelled`/`Expired` once
//!   the old lease is terminal, and otherwise waits (`buyout_verdict`).
//!
//! Between the command and the settlement (at most one 30 s sweep) the pilot holds
//! neither charter nor title, so board, undock and the queue refuse (owner item O6).

use mush_core::{CmdError, Ctx, ObjId, World};
use mush_economy::{
    AcceptOutcome, Amount, ConsiderationSpec, ContractError, ContractId, ContractRoles,
    ContractSpec, ContractState, ContractStateKind, EconomicClock, EconomicDeadline,
    EconomicSchedule, EconomicVisibility, EconomyError, EconomyStore, EvidenceKind, Expiry,
    GracePolicy, LeaseEnd, LeaseKey, LeaseSpec, ObligationTemplate, OutcomeTerms, PartyCap,
    PartyCapSource, PayoutShare, Share, TypedCondition,
};
use mush_economy::{AccountId, Deliverable};
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::port::coop::{
    BOOKS_UNREADABLE, Books, CHARTER_NOT_IN_FORCE, COUNTERSIGN, EndCause, Enrolment,
    EnrolmentStatus, NO_CHARTER, account_books, live_charter, money_minor, office_of,
    own_enrolment, port_name, programme_port, say, titled_to,
};
use crate::port::orders::{OrdersState, orders_state};
use crate::port::ship::{Berth, berth_of};
use crate::port::voyage::{VoyageState, goods_pending_aboard, pending_of};
use crate::store::{TacsimStore, put_enrolment};

/// The buyout contract's window, real seconds (lead ruling L11): the sweep settles
/// within 30 s, so the window only bounds a stuck settlement, which then expires to
/// a refund and a re-lease.
pub const BUYOUT_WINDOW_SECS: i64 = 3_600;

/// `+coop/buyout` with an argument.
pub const BUYOUT_USAGE: &str = "Usage: +coop/buyout";
/// A graduate asks again: `{name}`, `{mark}`.
pub const ALREADY_GRADUATED: &str =
    "The {name} ({mark}) is already yours: the Registry has her in your name.";
/// A buyout already being entered: `{name}`, `{mark}`.
pub const BUYOUT_PENDING: &str = "Your buyout of the {name} ({mark}) is already being entered.";
/// Under way: `{name}`, `{port}`.
pub const BUYOUT_UNDER_WAY: &str =
    "The {name} is under way; the buyout is signed with her docked at {port}.";
/// Docked elsewhere, or anywhere else: `{name}`, `{port}`.
pub const BUYOUT_NOT_HERE: &str = "The buyout is signed with the {name} docked at {port}.";
/// A clearance filed (or the sailing plan's voyage unreadable): `{name}`.
pub const CLEARED_WAIT: &str =
    "The {name} has clearance filed; nothing changes hands until it is flown or stood down.";
/// A queued step at the counter (or the queue unreadable): `{name}`.
pub const INFLIGHT_WAIT: &str =
    "The {name}'s sailing plan has a step at the counter; nothing changes hands until it is done.";
/// Goods still moving through the hold: `{name}`.
pub const GOODS_WAIT: &str = "Goods are still moving through the {name}'s hold; nothing changes hands until they are in or out.";
/// Away from the desk: `{office}`, `{port}`.
pub const BUYOUT_WHERE: &str = "The buyout is signed at the {office} in {port}.";
/// **A rig in the bay** (doc 20 cut SM.3a; reviewer F1(a)): `{n}` (the lowest live
/// record's slot, or `?`), `{name}`. No rig rides into a graduation.
pub const BUYOUT_BAY: &str =
    "Rig {n} is still in the {name}'s bay; return it to Occator before the buyout.";
/// A step that should not fail did; nothing was bought.
pub const BUYOUT_FAILED: &str =
    "The Registry could not enter your buyout; nothing was bought and your charter stands.";
/// Short of the price: `{buyout}`, `{available}`.
pub const BUYOUT_FUNDS: &str = "The buyout is {buyout}; you have {available} available.";
/// A cancel whose refund is still retrying in the sweep: `{buyout}`.
pub const BUYOUT_REFUND_PENDING: &str = "The Registry could not enter your buyout. The {buyout} held for it is still on its way back to you, and staff have been told.";
/// The lease end is retrying and the buyout's escrow came back.
pub const BUYOUT_HELD: &str = "The Registry could not close your charter just now; nothing was bought, your money is back, and staff have been told.";
/// The interim reply: `{name}`, `{mark}`, `{buyout}`, `{countersign}`.
pub const BUYOUT_FILED: &str = "The Registry writes the title transfer for the {name} ({mark}): {buyout} is held against it and the Co-op's charter is closed. The title passes to you at the next settlement. {countersign} countersigns.";
/// **The settlement notice** — doc 18 §2 step 19's line, verbatim with its two values
/// (owner item O4: the cut shows it at settlement, not on the verb): `{name}`,
/// `{buyout}`.
pub const GRADUATED: &str = "The Registry writes your name over the Co-op's mark. The {name} is yours: {buyout} paid; the charter ends; no further share.";
/// A refunded buyout, the charter renewed: `{name}`, `{mark}`, `{buyout}`.
pub const BUYOUT_REFUNDED: &str = "Your buyout of the {name} ({mark}) was not completed; the {buyout} held for it is yours again, and the Co-op charters her to you again.";
/// A refunded buyout whose charter could not be renewed: `{name}`, `{mark}`,
/// `{buyout}`.
pub const BUYOUT_UNLEASED: &str = "Your buyout of the {name} ({mark}) was not completed and the {buyout} is yours again, but her charter could not be renewed. Staff have been told.";
/// A refunded buyout whose first re-charter failed (doc 18 cut IC14 row 12; lead ruling
/// L11; proposed Q-18-71): told once, at the first failure; the recall arm retries
/// (owner voice, cut IC14 O1). `{name}`, `{mark}`, `{buyout}`.
pub const RELET_PENDING: &str = "Your buyout of the {name} ({mark}) was not completed and the {buyout} is yours again. Her charter is being renewed; she waits at her berth until it is, and staff have been told.";
/// How many re-charters one refunded buyout gets per boot (doc 18 cut IC14 row 12;
/// lead ruling L11): the observer's first and the recall arm's retries together. After
/// the last fails the enrolment ends exactly as cut IC13 built it (Revoked,
/// `EndCause::LeaseEnded`, [`BUYOUT_UNLEASED`]), so at most this many lease contracts
/// are minted per failure per boot; a reboot resets the count (bounded by reboots).
pub const MAX_RELET_ATTEMPTS: u8 = 6;
/// The longest gap between two re-charters, in port-tick fires (one hour at the 5 s
/// fire; doc 18 cut IC14 row 12).
pub const MAX_RELET_BACKOFF_FIRES: u64 = 720;

/// **The fires to wait after the `n`-th failed re-charter** (doc 18 cut IC14 row 12;
/// lead ruling L11): `RECALL_RECHECK_FIRES << (n - 1)`, capped at
/// [`MAX_RELET_BACKOFF_FIRES`] — about 2, 4, 8, 16 and 32 minutes between the six
/// attempts. Pure; `n` below 1 reads as 1.
#[must_use]
pub fn relet_backoff(n: u8) -> u64 {
    let shift = u32::from(n.max(1).saturating_sub(1));
    crate::port::recall::RECALL_RECHECK_FIRES
        .checked_shl(shift)
        .unwrap_or(u64::MAX)
        .min(MAX_RELET_BACKOFF_FIRES)
}

/// **Whether `e` waits on a re-charter** (doc 18 cut IC14 row 12): `BuyoutPending` with
/// its buyout refunded (Cancelled or Expired) and its old lease terminal — exactly
/// [`buyout_verdict`]'s `Relet`, read from the economy. The executor holds the plan
/// under `HELD_RELET` then, and the recall arm offers the hull for the retry.
#[must_use]
pub(crate) fn relet_pending(w: &World, e: &Enrolment) -> bool {
    if e.status != EnrolmentStatus::BuyoutPending {
        return false;
    }
    let Some(buyout) = e.buyout else {
        return false;
    };
    w.module_state::<EconomyStore>().is_some_and(|economy| {
        let snapshot = economy.snapshot();
        let kind = snapshot
            .contract(ContractId(buyout))
            .map(|c| c.state.kind());
        let lease_terminal = snapshot
            .contract(ContractId(e.lease))
            .is_none_or(|c| c.state.is_terminal());
        kind.is_some_and(|kind| buyout_verdict(kind, lease_terminal) == Verdict::Relet)
    })
}

/// `{name}` and `{mark}` filled from an enrolment.
fn named(template: &str, e: &Enrolment) -> String {
    template
        .replace("{name}", &e.name)
        .replace("{mark}", &e.mark)
}

/// **The buyout contract** (D-18-21's shape; the market's `Representation::Asset`
/// arm): pure, so the shape is tested without a world.
#[must_use]
#[allow(clippy::too_many_arguments)] // the contract's eight facts, each named
pub(crate) fn buyout_spec(
    pilot_account: AccountId,
    trust: &PrincipalId,
    trust_account: AccountId,
    hull: ObjId,
    pilot: ObjId,
    mark: &str,
    price: Amount,
    now: i64,
) -> ContractSpec {
    let window = EconomicDeadline {
        clock: EconomicClock::RealUtc,
        at: now.saturating_add(BUYOUT_WINDOW_SECS),
    };
    let me = PrincipalId::Object(pilot);
    ContractSpec {
        name: SmolStr::new(format!("First Captain buyout, {mark}")),
        // `None`: the pilot is the issuer, so the Trust's issuer cap is untouched.
        scope: None,
        parties: vec![trust.clone()],
        considerations: vec![ConsiderationSpec {
            account: pilot_account,
            amount: price,
        }],
        outcomes: vec![OutcomeTerms {
            name: SmolStr::new_static("bought out"),
            when: vec![0, 1],
            payouts: vec![PayoutShare {
                to: trust_account,
                share: Share::Exact(price),
            }],
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences: vec![0],
        }],
        remainder_to: pilot_account,
        conditions: vec![
            TypedCondition::TitleHeld {
                object: hull,
                by: trust.clone(),
            },
            TypedCondition::Before { deadline: window },
        ],
        adjudicator: None,
        evidence: vec![EvidenceKind::TitleHeld, EvidenceKind::Before],
        deadline: Some(window),
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles {
            author: Some(me.clone()),
            payer: Some(me.clone()),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables: vec![Deliverable::TitleTransfer {
            object: hull,
            from: trust.clone(),
            to: me,
        }],
    }
}

/// **Cancel the verb's own buyout offer** (lead ruling L7; reviewer F5): `true` only
/// when the contract is terminal now. `cancel_contract`'s `Ok(false)` (the refund
/// unwind was refused: the contract stays in force with its escrow reserved and
/// `pending` set while the sweep retries) and every `Err` are logged with the runbook
/// and answer `false`.
fn cancel_own(ctx: &mut Ctx<'_>, id: ContractId) -> bool {
    let outcome = mush_economy::cancel_contract(ctx, id);
    if matches!(outcome, Ok(true)) {
        return true;
    }
    tracing::error!(
        contract = id.0,
        ?outcome,
        "tacsim: +coop/buyout could not cancel its own buyout; the buyout's refund is retrying \
         in the economy sweep; `@contract/show #<id>`; the observer re-charters the pilot once \
         it lands"
    );
    false
}

/// A refusal after the first write: the deferred queue is pinned so RAM and the
/// store agree on the residue (`failed_after_writes`'s reason, `coop.rs`).
fn refuse_after_writes(ctx: &mut Ctx<'_>, line: String) -> CmdError {
    ctx.out.pin_deferred();
    CmdError::msg(line)
}

/// **Whether the pilot has an open sale** (lead 2h): a quote they sell into that is
/// activated, unsettled, and whose contract stands non-terminal.
fn seller_quote_open(w: &World, pilot: ObjId) -> bool {
    let (Some(market), Some(economy)) = (
        w.module_state::<MarketStore>(),
        w.module_state::<EconomyStore>(),
    ) else {
        return false;
    };
    market
        .quotes_of_seller(&PrincipalId::Object(pilot))
        .any(|quote| {
            quote.activated
                && quote.settled.is_none()
                && quote.contract.is_some_and(|contract| {
                    economy
                        .snapshot()
                        .contract(contract)
                        .is_some_and(|c| !c.state.is_terminal())
                })
        })
}

/// What is in flight on a hull (cut IC13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Busy {
    /// A clearance filed, or the voyage document unreadable.
    Cleared,
    /// A queued step at the counter, or the sailing plan unreadable.
    Inflight,
    /// Goods still moving through the hold, or an open sale of the pilot's.
    Goods,
}

/// **Nothing changes hands while anything is in flight on the hull** (the buyout's
/// checks 3–5 and the recall's 2–4, cut IC13): a filed clearance (or an unreadable
/// voyage), a queued step at the counter (or an unreadable queue), goods still moving
/// through the hold or an open sale of the pilot's. `Some` names the first, with its
/// line.
pub(crate) fn in_flight(w: &World, e: &Enrolment) -> Option<(Busy, String)> {
    in_flight_on(w, e.hull, e.pilot, mush_economy::VaultId(e.vault), &e.name)
}

/// The same flight and settlement guards for current hull facts, including a
/// title holder with no governing charter document and a lessee returning a rig.
pub(crate) fn in_flight_on(
    w: &World,
    hull: ObjId,
    pilot: ObjId,
    hold: mush_economy::VaultId,
    name: &str,
) -> Option<(Busy, String)> {
    let store = w.module_state::<TacsimStore>()?;
    let line = |text: &str| text.replace("{name}", name);
    if matches!(store.voyage_state(hull), VoyageState::Unreadable) || pending_of(w, hull).is_some()
    {
        return Some((Busy::Cleared, line(CLEARED_WAIT)));
    }
    match orders_state(w, hull) {
        OrdersState::Unreadable => return Some((Busy::Inflight, line(INFLIGHT_WAIT))),
        OrdersState::Present(orders) if orders.inflight.is_some() => {
            return Some((Busy::Inflight, line(INFLIGHT_WAIT)));
        }
        OrdersState::Present(_) | OrdersState::Absent => {}
    }
    if goods_pending_aboard(w, EndpointRef(hold)).is_some() || seller_quote_open(w, pilot) {
        return Some((Busy::Goods, line(GOODS_WAIT)));
    }
    None
}

/// Everything `+coop/buyout` resolved before its first write.
struct BuyoutPlan {
    enrolment: Enrolment,
    trust: PrincipalId,
    trust_account: AccountId,
    pilot_account: AccountId,
}

/// Checks 0–8 (lead ruling L14's order), read-only. `Err` carries the refusal.
/// Doc 20 cut SM.3a adds check 5b, after nothing in flight and before the office:
/// a rig in the hull's bay refuses [`BUYOUT_BAY`].
#[allow(clippy::too_many_lines)] // one ordered check list
fn plan_buyout(ctx: &Ctx<'_>) -> Result<BuyoutPlan, String> {
    let w: &World = ctx.w;
    let actor = ctx.actor;
    // 0. No argument.
    if !ctx.args.raw().trim().is_empty() {
        return Err(BUYOUT_USAGE.to_owned());
    }
    let Some(port) = programme_port(w) else {
        return Err(crate::port::coop::NOT_OPEN_HERE.to_owned());
    };
    // 1. The books, by gateway account.
    let own = own_enrolment(w, actor);
    let e = match account_books(w, actor, own.as_ref()) {
        Books::Absent => return Err(NO_CHARTER.to_owned()),
        Books::Unreadable => {
            tracing::error!(
                "tacsim: +coop/buyout found an unreadable enrolment for the asker's account; \
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
    // 2. The hull's berth (the pilot is aboard while she is under way, so this comes
    // before the office: L14).
    let port_label = port_name(port.def);
    match berth_of(w, e.hull) {
        Berth::UnderWay => {
            return Err(BUYOUT_UNDER_WAY
                .replace("{name}", &e.name)
                .replace("{port}", &port_label));
        }
        Berth::Docked { map, .. } if map.entry == port.entry => {}
        Berth::Docked { .. } | Berth::Elsewhere => {
            return Err(BUYOUT_NOT_HERE
                .replace("{name}", &e.name)
                .replace("{port}", &port_label));
        }
    }
    // 3–5. Nothing in flight.
    if let Some((_, line)) = in_flight(w, &e) {
        return Err(line);
    }
    // 5b. Doc 20 cut SM.3a (reviewer F1(a)): the bay is empty and no live lease record
    // names the hull (reviewer finding correctness/F2), so no rig rides into a
    // graduation (the pilot returns it to Occator first). An unreadable view fails
    // closed with the rigs' dark line (the SM.3a pre-commit round, correctness F5):
    // with no view the bay cannot be seen, so the buyout cannot know it is empty.
    let Some(view) = crate::port::ship::hull_view(w, e.hull) else {
        tracing::error!(
            hull = e.hull.0,
            pilot = actor.0,
            "tacsim: +coop/buyout cannot read the hull's view, so it cannot see the bay"
        );
        return Err(named(crate::port::tv::TV_DARK, &e));
    };
    if let Some(n) = crate::port::tv::bay_slot_word(w, &view) {
        return Err(BUYOUT_BAY.replace("{n}", &n).replace("{name}", &e.name));
    }
    // 6. At the office.
    let (office, office_name, port_label) = office_of(w, &port);
    let here = w.get(actor).and_then(|o| o.location);
    if office.is_none() || here != office {
        return Err(BUYOUT_WHERE
            .replace("{office}", &office_name)
            .replace("{port}", &port_label));
    }
    // 7. The economy and the parties resolve.
    let failed = |what: &str| {
        tracing::error!(
            what,
            pilot = actor.0,
            "tacsim: +coop/buyout could not resolve a step"
        );
        BUYOUT_FAILED.to_owned()
    };
    let economy = w
        .module_state::<EconomyStore>()
        .ok_or_else(|| failed("the economy"))?;
    if economy.is_frozen() {
        return Err(failed("the economy is frozen"));
    }
    let trust = port.trust().ok_or_else(|| failed("the Trust"))?;
    port.trust_scope()
        .ok_or_else(|| failed("the Trust's scope"))?;
    let trust_account = port
        .trust_account()
        .ok_or_else(|| failed("the Trust's account"))?;
    if !crate::port::market::PortPartyCaps.has_cap(w, &trust, PartyCap::Accept) {
        return Err(failed("the Trust's acceptance"));
    }
    if !titled_to(w, e.hull, &trust) {
        return Err(failed("the hull's title"));
    }
    let account = crate::port::credit::own_account(w, actor)
        .filter(|a| a.id.0 == e.credit_account)
        .ok_or_else(|| failed("the pilot's credit account"))?;
    // 8. The funds (doc 18 §4.1 step 1).
    let available = account
        .available()
        .ok_or_else(|| failed("the pilot's available"))?;
    if available.minor() < e.buyout_minor {
        return Err(BUYOUT_FUNDS
            .replace("{buyout}", &money_minor(ctx, e.buyout_minor))
            .replace("{available}", &money_minor(ctx, available.minor())));
    }
    Ok(BuyoutPlan {
        pilot_account: account.id,
        enrolment: e,
        trust,
        trust_account,
    })
}

/// **`+coop/buyout`** (doc 18 §4.1 "The buyout"; lead rulings L1, L14): every check
/// first, then offer → the pilot's escrow → `BuyoutPending` → `end_lease` → the
/// Trust's acceptance. The title passes at the next economy sweep (the observer
/// graduates); the verb answers [`BUYOUT_FILED`].
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
#[allow(clippy::too_many_lines)] // one ordered write list with each step's residue
pub(crate) fn buyout_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let plan = plan_buyout(ctx).map_err(CmdError::msg)?;
    let actor = ctx.actor;
    let e = plan.enrolment;
    let price = Amount::from_minor(e.buyout_minor);
    let buyout = money_minor(ctx, e.buyout_minor);
    let refund_pending = BUYOUT_REFUND_PENDING.replace("{buyout}", &buyout);
    // 1. The offer, as the pilot. Refused: nothing was written.
    let spec = buyout_spec(
        plan.pilot_account,
        &plan.trust,
        plan.trust_account,
        e.hull,
        actor,
        &e.mark,
        price,
        ctx.w.now(),
    );
    let id = match mush_economy::offer_contract(ctx, spec) {
        Ok(id) => id,
        Err(error) => {
            tracing::error!(%error, pilot = actor.0, "tacsim: +coop/buyout: the buyout was not offered");
            return Err(CmdError::msg(BUYOUT_FAILED));
        }
    };
    // 2. The pilot's escrow; the contract stays Offered (the Trust has not accepted).
    match mush_economy::accept_contract(ctx, id) {
        Ok(AcceptOutcome::Accepted { .. }) => {}
        other => {
            let short = matches!(
                other,
                Err(ContractError::Refused(EconomyError::InsufficientAvailable(
                    _
                )))
            );
            tracing::error!(contract = id.0, outcome = ?other, "tacsim: +coop/buyout: the pilot's escrow was not placed");
            let line = if !cancel_own(ctx, id) {
                refund_pending
            } else if short {
                let available = crate::port::credit::own_account(ctx.w, actor)
                    .and_then(mush_economy::Account::available)
                    .map_or(0, Amount::minor);
                BUYOUT_FUNDS
                    .replace("{buyout}", &buyout)
                    .replace("{available}", &money_minor(ctx, available))
            } else {
                BUYOUT_FAILED.to_owned()
            };
            return Err(refuse_after_writes(ctx, line));
        }
    }
    // 3. Mark the enrolment BEFORE the lease ends, so the observer's IC6 fold (which
    // acts on `Active` only) leaves the lease's Cancelled event alone.
    let pending = Enrolment {
        status: EnrolmentStatus::BuyoutPending,
        buyout: Some(id.0),
        ..e.clone()
    };
    put_enrolment(ctx, &pending);
    // 4. Close the charter.
    let key = LeaseKey {
        lessor: plan.trust.clone(),
        premises: e.hull,
    };
    match mush_economy::end_lease(ctx, ContractId(e.lease), &key) {
        Ok(LeaseEnd::Ended | LeaseEnd::AlreadyEnded) => {}
        Ok(LeaseEnd::Retrying) => {
            // The enrolment stays BuyoutPending: when the sweep finishes the lease
            // (and the refund, if it too retries) the observer re-charters the pilot.
            let refunded = cancel_own(ctx, id);
            tracing::error!(
                lease = e.lease,
                contract = id.0,
                "tacsim: +coop/buyout: the lease end is retrying; the observer re-charters the \
                 pilot when it completes; `@contract/show #<lease>`"
            );
            let line = if refunded {
                BUYOUT_HELD.to_owned()
            } else {
                refund_pending
            };
            return Err(refuse_after_writes(ctx, line));
        }
        Err(error) => {
            // Unreachable past check 7 (frozen and ownership were read): the lease
            // never moved, so the enrolment goes back to Active.
            tracing::error!(lease = e.lease, %error, "tacsim: +coop/buyout: the lease did not end");
            let refunded = cancel_own(ctx, id);
            put_enrolment(ctx, &e);
            let line = if refunded {
                BUYOUT_FAILED.to_owned()
            } else {
                refund_pending
            };
            return Err(refuse_after_writes(ctx, line));
        }
    }
    // 5. The Trust accepts through the party-cap source; the contract comes into force.
    match mush_economy::accept_contract_as(ctx, id, &plan.trust) {
        Ok(AcceptOutcome::Accepted {
            activated: true, ..
        }) => {}
        other => {
            tracing::error!(contract = id.0, outcome = ?other, "tacsim: +coop/buyout: the Trust's acceptance did not activate the buyout");
            // A landed cancel is a Cancelled event: the observer re-leases
            // synchronously (the old lease is terminal, the buyout unsettled) and has
            // told the pilot if it could not. A retrying one re-leases when it lands.
            let line = if cancel_own(ctx, id) {
                BUYOUT_FAILED.to_owned()
            } else {
                refund_pending
            };
            return Err(refuse_after_writes(ctx, line));
        }
    }
    // 6. The interim reply; the title passes at the next settlement.
    say(
        ctx,
        BUYOUT_FILED
            .replace("{name}", &e.name)
            .replace("{mark}", &e.mark)
            .replace("{buyout}", &buyout)
            .replace("{countersign}", COUNTERSIGN),
    );
    Ok(())
}

// ---- the observer's verdict -------------------------------------------------------

/// What [`settle_buyout`] does (lead ruling L7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The buyout settled: the enrolment graduates.
    Graduate,
    /// The buyout was refunded and the old lease is over: re-charter the pilot.
    Relet,
    /// Anything else: a later terminal event comes back.
    Wait,
}

/// **The decision table** (pure): `Settled` graduates; `Cancelled` or `Expired`
/// re-lets once the old lease is terminal; every other state (a live buyout, a
/// `Disputed` one awaiting `@contract/resolve`) waits.
#[must_use]
pub(crate) const fn buyout_verdict(buyout: ContractStateKind, lease_terminal: bool) -> Verdict {
    match buyout {
        ContractStateKind::Settled => Verdict::Graduate,
        ContractStateKind::Cancelled | ContractStateKind::Expired if lease_terminal => {
            Verdict::Relet
        }
        ContractStateKind::Cancelled
        | ContractStateKind::Expired
        | ContractStateKind::Draft
        | ContractStateKind::Offered
        | ContractStateKind::Active
        | ContractStateKind::ReadyToSettle
        | ContractStateKind::Disputed => Verdict::Wait,
    }
}

/// **A BuyoutPending enrolment's outcome** (doc 18 §4.1; cut IC13 item 5): the buyout
/// contract's state and the lease's, read from the economy (never the event alone).
/// Runs in the observer — the sweep's GOD context, a staff `@contract/cancel`, or the
/// buyout verb's own cancel.
pub(crate) fn settle_buyout(ctx: &mut Ctx<'_>, e: &Enrolment) {
    let Some(buyout) = e.buyout else {
        tracing::error!(
            pilot = e.pilot.0,
            "tacsim: a buyout-pending enrolment names no buyout contract"
        );
        return;
    };
    let states = ctx.w.module_state::<EconomyStore>().map(|economy| {
        let snapshot = economy.snapshot();
        (
            snapshot
                .contract(ContractId(buyout))
                .map(|c| c.state.kind()),
            snapshot
                .contract(ContractId(e.lease))
                .is_none_or(|c| c.state.is_terminal()),
        )
    });
    let Some((Some(kind), lease_terminal)) = states else {
        tracing::error!(
            contract = buyout,
            pilot = e.pilot.0,
            "tacsim: a buyout's contract does not read; the enrolment stays buyout-pending"
        );
        return;
    };
    match buyout_verdict(kind, lease_terminal) {
        Verdict::Graduate => graduate(ctx, e),
        // Doc 18 cut IC14 row 12: the observer runs the first re-charter; a failure
        // leaves the enrolment BuyoutPending and the recall arm retries.
        Verdict::Relet => {
            let _ = relet(ctx, e, 1, false);
        }
        Verdict::Wait => {}
    }
}

/// The buyout settled: the enrolment graduates and the pilot is told doc 18 §2 step
/// 19's line.
fn graduate(ctx: &mut Ctx<'_>, e: &Enrolment) {
    let pilot = PrincipalId::Object(e.pilot);
    if !titled_to(ctx.w, e.hull, &pilot) {
        tracing::error!(
            hull = e.hull.0,
            pilot = e.pilot.0,
            "tacsim: a buyout settled but the hull is not titled to the pilot; the enrolment \
             graduates all the same"
        );
    }
    let graduated = Enrolment {
        status: EnrolmentStatus::Graduated,
        ended_at: Some(ctx.w.now()),
        ended: Some(EndCause::BoughtOut),
        ..e.clone()
    };
    put_enrolment(ctx, &graduated);
    let line = GRADUATED
        .replace("{name}", &e.name)
        .replace("{buyout}", &money_minor(ctx, e.buyout_minor));
    crate::port_tick::notify(ctx, e.pilot, &line);
}

/// The re-charter's one acceptance, as the pilot: a plain save, set and restore of
/// `ctx.actor` (lead ruling L7 — not `Fire::as_principal`; the observer has no
/// `Fire`, and the accept is an ordinary normal-lane economy write).
fn accept_as_pilot(ctx: &mut Ctx<'_>, pilot: ObjId, lease: ContractId) -> bool {
    let saved = ctx.actor;
    ctx.actor = pilot;
    let accepted = matches!(
        mush_economy::accept_contract(ctx, lease),
        Ok(AcceptOutcome::Accepted { .. })
    );
    ctx.actor = saved;
    accepted
}

/// The buyout was refunded and the old lease is over: the Trust charters the same
/// hull to the pilot again on exactly the enrolment's terms (no readiness gate: it
/// restores, it does not enrol). `true` when the pilot is chartered again.
///
/// Doc 18 cut IC14 row 12 (lead ruling L11; proposed Q-18-71): `attempt` is this
/// re-charter's number (the observer's is 1, the recall arm's retries 2 and on). A
/// failure ends the half-made lease (unchanged) and, unless `give_up`, leaves the
/// enrolment **BuyoutPending** — its buyout refunded, its old lease terminal — for the
/// recall arm's next try; only the first failure tells the pilot ([`RELET_PENDING`]).
/// With `give_up` (the last attempt) a failure ends exactly as cut IC13 built it:
/// Revoked / `EndCause::LeaseEnded`, [`BUYOUT_UNLEASED`] to the pilot, and its own
/// staff page.
pub(crate) fn relet(ctx: &mut Ctx<'_>, e: &Enrolment, attempt: u8, give_up: bool) -> bool {
    let buyout = money_minor(ctx, e.buyout_minor);
    let resolved = programme_port(ctx.w)
        .and_then(|port| Some((port.trust()?, port.trust_scope()?, port.trust_account()?)));
    let mut offered: Option<(ContractId, PrincipalId)> = None;
    let relet =
        resolved
            .ok_or("the Trust does not resolve")
            .and_then(|(trust, scope, trust_account)| {
                let lease = mush_economy::offer_lease(
                    ctx,
                    LeaseSpec {
                        name: SmolStr::new(format!("First Captain charter, {}", e.mark)),
                        lessor: trust.clone(),
                        scope: Some(scope),
                        tenants: vec![PrincipalId::Object(e.pilot)],
                        premises: e.hull,
                        // Byte-for-byte `+coop/enrol`'s 10c (D-18-21; IC6 L3).
                        rent: ObligationTemplate {
                            debtor: PrincipalId::Object(e.pilot),
                            creditor: trust.clone(),
                            debit_account: AccountId(e.credit_account),
                            credit_account: trust_account,
                            amount: Amount::from_minor(1),
                            schedule: EconomicSchedule::new(
                                EconomicClock::Manual,
                                i64::MAX,
                                None,
                                1,
                            ),
                            grace: GracePolicy::default(),
                            notify_on_success: false,
                        },
                    },
                )
                .map_err(|_| "the lease was not offered")?;
                offered = Some((lease, trust));
                if !accept_as_pilot(ctx, e.pilot, lease) {
                    return Err("the pilot's acceptance did not land");
                }
                let active = ctx
                    .w
                    .module_state::<EconomyStore>()
                    .and_then(|store| store.snapshot().contract(lease).map(|c| c.state))
                    == Some(ContractState::Active);
                if active {
                    Ok(lease)
                } else {
                    Err("the lease did not come into force")
                }
            });
    match relet {
        Ok(lease) => {
            let renewed = Enrolment {
                status: EnrolmentStatus::Active,
                lease: lease.0,
                leased_at: ctx.w.now(),
                buyout: None,
                ..e.clone()
            };
            put_enrolment(ctx, &renewed);
            crate::port_tick::notify(
                ctx,
                e.pilot,
                &named(BUYOUT_REFUNDED, e).replace("{buyout}", &buyout),
            );
            true
        }
        Err(what) if !give_up => {
            if let Some((lease, trust)) = offered {
                let key = LeaseKey {
                    lessor: trust,
                    premises: e.hull,
                };
                if let Err(error) = mush_economy::end_lease(ctx, lease, &key) {
                    tracing::error!(lease = lease.0, %error, "tacsim: a half-made re-lease did not end");
                }
            }
            tracing::error!(
                what,
                attempt,
                pilot = e.pilot.0,
                hull = e.hull.0,
                "tacsim: a refunded buyout's re-lease failed; the enrolment stays buyout-pending \
                 and the recall arm retries under a backoff"
            );
            if attempt <= 1 {
                crate::port_tick::notify(
                    ctx,
                    e.pilot,
                    &named(RELET_PENDING, e).replace("{buyout}", &buyout),
                );
            }
            false
        }
        Err(what) => {
            if let Some((lease, trust)) = offered {
                let key = LeaseKey {
                    lessor: trust,
                    premises: e.hull,
                };
                if let Err(error) = mush_economy::end_lease(ctx, lease, &key) {
                    tracing::error!(lease = lease.0, %error, "tacsim: a half-made re-lease did not end");
                }
            }
            tracing::error!(
                what,
                pilot = e.pilot.0,
                hull = e.hull.0,
                buyout = e.buyout,
                attempts = attempt,
                "tacsim: a refunded buyout's re-charter failed {attempt} times; the enrolment is \
                 revoked and the hull returns through the recall arm once docked empty at Ceres; \
                 `@contract/show #<buyout>`"
            );
            let revoked = Enrolment {
                status: EnrolmentStatus::Revoked,
                ended_at: Some(ctx.w.now()),
                ended: Some(EndCause::LeaseEnded),
                ..e.clone()
            };
            put_enrolment(ctx, &revoked);
            crate::port_tick::notify(
                ctx,
                e.pilot,
                &named(BUYOUT_UNLEASED, e).replace("{buyout}", &buyout),
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Doc 18 cut IC13 item 4: the buyout is the market's asset-purchase shape.
    #[test]
    fn ic13_the_buyout_contract_is_the_market_asset_shape() {
        let trust = PrincipalId::Organization(mush_org::OrgId(3));
        let price = Amount::from_minor(9_000_000);
        let spec = buyout_spec(
            AccountId(11),
            &trust,
            AccountId(5),
            ObjId(500),
            ObjId(42),
            "FPC-D-0401",
            price,
            1_000,
        );
        assert_eq!(spec.name.as_str(), "First Captain buyout, FPC-D-0401");
        assert_eq!(spec.scope, None);
        assert_eq!(spec.parties, vec![trust.clone()]);
        assert_eq!(
            spec.considerations,
            vec![ConsiderationSpec {
                account: AccountId(11),
                amount: price
            }]
        );
        assert_eq!(spec.outcomes.len(), 1);
        let outcome = &spec.outcomes[0];
        assert_eq!(
            outcome.payouts,
            vec![PayoutShare {
                to: AccountId(5),
                share: Share::Exact(price)
            }]
        );
        assert_eq!(outcome.asset_consequences, vec![0]);
        assert_eq!(outcome.when, vec![0, 1]);
        assert_eq!(spec.remainder_to, AccountId(11));
        let window = EconomicDeadline {
            clock: EconomicClock::RealUtc,
            at: 1_000 + 3_600,
        };
        assert_eq!(
            spec.conditions,
            vec![
                TypedCondition::TitleHeld {
                    object: ObjId(500),
                    by: trust.clone()
                },
                TypedCondition::Before { deadline: window },
            ]
        );
        assert_eq!(
            spec.evidence,
            vec![EvidenceKind::TitleHeld, EvidenceKind::Before]
        );
        assert_eq!(spec.deadline, Some(window));
        assert_eq!(
            spec.deliverables,
            vec![Deliverable::TitleTransfer {
                object: ObjId(500),
                from: trust,
                to: PrincipalId::Object(ObjId(42)),
            }]
        );
        assert!(spec.deterministic);
        assert!(spec.adjudicator.is_none());
        assert_eq!(spec.on_expiry, Expiry::Refund);
        assert_eq!(spec.visibility, EconomicVisibility::Parties);
        assert_eq!(spec.roles.author, Some(PrincipalId::Object(ObjId(42))));
        assert_eq!(spec.roles.payer, Some(PrincipalId::Object(ObjId(42))));
    }

    /// Doc 18 cut IC13 item 5 (lead ruling L7): the observer's decision table.
    #[test]
    fn ic13_the_buyout_verdict_reads_the_economy() {
        use ContractStateKind as K;
        for lease_terminal in [false, true] {
            assert_eq!(
                buyout_verdict(K::Settled, lease_terminal),
                Verdict::Graduate
            );
            for live in [
                K::Draft,
                K::Offered,
                K::Active,
                K::ReadyToSettle,
                K::Disputed,
            ] {
                assert_eq!(
                    buyout_verdict(live, lease_terminal),
                    Verdict::Wait,
                    "{live:?}"
                );
            }
        }
        for refunded in [K::Cancelled, K::Expired] {
            assert_eq!(buyout_verdict(refunded, true), Verdict::Relet);
            assert_eq!(buyout_verdict(refunded, false), Verdict::Wait);
        }
    }

    /// Doc 18 cut IC14 row 12 (lead ruling L11): the re-charter backoff doubles from
    /// `RECALL_RECHECK_FIRES` (24 fires, 2 minutes at the 5 s fire) and is capped at an
    /// hour; six attempts in all, so the gaps after failures 1..=5 are 2, 4, 8, 16 and
    /// 32 minutes and the cap is reached only past them.
    #[test]
    fn ic14_the_relet_backoff_doubles_and_is_capped() {
        assert_eq!(MAX_RELET_ATTEMPTS, 6);
        assert_eq!(crate::port::recall::RECALL_RECHECK_FIRES, 24);
        let schedule: Vec<u64> = (1..=MAX_RELET_ATTEMPTS).map(relet_backoff).collect();
        assert_eq!(schedule, vec![24, 48, 96, 192, 384, 720]);
        assert_eq!(relet_backoff(0), 24, "an attempt below 1 reads as 1");
        assert_eq!(
            relet_backoff(40),
            MAX_RELET_BACKOFF_FIRES,
            "never past the cap"
        );
        assert_eq!(relet_backoff(u8::MAX), MAX_RELET_BACKOFF_FIRES);
    }
}
