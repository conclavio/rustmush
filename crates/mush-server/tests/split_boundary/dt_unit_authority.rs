//! Actual command, opaque offer, coordinator, durable store and restart path.
use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::{
    AuthoritativeSubmit, Engine, GOD, IngressEnvelope, ObjId, ObjKind, OfferExecutionContext,
    OfferKeyRing, OfferSecret, OfferSubmission, PerceivedRef, PrincipalId, World,
};
use mush_org::unit_authority::runtime::UnitAuthorityHandles;
use mush_org::unit_authority::{
    TitleLineage, UnitAssetFacts, UnitAuthorityConfig, UnitAuthorityFacts, UnitAuthorityWorldFacts,
    UnitGrantId, UnitGrantStatus,
};
use mush_org::{OrgModule, PrincipalId as OrgPrincipal};
use mush_server::action_coordinator::{ActionCoordinator, ActionPreflight};
use mush_server::modules::boot_modules;
use mush_server::persist::{batch_from_dirty, load_or_seed_world};
use mush_store::{AuthoritativeStore, InMemoryStore, ObjectStore};
use mush_testkit::TestWorld;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

// The injectable facts seam has its own Economy integration tests. Here actual
// World objects supply a minimal title fixture so the real coordinator/store
// behavior can be tested independently of market bootstrap.
#[derive(Debug)]
struct FactsFactory;
struct Facts<'a>(&'a World);
impl UnitAuthorityWorldFacts for FactsFactory {
    fn facts<'a>(&'a self, world: &'a World) -> Box<dyn UnitAuthorityFacts + 'a> {
        Box::new(Facts(world))
    }
}
impl UnitAuthorityFacts for Facts<'_> {
    fn principal_exists(&self, actor: ObjId) -> bool {
        self.0.get(actor).is_some_and(|o| o.kind.is_actor())
    }
    fn asset(&self, unit: ObjId) -> Option<UnitAssetFacts> {
        let row = self.0.get(unit)?;
        (row.kind == ObjKind::Thing).then_some(UnitAssetFacts {
            unit,
            owner: OrgPrincipal::Object(row.owner),
            lineage: TitleLineage {
                journal_seq: 1,
                effect_index: 0,
            },
            scope: None,
            active: true,
        })
    }
}
async fn fixture(
    store: &InMemoryStore,
    real_titles: bool,
) -> TestResult<(TestWorld, ObjId, ObjId)> {
    let (empty_world, registry) = mush_testkit::boot_world();
    let mut world = if real_titles {
        load_or_seed_world(store).await?
    } else {
        empty_world
    };
    let known_recipient = world
        .iter()
        .find(|row| row.name == "Recipient")
        .map(|row| row.id);
    let recipient = match known_recipient {
        Some(id) => id,
        None => world.create(ObjKind::Player, "Recipient", GOD, Some(mush_core::LIMBO))?,
    };
    let known_unit = world
        .iter()
        .find(|row| row.name == "Hull")
        .map(|row| row.id);
    let unit = match known_unit {
        Some(id) => id,
        None => world.create(ObjKind::Thing, "Hull", GOD, Some(mush_core::LIMBO))?,
    };
    world.set_now(100);
    let config = UnitAuthorityConfig {
        facts: if real_titles {
            Arc::new(mush_economy::unit_authority::EconomyUnitAuthorityWorldFacts)
        } else {
            Arc::new(FactsFactory)
        },
        requesters: BTreeSet::new(),
        dependencies: if real_titles {
            BTreeSet::from([mush_core::OwnerId::new("economy")?])
        } else {
            BTreeSet::new()
        },
    };
    let mut modules: Vec<Box<dyn mush_core::GameModule>> =
        vec![Box::new(OrgModule::new().with_unit_authority(config))];
    if real_titles {
        modules.push(Box::new(mush_economy::EconomyModule::new()));
    }
    let mut engine = Engine::with_modules(world, registry, modules)?;
    boot_modules(&mut engine, store).await?;
    engine.world.set_now(100);
    engine.set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x31; 32])?, [])?);
    let mut world = TestWorld::from_engine(engine);
    if real_titles
        && world
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .ok_or("economy")?
            .snapshot()
            .asset(unit)
            .is_none()
    {
        let _ = world.run(GOD, &format!("@asset/title #{}=#{}", unit.0, GOD.0));
        assert!(
            mush_economy::unit_authority::EconomyUnitAuthorityFacts::new(world.world())
                .asset(unit)
                .is_some()
        );
        persist_legacy(&mut world, store).await?;
    }
    let _ = world.drain_write_batch();
    Ok((world, recipient, unit))
}
async fn persist_legacy(world: &mut TestWorld, store: &InMemoryStore) -> TestResult {
    let pending = world.drain_write_batch();
    let mut batch = batch_from_dirty(world.world(), pending.dirty, 1, true);
    batch.module_ops = pending.module_ops;
    store.write(batch).await?;
    Ok(())
}
fn handles(world: &TestWorld) -> TestResult<UnitAuthorityHandles> {
    world
        .world()
        .module_state::<UnitAuthorityHandles>()
        .cloned()
        .ok_or_else(|| "unit handles missing".into())
}
fn submit(world: &mut TestWorld, actor: ObjId, line: &str) -> TestResult<Box<AuthoritativeSubmit>> {
    let events = world.run(actor, line);
    world
        .engine_mut()
        .take_authoritative_submit()
        .ok_or_else(|| {
            format!(
                "no submission: {:?}",
                events
                    .iter()
                    .map(|(_, event)| mush_testkit::text_of(event))
                    .collect::<Vec<_>>()
            )
            .into()
        })
}
async fn stage(
    world: &mut TestWorld,
    store: &InMemoryStore,
    submission: &AuthoritativeSubmit,
) -> TestResult<IngressEnvelope> {
    let ingress = ActionCoordinator::command_ingress(submission, 1)?;
    let ActionPreflight::Vacant(permit) =
        ActionCoordinator::preflight(store, ingress.clone()).await?
    else {
        return Err("request was not vacant".into());
    };
    let principal = PrincipalId::Object {
        id: submission.actor,
    };
    let query = world.engine().legal_action_query(
        principal.clone(),
        submission.actor,
        PerceivedRef::SelfActor,
    )?;
    let offers = world.engine().project_legal_actions(&query)?;
    let projected = offers
        .projected_for_action(&submission.action)
        .ok_or("no offered unit grant")?;
    let resolved = world.engine().resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: ingress.request(),
            principal,
            actor: submission.actor,
            idempotency: ingress.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: submission.targets.clone(),
            parameters: submission.parameters.clone(),
        },
    )?;
    ActionCoordinator::prepare_and_stage_resolved(permit, world.engine_mut(), resolved)?;
    Ok(ingress)
}

#[tokio::test]
async fn personal_loan_lost_ack_restart_exact_retry_and_immediate_revoke() -> TestResult {
    let store = InMemoryStore::new();
    let (mut world, recipient, unit) = fixture(&store, true).await?;
    let h = handles(&world)?;
    let submission = submit(
        &mut world,
        GOD,
        &format!("@unitgrant/issue #{}=#{} deploy", unit.0, recipient.0),
    )?;
    let ingress = stage(&mut world, &store, &submission).await?;
    assert!(
        world
            .engine()
            .action_state()
            .get(&h.registry, &())?
            .is_none(),
        "preparation publishes nothing"
    );
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged unit grant")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    assert!(matches!(
        ActionCoordinator::preflight(&store, ingress.clone()).await?,
        ActionPreflight::Pending { .. }
    ));
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert!(receipt.matches_bundle(&bundle));
    assert!(
        world
            .engine()
            .action_state()
            .get(&h.registry, &())?
            .is_none(),
        "missing acknowledgement cannot install or publish"
    );
    // The old process loses its acknowledgement. It fails stop; the next
    // composition obtains the durable post-image through normal store boot.
    drop(installation);
    drop(world);
    let (mut restarted, _, _) = fixture(&store, true).await?;
    let h = handles(&restarted)?;
    let registry = restarted
        .engine()
        .action_state()
        .get(&h.registry, &())?
        .ok_or("restarted registry")?;
    assert_eq!(registry.grants.len(), 1);
    assert_eq!(
        registry.grants.get(&UnitGrantId(1)).ok_or("loan")?.status,
        UnitGrantStatus::Active
    );
    let ActionPreflight::Finalized(retry) = ActionCoordinator::preflight(&store, ingress).await?
    else {
        return Err("lost-ack retry did not return its exact durable receipt".into());
    };
    assert_eq!(retry.token(), bundle.token());
    assert_eq!(retry.outcome(), receipt.outcome());
    let permission = restarted.run(recipient, &format!("@unitgrant/check #{}=deploy", unit.0));
    assert!(
        permission
            .iter()
            .any(|(_, event)| mush_testkit::text_of(event).contains("currently allowed"))
    );
    let original_lineage =
        mush_economy::unit_authority::EconomyUnitAuthorityFacts::new(restarted.world())
            .asset(unit)
            .ok_or("title")?
            .lineage;
    for owner in [recipient, GOD] {
        let _ = restarted.run(GOD, &format!("@asset/transfer #{}=#{}", unit.0, owner.0));
        let title = mush_economy::unit_authority::EconomyUnitAuthorityFacts::new(restarted.world())
            .asset(unit)
            .ok_or("transferred title")?;
        assert_eq!(title.owner, OrgPrincipal::Object(owner));
        persist_legacy(&mut restarted, &store).await?;
    }
    assert_ne!(
        mush_economy::unit_authority::EconomyUnitAuthorityFacts::new(restarted.world())
            .asset(unit)
            .ok_or("returned title")?
            .lineage,
        original_lineage
    );
    let permission = restarted.run(recipient, &format!("@unitgrant/check #{}=deploy", unit.0));
    assert!(
        permission
            .iter()
            .any(|(_, event)| mush_testkit::text_of(event).contains("unavailable")),
        "returning title cannot revive the old grant"
    );
    let submission = submit(&mut restarted, GOD, "@unitgrant/revoke 1")?;
    stage(&mut restarted, &store, &submission).await?;
    let (revocation, installation) = restarted
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged revoke")?
        .into_parts();
    store
        .reserve_authoritative_intent((*revocation).clone())
        .await?;
    store
        .reconcile_authoritative(revocation.token(), revocation.bundle_hash())
        .await?;
    restarted
        .engine_mut()
        .install_composed_action(installation)?;
    let registry = restarted
        .engine()
        .action_state()
        .get(&h.registry, &())?
        .ok_or("revoked registry")?;
    assert_eq!(
        registry.grants.get(&UnitGrantId(1)).ok_or("loan")?.status,
        UnitGrantStatus::Revoked
    );
    assert_eq!(registry.next_id, 2);
    // A completed request does not occupy the volatile pending slot for five minutes.
    assert!(
        submit(
            &mut restarted,
            GOD,
            &format!("@unitgrant/issue #{}=#{} operate_crew", unit.0, recipient.0)
        )
        .is_ok()
    );
    Ok(())
}

#[tokio::test]
async fn final_title_recheck_and_old_selector_refuse_without_writes() -> TestResult {
    let store = InMemoryStore::new();
    let (mut world, recipient, unit) = fixture(&store, false).await?;
    let h = handles(&world)?;
    let old = submit(
        &mut world,
        GOD,
        &format!("@unitgrant/issue #{}=#{} deploy", unit.0, recipient.0),
    )?;
    let newer = submit(
        &mut world,
        GOD,
        &format!("@unitgrant/issue #{}=#{} operate_crew", unit.0, recipient.0),
    )?;
    assert!(
        stage(&mut world, &store, &old).await.is_err(),
        "a taken selector cannot become its replacement request"
    );
    stage(&mut world, &store, &newer).await?;
    world.world_mut().chown(unit, recipient)?;
    assert!(
        world.engine_mut().take_staged_action().is_err(),
        "the final revision fence rechecks current title"
    );
    assert!(!world.engine().has_staged_action());
    assert!(
        world
            .engine()
            .action_state()
            .get(&h.registry, &())?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn final_clock_recheck_refuses_expiry_without_record_changes() -> TestResult {
    let store = InMemoryStore::new();
    let (mut world, recipient, unit) = fixture(&store, false).await?;
    let h = handles(&world)?;
    let submission = submit(
        &mut world,
        GOD,
        &format!(
            "@unitgrant/issue #{}=#{} deploy until=101",
            unit.0, recipient.0
        ),
    )?;
    stage(&mut world, &store, &submission).await?;
    // No title, grant or pending-input revision changes: only the authoritative
    // clock reaches the exclusive end of this candidate's validity window.
    world.world_mut().set_now(101);
    assert!(world.engine_mut().take_staged_action().is_err());
    assert!(!world.engine().has_staged_action());
    assert!(
        world
            .engine()
            .action_state()
            .get(&h.registry, &())?
            .is_none()
    );
    Ok(())
}
