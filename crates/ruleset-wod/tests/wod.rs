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
    // Under multi-template dispatch a character must BE the splat before its
    // fields exist on their sheet: a mortal's schema has no `disciplines`.
    let denied = tw.run(GOD, "@sheet/set Vlad/disciplines:Dominate=2");
    assert!(
        tw.seen_by(&denied, GOD)
            .iter()
            .any(|l| l.contains("No such sheet field")),
        "a mortal's active schema must not carry splat fields"
    );
    // Pick the template, then the splat field is settable and rolls.
    tw.run(GOD, "@sheet/set Vlad/template=testsplat");
    tw.run(GOD, "@sheet/set Vlad/disciplines:Dominate=2");
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

// ----------------------------------------------------------------------
// M6b: Vampire (V20) + the per-template dispatch
// ----------------------------------------------------------------------

/// The vampire's active schema carries Disciplines / Blood Pool / Generation;
/// a mortal's carries none of them (and neither carries a mage's Spheres).
#[test]
fn template_selects_the_active_schema() {
    let ruleset = WodRuleset::with_seed(S);
    let base = ruleset.sheet_schema().clone();

    let mut vamp = SheetDoc::new_default(&base);
    vamp.set_text("template", "vampire");
    let vsch = ruleset.schema_for(&vamp);
    assert!(vsch.field("disciplines").is_some());
    assert!(vsch.field("bloodpool").is_some());
    assert!(vsch.field("generation").is_some());
    assert!(vsch.field("spheres").is_none(), "a vampire is not a mage");

    let mortal = SheetDoc::new_default(&base);
    let msch = ruleset.schema_for(&mortal);
    assert!(msch.field("disciplines").is_none());
    assert!(msch.field("bloodpool").is_none());
    assert!(msch.field("generation").is_none());
}

/// The `+sheet` render surface selects the active schema: a vampire's sheet
/// shows the Kindred / Disciplines sections, a mortal's does not.
#[test]
fn sheet_render_dispatches_on_template() {
    let (mut tw, nick) = world(S);
    // Mortal sheet: no Kindred sections.
    set(&mut tw, "@sheet/set Nick/strength=3");
    let ev = tw.run(nick, "+sheet");
    let mortal = tw.seen_by(&ev, nick).join("\n");
    assert!(!mortal.contains("DISCIPLINES"));
    assert!(!mortal.contains("Blood Pool"));
    // Turn Kindred: the render now carries Disciplines + Blood Pool.
    set(&mut tw, "@sheet/set Nick/template=vampire");
    set(&mut tw, "@sheet/set Nick/disciplines:Celerity=2");
    let ev = tw.run(nick, "+sheet");
    let vamp = tw.seen_by(&ev, nick).join("\n");
    assert!(
        vamp.contains("DISCIPLINES"),
        "vampire sheet shows Disciplines"
    );
    assert!(
        vamp.contains("Blood Pool"),
        "vampire sheet shows Blood Pool"
    );
    assert!(vamp.contains("Celerity"));
}

/// Splat commands are always registered but self-gate: a mortal is refused.
#[test]
fn splat_commands_refuse_non_kindred() {
    let (mut tw, nick) = world(S);
    for cmd in ["+blood", "+disc", "+frenzy", "+generation"] {
        let ev = tw.run(nick, cmd);
        assert!(
            tw.seen_by(&ev, nick)
                .iter()
                .any(|l| l.contains("not Kindred")),
            "{cmd} must refuse a mortal"
        );
    }
}

/// Drive a full, budget-balancing Brujah chargen.
fn do_vampire_chargen(tw: &mut TestWorld, who: ObjId) {
    tw.run(who, "+chargen");
    tw.run(who, "+chargen/template vampire");
    tw.run(who, "+chargen/attr physical=7 social=5 mental=3");
    tw.run(who, "+chargen/abil talents=13 skills=9 knowledges=5");
    tw.run(who, "+stat clan=brujah");
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
        tw.run(who, c);
    }
    for c in [
        "+stat alertness=3",
        "+stat athletics=3",
        "+stat brawl=3",
        "+stat empathy=2",
        "+stat expression=2",
    ] {
        tw.run(who, c);
    }
    for c in ["+stat drive=3", "+stat firearms=3", "+stat stealth=3"] {
        tw.run(who, c);
    }
    for c in ["+stat academics=3", "+stat computer=2"] {
        tw.run(who, c);
    }
    tw.run(who, "+stat backgrounds:Resources=3");
    tw.run(who, "+stat backgrounds:Contacts=2");
    // Disciplines: 3 in-clan dots (Brujah = Celerity/Potence/Presence).
    tw.run(who, "+stat disciplines:Celerity=2");
    tw.run(who, "+stat disciplines:Potence=1");
    // Virtues: 7 dots on top of 1 each (2 + 2 + 3).
    tw.run(who, "+stat conscience=3");
    tw.run(who, "+stat self_control=3");
    tw.run(who, "+stat courage=4");
}

#[test]
fn vampire_full_chargen_derives_pools_and_approves() {
    let (mut tw, nick) = world(S);
    do_vampire_chargen(&mut tw, nick);

    // Budget (incl. Disciplines + Virtues pools) balances.
    let ev = tw.run(nick, "+chargen/status");
    let seen = tw.seen_by(&ev, nick);
    assert!(
        seen.iter().any(|l| l.contains("Budget balances")),
        "vampire budget must balance: {seen:?}"
    );
    assert!(
        seen.iter()
            .any(|l| l.contains("Disciplines") && l.contains("3/3"))
    );
    assert!(
        seen.iter()
            .any(|l| l.contains("Virtues") && l.contains("7/7"))
    );

    // Submit → Pending → staff approve → Approved.
    tw.run(nick, "+chargen/submit");
    let job = match read_sheet(&tw, nick).approved {
        ApprovalState::Pending(j) => j,
        other => panic!("expected Pending, got {other:?}"),
    };
    tw.run(GOD, &format!("+job/approve {job}=Welcome to the night."));
    let doc = read_sheet(&tw, nick);
    assert!(doc.approved.is_approved());
    assert_eq!(doc.text("template"), Some("vampire"));
    // Derived starting values.
    assert_eq!(doc.number("willpower"), Some(4), "Willpower = Courage");
    assert_eq!(doc.number("bloodpool"), Some(10), "13th-gen Blood Pool max");
    assert_eq!(
        doc.number("humanity"),
        Some(6),
        "Humanity = Conscience + Self-Control"
    );
    assert_eq!(doc.number("generation"), Some(13));
    assert!(
        doc.text("weakness")
            .unwrap_or("")
            .to_lowercase()
            .contains("frenzy"),
        "Brujah weakness derived onto the sheet"
    );
}

#[test]
fn disc_roll_builds_and_rolls_the_pool() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=vampire");
    set(&mut tw, "@sheet/set Nick/dexterity=3");
    set(&mut tw, "@sheet/set Nick/disciplines:Celerity=2");
    let ev = tw.run(nick, "+disc/roll celerity+dexterity vs 6");
    let out = last_outcome(&tw, nick);
    // Celerity 2 + Dexterity 3 = 5 dice at difficulty 6.
    assert_eq!(out.dice.len(), 5);
    assert_eq!(out.difficulty, 6);
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("discipline"))
    );
    // A discipline you do not have is refused.
    let denied = tw.run(nick, "+disc/roll dominate+wits");
    assert!(
        tw.seen_by(&denied, nick)
            .iter()
            .any(|l| l.contains("Discipline you have"))
    );
}

// ----------------------------------------------------------------------
// M14 — power use routed through the per-observer perception director
// ----------------------------------------------------------------------

/// A subtle Discipline (Dominate: Supernatural channel, gated behind Auspex) is
/// perceived by an Auspex-bearing, sharp-eyed onlooker, is NOT perceived by a
/// mundane one, and NEVER reaches the room as a raw dice line. The actor always
/// perceives their own act.
#[test]
fn subtle_dominate_seen_by_auspex_not_by_mundane_or_room() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=vampire");
    set(&mut tw, "@sheet/set Nick/manipulation=3");
    set(&mut tw, "@sheet/set Nick/disciplines:Dominate=3");

    // A sharp, Auspex-bearing witness (Kindred): can pierce the veil.
    let seer = tw.spawn_player("Seer");
    set(&mut tw, "@sheet/set Seer/template=vampire");
    set(&mut tw, "@sheet/set Seer/perception=5");
    set(&mut tw, "@sheet/set Seer/awareness=5");
    set(&mut tw, "@sheet/set Seer/disciplines:Auspex=3");

    // A mundane witness (no Auspex): cannot notice a subtle Discipline at all.
    let rube = tw.spawn_player("Rube");
    set(&mut tw, "@sheet/set Rube/perception=5");
    set(&mut tw, "@sheet/set Rube/awareness=5");

    let ev = tw.run(nick, "+disc/roll dominate+manipulation vs 6");

    // V1: the actor always perceives their own act.
    assert!(
        tw.seen_by(&ev, nick).iter().any(|l| l.contains("discipline")),
        "actor sees own act: {:?}",
        tw.seen_by(&ev, nick)
    );
    // The Auspex witness perceives the tailored line — but NOT the raw dice line.
    let seen = tw.seen_by(&ev, seer);
    assert!(
        seen.iter().any(|l| l.contains("bends another's will")),
        "the Auspex witness notices: {seen:?}"
    );
    assert!(
        !seen.iter().any(|l| l.contains("dice, diff")),
        "no raw dice line leaks to a witness: {seen:?}"
    );
    // V1: the mundane witness gets nothing at all (required sense: Auspex).
    assert!(
        tw.seen_by(&ev, rube).is_empty(),
        "the mundane witness gets true silence: {:?}",
        tw.seen_by(&ev, rube)
    );
    // ...and no witness roll was even made for them (no RNG spent).
    assert!(
        tw.world()
            .module_state::<WodState>()
            .unwrap()
            .recent_logs(rube, 1)
            .is_empty(),
        "a senseless observer advances no RNG"
    );
}

/// V3 determinism: a subtle Discipline in a room of only mundane onlookers
/// advances the seeded RNG for the actor's own throw ALONE — the required-sense
/// early-return skips every senseless observer before any witness roll.
#[test]
fn subtle_power_spends_no_rng_on_a_mundane_room() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=vampire");
    set(&mut tw, "@sheet/set Nick/manipulation=3");
    set(&mut tw, "@sheet/set Nick/disciplines:Dominate=3");
    // Two mundane onlookers (no Auspex): gated out BEFORE any roll.
    let ann = tw.spawn_player("Ann");
    set(&mut tw, "@sheet/set Ann/perception=5");
    let bob = tw.spawn_player("Bob");
    set(&mut tw, "@sheet/set Bob/perception=5");

    tw.run(nick, "+disc/roll dominate+manipulation vs 6");

    // Nick's Dominate pool = manipulation 3 + Dominate 3 = 6 dice at diff 6; the
    // witness phase spent no RNG, so the outcome is exactly the first six faces.
    let expected = Owod20Pool::score(&faces(S, 0, 6), &Owod20Opts::new(6));
    assert_eq!(last_outcome(&tw, nick), expected);
    // Neither onlooker has a witness roll logged.
    assert!(
        tw.world()
            .module_state::<WodState>()
            .unwrap()
            .recent_logs(ann, 1)
            .is_empty()
    );
    assert!(
        tw.world()
            .module_state::<WodState>()
            .unwrap()
            .recent_logs(bob, 1)
            .is_empty()
    );
}

/// V2: a blatant/vulgar act still reads to the room-at-large — a bystander who
/// makes no sense roll (no Perception) still sees the obvious-tier line.
#[test]
fn vulgar_cast_reads_to_the_room_at_large() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=mage");
    set(&mut tw, "@sheet/set Nick/arete=4");
    let bystander = tw.spawn_player("Bystander"); // no perception

    let ev = tw.run(nick, "+cast/vulgar Hurl a bolt of fire vs 6");

    // The actor sees their own working, mechanics and all (the raw dice line).
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.to_lowercase().contains("vulgar magick"))
    );
    // The room-at-large sees the obvious-tier, IN-FICTION line even with no sense
    // roll won — and NOT the raw mechanical dice string (that stays Actor-only).
    let seen = tw.seen_by(&ev, bystander);
    assert!(
        seen.iter().any(|l| l.contains("a working takes shape")),
        "a vulgar act reads to the room in fiction: {seen:?}"
    );
    assert!(
        !seen.iter().any(|l| l.to_lowercase().contains("vulgar magick")
            || l.contains("dice, diff")),
        "the raw dice line never leaks to the room-at-large: {seen:?}"
    );
}

/// A coincidental (non-vulgar) Discipline does NOT broadcast to the room: a
/// mundane bystander present for a Celerity roll sees no raw dice line unless
/// they make the (Visual) sense roll.
#[test]
fn coincidental_power_does_not_broadcast_the_dice_line() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=vampire");
    set(&mut tw, "@sheet/set Nick/dexterity=3");
    set(&mut tw, "@sheet/set Nick/disciplines:Celerity=2");
    let bystander = tw.spawn_player("Bystander");
    // No Perception/Awareness: cannot make the Visual sense roll (pool 0).

    let ev = tw.run(nick, "+disc/roll celerity+dexterity vs 6");
    // The actor sees their own act.
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("discipline")));
    // The oblivious bystander sees no raw dice line (no room broadcast).
    assert!(
        !tw.seen_by(&ev, bystander)
            .iter()
            .any(|l| l.contains("dice, diff")),
        "no raw dice line broadcast to the room: {:?}",
        tw.seen_by(&ev, bystander)
    );
}

#[test]
fn blood_spend_and_gain_are_bounded() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=vampire"); // 13th gen: max 10
    tw.run(nick, "+blood/gain 5");
    assert_eq!(read_sheet(&tw, nick).number("bloodpool"), Some(5));
    // Feeding caps at the generation maximum.
    tw.run(nick, "+blood/gain 100");
    assert_eq!(read_sheet(&tw, nick).number("bloodpool"), Some(10));
    tw.run(nick, "+blood/spend 3");
    assert_eq!(read_sheet(&tw, nick).number("bloodpool"), Some(7));
    // Cannot overspend the pool.
    let ev = tw.run(nick, "+blood/spend 100");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("only 7 blood"))
    );
    assert_eq!(read_sheet(&tw, nick).number("bloodpool"), Some(7));
    // Healing with no damage is refused (no blood wasted).
    let ev = tw.run(nick, "+blood/heal 2");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("no bashing or lethal"))
    );
    assert_eq!(read_sheet(&tw, nick).number("bloodpool"), Some(7));
    // With damage, healing spends blood 1:1 and clears the levels.
    tw.run(GOD, "+harm Nick=2b");
    tw.run(nick, "+blood/heal 2");
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.number("bloodpool"), Some(5));
    assert_eq!(
        doc.track("health")
            .unwrap()
            .iter()
            .filter(|m| m.is_some())
            .count(),
        0,
        "both bashing levels mended"
    );
}

#[test]
fn frenzy_rolls_the_resisting_virtue() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=vampire");
    set(&mut tw, "@sheet/set Nick/self_control=3");
    set(&mut tw, "@sheet/set Nick/courage=4");
    // Anger frenzy: Self-Control (3 dice).
    tw.run(nick, "+frenzy vs 6");
    assert_eq!(last_outcome(&tw, nick).dice.len(), 3);
    // Rötschreck: Courage (4 dice).
    tw.run(nick, "+frenzy/fear vs 6");
    assert_eq!(last_outcome(&tw, nick).dice.len(), 4);
}

#[test]
fn generation_derives_the_blood_table() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=vampire");
    set(&mut tw, "@sheet/set Nick/generation=8");
    let ev = tw.run(nick, "+generation");
    let seen = tw.seen_by(&ev, nick);
    // 8th generation: 15 blood / 3 per turn / trait max 5.
    assert!(
        seen.iter().any(|l| l.contains("max 15")
            && l.contains("3 blood/turn")
            && l.contains("trait max 5")),
        "generation table: {seen:?}"
    );
}

#[test]
fn discipline_xp_costs_in_clan_vs_out_of_clan() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=vampire");
    set(&mut tw, "@sheet/set Nick/clan=brujah");
    set(&mut tw, "@sheet/set Nick/disciplines:Celerity=2"); // in-clan
    set(&mut tw, "@sheet/set Nick/disciplines:Auspex=2"); // out-of-clan
    tw.run(GOD, "+xp/award Nick=50");
    // In-clan Celerity 2→3: current 2 × 5 = 10 XP.
    let ev = tw.run(nick, "+xp/spend disciplines:Celerity=3");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("for 10 XP")),
        "in-clan discipline is ×5"
    );
    // Out-of-clan Auspex 2→3: current 2 × 7 = 14 XP.
    let ev = tw.run(nick, "+xp/spend disciplines:Auspex=3");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("for 14 XP")),
        "out-of-clan discipline is ×7"
    );
    // Approve the in-clan raise (job 1): it applies atomically.
    tw.run(GOD, "+job/approve 1");
    let doc = read_sheet(&tw, nick);
    assert_eq!(
        doc.map("disciplines").unwrap()["Celerity"],
        serde_json::json!(3)
    );
    assert_eq!(doc.number("experience"), Some(40)); // 50 - 10
}

// ----------------------------------------------------------------------
// M6b: Werewolf (W20) on the same per-template dispatch
// ----------------------------------------------------------------------

/// The Garou's active schema carries Rage / Gnosis / Renown / Gifts / Forms and
/// none of the other splats' fields; a mortal's carries none of the Garou ones.
#[test]
fn werewolf_template_selects_the_active_schema() {
    let ruleset = WodRuleset::with_seed(S);
    let base = ruleset.sheet_schema().clone();

    let mut garou = SheetDoc::new_default(&base);
    garou.set_text("template", "werewolf");
    let w = ruleset.schema_for(&garou);
    for id in [
        "rage",
        "gnosis",
        "glory_perm",
        "gifts",
        "current_form",
        "breed",
    ] {
        assert!(w.field(id).is_some(), "werewolf schema missing {id}");
    }
    assert!(w.field("disciplines").is_none(), "a Garou is not Kindred");
    assert!(w.field("spheres").is_none(), "a Garou is not a mage");

    let mortal = SheetDoc::new_default(&base);
    let m = ruleset.schema_for(&mortal);
    assert!(m.field("rage").is_none());
    assert!(m.field("gnosis").is_none());
    assert!(m.field("gifts").is_none());
}

/// Werewolf splat commands are always registered but self-gate: a mortal is
/// refused politely. (Werewolf frenzy lives under `+rage/frenzy` because Vampire
/// already owns the `+frenzy` command name.)
#[test]
fn garou_commands_refuse_non_garou() {
    let (mut tw, nick) = world(S);
    for cmd in [
        "+rage",
        "+gnosis",
        "+renown",
        "+gifts",
        "+shift crinos",
        "+rank",
    ] {
        let ev = tw.run(nick, cmd);
        assert!(
            tw.seen_by(&ev, nick)
                .iter()
                .any(|l| l.contains("not Garou")),
            "{cmd} must refuse a mortal"
        );
    }
}

/// `+sheet` renders the Garou sections only for a werewolf.
#[test]
fn werewolf_sheet_render_dispatches_on_template() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/strength=3");
    let ev = tw.run(nick, "+sheet");
    let mortal = tw.seen_by(&ev, nick).join("\n");
    assert!(!mortal.contains("GAROU"));
    assert!(!mortal.contains("Gnosis"));
    set(&mut tw, "@sheet/set Nick/template=werewolf");
    let ev = tw.run(nick, "+sheet");
    let garou = tw.seen_by(&ev, nick).join("\n");
    assert!(garou.contains("GAROU"), "garou sheet shows the Garou block");
    assert!(garou.contains("Rage"));
    assert!(garou.contains("Gnosis"));
}

/// `+shift` sets the current form and the base `+roll` reflects the war form's
/// Physical deltas (Crinos Strength +4).
#[test]
fn shift_applies_form_deltas_to_roll() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=werewolf");
    set(&mut tw, "@sheet/set Nick/strength=3");
    // Homid: Strength+Brawl (3 + 0) = 3 dice.
    tw.run(nick, "+roll strength+brawl");
    assert_eq!(last_outcome(&tw, nick).dice.len(), 3);
    // Shift to Crinos: Str +4, so the same pool becomes 7 dice.
    let ev = tw.run(nick, "+shift crinos");
    let seen = tw.seen_by(&ev, nick);
    assert!(seen.iter().any(|l| l.to_lowercase().contains("crinos")));
    assert!(
        seen.iter().any(|l| l.contains("Delirium")),
        "the war form warns about the Delirium"
    );
    tw.run(nick, "+roll strength+brawl");
    assert_eq!(
        last_outcome(&tw, nick).dice.len(),
        7,
        "Crinos Strength +4 reaches the base +roll"
    );
    // A non-physical pool is unaffected by the form.
    set(&mut tw, "@sheet/set Nick/occult=2");
    tw.run(nick, "+roll occult");
    assert_eq!(last_outcome(&tw, nick).dice.len(), 2);
    // Shifting back to Homid drops the bonus.
    tw.run(nick, "+shift homid");
    tw.run(nick, "+roll strength+brawl");
    assert_eq!(last_outcome(&tw, nick).dice.len(), 3);
}

/// Rage/Gnosis spend and gain operate on the *temporary* pool: `/gain` recovers
/// only up to the permanent rating (never inflating it for free), and `/spend`
/// is bounded below. The permanent rating rises only through XP.
#[test]
fn rage_and_gnosis_pools_are_bounded() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=werewolf");
    // A permanent Rage of 5 with the temporary pool drained.
    set(&mut tw, "@sheet/set Nick/rage=5");
    set(&mut tw, "@sheet/set Nick/rage_temp=0");
    tw.run(nick, "+rage/gain 3");
    assert_eq!(read_sheet(&tw, nick).number("rage_temp"), Some(3));
    // Gain caps at the permanent rating (5), not the 0..10 schema range.
    tw.run(nick, "+rage/gain 100");
    assert_eq!(read_sheet(&tw, nick).number("rage_temp"), Some(5));
    // The permanent rating is never inflated by /gain (no free XP).
    assert_eq!(read_sheet(&tw, nick).number("rage"), Some(5));
    tw.run(nick, "+rage/spend 3");
    assert_eq!(read_sheet(&tw, nick).number("rage_temp"), Some(2));
    let ev = tw.run(nick, "+rage/spend 100");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("only 2 Rage"))
    );
    assert_eq!(read_sheet(&tw, nick).number("rage_temp"), Some(2));
    // Gnosis behaves the same way.
    set(&mut tw, "@sheet/set Nick/gnosis=5");
    set(&mut tw, "@sheet/set Nick/gnosis_temp=0");
    tw.run(nick, "+gnosis/gain 4");
    tw.run(nick, "+gnosis/spend 1");
    assert_eq!(read_sheet(&tw, nick).number("gnosis_temp"), Some(3));
    assert_eq!(
        read_sheet(&tw, nick).number("gnosis"),
        Some(5),
        "permanent untouched"
    );
}

/// `+renown/gain` logs temporary Renown; permanent Renown drives Rank.
#[test]
fn renown_gain_and_rank_thresholds() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=werewolf");
    let ev = tw.run(nick, "+renown/gain glory=2");
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("2/10")));
    assert_eq!(read_sheet(&tw, nick).number("glory_temp"), Some(2));
    // Staff raises permanent renown; Rank re-derives (7 total = Fostern, Rank 2).
    set(&mut tw, "@sheet/set Nick/glory_perm=7");
    assert_eq!(read_sheet(&tw, nick).number("rank"), Some(2));
    let ev = tw.run(nick, "+rank");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Rank 2") && l.contains("Fostern"))
    );
}

/// `+gift/roll` validates the Gift and builds+rolls its pool.
#[test]
fn gift_roll_builds_and_rolls_the_pool() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=werewolf");
    set(&mut tw, "@sheet/set Nick/dexterity=3");
    set(&mut tw, "@sheet/set Nick/brawl=2");
    set(&mut tw, "@sheet/set Nick/gifts:Razor Claws=1");
    let ev = tw.run(nick, "+gift/roll razor claws=dexterity+brawl vs 6");
    let out = last_outcome(&tw, nick);
    // Dexterity 3 + Brawl 2 = 5 dice at difficulty 6.
    assert_eq!(out.dice.len(), 5);
    assert_eq!(out.difficulty, 6);
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.to_lowercase().contains("razor claws"))
    );
    // A Gift you do not know is refused.
    let denied = tw.run(nick, "+gift/roll mother's touch=stamina+empathy");
    assert!(
        tw.seen_by(&denied, nick)
            .iter()
            .any(|l| l.contains("do not know the Gift"))
    );
}

/// Drive a full, budget-balancing Garou chargen (mortal-identical budget plus
/// the Breed/Auspice/Tribe choices).
fn do_werewolf_chargen(tw: &mut TestWorld, who: ObjId) {
    tw.run(who, "+chargen");
    tw.run(who, "+chargen/template werewolf");
    tw.run(who, "+chargen/attr physical=7 social=5 mental=3");
    tw.run(who, "+chargen/abil talents=13 skills=9 knowledges=5");
    tw.run(who, "+stat breed=lupus"); // Gnosis 5
    tw.run(who, "+stat auspice=ahroun"); // Rage 5, Glory 3
    tw.run(who, "+stat tribe=silver_fangs"); // Willpower 3
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
        tw.run(who, c);
    }
    for c in [
        "+stat alertness=3",
        "+stat athletics=3",
        "+stat brawl=3",
        "+stat empathy=2",
        "+stat expression=2",
    ] {
        tw.run(who, c);
    }
    for c in ["+stat drive=3", "+stat firearms=3", "+stat stealth=3"] {
        tw.run(who, c);
    }
    for c in ["+stat academics=3", "+stat computer=2"] {
        tw.run(who, c);
    }
    tw.run(who, "+stat backgrounds:Resources=3");
    tw.run(who, "+stat backgrounds:Contacts=2");
}

#[test]
fn werewolf_full_chargen_derives_pools_and_approves() {
    let (mut tw, nick) = world(S);
    do_werewolf_chargen(&mut tw, nick);

    // Budget balances byte-identically to a mortal (no splat pools).
    let ev = tw.run(nick, "+chargen/status");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Budget balances")),
        "werewolf budget must balance"
    );

    // A locked derived field cannot be set with +stat during chargen.
    let ev = tw.run(nick, "+stat rage=9");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("not a chargen-allocatable trait")),
        "Rage is derived, not +stat-settable"
    );

    // Submit → Pending → approve → Approved with derived starting values.
    tw.run(nick, "+chargen/submit");
    let job = match read_sheet(&tw, nick).approved {
        ApprovalState::Pending(j) => j,
        other => panic!("expected Pending, got {other:?}"),
    };
    tw.run(GOD, &format!("+job/approve {job}=For Gaia."));
    let doc = read_sheet(&tw, nick);
    assert!(doc.approved.is_approved());
    assert_eq!(doc.text("template"), Some("werewolf"));
    assert_eq!(doc.number("rage"), Some(5), "Ahroun Rage");
    assert_eq!(doc.number("rage_temp"), Some(5), "Rage pool starts full");
    assert_eq!(doc.number("gnosis"), Some(5), "Lupus Gnosis");
    assert_eq!(
        doc.number("gnosis_temp"),
        Some(5),
        "Gnosis pool starts full"
    );
    assert_eq!(doc.number("willpower"), Some(3), "Silver Fang Willpower");
    assert_eq!(doc.number("glory_perm"), Some(3), "Ahroun Glory");
    assert_eq!(doc.number("rank"), Some(1), "a starting Garou is a Cliath");
    // Three starting Gifts (one each from breed/auspice/tribe).
    assert_eq!(
        doc.map("gifts")
            .unwrap()
            .values()
            .filter(|v| v.as_i64().unwrap_or(0) > 0)
            .count(),
        3
    );
    assert!(
        doc.text("ban")
            .unwrap_or("")
            .to_lowercase()
            .contains("honour"),
        "Silver Fang ban derived onto the sheet"
    );
}

/// Gift and Rage XP spends route through the splat's cost hooks and apply on
/// approval.
#[test]
fn werewolf_gift_xp_spend_applies_on_approval() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=werewolf");
    set(&mut tw, "@sheet/set Nick/gifts:Razor Claws=1");
    tw.run(GOD, "+xp/award Nick=20");
    // Raise a Gift 1→2: new level 2 × 3 = 6 XP.
    let ev = tw.run(nick, "+xp/spend gifts:Razor Claws=2");
    assert!(
        tw.seen_by(&ev, nick).iter().any(|l| l.contains("for 6 XP")),
        "gift level 2 is 6 XP"
    );
    tw.run(GOD, "+job/approve 1");
    let doc = read_sheet(&tw, nick);
    assert_eq!(
        doc.map("gifts").unwrap()["Razor Claws"],
        serde_json::json!(2)
    );
    assert_eq!(doc.number("experience"), Some(14)); // 20 - 6
}

// ----------------------------------------------------------------------
// M6b: Mage (M20) on the same per-template dispatch
// ----------------------------------------------------------------------

/// A mage's active schema carries Spheres / Arete / Quintessence / Paradox; a
/// mortal's carries none of them, and a mage has no Vampire fields.
#[test]
fn mage_template_selects_the_active_schema() {
    let ruleset = WodRuleset::with_seed(S);
    let base = ruleset.sheet_schema().clone();

    let mut mage = SheetDoc::new_default(&base);
    mage.set_text("template", "mage");
    let msch = ruleset.schema_for(&mage);
    assert!(msch.field("spheres").is_some());
    assert!(msch.field("arete").is_some());
    assert!(msch.field("quintessence").is_some());
    assert!(msch.field("paradox").is_some());
    assert!(msch.field("affiliation").is_some());
    assert!(msch.field("disciplines").is_none(), "a mage is not Kindred");
    assert!(msch.field("bloodpool").is_none());

    let mortal = SheetDoc::new_default(&base);
    let mo = ruleset.schema_for(&mortal);
    assert!(mo.field("spheres").is_none());
    assert!(mo.field("arete").is_none());
}

/// Mage splat commands are always registered but self-gate: a mortal is refused.
#[test]
fn mage_commands_refuse_non_mage() {
    let (mut tw, nick) = world(S);
    for cmd in [
        "+sphere",
        "+arete",
        "+cast",
        "+quint",
        "+paradox",
        "+resonance",
    ] {
        let ev = tw.run(nick, cmd);
        assert!(
            tw.seen_by(&ev, nick)
                .iter()
                .any(|l| l.contains("not Awakened")),
            "{cmd} must refuse a mortal"
        );
    }
}

/// `+sphere` refuses a rating above the caster's Arete; raising Arete lifts the
/// cap. Direct `+sphere` sets are a chargen (or staff) action, so this drives a
/// chargen session; post-approval, Spheres rise only through `+xp/spend`.
#[test]
fn sphere_rating_is_capped_by_arete() {
    let (mut tw, nick) = world(S);
    tw.run(nick, "+chargen");
    tw.run(nick, "+chargen/template mage"); // Arete defaults to 1
    let ev = tw.run(nick, "+sphere Forces=2");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("cannot exceed your Arete")),
        "a Sphere above Arete is refused"
    );
    // At Arete 1 a single dot is fine.
    tw.run(nick, "+sphere Forces=1");
    assert_eq!(
        read_sheet(&tw, nick).map("spheres").unwrap()["Forces"],
        serde_json::json!(1)
    );
    // Raise Arete to 3 (staff bypass of the chargen lock), then a 3-dot Sphere
    // is allowed but a 4-dot is not.
    set(&mut tw, "@sheet/set Nick/arete=3");
    tw.run(nick, "+sphere Forces=3");
    assert_eq!(
        read_sheet(&tw, nick).map("spheres").unwrap()["Forces"],
        serde_json::json!(3)
    );
    let ev = tw.run(nick, "+sphere Forces=4");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("cannot exceed your Arete")),
    );
}

/// Once the sheet is approved (out of chargen), `+sphere` refuses a direct set
/// and points at the costed XP path — it is not a free way to raise Spheres.
#[test]
fn sphere_set_is_refused_after_chargen() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=mage");
    set(&mut tw, "@sheet/set Nick/arete=3"); // room under the Arete cap
    let ev = tw.run(nick, "+sphere Forces=2");
    let seen = tw.seen_by(&ev, nick);
    assert!(
        seen.iter().any(|l| l.contains("+xp/spend")),
        "a within-Arete set outside chargen is refused and points at XP: {seen:?}"
    );
    // The Sphere was not written.
    assert!(
        read_sheet(&tw, nick)
            .map("spheres")
            .and_then(|m| m.get("Forces"))
            .is_none()
    );
}

/// `+cast` rolls the Arete pool at the given difficulty (seeded/deterministic).
#[test]
fn cast_rolls_the_arete_pool() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=mage");
    set(&mut tw, "@sheet/set Nick/arete=4");
    let ev = tw.run(nick, "+cast Hurl a bolt of fire vs 7");
    let out = last_outcome(&tw, nick);
    assert_eq!(out.dice.len(), 4, "Arete 4 → 4 dice");
    assert_eq!(out.difficulty, 7);
    let seen = tw.seen_by(&ev, nick);
    assert!(seen.iter().any(|l| l.contains("magick")));
    // /vulgar flags Paradox risk.
    let ev = tw.run(nick, "+cast/vulgar Warp gravity vs 6");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.to_lowercase().contains("paradox"))
    );
}

/// Quintessence gains are capped at the Avatar rating; spends are bounded below.
#[test]
fn quintessence_is_avatar_bounded() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=mage");
    set(&mut tw, "@sheet/set Nick/backgrounds:Avatar=3");
    // Channelling caps at the Avatar rating (3).
    tw.run(nick, "+quint/gain 5");
    assert_eq!(read_sheet(&tw, nick).number("quintessence"), Some(3));
    tw.run(nick, "+quint/spend 2");
    assert_eq!(read_sheet(&tw, nick).number("quintessence"), Some(1));
    // Cannot overspend.
    let ev = tw.run(nick, "+quint/spend 5");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("only 1 Quintessence"))
    );
    assert_eq!(read_sheet(&tw, nick).number("quintessence"), Some(1));
}

/// Paradox accrues and is worked off, bounded at zero.
#[test]
fn paradox_accrues_and_burns_off() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=mage");
    tw.run(nick, "+paradox/gain 3");
    assert_eq!(read_sheet(&tw, nick).number("paradox"), Some(3));
    // Burning off more than held floors at zero.
    tw.run(nick, "+paradox/spend 5");
    assert_eq!(read_sheet(&tw, nick).number("paradox"), Some(0));
}

/// Drive a full, budget-balancing Order of Hermes chargen.
fn do_mage_chargen(tw: &mut TestWorld, who: ObjId) {
    tw.run(who, "+chargen");
    tw.run(who, "+chargen/template mage");
    tw.run(who, "+chargen/attr physical=7 social=5 mental=3");
    tw.run(who, "+chargen/abil talents=13 skills=9 knowledges=5");
    tw.run(who, "+stat affiliation=order_of_hermes");
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
        tw.run(who, c);
    }
    for c in [
        "+stat alertness=3",
        "+stat athletics=3",
        "+stat brawl=3",
        "+stat empathy=2",
        "+stat expression=2",
    ] {
        tw.run(who, c);
    }
    for c in ["+stat drive=3", "+stat firearms=3", "+stat stealth=3"] {
        tw.run(who, c);
    }
    for c in ["+stat academics=3", "+stat computer=2"] {
        tw.run(who, c);
    }
    // Backgrounds 5: Avatar 2 (starting Quintessence), Resources 3.
    tw.run(who, "+stat backgrounds:Avatar=2");
    tw.run(who, "+stat backgrounds:Resources=3");
    // Spheres: 5 dots at Arete 1 (one in the affinity Sphere, Forces).
    for c in [
        "+sphere Forces=1",
        "+sphere Correspondence=1",
        "+sphere Life=1",
        "+sphere Mind=1",
        "+sphere Prime=1",
    ] {
        tw.run(who, c);
    }
}

#[test]
fn mage_full_chargen_derives_pools_and_approves() {
    let (mut tw, nick) = world(S);
    do_mage_chargen(&mut tw, nick);

    // Budget (incl. the Spheres pool) balances.
    let ev = tw.run(nick, "+chargen/status");
    let seen = tw.seen_by(&ev, nick);
    assert!(
        seen.iter().any(|l| l.contains("Budget balances")),
        "mage budget must balance: {seen:?}"
    );
    assert!(
        seen.iter()
            .any(|l| l.contains("Spheres") && l.contains("5/5"))
    );

    // Submit → Pending → staff approve → Approved.
    tw.run(nick, "+chargen/submit");
    let job = match read_sheet(&tw, nick).approved {
        ApprovalState::Pending(j) => j,
        other => panic!("expected Pending, got {other:?}"),
    };
    tw.run(
        GOD,
        &format!("+job/approve {job}=The Awakened world welcomes you."),
    );
    let doc = read_sheet(&tw, nick);
    assert!(doc.approved.is_approved());
    assert_eq!(doc.text("template"), Some("mage"));
    // Derived starting values.
    assert_eq!(
        doc.number("willpower"),
        Some(5),
        "mage Willpower is a flat 5"
    );
    assert_eq!(
        doc.number("quintessence"),
        Some(2),
        "Quintessence = Avatar rating"
    );
    assert_eq!(doc.number("arete"), Some(1));
    assert_eq!(
        doc.text("affinity"),
        Some("Forces"),
        "Order of Hermes → Forces"
    );
}

#[test]
fn sphere_xp_costs_affinity_vs_other() {
    let (mut tw, nick) = world(S);
    set(&mut tw, "@sheet/set Nick/template=mage");
    set(&mut tw, "@sheet/set Nick/affiliation=order_of_hermes"); // affinity Forces
    set(&mut tw, "@sheet/set Nick/arete=5"); // Arete cap: room to raise Spheres to 3
    set(&mut tw, "@sheet/set Nick/spheres:Forces=2"); // affinity
    set(&mut tw, "@sheet/set Nick/spheres:Mind=2"); // other
    tw.run(GOD, "+xp/award Nick=50");
    // Affinity Forces 2→3: current 2 × 7 = 14 XP.
    let ev = tw.run(nick, "+xp/spend spheres:Forces=3");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("for 14 XP")),
        "affinity Sphere is ×7"
    );
    // Other Mind 2→3: current 2 × 8 = 16 XP.
    let ev = tw.run(nick, "+xp/spend spheres:Mind=3");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("for 16 XP")),
        "out-of-affinity Sphere is ×8"
    );
    // Approve the affinity raise (job 1): applies atomically.
    tw.run(GOD, "+job/approve 1");
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.map("spheres").unwrap()["Forces"], serde_json::json!(3));
    assert_eq!(doc.number("experience"), Some(36)); // 50 - 14
}

// ----------------------------------------------------------------------
// M8: NPC agent grounding — splat-aware agent_profile + the wod_roll ability
// ----------------------------------------------------------------------

/// A splat sheet built with raw setters (agent_profile only reads it; the
/// `template` field selects the active schema inside the trait method).
fn splat_doc(rs: &WodRuleset, template: &str) -> SheetDoc {
    let mut d = SheetDoc::new_default(rs.sheet_schema());
    d.set_text("template", template);
    d
}

#[test]
fn agent_profile_grounds_a_vampire() {
    let rs = WodRuleset::with_seed(S);
    let mut d = splat_doc(&rs, "vampire");
    d.set_number("strength", 4);
    d.set_number("brawl", 3);
    d.set_number("intimidation", 3);
    d.set_text("clan", "brujah");
    d.set_number("generation", 10);
    d.map_set("disciplines", "Celerity", serde_json::json!(2));
    d.map_set("disciplines", "Potence", serde_json::json!(1));
    d.set_number("bloodpool", 8);
    d.set_number("humanity", 6);
    d.set_number("willpower", 5);

    let p = rs.agent_profile(&d);
    // Splat-aware prose: clan, disciplines, pools.
    assert!(p.summary.contains("Brujah"), "summary: {}", p.summary);
    assert!(p.summary.contains("Celerity 2"), "summary: {}", p.summary);
    assert!(p.summary.contains("Blood pool 8"), "summary: {}", p.summary);
    // Tags: splat + clan + a domain read.
    assert!(p.tags.iter().any(|t| t == "vampire:brujah"));
    assert!(p.tags.iter().any(|t| t == "splat:vampire"));
    assert!(p.tags.iter().any(|t| t == "combat:brawl"));
    // Trimmed stats carry the powers.
    assert_eq!(p.stats["powers"]["clan"], serde_json::json!("brujah"));
    assert_eq!(
        p.stats["powers"]["disciplines"]["Celerity"],
        serde_json::json!(2)
    );
    assert_eq!(p.stats["powers"]["bloodpool"], serde_json::json!(8));
    // The stable tool manifest.
    assert_eq!(p.tools.len(), 1);
    assert_eq!(p.tools[0].name.as_str(), "wod_roll");
    assert_eq!(
        p.tools[0].params_schema["required"],
        serde_json::json!(["pool_expr"])
    );
}

#[test]
fn agent_profile_grounds_a_mage() {
    let rs = WodRuleset::with_seed(S);
    let mut d = splat_doc(&rs, "mage");
    d.set_text("affiliation", "order_of_hermes");
    d.set_number("arete", 3);
    d.map_set("spheres", "Forces", serde_json::json!(3));
    d.map_set("spheres", "Prime", serde_json::json!(2));
    d.set_number("quintessence", 5);
    d.set_number("paradox", 1);

    let p = rs.agent_profile(&d);
    assert!(p.summary.contains("Arete 3"), "summary: {}", p.summary);
    assert!(p.summary.contains("Forces 3"), "summary: {}", p.summary);
    assert!(p.tags.iter().any(|t| t == "splat:mage"));
    assert!(p.tags.iter().any(|t| t == "mage:order_of_hermes"));
    assert_eq!(p.stats["powers"]["arete"], serde_json::json!(3));
    assert_eq!(p.stats["powers"]["spheres"]["Forces"], serde_json::json!(3));
}

#[test]
fn agent_profile_grounds_a_werewolf() {
    let rs = WodRuleset::with_seed(S);
    let mut d = splat_doc(&rs, "werewolf");
    d.set_text("breed", "homid");
    d.set_text("auspice", "ahroun");
    d.set_text("tribe", "get_of_fenris");
    d.set_number("rank", 2);
    d.set_number("rage", 5);
    d.set_number("gnosis", 3);
    d.set_number("glory_perm", 2);

    let p = rs.agent_profile(&d);
    assert!(p.summary.contains("Rage 5"), "summary: {}", p.summary);
    assert!(p.summary.contains("Gnosis 3"), "summary: {}", p.summary);
    assert!(
        p.summary.contains("Get Of Fenris"),
        "summary: {}",
        p.summary
    );
    assert!(p.tags.iter().any(|t| t == "splat:werewolf"));
    assert!(p.tags.iter().any(|t| t == "werewolf:get_of_fenris"));
    assert_eq!(p.stats["powers"]["rage"], serde_json::json!(5));
    assert_eq!(p.stats["powers"]["renown"]["glory"], serde_json::json!(2));
}

/// Invoke the `wod_roll` ability against `npc` on `tw`'s world, returning the
/// structured outcome and the events it emitted.
fn wod_roll(
    tw: &mut TestWorld,
    npc: ObjId,
    input: serde_json::Value,
) -> Result<
    (
        mush_ruleset::AbilityOutcome,
        Vec<(mush_core::Audience, mush_proto::OutputEvent)>,
    ),
    mush_ruleset::AbilityError,
> {
    // A fresh ruleset instance only supplies the (default) schema dispatch; the
    // roll consumes the *world's* WodState RNG, so it stays deterministic.
    let rs = WodRuleset::with_seed(S);
    let mut out = mush_core::Outbox::new();
    let outcome = rs.invoke_ability(tw.world_mut(), &mut out, npc, "wod_roll", input)?;
    Ok((outcome, out.events().to_vec()))
}

#[test]
fn wod_roll_ability_rolls_from_sheet_logs_and_emits() {
    let (mut tw, _nick) = world(S);
    let bart = tw.spawn_player("Bartender");
    set(&mut tw, "@sheet/set Bartender/strength=3");
    set(&mut tw, "@sheet/set Bartender/brawl=2");

    let (outcome, events) = wod_roll(
        &mut tw,
        bart,
        serde_json::json!({ "pool_expr": "strength+brawl" }),
    )
    .expect("wod_roll resolves");

    // Uses the server RNG: matches a direct Owod20 roll on the same seed (pool
    // 5, difficulty 6) — the model cannot fabricate the dice.
    let expected = Owod20Pool::score(&faces(S, 0, 5), &Owod20Opts::new(6));
    assert_eq!(
        last_outcome(&tw, bart),
        expected,
        "the ability roll must match a direct Owod20Pool roll"
    );

    // Structured result exposes the polarity mush-npc's validator checks.
    assert_eq!(outcome.ok, expected.successes() > 0);
    let expected_polarity = if expected.is_botch() {
        "botch"
    } else if expected.successes() > 0 {
        "success"
    } else {
        "failure"
    };
    assert_eq!(
        outcome.data["polarity"],
        serde_json::json!(expected_polarity)
    );
    assert_eq!(outcome.data["difficulty"], serde_json::json!(6));
    assert_eq!(outcome.data["pool_size"], serde_json::json!(5));
    assert_eq!(
        outcome.data["successes"],
        serde_json::json!(expected.successes())
    );

    // Broadcast to the room like a player roll, plus a wod.roll panel keyframe.
    assert!(
        events
            .iter()
            .any(|(_, ev)| mush_testkit::text_of(ev).contains("Bartender rolls")),
        "the room must see the NPC's roll"
    );
    assert!(
        events.iter().any(|(_, ev)| matches!(
            ev,
            mush_proto::OutputEvent::Panel { feed, .. } if feed.as_str() == "wod.roll"
        )),
        "a wod.roll panel keyframe must be emitted"
    );
}

/// V4: the NPC power path routes through the SAME director. A Discipline pool
/// is projected per-observer (no room broadcast, mundane onlookers gated out);
/// a mundane pool still broadcasts to the room like a player roll.
#[test]
fn wod_roll_power_pool_routes_through_the_director() {
    let (mut tw, _nick) = world(S);
    let vamp = tw.spawn_player("Vamp");
    set(&mut tw, "@sheet/set Vamp/template=vampire");
    set(&mut tw, "@sheet/set Vamp/strength=3");
    set(&mut tw, "@sheet/set Vamp/brawl=2");
    set(&mut tw, "@sheet/set Vamp/manipulation=3");
    set(&mut tw, "@sheet/set Vamp/disciplines:Dominate=3");
    let rube = tw.spawn_player("Onlooker"); // mundane: no Auspex

    let has_room_line = |events: &[(mush_core::Audience, mush_proto::OutputEvent)]| {
        events.iter().any(|(aud, ev)| {
            matches!(aud, mush_core::Audience::Room { .. })
                && mush_testkit::text_of(ev).contains("rolls")
        })
    };

    // A subtle Discipline: no room broadcast, and the mundane onlooker is gated
    // out before any witness roll (required sense: Auspex).
    let (_o, ev_power) = wod_roll(
        &mut tw,
        vamp,
        serde_json::json!({ "pool_expr": "dominate+manipulation" }),
    )
    .expect("power roll resolves");
    assert!(
        !has_room_line(&ev_power),
        "an NPC's subtle power must not broadcast to the room: {ev_power:?}"
    );
    assert!(
        tw.world()
            .module_state::<WodState>()
            .unwrap()
            .recent_logs(rube, 1)
            .is_empty(),
        "a senseless onlooker advances no RNG on the NPC power path"
    );

    // A mundane pool still broadcasts to the room, exactly like a player roll.
    let (_o2, ev_mundane) = wod_roll(
        &mut tw,
        vamp,
        serde_json::json!({ "pool_expr": "strength+brawl" }),
    )
    .expect("mundane roll resolves");
    assert!(
        has_room_line(&ev_mundane),
        "a mundane NPC roll broadcasts to the room: {ev_mundane:?}"
    );
}

/// M14 D6 / V5 (NPC-actor branch): a POWER pool rolled by the NPC seam
/// announces the director-resolved NPC witness set back to the caller via
/// `data.power_use` (so `mush-npc` can emit `WorldEvent::PowerUsed` and file
/// intel). An Auspex-bearing NPC witness is included; a mundane NPC (gated out
/// of a subtle, Auspex-only Discipline) is not. A mundane pool announces nothing.
#[test]
fn wod_roll_power_pool_announces_npc_witnesses() {
    let (mut tw, _nick) = world(S);
    let vamp = tw.spawn_player("Vamp");
    set(&mut tw, "@sheet/set Vamp/template=vampire");
    set(&mut tw, "@sheet/set Vamp/strength=3");
    set(&mut tw, "@sheet/set Vamp/brawl=2");
    set(&mut tw, "@sheet/set Vamp/manipulation=3");
    set(&mut tw, "@sheet/set Vamp/disciplines:Dominate=3");

    // Two NPC onlookers co-located with the actor: one Auspex-bearing (a director
    // winner against subtle Dominate), one mundane (gated out before any roll).
    let room = tw.world().get(vamp).and_then(|o| o.location).expect("room");
    let seer = tw
        .world_mut()
        .create(mush_core::ObjKind::Npc, "Seeress", GOD, Some(room))
        .expect("npc seer");
    set(&mut tw, "@sheet/set Seeress/template=vampire");
    set(&mut tw, "@sheet/set Seeress/perception=5");
    set(&mut tw, "@sheet/set Seeress/awareness=5");
    set(&mut tw, "@sheet/set Seeress/disciplines:Auspex=5");
    let dolt = tw
        .world_mut()
        .create(mush_core::ObjKind::Npc, "Dolt", GOD, Some(room))
        .expect("npc dolt");
    set(&mut tw, "@sheet/set Dolt/perception=5");
    set(&mut tw, "@sheet/set Dolt/awareness=5");

    // A subtle Discipline: the announcement names the Auspex NPC, never the mundane one.
    let (outcome, _ev) = wod_roll(
        &mut tw,
        vamp,
        serde_json::json!({ "pool_expr": "dominate+manipulation" }),
    )
    .expect("power roll resolves");
    let power_use = outcome
        .data
        .get("power_use")
        .expect("a power pool announces power_use");
    let witnesses: Vec<u32> =
        serde_json::from_value(power_use["witnesses"].clone()).expect("witness list");
    assert!(
        witnesses.contains(&seer.0),
        "the Auspex NPC is a director winner: {witnesses:?}"
    );
    assert!(
        !witnesses.contains(&dolt.0),
        "the mundane NPC (no Auspex) is not a witness of a subtle power: {witnesses:?}"
    );
    assert!(
        power_use["detail"]
            .as_str()
            .is_some_and(|d| d.contains("Dominate")),
        "the detail names the power: {power_use}"
    );

    // A mundane pool routes to the room, not the director — no announcement.
    let (mundane, _ev2) = wod_roll(
        &mut tw,
        vamp,
        serde_json::json!({ "pool_expr": "strength+brawl" }),
    )
    .expect("mundane roll resolves");
    assert!(
        mundane.data.get("power_use").is_none(),
        "a mundane NPC roll announces no power use: {}",
        mundane.data
    );
}

#[test]
fn wod_roll_rejects_over_claimed_pools() {
    let (mut tw, _nick) = world(S);
    let bart = tw.spawn_player("Bartender");
    set(&mut tw, "@sheet/set Bartender/strength=3");
    set(&mut tw, "@sheet/set Bartender/brawl=2"); // Brawl 2 — cannot become 9.

    // A raw-dice injection ("+7") is refused: the pool may only name sheet traits.
    assert!(matches!(
        wod_roll(
            &mut tw,
            bart,
            serde_json::json!({ "pool_expr": "strength+brawl+7" })
        ),
        Err(mush_ruleset::AbilityError::Invalid(_))
    ));
    // A bare number is refused.
    assert!(matches!(
        wod_roll(&mut tw, bart, serde_json::json!({ "pool_expr": "9" })),
        Err(mush_ruleset::AbilityError::Invalid(_))
    ));
    // A named-value bonus is refused.
    assert!(matches!(
        wod_roll(&mut tw, bart, serde_json::json!({ "pool_expr": "brawl:9" })),
        Err(mush_ruleset::AbilityError::Invalid(_))
    ));
    // A power the mortal does not have is refused (no such trait on the sheet).
    assert!(matches!(
        wod_roll(
            &mut tw,
            bart,
            serde_json::json!({ "pool_expr": "celerity+dexterity" })
        ),
        Err(mush_ruleset::AbilityError::Invalid(_))
    ));
    // An unknown ability name is rejected distinctly.
    let rs = WodRuleset::with_seed(S);
    let mut out = mush_core::Outbox::new();
    assert!(matches!(
        rs.invoke_ability(
            tw.world_mut(),
            &mut out,
            bart,
            "teleport",
            serde_json::json!({})
        ),
        Err(mush_ruleset::AbilityError::Unknown(_))
    ));
    // None of the refusals logged a roll (the RNG stream is untouched).
    assert!(
        tw.world()
            .module_state::<WodState>()
            .unwrap()
            .recent_logs(bart, 1)
            .is_empty()
    );
}

#[test]
fn wod_roll_applies_wound_penalty() {
    let (mut tw, _nick) = world(S);
    let bart = tw.spawn_player("Bartender");
    set(&mut tw, "@sheet/set Bartender/strength=3");
    set(&mut tw, "@sheet/set Bartender/brawl=2");
    // 4 lethal → Wounded (-2). Pool 5 - 2 = 3.
    tw.run(GOD, "+harm Bartender=4L");

    let (outcome, _events) = wod_roll(
        &mut tw,
        bart,
        serde_json::json!({ "pool_expr": "strength+brawl" }),
    )
    .expect("wod_roll resolves");
    let expected = Owod20Pool::score(&faces(S, 0, 3), &Owod20Opts::new(6));
    assert_eq!(last_outcome(&tw, bart), expected);
    assert_eq!(outcome.data["pool_size"], serde_json::json!(3));
    assert_eq!(outcome.data["wound_penalty"], serde_json::json!(-2));
}

#[test]
fn wod_roll_willpower_spend_is_bounded_to_the_npc_sheet() {
    let (mut tw, _nick) = world(S);
    let bart = tw.spawn_player("Bartender");
    set(&mut tw, "@sheet/set Bartender/strength=3");
    set(&mut tw, "@sheet/set Bartender/willpower=5");
    set(&mut tw, "@sheet/set Bartender/willpower_temp=1");

    // First spend: honored — matches a direct roll with the willpower option,
    // and deducts the temporary point from the NPC's own sheet.
    let (outcome, _) = wod_roll(
        &mut tw,
        bart,
        serde_json::json!({ "pool_expr": "strength", "willpower": true }),
    )
    .expect("wod_roll resolves");
    let expected = Owod20Pool::score(
        &faces(S, 0, 3),
        &Owod20Opts {
            willpower: true,
            ..Owod20Opts::new(6)
        },
    );
    assert_eq!(last_outcome(&tw, bart), expected);
    assert_eq!(outcome.data["willpower_applied"], serde_json::json!(true));
    assert_eq!(read_sheet(&tw, bart).number("willpower_temp"), Some(0));

    // Second spend: no temporary point left, so the request is silently not
    // honored (the model cannot conjure an automatic success it hasn't got).
    let (outcome, _) = wod_roll(
        &mut tw,
        bart,
        serde_json::json!({ "pool_expr": "strength", "willpower": true }),
    )
    .expect("wod_roll resolves");
    assert_eq!(outcome.data["willpower_applied"], serde_json::json!(false));
}
