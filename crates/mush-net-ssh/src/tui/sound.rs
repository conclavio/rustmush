//! **Bells and sounds** (doc 21 cut G.10): the cockpit's notices as sounds.
//!
//! Every seated `arena.cockpit` frame carries the notices of its own pulse
//! (a read verb's re-emitted frame carries none), and each notice's `kind` is
//! a wire word — `acquired`, `hit`, `struck`, `power_online`, … — that is
//! already a sound name (1–64 of `[A-Za-z0-9_-]`). The overheat alarm is
//! `overheat`, said again on every frame that still says `overheated`.
//!
//! **Where a sound goes is what the terminal is.** A terminal whose
//! `XTVERSION` reply names w11term (the owner's terminal, its
//! `docs/game-client-protocol.md`) is sent w11term's four-field OSC 440,
//! `ESC ] 440 ; 1 ; play ; <name> ESC \`, for every kind: w11term plays the
//! name from its pack or its local sounds folder, and a name with no file is
//! silent there. Every other terminal is sent BEL, for the few kinds a pilot
//! must hear ([`BELL_KINDS`]), at most one a [`DEBOUNCE`]. (mintty's older
//! `440;file` form is not w11term's and is never sent. Kitty's OSC 99 would
//! pop a desktop notification with every sound, so kitty gets BEL too.)
//!
//! **Debounced here, not there.** w11term plays serially, 40 ms apart, from
//! a 16-deep queue, so the gateway sends at most one sound a kind every two
//! seconds; an alarm is the same name sent again on that cadence, which is
//! what a frame that still says `overheated` does.
//!
//! `@sounds on|off` (the transport's own, per connection) mutes it all.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

/// One sound a kind at most this often (w11term's queue is 16 deep and 40 ms
/// a sound; an alarm repeats on this cadence).
pub const DEBOUNCE: Duration = Duration::from_secs(2);

/// The overheat alarm's name: not a notice kind but the frame's own verdict
/// (`heat.overheated`), said while it stands.
pub const OVERHEAT: &str = "overheat";

/// The kinds that ring a plain terminal's bell: a contact gained, the lock
/// lost, the tank struck or losing a section, the power up, a kill, the tank
/// gone or crippled, and the overheat alarm. A w11term is sent every kind.
pub const BELL_KINDS: &[&str] = &[
    "acquired",
    "lock_lost",
    "struck",
    "section_lost",
    "kill",
    "power_online",
    "destroyed",
    "disabled",
    OVERHEAT,
];

/// Whether `name` may be sent as an OSC 440 argument: 1–64 ASCII letters,
/// digits, `_` or `-`, and not one of w11term's reserved words.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && name != "system"
        && name != "off"
}

/// w11term's OSC 440 `play` for `name` (wire version 1, ST-terminated).
#[must_use]
pub fn osc440_play(name: &str) -> Vec<u8> {
    format!("\x1b]440;1;play;{name}\x1b\\").into_bytes()
}

/// Whether an `XTVERSION` reply's text names w11term.
#[must_use]
pub fn names_w11term(version: &str) -> bool {
    version
        .split_whitespace()
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("w11term"))
}

/// **This connection's sounds**: on or off, what the terminal is, when each
/// kind last sounded, and the bytes waiting to go out with the next paint.
#[derive(Debug)]
pub struct Sounds {
    on: bool,
    w11term: bool,
    terminal: Option<String>,
    last: HashMap<String, Instant>,
    last_bell: Option<Instant>,
    out: Vec<u8>,
}

impl Default for Sounds {
    fn default() -> Self {
        Self::new()
    }
}

impl Sounds {
    /// Sounds on, the terminal not yet known (so BEL until it says).
    #[must_use]
    pub fn new() -> Sounds {
        Sounds {
            on: true,
            w11term: false,
            terminal: None,
            last: HashMap::new(),
            last_bell: None,
            out: Vec::new(),
        }
    }

    /// `@sounds on|off`.
    pub fn set_on(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.out.clear();
        }
    }

    /// Whether sounds are on.
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// The terminal's `XTVERSION` reply (the text after `>|`).
    pub fn set_terminal(&mut self, version: &str) {
        self.w11term = names_w11term(version);
        self.terminal = Some(version.chars().take(64).collect());
    }

    /// The terminal's name and version as it gave them, if it did.
    #[must_use]
    pub fn terminal(&self) -> Option<&str> {
        self.terminal.as_deref()
    }

    /// Whether sounds go out by name (w11term) rather than as BEL.
    #[must_use]
    pub fn by_name(&self) -> bool {
        self.w11term
    }

    /// A seated cockpit frame: a sound for each of its notices' kinds, and
    /// the overheat alarm while the frame says so.
    pub fn frame(&mut self, frame: &Value, now: Instant) {
        let kinds = frame
            .get("notices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|n| n.get("kind").and_then(Value::as_str));
        let overheated = frame.pointer("/heat/overheated").and_then(Value::as_bool) == Some(true);
        let kinds: Vec<String> = kinds
            .map(str::to_string)
            .chain(overheated.then(|| OVERHEAT.to_string()))
            .collect();
        for kind in kinds {
            self.play(&kind, now);
        }
    }

    /// Sound `name`, if sounds are on, the name is one, and that kind has not
    /// sounded within [`DEBOUNCE`]: by name to a w11term, BEL (for
    /// [`BELL_KINDS`], one a [`DEBOUNCE`] whatever the kind) to anything else.
    pub fn play(&mut self, name: &str, now: Instant) {
        if !self.on || !valid_name(name) {
            return;
        }
        let recent = |at: Option<&Instant>| {
            at.is_some_and(|at| now.saturating_duration_since(*at) < DEBOUNCE)
        };
        if recent(self.last.get(name)) {
            return;
        }
        if self.w11term {
            self.last.insert(name.to_string(), now);
            self.out.extend(osc440_play(name));
        } else if BELL_KINDS.contains(&name) && !recent(self.last_bell.as_ref()) {
            self.last.insert(name.to_string(), now);
            self.last_bell = Some(now);
            self.out.push(0x07);
        }
    }

    /// The bytes to send with the next paint.
    pub fn take_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(kinds: &[&str], overheated: bool) -> Value {
        serde_json::json!({
            "notices": kinds.iter().map(|k| serde_json::json!({"kind": k})).collect::<Vec<_>>(),
            "heat": {"heat": 120, "max": 100, "overheated": overheated},
        })
    }

    /// A plain terminal: BEL for a contact gained, nothing for a miss, one
    /// bell a frame however many kinds ring, and again after two seconds.
    #[test]
    fn a_plain_terminal_rings_its_bell_for_the_kinds_that_matter() {
        let t0 = Instant::now();
        let mut s = Sounds::new();
        s.frame(&frame(&["acquired", "miss", "struck"], false), t0);
        assert_eq!(s.take_bytes(), vec![0x07], "one bell for the frame");
        s.frame(&frame(&["miss", "classified"], false), t0);
        assert!(s.take_bytes().is_empty(), "a miss rings nothing");
        s.frame(&frame(&["struck"], false), t0 + Duration::from_millis(500));
        assert!(s.take_bytes().is_empty(), "a bell a debounce at most");
        s.frame(&frame(&["struck"], false), t0 + DEBOUNCE);
        assert_eq!(s.take_bytes(), vec![0x07]);
    }

    /// A w11term: every kind by name, each once a debounce, the overheat
    /// alarm while the frame says so and silent once it clears.
    #[test]
    fn a_w11term_hears_every_kind_by_name_and_the_alarm_repeats() {
        let t0 = Instant::now();
        let mut s = Sounds::new();
        s.set_terminal("w11term 0.4.1");
        assert!(s.by_name());
        s.frame(&frame(&["acquired", "miss"], true), t0);
        let out = String::from_utf8(s.take_bytes()).unwrap();
        assert_eq!(
            out,
            "\x1b]440;1;play;acquired\x1b\\\x1b]440;1;play;miss\x1b\\\x1b]440;1;play;overheat\x1b\\"
        );
        s.frame(&frame(&["miss"], true), t0 + Duration::from_secs(1));
        assert!(s.take_bytes().is_empty(), "debounced");
        s.frame(&frame(&[], true), t0 + DEBOUNCE);
        assert_eq!(
            String::from_utf8(s.take_bytes()).unwrap(),
            "\x1b]440;1;play;overheat\x1b\\",
            "the alarm again"
        );
        s.frame(&frame(&[], false), t0 + DEBOUNCE * 2);
        assert!(s.take_bytes().is_empty(), "cleared: the alarm stops");
    }

    /// Off is silent and drops what was waiting; a name that is not one is
    /// never sent; another terminal's version is not w11term.
    #[test]
    fn off_is_silent_and_names_are_checked() {
        let t0 = Instant::now();
        let mut s = Sounds::new();
        s.set_terminal("w11term 1");
        s.play("hit", t0);
        s.set_on(false);
        assert!(s.take_bytes().is_empty());
        s.play("acquired", t0);
        assert!(s.take_bytes().is_empty());
        s.set_on(true);
        for bad in ["", "a;b", "../x", "system", "off", &"x".repeat(65)] {
            s.play(bad, t0);
        }
        assert!(s.take_bytes().is_empty());
        assert!(names_w11term("W11TERM 2.0"));
        assert!(!names_w11term("WezTerm 20240203"));
        assert!(!names_w11term("mintty 3.7.0"));
        assert!(!names_w11term(""));
    }
}
