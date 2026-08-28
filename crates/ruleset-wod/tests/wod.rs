//! End-to-end tests for the oWoD20 base ruleset driven through the real
//! [`Engine`](mush_core) via [`mush_testkit::TestWorld`], with the jobs module
//! loaded so chargen/XP approval flows run.
//!
//! Dice are deterministic: the ruleset is built with a fixed RNG seed and the
//! test replays the same `ChaCha8Rng` stream to predict every roll.

use mush_core::{GOD, ObjId};
use mush_dice::owod20::{Owod20Opts, Owod20Outcome, Owod20Pool};
use mush_jobs::{JobStatus, JobsModule, get_job};
use mush_ruleset::schema::{FieldDef, TrackSize};
use mush_ruleset::schema::{
    FieldType, SchemaError, SectionLayout, SheetSchemaBuilder, SheetSection,
};
use mush_ruleset::{ApprovalState, Ruleset, SheetDoc};
use mush_testkit::TestWorld;
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use ruleset_wod::WodRuleset;
use ruleset_wod::config::WodConfig;
use ruleset_wod::schema::{Mortal, Template, build_schema};
use ruleset_wod::state::WodState;

/// Fixed RNG seed for deterministic dice across the suite.
const S: u64 = 12345;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

fn world(seed: u64) -> (TestWorld, ObjId) {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(seed)),
    ]);
    let nick = tw.spawn_player("Nick");
    (tw, nick)
}

/// The faces the seeded RNG yields for the next `n` dice (matching
/// `Owod20Pool::roll`).
fn faces(seed: u64, skip: usize, n: usize) -> Vec<u8> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    for _ in 0..skip {
        let _ = rng.next_u32();
    }
    (0..n).map(|_| (rng.next_u32() % 10) as u8 + 1).collect()
}

/// The most recent logged outcome for a character.
fn last_outcome(tw: &TestWorld, who: ObjId) -> Owod20Outcome {
    tw.world()
        .module_state::<WodState>()
        .expect("WodState")
        .recent_logs(who, 1)
        .first()
        .expect("a logged roll")
        .outcome
        .clone()
}

fn set(tw: &mut TestWorld, cmd: &str) {
    tw.run(GOD, cmd);
}

fn nick_sheet(tw: &mut TestWorld) {
    set(tw, "@sheet/set Nick/strength=3");
    set(tw, "@sheet/set Nick/brawl=2");
}

// ----------------------------------------------------------------------
// Schema
// ----------------------------------------------------------------------

#[test]
fn base_schema_validates() {
    let schema = build_schema("wod", &[Box::new(Mortal) as Box<dyn Template>]);
    assert!(mush_ruleset::validate(&schema, &schema.default_data()).is_ok());
    // The nine attributes, the ability groups, health, willpower are present.
    for id in [
        "strength",
        "wits",
        "brawl",
        "firearms",
        "occult",
        "willpower",
        "courage",
    ] {
        assert!(schema.field(id).is_some(), "missing field {id}");
    }
    assert!(schema.field("template").is_some());
}

// ----------------------------------------------------------------------
// +roll
// ----------------------------------------------------------------------

#[test]
fn roll_resolves_pool_from_sheet_and_is_deterministic() {
    let (mut tw, nick) = world(S);
    nick_sheet(&mut tw);
    // Pool = strength 3 + brawl 2 = 5, difficulty 6.
    let ev = tw.run(nick, "+roll strength+brawl");
    let expected = Owod20Pool::score(&faces(S, 0, 5), &Owod20Opts::new(6));
    assert_eq!(last_outcome(&tw, nick), expected);
    // The room saw the classic line with the dice count.
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("(5 dice, diff 6)"))
    );
    // A second identical world reproduces the same faces.
    let (mut tw2, nick2) = world(S);
    nick_sheet(&mut tw2);
    tw2.run(nick2, "+roll strength+brawl");
    assert_eq!(last_outcome(&tw2, nick2), expected);
}

#[test]
fn roll_raw_integer_needs_no_sheet() {
    let (mut tw, nick) = world(S);
    let ev = tw.run(nick, "+roll 4");
    let expected = Owod20Pool::score(&faces(S, 0, 4), &Owod20Opts::new(6));
    assert_eq!(last_outcome(&tw, nick), expected);
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("(4 dice")));
}

#[test]
fn roll_applies_wound_penalty() {
    let (mut tw, nick) = world(S);
    nick_sheet(&mut tw);
    // 4 lethal → Wounded (-2). Pool 5 - 2 = 3.
    set(&mut tw, "@sheet/set Nick/strength=3");
    tw.run(GOD, "+harm Nick=4L");
    let ev = tw.run(nick, "+roll strength+brawl");
    let expected = Owod20Pool::score(&faces(S, 0, 3), &Owod20Opts::new(6));
    assert_eq!(last_outcome(&tw, nick), expected);
    let seen = tw.seen_by(&ev, nick);
    assert!(seen.iter().any(|l| l.contains("(3 dice, diff 6)")));
    assert!(seen.iter().any(|l| l.contains("wound -2")));
    // /nowound ignores it: pool back to 5 (next 5 faces after the first 3).
    let ev2 = tw.run(nick, "+roll/nowound strength+brawl");
    assert!(tw.seen_by(&ev2, nick).iter().any(|l| l.contains("(5 dice")));
}

#[test]
fn difficulty_switch_and_vs() {
    let (mut tw, nick) = world(S);
    nick_sheet(&mut tw);
    tw.run(nick, "+roll strength+brawl vs 8");
    assert_eq!(last_outcome(&tw, nick).difficulty, 8);
}

#[test]
fn willpower_spends_and_prevents_botch() {
    let (mut tw, nick) = world(S);
    nick_sheet(&mut tw);
    set(&mut tw, "@sheet/set Nick/willpower_temp=1");
    tw.run(nick, "+roll/wp strength+brawl");
    let expected = Owod20Pool::score(
        &faces(S, 0, 5),
        &Owod20Opts {
            willpower: true,
            ..Owod20Opts::new(6)
        },
    );
    let out = last_outcome(&tw, nick);
    assert_eq!(out, expected);
    assert!(out.willpower_success);
    assert!(!out.is_botch(), "a willpower roll can never botch");
    // The temporary point was deducted; a second /wp is refused.
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.number("willpower_temp"), Some(0));
    let ev = tw.run(nick, "+roll/wp strength+brawl");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("no temporary Willpower"))
    );
}

#[test]
fn specialty_switch_and_named_specialty() {
    let (mut tw, nick) = world(S);
    nick_sheet(&mut tw);
    // /spec is gated on an applicable recorded specialty (crib): with none, it
    // is refused — and the refusal happens before any dice are rolled, so no
    // roll is logged and the RNG stream is untouched.
    let denied = tw.run(nick, "+roll/spec strength+brawl");
    assert!(
        tw.seen_by(&denied, nick)
            .iter()
            .any(|l| l.contains("no specialty recorded"))
    );
    // Record one on brawl (RNG-free), then /spec doubles 10s.
    let ev = tw.run(nick, "+spec brawl=Kickboxing");
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("added")));
    tw.run(nick, "+roll/spec strength+brawl");
    let expected = Owod20Pool::score(
        &faces(S, 0, 5),
        &Owod20Opts {
            specialty: true,
            ..Owod20Opts::new(6)
        },
    );
    assert_eq!(last_outcome(&tw, nick), expected);
    // A named specialty must be recorded on one of the pool's abilities: an
    // unrecorded name is refused, the recorded one is accepted.
    let ev = tw.run(nick, "+roll firearms spec Sniping");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("No specialty"))
    );
    let ok = tw.run(nick, "+roll brawl spec Kickboxing");
    assert!(
        tw.seen_by(&ok, nick)
            .iter()
            .all(|l| !l.contains("No specialty"))
    );
}

#[test]
fn contested_roll_resolves_both_sides() {
    let (mut tw, nick) = world(S);
    let _bob = tw.spawn_player("Bob");
    set(&mut tw, "@sheet/set Nick/strength=3");
    set(&mut tw, "@sheet/set Nick/brawl=2");
    set(&mut tw, "@sheet/set Bob/dexterity=3");
    set(&mut tw, "@sheet/set Bob/brawl=2");
    let ev = tw.run(nick, "+roll strength+brawl contested Bob=dexterity+brawl");
    // Nick draws first 5, Bob the next 5.
    let a = Owod20Pool::score(&faces(S, 0, 5), &Owod20Opts::new(6));
    let b = Owod20Pool::score(&faces(S, 5, 5), &Owod20Opts::new(6));
    assert_eq!(last_outcome(&tw, nick), a);
    let winner = match a.successes().cmp(&b.successes()) {
        std::cmp::Ordering::Greater => "Nick wins",
        std::cmp::Ordering::Less => "Bob wins",
        std::cmp::Ordering::Equal => "deadlock",
    };
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains(winner)));
}

#[test]
fn extended_roll_accumulates() {
    let (mut tw, nick) = world(S);
    nick_sheet(&mut tw);
    let ev = tw.run(nick, "+roll strength+brawl ext 3");
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("extended")));
    let st = tw.world().module_state::<WodState>().unwrap();
    let ext = st.extended(nick).expect("an extended roll");
    assert_eq!(ext.target, 3);
    assert_eq!(ext.rolls.len(), 1);
}

#[test]
fn private_roll_is_hidden_from_roommates() {
    let (mut tw, nick) = world(S);
    let bob = tw.spawn_player("Bob");
    nick_sheet(&mut tw);
    let ev = tw.run(nick, "+roll/priv strength+brawl");
    // Nick sees his own result; Bob (same room) does not.
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("rolls")));
    assert!(tw.seen_by(&ev, bob).iter().all(|l| !l.contains("rolls")));
}

#[test]
fn roll_last_is_staff_gated_for_others() {
    let (mut tw, nick) = world(S);
    let bob = tw.spawn_player("Bob");
    nick_sheet(&mut tw);
    tw.run(nick, "+roll strength+brawl");
    // Nick can read his own; Bob cannot read Nick's.
    let ev = tw.run(nick, "+roll/last");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Recent rolls"))
    );
    let denied = tw.run(bob, "+roll/last Nick");
    assert!(tw.seen_by(&denied, bob).iter().any(|l| l.contains("staff")));
    // Staff can.
    let ok = tw.run(GOD, "+roll/last Nick");
    assert!(
        tw.seen_by(&ok, GOD)
            .iter()
            .any(|l| l.contains("Recent rolls"))
    );
}

#[test]
fn roll_attaches_to_job() {
    let (mut tw, nick) = world(S);
    nick_sheet(&mut tw);
    // Open a request job as Nick.
    tw.run(nick, "+request A scene=Please run a fight.");
    let ev = tw.run(nick, "+roll strength+brawl job 1");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("attached to job 1"))
    );
    let job = get_job(tw.world(), 1).unwrap();
    assert!(job.comments.iter().any(|c| c.text.contains("[roll]")));
}

// ----------------------------------------------------------------------
// Damage
// ----------------------------------------------------------------------

fn read_sheet(tw: &TestWorld, who: ObjId) -> SheetDoc {
    let attr = tw
        .world()
        .attr(who, "sheet:wod")
        .and_then(mush_core::Attr::as_json)
        .expect("a sheet");
    serde_json::from_value(attr.clone()).expect("sheet decodes")
}

#[test]
fn harm_marks_upgrades_and_wound_penalty() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/strength=1");
    // 7 bashing → incapacitated; the 8th bashing rolls over into lethal.
    tw.run(GOD, "+harm Nick=7b");
    let ev = tw.run(GOD, "+harm Nick=1b");
    assert!(tw.seen_by(&ev, GOD).iter().any(|l| l.contains("[L")));
    let doc = read_sheet(&tw, nick);
    let track = doc.track("health").unwrap();
    assert_eq!(
        track
            .iter()
            .filter(|m| m.as_deref() == Some("lethal"))
            .count(),
        1
    );
    assert_eq!(
        track
            .iter()
            .filter(|m| m.as_deref() == Some("bashing"))
            .count(),
        6
    );
    // Incapacitation was announced when the track first filled.
    // Heal restores boxes.
    tw.run(GOD, "+heal Nick=6b");
    let doc = read_sheet(&tw, nick);
    assert_eq!(
        doc.track("health")
            .unwrap()
            .iter()
            .filter(|m| m.is_some())
            .count(),
        1
    );
}

#[test]
fn health_command_shows_track_and_incapacitation() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/strength=1");
    let ev = tw.run(nick, "+health");
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("unhurt")));
    tw.run(GOD, "+harm Nick=7L");
    let ev = tw.run(nick, "+health");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Incapacitated"))
    );
}

#[test]
fn harm_is_staff_gated() {
    let (mut tw, nick) = world(S);
    let ev = tw.run(nick, "+harm Nick=3L");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Permission denied"))
    );
}

// ----------------------------------------------------------------------
// Chargen
// ----------------------------------------------------------------------

/// Drive a full, budget-balancing chargen for Nick.
fn do_full_chargen(tw: &mut TestWorld, nick: ObjId) {
    tw.run(nick, "+chargen");
    tw.run(nick, "+chargen/attr physical=7 social=5 mental=3");
    tw.run(nick, "+chargen/abil talents=13 skills=9 knowledges=5");
    // Attributes: physical 7 (str/dex/sta 4/4/2 = 3+3+1), social 5 (cha/man/app 3/3/2),
    // mental 3 (per/int/wit 2/2/2).
    for c in [
        "+stat strength=4",
        "+stat dexterity=4",
        "+stat stamina=2",
        "+stat charisma=3",
        "+stat manipulation=3",
        "+stat appearance=2",
        "+stat perception=2",
        "+stat intelligence=2",
        "+stat wits=2",
    ] {
        tw.run(nick, c);
    }
    // Talents 13: alertness/athletics/brawl/... spread to 13 dots (cap 3).
    for c in [
        "+stat alertness=3",
        "+stat athletics=3",
        "+stat brawl=3",
        "+stat empathy=2",
        "+stat expression=2",
    ] {
        tw.run(nick, c);
    }
    // Skills 9.
    for c in ["+stat drive=3", "+stat firearms=3", "+stat stealth=3"] {
        tw.run(nick, c);
    }
    // Knowledges 5.
    for c in ["+stat academics=3", "+stat computer=2"] {
        tw.run(nick, c);
    }
    // Backgrounds 5.
    tw.run(nick, "+stat backgrounds:Resources=3");
    tw.run(nick, "+stat backgrounds:Contacts=2");
}

#[test]
fn chargen_budget_and_submit_to_approval() {
    let (mut tw, nick) = world(S);
    do_full_chargen(&mut tw, nick);

    // Budget balances: status reports readiness.
    let ev = tw.run(nick, "+chargen/status");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Budget balances"))
    );

    // Submit → sheet Pending, a wod-chargen job opened.
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
    // Willpower was derived from Courage (base 1) at least.
    assert!(doc.number("willpower").is_some());

    // Staff approve → handler stamps Approved.
    let ev = tw.run(GOD, &format!("+job/approve {job_id}=Welcome."));
    assert!(tw.seen_by(&ev, GOD).iter().any(|l| l.contains("approved")));
    let doc = read_sheet(&tw, nick);
    assert!(
        doc.approved.is_approved(),
        "sheet approved after job approval"
    );
    assert_eq!(
        get_job(tw.world(), job_id).unwrap().status,
        JobStatus::Resolved
    );
}

#[test]
fn chargen_flags_overspend_and_underspend() {
    let (mut tw, nick) = world(S);
    tw.run(nick, "+chargen");
    tw.run(nick, "+chargen/attr physical=7 social=5 mental=3");
    tw.run(nick, "+chargen/abil talents=13 skills=9 knowledges=5");
    // Underspent physical (only 2 of 7): submit refused.
    tw.run(nick, "+stat strength=3");
    let ev = tw.run(nick, "+chargen/submit");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Cannot submit"))
    );
}

#[test]
fn chargen_priority_must_be_a_permutation() {
    let (mut tw, nick) = world(S);
    tw.run(nick, "+chargen");
    let ev = tw.run(nick, "+chargen/attr physical=7 social=7 mental=3");
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("exactly")));
}

#[test]
fn stat_requires_open_chargen() {
    let (mut tw, nick) = world(S);
    let ev = tw.run(nick, "+stat strength=3");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("not in chargen"))
    );
}

// ----------------------------------------------------------------------
// XP
// ----------------------------------------------------------------------

#[test]
fn xp_award_show_spend_and_apply() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/brawl=2");
    // Award XP (staff, immediate).
    let ev = tw.run(GOD, "+xp/award Nick=10");
    assert!(
        tw.seen_by(&ev, GOD)
            .iter()
            .any(|l| l.contains("Awarded 10 XP"))
    );
    let ev = tw.run(nick, "+xp");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("10 XP available"))
    );

    // Spend to raise brawl 2 → 3: cost = current 2 × 2 = 4.
    let ev = tw.run(nick, "+xp/spend brawl=3");
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("for 4 XP")));
    // A wod-xp job opened; approve applies atomically.
    let job_id = get_job(tw.world(), 1).map(|j| j.id).unwrap();
    tw.run(GOD, &format!("+job/approve {job_id}"));
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.dots("brawl"), Some(3));
    assert_eq!(doc.number("experience"), Some(6)); // 10 - 4
    assert!(
        doc.list("xp_log")
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap_or("").contains("brawl"))
    );
}

#[test]
fn xp_spend_refused_when_unaffordable() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/strength=2");
    // No XP; raising strength 2 → 3 costs 2×4 = 8.
    let ev = tw.run(nick, "+xp/spend strength=3");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("costs 8 XP"))
    );
}

// ----------------------------------------------------------------------
// +sheet render
// ----------------------------------------------------------------------

#[test]
fn sheet_renders_classic_dots_and_track() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/strength=3");
    set(&mut tw, "@sheet/set Nick/brawl=2");
    tw.run(GOD, "+harm Nick=2b");
    let ev = tw.run(nick, "+sheet");
    let seen = tw.seen_by(&ev, nick);
    let joined = seen.join("\n");
    assert!(joined.contains("WOD SHEET"), "banner:\n{joined}");
    assert!(joined.contains("Strength"));
    assert!(joined.contains("***.."), "strength 3 dots");
}

// ----------------------------------------------------------------------
// Template seam (the M6b acceptance test)
// ----------------------------------------------------------------------

/// A throwaway splat template: adds one section, one power-pool track, and one
/// splat-specific (dots) background — exercising the merge.
struct TestSplat;

impl Template for TestSplat {
    fn id(&self) -> &'static str {
        "testsplat"
    }
    fn display_name(&self) -> &str {
        "Test Splat"
    }
    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError> {
        b.section(
            SheetSection::new("splat", "Test Splat", SectionLayout::Map)
                .field(FieldDef::map(
                    "disciplines",
                    "Disciplines",
                    FieldType::Dots { min: 0, max: 5 },
                ))
                .field(FieldDef::track(
                    "vitae",
                    "Vitae",
                    TrackSize::Fixed(10),
                    vec![],
                ))
                .field(FieldDef::dots("generation", "Generation", 0, 5)),
        )
    }
}

#[test]
fn test_splat_template_merges() {
    // Composed schema has both base and splat, base alone does not.
    let base = build_schema("wod", &[Box::new(Mortal) as Box<dyn Template>]);
    let composed = build_schema(
        "wod",
        &[Box::new(Mortal) as Box<dyn Template>, Box::new(TestSplat)],
    );
    assert!(base.section("splat").is_none());
    assert!(composed.section("attributes").is_some()); // inherited
    assert!(composed.section("splat").is_some()); // added
    assert!(composed.field("disciplines").is_some());
    assert!(composed.field("vitae").is_some());
    assert!(composed.field("generation").is_some());
    // The template enum now lists both ids.
    let opts = match &composed.field("template").unwrap().ty {
        FieldType::Enum { options } => options.clone(),
        _ => panic!("template is not an enum"),
    };
    assert!(opts.iter().any(|o| o == "mortal") && opts.iter().any(|o| o == "testsplat"));
    // A default composed sheet validates, and it drives through the engine.
    assert!(mush_ruleset::validate(&composed, &composed.default_data()).is_ok());

    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_templates(
            vec![Box::new(Mortal), Box::new(TestSplat)],
            WodConfig::default(),
            Some(S),
        )),
    ]);
    let v = tw.spawn_player("Vlad");
    tw.run(GOD, "@sheet/set Vlad/disciplines:Dominate=2");
    // Rolling a splat trait resolves from the map.
    tw.run(v, "+roll dominate");
    let out = last_outcome(&tw, v);
    // Dominate 2 → 2 dice.
    assert_eq!(out.dice.len(), 2);
}

// ----------------------------------------------------------------------
// agent_profile
// ----------------------------------------------------------------------

#[test]
fn agent_profile_grounds_a_sheet() {
    let ruleset = WodRuleset::with_seed(S);
    let mut doc = SheetDoc::new_default(ruleset.sheet_schema());
    doc.set_number("strength", 4);
    doc.set_number("brawl", 3);
    doc.set_number("firearms", 4);
    doc.set_number("willpower", 6);
    let profile = ruleset.agent_profile(&doc);
    assert!(
        profile.summary.contains("Strength 4"),
        "summary: {}",
        profile.summary
    );
    assert!(profile.summary.contains("Willpower 6"));
    assert!(
        profile
            .tags
            .iter()
            .any(|t| t.contains("skill:brawl") || t.contains("skill:firearms"))
    );
    assert!(profile.tags.iter().any(|t| t == "template:mortal"));
    assert_eq!(profile.tools.len(), 1);
    assert_eq!(profile.tools[0].name.as_str(), "wod_roll");
    assert_eq!(profile.stats["willpower"], serde_json::json!(6));
}

// ----------------------------------------------------------------------
// Fixes from the M6a review (rules-fidelity, framework, conformance)
// ----------------------------------------------------------------------

#[test]
fn botch_surfaces_through_roll() {
    // Seed 6's first two dice are [1, 3]: zero successes, one 1 -> botch. Prove
    // the command layer actually renders a "botch" line to the room, not just
    // that the dice engine classifies it.
    let (mut tw, nick) = world(6);
    let ev = tw.run(nick, "+roll 2");
    let out = last_outcome(&tw, nick);
    assert!(out.is_botch(), "expected a botch, got {out:?}");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.to_lowercase().contains("botch")),
        "the room must see a botch line"
    );
}

#[test]
fn contested_roll_validates_opponent_before_spending_willpower() {
    let (mut tw, nick) = world(S);
    let _bob = tw.spawn_player("Bob");
    set(&mut tw, "@sheet/set Nick/strength=3");
    set(&mut tw, "@sheet/set Nick/brawl=2");
    set(&mut tw, "@sheet/set Nick/willpower_temp=1");
    set(&mut tw, "@sheet/set Bob/strength=2");
    // Bob's pool expression is unresolvable, so the command errors before
    // Nick's Willpower is deducted.
    let ev = tw.run(nick, "+roll/wp strength+brawl contested Bob=notatrait");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("notatrait")),
        "the bad opponent pool should be reported"
    );
    // The temporary Willpower point is intact: it was not burned on a roll
    // that never resolved.
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.number("willpower_temp"), Some(1));
}

#[test]
fn willpower_temp_spend_is_clamped_to_permanent() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/strength=3");
    set(&mut tw, "@sheet/set Nick/willpower=2");
    // Staff over-sets the temporary pool above permanent (invariant break).
    set(&mut tw, "@sheet/set Nick/willpower_temp=9");
    // Spending clamps the read to permanent (2) and self-heals the stored
    // value: 2 - 1 = 1, not 8.
    tw.run(nick, "+roll/wp strength");
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.number("willpower_temp"), Some(1));
}

#[test]
fn stat_refuses_xp_and_bookkeeping_fields_in_chargen() {
    let (mut tw, nick) = world(S);
    tw.run(nick, "+chargen");
    for bad in ["experience", "total_xp", "willpower_temp"] {
        let ev = tw.run(nick, &format!("+stat {bad}=100"));
        assert!(
            tw.seen_by(&ev, nick)
                .iter()
                .any(|l| l.contains("not a chargen-allocatable trait")),
            "+stat {bad} must be refused during chargen"
        );
    }
    // A legitimate trait still works.
    let ev = tw.run(nick, "+stat strength=3");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Set strength"))
    );
    // No free starting XP was minted.
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.number("experience"), Some(0));
}

#[test]
fn chargen_and_stat_locked_while_pending() {
    let (mut tw, nick) = world(S);
    do_full_chargen(&mut tw, nick);
    tw.run(nick, "+chargen/submit");
    assert!(matches!(
        read_sheet(&tw, nick).approved,
        ApprovalState::Pending(_)
    ));
    // +chargen is refused while the approval job is open.
    let ev = tw.run(nick, "+chargen");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("awaiting approval"))
    );
    // +stat is refused (the session is closed).
    let ev = tw.run(nick, "+stat strength=5");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("not in chargen"))
    );
    // A second submit is refused rather than orphaning the first job.
    let ev = tw.run(nick, "+chargen/submit");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("already submitted"))
    );
    // Still exactly one chargen job.
    assert!(get_job(tw.world(), 1).is_some());
    assert!(get_job(tw.world(), 2).is_none());
}

#[test]
fn chargen_freebie_overspend_refused_at_submit() {
    let (mut tw, nick) = world(S);
    do_full_chargen(&mut tw, nick);
    // Everything balances at 0 freebies; blow past the 15-point freebie cap
    // with extra background dots (each overflow dot costs 1 freebie -> 16).
    for c in [
        "+stat backgrounds:Allies=5",
        "+stat backgrounds:Fame=5",
        "+stat backgrounds:Mentor=5",
        "+stat backgrounds:Influence=1",
    ] {
        tw.run(nick, c);
    }
    let ev = tw.run(nick, "+chargen/submit");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Cannot submit") && l.to_lowercase().contains("freebie")),
        "freebie overspend must block submit"
    );
    // No job opened; still unapproved.
    assert!(matches!(
        read_sheet(&tw, nick).approved,
        ApprovalState::Unapproved
    ));
    assert!(get_job(tw.world(), 1).is_none());
}

#[test]
fn chargen_approval_revalidates_budget() {
    let (mut tw, nick) = world(S);
    do_full_chargen(&mut tw, nick);
    tw.run(nick, "+chargen/submit");
    let job = match read_sheet(&tw, nick).approved {
        ApprovalState::Pending(j) => j,
        other => panic!("expected pending, got {other:?}"),
    };
    // Staff edits the pending sheet below budget (underspends Physical).
    set(&mut tw, "@sheet/set Nick/strength=1");
    // Approval re-validates and refuses; the job stays open.
    let ev = tw.run(GOD, &format!("+job/approve {job}"));
    assert!(
        tw.seen_by(&ev, GOD)
            .iter()
            .any(|l| l.contains("no longer balances")),
        "approval must re-check the budget"
    );
    assert!(!read_sheet(&tw, nick).approved.is_approved());
    assert_eq!(get_job(tw.world(), job).unwrap().status, JobStatus::Open);
}

#[test]
fn xp_double_spend_is_rejected_at_second_approval() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/brawl=2");
    tw.run(GOD, "+xp/award Nick=20");
    // Two spends filed against brawl 2->3 (cost 4 each): jobs 1 and 2.
    tw.run(nick, "+xp/spend brawl=3");
    tw.run(nick, "+xp/spend brawl=3");
    // Approve job 1: brawl -> 3, 20 - 4 = 16 XP.
    tw.run(GOD, "+job/approve 1");
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.dots("brawl"), Some(3));
    assert_eq!(doc.number("experience"), Some(16));
    // Approve job 2: brawl already moved off 2, so the atomic re-check fails,
    // nothing is applied, and the job stays open (no double-spend).
    let ev = tw.run(GOD, "+job/approve 2");
    assert!(
        tw.seen_by(&ev, GOD).iter().any(|l| l.contains("re-file")),
        "the moved-trait re-check must reject the stale spend"
    );
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.dots("brawl"), Some(3));
    assert_eq!(doc.number("experience"), Some(16));
    assert_eq!(get_job(tw.world(), 2).unwrap().status, JobStatus::Open);
}

#[test]
fn chargen_survives_a_reboot_via_persisted_pools() {
    let (mut tw, nick) = world(S);
    tw.run(nick, "+chargen");
    // A non-default assignment: social gets the top tier (7), not the default 5.
    tw.run(nick, "+chargen/attr physical=3 social=7 mental=5");
    tw.run(nick, "+chargen/abil talents=13 skills=9 knowledges=5");
    // Simulate a reboot / dropped link: the transient session is lost, but the
    // sheet (carrying the persisted chargen_pools) survives.
    tw.world_mut()
        .module_state_mut::<WodState>()
        .expect("WodState")
        .close_chargen(nick);
    // Resume: +chargen re-opens and re-hydrates the pools from the sheet.
    tw.run(nick, "+chargen");
    // Spend the social pool of 7 exactly (cha/man/app 4/4/2 = 3+3+1).
    for c in [
        "+stat charisma=4",
        "+stat manipulation=4",
        "+stat appearance=2",
    ] {
        tw.run(nick, c);
    }
    let ev = tw.run(nick, "+chargen/status");
    let seen = tw.seen_by(&ev, nick);
    // Social is assessed against the persisted 7, not the default 5.
    assert!(
        seen.iter()
            .any(|l| l.contains("Social") && l.contains("7/7")),
        "social must use the persisted pool of 7: {seen:?}"
    );
    // And it is NOT misreported as freebie overspend.
    assert!(seen.iter().all(|l| !l.contains("to freebies")));
}

#[test]
fn harm_lethal_rolls_over_to_aggravated_and_death_via_command() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/strength=1");
    // 7 lethal fills the track; the 8th lethal upgrades the least-severe box to
    // aggravated (rollover through the +harm command path).
    tw.run(GOD, "+harm Nick=7L");
    let ev = tw.run(GOD, "+harm Nick=1L");
    assert!(tw.seen_by(&ev, GOD).iter().any(|l| l.contains("[A")));
    let doc = read_sheet(&tw, nick);
    assert_eq!(
        doc.track("health")
            .unwrap()
            .iter()
            .filter(|m| m.as_deref() == Some("aggravated"))
            .count(),
        1
    );
    // Fill the rest with aggravated: the whole track is aggravated and death is
    // announced to the room (the occupant sees it).
    let ev = tw.run(GOD, "+harm Nick=7A");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.to_lowercase().contains("slain")),
        "a fully-aggravated track announces death"
    );
}

#[test]
fn unapproved_character_can_still_roll() {
    // Deliberate: play commands work before approval (rolling during chargen is
    // useful). Nothing gates +roll on ApprovalState; pinned so a regression
    // that accidentally added such a gate would be caught.
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/strength=3");
    assert!(matches!(
        read_sheet(&tw, nick).approved,
        ApprovalState::Unapproved
    ));
    let ev = tw.run(nick, "+roll strength");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Nick rolls"))
    );
}
