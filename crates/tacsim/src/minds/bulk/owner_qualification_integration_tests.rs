//! Exact dependent birth and actual additional paid grants. No synthetic rating,
//! grant or readiness is constructed; installation models the in-process ACK.
use super::*;
#[path = "owner_attestation_integration_tests.rs"]
mod attestation;
#[path = "owner_qualification_integration_tests/transferred_parent.rs"]
mod transferred_parent;
use mush_core::{OfferExecutionContext, OfferSubmission};
use mush_roll::commerce::records::OwnerTerm;
const START: u128 = 21_000_000;
const ACTION: &str = "replace_owner_parent_qualification";
const OPS: &str = "roll_mind_owner_qualification_operation";
const HEADS: &str = "roll_mind_owner_qualification_head";
fn boot(f: &PurchaseFixture, saved: &Durable) -> TestResult<PlayerWorld> {
    PlayerWorld::boot_owner_qualification(f, saved)
}
fn params(owner: LicenceId, parent: LicenceId) -> Value {
    json!({"owner_licence":owner.get(),"parent_licence":parent.get()})
}
fn prepare_q(
    w: &mut PlayerWorld,
    actor: ObjId,
    n: u128,
    p: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    super::prepare(w, "roll", ACTION, 1, actor, n, p)
}
fn perform(w: &mut PlayerWorld, n: u128, p: Value) -> TestResult {
    let (prepared, _) = prepare_q(w, BUYER, n, p)?;
    let (writes, install) = prepared.into_parts();
    apply(w, &writes, install)
}
fn signed(w: &mut PlayerWorld, n: u128, p: Value) -> TestResult<PreparedComposedTxn> {
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x61; 32])?,
        [],
    )?);
    let exact = action(w, "roll", ACTION, 1)?;
    let principal = PrincipalId::Object { id: BUYER };
    let query = w
        .engine
        .legal_action_query(principal.clone(), BUYER, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&exact)
        .ok_or("actual qualification offer")?;
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
    let (actual, request, _) = resolved.into_authoritative_parts();
    check_eq!(actual, exact);
    Ok(w.engine
        .prepare_registered_composed_resolved_authoritative_action(&exact, &request, token(n))?
        .0)
}
fn stage_q(
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
fn retained(w: &PlayerWorld) -> Vec<((String, String), Value)> {
    const FIXED: [&str; 8] = [
        "roll_mind_owner_term",
        "roll_mind_owner_parent",
        "roll_mind_initial_filing",
        "roll_mind_filed",
        "roll_mind_draft_parent",
        "roll_mind_paid_term",
        "roll_mind_paid_term_economics",
        "roll_mind_personal_paid_licence",
    ];
    disk(w)
        .into_iter()
        .filter(|(_, v)| {
            v.pointer("/family/id")
                .and_then(Value::as_str)
                .is_some_and(|family| FIXED.contains(&family))
        })
        .collect()
}
fn owner_term(w: &PlayerWorld, owner: LicenceId) -> TestResult<OwnerTerm> {
    Ok(serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_owner_term")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(owner)))
            .ok_or("actual child term")?,
    )?)
}
fn child_source(w: &PlayerWorld, owner: LicenceId) -> TestResult<mush_core::DefinitionDigest> {
    let birth: InitialFilingOperation = serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_initial_filing")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(owner)))
            .ok_or("actual dependent birth")?,
    )?;
    Ok(birth.revision().source())
}

fn parent_purchase(
    w: &mut PlayerWorld,
    n: u128,
    form: &str,
) -> TestResult<EntitlementAllocationV1> {
    for actor in [GOD, BUYER] {
        w.engine.world.move_to(actor, ROOM)?;
    }
    let faucet = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .snapshot()
        .accounts()
        .find(|a| matches!(&a.owner,EconomicOwner::System(s) if s.as_str()=="purchase.fixture"))
        .ok_or("faucet")?
        .id;
    w.say(GOD, &format!("@econ/mint {faucet}=2 1000"))?;
    let rate = if form == "open_resale" { 100 } else { 0 };
    commit(
        w,
        "market",
        "list_original_mind",
        3,
        GOD,
        n,
        listing_params(LicenceId::new(1)?, form, rate),
    )?;
    commit(
        w,
        "market",
        "quote_original_mind",
        3,
        BUYER,
        n + 1,
        json!({"listing":token(n)}),
    )?;
    commit(
        w,
        "market",
        "purchase_original_mind",
        3,
        BUYER,
        n + 2,
        json!({"quote":token(n+1)}),
    )?;
    commit(
        w,
        "market",
        "settle_original_mind",
        3,
        BUYER,
        n + 3,
        json!({"operation":token(n+2)}),
    )?;
    let actual = allocation(w, n + 2)?;
    check_eq!(actual.protocol(), EntitlementProtocol::OriginalV3);
    check_eq!(actual.holder(), mush_economy::Holder::Character(BUYER));
    Ok(actual)
}
fn corruptions(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    check!(
        crate::test_checks::refused(PlayerWorld::boot_entitlements(f, saved))?,
        "disabled qualification cannot drop its obligations"
    );
    for family in [OPS, HEADS] {
        let mut missing = saved.clone();
        check!(remove_family(&mut missing, "roll", family) > 0);
        check!(
            crate::test_checks::refused(boot(f, &missing))?,
            "missing qualification half"
        );
    }
    let mut all = saved.clone();
    check!(remove_family(&mut all, "roll", OPS) > 0);
    check!(remove_family(&mut all, "roll", HEADS) > 0);
    check!(
        crate::test_checks::refused(boot(f, &all))?,
        "retained cause prevents all typed-row loss"
    );
    let mut root = saved.clone();
    let before = root.disk.len();
    root.disk.retain(|(_, key), _| {
        !key.starts_with("reference:roll:commercial:v1/owner_qualification/")
    });
    check!(root.disk.len() < before);
    check!(
        crate::test_checks::refused(boot(f, &root))?,
        "exact permanent cause required"
    );
    for field in ["ordinal", "operation"] {
        let mut crossed = saved.clone();
        let head = crossed
            .disk
            .values_mut()
            .find(|v| v.pointer("/family/id") == Some(&json!(HEADS)))
            .ok_or("head")?;
        *head
            .get_mut("value")
            .and_then(|v| v.get_mut(field))
            .ok_or("head field")? = if field == "ordinal" {
            json!(99)
        } else {
            json!(token(START + 999))
        };
        check!(
            crate::test_checks::refused(boot(f, &crossed))?,
            "head cannot claim an unrelated history"
        );
    }
    // No recall producer is invented. Unbacked source status is a cold corruption.
    let mut recalled = saved.clone();
    let op = one(rows(&recalled.disk, "roll", OPS))?;
    let source = op
        .pointer("/source_parent/filed")
        .ok_or("source parent")?
        .clone();
    let status = recalled
        .disk
        .values_mut()
        .find(|v| {
            v.pointer("/family/id") == Some(&json!("roll_mind_status"))
                && v.pointer("/value/filed") == Some(&source)
        })
        .ok_or("source status")?;
    *status
        .get_mut("value")
        .and_then(|v| v.get_mut("recalled_by"))
        .ok_or("recall")? = json!(token(START + 998));
    check!(
        crate::test_checks::refused(boot(f, &recalled))?,
        "replacement cannot repair a forged recall cause"
    );
    Ok(())
}
fn fences(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    parent: &EntitlementAllocationV1,
) -> TestResult {
    let p = params(owner, parent.licence());
    let parent_window = {
        let w = boot(f, saved)?;
        own_term(&w, parent.licence())?.window()
    };
    for mode in ["foreign", "clock", "lapse_stage", "lapse_take", "title"] {
        let mut w = boot(f, saved)?;
        let before = disk(&w);
        let money = balances(&w)?;
        if mode == "foreign" {
            check!(prepare_q(&mut w, GOD, START + 30, p.clone()).is_err());
        } else {
            let (prepared, out) = prepare_q(&mut w, BUYER, START + 30, p.clone())?;
            if mode == "title" {
                // Core admits one pending commit: the charter's title cannot move
                // under a prepared qualification, so the transfer is refused and
                // the author keeps the title.
                let op = rows(&disk(&w), "roll", "roll_mind_initial_filing")
                    .into_iter()
                    .find(|v| v.get("licence") == Some(&json!(owner)))
                    .ok_or("birth")?;
                let charter: mush_roll::commerce::CharterId =
                    serde_json::from_value(op.get("charter").ok_or("charter")?.clone())?;
                let held = w
                    .engine
                    .world
                    .module_state::<EconomyStore>()
                    .ok_or("economy")?
                    .snapshot()
                    .asset(charter.object())
                    .ok_or("title")?
                    .title_owner
                    .clone();
                check!(
                    w.transfer_charter(charter.object(), GOD, 49_000_110)
                        .is_err(),
                    "title moved under a prepared qualification"
                );
                check_eq!(
                    w.engine
                        .world
                        .module_state::<EconomyStore>()
                        .ok_or("economy")?
                        .snapshot()
                        .asset(charter.object())
                        .ok_or("title")?
                        .title_owner,
                    held
                );
                drop((prepared, out));
            } else if mode == "lapse_take" {
                stage_q(&mut w, START + 30, p.clone(), prepared, out)?;
                w.engine.world.set_now(parent_window.grace_end().get());
                check!(w.engine.take_staged_action().is_err());
            } else {
                match mode {
                    "clock" => {
                        let epoch = w.engine.world.next_world_timeline_epoch(2)?;
                        w.engine.world.append_world_timeline_epoch(epoch)?;
                    }
                    _ => w.engine.world.set_now(parent_window.grace_end().get()),
                }
                check!(stage_q(&mut w, START + 30, p.clone(), prepared, out).is_err());
            }
        }
        check_eq!(balances(&w)?, money);
        check!(rows(&disk(&w), "roll", OPS).is_empty());
        check!(
            disk(&w) == before,
            "refusal changed retained business images"
        );
    }
    // Ordinary frozen custody completes after the actual parent lapses.
    let mut held = boot(f, saved)?;
    let (prepared, out) = prepare_q(&mut held, BUYER, START + 31, p.clone())?;
    stage_q(&mut held, START + 31, p, prepared, out)?;
    let (bundle, installation) = held
        .engine
        .take_staged_action()?
        .ok_or("actual qualification custody")?
        .into_parts();
    held.engine.world.set_now(parent_window.grace_end().get());
    apply(&mut held, bundle.writes(), installation)?;
    check_eq!(rows(&disk(&held), "roll", OPS).len(), 1);
    check!(boot(f, &capture(&held, &disk(&held))).is_ok());
    Ok(())
}
// A shorter *actual configured offer*, not altered persisted dates. The old
// birth/history stays valid under the later normal policy composition.
async fn short_parent_lapse_and_restore(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    customer: &EntitlementAllocationV1,
) -> TestResult {
    let mut w = PlayerWorld::boot_owner_qualification_short_parent(f, saved)?;
    let tasks = w.tasks.clone();
    let result: TestResult = async {
        let child = owner_term(&w, owner)?;
        let source = child_source(&w, owner)?;
        let short = parent_purchase(&mut w, START + 200, "open_resale")?;
        let short_window = own_term(&w, short.licence())?.window();
        assert_eq!(
            short_window.term_end().get() - short_window.issued_at().get(),
            60
        );
        assert_eq!(short_window.term_end(), short_window.grace_end());
        assert!(short_window.grace_end() < child.window().term_end());
        let immutable = retained(&w);
        perform(&mut w, START + 210, params(owner, short.licence()))?;
        assert!(retained(&w) == immutable);
        source_answer(&mut w, BUYER, owner, Some(source))?;
        list(&mut w, owner, START + 211)?;
        w.engine.world.set_now(short_window.grace_end().get());
        source_answer(&mut w, BUYER, owner, None)?;
        assert!(
            super::prepare(
                &mut w,
                "market",
                "list_derived_mind",
                1,
                BUYER,
                START + 212,
                listing_params(owner, "open_resale", 100),
            )
            .is_err(),
            "expired selected qualification cannot issue new rights"
        );
        source_answer(
            &mut w,
            GOD,
            customer.licence(),
            Some(customer.revision().source()),
        )?;
        let lapsed = capture(&w, &disk(&w));
        let mut restored = boot(f, &lapsed)?;
        source_answer(&mut restored, BUYER, owner, None)?;
        let replacement = parent_purchase(&mut restored, START + 220, "open_resale")?;
        let before = retained(&restored);
        perform(
            &mut restored,
            START + 230,
            params(owner, replacement.licence()),
        )?;
        assert!(retained(&restored) == before);
        source_answer(&mut restored, BUYER, owner, Some(source))?;
        list(&mut restored, owner, START + 231)?;
        assert_eq!(
            serde_json::to_value(owner_term(&restored, owner)?)?,
            serde_json::to_value(child)?
        );
        assert_eq!(rows(&disk(&restored), "roll", OPS).len(), 2);
        let cold = capture(&restored, &disk(&restored));
        let mut cold = boot(f, &cold)?;
        source_answer(&mut cold, BUYER, owner, Some(source))?;
        source_answer(
            &mut cold,
            GOD,
            customer.licence(),
            Some(customer.revision().source()),
        )?;
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        restored.tasks.shutdown(Duration::from_secs(5)).await;
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
pub(super) async fn exercise(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    customer: &EntitlementAllocationV1,
) -> TestResult {
    super::story_phase("qualification.transferred_parent", || {
        transferred_parent::exercise(f, saved, owner, customer)
    })?
    .await?;
    super::story_phase("qualification.chartered", || {
        chartered(f, saved, owner, customer)
    })?
    .await?;
    super::story_phase("qualification.short_parent", || {
        short_parent_lapse_and_restore(f, saved, owner, customer)
    })?
    .await?;
    let mut w = boot(f, saved)?;
    let tasks = w.tasks.clone();
    let result: TestResult = async {
        let child = owner_term(&w, owner)?;
        let source = child_source(&w, owner)?;
        source_answer(&mut w, BUYER, owner, Some(source))?;
        let open = parent_purchase(&mut w, START, "open")?;
        let unqualified = capture(&w, &disk(&w));
        fences(f, &unqualified, owner, &open)?;
        let before = retained(&w);
        let money = balances(&w)?;
        let prepared = signed(&mut w, START + 10, params(owner, open.licence()))?;
        assert_eq!(balances(&w)?, money);
        assert!(rows(&disk(&w), "roll", OPS).is_empty());
        let (writes, install) = prepared.into_parts();
        apply(&mut w, &writes, install)?;
        assert!(
            retained(&w) == before,
            "qualification rewrote immutable birth/source/child/paid history"
        );
        assert_eq!(balances(&w)?, money);
        source_answer(&mut w, BUYER, owner, Some(source))?;
        assert!(
            super::prepare(
                &mut w,
                "market",
                "list_derived_mind",
                1,
                BUYER,
                START + 11,
                listing_params(owner, "open_resale", 100)
            )
            .is_err(),
            "Open does not restore new resale rights"
        );
        source_answer(
            &mut w,
            GOD,
            customer.licence(),
            Some(customer.revision().source()),
        )?;
        let once = capture(&w, &disk(&w));
        corruptions(f, &once)?;
        let mut cold = boot(f, &once)?;
        source_answer(&mut cold, BUYER, owner, Some(source))?;
        let resale = parent_purchase(&mut cold, START + 40, "open_resale")?;
        let before = retained(&cold);
        perform(&mut cold, START + 50, params(owner, resale.licence()))?;
        assert!(retained(&cold) == before);
        assert_eq!(rows(&disk(&cold), "roll", OPS).len(), 2);
        assert_eq!(rows(&disk(&cold), "roll", HEADS).len(), 1);
        let head = one(rows(&disk(&cold), "roll", HEADS))?;
        assert_eq!(head.get("ordinal"), Some(&json!(2)));
        assert_eq!(head.get("operation"), Some(&json!(token(START + 50))));
        list(&mut cold, owner, START + 51)?;
        source_answer(&mut cold, BUYER, owner, Some(source))?;
        assert_eq!(
            serde_json::to_value(owner_term(&cold, owner)?)?,
            serde_json::to_value(&child)?
        );
        assert!(
            own_term(&cold, resale.licence())?.window().grace_end() > child.window().grace_end(),
            "actual longer replacement window"
        );
        let twice = capture(&cold, &disk(&cold));
        let mut expired = twice.clone();
        expired.now = child.window().grace_end().get();
        let mut expired = boot(f, &expired)?;
        source_answer(&mut expired, BUYER, owner, None)?;
        source_answer(
            &mut expired,
            GOD,
            customer.licence(),
            Some(customer.revision().source()),
        )?;
        assert!(
            super::prepare(
                &mut expired,
                "market",
                "list_derived_mind",
                1,
                BUYER,
                START + 52,
                listing_params(owner, "open_resale", 100)
            )
            .is_err(),
            "longer parent does not extend child dates"
        );
        // Source-free metadata replacement is allowed while the child is lapsed;
        // it still cannot create source/use permission or alter that child term.
        perform(&mut expired, START + 53, params(owner, resale.licence()))?;
        source_answer(&mut expired, BUYER, owner, None)?;
        assert_eq!(
            serde_json::to_value(owner_term(&expired, owner)?)?,
            serde_json::to_value(&child)?
        );
        assert!(boot(f, &capture(&expired, &disk(&expired))).is_ok());
        let restored_lapsed = capture(&expired, &disk(&expired));
        super::story_phase("qualification.attestation", || {
            attestation::exercise(f, &restored_lapsed, owner, customer)
        })?
        .await?;

        expired.tasks.shutdown(Duration::from_secs(5)).await;
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}

async fn chartered(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    customer: &EntitlementAllocationV1,
) -> TestResult {
    use crate::minds::bulk::registry_integration_tests::original_purchase::charter_transfer_fixture;
    const SALE: u128 = 48_200_000;
    let mut w = boot(f, saved)?;
    let mut tasks = vec![w.tasks.clone()];
    let result: TestResult = async {
        let child = owner_term(&w, owner)?;
        let source = child_source(&w, owner)?;
        let birth = rows(&disk(&w), "roll", "roll_mind_initial_filing")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(owner)))
            .ok_or("actual child birth")?;
        let child_charter: mush_roll::commerce::CharterId = serde_json::from_value(
            birth
                .get("charter")
                .cloned()
                .ok_or("actual child charter")?,
        )?;
        let parent_charter: mush_roll::commerce::CharterId = serde_json::from_value(
            birth
                .pointer("/parent/charter")
                .cloned()
                .ok_or("actual parent charter")?,
        )?;
        let old_parent: LicenceId = serde_json::from_value(
            birth
                .pointer("/parent/licence")
                .cloned()
                .ok_or("actual parent's acquired grant")?,
        )?;
        // The new child owner acquires a real same-line parent grant from the
        // new parent title holder. Neither title sale itself grants source use.
        w.transfer_charter(parent_charter.object(), BUYER, SALE)?;
        for actor in [GOD, BUYER] {
            w.engine.world.move_to(actor, ROOM)?;
        }
        commit(
            &mut w,
            "market",
            "list_original_mind",
            3,
            BUYER,
            SALE + 2,
            listing_params(LicenceId::new(1)?, "open", 0),
        )?;
        commit(
            &mut w,
            "market",
            "quote_original_mind",
            3,
            GOD,
            SALE + 3,
            json!({"listing":token(SALE + 2)}),
        )?;
        commit(
            &mut w,
            "market",
            "purchase_original_mind",
            3,
            GOD,
            SALE + 4,
            json!({"quote":token(SALE + 3)}),
        )?;
        commit(
            &mut w,
            "market",
            "settle_original_mind",
            3,
            GOD,
            SALE + 5,
            json!({"operation":token(SALE + 4)}),
        )?;
        let parent = allocation(&w, SALE + 4)?;
        assert_eq!(parent.holder(), mush_economy::Holder::Character(GOD));
        let immutable = retained(&w);
        w.transfer_charter(child_charter.object(), GOD, SALE + 6)?;
        source_answer(&mut w, BUYER, owner, None)?;
        source_answer(&mut w, GOD, owner, None)?;
        assert!(prepare_q(&mut w, GOD, SALE + 8, params(owner, old_parent)).is_err());
        assert!(prepare_q(&mut w, GOD, SALE + 8, params(owner, customer.licence())).is_err());
        let before = balances(&w)?;
        let p = charter_transfer_fixture::signed(
            &mut w.engine,
            ACTION,
            GOD,
            SALE + 8,
            params(owner, parent.licence()),
        )?;
        let (writes, install) = p.into_parts();
        apply(&mut w, &writes, install)?;
        assert_eq!(balances(&w)?, before);
        source_answer(&mut w, GOD, owner, Some(source))?;
        let proof = rows(&disk(&w), "roll", OPS)
            .into_iter()
            .find(|v| v.get("operation") == Some(&json!(token(SALE + 8))))
            .ok_or("actual new owner's qualification")?;
        assert_eq!(proof.get("format"), Some(&json!(2)));
        assert_eq!(proof.get("owner"), Some(&json!(GOD)));
        assert_eq!(
            proof.pointer("/charter_ownership/generation"),
            Some(&json!(2))
        );
        assert!(
            super::prepare(
                &mut w,
                "market",
                "list_derived_mind",
                1,
                GOD,
                SALE + 9,
                listing_params(owner, "open_resale", 100)
            )
            .is_err()
        );
        assert!(
            retained(&w) == immutable,
            "qualification rewrote immutable acquisition or term"
        );
        source_answer(
            &mut w,
            GOD,
            customer.licence(),
            Some(customer.revision().source()),
        )?;
        let saved = capture(&w, &disk(&w));
        let mut cold = boot(f, &saved)?;
        tasks.push(cold.tasks.clone());
        source_answer(&mut cold, GOD, owner, Some(source))?;
        cold.transfer_charter(child_charter.object(), BUYER, SALE + 10)?;
        // Reacquisition does not make the former owner's old qualification
        // current again: the new ownership generation needs its own selection.
        source_answer(&mut cold, BUYER, owner, None)?;
        source_answer(&mut cold, GOD, owner, None)?;
        let p = signed(&mut cold, SALE + 12, params(owner, old_parent))?;
        let (writes, install) = p.into_parts();
        apply(&mut cold, &writes, install)?;
        source_answer(&mut cold, BUYER, owner, Some(source))?;
        assert!(rows(&disk(&cold), "roll", OPS).contains(&proof));
        assert!(serde_json::to_value(owner_term(&cold, owner)?)? == serde_json::to_value(&child)?);
        let saved = capture(&cold, &disk(&cold));
        let mut final_world = boot(f, &saved)?;
        tasks.push(final_world.tasks.clone());
        source_answer(&mut final_world, BUYER, owner, Some(source))?;
        source_answer(
            &mut final_world,
            GOD,
            customer.licence(),
            Some(customer.revision().source()),
        )?;
        Ok(())
    }
    .await;
    for task in tasks {
        task.shutdown(Duration::from_secs(5)).await;
    }
    result
}
