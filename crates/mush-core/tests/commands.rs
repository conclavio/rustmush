//! End-to-end command-layer tests driving the public API only: a real
//! `World`, the real `Registry` with every core command, and the treaty
//! ruling 5 dispatch pipeline — the M0 exit criterion
//! ("@dig/@desc/look/say/pose/movement in in-process tests").

use mush_core::{
    AliasOutcome, AliasStage, Audience, Flags, GOD, LIMBO, ObjId, ObjKind, Outbox, Powers,
    Registry, World, dispatch, register_core_commands,
};
use mush_proto::{NarrativeKind, OutputEvent};

/// A world with the core commands, a wizardly God, a builder Bob, and a
/// plain player Ann — everyone starting in Limbo.
struct Game {
    w: World,
    reg: Registry,
    ann: ObjId,
    bob: ObjId,
}

impl Game {
    fn new() -> Game {
        let mut w = World::new_minimal();
        let ann = w.create(ObjKind::Player, "Ann", GOD, Some(LIMBO)).unwrap();
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();
        w.set_flag(bob, Flags::BUILDER, true).unwrap();
        let mut reg = Registry::new();
        register_core_commands(&mut reg).unwrap();
        Game { w, reg, ann, bob }
    }

    fn run(&mut self, actor: ObjId, line: &str) -> Outbox {
        let mut out = Outbox::new();
        dispatch(&self.reg, &mut self.w, actor, line, &mut out);
        out
    }

    fn find(&self, name: &str) -> ObjId {
        self.w
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
            .unwrap_or_else(|| panic!("no object named {name:?}"))
            .id
    }

    fn loc(&self, id: ObjId) -> Option<ObjId> {
        self.w.get(id).and_then(|o| o.location)
    }
}

/// Plain-text projection of narrative-ish events.
fn plain(e: &OutputEvent) -> Option<String> {
    match e {
        OutputEvent::Sys(t) => Some(t.to_plain_text()),
        OutputEvent::Narrative { text, .. } => Some(text.to_plain_text()),
        _ => None,
    }
}

/// All Sys/Narrative texts addressed directly to `actor`.
fn actor_texts(out: &Outbox, actor: ObjId) -> Vec<String> {
    out.events()
        .iter()
        .filter(|(a, _)| *a == Audience::Actor(actor))
        .filter_map(|(_, e)| plain(e))
        .collect()
}

/// All Sys/Narrative texts addressed to a room audience on `room`.
fn room_texts(out: &Outbox, room: ObjId) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(a, e)| match a {
            Audience::Room { room: r, .. } if *r == room => plain(e),
            _ => None,
        })
        .collect()
}

fn assert_error_contains(out: &Outbox, actor: ObjId, needle: &str) {
    let texts = actor_texts(out, actor);
    assert!(
        texts.iter().any(|t| t.contains(needle)),
        "expected an actor message containing {needle:?}, got {texts:?}"
    );
}

// ---------------------------------------------------------------------
// Dispatch order
// ---------------------------------------------------------------------

#[test]
fn say_prefix_emits_room_narrative() {
    let mut g = Game::new();
    let ann = g.ann;
    let out = g.run(ann, "\"hello there");
    let hit = out.events().iter().any(|(a, e)| {
        matches!(
            (a, e),
            (
                Audience::Room { room, exclude },
                OutputEvent::Narrative { kind: NarrativeKind::Say, actor: Some(r), text },
            ) if *room == LIMBO
                && exclude.is_empty()
                && r.name == "Ann"
                && text.to_plain_text() == "hello there"
        )
    });
    assert!(hit, "missing say narrative: {:?}", out.events());
}

#[test]
fn pose_semipose_emit_prefixes() {
    let mut g = Game::new();
    let ann = g.ann;
    for (line, kind, text) in [
        (":grins.", NarrativeKind::Pose, "grins."),
        (
            ";'s eyes narrow.",
            NarrativeKind::SemiPose,
            "'s eyes narrow.",
        ),
        ("\\The wind howls.", NarrativeKind::Emit, "The wind howls."),
    ] {
        let out = g.run(ann, line);
        let hit = out.events().iter().any(|(_, e)| {
            matches!(e, OutputEvent::Narrative { kind: k, text: t, .. }
                if *k == kind && t.to_plain_text() == text)
        });
        assert!(hit, "{line:?} did not produce {kind:?}: {:?}", out.events());
    }
}

#[test]
fn exit_shadows_command() {
    let mut g = Game::new();
    let ann = g.ann;
    // An exit named "look" from Aroom to Broom (away from Limbo, so it is
    // not a master-room global exit).
    let a = g.w.create(ObjKind::Room, "Aroom", GOD, None).unwrap();
    let b = g.w.create(ObjKind::Room, "Broom", GOD, None).unwrap();
    let ex = g.w.create(ObjKind::Exit, "look", GOD, Some(a)).unwrap();
    g.w.link_exit(ex, Some(b)).unwrap();
    g.w.move_to(ann, a).unwrap();
    // Ann types "look": the exit wins over the look command.
    let out = g.run(ann, "look");
    assert_eq!(g.loc(ann), Some(b));
    assert!(
        out.events()
            .iter()
            .any(|(a2, e)| *a2 == Audience::Actor(ann)
                && matches!(e, OutputEvent::Room(rv) if rv.name == "Broom")),
        "expected arrival room view"
    );
    // No such exit in Broom: "look" is the command again, and Ann stays.
    let out = g.run(ann, "look");
    assert_eq!(g.loc(ann), Some(b));
    assert!(
        out.events()
            .iter()
            .any(|(_, e)| matches!(e, OutputEvent::Room(rv) if rv.name == "Broom")),
        "expected look to render the room"
    );
}

#[test]
fn master_room_exits_are_global() {
    let mut g = Game::new();
    let ann = g.ann;
    // An exit in Limbo (the master room) works from anywhere.
    let sanctum = g.w.create(ObjKind::Room, "Sanctum", GOD, None).unwrap();
    let portal =
        g.w.create(ObjKind::Exit, "portal", GOD, Some(LIMBO))
            .unwrap();
    g.w.link_exit(portal, Some(sanctum)).unwrap();
    let far = g.w.create(ObjKind::Room, "Far", GOD, None).unwrap();
    g.w.move_to(ann, far).unwrap();
    g.run(ann, "portal");
    assert_eq!(g.loc(ann), Some(sanctum));
}

#[test]
fn zone_exits_are_reachable() {
    let mut g = Game::new();
    let ann = g.ann;
    // A ZMO carrying an exit: members of the zone can use it.
    let zmo = g.w.create(ObjKind::Thing, "ZMO", GOD, Some(LIMBO)).unwrap();
    g.w.set_flag(zmo, Flags::ZONE_MASTER, true).unwrap();
    let plaza = g.w.create(ObjKind::Room, "Plaza", GOD, None).unwrap();
    let station = g.w.create(ObjKind::Room, "Station", GOD, None).unwrap();
    g.w.set_zone(plaza, Some(zmo)).unwrap();
    let tram = g.w.create(ObjKind::Exit, "tram", GOD, Some(zmo)).unwrap();
    g.w.link_exit(tram, Some(station)).unwrap();
    g.w.move_to(ann, plaza).unwrap();
    g.run(ann, "tram");
    assert_eq!(g.loc(ann), Some(station));
    // From an unzoned room the tram is invisible.
    g.run(GOD, "@teleport *Ann=#0");
    let out = g.run(ann, "tram");
    assert_error_contains(&out, ann, "Huh?");
}

#[test]
fn huh_with_fuzzy_suggestion() {
    let mut g = Game::new();
    let ann = g.ann;
    let out = g.run(ann, "lok around");
    assert_error_contains(&out, ann, "Huh?");
    assert_error_contains(&out, ann, "\"look\"");
    // Nothing close: plain Huh.
    let out = g.run(ann, "xyzzyfrobnicate");
    assert_error_contains(&out, ann, "Huh?");
}

#[test]
fn alias_stage_runs_before_exits() {
    struct Stage;
    impl AliasStage for Stage {
        fn try_alias(
            &self,
            _w: &mut World,
            actor: ObjId,
            line: &str,
            out: &mut Outbox,
        ) -> AliasOutcome {
            if line == "zap" {
                out.info(actor, "ZAPPED");
                AliasOutcome::Handled
            } else if line == "hi5" {
                AliasOutcome::Rewrite("say High five!".to_string())
            } else {
                AliasOutcome::NotMine
            }
        }
    }
    let mut g = Game::new();
    let ann = g.ann;
    g.reg.add_alias_stage(std::sync::Arc::new(Stage));
    // An exit also named "zap" exists — but the alias stage runs first.
    g.run(GOD, "@dig Pit=zap");
    let out = g.run(ann, "zap");
    assert_error_contains(&out, ann, "ZAPPED");
    assert_eq!(g.loc(ann), Some(LIMBO), "alias stage must preempt the exit");
    // Rewrite path funnels back into normal dispatch.
    let out = g.run(ann, "hi5");
    assert!(
        out.events().iter().any(|(_, e)| matches!(
            e,
            OutputEvent::Narrative { kind: NarrativeKind::Say, text, .. }
                if text.to_plain_text() == "High five!"
        )),
        "rewrite should become a say"
    );
}

#[test]
fn home_keyword_beats_exits() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(GOD, "@dig Trap=home"); // an exit cheekily named "home"
    let hall = g.w.create(ObjKind::Room, "Annhome", GOD, None).unwrap();
    g.w.set_home(ann, Some(hall)).unwrap();
    let out = g.run(ann, "home");
    assert_eq!(g.loc(ann), Some(hall), "HOME keyword must not be shadowed");
    assert_error_contains(&out, ann, "There's no place like home");
}

// ---------------------------------------------------------------------
// Movement + building
// ---------------------------------------------------------------------

#[test]
fn dig_movement_and_messages() {
    let mut g = Game::new();
    let ann = g.ann;
    let out = g.run(GOD, "@dig Hall=north;n,south;s");
    assert_error_contains(&out, GOD, "created with room number");
    let hall = g.find("Hall");

    // Ann moves through the alias.
    let out = g.run(ann, "n");
    assert_eq!(g.loc(ann), Some(hall));
    assert!(
        room_texts(&out, LIMBO)
            .iter()
            .any(|t| t == "Ann goes north."),
        "leave message: {:?}",
        room_texts(&out, LIMBO)
    );
    assert!(
        room_texts(&out, hall)
            .iter()
            .any(|t| t == "Ann has arrived."),
        "arrive message: {:?}",
        room_texts(&out, hall)
    );
    // And back via "goto".
    let out = g.run(ann, "goto south");
    assert_eq!(g.loc(ann), Some(LIMBO));
    let _ = out;
    // The room view from Limbo lists the exit with aliases.
    let out = g.run(ann, "look");
    let rv = out
        .events()
        .iter()
        .find_map(|(_, e)| match e {
            OutputEvent::Room(rv) => Some(rv.clone()),
            _ => None,
        })
        .expect("room view");
    let north = rv.exits.iter().find(|e| e.name == "north").expect("north");
    assert_eq!(north.aliases, vec!["n".to_string()]);
}

#[test]
fn locked_exit_blocks_with_fail_messages() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(GOD, "@dig Vault=north;n");
    let out = g.run(GOD, "@lock north=#1");
    assert_error_contains(&out, GOD, "Locked.");
    // Ann fails the lock and stays put.
    let out = g.run(ann, "north");
    assert_error_contains(&out, ann, "You can't go that way.");
    assert_eq!(g.loc(ann), Some(LIMBO));
    // Custom fail message via the exit's fail attribute.
    g.run(GOD, "@set north/fail=The door is barred.");
    let out = g.run(ann, "north");
    assert_error_contains(&out, ann, "The door is barred.");
    // God passes his own lock.
    g.run(GOD, "north");
    assert_eq!(g.loc(GOD), Some(g.find("Vault")));
}

#[test]
fn open_and_link() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(GOD, "@dig Den");
    let den = g.find("Den");
    let out = g.run(GOD, "@open west;w");
    assert_error_contains(&out, GOD, "Opened.");
    // Unlinked exit leads nowhere.
    let out = g.run(ann, "west");
    assert_error_contains(&out, ann, "That exit doesn't lead anywhere.");
    assert_eq!(g.loc(ann), Some(LIMBO));
    // Linked, it works.
    let out = g.run(GOD, &format!("@link west={den}"));
    assert_error_contains(&out, GOD, "Linked.");
    g.run(ann, "w");
    assert_eq!(g.loc(ann), Some(den));
}

#[test]
fn desc_and_look_detail() {
    let mut g = Game::new();
    let ann = g.ann;
    let out = g.run(GOD, "@desc here=A quiet grey void.");
    assert_error_contains(&out, GOD, "Set.");
    let out = g.run(ann, "look");
    assert!(out.events().iter().any(|(_, e)| matches!(
        e,
        OutputEvent::Room(rv) if rv.desc.to_plain_text() == "A quiet grey void."
    )));
    // look <keyword> via detail: attributes on the room.
    g.run(GOD, "@set here/detail:mist=Curls of silver mist.");
    let out = g.run(ann, "look mist");
    assert_error_contains(&out, ann, "Curls of silver mist.");
    // Unknown target still fails.
    let out = g.run(ann, "look unicorn");
    assert_error_contains(&out, ann, "I don't see that here.");
}

#[test]
fn builder_gating() {
    let mut g = Game::new();
    let ann = g.ann;
    let bob = g.bob;
    // Ann is not a builder.
    let out = g.run(ann, "@dig Shack");
    assert_error_contains(&out, ann, "Permission denied.");
    // Bob is.
    let out = g.run(bob, "@create widget");
    assert_error_contains(&out, bob, "Created:");
    let widget = g.find("widget");
    assert_eq!(g.loc(widget), Some(bob));
    assert_eq!(g.w.get(widget).unwrap().owner, bob);
    // Bob digs, but does not control Limbo: room ok, exits refused.
    let out = g.run(bob, "@dig Shack=door");
    assert_error_contains(&out, bob, "created with room number");
    assert_error_contains(&out, bob, "Permission denied.");
}

#[test]
fn dig_teleport_switch_and_validation() {
    let mut g = Game::new();
    let out = g.run(GOD, "@dig/teleport Loft");
    let loft = g.find("Loft");
    assert_eq!(g.loc(GOD), Some(loft));
    let _ = out;
    // Unknown switch is rejected before anything happens.
    let out = g.run(GOD, "@dig/fly Attic");
    assert_error_contains(&out, GOD, "Unknown switch \"/fly\"");
    assert!(g.w.iter().all(|o| o.name != "Attic"));
}

// ---------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------

#[test]
fn get_drop_inventory() {
    let mut g = Game::new();
    let ann = g.ann;
    let ball =
        g.w.create(ObjKind::Thing, "ball", GOD, Some(LIMBO))
            .unwrap();
    let out = g.run(ann, "get ball");
    assert_error_contains(&out, ann, "You take ball.");
    assert!(
        room_texts(&out, LIMBO)
            .iter()
            .any(|t| t == "Ann takes ball.")
    );
    assert_eq!(g.loc(ball), Some(ann));
    // Already carried.
    let out = g.run(ann, "get ball");
    assert_error_contains(&out, ann, "You already have that!");
    // Inventory lists it.
    let out = g.run(ann, "i");
    assert_error_contains(&out, ann, "You are carrying:");
    assert_error_contains(&out, ann, "ball");
    // Drop it.
    let out = g.run(ann, "drop ball");
    assert_error_contains(&out, ann, "You drop ball.");
    assert_eq!(g.loc(ball), Some(LIMBO));
    let out = g.run(ann, "inventory");
    assert_error_contains(&out, ann, "You aren't carrying anything.");
    // Can't pick up players.
    let out = g.run(ann, "get Bob");
    assert_error_contains(&out, ann, "You can't pick that up!");
}

#[test]
fn get_respects_basic_lock() {
    let mut g = Game::new();
    let ann = g.ann;
    g.w.create(ObjKind::Thing, "relic", GOD, Some(LIMBO))
        .unwrap();
    g.run(GOD, "@lock relic=#1");
    let out = g.run(ann, "get relic");
    assert_error_contains(&out, ann, "You can't pick that up.");
    let relic = g.find("relic");
    assert_eq!(g.loc(relic), Some(LIMBO));
    // God takes it fine.
    let out = g.run(GOD, "get relic");
    assert_error_contains(&out, GOD, "You take relic.");
}

#[test]
fn sticky_things_go_home_when_dropped() {
    let mut g = Game::new();
    let ann = g.ann;
    let shed = g.w.create(ObjKind::Room, "Shed", GOD, None).unwrap();
    let boomerang =
        g.w.create(ObjKind::Thing, "boomerang", GOD, Some(LIMBO))
            .unwrap();
    g.w.set_home(boomerang, Some(shed)).unwrap();
    g.w.set_flag(boomerang, Flags::STICKY, true).unwrap();
    g.run(ann, "get boomerang");
    g.run(ann, "drop boomerang");
    assert_eq!(g.loc(boomerang), Some(shed));
}

#[test]
fn give_and_give_lock() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    let coin = g.w.create(ObjKind::Thing, "coin", GOD, Some(ann)).unwrap();
    let out = g.run(ann, "give coin=Bob");
    assert_error_contains(&out, ann, "You give coin to Bob.");
    assert!(
        out.events().iter().any(|(a, e)| *a == Audience::Actor(bob)
            && plain(e).is_some_and(|t| t == "Ann gives you coin.")),
        "recipient line missing"
    );
    assert_eq!(g.loc(coin), Some(bob));
    // Bob locks receiving to God only; Ann's next gift bounces.
    let gem = g.w.create(ObjKind::Thing, "gem", GOD, Some(ann)).unwrap();
    g.run(bob, "@lock/give me=#1");
    let out = g.run(ann, "give gem=Bob");
    assert_error_contains(&out, ann, "They don't want that.");
    assert_eq!(g.loc(gem), Some(ann));
    // Giving to something absent fails.
    let out = g.run(ann, "give gem=nobody");
    assert_error_contains(&out, ann, "I don't see that here.");
}

// ---------------------------------------------------------------------
// Examine + perms
// ---------------------------------------------------------------------

#[test]
fn examine_gated_by_can_examine() {
    let mut g = Game::new();
    let ann = g.ann;
    g.w.create(ObjKind::Thing, "orb", GOD, Some(LIMBO)).unwrap();
    g.run(GOD, "@set orb/secret=the password");
    // Ann does not control the orb: owner line only.
    let out = g.run(ann, "examine orb");
    assert_error_contains(&out, ann, "is owned by God");
    let all = actor_texts(&out, ann).join("\n");
    assert!(!all.contains("the password"), "attrs must be hidden: {all}");
    // God sees the full dump.
    let out = g.run(GOD, "examine orb");
    let all = actor_texts(&out, GOD).join("\n");
    assert!(all.contains("Type: Thing"), "{all}");
    assert!(all.contains("secret [#1]: the password"), "{all}");
    // EXAMINE_ALL grants Ann the full view.
    g.w.get_mut(ann).unwrap().powers = Powers::EXAMINE_ALL;
    let out = g.run(ann, "examine orb");
    let all = actor_texts(&out, ann).join("\n");
    assert!(all.contains("the password"), "{all}");
}

#[test]
fn examine_hides_wizard_only_attrs_from_non_wizards() {
    let mut g = Game::new();
    let ann = g.ann;
    let box_ = g.w.create(ObjKind::Thing, "box", ann, Some(ann)).unwrap();
    let mut attr = mush_core::Attr::text("hush", GOD);
    attr.flags = mush_core::AttrFlags::WIZARD_ONLY;
    g.w.set_attr(box_, "staffnote", attr).unwrap();
    // Ann owns the box but is not a wizard.
    let out = g.run(ann, "examine box");
    let all = actor_texts(&out, ann).join("\n");
    assert!(!all.contains("hush"), "{all}");
    let out = g.run(GOD, &format!("examine #{}", box_.0));
    let all = actor_texts(&out, GOD).join("\n");
    assert!(all.contains("hush"), "{all}");
}

// ---------------------------------------------------------------------
// Speech: whisper + page
// ---------------------------------------------------------------------

#[test]
fn whisper_requires_same_room() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    let out = g.run(ann, "whisper Bob=psst");
    assert!(
        out.events().iter().any(|(a, e)| *a == Audience::Actor(bob)
            && matches!(e, OutputEvent::Narrative { kind: NarrativeKind::Whisper, text, .. }
                if text.to_plain_text() == "psst")),
        "whisper missing"
    );
    assert_error_contains(&out, ann, "You whisper \"psst\" to Bob.");
    // Bob leaves; whispering across rooms fails — and delivers NOTHING to
    // Bob (a refusal that still queued the line would leak privacy).
    let den = g.w.create(ObjKind::Room, "Den", GOD, None).unwrap();
    g.w.move_to(bob, den).unwrap();
    let out = g.run(ann, "whisper *Bob=psst");
    assert_error_contains(&out, ann, "They aren't here.");
    assert!(
        out.events().iter().all(|(a, _)| *a != Audience::Actor(bob)),
        "refused whisper must not deliver: {:?}",
        out.events()
    );
}

#[test]
fn page_honors_page_lock_and_haven() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    // Pages reach across rooms.
    let den = g.w.create(ObjKind::Room, "Den", GOD, None).unwrap();
    g.w.move_to(bob, den).unwrap();
    let out = g.run(ann, "page Bob=hi there");
    assert!(
        out.events().iter().any(|(a, e)| *a == Audience::Actor(bob)
            && matches!(e, OutputEvent::Narrative { kind: NarrativeKind::Page, text, .. }
                if text.to_plain_text() == "hi there")),
        "page missing"
    );
    assert_error_contains(&out, ann, "You paged Bob with 'hi there'.");
    // Bob locks pages to God. A refused page delivers NOTHING to Bob —
    // asserting only the error string would let a queue-then-refuse
    // regression leak private messages.
    g.run(bob, "@lock/page me=#1");
    let out = g.run(ann, "page Bob=hello?");
    assert_error_contains(&out, ann, "does not wish to be disturbed");
    assert!(
        out.events().iter().all(|(a, _)| *a != Audience::Actor(bob)),
        "refused page must not deliver: {:?}",
        out.events()
    );
    let out = g.run(GOD, "page Bob=status?");
    assert_error_contains(&out, GOD, "You paged Bob");
    // HAVEN refuses everyone — and delivers nothing.
    g.run(GOD, "@set Ann=HAVEN");
    let out = g.run(bob, "page Ann=you there?");
    assert_error_contains(&out, bob, "does not wish to be disturbed");
    assert!(
        out.events().iter().all(|(a, _)| *a != Audience::Actor(ann)),
        "refused page must not deliver: {:?}",
        out.events()
    );
}

// ---------------------------------------------------------------------
// @set — flag and attribute forms
// ---------------------------------------------------------------------

#[test]
fn set_flag_and_attr_forms() {
    let mut g = Game::new();
    let ann = g.ann;
    let ball =
        g.w.create(ObjKind::Thing, "ball", GOD, Some(LIMBO))
            .unwrap();
    // Flag on / off.
    let out = g.run(GOD, "@set ball=STICKY");
    assert_error_contains(&out, GOD, "Set.");
    assert!(g.w.get(ball).unwrap().flags.contains(Flags::STICKY));
    let out = g.run(GOD, "@set ball=!sticky");
    assert_error_contains(&out, GOD, "Cleared.");
    assert!(!g.w.get(ball).unwrap().flags.contains(Flags::STICKY));
    // Unknown flag.
    let out = g.run(GOD, "@set ball=SHINY");
    assert_error_contains(&out, GOD, "I don't understand the flag \"SHINY\".");
    // Ann does not control God's ball.
    let out = g.run(ann, "@set ball=STICKY");
    assert_error_contains(&out, ann, "Permission denied.");
    // Attribute form.
    let out = g.run(GOD, "@set ball/label=Property of God");
    assert_error_contains(&out, GOD, "label - Set.");
    assert_eq!(
        g.w.attr(ball, "label").unwrap().as_text(),
        Some("Property of God")
    );
    let out = g.run(GOD, "@set ball/label=");
    assert_error_contains(&out, GOD, "label - Cleared.");
    assert!(g.w.attr(ball, "label").is_none());
    // GOING is not settable by hand.
    let out = g.run(GOD, "@set ball=GOING");
    assert_error_contains(&out, GOD, "Use @destroy for that.");
}

#[test]
fn set_privileged_flags() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    // A wizard (not God) cannot grant WIZARD.
    let wiz =
        g.w.create(ObjKind::Player, "Wiz", GOD, Some(LIMBO))
            .unwrap();
    g.w.set_flag(wiz, Flags::WIZARD, true).unwrap();
    let out = g.run(wiz, "@set Ann=WIZARD");
    assert_error_contains(&out, wiz, "Permission denied.");
    assert!(!g.w.get(ann).unwrap().flags.contains(Flags::WIZARD));
    // God can.
    let out = g.run(GOD, "@set Ann=WIZARD");
    assert_error_contains(&out, GOD, "Set.");
    assert!(g.w.get(ann).unwrap().flags.contains(Flags::WIZARD));
    // BUILDER needs a wizard: Bob can't promote himself further.
    let out = g.run(bob, "@set Bob=ROYALTY");
    assert_error_contains(&out, bob, "Permission denied.");
    // WIZARD_ONLY attributes refuse non-wizards even on their own stuff.
    let chest = g.w.create(ObjKind::Thing, "chest", bob, Some(bob)).unwrap();
    let mut attr = mush_core::Attr::text("sealed", GOD);
    attr.flags = mush_core::AttrFlags::WIZARD_ONLY;
    g.w.set_attr(chest, "seal", attr).unwrap();
    let out = g.run(bob, "@set chest/seal=broken");
    assert_error_contains(&out, bob, "Permission denied.");
    assert_eq!(g.w.attr(chest, "seal").unwrap().as_text(), Some("sealed"));
}

// ---------------------------------------------------------------------
// @lock / @unlock
// ---------------------------------------------------------------------

#[test]
fn lock_switch_validation_and_unlock() {
    let mut g = Game::new();
    let bob = g.bob;
    let out = g.run(bob, "@lock/give me=#1");
    assert_error_contains(&out, bob, "Locked.");
    assert!(
        g.w.get(bob)
            .unwrap()
            .locks
            .contains_key(&mush_core::LockType::Give)
    );
    let out = g.run(bob, "@unlock/give me");
    assert_error_contains(&out, bob, "Unlocked.");
    assert!(
        !g.w.get(bob)
            .unwrap()
            .locks
            .contains_key(&mush_core::LockType::Give)
    );
    let out = g.run(bob, "@unlock me");
    assert_error_contains(&out, bob, "It wasn't locked.");
    // Bad switch.
    let out = g.run(bob, "@lock/frobnicate me=#1");
    assert_error_contains(&out, bob, "Unknown switch \"/frobnicate\"");
    // Bad key.
    let out = g.run(bob, "@lock me=((#1");
    assert_error_contains(&out, bob, "I don't understand that key.");
    // Locking someone else's object is denied.
    let out = g.run(bob, "@lock *Ann=#1");
    assert_error_contains(&out, bob, "Permission denied.");
}

// ---------------------------------------------------------------------
// @teleport / @zone / @parent / @chown
// ---------------------------------------------------------------------

#[test]
fn teleport_rules_and_messages() {
    let mut g = Game::new();
    let ann = g.ann;
    let open = g.w.create(ObjKind::Room, "Open", GOD, None).unwrap();
    let closed = g.w.create(ObjKind::Room, "Closed", GOD, None).unwrap();
    g.w.set_flag(open, Flags::JUMP_OK, true).unwrap();
    // Ann may jump to a JUMP_OK room...
    let out = g.run(ann, &format!("@teleport #{}", open.0));
    assert_eq!(g.loc(ann), Some(open));
    assert!(
        out.events()
            .iter()
            .any(|(a, e)| *a == Audience::Actor(ann) && matches!(e, OutputEvent::Room(_))),
        "teleport should show the destination"
    );
    // ...but not a closed one.
    let out = g.run(ann, &format!("@teleport #{}", closed.0));
    assert_error_contains(&out, ann, "Permission denied.");
    assert_eq!(g.loc(ann), Some(open));
    // God moves Ann anywhere (cross-room targets need *name).
    let out = g.run(GOD, &format!("@teleport *Ann=#{}", closed.0));
    assert_error_contains(&out, GOD, "Teleported.");
    assert_eq!(g.loc(ann), Some(closed));
}

#[test]
fn zone_and_parent() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(GOD, "@create Downtown");
    let zmo = g.find("Downtown");
    g.run(GOD, "@dig Bar");
    let bar = g.find("Bar");
    // Not a ZMO yet.
    let out = g.run(GOD, &format!("@zone #{}=#{}", bar.0, zmo.0));
    assert_error_contains(&out, GOD, "not a Zone Master Object");
    g.run(GOD, "@set Downtown=ZONE_MASTER");
    let out = g.run(GOD, &format!("@zone #{}=#{}", bar.0, zmo.0));
    assert_error_contains(&out, GOD, "Zoned.");
    assert_eq!(g.w.get(bar).unwrap().zone, Some(zmo));
    let out = g.run(GOD, &format!("@zone #{}=none", bar.0));
    assert_error_contains(&out, GOD, "Zone cleared.");
    assert_eq!(g.w.get(bar).unwrap().zone, None);

    // @parent: Ann controls neither bob's things nor God's.
    let proto = g.w.create(ObjKind::Thing, "proto", ann, Some(ann)).unwrap();
    let toy = g.w.create(ObjKind::Thing, "toy", ann, Some(ann)).unwrap();
    let out = g.run(ann, "@parent toy=proto");
    assert_error_contains(&out, ann, "Parent set.");
    assert_eq!(g.w.get(toy).unwrap().parent, Some(proto));
    let out = g.run(ann, &format!("@parent toy=#{}", zmo.0));
    assert_error_contains(&out, ann, "Permission denied.");
    let out = g.run(ann, "@parent toy=none");
    assert_error_contains(&out, ann, "Parent cleared.");
    assert_eq!(g.w.get(toy).unwrap().parent, None);
}

#[test]
fn chown_is_power_gated() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    let gizmo =
        g.w.create(ObjKind::Thing, "gizmo", bob, Some(LIMBO))
            .unwrap();
    // No power: the command gate itself refuses.
    let out = g.run(ann, "@chown gizmo=Ann");
    assert_error_contains(&out, ann, "Permission denied.");
    assert_eq!(g.w.get(gizmo).unwrap().owner, bob);
    // With CHOWN_ANY it works.
    g.w.get_mut(ann).unwrap().powers = Powers::CHOWN_ANY;
    let out = g.run(ann, "@chown gizmo=Ann");
    assert_error_contains(&out, ann, "Owner changed.");
    assert_eq!(g.w.get(gizmo).unwrap().owner, ann);
    // Owners must be players; players own themselves.
    let out = g.run(ann, "@chown gizmo=gizmo");
    assert_error_contains(&out, ann, "Owners must be players.");
    let out = g.run(GOD, "@chown Bob=Ann");
    assert_error_contains(&out, GOD, "Players always own themselves.");
}

// ---------------------------------------------------------------------
// @destroy
// ---------------------------------------------------------------------

#[test]
fn destroy_semantics() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    g.run(bob, "@create crate");
    let crate_ = g.find("crate");
    // Ann doesn't control it (referenced by dbref since it's in Bob's arms).
    let out = g.run(ann, &format!("@destroy #{}", crate_.0));
    assert_error_contains(&out, ann, "Permission denied.");
    // Bob destroys his own: GOING set, husk swept later.
    let out = g.run(bob, "@destroy crate");
    assert_error_contains(&out, bob, "Destroyed.");
    assert!(g.w.get(crate_).unwrap().flags.contains(Flags::GOING));
    let swept = g.w.sweep();
    assert!(swept.contains(&crate_));
    assert!(g.w.get(crate_).is_none());

    // SAFE blocks; /override is wizard-only.
    g.run(bob, "@create vault");
    let vault = g.find("vault");
    g.run(bob, "@set vault=SAFE");
    let out = g.run(bob, "@destroy vault");
    assert_error_contains(&out, bob, "marked SAFE");
    let out = g.run(bob, "@destroy/override vault");
    assert_error_contains(&out, bob, "Permission denied.");
    assert!(!g.w.get(vault).unwrap().flags.contains(Flags::GOING));
    let out = g.run(GOD, &format!("@destroy/override #{}", vault.0));
    assert_error_contains(&out, GOD, "Destroyed.");
    assert!(g.w.get(vault).unwrap().flags.contains(Flags::GOING));

    // Destroying players is wizard-only.
    let out = g.run(ann, "@destroy me");
    assert_error_contains(&out, ann, "Only wizards may destroy players.");
    let _ = ann;
}

// ---------------------------------------------------------------------
// @find / WHO / help
// ---------------------------------------------------------------------

#[test]
fn find_is_owner_scoped() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    g.w.create(ObjKind::Thing, "red pen", ann, Some(ann))
        .unwrap();
    g.w.create(ObjKind::Thing, "red flag", bob, Some(bob))
        .unwrap();
    let out = g.run(ann, "@find red");
    let all = actor_texts(&out, ann).join("\n");
    assert!(all.contains("red pen"), "{all}");
    assert!(
        !all.contains("red flag"),
        "non-staff sees only their own: {all}"
    );
    assert!(all.contains("END OF LIST"), "{all}");
    // Wizards see everything; glob patterns work.
    let out = g.run(GOD, "@find red*");
    let all = actor_texts(&out, GOD).join("\n");
    assert!(all.contains("red pen") && all.contains("red flag"), "{all}");
}

#[test]
fn who_lists_connected_players() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    g.w.set_connected(ann, true);
    g.w.set_connected(bob, true);
    g.w.set_connected(GOD, true);
    g.w.set_flag(GOD, Flags::DARK, true).unwrap();
    // Ann sees Bob and herself, not the DARK wizard, no locations.
    let out = g.run(ann, "WHO");
    let who = out
        .events()
        .iter()
        .find_map(|(_, e)| match e {
            OutputEvent::Who(v) => Some(v.clone()),
            _ => None,
        })
        .expect("who event");
    let names: Vec<_> = who.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Ann", "Bob"]);
    assert!(who.iter().all(|e| e.location.is_none()));
    // God sees all three, with locations.
    let out = g.run(GOD, "who");
    let who = out
        .events()
        .iter()
        .find_map(|(_, e)| match e {
            OutputEvent::Who(v) => Some(v.clone()),
            _ => None,
        })
        .expect("who event");
    assert_eq!(who.len(), 3);
    assert!(who.iter().all(|e| e.location.as_deref() == Some("Limbo")));
    // Disconnect drops Bob from the list.
    g.w.set_connected(bob, false);
    let out = g.run(ann, "WHO");
    let who = out
        .events()
        .iter()
        .find_map(|(_, e)| match e {
            OutputEvent::Who(v) => Some(v.clone()),
            _ => None,
        })
        .expect("who event");
    assert_eq!(who.len(), 1);
}

#[test]
fn help_renders_specs() {
    let mut g = Game::new();
    let ann = g.ann;
    let out = g.run(ann, "help @dig");
    let all = actor_texts(&out, ann).join("\n");
    assert!(all.contains("@dig"), "{all}");
    assert!(all.contains("Create a new room"), "{all}");
    assert!(all.contains("/teleport"), "{all}");
    // Listing.
    let out = g.run(ann, "help");
    let all = actor_texts(&out, ann).join("\n");
    assert!(
        all.contains("look") && all.contains("@dig") && all.contains("WHO"),
        "{all}"
    );
    // +help is the same command.
    let out = g.run(ann, "+help say");
    assert_error_contains(&out, ann, "Speak to everyone");
    // Unknown topic.
    let out = g.run(ann, "help frobnicate");
    assert_error_contains(&out, ann, "No help available");
}

// ---------------------------------------------------------------------
// Privacy: inventory, DARK, hidden attributes
// ---------------------------------------------------------------------

#[test]
fn look_at_actor_hides_inventory() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    g.w.create(ObjKind::Thing, "love letter", ann, Some(ann))
        .unwrap();
    // Bob looking at Ann sees her desc but never her carry.
    let out = g.run(bob, "look Ann");
    let all = actor_texts(&out, bob).join("\n");
    assert!(!all.contains("love letter"), "inventory leaked: {all}");
    assert!(!all.contains("Contents:"), "inventory leaked: {all}");
    // A thing's contents still show (shop shelf).
    let shelf =
        g.w.create(ObjKind::Thing, "shelf", GOD, Some(LIMBO))
            .unwrap();
    g.w.create(ObjKind::Thing, "mug", GOD, Some(shelf)).unwrap();
    let out = g.run(bob, "look shelf");
    let all = actor_texts(&out, bob).join("\n");
    assert!(all.contains("Contents:") && all.contains("mug"), "{all}");
    // A controller (God) may still inspect an actor's carry.
    let out = g.run(GOD, "look Ann");
    let all = actor_texts(&out, GOD).join("\n");
    assert!(all.contains("love letter"), "{all}");
}

#[test]
fn dark_on_actors_is_wizard_only() {
    let mut g = Game::new();
    let ann = g.ann;
    // Unprivileged invisibility is refused.
    let out = g.run(ann, "@set me=DARK");
    assert_error_contains(&out, ann, "Permission denied.");
    assert!(!g.w.get(ann).unwrap().flags.contains(Flags::DARK));
    // DARK on your own thing stays an ordinary builder tool.
    let orb = g.w.create(ObjKind::Thing, "orb", ann, Some(ann)).unwrap();
    let out = g.run(ann, "@set orb=DARK");
    assert_error_contains(&out, ann, "Set.");
    assert!(g.w.get(orb).unwrap().flags.contains(Flags::DARK));
    // Wizards may hide players — and the player may clear it again.
    let out = g.run(GOD, "@set Ann=DARK");
    assert_error_contains(&out, GOD, "Set.");
    let out = g.run(ann, "@set me=!DARK");
    assert_error_contains(&out, ann, "Cleared.");
    assert!(!g.w.get(ann).unwrap().flags.contains(Flags::DARK));
}

#[test]
fn examine_honors_hidden_and_self_only() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    let mut hidden = mush_core::Attr::text("tucked away", GOD);
    hidden.flags = mush_core::AttrFlags::HIDDEN;
    g.w.set_attr(bob, "stash", hidden).unwrap();
    let mut self_only = mush_core::Attr::text("bob's diary", GOD);
    self_only.flags = mush_core::AttrFlags::SELF_ONLY;
    g.w.set_attr(bob, "diary", self_only).unwrap();
    g.w.get_mut(ann).unwrap().powers = Powers::EXAMINE_ALL;

    // A non-staff EXAMINE_ALL holder sees neither.
    let out = g.run(ann, "examine Bob");
    let all = actor_texts(&out, ann).join("\n");
    assert!(!all.contains("tucked away"), "HIDDEN leaked: {all}");
    assert!(!all.contains("bob's diary"), "SELF_ONLY leaked: {all}");
    // /all reveals HIDDEN but never SELF_ONLY (still not self or staff).
    let out = g.run(ann, "examine/all Bob");
    let all = actor_texts(&out, ann).join("\n");
    assert!(all.contains("tucked away"), "{all}");
    assert!(!all.contains("bob's diary"), "{all}");
    // The object itself reads its own SELF_ONLY attribute.
    let out = g.run(bob, "examine me");
    let all = actor_texts(&out, bob).join("\n");
    assert!(all.contains("bob's diary"), "{all}");
    // Staff with /all see everything.
    let out = g.run(GOD, "examine/all Bob");
    let all = actor_texts(&out, GOD).join("\n");
    assert!(
        all.contains("tucked away") && all.contains("bob's diary"),
        "{all}"
    );
}

#[test]
fn locked_attrs_refuse_non_setters() {
    let mut g = Game::new();
    let bob = g.bob;
    // God plants a LOCKED attribute on Bob's own chest.
    let chest = g.w.create(ObjKind::Thing, "chest", bob, Some(bob)).unwrap();
    let mut attr = mush_core::Attr::text("wax seal", GOD);
    attr.flags = mush_core::AttrFlags::LOCKED;
    g.w.set_attr(chest, "seal", attr).unwrap();
    // Bob controls the chest but did not set the attribute: locked.
    let out = g.run(bob, "@set chest/seal=broken");
    assert_error_contains(&out, bob, "That attribute is locked.");
    assert_eq!(g.w.attr(chest, "seal").unwrap().as_text(), Some("wax seal"));
    // Clearing is refused too.
    let out = g.run(bob, "@set chest/seal=");
    assert_error_contains(&out, bob, "That attribute is locked.");
    assert!(g.w.attr(chest, "seal").is_some());
    // The setter (a wizard here) may change it (by dbref; the chest is in
    // Bob's arms, out of God's matcher range).
    let out = g.run(GOD, &format!("@set #{}/seal=renewed", chest.0));
    assert_error_contains(&out, GOD, "seal - Set.");
}

// ---------------------------------------------------------------------
// @name
// ---------------------------------------------------------------------

#[test]
fn name_renames_and_replaces_aliases() {
    let mut g = Game::new();
    let ann = g.ann;
    g.w.add_alias(ann, "Annie").unwrap();
    let out = g.run(ann, "@name me=Anna;An");
    assert_error_contains(&out, ann, "Name set.");
    let o = g.w.get(ann).unwrap();
    assert_eq!(o.name, "Anna");
    assert_eq!(
        o.aliases
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["An".to_string()]
    );
    assert_eq!(g.w.resolve_player("anna"), Some(ann));
    assert_eq!(g.w.resolve_player("an"), Some(ann));
    assert_eq!(g.w.resolve_player("annie"), None);
    assert_eq!(g.w.resolve_player("ann"), None);
}

#[test]
fn name_collisions_refuse_atomically_in_mush_voice() {
    let mut g = Game::new();
    let ann = g.ann;
    g.w.add_alias(ann, "Annie").unwrap();
    // A trailing alias collision must refuse BEFORE anything mutates.
    let out = g.run(ann, "@name me=Anna;Bob");
    assert_error_contains(&out, ann, "That name is already in use.");
    let o = g.w.get(ann).unwrap();
    assert_eq!(o.name, "Ann", "no partial rename");
    assert!(
        o.aliases.iter().any(|a| a == "Annie"),
        "aliases untouched: {:?}",
        o.aliases
    );
    // Renaming straight onto another player's name: same voice, no change.
    let out = g.run(ann, "@name me=Bob");
    assert_error_contains(&out, ann, "That name is already in use.");
    assert_eq!(g.w.get(ann).unwrap().name, "Ann");
    // Non-controllers are denied.
    let out = g.run(ann, "@name Bob=Robert");
    assert_error_contains(&out, ann, "Permission denied.");
}

// ---------------------------------------------------------------------
// Movement failure voice + custom messages
// ---------------------------------------------------------------------

#[test]
fn move_failures_are_mush_voice_and_unannounced() {
    let mut g = Game::new();
    let ann = g.ann;
    // Exit whose destination was destroyed after linking: walking it fails
    // politely, and the room never hears a departure that didn't happen.
    g.run(GOD, "@dig Doomed=down;d");
    let doomed = g.find("Doomed");
    g.w.destroy(doomed).unwrap();
    g.w.sweep();
    let out = g.run(ann, "down");
    assert_error_contains(&out, ann, "You can't go that way.");
    assert!(
        room_texts(&out, LIMBO).is_empty(),
        "no departure announced: {:?}",
        room_texts(&out, LIMBO)
    );
    assert_eq!(g.loc(ann), Some(LIMBO));
    // @teleport into something you carry: MUSH voice, no raw cycle error,
    // no phantom "has left".
    g.w.create(ObjKind::Thing, "orb", GOD, Some(LIMBO)).unwrap();
    g.run(GOD, "get orb");
    let out = g.run(GOD, "@teleport orb");
    assert_error_contains(&out, GOD, "You can't go there.");
    let all = actor_texts(&out, GOD).join("\n");
    assert!(
        !all.contains("containment cycle"),
        "raw WorldError leaked: {all}"
    );
    assert!(
        room_texts(&out, LIMBO)
            .iter()
            .all(|t| !t.contains("has left")),
        "no departure spam: {:?}",
        room_texts(&out, LIMBO)
    );
}

#[test]
fn custom_move_messages() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(GOD, "@dig Garden=gate");
    let garden = g.find("Garden");
    g.run(GOD, "@set gate/succ=You slip through the gate.");
    g.run(GOD, "@set gate/osucc=slips through the gate.");
    g.run(GOD, "@set gate/odrop=arrives smelling of roses.");
    let out = g.run(ann, "gate");
    assert_eq!(g.loc(ann), Some(garden));
    assert_error_contains(&out, ann, "You slip through the gate.");
    assert!(
        room_texts(&out, LIMBO)
            .iter()
            .any(|t| t == "Ann slips through the gate."),
        "osucc: {:?}",
        room_texts(&out, LIMBO)
    );
    assert!(
        room_texts(&out, garden)
            .iter()
            .any(|t| t == "Ann arrives smelling of roses."),
        "odrop: {:?}",
        room_texts(&out, garden)
    );
}

// ---------------------------------------------------------------------
// Speech lock + lock-type contract
// ---------------------------------------------------------------------

#[test]
fn speech_lock_gates_room_speech() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    let out = g.run(GOD, "@lock/speech here=#1");
    assert_error_contains(&out, GOD, "Locked.");
    for line in [
        "say hi",
        ":waves.",
        "\\A hush falls.",
        "semipose 's lips move.",
    ] {
        let out = g.run(ann, line);
        assert_error_contains(&out, ann, "You can't speak here.");
        assert!(
            room_texts(&out, LIMBO).is_empty(),
            "narrative escaped a speech lock: {:?}",
            out.events()
        );
    }
    // God passes his own lock.
    let out = g.run(GOD, "say order restored");
    assert!(
        room_texts(&out, LIMBO)
            .iter()
            .any(|t| t == "order restored"),
        "{:?}",
        room_texts(&out, LIMBO)
    );
    // Whisper and page are not room speech.
    let out = g.run(ann, "whisper Bob=psst");
    assert_error_contains(&out, ann, "You whisper \"psst\" to Bob.");
    let _ = bob;
}

#[test]
fn only_enforced_lock_types_are_offered() {
    let mut g = Game::new();
    // Lock types nothing checks yet must not be settable (a "Locked."
    // placebo would be a silent no-op security control).
    for t in ["enter", "use", "mail"] {
        let out = g.run(GOD, &format!("@lock/{t} me=#1"));
        assert_error_contains(&out, GOD, &format!("Unknown switch \"/{t}\""));
    }
    // Every offered type really locks.
    for t in [
        "basic", "drop", "give", "page", "speech", "teleport", "zone",
    ] {
        let out = g.run(GOD, &format!("@lock/{t} me=#1"));
        assert_error_contains(&out, GOD, "Locked.");
    }
}

// ---------------------------------------------------------------------
// page targeting, @link forms, drop/give locks, zones, ambiguity
// ---------------------------------------------------------------------

#[test]
fn page_dbref_must_target_an_actor() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    // Paging a room by dbref is refused (no lock probing, no ghost pages).
    let out = g.run(ann, "page #0=hello?");
    assert_error_contains(&out, ann, "I don't recognize that player.");
    // A real actor by dbref works.
    let out = g.run(ann, &format!("page #{}=hi", bob.0));
    assert_error_contains(&out, ann, "You paged Bob");
}

#[test]
fn link_home_form_requires_authority() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(GOD, "@dig Sanctum");
    let sanctum = g.find("Sanctum");
    // Ann may not home herself into a wizard's private room.
    let out = g.run(ann, &format!("@link me=#{}", sanctum.0));
    assert_error_contains(&out, ann, "Permission denied.");
    assert_eq!(g.w.get(ann).unwrap().home, Some(LIMBO));
    // JUMP_OK doubles as link-ok (same gate as the exit form).
    g.run(GOD, &format!("@set #{}=JUMP_OK", sanctum.0));
    let out = g.run(ann, &format!("@link me=#{}", sanctum.0));
    assert_error_contains(&out, ann, "Home set.");
    assert_eq!(g.w.get(ann).unwrap().home, Some(sanctum));
}

#[test]
fn link_exit_must_target_room() {
    let mut g = Game::new();
    g.run(GOD, "@open west;w");
    let out = g.run(GOD, "@link west=Bob"); // a player, not a room
    assert_error_contains(&out, GOD, "Exits can only be linked to rooms.");
}

#[test]
fn drop_lock_via_command() {
    let mut g = Game::new();
    let ann = g.ann;
    g.w.create(ObjKind::Thing, "cursed ring", ann, Some(ann))
        .unwrap();
    let out = g.run(ann, "@lock/drop cursed ring=#1"); // only God may drop it
    assert_error_contains(&out, ann, "Locked.");
    let out = g.run(ann, "drop cursed ring");
    assert_error_contains(&out, ann, "You can't drop that.");
    assert_eq!(g.loc(g.find("cursed ring")), Some(ann));
}

#[test]
fn give_to_enter_ok_thing() {
    let mut g = Game::new();
    let ann = g.ann;
    let coin = g.w.create(ObjKind::Thing, "coin", ann, Some(ann)).unwrap();
    let machine =
        g.w.create(ObjKind::Thing, "vending machine", GOD, Some(LIMBO))
            .unwrap();
    // Not ENTER_OK: refused.
    let out = g.run(ann, "give coin=vending machine");
    assert_error_contains(&out, ann, "You can't give things to that.");
    assert_eq!(g.loc(coin), Some(ann));
    // ENTER_OK opens the vending seam.
    g.w.set_flag(machine, Flags::ENTER_OK, true).unwrap();
    let out = g.run(ann, "give coin=vending machine");
    assert_error_contains(&out, ann, "You give coin to vending machine.");
    assert_eq!(g.loc(coin), Some(machine));
}

#[test]
fn zone_lock_admits_non_controller_through_zone_cmd() {
    let mut g = Game::new();
    let ann = g.ann;
    let zmo =
        g.w.create(ObjKind::Thing, "District", GOD, Some(LIMBO))
            .unwrap();
    g.w.set_flag(zmo, Flags::ZONE_MASTER, true).unwrap();
    let toy = g.w.create(ObjKind::Thing, "toy", ann, Some(ann)).unwrap();
    // Ann controls the toy but not the ZMO, and there is no zone lock yet.
    let out = g.run(ann, "@zone toy=District");
    assert_error_contains(&out, ann, "Permission denied.");
    // The ZMO's @lock/zone admits Ann: now the command passes.
    g.w.get_mut(zmo)
        .unwrap()
        .locks
        .insert(mush_core::LockType::Zone, mush_core::LockExpr::Is(ann));
    let out = g.run(ann, "@zone toy=District");
    assert_error_contains(&out, ann, "Zoned.");
    assert_eq!(g.w.get(toy).unwrap().zone, Some(zmo));
}

#[test]
fn ambiguous_object_match_reports_classic_error() {
    let mut g = Game::new();
    let ann = g.ann;
    g.w.create(ObjKind::Thing, "sword", GOD, Some(LIMBO))
        .unwrap();
    g.w.create(ObjKind::Thing, "sword", GOD, Some(LIMBO))
        .unwrap();
    let out = g.run(ann, "get sword");
    assert_error_contains(&out, ann, "I don't know which one you mean!");
}

#[test]
fn room_contents_use_primary_name() {
    let mut g = Game::new();
    let ann = g.ann;
    g.w.create(ObjKind::Thing, "lever;switch", GOD, Some(LIMBO))
        .unwrap();
    let out = g.run(ann, "look");
    let rv = out
        .events()
        .iter()
        .find_map(|(_, e)| match e {
            OutputEvent::Room(rv) => Some(rv.clone()),
            _ => None,
        })
        .expect("room view");
    assert!(
        rv.contents.iter().any(|c| c.name == "lever"),
        "{:?}",
        rv.contents
    );
    assert!(rv.contents.iter().all(|c| c.name != "lever;switch"));
}

// ---------------------------------------------------------------------
// Misc dispatch details
// ---------------------------------------------------------------------

#[test]
fn think_is_private_and_l_abbreviates_look() {
    let mut g = Game::new();
    let ann = g.ann;
    let out = g.run(ann, "think must remember the ritual");
    assert!(out.events().iter().all(|(a, _)| *a == Audience::Actor(ann)));
    assert_error_contains(&out, ann, "must remember the ritual");
    // "l" (exact alias) and "lo" (unambiguous prefix) both reach look.
    for line in ["l", "lo"] {
        let out = g.run(ann, line);
        assert!(
            out.events()
                .iter()
                .any(|(_, e)| matches!(e, OutputEvent::Room(_))),
            "{line:?} should render the room"
        );
    }
}

#[test]
fn empty_and_missing_args_error_cleanly() {
    let mut g = Game::new();
    let ann = g.ann;
    let out = g.run(ann, "say");
    assert_error_contains(&out, ann, "Say what?");
    let out = g.run(ann, "whisper Bob");
    assert_error_contains(&out, ann, "Whisper what?");
    let out = g.run(ann, "get");
    assert_error_contains(&out, ann, "Get what?");
    let out = g.run(ann, "give coin");
    assert_error_contains(&out, ann, "Give what to whom?");
    let out = g.run(ann, "@teleport");
    assert_error_contains(&out, ann, "Teleport where?");
}
