//! The mining pilot's report (doc 20 cut SM.12c-bot): the run's `mine` block,
//! every beat and milestone stamped twice — the lane's session milliseconds
//! and the unix milliseconds the runner's epoch makes of them — so a cohort can
//! lay several bots' reports side by side.
use serde::{Deserialize, Serialize};

use super::lane::{Beat, Milestone};
use super::lines::{PlanFigures, SaleRead};

/// The run's `mine` block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// The site's name (`Ahuna Flank`).
    pub site: String,
    /// The hull's name the lane signed for.
    pub hull: String,
    /// The character the lane plays.
    pub me: String,
    /// Unix milliseconds at lane millisecond 0.
    pub epoch_unix_ms: u64,
    /// Every beat begun, in order.
    pub beats: Vec<BeatSpan>,
    /// Every milestone reached, in order.
    pub milestones: Vec<MilestoneAt>,
    /// The soundings: ordinal 1 (the pad's) and 2 (the added beat's).
    pub soundings: Vec<SoundingSpan>,
    /// Every heard line received.
    pub heard: Vec<HeardAt>,
    /// The witness print's grid rows, as printed (the rig still in the bay).
    pub witness: Vec<String>,
    /// The family mined, as it stands at the end (after any switch).
    pub family: Option<String>,
    /// The one switch (D20): no unclaimed pocket of the first family was left.
    pub family_switched: bool,
    /// Stakes the Registry refused (a block under claim, lapsing or cooling).
    pub stakes_refused: u32,
    /// The lane mined unstaked after its third refused stake.
    pub mined_open_ground: bool,
    /// Every `+market/list` row the lane read, raw, in reading order, exact
    /// duplicates dropped (re-read with [`super::lines::listings`]).
    pub listings: Vec<String>,
    /// The last bunker round's `+ship/plan {site}` figures (both `home_*`
    /// `Some`).
    pub plan: Option<PlanFigures>,
    /// The loop's receipts.
    pub receipts: Receipts,
    /// The hopper's tonnes after the last cycle.
    pub mined_t: u64,
    /// The tonnes the sale's receipt names.
    pub sold_t: u64,
    /// Lines the lane typed.
    pub lines_sent: u64,
    /// Bytes of decoded text the lane read.
    pub bytes_received: u64,
    /// Exactly when the sale's receipt arrived.
    pub success: bool,
    /// The lane's failure, when it blocked.
    pub failure: Option<String>,
}

/// One beat: when its first line went and its last answer was read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeatSpan {
    pub beat: Beat,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub start_unix_ms: u64,
    pub end_unix_ms: Option<u64>,
}

/// One milestone reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneAt {
    pub name: Milestone,
    pub lane_ms: u64,
    pub at_unix_ms: u64,
}

/// One sounding: sent, and done (its `Rig 1 sounded` line read) or skipped
/// with the answer that skipped it (the second only).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundingSpan {
    pub ordinal: u8,
    pub sent_ms: u64,
    pub sent_unix_ms: u64,
    pub done_ms: Option<u64>,
    pub done_unix_ms: Option<u64>,
    pub skipped: Option<String>,
}

/// One heard line: when, its sentence and the line as it arrived.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeardAt {
    pub lane_ms: u64,
    pub at_unix_ms: u64,
    pub sentence: String,
    pub line: String,
}

/// The loop's receipts: the stake's record, the unload and the sale.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipts {
    /// The site clerk's `records your claim on block …` line.
    pub stake: Option<String>,
    /// The rig's `Rig 1 unloads … into the …'s hold` line.
    pub unloaded: Option<String>,
    /// The sale's line, read.
    pub sale: Option<SaleRead>,
}

impl Report {
    /// The first time `name` was reached.
    pub fn milestone(&self, name: Milestone) -> Option<&MilestoneAt> {
        self.milestones.iter().find(|at| at.name == name)
    }

    /// The sounding with this ordinal (1 or 2).
    pub fn sounding(&self, ordinal: u8) -> Option<&SoundingSpan> {
        self.soundings.iter().find(|span| span.ordinal == ordinal)
    }

    /// The beat's span.
    pub fn beat(&self, beat: Beat) -> Option<&BeatSpan> {
        self.beats.iter().find(|span| span.beat == beat)
    }
}
