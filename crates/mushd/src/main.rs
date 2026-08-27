//! mushd — the rustmush server binary.
//!
//! M0 ships a **synchronous stdin REPL**: it boots the same world wiring
//! the test suite uses ([`mush_testkit::boot_world`]), reads lines from
//! stdin as God (`#1`), dispatches them through the real command pipeline,
//! and prints the resulting `mush-proto` `OutputEvent`s that would reach
//! God as plain text. Networking (SSH), the session manager,
//! and the tokio world task all land in M1; nothing here is async.

use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use mush_core::{Audience, GOD, Outbox, World, dispatch};
use mush_testkit::{boot_world, text_of};

fn main() {
    let (mut world, registry) = boot_world();
    world.set_connected(GOD, true);
    world.set_now(unix_now());

    println!(
        "rustmush {} -- M0 REPL driver (no networking yet)",
        env!("CARGO_PKG_VERSION")
    );
    println!("You are God (#1) in Limbo. Type \"help\" for commands, \"quit\" to exit.");
    println!();

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    loop {
        print!("mush> ");
        let _ = stdout.flush();

        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) | Err(_) => break, // EOF or a broken stdin: shut down.
            Ok(_) => {}
        }
        // Strip a leading BOM too: piping a script into the REPL on
        // Windows often prefixes the first line with U+FEFF.
        let trimmed = line.trim().trim_start_matches('\u{feff}').trim();
        if trimmed.eq_ignore_ascii_case("quit") {
            println!("Goodbye.");
            break;
        }
        if trimmed.is_empty() {
            continue;
        }

        world.set_now(unix_now());
        let mut out = Outbox::new();
        dispatch(&registry, &mut world, GOD, trimmed, &mut out);
        for (audience, event) in out.take() {
            if reaches_god(&world, &audience) {
                println!("{}", text_of(&event));
            }
        }
    }
}

/// Whether an event audience reaches God in this single-user REPL: God's
/// own actor stream, broadcasts, and events for the room God is standing in
/// (unless excluded). `Conn` audiences need the M1 session layer.
fn reaches_god(world: &World, audience: &Audience) -> bool {
    match audience {
        Audience::Actor(a) => *a == GOD,
        Audience::All => true,
        Audience::Room { room, exclude } => {
            !exclude.contains(&GOD) && world.get(GOD).and_then(|o| o.location) == Some(*room)
        }
        Audience::Conn(_) => false,
    }
}

/// The current wall-clock time as unix seconds (0 if the clock is broken).
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::LIMBO;

    #[test]
    fn reaches_god_resolves_audiences() {
        let (mut world, _registry) = boot_world();
        world.set_connected(GOD, true);

        assert!(reaches_god(&world, &Audience::Actor(GOD)));
        assert!(!reaches_god(&world, &Audience::Actor(LIMBO)));
        assert!(reaches_god(&world, &Audience::All));
        assert!(!reaches_god(&world, &Audience::Conn(1)));
        // God stands in Limbo.
        assert!(reaches_god(
            &world,
            &Audience::Room {
                room: LIMBO,
                exclude: vec![]
            }
        ));
        // ...but not when excluded, and not for other rooms.
        assert!(!reaches_god(
            &world,
            &Audience::Room {
                room: LIMBO,
                exclude: vec![GOD]
            }
        ));
        let hall = world
            .create(mush_core::ObjKind::Room, "Hall", GOD, None)
            .unwrap();
        assert!(!reaches_god(
            &world,
            &Audience::Room {
                room: hall,
                exclude: vec![]
            }
        ));
    }

    #[test]
    fn repl_dispatch_round_trip() {
        // The exact loop body: dispatch a line as God, render what reaches
        // God. Proves the REPL wiring without touching real stdin.
        let (mut world, registry) = boot_world();
        world.set_connected(GOD, true);
        let mut out = Outbox::new();
        dispatch(
            &registry,
            &mut world,
            GOD,
            "\"Let there be light.",
            &mut out,
        );
        let lines: Vec<String> = out
            .take()
            .iter()
            .filter(|(aud, _)| reaches_god(&world, aud))
            .map(|(_, ev)| text_of(ev))
            .collect();
        assert_eq!(lines, vec!["God says, \"Let there be light.\"".to_string()]);
    }

    #[test]
    fn unix_now_is_sane() {
        // Some time after 2020-01-01 and monotone-ish across two calls.
        let a = unix_now();
        assert!(a > 1_577_836_800, "unix_now returned {a}");
        assert!(unix_now() >= a);
    }
}
