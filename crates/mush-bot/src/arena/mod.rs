//! The simulator pilot's senses (cut AJ): what a seated line-mode pilot is told,
//! read into one picture of the match.
//!
//! **Independence.** Everything under this module is the bot's own. It reads
//! only the player-visible text of its own SSH line session — the answers to its
//! own `CONTACTS`, `WEAPONSTATUS`, `STATUS`, `RANGE` and `BEARING`, the cockpit's
//! notices and the simulator centre's announcements — and it is written from the
//! player help (`help simulator`, `help cockpit`) alone. No game crate is a
//! normal or build dependency; the one test that holds these parsers to the
//! game's own formatters takes `tacsim` as a development dependency, so a changed
//! sentence breaks a test and not a match. Nothing of the machines' own
//! controller is read, imported or mirrored.
//!
//! **No I/O and no clock.** Every function that needs time takes `now_ms`, the
//! runner's monotonic session milliseconds, and returns data; the runner does the
//! reading and the sending.
//!
//! **What flies** (cut AJ.2): [`lane::Lane`] ties the picture to the
//! [`lobby::Lobby`] (the walk, the claim, the start, the board and the
//! re-board, in a closed vocabulary) and to the [`pilot::Pilot`] (a
//! deterministic tactician), types at most four lines a second, and reports
//! ([`report::Report`]) on the centre's own result words. The `[arena]` table is
//! [`settings::Settings`].
//!
//! **Who commands** (cut AJ.3): optionally a model, through
//! [`commander`] — it sets the pilot's [`Doctrine`] (where it goes and what it
//! hunts) about once a second and at once on events, one call at a time, and
//! never pulls a trigger. The lane owns its schedule; the runner owns the call.
//!
//! **The tempo this is sized to**, as the server ships it (plain facts, not
//! links: the game crates are not dependencies):
//!
//! * a line-mode pilot is sent **no periodic picture**. Every read verb answers
//!   with one line per sentence and a status frame a line client prints as
//!   nothing; the cockpit's notices arrive one line each, on transitions only.
//!   So the pilot polls, and [`picture::Picture`] folds the answers in;
//! * the session's flood bucket refills at 10 lines a second with a burst of 30
//!   (the session manager's `SessionConfig`), so a lane that polls three verbs a
//!   second and orders a few more stays far inside it;
//! * the cockpit queues at most 8 pending orders per unit
//!   (`[tacsim.arena.intents].max_pending_per_unit`);
//! * the arena pulses every 5,000 ms while quiet, 1,000 ms on alert and 500 ms
//!   engaged (`[tacsim.arena.tempo]`), so an order's answer from the arena is a
//!   pulse away and a notice is at most one quiet pulse late.
use serde::{Deserialize, Serialize};

pub mod commander;
pub mod lane;
pub mod lines;
pub mod lobby;
pub mod picture;
pub mod pilot;
pub mod report;
pub mod settings;

pub use commander::{CommanderReply, Doctrine};
pub use lane::{Fed, Lane, LaneEvent, LaneResult, Purpose, Send};
pub use lines::{Assembler, Line, classify};
pub use picture::Picture;

/// One side of a simulator match (`help simulator`: "two sides fly stock tanks
/// against each other").
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Red,
    Blue,
}

impl Side {
    /// The word the centre prints: `red` or `blue`.
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Red => "red",
            Side::Blue => "blue",
        }
    }

    /// The side's callsign letter (`help cockpit` "Names": red pod 1 is `RA`).
    pub fn letter(self) -> char {
        match self {
            Side::Red => 'R',
            Side::Blue => 'B',
        }
    }

    pub fn other(self) -> Side {
        match self {
            Side::Red => Side::Blue,
            Side::Blue => Side::Red,
        }
    }

    /// `red` or `blue`, in any case, trimmed.
    pub fn parse(word: &str) -> Option<Side> {
        let word = word.trim();
        if word.eq_ignore_ascii_case("red") {
            Some(Side::Red)
        } else if word.eq_ignore_ascii_case("blue") {
            Some(Side::Blue)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Side;

    #[test]
    fn a_side_reads_both_ways() {
        for side in [Side::Red, Side::Blue] {
            assert_eq!(Side::parse(side.as_str()), Some(side));
            assert_eq!(
                Side::parse(&format!("  {}  ", side.as_str().to_uppercase())),
                Some(side)
            );
            assert_eq!(side.other().other(), side);
        }
        assert_eq!(Side::Red.letter(), 'R');
        assert_eq!(Side::Blue.letter(), 'B');
        assert_eq!(Side::parse("green"), None);
        assert_eq!(serde_json::to_string(&Side::Blue).unwrap(), "\"blue\"");
        assert_eq!(serde_json::from_str::<Side>("\"red\"").unwrap(), Side::Red);
    }
}
