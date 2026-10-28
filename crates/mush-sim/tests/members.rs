//! **Doc 18 cut IC4 acceptance — the boundary rule** (doc 06 §7.5; doc 18
//! §4.2 (a)–(e); D-18-22; lead rulings L1–L7 of the IC4 brief).
//!
//! One interior-space record governs several walkable rooms: the occupant of a
//! multi-room space walks its member rooms' exits with no placement write,
//! projects to the member it stands in (so a reboot or a carrier move never
//! teleports a walker back to the lock), and is refused every other way off —
//! non-member exits, `home`, `@teleport` (staff included), `egress`. Members
//! are declared by `Transition::Members`, a pure plan with its refusals; a
//! member cannot be destroyed or made a space; boot drops invalid members in
//! RAM and fails closed into the space; a multi-room space backs no vault; the
//! crew aboard hear the carrier's leg boundaries and its arrival.
//!
//! Driven through the real `Engine` via `TestWorld` with only the
//! `SimulationModule` (and scenes, for the journey test) installed. mush-sim
//! persists through `Outbox::defer(Deferred::Persist)`; direct `plan`/`apply`
//! and `emit_world_event` calls use a throwaway `Outbox`, so the rig keeps one
//! ordered log of every op — the engine's and those throwaway outboxes' — and
//! folds that log for a reboot.

use std::collections::{BTreeSet, HashMap};

use mush_core::{
    Audience, BehaviorBinding, BehaviorId, Ctx, Deferred, Effects, GOD, LIMBO, ModuleActionState,
    ModuleBootData, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox, ParsedArgs, SwitchSet,
    WorldEvent,
};
use mush_economy::{CustodyProvider, CustodyRef};
use mush_proto::OutputEvent;
use mush_sim::{
    InteriorCustodyProvider, SimulationModule, SlotId, SpaceAnchor, SpatialError, SpatialParent,
    SpatialState, Transition, verify_indexes, verify_projections,
};
use mush_testkit::{authoritative::AcknowledgedWorld as TestWorld, text_of};
use smol_str::SmolStr;

#[path = "support/space_command.rs"]
mod space_command;

// ---- the rig -------------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(SimulationModule::new()),
        Box::new(mush_scenes::ScenesModule::new()),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

/// A command `who` types that must be accepted.
fn ok_as(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = space_command::run(tw, who, line);
    let t = texts(&events);
    assert!(!has_error(&events), "{line:?} was refused: {t}");
    t
}

/// A command `who` types that must be refused; the refusal's text.
fn refused_as(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = space_command::run(tw, who, line);
    let t = texts(&events);
    assert!(has_error(&events), "{line:?} was accepted: {t}");
    t
}

fn ok(tw: &mut TestWorld, line: &str) -> String {
    ok_as(tw, GOD, line)
}

fn refused(tw: &mut TestWorld, line: &str) -> String {
    refused_as(tw, GOD, line)
}

fn bind(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new(behavior).unwrap(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
}

fn loc(tw: &TestWorld, id: ObjId) -> Option<ObjId> {
    tw.world().get(id).and_then(|o| o.location)
}

fn state(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim state")
}

fn check_invariants(tw: &TestWorld) {
    let st = state(tw);
    verify_indexes(st).unwrap();
    let bad = verify_projections(st, tw.world());
    assert!(bad.is_empty(), "projection drift: {bad:?}");
}

fn set(ids: &[ObjId]) -> BTreeSet<ObjId> {
    ids.iter().copied().collect()
}

/// The fixture's objects, created in a fixed order so a second `TestWorld`
/// allocates the same ids (Limbo #0, GOD #1, then these).
#[derive(Clone, Copy)]
struct Fx {
    dock: ObjId,
    hull: ObjId,
    lock: ObjId,
    spine: ObjId,
    deck: ObjId,
    store: ObjId,
    outside: ObjId,
    /// The exit `Lock → Spine` (`spine`), an Exit object.
    lock_to_spine: ObjId,
}

/// Create and bind the objects (no records): a domain host `Dock`, a carrier
/// Thing `Hull`, rooms `Lock`, `Spine`, `Deck`, `Store`, `Outside`, and the
/// exits `Lock→Spine` (spine), `Spine→Lock` (lock), `Spine→Deck` (deck),
/// `Deck→Spine` (spine), `Spine→Outside` (out).
fn objects(tw: &mut TestWorld) -> Fx {
    let room = |tw: &mut TestWorld, name: &str| {
        tw.world_mut()
            .create(ObjKind::Room, name, GOD, None)
            .unwrap()
    };
    let dock = room(tw, "Dock");
    let hull = tw
        .world_mut()
        .create(ObjKind::Thing, "Hull", GOD, Some(LIMBO))
        .unwrap();
    let lock = room(tw, "Lock");
    let spine = room(tw, "Spine");
    let deck = room(tw, "Deck");
    let store = room(tw, "Store");
    let outside = room(tw, "Outside");
    let exit = |tw: &mut TestWorld, from: ObjId, name: &str, to: ObjId| {
        let e = tw
            .world_mut()
            .create(ObjKind::Exit, name, GOD, Some(from))
            .unwrap();
        tw.world_mut().link_exit(e, Some(to)).unwrap();
        e
    };
    let lock_to_spine = exit(tw, lock, "spine", spine);
    exit(tw, spine, "lock", lock);
    exit(tw, spine, "deck", deck);
    exit(tw, deck, "spine", spine);
    exit(tw, spine, "out", outside);
    bind(tw, dock, "sim.domain");
    bind(tw, hull, "sim.participant");
    bind(tw, lock, "sim.interior");
    bind(tw, store, "sim.interior");
    Fx {
        dock,
        hull,
        lock,
        spine,
        deck,
        store,
        outside,
        lock_to_spine,
    }
}

/// The records: domain #0 hosted on `Dock` presenting in `Outside`, `Hull`
/// placed in it, `Lock` and `Store` spaces riding `Hull`.
fn records(tw: &mut TestWorld, fx: &Fx) {
    ok(
        tw,
        &format!(
            "@domain/create #{}=sim.passive,#{}",
            fx.dock.0, fx.outside.0
        ),
    );
    ok(tw, &format!("@spatial/place/domain #{}=0", fx.hull.0));
    ok(
        tw,
        &format!("@spatial/space #{}=moving:#{}", fx.lock.0, fx.hull.0),
    );
    ok(
        tw,
        &format!("@spatial/space #{}=moving:#{}", fx.store.0, fx.hull.0),
    );
}

/// Move every op the engine queued into the log.
fn drain(tw: &mut TestWorld, log: &mut Vec<ModuleOp>) {
    log.extend(tw.take_module_ops());
}

/// Every `Deferred::Persist` op a throwaway outbox collected, into the log.
fn drain_out(tw: &mut TestWorld, out: &mut Outbox, log: &mut Vec<ModuleOp>) {
    let mut ops = Vec::new();
    for deferred in out.take_deferred() {
        if let Deferred::Persist(op) = deferred {
            ops.push(op);
        }
    }
    log.extend(ops.iter().cloned());
    tw.persist_legacy_ops(ops);
}

/// Plan and apply `t` directly against the slot (taken out and put back on
/// both paths), logging its persistence.
fn transition(
    tw: &mut TestWorld,
    log: &mut Vec<ModuleOp>,
    t: &Transition,
) -> Result<(), SpatialError> {
    drain(tw, log);
    let mut st = std::mem::take(
        tw.world_mut()
            .module_state_mut::<SpatialState>()
            .expect("sim state"),
    );
    let mut out = Outbox::new();
    let result = mush_sim::plan(&st, tw.world(), t)
        .and_then(|plan| mush_sim::apply(&mut st, tw.world_mut(), &mut out, plan));
    *tw.world_mut()
        .module_state_mut::<SpatialState>()
        .expect("sim state") = st;
    drain_out(tw, &mut out, log);
    result
}

/// `Transition::Members { space, rooms }` through the rig.
fn members(
    tw: &mut TestWorld,
    log: &mut Vec<ModuleOp>,
    space: ObjId,
    rooms: &[ObjId],
) -> Result<(), SpatialError> {
    transition(
        tw,
        log,
        &Transition::Members {
            space,
            rooms: set(rooms),
        },
    )
}

/// A programmatic destroy (bypassing `@destroy`'s gate) and the event the
/// command layer would have fired, logging the backstop's persistence.
fn destroy(tw: &mut TestWorld, log: &mut Vec<ModuleOp>, obj: ObjId) {
    drain(tw, log);
    tw.world_mut().destroy(obj).expect("destroy");
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj }, &mut out, &mut effects);
    drain_out(tw, &mut out, log);
}

/// Run `f` with a command context over the engine's world and registry, as
/// staff, logging what it persisted.
fn with_ctx<R>(
    tw: &mut TestWorld,
    log: &mut Vec<ModuleOp>,
    f: impl FnOnce(&mut Ctx<'_>) -> R,
) -> R {
    drain(tw, log);
    let mut out = Outbox::new();
    let engine = tw.engine_mut();
    let result = {
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new("sim")),
            typed_state: ModuleActionState::unavailable(),
        };
        f(&mut ctx)
    };
    drain_out(tw, &mut out, log);
    result
}

/// The log folded into a `sim` key/value set, in order (`placement.rs`'s fold).
fn fold_kv_ops(log: &[ModuleOp]) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in log {
        if op.ns != "sim" {
            continue;
        }
        match &op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| k != key);
                kv.push((key.clone(), value.clone()));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn doc<'a>(kv: &'a [(SmolStr, serde_json::Value)], key: &str) -> &'a serde_json::Value {
    &kv.iter()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("no {key} document"))
        .1
}

fn doc_mut<'a>(kv: &'a mut [(SmolStr, serde_json::Value)], key: &str) -> &'a mut serde_json::Value {
    &mut kv
        .iter_mut()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("no {key} document"))
        .1
}

/// The room ids a stored space document lists.
fn listed_rooms(value: &serde_json::Value) -> Vec<u64> {
    value["rooms"]
        .as_array()
        .expect("rooms")
        .iter()
        .map(|r| r.as_u64().expect("an id"))
        .collect()
}

/// A fresh world over the same objects, `setup` run before the boot data
/// (recreate walkers, stand them somewhere, edit the documents), then booted
/// from `kv`.
fn reboot_with(
    source: &TestWorld,
    mut kv: Vec<(SmolStr, serde_json::Value)>,
    setup: impl FnOnce(&mut TestWorld, &Fx, &mut Vec<(SmolStr, serde_json::Value)>),
) -> (TestWorld, Fx) {
    let mut tw = boot();
    tw.adopt_store(source);
    let fx = objects(&mut tw);
    setup(&mut tw, &fx, &mut kv);
    tw.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    (tw, fx)
}

/// The `placement:` keys among `ops`.
fn placement_keys(ops: &[ModuleOp]) -> Vec<String> {
    keys_with(ops, "placement:")
}

fn keys_with(ops: &[ModuleOp], prefix: &str) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => {
                key.starts_with(prefix).then(|| key.to_string())
            }
            ModuleWrite::StreamAppend { .. } => None,
        })
        .collect()
}

/// A player made a participant (the module's behaviour bound directly).
fn walker(tw: &mut TestWorld, name: &str) -> ObjId {
    let who = tw.spawn_player(name);
    bind(tw, who, "sim.participant");
    who
}

/// The whole standard scene: objects, records, `Lock`'s members `{Lock,
/// Spine, Deck}`.
fn scene() -> (TestWorld, Fx, Vec<ModuleOp>) {
    let mut tw = boot();
    let fx = objects(&mut tw);
    records(&mut tw, &fx);
    let mut log = Vec::new();
    members(&mut tw, &mut log, fx.lock, &[fx.lock, fx.spine, fx.deck]).expect("members");
    (tw, fx, log)
}

fn aboard_sentence(who: &str) -> String {
    format!(
        "{who} is aboard Hull; it moves only between Hull's rooms, or leaves through the \
         simulation or staff recovery (@spatial)."
    )
}

// ---- 1. the plan's refusals ----------------------------------------------

/// **`Transition::Members` refuses what the boundary rule forbids** (doc 18
/// §4.2 (c)), each in turn, and mutates nothing.
#[test]
fn ic4_members_plan_refuses_what_the_boundary_rule_forbids() {
    let mut tw = boot();
    let fx = objects(&mut tw);
    records(&mut tw, &fx);
    let mut log = Vec::new();
    drain(&mut tw, &mut log);
    let before = state(&tw).spaces.clone();
    let refuse = |tw: &mut TestWorld, log: &mut Vec<ModuleOp>, space, rooms: &[ObjId]| {
        let mark = log.len();
        let err = members(tw, log, space, rooms).expect_err("refused");
        assert_eq!(log.len(), mark, "a refusal persists nothing ({err})");
        err
    };
    // The space is not among its rooms.
    let e = refuse(&mut tw, &mut log, fx.lock, &[fx.spine, fx.deck]);
    assert!(
        matches!(e, SpatialError::Members(ref s) if s.contains("must be a room of its own")),
        "{e}"
    );
    // Seventeen rooms.
    let mut many = vec![fx.lock];
    many.extend((100..116).map(ObjId));
    assert_eq!(many.len(), 17);
    let e = refuse(&mut tw, &mut log, fx.lock, &many);
    assert_eq!(
        e,
        SpatialError::Members("an interior space holds at most 16 rooms; 17 were named".into())
    );
    // A non-room (an Exit).
    let e = refuse(&mut tw, &mut log, fx.lock, &[fx.lock, fx.lock_to_spine]);
    assert_eq!(
        e,
        SpatialError::Members(format!("{} is not a room", fx.lock_to_spine))
    );
    // A space of its own.
    let e = refuse(&mut tw, &mut log, fx.lock, &[fx.lock, fx.store]);
    assert_eq!(
        e,
        SpatialError::Members(format!("{} is an interior space of its own", fx.store))
    );
    // A domain host.
    let e = refuse(&mut tw, &mut log, fx.lock, &[fx.lock, fx.dock]);
    assert_eq!(
        e,
        SpatialError::Members(format!("{} hosts domain #0", fx.dock))
    );
    // The domain's presentation room.
    let e = refuse(&mut tw, &mut log, fx.lock, &[fx.lock, fx.outside]);
    assert_eq!(
        e,
        SpatialError::Members(format!(
            "{} is the presentation room of domain #0",
            fx.outside
        ))
    );
    // Not a space at all.
    let e = refuse(&mut tw, &mut log, fx.spine, &[fx.spine]);
    assert_eq!(e, SpatialError::NoSuchSpace(fx.spine));
    assert_eq!(state(&tw).spaces, before, "nothing mutated");

    // A room already a member of another space.
    members(&mut tw, &mut log, fx.store, &[fx.store, fx.deck]).expect("store takes deck");
    let e = refuse(&mut tw, &mut log, fx.lock, &[fx.lock, fx.deck]);
    assert_eq!(
        e,
        SpatialError::Members(format!(
            "{} is already a room of interior space {}",
            fx.deck, fx.store
        ))
    );
    members(&mut tw, &mut log, fx.store, &[fx.store]).expect("store lets deck go");

    // Removing a member an occupant stands in.
    members(&mut tw, &mut log, fx.lock, &[fx.lock, fx.spine, fx.deck]).expect("members");
    let ann = walker(&mut tw, "Ann");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", ann.0, fx.lock.0),
    );
    ok_as(&mut tw, ann, "spine");
    ok_as(&mut tw, ann, "deck");
    assert_eq!(loc(&tw, ann), Some(fx.deck));
    drain(&mut tw, &mut log);
    let held = state(&tw).spaces.clone();
    let e = refuse(&mut tw, &mut log, fx.lock, &[fx.lock, fx.spine]);
    assert_eq!(
        e,
        SpatialError::Members(format!("{} still holds Ann; move them first", fx.deck))
    );
    assert_eq!(state(&tw).spaces, held, "nothing mutated");
    check_invariants(&tw);
}

// ---- 2. persistence and the index ------------------------------------------

/// **The members persist, are indexed and survive a reboot.**
#[test]
fn ic4_members_persist_index_and_survive_a_reboot() {
    let (mut tw, fx, mut log) = scene();
    drain(&mut tw, &mut log);
    let kv = tw.persisted_kv("sim");
    let key = format!("space:{}", fx.lock.0);
    assert_eq!(
        listed_rooms(doc(&kv, &key)),
        vec![
            u64::from(fx.lock.0),
            u64::from(fx.spine.0),
            u64::from(fx.deck.0)
        ]
    );
    let st = state(&tw);
    for room in [fx.lock, fx.spine, fx.deck] {
        assert_eq!(st.derived.member_of.get(&room), Some(&fx.lock), "{room}");
    }
    assert_eq!(st.derived.member_of.get(&fx.store), Some(&fx.store));
    verify_indexes(st).unwrap();
    let record = st.spaces[&fx.lock].clone();
    let index = st.derived.member_of.clone();
    check_invariants(&tw);

    let (tw2, fx2) = reboot_with(&tw, kv, |_, _, _| {});
    let st2 = state(&tw2);
    assert_eq!(st2.spaces[&fx2.lock], record);
    assert_eq!(st2.derived.member_of, index);
    check_invariants(&tw2);
}

// ---- 3. walking ------------------------------------------------------------

/// **An occupant walks member exits with no placement write** (doc 18 §4.2
/// (a)): the placement record's revision never moves.
#[test]
fn ic4_an_occupant_walks_member_exits_with_no_placement_write() {
    let (mut tw, fx, _) = scene();
    let ann = walker(&mut tw, "Ann");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", ann.0, fx.lock.0),
    );
    assert_eq!(
        loc(&tw, ann),
        Some(fx.lock),
        "an entrant arrives in the lock"
    );
    let _ = tw.take_module_ops();
    let revision = state(&tw).placements[&ann].clone();
    for (word, room) in [
        ("spine", fx.spine),
        ("deck", fx.deck),
        ("spine", fx.spine),
        ("lock", fx.lock),
    ] {
        ok_as(&mut tw, ann, word);
        assert_eq!(loc(&tw, ann), Some(room), "after {word}");
        let ops = tw.take_module_ops();
        assert!(placement_keys(&ops).is_empty(), "{word} wrote a placement");
        assert_eq!(state(&tw).placements[&ann], revision);
        check_invariants(&tw);
    }
}

// ---- 4. every other way off ------------------------------------------------

/// **A non-member destination, `home` and `@teleport` are refused** — staff's
/// `@teleport` too (design §7.5: staff are not exempt from the gate).
#[test]
fn ic4_non_member_destinations_home_and_teleport_are_refused() {
    let (mut tw, fx, _) = scene();
    let ann = walker(&mut tw, "Ann");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", ann.0, fx.lock.0),
    );
    ok_as(&mut tw, ann, "spine");
    let t = refused_as(&mut tw, ann, "out");
    assert!(t.contains(&aboard_sentence("Ann")), "{t}");
    assert_eq!(loc(&tw, ann), Some(fx.spine));
    let t = refused_as(&mut tw, ann, "home");
    assert!(t.contains(&aboard_sentence("Ann")), "{t}");
    assert_eq!(loc(&tw, ann), Some(fx.spine));
    // The player's own `@teleport` must clear `perm::can_teleport` (Outside
    // made JUMP_OK) so the refusal is the gate's, not the permission check's.
    tw.world_mut()
        .set_flag(fx.outside, mush_core::Flags::JUMP_OK, true)
        .unwrap();
    let t = refused_as(&mut tw, ann, &format!("@teleport me=#{}", fx.outside.0));
    assert!(t.contains(&aboard_sentence("Ann")), "{t}");
    assert_eq!(loc(&tw, ann), Some(fx.spine));
    let t = refused(&mut tw, &format!("@teleport #{}=#{}", ann.0, fx.outside.0));
    assert!(t.contains(&aboard_sentence("Ann")), "{t}");
    assert_eq!(loc(&tw, ann), Some(fx.spine));

    // The member arm needs an EXIT (L3): an exit-less move INTO a member room
    // is refused too — GOD's `@teleport` to the Deck, and `home` when home is
    // the Lock.
    let t = refused(&mut tw, &format!("@teleport #{}=#{}", ann.0, fx.deck.0));
    assert!(t.contains(&aboard_sentence("Ann")), "{t}");
    assert_eq!(loc(&tw, ann), Some(fx.spine));
    tw.world_mut().set_home(ann, Some(fx.lock)).unwrap();
    let t = refused_as(&mut tw, ann, "home");
    assert!(t.contains(&aboard_sentence("Ann")), "{t}");
    assert_eq!(loc(&tw, ann), Some(fx.spine));

    // The member arm reads the actor's OWN space: an occupant of the
    // single-room `Store` cannot walk an exit into `Lock`'s member `Spine`.
    let to_spine = tw
        .world_mut()
        .create(ObjKind::Exit, "spine", GOD, Some(fx.store))
        .unwrap();
    tw.world_mut().link_exit(to_spine, Some(fx.spine)).unwrap();
    let bea = walker(&mut tw, "Bea");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", bea.0, fx.store.0),
    );
    assert_eq!(loc(&tw, bea), Some(fx.store));
    let t = refused_as(&mut tw, bea, "spine");
    assert!(
        t.contains("Bea is inside Store; it leaves only through the simulation"),
        "{t}"
    );
    assert_eq!(loc(&tw, bea), Some(fx.store));
    check_invariants(&tw);
}

// ---- 5. projection across a reboot -----------------------------------------

/// **Projection keeps a walker in its member room across a reboot** (doc 18
/// §4.2 (b)); a walker standing outside the members at boot lands in the lock.
#[test]
fn ic4_projection_keeps_a_walker_in_its_member_room_across_a_reboot() {
    let (mut tw, fx, mut log) = scene();
    let ann = walker(&mut tw, "Ann");
    let bea = walker(&mut tw, "Bea");
    for who in [ann, bea] {
        ok(
            &mut tw,
            &format!("@spatial/place/interior #{}=#{}", who.0, fx.lock.0),
        );
    }
    ok_as(&mut tw, ann, "spine");
    ok_as(&mut tw, ann, "deck");
    assert_eq!(loc(&tw, ann), Some(fx.deck));
    let placement = state(&tw).placements[&ann].clone();
    drain(&mut tw, &mut log);
    let kv = tw.persisted_kv("sim");

    let (mut tw2, _) = reboot_with(&tw, kv, |tw, fx, _| {
        let ann = walker(tw, "Ann");
        let bea = walker(tw, "Bea");
        tw.world_mut().move_to(ann, fx.deck).unwrap();
        // Bea stands in Limbo: not a member.
        assert_eq!(loc(tw, bea), Some(LIMBO));
    });
    assert_eq!(
        loc(&tw2, ann),
        Some(fx.deck),
        "the walker stays where it walked"
    );
    assert_eq!(state(&tw2).placements[&ann], placement);
    assert_eq!(
        loc(&tw2, bea),
        Some(fx.lock),
        "a non-member location falls back"
    );
    assert!(placement_keys(&tw2.take_module_ops()).is_empty());
    check_invariants(&tw2);
}

// ---- 6. boot drops invalid members -----------------------------------------

/// **Boot drops invalid members in RAM, writes nothing, and fails closed into
/// the space** (doc 18 §4.2 (c); L5).
#[test]
fn ic4_boot_drops_invalid_members_and_fails_closed_into_the_space() {
    let (mut tw, fx, mut log) = scene();
    let ann = walker(&mut tw, "Ann");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", ann.0, fx.lock.0),
    );
    drain(&mut tw, &mut log);
    let kv = tw.persisted_kv("sim");
    let key = format!("space:{}", fx.lock.0);

    let (mut tw2, _) = reboot_with(&tw, kv, |tw, fx, kv| {
        let ann = walker(tw, "Ann");
        let gone = tw
            .world_mut()
            .create(ObjKind::Room, "Gone", GOD, None)
            .unwrap();
        tw.world_mut().destroy(gone).unwrap();
        // The document lists another space's object (Store), a destroyed room
        // and Deck; the walker stands in the dropped Store.
        doc_mut(kv, &key)["rooms"] = serde_json::json!([fx.lock.0, fx.deck.0, fx.store.0, gone.0]);
        tw.world_mut().move_to(ann, fx.store).unwrap();
    });
    assert_eq!(state(&tw2).spaces[&fx.lock].rooms, set(&[fx.lock, fx.deck]));
    assert!(
        keys_with(&tw2.take_module_ops(), "space:").is_empty(),
        "the stored document is never rewritten"
    );
    assert_eq!(loc(&tw2, ann), Some(fx.lock), "failed closed into the lock");
    check_invariants(&tw2);
}

/// **A Thing pod — the atlas's interior item — boots untouched** (F1): its
/// record keeps `rooms {pod}`, its occupant stays, nothing is written.
#[test]
fn ic4_a_thing_pod_boots_untouched() {
    let mut tw = boot();
    let _fx = objects(&mut tw);
    let mut log = Vec::new();
    let pod = tw
        .world_mut()
        .create(ObjKind::Thing, "Pod", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, pod, "sim.interior");
    transition(
        &mut tw,
        &mut log,
        &Transition::Space {
            space: pod,
            anchor: SpaceAnchor::Unanchored,
        },
    )
    .expect("the pod is a space");
    let bea = walker(&mut tw, "Bea");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", bea.0, pod.0),
    );
    assert_eq!(loc(&tw, bea), Some(pod));
    drain(&mut tw, &mut log);
    let kv = tw.persisted_kv("sim");

    let (mut tw2, _) = reboot_with(&tw, kv, |tw, _, _| {
        let pod2 = tw
            .world_mut()
            .create(ObjKind::Thing, "Pod", GOD, Some(LIMBO))
            .unwrap();
        assert_eq!(pod2, pod);
        bind(tw, pod2, "sim.interior");
        let bea = walker(tw, "Bea");
        tw.world_mut().move_to(bea, pod2).unwrap();
    });
    let st = state(&tw2);
    assert_eq!(st.spaces[&pod].rooms, set(&[pod]));
    assert_eq!(st.derived.member_of.get(&pod), Some(&pod));
    assert_eq!(loc(&tw2, bea), Some(pod));
    let ops = tw2.take_module_ops();
    assert!(keys_with(&ops, "space:").is_empty(), "{ops:?}");
    assert!(placement_keys(&ops).is_empty(), "{ops:?}");
    check_invariants(&tw2);
}

// ---- 7. destroy and space gates --------------------------------------------

/// **A member cannot be destroyed or made a space**; a programmatic destroy
/// drops it from the space and fails its walker closed into the lock.
#[test]
fn ic4_a_member_cannot_be_destroyed_or_made_a_space() {
    let (mut tw, fx, mut log) = scene();
    let t = refused(&mut tw, &format!("@destroy #{}", fx.deck.0));
    assert!(
        t.contains("Deck is a room of the interior space Lock; @spatial/unspace that space first."),
        "{t}"
    );
    assert!(tw.world().get(fx.deck).is_some());
    ok(
        &mut tw,
        &format!("@behavior/add #{}=sim.interior", fx.deck.0),
    );
    let t = refused(
        &mut tw,
        &format!("@spatial/space #{}=moving:#{}", fx.deck.0, fx.hull.0),
    );
    assert!(
        t.contains(&format!(
            "{} is a room of interior space {}; it cannot be a space itself",
            fx.deck, fx.lock
        )),
        "{t}"
    );
    assert!(!state(&tw).spaces.contains_key(&fx.deck));

    let ann = walker(&mut tw, "Ann");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", ann.0, fx.lock.0),
    );
    ok_as(&mut tw, ann, "spine");
    ok_as(&mut tw, ann, "deck");
    destroy(&mut tw, &mut log, fx.deck);
    assert_eq!(state(&tw).spaces[&fx.lock].rooms, set(&[fx.lock, fx.spine]));
    assert_eq!(loc(&tw, ann), Some(fx.lock), "not at home, not in Limbo");
    drain(&mut tw, &mut log);
    let kv = tw.persisted_kv("sim");
    assert_eq!(
        listed_rooms(doc(&kv, &format!("space:{}", fx.lock.0))),
        vec![u64::from(fx.lock.0), u64::from(fx.spine.0)]
    );
    check_invariants(&tw);
}

// ---- 8. custody and egress -------------------------------------------------

/// **A multi-room space backs no vault and refuses egress** (doc 18 §4.2 (d));
/// a single-room space's occupant still egresses.
#[test]
fn ic4_a_multi_room_space_backs_no_vault_and_refuses_egress() {
    let (mut tw, fx, _) = scene();
    let provider = InteriorCustodyProvider;
    assert_eq!(
        provider.backing_valid(tw.world(), &CustodyRef::SpatialInterior(fx.lock)),
        Some(false)
    );
    assert_eq!(
        provider.backing_valid(tw.world(), &CustodyRef::SpatialInterior(fx.store)),
        Some(true)
    );
    let ann = walker(&mut tw, "Ann");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", ann.0, fx.lock.0),
    );
    for word in ["egress", "disembark"] {
        let t = refused_as(&mut tw, ann, word);
        assert!(
            t.contains("You cannot egress from Hull; use +ship/leave in the Lock."),
            "{t}"
        );
        assert!(
            matches!(state(&tw).parent_of(ann), SpatialParent::Interior { space, .. } if space == fx.lock)
        );
    }
    let bea = walker(&mut tw, "Bea");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", bea.0, fx.store.0),
    );
    ok_as(&mut tw, bea, "egress");
    assert!(matches!(
        state(&tw).parent_of(bea),
        SpatialParent::Direct { .. }
    ));
    check_invariants(&tw);
}

// ---- 9. the compartment write ----------------------------------------------

/// **`declare_compartment` is `@spatial/slot`'s write** (L1): the same
/// document; the cap admits two and refuses a third.
#[test]
fn ic4_declare_compartment_is_the_slot_write() {
    let (mut tw, fx, mut log) = scene();
    let key = format!("space:{}", fx.lock.0);
    drain(&mut tw, &mut log);
    ok(&mut tw, &format!("@spatial/slot #{}=berths,2", fx.lock.0));
    let by_verb = fold_kv_ops(&tw.take_module_ops());
    let mut direct_log = Vec::new();
    let slot = SlotId::new("berths").expect("a slot id");
    with_ctx(&mut tw, &mut direct_log, |ctx| {
        mush_sim::declare_compartment(ctx, fx.lock, slot, Some(2))
    })
    .expect("declared");
    let by_fn = fold_kv_ops(&direct_log);
    assert_eq!(doc(&by_verb, &key), doc(&by_fn, &key));
    assert_eq!(
        doc(&by_fn, &key)["compartments"]["berths"]["maximum_items"],
        2
    );
    let not_a_space = with_ctx(&mut tw, &mut direct_log, |ctx| {
        mush_sim::declare_compartment(ctx, fx.spine, SlotId::new("berths").unwrap(), Some(2))
    });
    assert_eq!(
        not_a_space,
        Err("That is not an interior space.".to_string())
    );

    let crew: Vec<ObjId> = ["Ann", "Bea", "Cid"]
        .iter()
        .map(|name| walker(&mut tw, name))
        .collect();
    for who in &crew[..2] {
        ok(
            &mut tw,
            &format!("@spatial/place/interior #{}=#{},berths", who.0, fx.lock.0),
        );
    }
    let t = refused(
        &mut tw,
        &format!(
            "@spatial/place/interior #{}=#{},berths",
            crew[2].0, fx.lock.0
        ),
    );
    assert!(t.contains("is full (2 max)"), "{t}");
    check_invariants(&tw);
}

// ---- 10. narration aboard --------------------------------------------------

/// **The rooms aboard hear the leg boundary and the arrival** (doc 18 §4.2
/// (e)); the arrival's carrier move leaves the walker where it walked.
#[test]
fn ic4_aboard_rooms_hear_the_leg_boundary_and_the_arrival() {
    let mut tw = boot();
    tw.set_now(1_000);
    let fx = objects(&mut tw);
    records(&mut tw, &fx);
    let mut log = Vec::new();
    members(&mut tw, &mut log, fx.lock, &[fx.lock, fx.spine, fx.deck]).expect("members");
    let ann = walker(&mut tw, "Ann");
    let far = tw
        .world_mut()
        .create(ObjKind::Room, "Far", GOD, None)
        .unwrap();
    bind(&mut tw, far, "sim.domain");
    ok(&mut tw, &format!("@domain/create #{}=sim.passive", far.0));
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", ann.0, fx.lock.0),
    );
    ok_as(&mut tw, ann, "spine");
    ok_as(&mut tw, ann, "deck");
    ok(
        &mut tw,
        &format!(
            "@journey/depart #{}=corridor #{}:100 #{}:200",
            fx.hull.0, fx.dock.0, far.0
        ),
    );
    tw.set_now(1_150);
    let events = tw.fire_due(1_150 * 1000);
    let heard = tw.seen_by(&events, ann).join("\n");
    assert!(heard.contains("Hull passes Dock. Leg 2 of 2."), "{heard}");
    assert_eq!(loc(&tw, ann), Some(fx.deck));
    tw.set_now(1_301);
    let events = tw.fire_due(1_301 * 1000);
    let heard = tw.seen_by(&events, ann).join("\n");
    assert!(
        heard.contains("Hull arrives, and the passage ends."),
        "{heard}"
    );
    assert_eq!(
        loc(&tw, ann),
        Some(fx.deck),
        "the arrival leaves the walker in Deck"
    );
    assert!(matches!(
        state(&tw).parent_of(fx.hull),
        SpatialParent::Direct { domain, .. } if domain.0 == 1
    ));
    check_invariants(&tw);
}

// ---- doc 18 cut IC14 row 46: the engine's rooms and vault backings ----------

/// The rig at the refusal test's start: the fixture, its records, an empty log.
fn refusal_rig() -> (TestWorld, Fx, Vec<ModuleOp>) {
    let mut tw = boot();
    let fx = objects(&mut tw);
    records(&mut tw, &fx);
    let mut log = Vec::new();
    drain(&mut tw, &mut log);
    (tw, fx, log)
}

/// `Transition::Members` refused with nothing persisted and the records unchanged.
fn refused_members(
    tw: &mut TestWorld,
    log: &mut Vec<ModuleOp>,
    space: ObjId,
    rooms: &[ObjId],
) -> SpatialError {
    let before = state(tw).spaces.clone();
    let mark = log.len();
    let err = members(tw, log, space, rooms).expect_err("refused");
    assert_eq!(log.len(), mark, "a refusal persists nothing ({err})");
    assert_eq!(state(tw).spaces, before, "a refusal mutates nothing");
    err
}

/// **Doc 18 cut IC14 row 46: Limbo is never a member room.**
#[test]
fn ic14_members_refuse_limbo() {
    let (mut tw, fx, mut log) = refusal_rig();
    let e = refused_members(&mut tw, &mut log, fx.lock, &[fx.lock, LIMBO]);
    assert_eq!(e, SpatialError::Members(format!("{LIMBO} is Limbo")));
    check_invariants(&tw);
}

/// **Doc 18 cut IC14 row 46: the recovery room is never a member room.**
#[test]
fn ic14_members_refuse_the_recovery_room() {
    let (mut tw, fx, mut log) = refusal_rig();
    tw.world_mut()
        .module_state_mut::<SpatialState>()
        .expect("sim state")
        .recovery_room = fx.deck;
    let e = refused_members(&mut tw, &mut log, fx.lock, &[fx.lock, fx.spine, fx.deck]);
    assert_eq!(
        e,
        SpatialError::Members(format!("{} is the recovery room", fx.deck))
    );
}

/// **Doc 18 cut IC14 row 46: the in-transit room is never a member room.**
#[test]
fn ic14_members_refuse_the_in_transit_room() {
    let (mut tw, fx, mut log) = refusal_rig();
    tw.world_mut()
        .module_state_mut::<SpatialState>()
        .expect("sim state")
        .in_transit_room = fx.deck;
    let e = refused_members(&mut tw, &mut log, fx.lock, &[fx.lock, fx.spine, fx.deck]);
    assert_eq!(
        e,
        SpatialError::Members(format!("{} is the in-transit room", fx.deck))
    );
}

/// **Doc 18 cut IC14 row 46: a room that backs a custody vault is never a member
/// room** (a member is walkable; doc 18 §9 refuses a walkable room as a vault
/// backing). On the custody rig's composition (org, sim, economy with the
/// interior custody provider), a container vault on Deck refuses Deck as a member
/// with the vault's number; without the vault the same set is accepted.
#[test]
fn ic14_members_refuse_a_vault_backing_room() {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![
            std::sync::Arc::new(InteriorCustodyProvider),
        ])),
    ]);
    let fx = objects(&mut tw);
    records(&mut tw, &fx);
    let mut log = Vec::new();
    drain(&mut tw, &mut log);
    ok(
        &mut tw,
        &format!("@vault/create strongroom=container:#{}", fx.deck.0),
    );
    let vault = tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|store| {
            store
                .snapshot()
                .vault_by_backing(&CustodyRef::Container(fx.deck))
                .map(|v| v.id.0)
        })
        .expect("the vault backs Deck");
    drain(&mut tw, &mut log);
    let e = refused_members(&mut tw, &mut log, fx.lock, &[fx.lock, fx.spine, fx.deck]);
    assert_eq!(
        e,
        SpatialError::Members(format!("{} backs vault #{vault}", fx.deck))
    );
    // Without Deck the set is the boundary rule's, and accepted.
    members(&mut tw, &mut log, fx.lock, &[fx.lock, fx.spine]).expect("accepted");
    check_invariants(&tw);
}
