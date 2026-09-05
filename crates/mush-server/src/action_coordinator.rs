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
    ReceiptProjectionTable, ResolvedAuthoritativeOffer, ScheduleAdvance, ScopedIdempotencyKey,
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

/// Exact durable action identity plus provider-worker custody. The worker
/// lease may be released only after this exact action commits.
#[derive(Debug)]
pub struct StagedDecisionLease {
    pub staged: StagedAuthoritativeAction,
    pub worker: mush_core::DecisionWorkerLease,
}

/// Ordinary selected action staged from one exact authenticated provider
/// completion, plus the binding used by the decision lifecycle reconciler.
#[derive(Debug)]
pub struct StagedDecisionResponse {
    pub staged: StagedAuthoritativeAction,
    pub fence: mush_core::DecisionCompletionFence,
    pub binding: mush_core::DecisionActionBinding,
}

/// Exact provider-attempt action plus optional fallback custody released only
/// after this action commits.
#[derive(Debug)]
pub struct StagedDecisionAttempt {
    pub staged: StagedAuthoritativeAction,
    pub fallback_fence: Option<mush_core::DecisionCompletionFence>,
}

/// Exact fallback-custody action plus the fence released only after commit.
#[derive(Debug)]
pub struct StagedDecisionFallbackTrigger {
    pub staged: StagedAuthoritativeAction,
    pub fallback_fence: mush_core::DecisionCompletionFence,
}

/// Stateless coordinator; all mutable truth remains in the engine and store.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActionCoordinator;

/// Domain tag of the deterministic request identity minted for one
/// command-originated authoritative submission (E7c2 B1).
///
/// It names the **surface**, not a crate: this seam is shared by every future
/// `CommandCompatibility::Authoritative` handler, and the action's own owner and
/// id are inside the hashed value, so two crates' commands cannot collide.
pub(crate) const COMMAND_REQUEST_DOMAIN: &str = "cmd.legal_action.v1";
/// Canonical-encoding ceiling for that hashed request. Well below the bundle
/// ceiling: a command's selector is an action ref, a subject, bounded targets
/// and bounded parameters.
pub(crate) const MAX_COMMAND_REQUEST_BYTES: usize = 64 * 1024;

/// Process-local epoch for direct interactive action identities. Kept outside
/// provider implementation files so the C5 architectural tripwire can prove
/// providers have no ambient RNG access.
pub(crate) fn fresh_action_epoch() -> u64 {
    rand::random()
}

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

    // ---- E7c2: B1 — the `Authoritative` ingress adapter seam ----
    /// Construct canonical coordinator ingress for one command-originated
    /// authoritative submission.
    ///
    /// This sits beside [`Self::gateway_ingress`] and [`Self::scheduled_ingress`]
    /// as the third — and, for a `CommandCompatibility::Authoritative` handler,
    /// the only — way into the same mandatory store preflight. It discharges
    /// 08j §5 (iii) (*"the first `Authoritative` registration needs an adapter
    /// seam that does not exist"*) and 08i §2 row 5 (*"There is no `Ctx`-level
    /// action submit"*).
    ///
    /// **The request identity is deterministic**, not a fresh UID:
    /// `ActionCoordinator::status_id` refuses a `Deterministic` key
    /// ([`ActionStatusError::UnsupportedKey`]), so the ingress key must stay
    /// `IdempotencyKey::Request` — and a *random* UID would make a retyped
    /// identical command a second commit. The UID is therefore the first 16
    /// bytes of a canonical hash over the complete semantic request
    /// `{actor, action, subject, targets, parameters, key}`, so an identical
    /// command inside one process epoch preflights as a **HIT**. Cross-restart
    /// replay is the mechanic's job, not the ingress's:
    /// `ReservationId::derive(holder, resource)` makes the second attempt a HIT
    /// at the provider.
    pub fn command_ingress(
        submission: &mush_core::AuthoritativeSubmit,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, String> {
        let principal = PrincipalId::Object {
            id: submission.actor,
        };
        let namespace = CanonicalId::new(submission.action.key().id.as_str())
            .map_err(|error| error.to_string())?;
        let digest = CanonicalHash::for_canonical_value(
            COMMAND_REQUEST_DOMAIN,
            1,
            &json!({
                "actor": submission.actor,
                "action": submission.action,
                "subject": submission.subject,
                "targets": submission.targets,
                "parameters": submission.parameters,
                "key": submission.key,
            }),
            MAX_COMMAND_REQUEST_BYTES,
        )
        .map_err(|error| error.to_string())?;
        let mut uid = [0_u8; 16];
        uid.copy_from_slice(&digest.as_bytes()[..16]);
        let request = mush_core::RequestUid::from_bytes(uid);
        let idempotency = ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: submission.action.key().owner.clone(),
                namespace,
            },
            idempotency_epoch,
            IdempotencyKey::Request(request),
        );
        let canonical_payload = IngressPayload::structured(json!({
            "action": submission.action,
            "subject": submission.subject,
            "targets": submission.targets,
            "parameters": submission.parameters,
        }))
        .map_err(|error| error.to_string())?;
        IngressEnvelope::new(
            request,
            principal,
            IngressSurfaceVersion::new(
                CanonicalId::new("cmd.legal_action").expect("static surface id is valid"),
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

    /// Construct canonical coordinator ingress for one decision lease attempt.
    pub fn decision_lease_ingress(
        engine: &Engine,
        candidate: &mush_core::DecisionDispatchCandidate,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, ActionCoordinatorError> {
        engine
            .decision_lease_ingress(candidate, idempotency_epoch)
            .map_err(|source| ActionCoordinatorError::DecisionLeaseIngress(Box::new(source)))
    }

    /// Authenticate and canonicalize one exact delayed-decision response
    /// before its ordinary action idempotency lookup.
    pub fn decision_response_ingress(
        engine: &Engine,
        fence: &mush_core::DecisionCompletionFence,
        response: &mush_core::DecisionResponse,
    ) -> Result<IngressEnvelope, ActionCoordinatorError> {
        engine
            .decision_response_ingress(fence, response)
            .map_err(|source| ActionCoordinatorError::DecisionResponseIngress(Box::new(source)))
    }

    /// Select and canonicalize one deterministic projected-only fallback.
    pub fn decision_fallback_ingress(
        engine: &Engine,
        candidate: &mush_core::DecisionFallbackCandidate,
    ) -> Result<(mush_core::DecisionResponse, IngressEnvelope), ActionCoordinatorError> {
        let response = engine
            .select_decision_fallback(candidate)
            .map_err(|source| ActionCoordinatorError::DecisionFallbackSelect(Box::new(source)))?;
        let ingress = engine
            .decision_fallback_ingress(candidate, &response)
            .map_err(|source| ActionCoordinatorError::DecisionResponseIngress(Box::new(source)))?;
        Ok((response, ingress))
    }

    /// Construct canonical owner-local ingress for recording an already
    /// durable selected-action binding on its exact pending decision.
    pub fn decision_resolving_ingress(
        engine: &Engine,
        fence: &mush_core::DecisionCompletionFence,
        binding: &mush_core::DecisionActionBinding,
        outcome: mush_core::DecisionTerminalOutcome,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, ActionCoordinatorError> {
        engine
            .decision_resolving_ingress(fence, binding, outcome, idempotency_epoch)
            .map_err(|source| ActionCoordinatorError::DecisionResolvingIngress(Box::new(source)))
    }

    pub fn decision_attempt_ingress(
        engine: &Engine,
        fence: &mush_core::DecisionCompletionFence,
        kind: mush_core::DecisionAttemptFailureKind,
        max_attempts: u8,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, ActionCoordinatorError> {
        engine
            .decision_attempt_ingress(fence, kind, max_attempts, idempotency_epoch)
            .map_err(|source| ActionCoordinatorError::DecisionAttemptIngress(Box::new(source)))
    }

    pub fn decision_fallback_trigger_ingress(
        engine: &Engine,
        candidate: &mush_core::DecisionFallbackTriggerCandidate,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, ActionCoordinatorError> {
        engine
            .decision_fallback_trigger_ingress(candidate, idempotency_epoch)
            .map_err(|source| ActionCoordinatorError::DecisionFallbackTrigger(Box::new(source)))
    }

    /// Construct canonical owner-local ingress for terminalizing one exact
    /// resolving decision binding.
    pub fn decision_terminal_ingress(
        engine: &Engine,
        fence: &mush_core::DecisionCompletionFence,
        binding: &mush_core::DecisionActionBinding,
        outcome: mush_core::DecisionTerminalOutcome,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, ActionCoordinatorError> {
        engine
            .decision_terminal_ingress(fence, binding, outcome, idempotency_epoch)
            .map_err(|source| ActionCoordinatorError::DecisionTerminalIngress(Box::new(source)))
    }

    pub fn decision_compaction_ingress(
        engine: &Engine,
        candidate: &mush_core::DecisionCompactionCandidate,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, ActionCoordinatorError> {
        engine
            .decision_compaction_ingress(candidate, idempotency_epoch)
            .map_err(|source| ActionCoordinatorError::DecisionCompaction(Box::new(source)))
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
        resolved: ResolvedAuthoritativeOffer,
    ) -> Result<StagedAuthoritativeAction, ActionCoordinatorError> {
        let (action, resolved, guards) = resolved.into_authoritative_parts();
        let invocation = resolved.invocation();
        validate_invocation_identity(&permit.ingress, invocation)?;
        let token = deterministic_commit_token(&permit.ingress)?;
        let (prepared, output) = engine
            .prepare_registered_resolved_authoritative_action(&action, &resolved, token)
            .map_err(ActionCoordinatorError::Provider)?;
        let bundle = match assemble_bundle(
            &permit.ingress,
            &action,
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

    /// Prepare one operational delayed-decision lease through its registered
    /// owner action. The exact lease post-image and action outcome share one
    /// bundle; worker custody is returned but not spawned here.
    pub fn prepare_and_stage_decision_lease(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        candidate: &mush_core::DecisionDispatchCandidate,
    ) -> Result<StagedDecisionLease, ActionCoordinatorError> {
        let plan = engine
            .plan_decision_dispatch(candidate, permit.ingress())
            .map_err(|source| ActionCoordinatorError::DecisionLeasePlan(Box::new(source)))?;
        let (action, invocation, guards, transition) = plan.into_parts();
        validate_invocation_identity(permit.ingress(), &invocation)?;
        let token = deterministic_commit_token(permit.ingress())?;
        let (prepared, output, worker) = engine
            .prepare_registered_decision_lease_action(
                candidate,
                &action,
                &invocation,
                transition,
                token,
            )
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
        let staged = StagedAuthoritativeAction {
            token: bundle.token(),
            bundle_hash: bundle.bundle_hash(),
        };
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .map_err(ActionCoordinatorError::Stage)?;
        Ok(StagedDecisionLease { staged, worker })
    }

    /// Regenerate the selected prompt offer, restore private prompt-time
    /// witnesses, and enter the same resolved authoritative action path used
    /// by human clients.
    pub fn prepare_and_stage_decision_response(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        fence: mush_core::DecisionCompletionFence,
        response: &mush_core::DecisionResponse,
    ) -> Result<StagedDecisionResponse, ActionCoordinatorError> {
        let resolution = engine
            .resolve_decision_response(&fence, response, permit.ingress())
            .map_err(|source| ActionCoordinatorError::DecisionResponseResolve(Box::new(source)))?;
        require_current_guards(engine, resolution.resolved().guards())?;
        let binding = resolution.binding().clone();
        let staged = Self::prepare_and_stage_resolved(permit, engine, resolution.into_resolved())?;
        Ok(StagedDecisionResponse {
            staged,
            fence,
            binding,
        })
    }

    pub fn decision_cancellation_ingress(
        engine: &Engine,
        controller: &mush_core::DecisionControllerId,
        decision: mush_core::DecisionId,
        revision: u64,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, ActionCoordinatorError> {
        engine
            .decision_cancellation_ingress(controller, decision, revision, idempotency_epoch)
            .map_err(|source| ActionCoordinatorError::DecisionCancellationIngress(Box::new(source)))
    }

    /// Resolve a deterministic fallback through the same ordinary action
    /// coordinator used by async and human selections.
    pub fn prepare_and_stage_decision_fallback(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        candidate: &mush_core::DecisionFallbackCandidate,
        response: &mush_core::DecisionResponse,
    ) -> Result<StagedDecisionResponse, ActionCoordinatorError> {
        let resolution = engine
            .resolve_decision_fallback(candidate, response, permit.ingress())
            .map_err(|source| ActionCoordinatorError::DecisionResponseResolve(Box::new(source)))?;
        require_current_guards(engine, resolution.resolved().guards())?;
        let binding = resolution.binding().clone();
        let staged = Self::prepare_and_stage_resolved(permit, engine, resolution.into_resolved())?;
        Ok(StagedDecisionResponse {
            staged,
            fence: candidate.fence().clone(),
            binding,
        })
    }

    /// Record the exact selected-action binding through the controller
    /// owner's registered action. The post-image and lifecycle action share
    /// one authoritative bundle.
    pub fn prepare_and_stage_decision_resolving(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        fence: &mush_core::DecisionCompletionFence,
        binding: &mush_core::DecisionActionBinding,
        outcome: mush_core::DecisionTerminalOutcome,
    ) -> Result<StagedAuthoritativeAction, ActionCoordinatorError> {
        let plan = engine
            .plan_decision_resolving(fence, binding, outcome, permit.ingress())
            .map_err(|source| ActionCoordinatorError::DecisionResolvingPlan(Box::new(source)))?;
        let (action, invocation, guards, transition) = plan.into_parts();
        validate_invocation_identity(permit.ingress(), &invocation)?;
        let token = deterministic_commit_token(permit.ingress())?;
        let (prepared, output) = engine
            .prepare_registered_decision_resolving_action(
                fence,
                &action,
                &invocation,
                transition,
                token,
            )
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
        let staged = StagedAuthoritativeAction {
            token: bundle.token(),
            bundle_hash: bundle.bundle_hash(),
        };
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .map_err(ActionCoordinatorError::Stage)?;
        Ok(staged)
    }

    /// Record one exact resolving decision's terminal post-image through the
    /// controller owner's registered action.
    pub fn prepare_and_stage_decision_terminal(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        fence: &mush_core::DecisionCompletionFence,
        binding: &mush_core::DecisionActionBinding,
        outcome: mush_core::DecisionTerminalOutcome,
    ) -> Result<StagedAuthoritativeAction, ActionCoordinatorError> {
        let plan = engine
            .plan_decision_terminal(fence, binding, outcome, permit.ingress())
            .map_err(|source| ActionCoordinatorError::DecisionTerminalPlan(Box::new(source)))?;
        let (action, invocation, guards, transition) = plan.into_parts();
        validate_invocation_identity(permit.ingress(), &invocation)?;
        let token = deterministic_commit_token(permit.ingress())?;
        let (prepared, output) = engine
            .prepare_registered_decision_terminal_action(
                fence,
                &action,
                &invocation,
                transition,
                token,
            )
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
        let staged = StagedAuthoritativeAction {
            token: bundle.token(),
            bundle_hash: bundle.bundle_hash(),
        };
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .map_err(ActionCoordinatorError::Stage)?;
        Ok(staged)
    }

    /// Atomically replace one exact terminal decision bundle with its compact
    /// attribution receipt through the controller owner's registered action.
    pub fn prepare_and_stage_decision_compaction(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        candidate: &mush_core::DecisionCompactionCandidate,
    ) -> Result<StagedAuthoritativeAction, ActionCoordinatorError> {
        let plan = engine
            .plan_decision_compaction(candidate, permit.ingress())
            .map_err(|source| ActionCoordinatorError::DecisionCompaction(Box::new(source)))?;
        let (action, invocation, guards, transition) = plan.into_parts();
        validate_invocation_identity(permit.ingress(), &invocation)?;
        let token = deterministic_commit_token(permit.ingress())?;
        let (prepared, output) = engine
            .prepare_registered_decision_compaction_action(
                candidate,
                &action,
                &invocation,
                transition,
                token,
            )
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
        let staged = StagedAuthoritativeAction {
            token: bundle.token(),
            bundle_hash: bundle.bundle_hash(),
        };
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .map_err(ActionCoordinatorError::Stage)?;
        Ok(staged)
    }

    pub fn prepare_and_stage_decision_attempt(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        fence: &mush_core::DecisionCompletionFence,
        kind: mush_core::DecisionAttemptFailureKind,
        max_attempts: u8,
    ) -> Result<StagedDecisionAttempt, ActionCoordinatorError> {
        let plan = engine
            .plan_decision_attempt_failure(fence, kind, max_attempts, permit.ingress())
            .map_err(|source| ActionCoordinatorError::DecisionAttemptPlan(Box::new(source)))?;
        let (action, invocation, guards, transition) = plan.into_parts();
        validate_invocation_identity(permit.ingress(), &invocation)?;
        let token = deterministic_commit_token(permit.ingress())?;
        let (prepared, output, fallback_fence) = engine
            .prepare_registered_decision_attempt_action(
                fence,
                &action,
                &invocation,
                transition,
                token,
            )
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
        let staged = StagedAuthoritativeAction {
            token: bundle.token(),
            bundle_hash: bundle.bundle_hash(),
        };
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .map_err(ActionCoordinatorError::Stage)?;
        Ok(StagedDecisionAttempt {
            staged,
            fallback_fence,
        })
    }

    pub fn prepare_and_stage_decision_fallback_trigger(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        candidate: &mush_core::DecisionFallbackTriggerCandidate,
    ) -> Result<StagedDecisionFallbackTrigger, ActionCoordinatorError> {
        let plan = engine
            .plan_decision_fallback_trigger(candidate, permit.ingress())
            .map_err(|source| {
                ActionCoordinatorError::DecisionFallbackTriggerPlan(Box::new(source))
            })?;
        let (action, invocation, guards, transition) = plan.into_parts();
        validate_invocation_identity(permit.ingress(), &invocation)?;
        let token = deterministic_commit_token(permit.ingress())?;
        let (prepared, output, fallback_fence) = engine
            .prepare_registered_decision_fallback_trigger_action(
                candidate,
                &action,
                &invocation,
                transition,
                token,
            )
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
        let staged = StagedAuthoritativeAction {
            token: bundle.token(),
            bundle_hash: bundle.bundle_hash(),
        };
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .map_err(ActionCoordinatorError::Stage)?;
        Ok(StagedDecisionFallbackTrigger {
            staged,
            fallback_fence,
        })
    }

    pub fn prepare_and_stage_decision_cancellation(
        permit: Box<ActionPermit>,
        engine: &mut Engine,
        controller: &mush_core::DecisionControllerId,
        decision: mush_core::DecisionId,
        revision: u64,
    ) -> Result<StagedAuthoritativeAction, ActionCoordinatorError> {
        let plan = engine
            .plan_decision_cancellation(controller, decision, revision, permit.ingress())
            .map_err(|source| ActionCoordinatorError::DecisionCancellationPlan(Box::new(source)))?;
        let (action, invocation, guards, transition) = plan.into_parts();
        validate_invocation_identity(permit.ingress(), &invocation)?;
        let token = deterministic_commit_token(permit.ingress())?;
        let (prepared, output) = engine
            .prepare_registered_decision_cancellation_action(
                controller,
                &action,
                &invocation,
                transition,
                token,
            )
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
        let staged = StagedAuthoritativeAction {
            token: bundle.token(),
            bundle_hash: bundle.bundle_hash(),
        };
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .map_err(ActionCoordinatorError::Stage)?;
        Ok(staged)
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

fn require_current_guards(
    engine: &Engine,
    guards: &GuardSet,
) -> Result<(), ActionCoordinatorError> {
    let result = engine
        .revalidate_guards(guards)
        .map_err(ActionCoordinatorError::GuardAdmission)?;
    if result.is_current() {
        Ok(())
    } else {
        Err(ActionCoordinatorError::StaleGuards)
    }
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
    #[error("authoritative guard pre-admission failed: {0}")]
    GuardAdmission(#[source] mush_core::StagedActionAdmissionError),
    #[error("authoritative guards were stale before provider preparation")]
    StaleGuards,
    #[error("scheduled advance and action have different owners")]
    ScheduledOwnerMismatch,
    #[error("scheduled action ingress is invalid: {0}")]
    ScheduleIngress(#[source] Box<mush_core::ScheduleActionIngressError>),
    #[error("scheduled action planning failed: {0}")]
    SchedulePlan(#[source] Box<mush_core::ScheduleActionPlanError>),
    #[error("decision lease ingress is invalid: {0}")]
    DecisionLeaseIngress(#[source] Box<mush_core::DecisionLeaseIngressError>),
    #[error("decision lease planning failed: {0}")]
    DecisionLeasePlan(#[source] Box<mush_core::DecisionSourcePlanError>),
    #[error("decision response ingress is invalid: {0}")]
    DecisionResponseIngress(#[source] Box<mush_core::DecisionResponseIngressError>),
    #[error("decision response resolution failed: {0}")]
    DecisionResponseResolve(#[source] Box<mush_core::DecisionResponseResolveError>),
    #[error("decision fallback selection failed: {0}")]
    DecisionFallbackSelect(#[source] Box<mush_core::DecisionFallbackSelectionError>),
    #[error("decision resolving ingress is invalid: {0}")]
    DecisionResolvingIngress(#[source] Box<mush_core::DecisionResolvingIngressError>),
    #[error("decision resolving planning failed: {0}")]
    DecisionResolvingPlan(#[source] Box<mush_core::DecisionResolvingPlanError>),
    #[error("decision terminal ingress is invalid: {0}")]
    DecisionTerminalIngress(#[source] Box<mush_core::DecisionTerminalIngressError>),
    #[error("decision terminal planning failed: {0}")]
    DecisionTerminalPlan(#[source] Box<mush_core::DecisionTerminalPlanError>),
    #[error("decision compaction failed: {0}")]
    DecisionCompaction(#[source] Box<mush_core::DecisionCompactionError>),
    #[error("decision attempt ingress is invalid: {0}")]
    DecisionAttemptIngress(#[source] Box<mush_core::DecisionAttemptIngressError>),
    #[error("decision attempt planning failed: {0}")]
    DecisionAttemptPlan(#[source] Box<mush_core::DecisionAttemptPlanError>),
    #[error("decision fallback trigger is invalid: {0}")]
    DecisionFallbackTrigger(#[source] Box<mush_core::DecisionFallbackTriggerError>),
    #[error("decision fallback trigger planning failed: {0}")]
    DecisionFallbackTriggerPlan(#[source] Box<mush_core::DecisionFallbackTriggerPlanError>),
    #[error("decision cancellation ingress is invalid: {0}")]
    DecisionCancellationIngress(#[source] Box<mush_core::DecisionCancellationIngressError>),
    #[error("decision cancellation planning failed: {0}")]
    DecisionCancellationPlan(#[source] Box<mush_core::DecisionCancellationPlanError>),
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
