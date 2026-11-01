//! The mining pilot (doc 20 cut SM.12c-bot): a headless, model-free lane that
//! plays doc 20's surface-mining loop at one site, beat by beat, in line mode.
//!
//! **Independence.** Everything under this module is the bot's own. It reads
//! only the player-visible text of its own SSH line session — the answers to
//! its own lines, the rig's tells and the heard soundings — and it is written
//! from the player help (`help mining`, `help ships`, `help trade`,
//! `help coop`) and the beats of the SM.8 story (the loop as a pilot plays it)
//! alone. No game crate is a normal or build dependency. The `#[cfg(test)]`
//! module of [`lines`] reads `tacsim`'s `pub` sentence constants through the
//! crate's existing development dependency **as a contract check**, in the same
//! role as `tests/arena_contract.rs`: a changed game sentence breaks a test,
//! not a loop. That is the first game-crate import under `src/`, and
//! `AGENTS.md` allows it ("game crates … only in development dependencies");
//! the runtime graph stays clean (`cargo tree -p mush-bot --edges normal,build`
//! lists no RustMUSH package but `mush-bot`).
//!
//! **No I/O and no clock.** Every function that needs time takes `now_ms`, the
//! runner's monotonic session milliseconds, and returns data; the runner does
//! the reading and the sending. The wall-clock epoch the report's unix stamps
//! are made from arrives as data in [`Settings`].
//!
//! **What it plays** ([`lane::Lane`]): kit up at Ceres Freeport (enrol, refit,
//! licence, lease), bunker to the planner's own figures, board and lift, take
//! a witness print of the site map with the rig still in the bay, launch,
//! scan, sound, drive to the best pocket of its own reading, stake that block,
//! mine, **sound a second time** (the one beat the story does not have),
//! recover and unload, fly home and sell the hold at the house's ore bid. It
//! types only its closed vocabulary ([`lane::in_vocabulary`]), at most two
//! lines a second, never a staff `@` verb, and never acts on another bot's
//! text: a heard sounding, another pilot's arrival or a page is recorded and
//! nothing more.
//!
//! **The tempo it is sized to**, as the server ships it at world rate 24
//! (plain facts, not links: the game crates are not dependencies):
//!
//! * an IC hour is 150 real seconds, so a sounding is heard by a party at most
//!   once every 150 s from any one sounder on a site;
//! * the hop out to a Ceres site takes about 3.7 real minutes, the hop home
//!   about 4.8;
//! * a fair drill cycle is 1,800 IC seconds, 75 real seconds, and a run is at
//!   most eight cycles;
//! * a launch is 300 IC seconds (12.5 real), a scan 600 (25), a sounding 1,200
//!   (50).
use serde::{Deserialize, Serialize};

pub mod lane;
pub mod lines;
pub mod report;
pub mod space;

pub use lane::{Beat, Edge, Fed, Lane, LaneEvent, LaneResult, Milestone, Send, in_vocabulary};
pub use report::Report;
pub use space::{SpaceBeat, SpaceEvidence, SpaceLane, in_space_vocabulary};

/// `--mine`'s refusal for a word it does not know.
pub const MINE_SITE_USAGE: &str =
    "--mine takes one of ahuna, commons, juling, kerwan, occator, rheasilvia";

/// The `help coop` "Signing" rule for a ship's name, as the Registry refuses
/// a name that breaks it.
pub const HULL_NAME_RULE: &str = "A ship's name is 1 to 32 letters, digits, spaces, hyphens, \
     apostrophes or full stops, starting with a letter.";

/// A mining site the Registry lands at this phase (`help mining`: "Landing is
/// at the Registry's lettered sites"), named by the route word `+ship/plan`
/// and `+ship/lift` take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Site {
    Ahuna,
    Commons,
    Juling,
    Kerwan,
    Occator,
    Rheasilvia,
}

impl Site {
    /// Every site, in the route words' alphabetical order.
    pub const ALL: [Site; 6] = [
        Site::Ahuna,
        Site::Commons,
        Site::Juling,
        Site::Kerwan,
        Site::Occator,
        Site::Rheasilvia,
    ];

    /// The route word, in any case, trimmed; else [`MINE_SITE_USAGE`].
    pub fn parse(word: &str) -> Result<Site, String> {
        let word = word.trim();
        Site::ALL
            .into_iter()
            .find(|site| site.word().eq_ignore_ascii_case(word))
            .ok_or_else(|| MINE_SITE_USAGE.to_owned())
    }

    /// The route word: `ahuna`, `commons`, `juling`, `kerwan`, `occator`,
    /// `rheasilvia`.
    pub fn word(self) -> &'static str {
        match self {
            Site::Ahuna => "ahuna",
            Site::Commons => "commons",
            Site::Juling => "juling",
            Site::Kerwan => "kerwan",
            Site::Occator => "occator",
            Site::Rheasilvia => "rheasilvia",
        }
    }

    /// The site's name as the game prints it (`help mining`).
    pub fn name(self) -> &'static str {
        match self {
            Site::Ahuna => "Ahuna Flank",
            Site::Commons => "Freeport Commons",
            Site::Juling => "Juling Shadow",
            Site::Kerwan => "Kerwan Floor",
            Site::Occator => "Occator Rim",
            Site::Rheasilvia => "Rheasilvia Commons",
        }
    }
}

/// The two kinds of ore this lane mines and sells (doc 20 D10: one a loop).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    Metals,
    Volatiles,
}

impl Family {
    /// The sounding's label and `+tv/1 mine`'s word: `metals`, `volatiles`.
    pub fn word(self) -> &'static str {
        match self {
            Family::Metals => "metals",
            Family::Volatiles => "volatiles",
        }
    }

    /// The bid row's good, matched case-insensitively: `metal ore`, `ice ore`.
    pub fn ore_label(self) -> &'static str {
        match self {
            Family::Metals => "metal ore",
            Family::Volatiles => "ice ore",
        }
    }

    /// The other family (the one switch, doc 20 cut SM.12c-bot D20).
    pub fn other(self) -> Family {
        match self {
            Family::Metals => Family::Volatiles,
            Family::Volatiles => Family::Metals,
        }
    }
}

/// What the runner hands the lane at its start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The Dray's name `+coop/enrol` gives ([`hull_name`]).
    pub hull: String,
    /// Unix milliseconds at the runner's `lane_ms` 0, so a report stamp `t` is
    /// `epoch_unix_ms + t`.
    pub epoch_unix_ms: u64,
}

/// **The hull's name** from the SSH username: the character part of
/// `account+Character`, else the whole username; its ASCII letters, digits
/// and spaces, trimmed; then " Lode". `help coop` "Signing": 1 to 32 letters,
/// digits, spaces, hyphens, apostrophes or full stops, starting with a letter
/// — `Err` with that sentence ([`HULL_NAME_RULE`]) when the result breaks it.
pub fn hull_name(username: &str) -> Result<String, String> {
    let part = match username.split_once('+') {
        Some((_, character)) => character,
        None => username,
    };
    let kept: String = part
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ')
        .collect();
    let name = format!("{} Lode", kept.trim());
    let fits = (1..=32).contains(&name.chars().count())
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '\'' | '.'));
    if fits {
        Ok(name)
    } else {
        Err(HULL_NAME_RULE.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--mine`'s word and the hull's name (doc 20 cut SM.12c-bot, B6 test 4).
    #[test]
    fn mine_lane_refuses_an_unknown_site() {
        assert_eq!(Site::parse("vesta"), Err(MINE_SITE_USAGE.to_owned()));
        assert_eq!(Site::parse(""), Err(MINE_SITE_USAGE.to_owned()));
        for site in Site::ALL {
            assert_eq!(Site::parse(site.word()), Ok(site));
            assert_eq!(Site::parse(&site.word().to_uppercase()), Ok(site));
            assert_eq!(Site::parse(&format!("  {}  ", site.word())), Ok(site));
            assert!(!site.name().is_empty());
        }
        assert_eq!(hull_name("minera+Ardent"), Ok("Ardent Lode".to_owned()));
        assert_eq!(hull_name("ada"), Ok("ada Lode".to_owned()));
        // 27 characters make a 32-character name; 28 pass 32.
        let fits = format!("x+{}", "a".repeat(27));
        assert_eq!(hull_name(&fits).map(|n| n.len()), Ok(32));
        let long = format!("x+{}", "a".repeat(28));
        assert_eq!(hull_name(&long), Err(HULL_NAME_RULE.to_owned()));
        assert_eq!(hull_name("x+9lives"), Err(HULL_NAME_RULE.to_owned()));
        assert_eq!(hull_name("x+"), Err(HULL_NAME_RULE.to_owned()));
        assert_eq!(serde_json::to_string(&Site::Kerwan).unwrap(), "\"kerwan\"");
        assert_eq!(Family::Metals.other(), Family::Volatiles);
    }
}
