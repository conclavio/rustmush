//! **Declare and activate in one step, for a compiled owner** (doc 50 cut OP.0; Q-50-57 item 1,
//! Q-50-19).
//!
//! Until this cut only the staff verbs could declare (`@op/declare`, `@op/terms`,
//! `@op/activate`), and `CampaignStore::mint_operation` was the crate's own. A compiled arm (doc
//! 50's declaring arm, OP.5) has no actor at a keyboard: it names the declarer, the terms and the
//! bond, and [`declare_and_activate`] mints the operation, places the bond and freezes the terms in
//! one call. The operation is born `Activated`, so it never stands `Declared` with a bond and no
//! hold, which is the shape doc 50 §4.3's "unfunded" defect paused.
//!
//! - **Who declares** is any principal: an object, or an **organization** (Q-50-57 item 2). The
//!   bond is a reservation on the declarer's own account, and the economy authorizes the post as
//!   the declarer, so an organization's bond is placed on its own live compartment and nothing
//!   else.
//! - **No actor is asked.** The caller is code, and its authority is that it was composed. The
//!   per-issuer cap still binds ([`crate::MAX_OPERATIONS_PER_ISSUER`]).
//! - **The notice runs from now**: the window opens `notice_secs` after this call and lasts
//!   `window_secs`. The notice band ([`crate::OPERATION_NOTICE_MIN`] to
//!   [`crate::OPERATION_NOTICE_MAX`]) binds exactly as it binds `@op/terms`.
//! - **The tick tells the defenders** on its next fire, as it tells them after `@op/activate`.

use mush_core::Ctx;
use mush_economy::{AccountId, Amount, EconomicVisibility};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::bond::BondError;
use crate::ids::OperationId;
use crate::record::{
    AudienceSpec, Bond, Consequence, ObjectiveSpec, OperationDoc, OperationRuntime, OperationState,
    OperationTerms, SideSpec, TermsError,
};
use crate::store::{CampaignStore, persist_meta, put_operation};
use crate::terms::operation_consent_hash;

/// **The bond a compiled declaration reserves**: `amount` on the declarer's own `account`, and
/// who takes it when the declarer fails (`forfeit_to`; doc 50 Q-50-57 item 5, provisional). With
/// no beneficiary the bond is released on every outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondTerms {
    /// The declarer's own account.
    pub account: AccountId,
    /// How much is reserved (positive).
    pub amount: Amount,
    /// Who takes the bond on the declarer's failure, and into which account: for a compiled
    /// declarer, the defenders' treasury.
    pub forfeit_to: Option<(PrincipalId, AccountId)>,
}

/// **Everything a compiled owner declares** ([`declare_and_activate`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// Who declares: an object or an organization.
    pub declarer: PrincipalId,
    /// What is evaluated, and its one-line summary.
    pub objective: ObjectiveSpec,
    /// The legal-target classes.
    pub legal_targets: Vec<SmolStr>,
    /// What the resolution causes.
    pub stakes: Vec<Consequence>,
    /// The bond, if any.
    pub bond: Option<BondTerms>,
    /// The sides.
    pub sides: Vec<SideSpec>,
    /// Who may defend; an organization's notice reaches its members.
    pub eligible_defenders: AudienceSpec,
    /// IC seconds from now to the window's open.
    pub notice_secs: i64,
    /// IC seconds the window runs.
    pub window_secs: i64,
    /// Who may see it.
    pub visibility: EconomicVisibility,
}

/// **Why a compiled declaration was refused.** Nothing is written but the allocator, which never
/// reissues an id it minted.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeclareError {
    /// The campaign module is not composed.
    #[error("the campaign is not composed")]
    NoCampaign,
    /// The declarer already runs as many live operations as one issuer may.
    #[error("the declarer already runs {live} live operations, the limit")]
    IssuerCap {
        /// How many.
        live: usize,
    },
    /// The world carries as many operations as it can hold.
    #[error("the world carries as many operations as it can hold")]
    Full,
    /// The window does not last.
    #[error("the window must last at least one second")]
    Window,
    /// The terms refused their own constructor.
    #[error("the terms are refused: {0}")]
    Terms(TermsError),
    /// The bond's accounts or the outcome stakes are refused.
    #[error("{0}")]
    Refused(String),
    /// The economy would not place the bond (an unfunded account among them).
    #[error("the bond could not be placed: {0}")]
    Bond(String),
}

/// **Declare an operation and activate it in one step** (doc 50 cut OP.0; this module's head).
/// Answers the new operation's id.
///
/// # Errors
///
/// [`DeclareError`]: the campaign absent, the issuer's cap or the world's reached, the terms or
/// their accounts refused, or the bond not placed. On every refusal the operation is not
/// written.
pub fn declare_and_activate(
    ctx: &mut Ctx<'_>,
    declaration: Declaration,
) -> Result<OperationId, DeclareError> {
    let live = ctx
        .w
        .module_state::<CampaignStore>()
        .ok_or(DeclareError::NoCampaign)?
        .live_operations_for(&declaration.declarer);
    if live >= crate::store::MAX_OPERATIONS_PER_ISSUER {
        return Err(DeclareError::IssuerCap { live });
    }
    if declaration.window_secs < 1 {
        return Err(DeclareError::Window);
    }
    let id = ctx
        .w
        .module_state_mut::<CampaignStore>()
        .and_then(CampaignStore::mint_operation)
        .ok_or(DeclareError::Full)?;
    // The allocator moved; keep it on disk whatever follows, so the id is never reissued.
    persist_meta(ctx);
    let now = ctx.w.world_timeline().stamp_at(ctx.w.now());
    let mut open = now.clone();
    open.at = now.at.saturating_add(declaration.notice_secs);
    let mut close = open.clone();
    close.at = open.at.saturating_add(declaration.window_secs);
    let mut terms = OperationTerms {
        operation: id,
        attacker: declaration.declarer,
        objective: declaration.objective,
        legal_targets: declaration.legal_targets,
        stakes: declaration.stakes,
        bond: declaration.bond.map(|b| Bond {
            account: b.account,
            amount: b.amount,
            forfeit_to: b.forfeit_to,
            hold: None,
        }),
        sides: declaration.sides,
        eligible_defenders: declaration.eligible_defenders,
        notice: now,
        window: (open, close),
        visibility: declaration.visibility,
        consent_hash: [0; 32],
    };
    terms.consent_hash = operation_consent_hash(&terms);
    terms.check().map_err(DeclareError::Terms)?;
    if let Some(why) = crate::commands::bond_accounts_refusal(ctx.w, &terms, false) {
        return Err(DeclareError::Refused(why.to_owned()));
    }
    if let Some(why) = crate::commands::outcome_stakes_refusal(ctx.w, &terms) {
        return Err(DeclareError::Refused(why));
    }
    // The hold first, and only a placed hold writes the operation: the never-negative argument
    // `@op/activate` makes, in the same order.
    if let Some(bond) = terms.bond.clone() {
        let hold = crate::bond::place(ctx, &terms, &bond).map_err(|error| match error {
            BondError::Refused(reason) => DeclareError::Bond(reason),
            other => DeclareError::Bond(other.to_string()),
        })?;
        if let Some(slot) = terms.bond.as_mut() {
            slot.hold = Some(hold);
        }
    }
    let mut runtime = OperationRuntime::declared(id);
    runtime.state = OperationState::Activated;
    runtime.revision = runtime.revision.saturating_add(1);
    put_operation(ctx, OperationDoc { terms, runtime });
    tracing::info!(
        target: "campaign",
        operation = id.0,
        "campaign: an operation was declared and activated by a compiled owner"
    );
    Ok(id)
}
