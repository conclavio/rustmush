//! Actual TacsimRuleset load + registered recovery. The unresolved embedding
//! adapter exposes no successful/refused writer proof. Complete owner installs
//! still use the actual core composed test boundary, not a certification stub.
use super::*;
use crate::minds::bulk::{registry_recovery, world};

struct Unresolved;
impl mush_core::submission_observation::SubmissionDispositionReader for Unresolved {
    fn no_intent(&self) -> bool {
        false
    }
}
impl mush_core::submission_observation::SubmissionRuntime for Unresolved {
    fn observe(
        &self,
        _: &mush_core::AuthoritativeSubmit,
    ) -> mush_core::submission_observation::SubmissionRuntimeState {
        mush_core::submission_observation::SubmissionRuntimeState::new(Arc::new(Self), Box::new(()))
    }
}
fn command(engine: &mut Engine, line: &str) {
    engine.step(
        mush_core::Inbound::Cmd {
            conn: 0,
            actor: GOD,
            line: line.into(),
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
}
fn real_engine(f: &Fixture) -> TestResult<Box<Engine>> {
    let (mut engine, _) =
        composed_engine(&f.resources, f.accounts.clone(), CommerceMode::Active, true)?;
    engine.set_submission_runtime(Arc::new(Unresolved))?;
    Ok(engine)
}
fn continue_recovery(f: &mut Fixture, binding: Binding) -> TestResult<TicketId> {
    command(&mut f.engine, "@recovery-pulse");
    command(&mut f.engine, "@registry/continue 1");
    let ticket = TicketId::new(1)?;
    let actual = f.artifacts()?.registry.get(ticket)?;
    assert_eq!(actual.binding(), Some(binding));
    assert_eq!(actual.transition(), Transition::Recover);
    let driven = f
        .engine
        .take_observed_authoritative_submit()
        .ok_or("actual owner continuation")?;
    let (request, _unresolved_driver) = driven.into_parts();
    assert_eq!(request.actor, binding.character);
    assert_eq!(request.action, f.action(Transition::Recover)?);
    assert_eq!(request.subject, PerceivedRef::SelfActor);
    assert!(request.targets.is_empty());
    assert_eq!(
        request.parameters,
        json!({}),
        "no measurements or identity inputs"
    );
    // Re-entering with an unresolved attempt never manufactures another ticket
    // or retries the request based on the absence of an acknowledgement.
    command(&mut f.engine, "@registry/continue 1");
    assert!(f.engine.take_observed_authoritative_submit().is_none());
    f.artifacts()?.registry.check(ticket, &actual)?;
    Ok(ticket)
}
pub(super) fn cold_restart(f: &mut Fixture, binding: Binding) -> TestResult<TicketId> {
    let before = f.run(binding)?;
    let mut reboot = real_engine(f)?;
    load(&mut reboot, &f.disk)?;
    f.engine = reboot;
    assert_eq!(
        f.run(binding)?,
        before,
        "production hydration preserves exact Registry quota"
    );
    assert!(f.artifacts()?.registry.pristine());
    continue_recovery(f, binding)
}

#[test]
fn real_tacsim_boot_waits_for_final_audits_and_recovers_queued_or_paid_only_after_ack() -> TestResult
{
    for paid in [false, true] {
        let mut f = Fixture::new()?;
        let (binding, ready) = f.admitted()?;
        if paid {
            drop(f.grant(binding, &ready, 4)?);
        }
        drop(ready); // The old test process owns/stops every actual continuation.
        let before = f.run(binding)?;
        let roots = f.roots()?;
        let retained_job: RatingJob = f.retained("rating_jobs")?;
        let mut reboot = real_engine(&f)?;
        let mut typed = Vec::new();
        let mut modules = HashMap::<SmolStr, ModuleBootData>::new();
        for ((owner, key), value) in &f.disk {
            if mush_core::action_state::is_action_state_persistence_key(key) {
                typed.push(ActionStateBootRecord::new(
                    owner.as_str(),
                    key.as_str(),
                    value.clone(),
                ));
            } else {
                modules
                    .entry(owner.clone().into())
                    .or_default()
                    .kv
                    .push((key.clone().into(), value.clone()));
            }
        }
        reboot.hydrate_action_state(typed)?;
        reboot.module_loaded(
            "tacsim",
            modules.remove("tacsim").unwrap_or_default(),
            &mut Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        )?;
        f.engine = reboot;
        assert_eq!(f.run(binding)?, before);
        command(&mut f.engine, "@recovery-pulse");
        command(&mut f.engine, "@registry/continue 1");
        assert!(
            f.artifacts()?.registry.pristine(),
            "closed Roll cannot mint a recovery ticket"
        );
        assert!(f.engine.take_observed_authoritative_submit().is_none());
        for owner in ["roll", "economy"] {
            f.engine.module_loaded(
                owner,
                modules.remove(owner).unwrap_or_default(),
                &mut Outbox::new(),
                &mut mush_core::engine::Effects::new(),
            )?;
        }
        f.engine
            .all_modules_loaded(&mut Outbox::new(), &mut mush_core::engine::Effects::new())?;
        let ticket = continue_recovery(&mut f, binding)?;
        assert_eq!(f.run(binding)?, before);
        assert_eq!(f.roots()?, roots);
        let prepared = f.prepare(Transition::Recover, ticket, binding.job, 8)?;
        assert_eq!(f.run(binding)?, before, "prepare has no owner mutation");
        install(&mut f.engine, &mut f.disk, prepared)?;
        // Real Artifacts apply produced the ACK; let the actual service consume
        // it and validate both typed and private-quota post-images.
        assert!(f.artifacts()?.registry.peek_acknowledged().is_some());
        command(&mut f.engine, "@recovery-pulse");
        command(&mut f.engine, "@registry/continue 1");
        assert!(f.artifacts()?.registry.peek_acknowledged().is_none());
        assert!(f.artifacts()?.registry.get(ticket).is_err());
        let service = world::service(&f.engine.world)?;
        let runtime = service.0.lock().map_err(module_error)?;
        assert!(!runtime.recovery.pending());
        assert!(
            runtime.registry.accounts().next().is_none(),
            "only exact terminal readback releases custody"
        );
        drop(runtime);
        let recovered = f.run(binding)?;
        assert_eq!(recovered.status, Status::Interrupted);
        assert_eq!(recovered.charged_unit_ms, before.charged_unit_ms);
        assert!(recovered.flight.is_none() && recovered.result.is_none());
        let after: RatingJob = f.retained("rating_jobs")?;
        assert_eq!(after.subject(), retained_job.subject());
        assert_eq!(after.compatibility(), retained_job.compatibility());
        assert!(
            !f.roots()?
                .contains(&format!("v1/rating_pending/{}", binding.job))
        );
        assert!(
            f.roots()?
                .contains(&format!("v1/rating_job/{}", binding.job))
        );
        assert!(
            f.disk
                .keys()
                .all(|(_, key)| !key.starts_with("action-state/capacity/v1/"))
        );
        let mut final_boot = real_engine(&f)?;
        load(&mut final_boot, &f.disk)?;
        assert_eq!(
            final_boot
                .world
                .module_state::<BulkStore>()
                .ok_or("quota")?
                .account_run(binding.account, binding.run)?,
            &recovered
        );
        assert!(
            !world::service(&final_boot.world)?
                .0
                .lock()
                .map_err(module_error)?
                .recovery
                .pending()
        );
    }
    Ok(())
}

#[test]
fn missing_account_keeps_recovery_custody_after_complete_owner_audit() -> TestResult {
    let mut f = Fixture::new()?;
    let (binding, ready) = f.admitted()?;
    drop(f.grant(binding, &ready, 4)?);
    drop(ready);
    f.accounts.values.lock().map_err(module_error)?.remove(&GOD);
    let mut reboot = real_engine(&f)?;
    load(&mut reboot, &f.disk)?;
    f.engine = reboot;
    let before = f.run(binding)?;
    let roots = f.roots()?;
    command(&mut f.engine, "@recovery-pulse");
    command(&mut f.engine, "@registry/continue 1");
    assert!(f.artifacts()?.registry.pristine());
    assert!(f.engine.take_observed_authoritative_submit().is_none());
    assert_eq!(f.run(binding)?, before);
    assert_eq!(f.roots()?, roots);
    assert_eq!(
        world::service(&f.engine.world)?
            .0
            .lock()
            .map_err(module_error)?
            .registry
            .accounts()
            .collect::<Vec<_>>(),
        vec![binding.account]
    );
    assert_eq!(
        registry_recovery::unavailable(&f.engine.world, binding),
        Some(registry_recovery::Unavailable::MissingAccount)
    );
    assert!(
        f.disk
            .keys()
            .any(|(_, key)| key.starts_with("action-state/capacity/v1/"))
    );
    Ok(())
}

#[path = "registry_completeness_tests.rs"]
mod completeness;

#[test]
fn unavailable_boot_principals_have_explicit_retained_dispositions() -> TestResult {
    use registry_recovery::{Unavailable, unavailable};
    let mut world = World::new_minimal();
    let binding = Binding {
        job: CommitToken::from_bytes([31; 16]),
        account: AccountId::mint("recovery-fixture", 1)?,
        character: ObjId(u32::MAX),
        run: RunId(1),
    };
    assert_eq!(
        unavailable(&world, binding),
        Some(Unavailable::MissingActor)
    );
    let binding = Binding {
        character: GOD,
        ..binding
    };
    assert_eq!(
        unavailable(&world, binding),
        Some(Unavailable::MissingAccount)
    );
    world
        .get_mut(GOD)
        .ok_or("actor")?
        .flags
        .insert(mush_core::Flags::GOING);
    assert_eq!(unavailable(&world, binding), Some(Unavailable::GoingActor));
    let actor = world.get_mut(GOD).ok_or("actor")?;
    actor.flags.remove(mush_core::Flags::GOING);
    actor.kind = mush_core::ObjKind::Thing;
    assert_eq!(unavailable(&world, binding), Some(Unavailable::NotPlayer));
    Ok(())
}
