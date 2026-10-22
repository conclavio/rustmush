//! Actual same-slot major fee tests. New line/certificate authority is Roll-owned.
use super::*;

fn major_rules() -> Result<Rules> {
    let mut r = derived_rules()?;
    let d = descriptor("action", MAJOR_ACTION_ID)?;
    let action = DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, d.clone())?;
    r.declarations.push(d);
    r.major = Some(action);
    Ok(r)
}
// Keep repeated actual Engine boot results out of the caller's default stack.
#[inline(never)]
fn major_fixture() -> Result<Box<Fixture>> {
    Fixture::initialize(Fixture::empty_rules(major_rules()?)?)
}
#[inline(never)]
fn empty_fixture(rules: Rules) -> Result<Box<Fixture>> {
    Fixture::empty_rules(rules)
}
#[inline(never)]
fn registered(
    modules: Vec<Box<dyn GameModule>>,
) -> std::result::Result<Box<mush_core::Engine>, ModError> {
    mush_core::Engine::with_modules(World::new_minimal(), mush_core::Registry::new(), modules)
        .map(Box::new)
}
fn exact_receipt(f: &Fixture, n: u8) -> Result<UpdateAuditSubject> {
    f.state()
        .update_receipt_audit()
        .receipts()?
        .find(|r| r.commit == token(n))
        .cloned()
        .ok_or_else(|| "actual fee receipt".into())
}
fn checkpoint(audit: &UpdateReceiptAudit) -> Result<Vec<crate::checkpoint::IdemRow>> {
    Ok(audit
        .receipts()?
        .map(|s| crate::checkpoint::IdemRow {
            ns: receipt::namespace(s.kind).into(),
            key: s.commit.to_hex().into(),
            plan_hash: s.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
            tx: s.transaction,
            posted_at: s.posted_at_utc,
            seq: s.journal_seq,
        })
        .collect())
}

#[test]
fn major_fee_uses_actual_owner_and_keeps_both_compatible_protocols_and_title() -> Result {
    let mut f = major_fixture()?;
    let title = f
        .state()
        .snapshot
        .asset(f.shell)
        .ok_or("birth title")?
        .clone();
    let birth = serde_json::to_value(f.state().filing_receipt_audit())?;
    for (action, n) in [
        (f.rules.update.clone(), 2),
        (f.rules.derived.clone().ok_or("derived")?, 3),
    ] {
        let prepared = f.prepare(&action, f.actor, n)?;
        f.publish(action, n, prepared)?;
    }
    let original_json = serde_json::to_value(exact_receipt(&f, 2)?)?;
    let dependent_json = serde_json::to_value(exact_receipt(&f, 3)?)?;
    check!(original_json.get("kind").is_none());
    check_eq!(dependent_json.get("kind"), Some(&json!("dependent_v1")));
    let action = f.rules.major.clone().ok_or("major")?;
    let before = f.state().snapshot.clone();
    let prepared = f.prepare(&action, f.actor, 4)?;
    check_eq!(f.state().snapshot, before);
    let writes = f.stage(action.clone(), 4, prepared)?;
    check_eq!(f.state().snapshot, before);
    let FixtureEffect::Update(subject) =
        prepared_receipt(&writes, &f.rules, &action, token(4), f.actor)?.effect
    else {
        return Err("actual major fee output".into());
    };
    check_eq!(subject.kind, Some(UpdateKind::OriginalMajorV1));
    f.take_install()?;
    f.state().update_receipt_audit().verify(&subject)?;
    let tx = f
        .state()
        .transaction(subject.transaction)
        .ok_or("native major transaction")?;
    check_eq!(tx.idempotency.namespace.as_str(), receipt::MAJOR_NAMESPACE);
    check_eq!(receipt::subject_of(tx)?, subject);
    check_eq!(tx.description.as_str(), "Original mind major filing");
    check!(
        tx.lines
            .iter()
            .all(|line| line.memo.as_str() == "original mind major")
    );
    check!(!writes.iter().any(
        |w| matches!(w, ReplayWrite::ModuleJsonSet{key,..} if key.as_str().starts_with("asset:"))
    ));
    check_eq!(f.state().snapshot.asset(f.shell), Some(&title));
    check_eq!(
        serde_json::to_value(f.state().filing_receipt_audit())?,
        birth
    );
    for (n, expected) in [(2, &original_json), (3, &dependent_json)] {
        check_eq!(&serde_json::to_value(exact_receipt(&f, n)?)?, expected);
    }
    check_eq!(
        f.state()
            .snapshot
            .account(AccountId(1))
            .ok_or("payer")?
            .posted_balance,
        Amount::from_minor(9600)
    );
    check_eq!(
        f.state()
            .snapshot
            .account(AccountId(2))
            .ok_or("sink")?
            .posted_balance,
        Amount::from_minor(400)
    );
    for kind in [None, Some(UpdateKind::DependentV1)] {
        let mut crossed = subject.clone();
        crossed.kind = kind;
        check!(f.state().update_receipt_audit().verify(&crossed).is_err());
    }
    for kind in [Value::Null, json!("major"), json!({"original_major_v1":{}})] {
        let mut bad = serde_json::to_value(&subject)?;
        bad["kind"] = kind;
        check!(serde_json::from_value::<UpdateAuditSubject>(bad).is_err());
    }
    check_eq!(
        crate::contract::managed::require_legacy_transaction(tx, &f.state().snapshot),
        Err(crate::EconomyError::ManagedContract)
    );
    struct Permit;
    impl AuthEval for Permit {
        fn authorize(&self, _: &PrincipalId, _: &Account, _: Amount) -> bool {
            true
        }
    }
    let request = RegistryCompatibleUpdate {
        charter: f.shell,
        title_revision: title.revision,
        payer: AccountId(1),
    };
    let ordinary = plan(
        Some(UpdateKind::OriginalMajorV1),
        &policy(100),
        f.actor,
        token(5),
        &request,
    );
    let gate = f.state().key_gate();
    let env = ValidateEnv {
        auth: &Permit,
        custody: &NoCustody,
        facts: &NoFacts,
        now: f.world.world().now(),
        ic_now: f.world.world().game_now_unix(),
        next_hold_id: f.state().next_hold,
        spent: BTreeMap::new(),
        keys: &gate,
    };
    check!(matches!(
        crate::engine::validate(&ordinary, &f.state().snapshot, &env),
        PlanResult::Refused(crate::EconomyError::ManagedContract)
    ));
    Ok(())
}

#[test]
fn major_fee_complete_cold_inventory_rejects_missing_disabled_and_crossed_protocols() -> Result {
    let mut f = major_fixture()?;
    for (action, n) in [
        (f.rules.update.clone(), 2),
        (f.rules.derived.clone().ok_or("derived")?, 3),
        (f.rules.major.clone().ok_or("major")?, 4),
    ] {
        let p = f.prepare(&action, f.actor, n)?;
        f.publish(action, n, p)?;
    }
    let mut cold = empty_fixture(major_rules()?)?;
    cold.world.try_modules_loaded(f.boot.clone())?;
    check_eq!(cold.state().snapshot, f.state().snapshot);
    check_eq!(
        cold.state().update_receipt_audit(),
        f.state().update_receipt_audit()
    );
    let mut old_only = empty_fixture(derived_rules()?)?;
    check!(old_only.world.try_modules_loaded(f.boot.clone()).is_err());
    let audit = f.state().update_receipt_audit();
    audit.require_protocols(true, true, true)?;
    for enabled in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        check!(
            audit
                .require_protocols(enabled.0, enabled.1, enabled.2)
                .is_err()
        );
    }
    let rows = checkpoint(audit)?;
    check!(audit.checkpoint_complete(&rows));
    for i in 0..rows.len() {
        let mut missing = rows.clone();
        missing.remove(i);
        check!(!audit.checkpoint_complete(&missing));
        let mut crossed = rows.clone();
        let row = crossed.get_mut(i).ok_or("checkpoint")?;
        row.ns = if row.ns.as_str() == receipt::MAJOR_NAMESPACE {
            receipt::NAMESPACE
        } else {
            receipt::MAJOR_NAMESPACE
        }
        .into();
        check!(!audit.checkpoint_complete(&crossed));
    }
    check!(!UpdateReceiptAudit::default().checkpoint_complete(&rows));
    let major = exact_receipt(&f, 4)?;
    let mut replay = UpdateReceiptAudit::default();
    let mut transactions: Vec<_> = f.state().transactions.values().collect();
    transactions.sort_by_key(|t| t.journal_seq);
    for tx in transactions {
        replay.observe(tx);
    }
    replay.finish_replay()?;
    replay.verify(&major)?;
    check_eq!(replay, *audit);
    let tx = f
        .state()
        .transaction(major.transaction)
        .ok_or("major transaction")?;
    for change in 0..2 {
        let mut bad = tx.clone();
        if change == 0 {
            bad.idempotency.namespace = receipt::NAMESPACE.into();
        } else {
            let EconomicCause::Ruleset { namespace, .. } = &mut bad.cause else {
                return Err("cause".into());
            };
            *namespace = receipt::DERIVED_NAMESPACE.into();
        }
        let mut poisoned = UpdateReceiptAudit::default();
        poisoned.observe(&bad);
        poisoned.observe(tx);
        check!(poisoned.finish_replay().is_err());
    }
    Ok(())
}

#[test]
fn major_fee_repeats_current_title_payer_and_live_actor_at_prepare_stage_and_take() -> Result {
    for phase in 0..3 {
        for drift in 0..3 {
            let mut f = major_fixture()?;
            let action = f.rules.major.clone().ok_or("major")?;
            let mut prepared = if phase > 0 {
                Some(f.prepare(&action, f.actor, 2)?)
            } else {
                None
            };
            if phase == 2 {
                f.stage(action.clone(), 2, prepared.take().ok_or("prepared")?)?;
            }
            let shell = f.shell;
            let actor = f.actor;
            match drift {
                0 => {
                    f.state_mut()
                        .snapshot
                        .asset_mut(shell)
                        .ok_or("title")?
                        .frozen = true
                }
                1 => {
                    f.state_mut()
                        .snapshot
                        .account_mut(AccountId(1))
                        .ok_or("payer")?
                        .status = AccountStatus::Frozen
                }
                _ => {
                    f.world
                        .world_mut()
                        .get_mut(actor)
                        .ok_or("actor")?
                        .flags
                        .insert(mush_core::Flags::GOING);
                    let _ = f.world.world_mut().take_dirty();
                }
            }
            let before = f.state().snapshot.clone();
            let next = f.state().next_transaction;
            match phase {
                0 => check!(f.prepare(&action, actor, 2).is_err()),
                1 => check!(
                    f.stage(action, 2, prepared.take().ok_or("prepared")?)
                        .is_err()
                ),
                _ => check!(f.world.engine_mut().take_staged_action().is_err()),
            }
            check_eq!(f.state().snapshot, before);
            check_eq!(f.state().next_transaction, next);
            check_eq!(f.state().update_receipt_audit().receipts()?.len(), 0);
        }
    }
    Ok(())
}

#[test]
fn major_fee_registration_requires_exact_action_policy_and_preserves_owner_order() -> Result {
    let r = major_rules()?;
    for reverse in [false, true] {
        let mut m = modules(&r, 100);
        if reverse {
            m.reverse();
        }
        let _actual = registered(m)?;
    }
    check!(registered(modules(&r, 0)).is_err());
    let major = r.major.clone().ok_or("major")?;
    check!(
        registered(vec![Box::new(
            crate::EconomyModule::new().with_original_mind_majors(major.clone())
        )])
        .is_err()
    );
    for selected in [r.update.clone(), r.derived.clone().ok_or("derived")?] {
        let economy = crate::EconomyModule::new()
            .with_title_issuers(vec![
                SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
            ])
            .with_initial_filing(policy(100), r.initial.clone())
            .with_compatible_mind_updates(r.update.clone())
            .with_derived_mind_updates(r.derived.clone().ok_or("derived")?)
            .with_original_mind_majors(selected);
        let error = registered(vec![
            Box::new(mush_org::OrgModule::new()),
            Box::new(r.clone()),
            Box::new(economy),
        ])
        .err()
        .ok_or("crossed major action accepted")?;
        check!(
            error
                .to_string()
                .contains("original majors require the exact action")
        );
    }
    let mut f = major_fixture()?;
    let other = f.world.spawn_player("Other major filer");
    check!(f.prepare(&major, other, 2).is_err());
    let wrong = f.rules.wrong.clone();
    check!(f.prepare(&wrong, f.actor, 2).is_err());
    Ok(())
}
