//! Invoked only by the parent's real measured-certificate and paid-Grant gate.
//! Uses actual Tacsim source service, Roll provider/participant and typed store.
//! The in-process complete union models ACK; server writer/Unknown is separate.
use super::*;
#[path = "purchase_story_future.rs"]
mod story_future;
use story_future::story_phase;
#[path = "authored_access_integration_tests.rs"]
mod access_tests;
#[path = "authored_job_integration_tests.rs"]
mod job_tests;
#[path = "authored_paste_integration_tests.rs"]
mod paste_tests;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_proto::{
    InvokeRequest, InvokeResult, MindEditBase, MindLifecycle, MindRequest, MindResponse,
};

fn action(world: &PlayerWorld) -> TestResult<DefinitionRef> {
    world
        .engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|entry| entry.reference())
        .find(|reference| {
            reference.key().owner.as_str() == "roll"
                && reference.key().kind.as_str() == "action"
                && reference.key().id.as_str() == "fork_paid_mind"
        })
        .cloned()
        .ok_or_else(|| "actual fork action".into())
}
fn prepare(
    world: &mut PlayerWorld,
    actor: ObjId,
    number: u128,
    parameters: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    let action = action(world)?;
    Ok(world
        .engine
        .prepare_registered_composed_authoritative_action(
            &action,
            &request(number, actor, "roll", parameters)?,
            token(number),
        )?)
}
fn disk(world: &PlayerWorld) -> Disk {
    world
        .image
        .iter()
        .flat_map(|(owner, data)| {
            data.kv
                .iter()
                .map(move |(key, value)| ((owner.to_string(), key.to_string()), value.clone()))
        })
        .collect()
}
fn roll(world: &PlayerWorld) -> TestResult<&RollStore> {
    world
        .engine
        .world
        .module_state::<RollStore>()
        .ok_or_else(|| "actual Roll owner".into())
}
fn capture(world: &PlayerWorld, retained: &Disk) -> Durable {
    Durable::capture(&world.engine, retained)
}
fn move_selector(saved: &mut Durable, actor: ObjId, mind: u64) -> TestResult {
    let (old_key, mut envelope) = saved
        .disk
        .iter()
        .find(|((owner, _), row)| {
            owner == "roll" && row.pointer("/family/id") == Some(&json!("roll_mind_draft_parent"))
        })
        .map(|(key, row)| (key.clone(), row.clone()))
        .ok_or("actual selector")?;
    let old_encoded = envelope
        .get("key")
        .and_then(Value::as_str)
        .ok_or("selector key")?;
    let prefix = old_key
        .1
        .strip_suffix(old_encoded)
        .ok_or("canonical typed persistence suffix")?;
    let encoded = format!("draft/{:08x}/{mind:016x}", actor.0);
    let key = (old_key.0.clone(), format!("{prefix}{encoded}"));
    *envelope.get_mut("key").ok_or("selector key field")? = json!(encoded);
    let row = envelope.get_mut("value").ok_or("selector value")?;
    *row.get_mut("actor").ok_or("selector actor")? = json!(actor);
    *row.get_mut("mind").ok_or("selector mind")? = json!(mind);
    saved.disk.remove(&old_key);
    saved.disk.insert(key, envelope);
    Ok(())
}

fn stage(
    world: &mut PlayerWorld,
    number: u128,
    prepared: PreparedComposedTxn,
    output: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let action = action(world)?;
    let inv = request(
        number,
        BUYER,
        "roll",
        json!({"parent_licence":"2","name":"child"}),
    )?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        output.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(number),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([73; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([74; 32]),
        action,
        provider: OwnerId::new("roll")?,
        cause: mush_core::action::PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: prepared.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(number))),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?;
    let guards = world
        .engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())?
        .finish()?;
    let _ = world.engine.world.take_dirty();
    world
        .engine
        .stage_authoritative_action(prepared, guards, bundle)?;
    Ok(())
}

async fn run(f: &PurchaseFixture, saved: &Durable, permitted: bool) -> TestResult {
    eprintln!("M7_STORY phase=fork_runtime event=enter");
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    eprintln!("M7_STORY phase=fork_runtime event=booted");
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let before = disk(&world);
        assert!(rows(&before, "roll", "roll_mind_draft_parent").is_empty());
        for (actor, parameters) in [
            (GOD, json!({"parent_licence":"2","name":"child"})),
            (
                BUYER,
                json!({"parent_licence":"2","name":"child","actor":GOD}),
            ),
            (
                BUYER,
                json!({"parent_licence":"2","name":"child","source":"replacement"}),
            ),
            (
                BUYER,
                json!({"parent_licence":"2","name":"child","parent":null}),
            ),
            (BUYER, json!({"parent_licence":"02","name":"child"})),
        ] {
            assert!(prepare(&mut world, actor, 4_000_000, parameters).is_err());
            assert_eq!(disk(&world), before);
        }
        let alt =
            world
                .engine
                .world
                .create(ObjKind::Player, "Fork account alt", GOD, Some(ROOM))?;
        {
            let mut accounts = f.accounts.values.lock().map_err(module_error)?;
            let account = *accounts.get(&BUYER).ok_or("actual buyer account")?;
            f.accounts.epoch.invalidate();
            accounts.insert(alt, account);
        }
        assert!(
            prepare(
                &mut world,
                alt,
                4_000_001,
                json!({"parent_licence":"2","name":"child"})
            )
            .is_err(),
            "shared account is not the named paid holder"
        );
        let prepared = prepare(
            &mut world,
            BUYER,
            4_000_001,
            json!({"parent_licence":"2","name":"child"}),
        );
        if !permitted {
            assert!(prepared.is_err(), "Sealed is never fork permission");
            assert_eq!(disk(&world), before);
            return Ok(());
        }
        let (prepared, _) = prepared?;
        assert!(
            roll(&world)?.mind(BUYER, "child").is_err(),
            "no pre-ACK working copy"
        );
        assert!(
            rows(&disk(&world), "roll", "roll_mind_draft_parent").is_empty(),
            "no pre-ACK selector"
        );
        let (writes, installation) = prepared.into_parts();
        let mut committed = before.clone();
        retain_writes(&mut committed, &writes)?;
        // A crash after complete durable publication but before RAM install is
        // loaded from the exact owner+typed images, without rerunning eligibility.
        let cold = capture(&world, &committed);
        let cold_world = PlayerWorld::boot_forks(f, &cold, true)?;
        assert_eq!(
            roll(&cold_world)?.mind(BUYER, "child")?.draft().source,
            SOURCE
        );
        cold_world.tasks.shutdown(Duration::from_secs(5)).await;
        world.engine.install_composed_action(installation)?;
        world.commit()?;
        let child = roll(&world)?.mind(BUYER, "child")?.clone();
        assert_eq!(child.draft().source, SOURCE);
        let parent = roll(&world)?.mind(GOD, "Candidate")?;
        assert_eq!(child.draft().envelope, parent.draft().envelope);
        assert!(
            child.require_original_access().is_err(),
            "new fork does not unlock old authoring routes"
        );
        let row = serde_json::to_value(&child)?;
        let selector = one(rows(&committed, "roll", "roll_mind_draft_parent"))?;
        assert_eq!(
            row.get("provenance").and_then(|p| p.get("parent")),
            selector.get("parent")
        );
        assert_eq!(selector.get("actor"), Some(&json!(BUYER)));
        assert_eq!(selector.get("mind"), Some(&json!(child.id())));
        assert_eq!(selector.get("licence"), Some(&json!("2")));
        assert_eq!(selector.get("created_parent_term"), Some(&json!("1")));

        // Every production removal path reaches the new owner fence before any
        // library/selector write. The marked working copy survives both requests.
        let text = world.say(BUYER, "@mind/remove child")?;
        assert!(PlayerWorld::private_text(&text, BUYER).contains("removal requested"));
        let base = child.base();
        let result = world.step(Inbound::Invoke {
            actor: BUYER,
            conn: BUYER_CONN,
            id: 944,
            request: InvokeRequest::Mind(MindRequest::Lifecycle {
                name: "child".into(),
                base: MindEditBase {
                    edit: base.edit,
                    revision: base.revision,
                },
                edit: MindLifecycle::Remove {},
            }),
        })?;
        assert!(result.iter().any(|(a, e)| matches!(
            (a, e),
            (
                Audience::Conn(BUYER_CONN),
                OutputEvent::InvokeResult {
                    result: InvokeResult::Mind(MindResponse::Error { .. }),
                    ..
                }
            )
        )));
        let pending = world
            .engine
            .take_observed_authoritative_submit()
            .ok_or("captured removal")?;
        assert!(pending.is_observed());
        drop(pending); // Never driven/persisted here: this branch only checks non-optimistic entry.
        assert_eq!(roll(&world)?.mind(BUYER, "child")?, &child);
        assert_eq!(
            one(rows(&committed, "roll", "roll_mind_draft_parent"))?,
            selector
        );
        let restart = capture(&world, &committed);
        eprintln!("M7_STORY phase=fork_owned_cold event=complete");
        eprintln!("M7_STORY phase=b1 event=begin");
        story_phase("fork.access", || access_tests::exercise(f, &restart))?.await?;
        eprintln!("M7_STORY phase=b1 event=complete");
        eprintln!("M7_STORY phase=marked_paste event=begin");
        story_phase("fork.paste", || paste_tests::exercise(f, &restart))?.await?;
        eprintln!("M7_STORY phase=marked_paste event=complete");
        eprintln!("M7_STORY phase=b2 event=begin");
        story_phase("fork.jobs", || job_tests::exercise(f, &restart))?.await?;
        eprintln!("M7_STORY phase=b2 event=complete");
        let reloaded = PlayerWorld::boot_forks(f, &restart, true)?;
        assert_eq!(roll(&reloaded)?.mind(BUYER, "child")?, &child);
        reloaded.tasks.shutdown(Duration::from_secs(5)).await;
        assert!(
            prepare(
                &mut world,
                BUYER,
                4_000_002,
                json!({"parent_licence":"2","name":"child"})
            )
            .is_err()
        );

        // Deleting every selector, and crossing an actual paired draft to a
        // different character, must fail final completeness even with valid JSON.
        let mut absent = restart.clone();
        remove_family(&mut absent, "roll", "roll_mind_draft_parent");
        assert!(PlayerWorld::boot_forks(f, &absent, true).is_err());
        let mut crossed = restart.clone();
        let mut moved = row.clone();
        *moved.get_mut("character").ok_or("mind character")? = json!(GOD);
        *moved.get_mut("id").ok_or("mind id")? = json!(2);
        crossed
            .disk
            .remove(&("roll".into(), format!("mind:{}:{}", BUYER.0, child.id())));
        crossed
            .disk
            .insert(("roll".into(), format!("mind:{}:2", GOD.0)), moved);
        for ((owner, key), value) in &mut crossed.disk {
            if owner == "roll" && key == &format!("library:{}", BUYER.0) {
                value
                    .get_mut("names")
                    .and_then(Value::as_object_mut)
                    .ok_or("buyer names")?
                    .remove("child");
            }
            if owner == "roll" && key == &format!("library:{}", GOD.0) {
                *value.get_mut("next_mind").ok_or("library allocator")? = json!(3);
                value
                    .get_mut("names")
                    .and_then(Value::as_object_mut)
                    .ok_or("author names")?
                    .insert("child".into(), json!(2));
            }
        }
        // Repair the canonical typed key too; rejection must be the actual
        // historical paid holder join, not an uncorrected key/body mismatch.
        move_selector(&mut crossed, GOD, 2)?;
        assert!(PlayerWorld::boot_forks(f, &crossed, true).is_err());
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result?;
    if permitted {
        story_phase("fork.freshness", || freshness(f, saved))?.await?;
        story_phase("fork.issuer", || issuer_independence(f, saved))?.await?;
    }
    Ok(())
}

async fn freshness(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let term: PaidTermRecord = serde_json::from_value(one(f.rows("roll", "roll_mind_paid_term"))?)?;
    for boundary in [
        "expiry",
        "clock_generation",
        "roll_generation",
        "staged_expiry",
        "owned_expiry",
    ] {
        let mut world = PlayerWorld::boot_forks(f, saved, true)?;
        let tasks = world.tasks.clone();
        let result: TestResult = async {
            let before = disk(&world);
            let (prepared, output) = prepare(
                &mut world,
                BUYER,
                4_000_003,
                json!({"parent_licence":"2","name":"child"}),
            )?;
            if boundary == "owned_expiry" {
                // Positive control: the same actual bundle/guard path succeeds
                // while fresh. Only after handoff do we advance to exact expiry.
                stage(&mut world, 4_000_003, prepared, output)?;
                let staged = world
                    .engine
                    .take_staged_action()?
                    .ok_or("actual staged fork")?;
                let (bundle, installation) = staged.into_parts();
                world.engine.world.set_now(term.window().grace_end().get());
                let mut retained = before.clone();
                retain_writes(&mut retained, bundle.writes())?;
                world.engine.install_composed_action(installation)?;
                assert_eq!(roll(&world)?.mind(BUYER, "child")?.draft().source, SOURCE);
                let restarted = PlayerWorld::boot_forks(f, &capture(&world, &retained), true)?;
                assert!(
                    roll(&restarted)?.mind(BUYER, "child").is_ok(),
                    "legitimate lapse is not boot corruption"
                );
                restarted.tasks.shutdown(Duration::from_secs(5)).await;
            } else if boundary == "staged_expiry" {
                stage(&mut world, 4_000_003, prepared, output)?;
                world.engine.world.set_now(term.window().grace_end().get());
                assert!(
                    world.engine.take_staged_action().is_err(),
                    "final handoff repeats the current grant fence"
                );
                assert!(!world.engine.has_staged_action());
                assert!(!world.engine.action_state().reservation_lost());
                assert!(roll(&world)?.mind(BUYER, "child").is_err());
                assert_eq!(disk(&world), before);
            } else {
                match boundary {
                    "expiry" => world.engine.world.set_now(term.window().grace_end().get()),
                    "clock_generation" => {
                        let before_stamp = world.engine.world.world_ic_now();
                        let epoch = world.engine.world.next_world_timeline_epoch(2)?;
                        world.engine.world.append_world_timeline_epoch(epoch)?;
                        let after_stamp = world.engine.world.world_ic_now();
                        assert_eq!(before_stamp.at, after_stamp.at);
                        assert_ne!(before_stamp.clock, after_stamp.clock);
                    }
                    "roll_generation" => {
                        world.say(GOD, "@mind/new independent")?;
                        assert!(roll(&world)?.mind(GOD, "independent").is_ok());
                    }
                    _ => return Err("unexpected fixture boundary".into()),
                }
                let pre_custody = disk(&world);
                assert!(
                    stage(&mut world, 4_000_003, prepared, output).is_err(),
                    "{boundary} before custody must refuse atomically"
                );
                assert!(roll(&world)?.mind(BUYER, "child").is_err());
                assert_eq!(disk(&world), pre_custody);
                assert!(rows(&disk(&world), "roll", "roll_mind_draft_parent").is_empty());
                assert!(!world.engine.action_state().reservation_lost());
            }
            Ok(())
        }
        .await;
        tasks.shutdown(Duration::from_secs(5)).await;
        result?;
    }
    Ok(())
}

pub(super) fn exercise(f: &PurchaseFixture, saved: &Durable, permitted: bool) -> TestResult {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(story_phase("fork.run", || run(f, saved, permitted))?)
}

async fn issuer_independence(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let before = f.rows("roll", "roll_mind_paid_term");
        let term: PaidTermRecord = serde_json::from_value(one(before.clone())?)?;
        let licence: PersonalPaidLicence =
            serde_json::from_value(one(f.rows("roll", "roll_mind_personal_paid_licence"))?)?;
        let shell = licence.charter().object();
        world.say(GOD, &format!("@asset/transfer #{}=#{}", shell.0, BUYER.0))?;
        assert_eq!(
            world
                .engine
                .world
                .module_state::<EconomyStore>()
                .ok_or("Economy")?
                .snapshot()
                .asset(shell)
                .ok_or("title")?
                .title_owner,
            EconomicOwner::Object(BUYER)
        );
        world
            .engine
            .world
            .set_now(1000 + TERM_SECONDS + GRACE_SECONDS);
        assert!(world.engine.world.now() < term.window().grace_end().get());
        let (prepared, _) = prepare(
            &mut world,
            BUYER,
            4_000_004,
            json!({"parent_licence":"2","name":"independent_child"}),
        )?;
        let (writes, installation) = prepared.into_parts();
        let mut retained = disk(&world);
        retain_writes(&mut retained, &writes)?;
        world.engine.install_composed_action(installation)?;
        assert_eq!(
            roll(&world)?
                .mind(BUYER, "independent_child")?
                .draft()
                .source,
            SOURCE
        );
        assert_eq!(
            rows(&retained, "roll", "roll_mind_paid_term"),
            before,
            "a fork neither extends nor replaces its parent's grant"
        );
        let cold = PlayerWorld::boot_forks(f, &capture(&world, &retained), true)?;
        assert!(roll(&cold)?.mind(BUYER, "independent_child").is_ok());
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
