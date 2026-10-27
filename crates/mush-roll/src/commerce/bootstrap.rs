//! Explicit first-install setup. Missing normal-mode data is never initialization.
//!
//! The composition registers this provider only in Initialize mode. It publishes
//! both mandatory deployment markers under one real writer receipt, and leaves
//! commerce closed until a normal Active-mode restart completes every audit.

use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    AuthorityGuard, AuthorityGuardContext, AuthorityGuardKind, AuthorityGuardValidator,
    PerceivedRef, PrincipalId,
};
use mush_core::mechanics::{CanonicalId, DefinitionRef, RulesValue};
use mush_core::reserved_shell::{INITIALIZE_ACTION, ShellReservations};
use mush_core::{
    ActionTxn, Engine, GOD, LegalActionQuery, ModError, ModuleCtx, PrepareCtx, ProviderOfferDraft,
    Registrar, World,
};
use mush_economy::EconomyStore;
use serde_json::json;

use super::families::CommerceFamilies;
use super::rating::RatingFamilies;
use super::records::{CommerceDeployment, PUBLICATION_FORMAT, PublicationError, Record};
use crate::RollStore;

const OUTCOME: &str = "commerce_initialized";

fn record_error<T>(_: T) -> PublicationError {
    PublicationError::Unavailable
}
fn exact(def: &DefinitionRef, kind: &str, id: &str) -> bool {
    def.key().owner.as_str() == "roll"
        && def.key().kind.as_str() == kind
        && def.key().id.as_str() == id
        && def.version() == 1
}

fn valid_operator(inv: &ActionInvocation) -> bool {
    if inv.actor() != GOD
        || inv.principal() != &(PrincipalId::Object { id: GOD })
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || !inv
            .parameters()
            .as_object()
            .is_some_and(serde_json::Map::is_empty)
        || inv.idempotency().scope().owner.as_str() != "roll"
    {
        return false;
    }
    true
}

/// Actual source-audited owner state and journal history. An empty receipt map
/// whose replay has not completed is refused by the economy service itself.
fn initialization_generation(world: &World) -> Result<u64, PublicationError> {
    let roll = world
        .module_state::<RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    let (generation, _) = roll.commerce_initialization_base().map_err(record_error)?;
    let economy = world
        .module_state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if economy.is_frozen()
        || !economy
            .filing_receipt_audit()
            .is_empty()
            .map_err(record_error)?
    {
        return Err(PublicationError::Mismatch);
    }
    if economy
        .update_receipt_audit()
        .receipts()
        .map_err(record_error)?
        .next()
        .is_some()
    {
        return Err(PublicationError::Mismatch);
    }
    if economy
        .pending_charter_histories()
        .map_err(record_error)?
        .next()
        .is_some()
    {
        return Err(PublicationError::Mismatch);
    }
    let god = world.get(GOD).ok_or(PublicationError::Unavailable)?;
    if god.kind != mush_core::ObjKind::Player || god.flags.contains(mush_core::Flags::GOING) {
        return Err(PublicationError::Mismatch);
    }
    Ok(generation)
}

fn empty_in_txn(
    txn: &mut ActionTxn<'_>,
    f: &CommerceFamilies,
    r: &RatingFamilies,
) -> Result<(), PublicationError> {
    macro_rules! empty {
        ($handle:expr) => {
            if !txn.family_is_empty($handle).map_err(record_error)? {
                return Err(PublicationError::Mismatch);
            }
        };
    }
    empty!(&f.paid_resales);
    empty!(&f.paid_resale_heads);
    empty!(&f.paid_holders);
    empty!(&f.charter_pending_offers);
    empty!(&f.charter_pending_buyer_offers);
    empty!(&f.charter_pending);
    empty!(&f.charter_pending_heads);
    empty!(&f.charter_transfer_offers);
    empty!(&f.charter_transfers);
    empty!(&f.charter_ownership);
    empty!(&f.lapse_occurrences);
    empty!(&f.ic_claims);
    empty!(&f.coverages);
    empty!(&f.registry_recalls);
    empty!(&f.measured_recalls);
    empty!(&f.reconformance_heads);
    empty!(&f.registry_revocations);
    empty!(&f.revoked_terms);
    empty!(&f.withdrawals);
    empty!(&f.owner_qualification_operations);
    empty!(&f.owner_qualification_heads);
    empty!(&f.owner_attestations);
    empty!(&f.line_updates);
    empty!(&f.update_positions);
    empty!(&f.draft_parents);
    empty!(&f.authored_retirements);
    empty!(&f.retired_draft_parents);
    empty!(&f.owner_parents);
    empty!(&f.deployment);
    empty!(&f.charters);
    empty!(&f.lines);
    empty!(&f.filed);
    empty!(&f.status);
    empty!(&f.licences);
    empty!(&f.terms);
    empty!(&f.operations);
    empty!(&f.identities);
    empty!(&f.positions);
    empty!(&f.filed_royalties);
    empty!(&f.term_economics);
    empty!(&f.entitlement_allocations);
    empty!(&f.renewal_operations);
    empty!(&f.renewal_heads);
    empty!(&f.paid_allocations);
    empty!(&f.paid_licences);
    empty!(&f.paid_terms);
    empty!(&f.paid_holdings);
    empty!(&r.receipts);
    empty!(&r.pages);
    empty!(&r.jobs);
    empty!(&r.executions);
    empty!(&r.completions);
    empty!(&r.status);
    Ok(())
}

/// The final setup-mode hook validates the actual composition without opening
/// commerce. A configured setup mode over any existing commerce refuses boot.
fn validate_setup_boot(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    r: &RatingFamilies,
    shells: &ShellReservations,
) -> Result<(), PublicationError> {
    initialization_generation(ctx.w)?;
    macro_rules! empty {
        ($handle:expr) => {
            if !ctx
                .scan_action_state($handle, None, 1)
                .map_err(record_error)?
                .entries()
                .is_empty()
            {
                return Err(PublicationError::Mismatch);
            }
        };
    }
    empty!(&f.paid_resales);
    empty!(&f.paid_resale_heads);
    empty!(&f.paid_holders);
    empty!(&f.charter_pending_offers);
    empty!(&f.charter_pending_buyer_offers);
    empty!(&f.charter_pending);
    empty!(&f.charter_pending_heads);
    empty!(&f.charter_transfer_offers);
    empty!(&f.charter_transfers);
    empty!(&f.charter_ownership);
    empty!(&f.lapse_occurrences);
    empty!(&f.ic_claims);
    empty!(&f.coverages);
    empty!(&f.registry_recalls);
    empty!(&f.measured_recalls);
    empty!(&f.reconformance_heads);
    empty!(&f.registry_revocations);
    empty!(&f.revoked_terms);
    empty!(&f.withdrawals);
    empty!(&f.owner_qualification_operations);
    empty!(&f.owner_qualification_heads);
    empty!(&f.owner_attestations);
    empty!(&f.line_updates);
    empty!(&f.update_positions);
    empty!(&f.draft_parents);
    empty!(&f.authored_retirements);
    empty!(&f.retired_draft_parents);
    empty!(&f.owner_parents);
    empty!(&f.deployment);
    empty!(&f.charters);
    empty!(&f.lines);
    empty!(&f.filed);
    empty!(&f.status);
    empty!(&f.licences);
    empty!(&f.terms);
    empty!(&f.operations);
    empty!(&f.identities);
    empty!(&f.positions);
    empty!(&f.filed_royalties);
    empty!(&f.term_economics);
    empty!(&f.entitlement_allocations);
    empty!(&f.renewal_operations);
    empty!(&f.renewal_heads);
    empty!(&f.paid_allocations);
    empty!(&f.paid_licences);
    empty!(&f.paid_terms);
    empty!(&f.paid_holdings);
    empty!(&r.receipts);
    empty!(&r.pages);
    empty!(&r.jobs);
    empty!(&r.executions);
    empty!(&r.completions);
    empty!(&r.status);
    if !shells.empty_for_initialization(ctx).map_err(record_error)? {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

pub(crate) fn verify_initialize_mode(
    ctx: &ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
) -> Result<(), ModError> {
    validate_setup_boot(ctx, families, ratings, shells)
        .map_err(|error| ModError::msg(format!("commerce initialization preflight: {error}")))
}

struct InitializationGuard;
impl AuthorityGuardValidator for InitializationGuard {
    fn is_current(
        &self,
        context: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        let AuthorityGuard::Capability {
            provider,
            actor,
            capability: requested,
            generation,
        } = guard
        else {
            return Ok(false);
        };
        Ok(provider.as_str() == "roll"
            && *actor == GOD
            && requested.as_str() == INITIALIZE_ACTION
            && initialization_generation(context.world()).is_ok_and(|actual| actual == *generation))
    }
}

/// Called only by Initialize-mode Roll registration. The same `(Roll,
/// Capability)` validator must not be independently registered a second time.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    shells: ShellReservations,
) -> Result<(), ModError> {
    if reg.module() != "roll"
        || !exact(&action, "action", INITIALIZE_ACTION)
        || !exact(&outcome, "outcome", OUTCOME)
    {
        return Err(ModError::msg(
            "invalid explicit commerce initialization registration",
        ));
    }
    let capability =
        CanonicalId::new(INITIALIZE_ACTION).map_err(|error| ModError::msg(error.to_string()))?;
    let refusal = ActionError::new(ActionErrorCode::ProviderRejected, false,
        "Commerce setup requires the initial operator, empty commerce storage, and completed recovery checks.")
        .map_err(|error| ModError::msg(error.to_string()))?;
    reg.authority_guard_validator(AuthorityGuardKind::Capability, InitializationGuard)?;
    reg.action_provider(
        action.clone(),
        BootstrapProvider {
            action,
            outcome,
            capability,
            refusal,
            families,
            ratings,
            shells,
        },
    )
}

struct BootstrapProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    capability: CanonicalId,
    refusal: ActionError,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    shells: ShellReservations,
}
impl ActionProvider for BootstrapProvider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        if valid_operator(inv) {
            Ok(())
        } else {
            Err(self.refusal.clone())
        }
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.actor() != GOD
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: GOD })
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let Ok(generation) = initialization_generation(&engine.world) else {
            return Ok(None);
        };
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| self.refusal.clone())?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| self.refusal.clone())?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| self.refusal.clone())?;
        guards
            .add(AuthorityGuard::Capability {
                provider: self.action.key().owner.clone(),
                actor: GOD,
                capability: self.capability.clone(),
                generation,
            })
            .map_err(|_| self.refusal.clone())?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(Vec::new())
                .map_err(|_| self.refusal.clone())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "roll-initialize-commerce",
                "Initialize mind commerce",
                "Create the initial commerce records, then restart in active mode.",
            )
            .map_err(|_| self.refusal.clone())?,
            guards: guards.finish().map_err(|_| self.refusal.clone())?,
        }))
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(inv)?;
        initialization_generation(context.world().map_err(|_| self.refusal.clone())?)
            .map_err(|_| self.refusal.clone())?;
        empty_in_txn(txn, &self.families, &self.ratings).map_err(|_| self.refusal.clone())?;
        context
            .initialize_commerce_shells(&self.shells, txn)
            .map_err(|_| self.refusal.clone())?;
        let initialized_by = context.commit_token();
        let deployment = CommerceDeployment {
            format: PUBLICATION_FORMAT,
            initialized_by,
            next_line: 1,
            next_filed: 1,
            next_licence: 1,
        };
        deployment.validate().map_err(|_| self.refusal.clone())?;
        txn.insert(
            &self.families.deployment,
            &"registry".to_owned(),
            deployment,
        )
        .map_err(|_| self.refusal.clone())?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "initialized_by":initialized_by,"restart_in_active_mode":true,
            }),
        )
        .map_err(|_| self.refusal.clone())?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::mechanics::OwnerId;
    fn owner() -> OwnerId {
        OwnerId::new("roll").unwrap()
    }
    use mush_core::action::{
        ActionInvocationDraft, ActionOfferId, CanonicalHash, IdempotencyKey, IdempotencyScope,
        RequestUid, ScopedIdempotencyKey,
    };

    fn invocation(
        actor: mush_core::ObjId,
        scope: OwnerId,
        parameters: serde_json::Value,
        subject: PerceivedRef,
    ) -> ActionInvocation {
        let request = RequestUid::from_bytes([19; 16]);
        let principal = PrincipalId::Object { id: actor };
        ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([27; 32])),
            subject,
            targets: Vec::new(),
            parameters,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: scope,
                    namespace: CanonicalId::new("setup").unwrap(),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap()
    }

    #[test]
    fn setup_accepts_only_the_actual_operator_self_invocation_with_no_client_terms() {
        assert!(valid_operator(&invocation(
            GOD,
            owner(),
            json!({}),
            PerceivedRef::SelfActor
        )));
        for parameters in [
            json!({"actor":1}),
            json!({"receipt":"invented"}),
            json!({"empty":true}),
            json!([]),
        ] {
            assert!(!valid_operator(&invocation(
                GOD,
                owner(),
                parameters,
                PerceivedRef::SelfActor
            )));
        }
        assert!(!valid_operator(&invocation(
            mush_core::ObjId(2),
            owner(),
            json!({}),
            PerceivedRef::SelfActor
        )));
        assert!(!valid_operator(&invocation(
            GOD,
            OwnerId::new("foreign").unwrap(),
            json!({}),
            PerceivedRef::SelfActor
        )));
        assert!(!valid_operator(&invocation(
            GOD,
            owner(),
            json!({}),
            PerceivedRef::KnownObject {
                id: GOD,
                revision: 1
            }
        )));
    }

    #[test]
    fn an_empty_world_or_unverified_owner_is_never_an_initialization_proof() {
        let mut world = World::new_minimal();
        assert!(initialization_generation(&world).is_err());
        world.module_state_insert(RollStore::default());
        world.module_state_insert(EconomyStore::default());
        assert!(
            initialization_generation(&world).is_err(),
            "default maps do not prove source audit or historical journal replay"
        );
    }
}
