//! **The sale of a stored managed core with its title** (doc 47 unit CO.13; R-47-5's "or sold").
//! The one managed path that pays a core's holder and re-titles the core to a buyer: the
//! seizure's shape (`seizure`) with the charter transfer's two money lines
//! (`charter_transfer`). A closed plan whose guard (`ClosedCoreSalePlan`) admits exactly the
//! before-title image, the exact core `TitleBirth`, the two configured money lines and one
//! `TransferTitle` to the configured buy-back vendor, and nothing else.
//!
//! * **Who buys** is composition's, never the caller's: `CoreSaleConfig::vendor`, the fitting
//!   shop's integrator (its organization, scope and account). The request names only the core,
//!   the title revision it read, the seller's payout account and the price.
//! * **What the producer proves** in the same coordinator union: the registered tacsim producer
//!   (`tacsim::minds::core_sale`) reads the core in the seller's own hands, not fitted and not
//!   destroyed, prices it from the market's core catalogue at the buy-back rate, and moves it
//!   into the shop through the kernel's closed sale image (`mush_core::core_sale`). This leg
//!   owns the money and the title; the kernel owns the object.
//!
//! No public post, staff bit or serialized plan selects this guard; the participant's prepare is
//! reachable only inside the coordinator union (`OwnerPrepareContext`), from the exact composed
//! action. `managed_title::require_unmanaged_mutations` is untouched: every generic entry point
//! still refuses a `TransferTitle` on a managed title.
use super::*;
use crate::core_integration::ScopedCoreAccount;
use crate::record::{AccountStatus, AssetLifecycle, InstrumentStatus};
use mush_core::DefinitionRef;

mod install;
pub(crate) mod receipt;
pub use install::{BoundCoreSale, CoreSalePatch};
pub use receipt::{CoreSaleAuditSubject, CoreSaleReceiptAudit};

/// The sale action's id (tacsim-owned: the seller's verb at the fitting shop).
pub const ACTION_ID: &str = "sell_stored_core";
/// The owner of the action.
pub const ACTION_OWNER: &str = "tacsim";

/// **What composition freezes** for the closed sale: the exact action and the buy-back vendor
/// (the fitting shop's integrator). None of it is caller input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreSaleConfig {
    pub action: DefinitionRef,
    pub vendor: ScopedCoreAccount,
}
impl CoreSaleConfig {
    /// # Errors
    /// An action that is not the exact tacsim action of [`ACTION_ID`] at version 1.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.action.key().owner.as_str() != ACTION_OWNER
            || self.action.key().kind.as_str() != "action"
            || self.action.key().id.as_str() != ACTION_ID
            || self.action.version() != 1
        {
            return Err("core sale requires the exact tacsim action");
        }
        Ok(())
    }
    fn vendor(&self) -> PrincipalId {
        PrincipalId::Organization(self.vendor.organization)
    }
}

/// The closed request the producer constructs: the core, the title revision it read, the
/// seller's own payout account and the price it computed. The seller is the invocation's
/// actor. Not a wire format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryCoreSale {
    pub core: ObjId,
    pub title_revision: u64,
    pub seller_account: AccountId,
    pub price: Amount,
}

/// The owner output the producer reads back: the action and the exact receipt.
pub struct CoreSaleOutput {
    action: DefinitionRef,
    subject: CoreSaleAuditSubject,
}
impl CoreSaleOutput {
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub fn subject(&self) -> &CoreSaleAuditSubject {
        &self.subject
    }
    pub const fn commit_token(&self) -> CommitToken {
        self.subject.commit
    }
}

fn require_action(
    config: &CoreSaleConfig,
    c: &OwnerPrepareContext<'_>,
) -> Result<(), ParticipantError> {
    let actor = c.invocation().actor();
    if config.validate().is_err()
        || c.action() != &config.action
        || c.invocation().principal() != &(mush_core::action::PrincipalId::Object { id: actor })
        || c.invocation().subject() != &mush_core::action::PerceivedRef::SelfActor
        || !c.invocation().targets().is_empty()
        || c.world().get(actor).is_none_or(|o| {
            o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
        })
    {
        return Err(refused());
    }
    Ok(())
}

/// **The closed op shape**: exactly one `TransferTitle` of the core to the vendor org, in no
/// scope.
pub(crate) fn ops(core: ObjId, vendor: &PrincipalId) -> Vec<AssetOp> {
    vec![AssetOp::TransferTitle {
        object: core,
        to: vendor.clone(),
        to_scope: None,
    }]
}
/// **The closed money lines**: the vendor pays the price, the seller receives it.
pub(crate) fn lines(config: &CoreSaleConfig, r: &RegistryCoreSale) -> Vec<RequestedLine> {
    vec![
        RequestedLine {
            account: config.vendor.account,
            delta: Amount::from_minor(-r.price.minor()),
            memo: "stored core buy-back".into(),
        },
        RequestedLine {
            account: r.seller_account,
            delta: r.price,
            memo: "stored core sale".into(),
        },
    ]
}

/// The before-title, after every precondition held: a live managed core titled to the seller
/// alone (no scope, no claim, not frozen, not stolen, active), the seller's own active payout
/// account and the vendor's configured one in the same active instrument.
fn title<'a>(
    config: &CoreSaleConfig,
    policy: &InitialFilingPolicy,
    state: &'a EconomyStore,
    world: &World,
    seller: ObjId,
    r: &RegistryCoreSale,
) -> Result<&'a EconomicAsset, ParticipantError> {
    if state.frozen
        || r.title_revision == 0
        || r.title_revision == u64::MAX
        || r.price.minor() <= 0
        || r.seller_account == config.vendor.account
        || !state.is_title_issuer(&policy.issuer)
        || state.snapshot.managed_titles.core(r.core).is_none()
        || world
            .get(r.core)
            .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
    {
        return Err(refused());
    }
    let asset = state.snapshot.asset(r.core).ok_or_else(refused)?;
    if asset.object != r.core
        || asset.revision != r.title_revision
        || asset.title_owner != PrincipalId::Object(seller)
        || asset.scope.is_some()
        || asset.frozen
        || asset.stolen.is_some()
        || asset.lifecycle != AssetLifecycle::Active
        || !asset.encumbrances.is_empty()
        || state.fold.seizure_receipts.held(r.core).is_some()
    {
        return Err(refused());
    }
    let payer = state
        .snapshot
        .account(config.vendor.account)
        .ok_or_else(refused)?;
    let payee = state
        .snapshot
        .account(r.seller_account)
        .ok_or_else(refused)?;
    if payer.owner != config.vendor()
        || payer.scope != Some(config.vendor.scope)
        || payee.owner != PrincipalId::Object(seller)
        || payee.scope.is_some()
        || payer.status != AccountStatus::Active
        || payee.status != AccountStatus::Active
        || payer.instrument != payee.instrument
        || state
            .snapshot
            .instrument(&payer.instrument)
            .is_none_or(|i| i.status != InstrumentStatus::Active)
    {
        return Err(refused());
    }
    Ok(asset)
}

struct SaleAuth {
    issuer: PrincipalId,
    vendor: PrincipalId,
    seller: ObjId,
    lines: Vec<RequestedLine>,
    ops: Vec<AssetOp>,
    core: ObjId,
    title_revision: u64,
}
impl AuthEval for SaleAuth {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        actor == &self.issuer
            && self
                .lines
                .iter()
                .any(|line| line.account == account.id && line.delta == delta)
            && (account.owner == self.vendor || account.owner == PrincipalId::Object(self.seller))
    }
    fn authorize_asset(
        &self,
        actor: &PrincipalId,
        asset: Option<&EconomicAsset>,
        op: &AssetOp,
    ) -> bool {
        actor == &self.issuer
            && self.ops.contains(op)
            && asset.is_some_and(|a| {
                a.object == self.core
                    && a.revision == self.title_revision
                    && a.title_owner == PrincipalId::Object(self.seller)
            })
    }
    fn authorize_title_receipt(
        &self,
        actor: &PrincipalId,
        to: &PrincipalId,
        scope: Option<OrgScopeId>,
    ) -> bool {
        actor == &self.issuer && to == &self.vendor && scope.is_none()
    }
}

fn plan(
    config: &CoreSaleConfig,
    policy: &InitialFilingPolicy,
    (seller, commit, at_ic): (ObjId, CommitToken, i64),
    r: &RegistryCoreSale,
) -> EconomyPlan {
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(receipt::NAMESPACE, commit.to_hex()),
        PrincipalId::System(policy.issuer.clone()),
        EconomicCause::Ruleset {
            namespace: receipt::NAMESPACE.into(),
            correlation: receipt::correlation((commit, seller, at_ic), &config.vendor, r).into(),
        },
        lines(config, r),
    );
    plan.description = "Sale of a stored core with its title".into();
    plan.asset_ops = ops(r.core, &config.vendor());
    plan.expected_asset_revisions.push(crate::AssetRevision {
        object: r.core,
        revision: r.title_revision,
    });
    plan
}

fn validate(
    config: &CoreSaleConfig,
    policy: &InitialFilingPolicy,
    state: &EconomyStore,
    world: &World,
    seller: ObjId,
    r: &RegistryCoreSale,
    plan: &EconomyPlan,
) -> Result<Prepared, ParticipantError> {
    let before = title(config, policy, state, world, seller, r)?;
    let auth = SaleAuth {
        issuer: PrincipalId::System(policy.issuer.clone()),
        vendor: config.vendor(),
        seller,
        lines: lines(config, r),
        ops: ops(r.core, &config.vendor()),
        core: r.core,
        title_revision: r.title_revision,
    };
    let mut spent = BTreeMap::new();
    if let Some(window) = state
        .snapshot
        .account(config.vendor.account)
        .and_then(|a| a.approval.as_ref())
        .and_then(|a| a.window.as_ref())
    {
        spent.insert(
            config.vendor.account,
            state.spent_in_window(config.vendor.account, world.now(), window.period_secs),
        );
    }
    let gate = state.key_gate();
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: world.now(),
        ic_now: world.game_now_unix(),
        next_hold_id: state.next_hold,
        spent,
        keys: &gate,
    };
    let proof = ClosedCoreSalePlan {
        plan: plan.clone(),
        before_title: before.clone(),
        birth: state
            .snapshot
            .managed_titles
            .core(r.core)
            .cloned()
            .ok_or_else(refused)?,
        vendor: config.vendor(),
        lines: lines(config, r),
    };
    match crate::engine::validate_core_sale(&proof, &state.snapshot, &env) {
        PlanResult::Post(prepared) => Ok(*prepared),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) | PlanResult::Refused(_) => {
            Err(refused())
        }
    }
}

pub(super) use install::{bind, prepare, recheck};

impl EconomyStore {
    /// **Whether `object` is a managed core** (its title born of native manufacture): the cores
    /// only the closed families (seizure, refit, this sale) may re-title. `false` for anything
    /// else, and while the managed-title index is unavailable.
    #[must_use]
    pub fn is_managed_core(&self, object: ObjId) -> bool {
        self.snapshot.managed_titles.core(object).is_some()
    }
    /// **The sale receipt of `commit`** (doc 47 CO.13): the stored-core sale that commit made,
    /// once the audit is complete this boot.
    ///
    /// # Errors
    /// While the store is frozen, or the sale audit is not complete this boot.
    pub fn core_sale_receipt(
        &self,
        commit: CommitToken,
    ) -> Result<Option<&CoreSaleAuditSubject>, ReceiptAuditError> {
        if self.frozen {
            return Err(ReceiptAuditError::Incomplete);
        }
        self.fold.core_sale_receipts.get(commit)
    }
    /// **Every stored-core sale**, in commit order (doc 47 CO.13).
    ///
    /// # Errors
    /// As [`Self::core_sale_receipt`].
    pub fn core_sale_receipts(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &CoreSaleAuditSubject>, ReceiptAuditError> {
        if self.frozen {
            return Err(ReceiptAuditError::Incomplete);
        }
        self.fold.core_sale_receipts.receipts()
    }
}

/// **The private closed plan proof**: exact plan equality, the exact before-title image, the
/// exact core `TitleBirth`, exactly the two money lines and exactly `[TransferTitle { the
/// vendor org }]`. No hold or custody op rides along. Only this module constructs it.
pub(crate) struct ClosedCoreSalePlan {
    plan: EconomyPlan,
    before_title: EconomicAsset,
    birth: crate::managed_title::TitleBirth,
    vendor: PrincipalId,
    lines: Vec<RequestedLine>,
}
impl ClosedCoreSalePlan {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        let object = self.before_title.object;
        if plan != &self.plan
            || plan.idempotency.namespace != receipt::NAMESPACE
            || !matches!(&plan.cause,EconomicCause::Ruleset{namespace,..} if namespace==receipt::NAMESPACE)
            || snapshot.asset(object) != Some(&self.before_title)
            || snapshot.managed_titles.core(object) != Some(&self.birth)
            || plan.asset_ops != ops(object, &self.vendor)
            || plan.requested_lines != self.lines
            || !plan.hold_ops.is_empty()
            || !plan.custody_ops.is_empty()
        {
            return Err(crate::EconomyError::ManagedContract);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
