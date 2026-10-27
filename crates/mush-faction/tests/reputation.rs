//! Standing owner acceptance tests: real commands, immutable cause grants,
//! private adjudication, explicit publication, and persistence/replay.
use std::collections::HashMap;
use std::sync::Arc;

use mush_canon::{CanonModule, CanonScope, CanonState};
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, Effects, GOD, GameModule, ModError, ModuleBootData,
    ObjId, Outbox, Registrar,
};
use mush_faction::{
    FactionId, FactionModule, FactionStandingProvider, FactionState, StandingEffect,
    ensure_organization_faction, public_standing, record_standing,
};
use mush_org::{OrgId, OrgModule, OrgPredicateProvider};
use mush_testkit::{TestWorld, fold_module_ops_into, text_of};
use smol_str::SmolStr;

/// Test producer calls the owner API, never mutates another module's state.
struct Producer;
impl GameModule for Producer {
    fn name(&self) -> &'static str {
        "standing_test"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "test-grant".into(),
                shape: ArgShape::EqPair,
                ..CommandSpec::default()
            },
            Arc::new(grant),
        )?;
        reg.command(
            CommandSpec {
                name: "test-ensure".into(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                let id = ctx
                    .args
                    .raw()
                    .parse::<u64>()
                    .map_err(|_| CmdError::msg("org id"))?;
                let faction = ensure_organization_faction(ctx, OrgId(id)).map_err(CmdError::msg)?;
                ctx.out
                    .sys_to_actor(ctx.actor, "test", format!("Faction {}", faction.0));
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn grant(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let cause = ctx.args.lhs().to_owned();
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("grant parameters"))?
        .to_owned();
    let mut parts = rhs.splitn(3, '/');
    let subject = ObjId(
        parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| CmdError::msg("subject"))?,
    );
    let effects: Result<Vec<StandingEffect>, CmdError> = parts
        .next()
        .unwrap_or_default()
        .split(',')
        .map(|item| {
            let (fid, delta) = item
                .split_once(':')
                .ok_or_else(|| CmdError::msg("effect"))?;
            Ok(StandingEffect {
                faction: FactionId(fid.parse().map_err(|_| CmdError::msg("faction"))?),
                delta: delta.parse().map_err(|_| CmdError::msg("delta"))?,
            })
        })
        .collect();
    let fresh = record_standing(
        ctx,
        &cause,
        subject,
        &effects?,
        parts.next().unwrap_or_default(),
    )
    .map_err(CmdError::msg)?;
    ctx.out
        .sys_to_actor(ctx.actor, "test", if fresh { "applied" } else { "replay" });
    Ok(())
}

fn modules(with_canon: bool) -> Vec<Box<dyn GameModule>> {
    let mut result: Vec<Box<dyn GameModule>> =
        vec![Box::new(OrgModule::with_predicates(vec![Arc::new(
            FactionStandingProvider,
        )]))];
    if with_canon {
        result.push(Box::new(CanonModule::new()));
    }
    result.push(Box::new(FactionModule::new()));
    result.push(Box::new(Producer));
    result
}

fn boot(with_canon: bool) -> (TestWorld, ObjId, ObjId) {
    let mut world = TestWorld::with_modules(modules(with_canon));
    world.world_mut().set_now(1000);
    let pilot = world.spawn_player("Pilot");
    let victim = world.spawn_player("Victim");
    (world, pilot, victim)
}

fn run(world: &mut TestWorld, actor: ObjId, text: &str) -> String {
    world
        .run(actor, text)
        .iter()
        .map(|(_, event)| text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

fn faction(world: &TestWorld, name: &str) -> FactionId {
    world
        .world()
        .module_state::<FactionState>()
        .expect("factions")
        .faction_by_name(name)
        .expect("faction")
        .id
}

fn score(world: &TestWorld, faction: FactionId, subject: ObjId) -> i16 {
    public_standing(world.world(), faction, subject)
        .expect("public standing")
        .score
}

fn reboot(boot: HashMap<SmolStr, ModuleBootData>) -> (TestWorld, ObjId, ObjId) {
    let (mut world, pilot, victim) = self::boot(true);
    world.modules_loaded(boot);
    world.take_module_ops();
    (world, pilot, victim)
}

#[test]
fn immutable_grants_are_atomic_idempotent_and_do_not_make_members() {
    let (mut world, pilot, _) = boot(true);
    run(&mut world, GOD, "@faction/create Surveyors");
    run(&mut world, GOD, "@faction/create Rivals");
    let (ours, theirs) = (faction(&world, "Surveyors"), faction(&world, "Rivals"));
    let command = format!(
        "test-grant survey:complete:0={}/{}:15,{}:-5/Completed public commission",
        pilot.0, ours.0, theirs.0
    );
    assert!(run(&mut world, GOD, &command).contains("applied"));
    assert_eq!(score(&world, ours, pilot), 15);
    assert_eq!(score(&world, theirs, pilot), -5);
    assert!(run(&mut world, GOD, &command).contains("replay"));
    assert_eq!(score(&world, ours, pilot), 15);
    world.take_module_ops();
    assert!(run(&mut world, GOD, &command.replace(":15", ":16")).contains("conflicts"));
    assert!(world.take_module_ops().is_empty());
    assert!(
        run(
            &mut world,
            GOD,
            &format!(
                "test-grant bad:missing={}/{}:10,999:10/No partial grants",
                pilot.0, ours.0
            )
        )
        .contains("missing faction")
    );
    assert!(world.take_module_ops().is_empty());
    assert_eq!(score(&world, ours, pilot), 15);
    let provider = FactionStandingProvider;
    assert_eq!(
        provider.eval(world.world(), "member", &serde_json::json!(ours.0), pilot),
        Some(false)
    );
    assert_eq!(
        provider.eval(world.world(), "intel", &serde_json::json!(ours.0), pilot),
        Some(false)
    );
    assert_eq!(
        provider.eval(
            world.world(),
            "standing_at_least",
            &serde_json::json!({"faction":ours.0,"minimum":10}),
            pilot
        ),
        Some(true)
    );
    for minimum in [
        serde_json::json!(101),
        serde_json::json!("10"),
        serde_json::json!(1.5),
    ] {
        assert_eq!(
            provider.eval(
                world.world(),
                "standing_at_least",
                &serde_json::json!({"faction":ours.0,"minimum":minimum}),
                pilot
            ),
            None
        );
    }
}

#[test]
fn scores_derive_from_all_grants_not_intermediate_clamps_and_history_is_bounded() {
    let (mut world, pilot, _) = boot(true);
    run(&mut world, GOD, "@faction/create Surveyors");
    let fid = faction(&world, "Surveyors");
    for n in 0..12 {
        run(
            &mut world,
            GOD,
            &format!("test-grant work:{n}={}/{}:10/receipt{n}", pilot.0, fid.0),
        );
    }
    assert_eq!(score(&world, fid, pilot), 100);
    run(
        &mut world,
        GOD,
        &format!("test-grant correction:1={}/{}:-25/penalty", pilot.0, fid.0),
    );
    assert_eq!(score(&world, fid, pilot), 95);
    let output = run(&mut world, pilot, "+standing Surveyors");
    assert_eq!(output.lines().filter(|line| line.contains('[')).count(), 10);
    assert!(
        run(
            &mut world,
            GOD,
            &format!("test-grant work:0={}/{}:10/receipt0", pilot.0, fid.0)
        )
        .contains("replay")
    );
    assert_eq!(score(&world, fid, pilot), 95);
}

#[test]
fn private_attack_requires_publication_and_replays_after_reboot() {
    let (mut world, pilot, victim) = boot(true);
    run(&mut world, GOD, "@faction/create Surveyors");
    let fid = faction(&world, "Surveyors");
    let report = format!(
        "@faction/incident attack:1=Surveyors/#{}/#{}/Adjudicated ambush",
        victim.0, pilot.0
    );
    assert!(run(&mut world, pilot, &report).contains("Permission denied"));
    assert!(run(&mut world, GOD, &report).contains("Private incident recorded"));
    assert_eq!(score(&world, fid, pilot), 0);
    assert!(!run(&mut world, pilot, "+standing Surveyors").contains("ambush"));
    assert!(run(&mut world, pilot, "@faction/incident attack:1").contains("Permission denied"));
    assert_eq!(
        world
            .world()
            .module_state::<CanonState>()
            .expect("canon")
            .next_canon_id,
        0
    );
    let mut saved = HashMap::new();
    fold_module_ops_into(&mut saved, world.take_module_ops());
    let (mut world, pilot, _) = reboot(saved.clone());
    assert_eq!(score(&world, fid, pilot), 0);
    assert!(
        run(&mut world, GOD, "@faction/publish-incident attack:1").contains("Incident published")
    );
    assert_eq!(score(&world, fid, pilot), -20);
    let canon = world.world().module_state::<CanonState>().expect("canon");
    assert_eq!(canon.next_canon_id, 1);
    let event = canon.recent().pop().expect("public event");
    assert_eq!(event.scope, CanonScope::Public);
    assert_eq!(event.kind, "faction.attack_published");
    fold_module_ops_into(&mut saved, world.take_module_ops());
    let (mut world, pilot, _) = reboot(saved);
    assert!(
        run(&mut world, GOD, "@faction/publish-incident attack:1").contains("already published")
    );
    assert_eq!(score(&world, fid, pilot), -20);
    assert_eq!(
        world
            .world()
            .module_state::<CanonState>()
            .expect("canon")
            .next_canon_id,
        1
    );
    assert!(world.take_module_ops().is_empty());
}

#[test]
fn unknown_attacker_and_missing_canon_never_invent_a_penalty() {
    for with_canon in [false, true] {
        let (mut world, pilot, victim) = boot(with_canon);
        run(&mut world, GOD, "@faction/create Surveyors");
        let fid = faction(&world, "Surveyors");
        run(
            &mut world,
            GOD,
            &format!(
                "@faction/incident unknown:1=Surveyors/#{}/unknown/Unattributed attack",
                victim.0
            ),
        );
        world.take_module_ops();
        let output = run(&mut world, GOD, "@faction/publish-incident unknown:1");
        assert_eq!(score(&world, fid, pilot), 0);
        if with_canon {
            assert!(output.contains("Incident published"));
            assert!(
                run(&mut world, GOD, "@faction/publish-incident unknown:1")
                    .contains("already published")
            );
        } else {
            assert!(output.contains("report remains private"));
            assert!(world.take_module_ops().is_empty());
        }
    }
}

#[test]
fn hidden_factions_and_private_incident_details_are_not_public() {
    let (mut world, pilot, victim) = boot(true);
    run(&mut world, GOD, "@faction/create Quiet Order");
    let fid = faction(&world, "Quiet Order");
    run(&mut world, GOD, "@faction/set Quiet Order=hidden:yes");
    run(
        &mut world,
        GOD,
        &format!(
            "test-grant secret:work={}/{}:30/Do not disclose",
            pilot.0, fid.0
        ),
    );
    assert!(public_standing(world.world(), fid, pilot).is_none());
    assert_eq!(
        run(&mut world, pilot, "+standing Quiet Order"),
        run(&mut world, pilot, "+standing Missing Order")
    );
    run(
        &mut world,
        GOD,
        &format!(
            "@faction/incident hidden:1=Quiet Order/#{}/#{}/Secret victim",
            victim.0, pilot.0
        ),
    );
    assert!(run(&mut world, GOD, "@faction/publish-incident hidden:1").contains("hidden faction"));
    assert_eq!(
        world
            .world()
            .module_state::<CanonState>()
            .expect("canon")
            .next_canon_id,
        0
    );
}

#[test]
fn organization_resolver_reuses_valid_links_and_refuses_name_conflicts() {
    let (mut world, _, _) = boot(true);
    assert!(run(&mut world, GOD, "@org/create Occator Research").contains("Occator Research"));
    assert!(run(&mut world, GOD, "test-ensure 0").contains("Faction 0"));
    assert_eq!(faction(&world, "Occator Research"), FactionId(0));
    world.take_module_ops();
    assert!(run(&mut world, GOD, "test-ensure 0").contains("Faction 0"));
    assert!(world.take_module_ops().is_empty());
    run(&mut world, GOD, "@org/create Rival Office");
    run(&mut world, GOD, "@faction/create Rival Office");
    world.take_module_ops();
    assert!(run(&mut world, GOD, "test-ensure 1").contains("conflicts"));
    assert!(world.take_module_ops().is_empty());
}

#[test]
fn corrupt_receipt_prevents_boot_without_orphan_purging_deduplication() {
    let (mut world, pilot, _) = boot(true);
    run(&mut world, GOD, "@faction/create Surveyors");
    let fid = faction(&world, "Surveyors");
    run(
        &mut world,
        GOD,
        &format!("test-grant work:1={}/{}:10/Completed work", pilot.0, fid.0),
    );
    let mut saved = HashMap::new();
    fold_module_ops_into(&mut saved, world.take_module_ops());
    let entry = saved
        .get_mut("faction")
        .expect("faction docs")
        .kv
        .iter_mut()
        .find(|(key, _)| key.as_str() == "standing:grant:work:1")
        .expect("receipt");
    entry.1["effects"] = serde_json::json!([]);
    let (mut world, _, _) = boot(true);
    let result = world
        .engine_mut()
        .modules_loaded(saved, &mut Outbox::new(), &mut Effects::new());
    assert!(
        result.is_err(),
        "corrupt immutable receipt must not become a fresh cause"
    );
    assert!(
        world
            .take_module_ops()
            .iter()
            .all(|op| !matches!(op.op, mush_core::ModuleWrite::Del { .. }))
    );
}
