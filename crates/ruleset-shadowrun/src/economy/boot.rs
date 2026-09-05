//! The E6a boot: the instrument, the faucets, the `sr.month` clock, and the two
//! reconciliation passes (08f §3.3, §3.5's closing paragraph, DL-07).
//!
//! It runs from `ShadowrunRuleset::on_world_loaded`, which is **last** in the module
//! list, so `mush-org`, `mush-faction`, `mush-economy` and `mush-party` are all
//! loaded before a single question is asked of them.
//!
//! Nothing here moves money. It registers `sr.nuyen`, opens the three system
//! accounts a faucet needs to own, resolves the declared `sr.month` cadence by name,
//! and then reconciles two projections against records that self-identify:
//!
//! - **Lifestyle obligations (DL-07).** A record self-identifies by
//!   `cause: Ruleset { "sr.lifestyle" }` plus a credit to `system:sr.lifestyle`'s
//!   account. `SrProfile.upkeep` is a *cache*: boot adopts a live record whose
//!   debtor has a profile, and cancels one no profile claims. With
//!   `lifestyle = false` boot cancels **nothing** (§9's partial-switch table:
//!   `@sr/migrate` ends them, never boot).
//! - **Board listings.** A listing is a projection of a contract. Boot mirrors the
//!   contract's live state onto the listing, and closes a listing whose contract is
//!   gone or terminal — the money was always the contract's, so nothing is lost.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Ctx, ObjId};
use mush_economy::{
    Account, AccountStatus, Amount, CadenceId, ContractState, CreditPolicy, EconomyStore,
    InstrumentDef, InstrumentDisplay, InstrumentId, InstrumentKind, InstrumentStatus,
    NegativePolicy, Obligation, ObligationCause, ObligationId, ObligationStatus, SymbolPosition,
    cadence_by_name,
};
use mush_org::{PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;

use crate::economy::config::{
    FAUCET_CHARGEN, FAUCET_MIGRATION, SINK_LIFESTYLE, SR_NUYEN_SCALE, SR_NUYEN_SYMBOL,
    SrEconomyConfig,
};
use crate::economy::record::{RunState, clip};
use crate::economy::store::{SrEconomyStore, persist_meta, put_profile, put_run};

/// The ruleset namespace stamped on every lifestyle obligation's cause (DL-07's
/// self-identification key, and the lock `cancel_obligation` checks).
pub const LIFESTYLE_NAMESPACE: &str = "sr.lifestyle";

/// The three system principals E6a names (§3.3).
#[must_use]
pub fn faucets() -> [&'static str; 3] {
    [FAUCET_CHARGEN, FAUCET_MIGRATION, SINK_LIFESTYLE]
}

/// A well-formed [`SystemPrincipalId`] for one of ours.
///
/// The three names are compile-time literals in `[a-z0-9_.-]`, so this cannot fail
/// for any caller in this crate; every caller treats `None` as "do nothing".
#[must_use]
pub fn principal(name: &str) -> Option<SystemPrincipalId> {
    SystemPrincipalId::new(name)
}

/// The whole boot, in order: instrument, accounts, clock, then the two
/// reconciliation passes.
pub fn run(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) {
    if !cfg.enabled {
        // The layer exists and does nothing (E5c deviation 1). The clock is still
        // resolved so `@sr/economy` can say what it *would* use.
        resolve_cadence(ctx, cfg);
        return;
    }
    if let Some(instrument) = ensure_instrument(ctx, cfg) {
        if let Some(store) = ctx.w.module_state_mut::<SrEconomyStore>() {
            store.instrument = Some(instrument.clone());
        }
        for name in faucets() {
            ensure_system_account(ctx, name, &instrument);
        }
    }
    resolve_cadence(ctx, cfg);
    reconcile_accounts(ctx);
    reconcile_lifestyle(ctx, cfg);
    reconcile_board(ctx);
    reconcile_parties(ctx);
    persist_meta(ctx);
}

/// Register `sr.nuyen` at scale 0 with its `\u{a5}` prefix (D-E6-1, E2-C16).
///
/// `register_instrument` is create-only since the look-back fold (E2-C18), so this
/// asks first and never re-registers: a live instrument's display, scale and policy
/// are exactly as they were persisted.
fn ensure_instrument(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) -> Option<InstrumentId> {
    let id = InstrumentId::new(&cfg.instrument)?;
    let known = ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|s| s.snapshot().instrument(&id).is_some());
    if known {
        return Some(id);
    }
    if let Err(error) = mush_economy::register_instrument(
        ctx,
        InstrumentDef {
            id: id.clone(),
            ruleset: SmolStr::new_static("shadowrun"),
            code: SmolStr::new(cfg.instrument.as_str()),
            kind: InstrumentKind::Currency,
            scale: SR_NUYEN_SCALE,
            transferable: true,
            // A faucet must be able to run negative — that is what a faucet *is* —
            // and `SystemOnly` keeps the privilege system-side, so a runner's wallet
            // still cannot.
            negative_policy: NegativePolicy::SystemOnly,
            status: InstrumentStatus::Active,
            display: Some(InstrumentDisplay {
                symbol: SmolStr::new_static(SR_NUYEN_SYMBOL),
                position: SymbolPosition::Prefix,
            }),
            revision: 1,
        },
    ) {
        tracing::warn!(%error, instrument = %cfg.instrument,
            "sr.economy: the instrument was already registered under this boot");
    }
    Some(id)
}

/// Open a system principal's `sr.nuyen` account if it has none.
///
/// The faucet **must own its debit account** for `create_system_obligation` to accept
/// it (lock 1, 08c §4.7 M1), and only an account opened here or by `@account/system`
/// satisfies that.
fn ensure_system_account(ctx: &mut Ctx<'_>, name: &str, instrument: &InstrumentId) {
    let Some(sys) = principal(name) else {
        return;
    };
    let owner = PrincipalId::System(sys);
    let known = ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
        s.snapshot()
            .accounts()
            .any(|a| a.owner == owner && a.instrument == *instrument)
    });
    if known {
        return;
    }
    let Some(id) = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .and_then(EconomyStore::alloc_account)
    else {
        tracing::error!(
            principal = name,
            "sr.economy: the account-id allocator is exhausted; no faucet account was opened"
        );
        return;
    };
    mush_economy::open_account(
        ctx,
        Account {
            id,
            owner,
            instrument: instrument.clone(),
            scope: None,
            name: SmolStr::new(name),
            status: AccountStatus::Active,
            posted_balance: Amount::ZERO,
            held_amount: Amount::ZERO,
            last_hold_seq: 0,
            // A faucet is a source: unbounded negative is what makes it one, and the
            // instrument's `SystemOnly` policy keeps that privilege system-side. The
            // lifestyle *sink* carries the same policy harmlessly — it only ever
            // receives.
            credit: CreditPolicy::unbounded(),
            last_journal_seq: 0,
            revision: 1,
            approval: None,
        },
    );
    tracing::info!(
        principal = name,
        account = id.0,
        "sr.economy: opened a system account"
    );
}

/// Find the **declared** `sr.month` cadence by name (SP1, D-LBK-7).
///
/// E6a does not create it: the host contributes the `[economy.cadences]` stanza when
/// `[rulesets].enabled` names `shadowrun`, the economy registers a `ClockProvider`
/// for it at composition, and the economy's own boot — which runs before this one —
/// creates or adopts the clock. `@sr/clock` is therefore a **show** verb.
fn resolve_cadence(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) {
    let found: Option<CadenceId> = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| cadence_by_name(s.snapshot(), &cfg.lifestyle_clock).map(|c| c.id));
    match found {
        Some(id) => {
            if let Some(store) = ctx.w.module_state_mut::<SrEconomyStore>() {
                store.cadence = Some(id);
            }
        }
        None if cfg.enabled => tracing::warn!(
            cadence = %cfg.lifestyle_clock,
            "sr.economy: the lifestyle cadence does not exist; declare it in \
             [economy.cadences] (the host contributes it when [rulesets].enabled names \
             shadowrun). No lifestyle obligation can be created until it does."
        ),
        None => {}
    }
}

/// Whether `o` is one of ours (DL-07's self-identification: the cause *and* the
/// creditor account, never the profile's pointer).
#[must_use]
pub fn is_lifestyle_record(o: &Obligation, sink: mush_economy::AccountId) -> bool {
    let ours = matches!(&o.cause, ObligationCause::Ruleset { namespace } if namespace.as_str() == LIFESTYLE_NAMESPACE);
    ours && o.credit_account == sink
}

/// Whether an obligation is still live enough to be somebody's upkeep.
#[must_use]
pub fn is_live(o: &Obligation) -> bool {
    !matches!(
        o.status,
        ObligationStatus::Cancelled | ObligationStatus::Satisfied
    )
}

/// The `system:sr.lifestyle` account, when boot has opened it.
#[must_use]
pub fn sink_account(ctx: &Ctx<'_>, instrument: &InstrumentId) -> Option<mush_economy::AccountId> {
    system_account(ctx, SINK_LIFESTYLE, instrument)
}

/// The account owned by system principal `name` in `instrument`, if any.
#[must_use]
pub fn system_account(
    ctx: &Ctx<'_>,
    name: &str,
    instrument: &InstrumentId,
) -> Option<mush_economy::AccountId> {
    let owner = PrincipalId::System(principal(name)?);
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .accounts()
        .find(|a| a.owner == owner && a.instrument == *instrument)
        .map(|a| a.id)
}

/// `who`'s **live** personal account in `instrument`, if any.
///
/// Two rules, both from the E6a verify fold:
///
/// - **`Active` only** (R4). Matching on owner + instrument + scope alone meant a
///   *closed* account kept being picked — by migration, by the crew payout table, by
///   lifestyle — even after staff had opened a live one beside it, so closing an
///   account was a one-way exit from the layer with no way back.
/// - **Indexed** (D13). `accounts_of` is the owner index, so `crew_accounts` asking
///   this once per roster member is eight index lookups rather than eight full walks
///   of the account registry.
#[must_use]
pub fn personal_account(
    ctx: &Ctx<'_>,
    who: ObjId,
    instrument: &InstrumentId,
) -> Option<mush_economy::AccountId> {
    let owner = PrincipalId::Object(who);
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(&owner)
        .find(|a| {
            a.instrument == *instrument && a.scope.is_none() && a.status == AccountStatus::Active
        })
        .map(|a| a.id)
}

/// Open `who`'s personal `sr.nuyen` account, or return the one they have.
pub fn open_personal_account(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    instrument: &InstrumentId,
) -> Option<mush_economy::AccountId> {
    if let Some(id) = personal_account(ctx, who, instrument) {
        return Some(id);
    }
    let id = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .and_then(EconomyStore::alloc_account)?;
    let name = ctx.w.get(who).map_or_else(
        || format!("#{}", who.0),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    );
    mush_economy::open_account(
        ctx,
        Account {
            id,
            owner: PrincipalId::Object(who),
            instrument: instrument.clone(),
            scope: None,
            name: SmolStr::new(clip(&name, 32)),
            status: AccountStatus::Active,
            posted_balance: Amount::ZERO,
            held_amount: Amount::ZERO,
            last_hold_seq: 0,
            credit: CreditPolicy::forbid(),
            last_journal_seq: 0,
            revision: 1,
            approval: None,
        },
    );
    Some(id)
}

/// **R4**: a profile pointing at an account that is no longer `Active` is re-pointed
/// at the character's live one, or at nothing.
///
/// A closed account used to be a one-way exit: `SrProfile.account` kept naming it, and
/// every read that trusted the cache — `+nuyen`, `+lifestyle`, `crew_accounts`,
/// `resolve_account` — kept using it, so staff opening a replacement changed nothing.
/// The account is the economy's record and the profile's field is a *cache*, exactly
/// as `upkeep` is (DL-07), so boot re-derives it from what is live.
fn reconcile_accounts(ctx: &mut Ctx<'_>) {
    let Some(instrument) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())
    else {
        return;
    };
    let named: Vec<(ObjId, mush_economy::AccountId)> = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(|s| {
            s.profiles()
                .filter_map(|p| p.account.map(|a| (p.obj, a)))
                .collect()
        })
        .unwrap_or_default();
    let stale: Vec<ObjId> = ctx
        .w
        .module_state::<EconomyStore>()
        .map(|s| {
            let snap = s.snapshot();
            named
                .iter()
                .filter(|(_, a)| {
                    snap.account(*a)
                        .is_none_or(|a| a.status != AccountStatus::Active)
                })
                .map(|(who, _)| *who)
                .collect()
        })
        .unwrap_or_default();
    for who in stale {
        let Some(mut p) = ctx
            .w
            .module_state::<SrEconomyStore>()
            .and_then(|s| s.profile(who).cloned())
        else {
            continue;
        };
        let next = personal_account(ctx, who, &instrument);
        tracing::warn!(
            who = who.0,
            was = p.account.map_or(0, |a| a.0),
            now = next.map_or(0, |a| a.0),
            "sr.economy: a profile named an account that is no longer active; the cache was \
             re-derived from the live accounts"
        );
        p.account = next;
        p.revision = p.revision.saturating_add(1);
        put_profile(ctx, p);
    }
}

/// DL-07: adopt a self-identifying lifestyle record, cancel an orphan.
fn reconcile_lifestyle(ctx: &mut Ctx<'_>, cfg: &SrEconomyConfig) {
    let Some(instrument) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())
    else {
        return;
    };
    let Some(sink) = sink_account(ctx, &instrument) else {
        return;
    };
    // account → owner, so a record's debtor can be matched to a profile.
    let owners: BTreeMap<mush_economy::AccountId, ObjId> = ctx
        .w
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .accounts()
                .filter_map(|a| match a.owner {
                    PrincipalId::Object(o) => Some((a.id, o)),
                    PrincipalId::Organization(_) | PrincipalId::System(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let profiled: BTreeSet<ObjId> = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(|s| s.profiles().map(|p| p.obj).collect())
        .unwrap_or_default();
    let live: Vec<(ObligationId, mush_economy::AccountId)> = ctx
        .w
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .obligations()
                .filter(|o| is_lifestyle_record(o, sink) && is_live(o))
                .map(|o| (o.id, o.debit_account))
                .collect()
        })
        .unwrap_or_default();
    // M6's boot half: a profile that already points at a *different* live record has
    // been through a replacement whose cancel was refused, and the record it does not
    // point to is the stale one. Adopting it would move the pointer back to the tier
    // the character just left, so it is cancelled instead — the same treatment an
    // orphan gets, for the same reason (nothing claims it).
    let claimed: BTreeMap<ObjId, ObligationId> = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(|s| {
            s.profiles()
                .filter_map(|p| p.upkeep.map(|o| (p.obj, o)))
                .collect()
        })
        .unwrap_or_default();
    let mut adopted: BTreeSet<ObligationId> = BTreeSet::new();
    for (id, debit) in live {
        let who = owners
            .get(&debit)
            .copied()
            .filter(|o| profiled.contains(o))
            .filter(|o| claimed.get(o).is_none_or(|claim| *claim == id));
        match who {
            Some(who) => {
                adopted.insert(id);
                let stale = ctx
                    .w
                    .module_state::<SrEconomyStore>()
                    .and_then(|s| s.profile(who).cloned())
                    .filter(|p| p.upkeep != Some(id));
                if let Some(mut p) = stale {
                    tracing::warn!(
                        obligation = id.0,
                        who = who.0,
                        "sr.economy: adopted a lifestyle record the profile had lost the pointer \
                         to (the record named itself — DL-07)"
                    );
                    p.upkeep = Some(id);
                    p.revision = p.revision.saturating_add(1);
                    put_profile(ctx, p);
                }
            }
            None if cfg.lifestyle => {
                // `lifestyle = false` cancels nothing at boot (§9's table); only the
                // on-switch world ends a record nobody claims.
                match mush_economy::cancel_obligation(ctx, id, LIFESTYLE_NAMESPACE) {
                    Ok(()) => tracing::warn!(
                        obligation = id.0,
                        debit = debit.0,
                        "sr.economy: cancelled a lifestyle record no profile points at (an orphan, \
                         or a replacement whose cancel was refused — M6)"
                    ),
                    Err(error) => tracing::error!(
                        obligation = id.0, debit = debit.0, %error,
                        "sr.economy: could not cancel an unclaimed lifestyle record; it is still \
                         collecting (staff: @obligation/cancel)"
                    ),
                }
            }
            None => {}
        }
    }
    // A profile pointing at a record that is gone, terminal, or somebody else's.
    let stale: Vec<ObjId> = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(|s| {
            s.profiles()
                .filter(|p| p.upkeep.is_some_and(|id| !adopted.contains(&id)))
                .map(|p| p.obj)
                .collect()
        })
        .unwrap_or_default();
    for who in stale {
        let Some(mut p) = ctx
            .w
            .module_state::<SrEconomyStore>()
            .and_then(|s| s.profile(who).cloned())
        else {
            continue;
        };
        tracing::warn!(
            who = who.0,
            "sr.economy: a profile named an upkeep record that is no longer live; the cache was \
             cleared (+lifestyle/set re-creates it)"
        );
        p.upkeep = None;
        p.revision = p.revision.saturating_add(1);
        put_profile(ctx, p);
    }
}

/// Mirror every listing onto its contract's live state; close the ones whose
/// contract is gone or terminal, **and release the party of every one that is
/// closed** (E6a verify fold, R1).
///
/// Only the observer and `+run/withdraw` used to release a party, and both are live
/// paths. A terminal post whose board and party writes did not land, or a contract
/// that reached a terminal state while `[shadowrun.economy]` was off, therefore left
/// its crew `Locked` — refusing every membership verb, immune to idle expiry (a locked
/// party's clock is its contract's), and unreachable by staff, because nothing but the
/// ruleset can call `release`. Forever. The release runs whenever the *mirrored* state
/// is `Closed`, including the `now == was` case, because "the listing already said
/// closed" is precisely the shape a dropped write leaves behind. `release` is
/// idempotent in effect — a party that is not `Locked` answers `State` and is logged
/// at info.
fn reconcile_board(ctx: &mut Ctx<'_>) {
    let listings: Vec<(
        crate::economy::record::RunId,
        mush_economy::ContractId,
        RunState,
        Option<u64>,
    )> = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(|s| {
            s.runs()
                .map(|r| (r.id, r.contract, r.state, r.party))
                .collect()
        })
        .unwrap_or_default();
    for (id, contract, was, party) in listings {
        let state = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(contract).map(|c| c.state));
        let settled = matches!(state, Some(ContractState::Settled { .. }));
        let now = match &state {
            None => RunState::Closed,
            Some(state) => mirror(state, was, party.is_some()),
        };
        if now.is_closed() {
            // The party first: `put_run` on a closed listing deletes the record, so
            // the back-reference is read before it goes.
            crate::economy::board::release_party(
                ctx,
                party,
                if settled {
                    mush_party::DisbandCause::Settled
                } else {
                    mush_party::DisbandCause::Released
                },
            );
        }
        if now == was {
            continue;
        }
        let Some(mut r) = ctx
            .w
            .module_state::<SrEconomyStore>()
            .and_then(|s| s.run(id).cloned())
        else {
            continue;
        };
        tracing::info!(
            run = id.0,
            contract = contract.0,
            from = was.word(),
            to = now.word(),
            "sr.economy: a listing was re-mirrored onto its contract at boot"
        );
        r.state = now;
        r.revision = r.revision.saturating_add(1);
        put_run(ctx, r);
    }
}

/// **R2**: release every live party locked against a contract that is gone or
/// terminal.
///
/// [`reconcile_board`] can only reach a party a *listing* still names, and a listing
/// is exactly the thing this ruleset drops when it will not load: a skipped
/// `sr:run:{id}` document leaves `run_of_contract` empty, so nothing at all knows the
/// crew is frozen against contract N. The party stays `Locked` permanently — the one
/// unrecoverable state in the cut.
///
/// So the ruleset asks the question from the *party* side instead. `mush-party` stays
/// money-blind: it hands back the opaque [`mush_party::ContractRef`] it was given and
/// the ruleset — which owns both ends (D-E6-5) — is the one that resolves it. A ref
/// whose contract does not exist, or has reached a terminal state, releases; anything
/// else is left exactly as it is. One page per orphan, capped, because a crew released
/// out from under a job is a thing a person should hear about.
fn reconcile_parties(ctx: &mut Ctx<'_>) {
    /// The most orphaned parties one boot pages about. The releases are not capped —
    /// every one of them happens — only the pages, so a mass loss is one readable
    /// alert list rather than four thousand.
    const MAX_ORPHAN_PAGES: usize = 8;

    let locked: Vec<(mush_party::PartyId, mush_party::ContractRef)> = ctx
        .w
        .module_state::<mush_party::PartyStore>()
        .map(|s| {
            s.live_parties()
                .filter_map(|p| p.locked_by().map(|by| (p.id, by)))
                .collect()
        })
        .unwrap_or_default();
    let mut paged = 0usize;
    for (party, by) in locked {
        let contract = mush_economy::ContractId(by.0);
        let live = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(contract).map(|c| c.state));
        let (over, settled) = match &live {
            None => (true, false),
            Some(state) => (
                state.is_terminal(),
                matches!(state, ContractState::Settled { .. }),
            ),
        };
        if !over {
            continue;
        }
        let how = if settled {
            mush_party::DisbandCause::Settled
        } else {
            mush_party::DisbandCause::Released
        };
        tracing::warn!(
            party = %party,
            contract = contract.0,
            "sr.economy: a live party was locked against a contract that is gone or terminal; \
             releasing it (the listing that named it is not there — its document was skipped, or \
             the layer was off when the contract ended)"
        );
        if let Err(error) = mush_party::release(ctx, party, how) {
            tracing::error!(party = %party, %error, "sr.economy: an orphaned party would not release");
            continue;
        }
        if paged < MAX_ORPHAN_PAGES {
            paged = paged.saturating_add(1);
            mush_economy::open_escalation(
                ctx,
                mush_economy::EscalationKind::BootReconciliation,
                mush_economy::EscalationSubject::Contract(contract),
                format!(
                    "sr.economy: party {party} was frozen against contract #{} — which is gone or \
                     terminal — with no board listing naming it. The party has been released.",
                    contract.0
                ),
                vec![format!("@contract/show {}", contract.0)],
            );
        }
    }
}

/// The board's mirror of one contract state (§3.2 "Board").
///
/// `Draft` is the window between an A′ amendment and its re-offer, and `Offered`
/// after a crew was added is `Crewing` — so the mirror keeps whichever of
/// `Listed`/`Crewing` the board already had rather than walking backwards. `Active`
/// is `Crewed` only when a crew is on the listing: a one-party contract activates at
/// its author's own accept (S1 deviation 2), which is exactly what `Listed` is.
#[must_use]
pub fn mirror(state: &ContractState, was: RunState, crewed: bool) -> RunState {
    match state {
        ContractState::Draft | ContractState::Offered => match was {
            RunState::Listed => RunState::Listed,
            RunState::Crewing | RunState::Crewed => RunState::Crewing,
            RunState::Closed => RunState::Closed,
        },
        ContractState::Active | ContractState::ReadyToSettle if crewed => RunState::Crewed,
        ContractState::Active | ContractState::ReadyToSettle => was,
        ContractState::Settled { .. }
        | ContractState::Cancelled
        | ContractState::Expired
        | ContractState::Disputed => RunState::Closed,
    }
}
