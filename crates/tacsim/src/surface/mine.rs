//! **Mining's pure rules** (doc 20 cut SM.7; §6 "Mining", §3.4 "the cursor, the invariant, the
//! public pits", §11's Cycle, Yield, Hopper and Caps rows; D-20-5, D-20-9; the SM.7 brief's lead
//! rulings 1–4 and 6, its decisions R1, R8, R17 and R18).
//!
//! Everything the command's preview, the start's offer and prepare and the cycle's finish share,
//! **pure**: no world read, integers only, no RNG (D-20-4). The world half — where the rig stands,
//! the operator's own reading, the licence, the tenure, the locks, the rooms — is
//! `crate::surface::act::mine_plan`, which returns a [`MinePlan`].
//!
//! - **the yields** ([`class_yield`], [`pocket_yields`], [`cycle_yield`]): the drill row's tonnes
//!   a cycle for the pocket's class — lean 1, fair 2, rich 3 on the Mk1 — until the pocket is
//!   drawn, the last cycle the remainder; never rising (D-E7-6);
//! - **the pace** ([`cycle_secs`]): the drill's `cycle_s`, times its `rock_factor` on rock
//!   (§6 "terrain sets the pace, the pocket sets the tonnes");
//! - **the fit** ([`fit`]): the most leading cycles whose running tonnes fit the hopper's and the
//!   hold's unreserved room in every measure, and what bound it — the commitment is bounded by
//!   construction, never by a clamp (§6);
//! - **the cursor** ([`cursor_after`], [`cursor_locked`], [`cursor_released`], [`lock_live`]):
//!   SM.6's `tacsim.pockets` row a cycle draws from and a run locks (§3.4: the lock's one home),
//!   so `initial − pool == Σ drawn` holds on every sweep;
//! - **the blind room** ([`blind_room_t`]): the most any class yields, from the drill row
//!   (§19 item 7; reviewer F9), so fitting or refusing a blind core never tells the class;
//! - **the end of a run** ([`run_end`]): one precedence, `Refused > WorkedOut > Stopped > Dark >
//!   Licence > Claimed > Lease > Done` (reviewer F8; `Dark` the SM.7 lead's pre-commit round,
//!   item 3).
//!
//! **[`MinePlan`] holds the pocket's true class and tonnes** (`class_t`, `pocket_t`): derived
//! truth, so it derives no serde at all (`truth_serialization_policy.rs` pins it). The run record
//! (`crate::surface::excursion::Run`) and the start's server-only fixed parameters carry the same
//! figures because they must serialise (a durable family, and fixed parameters no player reads) —
//! the cursor's precedent; nothing a player reads carries them.

use mush_core::{CanonicalId, ObjId};
use mush_industry::OreDepositId;
use serde::{Deserialize, Serialize};

use crate::definitions::tools::DrillDef;
use crate::port::capacity::Load;
use crate::surface::cursor::{
    CursorRow, MAX_CURSOR_ROWS, MAX_POCKET_TONNES, POCKET_CURSOR_SCHEMA, PocketCursor, PocketLock,
};
use crate::surface::excursion::{
    DarkCause, LicenceEnd, LockRow, MAX_RUN_CYCLES, RigRecord, RunEnd,
};
use crate::surface::geology::{OreFamily, Word};
use crate::surface::reading::Footing;

// ---- the yields and the pace -------------------------------------------------------------------

/// **The drill's tonnes a cycle for a class** (D-20-9; §11's Yield row): the drill row's `lean`,
/// `fair` or `rich`, 0 barren.
#[must_use]
pub fn class_yield(drill: &DrillDef, class: Word) -> u32 {
    match class {
        Word::Barren => 0,
        Word::Lean => drill.yield_t.lean,
        Word::Fair => drill.yield_t.fair,
        Word::Rich => drill.yield_t.rich,
    }
}

/// **IC seconds a cycle on a footing** (§6 "terrain sets the pace"; §11's Cycle row): the drill's
/// `cycle_s` on regolith or ice, times its `rock_factor` on rock; `None` on overflow.
#[must_use]
pub fn cycle_secs(drill: &DrillDef, footing: Footing) -> Option<u32> {
    match footing {
        Footing::Rock => drill.cycle_s.checked_mul(drill.rock_factor),
        Footing::Regolith | Footing::Ice => Some(drill.cycle_s),
    }
}

/// **The yields of every cycle until the pocket is drawn** (§6; lead ruling 4): `[c, c, …, r −
/// c(n − 1)]` for `r = remaining_t` and `c = class_t`; empty when either is 0, and — outside the
/// geology's domain — when `r` is past [`MAX_POCKET_TONNES`] (no pocket holds more, §3.2), so the
/// list is never longer than 24. Never rising (D-E7-6); the sum is `r`.
#[must_use]
pub fn pocket_yields(class_t: u32, remaining_t: u32) -> Vec<u32> {
    if class_t == 0 || remaining_t == 0 || remaining_t > MAX_POCKET_TONNES {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut left = remaining_t;
    while left > 0 {
        let take = left.min(class_t);
        out.push(take);
        left = left.saturating_sub(take);
    }
    out
}

/// **One cycle's yield at landing** (§6; lead ruling 6): `min(class_t, pocket_t − drawn)`, 0 when
/// either is 0 or the pocket is drawn. The finish's one figure: the transfer, the cursor's
/// `drawn_t` and the outcome's tonnes all read it.
#[must_use]
pub fn cycle_yield(class_t: u8, pocket_t: u8, drawn_t: u32) -> u32 {
    if class_t == 0 || pocket_t == 0 {
        return 0;
    }
    u32::from(pocket_t)
        .saturating_sub(drawn_t)
        .min(u32::from(class_t))
}

/// **The blind room** (§6, §19 item 7: "the most any class yields"; the brief's reviewer finding
/// F9): `max(lean, fair, rich)` of the drill row — so a retuned row can never make the room a blind
/// core needs differ from the richest yield, which would let fitting or refusing tell the class. 3
/// on the shipped `drill_core_mk1`.
#[must_use]
pub fn blind_room_t(drill: &DrillDef) -> u32 {
    drill
        .yield_t
        .lean
        .max(drill.yield_t.fair)
        .max(drill.yield_t.rich)
}

// ---- the fit -----------------------------------------------------------------------------------

/// **Why a run stops where it does** (§6: "this pocket out, the hopper full or the hold's room used
/// up, whichever comes first (at most eight cycles)").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    /// The pocket's yields are exhausted.
    WorkedOut,
    /// The next cycle would pass the hopper's room.
    Hopper,
    /// The next cycle would pass the hold's unreserved room.
    Hold,
    /// [`MAX_RUN_CYCLES`] (unreachable under §11's ranges: rich ≤ 24 t at 3 t a cycle is eight).
    Cap,
}

/// Whether `units` of a good with `per_unit` figures fit `room` in every measure the room names (a
/// measure the room does not name is unlimited there): **the port's one per-measure rule**,
/// `crate::port::capacity::fits_in`, over the figures of the measures `room` names (the SM.7 lead's
/// pre-commit round, item 6 — never a second restatement that could drift).
fn fits(units: u64, per_unit: &[(CanonicalId, u64)], room: &Load) -> bool {
    let named: Vec<(CanonicalId, u64)> = per_unit
        .iter()
        .filter(|(measure, _)| room.contains_key(measure))
        .cloned()
        .collect();
    let (most, _) = crate::port::capacity::fits_in(room, &named);
    units <= most
}

/// **The bare form's count** (§6; lead ruling 4, the brief's decision R8): the most leading cycles
/// of `yields` — at most [`MAX_RUN_CYCLES`] — whose running sum of tonnes, weighed by the good's
/// `per_unit` figures, fits **both** rooms in every measure (mass and packed volume), and what bound
/// it: the pocket (the yields exhausted), the hopper, the hold (a cycle passing both is the
/// hopper's), or the cap.
#[must_use]
pub fn fit(
    yields: &[u32],
    per_unit: &[(CanonicalId, u64)],
    hopper_room: &Load,
    hold_room: &Load,
) -> (u8, Bound) {
    let cap = usize::from(MAX_RUN_CYCLES);
    let mut sum: u64 = 0;
    for (n, yield_t) in yields.iter().take(cap).enumerate() {
        let count = u8::try_from(n).unwrap_or(MAX_RUN_CYCLES);
        sum = sum.saturating_add(u64::from(*yield_t));
        if !fits(sum, per_unit, hopper_room) {
            return (count, Bound::Hopper);
        }
        if !fits(sum, per_unit, hold_room) {
            return (count, Bound::Hold);
        }
    }
    if yields.len() > cap {
        (MAX_RUN_CYCLES, Bound::Cap)
    } else {
        (
            u8::try_from(yields.len()).unwrap_or(MAX_RUN_CYCLES),
            Bound::WorkedOut,
        )
    }
}

/// **`first` … `eighth`**: the refusal's ordinal word (§6's "the hopper is full after the third");
/// `last` outside `1..=8`.
#[must_use]
pub fn ordinal_word(n: u8) -> &'static str {
    match n {
        1 => "first",
        2 => "second",
        3 => "third",
        4 => "fourth",
        5 => "fifth",
        6 => "sixth",
        7 => "seventh",
        8 => "eighth",
        _ => "last",
    }
}

// ---- the cursor ------------------------------------------------------------------------------

/// The row index of `pocket` in `rows` (kept in pocket order), or where it goes.
fn row_slot(rows: &[CursorRow], pocket: u16) -> Result<usize, usize> {
    rows.binary_search_by_key(&pocket, |row| row.pocket)
}

/// A cursor past its bounds is refused (`None`); else it, its empty rows dropped, `check`ed.
fn tidy(mut cursor: PocketCursor) -> Option<PocketCursor> {
    cursor
        .rows
        .retain(|row| row.drawn_t > 0 || row.lock.is_some());
    (cursor.rows.len() <= MAX_CURSOR_ROWS && cursor.check().is_ok()).then_some(cursor)
}

/// **The cursor after one cycle** (§3.4; lead rulings 3 and 6): `pocket`'s row drawn by `y` —
/// inserted when absent, rows kept in pocket order — its lock kept while `keep_lock` (set to
/// `ours` when the row had none) and released, **only when it is `ours`**, otherwise; a row neither
/// drawn nor locked is removed. `Some(None)`: the cursor is empty (delete the record, or write
/// nothing when there was none). `None`: refused — a cursor of another deposit, a draw past
/// [`MAX_POCKET_TONNES`], or past [`MAX_CURSOR_ROWS`] rows.
#[must_use]
pub fn cursor_after(
    cursor: Option<&PocketCursor>,
    deposit: OreDepositId,
    pocket: u16,
    y: u32,
    ours: &PocketLock,
    keep_lock: bool,
) -> Option<Option<PocketCursor>> {
    let mut next = cursor.cloned().unwrap_or(PocketCursor {
        schema: POCKET_CURSOR_SCHEMA,
        deposit,
        rows: Vec::new(),
    });
    if next.deposit != deposit {
        return None;
    }
    let at = match row_slot(&next.rows, pocket) {
        Ok(at) => Some(at),
        Err(at) if y > 0 || keep_lock => {
            next.rows.insert(
                at,
                CursorRow {
                    pocket,
                    drawn_t: 0,
                    lock: None,
                },
            );
            Some(at)
        }
        Err(_) => None,
    };
    if let Some(row) = at.and_then(|at| next.rows.get_mut(at)) {
        row.drawn_t = row.drawn_t.checked_add(y)?;
        if row.drawn_t > MAX_POCKET_TONNES {
            return None;
        }
        if keep_lock {
            if row.lock.is_none() {
                row.lock = Some(ours.clone());
            }
        } else if row.lock.as_ref() == Some(ours) {
            row.lock = None;
        }
    }
    let tidied = tidy(next)?;
    Some((!tidied.rows.is_empty()).then_some(tidied))
}

/// **The lock a start writes** (lead ruling 3): `pocket`'s row inserted with nothing drawn and
/// `lock`, or `lock` set on the existing row (a stale one replaced). `None` past
/// [`MAX_CURSOR_ROWS`] or a cursor of another deposit.
#[must_use]
pub fn cursor_locked(
    cursor: Option<&PocketCursor>,
    deposit: OreDepositId,
    pocket: u16,
    lock: PocketLock,
) -> Option<PocketCursor> {
    let mut next = cursor.cloned().unwrap_or(PocketCursor {
        schema: POCKET_CURSOR_SCHEMA,
        deposit,
        rows: Vec::new(),
    });
    if next.deposit != deposit {
        return None;
    }
    match row_slot(&next.rows, pocket) {
        Ok(at) => {
            if let Some(row) = next.rows.get_mut(at) {
                row.lock = Some(lock);
            }
        }
        Err(at) => next.rows.insert(
            at,
            CursorRow {
                pocket,
                drawn_t: 0,
                lock: Some(lock),
            },
        ),
    }
    tidy(next)
}

/// **The release at a close** (lead ruling 3): every row of `pocket` locked by `ours` unlocked,
/// empty rows removed. `None`: the cursor is empty (delete it).
#[must_use]
pub fn cursor_released(
    cursor: &PocketCursor,
    pocket: u16,
    ours: &PocketLock,
) -> Option<PocketCursor> {
    let mut next = cursor.clone();
    for row in next.rows.iter_mut().filter(|row| row.pocket == pocket) {
        if row.lock.as_ref() == Some(ours) {
            row.lock = None;
        }
    }
    next.rows
        .retain(|row| row.drawn_t > 0 || row.lock.is_some());
    (!next.rows.is_empty()).then_some(next)
}

/// **The staleness rule** (lead ruling 3; §3.4; the brief's reviewer finding F13): a lock is LIVE
/// iff its holder's record (read in the same transaction) has a run whose `ordinal` is the lock's,
/// on `site`, **whose `lock_rows` holds this very row** — the deposit (so the block and the
/// family) and the pocket's number (the SM.7 lead's pre-commit round, item 1); anything else — no
/// record, no run, a later run, a run with that ordinal on another pocket, or on another block's
/// pocket of the same number (a staff re-creation of the record through `RigRecord::fresh` resets
/// the ordinal, so a later run can reach a stale lock's ordinal: a stale lock on C17 must not read
/// live while that run drills F17) — is stale and may be taken.
#[must_use]
pub fn lock_live(
    lock: &PocketLock,
    holder: Option<&RigRecord>,
    site: &CanonicalId,
    row: &LockRow,
) -> bool {
    holder.is_some_and(|record| {
        record.tv == lock.tv
            && record.run.as_ref().is_some_and(|run| {
                run.ordinal == lock.ordinal && &run.site == site && run.lock_rows.contains(row)
            })
    })
}

// ---- the rights and the end of a run -----------------------------------------------------------

/// **The tenure of a run's block, read at landing** (doc 20 §6, §8; the SM.7 lead's pre-commit
/// round, item 3): allowed, barred by a claim, or **dark** — the registry could not be read
/// (fail closed, but never told as a claim).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TenureRead {
    /// The tenure allows the block.
    Allowed,
    /// A live claim the operator holds no right under.
    Claimed,
    /// The issuer, a deposit, a claim index or the site's definition does not read.
    Dark {
        /// Which read failed.
        why: DarkCause,
    },
}

/// **The rights of a run's next cycle, read at landing** (doc 20 §8 "rights are per act"; the
/// brief's 3.7): the licence standing (`Ok(lapses_ic)` in force or in its grace, else the
/// standing), the tenure's read for the run's block, and the lease (`Err(true)` forfeit,
/// `Err(false)` ended). The finish's offer fixes it per rig with a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RightsVerdict {
    /// The licence.
    pub licence: Result<i64, LicenceEnd>,
    /// The tenure.
    pub tenure: TenureRead,
    /// The lease.
    pub lease: Result<(), bool>,
}

impl RightsVerdict {
    /// **What stops the next cycle, beginning at `next_ic`** (the brief's 3.7 step 7): first a
    /// dark tenure read (the SM.7 lead's pre-commit round, item 3: nothing about the block can be
    /// told when its registry does not read, so nothing behind it is judged); then the licence
    /// when it lapses at or before `next_ic` (the next cycle begins at this cycle's due instant;
    /// the brief's decision R9), or when it is not a licence at all (unlicensed, suspended,
    /// another character's, unreadable); then the tenure; then the lease. `None`: the next cycle
    /// may run.
    ///
    /// **A lapse read at landing is judged at `next_ic`, never at landing** (R9's restart
    /// equivalence, doc 20 §8; the SM.7 fixer's F1): a standing read `Lapsed { at_ic }` because
    /// the finish landed late (a restart, a back-off, a tick period longer than `at_ic −
    /// next_ic`) decides exactly as the uninterrupted world's `Ok(at_ic)` would — the next cycle
    /// runs when it begins before the lapse. The lapse instant is the grace's end in both arms
    /// (`licence_standing`), so the two compare alike. The converse (a licence renewed between
    /// the due and a late landing reads in force) cannot be told apart from a renewal before the
    /// due without a history, and is the same read-at-landing residual as R9's tenure.
    #[must_use]
    pub fn stops_next(&self, next_ic: i64) -> Option<RunEnd> {
        if let TenureRead::Dark { why } = self.tenure {
            return Some(RunEnd::Dark { why });
        }
        match self.licence {
            Ok(lapses_ic) | Err(LicenceEnd::Lapsed { at_ic: lapses_ic }) => {
                if next_ic >= lapses_ic {
                    return Some(RunEnd::Licence {
                        why: LicenceEnd::Lapsed { at_ic: lapses_ic },
                    });
                }
            }
            Err(why) => return Some(RunEnd::Licence { why }),
        }
        if self.tenure == TenureRead::Claimed {
            return Some(RunEnd::Claimed);
        }
        match self.lease {
            Err(forfeit) => Some(RunEnd::Lease { forfeit }),
            Ok(()) => None,
        }
    }
}

/// **The run's end** (§3.1 of the brief, reviewer F8; decision R17): `Refused` (the cycle did not
/// happen) > `WorkedOut` (the pocket's tonnes are drawn) > `Stopped` (`+tv/mine stop`) > the
/// rights, in [`RightsVerdict::stops_next`]'s order (`Dark` > `Licence` > `Claimed` > `Lease`;
/// `Dark` the SM.7 lead's pre-commit round, item 3) > `Done`
/// (`last`: the run's last cycle) > `None` (re-arm). A right that fails at the last cycle still
/// outranks `Done`: it tells the operator why the next run would refuse.
#[must_use]
pub fn run_end(
    refused: bool,
    worked_out: bool,
    stop: bool,
    last: bool,
    rights: &RightsVerdict,
    next_ic: i64,
) -> Option<RunEnd> {
    if refused {
        return Some(RunEnd::Refused);
    }
    if worked_out {
        return Some(RunEnd::WorkedOut);
    }
    if stop {
        return Some(RunEnd::Stopped);
    }
    if let Some(right) = rights.stops_next(next_ic) {
        return Some(right);
    }
    last.then_some(RunEnd::Done)
}

// ---- the plan --------------------------------------------------------------------------------

/// **What `+tv/mine` was asked** (§6, §10): a family named (else the default), a count named (else
/// the bare form's fit).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MineChoice {
    /// The family named.
    pub family: Option<OreFamily>,
    /// The family's word as typed (a refusal's `{fam}` echoes it).
    pub word: Option<String>,
    /// The count named, `1..=99` as typed.
    pub cycles: Option<u8>,
}

/// **One mining run, planned against the world** (`crate::surface::act::mine_plan`; the SM.7
/// brief's 3.2 and 3.3): what the preview prints, what `go` freezes into its order, and what the
/// start's offer re-plans and requires equal. **Derived truth** (`class_t`, `pocket_t`, `class`
/// of an unread pocket): no serde (`truth_serialization_policy.rs`; reviewer F4) — only
/// `PartialEq` is load-bearing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinePlan {
    /// The site entry.
    pub(crate) site: CanonicalId,
    /// The site's body.
    pub(crate) body: CanonicalId,
    /// The rig's stand cell.
    pub(crate) stand: [i64; 2],
    /// The pocket under it.
    pub(crate) pocket: [i64; 2],
    /// Its number in the block.
    pub(crate) number: u16,
    /// Its block letter.
    pub(crate) letter: char,
    /// Its address (`C117`).
    pub(crate) addr: String,
    /// The family, frozen.
    pub(crate) family: OreFamily,
    /// The family's ore good.
    pub(crate) good: CanonicalId,
    /// The family's deposit id at this block.
    pub(crate) deposit: OreDepositId,
    /// The block vault, when the pocket holds the family's ore.
    pub(crate) source: Option<u64>,
    /// The rig's hopper vault.
    pub(crate) hopper: u64,
    /// The run's cycles.
    pub(crate) cycles: u8,
    /// The yields shown, one a cycle (a blind core: empty).
    pub(crate) yields: Vec<u32>,
    /// The room reserved, one a cycle.
    pub(crate) reserve_t: Vec<u8>,
    /// The drill's yield for the true class (0 barren) — truth.
    pub(crate) class_t: u8,
    /// The pocket's true tonnes of the family — truth.
    pub(crate) pocket_t: u8,
    /// The cursor's drawn tonnes of the pocket in the family.
    pub(crate) drawn: u32,
    /// The class the operator's reading shows (a read pocket only; `Barren` blind — never printed
    /// then).
    pub(crate) class: Word,
    /// The other families the operator's reading shows, with their class (`MINE_ALSO`).
    pub(crate) also: Vec<(OreFamily, Word)>,
    /// No reading of the pocket: one blind cycle.
    pub(crate) blind: bool,
    /// The public pit glyph a blind preview names (`v`, `V`), if any.
    pub(crate) pit: Option<char>,
    /// IC seconds a cycle.
    pub(crate) secs: u32,
    /// Charge a cycle, cu.
    pub(crate) cost_cu: u32,
    /// The charge home from the stand, cu.
    pub(crate) home_cu: u32,
    /// The cursor rows the run locks.
    pub(crate) lock_rows: Vec<LockRow>,
    /// The licence's lapse.
    pub(crate) lapses_ic: Option<i64>,
    /// The hold's unreserved room after the run, per measure.
    pub(crate) hold_after: Load,
    /// The hopper's mass before the run, kg.
    pub(crate) hopper_before_kg: u64,
    /// The hopper's mass after the run, kg.
    pub(crate) hopper_after_kg: u64,
    /// The hopper's mass limit, kg.
    pub(crate) hopper_cap_kg: u64,
    /// The hull's rigs the hold's room counted: each one's run ordinal and outstanding tonnes.
    pub(crate) outstanding: Vec<(ObjId, Option<(u32, u64)>)>,
}

impl MinePlan {
    /// The run's charge, cu: a cycle's times the cycles (checked; `None` past `u32`).
    #[must_use]
    pub(crate) fn run_cost_cu(&self) -> Option<u32> {
        self.cost_cu.checked_mul(u32::from(self.cycles))
    }

    /// The run's IC seconds (checked).
    #[must_use]
    pub(crate) fn run_secs(&self) -> Option<u32> {
        self.secs.checked_mul(u32::from(self.cycles))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::tools::{ToolDef, YieldDef, tools_table};
    use crate::surface::excursion::{Charge, EXCURSION_SCHEMA, Run};

    /// The shipped drill row.
    fn drill() -> DrillDef {
        *tools_table()
            .unwrap_or_else(|e| panic!("{e}"))
            .values()
            .find_map(ToolDef::drill)
            .unwrap_or_else(|| panic!("a drill row"))
    }

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap_or_else(|_| panic!("{text} is canonical"))
    }

    /// **A fair seven-tonne pocket is 2, 2, 2, 1** (A1's arithmetic; §1 beat 8's preview).
    #[test]
    fn sm7_a_fair_seven_tonne_pocket_is_two_two_two_one() {
        let d = drill();
        let c = class_yield(&d, Word::Fair);
        assert_eq!(c, 2);
        assert_eq!(pocket_yields(c, 7), vec![2, 2, 2, 1]);
        assert_eq!(pocket_yields(c, 7).iter().sum::<u32>(), 7);
        // The landings, cycle by cycle, from the cursor.
        let mut drawn = 0;
        let mut got = Vec::new();
        for _ in 0..4 {
            let y = cycle_yield(2, 7, drawn);
            got.push(y);
            drawn += y;
        }
        assert_eq!(got, vec![2, 2, 2, 1]);
        assert_eq!(cycle_yield(2, 7, 7), 0, "worked out");
        assert_eq!(cycle_yield(0, 7, 0), 0, "barren");
        assert_eq!(cycle_yield(2, 0, 0), 0);
    }

    /// **Yields never rise and sum to the remainder** (D-E7-6): every class × remaining 0..=24.
    #[test]
    fn sm7_yields_never_rise_and_sum_to_the_remainder() {
        let d = drill();
        for class in Word::ALL {
            let c = class_yield(&d, class);
            for r in 0..=MAX_POCKET_TONNES {
                let yields = pocket_yields(c, r);
                if c == 0 || r == 0 {
                    assert!(yields.is_empty(), "{class:?} {r}");
                    continue;
                }
                assert_eq!(yields.iter().sum::<u32>(), r, "{class:?} {r}");
                assert!(yields.windows(2).all(|w| w[0] >= w[1]), "{yields:?}");
                assert!(yields.iter().all(|y| *y >= 1 && *y <= c));
                assert_eq!(yields.len(), usize::try_from(r.div_ceil(c)).unwrap_or(0));
            }
        }
        assert!(
            pocket_yields(1, MAX_POCKET_TONNES + 1).is_empty(),
            "outside the domain"
        );
    }

    fn room(mass_kg: u64, volume_l: u64) -> Load {
        [(cid("si.mass_kg"), mass_kg), (cid("si.volume_l"), volume_l)]
            .into_iter()
            .collect()
    }

    fn per(mass: u64, volume: u64) -> Vec<(CanonicalId, u64)> {
        vec![(cid("si.mass_kg"), mass), (cid("si.volume_l"), volume)]
    }

    /// **The fit names its bound** (§6; lead ruling 4): the pocket, the hopper, the hold, a tie
    /// to the hopper, the cap at eight, and carbon ore (1,200 L a tonne) bound by packed volume
    /// before mass.
    #[test]
    fn sm7_the_fit_names_its_bound() {
        let metal = per(1_000, 500);
        let big = room(1_000_000, 1_000_000);
        assert_eq!(
            fit(&[2, 2, 2, 1], &metal, &big, &big),
            (4, Bound::WorkedOut)
        );
        assert_eq!(fit(&[], &metal, &big, &big), (0, Bound::WorkedOut));
        // The hopper holds 6 t more: three fair cycles fit, the fourth does not.
        assert_eq!(
            fit(&[2, 2, 2, 1], &metal, &room(6_000, 13_200), &big),
            (3, Bound::Hopper)
        );
        // The hold leaves 5 t: two cycles.
        assert_eq!(
            fit(&[2, 2, 2, 1], &metal, &big, &room(5_000, 100_000)),
            (2, Bound::Hold)
        );
        // Both bind at once: the hopper's.
        assert_eq!(
            fit(&[2, 2], &metal, &room(3_000, 13_200), &room(3_000, 100_000)),
            (1, Bound::Hopper)
        );
        // Nothing fits.
        assert_eq!(
            fit(&[3, 3], &metal, &room(2_000, 13_200), &big),
            (0, Bound::Hopper)
        );
        // The cap.
        assert_eq!(fit(&[1; 9], &metal, &big, &big), (8, Bound::Cap));
        assert_eq!(fit(&[1; 8], &metal, &big, &big), (8, Bound::WorkedOut));
        // Carbon: 12,000 kg of mass room, 6,000 L of volume — five tonnes by volume, twelve by
        // mass: the volume binds first.
        let carbon = per(1_000, 1_200);
        assert_eq!(
            fit(&[1; 8], &carbon, &room(12_000, 6_000), &big),
            (5, Bound::Hopper)
        );
        // A room that does not name a measure does not limit it.
        let mass_only: Load = [(cid("si.mass_kg"), 4_000)].into_iter().collect();
        assert_eq!(fit(&[2, 2, 2], &carbon, &big, &mass_only), (2, Bound::Hold));
    }

    /// **One per-measure rule** (the SM.7 lead's pre-commit round, item 6): for a two-measure room
    /// the count `fit` names equals `capacity::fits_in`'s — bound by packed volume (carbon ore,
    /// 1,200 L a tonne: 5 by volume, 12 by mass) and by mass (metal ore, 500 L a tonne) — and a
    /// measure the room does not name stays unlimited.
    #[test]
    fn sm7_the_fit_counts_as_the_port_does() {
        let big = room(1_000_000, 1_000_000);
        for (good, hopper, want) in [
            (per(1_000, 1_200), room(12_000, 6_000), 5),
            (per(1_000, 500), room(4_000, 13_200), 4),
            (per(1_000, 500), room(0, 13_200), 0),
        ] {
            let (n, bound) = fit(&[1; 8], &good, &hopper, &big);
            let (most, _) = crate::port::capacity::fits_in(&hopper, &good);
            assert_eq!(u64::from(n), most, "{good:?} in {hopper:?}");
            assert_eq!(most, want);
            assert_eq!(bound, Bound::Hopper);
        }
        let mass_only: Load = [(cid("si.mass_kg"), 3_000)].into_iter().collect();
        let (n, _) = fit(&[1; 8], &per(1_000, 1_200), &mass_only, &big);
        assert_eq!(n, 3, "the volume the room does not name does not bind");
    }

    /// **The pace follows the footing** (§6; lead ruling 4): 1,800 IC s on regolith and ice,
    /// 3,600 on rock, from the shipped drill row.
    #[test]
    fn sm7_the_pace_follows_the_footing() {
        let d = drill();
        assert_eq!(cycle_secs(&d, Footing::Regolith), Some(1_800));
        assert_eq!(cycle_secs(&d, Footing::Ice), Some(1_800));
        assert_eq!(cycle_secs(&d, Footing::Rock), Some(3_600));
        let huge = DrillDef {
            cycle_s: u32::MAX,
            ..d
        };
        assert_eq!(cycle_secs(&huge, Footing::Rock), None, "checked");
        assert_eq!(
            d.energy_cu, 2_000,
            "§11's Charge row: a cycle draws 2,000 cu"
        );
    }

    /// The ordinal words.
    #[test]
    fn sm7_ordinal_words() {
        let words: Vec<&str> = (1..=8).map(ordinal_word).collect();
        assert_eq!(
            words,
            [
                "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth"
            ]
        );
        assert_eq!(ordinal_word(0), "last");
        assert_eq!(ordinal_word(9), "last");
    }

    fn lock(tv: u32, ordinal: u32) -> PocketLock {
        PocketLock {
            tv: ObjId(tv),
            ordinal,
        }
    }

    /// **The cursor never passes 64 rows or 24 t** (lead ruling 3; SM.6's handoff F12): locking
    /// the 64 ore rows of a family succeeds, a 65th refuses; a draw past 24 t refuses; a release
    /// leaves the drawn rows and drops the empty ones; an emptied cursor is deleted.
    #[test]
    fn sm7_the_cursor_never_passes_64_rows_or_24_t() {
        let deposit = OreDepositId(7);
        let mut cursor: Option<PocketCursor> = None;
        for pocket in 1..=64_u16 {
            cursor = Some(
                cursor_locked(cursor.as_ref(), deposit, pocket * 4, lock(9, 1))
                    .unwrap_or_else(|| panic!("row {pocket}")),
            );
        }
        assert_eq!(cursor.as_ref().map(|c| c.rows.len()), Some(64));
        assert_eq!(
            cursor_locked(cursor.as_ref(), deposit, 1, lock(9, 1)),
            None,
            "a 65th row"
        );
        assert_eq!(
            cursor_locked(cursor.as_ref(), OreDepositId(8), 4, lock(9, 1)),
            None,
            "another deposit's cursor"
        );
        // Draws: 24 t fit, a 25th does not.
        let ours = lock(9, 1);
        let mut one = cursor_locked(None, deposit, 5, ours.clone());
        for _ in 0..8 {
            one = cursor_after(one.as_ref(), deposit, 5, 3, &ours, true).flatten();
        }
        assert_eq!(
            one.as_ref().and_then(|c| c.row(5)).map(|r| r.drawn_t),
            Some(24)
        );
        assert_eq!(cursor_after(one.as_ref(), deposit, 5, 1, &ours, true), None);
        // The close releases the lock and keeps the drawn row.
        let closed = cursor_after(one.as_ref(), deposit, 5, 0, &ours, false)
            .flatten()
            .expect("the drawn row stays");
        assert_eq!(
            closed.row(5).map(|r| (r.drawn_t, r.lock.clone())),
            Some((24, None))
        );
        // Another rig's lock is never released.
        let theirs = cursor_locked(None, deposit, 6, lock(10, 3));
        let kept = cursor_after(theirs.as_ref(), deposit, 6, 0, &ours, false)
            .flatten()
            .expect("their lock stays");
        assert_eq!(kept.row(6).and_then(|r| r.lock.clone()), Some(lock(10, 3)));
        // A locked row never drawn, released: the cursor empties.
        let locked = cursor_locked(None, deposit, 7, ours.clone()).expect("locked");
        assert_eq!(cursor_released(&locked, 7, &ours), None, "empty: deleted");
        assert_eq!(
            cursor_after(Some(&locked), deposit, 7, 0, &ours, false),
            Some(None)
        );
        // A barren core of an absent cursor writes nothing.
        assert_eq!(cursor_after(None, deposit, 9, 0, &ours, false), Some(None));
    }

    /// A block's metal deposit id in these tests: C is 7, every other block 8.
    fn deposit_of(letter: char) -> OreDepositId {
        OreDepositId(if letter == 'C' { 7 } else { 8 })
    }

    /// The cursor row of block `letter`'s metal deposit, pocket `number`.
    fn lock_row(letter: char, number: u16) -> LockRow {
        LockRow {
            deposit: deposit_of(letter),
            pocket: number,
        }
    }

    /// A record of rig `tv`, with a run `(ordinal, site, number, letter)` locking that block's row.
    fn record(tv: u32, run: Option<(u32, &str, u16, char)>) -> RigRecord {
        let site = run.map_or_else(|| cid("ahuna_port"), |(_, site, _, _)| cid(site));
        RigRecord {
            schema: EXCURSION_SCHEMA,
            queued: None,
            tv: ObjId(tv),
            ordinal: run.map_or(1, |(ordinal, _, _, _)| ordinal),
            charge: Charge {
                value_cu: 90_000,
                at_ic: 0,
                in_bay: false,
            },
            act: None,
            excursion: None,
            last: None,
            run: run.map(|(ordinal, _, number, letter)| Run {
                ordinal,
                operator: ObjId(42),
                hull: ObjId(500),
                slot: 1,
                site: site.clone(),
                body: cid("ceres"),
                stand: [0, 0],
                pocket: [0, 0],
                number,
                letter,
                family: OreFamily::Metals,
                deposit: deposit_of(letter),
                good: cid("metal_ore"),
                source: None,
                hopper: 1,
                cycles: 1,
                k: 1,
                secs: 1_800,
                cost_cu: 2_000,
                reserve_t: vec![3],
                class_t: 0,
                pocket_t: 0,
                blind: true,
                stop: false,
                lapses_ic: None,
                lock_rows: vec![lock_row(letter, number)],
                due_ic: 1_800,
                reserved_cu: 2_000,
                by: None,
            }),
            plan: None,
            plans: 0,
            recovery: None,
            sortie: None,
            field_crew: None,
        }
    }

    /// **A stale lock is taken and a live one is not** (lead ruling 3; reviewer F13; the SM.7
    /// lead's pre-commit round, item 1), pure over records, for a lock on C117: no record, no run,
    /// a later run, a re-created record whose new run has the lock's ordinal on another pocket, or
    /// **on another block's pocket of the same number** (the run drills F117: the C17/F17 case), a
    /// run of that ordinal on another site, another rig's record — all stale; the run of that
    /// ordinal whose `lock_rows` holds C117 on that site — live.
    #[test]
    fn sm7_a_stale_lock_is_taken_and_a_live_one_is_not() {
        let site = cid("ahuna_port");
        let held = lock(900, 4);
        let c117 = lock_row('C', 117);
        let live = |rec: Option<RigRecord>| lock_live(&held, rec.as_ref(), &site, &c117);
        assert!(!live(None), "no record");
        assert!(!live(Some(record(900, None))), "no run");
        assert!(
            !live(Some(record(900, Some((5, "ahuna_port", 117, 'C'))))),
            "a later run"
        );
        assert!(
            !live(Some(record(900, Some((4, "ahuna_port", 118, 'C'))))),
            "a re-created record's run of the lock's ordinal on another pocket"
        );
        assert!(
            !live(Some(record(900, Some((4, "ahuna_port", 117, 'F'))))),
            "a re-created record's run of the lock's ordinal on F117: the row, not the number"
        );
        assert!(
            !live(Some(record(900, Some((4, "kerwan_port", 117, 'C'))))),
            "another site"
        );
        assert!(
            !live(Some(record(901, Some((4, "ahuna_port", 117, 'C'))))),
            "another rig's record"
        );
        assert!(
            live(Some(record(900, Some((4, "ahuna_port", 117, 'C'))))),
            "live"
        );
        // The same run, asked about its F117 row it never locked: stale.
        assert!(!lock_live(
            &held,
            Some(&record(900, Some((4, "ahuna_port", 117, 'C')))),
            &site,
            &lock_row('F', 117)
        ));
    }

    /// **The blind room is the richest yield** (reviewer F9): the shipped row gives 3 (so doc 20's
    /// "3 t" lines are filled with 3 today); a synthetic `{1, 2, 4}` row gives 4.
    #[test]
    fn sm7_the_blind_room_is_the_richest_yield() {
        assert_eq!(blind_room_t(&drill()), 3);
        let synthetic = DrillDef {
            yield_t: YieldDef {
                lean: 1,
                fair: 2,
                rich: 4,
            },
            ..drill()
        };
        assert_eq!(blind_room_t(&synthetic), 4);
    }

    /// **The run's end, one precedence** (reviewer F8; decision R17; the SM.7 lead's pre-commit
    /// round, item 3): every combination of the refused, worked-out, stop and last flags and of
    /// each right failing, against `Refused > WorkedOut > Stopped > Dark > Licence > Claimed >
    /// Lease > Done > re-arm` — a dark tenure read outranks a licence and a lease that also fail.
    #[test]
    #[allow(clippy::too_many_lines)] // one table of verdicts, then every combination
    fn sm7_the_run_end_precedence() {
        let clear = RightsVerdict {
            licence: Ok(1_000_000),
            tenure: TenureRead::Allowed,
            lease: Ok(()),
        };
        let lapsed = RightsVerdict {
            licence: Ok(5_000),
            ..clear.clone()
        };
        let suspended = RightsVerdict {
            licence: Err(LicenceEnd::Suspended),
            tenure: TenureRead::Claimed,
            lease: Err(true),
        };
        let claimed = RightsVerdict {
            tenure: TenureRead::Claimed,
            lease: Err(false),
            ..clear.clone()
        };
        let ended = RightsVerdict {
            lease: Err(false),
            ..clear.clone()
        };
        let dark = RightsVerdict {
            licence: Err(LicenceEnd::Suspended),
            tenure: TenureRead::Dark {
                why: DarkCause::Deposit,
            },
            lease: Err(true),
        };
        let dark_alone = RightsVerdict {
            tenure: TenureRead::Dark {
                why: DarkCause::Issuer,
            },
            ..clear.clone()
        };
        let rights: [(&RightsVerdict, Option<RunEnd>); 7] = [
            (&clear, None),
            (
                &lapsed,
                Some(RunEnd::Licence {
                    why: LicenceEnd::Lapsed { at_ic: 5_000 },
                }),
            ),
            (
                &suspended,
                Some(RunEnd::Licence {
                    why: LicenceEnd::Suspended,
                }),
            ),
            (&claimed, Some(RunEnd::Claimed)),
            (&ended, Some(RunEnd::Lease { forfeit: false })),
            (
                &dark,
                Some(RunEnd::Dark {
                    why: DarkCause::Deposit,
                }),
            ),
            (
                &dark_alone,
                Some(RunEnd::Dark {
                    why: DarkCause::Issuer,
                }),
            ),
        ];
        for refused in [false, true] {
            for worked_out in [false, true] {
                for stop in [false, true] {
                    for last in [false, true] {
                        for (verdict, right) in &rights {
                            let want = if refused {
                                Some(RunEnd::Refused)
                            } else if worked_out {
                                Some(RunEnd::WorkedOut)
                            } else if stop {
                                Some(RunEnd::Stopped)
                            } else if let Some(right) = right {
                                Some(*right)
                            } else if last {
                                Some(RunEnd::Done)
                            } else {
                                None
                            };
                            assert_eq!(
                                run_end(refused, worked_out, stop, last, verdict, 5_000),
                                want,
                                "{refused} {worked_out} {stop} {last} {verdict:?}"
                            );
                        }
                    }
                }
            }
        }
        // The licence's lapse is judged at the next cycle's start: one second before it, clear.
        assert_eq!(lapsed.stops_next(4_999), None);
        assert_eq!(
            lapsed.stops_next(5_000),
            Some(RunEnd::Licence {
                why: LicenceEnd::Lapsed { at_ic: 5_000 }
            })
        );
        // R9's restart equivalence (the SM.7 fixer's F1): a finish that lands after the lapse
        // reads the standing `Lapsed { 5_000 }`, and decides exactly as the uninterrupted world's
        // `Ok(5_000)` — a next cycle beginning before the lapse still runs.
        let read_late = RightsVerdict {
            licence: Err(LicenceEnd::Lapsed { at_ic: 5_000 }),
            ..clear.clone()
        };
        for next_ic in [0, 4_999, 5_000, 5_001, 9_000] {
            assert_eq!(
                read_late.stops_next(next_ic),
                lapsed.stops_next(next_ic),
                "{next_ic}"
            );
            assert_eq!(
                run_end(false, false, false, false, &read_late, next_ic),
                run_end(false, false, false, false, &lapsed, next_ic),
                "{next_ic}"
            );
        }
        // Every other standing stops the next cycle whenever it begins.
        for why in [
            LicenceEnd::Unlicensed,
            LicenceEnd::Suspended,
            LicenceEnd::Other,
            LicenceEnd::Unreadable,
        ] {
            let verdict = RightsVerdict {
                licence: Err(why),
                ..clear.clone()
            };
            assert_eq!(verdict.stops_next(0), Some(RunEnd::Licence { why }));
        }
    }

    /// **Ten thousand cycles conserve** (A2's arithmetic half — the brief's reviewer finding F22: it
    /// proves `cycle_yield`, `pocket_yields`, `cursor_after` and the caps; the pool here is the
    /// model's own counter, so a transfer/cursor mismatch in the real bundle is `tests/mining.rs`
    /// M2's to catch). A synthetic block of 64 ore pockets of one family, tonnes and classes by a
    /// fixed formula over the pocket number (no RNG), a modelled pool initialised to Σ tonnes; runs
    /// worked pocket by pocket — each run's yields from `pocket_yields`, each cycle's from
    /// `cycle_yield` over the cursor, the cursor through `cursor_after` (locked at the start,
    /// released at the close), the pool down by the same tonnes — a fresh block once all are drawn.
    /// After every cycle `initial − pool == drawn_total`, the cursor checks, holds ≤ 64 rows, and no
    /// row passes its pocket's tonnes; exactly 10,000 cycles are counted.
    #[test]
    fn sm7_ten_thousand_cycles_conserve() {
        let d = drill();
        let deposit = OreDepositId(77);
        let block = |pocket: u16| -> (Word, u8) {
            let n = u32::from(pocket);
            let class = match n % 3 {
                0 => Word::Lean,
                1 => Word::Fair,
                _ => Word::Rich,
            };
            let (lo, hi) = class.tonnes_range();
            let span = u32::from(hi - lo) + 1;
            let tonnes = u32::from(lo) + (n * 7 + 3) % span;
            (class, u8::try_from(tonnes).unwrap_or(hi))
        };
        let pockets: Vec<u16> = (1..=64_u16).map(|i| i * 4 - 1).collect();
        let mut cycles = 0_u32;
        let mut blocks = 0_u32;
        let mut ordinal = 0_u32;
        'outer: loop {
            blocks += 1;
            let initial: u64 = pockets.iter().map(|p| u64::from(block(*p).1)).sum();
            let mut pool = initial;
            let mut cursor: Option<PocketCursor> = None;
            for pocket in &pockets {
                let (class, pocket_t) = block(*pocket);
                let class_t = u8::try_from(class_yield(&d, class)).unwrap_or(0);
                loop {
                    let drawn = cursor
                        .as_ref()
                        .and_then(|c| c.row(*pocket))
                        .map_or(0, |r| r.drawn_t);
                    let yields = pocket_yields(u32::from(class_t), u32::from(pocket_t) - drawn);
                    if yields.is_empty() {
                        break;
                    }
                    let run = usize::from(MAX_RUN_CYCLES).min(yields.len());
                    ordinal += 1;
                    let ours = lock(900, ordinal);
                    cursor = Some(
                        cursor_locked(cursor.as_ref(), deposit, *pocket, ours.clone())
                            .expect("the lock"),
                    );
                    for k in 1..=run {
                        let drawn = cursor
                            .as_ref()
                            .and_then(|c| c.row(*pocket))
                            .map_or(0, |r| r.drawn_t);
                        let y = cycle_yield(class_t, pocket_t, drawn);
                        assert_eq!(Some(&y), yields.get(k - 1), "the preview is the landing");
                        let keep = k < run && drawn + y < u32::from(pocket_t);
                        cursor = cursor_after(cursor.as_ref(), deposit, *pocket, y, &ours, keep)
                            .expect("the cursor stays in bounds");
                        pool -= u64::from(y);
                        cycles += 1;
                        let c = cursor.as_ref().expect("a drawn cursor");
                        assert_eq!(c.check(), Ok(()));
                        assert!(c.rows.len() <= MAX_CURSOR_ROWS);
                        assert_eq!(Some(initial - pool), c.drawn_total());
                        for row in &c.rows {
                            assert!(row.drawn_t <= u32::from(block(row.pocket).1));
                        }
                        if cycles == 10_000 {
                            break 'outer;
                        }
                        if !keep {
                            break;
                        }
                    }
                    if let Some(c) = cursor.as_ref() {
                        cursor = cursor_released(c, *pocket, &ours);
                    }
                }
            }
            assert_eq!(pool, 0, "the block is drawn out");
        }
        assert_eq!(cycles, 10_000);
        assert!(blocks >= 2, "the soak ran past one block ({blocks})");
    }
}
