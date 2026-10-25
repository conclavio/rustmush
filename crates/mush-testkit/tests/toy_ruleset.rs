//! The `toy` test ruleset, proved surface by surface (16b cut R-b).
//!
//! One test per seam the platform tests re-point onto: the sheet round trip and
//! its staff-only gate, chargen → approval job → stamped sheet → onboarding
//! lift, the concealment gate, the gated narration channel, the seeded
//! `toy_roll` die faces, and the authoritative Willpower roll committing
//! exactly once per point with a replayable resolution record, an I13
//! republish to the subject alone, and a typed post-image that rehydrates.

#![cfg(feature = "toy-ruleset")]

use std::sync::Arc;

use mush_core::{
    ActionStateBootRecord, Audience, CanonicalId, CommitToken, Effects, GOD, IdempotencyKey,
    IdempotencyScope, Inbound, LIMBO, ModuleWrite, ObjId, ObjKind, OfferExecutionContext,
    OfferKeyRing, OfferSecret, OfferSubmission, OnboardingConfig, Outbox, OwnerId, PerceivedRef,
    PrincipalId, RequestUid, ResolutionKeyRing, ResolutionSecret, ScopedIdempotencyKey,
    is_action_state_persistence_key,
};
use mush_jobs::JobsModule;
use mush_proto::{OutputEvent, PanelFrameKind, VitalValue, VitalsFrame};
use mush_ruleset::{ApprovalState, Ruleset, SheetDoc, SheetStore};
use mush_testkit::TestWorld;
use mush_testkit::toy_ruleset::{ToyRuleset, resolve_pool, toy_schema};
use serde_json::json;

const SEED: u64 = 7;

// ----------------------------------------------------------------------
// Rig
// ----------------------------------------------------------------------

fn world() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ToyRuleset::with_seed(SEED)),
    ])
}

fn store() -> SheetStore {
    SheetStore::new(Arc::new(toy_schema("toy")))
}

/// Seed a full, valid sheet without going through a command.
fn seed_sheet(tw: &mut TestWorld, who: ObjId, edit: impl FnOnce(&mut SheetDoc)) {
    let store = store();
    let mut doc = SheetDoc::new_default(store.schema());
    doc.set_text("template", "alpha");
    edit(&mut doc);
    store
        .put(tw.world_mut(), who, &doc)
        .expect("the fixture sheet validates");
}

fn lines(tw: &TestWorld, events: &[(Audience, OutputEvent)], viewer: ObjId) -> String {
    tw.seen_by(events, viewer).join("\n")
}

/// Everything an `Audience::Actor(who)` event delivered to `who`, as text.
fn text_to(events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    events
        .iter()
        .filter(|(aud, _)| matches!(aud, Audience::Actor(a) if *a == who))
        .map(|(_, ev)| mush_testkit::text_of(ev))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The job id out of a `Submitted for approval as job N.` confirmation.
fn job_id(text: &str) -> u64 {
    text.split("as job ")
        .nth(1)
        .and_then(|tail| {
            tail.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or_else(|| panic!("no job id in {text:?}"))
}

/// One live query through the engine (so the handler gets a typed-state lens
/// and the authoritative overlay applies).
fn query_panel(tw: &mut TestWorld, actor: ObjId, name: &str) -> serde_json::Value {
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    tw.engine_mut().step(
        Inbound::Query {
            conn: 1,
            actor,
            id: 91,
            name: name.into(),
            args: json!({}),
        },
        &mut out,
        &mut effects,
    );
    out.events()
        .iter()
        .find_map(|(_, event)| match event {
            OutputEvent::Reply { id: 91, event } => match event.as_ref() {
                OutputEvent::Panel { data, .. } => Some(data.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_else(|| panic!("{name} returned no panel keyframe"))
}

/// The `toy.willpower_current` gauge's current value, off a vitals panel.
fn pool_current(panel: &serde_json::Value) -> Option<i64> {
    let frame = VitalsFrame::from_panel_data(panel).ok()?;
    frame
        .vitals
        .iter()
        .find(|v| v.id == "toy.willpower_current")
        .and_then(|v| match v.value {
            VitalValue::Bounded { current, .. } => Some(current),
            _ => None,
        })
}

// ----------------------------------------------------------------------
// Sheet
// ----------------------------------------------------------------------

#[test]
fn the_sheet_round_trips_and_hides_staff_fields() {
    let mut tw = world();
    let bob = tw.spawn_player("Bob");

    assert_eq!(
        tw.registry()
            .attr_ns_owner("sheet")
            .map(smol_str::SmolStr::as_str),
        Some("toy")
    );

    let ev = tw.run(GOD, "@sheet/set Bob/strength=3");
    assert!(lines(&tw, &ev, GOD).contains("Set Bob/strength = 3"));
    let ev = tw.run(GOD, "@sheet/set Bob/strength=99");
    assert!(lines(&tw, &ev, GOD).contains("out of range"));
    let ev = tw.run(GOD, "@sheet/set Bob/template=gamma");
    assert!(lines(&tw, &ev, GOD).contains("not one of"));
    let ev = tw.run(GOD, "@sheet/set Bob/secret=4");
    assert!(lines(&tw, &ev, GOD).contains("Set Bob/secret = 4"));

    let ev = tw.run(bob, "+sheet");
    let seen = lines(&tw, &ev, bob);
    assert!(seen.contains("***.."), "strength 3 renders: {seen}");
    assert!(!seen.contains("ST Secret"), "staff-only hidden: {seen}");

    let ev = tw.run(GOD, "@sheet Bob");
    assert!(lines(&tw, &ev, GOD).contains("ST Secret"));

    // A player cannot run the staff editor at all.
    let ev = tw.run(bob, "@sheet/set Bob/strength=5");
    assert!(lines(&tw, &ev, bob).contains("Permission denied."));

    // The storage round trip is exact, and an invalid put leaves it untouched.
    let read = store().get(tw.world(), bob).expect("a sheet");
    assert_eq!(read.dots("strength"), Some(3));
    let mut bad = read.clone();
    bad.set_number("strength", 42);
    assert!(store().put(tw.world_mut(), bob, &bad).is_err());
    assert_eq!(
        store()
            .get(tw.world(), bob)
            .and_then(|d| d.dots("strength")),
        Some(3)
    );
}

#[test]
fn a_pool_resolves_only_against_real_sheet_traits() {
    let schema = toy_schema("toy");
    let mut doc = SheetDoc::new_default(&schema);
    doc.set_number("strength", 4);
    doc.set_number("brawl", 3);

    let ok = resolve_pool(&schema, &doc, "strength+brawl").expect("a real pool");
    assert_eq!(ok.total, 7);
    assert_eq!(ok.label(), "Strength+Brawl");

    // Raw numbers, invented traits, staff-only and non-numeric fields are all
    // refused — the fence that makes the ruleset, not the caller, size a pool.
    assert!(resolve_pool(&schema, &doc, "9").is_err());
    assert!(resolve_pool(&schema, &doc, "strength+wizardry").is_err());
    assert!(resolve_pool(&schema, &doc, "secret").is_err());
    assert!(resolve_pool(&schema, &doc, "concept").is_err());
}

// ----------------------------------------------------------------------
// Chargen + the approval job
// ----------------------------------------------------------------------

#[test]
fn chargen_files_a_job_and_approval_stamps_the_sheet() {
    let mut tw = world();
    let nick = tw.spawn_player("Nick");
    seed_sheet(&mut tw, nick, |_| {});

    // Underspent: submit is refused and nothing is filed.
    let ev = tw.run(nick, "+chargen/submit");
    assert!(lines(&tw, &ev, nick).contains("Cannot submit"));

    // Spend exactly five physical points with `+stat` (baseline 1 each).
    for line in ["+stat strength=4", "+stat dexterity=2", "+stat stamina=2"] {
        let ev = tw.run(nick, line);
        assert!(lines(&tw, &ev, nick).contains(" = "), "{line}");
    }
    let ev = tw.run(nick, "+chargen");
    assert!(lines(&tw, &ev, nick).contains("+chargen/submit"));

    let ev = tw.run(nick, "+chargen/submit");
    let submitted = lines(&tw, &ev, nick);
    assert!(
        submitted.contains("Submitted for approval as job"),
        "{submitted}"
    );
    let job = job_id(&submitted);

    let doc = store().get(tw.world(), nick).expect("a sheet");
    assert_eq!(doc.approved, ApprovalState::Pending(job));
    // Finalize derived Willpower = Stamina + 3 and filled the pool.
    assert_eq!(doc.number("willpower"), Some(5));
    assert_eq!(doc.number("willpower_current"), Some(5));

    // A second submit is refused rather than opening a second job.
    let ev = tw.run(nick, "+chargen/submit");
    assert!(lines(&tw, &ev, nick).contains("already submitted"));

    // Staff approve: the `toy.chargen` payload handler stamps it Approved.
    let ev = tw.run(GOD, &format!("+job/approve {job}=Welcome aboard."));
    let approved = lines(&tw, &ev, GOD);
    assert!(approved.contains("approved"), "{approved}");
    let doc = store().get(tw.world(), nick).expect("a sheet");
    assert!(
        matches!(doc.approved, ApprovalState::Approved { by, .. } if by == GOD),
        "{:?}",
        doc.approved
    );
}

#[test]
fn the_onboarding_trio_gates_spawns_greets_and_lifts() {
    // Limbo #0, God #1, so the two rooms created below are #2 and #3.
    let cfg = OnboardingConfig {
        chargen_required: true,
        start_room: ObjId(3),
        start_room_unapproved: ObjId(2),
    };
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ToyRuleset::with_seed(SEED).onboarding(cfg)),
    ]);
    let chargen_area = tw
        .world_mut()
        .create(ObjKind::Room, "Chargen Nexus", GOD, None)
        .expect("chargen area");
    let commons = tw
        .world_mut()
        .create(ObjKind::Room, "The Commons", GOD, None)
        .expect("start room");
    assert_eq!(chargen_area, ObjId(2));
    assert_eq!(commons, ObjId(3));

    // (1) The `ObjCreated` observer puts a fresh player in the chargen area.
    let nick = tw.spawn_player("Nick");
    assert_eq!(
        tw.world().get(nick).and_then(|o| o.location),
        Some(chargen_area)
    );

    // (2) The `Connected` hook greets a gated character on its first conn.
    let ana = tw
        .world_mut()
        .create(ObjKind::Player, "Ana", GOD, Some(chargen_area))
        .expect("a second player");
    let greeting = text_to(&tw.connect(ana), ana);
    assert!(
        greeting.contains("finish creating your character"),
        "{greeting}"
    );

    // (3) The gate holds: a live-grid command is refused, the allowlist passes.
    let ev = tw.run(nick, "+toyhide on");
    assert!(lines(&tw, &ev, nick).contains("must finish creating"));
    let ev = tw.run(nick, "look");
    assert!(!lines(&tw, &ev, nick).contains("must finish creating"));

    // (4) Build, submit, approve — the gate lifts and the character is moved.
    for line in ["+stat strength=4", "+stat dexterity=2", "+stat stamina=2"] {
        tw.run(nick, line);
    }
    let ev = tw.run(nick, "+chargen/submit");
    let job = job_id(&lines(&tw, &ev, nick));
    let ev = tw.run(GOD, &format!("+job/approve {job}=Welcome."));
    let to_nick = text_to(&ev, nick);
    assert!(to_nick.contains("has been approved"), "{to_nick}");
    assert_eq!(tw.world().get(nick).and_then(|o| o.location), Some(commons));

    let ev = tw.run(nick, "+toyhide on");
    assert!(!lines(&tw, &ev, nick).contains("must finish creating"));
}

// ----------------------------------------------------------------------
// Perception + the gated narration channel
// ----------------------------------------------------------------------

#[test]
fn hiding_conceals_from_anyone_whose_awareness_misses_the_stealth() {
    let mut tw = world();
    let vlad = tw.spawn_player("Vlad");
    let sharp = tw.spawn_player("Sharp");
    let dull = tw.spawn_player("Dull");
    seed_sheet(&mut tw, vlad, |d| d.set_number("stealth", 3));
    seed_sheet(&mut tw, sharp, |d| d.set_number("awareness", 4));
    seed_sheet(&mut tw, dull, |d| d.set_number("awareness", 1));

    // Not hidden yet: everyone hears him.
    let ev = tw.run(vlad, "\"Still here.");
    assert!(lines(&tw, &ev, sharp).contains("Still here."));
    assert!(lines(&tw, &ev, dull).contains("Still here."));

    let ev = tw.run(vlad, "+toyhide on");
    assert!(lines(&tw, &ev, vlad).contains("fade out of sight"));

    let ev = tw.run(vlad, "\"You never saw me.");
    assert!(
        lines(&tw, &ev, sharp).contains("You never saw me."),
        "awareness 4 >= stealth 3 pierces"
    );
    assert!(
        !lines(&tw, &ev, dull).contains("You never saw me."),
        "awareness 1 < stealth 3 is true silence: {}",
        lines(&tw, &ev, dull)
    );

    // Stepping back into view restores everyone.
    tw.run(vlad, "+toyhide off");
    let ev = tw.run(vlad, "\"Back again.");
    assert!(lines(&tw, &ev, dull).contains("Back again."));
}

#[test]
fn a_subtle_act_reaches_only_the_gated_sense_channel() {
    let mut tw = world();
    let vlad = tw.spawn_player("Vlad");
    let mark = tw.spawn_player("Mark");
    let sharp = tw.spawn_player("Sharp");
    let bystander = tw.spawn_player("Bob");
    seed_sheet(&mut tw, vlad, |_| {});
    seed_sheet(&mut tw, mark, |_| {});
    seed_sheet(&mut tw, sharp, |d| d.set_number("awareness", 3));
    seed_sheet(&mut tw, bystander, |d| d.set_number("awareness", 2));

    let ev = tw.run(vlad, "+toysubtle Mark=bend to my will");

    assert!(text_to(&ev, vlad).contains("You work subtly on Mark"));
    assert!(
        text_to(&ev, sharp).contains("bends another's will"),
        "awareness 3 is on the channel: {:?}",
        text_to(&ev, sharp)
    );
    let to_bystander = text_to(&ev, bystander);
    assert!(
        to_bystander.is_empty(),
        "awareness 2 receives NO event at all: {to_bystander:?}"
    );
    assert!(
        !text_to(&ev, mark).contains("bend to my will"),
        "the raw detail never leaves the actor's own stream"
    );
}

// ----------------------------------------------------------------------
// The `toy_roll` NPC ability
// ----------------------------------------------------------------------

/// A second `ToyRuleset` handle over the same world: the dice RNG lives in the
/// world's state slot, so this shares the registered module's seeded stream —
/// the stand-in for the NPC layer calling `Ruleset::invoke_ability`.
fn ability_host() -> ToyRuleset {
    ToyRuleset::with_seed(SEED)
}

fn npc_in(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Npc, name, GOD, Some(LIMBO))
        .expect("an npc")
}

#[test]
fn toy_roll_sizes_the_pool_from_the_stored_sheet() {
    let mut tw = world();
    let npc = npc_in(&mut tw, "Bartender");
    seed_sheet(&mut tw, npc, |d| {
        d.set_number("strength", 4);
        d.set_number("brawl", 3);
    });

    let host = ability_host();
    let mut out = Outbox::new();
    let outcome = host
        .invoke_ability(
            tw.world_mut(),
            &mut out,
            npc,
            "toy_roll",
            json!({ "pool_expr": "strength+brawl", "difficulty": 6 }),
        )
        .expect("the ability runs");

    // The ruleset, not the caller, sized the pool: Strength 4 + Brawl 3 = 7
    // faces, and the faces come back so a caller can check them.
    assert_eq!(outcome.data["pool"], 7);
    let faces = outcome.data["dice"].as_array().expect("die faces");
    assert_eq!(faces.len(), 7, "{:?}", outcome.data);
    assert_eq!(outcome.data["label"], "Strength+Brawl");

    // The room saw the classic roll line with exactly those seven faces.
    let events = out.take();
    let line = events
        .iter()
        .map(|(_, ev)| mush_testkit::text_of(ev))
        .find(|t| t.starts_with("roll  "))
        .expect("a roll narration reached the room");
    let thrown = line
        .split_once("Strength+Brawl  ")
        .and_then(|(_, rest)| rest.split_once(" = "))
        .map_or(0, |(dice, _)| dice.split_whitespace().count());
    assert_eq!(thrown, 7, "{line}");

    // An over-claimed pool is refused outright, and an unknown ability is
    // Unknown rather than silently run.
    let mut out = Outbox::new();
    assert!(
        host.invoke_ability(
            tw.world_mut(),
            &mut out,
            npc,
            "toy_roll",
            json!({ "pool_expr": "strength+9" }),
        )
        .is_err()
    );
    let mut out = Outbox::new();
    assert!(
        host.invoke_ability(tw.world_mut(), &mut out, npc, "@shutdown", json!({}))
            .is_err()
    );
}

#[test]
fn the_seed_makes_toy_roll_reproducible() {
    let faces = |seed: u64| -> Vec<u64> {
        let mut tw = TestWorld::with_modules(vec![Box::new(ToyRuleset::with_seed(seed))]);
        let npc = npc_in(&mut tw, "Bartender");
        seed_sheet(&mut tw, npc, |d| {
            d.set_number("strength", 4);
            d.set_number("brawl", 3);
        });
        let mut out = Outbox::new();
        let outcome = ToyRuleset::with_seed(seed)
            .invoke_ability(
                tw.world_mut(),
                &mut out,
                npc,
                "toy_roll",
                json!({ "pool_expr": "strength+brawl" }),
            )
            .expect("the ability runs");
        outcome.data["dice"]
            .as_array()
            .map(|a| a.iter().filter_map(serde_json::Value::as_u64).collect())
            .unwrap_or_default()
    };
    assert_eq!(faces(SEED), faces(SEED), "the same seed is the same dice");
    assert_ne!(faces(SEED), faces(SEED + 1));
}

#[test]
fn the_agent_profile_grounds_the_stored_sheet() {
    let toy = ToyRuleset::with_seed(SEED);
    let mut doc = SheetDoc::new_default(toy.sheet_schema());
    doc.set_text("template", "beta");
    doc.set_number("strength", 4);
    doc.set_number("brawl", 4);
    doc.set_number("willpower", 5);

    let profile = toy.agent_profile(&doc);
    assert!(profile.summary.contains("A beta."), "{}", profile.summary);
    assert!(profile.summary.contains("Best attribute Strength 4"));
    assert!(profile.summary.contains("Willpower 5"));
    assert!(profile.tags.iter().any(|t| t == "template:beta"));
    assert!(profile.tags.iter().any(|t| t == "combat:brawl"));
    assert_eq!(profile.stats["willpower"], 5);
    assert_eq!(
        profile.tools.first().map(|t| t.name.as_str()),
        Some("toy_roll")
    );
}

#[test]
fn a_willpower_backed_toy_roll_projects_onto_the_authoritative_action() {
    let mut tw = world();
    let nick = tw.spawn_player("Nick");
    seed_sheet(&mut tw, nick, |d| {
        d.set_number("strength", 3);
        d.set_number("brawl", 2);
        d.set_number("willpower", 2);
        d.set_number("willpower_current", 2);
    });
    let host = ability_host();

    // A plain roll stays on the legacy residual.
    assert!(
        host.projected_ability(
            tw.world(),
            nick,
            "toy_roll",
            &json!({ "pool_expr": "strength+brawl" })
        )
        .is_none()
    );
    // A jailbroken name maps to nothing.
    assert!(
        host.projected_ability(
            tw.world(),
            nick,
            "@shutdown",
            &json!({ "pool_expr": "strength", "willpower": true })
        )
        .is_none()
    );
    // A Willpower-backed one becomes the coordinator's own action.
    let projected = host
        .projected_ability(
            tw.world(),
            nick,
            "toy_roll",
            &json!({ "pool_expr": "strength+brawl", "willpower": true, "difficulty": 7 }),
        )
        .expect("a projected ability");
    assert_eq!(projected.action.key().id.as_str(), "toy_willpower_roll");
    assert_eq!(projected.parameters["pool"], "strength+brawl");
    assert_eq!(projected.parameters["difficulty"], 7);
    assert_eq!(projected.parameters["ignore_wounds"], false);
}

// ----------------------------------------------------------------------
// The authoritative Willpower roll
// ----------------------------------------------------------------------

fn install_action_keys(tw: &mut TestWorld) {
    tw.engine_mut().set_offer_keys(
        OfferKeyRing::new(OfferSecret::new(1, [0x41; 32]).expect("offer secret"), [])
            .expect("offer ring"),
    );
    tw.engine_mut().set_resolution_keys(
        ResolutionKeyRing::new(
            ResolutionSecret::new(1, [0x52; 32]).expect("resolution secret"),
            [],
        )
        .expect("resolution ring"),
    );
}

/// Whether `verb` is currently offered to `actor` over `subject`.
fn offers(tw: &TestWorld, actor: ObjId, subject: PerceivedRef, verb: &str) -> bool {
    let query = tw
        .engine()
        .legal_action_query(PrincipalId::Object { id: actor }, actor, subject)
        .expect("a legal-action query");
    tw.engine()
        .project_legal_actions(&query)
        .expect("a projection")
        .projected()
        .into_iter()
        .any(|offer| offer.presentation().verb == verb)
}

struct Committed {
    outcome: serde_json::Value,
    announced: Vec<(Audience, OutputEvent)>,
    replayable: bool,
    typed_writes: Vec<ActionStateBootRecord>,
}

/// Drive one projected offer through the engine's own authoritative lane (no
/// server coordinator) and install it the way the world task does.
fn execute(
    tw: &mut TestWorld,
    actor: ObjId,
    subject: PerceivedRef,
    verb: &str,
    parameters: serde_json::Value,
    byte: u8,
) -> Committed {
    let query = tw
        .engine()
        .legal_action_query(PrincipalId::Object { id: actor }, actor, subject)
        .expect("a legal-action query");
    let projection = tw
        .engine()
        .project_legal_actions(&query)
        .expect("a projection");
    let offer = projection
        .projected()
        .into_iter()
        .find(|offer| offer.presentation().verb == verb)
        .unwrap_or_else(|| panic!("missing projected {verb} offer"));
    let principal = PrincipalId::Object { id: actor };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: OwnerId::new("toy").expect("owner"),
            namespace: CanonicalId::new(verb.replace('-', "_")).expect("namespace"),
        },
        0,
        IdempotencyKey::Request(RequestUid::from_bytes([byte; 16])),
    );
    let resolved = tw
        .engine()
        .resolve_legal_action(
            &projection,
            OfferExecutionContext {
                request: RequestUid::from_bytes([byte; 16]),
                principal,
                actor: query.actor(),
                idempotency,
                viewer_scope: query.viewer_scope(),
                now: None,
            },
            OfferSubmission {
                offer: offer.offer(),
                subject: offer.subject().clone(),
                targets: Vec::new(),
                parameters,
            },
        )
        .expect("the offer resolves");
    let (action, resolved, _guards) = resolved.into_authoritative_parts();
    let token = CommitToken::from_bytes([byte; 16]);
    let (prepared, output) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(&action, &resolved, token)
        .expect("the provider prepares");
    let (outcome, resolution, _defs, _witnesses, _outbox, _events) = output.into_parts();
    let replayable = resolution.is_some_and(|r| {
        let bound = r.bind(token);
        bound.outcome() == &outcome && !bound.draws().is_empty()
    });
    let outcome = outcome.data().clone();
    let (ops, installation) = prepared.into_parts();
    // The typed post-images the server's boot partition would split out.
    let typed_writes = ops
        .iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, value }
                if is_action_state_persistence_key(key.as_str()) =>
            {
                Some(ActionStateBootRecord::new(
                    op.ns.as_str(),
                    key.as_str(),
                    value.clone(),
                ))
            }
            _ => None,
        })
        .collect();
    let announced = tw.install_prepared_action_announcing(installation);
    Committed {
        outcome,
        announced,
        replayable,
        typed_writes,
    }
}

#[test]
fn the_authoritative_roll_commits_once_per_point_and_republishes_the_subject() {
    let mut tw = world();
    let nick = tw.spawn_player("Nick");
    seed_sheet(&mut tw, nick, |d| {
        d.set_number("strength", 3);
        d.set_number("brawl", 2);
        d.set_number("willpower", 2);
        d.set_number("willpower_current", 2);
    });
    let _ = tw.world_mut().take_dirty();
    let _ = tw.take_module_ops();
    install_action_keys(&mut tw);

    let first = execute(
        &mut tw,
        nick,
        PerceivedRef::SelfActor,
        "toy-willpower-roll",
        json!({ "pool": "strength+brawl", "difficulty": 6 }),
        0x31,
    );
    assert_eq!(
        first.outcome["pool"], 5,
        "the provider sized the pool from the sheet"
    );
    assert_eq!(
        first.outcome["willpower_remaining"], 1,
        "exactly one point moved"
    );
    assert_eq!(first.outcome["roll_sequence"], 1);
    assert!(
        first.replayable,
        "the roll records a replayable resolution whose outcome is the committed one"
    );
    assert!(
        !first.typed_writes.is_empty(),
        "the commit carries a typed post-image"
    );

    // Interface plan I13: the commit republished the subject's vitals and
    // sheet through the overlay, to the subject ALONE.
    let vitals: Vec<_> = first
        .announced
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (Audience::Actor(a), OutputEvent::Panel { feed, kind, data })
                if *a == nick && feed.as_str() == "toy.vitals" =>
            {
                Some((*kind, data.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(vitals.len(), 1, "{:?}", first.announced);
    assert_eq!(
        vitals.first().map(|(k, _)| *k),
        Some(PanelFrameKind::Keyframe)
    );
    assert_eq!(
        vitals.first().and_then(|(_, data)| pool_current(data)),
        Some(1),
        "the spend is live in the republished frame"
    );
    let sheets = first
        .announced
        .iter()
        .filter(|(aud, ev)| {
            matches!(aud, Audience::Actor(a) if *a == nick)
                && matches!(ev, OutputEvent::Panel { feed, .. } if feed.as_str() == "toy.sheet")
        })
        .count();
    assert_eq!(sheets, 1, "{:?}", first.announced);
    assert!(
        !first
            .announced
            .iter()
            .any(|(aud, _)| matches!(aud, Audience::Actor(a) if *a == GOD)),
        "the republish is subject-only"
    );

    // The live queries read the same overlay; the generic (stored-only)
    // `toy.sheet` query deliberately still shows the legacy field.
    assert_eq!(
        pool_current(&query_panel(&mut tw, nick, "toy.vitals")),
        Some(1)
    );
    assert_eq!(
        query_panel(&mut tw, nick, "toy.sheet_live")["data"]["willpower_current"],
        1
    );
    assert_eq!(
        query_panel(&mut tw, nick, "toy.sheet")["data"]["willpower_current"],
        2,
        "the generic sheet query has no typed-state lens: ask toy.sheet_live"
    );

    // The second point commits; then the offer stops being projected.
    let second = execute(
        &mut tw,
        nick,
        PerceivedRef::SelfActor,
        "toy-willpower-roll",
        json!({ "pool": "strength+brawl", "difficulty": 6 }),
        0x32,
    );
    assert_eq!(second.outcome["willpower_remaining"], 0);
    assert_eq!(second.outcome["roll_sequence"], 2);
    assert!(
        !offers(&tw, nick, PerceivedRef::SelfActor, "toy-willpower-roll"),
        "an empty pool projects no offer"
    );

    // Staff restore the resource through the paired authoritative action.
    let restored = execute(
        &mut tw,
        GOD,
        PerceivedRef::KnownObject {
            id: nick,
            revision: 1,
        },
        "toy-willpower-set",
        json!({ "current": 2 }),
        0x33,
    );
    assert_eq!(restored.outcome["current"], 2);
    assert_eq!(
        restored.outcome["roll_sequence"], 2,
        "the roll counter is not reset"
    );
    assert!(offers(
        &tw,
        nick,
        PerceivedRef::SelfActor,
        "toy-willpower-roll"
    ));
}

#[test]
fn the_typed_post_image_rehydrates_into_a_fresh_composition() {
    let mut tw = world();
    let nick = tw.spawn_player("Nick");
    seed_sheet(&mut tw, nick, |d| {
        d.set_number("strength", 3);
        d.set_number("brawl", 2);
        d.set_number("willpower", 2);
        d.set_number("willpower_current", 2);
    });
    let _ = tw.world_mut().take_dirty();
    let _ = tw.take_module_ops();
    install_action_keys(&mut tw);

    let first = execute(
        &mut tw,
        nick,
        PerceivedRef::SelfActor,
        "toy-willpower-roll",
        json!({ "pool": "strength+brawl" }),
        0x51,
    );
    assert_eq!(first.outcome["willpower_remaining"], 1);

    // A fresh composition over a sheet that came back from the store with its
    // own stale value: the typed record still owns the pool.
    let mut rebooted = world();
    let reborn = rebooted.spawn_player("Nick");
    assert_eq!(reborn, nick, "the reboot restores the same object id");
    seed_sheet(&mut rebooted, reborn, |d| {
        d.set_number("willpower", 2);
        d.set_number("willpower_current", 2);
    });
    rebooted
        .engine_mut()
        .hydrate_action_state(first.typed_writes)
        .expect("the typed record hydrates into a fresh composition");

    assert_eq!(
        pool_current(&query_panel(&mut rebooted, reborn, "toy.vitals")),
        Some(1),
        "the typed record still owns the pool after the reboot"
    );
    assert_eq!(
        query_panel(&mut rebooted, reborn, "toy.sheet_live")["data"]["willpower_current"],
        1
    );
    assert_eq!(
        query_panel(&mut rebooted, reborn, "toy.sheet_live")["data"]["strength"],
        1,
        "the legacy half of the sheet survives beside the typed record"
    );
}

// ----------------------------------------------------------------------
// Two owners (the I13 pattern)
// ----------------------------------------------------------------------

#[test]
fn a_second_owner_may_share_the_family_name_without_publishing() {
    // The publisher plus a state-only twin: registration must not collide, and
    // only the publisher owns the `toy.vitals` feed.
    let tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ToyRuleset::with_seed(SEED)),
        Box::new(ToyRuleset::named("other").state_only()),
    ]);
    assert!(tw.registry().query_handler("toy.vitals").is_some());
    assert!(tw.registry().query_handler("other.vitals").is_none());
    assert!(tw.registry().query_handler("toy.sheet").is_some());
}

#[test]
fn a_real_sheet_query_reestablishes_the_next_push_keyframe() {
    let mut tw = world();
    let nick = tw.spawn_player("Nick");
    seed_sheet(&mut tw, nick, |_| {});
    let kind = |events: Vec<(Audience, OutputEvent)>| {
        events
            .into_iter()
            .find_map(|(_, ev)| match ev {
                OutputEvent::Panel { feed, kind, .. } if feed == "toy.sheet" => Some(kind),
                _ => None,
            })
            .expect("sheet republished")
    };
    assert_eq!(
        kind(tw.run(GOD, "@sheet/set Nick/strength=3")),
        PanelFrameKind::Keyframe
    );
    assert_eq!(
        kind(tw.run(GOD, "@sheet/set Nick/strength=4")),
        PanelFrameKind::Delta
    );
    query_panel(&mut tw, nick, "toy.sheet");
    assert_eq!(
        kind(tw.run(GOD, "@sheet/set Nick/strength=5")),
        PanelFrameKind::Keyframe
    );
}
