#![cfg(test)]

//! Runs only after the actual marked Save and complete Registry worker. All
//! filings below use real E4/roots/typed/shell installation; no certificate or
//! qualifying parent grant is constructed here.
use super::*;
use crate::minds::bulk::registry_integration_tests::original_purchase::request as action_request;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_proto::{InvokeRequest, InvokeResult, MindRequest, MindResponse};
use mush_roll::commerce::records::{FiledRecord, InitialFilingOperation, OwnerTerm};

const FIRST: u128 = 7_000_000;
type Events = Vec<(mush_core::Audience, mush_proto::OutputEvent)>;

fn definition(world: &PlayerWorld, id: &str) -> TestResult<DefinitionRef> {
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
                && reference.key().id.as_str() == id
        })
        .cloned()
        .ok_or_else(|| "actual filing action".into())
}
fn raw(world: &mut PlayerWorld, input: mush_core::Inbound) -> TestResult<Events> {
    let mut out = Outbox::new();
    world
        .engine
        .step(input, &mut out, &mut mush_core::engine::Effects::new());
    let events = out.take();
    world.commit()?;
    Ok(events)
}
fn filed_source(
    world: &mut PlayerWorld,
    actor: ObjId,
    licence: u64,
    allowed: bool,
    expected_source: mush_core::DefinitionDigest,
) -> TestResult {
    let conn = if actor == BUYER {
        BUYER_CONN
    } else {
        OBSERVER_CONN
    };
    let events = raw(
        world,
        mush_core::Inbound::Invoke {
            conn,
            actor,
            id: 7_000,
            request: InvokeRequest::Mind(MindRequest::FiledSource {
                licence: licence.to_string(),
                first: 1,
                last: 200,
            }),
        },
    )?;
    let mut answers = 0;
    for (audience, event) in &events {
        if let mush_proto::OutputEvent::InvokeResult {
            result: InvokeResult::Mind(response),
            ..
        } = event
        {
            answers += 1;
            assert_eq!(*audience, mush_core::Audience::Conn(conn));
            match response {
                MindResponse::FiledSource {
                    licence: actual,
                    source_sha256,
                    lines,
                    ..
                } => {
                    assert!(allowed, "refused source escaped");
                    assert_eq!(actual, &licence.to_string());
                    assert_eq!(source_sha256, &expected_source.to_hex());
                    assert!(!lines.is_empty());
                    if expected_source != mush_roll::source_digest(SOURCE) {
                        assert!(
                            lines.iter().any(
                                |line| line.contains("independently authored child annotation")
                            )
                        );
                    }
                }
                MindResponse::Error { .. } => assert!(!allowed, "lawful dependent source refused"),
                _ => return Err("unexpected source response".into()),
            }
        }
        if serde_json::to_string(event)?.contains("this.landed") {
            assert!(allowed);
            assert_eq!(*audience, mush_core::Audience::Conn(conn));
        }
    }
    assert_eq!(answers, 1);
    Ok(())
}
fn select(world: &mut PlayerWorld, licence: u64, allowed: bool) -> TestResult {
    world.say(BUYER, "SIM TEAM red")?;
    world.say(BUYER, "SIM AI 1")?;
    world.say(BUYER, "SIM UNIT heavy_mk1")?;
    let events = world.say(BUYER, &format!("SIM MIND filed:{licence} PROFILE Default"))?;
    assert_eq!(
        PlayerWorld::private_text(&events, BUYER).contains("Your slot will carry"),
        allowed
    );
    Ok(())
}
fn stage(
    world: &mut PlayerWorld,
    number: u128,
    parameters: Value,
    prepared: PreparedComposedTxn,
    output: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let action = definition(world, "dependent_initial_filing")?;
    let inv = action_request(number, BUYER, "roll", parameters)?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        output.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(number),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([81; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([82; 32]),
        action,
        provider: OwnerId::new("roll")?,
        cause: mush_core::action::PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: prepared.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|resolution| resolution.bind(token(number))),
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
fn prepare(
    world: &mut PlayerWorld,
    number: u128,
    parameters: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    let action = definition(world, "dependent_initial_filing")?;
    Ok(world
        .engine
        .prepare_registered_composed_authoritative_action(
            &action,
            &action_request(number, BUYER, "roll", parameters)?,
            token(number),
        )?)
}
pub(super) fn apply(
    world: &mut PlayerWorld,
    writes: &[ReplayWrite],
    install: mush_core::composed::ComposedInstallation,
) -> TestResult {
    let operations = writes
        .iter()
        .map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => Ok(Some(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: key.to_string().into(),
                    value: value.clone(),
                },
            })),
            ReplayWrite::ModuleDelete { namespace, key } => Ok(Some(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::Del {
                    key: key.to_string().into(),
                },
            })),
            ReplayWrite::LedgerChunkSet { write } => Ok(Some(ModuleOp {
                ns: write.namespace().to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: write.key().to_string().into(),
                    value: serde_json::to_value(write.chunk())?,
                },
            })),
            ReplayWrite::ObjectSet { .. } | ReplayWrite::ObjectAllocatorSet { .. } => Ok(None),
            ReplayWrite::ObjectDelete { .. } => Err("filing does not delete objects".into()),
        })
        .collect::<TestResult<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    mush_testkit::fold_module_ops_into(&mut world.image, operations);
    world.engine.install_composed_action(install)?;
    world.commit()?;
    Ok(())
}
fn wallet(world: &PlayerWorld) -> TestResult<Amount> {
    Ok(world
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .snapshot()
        .account(WalletId(2))
        .ok_or("actual funded buyer wallet")?
        .posted_balance)
}
fn operation(saved: &Durable) -> TestResult<InitialFilingOperation> {
    Ok(serde_json::from_value(
        rows(&saved.disk, "roll", "roll_mind_initial_filing")
            .into_iter()
            .find(|row| row.get("format") == Some(&json!(3)))
            .ok_or("actual dependent operation")?,
    )?)
}
fn alter_binding(saved: &mut Durable, owner: ObjId) -> TestResult {
    let (key, envelope) = saved
        .disk
        .iter_mut()
        .find(|((ns, _), row)| {
            ns == "roll" && row.pointer("/family/id") == Some(&json!("roll_mind_owner_parent"))
        })
        .ok_or("actual owner binding")?;
    let _ = key;
    *envelope
        .pointer_mut("/value/parent/owner")
        .ok_or("binding owner")? = json!(owner);
    Ok(())
}

pub(super) async fn exercise(
    f: &PurchaseFixture,
    certified: &Durable,
    rating: &RatingRef,
) -> TestResult {
    eprintln!("M7_STORY phase=dependent_owner event=begin");
    let mut world = PlayerWorld::boot_dependent(f, certified)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let saved = source(&world)?;
        assert!(saved.require_original_access().is_err());
        let receipt: RatingReceipt = serde_json::from_value(rows(&disk(&world), "roll", "rating_receipts")
            .into_iter().find(|row| row.pointer("/publication/job") == Some(&json!(rating.job())))
            .ok_or("exact measured marked receipt")?)?;
        assert_eq!(receipt.reference()?, *rating);
        assert_eq!(receipt.subject().revision().revision(), saved.reference());
        let parent: PaidTermRecord = serde_json::from_value(one(rows(&disk(&world), "roll", "roll_mind_paid_term"))?)?;
        assert!(matches!(parent.terms().form(), PaidForm::Open {}));
        let reserve = definition(&world, mush_core::reserved_shell::RESERVE_ACTION)?;
        let shell = ObjId(world.engine.world.next_id());
        let (reserved, _) = world.engine.prepare_registered_composed_authoritative_action(&reserve,
            &action_request(FIRST, BUYER, "roll", json!({}))?, token(FIRST))?;
        let (writes, install) = reserved.into_parts();
        apply(&mut world, &writes, install)?;
        let revision = saved.reference();
        let parameters = json!({"shell_request":RequestUid::from_bytes(FIRST.to_be_bytes()),
            "mind":revision.key.mind,"revision":revision.key.number,"revision_digest":revision.digest,
            "rating_job":rating.job(),"rating_id":rating.id(),"rating_digest":rating.receipt_digest(),
            "payer":WalletId(2),"charter_name":"Measured private dependent"});
        let before = capture(&world, &disk(&world));
        let before_cash = wallet(&world)?;
        assert!(before_cash >= Amount::from_minor(100));
        // Original filing remains closed even with a real marked certificate.
        let original = definition(&world, "initial_filing")?;
        assert!(world.engine.prepare_registered_composed_authoritative_action(&original,
            &action_request(FIRST + 1, BUYER, "roll", parameters.clone())?, token(FIRST + 1)).is_err());
        assert_eq!(disk(&world), before.disk);
        let (prepared, output) = prepare(&mut world, FIRST + 1, parameters.clone())?;
        assert_eq!(wallet(&world)?, before_cash);
        stage(&mut world, FIRST + 1, parameters.clone(), prepared, output)?;
        let (bundle, install) = world.engine.take_staged_action()?.ok_or("dependent custody")?.into_parts();
        let writes = bundle.writes().to_vec();
        assert!(writes.iter().any(|write| matches!(write, ReplayWrite::LedgerChunkSet { .. })));
        apply(&mut world, &writes, install)?;
        assert_eq!(wallet(&world)?, before_cash.checked_sub(Amount::from_minor(100)).ok_or("fee arithmetic")?);
        let filed = capture(&world, &disk(&world));
        let op = operation(&filed)?;
        let value = serde_json::to_value(&op)?;
        let licence = serde_json::from_value::<mush_roll::commerce::LicenceId>(value.get("licence").ok_or("issued owner licence")?.clone())?.get();
        let owner_term: OwnerTerm = serde_json::from_value(rows(&filed.disk, "roll", "roll_mind_owner_term")
            .into_iter().find(|row| row.get("licence") == Some(&json!(licence.to_string()))).ok_or("actual owner term")?)?;
        assert_eq!(owner_term.window(), mush_roll::commerce::rights::dependent_owner_window(
            mush_roll::commerce::WorldIcSeconds::from_stamp(&world.engine.world.world_ic_now())?, TERM_SECONDS,
            GRACE_SECONDS, parent.window())?);
        let child: FiledRecord = serde_json::from_value(rows(&filed.disk, "roll", "roll_mind_filed")
            .into_iter().find(|row| row.get("operation") == Some(&json!(token(FIRST + 1)))).ok_or("actual filed child")?)?;
        assert!(child.content().parent().is_some());
        assert_eq!(rows(&filed.disk, "roll", "roll_mind_owner_parent").len(), 1);
        assert_eq!(world.engine.world.module_state::<EconomyStore>().ok_or("economy")?.snapshot()
            .asset(shell).ok_or("actual child title")?.title_owner, EconomicOwner::Object(BUYER));
        // Exact same typed+owner union cold-loads, including complete roots and
        // historical parent grant/E4 evidence, before any player use.
        let mut cold = PlayerWorld::boot_dependent(f, &filed)?;
        filed_source(&mut cold, BUYER, licence, true, saved.source())?;
        filed_source(&mut cold, GOD, licence, false, saved.source())?;
        select(&mut cold, licence, true)?;
        cold.start()?;
        let boarded = cold.board()?;
        assert_eq!(cold.state()?, crate::match_::MatchState::Running,
            "{}", PlayerWorld::private_text(&boarded, BUYER));
        let id = crate::match_::centre_of(&cold.engine.world).and_then(|c|c.match_id).ok_or("actual match")?;
        let record = cold.record(id)?;
        let unit = record.teams.iter().flat_map(|team|team.pilots.iter())
            .find_map(|(pilot,unit)|(*pilot==BUYER).then_some(*unit)).ok_or("actual unit")?;
        let seal = record.minds.get(&unit).ok_or("dependent seal")?.seal.clone();
        let certificate = seal.requested.origin.certificate().ok_or("exact filed certificate")?;
        assert_eq!(certificate.rating_job, rating.job());
        assert_eq!(certificate.rating_id, rating.id().digest());
        assert_eq!(certificate.rating_digest, rating.receipt_digest());
        assert_eq!((certificate.revision_character, certificate.revision_mind, certificate.revision_number, certificate.revision_digest),
            (revision.key.character, revision.key.mind, revision.key.number, revision.digest));
        cold.read_sealed_mind(&seal.mind).await?;
        let deadline = Instant::now() + WAIT;
        while !cold.snapshots.contains_key(&id) {
            cold.pump()?;
            if Instant::now() >= deadline { return Err("dependent opening snapshot".into()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let initial = cold.snapshots.get(&id).ok_or("actual opening snapshot")?;
        let ground = crate::arena_bridge::ground_of(&cold.engine.world, id).ok_or("actual ground")?;
        let arena = crate::arena::Snapshot::decode(&initial.document, (*ground).clone())?;
        let piloted = arena.units.iter().find(|row| row.id == unit).ok_or("actual opening unit")?;
        assert_eq!(piloted.controller, crate::arena::Controller::Pilot);
        assert_eq!(piloted.seal, seal.seal_ref());
        cold.engine.world.set_now(parent.window().grace_end().get());
        filed_source(&mut cold, BUYER, licence, false, saved.source())?;
        cold.read_sealed_mind(&seal.mind).await?;
        let before_exit = cold.snapshots.get(&id).map_or(-1, |snapshot|snapshot.at_ms);
        cold.say(BUYER, "EXIT")?;
        let first = cold.progressed_snapshot(id, unit, before_exit, 0, -1).await?;
        let (thinks, landed) = crate::minds::bulk::registry_integration_tests::original_purchase::paid_checkout::assert_captured(&cold, id, unit, &seal, &first)?;
        let second = cold.progressed_snapshot(id, unit, first.at_ms, thinks, landed).await?;
        crate::minds::bulk::registry_integration_tests::original_purchase::paid_checkout::assert_captured(&cold, id, unit, &seal, &second)?;
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        let mut sale = PlayerWorld::boot_dependent(f, &filed)?;
        let listing = f.config.quotes.listing_action()?;
        for actor in [GOD, BUYER] { sale.engine.world.move_to(actor, ROOM)?; }
        // Positive control uses the same real OriginalSaleReader/venue/wallet
        // composition; the dependent owner cannot widen its original promise.
        let (prepared, _) = sale.engine.prepare_registered_composed_authoritative_action(&listing,
            &action_request(FIRST+10, GOD, "market", json!({"owner_licence":"1","form":"open"}))?, token(FIRST+10))?;
        let (writes, installation) = prepared.into_parts(); apply(&mut sale, &writes, installation)?;
        let before_sale = disk(&sale);
        assert!(sale.engine.prepare_registered_composed_authoritative_action(&listing,
            &action_request(FIRST+11, BUYER, "market", json!({"owner_licence":licence.to_string(),"form":"open"}))?, token(FIRST+11)).is_err());
        assert_eq!(disk(&sale), before_sale);
        sale.tasks.shutdown(Duration::from_secs(5)).await;
        let mut titles = PlayerWorld::boot_dependent(f, &filed)?;
        let parent_licence: PersonalPaidLicence = serde_json::from_value(one(rows(&filed.disk, "roll", "roll_mind_personal_paid_licence"))?)?;
        let parent_charter = parent_licence.charter().object();
        titles.transfer_charter(parent_charter, BUYER, 49_000_060)?;
        assert_eq!(titles.engine.world.module_state::<EconomyStore>().ok_or("economy")?.snapshot()
            .asset(parent_charter).ok_or("parent title")?.title_owner, EconomicOwner::Object(BUYER));
        // The issuer's title departure does not revoke the customer's own
        // qualifying paid grant or its dependent owner's source.
        filed_source(&mut titles, BUYER, licence, true, saved.source())?;
        filed_source(&mut titles, BUYER, parent_licence.id().get(), true, mush_roll::source_digest(SOURCE))?;
        titles.transfer_charter(shell, GOD, 49_000_070)?;
        assert_eq!(titles.engine.world.module_state::<EconomyStore>().ok_or("economy")?.snapshot()
            .asset(shell).ok_or("child title")?.title_owner, EconomicOwner::Object(GOD));
        filed_source(&mut titles, BUYER, licence, false, saved.source())?;
        filed_source(&mut titles, GOD, licence, false, saved.source())?;
        // A transferred original title or staff status cannot inherit the old
        // owner's parent binding. The paid grant itself remains independent.
        filed_source(&mut titles, BUYER, parent_licence.id().get(), true, mush_roll::source_digest(SOURCE))?;
        let transferred = capture(&titles, &disk(&titles));
        let mut transferred_cold = PlayerWorld::boot_dependent(f, &transferred)?;
        filed_source(&mut transferred_cold, GOD, licence, false, saved.source())?;
        transferred_cold.tasks.shutdown(Duration::from_secs(5)).await;
        titles.tasks.shutdown(Duration::from_secs(5)).await;
        // Fresh owner selection refuses at expiry, but historical cold audit
        // remains valid. No term is extended or a seller parent purchased.
        let mut expired = PlayerWorld::boot_dependent(f, &filed)?;
        expired.engine.world.set_now(parent.window().grace_end().get());
        select(&mut expired, licence, false)?;
        let expired_image = capture(&expired, &disk(&expired));
        let restarted = PlayerWorld::boot_dependent(f, &expired_image)?;
        restarted.tasks.shutdown(Duration::from_secs(5)).await;
        expired.tasks.shutdown(Duration::from_secs(5)).await;
        // Stage/take must re-resolve the actual parent's time fence.
        for clock in [false, true] {
            let mut staged = PlayerWorld::boot_dependent(f, &before)?;
            let (prepared, output) = prepare(&mut staged, FIRST + 1, parameters.clone())?;
            if clock {
                let stamp = staged.engine.world.world_ic_now();
                let epoch = staged.engine.world.next_world_timeline_epoch(2)?;
                staged.engine.world.append_world_timeline_epoch(epoch)?;
                assert_eq!(staged.engine.world.world_ic_now().at, stamp.at);
                assert_ne!(staged.engine.world.world_ic_now().clock, stamp.clock);
            } else { staged.engine.world.set_now(parent.window().grace_end().get()); }
            assert!(stage(&mut staged, FIRST + 1, parameters.clone(), prepared, output).is_err());
            assert_eq!(wallet(&staged)?, before_cash);
            assert!(rows(&disk(&staged), "roll", "roll_mind_owner_parent").is_empty());
            assert!(!staged.engine.action_state().reservation_lost());
            staged.tasks.shutdown(Duration::from_secs(5)).await;
        }
        let mut late = PlayerWorld::boot_dependent(f, &before)?;
        let (prepared, output) = prepare(&mut late, FIRST + 1, parameters.clone())?;
        stage(&mut late, FIRST + 1, parameters.clone(), prepared, output)?;
        late.engine.world.set_now(parent.window().grace_end().get());
        assert!(late.engine.take_staged_action().is_err());
        assert_eq!(wallet(&late)?, before_cash);
        assert!(rows(&disk(&late), "roll", "roll_mind_owner_parent").is_empty());
        late.tasks.shutdown(Duration::from_secs(5)).await;
        // Once taken into writer custody, the same acknowledged operation can
        // install after lapse; live access still refuses and boot keeps history.
        let mut owned = PlayerWorld::boot_dependent(f, &before)?;
        let (prepared, output) = prepare(&mut owned, FIRST + 1, parameters.clone())?;
        stage(&mut owned, FIRST + 1, parameters, prepared, output)?;
        let (bundle, install) = owned.engine.take_staged_action()?.ok_or("owned")?.into_parts();
        owned.engine.world.set_now(parent.window().grace_end().get());
        apply(&mut owned, bundle.writes(), install)?;
        filed_source(&mut owned, BUYER, licence, false, saved.source())?;
        let replayed = PlayerWorld::boot_dependent(f, &capture(&owned, &disk(&owned)))?;
        replayed.tasks.shutdown(Duration::from_secs(5)).await;
        owned.tasks.shutdown(Duration::from_secs(5)).await;
        // Mandatory scans reject all missing bindings and a crossed owner,
        // including when every ordinary operation/root remains retained.
        let mut missing = filed.clone();
        missing.disk.retain(|(ns,_), row| !(ns=="roll"
            && row.pointer("/family/id") == Some(&json!("roll_mind_owner_parent"))));
        assert!(PlayerWorld::boot_dependent(f, &missing).is_err());
        let mut crossed = filed.clone(); alter_binding(&mut crossed, GOD)?;
        assert!(PlayerWorld::boot_dependent(f, &crossed).is_err());
        Ok(())
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result?;
    eprintln!("M7_STORY phase=dependent_owner event=complete");
    Ok(())
}
