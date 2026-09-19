//! **What a unit sees with** (16b cut C.2, C1; doc 16 §3.6 "The pass", §4
//! "Content packs").
//!
//! The row cut S's perception pass reads: the candidate radius in cells, the
//! attenuation a `Partial` line of sight may carry and still count, how long a
//! contact must be held before it classifies, whether wrecks are candidates at
//! all, **the signature table** doc 16 §3.6 step 3 scales the range by, **the
//! comms reach** step 5 shares along — and the **two hooks**, `emission` and
//! `jamming`, which doc 16 §3.6 requires to be *present* and *null* until the
//! extensions of §9 read them, so that each is a value change rather than a
//! schema change.
//!
//! "Present and null" is load-bearing and is the reason both are `Option`s
//! without a serde default: a row that simply omits one is refused by the
//! decoder for a missing field, so a sight cannot quietly stop writing the two
//! `null`s its digest was taken over. Both cuts have now landed, and each hook is
//! refused by [`SensorDef::check`] — naming the hook — where it holds something
//! its own cut cannot read.
//!
//! **16b cut X.4 is that cut for `jamming`** (doc 16 §9 item 4: *the `jamming`
//! field becomes non-null*). It is now a typed [`JammingDef`] rather than an
//! opaque `Value`, and a row that fills it is a **jammer**: the pod of doc 16 §2
//! W1, which is why sensor ranges in this game are short.
//!
//! **16b cut X.5 is that cut for `emission`** (doc 16 §9 item 5: *a component
//! that subtracts from that term*). A row that fills it carries an [`EccmDef`]
//! and is a **filter**: what an observer standing in a bubble buys back of what
//! the bubble took. The hook keeps its name — doc 16 §3.6's own — and keeps its
//! refusal: a value of any other shape still refuses naming `emission`, which is
//! what [`EmissionHook::Unread`] is for.

use mush_atlas::TerrainClass;
use serde::{Deserialize, Serialize};

use crate::definitions::DefinitionError;

/// The widest candidate radius one sensor may declare, in cells. The arena is
/// 256 cells a side (D-16-5), so this is four arenas.
pub const MAX_RANGE_CELLS: u16 = 1_024;

/// The longest a classification may take: ten minutes is past any engagement.
pub const MAX_CLASSIFY_AFTER_MS: u32 = 600_000;

/// The widest comms reach one sensor may declare, in cells (16b §9 pins 30).
/// Bounded by the arena's own side, because a share beyond the window is a share
/// to nobody.
pub const MAX_SHARE_RANGE_CELLS: u16 = 256;

/// A signature factor is per mille, and `1000` — the factor of a target that
/// hides nothing — is its ceiling. A row above it would *lengthen* a sensor's
/// range for a particular target, which is the `max_signature_factor` knob's job
/// and not content's (doc 16 §3.6 steps 1 and 3).
pub const MAX_SIGNATURE_PERMILLE: u32 = 1_000;

/// The widest bubble one jammer may declare, in cells (16b cut X.4). Bounded by
/// the arena's own side for the reason [`MAX_SHARE_RANGE_CELLS`] is bounded by
/// it: a bubble wider than the window jams nobody it was not already jamming.
pub const MAX_JAM_RADIUS_CELLS: u16 = 256;

/// The most one jammer may take out of an observer's attenuation budget: the
/// whole of it, on the atlas's own 0-to-100 opacity scale (doc 16 §3.6 step 2).
///
/// A cost of `100` is a pod that closes the budget outright — an observer inside
/// that bubble accepts nothing but a `Clear` line — and is the ceiling rather
/// than the intention; the shipped pod costs 25, which is one belt of light
/// woods.
pub const MAX_ATTENUATION_COST: u8 = 100;

/// **One row of the signature table** (16b cut S.1; doc 16 §3.6 step 3: *a
/// stopped tank in heavy woods has a lower factor than one moving on a road*).
///
/// The factor scales the sensor's range for a target standing on `class`, per
/// mille: `1000` is a target that hides nothing, `500` a target detected at half
/// the sensor's reach. `moving` is read when the target's speed is non-zero and
/// `stopped` otherwise, which is the whole of the *posture* doc 16 §3.6 names —
/// there is no third state, and the arena reads the speed it already stores
/// rather than a derived one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureRow {
    /// The ground the target is standing on.
    pub class: TerrainClass,
    /// The factor while the target is under way, per mille.
    pub moving_permille: u32,
    /// The factor while the target is stopped, per mille.
    pub stopped_permille: u32,
}

/// **The signature term a thermal sight adds** (16b cut X.3; doc 16 §9 item 3:
/// *a sensor row with its own range and signature factors — an idling tank is
/// cold — one new signature term*; 16b §8 cut X: *keyed on power and speed*).
///
/// One more factor on doc 16 §3.6 step 3, multiplied into the one
/// [`SignatureRow`] reads off the ground, and keyed on the **target's** state
/// rather than on the ground it stands on:
///
/// | the target | the factor |
/// |---|---|
/// | under way (`speed_cms != 0`) and online | `1000` — the plume a running engine makes |
/// | online and stopped | [`ThermalSignature::powered_permille`] |
/// | not online | [`ThermalSignature::cold_permille`] |
///
/// Both default to [`MAX_SIGNATURE_PERMILLE`], so a sight that authors one of
/// them and not the other reads the other as *no discount at all*, and a sight
/// that authors neither is a sight with no thermal channel.
///
/// # Why it is authored on the mount and not here
///
/// It belongs, by the doc's reading, on [`SensorDef`] beside the signature table
/// — and cut X's rule is that an extension *touches no existing test*.
/// `SensorDef` and [`SignatureRow`] are both built as exhaustive struct literals
/// by `crates/tacsim/tests/definitions.rs`, and the arena's copies of them
/// ([`crate::arena::perception::SensorProfile`] and `SignatureFactor`) by seven
/// more test files, so a new field on any of the four is a diff in a test this
/// cut may not write. So the term is authored on the **sensor component that
/// mounts the sight** (`crate::definitions::unit::ComponentRole::Sensor`), which
/// is where doc 16 §3.2 already puts `sensor_height_cm` — how the sight is
/// fitted to *this* hull — and it reaches the arena the way 16b cut X.1's
/// `heat_per_shot` does: as integer cells on the component, read off the
/// `TacticalUnit` that crosses the custody seam anyway. Moving it onto the row
/// is a later cut that may open those four structs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThermalSignature {
    /// The factor a target that is **powered and stopped** presents — an idling
    /// tank, warm but not running.
    #[serde(default = "full_permille", skip_serializing_if = "is_full_permille")]
    pub powered_permille: u32,
    /// The factor a target whose power is **not online** presents — a tank that
    /// has been shut down, and cold metal to a thermal sight.
    #[serde(default = "full_permille", skip_serializing_if = "is_full_permille")]
    pub cold_permille: u32,
}

/// The default of both [`ThermalSignature`] columns, for `serde`.
const fn full_permille() -> u32 {
    MAX_SIGNATURE_PERMILLE
}

/// Whether an authored factor is the default, so an unauthored one is not
/// written back out.
fn is_full_permille(value: &u32) -> bool {
    *value == MAX_SIGNATURE_PERMILLE
}

impl ThermalSignature {
    /// Re-run both bounds.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and the column.
    pub fn check(&self, entry: &str, owner: &str) -> Result<(), DefinitionError> {
        for (column, permille) in [
            ("powered_permille", self.powered_permille),
            ("cold_permille", self.cold_permille),
        ] {
            if permille > MAX_SIGNATURE_PERMILLE {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "`{owner}`'s thermal `{column}` is {permille}, past \
                         {MAX_SIGNATURE_PERMILLE}: a factor lengthens no sensor's range, it \
                         only shortens it"
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// **What a jamming pod does to an observer standing inside its bubble** (16b cut
/// X.4; doc 16 §9 item 4: *a component that adds to the observer's
/// `max_attenuation` budget and lowers signature factors within a radius*; doc 16
/// §2 W1).
///
/// The `jamming` hook doc 16 §3.6 left null, filled. Three integers and no
/// geometry:
///
/// | field | what it does |
/// |---|---|
/// | [`JammingDef::radius_cells`] | how far the bubble reaches, in cells |
/// | [`JammingDef::attenuation_cost`] | what it takes out of the observer's step 2 budget |
/// | [`JammingDef::signature_permille`] | what it leaves of every target's step 3 factor |
///
/// # Whose position the radius is measured from
///
/// **The observer's** (doc 16 §9 item 4's own words: *adds to an observer's
/// `max_attenuation` budget … within a radius*). A pod degrades the band around
/// itself, so what it costs is paid by whoever is trying to *look* from inside
/// it; a target standing in the bubble is seen exactly as before by an observer
/// standing outside it. That is the reading the arena implements
/// ([`crate::arena::perception::Jamming`]) and the one 16b cut X.4's acceptance
/// asserts both ways round.
///
/// # Why it is authored here and not on the mount
///
/// Unlike 16b cut X.3's [`ThermalSignature`] — which had to move to the sensor
/// *mount* because [`SensorDef`] is built as an exhaustive struct literal by tests
/// this cut may not edit — `jamming` is a field [`SensorDef`] **already has**. It
/// was `Option<serde_json::Value>` and every literal in the tree writes `None`,
/// so giving it a type moves no test and moves no shipped
/// [`mush_core::mechanics::DefinitionRef`]: a `None` still serialises as
/// `"jamming": null`, which is the byte the shipped sights' digests were taken
/// over.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JammingDef {
    /// How far the bubble reaches from the pod, in cells. There is no inert
    /// jammer: a row that wants none authors no `jamming` at all, so zero is
    /// refused.
    pub radius_cells: u16,
    /// What an observer inside the bubble loses from its own sensor's
    /// `max_attenuation` (doc 16 §3.6 step 2), on the atlas's 0-to-100 scale.
    /// Several bubbles **sum**, and the budget floors at zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attenuation_cost: u8,
    /// What it leaves of every target's signature factor (doc 16 §3.6 step 3),
    /// per mille: `150` is a sensor pass that reaches 15% as far as it did.
    /// Several bubbles **multiply**. [`MAX_SIGNATURE_PERMILLE`] is *no discount*
    /// and is the default, so a pod may author the budget cost alone.
    #[serde(default = "full_permille", skip_serializing_if = "is_full_permille")]
    pub signature_permille: u32,
}

/// Whether an authored cost is the default, so an unauthored one is not written
/// back out.
fn is_zero(value: &u8) -> bool {
    *value == 0
}

impl JammingDef {
    /// Re-run every bound.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and the field.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        if self.radius_cells == 0 || self.radius_cells > MAX_JAM_RADIUS_CELLS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "`jamming.radius_cells` is 1 to {MAX_JAM_RADIUS_CELLS}; {} is not (a row \
                     that jams nothing authors no `jamming` at all)",
                    self.radius_cells
                ),
            ));
        }
        if self.attenuation_cost > MAX_ATTENUATION_COST {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "`jamming.attenuation_cost` is 0 to {MAX_ATTENUATION_COST} (the atlas's own \
                     scale); {} is not",
                    self.attenuation_cost
                ),
            ));
        }
        if self.signature_permille > MAX_SIGNATURE_PERMILLE {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "`jamming.signature_permille` is {}, past {MAX_SIGNATURE_PERMILLE}: a \
                     jammer shortens a sensor's reach and never lengthens it",
                    self.signature_permille
                ),
            ));
        }
        Ok(())
    }
}

/// **What a counter-jamming filter buys back of what a bubble took** (16b cut
/// X.5; doc 16 §9 item 5: *a component that subtracts from that term*).
///
/// The `emission` hook doc 16 §3.6 left null, filled. Two integers, each the
/// inverse of one of [`JammingDef`]'s:
///
/// | field | what it undoes |
/// |---|---|
/// | [`EccmDef::attenuation_recovery`] | [`JammingDef::attenuation_cost`], subtracted from the summed cost |
/// | [`EccmDef::signature_recovery_permille`] | [`JammingDef::signature_permille`], added back to the surviving fraction |
///
/// # Whose component it is
///
/// **The observer's.** A pod degrades the band around itself and the cost is paid
/// by whoever looks from inside it (see [`JammingDef`]); a filter is carried by
/// the looker and recovers part of that cost for that looker alone. It is not a
/// counter-*bubble*: it does nothing for a teammate standing beside it, and there
/// is no radius on it at all.
///
/// # It can never see better than no bubble
///
/// Both recoveries are bounded by the term they undo at apply time, not by
/// content: the cost floors at zero and the fraction ceilings at
/// [`MAX_SIGNATURE_PERMILLE`] ([`crate::arena::perception::Jamming::recovered`]).
/// So an observer with a filter and no bubble on it is *bit-identical* to one
/// without, and a fully rated filter is bit-identical to an unjammed pass — which
/// is the whole of what "subtracts from that term" is allowed to mean.
///
/// # Why it is authored under `emission` and not under a key of its own
///
/// Because doc 16 §3.6 names two hooks on this row, `emission` and `jamming`, and
/// §9 item 5 is the cut that reads the first of them — this module's head has said
/// so since cut X.4. A key of its own would be a **new field** on [`SensorDef`],
/// which cut X cannot have for the reason [`ThermalSignature`] gives: the struct is
/// built as an exhaustive literal by tests this cut may not edit. `emission` is a
/// field the row already has, every literal in the tree writes `None` for it, and a
/// `None` still serialises as `null` — so no shipped sight's bytes, and no persisted
/// unit's reference, moves.
///
/// # Both columns default to zero
///
/// Unlike [`JammingDef::signature_permille`], whose neutral value is the ceiling,
/// a filter's neutral value is *nothing recovered* — so a row may author either
/// column alone, and a row that authors neither recovers nothing and is refused by
/// [`EccmDef::check`] the way a `jamming` block with a zero radius is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EccmDef {
    /// What an observer carrying this filter takes back off the summed
    /// `attenuation_cost` of the bubbles it stands in (doc 16 §3.6 step 2), on the
    /// atlas's 0-to-100 scale. The recovered cost floors at zero, so the budget is
    /// never *better* than the sensor's own.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attenuation_recovery: u8,
    /// What it adds back to the surviving fraction of every target's signature
    /// factor (doc 16 §3.6 step 3), per mille. **Additive on whatever the bubbles
    /// left**, and capped at [`MAX_SIGNATURE_PERMILLE`]: against one pod that left
    /// 150 a recovery of 75 is 225, and against two that left 22 it is 97, which is
    /// what a filter is for — it is worth most where the noise is worst.
    #[serde(default, skip_serializing_if = "is_zero_permille")]
    pub signature_recovery_permille: u32,
}

/// Whether an authored recovery is the default, so an unauthored one is not
/// written back out.
fn is_zero_permille(value: &u32) -> bool {
    *value == 0
}

impl EccmDef {
    /// Re-run every bound.
    ///
    /// `owner` is the hook the block was authored under, so the message names the
    /// key an author would go and look for — which for a row is `emission`.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry, the hook and the column.
    pub fn check(&self, entry: &str, owner: &str) -> Result<(), DefinitionError> {
        if self.attenuation_recovery == 0 && self.signature_recovery_permille == 0 {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "`{owner}` recovers nothing: a filter authors an \
                     `attenuation_recovery`, a `signature_recovery_permille`, or both (a row \
                     that counters nothing authors no `{owner}` block at all)"
                ),
            ));
        }
        if self.attenuation_recovery > MAX_ATTENUATION_COST {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "`{owner}.attenuation_recovery` is 0 to {MAX_ATTENUATION_COST} (the \
                     atlas's own scale); {} is not",
                    self.attenuation_recovery
                ),
            ));
        }
        if self.signature_recovery_permille > MAX_SIGNATURE_PERMILLE {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "`{owner}.signature_recovery_permille` is {}, past \
                     {MAX_SIGNATURE_PERMILLE}: a filter gives back what a jammer took and \
                     never more",
                    self.signature_recovery_permille
                ),
            ));
        }
        Ok(())
    }
}

/// **What a row's `emission` hook holds** (16b cut X.5), which is doc 16 §9 item
/// 5's block and nothing else.
///
/// # Why the hook is not simply an `Option<EccmDef>`
///
/// Because of *how* a value of the wrong shape has to be refused. Doc 16 §3.6's
/// "present and null" rule is enforced by [`SensorDef::check`] naming the hook, and
/// `crates/tacsim/tests/definitions.rs` pins that refusal by putting
/// `{"band": "radar"}` in a shipped sight's `emission` and asserting the message
/// names **`emission`** — a test cut X may not edit. A bare `Option<EccmDef>` moves
/// that refusal out of `check` and into `serde`, whose message for an adjacently
/// tagged entry names the offending *inner* key and no path to it, so the pinned
/// assertion would fail on a message that no longer says which hook was wrong.
///
/// The untagged pair keeps the decode total and the refusal where the test (and an
/// author reading it) expects it: anything that is not an [`EccmDef`] lands in
/// [`EmissionHook::Unread`] and `check` refuses it by name.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmissionHook {
    /// **The filter block** (16b cut X.5): `attenuation_recovery` and
    /// `signature_recovery_permille`, each optional and each defaulting to zero.
    Eccm(EccmDef),
    /// Anything else the hook was filled with, kept only so that
    /// [`SensorDef::check`] can refuse it naming `emission` rather than leaving
    /// `serde` to refuse it naming one of its keys. Nothing reads it.
    Unread(serde_json::Value),
}

/// One sensor (doc 16 §3.6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorDef {
    /// The human-facing name.
    pub label: String,
    /// The candidate radius at signature factor 1, in cells (doc 16 §3.6 step
    /// 1; 16b §9 pins 60 — 600 m at `cell_cm` 1,000).
    pub max_range_cells: u16,
    /// The most `Opacity::Partial` attenuation a line of sight may carry and
    /// still be a detection (doc 16 §3.6 step 2). The atlas prices light woods
    /// at 25 and heavy woods at 60.
    pub max_attenuation: u8,
    /// How long a contact must be held, without a gap longer than
    /// `sensors.pass_ms`, before it classifies (doc 16 §3.6 step 4).
    pub classify_after_ms: u32,
    /// Whether a wreck is a candidate (doc 16 §3.6 step 1).
    pub sees_wrecks: bool,
    /// **The signature table** (doc 16 §3.6 step 3), by the ground the target
    /// stands on. A class the table does not name is `1000`/`1000` — a target
    /// that hides nothing — so the table is the list of grounds that *conceal*
    /// and never a list every class has to appear in.
    pub signature: Vec<SignatureRow>,
    /// **The comms reach** a classified contact is shared along (doc 16 §3.6
    /// step 5, W6), in cells; 16b §9 pins 30. Zero shares nothing.
    pub share_range_cells: u16,
    /// **What this row counters** (doc 16 §3.6 last line, §9 item 5), or `None` for
    /// every row that is a sight or a pod rather than a filter — which is every row
    /// the game ships, and the reason a shipped sight's payload is still the bytes
    /// it was: a `None` serialises as `"emission": null`, the byte the shipped
    /// sights' digests were taken over (16b cut X.5).
    pub emission: Option<EmissionHook>,
    /// **What this row jams** (doc 16 §3.6 last line, §9 item 4), or `None` for
    /// every row that is a sight rather than a pod — which is every row the game
    /// ships, and the reason a shipped sight's payload is the bytes it was (16b
    /// cut X.4).
    pub jamming: Option<JammingDef>,
    /// Why these numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl SensorDef {
    /// **The filter block this row carries** (16b cut X.5), or `None` for a row
    /// whose `emission` hook is null — every row the game ships.
    ///
    /// A hook filled with something that is not a filter answers `None` here and is
    /// refused by [`SensorDef::check`], so nothing downstream ever sees it: the
    /// catalog refuses the pack before a mount can read this.
    #[must_use]
    pub fn eccm(&self) -> Option<&EccmDef> {
        match &self.emission {
            Some(EmissionHook::Eccm(eccm)) => Some(eccm),
            Some(EmissionHook::Unread(_)) | None => None,
        }
    }

    /// Re-run every bound: (16b cut X.5) `emission`'s two numbers where a row
    /// carries a filter and its refusal where it carries anything else, and (16b
    /// cut X.4) `jamming`'s three numbers where a row carries them.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and the field.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        if self.max_range_cells == 0 || self.max_range_cells > MAX_RANGE_CELLS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "max_range_cells is 1 to {MAX_RANGE_CELLS}; {} is not",
                    self.max_range_cells
                ),
            ));
        }
        if self.max_attenuation > 100 {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "max_attenuation is 0 to 100 (the atlas's own scale); {} is not",
                    self.max_attenuation
                ),
            ));
        }
        if self.classify_after_ms > MAX_CLASSIFY_AFTER_MS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "classify_after_ms is at most {MAX_CLASSIFY_AFTER_MS}; {} is not",
                    self.classify_after_ms
                ),
            ));
        }
        if self.share_range_cells > MAX_SHARE_RANGE_CELLS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "share_range_cells is 0 to {MAX_SHARE_RANGE_CELLS}; {} is not",
                    self.share_range_cells
                ),
            ));
        }
        let mut seen: Vec<TerrainClass> = Vec::with_capacity(self.signature.len());
        for row in &self.signature {
            if seen.contains(&row.class) {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "the signature table names `{}` twice; one ground is one row",
                        row.class.label()
                    ),
                ));
            }
            seen.push(row.class);
            for (posture, permille) in [
                ("moving_permille", row.moving_permille),
                ("stopped_permille", row.stopped_permille),
            ] {
                if permille > MAX_SIGNATURE_PERMILLE {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "the signature table's `{}` row has {posture} {permille}, past \
                             {MAX_SIGNATURE_PERMILLE}: a factor lengthens no sensor's range, it \
                             only shortens it",
                            row.class.label()
                        ),
                    ));
                }
            }
        }
        // **16b cut X.5**: and `emission` is read (doc 16 §9 item 5), so a filter
        // block is bounded where every other sensor bound is re-run and anything
        // else is still refused naming the hook — which is the refusal
        // `tests/definitions.rs` pins.
        match &self.emission {
            None => {}
            Some(EmissionHook::Eccm(eccm)) => eccm.check(entry, "emission")?,
            Some(EmissionHook::Unread(_)) => {
                return Err(DefinitionError::value(
                    entry,
                    "`emission` is doc 16 §9 item 5's hook and holds a filter block — \
                     `attenuation_recovery`, `signature_recovery_permille`, or both — and \
                     nothing else; a value of another shape configures nothing",
                ));
            }
        }
        // **16b cut X.4**: and `jamming` is read, so it is bounded rather than
        // refused (doc 16 §9 item 4).
        if let Some(jamming) = &self.jamming {
            jamming.check(entry)?;
        }
        Ok(())
    }
}
