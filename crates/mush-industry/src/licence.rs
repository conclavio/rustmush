//! E7c3 B5 — the extraction-rights predicate (OPEN 3, Q1A/Q14A, D-E7-7).
//!
//! **There is no `ExtractionRight` record.** 10b §3.3 names none: it gives
//! tenure (the claim) and authorization (the licence entitlement) and nothing
//! between, and cut 3 builds nothing between either. What it builds is one
//! **pure predicate over live state**, conjoining three things:
//!
//! 1. a live, unexpired, unsuspended, unseized claim over the body whose holder
//!    authorizes the actor;
//! 2. the acting principal's own `Active` account in the extraction
//!    `InstrumentKind::Entitlement` instrument — the licence, checked **at
//!    transaction time** against the *acting* principal, never against the
//!    claim holder;
//! 3. the body not depleted and not unworkable.
//!
//! **Cut 4's work order re-runs this exact function** (§3.3's "hands to cut 4"),
//! and an operator agreement that hands the right to a third party is an
//! ordinary E4b2 contract, not a new record.
//!
//! # A licence is not tenure
//!
//! The two are never conflated. A claim past its `term` stops authorizing and
//! **does not vanish**; an expired licence refuses at transaction time though
//! the claim stands. An **org-held** licence authorizes a member acting on the
//! **org's scope**, not the member personally — which is E3 invariant 18's
//! `scope`/`OrgScopeId` rule, reused rather than restated.
//!
//! # One message, three reasons
//!
//! Every refusal below renders as **one** sentence to a principal who may not
//! know the reserve, and only staff get the reason (08c §6.2's D1 shape). That
//! is why this returns a typed code the renderer collapses rather than a
//! string.

use mush_core::{ClockStamp, ObjId, World};
use mush_economy::{AccountStatus, EconomyStore, InstrumentId, InstrumentKind, acts_for};
use mush_org::PrincipalId;

use crate::record::{ClaimId, ClaimRecord, ContestState, OreDeposit};

/// Why an extraction is not authorized. **Typed, so the renderer can collapse
/// every arm into one sentence for a non-staff viewer.**
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtractionRefusal {
    /// No live claim over the body authorizes this actor.
    NoClaim,
    /// A claim exists but is past its `term`.
    ClaimExpired,
    /// A claim exists but is suspended.
    ClaimSuspended,
    /// A claim exists but has been seized.
    ClaimSeized,
    /// A `term` minted on another clock generation. **Refuses comparison
    /// rather than expiring early** (K4a `ClockStamp::checked_cmp`).
    ClaimUncomparable,
    /// The acting principal holds no licence account.
    NoLicence,
    /// The licence account is not `Active`.
    LicenceInactive,
    /// The server names no extraction-licence instrument at all.
    NoLicenceInstrument,
    /// The body is collapsed or suspended.
    Unworkable,
    /// The body is exhausted.
    Depleted,
}

impl ExtractionRefusal {
    /// The one line every non-staff principal sees, whatever the reason.
    ///
    /// Depletion, a missing licence and a suspended claim answer **the same
    /// sentence**: a principal who may not know the reserve must not learn it
    /// from the shape of a refusal.
    #[must_use]
    pub const fn public(self) -> &'static str {
        "You cannot work that here."
    }

    /// The staff-only reason.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::NoClaim => "no live claim over that body authorizes this actor",
            Self::ClaimExpired => "the claim is past its term",
            Self::ClaimSuspended => "the claim is suspended",
            Self::ClaimSeized => "the claim has been seized",
            Self::ClaimUncomparable => {
                "the claim's term is stamped on another clock generation, so it cannot be compared"
            }
            Self::NoLicence => "the acting principal holds no extraction licence",
            Self::LicenceInactive => "the acting principal's extraction licence is not active",
            Self::NoLicenceInstrument => {
                "this server names no extraction-licence instrument; wire one with \
                 IndustryModule::with_extraction_licence"
            }
            Self::Unworkable => "that body is collapsed or suspended",
            Self::Depleted => "that body is exhausted",
        }
    }
}

/// What an authorized extraction rests on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Authorized {
    /// The claim that carries the tenure, and the revision it was read at.
    pub claim: ClaimId,
    /// That claim's revision, so a re-check can refuse a stale one.
    pub claim_revision: u64,
}

/// The acting principal's own `Active` account in one instrument.
///
/// The derivation is `mush-market`'s `principal_account` (the established
/// cross-crate read precedent), re-derived here rather than reached for,
/// because cut 3 adds no `mush-market` edge: a person's wallet is their
/// **unscoped** one, an organization's is always scoped, and "lowest id first"
/// comes from the snapshot's `BTreeSet<AccountId>` values rather than from luck.
#[must_use]
pub fn licence_accounts(
    world: &World,
    instrument: &InstrumentId,
    who: &PrincipalId,
) -> Option<AccountStatus> {
    let personal = matches!(who, PrincipalId::Object(_));
    world
        .module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(who)
        .find(|account| account.instrument == *instrument && (!personal || account.scope.is_none()))
        .map(|account| account.status)
}

/// Whether the named instrument is an entitlement at all.
///
/// A currency account is not a licence, and this is the check that keeps a
/// mis-wired composition from turning a wallet into one.
#[must_use]
pub fn is_entitlement(world: &World, instrument: &InstrumentId) -> bool {
    world
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().instrument(instrument).cloned())
        .is_some_and(|def| def.kind == InstrumentKind::Entitlement)
}

/// The one predicate. Pure over the state handed to it, so it is trivially the
/// same function in a command, in `project_offer` and in cut 4's work order.
///
/// `claims` is the caller's **bounded** page of live claims over the body: the
/// predicate never scans, because a predicate that scans is a predicate whose
/// cost depends on the world rather than on the caller.
///
/// # Errors
/// [`ExtractionRefusal`] — one typed code, collapsed to one sentence for a
/// non-staff viewer.
#[allow(clippy::too_many_arguments)]
pub fn extraction_authorized(
    world: &World,
    actor: ObjId,
    operator: &PrincipalId,
    deposit: &OreDeposit,
    remaining: u64,
    claims: &[ClaimRecord],
    licence_instrument: Option<&InstrumentId>,
    now: &ClockStamp,
) -> Result<Authorized, ExtractionRefusal> {
    // (c) the body first: it is the cheapest test and it never depends on who
    // is asking, so it cannot become an oracle about anyone else.
    if !deposit.accessibility().is_workable() {
        return Err(ExtractionRefusal::Unworkable);
    }
    if remaining == 0 || deposit.next_yield(remaining) == 0 {
        return Err(ExtractionRefusal::Depleted);
    }

    // (a) tenure. The *best* refusal is kept so staff see the most specific
    // reason, while every one of them renders identically to a player.
    let mut best: Option<ExtractionRefusal> = None;
    let mut authorized: Option<Authorized> = None;
    for claim in claims {
        if !claim.contest_state().authorizes() {
            let refusal = match claim.contest_state() {
                ContestState::Suspended { .. } => ExtractionRefusal::ClaimSuspended,
                ContestState::Seized { .. } => ExtractionRefusal::ClaimSeized,
                ContestState::Uncontested | ContestState::Contested { .. } => {
                    ExtractionRefusal::NoClaim
                }
            };
            best = Some(best.unwrap_or(refusal));
            continue;
        }
        // K4a: a stamp on another clock generation **refuses comparison rather
        // than expiring early**. Never `<`, never `partial_cmp`.
        match claim.term().checked_cmp(now) {
            Ok(std::cmp::Ordering::Less) => {
                best = Some(best.unwrap_or(ExtractionRefusal::ClaimExpired));
                continue;
            }
            Err(_) => {
                best = Some(best.unwrap_or(ExtractionRefusal::ClaimUncomparable));
                continue;
            }
            Ok(_) => {}
        }
        // The holder must authorize this actor. An org-held claim authorizes a
        // member acting **on the org's scope** — `acts_for` is E3's own rule,
        // not a second one.
        if claim.holder() != operator && !acts_for(world, actor, claim.holder()) {
            best = Some(best.unwrap_or(ExtractionRefusal::NoClaim));
            continue;
        }
        if claim.holder() == operator || acts_for(world, actor, operator) {
            authorized = Some(Authorized {
                claim: claim.id(),
                claim_revision: claim.revision(),
            });
            break;
        }
        best = Some(best.unwrap_or(ExtractionRefusal::NoClaim));
    }
    let authorized = authorized.ok_or(best.unwrap_or(ExtractionRefusal::NoClaim))?;

    // (b) the licence — the ACTING principal's, at transaction time.
    let instrument = licence_instrument.ok_or(ExtractionRefusal::NoLicenceInstrument)?;
    if !is_entitlement(world, instrument) {
        return Err(ExtractionRefusal::NoLicenceInstrument);
    }
    match licence_accounts(world, instrument, operator) {
        None => Err(ExtractionRefusal::NoLicence),
        Some(AccountStatus::Active) => Ok(authorized),
        Some(_) => Err(ExtractionRefusal::LicenceInactive),
    }
}
