//! Doc 20 cut SM.6 — the industry seams a game layer composes at the root
//! (§3.4, D-20-5, D-20-12; 10b §3.3's cut-3 handoffs).
//!
//! Every trait here is **declared** in this crate and **implemented** by the
//! composition root — `crate::workorder::RecipeAuthority`'s shape, and for the
//! same reason: this crate is neutral and has no edge to the game that works a
//! surface block. Unwired, each seam is fail-closed or a no-op:
//!
//! * no [`SystemOpener`]: `DepositOpen` stays staff-only, as before;
//! * no [`ExternalDraws`]: a deposit with `external` set is quarantined
//!   `ExternalUnwired` at every sweep (nothing drains a block pool behind the
//!   provider's back, and nobody can work it);
//! * no [`JurisdictionAuthority`]: the default [`ActsForJurisdiction`], which a
//!   deposit with no recorded jurisdiction never matches.

use std::sync::{Arc, OnceLock};

use mush_core::{Engine, ModuleCtx, ObjId};
use mush_inventory::EndpointRef;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::families::IndustryFamilies;
use crate::record::{ClaimId, CurvePoint, OreDeposit};

/// The composition root's cell for this crate's handles
/// (`mush_inventory::InventoryHandleCell`'s shape): filled by
/// `IndustryModule::register` right after the families are built, read by
/// providers composed before it registers (a [`SystemOpener`] composed onto this
/// module) or after it (the game's port tick and its site-work provider). It
/// carries a **capability**, not authority: writing an industry family still
/// needs this owner's transaction or a delegation
/// (`IndustryModule::with_state_delegates`).
pub type IndustryHandleCell = Arc<OnceLock<IndustryFamilies>>;

/// One server-authored opening a system opener may make (doc 20 §3.4,
/// D-20-5): everything `OreDeposit::new` needs, plus the provider that works
/// the body and the issuer whose jurisdiction it lies in. The deposit's id is
/// derived from `receipt` alone (`derive_system_deposit_id`), so a second
/// opening of one receipt refuses whoever submits it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemOpening {
    /// The replay guard and id source: `1..=96` bytes of `[a-z0-9_./-]`.
    pub receipt: SmolStr,
    /// The place the body sits at.
    pub place: ObjId,
    /// What comes out of it.
    pub commodity: mush_core::CanonicalId,
    /// Its grade (cut 2's `Grade`, checked at the prepare).
    pub grade: u8,
    /// Its accessibility, `0..=100`.
    pub accessibility: u8,
    /// The recorded grade band a survey copies verbatim.
    pub grade_class: String,
    /// The recorded reserve band a survey copies verbatim.
    pub reserve_class: String,
    /// The depletion curve.
    pub curve: Vec<CurvePoint>,
    /// The endpoint its pool lives at.
    pub endpoint: EndpointRef,
    /// The reserve credited to that pool. **Staff-only**: never in an outcome.
    pub reserve: u64,
    /// The provider that works it (`OreDeposit::external`).
    pub external: SmolStr,
    /// The issuer whose jurisdiction it lies in (`OreDeposit::jurisdiction`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<PrincipalId>,
}

/// A composed admission of a non-staff actor to `DepositOpen` (doc 20 §3.4): the
/// ONE opening `actor` makes next, server-authored, in the opener's own order;
/// `None` when `actor` is no opener or has nothing left to open. Called from
/// `project_offer` only (an `&Engine` and no transaction). One a call, because
/// the opener acts once a fire and a whole site's openings in one offer could
/// pass `MAX_ACTION_PARAMETER_BYTES` (reviewer F5).
pub trait SystemOpener: Send + Sync {
    /// The next opening `actor` may make, if any.
    fn next_opening(&self, engine: &Engine, actor: ObjId) -> Option<SystemOpening>;
}

/// The composed answer to "how much has the external provider drawn from this
/// deposit" (doc 20 §3.4; 10b §3.3 handoff). The sweep checks
/// `initial_reserve − pool == drawn` for every deposit whose `external` is set,
/// and quarantines the deposit's whole block on a mismatch. `None` = cannot
/// answer (fail closed: quarantine).
///
/// It takes a [`ModuleCtx`], not a `&World` (cut SM.6 D11): a typed family is
/// readable only through a module context, owner- or delegation-gated, and the
/// provider's own record of what it drew is one.
pub trait ExternalDraws: Send + Sync {
    /// What the external provider has drawn from `deposit`'s pool, in units.
    fn drawn(&self, ctx: &ModuleCtx<'_>, deposit: &OreDeposit) -> Option<u64>;
}

/// The contest offer's jurisdiction row (doc 20 §9.6, D-20-12): the claims,
/// beyond those whose issuer `actor` acts for, whose contest state `actor` may
/// move. Composed at the root; the default is [`ActsForJurisdiction`]. SM.10
/// composes its own to carry the Registrar's rule inside a player's own renewal
/// ("carried inside the player's own acts"). Bounded like
/// `provider::administrable_claims`: at most `MAX_DEPOSIT_CHOICES`, sorted.
pub trait JurisdictionAuthority: Send + Sync {
    /// The claims `actor` may move the contest state of by jurisdiction.
    fn claims(&self, engine: &Engine, families: &IndustryFamilies, actor: ObjId) -> Vec<ClaimId>;
}

/// The default row: every claim over a `Deposit` whose `jurisdiction()` `actor`
/// `acts_for` — so an issuer may suspend a foreign claim at its own sites and
/// nowhere else (`provider::jurisdiction_claims`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ActsForJurisdiction;

impl JurisdictionAuthority for ActsForJurisdiction {
    fn claims(&self, engine: &Engine, families: &IndustryFamilies, actor: ObjId) -> Vec<ClaimId> {
        crate::provider::jurisdiction_claims(engine, families, actor)
    }
}
