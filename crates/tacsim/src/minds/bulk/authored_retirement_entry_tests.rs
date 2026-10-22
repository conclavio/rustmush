//! Real ingress and observed selector handoff over the parent's genuine paid fork.
//! Core installation below models ACK; no server NoIntent/Unknown proof is forged.
use super::*;
use mush_core::submission_observation::{
    SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
};
use mush_core::{AuthoritativeSubmit, OfferExecutionContext, OfferSubmission};

struct Unresolved;
impl SubmissionDispositionReader for Unresolved {
    fn no_intent(&self) -> bool {
        false
    }
}
struct Runtime;
impl SubmissionRuntime for Runtime {
    fn observe(&self, _: &AuthoritativeSubmit) -> SubmissionRuntimeState {
        SubmissionRuntimeState::new(Arc::new(Unresolved), Box::new(()))
    }
}
fn configure(world: &mut PlayerWorld) -> TestResult {
    world.engine.set_submission_runtime(Arc::new(Runtime))?;
    world.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x74; 32])?,
        [],
    )?);
    Ok(())
}
fn prepare_selected(
    world: &mut PlayerWorld,
    submit: &AuthoritativeSubmit,
    number: u128,
) -> TestResult<PreparedComposedTxn> {
    let principal = PrincipalId::Object { id: submit.actor };
    let query =
        world
            .engine
            .legal_action_query(principal.clone(), submit.actor, submit.subject.clone())?;
    let offers = world.engine.project_legal_actions(&query)?;
    let projected = offers
        .projected_for_action(&submit.action)
        .ok_or("actual retirement offer")?;
    let resolved = world.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: RequestUid::from_bytes(number.to_be_bytes()),
            principal: principal.clone(),
            actor: submit.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: submit.action.key().owner.clone(),
                    namespace: CanonicalId::new(submit.action.key().id.as_str())?,
                },
                1,
                IdempotencyKey::Caller(submit.key.clone()),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: projected.offer(),
            subject: submit.subject.clone(),
            targets: submit.targets.clone(),
            parameters: submit.parameters.clone(),
        },
    )?;
    let (action, resolved, _) = resolved.into_authoritative_parts();
    check_eq!(action, submit.action);
    Ok(world
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            token(number),
        )?
        .0)
}
async fn source_free_entry(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        configure(&mut world)?;
        let child = roll(&world)?.mind(BUYER, "child")?.clone();
        let term: PaidTermRecord =
            serde_json::from_value(one(rows(&disk(&world), "roll", "roll_mind_paid_term"))?)?;
        world.engine.world.set_now(term.window().grace_end().get());
        assert!(matches!(
            invoke(
                &mut world,
                BUYER_CONN,
                REQUEST + 200,
                MindRequest::Read {
                    name: "child".into(),
                    revision: None
                }
            )?,
            MindResponse::Error { .. }
        ));
        let before = disk(&world);
        let result = invoke(
            &mut world,
            BUYER_CONN,
            REQUEST + 201,
            MindRequest::Remove {
                name: "child".into(),
            },
        )?;
        assert_eq!(
            result,
            MindResponse::RemovalRequested {
                name: "child".into()
            }
        );
        assert!(roll(&world)?.mind(BUYER, "child")? == &child && disk(&world) == before);
        assert!(
            matches!(
                invoke(
                    &mut world,
                    BUYER_CONN,
                    REQUEST + 202,
                    MindRequest::Remove {
                        name: "child".into()
                    }
                )?,
                MindResponse::Error { .. }
            ),
            "pending actor is backpressure, not a second selector"
        );
        let driven = world
            .engine
            .take_observed_authoritative_submit()
            .ok_or("actual captured selector")?;
        assert!(driven.is_observed());
        let submit = driven.request().clone();
        assert_eq!(submit.actor, BUYER);
        assert_eq!(submit.action.version(), 2);
        assert_eq!(submit.action.key().id.as_str(), "retire_authored_draft");
        assert_eq!(submit.subject, PerceivedRef::SelfActor);
        assert!(submit.targets.is_empty());
        let mut expected = parameters(&child);
        expected
            .as_object_mut()
            .ok_or("parameters")?
            .insert("mind".into(), json!(child.id().to_string()));
        assert_eq!(submit.parameters, expected);
        world
            .engine
            .requeue_observed_authoritative_submit(driven)
            .map_err(|_| "actual preflight requeue")?;
        let retried = world
            .engine
            .take_observed_authoritative_submit()
            .ok_or("same retained selector")?;
        assert!(retried.is_observed());
        assert_eq!(retried.request(), &submit);
        let prepared = prepare_selected(&mut world, retried.request(), 9_500_001)?;
        assert!(disk(&world) == before && roll(&world)?.mind(BUYER, "child").is_ok());
        let after = publish(&mut world, prepared)?;
        unchanged_business(&before, &after);
        assert!(roll(&world)?.mind(BUYER, "child").is_err());
        let operation = one(rows(&after, "roll", "roll_mind_authored_retirement"))?;
        let action: DefinitionRef =
            serde_json::from_value(operation.get("action").ok_or("retirement action")?.clone())?;
        assert_eq!(action, submit.action);
        let cold = PlayerWorld::boot_forks(f, &capture(&world, &after), true)?;
        assert!(roll(&cold)?.mind(BUYER, "child").is_err());
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        // The new name-only route refuses originals. Their existing lifecycle
        // remains direct and exact-CAS, with no commercial submission.
        no_source(&line(&mut world, "@mind/new plain")?);
        assert!(matches!(
            invoke(
                &mut world,
                BUYER_CONN,
                REQUEST + 203,
                MindRequest::Remove {
                    name: "plain".into()
                }
            )?,
            MindResponse::Error { .. }
        ));
        assert!(world.engine.take_observed_authoritative_submit().is_none());
        no_source(&line(&mut world, "@mind/remove plain")?);
        assert!(roll(&world)?.mind(BUYER, "plain").is_err());
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
async fn delayed_name_reuse(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        configure(&mut world)?;
        let (prepared, _) = super::super::super::prepare(
            &mut world,
            BUYER,
            9_500_010,
            json!({"parent_licence":"2","name":"reused"}),
        )?;
        publish(&mut world, prepared)?;
        let selected = roll(&world)?.mind(BUYER, "reused")?.clone();
        let events = line(&mut world, "@mind/remove reused")?;
        no_source(&events);
        assert!(PlayerWorld::private_text(&events, BUYER).contains("removal requested"));
        let driven = world
            .engine
            .take_observed_authoritative_submit()
            .ok_or("captured old selection")?;
        let exact = driven.request().clone();
        assert!(driven.is_observed());
        // Another real acknowledged cleanup and fork may reuse the label and
        // identical counters while the original immutable request is retained.
        let legacy = super::prepare(&mut world, BUYER, 9_500_011, parameters(&selected))?;
        publish(&mut world, legacy)?;
        let (replacement, _) = super::super::super::prepare(
            &mut world,
            BUYER,
            9_500_012,
            json!({"parent_licence":"2","name":"reused"}),
        )?;
        publish(&mut world, replacement)?;
        let replacement = roll(&world)?.mind(BUYER, "reused")?.clone();
        assert_eq!(replacement.name(), selected.name());
        assert_eq!(replacement.base(), selected.base());
        assert_ne!(replacement.id(), selected.id());
        world
            .engine
            .requeue_observed_authoritative_submit(driven)
            .map_err(|_| "restore exact old attempt")?;
        let retry = world
            .engine
            .take_observed_authoritative_submit()
            .ok_or("delayed exact retry")?;
        assert_eq!(retry.request(), &exact);
        let before = disk(&world);
        assert!(
            crate::test_checks::is_refusal(prepare_selected(
                &mut world,
                retry.request(),
                9_500_013
            )),
            "v2 captured identity refuses name/counter ABA"
        );
        assert!(disk(&world) == before && roll(&world)?.mind(BUYER, "reused")? == &replacement);
        // Current ordinary removal captures the new id and a distinct key.
        assert!(matches!(
            invoke(
                &mut world,
                BUYER_CONN,
                REQUEST + 210,
                MindRequest::Remove {
                    name: "reused".into()
                }
            )?,
            MindResponse::RemovalRequested { .. }
        ));
        let fresh = world
            .engine
            .take_observed_authoritative_submit()
            .ok_or("fresh explicit selection")?;
        assert_ne!(fresh.request().key, exact.key);
        let prepared = prepare_selected(&mut world, fresh.request(), 9_500_014)?;
        publish(&mut world, prepared)?;
        assert!(roll(&world)?.mind(BUYER, "reused").is_err());
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
pub(super) async fn exercise(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    story_phase("retirement.entry", || source_free_entry(f, saved))?.await?;
    story_phase("retirement.retry_identity", || delayed_name_reuse(f, saved))?.await
}
