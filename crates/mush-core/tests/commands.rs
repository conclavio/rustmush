//! End-to-end command-layer tests driving the public API only: a real
//! `World`, the real `Registry` with every core command, and the treaty
//! ruling 5 dispatch pipeline — the M0 exit criterion
//! ("@dig/@desc/look/say/pose/movement in in-process tests").

use mush_core::{
    AliasOutcome, AliasStage, Attr, AttrFlags, Audience, Flags, GOD, LIMBO, ObjId, ObjKind, Outbox,
    Powers, Registry, World, dispatch, register_core_commands,
};
use mush_proto::{NarrativeKind, OutputEvent};

/// A store-less lens for this harness: it holds no engine and never has, so
/// it names `Test` rather than reaching for a store-less twin.
const DETACHED: mush_core::TypedStateLens<'static> =
    mush_core::TypedStateLens::Detached(mush_core::DetachedTypedState::Test);

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
        dispatch(&self.reg, &mut self.w, actor, line, &mut out, DETACHED);
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
                Audience::Room { room, exclude, .. },
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
    // `@emit` is a granted licence: without SPOOF a player is refused and
    // the room hears nothing. Say and pose need no licence because they
    // always carry the speaker's name.
    let out = g.run(ann, "\\The wind howls.");
    assert_error_contains(&out, ann, "Permission denied");
    assert!(room_texts(&out, LIMBO).is_empty(), "{:?}", out.events());
    g.w.set_flag(ann, Flags::SPOOF, true).unwrap();
    for (line, kind, text) in [
        (":grins.", NarrativeKind::Pose, "grins."),
        (
            ";'s eyes narrow.",
            NarrativeKind::SemiPose,
            "'s eyes narrow.",
        ),
        // Below Wizard the emit travels as a spoof so the fan-out can show
        // its source to entitled listeners.
        ("\\The wind howls.", NarrativeKind::Spoof, "The wind howls."),
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

/// The dbref walk. `look #N` for every id in the database must answer the
/// same line as a name that matched nothing, unless the target is in reach.
/// Before the locality gate, `#N` and `*name` resolved globally in
/// `match_object` and `look` gated only on perception (a free pass with no
/// Perceive hook), so `#0..#next` read every desc and every room view.
#[test]
fn look_dbref_walk_reads_nothing_out_of_reach() {
    let mut g = Game::new();
    let ann = g.ann;
    let bob = g.bob;
    // Off-site: a room with a desc, a thing in it, and Bob standing there.
    let vault = g.w.create(ObjKind::Room, "Vault", GOD, None).unwrap();
    let relic =
        g.w.create(ObjKind::Thing, "relic", GOD, Some(vault))
            .unwrap();
    g.w.move_to(bob, vault).unwrap();
    g.run(
        GOD,
        &format!("@desc #{}=Rows of numbered drawers.", vault.0),
    );
    g.run(GOD, &format!("@set #{}/desc=A cracked seal.", relic.0));
    // In reach: Ann's room, a thing she carries, a thing on the floor.
    let coin = g.w.create(ObjKind::Thing, "coin", GOD, Some(ann)).unwrap();
    let rug = g.w.create(ObjKind::Thing, "rug", GOD, Some(LIMBO)).unwrap();

    // The refusal every out-of-reach dbref must reproduce byte for byte.
    let baseline = actor_texts(&g.run(ann, "look zzzznotathing"), ann);
    assert_eq!(baseline, vec!["I don't see that here.".to_string()]);

    // GOD starts in Limbo alongside Ann, so it is in reach like the rest.
    let in_reach = [ann, GOD, LIMBO, coin, rug];
    let ids: Vec<ObjId> = g.w.iter().map(|o| o.id).collect();
    for id in ids {
        let out = g.run(ann, &format!("look #{}", id.0));
        if in_reach.contains(&id) {
            assert_ne!(
                actor_texts(&out, ann),
                baseline,
                "#{} is in reach and must still render",
                id.0
            );
            continue;
        }
        assert_eq!(
            actor_texts(&out, ann),
            baseline,
            "#{} answered something other than the NoMatch line",
            id.0
        );
        assert!(
            !out.events()
                .iter()
                .any(|(_, e)| matches!(e, OutputEvent::Room(_))),
            "#{} leaked a room view",
            id.0
        );
    }
    // A dbref past the end of the database reads the same.
    let past_end = g.w.iter().map(|o| o.id.0).max().unwrap() + 7;
    assert_eq!(
        actor_texts(&g.run(ann, &format!("look #{past_end}")), ann),
        baseline
    );

    // `me`, `here` and plain names are untouched by the gate.
    assert_ne!(actor_texts(&g.run(ann, "look me"), ann), baseline);
    assert_ne!(actor_texts(&g.run(ann, "look coin"), ann), baseline);
    assert_ne!(actor_texts(&g.run(ann, "look rug"), ann), baseline);
    assert!(
        g.run(ann, "look here")
            .events()
            .iter()
            .any(|(_, e)| matches!(e, OutputEvent::Room(_))),
        "look here still renders the room"
    );
    // And the `detail:` keyword fallback still runs after a failed match.
    g.run(GOD, "@set here/detail:mist=Curls of silver mist.");
    assert_error_contains(&g.run(ann, "look mist"), ann, "Curls of silver mist.");
}

/// `*name` is the same oracle by another spelling: it resolves through the
/// global player index, so it gets the same locality test.
#[test]
fn look_star_name_is_gated_by_locality() {
    let mut g = Game::new();
    let ann = g.ann;
    let bob = g.bob;
    let vault = g.w.create(ObjKind::Room, "Vault", GOD, None).unwrap();
    let baseline = actor_texts(&g.run(ann, "look zzzznotathing"), ann);

    // Same room: `*Bob` renders.
    let out = actor_texts(&g.run(ann, "look *Bob"), ann);
    assert_ne!(out, baseline, "a player in the room still renders");
    assert!(
        out.join(
            "
"
        )
        .contains("Bob"),
        "{out:?}"
    );

    // Elsewhere: the same refusal a missing name gives.
    g.w.move_to(bob, vault).unwrap();
    assert_eq!(
        actor_texts(&g.run(ann, "look *Bob"), ann),
        baseline,
        "*name must not reach across the map"
    );
    // Nor may it distinguish an absent player from a non-existent one.
    assert_eq!(actor_texts(&g.run(ann, "look *Nobody"), ann), baseline);
}

/// Staff keep the walk: `can_examine` (control, or the `EXAMINE_ALL` power)
/// is the exemption, so the gate hides nothing a staffer could already read.
#[test]
fn look_locality_exempts_examine_all() {
    let mut g = Game::new();
    let ann = g.ann;
    let bob = g.bob;
    let vault = g.w.create(ObjKind::Room, "Vault", GOD, None).unwrap();
    g.run(
        GOD,
        &format!("@desc #{}=Rows of numbered drawers.", vault.0),
    );
    g.w.move_to(bob, vault).unwrap();
    let baseline = actor_texts(&g.run(ann, "look zzzznotathing"), ann);
    assert_eq!(
        actor_texts(&g.run(ann, &format!("look #{}", vault.0)), ann),
        baseline
    );

    g.w.get_mut(ann).unwrap().powers = Powers::EXAMINE_ALL;
    assert!(
        g.run(ann, &format!("look #{}", vault.0))
            .events()
            .iter()
            .any(|(_, e)| matches!(e, OutputEvent::Room(_))),
        "EXAMINE_ALL still reads a distant room"
    );
    let out = actor_texts(&g.run(ann, "look *Bob"), ann);
    assert_ne!(out, baseline, "EXAMINE_ALL still reads a distant player");
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
    // Bob asks for a room plus exit, but does not control Limbo. The command
    // validates the complete request before allocating anything: failure is
    // atomic rather than leaving an unannounced orphan room behind.
    let out = g.run(bob, "@dig Shack=door");
    assert_error_contains(&out, bob, "Permission denied.");
    assert!(g.w.iter().all(|object| object.name != "Shack"));
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

/// An over-deep key is refused at the command layer the same way any other
/// unparseable key is — a message to the player — and leaves no lock behind.
///
/// The three shapes are the three the parser bounds separately: a run of
/// `(` and a run of `!` (which recurse), and a flat `&` chain a full input
/// line long (which does not recurse but builds an unbounded tree).
#[test]
fn over_deep_lock_keys_are_refused_and_mutate_nothing() {
    let mut g = Game::new();
    let bob = g.bob;
    let keys = [
        "(".repeat(8192),
        "!".repeat(8192),
        format!("{}#1", "#1&".repeat(2730)),
        format!("{}#1", "#1|".repeat(2730)),
    ];
    for key in keys {
        let out = g.run(bob, &format!("@lock me={key}"));
        assert_error_contains(&out, bob, "I don't understand that key.");
        assert!(
            g.w.get(bob).unwrap().locks.is_empty(),
            "an over-deep key must leave no lock on the object"
        );
    }
    // The refusal is specific: it names the depth limit, not "bad object
    // reference" or a trailing-input byte offset.
    let out = g.run(bob, &format!("@lock me={}#1", "#1&".repeat(2730)));
    assert_error_contains(&out, bob, "nests too deeply");
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
    // Ann sees Bob and herself, not the DARK wizard. Bob shares her room, so
    // his location shows and he is `here` (interface plan I3, D-I-12).
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
    assert!(
        who.iter()
            .all(|e| e.here && e.location.as_deref() == Some("Limbo"))
    );
    // Once Bob is elsewhere, Ann learns neither where he is nor that he moved
    // — only that he is not here.
    let attic = g.w.create(ObjKind::Room, "Attic", GOD, None).unwrap();
    g.w.move_to(bob, attic).unwrap();
    let out = g.run(ann, "WHO");
    let who = out
        .events()
        .iter()
        .find_map(|(_, e)| match e {
            OutputEvent::Who(v) => Some(v.clone()),
            _ => None,
        })
        .expect("who event");
    let bob_row = who.iter().find(|e| e.name == "Bob").unwrap();
    assert!(!bob_row.here && bob_row.location.is_none());
    assert!(who.iter().find(|e| e.name == "Ann").unwrap().here);
    // God sees all three, with every location; Ann shares Limbo with God.
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
    assert!(who.iter().all(|e| e.location.is_some()));
    assert_eq!(
        who.iter()
            .find(|e| e.name == "Bob")
            .unwrap()
            .location
            .as_deref(),
        Some("Attic")
    );
    assert_eq!(who.iter().filter(|e| e.here).count(), 2, "God and Ann");
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
    let (ann, bob) = (g.ann, g.bob);
    // Bob holds BUILDER, so @dig is his to run and his to read about.
    let out = g.run(bob, "help @dig");
    let all = actor_texts(&out, bob).join("\n");
    assert!(all.contains("@dig"), "{all}");
    assert!(all.contains("Create a new room"), "{all}");
    assert!(all.contains("/teleport"), "{all}");
    // Listing: everyone sees the ungated verbs; only the builder sees @dig.
    let out = g.run(ann, "help");
    let all = actor_texts(&out, ann).join("\n");
    assert!(all.contains("look") && all.contains("WHO"), "{all}");
    let out = g.run(bob, "help");
    let all = actor_texts(&out, bob).join("\n");
    assert!(all.contains("@dig"), "{all}");
    // +help is the same command.
    let out = g.run(ann, "+help say");
    assert_error_contains(&out, ann, "Speak to everyone");
    // Unknown topic.
    let out = g.run(ann, "help frobnicate");
    assert_error_contains(&out, ann, "No help available");
}

/// The listing names only what the actor could actually dispatch, and a
/// refusal for an out-of-role verb is indistinguishable from an unknown
/// name — `help` is not an enumeration of, or an oracle over, the staff
/// surface.
#[test]
fn help_listing_is_role_filtered_and_never_confirms_a_gated_verb() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    let wiz =
        g.w.create(ObjKind::Player, "Wiz", GOD, Some(LIMBO))
            .unwrap();
    g.w.set_flag(wiz, Flags::WIZARD, true).unwrap();

    let listing = |g: &mut Game, who: ObjId| -> Vec<String> {
        let out = g.run(who, "help");
        actor_texts(&out, who)
            .join("\n")
            .split_whitespace()
            .map(ToString::to_string)
            .collect()
    };
    let player = listing(&mut g, ann);
    let builder = listing(&mut g, bob);
    let wizard = listing(&mut g, wiz);

    // A plain player's listing names no Builder- or Wizard-gated verb.
    assert!(player.iter().any(|n| n == "look"), "{player:?}");
    assert!(!player.iter().any(|n| n == "@dig"), "{player:?}");
    assert!(!player.iter().any(|n| n == "@wall"), "{player:?}");
    // Each staff step is a strict superset of the one below it.
    assert!(builder.iter().any(|n| n == "@dig"), "{builder:?}");
    assert!(!builder.iter().any(|n| n == "@wall"), "{builder:?}");
    assert!(wizard.iter().any(|n| n == "@wall"), "{wizard:?}");
    for name in &player {
        assert!(builder.contains(name), "builder lost {name}: {builder:?}");
    }
    for name in &builder {
        assert!(wizard.contains(name), "wizard lost {name}: {wizard:?}");
    }

    // The refusal for a real-but-gated verb is the refusal an unknown name
    // gets, word for word: no existence oracle.
    let gated = g.run(ann, "help @wall");
    let unknown = g.run(ann, "help frobnicate");
    assert_error_contains(&gated, ann, "No help available");
    assert_eq!(
        actor_texts(&gated, ann),
        vec!["No help available for \"@wall\".".to_string()]
    );
    assert_eq!(
        actor_texts(&unknown, ann),
        vec!["No help available for \"frobnicate\".".to_string()]
    );
    // And the wizard still reads it.
    let out = g.run(wiz, "help @wall");
    assert!(
        actor_texts(&out, wiz).join("\n").contains("Wizards only"),
        "{:?}",
        actor_texts(&out, wiz)
    );
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
    // Licensed to emit, so the lock is what refuses her below.
    g.w.set_flag(ann, Flags::SPOOF, true).unwrap();
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

/// Put `name`=`value` on `id` carrying `flags`, owned by God. `@set/flag` is
/// the in-game writer, but it is staff-only for the read guards and refuses
/// module-owned namespaces, so probes that want a flagged `wod:*` attribute
/// still set it through the World.
fn set_flagged_attr(g: &mut Game, id: ObjId, name: &str, value: &str, flags: AttrFlags) {
    let mut a = Attr::text(value, GOD);
    a.flags = flags;
    g.w.set_attr(id, name, a).unwrap();
}

/// A world where Ann carries `wod:secret`=`value` under `flags` and Limbo's
/// speech lock is `key`; returns what Ann gets back for trying to speak.
fn speech_probe(value: &str, flags: AttrFlags, key: &str) -> (Vec<String>, Vec<String>) {
    let mut g = Game::new();
    let ann = g.ann;
    set_flagged_attr(&mut g, ann, "wod:secret", value, flags);
    let set = g.run(GOD, &format!("@lock/speech here={key}"));
    assert_error_contains(&set, GOD, "Locked.");
    let out = g.run(ann, "say hi");
    (actor_texts(&out, ann), room_texts(&out, LIMBO))
}

#[test]
fn a_lock_key_cannot_probe_a_read_restricted_attribute() {
    // The unflagged baseline: an attribute lock is a real gate, and it is the
    // *value* that decides. This is the oracle the flags have to close.
    let (hit, room) = speech_probe("Ventrue", AttrFlags::empty(), "wod:secret:V*");
    assert!(
        hit.is_empty(),
        "an unflagged match must still pass: {hit:?}"
    );
    assert!(room.iter().any(|t| t == "hi"), "{room:?}");
    let (miss, room) = speech_probe("Tremere", AttrFlags::empty(), "wod:secret:V*");
    assert!(
        miss.iter().any(|t| t == "You can't speak here."),
        "an unflagged miss must still refuse: {miss:?}"
    );
    assert!(room.is_empty(), "{room:?}");

    for flag in [AttrFlags::WIZARD_ONLY, AttrFlags::SELF_ONLY] {
        // A key over a read-restricted attribute is refused, and refused
        // *identically* whether the value matches the glob or not: the two
        // outcomes differed above, so if they still differ here the lock is a
        // one-bit read of an attribute `examine` would not show.
        let (hit, hit_room) = speech_probe("Ventrue", flag, "wod:secret:V*");
        let (miss, miss_room) = speech_probe("Tremere", flag, "wod:secret:V*");
        assert!(
            hit.iter().any(|t| t == "You can't speak here."),
            "{flag:?} must fail closed: {hit:?}"
        );
        assert_eq!(hit, miss, "{flag:?} leaks the value through the refusal");
        assert!(
            hit_room.is_empty() && miss_room.is_empty(),
            "speech escaped"
        );

        // Negated, the term must still be false — not true. `Tremere` does
        // not match `V*`, so an ordinary `!` key would open here; the
        // restricted read has to poison the `!` above it instead of handing
        // back a plain `false` for it to invert.
        let (neg, neg_room) = speech_probe("Tremere", flag, "!wod:secret:V*");
        assert!(
            neg.iter().any(|t| t == "You can't speak here."),
            "{flag:?} negated into a pass: {neg:?}"
        );
        assert!(neg_room.is_empty(), "{neg_room:?}");

        // And it poisons the key it sits in rather than reading as one false
        // term: a disjunction whose other branch is reached only after the
        // restricted read does not open either.
        let (dis, dis_room) = speech_probe("Tremere", flag, "wod:secret:V*|#TRUE");
        assert!(
            dis.iter().any(|t| t == "You can't speak here."),
            "{flag:?} was absorbed as a plain false: {dis:?}"
        );
        assert!(dis_room.is_empty(), "{dis_room:?}");
    }

    // HIDDEN is the documented carve-out: `examine/all` shows a hidden
    // attribute to any examiner with no role test, so it is a display flag,
    // not a confidentiality boundary — and honouring it would let a player
    // defeat every attribute lock aimed at them by flagging their own
    // attribute. It still matches. Changing that is a deliberate act.
    let (hidden, room) = speech_probe("Ventrue", AttrFlags::HIDDEN, "wod:secret:V*");
    assert!(hidden.is_empty(), "HIDDEN must still match: {hidden:?}");
    assert!(room.iter().any(|t| t == "hi"), "{room:?}");
}

#[test]
fn a_page_lock_cannot_probe_a_read_restricted_attribute() {
    // Bob is a builder, not staff: he cannot `examine` Ann's WIZARD_ONLY
    // attribute, so he must not be able to spell it out one `@lock/page` at a
    // time either. Whether his page arrives is the bit he would be reading.
    for flag in [AttrFlags::WIZARD_ONLY, AttrFlags::SELF_ONLY] {
        let mut refusals = Vec::new();
        for value in ["Ventrue", "Tremere"] {
            let mut g = Game::new();
            let (ann, bob) = (g.ann, g.bob);
            set_flagged_attr(&mut g, ann, "wod:secret", value, flag);
            g.run(bob, "@lock/page me=wod:secret:V*");
            let out = g.run(ann, "page Bob=hello?");
            assert_error_contains(&out, ann, "does not wish to be disturbed");
            assert!(
                out.events().iter().all(|(a, _)| *a != Audience::Actor(bob)),
                "a refused page must deliver nothing: {:?}",
                out.events()
            );
            refusals.push(actor_texts(&out, ann));
        }
        assert_eq!(
            refusals[0], refusals[1],
            "{flag:?} leaks the value through the page refusal"
        );
    }
    // Unflagged, the same key still works as a lock in both directions.
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    set_flagged_attr(&mut g, ann, "wod:secret", "Ventrue", AttrFlags::empty());
    g.run(bob, "@lock/page me=wod:secret:V*");
    let out = g.run(ann, "page Bob=hello?");
    assert_error_contains(&out, ann, "You paged Bob");
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

// ---------------------------------------------------------------------
// @ignore delivery shaping (M3): silent page/whisper gag
// ---------------------------------------------------------------------

#[test]
fn ignored_page_and_whisper_drop_silently() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    // Bob mutes Ann.
    let out = g.run(bob, "@ignore Ann");
    assert_error_contains(&out, bob, "You are now ignoring Ann.");

    // Ann pages Bob: Ann gets the NORMAL confirmation (never told she is
    // ignored); Bob receives nothing at all.
    let out = g.run(ann, "page Bob=secret ping");
    assert_error_contains(&out, ann, "You paged Bob with 'secret ping'.");
    assert!(
        actor_texts(&out, bob).is_empty(),
        "gagged page must deliver nothing to the target: {:?}",
        out.events()
    );

    // Whisper likewise (same room).
    let out = g.run(ann, "whisper Bob=psst");
    assert_error_contains(&out, ann, "You whisper \"psst\" to Bob.");
    assert!(
        actor_texts(&out, bob).is_empty(),
        "gagged whisper must deliver nothing to the target"
    );

    // Staff cannot be gagged: a page from God delivers even if Bob's list
    // somehow contained him (adding staff is refused up front anyway).
    let out = g.run(GOD, "page Bob=staff line");
    assert!(
        actor_texts(&out, bob).iter().any(|t| t == "staff line"),
        "staff pages must always deliver"
    );

    // Lifting the mute restores delivery.
    g.run(bob, "@ignore/clear Ann");
    let out = g.run(ann, "page Bob=hello again");
    assert!(
        actor_texts(&out, bob).iter().any(|t| t == "hello again"),
        "cleared mute must deliver again: {:?}",
        out.events()
    );
}

// ---------------------------------------------------------------------
// Help topics (M3): prose topics with command precedence
// ---------------------------------------------------------------------

#[test]
fn help_serves_topics_with_command_precedence() {
    let mut g = Game::new();
    let ann = g.ann;
    g.reg
        .add_help_topic("building", "Building Things", "Dig rooms with @dig.");
    // A topic deliberately shadowing a command name: the command must win.
    g.reg
        .add_help_topic("look", "WRONG", "This must never be served.");

    // Topic lookup (case-insensitive).
    let out = g.run(ann, "help BUILDING");
    assert_error_contains(&out, ann, "Building Things");
    assert_error_contains(&out, ann, "Dig rooms with @dig.");

    // Exact command name wins over the identically named topic.
    let out = g.run(ann, "help look");
    let texts = actor_texts(&out, ann);
    assert!(
        texts.iter().any(|t| t.starts_with("look")),
        "command help expected: {texts:?}"
    );
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("This must never be served")),
        "topic must not shadow the command: {texts:?}"
    );

    // Bare help lists commands AND topics.
    let out = g.run(ann, "help");
    let texts = actor_texts(&out, ann);
    assert!(
        texts.iter().any(|t| t.contains("Topics")),
        "topic section expected: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("building")),
        "topic name expected in listing: {texts:?}"
    );

    // Unknown names still error.
    let out = g.run(ann, "help nonesuch");
    assert_error_contains(&out, ann, "No help available");
}

// ---- Interface plan I11: the notice seam + the page producer ---------------

#[test]
fn notice_seam_bounds_text_on_a_char_boundary_mints_monotonic_ids_and_maps_the_action() {
    let mut out = mush_core::Outbox::new();
    // Two bytes per char: twice the cap, so the cut must land on a boundary.
    let long = "é".repeat(mush_proto::MAX_NOTICE_TEXT_BYTES);
    out.notice_to_actor(
        GOD,
        1234,
        mush_proto::NoticeSeverity::Warn,
        "alert",
        long,
        Some(("Open", "@econ/alerts #7")),
    );
    out.notice_info_to_actor(GOD, 1235, "page", "short", None);
    let notices: Vec<_> = out
        .events()
        .iter()
        .filter_map(|(a, e)| match e {
            OutputEvent::Notice {
                id,
                severity,
                category,
                text,
                action,
                ts,
            } if *a == Audience::Actor(GOD) => Some((
                *id,
                *severity,
                category.to_string(),
                text.to_plain_text(),
                action.clone(),
                *ts,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(notices.len(), 2);
    let (id0, sev0, cat0, text0, act0, ts0) = &notices[0];
    assert_eq!(*sev0, mush_proto::NoticeSeverity::Warn);
    assert_eq!(cat0, "alert");
    assert!(
        text0.len() <= mush_proto::MAX_NOTICE_TEXT_BYTES,
        "bounded: {} bytes",
        text0.len()
    );
    assert!(
        !text0.is_empty() && text0.chars().all(|c| c == 'é'),
        "cut on a char boundary, never mid-char"
    );
    assert_eq!(
        act0.as_ref()
            .map(|a| (a.label.as_str(), a.prefill.as_str())),
        Some(("Open", "@econ/alerts #7"))
    );
    assert_eq!(*ts0, 1234);
    let (id1, sev1, cat1, text1, act1, _) = &notices[1];
    assert!(id1 > id0, "ids are monotonic");
    assert_eq!(*sev1, mush_proto::NoticeSeverity::Info);
    assert_eq!(cat1, "page");
    assert_eq!(text1, "short");
    assert!(act1.is_none());
}

#[test]
fn a_page_lands_a_reply_ready_notice_on_the_target_never_the_sender_nor_via_an_ignored_sender() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    let out = g.run(ann, "page Bob=hi there");
    let notice = out.events().iter().find_map(|(a, e)| match e {
        OutputEvent::Notice {
            category,
            text,
            action,
            ..
        } if *a == Audience::Actor(bob) && category.as_str() == "page" => {
            Some((text.to_plain_text(), action.clone()))
        }
        _ => None,
    });
    let (text, action) = notice.expect("a page lands a notice on the target");
    assert!(text.contains("pages: hi there"), "{text}");
    assert_eq!(
        action.as_ref().map(|a| a.prefill.as_str()),
        Some("page Ann="),
        "the reply is ready to type"
    );
    assert!(
        !out.events()
            .iter()
            .any(|(a, e)| *a == Audience::Actor(ann) && matches!(e, OutputEvent::Notice { .. })),
        "nothing on the sender's surface"
    );
    // An ignored sender's page is dropped silently — no line, no notice.
    g.run(bob, "@ignore Ann");
    let out = g.run(ann, "page Bob=still there?");
    assert!(
        !out.events()
            .iter()
            .any(|(a, e)| *a == Audience::Actor(bob) && matches!(e, OutputEvent::Notice { .. })),
        "an ignored sender's page never notifies"
    );
}

// ---------------------------------------------------------------------
// @set/flag: the writer for attribute flags
// ---------------------------------------------------------------------

/// The flags on `id`'s attribute `name`, or a panic if it is gone.
fn attr_flags(g: &Game, id: ObjId, name: &str) -> AttrFlags {
    g.w.get(id)
        .and_then(|o| o.attrs.get(name))
        .unwrap_or_else(|| panic!("no {name} on {id}"))
        .flags
}

#[test]
fn a_wizard_can_flag_an_attribute_wizard_only_and_take_it_back() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(ann, "@set me/diary=dear diary");
    let out = g.run(GOD, "@set/flag Ann/diary=wizard_only");
    assert_error_contains(&out, GOD, "diary - WIZARD_ONLY set.");
    assert_eq!(attr_flags(&g, ann, "diary"), AttrFlags::WIZARD_ONLY);

    // The owner is not exempt: WIZARD_ONLY is withheld from her own examine,
    // with or without /all.
    let all = actor_texts(&g.run(ann, "examine me"), ann).join("\n");
    assert!(!all.contains("dear diary"), "{all}");
    let all = actor_texts(&g.run(ann, "examine/all me"), ann).join("\n");
    assert!(!all.contains("dear diary"), "{all}");
    // Staff see the value and the flag that is hiding it.
    let all = actor_texts(&g.run(GOD, "examine/all Ann"), GOD).join("\n");
    assert!(all.contains("dear diary"), "{all}");
    assert!(
        all.contains("WIZARD_ONLY"),
        "examine must name the flag: {all}"
    );

    let out = g.run(GOD, "@set/flag Ann/diary=!wizard_only");
    assert_error_contains(&out, GOD, "diary - WIZARD_ONLY cleared.");
    assert!(attr_flags(&g, ann, "diary").is_empty());
    let all = actor_texts(&g.run(ann, "examine me"), ann).join("\n");
    assert!(
        all.contains("dear diary"),
        "clearing must give it back: {all}"
    );
}

#[test]
fn a_player_cannot_set_or_clear_the_read_guards_on_their_own_attribute() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(ann, "@set me/diary=dear diary");
    // She controls the attribute and may rewrite its value, but the two bits
    // a lock key refuses to read through are not hers to move — otherwise
    // every attribute lock aimed at her is one `@set/flag` from useless.
    for spelling in ["wizard_only", "WIZARD_ONLY", "self_only", "Self_Only"] {
        let out = g.run(ann, &format!("@set/flag me/diary={spelling}"));
        assert_error_contains(&out, ann, "Permission denied.");
    }
    assert!(attr_flags(&g, ann, "diary").is_empty());

    // And not in the clearing direction either, once staff have set one.
    g.run(GOD, "@set/flag Ann/diary=self_only");
    let out = g.run(ann, "@set/flag me/diary=!self_only");
    assert_error_contains(&out, ann, "Permission denied.");
    assert_eq!(attr_flags(&g, ann, "diary"), AttrFlags::SELF_ONLY);
}

#[test]
fn self_only_set_by_staff_hides_from_other_examiners_but_not_the_object() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    g.run(ann, "@set me/diary=dear diary");
    g.w.get_mut(bob).unwrap().powers = Powers::EXAMINE_ALL;
    let all = actor_texts(&g.run(bob, "examine/all Ann"), bob).join("\n");
    assert!(all.contains("dear diary"), "baseline: {all}");

    let out = g.run(GOD, "@set/flag Ann/diary=self_only");
    assert_error_contains(&out, GOD, "diary - SELF_ONLY set.");
    let all = actor_texts(&g.run(bob, "examine/all Ann"), bob).join("\n");
    assert!(!all.contains("dear diary"), "SELF_ONLY leaked: {all}");
    let all = actor_texts(&g.run(ann, "examine me"), ann).join("\n");
    assert!(
        all.contains("dear diary"),
        "the object still reads it: {all}"
    );
}

#[test]
fn a_player_may_flag_their_own_attribute_hidden_without_moving_a_lock_over_it() {
    // HIDDEN stays a controller tool, which is only safe because the lock
    // layer ignores it: flagging it must not change whether a third party's
    // key opens, in the plain form or the negated one.
    let mut outcomes = Vec::new();
    for key in ["secret:V*", "!secret:V*"] {
        let mut g = Game::new();
        let ann = g.ann;
        g.run(ann, "@set me/secret=Ventrue");
        let set = g.run(GOD, &format!("@lock/speech here={key}"));
        assert_error_contains(&set, GOD, "Locked.");
        let before = room_texts(&g.run(ann, "say hi"), LIMBO);

        let out = g.run(ann, "@set/flag me/secret=hidden");
        assert_error_contains(&out, ann, "secret - HIDDEN set.");
        assert_eq!(attr_flags(&g, ann, "secret"), AttrFlags::HIDDEN);

        let after = room_texts(&g.run(ann, "say hi"), LIMBO);
        assert_eq!(before, after, "HIDDEN moved the {key} lock: {before:?}");
        outcomes.push(after);
    }
    // Not vacuous: the two keys really do decide differently.
    assert!(outcomes[0].iter().any(|t| t == "hi"), "{outcomes:?}");
    assert!(outcomes[1].is_empty(), "{outcomes:?}");
}

#[test]
fn an_attribute_flag_survives_a_value_write_and_reaches_the_write_batch() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(ann, "@set me/diary=dear diary");
    let _ = g.w.take_dirty();
    let out = g.run(ann, "@set/flag me/diary=hidden");
    assert_error_contains(&out, ann, "diary - HIDDEN set.");
    assert!(
        g.w.take_dirty().contains(&ann),
        "a flag write must dirty the object, or the batch never carries it"
    );

    // The ordinary value path keeps the flag (the `existing_flags` carry).
    let out = g.run(ann, "@set me/diary=second entry");
    assert_error_contains(&out, ann, "diary - Set.");
    assert_eq!(attr_flags(&g, ann, "diary"), AttrFlags::HIDDEN);

    // And the persisted object doc re-hydrates with it.
    let json = serde_json::to_string(g.w.get(ann).unwrap()).unwrap();
    let back: mush_core::Obj = serde_json::from_str(&json).unwrap();
    let attr = back.attrs.get("diary").unwrap();
    assert!(attr.flags.contains(AttrFlags::HIDDEN), "{json}");
    assert_eq!(
        attr.value,
        mush_core::AttrValue::Text("second entry".to_string())
    );
}

#[test]
fn set_flag_refuses_a_missing_attribute_an_unknown_name_and_the_object_form() {
    let mut g = Game::new();
    let ann = g.ann;
    let out = g.run(ann, "@set/flag me/diary=hidden");
    assert_error_contains(&out, ann, "diary - No such attribute.");
    g.run(ann, "@set me/diary=dear diary");
    let out = g.run(ann, "@set/flag me/diary=sneaky");
    assert_error_contains(&out, ann, "I don't understand the attribute flag");
    let out = g.run(ann, "@set/flag me/diary=");
    assert_error_contains(&out, ann, "Set which attribute flag?");
    // `/flag` over the object-flag form is a typo, not a DARK write.
    let out = g.run(ann, "@set/flag me=DARK");
    assert_error_contains(&out, ann, "object flag, not an attribute flag");
    assert!(!g.w.get(ann).unwrap().flags.contains(Flags::DARK));
    assert!(attr_flags(&g, ann, "diary").is_empty());
}
