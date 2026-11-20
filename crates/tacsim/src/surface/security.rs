//! **Port security** (doc 41 cut RS.7; §4.6, Q-41-24, Q-41-25, Q-38-20, doc 27 Q-S4), on the
//! record of proven offenses (doc 50 unit LW.2; Q-50-37, Q-50-38, Q-50-68).
//!
//! A site's evidence log (`SiteRecord.unlawful`, Q-36-30) is evidence, never an enforcement. A
//! row becomes the Registry's business only when it is **proven**: a `Recovery` row whose claim a
//! `recovery` contest vacated (the registry adjudicated it), or any row a witness reported to the
//! registry with evidence of their own (`+salvage/report <site>[=<who>|#<hull>] registry`). At the
//! moment of proof the row is copied into the actor's record of proven offenses under the polity
//! that proved it ([`crate::offense`]), since the site's ring keeps only its last 64 rows
//! (round-2 item 41-6). The Freeport Registry keeps the lane's claims, so it is the Freeport that
//! proves a salvage offense ([`proving_polity`]).
//!
//! - **Jurisdiction** (Q-50-38): an offense binds only at the ports its polity holds de facto
//!   ([`crate::offense::binds_at`]). Other ports ignore it: wanted at Ceres is clean at Stickney.
//! - **Who answers** (Q-50-68, [`crate::offense::answerer_for_hull`]): the docking hull's pilot,
//!   or, for an organization's hull, the organization. Never a passenger or a crew member.
//! - **The fine** ([`on_docked`], the docking hook): at a port, every proven unfined offense
//!   against the hull's answerer that the port's holder proved is fined by kind ([`fine_major`])
//!   to the port's governing authority. The fine is a one-term debt (RS.5's `create_debt`, under
//!   [`DEBT_NAMESPACE`] so `+debts` lists it), paid in the same step when the account covers it;
//!   the row carries the debt's id, so it is never charged twice.
//! - **The quarantine** (`quarantine_line`, read by `prepare_departure`): while a fine of the
//!   port's holder stands past its grace against the hull's answerer, the hull that docked does
//!   not lift **from that polity's ports**. Elsewhere, a site's pad included, she lifts. It lifts
//!   when the debt is paid: `+debts/pay`, or the docking check reinstating it at every later
//!   docking there so the sweep tries again.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CmdError, Ctx, GOD, ObjId, World};
use mush_economy::{AccountId, Amount, ObligationId, ObligationStatus};
use mush_faction::FactionId;
use mush_org::{OrgId, PrincipalId};

use crate::offense::{
    Evidence, Offender, OffenseKey, OffenseKind, Proof, ProvenOffense, record_offense,
};
use crate::store::TacsimStore;
use crate::surface::claim::UnlawfulAct;
use crate::surface::retrieval::DEBT_NAMESPACE;

/// **The fine for an unlawful pickup**, major credits (Q-41-25, provisional).
pub const FINE_PICKUP_MAJOR: u64 = 500;
/// **The fine for unlawful scrapping**, major credits (Q-41-25, provisional).
pub const FINE_SCRAP_MAJOR: u64 = 2_000;
/// **The fine for an unlawful recovery**, major credits (Q-41-25, provisional).
pub const FINE_RECOVERY_MAJOR: u64 = 5_000;
/// How long after an act a witness's own record of the site still evidences it, IC seconds.
pub const REPORT_SIGHT_IC_S: i64 = 86_400;

/// `+salvage/report … registry`: the rows taken: `{n}`, `{site}`, `{actor}`.
pub const REPORTED: &str =
    "The Freeport Registry takes your report of {n} at {site} against {actor} on your evidence.";
/// Nothing on the site's log matches.
pub const REPORT_NOTHING: &str = "The registry's log for {site} shows nothing of that kind.";
/// Rows match but the reporter holds no evidence of them.
pub const REPORT_NO_EVIDENCE: &str =
    "You hold no record of your own of what happened at {site}; the registry acts on evidence.";
/// Every matching row is before the registry already.
pub const REPORT_ALREADY: &str = "The registry already has that before it.";
/// One's own act.
pub const REPORT_OWN: &str = "You cannot report yourself.";
/// No such site: `{site}`.
pub const REPORT_NO_SITE: &str = "There is no site called {site}.";
/// The registry's books are unavailable.
pub const REPORT_DARK: &str = "The registry cannot take a report now; try again shortly.";
/// The fine paid at docking: `{authority}`, `{name}`, `{port}`, `{amount}`, `{what}`.
pub const FINED_PAID: &str =
    "The {authority} fines you {amount} at {port} for {what}; it is paid from your account.";
/// The fine owed: `{authority}`, `{name}`, `{port}`, `{amount}`, `{what}`.
pub const FINED_OWED: &str = "The {authority} fines you {amount} at {port} for {what}; your \
     account cannot cover it, so it stands as a debt (+debts); unpaid when its 28 IC days of grace \
     run out, it holds the {name} at this polity's ports until it is paid.";
/// **The quarantine** (`DepartRefusalKind::Quarantined`): `{name}`, `{port}`, `{amount}`.
pub const QUARANTINED: &str = "The {name} is held at {port}: a fine of {amount} stands against \
     her master; it lifts when it is paid.";

/// **The fine for `kind`**, major credits (Q-41-25).
#[must_use]
pub const fn fine_major(kind: OffenseKind) -> u64 {
    match kind {
        OffenseKind::SalvagePickup => FINE_PICKUP_MAJOR,
        OffenseKind::SalvageScrap => FINE_SCRAP_MAJOR,
        OffenseKind::SalvageRecovery => FINE_RECOVERY_MAJOR,
    }
}

/// **The polity that proves a salvage offense** (Q-50-38): the Freeport Registry keeps the lane's
/// claims and adjudicates their contests, so the polity it answers to proves them. `None` before
/// the Registry or its polity is made.
#[must_use]
pub fn proving_polity(w: &World) -> Option<FactionId> {
    let (org, _, _) = crate::space::picket::resolve_registry_principal_and_account(w)?;
    crate::polity::polity_of_org(w, org)
}

/// **Prove `act` on `site`** (Q-41-24): written into its actor's record under the proving polity
/// ([`record_offense`]) unless it is there already. Answers whether it was added. A row that
/// proves no offense of its own (a `Vacated` row, a capture), or a world with no proving polity,
/// proves nothing.
pub(crate) fn prove(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    act: &UnlawfulAct,
    proof: Proof,
) -> bool {
    let Some(kind) = OffenseKind::of_unlawful(act.kind) else {
        return false;
    };
    let Some(polity) = proving_polity(ctx.w) else {
        tracing::error!(
            actor = act.actor.0,
            "tacsim: a proven salvage act has no proving polity (the Registry or its polity is \
             not made); it is not kept"
        );
        return false;
    };
    let key = OffenseKey {
        offender: Offender::Character(act.actor),
        polity,
    };
    let offense = ProvenOffense {
        kind,
        evidence: Evidence::Site {
            site: site.clone(),
            act: act.clone(),
        },
        proof,
        proven_ic: ctx.w.world_ic_now().at,
        debt: None,
    };
    record_offense(ctx, key, offense, settled)
}

/// The status of the obligation `id`, when it reads.
fn debt_status(w: &World, id: u64) -> Option<ObligationStatus> {
    w.module_state::<mush_economy::EconomyStore>()?
        .snapshot()
        .obligation(ObligationId(id))
        .map(|o| o.status.clone())
}

/// Whether a fined row's debt is still owed.
fn owed(w: &World, row: &ProvenOffense) -> bool {
    row.debt.is_some_and(|id| {
        matches!(
            debt_status(w, id),
            Some(
                ObligationStatus::Active
                    | ObligationStatus::Grace { .. }
                    | ObligationStatus::Delinquent
            )
        )
    })
}

/// Whether a row is settled: fined, and its debt no longer owed.
fn settled(w: &World, row: &ProvenOffense) -> bool {
    row.debt.is_some() && !owed(w, row)
}

/// **The lane port at `entry`**: its definition and map.
fn port_at<'w>(
    w: &'w World,
    entry: &CanonicalId,
) -> Option<(&'w crate::port::PortDef, &'w crate::port::PortMap)> {
    crate::port::market::ports(w)
        .into_iter()
        .find(|(id, _, _)| id == entry)
        .and_then(|(_, def, map)| Some((def, map?)))
}

/// **The port's governing authority** (Q-50-38: a fine is paid to the enforcing authority): its
/// name and its account, when the port authors one.
fn authority_of(w: &World, entry: &CanonicalId) -> Option<(String, AccountId)> {
    let (def, map) = port_at(w, entry)?;
    let row = map.orgs.get(def.authority.as_ref()?)?;
    let account = AccountId(row.account?);
    let name =
        crate::surface::retrieval::principal_name(w, &PrincipalId::Organization(OrgId(row.org)));
    Some((name, account))
}

/// **The credit account a fine is drawn on**: a character's own, or an organization's
/// (`port::berth`'s, which upkeep and berthing already charge).
fn account_of(w: &World, offender: Offender) -> Option<AccountId> {
    match offender {
        Offender::Character(who) => crate::port::credit::own_account(w, who).map(|a| a.id),
        Offender::Organization(org) => crate::port::berth::org_account(w, org),
        Offender::Account(_) => None,
    }
}

/// The character told of a fine: the character offender, or an organization's hull's pilot.
fn told(w: &World, offender: Offender, hull: ObjId) -> Option<ObjId> {
    match offender {
        Offender::Character(who) => Some(who),
        Offender::Organization(_) => crate::port::coop::helm_holder(w, hull)
            .or_else(|| crate::port::owned::org_flyer(w, hull)),
        Offender::Account(_) => None,
    }
}

/// **The minor units of the fines on `key` that hold a hull**: those past their 28-IC-day grace
/// unpaid (`retrieval::past_due`: `Delinquent`, or reinstated and not yet collected; the owner's
/// 2026-11-04 rule: a fine still within its grace is owed and listed on `+debts`, but the hull
/// may undock).
fn owed_minor(w: &World, key: &OffenseKey) -> i64 {
    let Some(record) = crate::offense::record_of(w, key) else {
        return 0;
    };
    let Some(debtor) = key.offender.principal() else {
        return 0;
    };
    let ids: Vec<u64> = record
        .rows
        .iter()
        .filter(|row| {
            row.debt.is_some_and(|id| {
                w.module_state::<mush_economy::EconomyStore>()
                    .and_then(|e| e.snapshot().obligation(ObligationId(id)).cloned())
                    .is_some_and(|o| crate::surface::retrieval::past_due(&o))
            })
        })
        .filter_map(|row| row.debt)
        .collect();
    if ids.is_empty() {
        return 0;
    }
    mush_economy::debts_of(w, &debtor, &[DEBT_NAMESPACE])
        .iter()
        .filter(|o| ids.contains(&o.id.0))
        .fold(0_i64, |sum, o| sum.saturating_add(o.amount.minor()))
}

/// **The record that binds `carrier` at `entry`**: her answerer's, under the port's de facto
/// holder. `None` at a place no polity holds, or for a hull nobody answers for.
fn binding_key(w: &World, carrier: ObjId, entry: &CanonicalId) -> Option<OffenseKey> {
    Some(OffenseKey {
        offender: crate::offense::answerer_for_hull(w, carrier)?,
        polity: crate::offense::jurisdiction_at(w, entry)?,
    })
}

/// **Whether port security holds `carrier` at `entry`** (`DepartRefusalKind::Quarantined`): a
/// fine of the port's holder against her answerer, past its 28-IC-day grace unpaid.
#[must_use]
pub fn holds_at(w: &World, carrier: ObjId, entry: &CanonicalId) -> bool {
    binding_key(w, carrier, entry).is_some_and(|key| owed_minor(w, &key) > 0)
}

/// **The quarantine's line for `carrier` docked at `entry`** (`DepartRefusalKind::Quarantined`):
/// `Some` while a fine of the port's holder stands past its grace against her answerer.
#[must_use]
pub(crate) fn quarantine_line(
    ctx: &Ctx<'_>,
    carrier: ObjId,
    entry: &CanonicalId,
    (name, port): (&str, &str),
) -> Option<String> {
    let total = binding_key(ctx.w, carrier, entry).map_or(0, |key| owed_minor(ctx.w, &key));
    (total > 0).then(|| {
        QUARANTINED
            .replace("{name}", name)
            .replace("{port}", port)
            .replace("{amount}", &crate::port::coop::money_minor(ctx, total))
    })
}

/// **A carrier docked at the port `entry`** (Q-41-25, Q-50-38; the docking hook): every offense
/// the port's holder proved against her answerer and has not fined is fined, and every such fine
/// still delinquent is reinstated so the sweep tries it again while the hold stands.
pub fn on_docked(ctx: &mut Ctx<'_>, carrier: ObjId, entry: &CanonicalId, port: &str) {
    let Some(key) = binding_key(ctx.w, carrier, entry) else {
        return;
    };
    let name = crate::port::coop::name_of(ctx.w, carrier);
    fine_offender(ctx, &key, carrier, entry, (&name, port));
}

/// The fines and the reinstatements on one record (`on_docked`).
#[allow(clippy::too_many_lines)] // the reinstatements, then each row's fine, in order
fn fine_offender(
    ctx: &mut Ctx<'_>,
    key: &OffenseKey,
    carrier: ObjId,
    entry: &CanonicalId,
    (name, port): (&str, &str),
) {
    let Some(mut record) = crate::offense::record_of(ctx.w, key) else {
        return;
    };
    // A character acts on their own account; an organization's fine is levied by the port's
    // authority with staff authority, as no character acts for it at a docking.
    let acting = match key.offender {
        Offender::Character(who) => who,
        Offender::Organization(_) | Offender::Account(_) => GOD,
    };
    let saved = ctx.actor;
    ctx.actor = acting;
    // The fines owed already: a delinquent one goes back to the due pool (a one-shot never
    // retries unprompted).
    for row in &record.rows {
        if let Some(id) = row.debt
            && matches!(debt_status(ctx.w, id), Some(ObligationStatus::Delinquent))
            && let Err(error) = mush_economy::reinstate_debt(ctx, ObligationId(id), DEBT_NAMESPACE)
        {
            tracing::warn!(key = %key.doc_key(), debt = id, %error, "tacsim: a delinquent fine could not be reinstated at docking");
        }
    }
    let Some((authority, payee)) = authority_of(ctx.w, entry) else {
        tracing::warn!(%entry, "tacsim: a port with no governing authority fines nothing");
        ctx.actor = saved;
        return;
    };
    let account = account_of(ctx.w, key.offender);
    let to = told(ctx.w, key.offender, carrier);
    let mut changed = false;
    for i in 0..record.rows.len() {
        if record.rows[i].debt.is_some() {
            continue;
        }
        let kind = record.rows[i].kind;
        let Some(minor) =
            crate::port::apply::minor_of(fine_major(kind), crate::port::apply::credit_scale(ctx.w))
        else {
            continue;
        };
        let Some(account) = account else {
            tracing::warn!(
                key = %key.doc_key(),
                "tacsim: a fine meets an offender with no account; it waits"
            );
            continue;
        };
        let due = crate::surface::retrieval::debt_due(ctx.w);
        let id = match mush_economy::create_debt_due(
            ctx,
            (account, payee),
            Amount::from_minor(minor),
            DEBT_NAMESPACE,
            due,
        ) {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(key = %key.doc_key(), %error, "tacsim: a port-security fine could not be opened; it waits");
                continue;
            }
        };
        record.rows[i].debt = Some(id.0);
        changed = true;
        let covers = ctx
            .w
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|e| e.snapshot().account(account).and_then(|a| a.available()))
            .is_some_and(|a| a.minor() >= minor);
        let paid = covers
            && matches!(
                mush_economy::pay_debt(ctx, id, DEBT_NAMESPACE),
                Ok(mush_economy::PostOutcome::Posted(_))
            );
        if let Some(to) = to {
            let line = if paid { FINED_PAID } else { FINED_OWED }
                .replace("{authority}", &authority)
                .replace("{name}", name)
                .replace("{port}", port)
                .replace("{amount}", &crate::port::coop::money_minor(ctx, minor))
                .replace("{what}", kind.what());
            ctx.out.sys_to_actor(to, "tacsim.security", line);
        }
    }
    if changed {
        crate::offense::put_receipts(ctx, &record);
    }
    ctx.actor = saved;
    ctx.out.pin_deferred();
}

/// **`+salvage/report <site>[=<who>|#<hull>] registry`** (Q-41-24): a witness reports the
/// site's logged acts against `who` (or on `#<hull>`, or every act there) to the registry; the
/// rows the reporter holds evidence of — a sensor or sight record pinned on the hull's custody,
/// or their own record of the site from within a day after the act — are proven. Committed on
/// the one command, as a report to a clerk is.
///
/// # Errors
///
/// The refusal.
pub(crate) fn report_cmd(ctx: &mut Ctx<'_>, head: &str) -> Result<(), CmdError> {
    let (word, target) = match head.split_once('=') {
        Some((word, target)) => (word.trim(), Some(target.trim())),
        None => (head.trim(), None),
    };
    let site = site_named(ctx.w, word)
        .ok_or_else(|| CmdError::msg(REPORT_NO_SITE.replace("{site}", word)))?;
    let me = ctx.actor;
    let (actor, subject) = match target {
        None => (None, None),
        Some(t) if t.starts_with('#') => (
            None,
            Some(
                t.trim_start_matches('#')
                    .parse::<u32>()
                    .map_err(|_| CmdError::msg(crate::surface::claim::SALVAGE_USAGE))?,
            ),
        ),
        Some(t) => {
            let who = ctx
                .w
                .iter()
                .find(|o| o.kind == mush_core::ObjKind::Player && o.name.eq_ignore_ascii_case(t))
                .map(|o| o.id)
                .ok_or_else(|| {
                    CmdError::msg(crate::surface::claim::NO_SUCH_WHO.replace("{who}", t))
                })?;
            (Some(who), None)
        }
    };
    if actor == Some(me) {
        return Err(CmdError::msg(REPORT_OWN));
    }
    let record = crate::surface::tow::record_or_new(ctx, &site.entry);
    let rows: Vec<UnlawfulAct> = record
        .unlawful
        .iter()
        .filter(|act| OffenseKind::of_unlawful(act.kind).is_some() && act.actor != me)
        .filter(|act| actor.is_none_or(|who| act.actor == who))
        .filter(|act| subject.is_none_or(|s| act.subject == s))
        .cloned()
        .collect();
    let fill = |line: &str| line.replace("{site}", &site.name);
    if rows.is_empty() {
        return Err(CmdError::msg(fill(REPORT_NOTHING)));
    }
    let seen: Vec<UnlawfulAct> = rows
        .into_iter()
        .filter(|act| witnessed(&record, me, act))
        .collect();
    if seen.is_empty() {
        return Err(CmdError::msg(fill(REPORT_NO_EVIDENCE)));
    }
    if ctx.w.module_state::<TacsimStore>().is_none() {
        return Err(CmdError::msg(REPORT_DARK));
    }
    let mut taken: BTreeMap<ObjId, usize> = BTreeMap::new();
    for act in &seen {
        if prove(ctx, &site.entry, act, Proof::Reported { by: me }) {
            *taken.entry(act.actor).or_default() += 1;
        }
    }
    if taken.is_empty() {
        return Err(CmdError::msg(REPORT_ALREADY));
    }
    for (who, n) in taken {
        let n = if n == 1 {
            "one act".to_owned()
        } else {
            format!("{n} acts")
        };
        let line = fill(REPORTED)
            .replace("{n}", &n)
            .replace("{actor}", &crate::port::coop::name_of(ctx.w, who));
        ctx.out.sys_to_actor(me, "tacsim.salvage", line);
    }
    Ok(())
}

/// **The lane site `word` names**: its name, location or entry, or its port's venue (`urvara`,
/// as the bot types it).
fn site_named(w: &World, word: &str) -> Option<crate::surface::chart::LaneSite> {
    crate::surface::chart::site_named(w, word).or_else(|| {
        let wanted = word.trim().to_lowercase();
        let entry = crate::port::voyage::lane_ports(w)
            .into_iter()
            .find(|(def, _)| def.site.is_some() && def.venue.as_str() == wanted)
            .and_then(|(_, map)| map.map(|m| m.entry.clone()))?;
        crate::surface::chart::site_named(w, entry.as_str())
    })
}

/// **Whether `who` holds evidence of `act`** (doc 27 Q-S4): a witness pinned on a live custody
/// row over the act's hull, or `who`'s own record of the site learned within
/// [`REPORT_SIGHT_IC_S`] after the act.
fn witnessed(
    record: &crate::surface::site_record::SiteRecord,
    who: ObjId,
    act: &UnlawfulAct,
) -> bool {
    let pinned = record.custody.iter().any(|row| {
        row.hull.0 == act.subject && crate::surface::claim::evidence_of(row, who).is_some()
    });
    pinned
        || record.known.iter().any(|known| {
            known.principal == who
                && known.seen_ic >= act.at_ic
                && known.seen_ic <= act.at_ic.saturating_add(REPORT_SIGHT_IC_S)
        })
}
