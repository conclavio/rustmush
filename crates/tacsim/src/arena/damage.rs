//! **What a round does when it arrives** (16b cut W.2, W3; doc 16 §3.5 "Scheduled
//! impact" and "Damage", §3.3 "A wreck" and "Facet attribution", §3.8 "Team
//! elimination").
//!
//! One entry point — [`Arena::resolve_impact`], the body of the
//! [`DueKind::Impact`](crate::arena::state::DueKind::Impact) cut W.1 scheduled —
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
//! [`EventKind::Miss`](crate::arena::ledger::EventKind::Miss), and the set is the
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
use crate::arena::state::{Arena, DueKind, EndReason, MatchEnd, Power, Team, UnitState};
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
/// Exactly the [`WRECK_CONDITION`] on the [`HULL_SECTION`], and nothing else. The
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
#[must_use]
pub fn is_wreck(unit: &mush_tactics::TacticalUnit) -> bool {
    let Ok(hull) = SectionId::new(HULL_SECTION) else {
        return false;
    };
    unit.section(&hull)
        .is_some_and(|section| section.condition.as_str() == WRECK_CONDITION)
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
    /// How many of them landed.
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
    /// The damage the round or the burst carried.
    damage: u32,
    /// The armour left in that facet.
    armor_left: u32,
    /// The internals left in that section.
    internals_left: u32,
    /// The consequences that fired, in authored order; empty unless the section's
    /// internals reached zero on this hit.
    consequences: Vec<Consequence>,
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
    /// [`Arena::apply_hit`]: one plan, atomically.
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
            None,
        ));
        let miss = |reason: &str, journal: &mut Vec<Notice>| {
            journal.push((
                t_ms,
                EventKind::Miss {
                    shot_seq,
                    reason: SmolStr::new(reason),
                },
                None,
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
            if !sight.is_clear_or_partial() {
                miss(BLOCKED, journal);
                return;
            }
            if cover == Cover::Full {
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
                Some(drawn),
            ));
            return;
        }
        self.apply_hit(&shot, &taken, landed, t_ms, Some(drawn), journal);
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
        let (Some(from), Some(to)) = (self.unit(shooter), self.unit(target)) else {
            return (Sight::Clear, Cover::None);
        };
        let (Some(a), Some(b)) = (self.fire_eye(from), self.fire_eye(to)) else {
            return (Sight::Clear, Cover::None);
        };
        let trace = trace_cells(&self.definition.terrain, a, b);
        (trace.sight, trace.target_cover)
    }

    /// Just the cover half of [`Arena::fire_line`] — what the cockpit's percentage
    /// needs.
    #[must_use]
    pub fn fire_cover(&self, shooter: ObjId, target: ObjId) -> Cover {
        self.fire_line(shooter, target).1
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
    fn apply_hit(
        &mut self,
        shot: &crate::arena::weapons::Shot,
        taken: &Resolution,
        landed: u32,
        t_ms: i64,
        drawn: Option<(String, u64)>,
        journal: &mut Vec<Notice>,
    ) {
        let damage = shot.damage.saturating_mul(landed);
        let Some(section) = self.choose_section(shot.target, &taken.band, shot.seq) else {
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
                drawn,
            ));
            return;
        };
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
        if let Some(state) = self.unit_mut(shot.target) {
            state.unit = post;
        }
        if let Some(state) = self.unit_mut(shot.shooter) {
            state.hits = state.hits.saturating_add(1);
        }
        journal.push((
            t_ms,
            EventKind::Hit {
                shot_seq: shot.seq,
                unit: shot.shooter,
                target: shot.target,
                section: struck.section.clone(),
                facet: struck.facet.clone(),
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
        if struck.internals_left == 0 {
            journal.push((
                t_ms,
                EventKind::SectionDestroyed {
                    unit: shot.target,
                    section: struck.section.clone(),
                    consequences: struck
                        .consequences
                        .iter()
                        .map(|consequence| SmolStr::new_static(consequence.label()))
                        .collect(),
                },
                None,
            ));
            for consequence in &struck.consequences {
                self.apply_consequence(shot.target, consequence, t_ms, journal);
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
    /// weights are all zero, answers `None`.
    #[must_use]
    pub fn choose_section(&self, target: ObjId, band: &str, shot_seq: u64) -> Option<SmolStr> {
        let profile = &self.unit(target)?.profile;
        let rows = &profile.band(band)?.sections;
        let total: u64 = rows.iter().map(|(_, weight)| u64::from(*weight)).sum();
        if total == 0 {
            return None;
        }
        let mut pick = draw(self.seed, &DrawLabel::Facet { shot_seq }) % total;
        for (section, weight) in rows {
            let weight = u64::from(*weight);
            if pick < weight {
                return Some(section.clone());
            }
            pick = pick.saturating_sub(weight);
        }
        rows.last().map(|(section, _)| section.clone())
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
                damage: u32::try_from(damage).unwrap_or(u32::MAX),
                armor_left: u32::try_from(armor_left.max(0)).unwrap_or(0),
                internals_left: u32::try_from(internals_left.max(0)).unwrap_or(0),
                consequences,
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

    /// **Apply one consequence to the arena's own facts** (doc 16 §3.3's column).
    ///
    /// `destroy_unit` is [`Arena::destroy_unit`]; the other four are the
    /// [`MotionLimits`](crate::arena::state::MotionLimits) A.2's motion reads and
    /// the weapon table W.1's acceptance reads. Each is idempotent: a second
    /// `slow(50)` on an already-slowed tank leaves it at 50 rather than at 25,
    /// because the consequence names the surviving percentage and not a factor to
    /// compound.
    pub fn apply_consequence(
        &mut self,
        target: ObjId,
        consequence: &Consequence,
        t_ms: i64,
        journal: &mut Vec<Notice>,
    ) {
        match consequence {
            Consequence::DestroyUnit => self.destroy_unit(target, t_ms, journal),
            Consequence::Slow { percent } => {
                if let Some(state) = self.unit_mut(target) {
                    state.limits.slow_pct = state.limits.slow_pct.min((*percent).clamp(1, 100));
                }
            }
            Consequence::NoTurnToward { side } => {
                if let Some(state) = self.unit_mut(target) {
                    state.limits.no_turn_toward = Some(*side);
                }
            }
            Consequence::DisableMotion => {
                if let Some(state) = self.unit_mut(target) {
                    state.limits.motion_disabled = true;
                    state.speed_cms = 0;
                    state.desired_speed_cms = 0;
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
                        state.ammo.remove(&id);
                    }
                }
            }
        }
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
    pub fn destroy_unit(&mut self, target: ObjId, t_ms: i64, journal: &mut Vec<Notice>) {
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
            state.ammo.clear();
            state.sensor = None;
        }
        self.contacts.clear_viewer(target);
        self.due.retain(|kind| match kind {
            DueKind::SensorPass { unit, .. }
            | DueKind::Share { unit }
            | DueKind::StartupComplete { unit }
            | DueKind::WeaponReady { unit, .. } => *unit != target,
            DueKind::Impact { .. } | DueKind::PositionSummary => true,
        });
        journal.push((t_ms, EventKind::UnitDestroyed { unit: target }, None));
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
        journal.push((t_ms, EventKind::UnitDisabled { unit: target }, None));
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
    use crate::arena::test_support::{arena_of, line_tank, pose};

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
            let section = arena
                .choose_section(BLUE, "front", seq)
                .unwrap_or_else(|| panic!("the front band has sections"));
            assert!(
                ["hull", "turret"].contains(&section.as_str()),
                "the front band sends rounds to hull and turret, not {section}"
            );
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
            None,
            &mut journal,
        );
        assert!(
            journal
                .iter()
                .any(|(_, kind, _)| matches!(kind, EventKind::Hit { .. })),
            "the hit is journalled: {journal:?}"
        );
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
        assert_eq!(struck.internals_left, 0);
        assert_eq!(struck.consequences, vec![Consequence::DisableMotion]);
        // The hull's rear armour is what was spent, and the engine's internals.
        let mut journal: Vec<Notice> = Vec::new();
        arena.apply_consequence(BLUE, &Consequence::DisableMotion, 0, &mut journal);
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
        arena.apply_consequence(BLUE, &Consequence::Slow { percent: 50 }, 0, &mut journal);
        arena.apply_consequence(
            BLUE,
            &Consequence::NoTurnToward {
                side: crate::arena::state::Side::Left,
            },
            0,
            &mut journal,
        );
        let state = arena.unit(BLUE).unwrap_or_else(|| panic!("blue"));
        assert_eq!(state.limits.slow_pct, 50);
        assert_eq!(
            state.limits.no_turn_toward,
            Some(crate::arena::state::Side::Left)
        );
        // Idempotent: the same consequence again does not compound.
        arena.apply_consequence(BLUE, &Consequence::Slow { percent: 50 }, 0, &mut journal);
        assert_eq!(
            arena.unit(BLUE).map(|state| state.limits.slow_pct),
            Some(50)
        );
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
            &mut journal,
        );
        let state = arena.unit(BLUE).unwrap_or_else(|| panic!("blue"));
        assert!(state.weapons.is_empty() && state.ammo.is_empty());
        // Not disabled yet: it can still drive.
        arena.settle_disabled(BLUE, 0, &mut journal);
        assert_eq!(arena.unit(BLUE).map(|state| state.disabled), Some(false));
        arena.apply_consequence(BLUE, &Consequence::DisableMotion, 0, &mut journal);
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
            crate::arena::state::ShadowContact {
                target: RED,
                class: crate::arena::state::Classification::Unknown,
                first_seen_ms: 0,
                last_seen_ms: 0,
            },
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
        arena.destroy_unit(BLUE, 500, &mut journal);
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
        arena.destroy_unit(BLUE, 600, &mut journal);
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
