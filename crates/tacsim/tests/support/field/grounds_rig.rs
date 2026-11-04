//! **The two grounds every field story stands on** (doc 40 cut FR.0, §4.7, Q-40-23): Ahuna
//! Flank at 256 a side and Urvara Reach at 512, hoisted here on 2026-11-03 from `field_story`
//! (crates/tacsim/tests/shard_9/field_story.rs) so that each later cut's story is one script over
//! both from any shard binary (Q-38-29: every new cut's story at 256 and 512). Every item is
//! `pub(crate)`; each shard root declares this module by path beside `field_raiders_rig`.
#![allow(dead_code)]

use crate::driving_rig::AHUNA;
use crate::field_raiders_rig::SKULKS;

/// A site cell and a heading, millidegrees.
pub(crate) type Placed = ((i64, i64), u32);

/// **The ground one run of the story stands on** (doc 32 cut S.2, Q-CF-51: the story runs once
/// at 256 a side and again at 512, one script over both): the site, its raider, and where the
/// story puts everyone. Every other number in the story is the site's own (its pad, its ramp, its
/// record) or the field's.
pub(crate) struct Ground {
    /// The site's port entry.
    pub(crate) entry: &'static str,
    /// Its `+ship/undock` word.
    pub(crate) venue: &'static str,
    /// The content's raider seeded on it.
    pub(crate) raider: &'static str,
    /// The site's side, cells.
    pub(crate) side: i64,
    /// The bystander's rig, in the next block.
    pub(crate) bystander: (i64, i64),
    /// Where the miner's rig scans and sounds.
    pub(crate) read: (i64, i64),
    /// The unread pocket it cores.
    pub(crate) core: (i64, i64),
    /// The player escort's hull, facing away from the raider's tank (G-6's engine hit).
    pub(crate) escort: Placed,
    /// The hired NPC's hull.
    pub(crate) ward: Placed,
    /// The raider's walker, in the escort's line.
    pub(crate) walker: Placed,
    /// Where a staff hand stands the raider's tank before it declares, or `None` to leave it
    /// where it launched.
    pub(crate) tank: Option<Placed>,
    /// The bearing the raider's leader declares on (the bystander's, from the leader's units).
    pub(crate) declare: &'static str,
    /// The tank's stand in the second field, past the wreck on the escort's row.
    pub(crate) second: Placed,
    /// The bearing it declares on the escort's hull by.
    pub(crate) second_mark: &'static str,
}

/// **Ahuna Flank at 256 a side** (doc 32 §17): everything within 32 cells of the pad, the
/// raider's carrier to the east at the pad's cell plus `(28, -2)`.
pub(crate) const AHUNA_FLANK: Ground = Ground {
    entry: AHUNA,
    venue: "ahuna",
    raider: SKULKS,
    side: 256,
    bystander: (140, 136),
    read: (140, 120),
    core: (136, 120),
    escort: ((150, 126), 270_000),
    ward: ((146, 120), 90_000),
    walker: ((153, 126), 270_000),
    tank: None,
    declare: "NW",
    second: ((160, 126), 270_000),
    second_mark: "W",
};

/// **Urvara Reach at 512 a side** (doc 32 §20): the pad at `(256, 256)`, the middle of the site.
/// The miner works its own block south-west of the pad, below cell 256 on both axes; the
/// bystander's block and the raider's carrier (the pad's cell plus `(40, 20)`, its ramp at
/// `(299, 276)`) are in the far quadrant, past 256 on both. The fight is on row 268 across the
/// 256 line: the tank stands west of it at `(250, 268)` and fires east through the escort's rear
/// plate, the walker and the escort east of it; the tank's withdrawal to its ramp crosses back.
pub(crate) const URVARA_REACH: Ground = Ground {
    entry: "urvara_port",
    venue: "urvara",
    raider: "tacsim.raider.urvara_reavers",
    side: 512,
    bystander: (282, 294),
    read: (242, 246),
    core: (238, 246),
    escort: ((262, 268), 90_000),
    ward: ((258, 262), 270_000),
    walker: ((258, 268), 90_000),
    tank: Some(((250, 268), 90_000)),
    declare: "NW",
    second: ((250, 268), 90_000),
    second_mark: "E",
};

impl Ground {
    /// The `(venue, entry)` the pad helpers take.
    pub(crate) fn site(&self) -> (&'static str, &'static str) {
        (self.venue, self.entry)
    }
}
