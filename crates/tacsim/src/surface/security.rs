//! **Port security** (doc 41 cut RS.7; §4.6, Q-41-24, Q-41-25, Q-38-20, doc 27 Q-S4).
//!
//! A site's evidence log (`SiteRecord.unlawful`, Q-36-30) is evidence, never an enforcement. A
//! row becomes the Registry's business only when it is **proven**: a `Recovery` row whose claim a
//! `recovery` contest vacated (the registry adjudicated it), or any row a witness reported to the
//! registry with evidence of their own (`+salvage/report <site>[=<who>|#<hull>] registry`). At the
//! moment of proof the row is copied to the actor's port-side record (`tacsim:security:{actor}`,
//! [`SecurityRecord`]), since the site's ring keeps only its last 64 rows (round-2 item 41-6).
//!
//! - **The fine** ([`on_docked`], the docking hook): at a Registry port ([`SECURITY_VENUES`]),
//!   for the carrier's master and her operator — never a passenger — every proven unsettled row
//!   is fined by kind ([`fine_major`]) to the Freeport Registry's account. The fine is a one-term
//!   debt (RS.5's `create_debt`, under [`DEBT_NAMESPACE`] so `+debts` lists it), paid in the same
//!   step when the account covers it; the row carries the debt's id, so it is never charged twice.
//! - **The quarantine** (`quarantine_line`, read by `prepare_departure`): while a fine stands
//!   owed against the carrier's master or operator she does not lift. It lifts when the debt is
//!   paid — `+debts/pay`, or the docking check reinstating it at every later docking so the sweep
//!   tries again.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::{Amount, ObligationId, ObligationStatus};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::store::TacsimStore;
use crate::surface::claim::{UnlawfulAct, UnlawfulKind};
use crate::surface::retrieval::DEBT_NAMESPACE;

/// The record's key prefix.
pub const SECURITY_PREFIX: &str = "tacsim:security:";
/// The record's schema.
pub const SECURITY_SCHEMA: u32 = 1;
/// The most proven rows one actor's record keeps; past it the oldest settled row goes.
pub const MAX_SECURITY_ROWS: usize = 64;
/// **The ports whose registry answers for the lane's sites** (§4.6): the Freeport Registry's,
/// at Ceres and at Stickney, by venue. A frontier site has no security.
pub const SECURITY_VENUES: &[&str] = &["ceres", "stickney"];
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
/// The fine paid at docking: `{name}`, `{port}`, `{amount}`, `{what}`.
pub const FINED_PAID: &str =
    "The Freeport Registry fines you {amount} at {port} for {what}; it is paid from your account.";
/// The fine owed: `{name}`, `{port}`, `{amount}`, `{what}`.
pub const FINED_OWED: &str = "The Freeport Registry fines you {amount} at {port} for {what}; your \
     account cannot cover it, so it stands as a debt (+debts); unpaid when its 28 IC days of grace \
     run out, it holds the {name} at port until it is paid.";
/// **The quarantine** (`DepartRefusalKind::Quarantined`): `{name}`, `{port}`, `{amount}`.
pub const QUARANTINED: &str = "The {name} is held at {port}: a fine of {amount} stands against \
     her master; it lifts when it is paid.";

/// **How a row was proven** (Q-41-24).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Proof {
    /// A `recovery` contest vacated the claim: the registry adjudicated it.
    Vacated,
    /// A witness reported it to the registry with evidence of their own.
    Reported {
        /// The witness.
        by: ObjId,
    },
}

/// **One proven act**, held port-side until its fine is settled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenAct {
    /// The site whose log it came from.
    pub site: CanonicalId,
    /// The act, as the site logged it.
    pub act: UnlawfulAct,
    /// How it was proven.
    pub proof: Proof,
    /// When it was proven, IC.
    pub proven_ic: i64,
    /// The fine's one-term debt, `ObligationId.0`, once fined: the receipt that it is never
    /// charged twice. Settled when the debt is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debt: Option<u64>,
}

impl ProvenAct {
    /// Whether this is `act` on `site` (a row's identity: its kind, actor, subject and instant).
    #[must_use]
    pub fn is(&self, site: &CanonicalId, act: &UnlawfulAct) -> bool {
        &self.site == site
            && self.act.kind == act.kind
            && self.act.actor == act.actor
            && self.act.subject == act.subject
            && self.act.at_ic == act.at_ic
    }
}

/// **`tacsim:security:{actor}`**: the proven acts against one actor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityRecord {
    /// [`SECURITY_SCHEMA`].
    pub schema: u32,
    /// The actor.
    pub actor: ObjId,
    /// The proven acts, oldest first, at most [`MAX_SECURITY_ROWS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<ProvenAct>,
}

impl SecurityRecord {
    /// An empty record for `actor`.
    #[must_use]
    pub fn new(actor: ObjId) -> Self {
        Self {
            schema: SECURITY_SCHEMA,
            actor,
            rows: Vec::new(),
        }
    }

    /// The record's own bounds.
    ///
    /// # Errors
    ///
    /// What is out of bounds.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != SECURITY_SCHEMA {
            return Err(format!("schema {} is not {SECURITY_SCHEMA}", self.schema));
        }
        if self.rows.len() > MAX_SECURITY_ROWS {
            return Err(format!(
                "{} rows, over {MAX_SECURITY_ROWS}",
                self.rows.len()
            ));
        }
        if self.rows.iter().any(|row| row.act.actor != self.actor) {
            return Err("a row names another actor".to_owned());
        }
        Ok(())
    }
}

/// **A security record's slot**: read, or claimed by an unreadable document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecuritySlot {
    /// Read and checked.
    Present(Box<SecurityRecord>),
    /// The key is claimed but the document will not read: nothing is proven or fined against
    /// the actor until staff repair it, and her carrier is not held on it.
    Unreadable,
}

/// The key of `actor`'s record.
#[must_use]
pub fn security_key(actor: ObjId) -> SmolStr {
    SmolStr::new(format!("{SECURITY_PREFIX}{}", actor.0))
}

/// The actor a security record's key names.
#[must_use]
pub fn security_key_owner(key: &str) -> Option<ObjId> {
    key.strip_prefix(SECURITY_PREFIX)?
        .parse::<u32>()
        .ok()
        .map(ObjId)
}

/// The slot of `actor`'s record, when one is claimed.
fn slot(w: &World, actor: ObjId) -> Option<&SecuritySlot> {
    w.module_state::<TacsimStore>()?.security.get(&actor)
}

/// **`actor`'s proven acts**, when a record reads.
#[must_use]
pub fn record_of(w: &World, actor: ObjId) -> Option<SecurityRecord> {
    match slot(w, actor)? {
        SecuritySlot::Present(record) => Some((**record).clone()),
        SecuritySlot::Unreadable => None,
    }
}

/// **The fine for `kind`**, major credits (Q-41-25): `None` for a row that carries none of its
/// own — a `Vacated` row proves the `Recovery` it follows, and a capture is doc 42's.
#[must_use]
pub fn fine_major(kind: UnlawfulKind) -> Option<u64> {
    match kind {
        UnlawfulKind::Pickup => Some(FINE_PICKUP_MAJOR),
        UnlawfulKind::Scrap => Some(FINE_SCRAP_MAJOR),
        UnlawfulKind::Recovery => Some(FINE_RECOVERY_MAJOR),
        UnlawfulKind::Vacated | UnlawfulKind::Capture => None,
    }
}

/// What a kind is called in the fine's line.
fn what(kind: UnlawfulKind) -> &'static str {
    match kind {
        UnlawfulKind::Pickup => "an unlawful pickup",
        UnlawfulKind::Scrap => "unlawful scrapping",
        UnlawfulKind::Recovery => "an unlawful recovery",
        UnlawfulKind::Vacated => "a vacated claim",
        UnlawfulKind::Capture => "an unlawful capture",
    }
}

/// **Prove `act` on `site`** (Q-41-24): copied to its actor's record unless it is there already.
/// Answers whether it was added. An actor whose record will not read proves nothing until staff
/// repair it (logged). A full record drops its oldest settled row; a record full of unsettled
/// rows takes no more (logged).
pub(crate) fn prove(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    act: &UnlawfulAct,
    proof: Proof,
) -> bool {
    if fine_major(act.kind).is_none() {
        return false;
    }
    let mut record = match slot(ctx.w, act.actor) {
        None => SecurityRecord::new(act.actor),
        Some(SecuritySlot::Present(record)) => (**record).clone(),
        Some(SecuritySlot::Unreadable) => {
            tracing::error!(
                actor = act.actor.0,
                "tacsim: a proven act meets an unreadable security record; staff repair it"
            );
            return false;
        }
    };
    if record.rows.iter().any(|row| row.is(site, act)) {
        return false;
    }
    if record.rows.len() >= MAX_SECURITY_ROWS {
        let w = &*ctx.w;
        if let Some(i) = record.rows.iter().position(|row| settled(w, row)) {
            record.rows.remove(i);
        } else {
            tracing::error!(
                actor = act.actor.0,
                "tacsim: a security record is full of unsettled fines; a proven act is not kept"
            );
            return false;
        }
    }
    record.rows.push(ProvenAct {
        site: site.clone(),
        act: act.clone(),
        proof,
        proven_ic: ctx.w.world_ic_now().at,
        debt: None,
    });
    crate::store::put_security(ctx, &record);
    true
}

/// The status of the obligation `id`, when it reads.
fn debt_status(w: &World, id: u64) -> Option<ObligationStatus> {
    w.module_state::<mush_economy::EconomyStore>()?
        .snapshot()
        .obligation(ObligationId(id))
        .map(|o| o.status.clone())
}

/// Whether a fined row's debt is still owed.
fn owed(w: &World, row: &ProvenAct) -> bool {
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
fn settled(w: &World, row: &ProvenAct) -> bool {
    row.debt.is_some() && !owed(w, row)
}

/// **Who answers for `carrier`** (§4.6, round-2 item 41-13): her master (the helm holder) and
/// her operator (her title holder, when a character), never a passenger or a crew member.
fn answerers(w: &World, carrier: ObjId) -> Vec<ObjId> {
    let mut who: Vec<ObjId> = crate::port::coop::helm_holder(w, carrier)
        .into_iter()
        .collect();
    let operator = w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().asset(carrier).map(|a| a.title_owner.clone()));
    if let Some(PrincipalId::Object(owner)) = operator
        && !who.contains(&owner)
    {
        who.push(owner);
    }
    who
}

/// Whether `venue` is a Registry port's ([`SECURITY_VENUES`]).
#[must_use]
pub fn answers_at(venue: &CanonicalId) -> bool {
    SECURITY_VENUES.contains(&venue.as_str())
}

/// **The minor units of the fines against `actor` that hold her carrier**: those past their
/// 28-IC-day grace unpaid (`retrieval::past_due`: `Delinquent`, or reinstated and not yet collected; the owner's 2026-11-04 rule — a fine still within its
/// grace is owed and listed on `+debts`, but the carrier may undock).
fn owed_minor(w: &World, actor: ObjId) -> i64 {
    let Some(record) = record_of(w, actor) else {
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
    mush_economy::debts_of(w, &PrincipalId::Object(actor), &[DEBT_NAMESPACE])
        .iter()
        .filter(|o| ids.contains(&o.id.0))
        .fold(0_i64, |sum, o| sum.saturating_add(o.amount.minor()))
}

/// The minor units of the delinquent fines that hold `carrier`: her master's and operator's.
fn held_minor(w: &World, carrier: ObjId) -> i64 {
    answerers(w, carrier)
        .into_iter()
        .fold(0_i64, |sum, who| sum.saturating_add(owed_minor(w, who)))
}

/// **Whether port security holds `carrier`** (`DepartRefusalKind::Quarantined`): a fine against
/// her master or operator past its 28-IC-day grace unpaid.
#[must_use]
pub fn holds(w: &World, carrier: ObjId) -> bool {
    held_minor(w, carrier) > 0
}

/// **The quarantine's line for `carrier`** (`DepartRefusalKind::Quarantined`): `Some` while a
/// fine past its grace stands owed against her master or her operator.
#[must_use]
pub(crate) fn quarantine_line(
    ctx: &Ctx<'_>,
    carrier: ObjId,
    name: &str,
    port: &str,
) -> Option<String> {
    let total = held_minor(ctx.w, carrier);
    (total > 0).then(|| {
        QUARANTINED
            .replace("{name}", name)
            .replace("{port}", port)
            .replace("{amount}", &crate::port::coop::money_minor(ctx, total))
    })
}

/// **A carrier docked at a port of `venue`** (Q-41-25; the docking hook): at a Registry port,
/// every proven unfined row against her master or operator is fined, and every fine still
/// delinquent is reinstated so the sweep tries it again while the hold stands.
pub fn on_docked(ctx: &mut Ctx<'_>, carrier: ObjId, venue: &CanonicalId, port: &str) {
    if !answers_at(venue) {
        return;
    }
    let name = crate::port::coop::name_of(ctx.w, carrier);
    for actor in answerers(ctx.w, carrier) {
        fine_actor(ctx, actor, (&name, port));
    }
}

/// The fines and the reinstatements for one actor (`on_docked`).
fn fine_actor(ctx: &mut Ctx<'_>, actor: ObjId, (name, port): (&str, &str)) {
    let Some(mut record) = record_of(ctx.w, actor) else {
        return;
    };
    let saved = ctx.actor;
    ctx.actor = actor;
    // The fines owed already: a delinquent one goes back to the due pool (a one-shot never
    // retries unprompted).
    for row in &record.rows {
        if let Some(id) = row.debt
            && matches!(debt_status(ctx.w, id), Some(ObligationStatus::Delinquent))
            && let Err(error) = mush_economy::reinstate_debt(ctx, ObligationId(id), DEBT_NAMESPACE)
        {
            tracing::warn!(actor = actor.0, debt = id, %error, "tacsim: a delinquent fine could not be reinstated at docking");
        }
    }
    let Some((_, _, registry_account)) =
        crate::space::picket::resolve_registry_principal_and_account(ctx.w)
    else {
        ctx.actor = saved;
        return;
    };
    let account = crate::port::credit::own_account(ctx.w, actor).map(|a| a.id);
    let mut changed = false;
    for i in 0..record.rows.len() {
        if record.rows[i].debt.is_some() {
            continue;
        }
        let kind = record.rows[i].act.kind;
        let Some(major) = fine_major(kind) else {
            continue;
        };
        let Some(minor) =
            crate::port::apply::minor_of(major, crate::port::apply::credit_scale(ctx.w))
        else {
            continue;
        };
        let Some(account) = account else {
            tracing::warn!(
                actor = actor.0,
                "tacsim: a fine meets an actor with no account; it waits"
            );
            continue;
        };
        let due = crate::surface::retrieval::debt_due(ctx.w);
        let id = match mush_economy::create_debt_due(
            ctx,
            (account, registry_account),
            Amount::from_minor(minor),
            DEBT_NAMESPACE,
            due,
        ) {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(actor = actor.0, %error, "tacsim: a port-security fine could not be opened; it waits");
                continue;
            }
        };
        record.rows[i].debt = Some(id.0);
        changed = true;
        let covers = crate::port::credit::own_account(ctx.w, actor)
            .and_then(|a| a.available())
            .is_some_and(|a| a.minor() >= minor);
        let paid = covers
            && matches!(
                mush_economy::pay_debt(ctx, id, DEBT_NAMESPACE),
                Ok(mush_economy::PostOutcome::Posted(_))
            );
        let line = if paid { FINED_PAID } else { FINED_OWED }
            .replace("{name}", name)
            .replace("{port}", port)
            .replace("{amount}", &crate::port::coop::money_minor(ctx, minor))
            .replace("{what}", what(kind));
        ctx.out.sys_to_actor(actor, "tacsim.security", line);
    }
    if changed {
        crate::store::put_security(ctx, &record);
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
        .filter(|act| fine_major(act.kind).is_some() && act.actor != me)
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
