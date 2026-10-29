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
/// M8c B5: the seal-mark writer and its ring family.
const SEAL_MARK: &str = "record_licence_seal_mark";
const MARKS: &str = "roll_licence_seal_marks";
const UNATTESTABLE: &str = "licence.unattestable";
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
    check!(driven.is_observed());
    let (request, proof) = driven.into_parts();
    check_eq!(request.actor, GOD);
    check_eq!(request.action.key().owner.as_str(), "roll");
    check_eq!(request.action.key().id.as_str(), ACTION);
    check_eq!(request.subject, PerceivedRef::SelfActor);
    check!(request.targets.is_empty());
    check_eq!(
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
    check!(disk(w) == before, "preparation published no fact");
    check_eq!(balances(w)?, money, "lapse publication has no money effect");
    let (writes, install) = prepared.into_parts();
    apply(w, &writes, install)?;
    for (key, value) in &before {
        check!(
            disk(w).get(key) == Some(value),
            "lapse rewrote existing evidence"
        );
    }
    check_eq!(balances(w)?, money);
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
    let reply = w.say(actor, &format!("@contract/draft {name}"))?;
    let snapshot = w
        .engine
        .world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or("Economy")?
        .snapshot();
    let issuer = mush_org::PrincipalId::Object(actor);
    let id = snapshot
        .contracts()
        .filter(|c| c.issuer == issuer)
        .map(|c| c.id)
        .max()
        .ok_or("actual draft")?;
    let open: Vec<_> = snapshot
        .contracts()
        .filter(|c| c.issuer == issuer && !c.state.is_terminal())
        .map(|c| (c.id, c.name.clone(), c.state))
        .collect();
    let drafted = snapshot.contract(id).is_some_and(|c| c.name == name);
    check!(
        drafted,
        "{actor:?} draft refused: {}; open as issuer: {open:?}",
        PlayerWorld::private_text(&reply, actor)
    );
    Ok(id)
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
    check!(
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
    check_eq!(attached.consent.conditions.len(), 1);
    check!(
        attached.consent.fact_consent.is_some(),
        "actual subject attachment consent"
    );
    for (actor, verb) in [(GOD, "offer"), (GOD, "accept"), (BUYER, "accept")] {
        w.say(actor, &format!("@contract/{verb} {id}"))?;
    }
    check_eq!(
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
    check!(public.satisfied);
    check_eq!(public.actor, None);
    check_eq!(public.at, 0, "filing has no invented RealUTC timestamp");
    for (kind, id, revision) in [
        ("filed", birth.filed().key(), None),
        ("filed", birth.filed().key(), Some(2)),
        ("transferred", birth.licence().key(), Some(1)),
        ("licence.lapsed", birth.licence().key(), Some(1)),
    ] {
        check!(
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
    check!(
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
    check!(
        contract(w, foreign)? == before,
        "staff did not attach another subject's fact"
    );
    let missing=w.say(GOD,&format!("@contract/condition {foreign}=fact minds licence.lapsed licence/ffffffffffffffff revision=1"))?;
    check_eq!(
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
    check_eq!(contract(w, own)?.consent.conditions.len(), 1);
    let private = w.say(GOD, &format!("@contract/show {own}"))?;
    check!(
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
    check_eq!(
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
        check_eq!(broken.disk.len() + 1, before);
        check!(
            crate::test_checks::refused(PlayerWorld::boot_minds_facts(f, &broken))?,
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
        check!(
            crate::test_checks::refused(PlayerWorld::boot_minds_facts(f, &broken))?,
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
    check!(
        crate::test_checks::refused(prepare(
            &mut w,
            "roll",
            ACTION,
            1,
            GOD,
            START,
            json!({"licence":birth.licence(),"generation":"1"})
        ))?,
        "staff has no private service intent"
    );
    fire(&mut w)?;
    check!(selected(&mut w)?.is_none());
    w.engine.world.set_now(term.window().grace_end().get());
    fire(&mut w)?;
    check_eq!(
        contract(&w, id)?.state,
        ContractState::Active,
        "clock alone did not invent occurrence"
    );
    let (first, unknown) = next(&mut w)?;
    let key = first.key.clone();
    for _ in 0..3 {
        later(&mut w)?;
        check!(
            selected(&mut w)?.is_none(),
            "Unknown retains one actual attempt"
        );
    }
    unknown.store(true, Ordering::Release);
    later(&mut w)?;
    let (retry, _) = next(&mut w)?;
    check_eq!(
        retry.key,
        key,
        "definitive absence retries same intent identity"
    );
    land(&mut w, &retry, START + 1)?;
    let event = drain_until(&mut w, birth.licence(), 1, START + 10)?;
    check_eq!(event.pointer("/term/accepted_holder"), Some(&json!(GOD)));
    check_eq!(event.get("actor"), Some(&json!(GOD)));
    check!(event.get("at_utc").and_then(Value::as_i64).is_some());
    // Contracts settle only in the Economy sweep, one bounded pass per period.
    for _ in 0..2 * mush_economy::SWEEP_PERIOD_SECS {
        if matches!(contract(&w, id)?.state, ContractState::Settled { .. }) {
            break;
        }
        later(&mut w)?;
    }
    check!(
        matches!(contract(&w, id)?.state, ContractState::Settled { .. }),
        "actual committed scoped fact settled escrow"
    );
    check_eq!(
        own_term(&w, birth.licence())?,
        term,
        "occurrence never extends own term"
    );
    source_answer(&mut w, GOD, birth.licence(), None)?;
    let retained = capture(&w, &disk(&w));
    let cold = branch(f, &retained)?;
    check!(
        occurrence(&cold, birth.licence(), 1).as_ref() == Some(&event),
        "cold committed event unchanged"
    );
    check!(matches!(
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
    check_eq!(current.generation().get(), 2);
    check!(
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
    check!(
        occurrence(&w, birth.licence(), 1).is_none(),
        "retained expired predecessor was continuously renewed"
    );
    let saved = capture(&w, &disk(&w));
    let cold = branch(f, &saved)?;
    check!(occurrence(&cold, birth.licence(), 1).is_none());
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
            check!(
                w.engine.take_staged_action().is_err(),
                "actual old-clock intent cannot cross custody"
            );
        } else {
            let epoch = w.engine.world.next_world_timeline_epoch(2)?;
            w.engine.world.append_world_timeline_epoch(epoch)?;
            check!(
                stage_event(&mut w, START + 200, action, inv, guards, prepared, output).is_err(),
                "actual old-clock intent cannot stage"
            );
        }
        check!(disk(&w) == before, "stale event published nothing");
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
            check!(
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
                check!(
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
                check!(
                    stage_event(&mut w, START + 450, action, inv, guards, prepared, output)
                        .is_err(),
                    "owner fence refuses departed service actor at stage"
                );
            }
        }
        check!(
            disk(&w) == before,
            "departed service actor published no occurrence"
        );
    }
    Ok(())
}
/// M8c B5/F7: one seal mark on `licence`, written by `writer` through the
/// writer: the hull an object named `hull` (made here), the sortie a
/// `SortieId`'s canonical string form (`<hull>@<at_ic>`).
#[allow(clippy::too_many_arguments)]
fn mark_by(
    w: &mut PlayerWorld,
    writer: ObjId,
    n: u128,
    licence: LicenceId,
    hull: &str,
    at_ic: i64,
    governed: bool,
) -> TestResult<ObjId> {
    let id = w
        .engine
        .world
        .create(mush_core::ObjKind::Thing, hull, GOD, None)?;
    commit(
        w,
        "roll",
        SEAL_MARK,
        1,
        writer,
        n,
        json!({"licence": licence, "hull": id, "at_ic": at_ic, "governed": governed, "sortie": format!("{}@{at_ic}", id.0)}),
    )?;
    Ok(id)
}
/// `mark_by` GOD.
fn mark(
    w: &mut PlayerWorld,
    n: u128,
    licence: LicenceId,
    hull: &str,
    at_ic: i64,
    governed: bool,
) -> TestResult<ObjId> {
    mark_by(w, GOD, n, licence, hull, at_ic, governed)
}
/// M8c B5: a paid contract whose one condition is the holder's own
/// `licence.unattestable` fact on `licence`'s first generation
/// (`paid_contract`'s shape, the kind swapped).
fn unattestable_contract(w: &mut PlayerWorld, licence: LicenceId) -> TestResult<ContractId> {
    let payer = acct(w, GOD)?;
    let seller = acct(w, BUYER)?;
    let id = draft(w, GOD, "Minds unattestable condition")?;
    for line in [
        format!("@contract/party {id}=#{}", BUYER.0),
        format!("@contract/pay {id}={payer} 1"),
        format!("@contract/split {id}={seller} 1"),
        format!("@contract/remainder {id}={payer}"),
        format!(
            "@contract/condition {id}=fact minds {UNATTESTABLE} {} revision=1",
            licence.key()
        ),
    ] {
        w.say(GOD, &line)?;
    }
    let attached = contract(w, id)?;
    check_eq!(
        attached.consent.conditions.len(),
        1,
        "the holder attaches its own fact"
    );
    check!(attached.consent.fact_consent.is_some());
    for (actor, verb) in [(GOD, "offer"), (GOD, "accept"), (BUYER, "accept")] {
        w.say(actor, &format!("@contract/{verb} {id}"))?;
    }
    check_eq!(contract(w, id)?.state, ContractState::Active);
    Ok(id)
}
/// M8c B5 (the brief's test 4; D-UM-42): GOD's original owner licence, due,
/// with seeded seal marks: a mark before the term's window and a governed
/// mark inside it leave the attestation attestable; one ungoverned mark
/// inside the window refuses it with the one sentence naming the hull and
/// the IC date, and publishes `licence.unattestable` on the term's
/// generation, which the holder's own E4 condition reads through the scoped
/// fact context (the contract settles in the Economy sweep) while the
/// public resolve never answers it; the ring keeps the last 64 marks.
fn unattestable(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut w = branch(f, saved)?;
    cash(&mut w)?;
    let birth_wire = rows(&disk(&w), "roll", "roll_mind_initial_filing")
        .into_iter()
        .find(|v| v.get("requester") == Some(&json!(GOD)))
        .ok_or("actual original birth")?;
    let licence: LicenceId =
        serde_json::from_value(birth_wire.get("licence").cloned().ok_or("owner licence")?)?;
    let term: mush_roll::commerce::records::OwnerTerm = serde_json::from_value(
        rows(&disk(&w), "roll", "roll_mind_owner_term")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(licence)))
            .ok_or("actual owner term")?,
    )?;
    let issued = term.window().issued_at().get();
    w.engine.world.set_now(term.window().grace_end().get());
    fire(&mut w)?;
    drain_until(&mut w, licence, 1, START + 600)?;
    let params = json!({"owner_licence": licence.get()});
    let provider = w.minds_facts.clone().ok_or("actual configured facts")?;
    check!(
        rows(&disk(&w), "roll", MARKS).is_empty(),
        "no production writer: the ring starts empty"
    );
    check!(
        crate::test_checks::refused(prepare(
            &mut w,
            "roll",
            SEAL_MARK,
            1,
            BUYER,
            START + 609,
            json!({"licence": licence, "hull": 1, "at_ic": issued, "governed": false, "sortie": "1@0"})
        ))?,
        "the writer is the Registry's"
    );
    // F7: the writer is gated on the Registry's capability word, not on
    // staff: a character granted it on an organisation's root scope writes.
    w.say(GOD, "@org/create Registry")?;
    w.say(
        GOD,
        &format!(
            "@org/grant #{}=Registry {}",
            BUYER.0,
            mush_roll::commerce::seal_marks::SEAL_MARK_CAPABILITY
        ),
    )?;
    // A mark before the term's window is ignored.
    let zero = mark_by(
        &mut w,
        BUYER,
        START + 610,
        licence,
        "Hull Zero",
        issued - 1,
        false,
    )?;
    check!(
        rows(&disk(&w), "roll", MARKS).len() == 1,
        "the granted writer wrote the ring"
    );
    check!(
        prepare(
            &mut w,
            "roll",
            "attest_owner_mind",
            1,
            GOD,
            START + 611,
            params.clone()
        )
        .is_ok(),
        "a mark outside the window is ignored"
    );
    // A governed mark inside it attests.
    mark(&mut w, START + 612, licence, "Hull One", issued + 10, true)?;
    check!(
        prepare(
            &mut w,
            "roll",
            "attest_owner_mind",
            1,
            GOD,
            START + 613,
            params.clone()
        )
        .is_ok(),
        "governed flights attest"
    );
    check!(
        provider
            .resolve(&w.engine.world, UNATTESTABLE, &licence.key(), Some(1))
            .is_none(),
        "the public resolve never answers the scoped fact"
    );
    let id = unattestable_contract(&mut w, licence)?;
    for _ in 0..2 {
        later(&mut w)?;
    }
    check_eq!(
        contract(&w, id)?.state,
        ContractState::Active,
        "governed flights publish no fact"
    );
    // One ungoverned flight in the window: the sentence, and the fact.
    let flew = issued + 20;
    let seven = mark(&mut w, START + 620, licence, "Hull Seven", flew, false)?;
    let error = prepare(
        &mut w,
        "roll",
        "attest_owner_mind",
        1,
        GOD,
        START + 621,
        params.clone(),
    )
    .err()
    .ok_or("an ungoverned flight refused the attestation")?
    .to_string();
    let expected = format!(
        "{} Hull Seven flew ungoverned on {}.",
        mush_roll::commerce::seal_marks::UNATTESTABLE,
        mush_core::clock::CivilDate::from_ic_seconds(flew)
    );
    check!(
        error.contains(&expected),
        "{error} should carry {expected:?}"
    );
    check!(
        provider
            .resolve(&w.engine.world, UNATTESTABLE, &licence.key(), Some(1))
            .is_none(),
        "still scoped: no public answer"
    );
    for _ in 0..2 * mush_economy::SWEEP_PERIOD_SECS {
        if matches!(contract(&w, id)?.state, ContractState::Settled { .. }) {
            break;
        }
        later(&mut w)?;
    }
    check!(
        matches!(contract(&w, id)?.state, ContractState::Settled { .. }),
        "the holder's own condition reads the unattestable fact"
    );
    // A later governed flight does not clear it.
    mark(&mut w, START + 630, licence, "Hull Eight", flew + 5, true)?;
    check!(
        prepare(
            &mut w,
            "roll",
            "attest_owner_mind",
            1,
            GOD,
            START + 631,
            params.clone()
        )
        .is_err(),
        "an ungoverned flight in the window is not forgiven"
    );
    // The ring keeps 64 marks: four so far, then 66 governed ones. F6: the
    // six dropped are the oldest GOVERNED marks (Hull One, Hull Eight, then
    // Hull 0..3); both ungoverned marks stay, whatever their age.
    let mut last = ObjId(0);
    for i in 0..66_u128 {
        last = mark(
            &mut w,
            START + 700 + i,
            licence,
            &format!("Hull {i}"),
            flew + 100 + i64::try_from(i)?,
            true,
        )?;
    }
    let ring = rows(&disk(&w), "roll", MARKS)
        .into_iter()
        .find(|v| v.get("licence") == Some(&json!(licence)))
        .ok_or("the licence's ring")?;
    let marks = ring.get("marks").and_then(Value::as_array).ok_or("marks")?;
    check_eq!(marks.len(), 64);
    check_eq!(
        marks.first().and_then(|m| m.get("hull")),
        Some(&json!(zero)),
        "the oldest ungoverned mark is kept"
    );
    check_eq!(
        marks.get(1).and_then(|m| m.get("hull")),
        Some(&json!(seven)),
        "the ungoverned flight in the window is kept"
    );
    check_eq!(
        marks.get(2).and_then(|m| m.get("at_ic")),
        Some(&json!(flew + 104)),
        "the oldest six governed dropped"
    );
    check_eq!(marks.last().and_then(|m| m.get("hull")), Some(&json!(last)));
    let cold = branch(f, &capture(&w, &disk(&w)))?;
    check_eq!(
        rows(&disk(&cold), "roll", MARKS),
        rows(&disk(&w), "roll", MARKS),
        "the ring is retained"
    );
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
    check!(birth_wire.get("parent").is_none());
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
    check_eq!(current.get("term"), Some(&json!("2")));
    check!(
        occurrence(&w, licence, 1).as_ref() == Some(&event),
        "later accepted term did not erase committed lapse"
    );
    let saved = capture(&w, &disk(&w));
    let cold = branch(f, &saved)?;
    check!(occurrence(&cold, licence, 1).as_ref() == Some(&event));
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
    unattestable(f, saved)?;
    Ok(())
}
