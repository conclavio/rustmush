//! Production K3 semantic action coordination.
//!
//! The typestate boundary in this module makes ordering visible in the API:
//! only [`ActionPreflight::Vacant`] yields the private [`ActionPermit`] needed
//! to parse/resolve/plan a new action. Pending and finalized retries return
//! durable identities/results directly and conflicting ingress never reaches a
//! provider.

use mush_core::{
    ActionInvocation, ActionMetadataError, ActionProviderPrepareError, AuthoritativeBundle,
    AuthoritativeBundleDraft, CanonicalHash, CanonicalHashError, CanonicalId, CanonicalIdError,
    CommitToken, DefinitionRef, Engine, GuardSet, IdempotencyKey, IdempotencyKeyTextError,
    IdempotencyScope, IngressEnvelope, IngressPayload, IngressSurfaceVersion,
    MAX_AUTHORITATIVE_BUNDLE_BYTES, OfferRouteTable, PreparedCausalRef, PrincipalId,
    ReceiptProjectionTable, ResolvedOffer, ScheduleAdvance, ScopedIdempotencyKey,
    StagePreparedActionError,
};
use mush_ipc::{GatewayInput, GatewayRequest};
use mush_proto::{
    ActionStatusId, ProjectedActionReceipt, ProjectedActionReceiptError, ProjectedActionStatus,
};
use mush_store::{
    AuthoritativeQuarantineReceipt, AuthoritativeReceipt, AuthoritativeStore,
    BoundIdempotencyStatus, IdempotencyStatus, StoreError,
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
    /// Construct the bounded public status reference for one request-key
    /// idempotency identity. No private ingress/plan/bundle hash is included.
    pub fn status_id(key: &ScopedIdempotencyKey) -> Result<ActionStatusId, ActionStatusError> {
        let IdempotencyKey::Request(request) = key.key() else {
            return Err(ActionStatusError::UnsupportedKey);
        };
        ActionStatusId::new(format!(
            "k5:{}:{}:{:016x}:{}",
            key.scope().owner,
            key.scope().namespace,
            key.epoch(),
            request
        ))
        .map_err(|_| ActionStatusError::InvalidReference)
    }

    /// Recover an exact scoped key by combining a public status reference with
    /// the transport-authenticated principal. The reference never supplies or
    /// overrides the principal.
    pub fn status_key(
        viewer: &PrincipalId,
        status: &ActionStatusId,
    ) -> Result<ScopedIdempotencyKey, ActionStatusError> {
        let mut parts = status.as_str().split(':');
        if parts.next() != Some("k5") {
            return Err(ActionStatusError::InvalidReference);
        }
        let owner = parts
            .next()
            .ok_or(ActionStatusError::InvalidReference)
            .and_then(|value| {
                mush_core::OwnerId::parse_canonical(value)
                    .map_err(|_| ActionStatusError::InvalidReference)
            })?;
        let namespace = parts
            .next()
            .ok_or(ActionStatusError::InvalidReference)
            .and_then(|value| {
                CanonicalId::parse_canonical(value).map_err(|_| ActionStatusError::InvalidReference)
            })?;
        let epoch = parts
            .next()
            .and_then(|value| u64::from_str_radix(value, 16).ok())
            .ok_or(ActionStatusError::InvalidReference)?;
        let request = parts
            .next()
            .ok_or(ActionStatusError::InvalidReference)
            .and_then(|value| {
                mush_core::RequestUid::from_hex(value)
                    .map_err(|_| ActionStatusError::InvalidReference)
            })?;
        if parts.next().is_some() {
            return Err(ActionStatusError::InvalidReference);
        }
        Ok(ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: viewer.clone(),
                owner,
                namespace,
            },
            epoch,
            IdempotencyKey::Request(request),
        ))
    }

    /// Project a compact durable receipt for one event-time eligible viewer.
    /// A provider that intentionally has no result projector still yields a
    /// generic committed receipt containing the stable transaction identity.
    pub fn project_finalized_receipt(
        projections: &ReceiptProjectionTable,
        viewer: &PrincipalId,
        status: ActionStatusId,
        receipt: &AuthoritativeReceipt,
    ) -> Result<ProjectedActionReceipt, ActionStatusError> {
        if !receipt.eligible_witnesses().contains(viewer) {
            return Err(ActionStatusError::Forbidden);
        }
        let projected = projections
            .project(receipt.action(), receipt.outcome(), viewer)
            .map_err(|source| ActionStatusError::Projection(source.to_string()))?;
        ProjectedActionReceipt::new(status, receipt.token().to_string(), projected)
            .map_err(ActionStatusError::Receipt)
    }

    /// Authorized public status lookup. A guessed reference for an absent or
    /// inaccessible request returns the same `Unknown` projection.
    pub async fn projected_status(
        store: &dyn AuthoritativeStore,
        projections: &ReceiptProjectionTable,
        viewer: &PrincipalId,
        status: &ActionStatusId,
    ) -> Result<ProjectedActionStatus, ActionStatusError> {
        let key = Self::status_key(viewer, status)?;
        let stored = store
            .bound_authoritative_idempotency_status(&key)
            .await
            .map_err(ActionStatusError::Store)?;
        Ok(match stored {
            BoundIdempotencyStatus::Retired { .. } => ProjectedActionStatus::Expired,
            BoundIdempotencyStatus::Vacant => ProjectedActionStatus::Unknown,
            BoundIdempotencyStatus::PendingExact { .. } => ProjectedActionStatus::Pending {
                reference: status.clone(),
            },
            BoundIdempotencyStatus::FinalizedExact(receipt) => {
                if !receipt.eligible_witnesses().contains(viewer) {
                    ProjectedActionStatus::Unknown
                } else {
                    ProjectedActionStatus::Committed(Self::project_finalized_receipt(
                        projections,
                        viewer,
                        status.clone(),
                        &receipt,
                    )?)
                }
            }
            BoundIdempotencyStatus::QuarantinedExact(receipt) => {
                if receipt.idempotency() != &key {
                    return Err(ActionStatusError::Integrity(
                        "quarantine status disagrees with its public reference".to_owned(),
                    ));
                }
                ProjectedActionStatus::Unavailable {
                    reference: status.clone(),
                }
            }
        })
    }

    /// Construct canonical coordinator ingress from an opaque public offer.
    /// The unauthenticated route prefix selects only the frozen provider and
    /// idempotency namespace required for the mandatory early lookup. It never
    /// authorizes execution; a vacant request must still regenerate and verify
    /// the complete offer inside the world task.
    pub fn gateway_ingress(
        request: &GatewayRequest,
        routes: &OfferRouteTable,
    ) -> Result<IngressEnvelope, String> {
        let GatewayInput::Action(input) = &request.input else {
            return Err("coordinator ingress received a non-action request".to_owned());
        };
        let action = routes
            .get(input.offered_action.route_id())
            .ok_or_else(|| "opaque offer route is not registered".to_owned())?;
        let principal = PrincipalId::Object { id: input.actor };
        let namespace =
            CanonicalId::new(action.key().id.as_str()).map_err(|error| error.to_string())?;
        let idempotency = ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: action.key().owner.clone(),
                namespace,
            },
            request.epoch,
            IdempotencyKey::Request(request.uid),
        );
        let canonical_payload = IngressPayload::structured(serde_json::json!({
            "offered_action": input.offered_action,
            "subject": input.subject,
            "targets": input.targets,
            "parameters": input.parameters,
        }))
        .map_err(|error| error.to_string())?;
        IngressEnvelope::new(
            request.uid,
            principal,
            IngressSurfaceVersion::new(
                CanonicalId::new("ipc.legal_action").expect("static surface id is valid"),
                1,
            )
            .expect("non-zero static surface version"),
            canonical_payload,
            idempotency,
        )
        .map_err(|error| error.to_string())
    }

    /// Construct deterministic internal ingress for one complete durable
    /// schedule advance.
    ///
    /// Scheduled work does not bypass K3: it receives a stable request UID,
    /// scoped deterministic idempotency, and canonical ingress before the
    /// same mandatory store preflight and provider preparation as public
    /// input. Ingress binds the durable pre-state cursor identity, not the
    /// mutable observation time: after a crash, re-evaluating that same cursor
    /// must rediscover its pending/finalized intent instead of conflicting
    /// merely because the clock advanced. The exact selected plan is bound by
    /// the resulting authoritative bundle. Cross-owner execution stays closed
    /// until an explicit bridge contract can prepare both owners atomically.
    pub fn scheduled_ingress(
        action: &DefinitionRef,
        advance: &ScheduleAdvance,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, ActionCoordinatorError> {
        mush_core::scheduled_action_ingress(action, advance, idempotency_epoch).map_err(|source| {
            if source == mush_core::ScheduleActionIngressError::OwnerMismatch {
                ActionCoordinatorError::ScheduledOwnerMismatch
            } else {
                ActionCoordinatorError::ScheduleIngress(Box::new(source))
            }
        })
    }

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
        let token = deterministic_commit_token(&permit.ingress)?;
        let (prepared, output) = engine
            .prepare_registered_authoritative_action(action, invocation, token)
            .map_err(ActionCoordinatorError::Provider)?;
        let bundle = match assemble_bundle(
            &permit.ingress,
            action,
            invocation,
            token,
            &prepared,
            output,
        ) {
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

    /// Execute only the exact route, invocation, and server-held guards
    /// produced by regenerating and resolving a K5 opaque offer.
    pub fn prepare_and_stage_resolved(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        resolved: ResolvedOffer,
    ) -> Result<StagedAuthoritativeAction, ActionCoordinatorError> {
        let (action, invocation, guards) = resolved.into_parts();
        Self::prepare_and_stage(permit, engine, &action, &invocation, guards)
    }

    /// Plan and prepare one discovered durable schedule after its canonical
    /// ingress has returned vacant from the authoritative store.
    ///
    /// The engine re-evaluates and seeds the exact cursor post-image into the
    /// provider transaction, so domain effects and schedule advancement share
    /// one bundle and a stale discovery cannot mutate either.
    pub fn prepare_and_stage_scheduled(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        candidate: &mush_core::ScheduledActionCandidate,
    ) -> Result<StagedAuthoritativeAction, ActionCoordinatorError> {
        let plan = engine
            .plan_scheduled_action(candidate, permit.ingress())
            .map_err(|source| ActionCoordinatorError::SchedulePlan(Box::new(source)))?;
        let (action, invocation, guards) = plan.into_parts();
        validate_invocation_identity(permit.ingress(), &invocation)?;
        let token = deterministic_commit_token(permit.ingress())?;
        let (prepared, output) = engine
            .prepare_registered_scheduled_authoritative_action(candidate, &invocation, token)
            .map_err(ActionCoordinatorError::Provider)?;
        let bundle = match assemble_bundle(
            permit.ingress(),
            &action,
            &invocation,
            token,
            &prepared,
            output,
        ) {
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

/// Failure at the authenticated durable-result projection boundary.
#[derive(Debug, thiserror::Error)]
pub enum ActionStatusError {
    #[error("action status supports request-key idempotency only")]
    UnsupportedKey,
    #[error("invalid action status reference")]
    InvalidReference,
    #[error("the caller is not eligible to observe this action")]
    Forbidden,
    #[error("authoritative status store lookup failed: {0}")]
    Store(StoreError),
    #[error("durable result projection failed: {0}")]
    Projection(String),
    #[error("projected receipt is invalid: {0}")]
    Receipt(ProjectedActionReceiptError),
    #[error("authoritative action status is internally inconsistent: {0}")]
    Integrity(String),
}

fn assemble_bundle(
    ingress: &IngressEnvelope,
    action: &DefinitionRef,
    invocation: &ActionInvocation,
    token: CommitToken,
    prepared: &mush_core::PreparedTxn,
    output: mush_core::PreparedAuthoritativeAction,
) -> Result<AuthoritativeBundle, ActionCoordinatorError> {
    let provider = prepared.owner().clone();
    let writes = prepared.authoritative_replay_writes();
    let (
        outcome,
        prepared_resolution,
        mut definitions,
        mut witnesses,
        durable_outbox,
        semantic_events,
    ) = output.into_parts();
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
    let resolution = prepared_resolution.map(|prepared| prepared.bind(token));
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
            "resolution": resolution,
            "semantic_events": semantic_events,
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
        resolution,
        semantic_events,
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
    #[error("scheduled advance and action have different owners")]
    ScheduledOwnerMismatch,
    #[error("scheduled action ingress is invalid: {0}")]
    ScheduleIngress(#[source] Box<mush_core::ScheduleActionIngressError>),
    #[error("scheduled action planning failed: {0}")]
    SchedulePlan(#[source] Box<mush_core::ScheduleActionPlanError>),
    #[error("semantic invocation identity does not match its preflight ingress")]
    IngressInvocationMismatch,
    #[error("action provider preparation failed: {0}")]
    Provider(#[source] ActionProviderPrepareError),
    #[error("root action causality is invalid: {0}")]
    Causality(#[source] mush_core::CausalMetadataError),
    #[error("authoritative action hashing failed: {0}")]
    Hash(#[from] CanonicalHashError),
    #[error("scheduled action metadata is invalid: {0}")]
    Metadata(#[from] ActionMetadataError),
    #[error("scheduled action canonical identity is invalid: {0}")]
    CanonicalId(#[from] CanonicalIdError),
    #[error("scheduled action idempotency text is invalid: {0}")]
    IdempotencyText(#[from] IdempotencyKeyTextError),
    #[error("authoritative bundle assembly failed: {0}")]
    Bundle(#[from] mush_core::AuthoritativeBundleError),
    #[error("authoritative action staging failed: {0}")]
    Stage(#[source] StagePreparedActionError),
}
