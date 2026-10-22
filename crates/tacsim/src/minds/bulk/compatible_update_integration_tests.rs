#![cfg(test)]

//! Additional genuine original revision, certificate, fee and line publication.
//! No receipt/rating/title is fabricated; cold mutations below are corruption tests.
use super::*;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::{Audience, Inbound, OfferExecutionContext, OfferSubmission};
use mush_proto::{
    InvokeRequest, InvokeResult, MindEditBase, MindRequest, MindResponse, OutputEvent,
};
use mush_roll::commerce::records::{FiledRecord, OwnerTerm, canonical_duplicate_digest};
#[path = "compatible_update_integration_tests/attested.rs"]
mod attested;
#[path = "compatible_update_integration_tests/captured.rs"]
mod captured;
#[path = "compatible_update_integration_tests/follow.rs"]
mod follow;
#[path = "compatible_update_integration_tests/registry.rs"]
mod registry;
const START: u128 = 20_000_000;
fn boot(f: &PurchaseFixture, saved: &Durable) -> TestResult<PurchaseFixture> {
    PurchaseFixture::boot_options(
        f.resources.clone(),
        f.accounts.clone(),
        saved,
        f.source_forms,
        true,
    )
}
fn roll(world: &PlayerWorld) -> TestResult<&RollStore> {
    world
        .engine
        .world
        .module_state::<RollStore>()
        .ok_or_else(|| "actual Roll".into())
}
fn disk(world: &PlayerWorld) -> Disk {
    world
        .image
        .iter()
        .flat_map(|(owner, data)| {
            // Keep the `log:` chunks boot normalisation moved into `ledger`.
            let kv = data
                .kv
                .iter()
                .map(move |(key, value)| ((owner.to_string(), key.to_string()), value.clone()));
            let ledger = data.ledger.iter().map(move |(key, text)| {
                (
                    (owner.to_string(), key.to_string()),
                    serde_json::from_str::<serde_json::Value>(text)
                        .expect("a ledger chunk is JSON"),
                )
            });
            kv.chain(ledger)
        })
        .collect()
}
async fn save(f: &PurchaseFixture, saved: &Durable) -> TestResult<(Durable, Revision)> {
    let mut world = PlayerWorld::boot_updates(f, saved)?;
    let tasks = world.tasks.clone();
    let result:TestResult<(Durable,Revision)>=async {
        let mind=roll(&world)?.mind(GOD,"Candidate")?;
        let old=roll(&world)?.revision(GOD,mind.latest().ok_or("original saved revision")?)?.clone();
        let events=world.say(GOD,"@mind/insert Candidate 1=// compatible authored update")?;
        assert!(PlayerWorld::private_text(&events,GOD).contains("Working source updated."));
        let base=roll(&world)?.mind(GOD,"Candidate")?.base();
        let events=world.step(Inbound::Invoke{actor:GOD,conn:OBSERVER_CONN,id:START as u64,request:InvokeRequest::Mind(MindRequest::Save{name:"Candidate".into(),base:MindEditBase{edit:base.edit,revision:base.revision},notes:"compatible update".into(),version:None})})?;
        assert!(events.iter().any(|(a,e)|matches!(e,OutputEvent::InvokeResult{id,result:InvokeResult::Mind(MindResponse::Queued{..})}if *id==START as u64)&&*a==Audience::Conn(OBSERVER_CONN)));
        let deadline=Instant::now()+Duration::from_secs(30);
        let revision=loop{
            let mind=roll(&world)?.mind(GOD,"Candidate")?;
            let current=roll(&world)?.revision(GOD,mind.latest().ok_or("saved source")?)?;
            if current.reference()!=old.reference(){break current.clone()}
            if Instant::now()>=deadline{return Err("actual original Save did not complete".into())}
            let when=world.engine.sched.next_due().ok_or("real author result scheduling")?;
            let mut out=Outbox::new();world.engine.fire_due(when,&mut out,&mut mush_core::Effects::new());world.commit()?;
            tokio::time::sleep(Duration::from_millis(2)).await;
        };
        revision.require_original_access()?;
        assert!(revision.envelope()==old.envelope(),"compatible edit preserves actual schema/card");
        assert_ne!(canonical_duplicate_digest(&revision)?,canonical_duplicate_digest(&old)?);
        // Return to the fixture's actual physical retail venue through normal
        // World movement before the next real quote; no venue authority is forged.
        world.engine.world.move_to(GOD,ROOM)?;world.engine.world.move_to(BUYER,ROOM)?;
        Ok((Durable::capture(&world.engine,&disk(&world)),revision))
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
fn parameters(revision: &Revision, rating: RatingRef) -> Value {
    let r = revision.reference();
    json!({"line":1,"mind":r.key.mind,"revision":r.key.number,"revision_digest":r.digest,"rating_job":rating.job(),"rating_id":rating.id(),"rating_digest":rating.receipt_digest(),"payer":0})
}
fn action(f: &PurchaseFixture) -> TestResult<DefinitionRef> {
    f.config
        .compatible_update
        .clone()
        .ok_or_else(|| "actual compatible update action".into())
}
fn signed(f: &mut PurchaseFixture, n: u128, params: Value) -> TestResult<PreparedComposedTxn> {
    f.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x58; 32])?,
        [],
    )?);
    let principal = PrincipalId::Object { id: GOD };
    let query = f
        .engine
        .legal_action_query(principal.clone(), GOD, PerceivedRef::SelfActor)?;
    let offers = f.engine.project_legal_actions(&query)?;
    let action = action(f)?;
    let offer = offers
        .projected_for_action(&action)
        .ok_or("actual signed compatible update")?;
    let inv = request(n, GOD, "roll", params.clone())?;
    let resolved = f.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: inv.request(),
            principal,
            actor: GOD,
            idempotency: inv.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: offer.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: params,
        },
    )?;
    let (exact, resolved, _) = resolved.into_authoritative_parts();
    assert_eq!(exact, action);
    Ok(f.engine
        .prepare_registered_composed_resolved_authoritative_action(&action, &resolved, token(n))?
        .0)
}
fn stage(
    f: &mut PurchaseFixture,
    n: u128,
    params: Value,
    p: PreparedComposedTxn,
    o: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let action = action(f)?;
    let inv = request(n, GOD, "roll", params)?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        o.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(n),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([91; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([92; 32]),
        action,
        provider: OwnerId::new("roll")?,
        cause: mush_core::action::PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: p.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(n))),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?;
    let guards = f
        .engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())?
        .finish()?;
    let _ = f.engine.world.take_dirty();
    f.engine.stage_authoritative_action(p, guards, bundle)?;
    Ok(())
}
fn refusals(
    f: &PurchaseFixture,
    saved: &Durable,
    revision: &Revision,
    rating: RatingRef,
) -> TestResult {
    let params = parameters(revision, rating);
    let owner: OwnerTerm =
        serde_json::from_value(one(rows(&saved.disk, "roll", "roll_mind_owner_term"))?)?;
    for mode in [
        "foreign",
        "old_rating",
        "title",
        "before_stage",
        "before_take",
    ] {
        let mut w = boot(f, saved)?;
        let action = action(&w)?;
        let mut before = w.balances()?;
        let mut p = params.clone();
        if mode == "foreign" {
            assert!(w.prepare(&action, BUYER, START + 1, p).is_err());
        } else if mode == "old_rating" {
            let old: FiledRecord = serde_json::from_value(one(w.rows("roll", "roll_mind_filed"))?)?;
            *p.get_mut("rating_job").ok_or("rating job")? = json!(old.rating().job());
            *p.get_mut("rating_id").ok_or("rating id")? = json!(old.rating().id());
            *p.get_mut("rating_digest").ok_or("rating digest")? =
                json!(old.rating().receipt_digest());
            assert!(w.prepare(&action, GOD, START + 1, p).is_err());
        } else if mode == "title" {
            w.engine.step(
                Inbound::ConnUp {
                    conn: 91,
                    actor: GOD,
                    caps_structured: false,
                    from_linkdead: false,
                },
                &mut Outbox::new(),
                &mut mush_core::Effects::new(),
            );
            let charter = one(w.rows("roll", "roll_mind_charter"))?
                .get("id")
                .cloned()
                .ok_or("charter")?;
            let charter: mush_roll::commerce::CharterId = serde_json::from_value(charter)?;
            w.transfer_charter(charter.object(), BUYER, 49_001_100)?;
            // Native sale payment is asserted by the helper; the refused
            // compatible update must add no Registry fee or other debit.
            before = w.balances()?;
            assert_eq!(
                w.economy()?
                    .snapshot()
                    .asset(charter.object())
                    .ok_or("actual title")?
                    .title_owner,
                EconomicOwner::Object(BUYER)
            );
            assert!(w.prepare(&action, GOD, START + 1, p).is_err());
        } else {
            let (prepared, out) = w.engine.prepare_registered_composed_authoritative_action(
                &action,
                &request(START + 1, GOD, "roll", p.clone())?,
                token(START + 1),
            )?;
            if mode == "before_take" {
                stage(&mut w, START + 1, p.clone(), prepared, out)?;
                w.engine.world.set_now(owner.window().grace_end().get());
                assert!(w.engine.take_staged_action().is_err());
            } else {
                w.engine.world.set_now(owner.window().grace_end().get());
                assert!(stage(&mut w, START + 1, p, prepared, out).is_err());
            }
        }
        assert_eq!(w.balances()?, before);
        assert!(w.rows("roll", "roll_mind_line_update").is_empty());
    }
    // Once custody is taken, lapse cannot cancel the already-owned union.
    let mut w = boot(f, saved)?;
    let action = action(&w)?;
    let (p, o) = w.engine.prepare_registered_composed_authoritative_action(
        &action,
        &request(START + 2, GOD, "roll", params.clone())?,
        token(START + 2),
    )?;
    stage(&mut w, START + 2, params, p, o)?;
    let (bundle, installation) = w
        .engine
        .take_staged_action()?
        .ok_or("actual update custody")?
        .into_parts();
    w.engine.world.set_now(owner.window().grace_end().get());
    retain_writes(&mut w.disk, bundle.writes())?;
    w.engine.install_composed_action(installation)?;
    assert_eq!(w.rows("roll", "roll_mind_line_update").len(), 1);
    assert!(boot(f, &w.snapshot()).is_ok());
    Ok(())
}
fn corrupted(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    assert!(
        f.reboot(saved).is_err(),
        "default boot cannot drop retained update obligations"
    );
    for family in ["roll_mind_line_update", "roll_mind_line_update_position"] {
        let mut lost = saved.clone();
        assert_eq!(remove_family(&mut lost, "roll", family), 1);
        assert!(boot(f, &lost).is_err(), "missing update family");
    }
    for field in ["head", "next_sequence"] {
        let mut crossed = saved.clone();
        let line = crossed
            .disk
            .values_mut()
            .find(|v| v.pointer("/family/id") == Some(&json!("roll_mind_line")))
            .ok_or("line")?;
        *line
            .get_mut("value")
            .and_then(|v| v.get_mut(field))
            .ok_or("line field")? = if field == "head" {
            json!("1")
        } else {
            json!(4)
        };
        assert!(boot(f, &crossed).is_err(), "unproven head/sequence");
    }
    let mut crossed_action = saved.clone();
    let operation = crossed_action
        .disk
        .values_mut()
        .find(|v| v.pointer("/family/id") == Some(&json!("roll_mind_line_update")))
        .ok_or("actual update operation")?;
    *operation
        .get_mut("value")
        .and_then(|v| v.get_mut("action"))
        .and_then(|v| v.get_mut("digest"))
        .ok_or("actual update action digest")? =
        json!(mush_core::DefinitionDigest::from_bytes([17; 32]));
    assert!(
        boot(f, &crossed_action).is_err(),
        "same-locator crossed update action cannot replace actual historical proof"
    );
    let mut missing = saved.clone();
    missing
        .disk
        .retain(|(_, key), _| !key.starts_with("reference:roll:commercial:v1/line_update/"));
    assert!(boot(f, &missing).is_err(), "missing permanent update root");
    Ok(())
}
pub(super) fn exercise(f: &PurchaseFixture, granted: &Durable) -> TestResult {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let (saved, revision) = runtime.block_on(save(f, granted))?;
    let mut w = boot(f, &saved)?;
    let rating = registry::certify(&mut w, &revision)?;
    let certified = w.snapshot();
    // Branch from the already measured second revision before either branch
    // files it. This earns no additional certificate and preserves the original
    // immediate-update/captured-sortie scenario below.
    runtime.block_on(super::fork_tests::boxed_story_phase(
        "compatible_update.attested",
        || attested::exercise(f, &certified, &revision, rating),
    )?)?;
    runtime.block_on(super::fork_tests::boxed_story_phase(
        "compatible_update.follow",
        || follow::exercise(f, &certified, &revision, rating),
    )?)?;
    refusals(f, &certified, &revision, rating)?;
    runtime.block_on(captured::exercise(f, &certified, &revision, rating))?;
    let before_money = w.balances()?;
    let before_roots = w.roots()?;
    let old_file = one(w.rows("roll", "roll_mind_filed"))?;
    let owner = one(w.rows("roll", "roll_mind_owner_term"))?;
    let paid = one(w.rows("roll", "roll_mind_paid_term"))?;
    let title = w
        .economy()?
        .snapshot()
        .assets()
        .cloned()
        .collect::<Vec<_>>();
    let p = signed(&mut w, START + 3, parameters(&revision, rating))?;
    assert_eq!(w.balances()?, before_money);
    assert_eq!(w.roots()?, before_roots);
    let writes = w.install(p)?;
    assert!(
        writes
            .iter()
            .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
    );
    assert_eq!(
        w.balances()?.first().ok_or("payer balance")?.0,
        before_money
            .first()
            .ok_or("prior payer")?
            .0
            .checked_sub(Amount::from_minor(100))
            .ok_or("fee")?
    );
    assert_eq!(
        w.balances()?.get(1).ok_or("sink balance")?.0,
        before_money
            .get(1)
            .ok_or("prior sink")?
            .0
            .checked_add(Amount::from_minor(100))
            .ok_or("fee")?
    );
    assert!(
        w.economy()?.snapshot().assets().eq(title.iter()),
        "update changed title"
    );
    assert_eq!(one(w.rows("roll", "roll_mind_owner_term"))?, owner);
    assert_eq!(one(w.rows("roll", "roll_mind_paid_term"))?, paid);
    assert!(w.rows("roll", "roll_mind_filed").contains(&old_file));
    let new: FiledRecord = serde_json::from_value(
        w.rows("roll", "roll_mind_filed")
            .into_iter()
            .find(|v| v.get("format") == Some(&json!(3)))
            .ok_or("actual update file")?,
    )?;
    assert_eq!(new.content().revision().revision(), revision.reference());
    assert_eq!(new.rating(), &rating);
    assert_eq!(new.content().position().sequence.get(), 2);
    assert_eq!(w.roots()?.len(), before_roots.len() + 2);
    let updated = w.snapshot();
    assert!(boot(f, &updated).is_ok());
    corrupted(f, &updated)?;
    // New commerce resolves the actual head; the old paid Pin remains byte-exact.
    w.perform(
        &w.config.quotes.listing_action()?,
        GOD,
        START + 4,
        json!({"owner_licence":"1"}),
    )?;
    w.perform(
        &w.config.quotes.quote_action()?,
        BUYER,
        START + 5,
        json!({"listing":token(START+4)}),
    )?;
    let [fund, grant, _] = w.config.purchases.actions()?;
    w.perform(&fund, BUYER, START + 6, json!({"quote":token(START+5)}))?;
    w.perform(
        &grant,
        BUYER,
        START + 7,
        json!({"operation":token(START+6)}),
    )?;
    assert!(w.rows("roll", "roll_mind_paid_term").contains(&paid));
    let newest: PaidTermRecord = serde_json::from_value(
        w.rows("roll", "roll_mind_paid_term")
            .into_iter()
            .find(|v| v.get("operation") == Some(&json!(token(START + 6))))
            .ok_or("new actual paid Pin")?,
    )?;
    assert!(
        matches!(newest.policy(),mush_roll::commerce::records::RevisionPolicy::Pin{filed,revision:identity} if filed==new.content().id()&&identity.revision()==revision.reference())
    );
    assert!(boot(f, &w.snapshot()).is_ok());
    // The separate captured branch began its old paid sortie before the
    // same actual update; its public EXIT proves the immutable seal continues.
    Ok(())
}
