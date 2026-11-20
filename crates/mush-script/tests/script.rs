//! End-to-end mush-script tests through the real [`Engine`].
//!
//! Every scenario drives scripts exactly as the game would: attributes set
//! through `@set` (which recompiles via the additive `AttrSet` signal),
//! commands and speech dispatched through the engine, timers fired through
//! the scheduler. Assertions read the outbox — no internals are poked.

use std::collections::HashMap;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::{
    Attr, AttrFlags, AttrValue, Audience, DeliveryClass, Flags, GOD, LIMBO, ObjId, ObjKind, Outbox,
    Registry, World, register_core_commands, register_engine_commands,
};
use mush_proto::OutputEvent;
use mush_script::{ScriptConfig, ScriptModule};

// Doc 43 cut TS.4: nested source against the depth guard.
#[path = "script/nesting.rs"]
mod nesting;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

struct Fixture {
    engine: Engine,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture::with_config(ScriptConfig::default())
    }

    fn with_config(cfg: ScriptConfig) -> Fixture {
        let mut world = World::new_minimal();
        world.set_now(1_000);
        let mut registry = Registry::new();
        register_core_commands(&mut registry).unwrap();
        register_engine_commands(&mut registry).unwrap();
        let mut engine = Engine::with_modules(
            world,
            registry,
            vec![Box::new(ScriptModule::with_config(cfg))],
        )
        .unwrap();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine
            .modules_loaded(HashMap::new(), &mut out, &mut fx)
            .unwrap();
        Fixture { engine }
    }

    fn player(&mut self, name: &str) -> ObjId {
        self.engine
            .world
            .create(ObjKind::Player, name, GOD, Some(LIMBO))
            .unwrap()
    }

    fn builder(&mut self, name: &str) -> ObjId {
        let id = self.player(name);
        self.engine
            .world
            .set_flag(id, Flags::BUILDER, true)
            .unwrap();
        id
    }

    fn thing(&mut self, name: &str, owner: ObjId, loc: ObjId) -> ObjId {
        self.engine
            .world
            .create(ObjKind::Thing, name, owner, Some(loc))
            .unwrap()
    }

    fn room(&mut self, name: &str) -> ObjId {
        self.engine
            .world
            .create(ObjKind::Room, name, GOD, None)
            .unwrap()
    }

    /// Set a `script:*` attribute directly, then let the module compile it
    /// (as if loaded at boot). Bypasses `@set` for scripts whose text is
    /// awkward to type on a command line.
    fn seed_script(&mut self, obj: ObjId, key: &str, code: &str) {
        let owner = self.engine.world.get(obj).unwrap().owner;
        self.engine
            .world
            .set_attr(obj, key, Attr::text(code, owner))
            .unwrap();
        // Trigger a recompile via the AttrSet observer path.
        self.run(GOD, &format!("@set {obj}/{key}={code}"));
    }

    fn run(&mut self, actor: ObjId, line: &str) -> Vec<(Audience, OutputEvent)> {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        self.engine.step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.into(),
            },
            &mut out,
            &mut fx,
        );
        out.take()
    }

    fn run_classified(
        &mut self,
        actor: ObjId,
        line: &str,
    ) -> Vec<(DeliveryClass, Audience, OutputEvent)> {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        self.engine.step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.into(),
            },
            &mut out,
            &mut fx,
        );
        out.take_classified()
    }

    fn run_fx(&mut self, actor: ObjId, line: &str) -> (Vec<(Audience, OutputEvent)>, Effects) {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        self.engine.step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.into(),
            },
            &mut out,
            &mut fx,
        );
        (out.take(), fx)
    }

    fn set_now(&mut self, secs: i64) {
        self.engine.set_now(secs);
    }

    fn fire_due(&mut self, now_secs: i64) -> Vec<(Audience, OutputEvent)> {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        self.engine.fire_due(now_secs * 1_000, &mut out, &mut fx);
        out.take()
    }

    fn flag(&self, obj: ObjId, flag: Flags) -> bool {
        self.engine
            .world
            .get(obj)
            .is_some_and(|o| o.flags.contains(flag))
    }

    fn desc(&self, obj: ObjId) -> String {
        self.engine
            .world
            .attr(obj, "desc")
            .and_then(|a| a.as_text().map(str::to_string))
            .unwrap_or_default()
    }
}

#[test]
fn script_commands_and_dollar_fallback_are_declared_legacy_compatibility() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.engine.registry.compatibility_of("use"),
        Some(mush_core::CommandCompatibility::LegacyPostImage)
    );
    assert_eq!(
        fixture
            .engine
            .registry
            .command_fallback_compatibilities()
            .collect::<Vec<_>>(),
        vec![(
            Some("script"),
            mush_core::CommandCompatibility::LegacyPostImage
        )]
    );
}

fn all_text(events: &[(Audience, OutputEvent)]) -> Vec<String> {
    events
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            OutputEvent::Narrative { text, .. } => Some(text.to_plain_text()),
            _ => None,
        })
        .collect()
}

fn saw(events: &[(Audience, OutputEvent)], needle: &str) -> bool {
    all_text(events).iter().any(|t| t.contains(needle))
}

fn actor_saw(events: &[(Audience, OutputEvent)], actor: ObjId, needle: &str) -> bool {
    events.iter().any(|(aud, e)| {
        matches!(aud, Audience::Actor(a) if *a == actor)
            && matches!(e,
                OutputEvent::Narrative { text, .. } | OutputEvent::Sys(text)
                if text.to_plain_text().contains(needle))
    })
}

fn classified_actor_saw(
    events: &[(DeliveryClass, Audience, OutputEvent)],
    class: DeliveryClass,
    actor: ObjId,
    needle: &str,
) -> bool {
    events.iter().any(|(actual_class, aud, event)| {
        *actual_class == class
            && matches!(aud, Audience::Actor(a) if *a == actor)
            && matches!(event,
                OutputEvent::Narrative { text, .. } | OutputEvent::Sys(text)
                if text.to_plain_text().contains(needle))
    })
}

fn count(events: &[(Audience, OutputEvent)], needle: &str) -> usize {
    all_text(events)
        .iter()
        .filter(|t| t.contains(needle))
        .count()
}

// ----------------------------------------------------------------------
// Compile / cache / recompile + a $-command
// ----------------------------------------------------------------------

#[test]
fn command_greet_pemits_caller_and_recompiles_on_change() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bell = f.thing("bell", nick, LIMBO);
    f.seed_script(
        bell,
        "script:command:greet",
        r#"mush::pemit(mush::caller(), "Hi " + arg)"#,
    );

    let out = f.run(nick, "greet Bob");
    assert!(actor_saw(&out, nick, "Hi Bob"), "{:?}", all_text(&out));

    // Recompile-on-change: edit the script, new behavior takes effect.
    f.run(
        GOD,
        &format!(r#"@set {bell}/script:command:greet=mush::pemit(mush::caller(), "Yo " + arg)"#),
    );
    let out = f.run(nick, "greet Sam");
    assert!(actor_saw(&out, nick, "Yo Sam"));
    assert!(
        !saw(&f.run(nick, "greet Sam"), "Hi "),
        "old AST must be gone"
    );
}

#[test]
fn unknown_verb_without_script_still_huhs() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let out = f.run(nick, "flibbertigibbet");
    assert!(saw(&out, "Huh?"), "no fallback should not swallow Huh");
}

#[test]
fn boot_compiles_existing_scripts() {
    // A script present before the module loads is compiled by on_world_loaded.
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    world.set_flag(nick, Flags::BUILDER, true).unwrap();
    let bell = world
        .create(ObjKind::Thing, "bell", nick, Some(LIMBO))
        .unwrap();
    world
        .set_attr(
            bell,
            "script:command:ping",
            Attr::text(r#"mush::pemit(mush::caller(), "pong")"#, nick),
        )
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, vec![Box::new(ScriptModule::new())]).unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();

    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor: nick,
            line: "ping".into(),
        },
        &mut out,
        &mut fx,
    );
    assert!(actor_saw(out.events(), nick, "pong"));
}

// ----------------------------------------------------------------------
// Hooks: enter / hear / use / timer
// ----------------------------------------------------------------------

#[test]
fn on_enter_announces_arrivals() {
    let mut f = Fixture::new();
    let hall = f.room("Hall");
    let foyer = f.room("Foyer");
    // Exit Foyer -> Hall.
    f.engine
        .world
        .create(ObjKind::Exit, "north;n", GOD, Some(foyer))
        .unwrap();
    let exit = f.engine.world.exits_of(foyer)[0];
    f.engine.world.link_exit(exit, Some(hall)).unwrap();

    let nick = f.player("Nick");
    f.engine.world.move_to(nick, foyer).unwrap();

    f.seed_script(
        hall,
        "script:enter",
        r#"mush::emit(mush::me(), mush::name(mover) + " strolls in.")"#,
    );

    let out = f.run(nick, "north");
    assert!(saw(&out, "Nick strolls in."), "{:?}", all_text(&out));
}

#[test]
fn on_hear_reacts_to_keyword() {
    let mut f = Fixture::new();
    let doorbell = f.thing("doorbell", GOD, LIMBO);
    let nick = f.player("Nick");
    f.seed_script(
        doorbell,
        "script:hear:ring",
        r#"if text.contains("hello") { mush::emit(mush::here(), "The doorbell chimes!") }"#,
    );

    // A line without the keyword does nothing.
    let out = f.run(nick, "say goodbye");
    assert!(!saw(&out, "chimes"));

    // The keyword triggers the reflex.
    let out = f.run(nick, "say well hello there");
    assert!(saw(&out, "The doorbell chimes!"), "{:?}", all_text(&out));
}

#[test]
fn on_use_runs_use_script_else_refuses() {
    let mut f = Fixture::new();
    let nick = f.player("Nick");
    let lever = f.thing("lever", GOD, LIMBO);
    let rock = f.thing("rock", GOD, LIMBO);
    f.seed_script(
        lever,
        "script:use",
        r#"mush::pemit(user, "The lever clunks.")"#,
    );

    let out = f.run(nick, "use lever");
    assert!(actor_saw(&out, nick, "The lever clunks."));

    let out = f.run(nick, "use rock");
    assert!(actor_saw(&out, nick, "You can't use that."));
    let _ = rock;
}

#[test]
fn on_timer_fires_via_schedule_and_manual_clock() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let clock = f.thing("clock", nick, LIMBO);
    f.seed_script(clock, "script:command:arm", r#"mush::schedule(5, "boom")"#);
    f.seed_script(
        clock,
        "script:timer:boom",
        r#"mush::emit(mush::here(), "BOOM")"#,
    );

    // Arm it: nothing fires yet, a WaitCmd is queued.
    let (out, _fx) = f.run_fx(nick, "arm");
    assert!(!saw(&out, "BOOM"));

    // Before the deadline: still nothing.
    f.set_now(1_003);
    assert!(!saw(&f.fire_due(1_003), "BOOM"));

    // At the deadline the timer fires as the object and emits.
    f.set_now(1_005);
    let out = f.fire_due(1_005);
    assert!(saw(&out, "BOOM"), "{:?}", all_text(&out));
}

// ----------------------------------------------------------------------
// Permission enforcement (as the OWNER, not the trigger)
// ----------------------------------------------------------------------

#[test]
fn mortal_script_cannot_set_attr_on_uncontrolled_object() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bob = f.player("Bob");
    let vault = f.thing("vault", bob, LIMBO); // owned by Bob
    f.engine
        .world
        .set_attr(vault, "desc", Attr::text("intact", GOD))
        .unwrap();
    let tool = f.thing("tool", nick, LIMBO); // Nick's scripted object
    f.seed_script(
        tool,
        "script:command:hack",
        r#"mush::set_attr(mush::matchit("vault"), "desc", "pwned")"#,
    );

    let out = f.run(nick, "hack");
    assert_eq!(f.desc(vault), "intact", "write must be refused");
    assert!(
        actor_saw(&out, nick, "failed"),
        "owner is told: {:?}",
        all_text(&out)
    );
    let _ = bob;
}

#[test]
fn mortal_script_cannot_move_a_wizards_object() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let idol = f.thing("idol", GOD, LIMBO); // wizard-owned
    let tool = f.thing("tool", nick, LIMBO);
    f.seed_script(
        tool,
        "script:command:grab",
        r#"mush::move(mush::matchit("idol"), mush::me())"#,
    );

    let out = f.run(nick, "grab");
    assert_eq!(
        f.engine.world.get(idol).unwrap().location,
        Some(LIMBO),
        "the idol must not move"
    );
    assert!(actor_saw(&out, nick, "failed"));
}

#[test]
fn owner_script_can_set_attr_on_its_own_object() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let diary = f.thing("diary", nick, LIMBO);
    f.seed_script(
        diary,
        "script:command:write",
        r#"mush::set_attr(mush::me(), "desc", "Dear diary")"#,
    );
    f.run(nick, "write");
    assert_eq!(f.desc(diary), "Dear diary");
}

// ----------------------------------------------------------------------
// Budget enforcement
// ----------------------------------------------------------------------

#[test]
fn infinite_loop_is_aborted_by_operation_cap() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(
        bomb,
        "script:command:spin",
        "let x = 0; while true { x += 1; }",
    );
    let out = f.run(nick, "spin");
    assert!(actor_saw(&out, nick, "failed"), "{:?}", all_text(&out));
}

#[test]
fn huge_string_is_aborted_by_memory_cap() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(
        bomb,
        "script:command:grow",
        r#"let s = "x"; while true { s += s; }"#,
    );
    let out = f.run(nick, "grow");
    assert!(actor_saw(&out, nick, "failed"));
}

#[test]
fn deep_recursion_is_aborted_by_call_level_cap() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(bomb, "script:command:dive", "fn dive() { dive(); } dive();");
    let out = f.run(nick, "dive");
    assert!(actor_saw(&out, nick, "failed"));
}

#[test]
fn wall_clock_backstop_aborts_a_slow_script() {
    let cfg = ScriptConfig {
        wall_clock_ms: 5,
        max_operations: 5_000_000_000,
        ..ScriptConfig::default()
    };
    let mut f = Fixture::with_config(cfg);
    let nick = f.builder("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(
        bomb,
        "script:command:stall",
        "let x = 0; while true { x += 1; }",
    );
    let out = f.run(nick, "stall");
    assert!(actor_saw(&out, nick, "failed"), "wall clock must trip");
}

#[test]
fn compile_error_disables_only_that_script_and_notifies_owner() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let obj = f.thing("obj", nick, LIMBO);
    // A good script and a broken one on the same object.
    f.seed_script(
        obj,
        "script:command:good",
        r#"mush::pemit(mush::caller(), "ok")"#,
    );
    let out = f.run_classified(
        GOD,
        &format!("@set {obj}/script:command:bad=this is (not valid rhai"),
    );
    assert!(
        classified_actor_saw(&out, DeliveryClass::OnCommit, nick, "failed to compile"),
        "a compile diagnostic accompanies the committed attribute change"
    );
    // The good one still runs; the bad one just Huhs (never compiled).
    assert!(actor_saw(&f.run(nick, "good"), nick, "ok"));
    assert!(saw(&f.run(nick, "bad blah"), "Huh?"));
}

// ----------------------------------------------------------------------
// 3-strike HALTED auto-disable
// ----------------------------------------------------------------------

#[test]
fn three_budget_overruns_halt_the_object() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bomb = f.thing("bomb", nick, LIMBO);
    f.seed_script(bomb, "script:command:spin", "while true {}");

    for _ in 0..2 {
        let out = f.run(nick, "spin");
        assert!(actor_saw(&out, nick, "failed"));
        assert!(!f.flag(bomb, Flags::HALTED));
    }
    // Third overrun trips the auto-disable.
    let out = f.run_classified(nick, "spin");
    assert!(
        classified_actor_saw(&out, DeliveryClass::OnCommit, nick, "failed"),
        "the third failure report must wait for the HALTED write"
    );
    assert!(
        classified_actor_saw(&out, DeliveryClass::OnCommit, nick, "HALTED"),
        "the HALTED claim must be released only after its write commits"
    );
    assert!(f.flag(bomb, Flags::HALTED));

    // A HALTED object's scripts no longer run at all.
    let out = f.run(nick, "spin");
    assert!(
        saw(&out, "Huh?"),
        "HALTED object is inert: {:?}",
        all_text(&out)
    );
}

// ----------------------------------------------------------------------
// Re-entrancy cap
// ----------------------------------------------------------------------

#[test]
fn cascade_cap_stops_an_emit_hear_ping_pong() {
    let cfg = ScriptConfig {
        max_total_invocations: 8,
        ..ScriptConfig::default()
    };
    let mut f = Fixture::with_config(cfg);
    let nick = f.player("Nick");
    let a = f.thing("alpha", GOD, LIMBO);
    let b = f.thing("beta", GOD, LIMBO);
    f.seed_script(a, "script:hear:x", r#"mush::emit(mush::here(), "ping")"#);
    f.seed_script(b, "script:hear:x", r#"mush::emit(mush::here(), "pong")"#);

    // Without a cap this never terminates; with one it must, and be bounded.
    let out = f.run(nick, "say start");
    let pings = count(&out, "ping");
    let pongs = count(&out, "pong");
    let total = pings + pongs;
    assert!(total >= 3, "the cascade actually cascaded: {total}");
    assert!(total <= 8, "the cascade is bounded by the cap: {total}");
}

// ----------------------------------------------------------------------
// API surface + ObjId round-trip
// ----------------------------------------------------------------------

#[test]
fn there_is_no_mush_cmd() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let obj = f.thing("obj", nick, LIMBO);
    f.seed_script(obj, "script:command:sneaky", r#"mush::cmd("look")"#);
    let out = f.run(nick, "sneaky");
    // The call errors (no such function) and no room render happens.
    assert!(actor_saw(&out, nick, "failed"));
    assert!(
        !out.iter().any(|(_, e)| matches!(e, OutputEvent::Room(_))),
        "mush::cmd must not have run a command"
    );
}

#[test]
fn objid_round_trips_through_rhai() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let mirror = f.thing("mirror", nick, LIMBO);
    f.seed_script(
        mirror,
        "script:command:whoami",
        r#"mush::pemit(mush::caller(), "me=" + to_string(mush::me()) + " caller=" + to_string(mush::caller()))"#,
    );
    let out = f.run(nick, "whoami");
    let expected = format!("me={mirror} caller={nick}");
    assert!(actor_saw(&out, nick, &expected), "{:?}", all_text(&out));

    // Equality of ObjId values works too.
    f.seed_script(
        mirror,
        "script:command:same",
        r#"if mush::here() == mush::loc(mush::me()) { mush::pemit(mush::caller(), "yes") } else { mush::pemit(mush::caller(), "no") }"#,
    );
    assert!(actor_saw(&f.run(nick, "same"), nick, "yes"));
}

#[test]
fn dice_rolls_within_bounds() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let die = f.thing("die", nick, LIMBO);
    f.seed_script(
        die,
        "script:command:roll",
        r#"mush::pemit(mush::caller(), "r=" + (mush::dice("2d6+1")).to_string())"#,
    );
    // Several rolls, all within [3, 13].
    for _ in 0..12 {
        let out = f.run(nick, "roll");
        let line = all_text(&out)
            .into_iter()
            .find(|t| t.starts_with("r="))
            .unwrap();
        let n: i64 = line.trim_start_matches("r=").parse().unwrap();
        assert!((3..=13).contains(&n), "2d6+1 out of range: {n}");
    }
}

#[test]
fn command_scripts_are_a_fallback_not_a_shadow() {
    // A script:command:say must NOT capture the built-in `say`.
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let obj = f.thing("obj", nick, LIMBO);
    f.seed_script(
        obj,
        "script:command:say",
        r#"mush::pemit(mush::caller(), "HIJACKED")"#,
    );
    let out = f.run(nick, "say hello");
    assert!(saw(&out, "hello"), "built-in say still runs");
    assert!(!saw(&out, "HIJACKED"), "script must not shadow a built-in");
}

#[test]
fn command_script_never_shadows_an_exit() {
    // Dispatch matches exits (step 4) before the script fallback (step 5): a
    // `script:command:north` must never fire while a `north` exit exists.
    let mut f = Fixture::new();
    let hall = f.room("Hall");
    let foyer = f.room("Foyer");
    f.engine
        .world
        .create(ObjKind::Exit, "north;n", GOD, Some(foyer))
        .unwrap();
    let exit = f.engine.world.exits_of(foyer)[0];
    f.engine.world.link_exit(exit, Some(hall)).unwrap();
    let nick = f.builder("Nick");
    f.engine.world.move_to(nick, foyer).unwrap();
    // A scripted object in the room offers a rival `north` $-command.
    let gadget = f.thing("gadget", nick, foyer);
    f.seed_script(
        gadget,
        "script:command:north",
        r#"mush::pemit(mush::caller(), "SHADOW")"#,
    );

    let out = f.run(nick, "north");
    assert_eq!(
        f.engine.world.get(nick).unwrap().location,
        Some(hall),
        "the exit wins over the script fallback"
    );
    assert!(
        !saw(&out, "SHADOW"),
        "script:command must not shadow an exit: {:?}",
        all_text(&out)
    );
}

// ----------------------------------------------------------------------
// Sandbox-safety regressions (M5 review)
// ----------------------------------------------------------------------

#[test]
fn dice_non_ascii_or_overflow_fails_without_panicking() {
    // Regression (SEV-1): a non-ASCII first char used to slice mid-codepoint
    // and panic the whole world task; an extreme modifier used to
    // overflow-panic in a debug build. Both must now be plain, catchable
    // script errors — and reaching the assertions at all proves no panic.
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let die = f.thing("die", nick, LIMBO);
    f.seed_script(
        die,
        "script:command:roll",
        r#"mush::pemit(mush::caller(), "r=" + (mush::dice(arg)).to_string())"#,
    );
    for bad in [
        "\u{a3}5",
        "\u{d7}2",
        "\u{b2}",
        "\u{df}d6",
        "1d1+9223372036854775807",
    ] {
        let out = f.run(nick, &format!("roll {bad}"));
        assert!(
            actor_saw(&out, nick, "failed"),
            "bad dice {bad:?} should fail gracefully: {:?}",
            all_text(&out)
        );
    }
    // A valid roll still works afterward (the object survived).
    let out = f.run(nick, "roll 2d6+1");
    assert!(actor_saw(&out, nick, "r="), "{:?}", all_text(&out));
}

#[test]
fn on_enter_reflex_fires_exactly_once_per_move() {
    // A double-fire (Moved + a manual collect, or a doubly-registered
    // observer) would still leave the token present; assert the *count*.
    let mut f = Fixture::new();
    let hall = f.room("Hall");
    let foyer = f.room("Foyer");
    f.engine
        .world
        .create(ObjKind::Exit, "north;n", GOD, Some(foyer))
        .unwrap();
    let exit = f.engine.world.exits_of(foyer)[0];
    f.engine.world.link_exit(exit, Some(hall)).unwrap();
    let nick = f.player("Nick");
    f.engine.world.move_to(nick, foyer).unwrap();
    f.seed_script(
        hall,
        "script:enter",
        r#"mush::emit(mush::me(), "ARRIVED-ONCE-TOKEN")"#,
    );

    let out = f.run(nick, "north");
    assert_eq!(
        count(&out, "ARRIVED-ONCE-TOKEN"),
        1,
        "the enter reflex fires exactly once: {:?}",
        all_text(&out)
    );
}

#[test]
fn script_authored_script_compiles_inline() {
    // A script writing a `script:*` attr recompiles inline, so a script
    // authored by a script is live immediately (not only after a reboot).
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let obj = f.thing("obj", nick, LIMBO);
    f.seed_script(
        obj,
        "script:command:author",
        r#"mush::set_attr(mush::me(), "script:command:foo", "mush::pemit(mush::caller(), arg)")"#,
    );
    // Before authoring, `foo` is unknown.
    assert!(saw(&f.run(nick, "foo hi"), "Huh?"));
    // Author it, then it runs immediately.
    f.run(nick, "author");
    let out = f.run(nick, "foo LIVE");
    assert!(
        actor_saw(&out, nick, "LIVE"),
        "inline recompile makes the new script live: {:?}",
        all_text(&out)
    );
}

#[test]
fn get_attr_withholds_hidden_attrs_from_uncontrolled_reader() {
    // SEV-2: HIDDEN (and other restricted) attrs must not be readable by a
    // script whose owner cannot examine the object; plain attrs stay readable.
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bob = f.player("Bob");
    let vault = f.thing("vault", bob, LIMBO); // Bob's — Nick can't examine
    f.engine
        .world
        .set_attr(
            vault,
            "secret",
            Attr {
                value: AttrValue::Text("password123".into()),
                owner: bob,
                flags: AttrFlags::HIDDEN,
            },
        )
        .unwrap();
    f.engine
        .world
        .set_attr(vault, "public", Attr::text("hello", bob))
        .unwrap();
    let tool = f.thing("tool", nick, LIMBO);
    f.seed_script(
        tool,
        "script:command:peek",
        r#"mush::pemit(mush::caller(), "s=[" + mush::get_attr(mush::matchit("vault"), "secret") + "] p=[" + mush::get_attr(mush::matchit("vault"), "public") + "]")"#,
    );

    let out = f.run(nick, "peek");
    assert!(
        actor_saw(&out, nick, "s=[]"),
        "HIDDEN attr must be withheld: {:?}",
        all_text(&out)
    );
    assert!(
        actor_saw(&out, nick, "p=[hello]"),
        "a plain attr stays readable: {:?}",
        all_text(&out)
    );
    let _ = bob;
}

#[test]
fn contents_omits_dark_members_a_script_cannot_see() {
    // SEV-3: contents honors DARK visibility — a DARK member the owner does
    // not control is omitted, exactly as the look layer would hide it.
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bin = f.thing("bin", nick, LIMBO); // Nick controls the container
    let plain = f.thing("plain", GOD, LIMBO);
    let dark = f.thing("dark", GOD, LIMBO);
    f.engine.world.move_to(plain, bin).unwrap();
    f.engine.world.move_to(dark, bin).unwrap();
    f.engine.world.set_flag(dark, Flags::DARK, true).unwrap();
    let tool = f.thing("tool", nick, LIMBO);
    f.seed_script(
        tool,
        "script:command:peek",
        r#"mush::pemit(mush::caller(), "n=" + mush::contents(mush::matchit("bin")).len().to_string())"#,
    );

    let out = f.run(nick, "peek");
    assert!(
        actor_saw(&out, nick, "n=1"),
        "the DARK member is omitted (only 'plain' is visible): {:?}",
        all_text(&out)
    );
}

#[test]
fn script_set_attr_respects_wizard_only_and_preserves_flags() {
    // SEV-4: mush::set_attr must not overwrite a WIZARD_ONLY attr as a
    // non-Wizard, and must carry existing attr flags forward on rewrite.
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let obj = f.thing("obj", nick, LIMBO); // Nick owns it
    // Staff stored a WIZARD_ONLY attr; Nick set a HIDDEN note of his own.
    f.engine
        .world
        .set_attr(
            obj,
            "staff",
            Attr {
                value: AttrValue::Text("secret".into()),
                owner: GOD,
                flags: AttrFlags::WIZARD_ONLY,
            },
        )
        .unwrap();
    f.engine
        .world
        .set_attr(
            obj,
            "note",
            Attr {
                value: AttrValue::Text("v1".into()),
                owner: nick,
                flags: AttrFlags::HIDDEN,
            },
        )
        .unwrap();
    f.seed_script(
        obj,
        "script:command:clobber",
        r#"mush::set_attr(mush::me(), "staff", "pwned")"#,
    );
    f.seed_script(
        obj,
        "script:command:edit",
        r#"mush::set_attr(mush::me(), "note", "v2")"#,
    );

    // The WIZARD_ONLY overwrite is refused; the value is unchanged.
    f.run(nick, "clobber");
    assert_eq!(
        f.engine
            .world
            .attr(obj, "staff")
            .and_then(|a| a.as_text().map(str::to_string)),
        Some("secret".to_string()),
        "a non-Wizard script must not overwrite a WIZARD_ONLY attr"
    );

    // Rewriting Nick's own HIDDEN note is allowed and keeps the HIDDEN flag.
    f.run(nick, "edit");
    let note = f.engine.world.get(obj).unwrap().attrs.get("note").cloned();
    let note = note.expect("note attr exists");
    assert_eq!(note.as_text(), Some("v2"));
    assert!(
        note.flags.contains(AttrFlags::HIDDEN),
        "existing attr flags must be carried forward, not stripped"
    );
}

// ----------------------------------------------------------------------
// Tier 0 security hardening regressions (CRIT-5, CRIT-6, CRIT-7, CRIT-8, HIGH-16)
// ----------------------------------------------------------------------

#[test]
fn crit5_plain_player_cannot_author_compile_or_execute_scripts() {
    // 1. Plain Role::Player cannot write `script:*` attributes via `@set`.
    let mut f = Fixture::new();
    let eve = f.player("Eve");
    let out = f.run(eve, r#"@set me/script:use=mush::pemit(user, "PWNED")"#);
    assert!(
        actor_saw(&out, eve, "Permission denied"),
        "plain player @set on script:* must be refused: {:?}",
        all_text(&out)
    );
    assert!(
        f.engine.world.attr(eve, "script:use").is_none(),
        "script:use must not be written by a plain player"
    );

    // 2. Pre-seeded `script:*` attrs on objects owned by a plain Role::Player
    //    must neither compile at boot nor execute on any hook (command, use,
    //    hear, enter, timer).
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let eve = world
        .create(ObjKind::Player, "Eve", GOD, Some(LIMBO))
        .unwrap();
    let foyer = world.create(ObjKind::Room, "Foyer", GOD, None).unwrap();
    let hall = world.create(ObjKind::Room, "Hall", eve, None).unwrap();
    let exit = world
        .create(ObjKind::Exit, "north;n", GOD, Some(foyer))
        .unwrap();
    world.link_exit(exit, Some(hall)).unwrap();
    let gadget = world
        .create(ObjKind::Thing, "gadget", eve, Some(foyer))
        .unwrap();
    world.move_to(eve, foyer).unwrap();

    for (obj, key, code) in [
        (
            gadget,
            "script:command:evil",
            r#"mush::pemit(mush::caller(), "EVIL-CMD")"#,
        ),
        (gadget, "script:use", r#"mush::pemit(user, "EVIL-USE")"#),
        (
            gadget,
            "script:hear:hi",
            r#"mush::emit(mush::here(), "EVIL-HEAR")"#,
        ),
        (
            gadget,
            "script:timer:boom",
            r#"mush::emit(mush::here(), "EVIL-TIMER")"#,
        ),
        (
            hall,
            "script:enter",
            r#"mush::emit(mush::me(), "EVIL-ENTER")"#,
        ),
    ] {
        world.set_attr(obj, key, Attr::text(code, eve)).unwrap();
    }

    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, vec![Box::new(ScriptModule::new())]).unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();

    let mut step = |actor: ObjId, line: &str| {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.into(),
            },
            &mut out,
            &mut fx,
        );
        out.take()
    };

    assert!(saw(&step(eve, "evil"), "Huh?"));
    assert!(actor_saw(
        &step(eve, "use gadget"),
        eve,
        "You can't use that."
    ));
    assert!(!saw(&step(eve, "say hi"), "EVIL-HEAR"));
    assert!(!saw(&step(gadget, "@script/timer boom"), "EVIL-TIMER"));
    assert!(!saw(&step(eve, "north"), "EVIL-ENTER"));

    // 3. Defense-in-depth on role demotion: removing BUILDER from an owner
    //    immediately prevents already-compiled scripts from running.
    let mut f2 = Fixture::new();
    let nick = f2.builder("Nick");
    let bell = f2.thing("bell", nick, LIMBO);
    f2.seed_script(
        bell,
        "script:command:ping",
        r#"mush::pemit(mush::caller(), "pong")"#,
    );
    assert!(actor_saw(&f2.run(nick, "ping"), nick, "pong"));

    f2.engine
        .world
        .set_flag(nick, Flags::BUILDER, false)
        .unwrap();
    let out = f2.run(nick, "ping");
    assert!(
        saw(&out, "Huh?") && !saw(&out, "pong"),
        "demoted player's compiled script must no longer run: {:?}",
        all_text(&out)
    );
}

#[test]
fn crit5_hardened_engine_disables_dangerous_symbols_packages_sleep_and_print() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let obj = f.thing("obj", nick, LIMBO);

    // Disabled symbols must fail at compile time.
    for (idx, src) in [
        r#"eval("1")"#,
        r#"import "x" as x;"#,
        r#"export const X = 1;"#,
        r#"Fn("f")"#,
        r#"let f = 1; f.call()"#,
        r#"let f = 1; f.curry(1)"#,
        r#"fn f() { this } [].f()"#,
    ]
    .into_iter()
    .enumerate()
    {
        let out = f.run_classified(GOD, &format!("@set {obj}/script:command:c{idx}={src}"));
        assert!(
            classified_actor_saw(&out, DeliveryClass::OnCommit, nick, "failed to compile"),
            "expected compile failure for `{src}`: {out:?}"
        );
    }

    // Excluded package functions must fail at runtime.
    for (idx, src) in [
        r#"let s = "a".replace("a", "b");"#,
        r#"let s = "a".pad(10, "");"#,
        r#"let t = timestamp();"#,
        r#"let b = blob(10);"#,
        r#"let f = || 1; let n = f.name();"#,
        r#"let x = sin(1.0);"#,
    ]
    .into_iter()
    .enumerate()
    {
        let word = format!("pkg{idx}");
        f.seed_script(obj, &format!("script:command:{word}"), src);
        let out = f.run(nick, &word);
        assert!(
            actor_saw(&out, nick, "failed"),
            "excluded package call `{src}` must fail: {:?}",
            all_text(&out)
        );
    }

    // `sleep` (INT & FLOAT) must fail immediately and fatally, even when caught
    // through a callback or masked by a subsequent throw.
    for (idx, src) in [
        "sleep(10);",
        "sleep(10.0);",
        "try { sleep(10); } catch {}",
        "try { [1].map(|x| sleep(10)); } catch {}",
        r#"try { [1].map(|x| sleep(10.0)); } catch {} throw "mask";"#,
    ]
    .into_iter()
    .enumerate()
    {
        let word = format!("slp{idx}");
        f.seed_script(obj, &format!("script:command:{word}"), src);
        let started = std::time::Instant::now();
        let out = f.run(nick, &word);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "sleep must not actually sleep: {src}"
        );
        assert!(
            actor_saw(&out, nick, "sleep is disabled in mush-script"),
            "sleep must report fatal refusal for `{src}`: {:?}",
            all_text(&out)
        );
    }

    // `print` and `debug` are silent no-ops (succeed without panicking).
    f.seed_script(
        obj,
        "script:command:quiet",
        r#"print("hello"); debug("world"); mush::pemit(mush::caller(), "clean")"#,
    );
    let out = f.run(nick, "quiet");
    assert!(actor_saw(&out, nick, "clean"), "{:?}", all_text(&out));
}

#[test]
fn crit8_monotonic_budget_and_callback_overruns_accrue_strikes_and_halt() {
    // 1. Sibling callbacks cannot reset the op counter; 3 consecutive runs HALT.
    {
        let cfg = ScriptConfig {
            max_operations: 500,
            ..ScriptConfig::default()
        };
        let mut f = Fixture::with_config(cfg);
        let nick = f.builder("Nick");
        let bomb = f.thing("bomb", nick, LIMBO);
        f.seed_script(
            bomb,
            "script:command:fan",
            "fn burn(x) { let i = 0; while i < 50 { i += 1; } x } \
             let v = []; v.pad(2000, 0); v.map(burn); \
             mush::pemit(mush::caller(), \"ESCAPED\");",
        );
        for attempt in 0..3 {
            let started = std::time::Instant::now();
            let out = f.run(nick, "fan");
            assert!(started.elapsed() < std::time::Duration::from_millis(500));
            assert!(!saw(&out, "ESCAPED"));
            assert!(
                actor_saw(&out, nick, "failed"),
                "attempt {attempt}: {:?}",
                all_text(&out)
            );
        }
        assert!(
            f.flag(bomb, Flags::HALTED),
            "3 callback op overruns must HALT"
        );
    }

    // 2. Callback overrun caught in `try`/`catch` still fails, still accrues
    //    strikes, and HALTs after 3 runs.
    {
        let cfg = ScriptConfig {
            max_operations: 500,
            ..ScriptConfig::default()
        };
        let mut f = Fixture::with_config(cfg);
        let nick = f.builder("Nick");
        let bomb = f.thing("bomb", nick, LIMBO);
        f.seed_script(
            bomb,
            "script:command:sneak",
            "fn spin(x) { let n = 0; loop { n += 1; } } \
             try { [1].map(spin); } catch {} \
             mush::pemit(mush::caller(), \"ESCAPED\");",
        );
        for _ in 0..3 {
            let out = f.run(nick, "sneak");
            assert!(actor_saw(&out, nick, "failed"), "{:?}", all_text(&out));
        }
        assert!(
            f.flag(bomb, Flags::HALTED),
            "caught callback overrun must still accrue strikes and HALT"
        );
    }

    // 3. Uncaught callback overrun (`ErrorInFunctionCall` wrapper) unwraps in
    //    `is_budget_overrun`, accrues strikes, and HALTs after 3 runs.
    {
        let cfg = ScriptConfig {
            max_operations: 500,
            ..ScriptConfig::default()
        };
        let mut f = Fixture::with_config(cfg);
        let nick = f.builder("Nick");
        let bomb = f.thing("bomb", nick, LIMBO);
        f.seed_script(
            bomb,
            "script:command:wrap",
            "fn spin(x) { let n = 0; loop { n += 1; } } [1].map(spin);",
        );
        for _ in 0..3 {
            let out = f.run(nick, "wrap");
            assert!(actor_saw(&out, nick, "failed"), "{:?}", all_text(&out));
        }
        assert!(
            f.flag(bomb, Flags::HALTED),
            "uncaught wrapped callback overrun must accrue strikes and HALT"
        );
    }

    // 4. Wall-clock backstop triggers inside callbacks too and accrues strikes.
    {
        let cfg = ScriptConfig {
            wall_clock_ms: 10,
            max_operations: 5_000_000_000,
            ..ScriptConfig::default()
        };
        let mut f = Fixture::with_config(cfg);
        let nick = f.builder("Nick");
        let bomb = f.thing("bomb", nick, LIMBO);
        f.seed_script(
            bomb,
            "script:command:slowcb",
            "fn burn(x) { let i = 0; while i < 50000 { i += 1; } x } \
             let v = []; v.pad(2000, 0); v.map(burn);",
        );
        for _ in 0..3 {
            let started = std::time::Instant::now();
            let out = f.run(nick, "slowcb");
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
            assert!(actor_saw(&out, nick, "failed"), "{:?}", all_text(&out));
        }
        assert!(
            f.flag(bomb, Flags::HALTED),
            "wall-clock overruns in callbacks must accrue strikes and HALT"
        );
    }
}

#[test]
fn crit6_array_pad_hardening_in_mush_script() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let obj = f.thing("obj", nick, LIMBO);

    // Non-positive lengths are no-ops; valid pad works.
    f.seed_script(
        obj,
        "script:command:okpad",
        "let v = [1]; v.pad(0, 9); v.pad(-5, 9); v.pad(4, 7); \
         mush::pemit(mush::caller(), v.to_string());",
    );
    let out = f.run(nick, "okpad");
    assert!(
        actor_saw(&out, nick, "[1, 7, 7, 7]"),
        "{:?}",
        all_text(&out)
    );

    // Oversized pad, exact release-mode u64 wraparound payload
    // ((4095 + 1) * (1 << 52) == 2^64 == 0 mod 2^64), and nested size overflow
    // all fail cleanly without OOM/abort and accrue strikes.
    let bad_cases = [
        "let v = []; v.pad(4097, 0);",
        "let item = []; item.pad(4095, 0); let v = []; v.pad(4503599627370496, item);",
        "let item = []; item.pad(50, 0); let v = []; v.pad(100, item);",
    ];
    for (idx, src) in bad_cases.into_iter().enumerate() {
        let bomb = f.thing(&format!("padbomb{idx}"), nick, LIMBO);
        let word = format!("badpad{idx}");
        f.seed_script(bomb, &format!("script:command:{word}"), src);
        for _ in 0..3 {
            let started = std::time::Instant::now();
            let out = f.run(nick, &word);
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
            assert!(
                actor_saw(&out, nick, "failed"),
                "bad pad `{src}` must fail cleanly: {:?}",
                all_text(&out)
            );
        }
        assert!(
            f.flag(bomb, Flags::HALTED),
            "bad pad `{src}` must accrue strikes"
        );
    }
}

#[test]
fn crit7_container_depth_cap_in_mush_script_on_small_stack() {
    std::thread::Builder::new()
        .name("script-crit7-depth".to_owned())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut f = Fixture::new();
            let nick = f.builder("Nick");
            let ok_obj = f.thing("ok", nick, LIMBO);

            // Depth-8 containers work cleanly for equality, formatting, and to_json.
            f.seed_script(
                ok_obj,
                "script:command:okdepth",
                "let a = [1]; for i in 0..7 { a = [a]; } \
                 let b = [1]; for i in 0..7 { b = [b]; } \
                 let m = #{a: 1}; for i in 0..7 { m = #{a: m}; } \
                 let s = a.to_string() + m.to_debug() + m.to_json(); \
                 print(a); debug(m); \
                 if a == b && s.len() > 0 { mush::pemit(mush::caller(), \"DEPTH8-OK\"); }",
            );
            let out = f.run(nick, "okdepth");
            assert!(actor_saw(&out, nick, "DEPTH8-OK"), "{:?}", all_text(&out));

            // Depth > 8 on every mutating, building, formatting, and equality path
            // fails cleanly without overflowing a 2 MiB stack.
            let deep_cases = [
                "let x = [1]; for i in 0..20 { x = [x]; }",
                "let m = #{k: 1}; for i in 0..20 { m = #{k: m}; }",
                "let x = [1]; for i in 0..20 { let y = []; y.push(x); x = y; }",
                "let x = [1]; for i in 0..20 { let y = []; y.insert(0, x); x = y; }",
                "let x = [1]; for i in 0..20 { let y = [0]; y.set(0, x); x = y; }",
                "let x = [1]; for i in 0..7 { x = [x]; } let y = []; y.pad(1, x);",
                "let x = [1]; for i in 0..20 { let y = []; y += [x]; x = y; }",
                "let x = [1]; for i in 0..20 { let y = []; y += #{k: x}; x = y; }",
                "let x = [1]; for i in 0..7 { x = [x]; } let y = []; y += [x];",
                "let x = [1]; for i in 0..7 { x = [x]; } let y = []; y.append([x]);",
                "let x = [1]; for i in 0..7 { x = [x]; } let y = [] + [x];",
                "let x = [1]; for i in 0..7 { x = [x]; } let y = [0]; y.splice(0, 1, [x]);",
                "let x = [1]; for i in 0..20 { let y = [0]; y[0] = x; x = y; }",
                "let m = #{k: 1}; for i in 0..20 { let n = #{}; n.k = m; m = n; }",
                "let m = #{k: 1}; for i in 0..20 { let n = #{}; n.set(\"k\", m); m = n; }",
                "let m = #{k: 1}; for i in 0..7 { m = #{k: m}; } let n = #{}; n.mixin(#{k: m});",
                "let m = #{k: 1}; for i in 0..7 { m = #{k: m}; } let n = #{} + #{k: m};",
                "let m = #{k: 1}; for i in 0..7 { m = #{k: m}; } let n = #{}; n.fill_with(#{k: m});",
                "let x = [1]; for i in 0..7 { x = [x]; } let eq = [x] == [x];",
                "let x = [1]; for i in 0..7 { x = [x]; } let ne = [x] != [x];",
                "let m = #{k: 1}; for i in 0..7 { m = #{k: m}; } let eq = #{k: m} == #{};",
                "let x = [1]; for i in 0..7 { x = [x]; } print([x]);",
                "let x = [1]; for i in 0..7 { x = [x]; } debug([x]);",
                "let x = [1]; for i in 0..7 { x = [x]; } let s = [x].to_string();",
                "let x = [1]; for i in 0..7 { x = [x]; } let s = [x].to_debug();",
                "let x = [1]; for i in 0..7 { x = [x]; } let s = `${[x]}`;",
                "let x = [1]; for i in 0..7 { x = [x]; } try { let s = `${[x]}`; } catch {} mush::pemit(mush::caller(), \"ESCAPED\");",
                "let m = #{k: 1}; for i in 0..7 { m = #{k: m}; } let j = (#{k: m}).to_json();",
                "fn wrap(x) { [x] } let v = [1]; for i in 0..20 { v = v.map(wrap); }",
                "fn acc(r, x) { [r] } let v = []; v.pad(50, 0); v.reduce(acc, 1);",
            ];
            for (idx, src) in deep_cases.into_iter().enumerate() {
                let obj = f.thing(&format!("deep{idx}"), nick, LIMBO);
                let word = format!("d{idx}");
                f.seed_script(obj, &format!("script:command:{word}"), src);
                let out = f.run(nick, &word);
                assert!(!saw(&out, "ESCAPED"), "escaped on `{src}`");
                assert!(
                    actor_saw(&out, nick, "failed"),
                    "expected depth failure for `{src}`: {:?}",
                    all_text(&out)
                );
            }

            // Static container-literal nesting > 8 (including the recursive
            // 50-bracket `f(30)` payload that would otherwise build 1,500+
            // nested arrays bypassing `on_var`) is rejected at compile time.
            let compile_obj = f.thing("staticdeep", nick, LIMBO);
            let deep50 = format!(
                "fn f(n) {{ if n == 0 {{ 1 }} else {{ {}f(n - 1){} }} }} f(30);",
                "[".repeat(50),
                "]".repeat(50)
            );
            for (idx, src) in [
                deep50.as_str(),
                "let x = [[[[[[[[[1]]]]]]]]];",
                "let m = #{a:#{a:#{a:#{a:#{a:#{a:#{a:#{a:#{a:1}}}}}}}}};",
            ]
            .into_iter()
            .enumerate()
            {
                let out = f.run_classified(
                    GOD,
                    &format!("@set {compile_obj}/script:command:sd{idx}={src}"),
                );
                assert!(
                    classified_actor_saw(&out, DeliveryClass::OnCommit, nick, "failed to compile"),
                    "expected compile failure for deep static literal `{src}`: {out:?}"
                );
            }

            // At-cap 8-bracket recursive `f(30)` compiles, runs safely on a
            // 2 MiB stack, and trips depth enforcement on variable read.
            let rec_obj = f.thing("rec8", nick, LIMBO);
            f.seed_script(
                rec_obj,
                "script:command:rec8",
                "fn f(n) { if n == 0 { 1 } else { [[[[[[[[f(n - 1)]]]]]]]] } } \
                 let a = f(30); let s = a.to_string();",
            );
            let out = f.run(nick, "rec8");
            assert!(
                actor_saw(&out, nick, "failed"),
                "recursive depth-8 build must fail safely on read: {:?}",
                all_text(&out)
            );

            // 3 consecutive depth-cap violations HALT the object.
            let bomb = f.thing("depthbomb", nick, LIMBO);
            f.seed_script(
                bomb,
                "script:command:nestbomb",
                "let x = [1]; for i in 0..20 { x = [x]; }",
            );
            for _ in 0..3 {
                let out = f.run(nick, "nestbomb");
                assert!(actor_saw(&out, nick, "failed"));
            }
            assert!(f.flag(bomb, Flags::HALTED));
        })
        .expect("spawn")
        .join()
        .expect("script crit7 thread must not overflow stack or panic");
}

#[test]
fn high16_parse_json_and_fallible_sort_dedup_in_mush_script() {
    // 1. `parse_json` is fatally refused and cannot be caught.
    {
        let mut f = Fixture::new();
        let nick = f.builder("Nick");
        let obj = f.thing("obj", nick, LIMBO);
        for (idx, src) in [
            r#"parse_json("{}")"#,
            r#"parse_json("1 + 2")"#,
            r#"try { parse_json("{}"); } catch {}"#,
            r#"try { [1].map(|x| parse_json("{}")); } catch {}"#,
        ]
        .into_iter()
        .enumerate()
        {
            let word = format!("pj{idx}");
            f.seed_script(obj, &format!("script:command:{word}"), src);
            let out = f.run(nick, &word);
            assert!(
                actor_saw(&out, nick, "parse_json is disabled in mush-script"),
                "`{src}` must fatally fail: {:?}",
                all_text(&out)
            );
        }

        // 2. Valid sort / order / dedup correctness + non-transitive safety + error propagation.
        f.seed_script(
            obj,
            "script:command:sortok",
            r#"fn desc(a, b) { b - a }
               fn same_mod(a, b) { (a % 2) == (b % 2) }
               fn crazy(a, b) { -1 }
               let v1 = [3, 1, 4, 1, 5]; v1.sort(desc);
               let v2 = [3, 1, 4]; v2.sort_by(desc);
               let v3 = [2, 9, 5]; v3.sort("desc");
               let orig = [2, 7, 4];
               let o1 = orig.order(desc);
               let o2 = orig.order_by(desc);
               let d0 = [1, 1, 2, 2, 3]; d0.dedup();
               let d1 = [2, 4, 3, 5, 6]; d1.dedup(same_mod);
               let d2 = [2, 4, 3, 5, 6]; d2.dedup("same_mod");
               let nt = [1, 2, 3, 4, 5, 6]; nt.sort(crazy);
               mush::pemit(
                   mush::caller(),
                   v1.to_string() + "|" + v2.to_string() + "|" + v3.to_string() + "|"
                   + orig.to_string() + "|" + o1.to_string() + "|" + o2.to_string() + "|"
                   + d0.to_string() + "|" + d1.to_string() + "|" + d2.to_string() + "|"
                   + nt.len().to_string()
               );"#,
        );
        let out = f.run(nick, "sortok");
        assert!(
            actor_saw(
                &out,
                nick,
                "[5, 4, 3, 1, 1]|[4, 3, 1]|[9, 5, 2]|[2, 7, 4]|[7, 4, 2]|[7, 4, 2]|[1, 2, 3]|[2, 3, 6]|[2, 3, 6]|6"
            ),
            "{:?}",
            all_text(&out)
        );

        f.seed_script(
            obj,
            "script:command:sorterr",
            r#"fn bad(a, b) { throw "boom"; } let v = [2, 1]; v.sort(bad);"#,
        );
        assert!(actor_saw(&f.run(nick, "sorterr"), nick, "boom"));
    }

    // 3. Budget overrun inside sort/order/dedup aborts immediately and accrues strikes.
    let cfg = ScriptConfig {
        max_operations: 500,
        ..ScriptConfig::default()
    };
    let mut f = Fixture::with_config(cfg);
    let nick = f.builder("Nick");
    let cases = [
        r#"fn cmp(a, b) { a - b } let v = []; v.pad(2000, 1); v.sort(cmp);"#,
        r#"fn cmp(a, b) { a - b } let v = []; v.pad(2000, 1); v.sort_by(cmp);"#,
        r#"fn cmp(a, b) { a - b } let v = []; v.pad(2000, 1); v.sort("cmp");"#,
        r#"fn cmp(a, b) { a - b } let v = []; v.pad(2000, 1); let r = v.order(cmp);"#,
        r#"fn cmp(a, b) { a - b } let v = []; v.pad(2000, 1); let r = v.order_by(cmp);"#,
        r#"fn eq(a, b) { a == b } let v = []; v.pad(2000, 1); v.dedup(eq);"#,
        r#"fn eq(a, b) { a == b } let v = []; v.pad(2000, 1); v.dedup("eq");"#,
    ];
    for (idx, src) in cases.into_iter().enumerate() {
        let bomb = f.thing(&format!("sortbomb{idx}"), nick, LIMBO);
        let word = format!("sb{idx}");
        f.seed_script(bomb, &format!("script:command:{word}"), src);
        for _ in 0..3 {
            let started = std::time::Instant::now();
            let out = f.run(nick, &word);
            assert!(
                started.elapsed() < std::time::Duration::from_millis(500),
                "`{src}` did not abort promptly"
            );
            assert!(
                actor_saw(&out, nick, "failed"),
                "`{src}` must fail on budget overrun: {:?}",
                all_text(&out)
            );
        }
        assert!(
            f.flag(bomb, Flags::HALTED),
            "`{src}` must accrue strikes and HALT after 3 overruns"
        );
    }
}

// ----------------------------------------------------------------------
// Tier 1C security regressions: HIGH-17 & HIGH-18
// ----------------------------------------------------------------------

#[test]
fn high17_get_attr_enforces_visibility_locality_and_per_flag_rules() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bob = f.player("Bob");
    let vault_room = f.room("Vault");
    let distant = f.thing("distant", bob, vault_room);
    let dark_local = f.thing("darkthing", bob, LIMBO);
    f.engine
        .world
        .set_flag(dark_local, Flags::DARK, true)
        .unwrap();
    let own_box = f.thing("ownbox", nick, LIMBO);
    let bob_box = f.thing("bobbox", bob, LIMBO);

    // Distant plain attr & local DARK plain attr.
    f.engine
        .world
        .set_attr(distant, "note", Attr::text("far-secret", bob))
        .unwrap();
    f.engine
        .world
        .set_attr(dark_local, "note", Attr::text("dark-secret", bob))
        .unwrap();

    // WIZARD_ONLY and SELF_ONLY on Nick's OWN object (Nick is Builder, not Wizard).
    f.engine
        .world
        .set_attr(
            own_box,
            "staffnote",
            Attr {
                value: AttrValue::Text("wiz-secret".into()),
                owner: GOD,
                flags: AttrFlags::WIZARD_ONLY,
            },
        )
        .unwrap();
    f.engine
        .world
        .set_attr(
            own_box,
            "selfnote",
            Attr {
                value: AttrValue::Text("self-secret".into()),
                owner: GOD,
                flags: AttrFlags::SELF_ONLY,
            },
        )
        .unwrap();
    // SELF_ONLY on Nick's own player object (owner == obj): readable.
    f.engine
        .world
        .set_attr(
            nick,
            "myshelf",
            Attr {
                value: AttrValue::Text("my-own".into()),
                owner: GOD,
                flags: AttrFlags::SELF_ONLY,
            },
        )
        .unwrap();
    // Module-owned namespace (`script:*`) on uncontrolled local object.
    f.engine
        .world
        .set_attr(bob_box, "script:use", Attr::text("source-code", GOD))
        .unwrap();

    let tool = f.thing("tool", nick, LIMBO);
    // Test WIZARD_ONLY + SELF_ONLY + module namespace + local/distant visibility.
    f.seed_script(
        tool,
        "script:command:probe",
        r#"let own = mush::matchit("ownbox");
           let bbx = mush::matchit("bobbox");
           mush::pemit(
               mush::caller(),
               "w=[" + mush::get_attr(own, "staffnote") + "] "
               + "so=[" + mush::get_attr(own, "selfnote") + "] "
               + "sm=[" + mush::get_attr(mush::owner(), "myshelf") + "] "
               + "sc=[" + mush::get_attr(bbx, "script:use") + "]"
           );"#,
    );
    let out = f.run(nick, "probe");
    assert!(
        actor_saw(&out, nick, "w=[] so=[] sm=[my-own] sc=[]"),
        "WIZARD_ONLY, SELF_ONLY on non-self, and uncontrolled script:* must be withheld: {:?}",
        all_text(&out)
    );

    // Distant and DARK objects cannot even be resolved via matchit, and even if
    // handed their ObjId via a room/caller probe (e.g. a DARK player triggering
    // hear), get_attr withholds their attributes.
    f.engine
        .world
        .set_attr(bob, "note", Attr::text("bob-secret", bob))
        .unwrap();
    f.engine.world.set_flag(bob, Flags::DARK, true).unwrap();
    f.seed_script(
        tool,
        "script:hear:spy",
        r#"mush::pemit(mush::owner(), "spied=[" + mush::get_attr(speaker, "note") + "] name=[" + mush::name(speaker) + "]");"#,
    );
    let out = f.run(bob, "say hello");
    assert!(
        actor_saw(&out, nick, "spied=[] name=[nothing]"),
        "DARK speaker's attr and name must not leak: {:?}",
        all_text(&out)
    );

    // Wizard script CAN read WIZARD_ONLY and distant attrs.
    let wiz_tool = f.thing("wiztool", GOD, LIMBO);
    f.seed_script(
        wiz_tool,
        "script:command:wprobe",
        &format!(
            r##"let d = mush::matchit("#{0}");
               let o = mush::matchit("#{1}");
               mush::pemit(mush::caller(), mush::get_attr(d, "note") + "|" + mush::get_attr(o, "staffnote"));"##,
            distant.0, own_box.0
        ),
    );
    let out = f.run(GOD, "wprobe");
    assert!(
        actor_saw(&out, GOD, "far-secret|wiz-secret"),
        "Wizard scripts keep full access: {:?}",
        all_text(&out)
    );
}

#[test]
fn high17_matchit_and_name_prevent_global_and_local_dark_oracles() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bob = f.player("Bob");
    let far_room = f.room("Far");
    let far_item = f.thing("faritem", GOD, far_room);
    f.engine.world.move_to(bob, far_room).unwrap();

    // Local DARK object sharing prefix with a visible object: must NOT leak via Ambiguous.
    let ghost = f.thing("ghost", GOD, LIMBO);
    let ghoul = f.thing("ghoul", GOD, LIMBO);
    f.engine.world.set_flag(ghost, Flags::DARK, true).unwrap();

    let tool = f.thing("tool", nick, LIMBO);
    f.seed_script(
        tool,
        "script:command:mtest",
        &format!(
            r##"let out = "";
               for q in ["#{0}", "#{1}", "*Bob", "ghost", "#{2}"] {{
                   try {{
                       let hit = mush::matchit(q);
                       out += q + "=HIT;";
                   }} catch {{
                       out += q + "=NONE;";
                   }}
               }}
               let g = mush::matchit("gh");
               out += "gh=" + mush::name(g);
               mush::pemit(mush::caller(), out);"##,
            far_room.0, far_item.0, ghost.0
        ),
    );
    let out = f.run(nick, "mtest");
    let expected = format!(
        "#{0}=NONE;#{1}=NONE;*Bob=NONE;ghost=NONE;#{2}=NONE;gh=ghoul",
        far_room.0, far_item.0, ghost.0
    );
    assert!(
        actor_saw(&out, nick, &expected),
        "expected `{expected}`, got {:?}",
        all_text(&out)
    );
    let _ = ghoul;
}

#[test]
fn high17_emit_and_pemit_enforce_locality_locks_haven_and_ignore() {
    let mut f = Fixture::new();
    let nick = f.builder("Nick");
    let bob = f.player("Bob");
    let far_room = f.room("Far");
    let own_room = f
        .engine
        .world
        .create(ObjKind::Room, "NickRoom", nick, None)
        .unwrap();
    let tool = f.thing("tool", nick, LIMBO);

    // 1. `mush::emit` to a distant uncontrolled room is refused; controlled room succeeds.
    f.seed_script(
        tool,
        "script:command:efar",
        &format!(
            r##"mush::emit(mush::matchit("#{}"), "sneak");"##,
            far_room.0
        ),
    );
    let out = f.run(nick, "efar");
    assert!(
        !saw(&out, "sneak"),
        "must not emit into uncontrolled far room"
    );

    f.seed_script(
        tool,
        "script:command:eown",
        &format!(
            r##"mush::emit(mush::matchit("#{}"), "own-room-ok");"##,
            own_room.0
        ),
    );
    let out = f.run(nick, "eown");
    assert!(
        saw(&out, "own-room-ok"),
        "builder may emit into a room they control: {:?}",
        all_text(&out)
    );

    // 2. `mush::emit` honors room `@lock/speech` for non-Wizards.
    f.run(GOD, "@lock/speech here=#1");
    f.seed_script(
        tool,
        "script:command:ehere",
        r#"mush::emit(mush::here(), "blocked-speech");"#,
    );
    let out = f.run(nick, "ehere");
    assert!(
        !saw(&out, "blocked-speech") && actor_saw(&out, nick, "can't speak here"),
        "room @lock/speech must block non-Wizard emit: {:?}",
        all_text(&out)
    );
    f.run(GOD, "@unlock/speech here");

    // 3. `mush::pemit` honors `@ignore` silently (zero output to Bob, no error to Nick).
    f.seed_script(
        tool,
        "script:command:poke",
        r#"mush::pemit(mush::matchit("Bob"), "ping-bob");"#,
    );
    // Before ignore: Bob hears it.
    let out = f.run(nick, "poke");
    assert!(actor_saw(&out, bob, "ping-bob"), "{:?}", all_text(&out));

    // Bob ignores Nick: Bob receives NOTHING, and Nick sees NO script error (silent gag).
    f.run(bob, "@ignore Nick");
    let out = f.run(nick, "poke");
    assert!(
        !actor_saw(&out, bob, "ping-bob"),
        "ignored player must not receive script pemit: {:?}",
        all_text(&out)
    );
    assert!(
        !actor_saw(&out, nick, "failed"),
        "@ignore must be a silent drop with no error oracle: {:?}",
        all_text(&out)
    );
    f.run(bob, "@ignore/clear Nick");

    // 4. `mush::pemit` honors `HAVEN` and `@lock/page`.
    f.engine.world.set_flag(bob, Flags::HAVEN, true).unwrap();
    let out = f.run(nick, "poke");
    assert!(
        !actor_saw(&out, bob, "ping-bob") && actor_saw(&out, nick, "failed"),
        "HAVEN must refuse non-Wizard pemit: {:?}",
        all_text(&out)
    );
    f.engine.world.set_flag(bob, Flags::HAVEN, false).unwrap();

    f.run(bob, "@lock/page me=#1");
    let out = f.run(nick, "poke");
    assert!(
        !actor_saw(&out, bob, "ping-bob") && actor_saw(&out, nick, "failed"),
        "@lock/page must refuse non-Wizard pemit: {:?}",
        all_text(&out)
    );
    f.run(bob, "@unlock/page me");

    // 5. `mush::pemit` refuses distant uncontrolled non-caller players.
    f.engine.world.move_to(bob, far_room).unwrap();
    let out = f.run(nick, "poke");
    assert!(
        !actor_saw(&out, bob, "ping-bob") && actor_saw(&out, nick, "failed"),
        "distant non-caller must not be pemitted: {:?}",
        all_text(&out)
    );
}

#[test]
fn high18_caps_script_attrs_coalesces_compiles_bounds_outbox_queue_and_min_timer() {
    // 1. Coalesced compilation + per-object `script:*` count cap.
    {
        let cfg = ScriptConfig {
            max_scripts_per_object: 12,
            ..ScriptConfig::default()
        };
        let mut f = Fixture::with_config(cfg);
        let nick = f.builder("Nick");
        let obj = f.thing("obj", nick, LIMBO);
        f.seed_script(
            obj,
            "script:command:batch",
            r#"for i in 0..10 {
                   mush::set_attr(mush::me(), "script:command:c" + i.to_string(), "let x = " + i.to_string() + ";");
               }
               for j in 0..20 {
                   mush::set_attr(mush::me(), "script:command:hot", "mush::pemit(mush::caller(), \"v" + j.to_string() + "\");");
               }"#,
        );
        let before = f
            .engine
            .world
            .module_state::<mush_script::ScriptState>()
            .unwrap()
            .compile_count();
        f.run(nick, "batch");
        let after = f
            .engine
            .world
            .module_state::<mush_script::ScriptState>()
            .unwrap()
            .compile_count();
        // 10 distinct `c0..c9` + 1 coalesced `hot` = 11 compiles (NOT O(K²) = 30+ or 200+).
        assert_eq!(
            after - before,
            11,
            "batch set_attr edits must coalesce into one compile per modified key"
        );
        // The last version of `hot` is live immediately.
        assert!(actor_saw(&f.run(nick, "hot"), nick, "v19"));

        // Object now has 12 script:* attrs (batch + c0..c9 + hot); a 13th must fail.
        f.seed_script(
            obj,
            "script:command:hot",
            r#"mush::set_attr(mush::me(), "script:command:overflow", "1");"#,
        );
        let out = f.run(nick, "hot");
        assert!(
            actor_saw(&out, nick, "Too many script attributes"),
            "per-object script:* cap must be enforced: {:?}",
            all_text(&out)
        );
    }

    // 2. Output cap (`sess.out`) aborts runaway emitters and accrues strikes -> HALTED.
    {
        let cfg = ScriptConfig {
            max_outbox_events: 8,
            ..ScriptConfig::default()
        };
        let mut f = Fixture::with_config(cfg);
        let nick = f.builder("Nick");
        let spammer = f.thing("spammer", nick, LIMBO);
        f.seed_script(
            spammer,
            "script:command:flood",
            r#"for i in 0..50 { mush::pemit(mush::caller(), "spam"); }"#,
        );
        for _ in 0..3 {
            let out = f.run(nick, "flood");
            let emitted = count(&out, "spam");
            assert_eq!(emitted, 8, "output must stop at max_outbox_events");
            assert!(actor_saw(&out, nick, "script output budget exceeded"));
        }
        assert!(
            f.flag(spammer, Flags::HALTED),
            "3 output budget overruns must HALT the object"
        );
    }

    // 3. `sess.queue` cap prevents unbounded trigger fan-out before run_loop pops.
    {
        let cfg = ScriptConfig {
            max_total_invocations: 10,
            max_queue_len: 10,
            ..ScriptConfig::default()
        };
        let mut f = Fixture::with_config(cfg);
        let nick = f.builder("Nick");
        for i in 0..20 {
            let ear = f.thing(&format!("ear{i}"), nick, LIMBO);
            f.seed_script(ear, "script:hear:x", "let y = 1;");
        }
        let horn = f.thing("horn", nick, LIMBO);
        f.seed_script(
            horn,
            "script:command:blast",
            r#"for i in 0..5 { mush::emit(mush::here(), "honk"); }"#,
        );
        let out = f.run(nick, "blast");
        assert_eq!(count(&out, "honk"), 5);
    }

    // 4. Minimum timer delay >= 1s (0s self-rearming refused) & per-cascade schedule cap.
    {
        let mut f = Fixture::new();
        let nick = f.builder("Nick");
        let clk = f.thing("clk", nick, LIMBO);
        f.seed_script(clk, "script:command:zero", r#"mush::schedule(0, "spin");"#);
        let out = f.run(nick, "zero");
        assert!(
            actor_saw(&out, nick, "at least 1 second"),
            "0s script timer must be refused: {:?}",
            all_text(&out)
        );
        assert!(
            f.engine.sched.next_due().is_none(),
            "refused 0s timer must not enter the scheduler"
        );

        f.seed_script(
            clk,
            "script:command:schedflood",
            r#"for i in 0..50 { mush::schedule(5, "t"); }"#,
        );
        for _ in 0..3 {
            let out = f.run(nick, "schedflood");
            assert!(actor_saw(&out, nick, "script schedule budget exceeded"));
        }
        assert!(
            f.flag(clk, Flags::HALTED),
            "3 schedule overruns must HALT the object"
        );
    }
}
