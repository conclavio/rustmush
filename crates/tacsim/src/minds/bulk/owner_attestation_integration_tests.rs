//! Actual free owner generations over the existing measured Registry and paid
//! histories. In-process installation models the writer ACK; no new certificate
//! or paid-grant constructor is used here.
use super::*;
const START: u128 = 23_000_000;
const ACTION: &str = "attest_owner_mind";
const OPS: &str = "roll_mind_owner_attestation_operation";
fn boot(f: &PurchaseFixture, saved: &Durable) -> TestResult<PlayerWorld> {
    PlayerWorld::boot_owner_attestation(f, saved)
}
fn params(owner: LicenceId) -> Value {
    json!({"owner_licence":owner.get()})
}
fn prepare_attest(
    w: &mut PlayerWorld,
    actor: ObjId,
    n: u128,
    p: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    super::super::prepare(w, "roll", ACTION, 1, actor, n, p)
}
fn signed(w: &mut PlayerWorld, actor: ObjId, n: u128, p: Value) -> TestResult<PreparedComposedTxn> {
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x61; 32])?,
        [],
    )?);
    let exact = action(w, "roll", ACTION, 1)?;
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&exact)
        .ok_or("actual owner attestation offer")?;
    let inv = invocation(n, actor, "roll", p.clone())?;
    let resolved = w.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: inv.request(),
            principal,
            actor,
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
    let (actual, request, _) = resolved.into_authoritative_parts();
    assert_eq!(actual, exact);
    Ok(w.engine
        .prepare_registered_composed_resolved_authoritative_action(&exact, &request, token(n))?
        .0)
}
fn stage_attest(
    w: &mut PlayerWorld,
    actor: ObjId,
    n: u128,
    p: Value,
    prepared: PreparedComposedTxn,
    out: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let exact = action(w, "roll", ACTION, 1)?;
    let inv = invocation(n, actor, "roll", p)?;
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

fn term(w: &PlayerWorld, owner: LicenceId, generation: u64) -> TestResult<OwnerTerm> {
    Ok(serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_owner_term")
            .into_iter()
            .find(|v| {
                v.get("licence") == Some(&json!(owner))
                    && v.get("generation") == Some(&json!(generation.to_string()))
            })
            .ok_or("actual owner generation")?,
    )?)
}
fn current(w: &PlayerWorld, owner: LicenceId) -> TestResult<u64> {
    let row = rows(&disk(w), "roll", "roll_mind_owner_licence")
        .into_iter()
        .find(|v| v.get("id") == Some(&json!(owner)))
        .ok_or("actual owner licence")?;
    Ok(row
        .get("term")
        .and_then(Value::as_str)
        .ok_or("generation")?
        .parse()?)
}
fn unchanged(w: &PlayerWorld, before: &[((String, String), Value)]) -> TestResult {
    let actual = disk(w);
    for (key, value) in before {
        assert!(
            actual.get(key) == Some(value),
            "attestation changed retained immutable history"
        );
    }
    Ok(())
}
fn install(w: &mut PlayerWorld, actor: ObjId, n: u128, owner: LicenceId) -> TestResult {
    let before = balances(w)?;
    let p = signed(w, actor, n, params(owner))?;
    assert_eq!(balances(w)?, before);
    let (writes, installation) = p.into_parts();
    apply(w, &writes, installation)?;
    assert_eq!(balances(w)?, before, "free owner attestation changed money");
    Ok(())
}
async fn fences(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    parent_end: i64,
) -> TestResult {
    for mode in ["foreign", "clock", "lapse_stage", "lapse_take", "title"] {
        let mut w = boot(f, saved)?;
        let tasks = w.tasks.clone();
        let result: TestResult = (|| {
            let before = disk(&w);
            let money = balances(&w)?;
            if mode == "foreign" {
                assert!(prepare_attest(&mut w, GOD, START + 30, params(owner)).is_err());
            } else {
                let (prepared, out) = prepare_attest(&mut w, BUYER, START + 30, params(owner))?;
                if mode == "title" {
                    // Core admits one pending commit: the charter's title cannot
                    // move under a prepared attestation; the transfer is refused.
                    let birth = rows(&disk(&w), "roll", "roll_mind_initial_filing")
                        .into_iter()
                        .find(|v| v.get("licence") == Some(&json!(owner)))
                        .ok_or("birth")?;
                    let charter: mush_roll::commerce::CharterId = serde_json::from_value(
                        birth.get("charter").ok_or("charter")?.clone(),
                    )?;
                    assert!(
                        w.transfer_charter(charter.object(), GOD, 49_000_100).is_err(),
                        "title moved under a prepared attestation"
                    );
                    drop((prepared, out));
                } else if mode == "lapse_take" {
                    stage_attest(&mut w, BUYER, START + 30, params(owner), prepared, out)?;
                    w.engine.world.set_now(parent_end);
                    assert!(w.engine.take_staged_action().is_err());
                } else {
                    match mode {
                        "clock" => {
                            let epoch = w.engine.world.next_world_timeline_epoch(2)?;
                            w.engine.world.append_world_timeline_epoch(epoch)?;
                        }
                        _ => w.engine.world.set_now(parent_end),
                    }
                    assert!(
                        stage_attest(&mut w, BUYER, START + 30, params(owner), prepared, out)
                            .is_err()
                    );
                }
            }
            assert_eq!(balances(&w)?, money);
            assert!(rows(&disk(&w), "roll", OPS).is_empty());
            assert!(disk(&w) == before, "refused attestation changed custody");
            Ok(())
        })();
        tasks.shutdown(Duration::from_secs(5)).await;
        result?;
    }
    let mut w = boot(f, saved)?;
    let tasks = w.tasks.clone();
    let result: TestResult = (|| {
        let (p, o) = prepare_attest(&mut w, BUYER, START + 31, params(owner))?;
        stage_attest(&mut w, BUYER, START + 31, params(owner), p, o)?;
        let (bundle, installation) = w
            .engine
            .take_staged_action()?
            .ok_or("actual attestation custody")?
            .into_parts();
        w.engine.world.set_now(parent_end);
        apply(&mut w, bundle.writes(), installation)?;
        assert_eq!(current(&w, owner)?, 2);
        source_answer(&mut w, BUYER, owner, None)?;
        Ok(())
    })();
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
// Original renewal has no ParentGrant read; its own captured actual Economy
// definition readiness must still fence title authority through custody.
async fn original_economy_freeze(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    for at in 0..3 {
        let mut w = boot(f, saved)?;
        let tasks = w.tasks.clone();
        let result: TestResult = (|| {
            let before = disk(&w);
            let money = balances(&w)?;
            let owner = LicenceId::new(1)?;
            let freeze = |w: &PlayerWorld| -> TestResult {
                w.engine.registry.freeze_mechanics_owner(
                    &OwnerId::new("economy")?,
                    mush_core::mechanics::IncidentId(88),
                    mush_core::mechanics::RecoveryReason::IntegrityViolation {
                        code: mush_core::mechanics::CanonicalId::new("attestation_fixture")?,
                    },
                )?;
                Ok(())
            };
            if at == 0 {
                freeze(&w)?;
                assert!(prepare_attest(&mut w, GOD, START + 32, params(owner)).is_err());
            } else {
                let (prepared, output) = prepare_attest(&mut w, GOD, START + 32, params(owner))?;
                if at == 1 {
                    freeze(&w)?;
                    assert!(
                        stage_attest(&mut w, GOD, START + 32, params(owner), prepared, output)
                            .is_err()
                    );
                } else {
                    stage_attest(&mut w, GOD, START + 32, params(owner), prepared, output)?;
                    freeze(&w)?;
                    assert!(w.engine.take_staged_action().is_err());
                }
            }
            assert_eq!(balances(&w)?, money);
            assert!(
                disk(&w) == before,
                "frozen Economy published an original owner generation"
            );
            Ok(())
        })();
        tasks.shutdown(Duration::from_secs(5)).await;
        result?;
    }
    Ok(())
}
async fn corruptions(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    assert!(
        PlayerWorld::boot_owner_qualification(f, saved).is_err(),
        "disabled attestation accepted retained generations"
    );
    for mode in [
        "operation",
        "term",
        "root",
        "pointer",
        "predecessor",
        "parent",
    ] {
        let mut bad = saved.clone();
        match mode {
            "operation" | "term" => {
                let key = bad
                    .disk
                    .iter()
                    .find(|(_, v)| {
                        let family = v.pointer("/family/id").and_then(Value::as_str);
                        if mode == "operation" {
                            family == Some(OPS)
                        } else {
                            family == Some("roll_mind_owner_term")
                                && v.pointer("/value/generation") == Some(&json!("2"))
                        }
                    })
                    .map(|(k, _)| k.clone())
                    .ok_or("retained target")?;
                bad.disk.remove(&key).ok_or("retained target")?;
            }
            "root" => {
                let key = bad
                    .disk
                    .keys()
                    .find(|(ns, k)| ns == "roll" && k.contains("owner_attestation/"))
                    .cloned()
                    .ok_or("cause root")?;
                bad.disk.remove(&key).ok_or("cause root")?;
            }
            "pointer" => {
                let v = bad
                    .disk
                    .values_mut()
                    .find(|v| {
                        v.pointer("/family/id").and_then(Value::as_str)
                            == Some("roll_mind_owner_licence")
                            && v.pointer("/value/format") == Some(&json!(2))
                    })
                    .ok_or("current licence")?;
                *v.pointer_mut("/value/format").ok_or("format")? = json!(1);
                *v.pointer_mut("/value/term").ok_or("term")? = json!("1");
            }
            "predecessor" => {
                let v = bad
                    .disk
                    .values_mut()
                    .find(|v| v.pointer("/family/id").and_then(Value::as_str) == Some(OPS))
                    .ok_or("cause")?;
                *v.pointer_mut("/value/predecessor_digest").ok_or("digest")? =
                    json!(mush_core::DefinitionDigest::from_bytes([0xff; 32]));
            }
            _ => {
                let v = bad
                    .disk
                    .values_mut()
                    .find(|v| {
                        v.pointer("/family/id").and_then(Value::as_str) == Some(OPS)
                            && v.pointer("/value/next/attestation/parent/selected")
                                .is_some()
                    })
                    .ok_or("parent cause")?;
                *v.pointer_mut("/value/next/attestation/parent/selected/terminal")
                    .ok_or("terminal")? = json!(token(START + 999));
            }
        }
        assert!(
            boot(f, &bad).is_err(),
            "cold generation/cause corruption became current authority"
        );
    }
    Ok(())
}
pub(super) async fn exercise(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    customer: &EntitlementAllocationV1,
) -> TestResult {
    let mut tasks = Vec::new();
    let result: TestResult = async {
        let mut w = boot(f, saved)?;
        tasks.push(w.tasks.clone());
        let child = term(&w, owner, 1)?;
        let source = super::child_source(&w, owner)?;
        let selected = rows(&disk(&w), "roll", "roll_mind_owner_qualification_operation")
            .into_iter()
            .filter(|v| v.get("owner_licence") == Some(&json!(owner)))
            .max_by_key(|v| v.get("ordinal").and_then(Value::as_u64).unwrap_or(0))
            .ok_or("actual qualification")?;
        let parent_end = selected
            .pointer("/selected_parent/window/grace_end")
            .and_then(Value::as_i64)
            .ok_or("parent end")?;
        assert!(w.engine.world.game_now_unix() >= child.window().grace_end().get());
        assert!(parent_end > w.engine.world.game_now_unix());
        super::super::story_phase("attestation.fences", || fences(f, saved, owner, parent_end))?
            .await?;
        super::super::story_phase("attestation.economy_freeze", || {
            original_economy_freeze(f, saved)
        })?
        .await?;
        let retained = super::retained(&w);
        source_answer(&mut w, BUYER, owner, None)?;
        install(&mut w, BUYER, START + 40, owner)?;
        assert_eq!(current(&w, owner)?, 2);
        let inherited = term(&w, owner, 2)?;
        assert_eq!(inherited.window().grace_end().get(), parent_end);
        assert_eq!(
            inherited
                .window()
                .phase_at(mush_roll::commerce::WorldIcSeconds::from_stamp(
                    &w.engine.world.world_ic_now()
                )?),
            mush_roll::commerce::TermPhase::Grace
        );
        assert!(
            prepare_attest(&mut w, BUYER, START + 41, params(owner)).is_err(),
            "same inherited Grace interval churned generations"
        );
        unchanged(&w, &retained)?;
        source_answer(&mut w, BUYER, owner, Some(source))?;
        let generation_two = capture(&w, &disk(&w));
        let mut cold_two = boot(f, &generation_two)?;
        tasks.push(cold_two.tasks.clone());
        assert_eq!(current(&cold_two, owner)?, 2);
        assert!(
            serde_json::to_value(term(&cold_two, owner, 2)?)? == serde_json::to_value(&inherited)?,
            "cold dependent generation changed its attested window"
        );
        source_answer(&mut cold_two, BUYER, owner, Some(source))?;
        unchanged(&cold_two, &retained)?;
        source_answer(
            &mut w,
            GOD,
            customer.licence(),
            Some(customer.revision().source()),
        )?;
        // Renew the actual original author's own licence, enabling lawful fresh
        // parent purchases without modifying any earlier customer grant.
        let original = LicenceId::new(1)?;
        install(&mut w, GOD, START + 50, original)?;
        assert_eq!(current(&w, original)?, 2);
        source_answer(&mut w, GOD, original, None)?;
        assert!(
            prepare_attest(&mut w, GOD, START + 51, params(original)).is_err(),
            "active term truncated"
        );
        assert!(prepare_attest(&mut w, GOD, START + 52, json!({"owner_licence":"1"})).is_err());
        let open = super::parent_purchase(&mut w, START + 60, "open")?;
        super::perform(&mut w, START + 70, super::params(owner, open.licence()))?;
        assert_eq!(
            current(&w, owner)?,
            2,
            "qualification extended current term"
        );
        install(&mut w, BUYER, START + 80, owner)?;
        assert_eq!(current(&w, owner)?, 3);
        let next = term(&w, owner, 3)?;
        assert!(next.window().grace_end() > inherited.window().grace_end());
        assert!(next.window().grace_end() <= own_term(&w, open.licence())?.window().grace_end());
        unchanged(&w, &retained)?;
        source_answer(&mut w, BUYER, owner, Some(source))?;
        assert!(
            super::super::prepare(
                &mut w,
                "market",
                "list_derived_mind",
                1,
                BUYER,
                START + 81,
                listing_params(owner, "open_resale", 100)
            )
            .is_err(),
            "Open attestation restored resale"
        );
        source_answer(
            &mut w,
            GOD,
            customer.licence(),
            Some(customer.revision().source()),
        )?;
        let saved = capture(&w, &disk(&w));
        super::super::story_phase("attestation.corruptions", || corruptions(f, &saved))?.await?;
        let mut cold = boot(f, &saved)?;
        tasks.push(cold.tasks.clone());
        assert_eq!(current(&cold, owner)?, 3);
        source_answer(&mut cold, BUYER, owner, Some(source))?;
        let resale = super::parent_purchase(&mut cold, START + 90, "open_resale")?;
        super::perform(
            &mut cold,
            START + 100,
            super::params(owner, resale.licence()),
        )?;
        assert!(serde_json::to_value(term(&cold, owner, 3)?)? == serde_json::to_value(next)?);
        list(&mut cold, owner, START + 101)?;
        Ok(())
    }
    .await;
    for task in tasks {
        task.shutdown(Duration::from_secs(5)).await;
    }
    result
}
