//! **The mind market, walked by two players** (doc 44 unit MW.19; Q-44-33): the one walk of
//! the market's whole path as a player makes it, over SSH, on a world brought up as the launch
//! brings a live server up. Written from the player help (`help mindcraft`, `help
//! personal-minds`, `help simulator`, `help mining`, `help coop`), as the rest of the lane is.
//!
//! Two roles, each its own bot on its own login (a sale between two characters of one login is
//! refused):
//!
//! **The author** (`--minds author`). The runner has already written the mind: before this
//! lane's first line it pushed the bot's own source over the SSH mind service (`mind push`,
//! `mind check`; [`crate::transport::mind_exec`]) and holds the saved revision. The lane then
//! enrols with the Co-op at the Registry's desk (the only way a new character has a credit
//! account; the filing fee is paid from the stake), and
//!
//! 1. reserves a charter: `@mind/file <mind>@<rev>=<charter>`, confirmed by the engine's own
//!    `Done.`;
//! 2. asks for the rating, `@mind/rate <mind>@<rev>`, and waits for `Registry work finished.`,
//!    counting every line the Registry sends it meanwhile that it did not ask for;
//! 3. files: the same `@mind/file` line again, until the Registry has released its job;
//! 4. reads its filed number from `+minds/holdings`;
//! 5. walks through the counter's door, lists the mind sealed (`@mind/publish <filed>
//!    form=sealed`) and reads its own row on `+minds`.
//!
//! **The buyer** (`--minds buyer`). The mining pilot's kit-up with the escort's lease (the
//! armed rig is the one Occator leases with a core; a prospector carries none), the bunkers
//! for the hop, and then
//!
//! 1. to the counter, where it reads `+minds` until the author's charter is on it;
//! 2. the purchase, three confirmed steps: `+minds/buy <code>`, `+minds/buy/pay <code>`,
//!    `+minds/buy/complete <code>`; its licence number from `+minds/holdings`;
//! 3. the simulator: out of character to the centre, `SIM TEAM red`, `SIM MIND
//!    filed:<licence>`, `SIM START`, the pod, where the seal names the mind before `Good
//!    hunting.`; `MIND` reads it; `EXIT` hands the tank to it; the match's end; `SIM REPORT`'s
//!    `Your mind` line;
//! 4. back in character, aboard, the hop to the site, and `+tv/1 launch mind=filed:<licence>
//!    go` from the bay; `+tv/1` until the rig stands at its ramp; `+tv/1 recover go`; `+tv/1`
//!    until it is in the bay; the hop home.
//!
//! **What it asserts on** is the game's own sentences, through the bot's table
//! ([`crate::mine::minds`]): no step reads a store, a log or a staff view. A step whose answer
//! is not the sentence it waits for runs its deadline out and fails quoting what did arrive.
//!
//! **What it leaves out**, because a player has no way to do it on a launch world today:
//! nothing of the list above. Two things the lane cannot see are named where they are met:
//! the launch's success tells the operator nothing about the mind sealed aboard (only a lapsed
//! licence is spoken of), and with no field on the site the rig's mind never thinks, so "flies
//! it on an IC sortie" is the launch under the licence, the rig down the ramp, and home.
//!
//! It types only [`in_vocabulary`]: two `@mind/` verbs of the player's own workshop and
//! market, and no staff verb.
use serde::{Deserialize, Serialize};

use super::{Beat, Kind, Lane, Verdict, lines};
use crate::arena::Side;
use crate::arena::lines::{Line as ArenaLine, ResultWord, classify};
use crate::arena::lobby::LobbyLine;
use crate::mine::minds::{claim, dispatch, market as words, walk};

/// The side the buyer claims in the simulator.
pub const SIDE: Side = Side::Red;
/// **The counter's room**, by the name the launch's bring-up digs it under
/// (`deploy/instance/launch-bringup.py`).
pub const COUNTER_ROOM: &str = "Named launch counter";
/// The word that walks through the counter's door from the Registry's desk (the bring-up's
/// `Mind Counter <MC>`).
pub const COUNTER_DOOR: &str = "mc";
/// The simulator centre's lobby.
pub const ROOM_SIMULATOR: &str = "Simulator Lobby";
/// The out-of-character hall the simulator opens off.
pub const ROOM_HALL: &str = "Welcome Hall";
/// Limbo, between the story and the out-of-character area.
pub const ROOM_LIMBO: &str = "Limbo";
/// The red side's pod room.
pub const ROOM_POD: &str = "Red Room";
/// A read that has not shown its row yet is typed again this often.
pub const POLL_MS: u64 = 3_000;
/// A confirmed step's refusal or busy answer is typed again this many times.
pub const SUBMIT_TRIES: u32 = 10;
/// The buyer reads the counter this often while the author's line is not on it.
pub const BROWSE_EVERY_MS: u64 = 20_000;
/// **The longest a rating is waited for**, and so the longest the buyer reads the counter:
/// two hours. A rating is about half an hour on a dev box's pulse.
pub const RATING_MS: u64 = 2 * 60 * 60_000;
/// The buyer's reads of the counter inside [`RATING_MS`].
pub const BROWSE_TRIES: u32 = 400;
/// The filing typed again while the Registry still holds its finished job (a pulse or two).
pub const FILE_TRIES: u32 = 20;
/// A pod's boarding window (two minutes when nobody else is to be seated) and slack.
pub const BOARDING_MS: u64 = 4 * 60_000;
/// **A match nobody is seated in** is ended by the centre after five minutes
/// (`[tacsim].unattended_ms`); the lane waits twice that.
pub const MATCH_MS: u64 = 10 * 60_000;
/// How many quotes the buyer asks for before it gives up on paying one.
pub const QUOTE_TRIES: u32 = 3;

/// **The library name the author's mind is written under** (the game stores names lower-case).
pub const MIND_NAME: &str = "sentry";
/// **The author's mind**: the bot's own source, pushed over the SSH mind service.
pub const MIND_SOURCE: &str = include_str!("market_mind.rhai");

/// What the runner wrote over the SSH mind service before the lane began.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// The library name.
    pub name: String,
    /// The saved revision's number: what `@mind/rate` and `@mind/file` name.
    pub revision: u64,
    /// The saved revision's digest, as the service printed it.
    pub digest: String,
    /// How many of the check's pictures ran, none with a fault.
    pub pictures: usize,
}

// -- the SSH mind service's requests and replies (`help mindcraft`, "Source over SSH") --------
//
// The bot writes and reads these as JSON of its own, from the help's own description: one
// object with an `op`, and the `base` a reply carries sent back with the next change.

/// `mind push`: a new, empty draft called `name`.
#[must_use]
pub fn create_request(name: &str) -> serde_json::Value {
    serde_json::json!({"op": "create", "name": name, "from": null})
}

/// `mind pull`: the working copy of `name`.
#[must_use]
pub fn read_request(name: &str) -> serde_json::Value {
    serde_json::json!({"op": "read", "name": name, "revision": null})
}

/// `mind push`: the working copy replaced by `draft` with `source` as its source.
#[must_use]
pub fn replace_request(
    name: &str,
    base: &serde_json::Value,
    draft: &serde_json::Value,
    source: &str,
) -> serde_json::Value {
    let mut draft = draft.clone();
    if let Some(fields) = draft.as_object_mut() {
        fields.insert("source".to_owned(), source.into());
    }
    serde_json::json!({"op": "replace", "name": name, "base": base, "draft": draft})
}

/// `mind check`: the six pictures over the working copy at `base`.
#[must_use]
pub fn check_request(name: &str, base: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({"op": "check", "name": name, "base": base})
}

/// `mind push`: the working copy at `base` saved as a revision.
#[must_use]
pub fn save_request(name: &str, base: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "op": "save",
        "name": name,
        "base": base,
        "notes": "the minds lane",
        "version": null
    })
}

/// A reply that is not the status wanted, as a failure quoting the service.
fn unexpected(what: &str, reply: &serde_json::Value) -> String {
    let said = reply
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| reply.to_string(), str::to_owned);
    format!(
        "the SSH mind service did not {what}: {}",
        crate::tail(&said, 300)
    )
}

/// Whether `reply` carries `status`.
fn status_is(reply: &serde_json::Value, status: &str) -> bool {
    reply.get("status").and_then(serde_json::Value::as_str) == Some(status)
}

/// A read's `base` and `draft`.
pub fn read_reply(
    reply: &serde_json::Value,
) -> Result<(serde_json::Value, serde_json::Value), String> {
    match (
        status_is(reply, "read"),
        reply.get("base"),
        reply.get("draft"),
    ) {
        (true, Some(base), Some(draft)) => Ok((base.clone(), draft.clone())),
        _ => Err(unexpected("read the draft", reply)),
    }
}

/// A draft's source.
#[must_use]
pub fn draft_source(draft: &serde_json::Value) -> Option<&str> {
    draft.get("source").and_then(serde_json::Value::as_str)
}

/// A replace's new `base`.
pub fn edited_reply(reply: &serde_json::Value) -> Result<serde_json::Value, String> {
    match (status_is(reply, "edited"), reply.get("base")) {
        (true, Some(base)) => Ok(base.clone()),
        _ => Err(unexpected("take the source", reply)),
    }
}

/// **A check that found nothing**: how many pictures ran. A diagnostic (the source does not
/// compile) or a picture that faulted is a failure quoting it.
pub fn checked_reply(reply: &serde_json::Value) -> Result<usize, String> {
    if !status_is(reply, "checked") {
        return Err(unexpected("check the draft", reply));
    }
    let list = |key: &str| {
        reply
            .get(key)
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    if let Some(diagnostic) = list("diagnostics").first() {
        return Err(format!("the check refused the source: {diagnostic}"));
    }
    let reports = list("reports");
    if let Some(faulted) = reports
        .iter()
        .find(|report| report.get("fault").is_some_and(|fault| !fault.is_null()))
    {
        return Err(format!("a check picture faulted: {faulted}"));
    }
    if reports.is_empty() {
        return Err("the check ran no picture".to_owned());
    }
    Ok(reports.len())
}

/// A save's revision number and digest.
pub fn saved_reply(reply: &serde_json::Value) -> Result<(u64, String), String> {
    let revision = reply.get("revision").and_then(serde_json::Value::as_u64);
    let digest = reply.get("digest").and_then(serde_json::Value::as_str);
    match (status_is(reply, "saved"), revision, digest) {
        (true, Some(revision), Some(digest)) => Ok((revision, digest.to_owned())),
        _ => Err(unexpected("save the revision", reply)),
    }
}

/// Which half of the walk this bot plays.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Files `mind`@`revision`, the revision the runner pushed and saved over SSH.
    Author { mind: String, revision: u64 },
    /// Buys a licence and flies it.
    Buyer,
}

/// What the walk did, in the game's own lines: the report's `market` block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// `author` or `buyer`.
    pub role: String,
    /// The charter both roles name.
    pub charter: String,
    /// The author's mind as filed: `name@revision`.
    pub reference: Option<String>,
    /// The charter's reservation, confirmed.
    pub reserved: bool,
    /// When the rating was asked for and when the Registry said it had finished, lane ms.
    pub rating_asked_ms: Option<u64>,
    pub rating_finished_ms: Option<u64>,
    /// **Lines the Registry sent the author during the rating that answer nothing the author
    /// typed**: the engine's `Queued.` and `Done.` for the Registry's own steps.
    pub rating_unasked_lines: u32,
    /// The filing's line, confirmed.
    pub filing: Option<String>,
    /// The filed number, from the author's `+minds/holdings`.
    pub filed: Option<u64>,
    /// The listing's line, confirmed, and the row `+minds` shows for it.
    pub publishing: Option<String>,
    pub row: Option<String>,
    /// The listing's code, from the row.
    pub code: Option<String>,
    /// How many times the buyer read the counter before the row was on it.
    pub browses: u32,
    /// The purchase's three confirmed lines.
    pub quoted: Option<String>,
    pub paid: Option<String>,
    pub taken: Option<String>,
    /// The buyer's holding row and its licence number.
    pub holding: Option<String>,
    pub licence: Option<u64>,
    /// The simulator: the choice's answer, the match, the seal line read in the pod, the
    /// cockpit's `MIND` head, the hand-over at `EXIT`, how the match ended, and `SIM REPORT`'s
    /// `Your mind` line.
    pub chosen: Option<String>,
    pub match_id: Option<u64>,
    pub sealed: Option<String>,
    pub cockpit: Option<String>,
    pub left_to_mind: Option<String>,
    pub match_end: Option<String>,
    pub report: Option<String>,
    /// `SIM REPORT`'s line that the mind flew the tank, and from when.
    pub flew: Option<String>,
    /// The sortie: the launch's line, `+tv/1` with the rig on the ground, and in the bay.
    pub launched: Option<String>,
    pub aground: Option<String>,
    pub in_bay: Option<String>,
}

/// A walk of several rooms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// The author, from the Registry's desk through the counter's door.
    Desk,
    /// The buyer, from the chandlery to the counter.
    Counter,
    /// From the counter out of character to the simulator's lobby.
    Simulator,
    /// From the lobby to the pod room.
    Pod,
    /// From the pod room back into character, to Arrivals.
    Port,
}

impl Route {
    /// The route's walks: the word typed and the room it arrives in.
    fn steps(self) -> Vec<(String, &'static str)> {
        let own = |word: &str, room: &'static str| (word.to_owned(), room);
        match self {
            Route::Desk => vec![own(COUNTER_DOOR, COUNTER_ROOM)],
            Route::Counter => vec![
                own("exchange", lines::ROOM_EXCHANGE),
                own("concourse", lines::ROOM_CONCOURSE),
                own("arrivals", lines::ROOM_ARRIVALS),
                own("registry", lines::ROOM_REGISTRY),
                own(COUNTER_DOOR, COUNTER_ROOM),
            ],
            Route::Simulator => vec![
                own("out", lines::ROOM_REGISTRY),
                own("out", lines::ROOM_ARRIVALS),
                own("limbo", ROOM_LIMBO),
                (LobbyLine::Ooc.line(), ROOM_HALL),
                (LobbyLine::Simulator.line(), ROOM_SIMULATOR),
            ],
            Route::Pod => vec![(LobbyLine::TeamRoom(SIDE).line(), ROOM_POD)],
            Route::Port => vec![
                own("lobby", ROOM_SIMULATOR),
                own("hall", ROOM_HALL),
                own("limbo", ROOM_LIMBO),
                own("c", lines::ROOM_ARRIVALS),
            ],
        }
    }
}

/// A market step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// One walk of a route.
    Go(Route, usize),
    // -- the author
    Reserve,
    Rate,
    /// Follow-on: the Registry's last word on the rating.
    Rating,
    File,
    Owned,
    Publish,
    Listed,
    // -- the buyer
    Browse,
    Quote,
    Pay,
    Complete,
    Held,
    SimTeam,
    SimMind,
    SimStart,
    SimBoard,
    Cockpit,
    SimExit,
    /// Follow-on: the match's end, heard in the pod room.
    SimOver,
    SimReport,
    Launch,
    Aground,
    Recover,
    InBay,
}

/// One market step as the lane's window judges it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Task {
    pub phase: Phase,
}

/// The walk's state.
pub(super) struct Market {
    role: Role,
    charter: String,
    filed: Option<u64>,
    code: Option<String>,
    licence: Option<u64>,
    match_id: Option<u64>,
    quotes: u32,
    /// The buyer holds its licence and has flown the simulator: the kit-up's next walk is the
    /// mining pilot's own.
    flown: bool,
}

impl Market {
    pub(super) fn new(role: Role, charter: String) -> Market {
        Market {
            role,
            charter,
            filed: None,
            code: None,
            licence: None,
            match_id: None,
            quotes: 0,
            flown: false,
        }
    }

    /// The author's `name@revision`.
    fn reference(&self) -> Option<String> {
        match &self.role {
            Role::Author { mind, revision } => Some(format!("{mind}@{revision}")),
            Role::Buyer => None,
        }
    }
}

/// **A charter's name as the lane takes it**: 1 to 40 letters, digits and single spaces,
/// starting with a letter (the game allows 80 bytes of anything printable; the lane's own
/// rule keeps it one token run on every row it is read back from).
pub fn charter_ok(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && !name.ends_with(' ')
        && !name.contains("  ")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ')
}

/// **A mind's library name as the lane takes it**: 1 to 24 lower-case letters and digits,
/// starting with a letter (the game stores a name lower-case).
pub fn mind_ok(name: &str) -> bool {
    (1..=24).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// **The walk's lines**, each in closed form, beside the mining pilot's and the escort's:
///
/// - `@mind/file <mind>@<rev>=<charter>` and `@mind/rate <mind>@<rev>` ([`mind_ok`],
///   [`charter_ok`], `<rev>` a decimal number): the only lines beginning `@`;
/// - `@mind/publish <filed> form=sealed`;
/// - `+minds`, `+minds/holdings`, `+minds/buy <code>`, `+minds/buy/pay <code>`,
///   `+minds/buy/complete <code>`, `<code>` 8 letters and digits;
/// - the walks `mc`, `out`, `limbo`, `hall`, `lobby`, `c`, `registry`, `exchange`,
///   `concourse`, `arrivals`, and the lobby's own `ooc`, `simulator`, `red`;
/// - the centre's `SIM TEAM red`, `SIM MIND filed:<licence>`, `SIM START`, `BOARD`, and the
///   three the arena pilot never types: `MIND`, `EXIT`, `SIM REPORT`;
/// - `+tv/1 launch mind=filed:<licence> go`, `+tv/1`, `+tv/1 recover go`.
#[must_use]
pub fn in_vocabulary(line: &str) -> bool {
    if line.len() > 120 || line.contains(['\n', '\r']) {
        return false;
    }
    let number = |text: &str| {
        (1..=18).contains(&text.len())
            && text.bytes().all(|b| b.is_ascii_digit())
            && !text.starts_with('0')
    };
    let reference = |text: &str| {
        text.split_once('@')
            .is_some_and(|(mind, revision)| mind_ok(mind) && number(revision))
    };
    let code = |text: &str| text.len() == 8 && text.bytes().all(|b| b.is_ascii_alphanumeric());
    if let Some(rest) = line.strip_prefix("@mind/file ") {
        return rest
            .split_once('=')
            .is_some_and(|(mind, charter)| reference(mind) && charter_ok(charter));
    }
    if let Some(rest) = line.strip_prefix("@mind/rate ") {
        return reference(rest);
    }
    if let Some(rest) = line.strip_prefix("@mind/publish ") {
        return rest.strip_suffix(" form=sealed").is_some_and(number);
    }
    if line.starts_with('@') {
        return false;
    }
    for verb in ["+minds/buy/complete ", "+minds/buy/pay ", "+minds/buy "] {
        if let Some(rest) = line.strip_prefix(verb) {
            return code(rest);
        }
    }
    if let Some(rest) = line.strip_prefix("SIM MIND filed:") {
        return number(rest)
            && rest
                .parse()
                .is_ok_and(|n| LobbyLine::SimMind(n).line() == line);
    }
    if let Some(rest) = line
        .strip_prefix("+tv/1 launch mind=filed:")
        .and_then(|rest| rest.strip_suffix(" go"))
    {
        return number(rest);
    }
    let lobby = [
        LobbyLine::Ooc,
        LobbyLine::Simulator,
        LobbyLine::TeamRoom(SIDE),
        LobbyLine::SimTeam(SIDE),
        LobbyLine::SimStart,
        LobbyLine::Board,
    ];
    lobby.iter().any(|word| word.line() == line)
        || matches!(
            line,
            "+minds"
                | "+minds/holdings"
                | "mc"
                | "out"
                | "limbo"
                | "hall"
                | "lobby"
                | "c"
                | "registry"
                | "exchange"
                | "concourse"
                | "arrivals"
                | "MIND"
                | "EXIT"
                | "SIM REPORT"
                | "+tv/1"
                | "+tv/1 recover go"
        )
}

/// A line that is exactly the engine's `Done.`: the confirmation of a submitted step.
fn confirmed(raw: &[String]) -> bool {
    raw.iter().any(|line| line.trim() == walk::DONE)
}

/// A line that is exactly `Queued.` or `Done.`.
fn engine_word(line: &str) -> bool {
    let body = line.trim();
    body == walk::QUEUED || body == walk::DONE
}

/// **The filed number of the author's own row** in `+minds/holdings`: `  filed <n>  owner
/// <name>  …`, the row whose name is the charter's (a name is cut to twenty characters there,
/// and printed lower-case).
#[must_use]
pub fn owner_row(raw: &[String], charter: &str) -> Option<(u64, String)> {
    holding_row(raw, "filed", "owner", charter)
}

/// **The licence number of the buyer's paid row** in `+minds/holdings`: `  licence <n>  paid
/// <name>  <form>  …`.
#[must_use]
pub fn paid_row(raw: &[String], charter: &str) -> Option<(u64, String)> {
    holding_row(raw, "licence", "paid", charter)
}

/// **A table row's columns**: the line split on runs of two or more spaces. The market's
/// tables pad every column and put two spaces between them, and a charter the lane files has
/// no double space in it ([`charter_ok`]), so a name is one column however many words it has.
fn columns(line: &str) -> Vec<&str> {
    line.split("  ")
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .collect()
}

/// A holdings row after the table's head: `<word> <number>`, its kind, and a name that is
/// `charter` as the table prints it (cut to twenty characters).
fn holding_row(raw: &[String], word: &str, kind: &str, charter: &str) -> Option<(u64, String)> {
    let start = raw
        .iter()
        .position(|line| line.contains(words::HOLD_HEAD))?;
    let wanted = name_start(charter, 20);
    raw[start..].iter().find_map(|line| {
        let columns = columns(line);
        let number = columns
            .first()?
            .strip_prefix(word)?
            .strip_prefix(' ')?
            .trim()
            .parse()
            .ok()?;
        (columns.get(1) == Some(&kind) && columns.get(2)?.to_lowercase() == wanted)
            .then(|| (number, line.trim().to_owned()))
    })
}

/// A name as a table prints its start: lower-case, at most `width` characters, trimmed.
fn name_start(name: &str, width: usize) -> String {
    name.to_lowercase()
        .chars()
        .take(width)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// **The counter's row for `charter`**: a line after `+minds`' head that holds the table's
/// `  class ` gap (spaces not collapsed) and whose name column is the charter (cut to
/// twenty-four characters). Its first column is the listing's code.
#[must_use]
pub fn listing_row(raw: &[String], charter: &str) -> Option<(String, String)> {
    let start = raw.iter().position(|line| line.contains(words::HEAD))?;
    let wanted = name_start(charter, 24);
    raw[start..].iter().find_map(|line| {
        if !line.contains(words::ROW) {
            return None;
        }
        let columns = columns(line);
        let code = *columns.first()?;
        (code.len() == 8
            && code.bytes().all(|b| b.is_ascii_alphanumeric())
            && columns.get(1)?.to_lowercase() == wanted)
            .then(|| (code.to_owned(), line.trim().to_owned()))
    })
}

/// The first line of `raw` that holds `needle`, trimmed.
fn line_with(raw: &[String], needle: &str) -> Option<String> {
    raw.iter()
        .find(|line| line.contains(needle))
        .map(|line| line.trim().to_owned())
}

impl Lane {
    /// Whether this lane is the author's.
    pub(super) fn is_author(&self) -> bool {
        matches!(&self.market, Some(state) if matches!(state.role, Role::Author { .. }))
    }

    /// Whether this lane is the buyer's.
    pub(super) fn is_buyer(&self) -> bool {
        matches!(&self.market, Some(state) if state.role == Role::Buyer)
    }

    /// Whether the buyer still has the counter and the simulator ahead of it.
    pub(super) fn buyer_owes_the_market(&self) -> bool {
        matches!(&self.market, Some(state) if state.role == Role::Buyer && !state.flown)
    }

    fn market_evidence(&mut self) -> Option<&mut Evidence> {
        self.report.market.as_mut()
    }

    fn market_send(&mut self, phase: Phase, line: &str, not_before: u64) {
        if !in_vocabulary(line) && !super::in_vocabulary(line) {
            self.fail(
                not_before,
                format!("market command outside its vocabulary: {line}"),
            );
            return;
        }
        let (beat, timeout_ms) = match phase {
            Phase::Go(Route::Simulator | Route::Pod | Route::Port, _) => {
                (Beat::Simulator, super::ANSWER_MS)
            }
            Phase::Go(..) => (Beat::Market, super::ANSWER_MS),
            Phase::SimBoard => (Beat::Simulator, BOARDING_MS),
            Phase::SimTeam
            | Phase::SimMind
            | Phase::SimStart
            | Phase::Cockpit
            | Phase::SimExit
            | Phase::SimOver
            | Phase::SimReport => (Beat::Simulator, super::ANSWER_MS),
            Phase::Launch | Phase::Recover => (Beat::OnPad, super::ACT_MS),
            Phase::Aground | Phase::InBay => (Beat::OnPad, super::ANSWER_MS),
            _ => (Beat::Market, super::ANSWER_MS),
        };
        self.send(beat, Kind::Market(Task { phase }), line, not_before);
        if let Some(step) = self.step.as_mut() {
            step.timeout_ms = timeout_ms;
        }
    }

    /// The next walk of `route` from its step `at`, or `false` when the route is walked.
    fn market_go(&mut self, route: Route, at: usize) -> bool {
        match route.steps().get(at) {
            Some((word, _)) => {
                self.market_send(Phase::Go(route, at), word, 0);
                true
            }
            None => false,
        }
    }

    /// **The author's first market step**, once the Co-op has opened its account: the
    /// charter's reservation.
    pub(super) fn market_author_start(&mut self) {
        let Some((reference, charter)) = self
            .market
            .as_ref()
            .and_then(|state| Some((state.reference()?, state.charter.clone())))
        else {
            return;
        };
        if let Some(evidence) = self.market_evidence() {
            evidence.reference = Some(reference.clone());
        }
        self.market_send(
            Phase::Reserve,
            &format!("@mind/file {reference}={charter}"),
            0,
        );
    }

    /// **The buyer's bunkers are aboard**: to the counter.
    pub(super) fn market_buyer_start(&mut self) {
        self.market_go(Route::Counter, 0);
    }

    /// **The hull settled on the site**: the launch under the licence, from the bay.
    pub(super) fn market_launch(&mut self) {
        let Some(licence) = self.market.as_ref().and_then(|state| state.licence) else {
            self.fail(0, "the buyer reached the site with no licence".to_owned());
            return;
        };
        self.market_send(
            Phase::Launch,
            &lines::launch(1, Some(&format!("filed:{licence}"))),
            0,
        );
    }

    /// **The buyer is home**: the lane's end.
    pub(super) fn market_home(&mut self, now: u64) {
        self.report.success = true;
        self.milestone(super::Milestone::DockedHome, now);
        self.beat = Beat::Done;
        let (licence, match_id) = self.market.as_ref().map_or((0, 0), |state| {
            (state.licence.unwrap_or(0), state.match_id.unwrap_or(0))
        });
        let charter = self
            .market
            .as_ref()
            .map(|state| state.charter.clone())
            .unwrap_or_default();
        let outcome = format!(
            "bought licence {licence} of {charter}; it flew simulator match {match_id} and rig 1 \
             at {}; home",
            self.site.name()
        );
        self.events.push(super::LaneEvent::Said(outcome.clone()));
        self.result = Some(super::LaneResult {
            status: "done".to_owned(),
            outcome,
        });
    }

    /// **A line the Registry sent the author that the author did not ask for**: during the
    /// rating's wait, the engine's `Queued.` and `Done.` for the Registry's own steps. Counted
    /// and kept out of the window (there are thousands in a rating).
    pub(super) fn market_unasked(&mut self, raw: &str) -> bool {
        let rating = matches!(
            &self.step,
            Some(step) if matches!(&step.kind, Kind::Market(task) if task.phase == Phase::Rating)
        );
        if !rating || !engine_word(raw) {
            return false;
        }
        if let Some(evidence) = self.market_evidence() {
            evidence.rating_unasked_lines = evidence.rating_unasked_lines.saturating_add(1);
        }
        true
    }

    /// What a market step's window says.
    #[allow(clippy::too_many_lines)] // one arm a step, in the walk's order
    pub(super) fn market_verdict(
        &self,
        task: &Task,
        opened: u64,
        text: &str,
        raw: &[String],
        quiet: bool,
        now: u64,
    ) -> Verdict {
        let Some(state) = self.market.as_ref() else {
            return Verdict::Fail("a market step with no market".into());
        };
        let poll = |needle: &'static str, wait: u64, tries: u32| {
            if quiet {
                Verdict::Retry {
                    needle,
                    wait,
                    tries,
                }
            } else {
                Verdict::Wait
            }
        };
        let done_if = |yes: bool| {
            if yes && quiet {
                Verdict::Done
            } else {
                Verdict::Wait
            }
        };
        // A submitted step: its own line, then the engine's `Done.`. The engine's refusal and
        // the market's busy answer are typed again; the market's counter rule is a failure.
        let submitted = |request: &'static str| {
            if lines::has(text, words::COUNTER) {
                Verdict::Fail(format!(
                    "the market's counter rule refused the step where the lane stands: {text}"
                ))
            } else if lines::has(text, words::NOT_OPEN) || lines::has(text, words::ACCOUNT) {
                Verdict::Fail(format!("the market refused the step: {text}"))
            } else if lines::has(text, walk::REFUSED) {
                poll(walk::REFUSED, POLL_MS, SUBMIT_TRIES)
            } else if lines::has(text, words::BUSY) {
                poll(words::BUSY, POLL_MS, SUBMIT_TRIES)
            } else {
                done_if(lines::has(text, request) && confirmed(raw))
            }
        };
        match task.phase {
            Phase::Go(route, at) => match route.steps().get(at) {
                Some((_, room)) => done_if(lines::has(text, room)),
                None => Verdict::Fail("a walk past its route's end".into()),
            },
            Phase::Reserve => {
                // A charter already reserved (a run taken up again): the line is the filing's.
                if lines::has(text, words::NOT_RATED) && quiet {
                    Verdict::Skip(text.to_owned())
                } else if lines::has(text, words::NO_REVISION) {
                    Verdict::Fail(format!("the saved revision is not there to file: {text}"))
                } else {
                    submitted(words::RESERVING)
                }
            }
            Phase::Rate => {
                // The request is the author's own submitted step: its sentence and the engine's
                // `Done.` for it. Not `quiet`: the Registry's own steps follow at once, about one
                // a second, and every line after this pair is one the author did not ask for.
                if lines::has(text, walk::RATING_QUEUED) && confirmed(raw) {
                    Verdict::Done
                } else if lines::has(text, walk::REFUSED) {
                    poll(walk::REFUSED, POLL_MS, SUBMIT_TRIES)
                } else if lines::has(text, words::RATING_BUSY) && quiet {
                    Verdict::Skip(text.to_owned())
                } else if lines::has(text, words::RATING_UNAVAILABLE) {
                    poll(words::RATING_UNAVAILABLE, POLL_MS, 60)
                } else {
                    Verdict::Wait
                }
            }
            Phase::Rating => {
                if lines::has(text, walk::RATING_REFUSED)
                    || lines::has(text, walk::RATING_NOT_ADMITTED)
                {
                    Verdict::Fail(format!("the Registry would not rate the mind: {text}"))
                } else if lines::has(text, walk::RATING_FINISHED) {
                    Verdict::Done
                } else {
                    Verdict::Wait
                }
            }
            Phase::File => {
                if lines::has(text, words::NOT_RATED) {
                    // The Registry holds a finished job a pulse or two before its rating can be
                    // filed with. Past that, a finished rating that left nothing is a failed one.
                    poll(words::NOT_RATED, POLL_MS, FILE_TRIES)
                } else if lines::has(text, words::RESERVING) {
                    // The reservation had not landed when the rating was asked for.
                    if confirmed(raw) {
                        poll(words::RESERVING, POLL_MS, SUBMIT_TRIES)
                    } else {
                        Verdict::Wait
                    }
                } else if lines::has(text, words::UPDATING) {
                    Verdict::Fail(format!("the filing answered as an update: {text}"))
                } else {
                    submitted(words::FILING)
                }
            }
            Phase::Owned => {
                if owner_row(raw, &state.charter).is_some() && quiet {
                    Verdict::Done
                } else {
                    poll("no owner's row for the charter in +minds/holdings", POLL_MS, 40)
                }
            }
            Phase::Publish => {
                if lines::has(text, words::NO_FILED) || lines::has(text, words::TERMS) {
                    Verdict::Fail(format!("the listing was refused: {text}"))
                } else {
                    submitted(words::PUBLISHING)
                }
            }
            Phase::Listed => {
                if lines::has(text, words::COUNTER) {
                    Verdict::Fail(format!("`+minds` was refused at the counter: {text}"))
                } else if listing_row(raw, &state.charter).is_some() && quiet {
                    Verdict::Done
                } else {
                    poll("the author's listing is not on the counter", POLL_MS, 60)
                }
            }
            Phase::Browse => {
                if lines::has(text, words::COUNTER) || lines::has(text, words::NOT_OPEN) {
                    Verdict::Fail(format!("`+minds` was refused at the counter: {text}"))
                } else if listing_row(raw, &state.charter).is_some() && quiet {
                    Verdict::Done
                } else if lines::has(text, words::EMPTY) || lines::has(text, words::FOOT) {
                    poll(
                        "the author's charter is not for sale at the counter yet",
                        BROWSE_EVERY_MS,
                        BROWSE_TRIES,
                    )
                } else {
                    Verdict::Wait
                }
            }
            Phase::Quote => {
                if lines::has(text, words::NO_LISTING) {
                    Verdict::Fail(format!("the listing left the counter: {text}"))
                } else {
                    submitted(words::QUOTING)
                }
            }
            Phase::Pay => {
                if lines::has(text, words::NO_QUOTE) && quiet {
                    Verdict::Skip(text.to_owned())
                } else {
                    submitted(words::PAYING)
                }
            }
            Phase::Complete => {
                if lines::has(text, words::NO_PURCHASE) {
                    poll(words::NO_PURCHASE, POLL_MS, SUBMIT_TRIES)
                } else {
                    submitted(words::COMPLETING)
                }
            }
            Phase::Held => {
                if paid_row(raw, &state.charter).is_some() && quiet {
                    Verdict::Done
                } else {
                    poll("no paid licence for the charter in +minds/holdings", POLL_MS, 40)
                }
            }
            Phase::SimTeam => done_if(raw.iter().any(|line| {
                matches!(
                    classify(line),
                    ArenaLine::Claimed { side, .. } | ArenaLine::AlreadyHold { side, .. } if side == SIDE
                )
            })),
            Phase::SimMind => {
                if lines::has(text, walk::SIM_CARRY) && quiet {
                    Verdict::Done
                } else if quiet && now >= opened.saturating_add(POLL_MS) {
                    Verdict::Fail(format!("the slot would not carry the licensed mind: {text}"))
                } else {
                    Verdict::Wait
                }
            }
            Phase::SimStart => done_if(
                raw.iter()
                    .any(|line| matches!(classify(line), ArenaLine::Boarding { .. })),
            ),
            Phase::SimBoard => {
                let running = raw.iter().any(|line| {
                    matches!(classify(line), ArenaLine::Running { match_id } if Some(match_id) == state.match_id)
                });
                if running && !lines::has(text, walk::SIM_SEALED) {
                    Verdict::Fail(format!(
                        "the match started and the seat was told of no sealed mind: {text}"
                    ))
                } else {
                    done_if(running)
                }
            }
            Phase::Cockpit => done_if(lines::has(text, walk::COCKPIT_MIND)),
            Phase::SimExit => done_if(lines::has(text, walk::SIM_ON_ITS_OWN)),
            Phase::SimOver => {
                let over = raw.iter().any(|line| {
                    matches!(classify(line), ArenaLine::Over { match_id, .. } if Some(match_id) == state.match_id)
                });
                if over { Verdict::Done } else { Verdict::Wait }
            }
            Phase::SimReport => {
                if lines::has(text, walk::SIM_YOUR_MIND)
                    && lines::has(text, walk::SIM_IT_FLEW)
                    && quiet
                {
                    Verdict::Done
                } else if quiet && now >= opened.saturating_add(POLL_MS) {
                    Verdict::Fail(format!(
                        "the match's report does not say the pilot's mind flew the tank: {}",
                        crate::tail(text, 600)
                    ))
                } else {
                    Verdict::Wait
                }
            }
            Phase::Launch => {
                let refusal = [
                    dispatch::MIND_NO_LICENCE,
                    dispatch::MIND_PENDING,
                    dispatch::MIND_NO_CORE,
                    dispatch::MIND_SEATS_HELD,
                    dispatch::MIND_UNAVAILABLE,
                    dispatch::MIND_UNTITLED,
                    dispatch::MIND_GOVERNED_DRAFT,
                    claim::CLAIM_REFUSED,
                    claim::CLAIM_CANCELLED,
                    walk::LAUNCHED_DOCTRINE,
                ]
                .into_iter()
                .find(|needle| lines::has(text, needle));
                if let Some(needle) = refusal {
                    Verdict::Fail(format!(
                        "the licensed mind was not sealed aboard (`{needle}`): {text}"
                    ))
                } else if lines::has(text, lines::LAUNCHED) && quiet {
                    Verdict::Done
                } else if lines::has(text, lines::COLD) && quiet {
                    poll("the ground is still being mapped", POLL_MS, 20)
                } else {
                    Verdict::Wait
                }
            }
            Phase::Aground => {
                if lines::has(text, super::escort::AT_RAMP) && quiet {
                    Verdict::Done
                } else {
                    poll("the rig is not on the ground yet", POLL_MS, 20)
                }
            }
            Phase::Recover => done_if(lines::has(text, lines::RIG_BACK)),
            Phase::InBay => {
                if lines::has(text, super::escort::IN_BAY) && quiet {
                    Verdict::Done
                } else {
                    poll("the rig is not in the bay yet", POLL_MS, 60)
                }
            }
        }
    }

    /// A market step decided: the next one.
    #[allow(clippy::too_many_lines)] // one arm a step, in the walk's order
    pub(super) fn market_next(
        &mut self,
        phase: Phase,
        verdict: &Verdict,
        raw: &[String],
        decided_at: usize,
        sends: u32,
        now: u64,
    ) -> Result<(), String> {
        let skipped = matches!(verdict, Verdict::Skip(_));
        let state = self.market.as_ref().ok_or("no market state")?;
        let charter = state.charter.clone();
        let reference = state.reference();
        let code = state.code.clone();
        let licence = state.licence;
        match phase {
            Phase::Go(route, at) => {
                if self.market_go(route, at.saturating_add(1)) {
                    return Ok(());
                }
                match route {
                    Route::Desk => {
                        let filed = self
                            .market
                            .as_ref()
                            .and_then(|state| state.filed)
                            .ok_or("no filed number at the counter")?;
                        self.market_send(
                            Phase::Publish,
                            &format!("@mind/publish {filed} form=sealed"),
                            0,
                        );
                    }
                    Route::Counter => self.market_send(Phase::Browse, "+minds", 0),
                    Route::Simulator => {
                        self.market_send(Phase::SimTeam, &LobbyLine::SimTeam(SIDE).line(), 0);
                    }
                    Route::Pod => self.market_send(Phase::SimBoard, &LobbyLine::Board.line(), 0),
                    // Back in character at Arrivals: the mining pilot's walk to its hull.
                    Route::Port => {
                        if let Some(state) = self.market.as_mut() {
                            state.flown = true;
                        }
                        self.walk(Beat::BoardAndLift, "lobby", lines::ROOM_LOBBY, 0);
                    }
                }
            }
            Phase::Reserve => {
                let reference = reference.ok_or("no mind to file")?;
                if let Some(evidence) = self.market_evidence() {
                    evidence.reserved = !skipped;
                }
                self.market_send(Phase::Rate, &format!("@mind/rate {reference}"), 0);
            }
            Phase::Rate => {
                if let Some(evidence) = self.market_evidence() {
                    evidence.rating_asked_ms = Some(now);
                }
                self.follow_on(
                    now,
                    Beat::Market,
                    Kind::Market(Task {
                        phase: Phase::Rating,
                    }),
                    RATING_MS,
                    decided_at,
                );
            }
            Phase::Rating => {
                let reference = reference.ok_or("no mind to file")?;
                if let Some(evidence) = self.market_evidence() {
                    evidence.rating_finished_ms = Some(now);
                }
                self.milestone(super::Milestone::MindRated, now);
                self.market_send(Phase::File, &format!("@mind/file {reference}={charter}"), 0);
            }
            Phase::File => {
                let filing = line_with(raw, words::FILING);
                if let Some(evidence) = self.market_evidence() {
                    evidence.filing = filing;
                }
                self.market_send(Phase::Owned, "+minds/holdings", 0);
            }
            Phase::Owned => {
                let (filed, _) = owner_row(raw, &charter).ok_or("no owner's row")?;
                if let Some(state) = self.market.as_mut() {
                    state.filed = Some(filed);
                }
                if let Some(evidence) = self.market_evidence() {
                    evidence.filed = Some(filed);
                }
                self.milestone(super::Milestone::MindFiled, now);
                self.market_go(Route::Desk, 0);
            }
            Phase::Publish => {
                let publishing = line_with(raw, words::PUBLISHING);
                if let Some(evidence) = self.market_evidence() {
                    evidence.publishing = publishing;
                }
                self.market_send(Phase::Listed, "+minds", 0);
            }
            Phase::Listed => {
                let (code, row) = listing_row(raw, &charter).ok_or("no row for the charter")?;
                if let Some(evidence) = self.market_evidence() {
                    evidence.code = Some(code.clone());
                    evidence.row = Some(row);
                }
                self.milestone(super::Milestone::MindListed, now);
                self.report.success = true;
                self.beat = Beat::Done;
                let filed = self
                    .market
                    .as_ref()
                    .and_then(|state| state.filed)
                    .unwrap_or(0);
                let (asked, finished, unasked) =
                    self.report.market.as_ref().map_or((0, 0, 0), |evidence| {
                        (
                            evidence.rating_asked_ms.unwrap_or(0),
                            evidence.rating_finished_ms.unwrap_or(0),
                            evidence.rating_unasked_lines,
                        )
                    });
                let outcome = format!(
                    "filed {charter} as filed mind {filed} and listed it as {code}; the rating \
                     took {} s and sent {unasked} lines nobody asked for",
                    finished.saturating_sub(asked) / 1_000
                );
                self.events.push(super::LaneEvent::Said(outcome.clone()));
                self.result = Some(super::LaneResult {
                    status: "done".to_owned(),
                    outcome,
                });
            }
            Phase::Browse => {
                let (code, row) = listing_row(raw, &charter).ok_or("no row for the charter")?;
                let browses = sends;
                if let Some(state) = self.market.as_mut() {
                    state.code = Some(code.clone());
                }
                if let Some(evidence) = self.market_evidence() {
                    evidence.code = Some(code.clone());
                    evidence.row = Some(row);
                    evidence.browses = browses;
                }
                self.market_send(Phase::Quote, &format!("+minds/buy {code}"), 0);
            }
            Phase::Quote => {
                let code = code.ok_or("no listing code")?;
                let quoted = line_with(raw, words::QUOTING);
                if let Some(evidence) = self.market_evidence() {
                    evidence.quoted = quoted;
                }
                self.market_send(Phase::Pay, &format!("+minds/buy/pay {code}"), 0);
            }
            Phase::Pay => {
                let code = code.ok_or("no listing code")?;
                if skipped {
                    // The quote ran out before the payment: ask again, a bounded number of times.
                    let quotes = self.market.as_mut().map_or(0, |state| {
                        state.quotes = state.quotes.saturating_add(1);
                        state.quotes
                    });
                    if quotes >= QUOTE_TRIES {
                        return Err(format!("no quote lasted to its payment in {quotes} tries"));
                    }
                    self.market_send(Phase::Quote, &format!("+minds/buy {code}"), 0);
                    return Ok(());
                }
                let paid = line_with(raw, words::PAYING);
                if let Some(evidence) = self.market_evidence() {
                    evidence.paid = paid;
                }
                self.market_send(Phase::Complete, &format!("+minds/buy/complete {code}"), 0);
            }
            Phase::Complete => {
                let taken = line_with(raw, words::COMPLETING);
                if let Some(evidence) = self.market_evidence() {
                    evidence.taken = taken;
                }
                self.market_send(Phase::Held, "+minds/holdings", 0);
            }
            Phase::Held => {
                let (licence, row) = paid_row(raw, &charter).ok_or("no paid row")?;
                if let Some(state) = self.market.as_mut() {
                    state.licence = Some(licence);
                }
                if let Some(evidence) = self.market_evidence() {
                    evidence.licence = Some(licence);
                    evidence.holding = Some(row);
                }
                self.milestone(super::Milestone::LicenceBought, now);
                self.market_go(Route::Simulator, 0);
            }
            Phase::SimTeam => {
                let licence = licence.ok_or("no licence at the simulator")?;
                self.market_send(Phase::SimMind, &LobbyLine::SimMind(licence).line(), 0);
            }
            Phase::SimMind => {
                let chosen = line_with(raw, walk::SIM_CARRY);
                if let Some(evidence) = self.market_evidence() {
                    evidence.chosen = chosen;
                }
                self.market_send(Phase::SimStart, &LobbyLine::SimStart.line(), 0);
            }
            Phase::SimStart => {
                let match_id = raw
                    .iter()
                    .find_map(|line| match classify(line) {
                        ArenaLine::Boarding { match_id } => Some(match_id),
                        _ => None,
                    })
                    .ok_or("no boarding match")?;
                if let Some(state) = self.market.as_mut() {
                    state.match_id = Some(match_id);
                }
                if let Some(evidence) = self.market_evidence() {
                    evidence.match_id = Some(match_id);
                }
                self.market_go(Route::Pod, 0);
            }
            Phase::SimBoard => {
                let sealed = line_with(raw, walk::SIM_SEALED);
                if let Some(evidence) = self.market_evidence() {
                    evidence.sealed = sealed;
                }
                self.market_send(Phase::Cockpit, "MIND", 0);
            }
            Phase::Cockpit => {
                let cockpit = line_with(raw, walk::COCKPIT_MIND);
                if let Some(evidence) = self.market_evidence() {
                    evidence.cockpit = cockpit;
                }
                self.market_send(Phase::SimExit, "EXIT", 0);
            }
            Phase::SimExit => {
                let left = line_with(raw, walk::SIM_ON_ITS_OWN);
                if let Some(evidence) = self.market_evidence() {
                    evidence.left_to_mind = left;
                }
                self.follow_on(
                    now,
                    Beat::Simulator,
                    Kind::Market(Task {
                        phase: Phase::SimOver,
                    }),
                    MATCH_MS,
                    decided_at,
                );
            }
            Phase::SimOver => {
                let followed = self.market.as_ref().and_then(|state| state.match_id);
                let end = raw.iter().find_map(|line| match classify(line) {
                    ArenaLine::Over { match_id, result } if Some(match_id) == followed => {
                        Some(result)
                    }
                    _ => None,
                });
                if let Some(evidence) = self.market_evidence() {
                    evidence.match_end = end.map(|word: ResultWord| word.as_str().to_owned());
                }
                self.market_send(Phase::SimReport, "SIM REPORT", 0);
            }
            Phase::SimReport => {
                let report = line_with(raw, walk::SIM_YOUR_MIND);
                let flew = line_with(raw, walk::SIM_IT_FLEW);
                if let Some(evidence) = self.market_evidence() {
                    evidence.report = report;
                    evidence.flew = flew;
                }
                self.milestone(super::Milestone::MindFlownSim, now);
                self.market_go(Route::Port, 0);
            }
            Phase::Launch => {
                let launched = line_with(raw, lines::LAUNCHED);
                if let Some(evidence) = self.market_evidence() {
                    evidence.launched = launched;
                }
                self.milestone(super::Milestone::Launched, now);
                self.market_send(Phase::Aground, "+tv/1", 0);
            }
            Phase::Aground => {
                let aground = line_with(raw, "Rig 1");
                if let Some(evidence) = self.market_evidence() {
                    evidence.aground = aground;
                }
                self.market_send(Phase::Recover, "+tv/1 recover go", 0);
            }
            Phase::Recover => self.market_send(Phase::InBay, "+tv/1", 0),
            Phase::InBay => {
                let in_bay = line_with(raw, "Rig 1");
                if let Some(evidence) = self.market_evidence() {
                    evidence.in_bay = in_bay;
                }
                self.milestone(super::Milestone::Recovered, now);
                self.send(Beat::Home, Kind::PlanHome, "+ship/plan ceres", 0);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The walk's vocabulary is closed: its own lines in, everything else out.
    #[test]
    fn the_walks_vocabulary_is_closed() {
        for line in [
            "@mind/file sentry@1=Sentry Line",
            "@mind/rate sentry@1",
            "@mind/publish 3 form=sealed",
            "+minds",
            "+minds/holdings",
            "+minds/buy 59809cd4",
            "+minds/buy/pay 59809cd4",
            "+minds/buy/complete 59809cd4",
            "mc",
            "out",
            "limbo",
            "hall",
            "lobby",
            "c",
            "ooc",
            "simulator",
            "red",
            "SIM TEAM red",
            "SIM MIND filed:2",
            "SIM START",
            "BOARD",
            "MIND",
            "EXIT",
            "SIM REPORT",
            "+tv/1 launch mind=filed:2 go",
            "+tv/1",
            "+tv/1 recover go",
        ] {
            assert!(in_vocabulary(line), "{line}");
        }
        for line in [
            // No other `@` verb, and none of the two with anything else in it.
            "@mind/withdraw 3",
            "@mind/file sentry@1=Sentry Line; @teleport #0",
            "@mind/file Sentry@1=Sentry Line",
            "@mind/file sentry@0=Sentry Line",
            "@mind/rate sentry",
            "@mind/publish 3 form=open",
            "@mind/publish 3 form=sealed price=1",
            "@teleport #811",
            "@registry/cancel",
            // The centre's other verbs, and the cockpit's.
            "SIM MIND Standard",
            "SIM MIND filed:02",
            "SIM MIND filed:2 PROFILE Line",
            "SIM AI 3",
            "SIM ABORT",
            "SIM TEAM blue",
            "MIND PROFILE Line",
            "FIRE 1",
            // The market's other verbs.
            "+minds/buy/refund 59809cd4",
            "+minds/buy 59809cd",
            "+minds/buy 59809cd4 org=Basin",
            "+minds/resell 2=tally price=1",
            // A draft is not a licence.
            "+tv/1 launch mind=draft:sentry go",
            "+tv/1 launch mind=filed:2",
            "+tv/2 launch mind=filed:2 go",
        ] {
            assert!(!in_vocabulary(line), "{line}");
        }
        assert!(charter_ok("Sentry Line"));
        assert!(charter_ok("A"));
        for name in [
            "",
            " Sentry",
            "Sentry ",
            "Sentry  Line",
            "9lives",
            "Sentry=Line",
            "Séntry",
        ] {
            assert!(!charter_ok(name), "{name:?}");
        }
        assert!(!charter_ok(&"a".repeat(41)));
        assert!(mind_ok(MIND_NAME));
        assert!(!mind_ok("Sentry"));
        assert!(!mind_ok("sentry line"));
    }

    /// The SSH mind service's requests are the help's own shape, and its replies are read for
    /// their status, never trusted for it.
    #[test]
    fn the_mind_services_requests_and_replies() {
        use serde_json::json;
        assert_eq!(
            create_request("sentry"),
            json!({"op": "create", "name": "sentry", "from": null})
        );
        assert_eq!(
            read_request("sentry"),
            json!({"op": "read", "name": "sentry", "revision": null})
        );
        let read = json!({
            "status": "read", "name": "sentry", "revision": null,
            "base": {"edit": 0, "revision": null},
            "draft": {"source": "fn think(view, card) {\n}\n", "declared_class": "I", "profiles": {"Default": {}}},
            "diagnostics": []
        });
        let (base, draft) = read_reply(&read).expect("a read");
        assert_eq!(base, json!({"edit": 0, "revision": null}));
        assert_eq!(draft_source(&draft), Some("fn think(view, card) {\n}\n"));
        let replace = replace_request("sentry", &base, &draft, MIND_SOURCE);
        assert_eq!(replace["op"], "replace");
        assert_eq!(replace["base"], base);
        assert_eq!(replace["draft"]["source"], MIND_SOURCE);
        // Everything else of the draft goes back as it came.
        assert_eq!(replace["draft"]["declared_class"], "I");
        assert_eq!(replace["draft"]["profiles"], json!({"Default": {}}));
        let edited =
            json!({"status": "edited", "name": "sentry", "base": {"edit": 1, "revision": null}});
        let base = edited_reply(&edited).expect("an edit");
        assert_eq!(check_request("sentry", &base)["base"]["edit"], 1);
        let save = save_request("sentry", &base);
        assert_eq!(save["op"], "save");
        assert_eq!(save["version"], json!(null));
        let checked = json!({"status": "checked", "job": 3, "diagnostics": [], "reports": [
            {"picture": "empty", "armed": false, "fault": null},
            {"picture": "empty", "armed": true, "fault": null}
        ]});
        assert_eq!(checked_reply(&checked), Ok(2));
        let faulted = json!({"status": "checked", "diagnostics": [], "reports": [
            {"picture": "empty", "armed": false, "fault": "the hull is cold"}
        ]});
        assert!(checked_reply(&faulted).unwrap_err().contains("faulted"));
        let broken = json!({"status": "checked", "reports": [], "diagnostics": [
            {"field": "source", "message": "expected `}`", "line": 3, "column": 1}
        ]});
        assert!(checked_reply(&broken).unwrap_err().contains("expected `}`"));
        assert!(
            checked_reply(&json!({"status": "checked", "reports": [], "diagnostics": []})).is_err()
        );
        let saved = json!({"status": "saved", "job": 4, "revision": 1, "digest": "ab12", "base": {"edit": 2, "revision": 1}});
        assert_eq!(saved_reply(&saved), Ok((1, "ab12".to_owned())));
        // The workshop's refusal, as an offline player met it before doc 44 unit MW.19.
        let refused = json!({"status": "error", "code": "missing", "message": "No owned mind or authoring job is available."});
        for failed in [
            read_reply(&refused).map(|_| ()),
            edited_reply(&refused).map(|_| ()),
            checked_reply(&refused).map(|_| ()),
            saved_reply(&refused).map(|_| ()),
        ] {
            let said = failed.unwrap_err();
            assert!(
                said.contains("No owned mind or authoring job is available."),
                "{said}"
            );
        }
        // The bot's own source is what it says it is: a `think` and the stock rules.
        assert!(MIND_SOURCE.contains("fn think(view, card)"));
        assert!(MIND_SOURCE.len() < 32 * 1024);
    }

    // -- the walk against a scripted server ---------------------------------------------------
    //
    // The answers below are the game's own lines, as a live launch world printed them to the
    // two players (doc 44 unit MW.19's cut note): nothing here is a guess at a sentence.

    use crate::mine::{Milestone, Settings, Site};

    /// A lane, the lines it typed, and lines still to arrive.
    struct Rig {
        lane: Lane,
        now: u64,
        typed: Vec<String>,
        later: Vec<(u64, String)>,
    }

    impl Rig {
        fn new(lane: Lane) -> Rig {
            Rig {
                lane,
                now: 0,
                typed: Vec::new(),
                later: Vec::new(),
            }
        }

        fn author() -> Rig {
            Rig::new(
                Lane::new(
                    Site::Kerwan,
                    Settings {
                        hull: "lathe Lode".to_owned(),
                        epoch_unix_ms: 1_900_000_000_000,
                    },
                    "lathe".to_owned(),
                )
                .with_author(MIND_NAME.to_owned(), 1, "Sentry Line".to_owned()),
            )
        }

        fn buyer() -> Rig {
            let mut lane = Lane::new(
                Site::Kerwan,
                Settings {
                    hull: "tally Lode".to_owned(),
                    epoch_unix_ms: 1_900_000_000_000,
                },
                "tally".to_owned(),
            )
            .with_buyer("Sentry Line".to_owned());
            // The kit-up and the bunkers are the mining pilot's and the escort's, tested there.
            lane.step = None;
            Rig::new(lane)
        }

        /// Play until the lane ends, `stop` says so, or `limit_ms` of lane time has passed.
        /// `answer` gives each typed line's reply as (how much later, the line).
        fn play(
            &mut self,
            limit_ms: u64,
            mut answer: impl FnMut(&str, usize) -> Vec<(u64, String)>,
            stop: impl Fn(&Rig) -> bool,
        ) {
            let end = self.now.saturating_add(limit_ms);
            while self.now < end && self.lane.result().is_none() && !stop(self) {
                for send in self.lane.tick(self.now) {
                    assert!(
                        in_vocabulary(&send.line) || super::super::in_vocabulary(&send.line),
                        "outside the vocabulary: {}",
                        send.line
                    );
                    let nth = self.typed.iter().filter(|line| **line == send.line).count();
                    for (after, line) in answer(&send.line, nth) {
                        self.later.push((self.now.saturating_add(after), line));
                    }
                    self.typed.push(send.line);
                }
                let now = self.now;
                let (due, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.later)
                    .into_iter()
                    .partition(|(at, _)| *at <= now);
                self.later = rest;
                for (_, line) in due {
                    self.lane.feed(now, &format!("{line}\n"));
                }
                self.now = self.now.saturating_add(50);
            }
        }

        fn evidence(&self) -> Evidence {
            self.lane.report().market.expect("the market block")
        }
    }

    fn now(lines: &[&str]) -> Vec<(u64, String)> {
        lines.iter().map(|line| (0, (*line).to_owned())).collect()
    }

    /// A room's head as `look` prints it.
    fn room(name: &str) -> Vec<(u64, String)> {
        now(&[&format!("{name} #105"), "You see nothing special."])
    }

    /// The author's world: `unasked` pairs of the engine's words during the rating, and the
    /// filing refused `held` times while the Registry still holds its finished job.
    fn author_answers(unasked: u32, held: usize) -> impl FnMut(&str, usize) -> Vec<(u64, String)> {
        move |line, nth| match line {
            "c" => room(lines::ROOM_ARRIVALS),
            "registry" => room(lines::ROOM_REGISTRY),
            "+coop/enrol lathe Lode" => now(&[
                "Ceres Clearing opens account #41 in your name.",
                "The Co-op charters you FPC-D-0401, laid up as the Slack Water, now the lathe Lode, cradled at the Visiting Berth",
                "Gallery.",
                "25000.00cr staked to you. Buyout 90000.00cr. Ilse Oduya-Brandt countersigns.",
            ]),
            "@mind/file sentry@1=Sentry Line" if nth == 0 => now(&[
                "Reserving a charter for this filing first. Once that is confirmed, type @mind/file sentry@1=Sentry Line again.",
                "Queued.",
                "Done.",
            ]),
            "@mind/rate sentry@1" => {
                let mut said = vec![
                    (0, "Registry preparation queued.".to_owned()),
                    (0, "Queued.".to_owned()),
                    (0, "Done.".to_owned()),
                    (1_000, "Registry work retained and queued.".to_owned()),
                    (1_500, "tally goes Mind Counter <MC>.".to_owned()),
                ];
                for pair in 0..unasked {
                    let at = 2_000 + u64::from(pair) * 100;
                    said.push((at, "Queued.".to_owned()));
                    said.push((at + 50, "Done.".to_owned()));
                }
                said.push((
                    2_000 + u64::from(unasked) * 100 + 200,
                    "Registry work finished.".to_owned(),
                ));
                said
            }
            "@mind/file sentry@1=Sentry Line" if nth <= held => now(&[
                "sentry@1 has no finished Registry rating. Rate it with @mind/rate sentry@1.",
            ]),
            "@mind/file sentry@1=Sentry Line" => {
                now(&["Filing sentry@1 as Sentry Line.", "Queued.", "Done."])
            }
            "+minds/holdings" => now(&[
                "Your mind licences:",
                "  filed 1         owner      Sentry Line           Active, 90 days left",
            ]),
            "mc" => now(&["Named launch counter #811", "You see nothing special."]),
            "@mind/publish 1 form=sealed" => {
                now(&["Listing filed mind 1 here as sealed.", "Queued.", "Done."])
            }
            "+minds" => now(&[
                "Minds for sale here (term and Grace in IC days):",
                "  b8c32048  Sentry Line               sealed       class I           400cr  90+28 days",
                "Use +minds <code> for the full terms and +minds/buy <code> to ask for a quote.",
            ]),
            other => panic!("the author typed a line the script does not know: {other}"),
        }
    }

    /// The author's whole walk: reserved, rated, filed, listed, in that order and nothing else.
    #[test]
    fn the_author_reserves_rates_files_and_lists() {
        let mut rig = Rig::author();
        rig.play(600_000, author_answers(40, 2), |_| false);
        let result = rig.lane.result().cloned().expect("the lane ended");
        assert_eq!(result.status, "done", "{}", result.outcome);
        assert_eq!(
            rig.typed,
            [
                "c",
                "registry",
                "+coop/enrol lathe Lode",
                "@mind/file sentry@1=Sentry Line",
                "@mind/rate sentry@1",
                // Twice while the Registry holds its finished job, then the filing.
                "@mind/file sentry@1=Sentry Line",
                "@mind/file sentry@1=Sentry Line",
                "@mind/file sentry@1=Sentry Line",
                "+minds/holdings",
                "mc",
                "@mind/publish 1 form=sealed",
                "+minds",
            ]
        );
        let evidence = rig.evidence();
        assert_eq!(evidence.role, "author");
        assert_eq!(evidence.reference.as_deref(), Some("sentry@1"));
        assert!(evidence.reserved);
        assert_eq!(
            evidence.rating_unasked_lines, 80,
            "forty pairs nobody asked for"
        );
        assert!(evidence.rating_finished_ms > evidence.rating_asked_ms);
        assert_eq!(
            evidence.filing.as_deref(),
            Some("Filing sentry@1 as Sentry Line.")
        );
        assert_eq!(evidence.filed, Some(1));
        assert_eq!(evidence.code.as_deref(), Some("b8c32048"));
        assert!(evidence.row.is_some_and(|row| row.starts_with("b8c32048")));
        for name in [
            Milestone::MindRated,
            Milestone::MindFiled,
            Milestone::MindListed,
        ] {
            assert!(rig.lane.reached(name), "{name:?}");
        }
        assert!(rig.lane.report().success);
        assert!(
            result.outcome.contains("sent 80 lines nobody asked for"),
            "{}",
            result.outcome
        );
    }

    /// A rating that finished and left nothing to file with (the Registry says the same
    /// `finished` for a failed rating) ends the lane, naming it.
    #[test]
    fn a_rating_that_left_nothing_fails_the_author() {
        let mut rig = Rig::author();
        rig.play(600_000, author_answers(0, usize::MAX), |_| false);
        let result = rig.lane.result().cloned().expect("the lane ended");
        assert_eq!(result.status, "blocked");
        assert!(
            result.outcome.contains("has no finished Registry rating"),
            "{}",
            result.outcome
        );
        let filings = rig
            .typed
            .iter()
            .filter(|line| line.starts_with("@mind/file"))
            .count();
        assert_eq!(
            filings,
            1 + FILE_TRIES as usize,
            "the reservation and {FILE_TRIES} tries"
        );
    }

    /// The buyer's world from the chandlery to the pod and back to Arrivals.
    fn buyer_answers(listed_after: usize) -> impl FnMut(&str, usize) -> Vec<(u64, String)> {
        move |line, nth| {
            match line {
            "exchange" => room(lines::ROOM_EXCHANGE),
            "concourse" => room(lines::ROOM_CONCOURSE),
            "arrivals" => room(lines::ROOM_ARRIVALS),
            "registry" => room(lines::ROOM_REGISTRY),
            "mc" => now(&["Named launch counter #811", "You see nothing special."]),
            "+minds" if nth < listed_after => now(&["No minds are for sale at this counter."]),
            "+minds" => now(&[
                "Minds for sale here (term and Grace in IC days):",
                "  0a1b2c3d  Another Line              open         class II          400cr  90+28 days",
                "  b8c32048  Sentry Line               sealed       class I           400cr  90+28 days",
                "Use +minds <code> for the full terms and +minds/buy <code> to ask for a quote.",
            ]),
            "+minds/buy b8c32048" => now(&[
                "Asking for a quote on listing b8c32048. +minds b8c32048 shows it once it is confirmed.",
                "Queued.",
                "Done.",
            ]),
            "+minds/buy/pay b8c32048" => now(&[
                "Paying your quote on listing b8c32048 into escrow.",
                "Queued.",
                "Done.",
            ]),
            "+minds/buy/complete b8c32048" => now(&[
                "Taking the licence from listing b8c32048.",
                "Queued.",
                "Done.",
            ]),
            "+minds/holdings" => now(&[
                "Your mind licences:",
                "  licence 2         paid       Sentry Line           sealed       Active, 90 days left",
            ]),
            "out" if nth == 0 => room(lines::ROOM_REGISTRY),
            "out" => room(lines::ROOM_ARRIVALS),
            "limbo" => now(&["Limbo #0", "You see nothing special."]),
            "ooc" | "hall" => now(&["Welcome Hall #2"]),
            "simulator" => now(&["Simulator Lobby #7"]),
            "lobby" if nth == 0 => now(&["Simulator Lobby #7"]),
            // From Arrivals it is the mining pilot's walk to the pressure lobby: the test's end.
            "lobby" => Vec::new(),
            "SIM TEAM red" => now(&[
                "tally takes red slot 1. `SIM START` begins the match; the machines take the pods nobody claims.",
            ]),
            "SIM MIND filed:2" => now(&["Your slot will carry Sentry Line [filed:2] (Line)."]),
            "SIM START" => now(&[
                "Match 2 is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 120 second(s).",
            ]),
            "red" => now(&["Red Room #6"]),
            "BOARD" => now(&[
                "You climb into Red Pod 1. The panel wakes when every pilot is seated, or in 118 second(s). EXIT to get out — once the",
                "match is running, leaving abandons the tank for the rest of it.",
                "Red Pod 1 #21",
                "You have been joined to channel RedNet (alias: rg).",
                "Sealed for this sortie: Sentry Line (Line), Class I; MIND reads it.",
                "Match 2 is running. Good hunting.",
            ]),
            "MIND" => now(&[
                "Mind: Sentry Line, sealed for this sortie; it thinks at Class I on a Class II core.",
                "Profile: Line, 1 of 1; no other is sealed to switch to.",
                "Channels: you hold helm, sensors and fire; the mind is dormant while you fly.",
            ]),
            "EXIT" => vec![
                (0, "You have been removed from channel RedNet.".to_owned()),
                (
                    0,
                    "You climb out of Red Pod 1. Red Tank 1 is on its own for the rest of the match; you cannot take it back."
                        .to_owned(),
                ),
                (0, "Red Room #6".to_owned()),
                // Another match's end is not this one's.
                (20_000, "Match 1 is over (red). Its units are checked in.".to_owned()),
                (264_000, "Match 2 is over (blue). Its units are checked in.".to_owned()),
            ],
            "SIM REPORT" => now(&[
                "Reading your report of match 2 …",
                "Match 2 on The Proving Ground, 2026-11-12 22:21 UTC: blue won (red eliminated), 4 m 24 s.",
                "You flew RG: intact; 6 shots, 2 hits (33%).",
                "Your mind: Sentry Line (Line).",
                "  It flew your tank from 0:07, when you climbed out, to the end.",
                "  Its shots: cannon 6, 2 of them hits; it destroyed nothing.",
            ]),
            "c" => room(lines::ROOM_ARRIVALS),
            other => panic!("the buyer typed a line the script does not know: {other}"),
        }
        }
    }

    /// The buyer reads the counter until the charter is on it, buys in three confirmed steps,
    /// and flies the licence in the simulator; then the walk is the mining pilot's again.
    #[test]
    fn the_buyer_buys_and_the_licence_flies_the_simulator() {
        let mut rig = Rig::buyer();
        rig.lane.market_buyer_start();
        rig.play(900_000, buyer_answers(3), |rig| {
            rig.typed.iter().filter(|line| *line == "lobby").count() == 2
        });
        assert!(rig.lane.result().is_none(), "{:?}", rig.lane.result());
        assert_eq!(
            rig.typed,
            [
                "exchange",
                "concourse",
                "arrivals",
                "registry",
                "mc",
                "+minds",
                "+minds",
                "+minds",
                "+minds",
                "+minds/buy b8c32048",
                "+minds/buy/pay b8c32048",
                "+minds/buy/complete b8c32048",
                "+minds/holdings",
                "out",
                "out",
                "limbo",
                "ooc",
                "simulator",
                "SIM TEAM red",
                "SIM MIND filed:2",
                "SIM START",
                "red",
                "BOARD",
                "MIND",
                "EXIT",
                "SIM REPORT",
                "lobby",
                "hall",
                "limbo",
                "c",
                // The mining pilot's own walk to its hull, from Arrivals.
                "lobby",
            ]
        );
        let evidence = rig.evidence();
        assert_eq!(evidence.role, "buyer");
        assert_eq!(evidence.browses, 4);
        assert_eq!(evidence.code.as_deref(), Some("b8c32048"));
        assert_eq!(evidence.licence, Some(2));
        assert!(evidence.quoted.is_some() && evidence.paid.is_some() && evidence.taken.is_some());
        assert_eq!(
            evidence.chosen.as_deref(),
            Some("Your slot will carry Sentry Line [filed:2] (Line).")
        );
        assert_eq!(evidence.match_id, Some(2));
        assert_eq!(
            evidence.sealed.as_deref(),
            Some("Sealed for this sortie: Sentry Line (Line), Class I; MIND reads it.")
        );
        assert!(
            evidence
                .cockpit
                .is_some_and(|line| line.starts_with("Mind: Sentry Line"))
        );
        assert!(evidence.left_to_mind.is_some());
        assert_eq!(evidence.match_end.as_deref(), Some("blue"));
        assert_eq!(
            evidence.report.as_deref(),
            Some("Your mind: Sentry Line (Line).")
        );
        assert!(
            evidence
                .flew
                .is_some_and(|line| line.starts_with("It flew your tank from 0:07"))
        );
        assert!(rig.lane.reached(Milestone::LicenceBought));
        assert!(rig.lane.reached(Milestone::MindFlownSim));
        assert!(!rig.lane.buyer_owes_the_market());
    }

    /// At the site: the launch under the licence, the rig at its ramp, back in the bay, and
    /// the plan home, which is the mining pilot's.
    #[test]
    fn the_buyer_launches_under_the_licence_and_brings_the_rig_home() {
        let mut rig = Rig::buyer();
        rig.lane.market.as_mut().expect("the buyer").licence = Some(2);
        rig.lane.market_launch();
        rig.play(
            300_000,
            |line, nth| match line {
                "+tv/1 launch mind=filed:2 go" => vec![
                    (
                        0,
                        "Rig 1 starts down the stern ramp: on the ground 22:59 UTC (IC December 30, 2352, 05:47; in 13 s)."
                            .to_owned(),
                    ),
                    (0, "Queued.".to_owned()),
                    (0, "Done.".to_owned()),
                    (500, "Queued.".to_owned()),
                    (600, "Done.".to_owned()),
                    (13_000, "Rig 1 is down the stern ramp on Kerwan Floor.".to_owned()),
                ],
                "+tv/1" if nth == 0 => now(&[
                    "Rig 1: OWP-ES-01, on the ground at the stern ramp on Kerwan Floor; charge 100 %; leased from Occator for 6 d 23 h more;",
                    "hopper empty.",
                ]),
                "+tv/1 recover go" => vec![
                    (
                        0,
                        "Rig 1 starts up the stern ramp: in the bay 22:59 UTC (IC December 30, 2352, 05:56; in 13 s)."
                            .to_owned(),
                    ),
                    (0, "Queued.".to_owned()),
                    (0, "Done.".to_owned()),
                    (13_000, "Rig 1 is back in the Tally Lode's bay.".to_owned()),
                    (13_000, "Done.".to_owned()),
                ],
                "+tv/1" => now(&[
                    "Rig 1: OWP-ES-01, in the bay; charge 100 %; leased from Occator for 6 d 23 h more; hopper empty.",
                ]),
                "+ship/plan ceres" => Vec::new(),
                other => panic!("the buyer typed a line the script does not know: {other}"),
            },
            |rig| rig.typed.last().is_some_and(|line| line == "+ship/plan ceres"),
        );
        assert!(rig.lane.result().is_none(), "{:?}", rig.lane.result());
        assert_eq!(
            rig.typed,
            [
                "+tv/1 launch mind=filed:2 go",
                "+tv/1",
                "+tv/1 recover go",
                "+tv/1",
                "+ship/plan ceres",
            ]
        );
        let evidence = rig.evidence();
        assert_eq!(
            evidence.launched.as_deref(),
            Some("Rig 1 is down the stern ramp on Kerwan Floor.")
        );
        assert!(
            evidence
                .aground
                .is_some_and(|line| line.contains("on the ground at the stern ramp"))
        );
        assert!(
            evidence
                .in_bay
                .is_some_and(|line| line.contains(", in the bay;"))
        );
        assert!(rig.lane.reached(Milestone::Launched));
        assert!(rig.lane.reached(Milestone::Recovered));
    }

    /// Every way the game refuses to seal the licence aboard ends the lane naming the
    /// sentence, as does a licence that fell to the stock doctrine.
    #[test]
    fn a_launch_the_game_refuses_fails_the_buyer_by_its_sentence() {
        for (answer, needle) in [
            ("You hold no licence 2.", dispatch::MIND_NO_LICENCE),
            (
                "That rig carries no core; only Standard Doctrine flies it.",
                dispatch::MIND_NO_CORE,
            ),
            (
                "Licence 2 already flies 1 of its 1 seats; bring a hull home first.",
                dispatch::MIND_SEATS_HELD,
            ),
            (
                "The Registry did not admit the mind's claim; the launch is cancelled and the rig stays in its bay.",
                claim::CLAIM_REFUSED,
            ),
            (
                "Licence 2 is lapsed: rig 1 flies Standard Doctrine sealed aboard.",
                walk::LAUNCHED_DOCTRINE,
            ),
        ] {
            let mut rig = Rig::buyer();
            rig.lane.market.as_mut().expect("the buyer").licence = Some(2);
            rig.lane.market_launch();
            rig.play(60_000, |_, _| now(&[answer]), |_| false);
            let result = rig.lane.result().cloned().expect("the lane ended");
            assert_eq!(result.status, "blocked", "{answer}");
            assert!(result.outcome.contains(needle), "{}", result.outcome);
            assert_eq!(rig.typed, ["+tv/1 launch mind=filed:2 go"]);
        }
    }

    /// The counter's rule, the simulator's refusal of a licence, and a seat told of no mind
    /// are each a failure with the game's words in it.
    #[test]
    fn the_markets_and_the_centres_refusals_fail_the_walk() {
        // The author lists away from a counter.
        let mut rig = Rig::author();
        let mut script = author_answers(0, 0);
        rig.play(
            600_000,
            |line, nth| match line {
                "@mind/publish 1 form=sealed" => {
                    now(&["Listing and buying minds happen at a market counter."])
                }
                other => script(other, nth),
            },
            |_| false,
        );
        let result = rig.lane.result().cloned().expect("the lane ended");
        assert_eq!(result.status, "blocked");
        assert!(
            result.outcome.contains(words::COUNTER),
            "{}",
            result.outcome
        );

        // The centre does not know the licence.
        let mut rig = Rig::buyer();
        rig.lane.market_buyer_start();
        let mut script = buyer_answers(0);
        rig.play(
            600_000,
            |line, nth| match line {
                "SIM MIND filed:2" => now(&["No such owned mind or revision."]),
                other => script(other, nth),
            },
            |_| false,
        );
        let result = rig.lane.result().cloned().expect("the lane ended");
        assert_eq!(result.status, "blocked");
        assert!(
            result.outcome.contains("No such owned mind or revision."),
            "{}",
            result.outcome
        );
        assert_eq!(
            rig.typed.last().map(String::as_str),
            Some("SIM MIND filed:2")
        );

        // The match starts and the seat is told of no sealed mind.
        let mut rig = Rig::buyer();
        rig.lane.market_buyer_start();
        let mut script = buyer_answers(0);
        rig.play(
            600_000,
            |line, nth| match line {
                "BOARD" => now(&["Red Pod 1 #21", "Match 2 is running. Good hunting."]),
                other => script(other, nth),
            },
            |_| false,
        );
        let result = rig.lane.result().cloned().expect("the lane ended");
        assert_eq!(result.status, "blocked");
        assert!(
            result.outcome.contains("told of no sealed mind"),
            "{}",
            result.outcome
        );
    }

    /// The rows the walk reads, as the game prints them.
    #[test]
    fn the_holdings_and_the_counters_rows_are_read_by_their_charter() {
        let own = |lines: &[&str]| {
            lines
                .iter()
                .map(|line| (*line).to_owned())
                .collect::<Vec<_>>()
        };
        let holdings = own(&[
            "Your mind licences:",
            "  filed 3         owner      Other Line            Active, 90 days left",
            "  filed 1         owner      Sentry Line           Active, 90 days left",
            "  licence 2         paid       Sentry Line           sealed       Active, 90 days left",
        ]);
        assert_eq!(
            owner_row(&holdings, "Sentry Line").map(|row| row.0),
            Some(1)
        );
        assert_eq!(
            owner_row(&holdings, "sentry line").map(|row| row.0),
            Some(1)
        );
        assert_eq!(paid_row(&holdings, "Sentry Line").map(|row| row.0), Some(2));
        assert_eq!(owner_row(&holdings, "Missing Line"), None);
        // A row before the table's head is not a row.
        assert_eq!(owner_row(&holdings[1..], "Sentry Line"), None);
        // A name longer than its column is known by its start.
        let long = own(&[
            "Your mind licences:",
            "  filed 9         owner      A Rather Long Charte  Active, 90 days left",
        ]);
        assert_eq!(
            owner_row(&long, "A Rather Long Charter Name").map(|row| row.0),
            Some(9)
        );
        let counter = own(&[
            "Minds for sale here (term and Grace in IC days):",
            "  0a1b2c3d  Another Line              open         class II          400cr  90+28 days",
            "  b8c32048  Sentry Line               sealed       class I           400cr  90+28 days",
            "Use +minds <code> for the full terms and +minds/buy <code> to ask for a quote.",
        ]);
        assert_eq!(
            listing_row(&counter, "Sentry Line").map(|row| row.0),
            Some("b8c32048".to_owned())
        );
        // A charter that is the start of another's name is not that other.
        assert_eq!(listing_row(&counter, "Sentry"), None);
        assert_eq!(owner_row(&holdings, "Sentry"), None);
        assert_eq!(listing_row(&counter, "Missing Line"), None);
        assert_eq!(
            listing_row(
                &own(&["No minds are for sale at this counter."]),
                "Sentry Line"
            ),
            None
        );
    }
}
