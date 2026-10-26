//! Closed buyer acceptance of a genuinely retained personal charter offer.
//! The Roll producer proves the actual signed offer and complete claim inventory;
//! this native leg owns the exact money/title transition and its journal images.
//! No public post, staff bit or serialized plan may select its private guard.
use super::*;
use crate::record::{AccountStatus, AssetLifecycle, InstrumentStatus};
use mush_core::{DefinitionDigest, DefinitionRef};

mod install;
pub(crate) mod lineage;
pub(crate) mod receipt;
pub use install::{BoundCharterTransfer, CharterTransferPatch};
pub use receipt::{CharterTransferAuditSubject, CharterTransferReceiptAudit};

pub const ACTION_ID: &str = "accept_mind_charter_transfer";

/// Closed same-slot request. It is not a wire format or a reusable seller grant.
/// Only the exact registered Roll provider can reach this participant action;
/// that provider constructs it from its acknowledged offer and binds its Root
/// proof in the same coordinator union. The client supplies only offer and payer.
pub struct RegistryCharterTransfer {
    pub offer: CommitToken,
    pub offer_digest: DefinitionDigest,
    pub charter: ObjId,
    pub title_revision: u64,
    pub seller: ObjId,
    pub buyer: ObjId,
    pub payer: AccountId,
    pub seller_account: AccountId,
    pub price: Amount,
}
pub struct CharterTransferOutput {
    action: DefinitionRef,
    subject: CharterTransferAuditSubject,
}
impl CharterTransferOutput {
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub fn subject(&self) -> &CharterTransferAuditSubject {
        &self.subject
    }
    pub const fn commit_token(&self) -> CommitToken {
        self.subject.commit
    }
}

fn require_action(
    expected: &DefinitionRef,
    c: &OwnerPrepareContext<'_>,
) -> Result<(), ParticipantError> {
    let actor = c.invocation().actor();
    if c.action() != expected
        || expected.key().owner.as_str() != "roll"
        || expected.key().kind.as_str() != "action"
        || expected.key().id.as_str() != ACTION_ID
        || expected.version() != 1
        || c.invocation().principal() != &(mush_core::action::PrincipalId::Object { id: actor })
        || c.invocation().subject() != &mush_core::action::PerceivedRef::SelfActor
        || !c.invocation().targets().is_empty()
    {
        return Err(refused());
    }
    Ok(())
}

fn title<'a>(
    state: &'a EconomyStore,
    world: &World,
    actor: ObjId,
    r: &RegistryCharterTransfer,
    policy: &InitialFilingPolicy,
) -> Result<&'a EconomicAsset, ParticipantError> {
    if state.frozen
        || actor != r.buyer
        || r.seller == r.buyer
        || r.price.minor() <= 0
        || r.title_revision == 0
        || r.title_revision == u64::MAX
        || r.payer == r.seller_account
        || !state.is_title_issuer(&policy.issuer)
        || state.snapshot.managed_titles.charter(r.charter).is_none()
        || [r.seller, r.buyer].into_iter().any(|id| {
            world.get(id).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
        })
        || world
            .get(r.charter)
            .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
    {
        return Err(refused());
    }
    let asset = state.snapshot.asset(r.charter).ok_or_else(refused)?;
    if asset.object != r.charter
        || asset.revision != r.title_revision
        || asset.title_owner != PrincipalId::Object(r.seller)
        || asset.scope.is_some()
        || asset.frozen
        || asset.stolen.is_some()
        || asset.lifecycle != AssetLifecycle::Active
        || !asset.encumbrances.is_empty()
        || asset.catalog.as_ref() != Some(&policy.catalog)
    {
        return Err(refused());
    }
    let payer = state.snapshot.account(r.payer).ok_or_else(refused)?;
    let seller = state
        .snapshot
        .account(r.seller_account)
        .ok_or_else(refused)?;
    if payer.owner != PrincipalId::Object(r.buyer)
        || seller.owner != PrincipalId::Object(r.seller)
        || payer.scope.is_some()
        || seller.scope.is_some()
        || payer.status != AccountStatus::Active
        || seller.status != AccountStatus::Active
        || payer.instrument != seller.instrument
        || state
            .snapshot
            .instrument(&payer.instrument)
            .is_none_or(|i| i.status != InstrumentStatus::Active)
    {
        return Err(refused());
    }
    Ok(asset)
}

struct TransferAuth<'a> {
    world: WorldAuth<'a>,
    issuer: PrincipalId,
    request: &'a RegistryCharterTransfer,
}
impl AuthEval for TransferAuth<'_> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        actor == &self.issuer
            && ((account.id == self.request.payer
                && delta.checked_neg() == Some(self.request.price)
                && self
                    .world
                    .authorize(&PrincipalId::Object(self.request.buyer), account, delta))
                || (account.id == self.request.seller_account
                    && delta == self.request.price
                    && account.owner == PrincipalId::Object(self.request.seller)))
    }
    fn authorize_asset(
        &self,
        actor: &PrincipalId,
        asset: Option<&EconomicAsset>,
        op: &AssetOp,
    ) -> bool {
        let exact = AssetOp::TransferTitle {
            object: self.request.charter,
            to: PrincipalId::Object(self.request.buyer),
            to_scope: None,
        };
        actor == &self.issuer
            && op == &exact
            && asset.is_some_and(|a| {
                a.title_owner == PrincipalId::Object(self.request.seller)
                    && a.object == self.request.charter
                    && a.revision == self.request.title_revision
            })
            && self
                .world
                .authorize_asset(&PrincipalId::Object(self.request.seller), asset, op)
    }
    fn authorize_title_receipt(
        &self,
        actor: &PrincipalId,
        to: &PrincipalId,
        scope: Option<OrgScopeId>,
    ) -> bool {
        actor == &self.issuer
            && to == &PrincipalId::Object(self.request.buyer)
            && scope.is_none()
            && self.world.authorize_title_receipt(
                &PrincipalId::Object(self.request.buyer),
                to,
                scope,
            )
    }
}
fn plan(
    policy: &InitialFilingPolicy,
    actor: ObjId,
    commit: CommitToken,
    r: &RegistryCharterTransfer,
) -> EconomyPlan {
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(receipt::NAMESPACE, commit.to_hex()),
        PrincipalId::System(policy.issuer.clone()),
        EconomicCause::Ruleset {
            namespace: receipt::NAMESPACE.into(),
            correlation: receipt::correlation(commit, actor, r).into(),
        },
        vec![
            RequestedLine {
                account: r.payer,
                delta: Amount::from_minor(-r.price.minor()),
                memo: "charter title purchase".into(),
            },
            RequestedLine {
                account: r.seller_account,
                delta: r.price,
                memo: "charter title sale".into(),
            },
        ],
    );
    plan.description = "Consented mind charter title transfer".into();
    plan.asset_ops.push(AssetOp::TransferTitle {
        object: r.charter,
        to: PrincipalId::Object(r.buyer),
        to_scope: None,
    });
    plan.expected_asset_revisions.push(crate::AssetRevision {
        object: r.charter,
        revision: r.title_revision,
    });
    plan
}
fn validate(
    policy: &InitialFilingPolicy,
    state: &EconomyStore,
    world: &World,
    actor: ObjId,
    r: &RegistryCharterTransfer,
    plan: &EconomyPlan,
) -> Result<Prepared, ParticipantError> {
    let before = title(state, world, actor, r, policy)?.clone();
    let auth = TransferAuth {
        world: WorldAuth { w: world },
        issuer: PrincipalId::System(policy.issuer.clone()),
        request: r,
    };
    let mut spent = BTreeMap::new();
    if let Some(window) = state
        .snapshot
        .account(r.payer)
        .and_then(|a| a.approval.as_ref())
        .and_then(|a| a.window.as_ref())
    {
        spent.insert(
            r.payer,
            state.spent_in_window(r.payer, world.now(), window.period_secs),
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
    let proof = ClosedCharterTransferPlan {
        plan: plan.clone(),
        before_title: before,
        birth: state
            .snapshot
            .managed_titles
            .charter(r.charter)
            .cloned()
            .ok_or_else(refused)?,
        buyer: r.buyer,
    };
    match crate::engine::validate_charter_transfer(&proof, &state.snapshot, &env) {
        PlanResult::Post(prepared) => Ok(*prepared),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) | PlanResult::Refused(_) => {
            Err(refused())
        }
    }
}

pub(super) use install::{bind, prepare, recheck};
pub(crate) struct ClosedCharterTransferPlan {
    plan: EconomyPlan,
    before_title: EconomicAsset,
    birth: crate::managed_title::TitleBirth,
    buyer: ObjId,
}
impl ClosedCharterTransferPlan {
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
            || snapshot.managed_titles.charter(object) != Some(&self.birth)
            || plan.asset_ops.as_slice()
                != [AssetOp::TransferTitle {
                    object,
                    to: PrincipalId::Object(self.buyer),
                    to_scope: None,
                }]
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
