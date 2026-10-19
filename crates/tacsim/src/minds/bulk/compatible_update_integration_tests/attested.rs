//! Same genuine second certificate, independently published after a real free
//! owner attestation. No additional machine run, clock reset or fake receipt.
use super::*;
use mush_roll::commerce::{TermPhase, WorldIcSeconds};
const START: u128 = 24_000_000;
const OPS: &str = "roll_mind_owner_attestation_operation";
fn boot(f: &PurchaseFixture, saved: &Durable) -> TestResult<PlayerWorld> {
    PlayerWorld::boot_updates_with_attestation(f, saved)
}
fn action(w: &PlayerWorld, id: &str) -> TestResult<DefinitionRef> {
    w.engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|entry| entry.reference())
        .find(|r| {
            r.key().owner.as_str() == "roll"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == 1
        })
        .cloned()
        .ok_or_else(|| "actual registered owner action".into())
}
fn signed(
    w: &mut PlayerWorld,
    id: &str,
    n: u128,
    params: Value,
) -> TestResult<PreparedComposedTxn> {
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x62; 32])?,
        [],
    )?);
    let exact = action(w, id)?;
    let principal = PrincipalId::Object { id: GOD };
    let query = w
        .engine
        .legal_action_query(principal.clone(), GOD, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&exact)
        .ok_or("actual signed owner action")?;
    let invocation = request(n, GOD, "roll", params.clone())?;
    let resolved = w.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: invocation.request(),
            principal,
            actor: GOD,
            idempotency: invocation.idempotency().clone(),
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
    let (actual, resolved, _) = resolved.into_authoritative_parts();
    assert_eq!(actual, exact);
    Ok(w.engine
        .prepare_registered_composed_resolved_authoritative_action(&exact, &resolved, token(n))?
        .0)
}
fn balances(w: &PlayerWorld) -> TestResult<Vec<(Amount, Amount)>> {
    let economy = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("actual Economy")?;
    [0, 1, 2]
        .into_iter()
        .map(|id| {
            let a = economy
                .snapshot()
                .account(WalletId(id))
                .ok_or("actual account")?;
            Ok((a.posted_balance, a.held_amount))
        })
        .collect()
}
fn owner(w: &PlayerWorld, generation: u64) -> TestResult<OwnerTerm> {
    Ok(serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_owner_term")
            .into_iter()
            .find(|v| {
                v.get("licence") == Some(&json!("1"))
                    && v.get("generation") == Some(&json!(generation.to_string()))
            })
            .ok_or("actual owner generation")?,
    )?)
}
fn current(w: &PlayerWorld) -> TestResult<u64> {
    let row = one(rows(&disk(w), "roll", "roll_mind_owner_licence"))?;
    Ok(row
        .get("term")
        .and_then(Value::as_str)
        .ok_or("current generation")?
        .parse()?)
}
fn attest(w: &mut PlayerWorld, n: u128) -> TestResult {
    let before = balances(w)?;
    let p = signed(w, "attest_owner_mind", n, json!({"owner_licence":1}))?;
    let (writes, install) = p.into_parts();
    captured::apply(w, &writes, install)?;
    assert_eq!(balances(w)?, before, "free attestation changed balances");
    Ok(())
}
fn owner_available(w: &mut PlayerWorld, expected: bool) -> TestResult {
    let events = w.say(GOD, "SIM MIND /filed")?;
    let text = PlayerWorld::private_text(&events, GOD);
    assert_eq!(
        text.contains("filed:1:"),
        expected,
        "actual current owner projection"
    );
    Ok(())
}
async fn cold_refusals(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    for mode in [
        "cause",
        "term",
        "root",
        "old_format",
        "old_generation",
        "crossed_cause",
    ] {
        let mut corrupt = saved.clone();
        let count = corrupt.disk.len();
        match mode {
            "cause" => corrupt.disk.retain(|_, v| {
                !(v.pointer("/family/id") == Some(&json!(OPS))
                    && v.pointer("/value/operation") == Some(&json!(token(START))))
            }),
            "term" => corrupt.disk.retain(|_, v| {
                !(v.pointer("/family/id") == Some(&json!("roll_mind_owner_term"))
                    && v.pointer("/value/generation") == Some(&json!("2")))
            }),
            "root" => corrupt.disk.retain(|(_, k), _| {
                !k.starts_with(&format!(
                    "reference:roll:commercial:v1/owner_attestation/{}/",
                    token(START)
                ))
            }),
            "crossed_cause" => {
                let row = corrupt
                    .disk
                    .values_mut()
                    .find(|v| {
                        v.pointer("/family/id") == Some(&json!("roll_mind_owner_term"))
                            && v.pointer("/value/generation") == Some(&json!("2"))
                    })
                    .ok_or("second term")?;
                *row.pointer_mut("/value/operation").ok_or("term cause")? =
                    json!(token(START + 99));
            }
            _ => {
                let row = corrupt
                    .disk
                    .values_mut()
                    .find(|v| v.pointer("/family/id") == Some(&json!("roll_mind_line_update")))
                    .ok_or("actual line update")?;
                *row.pointer_mut("/value/format").ok_or("format")? = json!(1);
                if mode == "old_generation" {
                    *row.pointer_mut("/value/term").ok_or("term")? = json!("1");
                }
            }
        }
        if matches!(mode, "cause" | "term" | "root") {
            assert_eq!(
                corrupt.disk.len().checked_add(1),
                Some(count),
                "exact historical evidence removed"
            );
        }
        if let Ok(world) = boot(f, &corrupt) {
            world.tasks.shutdown(Duration::from_secs(5)).await;
            return Err("corrupted attested update history passed cold boot".into());
        }
    }
    Ok(())
}
pub(super) async fn exercise(
    f: &PurchaseFixture,
    certified: &Durable,
    revision: &Revision,
    rating: RatingRef,
) -> TestResult {
    super::super::fork_tests::boxed_story_phase("compatible_update.chartered", || {
        chartered(f, certified, revision, rating)
    })?
    .await?;
    let mut w = boot(f, certified)?;
    let mut tasks = vec![w.tasks.clone()];
    let result: TestResult = async {
        let birth = owner(&w, 1)?;
        let old_file: FiledRecord =
            serde_json::from_value(one(rows(&disk(&w), "roll", "roll_mind_filed"))?)?;
        let paid_value = one(rows(&disk(&w), "roll", "roll_mind_paid_term"))?;
        let paid: PaidTermRecord = serde_json::from_value(paid_value.clone())?;
        let before_certificates = rows(&disk(&w), "roll", "rating_receipts");
        assert!(
            !before_certificates.is_empty(),
            "actual measured certificates retained"
        );
        assert!(w.engine.world.world_ic_now().at < birth.window().grace_end().get());
        w.engine.world.set_now(birth.window().grace_end().get());
        let now = WorldIcSeconds::from_stamp(&w.engine.world.world_ic_now())?;
        assert_eq!(birth.window().phase_at(now), TermPhase::Lapsed);
        assert!(matches!(
            paid.window().phase_at(now),
            TermPhase::Active | TermPhase::Grace
        ));
        owner_available(&mut w, false)?;
        let update = action(&w, "update_original_mind")?;
        assert!(
            w.engine
                .prepare_registered_composed_authoritative_action(
                    &update,
                    &request(START + 10, GOD, "roll", parameters(revision, rating))?,
                    token(START + 10)
                )
                .is_err(),
            "lapsed predecessor cannot authorize compatible update"
        );
        attest(&mut w, START)?;
        assert_eq!(current(&w)?, 2);
        let term2 = owner(&w, 2)?;
        let cause = one(rows(&disk(&w), "roll", OPS))?;
        let before = balances(&w)?;
        let p = signed(
            &mut w,
            "update_original_mind",
            START + 1,
            parameters(revision, rating),
        )?;
        assert_eq!(balances(&w)?, before);
        let (writes, install) = p.into_parts();
        assert!(
            writes
                .iter()
                .any(|v| matches!(v, ReplayWrite::LedgerChunkSet { .. }))
        );
        captured::apply(&mut w, &writes, install)?;
        let after = balances(&w)?;
        assert_eq!(
            after.first().ok_or("payer")?.0,
            before
                .first()
                .ok_or("payer")?
                .0
                .checked_sub(Amount::from_minor(100))
                .ok_or("fee")?
        );
        assert_eq!(
            after.get(1).ok_or("sink")?.0,
            before
                .get(1)
                .ok_or("sink")?
                .0
                .checked_add(Amount::from_minor(100))
                .ok_or("fee")?
        );
        assert!(
            serde_json::to_value(owner(&w, 1)?)? == serde_json::to_value(&birth)?,
            "birth term changed"
        );
        assert!(
            serde_json::to_value(owner(&w, 2)?)? == serde_json::to_value(&term2)?,
            "update extended owner term"
        );
        assert!(
            one(rows(&disk(&w), "roll", OPS))? == cause,
            "update replaced attestation cause"
        );
        assert!(
            one(rows(&disk(&w), "roll", "roll_mind_paid_term"))? == paid_value,
            "update changed paid Pin"
        );
        assert!(
            rows(&disk(&w), "roll", "roll_mind_filed").contains(&serde_json::to_value(&old_file)?),
            "update removed immutable original source history"
        );
        let update = one(rows(&disk(&w), "roll", "roll_mind_line_update"))?;
        assert_eq!(update.get("format"), Some(&json!(2)));
        assert_eq!(update.get("term"), Some(&json!("2")));
        let filed: FiledRecord = serde_json::from_value(
            rows(&disk(&w), "roll", "roll_mind_filed")
                .into_iter()
                .find(|v| v.get("operation") == Some(&json!(token(START + 1))))
                .ok_or("updated filed revision")?,
        )?;
        assert_eq!(filed.content().revision().revision(), revision.reference());
        assert_eq!(filed.rating(), &rating);
        owner_available(&mut w, true)?;
        let saved = Durable::capture(&w.engine, &disk(&w));
        let cold = boot(f, &saved)?;
        tasks.push(cold.tasks.clone());
        assert_eq!(current(&cold)?, 2);
        // Advance the current pointer again. The update must continue proving
        // its historical generation2 and actual cause, not bind to current3.
        w.engine.world.set_now(term2.window().term_end().get());
        attest(&mut w, START + 2)?;
        assert_eq!(current(&w)?, 3);
        assert!(one(rows(&disk(&w), "roll", "roll_mind_line_update"))? == update);
        assert!(
            rows(&disk(&w), "roll", "rating_receipts") == before_certificates,
            "attestation/update added a certificate instead of reusing measured evidence"
        );
        let saved = Durable::capture(&w.engine, &disk(&w));
        let mut cold = boot(f, &saved)?;
        tasks.push(cold.tasks.clone());
        assert_eq!(current(&cold)?, 3);
        owner_available(&mut cold, true)?;
        super::super::fork_tests::boxed_story_phase(
            "compatible_update.attested_corruption",
            || cold_refusals(f, &saved),
        )?
        .await?;
        Ok(())
    }
    .await;
    for task in tasks {
        task.shutdown(Duration::from_secs(5)).await;
    }
    result
}

/// Reuse the same actual second certificate, with a genuinely different owner
/// attesting the inherited licence before its original author reacquires title.
/// The later update must prove the attester's historical ownership independently
/// of the updater's different current ownership generation.
async fn chartered(
    f: &PurchaseFixture,
    certified: &Durable,
    revision: &Revision,
    rating: RatingRef,
) -> TestResult {
    use super::super::super::charter_transfer_fixture;
    const SALE: u128 = 48_100_000;
    let mut w = boot(f, certified)?;
    let mut tasks = vec![w.tasks.clone()];
    let result: TestResult = async {
        let birth = owner(&w, 1)?;
        let certificates = rows(&disk(&w), "roll", "rating_receipts");
        let paid = rows(&disk(&w), "roll", "roll_mind_paid_term");
        let initial = rows(&disk(&w), "roll", "roll_mind_initial_filing");
        let charter: mush_roll::commerce::CharterId = serde_json::from_value(
            initial
                .first()
                .and_then(|v| v.get("charter"))
                .cloned()
                .ok_or("actual charter")?,
        )?;
        w.engine.world.set_now(birth.window().grace_end().get());
        w.transfer_charter(charter.object(), BUYER, SALE)?;
        owner_available(&mut w, false)?;
        assert!(
            signed(
                &mut w,
                "attest_owner_mind",
                SALE + 2,
                json!({"owner_licence":1})
            )
            .is_err()
        );
        let before = balances(&w)?;
        let p = charter_transfer_fixture::signed(
            &mut w.engine,
            "attest_owner_mind",
            BUYER,
            SALE + 2,
            json!({"owner_licence":1}),
        )?;
        let (writes, install) = p.into_parts();
        captured::apply(&mut w, &writes, install)?;
        assert_eq!(balances(&w)?, before);
        assert_eq!(current(&w)?, 2);
        let term2 = owner(&w, 2)?;
        let cause = one(rows(&disk(&w), "roll", OPS))?;
        assert_eq!(cause.get("owner"), Some(&json!(BUYER)));
        assert_eq!(cause.get("format"), Some(&json!(2)));
        assert_eq!(
            cause.pointer("/charter_ownership/generation"),
            Some(&json!(2))
        );
        let events = w.say(BUYER, "SIM MIND /filed")?;
        assert!(PlayerWorld::private_text(&events, BUYER).contains("filed:1:"));
        assert!(serde_json::to_value(owner(&w, 1)?)? == serde_json::to_value(&birth)?);

        w.transfer_charter(charter.object(), GOD, SALE + 3)?;
        assert!(
            charter_transfer_fixture::signed(
                &mut w.engine,
                "attest_owner_mind",
                BUYER,
                SALE + 5,
                json!({"owner_licence":1}),
            )
            .is_err()
        );
        owner_available(&mut w, true)?;
        let p = signed(
            &mut w,
            "update_original_mind",
            SALE + 5,
            parameters(revision, rating),
        )?;
        let (writes, install) = p.into_parts();
        captured::apply(&mut w, &writes, install)?;
        let update = one(rows(&disk(&w), "roll", "roll_mind_line_update"))?;
        assert_eq!(update.get("format"), Some(&json!(3)));
        assert_eq!(update.get("term"), Some(&json!("2")));
        assert_eq!(
            update.pointer("/charter_ownership/generation"),
            Some(&json!(3))
        );
        assert!(one(rows(&disk(&w), "roll", OPS))? == cause);
        assert!(serde_json::to_value(owner(&w, 2)?)? == serde_json::to_value(&term2)?);
        assert!(rows(&disk(&w), "roll", "roll_mind_initial_filing") == initial);
        assert!(rows(&disk(&w), "roll", "roll_mind_paid_term") == paid);
        assert!(rows(&disk(&w), "roll", "rating_receipts") == certificates);
        let saved = Durable::capture(&w.engine, &disk(&w));
        let mut cold = boot(f, &saved)?;
        tasks.push(cold.tasks.clone());
        owner_available(&mut cold, true)?;
        cold.engine.world.set_now(term2.window().term_end().get());
        attest(&mut cold, SALE + 6)?;
        assert_eq!(current(&cold)?, 3);
        assert!(one(rows(&disk(&cold), "roll", "roll_mind_line_update"))? == update);
        let saved = Durable::capture(&cold.engine, &disk(&cold));
        let final_world = boot(f, &saved)?;
        tasks.push(final_world.tasks.clone());
        assert_eq!(current(&final_world)?, 3);
        for family in [OPS, "roll_mind_line_update"] {
            let mut crossed = saved.clone();
            let row = crossed
                .disk
                .values_mut()
                .find(|v| {
                    v.pointer("/family/id") == Some(&json!(family))
                        && (family != OPS
                            || v.pointer("/value/operation") == Some(&json!(token(SALE + 2))))
                })
                .ok_or("actual captured ownership row")?;
            *row.pointer_mut("/value/charter_ownership/generation")
                .ok_or("ownership generation")? = json!(4);
            if let Ok(world) = boot(f, &crossed) {
                world.tasks.shutdown(Duration::from_secs(5)).await;
                return Err("crossed historical charter ownership passed cold boot".into());
            }
        }
        Ok(())
    }
    .await;
    for task in tasks {
        task.shutdown(Duration::from_secs(5)).await;
    }
    result
}
