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

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::definitions::logistics::{BayDef, MAX_BAY_ITEMS};
use crate::definitions::tools::BatteryDef;
use crate::surface::ground::SITE_CELL_CM;

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
/// The most excursions one operator runs (§8, §11 Caps: the Compact's span-of-control
/// lever).
pub const MAX_EXCURSIONS_PER_OPERATOR: u32 = 4;
/// The most excursions one site holds (§8, §11 Caps).
pub const MAX_EXCURSIONS_PER_SITE: u32 = 32;
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
    /// **A test world's act with a cost** (the brief's 3.7): exercises the reserve rule
    /// before SM.4 and SM.7 give real acts costs. Never offered in production.
    Probe,
}

impl ActKind {
    /// The act's word on the verb and the wire (`launch`, `recover`, `dump`, `probe`).
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            ActKind::Launch => "launch",
            ActKind::Recover => "recover",
            ActKind::Dump => "dump",
            ActKind::Probe => "probe",
        }
    }

    /// The inverse of [`ActKind::word`].
    #[must_use]
    pub fn from_word(word: &str) -> Option<ActKind> {
        match word {
            "launch" => Some(ActKind::Launch),
            "recover" => Some(ActKind::Recover),
            "dump" => Some(ActKind::Dump),
            "probe" => Some(ActKind::Probe),
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
    /// The test world's probe: its cost, cu.
    Probe {
        /// What the probe spends, cu.
        cost_cu: u32,
    },
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
}

/// **One rig's record in `tacsim.excursion`** (§8; lead ruling 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RigRecord {
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
        if self.schema != EXCURSION_SCHEMA {
            return Err(format!(
                "an excursion record is schema {EXCURSION_SCHEMA}, not {}",
                self.schema
            ));
        }
        if let Some(act) = &self.act {
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
        }
        if let Some(excursion) = &self.excursion {
            if excursion.id.tv != self.tv {
                return Err("an excursion is its own rig's".to_owned());
            }
            if !slot_ok(excursion.slot) {
                return Err(format!("an excursion's slot is 1 to {MAX_BAY_ITEMS}"));
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
        }
        Ok(())
    }
}

/// **Every in-flight reservation the record holds**, summed (§4.1, §19 item 4: "two acts can
/// never spend one margin"). In SM.3b that is the act's `reserved_cu`, and because the
/// start's prepare has already required `act` to be `None` when it reads this (the one act
/// slot and the ordinal), **it is always 0 at the reserve check in SM.3b**: the "two acts
/// cannot both start" half holds through the slot and the ordinal. SM.7's cycle slot is the
/// first reservation that can coexist with a new act; it extends this one function and must
/// prove the sum in the world (the brief's decision 4).
#[must_use]
pub fn reserved_total(record: &RigRecord) -> u32 {
    record.act.as_ref().map_or(0, |act| act.reserved_cu)
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
/// settles the charge the bay put back (the recharge rate is the battery's). A recover and a
/// dump spend nothing and never read it: a rig whose battery row stops resolving still plans,
/// starts and finishes its way home and still empties its hopper; a launch fails closed.
#[must_use]
pub fn needs_battery(kind: ActKind, cost_cu: u32, home_cu: u32) -> bool {
    kind == ActKind::Launch || u64::from(cost_cu).saturating_add(u64::from(home_cu)) > 0
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
    if counts.operator >= MAX_EXCURSIONS_PER_OPERATOR {
        Some(CapRefused::Operator)
    } else if counts.site >= MAX_EXCURSIONS_PER_SITE {
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
    /// Parked on the ground (SM.11).
    Parked,
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

/// One waypoint of a frozen leg (§8; SM.4's slot).
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
/// positions between are pure interpolation). SM.4's slot: always `None` in SM.3b.
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
    /// SM.4's frozen leg; always `None` in SM.3b.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leg: Option<FrozenLeg>,
    /// The act ordinal whose placement the tick wrote (the idempotency key).
    pub applied: u32,
    /// The act ordinal whose line the operator has been told.
    pub told: u32,
    /// The newest [`MAX_TV_LOG_LINES`] lines, oldest first.
    pub log: Vec<TvLogLine>,
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
            log: Vec::new(),
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

    /// **The projection's bounds**: the schema; at most [`MAX_TV_LOG_LINES`] log lines, each
    /// at most [`MAX_TV_LOG_BYTES`] and on one row; a leg of at most [`MAX_LEG_WAYPOINTS`];
    /// the canonical JSON at most [`MAX_TV_REC_BYTES`]. (`told <= applied` is **not**
    /// required: a refused finish is told without a placement.)
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
        assert!(needs_battery(ActKind::Probe, 1, 0));
        assert!(needs_battery(ActKind::Recover, 0, 1), "a spend home");
        assert!(!needs_battery(ActKind::Recover, 0, 0));
        assert!(!needs_battery(ActKind::Dump, 0, 0));
        assert!(!needs_battery(ActKind::Probe, 0, 0));
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
        assert_eq!(CapRefused::Site.cap(), 32);
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
            }),
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
            ActKind::Probe,
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
            ActParams::Probe { cost_cu: 60_000 },
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
}
