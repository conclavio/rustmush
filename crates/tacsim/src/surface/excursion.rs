//! **The excursion and its projection** (doc 20 cut SM.3b; §4.1 "charge and the reserve
//! rule", §4.3 "`ExcursionId`", §8 "The clock", D-20-6, D-20-7, D-20-11; the SM.3b brief's
//! lead rulings 1, 2 and 4).
//!
//! Two records on two lanes (lead ruling 1):
//!
//! - **`tacsim.excursion`** — the typed, authoritative family: one
//!   [`RigRecord`] per rig, keyed by the terrain
//!   vehicle ([`RigKey`]). It holds the rig's act ordinal,
//!   its charge (lazy: a value, the instant it was true and whether it was in the bay), the
//!   act in flight with the charge it reserved, the open excursion and the last finished
//!   act. It is written **only** in a prepare: the start (`tacsim_surface_act`) and the
//!   finish (`tacsim_surface_done`), both in `crate::surface::act`. The reserve rule (§4.1)
//!   is enforced in the start's prepare and nowhere else, through
//!   [`reserve_check`].
//! - **`tacsim:tv:<tv>`** — the normal-lane projection
//!   ([`TvRec`], the `JourneyRec` shape, at most 4 KiB, a
//!   sixteen-line log), written only by the `tacsim.surface` tick in its own outbox batch
//!   beside the placement (`crate::surface::tick`). **It decides nothing**: every decision
//!   reads the typed family. Every stamp in it is an IC instant (never `World::now`), so a
//!   restart's post-image equals an uninterrupted run's (§8; the SM.3b acceptance A4).
//!
//! The pure functions here are the arithmetic every reader shares:
//! [`charge_at`] (the lazy charge, §4.1),
//! [`reserve_check`] (the 5 % reserve, a zero
//! spend exempt), [`cap_refusal`] (§8's caps),
//! [`ramp_pose`] (the hull's ramp as a site pose, the
//! brief's decision 3) and the site grid's
//! [`site_cell`] /
//! [`cells_apart`]. Integers throughout (§16); no RNG.
//!
//! **Doc 20 cut SM.4b** adds the drive: [`ActKind::Drive`] and [`ActParams::Drive`] — the frozen
//! leg, the one leg variant a far recover carries too — [`Outcome::arrived`], the leg in
//! [`TvRec::leg`] (fitted under 4 KiB by [`TvRec::fit`]), [`leg_affordable`] (a recover's way
//! home, never held to the floor) and [`cell_pose`].
//!
//! **Doc 20 cut SM.8** adds the unload: [`ActKind::Unload`] and [`ActParams::Unload`] (the goods
//! frozen at the start, [`UnloadHow`], the excursion the ore came home from), [`UNLOAD_S`], the
//! record's durable queue [`Queued`] (`RigRecord::queued`, written by a `Done` recover with ore in
//! the hopper and cleared by any start), [`Outcome::stays`] and the refusal codes
//! [`RefusedCode::LeftBay`], [`RefusedCode::HoldRefused`] and [`RefusedCode::MarkersFull`].
//!
//! **Doc 20 cut SM.11** adds the fill plan (§8 "Leaving it running"; D-20-11, Q-20-3): the rig's
//! durable plan ([`RigRecord::plan`], a [`FillPlan`] — the record type is
//! `crate::surface::plan`'s) and its count ([`RigRecord::plans`]); who set an act going
//! ([`Act::by`], [`Run::by`], [`Outcome::by`] — a plan's step or the idle return, never a player's
//! word); the projection's plan view ([`TvRec::plan`], [`PlanView`], [`PlanTold`]); and
//! [`TvState::Parked`]'s meaning, a rig on the ground whose charge cannot pay its way home.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::definitions::logistics::{BayDef, MAX_BAY_ITEMS};
use crate::definitions::tools::BatteryDef;
use crate::surface::ground::SITE_CELL_CM;
use crate::surface::plan::{ActBy, FillPlan, FillState};
use crate::surface::reading::Footing;

/// The typed family (§8; lead ruling 1).
pub const EXCURSION_FAMILY: &str = "tacsim.excursion";
/// [`RigRecord::schema`].
pub const EXCURSION_SCHEMA: u16 = 1;
/// The projection's document prefix (`tacsim:tv:{tv}`); never a prefix of
/// `crate::port::tv::TV_LEASE_PREFIX`'s keys (they differ at the colon).
pub const TV_REC_PREFIX: &str = "tacsim:tv:";
/// [`TvRec::schema`].
pub const TV_REC_SCHEMA: u16 = 1;
/// A launch down the ramp, IC seconds (§1 beat 4: "300 IC s").
pub const LAUNCH_S: u32 = 300;
/// A recover up the ramp, IC seconds (the brief's decision 9: doc 20 gives no figure for
/// the placement half; the launch's mirror).
pub const RECOVER_S: u32 = 300;
/// A dump, IC seconds (decision 10): a zero-length act finished on the next fire, so every
/// act has one start, one finish and one completion line.
pub const DUMP_S: u32 = 0;
/// **An unload, IC seconds** (doc 20 cut SM.8; the brief's lead ruling 2, decision R13): the
/// hopper into the hold from the bay. Doc 20 §4.1's basis — "a 600 IC s unload regains 8,300
/// cu" (the battery row's own note says the same) — and §12.1's "home and unload (a minute)":
/// 25 real seconds at rate 24. A code constant, as [`RECOVER_S`]: no tools or battery row carries
/// it, so no content digest moves.
pub const UNLOAD_S: u32 = 600;
/// The most excursions one operator runs (§8, §11 Caps: the Compact's span-of-control
/// lever).
pub const MAX_EXCURSIONS_PER_OPERATOR: u32 = 4;
/// The most excursions one site holds (§8, §11 Caps). **Doc 32 cut CF.1** (Q-CF-21): lowered
/// from 32 to the field cap, so a promotion can always admit every rig on the ground.
pub const MAX_EXCURSIONS_PER_SITE: u32 = crate::surface::site_record::FIELD_MAX_UNITS;
/// The most excursions the world holds (§8, §11 Caps).
pub const MAX_EXCURSIONS_WORLD: u32 = 1_024;
/// The most goods one dump freezes (a hopper accepts the four ores).
pub const MAX_DUMP_GOODS: usize = 8;
/// The projection's log ring (§8: "a log ring of sixteen lines").
pub const MAX_TV_LOG_LINES: usize = 16;
/// The most bytes one log line keeps (cut on a char boundary).
pub const MAX_TV_LOG_BYTES: usize = 160;
/// The projection's ceiling, canonical JSON bytes (§8: "at most 4 KiB").
pub const MAX_TV_REC_BYTES: usize = 4_096;
/// The most waypoints a frozen leg carries (§8; SM.4's slot).
pub const MAX_LEG_WAYPOINTS: usize = 32;
/// The pad's centre on a site's grid, centimetres on both axes: `SITE_CELLS / 2 ×
/// SITE_CELL_CM` (the site packs put `room.pad` at 1,260–1,300 m in a 2,560 m region whose
/// origin is the south-west corner).
pub const PAD_CENTRE_CM: i64 = 128_000;
/// IC seconds in an IC hour (the battery's recharge is per IC hour).
pub const IC_HOUR: i64 = 3_600;

// ---- the family's key and value ------------------------------------------------------

/// **The family key** (lead ruling 1): one record per rig, keyed by the terrain vehicle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RigKey(pub ObjId);

impl RigKey {
    /// The TV's id in decimal (`tv.0`, never `ObjId`'s `#`-prefixed Display).
    #[must_use]
    pub fn key_text(&self) -> String {
        self.0.0.to_string()
    }

    /// The strict inverse of [`RigKey::key_text`] (`crate::port::families`'s
    /// `strict_decimal_u32`: ASCII digits, no sign, no leading zero but `0`).
    #[must_use]
    pub fn parse(text: &str) -> Option<RigKey> {
        crate::port::families::strict_decimal_u32(text).map(|tv| RigKey(ObjId(tv)))
    }
}

/// **A rig's charge, lazily** (§4.1; lead ruling 4): `value_cu` was true at `at_ic`, with
/// the rig in the bay (`in_bay`) or out of it. Read through [`charge_at`]; only a finish
/// writes it, at its act's own stamp — nothing writes charge on a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Charge {
    /// The charge, cu.
    pub value_cu: u32,
    /// The IC instant it was that.
    pub at_ic: i64,
    /// Whether the rig stood in a bay's umbilical from then.
    pub in_bay: bool,
}

/// **What an act is** (§8's closed vocabulary as SM.3b builds it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActKind {
    /// Down the ramp onto the site (§1 beat 4).
    Launch,
    /// Back up the ramp into the bay (§7.1's placement half).
    Recover,
    /// The hopper emptied onto the site's spoil (§4.3).
    Dump,
    /// **A scan** (doc 20 cut SM.4a; §5's R1, D-20-8): the nine fields around the rig's own
    /// that the mast sees, sharpened by its footing. `survey` is the verb's alias only.
    Scan,
    /// **A sounding** (doc 20 cut SM.4a; §5's R2): every pocket within the sounding's reach,
    /// loud (heard across the site) or quiet (a quarter of the reach, heard by nobody) — the
    /// quiet flag is the act's parameter, not a kind ([`ActParams::Sound`]).
    Sound,
    /// **A drive** (doc 20 cut SM.4b; §8 "the frozen leg", D-20-11): over the site's ground along
    /// the leg frozen at the start ([`ActParams::Drive`]); from the bay the launch is implied.
    Drive,
    /// **A mining run** (doc 20 cut SM.7; §6, D-20-9; the SM.7 brief's lead ruling 1): started
    /// once on the one start action (`act = mine`), its cycles landing at their own dues through
    /// the one finish. **A run never occupies the act slot**: it lives in the record's cycle slot
    /// ([`RigRecord::run`]), so a `Mine` [`Act`] never exists ([`RigRecord::check`] refuses one);
    /// the kind names the run's outcomes ([`Outcome::mined`]) and its word.
    Mine,
    /// **An unload** (doc 20 cut SM.8; §7.1, D-20-10; the brief's lead ruling 1): the hopper's ore
    /// into the hull's hold from the bay ([`ActParams::Unload`]), by an inventory-owned marked
    /// transfer at the act's due. Filed by a `Done` recover with a loaded hopper (the record's
    /// [`RigRecord::queued`], started by the surface tick as the operator) or by `+tv/unload`.
    Unload,
}

impl ActKind {
    /// The act's word on the verb and in every line a player reads (`launch`, `recover`,
    /// `dump`, `scan`, `sound`). The start's `act` parameter spells a quiet sounding
    /// `sound_quiet` (`crate::surface::act::wire_word`).
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            ActKind::Launch => "launch",
            ActKind::Recover => "recover",
            ActKind::Dump => "dump",
            ActKind::Scan => "scan",
            ActKind::Sound => "sound",
            ActKind::Drive => "drive",
            ActKind::Mine => "mine",
            ActKind::Unload => "unload",
        }
    }

    /// The inverse of [`ActKind::word`].
    #[must_use]
    pub fn from_word(word: &str) -> Option<ActKind> {
        match word {
            "launch" => Some(ActKind::Launch),
            "recover" => Some(ActKind::Recover),
            "dump" => Some(ActKind::Dump),
            "scan" => Some(ActKind::Scan),
            "sound" => Some(ActKind::Sound),
            "drive" => Some(ActKind::Drive),
            "mine" => Some(ActKind::Mine),
            "unload" => Some(ActKind::Unload),
            _ => None,
        }
    }
}

/// **An act's frozen parameters** (§8: "the current act `{kind, params, …}`").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActParams {
    /// Launch and recover carry nothing (a struct variant, so an unknown field is refused
    /// on the wire as everywhere else).
    None {},
    /// A dump: the hopper and spoil vaults and the quantities frozen at the start
    /// (`1..=`[`MAX_DUMP_GOODS`] positive rows); the finish moves exactly these.
    Dump {
        /// The rig's hopper vault, `VaultId.0`.
        hopper: u64,
        /// The site's spoil vault, `VaultId.0`.
        spoil: u64,
        /// Good → the quantity the hopper held at the start.
        goods: BTreeMap<CanonicalId, u64>,
    },
    /// **A scan** (doc 20 cut SM.4a; lead ruling 6): everything it reads, frozen at the start
    /// so the finish needs no composite (cells are never edited, D-14-9).
    Scan {
        /// The launch is implied: the rig stood in the bay at plan time, and the scan reads
        /// from the ramp's cell once the tick has placed it there.
        from_bay: bool,
        /// The site's body (the chart row's).
        body: CanonicalId,
        /// The stand cell, site-local `[x, y]` from the south-west.
        cell: [i64; 2],
        /// The footing at the stand.
        footing: Footing,
        /// The fields the mast sees (site field indexes, `fy · 8 + fx`), 1..=9, the stand's
        /// own always among them.
        fields: Vec<u8>,
        /// The fields in reach the mast does not see, at most 8, disjoint from `fields`.
        hidden: Vec<u8>,
    },
    /// **A sounding** (doc 20 cut SM.4a): its stand and reach, frozen at the start.
    Sound {
        /// The launch is implied (as the scan's).
        from_bay: bool,
        /// The site's body.
        body: CanonicalId,
        /// The stand cell.
        cell: [i64; 2],
        /// The reach, cells per axis (16 loud, 8 quiet on the Mk1 mast), 1..=48.
        radius: u32,
        /// Quiet: heard by nobody.
        quiet: bool,
    },
    /// **A drive's frozen leg** (doc 20 cut SM.4b; §8; lead rulings 1 and 5): the one leg variant
    /// — a drive's, and a recover's from farther than beside the ramp. The start's prepare writes
    /// it; the tick projects its waypoints into `TvRec.leg` for interpolation and places the rig
    /// at `to` at the act's due instant; positions between are never written.
    Drive {
        /// The launch is implied (a drive typed from the bay; a recover's is always `false`).
        from_bay: bool,
        /// The target in words (`crate::surface::drive::target_words`, or "the {ramp}" for a
        /// recover), at most [`MAX_DRIVE_TARGET_BYTES`].
        target: String,
        /// The end cell, site-local `[x, y]` (a recover's: the beside-ramp cell its leg ends on).
        to: [i64; 2],
        /// The frozen display leg, `2..=`[`MAX_LEG_WAYPOINTS`], `at_s` from the leg's start,
        /// non-decreasing.
        waypoints: Vec<LegWaypoint>,
        /// The leg's own IC seconds (the last waypoint's `at_s`).
        secs: u32,
        /// The exact path's atlas cost (for the log and the tests).
        cost: u64,
        /// The exact path's length, metres.
        metres: u32,
        /// The heading at the end, millidegrees clockwise from north, `< 360,000`.
        heading_mdeg: u32,
        /// **The site's side the leg was planned over** (doc 32 cut S.0, §18), which `to` is
        /// held to, `1..=`[`crate::surface::ground::MAX_SITE_CELLS`]; omitted at the shipped
        /// side, so a 256 site's drive is byte-identical to SM.4b's.
        #[serde(default = "shipped_side", skip_serializing_if = "is_shipped_side")]
        side: u16,
    },
    /// **An unload** (doc 20 §7.1, cut SM.8): the hopper's goods into the hull's hold, frozen at
    /// the start; the finish moves exactly these by an inventory-owned marked transfer
    /// (`InventoryFamilies::transfer_many_marked`).
    Unload {
        /// The rig's hopper vault, `VaultId.0`.
        hopper: u64,
        /// The hull's hold vault, `VaultId.0`.
        hold: u64,
        /// Good → the quantity that moves: at most [`MAX_DUMP_GOODS`] positive rows; empty only
        /// for a [`UnloadHow::Queued`] unload the hold has no room for (a 0 s act, told once).
        goods: BTreeMap<CanonicalId, u64>,
        /// How it was filed.
        how: UnloadHow,
        /// The excursion the ore came home from (a queued unload's: the one its recover closed);
        /// part of each move's marker (`crate::surface::unload::marker`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        excursion: Option<ExcursionId>,
    },
}

/// **How an unload was filed** (doc 20 cut SM.8; the brief's decision R15): the start's `how`
/// parameter, which binds a start to the plan it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnloadHow {
    /// Filed by a `Done` recover ([`RigRecord::queued`]) and started by the surface tick.
    Queued,
    /// A bare `+tv/unload go`: every good, up to the hold's room.
    Whole,
    /// `+tv/unload <t> go`: a named tonnage of the first good.
    Part,
}

impl UnloadHow {
    /// The start's `how` word.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            UnloadHow::Queued => "queued",
            UnloadHow::Whole => "whole",
            UnloadHow::Part => "part",
        }
    }

    /// The inverse of [`UnloadHow::word`].
    #[must_use]
    pub fn from_word(word: &str) -> Option<UnloadHow> {
        match word {
            "queued" => Some(UnloadHow::Queued),
            "whole" => Some(UnloadHow::Whole),
            "part" => Some(UnloadHow::Part),
            _ => None,
        }
    }
}

/// The most bytes a drive's target words carry.
pub const MAX_DRIVE_TARGET_BYTES: usize = 32;

/// The most cells an instrument's reach may name (the scan's 48 on the Mk1 mast).
pub const MAX_READ_RADIUS_CELLS: u32 = 48;
/// The most fields a scan reads: the stand's own and its eight neighbours.
pub const MAX_SCAN_FIELDS: usize = 9;

impl ActParams {
    /// Whether the act's launch is implied — a scan's, a sounding's or (doc 20 cut SM.4b) a
    /// drive's from the bay; `false` for every other act (and for a recover's leg).
    #[must_use]
    pub fn launches(&self) -> bool {
        match self {
            ActParams::Scan { from_bay, .. }
            | ActParams::Sound { from_bay, .. }
            | ActParams::Drive { from_bay, .. } => *from_bay,
            ActParams::None {} | ActParams::Dump { .. } | ActParams::Unload { .. } => false,
        }
    }

    /// A scan's or a sounding's stand cell (a drive has no reading stand).
    #[must_use]
    pub fn stand(&self) -> Option<[i64; 2]> {
        match self {
            ActParams::Scan { cell, .. } | ActParams::Sound { cell, .. } => Some(*cell),
            ActParams::None {}
            | ActParams::Dump { .. }
            | ActParams::Drive { .. }
            | ActParams::Unload { .. } => None,
        }
    }

    /// **The frozen leg** (doc 20 cut SM.4b): a drive's or a recover's waypoints and end cell;
    /// `None` for every act without one.
    #[must_use]
    pub fn leg(&self) -> Option<(&[LegWaypoint], [i64; 2])> {
        match self {
            ActParams::Drive { waypoints, to, .. } => Some((waypoints.as_slice(), *to)),
            ActParams::None {}
            | ActParams::Dump { .. }
            | ActParams::Scan { .. }
            | ActParams::Sound { .. }
            | ActParams::Unload { .. } => None,
        }
    }

    /// **An act's own bounds** (doc 20 cut SM.4a): the stand on the site; a scan's `1..=9`
    /// distinct field indexes and at most 8 hidden ones, distinct and disjoint; a sounding's
    /// reach `1..=48`. **Doc 20 cut SM.4b**: a leg's `2..=`[`MAX_LEG_WAYPOINTS`] waypoints with
    /// non-decreasing `at_s`, the last equal to `secs`; `to` on the site; the target at most
    /// [`MAX_DRIVE_TARGET_BYTES`]; the heading under 360,000.
    pub(crate) fn check(&self) -> Result<(), String> {
        match self {
            ActParams::None {} | ActParams::Dump { .. } => Ok(()),
            // Doc 20 cut SM.8: at most `MAX_DUMP_GOODS` positive rows, empty only when queued.
            ActParams::Unload { goods, how, .. } => {
                if goods.len() > MAX_DUMP_GOODS
                    || goods.values().any(|qty| *qty == 0)
                    || (goods.is_empty() && *how != UnloadHow::Queued)
                {
                    return Err(format!(
                        "an unload moves at most {MAX_DUMP_GOODS} goods, each a positive \
                         quantity, and none only when queued"
                    ));
                }
                Ok(())
            }
            ActParams::Drive {
                target,
                to,
                waypoints,
                secs,
                heading_mdeg,
                side,
                ..
            } => {
                if !(1..=crate::surface::ground::MAX_SITE_CELLS).contains(side) {
                    return Err(format!(
                        "a leg's site is 1 to {} cells a side",
                        crate::surface::ground::MAX_SITE_CELLS
                    ));
                }
                if waypoints.len() < 2 || waypoints.len() > MAX_LEG_WAYPOINTS {
                    return Err(format!("a leg carries 2 to {MAX_LEG_WAYPOINTS} waypoints"));
                }
                if waypoints
                    .windows(2)
                    .any(|pair| match (pair.first(), pair.get(1)) {
                        (Some(a), Some(b)) => a.at_s > b.at_s,
                        _ => true,
                    })
                {
                    return Err("a leg's waypoints run forward in time".to_owned());
                }
                if waypoints.last().map(|last| last.at_s) != Some(*secs) {
                    return Err("a leg ends at its own seconds".to_owned());
                }
                if !crate::surface::address::cell_on_site((to[0], to[1]), *side) {
                    return Err("a leg ends on its site".to_owned());
                }
                if target.len() > MAX_DRIVE_TARGET_BYTES {
                    return Err(format!(
                        "a drive's target is at most {MAX_DRIVE_TARGET_BYTES} bytes"
                    ));
                }
                if *heading_mdeg >= 360_000 {
                    return Err("a leg's heading is under 360,000 millidegrees".to_owned());
                }
                Ok(())
            }
            ActParams::Scan {
                cell,
                fields,
                hidden,
                ..
            } => {
                if !stand_on_site(*cell) {
                    return Err("a scan stands on its site".to_owned());
                }
                check_fields(fields, hidden)
            }
            ActParams::Sound { cell, radius, .. } => {
                if !stand_on_site(*cell) {
                    return Err("a sounding stands on its site".to_owned());
                }
                check_radius(*radius)
            }
        }
    }
}

/// A reading's or a run's stand cell on the site, `0..256` on both axes: the address scheme's
/// shipped side until doc 32 cut S.1 scales it (a drive's `to` is held to its own `side`).
fn stand_on_site([x, y]: [i64; 2]) -> bool {
    crate::surface::address::cell_on_site((x, y), crate::surface::ground::SITE_CELLS)
}

/// [`ActParams::Drive`]'s `side` when the wire names none: the shipped side.
fn shipped_side() -> u16 {
    crate::surface::ground::SITE_CELLS
}

/// Whether a drive's `side` is the shipped side (and so is not written).
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's `skip_serializing_if` passes a reference
fn is_shipped_side(side: &u16) -> bool {
    *side == crate::surface::ground::SITE_CELLS
}

/// A sounding's reach, `1..=MAX_READ_RADIUS_CELLS`.
fn check_radius(radius: u32) -> Result<(), String> {
    if radius == 0 || radius > MAX_READ_RADIUS_CELLS {
        return Err(format!(
            "a sounding reads 1 to {MAX_READ_RADIUS_CELLS} cells"
        ));
    }
    Ok(())
}

/// A scan's seen and hidden fields: `1..=9` distinct valid seen, at most 8 distinct valid
/// hidden, the two disjoint.
fn check_fields(fields: &[u8], hidden: &[u8]) -> Result<(), String> {
    let valid = |index: &u8| crate::surface::address::field_of_index(*index).is_some();
    let distinct = |list: &[u8]| {
        let set: std::collections::BTreeSet<u8> = list.iter().copied().collect();
        set.len() == list.len()
    };
    if fields.is_empty() || fields.len() > MAX_SCAN_FIELDS || !fields.iter().all(valid) {
        return Err(format!(
            "a scan reads 1 to {MAX_SCAN_FIELDS} fields of its site"
        ));
    }
    if hidden.len() >= MAX_SCAN_FIELDS || !hidden.iter().all(valid) {
        return Err("a scan hides fewer than nine fields of its site".to_owned());
    }
    if !distinct(fields) || !distinct(hidden) || fields.iter().any(|f| hidden.contains(f)) {
        return Err("a scan's fields are distinct, each seen or hidden, never both".to_owned());
    }
    Ok(())
}

/// **What a finished scan or sounding read** (doc 20 cut SM.4a; §3.3 of the brief): carried on
/// its [`Outcome`] so the tell names what **this** act read after the act has cleared — the
/// stand, and the scan's fields or the sounding's reach — and whether a loud sounding was
/// heard (the heard budget, `crate::surface::chart::heard_line_due`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadSpan {
    /// A scan's.
    Scan {
        /// The stand cell.
        cell: [i64; 2],
        /// The footing it read from.
        footing: Footing,
        /// The fields it read.
        fields: Vec<u8>,
        /// The fields in reach the mast did not see.
        hidden: Vec<u8>,
    },
    /// A sounding's.
    Sound {
        /// The stand cell.
        cell: [i64; 2],
        /// Its reach, cells per axis.
        radius: u32,
        /// Quiet.
        quiet: bool,
        /// A loud sounding the site heard (inside the sounder's budget); never a quiet one.
        heard: bool,
    },
}

impl ReadSpan {
    /// The span's bounds: [`ActParams`]'s for the same act, and a quiet sounding never heard.
    fn check(&self) -> Result<(), String> {
        match self {
            ReadSpan::Scan {
                cell,
                fields,
                hidden,
                ..
            } => {
                if !stand_on_site(*cell) {
                    return Err("a scan stands on its site".to_owned());
                }
                check_fields(fields, hidden)
            }
            ReadSpan::Sound {
                cell,
                radius,
                quiet,
                heard,
            } => {
                if !stand_on_site(*cell) {
                    return Err("a sounding stands on its site".to_owned());
                }
                if *quiet && *heard {
                    return Err("a quiet sounding is heard by nobody".to_owned());
                }
                check_radius(*radius)
            }
        }
    }
}

/// **The act in flight** (§8; lead ruling 2). One at a time in SM.3b; SM.7 adds the cycle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Act {
    /// The record's ordinal when this act started.
    pub ordinal: u32,
    /// What it is.
    pub kind: ActKind,
    /// Who started it: the tick finishes it as this principal.
    pub operator: ObjId,
    /// The hull the rig works from.
    pub hull: ObjId,
    /// The rig's bay slot, `1..=`[`MAX_BAY_ITEMS`].
    pub slot: u8,
    /// The site port entry the hull is settled at.
    pub site: CanonicalId,
    /// The IC instant the start landed.
    pub started_ic: i64,
    /// The IC instant it is due: `started_ic + secs`.
    pub due_ic: i64,
    /// The charge this act reserved at its start (§4.1), cu.
    pub reserved_cu: u32,
    /// Its frozen parameters.
    pub params: ActParams,
    /// **Who set it going** (doc 20 cut SM.11; lead rulings 1 and 4): a fill plan's step or the
    /// idle return, from the start's server-authored `by` parameter; `None` for the hand.
    /// Skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<ActBy>,
    /// **The mind sealed aboard at the dispatch** (doc 17h cut M8e E4; doc 23 S.4a): a launch's
    /// `mind=` seal over a cored rig, taken at the start, which the finish's open line prints
    /// in place of the ROM seal. Boxed: the record rides many a tick's frame (the M7 stack-size
    /// lint). Skipped when `None` (schema 1 stands: pre-launch, no migration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<Box<crate::surface::dispatch::DispatchSeal>>,
}

/// **The sortie identity** (§4.3; D-UM-45): `(tv, launch_ic)` — the bay is the hangar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExcursionId {
    /// The terrain vehicle.
    pub tv: ObjId,
    /// The IC instant its launch started.
    pub launch_ic: i64,
}

impl std::fmt::Display for ExcursionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.tv.0, self.launch_ic)
    }
}

/// **An open excursion** (§4.3): from a launch's start to a recover's `Done`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Excursion {
    /// Its identity.
    pub id: ExcursionId,
    /// Who launched it.
    pub operator: ObjId,
    /// The hull it left.
    pub hull: ObjId,
    /// The rig's bay slot.
    pub slot: u8,
    /// The site port entry it went down onto.
    pub site: CanonicalId,
}

/// **Why a finish refused** (the brief's 3.4): the world at landing did not show the act
/// done.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusedCode {
    /// A launch whose rig never left the bay.
    NeverLeft,
    /// A recover whose rig was not beside the ramp.
    NotBeside,
    /// A dump whose hopper no longer held what was frozen.
    HopperChanged,
    /// **A dump the site's spoil would not take** (the SM.3b fixer round's item 4): the spoil
    /// vault the start froze is no longer the site's live spoil, a spoil pool will not read,
    /// or the transfer itself refused. Nothing moved; the load stays in the hopper.
    SpoilGone,
    /// **A scan or a sounding whose rig was not, at landing, where it stood when the act
    /// began** (doc 20 cut SM.4a, D14: a staff move). The charge is untouched and nothing is
    /// written to the chart.
    Moved,
    /// **A mining cycle whose block would not give up its ore** (doc 20 cut SM.7; §3.4): the
    /// block vault's pool held less than the cycle's yield, or the transfer refused. Nothing moved,
    /// nothing was spent, the chart is untouched and the run ends; the sweep's
    /// `initial − pool == Σ drawn` invariant decides whether the books disagree.
    Unworked,
    /// **An unload whose rig was not, at landing, in its hull's bay** (doc 20 cut SM.8; a staff
    /// move): nothing moved; the charge settles as it stands.
    LeftBay,
    /// **An unload the hold would not take** (doc 20 cut SM.8; the brief's decision R8): the hold
    /// vault or its plan changed, its ore compartment would not admit the frozen goods, or an
    /// earlier finish of this unload refused mid-bundle. Nothing moved; the ore stays in the
    /// hopper. Staff state only: §6's gate keeps an honest fill inside the hold's room.
    HoldRefused,
    /// **An unload whose hopper's marker ring is full** (doc 20 cut SM.8, the fix round's
    /// correctness/F2): the hopper vault's ring holds `mush_inventory::MAX_MARKER_RING` markers
    /// younger than the retention horizon, so the marked move refused before anything was staged.
    /// Nothing moved; the ore stays in the hopper until older markers age out. The ring is the
    /// hopper vault's — Occator's rig's — so it outlives a lease.
    MarkersFull,
    /// **The site promoted while it ran** (doc 32 cut CF.2; Q-CF-30, Q-CF-31): a drive stopped at
    /// the point its leg had reached and paid for the share it drove; a scan, a sounding or a dump
    /// was cancelled and paid for the share of its time that ran. Work resumes after the field by
    /// a fresh act.
    FieldOpened,
}

/// **How an act finished.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActResult {
    /// It happened.
    Done,
    /// It did not, and why.
    Refused {
        /// Why.
        code: RefusedCode,
    },
}

/// **The last finished act** — what the tick tells the operator once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    /// Its ordinal.
    pub ordinal: u32,
    /// What it was.
    pub kind: ActKind,
    /// Who it was for.
    pub operator: ObjId,
    /// The rig's slot.
    pub slot: u8,
    /// The site entry.
    pub site: CanonicalId,
    /// The IC instant it finished at: the act's own `due_ic` (lead ruling 2).
    pub at_ic: i64,
    /// How.
    pub result: ActResult,
    /// **What a dump moved** onto the spoil (good → quantity; empty for every other act and
    /// for a refused dump), so the tick's completion line can name the load after the act
    /// that froze it has cleared. Skipped when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub moved: BTreeMap<CanonicalId, u64>,
    /// **What a finished scan or sounding read** (doc 20 cut SM.4a), so the tell renders this
    /// act's readings from the chart after the act has cleared. `None` for every other act
    /// and for a refused one; skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<ReadSpan>,
    /// **Where a finished drive arrived** (doc 20 cut SM.4b): the end cell of a `Done` drive, so
    /// the tell names it after the act has cleared. `None` for every other act and for a refused
    /// drive; skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrived: Option<[i64; 2]>,
    /// **What a mining cycle did** (doc 20 cut SM.7; §6): every cycle's outcome — its tonnes, the
    /// cycle of the run, how the run ended — so the tick tells the cycle line after the finish
    /// has re-armed or closed the run. `Some` exactly for a [`ActKind::Mine`] outcome; skipped
    /// when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mined: Option<Mined>,
    /// **What an unload left in the hopper** (doc 20 cut SM.8; lead ruling 3: "unloads what fits
    /// and says what stays"): for a `Done` queued or whole unload, each hopper good's available
    /// quantity at landing less what moved, when positive; empty for every other act (`moved`
    /// carries what an unload moved). Skipped when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stays: BTreeMap<CanonicalId, u64>,
    /// **Who set the act going** (doc 20 cut SM.11): the act's (or, for a cycle, the run's)
    /// [`Act::by`], copied by the finish, so the tell knows a plan's or the idle return's act.
    /// Skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<ActBy>,
}

/// **What one mining cycle did** (doc 20 cut SM.7; §6, §1 beat 8's line): carried on its
/// [`Outcome`]. Nothing of the pocket's class, its tonnes left, or the deposit is here — a blind
/// core's outcome must read the same for a barren pocket and an ore pocket until the tell renders
/// the operator's own new reading (lead ruling 2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mined {
    /// The cycle of the run, `1..=cycles`.
    pub k: u8,
    /// The run's cycles.
    pub cycles: u8,
    /// The site-local pocket.
    pub pocket: [i64; 2],
    /// The family's ore good.
    pub good: CanonicalId,
    /// The tonnes this cycle moved into the hopper (0 for a barren core or a refused cycle).
    pub tonnes: u32,
    /// The run was a blind core.
    pub blind: bool,
    /// How the run ended with this cycle; `None` when it re-armed for the next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<RunEnd>,
}

/// **Why a run ended** (doc 20 cut SM.7; §6, §8 "rights are per act"; the brief's reviewer
/// finding F8): the precedence is `crate::surface::mine::run_end`'s, `Refused > WorkedOut >
/// Stopped > Dark > Licence > Claimed > Lease > Done`. Stores the standing, never a formatted
/// line: the tell formats it (reviewer F1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunEnd {
    /// The run's last cycle landed.
    Done,
    /// The pocket's tonnes of the family are drawn.
    WorkedOut,
    /// `+tv/mine stop` asked for it.
    Stopped,
    /// The licence stops the next cycle.
    Licence {
        /// The standing that stops it.
        why: LicenceEnd,
    },
    /// **The registry cannot be read for the run's block** (the SM.7 lead's pre-commit round,
    /// item 3): the tenure read at landing failed, so the next cycle stops (fail closed) — and the
    /// operator hears the dark line, never "That block is under claim.", which would be a false
    /// public fact. The cause is for the page, never a player.
    Dark {
        /// Which read failed.
        why: DarkCause,
    },
    /// A live claim the operator holds no right under stops the next cycle.
    Claimed,
    /// The rig's lease stops the next cycle: forfeit (`true`) or ended.
    Lease {
        /// The deposit is forfeit (else the economy lease ended).
        forfeit: bool,
    },
    /// The cycle did not happen (the rig moved, or the ground would not give up its ore).
    Refused,
}

/// **Why a block's tenure cannot be read** (doc 20 cut SM.7; the SM.7 lead's pre-commit round,
/// item 3): the cause [`RunEnd::Dark`] carries to its page. Never printed to a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DarkCause {
    /// The site's issuer does not resolve to a principal.
    Issuer,
    /// The run's site has no lane site definition.
    Site,
    /// One of the block's deposits does not read.
    Deposit,
    /// One of the block's deposits' claim index does not read.
    Claims,
}

impl DarkCause {
    /// The cause in words, for the page.
    #[must_use]
    pub fn words(self) -> &'static str {
        match self {
            DarkCause::Issuer => "the site's issuer does not resolve",
            DarkCause::Site => "the site's definition is missing",
            DarkCause::Deposit => "a deposit of the block does not read",
            DarkCause::Claims => "a deposit's claims do not read",
        }
    }
}

/// **Why the licence stops the next cycle** — the standing, never a line (the brief's reviewer
/// finding F1): the tell formats it through `crate::port::licence::standing_line`, as `+licence`
/// would.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LicenceEnd {
    /// Past its grace.
    Lapsed {
        /// The instant it lapsed.
        at_ic: i64,
    },
    /// No licence at all.
    Unlicensed,
    /// The entitlement account is not Active.
    Suspended,
    /// Another character of the account holds it.
    Other,
    /// The record cannot be read.
    Unreadable,
}

/// The most cycles one run commits (doc 20 §6 "at most eight cycles, by the 24 t cap"; §11).
pub const MAX_RUN_CYCLES: u8 = 8;
/// **The most cursor rows one run locks** (the SM.7 brief's §3.1, reviewer F14): SM.1 as built
/// lets a pocket carry at most two families (doc 20 §11's Geology row, "as built (SM.1)"), and a
/// run locks one row a family with ore — so the finish's close bundle stays within §11's ≤ 8
/// writes.
pub const MAX_LOCK_ROWS: usize = 2;

/// **One cursor row a run locked** (doc 20 cut SM.7; lead ruling 3): the deposit's cursor and the
/// pocket's number in the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockRow {
    /// The deposit whose cursor holds the row.
    pub deposit: mush_industry::OreDepositId,
    /// The pocket's number in its block, `1..=256`.
    pub pocket: u16,
}

/// **A mining run in flight** (doc 20 §6, §8 "the current cycle {pocket, family, k, cycles,
/// yield}"; the SM.7 brief's lead ruling 1 and decision R1): one act started once, its cycles
/// landing at their own dues — the record's **cycle slot**, beside the act slot, so a scan or a
/// sounding may run from the same rig while it drills (D-20-9). Everything the finish needs is
/// frozen here at the start, so a cycle reads no geology and no site. **Derived truth on a durable
/// record** (`class_t`, `pocket_t`; the brief's decision R13, the cursor's precedent): never
/// projected — [`RunView`] carries none of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    /// The record's ordinal at its start; never reused.
    pub ordinal: u32,
    /// Who started it: the tick finishes each cycle as this principal.
    pub operator: ObjId,
    /// The hull the rig works from.
    pub hull: ObjId,
    /// The rig's bay slot.
    pub slot: u8,
    /// The site port entry.
    pub site: CanonicalId,
    /// The site's body (a core creates the operator's chart row when they have none).
    pub body: CanonicalId,
    /// The rig's site cell, frozen: it never moves during a run.
    pub stand: [i64; 2],
    /// The site-local pocket under the stand.
    pub pocket: [i64; 2],
    /// The pocket's number in its block, `1..=256` (the cursor row's key).
    pub number: u16,
    /// The pocket's block letter.
    pub letter: char,
    /// The family mined, frozen (§6).
    pub family: crate::surface::geology::OreFamily,
    /// The (site, block, family)'s receipt-derived deposit id — derivable whether or not it was
    /// opened; never shown.
    pub deposit: mush_industry::OreDepositId,
    /// The family's ore good.
    pub good: CanonicalId,
    /// The block vault's endpoint when the pocket holds the family's ore, else `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<u64>,
    /// The rig's hopper endpoint.
    pub hopper: u64,
    /// The run's cycles, `1..=`[`MAX_RUN_CYCLES`].
    pub cycles: u8,
    /// The cycle in flight, `1..=cycles`.
    pub k: u8,
    /// IC seconds a cycle (the drill's `cycle_s`, × its `rock_factor` on rock).
    pub secs: u32,
    /// Charge a cycle, cu (the drill's `energy_cu`).
    pub cost_cu: u32,
    /// Per cycle, the room the run reserves, whole tonnes: the plan's yields, or the blind room.
    pub reserve_t: Vec<u8>,
    /// The drill's yield for the pocket's true class in `family` (0 barren) — truth, never shown.
    pub class_t: u8,
    /// The pocket's true tonnes of `family` — truth, never shown.
    pub pocket_t: u8,
    /// The operator had no reading of the pocket at the start: one blind cycle.
    pub blind: bool,
    /// `+tv/mine stop`: the run closes after cycle `k`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stop: bool,
    /// The licence's lapse, frozen at the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lapses_ic: Option<i64>,
    /// The cursor rows this run locked, at most [`MAX_LOCK_ROWS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lock_rows: Vec<LockRow>,
    /// Cycle `k`'s due instant.
    pub due_ic: i64,
    /// The run's remaining reservation: `cost_cu × (cycles − k + 1)`.
    pub reserved_cu: u32,
    /// **Who set it going** (doc 20 cut SM.11): a fill plan's mine step, or `None` for the hand;
    /// each cycle's outcome carries it, and its cores are written `via: Plan`. Skipped when
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<ActBy>,
}

impl Run {
    /// **The tonnes still reserved** (doc 20 §3.5 "every outstanding, uncompleted cycle"; the
    /// brief's decision R8): `Σ reserve_t[k − 1 ..]` — the cycle in flight included, its tonnes
    /// not yet in the hopper.
    #[must_use]
    pub fn outstanding_t(&self) -> u64 {
        let from = usize::from(self.k.saturating_sub(1));
        self.reserve_t
            .iter()
            .skip(from)
            .map(|t| u64::from(*t))
            .sum()
    }

    /// The run's bounds (`RigRecord::check` calls it).
    fn check(&self) -> Result<(), String> {
        use crate::surface::address;
        if !slot_ok(self.slot) {
            return Err(format!("a run's slot is 1 to {MAX_BAY_ITEMS}"));
        }
        if self.cycles == 0 || self.cycles > MAX_RUN_CYCLES || self.k == 0 || self.k > self.cycles {
            return Err(format!(
                "a run's cycle k is 1 to its cycles, at most {MAX_RUN_CYCLES}"
            ));
        }
        if self.reserve_t.len() != usize::from(self.cycles) {
            return Err("a run reserves room for each of its cycles".to_owned());
        }
        let cap = crate::surface::cursor::MAX_POCKET_TONNES;
        if self.reserve_t.iter().any(|t| u32::from(*t) > cap)
            || u32::from(self.class_t) > cap
            || u32::from(self.pocket_t) > cap
        {
            return Err(format!("a run's tonnes are at most {cap}"));
        }
        if self.blind && self.cycles != 1 {
            return Err("a blind run is one cycle".to_owned());
        }
        if !stand_on_site(self.stand) {
            return Err("a run stands on its site".to_owned());
        }
        let pocket = address::pocket_of_cell((self.stand[0], self.stand[1]));
        if pocket != Some((self.pocket[0], self.pocket[1])) {
            return Err("a run's pocket is the one under its stand".to_owned());
        }
        if address::pocket_number(self.pocket[0], self.pocket[1]) != Some(self.number)
            || address::block_letter(self.pocket[0], self.pocket[1]) != Some(self.letter)
        {
            return Err("a run names its pocket's number and block".to_owned());
        }
        if self.source.is_some() != (self.class_t > 0 && self.pocket_t > 0) {
            return Err(
                "a run draws from the block's vault exactly when the pocket holds ore".to_owned(),
            );
        }
        let remaining = u32::from(self.cycles)
            .checked_sub(u32::from(self.k))
            .and_then(|left| left.checked_add(1))
            .and_then(|left| left.checked_mul(self.cost_cu));
        if remaining != Some(self.reserved_cu) {
            return Err("a run reserves its remaining cycles' charge".to_owned());
        }
        if self.lock_rows.len() > MAX_LOCK_ROWS
            || self.lock_rows.iter().any(|row| row.pocket != self.number)
        {
            return Err(format!(
                "a run locks at most {MAX_LOCK_ROWS} rows, each its own pocket's"
            ));
        }
        if self.due_ic < 0 {
            return Err("a run's due instant is on the IC clock".to_owned());
        }
        Ok(())
    }
}

/// **One rig's record in `tacsim.excursion`** (§8; lead ruling 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RigRecord {
    /// Causal link to the existing DT flight for a physically crewed rig.
    /// The flight owns its hull epoch; this record owns only surface work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_crew: Option<Box<crate::detachment::sortie::SurfaceCrew>>,
    /// Latest real action/receipt for a delivered NPC task, bounded to one link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_work: Option<Box<crate::detachment::sortie::FieldWork>>,
    /// [`EXCURSION_SCHEMA`].
    pub schema: u16,
    /// The terrain vehicle: the key.
    pub tv: ObjId,
    /// The last act ordinal started on this rig (0: none); monotonic, never reused.
    pub ordinal: u32,
    /// The charge, lazily (§4.1).
    pub charge: Charge,
    /// The act in flight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act: Option<Act>,
    /// The open excursion: `Some` from a launch's start to a recover's `Done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excursion: Option<Excursion>,
    /// The last finished act.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<Outcome>,
    /// **The cycle slot** (doc 20 cut SM.7; §8; the brief's decision R1): the mining run in
    /// flight, beside the act slot. Skipped when `None` (schema 1 stands: pre-launch, no
    /// migration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Run>,
    /// **An act a finish queued for the tick to start** (doc 20 cut SM.8; the brief's lead ruling
    /// 1, decision R1): the unload a `Done` recover files when the hopper holds ore. Skipped when
    /// `None` (schema 1 stands: pre-launch, no migration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued: Option<Queued>,
    /// **The rig's fill plan** (doc 20 cut SM.11; §8 "Leaving it running"; the SM.11 brief's lead
    /// ruling 1, decision D1): live, parking or ended (an ended plan keeps its outcome until the
    /// next filing or `+tv/plan clear`). Written only in the start's prepare — the filing, every
    /// plan step's start, the end, the clear, the strike — and by the finish's tally, each in the
    /// bundle that writes this record. Skipped when `None` (schema 1 stands: pre-launch, no
    /// migration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<FillPlan>,
    /// **Plans ever filed on the rig** (doc 20 cut SM.11): the next plan's number is `plans + 1`.
    /// Skipped at 0.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub plans: u32,
    /// Last staff recovery; a pending home owns the otherwise idle rig.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Box<crate::surface::recovery::RecoveryReceipt>>,
    /// **The rig's open seal epoch** (doc 23 cut S.1; §6, §7, D-23-1, D-23-4; the lead's L5):
    /// `Some` from the launch's `Done` — the rig left the bay — to the epoch's close, a
    /// recover's `Done` or a staff home. Independent of [`RigRecord::excursion`], the act-level
    /// "out": a staff close clears the excursion and **keeps** the epoch, flagged
    /// [`OpenSortie::abandoned`] (abandonment never closes an epoch). Written only in the
    /// finish's and the staff recovery's prepares, each in the bundle that writes its black box
    /// line. Skipped when `None` (schema 1 stands: pre-launch, no migration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sortie: Option<OpenSortie>,
}

/// **An open seal epoch on a rig** (doc 23 cut S.1, the lead's L5): its id, the hangar it left,
/// the site, who launched it, when it opened, whether staff abandoned it, and its seal's facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenSortie {
    /// Its id: [`SortieId::ic`](crate::arena::mind::SortieId::ic)`(tv, launch_ic)` — the rig and
    /// the instant its launch started ([`ExcursionId`]'s pair).
    pub id: crate::arena::mind::SortieId,
    /// The carrier whose bay it left (the hangar, D-23-2).
    pub hull: ObjId,
    /// The rig's bay slot.
    pub slot: u8,
    /// The site port entry it went down onto.
    pub site: CanonicalId,
    /// Who launched it (the plan's actor, D-23-5).
    pub operator: ObjId,
    /// The IC instant it opened: the launch's due instant, when the rig left the bay.
    pub opened_ic: i64,
    /// **Whether staff closed the excursion with the rig out** (D-23-4): the epoch stays open
    /// and flagged until a home or a recover closes it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub abandoned: bool,
    /// **The IC instant staff abandoned it** (doc 23 cut S.3, the lead's A1; D-23-4: "flagged
    /// `abandoned` with the instant"): the instant of the staff `close` that first set
    /// [`OpenSortie::abandoned`] — a second `close` keeps the first instant. The flag and the
    /// instant live until the epoch closes (a home or a recover takes the record's `sortie`)
    /// and nothing else clears them. Skipped when `None` (schema 1 stands).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abandoned_ic: Option<i64>,
    /// **Whether the epoch's open line was never written** (the S.1 fix round's F2): the black
    /// box was dark or full, or the open was prepared off the composed path. The epoch is real
    /// either way; the close line carries the flag so an inspection tells "no line" from "no
    /// epoch". Skipped when `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unrecorded: bool,
    /// **The seal's facts** (cut S.1's L8): `None` when the rig had no `TacticalUnit` to seal —
    /// the epoch opened anyway and its line says `seal: absent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<crate::surface::black_box::SealFacts>,
}

/// serde's `skip_serializing_if` for [`RigRecord::plans`].
#[allow(clippy::trivially_copy_pass_by_ref)] // serde passes a reference
const fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

/// **An act a finish queued for the tick to start** (doc 20 §7.1; cut SM.8 lead ruling 1): the
/// unload a `Done` recover files when the hopper holds ore. Durable, so a restart between the
/// recover and the unload loses nothing (§8 "nothing is RAM-only"); cleared by ANY start that
/// lands on the rig (the operator's newer act supersedes it). Shaped so a fill plan's chain
/// (SM.11) can reuse it; no chain is built here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Queued {
    /// What the tick starts: [`ActKind::Unload`], the only kind SM.8 queues.
    pub kind: ActKind,
    /// Who it is started for: the tick submits as this principal.
    pub operator: ObjId,
    /// The hull the rig came home to.
    pub hull: ObjId,
    /// The rig's bay slot.
    pub slot: u8,
    /// The site port entry the hull is settled at.
    pub site: CanonicalId,
    /// The ordinal of the recover that queued it.
    pub after: u32,
    /// The recover's due instant: the tick's order key.
    pub at_ic: i64,
    /// The excursion that recover closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excursion: Option<ExcursionId>,
}

/// A slot word a record may carry: `1..=MAX_BAY_ITEMS`.
fn slot_ok(slot: u8) -> bool {
    slot >= 1 && u32::from(slot) <= MAX_BAY_ITEMS
}

impl RigRecord {
    /// **A rig never acted on**: ordinal 0, a full battery in the bay at IC 0, nothing in
    /// flight. A full battery at IC 0 in the bay reads full at every later instant
    /// ([`charge_at`] caps at the capacity).
    #[must_use]
    pub fn fresh(tv: ObjId, battery: &BatteryDef) -> RigRecord {
        RigRecord {
            schema: EXCURSION_SCHEMA,
            tv,
            ordinal: 0,
            charge: Charge {
                value_cu: battery.capacity_cu,
                at_ic: 0,
                in_bay: true,
            },
            act: None,
            excursion: None,
            last: None,
            run: None,
            queued: None,
            plan: None,
            plans: 0,
            recovery: None,
            sortie: None,
            field_crew: None,
            field_work: None,
        }
    }

    /// **A rig never acted on whose battery row cannot be read** (§4.1 "a TV cannot be
    /// stranded by its battery"; the SM.3b fixer round's item 3): [`RigRecord::fresh`] with the
    /// charge written as `u32::MAX` at IC 0 in the bay — full under **whichever** battery reads
    /// it next, because [`charge_at`] never reads above the capacity. Only a zero-spend act
    /// (a recover or a dump, [`needs_battery`] false) starts on it.
    #[must_use]
    pub fn fresh_unread(tv: ObjId) -> RigRecord {
        RigRecord {
            schema: EXCURSION_SCHEMA,
            tv,
            ordinal: 0,
            charge: Charge {
                value_cu: u32::MAX,
                at_ic: 0,
                in_bay: true,
            },
            act: None,
            excursion: None,
            last: None,
            run: None,
            queued: None,
            plan: None,
            plans: 0,
            recovery: None,
            sortie: None,
            field_crew: None,
            field_work: None,
        }
    }

    /// The record's key, derived from the value (the families rule: writers derive the key
    /// from the value, readers re-check it).
    #[must_use]
    pub fn key(&self) -> RigKey {
        RigKey(self.tv)
    }

    /// **The record's own invariants** (the family's validator calls it): the schema; an
    /// act's ordinal is the record's, `due_ic >= started_ic`, a dump freezes
    /// `1..=`[`MAX_DUMP_GOODS`] positive rows; the excursion is this rig's; the last act is
    /// no later than the ordinal; every slot `1..=`[`MAX_BAY_ITEMS`].
    ///
    /// # Errors
    ///
    /// The first invariant broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if let Some(work) = self.field_work.as_deref() {
            work.check(
                self.field_crew
                    .as_deref()
                    .ok_or("Field work needs its physical crew link.")?,
                self.ordinal,
            )?;
        }
        if let Some(crew) = self.field_crew.as_deref() {
            crew.operation.key()?;
            if crew.flight.unit != self.tv
                || crew.flight.ordinal == 0
                || crew.operator.0 == 0
                || crew.principal.0 == 0
                || self.sortie.is_some()
                || self
                    .excursion
                    .as_ref()
                    .is_some_and(|e| e.operator != crew.operator)
                || self
                    .act
                    .as_ref()
                    .is_some_and(|a| a.operator != crew.operator)
                || self
                    .run
                    .as_ref()
                    .is_some_and(|r| r.operator != crew.operator)
            {
                return Err("physical surface work must match its actual flight operator and own no second epoch".into());
            }
        }
        if self.schema != EXCURSION_SCHEMA {
            return Err(format!(
                "an excursion record is schema {EXCURSION_SCHEMA}, not {}",
                self.schema
            ));
        }
        if let Some(recovery) = &self.recovery {
            recovery.check(self)?;
        }
        // Doc 20 cut SM.7 (lead ruling 1, decision R1): the run in the cycle slot, and the one
        // kind of act that may share the rig with it.
        if let Some(run) = &self.run {
            if run.ordinal > self.ordinal || run.ordinal == 0 {
                return Err("a run's ordinal is no later than the record's".to_owned());
            }
            run.check()?;
            if let Some(act) = &self.act
                && (act.ordinal <= run.ordinal
                    || !matches!(act.kind, ActKind::Scan | ActKind::Sound))
            {
                return Err(
                    "only a scan or a sounding started later shares a rig with its run".to_owned(),
                );
            }
        }
        if let Some(act) = &self.act {
            if act.kind == ActKind::Mine {
                return Err("a run lives in the cycle slot, never the act slot".to_owned());
            }
            if act.ordinal != self.ordinal {
                return Err("an act in flight carries the record's ordinal".to_owned());
            }
            if act.due_ic < act.started_ic {
                return Err("an act is due no earlier than it started".to_owned());
            }
            if !slot_ok(act.slot) {
                return Err(format!("an act's slot is 1 to {MAX_BAY_ITEMS}"));
            }
            if let ActParams::Dump { goods, .. } = &act.params
                && (goods.is_empty()
                    || goods.len() > MAX_DUMP_GOODS
                    || goods.values().any(|qty| *qty == 0))
            {
                return Err(format!(
                    "a dump freezes 1 to {MAX_DUMP_GOODS} goods, each a positive quantity"
                ));
            }
            // Doc 20 cut SM.4a: a scan's or a sounding's frozen reach, and its kind's shape.
            act.params.check()?;
            // Doc 20 cut SM.4b: a drive carries its leg; a recover carries none, or a leg home
            // that never implies a launch.
            let shaped = match (&act.kind, &act.params) {
                (ActKind::Launch | ActKind::Recover, ActParams::None {})
                | (ActKind::Dump, ActParams::Dump { .. })
                | (ActKind::Scan, ActParams::Scan { .. })
                | (ActKind::Sound, ActParams::Sound { .. })
                | (ActKind::Drive, ActParams::Drive { .. })
                | (ActKind::Unload, ActParams::Unload { .. })
                | (
                    ActKind::Recover,
                    ActParams::Drive {
                        from_bay: false, ..
                    },
                ) => true,
                (
                    ActKind::Launch
                    | ActKind::Recover
                    | ActKind::Dump
                    | ActKind::Scan
                    | ActKind::Sound
                    | ActKind::Drive
                    | ActKind::Mine
                    | ActKind::Unload,
                    _,
                ) => false,
            };
            if !shaped {
                return Err(format!(
                    "an act's parameters are its kind's ({})",
                    act.kind.word()
                ));
            }
        }
        if let Some(excursion) = &self.excursion {
            if excursion.id.tv != self.tv {
                return Err("an excursion is its own rig's".to_owned());
            }
            if !slot_ok(excursion.slot) {
                return Err(format!("an excursion's slot is 1 to {MAX_BAY_ITEMS}"));
            }
        }
        // Doc 23 cut S.1 (L1, L5): the open epoch is an IC epoch of this rig, and while the
        // excursion that opened it is still open the two name the same launch.
        if let Some(sortie) = &self.sortie {
            let crate::arena::mind::SortieId::Ic { hull, at_ic } = sortie.id else {
                return Err("a rig's epoch is an IC epoch".to_owned());
            };
            if hull != self.tv {
                return Err("a rig's epoch is its own".to_owned());
            }
            if !slot_ok(sortie.slot) || sortie.hull == self.tv {
                return Err(format!(
                    "an epoch leaves another hull's bay slot, 1 to {MAX_BAY_ITEMS}"
                ));
            }
            if self
                .excursion
                .as_ref()
                .is_some_and(|open| open.id.launch_ic != at_ic)
            {
                return Err("an epoch and its open excursion name one launch".to_owned());
            }
            // Doc 23 cut S.3 (A1): the abandonment's instant belongs to an abandoned epoch.
            if sortie.abandoned_ic.is_some() && !sortie.abandoned {
                return Err("only an abandoned epoch carries its abandonment's instant".to_owned());
            }
        }
        if let Some(last) = &self.last {
            if last.ordinal > self.ordinal {
                return Err("the last act is no later than the record's ordinal".to_owned());
            }
            if !slot_ok(last.slot) {
                return Err(format!("a finished act's slot is 1 to {MAX_BAY_ITEMS}"));
            }
            if last.moved.len() > MAX_DUMP_GOODS || last.moved.values().any(|qty| *qty == 0) {
                return Err(format!(
                    "a finished dump moved at most {MAX_DUMP_GOODS} goods, each a positive quantity"
                ));
            }
            // Doc 20 cut SM.8: only an unload says what stayed in the hopper.
            if !last.stays.is_empty()
                && (last.kind != ActKind::Unload
                    || last.stays.len() > MAX_DUMP_GOODS
                    || last.stays.values().any(|qty| *qty == 0))
            {
                return Err(format!(
                    "only a finished unload names what stays, at most {MAX_DUMP_GOODS} goods, \
                     each a positive quantity"
                ));
            }
            // Doc 20 cut SM.4a: what a finished scan or sounding read, within its bounds.
            if let Some(span) = &last.span {
                span.check()?;
                let shaped = matches!(
                    (last.kind, span),
                    (ActKind::Scan, ReadSpan::Scan { .. })
                        | (ActKind::Sound, ReadSpan::Sound { .. })
                );
                if !shaped {
                    return Err("a finished act's span is a scan's or a sounding's".to_owned());
                }
            }
            // Doc 20 cut SM.4b: only a finished drive arrived, on the site.
            // Doc 32 cut S.0: on the widest site; the drive's own `to` held it to its own side.
            if let Some(cell) = last.arrived
                && (last.kind != ActKind::Drive
                    || last.result != ActResult::Done
                    || !crate::surface::address::cell_on_site(
                        (cell[0], cell[1]),
                        crate::surface::ground::MAX_SITE_CELLS,
                    ))
            {
                return Err("only a finished drive arrives, on its site".to_owned());
            }
            // Doc 20 cut SM.7: a mining outcome carries what its cycle did, and only it.
            if last.mined.is_some() != (last.kind == ActKind::Mine) {
                return Err("a mining outcome, and only one, carries its cycle".to_owned());
            }
            if let Some(mined) = &last.mined
                && (mined.cycles == 0
                    || mined.cycles > MAX_RUN_CYCLES
                    || mined.k == 0
                    || mined.k > mined.cycles
                    || mined.tonnes > crate::surface::cursor::MAX_POCKET_TONNES)
            {
                return Err("a mining outcome names a cycle of its run and its tonnes".to_owned());
            }
        }
        // Doc 20 cut SM.8 (lead ruling 1): a queued unload waits on an idle rig — no act, no run.
        if let Some(queued) = &self.queued {
            if queued.kind != ActKind::Unload {
                return Err("only an unload is queued".to_owned());
            }
            if queued.after > self.ordinal || queued.after == 0 {
                return Err("a queued act follows an act of the record".to_owned());
            }
            if !slot_ok(queued.slot) {
                return Err(format!("a queued act's slot is 1 to {MAX_BAY_ITEMS}"));
            }
            if self.act.is_some() || self.run.is_some() {
                return Err("a queued act waits on an idle rig".to_owned());
            }
        }
        // Doc 20 cut SM.11 (lead rulings 1 and 4): the plan's own bounds and number; who set an
        // act, a run or an outcome going — the idle return only ever recovers, and a plan never
        // dumps or unloads (SM.8's queued unload is the plan's recover's second half).
        if let Some(plan) = &self.plan {
            plan.check()?;
            if plan.no == 0 || plan.no > self.plans {
                return Err("a plan's number is one of the rig's plans".to_owned());
            }
        }
        let by_ok = |kind: ActKind, by: Option<&ActBy>| match by {
            None => true,
            Some(ActBy::Idle) => kind == ActKind::Recover,
            Some(ActBy::Plan { .. }) => !matches!(kind, ActKind::Dump | ActKind::Unload),
        };
        if let Some(act) = &self.act
            && !by_ok(act.kind, act.by.as_ref())
        {
            return Err(
                "an act is set going by the hand, a plan's step or the idle return".to_owned(),
            );
        }
        if let Some(run) = &self.run
            && !by_ok(ActKind::Mine, run.by.as_ref())
        {
            return Err("a run is set going by the hand or a plan's step".to_owned());
        }
        if let Some(last) = &self.last
            && !by_ok(last.kind, last.by.as_ref())
        {
            return Err(
                "a finished act was set going by the hand, a plan's step or the idle return"
                    .to_owned(),
            );
        }
        Ok(())
    }

    /// **The rig's plan, when it is live for `lessee`** (doc 20 cut SM.11; the brief's decision
    /// D12): a plan whose author no longer leases the rig is orphaned — inert.
    #[must_use]
    pub fn live_plan_for(&self, lessee: ObjId) -> Option<&FillPlan> {
        self.plan.as_ref().filter(|plan| plan.live_for(lessee))
    }

    /// **The instant the rig last acted or its plan ended** (doc 20 cut SM.11; the brief's 3.6
    /// and 3.8, lead ruling 6): `max(last.at_ic, the ended plan's at_ic)`, else the open
    /// excursion's launch; `None` with neither. The idle return reads it.
    #[must_use]
    pub fn idle_since(&self) -> Option<i64> {
        let last = self.last.as_ref().map(|last| last.at_ic);
        let ended = self.plan.as_ref().and_then(|plan| match &plan.state {
            FillState::Ended { at_ic, .. } => Some(*at_ic),
            FillState::Live | FillState::Parking { .. } => None,
        });
        match (last, ended) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a
                .or(b)
                .or_else(|| self.excursion.as_ref().map(|open| open.id.launch_ic)),
        }
    }
}

/// **Every in-flight reservation the record holds**, summed (§4.1, §19 item 4: "two acts can
/// never spend one margin"): the act's `reserved_cu` plus — since doc 20 cut SM.7 — the run's
/// remaining reservation (`Run::reserved_cu`, its cycles from the one in flight on), checked and
/// saturating at `u32::MAX`. The act slot alone is always empty at a start's reserve check (the
/// one act slot and the ordinal); **the cycle slot is the reservation that coexists with a new
/// act**: a scan or a sounding started during a run reads the run's remaining reservation here,
/// so the run's margin is never spent twice (SM.7's `tests/mining.rs` M10 proves the sum in the
/// world).
#[must_use]
pub fn reserved_total(record: &RigRecord) -> u32 {
    let act = record.act.as_ref().map_or(0, |act| act.reserved_cu);
    let run = record.run.as_ref().map_or(0, |run| run.reserved_cu);
    act.saturating_add(run)
}

// ---- the pure arithmetic ----------------------------------------------------------------

/// **The charge at `now_ic`** (§4.1; lead ruling 4): in the bay `min(capacity, value + ⌊(now
/// − at) × recharge / 3600⌋)` in checked `u64` (a negative elapsed reads as 0, an overflow
/// saturates at the capacity); out of the bay `value` — out of the bay charge only falls by
/// acts. Never above the capacity.
#[must_use]
pub fn charge_at(charge: &Charge, now_ic: i64, battery: &BatteryDef) -> u32 {
    let capacity = battery.capacity_cu;
    if !charge.in_bay {
        return charge.value_cu.min(capacity);
    }
    let elapsed = u64::try_from(now_ic.saturating_sub(charge.at_ic).max(0)).unwrap_or(0);
    let hour = u64::try_from(IC_HOUR).unwrap_or(3_600);
    let gained = elapsed
        .checked_mul(u64::from(battery.bay_recharge_cu_per_ic_hour))
        .map(|product| product / hour);
    let total = gained.and_then(|gained| gained.checked_add(u64::from(charge.value_cu)));
    match total {
        Some(total) => u32::try_from(total.min(u64::from(capacity))).unwrap_or(capacity),
        None => capacity,
    }
}

/// **The charge an act will spend from** (doc 20 cut SM.4a; §4.1; the brief's reviewer finding
/// F1, D22): in the bay with the launch implied, the bay's recharge up to the instant the
/// launch half ends (`now + LAUNCH_S`) and no further — out of the bay charge only falls by
/// acts; otherwise [`charge_at`] `now`. The command's preview and reserve, the start's
/// prepare reserve and the finish's settlement (`spent_after_launch` in
/// `crate::surface::act`) all read this same instant, so the reserve admits exactly what the
/// finish settles.
#[must_use]
pub fn charge_for_act(charge: &Charge, now_ic: i64, from_bay: bool, battery: &BatteryDef) -> u32 {
    if from_bay && charge.in_bay {
        charge_at(charge, now_ic.saturating_add(i64::from(LAUNCH_S)), battery)
    } else {
        charge_at(charge, now_ic, battery)
    }
}

/// **The charge `+tv` shows** (doc 20 cut SM.4a, D22): [`charge_at`] `now`, except while an
/// implied launch is in flight — its record stays `in_bay` until the finish, so its bay credit
/// is read no later than `started_ic + LAUNCH_S`, the instant the finish will settle it at.
/// Since doc 20 cut SM.4b a drive from the bay is an implied launch too (`params.launches()`).
#[must_use]
pub fn charge_shown(record: &RigRecord, now_ic: i64, battery: &BatteryDef) -> u32 {
    match &record.act {
        Some(act) if act.params.launches() => {
            let launched = act.started_ic.saturating_add(i64::from(LAUNCH_S));
            charge_at(&record.charge, now_ic.min(launched), battery)
        }
        Some(_) | None => charge_at(&record.charge, now_ic, battery),
    }
}

/// **What an act leaves** (§4.1): after the act itself, and after the trip home too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChargeAfter {
    /// The charge after the act, cu.
    pub after_act: u32,
    /// The charge after the act and the trip home, cu.
    pub after_home: u32,
}

/// **Why the reserve refuses** (§4.1): what would be left after the trip home (possibly
/// negative) and the floor it falls under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReserveShort {
    /// The charge after the act and the trip home, cu; may be negative.
    pub after_home: i64,
    /// The floor: `capacity × reserve_permille / 1000`, cu.
    pub floor: u32,
}

/// **The reserve rule** (§4.1: "refused when that would fall under 5 %"; enforced in the
/// start's prepare and nowhere else, lead ruling 3): `floor = capacity × reserve_permille /
/// 1000` (5,000 cu on the Mk1); refused iff `now − reserved − cost − home < floor` in signed
/// `i64` — exactly the floor fits. **A zero spend always passes** (`cost + home == 0`): the
/// rule guards a spend, and §4.1 "a TV cannot be stranded by its battery" and §8 / §19 item 3
/// "never a rig's way home" mean a recover or a dump is never refused by it, even when a
/// tools-data change leaves a rig under the floor.
///
/// # Errors
///
/// [`ReserveShort`] when a spend would fall under the floor.
pub fn reserve_check(
    now_cu: u32,
    reserved_cu: u32,
    cost_cu: u32,
    home_cu: u32,
    battery: &BatteryDef,
) -> Result<ChargeAfter, ReserveShort> {
    let floor = u32::try_from(
        u64::from(battery.capacity_cu).saturating_mul(u64::from(battery.reserve_permille)) / 1_000,
    )
    .unwrap_or(u32::MAX);
    let after_act = i64::from(now_cu) - i64::from(reserved_cu) - i64::from(cost_cu);
    let after_home = after_act - i64::from(home_cu);
    let clamp = |cu: i64| u32::try_from(cu.max(0)).unwrap_or(u32::MAX);
    let spend = u64::from(cost_cu).saturating_add(u64::from(home_cu));
    if spend > 0 && after_home < i64::from(floor) {
        return Err(ReserveShort { after_home, floor });
    }
    Ok(ChargeAfter {
        after_act: clamp(after_act),
        after_home: clamp(after_home),
    })
}

/// **Whether an act must read its rig's battery** (§4.1 "a TV cannot be stranded by its
/// battery"; the SM.3b fixer round's item 3): only a **spend** (`cost + home > 0`, where the
/// reserve rule reads the floor) or a **launch**, which leaves the bay's umbilical and so
/// settles the charge the bay put back (the recharge rate is the battery's). A dump spends
/// nothing and never reads it, and — since doc 20 cut SM.4b — **a recover never does, even one
/// that drives home and spends its leg's charge**: its leg is checked against the record's own
/// out-of-bay value ([`leg_affordable`]), which needs no battery, so a rig whose battery row
/// stops resolving still plans, starts and finishes its way home and still empties its hopper;
/// a launch fails closed.
#[must_use]
pub fn needs_battery(kind: ActKind, cost_cu: u32, home_cu: u32) -> bool {
    kind == ActKind::Launch
        || (kind != ActKind::Recover && u64::from(cost_cu).saturating_add(u64::from(home_cu)) > 0)
}

/// **Whether a rig can afford its own leg home** (doc 20 cut SM.4b; lead ruling 5, the brief's
/// decision P6): out of the bay, `value − reserved − cost ≥ 0` on the record's own value (out of
/// the bay charge only falls by acts, so no battery is read); in the bay always (a recover from
/// the bay drives nowhere). The 5 % floor is never applied to the way home: the floor exists to
/// pay it.
#[must_use]
pub fn leg_affordable(charge: &Charge, reserved_cu: u32, cost_cu: u32) -> bool {
    charge.in_bay
        || u64::from(charge.value_cu) >= u64::from(reserved_cu).saturating_add(u64::from(cost_cu))
}

/// **A zero spend's charge at its act's stamp, without the battery** (§4.1; the SM.3b fixer
/// round's item 3): staying in or out of the bay, the lazy charge is unchanged (its value
/// read later through [`charge_at`] is the same); coming into the bay from the ground, the
/// value is what it was (out of the bay charge only falls by acts) and the bay's recharge
/// starts at `at_ic`. `None` for leaving the bay — the recharge it gained must be settled,
/// which needs the battery (a launch, which [`needs_battery`] always reads).
#[must_use]
pub fn settle_free(charge: &Charge, at_ic: i64, in_bay: bool) -> Option<Charge> {
    match (charge.in_bay, in_bay) {
        (true, true) | (false, false) => Some(*charge),
        (false, true) => Some(Charge {
            value_cu: charge.value_cu,
            at_ic,
            in_bay: true,
        }),
        (true, false) => None,
    }
}

/// **A spend out of the bay, without the battery** (doc 20 cut SM.4a's pre-commit round, item 2;
/// §4.1 "a TV cannot be stranded by its battery"): out of the bay [`charge_at`] reads the stored
/// value and adds nothing (charge only falls by acts), so a scan or a sounding started from the
/// ground settles without its battery's row exactly as it would with it — the reservation off
/// the value, stamped `at_ic`, still out of the bay — and a battery row that stops resolving
/// between the start and the finish never wedges the rig. `None` for a charge in the bay
/// (leaving the umbilical settles the bay's credit, which needs the battery: the implied launch
/// fails closed, as SM.3b's launch does) or a reservation past the value.
#[must_use]
pub fn settle_spent_out_of_bay(charge: &Charge, reserved_cu: u32, at_ic: i64) -> Option<Charge> {
    if charge.in_bay {
        return None;
    }
    Some(Charge {
        value_cu: charge.value_cu.checked_sub(reserved_cu)?,
        at_ic,
        in_bay: false,
    })
}

/// **The open excursions an act's caps count** (§8, §11 Caps): the operator's, the
/// site's and the world's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExcursionCounts {
    /// Open excursions whose operator is the actor.
    pub operator: u32,
    /// Open excursions at the act's site.
    pub site: u32,
    /// Open excursions anywhere.
    pub world: u32,
}

/// **Which cap refuses** (§8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapRefused {
    /// [`MAX_EXCURSIONS_PER_OPERATOR`].
    Operator,
    /// [`MAX_EXCURSIONS_PER_SITE`].
    Site,
    /// [`MAX_EXCURSIONS_WORLD`].
    World,
}

impl CapRefused {
    /// The cap's figure.
    #[must_use]
    pub fn cap(self) -> u32 {
        match self {
            CapRefused::Operator => MAX_EXCURSIONS_PER_OPERATOR,
            CapRefused::Site => MAX_EXCURSIONS_PER_SITE,
            CapRefused::World => MAX_EXCURSIONS_WORLD,
        }
    }
}

/// **The caps, in order** (§8; lead ruling 8): the operator's first, then the site's, then
/// the world's; each refuses at or past its cap (a new launch would make one more).
#[must_use]
pub fn cap_refusal(counts: &ExcursionCounts) -> Option<CapRefused> {
    cap_refusal_at(counts, MAX_EXCURSIONS_PER_SITE)
}

/// **[`cap_refusal`] under a site cap of `site_cap`** (doc 32 cut CF.2; Q-CF-21): the
/// configured `[tacsim.field].max_units`, never above [`MAX_EXCURSIONS_PER_SITE`]. The command
/// refuses at the configured cap; the start's prepare and the fill plan keep the ceiling as their
/// backstop.
#[must_use]
pub fn cap_refusal_at(counts: &ExcursionCounts, site_cap: u32) -> Option<CapRefused> {
    let site_cap = site_cap.min(MAX_EXCURSIONS_PER_SITE);
    if counts.operator >= MAX_EXCURSIONS_PER_OPERATOR {
        Some(CapRefused::Operator)
    } else if counts.site >= site_cap {
        Some(CapRefused::Site)
    } else if counts.world >= MAX_EXCURSIONS_WORLD {
        Some(CapRefused::World)
    } else {
        None
    }
}

/// **The ramp as a site pose** (the brief's decision 3): a hull settled on a pad is `Direct`
/// at its domain's passive origin (`mush_sim`'s arrival), so its site position is by
/// convention the pad centre ([`PAD_CENTRE_CM`], both axes) facing north, and the ramp is the
/// bay's egress offset from it — `x = centre + egress.x_cm`, `y = centre + egress.y_cm`, the
/// heading the egress's, `z` 0, the passive posture every tacsim pose uses. The Dray-P's
/// stern ramp reads `(128_000, 124_500, 180_000)`. (mush-sim's `egress_parent` is not used:
/// it folds an egress pose into a posture at the origin.)
///
/// # Errors
///
/// The posture's own refusal, or an offset that leaves `i64`.
pub fn ramp_pose(bay: &BayDef) -> Result<mush_sim::data::Pose, String> {
    let egress = &bay.egress;
    let x = PAD_CENTRE_CM
        .checked_add(egress.x_cm)
        .ok_or_else(|| "the ramp's x leaves the site".to_owned())?;
    let y = PAD_CENTRE_CM
        .checked_add(egress.y_cm)
        .ok_or_else(|| "the ramp's y leaves the site".to_owned())?;
    let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
        .map_err(|error| error.to_string())?;
    Ok(mush_sim::data::Pose {
        x_cm: x,
        y_cm: y,
        z_cm: 0,
        heading_mdeg: egress.heading_mdeg % 360_000,
        posture,
    })
}

/// **A site cell's centre as a pose** (doc 20 cut SM.4b: where a drive places its rig):
/// `(x · 1000 + 500, y · 1000 + 500)`, `z` 0, the heading given (`% 360,000`), the passive
/// posture every tacsim pose uses ([`ramp_pose`]'s construction).
///
/// # Errors
///
/// The posture's own refusal, or a cell whose centre leaves `i64`.
pub fn cell_pose(cell: [i64; 2], heading_mdeg: u32) -> Result<mush_sim::data::Pose, String> {
    let side = i64::from(SITE_CELL_CM);
    let centre = |v: i64| {
        v.checked_mul(side)
            .and_then(|cm| cm.checked_add(side / 2))
            .ok_or_else(|| "the cell leaves the site".to_owned())
    };
    let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
        .map_err(|error| error.to_string())?;
    Ok(mush_sim::data::Pose {
        x_cm: centre(cell[0])?,
        y_cm: centre(cell[1])?,
        z_cm: 0,
        heading_mdeg: heading_mdeg % 360_000,
        posture,
    })
}

/// **The site cell a position stands in** (§3.1: 10 m cells; `div_euclid`, so a position
/// south or west of the origin never snaps toward it).
#[must_use]
pub fn site_cell(x_cm: i64, y_cm: i64) -> (i64, i64) {
    let cell = i64::from(SITE_CELL_CM);
    (x_cm.div_euclid(cell), y_cm.div_euclid(cell))
}

/// **Cells apart, Chebyshev** (the brief's decision 3: "within one cell" is Chebyshev on the
/// 10 m site grid).
#[must_use]
pub fn cells_apart(a: (i64, i64), b: (i64, i64)) -> u64 {
    a.0.abs_diff(b.0).max(a.1.abs_diff(b.1))
}

// ---- the projection ---------------------------------------------------------------------

/// **Where the projection says a rig is** (§8: `Bay | Out | Held | Parked`). SM.3b writes
/// `Bay` and `Out`; `Held` is phase 3's, `Parked` SM.11's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TvState {
    /// In its hull's bay.
    Bay,
    /// Out of it.
    Out,
    /// Promoted and held (phase 3).
    Held,
    /// **Parked where it stands** (doc 20 cut SM.11; the brief's lead ruling 6): a rig on the
    /// ground with nothing in flight whose charge cannot pay its way home — the idle return's
    /// recover refused `leg_short`, or a fill plan that ended `Stranded`. Staff bring it in. A
    /// plan's normal park is the bay ([`TvState::Bay`]).
    Parked,
}

/// **A fill plan as the projection shows it** (doc 20 cut SM.11; the brief's lead ruling 10): the
/// plan's number and state, the step it works and the step words from it, its deadline and, once
/// it has ended, the end's reason word — never a class, a tonnage or a reserve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanView {
    /// The rig's plan number.
    pub no: u32,
    /// `live`, `parking` or `ended`.
    pub state: String,
    /// The step it works, from 1 (`steps + 1` once the list is worked out).
    pub step: u16,
    /// Its steps.
    pub steps: u16,
    /// At most [`MAX_PLAN_VIEW_NEXT`] step words from the step it works (`mine C117`).
    pub next: Vec<String>,
    /// Its deadline, IC.
    pub deadline_ic: i64,
    /// Why it parked or ended, in a short word (`hold full`); skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

/// The most step words a [`PlanView`] carries.
pub const MAX_PLAN_VIEW_NEXT: usize = 4;

/// **What of a plan the operator has been told** (doc 20 cut SM.11; the brief's 3.8): the plan's
/// number and its `seq` at the last tell — never cleared on an operator change (clearing it would
/// re-tell, `told`'s rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTold {
    /// The plan's number.
    pub no: u32,
    /// Its `seq` told.
    pub seq: u32,
}

/// A pose on a site's grid, as the projection carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TvPose {
    /// East, centimetres.
    pub x_cm: i64,
    /// North, centimetres.
    pub y_cm: i64,
    /// Millidegrees clockwise from north.
    pub heading_mdeg: u32,
}

/// The act in flight, as the projection shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActView {
    /// Its ordinal.
    pub ordinal: u32,
    /// What it is.
    pub kind: ActKind,
    /// When it is due.
    pub due_ic: i64,
}

/// **The run in flight, as the projection shows it** (doc 20 cut SM.7; §8): the run as its
/// operator chose it — which pocket, which family, how many cycles, whether it is a blind core,
/// whether it stops after this cycle, when this cycle is due — **never** the pocket's class,
/// yield or tonnes (the operator chose the pocket and family and knows whether they had a
/// reading; nothing here is truth). **Doc 20 cut SM.9** (the brief's 3.3 item 7, under its lead
/// ruling 4: "the screen's rig status reads `run` (pocket, family, k, cycles, next due)") added
/// `addr`, `good`, `stop` and `blind`, reversing SM.7's rule that the view named no pocket and
/// no family; the quarry screen's rig status reads this view, never `RigRecord.run`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunView {
    /// The run's ordinal.
    pub ordinal: u32,
    /// The cycle in flight.
    pub k: u8,
    /// The run's cycles.
    pub cycles: u8,
    /// When the cycle in flight is due.
    pub due_ic: i64,
    /// **The pocket's address** (doc 20 cut SM.9), e.g. `C117`; empty only on a view written
    /// before SM.9 (skipped when empty).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub addr: String,
    /// **The family's ore good** the operator chose (doc 20 cut SM.9); absent only on a view
    /// written before SM.9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub good: Option<CanonicalId>,
    /// **`+tv/mine stop`**: the run closes after this cycle (doc 20 cut SM.9; skipped when
    /// `false`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stop: bool,
    /// **A blind core** (doc 20 cut SM.9): the operator had no reading of the pocket at the
    /// start (skipped when `false`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blind: bool,
}

/// The longest pocket address a run's view carries (`C117` is four bytes).
pub const MAX_RUN_ADDR_BYTES: usize = 8;

impl RunView {
    /// The view of `run`.
    #[must_use]
    pub fn of(run: &Run) -> RunView {
        RunView {
            ordinal: run.ordinal,
            k: run.k,
            cycles: run.cycles,
            due_ic: run.due_ic,
            addr: crate::surface::address::pocket_address(run.pocket[0], run.pocket[1])
                .unwrap_or_default(),
            good: Some(run.good.clone()),
            stop: run.stop,
            blind: run.blind,
        }
    }
}

/// One waypoint of a frozen leg (§8; doc 20 cut SM.4b fills it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegWaypoint {
    /// East, centimetres.
    pub x_cm: i64,
    /// North, centimetres.
    pub y_cm: i64,
    /// Cumulative IC seconds from the leg's start.
    pub at_s: u32,
}

/// **A frozen leg** (§8: at most [`MAX_LEG_WAYPOINTS`] waypoints with cumulative IC seconds;
/// positions between are pure interpolation). Doc 20 cut SM.4b projects a drive's into
/// [`TvRec::leg`] from its act's [`ActParams::Drive`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenLeg {
    /// The IC instant the leg starts.
    pub started_ic: i64,
    /// The waypoints.
    pub waypoints: Vec<LegWaypoint>,
}

/// One line of the projection's log, stamped with an IC instant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TvLogLine {
    /// The IC instant.
    pub at_ic: i64,
    /// The line, at most [`MAX_TV_LOG_BYTES`].
    pub text: String,
}

/// **`tacsim:tv:<tv>`** — the normal-lane projection (§8; lead ruling 1): bounded, whole
/// document, strict, written only by the `tacsim.surface` tick. It decides nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TvRec {
    /// [`TV_REC_SCHEMA`].
    pub schema: u16,
    /// The terrain vehicle: the key.
    pub tv: ObjId,
    /// Its bay slot.
    pub slot: u8,
    /// The operator of the last act.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<ObjId>,
    /// The hull of the last act.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<ObjId>,
    /// The open excursion's identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excursion: Option<ExcursionId>,
    /// Where it is.
    pub state: TvState,
    /// The site entry of the open excursion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<CanonicalId>,
    /// Its pose on the ground, when it stands there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<TvPose>,
    /// The act in flight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act: Option<ActView>,
    /// **The frozen leg of the act in flight** (doc 20 cut SM.4b; §8): a drive's (or a recover's
    /// way home) from the tick's first visit after its start until its placement, so clients
    /// interpolate the rig between waypoints; `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leg: Option<FrozenLeg>,
    /// The act ordinal whose placement the tick wrote (the idempotency key).
    pub applied: u32,
    /// The act ordinal whose line the operator has been told.
    pub told: u32,
    /// **The cycle of `told` whose line the operator has been told** (doc 20 cut SM.7; the brief's
    /// decision R3): 0 for an act's outcome. An outcome is untold iff `(last.ordinal, its cycle)`
    /// differs from `(told, told_cycle)` — equality, never `>`, since a run's cycles share its
    /// ordinal and may finish after a later scan's. Skipped at 0.
    #[serde(default, skip_serializing_if = "is_zero_cycle")]
    pub told_cycle: u8,
    /// Staff receipt already reported, independent of the current work ordinal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_told: Option<u32>,
    /// Pending staff receipt whose blocked placement has been reported once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_wait_told: Option<u32>,
    /// **The run in flight** (doc 20 cut SM.7), as [`RunView`] shows it; skipped when `None`. Boxed
    /// so [`TvRecState`] stays within clippy's `large_enum_variant` (the wire form is the view's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Box<RunView>>,
    /// The newest [`MAX_TV_LOG_LINES`] lines, oldest first.
    pub log: Vec<TvLogLine>,
    /// **The rig's fill plan, as [`PlanView`] shows it** (doc 20 cut SM.11; lead ruling 10);
    /// skipped when `None`, so a projection with no plan reads byte for byte as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanView>,
    /// **The plan notes told** (doc 20 cut SM.11); skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_told: Option<PlanTold>,
}

/// serde's `skip_serializing_if` for [`TvRec::told_cycle`].
#[allow(clippy::trivially_copy_pass_by_ref)] // serde passes a reference
const fn is_zero_cycle(cycle: &u8) -> bool {
    *cycle == 0
}

impl TvRec {
    /// A projection for `tv` in `slot` with nothing applied or told.
    #[must_use]
    pub fn new(tv: ObjId, slot: u8) -> TvRec {
        TvRec {
            schema: TV_REC_SCHEMA,
            tv,
            slot,
            operator: None,
            hull: None,
            excursion: None,
            state: TvState::Bay,
            site: None,
            pose: None,
            act: None,
            leg: None,
            applied: 0,
            told: 0,
            told_cycle: 0,
            recovery_told: None,
            recovery_wait_told: None,
            run: None,
            log: Vec::new(),
            plan: None,
            plan_told: None,
        }
    }

    /// **`tacsim:tv:{tv.0}`** — the one key of this projection.
    #[must_use]
    pub fn key(&self) -> SmolStr {
        tv_rec_key(self.tv)
    }

    /// Append one line stamped `at_ic`: newlines become spaces, the text is cut to
    /// [`MAX_TV_LOG_BYTES`] on a char boundary, and the oldest line is dropped while the ring
    /// is full.
    pub fn push_log(&mut self, at_ic: i64, text: &str) {
        let mut flat: String = text
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        if flat.len() > MAX_TV_LOG_BYTES {
            let mut cut = MAX_TV_LOG_BYTES;
            while cut > 0 && !flat.is_char_boundary(cut) {
                cut = cut.saturating_sub(1);
            }
            flat.truncate(cut);
        }
        while self.log.len() >= MAX_TV_LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(TvLogLine { at_ic, text: flat });
    }

    /// **Fit the projection under [`MAX_TV_REC_BYTES`]** (doc 20 cut SM.4b; the brief's decision
    /// P7): while its canonical JSON is past the ceiling, drop the oldest log line. The leg is
    /// never cut (doc 20 §8's 4 KiB stands; a 32-waypoint leg and a full sixteen-line log do not
    /// fit together). A record that will not serialise is left as it is (its `check` refuses).
    pub fn fit(&mut self) {
        while !self.log.is_empty()
            && serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() > MAX_TV_REC_BYTES)
        {
            self.log.remove(0);
        }
    }

    /// **The projection's bounds**: the schema; at most [`MAX_TV_LOG_LINES`] log lines, each
    /// at most [`MAX_TV_LOG_BYTES`] and on one row; a leg of at most [`MAX_LEG_WAYPOINTS`];
    /// a run of at most [`MAX_RUN_CYCLES`] cycles whose address is at most
    /// [`MAX_RUN_ADDR_BYTES`]; the canonical JSON at most [`MAX_TV_REC_BYTES`].
    /// (`told <= applied` is **not** required: a refused finish is told without a placement.)
    ///
    /// # Errors
    ///
    /// The first bound broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != TV_REC_SCHEMA {
            return Err(format!(
                "a rig projection is schema {TV_REC_SCHEMA}, not {}",
                self.schema
            ));
        }
        if self.log.len() > MAX_TV_LOG_LINES {
            return Err(format!(
                "a rig projection logs at most {MAX_TV_LOG_LINES} lines"
            ));
        }
        for line in &self.log {
            if line.text.len() > MAX_TV_LOG_BYTES {
                return Err(format!(
                    "a rig projection's log line is at most {MAX_TV_LOG_BYTES} bytes"
                ));
            }
            if line.text.contains(['\n', '\r']) {
                return Err("a rig projection's log line is one row".to_owned());
            }
        }
        if let Some(leg) = &self.leg
            && leg.waypoints.len() > MAX_LEG_WAYPOINTS
        {
            return Err(format!(
                "a frozen leg has at most {MAX_LEG_WAYPOINTS} waypoints"
            ));
        }
        // Doc 20 cut SM.7: a run's view names a cycle of at most eight.
        if let Some(run) = &self.run
            && (run.cycles == 0 || run.cycles > MAX_RUN_CYCLES || run.k == 0 || run.k > run.cycles)
        {
            return Err(format!(
                "a rig projection's run is a cycle of at most {MAX_RUN_CYCLES}"
            ));
        }
        // Doc 20 cut SM.9 (the judge's conformance/F11): the run's pocket address, its own
        // bound and its own sentence.
        if let Some(run) = &self.run
            && run.addr.len() > MAX_RUN_ADDR_BYTES
        {
            return Err(format!(
                "a rig projection's run address is at most {MAX_RUN_ADDR_BYTES} bytes"
            ));
        }
        if self.told_cycle > MAX_RUN_CYCLES {
            return Err(format!(
                "a rig projection's told cycle is at most {MAX_RUN_CYCLES}"
            ));
        }
        // Doc 20 cut SM.11 (lead ruling 10): the plan view's step words and end, bounded.
        if let Some(plan) = &self.plan
            && (plan.next.len() > MAX_PLAN_VIEW_NEXT
                || plan
                    .next
                    .iter()
                    .chain(plan.end.iter())
                    .chain(std::iter::once(&plan.state))
                    .any(|words| words.len() > MAX_TV_LOG_BYTES || words.contains(['\n', '\r'])))
        {
            return Err(format!(
                "a rig projection's plan shows at most {MAX_PLAN_VIEW_NEXT} steps, each one row of \
                 at most {MAX_TV_LOG_BYTES} bytes"
            ));
        }
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_TV_REC_BYTES {
            return Err(format!(
                "a rig projection is at most {MAX_TV_REC_BYTES} bytes"
            ));
        }
        Ok(())
    }
}

/// **`tacsim:tv:{tv.0}`** — the numeric id, never `ObjId`'s `#`-prefixed Display.
#[must_use]
pub fn tv_rec_key(tv: ObjId) -> SmolStr {
    SmolStr::new(format!("{TV_REC_PREFIX}{}", tv.0))
}

/// The inverse: the tail after [`TV_REC_PREFIX`] as a strict decimal `u32`.
#[must_use]
pub fn tv_rec_key_owner(key: &str) -> Option<ObjId> {
    key.strip_prefix(TV_REC_PREFIX)
        .and_then(crate::port::families::strict_decimal_u32)
        .map(ObjId)
}

/// **One rig's projection, as the store holds it.**
// Doc 20 cut SM.11: the plan view made `Present` the large variant; the store holds one per rig
// (at most `MAX_RIG_RECORDS`), read far more often than written, so it stays unboxed.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TvRecState {
    /// The document read.
    Present(TvRec),
    /// A key claims the projection but its body did not read: logged once, and it bars
    /// nothing (the projection decides nothing); the tick's next write replaces it.
    Unreadable,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery() -> BatteryDef {
        BatteryDef {
            capacity_cu: 100_000,
            bay_recharge_cu_per_ic_hour: 50_000,
            drive_cu_per_nominal_cell: 15,
            reserve_permille: 50,
        }
    }

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap_or_else(|_| panic!("{text} is canonical"))
    }

    /// The shipped battery row is the one these tests read.
    #[test]
    fn sm3b_the_test_battery_is_the_shipped_row() {
        let tools = crate::definitions::tools::tools_table().unwrap_or_else(|e| panic!("{e}"));
        let shipped = tools
            .values()
            .find_map(crate::definitions::tools::ToolDef::battery)
            .unwrap_or_else(|| panic!("a battery row"));
        assert_eq!(*shipped, battery());
    }

    /// `charge_at`: in the bay it grows 50,000 cu an IC hour to the cap; out of the bay it
    /// is flat; a negative elapsed reads as no time; an overflow saturates at the capacity.
    #[test]
    fn sm3b_charge_at_grows_in_the_bay_and_is_flat_out_of_it() {
        let b = battery();
        let bay = Charge {
            value_cu: 10_000,
            at_ic: 1_000,
            in_bay: true,
        };
        assert_eq!(charge_at(&bay, 1_000, &b), 10_000);
        assert_eq!(charge_at(&bay, 1_000 + 3_600, &b), 60_000);
        assert_eq!(charge_at(&bay, 1_000 + 1_800, &b), 35_000);
        assert_eq!(charge_at(&bay, 1_000 + 1, &b), 10_013, "floored per second");
        assert_eq!(charge_at(&bay, 1_000 + 2 * 3_600, &b), 100_000, "capped");
        assert_eq!(charge_at(&bay, 1_000 + 100 * 3_600, &b), 100_000);
        assert_eq!(charge_at(&bay, 0, &b), 10_000, "a negative elapsed is none");
        assert_eq!(
            charge_at(&bay, i64::MAX, &b),
            100_000,
            "an overflow saturates"
        );
        let out = Charge {
            in_bay: false,
            ..bay
        };
        assert_eq!(charge_at(&out, 1_000 + 10 * 3_600, &b), 10_000, "flat");
        assert_eq!(charge_at(&out, 0, &b), 10_000);
    }

    /// **A rig is never stranded by its battery** (§4.1; the SM.3b fixer round's item 3): only
    /// a launch or a spend reads the battery; a recover's and a dump's charge settle without it,
    /// and agree with the battery's own settle whenever it is there; leaving the bay does not
    /// settle without it; an unread fresh record reads full under any battery.
    #[test]
    fn sm3b_a_zero_spend_settles_without_the_battery() {
        assert!(
            needs_battery(ActKind::Launch, 0, 0),
            "a launch leaves the bay"
        );
        assert!(needs_battery(ActKind::Scan, 1_000, 0));
        assert!(needs_battery(ActKind::Sound, 2_000, 0));
        // Doc 20 cut SM.4b (lead ruling 5): a recover never reads the battery, even one that
        // spends its leg's charge home (`leg_affordable` checks it on the record's own value).
        assert!(
            !needs_battery(ActKind::Recover, 0, 1),
            "a recover spends home unread"
        );
        assert!(!needs_battery(ActKind::Recover, 0, 0));
        assert!(!needs_battery(ActKind::Dump, 0, 0));
        let b = battery();
        let ground = Charge {
            value_cu: 42_000,
            at_ic: 1_000,
            in_bay: false,
        };
        // A recover from the ground: the value stands, the bay's recharge starts at the stamp —
        // exactly what the battery's settle (`charge_at` at the stamp, nothing spent) writes.
        let home = settle_free(&ground, 5_000, true).expect("a recover settles");
        assert_eq!(
            home,
            Charge {
                value_cu: 42_000,
                at_ic: 5_000,
                in_bay: true
            }
        );
        assert_eq!(home.value_cu, charge_at(&ground, 5_000, &b));
        // A dump or a recover closing an excursion from the bay: unchanged, and so its value
        // later reads the same.
        let bay = Charge {
            in_bay: true,
            ..ground
        };
        assert_eq!(settle_free(&bay, 5_000, true), Some(bay));
        assert_eq!(settle_free(&ground, 5_000, false), Some(ground));
        assert_eq!(
            charge_at(&settle_free(&bay, 5_000, true).expect("settles"), 9_000, &b),
            charge_at(&bay, 9_000, &b)
        );
        // Leaving the bay settles the recharge: never without the battery.
        assert_eq!(settle_free(&bay, 5_000, false), None);
        // The unread fresh record reads full under any battery.
        let unread = RigRecord::fresh_unread(ObjId(7));
        assert!(unread.check().is_ok());
        assert_eq!(charge_at(&unread.charge, 0, &b), b.capacity_cu);
        assert_eq!(
            charge_at(&unread.charge, 1_000_000, &b),
            charge_at(&RigRecord::fresh(ObjId(7), &b).charge, 1_000_000, &b)
        );
    }

    /// **A ground reading settles without the battery** (doc 20 cut SM.4a's pre-commit round,
    /// item 2; §4.1): out of the bay `settle_spent_out_of_bay` is exactly what the battery's own
    /// settle writes (`charge_at` at the stamp — flat out of the bay — less the reservation,
    /// stamped, out of the bay), at any stamp; a charge in the bay never settles without the
    /// battery (the implied launch fails closed); a reservation past the value is refused.
    #[test]
    fn sm4a_a_ground_reading_settles_without_the_battery() {
        let b = battery();
        let ground = Charge {
            value_cu: 42_000,
            at_ic: 1_000,
            in_bay: false,
        };
        for (reserved, at) in [(1_000, 1_600), (2_000, 9_000), (42_000, 1_000)] {
            let settled =
                settle_spent_out_of_bay(&ground, reserved, at).expect("a ground reading settles");
            assert_eq!(
                settled,
                Charge {
                    value_cu: charge_at(&ground, at, &b) - reserved,
                    at_ic: at,
                    in_bay: false,
                }
            );
        }
        assert_eq!(settle_spent_out_of_bay(&ground, 42_001, 1_600), None);
        let bay = Charge {
            in_bay: true,
            ..ground
        };
        assert_eq!(settle_spent_out_of_bay(&bay, 1_000, 1_600), None);
    }

    /// `reserve_check` with the A7 numbers: two 60,000 reservations fit alone on 100,000
    /// and not together; 5,000 left fits, 4,999 does not; a zero spend always passes.
    #[test]
    fn sm3b_reserve_check_holds_the_five_per_cent_floor() {
        let b = battery();
        assert_eq!(
            reserve_check(100_000, 0, 60_000, 0, &b),
            Ok(ChargeAfter {
                after_act: 40_000,
                after_home: 40_000
            })
        );
        assert_eq!(
            reserve_check(100_000, 60_000, 60_000, 0, &b),
            Err(ReserveShort {
                after_home: -20_000,
                floor: 5_000
            }),
            "the summing arithmetic SM.7 relies on"
        );
        assert!(
            reserve_check(40_000, 0, 35_000, 0, &b).is_ok(),
            "5,000 left fits"
        );
        assert_eq!(
            reserve_check(5_000, 0, 1, 0, &b),
            Err(ReserveShort {
                after_home: 4_999,
                floor: 5_000
            })
        );
        assert_eq!(
            reserve_check(40_000, 0, 30_000, 5_001, &b),
            Err(ReserveShort {
                after_home: 4_999,
                floor: 5_000
            }),
            "the trip home counts"
        );
        assert_eq!(
            reserve_check(4_000, 0, 0, 0, &b),
            Ok(ChargeAfter {
                after_act: 4_000,
                after_home: 4_000
            }),
            "the zero-spend exemption"
        );
    }

    /// The caps refuse at each cap, in order (operator, site, world), and pass one below.
    #[test]
    fn sm3b_cap_refusal_at_each_cap_in_order() {
        let below = ExcursionCounts {
            operator: MAX_EXCURSIONS_PER_OPERATOR - 1,
            site: MAX_EXCURSIONS_PER_SITE - 1,
            world: MAX_EXCURSIONS_WORLD - 1,
        };
        assert_eq!(cap_refusal(&below), None);
        let operator = ExcursionCounts {
            operator: MAX_EXCURSIONS_PER_OPERATOR,
            ..below
        };
        assert_eq!(cap_refusal(&operator), Some(CapRefused::Operator));
        let site = ExcursionCounts {
            site: MAX_EXCURSIONS_PER_SITE,
            ..below
        };
        assert_eq!(cap_refusal(&site), Some(CapRefused::Site));
        let world = ExcursionCounts {
            world: MAX_EXCURSIONS_WORLD,
            ..below
        };
        assert_eq!(cap_refusal(&world), Some(CapRefused::World));
        let all = ExcursionCounts {
            operator: MAX_EXCURSIONS_PER_OPERATOR,
            site: MAX_EXCURSIONS_PER_SITE,
            world: MAX_EXCURSIONS_WORLD,
        };
        assert_eq!(
            cap_refusal(&all),
            Some(CapRefused::Operator),
            "operator first"
        );
        let site_world = ExcursionCounts {
            site: MAX_EXCURSIONS_PER_SITE,
            world: MAX_EXCURSIONS_WORLD,
            ..below
        };
        assert_eq!(
            cap_refusal(&site_world),
            Some(CapRefused::Site),
            "then site"
        );
        assert_eq!(CapRefused::Operator.cap(), 4);
        assert_eq!(CapRefused::Site.cap(), 24, "the field cap (doc 32 Q-CF-21)");
        assert_eq!(CapRefused::World.cap(), 1_024);
    }

    /// The Dray-P's stern ramp as a site pose, and the pad centre's derivation.
    #[test]
    fn sm3b_the_ramp_pose_of_the_dray_p_bay() {
        assert_eq!(
            PAD_CENTRE_CM,
            i64::from(crate::surface::ground::SITE_CELLS) / 2 * i64::from(SITE_CELL_CM),
            "the pad centre is the site's middle"
        );
        let catalog = crate::definitions::Catalog::build_fielded(&[], crate::NS)
            .unwrap_or_else(|e| panic!("{e}"));
        let unit = cid(crate::definitions::content::DRAY_P);
        let bay = catalog
            .specs
            .get(&unit)
            .and_then(|spec| spec.interior.as_ref())
            .and_then(|interior| interior.bay())
            .unwrap_or_else(|| panic!("the Dray-P has a bay"));
        let pose = ramp_pose(bay).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            (pose.x_cm, pose.y_cm, pose.z_cm, pose.heading_mdeg),
            (128_000, 124_500, 0, 180_000)
        );
        assert!(pose.validate().is_ok());
    }

    /// The site grid: the ramp cell, Chebyshev distance, `div_euclid` below the origin.
    #[test]
    fn sm3b_site_cells_are_ten_metres_and_apart_is_chebyshev() {
        assert_eq!(site_cell(128_000, 124_500), (128, 124));
        assert_eq!(site_cell(-1, -1_000), (-1, -1));
        assert_eq!(cells_apart((128, 124), (128, 124)), 0);
        assert_eq!(cells_apart((128, 124), (129, 125)), 1);
        assert_eq!(cells_apart((128, 124), (128, 122)), 2);
        assert_eq!(cells_apart((128, 124), (125, 123)), 3);
    }

    fn act(ordinal: u32, params: ActParams) -> Act {
        Act {
            ordinal,
            kind: ActKind::Launch,
            operator: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            site: cid("ahuna_port"),
            started_ic: 1_000,
            due_ic: 1_300,
            reserved_cu: 0,
            params,
            by: None,
            seal: None,
        }
    }

    fn sample() -> RigRecord {
        RigRecord {
            schema: EXCURSION_SCHEMA,
            tv: ObjId(900),
            ordinal: 3,
            charge: Charge {
                value_cu: 90_000,
                at_ic: 900,
                in_bay: false,
            },
            act: Some(act(3, ActParams::None {})),
            excursion: Some(Excursion {
                id: ExcursionId {
                    tv: ObjId(900),
                    launch_ic: 1_000,
                },
                operator: ObjId(42),
                hull: ObjId(500),
                slot: 1,
                site: cid("ahuna_port"),
            }),
            last: Some(Outcome {
                ordinal: 2,
                kind: ActKind::Recover,
                operator: ObjId(42),
                slot: 1,
                site: cid("ahuna_port"),
                at_ic: 800,
                result: ActResult::Refused {
                    code: RefusedCode::NotBeside,
                },
                moved: BTreeMap::new(),
                span: None,
                arrived: None,
                mined: None,
                stays: BTreeMap::new(),
                by: None,
            }),
            run: None,
            queued: None,
            plan: None,
            plans: 0,
            recovery: None,
            sortie: None,
            field_crew: None,
            field_work: None,
        }
    }

    /// `RigRecord::check` refuses each broken invariant; `fresh` is full and empty.
    #[test]
    fn sm3b_the_rig_record_checks_each_invariant() {
        assert_eq!(sample().check(), Ok(()));
        let fresh = RigRecord::fresh(ObjId(7), &battery());
        assert_eq!(fresh.check(), Ok(()));
        assert_eq!(fresh.charge.value_cu, 100_000);
        assert!(fresh.charge.in_bay && fresh.act.is_none() && fresh.ordinal == 0);
        assert_eq!(fresh.key(), RigKey(ObjId(7)));
        let mut broken = Vec::new();
        broken.push(RigRecord {
            schema: 2,
            ..sample()
        });
        broken.push(RigRecord {
            act: Some(act(2, ActParams::None {})),
            ..sample()
        });
        let mut early = act(3, ActParams::None {});
        early.due_ic = 999;
        broken.push(RigRecord {
            act: Some(early),
            ..sample()
        });
        let mut slot = act(3, ActParams::None {});
        slot.slot = 0;
        broken.push(RigRecord {
            act: Some(slot),
            ..sample()
        });
        let mut slot = act(3, ActParams::None {});
        slot.slot = 5;
        broken.push(RigRecord {
            act: Some(slot),
            ..sample()
        });
        broken.push(RigRecord {
            act: Some(act(
                3,
                ActParams::Dump {
                    hopper: 1,
                    spoil: 2,
                    goods: BTreeMap::new(),
                },
            )),
            ..sample()
        });
        broken.push(RigRecord {
            act: Some(act(
                3,
                ActParams::Dump {
                    hopper: 1,
                    spoil: 2,
                    goods: [(cid("metal_ore"), 0)].into_iter().collect(),
                },
            )),
            ..sample()
        });
        broken.push(RigRecord {
            act: Some(act(
                3,
                ActParams::Dump {
                    hopper: 1,
                    spoil: 2,
                    goods: (0..9).map(|n| (cid(&format!("ore_{n}")), 1)).collect(),
                },
            )),
            ..sample()
        });
        let mut other = sample();
        if let Some(excursion) = other.excursion.as_mut() {
            excursion.id.tv = ObjId(901);
        }
        broken.push(other);
        let mut late = sample();
        if let Some(last) = late.last.as_mut() {
            last.ordinal = 4;
        }
        broken.push(late);
        for record in broken {
            assert!(record.check().is_err(), "{record:?} passed its check");
        }
    }

    /// Every kind, every parameter shape and every result round-trips the strict wire.
    #[test]
    fn sm3b_the_record_wire_is_strict_and_round_trips() {
        for kind in [
            ActKind::Launch,
            ActKind::Recover,
            ActKind::Dump,
            ActKind::Scan,
            ActKind::Sound,
            ActKind::Drive,
            ActKind::Mine,
            ActKind::Unload,
        ] {
            let wire = serde_json::to_value(kind).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(wire, serde_json::json!(kind.word()));
            assert_eq!(ActKind::from_word(kind.word()), Some(kind));
            let back: ActKind = serde_json::from_value(wire).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, kind);
        }
        for params in [
            ActParams::None {},
            ActParams::Dump {
                hopper: 7,
                spoil: 8,
                goods: [(cid("metal_ore"), 3)].into_iter().collect(),
            },
            scan_params(),
            sound_params(),
            drive_params(true),
            drive_params(false),
        ] {
            let wire = serde_json::to_value(&params).unwrap_or_else(|e| panic!("{e}"));
            let back: ActParams =
                serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, params, "{wire}");
        }
        for result in [
            ActResult::Done,
            ActResult::Refused {
                code: RefusedCode::NeverLeft,
            },
            ActResult::Refused {
                code: RefusedCode::NotBeside,
            },
            ActResult::Refused {
                code: RefusedCode::HopperChanged,
            },
            ActResult::Refused {
                code: RefusedCode::Moved,
            },
        ] {
            let wire = serde_json::to_value(result).unwrap_or_else(|e| panic!("{e}"));
            let back: ActResult = serde_json::from_value(wire).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, result);
        }
        let record = sample();
        let wire = serde_json::to_value(&record).unwrap_or_else(|e| panic!("{e}"));
        let back: RigRecord =
            serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, record);
        let mut extra = wire.clone();
        extra["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<RigRecord>(extra).is_err());
        let mut extra = wire;
        extra["act"]["params"]["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<RigRecord>(extra).is_err());
        let fresh = serde_json::to_value(RigRecord::fresh(ObjId(7), &battery()))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(fresh.get("act").is_none() && fresh.get("excursion").is_none());
        assert_eq!(
            ExcursionId {
                tv: ObjId(900),
                launch_ic: 1_000
            }
            .to_string(),
            "900@1000"
        );
    }

    fn scan_params() -> ActParams {
        ActParams::Scan {
            from_bay: true,
            body: cid("ceres"),
            cell: [128, 124],
            footing: Footing::Regolith,
            fields: vec![27, 28, 29, 19, 20, 21, 11, 12],
            hidden: vec![13],
        }
    }

    fn sound_params() -> ActParams {
        ActParams::Sound {
            from_bay: false,
            body: cid("ceres"),
            cell: [128, 124],
            radius: 16,
            quiet: false,
        }
    }

    /// A drive's leg (doc 20 cut SM.4b): from the ramp pose to `C117`'s cell, three waypoints.
    fn drive_params(from_bay: bool) -> ActParams {
        ActParams::Drive {
            from_bay,
            target: "C117".to_owned(),
            to: [146, 226],
            waypoints: vec![
                LegWaypoint {
                    x_cm: 128_000,
                    y_cm: 124_500,
                    at_s: 0,
                },
                LegWaypoint {
                    x_cm: 140_500,
                    y_cm: 180_500,
                    at_s: 400,
                },
                LegWaypoint {
                    x_cm: 146_500,
                    y_cm: 226_500,
                    at_s: 640,
                },
            ],
            secs: 640,
            cost: 19_200,
            metres: 1_040,
            heading_mdeg: 45_000,
            side: crate::surface::ground::SITE_CELLS,
        }
    }

    /// **A drive record checks its leg** (doc 20 cut SM.4b): a drive with its leg and a recover
    /// with a leg home pass; a leg of one waypoint or thirty-three, running back in time, ending
    /// off its own seconds, off the site, with a long target or a full-circle heading, a drive
    /// with another kind's parameters, a recover whose leg implies a launch, and an `arrived`
    /// on anything but a finished drive are each refused.
    #[test]
    fn sm4b_a_drive_record_checks_its_leg() {
        let with = |kind: ActKind, params: ActParams| {
            let mut record = sample();
            record.act = Some(Act {
                kind,
                params,
                ..act(3, ActParams::None {})
            });
            record
        };
        assert_eq!(with(ActKind::Drive, drive_params(true)).check(), Ok(()));
        assert_eq!(with(ActKind::Drive, drive_params(false)).check(), Ok(()));
        assert_eq!(with(ActKind::Recover, drive_params(false)).check(), Ok(()));
        assert_eq!(
            drive_params(true).leg().map(|(w, to)| (w.len(), to)),
            Some((3, [146, 226]))
        );
        assert!(drive_params(true).launches());
        assert!(!drive_params(false).launches());
        assert_eq!(drive_params(true).stand(), None);
        let edit = |change: &dyn Fn(&mut ActParams)| {
            let mut params = drive_params(false);
            change(&mut params);
            with(ActKind::Drive, params)
        };
        let mut broken = vec![
            edit(&|p| {
                if let ActParams::Drive { waypoints, .. } = p {
                    waypoints.truncate(1);
                }
            }),
            edit(&|p| {
                if let ActParams::Drive {
                    waypoints, secs, ..
                } = p
                {
                    *waypoints = (0..33_u32)
                        .map(|n| LegWaypoint {
                            x_cm: 128_000,
                            y_cm: 124_500,
                            at_s: n,
                        })
                        .collect();
                    *secs = 32;
                }
            }),
            edit(&|p| {
                if let ActParams::Drive { waypoints, .. } = p
                    && let Some(second) = waypoints.get_mut(1)
                {
                    second.at_s = 700;
                }
            }),
            edit(&|p| {
                if let ActParams::Drive { secs, .. } = p {
                    *secs = 641;
                }
            }),
            edit(&|p| {
                if let ActParams::Drive { to, .. } = p {
                    *to = [256, 0];
                }
            }),
            edit(&|p| {
                if let ActParams::Drive { target, .. } = p {
                    *target = "x".repeat(MAX_DRIVE_TARGET_BYTES + 1);
                }
            }),
            edit(&|p| {
                if let ActParams::Drive { heading_mdeg, .. } = p {
                    *heading_mdeg = 360_000;
                }
            }),
            with(ActKind::Drive, ActParams::None {}),
            with(ActKind::Scan, drive_params(false)),
            with(ActKind::Recover, drive_params(true)),
        ];
        // `arrived` only on a finished drive, on the site.
        let arrived = |kind: ActKind, result: ActResult, cell: [i64; 2]| {
            let mut record = sample();
            if let Some(last) = record.last.as_mut() {
                last.kind = kind;
                last.result = result;
                last.arrived = Some(cell);
            }
            record
        };
        assert_eq!(
            arrived(ActKind::Drive, ActResult::Done, [146, 226]).check(),
            Ok(())
        );
        broken.push(arrived(ActKind::Scan, ActResult::Done, [146, 226]));
        broken.push(arrived(
            ActKind::Drive,
            ActResult::Refused {
                code: RefusedCode::Moved,
            },
            [146, 226],
        ));
        broken.push(arrived(ActKind::Drive, ActResult::Done, [-1, 3]));
        for record in broken {
            assert!(record.check().is_err(), "{record:?} passed its check");
        }
    }

    /// **A recover never needs the battery** (doc 20 cut SM.4b; lead ruling 5, decision P6): not
    /// even with a leg's charge to spend; a drive does; and a leg is affordable out of the bay
    /// exactly when the value covers the reservation and the cost — never the 5 % floor — and
    /// always from the bay.
    #[test]
    fn sm4b_a_recover_never_needs_the_battery() {
        assert!(!needs_battery(ActKind::Recover, 4_500, 0));
        assert!(!needs_battery(ActKind::Recover, 0, 0));
        assert!(needs_battery(ActKind::Drive, 2_250, 0));
        assert!(needs_battery(ActKind::Drive, 0, 900));
        assert!(needs_battery(ActKind::Launch, 0, 0));
        let out = Charge {
            value_cu: 3_000,
            at_ic: 10,
            in_bay: false,
        };
        assert!(leg_affordable(&out, 0, 3_000), "exactly the leg fits");
        assert!(!leg_affordable(&out, 0, 3_001));
        assert!(!leg_affordable(&out, 1, 3_000), "the reservations count");
        assert!(leg_affordable(&out, 0, 0));
        let bay = Charge {
            in_bay: true,
            ..out
        };
        assert!(leg_affordable(&bay, 0, u32::MAX));
    }

    /// **A full leg and a full log fit** (doc 20 cut SM.4b; the brief's decision P7): 32
    /// waypoints at six-digit coordinates and sixteen lines of 160 bytes run past 4 KiB; `fit`
    /// drops the oldest lines until it fits, keeps the newest, and never cuts the leg.
    #[test]
    fn sm4b_a_full_leg_and_a_full_log_fit() {
        let mut rec = TvRec::new(ObjId(900), 1);
        rec.leg = Some(FrozenLeg {
            started_ic: 123_456_789,
            waypoints: (0..32_i64)
                .map(|n| LegWaypoint {
                    x_cm: 255_500 - n * 1_000,
                    y_cm: 200_500 + n * 1_000,
                    at_s: u32::try_from(n * 30).unwrap_or(0),
                })
                .collect(),
        });
        for n in 0..16_i64 {
            rec.push_log(1_000_000 + n, &format!("{n:02}{}", "x".repeat(158)));
        }
        let size = |rec: &TvRec| serde_json::to_vec(rec).map(|b| b.len()).unwrap_or(0);
        assert!(size(&rec) > MAX_TV_REC_BYTES, "{}", size(&rec));
        assert!(rec.check().is_err());
        let leg = rec.leg.clone();
        rec.fit();
        assert_eq!(rec.check(), Ok(()));
        assert!(size(&rec) <= MAX_TV_REC_BYTES);
        assert_eq!(rec.leg, leg, "the leg is whole");
        assert!(rec.log.len() < MAX_TV_LOG_LINES && !rec.log.is_empty());
        assert_eq!(
            rec.log.last().map(|l| l.at_ic),
            Some(1_000_015),
            "the newest line is kept"
        );
        // Dropping one line fewer would not fit: `fit` drops only what it must.
        let kept = rec.log.len();
        assert!(kept < MAX_TV_LOG_LINES);
        let mut one_more = rec.clone();
        one_more.log.insert(
            0,
            TvLogLine {
                at_ic: 1_000_000,
                text: format!("00{}", "x".repeat(158)),
            },
        );
        assert!(size(&one_more) > MAX_TV_REC_BYTES);
    }

    /// **A scan's and a sounding's frozen reach is bounded** (doc 20 cut SM.4a): a stand off
    /// the site, no fields or ten, a repeated or invalid index, a field both seen and hidden,
    /// a reach of 0 or 49, a quiet sounding heard, a kind with another kind's parameters and a
    /// span on another kind's outcome are each refused; the shapes in bounds pass.
    #[test]
    fn sm4a_the_reading_acts_are_bounded() {
        let mut ok = sample();
        ok.act = Some(Act {
            kind: ActKind::Scan,
            params: scan_params(),
            ..act(3, ActParams::None {})
        });
        assert_eq!(ok.check(), Ok(()));
        let scan = |edit: &dyn Fn(&mut ActParams)| {
            let mut record = ok.clone();
            if let Some(act) = record.act.as_mut() {
                edit(&mut act.params);
            }
            record
        };
        let mut broken = vec![
            scan(&|p| {
                if let ActParams::Scan { cell, .. } = p {
                    *cell = [256, 0];
                }
            }),
            scan(&|p| {
                if let ActParams::Scan { fields, .. } = p {
                    fields.clear();
                }
            }),
            scan(&|p| {
                if let ActParams::Scan { fields, .. } = p {
                    *fields = (0..10).collect();
                }
            }),
            scan(&|p| {
                if let ActParams::Scan { fields, .. } = p {
                    fields.push(27);
                }
            }),
            scan(&|p| {
                if let ActParams::Scan { fields, .. } = p {
                    fields.push(64);
                }
            }),
            scan(&|p| {
                if let ActParams::Scan { hidden, .. } = p {
                    hidden.push(27);
                }
            }),
        ];
        let mut sound = ok.clone();
        if let Some(act) = sound.act.as_mut() {
            act.kind = ActKind::Sound;
            act.params = sound_params();
        }
        assert_eq!(sound.check(), Ok(()));
        for radius in [0, 49] {
            let mut bad = sound.clone();
            if let Some(Act {
                params: ActParams::Sound { radius: r, .. },
                ..
            }) = bad.act.as_mut()
            {
                *r = radius;
            }
            broken.push(bad);
        }
        let mut mismatched = sound.clone();
        if let Some(act) = mismatched.act.as_mut() {
            act.kind = ActKind::Scan;
        }
        broken.push(mismatched);
        let mut heard_quiet = sample();
        if let Some(last) = heard_quiet.last.as_mut() {
            last.kind = ActKind::Sound;
            last.result = ActResult::Done;
            last.span = Some(ReadSpan::Sound {
                cell: [128, 124],
                radius: 8,
                quiet: true,
                heard: true,
            });
        }
        broken.push(heard_quiet.clone());
        if let Some(last) = heard_quiet.last.as_mut() {
            last.span = Some(ReadSpan::Sound {
                cell: [128, 124],
                radius: 8,
                quiet: true,
                heard: false,
            });
        }
        assert_eq!(heard_quiet.check(), Ok(()));
        let mut wrong_kind = heard_quiet.clone();
        if let Some(last) = wrong_kind.last.as_mut() {
            last.kind = ActKind::Scan;
        }
        broken.push(wrong_kind);
        for record in broken {
            assert!(record.check().is_err(), "{record:?} passed its check");
        }
        // The span round-trips strictly.
        let span = ReadSpan::Scan {
            cell: [1, 2],
            footing: Footing::Rock,
            fields: vec![0],
            hidden: Vec::new(),
        };
        let wire = serde_json::to_value(&span).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(wire["kind"], "scan");
        let back: ReadSpan = serde_json::from_value(wire).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, span);
    }

    /// **An implied launch settles at the launch half** (doc 20 cut SM.4a; the brief's reviewer
    /// finding F1, D22): in the bay under capacity, `charge_for_act` reads the recharge up to
    /// `now + LAUNCH_S` and no further, out of the bay `charge_at` now; `charge_shown` holds an
    /// in-flight implied launch's bay credit at `started + LAUNCH_S` while an ordinary act
    /// (and a scan from the ground) reads `charge_at` now.
    #[test]
    fn sm4a_an_implied_launch_settles_at_the_launch_half() {
        let b = battery();
        let c = Charge {
            value_cu: 20_000,
            at_ic: 1_000,
            in_bay: true,
        };
        let now = 1_000;
        let launched = charge_at(&c, now + i64::from(LAUNCH_S), &b);
        assert_eq!(launched, 20_000 + 300 * 50_000 / 3_600);
        assert_eq!(charge_for_act(&c, now, true, &b), launched);
        assert_eq!(charge_for_act(&c, now, false, &b), 20_000);
        let out = Charge { in_bay: false, ..c };
        assert_eq!(
            charge_for_act(&out, now, true, &b),
            20_000,
            "out of the bay"
        );
        // While the implied launch's scan is in flight: never past the launch half.
        let mut record = RigRecord::fresh(ObjId(7), &b);
        record.charge = c;
        record.ordinal = 1;
        record.act = Some(Act {
            ordinal: 1,
            kind: ActKind::Scan,
            started_ic: now,
            due_ic: now + 900,
            reserved_cu: 1_000,
            params: scan_params(),
            ..act(1, ActParams::None {})
        });
        assert_eq!(
            charge_shown(&record, now + 100, &b),
            charge_at(&c, now + 100, &b)
        );
        assert_eq!(charge_shown(&record, now + 900, &b), launched);
        assert!(charge_shown(&record, now + 900, &b) < charge_at(&c, now + 900, &b));
        // A scan from the ground reads `charge_at` now.
        if let Some(act) = record.act.as_mut() {
            act.params = ActParams::Scan {
                from_bay: false,
                body: cid("ceres"),
                cell: [128, 124],
                footing: Footing::Rock,
                fields: vec![28],
                hidden: Vec::new(),
            };
        }
        assert_eq!(
            charge_shown(&record, now + 900, &b),
            charge_at(&c, now + 900, &b)
        );
    }

    /// The key's text is the TV's decimal id and its parse is strict.
    #[test]
    fn sm3b_the_rig_key_is_strict_decimal() {
        assert_eq!(RigKey(ObjId(900)).key_text(), "900");
        assert_eq!(RigKey::parse("900"), Some(RigKey(ObjId(900))));
        assert_eq!(RigKey::parse("0"), Some(RigKey(ObjId(0))));
        for bad in ["0900", "#900", "+900", "9 00", "", "4294967296", "a"] {
            assert_eq!(RigKey::parse(bad), None, "{bad:?}");
        }
        assert_eq!(tv_rec_key(ObjId(900)).as_str(), "tacsim:tv:900");
        assert_eq!(tv_rec_key_owner("tacsim:tv:900"), Some(ObjId(900)));
        assert_eq!(tv_rec_key_owner("tacsim:tvlease:900"), None);
    }

    /// `TvRec::check` refuses seventeen lines, an over-long line, a record past 4 KiB and a
    /// 33-waypoint leg; `push_log` keeps the newest sixteen, cut on a char boundary.
    #[test]
    fn sm3b_the_projection_is_bounded_and_its_log_is_a_ring() {
        let mut rec = TvRec::new(ObjId(900), 1);
        assert_eq!(rec.check(), Ok(()));
        for n in 0..20_i64 {
            rec.push_log(n, &format!("line {n}"));
        }
        assert_eq!(rec.log.len(), MAX_TV_LOG_LINES);
        assert_eq!(rec.log.first().map(|l| l.at_ic), Some(4));
        assert_eq!(rec.log.last().map(|l| l.text.as_str()), Some("line 19"));
        assert_eq!(rec.check(), Ok(()));
        let mut long = TvRec::new(ObjId(900), 1);
        long.push_log(0, &"é".repeat(200));
        let text = long.log.first().map(|l| l.text.clone()).unwrap_or_default();
        assert!(text.len() <= MAX_TV_LOG_BYTES && text.len() >= MAX_TV_LOG_BYTES - 1);
        long.push_log(1, "two\nrows");
        assert_eq!(long.log.last().map(|l| l.text.as_str()), Some("two rows"));
        assert_eq!(long.check(), Ok(()));

        let mut seventeen = rec.clone();
        seventeen.log.push(TvLogLine {
            at_ic: 99,
            text: "extra".to_owned(),
        });
        assert!(seventeen.check().is_err(), "seventeen lines");
        let mut over = TvRec::new(ObjId(900), 1);
        over.log.push(TvLogLine {
            at_ic: 0,
            text: "x".repeat(MAX_TV_LOG_BYTES + 1),
        });
        assert!(over.check().is_err(), "an over-long line");
        let mut leg = TvRec::new(ObjId(900), 1);
        leg.leg = Some(FrozenLeg {
            started_ic: 0,
            waypoints: (0..33)
                .map(|n| LegWaypoint {
                    x_cm: n,
                    y_cm: n,
                    at_s: u32::try_from(n).unwrap_or(0),
                })
                .collect(),
        });
        assert!(leg.check().is_err(), "33 waypoints");
        // A record of exactly 4,097 canonical bytes: sixteen full lines and a full leg,
        // then the lines shortened one byte at a time to the edge.
        let mut big = TvRec::new(ObjId(900), 1);
        big.leg = Some(FrozenLeg {
            started_ic: 0,
            waypoints: (0..32)
                .map(|n| LegWaypoint {
                    x_cm: i64::MAX - n,
                    y_cm: i64::MAX - n,
                    at_s: u32::MAX,
                })
                .collect(),
        });
        for n in 0..16_i64 {
            big.push_log(i64::MAX - n, &"x".repeat(MAX_TV_LOG_BYTES));
        }
        let size = |rec: &TvRec| serde_json::to_vec(rec).map(|b| b.len()).unwrap_or(0);
        let mut line = 0_usize;
        while size(&big) > MAX_TV_REC_BYTES + 1 {
            if let Some(entry) = big.log.get_mut(line % MAX_TV_LOG_LINES) {
                entry.text.pop();
            }
            line += 1;
        }
        assert_eq!(
            size(&big),
            MAX_TV_REC_BYTES + 1,
            "the fixture is 4,097 bytes"
        );
        assert!(big.check().is_err(), "a 4,097-byte record");
        if let Some(entry) = big.log.first_mut() {
            entry.text.pop();
        }
        assert_eq!(big.check(), Ok(()), "4,096 bytes fit");
        let wire = serde_json::to_value(&rec).unwrap_or_else(|e| panic!("{e}"));
        let back: TvRec = serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, rec);
        let mut extra = wire;
        extra["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<TvRec>(extra).is_err());
    }

    /// A run of four fair metal cycles at `C`-block's pocket under `[146, 226]`, cycle 1.
    fn run_sample(ordinal: u32) -> Run {
        let stand = [146_i64, 226];
        let (px, py) = crate::surface::address::pocket_of_cell((stand[0], stand[1]))
            .unwrap_or_else(|| panic!("a pocket"));
        Run {
            ordinal,
            operator: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            site: cid("ahuna_port"),
            body: cid("ceres"),
            stand,
            pocket: [px, py],
            number: crate::surface::address::pocket_number(px, py)
                .unwrap_or_else(|| panic!("a number")),
            letter: crate::surface::address::block_letter(px, py)
                .unwrap_or_else(|| panic!("a letter")),
            family: crate::surface::geology::OreFamily::Metals,
            deposit: mush_industry::OreDepositId(77),
            good: cid("metal_ore"),
            source: Some(12),
            hopper: 13,
            cycles: 4,
            k: 1,
            secs: 1_800,
            cost_cu: 2_000,
            reserve_t: vec![2, 2, 2, 1],
            class_t: 2,
            pocket_t: 7,
            blind: false,
            stop: false,
            lapses_ic: Some(9_000_000),
            lock_rows: vec![LockRow {
                deposit: mush_industry::OreDepositId(77),
                pocket: crate::surface::address::pocket_number(px, py)
                    .unwrap_or_else(|| panic!("a number")),
            }],
            due_ic: 2_800,
            reserved_cu: 8_000,
            by: None,
        }
    }

    /// A record holding `run_sample(3)` in its cycle slot, the act slot empty.
    fn running() -> RigRecord {
        RigRecord {
            act: None,
            run: Some(run_sample(3)),
            ..sample()
        }
    }

    /// **A run record checks its bounds** (doc 20 cut SM.7; the brief's 3.1): the sample passes;
    /// a scan act beside the run passes; a drive act beside it, an act no later than the run, a
    /// `Mine` act, a run ordinal past the record's, k past the cycles, nine cycles, reserves of the
    /// wrong length or past 24 t, a blind run of two cycles, a pocket not under the stand, a wrong
    /// pocket number or letter, a source on a barren run (and none on an ore run), the remaining
    /// reservation wrong, three lock rows or one of another pocket are each refused; a mining
    /// outcome carries `mined` and only it does.
    #[test]
    fn sm7_a_run_record_checks_its_bounds() {
        assert_eq!(running().check(), Ok(()));
        let with_act = |kind: ActKind, params: ActParams, ordinal: u32| {
            let mut record = running();
            record.ordinal = ordinal;
            record.act = Some(Act {
                kind,
                params,
                ..act(ordinal, ActParams::None {})
            });
            record
        };
        assert_eq!(
            with_act(ActKind::Scan, scan_params(), 4).check(),
            Ok(()),
            "a scan beside the run"
        );
        assert_eq!(with_act(ActKind::Sound, sound_params(), 4).check(), Ok(()));
        let mut broken = vec![
            with_act(ActKind::Drive, drive_params(false), 4),
            with_act(ActKind::Scan, scan_params(), 3),
            with_act(ActKind::Mine, ActParams::None {}, 4),
        ];
        let edit = |change: &dyn Fn(&mut Run)| {
            let mut record = running();
            if let Some(run) = record.run.as_mut() {
                change(run);
            }
            record
        };
        broken.push(edit(&|r| r.ordinal = 4));
        broken.push(edit(&|r| r.k = 5));
        broken.push(edit(&|r| r.k = 0));
        broken.push(edit(&|r| {
            r.cycles = 9;
            r.reserve_t = vec![1; 9];
            r.reserved_cu = 18_000;
        }));
        broken.push(edit(&|r| r.reserve_t = vec![2, 2, 2]));
        broken.push(edit(&|r| r.reserve_t = vec![25, 2, 2, 1]));
        broken.push(edit(&|r| r.pocket_t = 25));
        broken.push(edit(&|r| {
            r.blind = true;
            r.cycles = 2;
            r.reserve_t = vec![3, 3];
            r.reserved_cu = 4_000;
        }));
        broken.push(edit(&|r| r.pocket[0] += 1));
        broken.push(edit(&|r| r.number += 1));
        broken.push(edit(&|r| r.letter = 'P'));
        broken.push(edit(&|r| r.source = None));
        broken.push(edit(&|r| {
            r.class_t = 0;
            r.pocket_t = 0;
        }));
        broken.push(edit(&|r| r.reserved_cu = 6_000));
        broken.push(edit(&|r| {
            let row = r.lock_rows[0];
            r.lock_rows = vec![row, row, row];
        }));
        broken.push(edit(&|r| r.lock_rows[0].pocket += 1));
        broken.push(edit(&|r| r.stand = [256, 0]));
        for record in broken {
            assert!(record.check().is_err(), "{record:?} passed its check");
        }
        // A blind barren core passes: one cycle, the blind room, no source.
        let blind = edit(&|r| {
            r.blind = true;
            r.cycles = 1;
            r.reserve_t = vec![3];
            r.reserved_cu = 2_000;
            r.class_t = 0;
            r.pocket_t = 0;
            r.source = None;
            r.lock_rows = Vec::new();
        });
        assert_eq!(blind.check(), Ok(()));
        // The mining outcome.
        let outcome = |kind: ActKind, mined: Option<Mined>| {
            let mut record = running();
            if let Some(last) = record.last.as_mut() {
                last.kind = kind;
                last.result = ActResult::Done;
                last.mined = mined;
            }
            record
        };
        let mined = Mined {
            k: 2,
            cycles: 4,
            pocket: [36, 56],
            good: cid("metal_ore"),
            tonnes: 2,
            blind: false,
            end: None,
        };
        assert_eq!(outcome(ActKind::Mine, Some(mined.clone())).check(), Ok(()));
        assert!(outcome(ActKind::Mine, None).check().is_err());
        assert!(outcome(ActKind::Scan, Some(mined.clone())).check().is_err());
        assert!(
            outcome(
                ActKind::Mine,
                Some(Mined {
                    k: 5,
                    ..mined.clone()
                })
            )
            .check()
            .is_err()
        );
        assert!(
            outcome(
                ActKind::Mine,
                Some(Mined {
                    tonnes: 25,
                    ..mined
                })
            )
            .check()
            .is_err()
        );
    }

    /// **`reserved_total` adds the run** (doc 20 cut SM.7; lead ruling 1): the act's reservation
    /// plus the run's remaining one; either alone; an overflow saturates.
    #[test]
    fn sm7_reserved_total_adds_the_run() {
        let mut record = running();
        assert_eq!(reserved_total(&record), 8_000, "the run alone");
        record.ordinal = 4;
        record.act = Some(Act {
            kind: ActKind::Scan,
            reserved_cu: 1_000,
            params: scan_params(),
            ..act(4, ActParams::None {})
        });
        assert_eq!(reserved_total(&record), 9_000, "a scan beside the run");
        record.run = None;
        assert_eq!(reserved_total(&record), 1_000, "the act alone");
        let mut huge = running();
        huge.act = Some(Act {
            reserved_cu: u32::MAX,
            ..act(3, ActParams::None {})
        });
        assert_eq!(reserved_total(&huge), u32::MAX, "saturates");
        assert_eq!(run_sample(3).outstanding_t(), 7);
        let later = Run {
            k: 3,
            reserved_cu: 4_000,
            ..run_sample(3)
        };
        assert_eq!(later.outstanding_t(), 3, "the cycle in flight included");
    }

    /// **The run and the mined outcome round-trip the strict wire** (doc 20 cut SM.7): a record with
    /// a run, an outcome with `mined` and every `RunEnd`, a projection with a run view and a told
    /// cycle; an unknown field refused; a record without a run encodes no `run` key.
    #[test]
    fn sm7_the_run_and_its_outcome_round_trip() {
        let record = running();
        let wire = serde_json::to_value(&record).unwrap_or_else(|e| panic!("{e}"));
        let back: RigRecord =
            serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, record);
        let mut extra = wire;
        extra["run"]["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<RigRecord>(extra).is_err());
        let plain = serde_json::to_value(sample()).unwrap_or_else(|e| panic!("{e}"));
        assert!(plain.get("run").is_none());
        for end in [
            RunEnd::Done,
            RunEnd::WorkedOut,
            RunEnd::Stopped,
            RunEnd::Licence {
                why: LicenceEnd::Lapsed { at_ic: 5 },
            },
            RunEnd::Licence {
                why: LicenceEnd::Other,
            },
            RunEnd::Dark {
                why: DarkCause::Claims,
            },
            RunEnd::Claimed,
            RunEnd::Lease { forfeit: true },
            RunEnd::Refused,
        ] {
            let mined = Mined {
                k: 1,
                cycles: 1,
                pocket: [36, 56],
                good: cid("ice_ore"),
                tonnes: 0,
                blind: true,
                end: Some(end),
            };
            let wire = serde_json::to_value(&mined).unwrap_or_else(|e| panic!("{e}"));
            let back: Mined = serde_json::from_value(wire).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, mined);
        }
        let mut rec = TvRec::new(ObjId(900), 1);
        rec.told_cycle = 2;
        rec.run = Some(Box::new(RunView::of(&run_sample(3))));
        assert_eq!(rec.check(), Ok(()));
        let wire = serde_json::to_value(&rec).unwrap_or_else(|e| panic!("{e}"));
        let back: TvRec = serde_json::from_value(wire).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, rec);
        let fresh =
            serde_json::to_value(TvRec::new(ObjId(900), 1)).unwrap_or_else(|e| panic!("{e}"));
        assert!(fresh.get("told_cycle").is_none() && fresh.get("run").is_none());
        let mut bad = rec.clone();
        bad.run = Some(Box::new(RunView {
            k: 9,
            cycles: 8,
            ..RunView::of(&run_sample(3))
        }));
        assert_eq!(
            bad.check(),
            Err("a rig projection's run is a cycle of at most 8".to_owned())
        );
        // The address has its own bound and its own sentence (conformance/F11): eight bytes
        // pass, nine are refused as an address, never as a cycle count.
        let mut addr = rec.clone();
        addr.run = Some(Box::new(RunView {
            addr: "A".repeat(MAX_RUN_ADDR_BYTES),
            ..RunView::of(&run_sample(3))
        }));
        assert_eq!(addr.check(), Ok(()));
        addr.run = Some(Box::new(RunView {
            addr: "A".repeat(MAX_RUN_ADDR_BYTES + 1),
            ..RunView::of(&run_sample(3))
        }));
        assert_eq!(
            addr.check(),
            Err("a rig projection's run address is at most 8 bytes".to_owned())
        );
    }

    /// An idle record (no act) holding a queued unload after its recover, ordinal 3.
    fn queued_unload() -> RigRecord {
        RigRecord {
            act: None,
            queued: Some(Queued {
                kind: ActKind::Unload,
                operator: ObjId(42),
                hull: ObjId(500),
                slot: 1,
                site: cid("ahuna_port"),
                after: 3,
                at_ic: 1_300,
                excursion: Some(ExcursionId {
                    tv: ObjId(900),
                    launch_ic: 1_000,
                }),
            }),
            ..sample()
        }
    }

    /// An unload act at `ordinal` moving `goods`, filed `how`.
    fn unload_act(ordinal: u32, goods: &[(&str, u64)], how: UnloadHow) -> Act {
        Act {
            kind: ActKind::Unload,
            due_ic: 1_000 + i64::from(UNLOAD_S),
            params: ActParams::Unload {
                hopper: 13,
                hold: 14,
                goods: goods.iter().map(|(g, q)| (cid(g), *q)).collect(),
                how,
                excursion: None,
            },
            ..act(ordinal, ActParams::None {})
        }
    }

    /// **`UNLOAD_S` is 600 IC s** (doc 20 §4.1's "a 600 IC s unload regains 8,300 cu"; the
    /// brief's lead ruling 2): the code constant, pinned.
    #[test]
    fn sm8_unload_s_is_six_hundred() {
        assert_eq!(UNLOAD_S, 600);
        assert_eq!(ActKind::Unload.word(), "unload");
        assert_eq!(ActKind::from_word("unload"), Some(ActKind::Unload));
        assert!(
            !needs_battery(ActKind::Unload, 0, 0),
            "an unload spends nothing and never reads the battery"
        );
    }

    /// **A record with a queued unload checks its bounds** (doc 20 cut SM.8; lead ruling 1): an
    /// idle record with a queued unload passes; beside an act, beside a run, a kind other than
    /// `Unload`, an `after` past the ordinal or of 0, and a slot of 0 are each refused.
    #[test]
    fn sm8_a_record_with_a_queued_unload_checks_its_bounds() {
        let good = queued_unload();
        assert_eq!(good.check(), Ok(()));
        let with = |edit: &dyn Fn(&mut RigRecord)| {
            let mut record = queued_unload();
            edit(&mut record);
            record.check()
        };
        assert!(
            with(&|r| {
                r.ordinal = 4;
                r.act = Some(act(4, ActParams::None {}));
            })
            .is_err(),
            "beside an act"
        );
        assert!(
            with(&|r| {
                r.run = Some(run_sample(3));
            })
            .is_err(),
            "beside a run"
        );
        let edit_queued = |edit: &dyn Fn(&mut Queued)| {
            with(&|r| {
                if let Some(queued) = r.queued.as_mut() {
                    edit(queued);
                }
            })
        };
        assert!(edit_queued(&|q| q.kind = ActKind::Dump).is_err(), "a dump");
        assert!(
            edit_queued(&|q| q.after = 4).is_err(),
            "after past the ordinal"
        );
        assert!(edit_queued(&|q| q.after = 0).is_err(), "after 0");
        assert!(edit_queued(&|q| q.slot = 0).is_err(), "slot 0");
        assert_eq!(edit_queued(&|q| q.after = 2), Ok(()), "an earlier act's");
    }

    /// **An unload act checks its goods** (doc 20 cut SM.8): one or eight positive goods pass in
    /// every form; nine goods, a zero, and an empty list but for a queued unload are refused; the
    /// unload's parameters with any other kind, and any other parameters with `Unload`, are
    /// refused; `stays` belongs to an unload's outcome alone.
    #[test]
    fn sm8_an_unload_act_checks_its_goods() {
        let with_act = |act: Act| {
            let mut record = sample();
            record.act = Some(act);
            record.check()
        };
        for how in [UnloadHow::Queued, UnloadHow::Whole, UnloadHow::Part] {
            assert_eq!(
                with_act(unload_act(3, &[("metal_ore", 12)], how)),
                Ok(()),
                "{how:?}"
            );
        }
        let eight: Vec<(String, u64)> = (0..8).map(|n| (format!("good_{n}"), 1)).collect();
        let eight: Vec<(&str, u64)> = eight.iter().map(|(g, q)| (g.as_str(), *q)).collect();
        assert_eq!(with_act(unload_act(3, &eight, UnloadHow::Whole)), Ok(()));
        let nine: Vec<(String, u64)> = (0..9).map(|n| (format!("good_{n}"), 1)).collect();
        let nine: Vec<(&str, u64)> = nine.iter().map(|(g, q)| (g.as_str(), *q)).collect();
        assert!(
            with_act(unload_act(3, &nine, UnloadHow::Whole)).is_err(),
            "nine"
        );
        assert!(
            with_act(unload_act(3, &[("metal_ore", 0)], UnloadHow::Whole)).is_err(),
            "a zero"
        );
        assert_eq!(
            with_act(unload_act(3, &[], UnloadHow::Queued)),
            Ok(()),
            "a queued unload the hold cannot take moves nothing"
        );
        assert!(with_act(unload_act(3, &[], UnloadHow::Whole)).is_err());
        assert!(with_act(unload_act(3, &[], UnloadHow::Part)).is_err());
        let mut wrong = unload_act(3, &[("metal_ore", 1)], UnloadHow::Whole);
        wrong.kind = ActKind::Dump;
        assert!(with_act(wrong).is_err(), "unload parameters on a dump");
        let mut wrong = act(3, ActParams::None {});
        wrong.kind = ActKind::Unload;
        assert!(with_act(wrong).is_err(), "an unload without its parameters");
        let mut stays = sample();
        if let Some(last) = stays.last.as_mut() {
            last.stays = [(cid("metal_ore"), 7)].into_iter().collect();
        }
        assert!(stays.check().is_err(), "a recover's outcome names no stays");
        if let Some(last) = stays.last.as_mut() {
            last.kind = ActKind::Unload;
            last.result = ActResult::Done;
        }
        assert_eq!(stays.check(), Ok(()));
        if let Some(last) = stays.last.as_mut() {
            last.stays = [(cid("metal_ore"), 0)].into_iter().collect();
        }
        assert!(stays.check().is_err(), "a zero stays");
    }

    /// **The unload's record shapes round-trip the strict wire** (doc 20 cut SM.8): a record with a
    /// queued unload, an unload act of every `how` (with and without its excursion), an outcome
    /// with `stays`, and the two new refusal codes; an unknown field refused; a record with no
    /// queue encodes no `queued` key and an outcome with nothing left no `stays` key.
    #[test]
    fn sm8_the_unload_wire_round_trips() {
        let record = queued_unload();
        let wire = serde_json::to_value(&record).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(wire["queued"]["kind"], serde_json::json!("unload"));
        let back: RigRecord =
            serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, record);
        let mut extra = wire;
        extra["queued"]["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<RigRecord>(extra).is_err());
        let plain = serde_json::to_value(sample()).unwrap_or_else(|e| panic!("{e}"));
        assert!(plain.get("queued").is_none());
        assert!(plain["last"].get("stays").is_none());
        for how in [UnloadHow::Queued, UnloadHow::Whole, UnloadHow::Part] {
            assert_eq!(UnloadHow::from_word(how.word()), Some(how));
            for excursion in [
                None,
                Some(ExcursionId {
                    tv: ObjId(900),
                    launch_ic: 1_000,
                }),
            ] {
                let mut unloading = sample();
                let mut act = unload_act(3, &[("metal_ore", 5), ("ice_ore", 2)], how);
                if let ActParams::Unload {
                    excursion: slot, ..
                } = &mut act.params
                {
                    *slot = excursion;
                }
                unloading.act = Some(act);
                let wire = serde_json::to_value(&unloading).unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(wire["act"]["params"]["how"], serde_json::json!(how.word()));
                let back: RigRecord =
                    serde_json::from_value(wire).unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(back, unloading);
            }
        }
        let mut told = sample();
        if let Some(last) = told.last.as_mut() {
            last.kind = ActKind::Unload;
            last.result = ActResult::Done;
            last.moved = [(cid("metal_ore"), 5)].into_iter().collect();
            last.stays = [(cid("metal_ore"), 7)].into_iter().collect();
        }
        let wire = serde_json::to_value(&told).unwrap_or_else(|e| panic!("{e}"));
        let back: RigRecord = serde_json::from_value(wire).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, told);
        for (code, word) in [
            (RefusedCode::LeftBay, "left_bay"),
            (RefusedCode::HoldRefused, "hold_refused"),
            (RefusedCode::MarkersFull, "markers_full"),
        ] {
            let wire = serde_json::to_value(code).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(wire, serde_json::json!(word));
            let back: RefusedCode = serde_json::from_value(wire).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, code);
        }
    }
    /// A live plan 1 of three steps on `ahuna_port`, filed at 1,000.
    fn fill_plan() -> FillPlan {
        use crate::surface::plan::{PLAN_BOUND_IC_SECS, PlanStep, PlanTally};
        FillPlan {
            no: 1,
            author: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            site: cid("ahuna_port"),
            family: crate::surface::geology::OreFamily::Metals,
            filed_ic: 1_000,
            deadline_ic: 1_000 + PLAN_BOUND_IC_SECS,
            list: vec!["C117".to_owned(), "D".to_owned()],
            steps: vec![
                PlanStep::Mine { pocket: [20, 52] },
                PlanStep::Sound { field: 30 },
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

    /// **A record carries its plan and its `by`** (doc 20 cut SM.11; the brief's 3.1): a record
    /// with a plan and an act set going by the plan checks and round-trips; `by: Idle` on a scan
    /// (or any act but a recover) is refused; a plan never dumps or unloads; a plan whose `no` is
    /// above `plans` (or 0) is refused; the plan's own bounds are read; `live_plan_for` names only
    /// the author; `idle_since` reads the later of the last outcome and the plan's end.
    #[test]
    fn sm11_a_record_carries_its_plan_and_its_by() {
        let mut record = sample();
        record.plans = 1;
        record.plan = Some(fill_plan());
        if let Some(act) = record.act.as_mut() {
            act.by = Some(ActBy::Plan { no: 1, step: 0 });
        }
        assert_eq!(record.check(), Ok(()));
        let wire = serde_json::to_string(&record).unwrap_or_else(|e| panic!("{e}"));
        let back: RigRecord = serde_json::from_str(&wire).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, record);
        assert!(
            wire.contains(r#""by":{"by":"plan","no":1,"step":0}"#),
            "{wire}"
        );
        // An empty record's wire carries none of the new keys.
        let plain = serde_json::to_string(&sample()).unwrap_or_default();
        assert!(
            !plain.contains("\"plan") && !plain.contains("\"by\""),
            "{plain}"
        );
        // Who set it going.
        let mut idle_scan = record.clone();
        if let Some(act) = idle_scan.act.as_mut() {
            act.kind = ActKind::Scan;
            act.by = Some(ActBy::Idle);
        }
        assert!(idle_scan.check().is_err(), "the idle return only recovers");
        let mut idle_recover = sample();
        if let Some(last) = idle_recover.last.as_mut() {
            last.by = Some(ActBy::Idle);
        }
        assert_eq!(idle_recover.check(), Ok(()), "an idle recover's outcome");
        let mut plan_dump = record.clone();
        if let Some(act) = plan_dump.act.as_mut() {
            act.kind = ActKind::Dump;
        }
        assert!(plan_dump.check().is_err(), "a plan never dumps");
        let mut unload_by = sample();
        if let Some(last) = unload_by.last.as_mut() {
            last.kind = ActKind::Unload;
            last.by = Some(ActBy::Plan { no: 1, step: 0 });
        }
        assert!(unload_by.check().is_err(), "a plan never unloads");
        // The plan's number.
        let mut above = record.clone();
        above.plans = 0;
        assert!(above.check().is_err(), "a plan no above plans");
        let mut zero = record.clone();
        if let Some(plan) = zero.plan.as_mut() {
            plan.no = 0;
        }
        assert!(zero.check().is_err(), "a plan no of 0");
        let mut bad_cursor = record.clone();
        if let Some(plan) = bad_cursor.plan.as_mut() {
            plan.cursor = 4;
        }
        assert!(bad_cursor.check().is_err(), "the plan's own bounds");
        // Whose plan it is.
        assert!(record.live_plan_for(ObjId(42)).is_some());
        assert!(
            record.live_plan_for(ObjId(43)).is_none(),
            "orphaned for another lessee"
        );
        // The idle instant.
        assert_eq!(record.idle_since(), Some(800), "the last outcome");
        let mut ended = record.clone();
        if let Some(plan) = ended.plan.as_mut() {
            plan.state = FillState::Ended {
                why: crate::surface::plan::PlanEnd::WorkedOut,
                at_ic: 5_000,
            };
        }
        assert_eq!(ended.idle_since(), Some(5_000), "the plan's later end");
        assert!(
            ended.live_plan_for(ObjId(42)).is_none(),
            "an ended plan is not live"
        );
        let mut fresh_out = sample();
        fresh_out.last = None;
        assert_eq!(
            fresh_out.idle_since(),
            Some(1_000),
            "the launch, with no outcome"
        );
    }

    /// **A projection with a plan fits 4 KiB** (doc 20 cut SM.11; the brief's 3.1): a `TvRec`
    /// with a sixteen-line log of 160-byte lines and a full `PlanView` (four step words, an end)
    /// fits after `fit`, the plan view kept whole and the newest line kept.
    #[test]
    fn sm11_a_projection_with_a_plan_fits_four_kib() {
        let mut rec = TvRec::new(ObjId(900), 1);
        rec.plan = Some(PlanView {
            no: 4_000_000_000,
            state: "parking".to_owned(),
            step: 64,
            steps: 64,
            next: vec![
                "mine block D, fair or better".to_owned(),
                "mine block E, fair or better".to_owned(),
                "sound D-NW".to_owned(),
                "mine C117".to_owned(),
            ],
            deadline_ic: 123_456_789_012,
            end: Some("its licence lapsed".to_owned()),
        });
        rec.plan_told = Some(PlanTold {
            no: 4_000_000_000,
            seq: 4_000_000_000,
        });
        for n in 0..16_i64 {
            rec.push_log(1_000_000 + n, &format!("{n:02}{}", "x".repeat(158)));
        }
        let size = |rec: &TvRec| serde_json::to_vec(rec).map(|b| b.len()).unwrap_or(0);
        let plan = rec.plan.clone();
        rec.fit();
        assert_eq!(rec.check(), Ok(()));
        assert!(size(&rec) <= MAX_TV_REC_BYTES, "{}", size(&rec));
        assert_eq!(rec.plan, plan, "the plan view is kept whole");
        assert_eq!(
            rec.log.last().map(|l| l.at_ic),
            Some(1_000_015),
            "the newest line"
        );
        // The view's own bounds.
        let mut five = rec.clone();
        if let Some(view) = five.plan.as_mut() {
            view.next.push("mine C118".to_owned());
        }
        assert!(five.check().is_err(), "at most four step words");
        let wire = serde_json::to_string(&rec).unwrap_or_else(|e| panic!("{e}"));
        let back: TvRec = serde_json::from_str(&wire).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, rec);
        let bare = serde_json::to_string(&TvRec::new(ObjId(900), 1)).unwrap_or_default();
        assert!(!bare.contains("plan"), "skipped when empty: {bare}");
    }
}
