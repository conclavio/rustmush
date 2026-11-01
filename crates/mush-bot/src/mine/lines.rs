//! What the mining pilot reads (doc 20 cut SM.12c-bot): pure functions over
//! text, and the needles they match, each the lane's own copy of a sentence
//! the game prints.
//!
//! **The window rule** (the brief's D8). The bot's PTY is
//! [`PTY_COLUMNS`](crate::transport::PTY_COLUMNS) wide and the server wraps
//! any sentence longer than that at a space, so a needle is matched against a
//! **window** — the whole lines received since the line that asked, joined
//! with one space, every whitespace run collapsed to one ([`normalise`]) —
//! never against one physical line. Only a parser that says so reads line
//! heads, and only where the head cannot wrap: `+ship/plan`'s `Burn `,
//! `Stores ` and `Leg home from ` lines, `+market/list`'s `#n` rows, and the
//! site map's legend and grid rows (every `+tv/map` line is at most 118
//! columns).
//!
//! Every needle's doc names the game sentence it mirrors by path and line, as
//! plain text: the game crates are not dependencies. The test at the foot of
//! this file builds each sentence from the game's own `pub` constant and holds
//! the needle and the parser to it.
use serde::{Deserialize, Serialize};

use super::Family;

/// Collapse every whitespace run to one space and trim: the form every needle
/// is matched in.
pub fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `window` (already normalised) contains `needle` (normalised too).
pub fn has(window: &str, needle: &str) -> bool {
    window.contains(&normalise(needle))
}

/// The tails of the world's refusals of a typed line it never ran
/// (mush-server's `world_task.rs`, `RefusalReason::text`, `:992-999`: "The
/// world is committing another action; please retry in a moment.", "The
/// world was busy too long for that line; please retry in a moment." — a line
/// deferred behind a commit for more than 250 ms — and "The world is still
/// starting; please try again shortly."). The same tails the arena reader
/// and the test rig's `TRANSIENTS` retry on.
pub const TRANSIENT_TAILS: [&str; 2] = ["please retry in a moment.", "please try again shortly."];

/// **A line the world refused as transient**: it ends with one of
/// [`TRANSIENT_TAILS`] and does not begin with `[` (a channel line or a
/// notice is another party's text, never the world's refusal of this
/// session's line — the arena reader's rule). The line was never run, so the
/// lane types it again.
pub fn is_transient(line: &str) -> bool {
    let body = line.trim();
    !body.starts_with('[') && TRANSIENT_TAILS.iter().any(|tail| body.ends_with(tail))
}

// ---------------------------------------------------------------------------
// The needles
// ---------------------------------------------------------------------------

/// The berth gallery (`neighborhood.ceres_freeport.json`, room c01).
pub const ROOM_GALLERY: &str = "Visiting Berth Gallery";
/// The pressure lobby (c02).
pub const ROOM_LOBBY: &str = "Pressure Lobby";
/// The arrivals hall (c03).
pub const ROOM_ARRIVALS: &str = "Port Watch Arrivals";
/// The concourse (c04).
pub const ROOM_CONCOURSE: &str = "Freeport Concourse";
/// The Registry & Co-op desk (c05).
pub const ROOM_REGISTRY: &str = "Registry & Co-op Desk";
/// The Exchange Hall (c07).
pub const ROOM_EXCHANGE: &str = "Exchange Hall";
/// The Co-op chandlery (c08).
pub const ROOM_CHANDLERY: &str = "Co-op Chandlery";
/// `tacsim::port::coop::NOT_OPEN` (`coop.rs:125`): the readiness gate is shut.
pub const NOT_OPEN: &str = "The Co-op is not enrolling yet";
/// `tacsim::port::refit::REFITTED_PROSPECTOR` (`refit.rs:51`, "… as a Dray-P: …").
pub const REFITTED: &str = "as a Dray-P";
/// `tacsim::port::licence::LICENCE_PREVIEW`'s tail (`licence.rs:138`).
pub const LICENCE_PREVIEWED: &str = "+licence/buy go buys it.";
/// `licence::LICENCE_IN_FORCE` (`licence.rs:112`).
pub const LICENCE_IN_FORCE: &str = "in force until";
/// `licence::LICENCE_NONE` (`licence.rs:109`).
pub const LICENCE_NONE: &str = "You hold no prospecting licence";
/// `tacsim::port::tv::LEASE_PREVIEW`'s tail (`tv.rs:116`).
pub const LEASE_PREVIEWED: &str = "+tv/lease go signs it.";
/// `tv::LEASED`'s head (`tv.rs:118`).
pub const LEASE_SIGNED: &str = "Occator Water & Propellant leases you ";
/// `tv::LEASE_LICENCE` (`tv.rs:145`).
pub const LEASE_NO_LICENCE: &str = "Occator leases rigs only to a licensed prospector";
/// `tv::LEASE_FUNDS` (`tv.rs:114`).
pub const LEASE_SHORT: &str = "The lease needs ";
/// `tv::RIG_LINE` for rig 1 (`tv.rs:154`).
pub const RIG_ONE: &str = "Rig 1: ";
/// `+ship/plan`'s head (`voyage.rs`'s `plan_block`, "Plan for {port}:").
pub const PLAN_HEAD: &str = "Plan for ";
/// `+market/quote`'s answer (`mush-market` `commands.rs:625`, "Quote #{id}: …").
pub const QUOTED: &str = "Quote #";
/// `+market/accept`'s answer (`commands.rs:724`, "Agreed. Contract #…").
pub const AGREED: &str = "Agreed.";
/// `mush_market::trade::STALE_QUOTE`'s head (`mush-market` `trade.rs:141`, "That
/// quote no longer stands — the terms moved or it lapsed. Ask for a new one;
/// nothing was charged."): another pilot's deal on the same listing between
/// the quote and its acceptance moved the terms (read live by the cohort, two
/// bots bunkering at once; `mush-market` is not a dependency of this crate, so
/// no unit test reads the constant).
pub const QUOTE_STALE: &str = "That quote no longer stands";
/// `+ship/board`'s answer (`ship.rs:936`, "You cycle the lock into the {name}.").
pub const BOARDED: &str = "You cycle the lock into the ";
/// `tacsim::port::voyage::CLEARANCE_FILED`'s head (`voyage.rs:294`).
pub const CLEARANCE: &str = "Clearance filed for ";
/// `voyage::SITE_CLOSED` (`voyage.rs:401`): the site's clerk has not opened its
/// pad yet (`+ship/plan` and `+ship/lift` both answer it until the site's
/// readiness lines are in place).
pub const SITE_CLOSED: &str = "is closed: its pad is not ready";
/// `voyage::GOODS_COMING` (`voyage.rs:322`).
pub const GOODS_WAIT: &str = "have not come aboard yet";
/// `voyage::GOODS_LEAVING` (`voyage.rs:325`).
pub const GOODS_LEAVING: &str = "have not left the hold yet";
/// `+ship/leave`'s answer (`ship.rs:1030`, "You cycle the lock out into the {gallery}.").
pub const ASHORE: &str = "You cycle the lock out into the ";
/// `tv::GROUND_COLD` (`tv.rs:391`): the site's ground is not warm yet.
pub const COLD: &str = "is still being mapped";
/// `tv::SCAN_PREVIEW`'s tail for rig 1 (`tv.rs:307`).
pub const SCAN_PREVIEWED: &str = "+tv/1 scan go starts it.";
/// `site_map::SITE_LEGEND`'s head as printed (`site_map.rs:79`; raw, two spaces).
pub const LEGEND_HEAD: &str = "@ you  H hull";
/// `tv::LAUNCHED` for rig 1 (`tv.rs:209`).
pub const LAUNCHED: &str = "Rig 1 is down the ";
/// `tv::SCANNED` and `tv::SCANNED_ICE` for rig 1 (`tv.rs:342-344`).
pub const SCANNED: &str = "Rig 1 scanned ";
/// `tv::SOUNDED` and `tv::SOUNDED_BARREN` for rig 1 (`tv.rs:347-350`).
pub const SOUNDED: &str = "Rig 1 sounded ";
/// `tv::LEARN_NOTHING` (`tv.rs:363`).
pub const LEARN_NOTHING: &str = "You learn nothing new here.";
/// `tv::SOUND_FILED` and `tv::SOUND_FILED_LAUNCH` for rig 1 (`tv.rs:337-339`,
/// "Rig {n} starts its sounding: done {when}." and "Rig {n} starts down the
/// {ramp} to sound: done {when}."): the sounding was filed and is running.
/// Its `SOUNDED` tell comes when it is done, about 50 real seconds later at
/// rate 24, so the second sounding keeps waiting past its immediate answer
/// once this has arrived, and skips at once when it has not.
pub const SOUND_STARTED: &str = "Rig 1 starts ";
/// `tv::ARRIVED` for rig 1 (`tv.rs:440`), followed by the address.
pub const ARRIVED: &str = "Rig 1 arrives at ";
/// `tv::DRIVE_HERE` for rig 1 (`tv.rs:444`): the rig already stands on the
/// target's cell, so the drive is done without moving.
pub const DRIVE_HERE: &str = "Rig 1 is there already.";
/// `tacsim::port::claims::STAKE_FILED`'s head (`claims.rs:77`).
pub const STAKE_FILED: &str = "You stake block ";
/// `claims::STAKE_RECORDED` (`claims.rs:79`), the site clerk's tell.
pub const STAKE_RECORDED: &str = "records your claim on block ";
/// `tv::MINE_CLAIMED` (`tv.rs:564`): the stake's and the mine's answer on a
/// block someone holds.
pub const CLAIMED: &str = "That block is under claim.";
/// `claims::STAKE_YOURS`'s tail (`claims.rs:84`).
pub const STAKE_YOURS: &str = "is already yours.";
/// `claims::STAKE_LAPSING` (`claims.rs:86`).
pub const STAKE_LAPSING: &str = "claim is lapsing";
/// `claims::STAKE_COOLING` (`claims.rs:89`).
pub const STAKE_COOLING: &str = "takes no stake on it from you before";
/// `claims::STAKE_NO_EVIDENCE` (`claims.rs:91`).
pub const STAKE_NO_EVIDENCE: &str = "You have no reading of your own of block";
/// `claims::STAKE_IN_BAY` (`claims.rs:93`).
pub const STAKE_IN_BAY: &str = "is in the bay; drive it onto the block";
/// `claims::STAKE_COMMONS` (`claims.rs:82`).
pub const STAKE_COMMONS: &str = "Commons ground is never staked.";
/// `claims::CLAIM_FUNDS`'s head (`claims.rs:100`).
pub const CLAIM_FUNDS: &str = "That costs ";
/// `claims::CLAIM_CAP` (`claims.rs:97`).
pub const CLAIM_CAP: &str = "the most one account may";
/// `tv::MINE_PREVIEW` (`tv.rs:504`, "… would mine {addr} …").
pub const WOULD_MINE: &str = " would mine ";
/// `tv::MINE_FILED` for rig 1 (`tv.rs:520`).
pub const DRILLING: &str = "Rig 1 starts drilling ";
/// `tv::MINE_LOCKED` (`tv.rs:566`).
pub const LOCKED: &str = "Another rig is working that pocket.";
/// `tv::MINE_WORKED_OUT` (`tv.rs:572`).
pub const WORKED_OUT: &str = " is worked out for ";
/// `tv::MINE_NO_ORE` (`tv.rs:592`).
pub const NO_ORE: &str = " shows no ";
/// `tv::MINE_ALL_BARREN` (`tv.rs:594`).
pub const ALL_BARREN: &str = " is barren: nothing to mine here.";
/// `tv::MINE_HOPPER_FULL` (`tv.rs:568`).
pub const NO_ROOM: &str = " has no room for another cycle";
/// `tv::MINE_HOLD_FULL` (`tv.rs:570`).
pub const NO_HOLD_ROOM: &str = "has no unreserved room for another cycle";
/// `tv::RECOVER_PREVIEW`'s tail for rig 1 (`tv.rs:222`).
pub const RECOVER_PREVIEWED: &str = "+tv/1 recover go brings it in.";
/// `tv::RECOVERED_UNLOADING` (`tv.rs:656`).
pub const BACK_IN_BAY: &str = "it unloads its hopper into the hold next";
/// `tv::UNLOADED` for rig 1 (`tv.rs:647`).
pub const UNLOADED: &str = "Rig 1 unloads ";
/// `tv::UNLOADED_NONE` for rig 1 (`tv.rs:649`).
pub const UNLOADED_NONE: &str = "Rig 1's hopper still holds ";
/// `tv::UNLOADED_NONE_EMPTY` for rig 1 (`tv.rs:653`).
pub const UNLOADED_EMPTY: &str = "Rig 1 unloads nothing into the ";
/// `+market/sell {n}`'s second line (`proceeds`'s preview, "{line}=all sells {q} for {t}.").
pub const SELL_PREVIEW: &str = "=all sells ";
/// `tacsim::port::proceeds::SALE_LINE`'s head (`proceeds.rs:66`).
pub const SOLD: &str = "Sold ";

/// **`voyage::SETTLES` for this hull** (`voyage.rs:379`, "{name} settles onto
/// the pad at {port}."): `{name}` is the hull object's name, which the
/// enrolment set to the signed name, so matching on it rules out every other
/// pilot's hull on the same route.
pub fn settles(hull: &str) -> String {
    format!("{hull} settles onto the pad at ")
}

/// **`voyage::DOCKS` for this hull** (`voyage.rs:274`, "{name} docks at
/// {port}."), read through the same name as [`settles`].
pub fn docks(hull: &str) -> String {
    format!("{hull} docks at ")
}

/// **Whether `window` holds `prefix` followed by `addr` as a whole address**
/// (normalised): the next char is not a digit, so `F12` never reads as `F123`.
pub fn has_addr(window: &str, prefix: &str, addr: &str) -> bool {
    let needle = normalise(&format!("{prefix}{addr}"));
    window.match_indices(&needle).any(|(at, _)| {
        !window[at + needle.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    })
}

// ---------------------------------------------------------------------------
// The sounding
// ---------------------------------------------------------------------------

/// A reading's class, rank order: rich before fair before lean.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    Rich,
    Fair,
    Lean,
}

impl Class {
    /// The word the game prints.
    pub fn parse(word: &str) -> Option<Class> {
        match word {
            "rich" => Some(Class::Rich),
            "fair" => Some(Class::Fair),
            "lean" => Some(Class::Lean),
            _ => None,
        }
    }
}

/// One pocket of a sounding: its address, its block letter, its family and
/// class, and its position in the sounding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pocket {
    pub addr: String,
    pub block: char,
    pub family: Family,
    pub class: Class,
    pub order: usize,
}

/// A pocket address: one letter `A`–`P`, then 1 to 3 digits.
pub fn is_addr(token: &str) -> bool {
    let mut chars = token.chars();
    let Some(letter) = chars.next() else {
        return false;
    };
    let digits = chars.as_str();
    ('A'..='P').contains(&letter)
        && (1..=3).contains(&digits.len())
        && digits.chars().all(|c| c.is_ascii_digit())
}

/// **The sounding's pockets** (`tv::SOUNDED` and its `"  {family}: {list}"`
/// lines): from the first `"Rig 1 sounded "` in the normalised window, a token
/// scan — a token `metals:` or `volatiles:` sets the family, any other `word:`
/// token clears it (carbon and strategic ore are not this lane's), and an
/// `ADDR CLASS` pair (ADDR one letter `A`–`P` then 1–3 digits; CLASS
/// rich|fair|lean, a trailing `,` or `.` stripped) under a family is a pocket,
/// `order` its position. Wrapped lines therefore read whole, and an
/// interleaved heard line adds nothing. `None` without the head; `Some(vec![])`
/// for `SOUNDED_BARREN`.
pub fn sounding_pockets(window: &str) -> Option<Vec<Pocket>> {
    let window = normalise(window);
    let at = window.find(SOUNDED)?;
    let tokens: Vec<&str> = window[at..].split(' ').collect();
    let mut family = None;
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        if let Some(word) = token.strip_suffix(':') {
            family = match word {
                "metals" => Some(Family::Metals),
                "volatiles" => Some(Family::Volatiles),
                _ => None,
            };
            i += 1;
            continue;
        }
        if let Some(family) = family
            && is_addr(token)
            && let Some(class) = tokens
                .get(i + 1)
                .map(|next| next.trim_end_matches([',', '.']))
                .and_then(Class::parse)
        {
            let block = token.chars().next().unwrap_or('A');
            out.push(Pocket {
                addr: token.to_owned(),
                block,
                family,
                class,
                order: out.len(),
            });
            i += 2;
            continue;
        }
        i += 1;
    }
    Some(out)
}

/// Rich before fair before lean; metals before volatiles at one class; then
/// `order`.
pub fn ranked(pockets: &[Pocket]) -> Vec<Pocket> {
    let mut out = pockets.to_vec();
    out.sort_by_key(|pocket| (pocket.class, pocket.family, pocket.order));
    out
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

/// `+ship/plan`'s figures: the passage's burn and the tank, its crew-days and
/// the locker, and for a site the leg home's burn and crew-days.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanFigures {
    pub burn_t: u64,
    pub tank_t: u64,
    pub crew_days: u64,
    pub locker: u64,
    pub home_burn_t: Option<u64>,
    pub home_days: Option<u64>,
}

/// The first number in `text` (leading spaces skipped).
fn first_number(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse().ok()
}

/// The number at the head of `text`: its leading digits.
fn leading_digits(text: &str) -> Option<u64> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `{head}{a} … {open}{b}` on the first line starting `head` → `(a, b)`.
fn head_pair(lines: &[String], head: &str, open: &str) -> Option<(u64, u64)> {
    let line = lines.iter().find(|line| line.starts_with(head))?;
    let first = first_number(line.strip_prefix(head)?)?;
    let second = leading_digits(line.split(open).nth(1)?)?;
    Some((first, second))
}

/// **`+ship/plan`'s figures**, read from line heads exactly as `sm_story`'s
/// `plan_numbers` and `leg_home` read them: the first line starting `Burn `
/// (its first number, then the number after `(tank `), the first starting
/// `Stores ` with `(locker `, and `Leg home from ` (after `: burn `, the burn
/// and the crew-days) — `home_*` `None` for a plan to Ceres. `None` without
/// `Burn ` and `Stores `. The leg home read is the one `with a full hold`
/// when the plan prices it (`voyage::SITE_HOME_LADEN`, doc 20b RK-1), the
/// figure a captain who means to fill the hold bunkers to; else the leg home
/// the filing holds back.
pub fn plan_figures(lines: &[String]) -> Option<PlanFigures> {
    let (burn_t, tank_t) = head_pair(lines, "Burn ", "(tank ")?;
    let (crew_days, locker) = head_pair(lines, "Stores ", "(locker ")?;
    let leg_home = |line: &&String| line.starts_with("Leg home from ");
    let home = lines
        .iter()
        .filter(leg_home)
        .find(|line| line.contains(" with a full hold ("))
        .or_else(|| lines.iter().find(leg_home))
        .and_then(|line| {
            let rest = line.split(": burn ").nth(1)?;
            let burn = first_number(rest)?;
            let days = first_number(rest.split(", ").nth(1)?)?;
            Some((burn, days))
        });
    Some(PlanFigures {
        burn_t,
        tank_t,
        crew_days,
        locker,
        home_burn_t: home.map(|(burn, _)| burn),
        home_days: home.map(|(_, days)| days),
    })
}

// ---------------------------------------------------------------------------
// The market
// ---------------------------------------------------------------------------

/// One `+market/list` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub number: u64,
    pub wanted: bool,
    pub not_buying: bool,
    pub text: String,
}

/// Every `+market/list` row in `lines` (`mush-market` `commands.rs:360-420`'s
/// `listing_line`): a line starting `#` then digits then a space, `wanted`
/// when it holds " — wanted ", `not_buying` when it ends " — not buying".
pub fn listings(lines: &[String]) -> Vec<Listing> {
    lines
        .iter()
        .filter_map(|line| {
            let rest = line.strip_prefix('#')?;
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() || !rest[digits.len()..].starts_with(' ') {
                return None;
            }
            Some(Listing {
                number: digits.parse().ok()?,
                wanted: line.contains(" — wanted "),
                not_buying: line.ends_with(" — not buying"),
                text: line.clone(),
            })
        })
        .collect()
}

/// The ask whose row holds `label` (case-insensitive) and is not `wanted`:
/// "reaction mass", "provisions". The first such row.
pub fn ask_for(rows: &[Listing], label: &str) -> Option<u64> {
    let label = label.to_lowercase();
    rows.iter()
        .find(|row| !row.wanted && row.text.to_lowercase().contains(&label))
        .map(|row| row.number)
}

/// The bid (`wanted`) whose row holds `family.ore_label()`
/// (case-insensitive): the first such row that is buying, else the first
/// such row, so a board whose only bid is not buying still names it and the
/// caller reads [`Listing::not_buying`] off [`bid_row`] (doc 20b RK-1: it
/// used to take the first row, buying or not, past a buying one).
pub fn bid_for(rows: &[Listing], family: Family) -> Option<u64> {
    bid_row(rows, family).map(|row| row.number)
}

/// The row [`bid_for`] names.
pub fn bid_row(rows: &[Listing], family: Family) -> Option<&Listing> {
    let label = family.ore_label();
    let mut bids = rows
        .iter()
        .filter(|row| row.wanted && row.text.to_lowercase().contains(label));
    let first = bids.clone().next();
    bids.find(|row| !row.not_buying).or(first)
}

/// "Quote #N" → N.
pub fn quote_number(window: &str) -> Option<u64> {
    let at = window.find(QUOTED)?;
    leading_digits(&window[at + QUOTED.len()..])
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

/// The last cycle of a run as its line reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CycleRead {
    pub k: u32,
    pub cycles: u32,
    pub hopper_t: u64,
    pub cap_t: Option<u64>,
    pub ended: bool,
}

/// `tv::CYCLE_DONE`'s head for rig 1 (`tv.rs:524`).
pub const CYCLE_HEAD: &str = "Rig 1, cycle ";
/// `tv::CYCLE_FAILED`'s middle (`tv.rs:545`).
pub const CYCLE_FAILED: &str = " did not happen: ";
/// The run's end clauses (`tv::RUN_DONE`, `RUN_WORKED_OUT`, `RUN_STOPPED` and
/// `RUN_RIGHTS`, `RUN_DARK`; `tv.rs:526-541`).
pub const RUN_ENDS: [&str; 4] = [
    " The run at ",
    " is worked out for ",
    " stops drilling",
    " The registry cannot be read here",
];

/// **The last `CYCLE_DONE`** in the window ("Rig 1, cycle {k} of {cycles}: {t}
/// t {ore}; hopper {hopper}."): `hopper` "empty" → 0, "{t} of {cap} t" → (t,
/// cap); `ended` when `k == cycles`, or the window holds one of the run's end
/// clauses after it, or `CYCLE_FAILED`'s " did not happen: ". A window holding
/// only a failed cycle reads as that cycle's `k` and `cycles`, ended, with no
/// hopper figure (0 and no cap).
pub fn last_cycle(window: &str) -> Option<CycleRead> {
    let window = normalise(window);
    let failed = window.contains(CYCLE_FAILED);
    let Some(at) = window.rfind(CYCLE_HEAD) else {
        if !failed {
            return None;
        }
        // "Rig 1's cycle {k} of {cycles} at {addr} did not happen: …"
        let at = window.find("Rig 1's cycle ")?;
        let rest = &window[at + "Rig 1's cycle ".len()..];
        let k = u32::try_from(leading_digits(rest)?).ok()?;
        let cycles = u32::try_from(leading_digits(rest.split(" of ").nth(1)?)?).ok()?;
        return Some(CycleRead {
            k,
            cycles,
            hopper_t: 0,
            cap_t: None,
            ended: true,
        });
    };
    let rest = &window[at + CYCLE_HEAD.len()..];
    let k = u32::try_from(leading_digits(rest)?).ok()?;
    let cycles = u32::try_from(leading_digits(rest.split(" of ").nth(1)?)?).ok()?;
    let hopper = rest.split("; hopper ").nth(1)?;
    let (hopper_t, cap_t) = if hopper.starts_with("empty") {
        (0, None)
    } else {
        let t = leading_digits(hopper)?;
        let cap = hopper.split(" of ").nth(1).and_then(leading_digits);
        (t, cap)
    };
    let ended = k >= cycles || failed || RUN_ENDS.iter().any(|end| rest.contains(end));
    Some(CycleRead {
        k,
        cycles,
        hopper_t,
        cap_t,
        ended,
    })
}

// ---------------------------------------------------------------------------
// The heard sounding
// ---------------------------------------------------------------------------

/// `tv::HEARD`'s and `tv::HEARD_HERE`'s shared head (`tv.rs:353-355`).
pub const HEARD_HEAD: &str = "A sounding thumps through the ground";
/// The eight octant words (`tacsim::surface::Octant::word`, `address.rs:380`).
pub const OCTANT_WORDS: [&str; 8] = [
    "north",
    "northeast",
    "east",
    "southeast",
    "south",
    "southwest",
    "west",
    "northwest",
];
/// The three range bands (`tv::BAND_NEAR`, `BAND_MID`, `BAND_FAR`, `tv.rs:357-361`).
pub const BANDS: [&str; 3] = ["under 500 m off", "500 m to 1 km off", "over 1 km off"];

/// **A heard sentence in one received line**: the text from [`HEARD_HEAD`] to
/// the line's end, or `None`.
pub fn heard_sentence(line: &str) -> Option<&str> {
    line.find(HEARD_HEAD).map(|at| line[at..].trim_end())
}

/// **Every exact heard sentence** the game can print: `HEARD` over the eight
/// octant words × the three bands, and `HEARD_HERE` — 25 strings, built from
/// the lane's own copies of the words.
pub fn heard_sentences() -> Vec<String> {
    let mut out = Vec::with_capacity(25);
    for bearing in OCTANT_WORDS {
        for band in BANDS {
            out.push(format!("{HEARD_HEAD} from the {bearing}, {band}."));
        }
    }
    out.push(format!("{HEARD_HEAD} right under you."));
    out
}

/// **A heard line carries nothing but the sentence**: the sentence runs to the
/// line's end (its one full stop is its last char, so
/// [`heard_sentence`] returned all of it and nothing follows it) and the text
/// before [`HEARD_HEAD`] holds no letter and no digit. Line mode prints the tag
/// of the game's `sys_to_actor` as style only, so in play the prefix is empty;
/// the allowance for punctuation and whitespace (a prompt's "> " the assembler
/// joined onto the line) is defensive. A name, a coordinate or a `[tag]` before
/// the head is not bare.
pub fn heard_line_is_bare(line: &str) -> bool {
    let Some(at) = line.find(HEARD_HEAD) else {
        return false;
    };
    let prefix = &line[..at];
    let sentence = line[at..].trim_end();
    !prefix.chars().any(char::is_alphanumeric) && sentence.find('.') == Some(sentence.len() - 1)
}

// ---------------------------------------------------------------------------
// The site map
// ---------------------------------------------------------------------------

/// The site view's side in cells (the lane's copy of
/// `site_map::SITE_VIEW_CELLS`, `site_map.rs:69`).
pub const SITE_VIEW_CELLS: usize = 32;
/// The widest grid row: [`SITE_VIEW_CELLS`] cells of two columns each
/// (`site_map_lines`' `max(cell_columns, 2)`).
pub const GRID_COLUMNS: usize = 64;
/// Doc 20 §10's legend verbatim (the lane's copy of `site_map::SITE_LEGEND`,
/// `site_map.rs:79`).
pub const SITE_LEGEND: &str = "@ you  H hull  o rig  , regolith  ^ rock  = ice  O crater  1–3 ore (t a cycle)  + other family  . barren  v/V pit  # claim";
/// Every glyph a site-view grid row may hold: [`SITE_LEGEND`]'s, `_` (a cell
/// with no footing that the tracked locomotion passes) and a space (an
/// impassable cell) — `site_map.rs`'s `terrain_glyph`, `:885-899` — and the
/// sixteen block letters `A`–`P` the site view prints at each block's
/// north-west corner (`site_map.rs`'s letters layer, `address::letter_of_block`;
/// read live by the canary, the legend does not name them).
pub const GRID_GLYPHS: &str = "@Ho,^=O123+.vV#_ ABCDEFGHIJKLMNOP";
/// The glyphs only a reading draws (SM.4b D12's `ORE_GLYPHS`,
/// `crates/tacsim/tests/driving.rs:2939`).
pub const ORE_GLYPHS: [char; 5] = ['1', '2', '3', '+', '.'];

/// **The grid rows of a `+tv/map site` print, by position**: the first line
/// starting with `"@ you  H hull"` (the legend's head, matched on the raw line)
/// opens the legend; when that line and the next, concatenated, equal
/// [`SITE_LEGEND`] (the game splits it in two), the next is the legend's tail
/// and is skipped; when the first line alone equals it, nothing is skipped.
/// **The [`SITE_VIEW_CELLS`] lines after the legend are the grid** — each, as
/// the assembler trimmed it, at most [`GRID_COLUMNS`] chars and made only of
/// [`GRID_GLYPHS`] (so a row trimmed short by trailing impassable cells, or
/// trimmed to `""` when it is all impassable, is still a row). Empty when there
/// is no legend (a cold site's `GROUND_COLD`), when fewer than
/// [`SITE_VIEW_CELLS`] lines follow it yet, or when any of those lines breaks
/// the rule (the print is not a site view).
pub fn grid_rows(lines: &[String]) -> Vec<String> {
    let Some(head) = lines.iter().position(|line| line.starts_with(LEGEND_HEAD)) else {
        return Vec::new();
    };
    let first = lines[head].as_str();
    let grid_start = if first == SITE_LEGEND {
        head + 1
    } else if lines
        .get(head + 1)
        .is_some_and(|tail| format!("{first}{tail}") == SITE_LEGEND)
    {
        head + 2
    } else {
        return Vec::new();
    };
    let Some(rows) = lines.get(grid_start..grid_start + SITE_VIEW_CELLS) else {
        return Vec::new();
    };
    let whole = rows.iter().all(|row| {
        row.chars().count() <= GRID_COLUMNS && row.chars().all(|c| GRID_GLYPHS.contains(c))
    });
    if whole { rows.to_vec() } else { Vec::new() }
}

// ---------------------------------------------------------------------------
// The sale
// ---------------------------------------------------------------------------

/// The sale's receipt as its line reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaleRead {
    pub qty_t: u64,
    pub gross: String,
    pub share: Option<String>,
    pub net: String,
}

/// **The sale's receipt** (`proceeds::SALE_LINE` / `SALE_LINE_NO_SHARE`,
/// `proceeds.rs:66-68`): from "Sold " in the window, the quantity, the gross
/// after the first ": ", the share after "Co-op share ", the net after "you
/// receive ". `None` until the sentence is whole (its " when the goods are
/// in" clause read).
pub fn sale_receipt(window: &str) -> Option<SaleRead> {
    let window = normalise(window);
    let at = window.find(SOLD)?;
    let sentence = &window[at..];
    let end = sentence.find(" when the goods are in")?;
    let sentence = &sentence[..end];
    let qty_t = first_number(&sentence[SOLD.len()..])?;
    let after = sentence.split_once(": ")?.1;
    let gross = after.split(';').next()?.trim().to_owned();
    let share = after
        .split_once("Co-op share ")
        .and_then(|(_, rest)| rest.split(';').next())
        .map(|share| share.trim().to_owned());
    let net = after.split_once("you receive ")?.1.trim().to_owned();
    Some(SaleRead {
        qty_t,
        gross,
        share,
        net,
    })
}

// ---------------------------------------------------------------------------
// The IC field (doc 32 G-4): what a captain riding a rig's cab reads
// ---------------------------------------------------------------------------

/// `tacsim::field::FIELD_OPENED`'s tail after the site's name (`field/mod.rs`), told to
/// everyone on the site when it promotes.
pub const FIELD_OPENED_TAIL: &str = ": the ground there is tactical until it settles.";
/// `tacsim::field::FIELD_SETTLED`'s tail after the site's name: the field demoted.
pub const FIELD_SETTLED_TAIL: &str = " has settled: the ground there is the surface's again.";
/// `tacsim::field::NO_FIELD`: `+field` with no party of the actor's in a field.
pub const NO_FIELD: &str = "No field holds a party of yours.";
/// `tacsim::field::FIELD_STATUS`'s middle, before the party's letter.
pub const YOUR_PARTY: &str = "Your party is ";
/// `tacsim::field::PARTY_MARKED`'s head, the mark's acknowledgement.
pub const PARTY_MARKED: &str = "Your party now marks party ";
/// `tacsim::surface::site_record::FIELD_OPEN`'s tail: surface work refused while a field runs.
pub const SURFACE_WAITS: &str = "Surface work there waits until it settles.";
/// `tacsim::detachment::sortie::ORDER_PREFILL`'s middle, before the fence.
pub const ORDER_PREFILL: &str = "Order prefill: ";
/// `tacsim::port::tv::RECOVERED`'s head for rig 1: home with nothing to unload.
pub const RIG_BACK: &str = "Rig 1 is back in the ";
/// `tacsim::port::voyage::LEAVE_HINT`'s head, after a live departure's refusal.
pub const LEAVE_HINT: &str = "To lift without them, add leave=";
/// The party letters a field draws (`tacsim::arena::party::LETTERS`).
pub const PARTY_LETTERS: &str = "ABCDEFGHJKLMNPQRSTUVWYZ";

/// **The field that opened on `site`** in `window` (normalised): its ordinal, from
/// `Field {n} is open on {site}: …`.
pub fn field_opened(window: &str, site: &str) -> Option<u32> {
    ordinal_before(window, &format!(" is open on {site}{FIELD_OPENED_TAIL}"))
}

/// **The field on `site` that settled** in `window`: its ordinal.
pub fn field_settled(window: &str, site: &str) -> Option<u32> {
    ordinal_before(window, &format!(" on {site}{FIELD_SETTLED_TAIL}"))
}

/// The `Field {n}` immediately before `tail`.
fn ordinal_before(window: &str, tail: &str) -> Option<u32> {
    let (head, _) = window.split_once(&normalise(tail))?;
    let (_, n) = head.rsplit_once("Field ")?;
    n.trim_end().parse().ok()
}

/// **The actor's own party letter**, from `+field`'s `Your party is {letter};`.
pub fn own_party(window: &str) -> Option<char> {
    let (_, rest) = window.split_once(YOUR_PARTY)?;
    let letter = rest.chars().next()?;
    (rest[letter.len_utf8()..].starts_with(';') && PARTY_LETTERS.contains(letter)).then_some(letter)
}

/// **The party a `CONTACTS` row's class word names** (`tacsim::arena::party::PartyId::as_str`):
/// `red` and `blue` for `R` and `B`, else the lower-case letter; `unknown` (not yet
/// classified) and anything else is `None`.
pub fn party_of_word(word: &str) -> Option<char> {
    match word {
        "red" => Some('R'),
        "blue" => Some('B'),
        _ => {
            let mut chars = word.chars();
            let letter = chars.next()?.to_ascii_uppercase();
            (chars.next().is_none() && word.chars().all(|c| c.is_ascii_lowercase()))
                .then_some(letter)
                .filter(|letter| PARTY_LETTERS.contains(*letter))
        }
    }
}

/// **A field designator**: a party letter and one or two unit letters (`BD`, `BKQ`), or a
/// provisional `X` and a number (`X1`).
pub fn is_field_designator(word: &str) -> bool {
    if let Some(number) = word.strip_prefix('X') {
        return (1..=6).contains(&number.len()) && number.bytes().all(|b| b.is_ascii_digit());
    }
    (2..=3).contains(&word.len()) && word.chars().all(|c| PARTY_LETTERS.contains(c))
}

/// **One `CONTACTS` row in a field** (`tacsim::arena::view::ContactRow::render`):
/// `BD  blue  bearing 090° (rel 000°)  beyond  100 m  0 s  rear` — the designator and the
/// party its class word names, `None` while it reads `unknown`.
pub fn field_contact(line: &str) -> Option<(String, Option<char>)> {
    let mut words = line.split_whitespace();
    let designator = words.next().filter(|w| is_field_designator(w))?;
    let class = words.next()?;
    if words.next() != Some("bearing") {
        return None;
    }
    Some((designator.to_owned(), party_of_word(class)))
}

/// **The replies `+sortie/orders` lists for slot `slot`**
/// (`tacsim::detachment::sortie::ORDER_SENT`): each order's sequence and its reply word
/// (`WILCO`, `NEGATIVE`), or `None` while no reply is confirmed.
pub fn order_replies(window: &str, slot: u8) -> Vec<(u64, Option<String>)> {
    let head = format!("Sent slot {slot} order ");
    window
        .split(head.as_str())
        .skip(1)
        .filter_map(|rest| {
            let (sequence, rest) = rest.split_once(": ")?;
            let sequence = sequence.parse().ok()?;
            let word = rest
                .split_once(" received.")
                .map(|(word, _)| word)
                .filter(|word| word.bytes().all(|b| b.is_ascii_uppercase()))
                .map(str::to_owned);
            Some((sequence, word))
        })
        .collect()
}

/// **The escort's report to its captain** (doc 32 G-4), the bots' own words sent by `page`:
/// `Raider: party {party}, contact {designator}.` Line text gives no relation, and a captain in a
/// field of several parties cannot tell the raider's from its escort's; the escort, which looks
/// at each armed contact, says which it is fighting.
pub const RAIDER_REPORT: &str = "Raider: party {party}, contact {designator}.";

/// The page that carries it, as the line client prints it (mush-ansi's `Page` narrative):
/// `{sender} pages: …`.
pub const PAGED: &str = " pages: ";

/// **The raider a page in `line` reports**: its party letter and designator.
pub fn raider_report(line: &str) -> Option<(char, String)> {
    let (_, message) = line.split_once(PAGED)?;
    let rest = message.trim().strip_prefix("Raider: party ")?;
    let (party, rest) = rest.split_once(", contact ")?;
    let designator = rest.strip_suffix('.')?;
    let mut letters = party.chars();
    let party = letters.next().filter(|p| PARTY_LETTERS.contains(*p))?;
    (letters.next().is_none() && is_field_designator(designator) && designator.starts_with(party))
        .then(|| (party, designator.to_owned()))
}

/// **The captain's word to its escort** (doc 32 G-4), by `page`: `Captain: party {party}.` The
/// escort sets that party aside, with its own and every party its `LOCK` reads `(FRIENDLY)`, and
/// the one left is the raider.
pub const CAPTAIN_REPORT: &str = "Captain: party {party}.";

/// **The captain's party a page in `line` reports.**
pub fn captain_report(line: &str) -> Option<char> {
    let (_, message) = line.split_once(PAGED)?;
    let party = message
        .trim()
        .strip_prefix("Captain: party ")?
        .strip_suffix('.')?;
    let mut letters = party.chars();
    let letter = letters.next().filter(|p| PARTY_LETTERS.contains(*p))?;
    letters.next().is_none().then_some(letter)
}

/// **Whether a `LOCK`'s answer marks its contact a friend** (`tacsim`'s lock line, `Locked on BA
/// (FRIENDLY), 62% to hit.`): own party or a standing friend, never to be fired on.
pub fn lock_is_friendly(window: &str) -> bool {
    window.split("Locked on ").skip(1).any(|rest| {
        rest.split([',', '.'])
            .next()
            .is_some_and(|head| head.contains("(FRIENDLY)"))
    })
}

/// **The order fence** `ORDER DT:a:b:c` from `+sortie/orders`' prefill line.
pub fn order_fence(window: &str) -> Option<String> {
    let (_, rest) = window.split_once(ORDER_PREFILL)?;
    let (fence, _) = rest.split_once(" ALL HOLD")?;
    fence.starts_with("ORDER DT:").then(|| fence.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Assembler;
    use tacsim::port::{claims, coop, licence, proceeds, site_map, tv, voyage};
    use tacsim::surface::Octant;

    /// Split `text` at spaces so no line passes `width` columns, the way the
    /// server's renderer wraps a long sentence.
    fn wrap(text: &str, width: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut line = String::new();
        for word in text.split(' ') {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
        out
    }

    fn joined(lines: &[String]) -> String {
        normalise(&lines.join(" "))
    }

    /// **Every parser, fed the game's own sentences** (doc 20 cut SM.12c-bot,
    /// B6 test 1): each needle a substring of the filled sentence it mirrors,
    /// so a changed game sentence breaks this test, not a loop.
    #[test]
    fn mine_lines_read_the_games_own_sentences() {
        // The rig's tells.
        let launched = tv::LAUNCHED
            .replace("{n}", "1")
            .replace("{ramp}", "stern ramp")
            .replace("{site}", "Ahuna Flank");
        assert!(launched.contains(LAUNCHED), "{launched}");
        let scanned = tv::SCANNED
            .replace("{n}", "1")
            .replace("{count}", "9")
            .replace("{footing}", "rock")
            .replace("{hidden}", "");
        assert!(scanned.contains(SCANNED), "{scanned}");
        let scanned_ice = tv::SCANNED_ICE.replace("{n}", "1").replace("{count}", "9");
        assert!(scanned_ice.contains(SCANNED));
        let preview = tv::SCAN_PREVIEW.replace("{n}", "1");
        assert!(preview.contains(SCAN_PREVIEWED), "{preview}");
        let cold = tv::GROUND_COLD.replace("{site}", "Ahuna Flank");
        assert!(cold.contains(COLD));

        // The sounding: three families, wrapped at 120 columns at spaces.
        let head = tv::SOUNDED
            .replace("{n}", "1")
            .replace("{count}", "64")
            .replace("{addr}", "K1")
            .replace("{quietly}", "");
        assert!(head.contains(SOUNDED));
        let metals: Vec<String> = (1..=14)
            .map(|n| format!("F{n} {}", ["rich", "fair", "lean"][n % 3]))
            .collect();
        let block = [
            head.clone(),
            format!("  metals: {}", metals.join(", ")),
            "  volatiles: G12 fair, K3 lean".to_owned(),
            "  carbon feedstock: J7 rich, J8 fair".to_owned(),
        ];
        let mut received = Vec::new();
        for line in &block {
            received.extend(wrap(line, 120));
        }
        assert!(received.len() > block.len(), "the block wrapped");
        let pockets = sounding_pockets(&joined(&received)).expect("the head");
        assert_eq!(pockets.len(), 16, "{pockets:?}");
        assert!(pockets[..14].iter().all(|p| p.family == Family::Metals));
        assert_eq!(pockets[14].addr, "G12");
        assert_eq!(pockets[14].family, Family::Volatiles);
        assert_eq!(pockets[14].class, Class::Fair);
        assert_eq!(pockets[15].block, 'K');
        assert!(
            !pockets.iter().any(|p| p.block == 'J'),
            "carbon is not read"
        );
        let best = ranked(&pockets);
        assert_eq!(best[0].class, Class::Rich);
        assert_eq!(best[0].family, Family::Metals);
        let barren = tv::SOUNDED_BARREN
            .replace("{n}", "1")
            .replace("{count}", "64")
            .replace("{addr}", "K1")
            .replace("{quietly}", "");
        assert_eq!(sounding_pockets(&barren), Some(Vec::new()));
        assert_eq!(sounding_pockets("Rig 1 scanned 9 fields"), None);
        assert!(tv::LEARN_NOTHING.contains(LEARN_NOTHING));
        // The sounding filed: the second sounding waits on for its tell.
        for filed in [tv::SOUND_FILED, tv::SOUND_FILED_LAUNCH] {
            let filed = filed
                .replace("{n}", "1")
                .replace("{ramp}", "stern ramp")
                .replace("{when}", "in 50 s");
            assert!(filed.starts_with(SOUND_STARTED), "{filed}");
        }

        // The world's transient refusals (mush-server's `RefusalReason::text`,
        // private, so spelled here as the arena reader's test spells them):
        // typed again; another party's channel line or a game sentence that
        // merely says "try again shortly" is not one.
        for refusal in [
            "The world is committing another action; please retry in a moment.",
            "The world was busy too long for that line; please retry in a moment.",
            "The world is still starting; please try again shortly.",
            "> The world was busy too long for that line; please retry in a moment.",
        ] {
            assert!(is_transient(refusal), "{refusal}");
        }
        let pending = claims::CLAIM_PENDING
            .replace("{block}", "F")
            .replace("{site}", "Ahuna Flank");
        for other in [
            "[Public] Ada: the world is busy; please retry in a moment.",
            pending.as_str(),
            "Rig 1 sounded 64 pockets around K1; every pocket not named reads barren:",
        ] {
            assert!(!is_transient(other), "{other}");
        }

        // The drive.
        let arrived = tv::ARRIVED
            .replace("{n}", "1")
            .replace("{addr}", "F12")
            .replace("{reading}", ", fair metals");
        assert!(has_addr(&normalise(&arrived), ARRIVED, "F12"));
        assert!(!has_addr(&normalise(&arrived), ARRIVED, "F1"));
        assert!(tv::DRIVE_HERE.replace("{n}", "1").contains(DRIVE_HERE));

        // The run.
        let cycle = |k: u32, cycles: u32, hopper: &str| {
            tv::CYCLE_DONE
                .replace("{n}", "1")
                .replace("{k}", &k.to_string())
                .replace("{cycles}", &cycles.to_string())
                .replace("{t}", "2")
                .replace("{ore}", "metal ore, run-of-mine")
                .replace("{hopper}", hopper)
        };
        let mid = cycle(2, 6, "4 of 12 t");
        assert_eq!(
            last_cycle(&mid),
            Some(CycleRead {
                k: 2,
                cycles: 6,
                hopper_t: 4,
                cap_t: Some(12),
                ended: false
            })
        );
        assert_eq!(
            last_cycle(&cycle(1, 1, "empty")).map(|c| (c.hopper_t, c.ended)),
            Some((0, true))
        );
        let done = format!(
            "{}{}",
            cycle(3, 6, "6 of 12 t"),
            tv::RUN_DONE.replace("{addr}", "F12")
        );
        assert!(last_cycle(&done).is_some_and(|c| c.ended));
        let worked = format!(
            "{} {}{}",
            mid,
            cycle(3, 6, "6 of 12 t"),
            tv::RUN_WORKED_OUT
                .replace("{addr}", "F12")
                .replace("{ore}", "metal ore, run-of-mine")
        );
        assert_eq!(
            last_cycle(&worked).map(|c| (c.k, c.hopper_t, c.ended)),
            Some((3, 6, true))
        );
        for end in [
            tv::RUN_STOPPED.replace("{n}", "1"),
            tv::RUN_RIGHTS
                .replace("{n}", "1")
                .replace("{addr}", "F12")
                .replace("{line}", tv::MINE_CLAIMED),
            tv::RUN_DARK.replace("{addr}", "F12"),
        ] {
            let line = format!("{}{end}", cycle(2, 6, "4 of 12 t"));
            assert!(last_cycle(&line).is_some_and(|c| c.ended), "{line}");
        }
        let failed = tv::CYCLE_FAILED
            .replace("{n}", "1")
            .replace("{k}", "1")
            .replace("{cycles}", "6")
            .replace("{addr}", "F12")
            .replace("{why}", tv::WHY_UNWORKED);
        assert!(last_cycle(&failed).is_some_and(|c| c.ended && c.k == 1));
        assert_eq!(last_cycle("Rig 1 starts drilling F12"), None);
        let preview = tv::MINE_PREVIEW
            .replace("{n}", "1")
            .replace("{addr}", "F12");
        assert!(preview.contains(WOULD_MINE));
        let filed = tv::MINE_FILED.replace("{n}", "1").replace("{addr}", "F12");
        assert!(filed.contains(DRILLING));
        assert!(tv::MINE_LOCKED.contains(LOCKED));
        assert!(
            tv::MINE_WORKED_OUT
                .replace("{addr}", "F12")
                .contains(WORKED_OUT)
        );
        assert!(tv::MINE_NO_ORE.replace("{addr}", "F12").contains(NO_ORE));
        assert!(
            tv::MINE_ALL_BARREN
                .replace("{addr}", "F12")
                .contains(ALL_BARREN)
        );
        assert!(tv::MINE_HOPPER_FULL.replace("{n}", "1").contains(NO_ROOM));
        assert!(tv::MINE_HOLD_FULL.contains(NO_HOLD_ROOM));

        // The recover and the unload.
        let recover = tv::RECOVER_PREVIEW.replace("{n}", "1");
        assert!(recover.contains(RECOVER_PREVIEWED));
        let back = tv::RECOVERED_UNLOADING
            .replace("{n}", "1")
            .replace("{name}", "Ardent Lode");
        assert!(back.contains(BACK_IN_BAY));
        let unloaded = tv::UNLOADED
            .replace("{n}", "1")
            .replace("{load}", "12 t of metal ore, run-of-mine")
            .replace("{name}", "Ardent Lode")
            .replace("{hopper}", "empty");
        assert!(unloaded.contains(UNLOADED));
        assert!(!unloaded.contains(UNLOADED_EMPTY));
        assert!(
            tv::UNLOADED_NONE
                .replace("{n}", "1")
                .contains(UNLOADED_NONE)
        );
        assert!(
            tv::UNLOADED_NONE_EMPTY
                .replace("{n}", "1")
                .contains(UNLOADED_EMPTY)
        );

        // The stake.
        let filed = claims::STAKE_FILED
            .replace("{block}", "F")
            .replace("{site}", "Ahuna Flank");
        assert!(filed.contains(STAKE_FILED));
        let recorded = claims::STAKE_RECORDED
            .replace("{issuer}", "Freeport Registry")
            .replace("{block}", "F")
            .replace("{site}", "Ahuna Flank");
        assert!(recorded.contains(STAKE_RECORDED));
        assert!(tv::MINE_CLAIMED.contains(CLAIMED));
        assert!(claims::STAKE_YOURS.contains(STAKE_YOURS));
        assert!(claims::STAKE_LAPSING.contains(STAKE_LAPSING));
        assert!(claims::STAKE_COOLING.contains(STAKE_COOLING));
        assert!(claims::STAKE_NO_EVIDENCE.contains(STAKE_NO_EVIDENCE));
        assert!(
            claims::STAKE_IN_BAY
                .replace("{n}", "1")
                .contains(STAKE_IN_BAY)
        );
        assert!(claims::STAKE_COMMONS.contains(STAKE_COMMONS));
        assert!(claims::CLAIM_CAP.contains(CLAIM_CAP));
        assert!(claims::CLAIM_FUNDS.contains(CLAIM_FUNDS));

        // The passage.
        assert!(voyage::CLEARANCE_FILED.contains(CLEARANCE));
        assert!(voyage::GOODS_COMING.contains(GOODS_WAIT));
        assert!(voyage::SITE_CLOSED.contains(SITE_CLOSED));
        assert!(voyage::GOODS_LEAVING.contains(GOODS_LEAVING));
        for (template, needle) in [
            (voyage::SETTLES, settles as fn(&str) -> String),
            (voyage::DOCKS, docks),
        ] {
            let line = template
                .replace("{name}", "Ardent Lode")
                .replace("{port}", "Ahuna Flank");
            assert!(line.contains(&needle("Ardent Lode")), "{line}");
            let other = template
                .replace("{name}", "Cobalt Lode")
                .replace("{port}", "Ahuna Flank");
            assert!(!other.contains(&needle("Ardent Lode")), "{other}");
        }

        // The desk.
        assert!(coop::NOT_OPEN.contains(NOT_OPEN));
        assert!(tacsim::port::refit::REFITTED_PROSPECTOR.contains(REFITTED));
        assert_eq!(super::super::HULL_NAME_RULE, coop::BAD_NAME);
        assert!(licence::LICENCE_IN_FORCE.contains(LICENCE_IN_FORCE));
        assert!(licence::LICENCE_NONE.contains(LICENCE_NONE));
        assert!(licence::LICENCE_PREVIEW.contains(LICENCE_PREVIEWED));
        assert!(tv::LEASED.contains(LEASE_SIGNED));
        assert!(tv::LEASE_PREVIEW.contains(LEASE_PREVIEWED));
        assert!(tv::LEASE_LICENCE.contains(LEASE_NO_LICENCE));
        assert!(tv::LEASE_FUNDS.contains(LEASE_SHORT));
        assert!(tv::RIG_LINE.replace("{n}", "1").contains(RIG_ONE));

        // The sale.
        let sold = proceeds::SALE_LINE
            .replace("{qty}", "12")
            .replace("{unit}", "t")
            .replace("{label}", "metal ore, run-of-mine")
            .replace("{house}", "Freeport Exchange")
            .replace("{price}", "70.00cr")
            .replace("{gross}", "840.00cr")
            .replace("{share}", "84.00cr")
            .replace("{net}", "756.00cr")
            .replace("{signatory}", "The Co-op");
        let read = sale_receipt(&joined(&wrap(&sold, 120))).expect("the receipt");
        assert_eq!(
            read,
            SaleRead {
                qty_t: 12,
                gross: "840.00cr".to_owned(),
                share: Some("84.00cr".to_owned()),
                net: "756.00cr".to_owned(),
            }
        );
        let no_share = proceeds::SALE_LINE_NO_SHARE
            .replace("{qty}", "7")
            .replace("{unit}", "t")
            .replace("{label}", "ice ore, run-of-mine")
            .replace("{house}", "Occator Water & Propellant")
            .replace("{price}", "35.00cr")
            .replace("{gross}", "245.00cr")
            .replace("{signatory}", "Occator");
        let read = sale_receipt(&no_share).expect("the receipt");
        assert_eq!(
            (read.qty_t, read.share, read.net.as_str()),
            (7, None, "245.00cr")
        );

        // The market: an ask, a bid and a bid not buying, in `listing_line`'s shapes.
        let list: Vec<String> = [
            "#3 reaction mass — 500 × 40.00cr [exchange] at Occator Water & Propellant",
            "#4 voyage provisions — 200 × 10.00cr [exchange] at Ceres Co-op",
            "#7 metal ore, run-of-mine — wanted 240 × 70.00cr [exchange] by Freeport Exchange",
            "#8 ice ore, run-of-mine — wanted 0 × 35.00cr [exchange] by Occator Water & Propellant — not buying",
            "  a wrapped tail",
            "#x not a row",
        ]
        .map(str::to_owned)
        .to_vec();
        let rows = listings(&list);
        assert_eq!(rows.len(), 4);
        assert_eq!(ask_for(&rows, "reaction mass"), Some(3));
        assert_eq!(ask_for(&rows, "Provisions"), Some(4));
        assert_eq!(bid_for(&rows, Family::Metals), Some(7));
        assert_eq!(bid_for(&rows, Family::Volatiles), Some(8));
        assert!(bid_row(&rows, Family::Volatiles).is_some_and(|row| row.not_buying));
        assert!(!bid_row(&rows, Family::Metals).is_some_and(|row| row.not_buying));
        // A buying bid listed after one that is not is the one taken (doc 20b RK-1).
        let two: Vec<String> = [
            "#5 ice ore, run-of-mine — wanted 0 × 35.00cr [exchange] by Freeport Exchange — not buying",
            "#8 ice ore, run-of-mine — wanted 120 × 35.00cr [exchange] by Occator Water & Propellant",
        ]
        .map(str::to_owned)
        .to_vec();
        let two = listings(&two);
        assert_eq!(bid_for(&two, Family::Volatiles), Some(8));
        assert!(!bid_row(&two, Family::Volatiles).is_some_and(|row| row.not_buying));
        assert_eq!(quote_number("Quote #42: 5 × 40.00cr = 200.00cr"), Some(42));

        // The plan, in `voyage.rs`'s shapes, with the leg home.
        let plan: Vec<String> = [
            "Plan for Ahuna Flank:".to_owned(),
            "Burn 3 t (tank 0 t; 3 t short). Flip and burn.".to_owned(),
            "Stores 1 crew-days (locker 0; 1 short).".to_owned(),
            voyage::SITE_HOME
                .replace("{port}", "Ahuna Flank")
                .replace("{burn}", "2")
                .replace("{days}", "1")
                .replace("{throttle}", "0.5"),
        ]
        .to_vec();
        assert_eq!(
            plan_figures(&plan),
            Some(PlanFigures {
                burn_t: 3,
                tank_t: 0,
                crew_days: 1,
                locker: 0,
                home_burn_t: Some(2),
                home_days: Some(1),
            })
        );
        assert_eq!(plan_figures(&plan[..3]).and_then(|p| p.home_burn_t), None);
        assert_eq!(plan_figures(&plan[..2]), None);
        // With the full-hold line too, its figures are the leg home read (doc 20b RK-1).
        let mut laden = plan.clone();
        laden.push(
            voyage::SITE_HOME_LADEN
                .replace("{port}", "Ahuna Flank")
                .replace("{cargo}", "40")
                .replace("{burn}", "3")
                .replace("{days}", "2")
                .replace("{throttle}", "0.5"),
        );
        assert_eq!(
            plan_figures(&laden).map(|p| (p.home_burn_t, p.home_days)),
            Some((Some(3), Some(2)))
        );

        // The site map, received through the assembler as the lane receives it.
        assert_eq!(SITE_LEGEND, site_map::SITE_LEGEND);
        assert_eq!(SITE_VIEW_CELLS, site_map::SITE_VIEW_CELLS);
        assert_eq!(GRID_COLUMNS, 2 * site_map::SITE_VIEW_CELLS);
        // Every legend entry's glyph (its first word: `@`, `H`, …, `1–3`,
        // `v/V`, `#`) is a grid glyph.
        let mut entries = 0;
        for entry in site_map::SITE_LEGEND.split("  ") {
            let glyph = entry.split(' ').next().expect("an entry's glyph");
            let glyphs: Vec<char> = match glyph {
                "1–3" => vec!['1', '2', '3'],
                "v/V" => vec!['v', 'V'],
                single => single.chars().collect(),
            };
            assert!(!glyphs.is_empty(), "{entry:?}");
            for c in glyphs {
                assert!(GRID_GLYPHS.contains(c), "{c:?} of {entry:?}");
            }
            entries += 1;
        }
        assert_eq!(entries, 12, "the legend's twelve entries");
        // The legend split as `legend_lines` splits it.
        let cut = site_map::SITE_LEGEND
            .match_indices("  ")
            .map(|(at, _)| at)
            .filter(|at| site_map::SITE_LEGEND[..*at].chars().count() <= site_map::MAX_LINE_COLUMNS)
            .last()
            .expect("a cut");
        let mut print = vec![
            site_map::MAP_TITLE_SITE
                .replace("{site}", "Ahuna Flank")
                .replace("{family}", "metals"),
            site_map::SITE_LEGEND[..cut].to_owned(),
            site_map::SITE_LEGEND[cut..].to_owned(),
        ];
        let mut grid = Vec::new();
        for row in 0..SITE_VIEW_CELLS {
            let cells: String = (0..SITE_VIEW_CELLS)
                .map(|col| match (row, col) {
                    // A block's letter at its north-west corner.
                    (0, 0) => 'A',
                    (8, 8) => 'F',
                    (3, _) if col % 4 == 0 => '_',
                    (5, c) if c >= 20 => ' ',
                    (9, _) => ' ',
                    (16, 16) => 'H',
                    (_, c) if c % 5 == 0 => '^',
                    _ => ',',
                })
                .collect();
            grid.push(cells.chars().flat_map(|c| [c, c]).collect::<String>());
        }
        print.extend(grid.iter().cloned());
        print.push(site_map::MAP_NO_READINGS.replace("{family}", "metals"));
        let mut assembler = Assembler::default();
        let mut text = print.join("\r\n");
        text.push_str("\r\n");
        let received = assembler.feed(0, &text);
        assert_eq!(
            received[3 + 5].chars().count(),
            40,
            "trailing spaces trimmed"
        );
        assert_eq!(received[12], "", "an impassable row arrives empty");
        let rows = grid_rows(&received);
        assert_eq!(rows.len(), SITE_VIEW_CELLS);
        assert!(rows[3].contains('_'), "a row holding `_`");
        assert_eq!(rows, received[3..3 + SITE_VIEW_CELLS].to_vec());
        assert!(grid_rows(&received[..3 + 20]).is_empty(), "not yet whole");
        assert!(grid_rows(std::slice::from_ref(&cold)).is_empty());
        let one_line: Vec<String> = [site_map::SITE_LEGEND.to_owned()]
            .into_iter()
            .chain(grid.iter().cloned())
            .collect();
        assert_eq!(grid_rows(&one_line).len(), SITE_VIEW_CELLS);

        // The heard line, from the game's own constants and every octant.
        let octants = [
            Octant::N,
            Octant::NE,
            Octant::E,
            Octant::SE,
            Octant::S,
            Octant::SW,
            Octant::W,
            Octant::NW,
        ];
        let mut game: Vec<String> = Vec::new();
        for octant in octants {
            for band in [tv::BAND_NEAR, tv::BAND_MID, tv::BAND_FAR] {
                game.push(
                    tv::HEARD
                        .replace("{bearing}", octant.word())
                        .replace("{band}", band),
                );
            }
        }
        game.push(tv::HEARD_HERE.to_owned());
        let mine = heard_sentences();
        assert_eq!(mine.len(), 25);
        let game_set: std::collections::BTreeSet<&String> = game.iter().collect();
        let mine_set: std::collections::BTreeSet<&String> = mine.iter().collect();
        assert_eq!(game_set, mine_set);
        for sentence in &game {
            assert!(sentence.starts_with(HEARD_HEAD));
            assert_eq!(heard_sentence(sentence), Some(sentence.as_str()));
            assert!(heard_line_is_bare(sentence));
            let prompted = format!("> {sentence}");
            assert_eq!(heard_sentence(&prompted), Some(sentence.as_str()));
            assert!(heard_line_is_bare(&prompted));
            for prefix in ["[tacsim] ", "Ardent Lode: ", "128,124 "] {
                assert!(
                    !heard_line_is_bare(&format!("{prefix}{sentence}")),
                    "{prefix}"
                );
            }
            assert!(!heard_line_is_bare(&format!("{sentence} F12 rich.")));
        }
        assert_eq!(heard_sentence("Rig 1 sounded 64 pockets"), None);
    }

    /// **The captain's field sentences** (doc 32 G-4), each built from tacsim's own constant:
    /// a changed sentence breaks this test, not a field.
    #[test]
    fn field_lines_read_the_games_own_sentences() {
        use tacsim::arena::party::{LETTERS, PartyId};
        let site = "Ahuna Flank";
        let opened = tacsim::field::FIELD_OPENED
            .replace("{n}", "3")
            .replace("{site}", site);
        assert_eq!(field_opened(&normalise(&opened), site), Some(3), "{opened}");
        assert_eq!(field_opened(&normalise(&opened), "Kerwan Floor"), None);
        let settled = tacsim::field::FIELD_SETTLED
            .replace("{n}", "3")
            .replace("{site}", site);
        assert_eq!(
            field_settled(&normalise(&settled), site),
            Some(3),
            "{settled}"
        );
        assert_eq!(field_settled(&normalise(&opened), site), None);
        // Wrapped at 120 columns, the window still reads.
        let wrapped = joined(&wrap(&format!("[tacsim] {opened}"), 40));
        assert_eq!(field_opened(&wrapped, site), Some(3));

        let status = tacsim::field::FIELD_STATUS
            .replace("{n}", "1")
            .replace("{site}", site)
            .replace("{party}", "A")
            .replace("{parties}", "2");
        assert_eq!(own_party(&normalise(&status)), Some('A'), "{status}");
        assert_eq!(NO_FIELD, tacsim::field::NO_FIELD);
        let marked = tacsim::field::PARTY_MARKED
            .replace("{party}", "B")
            .replace("{word}", "Hostile");
        assert!(marked.starts_with(PARTY_MARKED), "{marked}");
        let waits = tacsim::surface::site_record::FIELD_OPEN
            .replace("{ordinal}", "1")
            .replace("{site}", site);
        assert!(has(&normalise(&waits), SURFACE_WAITS), "{waits}");

        // Every party letter's class word reads back to its letter.
        assert_eq!(PARTY_LETTERS.as_bytes(), LETTERS.as_slice());
        for letter in LETTERS {
            let party = PartyId::from_letter(char::from(letter)).expect("a party");
            assert_eq!(
                party_of_word(party.as_str()),
                Some(party.letter()),
                "{letter}"
            );
            let designator = format!("{}K", party.letter());
            assert!(is_field_designator(&designator), "{designator}");
        }
        assert_eq!(party_of_word("unknown"), None);
        // The rows a rig's cab printed in the CF.6 transport gate (G-4's probe).
        assert_eq!(
            field_contact("BD  blue  bearing 090° (rel 000°)  beyond  100 m  0 s  rear"),
            Some(("BD".to_owned(), Some('B')))
        );
        assert_eq!(
            field_contact("AK  a  bearing 083° (rel 353°)  beyond  200 m  0 s  side"),
            Some(("AK".to_owned(), Some('A')))
        );
        assert_eq!(
            field_contact("X2  unknown  bearing 090° (rel 000°)  beyond  100 m  0 s  rear"),
            Some(("X2".to_owned(), None))
        );
        assert_eq!(field_contact("Callsign AG."), None);

        let prefill = tacsim::detachment::sortie::ORDER_PREFILL
            .replace("{term}", "1")
            .replace("{roster}", "8589934593")
            .replace("{revision}", "1");
        assert_eq!(
            order_fence(&normalise(&prefill)).as_deref(),
            Some("ORDER DT:8589934593:1:1"),
            "{prefill}"
        );
        let sent = |sequence: &str, status: &str| {
            tacsim::detachment::sortie::ORDER_SENT
                .replace("{slot}", "1")
                .replace("{sequence}", sequence)
                .replace("{status}", status)
        };
        let window = normalise(&format!(
            "{} {} {}",
            sent("1", "WILCO received"),
            sent("2", "NEGATIVE received"),
            sent("3", "submitted; no reply confirmed")
        ));
        assert_eq!(
            order_replies(&window, 1),
            vec![
                (1, Some("WILCO".to_owned())),
                (2, Some("NEGATIVE".to_owned())),
                (3, None)
            ]
        );
        assert!(order_replies(&window, 2).is_empty());

        let hint = tacsim::port::voyage::LEAVE_HINT.replace("{list}", "1,#345");
        assert!(has(&normalise(&hint), LEAVE_HINT), "{hint}");
        // The bay gate's refusal of a rig left out, which the lift reads for `leave=`.
        let out = tacsim::port::voyage::TV_OUT
            .replace("{n}", "1")
            .replace("{site}", site)
            .replace("{name}", "Fieldcap Lode");
        assert!(has(&normalise(&out), "lifts only with every"), "{out}");
        assert!(out.contains("Rig 1 is out on "), "{out}");

        // Rig 1's run after the field ends on the run's own last line (doc 32 §15); its other
        // tells are the solo pilot's, pinned above.
        use crate::mine::lane::field::RUN_DONE_HEAD;
        let cycle = format!(
            "{}{}",
            tv::CYCLE_DONE
                .replace("{n}", "1")
                .replace("{k}", "4")
                .replace("{cycles}", "4")
                .replace("{t}", "3")
                .replace("{ore}", "metal ore")
                .replace("{hopper}", "12 of 12 t"),
            tv::RUN_DONE.replace("{addr}", "F224")
        );
        assert!(cycle.contains(CYCLE_HEAD), "{cycle}");
        // The escort's report, as a page prints it (mush-ansi's `Page` narrative).
        let report = RAIDER_REPORT
            .replace("{party}", "B")
            .replace("{designator}", "BK");
        assert_eq!(
            raider_report(&format!("Fieldesc{PAGED}{report}")),
            Some(('B', "BK".to_owned()))
        );
        assert_eq!(raider_report(&report), None, "only a page carries it");
        let word = CAPTAIN_REPORT.replace("{party}", "A");
        assert_eq!(captain_report(&format!("Fieldcap{PAGED}{word}")), Some('A'));
        assert_eq!(captain_report(&word), None, "only a page carries it");
        assert_eq!(captain_report("Fieldcap pages: Captain: party AB."), None);
        assert_eq!(
            raider_report("Fieldesc pages: Raider: party B, contact AK."),
            None,
            "the designator is the party's"
        );
        let back = tv::RECOVERED
            .replace("{n}", "1")
            .replace("{name}", "Fieldcap Lode");
        assert!(back.starts_with(RIG_BACK), "{back}");
        assert!(!has(&normalise(&back), BACK_IN_BAY), "{back}");
        assert!(cycle.contains(RUN_DONE_HEAD), "{cycle}");
    }
}
