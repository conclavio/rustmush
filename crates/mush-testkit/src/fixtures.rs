//! The small fixed values and one-line drivers a test file used to define for
//! itself (doc 46 cut CT.2).
//!
//! Each of these was written out privately, word for word, in between five and
//! thirty files of `mush-economy`, `mush-roll` and `mush-market`. They are the
//! same thing in every one of them, so they live here once. A helper of the
//! same name that means something else in its file (a `token` that counts in
//! big-endian, a `digest` of a fixed subject) stays in that file.
//!
//! The values are part of what the commerce snapshots pin (doc 46 cut CT.0):
//! `token(7)` is sixteen bytes of `7`, and a test that takes it from here
//! writes the bytes it wrote before.

use std::collections::BTreeSet;

use mush_core::{
    Audience, ClockRef, ClockStamp, CommitToken, DefinitionDigest, Engine, GameModule, ModError,
    ObjId, Registry, World,
};
use mush_proto::{OutputEvent, text_of};

use crate::TestWorld;

/// The commit token whose sixteen bytes are all `n`.
///
/// ```
/// assert_eq!(mush_testkit::fixtures::token(7).to_hex(), "07".repeat(16));
/// ```
#[must_use]
pub fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}

/// The set of [`token`]s of `ids`.
#[must_use]
pub fn tokens(ids: &[u8]) -> BTreeSet<CommitToken> {
    ids.iter().map(|n| token(*n)).collect()
}

/// The definition digest whose thirty-two bytes are all `n`. The test files
/// call it `hash` or `digest`.
#[must_use]
pub fn digest(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}

/// A world-IC clock stamp of generation 1 at `at`.
#[must_use]
pub fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}

/// [`Engine::with_modules`], boxed and never inlined: an [`Engine`] built in a
/// test's own frame costs that frame the whole engine
/// (`clippy::large_stack_frames`).
///
/// # Errors
///
/// A registration collision in `modules`.
#[inline(never)]
pub fn boxed_engine(
    world: World,
    registry: Registry,
    modules: Vec<Box<dyn GameModule>>,
) -> Result<Box<Engine>, ModError> {
    Engine::with_modules(world, registry, modules).map(Box::new)
}

/// Every event's plain text, a line each.
#[must_use]
pub fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether any event is an error-styled system line (a refusal, a usage line,
/// `Huh?`).
#[must_use]
pub fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

/// Run `line` as `actor`; it must not be refused. Returns what it printed.
///
/// # Panics
///
/// Panics, with the output, if the line answered with an error.
#[track_caller]
pub fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let events = tw.run(actor, line);
    assert!(
        !has_error(&events),
        "`{line}` was refused: {}",
        texts(&events)
    );
    texts(&events)
}

/// Run `line` as `actor`; it must be refused. Returns the refusal as printed.
///
/// # Panics
///
/// Panics, with the output, if the line was not refused.
#[track_caller]
pub fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let events = tw.run(actor, line);
    assert!(
        has_error(&events),
        "`{line}` should have been refused, got: {}",
        texts(&events)
    );
    texts(&events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::GOD;

    #[test]
    fn the_fixed_values_are_the_bytes_the_test_files_wrote() {
        assert_eq!(token(7).to_hex(), "07".repeat(16));
        assert_eq!(tokens(&[7, 7, 9]), BTreeSet::from([token(7), token(9)]));
        assert_eq!(digest(9), DefinitionDigest::from_bytes([9; 32]));
        assert_eq!(
            stamp(40),
            ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 40
            }
        );
    }

    #[test]
    fn ok_and_err_tell_a_refusal_from_an_answer() {
        let mut tw = TestWorld::new();
        let said = ok(&mut tw, GOD, "\"hello");
        assert!(said.contains("hello"), "{said}");
        let refused = err(&mut tw, GOD, "no-such-verb at all");
        assert!(!refused.is_empty());
        assert!(has_error(&tw.run(GOD, "no-such-verb at all")));
        assert!(!has_error(&tw.run(GOD, "\"hello")));
    }

    #[test]
    #[should_panic(expected = "was refused")]
    fn ok_panics_on_a_refusal() {
        let mut tw = TestWorld::new();
        let _ = ok(&mut tw, GOD, "no-such-verb at all");
    }

    #[test]
    #[should_panic(expected = "should have been refused")]
    fn err_panics_on_an_answer() {
        let mut tw = TestWorld::new();
        let _ = err(&mut tw, GOD, "\"hello");
    }

    #[test]
    fn boxed_engine_composes_what_with_modules_composes() {
        let (world, registry) = crate::boot_world();
        let engine = boxed_engine(world, registry, Vec::new()).expect("no module, no collision");
        assert!(engine.world.get(GOD).is_some());
    }
}
