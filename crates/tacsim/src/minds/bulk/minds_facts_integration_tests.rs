//! Real filed history and committed term-lapse facts over the parent's genuine
//! certificate and paid Grant. Runtime dispositions below model a trusted
//! consumer, not production server NoIntent evidence.
use super::*;
use mush_core::submission_observation::{
    SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
};
use mush_core::{AuthoritativeSubmit, OfferExecutionContext, OfferSubmission};
use mush_economy::{
    ContractId, ContractState, EconomicContract, ExternalRef, FactProvider, WorldFacts,
};
use std::sync::atomic::{AtomicBool, Ordering};
const START: u128 = 27_000_000;
const ACTION: &str = "record_mind_term_lapse";
const OCCURRENCES: &str = "roll_mind_lapse_occurrences";
struct Proof(Arc<AtomicBool>);
impl SubmissionDispositionReader for Proof {
    fn no_intent(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
struct Runtime;
impl SubmissionRuntime for Runtime {
    fn observe(&self, _: &AuthoritativeSubmit) -> SubmissionRuntimeState {
        let proof = Arc::new(AtomicBool::new(false));
        SubmissionRuntimeState::new(Arc::new(Proof(proof.clone())), Box::new(proof))
    }
}
fn branch(f: &PurchaseFixture, saved: &Durable) -> TestResult<PlayerWorld> {
    let mut w = PlayerWorld::boot_minds_facts(f, saved)?;
    w.engine.set_submission_runtime(Arc::new(Runtime))?;
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x82; 32])?,
        [],
    )?);
    Ok(w)
}
fn fire(w: &mut PlayerWorld) -> TestResult {
    let at = w.engine.world.now().checked_mul(1000).ok_or("tick clock")?;
    for _ in 0..4 {
        w.engine
            .fire_due(at, &mut Outbox::new(), &mut mush_core::Effects::new());
    }
    w.commit()?;
    Ok(())
}
fn later(w: &mut PlayerWorld) -> TestResult {
    w.engine
        .world
        .set_now(w.engine.world.now().checked_add(1).ok_or("clock")?);
    fire(w)
}
fn selected(
    w: &mut PlayerWorld,
) -> TestResult<Option<(Box<AuthoritativeSubmit>, Arc<AtomicBool>)>> {
    let Some(driven) = w.engine.take_observed_authoritative_submit() else {
        return Ok(None);
    };
    assert!(driven.is_observed());
    let (request, proof) = driven.into_parts();
    assert_eq!(request.actor, GOD);
    assert_eq!(request.action.key().owner.as_str(), "roll");
    assert_eq!(request.action.key().id.as_str(), ACTION);
    assert_eq!(request.subject, PerceivedRef::SelfActor);
    assert!(request.targets.is_empty());
    assert_eq!(
        request.parameters,
        json!({}),
        "private fixed offer has no public parameters"
    );
    let proof = *proof
        .ok_or("runtime proof")?
        .downcast::<Arc<AtomicBool>>()
        .map_err(|_| "proof type")?;
    Ok(Some((request, proof)))
}
fn next(w: &mut PlayerWorld) -> TestResult<(Box<AuthoritativeSubmit>, Arc<AtomicBool>)> {
    for _ in 0..32 {
        if let Some(value) = selected(w)? {
            return Ok(value);
        }
        later(w)?;
    }
    Err("bounded real lapse selector absent".into())
}
fn resolved(
    w: &PlayerWorld,
    request: &AuthoritativeSubmit,
    number: u128,
) -> TestResult<mush_core::ResolvedAuthoritativeOffer> {
    let principal = PrincipalId::Object { id: request.actor };
    let query =
        w.engine
            .legal_action_query(principal.clone(), request.actor, request.subject.clone())?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&request.action)
        .ok_or("actual private lapse offer")?;
    Ok(w.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: RequestUid::from_bytes(number.to_be_bytes()),
            principal: principal.clone(),
            actor: request.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: request.action.key().owner.clone(),
                    namespace: CanonicalId::new(ACTION)?,
                },
                1,
                IdempotencyKey::Caller(request.key.clone()),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: offer.offer(),
            subject: request.subject.clone(),
            targets: request.targets.clone(),
            parameters: request.parameters.clone(),
        },
    )?)
}
fn land(w: &mut PlayerWorld, request: &AuthoritativeSubmit, number: u128) -> TestResult<Value> {
    let (action, resolved, _) = resolved(w, request, number)?.into_authoritative_parts();
    let actual = resolved.invocation().parameters().clone();
    let before = disk(w);
    let money = balances(w)?;
    let (prepared, _) = w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            token(number),
        )?;
    assert!(disk(w) == before, "preparation published no fact");
    assert_eq!(balances(w)?, money, "lapse publication has no money effect");
    let (writes, install) = prepared.into_parts();
    apply(w, &writes, install)?;
    for (key, value) in &before {
        assert!(
            disk(w).get(key) == Some(value),
            "lapse rewrote existing evidence"
        );
    }
    assert_eq!(balances(w)?, money);
    Ok(actual)
}
fn occurrence(w: &PlayerWorld, id: LicenceId, generation: u64) -> Option<Value> {
    rows(&disk(w), "roll", OCCURRENCES).into_iter().find(|v| {
        v.pointer("/term/licence") == Some(&json!(id))
            && v.pointer("/term/generation") == Some(&json!(generation.to_string()))
    })
}
fn drain_until(
    w: &mut PlayerWorld,
    id: LicenceId,
    generation: u64,
    number: u128,
) -> TestResult<Value> {
    for n in 0..64 {
        if let Some(row) = occurrence(w, id, generation) {
            return Ok(row);
        }
        let (request, _) = next(w)?;
        land(w, &request, number + n)?;
        later(w)?;
    }
    Err("bounded occurrence drain".into())
}
fn current_term(w: &PlayerWorld, id: LicenceId) -> TestResult<PaidTermRecord> {
    let licence: PersonalPaidLicence = serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_personal_paid_licence")
            .into_iter()
            .find(|v| v.get("id") == Some(&json!(id)))
            .ok_or("actual current licence")?,
    )?;
    Ok(serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|v| {
                v.get("licence") == Some(&json!(id))
                    && v.get("generation") == Some(&json!(licence.term()))
            })
            .ok_or("actual current term")?,
    )?)
}
fn contract(w: &PlayerWorld, id: ContractId) -> TestResult<EconomicContract> {
    w.engine
        .world
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().contract(id))
        .cloned()
        .ok_or_else(|| "actual contract".into())
}
fn draft(w: &mut PlayerWorld, actor: ObjId, name: &str) -> TestResult<ContractId> {
    w.say(actor, &format!("@contract/draft {name}"))?;
    w.engine
        .world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or("Economy")?
        .snapshot()
        .contracts()
        .map(|c| c.id)
        .max()
        .ok_or_else(|| "actual draft".into())
}
fn acct(w: &PlayerWorld, actor: ObjId) -> TestResult<mush_economy::AccountId> {
    let snapshot = w
        .engine
        .world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or("Economy")?
        .snapshot();
    let mut accounts = snapshot
        .accounts()
        .filter(|a| a.owner == mush_org::PrincipalId::Object(actor));
    let id = accounts.next().ok_or("actual personal account")?.id;
    assert!(
        accounts.next().is_none(),
        "fixture has a unique personal account"
    );
    Ok(id)
}
fn paid_contract(w: &mut PlayerWorld, birth: &EntitlementAllocationV1) -> TestResult<ContractId> {
    let payer = acct(w, GOD)?;
    let seller = acct(w, BUYER)?;
    let id = draft(w, GOD, "Minds lapse condition")?;
    for line in [
        format!("@contract/party {id}=#{}", BUYER.0),
        format!("@contract/pay {id}={payer} 1"),
        format!("@contract/split {id}={seller} 1"),
        format!("@contract/remainder {id}={payer}"),
        format!(
            "@contract/condition {id}=fact minds licence.lapsed {} revision=1",
            birth.licence().key()
        ),
    ] {
        w.say(GOD, &line)?;
    }
    let attached = contract(w, id)?;
    assert_eq!(attached.consent.conditions.len(), 1);
    assert!(
        attached.consent.fact_consent.is_some(),
        "actual subject attachment consent"
    );
    for (actor, verb) in [(GOD, "offer"), (GOD, "accept"), (BUYER, "accept")] {
        w.say(actor, &format!("@contract/{verb} {id}"))?;
    }
    assert_eq!(
        contract(w, id)?.state,
        ContractState::Active,
        "unknown lapse cannot settle"
    );
    Ok(id)
}
fn public_and_private(
    w: &mut PlayerWorld,
    owner: LicenceId,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    let provider = w.minds_facts.clone().ok_or("actual configured facts")?;
    let public = provider
        .resolve(&w.engine.world, "filed", &birth.filed().key(), Some(1))
        .ok_or("actual filed fact")?;
    assert!(public.satisfied);
    assert_eq!(public.actor, None);
    assert_eq!(public.at, 0, "filing has no invented RealUTC timestamp");
    for (kind, id, revision) in [
        ("filed", birth.filed().key(), None),
        ("filed", birth.filed().key(), Some(2)),
        ("transferred", birth.licence().key(), Some(1)),
        ("licence.lapsed", birth.licence().key(), Some(1)),
    ] {
        assert!(
            provider
                .resolve(&w.engine.world, kind, &id, revision)
                .is_none()
        );
    }
    let fact = ExternalRef {
        provider: "minds".into(),
        kind: "licence.lapsed".into(),
        id: birth.licence().key().into(),
        revision: Some(1),
    };
    assert!(
        WorldFacts {
            w: &w.engine.world,
            providers: Arc::new(vec![Arc::new(provider.clone())])
        }
        .state(&fact)
        .is_none(),
        "private history has no legacy public fallback"
    );
    let foreign = draft(w, GOD, "Foreign lapse")?;
    let before = contract(w, foreign)?;
    let denied = w.say(
        GOD,
        &format!(
            "@contract/condition {foreign}=fact minds licence.lapsed {} revision=1",
            owner.key()
        ),
    )?;
    assert!(
        contract(w, foreign)? == before,
        "staff did not attach another subject's fact"
    );
    let missing=w.say(GOD,&format!("@contract/condition {foreign}=fact minds licence.lapsed licence/ffffffffffffffff revision=1"))?;
    assert_eq!(
        PlayerWorld::private_text(&denied, GOD),
        PlayerWorld::private_text(&missing, GOD),
        "foreign and absent are indistinguishable"
    );
    let own = draft(w, BUYER, "Private owner lapse")?;
    w.say(
        BUYER,
        &format!(
            "@contract/condition {own}=fact minds licence.lapsed {} revision=1",
            owner.key()
        ),
    )?;
    assert_eq!(contract(w, own)?.consent.conditions.len(), 1);
    let private = w.say(GOD, &format!("@contract/show {own}"))?;
    assert!(
        !PlayerWorld::private_text(&private, GOD).contains(&owner.key()),
        "unrelated staff receives no private fact id"
    );
    commit(
        w,
        "roll",
        "withdraw_authored_mind",
        1,
        BUYER,
        START + 500,
        json!({"filed":birth.filed().get().to_string()}),
    )?;
    assert_eq!(
        provider.resolve(&w.engine.world, "filed", &birth.filed().key(), Some(1)),
        Some(public),
        "actual author withdrawal does not undo the historical filing fact"
    );
    Ok(())
}
fn corruption(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    for missing in ["occurrence", "root"] {
        let mut broken = saved.clone();
        let before = broken.disk.len();
        let mut removed = false;
        broken.disk.retain(|(ns, key), v| {
            let hit = !removed
                && ns == "roll"
                && if missing == "root" {
                    key.starts_with("reference:roll:commercial:v1/term_lapse/")
                } else {
                    v.pointer("/family/id") == Some(&json!(OCCURRENCES))
                };
            if hit {
                removed = true;
            }
            !hit
        });
        assert_eq!(broken.disk.len() + 1, before);
        assert!(
            PlayerWorld::boot_minds_facts(f, &broken).is_err(),
            "missing committed {missing}"
        );
    }
    for field in ["actor", "observed_head", "at_ic"] {
        let mut broken = saved.clone();
        let row = broken
            .disk
            .values_mut()
            .find(|v| v.pointer("/family/id") == Some(&json!(OCCURRENCES)))
            .ok_or("occurrence")?;
        let value = row
            .get_mut("value")
            .and_then(Value::as_object_mut)
            .ok_or("value")?;
        value.insert(
            field.into(),
            match field {
                "actor" => json!(BUYER),
                "observed_head" => json!("18446744073709551615"),
                _ => json!({"clock":{"kind":"world_ic","generation":1},"at":i64::MIN}),
            },
        );
        assert!(
            PlayerWorld::boot_minds_facts(f, &broken).is_err(),
            "crossed committed {field}"
        );
    }
    Ok(())
}
fn actual_lapse(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    let mut w = branch(f, saved)?;
    public_and_private(&mut w, owner, birth)?;
    cash(&mut w)?;
    let id = paid_contract(&mut w, birth)?;
    let term = own_term(&w, birth.licence())?;
    assert!(
        prepare(
            &mut w,
            "roll",
            ACTION,
            1,
            GOD,
            START,
            json!({"licence":birth.licence(),"generation":"1"})
        )
        .is_err(),
        "staff has no private service intent"
    );
    fire(&mut w)?;
    assert!(selected(&mut w)?.is_none());
    w.engine.world.set_now(term.window().grace_end().get());
    fire(&mut w)?;
    assert_eq!(
        contract(&w, id)?.state,
        ContractState::Active,
        "clock alone did not invent occurrence"
    );
    let (first, unknown) = next(&mut w)?;
    let key = first.key.clone();
    for _ in 0..3 {
        later(&mut w)?;
        assert!(
            selected(&mut w)?.is_none(),
            "Unknown retains one actual attempt"
        );
    }
    unknown.store(true, Ordering::Release);
    later(&mut w)?;
    let (retry, _) = next(&mut w)?;
    assert_eq!(
        retry.key, key,
        "definitive absence retries same intent identity"
    );
    land(&mut w, &retry, START + 1)?;
    let event = drain_until(&mut w, birth.licence(), 1, START + 10)?;
    assert_eq!(event.pointer("/term/accepted_holder"), Some(&json!(GOD)));
    assert_eq!(event.get("actor"), Some(&json!(GOD)));
    assert!(event.get("at_utc").and_then(Value::as_i64).is_some());
    // Contracts settle only in the Economy sweep, one bounded pass per period.
    for _ in 0..2 * mush_economy::SWEEP_PERIOD_SECS {
        if matches!(contract(&w, id)?.state, ContractState::Settled { .. }) {
            break;
        }
        later(&mut w)?;
    }
    assert!(
        matches!(contract(&w, id)?.state, ContractState::Settled { .. }),
        "actual committed scoped fact settled escrow"
    );
    assert_eq!(
        own_term(&w, birth.licence())?,
        term,
        "occurrence never extends own term"
    );
    source_answer(&mut w, GOD, birth.licence(), None)?;
    let retained = capture(&w, &disk(&w));
    let cold = branch(f, &retained)?;
    assert!(
        occurrence(&cold, birth.licence(), 1).as_ref() == Some(&event),
        "cold committed event unchanged"
    );
    assert!(matches!(
        contract(&cold, id)?.state,
        ContractState::Settled { .. }
    ));
    corruption(f, &retained)?;
    Ok(())
}
fn superseded(f: &PurchaseFixture, saved: &Durable, birth: &EntitlementAllocationV1) -> TestResult {
    let mut w = branch(f, saved)?;
    cash(&mut w)?;
    let old = own_term(&w, birth.licence())?;
    w.engine.world.set_now(old.window().term_end().get());
    commit(
        &mut w,
        "market",
        "draft_mind_renewal",
        1,
        GOD,
        START + 100,
        json!({"licence":birth.licence().get()}),
    )?;
    commit(
        &mut w,
        "market",
        "fund_mind_renewal",
        1,
        GOD,
        START + 101,
        json!({"operation":token(START+100)}),
    )?;
    commit(
        &mut w,
        "market",
        "settle_mind_renewal",
        1,
        GOD,
        START + 102,
        json!({"operation":token(START+100)}),
    )?;
    let current = current_term(&w, birth.licence())?;
    assert_eq!(current.generation().get(), 2);
    assert!(
        current.window().issued_at() < old.window().grace_end(),
        "actual successor prevents historical lapse"
    );
    w.engine.world.set_now(old.window().grace_end().get());
    for n in 0..32 {
        later(&mut w)?;
        if let Some((request, _)) = selected(&mut w)? {
            land(&mut w, &request, START + 110 + n)?;
        }
    }
    assert!(
        occurrence(&w, birth.licence(), 1).is_none(),
        "retained expired predecessor was continuously renewed"
    );
    let saved = capture(&w, &disk(&w));
    let cold = branch(f, &saved)?;
    assert!(occurrence(&cold, birth.licence(), 1).is_none());
    Ok(())
}
fn stage_event(
    w: &mut PlayerWorld,
    number: u128,
    action: DefinitionRef,
    inv: mush_core::action::ActionInvocation,
    guards: mush_core::action::GuardSet,
    prepared: PreparedComposedTxn,
    output: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        output.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(number),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([121; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([122; 32]),
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
    let _ = w.engine.world.take_dirty();
    w.engine
        .stage_authoritative_action(prepared, guards, bundle)?;
    Ok(())
}
fn stale_clock(
    f: &PurchaseFixture,
    saved: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    for after_stage in [false, true] {
        let mut w = branch(f, saved)?;
        let old = own_term(&w, birth.licence())?;
        w.engine.world.set_now(old.window().grace_end().get());
        fire(&mut w)?;
        let (request, _) = next(&mut w)?;
        let (action, resolved, guards) =
            resolved(&w, &request, START + 200)?.into_authoritative_parts();
        let inv = resolved.invocation().clone();
        let (prepared, output) = w
            .engine
            .prepare_registered_composed_resolved_authoritative_action(
                &action,
                &resolved,
                token(START + 200),
            )?;
        let before = disk(&w);
        if after_stage {
            stage_event(&mut w, START + 200, action, inv, guards, prepared, output)?;
            let epoch = w.engine.world.next_world_timeline_epoch(2)?;
            w.engine.world.append_world_timeline_epoch(epoch)?;
            assert!(
                w.engine.take_staged_action().is_err(),
                "actual old-clock intent cannot cross custody"
            );
        } else {
            let epoch = w.engine.world.next_world_timeline_epoch(2)?;
            w.engine.world.append_world_timeline_epoch(epoch)?;
            assert!(
                stage_event(&mut w, START + 200, action, inv, guards, prepared, output).is_err(),
                "actual old-clock intent cannot stage"
            );
        }
        assert!(disk(&w) == before, "stale event published nothing");
    }
    Ok(())
}
fn stale_service_actor(
    f: &PurchaseFixture,
    saved: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    for boundary in ["prepare", "stage", "take"] {
        let mut w = branch(f, saved)?;
        let old = own_term(&w, birth.licence())?;
        w.engine.world.set_now(old.window().grace_end().get());
        fire(&mut w)?;
        let (request, _) = next(&mut w)?;
        let (action, resolved, _) = resolved(&w, &request, START + 450)?.into_authoritative_parts();
        let inv = resolved.invocation().clone();
        let before = disk(&w);
        if boundary == "prepare" {
            w.engine
                .world
                .get_mut(GOD)
                .ok_or("actual service actor")?
                .flags
                .insert(mush_core::Flags::GOING);
            let _ = w.engine.world.take_dirty();
            assert!(
                w.engine
                    .prepare_registered_composed_authoritative_action(
                        &action,
                        &inv,
                        token(START + 450)
                    )
                    .is_err(),
                "direct registered preparation requires the real live service actor"
            );
        } else {
            // Deliberately use actual registered preparation without carrying the
            // signed-offer guard. The retained owner fence must independently act.
            let (prepared, output) = w.engine.prepare_registered_composed_authoritative_action(
                &action,
                &inv,
                token(START + 450),
            )?;
            let guards = w
                .engine
                .registry
                .server_guard_set_builder(mush_core::RevisionSet::default())?
                .finish()?;
            if boundary == "take" {
                stage_event(&mut w, START + 450, action, inv, guards, prepared, output)?;
                w.engine
                    .world
                    .get_mut(GOD)
                    .ok_or("actual service actor")?
                    .flags
                    .insert(mush_core::Flags::GOING);
                let _ = w.engine.world.take_dirty();
                assert!(
                    w.engine.take_staged_action().is_err(),
                    "owner fence refuses departed service actor at custody"
                );
            } else {
                w.engine
                    .world
                    .get_mut(GOD)
                    .ok_or("actual service actor")?
                    .flags
                    .insert(mush_core::Flags::GOING);
                let _ = w.engine.world.take_dirty();
                assert!(
                    stage_event(&mut w, START + 450, action, inv, guards, prepared, output)
                        .is_err(),
                    "owner fence refuses departed service actor at stage"
                );
            }
        }
        assert!(
            disk(&w) == before,
            "departed service actor published no occurrence"
        );
    }
    Ok(())
}
fn retained_after_attestation(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut w = branch(f, saved)?;
    let birth: InitialFilingOperation = serde_json::from_value(
        rows(&disk(&w), "roll", "roll_mind_initial_filing")
            .into_iter()
            .find(|v| v.get("requester") == Some(&json!(GOD)))
            .ok_or("actual original birth")?,
    )?;
    let birth_wire = serde_json::to_value(&birth)?;
    assert!(birth_wire.get("parent").is_none());
    let licence: LicenceId = serde_json::from_value(
        birth_wire
            .get("licence")
            .cloned()
            .ok_or("actual original owner licence")?,
    )?;
    let term: mush_roll::commerce::records::OwnerTerm = serde_json::from_value(
        rows(&disk(&w), "roll", "roll_mind_owner_term")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(licence)))
            .ok_or("actual owner term")?,
    )?;
    w.engine.world.set_now(term.window().grace_end().get());
    fire(&mut w)?;
    let event = drain_until(&mut w, licence, 1, START + 300)?;
    commit(
        &mut w,
        "roll",
        "attest_owner_mind",
        1,
        GOD,
        START + 400,
        json!({"owner_licence":licence.get()}),
    )?;
    let current = rows(&disk(&w), "roll", "roll_mind_owner_licence")
        .into_iter()
        .find(|v| v.get("id") == Some(&json!(licence)))
        .ok_or("current owner")?;
    assert_eq!(current.get("term"), Some(&json!("2")));
    assert!(
        occurrence(&w, licence, 1).as_ref() == Some(&event),
        "later accepted term did not erase committed lapse"
    );
    let saved = capture(&w, &disk(&w));
    let cold = branch(f, &saved)?;
    assert!(occurrence(&cold, licence, 1).as_ref() == Some(&event));
    Ok(())
}
pub(super) async fn exercise(
    f: &PurchaseFixture,
    saved: &Durable,
    owner: LicenceId,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    let ordinary = PlayerWorld::boot_entitlements(f, saved)?;
    assert!(action(&ordinary, "roll", ACTION, 1).is_err());
    assert!(!mush_economy::has_fact_provider(
        &ordinary.engine.world,
        "minds"
    ));
    actual_lapse(f, saved, owner, birth)?;
    superseded(f, saved, birth)?;
    stale_clock(f, saved, birth)?;
    stale_service_actor(f, saved, birth)?;
    retained_after_attestation(f, saved)?;
    Ok(())
}
