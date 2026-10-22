//! Actual resale-owned parent selection and later immutable attestation history.
//! Reuses the parent's genuine child and certificates; no additional worker run.
use super::*;
use mush_economy::{AccountId as WalletId, AccountStatus};
const N: u128 = 82_000_000;

fn boot_all(f: &PurchaseFixture, saved: &Durable) -> TestResult<PlayerWorld> {
    PlayerWorld::boot_options_with_resales(
        f, saved, true, true, true, true, true, false, false, false, true, false, true, None, None,
        false, true,
    )
}
fn wallet(w: &PlayerWorld, actor: ObjId) -> TestResult<WalletId> {
    let economy = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?;
    let snapshot = economy.snapshot();
    let mut rows = snapshot.accounts().filter(|a| {
        a.owner == EconomicOwner::Object(actor)
            && a.scope.is_none()
            && a.instrument.as_str() == "cr"
            && a.status == AccountStatus::Active
    });
    let id = rows.next().ok_or("actual wallet")?.id;
    assert!(rows.next().is_none(), "actual wallet is unique");
    Ok(id)
}
fn signed_action(
    w: &mut PlayerWorld,
    owner: &str,
    id: &str,
    actor: ObjId,
    n: u128,
    p: Value,
) -> TestResult {
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x43; 32])?,
        [],
    )?);
    let exact = action(w, owner, id, 1)?;
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&exact)
        .ok_or("actual signed action")?;
    let inv = invocation(n, actor, owner, p.clone())?;
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
    let (prepared, _) = w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(&exact, &request, token(n))?;
    let (writes, install) = prepared.into_parts();
    apply(w, &writes, install)
}
fn resale(
    w: &mut PlayerWorld,
    licence: LicenceId,
    seller: ObjId,
    buyer: ObjId,
    n: u128,
) -> TestResult {
    for actor in [seller, buyer] {
        w.engine.world.move_to(actor, ROOM)?;
    }
    w.commit()?;
    let payout = wallet(w, seller)?;
    let payer = wallet(w, buyer)?;
    signed_action(
        w,
        "market",
        "offer_paid_mind_resale",
        seller,
        n,
        json!({"licence":licence.get(),"buyer":buyer.0,"price":1,"seller_account":payout.0}),
    )?;
    signed_action(
        w,
        "market",
        "accept_paid_mind_resale",
        buyer,
        n + 1,
        json!({"offer":token(n),"payer":payer.0}),
    )
}
fn cause(w: &PlayerWorld, family: &str, n: u128) -> TestResult<Value> {
    rows(&disk(w), "roll", family)
        .into_iter()
        .find(|v| v.get("operation") == Some(&json!(token(n))))
        .ok_or_else(|| "actual operation".into())
}
fn immutable(w: &PlayerWorld) -> Vec<((String, String), Value)> {
    disk(w)
        .into_iter()
        .filter(|(_, v)| {
            matches!(
                v.pointer("/family/id").and_then(Value::as_str),
                Some(
                    "roll_mind_initial_filing"
                        | "roll_mind_filed"
                        | "roll_mind_owner_parent"
                        | "roll_mind_draft_parent"
                        | "roll_mind_paid_term"
                        | "roll_mind_paid_term_economics"
                )
            )
        })
        .collect()
}
pub(super) async fn exercise(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    customer: &EntitlementAllocationV1,
) -> TestResult {
    let mut w = boot_all(f, saved)?;
    let mut tasks = vec![w.tasks.clone()];
    let result: TestResult = async {
        let child = owner_term(&w, owner)?;
        let source = child_source(&w, owner)?;
        let birth = rows(&disk(&w), "roll", "roll_mind_initial_filing")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(owner)))
            .ok_or("child birth")?;
        let parent_charter: mush_roll::commerce::CharterId = serde_json::from_value(
            birth
                .pointer("/parent/charter")
                .cloned()
                .ok_or("parent charter")?,
        )?;
        // Actual parent title goes to BUYER, who sells a fresh Open grant to GOD.
        // Its later paid resale to BUYER must not relabel the immutable term holder.
        w.transfer_charter(parent_charter.object(), BUYER, N)?;
        w.engine
            .world
            .set_now(w.engine.world.now().checked_add(60).ok_or("UTC")?);
        for actor in [GOD, BUYER] {
            w.engine.world.move_to(actor, ROOM)?;
        }
        w.commit()?;
        commit(
            &mut w,
            "market",
            "list_original_mind",
            3,
            BUYER,
            N + 2,
            listing_params(LicenceId::new(1)?, "open", 0),
        )?;
        commit(
            &mut w,
            "market",
            "quote_original_mind",
            3,
            GOD,
            N + 3,
            json!({"listing":token(N+2)}),
        )?;
        commit(
            &mut w,
            "market",
            "purchase_original_mind",
            3,
            GOD,
            N + 4,
            json!({"quote":token(N+3)}),
        )?;
        commit(
            &mut w,
            "market",
            "settle_original_mind",
            3,
            GOD,
            N + 5,
            json!({"operation":token(N+4)}),
        )?;
        let parent = allocation(&w, N + 4)?;
        assert_eq!(parent.holder(), GOD);
        let frozen = immutable(&w);
        resale(&mut w, parent.licence(), GOD, BUYER, N + 6)?;
        let acquired = capture(&w, &disk(&w));
        source_fork_history(f, &acquired, parent.licence()).await?;
        let paid = rows(&disk(&w), "roll", "roll_mind_personal_paid_licence")
            .into_iter()
            .find(|v| v.get("id") == Some(&json!(parent.licence())))
            .ok_or("paid pointer")?;
        assert_eq!(paid.get("holder"), Some(&json!(BUYER)));
        assert_eq!(paid.pointer("/ownership/generation"), Some(&json!(2)));
        signed_action(
            &mut w,
            "roll",
            ACTION,
            BUYER,
            N + 8,
            params(owner, parent.licence()),
        )?;
        let qualification = cause(&w, OPS, N + 8)?;
        let basis = qualification
            .pointer("/selected_parent/holder_basis")
            .cloned()
            .ok_or("captured parent holder")?;
        assert_eq!(basis.get("format"), Some(&json!(1)));
        assert_eq!(basis.get("acceptance"), Some(&json!(token(N + 7))));
        assert_eq!(basis.get("ownership"), paid.get("ownership"));
        assert_eq!(
            immutable(&w),
            frozen,
            "qualification altered immutable source/term history"
        );
        source_answer(&mut w, BUYER, owner, Some(source))?;
        let qualified = capture(&w, &disk(&w));
        let mut cold = boot_all(f, &qualified)?;
        tasks.push(cold.tasks.clone());
        source_answer(&mut cold, BUYER, owner, Some(source))?;
        // The old child dates remain unchanged; only a due actual attestation
        // may use the separately acquired parent's slightly later window.
        assert_eq!(
            serde_json::to_value(owner_term(&cold, owner)?)?,
            serde_json::to_value(&child)?
        );
        let at = child.window().term_end().get();
        let stamp = cold.engine.world.world_ic_now();
        assert_eq!(
            stamp.at,
            cold.engine.world.now(),
            "fixture uses the actual ratio-one clock"
        );
        cold.engine.world.set_now(at);
        cold.commit()?;
        signed_action(
            &mut cold,
            "roll",
            "attest_owner_mind",
            BUYER,
            N + 10,
            json!({"owner_licence":owner.get()}),
        )?;
        let attested = cause(&cold, "roll_mind_owner_attestation_operation", N + 10)?;
        assert_eq!(
            attested.pointer("/next/attestation/parent/selected/holder_basis"),
            Some(&basis)
        );
        assert_eq!(attested.pointer("/next/generation"), Some(&json!("2")));
        let new_grace = attested
            .pointer("/next/window/grace_end")
            .and_then(Value::as_i64)
            .ok_or("new grace")?;
        assert!(new_grace > child.window().grace_end().get());
        let attested_saved = capture(&cold, &disk(&cold));
        let mut later = boot_all(f, &attested_saved)?;
        tasks.push(later.tasks.clone());
        source_answer(&mut later, BUYER, owner, Some(source))?;
        resale(&mut later, parent.licence(), BUYER, GOD, N + 12)?;
        source_answer(&mut later, BUYER, owner, None)?;
        let final_saved = capture(&later, &disk(&later));
        let mut final_world = boot_all(f, &final_saved)?;
        tasks.push(final_world.tasks.clone());
        assert!(rows(&disk(&final_world), "roll", OPS).contains(&qualification));
        assert!(
            rows(
                &disk(&final_world),
                "roll",
                "roll_mind_owner_attestation_operation"
            )
            .contains(&attested)
        );
        source_answer(&mut final_world, BUYER, owner, None)?;
        source_answer(
            &mut final_world,
            GOD,
            customer.licence(),
            Some(customer.revision().source()),
        )?;
        // Both actual native custody fences retain the exact current parent.
        for take in [false, true] {
            let mut probe = boot_all(f, &acquired)?;
            tasks.push(probe.tasks.clone());
            let (prepared, out) =
                prepare_q(&mut probe, BUYER, N + 20, params(owner, parent.licence()))?;
            // Core admits one pending commit: the parent cannot be resold
            // between a prepared (or staged) qualification and its install,
            // so the resale itself is refused and no qualification lands.
            if take {
                stage_q(
                    &mut probe,
                    N + 20,
                    params(owner, parent.licence()),
                    prepared,
                    out,
                )?;
                assert!(
                    resale(&mut probe, parent.licence(), BUYER, GOD, N + 21).is_err(),
                    "parent resold under a staged qualification"
                );
            } else {
                assert!(
                    resale(&mut probe, parent.licence(), BUYER, GOD, N + 21).is_err(),
                    "parent resold under a prepared qualification"
                );
                drop((prepared, out));
            }
            assert!(rows(&disk(&probe), "roll", OPS).is_empty());
        }
        for mode in ["holder", "capture", "crossed"] {
            let mut bad = attested_saved.clone();
            let key = bad
                .disk
                .iter()
                .find(|(_, v)| {
                    v.pointer("/family/id").and_then(Value::as_str)
                        == Some(if mode == "holder" {
                            "roll_mind_paid_holders"
                        } else {
                            OPS
                        })
                })
                .map(|(k, _)| k.clone())
                .ok_or("actual corruptible row")?;
            if mode == "holder" {
                bad.disk.remove(&key).ok_or("holder")?;
            } else {
                let row = bad.disk.get_mut(&key).ok_or("row")?;
                if mode == "capture" {
                    row.pointer_mut("/value/selected_parent")
                        .and_then(Value::as_object_mut)
                        .ok_or("parent")?
                        .remove("holder_basis");
                } else {
                    *row.pointer_mut("/value/selected_parent/holder_basis/acceptance")
                        .ok_or("basis")? = json!(token(N + 4));
                }
            }
            assert!(
                boot_all(f, &bad).is_err(),
                "missing/crossed captured holder must refuse cold reconstruction"
            );
        }
        Ok(())
    }
    .await;
    for task in tasks {
        task.shutdown(Duration::from_secs(5)).await;
    }
    result
}

/// A real transferred Open parent can be copied while held. Losing it later
/// removes fresh authority without invalidating the accepted source history.
async fn source_fork_history(
    f: &PurchaseFixture,
    saved: &Durable,
    parent: LicenceId,
) -> TestResult {
    let mut w = boot_all(f, saved)?;
    let mut tasks = vec![w.tasks.clone()];
    let result: TestResult = async {
        signed_action(
            &mut w,
            "roll",
            "fork_paid_mind",
            BUYER,
            N + 100,
            json!({"parent_licence":parent,"name":"resold-parent-acquisition"}),
        )?;
        let selector = rows(&disk(&w), "roll", "roll_mind_draft_parent")
            .into_iter()
            .find(|v| v.get("created") == Some(&json!(token(N + 100))))
            .ok_or("actual source selector")?;
        assert_eq!(selector.get("format"), Some(&json!(2)));
        assert_eq!(
            selector.pointer("/holder_capture/basis/acceptance"),
            Some(&json!(token(N + 7)))
        );
        assert_eq!(
            selector.pointer("/holder_capture/basis/ownership/generation"),
            Some(&json!(2))
        );
        let forked = capture(&w, &disk(&w));
        let mut cold = boot_all(f, &forked)?;
        tasks.push(cold.tasks.clone());
        let before = cold
            .engine
            .world
            .module_state::<RollStore>()
            .ok_or("Roll")?
            .mind(BUYER, "resold-parent-acquisition")?
            .clone();
        resale(&mut cold, parent, BUYER, GOD, N + 102)?;
        let later = capture(&cold, &disk(&cold));
        let mut restored = boot_all(f, &later)?;
        tasks.push(restored.tasks.clone());
        assert_eq!(
            restored
                .engine
                .world
                .module_state::<RollStore>()
                .ok_or("Roll")?
                .mind(BUYER, "resold-parent-acquisition")?,
            &before
        );
        assert!(rows(&disk(&restored), "roll", "roll_mind_draft_parent").contains(&selector));
        assert!(
            signed_action(
                &mut restored,
                "roll",
                "fork_paid_mind",
                BUYER,
                N + 104,
                json!({"parent_licence":parent,"name":"lost-parent-acquisition"})
            )
            .is_err()
        );
        for mode in ["missing", "crossed", "earlier"] {
            let mut bad = forked.clone();
            let row = bad
                .disk
                .values_mut()
                .find(|v| {
                    v.pointer("/family/id").and_then(Value::as_str)
                        == Some("roll_mind_draft_parent")
                        && v.pointer("/value/created") == Some(&json!(token(N + 100)))
                })
                .ok_or("actual source row")?;
            match mode {
                "missing" => {
                    row.pointer_mut("/value")
                        .and_then(Value::as_object_mut)
                        .ok_or("selector")?
                        .remove("holder_capture");
                }
                "crossed" => {
                    *row.pointer_mut("/value/holder_capture/basis/acceptance")
                        .ok_or("basis")? = json!(token(N + 4))
                }
                _ => {
                    *row.pointer_mut("/value/holder_capture/observed_ic/at")
                        .ok_or("clock")? = json!(i64::MIN)
                }
            }
            assert!(
                boot_all(f, &bad).is_err(),
                "source capture must retain exact accepted ownership"
            );
        }
        Ok(())
    }
    .await;
    for task in tasks {
        task.shutdown(Duration::from_secs(5)).await;
    }
    result
}
