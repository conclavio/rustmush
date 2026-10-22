//! Actual filed derivative -> authored compatible revision -> real certificate
//! -> fee/typed/root ACK. Existing customer Pins and source acquisition stay fixed.
use super::*;
use mush_core::{Audience, Inbound, OfferExecutionContext, OfferSubmission};
use mush_proto::{
    InvokeRequest, InvokeResult, MindEditBase, MindRequest, MindResponse, OutputEvent,
};
#[path = "derived_update_integration_tests/registry.rs"]
mod registry;
const ACTION: &str = "update_derived_mind";
const START: u128 = 81_000_000;
fn boot(f: &PurchaseFixture, s: &Durable) -> TestResult<PlayerWorld> {
    PlayerWorld::boot_derived_updates(f, s)
}
fn current_source(w: &PlayerWorld) -> TestResult<Revision> {
    let r = w.engine.world.module_state::<RollStore>().ok_or("Roll")?;
    let m = r.mind(BUYER, "child")?;
    Ok(r.revision(BUYER, m.latest().ok_or("saved child")?)?.clone())
}
fn parent_end(w: &PlayerWorld) -> TestResult<i64> {
    let binding = one(rows(&disk(w), "roll", "roll_mind_owner_parent"))?;
    let licence: LicenceId = serde_json::from_value(
        binding
            .pointer("/parent/licence")
            .ok_or("parent licence")?
            .clone(),
    )?;
    let term: PaidTermRecord = serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(licence)))
            .ok_or("actual parent term")?,
    )?;
    Ok(term.window().grace_end().get())
}
async fn save(f: &PurchaseFixture, saved: &Durable) -> TestResult<(Durable, Revision)> {
    let mut w = boot(f, saved)?;
    let tasks = w.tasks.clone();
    let result: TestResult<(Durable, Revision)> = async {
  let old=current_source(&w)?;
  let events=w.say(BUYER,"@mind/insert child 1=// compatible derived update")?;
  assert!(PlayerWorld::private_text(&events,BUYER).contains("Working source updated."));
  let base=w.engine.world.module_state::<RollStore>().ok_or("Roll")?.mind(BUYER,"child")?.base();
  let events=w.step(Inbound::Invoke{actor:BUYER,conn:BUYER_CONN,id:START as u64,request:InvokeRequest::Mind(MindRequest::Save{name:"child".into(),base:MindEditBase{edit:base.edit,revision:base.revision},notes:"compatible derived update".into(),version:None})})?;
  assert!(events.iter().any(|(a,e)|*a==Audience::Conn(BUYER_CONN)&&matches!(e,OutputEvent::InvokeResult{id,result:InvokeResult::Mind(MindResponse::Queued{..})}if *id==START as u64)));
  let deadline=Instant::now()+Duration::from_secs(60);
  let next=loop{let next=current_source(&w)?;if next.reference()!=old.reference(){break next}
   if Instant::now()>=deadline{return Err("actual derived Save did not complete".into())}
   let when=w.engine.sched.next_due().ok_or("author scheduling")?;w.engine.fire_due(when,&mut Outbox::new(),&mut mush_core::Effects::new());w.commit()?;
   tokio::time::sleep(Duration::from_millis(2)).await;
  };
  assert_eq!(next.envelope(),old.envelope());assert_eq!(serde_json::to_value(&next)?.get("provenance"),serde_json::to_value(&old)?.get("provenance"));
  assert_ne!(next.source(),old.source());
  assert_ne!(mush_roll::commerce::records::canonical_duplicate_digest(&next)?,mush_roll::commerce::records::canonical_duplicate_digest(&old)?);assert!(next.require_original_access().is_err());
  assert_eq!(rows(&disk(&w),"roll","roll_mind_draft_parent"),rows(&saved.disk,"roll","roll_mind_draft_parent"));
  Ok((capture(&w,&disk(&w)),next))
 }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
fn parameters(line: mush_roll::commerce::LineId, r: &Revision, rating: &RatingRef) -> Value {
    let r = r.reference();
    json!({"line":line.get(),"mind":r.key.mind,"revision":r.key.number,"revision_digest":r.digest,
 "rating_job":rating.job(),"rating_id":rating.id(),"rating_digest":rating.receipt_digest(),"payer":2})
}
fn signed(w: &mut PlayerWorld, n: u128, p: Value) -> TestResult<PreparedComposedTxn> {
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x71; 32])?,
        [],
    )?);
    let exact = action(w, "roll", ACTION, 1)?;
    let principal = PrincipalId::Object { id: BUYER };
    let query = w
        .engine
        .legal_action_query(principal.clone(), BUYER, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let projected = offers.projected();
    let mut matches = projected
        .iter()
        .filter(|o| o.presentation().verb == "roll-update-derived-mind");
    let offer = matches.next().ok_or("actual derived update offer")?;
    check!(matches.next().is_none());
    let inv = invocation(n, BUYER, "roll", p.clone())?;
    let resolved = w.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: inv.request(),
            principal,
            actor: BUYER,
            idempotency: inv.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: offer.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: p,
        },
    )?;
    let (actual, inv, _) = resolved.into_authoritative_parts();
    check_eq!(actual, exact);
    Ok(w.engine
        .prepare_registered_composed_resolved_authoritative_action(&exact, &inv, token(n))?
        .0)
}
fn stage_update(
    w: &mut PlayerWorld,
    n: u128,
    p: Value,
    prepared: PreparedComposedTxn,
    out: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let exact = action(w, "roll", ACTION, 1)?;
    let inv = invocation(n, BUYER, "roll", p)?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        out.into_parts();
    definitions.push(exact.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(n),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([61; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([62; 32]),
        action: exact,
        provider: OwnerId::new("roll")?,
        cause: mush_core::action::PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: prepared.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(n))),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?;
    let guards = w
        .engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())?
        .finish()?;
    let _ = w.engine.world.take_dirty();
    w.engine
        .stage_authoritative_action(prepared, guards, bundle)?;
    Ok(())
}
fn cash(w: &PlayerWorld) -> TestResult<Amount> {
    Ok(w.engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .snapshot()
        .account(WalletId(2))
        .ok_or("actual payer")?
        .posted_balance)
}
pub(super) async fn exercise(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    paid: &EntitlementAllocationV1,
) -> TestResult {
    // Compare typed ids: the content's serialised field is `filed`, not `id`.
    let initial: FiledRecord = rows(&saved.disk, "roll", "roll_mind_filed")
        .into_iter()
        .filter_map(|v| serde_json::from_value::<FiledRecord>(v).ok())
        .find(|r| r.content().id() == paid.filed())
        .ok_or("actual initial child")?;
    let (edited, revision) = save(f, saved).await?;
    let (certified, rating) = registry::certify(f, &edited).await?;
    let mut w = boot(f, &certified)?;
    let tasks = w.tasks.clone();
    let result: TestResult = async {
        let line = initial.content().position().line;
        let params = parameters(line, &revision, &rating);
        let before = cash(&w)?;
        let prior = disk(&w);
        assert!(crate::test_checks::is_refusal(prepare(
            &mut w,
            "roll",
            ACTION,
            1,
            GOD,
            START + 1,
            params.clone()
        )));
        let mut old = params.clone();
        *old.get_mut("rating_job").ok_or("rating")? = json!(initial.rating().job());
        *old.get_mut("rating_id").ok_or("rating")? = json!(initial.rating().id());
        *old.get_mut("rating_digest").ok_or("rating")? = json!(initial.rating().receipt_digest());
        assert!(crate::test_checks::is_refusal(prepare(
            &mut w,
            "roll",
            ACTION,
            1,
            BUYER,
            START + 1,
            old
        )));
        assert_eq!(disk(&w), prior);
        let p = signed(&mut w, START + 1, params.clone())?;
        assert_eq!(cash(&w)?, before);
        let (writes, install) = p.into_parts();
        apply(&mut w, &writes, install)?;
        assert_eq!(
            cash(&w)?,
            before.checked_sub(Amount::from_minor(100)).ok_or("fee")?
        );
        for family in [
            "roll_mind_initial_filing",
            "roll_mind_owner_term",
            "roll_mind_owner_parent",
            "roll_mind_draft_parent",
            "roll_mind_paid_term",
            "roll_mind_paid_term_economics",
        ] {
            assert_eq!(
                rows(&disk(&w), "roll", family),
                rows(&saved.disk, "roll", family),
                "immutable accepted rows"
            );
        }
        let updated: FiledRecord = serde_json::from_value(
            rows(&disk(&w), "roll", "roll_mind_filed")
                .into_iter()
                .find(|v| v.get("operation") == Some(&json!(token(START + 1))))
                .ok_or("actual derived update")?,
        )?;
        assert_eq!(updated.content().parent(), initial.content().parent());
        assert_eq!(updated.content().position().line, line);
        assert_eq!(updated.content().position().sequence.get(), 2);
        assert_eq!(
            updated.content().revision().revision(),
            revision.reference()
        );
        source_answer(&mut w, BUYER, owner, Some(revision.source()))?;
        source_answer(&mut w, GOD, paid.licence(), Some(paid.revision().source()))?;
        // A new real listing materializes the original acquisition royalty chain for
        // the selected updated revision, independently of its current qualification.
        for actor in [GOD, BUYER] {
            w.engine.world.move_to(actor, ROOM)?;
        }
        w.commit()?;
        commit(
            &mut w,
            "market",
            "list_derived_mind",
            1,
            BUYER,
            START + 2,
            // `royalty_permille` is a required listing parameter (as in the
            // entitlement story's `listing_params`).
            json!({"owner_licence":owner,"form":"open","royalty_permille":100}),
        )?;
        let old_royalty = rows(&saved.disk, "roll", "roll_mind_filed_royalty_terms")
            .into_iter()
            .find(|v| v.get("filed") == Some(&json!(initial.content().id())))
            .ok_or("accepted royalties")?;
        let next_royalty = rows(&disk(&w), "roll", "roll_mind_filed_royalty_terms")
            .into_iter()
            .find(|v| v.get("filed") == Some(&json!(updated.content().id())))
            .ok_or("updated royalties")?;
        assert_eq!(old_royalty.get("edges"), next_royalty.get("edges"));
        // Requalification still uses immutable birth, while its actual current-line
        // preimage proves the new compatible head.
        let parent_licence: LicenceId = serde_json::from_value(
            one(rows(&saved.disk, "roll", "roll_mind_owner_parent"))?
                .pointer("/parent/licence")
                .ok_or("parent")?
                .clone(),
        )?;
        commit(
            &mut w,
            "roll",
            "replace_owner_parent_qualification",
            1,
            BUYER,
            START + 3,
            json!({"owner_licence":owner.get(),"parent_licence":parent_licence.get()}),
        )?;
        let after = capture(&w, &disk(&w));
        assert!(
            crate::test_checks::is_refusal(PlayerWorld::boot_entitlements(f, &after)),
            "missing derived protocol must refuse history"
        );
        let mut cold = boot(f, &after)?;
        source_answer(&mut cold, BUYER, owner, Some(revision.source()))?;
        source_answer(
            &mut cold,
            GOD,
            paid.licence(),
            Some(paid.revision().source()),
        )?;
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        for family in ["roll_mind_line_update", "roll_mind_line_update_position"] {
            let mut lost = after.clone();
            lost.disk.retain(|(ns, _), v| {
                !(ns == "roll" && v.pointer("/family/id") == Some(&json!(family)))
            });
            assert!(crate::test_checks::is_refusal(boot(f, &lost)));
        }
        let mut crossed = after.clone();
        let value = crossed
            .disk
            .values_mut()
            .find(|v| v.pointer("/family/id") == Some(&json!("roll_mind_line_update")))
            .ok_or("operation envelope")?;
        *value
            .pointer_mut("/value/parent/selected/terminal")
            .ok_or("actual parent terminal")? = json!(token(START + 99));
        assert!(crate::test_checks::is_refusal(boot(f, &crossed)));
        // Exact parent window and owner readiness remain live at both custody fences.
        for change in ["parent_lapse", "clock", "economy"] {
            for take in [false, true] {
                let mut stale = boot(f, &certified)?;
                let mut p = Some(prepare(
                    &mut stale,
                    "roll",
                    ACTION,
                    1,
                    BUYER,
                    START + 4,
                    params.clone(),
                )?);
                if take {
                    let (p, o) = p.take().ok_or("preparation")?;
                    stage_update(&mut stale, START + 4, params.clone(), p, o)?;
                }
                match change {
                    "parent_lapse" => {
                        let end = parent_end(&stale)?;
                        stale.engine.world.set_now(end);
                    }
                    "clock" => {
                        let epoch = stale.engine.world.next_world_timeline_epoch(2)?;
                        stale.engine.world.append_world_timeline_epoch(epoch)?;
                    }
                    _ => {
                        stale.engine.registry.freeze_mechanics_owner(
                            &OwnerId::new("economy")?,
                            mush_core::mechanics::IncidentId(89),
                            mush_core::mechanics::RecoveryReason::IntegrityViolation {
                                code: mush_core::mechanics::CanonicalId::new(
                                    "derived_update_fixture",
                                )?,
                            },
                        )?;
                    }
                }
                if take {
                    assert!(stale.engine.take_staged_action().is_err());
                } else {
                    let (p, o) = p.take().ok_or("preparation")?;
                    assert!(stage_update(&mut stale, START + 4, params.clone(), p, o).is_err());
                }
                assert_eq!(cash(&stale)?, before);
                assert!(rows(&disk(&stale), "roll", "roll_mind_line_update").is_empty());
                stale.tasks.shutdown(Duration::from_secs(5)).await;
            }
        }
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
