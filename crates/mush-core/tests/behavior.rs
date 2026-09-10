//! Phase A: object behavior bindings and the pre-destroy veto (design 06 §4, D8).
//! Drives the public API — a real `World`, the real `Registry` with the core
//! commands, and the dispatch pipeline — plus direct registration of test
//! behaviors and a test [`DestroyGate`].

use std::sync::Arc;

use mush_core::{
    ArgShape, BehaviorBinding, BehaviorCommand, BehaviorId, BehaviorSpec, BehaviorTarget,
    ChannelId, CmdError, CommandReach, CommandSpec, CommandSubjectLink, Ctx, DestroyGate,
    DestroyPolicy, Flags, GOD, LIMBO, ModError, ObjId, ObjKind, Outbox, Registrar, Registry, Role,
    World, dispatch, register_core_commands,
};
use mush_proto::OutputEvent;

/// A store-less lens for this harness: it holds no engine and never has, so
/// it names `Test` rather than reaching for a store-less twin.
const DETACHED: mush_core::TypedStateLens<'static> =
    mush_core::TypedStateLens::Detached(mush_core::DetachedTypedState::Test);

/// Refuses destroying anything named "Fortress" (a dynamic structural veto).
struct RefuseFortress;
impl DestroyGate for RefuseFortress {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        _actor: ObjId,
        target: ObjId,
    ) -> Result<(), String> {
        if w.get(target)
            .is_some_and(|o| o.name.eq_ignore_ascii_case("Fortress"))
        {
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
        dispatch(&self.reg, &mut self.w, actor, line, &mut out, DETACHED);
        out
    }

    /// Create a thing owned by `owner` in Limbo, returning its id.
    fn thing(&mut self, name: &str, owner: ObjId) -> ObjId {
        self.w
            .create(ObjKind::Thing, name, owner, Some(LIMBO))
            .unwrap()
    }

    fn has(&self, id: ObjId, behavior: &str) -> bool {
        self.w.has_behavior(id, &BehaviorId::new(behavior).unwrap())
    }

    fn going(&self, id: ObjId) -> bool {
        self.w
            .get(id)
            .is_some_and(|o| o.flags.contains(Flags::GOING))
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
    assert!(
        !g.has(c, "test.widget"),
        "unbound object exposes no behavior"
    );
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
    assert!(
        texts(&out).to_lowercase().contains("dotted"),
        "{}",
        texts(&out)
    );
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
    assert!(
        texts(&out).contains("conflicts"),
        "symmetric: {}",
        texts(&out)
    );
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
    assert!(
        texts(&out).contains("refuses destruction"),
        "{}",
        texts(&out)
    );
    assert!(!g.going(w), "object survived the vetoed destroy");

    // Remove the behavior, then destruction proceeds.
    g.run(g.bob, "@behavior/remove Widget=test.fragile");
    let out = g.run(g.bob, "@destroy Widget");
    assert!(texts(&out).contains("Destroyed"), "{}", texts(&out));
    assert!(
        g.going(w),
        "object destroyed after the behavior was removed"
    );
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
    let exit =
        g.w.create(ObjKind::Exit, "South", g.bob, Some(hall))
            .unwrap();
    g.w.attach_behavior(
        exit,
        bid("test.fragile"),
        BehaviorBinding {
            schema_version: 1,
            attached_by: g.bob,
            attached_at: 0,
        },
    )
    .unwrap();

    let out = g.run(g.bob, &format!("@destroy #{}", hall.0));
    assert!(
        texts(&out).contains("would also destroy"),
        "{}",
        texts(&out)
    );
    assert!(!g.going(hall), "room survived because its exit refused");
    assert!(!g.going(exit), "exit survived");
}

#[test]
fn world_destroy_returns_the_full_cascade_set() {
    // destroy_cmd emits one ObjDestroyed per id in this set, so cascaded exits
    // get cleanup too. The room's exit must be in it; a foreign exit must not.
    let mut g = Game::new();
    let hall = g.w.create(ObjKind::Room, "Hall", g.bob, None).unwrap();
    let back =
        g.w.create(ObjKind::Exit, "South", g.bob, Some(hall))
            .unwrap();
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
        BehaviorBinding {
            schema_version: 1,
            attached_by: g.bob,
            attached_at: 0,
        },
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
        BehaviorBinding {
            schema_version: 1,
            attached_by: g.bob,
            attached_at: 0,
        },
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
    assert!(
        g.going(w),
        "World::destroy ignores the behavior veto by design"
    );
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
    ctx.out.info(
        who,
        format!("ran on {} via {:?} as {:?}", t.subject, t.reach, module),
    );
    Ok(())
}

fn verb_spec(id: &str, verb: &str, reach: Option<Vec<CommandReach>>) -> BehaviorSpec {
    let cmd = BehaviorCommand::new(
        CommandSpec {
            name: verb.into(),
            shape: ArgShape::None,
            ..CommandSpec::default()
        },
        Arc::new(echo),
    );
    let cmd = match reach {
        Some(r) => cmd.with_reach(r),
        None => cmd,
    };
    BehaviorSpec::new(bid(id)).with_command(cmd)
}

/// `test.console` supplies `startup` (default reach); `test.beacon` supplies an
/// ambient-only `beacon`. Registered through a Registrar so core stamps `owner`.
fn console_game() -> Game {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    r.behavior(verb_spec("test.console", "startup", None))
        .unwrap();
    r.behavior(verb_spec(
        "test.beacon",
        "beacon",
        Some(vec![CommandReach::Ambient]),
    ))
    .unwrap();
    g
}

fn bind(g: &mut Game, obj: ObjId, behavior: &str) {
    g.w.attach_behavior(
        obj,
        bid(behavior),
        BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 0,
        },
    )
    .unwrap();
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
    assert!(
        t.contains(&format!("ran on {c}")) && t.contains("Carried"),
        "{t}"
    );
    assert!(
        t.contains("Some(\"vehicle\")"),
        "ctx.module is the behavior owner: {t}"
    );
    // An actor who can reach no console gets the ordinary Huh?.
    assert!(texts(&g.run(g.ann, "startup")).contains("Huh?"));
}

#[test]
fn two_carried_consoles_are_ambiguous_until_one_is_removed() {
    let mut g = console_game();
    let a = carried(&mut g, "Alpha");
    let b = carried(&mut g, "Beta");
    let t = texts(&g.run(g.bob, "startup"));
    assert!(
        t.contains("Ambiguous") && t.contains("Alpha") && t.contains("Beta"),
        "{t}"
    );
    g.w.detach_behavior(b, &bid("test.console")).unwrap();
    assert!(texts(&g.run(g.bob, "startup")).contains(&format!("ran on {a}")));
}

#[test]
fn behavior_verbs_are_exact_only_and_a_later_global_verb_shadows_them() {
    let mut g = console_game();
    carried(&mut g, "Console");
    assert!(
        texts(&g.run(g.bob, "startu")).contains("Huh?"),
        "no prefix matching"
    );
    assert!(
        texts(&g.run(g.bob, "STARTUP")).contains("ran on"),
        "case-insensitive exact"
    );
    g.reg
        .register(
            CommandSpec {
                name: "startup".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                ctx.out.info(ctx.actor, "global startup");
                Ok(())
            }),
        )
        .unwrap();
    let t = texts(&g.run(g.bob, "startup"));
    assert!(
        t.contains("global startup") && !t.contains("ran on"),
        "global wins: {t}"
    );
}

#[test]
fn registration_rejects_global_collisions_duplicates_and_empty_verbs() {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    let err = r.behavior(verb_spec("test.bad", "look", None)).unwrap_err();
    assert!(
        matches!(
            err,
            ModError::Conflict {
                what: "behavior command",
                ..
            }
        ),
        "core `look`: {err}"
    );
    let dup = BehaviorSpec::new(bid("test.dup"))
        .with_command(BehaviorCommand::new(
            CommandSpec {
                name: "ping".into(),
                ..CommandSpec::default()
            },
            Arc::new(echo),
        ))
        .with_command(BehaviorCommand::new(
            CommandSpec {
                name: "PING".into(),
                ..CommandSpec::default()
            },
            Arc::new(echo),
        ));
    let err = r.behavior(dup).unwrap_err();
    assert!(
        err.to_string().contains("twice"),
        "duplicate verb (case-insensitive): {err}"
    );
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
        actor: g.bob,
        subject: remote,
        station: "cockpit".into(),
        relation: "pilot".into(),
        owner: "vehicle".into(),
        priority: 0,
        channels: Default::default(),
        epoch: 0,
    });
    let t = texts(&g.run(g.bob, "startup"));
    assert!(
        t.contains(&format!("ran on {remote}")) && t.contains("Operated"),
        "{t}"
    );
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
        actor: g.bob,
        subject: remote,
        station: "cockpit".into(),
        relation: "pilot".into(),
        owner: "vehicle".into(),
        priority: 0,
        channels: Default::default(),
        epoch: 0,
    });
    assert_eq!(g.w.command_subjects_of(g.bob).len(), 1);
    g.w.destroy(remote).unwrap();
    assert!(
        g.w.command_subjects_of(g.bob).is_empty(),
        "link scrubbed with the subject"
    );
}

// ---------------------------------------------------------------------
// Review fixes: prefix shadowing, reach sets, link priority, folding, no fall-through
// ---------------------------------------------------------------------

#[test]
fn a_bare_prefix_of_a_global_is_refused_at_registration() {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    // `thin` is a strict prefix of core `think`: dispatch would run `think`.
    let err = r
        .behavior(verb_spec("test.thin", "thin", None))
        .unwrap_err();
    assert!(
        matches!(
            err,
            ModError::Conflict {
                what: "behavior command",
                ..
            }
        ),
        "{err}"
    );
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
    r.behavior(verb_spec(
        "test.stowable",
        "stow",
        Some(vec![CommandReach::Carried]),
    ))
    .unwrap();
    drop(r);
    let bob = g.bob;
    let c = carried(&mut g, "Console");
    bind(&mut g, c, "test.stowable");
    assert!(texts(&g.run(bob, "stow")).contains("Carried"));
    g.w.publish_command_subject(CommandSubjectLink {
        actor: bob,
        subject: c,
        station: "cockpit".into(),
        relation: "pilot".into(),
        owner: "vehicle".into(),
        priority: 0,
        channels: Default::default(),
        epoch: 0,
    });
    let t = texts(&g.run(bob, "stow"));
    assert!(
        t.contains(&format!("ran on {c}")) && t.contains("Carried"),
        "linked AND carried: {t}"
    );
    assert!(
        texts(&g.run(bob, "startup")).contains("Operated"),
        "default-reach verb ranks Operated"
    );
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
        actor: bob,
        subject: s,
        station: "remote".into(),
        relation: "remote".into(),
        owner: "vehicle".into(),
        priority: p,
        channels: Default::default(),
        epoch: 0,
    };
    g.w.publish_command_subject(link(a, 5));
    g.w.publish_command_subject(link(b, 0));
    let t = texts(&g.run(bob, "startup"));
    assert!(
        t.contains(&format!("ran on {a}")) && !t.contains("Ambiguous"),
        "higher priority wins: {t}"
    );
    g.w.publish_command_subject(link(b, 5));
    let t = texts(&g.run(bob, "startup"));
    assert!(
        t.contains("Ambiguous") && t.contains("Alpha") && t.contains("Beta"),
        "{t}"
    );
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
            actor: bob,
            subject: s,
            station: "remote".into(),
            relation: "remote".into(),
            owner: "vehicle".into(),
            priority: 0,
            channels: Default::default(),
            epoch: 0,
        });
    }
    let t = texts(&g.run(bob, "startup"));
    assert!(
        t.contains("Ambiguous") && t.contains("Alpha") && t.contains("Beta"),
        "{t}"
    );
    assert!(
        !t.contains("ran on"),
        "the carried console must not answer for an ambiguous higher tier: {t}"
    );
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

// ---------------------------------------------------------------------
// Core seams for the spatial graph: detach gate, carry gate, can_move_to
// ---------------------------------------------------------------------

/// Refuses detaching `test.fragile` from anything named "Anchor".
struct KeepFragileOnAnchor;
impl mush_core::DetachGate for KeepFragileOnAnchor {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        _actor: ObjId,
        obj: ObjId,
        behavior: &BehaviorId,
    ) -> Result<(), String> {
        if behavior == &bid("test.fragile") && w.get(obj).is_some_and(|o| o.name == "Anchor") {
            Err("The anchor still holds a live domain; retire it first.".into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_detach_gate_can_refuse_behavior_remove() {
    let mut g = Game::new();
    g.reg.add_detach_gate(Arc::new(KeepFragileOnAnchor));
    let a = g.thing("Anchor", g.bob);
    g.run(g.bob, "@behavior/add Anchor=test.fragile");
    let t = texts(&g.run(g.bob, "@behavior/remove Anchor=test.fragile"));
    assert!(t.contains("retire it first"), "{t}");
    assert!(
        g.has(a, "test.fragile"),
        "binding survived the refused remove"
    );
    let w = g.thing("Widget", g.bob);
    g.run(g.bob, "@behavior/add Widget=test.fragile");
    g.run(g.bob, "@behavior/remove Widget=test.fragile");
    assert!(!g.has(w, "test.fragile"), "other objects unaffected");
}

/// Vetoes carrying anything named "Bolted"; vetoes MOVING into anything named "Vault".
struct BoltedAndVault;
impl mush_core::EventHook for BoltedAndVault {
    fn carry_attempt(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        _mover: ObjId,
        obj: ObjId,
        _dest: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        if ctx.world().get(obj).is_some_and(|o| o.name == "Bolted") {
            Err(mush_proto::RichText::tag("error", "It is bolted down."))
        } else {
            Ok(())
        }
    }
    fn move_attempt(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        _actor: ObjId,
        _exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        if ctx.world().get(dest).is_some_and(|o| o.name == "Vault") {
            Err(mush_proto::RichText::tag("error", "The vault is sealed."))
        } else {
            Ok(())
        }
    }
}

#[test]
fn carry_gate_covers_get_and_drop_and_the_move_gate_never_fires_on_a_carry() {
    let mut g = Game::new();
    {
        let mut r = Registrar::new(&mut g.reg, "test");
        r.hook(mush_core::HookPoint::CarryAttempt, Arc::new(BoltedAndVault));
        r.hook(mush_core::HookPoint::MoveAttempt, Arc::new(BoltedAndVault));
    }
    let bob = g.bob;
    let bolted = g.thing("Bolted", bob);
    let t = texts(&g.run(bob, "get Bolted"));
    assert!(t.contains("bolted down"), "{t}");
    assert_eq!(
        g.w.get(bolted).and_then(|o| o.location),
        Some(LIMBO),
        "get refused, no move"
    );
    // A plain thing can be dropped inside the Vault: the MOVE gate (which vetoes
    // that destination) is deliberately not consulted for carry moves.
    let vault = g.w.create(ObjKind::Room, "Vault", GOD, None).unwrap();
    let wrench = g.thing("Wrench", bob);
    g.w.move_to(bob, vault).unwrap();
    g.w.move_to(wrench, bob).unwrap();
    let t = texts(&g.run(bob, "drop Wrench"));
    assert!(
        !t.contains("sealed"),
        "drop must not trigger the move gate: {t}"
    );
    assert_eq!(g.w.get(wrench).and_then(|o| o.location), Some(vault));
    // ...but walking INTO the vault is still vetoed by the move gate.
    g.w.move_to(bob, LIMBO).unwrap();
    let ex =
        g.w.create(ObjKind::Exit, "vaultdoor", GOD, Some(LIMBO))
            .unwrap();
    g.w.link_exit(ex, Some(vault)).unwrap();
    let t = texts(&g.run(bob, "vaultdoor"));
    assert!(t.contains("sealed"), "{t}");
    assert_eq!(g.w.get(bob).and_then(|o| o.location), Some(LIMBO));
}

#[test]
fn can_move_to_mirrors_move_to_checks_without_mutating() {
    let mut g = Game::new();
    let bag = g.thing("Bag", g.bob);
    let pouch = g.thing("Pouch", g.bob);
    g.w.move_to(pouch, bag).unwrap();
    assert!(
        g.w.can_move_to(bag, pouch).is_err(),
        "cycle: bag into its own pouch"
    );
    assert!(g.w.can_move_to(bag, bag).is_err(), "self");
    assert!(g.w.can_move_to(bag, ObjId(9999)).is_err(), "missing dest");
    assert!(g.w.can_move_to(bag, g.bob).is_ok());
    assert_eq!(
        g.w.get(bag).and_then(|o| o.location),
        Some(LIMBO),
        "no mutation"
    );
}

// ---------------------------------------------------------------------
// F3: control-channel authorization
// ---------------------------------------------------------------------

fn chan_spec(id: &str, verb: &str, channel: &str) -> BehaviorSpec {
    let cmd = BehaviorCommand::new(
        CommandSpec {
            name: verb.into(),
            shape: ArgShape::None,
            ..CommandSpec::default()
        },
        Arc::new(echo),
    )
    .with_channel(ChannelId::new(channel).unwrap());
    BehaviorSpec::new(bid(id)).with_command(cmd)
}
fn chans(names: &[&str]) -> std::collections::BTreeSet<ChannelId> {
    names.iter().map(|c| ChannelId::new(c).unwrap()).collect()
}
fn link(
    actor: ObjId,
    subject: ObjId,
    owner: &str,
    priority: u16,
    channels: &[&str],
) -> CommandSubjectLink {
    CommandSubjectLink {
        actor,
        subject,
        station: "primary".into(),
        relation: "pilot".into(),
        owner: owner.into(),
        priority,
        channels: chans(channels),
        epoch: 1,
    }
}

#[test]
fn a_channelled_command_resolves_only_with_the_granted_channel() {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    r.behavior(chan_spec("test.drive", "drive", "movement"))
        .unwrap();
    drop(r);
    let bob = g.bob;
    let mech = g.thing("Mech", bob);
    bind(&mut g, mech, "test.drive");
    // A gunner-style link (weapons only) cannot drive — the command does not resolve.
    g.w.publish_command_subject(link(bob, mech, "sim", 1, &["weapons"]));
    assert!(
        texts(&g.run(bob, "drive")).contains("Huh?"),
        "no movement channel: no resolution"
    );
    // A pilot-style link (movement) authorizes it (republish replaces the link).
    g.w.publish_command_subject(link(bob, mech, "sim", 2, &["movement"]));
    assert!(
        texts(&g.run(bob, "drive")).contains(&format!("ran on {mech}")),
        "movement channel: resolves at Operated"
    );
}

#[test]
fn a_channel_grant_from_another_owner_remains_independently_eligible() {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    r.behavior(chan_spec("test.drive", "drive", "movement"))
        .unwrap();
    drop(r);
    let bob = g.bob;
    let mech = g.thing("Mech", bob);
    bind(&mut g, mech, "test.drive");
    // `sim` grants weapons; a separate `rigging` grant carries movement. The
    // movement grant authorizes on its own; the two grants are never unioned.
    g.w.publish_command_subject(link(bob, mech, "sim", 1, &["weapons"]));
    g.w.publish_command_subject(link(bob, mech, "rigging", 0, &["movement"]));
    assert!(
        texts(&g.run(bob, "drive")).contains(&format!("ran on {mech}")),
        "independent movement grant remains eligible"
    );
}

#[test]
fn station_priority_cannot_promote_another_stations_channel() {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    r.behavior(chan_spec("test.drive", "drive", "movement"))
        .unwrap();
    drop(r);
    let bob = g.bob;
    let mixed = g.thing("Mixed", bob);
    let driver = g.thing("Driver", bob);
    bind(&mut g, mixed, "test.drive");
    bind(&mut g, driver, "test.drive");

    // Mixed has a priority-10 sensor station and a priority-1 movement
    // station. Driver has a priority-5 movement station. The old union/max
    // projection incorrectly promoted Mixed's movement to priority 10.
    g.w.publish_command_subject(CommandSubjectLink {
        actor: bob,
        subject: mixed,
        station: "sensors".into(),
        relation: "sensor".into(),
        owner: "sim".into(),
        priority: 10,
        channels: chans(&["sensors"]),
        epoch: 4,
    });
    g.w.publish_command_subject(CommandSubjectLink {
        actor: bob,
        subject: mixed,
        station: "driver".into(),
        relation: "driver".into(),
        owner: "sim".into(),
        priority: 1,
        channels: chans(&["movement"]),
        epoch: 4,
    });
    g.w.publish_command_subject(CommandSubjectLink {
        actor: bob,
        subject: driver,
        station: "driver".into(),
        relation: "driver".into(),
        owner: "sim".into(),
        priority: 5,
        channels: chans(&["movement"]),
        epoch: 8,
    });

    let text = texts(&g.run(bob, "drive"));
    assert!(
        text.contains(&format!("ran on {driver}")) && !text.contains("Ambiguous"),
        "priority must be selected from the movement-carrying grant: {text}"
    );
}

#[test]
fn exact_station_epoch_and_channel_revalidate_stale_intents() {
    let mut g = Game::new();
    let bob = g.bob;
    let mech = g.thing("Mech", bob);
    let movement = ChannelId::new("movement").unwrap();
    g.w.publish_command_subject(CommandSubjectLink {
        actor: bob,
        subject: mech,
        station: "cockpit".into(),
        relation: "pilot".into(),
        owner: "sim".into(),
        priority: 2,
        channels: chans(&["movement"]),
        epoch: 12,
    });
    assert!(g.w.channel_grant_is_current(bob, mech, "sim", "cockpit", 12, &movement));
    assert!(!g.w.channel_grant_is_current(bob, mech, "sim", "cockpit", 11, &movement));

    // Revocation/republish advances the epoch; a previously issued intent no
    // longer validates even though actor, station, and channel are unchanged.
    g.w.publish_command_subject(CommandSubjectLink {
        actor: bob,
        subject: mech,
        station: "cockpit".into(),
        relation: "pilot".into(),
        owner: "sim".into(),
        priority: 2,
        channels: chans(&["movement"]),
        epoch: 13,
    });
    assert!(!g.w.channel_grant_is_current(bob, mech, "sim", "cockpit", 12, &movement));
}

#[test]
fn a_channelled_command_must_be_operated_exclusive() {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    // Manually pair a required channel with a non-Operated reach — registration
    // must refuse it (else a Carried/Container/Ambient reach bypasses the gate).
    let mut cmd = BehaviorCommand::new(
        CommandSpec {
            name: "drive".into(),
            shape: ArgShape::None,
            ..CommandSpec::default()
        },
        Arc::new(echo),
    )
    .with_reach(vec![CommandReach::Operated, CommandReach::Carried]);
    cmd.required_channel = Some(ChannelId::new("movement").unwrap());
    let err = r
        .behavior(BehaviorSpec::new(bid("test.bad")).with_command(cmd))
        .unwrap_err();
    assert!(err.to_string().contains("Operated-exclusive"), "{err}");
}

#[test]
fn a_channel_denied_candidate_drops_before_ambiguity() {
    let mut g = Game::new();
    let mut r = Registrar::new(&mut g.reg, "vehicle");
    r.behavior(chan_spec("test.drive", "drive", "movement"))
        .unwrap();
    drop(r);
    let bob = g.bob;
    let a = g.thing("Alpha", bob);
    let b = g.thing("Beta", bob);
    bind(&mut g, a, "test.drive");
    bind(&mut g, b, "test.drive");
    // Two SAME-priority Operated links; only Alpha grants movement. Beta is
    // dropped BEFORE ambiguity counting, so this resolves to Alpha, not Ambiguous.
    g.w.publish_command_subject(link(bob, a, "sim", 0, &["movement"]));
    g.w.publish_command_subject(link(bob, b, "sim", 0, &[]));
    let t = texts(&g.run(bob, "drive"));
    assert!(
        t.contains(&format!("ran on {a}")) && !t.contains("Ambiguous"),
        "denied Beta drops: {t}"
    );
}
