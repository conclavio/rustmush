//! What the pilot reports (cut AJ.2): one [`MatchResult`] a match and the run's
//! [`Report`], the `arena` block of the bot's JSON report.
//!
//! **Honest by construction.** A match counts as won only on the centre's own
//! result word being the pilot's side (`Match {n} is over (red).`); the shots
//! are the picture's paired tally (`Firing` acks less the refusals paired with
//! them), never a count of `FIRE` lines typed; and the run is `done` only when
//! every match it played was won and nothing blocked it, so a headless run's
//! exit code 0 means exactly that.
use serde::{Deserialize, Serialize};

use super::{Side, picture::Picture};

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
        }
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
    fn the_match_key_is_match() {
        let value = serde_json::to_value(flown(7, "red")).expect("json");
        assert_eq!(value["match"], 7);
        assert!(value.get("match_id").is_none());
    }
}
