//! **The busy sentences: one table every reader reads** (doc 51 §3.4, unit
//! FL.5).
//!
//! A server refuses a typed line it did not run, for a moment, with one of a
//! few sentences: the world is committing, the engine's queue is full, the
//! arena is busy. A story's reader types the line again. Before this table
//! each reader kept its own list, and row 5 of `docs/flaky-tests.md` was a
//! sentence missing from one of them: a `+sortie/refresh` refused with `An
//! authoritative request is already pending.` was read as its answer and never
//! typed again.
//!
//! Each row names the product file the sentence is written in, and the kit's
//! own test reads that file and fails when the sentence is no longer there. So
//! a reworded refusal fails here, in one place, instead of in every story that
//! meets it. The bot's own lists (`mush_bot::mine::lines::ENGINE_BUSY` and
//! `TRANSIENT_TAILS`) are held to this table by mush-bot's tests.
//!
//! **Not here**: sentences that refuse a login or a connection (the
//! authentication service's "please try again shortly"), which no story types
//! a line into, and game sentences that end "try again in a moment." but
//! answer a line the world did run (`ENGINE_BUSY`'s own comment has examples).

/// One busy sentence: what a reader matches, and where the product writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusySentence {
    /// What a line ends with when it is this refusal, its closing `.` left
    /// off (the seat's is printed both with one and without). Usually the
    /// whole sentence; a tail where the product puts a varying head before it
    /// (the seat's `That cockpit will not wake for you: control state
    /// unknown`).
    pub ends: &'static str,
    /// The product files, from the workspace's `crates/`, that write it. The
    /// kit's test holds each to holding `ends`.
    pub sources: &'static [&'static str],
}

/// **Every sentence a server answers a typed line with when it did not run
/// it, for now.** The line is typed again.
pub const BUSY_SENTENCES: [BusySentence; 9] = [
    // mush-server's world task, `RefusalReason::text`: a commit in flight, a
    // line deferred behind one for too long, the world still booting.
    BusySentence {
        ends: "The world is committing another action; please retry in a moment",
        sources: &["mush-server/src/world_task/mode.rs"],
    },
    BusySentence {
        ends: "The world was busy too long for that line; please retry in a moment",
        sources: &["mush-server/src/world_task/mode.rs"],
    },
    BusySentence {
        ends: "The world is still starting; please try again shortly",
        sources: &["mush-server/src/world_task/mode.rs"],
    },
    // The engine's back-pressure (`drain_authoritative_submit`).
    BusySentence {
        ends: "Your earlier action is still committing; try again in a moment",
        sources: &["mush-core/src/engine.rs"],
    },
    BusySentence {
        ends: "Too many actions are committing right now; try again in a moment",
        sources: &["mush-core/src/engine.rs"],
    },
    // The same back-pressure, one step earlier, in the modules that would
    // queue the line (row 5's sentence).
    BusySentence {
        ends: "An authoritative request is already pending",
        sources: &[
            "mush-sim/src/physical.rs",
            "tacsim/src/detachment/sortie/runtime.rs",
            "tacsim/src/detachment/planning/runtime.rs",
            "tacsim/src/detachment/activation/runtime.rs",
            "tacsim/src/detachment/crew/boarding.rs",
        ],
    },
    // The seat's control-lease mirror, a 1 Hz projection
    // (`mush_sim::seat`'s error), after the cockpit's own head.
    BusySentence {
        ends: "control state unknown",
        sources: &["mush-sim/src/seat.rs"],
    },
    // The sim center's lobby while the arena task is mid-step.
    BusySentence {
        ends: "The arena is busy; try that again in a moment",
        sources: &["tacsim/src/match_/lobby.rs"],
    },
    // Staff's `@newchar` while a commit is in flight.
    BusySentence {
        ends: "the world is committing another action; please retry shortly",
        sources: &["mush-server/src/world_task/staff_ops.rs"],
    },
];

/// **Whether `line` is a busy refusal**: it ends with one of
/// [`BUSY_SENTENCES`] (a closing `.` aside), and does not open with `[`. A
/// channel line or a notice is another party's text, never the world's answer
/// to this session's line: the bot's own readers' rule.
#[must_use]
pub fn is_busy(line: &str) -> bool {
    let body = line.trim();
    if body.starts_with('[') {
        return false;
    }
    let bare = body.strip_suffix('.').unwrap_or(body);
    BUSY_SENTENCES.iter().any(|busy| bare.ends_with(busy.ends))
        || bare.ends_with(crate::readout::FLOODED.ends)
}

/// **Whether any line of `text` is a busy refusal** ([`is_busy`]).
#[must_use]
pub fn holds_busy(text: &str) -> bool {
    text.lines().any(is_busy)
}

/// **The table as needles**, for a reader that waits on substrings: each
/// sentence's `ends`. A needle read finds a busy sentence anywhere in what
/// arrived, so it is a reason to look; [`is_busy`] judges the line.
pub const BUSY_NEEDLES: [&str; BUSY_SENTENCES.len()] = {
    // One name a row: a row added to the table fails to compile here until
    // it is named, so the two cannot drift apart.
    let [a, b, c, d, e, f, g, h, i] = BUSY_SENTENCES;
    [
        a.ends, b.ends, c.ends, d.ends, e.ends, f.ends, g.ends, h.ends, i.ends,
    ]
};

/// [`BUSY_NEEDLES`] as a list, to extend a reader's own needles with, and
/// the flood bucket's refusal ([`crate::readout::FLOODED`], unit FL.8), which
/// the kit's readers also type a line again after.
#[must_use]
pub fn busy_needles() -> Vec<&'static str> {
    let mut needles = BUSY_NEEDLES.to_vec();
    needles.push(crate::readout::FLOODED.ends);
    needles
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The workspace's `crates/` directory, from this crate's own.
    fn crates_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default()
    }

    #[test]
    fn every_busy_sentence_is_still_written_where_the_table_says() {
        let mut missing = Vec::new();
        for busy in BUSY_SENTENCES {
            assert!(!busy.sources.is_empty(), "{busy:?} names no source");
            for source in busy.sources {
                let path = crates_dir().join(source);
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                if !text.contains(busy.ends) {
                    missing.push(format!("{source}: {:?}", busy.ends));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "a busy sentence moved or was reworded; update mush-storykit's table (and the bot's \
             lists): {missing:#?}"
        );
    }

    #[test]
    fn a_busy_line_is_known_at_its_end_and_never_in_a_channel() {
        assert!(is_busy(
            "The world is committing another action; please retry in a moment."
        ));
        assert!(is_busy(
            "That cockpit will not wake for you: control state unknown"
        ));
        assert!(is_busy(
            "That cockpit will not wake for you: control state unknown."
        ));
        assert!(is_busy("An authoritative request is already pending.\r"));
        assert!(!is_busy(
            "[Public] Ada: An authoritative request is already pending."
        ));
        assert!(!is_busy("control state unknown, she said, and left"));
        assert!(holds_busy(
            "Rig 1 status:\r\nAn authoritative request is already pending.\r\n"
        ));
        assert!(!holds_busy("Rig 1 status:\r\n  fuel 40 of 60\r\n"));
    }

    #[test]
    fn the_needles_find_every_sentence() {
        let needles = busy_needles();
        for busy in BUSY_SENTENCES {
            assert!(
                needles.iter().any(|needle| busy.ends.contains(needle)),
                "{busy:?}"
            );
        }
    }
}
