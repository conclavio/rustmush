//! Closed fee-only owner leg for revisions and explicit majors of an existing charter.
//! The registered Roll producer owns certification and sequence publication.
pub(crate) mod house;
mod install;
pub use house::RegistryHouseCompatibleUpdate;
use house::UpdateAuthority;
pub(crate) mod receipt;
use super::*;
use crate::record::{AccountStatus, AssetLifecycle, InstrumentStatus};
pub use install::{BoundCompatibleUpdate, CompatibleUpdatePatch};
use mush_core::mechanics::DefinitionRef;
pub use receipt::{UpdateAuditSubject, UpdateKind, UpdateReceiptAudit};

pub const ACTION_ID: &str = "update_original_mind";
pub const DERIVED_ACTION_ID: &str = "update_derived_mind";
pub const MAJOR_ACTION_ID: &str = "file_original_major";
pub const HOUSE_ACTION_ID: &str = "update_house_mind";

/// Internal same-slot request. No plan, fee, issuer, actor or postimage input.
pub struct RegistryCompatibleUpdate {
    pub charter: ObjId,
    pub title_revision: u64,
    pub payer: AccountId,
}
/// Actual prepared fee facts. Only the Economy owner constructs this value.
pub struct CompatibleUpdateOutput {
    action: DefinitionRef,
    subject: UpdateAuditSubject,
}
impl CompatibleUpdateOutput {
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub fn subject(&self) -> &UpdateAuditSubject {
        &self.subject
    }
    pub const fn commit_token(&self) -> CommitToken {
        self.subject.commit
    }
    pub const fn actor(&self) -> ObjId {
        self.subject.actor
    }
}

fn require_action(
    expected: &DefinitionRef,
    context: &OwnerPrepareContext<'_>,
) -> Result<(), ParticipantError> {
    let actor = context.invocation().actor();
    if context.action() != expected
        || expected.key().owner.as_str() != "roll"
        || expected.key().kind.as_str() != "action"
        || !matches!(
            expected.key().id.as_str(),
            ACTION_ID | DERIVED_ACTION_ID | MAJOR_ACTION_ID | HOUSE_ACTION_ID
        )
        || expected.version() != 1
        || context.invocation().principal()
            != &(mush_core::action::PrincipalId::Object { id: actor })
        || context.invocation().subject() != &mush_core::action::PerceivedRef::SelfActor
        || !context.invocation().targets().is_empty()
    {
        return Err(refused());
    }
    Ok(())
}

fn title<'a>(
    state: &'a EconomyStore,
    world: &World,
    actor: ObjId,
    request: &RegistryCompatibleUpdate,
    policy: &InitialFilingPolicy,
    authority: &UpdateAuthority,
) -> Result<&'a EconomicAsset, ParticipantError> {
    authority.require(world, state, actor, request, policy.fee)?;
    if state.frozen
        || policy.fee.minor() <= 0
        || !state.is_title_issuer(&policy.issuer)
        || request.payer == policy.sink
        || request.title_revision == 0
        || world.get(actor).is_none_or(|o| {
            o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
        })
        || world
            .get(request.charter)
            .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
    {
        return Err(refused());
    }
    let title = state.snapshot.asset(request.charter).ok_or_else(refused)?;
    if title.object != request.charter
        || title.revision != request.title_revision
        || title.title_owner != authority.owner(actor)
        || title.scope != authority.scope()
        || title.frozen
        || title.stolen.is_some()
        || title.lifecycle != AssetLifecycle::Active
        || title.catalog.as_ref() != Some(&policy.catalog)
    {
        return Err(refused());
    }
    let payer = state.snapshot.account(request.payer).ok_or_else(refused)?;
    let sink = state.snapshot.account(policy.sink).ok_or_else(refused)?;
    if payer.owner != authority.owner(actor)
        || payer.scope != authority.scope()
        || payer.status != AccountStatus::Active
        || sink.status != AccountStatus::Active
        || payer.instrument != sink.instrument
        || state
            .snapshot
            .instrument(&payer.instrument)
            .is_none_or(|i| i.status != InstrumentStatus::Active)
    {
        return Err(refused());
    }
    Ok(title)
}

struct UpdateAuth<'a> {
    world: WorldAuth<'a>,
    actor: PrincipalId,
    issuer: PrincipalId,
    payer: AccountId,
    sink: AccountId,
    fee: Amount,
}
impl AuthEval for UpdateAuth<'_> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        actor == &self.issuer
            && ((account.id == self.payer && delta.checked_neg() == Some(self.fee))
                || (account.id == self.sink && delta == self.fee))
            && self.world.authorize(&self.actor, account, delta)
    }
}
fn plan(
    kind: Option<UpdateKind>,
    policy: &InitialFilingPolicy,
    actor: ObjId,
    commit: CommitToken,
    request: &RegistryCompatibleUpdate,
) -> EconomyPlan {
    plan_for(kind, policy, actor, commit, request, None)
}
fn plan_for(
    kind: Option<UpdateKind>,
    policy: &InitialFilingPolicy,
    actor: ObjId,
    commit: CommitToken,
    request: &RegistryCompatibleUpdate,
    house: Option<&HouseFilingAttribution>,
) -> EconomyPlan {
    let memo = match kind {
        Some(UpdateKind::OriginalMajorV1) => "original mind major",
        _ => "compatible mind revision",
    };
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(receipt::namespace(kind), commit.to_hex()),
        PrincipalId::System(policy.issuer.clone()),
        EconomicCause::Ruleset {
            namespace: receipt::namespace(kind).into(),
            correlation: receipt::correlation_for(commit, actor, request, policy.sink, house)
                .into(),
        },
        vec![
            RequestedLine {
                account: request.payer,
                delta: Amount::from_minor(-policy.fee.minor()),
                memo: memo.into(),
            },
            RequestedLine {
                account: policy.sink,
                delta: policy.fee,
                memo: memo.into(),
            },
        ],
    );
    plan.description = match kind {
        None => "Compatible original mind revision filing",
        Some(UpdateKind::DependentV1) => "Compatible derived mind revision filing",
        Some(UpdateKind::OriginalMajorV1) => "Original mind major filing",
        Some(UpdateKind::HouseCompatibleV1) => "Compatible House mind revision filing",
    }
    .into();
    plan
}
fn validate(
    policy: &InitialFilingPolicy,
    state: &EconomyStore,
    world: &World,
    actor: ObjId,
    request: &RegistryCompatibleUpdate,
    plan: &EconomyPlan,
    authority: &UpdateAuthority,
) -> Result<Prepared, ParticipantError> {
    title(state, world, actor, request, policy, authority)?;
    let auth = UpdateAuth {
        world: WorldAuth { w: world },
        actor: PrincipalId::Object(actor),
        issuer: PrincipalId::System(policy.issuer.clone()),
        payer: request.payer,
        sink: policy.sink,
        fee: policy.fee,
    };
    let mut spent = BTreeMap::new();
    if let Some(window) = state
        .snapshot
        .account(request.payer)
        .and_then(|a| a.approval.as_ref())
        .and_then(|a| a.window.as_ref())
    {
        spent.insert(
            request.payer,
            state.spent_in_window(request.payer, world.now(), window.period_secs),
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
    let proof = ClosedUpdatePlan {
        plan: plan.clone(),
        before_title: title(state, world, actor, request, policy, authority)?.clone(),
    };
    match crate::engine::validate_compatible_update(&proof, &state.snapshot, &env) {
        PlanResult::Post(prepared) => Ok(*prepared),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) | PlanResult::Refused(_) => {
            Err(refused())
        }
    }
}

pub(super) use install::{bind, prepare, prepare_house, recheck};

/// Minted only after this closed owner validated current title and exact fee plan.
pub(crate) struct ClosedUpdatePlan {
    plan: EconomyPlan,
    before_title: EconomicAsset,
}
impl ClosedUpdatePlan {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        if plan != &self.plan
            || !receipt::is_namespace(&plan.idempotency.namespace)
            || !matches!(&plan.cause, EconomicCause::Ruleset { namespace, .. } if namespace == &plan.idempotency.namespace)
            || snapshot.asset(self.before_title.object) != Some(&self.before_title)
            || !plan.asset_ops.is_empty()
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
