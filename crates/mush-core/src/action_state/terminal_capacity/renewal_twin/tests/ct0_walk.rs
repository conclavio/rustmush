//! Doc 46 CT.0: the renewal twin's walk, pinned in `tests/ct0/renewal_personal.snap` and
//! `tests/ct0/renewal_house.snap`. Since CT.3 it is one generic function over the issuer
//! (it was a macro expanded in each twin's test module); every section, label and value it
//! writes is what the macro wrote, so the two committed files do not move.
#![cfg(test)]
use super::service::{actor, close, closure, service_actor};
use super::*;
use crate::action_state::terminal_capacity::tests::ct0_kit as kit;
use mush_testkit::snapshot::Snapshot;

fn ct0_op(byte: u8) -> CommitToken {
    CommitToken::from_bytes([byte; 16])
}

/// The pair's identity, digests and derived outputs at one state.
fn ct0_state<I: Rig>(snap: &mut Snapshot, f: &Fixture<I>, operation: CommitToken) -> TestResult {
    let p = pair::<I>(&view(f), operation)?;
    snap.line("phase", &p.phase);
    snap.line(
        "licence predecessor next",
        &format!("{} {} {}", p.licence, p.predecessor, p.next),
    );
    let binding = p.binding()?;
    let offer = field(&p.market, "offer")?;
    let basis = field(offer, "basis")?;
    snap.wire("offer digest", field(&p.market, "offer_digest")?)?;
    snap.wire(
        "predecessor term digest",
        field(binding, "predecessor_term")?,
    )?;
    snap.wire(
        "predecessor economics digest",
        field(binding, "predecessor_economics")?,
    )?;
    snap.wire("template digest", field(basis, "template_digest")?)?;
    snap.wire(
        "grant economics digest",
        field(basis, "grant_economics_digest")?,
    )?;
    if let Some(chain) = basis.get("royalty_chain_digest") {
        snap.wire("royalty chain digest", chain)?;
    }
    snap.hex("capacity fingerprint", fingerprint::<I>(&p)?.as_bytes());
    snap.line("grant domain", I::grant_domain(field(binding, "grant")?)?);
    let economics = required(&view(f), &term_key(I::ECONOMICS, p.licence, p.predecessor)?)?;
    snap.line(
        "predecessor economics domain",
        I::economics_domain(&economics)?,
    );
    snap.wire("binding revision", &entitlement::binding_revision(binding)?)?;
    snap.wire("historical draft action", &historical_action::<I>(&p, 0)?)?;
    snap.wire("expected head", &expected_head(&p, operation)?)?;
    snap.line(
        "target count",
        &targets::<I>(operation, &p)?.len().to_string(),
    );
    snap.wire(
        "receipt",
        &value(&view(f), &operation_key(I::RECEIPT, operation)?)?,
    )?;
    snap.wire(
        "licence",
        &required(&view(f), &licence_key(LICENCE, p.licence)?)?,
    )?;
    snap.wire(
        "next term",
        &value(&view(f), &term_key(TERM, p.licence, p.next)?)?,
    )?;
    snap.wire(
        "next economics",
        &value(&view(f), &term_key(I::ECONOMICS, p.licence, p.next)?)?,
    )?;
    Ok(())
}

/// One accepted transition: its writes, the index after the install, the pair, and a
/// cold rebuild from the written documents.
fn ct0_step<I: Rig>(
    snap: &mut Snapshot,
    f: &mut Fixture<I>,
    twin: &str,
    title: &str,
    operation: CommitToken,
    prepared: PreparedTxn,
) -> TestResult {
    snap.section(&format!("{twin} renewal: {title}"));
    kit::prepared(snap, &prepared)?;
    install(f, prepared)?;
    kit::reservations(snap, &f.store)?;
    ct0_state(snap, f, operation)?;
    let cold = hydrate(f, &f.disk)?;
    kit::same_cold(snap, &f.store, &cold)
}

fn ct0_identity<I: Rig>(snap: &mut Snapshot, twin: &str) -> TestResult {
    snap.section(&format!("{twin} renewal: identity"));
    for (index, id) in I::ACTIONS.iter().enumerate() {
        snap.line(&format!("action id {index}"), id);
    }
    snap.line("service action id", I::SERVICE_ACTION);
    for (index, transition) in TRANSITIONS.iter().enumerate() {
        snap.line(&format!("transition {index}"), transition);
    }
    for (owner, family, ceiling) in families::<I>() {
        snap.line("family", &format!("{owner} {family} {ceiling}"));
    }
    for index in 0..6 {
        snap.wire(
            &format!("action payload {index}"),
            &action_payload::<I>(index),
        )?;
    }
    let plain = fixture::<I>()?;
    snap.wire("manifest", &plain.manifest)?;
    snap.wire("participants", &plain.participants)?;
    for index in 0..6 {
        snap.wire(
            &format!("action reference {index}"),
            &action_reference::<I>(&plain.manifest, &plain.participants, index)?,
        )?;
    }
    let protocol = I::protocol(&plain.store.registry).ok_or("protocol")?;
    snap.wire("protocol", protocol)?;
    protocol.validate_families(&plain.store.registry)?;
    snap.line("families", "valid");
    let market = OwnerId::new("market")?;
    for (owner, family, _) in families::<I>() {
        let owner = OwnerId::new(owner)?;
        let id = StateFamilyId::new(family)?;
        let entry = plain
            .store
            .registry
            .entries
            .get(&(owner.clone(), id.clone()))
            .ok_or("family entry")?;
        snap.wire(
            &format!("descriptor {family}"),
            &entry.descriptor().canonical_value(),
        )?;
        snap.line(
            &format!("delegated to market {family}"),
            &plain
                .store
                .registry
                .delegates_family(&owner, &id, &market)
                .to_string(),
        );
    }
    let service = fixture_config::<I>(1, true, true)?;
    snap.wire(
        "service protocol",
        I::protocol(&service.store.registry).ok_or("service protocol")?,
    )?;
    snap.wire("service action", &service.service)?;
    kit::refusal(
        snap,
        "declaration by another owner",
        Protocol::<I>::new(
            &OwnerId::new("roll")?,
            plain.manifest.clone(),
            plain.participants.clone(),
            plain.actions.clone(),
        )
        .map_err(Into::into),
    )?;
    let mut swapped = plain.actions.clone();
    swapped.swap(0, 1);
    kit::refusal(
        snap,
        "declaration with reordered actions",
        Protocol::<I>::new(
            &market,
            plain.manifest.clone(),
            plain.participants.clone(),
            swapped,
        )
        .map_err(Into::into),
    )?;
    let [first, second] = plain.participants.clone();
    kit::refusal(
        snap,
        "declaration with swapped participants",
        Protocol::<I>::new(
            &market,
            plain.manifest.clone(),
            [second, first],
            plain.actions.clone(),
        )
        .map_err(Into::into),
    )?;
    kit::refusal(
        snap,
        "declaration with a foreign service action",
        Protocol::<I>::with_service(
            &market,
            plain.manifest.clone(),
            plain.participants.clone(),
            plain.actions.clone(),
            plain.actions[4].clone(),
        )
        .map_err(Into::into),
    )
}

fn ct0_grant<I: Rig>(snap: &mut Snapshot, twin: &str) -> TestResult {
    let mut f = fixture::<I>()?;
    let op = ct0_op(31);
    let prepared = draft(&mut f, 31, None)?;
    ct0_step(
        snap,
        &mut f,
        twin,
        "grant walk: drafted (automatic)",
        op,
        prepared,
    )?;
    kit::refusal(
        snap,
        "second automatic draft while pending",
        draft(&mut f, 34, None),
    )?;
    let prepared = advance(&mut f, op, 1, 32)?;
    ct0_step(snap, &mut f, twin, "grant walk: funded", op, prepared)?;
    kit::refusal(snap, "fund twice", advance(&mut f, op, 1, 40))?;
    kit::refusal(
        snap,
        "unfunded close while funded",
        advance(&mut f, op, 4, 40),
    )?;
    let prepared = advance(&mut f, op, 2, 33)?;
    ct0_step(snap, &mut f, twin, "grant walk: granted", op, prepared)?;
    kit::refusal(snap, "grant twice", advance(&mut f, op, 2, 34))?;
    kit::refusal(snap, "refund after grant", advance(&mut f, op, 3, 34))?;
    let next = ct0_op(37);
    let prepared = draft(&mut f, 37, None)?;
    ct0_step(
        snap,
        &mut f,
        twin,
        "grant walk: next term drafted (automatic)",
        next,
        prepared,
    )?;
    kit::disk_keys(snap, &f.disk);
    Ok(())
}

fn ct0_refund<I: Rig>(snap: &mut Snapshot, twin: &str) -> TestResult {
    let mut f = fixture::<I>()?;
    let op = ct0_op(31);
    let prepared = draft(&mut f, 31, None)?;
    ct0_step(snap, &mut f, twin, "refund walk: drafted", op, prepared)?;
    let prepared = advance(&mut f, op, 1, 32)?;
    ct0_step(snap, &mut f, twin, "refund walk: funded", op, prepared)?;
    let prepared = advance(&mut f, op, 3, 33)?;
    ct0_step(snap, &mut f, twin, "refund walk: refunded", op, prepared)?;
    kit::refusal(snap, "grant after refund", advance(&mut f, op, 2, 34))?;
    kit::refusal(
        snap,
        "automatic draft after a refusal",
        draft(&mut f, 34, None),
    )?;
    kit::refusal(
        snap,
        "retry prepared before the refund",
        draft(&mut f, 20, Some(op)),
    )?;
    let retry = ct0_op(34);
    let prepared = draft(&mut f, 34, Some(op))?;
    ct0_step(
        snap,
        &mut f,
        twin,
        "refund walk: retry drafted",
        retry,
        prepared,
    )?;
    let prepared = advance(&mut f, retry, 1, 35)?;
    ct0_step(
        snap,
        &mut f,
        twin,
        "refund walk: retry funded",
        retry,
        prepared,
    )?;
    let prepared = advance(&mut f, retry, 2, 36)?;
    ct0_step(
        snap,
        &mut f,
        twin,
        "refund walk: retry granted",
        retry,
        prepared,
    )?;
    snap.section(&format!(
        "{twin} renewal: refund walk: the refunded attempt after the retry grant"
    ));
    ct0_state(snap, &f, op)?;
    kit::disk_keys(snap, &f.disk);
    Ok(())
}

fn ct0_unfunded<I: Rig>(snap: &mut Snapshot, twin: &str, reason: &str) -> TestResult {
    let mut f = fixture::<I>()?;
    let op = ct0_op(31);
    let prepared = draft(&mut f, 31, None)?;
    ct0_step(
        snap,
        &mut f,
        twin,
        &format!("unfunded {reason} walk: drafted"),
        op,
        prepared,
    )?;
    let action = f.actions[4].clone();
    let closure = |reason: &str| {
        json!({"kind":"unfunded_closed","closure":{"commit":ct0_op(32),
            "action":action,
            "at_ic":{"clock":{"kind":"world_ic","generation":1},"at":1032},
            "posted_at_utc":1032,"reason":reason}})
    };
    kit::refusal(
        snap,
        "unfunded close with an unknown reason",
        advance_with_service(&mut f, op, 4, 32, Some(closure("lapsed"))),
    )?;
    let prepared = advance_with_service(&mut f, op, 4, 32, Some(closure(reason)))?;
    ct0_step(
        snap,
        &mut f,
        twin,
        &format!("unfunded {reason} walk: closed"),
        op,
        prepared,
    )?;
    kit::refusal(snap, "fund after close", advance(&mut f, op, 1, 33))?;
    let retry = ct0_op(33);
    let prepared = draft(&mut f, 33, Some(op))?;
    ct0_step(
        snap,
        &mut f,
        twin,
        &format!("unfunded {reason} walk: retry drafted"),
        retry,
        prepared,
    )?;
    let prepared = advance(&mut f, retry, 4, 34)?;
    ct0_step(
        snap,
        &mut f,
        twin,
        &format!("unfunded {reason} walk: retry closed (declined)"),
        retry,
        prepared,
    )?;
    kit::disk_keys(snap, &f.disk);
    Ok(())
}

fn ct0_service<I: Rig>(snap: &mut Snapshot, twin: &str, reason: &str) -> TestResult {
    let mut f = fixture_config::<I>(1, true, true)?;
    let op = ct0_op(31);
    let prepared = draft(&mut f, 31, None)?;
    ct0_step(
        snap,
        &mut f,
        twin,
        &format!("service {reason} walk: drafted"),
        op,
        prepared,
    )?;
    let state = closure(&f, reason)?;
    kit::refusal(
        snap,
        "service close by the holder",
        close(&mut f, state.clone()),
    )?;
    service_actor(&mut f)?;
    kit::refusal(
        snap,
        "unfunded close by the service",
        advance(&mut f, op, 4, 32),
    )?;
    let mut wrong = state.clone();
    wrong["closure"]["reason"] = json!("expired");
    kit::refusal(
        snap,
        "service close for a holder reason",
        close(&mut f, wrong),
    )?;
    let prepared = close(&mut f, state.clone())?;
    ct0_step(
        snap,
        &mut f,
        twin,
        &format!("service {reason} walk: service closed"),
        op,
        prepared,
    )?;
    kit::refusal(snap, "service close twice", close(&mut f, state))?;
    actor(&mut f, ObjId(7), PrincipalId::Object { id: ObjId(7) })?;
    let retry = ct0_op(33);
    let prepared = draft(&mut f, 33, Some(op))?;
    ct0_step(
        snap,
        &mut f,
        twin,
        &format!("service {reason} walk: retry drafted"),
        retry,
        prepared,
    )?;
    kit::disk_keys(snap, &f.disk);
    Ok(())
}

fn ct0_service_funded<I: Rig>(snap: &mut Snapshot, twin: &str) -> TestResult {
    let mut f = fixture_config::<I>(1, true, true)?;
    let op = ct0_op(31);
    let prepared = draft(&mut f, 31, None)?;
    ct0_step(
        snap,
        &mut f,
        twin,
        "service funded walk: drafted",
        op,
        prepared,
    )?;
    let prepared = advance(&mut f, op, 1, 32)?;
    ct0_step(
        snap,
        &mut f,
        twin,
        "service funded walk: funded",
        op,
        prepared,
    )?;
    service_actor(&mut f)?;
    let state = closure(&f, "clock_invalidated")?;
    kit::refusal(snap, "service close of funded work", close(&mut f, state))?;
    kit::refusal(snap, "grant by the service", advance(&mut f, op, 2, 33))
}

fn ct0_manifest_changed<I: Rig>(snap: &mut Snapshot, twin: &str) -> TestResult {
    let mut old = fixture::<I>()?;
    let op = ct0_op(31);
    let prepared = draft(&mut old, 31, None)?;
    ct0_step(
        snap,
        &mut old,
        twin,
        "manifest changed walk: drafted",
        op,
        prepared,
    )?;
    let prepared = advance(&mut old, op, 1, 32)?;
    ct0_step(
        snap,
        &mut old,
        twin,
        "manifest changed walk: funded",
        op,
        prepared,
    )?;
    let mut current = fixture_with_manifest::<I>(2)?;
    current.store = hydrate(&current, &old.disk)?;
    current.disk = old.disk.clone();
    snap.wire("current manifest", &current.manifest)?;
    kit::refusal(
        snap,
        "grant under a changed manifest",
        advance(&mut current, op, 2, 33),
    )?;
    let prepared = advance(&mut current, op, 3, 34)?;
    ct0_step(
        snap,
        &mut current,
        twin,
        "manifest changed walk: refunded under the current manifest",
        op,
        prepared,
    )
}

fn ct0_follow<I: Rig>(snap: &mut Snapshot, twin: &str) -> TestResult {
    let mut f = fixture_from_seed::<I>(1, false, false, I::follow_birth_seed()?)?;
    let op = ct0_op(31);
    let prepared = draft(&mut f, 31, None)?;
    ct0_step(snap, &mut f, twin, "follow walk: drafted", op, prepared)?;
    let prepared = advance(&mut f, op, 1, 32)?;
    ct0_step(snap, &mut f, twin, "follow walk: funded", op, prepared)?;
    let prepared = advance(&mut f, op, 2, 33)?;
    ct0_step(snap, &mut f, twin, "follow walk: granted", op, prepared)?;
    kit::disk_keys(snap, &f.disk);
    Ok(())
}

/// The whole walk of one issuer, compared with its committed file. `twin` is the word the
/// file's sections carry (`personal`, `house`).
pub(in crate::action_state::terminal_capacity) fn walk<I: Rig>(
    twin: &str,
    file: &str,
) -> TestResult {
    let mut snap = Snapshot::new();
    ct0_identity::<I>(&mut snap, twin)?;
    ct0_grant::<I>(&mut snap, twin)?;
    ct0_refund::<I>(&mut snap, twin)?;
    ct0_unfunded::<I>(&mut snap, twin, "declined")?;
    ct0_unfunded::<I>(&mut snap, twin, "expired")?;
    ct0_service::<I>(&mut snap, twin, "deadline_reached")?;
    ct0_service::<I>(&mut snap, twin, "clock_invalidated")?;
    ct0_service_funded::<I>(&mut snap, twin)?;
    ct0_manifest_changed::<I>(&mut snap, twin)?;
    ct0_follow::<I>(&mut snap, twin)?;
    snap.check(env!("CARGO_MANIFEST_DIR"), file)?;
    Ok(())
}
