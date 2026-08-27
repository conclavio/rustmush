//! The M0 acceptance suite (docs/PLAN.md, milestone M0 demo criteria),
//! driven end to end through `mush-testkit`: real dispatch, real command
//! handlers, real `mush-proto` events — no sockets, no tokio.

use mush_core::{Attr, GOD, LIMBO, ObjId, ObjKind};
use mush_testkit::{TestWorld, assert_actor_saw, assert_room_saw, last_room_view};

/// Two players standing in Limbo see each other's say and pose.
#[test]
fn two_players_see_say_and_pose() {
    let mut tw = TestWorld::new();
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");

    // `"` prefix shorthand for say.
    let events = tw.run(alice, "\"Hello there!");
    assert_room_saw(&events, LIMBO, "Hello there!");
    let bob_saw = tw.seen_by(&events, bob);
    assert!(
        bob_saw.contains(&"Alice says, \"Hello there!\"".to_string()),
        "Bob saw: {bob_saw:?}"
    );
    // The speaker's own room copy exists too (renderer owns You-vs-Name).
    assert!(!tw.seen_by(&events, alice).is_empty());

    // `:` prefix shorthand for pose.
    let events = tw.run(bob, ":grins broadly.");
    let alice_saw = tw.seen_by(&events, alice);
    assert!(
        alice_saw.contains(&"Bob grins broadly.".to_string()),
        "Alice saw: {alice_saw:?}"
    );
}

/// `@dig` a room with exits both ways, walk through, and `look` shows the
/// new room; walking back returns to Limbo.
#[test]
fn dig_walk_through_and_look() {
    let mut tw = TestWorld::new();
    let events = tw.run(GOD, "@dig Hall=north;n,south;s");
    assert_actor_saw(&events, GOD, "created with room number");
    assert_actor_saw(&events, GOD, "Opened exit north");
    assert_actor_saw(&events, GOD, "Opened exit south");

    // Walk through by typing the exit alias bare (dispatch exit-match step).
    let events = tw.run(GOD, "n");
    let rv = last_room_view(&events).expect("arrival renders the destination room");
    assert_eq!(rv.name, "Hall");
    assert!(
        rv.exits.iter().any(|e| e.name == "south"),
        "the back exit is visible from inside: {:?}",
        rv.exits
    );
    let hall: ObjId = rv.id.parse().expect("room id is a dbref");
    // Movement spam: the departure was told to Limbo, arrival to Hall.
    assert_room_saw(&events, LIMBO, "goes north.");
    assert_room_saw(&events, hall, "has arrived.");

    // An explicit look shows the same room.
    let events = tw.run(GOD, "look");
    assert_eq!(last_room_view(&events).unwrap().name, "Hall");

    // And the back exit really leads back.
    let events = tw.run(GOD, "south");
    assert_eq!(last_room_view(&events).unwrap().name, "Limbo");
}

/// A plain (non-BUILDER) player may not `@dig`.
#[test]
fn plain_player_cannot_dig() {
    let mut tw = TestWorld::new();
    let alice = tw.spawn_player("Alice");
    let events = tw.run(alice, "@dig Shack");
    assert_actor_saw(&events, alice, "Permission denied.");
}

/// `@desc` then `look` shows the description.
#[test]
fn desc_then_look_shows_desc() {
    let mut tw = TestWorld::new();
    let events = tw.run(GOD, "@desc here=A featureless expanse of white.");
    assert_actor_saw(&events, GOD, "Set.");

    let events = tw.run(GOD, "look");
    let rv = last_room_view(&events).unwrap();
    assert_eq!(rv.desc.to_plain_text(), "A featureless expanse of white.");

    // Another player looking sees it too.
    let alice = tw.spawn_player("Alice");
    let events = tw.run(alice, "look");
    assert_eq!(
        last_room_view(&events).unwrap().desc.to_plain_text(),
        "A featureless expanse of white."
    );
}

/// A locked exit blocks the other player with its custom fail message,
/// while the lock's owner passes through.
#[test]
fn locked_exit_blocks_with_fail_message() {
    let mut tw = TestWorld::new();
    let alice = tw.spawn_player("Alice");
    tw.run(GOD, "@dig Vault=north;n");
    let events = tw.run(GOD, "@lock north=me");
    assert_actor_saw(&events, GOD, "Locked.");
    tw.run(GOD, "@set north/fail=The vault door is barred.");

    let events = tw.run(alice, "north");
    assert_actor_saw(&events, alice, "The vault door is barred.");
    assert_eq!(
        tw.world().get(alice).unwrap().location,
        Some(LIMBO),
        "the blocked player did not move"
    );

    // The locking player walks straight through.
    let events = tw.run(GOD, "north");
    assert_eq!(last_room_view(&events).unwrap().name, "Vault");
}

/// A page reaches its target in another room — and nobody else.
#[test]
fn page_between_rooms() {
    let mut tw = TestWorld::new();
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    tw.run(GOD, "@dig Study=up;u");
    tw.run(GOD, "up");
    assert_ne!(tw.world().get(GOD).unwrap().location, Some(LIMBO));

    let events = tw.run(alice, "page God=Are you there?");
    assert_actor_saw(&events, GOD, "Alice pages: Are you there?");
    assert_actor_saw(&events, alice, "You paged God with 'Are you there?'.");
    assert!(
        tw.seen_by(&events, bob).is_empty(),
        "a bystander sees nothing of a page"
    );
}

/// Examining your own object shows the full technical view; examining
/// someone else's shows only the owner line.
#[test]
fn examine_own_vs_others_object() {
    let mut tw = TestWorld::new();
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    let locket = tw
        .world_mut()
        .create(ObjKind::Thing, "locket", alice, Some(alice))
        .unwrap();
    tw.world_mut()
        .set_attr(locket, "secret", Attr::text("engraved initials", alice))
        .unwrap();

    // Owner: full view with type, owner line, and attributes.
    let events = tw.run(alice, "examine locket");
    assert_actor_saw(&events, alice, "Type: Thing");
    assert_actor_saw(&events, alice, "engraved initials");

    // Someone else (by dbref; the locket is in Alice's pocket): public view.
    let events = tw.run(bob, &format!("examine {locket}"));
    assert_actor_saw(&events, bob, "is owned by Alice");
    assert!(
        !tw.seen_by(&events, bob)
            .iter()
            .any(|l| l.contains("engraved initials")),
        "attribute values are hidden from non-controllers"
    );
}

/// `help @dig` returns its usage text, aliases, and switches.
#[test]
fn help_at_dig_returns_usage() {
    let mut tw = TestWorld::new();
    let alice = tw.spawn_player("Alice");
    let events = tw.run(alice, "help @dig");
    assert_actor_saw(&events, alice, "@dig <name>");
    assert_actor_saw(&events, alice, "/teleport");
    assert_actor_saw(&events, alice, "Create a new room");
}

/// WHO lists every connected player.
#[test]
fn who_lists_connected_players() {
    let mut tw = TestWorld::new();
    let alice = tw.spawn_player("Alice");
    tw.spawn_player("Bob");
    let events = tw.run(alice, "WHO");
    assert_actor_saw(&events, alice, "Alice");
    assert_actor_saw(&events, alice, "Bob");
    assert_actor_saw(&events, alice, "God");
    assert_actor_saw(&events, alice, "3 players connected.");
}
