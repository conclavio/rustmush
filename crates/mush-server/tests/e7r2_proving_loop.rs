//! **E7 residue pass 2 — P3: doc 10 §11's proving loop, and whether any arrow still
//! needs a staff act** (10b §15.1 item 11).
//!
//! §11's proving story, verbatim: *extract water and metals near Ceres → hire players
//! to haul to Phobos → fight or evade an interception → manufacture or refit a ground
//! unit → load it aboard a carrier → deploy to Mars → complete a ground operation that
//! changes control or access → salvage, repair, pay, repeat.*
//!
//! Item 11's standard has two limbs — *every arrow leaves a recorded fact* and *no
//! arrow needs a staff act* — and this file audits the **second**, which is the one
//! the residue passes were opened to close and the one a single missing `require_staff`
//! silently breaks. It runs against the **production composition**: the same module
//! list `mushd` boots, with `tacsim.corridor@2` seeded, so what is asserted is the
//! authority a real server publishes and not a rig's.
//!
//! **How an arrow is judged.** For each arrow the loop names, an ordinary player — no
//! `Role::Royalty`, no `SIM_ADMIN`, no capability grant — types the verb. The arrow is
//! **staff-free** when the answer is anything other than a *permission* refusal: a
//! usage line, a "no such thing", a missing-funds refusal or an outright success all
//! mean the lane is open to a player and the rest is content. A permission refusal
//! means the arrow needs a staffer, and the test names it.
//!
//! That is deliberately a weaker claim than "the loop runs", and the difference is
//! stated rather than hidden: driving one continuous corridor run end to end needs a
//! seeded fleet, funded accounts, a live operation and a fight, which is a fixture,
//! not a gate. What item 11 asks — and what a regression would break — is whether a
//! *player* can reach every arrow, and that is exactly what this measures.

use mush_core::{Audience, GOD, ObjId, ObjKind, Powers, Role};
use mush_proto::OutputEvent;
use mush_server::modules::{server_modules_with, sheet_modules, tacsim_module};
use mush_testkit::{TestWorld, text_of};

// ---- the rig: the PRODUCTION composition, with the whole corridor -----------

fn boot() -> TestWorld {
    let onboarding = mush_core::OnboardingConfig::default();
    // 16b cut R-b: one composition. The four ruleset lines this used to carry
    // selected `shadowrun` and passed `[shadowrun.economy]` into the built-ins;
    // the body below is TacSim-only and always was.
    let game = tacsim::GameEconomyConfig::default();
    let mut modules = server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        mush_campaign::RecoveryConfig::default(),
    );
    modules.extend(sheet_modules(&onboarding, &game));
    // E7r2 P5: the whole Mars–Phobos–Ceres corridor, as `tacsim.corridor@2`.
    modules.push(tacsim_module(tacsim::fixture::config_v2()));
    TestWorld::with_modules(modules)
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The lines this tree uses to say **"you are not staff"**, and only those.
///
/// A refusal that is not one of these is a content answer — a usage line, an unknown
/// id, an empty account — and the lane is open to the player who typed it.
const STAFF_REFUSALS: [&str; 6] = [
    "Permission denied",
    "Only staff",
    "only staff",
    "You are not staff",
    "staff only",
    "Staff only",
];

fn is_staff_refusal(text: &str) -> bool {
    STAFF_REFUSALS.iter().any(|needle| text.contains(needle))
}

/// One ordinary player, with no powers and no role at all.
fn player(tw: &mut TestWorld, name: &str) -> ObjId {
    let who = tw.spawn_player(name);
    let obj = tw.world_mut().get_mut(who).expect("the spawned player");
    obj.powers = Powers::empty();
    who
}

/// Type one line as `actor` and hand back what they saw.
fn say(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    texts(&tw.run(actor, line))
}

/// One arrow of §11's loop: the step and the line a player types for it.
struct Arrow {
    step: String,
    line: String,
}

/// The loop, arrow by arrow, in §11's own order, for a player who **owns `mine`**.
///
/// Every line is well-formed and most name ids that need not exist: what is under test
/// is the **authority** gate, which runs before any id is resolved, so an unknown id is
/// the honest content answer and a permission refusal is the failure.
///
/// The two `@journey/depart` arrows are the exception and they name `mine`
/// deliberately. E7 residue pass 1's R6 gates departure on `is_admin ||
/// perm::controls(carrier) || drives(carrier)` — an authority over **that hull**, not a
/// staff lane — so pointing them at somebody else's carrier would measure the wrong
/// thing and report a staff step that is not there. A player who controls the hull is
/// the case §11 means by *"hire players to haul"*.
fn loop_arrows(mine: ObjId) -> Vec<Arrow> {
    let a = |step: &str, line: String| Arrow {
        step: step.to_owned(),
        line,
    };
    vec![
        a("extract water and metals near Ceres", "@extract 1".into()),
        a(
            "haul to Phobos (crew-driven since E7r1 R6)",
            format!("@journey/depart #{}=ceres_phobos", mine.0),
        ),
        a(
            "fight or evade an interception",
            "@corridor/show ceres_phobos".into(),
        ),
        a(
            "manufacture or refit a ground unit (draft)",
            "@wo/draft tacsim.core/alloy_smelt/1=1 1 1 1 1".into(),
        ),
        a(
            "manufacture or refit a ground unit (submit, paid since E7r1 R1)",
            "@wo/submit 1".into(),
        ),
        a(
            "load it aboard a carrier",
            format!("@vault/deposit #{}=1", mine.0),
        ),
        a(
            "deploy to Mars",
            format!("@journey/depart #{}=phobos_mars", mine.0),
        ),
        a("complete a ground operation", "@op/show 1".into()),
        a("salvage (E7r2 P2)", "@salvage/status".into()),
        a(
            "salvage the cargo out of the wreck (E7r2 P2)",
            "@salvage/recover 1/1/lot#0000000000000001=1".into(),
        ),
        a(
            "pay: list what the run produced (E7r2 P1)",
            "@market/vendor stock 1=tacsim:iron price=10 endpoint=1".into(),
        ),
        a("pay: buy it (E7r2 P1)", "+market/accept 1=1".into()),
    ]
}

/// One `Thing` the player owns outright, so `perm::controls` answers true for it.
fn owned_by(tw: &mut TestWorld, who: ObjId, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, who, Some(mush_core::LIMBO))
        .expect("an owned object")
}

// ---- E7r2: P3 — the audit --------------------------------------------------

/// **Every arrow of doc 10 §11's proving loop is reachable by an ordinary player.**
///
/// The test that closes item 11's second limb, or names exactly which arrow does not.
/// It fails with the step, the verb and the refusal, so a regression says which arrow
/// went back to staff rather than "the loop broke".
#[test]
fn e7r2_p3_every_arrow_of_the_proving_loop_is_reachable_without_a_staff_act() {
    let mut tw = boot();
    let pat = player(&mut tw, "Pat");
    assert!(
        !mush_core::perm::has_role(tw.world(), pat, Role::Royalty),
        "the actor must be an ordinary player, or the audit measures nothing"
    );

    let mine = owned_by(&mut tw, pat, "Cutter");
    let mut blocked: Vec<String> = Vec::new();
    for arrow in loop_arrows(mine) {
        let answer = say(&mut tw, pat, &arrow.line);
        if is_staff_refusal(&answer) {
            blocked.push(format!(
                "  · {} — `{}` answered: {}",
                arrow.step,
                arrow.line,
                answer.replace('\n', " / ")
            ));
        }
    }
    assert!(
        blocked.is_empty(),
        "doc 10 §11's proving loop still needs a staff act on {} arrow(s):\n{}\n\
         Restate 10b §15.1 item 11 with exactly these, and do not soften it.",
        blocked.len(),
        blocked.join("\n")
    );
}

/// **The two arrows E7 residue pass 2 moved off staff, named individually.**
///
/// A single all-arrows test can go green because a verb stopped existing, so the two
/// this pass is responsible for are asserted in their own right — including that a
/// staffer still gets the same answer, which is what says the change was an
/// *authority* change and not a lane that silently broke for everybody.
#[test]
fn e7r2_p3_stocking_a_lot_and_recovering_a_wreck_are_no_longer_staff_acts() {
    let mut tw = boot();
    let pat = player(&mut tw, "Pat");

    // P1: `@market/vendor stock` and `/withdraw` are acts of the vendor's own
    // organization for a lot-represented good. `create`, `fund`, `retire`, `/catalog`
    // and `/source` are deliberately still staff, which the next test pins.
    for line in [
        "@market/vendor stock 1=tacsim:iron price=10 endpoint=1",
        "@market/vendor withdraw 1",
    ] {
        let answer = say(&mut tw, pat, line);
        assert!(
            !is_staff_refusal(&answer),
            "E7r2 P1 moved `{line}` off the staff lane: {answer}"
        );
    }

    // P2: the wreck's own custodian recovers its cargo.
    for line in [
        "@salvage/status",
        "@salvage/recover 1/1/lot#0000000000000001=1",
    ] {
        let answer = say(&mut tw, pat, line);
        assert!(
            !is_staff_refusal(&answer),
            "E7r2 P2 made `{line}` the custodian's own act: {answer}"
        );
    }
}

/// **What stayed staff, stayed staff.** Moving two arrows off the staff lane is a
/// narrowing, not a removal: the arms that mint title, open a faucet or retire a shop
/// are still staff acts, and this is the pin that stops the next pass widening them by
/// accident.
#[test]
fn e7r2_p3_the_staff_arms_of_the_market_and_the_yard_are_unchanged() {
    let mut tw = boot();
    let pat = player(&mut tw, "Pat");
    for line in [
        "@market/vendor create 1=1",
        "@market/vendor fund 1=100 faucet=1",
        "@market/vendor retire 1",
        "@market/catalog list tacsim",
        "@market/source create 1=1",
        // Instantiating a facility is authored data, exactly as `@deposit/open` is.
        "@facility/open 7=2 smelt",
    ] {
        let answer = say(&mut tw, pat, line);
        assert!(
            is_staff_refusal(&answer),
            "`{line}` must stay a staff act: {answer}"
        );
    }
}

/// **The corridor a player can actually see is the whole one** — the seeded
/// `tacsim.corridor@2`, with the Mars end in it, read through the ordinary
/// `@corridor` surface by an ordinary player.
///
/// Without this the audit above could pass on a world whose corridor stops at Phobos,
/// which is precisely the state item 9's content half was opened to fix.
#[test]
fn e7r2_p3_a_player_reads_the_whole_corridor_including_the_mars_end() {
    let mut tw = boot();
    let pat = player(&mut tw, "Pat");
    // Seeding is staff's — it is authored data, like a pack load — so GOD does it and
    // the player reads the result.
    let seeded = say(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    assert!(!seeded.contains("No such"), "{seeded}");

    let ceres = say(&mut tw, pat, "@corridor/show ceres_phobos");
    assert!(!is_staff_refusal(&ceres), "{ceres}");
    let mars = say(&mut tw, pat, "@corridor/show phobos_mars");
    assert!(
        !is_staff_refusal(&mars) && !mars.contains("No such route"),
        "the Mars leg is authored and readable: {mars}"
    );
}

/// **The wreck a player cannot draw from is invisible to them**, on the production
/// composition and through the real `SalvageSource` — so the audit above is not
/// green because the salvage lane answers everybody the same way for want of a
/// composition.
#[test]
fn e7r2_p3_the_production_salvage_source_is_wired_and_existence_hiding() {
    let mut tw = boot();
    let pat = player(&mut tw, "Pat");
    // The action is registered at all, which is what says the composition named a
    // source: an unwired one registers none.
    assert!(
        mush_inventory::salvage::action_ref(tw.registry()).is_some(),
        "the production composition wires `with_salvage_source`"
    );
    // …and with no wreck in the world, every principal reads the same nothing.
    let theirs = say(&mut tw, pat, "@salvage/status");
    let sam = player(&mut tw, "Sam");
    assert_eq!(theirs, say(&mut tw, sam, "@salvage/status"));
    assert!(theirs.contains("(nothing to recover)"), "{theirs}");

    // A wreck's cargo endpoint belongs to whoever may take from it, and a stranger's
    // vault is not one: the two refusals are byte-identical.
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "Crate", pat, Some(mush_core::LIMBO))
        .expect("a backing object");
    let _ = say(
        &mut tw,
        GOD,
        &format!(
            "@vault/create hold=container:#{} owner=#{}",
            object.0, pat.0
        ),
    );
    let stranger = say(&mut tw, sam, "@salvage/recover 1/1/lot#0000000000000001=1");
    let phantom = say(
        &mut tw,
        sam,
        "@salvage/recover 999/999/lot#0000000000000001=1",
    );
    assert_eq!(stranger, phantom, "existence-hiding on the production lane");
}

// ---- E7r2: the adversarial verify's own fold ---------------------------------

/// **vf_a1 — the wreck walk is bounded on records VISITED, not on records matched.**
///
/// D-LBK-5's rule is *bounded on records visited, never on records matched*, and
/// `SalvageSource::wrecks` shipped as `.filter(is_wreck).take(n)` — which bounds the
/// **result** and lets the walk run to the end of the journey book. `sites` is called
/// three times per `@salvage` command (the offer's projection, the command's
/// pre-check and `/status`), so a world whose journeys are mostly ordinary arrivals
/// paid for every one of them, three times, on every salvage a player typed.
///
/// The fix is a visit counter and the pin is a **source probe**, because the property
/// is about the shape of the walk rather than about a result a small fixture could
/// distinguish: a world with few enough journeys to build in a test is exactly the
/// world in which the unbounded version behaves identically.
#[test]
fn vf_a1_the_wreck_walk_is_bounded_on_records_visited() {
    let src = std::fs::read_to_string(
        // The salvage adapter lives with the composition root in the game crate
        // since 16b cut R-a (R2).
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tacsim/src/composition/salvage_source.rs"),
    )
    .expect("the salvage source");
    let at = src.find("fn wrecks(").expect("the wreck walk");
    let end = src[at..]
        .find("\n    fn ")
        .map_or(src.len(), |offset| at + offset);
    let body = &src[at..end];
    assert!(
        body.contains("MAX_SALVAGE_JOURNEY_VISIT"),
        "the walk must carry its own VISIT budget: {body}"
    );
    assert!(
        !body.contains(".filter("),
        "a `filter().take()` bounds the RESULT and not the walk, which is the defect \
         D-LBK-5 names: {body}"
    );
    // The budget is encoded and is deliberately far above the result budget, because
    // the common case is a long tail of `Arrived` records with a handful of wrecks at
    // unpredictable keys.
    assert_eq!(
        mush_server::salvage_source::MAX_SALVAGE_JOURNEY_VISIT,
        4_096
    );
    // Both sides are `const`, so this is a compile-time invariant, not a runtime
    // assertion (clippy::assertions_on_constants under `-D warnings`).
    const _: () = assert!(
        mush_server::salvage_source::MAX_SALVAGE_JOURNEY_VISIT > mush_inventory::MAX_SALVAGE_WRECKS,
        "a visit budget as tight as the result budget would hide live wrecks behind old arrivals"
    );
    // …and the whole thing still answers on a live world.
    let mut tw = boot();
    let pat = player(&mut tw, "Pat");
    assert!(say(&mut tw, pat, "@salvage/status").contains("(nothing to recover)"));
}
