//! 16b cut R-b (R4): the one game sheet, end to end through the real engine.
//!
//! Everything here runs the actual command surface — `+chargen`, `+stat`,
//! `+chargen/submit`, `+job/approve`, `+roll`, `+sheet` — over
//! [`mush_testkit::TestWorld`] with the jobs module loaded, because the approval
//! hop is the part worth proving and it crosses two modules.
//!
//! The claims, in order:
//!
//! 1. a fresh sheet is a green pilot — every rating at 1, and [`PilotProfile`]
//!    reads it back;
//! 2. `+stat` spends the pool and refuses the overspend at the keystroke;
//! 3. `+chargen/submit` files a job, `+job/approve` stamps the sheet and lifts
//!    the onboarding gate onto the live start room;
//! 4. while `chargen_required` is on, an exit and an unlisted verb are both
//!    refused — including an exit aliased like an allowlisted verb;
//! 5. `+roll 2d6` rolls two six-siders and shows the room;
//! 6. the one NPC ability, `check`, resolves against the sheet and refuses
//!    everything else.

use mush_core::{GOD, LIMBO, ObjId, ObjKind, Outbox};
use mush_jobs::JobsModule;
use mush_ruleset::{ApprovalState, Ruleset, SheetDoc};
use mush_testkit::TestWorld;
use tacsim::sheet::schema::{
    FIELD_CALLSIGN, FIELD_GUNNERY, FIELD_NAME, FIELD_PILOTING, FIELD_SENSORS, RATING_MIN, RATINGS,
};
use tacsim::sheet::{PilotProfile, PilotRuleset, agent};

/// A fixed dice seed, so `+roll` and `check` are reproducible.
const SEED: u64 = 4242;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

/// A world with onboarding off — the default shape.
fn plain_world() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(PilotRuleset::with_seed(SEED)),
    ])
}

/// A world with `chargen_required` on and both rooms = Limbo.
fn gated_world() -> TestWorld {
    let cfg = mush_core::OnboardingConfig {
        chargen_required: true,
        start_room: LIMBO,
        start_room_unapproved: LIMBO,
    };
    TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(PilotRuleset::with_seed(SEED).onboarding(cfg)),
    ])
}

/// The stored document, decoded from the attribute the store actually wrote.
fn read_sheet(tw: &TestWorld, who: ObjId) -> SheetDoc {
    let attr = tw
        .world()
        .attr(who, "sheet:tacsim")
        .and_then(mush_core::Attr::as_json)
        .expect("the sheet lives at sheet:tacsim");
    serde_json::from_value(attr.clone()).expect("the sheet decodes")
}

fn saw(
    tw: &TestWorld,
    ev: &[(mush_core::Audience, mush_proto::OutputEvent)],
    who: ObjId,
    needle: &str,
) -> bool {
    tw.seen_by(ev, who).iter().any(|line| line.contains(needle))
}

/// Whether the onboarding gate refused this line.
fn refused(
    tw: &TestWorld,
    ev: &[(mush_core::Audience, mush_proto::OutputEvent)],
    who: ObjId,
) -> bool {
    saw(tw, ev, who, "must finish creating")
}

/// Spend the six points exactly and name the pilot: gunnery 4, piloting 3,
/// sensors 2 is `3 + 2 + 1 == 6`.
fn build_a_balanced_pilot(tw: &mut TestWorld, who: ObjId) {
    tw.run(who, "+chargen");
    tw.run(who, "+stat name=Ada Renard");
    tw.run(who, "+stat callsign=Ladder");
    tw.run(who, "+stat gunnery=4");
    tw.run(who, "+stat piloting=3");
    tw.run(who, "+stat sensors=2");
}

// ----------------------------------------------------------------------
// 1. A fresh sheet is a green pilot
// ----------------------------------------------------------------------

#[test]
fn a_fresh_sheet_starts_every_rating_at_one() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    tw.run(nick, "+chargen");

    let doc = read_sheet(&tw, nick);
    for rating in RATINGS {
        assert_eq!(
            doc.number(rating),
            Some(i64::from(RATING_MIN)),
            "{rating} starts at {RATING_MIN}"
        );
    }
    assert_eq!(doc.approved, ApprovalState::Unapproved);
    assert_eq!(
        PilotProfile::from_sheet(&doc),
        PilotProfile::default(),
        "the profile seam reads a default sheet as a green pilot"
    );
}

#[test]
fn the_profile_seam_reads_the_ratings_a_player_bought() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    build_a_balanced_pilot(&mut tw, nick);

    let profile = PilotProfile::from_sheet(&read_sheet(&tw, nick));
    assert_eq!(
        profile,
        PilotProfile {
            gunnery: 4,
            piloting: 3,
            sensors: 2,
        }
    );
}

#[test]
fn the_sheet_renders_without_a_health_track_or_a_pool() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    build_a_balanced_pilot(&mut tw, nick);

    let ev = tw.run(nick, "+sheet");
    let lines = tw.seen_by(&ev, nick).join("\n");
    assert!(lines.contains("Gunnery"), "{lines}");
    assert!(lines.contains("Ladder"), "{lines}");
    assert!(
        !lines.to_lowercase().contains("health"),
        "this sheet has no health track: {lines}"
    );
}

// ----------------------------------------------------------------------
// 2. The budget
// ----------------------------------------------------------------------

#[test]
fn stat_spends_within_the_budget() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    tw.run(nick, "+chargen");

    let ev = tw.run(nick, "+stat gunnery=4");
    assert!(
        saw(&tw, &ev, nick, "Set gunnery = 4"),
        "{:?}",
        tw.seen_by(&ev, nick)
    );
    assert!(
        saw(&tw, &ev, nick, "3 of 6 spent"),
        "the live ledger follows the keystroke: {:?}",
        tw.seen_by(&ev, nick)
    );
    assert_eq!(read_sheet(&tw, nick).number(FIELD_GUNNERY), Some(4));
}

#[test]
fn stat_refuses_an_overspend_and_writes_nothing() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    tw.run(nick, "+chargen");
    tw.run(nick, "+stat gunnery=5"); // 4 of 6
    tw.run(nick, "+stat piloting=3"); // 6 of 6 — exactly spent

    let ev = tw.run(nick, "+stat sensors=2");
    assert!(
        saw(&tw, &ev, nick, "Not enough points"),
        "the seventh point is refused: {:?}",
        tw.seen_by(&ev, nick)
    );
    assert_eq!(
        read_sheet(&tw, nick).number(FIELD_SENSORS),
        Some(i64::from(RATING_MIN)),
        "a refused spend leaves the sheet untouched"
    );
}

#[test]
fn stat_refuses_a_staff_only_field_and_a_field_that_is_not_on_the_sheet() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    tw.run(nick, "+chargen");

    // Canonicalised before the gate reads it: the shouted spelling is refused too.
    let ev = tw.run(nick, "+stat REPUTATION=5");
    assert!(
        saw(&tw, &ev, nick, "set by staff"),
        "{:?}",
        tw.seen_by(&ev, nick)
    );
    assert_eq!(read_sheet(&tw, nick).number("reputation"), Some(0));

    let ev = tw.run(nick, "+stat willpower=5");
    assert!(
        saw(&tw, &ev, nick, "No such sheet field"),
        "{:?}",
        tw.seen_by(&ev, nick)
    );
}

#[test]
fn status_reports_the_pool_and_conjures_no_sheet() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");

    let ev = tw.run(nick, "+chargen/status");
    assert!(
        saw(&tw, &ev, nick, "not started a character"),
        "{:?}",
        tw.seen_by(&ev, nick)
    );
    assert!(
        tw.world().attr(nick, "sheet:tacsim").is_none(),
        "asking for the budget does not write a sheet"
    );

    tw.run(nick, "+chargen");
    let ev = tw.run(nick, "+chargen/status");
    assert!(
        saw(&tw, &ev, nick, "0 of 6 spent"),
        "{:?}",
        tw.seen_by(&ev, nick)
    );
}

#[test]
fn submit_refuses_an_unbalanced_sheet() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    tw.run(nick, "+chargen");

    let ev = tw.run(nick, "+chargen/submit");
    assert!(
        saw(&tw, &ev, nick, "Cannot submit"),
        "an unspent pool blocks the filing: {:?}",
        tw.seen_by(&ev, nick)
    );
    assert_eq!(read_sheet(&tw, nick).approved, ApprovalState::Unapproved);
}

// ----------------------------------------------------------------------
// 3. Submit, approve, move
// ----------------------------------------------------------------------

#[test]
fn submit_files_a_job_and_locks_the_sheet() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    build_a_balanced_pilot(&mut tw, nick);

    let ev = tw.run(nick, "+chargen/submit");
    assert!(
        saw(&tw, &ev, nick, "Submitted for approval"),
        "{:?}",
        tw.seen_by(&ev, nick)
    );
    let job = match read_sheet(&tw, nick).approved {
        ApprovalState::Pending(job) => job,
        other => panic!("expected Pending, got {other:?}"),
    };

    // The queue is this module's, and the ticket is in it.
    let ev = tw.run(GOD, "+jobs pilot-chargen");
    assert!(
        saw(&tw, &ev, GOD, "Pilot approval: Nick"),
        "{:?}",
        tw.seen_by(&ev, GOD)
    );

    // A Pending sheet is locked against further edits.
    let ev = tw.run(nick, "+stat gunnery=5");
    assert!(
        saw(&tw, &ev, nick, "awaiting approval"),
        "{:?}",
        tw.seen_by(&ev, nick)
    );
    assert_eq!(read_sheet(&tw, nick).number(FIELD_GUNNERY), Some(4));
    assert!(job > 0);
}

#[test]
fn approval_stamps_the_sheet_lifts_the_gate_and_moves_to_the_start_room() {
    // A distinct live start room; the chargen area is Limbo.
    let cfg = mush_core::OnboardingConfig {
        chargen_required: true,
        start_room: ObjId(2),
        start_room_unapproved: LIMBO,
    };
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(PilotRuleset::with_seed(SEED).onboarding(cfg)),
    ]);
    // The first object created after boot is #2 — the hangar deck.
    let hangar = tw
        .world_mut()
        .create(ObjKind::Room, "Hangar Deck", GOD, None)
        .expect("the start room is created");
    assert_eq!(hangar, ObjId(2), "start-room id assumption");

    let nick = tw.spawn_player("Nick");
    assert_eq!(
        tw.world().get(nick).and_then(|o| o.location),
        Some(LIMBO),
        "a new unapproved player lands in the chargen area"
    );

    build_a_balanced_pilot(&mut tw, nick);
    tw.run(nick, "+chargen/submit");
    let job = match read_sheet(&tw, nick).approved {
        ApprovalState::Pending(job) => job,
        other => panic!("expected Pending, got {other:?}"),
    };

    // Still gated while pending.
    let ev = tw.run(nick, "+roll 2d6");
    assert!(refused(&tw, &ev, nick), "still gated while pending");

    let ev = tw.run(GOD, &format!("+job/approve {job}=Welcome aboard."));
    assert!(saw(&tw, &ev, GOD, "approved"), "{:?}", tw.seen_by(&ev, GOD));

    let doc = read_sheet(&tw, nick);
    assert!(
        matches!(doc.approved, ApprovalState::Approved { by, .. } if by == GOD),
        "stamped by the approving staffer: {:?}",
        doc.approved
    );
    assert_eq!(
        tw.world().get(nick).and_then(|o| o.location),
        Some(hangar),
        "moved to the live start room"
    );
    // The gate lifts on its own: the sheet is Approved, nothing is un-wired.
    let ev = tw.run(nick, "+roll 2d6");
    assert!(!refused(&tw, &ev, nick), "gate lifted after approval");
}

#[test]
fn approval_refuses_a_sheet_that_no_longer_balances() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    build_a_balanced_pilot(&mut tw, nick);
    tw.run(nick, "+chargen/submit");
    let job = match read_sheet(&tw, nick).approved {
        ApprovalState::Pending(job) => job,
        other => panic!("expected Pending, got {other:?}"),
    };

    // Staff edit it out of balance behind the queue's back.
    tw.run(GOD, "+sheet"); // no-op, proves the staff surface is registered
    tw.run(GOD, "@sheet/set Nick/sensors=5");

    let ev = tw.run(GOD, &format!("+job/approve {job}=Welcome aboard."));
    assert!(
        saw(&tw, &ev, GOD, "no longer balances"),
        "re-validation at approval, not a rubber stamp: {:?}",
        tw.seen_by(&ev, GOD)
    );
    assert!(
        matches!(read_sheet(&tw, nick).approved, ApprovalState::Pending(_)),
        "the job stays open and the sheet stays pending"
    );
}

// ----------------------------------------------------------------------
// 4. The gate
// ----------------------------------------------------------------------

#[test]
fn the_gate_refuses_an_unlisted_verb_and_allows_the_chargen_set() {
    let mut tw = gated_world();
    let nick = tw.spawn_player("Nick");

    let ev = tw.run(nick, "+roll 2d6");
    assert!(refused(&tw, &ev, nick), "+roll is not an onboarding verb");

    for line in ["+chargen", "+sheet", "look"] {
        let ev = tw.run(nick, line);
        assert!(!refused(&tw, &ev, nick), "{line} must stay available");
    }
}

#[test]
fn the_gate_refuses_an_exit_even_aliased_like_an_onboarding_verb() {
    let mut tw = gated_world();
    // An exit in Limbo whose alias `l` collides with `look`/`l`.
    tw.run(GOD, "@dig Nowhere=Somewhere;l,back");
    let nick = tw.spawn_player("Nick");

    let ev = tw.run(nick, "l");
    assert!(refused(&tw, &ev, nick), "an exit aliased `l` must be gated");
    assert_eq!(
        tw.world().get(nick).and_then(|o| o.location),
        Some(LIMBO),
        "the gated exit-alias did not move Nick"
    );
    // Plain `look` (no exit spells that word) is still allowed.
    let ev = tw.run(nick, "look");
    assert!(!refused(&tw, &ev, nick), "look must still be allowed");
}

#[test]
fn the_gate_is_inert_with_chargen_required_off() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    tw.run(GOD, "@dig Elsewhere=out,in");

    let ev = tw.run(nick, "+roll 2d6");
    assert!(!refused(&tw, &ev, nick));
    let ev = tw.run(nick, "out");
    assert!(!refused(&tw, &ev, nick));
    assert_ne!(
        tw.world().get(nick).and_then(|o| o.location),
        Some(LIMBO),
        "Nick actually moved through the exit"
    );
}

// ----------------------------------------------------------------------
// 5. +roll
// ----------------------------------------------------------------------

#[test]
fn roll_prints_a_result_the_room_can_read() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");

    let ev = tw.run(nick, "+roll 2d6");
    let lines = tw.seen_by(&ev, nick);
    let rolled = lines
        .iter()
        .find(|line| line.contains("2d6"))
        .unwrap_or_else(|| panic!("a roll line: {lines:?}"));
    assert!(rolled.contains("Nick"), "{rolled}");
    assert!(rolled.contains('='), "the total is shown: {rolled}");
    // The two faces and the total, lifted out of `<expr>  <a> <b> = <total>`.
    let (_, tail) = rolled.split_once("2d6").expect("the expression");
    let (dice, total) = tail.split_once('=').expect("the total");
    let faces: Vec<i64> = dice
        .split_whitespace()
        .filter_map(|face| face.parse().ok())
        .collect();
    assert_eq!(faces.len(), 2, "two six-siders: {rolled}");
    assert!(faces.iter().all(|face| (1..=6).contains(face)), "{rolled}");
    assert_eq!(
        total.trim().parse::<i64>().ok(),
        Some(faces.iter().sum()),
        "{rolled}"
    );
}

#[test]
fn roll_refuses_what_is_not_a_dice_expression() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");

    let ev = tw.run(nick, "+roll strength+brawl");
    assert!(
        saw(&tw, &ev, nick, "invalid dice expression"),
        "a trait pool is not a dice expression any more: {:?}",
        tw.seen_by(&ev, nick)
    );
    assert!(!saw(&tw, &ev, nick, " = "), "and nothing was rolled");

    let ev = tw.run(nick, "+roll 9999d6");
    assert!(
        saw(&tw, &ev, nick, "too many dice"),
        "an oversized expression is refused, not capped: {:?}",
        tw.seen_by(&ev, nick)
    );
    assert!(!saw(&tw, &ev, nick, " = "), "and nothing was rolled");
}

// ----------------------------------------------------------------------
// 6. The one NPC ability
// ----------------------------------------------------------------------

#[test]
fn the_check_ability_returns_an_outcome_against_the_sheet() {
    let mut tw = plain_world();
    let npc = tw.spawn_player("Ada");
    build_a_balanced_pilot(&mut tw, npc);

    let ruleset = PilotRuleset::new();
    let mut out = Outbox::new();
    let outcome = ruleset
        .invoke_ability(
            tw.world_mut(),
            &mut out,
            npc,
            agent::CHECK_ABILITY,
            serde_json::json!({ "rating": "gunnery", "difficulty": 4, "modifier": 1 }),
        )
        .expect("a gunnery check resolves");

    assert_eq!(outcome.data.get("dots"), Some(&serde_json::json!(4)));
    assert_eq!(outcome.data.get("difficulty"), Some(&serde_json::json!(4)));
    assert_eq!(outcome.data.get("modifier"), Some(&serde_json::json!(1)));
    let dice = outcome
        .data
        .get("dice")
        .and_then(serde_json::Value::as_array)
        .expect("the per-die provenance");
    assert_eq!(dice.len(), 4, "gunnery 4 throws 4d6");
    let total = outcome
        .data
        .get("total")
        .and_then(serde_json::Value::as_i64)
        .expect("a total");
    assert!((5..=25).contains(&total), "4d6 + 1 = {total}");
    assert_eq!(outcome.ok, total >= 4);
    assert!(outcome.summary.contains("gunnery"), "{}", outcome.summary);
}

#[test]
fn check_is_the_only_ability_and_only_over_the_three_ratings() {
    let mut tw = plain_world();
    let npc = tw.spawn_player("Ada");
    let ruleset = PilotRuleset::new();
    let mut out = Outbox::new();

    let unknown = ruleset.invoke_ability(
        tw.world_mut(),
        &mut out,
        npc,
        "wod_roll",
        serde_json::json!({ "pool_expr": "strength+brawl" }),
    );
    assert!(
        matches!(unknown, Err(mush_ruleset::AbilityError::Unknown(name)) if name == "wod_roll"),
        "the two deleted abilities are gone"
    );

    let bad_rating = ruleset.invoke_ability(
        tw.world_mut(),
        &mut out,
        npc,
        agent::CHECK_ABILITY,
        serde_json::json!({ "rating": "willpower" }),
    );
    assert!(matches!(
        bad_rating,
        Err(mush_ruleset::AbilityError::Invalid(_))
    ));
}

#[test]
fn an_npc_with_no_sheet_checks_as_a_green_pilot() {
    let mut tw = plain_world();
    let npc = tw.spawn_player("Nobody");
    let ruleset = PilotRuleset::new();
    let mut out = Outbox::new();

    let outcome = ruleset
        .invoke_ability(
            tw.world_mut(),
            &mut out,
            npc,
            agent::CHECK_ABILITY,
            serde_json::json!({ "rating": "sensors" }),
        )
        .expect("a sheetless NPC still resolves");
    assert_eq!(outcome.data.get("dots"), Some(&serde_json::json!(1)));
    assert_eq!(
        outcome.data.get("difficulty"),
        Some(&serde_json::json!(agent::DEFAULT_DIFFICULTY)),
    );
}

#[test]
fn init_sheet_bootstraps_an_npc_and_leaves_an_existing_one_alone() {
    let mut tw = plain_world();
    let npc = tw.spawn_player("Ada");
    let ruleset = PilotRuleset::new();

    let first = ruleset
        .init_sheet(tw.world_mut(), npc, "", GOD)
        .expect("the bootstrap writes");
    assert!(first.contains("pilot sheet"), "{first}");
    assert_eq!(read_sheet(&tw, npc).number(FIELD_PILOTING), Some(1));

    tw.run(GOD, "@sheet/set Ada/piloting=3");
    let second = ruleset
        .init_sheet(tw.world_mut(), npc, "", GOD)
        .expect("the second call is a no-op");
    assert!(second.contains("already"), "{second}");
    assert_eq!(
        read_sheet(&tw, npc).number(FIELD_PILOTING),
        Some(3),
        "a bootstrap is not an eraser"
    );
}

#[test]
fn the_grounding_profile_carries_the_ratings_and_the_one_tool() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    build_a_balanced_pilot(&mut tw, nick);

    let profile = PilotRuleset::new().agent_profile(&read_sheet(&tw, nick));
    assert!(profile.summary.contains("Ladder"), "{}", profile.summary);
    assert_eq!(
        profile.stats.pointer("/ratings/piloting"),
        Some(&serde_json::json!(3))
    );
    assert_eq!(profile.tools.len(), 1);
    assert_eq!(profile.tools[0].name.as_str(), agent::CHECK_ABILITY);
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.text(FIELD_NAME), Some("Ada Renard"));
    assert_eq!(doc.text(FIELD_CALLSIGN), Some("Ladder"));
}
