//! Doc 43 cut TS.4: nested source against the builder engine's depth guard.
//!
//! A `script:*` attribute is compiled on the world task, at boot and on every
//! `@set`. A stack overflow there is not a panic `guard()` can catch: it
//! aborts the server. `ScriptConfig::max_expr_depth` (64) is the guard; these
//! tests feed every nesting shape the grammar has (the same list as
//! mush-mind's `tests/sandbox/nesting.rs`), at the guard's edge and far past
//! it, through the real boot compile and dispatch, on a thread with the
//! default 2 MiB stack the world task runs on, and require a compile error or
//! a clean run, never an abort.

use std::collections::HashMap;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::{
    Attr, GOD, LIMBO, ObjId, ObjKind, Outbox, Registry, World, register_core_commands,
    register_engine_commands,
};
use mush_script::{ScriptConfig, ScriptModule};

use super::{actor_saw, all_text};

const MAX_EXPR_DEPTH: usize = 64;

/// An attribute's worth of source: far past every guard.
const SOURCE_CAP: usize = 32 * 1024;

/// A world-task thread's stack: the platform default (nothing in the tree
/// sets a larger one).
const TASK_STACK: usize = 2 * 1024 * 1024;

fn rep(s: &str, n: usize) -> String {
    s.repeat(n)
}

/// A shape: the expression nested `n` deep.
type Nest = fn(usize) -> String;

/// Every way the grammar nests an expression or a block.
const SHAPES: &[(&str, Nest)] = &[
    ("parens", |n| format!("{}1{}", rep("(", n), rep(")", n))),
    ("unary minus", |n| format!("{}1", rep("- ", n))),
    ("unary not", |n| format!("{}true", rep("!", n))),
    ("arrays", |n| format!("{}{}", rep("[", n), rep("]", n))),
    ("maps", |n| format!("{}1{}", rep("#{a:", n), rep("}", n))),
    ("blocks", |n| format!("{}1{}", rep("{", n), rep("}", n))),
    ("if", |n| {
        format!("{}1{}", rep("if true { ", n), rep(" }", n))
    }),
    ("while", |n| {
        format!("{}1{}", rep("while false { ", n), rep(" }", n))
    }),
    ("calls", |n| format!("{}1{}", rep("abs(", n), rep(")", n))),
    ("interpolation", |n| {
        format!("{}1{}", rep("`${", n), rep("}`", n))
    }),
    ("closures", |n| format!("{}1", rep("|| ", n))),
    ("flat sum", |n| format!("1{}", rep(" + 1", n))),
    ("flat and", |n| format!("true{}", rep(" && true", n))),
    ("index chain", |n| format!("[[1]]{}", rep("[0]", n))),
    ("method chain", |n| format!("1{}", rep(".abs()", n))),
];

fn depths(unit: usize) -> Vec<usize> {
    let at_cap = SOURCE_CAP / unit.max(1);
    let mut out = vec![
        1,
        MAX_EXPR_DEPTH - 1,
        MAX_EXPR_DEPTH,
        MAX_EXPR_DEPTH + 1,
        256,
        2_048,
        at_cap,
    ];
    out.retain(|&d| d <= at_cap);
    out
}

/// The script wrapping: at the top level and inside a function (Rhai keeps
/// a separate depth for function bodies).
fn scripts(expr: &str) -> [String; 2] {
    [
        format!("let x = {expr}; mush::pemit(mush::caller(), \"ran\")"),
        format!("fn f() {{ let x = {expr}; }} f(); mush::pemit(mush::caller(), \"ran\")"),
    ]
}

/// Boot an engine whose world already holds `scripts` (one object and one
/// command word each), so `on_world_loaded` compiles them all. Returns the
/// engine, the boot output and the player.
fn boot(scripts: &[(String, String)]) -> (Engine, Vec<String>, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    for (word, code) in scripts {
        let obj = world
            .create(ObjKind::Thing, word, nick, Some(LIMBO))
            .unwrap();
        world
            .set_attr(
                obj,
                &format!("script:command:{word}"),
                Attr::text(code, nick),
            )
            .unwrap();
    }
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![Box::new(ScriptModule::with_config(ScriptConfig::default()))],
    )
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();
    let boot_text = all_text(&out.take());
    (engine, boot_text, nick)
}

fn run(
    engine: &mut Engine,
    actor: ObjId,
    line: &str,
) -> Vec<(mush_core::Audience, mush_proto::OutputEvent)> {
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
}

fn on_task_stack<F: FnOnce() + Send + 'static>(what: &str, f: F) {
    std::thread::Builder::new()
        .name(what.to_owned())
        .stack_size(TASK_STACK)
        .spawn(f)
        .expect("spawn")
        .join()
        .unwrap_or_else(|_| panic!("{what}: panicked"));
}

/// Every shape at every depth, compiled at boot and run: the process
/// surviving is the assertion.
#[test]
fn every_nesting_shape_is_bounded() {
    on_task_stack("nesting", || {
        let mut all = Vec::new();
        for (i, (_, nest)) in SHAPES.iter().enumerate() {
            for depth in depths(nest(1).len()) {
                for (j, code) in scripts(&nest(depth)).into_iter().enumerate() {
                    if code.len() <= SOURCE_CAP + 128 {
                        all.push((format!("n{i}x{depth}x{j}"), code));
                    }
                }
            }
        }
        let (mut engine, _, nick) = boot(&all);
        for (word, _) in &all {
            let _ = run(&mut engine, nick, word);
        }
    });
}

/// Each shape one deep compiles and runs (the shapes are valid Rhai); twice
/// the guard's depth is refused by the guard itself at boot ("exceeds maximum
/// complexity"), and its command is never found.
#[test]
fn the_depth_guard_engages_past_its_limit() {
    on_task_stack("guard", || {
        for (name, nest) in SHAPES {
            for (shallow, deep) in scripts(&nest(1))
                .into_iter()
                .zip(scripts(&nest(MAX_EXPR_DEPTH * 2)))
            {
                let (mut engine, boot_text, nick) =
                    boot(&[("ok".into(), shallow.clone()), ("deep".into(), deep)]);
                assert!(
                    actor_saw(&run(&mut engine, nick, "ok"), nick, "ran"),
                    "{name}: one deep must run: {shallow}"
                );
                assert!(
                    boot_text
                        .iter()
                        .any(|t| t.contains("script:command:deep")
                            && t.contains("maximum complexity")),
                    "{name}: {} deep was not refused by the guard: {boot_text:?}",
                    MAX_EXPR_DEPTH * 2
                );
            }
        }
    });
}

/// Recursion at run time is bounded by `max_call_levels`, not the stack.
#[test]
fn runtime_recursion_is_bounded() {
    on_task_stack("recursion", || {
        let all = [
            ("r1".to_owned(), "fn f(n) { f(n + 1) } f(0)".to_owned()),
            (
                "r2".to_owned(),
                "fn f(n) { g(n) } fn g(n) { f(n) } f(0)".to_owned(),
            ),
        ];
        let (mut engine, _, nick) = boot(&all);
        for (word, _) in &all {
            let out = run(&mut engine, nick, word);
            assert!(
                actor_saw(&out, nick, "failed"),
                "{word}: unbounded recursion ran clean: {:?}",
                all_text(&out)
            );
        }
    });
}
