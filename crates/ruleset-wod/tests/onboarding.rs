//! M9 onboarding: the guided chargen **wizard**, the `chargen_required`
//! **gate**, the connect-time drop into the wizard, and the approval move —
//! driven through the real [`Engine`](mush_core) via
//! [`mush_testkit::TestWorld`] with the jobs module loaded so approval runs.

use mush_core::{GOD, LIMBO, ObjId, ObjKind};
use mush_jobs::JobsModule;
use mush_ruleset::{ApprovalState, SheetDoc};
use mush_testkit::TestWorld;
use ruleset_wod::{OnboardingConfig, WodRuleset};

const S: u64 = 12345;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

/// A world with onboarding OFF (the default, M6 behaviour).
fn plain_world() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(S)),
    ])
}

/// A world with `chargen_required` on and both rooms = Limbo.
fn gated_world() -> TestWorld {
    let cfg = OnboardingConfig {
        chargen_required: true,
        start_room: LIMBO,
        start_room_unapproved: LIMBO,
    };
    TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(S).onboarding(cfg)),
    ])
}

fn read_sheet(tw: &TestWorld, who: ObjId) -> SheetDoc {
    let attr = tw
        .world()
        .attr(who, "sheet:wod")
        .and_then(mush_core::Attr::as_json)
        .expect("a sheet");
    serde_json::from_value(attr.clone()).expect("sheet decodes")
}

/// Whether any event to `who` was the onboarding-gate refusal.
fn refused(
    tw: &TestWorld,
    ev: &[(mush_core::Audience, mush_proto::OutputEvent)],
    who: ObjId,
) -> bool {
    tw.seen_by(ev, who)
        .iter()
        .any(|l| l.contains("must finish creating"))
}

/// Drive the wizard through a fully balanced allocation (the shared body of
/// the vampire and mortal flows up to Backgrounds); `nick` must already be in
/// the wizard (either via `+chargen/wizard` or the connect-time drop).
fn wizard_core_allocation(tw: &mut TestWorld, nick: ObjId) {
    tw.run(nick, "next"); // Concept
    tw.run(nick, "concept=Jaded Detective");
    tw.run(nick, "next"); // Attribute priorities
    tw.run(nick, "physical=7 social=5 mental=3");
    tw.run(nick, "next"); // Attributes  (7 / 5 / 3 above the free 1)
    tw.run(nick, "strength=4");
    tw.run(nick, "dexterity=4");
    tw.run(nick, "stamina=2");
    tw.run(nick, "charisma=3");
    tw.run(nick, "manipulation=3");
    tw.run(nick, "appearance=2");
    tw.run(nick, "perception=2");
    tw.run(nick, "intelligence=2");
    tw.run(nick, "wits=2");
    tw.run(nick, "next"); // Ability priorities
    tw.run(nick, "talents=13 skills=9 knowledges=5");
    tw.run(nick, "next"); // Abilities (13 / 9 / 5, cap 3 each)
    tw.run(nick, "brawl=3");
    tw.run(nick, "alertness=3");
    tw.run(nick, "subterfuge=3");
    tw.run(nick, "intimidation=3");
    tw.run(nick, "streetwise=1");
    tw.run(nick, "firearms=3");
    tw.run(nick, "drive=3");
    tw.run(nick, "stealth=3");
    tw.run(nick, "investigation=3");
    tw.run(nick, "occult=2");
    tw.run(nick, "next"); // Backgrounds (5)
    tw.run(nick, "Resources=3");
    tw.run(nick, "Allies=2");
}

// ----------------------------------------------------------------------
// The wizard
// ----------------------------------------------------------------------

#[test]
fn wizard_builds_a_valid_vampire_end_to_end() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");

    // Enter the wizard.
    let ev = tw.run(nick, "+chargen/wizard");
    assert!(
        tw.seen_by(&ev, nick).iter().any(|l| l.contains("Guided")),
        "wizard greeting: {:?}",
        tw.seen_by(&ev, nick)
    );
    assert!(tw.world().input_depth(nick) > 0, "wizard is pushed");

    // Template step.
    let ev = tw.run(nick, "vampire");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Template set to vampire"))
    );

    // Shared allocation through Backgrounds.
    wizard_core_allocation(&mut tw, nick);

    // Splat step (vampire): clan + Discipline pool (3 dots), with a live
    // budget shown after each set.
    tw.run(nick, "next");
    tw.run(nick, "clan=brujah");
    let ev = tw.run(nick, "Celerity=2");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Disciplines")),
        "budget shows the Disciplines pool: {:?}",
        tw.seen_by(&ev, nick)
    );
    tw.run(nick, "Potence=1");

    // Virtues (vampire pool = 7 dots above the free 1).
    tw.run(nick, "next");
    tw.run(nick, "conscience=4");
    tw.run(nick, "self_control=3");
    tw.run(nick, "courage=3");

    // Freebies (nothing to spend — the pools balance) → Review.
    tw.run(nick, "next"); // Freebies
    let ev = tw.run(nick, "next"); // Review
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("balances") || l.contains("submit")),
        "review says it's ready: {:?}",
        tw.seen_by(&ev, nick)
    );

    // Submit → the wizard pops, the sheet is Pending, a wod-chargen job opens.
    let ev = tw.run(nick, "submit");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Submitted")),
        "submit confirmation: {:?}",
        tw.seen_by(&ev, nick)
    );
    assert_eq!(tw.world().input_depth(nick), 0, "wizard popped on submit");
    let doc = read_sheet(&tw, nick);
    assert!(
        matches!(doc.approved, ApprovalState::Pending(_)),
        "sheet pending"
    );
    assert_eq!(doc.text("template"), Some("vampire"));
    assert_eq!(doc.text("clan"), Some("brujah"));
    // Finalize derived the vampire starting values.
    assert_eq!(
        doc.number("bloodpool"),
        Some(10),
        "13th-gen blood pool full"
    );
}

#[test]
fn wizard_validates_each_step() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    tw.run(nick, "+chargen/wizard");
    tw.run(nick, "mortal");
    tw.run(nick, "next"); // Concept
    tw.run(nick, "next"); // Attribute priorities

    // A non-permutation priority set is refused.
    let ev = tw.run(nick, "physical=7 social=7 mental=3");
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("must be exactly")),
        "priority validation: {:?}",
        tw.seen_by(&ev, nick)
    );

    // A valid set is accepted and shows the budget.
    let ev = tw.run(nick, "physical=7 social=5 mental=3");
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("Physical")));

    // In the Attributes step the budget tracks each dot.
    tw.run(nick, "next");
    let ev = tw.run(nick, "strength=4");
    assert!(
        tw.seen_by(&ev, nick).iter().any(|l| l.contains("3/7")),
        "physical budget 3/7 after strength=4: {:?}",
        tw.seen_by(&ev, nick)
    );
}

#[test]
fn wizard_abort_preserves_the_partial_sheet() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    tw.run(nick, "+chargen/wizard");
    tw.run(nick, "vampire");
    tw.run(nick, "next");
    tw.run(nick, "concept=Detective");

    // The hard escape saves and exits.
    let ev = tw.run(nick, "~q");
    assert!(
        tw.seen_by(&ev, nick).iter().any(|l| l.contains("Aborted")),
        "~q aborts: {:?}",
        tw.seen_by(&ev, nick)
    );
    assert_eq!(tw.world().input_depth(nick), 0, "wizard popped");

    // The partial sheet survives — nothing was discarded.
    let doc = read_sheet(&tw, nick);
    assert_eq!(doc.text("template"), Some("vampire"));
    assert_eq!(doc.text("concept"), Some("Detective"));
    assert!(matches!(doc.approved, ApprovalState::Unapproved));

    // And it resumes.
    let ev = tw.run(nick, "+chargen/wizard");
    assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("Guided")));
    assert!(tw.world().input_depth(nick) > 0);
}

#[test]
fn spawn_places_unapproved_in_the_chargen_area() {
    // A distinct chargen area (#2); the live start room is Limbo.
    let cfg = OnboardingConfig {
        chargen_required: true,
        start_room: LIMBO,
        start_room_unapproved: ObjId(2),
    };
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(S).onboarding(cfg)),
    ]);
    let cradle = tw
        .world_mut()
        .create(ObjKind::Room, "Cradle", GOD, None)
        .unwrap();
    assert_eq!(cradle, ObjId(2), "chargen-area id assumption");

    // A freshly created player is placed in the chargen area, not Limbo.
    let nick = tw.spawn_player("Nick");
    assert_eq!(
        tw.world().get(nick).and_then(|o| o.location),
        Some(cradle),
        "placed in the chargen area on creation"
    );
}

// ----------------------------------------------------------------------
// The gate
// ----------------------------------------------------------------------

#[test]
fn gate_blocks_grid_commands_and_allows_onboarding() {
    let mut tw = gated_world();
    // A live grid room + exit to test movement blocking (built by God, who
    // bypasses the gate).
    tw.run(GOD, "@dig Elsewhere=out,in");
    let nick = tw.spawn_player("Nick");
    // Spawn drops Nick into the wizard; step out of it so we exercise the gate
    // directly.
    tw.run(nick, "~q");
    assert_eq!(tw.world().input_depth(nick), 0);

    // Allowed: perception, speech, help, chargen.
    for allowed in ["look", "say hi", ":waves", "help", "+chargen/status"] {
        let ev = tw.run(nick, allowed);
        assert!(!refused(&tw, &ev, nick), "{allowed:?} must be allowed");
    }

    // Blocked: a live-grid roll, an exit (movement), and building.
    for blocked in ["+roll strength", "out", "@dig X=a,b"] {
        let ev = tw.run(nick, blocked);
        assert!(refused(&tw, &ev, nick), "{blocked:?} must be gated");
    }
    // The exit really did not move Nick.
    assert_eq!(tw.world().get(nick).and_then(|o| o.location), Some(LIMBO));
}

#[test]
fn gate_blocks_an_exit_aliased_like_an_onboarding_verb() {
    // Regression: an exit whose name/alias collides with an allowlisted verb
    // must not defeat the movement block. Dispatch matches exits (step 4)
    // after the gate (step 1.5) against the whole line, so the gate has to
    // resolve the line against exits itself — checking only the leading verb
    // (`l` is an onboarding verb) would wave the exit-alias through.
    let mut tw = gated_world();
    // A global-style exit in Limbo whose alias `l` collides with `look`/`l`.
    tw.run(GOD, "@dig Nowhere=Somewhere;l,back");
    let nick = tw.spawn_player("Nick");
    // Spawn drops Nick into the wizard; step out so we exercise the gate.
    tw.run(nick, "~q");
    assert_eq!(tw.world().input_depth(nick), 0);

    // Typing the exit alias is refused, and Nick does not move onto the grid.
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
fn connect_drops_unapproved_char_into_the_wizard() {
    let mut tw = gated_world();
    // Create without the auto-connect so we can capture the connect output.
    let nick = tw
        .world_mut()
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let ev = tw.connect(nick);
    assert!(
        tw.seen_by(&ev, nick)
            .iter()
            .any(|l| l.contains("Welcome") || l.contains("Guided")),
        "connect greeting drops into the wizard: {:?}",
        tw.seen_by(&ev, nick)
    );
    assert!(
        tw.world().input_depth(nick) > 0,
        "in the wizard after connect"
    );
}

// ----------------------------------------------------------------------
// Approval lifts the gate + moves to the live start room
// ----------------------------------------------------------------------

#[test]
fn approval_lifts_the_gate_and_moves_to_start_room() {
    // Distinct live start room; the chargen area is Limbo.
    let cfg = OnboardingConfig {
        chargen_required: true,
        start_room: ObjId(2),
        start_room_unapproved: LIMBO,
    };
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(S).onboarding(cfg)),
    ]);
    // The first object created after boot is #2 — our Sanctum start room.
    let sanctum = tw
        .world_mut()
        .create(ObjKind::Room, "Sanctum", GOD, None)
        .unwrap();
    assert_eq!(sanctum, ObjId(2), "start-room id assumption");

    let nick = tw.spawn_player("Nick");
    assert!(tw.world().input_depth(nick) > 0, "dropped into the wizard");

    // Build a balanced Mortal through the wizard (the connect-time drop already
    // opened it), then submit.
    tw.run(nick, "mortal");
    wizard_core_allocation(&mut tw, nick);
    // Backgrounds already at 5; the splat step is skipped for a Mortal.
    tw.run(nick, "next"); // Virtues (leave at 1 each — 0 spent)
    tw.run(nick, "next"); // Freebies
    tw.run(nick, "next"); // Review
    tw.run(nick, "submit");

    let doc = read_sheet(&tw, nick);
    let job = match doc.approved {
        ApprovalState::Pending(j) => j,
        other => panic!("expected pending, got {other:?}"),
    };
    // Still gated while pending.
    let ev = tw.run(nick, "+roll strength");
    assert!(refused(&tw, &ev, nick), "still gated while pending");

    // Staff approve → gate lifts, Nick moves to the live start room.
    let ev = tw.run(GOD, &format!("+job/approve {job}=Welcome."));
    assert!(tw.seen_by(&ev, GOD).iter().any(|l| l.contains("approved")));
    let doc = read_sheet(&tw, nick);
    assert!(doc.approved.is_approved(), "sheet approved");
    assert_eq!(
        tw.world().get(nick).and_then(|o| o.location),
        Some(sanctum),
        "moved to the live start room"
    );
    // Access granted: a grid command is no longer refused.
    let ev = tw.run(nick, "+roll strength");
    assert!(!refused(&tw, &ev, nick), "gate lifted after approval");
}

// ----------------------------------------------------------------------
// Regression: onboarding OFF leaves M6 behaviour identical
// ----------------------------------------------------------------------

#[test]
fn chargen_required_off_is_unchanged() {
    let mut tw = plain_world();
    let nick = tw.spawn_player("Nick");
    // No auto-drop into the wizard.
    assert_eq!(
        tw.world().input_depth(nick),
        0,
        "no wizard forced on connect"
    );
    // No gate: a grid command runs normally (no refusal), even with no sheet.
    let ev = tw.run(nick, "+roll 3");
    assert!(!refused(&tw, &ev, nick));
    // Building by God, movement — the ordinary surface is untouched.
    tw.run(GOD, "@dig Elsewhere=out,in");
    let ev = tw.run(nick, "out");
    assert!(!refused(&tw, &ev, nick));
    assert_ne!(
        tw.world().get(nick).and_then(|o| o.location),
        Some(LIMBO),
        "Nick actually moved through the exit"
    );
}

// ----------------------------------------------------------------------
// ---- convergence fold ----
// ----------------------------------------------------------------------

/// A `LookRendered` hook that records, per room render, whether the render's
/// `HookReadContext` carried a live typed-state snapshot.
///
/// 08j S2: `on_approved` used to re-render by dispatching `"look"` through the
/// compat `mush_core::dispatch`, which is the `None` wrapper over
/// `dispatch_with_action_state` — so every hook in that render, and every
/// `can_perceive` / `is_concealed` under it, ran with `typed_state.store() ==
/// None`. Fail-closed held, but it was the one production engine dispatch that
/// did not use the live store (09 §C2). It now goes through `Ctx::show_room`.
struct TypedStateProbe {
    seen: std::sync::Arc<std::sync::Mutex<Vec<bool>>>,
}

impl mush_core::EventHook for TypedStateProbe {
    fn look_rendered(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        _looker: ObjId,
        _room: ObjId,
    ) -> Vec<mush_proto::RichText> {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(ctx.typed_state_available());
        }
        Vec::new()
    }
}

struct TypedStateProbeModule {
    seen: std::sync::Arc<std::sync::Mutex<Vec<bool>>>,
}

impl mush_core::GameModule for TypedStateProbeModule {
    fn name(&self) -> &'static str {
        "typed_state_probe"
    }

    fn register(&mut self, reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
        reg.hook(
            mush_core::HookPoint::LookRendered,
            std::sync::Arc::new(TypedStateProbe {
                seen: std::sync::Arc::clone(&self.seen),
            }),
        );
        Ok(())
    }
}

/// S2 — the post-approval room render sees a **live** typed store.
#[test]
fn cv_2_the_post_approval_room_render_sees_a_live_typed_store() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let cfg = OnboardingConfig {
        chargen_required: true,
        start_room: ObjId(2),
        start_room_unapproved: LIMBO,
    };
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(S).onboarding(cfg)),
        Box::new(TypedStateProbeModule {
            seen: std::sync::Arc::clone(&seen),
        }),
    ]);
    let sanctum = tw
        .world_mut()
        .create(ObjKind::Room, "Sanctum", GOD, None)
        .unwrap();
    assert_eq!(sanctum, ObjId(2), "start-room id assumption");

    let nick = tw.spawn_player("Nick");
    tw.run(nick, "mortal");
    wizard_core_allocation(&mut tw, nick);
    tw.run(nick, "next"); // Virtues
    tw.run(nick, "next"); // Freebies
    tw.run(nick, "next"); // Review
    tw.run(nick, "submit");
    let doc = read_sheet(&tw, nick);
    let job = match doc.approved {
        ApprovalState::Pending(j) => j,
        other => panic!("expected pending, got {other:?}"),
    };

    // Only the approval render is under test.
    seen.lock().expect("probe mutex").clear();
    tw.run(GOD, &format!("+job/approve {job}=Welcome."));
    assert_eq!(
        tw.world().get(nick).and_then(|o| o.location),
        Some(sanctum),
        "approval moved Nick to the live start room"
    );

    let renders = seen.lock().expect("probe mutex").clone();
    assert!(
        !renders.is_empty(),
        "approval rendered the new room at least once"
    );
    assert!(
        renders.iter().all(|available| *available),
        "every post-approval room render must carry the live typed store, got {renders:?}"
    );
}
