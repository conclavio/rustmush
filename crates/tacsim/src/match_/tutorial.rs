//! **First-time hints: one line naming the next verb, at seven points of a
//! pilot’s first match and first authored save, once per account** (16b cut AH.1; 16c N18; doc 16 §1, §3.7
//! "Verbs", §3.8 "Lifecycle", §5; the sixth, `mind`, 17b cut M5.6).
//!
//! A pilot who has never typed a verb is not helped by a line that tells the
//! truth about the match; every line the lobby and the cockpit already say
//! does that. What the smoke's first-time pilot lacked (16c T-77) was the NEXT
//! word. So at seven milestones the simulator says one sentence,
//! opening [`HINT_PREFIX`], naming the verb that works there — and never again
//! for that account:
//!
//! ```text
//! lobby     SIM TEAM    a Player walks into the lobby while a claim would be taken
//! boarding  BOARD       SIM START opened boarding: every claimed slot's pilot
//! panel     STARTUP     the pilot's first frame of the match, power off
//! power     VIEW        the frame that says "Power online."
//! contact   LOCK, FIRE  a frame that tells an enemy apart, with a gun and power up
//! mind      SIM MIND    the match's end (`match_::finish`), to each of its pilots
//! mind_saved @mind/prove the author's first successful saved revision
//! ```
//!
//! The sixth (17b cut M5.6; doc 17 §15's first hint, reworded by the M5 design
//! note's C-12 to what M5 makes true — no rule is sealed before the rule
//! language's cut) names the verb that chooses the mind which flies a tank its
//! pilot leaves; it is said once the first match is over, when the pilot is
//! back in the lobby's reach and `SIM MIND` is the next match's word. The
//! seventh (M6; OQ-10 (a)) follows the first successful `@mind/save` and names
//! `@mind/prove`, which flies that saved revision in private paired trials.
//!
//! Each sentence is one of seven string constants ([`HINT_LINES`]): no hint
//! interpolates anything, so no hint can name a unit, a pilot, a designator or
//! a place (doc 16 §1's observer rule holds by construction, and the gates
//! below read a frame's own fields without ever reaching a sentence). A hint is
//! `Sys` output on the arena's reply category (`NOTICE_CATEGORY`), so it rings
//! on the TUI's notices pane and the web's ring while seated and stays in the
//! scrollback; it is never a frame field, never a ledger event, never on the
//! tape, the replay or the report.
//!
//! # Per account, not per character
//!
//! A macro is a character's own vocabulary and lives on the character (`@macro`,
//! 16b cut AD.1, twenty per character); a hint is about the person learning the
//! game, and an alt is the same person, so the flag is keyed by the immutable
//! [`mush_core::AccountId`] doc 16 §3.8 keys a slot by. The record is one small
//! document per account, `tacsim:tutorial:<account>` ([`tutorial_key`]), on
//! the module's own persist lane beside its pins, loaded at boot into the
//! `TacsimStore` slot; an unmapped character has no account, and a hint to it
//! fails closed (the lobby's own rule for a claim).
//!
//! # Said only when the verb it names would work
//!
//! A point is consumed only when it is said, and it is said only when the verb
//! it names would work: to a session with a live connection
//! (`World::has_live_conns`, the world's own delivery predicate), with the
//! layer on, and at an occasion whose gate holds — a claimable centre and no
//! slot of one's own for `SIM TEAM`, power off for `STARTUP`, an enemy, a gun
//! and power on for `LOCK` and `FIRE`. A point refused by any of them is not
//! marked, so it waits for its next occasion; a point said is marked and never
//! said again, and `@tutorial on` does not re-arm it (once means once).
//!
//! # Why a verb of its own and not a `SIM` selector
//!
//! `SIM` is `authoritative` and refused outside the three centre rooms, while
//! the switch must work from Limbo, a pod or anywhere else a player first reads
//! a hint they do not want (the plan's D7), so `@tutorial` is a Player verb
//! registered globally with no gate and no layer check.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{
    AccountId, ArgShape, CmdError, CommandSpec, Ctx, ObjId, ObjKind, World, WorldEvent,
    WorldObserver,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::perception::FrameNotice;
use crate::arena::state::Power;
use crate::arena::step::Frame;
use crate::arena_bridge::NOTICE_CATEGORY;
use crate::match_::MatchState;
use crate::match_::centre;
use crate::match_::lobby;

/// What every hint opens with — the word a reader, a panel and a test find it by.
pub const HINT_PREFIX: &str = "Hint: ";

/// The document's `record` word: a body that does not carry it is not a
/// tutorial record, whatever key it sits under.
pub const RECORD: &str = "tacsim.tutorial";

/// The key family: `tacsim:tutorial:<account>`.
pub const KEY_PREFIX: &str = "tacsim:tutorial:";

/// The verb.
pub const TUTORIAL_VERB: &str = "@tutorial";

/// **One account's tutorial document key**: [`KEY_PREFIX`] and the account's
/// canonical hash in its string form.
///
/// A store key the module's own namespace keeps, like the body's `account`
/// field beside it (the store's own round-trip, which is what `AccountId`'s
/// serde form is for): never a line a player reads and never a field a log
/// carries — `AccountId`'s own contract, and why the loader's warnings carry no
/// key.
#[must_use]
pub fn tutorial_key(account: AccountId) -> SmolStr {
    SmolStr::new(format!("{KEY_PREFIX}{}", account.canonical_hash()))
}

/// **The six points of a first match**, in the order a first match usually
/// takes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HintPoint {
    /// Walking into the Simulator Lobby while a claim would be taken.
    Lobby,
    /// `SIM START` opened boarding.
    Boarding,
    /// The pilot's first frame of the match, with its power off.
    Panel,
    /// The frame that carries `Power online.`
    Power,
    /// The frame that first tells an enemy apart, in a unit with a gun and power up.
    Contact,
    /// The match's end (`match_::finish`), to each of its pilots still connected:
    /// `SIM MIND` chooses the mind that flies the tank its pilot leaves (17b cut
    /// M5.6).
    Mind,
    /// First successful authored save, once per account.
    MindSaved,
}

impl HintPoint {
    /// Every point, in order.
    pub const ALL: [HintPoint; 7] = [
        HintPoint::Lobby,
        HintPoint::Boarding,
        HintPoint::Panel,
        HintPoint::Power,
        HintPoint::Contact,
        HintPoint::Mind,
        HintPoint::MindSaved,
    ];

    /// The point's word, as the document stores it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            HintPoint::Lobby => "lobby",
            HintPoint::Boarding => "boarding",
            HintPoint::Panel => "panel",
            HintPoint::Power => "power",
            HintPoint::Contact => "contact",
            HintPoint::Mind => "mind",
            HintPoint::MindSaved => "mind_saved",
        }
    }

    /// One stored word, or `None` for a word this build does not know.
    #[must_use]
    pub fn parse(word: &str) -> Option<HintPoint> {
        HintPoint::ALL
            .iter()
            .copied()
            .find(|point| point.as_str() == word)
    }

    /// **The sentence**, verbatim: one fixed string per point, each under the
    /// 120-column floor, and nothing interpolated into any of them.
    #[must_use]
    pub const fn line(self) -> &'static str {
        match self {
            HintPoint::Lobby => {
                "Hint: SIM TEAM red (or SIM TEAM blue) claims a pilot slot; @tutorial off stops \
                 these hints."
            }
            HintPoint::Boarding => {
                "Hint: walk into your side's team room (red or blue from the lobby) and type BOARD."
            }
            HintPoint::Panel => "Hint: the panel is yours now: STARTUP brings your power up.",
            HintPoint::Power => {
                "Hint: VIEW draws the ground around you as a map, north up, with the named spots \
                 on it."
            }
            HintPoint::Contact => {
                "Hint: LOCK <contact> holds the contact your panel just named; FIRE shoots your \
                 lock."
            }
            HintPoint::Mind => {
                "Hint: SIM MIND Standard lets your tank's core fly the Registry's doctrine your \
                 way when you climb out."
            }
            HintPoint::MindSaved => {
                "Hint: @mind/prove flies your saved mind in private matches; help proving explains the paired trials."
            }
        }
    }
}

/// [`HintPoint::ALL`]'s sentences, in order — what the contract test and the
/// SSH suites read the hint lines from.
pub const HINT_LINES: [&str; 7] = [
    HintPoint::Lobby.line(),
    HintPoint::Boarding.line(),
    HintPoint::Panel.line(),
    HintPoint::Power.line(),
    HintPoint::Contact.line(),
    HintPoint::Mind.line(),
    HintPoint::MindSaved.line(),
];

/// **One account's hints**: whether they are switched off, and which points it
/// has read. At most one bool and six words — the whole of what B4's bound
/// asks of a family with no cap and no `Del` (see `store.rs`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TutorialRec {
    /// `@tutorial off`: no point is said while this is set.
    pub off: bool,
    /// Every point this account has read. Never shrinks.
    pub shown: BTreeSet<HintPoint>,
}

impl TutorialRec {
    /// The document: `{"record": "tacsim.tutorial", "account": <id>, "off":
    /// <bool>, "shown": ["lobby", …]}`, the words in point order. The account
    /// rides in the body so the loader can hold the body against its own key
    /// (the identity rule every arm of `store::load` applies).
    #[must_use]
    pub fn to_document(&self, account: AccountId) -> serde_json::Value {
        serde_json::json!({
            "record": RECORD,
            "account": account,
            "off": self.off,
            "shown": self.shown.iter().map(|point| point.as_str()).collect::<Vec<_>>(),
        })
    }

    /// Read a document back: `None` unless its `record` is [`RECORD`], its
    /// `account` parses and its `off` is a bool. A `shown` word this build
    /// does not know is dropped rather than refused — a later build's point
    /// read by an earlier one is simply not a point here.
    #[must_use]
    pub fn from_document(value: &serde_json::Value) -> Option<(AccountId, TutorialRec)> {
        if value.get("record").and_then(serde_json::Value::as_str) != Some(RECORD) {
            return None;
        }
        let account: AccountId = serde_json::from_value(value.get("account")?.clone()).ok()?;
        let off = value.get("off")?.as_bool()?;
        let shown = value
            .get("shown")
            .and_then(serde_json::Value::as_array)
            .map(|words| {
                words
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .filter_map(HintPoint::parse)
                    .collect()
            })
            .unwrap_or_default();
        Some((account, TutorialRec { off, shown }))
    }

    /// Whether `point` would be said to this account now: switched on, and not
    /// read before.
    #[must_use]
    pub fn would_show(&self, point: HintPoint) -> bool {
        !self.off && !self.shown.contains(&point)
    }
}

/// **The bare `@tutorial`'s answer**: `Hints are on; you have read 2 of 7.`
/// (or `off`). A function and not a constant, because the count is a number.
#[must_use]
pub fn read_line(off: bool, read: usize) -> String {
    let state = if off { "off" } else { "on" };
    format!(
        "Hints are {state}; you have read {read} of {}.",
        HintPoint::ALL.len()
    )
}

/// `@tutorial off`'s answer, whether or not it changed anything.
pub const SET_OFF: &str = "Hints are off: the ones you have not reached stay unsaid.";

/// `@tutorial on`'s answer, whether or not it changed anything.
pub const SET_ON: &str = "Hints are on: the ones you have read stay read.";

/// Any other word.
pub const USAGE: &str = "@tutorial [on|off]; bare says where you stand.";

/// A character the account oracle does not map (fail closed, as a claim does).
pub const NO_ACCOUNT: &str = "This character has no account on record, so it has no hints to keep.";

/// **The frame-borne points one frame earns**, in point order — pure, and read
/// off the frame's own fields alone (`power`, `team`, `weapons`, `notices`).
///
/// - `Panel` when this is the pilot's `first` frame of the match **and its
///   power is off**: a reclaim's or a resume's first frame finds the doctrine's
///   power up, and telling that pilot `STARTUP` would be telling them to do
///   what is done. It is not consumed there; it waits for a later first frame.
/// - `Power` when the frame carries `FrameNotice::PowerOnline`.
/// - `Contact` when the frame's power is `Online`, its unit carries a weapon,
///   and a `FrameNotice::Contact` on it is classified as **the other side's**.
///   The sensor pass has no team filter and no power gate, so a wingman is the
///   first contact classified in any match with one, a drone has no gun, and a
///   hostile can be told apart before `STARTUP` while `FIRE` answers `Power not
///   online: STARTUP first.` — none of the three consumes the point. The
///   contact's designator and id are never read.
#[must_use]
pub fn frame_points(first: bool, frame: &Frame) -> Vec<HintPoint> {
    let mut points = Vec::new();
    if first && frame.power == Power::Off {
        points.push(HintPoint::Panel);
    }
    if frame
        .notices
        .iter()
        .any(|notice| matches!(notice, FrameNotice::PowerOnline))
    {
        points.push(HintPoint::Power);
    }
    // Doc 32 cut CF.0: a contact notice about a row the frame holds hostile.
    let hostile = frame.notices.iter().any(|notice| match notice {
        FrameNotice::Contact { contact, .. } => frame.contacts.iter().any(|held| {
            held.id == contact.id && held.relation == crate::arena::party::Relation::Hostile
        }),
        _ => false,
    });
    if frame.power == Power::Online && !frame.weapons.is_empty() && hostile {
        points.push(HintPoint::Contact);
    }
    points
}

/// The account behind a character, or `None` — every caller fails closed on it.
fn account_of(w: &World, actor: ObjId) -> Option<AccountId> {
    w.module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.account_of(actor))
}

/// One account's record as it stands in RAM, or the empty one.
fn record_of(w: &World, account: AccountId) -> TutorialRec {
    w.module_state::<crate::TacsimStore>()
        .and_then(|store| store.tutorials.get(&account).cloned())
        .unwrap_or_default()
}

/// **Say one point to one actor, once per account** — the one entry every
/// occasion calls. Returns whether the line was said.
///
/// In this order and no other:
///
/// 1. **A live connection.** A line to a session with none is lost, and a hint
///    lost unread is a hint the person never gets — so it is neither said nor
///    marked, and its occasion recurs. `World::has_live_conns` and not
///    `is_connected`: a linkdead pilot is still `is_connected`, keeps its seat
///    through the grace and keeps receiving frames, and its doc names this use
///    (modules use it "so link-dead players never read what they never
///    received"; the composition's `econ_inbox` gates its live notice the same
///    way).
/// 2. **The layer.** With `[tacsim].enabled` off no verb a hint names works.
/// 3. **The account**, through the sim's oracle; `None` says nothing.
/// 4. **The record** in RAM, or the empty one.
/// 5. **Switched on and not read before**, else nothing.
/// 6. **Mark, persist, say**: the point joins `shown`, the record is queued on
///    the module's own lane, and the line is said on the arena's reply
///    category — never `NOTICE_SENTENCE_TAG`, which both panel rings drop.
pub fn hint(ctx: &mut Ctx<'_>, actor: ObjId, point: HintPoint) -> bool {
    if !ctx.w.has_live_conns(actor) {
        return false;
    }
    if !ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_some_and(crate::TacsimStore::enabled)
    {
        return false;
    }
    let Some(account) = account_of(ctx.w, actor) else {
        return false;
    };
    let mut rec = record_of(ctx.w, account);
    if !rec.would_show(point) {
        return false;
    }
    rec.shown.insert(point);
    crate::store::put_tutorial(ctx, account, rec);
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, point.line());
    true
}

/// **The lobby point's occasion**: a Player moved into the Simulator Lobby
/// room, by any route — an exit, `home`, `@teleport` — since the kernel emits
/// `WorldEvent::Moved` for each, after the mover's own room view.
///
/// The gates, cheapest first: a Player (one object read); the lobby (one
/// content-latch probe); a centre in which `claim` would be taken — `Idle`,
/// `Selection` or terminal, the states `lobby::claim` folds or accepts, so a
/// player walking in during `Boarding`, `Running` or `Stopping` keeps the point
/// for a later entry; an account (fail closed); and **no slot of that
/// account's own** — a slot holder walking back in from the team room or from
/// Limbo, or an alt of one, would be told to `SIM TEAM` and refused `You
/// already hold red slot 1.`, so the point waits for an entry after `SIM LEAVE`
/// or the match's end. Then [`hint`], whose own steps cover the session and the
/// layer. A centre whose every pod is taken is not gated (the plan's residual).
pub struct LobbyEntry;

impl WorldObserver for LobbyEntry {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        let WorldEvent::Moved { who, to, .. } = ev else {
            return;
        };
        if ctx.w.get(*who).map(|object| object.kind) != Some(ObjKind::Player) {
            return;
        }
        if centre::centre_object(ctx.w, centre::LOBBY_ENTRY) != Some(*to) {
            return;
        }
        if !lobby::centre(ctx.w).is_none_or(|c| {
            matches!(c.state, MatchState::Idle | MatchState::Selection) || c.state.is_terminal()
        }) {
            return;
        }
        let Some(account) = account_of(ctx.w, *who) else {
            return;
        };
        if lobby::centre(ctx.w)
            .and_then(|c| c.slot_of_account(account))
            .is_some()
        {
            return;
        }
        hint(ctx, *who, HintPoint::Lobby);
    }
}

/// **The verb's one word, folded**: `Some(None)` for the bare read,
/// `Some(Some(off))` for a switch, `None` for anything else (the usage line).
/// Trimmed and case-folded, so `OFF` and ` On ` are the switch.
fn switch_of(raw: &str) -> Option<Option<bool>> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => Some(None),
        "off" => Some(Some(true)),
        "on" => Some(Some(false)),
        _ => None,
    }
}

/// **`@tutorial [on|off]`**: bare reads where the account stands, `off` stops
/// the points not yet reached, `on` starts them again and re-arms nothing read.
///
/// The account first: a character the oracle does not map is told so and
/// nothing else happens. A switch that changes nothing still answers its
/// sentence and writes nothing; one that changes `off` writes the record.
/// Every answer is `Sys` on the arena's reply category.
///
/// # Errors
///
/// None: every answer, the usage line included, is a line said to the actor.
pub fn tutorial_command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Some(account) = account_of(ctx.w, actor) else {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, NO_ACCOUNT);
        return Ok(());
    };
    let mut rec = record_of(ctx.w, account);
    let (off, answer) = match switch_of(ctx.args.raw()) {
        Some(None) => {
            let line = read_line(rec.off, rec.shown.len());
            ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
            return Ok(());
        }
        Some(Some(true)) => (true, SET_OFF),
        Some(Some(false)) => (false, SET_ON),
        None => {
            ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, USAGE);
            return Ok(());
        }
    };
    if rec.off != off {
        rec.off = off;
        crate::store::put_tutorial(ctx, account, rec);
    }
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, answer);
    Ok(())
}

/// Register `@tutorial` and the lobby's `Moved` observer on the **tacsim**
/// module. Called from `TacsimRuleset::register` under the restored
/// `LegacyPostImage` classification: the verb's one write is a bounded
/// whole-document record on the module's own lane.
///
/// # Errors
///
/// [`ModError`] if the verb name is already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: TUTORIAL_VERB.into(),
            shape: ArgShape::Raw,
            help: "@tutorial [on|off]  --  The simulator says one line at six points of your \
                   first match, naming the verb to type next: walking into the lobby, when \
                   boarding opens, when the panel wakes, when power comes up, when your \
                   sensors first tell an enemy apart, and when the match is over. Each is said \
                   once per account, so an alt does not read them again. @tutorial off stops \
                   the ones you have not reached; @tutorial on starts them again, and the ones \
                   you have read stay read; bare @tutorial says where you stand."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(tutorial_command),
    )?;
    reg.subscribe_moved("tutorial_lobby_entry", 1, Arc::new(LobbyEntry))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::arena::perception::{Named, NoticeKind};
    use crate::arena::state::{Classification, ContactId, Designator, PartyId};
    use crate::arena::test_support::{arena_at_epoch, pose, scout, tank};

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);

    fn account(n: i64) -> AccountId {
        AccountId::mint(&format!("account-{n}"), n).expect("an account id")
    }

    /// Every sentence opens with the prefix, fits the floor, and names no
    /// thing: no number, no object, no designator, no unit, and a side only
    /// inside the three phrases that name a side as a word to type or a room.
    ///
    /// The widths were `[91, 82, 59, 86, 84]` at 16b cut AH.1; the sixth,
    /// `mind`'s, adds 102 (17b cut M5.6, the design note's item 2: "102
    /// columns, no digit, no `Tank`, `#`, `X1`, `BA` or `RA`").
    #[test]
    fn ah1_every_line_opens_with_the_prefix_and_names_no_thing() {
        assert_eq!(HINT_LINES.len(), HintPoint::ALL.len());
        for (point, line) in HintPoint::ALL.iter().zip(HINT_LINES) {
            assert_eq!(point.line(), line);
            assert!(line.starts_with(HINT_PREFIX), "{line}");
            assert!(line.chars().count() < 120, "{line}");
            assert!(!line.chars().any(|c| c.is_ascii_digit()), "{line}");
            for word in ["#", "X1", "BA", "RA", "Tank"] {
                assert!(!line.contains(word), "{word} in {line}");
            }
            let mut rest = line.to_owned();
            for phrase in [
                "SIM TEAM red",
                "SIM TEAM blue",
                "red or blue from the lobby",
            ] {
                rest = rest.replace(phrase, "");
            }
            assert!(
                !rest
                    .split(|c: char| !c.is_alphabetic())
                    .any(|word| word.eq_ignore_ascii_case("red")
                        || word.eq_ignore_ascii_case("blue")),
                "{line}"
            );
        }
        assert_eq!(
            HINT_LINES.map(|line| line.chars().count()),
            [91, 82, 59, 86, 84, 102, 100]
        );
    }

    #[test]
    fn ah1_the_record_round_trips_through_its_document() {
        let rec = TutorialRec {
            off: true,
            shown: [HintPoint::Lobby, HintPoint::Power].into_iter().collect(),
        };
        let document = rec.to_document(account(1));
        assert_eq!(document["record"], RECORD);
        assert_eq!(document["shown"], serde_json::json!(["lobby", "power"]));
        assert_eq!(
            TutorialRec::from_document(&document),
            Some((account(1), rec.clone()))
        );
        let mut wrong = document.clone();
        wrong["record"] = serde_json::json!("tacsim.pin");
        assert_eq!(TutorialRec::from_document(&wrong), None);
        let mut later = document.clone();
        later["shown"] = serde_json::json!(["lobby", "debrief", "power"]);
        assert_eq!(
            TutorialRec::from_document(&later),
            Some((account(1), rec)),
            "an unknown word is dropped, not refused"
        );
    }

    #[test]
    fn ah1_the_loader_applies_the_identity_rule() {
        let rec = TutorialRec {
            off: false,
            shown: [HintPoint::Lobby].into_iter().collect(),
        };
        let other = TutorialRec {
            off: true,
            shown: HintPoint::ALL.into_iter().collect(),
        };
        let kv = vec![
            // Account 1's own record under its own key: kept.
            (tutorial_key(account(1)), rec.to_document(account(1))),
            // Account 3's body under account 2's key, and account 3 has no
            // key of its own: neither the key's account nor the body's may
            // gain a record. Without the rule, account 3 would.
            (tutorial_key(account(2)), other.to_document(account(3))),
            // A different body for account 1 under account 4's key, AFTER the
            // matching one: without the rule it would overwrite account 1.
            (tutorial_key(account(4)), other.to_document(account(1))),
            // A body that does not parse.
            (
                tutorial_key(account(5)),
                serde_json::json!({ "record": RECORD, "account": 7 }),
            ),
        ];
        let store = crate::store::load(&kv, crate::config::TacsimConfig::default());
        assert_eq!(
            store.tutorials.get(&account(1)),
            Some(&rec),
            "a body under another account's key never overwrites the owner's own"
        );
        for n in [2, 3, 4, 5] {
            assert!(
                !store.tutorials.contains_key(&account(n)),
                "account {n}: a misfiled or unparseable body is skipped"
            );
        }
        assert_eq!(store.tutorials.len(), 1);
    }

    /// Red's opening frame of a stepped scout-and-tank arena: a real `Frame`,
    /// whose `weapons` is the scout's cannon.
    fn red_frame() -> Frame {
        let mut arena = arena_at_epoch(
            vec![
                scout(RED, PartyId::RED, pose(1_500, 1_500)),
                tank(BLUE, PartyId::BLUE, pose(6_500, 6_500)),
            ],
            1,
        );
        let out = arena.step(0, &[], &[]);
        let frame = out
            .frames
            .into_iter()
            .map(|(_, frame)| frame)
            .find(|frame| frame.unit == RED)
            .expect("red is sent its opening frame");
        assert!(!frame.weapons.is_empty(), "the scout carries its cannon");
        assert_eq!(frame.party, PartyId::RED);
        frame
    }

    fn classified(team: PartyId) -> FrameNotice {
        FrameNotice::Contact {
            kind: NoticeKind::Classified,
            contact: Named {
                id: ContactId(1),
                designator: Designator::Unit { party: team, letter: 'A' },
            },
            class: Classification::Classified { party: team },
            was: None,
        }
    }

    /// The frame at `power` carrying `notices`, and (doc 32 cut CF.0) a held
    /// row for each contact notice, related as the OOC host relates it: its
    /// own party own, any other hostile.
    fn with(frame: &Frame, power: Power, notices: Vec<FrameNotice>) -> Frame {
        let mut frame = frame.clone();
        frame.power = power;
        for notice in &notices {
            if let FrameNotice::Contact { contact, class, .. } = notice {
                let relation = match class.party() {
                    None => crate::arena::party::Relation::Unknown,
                    Some(party) if party == frame.party => crate::arena::party::Relation::Own,
                    Some(_) => crate::arena::party::Relation::Hostile,
                };
                frame.contacts.push(crate::arena::perception::FrameContact {
                    id: contact.id,
                    designator: contact.designator,
                    alias: None,
                    class: *class,
                    relation,
                    moving: false,
                    bearing_mdeg: 0,
                    range_cm: 10_000,
                    age_ms: 0,
                    cell: None,
                    condition: None,
                    aspect: crate::arena::perception::Aspect::Front,
                    facing: crate::arena::perception::ContactFacing::Front,
                    reads_dead: None,
                    fatal: None,
                    unit_type: None,
                });
            }
        }
        frame.notices = notices;
        frame
    }

    #[test]
    fn ah1_frame_points_reads_first_power_a_gun_and_a_hostile_classified_contact_in_order() {
        let base = red_frame();
        assert_eq!(
            frame_points(true, &with(&base, Power::Off, vec![])),
            vec![HintPoint::Panel]
        );
        assert_eq!(
            frame_points(true, &with(&base, Power::Online, vec![])),
            vec![],
            "a reclaim's first frame finds power up"
        );
        assert_eq!(
            frame_points(false, &with(&base, Power::Off, vec![])),
            vec![]
        );
        assert_eq!(
            frame_points(
                false,
                &with(&base, Power::Online, vec![FrameNotice::PowerOnline])
            ),
            vec![HintPoint::Power]
        );
        assert_eq!(
            frame_points(
                false,
                &with(&base, Power::Online, vec![classified(PartyId::BLUE)])
            ),
            vec![HintPoint::Contact]
        );
        assert_eq!(
            frame_points(
                false,
                &with(&base, Power::Online, vec![classified(PartyId::RED)])
            ),
            vec![],
            "a wingman told apart is no enemy"
        );
        let mut gunless = with(&base, Power::Online, vec![classified(PartyId::BLUE)]);
        gunless.weapons.clear();
        assert_eq!(frame_points(false, &gunless), vec![], "no gun to FIRE");
        assert_eq!(
            frame_points(
                false,
                &with(&base, Power::Off, vec![classified(PartyId::BLUE)])
            ),
            vec![],
            "FIRE would answer Power not online"
        );
        assert_eq!(
            frame_points(
                false,
                &with(&base, Power::Starting, vec![classified(PartyId::BLUE)])
            ),
            vec![]
        );
        let acquired = FrameNotice::Contact {
            kind: NoticeKind::Acquired,
            contact: Named {
                id: ContactId(2),
                designator: Designator::Provisional(1),
            },
            class: Classification::Unknown,
            was: None,
        };
        assert_eq!(
            frame_points(false, &with(&base, Power::Online, vec![acquired])),
            vec![]
        );
        assert_eq!(
            frame_points(
                true,
                &with(
                    &base,
                    Power::Online,
                    vec![FrameNotice::PowerOnline, classified(PartyId::BLUE)]
                )
            ),
            vec![HintPoint::Power, HintPoint::Contact]
        );
    }

    #[test]
    fn ah1_the_module_head_names_every_point() {
        let source = include_str!("tutorial.rs");
        for point in HintPoint::ALL {
            assert!(
                source.contains(&format!("//! {}", point.as_str())),
                "{}",
                point.as_str()
            );
        }
    }

    #[test]
    fn ah1_the_verb_words_fold() {
        assert_eq!(switch_of("on"), Some(Some(false)));
        assert_eq!(switch_of("OFF"), Some(Some(true)));
        assert_eq!(switch_of(" On "), Some(Some(false)));
        assert_eq!(switch_of("  "), Some(None), "bare reads");
        assert_eq!(switch_of("onn"), None, "the usage arm");
        assert_eq!(switch_of("off please"), None, "the usage arm");
        assert_eq!(HintPoint::parse("contact"), Some(HintPoint::Contact));
        assert_eq!(HintPoint::parse("Contact"), None, "stored words are exact");
        assert_eq!(
            tutorial_key(account(1))
                .strip_prefix(KEY_PREFIX)
                .map(str::len),
            Some(64),
            "the key's tail is the account's canonical hash in its string form"
        );
    }

    /// Was `… 2 of 5.` / `… 5 of 5.` at 16b cut AH.1; the table holds six since
    /// `HintPoint::Mind` (17b cut M5.6), and the count is the table's.
    #[test]
    fn ah1_read_line_counts_out_of_the_table() {
        assert_eq!(read_line(false, 2), "Hints are on; you have read 2 of 7.");
        assert_eq!(read_line(true, 6), "Hints are off; you have read 6 of 7.");
    }

    /// **17b cut M5.6**: the sixth point is the mind's — its word, its place
    /// last in the table, and its sentence naming `SIM MIND` and nothing a
    /// first match has not shown (no rule: the rule language is a later cut).
    #[test]
    fn m5_the_sixth_hint_names_sim_mind() {
        assert_eq!(HintPoint::ALL.get(5), Some(&HintPoint::Mind));
        assert_eq!(HintPoint::ALL.last(), Some(&HintPoint::MindSaved));
        assert_eq!(HintPoint::parse("mind_saved"), Some(HintPoint::MindSaved));
        assert_eq!(HintPoint::parse("mind"), Some(HintPoint::Mind));
        assert_eq!(
            HINT_LINES[5],
            "Hint: SIM MIND Standard lets your tank's core fly the Registry's doctrine your way \
             when you climb out."
        );
        assert!(!HINT_LINES[5].contains("rule"), "{}", HINT_LINES[5]);
    }
}
