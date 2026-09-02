//! Phase A: object behavior bindings and the pre-destroy veto (design 06 §4, D8).
//! Drives the public API — a real `World`, the real `Registry` with the core
//! commands, and the dispatch pipeline — plus direct registration of test
//! behaviors and a test [`DestroyGate`].

use std::sync::Arc;

use mush_core::{
    ArgShape, BehaviorBinding, BehaviorCommand, BehaviorId, BehaviorSpec, BehaviorTarget, CmdError,
    CommandReach, CommandSpec, CommandSubjectLink, Ctx, DestroyGate, DestroyPolicy, Flags, GOD,
    LIMBO, ModError, ObjId, ObjKind, Outbox, Registrar, Registry, Role, World, dispatch,
    register_core_commands,
};
use mush_proto::OutputEvent;

/// Refuses destroying anything named "Fortress" (a dynamic structural veto).
struct RefuseFortress;
impl DestroyGate for RefuseFortress {
    fn check(&self, w: &World, _reg: &Registry, _actor: ObjId, target: ObjId) -> Result<(), String> {
        if w.get(target).is_some_and(|o| o.name.eq_ignore_ascii_case("Fortress")) {
            Err("The fortress cannot be destroyed while under siege.".into())
        } else {
            Ok(())
        }
    }
}

struct Game {
    w: World,
    reg: Registry,
    bob: ObjId,
    ann: ObjId,
}

impl Game {
    fn new() -> Game {
        let mut w = World::new_minimal();
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();
        let ann = w.create(ObjKind::Player, "Ann", GOD, Some(LIMBO)).unwrap();
        w.set_flag(bob, Flags::BUILDER, true).unwrap();
        w.set_flag(ann, Flags::BUILDER, true).unwrap();
        let mut reg = Registry::new();
        register_core_commands(&mut reg).unwrap();
        register_test_behaviors(&mut reg);
        Game { w, reg, bob, ann }
    }

    fn run(&mut self, actor: ObjId, line: &str) -> Outbox {
        let mut out = Outbox::new();
        dispatch(&self.reg, &mut self.w, actor, line, &mut out);
        out
    }

    /// Create a thing owned by `owner` in Limbo, returning its id.
    fn thing(&mut self, name: &str, owner: ObjId) -> ObjId {
        self.w.create(ObjKind::Thing, name, owner, Some(LIMBO)).unwrap()
    }

    fn has(&self, id: ObjId, behavior: &str) -> bool {
        self.w.has_behavior(id, &BehaviorId::new(behavior).unwrap())
    }

    fn going(&self, id: ObjId) -> bool {
        self.w.get(id).is_some_and(|o| o.flags.contains(Flags::GOING))
    }
}

fn bid(s: &str) -> BehaviorId {
    BehaviorId::new(s).unwrap()
}

fn register_test_behaviors(reg: &mut Registry) {
    let mut widget = BehaviorSpec::new(bid("test.widget"));
    widget.owner = "test".into();
    reg.register_behavior(widget).unwrap();

    let mut fragile = BehaviorSpec::new(bid("test.fragile"));
    fragile.owner = "test".into();
    fragile.destroy = DestroyPolicy::Refuse;
    reg.register_behavior(fragile).unwrap();

    let mut roomonly = BehaviorSpec::new(bid("test.roomonly"));
    roomonly.owner = "test".into();
    roomonly.allowed_kinds = vec![ObjKind::Room];
    reg.register_behavior(roomonly).unwrap();

    // alpha forbids beta; beta declares nothing (so we exercise both the direct
    // and the symmetric conflict checks).
    let mut alpha = BehaviorSpec::new(bid("test.alpha"));
    alpha.owner = "test".into();
    alpha.conflicts = vec![bid("test.beta")];
    reg.register_behavior(alpha).unwrap();
    let mut beta = BehaviorSpec::new(bid("test.beta"));
    beta.owner = "test".into();
    reg.register_behavior(beta).unwrap();

    let mut staff = BehaviorSpec::new(bid("test.staff"));
    staff.owner = "test".into();
    staff.min_role = Role::Wizard;
    reg.register_behavior(staff).unwrap();
}

fn texts(out: &Outbox) -> String {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            OutputEvent::Narrative { text, .. } => Some(text.to_plain_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------
// Bindings
// ---------------------------------------------------------------------

#[test]
fn attach_list_remove_round_trip() {
    let mut g = Game::new();
    let w = g.thing("Widget", g.bob);

    let out = g.run(g.bob, "@behavior/add Widget=test.widget");
    assert!(texts(&out).contains("Attached"), "{}", texts(&out));
    assert!(g.has(w, "test.widget"));

    let out = g.run(g.bob, "@behavior/list Widget");
    assert!(texts(&out).contains("test.widget"), "{}", texts(&out));

    let out = g.run(g.bob, "@behavior/remove Widget=test.widget");
    assert!(texts(&out).contains("Removed"), "{}", texts(&out));
    assert!(!g.has(w, "test.widget"));
}

#[test]
fn same_behavior_on_two_objects_unbound_object_has_none() {
    let mut g = Game::new();
    let a = g.thing("Anvil", g.bob);
    let b = g.thing("Barrel", g.bob);
    let c = g.thing("Crate", g.bob);
    g.run(g.bob, "@behavior/add Anvil=test.widget");
    g.run(g.bob, "@behavior/add Barrel=test.widget");
    assert!(g.has(a, "test.widget"));
    assert!(g.has(b, "test.widget"));
    assert!(!g.has(c, "test.widget"), "unbound object exposes no behavior");
}

#[test]
fn unknown_behavior_is_refused() {
    let mut g = Game::new();
    g.thing("Widget", g.bob);
    let out = g.run(g.bob, "@behavior/add Widget=test.nope");
    assert!(texts(&out).contains("No such behavior"), "{}", texts(&out));
}

#[test]
fn malformed_id_is_refused() {
    let mut g = Game::new();
    g.thing("Widget", g.bob);
    let out = g.run(g.bob, "@behavior/add Widget=NotDotted");
    assert!(texts(&out).to_lowercase().contains("dotted"), "{}", texts(&out));
}

#[test]
fn kind_restriction_is_enforced() {
    let mut g = Game::new();
    let w = g.thing("Widget", g.bob);
    let out = g.run(g.bob, "@behavior/add Widget=test.roomonly");
    assert!(texts(&out).contains("cannot attach"), "{}", texts(&out));
    assert!(!g.has(w, "test.roomonly"));
}

#[test]
fn conflicts_block_in_both_orders() {
    let mut g = Game::new();
    let a = g.thing("Anvil", g.bob);
    let b = g.thing("Barrel", g.bob);

    // Direct: alpha declares beta a conflict.
    g.run(g.bob, "@behavior/add Anvil=test.beta");
    let out = g.run(g.bob, "@behavior/add Anvil=test.alpha");
    assert!(texts(&out).contains("conflicts"), "direct: {}", texts(&out));
    assert!(!g.has(a, "test.alpha"));

    // Symmetric: beta declares nothing, but alpha (already held) forbids it.
    g.run(g.bob, "@behavior/add Barrel=test.alpha");
    let out = g.run(g.bob, "@behavior/add Barrel=test.beta");
    assert!(texts(&out).contains("conflicts"), "symmetric: {}", texts(&out));
    assert!(!g.has(b, "test.beta"));
}

#[test]
fn double_attach_is_refused() {
    let mut g = Game::new();
    g.thing("Widget", g.bob);
    g.run(g.bob, "@behavior/add Widget=test.widget");
    let out = g.run(g.bob, "@behavior/add Widget=test.widget");
    assert!(texts(&out).contains("already"), "{}", texts(&out));
}

#[test]
fn behavior_min_role_gates_attachment() {
    let mut g = Game::new();
    let w = g.thing("Widget", g.bob);
    // Bob is a Builder, not a Wizard; test.staff needs Wizard.
    let out = g.run(g.bob, "@behavior/add Widget=test.staff");
    assert!(texts(&out).contains("Permission denied"), "{}", texts(&out));
    assert!(!g.has(w, "test.staff"));
    // God (wizard) may.
    let out = g.run(GOD, "@behavior/add Widget=test.staff");
    assert!(texts(&out).contains("Attached"), "{}", texts(&out));
    assert!(g.has(w, "test.staff"));
}

#[test]
fn controls_is_required_to_attach() {
    let mut g = Game::new();
    // A thing owned by Ann; Bob (builder) does not control it.
    let w = g.thing("Widget", g.ann);
    let out = g.run(g.bob, "@behavior/add Widget=test.widget");
    assert!(texts(&out).contains("Permission denied"), "{}", texts(&out));
    assert!(!g.has(w, "test.widget"));
}

// ---------------------------------------------------------------------
// Destroy veto (D8)
// ---------------------------------------------------------------------

#[test]
fn refuse_policy_vetoes_destroy_until_removed() {
    let mut g = Game::new();
    let w = g.thing("Widget", g.bob);
    g.run(g.bob, "@behavior/add Widget=test.fragile");

    let out = g.run(g.bob, "@destroy Widget");
    assert!(texts(&out).contains("refuses destruction"), "{}", texts(&out));
    assert!(!g.going(w), "object survived the vetoed destroy");

    // Remove the behavior, then destruction proceeds.
    g.run(g.bob, "@behavior/remove Widget=test.fragile");
    let out = g.run(g.bob, "@destroy Widget");
    assert!(texts(&out).contains("Destroyed"), "{}", texts(&out));
    assert!(g.going(w), "object destroyed after the behavior was removed");
}

#[test]
fn allow_policy_does_not_block_destroy() {
    let mut g = Game::new();
    let w = g.thing("Widget", g.bob);
    g.run(g.bob, "@behavior/add Widget=test.widget");
    let out = g.run(g.bob, "@destroy Widget");
    assert!(texts(&out).contains("Destroyed"), "{}", texts(&out));
    assert!(g.going(w));
}

#[test]
fn destroy_gate_vetoes_dynamically() {
    let mut g = Game::new();
    g.reg.add_destroy_gate(Arc::new(RefuseFortress));
    let fortress = g.thing("Fortress", g.bob);
    let shed = g.thing("Shed", g.bob);

    let out = g.run(g.bob, "@destroy Fortress");
    assert!(texts(&out).contains("under siege"), "{}", texts(&out));
    assert!(!g.going(fortress));

    // A different object is unaffected by the gate.
    let out = g.run(g.bob, "@destroy Shed");
    assert!(texts(&out).contains("Destroyed"), "{}", texts(&out));
    assert!(g.going(shed));
}

#[test]
fn room_destroy_is_refused_when_an_exit_refuses() {
    // The veto covers the cascade set: destroying a room would take its exits
    // with it, so an exit's own Refuse behavior must block the room's destroy.
    let mut g = Game::new();
    let hall = g.w.create(ObjKind::Room, "Hall", g.bob, None).unwrap();
    let exit = g.w.create(ObjKind::Exit, "South", g.bob, Some(hall)).unwrap();
    g.w.attach_behavior(
        exit,
        bid("test.fragile"),
        BehaviorBinding { schema_version: 1, attached_by: g.bob, attached_at: 0 },
    )
    .unwrap();

    let out = g.run(g.bob, &format!("@destroy #{}", hall.0));
    assert!(texts(&out).contains("would also destroy"), "{}", texts(&out));
    assert!(!g.going(hall), "room survived because its exit refused");
    assert!(!g.going(exit), "exit survived");
}

#[test]
fn world_destroy_returns_the_full_cascade_set() {
    // destroy_cmd emits one ObjDestroyed per id in this set, so cascaded exits
    // get cleanup too. The room's exit must be in it; a foreign exit must not.
    let mut g = Game::new();
    let hall = g.w.create(ObjKind::Room, "Hall", g.bob, None).unwrap();
    let back = g.w.create(ObjKind::Exit, "South", g.bob, Some(hall)).unwrap();
    let destroyed = g.w.destroy(hall).unwrap();
    assert!(destroyed.contains(&hall), "target in the set");
    assert!(destroyed.contains(&back), "the room's exit cascaded");
}

#[test]
fn unregistered_behavior_can_be_removed() {
    // A binding whose module is gone must still be removable to clean up cruft.
    let mut g = Game::new();
    let w = g.thing("Widget", g.bob);
    g.w.attach_behavior(
        w,
        bid("ghost.thing"),
        BehaviorBinding { schema_version: 1, attached_by: g.bob, attached_at: 0 },
    )
    .unwrap();

    let out = g.run(g.bob, "@behavior/list Widget");
    assert!(texts(&out).contains("unregistered"), "{}", texts(&out));

    let out = g.run(g.bob, "@behavior/remove Widget=ghost.thing");
    assert!(texts(&out).contains("Removed"), "{}", texts(&out));
    assert!(!g.has(w, "ghost.thing"));
}

#[test]
fn unregistered_behavior_does_not_block_destroy() {
    // Documented lenience (D8): a Refuse host loses protection when its module
    // isn't loaded to enforce it — a removed module can't make objects immortal.
    let mut g = Game::new();
    let w = g.thing("Widget", g.bob);
    g.w.attach_behavior(
        w,
        bid("ghost.thing"),
        BehaviorBinding { schema_version: 1, attached_by: g.bob, attached_at: 0 },
    )
    .unwrap();
    let out = g.run(g.bob, "@destroy Widget");
    assert!(texts(&out).contains("Destroyed"), "{}", texts(&out));
    assert!(g.going(w));
}

#[test]
fn world_destroy_stays_observer_free_and_bypasses_the_veto() {
    // The veto lives at the command layer (D8); `World::destroy` never consults
    // it, so a programmatic caller can still tear a bound object down (and must
    // validate first). This pins that invariant.
    let mut g = Game::new();
    let w = g.thing("Widget", g.bob);
    g.run(g.bob, "@behavior/add Widget=test.fragile");
    g.w.destroy(w).unwrap();
    assert!(g.going(w), "World::destroy ignores the behavior veto by design");
}

// ---------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------

#[test]
fn binding_survives_a_serde_round_trip() {
    let mut g = Game::new();
    let w = g.thing("Widget", g.bob);
    g.run(g.bob, "@behavior/add Widget=test.widget");

    let obj = g.w.get(w).unwrap().clone();
    let json = serde_json::to_string(&obj).unwrap();
    let back: mush_core::Obj = serde_json::from_str(&json).unwrap();
    assert!(back.behaviors.contains_key(&bid("test.widget")));
    let b: &BehaviorBinding = &back.behaviors[&bid("test.widget")];
    assert_eq!(b.attached_by, g.bob);
}

#[test]
fn empty_behaviors_are_omitted_from_json() {
    let g = Game::new();
    let obj = g.w.get(g.bob).unwrap();
    let json = serde_json::to_string(obj).unwrap();
    assert!(
        !json.contains("behaviors"),
        "empty behavior map should be skipped in the persisted doc: {json}"
    );
}

// ---------------------------------------------------------------------
// Contextual commands (cut 2: design 06 §4.3-4.4)
// ---------------------------------------------------------------------

fn echo(ctx: &mut Ctx<'_>, t: &BehaviorTarget) -> Result<(), CmdError> {
    let who = ctx.actor;
    let module = ctx.module.clone();
    ctx.out.info(who, format!("ran on {} via {:?} as {:?}", t.subject, t.reach, module));
    Ok(())
}

fn verb_spec(id: &str, verb: &str, reach: Option<Vec<CommandReach>>) -> BehaviorSpec {
    let cmd = BehaviorCommand::new(
        CommandSpec { name: verb.into(), shape: ArgShape::None, ..CommandSpec::default() },
        Arc::new(echo),
    );
    let cmd = match reach { Some(r) => cmd.with_reach(r), None => cmd };
    BehaviorSpec::new(bid(id)).with_command(cmd)
}

/// `test.console` supplies `startup` (default reach); `test.beacon` supplies an
/// ambient-only `beacon`. Registered through a Registrar so core stamps `owner`.
fn console_game() -> Game {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    r.behavior(verb_spec("test.console", "startup", None)).unwrap();
    r.behavior(verb_spec("test.beacon", "beacon", Some(vec![CommandReach::Ambient]))).unwrap();
    g
}

fn bind(g: &mut Game, obj: ObjId, behavior: &str) {
    g.w.attach_behavior(obj, bid(behavior), BehaviorBinding { schema_version: 1, attached_by: GOD, attached_at: 0 }).unwrap();
}

fn carried(g: &mut Game, name: &str) -> ObjId {
    let c = g.thing(name, g.bob);
    g.w.move_to(c, g.bob).unwrap();
    bind(g, c, "test.console");
    c
}

#[test]
fn carried_console_supplies_the_verb_with_owner_as_module() {
    let mut g = console_game();
    let c = carried(&mut g, "Console");
    let t = texts(&g.run(g.bob, "startup"));
    assert!(t.contains(&format!("ran on {c}")) && t.contains("Carried"), "{t}");
    assert!(t.contains("Some(\"vehicle\")"), "ctx.module is the behavior owner: {t}");
    // An actor who can reach no console gets the ordinary Huh?.
    assert!(texts(&g.run(g.ann, "startup")).contains("Huh?"));
}

#[test]
fn two_carried_consoles_are_ambiguous_until_one_is_removed() {
    let mut g = console_game();
    let a = carried(&mut g, "Alpha");
    let b = carried(&mut g, "Beta");
    let t = texts(&g.run(g.bob, "startup"));
    assert!(t.contains("Ambiguous") && t.contains("Alpha") && t.contains("Beta"), "{t}");
    g.w.detach_behavior(b, &bid("test.console")).unwrap();
    assert!(texts(&g.run(g.bob, "startup")).contains(&format!("ran on {a}")));
}

#[test]
fn behavior_verbs_are_exact_only_and_a_later_global_verb_shadows_them() {
    let mut g = console_game();
    carried(&mut g, "Console");
    assert!(texts(&g.run(g.bob, "startu")).contains("Huh?"), "no prefix matching");
    assert!(texts(&g.run(g.bob, "STARTUP")).contains("ran on"), "case-insensitive exact");
    g.reg
        .register(
            CommandSpec { name: "startup".into(), ..CommandSpec::default() },
            Arc::new(|ctx: &mut Ctx<'_>| { ctx.out.info(ctx.actor, "global startup"); Ok(()) }),
        )
        .unwrap();
    let t = texts(&g.run(g.bob, "startup"));
    assert!(t.contains("global startup") && !t.contains("ran on"), "global wins: {t}");
}

#[test]
fn registration_rejects_global_collisions_duplicates_and_empty_verbs() {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    let err = r.behavior(verb_spec("test.bad", "look", None)).unwrap_err();
    assert!(matches!(err, ModError::Conflict { what: "behavior command", .. }), "core `look`: {err}");
    let dup = BehaviorSpec::new(bid("test.dup"))
        .with_command(BehaviorCommand::new(CommandSpec { name: "ping".into(), ..CommandSpec::default() }, Arc::new(echo)))
        .with_command(BehaviorCommand::new(CommandSpec { name: "PING".into(), ..CommandSpec::default() }, Arc::new(echo)));
    let err = r.behavior(dup).unwrap_err();
    assert!(err.to_string().contains("twice"), "duplicate verb (case-insensitive): {err}");
    let empty = BehaviorSpec::new(bid("test.empty"))
        .with_command(BehaviorCommand::new(CommandSpec::default(), Arc::new(echo)));
    let err = r.behavior(empty).unwrap_err();
    assert!(err.to_string().contains("empty name"), "empty verb: {err}");
    assert!(r.behavior(verb_spec("test.ok", "zap", None)).is_ok());
}

#[test]
fn operated_link_outranks_carried_and_ambient_is_the_room_by_opt_in() {
    let mut g = console_game();
    let c = carried(&mut g, "Carried");
    let remote = g.thing("Remote", g.bob); // in the room, not carried: unreachable without a link
    bind(&mut g, remote, "test.console");
    assert!(texts(&g.run(g.bob, "startup")).contains(&format!("ran on {c}")));
    g.w.publish_command_subject(CommandSubjectLink {
        actor: g.bob, subject: remote, relation: "pilot".into(), owner: "vehicle".into(), priority: 0,
    });
    let t = texts(&g.run(g.bob, "startup"));
    assert!(t.contains(&format!("ran on {remote}")) && t.contains("Operated"), "{t}");
    // Ambient reach is the containing ROOM itself, never objects in it.
    bind(&mut g, LIMBO, "test.beacon");
    let t = texts(&g.run(g.bob, "beacon"));
    assert!(t.contains("ran on #0") && t.contains("Ambient"), "{t}");
    assert_eq!(g.w.retract_command_subjects("vehicle", Some(g.bob)), 1);
    assert!(texts(&g.run(g.bob, "startup")).contains(&format!("ran on {c}")));
}

#[test]
fn destroying_a_subject_scrubs_its_links() {
    let mut g = console_game();
    let remote = g.thing("Remote", g.bob);
    bind(&mut g, remote, "test.console");
    g.w.publish_command_subject(CommandSubjectLink {
        actor: g.bob, subject: remote, relation: "pilot".into(), owner: "vehicle".into(), priority: 0,
    });
    assert_eq!(g.w.command_subjects_of(g.bob).len(), 1);
    g.w.destroy(remote).unwrap();
    assert!(g.w.command_subjects_of(g.bob).is_empty(), "link scrubbed with the subject");
}

// ---------------------------------------------------------------------
// Review fixes: prefix shadowing, reach sets, link priority, folding, no fall-through
// ---------------------------------------------------------------------

#[test]
fn a_bare_prefix_of_a_global_is_refused_at_registration() {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    // `thin` is a strict prefix of core `think`: dispatch would run `think`.
    let err = r.behavior(verb_spec("test.thin", "thin", None)).unwrap_err();
    assert!(matches!(err, ModError::Conflict { what: "behavior command", .. }), "{err}");
    // `g` prefixes several bare globals: dispatch refuses it as ambiguous first.
    let err = r.behavior(verb_spec("test.g", "g", None)).unwrap_err();
    assert!(err.to_string().contains("ambiguous prefix"), "{err}");
    // A `+`/`@` verb never prefix-matches, so only an exact hit collides.
    assert!(r.behavior(verb_spec("test.plus", "+startup", None)).is_ok());
}

#[test]
fn a_linked_subject_keeps_its_carried_only_verbs() {
    let mut g = console_game();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    r.behavior(verb_spec("test.stowable", "stow", Some(vec![CommandReach::Carried]))).unwrap();
    drop(r);
    let bob = g.bob;
    let c = carried(&mut g, "Console");
    bind(&mut g, c, "test.stowable");
    assert!(texts(&g.run(bob, "stow")).contains("Carried"));
    g.w.publish_command_subject(CommandSubjectLink {
        actor: bob, subject: c, relation: "pilot".into(), owner: "vehicle".into(), priority: 0,
    });
    let t = texts(&g.run(bob, "stow"));
    assert!(t.contains(&format!("ran on {c}")) && t.contains("Carried"), "linked AND carried: {t}");
    assert!(texts(&g.run(bob, "startup")).contains("Operated"), "default-reach verb ranks Operated");
}

#[test]
fn link_priority_forms_subtiers_and_equal_priority_is_ambiguous() {
    let mut g = console_game();
    let bob = g.bob;
    let a = g.thing("Alpha", bob);
    bind(&mut g, a, "test.console");
    let b = g.thing("Beta", bob);
    bind(&mut g, b, "test.console");
    let link = |s: ObjId, p: u16| CommandSubjectLink {
        actor: bob, subject: s, relation: "remote".into(), owner: "vehicle".into(), priority: p,
    };
    g.w.publish_command_subject(link(a, 5));
    g.w.publish_command_subject(link(b, 0));
    let t = texts(&g.run(bob, "startup"));
    assert!(t.contains(&format!("ran on {a}")) && !t.contains("Ambiguous"), "higher priority wins: {t}");
    g.w.publish_command_subject(link(b, 5));
    let t = texts(&g.run(bob, "startup"));
    assert!(t.contains("Ambiguous") && t.contains("Alpha") && t.contains("Beta"), "{t}");
}

#[test]
fn an_ambiguous_tier_never_falls_through() {
    let mut g = console_game();
    let bob = g.bob;
    let a = g.thing("Alpha", bob);
    bind(&mut g, a, "test.console");
    let b = g.thing("Beta", bob);
    bind(&mut g, b, "test.console");
    let _c = carried(&mut g, "Carried"); // would answer alone
    for s in [a, b] {
        g.w.publish_command_subject(CommandSubjectLink {
            actor: bob, subject: s, relation: "remote".into(), owner: "vehicle".into(), priority: 0,
        });
    }
    let t = texts(&g.run(bob, "startup"));
    assert!(t.contains("Ambiguous") && t.contains("Alpha") && t.contains("Beta"), "{t}");
    assert!(!t.contains("ran on"), "the carried console must not answer for an ambiguous higher tier: {t}");
}

#[test]
fn verbs_fold_case_like_the_global_table() {
    let mut g = console_game();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    r.behavior(verb_spec("test.umlaut", "Über", None)).unwrap();
    drop(r);
    let bob = g.bob;
    let c = carried(&mut g, "Console");
    bind(&mut g, c, "test.umlaut");
    assert!(texts(&g.run(bob, "über")).contains(&format!("ran on {c}")));
    assert!(texts(&g.run(bob, "ÜBER")).contains(&format!("ran on {c}")));
}
