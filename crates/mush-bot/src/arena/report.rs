//! What the pilot reports (cut AJ.2): one [`MatchResult`] a match and the run's
//! [`Report`], the `arena` block of the bot's JSON report.
//!
//! **Honest by construction.** A match counts as won only on the centre's own
//! result word being the pilot's side (`Match {n} is over (red).`); the shots
//! are the picture's paired tally (`Firing` acks less the refusals paired with
//! them), never a count of `FIRE` lines typed; and the run is `done` only when
//! every match it played was won and nothing blocked it, so a headless run's
//! exit code 0 means exactly that.
//!
//! **The fire gaps** (cut AL.1): [`fire_gaps`] is the programme lead's own
//! count of a transcript's `FIRE` lines, and [`shot_gaps`] the same count over
//! the cockpit's acceptances, which a refused and retried `FIRE` does not
//! split. The offline lane asserts on both; the win-rate lane prints both.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Side, picture::Picture};

/// Two `FIRE` lines (or two shots) of one gun further apart than this are not a
/// gap in its fire: the fight broke off between them (the lead's count).
pub const FIRE_GAP_WINDOW_MS: u64 = 20_000;

/// One gun's gaps: how many, and their 10th, 50th and 90th percentiles, each
/// the sorted gaps' element at `floor(q × (len − 1))`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GapStats {
    pub gaps: usize,
    pub p10_ms: u64,
    pub p50_ms: u64,
    pub p90_ms: u64,
}

/// The gaps of each key under [`FIRE_GAP_WINDOW_MS`], in the order of `lines`,
/// for the lines `key` reads a key from.
fn gaps_by<'a>(
    lines: impl IntoIterator<Item = (u64, &'a str)>,
    key: impl Fn(&'a str) -> Option<&'a str>,
) -> BTreeMap<String, GapStats> {
    let mut last: BTreeMap<&str, u64> = BTreeMap::new();
    let mut gaps: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for (at, line) in lines {
        let Some(key) = key(line) else {
            continue;
        };
        if let Some(before) = last.insert(key, at) {
            let gap = at.saturating_sub(before);
            if gap < FIRE_GAP_WINDOW_MS {
                gaps.entry(key.to_owned()).or_default().push(gap);
            }
        }
    }
    gaps.into_iter()
        .filter_map(|(key, gaps)| percentiles(gaps).map(|stats| (key, stats)))
        .collect()
}

/// The percentiles of `gaps` by index (the lead's rule: the sorted gaps'
/// element at `floor(q × (len − 1))`); `None` for no gap.
fn percentiles(mut gaps: Vec<u64>) -> Option<GapStats> {
    gaps.sort_unstable();
    let last = gaps.len().checked_sub(1)?;
    let at = |tenths: usize| gaps.get(last * tenths / 10).copied().unwrap_or(0);
    Some(GapStats {
        gaps: gaps.len(),
        p10_ms: at(1),
        p50_ms: at(5),
        p90_ms: at(9),
    })
}

/// **The lead's count of the `FIRE` lines** (`sends.js` beside the AL plan):
/// every line starting `FIRE ` is keyed by the word after it as typed, and its
/// gap to that key's previous `FIRE` line counts when it is under
/// [`FIRE_GAP_WINDOW_MS`]. A key with no gap is left out. `sends` are the
/// lane's own `(lane_ms, line)`, in the order they went.
pub fn fire_gaps<'a>(
    sends: impl IntoIterator<Item = (u64, &'a str)>,
) -> BTreeMap<String, GapStats> {
    gaps_by(sends, |line| {
        line.strip_prefix("FIRE ")
            .and_then(|rest| rest.split_whitespace().next())
    })
}

/// **The same count over the shots that left**: the lines that are exactly
/// `Firing {w}.`, keyed by `{w}`. A `FIRE` refused and tried again inside its
/// second is two `FIRE` lines but one shot, so the lead's count alone would
/// read it as a quarter-second gap and flatter the median; this one does not.
pub fn shot_gaps<'a>(
    lines: impl IntoIterator<Item = (u64, &'a str)>,
) -> BTreeMap<String, GapStats> {
    gaps_by(lines, |line| {
        line.strip_prefix("Firing ")
            .and_then(|rest| rest.strip_suffix('.'))
            .filter(|w| !w.is_empty() && !w.contains(char::is_whitespace))
    })
}

/// One match the pilot flew (or tried to).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct MatchResult {
    /// The centre's match number.
    #[serde(rename = "match")]
    pub match_id: u64,
    /// The tank's callsign (`RA`), from `STATUS` or the claimed slot.
    pub callsign: String,
    /// Whether the pilot led its side into this match.
    pub lead: bool,
    /// The result word: `red`, `blue`, `draw`, `timeout`, `abort`, `void`,
    /// `blocked` (a blocker ended the pilot's part) or `left` (the tank was left
    /// for the rest of the match and could not be taken back).
    pub result: String,
    /// `result` is the pilot's own side.
    pub won: bool,
    /// Shots that left: `Firing` acks less the refusals paired with them.
    pub shots: u32,
    /// Orders refused with no ack to pair (the cockpit's own pre-check).
    pub refused_orders: u32,
    pub hits_scored: u32,
    pub misses: u32,
    pub kills: u32,
    pub hits_taken: u32,
    /// The own sections the panel said were destroyed.
    pub sections_lost: Vec<String>,
    /// The own tank was destroyed.
    pub destroyed: bool,
    /// Times the tank was taken back after a drop or a restart.
    pub reclaims: u32,
    /// How long those drops lasted, together.
    pub away_ms: u64,
    /// From `Match {n} is running.` to the pilot's end of it; 0 if it never ran.
    pub duration_ms: u64,
    /// Commander calls sent this match — the requests the cost ledger holds;
    /// an ask the budget refused or whose request could not be built is not
    /// one (cut AJ.3; the lane fills these in).
    #[serde(default)]
    pub commander_calls: u32,
    /// Timely valid answers: doctrines applied.
    #[serde(default)]
    pub commander_applied: u32,
    /// Answers dropped as later than twice the contact period at the ask.
    #[serde(default)]
    pub commander_stale: u32,
    /// Timely answers that were not a doctrine.
    #[serde(default)]
    pub commander_invalid: u32,
    /// Calls that failed, and asks whose request could not be built (each
    /// with its `commander-error` record).
    #[serde(default)]
    pub commander_errors: u32,
    /// Applied `withdraw`s the pilot's rule flew as `hunt` or `orbit` (cut
    /// AK.2; each with its `commander-overruled` record). Always written.
    #[serde(default)]
    pub commander_overruled: u32,
    /// Why the commander stopped before the match's end, when it did.
    #[serde(default)]
    pub commander_off: Option<String>,
}

impl MatchResult {
    /// The record of `match_id` from the picture's tally as it stands.
    #[allow(clippy::too_many_arguments)]
    pub fn from_picture(
        picture: &Picture,
        side: Side,
        match_id: u64,
        callsign: String,
        lead: bool,
        result: &str,
        reclaims: u32,
        away_ms: u64,
        duration_ms: u64,
    ) -> MatchResult {
        let tally = &picture.tally;
        MatchResult {
            match_id,
            callsign,
            lead,
            result: result.to_owned(),
            won: result == side.as_str(),
            shots: tally.fired(),
            refused_orders: tally.refused_orders,
            hits_scored: tally.hits_scored,
            misses: tally.misses,
            kills: tally.kills,
            hits_taken: tally.hits_taken,
            sections_lost: tally.sections_lost.clone(),
            destroyed: picture.own.destroyed,
            reclaims,
            away_ms,
            duration_ms,
            commander_calls: 0,
            commander_applied: 0,
            commander_stale: 0,
            commander_invalid: 0,
            commander_errors: 0,
            commander_overruled: 0,
            commander_off: None,
        }
    }

    /// Fold in the commander's counts of this match.
    pub fn with_commander(mut self, tally: &super::commander::Tally) -> MatchResult {
        self.commander_calls = tally.calls;
        self.commander_applied = tally.applied;
        self.commander_stale = tally.stale;
        self.commander_invalid = tally.invalid;
        self.commander_errors = tally.errors;
        self.commander_overruled = tally.overruled;
        self.commander_off = tally.off.clone();
        self
    }

    /// `won`, `lost`, `drawn` or `ended`, from the pilot's side.
    pub fn verdict(&self, side: Side) -> &'static str {
        if self.result == side.as_str() {
            "won"
        } else if self.result == side.other().as_str() {
            "lost"
        } else if self.result == "draw" {
            "drawn"
        } else {
            "ended"
        }
    }
}

/// The run's `arena` block.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Report {
    pub side: Side,
    pub matches_asked: u32,
    pub matches: Vec<MatchResult>,
    pub wins: u32,
    /// The other side's word.
    pub losses: u32,
    pub draws: u32,
    pub timeouts: u32,
    /// `abort`, `void`, `blocked`, `left` and any word this build does not know.
    pub other: u32,
}

impl Report {
    /// The block for `matches` flown for `side` out of `matches_asked`.
    pub fn new(side: Side, matches_asked: u32, matches: Vec<MatchResult>) -> Report {
        let count = |word: &str| {
            u32::try_from(matches.iter().filter(|m| m.result == word).count()).unwrap_or(u32::MAX)
        };
        let wins = count(side.as_str());
        let losses = count(side.other().as_str());
        let draws = count("draw");
        let timeouts = count("timeout");
        let total = u32::try_from(matches.len()).unwrap_or(u32::MAX);
        Report {
            side,
            matches_asked,
            other: total
                .saturating_sub(wins)
                .saturating_sub(losses)
                .saturating_sub(draws)
                .saturating_sub(timeouts),
            matches,
            wins,
            losses,
            draws,
            timeouts,
        }
    }

    /// `true` when the run is `done`: at least one match, every match's word
    /// the pilot's own side, and no blocker.
    pub fn done(&self, blocked: bool) -> bool {
        !blocked && !self.matches.is_empty() && self.matches.iter().all(|m| m.won)
    }

    /// The run's outcome sentence. A blocker before any match is its own
    /// sentence; otherwise the sentence is the matches'.
    pub fn outcome(&self, blocker: Option<&str>) -> String {
        let side = self.side.as_str();
        match self.matches.as_slice() {
            [] => blocker.map_or_else(|| format!("No match was flown for {side}."), str::to_owned),
            [one] if one.won => format!(
                "Won match {} for {side} as {}: {} shot(s), {} hit(s), {} kill(s); took {} hit(s).",
                one.match_id, one.callsign, one.shots, one.hits_scored, one.kills, one.hits_taken
            ),
            [one] => format!(
                "Match {} ended ({}); the bot flew {} for {side}: {} shot(s), {} hit(s), {} kill(s); \
                 took {} hit(s).",
                one.match_id,
                one.result,
                one.callsign,
                one.shots,
                one.hits_scored,
                one.kills,
                one.hits_taken
            ),
            several => format!(
                "Won {} of {} matches for {side} (lost {}, drew {}, timed out {}, other {}).",
                self.wins,
                several.len(),
                self.losses,
                self.draws,
                self.timeouts,
                self.other
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flown(match_id: u64, result: &str) -> MatchResult {
        MatchResult {
            match_id,
            callsign: "RA".into(),
            lead: true,
            result: result.into(),
            won: result == "red",
            shots: 4,
            refused_orders: 1,
            hits_scored: 2,
            misses: 2,
            kills: 1,
            hits_taken: 3,
            sections_lost: vec![],
            destroyed: false,
            reclaims: 0,
            away_ms: 0,
            duration_ms: 60_000,
            commander_calls: 0,
            commander_applied: 0,
            commander_stale: 0,
            commander_invalid: 0,
            commander_errors: 0,
            commander_overruled: 0,
            commander_off: None,
        }
    }

    #[test]
    fn the_sentences_and_the_counts() {
        let won = Report::new(Side::Red, 1, vec![flown(3, "red")]);
        assert!(won.done(false));
        assert!(!won.done(true));
        assert_eq!(
            won.outcome(None),
            "Won match 3 for red as RA: 4 shot(s), 2 hit(s), 1 kill(s); took 3 hit(s)."
        );
        let lost = Report::new(Side::Red, 1, vec![flown(4, "blue")]);
        assert!(!lost.done(false));
        assert_eq!(
            lost.outcome(None),
            "Match 4 ended (blue); the bot flew RA for red: 4 shot(s), 2 hit(s), 1 kill(s); took 3 \
             hit(s)."
        );
        assert_eq!(lost.losses, 1);
        let several = Report::new(
            Side::Red,
            6,
            vec![
                flown(1, "red"),
                flown(2, "blue"),
                flown(3, "draw"),
                flown(4, "timeout"),
                flown(5, "abort"),
                flown(6, "left"),
            ],
        );
        assert_eq!(
            (
                several.wins,
                several.losses,
                several.draws,
                several.timeouts,
                several.other
            ),
            (1, 1, 1, 1, 2)
        );
        assert_eq!(
            several.outcome(None),
            "Won 1 of 6 matches for red (lost 1, drew 1, timed out 1, other 2)."
        );
        let none = Report::new(Side::Blue, 1, vec![]);
        assert!(!none.done(false), "no match is not a win");
        assert_eq!(
            none.outcome(Some("The blue side is full.")),
            "The blue side is full."
        );
        assert_eq!(flown(1, "draw").verdict(Side::Red), "drawn");
        assert_eq!(flown(1, "abort").verdict(Side::Red), "ended");
    }

    #[test]
    fn commander_overruled_is_written_and_folded_from_the_tally() {
        let value = serde_json::to_value(flown(1, "red")).expect("json");
        assert_eq!(value["commander_overruled"], 0, "always written");
        let tally = crate::arena::commander::Tally {
            calls: 9,
            applied: 7,
            overruled: 4,
            ..Default::default()
        };
        let folded = flown(1, "red").with_commander(&tally);
        assert_eq!(folded.commander_overruled, 4);
        assert_eq!(
            serde_json::to_value(&folded).expect("json")["commander_overruled"],
            4
        );
        // A report written before the field reads as none overruled.
        let mut old = value;
        old.as_object_mut()
            .expect("an object")
            .remove("commander_overruled");
        let read: MatchResult = serde_json::from_value(old).expect("reads");
        assert_eq!(read.commander_overruled, 0);
    }

    /// **The lead's count** (cut AL.1): per word after `FIRE `, a gap to that
    /// word's last line under the window, percentiles by index.
    #[test]
    fn fire_gaps_follow_the_leads_count() {
        let sends = [
            (0, "FIRE cannon"),
            (100, "FIRE machine_gun"),
            (1_200, "FIRE machine_gun"),
            (2_300, "CONTACTS"),
            (2_400, "FIRE machine_gun"),
            (3_000, "FIRE machine_gun"),
            (6_500, "FIRE cannon"),
            (8_000, "FIRE machine_gun"),
            // Over the window from the last: no gap, but the next counts from it.
            (8_000 + FIRE_GAP_WINDOW_MS, "FIRE machine_gun"),
            (9_000 + FIRE_GAP_WINDOW_MS, "FIRE machine_gun"),
            // A lone line of its own key: no gap, left out.
            (30_000, "FIRE mg"),
            // Not a `FIRE` line.
            (30_001, "FIREcannon"),
            (30_002, "FIRE"),
        ];
        let gaps = fire_gaps(sends.iter().map(|(at, line)| (*at, *line)));
        assert_eq!(gaps.keys().collect::<Vec<_>>(), ["cannon", "machine_gun"]);
        assert_eq!(
            gaps["cannon"],
            GapStats {
                gaps: 1,
                p10_ms: 6_500,
                p50_ms: 6_500,
                p90_ms: 6_500
            }
        );
        // Machine-gun gaps 1 100, 1 200, 600, 5 000 and 1 000 (the window's
        // 20 000 left out); sorted 600, 1 000, 1 100, 1 200, 5 000: index
        // floor(0.1 × 4) = 0, floor(0.5 × 4) = 2, floor(0.9 × 4) = 3.
        assert_eq!(
            gaps["machine_gun"],
            GapStats {
                gaps: 5,
                p10_ms: 600,
                p50_ms: 1_100,
                p90_ms: 1_200
            }
        );
    }

    /// **The shots that left** (cut AL.1): the same rule over `Firing {w}.`
    /// lines only, so a refusal between two acks splits nothing.
    #[test]
    fn shot_gaps_count_accepted_shots_only() {
        let lines = [
            (0, "Firing machine_gun."),
            (1_050, "FIRE machine_gun refused. Still cycling: 1 s to go."),
            (1_300, "Firing machine_gun."),
            (2_600, "Firing machine_gun."),
            (3_000, "Firing cannon."),
            (9_250, "Firing cannon."),
            (9_300, "Firing  cannon."),
            (9_400, "Firing cannon"),
        ];
        let gaps = shot_gaps(lines.iter().map(|(at, line)| (*at, *line)));
        assert_eq!(gaps.keys().collect::<Vec<_>>(), ["cannon", "machine_gun"]);
        assert_eq!(
            gaps["machine_gun"],
            GapStats {
                gaps: 2,
                p10_ms: 1_300,
                p50_ms: 1_300,
                p90_ms: 1_300
            },
            "1 300 and 1 300: the refusal splits nothing"
        );
        assert_eq!(gaps["cannon"].gaps, 1);
        assert_eq!(gaps["cannon"].p50_ms, 6_250);
        // The lead's count over the same lines sees no `FIRE` line at all.
        assert!(fire_gaps(lines.iter().map(|(at, line)| (*at, *line))).is_empty());
    }

    #[test]
    fn the_match_key_is_match() {
        let value = serde_json::to_value(flown(7, "red")).expect("json");
        assert_eq!(value["match"], 7);
        assert!(value.get("match_id").is_none());
    }
}
