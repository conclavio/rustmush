//! **What a round does when it arrives** (16b cut W.2, W3; doc 16 §3.5 "Scheduled
//! impact" and "Damage", §3.3 "A wreck" and "Facet attribution", §3.8 "Team
//! elimination").
//!
//! One entry point — [`Arena::resolve_impact`], the body of the
//! [`DueKind::Impact`] cut W.1 scheduled —
//! and everything below it is the shape of one hit: re-validate, choose a section,
//! spend the armour, overflow into the internals, and apply the consequence if the
//! section is gone.
//!
//! # Re-validation is four questions and their four reasons
//!
//! Doc 16 §3.5: *at impact the arena re-validates — the target exists and is not a
//! wreck, its position is within one cell of the prediction (a target that changed
//! facet band re-resolves with the same `hit` draw), the line is clear — and a
//! failed validation is a `miss` with its reason.* The four reasons are
//! [`TARGET_GONE`], [`WRECK`], [`MOVED`] and [`BLOCKED`], and [`COVER`] is the
//! fifth: doc 16 §3.5 accepts a shot at a target behind full cover, costs it its
//! round and its cycle, and resolves it as a miss *with the blocker as its reason*
//! — which is a thing only the impact can say, because acceptance is forbidden to
//! refuse for it (*there is no free probe of line of sight*).
//! [`DISPERSION`] is the sixth and the only one that is not a validation: it is the
//! round that was aimed correctly and went wide, which is the draw's answer.
//!
//! All six are journalled as the `reason` field of
//! [`EventKind::Miss`], and the set is the
//! whole vocabulary — a seventh is a reviewed diff here.
//!
//! # One plan per impact, and why that matters
//!
//! Doc 16 §3.5: *all of it is one [`UnitMutationPlan`] per impact through
//! [`apply`](mush_tactics::apply), journalled, with one `hit` ledger event carrying
//! the plan's cause.* Refuse, never clamp (M32): the plan is built to be in bounds
//! — the armour spend is floored at the cell's own minimum and the overflow is what
//! is left over, so no op can ever ask for a value outside a cell's range — and if
//! M32 refuses it anyway **nothing** moves and the shot is journalled as a miss
//! with the refusal as its reason. A half-applied hit is not a state this code can
//! produce, because `apply` folds the whole sequence on a scratch clone before it
//! returns anything.
//!
//! The consequences split in two, and the split is not arbitrary. `destroy_unit` is
//! an **M32 fact** — doc 16 §3.3 defines a wreck as `hull` condition `wreck` plus
//! the revocation of three capabilities — so it is ops in the same plan. `slow`,
//! `no_turn_toward`, `disable_motion` and `remove_weapons` are **arena facts**:
//! there is no mutation op for "this tank is now half as fast", and inventing one
//! would be teaching M32 a formula (the thing its own module head refuses). They
//! land on [`MotionLimits`](crate::arena::state::MotionLimits) and on the unit's
//! weapon table, which is where A.2's motion and W.1's acceptance already read
//! them.
//!
//! # A wreck stops being an observer, and stops being observed
//!
//! Doc 16 §3.3: *a wreck is an obstacle and a contact, never a target.* So
//! [`Arena::destroy_unit`] clears the wreck's own contact store (it holds nothing:
//! S.1's residual, closed here), drops its scheduled sensor work off the heap, and
//! leaves it in the units vector at the pose it died at — where the index keeps it
//! as an obstacle and every other platform's contact on it expires through the
//! ordinary time-to-live, which is what clears their locks and tells their pilots.

use mush_atlas::Cover;
use mush_atlas::trace::{Sight, trace_cells};
use mush_core::ObjId;
use mush_tactics::{
    CapabilityKey, CellAddr, ConditionId, SectionId, UnitMutationOp, UnitMutationPlan,
};
use smol_str::SmolStr;

use crate::arena::ledger::EventKind;
use crate::arena::perception::Notice;
use crate::arena::resolution::{Consequence, Resolution};
use crate::arena::rng::{DrawLabel, draw};
use crate::arena::state::{
    Arena, Controller, DueKind, EndReason, MatchEnd, Power, Team, UnitState,
};
use crate::definitions::unit::{ARMOR_CELL, INTERNALS_CELL};

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

/// The target left the match between the shot and its arrival.
pub const TARGET_GONE: &str = "target_gone";

/// The target was already a wreck when the round arrived (doc 16 §3.3: *never a
/// target*).
pub const WRECK: &str = "wreck";

/// The target was more than one cell from the predicted point.
pub const MOVED: &str = "moved";

/// Something stood in the line when the round arrived.
pub const BLOCKED: &str = "blocked";

/// The target was behind full cover — the shot doc 16 §3.5 accepts anyway.
pub const COVER: &str = "cover";

/// The round was aimed at the target and went wide: the draw's answer, and the
/// only reason that is not a failed validation.
pub const DISPERSION: &str = "dispersion";

/// [`apply`](mush_tactics::apply) refused the plan. It cannot happen for a plan
/// this module builds — every value is clamped into its cell's own range before the
/// op is made — and it is in the vocabulary because a refusal that left no line in
/// the black box would be worse than one nobody can explain.
pub const REFUSED: &str = "refused";

/// **Every reason a shot can miss for**, in code order. Pinned by
/// `crates/tacsim/tests/resolution.rs`, so a seventh is a reviewed diff.
pub const MISS_REASONS: [&str; 7] = [
    TARGET_GONE,
    WRECK,
    MOVED,
    BLOCKED,
    COVER,
    DISPERSION,
    REFUSED,
];

/// The condition a section takes when its internals reach zero.
pub const DESTROYED: &str = "destroyed";

/// The condition doc 16 §3.3 makes a wreck: `hull` condition `wreck`.
pub const WRECK_CONDITION: &str = "wreck";

/// The section doc 16 §3.3 puts that condition on.
pub const HULL_SECTION: &str = "hull";

/// The power capability a wreck loses (doc 16 §3.3).
pub const POWER_CAPABILITY: &str = "tacsim.power";

/// The mobility capability a wreck loses (doc 16 §3.3; M32's own id).
pub const MOBILITY_CAPABILITY: &str = "tactics.mobility";

/// The prefix of every weapon capability a wreck loses (doc 16 §3.3: *every
/// `tacsim.weapon.*`*).
pub const WEAPON_CAPABILITY_PREFIX: &str = "tacsim.weapon.";

/// How far from the predicted point a target may be and still be hit, in cells
/// (doc 16 §3.5: *within one cell of the prediction*). Chebyshev, so a diagonal
/// neighbour counts.
pub const PREDICTION_SLACK_CELLS: u16 = 1;

/// **Whether one unit is a wreck** (doc 16 §3.3 "A wreck": *the game-crate
/// predicate `is_wreck(unit)` reads exactly that*).
///
/// Exactly the [`WRECK_CONDITION`] on the unit's body — the [`HULL_SECTION`] for
/// every unit that has one — and nothing else. The
/// three capability revocations doc 16 names are in the **same plan**, but they are
/// not what the predicate reads, for a reason worth writing down: a revoke is
/// idempotent and a unit that never held a capability loses nothing by it, so a
/// predicate over the capability set would answer "wreck" for a tank whose content
/// declared no power plant. The condition is the mark; the revocations are the
/// consequences of carrying it.
///
/// It is the **world's** reader, not the arena's — the arena has
/// `UnitState::limits.wreck` beside the unit and never needs to ask the M32 cells.
/// This is what a checked-in tank, a `@unit/show` and cut W.3's story test read, and
/// it is a pure function of a [`mush_tactics::TacticalUnit`] so it can be called from
/// either side of the seam.
///
/// **16b cut X.2** made it read *any* section carrying the mark rather than the
/// [`HULL_SECTION`] alone, which is where `Arena::wreck_ops` has always put it:
/// the hull when the unit has one, and the triggering section when it does not. A
/// biped has a torso and no hull, and until this cut the predicate could not see
/// its wreck. For every unit that has a hull the answer is what it was — nothing
/// but `wreck_ops` ever writes the condition, and it writes it once.
#[must_use]
pub fn is_wreck(unit: &mush_tactics::TacticalUnit) -> bool {
    unit.sections()
        .any(|(_, section)| section.condition.as_str() == WRECK_CONDITION)
}

/// **What a unit's own record already says about it, at check-out** (16b cut
/// F.1; doc 16 §3.1 "Custody": *`snapshot` is the unit's cells*).
///
/// Custody hands the arena a `TacticalUnit` and the arena used to open every match
/// with [`MotionLimits::default()`](crate::arena::state::MotionLimits) — no wreck,
/// no disabled motion, no lost track — however the record read. A tank whose left
/// track had been shot off in a previous match, or by a staff `@unit/damage`, drove
/// at full speed and turned both ways; a tank whose hull was a wreck started up and
/// fought, and could never be wrecked again because `Arena::plan_hit` fires a
/// section's consequences only on the **transition** to zero internals.
///
/// This is the missing read: the sections the record calls `destroyed` (and the
/// body it calls `wreck`) are walked once, in **section-id order** — the order
/// [`mush_tactics::TacticalUnit`]'s own ordered map holds them in, and not the
/// order the pack authored them in — and
/// every consequence they carry is put into force before the first pulse — exactly
/// the consequences [`Arena::apply_consequence`] would have applied had the arena
/// watched them happen. The two are deliberately the same vocabulary and the same
/// rules, including `slow`'s "name the surviving percentage" and
/// `disable_motion_when_all_of`'s pair rule.
///
/// The caller ([`crate::arena::state::Arena::new`]) applies the two halves this
/// cannot: the weapons of a [`Consequence::RemoveWeapons`] section (and of a wreck)
/// leave the table, and a unit with no weapons and no motion opens `disabled`.
#[must_use]
pub fn entry_consequences(
    unit: &mush_tactics::TacticalUnit,
    profile: &crate::arena::resolution::UnitProfile,
) -> EntryConsequences {
    let mut entry = EntryConsequences::default();
    let destroyed: Vec<&str> = unit
        .sections()
        .filter(|(_, state)| {
            state.condition.as_str() == DESTROYED || state.condition.as_str() == WRECK_CONDITION
        })
        .map(|(id, _)| id.as_str())
        .collect();
    // A wreck is the mark itself, whether or not its hull's own consequence list
    // survived the content edit that wrote it: `is_wreck` is the predicate doc 16
    // §3.3 names and this honours it directly.
    if is_wreck(unit) {
        entry.limits.wreck = true;
        entry.limits.motion_disabled = true;
    }
    for section in &destroyed {
        let Some(row) = profile.section(section) else {
            continue;
        };
        for consequence in &row.consequences {
            match consequence {
                Consequence::DestroyUnit => {
                    entry.limits.wreck = true;
                    entry.limits.motion_disabled = true;
                }
                Consequence::Slow { percent } => {
                    entry.limits.slow_pct = entry.limits.slow_pct.min((*percent).clamp(1, 100));
                }
                // 16b cut M.4 (`B-47`): ORed in, so a record with both tracks
                // destroyed opens closed both ways, whichever is read first.
                Consequence::NoTurnToward { side } => entry.limits.close_turn(*side),
                Consequence::DisableMotion => entry.limits.motion_disabled = true,
                Consequence::DisableMotionWhenAllOf { sections } => {
                    if !sections.is_empty()
                        && sections
                            .iter()
                            .all(|want| destroyed.iter().any(|gone| *gone == want.as_str()))
                    {
                        entry.limits.motion_disabled = true;
                    }
                }
                Consequence::RemoveWeapons { section } => entry.disarmed.push(section.clone()),
            }
        }
    }
    entry
}

/// What [`entry_consequences`] found on a unit's record (16b cut F.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryConsequences {
    /// The motion limits the match opens with.
    pub limits: crate::arena::state::MotionLimits,
    /// The sections whose weapons are already gone, in the order they were read.
    pub disarmed: Vec<SmolStr>,
}

impl EntryConsequences {
    /// Whether a weapon mounted on `section` is still on the platform.
    #[must_use]
    pub fn carries(&self, section: &str) -> bool {
        !self.limits.wreck && !self.disarmed.iter().any(|gone| gone.as_str() == section)
    }
}

// ---------------------------------------------------------------------------
// The impact
// ---------------------------------------------------------------------------

/// **One unit as the arena hands it back** (16b cut W.2; doc 16 §3.1's
/// `CustodyIn { units[{cells, final_pose, outcome}] }`).
///
/// It is the per-unit half of the seventh seam message, which cut T.2's `Ended`
/// carried none of because there was no damage model to carry: the cells the match
/// left the unit with, the pose it left it at, what became of it, and the two
/// counters the result record's per-unit row wants. The world turns it into one
/// [`mush_tactics::UnitMutationPlan`] per unit against the check-out snapshot; the
/// arena builds no plan for the world, because a plan is a statement about a
/// *stored* unit and the arena has never seen one.
#[derive(Clone, Debug)]
pub struct UnitReturn {
    /// The unit.
    pub unit: ObjId,
    /// Its cells, whole — every damage plan the match applied, folded.
    pub cells: mush_tactics::TacticalUnit,
    /// Where the arena left it.
    pub pose: mush_sim::data::Pose,
    /// How many shots it fired.
    pub shots: u32,
    /// **How many of them took something off something** (16b cut Q.6).
    ///
    /// Not the same number as the ledger's count of `hit` lines, and deliberately
    /// so: a round that reaches a section already at zero armour and zero
    /// internals did land, and the black box records it, but it moved no cell and
    /// bumped no revision — so counting it here flattered every shooter who kept
    /// firing at a spent track. This is the figure a pilot reads his own gunnery
    /// off ([`Arena::resolve_impact`]'s guard is the one place it is raised).
    pub hits: u32,
    /// It is a wreck (doc 16 §3.3).
    pub wreck: bool,
    /// It has no weapons and cannot move (doc 16 §3.8).
    pub disabled: bool,
}

/// What one impact did to one section, for the `hit` line and for the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Struck {
    /// The section the facet draw chose.
    section: SmolStr,
    /// The facet whose armour the round passed.
    facet: SmolStr,
    /// **Whose armour that facet is** (16b cut M.3, 16c `T-13`): the
    /// struck-through section's for a section with no armour of its own, else
    /// `section` itself — `plan_hit`'s own `struck_through` match, carried out to
    /// the `hit` line so the facet is never read against the wrong section.
    armor_section: SmolStr,
    /// The damage the round or the burst carried.
    damage: u32,
    /// The armour left in that facet.
    armor_left: u32,
    /// The internals left in that section.
    internals_left: u32,
    /// The consequences that fired, in authored order; empty unless the section's
    /// internals reached zero on this hit.
    consequences: Vec<Consequence>,
    /// **The section reached zero internals on THIS hit** (16b cut F.2).
    ///
    /// The transition, and not the value. Until this cut `apply_hit` journalled
    /// `section_destroyed` whenever `internals_left == 0`, so every further round
    /// that landed on a spent turret wrote a second destruction — and a reader
    /// counting destructions counted them twice. The consequences were already
    /// gated on the transition (`plan_hit`'s `fresh`); this carries the same fact
    /// out so the line is too.
    fresh: bool,
}

impl Arena {
    /// **Apply one scheduled impact** (16b cut W.2's half of doc 16 §3.5), inside
    /// the substep whose boundary its timestamp fell in and against the positions
    /// that substep committed.
    ///
    /// The shot leaves the in-flight table and journals `impact`, then either one
    /// `hit` or one `miss` — never both, never neither. An `Impact` whose shot is
    /// not in the table journals nothing: the only way to reach it is a shot
    /// already resolved, and a black-box line about a shot nobody fired would be
    /// worse than silence.
    ///
    /// Everything it can do to the target it does through
    /// `Arena::apply_hit`: one plan, atomically.
    pub fn resolve_impact(&mut self, shot_seq: u64, t_ms: i64, journal: &mut Vec<Notice>) {
        let Some(shot) = self.shots.remove(&shot_seq) else {
            return;
        };
        journal.push((
            t_ms,
            EventKind::Impact {
                shot_seq,
                unit: shot.shooter,
                target: shot.target,
            },
            Vec::new(),
        ));
        let miss = |reason: &str, journal: &mut Vec<Notice>| {
            journal.push((
                t_ms,
                EventKind::Miss {
                    shot_seq,
                    reason: SmolStr::new(reason),
                },
                Vec::new(),
            ));
        };
        // (1) The target exists, and is not already a wreck.
        let Some(target) = self.unit(shot.target) else {
            miss(TARGET_GONE, journal);
            return;
        };
        if target.limits.wreck {
            miss(WRECK, journal);
            return;
        }
        // (2) It is within one cell of the prediction. A target that was off the
        // window when the shot was taken, or is off it now, is read as moved: the
        // prediction has no cell to be near.
        let (Some(aimed), Some(now_at)) =
            (shot.aim_cell, self.definition.terrain.cell_of(&target.pose))
        else {
            miss(MOVED, journal);
            return;
        };
        if aimed.0.abs_diff(now_at.0).max(aimed.1.abs_diff(now_at.1)) > PREDICTION_SLACK_CELLS {
            miss(MOVED, journal);
            return;
        }
        // (3) The line is still clear, and the ground it ends on is not full cover.
        // A shooter that has left the match (its own wreck, mid-flight) leaves
        // nothing to trace from: the round is already in the air, so the line is
        // taken as it was and the shot resolves on its own merits.
        if self.unit(shot.shooter).is_some() {
            let (sight, cover) = self.fire_line(shot.shooter, shot.target);
            // **16b cut N.1**: doc 16 §6's `no line` edge is fed from here and from
            // nowhere else. The reflex controller learns that a round of its own was
            // stopped by the ground the way anybody would — by the shot missing —
            // and re-plans from where it is standing, rather than by taking a trace
            // of its own before every shot. A pilot's unit notes nothing.
            if !sight.is_clear_or_partial() {
                self.note_no_line(shot.shooter);
                miss(BLOCKED, journal);
                return;
            }
            if cover == Cover::Full {
                self.note_no_line(shot.shooter);
                miss(COVER, journal);
                return;
            }
        }
        // (4) The outcome. The probability is the one taken at acceptance, unless
        // the target turned: then the same cone and the same draw against the new
        // band's width (doc 16 §3.5).
        let band = match self.unit(shot.shooter) {
            Some(from) => crate::arena::resolution::facet_band(target, from),
            // A shooter that has left the match — its own wreck, mid-flight. The
            // face the target was showing when the round left is the best the record
            // has; re-deriving one from a shooter that is not there would invent it.
            None => shot.resolution.band.as_str(),
        };
        let taken = if band == shot.resolution.band.as_str() {
            shot.resolution.clone()
        } else {
            self.reresolve(shot.target, &shot.resolution, band, shot.range_cm)
        };
        let landed = self.rounds_landed(shot_seq, shot.rounds, taken.p_permille);
        let drawn = self.shot_draw(shot_seq, shot.rounds);
        if landed == 0 {
            journal.push((
                t_ms,
                EventKind::Miss {
                    shot_seq,
                    reason: SmolStr::new_static(DISPERSION),
                },
                vec![drawn],
            ));
            return;
        }
        self.apply_hit(&shot, &taken, landed, t_ms, drawn, journal);
    }

    /// **The one trace a shot takes**, answering what stood in the line and what
    /// cover the ground the target stands on gives (doc 16 §3.5 step 1 and
    /// "Scheduled impact").
    ///
    /// The arena holds a cell grid and no window (D-16-3), so this is
    /// `mush_atlas::trace_cells` — the same line walk `direct_fire_path` is, with
    /// no feature overlay — and [`mush_atlas::trace::CellTrace::target_cover`] is
    /// the field doc 16 names. Both eyes are the units' own `sensor_height_cm`, as
    /// S.1's perception pass takes them: a tank shoots from about where it looks.
    #[must_use]
    pub fn fire_line(&self, shooter: ObjId, target: ObjId) -> (Sight, Cover) {
        match self.fire_trace(shooter, target) {
            Some(trace) => (trace.sight, trace.target_cover),
            None => (Sight::Clear, Cover::None),
        }
    }

    /// Just the cover half of [`Arena::fire_line`].
    #[must_use]
    pub fn fire_cover(&self, shooter: ObjId, target: ObjId) -> Cover {
        self.fire_line(shooter, target).1
    }

    /// **The cover a target would have at a cell it is not necessarily in** (16b
    /// cut F.3) — what the cockpit's advertised percentage needs.
    ///
    /// The shot itself traces to where the target *is* (that is physics, and
    /// [`Arena::fire_line`] is what it uses). The number the cockpit shows a pilot
    /// is **advice**, and advice may only be computed from what the platform
    /// knows, so it traces to the cell the contact was last seen in. A target that
    /// has driven behind a building since the last sighting therefore still shows
    /// the chance its plot implies, and the shot that follows finds the building
    /// — which is the honest shape of a stale track and not a bug.
    ///
    /// The target's own eye height is used, because how tall a tank is is not
    /// something a plot goes stale about.
    #[must_use]
    pub fn fire_cover_at(&self, shooter: ObjId, target: ObjId, cell: (u16, u16)) -> Cover {
        self.fire_trace_at(shooter, target, cell)
            .map_or(Cover::None, |trace| trace.target_cover)
    }

    /// **What a beam would lose on the way to a cell the target is not necessarily
    /// in** (16b cut F.3), percent — [`Arena::fire_attenuation`]'s advice half, for
    /// the cone [`crate::arena::Arena::resolve_seen`] advertises.
    #[must_use]
    pub fn fire_attenuation_at(&self, shooter: ObjId, target: ObjId, cell: (u16, u16)) -> u32 {
        self.fire_trace_at(shooter, target, cell)
            .map_or(0, |trace| trace.attenuation)
    }

    /// **What a beam loses on the way** (16b cut X.1; doc 16 §9 item 1:
    /// *resolution uses `beam_path` attenuation as a cover term*), percent.
    ///
    /// It is the third field of the very same walk [`Arena::fire_line`] reads, and
    /// deliberately so. `mush_atlas::beam_path` and `direct_fire_path` differ by
    /// one line — the beam levels an eye that a straight flight has already left
    /// level — so over a [`mush_atlas::trace::CellGrid`] the beam walk **is**
    /// `trace_cells`' walk, and a `beam_cells` beside it would be the same
    /// function under a second name. What the beam wants that direct fire does not
    /// is the attenuation the walk gathered, which cut S.1 computed and cut W.2 had
    /// no reader for.
    ///
    /// It is a read of its own rather than a third element of `fire_line`'s tuple
    /// because that tuple is destructured in the impact and in the reflex
    /// controller, and because only a beam ever asks: a direct-fire shot pays
    /// nothing for this existing.
    #[must_use]
    pub fn fire_attenuation(&self, shooter: ObjId, target: ObjId) -> u32 {
        self.fire_trace(shooter, target)
            .map_or(0, |trace| trace.attenuation)
    }

    /// The one walk `fire_line` and `fire_attenuation` are two readings of.
    fn fire_trace(&self, shooter: ObjId, target: ObjId) -> Option<mush_atlas::trace::CellTrace> {
        let cell = self.definition.terrain.cell_of(&self.unit(target)?.pose)?;
        self.fire_trace_at(shooter, target, cell)
    }

    /// The same walk, to a cell the target is not necessarily in (16b cut F.3) —
    /// the one `fire_cover_at` and `fire_attenuation_at` are two readings of.
    fn fire_trace_at(
        &self,
        shooter: ObjId,
        target: ObjId,
        cell: (u16, u16),
    ) -> Option<mush_atlas::trace::CellTrace> {
        let from = self.fire_eye(self.unit(shooter)?)?;
        let at = self.unit(target)?;
        let to = mush_atlas::trace::Eye {
            cell,
            height_cm: i32::try_from(at.sensor_height_cm).unwrap_or(i32::MAX),
        };
        Some(trace_cells(&self.definition.terrain, from, to))
    }

    /// One unit's eye, for a fire trace.
    fn fire_eye(&self, unit: &UnitState) -> Option<mush_atlas::trace::Eye> {
        let cell = self.definition.terrain.cell_of(&unit.pose)?;
        Some(mush_atlas::trace::Eye {
            cell,
            height_cm: i32::try_from(unit.sensor_height_cm).unwrap_or(i32::MAX),
        })
    }

    /// **One hit** (doc 16 §3.5 "Damage"): the facet draw, the armour, the
    /// overflow, the consequence, and one plan for all of it.
    ///
    /// `drawn` is the shot's own `hit:<seq>` or `burst:<seq>` draw. **16b cut
    /// M.3** (16c `T-07`): every line this writes after the section is chosen
    /// carries both draws the round made, in the order it made them — the shot's,
    /// then `facet:<seq>` — and the one miss written before it (no facet table
    /// for the band) carries the shot's alone, because the facet draw was never
    /// taken.
    fn apply_hit(
        &mut self,
        shot: &crate::arena::weapons::Shot,
        taken: &Resolution,
        landed: u32,
        t_ms: i64,
        drawn: (String, u64),
        journal: &mut Vec<Notice>,
    ) {
        let damage = shot.damage.saturating_mul(landed);
        let Some((section, facet_value)) = self.choose_section(shot.target, &taken.band, shot.seq)
        else {
            // A target whose content authored no facet table for this band has
            // nothing for the round to strike. It is not a validation failure and
            // not a dispersion: the round arrived and found no structure, which is
            // the same outcome as a round that went wide, said with the same word.
            journal.push((
                t_ms,
                EventKind::Miss {
                    shot_seq: shot.seq,
                    reason: SmolStr::new_static(DISPERSION),
                },
                vec![drawn],
            ));
            return;
        };
        let drawn = vec![
            drawn,
            (
                DrawLabel::Facet { shot_seq: shot.seq }.encode(),
                facet_value,
            ),
        ];
        let Some(struck) = self.plan_hit(shot.target, &section, &taken.band, damage) else {
            journal.push((
                t_ms,
                EventKind::Miss {
                    shot_seq: shot.seq,
                    reason: SmolStr::new_static(REFUSED),
                },
                drawn,
            ));
            return;
        };
        let (struck, ops) = struck;
        let cause = SmolStr::new(format!("shot:{}", shot.seq));
        let Ok(plan) = UnitMutationPlan::new(ops, None, Some(cause.clone())) else {
            journal.push((
                t_ms,
                EventKind::Miss {
                    shot_seq: shot.seq,
                    reason: SmolStr::new_static(REFUSED),
                },
                drawn,
            ));
            return;
        };
        // **16b cut F.2**: a plan with no ops is not applied at all. It happens for
        // exactly one shot — a round that lands on a section already at zero
        // armour and zero internals, where there is nothing left to spend — and
        // `apply` would fold no ops and hand back a unit whose only difference is
        // a bumped revision. A revision is what the check-out diff is taken
        // against; bumping it for a round that moved nothing put a lie in the one
        // document that outlives the match. The round is still journalled as a
        // hit, truthfully (see below) — but from 16b cut Q.6 it is not counted as
        // one on the shooter's result row, which is inside this guard.
        if !plan.ops().is_empty() {
            let applied = self
                .unit(shot.target)
                .map(|state| mush_tactics::apply(&state.unit, &plan, t_ms));
            let Some(Ok((post, _))) = applied else {
                // Refuse, never clamp (M32): the caller's unit is untouched, so the
                // only thing left to do is say so.
                journal.push((
                    t_ms,
                    EventKind::Miss {
                        shot_seq: shot.seq,
                        reason: SmolStr::new_static(REFUSED),
                    },
                    drawn,
                ));
                return;
            };
            // **16b cut Q.2**: through the one writer, so the band cache the
            // gunner's `SCAN` reads moves with the round that changed it.
            let touched = Arena::plan_touches_bands(&plan);
            self.set_unit(shot.target, post, touched);
            // **16b cut Q.6**: and the shooter's count is inside the guard with
            // it. The result record's `hits` (doc 16 §4) is what a pilot reads
            // his own gunnery off, and a round that reached a section already at
            // zero armour and zero internals took nothing off anything — counting
            // it flattered every shooter who kept firing at a spent track. The
            // **ledger's** `hit` line below and the pilot's notice are unchanged:
            // the round did land, and that is a different sentence.
            if let Some(state) = self.unit_mut(shot.shooter) {
                state.hits = state.hits.saturating_add(1);
            }
        } else {
            // **16b cut D.2** (`B-15`): this is the one branch a round that found
            // nothing left to break reaches, so it is where the fact is recorded
            // for the staging scan a few lines later in the same substep. The
            // `hit` line below does not change by a byte; what changes is what
            // the victim and the gunner who has told the target apart are told
            // about it (`Arena::spent_hits`).
            self.spent_hits.insert(shot.seq);
        }
        journal.push((
            t_ms,
            EventKind::Hit {
                shot_seq: shot.seq,
                unit: shot.shooter,
                target: shot.target,
                section: struck.section.clone(),
                facet: struck.facet.clone(),
                armor_section: struck.armor_section.clone(),
                rounds: landed,
                damage: struck.damage,
                armor_left: struck.armor_left,
                internals_left: struck.internals_left,
                consequences: struck
                    .consequences
                    .iter()
                    .map(|consequence| SmolStr::new_static(consequence.label()))
                    .collect(),
                cause,
            },
            drawn,
        ));
        // **16b cut F.2**: on the **transition**, not on the value. Doc 16 §3.5
        // names no rule for a round that arrives at a section already at zero, so
        // this engine's answer is that it is a hit that finds nothing left to
        // break: it is journalled as the `hit` above, with its damage and its
        // two noughts, and the section is destroyed **once**. The round is not
        // spilled to the hull — that would be a damage rule doc 16 does not have
        // — and `struck_through` stays the only way a section's damage reaches
        // another's armour (doc 16 §3.3).
        if struck.fresh {
            journal.push((
                t_ms,
                EventKind::SectionDestroyed {
                    unit: shot.target,
                    section: struck.section.clone(),
                    // **16b cut D.4** (`T-08`): the round that did it, carried on
                    // the line rather than inferred from whatever impact the
                    // staging scan happened to be holding.
                    by: Some(shot.shooter),
                    consequences: struck
                        .consequences
                        .iter()
                        .map(|consequence| SmolStr::new_static(consequence.label()))
                        .collect(),
                },
                Vec::new(),
            ));
            for consequence in &struck.consequences {
                self.apply_consequence(shot.target, consequence, t_ms, Some(shot.shooter), journal);
            }
            self.settle_disabled(shot.target, t_ms, journal);
        }
    }

    /// **Which section a hit on `band` strikes** (doc 16 §3.3 "Facet attribution",
    /// W12): the band's weighted table, drawn with `facet:<shot_seq>`.
    ///
    /// The draw is taken modulo the weights' sum and the table is walked in
    /// authored order, so the choice is a function of the shot's sequence and the
    /// content's own order and nothing else. A band with no sections, or whose
    /// weights are all zero, answers `None` — and takes no draw.
    ///
    /// **16b cut M.3** (16c `T-07`): it answers the section **and the draw's
    /// value**, the whole `u64` before the modulus, so the `hit` line can carry
    /// `facet:<shot_seq>` beside the shot's own draw and a reader holding the
    /// seed can re-derive which section a round found.
    #[must_use]
    pub fn choose_section(
        &self,
        target: ObjId,
        band: &str,
        shot_seq: u64,
    ) -> Option<(SmolStr, u64)> {
        let profile = &self.unit(target)?.profile;
        let rows = &profile.band(band)?.sections;
        let total: u64 = rows.iter().map(|(_, weight)| u64::from(*weight)).sum();
        if total == 0 {
            return None;
        }
        let value = draw(self.seed, &DrawLabel::Facet { shot_seq });
        let mut pick = value % total;
        for (section, weight) in rows {
            let weight = u64::from(*weight);
            if pick < weight {
                return Some((section.clone(), value));
            }
            pick = pick.saturating_sub(weight);
        }
        rows.last().map(|(section, _)| (section.clone(), value))
    }

    /// **Build the ops of one hit**, and say what they will leave behind.
    ///
    /// Doc 16 §3.5 "Damage": *reduces the facet's `armor` cell by `damage` down to
    /// its floor, applies overflow to the section's `internals`, and at zero
    /// internals sets the section `destroyed` and applies its consequence.* Every
    /// value is computed against the cell's own `min` first, so no op this builds
    /// can be out of bounds and M32's refusal is a guard rather than a path.
    ///
    /// The armour a round must pass is the **struck-through** section's when the
    /// chosen section declares none of its own (doc 16 §3.3: the engine is *struck
    /// through `hull.rear`*), and otherwise the chosen section's facet **of this
    /// band** — falling back to its first authored facet, which is what a track
    /// hit from the rear quarter takes.
    fn plan_hit(
        &self,
        target: ObjId,
        section: &str,
        band: &str,
        damage: u32,
    ) -> Option<(Struck, Vec<UnitMutationOp>)> {
        let state = self.unit(target)?;
        let profile = state.profile.section(section)?;
        let (armor_section, armor_facet) = match &profile.struck_through {
            Some((through, facet)) => (through.clone(), facet.clone()),
            None => {
                let facet = profile
                    .facets
                    .iter()
                    .find(|facet| facet.as_str() == band)
                    .or_else(|| profile.facets.first())?;
                (SmolStr::new(section), facet.clone())
            }
        };
        let armor_addr = CellAddr::Facet {
            section: SectionId::new(armor_section.as_str()).ok()?,
            facet: mush_tactics::FacetId::new(armor_facet.as_str()).ok()?,
            cell: mush_tactics::CellId::new(ARMOR_CELL).ok()?,
        };
        let internals_addr = CellAddr::Section {
            section: SectionId::new(section).ok()?,
            cell: mush_tactics::CellId::new(INTERNALS_CELL).ok()?,
        };
        let armor = state.unit.cell(&armor_addr)?;
        let internals = state.unit.cell(&internals_addr)?;
        let damage = i64::from(damage);
        let spent = damage.min(armor.current.saturating_sub(armor.min));
        let armor_left = armor.current.saturating_sub(spent);
        let overflow = damage.saturating_sub(spent);
        let internals_left = internals
            .current
            .saturating_sub(overflow.min(internals.current.saturating_sub(internals.min)));
        let mut ops: Vec<UnitMutationOp> = Vec::new();
        if armor_left != armor.current {
            ops.push(UnitMutationOp::SetCell {
                addr: armor_addr,
                value: armor_left,
            });
        }
        if internals_left != internals.current {
            ops.push(UnitMutationOp::SetCell {
                addr: internals_addr,
                value: internals_left,
            });
        }
        let gone = internals_left <= internals.min;
        // The consequences fire on the **transition** to zero and not on every
        // round that arrives at a section already gone, so a dead track is not
        // re-slowed and a wreck is not re-wrecked.
        let fresh = gone && internals.current > internals.min;
        let consequences = if fresh {
            profile.consequences.clone()
        } else {
            Vec::new()
        };
        if fresh {
            ops.push(UnitMutationOp::SetSectionCondition {
                section: SectionId::new(section).ok()?,
                condition: ConditionId::new(DESTROYED).ok()?,
            });
            if consequences
                .iter()
                .any(|consequence| matches!(consequence, Consequence::DestroyUnit))
            {
                ops.extend(self.wreck_ops(target, section));
            }
        }
        Some((
            Struck {
                section: SmolStr::new(section),
                facet: armor_facet,
                armor_section,
                damage: u32::try_from(damage).unwrap_or(u32::MAX),
                armor_left: u32::try_from(armor_left.max(0)).unwrap_or(0),
                internals_left: u32::try_from(internals_left.max(0)).unwrap_or(0),
                consequences,
                fresh,
            },
            ops,
        ))
    }

    /// **The ops that make a wreck** (doc 16 §3.3 "A wreck"): `hull` condition
    /// `wreck`, plus the revocation of `tacsim.power`, `tactics.mobility` and every
    /// `tacsim.weapon.*` the unit holds.
    ///
    /// The condition goes on the section doc 16 names — `hull` — and on the
    /// triggering section when the unit has no hull, so a unit whose content calls
    /// its body something else still carries the mark that `is_wreck` reads.
    ///
    /// The revocations are taken from the keys the unit **actually holds**, because
    /// a [`CapabilityKey`] is an id *and* its source component and only the unit
    /// knows which component granted what. Revoking is idempotent, so a unit
    /// holding none of the three produces no op and is no less a wreck for it: the
    /// condition is what the predicate reads.
    fn wreck_ops(&self, target: ObjId, section: &str) -> Vec<UnitMutationOp> {
        let Some(state) = self.unit(target) else {
            return Vec::new();
        };
        let body = if state.profile.section(HULL_SECTION).is_some() {
            HULL_SECTION
        } else {
            section
        };
        let mut ops: Vec<UnitMutationOp> = Vec::new();
        if let (Ok(section), Ok(condition)) =
            (SectionId::new(body), ConditionId::new(WRECK_CONDITION))
        {
            ops.push(UnitMutationOp::SetSectionCondition { section, condition });
        }
        let revoked: Vec<CapabilityKey> = state
            .unit
            .capabilities()
            .filter(|key| {
                let id = key.id.as_str();
                id == POWER_CAPABILITY
                    || id == MOBILITY_CAPABILITY
                    || id.starts_with(WEAPON_CAPABILITY_PREFIX)
            })
            .cloned()
            .collect();
        for key in revoked {
            ops.push(UnitMutationOp::RevokeCapability { key });
        }
        ops
    }

    /// **Which of a destroyed section's authored consequences actually took
    /// effect** (16b cut F.2), in authored order and by their ledger words.
    ///
    /// The `section_destroyed` line carries the section's **whole** authored list,
    /// because the black box records what the content says about the section, and
    /// that is not changed here. But the pilot's line is a statement about this
    /// tank now, and [`Consequence::DisableMotionWhenAllOf`] is the one
    /// consequence that is written down and then does nothing: a biped that loses
    /// one leg is slowed and still walking (16b cut X.2, doc 16 §9 item 2), and
    /// `The left_leg is destroyed: the tank is slowed; the tank cannot move.` is
    /// a contradiction whose second half the pilot would act on. So the notice's
    /// list is put through the same question [`Arena::apply_consequence`] asked.
    #[must_use]
    pub fn fired_consequences(&self, target: ObjId, section: &str) -> Vec<SmolStr> {
        let Some(state) = self.unit(target) else {
            return Vec::new();
        };
        let Some(profile) = state.profile.section(section) else {
            return Vec::new();
        };
        profile
            .consequences
            .iter()
            .filter(|consequence| match consequence {
                Consequence::DisableMotionWhenAllOf { sections } => {
                    self.all_destroyed(target, sections)
                }
                _ => true,
            })
            .map(|consequence| SmolStr::new_static(consequence.label()))
            .collect()
    }

    /// **Apply one consequence to the arena's own facts** (doc 16 §3.3's column).
    ///
    /// `destroy_unit` is [`Arena::destroy_unit`]; the other four are the
    /// [`MotionLimits`](crate::arena::state::MotionLimits) A.2's motion reads and
    /// the weapon table W.1's acceptance reads. Each is idempotent: a second
    /// `slow(50)` on an already-slowed tank leaves it at 50 rather than at 25,
    /// because the consequence names the surviving percentage and not a factor to
    /// compound.
    ///
    /// **16b cut D.4** (`T-08`) threads `by` through: the shooter whose round
    /// fired this consequence, or `None` for a consequence applied by anything
    /// else. It reaches exactly one line, `unit_destroyed`'s.
    pub fn apply_consequence(
        &mut self,
        target: ObjId,
        consequence: &Consequence,
        t_ms: i64,
        by: Option<ObjId>,
        journal: &mut Vec<Notice>,
    ) {
        match consequence {
            Consequence::DestroyUnit => self.destroy_unit(target, t_ms, by, journal),
            Consequence::Slow { percent } => {
                if let Some(state) = self.unit_mut(target) {
                    state.limits.slow_pct = state.limits.slow_pct.min((*percent).clamp(1, 100));
                }
            }
            // **16b cut M.4** (16c `B-47`): ORed in. Before M.4 the second
            // track's loss overwrote the first's side and a tank on no tracks
            // still turned one way; now both sides stay closed and
            // `crate::arena::motion`'s `turn` does not turn at all.
            Consequence::NoTurnToward { side } => {
                if let Some(state) = self.unit_mut(target) {
                    state.limits.close_turn(*side);
                }
            }
            Consequence::DisableMotion => self.disable_motion(target),
            // **16b cut X.2** (doc 16 §9 item 2): the pair rule, read off the M32
            // unit's own section conditions — the very conditions the impact's
            // plan set a moment ago, so "the second leg has just gone" is a fact
            // about the unit and not a count the arena keeps beside it. A section
            // this unit does not declare can never be destroyed, so a list naming
            // one never fires; the loader refuses that list anyway.
            Consequence::DisableMotionWhenAllOf { sections } => {
                if self.all_destroyed(target, sections) {
                    self.disable_motion(target);
                }
            }
            Consequence::RemoveWeapons { section } => {
                if let Some(state) = self.unit_mut(target) {
                    let going: Vec<SmolStr> = state
                        .weapons
                        .iter()
                        .filter(|weapon| weapon.mount_section == *section)
                        .map(|weapon| weapon.id.clone())
                        .collect();
                    state
                        .weapons
                        .retain(|weapon| weapon.mount_section != *section);
                    for id in going {
                        state.ready_at_ms.remove(&id);
                    }
                    // **16b cut F.1**: the rounds STAY. `ammo` is not the weapon
                    // table — it is the arena's record of how many rounds that
                    // barrel has left, and [`Arena::returned_units`] is what
                    // carries it home to the unit's own `ammo` cell. Dropping the
                    // row made a turret shot off at round twelve check in with a
                    // full magazine, which is a lie about the match in the one
                    // document that outlives it. The weapon cannot fire either
                    // way: `weapons` is what `FIRE` resolves against.
                }
            }
        }
    }

    /// The motion half of `disable_motion`, shared by the consequence and by the
    /// pair rule above so the two cannot come to mean different things.
    fn disable_motion(&mut self, target: ObjId) {
        if let Some(state) = self.unit_mut(target) {
            state.limits.motion_disabled = true;
            state.speed_cms = 0;
            state.desired_speed_cms = 0;
        }
    }

    /// **Whether every one of `sections` is destroyed** (16b cut X.2), off the M32
    /// unit's section conditions.
    ///
    /// An empty list is `false` rather than the vacuous truth: "all of nothing"
    /// would disable a unit's motion the first time any section carrying the
    /// consequence was lost, which is `disable_motion` under another name and
    /// never what an author meant. The loader refuses the empty list too, so this
    /// is the second of two guards on the same mistake.
    fn all_destroyed(&self, target: ObjId, sections: &[SmolStr]) -> bool {
        let Some(state) = self.unit(target) else {
            return false;
        };
        !sections.is_empty()
            && sections.iter().all(|section| {
                SectionId::new(section.as_str()).is_ok_and(|id| {
                    state
                        .unit
                        .section(&id)
                        .is_some_and(|section| section.condition.as_str() == DESTROYED)
                })
            })
    }

    /// **Make a wreck** (doc 16 §3.3): the M32 half is already in the impact's own
    /// plan, and this is the arena's half.
    ///
    /// Everything that made the unit an actor stops: the motion limits say wreck,
    /// the power goes out, it stops where it is, it drops its lock, it loses every
    /// weapon, its scheduled sensor and share work leaves the heap, and its own
    /// contact store is emptied — *a wreck is an obstacle and a contact, never a
    /// target*, and a thing that holds no contacts is a thing that reports nothing
    /// and shares nothing. Every **other** platform's contact on it is left exactly
    /// where it is and expires through the ordinary time-to-live, which is what
    /// clears their locks and says so once (W.1 put that in the `lost` arm).
    ///
    /// Idempotent: a wreck destroyed again journals nothing.
    ///
    /// **16b cut D.4** (`T-08`): `by` is the shooter whose round finished it, and
    /// `None` for a hull destroyed by anything else — which is what the gunner's
    /// kill line is gated on, in place of the stale `in_flight` it used to read.
    pub fn destroy_unit(
        &mut self,
        target: ObjId,
        t_ms: i64,
        by: Option<ObjId>,
        journal: &mut Vec<Notice>,
    ) {
        if self.unit(target).is_none_or(|state| state.limits.wreck) {
            return;
        }
        if let Some(state) = self.unit_mut(target) {
            state.limits.wreck = true;
            state.limits.motion_disabled = true;
            state.power = Power::Off;
            state.speed_cms = 0;
            state.desired_speed_cms = 0;
            state.lock = None;
            state.weapons.clear();
            state.ready_at_ms.clear();
            // **16b cut F.1**: the rounds stay, for the reason `RemoveWeapons`
            // gives above — a destroyed tank checked in with a full magazine
            // although it had fired all match, because the one record of what it
            // spent was thrown away here.
            state.sensor = None;
            // **16b cut D.4** (`B-22`): and its receiver goes with the sight. The
            // sensor pass is the only writer of the field, and the retain below
            // takes this unit's pass off the heap for good, so a wreck that was
            // degraded when it died would have carried
            // `receiver degraded: 15 of 40 through cover` on `STATUS` for the rest
            // of the match — hashed state that never converged. It is turned off
            // **silently**, with no ledger line and no notice: a wreck's receiver
            // is off rather than clear, which is the rule `Arena::sensor_pass`
            // applies to a pass that does not run.
            state.receiver = None;
        }
        self.contacts.clear_viewer(target);
        self.due.retain(|kind| match kind {
            DueKind::SensorPass { unit, .. }
            | DueKind::Share { unit }
            | DueKind::StartupComplete { unit }
            | DueKind::WeaponReady { unit, .. } => *unit != target,
            // **16b cut N.1**: the wreck's own reflex tick goes with the rest of
            // its scheduled work, and its machine is put into `Disabled` here
            // rather than on a tick that is never going to come — doc 16 §6's
            // `Disabled` state *is* a wreck, so the distribution a report prints
            // has to agree with the field.
            DueKind::ReflexTick { unit, .. } => *unit != target,
            // **16b cut M.2**: the duration bound is the match's, and a wreck's
            // held losses are still said to whoever sits in it — the sweep this
            // event replaced said them too.
            DueKind::Impact { .. }
            | DueKind::PositionSummary
            | DueKind::MatchDeadline
            | DueKind::LossSettles { .. } => true,
            // **16b cut O.1**: a wreck cancels nothing of the hold's. The clock
            // is a side's, not a unit's, and the arm re-reads presence at the pop
            // (`Arena::hold_presence`), so a holder that became this wreck is
            // simply not there when its event comes round.
            DueKind::HoldSettles { .. } => true,
        });
        if self
            .unit(target)
            .is_some_and(|state| state.controller == Controller::Reflex)
        {
            self.reflex
                .adopt(target, crate::arena::reflex::ReflexState::Disabled);
        }
        journal.push((
            t_ms,
            EventKind::UnitDestroyed { unit: target, by },
            Vec::new(),
        ));
    }

    /// **A unit with no weapons and no motion is disabled** (doc 16 §3.8; doc 10
    /// 3A: *disabled precedes destroyed*).
    ///
    /// Said once, on the transition, and never for a wreck: a wreck is already past
    /// disabled and doc 10's ordering is about which word a unit is described by,
    /// not about journalling both.
    pub fn settle_disabled(&mut self, target: ObjId, t_ms: i64, journal: &mut Vec<Notice>) {
        let disable = self.unit(target).is_some_and(|state| {
            !state.disabled
                && !state.limits.wreck
                && state.weapons.is_empty()
                && state.limits.motion_disabled
        });
        if !disable {
            return;
        }
        if let Some(state) = self.unit_mut(target) {
            state.disabled = true;
        }
        journal.push((t_ms, EventKind::UnitDisabled { unit: target }, Vec::new()));
    }

    /// **Whether a unit is out of the fight** (doc 16 §3.8's *a wreck or disabled
    /// with no weapons*).
    #[must_use]
    pub fn out_of_action(&self, unit: ObjId) -> bool {
        self.unit(unit)
            .is_some_and(|state| state.limits.wreck || state.disabled)
    }

    /// **Which side has been eliminated** (doc 16 §3.8: *team elimination — every
    /// unit of a side a wreck or disabled with no weapons — ends the match*).
    ///
    /// `Some(Some(team))` is the **survivor**; `Some(None)` is a draw, both sides
    /// gone at once, which one shot can do when the last two units destroy each
    /// other in one substep. `None` is a match still worth running, which includes
    /// a side with no units at all: an empty side is `SIM START`'s refusal and the
    /// lobby's business, not a victory condition.
    #[must_use]
    pub fn eliminated(&self) -> Option<Option<Team>> {
        let mut standing: Vec<Team> = Vec::new();
        let mut present: Vec<Team> = Vec::new();
        for unit in &self.units {
            if !present.contains(&unit.team) {
                present.push(unit.team);
            }
            if !self.out_of_action(unit.id) && !standing.contains(&unit.team) {
                standing.push(unit.team);
            }
        }
        if present.len() < 2 {
            return None;
        }
        match standing.len() {
            0 => Some(None),
            1 => standing.first().copied().map(Some),
            _ => None,
        }
    }

    /// **End the match on an elimination**, if one has happened (doc 16 §3.8).
    ///
    /// Answers the end it recorded, so phase 5 can journal it against the hash it
    /// already has. A match that has already ended is left alone.
    pub fn settle_elimination(&mut self) -> Option<MatchEnd> {
        if self.ended.is_some() {
            return None;
        }
        let winner = self.eliminated()?;
        let end = MatchEnd {
            reason: EndReason::Elimination { winner },
            t_ms: self.now_ms,
        };
        self.ended = Some(end);
        Some(end)
    }

    /// **Whether nobody left on either side can fire a round** (16b cut Q.7) —
    /// the predicate behind [`EndReason::Spent`].
    ///
    /// Four conditions, and each of them is load-bearing:
    ///
    /// 1. **both sides are present**, the same test [`Arena::eliminated`] makes,
    ///    because a one-sided window is the lobby's business and never a result;
    /// 2. **each side brought a gun and still has the platform that carried it.**
    ///    *Out of ammunition* presumes ammunition: a side of unarmed units — every
    ///    cut-A fixture's target, and the harness's movers — has not run dry, it
    ///    never had a gun, and calling that a draw would be a sentence that is not
    ///    true. The test is [`UnitState::armed_at_checkout`], which survives a
    ///    turret being shot off because `destroy_unit` and `remove_weapons`
    ///    deliberately leave the rounds where they are (16b cut F.1);
    /// 3. **no unit still in action has a weapon row holding rounds.** Rows
    ///    emptied by `remove_weapons` (doc 16 §3.3, a turret shot off) count as
    ///    none, which is the honest reading: that tank cannot shoot either;
    /// 4. **nothing is in flight.** A round already on its way can still decide
    ///    the match, and ending before it lands would be calling a draw over a
    ///    kill.
    #[must_use]
    pub fn spent(&self) -> bool {
        let mut present: Vec<Team> = Vec::new();
        let mut armed: Vec<Team> = Vec::new();
        for unit in &self.units {
            if !present.contains(&unit.team) {
                present.push(unit.team);
            }
            if self.out_of_action(unit.id) {
                continue;
            }
            if unit.armed_at_checkout() && !armed.contains(&unit.team) {
                armed.push(unit.team);
            }
            if unit
                .weapons
                .iter()
                .any(|weapon| unit.weapon_ammo(&weapon.id) > 0)
            {
                return false;
            }
        }
        if present.len() < 2 || armed.len() < 2 {
            return false;
        }
        !self
            .due
            .iter()
            .any(|event| matches!(event.kind, DueKind::Impact { .. }))
    }

    /// **End the match because nothing can shoot any more** (16b cut Q.7), if
    /// [`Arena::spent`] says so.
    ///
    /// Called beside [`Arena::settle_elimination`] and never before it: a side
    /// eliminated on the same substep is an elimination with a winner, and a draw
    /// over the top of it would lose the result.
    pub fn settle_spent(&mut self) -> Option<MatchEnd> {
        if self.ended.is_some() || !self.spent() {
            return None;
        }
        let end = MatchEnd {
            reason: EndReason::Spent,
            t_ms: self.now_ms,
        };
        self.ended = Some(end);
        Some(end)
    }

    /// **Which side stands alone on the key spot** (16b cut O.1; 16c N11), read
    /// over the poses as they stand.
    ///
    /// `None` on a match with no [`crate::arena::state::HoldState`] — which
    /// returns before any walk, so a `Skirmish` match pays one `Option` test —
    /// and `None` with nobody on the spot or with both sides on it. `Some(team)`
    /// when exactly one side has a unit on the spot that is **in action** (not
    /// [`Arena::out_of_action`]: a wreck or a disabled hulk holds nothing, doc 16
    /// §3.8's own reading of *out of the fight*) and whose cell is on the window
    /// and inside [`crate::arena::state::HoldState::square_contains`].
    ///
    /// One Chebyshev comparison per unit, in unit-id order, with no allocation.
    #[must_use]
    pub fn hold_presence(&self) -> Option<Team> {
        let hold = self.hold.as_ref()?;
        let mut alone: Option<Team> = None;
        for unit in &self.units {
            if unit.limits.wreck || unit.disabled {
                continue;
            }
            let Some(cell) = self.definition.terrain.cell_of(&unit.pose) else {
                continue;
            };
            if !hold.square_contains(cell) {
                continue;
            }
            match alone {
                None => alone = Some(unit.team),
                Some(team) if team == unit.team => {}
                // Both sides on it: contested, and nothing further can change
                // that answer.
                Some(_) => return None,
            }
        }
        alone
    }

    /// **Settle who holds the key spot, at a substep's end** (16b cut O.1; 16c
    /// N11; plan items 1, 15, 40), after the due loop and before the two
    /// settles.
    ///
    /// Three answers, against [`Arena::hold_presence`] over the positions this
    /// substep's motion committed and every impact its due loop applied:
    ///
    /// * **the same side still alone** — unchanged: the clock that is running
    ///   keeps running, and its event is still on the heap;
    /// * **a side newly alone** — `held_by = Some((team, now_ms))` and one
    ///   [`DueKind::HoldSettles`] at `now_ms + hold_ms`, exactly as
    ///   [`DueKind::MatchDeadline`] is scheduled at the bound, so the win is
    ///   settled at its own millisecond whatever the cadence (`now_ms` is a
    ///   substep boundary and the shipped substep divides every shipped period;
    ///   `hold_ms` is whole seconds);
    /// * **nobody, or both** — `held_by = None`.
    ///
    /// **[`crate::arena::state::DueHeap::retain`] exists and is deliberately not
    /// used here.** It is a `BTreeSet` rebuild, and an occupancy flipping at the
    /// edge of the square would rebuild the whole schedule once per flip. The
    /// event is left in the heap instead: `Arena::substep`'s arm pops it at its
    /// own millisecond and drops it, because `held_by` no longer names that
    /// `(team, since_ms)`. The cost is bounded — one event per *opening*, each
    /// popped and dropped at its own millisecond, so at most as many as there
    /// were openings in the last `hold_ms` — and it is the rule
    /// [`DueKind::LossSettles`] already lives by.
    ///
    /// Nothing is journalled: `held_by` is folded into the state hash (item 12),
    /// which is how a replay that opened the clock a substep earlier is caught
    /// at the substep it happened, and it reaches no pilot (plan item 10).
    pub fn settle_hold_presence(&mut self) {
        let Some(held_by) = self.hold.as_ref().map(|hold| hold.held_by) else {
            return;
        };
        let presence = self.hold_presence();
        let now_ms = self.now_ms;
        let (next, schedule) = match (held_by, presence) {
            (Some((holder, since)), Some(team)) if holder == team => (Some((holder, since)), None),
            (_, Some(team)) => (Some((team, now_ms)), Some(team)),
            (_, None) => (None, None),
        };
        let hold_ms = self.hold.as_ref().map_or(0, |hold| hold.hold_ms);
        if let Some(hold) = self.hold.as_mut() {
            hold.held_by = next;
        }
        if let Some(team) = schedule {
            self.due.schedule(
                now_ms.saturating_add(i64::from(hold_ms)),
                DueKind::HoldSettles {
                    team,
                    since_ms: now_ms,
                },
            );
        }
    }

    /// **Every unit as the check-in takes it back** (doc 16 §3.1 "Custody",
    /// check-in; 16b cut W.2 fills the seam T.3 left), in unit-id order.
    ///
    /// The arena's own live weapon cells are folded into the M32 unit first, so the
    /// returned cells are the whole of what the match changed rather than only the
    /// damage: W.1 keeps a weapon's rounds in the arena's own table while a match
    /// runs (it is written every pulse and the arena is its single writer), and this
    /// is where that table goes home. `ready_at_ms` is deliberately **not** returned
    /// — it is a time in *arena* milliseconds and means nothing outside the match,
    /// and a weapon whose cycle was running when the shooting stopped is out of it
    /// by the time anybody sees the tank again.
    ///
    /// A fold M32 refuses leaves the unit exactly as the damage left it, which is
    /// still a correct return: the ammunition is the one thing that is then stale,
    /// and doc 16 §3.3 resets every stock tank at the next `Idle` anyway.
    #[must_use]
    pub fn returned_units(&self) -> Vec<UnitReturn> {
        let Ok(cell) = mush_tactics::CellId::new(crate::definitions::unit::AMMO_CELL) else {
            return Vec::new();
        };
        self.units
            .iter()
            .map(|state| {
                let mut unit = state.unit.clone();
                let ops: Vec<UnitMutationOp> = state
                    .ammo
                    .iter()
                    .filter_map(|(weapon, rounds)| {
                        let addr = CellAddr::Component {
                            component: mush_tactics::ComponentId::new(weapon.as_str()).ok()?,
                            cell: cell.clone(),
                        };
                        let held = unit.cell(&addr)?;
                        let value = i64::from(*rounds).clamp(held.min, held.max);
                        (value != held.current).then_some(UnitMutationOp::SetCell { addr, value })
                    })
                    .collect();
                if !ops.is_empty()
                    && let Ok(plan) =
                        UnitMutationPlan::new(ops, None, Some(SmolStr::new_static("arena:rounds")))
                    && let Ok((post, _)) = mush_tactics::apply(&unit, &plan, self.now_ms)
                {
                    unit = post;
                }
                UnitReturn {
                    unit: state.id,
                    cells: unit,
                    pose: state.pose.clone(),
                    shots: state.shots_fired,
                    hits: state.hits,
                    wreck: state.limits.wreck,
                    disabled: state.disabled,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::Team;
    use crate::arena::test_support::{arena_of, line_tank, line_tank_unit, pose};

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);

    fn duel() -> Arena {
        let mut arena = arena_of(vec![
            line_tank(RED, Team::Red, pose(1_500, 1_500)),
            line_tank(BLUE, Team::Blue, pose(1_500, 2_500)),
        ]);
        for id in [RED, BLUE] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        arena
    }

    /// The cell value at one address.
    fn cell(arena: &Arena, unit: ObjId, addr: &CellAddr) -> Option<i64> {
        arena
            .unit(unit)?
            .unit
            .cell(addr)
            .map(|scalar| scalar.current)
    }

    fn armor(section: &str, facet: &str) -> CellAddr {
        CellAddr::Facet {
            section: SectionId::new(section).expect("a section"),
            facet: mush_tactics::FacetId::new(facet).expect("a facet"),
            cell: mush_tactics::CellId::new(ARMOR_CELL).expect("a cell"),
        }
    }

    fn internals(section: &str) -> CellAddr {
        CellAddr::Section {
            section: SectionId::new(section).expect("a section"),
            cell: mush_tactics::CellId::new(INTERNALS_CELL).expect("a cell"),
        }
    }

    #[test]
    fn the_facet_draw_lands_on_a_section_the_band_declares() {
        let arena = duel();
        for seq in 1..40_u64 {
            let (section, value) = arena
                .choose_section(BLUE, "front", seq)
                .unwrap_or_else(|| panic!("the front band has sections"));
            assert!(
                ["hull", "turret"].contains(&section.as_str()),
                "the front band sends rounds to hull and turret, not {section}"
            );
            // 16b cut M.3: and the value it answers is the draw itself, whole.
            assert_eq!(value, draw(arena.seed, &DrawLabel::Facet { shot_seq: seq }));
        }
        assert_eq!(arena.choose_section(BLUE, "nowhere", 1), None);
    }

    #[test]
    fn armour_floors_before_the_overflow_reaches_the_internals() {
        let mut arena = duel();
        // The turret's front facet is 350 and its internals 150.
        let (struck, ops) = arena
            .plan_hit(BLUE, "turret", "front", 400)
            .unwrap_or_else(|| panic!("a turret hit plans"));
        assert_eq!(struck.armor_left, 0, "350 of armour is spent");
        assert_eq!(struck.internals_left, 100, "50 overflows");
        assert_eq!(ops.len(), 2, "one cell each, and no consequence yet");
        // And the plan really lands those two numbers.
        let mut journal: Vec<Notice> = Vec::new();
        let shot = crate::arena::weapons::Shot {
            seq: 1,
            shooter: RED,
            weapon: SmolStr::new("cannon"),
            contact: crate::arena::state::ContactId(1),
            target: BLUE,
            rounds: 1,
            fired_at_ms: 0,
            impact_at_ms: 0,
            range_cm: 1_000,
            aim_x_cm: 1_500,
            aim_y_cm: 2_500,
            aim_cell: Some((1, 2)),
            damage: 400,
            resolution: Resolution::blind("front", 3_000, Cover::None),
        };
        arena.apply_hit(
            &shot,
            &Resolution {
                p_permille: 1_000,
                ..Resolution::blind("front", 3_000, Cover::None)
            },
            1,
            10,
            arena.shot_draw(1, 1),
            &mut journal,
        );
        let draws = journal
            .iter()
            .find(|(_, kind, _)| matches!(kind, EventKind::Hit { .. }))
            .map(|(_, _, draws)| draws.clone())
            .unwrap_or_else(|| panic!("the hit is journalled: {journal:?}"));
        // **16b cut M.3** (16c `T-07`): both draws the round made, in the order
        // it made them — the shot's own, then the facet's.
        let labels: Vec<&str> = draws.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, vec!["hit:1", "facet:1"]);
        assert_eq!(
            draws.get(1).map(|(_, value)| *value),
            Some(draw(arena.seed, &DrawLabel::Facet { shot_seq: 1 }))
        );
    }

    /// **16b cut M.3** (16c `T-07`): a burst's `hit` line carries `burst:<seq>`
    /// and then `facet:<seq>` — the same two-draw shape, keyed on the burst.
    #[test]
    fn m3_a_burst_hit_carries_the_burst_draw_then_the_facet_draw() {
        let mut arena = duel();
        let mut journal: Vec<Notice> = Vec::new();
        let shot = crate::arena::weapons::Shot {
            seq: 5,
            shooter: RED,
            weapon: SmolStr::new("cannon"),
            contact: crate::arena::state::ContactId(1),
            target: BLUE,
            rounds: 3,
            fired_at_ms: 0,
            impact_at_ms: 0,
            range_cm: 1_000,
            aim_x_cm: 1_500,
            aim_y_cm: 2_500,
            aim_cell: Some((1, 2)),
            damage: 10,
            resolution: Resolution::blind("front", 3_000, Cover::None),
        };
        let drawn = arena.shot_draw(5, 3);
        assert_eq!(
            drawn.0, "burst:5",
            "a burst's shot draw is keyed on the burst"
        );
        arena.apply_hit(
            &shot,
            &Resolution {
                p_permille: 1_000,
                ..Resolution::blind("front", 3_000, Cover::None)
            },
            2,
            10,
            drawn,
            &mut journal,
        );
        let labels: Vec<String> = journal
            .iter()
            .find(|(_, kind, _)| matches!(kind, EventKind::Hit { .. }))
            .map(|(_, _, draws)| draws.iter().map(|(label, _)| label.clone()).collect())
            .unwrap_or_else(|| panic!("the hit is journalled: {journal:?}"));
        assert_eq!(labels, vec!["burst:5".to_owned(), "facet:5".to_owned()]);
    }

    #[test]
    fn a_hull_through_its_internals_is_a_wreck_in_one_plan() {
        let arena = duel();
        // 400 of hull front armour and 200 of internals: 600 exactly.
        let (struck, ops) = arena
            .plan_hit(BLUE, "hull", "front", 600)
            .unwrap_or_else(|| panic!("a hull hit plans"));
        assert_eq!((struck.armor_left, struck.internals_left), (0, 0));
        assert_eq!(struck.consequences, vec![Consequence::DestroyUnit]);
        // Two cells, the section's own condition, the wreck condition and the
        // power capability the stock tank holds.
        assert!(
            ops.iter()
                .any(|op| matches!(op, UnitMutationOp::RevokeCapability { key } if key.id.as_str() == POWER_CAPABILITY)),
            "the wreck loses its power capability: {ops:?}"
        );
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, UnitMutationOp::SetSectionCondition { .. }))
                .count(),
            2,
            "destroyed, then wreck"
        );
    }

    #[test]
    fn the_engine_is_struck_through_the_hulls_rear_and_disables_motion() {
        let mut arena = duel();
        let (struck, _) = arena
            .plan_hit(BLUE, "engine", "rear", 1_000)
            .unwrap_or_else(|| panic!("an engine hit plans"));
        assert_eq!(struck.section, "engine");
        assert_eq!(
            struck.facet, "rear",
            "the hull's rear, not the engine's own"
        );
        // **16b cut M.3** (16c `T-13`): and the line names whose armour that
        // facet is — the hull's, which the engine is struck through — so the
        // `rear` beside `engine` is never read as the engine's own.
        assert_eq!(struck.armor_section, "hull");
        // A section with armour of its own names itself.
        let (turret, _) = arena
            .plan_hit(BLUE, "turret", "front", 10)
            .unwrap_or_else(|| panic!("a turret hit plans"));
        assert_eq!(
            (turret.section.as_str(), turret.armor_section.as_str()),
            ("turret", "turret")
        );
        assert_eq!(struck.internals_left, 0);
        assert_eq!(struck.consequences, vec![Consequence::DisableMotion]);
        // The hull's rear armour is what was spent, and the engine's internals.
        let mut journal: Vec<Notice> = Vec::new();
        arena.apply_consequence(BLUE, &Consequence::DisableMotion, 0, None, &mut journal);
        assert!(
            arena
                .unit(BLUE)
                .is_some_and(|state| state.limits.motion_disabled)
        );
    }

    #[test]
    fn a_track_slows_the_tank_and_shuts_one_way_of_turning() {
        let mut arena = duel();
        let mut journal: Vec<Notice> = Vec::new();
        arena.apply_consequence(
            BLUE,
            &Consequence::Slow { percent: 50 },
            0,
            None,
            &mut journal,
        );
        arena.apply_consequence(
            BLUE,
            &Consequence::NoTurnToward {
                side: crate::arena::state::Side::Left,
            },
            0,
            None,
            &mut journal,
        );
        let state = arena.unit(BLUE).unwrap_or_else(|| panic!("blue"));
        assert_eq!(state.limits.slow_pct, 50);
        // 16b cut M.4 (`B-47`): the side is a flag of its own now.
        assert_eq!(
            (state.limits.no_turn_left, state.limits.no_turn_right),
            (true, false)
        );
        // Idempotent: the same consequence again does not compound.
        arena.apply_consequence(
            BLUE,
            &Consequence::Slow { percent: 50 },
            0,
            None,
            &mut journal,
        );
        assert_eq!(
            arena.unit(BLUE).map(|state| state.limits.slow_pct),
            Some(50)
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut F.1: what a unit's own record says at check-out
    // -----------------------------------------------------------------------

    /// One plan against a fixture unit, applied.
    fn apply_ops(
        unit: &mush_tactics::TacticalUnit,
        ops: Vec<UnitMutationOp>,
    ) -> mush_tactics::TacticalUnit {
        let plan = UnitMutationPlan::new(ops, None, Some(SmolStr::new_static("test")))
            .unwrap_or_else(|_| panic!("the fixture plan fits"));
        let (post, _) = mush_tactics::apply(unit, &plan, 0)
            .unwrap_or_else(|error| panic!("the fixture plan applies: {error}"));
        post
    }

    /// One record checked out into a duel, and the arena it makes of it.
    fn enter_with(record: mush_tactics::TacticalUnit) -> Arena {
        let mut custody = line_tank(BLUE, Team::Blue, pose(1_500, 2_500));
        custody.unit = record;
        arena_of(vec![line_tank(RED, Team::Red, pose(1_500, 1_500)), custody])
    }

    /// What a record becomes when the match opens on it.
    fn enter(record: mush_tactics::TacticalUnit) -> UnitState {
        enter_with(record)
            .unit(BLUE)
            .cloned()
            .unwrap_or_else(|| panic!("the mutated unit is in the arena"))
    }

    /// One plan against a fixture unit, applied and checked out.
    fn mutate(unit: &mush_tactics::TacticalUnit, ops: Vec<UnitMutationOp>) -> UnitState {
        enter(apply_ops(unit, ops))
    }

    #[test]
    fn a_unit_whose_record_says_wreck_enters_the_match_as_one() {
        let fixture = line_tank_unit(BLUE);
        let state = mutate(
            &fixture,
            vec![UnitMutationOp::SetSectionCondition {
                section: SectionId::new(HULL_SECTION).expect("a section"),
                condition: ConditionId::new(WRECK_CONDITION).expect("a condition"),
            }],
        );
        // Before this cut every match opened at `MotionLimits::default()`, so the
        // hulk drove, saw and shot — and could never be wrecked again, because
        // `plan_hit` fires a section's consequences only on the transition to
        // zero.
        assert!(
            state.limits.wreck,
            "a wrecked record entered as a live tank"
        );
        assert!(state.limits.motion_disabled);
        assert!(state.weapons.is_empty(), "a wreck carries no weapon");
        assert!(state.sensor.is_none(), "and sees nothing");
        assert!(!state.disabled, "a wreck is past disabled, never both");
    }

    #[test]
    fn a_unit_whose_track_is_gone_enters_slowed_and_unable_to_turn_that_way() {
        let fixture = line_tank_unit(BLUE);
        let state = mutate(
            &fixture,
            vec![UnitMutationOp::SetSectionCondition {
                section: SectionId::new("left_track").expect("a section"),
                condition: ConditionId::new(DESTROYED).expect("a condition"),
            }],
        );
        assert_eq!(state.limits.slow_pct, 50, "the track's `slow(50)`");
        assert_eq!(
            (state.limits.no_turn_left, state.limits.no_turn_right),
            (true, false),
            "and its `no_turn_toward(left)`"
        );
        assert!(!state.limits.wreck && !state.limits.motion_disabled);
        assert_eq!(state.weapons.len(), 1, "it still has its gun");
    }

    /// **16b cut M.4** (16c `B-47`): a record with **both** tracks destroyed
    /// opens closed both ways — the second `no_turn_toward` is ORed in, not
    /// written over the first — and, through the stock tank's pair rule, with its
    /// motion disabled.
    #[test]
    fn m4_a_unit_whose_tracks_are_both_gone_enters_unable_to_turn_or_move() {
        let fixture = line_tank_unit(BLUE);
        let gone = |name: &str| UnitMutationOp::SetSectionCondition {
            section: SectionId::new(name).expect("a section"),
            condition: ConditionId::new(DESTROYED).expect("a condition"),
        };
        let state = mutate(&fixture, vec![gone("left_track"), gone("right_track")]);
        assert_eq!(
            (state.limits.no_turn_left, state.limits.no_turn_right),
            (true, true),
            "both sides closed, whichever track was read first"
        );
        assert!(
            state.limits.motion_disabled,
            "the pair rule: no tracks is no motion"
        );
        assert_eq!(state.limits.slow_pct, 50);
        assert!(!state.limits.wreck);
        assert_eq!(state.weapons.len(), 1, "it still has its gun");
        assert!(!state.disabled, "an armed tank is not disabled");
    }

    /// **16b cut M.4** (16c `B-47`): the two track losses through
    /// `apply_consequence`, in either order, leave one state — both sides closed
    /// and, once the second section is actually destroyed, motion disabled by
    /// the pair rule.
    #[test]
    fn m4_two_track_losses_in_either_order_close_both_sides_and_stop_the_tank() {
        use crate::arena::state::Side;
        let pair = Consequence::DisableMotionWhenAllOf {
            sections: vec![SmolStr::new("left_track"), SmolStr::new("right_track")],
        };
        let mut outcomes = Vec::new();
        for order in [[Side::Left, Side::Right], [Side::Right, Side::Left]] {
            let mut arena = duel();
            let mut journal: Vec<Notice> = Vec::new();
            for side in order {
                let section = match side {
                    Side::Left => "left_track",
                    Side::Right => "right_track",
                };
                // The section's own condition first, as `apply_hit`'s plan sets
                // it, so the pair rule reads a fact about the unit.
                let plan = UnitMutationPlan::new(
                    vec![UnitMutationOp::SetSectionCondition {
                        section: SectionId::new(section).expect("a section"),
                        condition: ConditionId::new(DESTROYED).expect("a condition"),
                    }],
                    None,
                    None,
                )
                .expect("a plan");
                let post = arena
                    .unit(BLUE)
                    .map(|state| mush_tactics::apply(&state.unit, &plan, 0))
                    .and_then(Result::ok)
                    .map(|(post, _)| post)
                    .unwrap_or_else(|| panic!("the plan applies"));
                arena.set_unit(BLUE, post, false);
                arena.apply_consequence(
                    BLUE,
                    &Consequence::NoTurnToward { side },
                    0,
                    None,
                    &mut journal,
                );
                arena.apply_consequence(BLUE, &pair, 0, None, &mut journal);
            }
            let limits = arena
                .unit(BLUE)
                .map(|state| state.limits)
                .unwrap_or_else(|| panic!("blue"));
            outcomes.push(limits);
        }
        assert_eq!(outcomes.first(), outcomes.get(1), "one state either way");
        let limits = outcomes.first().copied().unwrap_or_default();
        assert!(limits.no_turn_left && limits.no_turn_right);
        assert!(limits.motion_disabled);
    }

    /// **16b cut M.4** (16c `B-47`): the stock tank's two tracks shot off by
    /// rounds. The first round leaves the tank slowed and closed one way, and its
    /// pilot's list does **not** carry the pair rule (it did not fire); the
    /// second fires `disable_motion_when_all_of`, and the list the `SectionLost`
    /// notice reads (`fired_consequences`) names it.
    #[test]
    fn m4_the_second_track_shot_off_fires_the_pair_rule() {
        let mut arena = duel();
        // Every band strikes one track, so the facet draw cannot choose.
        let only = |section: &str| vec![(SmolStr::new(section), 1_u16)];
        let mut journal: Vec<Notice> = Vec::new();
        for (seq, section) in [(1_u64, "left_track"), (2, "right_track")] {
            if let Some(state) = arena.unit_mut(BLUE) {
                for band in &mut state.profile.bands {
                    band.sections = only(section);
                }
            }
            let shot = crate::arena::weapons::Shot {
                seq,
                shooter: RED,
                weapon: SmolStr::new("cannon"),
                contact: crate::arena::state::ContactId(1),
                target: BLUE,
                rounds: 1,
                fired_at_ms: 0,
                impact_at_ms: 0,
                range_cm: 1_000,
                aim_x_cm: 1_500,
                aim_y_cm: 2_500,
                aim_cell: Some((1, 2)),
                damage: 1_000,
                resolution: Resolution::blind("left", 3_000, Cover::None),
            };
            let taken = Resolution {
                p_permille: 1_000,
                ..Resolution::blind("left", 3_000, Cover::None)
            };
            arena.apply_hit(&shot, &taken, 1, 10, arena.shot_draw(seq, 1), &mut journal);
            let fired: Vec<SmolStr> = arena.fired_consequences(BLUE, section);
            let limits = arena
                .unit(BLUE)
                .map(|state| state.limits)
                .unwrap_or_else(|| panic!("blue"));
            if seq == 1 {
                assert_eq!(fired, vec!["slow", "no_turn_toward"], "one track gone");
                assert!(limits.no_turn_left && !limits.no_turn_right);
                assert!(!limits.motion_disabled, "one track still drives");
            } else {
                assert_eq!(
                    fired,
                    vec!["slow", "no_turn_toward", "disable_motion_when_all_of"],
                    "the second track's notice names the pair rule"
                );
                assert!(limits.no_turn_left && limits.no_turn_right);
                assert!(limits.motion_disabled, "no tracks, no motion");
            }
        }
        // Both `section_destroyed` lines carry the whole authored list.
        let destroyed = journal
            .iter()
            .filter(|(_, kind, _)| matches!(kind, EventKind::SectionDestroyed { .. }))
            .count();
        assert_eq!(destroyed, 2);
    }

    #[test]
    fn a_unit_whose_turret_is_gone_enters_disarmed_and_a_dead_engine_makes_it_disabled() {
        let fixture = line_tank_unit(BLUE);
        let gone = |name: &str| UnitMutationOp::SetSectionCondition {
            section: SectionId::new(name).expect("a section"),
            condition: ConditionId::new(DESTROYED).expect("a condition"),
        };
        let state = mutate(&fixture, vec![gone("turret")]);
        assert!(state.weapons.is_empty(), "the turret took the gun with it");
        assert!(!state.disabled, "it can still drive");
        let state = mutate(&fixture, vec![gone("turret"), gone("engine")]);
        assert!(
            state.limits.motion_disabled,
            "the engine's `disable_motion`"
        );
        assert!(
            state.disabled,
            "no weapons and no motion is doc 16 §3.8's disabled"
        );
    }

    #[test]
    fn a_unit_enters_with_the_rounds_its_own_record_carries() {
        let fixture = line_tank_unit(BLUE);
        let addr = CellAddr::Component {
            component: mush_tactics::ComponentId::new("cannon").expect("a component"),
            cell: mush_tactics::CellId::new(crate::definitions::unit::AMMO_CELL).expect("a cell"),
        };
        let state = mutate(&fixture, vec![UnitMutationOp::SetCell { addr, value: 31 }]);
        assert_eq!(
            state.ammo.get("cannon").copied(),
            Some(31),
            "a tank that enters with 31 rounds used to be handed the catalog's 40"
        );
    }

    /// **A tank wrecked in match one, reset, can be wrecked again in match two**
    /// (16b cut F.1; doc 16 §3.3) — the whole reason the cut exists, end to end
    /// in one function and with no wall clock in it.
    ///
    /// The readiness review's symptom was the second half of this: a hull carried
    /// into match two at zero internals absorbed fourteen rounds without the match
    /// ending, because [`Arena::plan_hit`] fires a section's consequences only on
    /// the **transition** to zero. Nothing bound "the reset makes the record equal
    /// to a fresh instantiation" to "and therefore the destroy consequence fires
    /// again"; this is that binding, and the wrecked leg is the control that shows
    /// the difference is the reset's and not the fixture's.
    #[test]
    fn a_tank_wrecked_and_then_reset_can_be_wrecked_again() {
        let spec = line_tank_unit(BLUE);
        // (1) Match one: the hull through its internals, by the plan the arena
        //     itself builds for that hit, committed to the record the way a
        //     check-in commits one.
        let (_, ops) = duel()
            .plan_hit(BLUE, "hull", "front", 600)
            .unwrap_or_else(|| panic!("a hull hit plans"));
        let wrecked = apply_ops(&spec, ops);
        assert!(is_wreck(&wrecked), "match one did not leave a wreck");

        // (2) Match two on the UNRESET record: it enters as a wreck, and the very
        //     same hit now carries no consequence at all — the transition it
        //     needs has already happened.
        let held = enter_with(wrecked.clone());
        assert!(
            held.unit(BLUE).is_some_and(|state| state.limits.wreck),
            "a wrecked record entered as a live tank"
        );
        let (struck, _) = held
            .plan_hit(BLUE, "hull", "front", 600)
            .unwrap_or_else(|| panic!("a hull hit plans"));
        assert!(
            struck.consequences.is_empty(),
            "a spent hull cannot be destroyed a second time — the defect: {:?}",
            struck.consequences
        );

        // (3) The reset, as `lobby::reset` performs it: one plan back to the
        //     unit's own spec.
        let plan = crate::match_::spec_plan(&wrecked, &spec)
            .unwrap_or_else(|| panic!("the reset had nothing to do on a wreck"));
        let after = apply_ops(&wrecked, plan.ops().to_vec());
        assert_eq!(
            crate::match_::spec_plan(&after, &spec),
            None,
            "the reset left something to do"
        );

        // (4) And match two on the reset record is a match: it opens whole, and
        //     the same hit destroys it again.
        let state = enter(after.clone());
        assert_eq!(
            state.limits,
            crate::arena::state::MotionLimits::default(),
            "the reset tank did not open whole"
        );
        assert!(!state.weapons.is_empty(), "and it carries its gun again");
        let mut arena = enter_with(after.clone());
        // **A hit in match two moves armour.** The reset put the hull's front
        // facet back at its initial value, so the round has armour to spend — the
        // other half of the readiness review's symptom, where the second match's
        // rounds landed on cells already at their floor and journalled nothing.
        let front = armor("hull", "front");
        let before = after
            .cell(&front)
            .map(|scalar| scalar.current)
            .unwrap_or_else(|| panic!("the reset tank has hull front armour"));
        assert!(before > 0, "the reset left the hull's armour spent");
        let (struck, ops) = arena
            .plan_hit(BLUE, "hull", "front", 600)
            .unwrap_or_else(|| panic!("a hull hit plans"));
        assert!(
            ops.iter().any(
                |op| matches!(op, UnitMutationOp::SetCell { addr, value } if *addr == front && *value < before)
            ),
            "the hit moved no armour: {ops:?}"
        );
        assert_eq!(struck.armor_left, 0, "and spent all of it");
        assert_eq!(
            struck.consequences,
            vec![Consequence::DestroyUnit],
            "the reset tank cannot be killed again"
        );
        // And the consequence really lands.
        let mut journal: Vec<Notice> = Vec::new();
        arena.apply_consequence(BLUE, &Consequence::DestroyUnit, 0, None, &mut journal);
        assert!(
            arena.unit(BLUE).is_some_and(|state| state.limits.wreck),
            "the destroy consequence did not wreck it"
        );
        assert_eq!(arena.eliminated(), Some(Some(Team::Red)));
    }

    #[test]
    fn a_turret_takes_its_weapons_with_it_and_an_immobile_disarmed_tank_is_disabled() {
        let mut arena = duel();
        let mut journal: Vec<Notice> = Vec::new();
        assert_eq!(
            arena.unit(BLUE).map(|state| state.weapons.len()),
            Some(1),
            "the fixture's one turret weapon"
        );
        arena.apply_consequence(
            BLUE,
            &Consequence::RemoveWeapons {
                section: SmolStr::new("turret"),
            },
            0,
            None,
            &mut journal,
        );
        let state = arena.unit(BLUE).unwrap_or_else(|| panic!("blue"));
        assert!(state.weapons.is_empty() && state.ready_at_ms.is_empty());
        // **16b cut F.1**: the ROUNDS stay. `ammo` is the arena's record of what
        // that barrel has left and `returned_units` is what carries it home to the
        // unit's own cell — a turret shot off at round twelve used to check in
        // with a full magazine.
        assert!(
            !state.ammo.is_empty(),
            "the rounds the turret had left are still on the record"
        );
        // Not disabled yet: it can still drive.
        arena.settle_disabled(BLUE, 0, &mut journal);
        assert_eq!(arena.unit(BLUE).map(|state| state.disabled), Some(false));
        arena.apply_consequence(BLUE, &Consequence::DisableMotion, 0, None, &mut journal);
        arena.settle_disabled(BLUE, 0, &mut journal);
        assert_eq!(arena.unit(BLUE).map(|state| state.disabled), Some(true));
        assert_eq!(
            journal
                .iter()
                .filter(|(_, kind, _)| matches!(kind, EventKind::UnitDisabled { .. }))
                .count(),
            1,
            "said once"
        );
        // And that is an elimination, because blue was the whole blue side.
        assert_eq!(arena.eliminated(), Some(Some(Team::Red)));
    }

    #[test]
    fn a_wreck_holds_nothing_keeps_nothing_scheduled_and_is_destroyed_once() {
        let mut arena = duel();
        // Give the wreck-to-be a contact of its own and something on the heap.
        let id = crate::arena::rng::draw_contact_id(arena.seed, BLUE, RED);
        arena.contacts.insert(
            BLUE,
            id,
            crate::arena::state::ShadowContact::sighted(
                RED,
                crate::arena::state::Classification::Unknown,
                0,
                // 16b cut F.3: where the red tank actually stands, which is what
                // every fixture that plants a contact by hand plants.
                arena
                    .unit(RED)
                    .map(crate::arena::state::SeenPose::of)
                    .unwrap_or_default(),
            ),
        );
        arena.due.schedule(
            1_000,
            DueKind::SensorPass {
                unit: BLUE,
                pass_seq: 1,
            },
        );
        let before = arena.due.len();
        let mut journal: Vec<Notice> = Vec::new();
        arena.destroy_unit(BLUE, 500, None, &mut journal);
        let state = arena.unit(BLUE).unwrap_or_else(|| panic!("the wreck"));
        assert!(state.limits.wreck && state.limits.motion_disabled);
        assert_eq!(state.power, Power::Off);
        assert!(state.weapons.is_empty() && state.lock.is_none());
        assert_eq!(arena.contacts.of(BLUE).count(), 0, "a wreck holds nothing");
        assert!(arena.due.len() < before, "its scheduled work went");
        assert_eq!(journal.len(), 1);
        // `destroy_unit` is the arena's half; the M32 mark is the impact plan's, so a
        // wreck made this way carries the arena's flag and not yet the condition —
        // which is what `is_wreck` reads, and why the two are tested apart.
        assert!(!is_wreck(&state.unit));
        let marked = mush_tactics::apply(
            &state.unit,
            &UnitMutationPlan::new(
                vec![UnitMutationOp::SetSectionCondition {
                    section: SectionId::new(HULL_SECTION).expect("a section"),
                    condition: ConditionId::new(WRECK_CONDITION).expect("a condition"),
                }],
                None,
                None,
            )
            .expect("a one-op plan"),
            0,
        )
        .map(|(post, _)| post)
        .expect("the mark applies");
        assert!(is_wreck(&marked), "the condition is the mark");
        // The other platform's contact on it is untouched: it expires the ordinary
        // way, which is what clears the lock (W.1's `lost` arm).
        journal.clear();
        arena.destroy_unit(BLUE, 600, None, &mut journal);
        assert!(journal.is_empty(), "destroyed once");
        assert_eq!(arena.eliminated(), Some(Some(Team::Red)));
    }

    #[test]
    fn a_returned_unit_carries_the_rounds_it_has_left() {
        let mut arena = duel();
        if let Some(state) = arena.unit_mut(RED) {
            state.ammo.insert(SmolStr::new("cannon"), 7);
            state.shots_fired = 33;
            state.hits = 11;
        }
        let returned = arena.returned_units();
        let back = returned
            .iter()
            .find(|back| back.unit == RED)
            .cloned()
            .unwrap_or_else(|| panic!("red came back"));
        let unit = back.cells;
        assert_eq!(
            (back.shots, back.hits, back.wreck, back.disabled),
            (33, 11, false, false)
        );
        let addr = CellAddr::Component {
            component: mush_tactics::ComponentId::new("cannon").expect("a component"),
            cell: mush_tactics::CellId::new(crate::definitions::unit::AMMO_CELL).expect("a cell"),
        };
        assert_eq!(
            unit.cell(&addr).map(|scalar| scalar.current),
            Some(7),
            "the rounds went home"
        );
        assert_eq!(
            cell(&arena, RED, &armor("hull", "front")),
            Some(400),
            "and nothing else moved"
        );
        assert_eq!(cell(&arena, RED, &internals("hull")), Some(200));
    }
}
