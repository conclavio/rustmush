//! **The Freeport Registry's prospecting licence** (doc 20 cut SM.5; doc 20 §9.1,
//! §1 beat 1, §5's public-fact line, §11's Tenure row; D-20-12, Q-20-2).
//!
//! A pilot anywhere types `+licence/buy` (a preview, which writes nothing) and
//! `+licence/buy go`: in **one command**, their own act under the issuer the lane's
//! `licence` block names (`freeport_registry`; the `+coop/enrol` shape, no clerk and
//! no NPC on the path, D-18-4), they open an `Entitlement` account in
//! [`crate::economy::LICENCE_FREEPORT`] when they hold none, and pay the fee through
//! a **Public E4 contract** into the Registry's account — offered and accepted by the
//! pilot, accepted by the Registry through the port party-cap source
//! ([`crate::port::market::PortPartyCaps`], no money authority), Ready the moment it
//! activates, settled by the economy's next sweep and chronicled by
//! `CanonEvidenceBridge` with no new code. One [`Licence`] record per **gateway
//! account** (`tacsim:licence:{account}`, the `tacsim:coop:` precedent, C23A) holds
//! the term the economy account cannot: the holder, the expiry, the grace, the
//! terms bought and the last fee contract.
//!
//! **The lapse is read, never written** (lead ruling 5): [`standing_at`] is pure over
//! the record, the entitlement account's status and the IC instant, and every
//! licence-gated act reads [`licence_gate`] at the act. No tick writes anything about
//! a licence. A term runs `term_ic_days` from its start, then `grace_ic_days` of
//! grace, in which the licence **still authorizes** and a buy extends from the
//! expiry (the lead's owner-critical default, §4 decision 16); from `expires + grace`
//! it is lapsed, and a buy then runs from now.
//!
//! **The economy account is never closed on a lapse**, so
//! `mush_industry::extraction_authorized`'s licence half alone stays `Ok` after a
//! lapse: an `Active` account is the whole of what it asks. Every act a licence gates
//! therefore calls [`licence_gate`] **first** — `+tv/lease` now, SM.7's mine start and
//! SM.10's stake and renewal later. Two inheritance rules (reviewer F12):
//!
//! - **(i) SM.7's mine start is an authoritative submission** whose prepare cannot
//!   read a normal-lane tacsim document, so SM.7 reads `licence_gate` at submit and
//!   passes the record's `lapses_ic` as a server-authenticated fixed parameter that
//!   its prepare re-compares with the world's IC now;
//! - **(ii) industry's own `@extract/start`/`@extract/cycle` path**
//!   (`crates/mush-industry/src/provider.rs`, which calls `extraction_authorized`
//!   alone) does not read `licence_gate`; for a surface block it is closed only by
//!   SM.6's "`@extract/start` refuses a surface block" — until SM.6 lands, a
//!   staff-seeded claim on a surface deposit could extract there after a lapse (no
//!   player can hold such a claim before SM.10).
//!
//! `/buy go`'s writes follow `+coop/enrol`'s order — every check first (read-only,
//! `plan_buy`), then the entitlement account (when absent), the offer, the pilot's
//! escrow, the Registry's acceptance, and past the activation nothing fallible: the
//! entitlement account's activation, the record and the reply. A refusal after the
//! first write pins the deferred queue.
//!
//! **No unpaid entitlement is ever `Active`** (the lead's pre-commit round, item 1): the
//! account is opened `Frozen` and set `Active` only once the fee contract has activated,
//! so a buy refused at the offer, the escrow or the Registry's acceptance leaves a
//! `Frozen` account with no record, which `licence_gate` (no record: `Unlicensed`) and
//! `mush_industry::extraction_authorized` (`LicenceInactive`) both refuse. With **no**
//! record a `Frozen` entitlement is read as a refused buy's residue and is reused and
//! activated by the next buy; with a record, a non-`Active` one is a staff suspension
//! and reads [`LICENCE_SUSPENDED`]. **Staff must not unfreeze such a residue by hand**:
//! `@econ/unfreeze` restores only accounts boot froze, but `@account/unfreeze` clears
//! any `Frozen` account, and an `Active` entitlement with no record is exactly the
//! unpaid state above — `licence_gate` still refuses it (`Unlicensed`), but industry's
//! own `@extract` path (rule (ii)) reads `extraction_authorized` alone and would accept
//! it, so until SM.6/SM.10 put `licence_gate` first on every extraction path that
//! unfreeze licenses extraction without a fee; the next `+licence/buy go` is the only
//! way it should ever become `Active`.
//! The contract's `on_expiry` is `Forfeit` to the Registry (reviewer F1): the licence
//! is granted at activation, so the escrow may only ever reach the Registry.

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::{
    AcceptOutcome, Account, AccountId, AccountStatus, Amount, ConsiderationSpec, ContractError,
    ContractId, ContractRoles, ContractSpec, CreditPolicy, EconomicClock, EconomicDeadline,
    EconomicVisibility, EconomyError, EconomyStore, EvidenceKind, Expiry, InstrumentId,
    OutcomeTerms, PayoutShare, Share, TypedCondition,
};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::commands::NOT_ENABLED;
use crate::port::apply::{credit_scale, minor_of};
use crate::port::coop::{money_minor, say};
use crate::port::def::{LicenceDef, PortDef};
use crate::port::map::PortMap;
use crate::store::{TacsimStore, put_licence};

/// The verb.
pub const LICENCE_VERB: &str = "+licence";
/// The document prefix: `tacsim:licence:{account canonical hash}`.
pub const LICENCE_PREFIX: &str = "tacsim:licence:";
/// The record's schema.
pub const LICENCE_SCHEMA: u16 = 1;
/// **The fee contract's real-time settlement window**, seconds (the buyout's
/// `BUYOUT_WINDOW_SECS`): its `Before` row is Ready at activation, so the next
/// economy sweep settles it long before this; past it, a stalled settlement forfeits
/// the escrow to the Registry (reviewer F1).
pub const LICENCE_SETTLE_WINDOW_SECS: i64 = 3_600;
/// An IC day, seconds.
const IC_DAY: i64 = 86_400;
/// The name of the pilot's entitlement account and of the fee contract.
const LICENCE_NAME: &str = "Freeport prospecting licence";

/// `+licence`'s help.
pub const LICENCE_HELP: &str = "+licence  --  The Freeport Registry's prospecting licence: it \
     lets you work ore at the Registry's mining sites. +licence (or +licence/show) reads yours, \
     anywhere.\n+licence/buy  previews a licence: 2500.00cr for 168 IC days, then 3 IC days' \
     grace, bought from anywhere as a public contract with the Registry; bought while yours \
     stands, the new term runs on from the day it ends. +licence/buy go buys exactly what the \
     preview printed. One licence an account.";
/// No licence held.
pub const LICENCE_NONE: &str = "You hold no prospecting licence. +licence/buy takes one from the \
     {issuer}: {fee} for {term} IC days, from anywhere.";
/// A licence in force.
pub const LICENCE_IN_FORCE: &str = "Your prospecting licence from the {issuer}, account \
     #{account}: in force until {until} ({days} IC days left), then {grace} IC days' grace.";
/// A licence in its grace.
pub const LICENCE_GRACE: &str = "Your prospecting licence from the {issuer}, account #{account}: \
     it ended at {until}; its grace runs to {lapses}. +licence/buy renews it from {until}.";
/// A lapsed licence (doc 20 §5's public fact).
pub const LICENCE_LAPSED: &str =
    "Your licence lapsed at {lapses}. +licence/buy takes a new one from today.";
/// The entitlement account is frozen.
pub const LICENCE_SUSPENDED: &str = "The {issuer} has suspended your prospecting licence, \
     account #{account}. Ask staff about it.";
/// Another character of the account holds the licence.
pub const LICENCE_OTHER: &str = "The {issuer} already licenses another character of your \
     account: one licence an account.";
/// The record cannot be read.
pub const LICENCE_BOOKS_DARK: &str =
    "The Registry's books for your licence cannot be read right now. Ask staff to look at them.";
/// No gateway account.
pub const LICENCE_NO_MATCH: &str =
    "The Registry cannot match you to an account, so it cannot license you.";
/// No credit account.
pub const LICENCE_NO_CREDIT: &str = "You hold no credit account to pay from; Ceres Clearing opens \
     one when you enrol at the Registry & Co-op desk.";
/// Short of the fee.
pub const LICENCE_FUNDS: &str = "A licence costs {fee}; you have {available}.";
/// The preview of a new licence (or one after a lapse).
pub const LICENCE_PREVIEW: &str = "The {issuer} licenses you to prospect at its sites for {term} \
     IC days, until {until}, for {fee}. +licence/buy go buys it.";
/// The preview of an extension.
pub const LICENCE_PREVIEW_EXTEND: &str = "The {issuer} extends your licence by {term} IC days, \
     from {from} to {until}, for {fee}. +licence/buy go buys it.";
/// A licence bought.
pub const LICENCE_BOUGHT: &str = "The {issuer} licenses you to prospect at its sites until \
     {until}: {fee} paid, account #{account}.";
/// A licence extended.
pub const LICENCE_EXTENDED: &str = "The {issuer} extends your licence to {until}: {fee} paid.";
/// The licence could not be issued; nothing was paid.
pub const LICENCE_FAILED: &str = "The Registry could not issue your licence; nothing was paid.";
/// The licence could not be issued and the refund is the sweep's.
pub const LICENCE_REFUND_PENDING: &str = "The Registry could not issue your licence; your {fee} \
     comes back when the exchange settles it.";
/// No lane port authors a licence.
pub const LICENCE_NOT_HERE: &str = "The Freeport Registry issues no prospecting licences on this \
     world.";
/// The grammar.
pub const LICENCE_USAGE: &str = "Usage: +licence[/show]  or  +licence/buy [go]";

/// **`tacsim:licence:{account}`** — the one key of one gateway account's licence
/// (the `tacsim:coop:` precedent). Never logged: its tail is the account's
/// canonical hash.
#[must_use]
pub fn licence_key(account: mush_core::AccountId) -> SmolStr {
    SmolStr::new(format!("{LICENCE_PREFIX}{}", account.canonical_hash()))
}

/// **One gateway account's prospecting licence** (doc 20 §9.1; D-18-25's "per
/// gateway account"): the term the entitlement account cannot carry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Licence {
    /// [`LICENCE_SCHEMA`].
    pub schema: u16,
    /// The gateway account (the key).
    pub account: mush_core::AccountId,
    /// The character that bought it: one licence an account (C23A).
    pub holder: ObjId,
    /// The lane block's org (`freeport_registry`).
    pub issuer: CanonicalId,
    /// The holder's entitlement account, `mush_economy::AccountId.0` in
    /// [`crate::economy::LICENCE_FREEPORT`].
    pub entitlement: u64,
    /// The IC instant the first term began.
    pub first_bought_ic: i64,
    /// The IC instant the current term ends (the grace runs on from it).
    pub expires_ic: i64,
    /// The grace after the term, IC seconds.
    pub grace_secs: i64,
    /// Terms bought, `>= 1`.
    pub terms: u32,
    /// The last fee contract, `ContractId.0`.
    pub contract: u64,
}

impl Licence {
    /// The record's own bounds.
    ///
    /// # Errors
    ///
    /// The first bound broken, named.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != LICENCE_SCHEMA {
            return Err(format!("schema {} is not {LICENCE_SCHEMA}", self.schema));
        }
        if self.terms == 0 {
            return Err("terms must be at least 1".to_owned());
        }
        if self.entitlement == 0 || self.contract == 0 {
            return Err("the entitlement account and the contract are named".to_owned());
        }
        if self.expires_ic <= self.first_bought_ic {
            return Err("the licence expires after it was first bought".to_owned());
        }
        if self.grace_secs < 0 {
            return Err("the grace is not negative".to_owned());
        }
        if self.lapses_ic().is_none() {
            return Err("the expiry and the grace overflow".to_owned());
        }
        Ok(())
    }

    /// The IC instant the licence lapses: the expiry plus the grace, checked.
    #[must_use]
    pub fn lapses_ic(&self) -> Option<i64> {
        self.expires_ic.checked_add(self.grace_secs)
    }
}

/// One gateway account's licence as the store holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LicenceState<'a> {
    /// No key claims it.
    Absent,
    /// The readable record.
    Present(&'a Licence),
    /// A claimed key whose body did not read: bars that account.
    Unreadable,
}

/// **Where a pilot's licence stands at an IC instant** (lead ruling 5). `Unlicensed`,
/// never `None` (reviewer F9: a variant named `None` shadows `Option::None` under a
/// glob import).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LicenceStanding {
    /// No licence for the pilot's account (or, with no account, for the pilot).
    Unlicensed,
    /// In its term.
    InForce {
        /// The term's end.
        expires_ic: i64,
        /// The grace's end.
        lapses_ic: i64,
    },
    /// Past its term, in its grace: it still authorizes.
    Grace {
        /// The term's end.
        expired_ic: i64,
        /// The grace's end.
        lapses_ic: i64,
    },
    /// Past its grace.
    Lapsed {
        /// The instant it lapsed.
        at_ic: i64,
    },
    /// The entitlement account is absent or not `Active` (staff froze it).
    Suspended,
    /// Another character of the pilot's gateway account holds it.
    Other,
    /// The record cannot be read.
    Unreadable,
}

/// **The standing of one record** — pure, read at every act. `Suspended` first (an
/// entitlement account that is not `Active`), then the dates: in force while
/// `now < expires_ic`, in grace while `expires_ic <= now < lapses_ic`, lapsed from
/// `lapses_ic` (bought at `t0` with 168 + 3 IC days: in force to `t0 + 168 d`, grace to
/// `t0 + 171 d`, lapsed from `t0 + 171 d`).
#[must_use]
pub fn standing_at(record: &Licence, entitlement_active: bool, now_ic: i64) -> LicenceStanding {
    if !entitlement_active {
        return LicenceStanding::Suspended;
    }
    let lapses_ic = record.lapses_ic().unwrap_or(i64::MAX);
    if now_ic < record.expires_ic {
        LicenceStanding::InForce {
            expires_ic: record.expires_ic,
            lapses_ic,
        }
    } else if now_ic < lapses_ic {
        LicenceStanding::Grace {
            expired_ic: record.expires_ic,
            lapses_ic,
        }
    } else {
        LicenceStanding::Lapsed { at_ic: lapses_ic }
    }
}

/// The pilot's gateway account, through the account oracle.
pub(crate) fn gateway_account(w: &World, pilot: ObjId) -> Option<mush_core::AccountId> {
    w.module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.account_of(pilot))
}

/// The pilot's **unscoped** account in the licence instrument and its status
/// (`mush_industry::licence_accounts`'s derivation, with the id, lowest first).
fn entitlement_account(w: &World, pilot: ObjId) -> Option<(AccountId, AccountStatus)> {
    let instrument = crate::economy::licence_instrument()?;
    w.module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(&PrincipalId::Object(pilot))
        .filter(|account| account.instrument == instrument && account.scope.is_none())
        .min_by_key(|account| account.id)
        .map(|account| (account.id, account.status))
}

/// Whether the pilot's entitlement account is `Active` (`mush_industry`'s one licence
/// predicate, read through its own function).
fn entitlement_active(w: &World, pilot: ObjId) -> bool {
    crate::economy::licence_instrument().is_some_and(|instrument| {
        mush_industry::licence::licence_accounts(w, &instrument, &PrincipalId::Object(pilot))
            == Some(AccountStatus::Active)
    })
}

/// What the store holds for the pilot: by gateway account; with no oracle answer,
/// the pilot's own record by holder.
fn record_for(w: &World, pilot: ObjId) -> LicenceState<'_> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return LicenceState::Absent;
    };
    match gateway_account(w, pilot) {
        Some(account) => store.licence_state(account),
        None => store
            .licence_of_holder(pilot)
            .map_or(LicenceState::Absent, LicenceState::Present),
    }
}

/// **A pilot's standing at `now_ic`**: the record by gateway account (an alt of a
/// licensed account reads `Other`), the entitlement account's status, then the dates.
#[must_use]
pub fn licence_standing(w: &World, pilot: ObjId, now_ic: i64) -> LicenceStanding {
    match record_for(w, pilot) {
        LicenceState::Absent => LicenceStanding::Unlicensed,
        LicenceState::Unreadable => LicenceStanding::Unreadable,
        LicenceState::Present(record) if record.holder != pilot => LicenceStanding::Other,
        LicenceState::Present(record) => standing_at(record, entitlement_active(w, pilot), now_ic),
    }
}

/// **The licence gate** every licence-gated act reads first (lead ruling 5; doc 20
/// §8): `Ok` while the licence is in force or in its grace.
///
/// # Errors
///
/// The standing, for every other case.
pub fn licence_gate(w: &World, pilot: ObjId, now_ic: i64) -> Result<(), LicenceStanding> {
    match licence_standing(w, pilot, now_ic) {
        LicenceStanding::InForce { .. } | LicenceStanding::Grace { .. } => Ok(()),
        other => Err(other),
    }
}

/// **An IC instant as doc 20 §5's public fact prints it**: `"MM-DD HH:MM"` on the game
/// calendar (`mush_core::CivilDate`), zero-padded, no year — "Your licence lapsed at
/// 05-21 14:00." (reviewer F9).
#[must_use]
pub fn ic_stamp(at: i64) -> String {
    let date = mush_core::CivilDate::from_ic_seconds(at);
    let secs = at.rem_euclid(IC_DAY);
    format!(
        "{:02}-{:02} {:02}:{:02}",
        date.month,
        date.day,
        secs / 3_600,
        (secs % 3_600) / 60
    )
}

/// Whole IC days from `now` to `until`, rounded up (0 when past).
fn days_left(now: i64, until: i64) -> i64 {
    until.saturating_sub(now).max(0).saturating_add(IC_DAY - 1) / IC_DAY
}

/// **The lane's licence port**: the first lane port entry (pack order) authoring a
/// `licence` block whose map is present.
pub(crate) fn licence_port(w: &World) -> Option<(&PortDef, &PortMap, &LicenceDef)> {
    crate::port::market::ports(w)
        .into_iter()
        .find_map(|(_, def, map)| Some((def, map?, def.licence.as_deref()?)))
}

/// The issuer's name in the world (the org's authored name).
fn issuer_name(def: &PortDef, licence: &LicenceDef) -> String {
    def.orgs
        .iter()
        .find(|org| org.id == licence.org)
        .map_or_else(|| licence.org.to_string(), |org| org.name.clone())
}

/// **The readiness report's Licence line** (doc 20 cut SM.5, lead ruling 10;
/// `Binding::Licence`, reported and never gated — mining content never closes the
/// trade-run desk): for a port whose entry authors `licence`, Ready when the industry
/// layer names [`crate::economy::LICENCE_FREEPORT`] as its extraction licence, the
/// instrument is a registered entitlement and the issuer has its account; Missing,
/// naming which, otherwise.
pub(crate) fn licence_line(
    w: &World,
    entry: &CanonicalId,
    def: &PortDef,
    map: &PortMap,
) -> Option<(crate::port::LineState, String)> {
    use crate::port::LineState;
    let licence = def.licence.as_ref()?;
    let head = format!("{entry} licence {}", crate::economy::LICENCE_FREEPORT);
    let issuer = issuer_name(def, licence);
    let named = w
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|store| store.licence().cloned());
    let missing = |why: String| Some((LineState::Missing, format!("{head}: {why}")));
    let Some(named) = named else {
        return missing("the industry layer names no extraction licence.".to_owned());
    };
    if named.as_str() != crate::economy::LICENCE_FREEPORT {
        return missing(format!("the industry layer names {named}."));
    }
    if !mush_industry::licence::is_entitlement(w, &named) {
        return missing("it is not registered as an entitlement.".to_owned());
    }
    let account = map
        .orgs
        .get(&licence.org)
        .and_then(|row| row.account)
        .and_then(|id| {
            w.module_state::<EconomyStore>()
                .and_then(|economy| economy.snapshot().account(AccountId(id)).cloned())
        });
    if account.is_none() {
        return missing(format!("the {issuer} has no account."));
    }
    Some((
        LineState::Ready,
        format!(
            "{head}: the {issuer} issues it at {} for {} IC days with {} IC days' grace; \
             extraction reads it.",
            crate::port::readiness::money_major(w, licence.fee_major),
            licence.term_ic_days,
            licence.grace_ic_days
        ),
    ))
}

/// The fee in minor units of the credit.
fn fee_minor(w: &World, licence: &LicenceDef) -> Option<i64> {
    minor_of(licence.fee_major, credit_scale(w))
}

/// **The fee contract** (doc 20 §9.1; `buyout_spec`'s shape): pure, so its shape is
/// tested without a world. The pilot is issuer, author and payer; the Registry the
/// one party; one consideration (the fee from the pilot's account), one outcome
/// paying exactly the fee to the Registry's account, a bare `Before` row on a real
/// deadline (Ready at activation, so the sweep settles it), `Forfeit` to the Registry
/// on expiry (reviewer F1), and **Public**, so `CanonEvidenceBridge` chronicles it.
#[must_use]
pub(crate) fn licence_spec(
    pilot_account: AccountId,
    registry: &PrincipalId,
    registry_account: AccountId,
    pilot: ObjId,
    fee: Amount,
    now: i64,
) -> ContractSpec {
    let deadline = EconomicDeadline {
        clock: EconomicClock::RealUtc,
        at: now.saturating_add(LICENCE_SETTLE_WINDOW_SECS),
    };
    let me = PrincipalId::Object(pilot);
    ContractSpec {
        name: SmolStr::new_static(LICENCE_NAME),
        // `None`: the pilot is the issuer, so the Registry's issuer cap is untouched.
        scope: None,
        parties: vec![registry.clone()],
        considerations: vec![ConsiderationSpec {
            account: pilot_account,
            amount: fee,
        }],
        outcomes: vec![OutcomeTerms {
            name: SmolStr::new_static("licensed"),
            when: vec![0],
            payouts: vec![PayoutShare {
                to: registry_account,
                share: Share::Exact(fee),
            }],
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences: Vec::new(),
        }],
        remainder_to: pilot_account,
        conditions: vec![TypedCondition::Before { deadline }],
        adjudicator: None,
        evidence: vec![EvidenceKind::Before],
        deadline: Some(deadline),
        on_expiry: Expiry::Forfeit {
            to: registry_account,
        },
        visibility: EconomicVisibility::Public,
        roles: ContractRoles {
            author: Some(me.clone()),
            payer: Some(me),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables: Vec::new(),
    }
}

/// **`+licence`, `+licence/show`, `+licence/buy [go]`** (doc 20 §9.1). Accepted from
/// anywhere.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn licence_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let enabled = ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled);
    if !enabled {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    if licence_port(ctx.w).is_none() {
        return Err(CmdError::msg(LICENCE_NOT_HERE));
    }
    let raw = ctx.args.raw().trim().to_owned();
    if ctx.switches.has("buy") {
        let mut words = raw.split_whitespace();
        match (words.next(), words.next()) {
            (None, _) => preview(ctx),
            (Some(word), None) if word.eq_ignore_ascii_case("go") => buy(ctx),
            _ => Err(CmdError::msg(LICENCE_USAGE)),
        }
    } else if raw.is_empty() {
        show(ctx)
    } else {
        Err(CmdError::msg(LICENCE_USAGE))
    }
}

/// The bare verb and `/show`: one line by standing.
fn show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let now = ctx.w.world_ic_now().at;
    let Some((def, _, licence)) = licence_port(ctx.w) else {
        return Err(CmdError::msg(LICENCE_NOT_HERE));
    };
    let issuer = issuer_name(def, licence);
    let fee = fee_minor(ctx.w, licence).map_or_else(
        || format!("{}cr", licence.fee_major),
        |minor| money_minor(ctx, minor),
    );
    let (term, grace) = (licence.term_ic_days, licence.grace_ic_days);
    let standing = licence_standing(ctx.w, actor, now);
    let entitlement = match record_for(ctx.w, actor) {
        LicenceState::Present(record) => record.entitlement,
        LicenceState::Absent | LicenceState::Unreadable => 0,
    };
    let line = match standing {
        LicenceStanding::Unlicensed => LICENCE_NONE
            .replace("{issuer}", &issuer)
            .replace("{fee}", &fee)
            .replace("{term}", &term.to_string()),
        LicenceStanding::InForce { expires_ic, .. } => LICENCE_IN_FORCE
            .replace("{issuer}", &issuer)
            .replace("{account}", &entitlement.to_string())
            .replace("{until}", &ic_stamp(expires_ic))
            .replace("{days}", &days_left(now, expires_ic).to_string())
            .replace("{grace}", &grace.to_string()),
        LicenceStanding::Grace {
            expired_ic,
            lapses_ic,
        } => LICENCE_GRACE
            .replace("{issuer}", &issuer)
            .replace("{account}", &entitlement.to_string())
            .replace("{until}", &ic_stamp(expired_ic))
            .replace("{lapses}", &ic_stamp(lapses_ic)),
        // Doc 20 cut SM.7 (the brief's reviewer finding F21): every standing that stops an act
        // prints one line, here and wherever a licence-gated act reads it.
        LicenceStanding::Lapsed { .. }
        | LicenceStanding::Suspended
        | LicenceStanding::Other
        | LicenceStanding::Unreadable => {
            standing_line(ctx.w, actor, &standing).unwrap_or_else(|| LICENCE_BOOKS_DARK.to_owned())
        }
    };
    say(ctx, line);
    Ok(())
}

/// **The line a refusing standing prints** (doc 20 cut SM.7; the SM.7 brief's 3.3 item 4,
/// reviewer F21): one line a standing, wherever it is read — `+licence` and every licence-gated
/// act (mining's start and the run's next cycle). `Lapsed` → [`LICENCE_LAPSED`] at
/// [`ic_stamp`]; `Suspended` → [`LICENCE_SUSPENDED`] with the issuer and the asker's own
/// entitlement account (theirs to see); `Other` → [`LICENCE_OTHER`] ("one licence an account" —
/// never a `+licence/buy` line that would only refuse them); `Unreadable` →
/// [`LICENCE_BOOKS_DARK`], logged. `None` for a standing that authorizes (in force, in grace) and
/// for `Unlicensed`, whose line names the fee and is each verb's own. With no licence port on the
/// lane, the line `+licence` prints then: [`LICENCE_NOT_HERE`].
#[must_use]
pub(crate) fn standing_line(w: &World, pilot: ObjId, standing: &LicenceStanding) -> Option<String> {
    let issuer = || licence_port(w).map(|(def, _, licence)| issuer_name(def, licence));
    let entitlement = || match record_for(w, pilot) {
        LicenceState::Present(record) => record.entitlement,
        LicenceState::Absent | LicenceState::Unreadable => 0,
    };
    let line = match standing {
        LicenceStanding::Unlicensed
        | LicenceStanding::InForce { .. }
        | LicenceStanding::Grace { .. } => return None,
        LicenceStanding::Lapsed { at_ic } => LICENCE_LAPSED.replace("{lapses}", &ic_stamp(*at_ic)),
        LicenceStanding::Suspended => match issuer() {
            Some(issuer) => LICENCE_SUSPENDED
                .replace("{issuer}", &issuer)
                .replace("{account}", &entitlement().to_string()),
            None => LICENCE_NOT_HERE.to_owned(),
        },
        LicenceStanding::Other => match issuer() {
            Some(issuer) => LICENCE_OTHER.replace("{issuer}", &issuer),
            None => LICENCE_NOT_HERE.to_owned(),
        },
        LicenceStanding::Unreadable => {
            tracing::error!(
                pilot = pilot.0,
                "tacsim: a licence-gated read found an unreadable licence for the asking account; \
                 staff repair the tacsim:licence: document"
            );
            LICENCE_BOOKS_DARK.to_owned()
        }
    };
    Some(line)
}

/// Everything `/buy go` writes, decided before the first write.
struct BuyPlan {
    /// The pilot's gateway account (the record's key).
    account: mush_core::AccountId,
    /// The record standing now, when one does (an extension keeps its holder and
    /// first purchase).
    current: Option<Licence>,
    /// The pilot's entitlement account, when it exists: `Active`, or the `Frozen` residue
    /// of a refused buy (no record), which this buy activates.
    entitlement: Option<AccountId>,
    /// Whether this buy must set the entitlement account `Active` once the fee contract
    /// activates: one it opens, or a refused buy's `Frozen` residue.
    activate: bool,
    /// The pilot's credit account.
    pilot_account: AccountId,
    /// The Registry as a principal and its account.
    registry: PrincipalId,
    registry_account: AccountId,
    /// The issuer's org id and name.
    issuer: CanonicalId,
    issuer_name: String,
    /// The fee, minor units.
    fee_minor: i64,
    /// The new term's start (the expiry when extending, else now) and end.
    from: i64,
    until: i64,
    /// The grace, IC seconds.
    grace_secs: i64,
    /// The term, IC days.
    term: u32,
    /// Whether the buy extends a standing licence (in force or in grace).
    extends: bool,
}

/// **`/buy`'s checks, in order, read-only** (`plan_enrolment`'s shape). `Err` carries
/// the line.
fn plan_buy(ctx: &Ctx<'_>) -> Result<BuyPlan, String> {
    let w: &World = ctx.w;
    let actor = ctx.actor;
    let now = w.world_ic_now().at;
    let Some((def, map, licence)) = licence_port(w) else {
        return Err(LICENCE_NOT_HERE.to_owned());
    };
    let issuer_name = issuer_name(def, licence);
    // 1. The gateway account (fail closed).
    let Some(account) = gateway_account(w, actor) else {
        return Err(LICENCE_NO_MATCH.to_owned());
    };
    // 2. The account's record.
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Err(NOT_ENABLED.to_owned());
    };
    let current = match store.licence_state(account) {
        LicenceState::Absent => None,
        LicenceState::Unreadable => {
            tracing::error!(
                "tacsim: +licence/buy found an unreadable licence for the buying account; staff \
                 repair the tacsim:licence: document"
            );
            return Err(LICENCE_BOOKS_DARK.to_owned());
        }
        LicenceState::Present(record) if record.holder != actor => {
            return Err(LICENCE_OTHER.replace("{issuer}", &issuer_name));
        }
        LicenceState::Present(record) => Some(record.clone()),
    };
    // The entitlement account (the lead's pre-commit round, item 1): an `Active` one is
    // reused; a `Frozen` one with no record is the residue of a buy refused before its
    // contract activated (step a opens it `Frozen`), reused and activated by this buy;
    // any other non-`Active` one — `Frozen` beside a record, or `Closing`/`Closed`, which
    // only staff write — is a suspension.
    let (entitlement, activate) = match (entitlement_account(w, actor), &current) {
        (Some((id, AccountStatus::Active)), _) => (Some(id), false),
        (Some((id, AccountStatus::Frozen)), None) => (Some(id), true),
        (Some((id, _)), _) => {
            return Err(LICENCE_SUSPENDED
                .replace("{issuer}", &issuer_name)
                .replace("{account}", &id.0.to_string()));
        }
        (None, _) => (None, true),
    };
    // 3. The pilot's credit account and its funds.
    let Some(credit) = crate::port::credit::own_account(w, actor) else {
        return Err(LICENCE_NO_CREDIT.to_owned());
    };
    let pilot_account = credit.id;
    let Some(fee_minor) = fee_minor(w, licence) else {
        tracing::error!("tacsim: +licence/buy: the fee overflows the credit's scale");
        return Err(LICENCE_FAILED.to_owned());
    };
    let available = credit.available().map_or(0, Amount::minor);
    if available < fee_minor {
        return Err(LICENCE_FUNDS
            .replace("{fee}", &money_minor(ctx, fee_minor))
            .replace("{available}", &money_minor(ctx, available)));
    }
    // 4. The economy, the instrument and the Registry.
    let Some(economy) = w.module_state::<EconomyStore>() else {
        tracing::error!("tacsim: +licence/buy: the economy is not composed");
        return Err(LICENCE_FAILED.to_owned());
    };
    if economy.is_frozen() {
        tracing::error!("tacsim: +licence/buy: the economy is frozen");
        return Err(LICENCE_FAILED.to_owned());
    }
    let instrument: Option<InstrumentId> = crate::economy::licence_instrument();
    if !instrument.is_some_and(|id| mush_industry::licence::is_entitlement(w, &id)) {
        tracing::error!(
            instrument = crate::economy::LICENCE_FREEPORT,
            "tacsim: +licence/buy: the licence instrument is not registered as an entitlement"
        );
        return Err(LICENCE_FAILED.to_owned());
    }
    let Some(row) = map.orgs.get(&licence.org) else {
        tracing::error!(org = %licence.org, "tacsim: +licence/buy: the issuer has no map row");
        return Err(LICENCE_FAILED.to_owned());
    };
    let registry = PrincipalId::Organization(OrgId(row.org));
    let org_live = w
        .module_state::<mush_org::OrgStore>()
        .is_some_and(|orgs| orgs.state().org(OrgId(row.org)).is_some());
    if !org_live {
        tracing::error!(org = %licence.org, "tacsim: +licence/buy: the issuer's org is not on this world");
        return Err(LICENCE_FAILED.to_owned());
    }
    let Some(registry_account) = row.account.map(AccountId).filter(|id| {
        economy
            .snapshot()
            .account(*id)
            .is_some_and(|a| a.status == AccountStatus::Active)
    }) else {
        tracing::error!(org = %licence.org, "tacsim: +licence/buy: the issuer has no live account");
        return Err(LICENCE_FAILED.to_owned());
    };
    // 5. The new term: from the expiry while the licence stands (in force or in its
    // grace), else from now. No cap on how far ahead (§4 decision 8).
    let standing_until = current
        .as_ref()
        .and_then(Licence::lapses_ic)
        .filter(|lapses| now < *lapses);
    let (from, extends) = match (&current, standing_until) {
        (Some(record), Some(_)) => (record.expires_ic, true),
        _ => (now, false),
    };
    let term_secs = i64::from(licence.term_ic_days).checked_mul(IC_DAY);
    let grace_secs = i64::from(licence.grace_ic_days).checked_mul(IC_DAY);
    let (Some(until), Some(grace_secs)) = (term_secs.and_then(|t| from.checked_add(t)), grace_secs)
    else {
        tracing::error!("tacsim: +licence/buy: the new term overflows");
        return Err(LICENCE_FAILED.to_owned());
    };
    if until.checked_add(grace_secs).is_none() {
        tracing::error!("tacsim: +licence/buy: the new term's grace overflows");
        return Err(LICENCE_FAILED.to_owned());
    }
    Ok(BuyPlan {
        account,
        current,
        entitlement,
        activate,
        pilot_account,
        registry,
        registry_account,
        issuer: licence.org.clone(),
        issuer_name,
        fee_minor,
        from,
        until,
        grace_secs,
        term: licence.term_ic_days,
        extends,
    })
}

/// `/buy`: the preview. Writes nothing.
fn preview(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let plan = plan_buy(ctx).map_err(CmdError::msg)?;
    let fee = money_minor(ctx, plan.fee_minor);
    let line = if plan.extends {
        LICENCE_PREVIEW_EXTEND
            .replace("{issuer}", &plan.issuer_name)
            .replace("{term}", &plan.term.to_string())
            .replace("{from}", &ic_stamp(plan.from))
            .replace("{until}", &ic_stamp(plan.until))
            .replace("{fee}", &fee)
    } else {
        LICENCE_PREVIEW
            .replace("{issuer}", &plan.issuer_name)
            .replace("{term}", &plan.term.to_string())
            .replace("{until}", &ic_stamp(plan.until))
            .replace("{fee}", &fee)
    };
    say(ctx, line);
    Ok(())
}

/// A refusal after the first write: the deferred queue is pinned so RAM and the
/// store agree on the residue (`failed_after_writes`'s reason, `coop.rs`).
fn refuse_after_writes(ctx: &mut Ctx<'_>, line: String) -> CmdError {
    ctx.out.pin_deferred();
    CmdError::msg(line)
}

/// Cancel the verb's own fee contract: `true` only when it is terminal now
/// (`buyout::cancel_own`'s rule); anything else is logged and answers `false`.
fn cancel_own(ctx: &mut Ctx<'_>, id: ContractId) -> bool {
    let outcome = mush_economy::cancel_contract(ctx, id);
    if matches!(outcome, Ok(true)) {
        return true;
    }
    tracing::error!(
        contract = id.0,
        ?outcome,
        "tacsim: +licence/buy could not cancel its own fee contract; its refund is retrying in \
         the economy sweep; `@contract/show #<id>`"
    );
    false
}

/// **`/buy go`** — ONE command (doc 20 §9.1; `enrol`'s order).
#[allow(clippy::too_many_lines)] // one ordered write list with each step's residue
fn buy(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let plan = plan_buy(ctx).map_err(CmdError::msg)?;
    let actor = ctx.actor;
    let fee = money_minor(ctx, plan.fee_minor);
    let refund_pending = LICENCE_REFUND_PENDING.replace("{fee}", &fee);
    // a. The entitlement account, when absent (its own allocator reach), opened `Frozen`:
    // it is set `Active` only once the fee contract activates (step e), so no refusal
    // below leaves an unpaid entitlement `extraction_authorized` would accept.
    let (entitlement, opened) = match plan.entitlement {
        Some(id) => (id, false),
        None => {
            let Some(instrument) = crate::economy::licence_instrument() else {
                return Err(CmdError::msg(LICENCE_FAILED));
            };
            let Some(id) = ctx
                .w
                .module_state_mut::<EconomyStore>()
                .and_then(EconomyStore::alloc_account)
            else {
                tracing::error!("tacsim: +licence/buy: the account allocator is spent");
                return Err(CmdError::msg(LICENCE_FAILED));
            };
            mush_economy::open_account(
                ctx,
                Account {
                    id,
                    owner: PrincipalId::Object(actor),
                    instrument,
                    scope: None,
                    name: SmolStr::new_static(LICENCE_NAME),
                    status: AccountStatus::Frozen,
                    posted_balance: Amount::ZERO,
                    held_amount: Amount::ZERO,
                    last_hold_seq: 0,
                    credit: CreditPolicy::forbid(),
                    last_journal_seq: 0,
                    revision: 1,
                    approval: None,
                },
            );
            (id, true)
        }
    };
    // b. The offer, as the pilot.
    let spec = licence_spec(
        plan.pilot_account,
        &plan.registry,
        plan.registry_account,
        actor,
        Amount::from_minor(plan.fee_minor),
        ctx.w.now(),
    );
    let id = match mush_economy::offer_contract(ctx, spec) {
        Ok(id) => id,
        Err(error) => {
            tracing::error!(%error, pilot = actor.0, "tacsim: +licence/buy: the fee contract was not offered");
            let line = LICENCE_FAILED.to_owned();
            return Err(if opened {
                refuse_after_writes(ctx, line)
            } else {
                CmdError::msg(line)
            });
        }
    };
    // c. The pilot's escrow; the contract stays Offered (the Registry has not accepted).
    match mush_economy::accept_contract(ctx, id) {
        Ok(AcceptOutcome::Accepted { .. }) => {}
        other => {
            let short = matches!(
                other,
                Err(ContractError::Refused(EconomyError::InsufficientAvailable(
                    _
                )))
            );
            tracing::error!(contract = id.0, outcome = ?other, "tacsim: +licence/buy: the pilot's escrow was not placed");
            let line = if !cancel_own(ctx, id) {
                refund_pending
            } else if short {
                let available = crate::port::credit::own_account(ctx.w, actor)
                    .and_then(Account::available)
                    .map_or(0, Amount::minor);
                LICENCE_FUNDS
                    .replace("{fee}", &fee)
                    .replace("{available}", &money_minor(ctx, available))
            } else {
                LICENCE_FAILED.to_owned()
            };
            return Err(refuse_after_writes(ctx, line));
        }
    }
    // d. The Registry accepts through the party-cap source; the contract activates.
    match mush_economy::accept_contract_as(ctx, id, &plan.registry) {
        Ok(AcceptOutcome::Accepted {
            activated: true, ..
        }) => {}
        other => {
            tracing::error!(contract = id.0, outcome = ?other, "tacsim: +licence/buy: the Registry's acceptance did not activate the fee contract");
            let line = if cancel_own(ctx, id) {
                LICENCE_FAILED.to_owned()
            } else {
                refund_pending
            };
            return Err(refuse_after_writes(ctx, line));
        }
    }
    // e. Past the activation nothing is fallible: the entitlement account goes `Active`
    // (one this buy opened, or a refused buy's residue; a renewal's already is), then the
    // record.
    if plan.activate && !mush_economy::set_account_status(ctx, entitlement, AccountStatus::Active) {
        // Unreachable (step a opened it, or the plan read it); logged, and the record is
        // written anyway (the contract stands) — the standing then reads `Suspended`.
        tracing::error!(
            contract = id.0,
            entitlement = entitlement.0,
            "tacsim: +licence/buy could not activate the entitlement account; staff \
             `@account/unfreeze` it"
        );
    }
    let record = match &plan.current {
        Some(current) => Licence {
            schema: LICENCE_SCHEMA,
            account: plan.account,
            holder: current.holder,
            issuer: plan.issuer.clone(),
            entitlement: entitlement.0,
            first_bought_ic: current.first_bought_ic,
            expires_ic: plan.until,
            grace_secs: plan.grace_secs,
            terms: current.terms.saturating_add(1),
            contract: id.0,
        },
        None => Licence {
            schema: LICENCE_SCHEMA,
            account: plan.account,
            holder: actor,
            issuer: plan.issuer.clone(),
            entitlement: entitlement.0,
            first_bought_ic: plan.from,
            expires_ic: plan.until,
            grace_secs: plan.grace_secs,
            terms: 1,
            contract: id.0,
        },
    };
    if let Err(error) = record.check() {
        // The `TvLease` precedent: logged, and written anyway (the contract stands).
        tracing::error!(%error, contract = id.0, "tacsim: +licence/buy wrote a licence that fails its own check");
    }
    put_licence(ctx, &record);
    // f. The reply; the economy's next sweep settles the fee.
    let line = if plan.extends {
        LICENCE_EXTENDED
            .replace("{issuer}", &plan.issuer_name)
            .replace("{until}", &ic_stamp(plan.until))
            .replace("{fee}", &fee)
    } else {
        LICENCE_BOUGHT
            .replace("{issuer}", &plan.issuer_name)
            .replace("{until}", &ic_stamp(plan.until))
            .replace("{fee}", &fee)
            .replace("{account}", &entitlement.0.to_string())
    };
    say(ctx, line);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(t0: i64) -> Licence {
        Licence {
            schema: LICENCE_SCHEMA,
            account: mush_core::AccountId::mint("pilot", 1).expect("an account"),
            holder: ObjId(42),
            issuer: CanonicalId::new("freeport_registry").expect("id"),
            entitlement: 9,
            first_bought_ic: t0,
            expires_ic: t0 + 168 * IC_DAY,
            grace_secs: 3 * IC_DAY,
            terms: 1,
            contract: 11,
        }
    }

    /// Lead ruling 5: in force to `t0 + 168 d`, grace to `t0 + 171 d`, lapsed from
    /// `t0 + 171 d`; a frozen entitlement reads suspended before any date.
    #[test]
    fn sm5_standing_runs_in_force_then_grace_then_lapsed() {
        let t0 = 1_000_000;
        let r = record(t0);
        let expires = t0 + 168 * IC_DAY;
        let lapses = t0 + 171 * IC_DAY;
        assert_eq!(
            standing_at(&r, true, t0),
            LicenceStanding::InForce {
                expires_ic: expires,
                lapses_ic: lapses
            }
        );
        assert_eq!(
            standing_at(&r, true, expires - 1),
            LicenceStanding::InForce {
                expires_ic: expires,
                lapses_ic: lapses
            }
        );
        assert_eq!(
            standing_at(&r, true, expires),
            LicenceStanding::Grace {
                expired_ic: expires,
                lapses_ic: lapses
            }
        );
        assert_eq!(
            standing_at(&r, true, lapses - 1),
            LicenceStanding::Grace {
                expired_ic: expires,
                lapses_ic: lapses
            }
        );
        assert_eq!(
            standing_at(&r, true, lapses),
            LicenceStanding::Lapsed { at_ic: lapses }
        );
        assert_eq!(
            standing_at(&r, true, t0 + 172 * IC_DAY),
            LicenceStanding::Lapsed { at_ic: lapses }
        );
        // Suspended first, whatever the date.
        for now in [t0, expires, lapses] {
            assert_eq!(standing_at(&r, false, now), LicenceStanding::Suspended);
        }
    }

    /// Reviewer F1 and doc 20 §9.1: Public, one party, one exact payout, the bare
    /// `Before` row and its deadline, forfeit to the Registry on expiry.
    #[test]
    fn sm5_the_licence_spec_is_a_public_one_row_contract() {
        let registry = PrincipalId::Organization(OrgId(5));
        let fee = Amount::from_minor(250_000);
        let spec = licence_spec(AccountId(3), &registry, AccountId(4), ObjId(42), fee, 1_000);
        assert_eq!(spec.visibility, EconomicVisibility::Public);
        assert_eq!(spec.parties, vec![registry.clone()]);
        assert_eq!(spec.considerations.len(), 1);
        assert_eq!(spec.considerations[0].account, AccountId(3));
        assert_eq!(spec.considerations[0].amount, fee);
        assert_eq!(spec.outcomes.len(), 1);
        let outcome = &spec.outcomes[0];
        assert_eq!(outcome.when, vec![0]);
        assert_eq!(outcome.payouts.len(), 1);
        assert_eq!(outcome.payouts[0].to, AccountId(4));
        assert_eq!(outcome.payouts[0].share, Share::Exact(fee));
        assert!(outcome.asset_consequences.is_empty());
        assert_eq!(spec.remainder_to, AccountId(3));
        let deadline = EconomicDeadline {
            clock: EconomicClock::RealUtc,
            at: 1_000 + LICENCE_SETTLE_WINDOW_SECS,
        };
        assert_eq!(spec.conditions, vec![TypedCondition::Before { deadline }]);
        assert_eq!(spec.evidence, vec![EvidenceKind::Before]);
        assert_eq!(spec.deadline, Some(deadline));
        assert_eq!(spec.on_expiry, Expiry::Forfeit { to: AccountId(4) });
        assert_eq!(spec.roles.author, Some(PrincipalId::Object(ObjId(42))));
        assert_eq!(spec.roles.payer, Some(PrincipalId::Object(ObjId(42))));
        assert!(spec.deterministic);
        assert!(spec.deliverables.is_empty());
        assert!(spec.scope.is_none());
    }

    /// The record's own bounds, and its strict wire.
    #[test]
    fn sm5_a_record_checks_its_own_bounds() {
        let good = record(0);
        assert_eq!(good.check(), Ok(()));
        assert_eq!(good.lapses_ic(), Some(171 * IC_DAY));
        let mut r = good.clone();
        r.schema = 2;
        assert!(r.check().is_err());
        let mut r = good.clone();
        r.terms = 0;
        assert!(r.check().is_err());
        let mut r = good.clone();
        r.entitlement = 0;
        assert!(r.check().is_err());
        let mut r = good.clone();
        r.contract = 0;
        assert!(r.check().is_err());
        let mut r = good.clone();
        r.expires_ic = r.first_bought_ic;
        assert!(r.check().is_err());
        let mut r = good.clone();
        r.grace_secs = -1;
        assert!(r.check().is_err());
        let mut r = good.clone();
        r.expires_ic = i64::MAX;
        assert!(r.check().is_err(), "the lapse overflows");
        // Strict on the wire.
        let mut value = serde_json::to_value(&good).unwrap_or_default();
        if let Some(map) = value.as_object_mut() {
            map.insert("extra".to_owned(), serde_json::json!(1));
        }
        assert!(serde_json::from_value::<Licence>(value).is_err());
        let round = serde_json::to_value(&good)
            .ok()
            .and_then(|v| serde_json::from_value::<Licence>(v).ok());
        assert_eq!(round, Some(good.clone()));
        // The key is the account's.
        assert!(licence_key(good.account).starts_with(LICENCE_PREFIX));
        assert_ne!(
            licence_key(good.account),
            licence_key(mush_core::AccountId::mint("other", 1).expect("an account"))
        );
    }

    /// Reviewer F9: doc 20 §5's `MM-DD HH:MM`, read from `CivilDate`.
    #[test]
    fn sm5_the_ic_stamp_is_month_day_hour_minute() {
        for at in [
            0_i64,
            74_152_540_800 + 3_600 * 14 + 60 * 5,
            86_399,
            -1,
            1_234_567_890,
        ] {
            let date = mush_core::CivilDate::from_ic_seconds(at);
            assert_eq!(
                ic_stamp(at),
                format!(
                    "{:02}-{:02} {:02}:{:02}",
                    date.month, date.day, date.hour, date.minute
                ),
                "{at}"
            );
        }
        assert_eq!(ic_stamp(0), "01-01 00:00");
        assert_eq!(ic_stamp(IC_DAY + 14 * 3_600), "01-02 14:00");
        assert_eq!(days_left(0, IC_DAY), 1);
        assert_eq!(days_left(0, IC_DAY + 1), 2);
        assert_eq!(days_left(IC_DAY, 0), 0);
    }

    /// The instrument constant is a valid id.
    #[test]
    fn sm5_the_licence_instrument_is_an_id() {
        assert_eq!(
            crate::economy::licence_instrument().map(|id| id.to_string()),
            Some(crate::economy::LICENCE_FREEPORT.to_owned())
        );
    }
}
