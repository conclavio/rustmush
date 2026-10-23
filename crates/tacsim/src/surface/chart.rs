//! **The chart: what an operator knows of the ground** (doc 20 cut SM.4a; §5 "Knowing",
//! D-20-8; the SM.4a brief's lead rulings 3, 4, 5 and 7, and its decisions D1–D7, D9, D17,
//! D18, D20 and D26).
//!
//! A player never sees the ground; they see **their own readings** (§5). This module holds:
//!
//! - **`tacsim.chart`** — the typed, authoritative family of [`ChartRecord`]s, one per
//!   (principal, site) ([`ChartKey`]; D1: a per-body row's worst case in canonical JSON runs
//!   past the family's 256 KiB record cap, so the row is a site's and doc 20's per-body caps are
//!   enforced at the start across the principal's rows on the body, [`body_admits`]). A row
//!   holds **coverage and provenance, never a reading**: which fields were scanned and at what
//!   footing, which pockets were sounded, when and by which instrument
//!   ([`ChartProvenance::Local`]), the stands the last scan and sounding were made from (the
//!   same-cell rule, [`repeat_refused`]) and the sounder's heard budget ([`heard_line_due`]).
//!   It is written only in an `ActionTxn`: by `tacsim_surface_done`'s finish for a scan or a
//!   sounding (`crate::surface::act`) and by `+tv/forget`'s own action
//!   `tacsim_chart_forget` (`ChartForgetProvider`);
//! - **the pure rules** the command, the start's prepare and the finish share: [`scan_adds`],
//!   [`sound_adds`], [`repeat_refused`], [`body_admits`], [`heard_line_due`], [`write_scan`],
//!   [`write_sound`], [`forget_row`];
//! - **the render** (lead ruling 3): every reading is re-derived from the keyed geology through
//!   reading rule v1 (`crate::surface::reading`) under the row's pinned `rule`, at render, from
//!   the viewer's own row — [`field_reading`], [`pocket_reading`]. "You have no reading there." is
//!   byte-identical for an unknown site, an uncovered unit and a barren pocket never sounded
//!   (§5 "Fog, by construction");
//! - **coverage over the composite** (lead ruling 5): [`footing_at`], [`seen_fields`] (the atlas
//!   `line_of_sight` walk at the 2.5 m mast, D5) and [`better_footing`] (D20) — pure over a
//!   `SurfaceComposite` the caller peeked (`peek_site`); the finish never reads one.
//!
//! Knowledge needs no tenure and no licence (§8 "rights are per act", §19 item 3; lead ruling
//! 9): nothing here reads either. Integers throughout; no RNG.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_atlas::trace::{CellGrid, Eye, line_of_sight};
use mush_atlas::window::{SurfaceComposite, WindowFrame};
use mush_atlas::{SurfaceRef, TerrainClass};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalId, DefinitionRef, GeologyMac, ObjId,
    ProjectedParameterKind, ProviderOfferDraft, RulesValue, StateFamilyHandle, World,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::definitions::tools::ScannerDef;
use crate::pack::{LocationDef, PackEntry, RulesPack};
use crate::port_tick::port_error;
use crate::store::TacsimStore;
use crate::surface::address::{
    self, Octant, composite_cell, field_index, field_name, field_of_index, field_of_pocket,
    pocket_address, pocket_in_block,
};
use crate::surface::geology::{
    BodyGeology, FieldGround, GeologyError, OreFamily, POCKETS_PER_FIELD_SIDE, PocketRow, Word,
    by_family, field_ground,
};
use crate::surface::ground::{SITE_BLOCKS, SiteGeologySpec, SiteGround};
use crate::surface::reading::{FieldReading, Footing, READING_RULE_VERSION, read_r1, read_r2};

/// The typed family (lead ruling 3).
pub const CHART_FAMILY: &str = "tacsim.chart";
/// [`ChartRecord::schema`].
pub const CHART_SCHEMA: u16 = 1;
/// Provenance entries a row keeps (§5 "provenance ≤ 4"): a ring of the newest four.
pub const MAX_CHART_PROVENANCE: usize = 4;
/// Field rows a principal's chart of one body holds (§5).
pub const MAX_CHART_FIELD_ROWS_PER_BODY: usize = 1_024;
/// Pocket blocks a principal's chart of one body holds (§5).
pub const MAX_CHART_POCKET_BLOCKS_PER_BODY: usize = 64;
/// The most site entries one body names in the start's offer (the body cap's census).
pub const MAX_CHART_SITES_PER_BODY: usize = 32;
/// The family's record cap, canonical JSON bytes: the worst-case row (64 fields and 16 full
/// blocks, four provenance entries each) encodes far under it (a unit test).
pub const MAX_CHART_RECORD_BYTES: usize = 65_536;
/// **A pocket cell's byte layout** (the pre-commit round's item 3; §5): bits 0..=1 are its tier
/// ([`TIER_MASK`]: `0` unsounded, [`TIER_R2`] sounded), bit 2 is [`PIT_SEEN`] (written by a
/// core, doc 20 cut SM.7: [`write_core`]), bits 3..=7 are reserved and zero. Every reader masks
/// the tier out with [`cell_tier`] and every writer keeps the bits it does not own, so SM.7's bit
/// never reads as unsounded and a re-sounding never clears it.
pub const TIER_MASK: u8 = 0b0000_0011;
/// The R2 tier: a sounding (or, from SM.7, a core).
pub const TIER_R2: u8 = 2;
/// **The pit-seen bit** (doc 20 cut SM.7): set by a core ([`write_core`]), valid only beside a
/// sounded tier.
pub const PIT_SEEN: u8 = 0b0000_0100;
/// One heard line a sounder an IC hour (§11 Caps; Q-20-9).
pub const HEARD_EVERY_IC: i64 = 3_600;
/// Field rows one site holds: its 64 fields.
pub const MAX_CHART_FIELD_ROWS_PER_SITE: usize = 64;
/// Pocket blocks one site holds: its 16 blocks.
pub const MAX_CHART_BLOCKS_PER_SITE: usize = 16;
/// Pocket cells in one block, and so bytes in one [`PocketBlock::cells`].
pub const BLOCK_POCKETS: usize = 256;

/// A pocket cell's tier: its byte's low two bits ([`TIER_MASK`]).
#[must_use]
pub const fn cell_tier(byte: u8) -> u8 {
    byte & TIER_MASK
}

// ---- the family's key and value ------------------------------------------------------------

/// **The family key** (D1, D2): one row per (character, site).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChartKey {
    /// The principal: the character who read the ground.
    pub principal: ObjId,
    /// The site's port entry.
    pub site: CanonicalId,
}

impl ChartKey {
    /// `{principal.0}/{site}` (the decimal id, never `ObjId`'s `#`-prefixed Display).
    #[must_use]
    pub fn key_text(&self) -> String {
        format!("{}/{}", self.principal.0, self.site)
    }

    /// The strict inverse of [`ChartKey::key_text`]: one `/`, a strict decimal principal and a
    /// canonical site id.
    #[must_use]
    pub fn parse(text: &str) -> Option<ChartKey> {
        let (principal, site) = text.split_once('/')?;
        Some(ChartKey {
            principal: ObjId(crate::port::families::strict_decimal_u32(principal)?),
            site: CanonicalId::new(site).ok()?,
        })
    }
}

/// **Which instrument read a row** (§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Instrument {
    /// A scan (R1).
    Scan,
    /// A loud sounding (R2), heard across the site.
    Sound,
    /// A quiet sounding (R2), heard by nobody.
    QuietSound,
    /// **A core** (doc 20 cut SM.7; §6 "the core is a reading"; the brief's lead ruling 7): a
    /// mining cycle's R2 of the pocket it drilled, written at the cycle's own due instant.
    Core,
}

/// **How the reading was made** (§5, §8): by hand, or by a fill plan's instruments while the
/// operator was away. SM.4a writes only `Hand`; **doc 20 cut SM.11** (lead ruling 4) writes `Plan`
/// for every reading a plan's scan, sounding or core makes — the character's own for every use but
/// a stake (`crate::surface::tenure::evidence_at` never counts it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// The operator's own hand.
    Hand,
    /// A plan (SM.11).
    Plan,
}

/// **Where a row's knowledge came from** (§5, the chart's own enum, modelled on
/// `mush-propagation`'s but never changing it). `Reported { source, at, via: Gift | Sale |
/// Salvage }` is phase 2's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChartProvenance {
    /// The principal's own instrument.
    Local {
        /// Which instrument.
        instrument: Instrument,
        /// The IC instant: the act's own `due_ic`.
        at: i64,
        /// By hand or by plan.
        via: Via,
    },
}

impl ChartProvenance {
    /// The stamp.
    #[must_use]
    pub fn at(&self) -> i64 {
        match self {
            ChartProvenance::Local { at, .. } => *at,
        }
    }
}

/// **One scanned field** (§5): the sharpest footing it was read from, when, and by what.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldRow {
    /// The sharpest footing any scan read it from (it only ever sharpens).
    pub footing: Footing,
    /// The earliest stamp ever written (SM.10 reads discovery from it).
    pub first_local_at: i64,
    /// The latest stamp.
    pub last_at: i64,
    /// The newest four provenance entries by stamp, oldest first — hand stamps kept before plan
    /// stamps ([`write_scan`]'s trim, doc 20 cut SM.11).
    pub provenance: Vec<ChartProvenance>,
    /// **The earliest hand-made stamp ever written** (doc 20 cut SM.11; lead ruling 4): a plan's
    /// stamp never touches it, so SM.10's stake evidence reads it. Skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hand_at: Option<i64>,
}

/// **One block's sounded pockets** (§5): 256 one-byte cells, `index = pocket number − 1` (row-
/// major from the block's north-west, `crate::surface::address::pocket_in_block`), as 512
/// lowercase base-16 characters; a cell's low two bits are its tier ([`TIER_MASK`], the layout
/// at [`TIER_MASK`]'s note).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PocketBlock {
    /// The cells.
    pub cells: String,
    /// The earliest stamp ever written.
    pub first_local_at: i64,
    /// The latest stamp.
    pub last_at: i64,
    /// The newest four provenance entries by stamp, oldest first — hand stamps kept before plan
    /// stamps (doc 20 cut SM.11).
    pub provenance: Vec<ChartProvenance>,
    /// **The earliest hand-made stamp ever written** (doc 20 cut SM.11); skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hand_at: Option<i64>,
}

impl PocketBlock {
    /// A block with no pocket sounded.
    fn empty(at: i64) -> PocketBlock {
        PocketBlock {
            cells: "0".repeat(BLOCK_POCKETS * 2),
            first_local_at: at,
            last_at: at,
            provenance: Vec::new(),
            first_hand_at: None,
        }
    }

    /// The cells as bytes; `None` when the string is not 512 lowercase base-16 characters.
    #[must_use]
    pub fn bytes(&self) -> Option<Vec<u8>> {
        if self.cells.len() != BLOCK_POCKETS * 2
            || !self
                .cells
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let raw = self.cells.as_bytes();
        let mut out = Vec::with_capacity(BLOCK_POCKETS);
        for pair in raw.chunks(2) {
            let text = std::str::from_utf8(pair).ok()?;
            out.push(u8::from_str_radix(text, 16).ok()?);
        }
        Some(out)
    }

    /// One cell's byte.
    #[must_use]
    pub fn cell(&self, index: usize) -> Option<u8> {
        let at = index.checked_mul(2)?;
        let text = self.cells.get(at..at.checked_add(2)?)?;
        u8::from_str_radix(text, 16).ok()
    }

    /// How many of its pockets are sounded.
    #[must_use]
    pub fn sounded(&self) -> usize {
        self.bytes().map_or(0, |bytes| {
            bytes.iter().filter(|b| cell_tier(**b) == TIER_R2).count()
        })
    }
}

/// The bytes as 512 lowercase base-16 characters.
fn encode_cells(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// **One principal's chart of one site** (§5; D1, D2, D7): coverage and provenance only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChartRecord {
    /// [`CHART_SCHEMA`].
    pub schema: u16,
    /// The principal (the character).
    pub principal: ObjId,
    /// The site's port entry.
    pub site: CanonicalId,
    /// The site's body.
    pub body: CanonicalId,
    /// The [`READING_RULE_VERSION`] the rows render under (a row under another version renders
    /// nothing, never a re-read).
    pub rule: u16,
    /// Field index (`fy · 8 + fx`) → row, at most 64.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<u8, FieldRow>,
    /// Block letter `"A"`..`"P"` → its sounded pockets, at most 16.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blocks: BTreeMap<String, PocketBlock>,
    /// The last heard line's stamp (the sounder's budget, D7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heard_ic: Option<i64>,
    /// The cell the last scan was made from (D4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_stand: Option<[i64; 2]>,
    /// The cell the last sounding, loud or quiet, was made from (D4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound_stand: Option<[i64; 2]>,
}

/// One row's provenance: `1..=4`, each stamp within `[first, last]`, `first ≤ last`; doc 20 cut
/// SM.11: the first hand stamp, when there is one, within `[first, last]` too.
fn check_stamps(
    first: i64,
    last: i64,
    provenance: &[ChartProvenance],
    first_hand: Option<i64>,
) -> Result<(), String> {
    if first > last {
        return Err("a chart row's first stamp is no later than its last".to_owned());
    }
    if first_hand.is_some_and(|hand| hand < first || hand > last) {
        return Err("a chart row's first hand stamp lies within its stamps".to_owned());
    }
    if provenance.is_empty() || provenance.len() > MAX_CHART_PROVENANCE {
        return Err(format!(
            "a chart row keeps 1 to {MAX_CHART_PROVENANCE} provenance entries"
        ));
    }
    if provenance
        .iter()
        .any(|entry| entry.at() < first || entry.at() > last)
    {
        return Err("a chart row's provenance lies within its stamps".to_owned());
    }
    Ok(())
}

impl ChartRecord {
    /// An empty row for `principal` on `site` (on `body`), under the current rule.
    #[must_use]
    pub fn new(principal: ObjId, site: CanonicalId, body: CanonicalId) -> ChartRecord {
        ChartRecord {
            schema: CHART_SCHEMA,
            principal,
            site,
            body,
            rule: READING_RULE_VERSION,
            fields: BTreeMap::new(),
            blocks: BTreeMap::new(),
            heard_ic: None,
            scan_stand: None,
            sound_stand: None,
        }
    }

    /// The record's key, derived from the value (the families rule).
    #[must_use]
    pub fn key(&self) -> ChartKey {
        ChartKey {
            principal: self.principal,
            site: self.site.clone(),
        }
    }

    /// Whether the row covers anything (a field scanned or a block sounded).
    #[must_use]
    pub fn has_coverage(&self) -> bool {
        !self.fields.is_empty() || !self.blocks.is_empty()
    }

    /// How many pockets the row has sounded.
    #[must_use]
    pub fn sounded_pockets(&self) -> usize {
        self.blocks.values().map(PocketBlock::sounded).sum()
    }

    /// Whether `pocket` (site-local) is sounded.
    #[must_use]
    pub fn pocket_sounded(&self, pocket: (i64, i64)) -> bool {
        pocket_in_block(pocket.0, pocket.1).is_some_and(|(letter, index)| {
            self.blocks
                .get(letter.to_string().as_str())
                .and_then(|block| block.cell(index))
                .map(cell_tier)
                == Some(TIER_R2)
        })
    }

    /// **The row's own invariants** (the family's validator calls it): the schema; `rule ≥ 1`;
    /// **a row with no field row and no block is valid only when it carries `heard_ic`**
    /// (`+tv/forget`'s budget-keeper, D7) **and then no stand**; each stand on the site; at
    /// most 64 field rows each a valid index and 16 blocks each a letter `A`–`P`; every block's
    /// cells 512 lowercase base-16 characters, each byte's tier `0` or [`TIER_R2`], its
    /// [`PIT_SEEN`] bit only beside a sounded tier and no reserved bit set (the pre-commit
    /// round's item 3), at least one sounded; every row's stamps and `1..=4` provenance.
    ///
    /// # Errors
    ///
    /// The first invariant broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != CHART_SCHEMA {
            return Err(format!(
                "a chart row is schema {CHART_SCHEMA}, not {}",
                self.schema
            ));
        }
        if self.rule == 0 {
            return Err("a chart row names its reading rule".to_owned());
        }
        if !self.has_coverage() {
            if self.heard_ic.is_none() {
                return Err("a chart row with no readings keeps only a heard budget".to_owned());
            }
            if self.scan_stand.is_some() || self.sound_stand.is_some() {
                return Err("a chart row with no readings names no stand".to_owned());
            }
        }
        for stand in [self.scan_stand, self.sound_stand].into_iter().flatten() {
            if !address::cell_on_site((stand[0], stand[1])) {
                return Err("a chart row's stands are on its site".to_owned());
            }
        }
        if self.fields.len() > MAX_CHART_FIELD_ROWS_PER_SITE {
            return Err(format!(
                "a chart row holds at most {MAX_CHART_FIELD_ROWS_PER_SITE} fields"
            ));
        }
        for (index, row) in &self.fields {
            if field_of_index(*index).is_none() {
                return Err(format!("field {index} is not a field of a site"));
            }
            check_stamps(
                row.first_local_at,
                row.last_at,
                &row.provenance,
                row.first_hand_at,
            )?;
        }
        if self.blocks.len() > MAX_CHART_BLOCKS_PER_SITE {
            return Err(format!(
                "a chart row holds at most {MAX_CHART_BLOCKS_PER_SITE} blocks"
            ));
        }
        for (letter, block) in &self.blocks {
            let one = {
                let mut chars = letter.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => crate::port::def::BLOCK_LETTERS.contains(&c),
                    _ => false,
                }
            };
            if !one {
                return Err(format!("block {letter} is not a block letter A to P"));
            }
            let Some(bytes) = block.bytes() else {
                return Err(format!(
                    "block {letter}'s cells are 512 lowercase base-16 characters"
                ));
            };
            let well_formed = |b: u8| {
                let tier = cell_tier(b);
                b & !(TIER_MASK | PIT_SEEN) == 0
                    && (tier == 0 || tier == TIER_R2)
                    && (b & PIT_SEEN == 0 || tier == TIER_R2)
            };
            if !bytes.iter().all(|b| well_formed(*b)) {
                return Err(format!(
                    "block {letter}'s cells carry only the R2 tier and, beside it, the pit-seen bit"
                ));
            }
            if !bytes.iter().any(|b| cell_tier(*b) == TIER_R2) {
                return Err(format!("block {letter} holds at least one sounded pocket"));
            }
            check_stamps(
                block.first_local_at,
                block.last_at,
                &block.provenance,
                block.first_hand_at,
            )?;
        }
        Ok(())
    }
}

// ---- the pure rules ---------------------------------------------------------------------------

/// A row under the current rule, else `None` (a row under another rule renders nothing and is
/// reset by the next write, never re-read).
fn current(row: Option<&ChartRecord>) -> Option<&ChartRecord> {
    row.filter(|row| row.rule == READING_RULE_VERSION)
}

/// **Whether a viewer's row reads a pocket** (§5; doc 20 cut SM.4b): the row under the current
/// rule has sounded it. **The one gate** every surface renders a pocket's classes through —
/// [`pocket_text`] (the render door, [`pocket_reading`]) and the `site.map` frame's digits
/// (`crate::port::site_map`) — so no surface shows a pocket the door would not.
#[must_use]
pub(crate) fn row_reads_pocket(row: Option<&ChartRecord>, pocket: (i64, i64)) -> bool {
    current(row).is_some_and(|row| row.pocket_sounded(pocket))
}

/// **The footing a viewer's row reads a field at** (§5; doc 20 cut SM.4b), or `None` when the
/// row (under the current rule) does not cover it. The one gate [`field_text`] and the
/// `site.map` frame's readings list both render a field's R1 readings through
/// ([`row_reads_pocket`]'s twin).
#[must_use]
pub(crate) fn row_reads_field(row: Option<&ChartRecord>, (fx, fy): (i64, i64)) -> Option<Footing> {
    let row = current(row)?;
    let index = field_index(fx, fy)?;
    row.fields.get(&index).map(|known| known.footing)
}

/// **Whether a scan adds to the chart**: some field not in the row, or in it at a duller
/// footing.
#[must_use]
pub fn scan_adds(row: Option<&ChartRecord>, fields: &[u8], footing: Footing) -> bool {
    let Some(row) = current(row) else {
        return !fields.is_empty();
    };
    fields.iter().any(|index| {
        row.fields
            .get(index)
            .is_none_or(|known| known.footing.sharpness() < footing.sharpness())
    })
}

/// **Whether a sounding adds to the chart**: some pocket in reach whose cell is not R2.
#[must_use]
pub fn sound_adds(row: Option<&ChartRecord>, pockets: &[(i64, i64)]) -> bool {
    let Some(row) = current(row) else {
        return !pockets.is_empty();
    };
    pockets.iter().any(|pocket| !row.pocket_sounded(*pocket))
}

/// **"You learn nothing new here."** (§5, literally; D4; the brief's reviewer finding F2): the
/// same instrument class — a scan, or a sounding loud or quiet — last used from this very
/// `stand` by this principal on this site, **and** the act would add nothing. The same
/// instrument from any other cell always runs and stamps fresh provenance (SM.10's in-epoch
/// contest evidence, D-20-12, §9.4, §19 item 8).
#[must_use]
pub fn repeat_refused(row: Option<&ChartRecord>, sound: bool, stand: [i64; 2], adds: bool) -> bool {
    let Some(row) = current(row) else {
        return false;
    };
    let last = if sound {
        row.sound_stand
    } else {
        row.scan_stand
    };
    last == Some(stand) && !adds
}

/// **Whether the body's caps admit an act** (§5 "at the caps the server refuses"; D1, D3):
/// the principal's rows on the body (`rows`, this site's among them) with `new_fields` (field
/// indexes on `site`) and `new_blocks` (block letters on `site`) added stay within
/// [`MAX_CHART_FIELD_ROWS_PER_BODY`] field rows and [`MAX_CHART_POCKET_BLOCKS_PER_BODY`]
/// blocks. A unit the site's row already holds is not new.
#[must_use]
pub fn body_admits(
    rows: &[ChartRecord],
    site: &CanonicalId,
    new_fields: &[u8],
    new_blocks: &[String],
) -> bool {
    let mut fields = 0_usize;
    let mut blocks = 0_usize;
    let mut here_fields: BTreeSet<u8> = BTreeSet::new();
    let mut here_blocks: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        if &row.site == site {
            here_fields.extend(row.fields.keys().copied());
            here_blocks.extend(row.blocks.keys().cloned());
        } else {
            fields = fields.saturating_add(row.fields.len());
            blocks = blocks.saturating_add(row.blocks.len());
        }
    }
    here_fields.extend(new_fields.iter().copied());
    here_blocks.extend(new_blocks.iter().cloned());
    fields.saturating_add(here_fields.len()) <= MAX_CHART_FIELD_ROWS_PER_BODY
        && blocks.saturating_add(here_blocks.len()) <= MAX_CHART_POCKET_BLOCKS_PER_BODY
}

/// **Whether a loud sounding at `at` is heard** (§11 Caps, Q-20-9; D7): the sounder's last
/// heard line on this site an IC hour or more before, or none.
#[must_use]
pub fn heard_line_due(last: Option<i64>, at: i64) -> bool {
    last.is_none_or(|last| at.saturating_sub(last) >= HEARD_EVERY_IC)
}

/// One row's stamps a write moves: its first, its last, its ring and its first hand stamp.
struct Stamps<'a> {
    first: &'a mut i64,
    last: &'a mut i64,
    provenance: &'a mut Vec<ChartProvenance>,
    first_hand: &'a mut Option<i64>,
}

/// Push one provenance entry into a ring of four, keep `first_local_at` the earliest stamp and
/// move `last_at` to the latest. **The ring is ordered by stamp** (the pre-commit round's item 4;
/// SM.10's contest evidence reads it): the entry goes in after every entry stamped no later than
/// `at` (ties keep their insertion order). **Doc 20 cut SM.11** (lead ruling 4; the reviewer's F7):
/// a hand stamp also moves `first_hand_at` to the earliest hand stamp (a plan's never touches it);
/// and the trim, while the ring holds more than four, drops **the oldest `via: Plan` entry if the
/// ring holds one, else the oldest entry** — so the ring holds the four newest hand stamps, then
/// the newest plan stamps in what room is left: a plan's stamps never evict a hand stamp, and for a
/// pilot who never files a plan the ring behaves byte for byte as SM.4a built it.
fn stamp(stamps: Stamps<'_>, instrument: Instrument, at: i64, via: Via) {
    *stamps.first = (*stamps.first).min(at);
    *stamps.last = (*stamps.last).max(at);
    if via == Via::Hand {
        *stamps.first_hand = Some(stamps.first_hand.map_or(at, |first| first.min(at)));
    }
    let provenance = stamps.provenance;
    let slot = provenance
        .iter()
        .position(|entry| entry.at() > at)
        .unwrap_or(provenance.len());
    provenance.insert(
        slot,
        ChartProvenance::Local {
            instrument,
            at,
            via,
        },
    );
    while provenance.len() > MAX_CHART_PROVENANCE {
        let oldest_plan = provenance
            .iter()
            .position(|entry| matches!(entry, ChartProvenance::Local { via: Via::Plan, .. }));
        provenance.remove(oldest_plan.unwrap_or(0));
    }
}

/// A row about to be written under the current rule: one under another rule keeps only its
/// heard budget (a reading never drifts across rules, lead ruling 3).
fn renew(row: &mut ChartRecord) {
    if row.rule != READING_RULE_VERSION {
        row.rule = READING_RULE_VERSION;
        row.fields.clear();
        row.blocks.clear();
        row.scan_stand = None;
        row.sound_stand = None;
    }
}

/// **A scan's write** (lead ruling 3): each of `fields` covered at `footing` — footing only
/// ever sharpens — `scan_stand` set to `stand`, and for **every** field the act read (whether
/// or not its coverage changed) `last_at = at`, `first_local_at` kept the earliest and one
/// `Local { Scan, at, via }` pushed into its ring (doc 20 cut SM.11: `via` the act's — `Hand`, or
/// `Plan` for a fill plan's).
pub fn write_scan(
    row: &mut ChartRecord,
    stand: [i64; 2],
    fields: &[u8],
    footing: Footing,
    at: i64,
    via: Via,
) {
    renew(row);
    row.scan_stand = Some(stand);
    for index in fields {
        let entry = row.fields.entry(*index).or_insert_with(|| FieldRow {
            footing,
            first_local_at: at,
            last_at: at,
            provenance: Vec::new(),
            first_hand_at: None,
        });
        if footing.sharpness() > entry.footing.sharpness() {
            entry.footing = footing;
        }
        stamp(
            Stamps {
                first: &mut entry.first_local_at,
                last: &mut entry.last_at,
                provenance: &mut entry.provenance,
                first_hand: &mut entry.first_hand_at,
            },
            Instrument::Scan,
            at,
            via,
        );
    }
}

/// **A sounding's write** (lead ruling 3): each of `pockets`' tier set to [`TIER_R2`] in its
/// block — the cell's other bits ([`PIT_SEEN`]) kept (the pre-commit round's item 3) —
/// `sound_stand` set to `stand`, and every block the act read stamped once (`Local { Sound |
/// QuietSound, at, via }`; doc 20 cut SM.11's `via`).
pub fn write_sound(
    row: &mut ChartRecord,
    stand: [i64; 2],
    pockets: &[(i64, i64)],
    quiet: bool,
    at: i64,
    via: Via,
) {
    renew(row);
    row.sound_stand = Some(stand);
    let instrument = if quiet {
        Instrument::QuietSound
    } else {
        Instrument::Sound
    };
    let mut by_block: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for pocket in pockets {
        if let Some((letter, index)) = pocket_in_block(pocket.0, pocket.1) {
            by_block.entry(letter.to_string()).or_default().push(index);
        }
    }
    for (letter, indexes) in by_block {
        let block = row
            .blocks
            .entry(letter)
            .or_insert_with(|| PocketBlock::empty(at));
        let mut bytes = block.bytes().unwrap_or_else(|| vec![0; BLOCK_POCKETS]);
        for index in indexes {
            if let Some(cell) = bytes.get_mut(index) {
                *cell = (*cell & !TIER_MASK) | TIER_R2;
            }
        }
        block.cells = encode_cells(&bytes);
        stamp(
            Stamps {
                first: &mut block.first_local_at,
                last: &mut block.last_at,
                provenance: &mut block.provenance,
                first_hand: &mut block.first_hand_at,
            },
            instrument,
            at,
            via,
        );
    }
}

/// **A core's write** (doc 20 cut SM.7; §5 "the core is a reading", §6; the brief's lead ruling 7
/// and 3.9): the row renewed under the current rule; `pocket`'s block (created when absent); its
/// cell set to [`TIER_R2`] with [`PIT_SEEN`] — the cell's other bits kept; the block stamped once,
/// `Local { Core, at, via }` (doc 20 cut SM.11's `via`). **Neither stand is touched**: a core
/// never feeds the same-cell repeat rule. A pocket off the site writes nothing.
pub fn write_core(row: &mut ChartRecord, pocket: (i64, i64), at: i64, via: Via) {
    renew(row);
    let Some((letter, index)) = pocket_in_block(pocket.0, pocket.1) else {
        return;
    };
    let block = row
        .blocks
        .entry(letter.to_string())
        .or_insert_with(|| PocketBlock::empty(at));
    let mut bytes = block.bytes().unwrap_or_else(|| vec![0; BLOCK_POCKETS]);
    if let Some(cell) = bytes.get_mut(index) {
        *cell = (*cell & !TIER_MASK) | TIER_R2 | PIT_SEEN;
    }
    block.cells = encode_cells(&bytes);
    stamp(
        Stamps {
            first: &mut block.first_local_at,
            last: &mut block.last_at,
            provenance: &mut block.provenance,
            first_hand: &mut block.first_hand_at,
        },
        Instrument::Core,
        at,
        via,
    );
}

/// **`+tv/forget`'s rule** (D7; the brief's reviewer finding F3): `None` (delete the row) when
/// it holds no heard budget or the budget's hour is over at `now`; else the row with every
/// field, block and stand cleared and `heard_ic` kept, so forgetting never resets the
/// sounder's heard budget. A coverage-free budget-keeper comes back unchanged inside its hour
/// and `None` after it, so the next forget prunes it (the pre-commit round's item 6).
#[must_use]
pub fn forget_row(row: ChartRecord, now: i64) -> Option<ChartRecord> {
    match row.heard_ic {
        Some(heard) if !heard_line_due(Some(heard), now) => Some(ChartRecord {
            fields: BTreeMap::new(),
            blocks: BTreeMap::new(),
            scan_stand: None,
            sound_stand: None,
            ..row
        }),
        Some(_) | None => None,
    }
}

// ---- the site, for the render and the plans ---------------------------------------------------

/// **One lane site's ground, resolved** (doc 20 cut SM.4a): its port entry and definition, the
/// keyed geology it renders from, and the 256 × 256 window its composite is captured over.
#[derive(Clone, Debug)]
pub(crate) struct SiteContext {
    /// The site's port entry.
    pub entry: CanonicalId,
    /// The site's name.
    pub name: String,
    /// Its body's ground.
    pub body: Arc<BodyGeology>,
    /// Its geology spec (`crate::port::site::geology_spec`).
    pub spec: SiteGeologySpec,
    /// Its window: `region.<location>` at 256 × 256 cells of 10 m.
    pub frame: WindowFrame,
    /// The live key.
    pub mac: GeologyMac,
}

/// **Why a site's ground cannot be read** (D16, D25): each reads `INSTRUMENTS_DARK` to an act.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SiteDark {
    /// The geology layer is disabled (no key, a refused key).
    Layer,
    /// Not a lane site, or its lane rows do not read.
    NotSite,
    /// No table is installed for the site's body.
    NoTable,
    /// The site's spec does not form.
    Spec(String),
}

impl SiteDark {
    /// Why, for a log line.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            SiteDark::Layer => "the geology layer is disabled".to_owned(),
            SiteDark::NotSite => "not a lane site".to_owned(),
            SiteDark::NoTable => "no geology table for the site's body".to_owned(),
            SiteDark::Spec(error) => format!("the site's geology spec: {error}"),
        }
    }
}

/// **A site's window**: `region.<location>` (the region pack's id is the record's pack and entry
/// both), its ground surface, 256 × 256 cells of 10 m from `(0, 0)` (SM.2's `site_frame`, made
/// production).
#[must_use]
pub(crate) fn site_frame(location: &CanonicalId) -> Option<WindowFrame> {
    let region = CanonicalId::new(format!("region.{location}")).ok()?;
    let side = i64::from(crate::surface::ground::SITE_CELLS)
        .checked_mul(i64::from(crate::surface::ground::SITE_CELL_CM))?;
    Some(WindowFrame {
        region: mush_atlas::RecordKey::new(region.clone(), region),
        surfaces: vec![SurfaceRef::Ground],
        bbox: mush_atlas::RectCm {
            min: mush_atlas::PointCm { x: 0, y: 0 },
            max: mush_atlas::PointCm { x: side, y: side },
        },
        cell_cm: crate::surface::ground::SITE_CELL_CM,
    })
}

/// The location entry `id` in `pack`.
fn location_of<'p>(pack: &'p RulesPack, id: &CanonicalId) -> Option<&'p LocationDef> {
    match pack.entry(id) {
        Some(PackEntry::Location(location)) => Some(location),
        _ => None,
    }
}

/// **One lane site, as the chart names it**: its entry, its body, its name and its location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaneSite {
    /// The site's port entry.
    pub entry: CanonicalId,
    /// Its body.
    pub body: CanonicalId,
    /// Its name.
    pub name: String,
    /// Its location id.
    pub location: CanonicalId,
}

/// **Every lane site** (the report's lane pack: the latched version, else the shipped one), in
/// entry order: each port entry with a `site` block whose location stands on a body.
#[must_use]
pub(crate) fn lane_sites(w: &World) -> Vec<LaneSite> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    let (pack, ports) = crate::port::readiness::report_ports(store, w);
    let Some(pack) = pack else {
        return Vec::new();
    };
    ports
        .into_iter()
        .filter_map(|(entry, def)| {
            let site = def.site.as_ref()?;
            let body = location_of(pack, &def.location)?.body.clone()?;
            Some(LaneSite {
                entry,
                body,
                name: site.name.clone(),
                location: def.location.clone(),
            })
        })
        .collect()
}

/// **A lane site by a name a player or staff typed** (doc 20 cut SM.4b, factored out of
/// `+tv/forget`'s resolver): its name, its location id or its entry id, case-insensitive; a
/// multi-word name as typed (`Ahuna Flank`). `None` for anything else.
#[must_use]
pub(crate) fn site_named(w: &World, name: &str) -> Option<LaneSite> {
    let wanted = name.trim().to_lowercase();
    lane_sites(w).into_iter().find(|site| {
        site.name.to_lowercase() == wanted
            || site.location.as_str() == wanted
            || site.entry.as_str() == wanted
    })
}

/// **The site entries on `body`**, entry order, at most [`MAX_CHART_SITES_PER_BODY`] (the start
/// offer's census for the body caps).
#[must_use]
pub(crate) fn body_sites(w: &World, body: &CanonicalId) -> Vec<CanonicalId> {
    lane_sites(w)
        .into_iter()
        .filter(|site| &site.body == body)
        .map(|site| site.entry)
        .take(MAX_CHART_SITES_PER_BODY)
        .collect()
}

/// **A lane site's ground, resolved** (D16, D25): the lane's port definition and location, the
/// body's installed table, the site's spec and the live key.
///
/// # Errors
///
/// [`SiteDark`]: the layer disabled, not a lane site, no table, or a spec that does not form.
pub(crate) fn site_context(w: &World, entry: &CanonicalId) -> Result<SiteContext, SiteDark> {
    let (mac, _) = crate::surface::live(w).map_err(|_| SiteDark::Layer)?;
    let store = w.module_state::<TacsimStore>().ok_or(SiteDark::NotSite)?;
    let (pack, ports) = crate::port::readiness::report_ports(store, w);
    let pack = pack.ok_or(SiteDark::NotSite)?;
    let def = ports
        .into_iter()
        .find(|(id, _)| id == entry)
        .map(|(_, def)| def)
        .ok_or(SiteDark::NotSite)?;
    let site = def.site.as_ref().ok_or(SiteDark::NotSite)?;
    let location = location_of(pack, &def.location).ok_or(SiteDark::NotSite)?;
    let body_id = location.body.as_ref().ok_or(SiteDark::NotSite)?;
    let body = crate::surface::body_geology(w, body_id).ok_or(SiteDark::NoTable)?;
    let spec = crate::port::site::geology_spec(site, location, &body).map_err(SiteDark::Spec)?;
    let frame = site_frame(&def.location).ok_or(SiteDark::NotSite)?;
    Ok(SiteContext {
        entry: entry.clone(),
        name: crate::port::coop::port_name(def),
        body,
        spec,
        frame,
        mac,
    })
}

/// **A site field's ground**: [`field_ground`] at the body lattice field `(origin / 8 + fx,
/// origin / 8 + fy)` under the field's block's authored shift — exactly the site table's
/// build (`crate::surface::ground::site_geology`), so the render is the ground.
///
/// # Errors
///
/// The geology's refusal, or a field off the site.
pub(crate) fn field_ground_at(
    ctx: &SiteContext,
    (fx, fy): (i64, i64),
) -> Result<FieldGround, GeologyError> {
    if field_index(fx, fy).is_none() {
        return Err(GeologyError::OffLattice);
    }
    let origin = (
        ctx.spec.origin.0.div_euclid(POCKETS_PER_FIELD_SIDE),
        ctx.spec.origin.1.div_euclid(POCKETS_PER_FIELD_SIDE),
    );
    let block = usize::try_from(
        fy.div_euclid(2)
            .saturating_mul(SITE_BLOCKS)
            .saturating_add(fx.div_euclid(2)),
    )
    .unwrap_or(0);
    let shift = ctx.spec.shifts.get(block).copied().unwrap_or(0);
    field_ground(
        &ctx.mac,
        &ctx.body,
        (origin.0.saturating_add(fx), origin.1.saturating_add(fy)),
        shift,
    )
}

/// **A site pocket's row** through its field's ground.
///
/// # Errors
///
/// The geology's refusal, or a pocket off the site.
pub(crate) fn pocket_at(
    ctx: &SiteContext,
    (px, py): (i64, i64),
) -> Result<PocketRow, GeologyError> {
    let field = field_of_pocket((px, py)).ok_or(GeologyError::OffLattice)?;
    field_ground_at(ctx, field)?
        .pocket(
            px.rem_euclid(POCKETS_PER_FIELD_SIDE),
            py.rem_euclid(POCKETS_PER_FIELD_SIDE),
        )
        .ok_or(GeologyError::OffLattice)
}

// ---- the render ----------------------------------------------------------------------------------

/// **A field's row text** from its R1 readings (the brief's 3.8 "Rows"): `"{field} {words}
/// {family} ({lo}–{hi} pockets)"` per family whose reading is not exactly barren, joined `"; "`
/// in [`OreFamily::ALL`] order; all four exactly barren → `"{field} barren"`; ice →
/// `"{field} no return"`. `{lo}–{hi}` is [`FieldReading::pockets`], the union the reading
/// admits (a band never claims a count that excludes the truth).
#[must_use]
pub fn field_row_text(name: &str, readings: &[FieldReading; 4], footing: Footing) -> String {
    if footing == Footing::Ice {
        return format!("{name} no return");
    }
    let parts: Vec<String> = OreFamily::ALL
        .into_iter()
        .zip(readings.iter())
        .filter(|(_, reading)| **reading != FieldReading::Exact(Word::Barren))
        .map(|(family, reading)| match reading.pockets() {
            Some((lo, hi)) => format!("{} {} ({lo}–{hi} pockets)", reading.words(), family.label()),
            None => format!("{} {}", reading.words(), family.label()),
        })
        .collect();
    if parts.is_empty() {
        format!("{name} barren")
    } else {
        format!("{name} {}", parts.join("; "))
    }
}

/// **A pocket's text** from its R2 classes: `"{addr} {class} {family}"` per family with ore,
/// joined `", "`, else `"{addr} barren"`.
#[must_use]
pub fn pocket_row_text(addr: &str, classes: &[Word; 4]) -> String {
    let parts: Vec<String> = OreFamily::ALL
        .into_iter()
        .zip(classes.iter())
        .filter(|(_, class)| **class != Word::Barren)
        .map(|(family, class)| format!("{} {}", class.name(), family.label()))
        .collect();
    if parts.is_empty() {
        format!("{addr} barren")
    } else {
        format!("{addr} {}", parts.join(", "))
    }
}

/// **A field's reading for its viewer** (§5): `NO_READING` when the row is absent, under
/// another rule, or does not cover the field; else its row text at the row's (sharpest)
/// footing. Byte-identical `NO_READING` for an unknown site and an uncovered field.
#[must_use]
pub(crate) fn field_text(
    ctx: &SiteContext,
    row: Option<&ChartRecord>,
    (fx, fy): (i64, i64),
) -> String {
    let no = || crate::port::tv::NO_READING.to_owned();
    let Some(footing) = row_reads_field(row, (fx, fy)) else {
        return no();
    };
    let Some(name) = field_name(fx, fy) else {
        return no();
    };
    match field_ground_at(ctx, (fx, fy)) {
        Ok(ground) => field_row_text(
            &name,
            &read_r1(&ctx.mac, &ctx.body, &ground, footing),
            footing,
        ),
        Err(error) => {
            tracing::warn!(site = %ctx.entry, field = ?(fx, fy), %error, "tacsim: a covered field's ground does not read");
            no()
        }
    }
}

/// **A pocket's reading for its viewer** (§5): `NO_READING` when the row is absent, under
/// another rule, or has not sounded the pocket — byte-identical for an unknown site, an
/// unsounded pocket and a barren pocket never sounded; else its pocket text.
#[must_use]
pub(crate) fn pocket_text(
    ctx: &SiteContext,
    row: Option<&ChartRecord>,
    (px, py): (i64, i64),
) -> String {
    let no = || crate::port::tv::NO_READING.to_owned();
    if !row_reads_pocket(row, (px, py)) {
        return no();
    }
    let Some(addr) = pocket_address(px, py) else {
        return no();
    };
    match pocket_at(ctx, (px, py)) {
        Ok(pocket) => pocket_row_text(&addr, &read_r2(&pocket)),
        Err(error) => {
            tracing::warn!(site = %ctx.entry, %addr, %error, "tacsim: a sounded pocket's ground does not read");
            no()
        }
    }
}

/// **A field's reading for its viewer, from the world** (doc 20 cut SM.4a): `field_text` over
/// the lane site `site` with `row` — the viewer's own row for that site, or none — and
/// `NO_READING` when the site's ground cannot be resolved or the row is another site's. The
/// one public render door for a reading's **text**: the acceptance tests read every reading
/// through it. The `site.map` frame (doc 20 cut SM.4b) draws glyphs and a family-restricted list
/// rather than this text, so it renders through the same two pieces the door is made of — the
/// one gate (`row_reads_field`, `row_reads_pocket`) and the same pure reads (`read_r1`,
/// `read_r2`) over the same keyed ground (`field_ground_at`) — and so shows exactly what the
/// door would, never a reading the door would not (the review's correctness/F6; the map's D11
/// holds the two equal).
#[must_use]
pub fn field_reading(
    w: &World,
    site: &CanonicalId,
    row: Option<&ChartRecord>,
    field: (i64, i64),
) -> String {
    let row = row.filter(|row| &row.site == site);
    match site_context(w, site) {
        Ok(ctx) => field_text(&ctx, row, field),
        Err(_) => crate::port::tv::NO_READING.to_owned(),
    }
}

/// **A pocket's reading for its viewer, from the world** (doc 20 cut SM.4a): `pocket_text`
/// through the same door as [`field_reading`].
#[must_use]
pub fn pocket_reading(
    w: &World,
    site: &CanonicalId,
    row: Option<&ChartRecord>,
    pocket: (i64, i64),
) -> String {
    let row = row.filter(|row| &row.site == site);
    match site_context(w, site) {
        Ok(ctx) => pocket_text(&ctx, row, pocket),
        Err(_) => crate::port::tv::NO_READING.to_owned(),
    }
}

/// **A scan's rows** (the completion block, D17): one `"  {row}"` per field of `fields` the row
/// covers, north-west first (rows from the north, each west to east), at the row's sharpest
/// footing. Each field's ground is read once.
#[must_use]
pub(crate) fn scan_rows(ctx: &SiteContext, row: &ChartRecord, fields: &[u8]) -> Vec<String> {
    let mut ordered: Vec<(i64, i64)> = fields.iter().filter_map(|i| field_of_index(*i)).collect();
    ordered.sort_by_key(|(fx, fy)| (std::cmp::Reverse(*fy), *fx));
    ordered
        .into_iter()
        .filter(|(fx, fy)| field_index(*fx, *fy).is_some_and(|i| row.fields.contains_key(&i)))
        .map(|field| format!("  {}", field_text(ctx, Some(row), field)))
        .collect()
}

/// **A sounding's family lines** (the completion block): per family with ore among `pockets`
/// (north-west first, as given), `"  {family}: {addr} {class}, …"`; empty when every pocket
/// reads barren. Each field's ground is read once; a pocket the row has not sounded is left
/// out.
#[must_use]
pub(crate) fn sound_lines(
    ctx: &SiteContext,
    row: &ChartRecord,
    pockets: &[(i64, i64)],
) -> Vec<String> {
    let mut grounds: BTreeMap<(i64, i64), Option<FieldGround>> = BTreeMap::new();
    let mut per_family: BTreeMap<u8, (OreFamily, Vec<String>)> = BTreeMap::new();
    for pocket in pockets {
        if !row.pocket_sounded(*pocket) {
            continue;
        }
        let (Some(field), Some(addr)) =
            (field_of_pocket(*pocket), pocket_address(pocket.0, pocket.1))
        else {
            continue;
        };
        let ground = grounds
            .entry(field)
            .or_insert_with(|| field_ground_at(ctx, field).ok());
        let Some(ground) = ground.as_ref() else {
            continue;
        };
        let Some(cell) = ground.pocket(
            pocket.0.rem_euclid(POCKETS_PER_FIELD_SIDE),
            pocket.1.rem_euclid(POCKETS_PER_FIELD_SIDE),
        ) else {
            continue;
        };
        let classes = read_r2(&cell);
        for family in OreFamily::ALL {
            let class = by_family(classes, family);
            if class != Word::Barren {
                per_family
                    .entry(family.code())
                    .or_insert_with(|| (family, Vec::new()))
                    .1
                    .push(format!("{addr} {}", class.name()));
            }
        }
    }
    per_family
        .into_values()
        .map(|(family, list)| format!("  {}: {}", family.label(), list.join(", ")))
        .collect()
}

// ---- coverage over the composite --------------------------------------------------------------

/// **The footing at a site cell** (§5): the composite cell's class as a [`Footing`], or the
/// class that gives none (`Void` off the grid).
///
/// # Errors
///
/// The terrain class when it gives no footing.
pub fn footing_at(surface: &SurfaceComposite, cell: (i64, i64)) -> Result<Footing, TerrainClass> {
    let class = composite_cell(cell)
        .and_then(|(x, y)| surface.cell_at(x, y))
        .map_or(TerrainClass::Void, |(class, _)| class);
    Footing::of(class).ok_or(class)
}

/// **The fields a scan from `stand` sees** (lead ruling 5; D5): over the fields in reach of
/// `scanner.scan_radius_cells`, the stand's own is always seen; any other is seen when the
/// atlas `line_of_sight` from the mast (`mast_cm` over the stand) to a point a mast's height
/// over the field's centre cell `(fx · 32 + 16, fy · 32 + 16)` is clear or partial. Returns
/// (seen, hidden) as field indexes, north-west first.
#[must_use]
pub fn seen_fields(
    surface: &SurfaceComposite,
    stand: (i64, i64),
    scanner: &ScannerDef,
) -> (Vec<u8>, Vec<u8>) {
    let own = address::field_of_cell(stand);
    let mast = i32::try_from(scanner.mast_cm).unwrap_or(i32::MAX);
    let mut seen = Vec::new();
    let mut hidden = Vec::new();
    let Some(eye) = composite_cell(stand) else {
        return (seen, hidden);
    };
    for field in address::fields_in_reach(stand, i64::from(scanner.scan_radius_cells)) {
        let Some(index) = field_index(field.0, field.1) else {
            continue;
        };
        if Some(field) == own {
            seen.push(index);
            continue;
        }
        let centre = (
            field
                .0
                .saturating_mul(address::CELLS_PER_FIELD)
                .saturating_add(16),
            field
                .1
                .saturating_mul(address::CELLS_PER_FIELD)
                .saturating_add(16),
        );
        let visible = composite_cell(centre).is_some_and(|target| {
            line_of_sight(
                surface,
                Eye {
                    cell: eye,
                    height_cm: mast,
                },
                Eye {
                    cell: target,
                    height_cm: mast,
                },
            )
            .is_clear_or_partial()
        });
        if visible {
            seen.push(index);
        } else {
            hidden.push(index);
        }
    }
    (seen, hidden)
}

/// **The nearest sharper footing** (D20): over Chebyshev rings `1..=radius` from `stand`, each
/// ring read row by row from its north-west, the first cell whose footing is sharper than
/// `footing`; its footing, the metres to it (cells × 10) and its octant. `None` within the
/// radius (or from rock, which nothing betters).
#[must_use]
pub fn better_footing(
    surface: &SurfaceComposite,
    stand: (i64, i64),
    footing: Footing,
    radius: i64,
) -> Option<(Footing, u64, Octant)> {
    for d in 1..=radius.max(0) {
        for dy in (-d..=d).rev() {
            for dx in -d..=d {
                if dx.abs().max(dy.abs()) != d {
                    continue;
                }
                let cell = (stand.0.saturating_add(dx), stand.1.saturating_add(dy));
                if let Ok(found) = footing_at(surface, cell)
                    && found.sharpness() > footing.sharpness()
                {
                    let metres = u64::try_from(d).unwrap_or(0).saturating_mul(10);
                    return Octant::of(dx, dy).map(|octant| (found, metres, octant));
                }
            }
        }
    }
    None
}

// ---- the non-building peek ----------------------------------------------------------------------

/// **What a peek at a site's ground found** (lead ruling 6; D11, D25).
#[derive(Clone, Debug)]
pub(crate) enum SitePeek {
    /// The layer is dark for this site: no key, no table, not a lane site, or its build or its
    /// capture failed this boot (`warm_failed`). Never wanted, never warmed.
    Dark,
    /// The layer is live and the site's ground is not cached yet: wanted.
    Cold,
    /// The cached ground.
    Warm(Arc<SiteGround>),
}

/// **Peek a site's ground, never building it**, at most once per site per call (`peeks` is the
/// call's own bundle — since doc 20 cut SM.4b `crate::surface::Peeks`, which carries the home
/// fields' peeks beside the sites'; the brief's reviewer finding F5): `site_context` dark →
/// `Dark`; a site whose build (or capture) failed this boot → `Dark`; `site_ground_cached` hit →
/// `Warm`, miss → `Cold`; a capture that refuses → `Dark`, and the site joins the once-a-boot dark
/// set (`crate::surface::fail_site`; the pre-commit round's item 5), so it is warned once a boot
/// and never captured again — not once a peek.
pub(crate) fn peek_site(
    w: &World,
    entry: &CanonicalId,
    peeks: &mut crate::surface::Peeks,
) -> SitePeek {
    let peeks = &mut peeks.sites;
    if let Some(found) = peeks.get(entry) {
        return found.clone();
    }
    let found = match site_context(w, entry) {
        Err(_) => SitePeek::Dark,
        Ok(_) if crate::surface::warm_failed(w, entry) => SitePeek::Dark,
        Ok(site) => match crate::surface::ground::site_ground_cached(w, &site.frame, &site.spec) {
            Ok(Some(ground)) => SitePeek::Warm(ground),
            Ok(None) => SitePeek::Cold,
            Err(error) => {
                if crate::surface::fail_site(w, entry) {
                    tracing::warn!(
                        site = %entry,
                        %error,
                        "tacsim: a site's ground cannot be captured; it reads dark until the next boot"
                    );
                }
                SitePeek::Dark
            }
        },
    };
    peeks.insert(entry.clone(), found.clone());
    found
}

/// The ground surface of a warm site.
#[must_use]
pub(crate) fn ground_surface(ground: &SiteGround) -> Option<&SurfaceComposite> {
    ground.composite.surface(&SurfaceRef::Ground)
}

// ---- tacsim_chart_forget ---------------------------------------------------------------------------

/// `+tv/forget`'s action (D9).
pub const CHART_FORGET_ACTION: &str = "tacsim_chart_forget";
/// Its outcome.
pub const CHART_FORGET_OUTCOME: &str = "tacsim_chart_forget_result";
/// Its idempotency namespace: `tacsim.chart/{actor}/forget/{target}/{now_ic}`.
pub const CHART_KEY_PREFIX: &str = "tacsim.chart/";

/// The forget's key.
#[must_use]
pub fn forget_key(
    actor: ObjId,
    target: &CanonicalId,
    now_ic: i64,
) -> Option<mush_core::IdempotencyKeyText> {
    mush_core::IdempotencyKeyText::parse_canonical(format!(
        "{CHART_KEY_PREFIX}{}/forget/{target}/{now_ic}",
        actor.0
    ))
    .ok()
}

/// **`tacsim_chart_forget`** — `+tv/forget <site>|<body> go`'s own authoritative action (D9): it
/// names no rig, takes no charge and has no tick finish. Self-scoped; its offer fixes `now` and
/// the lane's sites with their bodies; its prepare applies [`forget_row`] to each of the actor's
/// rows the target names — removing it, or keeping the coverage-empty budget-keeper — and since
/// the pre-commit round's item 6 a budget-keeper whose hour is over is removed too (it has no
/// other path to deletion); a keeper still inside its hour is left as it is. It refuses
/// "nothing to forget" only when no row would change.
pub(crate) struct ChartForgetProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub charts: StateFamilyHandle<ChartKey, ChartRecord>,
}

fn invalid(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::InvalidRequest, message)
}

/// The invocation's parameters: self-scoped, no targets, an object.
fn forget_parameters(invocation: &ActionInvocation) -> Result<&Map<String, Value>, ActionError> {
    if invocation.subject() != &mush_core::PerceivedRef::SelfActor
        || !invocation.targets().is_empty()
    {
        return Err(invalid("a forget is self-scoped and takes no targets"));
    }
    invocation
        .parameters()
        .as_object()
        .ok_or_else(|| invalid("a forget names a site or a body"))
}

impl ActionProvider for ChartForgetProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = forget_parameters(invocation)?;
        parameters
            .get("target")
            .and_then(Value::as_str)
            .and_then(|text| CanonicalId::new(text).ok())
            .ok_or_else(|| invalid("a forget names a site or a body"))?;
        Ok(())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !crate::surface::act::self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w: &World = &engine.world;
        if !w
            .module_state::<TacsimStore>()
            .is_some_and(TacsimStore::enabled)
        {
            return Ok(None);
        }
        let sites = lane_sites(w);
        if sites.is_empty() {
            return Ok(None);
        }
        let mut targets: BTreeSet<String> = BTreeSet::new();
        let mut fixed_sites: BTreeMap<String, String> = BTreeMap::new();
        for site in &sites {
            targets.insert(site.entry.to_string());
            targets.insert(site.body.to_string());
            fixed_sites.insert(site.entry.to_string(), site.body.to_string());
        }
        let fixed = json!({
            "now": w.world_ic_now().at,
            "sites": fixed_sites,
        });
        let parameters = vec![crate::surface::act::parameter(
            "target",
            true,
            ProjectedParameterKind::Choice {
                choices: targets.into_iter().collect(),
            },
        )?];
        crate::surface::act::offer_draft(
            action,
            engine,
            fixed,
            parameters,
            (
                "tacsim-chart-forget",
                "Forget readings",
                "Strike your readings of a site, or of every site on a body, from your chart.",
            ),
        )
        .map(Some)
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let parameters = forget_parameters(invocation)?;
        let actor = invocation.actor();
        let now = parameters
            .get("now")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid("a forget's offer fixes the instant"))?;
        let target = parameters
            .get("target")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("a forget names a site or a body"))?;
        let sites: BTreeMap<String, String> = parameters
            .get("sites")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| invalid("a forget's offer fixes the sites"))?;
        let named: Vec<&String> = if sites.contains_key(target) {
            sites
                .keys()
                .filter(|site| site.as_str() == target)
                .collect()
        } else {
            sites
                .iter()
                .filter(|(_, body)| body.as_str() == target)
                .map(|(site, _)| site)
                .collect()
        };
        if named.is_empty() {
            return Err(invalid("a forget names a site or a body"));
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let mut rows = 0_u32;
        for site in named {
            let site = CanonicalId::new(site).map_err(|_| invalid("a forget names a site"))?;
            let key = ChartKey {
                principal: actor,
                site,
            };
            let found = txn
                .get(&self.charts, &key)
                .map_err(|_| integrity("the charts are unavailable"))?
                .cloned();
            let Some(row) = found else {
                continue;
            };
            if row.key() != key {
                return Err(integrity("a chart row disagrees with its own key"));
            }
            let before = row.clone();
            match forget_row(row, now) {
                None => txn
                    .remove(&self.charts, &key)
                    .map_err(|_| integrity("the chart row did not stage"))?,
                // A budget-keeper still inside its hour: nothing changes.
                Some(kept) if kept == before => continue,
                Some(kept) => {
                    kept.check()
                        .map_err(|_| integrity("the chart row is out of bounds"))?;
                    txn.replace(&self.charts, &key, kept)
                        .map_err(|_| integrity("the chart row did not stage"))?;
                }
            }
            rows = rows.saturating_add(1);
        }
        if rows == 0 {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "nothing to forget",
            ));
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "actor": actor.0, "rows": rows }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the forget's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap_or_else(|_| panic!("{text} is canonical"))
    }

    fn row() -> ChartRecord {
        ChartRecord::new(ObjId(42), cid("ahuna_port"), cid("ceres"))
    }

    /// **What a scan and a sounding add**: a fresh row takes anything; a field read at
    /// regolith is added again only from rock; a sounded pocket adds nothing.
    #[test]
    fn sm4a_scan_and_sound_adds() {
        assert!(scan_adds(None, &[27], Footing::Ice));
        assert!(!scan_adds(None, &[], Footing::Rock));
        let mut r = row();
        write_scan(
            &mut r,
            [128, 124],
            &[27, 28],
            Footing::Regolith,
            1_000,
            Via::Hand,
        );
        assert!(!scan_adds(Some(&r), &[27, 28], Footing::Regolith));
        assert!(!scan_adds(Some(&r), &[27], Footing::Ice), "never duller");
        assert!(scan_adds(Some(&r), &[27], Footing::Rock), "sharper");
        assert!(
            scan_adds(Some(&r), &[27, 29], Footing::Regolith),
            "a new field"
        );
        let pockets = address::pockets_in_reach((112, 112), 8);
        assert!(sound_adds(Some(&r), &pockets));
        write_sound(&mut r, [112, 112], &pockets, true, 2_000, Via::Hand);
        assert!(!sound_adds(Some(&r), &pockets));
        assert!(sound_adds(
            Some(&r),
            &address::pockets_in_reach((112, 112), 16)
        ));
        assert_eq!(r.sounded_pockets(), 16);
        assert_eq!(r.check(), Ok(()));
        // A row under another rule adds everything (it would be reset).
        let mut old = r.clone();
        old.rule = 99;
        assert!(scan_adds(Some(&old), &[27], Footing::Ice));
        assert!(sound_adds(Some(&old), &pockets));
    }

    /// **The repeat rule is the same cell** (D4; F2): the same stand with nothing added is
    /// refused; another stand with nothing added runs; a loud sounding after a quiet one adds
    /// pockets and runs, a quiet one after a loud one adds nothing and is refused; the scan's
    /// and the sounding's stands are independent.
    #[test]
    fn sm4a_the_repeat_rule_is_the_same_cell() {
        let mut r = row();
        let stand = [112, 112];
        assert!(
            !repeat_refused(None, false, stand, false),
            "no row, nothing to repeat"
        );
        write_scan(&mut r, stand, &[27], Footing::Rock, 1_000, Via::Hand);
        assert!(repeat_refused(Some(&r), false, stand, false));
        assert!(!repeat_refused(Some(&r), false, stand, true), "it adds");
        assert!(
            !repeat_refused(Some(&r), false, [113, 112], false),
            "another cell"
        );
        assert!(
            !repeat_refused(Some(&r), true, stand, false),
            "the sounding is apart"
        );
        let quiet = address::pockets_in_reach((112, 112), 8);
        let loud = address::pockets_in_reach((112, 112), 16);
        write_sound(&mut r, stand, &quiet, true, 2_000, Via::Hand);
        let adds = sound_adds(Some(&r), &loud);
        assert!(
            adds && !repeat_refused(Some(&r), true, stand, adds),
            "loud after quiet"
        );
        write_sound(&mut r, stand, &loud, false, 3_000, Via::Hand);
        let adds = sound_adds(Some(&r), &quiet);
        assert!(
            !adds && repeat_refused(Some(&r), true, stand, adds),
            "quiet after loud"
        );
        assert!(
            repeat_refused(Some(&r), false, stand, false),
            "the scan's stand stands"
        );
    }

    /// **Forget keeps the heard budget** (D7; F3): a row heard at 1,000 forgotten at 4,599 keeps
    /// a coverage-free row holding only `heard_ic` (which checks), at 4,600 it is deleted; a
    /// row never heard is deleted.
    #[test]
    fn sm4a_forget_keeps_the_heard_budget() {
        let mut r = row();
        let pockets = address::pockets_in_reach((112, 112), 16);
        write_sound(&mut r, [112, 112], &pockets, false, 1_000, Via::Hand);
        r.heard_ic = Some(1_000);
        let kept = forget_row(r.clone(), 1_000 + 3_599).expect("inside the hour");
        assert!(!kept.has_coverage());
        assert_eq!(kept.heard_ic, Some(1_000));
        assert_eq!((kept.scan_stand, kept.sound_stand), (None, None));
        assert_eq!(kept.check(), Ok(()));
        assert_eq!(forget_row(r.clone(), 1_000 + 3_600), None);
        let mut never = r;
        never.heard_ic = None;
        assert_eq!(forget_row(never, 1_000), None);
        // The tenth sounding in an hour.
        assert!(heard_line_due(None, 0));
        assert!(!heard_line_due(Some(0), 3_599));
        assert!(heard_line_due(Some(0), 3_600));
    }

    /// **The tenth sounding in an IC hour prints no line** (K6, pure): a sounder sounding every
    /// 360 IC s from 0 is heard at 0 and next at 3,600; the nine between are not.
    #[test]
    fn sm4a_the_tenth_sounding_in_an_ic_hour_prints_no_line() {
        let mut heard: Option<i64> = None;
        let mut lines = Vec::new();
        for n in 0..11_i64 {
            let at = n * 360;
            if heard_line_due(heard, at) {
                heard = Some(at);
                lines.push(at);
            }
        }
        assert_eq!(lines, vec![0, 3_600]);
    }

    /// **The body caps** (D1, D3): rows of other sites count whole, the site's own row counts
    /// its union with the act's new units, and the caps are inclusive.
    #[test]
    fn sm4a_the_body_caps() {
        let site = cid("ahuna_port");
        let mut full = Vec::new();
        for n in 0..15 {
            let mut other = ChartRecord::new(ObjId(42), cid(&format!("site_{n}")), cid("ceres"));
            for index in 0..64_u8 {
                write_scan(&mut other, [0, 0], &[index], Footing::Rock, 1, Via::Hand);
            }
            full.push(other);
        }
        // 15 × 64 = 960 field rows elsewhere; the site may add 64.
        let all: Vec<u8> = (0..64).collect();
        assert!(body_admits(&full, &site, &all, &[]));
        let mut here = row();
        write_scan(&mut here, [0, 0], &all, Footing::Rock, 1, Via::Hand);
        full.push(here);
        assert!(
            body_admits(&full, &site, &[0], &[]),
            "already held is not new"
        );
        let mut sixteenth = ChartRecord::new(ObjId(42), cid("site_x"), cid("ceres"));
        write_scan(&mut sixteenth, [0, 0], &[0], Footing::Rock, 1, Via::Hand);
        full.push(sixteenth);
        assert!(!body_admits(&full, &site, &[], &[]), "1,025 field rows");
        // Blocks: four full sites of 16 blocks is the cap.
        let letters: Vec<String> = crate::port::def::BLOCK_LETTERS
            .iter()
            .map(char::to_string)
            .collect();
        let mut rows = Vec::new();
        for n in 0..3 {
            let mut other = ChartRecord::new(ObjId(42), cid(&format!("site_{n}")), cid("ceres"));
            for letter in &letters {
                other.blocks.insert(letter.clone(), PocketBlock::empty(1));
            }
            rows.push(other);
        }
        assert!(body_admits(&rows, &site, &[], &letters));
        let mut extra = letters.clone();
        extra.push("A".to_owned());
        assert!(body_admits(&rows, &site, &[], &extra), "a set, not a list");
        let mut more = ChartRecord::new(ObjId(42), cid("site_z"), cid("ceres"));
        more.blocks.insert("A".to_owned(), PocketBlock::empty(1));
        rows.push(more);
        assert!(!body_admits(&rows, &site, &[], &letters));
    }

    /// **A worst-case row encodes under its cap**: 64 fields and 16 full blocks, four
    /// provenance entries each at the widest stamps, through `serde_json::to_vec`.
    #[test]
    fn sm4a_a_worst_case_chart_row_encodes_under_its_cap() {
        let mut r = ChartRecord::new(ObjId(u32::MAX), cid("rheasilvia_port"), cid("vesta"));
        let all: Vec<u8> = (0..64).collect();
        let every: Vec<(i64, i64)> = (0..64)
            .flat_map(|py| (0..64).map(move |px| (px, py)))
            .collect();
        for n in 0..4 {
            let at = i64::MAX - n;
            write_scan(&mut r, [255, 255], &all, Footing::Rock, at, Via::Hand);
            write_sound(&mut r, [255, 255], &every, false, at, Via::Hand);
        }
        r.heard_ic = Some(i64::MIN);
        assert_eq!(r.check(), Ok(()));
        let bytes = serde_json::to_vec(&r).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            bytes.len() < MAX_CHART_RECORD_BYTES,
            "{} bytes",
            bytes.len()
        );
        assert_eq!(r.sounded_pockets(), 4_096);
    }

    /// **A chart row checks its bounds**: the schema, the rule, a coverage-free row without a
    /// budget or with a stand, a stand off the site, a field index 64, a block letter `Q`, a
    /// short or upper-case or reserved-bit (bit 3) or tier-1 or pit-without-a-tier or all-zero
    /// block, stamps out of order, no provenance or five; the wire is strict and the key text
    /// strict.
    #[test]
    fn sm4a_a_chart_row_checks_its_bounds() {
        let mut ok = row();
        write_scan(
            &mut ok,
            [128, 124],
            &[27],
            Footing::Regolith,
            1_000,
            Via::Hand,
        );
        write_sound(&mut ok, [128, 124], &[(32, 31)], false, 1_000, Via::Hand);
        assert_eq!(ok.check(), Ok(()));
        let mut broken = vec![
            ChartRecord {
                schema: 2,
                ..ok.clone()
            },
            ChartRecord {
                rule: 0,
                ..ok.clone()
            },
            row(),
            ChartRecord {
                heard_ic: Some(1),
                scan_stand: Some([1, 1]),
                ..row()
            },
            ChartRecord {
                scan_stand: Some([256, 0]),
                ..ok.clone()
            },
        ];
        let mut field = ok.clone();
        if let Some(entry) = field.fields.remove(&27) {
            field.fields.insert(64, entry);
        }
        broken.push(field);
        let mut letter = ok.clone();
        if let Some(block) = letter.blocks.remove("K") {
            letter.blocks.insert("Q".to_owned(), block);
        }
        broken.push(letter);
        for cells in [
            "00".repeat(255),
            format!("{}0A", "00".repeat(255)),
            format!("{}0a", "00".repeat(255)),
            format!("{}01", "00".repeat(255)),
            format!("{}0402", "00".repeat(254)),
            "00".repeat(256),
        ] {
            let mut bad = ok.clone();
            if let Some(block) = bad.blocks.get_mut("K") {
                block.cells = cells;
            }
            broken.push(bad);
        }
        let mut order = ok.clone();
        if let Some(entry) = order.fields.get_mut(&27) {
            entry.first_local_at = 2_000;
        }
        broken.push(order);
        let mut none = ok.clone();
        if let Some(entry) = none.fields.get_mut(&27) {
            entry.provenance.clear();
        }
        broken.push(none);
        let mut five = ok.clone();
        if let Some(entry) = five.fields.get_mut(&27) {
            for _ in 0..4 {
                entry.provenance.push(ChartProvenance::Local {
                    instrument: Instrument::Scan,
                    at: 1_000,
                    via: Via::Hand,
                });
            }
        }
        broken.push(five);
        for record in broken {
            assert!(record.check().is_err(), "{record:?} passed its check");
        }
        // A budget-keeper alone checks.
        let keeper = ChartRecord {
            heard_ic: Some(5),
            ..row()
        };
        assert_eq!(keeper.check(), Ok(()));
        // The ring keeps the newest four and the earliest first stamp.
        let mut ring = row();
        for at in [10, 20, 30, 40, 50] {
            write_scan(&mut ring, [0, 0], &[0], Footing::Ice, at, Via::Hand);
        }
        let entry = ring
            .fields
            .get(&0)
            .cloned()
            .unwrap_or_else(|| panic!("a row"));
        assert_eq!(entry.first_local_at, 10);
        assert_eq!(entry.last_at, 50);
        assert_eq!(
            entry
                .provenance
                .iter()
                .map(ChartProvenance::at)
                .collect::<Vec<_>>(),
            vec![20, 30, 40, 50]
        );
        // The wire.
        let wire = serde_json::to_value(&ok).unwrap_or_else(|e| panic!("{e}"));
        assert!(wire.get("heard_ic").is_none());
        let back: ChartRecord =
            serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, ok);
        let mut extra = wire;
        extra["surprise"] = json!(1);
        assert!(serde_json::from_value::<ChartRecord>(extra).is_err());
        let key = ok.key();
        assert_eq!(key.key_text(), "42/ahuna_port");
        assert_eq!(ChartKey::parse("42/ahuna_port"), Some(key));
        for bad in ["042/ahuna_port", "42", "#42/ahuna_port", "42/a b", "42/a/b"] {
            assert_eq!(ChartKey::parse(bad), None, "{bad:?}");
        }
    }

    /// **The pocket byte has a layout** (the pre-commit round's item 3): a cell carrying SM.7's
    /// [`PIT_SEEN`] beside its R2 tier reads sounded (`pocket_sounded`, `PocketBlock::sounded`,
    /// `sound_adds`), checks, and keeps its pit bit across a re-sounding, which sets the tier of
    /// the cells it reaches and nothing else.
    #[test]
    fn sm4a_the_pit_seen_bit_reads_sounded_and_survives_a_resounding() {
        let mut r = row();
        let pockets = address::pockets_in_reach((112, 112), 8);
        write_sound(&mut r, [112, 112], &pockets, true, 1_000, Via::Hand);
        let pit = pockets[0];
        let (letter, index) = pocket_in_block(pit.0, pit.1).unwrap_or_else(|| panic!("a pocket"));
        let set_pit = |r: &mut ChartRecord| {
            let block = r
                .blocks
                .get_mut(letter.to_string().as_str())
                .unwrap_or_else(|| panic!("the block"));
            let mut bytes = block.bytes().unwrap_or_else(|| panic!("the cells"));
            bytes[index] |= PIT_SEEN;
            block.cells = encode_cells(&bytes);
        };
        set_pit(&mut r);
        let byte = |r: &ChartRecord| {
            r.blocks
                .get(letter.to_string().as_str())
                .and_then(|block| block.cell(index))
        };
        assert_eq!(byte(&r), Some(TIER_R2 | PIT_SEEN));
        assert_eq!(cell_tier(TIER_R2 | PIT_SEEN), TIER_R2);
        assert!(r.pocket_sounded(pit), "the pit bit reads sounded");
        assert_eq!(r.sounded_pockets(), 16);
        assert!(!sound_adds(Some(&r), &pockets), "nothing new to sound");
        assert_eq!(r.check(), Ok(()));
        // A loud re-sounding over the same cells keeps the bit and adds the ring around them.
        write_sound(
            &mut r,
            [113, 112],
            &address::pockets_in_reach((112, 112), 16),
            false,
            2_000,
            Via::Hand,
        );
        assert_eq!(byte(&r), Some(TIER_R2 | PIT_SEEN), "the pit bit survives");
        assert_eq!(r.sounded_pockets(), 64);
        assert_eq!(r.check(), Ok(()));
    }

    /// The byte of `pocket`'s cell in `r`.
    fn cell_of(r: &ChartRecord, pocket: (i64, i64)) -> Option<u8> {
        let (letter, index) = pocket_in_block(pocket.0, pocket.1)?;
        r.blocks
            .get(letter.to_string().as_str())
            .and_then(|block| block.cell(index))
    }

    /// **A core writes R2 and the pit bit** (doc 20 cut SM.7; the brief's 3.9): on a fresh row the
    /// pocket's block is created and its cell reads `TIER_R2 | PIT_SEEN`, stamped `Local { Core, at,
    /// Hand }`; on a row whose pocket was sounded the cell keeps R2 and gains the bit; a re-sounding
    /// keeps the bit; the block's ring holds the Core stamp in stamp order; neither stand moves.
    #[test]
    fn sm7_a_core_writes_r2_and_the_pit_bit() {
        let pocket = (36, 44);
        let mut fresh = row();
        write_core(&mut fresh, pocket, 5_000, Via::Hand);
        assert_eq!(cell_of(&fresh, pocket), Some(TIER_R2 | PIT_SEEN));
        assert_eq!(fresh.sounded_pockets(), 1);
        assert_eq!(
            (fresh.scan_stand, fresh.sound_stand),
            (None, None),
            "no stand"
        );
        assert_eq!(fresh.check(), Ok(()));
        let (letter, _) = pocket_in_block(pocket.0, pocket.1).unwrap_or_else(|| panic!("a pocket"));
        let block = fresh
            .blocks
            .get(letter.to_string().as_str())
            .cloned()
            .unwrap_or_else(|| panic!("the block"));
        assert_eq!(
            block.provenance,
            vec![ChartProvenance::Local {
                instrument: Instrument::Core,
                at: 5_000,
                via: Via::Hand
            }]
        );
        // A sounded pocket gains the bit and keeps its tier; the stand of the sounding stays.
        let stand = [pocket.0 * 4 + 2, pocket.1 * 4 + 2];
        let mut sounded = row();
        write_sound(
            &mut sounded,
            stand,
            &address::pockets_in_reach((stand[0], stand[1]), 8),
            false,
            1_000,
            Via::Hand,
        );
        assert_eq!(cell_of(&sounded, pocket), Some(TIER_R2));
        write_core(&mut sounded, pocket, 3_000, Via::Hand);
        assert_eq!(cell_of(&sounded, pocket), Some(TIER_R2 | PIT_SEEN));
        assert_eq!(
            sounded.sound_stand,
            Some(stand),
            "the stand is the sounding's"
        );
        // A re-sounding keeps the bit.
        write_sound(
            &mut sounded,
            stand,
            &address::pockets_in_reach((stand[0], stand[1]), 8),
            true,
            4_000,
            Via::Hand,
        );
        assert_eq!(cell_of(&sounded, pocket), Some(TIER_R2 | PIT_SEEN));
        let ring: Vec<(Instrument, i64)> = sounded
            .blocks
            .get(letter.to_string().as_str())
            .map(|block| {
                block
                    .provenance
                    .iter()
                    .map(|entry| match entry {
                        ChartProvenance::Local { instrument, at, .. } => (*instrument, *at),
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            ring,
            vec![
                (Instrument::Sound, 1_000),
                (Instrument::Core, 3_000),
                (Instrument::QuietSound, 4_000)
            ],
            "stamp order"
        );
        assert_eq!(sounded.check(), Ok(()));
        // The wire word.
        assert_eq!(
            serde_json::to_value(Instrument::Core).unwrap_or_default(),
            serde_json::json!("core")
        );
    }

    /// **The pit bit reads as sounded** (doc 20 cut SM.7): a cored pocket is `pocket_sounded`, the
    /// one gate every surface renders through reads it, and a sounding over it adds nothing; a
    /// byte carrying the pit bit with no tier beside it is refused by `check` (the SM.7 lead's
    /// pre-commit round, item 8 — the R2 text through the render door is `tests/mining.rs` M1's,
    /// which reads `pocket_reading` over the keyed ground).
    #[test]
    fn sm7_the_pit_bit_reads_as_sounded() {
        let pocket = (36, 44);
        let mut r = row();
        write_core(&mut r, pocket, 5_000, Via::Hand);
        assert!(r.pocket_sounded(pocket));
        assert!(row_reads_pocket(Some(&r), pocket));
        assert!(!sound_adds(Some(&r), &[pocket]));
        assert!(!r.pocket_sounded((37, 44)), "only the cored pocket");
        assert_eq!(cell_of(&r, pocket), Some(TIER_R2 | PIT_SEEN));
        assert_eq!(r.check(), Ok(()));
        // The pit bit alone, no tier: refused.
        let (letter, index) =
            pocket_in_block(pocket.0, pocket.1).unwrap_or_else(|| panic!("a pocket"));
        let mut bad = r.clone();
        let block = bad
            .blocks
            .get_mut(letter.to_string().as_str())
            .unwrap_or_else(|| panic!("the block"));
        let mut bytes = block.bytes().unwrap_or_else(|| panic!("the cells"));
        bytes[index] = PIT_SEEN;
        block.cells = encode_cells(&bytes);
        assert!(
            bad.check().is_err(),
            "the pit bit needs the R2 tier beside it"
        );
        assert!(!bad.pocket_sounded(pocket), "and never reads sounded");
    }

    /// **The provenance ring is ordered by stamp** (the pre-commit round's item 4): finishes
    /// landing out of order — 50, 10, 40, 20, 30, then 5 and 45 — leave the four newest stamps,
    /// oldest first, whatever order they came in (5 never enters a full ring of newer ones; 45
    /// evicts the oldest, 20); ties keep their insertion order; `first_local_at` is the earliest
    /// stamp ever written and `last_at` the latest.
    #[test]
    fn sm4a_the_provenance_ring_is_ordered_by_stamp() {
        let mut r = row();
        for at in [50, 10, 40, 20, 30] {
            write_scan(&mut r, [0, 0], &[0], Footing::Rock, at, Via::Hand);
        }
        let ring = |r: &ChartRecord| {
            let entry = r.fields.get(&0).cloned().unwrap_or_else(|| panic!("a row"));
            (
                entry.first_local_at,
                entry.last_at,
                entry
                    .provenance
                    .iter()
                    .map(ChartProvenance::at)
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(ring(&r), (10, 50, vec![20, 30, 40, 50]));
        write_scan(&mut r, [0, 0], &[0], Footing::Rock, 5, Via::Hand);
        assert_eq!(
            ring(&r),
            (5, 50, vec![20, 30, 40, 50]),
            "an older one never evicts"
        );
        write_scan(&mut r, [0, 0], &[0], Footing::Rock, 45, Via::Hand);
        assert_eq!(ring(&r), (5, 50, vec![30, 40, 45, 50]));
        assert_eq!(r.check(), Ok(()));
        // Ties keep their insertion order: a quiet then a loud sounding at one instant.
        let mut b = row();
        let pockets = address::pockets_in_reach((112, 112), 8);
        write_sound(&mut b, [112, 112], &pockets, false, 70, Via::Hand);
        write_sound(&mut b, [112, 112], &pockets, true, 60, Via::Hand);
        write_sound(&mut b, [112, 112], &pockets, false, 60, Via::Hand);
        let block = b
            .blocks
            .values()
            .next()
            .cloned()
            .unwrap_or_else(|| panic!("a block"));
        let instruments: Vec<(i64, Instrument)> = block
            .provenance
            .iter()
            .map(|entry| match entry {
                ChartProvenance::Local { instrument, at, .. } => (*at, *instrument),
            })
            .collect();
        assert_eq!(
            instruments,
            vec![
                (60, Instrument::QuietSound),
                (60, Instrument::Sound),
                (70, Instrument::Sound)
            ]
        );
        assert_eq!((block.first_local_at, block.last_at), (60, 70));
    }

    /// **A scan never dulls a footing** (the pre-commit round's item 7 g; §5): a field read from
    /// rock and then scanned from regolith (and from ice) keeps its rock footing — the rock
    /// reading is never replaced by a regolith band — while each later scan still stamps its
    /// provenance and moves `last_at`; a sharper footing replaces a duller one.
    #[test]
    fn sm4a_a_scan_never_dulls_a_footing() {
        let mut r = row();
        write_scan(&mut r, [16, 16], &[0], Footing::Rock, 100, Via::Hand);
        write_scan(&mut r, [17, 16], &[0], Footing::Regolith, 200, Via::Hand);
        write_scan(&mut r, [18, 16], &[0], Footing::Ice, 300, Via::Hand);
        let entry = r.fields.get(&0).cloned().unwrap_or_else(|| panic!("a row"));
        assert_eq!(entry.footing, Footing::Rock, "never duller");
        assert_eq!((entry.first_local_at, entry.last_at), (100, 300));
        assert_eq!(entry.provenance.len(), 3);
        assert_eq!(r.scan_stand, Some([18, 16]));
        let mut sharpen = row();
        write_scan(&mut sharpen, [16, 16], &[0], Footing::Ice, 100, Via::Hand);
        write_scan(
            &mut sharpen,
            [16, 16],
            &[0],
            Footing::Regolith,
            200,
            Via::Hand,
        );
        assert_eq!(
            sharpen.fields.get(&0).map(|f| f.footing),
            Some(Footing::Regolith)
        );
        write_scan(&mut sharpen, [16, 16], &[0], Footing::Rock, 300, Via::Hand);
        assert_eq!(
            sharpen.fields.get(&0).map(|f| f.footing),
            Some(Footing::Rock)
        );
    }

    /// **The rows speak the one word set** (§3.2): exact words, bands with the en dash and the
    /// union of their pocket ranges, barren left out, all-barren and ice in their own words;
    /// pocket texts by class and family. Never P0–P3 or A–D.
    #[test]
    fn sm4a_rows_render_in_the_one_word_set() {
        let rows = [
            FieldReading::Band(Word::Fair, Word::Rich),
            FieldReading::Exact(Word::Lean),
            FieldReading::Exact(Word::Barren),
            FieldReading::Band(Word::Barren, Word::Lean),
        ];
        assert_eq!(
            field_row_text("C-NW", &rows, Footing::Regolith),
            "C-NW fair–rich metals (5–16 pockets); lean volatiles (1–4 pockets); barren–lean \
             strategic elements (0–4 pockets)"
        );
        let barren = [FieldReading::Exact(Word::Barren); 4];
        assert_eq!(
            field_row_text("A-SE", &barren, Footing::Rock),
            "A-SE barren"
        );
        assert_eq!(
            field_row_text("A-SE", &[FieldReading::NoReturn; 4], Footing::Ice),
            "A-SE no return"
        );
        assert_eq!(
            pocket_row_text(
                "C117",
                &[Word::Fair, Word::Lean, Word::Barren, Word::Barren]
            ),
            "C117 fair metals, lean volatiles"
        );
        assert_eq!(pocket_row_text("C117", &[Word::Barren; 4]), "C117 barren");
        for text in [
            field_row_text("C-NW", &rows, Footing::Regolith),
            pocket_row_text("C117", &[Word::Rich; 4]),
        ] {
            for code in ["P0", "P1", "P2", "P3"] {
                assert!(!text.contains(code), "{text}");
            }
        }
    }

    // ---- coverage over a hand-built composite (the SM.4a fixer round's finding T3) ---------

    /// A 256 × 256 site composite of 10 m cells, every cell `class` at datum (`trace.rs`'s
    /// hand-built surface, at the site's grid).
    fn composite(class: TerrainClass) -> SurfaceComposite {
        use mush_atlas::cells::{Cell, Cells};
        SurfaceComposite {
            surface: SurfaceRef::Ground,
            grid: mush_atlas::raster::Grid {
                origin: mush_atlas::PointCm { x: 0, y: 0 },
                resolution_cm: 1_000,
                width: 256,
                height: 256,
            },
            cells: Cells::filled(
                256,
                256,
                Cell {
                    class,
                    elev_cm: 0,
                    cover: 0,
                    room: 0,
                },
                65_536,
            )
            .unwrap_or_else(|e| panic!("{e}")),
            rooms: Vec::new(),
            features: Vec::new(),
        }
    }

    /// Set one site cell's class and elevation.
    fn put(surface: &mut SurfaceComposite, (x, y): (u16, u16), class: TerrainClass, elev_cm: i16) {
        assert!(surface.cells.set(
            x,
            y,
            mush_atlas::cells::Cell {
                class,
                elev_cm,
                cover: 0,
                room: 0,
            },
        ));
    }

    /// The shipped Mk1 scanner (mast 2.5 m, scan reach 48 cells).
    fn mk1() -> ScannerDef {
        *crate::definitions::tools::tools_table()
            .unwrap_or_else(|e| panic!("{e}"))
            .values()
            .find_map(crate::definitions::tools::ToolDef::scanner)
            .unwrap_or_else(|| panic!("a scanner row"))
    }

    /// **The sight rule** (D5), independent of the world: from the middle of field (4, 4) on
    /// flat regolith the mast sees all nine fields, its own included, listed north-west first;
    /// a 10 m ridge across the line to field (5, 4)'s centre cell (176, 144) — the cells
    /// x = 160, y = 136..=152, which no line to another neighbour's centre crosses — hides
    /// exactly that field; and a ridge across the stand's own field never hides it.
    #[test]
    fn sm4a_the_mast_sees_a_field_middle_over_the_composite() {
        let scanner = mk1();
        assert_eq!(scanner.mast_cm, 250);
        let mut flat = composite(TerrainClass::Regolith);
        let stand = (144, 144);
        let nine = vec![43, 44, 45, 35, 36, 37, 27, 28, 29];
        assert_eq!(seen_fields(&flat, stand, &scanner), (nine, Vec::new()));
        for y in 136..=152 {
            put(&mut flat, (160, y), TerrainClass::Regolith, 1_000);
        }
        let (seen, hidden) = seen_fields(&flat, stand, &scanner);
        assert_eq!(hidden, vec![37], "only field (5, 4) is behind the ridge");
        assert_eq!(seen, vec![43, 44, 45, 35, 36, 27, 28, 29]);
        assert_eq!(field_index(5, 4), Some(37));
        // The stand's own field is seen whatever stands in it.
        let mut walled = composite(TerrainClass::Regolith);
        for x in 128..160 {
            put(&mut walled, (x, 150), TerrainClass::Regolith, 5_000);
        }
        let (seen, _) = seen_fields(&walled, stand, &scanner);
        assert!(seen.contains(&36), "{seen:?}");
        // A rise lower than the mast hides nothing: the line runs at mast height end to end.
        let mut low = composite(TerrainClass::Regolith);
        for y in 136..=152 {
            put(&mut low, (160, y), TerrainClass::Regolith, 200);
        }
        assert!(seen_fields(&low, stand, &scanner).1.is_empty());
    }

    /// **The better-footing rule** (D20), independent of the world: from ice with regolith two
    /// rings west and rock three rings east, the nearest sharper footing is the regolith, 20 m
    /// W; two sharper cells in one ring resolve north-west first; from regolith only rock
    /// betters it (ice is duller); from rock nothing does; outside the radius nothing is found.
    #[test]
    fn sm4a_better_footing_reads_rings_from_the_north_west() {
        let mut ground = composite(TerrainClass::Ice);
        put(&mut ground, (98, 100), TerrainClass::Regolith, 0);
        put(&mut ground, (103, 100), TerrainClass::Rock, 0);
        assert_eq!(
            better_footing(&ground, (100, 100), Footing::Ice, 48),
            Some((Footing::Regolith, 20, Octant::W))
        );
        assert_eq!(
            better_footing(&ground, (100, 100), Footing::Regolith, 48),
            Some((Footing::Rock, 30, Octant::E))
        );
        assert_eq!(better_footing(&ground, (100, 100), Footing::Rock, 48), None);
        assert_eq!(better_footing(&ground, (100, 100), Footing::Ice, 1), None);
        // Two in one ring: the north-west one (98, 102) is read before the south-east (102, 98).
        let mut tie = composite(TerrainClass::Ice);
        put(&mut tie, (102, 98), TerrainClass::Regolith, 0);
        put(&mut tie, (98, 102), TerrainClass::Regolith, 0);
        assert_eq!(
            better_footing(&tie, (100, 100), Footing::Ice, 48),
            Some((Footing::Regolith, 20, Octant::NW))
        );
        // Within a ring, a row from the north is read west to east and the first sharper cell
        // wins, even before a sharper one further east in the same row.
        let mut row = composite(TerrainClass::Ice);
        put(&mut row, (101, 102), TerrainClass::Rock, 0);
        put(&mut row, (100, 102), TerrainClass::Regolith, 0);
        assert_eq!(
            better_footing(&row, (100, 100), Footing::Ice, 48),
            Some((Footing::Regolith, 20, Octant::N))
        );
    }

    /// **The footing at a cell**: rock, regolith and ice give their footing; paved gives none
    /// and names itself; a cell off the site is `Void`.
    #[test]
    fn sm4a_the_footing_at_a_cell() {
        let mut ground = composite(TerrainClass::Paved);
        put(&mut ground, (10, 20), TerrainClass::Rock, 0);
        put(&mut ground, (11, 20), TerrainClass::Regolith, 0);
        put(&mut ground, (12, 20), TerrainClass::Ice, 0);
        assert_eq!(footing_at(&ground, (10, 20)), Ok(Footing::Rock));
        assert_eq!(footing_at(&ground, (11, 20)), Ok(Footing::Regolith));
        assert_eq!(footing_at(&ground, (12, 20)), Ok(Footing::Ice));
        assert_eq!(footing_at(&ground, (13, 20)), Err(TerrainClass::Paved));
        for off in [(-1, 0), (0, -1), (256, 0), (0, 256)] {
            assert_eq!(footing_at(&ground, off), Err(TerrainClass::Void), "{off:?}");
        }
    }

    /// A ring's `(instrument, at, via)` triples.
    fn ring_of(ring: &[ChartProvenance]) -> Vec<(Instrument, i64, Via)> {
        ring.iter()
            .map(|entry| match entry {
                ChartProvenance::Local {
                    instrument,
                    at,
                    via,
                } => (*instrument, *at, *via),
            })
            .collect()
    }

    /// **A plan's stamp never touches `first_hand_at`** (doc 20 cut SM.11; lead ruling 4, the
    /// reviewer's F7): a field scanned by a plan, then by hand, then four more times by the plan
    /// keeps the hand entry and the three newest plan entries; `first_local_at` is the first plan
    /// stamp and `first_hand_at` the hand one. A sounding and a core alike.
    #[test]
    fn sm11_a_plan_stamp_never_touches_first_hand_at() {
        let mut r = row();
        write_scan(&mut r, [128, 124], &[27], Footing::Regolith, 100, Via::Plan);
        write_scan(&mut r, [128, 124], &[27], Footing::Regolith, 200, Via::Hand);
        for at in [300, 400, 500, 600] {
            write_scan(&mut r, [128, 124], &[27], Footing::Regolith, at, Via::Plan);
        }
        let field = r.fields.get(&27).unwrap_or_else(|| panic!("the field row"));
        assert_eq!(
            ring_of(&field.provenance),
            vec![
                (Instrument::Scan, 200, Via::Hand),
                (Instrument::Scan, 400, Via::Plan),
                (Instrument::Scan, 500, Via::Plan),
                (Instrument::Scan, 600, Via::Plan),
            ]
        );
        assert_eq!(
            (field.first_local_at, field.first_hand_at),
            (100, Some(200))
        );
        assert_eq!(r.check(), Ok(()));
        // A sounding and a core: the plan's alone leave `first_hand_at` absent.
        let pockets = address::pockets_in_reach((112, 112), 8);
        let pocket = pockets.first().copied().unwrap_or((28, 28));
        let mut b = row();
        write_sound(&mut b, [112, 112], &pockets, false, 100, Via::Plan);
        write_core(&mut b, pocket, 150, Via::Plan);
        let letter = pocket_in_block(pocket.0, pocket.1)
            .map(|(letter, _)| letter.to_string())
            .unwrap_or_default();
        let block = b
            .blocks
            .get(letter.as_str())
            .unwrap_or_else(|| panic!("the block"));
        assert_eq!((block.first_local_at, block.first_hand_at), (100, None));
        write_core(&mut b, pocket, 175, Via::Hand);
        let block = b
            .blocks
            .get(letter.as_str())
            .unwrap_or_else(|| panic!("the block"));
        assert_eq!(block.first_hand_at, Some(175));
        assert_eq!(b.check(), Ok(()));
    }

    /// **The trim evicts a plan's stamp before a hand stamp** (doc 20 cut SM.11; the reviewer's
    /// F7): four hand stamps then a later plan stamp keep the four hand stamps; three hand and one
    /// plan then a hand drop the plan entry; five hand stamps and no plan drop the oldest, as
    /// SM.4a built it.
    #[test]
    fn sm11_the_trim_evicts_a_plan_stamp_before_a_hand_one() {
        let mut r = row();
        for at in [10, 20, 30, 40] {
            write_scan(&mut r, [0, 0], &[0], Footing::Rock, at, Via::Hand);
        }
        write_scan(&mut r, [0, 0], &[0], Footing::Rock, 50, Via::Plan);
        let ats = |r: &ChartRecord| -> Vec<(i64, Via)> {
            r.fields
                .get(&0)
                .map(|field| {
                    ring_of(&field.provenance)
                        .into_iter()
                        .map(|(_, at, via)| (at, via))
                        .collect()
                })
                .unwrap_or_default()
        };
        assert_eq!(
            ats(&r),
            vec![
                (10, Via::Hand),
                (20, Via::Hand),
                (30, Via::Hand),
                (40, Via::Hand)
            ]
        );
        let mut mixed = row();
        for (at, via) in [
            (10, Via::Hand),
            (20, Via::Plan),
            (30, Via::Hand),
            (40, Via::Hand),
        ] {
            write_scan(&mut mixed, [0, 0], &[0], Footing::Rock, at, via);
        }
        write_scan(&mut mixed, [0, 0], &[0], Footing::Rock, 50, Via::Hand);
        assert_eq!(
            ats(&mixed),
            vec![
                (10, Via::Hand),
                (30, Via::Hand),
                (40, Via::Hand),
                (50, Via::Hand)
            ]
        );
        let mut hand = row();
        for at in [10, 20, 30, 40, 50] {
            write_scan(&mut hand, [0, 0], &[0], Footing::Rock, at, Via::Hand);
        }
        assert_eq!(
            ats(&hand),
            vec![
                (20, Via::Hand),
                (30, Via::Hand),
                (40, Via::Hand),
                (50, Via::Hand)
            ]
        );
    }
}
