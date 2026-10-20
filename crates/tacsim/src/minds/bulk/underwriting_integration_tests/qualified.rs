#![cfg(test)]

//! A single real Safe Conduct certificate and completed Transit trial feed the
//! actual coverage owners. Only region events and submission dispositions use
//! the parent's labeled fixture boundaries; no decoded approval grants rights.
use super::*;
use crate::underwriting::{ApprovedConductV1, SafeConductRole};
use mush_roll::commerce::{conduct::ConductTrialProof, registry_recall::RegistryRecallPolicy};
use std::collections::BTreeMap;

const N: u128 = 154_000_000;

#[derive(Clone)]
pub(crate) struct Configuration {
    pub(crate) manifest: DefinitionRef,
    pub(crate) selections: BTreeMap<DefinitionDigest, Vec<CommitToken>>,
    pub(crate) recall: RegistryRecallPolicy,
}

fn product(
    ordinary: &CoverageProductV1,
    proof: &ConductTrialProof,
    role: SafeConductRole,
) -> TestResult<CoverageProductV1> {
    Ok(CoverageProductV1::new(
        ordinary.insurer(),
        ordinary.scope(),
        ordinary.treasury(),
        ordinary.adjudicator(),
        ordinary.terms().clone(),
        ordinary.hulls().to_vec(),
        ordinary.zones().to_vec(),
        vec![ApprovedConductV1::new(
            proof.charter(),
            proof.line(),
            proof.filed(),
            proof.revision(),
            proof.rating().receipt_digest(),
            proof.digest(),
            role,
            proof.profile().into(),
        )?],
        ordinary.quote_seconds(),
    )?)
}

fn recall(w: &mut IcWorld, proof: &ConductTrialProof, n: u128) -> TestResult {
    let action = w
        .qualified_coverage
        .as_ref()
        .ok_or("actual recall configuration")?
        .recall
        .action()?;
    let (prepared, _, _) = prepare(
        w,
        GOD,
        &action,
        n,
        json!({"filed":proof.filed().get().to_string(),"reason":"registry_review"}),
    )?;
    // Install another actual acknowledged owner transaction while Bind may
    // hold native custody. This creates no synthetic eligibility/status row.
    let (writes, installation) = prepared.into_parts();
    retain_writes(&mut w.disk, &writes)?;
    w.engine.install_composed_action(installation)?;
    Ok(())
}

fn quote_row(w: &IcWorld, n: u128) -> TestResult<Value> {
    rows(&w.disk, "tacsim", "tacsim_coverage_quotes")
        .into_iter()
        .find(|v| v.get("operation") == Some(&json!(token(n))))
        .ok_or_else(|| "actual quote".into())
}

fn cold_inverse(f: &PurchaseFixture, w: &IcWorld, saved: &Durable) -> TestResult {
    for family in ["roll_coverage_publication", "roll_conduct_trial_result"] {
        let mut bad = saved.clone();
        let keys = bad
            .disk
            .iter()
            .filter_map(|(k, v)| {
                (v.pointer("/family/id").and_then(Value::as_str) == Some(family))
                    .then_some(k.clone())
            })
            .collect::<Vec<_>>();
        assert!(
            !keys.is_empty(),
            "actual retained qualification counterpart"
        );
        for key in keys {
            bad.disk.remove(&key);
        }
        assert!(
            w.reboot(f, &bad).is_err(),
            "missing actual qualification counterpart"
        );
    }
    let mut bad = saved.clone();
    let keys = bad
        .disk
        .keys()
        .filter(|(namespace, key)| {
            namespace == "roll"
                && key.starts_with("reference:roll:commercial:v1/coverage_publication/")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(keys.len(), 1, "one actual publication retention root");
    for key in keys {
        bad.disk.remove(&key);
    }
    assert!(
        w.reboot(f, &bad).is_err(),
        "missing publication source retention"
    );
    Ok(())
}

#[inline(never)]
fn configure_recall(
    w: &mut IcWorld,
    retained: &ConductTrialProof,
    qualified: &CoverageProductV1,
    wrong_role: &CoverageProductV1,
) -> TestResult<Configuration> {
    w.command("@org/create ConductRegistry")?;
    w.command("@org/scope create ConductRegistry=recall")?;
    let state = w
        .engine
        .world
        .module_state::<OrgStore>()
        .ok_or("Org")?
        .state();
    let org = state
        .orgs()
        .find(|r| r.name.as_str() == "ConductRegistry")
        .ok_or("Registry")?
        .id;
    let scope = state
        .scopes_in(org)
        .find(|r| r.name.as_str() == "recall")
        .ok_or("scope")?
        .id;
    w.command(&format!(
        "@org/grant #{}=ConductRegistry roll.registry.recall scope={}",
        GOD.0, scope.0
    ))?;
    let config = Configuration {
        manifest: retained.manifest().clone(),
        selections: BTreeMap::from([
            (qualified.digest()?, vec![retained.operation()]),
            (wrong_role.digest()?, vec![retained.operation()]),
        ]),
        recall: RegistryRecallPolicy::new(issuer()?, org, scope, vec![GOD])?,
    };
    Ok(config)
}

#[inline(never)]
fn publish_products(
    f: &PurchaseFixture,
    w: &mut IcWorld,
    ordinary: &CoverageProductV1,
    qualified: &CoverageProductV1,
    wrong_role: &CoverageProductV1,
) -> TestResult<Durable> {
    let publication = reference(w, "publish_hull_coverage")?;
    let before = w.disk.clone();
    assert!(
        prepare(
            w,
            GOD,
            &publication,
            N,
            json!({"product":wrong_role.digest()?.to_hex()})
        )
        .is_err()
    );
    assert!(w.disk == before, "wrong role changed durable records");
    for (n, product) in [(N, ordinary), (N + 1, qualified)] {
        let p = prepare(
            w,
            GOD,
            &publication,
            n,
            json!({"product":product.digest()?.to_hex()}),
        )?;
        install(w, &publication, n, p)?;
    }
    let published = w.snapshot();
    cold_inverse(f, w, &published)?;
    Ok(published)
}

#[inline(never)]
fn quote_other_profile(
    other: &mut IcWorld,
    zone: &DefinitionRef,
    hull: ObjId,
    payer: MoneyAccount,
    treasury: MoneyAccount,
    original: (Amount, Amount),
) -> TestResult {
    let other_permit = ic_sortie::fixture_withheld(
        ic_sortie::fixture_start(&other.engine.world, BUYER, hull)?,
        zone.clone(),
        DefinitionDigest::from_bytes([111; 32]),
    )?;
    let pending =
        other
            .host
            .enqueue_withheld(&other_permit, LicenceId::new(2)?, vec!["Escort".into()])?;
    other.land(pending.ticket(), N + 2, hull, false)?;
    let departure = Arc::new(
        other
            .host
            .acknowledge_withheld(&other.engine.world, &pending)?
            .ok_or("actual Escort Open")?,
    );
    let other_host = host(other)?;
    let quote = other_host.quote(
        &other.engine.world,
        departure,
        token(N + 1),
        payer,
        Amount::from_minor(10_000),
    )?;
    observed_quote(other, quote, N + 3)?;
    let row = quote_row(other, N + 3)?;
    assert_eq!(row["terms"]["premium"], json!(100));
    assert_eq!(row["risk"]["qualified"], Value::Null);
    assert_eq!((money(other, payer)?, money(other, treasury)?), original);

    Ok(())
}

#[inline(never)]
fn recall_bind_fences(
    f: &PurchaseFixture,
    w: &IcWorld,
    quoted: &Durable,
    retained: &ConductTrialProof,
    departure: &Arc<ic_sortie::AcknowledgedDeparture>,
    accounts: (MoneyAccount, MoneyAccount),
    original: (Amount, Amount),
) -> TestResult {
    let (payer, treasury) = accounts;
    for phase in 0..3 {
        let mut probe = w.reboot(f, quoted)?;
        let ph = host(&probe)?;
        let bind = ph.bind(&probe.engine.world, departure.clone(), token(N + 6))?;
        if phase == 0 {
            recall(&mut probe, retained, N + 10)?;
            assert!(prepare(&mut probe, BUYER, bind.action(), N + 11, json!({})).is_err());
        } else {
            let (p, o, inv) = prepare(&mut probe, BUYER, bind.action(), N + 11, json!({}))?;
            if phase == 1 {
                recall(&mut probe, retained, N + 10)?;
                assert!(
                    probe
                        .stage_action(bind.action(), N + 11, p, o, &inv)
                        .is_err()
                );
            } else {
                probe.stage_action(bind.action(), N + 11, p, o, &inv)?;
                recall(&mut probe, retained, N + 10)?;
                assert!(probe.engine.take_staged_action().is_err());
            }
        }
        assert!(rows(&probe.disk, "tacsim", "tacsim_coverage_policies").is_empty());
        assert_eq!((money(&probe, payer)?, money(&probe, treasury)?), original);
    }
    Ok(())
}

#[inline(never)]
fn create_qualified_carrier(w: &mut IcWorld) -> TestResult<ObjId> {
    // Author one real Class-II carrier through the registered unit command.
    let hull =
        w.engine
            .world
            .create(ObjKind::Thing, "Conduct insured carrier", GOD, Some(w.base))?;
    w.engine.world.attach_behavior(
        hull,
        mush_core::BehaviorId::new("sim.participant")?,
        mush_core::BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 0,
        },
    )?;
    w.persist_setup()?;
    w.command(&format!(
        "@unit/create #{}=tacsim.unit.conduct_carrier",
        hull.0
    ))?;
    w.command(&format!("@asset/title #{}=#{}", hull.0, BUYER.0))?;
    w.command(&format!(
        "@spatial/place/interior #{}=#{}",
        hull.0, w.base.0
    ))?;
    assert!(mush_tactics::module::unit_of(&w.engine.world, hull).is_some());
    w.hulls[0] = hull;
    Ok(hull)
}

#[inline(never)]
fn quote_and_bind(
    f: &PurchaseFixture,
    w: &mut IcWorld,
    retained: &ConductTrialProof,
    hull: ObjId,
    zone: &DefinitionRef,
    accounts: (MoneyAccount, MoneyAccount),
    original: (Amount, Amount),
) -> TestResult<(
    Arc<ic_sortie::WithheldDeparture>,
    Arc<ic_sortie::AcknowledgedDeparture>,
    Amount,
)> {
    let (payer, treasury) = accounts;
    let permit = ic_sortie::fixture_withheld(
        ic_sortie::fixture_start(&w.engine.world, BUYER, hull)?,
        zone.clone(),
        DefinitionDigest::from_bytes([112; 32]),
    )?;
    let pending = w
        .host
        .enqueue_withheld(&permit, LicenceId::new(2)?, vec!["Transit".into()])?;
    w.land(pending.ticket(), N + 4, hull, false)?;
    let departure = Arc::new(
        w.host
            .acknowledge_withheld(&w.engine.world, &pending)?
            .ok_or("actual Transit Open")?,
    );
    let h = host(w)?;
    for (publication, n, premium, discount) in [(N, N + 5, 100, 0), (N + 1, N + 6, 90, 10)] {
        let t = h.quote(
            &w.engine.world,
            departure.clone(),
            token(publication),
            payer,
            Amount::from_minor(10_000),
        )?;
        observed_quote(w, t, n)?;
        let row = quote_row(w, n)?;
        assert_eq!(row["terms"]["premium"], json!(premium));
        assert_eq!(row["terms"]["discount"], json!(discount));
        assert_eq!(row["risk"]["qualified"].is_null(), discount == 0);
    }
    assert_eq!((money(w, payer)?, money(w, treasury)?), original);
    let quoted = w.snapshot();
    recall_bind_fences(
        f,
        w,
        &quoted,
        retained,
        &departure,
        (payer, treasury),
        original,
    )?;
    let bind = h.bind(&w.engine.world, departure.clone(), token(N + 6))?;
    assert_eq!(land_ticket(w, bind, N + 7)?.kind(), "bound");
    let paid = original
        .0
        .checked_sub(Amount::from_minor(90))
        .ok_or("premium")?;
    assert_eq!(money(w, payer)?, paid);
    assert_eq!(
        money(w, treasury)?,
        original
            .1
            .checked_add(Amount::from_minor(90))
            .ok_or("premium")?
    );
    Ok((permit, departure, paid))
}

pub(in super::super) fn exercise(
    f: &PurchaseFixture,
    base: &IcWorld,
    saved: &Durable,
    retained: &ConductTrialProof,
) -> TestResult {
    eprintln!("M7_STORY phase=qualified_coverage event=begin");
    assert_eq!(retained.profile(), "Transit");
    assert_eq!(
        retained.role(),
        mush_roll::commerce::conduct::ConductRole::Transit
    );
    let mut w = base.reboot(f, saved)?;
    let hull = create_qualified_carrier(&mut w)?;
    let (mut w, ordinary, payer, treasury, _, _) = setup(f, &w, &w.snapshot())?;
    let qualified = product(&ordinary, retained, SafeConductRole::Transit)?;
    let wrong_role = product(&ordinary, retained, SafeConductRole::Escort)?;
    let config = configure_recall(&mut w, retained, &qualified, &wrong_role)?;
    let mut w = IcWorld::boot_source_qualified(
        f.into(),
        &w.snapshot(),
        w.base,
        w.field,
        w.hulls,
        true,
        w.pending_charters,
        w.paid_resales,
        w.registry_revocation.clone(),
        Some(vec![
            ordinary.clone(),
            qualified.clone(),
            wrong_role.clone(),
        ]),
        false,
        w.transfer_facts.clone(),
        true,
        Some(config),
    )?;
    let published = publish_products(f, &mut w, &ordinary, &qualified, &wrong_role)?;
    let original = (money(&w, payer)?, money(&w, treasury)?);
    let zone = selected_base(w.base, w.field)?.definition().clone();
    // A genuine different selected profile does not inherit Transit pricing.
    let mut other = w.reboot(f, &published)?;
    quote_other_profile(&mut other, &zone, hull, payer, treasury, original)?;

    let (permit, departure, paid) = quote_and_bind(
        f,
        &mut w,
        retained,
        hull,
        &zone,
        (payer, treasury),
        original,
    )?;
    let mut w = w.reboot(f, &w.snapshot())?;
    let recovered_host = host(&w)?;
    context(&mut w, BUYER, move |ctx| {
        recovered_host.recover_binding(ctx, departure, token(N + 7))
    })?;
    recall(&mut w, retained, N + 12)?;
    assert!(super::super::conduct::proof(&mut w, retained.operation()).is_err());
    // Historical proof remains valid after recall. It cannot buy a new policy,
    // but the already paid policy still permits its actual terminal transition.
    let mut w = w.reboot(f, &w.snapshot())?;
    let close = ic_sortie::fixture_covered_close(
        &w.engine.world,
        BUYER,
        token(N + 4),
        w.base,
        permit,
        zone,
        DefinitionDigest::from_bytes([113; 32]),
    )?;
    let close = w.host.enqueue_close(&close)?;
    w.land(&close, N + 13, hull, true)?;
    let hc = host(&w)?;
    let terminal = context(&mut w, BUYER, move |ctx| {
        hc.terminal(ctx, token(N + 7), true)
    })?;
    assert_eq!(land_ticket(&mut w, terminal, N + 14)?.kind(), "released");
    assert_eq!(money(&w, payer)?, paid, "cancellation retains the premium");
    let history = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("E4")?
        .coverage_history(token(N + 7))?
        .clone();
    assert!(matches!(
        &history
            .terminal()
            .ok_or("actual terminal")?
            .decision()
            .reason,
        mush_economy::managed_coverage::CoverageDecisionKindV1::Cancelled { .. }
    ));
    cold_inverse(f, &w, &w.snapshot())?;
    let cold = w.reboot(f, &w.snapshot())?;
    assert_eq!(
        cold.engine
            .world
            .module_state::<EconomyStore>()
            .ok_or("E4")?
            .coverage_history(token(N + 7))?,
        &history
    );
    assert_eq!(money(&cold, payer)?, paid);
    eprintln!("M7_STORY phase=qualified_coverage event=complete");
    Ok(())
}
