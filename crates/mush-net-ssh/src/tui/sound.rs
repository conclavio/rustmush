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
//! **One sound a frame** (the dev2 smoke of 2026-10-23): w11term drops a
//! sound that comes within 40 ms of the one before (`rate-limited`) rather
//! than queueing it, so a frame's kinds sent back to back played the first
//! alone — and when that first was `slot_fired`, a name with no file, nothing
//! played at all. A frame now sounds **one** name: the most important of its
//! kinds ([`PRIORITY`]) that the terminal can play — a name the shipped pack
//! has, for a w11term; a [`BELL_KINDS`] kind, for a bell — and that has not
//! sounded within [`DEBOUNCE`]. An alarm is the same name sent again on that
//! cadence, which is what a frame that still says `overheated` does.
//!
//! `@sounds on|off` (the transport's own, per connection) mutes it all.
//!
//! **The pack** (doc 21 cut G.11): a w11term is also told the asset pack's
//! version — OSC 440's `pack` verb, `ESC ] 440 ; 1 ; pack ; <version> ESC \`
//! — the moment its `XTVERSION` names it, and again every [`REANNOUNCE`]
//! (w11term drops an announcement that lands during a sync, so a slow repeat
//! is how a failed one is retried; never with a frame). It then fetches what
//! changed over `sftp` on this same connection, and its OSC 440 names resolve
//! from that pack first.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

/// One sound a kind at most this often (w11term's queue is 16 deep and 40 ms
/// a sound; an alarm repeats on this cadence).
pub const DEBOUNCE: Duration = Duration::from_secs(2);

/// How often a w11term is told the pack's version again (G.11).
pub const REANNOUNCE: Duration = Duration::from_secs(600);

/// w11term's OSC 440 `pack` announcement (wire version 1, ST-terminated).
#[must_use]
pub fn osc440_pack(version: &str) -> Vec<u8> {
    format!("\x1b]440;1;pack;{version}\x1b\\").into_bytes()
}

/// The overheat alarm's name: not a notice kind but the frame's own verdict
/// (`heat.overheated`), said while it stands.
pub const OVERHEAT: &str = "overheat";

/// **What sounds first** when a frame has several kinds: the tank lost or
/// crippled, a kill, the tank hurt, the overheat alarm, the target hurt, the
/// lock lost, a hit, the power up, a new contact, a lock, and the rest after.
/// A kind not listed comes after every listed one, in the frame's order.
pub const PRIORITY: &[&str] = &[
    "destroyed",
    "disabled",
    "kill",
    "section_lost",
    "struck",
    OVERHEAT,
    "target_section_lost",
    "lock_lost",
    "hit",
    "power_online",
    "acquired",
    "locked",
    "unidentified",
    "lost",
    "miss",
    "struck_spent",
    "classified",
    "refused",
    "blocked",
    "receiver_degraded",
    "receiver_clear",
    "drive_no_route",
    "drive_arrived",
    "emcon_on",
    "emcon_off",
    "slot_fired",
    "slot_held",
];

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
    /// The asset pack's version, when one is served (G.11).
    pack: Option<String>,
    /// The pack's sound names: the only names a w11term is sent, so none is
    /// spent on a name with no file. `None` sends any (no pack served).
    names: Option<std::collections::BTreeSet<String>>,
    /// When the pack was last announced.
    announced: Option<Instant>,
}

impl Default for Sounds {
    fn default() -> Self {
        Self::new()
    }
}

impl Sounds {
    /// Sounds on, the terminal not yet known (so BEL until it says), the
    /// shipped asset pack the one announced.
    #[must_use]
    pub fn new() -> Sounds {
        let pack = crate::assets::AssetPack::shipped();
        let mut sounds = Sounds::with_pack(pack.map(|p| p.pack_version().to_string()));
        sounds.names = pack.map(|p| p.sound_names().clone());
        sounds
    }

    /// [`Sounds::new`] announcing `pack` (or none).
    #[must_use]
    pub fn with_pack(pack: Option<String>) -> Sounds {
        Sounds {
            on: true,
            w11term: false,
            terminal: None,
            last: HashMap::new(),
            last_bell: None,
            out: Vec::new(),
            pack: pack.filter(|v| valid_name(v)),
            names: None,
            announced: None,
        }
    }

    /// [`Sounds::with_pack`] that sends a w11term only `names`.
    #[must_use]
    pub fn with_pack_names(pack: Option<String>, names: &[&str]) -> Sounds {
        let mut sounds = Sounds::with_pack(pack);
        sounds.names = Some(names.iter().map(|n| (*n).to_string()).collect());
        sounds
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

    /// The terminal's `XTVERSION` reply (the text after `>|`); a w11term is
    /// told the pack at once (G.11).
    pub fn set_terminal(&mut self, version: &str) {
        self.w11term = names_w11term(version);
        self.terminal = Some(version.chars().take(64).collect());
        self.announced = None;
        self.tick(Instant::now());
    }

    /// Announce the pack to a w11term not told within [`REANNOUNCE`] (G.11).
    pub fn tick(&mut self, now: Instant) {
        let Some(pack) = self.pack.as_deref().filter(|_| self.w11term) else {
            return;
        };
        if self
            .announced
            .is_none_or(|at| now.saturating_duration_since(at) >= REANNOUNCE)
        {
            self.announced = Some(now);
            self.out.extend(osc440_pack(pack));
        }
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

    /// A seated cockpit frame: one sound for its notices' kinds and the
    /// overheat alarm while the frame says so (see [`Sounds::play_one`]).
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
        let names: Vec<&str> = kinds.iter().map(String::as_str).collect();
        self.play_one(&names, now);
    }

    /// Sound `name` alone (see [`Sounds::play_one`]).
    pub fn play(&mut self, name: &str, now: Instant) {
        self.play_one(&[name], now);
    }

    /// **Sound one of `names`**, if sounds are on: the first in [`PRIORITY`]
    /// order that is a name, that the terminal can play — one the pack has,
    /// for a w11term; a [`BELL_KINDS`] kind, for a bell — and that has not
    /// sounded within [`DEBOUNCE`]; by name to a w11term, as BEL (one a
    /// [`DEBOUNCE`] whatever the kind) to anything else. One at most: w11term
    /// drops a sound within 40 ms of the last.
    pub fn play_one(&mut self, names: &[&str], now: Instant) {
        if !self.on {
            return;
        }
        let rank = |name: &str| {
            PRIORITY
                .iter()
                .position(|p| *p == name)
                .unwrap_or(PRIORITY.len())
        };
        let mut ranked: Vec<&str> = Vec::new();
        for name in names {
            if !ranked.contains(name) {
                ranked.push(name);
            }
        }
        ranked.sort_by_key(|name| rank(name));
        let recent = |at: Option<&Instant>| {
            at.is_some_and(|at| now.saturating_duration_since(*at) < DEBOUNCE)
        };
        if !self.w11term && recent(self.last_bell.as_ref()) {
            return;
        }
        let pick = ranked.into_iter().find(|name| {
            valid_name(name)
                && !recent(self.last.get(*name))
                && if self.w11term {
                    self.names
                        .as_ref()
                        .is_none_or(|names| names.contains(*name))
                } else {
                    BELL_KINDS.contains(name)
                }
        });
        let Some(name) = pick else {
            return;
        };
        self.last.insert(name.to_string(), now);
        if self.w11term {
            self.out.extend(osc440_play(name));
        } else {
            self.last_bell = Some(now);
            self.out.push(0x07);
        }
    }

    /// The bytes to send with the next paint.
    pub fn take_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }
}

/// The pack a [`Sounds`] announces, for `@sounds`' answer.
impl Sounds {
    /// The pack's version, when one is served.
    #[must_use]
    pub fn pack(&self) -> Option<&str> {
        self.pack.as_deref()
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
        let mut s = Sounds::with_pack(Some("pack_1".into()));
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
        let mut s = Sounds::with_pack(None);
        s.set_terminal("w11term 0.4.1");
        assert!(s.by_name());
        s.frame(&frame(&["acquired", "miss"], true), t0);
        assert_eq!(
            s.take_bytes(),
            osc440_play("overheat"),
            "one sound a frame: the alarm outranks a contact and a miss"
        );
        s.frame(&frame(&["miss"], true), t0 + Duration::from_secs(1));
        assert_eq!(
            s.take_bytes(),
            osc440_play("miss"),
            "the alarm debounced, the next kind sounds"
        );
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
        let mut s = Sounds::with_pack(None);
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

    /// **The dev2 smoke's burst** (2026-10-23): a frame with a slot's shot,
    /// a hit, a section lost and a kill sounds the kill alone — never a name
    /// the pack has no file for — and the next frame's best undebounced kind
    /// after it.
    #[test]
    fn a_burst_sounds_its_most_important_name_the_pack_has() {
        let t0 = Instant::now();
        let mut s = Sounds::with_pack_names(None, &["hit", "kill", "target_section_lost"]);
        s.set_terminal("w11term 0.4.1");
        s.frame(
            &frame(&["slot_fired", "hit", "target_section_lost", "kill"], false),
            t0,
        );
        assert_eq!(s.take_bytes(), osc440_play("kill"));
        s.frame(&frame(&["slot_fired"], false), t0);
        assert!(s.take_bytes().is_empty(), "no file, no sound");
        s.frame(
            &frame(&["kill", "hit"], false),
            t0 + Duration::from_millis(500),
        );
        assert_eq!(s.take_bytes(), osc440_play("hit"), "the kill debounced");
        // A plain terminal: one bell for the frame, the bell kind found among
        // the rest.
        let mut plain = Sounds::with_pack(None);
        plain.frame(&frame(&["slot_fired", "hit", "kill"], false), t0);
        assert_eq!(plain.take_bytes(), vec![0x07]);
        // The shipped pack's names are the ones a new Sounds sends.
        let shipped = Sounds::new();
        assert!(
            shipped
                .names
                .as_ref()
                .is_some_and(|n| n.contains("kill") && !n.contains("slot_fired"))
        );
    }

    /// **Doc 21 cut G.11**: a w11term is told the pack the moment it says
    /// what it is, and again every ten minutes, never sooner; a plain
    /// terminal never is.
    #[test]
    fn a_w11term_is_told_the_pack_and_told_again_slowly() {
        let mut plain = Sounds::with_pack(Some("pack_abc".into()));
        plain.set_terminal("WezTerm 1");
        plain.tick(Instant::now() + REANNOUNCE * 2);
        assert!(plain.take_bytes().is_empty());
        let mut s = Sounds::with_pack(Some("pack_abc".into()));
        s.set_terminal("w11term 0.4.1");
        assert_eq!(
            String::from_utf8(s.take_bytes()).unwrap(),
            "\x1b]440;1;pack;pack_abc\x1b\\"
        );
        let t0 = Instant::now();
        s.tick(t0 + Duration::from_secs(5));
        assert!(s.take_bytes().is_empty(), "not with every tick");
        s.tick(t0 + REANNOUNCE + Duration::from_secs(1));
        assert_eq!(s.take_bytes(), osc440_pack("pack_abc"));
        // A version that is not a token is never announced.
        let mut bad = Sounds::with_pack(Some("../x".into()));
        bad.set_terminal("w11term 1");
        assert!(bad.take_bytes().is_empty());
        // The shipped pack is the one a new Sounds announces.
        assert_eq!(
            Sounds::new().pack(),
            crate::assets::AssetPack::shipped().map(|p| p.pack_version())
        );
    }
}
