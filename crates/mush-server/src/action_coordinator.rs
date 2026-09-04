//! Production K3 semantic action coordination.
//!
//! The typestate boundary in this module makes ordering visible in the API:
//! only [`ActionPreflight::Vacant`] yields the private [`ActionPermit`] needed
//! to parse/resolve/plan a new action. Pending and finalized retries return
//! durable identities/results directly and conflicting ingress never reaches a
//! provider.

use mush_core::{
    ActionInvocation, ActionProviderPrepareError, AuthoritativeBundle, AuthoritativeBundleDraft,
    CanonicalHash, CanonicalHashError, CommitToken, DefinitionRef, Engine, GuardSet,
    IngressEnvelope, MAX_AUTHORITATIVE_BUNDLE_BYTES, PreparedCausalRef, PrincipalId,
    ScopedIdempotencyKey, StagePreparedActionError,
};
use mush_store::{
    AuthoritativeQuarantineReceipt, AuthoritativeReceipt, AuthoritativeStore, IdempotencyStatus,
    StoreError,
};
use serde_json::json;

/// Result of the mandatory durable lookup that precedes parsing and planning.
#[derive(Debug)]
pub enum ActionPreflight {
    /// This request belongs to an epoch outside the durable retry horizon.
    /// The caller must obtain a fresh epoch/key; it must never be replanned.
    Retired { minimum_accepted_epoch: u64 },
    /// No binding exists; this opaque permit is the only input accepted by
    /// [`ActionCoordinator::prepare_and_stage`].
    Vacant(Box<ActionPermit>),
    /// The same request already owns a durable, not-yet-finalized intent.
    Pending {
        token: CommitToken,
        bundle_hash: CanonicalHash,
    },
    /// The same request already completed. Callers project this receipt
    /// without invoking the provider or RNG again.
    Finalized(Box<AuthoritativeReceipt>),
    /// The request was consumed by a never-finalized intent whose complete
    /// recovery material is in quarantine. It must not be planned again.
    Quarantined(Box<AuthoritativeQuarantineReceipt>),
    /// The scoped key is already bound to different canonical ingress.
    Conflict,
}

/// Viewer-projected semantic status. Unauthorized callers never receive the
/// existence, token, action identity, or outcome of another principal's key.
#[derive(Debug)]
pub enum SemanticActionStatus {
    Forbidden,
    Retired {
        minimum_accepted_epoch: u64,
    },
    Vacant,
    Pending {
        token: CommitToken,
        bundle_hash: CanonicalHash,
    },
    Finalized(Box<AuthoritativeReceipt>),
    Quarantined(Box<AuthoritativeQuarantineReceipt>),
    Conflict,
}

/// Proof that exact idempotency lookup returned vacant for this ingress.
/// Fields and construction remain private to prevent bypassing preflight.
#[derive(Debug)]
pub struct ActionPermit {
    ingress: IngressEnvelope,
}

impl ActionPermit {
    #[must_use]
    pub const fn ingress(&self) -> &IngressEnvelope {
        &self.ingress
    }
}

/// Exact identity of an action staged into the engine's one-shot K3 lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StagedAuthoritativeAction {
    pub token: CommitToken,
    pub bundle_hash: CanonicalHash,
}

/// Stateless coordinator; all mutable truth remains in the engine and store.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActionCoordinator;

impl ActionCoordinator {
    /// Authorized semantic `action/status` lookup. Finalized outcomes are
    /// returned only to an event-time eligible witness.
    pub async fn semantic_status(
        store: &dyn AuthoritativeStore,
        viewer: &PrincipalId,
        key: &ScopedIdempotencyKey,
        ingress_hash: CanonicalHash,
    ) -> Result<SemanticActionStatus, StoreError> {
        if &key.scope().principal != viewer {
            return Ok(SemanticActionStatus::Forbidden);
        }
        Ok(
            match store
                .authoritative_idempotency_status(key, ingress_hash)
                .await?
            {
                IdempotencyStatus::Retired {
                    minimum_accepted_epoch,
                } => SemanticActionStatus::Retired {
                    minimum_accepted_epoch,
                },
                IdempotencyStatus::Vacant => SemanticActionStatus::Vacant,
                IdempotencyStatus::PendingExact { token, bundle_hash } => {
                    SemanticActionStatus::Pending { token, bundle_hash }
                }
                IdempotencyStatus::FinalizedExact(receipt) => {
                    if receipt.eligible_witnesses().contains(viewer) {
                        SemanticActionStatus::Finalized(receipt)
                    } else {
                        SemanticActionStatus::Forbidden
                    }
                }
                IdempotencyStatus::QuarantinedExact(receipt) => {
                    SemanticActionStatus::Quarantined(receipt)
                }
                IdempotencyStatus::Conflict => SemanticActionStatus::Conflict,
            },
        )
    }

    /// Look up exact durable idempotency before any parser, resolver, provider,
    /// truth read or RNG is allowed to run.
    pub async fn preflight(
        store: &dyn AuthoritativeStore,
        ingress: IngressEnvelope,
    ) -> Result<ActionPreflight, StoreError> {
        match store
            .authoritative_idempotency_status(ingress.idempotency(), ingress.ingress_hash())
            .await?
        {
            IdempotencyStatus::Retired {
                minimum_accepted_epoch,
            } => Ok(ActionPreflight::Retired {
                minimum_accepted_epoch,
            }),
            IdempotencyStatus::Vacant => {
                Ok(ActionPreflight::Vacant(Box::new(ActionPermit { ingress })))
            }
            IdempotencyStatus::PendingExact { token, bundle_hash } => {
                Ok(ActionPreflight::Pending { token, bundle_hash })
            }
            IdempotencyStatus::FinalizedExact(receipt) => Ok(ActionPreflight::Finalized(receipt)),
            IdempotencyStatus::QuarantinedExact(receipt) => {
                Ok(ActionPreflight::Quarantined(receipt))
            }
            IdempotencyStatus::Conflict => Ok(ActionPreflight::Conflict),
        }
    }

    /// Run the selected production provider, assemble every semantic bundle
    /// field centrally, and stage the exact write/install pair. The provider
    /// controls only typed post-images and its bounded semantic output.
    pub fn prepare_and_stage(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        action: &DefinitionRef,
        invocation: &ActionInvocation,
        guards: GuardSet,
    ) -> Result<StagedAuthoritativeAction, ActionCoordinatorError> {
        validate_invocation_identity(&permit.ingress, invocation)?;
        let (prepared, output) = engine
            .prepare_registered_authoritative_action(action, invocation)
            .map_err(ActionCoordinatorError::Provider)?;
        let bundle = match assemble_bundle(&permit.ingress, action, invocation, &prepared, output) {
            Ok(bundle) => bundle,
            Err(error) => {
                engine.abort_prepared_transaction(prepared);
                return Err(error);
            }
        };
        let token = bundle.token();
        let bundle_hash = bundle.bundle_hash();
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .map_err(ActionCoordinatorError::Stage)?;
        Ok(StagedAuthoritativeAction { token, bundle_hash })
    }
}

fn assemble_bundle(
    ingress: &IngressEnvelope,
    action: &DefinitionRef,
    invocation: &ActionInvocation,
    prepared: &mush_core::PreparedTxn,
    output: mush_core::AuthoritativeActionOutput,
) -> Result<AuthoritativeBundle, ActionCoordinatorError> {
    let provider = prepared.owner().clone();
    let writes = prepared.authoritative_replay_writes();
    let (outcome, mut definitions, mut witnesses, durable_outbox) = output.into_parts();
    definitions.push(action.clone());
    definitions.push(outcome.schema().clone());
    definitions.sort();
    definitions.dedup();
    if witnesses.is_empty() {
        witnesses.push(invocation.principal().clone());
    }
    witnesses.sort();
    witnesses.dedup();
    let mut durable_outbox = durable_outbox;
    durable_outbox.sort_by_key(mush_core::DurableOutboxIntent::ordinal);
    let cause = PreparedCausalRef::new(invocation.request(), None, 0)
        .map_err(ActionCoordinatorError::Causality)?;
    let token = deterministic_commit_token(ingress)?;
    let plan_hash = CanonicalHash::for_canonical_value(
        "authoritative_action_plan.v1",
        1,
        &json!({
            "request_hash": invocation.request_hash(),
            "action": action,
            "provider": provider,
            "cause": cause,
            "definitions": definitions,
            "writes": writes,
            "outcome": outcome,
            "eligible_witnesses": witnesses,
            "durable_outbox": durable_outbox,
        }),
        MAX_AUTHORITATIVE_BUNDLE_BYTES,
    )?;
    Ok(AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token,
        request: invocation.request(),
        idempotency: invocation.idempotency().clone(),
        ingress_hash: ingress.ingress_hash(),
        request_hash: invocation.request_hash(),
        plan_hash,
        action: action.clone(),
        provider,
        cause,
        definitions,
        writes,
        outcome,
        eligible_witnesses: witnesses,
        durable_outbox,
    })?)
}

fn validate_invocation_identity(
    ingress: &IngressEnvelope,
    invocation: &ActionInvocation,
) -> Result<(), ActionCoordinatorError> {
    if invocation.request() != ingress.request()
        || invocation.principal() != ingress.principal()
        || invocation.idempotency() != ingress.idempotency()
    {
        return Err(ActionCoordinatorError::IngressInvocationMismatch);
    }
    Ok(())
}

fn deterministic_commit_token(
    ingress: &IngressEnvelope,
) -> Result<CommitToken, CanonicalHashError> {
    let digest = CanonicalHash::for_canonical_value(
        "authoritative_commit_token.v1",
        1,
        &json!({
            "request": ingress.request(),
            "idempotency": ingress.idempotency(),
            "ingress_hash": ingress.ingress_hash(),
        }),
        8 * 1024,
    )?;
    let mut token = [0_u8; 16];
    token.copy_from_slice(&digest.as_bytes()[..16]);
    Ok(CommitToken::from_bytes(token))
}

#[derive(Debug, thiserror::Error)]
pub enum ActionCoordinatorError {
    #[error("semantic invocation identity does not match its preflight ingress")]
    IngressInvocationMismatch,
    #[error("action provider preparation failed: {0}")]
    Provider(#[source] ActionProviderPrepareError),
    #[error("root action causality is invalid: {0}")]
    Causality(#[source] mush_core::CausalMetadataError),
    #[error("authoritative action hashing failed: {0}")]
    Hash(#[from] CanonicalHashError),
    #[error("authoritative bundle assembly failed: {0}")]
    Bundle(#[from] mush_core::AuthoritativeBundleError),
    #[error("authoritative action staging failed: {0}")]
    Stage(#[source] StagePreparedActionError),
}
