//! **The fill plan: leave it running** (doc 20 cut SM.11; §1 beat 11, §8 "Leaving it running",
//! §5's `via: Plan`, §11's Tick, Caps and Income rows; D-20-6, D-20-9, D-20-11; Q-20-3 accepted:
//! one landed hold a plan, 288 IC hours, never re-armed; §19 items 1 and 3; the SM.11 brief's lead
//! rulings 1–10 and decisions D1–D23).
//!
//! **A plan is a durable record stepped by the surface tick through the existing acts; it decides
//! nothing itself; every act still starts and finishes as SM.3b built it** (lead ruling 1). The
//! record, [`FillPlan`], rides the rig's own `tacsim.excursion` record
//! (`crate::surface::excursion::RigRecord::plan`, the brief's D1): the compiled steps, the frozen
//! family, the author, the filing instant and the 288-IC-hour deadline, the cursor and the pockets
//! of a block step passed over, its state (live, parking, ended), its tally and its notes.
//!
//! - **The compiler** (`compile`, pure; lead ruling 2) turns a typed list of pocket addresses and
//!   block letters into the closed vocabulary — a loud sounding of a field ([`PlanStep::Sound`]),
//!   a bare mining run of a pocket ([`PlanStep::Mine`]), every pocket of a block the author reads
//!   fair or better ([`PlanStep::Block`]) — over the author's OWN knowledge (their chart row) and
//!   rights (the licence, the lease, the block's tenure), the reserve rule leg by leg from a full
//!   battery, one landed hold and the 288-hour bound. It reads nothing: the caller (the command
//!   and the start's offer, one function) gathers every figure.
//! - **The interpreter** (`decide`, `pub(crate)`) chooses, over the durable record and the world,
//!   the next move — start one of the existing acts (a drive, a sounding, a mining run, a recover),
//!   end the plan, or wait — the same function for the tick (to choose its submission) and the
//!   start's offer (to fix it), so a restart re-derives the same answer from the record (D16).
//!   Every gate that needs no drive leg runs before the one A*; a waiting plan never plans a leg.
//! - **The refusal table** (`fate_of`, `pub(crate)`) classifies every `Refusal` an act's world
//!   plan can answer — exhaustively, no wildcard (the brief's D18).
//!
//! No RNG; integers throughout. **Truth stays server-side**: [`PlanMove`] and [`PlanAct`] hold a
//! `MinePlan` (the pocket's true class and tonnes) and derive no serde
//! (`truth_serialization_policy.rs` asserts it); [`FillPlan`] holds addresses, words and a tally
//! of tonnes the operator was told, never a class or a reserve.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, ObjId, World};
use serde::{Deserialize, Serialize};

use crate::definitions::logistics::MAX_BAY_ITEMS;
use crate::definitions::tools::{BatteryDef, DrillDef, ScannerDef, ToolDef};
use crate::port::site_map::SurfaceReads;
use crate::surface::act::{
    ActPlan, LeaseBar, Refusal, RigAt, TenureBar, Where, lease_gate, licence_verdict, where_is,
};
use crate::surface::address;
use crate::surface::chart::{self, ChartRecord, SitePeek};
use crate::surface::drive::{self, Target};
use crate::surface::excursion::{
    ExcursionCounts, LicenceEnd, MAX_RUN_CYCLES, RECOVER_S, RigRecord, UNLOAD_S, cap_refusal,
    cells_apart, charge_at, charge_for_act, reserve_check, reserved_total, site_cell,
};
use crate::surface::geology::{FieldGround, OreFamily, Word, by_family};
use crate::surface::mine::{MineChoice, MinePlan, blind_room_t, class_yield, cycle_secs};
use crate::surface::reading::{FieldReading, Footing};
use crate::surface::{HomePeek, Peeks};

// ---- the numbers ------------------------------------------------------------------------------

/// **A plan's bound, IC seconds** (doc 20 §8, Q-20-3 accepted: "288 IC hours, twelve real hours at
/// rate 24"): the deadline is the filing instant plus this; no step starts after it (a step in
/// flight finishes by its own rules, the brief's D7).
pub const PLAN_BOUND_IC_SECS: i64 = 288 * 3_600;

/// **The idle return, IC seconds** (doc 20 §8 "With no order for 24 IC hours a TV out returns and
/// parks"; D-20-11 "idle return after 24 IC h"): a rig on the ground with nothing in flight and
/// no live plan for this long is recovered by the tick as its operator. A code constant, never
/// the captain's queue's bound.
pub const IDLE_PARK_IC_SECS: i64 = 24 * 3_600;

/// **The most addresses a plan names** (lead ruling 2): a Dray-P hold is 86 t (§11's Dray-P row)
/// and a fair pocket holds at least 6 t (§11's Geology row), so 15 named pockets fill a hold; 16
/// covers it.
pub const MAX_PLAN_ADDRESSES: usize = 16;

/// **The most steps a plan compiles to** (lead ruling 2): one hold is at most 29 lean pockets of
/// 3 t (86 / 3, §11's Geology row "lean 3–6 t"), each preceded by at most one sounding of its field
/// — 58 — rounded to 64; also §11's "≤ 64 ore pockets a block a family", so one block step can
/// never name more pockets than the cap allows.
pub const MAX_PLAN_STEPS: usize = 64;

/// **The plan's note ring** (§11's Caps row "≤ 1 notice a trip"): a plan tells at most a parking,
/// an end and its pass-overs; the newest eight are kept.
pub const MAX_PLAN_NOTES: usize = 8;

/// **The re-decision spacing of a wait with no computable end**, IC seconds (the brief's 3.1 and
/// 3.8, D16; the reviewer's F10): the tick caches a helm, ashore, cap, cold or dark wait in RAM
/// for this long and does not re-run the interpreter for the rig until it passes — `LAUNCH_S`'s
/// 300 IC s, 12.5 real seconds at rate 24.
pub const PLAN_WAIT_IC_SECS: i64 = 300;

/// **`await tv`'s bound and its default, IC hours** (doc 20 §8 "`until=<≤288 IC h>`"; lead ruling
/// 5).
pub const MAX_AWAIT_IC_HOURS: u16 = 288;

/// The most pocket numbers a block step passes over: a block's 256 pockets.
pub const MAX_PLAN_PASSED: usize = 256;

/// The most bytes one listed address keeps (`C117`, `D`).
pub const MAX_PLAN_ADDRESS_BYTES: usize = 8;

// ---- the record -------------------------------------------------------------------------------

/// **Who set an act going** (lead rulings 1 and 4; doc 20 §5, §8): server-authored, carried from
/// the start's `by` parameter onto the act, the run and the outcome, so the finish writes the
/// chart with `via: Plan` and the tell knows a plan's or the idle return's act. Absent is the hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActBy {
    /// A fill plan's step.
    Plan {
        /// The rig's plan number.
        no: u32,
        /// The step it served (its index in the plan's steps).
        step: u16,
    },
    /// The idle return (a recover only).
    Idle,
}

/// **One compiled step** (the closed vocabulary's targets; the moves between them — the drive,
/// the recover and SM.8's unload, the park — are the interpreter's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanStep {
    /// A loud sounding from the middle of field `field` (`address::field_index`).
    Sound {
        /// The field's index on the site.
        field: u8,
    },
    /// Mine this pocket (site pocket `[px, py]`), bare: out, full or the hold's room.
    Mine {
        /// The site pocket.
        pocket: [i64; 2],
    },
    /// Mine every pocket of block `letter` the author reads fair or better in the family, nearest
    /// first (§8's block rule), each chosen at its own step's start.
    Block {
        /// The block's letter, `A..=P`.
        letter: char,
    },
}

/// **Why a plan parks or ended** (§8; lead ruling 3). Stores the standing, never prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanEnd {
    /// The list is worked out.
    WorkedOut,
    /// No cycle of the next pocket fits the hold's unreserved room (one landed hold).
    HoldFull,
    /// Its 288 IC hours ran out.
    Bound,
    /// The author's licence stops it.
    Licence {
        /// The standing.
        why: LicenceEnd,
    },
    /// A live claim the author holds no right under.
    Claimed,
    /// The rig's lease: forfeit (`true`) or ended.
    Lease {
        /// The deposit is forfeit.
        forfeit: bool,
    },
    /// The registry cannot be read for the block (fail closed).
    Dark,
    /// The author's chart of the body is full (`chart_full`): no sounding or core can be written
    /// until they `+tv/forget` (the brief's 3.4 table; the reviewer's F2).
    ChartFull,
    /// The author's `+tv/plan clear`.
    Cleared,
    /// The rig cannot reach the ramp on its charge: the plan ends where it stands.
    Stranded,
}

impl PlanEnd {
    /// **A right** (lead ruling 3: a licence, a claim, a lease, the dark registry — and the full
    /// chart): its parking is noted and told.
    #[must_use]
    pub fn is_right(self) -> bool {
        match self {
            PlanEnd::Licence { .. }
            | PlanEnd::Claimed
            | PlanEnd::Lease { .. }
            | PlanEnd::Dark
            | PlanEnd::ChartFull => true,
            PlanEnd::WorkedOut
            | PlanEnd::HoldFull
            | PlanEnd::Bound
            | PlanEnd::Cleared
            | PlanEnd::Stranded => false,
        }
    }

    /// **The short word the projection shows** (lead ruling 10): no world read, no figure.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            PlanEnd::WorkedOut => "worked out",
            PlanEnd::HoldFull => "hold full",
            PlanEnd::Bound => "out of time",
            PlanEnd::Licence { .. } => "licence",
            PlanEnd::Claimed => "under claim",
            PlanEnd::Lease { .. } => "lease",
            PlanEnd::Dark => "registry dark",
            PlanEnd::ChartFull => "chart full",
            PlanEnd::Cleared => "cleared",
            PlanEnd::Stranded => "stranded",
        }
    }
}

/// **A plan's state** (lead rulings 1 and 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum FillState {
    /// Working its list.
    Live,
    /// Its remaining steps skipped: it comes home, unloads and ends.
    Parking {
        /// Why.
        why: PlanEnd,
    },
    /// Ended; the outcome stays on the record until the next filing or `+tv/plan clear`.
    Ended {
        /// Why.
        why: PlanEnd,
        /// The IC instant it ended.
        at_ic: i64,
    },
}

/// **Why the interpreter passed over a pocket or a field** (a note, never a refusal to the
/// player).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassWhy {
    /// Its tonnes of the family are drawn.
    WorkedOut,
    /// The author reads it barren in the family.
    Barren,
    /// Another rig holds its cursor rows.
    Locked,
    /// The stand cannot be reached or worked.
    NoWay,
    /// A full charge cannot work it and come home.
    TooFar,
    /// A sounding there would add nothing.
    NothingNew,
}

/// **One note of a plan** — what the tick tells (lead ruling 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanNoteKind {
    /// A pocket (or, `field: true`, a field) passed over.
    Passed {
        /// The site pocket, or the site field.
        at: [i64; 2],
        /// A field rather than a pocket.
        field: bool,
        /// Why.
        why: PassWhy,
    },
    /// The plan began to park, for a right (or the full chart).
    Parking {
        /// Why.
        why: PlanEnd,
    },
    /// The plan ended.
    Ended {
        /// Why.
        why: PlanEnd,
    },
}

/// **One numbered note** (its `seq` is the plan's at the write that made it; the tick tells every
/// note above the projection's told `seq`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanNote {
    /// The plan's `seq` it was written under.
    pub seq: u32,
    /// The IC instant.
    pub at_ic: i64,
    /// What.
    pub note: PlanNoteKind,
}

/// **What a plan did** (the end's summary and `+tv/plan show`; the reviewer's F9): `trips` are the
/// plan's recovers that brought ore home (a recover that queued SM.8's unload — an empty recover
/// is not a trip).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTally {
    /// Tonnes the plan's cycles moved into the hopper.
    pub tonnes: u32,
    /// The plan's cycles that landed.
    pub cycles: u16,
    /// The plan's recovers that queued an unload.
    pub trips: u16,
}

/// **A fill plan** (§8 "Leaving it running"; lead ruling 1) on its rig's record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillPlan {
    /// The rig's plan number (`RigRecord::plans` at filing).
    pub no: u32,
    /// The operator who filed it: the tick acts only as them.
    pub author: ObjId,
    /// The hull the rig works from.
    pub hull: ObjId,
    /// The rig's bay slot.
    pub slot: u8,
    /// The site port entry it was filed on (its pad).
    pub site: CanonicalId,
    /// The family, frozen (§8).
    pub family: OreFamily,
    /// The IC instant it was filed.
    pub filed_ic: i64,
    /// `filed_ic + PLAN_BOUND_IC_SECS`.
    pub deadline_ic: i64,
    /// The addresses as filed, upper-cased (`C117`, `D`), `1..=`[`MAX_PLAN_ADDRESSES`].
    pub list: Vec<String>,
    /// The compiled steps, `1..=`[`MAX_PLAN_STEPS`].
    pub steps: Vec<PlanStep>,
    /// The step being worked; `steps.len()` once the list is done.
    pub cursor: u16,
    /// The pocket numbers of the current block step passed over (at most 256).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passed: Vec<u16>,
    /// Live, parking or ended.
    pub state: FillState,
    /// Bumped by every prepare that changes the plan (its tell's key).
    pub seq: u32,
    /// What it did.
    pub tally: PlanTally,
    /// Newest last, at most [`MAX_PLAN_NOTES`] (the oldest dropped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<PlanNote>,
}

impl FillPlan {
    /// **Live** — working or parking.
    #[must_use]
    pub fn live(&self) -> bool {
        matches!(self.state, FillState::Live | FillState::Parking { .. })
    }

    /// **Live for `lessee`** — live, and its author still leases the rig. A plan whose author no
    /// longer does is orphaned: inert, refusing nothing, replaced by the next filing (the brief's
    /// D12).
    #[must_use]
    pub fn live_for(&self, lessee: ObjId) -> bool {
        self.live() && self.author == lessee
    }

    /// **The plan's own bounds** (`RigRecord::check` calls it).
    ///
    /// # Errors
    ///
    /// The first bound broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.slot == 0 || u32::from(self.slot) > MAX_BAY_ITEMS {
            return Err(format!("a plan's slot is 1 to {MAX_BAY_ITEMS}"));
        }
        if self.filed_ic.checked_add(PLAN_BOUND_IC_SECS) != Some(self.deadline_ic) {
            return Err("a plan works at most 288 IC hours from its filing".to_owned());
        }
        if self.list.is_empty()
            || self.list.len() > MAX_PLAN_ADDRESSES
            || self
                .list
                .iter()
                .any(|addr| addr.is_empty() || addr.len() > MAX_PLAN_ADDRESS_BYTES)
        {
            return Err(format!(
                "a plan names 1 to {MAX_PLAN_ADDRESSES} addresses of at most \
                 {MAX_PLAN_ADDRESS_BYTES} bytes"
            ));
        }
        if self.steps.is_empty() || self.steps.len() > MAX_PLAN_STEPS {
            return Err(format!("a plan compiles to 1 to {MAX_PLAN_STEPS} steps"));
        }
        for step in &self.steps {
            let ok = match step {
                PlanStep::Sound { field } => address::field_of_index(*field).is_some(),
                PlanStep::Mine { pocket } => {
                    address::pocket_address(pocket[0], pocket[1]).is_some()
                }
                PlanStep::Block { letter } => address::block_of_letter(*letter).is_some(),
            };
            if !ok {
                return Err("a plan's steps are fields, pockets and blocks of its site".to_owned());
            }
        }
        if usize::from(self.cursor) > self.steps.len() {
            return Err("a plan's cursor is within its steps".to_owned());
        }
        let distinct: BTreeSet<u16> = self.passed.iter().copied().collect();
        let on_block = matches!(
            self.steps.get(usize::from(self.cursor)),
            Some(PlanStep::Block { .. })
        );
        if distinct.len() != self.passed.len()
            || self.passed.len() > MAX_PLAN_PASSED
            || self.passed.iter().any(|n| *n == 0 || *n > 256)
            || (!self.passed.is_empty() && !on_block)
        {
            return Err(
                "a plan passes over distinct pockets of the block step it works".to_owned(),
            );
        }
        if self.seq == 0 {
            return Err("a plan's seq counts from 1".to_owned());
        }
        if self.notes.len() > MAX_PLAN_NOTES
            || self
                .notes
                .windows(2)
                .any(|pair| match (pair.first(), pair.get(1)) {
                    (Some(a), Some(b)) => a.seq >= b.seq,
                    _ => true,
                })
            || self.notes.iter().any(|note| note.seq > self.seq)
        {
            return Err(format!(
                "a plan keeps at most {MAX_PLAN_NOTES} notes, in order, none past its seq"
            ));
        }
        if let FillState::Ended { at_ic, .. } = self.state
            && at_ic < self.filed_ic
        {
            return Err("a plan ends no earlier than it was filed".to_owned());
        }
        Ok(())
    }

    /// The `{no}`-th step's words ([`step_words`]).
    #[must_use]
    pub fn step_words(&self, k: usize, long: bool) -> String {
        self.steps
            .get(k)
            .map_or_else(String::new, |step| step_words(step, long))
    }
}

/// **A step in words** (§10): `sound C-NW`, `mine C117`, and a block step `mine block D, fair or
/// better` — or, `long` (the preview), `mine every pocket of block D that reads fair or better`.
#[must_use]
pub fn step_words(step: &PlanStep, long: bool) -> String {
    match step {
        PlanStep::Sound { field } => {
            let name = address::field_of_index(*field)
                .and_then(|(fx, fy)| address::field_name(fx, fy))
                .unwrap_or_default();
            format!("sound {name}")
        }
        PlanStep::Mine { pocket } => format!(
            "mine {}",
            address::pocket_address(pocket[0], pocket[1]).unwrap_or_default()
        ),
        PlanStep::Block { letter } if long => {
            format!("mine every pocket of block {letter} that reads fair or better")
        }
        PlanStep::Block { letter } => format!("mine block {letter}, fair or better"),
    }
}

/// **The plan as the projection shows it** (lead ruling 10): pure over the record — the state
/// word, the step from 1, up to four step words from the cursor, the deadline and the end's word.
/// The step is clamped to the step count (a list worked out leaves the cursor past its last step),
/// and a plan that is parking or ended names no next step: it works none of them (the SM.11 fixer
/// round's correctness/F4 — a parking plan read "step 4 of 3" or named the mine step it skips).
#[must_use]
pub fn plan_view(plan: &FillPlan) -> crate::surface::excursion::PlanView {
    let (state, end) = match plan.state {
        FillState::Live => ("live", None),
        FillState::Parking { why } => ("parking", Some(why.word().to_owned())),
        FillState::Ended { why, .. } => ("ended", Some(why.word().to_owned())),
    };
    let from = usize::from(plan.cursor);
    let steps = u16::try_from(plan.steps.len()).unwrap_or(u16::MAX);
    let next = match plan.state {
        FillState::Live => plan
            .steps
            .iter()
            .skip(from)
            .take(crate::surface::excursion::MAX_PLAN_VIEW_NEXT)
            .map(|step| step_words(step, false))
            .collect(),
        FillState::Parking { .. } | FillState::Ended { .. } => Vec::new(),
    };
    crate::surface::excursion::PlanView {
        no: plan.no,
        state: state.to_owned(),
        step: plan.cursor.saturating_add(1).min(steps),
        steps,
        next,
        deadline_ic: plan.deadline_ic,
        end,
    }
}

/// **The plan view `viewer` may see** (the SM.11 fixer round's conformance/F1; doc 20 §5 and §16,
/// "no reply names what the asker may not see"): `plan`'s view only when `viewer` filed it, so a
/// previous lessee's plan — its pockets, its family, its tally — never reaches the next lessee's
/// projection, status or `plan show`. The tick's projection writers pass the rig's live lessee.
#[must_use]
pub fn plan_view_for(
    plan: Option<&FillPlan>,
    viewer: Option<ObjId>,
) -> Option<crate::surface::excursion::PlanView> {
    plan.filter(|plan| viewer == Some(plan.author))
        .map(plan_view)
}

// ---- the compiler -------------------------------------------------------------------------------

/// **Everything the compiler reads**, gathered by the caller (the command and the start's offer)
/// from the world; the compiler itself reads nothing (lead ruling 2). The readings are the
/// author's OWN: `classes` answers a pocket's R2 only where the author's row has sounded it, and
/// `field_readings` a field's R1 only where the row has scanned it.
pub(crate) struct CompileInput<'a> {
    /// The author's chart row of the site.
    pub row: Option<&'a ChartRecord>,
    /// The addresses as typed (commas and spaces allowed).
    pub list: &'a str,
    /// `family=` (an ore good id or a family word), when typed.
    pub family: Option<OreFamily>,
    /// `licence_verdict(author, now)`.
    pub licence: Result<i64, LicenceEnd>,
    /// `lease_gate(the rig's lease)`.
    pub lease: Result<(), LeaseBar>,
    /// `mine_tenure` for a block letter.
    pub tenure: &'a dyn Fn(char) -> Result<(), TenureBar>,
    /// The author's R2 of a site pocket, `None` unread.
    pub classes: &'a dyn Fn((i64, i64)) -> Option<[Word; 4]>,
    /// The author's R1 of a site field, `None` unread.
    pub field_readings: &'a dyn Fn((i64, i64)) -> Option<[FieldReading; 4]>,
    /// The rig's cell, or the ramp's from the bay.
    pub start_cell: (i64, i64),
    /// `home_cu_to` over the warm home field (`None`: no way home).
    pub home: &'a dyn Fn((i64, i64)) -> Option<u32>,
    /// The rig's battery row.
    pub battery: &'a BatteryDef,
    /// The rig's scanner row.
    pub scanner: &'a ScannerDef,
    /// The rig's drill row.
    pub drill: &'a DrillDef,
    /// The hold's unreserved room, whole tonnes (`hold_unreserved_kg`).
    pub hold_room_t: u64,
    /// The hopper's mass limit, whole tonnes (a trip's tonnes for the estimate).
    pub hopper_t: u64,
}

/// **A compiled plan** — what the preview prints and `go` files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Compiled {
    /// The addresses, upper-cased, in list order.
    pub list: Vec<String>,
    /// The family, frozen.
    pub family: OreFamily,
    /// The steps.
    pub steps: Vec<PlanStep>,
    /// The whole plan's estimate, IC seconds — a lower bound.
    pub est_secs: u64,
    /// Each step's own estimate, IC seconds (0: not estimated — a sounding, a block, an unread
    /// pocket).
    pub step_secs: Vec<u64>,
    /// `(block step index, k)`: the block's pockets the author reads fair or better that a full
    /// charge cannot work and bring home (`PLAN_PREVIEW_FAR`).
    pub far: Vec<(u16, u16)>,
}

/// **Why a list does not compile** (§3.3's order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PlanRefusal {
    /// Not a list of pockets and blocks.
    Usage,
    /// More than [`MAX_PLAN_ADDRESSES`].
    TooMany,
    /// Named twice (or a pocket inside a named block).
    Twice {
        /// The address.
        addr: String,
    },
    /// Not in the author's knowledge ("You have no reading there.").
    NoReading {
        /// The address.
        addr: String,
    },
    /// The author has sounded it and reads it barren in the family.
    Barren {
        /// The address.
        addr: String,
        /// The family.
        family: OreFamily,
    },
    /// The licence.
    Licence {
        /// The standing.
        why: LicenceEnd,
    },
    /// The lease.
    Lease {
        /// The bar.
        bar: LeaseBar,
    },
    /// Another's live claim ("That block is under claim.").
    Claimed {
        /// The address.
        addr: String,
    },
    /// The registry cannot be read.
    Dark {
        /// The address.
        addr: String,
    },
    /// A full charge cannot work it and bring the rig home.
    TooFar {
        /// The address.
        addr: String,
    },
    /// More than the bound.
    TooLong {
        /// Whole IC hours, rounded up.
        hours: u64,
    },
    /// More than [`MAX_PLAN_STEPS`].
    TooManySteps,
    /// The hold has no room for one cycle.
    HoldFull,
}

/// One parsed address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Addr {
    Pocket((i64, i64)),
    Block(char),
}

/// The 64 pockets of a site field, north-west first.
pub(crate) fn field_pockets((fx, fy): (i64, i64)) -> Vec<(i64, i64)> {
    let side = crate::surface::geology::POCKETS_PER_FIELD_SIDE;
    let mut out = Vec::with_capacity(64);
    for py in (0..side).rev() {
        for px in 0..side {
            out.push((fx * side + px, fy * side + py));
        }
    }
    out
}

/// A block's four fields, south-west first.
fn block_fields(letter: char) -> Vec<(i64, i64)> {
    let Some((bx, by)) = address::block_of_letter(letter) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(4);
    for dy in 0..2 {
        for dx in 0..2 {
            out.push((bx * 2 + dx, by * 2 + dy));
        }
    }
    out
}

/// A field's middle cell (`drive::target_cell` of `Target::Field`).
fn field_middle((fx, fy): (i64, i64)) -> (i64, i64) {
    let side = address::CELLS_PER_FIELD;
    (fx * side + side / 2, fy * side + side / 2)
}

/// A pocket's address cell (`drive::target_cell` of `Target::Pocket`).
pub(crate) fn pocket_cell((px, py): (i64, i64)) -> (i64, i64) {
    let side = address::CELLS_PER_POCKET;
    (px * side + side / 2, py * side + side / 2)
}

/// The rank of a class or a reading (`(high, low)` word codes): an R2 class reads exactly.
fn rank_of(reading: FieldReading) -> Option<(u8, u8)> {
    match reading {
        FieldReading::Exact(word) => Some((word.code(), word.code())),
        FieldReading::Band(low, high) => Some((high.code(), low.code())),
        FieldReading::NoReturn => None,
    }
}

/// The richest family over a set of ranked readings, ties to `OreFamily::ALL`'s order; barren
/// everywhere → `None`.
fn richest(readings: &[[Option<(u8, u8)>; 4]]) -> Option<OreFamily> {
    let mut best: Option<((u8, u8), OreFamily)> = None;
    for row in readings {
        for (family, rank) in OreFamily::ALL.into_iter().zip(row.iter()) {
            let Some(rank) = rank else {
                continue;
            };
            if rank.0 == 0 {
                continue;
            }
            if best.is_none_or(|(kept, kept_family)| {
                *rank > kept || (*rank == kept && family.code() < kept_family.code())
            }) {
                best = Some((*rank, family));
            }
        }
    }
    best.map(|(_, family)| family)
}

/// A pocket's R2 classes as ranks.
fn class_ranks(classes: [Word; 4]) -> [Option<(u8, u8)>; 4] {
    classes.map(|word| Some((word.code(), word.code())))
}

/// A field's R1 readings as ranks.
fn field_ranks(readings: [FieldReading; 4]) -> [Option<(u8, u8)>; 4] {
    readings.map(rank_of)
}

/// **Compile a list** (lead ruling 2; the brief's 3.3), in this order, each refusal naming the
/// first address that fails in list order: parse; the rights; the knowledge; the family (fixed
/// before any per-family check, the reviewer's F13); a sounded-barren pocket; the tenure; the
/// steps; the charge leg by leg from a full battery; one landed hold; the bound.
///
/// # Errors
///
/// The [`PlanRefusal`].
#[allow(clippy::too_many_lines)] // one ordered list of refusals, then the steps
pub(crate) fn compile(input: &CompileInput<'_>) -> Result<Compiled, PlanRefusal> {
    // 1. Parse.
    let mut addrs: Vec<(String, Addr)> = Vec::new();
    for token in input
        .list
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|token| !token.is_empty())
    {
        let upper = token.to_ascii_uppercase();
        let mut chars = upper.chars();
        let parsed = match (chars.next(), chars.next()) {
            (Some(letter), None) if address::block_of_letter(letter).is_some() => {
                Addr::Block(letter)
            }
            _ => match address::parse_pocket_address(&upper) {
                Some(pocket) => Addr::Pocket(pocket),
                None => return Err(PlanRefusal::Usage),
            },
        };
        let canonical = match parsed {
            Addr::Block(letter) => letter.to_string(),
            Addr::Pocket((px, py)) => address::pocket_address(px, py).ok_or(PlanRefusal::Usage)?,
        };
        addrs.push((canonical, parsed));
    }
    if addrs.is_empty() {
        return Err(PlanRefusal::Usage);
    }
    if addrs.len() > MAX_PLAN_ADDRESSES {
        return Err(PlanRefusal::TooMany);
    }
    let blocks: BTreeSet<char> = addrs
        .iter()
        .filter_map(|(_, addr)| match addr {
            Addr::Block(letter) => Some(*letter),
            Addr::Pocket(_) => None,
        })
        .collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (canonical, addr) in &addrs {
        let inside = match addr {
            Addr::Pocket((px, py)) => {
                address::block_letter(*px, *py).is_some_and(|letter| blocks.contains(&letter))
            }
            Addr::Block(_) => false,
        };
        if !seen.insert(canonical.clone()) || inside {
            return Err(PlanRefusal::Twice {
                addr: canonical.clone(),
            });
        }
    }
    // 2. The rights, before anything is revealed.
    if let Err(why) = input.licence {
        return Err(PlanRefusal::Licence { why });
    }
    if let Err(bar) = input.lease {
        return Err(PlanRefusal::Lease { bar });
    }
    // 3. The knowledge: the author's own row.
    let sounded_in = |letter: char| {
        input
            .row
            .filter(|row| row.rule == crate::surface::READING_RULE_VERSION)
            .and_then(|row| row.blocks.get(letter.to_string().as_str()))
            .is_some_and(|block| block.sounded() > 0)
    };
    for (canonical, addr) in &addrs {
        let known = match addr {
            Addr::Pocket(pocket) => {
                (input.classes)(*pocket).is_some()
                    || address::field_of_pocket(*pocket)
                        .is_some_and(|field| (input.field_readings)(field).is_some())
            }
            Addr::Block(letter) => {
                sounded_in(*letter)
                    || block_fields(*letter)
                        .into_iter()
                        .any(|field| (input.field_readings)(field).is_some())
            }
        };
        if !known {
            return Err(PlanRefusal::NoReading {
                addr: canonical.clone(),
            });
        }
    }
    // 4. The family, fixed before any per-family check (the reviewer's F13).
    let family = match input.family {
        Some(family) => family,
        None => {
            let first = addrs.first().map(|(_, addr)| *addr);
            let named = match first {
                Some(Addr::Pocket(pocket)) => (input.classes)(pocket)
                    .and_then(|classes| richest(&[class_ranks(classes)]))
                    .or_else(|| {
                        address::field_of_pocket(pocket)
                            .and_then(|field| (input.field_readings)(field))
                            .and_then(|readings| richest(&[field_ranks(readings)]))
                    }),
                Some(Addr::Block(letter)) => {
                    let mut ranks: Vec<[Option<(u8, u8)>; 4]> = Vec::new();
                    for field in block_fields(letter) {
                        if let Some(readings) = (input.field_readings)(field) {
                            ranks.push(field_ranks(readings));
                        }
                        for pocket in field_pockets(field) {
                            if let Some(classes) = (input.classes)(pocket) {
                                ranks.push(class_ranks(classes));
                            }
                        }
                    }
                    richest(&ranks)
                }
                None => None,
            };
            named.unwrap_or(OreFamily::Metals)
        }
    };
    // 5. A pocket the author has sounded and reads barren in the family.
    for (canonical, addr) in &addrs {
        if let Addr::Pocket(pocket) = addr
            && (input.classes)(*pocket)
                .is_some_and(|classes| by_family(classes, family) == Word::Barren)
        {
            return Err(PlanRefusal::Barren {
                addr: canonical.clone(),
                family,
            });
        }
    }
    // 6. The tenure, once a letter, in list order.
    let mut asked: BTreeSet<char> = BTreeSet::new();
    for (canonical, addr) in &addrs {
        let letter = match addr {
            Addr::Block(letter) => Some(*letter),
            Addr::Pocket((px, py)) => address::block_letter(*px, *py),
        };
        let Some(letter) = letter else {
            continue;
        };
        if !asked.insert(letter) {
            continue;
        }
        match (input.tenure)(letter) {
            Ok(()) => {}
            Err(TenureBar::Claimed) => {
                return Err(PlanRefusal::Claimed {
                    addr: canonical.clone(),
                });
            }
            Err(TenureBar::Dark(_)) => {
                return Err(PlanRefusal::Dark {
                    addr: canonical.clone(),
                });
            }
        }
    }
    // 7. The steps, walking the list from the start cell.
    let mut steps: Vec<PlanStep> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut from = input.start_cell;
    let sounded_field = |steps: &[PlanStep], index: u8| {
        steps
            .iter()
            .any(|step| matches!(step, PlanStep::Sound { field } if *field == index))
    };
    for (canonical, addr) in &addrs {
        match addr {
            Addr::Pocket(pocket) => {
                if (input.classes)(*pocket).is_none()
                    && let Some(field) = address::field_of_pocket(*pocket)
                    && let Some(index) = address::field_index(field.0, field.1)
                    && !sounded_field(&steps, index)
                {
                    steps.push(PlanStep::Sound { field: index });
                    sources.push(canonical.clone());
                }
                steps.push(PlanStep::Mine {
                    pocket: [pocket.0, pocket.1],
                });
                sources.push(canonical.clone());
                from = pocket_cell(*pocket);
            }
            Addr::Block(letter) => {
                let mut fields: Vec<((i64, i64), u8)> = block_fields(*letter)
                    .into_iter()
                    .filter(|field| chart::sound_adds(input.row, &field_pockets(*field)))
                    .filter_map(|field| {
                        address::field_index(field.0, field.1).map(|index| (field, index))
                    })
                    .filter(|(_, index)| !sounded_field(&steps, *index))
                    .collect();
                let start = from;
                fields.sort_by_key(|(field, index)| {
                    (cells_apart(start, field_middle(*field)), *index)
                });
                for (field, index) in fields {
                    steps.push(PlanStep::Sound { field: index });
                    sources.push(canonical.clone());
                    from = field_middle(field);
                }
                steps.push(PlanStep::Block { letter: *letter });
                sources.push(canonical.clone());
            }
        }
        if steps.len() > MAX_PLAN_STEPS {
            return Err(PlanRefusal::TooManySteps);
        }
    }
    // 8. The charge, leg by leg, from a full battery (the reviewer's F8 and F14): the drive out is
    // bounded by the home field's cost back, so `2 × home + the act` must leave the floor.
    let full = input.battery.capacity_cu;
    let affords = |stand: (i64, i64), act_cu: u32| -> bool {
        let Some(home) = (input.home)(stand) else {
            return false;
        };
        let cost = home.saturating_add(act_cu);
        reserve_check(full, 0, cost, home, input.battery).is_ok()
    };
    let mut far: Vec<(u16, u16)> = Vec::new();
    for (index, (step, source)) in steps.iter().zip(sources.iter()).enumerate() {
        let too_far = || PlanRefusal::TooFar {
            addr: source.clone(),
        };
        match step {
            PlanStep::Sound { field } => {
                let middle = address::field_of_index(*field).map(field_middle);
                if !middle.is_some_and(|stand| affords(stand, input.scanner.sound_cu)) {
                    return Err(too_far());
                }
            }
            PlanStep::Mine { pocket } => {
                if !affords(pocket_cell((pocket[0], pocket[1])), input.drill.energy_cu) {
                    return Err(too_far());
                }
            }
            PlanStep::Block { letter } => {
                let mut k: u16 = 0;
                for field in block_fields(*letter) {
                    for pocket in field_pockets(field) {
                        let fair = (input.classes)(pocket)
                            .is_some_and(|classes| by_family(classes, family) >= Word::Fair);
                        if fair && !affords(pocket_cell(pocket), input.drill.energy_cu) {
                            k = k.saturating_add(1);
                        }
                    }
                }
                if k > 0 {
                    far.push((u16::try_from(index).unwrap_or(u16::MAX), k));
                }
            }
        }
    }
    // 9. One landed hold: a plan that could only park is refused.
    if input.hold_room_t < u64::from(blind_room_t(input.drill)) {
        return Err(PlanRefusal::HoldFull);
    }
    // 10. The bound: a lower bound of the work.
    let mut step_secs: Vec<u64> = Vec::with_capacity(steps.len());
    let mut tonnes: u64 = 0;
    for step in &steps {
        let secs = match step {
            PlanStep::Sound { .. } | PlanStep::Block { .. } => match step {
                PlanStep::Sound { .. } => u64::from(input.scanner.sound_s),
                _ => 0,
            },
            PlanStep::Mine { pocket } => {
                let pocket = (pocket[0], pocket[1]);
                match (input.classes)(pocket).map(|classes| by_family(classes, family)) {
                    Some(class) if class != Word::Barren => {
                        let least = u64::from(class.tonnes_range().0);
                        let per = u64::from(class_yield(input.drill, class)).max(1);
                        let cycles = least.div_ceil(per);
                        let rock = address::field_of_pocket(pocket)
                            .and_then(|field| chart::row_reads_field(input.row, field))
                            == Some(Footing::Rock);
                        let footing = if rock {
                            Footing::Rock
                        } else {
                            Footing::Regolith
                        };
                        tonnes = tonnes.saturating_add(least);
                        cycles.saturating_mul(u64::from(
                            cycle_secs(input.drill, footing).unwrap_or(u32::MAX),
                        ))
                    }
                    Some(_) | None => 0,
                }
            }
        };
        step_secs.push(secs);
    }
    let trips = tonnes.div_ceil(input.hopper_t.max(1));
    let est_secs = step_secs
        .iter()
        .copied()
        .fold(0_u64, u64::saturating_add)
        .saturating_add(trips.saturating_mul(u64::from(RECOVER_S.saturating_add(UNLOAD_S))));
    if est_secs > u64::try_from(PLAN_BOUND_IC_SECS).unwrap_or(u64::MAX) {
        return Err(PlanRefusal::TooLong {
            hours: est_secs.div_ceil(3_600),
        });
    }
    Ok(Compiled {
        list: addrs.into_iter().map(|(canonical, _)| canonical).collect(),
        family,
        steps,
        est_secs,
        step_secs,
        far,
    })
}

// ---- the interpreter --------------------------------------------------------------------------

/// **Why a plan's next step cannot start now** (the tick caches each, the texts word each).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanWait {
    /// An act, a run or a queued unload is in flight (never told, never cached).
    Busy,
    /// The rig charges in the bay until this IC instant.
    Charging {
        /// The instant the charge suffices.
        until_ic: i64,
    },
    /// The author no longer holds the hull's helm.
    Helm,
    /// The author is not aboard the hull.
    Ashore,
    /// The author has as many rigs out as one pilot may run.
    Cap,
    /// The site's ground or home field is still being mapped.
    Cold,
    /// The rig, the server or the ground cannot be read now.
    Dark,
    /// **The hopper's queued unload waits on its full marker ring** (SM.8's ring; the SM.11 fixer
    /// round's correctness/F1; the lead's 0a note: "a wait with the record's back-off, never a
    /// park"; the lead's pre-commit round, item 1; the reviewer's R1): a queued unload stands and
    /// its hopper's ring cannot record the moves it would make
    /// (`crate::surface::act::queued_ring_full`) — whether the ring filled before the queued start
    /// (the common way: earlier unloads) or between its start and its finish (the finish then
    /// re-files the queue, `queue_waits_on_markers`). The surface tick submits no start into such
    /// a ring: it holds, tells `WHY_MARKERS_QUEUED` once (queued under the notice budget for an
    /// offline pilot) and re-reads the ring every 60 fires; the start's offer refusing
    /// `unload_markers` is only the backstop for a ring that filled in between. When the ring
    /// frees the unload lands and the plan goes on by itself. Never cached and never told by the
    /// plan; `+tv/plan show` names it.
    Markers,
}

impl PlanWait {
    /// The wait's code for the tick's once-a-boot told set.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            PlanWait::Busy => 0,
            PlanWait::Charging { .. } => 1,
            PlanWait::Helm => 2,
            PlanWait::Ashore => 3,
            PlanWait::Cap => 4,
            PlanWait::Cold => 5,
            PlanWait::Dark => 6,
            PlanWait::Markers => 7,
        }
    }
}

/// **What a plan's start writes beside the act** (the cursor, the passes, a parking, the notes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanUpdate {
    /// The cursor after the walk.
    pub cursor: u16,
    /// The current block step's passed pockets after the walk.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passed: Vec<u16>,
    /// Set when the plan begins to park now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parking: Option<PlanEnd>,
    /// The notes the walk made, oldest first (at most [`MAX_PLAN_NOTES`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<PlanNoteKind>,
}

impl PlanUpdate {
    fn of(plan: &FillPlan) -> PlanUpdate {
        PlanUpdate {
            cursor: plan.cursor,
            passed: plan.passed.clone(),
            parking: None,
            notes: Vec::new(),
        }
    }

    fn note(&mut self, note: PlanNoteKind) {
        self.notes.push(note);
        while self.notes.len() > MAX_PLAN_NOTES {
            self.notes.remove(0);
        }
    }
}

/// **The interpreter's move** — never `Serialize`: its [`PlanAct`] holds a `MinePlan` (truth).
#[derive(Clone, Debug)]
pub enum PlanMove {
    /// Start an act: the start word (an act word of the start), the step it serves, the plan's
    /// update the start's prepare writes with the act, and the act's world plan.
    Start {
        /// The start's `act` word: `drive`, `sound`, `mine` or `recover`.
        word: &'static str,
        /// The step it serves.
        step: u16,
        /// The plan's update.
        update: PlanUpdate,
        /// The act's world plan.
        act: PlanAct,
    },
    /// End the plan (nothing to start): `plan_end` with its reason and update.
    End {
        /// Why.
        why: PlanEnd,
        /// The update.
        update: PlanUpdate,
    },
    /// Nothing can start now.
    Wait(PlanWait),
}

/// **An act's world plan, opaque outside the crate** — the same values the hand path builds.
#[derive(Clone, Debug)]
pub struct PlanAct(pub(crate) PlanActInner);

/// [`PlanAct`]'s payload.
#[derive(Clone, Debug)]
pub(crate) enum PlanActInner {
    /// A sounding or a recover.
    Act(ActPlan),
    /// A drive (its leg is the plan's own).
    Drive(ActPlan),
    /// A mining run.
    Mine(Box<MinePlan>),
}

/// **How the interpreter takes a world plan's refusal** (the brief's 3.4 step 6, D18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fate {
    /// Set parking with this reason.
    Park(PlanEnd),
    /// Set parking `Licence { why }` from a fresh `licence_verdict` (`Ok` → `Wait(Dark)`: a race).
    Licence,
    /// Recover now; the step stays.
    Recover,
    /// `no_fit` / `blind_room`: the hopper bound → recover, the hold bound → park `HoldFull`.
    Room,
    /// A charge shortfall: in the bay below full → charging, in the bay at full → pass `TooFar`,
    /// out → recover; but from a recover (the way home itself) → end `Stranded`.
    Short,
    /// End `Stranded` (a recover with no way home at all).
    Stranded,
    /// Pass this pocket or field and continue the walk.
    Pass(PassWhy),
    /// Wait.
    Wait(PlanWait),
    /// A word a plan's own move cannot produce: `Wait(Dark)` and one `tracing::error!`; the
    /// 288-hour bound ends it if it persists.
    Unreachable,
}

/// **The refusal table** (the brief's 3.4 step 6; the reviewer's F2): every `Refusal` variant, one
/// exhaustive match — no wildcard — so a later cut's new word must be given its fate here.
#[must_use]
pub(crate) fn fate_of(refusal: &Refusal) -> Fate {
    match refusal {
        Refusal::Dark { .. }
        | Refusal::Unwired
        | Refusal::InstrumentsDark
        | Refusal::RecoverDark { .. } => Fate::Wait(PlanWait::Dark),
        Refusal::NotAboard { .. } => Fate::Wait(PlanWait::Ashore),
        Refusal::NotSite { .. } | Refusal::UnderWay { .. } | Refusal::Cleared { .. } => {
            Fate::Wait(PlanWait::Dark)
        }
        Refusal::Out { .. }
        | Refusal::Open { .. }
        | Refusal::InBay { .. }
        | Refusal::Elsewhere { .. }
        | Refusal::MineInBay { .. } => Fate::Unreachable,
        Refusal::Forfeit { .. } | Refusal::MineForfeit { .. } => {
            Fate::Park(PlanEnd::Lease { forfeit: true })
        }
        Refusal::Ended { .. } | Refusal::MineEnded { .. } => {
            Fate::Park(PlanEnd::Lease { forfeit: false })
        }
        Refusal::Licence { .. } => Fate::Licence,
        Refusal::Claimed => Fate::Park(PlanEnd::Claimed),
        Refusal::Far { .. } => Fate::Stranded,
        Refusal::Reserve { .. } | Refusal::LegShort { .. } => Fate::Short,
        Refusal::Cap { .. } => Fate::Wait(PlanWait::Cap),
        Refusal::GroundCold { .. } => Fate::Wait(PlanWait::Cold),
        Refusal::Busy { .. } | Refusal::MineRunning { .. } => Fate::Wait(PlanWait::Busy),
        Refusal::NothingNew => Fate::Pass(PassWhy::NothingNew),
        Refusal::ChartFull { .. } => Fate::Park(PlanEnd::ChartFull),
        Refusal::NoFooting { .. }
        | Refusal::OffSite { .. }
        | Refusal::Here { .. }
        | Refusal::NoPath { .. }
        | Refusal::Winding { .. }
        | Refusal::NoWayHome { .. }
        | Refusal::MineNoFooting { .. } => Fate::Pass(PassWhy::NoWay),
        Refusal::Locked => Fate::Pass(PassWhy::Locked),
        Refusal::WorkedOut { .. } => Fate::Pass(PassWhy::WorkedOut),
        Refusal::NoOre { .. } | Refusal::AllBarren { .. } => Fate::Pass(PassWhy::Barren),
        Refusal::HopperFull { .. } => Fate::Recover,
        Refusal::HoldFull { .. } => Fate::Park(PlanEnd::HoldFull),
        Refusal::NoFit { .. } | Refusal::BlindRoom { .. } => Fate::Room,
        Refusal::BlindCount { .. } => Fate::Unreachable,
        Refusal::DumpNotSite { .. } | Refusal::NoSpoil { .. } | Refusal::Empty { .. } => {
            Fate::Unreachable
        }
        Refusal::Unplanned | Refusal::Moved | Refusal::MineChanged => Fate::Unreachable,
        // Doc 20 cut SM.8: a plan never starts an unload (SM.8's queued unload is the tick's own,
        // and the plan waits on it).
        Refusal::UnloadUnderWay { .. }
        | Refusal::UnloadNotSite { .. }
        | Refusal::UnloadOut { .. }
        | Refusal::UnloadNoRoom { .. }
        | Refusal::UnloadNoFit { .. }
        | Refusal::UnloadNoStock { .. }
        | Refusal::UnloadChanged { .. }
        | Refusal::UnloadMarkers { .. } => Fate::Unreachable,
        // This cut's prepare refusals: never a world plan's.
        Refusal::PlanLive { .. }
        | Refusal::PlanFiled { .. }
        | Refusal::PlanChanged { .. }
        | Refusal::PlanWaits { .. }
        | Refusal::PlanMoved { .. }
        | Refusal::PlanNone { .. }
        | Refusal::PlanEnded { .. } => Fate::Unreachable,
    }
}

/// **The instant a bay charge suffices** (the brief's 3.4 step 5(f)): the first IC instant `t ≥
/// now` at which `charge_for_act(charge, t, true, battery)` — the bay credited to the end of the
/// launch half — reaches `required`; `None` when it never does (past the capacity, or no
/// recharge), or the charge is not in the bay. Integer, exact.
#[must_use]
pub(crate) fn charging_until(
    charge: &crate::surface::excursion::Charge,
    now_ic: i64,
    required: u64,
    battery: &BatteryDef,
) -> Option<i64> {
    if !charge.in_bay || required > u64::from(battery.capacity_cu) {
        return None;
    }
    if u64::from(charge_for_act(charge, now_ic, true, battery)) >= required {
        return Some(now_ic);
    }
    let rate = u64::from(battery.bay_recharge_cu_per_ic_hour);
    if rate == 0 {
        return None;
    }
    let short = required.checked_sub(u64::from(charge.value_cu))?;
    // `value + ⌊e × rate / 3600⌋ ≥ required` ⇔ `e ≥ ⌈short × 3600 / rate⌉`, at `e = t + LAUNCH_S
    // − at`.
    let elapsed = short.checked_mul(3_600)?.div_ceil(rate);
    let at = charge
        .at_ic
        .checked_add(i64::try_from(elapsed).ok()?)?
        .checked_sub(i64::from(crate::surface::excursion::LAUNCH_S))?;
    Some(at.max(now_ic))
}

/// **What a charge shortfall does** (the brief's 3.4 step 5(f), D19), pure: out of the bay the
/// rig recovers (it charges in the bay, where the check runs again); in the bay at full charge the
/// stand is too far (passed); in the bay below it, the rig waits until the charge suffices — or,
/// when it never would, the stand is too far. So a step never loops drive and recover: the check
/// from a full bay is the one that decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnShort {
    /// Recover now.
    Recover,
    /// Pass the stand `TooFar`.
    Pass,
    /// Wait in the bay until this IC instant.
    Charge(i64),
}

/// [`OnShort`]'s rule.
#[must_use]
pub(crate) fn on_short(in_bay: bool, full: bool, until: Option<i64>) -> OnShort {
    match (in_bay, full, until) {
        (false, _, _) => OnShort::Recover,
        (true, true, _) | (true, false, None) => OnShort::Pass,
        (true, false, Some(until_ic)) => OnShort::Charge(until_ic),
    }
}

/// **The hold's room before any drive** (the brief's 3.4 step 5(d), D20), pure: `None` while one
/// cycle's need fits; else the plan parks `HoldFull` — from the bay it ends at once (no empty
/// trip), from the ground it recovers first.
#[must_use]
pub(crate) fn hold_gate(in_bay: bool, hold_t: u64, need_t: u32) -> Option<HoldGate> {
    if hold_t >= u64::from(need_t) {
        return None;
    }
    Some(if in_bay {
        HoldGate::EndHere
    } else {
        HoldGate::RecoverThenEnd
    })
}

/// [`hold_gate`]'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HoldGate {
    /// In the bay: `End { HoldFull }` with no start.
    EndHere,
    /// On the ground: recover, then end from the bay.
    RecoverThenEnd,
}

/// What one walk chose.
#[derive(Clone, Copy, Debug)]
enum Chosen {
    /// A field's sounding.
    Sound { field: (i64, i64) },
    /// A pocket's run, with the author's class (`None` unread) and whether it serves a block.
    Mine {
        pocket: (i64, i64),
        class: Option<Word>,
        block: bool,
    },
}

/// **The decision's counters** — test hooks (`#[doc(hidden)]` readers in `act.rs`): how many times
/// the interpreter ran, and how many drive legs it planned.
#[derive(Debug, Default)]
pub(crate) struct DecideCounters {
    /// `decide` calls that reached a plan.
    pub decides: std::sync::atomic::AtomicU64,
    /// Drive legs planned (`drive_plan` calls).
    pub legs: std::sync::atomic::AtomicU64,
    /// The offer's compiles.
    pub compiles: std::sync::atomic::AtomicU64,
}

/// **The counters' RAM slot** (the tick registers it; test hooks only).
#[derive(Clone, Debug, Default)]
pub(crate) struct PlanCounters(pub std::sync::Arc<DecideCounters>);

/// Bump a counter of the world's slot, if any.
pub(crate) fn count(w: &World, which: fn(&DecideCounters) -> &std::sync::atomic::AtomicU64) {
    if let Some(counters) = w.module_state::<PlanCounters>() {
        which(&counters.0).fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The author's R2 classes of a pocket, through the site's geology and the author's row (`None`
/// unread or dark); fields cached for the call.
pub(crate) fn read_classes(
    site: &chart::SiteContext,
    row: Option<&ChartRecord>,
    pocket: (i64, i64),
    cache: &mut BTreeMap<(i64, i64), Option<FieldGround>>,
) -> Option<[Word; 4]> {
    if !chart::row_reads_pocket(row, pocket) {
        return None;
    }
    let field = address::field_of_pocket(pocket)?;
    let ground = cache
        .entry(field)
        .or_insert_with(|| chart::field_ground_at(site, field).ok())
        .as_ref()?;
    let side = crate::surface::geology::POCKETS_PER_FIELD_SIDE;
    ground
        .pocket(pocket.0.rem_euclid(side), pocket.1.rem_euclid(side))
        .map(|row| crate::surface::reading::read_r2(&row))
}

/// The author's R1 readings of a field (`None` unread or dark).
pub(crate) fn read_field(
    site: &chart::SiteContext,
    row: Option<&ChartRecord>,
    field: (i64, i64),
) -> Option<[FieldReading; 4]> {
    let footing = chart::row_reads_field(row, field)?;
    let ground = chart::field_ground_at(site, field).ok()?;
    Some(crate::surface::reading::read_r1(
        &site.mac, &site.body, &ground, footing,
    ))
}

/// A block step's best pocket so far: its rank `(cells away, number)`, the pocket and its class.
type BlockPick = ((u64, u16), (i64, i64), Word);

/// The rig's tools rows.
struct Tools {
    battery: Option<BatteryDef>,
    scanner: Option<ScannerDef>,
    drill: Option<DrillDef>,
}

fn tools_of(w: &World, tv: ObjId, tools: &BTreeMap<CanonicalId, ToolDef>) -> Tools {
    use crate::surface::act::{
        battery_of, drill_of, rig_battery_id, rig_drill_id, rig_scanner_id, scanner_of,
    };
    Tools {
        battery: rig_battery_id(w, tv).and_then(|id| battery_of(tools, &id).copied()),
        scanner: rig_scanner_id(w, tv).and_then(|id| scanner_of(tools, &id).copied()),
        drill: rig_drill_id(w, tv).and_then(|id| drill_of(tools, &id).copied()),
    }
}

/// The hull whose helm `who` holds, if any (`crate::port::coop::helm_of`).
pub(crate) fn helm_hull(w: &World, who: ObjId) -> Option<ObjId> {
    crate::port::coop::helm_of(w, who).map(|helm| helm.hull)
}

/// **Whether SM.8's queued unload is orphaned under `plan`** (pure; the SM.8 handoff, the SM.11
/// fixer round's correctness/F3): the queue is the plan's author's own and the author's helm
/// (`helm`, [`helm_hull`]) is not the plan's hull. SM.8 refuses such a queue at every attempt until
/// any start lands (its own page stands), and no plan start can land while the author is off the
/// helm (the start's head reads the author's helm hull), so `decide` and the tick read the plan's
/// helm wait — told once to a connected author, cached — rather than a `Busy` that is never told.
/// The queue itself is dropped only by a start that lands: the author's, once they hold the helm
/// again (then it is no longer orphaned and SM.8 starts it), or the next lessee's.
#[must_use]
pub(crate) fn queue_orphaned(
    queued: &crate::surface::excursion::Queued,
    plan: &FillPlan,
    helm: Option<ObjId>,
) -> bool {
    queued.operator == plan.author && helm != Some(plan.hull)
}

/// **Whether SM.8's queued unload stands because its hopper's marker ring was full** (pure; the
/// lead's pre-commit round, item 1): the record's latest outcome is an unload refused
/// `MarkersFull` and the queue was re-filed after it (`Queued::after` is that unload's ordinal) —
/// the finish's own re-filing. The refusal's own told line already says `WHY_MARKERS_QUEUED`, so
/// the surface tick, holding such a queue on its full ring, does not tell it a second time (the
/// reviewer's R1); the plan's `Markers` wait reads the ring itself
/// (`crate::surface::act::queued_ring_full`).
#[must_use]
pub(crate) fn queue_waits_on_markers(record: &RigRecord) -> bool {
    use crate::surface::excursion::{ActKind, ActResult, RefusedCode};
    match (record.queued.as_ref(), record.last.as_ref()) {
        (Some(queued), Some(last)) => {
            last.kind == ActKind::Unload
                && last.result
                    == (ActResult::Refused {
                        code: RefusedCode::MarkersFull,
                    })
                && queued.after == last.ordinal
        }
        _ => false,
    }
}

/// **The hold's unreserved room in whole tonnes of the plan's ore, in every measure** (the SM.11
/// fixer round's correctness/F1): `crate::port::ship::hold_unreserved`'s per-measure room — every
/// hopper of `rigs` and every outstanding cycle taken off — divided by the family's ore good's
/// per-unit figures (`capacity::fits_in`, mining's own fit rule), so a hold whose packed volume
/// runs out before its mass reads full (the mass entry alone would not). `None` unreadable.
fn hold_room_t(
    w: &World,
    view: &crate::port::ship::HullView<'_>,
    rigs: &[ObjId],
    read: &dyn Fn(
        mush_inventory::record::EndpointRef,
        &CanonicalId,
    ) -> Result<u64, crate::port_tick::PoolReadError>,
    read_rig: &dyn Fn(ObjId) -> Result<Option<RigRecord>, String>,
    family: OreFamily,
) -> Option<u64> {
    let room = crate::port::ship::hold_unreserved(w, view, rigs, read, read_rig)?;
    let goods = w
        .module_state::<crate::TacsimStore>()
        .and_then(crate::TacsimStore::goods)?;
    let good = CanonicalId::new(crate::surface::opening::ore_good(family)).ok()?;
    let def = goods.good(&good)?;
    let mut per_unit: Vec<(CanonicalId, u64)> = Vec::with_capacity(room.len());
    for measure in room.keys() {
        per_unit.push((measure.clone(), def.per_unit(measure)?));
    }
    Some(crate::port::capacity::fits_in(&room, &per_unit).0)
}

/// **What a plan does in the bay with a hopper that cannot take one more cycle** (the SM.11 fixer
/// round's correctness/F1), [`bay_hopper`]'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BayHopper {
    /// The hopper's last unload left ore behind (a queued or whole unload with goods staying, or
    /// refused for the hold): the hold will not take it — one landed hold, so the plan ends here.
    HoldFull,
    /// No unload has been tried on this load (ore left by hand), or a HAND unload's was refused
    /// because the hopper's marker ring was full (the lead's pre-commit round, item 1: a queued
    /// unload's refusal keeps its queue, so the plan waits on it before reaching here; a hand one
    /// files no queue): the step goes on, and its recover queues the unload, which waits on the
    /// ring by the queue's own attempt rule — at most one trip before the queue or the answer
    /// above.
    Go,
}

/// **[`BayHopper`] from the record's latest outcome** (pure). In the bay with nothing in flight,
/// SM.8's queued unload has run, so a short hopper after an unload is one the unload could not
/// empty: launching would loop launch → recover → refused unload until the plan's bound — except
/// a full marker ring's refusal, which ages away (`MarkersFull` → [`BayHopper::Go`]).
pub(crate) fn bay_hopper(last: Option<&crate::surface::excursion::Outcome>) -> BayHopper {
    use crate::surface::excursion::{ActKind, ActResult, RefusedCode};
    match last {
        Some(last) if last.kind == ActKind::Unload => match last.result {
            ActResult::Refused {
                code: RefusedCode::MarkersFull,
            } => BayHopper::Go,
            ActResult::Refused { .. } => BayHopper::HoldFull,
            ActResult::Done if !last.stays.is_empty() => BayHopper::HoldFull,
            ActResult::Done => BayHopper::Go,
        },
        Some(_) | None => BayHopper::Go,
    }
}

/// **The interpreter** (the brief's 3.4): the next move of the plan `record` holds for `rig`'s
/// lessee, over the durable record and the world — the same for the tick and the start's offer.
/// `None` when the record holds no plan live for the rig's lessee (absent, ended or orphaned).
///
/// **The bound precedes every wait** (the lead's pre-commit round, item 3; the judge's
/// correctness/F5): once `now_ic` reaches the plan's deadline, any [`PlanMove::Wait`] the walk
/// would answer — the helm, ashore, dark, cold, the cap, a charge, a queued unload still standing
/// — is instead `End { Bound }` where the rig stands (told once, by the end's note), so a plan
/// whose author is ashore or whose rig is dark never waits past its 288 IC hours. Only an act or a
/// run in flight holds it (`Busy`: the end needs an idle rig, and the step finishes by its own
/// rules first). A plan past its bound with nothing barring it still parks as before: from the
/// ground it recovers, from the bay it ends.
#[allow(clippy::too_many_arguments)] // `decide_walk`'s arguments, passed through
pub(crate) fn decide(
    w: &World,
    reads: &dyn SurfaceReads,
    rig: &RigAt,
    record: &RigRecord,
    counts: &ExcursionCounts,
    tools: &BTreeMap<CanonicalId, ToolDef>,
    peeks: &mut Peeks,
    now_ic: i64,
) -> Option<PlanMove> {
    let moved = decide_walk(w, reads, rig, record, counts, tools, peeks, now_ic)?;
    let plan = record.live_plan_for(rig.record.pilot)?;
    let in_flight = record.act.is_some() || record.run.is_some();
    Some(match moved {
        PlanMove::Wait(_) if now_ic >= plan.deadline_ic && !in_flight => PlanMove::End {
            why: PlanEnd::Bound,
            update: PlanUpdate::of(plan),
        },
        other => other,
    })
}

/// **The walk** — [`decide`]'s body, before the bound's precedence. In this order: in flight
/// (`Busy`; SM.8's queued unload too while the plan's bound has not passed — `Markers` when it
/// waits on a full marker ring); the author's helm (`Helm`); the rights at the step's start
/// (the licence, the lease — parking); the bound (parking `Bound`); the author aboard (`Ashore`);
/// then, unless parking, the site's ground and home warm (`Cold`/`Dark`), the walk from the
/// cursor (passing what the author reads barren or the public pit shows worked out), and for the
/// chosen step the cap (from the bay), the hold's room (parking `HoldFull`), the hopper's room (a
/// recover), the combined reserve — leg + act + home — before the one A*; then the move, its world
/// plan's refusal classified by [`fate_of`]. A parking plan recovers from the ground and ends from
/// the bay.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // one ordered decision
fn decide_walk(
    w: &World,
    reads: &dyn SurfaceReads,
    rig: &RigAt,
    record: &RigRecord,
    counts: &ExcursionCounts,
    tools: &BTreeMap<CanonicalId, ToolDef>,
    peeks: &mut Peeks,
    now_ic: i64,
) -> Option<PlanMove> {
    let plan = record.live_plan_for(rig.record.pilot)?;
    count(w, |c| &c.decides);
    // 1. In flight — SM.8's queued unload too, unless it is orphaned (`queue_orphaned`: the
    // author's own queue, the author off the plan's hull's helm, so SM.8 refuses it at every
    // attempt until any start lands): such a queue is no step in flight, and the plan reads the
    // helm's wait below instead of a silent `Busy` (the SM.8 handoff; the SM.11 fixer round's
    // correctness/F3). A standing queue is a wait like any other once the plan's bound has
    // passed (the lead's pre-commit round, item 3: the walk goes on, and `decide` ends it); one
    // re-filed after a full marker ring reads `Markers` (item 1).
    if record.act.is_some() || record.run.is_some() {
        return Some(PlanMove::Wait(PlanWait::Busy));
    }
    if now_ic < plan.deadline_ic
        && let Some(queued) = record
            .queued
            .as_ref()
            .filter(|queued| !queue_orphaned(queued, plan, helm_hull(w, plan.author)))
    {
        // The reviewer's R1: the wait reads the ring itself — the rule the tick holds the queued
        // start by and the offer refuses it by — whichever way the ring filled (before the queued
        // start, or between its start and its finish).
        return Some(PlanMove::Wait(
            if crate::surface::act::queued_ring_full(w, reads, rig.tv, queued, w.now()) {
                PlanWait::Markers
            } else {
                PlanWait::Busy
            },
        ));
    }
    let author = plan.author;
    // 2. The helm (the reviewer's F18).
    if helm_hull(w, author) != Some(plan.hull) {
        return Some(PlanMove::Wait(PlanWait::Helm));
    }
    let mut update = PlanUpdate::of(plan);
    let mut parking: Option<PlanEnd> = match plan.state {
        FillState::Parking { why } => Some(why),
        FillState::Live => None,
        FillState::Ended { .. } => return None,
    };
    let park = |update: &mut PlanUpdate, why: PlanEnd| {
        update.parking = Some(why);
        if why.is_right() {
            update.note(PlanNoteKind::Parking { why });
        }
    };
    // 3. The rights at the step's start (lead ruling 3), and 4. the bound.
    if parking.is_none() {
        let why = match licence_verdict(w, author, now_ic) {
            Err(why) => Some(PlanEnd::Licence { why }),
            Ok(_) => match lease_gate(w, &rig.record) {
                Err(bar) => Some(PlanEnd::Lease {
                    forfeit: bar == LeaseBar::Forfeit,
                }),
                Ok(()) if now_ic >= plan.deadline_ic => Some(PlanEnd::Bound),
                Ok(()) => None,
            },
        };
        if let Some(why) = why {
            park(&mut update, why);
            parking = Some(why);
        }
    }
    // The author aboard (the brief's D4): every act, the parking recover too.
    if !crate::port::voyage::aboard(w, rig.hull, author) {
        return Some(PlanMove::Wait(PlanWait::Ashore));
    }
    let tools = tools_of(w, rig.tv, tools);
    let rates = crate::surface::act::rig_rates(w, rig.tv, tools.battery.as_ref());
    // Where the rig is: in this hull's bay, or on the plan's site's ground.
    let Some(view) = crate::port::ship::hull_view(w, rig.hull) else {
        return Some(PlanMove::Wait(PlanWait::Dark));
    };
    let Ok(berth) = crate::surface::act::site_berth(w, &view) else {
        return Some(PlanMove::Wait(PlanWait::Dark));
    };
    if berth.entry != plan.site {
        return Some(PlanMove::Wait(PlanWait::Dark));
    }
    let ramp_cell = site_cell(berth.ramp.x_cm, berth.ramp.y_cm);
    let (in_bay, rig_cell) = match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => (true, ramp_cell),
        Where::Ground {
            domain, x_cm, y_cm, ..
        } if domain == berth.domain => (false, site_cell(x_cm, y_cm)),
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Some(PlanMove::Wait(PlanWait::Dark));
        }
    };
    // The recover move, shared by the hopper's room, a short charge out and the park.
    let recover = |peeks: &mut Peeks, update: PlanUpdate, step: u16| -> PlanMove {
        match crate::surface::act::recover_plan(w, author, rig, peeks, rates) {
            Ok(act) => {
                let ramp = crate::surface::act::ramp_label(w, rig.hull);
                if crate::surface::act::leg_home(record, &act, tools.battery.as_ref(), rig, &ramp)
                    .is_err()
                {
                    return PlanMove::End {
                        why: PlanEnd::Stranded,
                        update,
                    };
                }
                PlanMove::Start {
                    word: "recover",
                    step,
                    update,
                    act: PlanAct(PlanActInner::Act(act)),
                }
            }
            Err(refusal) => match fate_of(&refusal) {
                Fate::Short | Fate::Stranded => PlanMove::End {
                    why: PlanEnd::Stranded,
                    update,
                },
                Fate::Wait(wait) => PlanMove::Wait(wait),
                Fate::Unreachable
                | Fate::Park(_)
                | Fate::Licence
                | Fate::Recover
                | Fate::Room
                | Fate::Pass(_) => {
                    tracing::error!(
                        tv = rig.tv.0,
                        word = refusal.word(),
                        "tacsim: a plan's recover met a refusal a recover cannot answer; it waits"
                    );
                    PlanMove::Wait(PlanWait::Dark)
                }
            },
        }
    };
    // 7. Park: recover from the ground, end from the bay.
    if let Some(why) = parking {
        let step = update.cursor;
        return Some(if in_bay {
            PlanMove::End { why, update }
        } else {
            recover(peeks, update, step)
        });
    }
    // 5. The site's ground and home warm.
    let ground = match chart::peek_site(w, &plan.site, peeks) {
        SitePeek::Warm(ground) => ground,
        SitePeek::Cold => return Some(PlanMove::Wait(PlanWait::Cold)),
        SitePeek::Dark => return Some(PlanMove::Wait(PlanWait::Dark)),
    };
    let field = match crate::surface::peek_home(w, &plan.site, ramp_cell, peeks) {
        HomePeek::Warm(field) => field,
        HomePeek::Cold => return Some(PlanMove::Wait(PlanWait::Cold)),
        HomePeek::Dark => return Some(PlanMove::Wait(PlanWait::Dark)),
    };
    let (Some(battery), Some(scanner), Some(drill), Some(rates)) =
        (tools.battery, tools.scanner, tools.drill, rates)
    else {
        return Some(PlanMove::Wait(PlanWait::Dark));
    };
    let Ok(site) = chart::site_context(w, &plan.site) else {
        return Some(PlanMove::Wait(PlanWait::Dark));
    };
    let Ok(row) = reads.chart(author, &plan.site) else {
        return Some(PlanMove::Wait(PlanWait::Dark));
    };
    let row = row.as_ref();
    let mut cache: BTreeMap<(i64, i64), Option<FieldGround>> = BTreeMap::new();
    let family = plan.family;
    let pit_out = |pocket: (i64, i64)| {
        crate::port::site_map::pit_exhausted(reads, &plan.site, &ground, pocket, family)
    };
    let hull_rigs: Vec<ObjId> = crate::port::tv::live_on_hull(w, rig.hull)
        .iter()
        .map(|lease| lease.tv)
        .collect();
    let read_pool = |endpoint: mush_inventory::record::EndpointRef, good: &CanonicalId| {
        reads.pool(endpoint, good)
    };
    let read_rig = |tv: ObjId| reads.rig(tv);
    let mut passes: usize = 0;
    let bound = MAX_PLAN_STEPS.saturating_add(MAX_PLAN_PASSED);
    loop {
        passes = passes.saturating_add(1);
        if passes > bound {
            tracing::error!(
                tv = rig.tv.0,
                plan = plan.no,
                "tacsim: a fill plan's walk passed its bound; the plan ends dark"
            );
            let step = update.cursor;
            return Some(PlanMove::End {
                why: PlanEnd::Dark,
                update: PlanUpdate {
                    cursor: step,
                    ..update
                },
            });
        }
        let cursor = usize::from(update.cursor);
        let advance = |update: &mut PlanUpdate| {
            update.cursor = update.cursor.saturating_add(1);
            update.passed.clear();
        };
        // The walk: choose the step's target.
        let chosen = match plan.steps.get(cursor) {
            None => {
                park(&mut update, PlanEnd::WorkedOut);
                let step = update.cursor;
                return Some(if in_bay {
                    PlanMove::End {
                        why: PlanEnd::WorkedOut,
                        update,
                    }
                } else {
                    recover(peeks, update, step)
                });
            }
            Some(PlanStep::Sound { field: index }) => {
                let Some(field_xy) = address::field_of_index(*index) else {
                    advance(&mut update);
                    continue;
                };
                if !chart::sound_adds(row, &field_pockets(field_xy)) {
                    advance(&mut update);
                    continue;
                }
                Chosen::Sound { field: field_xy }
            }
            Some(PlanStep::Mine { pocket }) => {
                let pocket = (pocket[0], pocket[1]);
                let classes = read_classes(&site, row, pocket, &mut cache);
                if classes.is_some_and(|classes| by_family(classes, family) == Word::Barren) {
                    update.note(PlanNoteKind::Passed {
                        at: [pocket.0, pocket.1],
                        field: false,
                        why: PassWhy::Barren,
                    });
                    advance(&mut update);
                    continue;
                }
                if pit_out(pocket) {
                    // Worked out by this step's own last run: done, not passed over (no note).
                    let own = record.last.as_ref().is_some_and(|last| {
                        last.by
                            == Some(ActBy::Plan {
                                no: plan.no,
                                step: update.cursor,
                            })
                    });
                    if !own {
                        update.note(PlanNoteKind::Passed {
                            at: [pocket.0, pocket.1],
                            field: false,
                            why: PassWhy::WorkedOut,
                        });
                    }
                    advance(&mut update);
                    continue;
                }
                Chosen::Mine {
                    pocket,
                    class: classes.map(|classes| by_family(classes, family)),
                    block: false,
                }
            }
            Some(PlanStep::Block { letter }) => {
                let mut best: Option<BlockPick> = None;
                for index in 0..usize::try_from(address::POCKETS_IN_BLOCK).unwrap_or(256) {
                    let Some(pocket) = address::pocket_of_block_index(*letter, index) else {
                        continue;
                    };
                    let Some(number) = address::pocket_number(pocket.0, pocket.1) else {
                        continue;
                    };
                    if update.passed.contains(&number) {
                        continue;
                    }
                    let Some(class) = read_classes(&site, row, pocket, &mut cache)
                        .map(|classes| by_family(classes, family))
                    else {
                        continue;
                    };
                    if class < Word::Fair || pit_out(pocket) {
                        continue;
                    }
                    let rank = (cells_apart(rig_cell, pocket_cell(pocket)), number);
                    if best.is_none_or(|(kept, _, _)| rank < kept) {
                        best = Some((rank, pocket, class));
                    }
                }
                match best {
                    None => {
                        advance(&mut update);
                        continue;
                    }
                    Some((_, pocket, class)) => Chosen::Mine {
                        pocket,
                        class: Some(class),
                        block: true,
                    },
                }
            }
        };
        let step = update.cursor;
        // A pass of the chosen step: a block's pocket joins `passed`, anything else advances.
        let pass = |update: &mut PlanUpdate, why: PassWhy| match chosen {
            Chosen::Sound { field } => {
                update.note(PlanNoteKind::Passed {
                    at: [field.0, field.1],
                    field: true,
                    why,
                });
                update.cursor = update.cursor.saturating_add(1);
                update.passed.clear();
            }
            Chosen::Mine { pocket, block, .. } => {
                update.note(PlanNoteKind::Passed {
                    at: [pocket.0, pocket.1],
                    field: false,
                    why,
                });
                if block {
                    if let Some(number) = address::pocket_number(pocket.0, pocket.1)
                        && !update.passed.contains(&number)
                    {
                        update.passed.push(number);
                    }
                } else {
                    update.cursor = update.cursor.saturating_add(1);
                    update.passed.clear();
                }
            }
        };
        let (stand, target) = match chosen {
            Chosen::Sound { field } => (
                field_middle(field),
                Target::Field {
                    fx: field.0,
                    fy: field.1,
                },
            ),
            Chosen::Mine { pocket, .. } => (
                pocket_cell(pocket),
                Target::Pocket {
                    px: pocket.0,
                    py: pocket.1,
                },
            ),
        };
        // (b) The cap, only when the step implies a launch.
        if in_bay && cap_refusal(counts).is_some() {
            return Some(PlanMove::Wait(PlanWait::Cap));
        }
        // (d) The hold's room (the reviewer's F9): every step; a sounding serves the mining after.
        let need_t = match chosen {
            Chosen::Mine {
                class: Some(class), ..
            } if class != Word::Barren => class_yield(&drill, class),
            Chosen::Mine { .. } | Chosen::Sound { .. } => blind_room_t(&drill),
        };
        // In every measure the hold limits (the fixer round's correctness/F1), never the mass alone.
        let Some(hold_t) = hold_room_t(w, &view, &hull_rigs, &read_pool, &read_rig, family) else {
            return Some(PlanMove::Wait(PlanWait::Dark));
        };
        if let Some(gate) = hold_gate(in_bay, hold_t, need_t) {
            park(&mut update, PlanEnd::HoldFull);
            return Some(match gate {
                HoldGate::EndHere => PlanMove::End {
                    why: PlanEnd::HoldFull,
                    update,
                },
                HoldGate::RecoverThenEnd => recover(peeks, update, step),
            });
        }
        // (e) The hopper's room (a mine step, on the ground).
        let pool_unit = crate::port::tv::pool_unit(w);
        let Ok((hopper_kg, hopper_cap_kg)) =
            crate::port::tv::hopper_load_with(w, rig.tv, pool_unit.as_ref(), &read_pool)
        else {
            return Some(PlanMove::Wait(PlanWait::Dark));
        };
        let hopper_free_t = hopper_cap_kg.saturating_sub(hopper_kg) / 1_000;
        let mining = matches!(chosen, Chosen::Mine { .. });
        let hopper_short = hopper_free_t < u64::from(need_t);
        if hopper_short && in_bay {
            // In the bay a hopper its unload could not empty never launches again (the fixer
            // round's correctness/F1; every step, a sounding serving the mining after it):
            // `bay_hopper` reads the latest outcome.
            match bay_hopper(record.last.as_ref()) {
                BayHopper::HoldFull => {
                    park(&mut update, PlanEnd::HoldFull);
                    return Some(PlanMove::End {
                        why: PlanEnd::HoldFull,
                        update,
                    });
                }
                BayHopper::Go => {}
            }
        }
        if mining && hopper_short && !in_bay {
            return Some(recover(peeks, update, step));
        }
        // (f) The combined reserve (the reviewer's F8): leg + act + home, before the drive.
        let act_cu = match chosen {
            Chosen::Sound { .. } => scanner.sound_cu,
            Chosen::Mine { class, .. } => {
                let cycles = match class {
                    Some(class) if class != Word::Barren => {
                        let per = u64::from(class_yield(&drill, class)).max(1);
                        (hopper_free_t.min(hold_t) / per)
                            .saturating_add(1)
                            .min(u64::from(MAX_RUN_CYCLES))
                    }
                    Some(_) | None => 1,
                };
                drill
                    .energy_cu
                    .saturating_mul(u32::try_from(cycles).unwrap_or(u32::from(MAX_RUN_CYCLES)))
            }
        };
        let Some(home_cu) = drive::home_cu_to(&field, stand, &rates) else {
            pass(&mut update, PassWhy::NoWay);
            continue;
        };
        let standing = !in_bay && rig_cell == stand;
        let from = charge_for_act(&record.charge, now_ic, in_bay, &battery);
        let reserved = reserved_total(record);
        let short = |leg_cu: u32| {
            reserve_check(
                from,
                reserved,
                leg_cu.saturating_add(act_cu),
                home_cu,
                &battery,
            )
            .is_err()
        };
        // From the bay the leg out is bounded by the home field's cost back (symmetric moves), so a
        // stand a full battery cannot work is passed without an A*.
        let full = charge_at(&record.charge, now_ic, &battery) >= battery.capacity_cu;
        let floor = u64::from(battery.capacity_cu)
            .saturating_mul(u64::from(battery.reserve_permille))
            / 1_000;
        // The charge a leg of `leg_cu` needs (the reserve rule's own sum), and when the bay has it.
        let until = |leg_cu: u32| {
            let required = floor
                .saturating_add(u64::from(reserved))
                .saturating_add(u64::from(leg_cu))
                .saturating_add(u64::from(act_cu))
                .saturating_add(u64::from(home_cu));
            charging_until(&record.charge, now_ic, required, &battery)
        };
        if in_bay && short(home_cu) {
            match on_short(in_bay, full, until(home_cu)) {
                OnShort::Charge(until_ic) => {
                    return Some(PlanMove::Wait(PlanWait::Charging { until_ic }));
                }
                OnShort::Pass => {
                    pass(&mut update, PassWhy::TooFar);
                    continue;
                }
                OnShort::Recover => return Some(recover(peeks, update, step)),
            }
        }
        let leg = if standing {
            None
        } else {
            let stand_of = match crate::surface::act::stand_of(w, author, rig) {
                Ok(stand) => stand,
                Err(refusal) => match fate_of(&refusal) {
                    Fate::Wait(wait) => return Some(PlanMove::Wait(wait)),
                    Fate::Park(why) => {
                        park(&mut update, why);
                        return Some(if in_bay {
                            PlanMove::End { why, update }
                        } else {
                            recover(peeks, update, step)
                        });
                    }
                    Fate::Licence
                    | Fate::Recover
                    | Fate::Room
                    | Fate::Short
                    | Fate::Stranded
                    | Fate::Pass(_)
                    | Fate::Unreachable => {
                        tracing::error!(
                            tv = rig.tv.0,
                            word = refusal.word(),
                            "tacsim: a plan's stand met a refusal it cannot answer; it waits"
                        );
                        return Some(PlanMove::Wait(PlanWait::Dark));
                    }
                },
            };
            count(w, |c| &c.legs);
            match crate::surface::act::drive_plan(w, rig, &stand_of, &target, &rates, peeks) {
                Ok(planned) => Some(planned),
                Err(refusal) => match fate_of(&refusal) {
                    Fate::Pass(why) => {
                        pass(&mut update, why);
                        continue;
                    }
                    Fate::Wait(wait) => return Some(PlanMove::Wait(wait)),
                    Fate::Short => match on_short(in_bay, full, until(home_cu)) {
                        OnShort::Charge(until_ic) => {
                            return Some(PlanMove::Wait(PlanWait::Charging { until_ic }));
                        }
                        OnShort::Pass => {
                            pass(&mut update, PassWhy::TooFar);
                            continue;
                        }
                        OnShort::Recover => return Some(recover(peeks, update, step)),
                    },
                    Fate::Park(_)
                    | Fate::Licence
                    | Fate::Recover
                    | Fate::Room
                    | Fate::Stranded
                    | Fate::Unreachable => {
                        tracing::error!(
                            tv = rig.tv.0,
                            word = refusal.word(),
                            "tacsim: a plan's drive met a refusal it cannot answer; it waits"
                        );
                        return Some(PlanMove::Wait(PlanWait::Dark));
                    }
                },
            }
        };
        let leg_cu = leg.as_ref().map_or(0, |(plan, _)| plan.cost_cu);
        if short(leg_cu) {
            match on_short(in_bay, full, until(leg_cu)) {
                OnShort::Charge(until_ic) => {
                    return Some(PlanMove::Wait(PlanWait::Charging { until_ic }));
                }
                OnShort::Pass => {
                    pass(&mut update, PassWhy::TooFar);
                    continue;
                }
                OnShort::Recover => return Some(recover(peeks, update, step)),
            }
        }
        // The move: drive there, or the act standing there.
        if let Some((act, _leg)) = leg {
            return Some(PlanMove::Start {
                word: "drive",
                step,
                update,
                act: PlanAct(PlanActInner::Drive(act)),
            });
        }
        let planned: Result<PlanActInner, Refusal> = match chosen {
            Chosen::Sound { .. } => crate::surface::act::sound_plan(
                w,
                author,
                rig,
                &scanner,
                false,
                peeks,
                Some(&rates),
            )
            .map(PlanActInner::Act),
            Chosen::Mine { pocket, .. } => {
                let letter = address::block_letter(pocket.0, pocket.1).unwrap_or('A');
                let site_def = berth.def.site.as_ref();
                let now = w.world_ic_now();
                let tenure = match site_def {
                    Some(site_def) => crate::surface::act::mine_tenure(
                        w,
                        reads,
                        author,
                        (&plan.site, site_def),
                        letter,
                        &now,
                    ),
                    None => Err(TenureBar::Dark(crate::surface::excursion::DarkCause::Site)),
                };
                match tenure {
                    Err(TenureBar::Claimed) => {
                        park(&mut update, PlanEnd::Claimed);
                        return Some(recover(peeks, update, step));
                    }
                    Err(TenureBar::Dark(_)) => {
                        park(&mut update, PlanEnd::Dark);
                        return Some(recover(peeks, update, step));
                    }
                    Ok(()) => {}
                }
                crate::surface::act::mine_plan(
                    w,
                    reads,
                    author,
                    rig,
                    &MineChoice {
                        family: Some(family),
                        word: None,
                        cycles: None,
                    },
                    &drill,
                    Some(&rates),
                    peeks,
                )
                .map(|planned| PlanActInner::Mine(Box::new(planned)))
            }
        };
        let word = match chosen {
            Chosen::Sound { .. } => "sound",
            Chosen::Mine { .. } => "mine",
        };
        match planned {
            Ok(act) => {
                return Some(PlanMove::Start {
                    word,
                    step,
                    update,
                    act: PlanAct(act),
                });
            }
            Err(refusal) => match fate_of(&refusal) {
                Fate::Park(why) => {
                    park(&mut update, why);
                    return Some(recover(peeks, update, step));
                }
                Fate::Licence => {
                    return Some(match licence_verdict(w, author, now_ic) {
                        Err(why) => {
                            park(&mut update, PlanEnd::Licence { why });
                            recover(peeks, update, step)
                        }
                        Ok(_) => PlanMove::Wait(PlanWait::Dark),
                    });
                }
                Fate::Recover => return Some(recover(peeks, update, step)),
                Fate::Room => {
                    if hopper_free_t < u64::from(need_t) {
                        return Some(recover(peeks, update, step));
                    }
                    park(&mut update, PlanEnd::HoldFull);
                    return Some(recover(peeks, update, step));
                }
                Fate::Short => return Some(recover(peeks, update, step)),
                Fate::Stranded => {
                    return Some(PlanMove::End {
                        why: PlanEnd::Stranded,
                        update,
                    });
                }
                Fate::Pass(why) => {
                    pass(&mut update, why);
                    continue;
                }
                Fate::Wait(wait) => return Some(PlanMove::Wait(wait)),
                Fate::Unreachable => {
                    tracing::error!(
                        tv = rig.tv.0,
                        word = refusal.word(),
                        "tacsim: a plan's act met a refusal its own move cannot produce; it waits"
                    );
                    return Some(PlanMove::Wait(PlanWait::Dark));
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::chart::Via;

    fn battery() -> BatteryDef {
        BatteryDef {
            capacity_cu: 100_000,
            bay_recharge_cu_per_ic_hour: 50_000,
            drive_cu_per_nominal_cell: 15,
            reserve_permille: 50,
        }
    }

    fn tools() -> (ScannerDef, DrillDef) {
        let table = crate::definitions::tools::tools_table().unwrap_or_else(|e| panic!("{e}"));
        let scanner = table
            .values()
            .find_map(ToolDef::scanner)
            .copied()
            .unwrap_or_else(|| panic!("a scanner"));
        let drill = table
            .values()
            .find_map(ToolDef::drill)
            .copied()
            .unwrap_or_else(|| panic!("a drill"));
        (scanner, drill)
    }

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap_or_else(|_| panic!("{text}"))
    }

    fn sample() -> FillPlan {
        let (px, py) =
            address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117 parses"));
        let field = address::parse_field_name("D-SW")
            .and_then(|(fx, fy)| address::field_index(fx, fy))
            .unwrap_or_else(|| panic!("D-SW parses"));
        FillPlan {
            no: 1,
            author: ObjId(7),
            hull: ObjId(8),
            slot: 1,
            site: cid("ahuna_port"),
            family: OreFamily::Metals,
            filed_ic: 1_000,
            deadline_ic: 1_000 + PLAN_BOUND_IC_SECS,
            list: vec!["C117".to_owned(), "D".to_owned()],
            steps: vec![
                PlanStep::Mine { pocket: [px, py] },
                PlanStep::Sound { field },
                PlanStep::Block { letter: 'D' },
            ],
            cursor: 0,
            passed: Vec::new(),
            state: FillState::Live,
            seq: 1,
            tally: PlanTally::default(),
            notes: Vec::new(),
        }
    }

    /// The plan's numbers, pinned with their §11 basis.
    #[test]
    fn sm11_the_plan_numbers_are_pinned() {
        assert_eq!(PLAN_BOUND_IC_SECS, 1_036_800, "288 IC h");
        assert_eq!(IDLE_PARK_IC_SECS, 86_400, "24 IC h");
        assert_eq!(MAX_PLAN_ADDRESSES, 16);
        assert_eq!(MAX_PLAN_STEPS, 64);
        assert_eq!(MAX_PLAN_NOTES, 8);
        assert_eq!(
            PLAN_WAIT_IC_SECS,
            i64::from(crate::surface::excursion::LAUNCH_S)
        );
        assert_eq!(MAX_AWAIT_IC_HOURS, 288);
        assert_eq!(
            i64::from(MAX_AWAIT_IC_HOURS) * 3_600,
            PLAN_BOUND_IC_SECS,
            "the await's bound is the plan's"
        );
    }

    /// Each bound refused, and the sample passes.
    #[test]
    fn sm11_a_plan_checks_its_bounds() {
        assert_eq!(sample().check(), Ok(()));
        let bad = |edit: &dyn Fn(&mut FillPlan)| {
            let mut plan = sample();
            edit(&mut plan);
            plan.check()
        };
        assert!(bad(&|p| p.list = (0..17).map(|n| format!("A{n}")).collect()).is_err());
        assert!(bad(&|p| p.list.clear()).is_err());
        assert!(bad(&|p| p.steps = vec![PlanStep::Sound { field: 0 }; 65]).is_err());
        assert!(bad(&|p| p.steps = vec![PlanStep::Sound { field: 64 }]).is_err());
        assert!(bad(&|p| p.steps = vec![PlanStep::Mine { pocket: [64, 0] }]).is_err());
        assert!(bad(&|p| p.steps = vec![PlanStep::Block { letter: 'Q' }]).is_err());
        assert!(bad(&|p| p.cursor = 4).is_err());
        assert!(
            bad(&|p| p.passed = vec![3]).is_err(),
            "passed under a mine step"
        );
        let mut on_block = sample();
        on_block.cursor = 2;
        on_block.passed = vec![3, 9];
        assert_eq!(on_block.check(), Ok(()));
        on_block.passed = vec![3, 3];
        assert!(on_block.check().is_err(), "distinct");
        assert!(bad(&|p| p.deadline_ic += 1).is_err());
        assert!(bad(&|p| p.slot = 0).is_err());
        assert!(bad(&|p| p.seq = 0).is_err());
        let note = |seq: u32| PlanNote {
            seq,
            at_ic: 2_000,
            note: PlanNoteKind::Parking {
                why: PlanEnd::Claimed,
            },
        };
        assert!(
            bad(&|p| {
                p.seq = 5;
                p.notes = vec![note(3), note(2)];
            })
            .is_err(),
            "a note out of order"
        );
        assert!(
            bad(&|p| p.notes = vec![note(2)]).is_err(),
            "a note past the seq"
        );
        assert!(
            bad(&|p| {
                p.state = FillState::Ended {
                    why: PlanEnd::Cleared,
                    at_ic: 999,
                };
            })
            .is_err(),
            "ended before it was filed"
        );
    }

    /// Every state, both `ActBy` kinds and the projection's view round-trip on the wire.
    #[test]
    fn sm11_the_plan_round_trips() {
        for state in [
            FillState::Live,
            FillState::Parking {
                why: PlanEnd::Licence {
                    why: LicenceEnd::Lapsed { at_ic: 5 },
                },
            },
            FillState::Ended {
                why: PlanEnd::HoldFull,
                at_ic: 9_000,
            },
        ] {
            let mut plan = sample();
            plan.state = state;
            plan.seq = 3;
            plan.notes = vec![PlanNote {
                seq: 2,
                at_ic: 2_000,
                note: PlanNoteKind::Passed {
                    at: [20, 52],
                    field: false,
                    why: PassWhy::TooFar,
                },
            }];
            let wire = serde_json::to_string(&plan).unwrap_or_else(|e| panic!("{e}"));
            let back: FillPlan = serde_json::from_str(&wire).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, plan);
        }
        for by in [ActBy::Plan { no: 2, step: 5 }, ActBy::Idle] {
            let wire = serde_json::to_string(&by).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(
                serde_json::from_str::<ActBy>(&wire).unwrap_or_else(|e| panic!("{e}")),
                by
            );
        }
        assert_eq!(
            serde_json::to_string(&ActBy::Idle).unwrap_or_default(),
            r#"{"by":"idle"}"#
        );
        let view = plan_view(&sample());
        assert_eq!(
            view.next,
            vec!["mine C117", "sound D-SW", "mine block D, fair or better"]
        );
        let wire = serde_json::to_string(&view).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            serde_json::from_str::<crate::surface::excursion::PlanView>(&wire)
                .unwrap_or_else(|e| panic!("{e}")),
            view
        );
    }

    /// A chart row with the given fields scanned (regolith) and pockets sounded, all by hand.
    fn row(fields: &[(i64, i64)], pockets: &[(i64, i64)]) -> ChartRecord {
        let mut record = ChartRecord::new(ObjId(7), cid("ahuna_port"), cid("ceres"));
        let indexes: Vec<u8> = fields
            .iter()
            .filter_map(|(fx, fy)| address::field_index(*fx, *fy))
            .collect();
        if !indexes.is_empty() {
            chart::write_scan(
                &mut record,
                [128, 124],
                &indexes,
                Footing::Regolith,
                10,
                Via::Hand,
            );
        }
        if !pockets.is_empty() {
            chart::write_sound(&mut record, [128, 124], pockets, false, 20, Via::Hand);
        }
        record
    }

    /// One input over a hand-built row and the truth the author's own readings render (the
    /// closures answer only where the row reads).
    struct Fixture {
        row: Option<ChartRecord>,
        classes: BTreeMap<(i64, i64), [Word; 4]>,
        fields: BTreeMap<(i64, i64), [FieldReading; 4]>,
        home: u32,
        far: BTreeMap<(i64, i64), u32>,
        hold: u64,
        licence: Result<i64, LicenceEnd>,
        lease: Result<(), LeaseBar>,
        claimed: Option<char>,
    }

    impl Fixture {
        fn new(row: ChartRecord) -> Fixture {
            Fixture {
                row: Some(row),
                classes: BTreeMap::new(),
                fields: BTreeMap::new(),
                home: 1_000,
                far: BTreeMap::new(),
                hold: 60,
                licence: Ok(9_999_999),
                lease: Ok(()),
                claimed: None,
            }
        }

        fn compile(&self, list: &str, family: Option<OreFamily>) -> Result<Compiled, PlanRefusal> {
            let (scanner, drill) = tools();
            let battery = battery();
            let row = self.row.as_ref();
            let classes = |pocket: (i64, i64)| {
                chart::row_reads_pocket(row, pocket).then(|| {
                    self.classes
                        .get(&pocket)
                        .copied()
                        .unwrap_or([Word::Barren; 4])
                })
            };
            let fields = |field: (i64, i64)| {
                chart::row_reads_field(row, field).map(|_| {
                    self.fields
                        .get(&field)
                        .copied()
                        .unwrap_or([FieldReading::Exact(Word::Barren); 4])
                })
            };
            let claimed = self.claimed;
            let tenure = move |letter: char| {
                if Some(letter) == claimed {
                    Err(TenureBar::Claimed)
                } else {
                    Ok(())
                }
            };
            let home = self.home;
            let far = self.far.clone();
            let home_of = move |stand: (i64, i64)| Some(far.get(&stand).copied().unwrap_or(home));
            compile(&CompileInput {
                row,
                list,
                family,
                licence: self.licence,
                lease: self.lease,
                tenure: &tenure,
                classes: &classes,
                field_readings: &fields,
                start_cell: (128, 124),
                home: &home_of,
                battery: &battery,
                scanner: &scanner,
                drill: &drill,
                hold_room_t: self.hold,
                hopper_t: 12,
            })
        }
    }

    fn fair_metals() -> [Word; 4] {
        [Word::Fair, Word::Barren, Word::Barren, Word::Barren]
    }

    /// Unknown, uncovered and barren-unsounded addresses answer the same refusal; a sounded barren
    /// pocket answers `Barren` — the author's own reading.
    #[test]
    fn sm11_compile_names_only_your_own_knowledge() {
        let c117 = address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117"));
        let d1 = address::parse_pocket_address("D1").unwrap_or_else(|| panic!("D1"));
        // Unknown: no row of the site at all.
        let mut unknown = Fixture::new(row(&[], &[]));
        unknown.row = None;
        unknown.classes.insert(c117, fair_metals());
        // Uncovered: a row that reads elsewhere.
        let mut uncovered = Fixture::new(row(&[], &[c117]));
        uncovered.classes.insert(c117, fair_metals());
        // Barren, never sounded: the truth is barren, and the row does not read it.
        let mut barren = Fixture::new(row(&[], &[c117]));
        barren.classes.insert(d1, [Word::Barren; 4]);
        for (fixture, case) in [
            (&unknown, "unknown"),
            (&uncovered, "uncovered"),
            (&barren, "barren, unsounded"),
        ] {
            let list = if case == "unknown" { "C117" } else { "D1" };
            assert_eq!(
                fixture.compile(list, None),
                Err(PlanRefusal::NoReading {
                    addr: list.to_owned()
                }),
                "{case}"
            );
        }
        // Sounded and barren: the author's own reading, said back.
        let mut sounded = Fixture::new(row(&[], &[c117]));
        sounded.classes.insert(c117, [Word::Barren; 4]);
        assert_eq!(
            sounded.compile("C117", None),
            Err(PlanRefusal::Barren {
                addr: "C117".to_owned(),
                family: OreFamily::Metals
            }),
            "sounded barren"
        );
        assert_eq!(
            sounded.compile("C1X7", None),
            Err(PlanRefusal::Usage),
            "not an address"
        );
    }

    /// The rights come before the knowledge.
    #[test]
    fn sm11_compile_reads_rights_before_knowledge() {
        let mut fixture = Fixture::new(row(&[], &[]));
        fixture.licence = Err(LicenceEnd::Unlicensed);
        assert_eq!(
            fixture.compile("C117", None),
            Err(PlanRefusal::Licence {
                why: LicenceEnd::Unlicensed
            })
        );
        fixture.licence = Err(LicenceEnd::Lapsed { at_ic: 5 });
        assert_eq!(
            fixture.compile("C117", None),
            Err(PlanRefusal::Licence {
                why: LicenceEnd::Lapsed { at_ic: 5 }
            })
        );
        fixture.licence = Ok(9_999);
        fixture.lease = Err(LeaseBar::Forfeit);
        assert_eq!(
            fixture.compile("C117", None),
            Err(PlanRefusal::Lease {
                bar: LeaseBar::Forfeit
            })
        );
        let c117 = address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117"));
        let mut claimed = Fixture::new(row(&[], &[c117]));
        claimed.classes.insert(c117, fair_metals());
        claimed.claimed = Some('C');
        assert_eq!(
            claimed.compile("C117", None),
            Err(PlanRefusal::Claimed {
                addr: "C117".to_owned()
            })
        );
    }

    /// A block's four soundings, nearest first from the start cell, a fully sounded field
    /// skipped, then the block step.
    #[test]
    fn sm11_compile_expands_a_block_nearest_first() {
        // Block K is bx 2, by 1 (fields (4..6, 2..4)).
        let fields = block_fields('K');
        let mut fixture = Fixture::new(row(&fields, &field_pockets(fields[3])));
        for field in &fields {
            fixture
                .fields
                .insert(*field, [FieldReading::Exact(Word::Fair); 4]);
        }
        let compiled = fixture
            .compile("K", Some(OreFamily::Metals))
            .unwrap_or_else(|e| panic!("{e:?}"));
        let start = (128, 124);
        let mut expected: Vec<((i64, i64), u8)> = fields
            .iter()
            .take(3)
            .map(|field| (*field, address::field_index(field.0, field.1).unwrap_or(0)))
            .collect();
        expected.sort_by_key(|(field, index)| (cells_apart(start, field_middle(*field)), *index));
        let mut want: Vec<PlanStep> = expected
            .into_iter()
            .map(|(_, index)| PlanStep::Sound { field: index })
            .collect();
        want.push(PlanStep::Block { letter: 'K' });
        assert_eq!(compiled.steps, want);
        assert_eq!(compiled.list, vec!["K"]);
    }

    /// An unsounded pocket's field is sounded first, once a field however many pockets it holds.
    #[test]
    fn sm11_compile_sounds_a_field_before_an_unsounded_pocket() {
        let a = address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117"));
        let field = address::field_of_pocket(a).unwrap_or_else(|| panic!("a field"));
        let pockets = field_pockets(field);
        let b = pockets
            .iter()
            .copied()
            .find(|p| *p != a)
            .unwrap_or_else(|| panic!("a second pocket"));
        let b_addr = address::pocket_address(b.0, b.1).unwrap_or_default();
        let a_addr = address::pocket_address(a.0, a.1).unwrap_or_default();
        let mut fixture = Fixture::new(row(&[field], &[]));
        fixture
            .fields
            .insert(field, [FieldReading::Exact(Word::Fair); 4]);
        let compiled = fixture
            .compile(&format!("{a_addr}, {b_addr}"), Some(OreFamily::Metals))
            .unwrap_or_else(|e| panic!("{e:?}"));
        let index = address::field_index(field.0, field.1).unwrap_or(0);
        assert_eq!(
            compiled.steps,
            vec![
                PlanStep::Sound { field: index },
                PlanStep::Mine { pocket: [a.0, a.1] },
                PlanStep::Mine { pocket: [b.0, b.1] },
            ]
        );
    }

    /// The family defaults to the first address's richest reading; `family=` overrides it.
    #[test]
    fn sm11_the_family_defaults_to_the_first_address_best_reading() {
        let c117 = address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117"));
        let mut fixture = Fixture::new(row(&[], &[c117]));
        fixture
            .classes
            .insert(c117, [Word::Lean, Word::Rich, Word::Barren, Word::Barren]);
        let compiled = fixture
            .compile("c117", None)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(compiled.family, OreFamily::Volatiles, "the richest class");
        assert_eq!(compiled.list, vec!["C117"], "upper-cased");
        let named = fixture
            .compile("C117", Some(OreFamily::Metals))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(named.family, OreFamily::Metals);
        // A tie goes to the families' order.
        fixture
            .classes
            .insert(c117, [Word::Fair, Word::Fair, Word::Barren, Word::Barren]);
        assert_eq!(
            fixture.compile("C117", None).map(|c| c.family),
            Ok(OreFamily::Metals)
        );
    }

    /// A pocket whose `2 × home + one cycle` passes a full battery is too far; one inside files; a
    /// block with a far pocket still files, the far pocket counted; a full hold refuses.
    #[test]
    fn sm11_compile_refuses_too_far_and_a_full_hold() {
        let c117 = address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117"));
        let (_, drill) = tools();
        let b = battery();
        let floor = b.capacity_cu * b.reserve_permille / 1_000;
        let mut fixture = Fixture::new(row(&[], &[c117]));
        fixture.classes.insert(c117, fair_metals());
        // 2h + e ≤ capacity − floor files; one more refuses.
        let fits = (b.capacity_cu - floor - drill.energy_cu) / 2;
        fixture.home = fits;
        assert!(fixture.compile("C117", None).is_ok());
        fixture.home = fits + 1;
        assert_eq!(
            fixture.compile("C117", None),
            Err(PlanRefusal::TooFar {
                addr: "C117".to_owned()
            })
        );
        // A block with one far pocket the author reads fair still files, the pocket counted.
        fixture.home = 1_000;
        fixture.far.insert(pocket_cell(c117), fits + 1);
        let letter = address::block_letter(c117.0, c117.1).unwrap_or('C');
        let compiled = fixture
            .compile(&letter.to_string(), Some(OreFamily::Metals))
            .unwrap_or_else(|e| panic!("{e:?}"));
        let block_at = compiled
            .steps
            .iter()
            .position(|step| matches!(step, PlanStep::Block { .. }))
            .and_then(|at| u16::try_from(at).ok())
            .unwrap_or_else(|| panic!("a block step"));
        assert_eq!(compiled.far, vec![(block_at, 1)], "{compiled:?}");
        fixture.far.clear();
        fixture.hold = u64::from(blind_room_t(&drill)) - 1;
        assert_eq!(fixture.compile("C117", None), Err(PlanRefusal::HoldFull));
    }

    /// The family is fixed before the barren check (the reviewer's F13).
    #[test]
    fn sm11_the_family_is_fixed_before_barren() {
        let a = address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117"));
        let b = address::parse_pocket_address("C118").unwrap_or_else(|| panic!("C118"));
        let mut fixture = Fixture::new(row(&[], &[a, b]));
        fixture
            .classes
            .insert(a, [Word::Lean, Word::Rich, Word::Barren, Word::Barren]);
        fixture
            .classes
            .insert(b, [Word::Fair, Word::Barren, Word::Barren, Word::Barren]);
        assert_eq!(
            fixture.compile("C117,C118", None),
            Err(PlanRefusal::Barren {
                addr: "C118".to_owned(),
                family: OreFamily::Volatiles
            })
        );
        assert!(
            fixture
                .compile("C117,C118", Some(OreFamily::Metals))
                .is_ok()
        );
    }

    /// The estimate: a fair pocket on regolith is three 1,800 s cycles; rock doubles; a block adds
    /// only its soundings; trips add a recover and an unload per hopper.
    #[test]
    fn sm11_the_estimate_is_a_lower_bound() {
        let (scanner, drill) = tools();
        let c117 = address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117"));
        let mut fixture = Fixture::new(row(&[], &[c117]));
        fixture.classes.insert(c117, fair_metals());
        let compiled = fixture
            .compile("C117", None)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let cycle = u64::from(drill.cycle_s);
        assert_eq!(compiled.step_secs, vec![3 * cycle]);
        assert_eq!(
            compiled.est_secs,
            3 * cycle + u64::from(RECOVER_S + UNLOAD_S),
            "one trip"
        );
        // Rock doubles the cycle.
        let field = address::field_of_pocket(c117).unwrap_or_else(|| panic!("a field"));
        let Some(mut rock) = fixture.row.clone() else {
            panic!("a row");
        };
        if let Some(index) = address::field_index(field.0, field.1) {
            chart::write_scan(&mut rock, [10, 10], &[index], Footing::Rock, 30, Via::Hand);
        }
        let mut on_rock = Fixture::new(rock);
        on_rock.classes.insert(c117, fair_metals());
        let compiled = on_rock
            .compile("C117", None)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            compiled.step_secs,
            vec![3 * cycle * u64::from(drill.rock_factor)]
        );
        // A block: only its soundings.
        let fields = block_fields('K');
        let mut block = Fixture::new(row(&fields, &[]));
        for field in &fields {
            block
                .fields
                .insert(*field, [FieldReading::Exact(Word::Fair); 4]);
        }
        let compiled = block
            .compile("K", Some(OreFamily::Metals))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(compiled.est_secs, 4 * u64::from(scanner.sound_s));
    }

    /// Seventeen addresses, a twice-named pocket, a pocket inside a named block, a field name and
    /// an empty list each refuse.
    #[test]
    fn sm11_compile_bounds_the_list() {
        let c117 = address::parse_pocket_address("C117").unwrap_or_else(|| panic!("C117"));
        let mut fixture = Fixture::new(row(&[], &[c117]));
        fixture.classes.insert(c117, fair_metals());
        let seventeen: Vec<String> = (1..=17).map(|n| format!("A{n}")).collect();
        assert_eq!(
            fixture.compile(&seventeen.join(","), None),
            Err(PlanRefusal::TooMany)
        );
        assert_eq!(
            fixture.compile("C117 c117", None),
            Err(PlanRefusal::Twice {
                addr: "C117".to_owned()
            })
        );
        let letter = address::block_letter(c117.0, c117.1).unwrap_or('C');
        assert_eq!(
            fixture.compile(&format!("{letter}, C117"), None),
            Err(PlanRefusal::Twice {
                addr: "C117".to_owned()
            })
        );
        assert_eq!(fixture.compile("C-NW", None), Err(PlanRefusal::Usage));
        assert_eq!(fixture.compile(" , ", None), Err(PlanRefusal::Usage));
        // Past 64 steps: sixteen blocks, each scanned and never sounded, compile to four
        // soundings and a block step apiece — the thirteenth passes the cap.
        let every: Vec<(i64, i64)> = (0..64_u8).filter_map(address::field_of_index).collect();
        let mut wide = Fixture::new(row(&every, &[]));
        for field in &every {
            wide.fields
                .insert(*field, [FieldReading::Exact(Word::Fair); 4]);
        }
        let letters: Vec<String> = ('A'..='P').map(|letter| letter.to_string()).collect();
        assert_eq!(
            wide.compile(&letters.join(" "), Some(OreFamily::Metals)),
            Err(PlanRefusal::TooManySteps)
        );
        assert!(
            wide.compile(&letters[..12].join(" "), Some(OreFamily::Metals))
                .is_ok_and(|compiled| compiled.steps.len() == 60)
        );
    }

    /// The charging instant, exact at the battery's rate: the bay credited to the launch half.
    #[test]
    fn sm11_the_charging_wait_ends_when_the_charge_suffices() {
        let b = battery();
        let charge = crate::surface::excursion::Charge {
            value_cu: 10_000,
            at_ic: 1_000,
            in_bay: true,
        };
        let required = 60_000_u64;
        let until =
            charging_until(&charge, 1_000, required, &b).unwrap_or_else(|| panic!("it charges"));
        // 50,000 cu short at 50,000 cu an IC hour: 3,600 s of bay, less the launch half's 300.
        assert_eq!(until, 1_000 + 3_600 - 300);
        assert!(u64::from(charge_for_act(&charge, until, true, &b)) >= required);
        assert!(u64::from(charge_for_act(&charge, until - 1, true, &b)) < required);
        assert_eq!(
            charging_until(&charge, 1_000, 100_001, &b),
            None,
            "past capacity"
        );
        assert_eq!(
            charging_until(&charge, 1_000, 10_000, &b),
            Some(1_000),
            "already enough"
        );
        let out = crate::surface::excursion::Charge {
            in_bay: false,
            ..charge
        };
        assert_eq!(charging_until(&out, 1_000, required, &b), None);
    }

    /// A short charge never loops: the reserve rule over full and low charges, as `decide`'s
    /// step 5(f) reads it — from a full bay a stand whose leg + act + home passes the battery is
    /// too far (no drive), out with a low charge the rig recovers, and after the recover the same
    /// stand from a full charge is again too far, never another drive.
    #[test]
    fn sm11_a_short_charge_never_loops() {
        let b = battery();
        let (_, drill) = tools();
        let floor = u64::from(b.capacity_cu * b.reserve_permille / 1_000);
        let home = 47_000_u32;
        let act = drill.energy_cu;
        let short = |charge: u32, leg: u32| reserve_check(charge, 0, leg + act, home, &b).is_err();
        // From a full bay, a stand whose leg + act + home passes the battery: too far, no drive.
        assert!(short(b.capacity_cu, home));
        assert_eq!(on_short(true, true, None), OnShort::Pass);
        // Out with a low charge: the move is a recover, never a drive.
        assert!(short(20_000, 0));
        assert_eq!(on_short(false, false, None), OnShort::Recover);
        assert_eq!(on_short(false, true, Some(5)), OnShort::Recover);
        // Back in the bay at full charge the same stand is too far again — never another drive.
        assert_eq!(on_short(true, true, Some(5)), OnShort::Pass);
        // Below full in the bay: wait for the charge when it would ever suffice, else too far.
        let charge = crate::surface::excursion::Charge {
            value_cu: 20_000,
            at_ic: 0,
            in_bay: true,
        };
        let need = floor + u64::from(home) * 2 + u64::from(act);
        let until = charging_until(&charge, 0, need, &b);
        assert_eq!(until, None, "never: the stand is too far from any charge");
        assert_eq!(on_short(true, false, until), OnShort::Pass);
        let near = 10_000_u32;
        let need = floor + u64::from(near) * 2 + u64::from(act);
        let until = charging_until(&charge, 0, need, &b);
        assert!(until.is_some_and(|at| at > 0));
        assert_eq!(
            on_short(true, false, until),
            OnShort::Charge(until.unwrap_or(0))
        );
    }

    /// Every refusal word has a fate, row for row (the reviewer's F2): the table below is the
    /// brief's, iterated over the start's own word list, so a later cut's new word fails here
    /// until it is given a row.
    #[test]
    fn sm11_every_refusal_has_a_fate() {
        use crate::surface::act::{refusal_samples, refusal_words};
        let fates: [(&str, Fate); 64] = [
            ("tv_dark", Fate::Wait(PlanWait::Dark)),
            ("tv_unwired", Fate::Wait(PlanWait::Dark)),
            ("not_aboard", Fate::Wait(PlanWait::Ashore)),
            ("not_site", Fate::Wait(PlanWait::Dark)),
            ("under_way", Fate::Wait(PlanWait::Dark)),
            ("cleared", Fate::Wait(PlanWait::Dark)),
            ("out", Fate::Unreachable),
            ("open", Fate::Unreachable),
            ("forfeit", Fate::Park(PlanEnd::Lease { forfeit: true })),
            ("ended", Fate::Park(PlanEnd::Lease { forfeit: false })),
            ("in_bay", Fate::Unreachable),
            ("elsewhere", Fate::Unreachable),
            ("far", Fate::Stranded),
            ("dump_not_site", Fate::Unreachable),
            ("no_spoil", Fate::Unreachable),
            ("empty", Fate::Unreachable),
            ("busy", Fate::Wait(PlanWait::Busy)),
            ("reserve", Fate::Short),
            ("cap", Fate::Wait(PlanWait::Cap)),
            ("nothing_new", Fate::Pass(PassWhy::NothingNew)),
            ("chart_full", Fate::Park(PlanEnd::ChartFull)),
            ("ground_cold", Fate::Wait(PlanWait::Cold)),
            ("instruments_dark", Fate::Wait(PlanWait::Dark)),
            ("no_footing", Fate::Pass(PassWhy::NoWay)),
            ("off_site", Fate::Pass(PassWhy::NoWay)),
            ("here", Fate::Pass(PassWhy::NoWay)),
            ("no_path", Fate::Pass(PassWhy::NoWay)),
            ("winding", Fate::Pass(PassWhy::NoWay)),
            ("no_way_home", Fate::Pass(PassWhy::NoWay)),
            ("unplanned", Fate::Unreachable),
            ("moved", Fate::Unreachable),
            ("leg_short", Fate::Short),
            ("recover_dark", Fate::Wait(PlanWait::Dark)),
            ("mine_in_bay", Fate::Unreachable),
            ("mine_running", Fate::Wait(PlanWait::Busy)),
            ("mine_no_footing", Fate::Pass(PassWhy::NoWay)),
            ("unlicensed", Fate::Licence),
            ("claimed", Fate::Park(PlanEnd::Claimed)),
            ("locked", Fate::Pass(PassWhy::Locked)),
            ("hopper_full", Fate::Recover),
            ("hold_full", Fate::Park(PlanEnd::HoldFull)),
            ("worked_out", Fate::Pass(PassWhy::WorkedOut)),
            ("no_fit", Fate::Room),
            ("blind_room", Fate::Room),
            ("blind_count", Fate::Unreachable),
            ("no_ore", Fate::Pass(PassWhy::Barren)),
            ("mine_forfeit", Fate::Park(PlanEnd::Lease { forfeit: true })),
            ("mine_ended", Fate::Park(PlanEnd::Lease { forfeit: false })),
            ("mine_changed", Fate::Unreachable),
            ("unload_under_way", Fate::Unreachable),
            ("unload_not_site", Fate::Unreachable),
            ("unload_out", Fate::Unreachable),
            ("unload_no_room", Fate::Unreachable),
            ("unload_no_fit", Fate::Unreachable),
            ("unload_no_stock", Fate::Unreachable),
            ("unload_changed", Fate::Unreachable),
            ("unload_markers", Fate::Unreachable),
            ("plan_live", Fate::Unreachable),
            ("plan_filed", Fate::Unreachable),
            ("plan_changed", Fate::Unreachable),
            ("plan_waits", Fate::Unreachable),
            ("plan_moved", Fate::Unreachable),
            ("plan_none", Fate::Unreachable),
            ("plan_ended", Fate::Unreachable),
        ];
        let words = refusal_words();
        assert_eq!(words.len(), fates.len(), "a row for every word");
        for word in words {
            let Some((_, fate)) = fates.iter().find(|(w, _)| *w == word) else {
                panic!("no fate row for {word}");
            };
            let samples: Vec<Refusal> = refusal_samples()
                .into_iter()
                .filter(|refusal| refusal.word() == word)
                .collect();
            assert!(!samples.is_empty(), "a sample of {word}");
            for refusal in samples {
                assert_eq!(fate_of(&refusal), *fate, "{word}: {refusal:?}");
            }
        }
    }

    /// **The hold's room is read before any drive** (the reviewer's F9): `decide` runs
    /// [`hold_gate`] ahead of the leg — a hold below one cycle's need ends the plan from the bay
    /// with no start (no empty trip), and from the ground recovers first; one cycle's need fits.
    /// The world proof (no launch after the last unload) is `tests/plans.rs`'s P1 and P2.
    #[test]
    fn sm11_the_hold_check_comes_before_the_drive() {
        let (_, drill) = tools();
        let need = blind_room_t(&drill);
        assert_eq!(
            hold_gate(true, u64::from(need) - 1, need),
            Some(HoldGate::EndHere)
        );
        assert_eq!(
            hold_gate(false, u64::from(need) - 1, need),
            Some(HoldGate::RecoverThenEnd)
        );
        assert_eq!(hold_gate(true, u64::from(need), need), None);
    }

    /// **A short hopper in the bay reads its last unload** (the fixer round's correctness/F1): one
    /// that left ore behind (goods staying, or refused for the hold) ends the plan `HoldFull`; no
    /// unload since — ore left by hand, a done unload that moved everything — goes on (its recover
    /// queues the unload); and, the lead's pre-commit round item 1, an unload refused for its
    /// marker ring goes on too: a queued one's refusal keeps its queue (the plan waits on it before
    /// this rule is read, [`queue_waits_on_markers`]), so only a hand one's reaches here, and the
    /// step's recover files the queue that waits on the ring.
    #[test]
    fn sm11_a_short_hopper_in_the_bay_reads_its_last_unload() {
        use crate::surface::excursion::{ActKind, ActResult, Outcome, RefusedCode};
        let outcome = |kind: ActKind, result: ActResult, stays: u64| Outcome {
            ordinal: 9,
            kind,
            operator: ObjId(7),
            slot: 1,
            site: cid("ahuna_port"),
            at_ic: 5_000,
            result,
            moved: BTreeMap::new(),
            span: None,
            arrived: None,
            mined: None,
            stays: if stays > 0 {
                BTreeMap::from([(cid("metal_ore"), stays)])
            } else {
                BTreeMap::new()
            },
            by: None,
        };
        let refused = |code| ActResult::Refused { code };
        assert_eq!(bay_hopper(None), BayHopper::Go, "no outcome");
        assert_eq!(
            bay_hopper(Some(&outcome(ActKind::Recover, ActResult::Done, 0))),
            BayHopper::Go,
            "a recover's own outcome: the unload has not been tried"
        );
        assert_eq!(
            bay_hopper(Some(&outcome(
                ActKind::Unload,
                refused(RefusedCode::MarkersFull),
                0
            ))),
            BayHopper::Go,
            "a full marker ring ages away: never the plan's end, never a dead wait"
        );
        assert_eq!(
            bay_hopper(Some(&outcome(
                ActKind::Unload,
                refused(RefusedCode::HoldRefused),
                0
            ))),
            BayHopper::HoldFull
        );
        assert_eq!(
            bay_hopper(Some(&outcome(ActKind::Unload, ActResult::Done, 4))),
            BayHopper::HoldFull,
            "goods stayed: the hold took what fit"
        );
        assert_eq!(
            bay_hopper(Some(&outcome(ActKind::Unload, ActResult::Done, 0))),
            BayHopper::Go
        );
    }

    /// **A queue re-filed after a full marker ring is the `Markers` wait** (the lead's pre-commit
    /// round, item 1): an unload refused `MarkersFull` with the queue filed after it reads true; the
    /// queue a recover filed (after the recover), a queue after another refusal, an outcome with no
    /// queue and a queue with no outcome read false.
    #[test]
    fn sm11_a_queue_refiled_after_a_full_ring_waits_on_markers() {
        use crate::surface::excursion::{
            ActKind, ActResult, Outcome, Queued, RefusedCode, RigRecord,
        };
        let outcome = |kind: ActKind, result: ActResult, ordinal: u32| Outcome {
            ordinal,
            kind,
            operator: ObjId(7),
            slot: 1,
            site: cid("ahuna_port"),
            at_ic: 5_000,
            result,
            moved: BTreeMap::new(),
            span: None,
            arrived: None,
            mined: None,
            stays: BTreeMap::new(),
            by: None,
        };
        let queued = |after: u32| Queued {
            kind: ActKind::Unload,
            operator: ObjId(7),
            hull: ObjId(8),
            slot: 1,
            site: cid("ahuna_port"),
            after,
            at_ic: 5_000,
            excursion: None,
        };
        let markers = ActResult::Refused {
            code: RefusedCode::MarkersFull,
        };
        let mut record = RigRecord::fresh_unread(ObjId(900));
        record.ordinal = 6;
        record.last = Some(outcome(ActKind::Unload, markers, 6));
        record.queued = Some(queued(6));
        assert!(queue_waits_on_markers(&record));
        record.queued = Some(queued(5));
        assert!(
            !queue_waits_on_markers(&record),
            "a queue filed before the refusal"
        );
        record.queued = None;
        assert!(!queue_waits_on_markers(&record), "no queue");
        record.queued = Some(queued(6));
        record.last = Some(outcome(
            ActKind::Unload,
            ActResult::Refused {
                code: RefusedCode::HoldRefused,
            },
            6,
        ));
        assert!(!queue_waits_on_markers(&record), "another refusal");
        record.last = Some(outcome(ActKind::Recover, ActResult::Done, 6));
        assert!(!queue_waits_on_markers(&record), "a recover's own queue");
        record.last = None;
        assert!(!queue_waits_on_markers(&record), "no outcome");
    }

    /// **The projection's view of a parking or ended plan names no step it skips** (the fixer
    /// round's correctness/F4): the step clamped to the count, `next` empty; a live plan's view
    /// names its next steps from the cursor.
    #[test]
    fn sm11_a_parking_view_names_no_step() {
        let live = plan_view(&sample());
        assert_eq!((live.step, live.steps), (1, 3));
        assert_eq!(live.next.len(), 3);
        let mut parking = sample();
        parking.cursor = 1;
        parking.state = FillState::Parking {
            why: PlanEnd::HoldFull,
        };
        let view = plan_view(&parking);
        assert_eq!((view.step, view.steps), (2, 3));
        assert!(view.next.is_empty(), "{view:?}");
        assert_eq!(view.state, "parking");
        parking.cursor = 3;
        parking.state = FillState::Parking {
            why: PlanEnd::WorkedOut,
        };
        assert_eq!(plan_view(&parking).step, 3, "never step 4 of 3");
        let mut ended = sample();
        ended.cursor = 3;
        ended.state = FillState::Ended {
            why: PlanEnd::WorkedOut,
            at_ic: 2_000,
        };
        let view = plan_view(&ended);
        assert_eq!(view.step, 3);
        assert!(view.next.is_empty());
    }

    /// **A plan's view is its author's own** (the fixer round's conformance/F1): the next lessee
    /// (or nobody) sees none of it.
    #[test]
    fn sm11_a_view_is_its_authors_own() {
        let plan = sample();
        assert_eq!(
            plan_view_for(Some(&plan), Some(plan.author)),
            Some(plan_view(&plan))
        );
        assert_eq!(plan_view_for(Some(&plan), Some(ObjId(99))), None);
        assert_eq!(plan_view_for(Some(&plan), None), None);
        assert_eq!(plan_view_for(None, Some(plan.author)), None);
    }

    /// **An orphaned queue is the author's own with the author off the plan's hull's helm** (the
    /// fixer round's correctness/F3): another's queue, or the author at the helm, is not.
    #[test]
    fn sm11_an_orphaned_queue_is_the_authors_off_the_helm() {
        let plan = sample();
        let queued = |operator: ObjId| crate::surface::excursion::Queued {
            kind: crate::surface::excursion::ActKind::Unload,
            operator,
            hull: plan.hull,
            slot: 1,
            site: cid("ahuna_port"),
            after: 3,
            at_ic: 1_500,
            excursion: None,
        };
        assert!(queue_orphaned(&queued(plan.author), &plan, None));
        assert!(queue_orphaned(&queued(plan.author), &plan, Some(ObjId(55))));
        assert!(!queue_orphaned(
            &queued(plan.author),
            &plan,
            Some(plan.hull)
        ));
        assert!(!queue_orphaned(&queued(ObjId(99)), &plan, None));
    }
}
