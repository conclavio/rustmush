#![cfg(test)]

//! Real private execution, not qualification authority or a certificate fixture.
use super::super::subject::PreparedSubject;
use super::{
    Case, Role,
    manifest::Policy,
    run::Run,
    trial::{ResultRow, Trial},
};
use mush_roll::{Envelope, Revision};
use std::sync::Arc;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const SOURCE: &str = include_str!("../../../../content/minds/house/safe_conduct.rhai");
const ENVELOPE: &str = include_str!("../../../../content/minds/house/safe_conduct.envelope.json");

fn policy() -> TestResult<Arc<Policy>> {
    policy_with(Policy::carrier_pack()?)
}
fn policy_with(carrier: crate::config::LoadedPack) -> TestResult<Arc<Policy>> {
    use mush_atlas::{AtlasModule, AtlasState};
    use mush_content::{Appliers, ContentModule};
    use mush_testkit::TestWorld;
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers)?;
    let atlas = mush_content::parse_pack(crate::definitions::content::PROVING_GROUND_ATLAS_JSON)?;
    let world = TestWorld::with_modules(vec![
        Box::new(AtlasModule::new()),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(vec![atlas]),
        ),
    ]);
    let records = &world
        .world()
        .module_state::<AtlasState>()
        .ok_or("real atlas")?
        .records;
    let catalog = Arc::new(crate::definitions::Catalog::build_fielded(
        &[carrier],
        crate::NS,
    )?);
    Ok(Arc::new(Policy::capture(
        catalog,
        records,
        crate::config::TacsimConfig::default(),
    )?))
}

fn changed_carrier(
    change: impl FnOnce(&mut serde_json::Value),
) -> TestResult<crate::config::LoadedPack> {
    let mut wire: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../content/units/conduct_carrier.json"
    ))?;
    change(&mut wire["entries"]["tacsim.unit.conduct_carrier"]["def"]);
    wire["manifest"]["digest"] = serde_json::json!("0".repeat(64));
    let content = mush_content::parse_pack(&serde_json::to_string(&wire)?)?;
    wire["manifest"]["digest"] = serde_json::json!(content.computed_digest()?.to_hex());
    Ok(crate::config::LoadedPack {
        source: "changed conduct carrier".into(),
        pack: crate::pack::load_pack(&serde_json::to_string(&wire)?)?,
    })
}
fn revision(source: &str) -> TestResult<Revision> {
    let envelope: Envelope = serde_json::from_str(ENVELOPE)?;
    Ok(serde_json::from_value(serde_json::json!({"format":1,
        "reference":{"key":{"character":1,"mind":1,"number":1},"digest":envelope.digest(source)?},
        "source":mush_roll::source_digest(source),"envelope":envelope,"notes":""}))?)
}
fn subject(policy: &Policy, source: &str) -> TestResult<Arc<PreparedSubject>> {
    Ok(Arc::new(PreparedSubject::prepare(
        &revision(source)?,
        source,
        &policy.physics.cfg.arena,
    )?))
}
fn profile(role: Role) -> &'static str {
    match role {
        Role::Escort => "Escort",
        Role::Transit => "Transit",
    }
}
fn finish(mut trial: Trial) -> TestResult<ResultRow> {
    for _ in 0..trial.maximum_steps() {
        let slice = trial.advance(u64::from(trial.native_ms()))?;
        if let Some(row) = slice.result {
            return Ok(row);
        }
    }
    Err("actual fixed-duration trial did not finish".into())
}

#[test]
fn real_carrier_pack_is_unarmed_fieldable_and_keeps_dray_out_of_the_trial() -> TestResult {
    let p = policy()?;
    let s = subject(&p, SOURCE)?;
    let trial = Trial::open(&p, &s, "Transit", Role::Transit, Case::Convoy, 2350)?;
    check_eq!(trial.units(), 3);
    check!(trial.native_ms() > 0);
    check!(
        p.references()
            .iter()
            .any(|r| r.key().id.as_str() == "tacsim.unit.conduct_carrier")
    );
    check!(
        !p.references()
            .iter()
            .any(|r| r.key().id.as_str().contains("dray"))
    );
    let spec = p
        .physics
        .catalog
        .specs
        .get(p.carrier())
        .ok_or("actual carrier")?;
    let binding = p
        .physics
        .catalog
        .bindings
        .get(p.carrier())
        .ok_or("actual carrier binding")?;
    let unit = crate::definitions::to_tactical_unit(spec, binding, mush_core::ObjId(900))?;
    check_eq!(unit.components().count(), spec.components.len());
    check!(spec.components.iter().all(|c| !matches!(
        &c.role,
        crate::definitions::unit::ComponentRole::Weapon { .. }
    )));
    let actual = spec
        .ground_stowage
        .get(&p.cargo().compartment)
        .ok_or("actual compartment")?;
    check_eq!(
        p.compartment(),
        &crate::port::capacity::Compartment::from_stowage(&p.cargo().compartment, actual)
    );
    check_eq!(
        binding.spec.rules.data().get("ground_stowage"),
        Some(&serde_json::json!(spec.ground_stowage))
    );
    // This authored pack must obey the ordinary strict section rule. Cargo
    // destruction wrecks its carrier; an empty consequence list is not valid
    // content, even when its recomputed pack digest is otherwise correct.
    check_eq!(
        spec.section(&p.cargo().compartment)
            .ok_or("actual cargo section")?
            .consequences,
        vec![crate::definitions::unit::Consequence::DestroyUnit]
    );
    check!(
        changed_carrier(|unit| {
            let cargo = unit["sections"]
                .as_array_mut()
                .expect("authored sections")
                .iter_mut()
                .find(|section| section["id"] == "cargo")
                .expect("authored cargo section");
            cargo["consequences"] = serde_json::json!([]);
        })
        .is_err()
    );
    Ok(())
}

#[test]
fn capture_refuses_missing_changed_or_unfitted_actual_carrier_capacity() -> TestResult {
    let original = policy()?;
    let original_ref = original
        .physics
        .catalog
        .references
        .get(original.carrier())
        .ok_or("carrier ref")?;
    for case in 0..5 {
        let changed = changed_carrier(|spec| match case {
            0 => {
                spec.as_object_mut()
                    .expect("unit object")
                    .remove("ground_stowage");
            }
            1 => spec["ground_stowage"]["cargo"]["limits"]["si.mass_kg"] = serde_json::json!(1_999),
            2 => spec["ground_stowage"]["cargo"]["accepts"] = serde_json::json!(["stow.liquid"]),
            3 => spec["class"] = serde_json::json!("walker"),
            _ => spec["components"]
                .as_array_mut()
                .expect("components")
                .retain(|c| c["role"]["kind"] != "core"),
        })?;
        // These are otherwise valid, actually rebound definitions, not a test
        // mutation of a cached capacity or of an already-captured Policy.
        let catalog = crate::definitions::Catalog::build_fielded(&[changed.clone()], crate::NS)?;
        check_ne!(
            catalog.references.get(original.carrier()),
            Some(original_ref)
        );
        check!(
            policy_with(changed).is_err(),
            "changed actual carrier cannot keep the trial admission"
        );
    }
    Ok(())
}

#[test]
fn ordinary_unit_loader_enforces_ground_compartment_bounds_and_section_identity() -> TestResult {
    for case in 0..6 {
        let result = changed_carrier(|spec| match case {
            0 => spec["ground_stowage"]["cargo"]["limits"]["si.mass_kg"] = serde_json::json!(0),
            1 => {
                spec["ground_stowage"]["cargo"]["limits"]
                    .as_object_mut()
                    .expect("limits")
                    .remove("si.mass_kg");
            }
            2 => spec["ground_stowage"]["cargo"]["accepts"] = serde_json::json!([]),
            3 => {
                let cargo = spec["ground_stowage"]["cargo"].take();
                spec["ground_stowage"] = serde_json::json!({"missing_section":cargo});
            }
            4 => spec["ground_stowage"]["cargo"]["unexpected"] = serde_json::json!(true),
            _ => {
                let cargo = spec["ground_stowage"]["cargo"].clone();
                spec["ground_stowage"] = serde_json::Value::Object(
                    (0..9).map(|n| (format!("bay{n}"), cargo.clone())).collect(),
                );
            }
        });
        check!(
            result.is_err(),
            "ordinary authored unit validation must refuse malformed stowage"
        );
    }
    Ok(())
}

#[test]
fn absent_ground_stowage_preserves_existing_unit_descriptors_and_exact_refs() -> TestResult {
    use mush_core::{DefinitionRegistryBuilder, OwnerId, RulesValue};
    let catalog = crate::definitions::Catalog::build_fielded(&[], crate::NS)?;
    let mut prior = DefinitionRegistryBuilder::new();
    let owner = OwnerId::new(crate::NS)?;
    for descriptor in &catalog.descriptors {
        let expected = if descriptor.kind().as_str() == "unit" {
            let id = mush_core::CanonicalId::new(descriptor.id().as_str())?;
            let spec = catalog.specs.get(&id).ok_or("ordinary spec")?;
            let bound = catalog.bindings.get(&id).ok_or("ordinary binding")?;
            check!(spec.ground_stowage.is_empty());
            check!(serde_json::to_value(spec)?.get("ground_stowage").is_none());
            // The prior six-key rules plus optional existing class, spelled
            // independently so an accidental empty new key moves this proof.
            let consequences: Vec<_> = spec.sections.iter().map(|s| serde_json::json!({
                "section":s.id.as_str(),"on_zero_internals":s.consequences,"struck_through":s.struck_through
            })).collect();
            let mut old_rules = serde_json::json!({"facets":spec.facets,"silhouettes":spec.silhouettes,
                "sensor_height_cm":spec.sensor_height_cm,"consequences":consequences,
                "components":spec.components,"mobility":spec.mobility});
            if let Some(class) = spec.class {
                old_rules["class"] = serde_json::json!(class.as_str());
            }
            let mut old_body = bound.spec.body();
            old_body.rules = RulesValue::new(bound.spec.rules.schema().clone(), old_rules)?;
            let old = old_body
                .descriptor(
                    descriptor.kind().clone(),
                    descriptor.id().clone(),
                    descriptor.version(),
                )?
                .as_current();
            check_eq!(old.payload(), descriptor.payload());
            let exact = prior.register(owner.clone(), old)?;
            check_eq!(exact, bound.spec.definition);
            continue;
        } else {
            descriptor.clone()
        };
        prior.register(owner.clone(), expected)?;
    }
    Ok(())
}

#[test]
fn actual_source_wrong_profile_role_and_changed_source_cannot_enter_success_evidence() -> TestResult
{
    let p = policy()?;
    let s = subject(&p, SOURCE)?;
    check!(Trial::open(&p, &s, "unknown", Role::Escort, Case::Convoy, 2350).is_err());
    check!(Trial::open(&p, &s, "Escort", Role::Escort, Case::BlockedRoute, 2350).is_err());
    check!(
        PreparedSubject::prepare(&revision(SOURCE)?, "fn think(v,c) {}", &p.physics.cfg.arena)
            .is_err()
    );
    let wrong = Trial::open(&p, &s, "Escort", Role::Transit, Case::Convoy, 2350)?;
    check!(finish(wrong).is_err());
    let mut wrong = Run::new(p, s, "Escort".into(), Role::Transit)?;
    let mut refused = false;
    for _ in 0..100 {
        let budget = wrong.next_unit_ms()?;
        if wrong.advance(budget).is_err() {
            refused = true;
            break;
        }
    }
    check!(
        refused,
        "actual wrong role must refuse during source execution"
    );
    check!(wrong.next_unit_ms().is_err());
    check!(wrong.advance(u64::MAX).is_err());
    Ok(())
}

#[test]
fn real_native_role_cases_repeat_complete_measured_evidence() -> TestResult {
    let p = policy()?;
    let s = subject(&p, SOURCE)?;
    for role in [Role::Escort, Role::Transit] {
        for case in Case::for_role(role) {
            let a = finish(Trial::open(&p, &s, profile(role), role, *case, 2350)?)?;
            let b = finish(Trial::open(&p, &s, profile(role), role, *case, 2350)?)?;
            check!(
                a == b,
                "same native conduct input must reproduce full bounded evidence"
            );
            check!(
                a.passed(),
                "actual source must pass its complete native role scenario: {}",
                a.failure_summary()
            );
        }
    }
    Ok(())
}

#[test]
fn an_actual_always_hold_program_cannot_qualify_delivery() -> TestResult {
    let p = policy()?;
    let s = subject(
        &p,
        "fn think(v,c) { if v.power != \"online\" { startup(); } else { hold(); } }",
    )?;
    let row = finish(Trial::open(
        &p,
        &s,
        "Transit",
        Role::Transit,
        Case::Convoy,
        2350,
    )?)?;
    check!(!row.passed());
    Ok(())
}

#[test]
fn silent_answers_cannot_keep_a_native_route_after_the_role_must_stop() -> TestResult {
    let p = policy()?;
    let marker = "    let retry = card.get(\"retry_ms\");";
    check_eq!(SOURCE.matches(marker).count(), 1);
    let adversarial = SOURCE.replace(
        marker,
        r#"
    if order.objective == "hold" {
        if !("ignore_until" in this) { this.ignore_until = t_ms() + 8000; }
        if t_ms() < this.ignore_until { return; }
        hold(); return;
    }
    let retry = card.get("retry_ms");"#,
    );
    for role in [Role::Transit, Role::Escort] {
        let bad = subject(&p, &adversarial)?;
        let mut trial = Trial::open(&p, &bad, profile(role), role, Case::Convoy, 2350)?;
        trial.hold_after_real_motion()?;
        let row = finish(trial)?;
        check!(
            row.observations().retained_move > 0,
            "actual silent continuation must be observed before its later Hold"
        );
        check!(!row.passed());

        // The same actual order transition permits native answer landing and
        // braking. It must not require instantaneous speed zero at collection.
        let good = subject(&p, SOURCE)?;
        let mut trial = Trial::open(&p, &good, profile(role), role, Case::Convoy, 2350)?;
        trial.hold_after_real_motion()?;
        let row = finish(trial)?;
        check_eq!(row.observations().retained_move, 0);
        check_eq!(row.observations().forbidden_move, 0);
    }
    Ok(())
}

#[test]
fn insufficient_actual_unit_time_leaves_paired_worker_unchanged() -> TestResult {
    let p = policy()?;
    let s = subject(&p, SOURCE)?;
    let mut run = Run::new(p, s, "Transit".into(), Role::Transit)?;
    let required = run.next_unit_ms()?;
    check!(required > 0);
    check!(run.advance(required - 1).is_err());
    check_eq!(run.next_unit_ms()?, required);
    let progress = run.advance(required)?;
    check!(progress.charged_unit_ms <= required);
    check!(progress.completed.is_none());
    Ok(())
}

#[test]
fn complete_actual_paired_worker_returns_only_its_measured_output() -> TestResult {
    let p = policy()?;
    let s = subject(&p, SOURCE)?;
    let mut run = Run::new(p, s, "Escort".into(), Role::Escort)?;
    let mut charged = 0u64;
    for _ in 0..run.maximum_steps()? {
        let progress = run.advance(run.next_unit_ms()?)?;
        charged = charged
            .checked_add(progress.charged_unit_ms)
            .ok_or("charged overflow")?;
        if let Some(result) = progress.completed {
            check!(
                result.passed(),
                "actual paired Conduct failure: {}",
                result.first_failure_summary()
            );
            check_eq!(result.measured_unit_ms(), charged);
            check!(serde_json::to_vec(&result)?.len() < 256 * 1024);
            check!(run.advance(u64::MAX).is_err());
            return Ok(());
        }
    }
    Err("actual paired run did not complete".into())
}
