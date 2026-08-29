//! The M0 stdin REPL, preserved behind `--repl` for offline poking.
//!
//! Drives the same [`Engine`](mush_core::Engine) the server runs — core +
//! engine commands (`@slowtest`, `@wait` included) — synchronously: you are
//! God (`#1`), each line becomes an [`Inbound::Cmd`], and events whose
//! audience reaches God print as plain text. Deferred effects run
//! **synchronously** here (a `@slowtest 3` really sleeps 3 seconds before
//! the answer prints — the REPL is single-threaded on purpose); `@wait`
//! timers fire on the first prompt after they come due.

use std::io::Write as _;

use mush_core::{
    Audience, Effect, Effects, GOD, Inbound, Outbox, StepOutcome, World, WorldCommand,
};
use mush_testkit::text_of;

use crate::world_task::boot_engine;
use crate::{unix_now_ms, unix_now_s};

/// Connection id the REPL pretends God is on.
const REPL_CONN: u64 = 0;

/// Run the REPL until EOF or `quit`.
pub fn run() {
    let mut engine = boot_engine();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::ConnUp {
            conn: REPL_CONN,
            actor: GOD,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    out.take(); // the connect announce has no other audience here

    println!(
        "rustmush {} -- offline REPL (--repl; no networking)",
        env!("CARGO_PKG_VERSION")
    );
    println!("You are God (#1) in Limbo. Type \"help\" for commands, \"quit\" to exit.");
    println!("Deferred effects (@slowtest) run synchronously in this mode.");
    println!();

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    loop {
        print!("mush> ");
        let _ = stdout.flush();

        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) | Err(_) => break, // EOF or broken stdin: shut down.
            Ok(_) => {}
        }
        // Strip a leading BOM: piping a script into the REPL on Windows
        // often prefixes the first line with U+FEFF.
        let trimmed = line.trim().trim_start_matches('\u{feff}').trim();
        if trimmed.eq_ignore_ascii_case("quit") {
            println!("Goodbye.");
            break;
        }

        engine.set_now(unix_now_s());
        // Fire any timers that came due while we sat at the prompt.
        engine.fire_due(unix_now_ms(), &mut out, &mut fx);
        let mut outcome = StepOutcome::Continue;
        if !trimmed.is_empty() {
            outcome = engine.step(
                Inbound::Cmd {
                    conn: REPL_CONN,
                    actor: GOD,
                    line: trimmed.to_string(),
                },
                &mut out,
                &mut fx,
            );
        }
        print_gods_view(&engine.world, &mut out);
        run_effects_sync(&mut engine, &mut fx, &mut out);
        if outcome == StepOutcome::Shutdown {
            break;
        }
    }
}

/// Drain the outbox, printing every event whose audience reaches God: God's
/// own actor stream, broadcasts, and events for God's room (unless
/// excluded). `Conn` audiences other than the REPL's own never deliver.
fn print_gods_view(world: &World, out: &mut Outbox) {
    for (audience, event) in out.take() {
        let reaches = match &audience {
            Audience::Actor(a) => *a == GOD,
            Audience::All => true,
            Audience::Room { room, exclude } => {
                !exclude.contains(&GOD) && world.get(GOD).and_then(|o| o.location) == Some(*room)
            }
            Audience::Conn(c) => *c == REPL_CONN,
        };
        if reaches {
            println!("{}", text_of(&event));
        }
    }
}

/// Run deferred effects to completion, synchronously, re-entering the
/// engine with the typed completion exactly as the world task would.
fn run_effects_sync(engine: &mut mush_core::Engine, fx: &mut Effects, out: &mut Outbox) {
    // Effects can queue further effects; loop until quiescent.
    while !fx.is_empty() {
        for effect in fx.drain() {
            match effect {
                Effect::SlowTest {
                    actor,
                    correlation,
                    seconds,
                } => {
                    println!("(repl: running the fake {seconds}s call inline...)");
                    std::thread::sleep(std::time::Duration::from_secs(seconds));
                    engine.set_now(unix_now_s());
                    engine.step(
                        Inbound::Apply(WorldCommand::SlowTestDone {
                            actor,
                            correlation,
                            text: format!(
                                "SLOWTEST: fake async call #{correlation} finished after \
                                 {seconds}s -- the world loop never blocked."
                            ),
                        }),
                        out,
                        fx,
                    );
                }
                // M7 LLM re-entry seam: the REPL has no `LlmService`, so an
                // `Effect::ModuleAsync` cannot be run inline here. worldgen is
                // not booted in the REPL, so this arm is unreachable in
                // practice; it stays a no-op to keep the match exhaustive.
                Effect::ModuleAsync {
                    module,
                    correlation,
                    ..
                } => {
                    println!(
                        "(repl: Effect::ModuleAsync from {module} #{correlation} dropped -- \
                         no LLM service in the REPL)"
                    );
                }
            }
            print_gods_view(&engine.world, out);
        }
    }
}
