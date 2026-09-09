//! End-to-end tests for the SR5 ruleset driven through the real
//! [`Engine`](mush_core) via [`mush_testkit::TestWorld`], with the jobs module
//! loaded so chargen / Karma approval flows run.
//!
//! Dice are deterministic: the ruleset is built with a fixed RNG seed and the
//! test replays the same `ChaCha8Rng` stream to predict every roll.

use mush_core::{GOD, ObjId, ObjKind};
use mush_dice::sr5::{Sr5Opts, Sr5Outcome, Sr5Pool, Sr5Result};
use mush_jobs::{JobStatus, JobsModule, get_job};
use mush_ruleset::{ApprovalState, SheetDoc};
use mush_testkit::TestWorld;
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use ruleset_shadowrun::ShadowrunRuleset;
use ruleset_shadowrun::state::Sr5State;
use ruleset_shadowrun::{derive, schema};

/// Fixed RNG seed for deterministic dice across the suite.
const S: u64 = 777;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

fn world(seed: u64) -> (TestWorld, ObjId) {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ShadowrunRuleset::with_seed(seed)),
    ]);
    let nick = tw.spawn_player("Nick");
    (tw, nick)
}

/// The d6 faces the seeded RNG yields (matching `Sr5Pool::roll`'s
/// `next_u32() % 6 + 1`), skipping `skip` earlier draws.
fn faces(seed: u64, skip: usize, n: usize) -> Vec<u8> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    for _ in 0..skip {
        let _ = rng.next_u32();
    }
    (0..n).map(|_| (rng.next_u32() % 6) as u8 + 1).collect()
}

/// Reproduce the Rule-of-Six exploding sequence (`roll_exploding`): `n` plain
/// dice, each `6` (original or exploded) spawning one more, bounded by the
/// default `max_dice` (100).
fn faces_exploding(seed: u64, n: usize) -> Vec<u8> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut roll = || (rng.next_u32() % 6) as u8 + 1;
    let budget = 100usize;
    let mut faces: Vec<u8> = (0..n).map(|_| roll()).collect();
    let mut i = 0;
    while i < faces.len() {
        if faces[i] == 6 && faces.len() < budget {
            faces.push(roll());
        }
        i += 1;
    }
    faces
}

fn last_outcome(tw: &TestWorld, who: ObjId) -> Sr5Outcome {
    tw.world()
        .module_state::<Sr5State>()
        .expect("Sr5State")
        .recent_logs(who, 1)
        .first()
        .expect("a logged roll")
        .outcome
        .clone()
}

fn recent(tw: &TestWorld, who: ObjId, n: usize) -> Vec<Sr5Outcome> {
    tw.world()
        .module_state::<Sr5State>()
        .expect("Sr5State")
        .recent_logs(who, n)
        .iter()
        .map(|e| e.outcome.clone())
        .collect()
}

fn read_sheet(tw: &TestWorld, who: ObjId) -> SheetDoc {
    let attr = tw
        .world()
        .attr(who, "sheet:shadowrun")
        .and_then(mush_core::Attr::as_json)
        .expect("a sheet");
    serde_json::from_value(attr.clone()).expect("sheet decodes")
}

/// The number of dice shown on a one-line roll narration (interface plan
/// I3): `roll  Nick  Agility+Pistols  5 6 3 1 = 2 hits  [Physical limit 6]`.
fn faces_shown(line: &str) -> Option<usize> {
    let (left, _) = line.split_once(" = ")?;
    let dice = left.rsplit("  ").next()?;
    Some(
        dice.split_whitespace()
            .filter(|t| t.parse::<u8>().is_ok())
            .count(),
    )
}

/// The one-line roll narrations among `lines`, by their `roll  ` marker.
fn roll_lines(lines: &[String]) -> Vec<&String> {
    lines.iter().filter(|l| l.starts_with("roll  ")).collect()
}

fn set(tw: &mut TestWorld, cmd: &str) {
    tw.run(GOD, cmd);
}

/// Give Nick a basic street-samurai body via staff edits (each recomputes
/// derived stats). Agility 6, Pistols 4, Body/Reaction/Strength for the limit.
fn samurai(tw: &mut TestWorld) {
    set(tw, "@sheet/set Nick/metatype=human");
    set(tw, "@sheet/set Nick/body=5");
    set(tw, "@sheet/set Nick/agility=6");
    set(tw, "@sheet/set Nick/reaction=5");
    set(tw, "@sheet/set Nick/strength=4");
    set(tw, "@sheet/set Nick/willpower=4");
    set(tw, "@sheet/set Nick/active_skills:pistols=4");
}

// ----------------------------------------------------------------------
// Schema / sheet
// ----------------------------------------------------------------------

#[test]
fn sheet_renders_with_the_framework_renderer() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    let ev = tw.run(GOD, "@sheet Nick");
    let seen = tw.seen_by(&ev, GOD);
    assert!(seen.iter().any(|l| l.contains("SHADOWRUN SHEET")));
    assert!(seen.iter().any(|l| l.to_lowercase().contains("body")));
    // Staff-set edits recomputed the Physical CM (Body 5 → 8+ceil(5/2)=11).
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.number(schema::PHYS_CM_MAX), Some(11));
    assert_eq!(doc.track(schema::CONDITION_PHYSICAL).unwrap().len(), 11);
}

// ----------------------------------------------------------------------
// +roll
// ----------------------------------------------------------------------

#[test]
fn roll_resolves_attr_plus_skill_and_applies_the_limit() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    // Pool = Agility 6 + Pistols 4 = 10; Physical Limit ceil((STR8+BOD5+REA5)/3)
    // = ceil(18/3) = 6.
    let doc = read_sheet(&tw, nick);
    let phys = derive::limits(&doc).physical as u8;
    assert_eq!(phys, 6);
    let ev = tw.run(nick, "+roll agility+pistols");
    let expected = Sr5Pool::score(&faces(S, 0, 10), &Sr5Opts::new(1).with_limit(phys));
    assert_eq!(last_outcome(&tw, nick), expected);
    assert_eq!(last_outcome(&tw, nick).dice.len(), 10);
    // The room saw the one-line narration with ten dice and the limit.
    let seen = tw.seen_by(&ev, nick);
    let line = roll_lines(&seen).pop().expect("a roll line");
    assert_eq!(faces_shown(line), Some(10), "{line}");
    assert!(line.contains("  [Physical limit 6]"), "{line}");
    // Determinism: a fresh identical world reproduces it.
    let (mut tw2, nick2) = world(S);
    samurai(&mut tw2);
    tw2.run(nick2, "+roll agility+pistols");
    assert_eq!(last_outcome(&tw2, nick2), expected);
}

#[test]
fn roll_raw_integer_needs_no_sheet() {
    let (mut tw, nick) = world(S);
    let ev = tw.run(nick, "+roll 5");
    let expected = Sr5Pool::score(&faces(S, 0, 5), &Sr5Opts::default());
    assert_eq!(last_outcome(&tw, nick), expected);
    let seen = tw.seen_by(&ev, nick);
    assert_eq!(
        roll_lines(&seen).pop().and_then(|l| faces_shown(l)),
        Some(5)
    );
}

#[test]
fn roll_applies_the_wound_modifier() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    // 3 physical boxes → wound −1; the pool loses one die (10 → 9).
    let mut doc = read_sheet(&tw, nick);
    ruleset_shadowrun::condition::apply(
        &mut doc,
        ruleset_shadowrun::condition::Damage::Physical,
        3,
    );
    tw.world_mut()
        .set_attr(
            nick,
            "sheet:shadowrun",
            mush_core::Attr::json(serde_json::to_value(&doc).unwrap(), nick),
        )
        .unwrap();
    let phys = derive::limits(&doc).physical as u8;
    let ev = tw.run(nick, "+roll agility+pistols");
    let expected = Sr5Pool::score(&faces(S, 0, 9), &Sr5Opts::new(1).with_limit(phys));
    assert_eq!(last_outcome(&tw, nick), expected);
    let seen = tw.seen_by(&ev, nick);
    let line = roll_lines(&seen).pop().expect("a roll line");
    assert_eq!(faces_shown(line), Some(9), "{line}");
    assert!(line.contains("[wound -1]"), "{line}");
    // /nowound ignores it: back to 10 dice.
    let ev2 = tw.run(nick, "+roll/nowound agility+pistols");
    let seen2 = tw.seen_by(&ev2, nick);
    assert_eq!(
        roll_lines(&seen2).pop().and_then(|l| faces_shown(l)),
        Some(10)
    );
}

#[test]
fn threshold_switch_sets_difficulty() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    tw.run(nick, "+roll agility+pistols threshold 3");
    // The logged outcome scored against threshold 3.
    let doc = read_sheet(&tw, nick);
    let phys = derive::limits(&doc).physical as u8;
    let expected = Sr5Pool::score(&faces(S, 0, 10), &Sr5Opts::new(3).with_limit(phys));
    assert_eq!(last_outcome(&tw, nick), expected);
}

#[test]
fn edge_pushes_the_limit_waives_it_and_explodes_sixes() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    set(&mut tw, "@sheet/set Nick/edge=3");
    // edge_current defaults to 1 — enough for one push.
    let ev = tw.run(nick, "+roll/edge agility+pistols");
    let out = last_outcome(&tw, nick);
    // The Limit was waived.
    assert_eq!(out.limit_applied, None);
    // Edge dice added: base pool 10 + Edge 3 = 13 dice, then Rule-of-Six.
    let exp_faces = faces_exploding(S, 13);
    assert_eq!(out.dice.len(), exp_faces.len());
    assert_eq!(
        out.dice.iter().map(|d| d.face).collect::<Vec<_>>(),
        exp_faces
    );
    // With no limit, hits == raw hits (faces >= 5).
    let raw = exp_faces.iter().filter(|&&f| f >= 5).count() as u32;
    assert_eq!(out.hits(), raw);
    // Any 6 in the base pool spawned an exploded die.
    if exp_faces.len() > 13 {
        assert!(out.dice.iter().any(|d| d.exploded_from.is_some()));
    }
    // The Edge point was spent.
    assert_eq!(read_sheet(&tw, nick).number(schema::EDGE_CURRENT), Some(0));
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("pushed the limit"))
    );
    // No Edge left → a second push is refused.
    let ev2 = tw.run(nick, "+roll/edge agility+pistols");
    assert!(
        tw.seen_by(&ev2, nick)
            .iter()
            .any(|l| l.contains("no Edge left"))
    );
}

#[test]
fn glitch_and_critical_glitch_surface_through_roll() {
    // Find a seed whose first 4 raw dice glitch with hits, and one that
    // critically glitches (glitch + zero hits).
    let glitch_seed = find_seed(4, |o| matches!(o.result, Sr5Result::Glitch(_)));
    let crit_seed = find_seed(4, |o| matches!(o.result, Sr5Result::CriticalGlitch));

    let (mut tw, nick) = world(glitch_seed);
    let ev = tw.run(nick, "+roll 4");
    assert!(matches!(
        last_outcome(&tw, nick).result,
        Sr5Result::Glitch(_)
    ));
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("glitch")));

    let (mut tw, nick) = world(crit_seed);
    let ev = tw.run(nick, "+roll 4");
    assert!(matches!(
        last_outcome(&tw, nick).result,
        Sr5Result::CriticalGlitch
    ));
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("critical glitch"))
    );
}

/// Find a seed whose first `n` raw d6 satisfy `pred` under default options.
fn find_seed(n: usize, pred: impl Fn(&Sr5Outcome) -> bool) -> u64 {
    for seed in 0u64..2_000_000 {
        let out = Sr5Pool::score(&faces(seed, 0, n), &Sr5Opts::default());
        if pred(&out) {
            return seed;
        }
    }
    panic!("no seed found");
}

#[test]
fn contested_roll_resolves_both_sides() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    let _vlad = tw.spawn_player("Vlad");
    set(&mut tw, "@sheet/set Vlad/reaction=4");
    set(&mut tw, "@sheet/set Vlad/intuition=3");
    let ev = tw.run(
        nick,
        "+roll agility+pistols contested Vlad=reaction+intuition",
    );
    assert!(tw.seen_by(&ev, nick).iter().any(|l| {
        l.starts_with("roll  Nick  ")
            && l.contains("  vs  Vlad  ")
            && l.contains("  => ")
            && (l.contains("wins") || l.contains("tie"))
    }));
}

#[test]
fn extended_test_pool_shrinks_each_interval() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/logic=5");
    set(&mut tw, "@sheet/set Nick/active_skills:computer=5"); // pool = 10
    // SR5 extended tests lose one die from the pool each interval.
    tw.run(nick, "+roll logic+computer extended 20");
    let first = last_outcome(&tw, nick).dice.len();
    tw.run(nick, "+roll logic+computer extended 20");
    let second = last_outcome(&tw, nick).dice.len();
    assert_eq!(first, 10, "first interval rolls the full pool");
    assert_eq!(
        second,
        first - 1,
        "the pool must shrink by one die each interval (was {first}, then {second})"
    );
}

#[test]
fn contested_edge_is_spent_once_and_never_on_a_failed_roll() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    set(&mut tw, "@sheet/set Nick/edge=3");
    let _vlad = tw.spawn_player("Vlad");
    set(&mut tw, "@sheet/set Vlad/reaction=4");
    set(&mut tw, "@sheet/set Vlad/intuition=3");
    // A contested roll whose target pool cannot resolve fails; the actor's Edge
    // must NOT be spent (edge_current defaults to 1).
    tw.run(
        nick,
        "+roll/edge agility+pistols contested Vlad=bogusattr+bogusskill",
    );
    assert_eq!(
        read_sheet(&tw, nick).number(schema::EDGE_CURRENT),
        Some(1),
        "Edge must survive a contested roll that fails to resolve"
    );
    // A valid contested /edge spends exactly one Edge.
    tw.run(
        nick,
        "+roll/edge agility+pistols contested Vlad=reaction+intuition",
    );
    assert_eq!(
        read_sheet(&tw, nick).number(schema::EDGE_CURRENT),
        Some(0),
        "a successful contested /edge spends exactly one Edge"
    );
}

/// The contested-roll oracle. `+roll ... contested <target>=<pool>` resolved
/// any matched target's sheet with no gate at all — the rest of the crate
/// checks `perm::can_examine` before it reads another character (`+sheet`,
/// `+karma`, `+condition`) — so the `#dbref` and `*name` spellings walked the
/// database for who exists and where they stand, and the pool spelling then
/// read them one rating at a time off the distinct per-term refusal. All of
/// it now answers with one sentence, and none of it costs Edge.
#[test]
fn contested_refusals_are_one_sentence_and_spend_no_edge() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    // Vlad has a sheet that would resolve — and stands in another room.
    let vlad = tw.spawn_player("Vlad");
    set(&mut tw, "@sheet/set Vlad/reaction=4");
    set(&mut tw, "@sheet/set Vlad/intuition=3");
    let vault = tw
        .world_mut()
        .create(ObjKind::Room, "Vault", GOD, None)
        .expect("a room off-stage");
    tw.world_mut()
        .move_to(vlad, vault)
        .expect("Vlad walks off-stage");
    // Wisp is in the room with Nick, and has no Alchemy (which never defaults).
    let _wisp = tw.spawn_player("Wisp");
    set(&mut tw, "@sheet/set Wisp/agility=3");

    let edge_before = read_sheet(&tw, nick).number(schema::EDGE_CURRENT);
    let lines = [
        // Out of reach, by the dbref spelling that resolves globally.
        format!(
            "+roll/edge agility+pistols contested #{}=reaction+intuition",
            vlad.0
        ),
        // Out of reach, by the `*name` spelling, which also resolves globally.
        "+roll/edge agility+pistols contested *Vlad=reaction+intuition".to_string(),
        // A dbref that names nothing at all.
        "+roll/edge agility+pistols contested #99999=reaction+intuition".to_string(),
        // In the room, but with no rating in the named non-defaultable skill.
        "+roll/edge agility+pistols contested Wisp=agility+alchemy".to_string(),
        // In the room, and the named term is not a trait on any sheet.
        "+roll/edge agility+pistols contested Wisp=bogusattr+bogusskill".to_string(),
    ];
    let refusals: Vec<Vec<String>> = lines
        .iter()
        .map(|line| {
            let ev = tw.run(nick, line);
            tw.seen_by(&ev, nick)
        })
        .collect();
    assert!(
        refusals[0]
            .iter()
            .any(|l| l.contains("You can't contest a roll against that target.")),
        "{:?}",
        refusals[0]
    );
    for (i, seen) in refusals.iter().enumerate() {
        assert_eq!(
            seen, &refusals[0],
            "refusal {i} ({}) must be byte-identical to the first",
            lines[i]
        );
        assert!(
            roll_lines(seen).is_empty(),
            "a refused contest rolls nothing: {seen:?}"
        );
    }
    assert_eq!(
        read_sheet(&tw, nick).number(schema::EDGE_CURRENT),
        edge_before,
        "a refused contest must not spend Edge"
    );

    // The gate is locality-or-authority, not a ban: an in-room target still
    // resolves and the room still sees both pools.
    set(&mut tw, "@sheet/set Wisp/reaction=4");
    set(&mut tw, "@sheet/set Wisp/intuition=3");
    let ev = tw.run(
        nick,
        "+roll agility+pistols contested Wisp=reaction+intuition",
    );
    assert!(
        tw.seen_by(&ev, nick).iter().any(|l| {
            l.starts_with("roll  Nick  ")
                && l.contains("  vs  Wisp  ")
                && l.contains("  => ")
                && (l.contains("wins") || l.contains("tie"))
        }),
        "{:?}",
        tw.seen_by(&ev, nick)
    );
    // And staff holding EXAMINE_ALL still reach the off-stage NPC by dbref.
    let ev = tw.run(
        GOD,
        &format!("+roll 4 contested #{}=reaction+intuition", vlad.0),
    );
    assert!(
        tw.seen_by(&ev, GOD)
            .iter()
            .any(|l| l.starts_with("roll  ") && l.contains("  vs  Vlad  ")),
        "{:?}",
        tw.seen_by(&ev, GOD)
    );
}

#[test]
fn roll_last_shows_history_for_staff() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    tw.run(nick, "+roll agility+pistols");
    let ev = tw.run(GOD, "+roll/last Nick");
    assert!(
        tw.seen_by(&ev, GOD)
            .iter()
            .any(|l| l.contains("Recent rolls"))
    );
}

// ----------------------------------------------------------------------
// Condition monitors + damage
// ----------------------------------------------------------------------

#[test]
fn legacy_damage_and_heal_aliases_refuse_without_mutation() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    let before = read_sheet(&tw, nick);
    let damage = tw.run(GOD, "+damage Nick=12S");
    assert!(
        tw.seen_by(&damage, GOD)
            .iter()
            .any(|line| line.contains("+act condition-damage on #"))
    );
    let heal = tw.run(GOD, "+heal Nick=5S");
    assert!(
        tw.seen_by(&heal, GOD)
            .iter()
            .any(|line| line.contains("+act condition-heal on #"))
    );
    assert_eq!(read_sheet(&tw, nick), before);
}

#[test]
fn legacy_physical_damage_alias_names_the_authoritative_parameters() {
    let (mut tw, nick) = world(S);
    samurai(&mut tw);
    let ev = tw.run(GOD, "+damage Nick=14P");
    assert!(
        tw.seen_by(&ev, GOD)
            .iter()
            .any(|line| line.contains(r#"{"amount":14,"kind":"physical"}"#))
    );
    assert_eq!(
        read_sheet(&tw, nick).number(schema::PHYSICAL_OVERFLOW),
        Some(0)
    );
}

// ----------------------------------------------------------------------
// Initiative
// ----------------------------------------------------------------------

#[test]
fn initiative_rolls_reaction_plus_intuition_plus_dice() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/reaction=5");
    set(&mut tw, "@sheet/set Nick/intuition=4");
    // Base = 9, 1 initiative die.
    let ev = tw.run(nick, "+init/roll");
    let die = faces(S, 0, 1)[0] as i64;
    let score = 9 + die;
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains(&format!("= {score}")))
    );
}

// ----------------------------------------------------------------------
// Magic: +cast rolls the spellcasting pool + a drain roll
// ----------------------------------------------------------------------

#[test]
fn cast_rolls_spellcasting_then_drain_and_applies_it() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/awakened_type=magician");
    set(&mut tw, "@sheet/set Nick/tradition=hermetic");
    set(&mut tw, "@sheet/set Nick/magic=5");
    set(&mut tw, "@sheet/set Nick/willpower=5");
    set(&mut tw, "@sheet/set Nick/logic=4");
    set(&mut tw, "@sheet/set Nick/active_skills:spellcasting=4");
    tw.run(nick, "+spell Manabolt");

    tw.run(nick, "+cast Manabolt force 3");

    // Spellcasting: Magic 5 + Spellcasting 4 = 9 dice, threshold 2, Limit 3.
    let cast_expected = Sr5Pool::score(&faces(S, 0, 9), &Sr5Opts::new(2).with_limit(3));
    // Drain: DV = max(2, ceil(3/2)) = 2; pool Willpower 5 + Logic 4 = 9 dice.
    let drain_expected = Sr5Pool::score(&faces(S, 9, 9), &Sr5Opts::new(0));
    let logs = recent(&tw, nick, 2);
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0], cast_expected); // cast logged first
    assert_eq!(logs[1], drain_expected); // drain logged second

    // Drain taken = max(DV 2 − drain hits, 0), as Stun (Force 3 <= Magic 5).
    let drain_taken = (2i64 - i64::from(drain_expected.hits())).max(0);
    let doc = read_sheet(&tw, nick);
    let stun_filled = doc
        .track(schema::CONDITION_STUN)
        .unwrap()
        .iter()
        .filter(|b| b.is_some())
        .count() as i64;
    assert_eq!(stun_filled, drain_taken);
}

#[test]
fn cast_refuses_unknown_spell_and_mundane() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/body=3"); // a mundane sheet exists
    let ev = tw.run(nick, "+cast Fireball");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("not a spellcaster"))
    );
    set(&mut tw, "@sheet/set Nick/awakened_type=magician");
    set(&mut tw, "@sheet/set Nick/magic=4");
    let ev = tw.run(nick, "+cast Fireball");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("don't know"))
    );
}

// ----------------------------------------------------------------------
// Chargen: the priority system → submit → approve → Approved
// ----------------------------------------------------------------------

/// Build a fully balanced human street samurai through the command surface.
fn full_chargen(tw: &mut TestWorld, who: ObjId) {
    tw.run(who, "+chargen");
    tw.run(
        who,
        "+chargen/priority metatype=C attributes=A magic=E skills=B resources=D",
    );
    tw.run(who, "+stat metatype=human");
    // Attributes A = 24: 5+4+4+3+2+2+2+2 = 24 (Body at max 6).
    for c in [
        "body=6",
        "agility=5",
        "reaction=5",
        "strength=4",
        "charisma=3",
        "intuition=3",
        "logic=3",
        "willpower=3",
    ] {
        tw.run(who, &format!("+stat {c}"));
    }
    // Special C/human = 5: Edge 2→7.
    tw.run(who, "+stat edge=7");
    // Active skills B = 36: six skills at 6.
    for s in [
        "pistols",
        "automatics",
        "longarms",
        "blades",
        "sneaking",
        "perception",
    ] {
        tw.run(who, &format!("+stat active_skills:{s}=6"));
    }
    // Skill groups B = 5.
    tw.run(who, "+stat skill_groups:influence=5");
    // Resources D = 50000¥.
    tw.run(who, "+stat nuyen=50000");
}

#[test]
fn chargen_priority_full_build_submit_and_approve() {
    let (mut tw, nick) = world(S);
    full_chargen(&mut tw, nick);

    // Budget balances.
    let ev = tw.run(nick, "+chargen/status");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Budget balances"))
    );

    // Submit → Pending + a shadowrun-chargen job.
    let ev = tw.run(nick, "+chargen/submit");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Submitted for approval"))
    );
    let doc = read_sheet(&tw, nick);
    let job_id = match doc.approved {
        ApprovalState::Pending(j) => j,
        other => panic!("expected Pending, got {other:?}"),
    };
    // Finalize derived: Physical CM 8+ceil(6/2)=11; Initiative REA5+INT3=8;
    // Physical Limit ceil((STR8+BOD6+REA5)/3)=ceil(19/3)=7; Edge refreshed.
    assert_eq!(doc.number(schema::PHYS_CM_MAX), Some(11));
    assert_eq!(doc.number(schema::INITIATIVE_BASE), Some(8));
    assert_eq!(doc.number(schema::PHYS_LIMIT), Some(7));
    assert_eq!(doc.number(schema::EDGE_CURRENT), Some(7));

    // Staff approve → handler stamps Approved.
    let ev = tw.run(
        GOD,
        &format!("+job/approve {job_id}=Welcome to the shadows."),
    );
    assert!(tw.seen_by(&ev, GOD).iter().any(|l| l.contains("approved")));
    assert!(read_sheet(&tw, nick).approved.is_approved());
    assert_eq!(
        get_job(tw.world(), job_id).unwrap().status,
        JobStatus::Resolved
    );
}

#[test]
fn wizard_submit_rejects_malformed_finalized_sheet_without_opening_a_job() {
    let (mut tw, nick) = world(S);
    full_chargen(&mut tw, nick);
    tw.run(nick, "+chargen/wizard");
    tw.run(nick, "review");

    let mut doc = read_sheet(&tw, nick);
    doc.set("unknown_injected_field", serde_json::json!(true));
    tw.world_mut()
        .set_attr(
            nick,
            "sheet:shadowrun",
            mush_core::Attr::json(serde_json::to_value(doc).unwrap(), nick),
        )
        .unwrap();
    let _ = tw.take_module_ops();

    let events = tw.run(nick, "submit");
    let seen = tw.seen_by(&events, nick);
    assert!(
        seen.iter().any(|line| line.contains("unknown field")),
        "malformed sheet refusal missing: {seen:?}"
    );
    assert!(
        get_job(tw.world(), 1).is_none(),
        "submit opened an orphan job"
    );
    assert!(
        tw.take_module_ops().is_empty(),
        "failed wizard submit queued partial job persistence"
    );
    assert_eq!(tw.world().input_depth(nick), 1, "wizard should remain open");
}

#[test]
fn chargen_flags_overspend_and_maxima() {
    let (mut tw, nick) = world(S);
    tw.run(nick, "+chargen");
    tw.run(
        nick,
        "+chargen/priority metatype=A attributes=E magic=B skills=C resources=D",
    );
    tw.run(nick, "+stat metatype=human");
    // Attributes E = 12, but spend way over (5 attrs at 6 = 25 above min).
    for a in ["body", "agility", "reaction", "strength", "charisma"] {
        tw.run(nick, &format!("+stat {a}=6"));
    }
    let ev = tw.run(nick, "+chargen/submit");
    let seen = tw.seen_by(&ev, nick);
    assert!(seen.iter().any(|l| l.contains("Cannot submit")));
}

#[test]
fn chargen_priority_must_be_a_permutation() {
    let (mut tw, nick) = world(S);
    tw.run(nick, "+chargen");
    let ev = tw.run(
        nick,
        "+chargen/priority metatype=A attributes=A magic=C skills=D resources=E",
    );
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("exactly once"))
    );
}

#[test]
fn magician_chargen_needs_a_magic_priority() {
    let (mut tw, nick) = world(S);
    tw.run(nick, "+chargen");
    tw.run(
        nick,
        "+chargen/priority metatype=C attributes=A magic=E skills=B resources=D",
    );
    tw.run(nick, "+stat awakened_type=magician");
    let ev = tw.run(nick, "+chargen/submit");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Cannot submit"))
    );
}

#[test]
fn stat_requires_open_chargen() {
    let (mut tw, nick) = world(S);
    let ev = tw.run(nick, "+stat body=5");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("not in chargen"))
    );
}

// ----------------------------------------------------------------------
// Karma (the XP analog)
// ----------------------------------------------------------------------

#[test]
fn karma_award_spend_and_apply() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/active_skills:pistols=4");
    // Award Karma (staff, immediate).
    let ev = tw.run(GOD, "+karma/award Nick=20");
    assert!(
        tw.seen_by(&ev, GOD)
            .iter()
            .any(|l| l.contains("Awarded 20 Karma"))
    );
    // Spend to raise Pistols 4 → 5: cost = 5 × 2 = 10.
    let ev = tw.run(nick, "+karma/spend active_skills:pistols=5");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("for 10 Karma"))
    );
    let job_id = get_job(tw.world(), 1).map(|j| j.id).unwrap();
    tw.run(GOD, &format!("+job/approve {job_id}"));
    let doc = read_sheet(&tw, nick);
    assert_eq!(
        doc.map(schema::ACTIVE_SKILLS_FIELD).unwrap()["pistols"],
        serde_json::json!(5)
    );
    assert_eq!(doc.number("karma"), Some(10)); // 20 − 10
    assert!(
        doc.list("karma_log")
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap_or("").contains("pistols"))
    );
}

#[test]
fn karma_spend_refused_when_unaffordable() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/body=3");
    tw.run(GOD, "+karma/award Nick=5");
    // Raising Body 3 → 5 costs 4×5 + 5×5 = 45 > 5.
    let ev = tw.run(nick, "+karma/spend body=5");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("you have 5"))
    );
}

// ----------------------------------------------------------------------
// Agent grounding
// ----------------------------------------------------------------------

#[test]
fn agent_profile_and_sr_roll_read_own_sheet() {
    use mush_ruleset::Ruleset;
    let rs = ShadowrunRuleset::with_seed(S);
    let mut doc = SheetDoc::new_default(&rs.schema());
    doc.set_text("metatype", "ork");
    doc.set_number("agility", 5);
    doc.map_set(schema::ACTIVE_SKILLS_FIELD, "pistols", serde_json::json!(5));
    let prof = rs.agent_profile(&doc);
    assert!(prof.summary.contains("ork"));
    assert!(prof.tags.iter().any(|t| t.starts_with("combat:")));
    assert!(prof.tools.iter().any(|t| t.name == "sr_roll"));
}
