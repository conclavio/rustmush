use super::*;
use mush_core::action::{
    ActionInvocationDraft, ActionOfferId, PreparedAuthoritativeAction, PreparedCausalRef,
};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use mush_core::composed::PreparedComposedTxn;
use mush_core::{
    ActionInvocation, CanonicalHash, GuardSet, IdempotencyKey, IdempotencyScope,
    OfferExecutionContext, OfferSubmission, PerceivedRef, PrincipalId, RequestUid,
    ScopedIdempotencyKey,
};

pub(super) type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);
pub(super) fn token(n: u128) -> CommitToken {
    CommitToken::from_bytes(n.to_be_bytes())
}
fn request(n: u128, actor: ObjId, owner: &str, parameters: Value) -> Result<ActionInvocation> {
    let request = RequestUid::from_bytes(n.to_be_bytes());
    let principal = PrincipalId::Object { id: actor };
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([7; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new(owner)?,
                namespace: mush_core::CanonicalId::new("hardware_flow")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
pub(super) struct Planned {
    pub action: DefinitionRef,
    pub invocation: ActionInvocation,
    pub guards: GuardSet,
    pub prepared: Prepared,
    pub commit: CommitToken,
}
impl Rig {
    pub(super) fn action(&self, owner: &str, name: &str) -> Result<DefinitionRef> {
        self.world()
            .engine()
            .registry
            .definitions()
            .ok_or("definitions")?
            .iter()
            .map(|d| d.reference())
            .find(|r| {
                r.key().owner.as_str() == owner
                    && r.key().kind.as_str() == "action"
                    && r.key().id.as_str() == name
            })
            .cloned()
            .ok_or_else(|| "registered hardware action".into())
    }
    pub(super) fn plan(
        &mut self,
        owner: &str,
        name: &str,
        actor: ObjId,
        parameters: Value,
    ) -> Result<Planned> {
        self.serial += 1;
        let n = self.serial;
        let exact = self.action(owner, name)?;
        let req = request(n, actor, owner, parameters.clone())?;
        let e = self.world_mut().engine_mut();
        e.set_offer_keys(mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x46; 32])?,
            [],
        )?);
        let principal = PrincipalId::Object { id: actor };
        let query = e.legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
        let offers = e.project_legal_actions(&query)?;
        let offer = offers
            .projected_for_action(&exact)
            .ok_or("actual signed hardware offer")?;
        let resolved = e.resolve_legal_action(
            &offers,
            OfferExecutionContext {
                request: req.request(),
                principal,
                actor,
                idempotency: req.idempotency().clone(),
                viewer_scope: query.viewer_scope(),
                now: None,
            },
            OfferSubmission {
                offer: offer.offer(),
                subject: PerceivedRef::SelfActor,
                targets: vec![],
                parameters,
            },
        )?;
        let (action, resolved, guards) = resolved.into_authoritative_parts();
        assert_eq!(action, exact);
        let invocation = resolved.invocation().clone();
        let prepared = e.prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            token(n),
        )?;
        Ok(Planned {
            action,
            invocation,
            guards,
            prepared,
            commit: token(n),
        })
    }
    pub(super) fn perform(
        &mut self,
        owner: &str,
        name: &str,
        actor: ObjId,
        parameters: Value,
    ) -> Result<(CommitToken, Value)> {
        let plan = self.plan(owner, name, actor, parameters)?;
        let commit = plan.commit;
        let result = self.install(plan)?;
        Ok((commit, result))
    }
    pub(super) fn initialize_roll(&mut self) -> Result {
        self.perform("roll", "initialize_commerce", GOD, json!({}))?;
        Ok(())
    }
    pub(super) fn stage(&mut self, plan: Planned) -> Result<Value> {
        let outcome_copy = plan.prepared.1.outcome().data().clone();
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = plan.prepared.1.into_parts();
        definitions.push(plan.action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: plan.commit,
            request: plan.invocation.request(),
            idempotency: plan.invocation.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: plan.invocation.request_hash(),
            plan_hash: CanonicalHash::from_bytes([2; 32]),
            provider: plan.action.key().owner.clone(),
            action: plan.action,
            cause: PreparedCausalRef::new(plan.invocation.request(), None, 0)?,
            definitions,
            writes: plan.prepared.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(plan.commit)),
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        })?;
        self.world_mut().engine_mut().stage_authoritative_action(
            plan.prepared.0,
            plan.guards,
            bundle,
        )?;
        Ok(outcome_copy)
    }
    pub(super) fn install(&mut self, plan: Planned) -> Result<Value> {
        let outcome = self.stage(plan)?;
        self.take_install()?;
        Ok(outcome)
    }
    pub(super) fn take_install(&mut self) -> Result {
        let staged = self
            .world_mut()
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged hardware action")?;
        let (bundle, install) = staged.into_parts();
        let mut ops = Vec::new();
        for write in bundle.writes() {
            let op = match write {
                ReplayWrite::ModuleJsonSet {
                    namespace,
                    key,
                    value,
                } => Some(ModuleOp {
                    ns: namespace.as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: key.as_str().into(),
                        value: value.clone(),
                    },
                }),
                ReplayWrite::ModuleDelete { namespace, key } => Some(ModuleOp {
                    ns: namespace.as_str().into(),
                    op: ModuleWrite::Del {
                        key: key.as_str().into(),
                    },
                }),
                ReplayWrite::LedgerChunkSet { write } => Some(ModuleOp {
                    ns: write.namespace().as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: write.key().as_str().into(),
                        value: serde_json::to_value(write.chunk())?,
                    },
                }),
                ReplayWrite::ObjectSet { .. } | ReplayWrite::ObjectAllocatorSet { .. } => None,
                ReplayWrite::ObjectDelete { .. } => {
                    return Err("hardware action unexpectedly deleted an object".into());
                }
            };
            if let Some(op) = op {
                ops.push(op)
            }
        }
        // Exact actual union is the durable ACK boundary in this in-process rig.
        fold_module_ops_into(&mut self.boot, ops);
        self.world_mut()
            .engine_mut()
            .install_composed_action(install)?;
        Ok(())
    }
    pub(super) fn progress(&mut self, name: &str, hours: usize) -> Result {
        for hour in 0..hours {
            let utc = self.world().world().now();
            self.world_mut()
                .world_mut()
                .set_now(utc.checked_add(3600).ok_or("clock overflow")?);
            self.flush();
            let page = self.world().engine().discover_scheduled_actions(
                mush_core::ScheduleDriveCause::Live,
                None,
                256,
            )?;
            let candidates: Vec<_> = page
                .candidates()
                .iter()
                .filter(|c| c.action().key().id.as_str() == name)
                .cloned()
                .collect();
            assert_eq!(candidates.len(), 1, "one real due production schedule");
            let candidate = &candidates[0];
            let ingress =
                mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 1)?;
            let plan = self
                .world()
                .engine()
                .plan_scheduled_action(candidate, &ingress)?;
            let (action, invocation, guards) = plan.into_parts();
            self.serial += 1;
            let commit = token(self.serial);
            let prepared = if name == "advance_core_integration" {
                // Industry is an ordinary dependency of this test crate. Its
                // quiet fixed-label events are captured only for this call.
                let diagnostic = progress_diagnostic::Phase::default();
                tracing::subscriber::with_default(diagnostic.clone(), || {
                    self.world_mut()
                        .engine_mut()
                        .prepare_registered_composed_scheduled_authoritative_action(
                            candidate,
                            &invocation,
                            commit,
                        )
                })
                .map_err(|_| diagnostic.error(hour))?
            } else {
                self.world_mut()
                    .engine_mut()
                    .prepare_registered_composed_scheduled_authoritative_action(
                        candidate,
                        &invocation,
                        commit,
                    )?
            };
            self.install(Planned {
                action,
                invocation,
                guards,
                prepared,
                commit,
            })?;
        }
        Ok(())
    }
}

/// Test-only capture: an exact whitelist, first failing boundary, no error or
/// event payload formatter. The actual prepare still has to succeed.
mod progress_diagnostic {
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub(super) struct Phase(Arc<Mutex<Option<&'static str>>>);
    impl Phase {
        pub(super) fn error(&self, hour: usize) -> String {
            let stage = self
                .0
                .lock()
                .ok()
                .and_then(|value| *value)
                .unwrap_or("unobserved");
            format!("Hardware manufacture progress preparation refused: stage={stage}, hour={hour}")
        }
    }
    struct PhaseVisitor(Option<&'static str>);
    impl tracing::field::Visit for PhaseVisitor {
        fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() != "stage" {
                return;
            }
            self.0 = match value {
                "provider-entry" => Some("provider-entry"),
                "provider-selector" => Some("provider-selector"),
                "provider-order" => Some("provider-order"),
                "provider-clock" => Some("provider-clock"),
                "provider-schedule" => Some("provider-schedule"),
                "provider-owner" => Some("provider-owner"),
                "provider-owner-output" => Some("provider-owner-output"),
                "provider-order-write" => Some("provider-order-write"),
                "provider-schedule-write" => Some("provider-schedule-write"),
                "provider-core-capacity" => Some("provider-core-capacity"),
                "owner-readiness" => Some("owner-readiness"),
                "owner-current-order" => Some("owner-current-order"),
                "owner-action-clock" => Some("owner-action-clock"),
                "owner-cleanup" => Some("owner-cleanup"),
                "owner-progress" => Some("owner-progress"),
                "owner-postimage" => Some("owner-postimage"),
                _ => None,
            };
        }
    }
    impl tracing::Subscriber for Phase {
        fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
            m.target() == "core_integration_preparation"
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if event.metadata().target() != "core_integration_preparation" {
                return;
            }
            let mut visitor = PhaseVisitor(None);
            event.record(&mut visitor);
            if let Ok(mut value) = self.0.lock() {
                if value.is_none() {
                    *value = visitor.0;
                }
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
}
