//! **The sensor pass and the contact share** (16b cut S.1; doc 16 §3.6
//! "Perception", §3.2 "Line of sight" and "The spatial index", §3.1 "Rate
//! independence"; D-16-10, W1, W3, W4, W5, W6).
//!
//! One unit's pass is a [`DueKind::SensorPass`] every `sensors.pass_ms` of arena
//! time, **staggered by unit id**, applied at its own timestamp against the
//! positions the substep boundary left (doc 16 §3.1's rate-independence rule). It
//! is doc 16 §3.6's five steps and nothing else — no emitter term (W3), no
//! cross-observer cache (§3.2), no state outside the shadow contact store.
//!
//! # The five steps, and the one place they are not in the doc's order
//!
//! 1. **Candidates** from the spatial index, inside
//!    `max_range_cells × cell_cm × sensors.max_signature_factor` of the observer,
//!    excluding wrecks unless the sensor `sees_wrecks` (doc 16 §3.6 step 1, W1).
//! 2. **Line of sight** from the observer's `Eye` to the candidate's, each at its
//!    unit's `sensor_height_cm`, accepted as `Clear` or as `Partial` under the
//!    sensor's `max_attenuation` (step 2).
//! 3. **The range test**, `range_cm ≤ max_range_cm × signature_factor / 1000`,
//!    the factor read off the sensor definition's signature table by the ground
//!    the target stands on and whether it is moving (step 3).
//! 4. **The contact**: a passing candidate becomes or refreshes a shadow contact
//!    with `class = unknown`, and after `classify_after_ms` of detection with no
//!    gap longer than `pass_ms` it becomes `classified`; a contact not refreshed
//!    within `pass_ms × lost_after_passes` is lost (step 4, W5).
//! 5. **The share**: a unit that ends its pass holding anything classified
//!    schedules a [`DueKind::Share`], which hands its classified contacts to
//!    teammates inside `share_range_cells` with line of sight (step 5, W6).
//!
//! # The close look (16b cut Q.2)
//!
//! Step 4 has one more thing in it. A contact that is **classified** and that the
//! pass found inside `condition_reach_permille` of the effective reach step 3
//! just used earns a [`crate::arena::condition::Reading`] — the target's
//! per-section bands, with the timestamp of the look — which is what `SCAN`
//! prints. It costs one comparison per candidate that has already passed every
//! other test and a thirteen-byte copy per contact that earns one; no new trace,
//! no new cell probe.
//!
//! It is written **after** the classification arm and not beside the plot, so the
//! pass that classifies is also the pass that takes the first reading. A refresh
//! that does not earn a new look **carries the old one forward** with its own
//! older stamp rather than dropping it, because knowledge goes stale rather than
//! vanishing; and a share hands over the sharer's **own** reading under the same
//! strictly-newer rule the plot follows, so there is still no second hop.
//!
//! # The mesh is continuous, and what that costs
//!
//! The share runs on **every** pass of a unit that has something to say, not only
//! on the pass that classified it. It has to: a share *refreshes* the copy it hands
//! over, and the receiver's own pass expires what nothing refreshed
//! (`pass_ms × lost_after_passes`), so a mesh that only spoke on the transition
//! would hand a teammate a contact and then take it away three passes later while
//! the sharer was still looking straight at it. W6's *shared contacts fan out only
//! within comms range* is a continuous datalink, and this is that.
//!
//! The price is one `(sharer, teammate)` pair — one range test and one trace — per
//! sharer per pass, which is why `comms.share_range_cells` is a *short* reach and
//! why `[tacsim.harness].max_share_ops_per_pulse` exists at all. There is no
//! cheaper continuous mesh: a receiver that pulled instead of the sharer pushing
//! would walk the same pairs.
//!
//! **Step 3 runs before step 2**, and that is the only departure from doc 16's
//! numbering. The two are a conjunction — a candidate must pass both — so their
//! order cannot change any answer, and the range test is integer arithmetic over
//! one cell read while the trace walks up to ninety cells. Doc 16 §2 W4 bounds
//! *traces and cell probes per pulse*; running the cheap half of the conjunction
//! first is how that bound is met without weakening it.
//!
//! # Why nothing here is cached and nothing here is shared
//!
//! Doc 16 §3.2: *It is directional; there is no cross-observer cache.* A cache
//! keyed on a cell pair would make one observer's pass depend on which observer
//! ran first, which is exactly the kind of order dependence that the cross-tempo
//! property (A7) exists to refuse. So the pass is a pure function of the arena's
//! state at the event's timestamp, every observer pays for its own traces, and
//! the counters below are what makes the cost visible rather than assumed.
//!
//! # Power
//!
//! The pass does **not** require power. Doc 16 §3.3's `STARTUP` gate is written
//! against *motion and fire intents*; optics are optics, and a cold tank with an
//! intact sight can be looked out of. A wreck is different: it observes nothing,
//! because the `destroy_unit` consequence revokes its capabilities (doc 16 §3.3).
//!
//! The **target's** power is a different question, and **16b cut X.3** is where
//! it is asked: a sight with a thermal channel reads a second signature factor
//! off the target's power state and speed ([`ThermalSignature`]), so a tank that
//! has been shut down goes dark to it at a range where a running one is held.
//! It is one more integer multiply inside step 3's range test, behind an
//! `Option` that is `None` for every sight the game ships — and therefore **no
//! new trace**: the term can only shorten a reach, so it culls candidates
//! *before* the trace that step 2 would have walked.
//!
//! The **jammer's** power is a third question, and **16b cut X.4** asks it: a
//! jamming pod is an emitter, so a pod whose hull has not started up radiates
//! nothing (see [`Jamming`]).
//!
//! # ECM (16b cut X.4; doc 16 §9 item 4, W1)
//!
//! A pod degrades the band **around itself**, and what that costs is paid by
//! whoever is looking from inside the bubble: for each observer, the pass
//! collects the live pods within their own `radius_cm` of the **observer** and
//! applies two terms to that one pass —
//!
//! 1. the observer's step 2 attenuation budget loses the pods' summed
//!    `attenuation_cost`, floored at zero, so a line the eye would have accepted
//!    through one belt of light woods is no longer a detection; and
//! 2. every target's step 3 signature factor is multiplied by the pods'
//!    `signature_permille`, so the reach itself collapses — which is the whole of
//!    doc 16 §2 W1, *standard ECM defeats detection at long range*, as arithmetic.
//!
//! A target standing in a bubble is seen exactly as it was by an observer
//! standing outside one, and a pod is in its own bubble: this is a rule about
//! where the *looker* is, with no exception for the unit carrying the thing.
//!
//! **W3 still holds**: this is not an emitter term in the sense that fact forbids.
//! Nothing here lengthens a reach, accepts a line the sensor would have refused or
//! detects anything at all — a pod can only ever take away, which is why the pass
//! is still *range plus line of sight and nothing else*.
//!
//! The scan is bounded by the same spatial index the candidate cull uses, at the
//! widest radius any pod in the match declared ([`crate::arena::state::Arena`]'s
//! `jam_radius_cm`), and a match in which nothing carries a pod — every match the
//! game ships — never looks: the radius is zero and the whole term is skipped, so
//! the arithmetic above is bit-identical to the arithmetic before this cut.
//!
//! # ECCM (16b cut X.5; doc 16 §9 item 5)
//!
//! *A component that subtracts from that term.* A **filter** is carried by the
//! observer, and before either of the two terms above is read, the fold the observer
//! is standing in loses what its own filter recovers ([`Jamming::recovered`]): the
//! summed cost loses the filter's `attenuation_recovery`, floored at zero, and the
//! surviving signature fraction gains its `signature_recovery_permille`, capped at
//! [`FULL_SIGNATURE_PERMILLE`].
//!
//! Two properties make it an *inverse* of the ECM term rather than a second term:
//!
//! - **it can never see better than no bubble at all** — the cost floors where the
//!   sensor's own budget is and the fraction ceilings where the unjammed reach is, so
//!   a fully rated filter is the unjammed pass to the centimetre and an
//!   over-rated one is nothing more; and
//! - **it is invisible without a bubble** — the recovery is applied inside
//!   [`Arena::jamming_at`]'s `Option`, so an observer nothing is jamming runs the
//!   arithmetic it ran before this cut, filter or no filter.
//!
//! A filter is **not** an emitter, which is why — unlike a pod — it needs no power
//! (see [`Eccm::read`]); what it does need is to still be there, and its component's
//! `condition` is read for that at check-out.

use mush_atlas::TerrainClass;
use mush_atlas::trace::{Eye, Sight, trace_cells};
use mush_core::ObjId;
use mush_tactics::TacticalUnit;
use smol_str::SmolStr;

use crate::arena::index::range_cm;
use crate::arena::ledger::EventKind;
use crate::arena::rng::{DrawLabel, draw, draw_contact_id};
use crate::arena::state::{
    Arena, Classification, ContactId, Designator, DueKind, OwnSighting, Power, SeenPose,
    ShadowContact, Team, UnitState,
};

/// The factor of a target that hides nothing, per mille.
pub const FULL_SIGNATURE_PERMILLE: u32 = 1_000;

/// **One row of a sensor's signature table**, as the arena carries it (16b cut
/// S.1; doc 16 §3.6 step 3).
///
/// The arena's own copy of `crate::definitions::sensor::SignatureRow`: the
/// content decoder's type is `serde`-shaped and lives beside the pack loader,
/// and the arena holds neither. Cut T's check-out copies the rows across
/// ([`crate::match_::arena_def::sensor_profile`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignatureFactor {
    /// The ground the target stands on.
    pub class: TerrainClass,
    /// The factor while it is under way, per mille.
    pub moving_permille: u32,
    /// The factor while it is stopped, per mille.
    pub stopped_permille: u32,
}

/// **What a unit sees with**, as the arena carries it (16b cut S.1; doc 16 §3.6).
///
/// Every field is an integer (D-16-4) and every one is the decoded form of a
/// `sensor` definition row. A unit with `None` for this runs no pass at all,
/// which is what an unmodelled hull and the unit tests of cut A have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensorProfile {
    /// The candidate radius at signature factor 1, in cells (doc 16 §3.6 step 1).
    pub max_range_cells: u16,
    /// The most `Partial` attenuation a line of sight may carry and still be a
    /// detection (step 2). The atlas prices light woods at 25 and heavy at 60.
    pub max_attenuation: u8,
    /// How long a contact must be held, with no gap longer than `pass_ms`, before
    /// it classifies (step 4).
    pub classify_after_ms: u32,
    /// Whether a wreck is a candidate (step 1).
    pub sees_wrecks: bool,
    /// The comms reach a classified contact is shared along, in cells (step 5,
    /// W6). Zero shares nothing.
    pub share_range_cells: u16,
    /// The signature table, by the ground the target stands on. A class it does
    /// not name is [`FULL_SIGNATURE_PERMILLE`] in both postures.
    pub signature: Vec<SignatureFactor>,
}

impl SensorProfile {
    /// **The signature factor of a target**, per mille (doc 16 §3.6 step 3).
    ///
    /// The row for `class`, read at `moving` or at stopped; a class the table does
    /// not name is a target that hides nothing. The answer is never above
    /// [`FULL_SIGNATURE_PERMILLE`], which the definition's own `check` already
    /// refuses — asserted here too, because a factor above 1 would *lengthen* a
    /// sensor's range and that is `sensors.max_signature_factor`'s job.
    #[must_use]
    pub fn signature_permille(&self, class: TerrainClass, moving: bool) -> u32 {
        let row = self.signature.iter().find(|row| row.class == class);
        let permille = match row {
            Some(row) if moving => row.moving_permille,
            Some(row) => row.stopped_permille,
            None => FULL_SIGNATURE_PERMILLE,
        };
        permille.min(FULL_SIGNATURE_PERMILLE)
    }
}

/// **What a sight makes of a target's heat** (16b cut X.3; doc 16 §9 item 3: *an
/// idling tank is cold*), as the arena carries it.
///
/// The arena's own copy of `crate::definitions::sensor::ThermalSignature`, read
/// once at check-out off the cells the `TacticalUnit` already carries — the same
/// route 16b cut X.1's heat takes, and for the same reason: the check-out's
/// sensor row is a struct seven test files build literally, so the numbers ride
/// on the unit rather than across a widened seam.
///
/// A sight that authored no thermal channel has **no** `ThermalSignature` at all,
/// which is what makes the term an `Option` test per candidate for every unit the
/// game ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThermalSignature {
    /// The factor a powered, stopped target presents, per mille.
    pub powered_permille: u32,
    /// The factor a target whose power is not online presents, per mille.
    pub cold_permille: u32,
}

impl ThermalSignature {
    /// **Read one unit's thermal channel off its cells** (16b cut X.3), or `None`
    /// for a sight that carries no [`THERMAL_POWERED_CELL`] — every stock sight.
    ///
    /// The component is **found** rather than derived, exactly as
    /// [`crate::arena::state::HeatState::read`] finds the section carrying the
    /// heat cells: `bind`'s choice of which component the cells sit on is
    /// authoritative and nothing here re-derives it. A unit that somehow mounted
    /// two thermal sights reads the first in component order, which is the same
    /// rule `sensor_profile` follows for the sensor row itself.
    ///
    /// [`THERMAL_POWERED_CELL`]: crate::definitions::unit::THERMAL_POWERED_CELL
    #[must_use]
    pub fn read(unit: &TacticalUnit) -> Option<ThermalSignature> {
        let powered =
            mush_tactics::CellId::new(crate::definitions::unit::THERMAL_POWERED_CELL).ok()?;
        let cold = mush_tactics::CellId::new(crate::definitions::unit::THERMAL_COLD_CELL).ok()?;
        unit.components().find_map(|(component, _)| {
            let at = |cell: &mush_tactics::CellId| {
                let addr = mush_tactics::CellAddr::Component {
                    component: component.clone(),
                    cell: cell.clone(),
                };
                u32::try_from(unit.cell(&addr)?.current.max(0)).ok()
            };
            Some(ThermalSignature {
                powered_permille: at(&powered)?,
                cold_permille: at(&cold)?,
            })
        })
    }

    /// **The factor this sight makes of a target in that state**, per mille (doc
    /// 16 §9 item 3, keyed on power and speed).
    ///
    /// A target under way is the plume a running engine makes and gets no
    /// discount; a powered, stopped one is an idling engine; anything not online
    /// is cold metal. The answer is never above [`FULL_SIGNATURE_PERMILLE`] —
    /// the definition's own `check` refuses that, and it is clamped here for the
    /// same reason the terrain factor is: a factor above 1 would *lengthen* a
    /// sensor's range, which is `sensors.max_signature_factor`'s job.
    #[must_use]
    pub fn permille(&self, power: Power, speed_cms: i32) -> u32 {
        let permille = if power != Power::Online {
            self.cold_permille
        } else if speed_cms == 0 {
            self.powered_permille
        } else {
            FULL_SIGNATURE_PERMILLE
        };
        permille.min(FULL_SIGNATURE_PERMILLE)
    }
}

/// **The jamming pod one unit carries** (16b cut X.4; doc 16 §9 item 4), as the
/// arena carries it.
///
/// The arena's own copy of `crate::definitions::sensor::JammingDef`, read once at
/// check-out off the cells the `TacticalUnit` already carries — the route cut X.1's
/// heat and cut X.3's thermal band take, for the reason [`ThermalSignature`] gives.
/// `None` for every unit the game ships.
///
/// `radius_cm` and not `radius_cells`: the authored cell is in cells, and the
/// window's own `cell_cm` is applied once here rather than on every pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Jammer {
    /// How far the bubble reaches from the pod, centimetres.
    pub radius_cm: u32,
    /// What an observer inside it loses from its sensor's `max_attenuation`, on
    /// the atlas's 0-to-100 scale.
    pub attenuation_cost: u8,
    /// What it leaves of a target's signature factor, per mille.
    pub signature_permille: u32,
}

impl Jammer {
    /// **Read one unit's pod off its cells** (16b cut X.4), or `None` for a unit
    /// carrying no [`JAM_RADIUS_CELL`] — every stock unit.
    ///
    /// The component is **found** rather than derived, exactly as
    /// [`ThermalSignature::read`] finds the sight that carries the thermal band: a
    /// unit that somehow mounted two pods radiates the first in component order.
    ///
    /// [`JAM_RADIUS_CELL`]: crate::definitions::unit::JAM_RADIUS_CELL
    #[must_use]
    pub fn read(unit: &TacticalUnit, cell_cm: u16) -> Option<Jammer> {
        let radius = mush_tactics::CellId::new(crate::definitions::unit::JAM_RADIUS_CELL).ok()?;
        let cost =
            mush_tactics::CellId::new(crate::definitions::unit::JAM_ATTENUATION_CELL).ok()?;
        let signature =
            mush_tactics::CellId::new(crate::definitions::unit::JAM_SIGNATURE_CELL).ok()?;
        unit.components().find_map(|(component, _)| {
            let at = |cell: &mush_tactics::CellId| {
                let addr = mush_tactics::CellAddr::Component {
                    component: component.clone(),
                    cell: cell.clone(),
                };
                u32::try_from(unit.cell(&addr)?.current.max(0)).ok()
            };
            Some(Jammer {
                radius_cm: at(&radius)?.saturating_mul(u32::from(cell_cm)),
                attenuation_cost: u8::try_from(at(&cost)?).unwrap_or(u8::MAX),
                signature_permille: at(&signature)?.min(FULL_SIGNATURE_PERMILLE),
            })
        })
    }
}

/// **The counter-jamming filter one unit carries** (16b cut X.5; doc 16 §9 item 5:
/// *a component that subtracts from that term*), as the arena carries it.
///
/// The arena's own copy of `crate::definitions::sensor::EccmDef`, read once at
/// check-out off the cells the `TacticalUnit` already carries — the route cut X.1's
/// heat, cut X.3's thermal band and cut X.4's pod take, for the reason
/// [`ThermalSignature`] gives. `None` for every unit the game ships.
///
/// It sits on the **observer**: what it recovers, it recovers for the unit carrying
/// it and for nothing else on the field (see [`Jamming::recovered`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Eccm {
    /// What it takes back off the summed cost against the observer's
    /// `max_attenuation`, on the atlas's 0-to-100 scale.
    pub attenuation_recovery: u8,
    /// What it adds back to the surviving fraction of a target's signature factor,
    /// per mille.
    pub signature_recovery_permille: u32,
}

impl Eccm {
    /// **Read one unit's filter off its cells** (16b cut X.5), or `None` for a unit
    /// carrying no [`ECCM_ATTENUATION_CELL`] — every stock unit.
    ///
    /// The component is **found** rather than derived, exactly as [`Jammer::read`]
    /// finds the pod: a unit that somehow mounted two filters reads the first one in
    /// component order that is still there.
    ///
    /// **A filter must still be there to count**: the same component's
    /// [`CONDITION_CELL`] has to be above zero, which is the one thing that can stop
    /// a filter working. It needs **no power**, and that is deliberate — a pod is an
    /// *emitter* and a hull that has not started up radiates nothing (see
    /// [`Arena::jamming_at`]), while a filter radiates nothing at all: it
    /// discriminates signal from noise in the sight's own returns, so it follows the
    /// sight, and this module's head says optics are optics. A cold tank with an
    /// intact sight and an intact filter sees exactly what a warm one does.
    ///
    /// [`ECCM_ATTENUATION_CELL`]: crate::definitions::unit::ECCM_ATTENUATION_CELL
    /// [`CONDITION_CELL`]: crate::definitions::unit::CONDITION_CELL
    #[must_use]
    pub fn read(unit: &TacticalUnit) -> Option<Eccm> {
        let attenuation =
            mush_tactics::CellId::new(crate::definitions::unit::ECCM_ATTENUATION_CELL).ok()?;
        let signature =
            mush_tactics::CellId::new(crate::definitions::unit::ECCM_SIGNATURE_CELL).ok()?;
        let condition = mush_tactics::CellId::new(crate::definitions::unit::CONDITION_CELL).ok()?;
        unit.components().find_map(|(component, _)| {
            let at = |cell: &mush_tactics::CellId| {
                let addr = mush_tactics::CellAddr::Component {
                    component: component.clone(),
                    cell: cell.clone(),
                };
                u32::try_from(unit.cell(&addr)?.current.max(0)).ok()
            };
            let recovery = at(&attenuation)?;
            let permille = at(&signature)?;
            // A filter that has been shot off its mount is not a filter. The
            // `condition` cell is laid beside the two above by `bind`, so a
            // component that carries them and no condition is a rig nothing can
            // author; `?` treats it as no filter either way.
            if at(&condition)? == 0 {
                return None;
            }
            Some(Eccm {
                attenuation_recovery: u8::try_from(recovery).unwrap_or(u8::MAX),
                signature_recovery_permille: permille.min(FULL_SIGNATURE_PERMILLE),
            })
        })
    }
}

/// **What the pods one observer is standing among do to its pass** (16b cut X.4;
/// doc 16 §9 item 4), accumulated once per pass by [`Arena::jamming_at`].
///
/// `None` from that function is *no pod in reach*, and the pass then runs the
/// arithmetic it ran before this cut to the centimetre. A `Some` is two terms:
///
/// - [`Jamming::attenuation_cost`] — the pods' costs **summed**, saturating, and
///   taken off the observer's step 2 budget by [`Jamming::budget`], floored at
///   zero. Summed because two pods each eating a quarter of the budget have eaten
///   half of it.
/// - [`Jamming::signature_permille`] — the pods' factors **multiplied**, per
///   mille, applied to every target's step 3 reach by [`Jamming::scale`].
///   Multiplied because each is a fraction of what is left, and folded in
///   ascending unit-id order (the order [`crate::arena::index::SpatialIndex`]
///   answers in) so that three pods round the same way on a replay.
///
/// One pod's factor survives the fold exactly: `1000 × p / 1000` is `p`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Jamming {
    /// The summed cost against the observer's attenuation budget.
    pub attenuation_cost: u32,
    /// The surviving fraction of every target's signature factor, per mille.
    pub signature_permille: u32,
}

impl Jamming {
    /// **What is left of this jamming once the observer's own filter has taken its
    /// share** (16b cut X.5; doc 16 §9 item 5: *a component that subtracts from that
    /// term*), applied to the fold before either of the two terms is read.
    ///
    /// One subtraction and one addition, each the inverse of the fold's own
    /// arithmetic:
    ///
    /// - `attenuation_cost` loses `attenuation_recovery`, **saturating at zero**, so
    ///   the recovered budget is never wider than the sensor's own
    ///   ([`Jamming::budget`] subtracts from `max_attenuation`);
    /// - `signature_permille` gains `signature_recovery_permille`, **capped at**
    ///   [`FULL_SIGNATURE_PERMILLE`], so the recovered reach is never longer than the
    ///   unjammed one ([`Jamming::scale`] of `1000` is the identity).
    ///
    /// `None` — every unit the game ships — returns the fold untouched, which is what
    /// makes a pass with no filter on it bit-identical to the pass cut X.4 left. A
    /// filter with **no bubble** never reaches here at all: [`Arena::jamming_at`]
    /// answers `None` and the whole term is skipped, so a filter is not a thing that
    /// can be detected by its effect on an unjammed pass.
    #[must_use]
    pub fn recovered(self, eccm: Option<Eccm>) -> Jamming {
        let Some(eccm) = eccm else {
            return self;
        };
        Jamming {
            attenuation_cost: self
                .attenuation_cost
                .saturating_sub(u32::from(eccm.attenuation_recovery)),
            signature_permille: self
                .signature_permille
                .saturating_add(eccm.signature_recovery_permille)
                .min(FULL_SIGNATURE_PERMILLE),
        }
    }

    /// **The observer's step 2 budget under this much jamming** (doc 16 §9 item 4:
    /// *adds to the observer's `max_attenuation` budget* consumed), floored at
    /// zero: a budget of zero accepts a `Clear` line and nothing else, which is a
    /// sight that has to be looking straight at what it holds.
    #[must_use]
    pub fn budget(&self, max_attenuation: u8) -> u8 {
        max_attenuation.saturating_sub(u8::try_from(self.attenuation_cost).unwrap_or(u8::MAX))
    }

    /// **The reach that is left of `reach_cm`** (doc 16 §9 item 4: *lowers
    /// signature factors within a radius*), per mille, as step 3's own arithmetic
    /// does it.
    #[must_use]
    pub fn scale(&self, reach_cm: u64) -> u64 {
        reach_cm
            .saturating_mul(u64::from(self.signature_permille))
            .saturating_div(u64::from(FULL_SIGNATURE_PERMILLE))
    }
}

/// **What one pulse's perception cost** (16b cut S.1; doc 16 §2 W4 and W6: *traces
/// and cell probes per pulse under `harness.max_traces_per_pulse`*, *share
/// operations per pulse bounded*).
///
/// Counted by the arena and reported on [`crate::arena::step::StepOutput`], so the
/// harness asserts a measurement rather than an estimate. They are **counters, not
/// throttles**: an engine that stopped tracing halfway through a pulse would make
/// the outcome a function of the tempo, which D-16-2 forbids.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerceptionCounters {
    /// Sensor passes applied.
    pub passes: u32,
    /// Candidates the spatial index offered, after the wreck rule.
    pub candidates: u32,
    /// Line-of-sight traces walked — detection's and the share mesh's together.
    pub traces: u32,
    /// Cells read by those traces, hit or miss.
    pub cell_probes: u64,
    /// `(sharer, teammate)` pairs considered.
    pub share_ops: u32,
    /// **Units the jammer scan looked at** (16b cut X.4), one per unit the spatial
    /// index offered inside the match's widest pod radius, pod or not.
    ///
    /// Zero for every match in which nothing carries a pod, because the scan does
    /// not run at all then — which is the cost claim this counter exists to make
    /// checkable rather than assumed.
    pub jam_checks: u32,
    /// Contacts newly held.
    pub detected: u32,
    /// Contacts newly told apart.
    pub classified: u32,
    /// Contacts lost to the time-to-live.
    pub lost: u32,
}

impl PerceptionCounters {
    /// Add another pulse's counts — the harness's running total.
    pub fn absorb(&mut self, other: &PerceptionCounters) {
        self.passes = self.passes.saturating_add(other.passes);
        self.candidates = self.candidates.saturating_add(other.candidates);
        self.traces = self.traces.saturating_add(other.traces);
        self.cell_probes = self.cell_probes.saturating_add(other.cell_probes);
        self.share_ops = self.share_ops.saturating_add(other.share_ops);
        self.jam_checks = self.jam_checks.saturating_add(other.jam_checks);
        self.detected = self.detected.saturating_add(other.detected);
        self.classified = self.classified.saturating_add(other.classified);
        self.lost = self.lost.saturating_add(other.lost);
    }
}

/// One journalled perception event: its arena timestamp, what happened, and the
/// draw where one was made.
pub type Notice = (i64, EventKind, Option<(String, u64)>);

/// A candidate that passed every test of the pass.
struct Detection {
    target: ObjId,
    team: Team,
    /// **Where it stood at this sighting** (16b cut F.3), taken from the same
    /// snapshot the range test and the trace were decided against — so the plot a
    /// contact records is the substep boundary's position and not the position at
    /// whatever later moment somebody reads the contact.
    seen_at: SeenPose,
    /// **The observer is inside the close-look reach** (16b cut Q.2; doc 16 §2
    /// W5): `range ≤ reach_cm × condition_reach_permille / 1000`, against the
    /// **same** effective reach step 3 just tested — the ground's signature
    /// factor, the thermal term and the jamming scale all already folded into it.
    ///
    /// One comparison per candidate that has already passed every other test, and
    /// no new trace and no new cell probe: a target that is concealed, cold or
    /// jammed is that much harder to read for exactly the reason it is harder to
    /// see. It is what earns a [`crate::arena::condition::Reading`], and nothing
    /// else reads it.
    close: bool,
    /// **What the target's hull read at this sighting** (16b cut Q.2), off the
    /// same snapshot every test above was decided against.
    ///
    /// Carried on the detection rather than read at the write, because the write
    /// happens while the contact store is mutably borrowed and a second read of
    /// the unit vector there would be a read of a *later* moment than the one the
    /// detection was decided at. It is [`UnitState::condition`], which is a cache
    /// and costs a copy of two integers.
    condition: crate::arena::condition::Condition,
}

/// **One classified contact, as the mesh carries it** (16b cut F.3; doc 16 §3.6
/// step 5, W6).
///
/// A share hands over the sharer's **own** sighting — its timestamp and the plot
/// that goes with it, from [`ShadowContact::own_seen`] and never from the
/// sharer's copy of somebody else's. That is what makes a mesh copy expire when
/// the sighting behind it does, and it is why this carries a stamp at all rather
/// than being refreshed to "now".
struct Shared {
    target: ObjId,
    team: Team,
    /// The sharer's own last sighting: when, and where the target stood at it.
    own: OwnSighting,
}

impl Arena {
    /// **The time-to-live of a contact**, arena milliseconds (doc 16 §3.6 step 4):
    /// `sensors.pass_ms × sensors.lost_after_passes`, each read as at least one so
    /// that a misconfigured zero cannot make every contact expire the instant it
    /// is minted.
    #[must_use]
    pub fn contact_ttl_ms(&self) -> i64 {
        let pass = i64::from(self.sensors_cfg.pass_ms.max(1));
        let passes = i64::from(self.sensors_cfg.lost_after_passes.max(1));
        pass.saturating_mul(passes)
    }

    /// **The short name one viewer's frame gives one contact it holds** (16b cut
    /// Q.1; doc 16 §3.6 "What a pilot sees").
    ///
    /// It is read off the **live** contact and off nothing else: the target's
    /// public callsign while `held.class` is `Classified` *now* (or while
    /// `[tacsim.sensors].designate_on_detection` is on), and the viewer's own
    /// provisional number otherwise.
    ///
    /// That is the correction this cut's design needed. A contact is not a
    /// durable object — `ShadowContacts::expire` removes the row and the next
    /// detection re-inserts it `Unknown` under the same id — so a "has been
    /// classified" bit kept anywhere that outlived the row would put the team and
    /// the slot on a blip the platform has just re-found and not yet told apart.
    /// Reading the class here means a re-acquired contact reads `X2` again, and
    /// says so again three seconds later.
    ///
    /// A missing number cannot happen by construction: `sensor_pass` issues one
    /// at the same insert that makes a contact provisional. If one were missing
    /// anyway the row renders `X0`, which resolves against nothing, rather than
    /// taking the arena down.
    #[must_use]
    pub fn designator_for(&self, viewer: ObjId, held: &ShadowContact) -> Designator {
        let named = self.sensors_cfg.designate_on_detection
            || matches!(held.class, Classification::Classified { .. });
        if named && let Some(target) = self.unit(held.target) {
            return target.callsign;
        }
        Designator::Provisional(
            self.designators
                .provisional(viewer, held.target)
                .unwrap_or(0),
        )
    }

    /// **The number behind a row that is currently naming its target by slot**
    /// (16b cut Q.1) — `X<n>` stays typable after the rename, and the
    /// `identified` line quotes it.
    ///
    /// `None` while the row is itself provisional (the number *is* the
    /// designator) and for a contact that was never provisional at all, which is
    /// a mesh arrival.
    #[must_use]
    pub fn alias_for(&self, viewer: ObjId, held: &ShadowContact) -> Option<u16> {
        match self.designator_for(viewer, held) {
            Designator::Slot { .. } => self.designators.provisional(viewer, held.target),
            Designator::Provisional(_) => None,
        }
    }

    /// The observer's eye and the target's, as the atlas trace wants them: each
    /// unit's cell and its own `sensor_height_cm` (doc 16 §3.2 "Line of sight").
    fn eye_of(&self, unit: &UnitState) -> Option<Eye> {
        let cell = self.definition.terrain.cell_of(&unit.pose)?;
        Some(Eye {
            cell,
            height_cm: i32::try_from(unit.sensor_height_cm).unwrap_or(i32::MAX),
        })
    }

    /// **One unit's sensor pass** (doc 16 §3.6), applied at `t_ms` against the
    /// positions the substep boundary committed.
    ///
    /// Answers whether the observer **now holds anything the mesh may carry** —
    /// a classified contact its own sensors saw inside the time to live — which is
    /// what makes the caller schedule a [`DueKind::Share`]; everything else it did
    /// is in `journal` and in `counters`.
    ///
    /// The next pass is not scheduled here — [`Arena::step`] does that from the
    /// event's own timestamp, so the cadence is a function of arena time alone.
    ///
    /// **16b cut F.3 split it in two.** The read half short-circuits for a
    /// platform that detects nothing by construction — no sensor, a wreck, a pose
    /// off the window — but the **loss band runs for every platform that holds
    /// contacts**, because the expiry inside this function is the only one in the
    /// engine and a platform that runs no detections is exactly the platform a
    /// teammate can hand contacts to for ever. It stays here, at the platform's
    /// own pass time, rather than becoming a per-pulse sweep.
    pub fn sensor_pass(
        &mut self,
        observer: ObjId,
        pass_seq: u64,
        t_ms: i64,
        counters: &mut PerceptionCounters,
        journal: &mut Vec<Notice>,
    ) -> bool {
        // ---- the read half: who is detected, decided against one snapshot ----
        let found = self.detections(observer, counters);

        // ---- the write half: the store, then the loss band ----
        let gap_ms = i64::from(self.sensors_cfg.pass_ms.max(1));
        let classify_after_ms = self
            .unit(observer)
            .and_then(|unit| unit.sensor.as_ref())
            .map_or(i64::MAX, |sensor| i64::from(sensor.classify_after_ms));
        let seed = self.seed;
        if found.is_some() {
            counters.passes = counters.passes.saturating_add(1);
        }
        for Detection {
            target,
            team,
            seen_at,
            close,
            condition,
        } in found.into_iter().flatten()
        {
            let id = draw_contact_id(seed, observer, target);
            match self.contacts.get_mut(observer, id) {
                Some(held) => {
                    // Doc 16 §3.6 step 4: *no gap longer than `pass_ms`*. A gap
                    // restarts the continuity clock of a contact that has not
                    // classified yet; one that has stays classified, because
                    // telling a tank apart is not something a viewer forgets while
                    // still holding it.
                    //
                    // 16b cut F.3: read against `last_seen_ms` still, and the two
                    // stamps coincide for the whole of the arm that reads it —
                    // only classified contacts travel over the mesh, so nothing
                    // but this platform's own sensors has ever touched the stamps
                    // of a contact that is still `Unknown`.
                    if held.class == Classification::Unknown
                        && t_ms.saturating_sub(held.last_seen_ms) > gap_ms
                    {
                        held.first_seen_ms = t_ms;
                    }
                    held.last_seen_ms = t_ms;
                    held.seen_at = seen_at;
                    // **16b cut Q.2**: the `OwnSighting` is rebuilt whole on
                    // every refresh, so the reading has to be carried across it
                    // or a pass that did not earn a new look would *delete* the
                    // last one — and a platform that drives out to 900 m would
                    // lose what it read at 200 m, which is the opposite of the
                    // rule that knowledge goes stale rather than vanishing. It
                    // keeps its own older stamp; only a fresh look moves that.
                    let read_before = held.own_seen.and_then(|own| own.condition);
                    held.own_seen = Some(OwnSighting {
                        at_ms: t_ms,
                        plot: seen_at,
                        condition: read_before,
                    });
                    if held.class == Classification::Unknown
                        && t_ms.saturating_sub(held.first_seen_ms) >= classify_after_ms
                    {
                        held.class = Classification::Classified { team };
                        // The draw doc 16 §3.5 pins for this transition. It
                        // decides **nothing** — classification is the elapsed-time
                        // rule above — and it is recorded so that the label stays
                        // in the ledger's pinned vocabulary and a later rule that
                        // wants a roll here finds the stream already keyed.
                        let label = DrawLabel::Classify {
                            observer,
                            target,
                            pass_seq,
                        };
                        let value = draw(seed, &label);
                        journal.push((
                            t_ms,
                            EventKind::Classified {
                                viewer: observer,
                                contact: id,
                                target,
                                team,
                            },
                            Some((label.encode(), value)),
                        ));
                        counters.classified = counters.classified.saturating_add(1);
                    }
                    // **16b cut Q.2: the close look, written AFTER the
                    // classification arm.** The plot above is written before the
                    // class is tested, and a reading written beside it would
                    // record nothing on the very pass that classified the
                    // contact — the pilot would be told `Contact X2 identified:
                    // BA, blue.` and then `No reading on BA yet` about a tank
                    // they are looking straight at. Here the class is whatever
                    // this pass just made it, so the pass that classifies is also
                    // the pass that takes the first reading.
                    //
                    // Both fields move together and only together: what this
                    // platform's own sensors read is what it may put on the mesh
                    // (doc 16 §2 W6), and what it holds is that or a teammate's,
                    // whichever is newer.
                    if close && matches!(held.class, Classification::Classified { .. }) {
                        let reading = Some(crate::arena::condition::Reading {
                            at_ms: t_ms,
                            condition,
                        });
                        held.seen_condition = reading;
                        if let Some(own) = held.own_seen.as_mut() {
                            own.condition = reading;
                        }
                    }
                }
                None => {
                    // **16b cut Q.1**: the one site the designator book is
                    // written. A number is issued exactly when this platform's
                    // **own** sensors first insert an unresolved return, so the
                    // sequence means "things my sight found, in order" and stays
                    // contiguous; a mesh arrival (`Arena::share_pass`) writes
                    // nothing and consumes nothing. With the switch on no
                    // provisional is ever issued, because nothing ever renders
                    // one.
                    if !self.sensors_cfg.designate_on_detection {
                        self.designators.assign(observer, target);
                    }
                    self.contacts.insert(
                        observer,
                        id,
                        ShadowContact::sighted(target, Classification::Unknown, t_ms, seen_at),
                    );
                    journal.push((
                        t_ms,
                        EventKind::Detected {
                            viewer: observer,
                            contact: id,
                            target,
                        },
                        None,
                    ));
                    counters.detected = counters.detected.saturating_add(1);
                }
            }
        }

        // Doc 16 §3.6 step 4's loss. The cutoff is `t - ttl + 1` because
        // `ShadowContacts::expire` drops what was last seen *before* it: a contact
        // refreshed exactly `ttl` ago has gone `lost_after_passes` passes without
        // a refresh and is lost, and one refreshed more recently is not.
        let cutoff = t_ms.saturating_sub(self.contact_ttl_ms()).saturating_add(1);
        for (id, lost) in self.contacts.expire(observer, cutoff) {
            // **16b cut Q.1**: the name the row was carrying, captured here and
            // nowhere else. `lost` is the record `expire` handed back, so its
            // class is still readable; by the time the `lost` event is staged the
            // row is gone and nothing could tell `BA` from `X2` any more. The
            // staging scan of this same substep takes the entry out again, so
            // nothing put here survives the substep — and nothing reads it
            // anywhere else, the state hash included
            // (`Arena::lost_designators`).
            let named = self.designator_for(observer, &lost);
            self.lost_designators.insert((observer, id), named);
            journal.push((
                t_ms,
                EventKind::Lost {
                    viewer: observer,
                    contact: id,
                    target: lost.target,
                },
                None,
            ));
            counters.lost = counters.lost.saturating_add(1);
        }
        // Whether the mesh has anything to carry, read after the losses: a unit
        // whose last classified contact just expired says nothing this pass.
        //
        // **16b cut F.3**: and a unit whose only classified contacts arrived over
        // the mesh says nothing either, because a share is built from the sharer's
        // **own** sighting (doc 16 §2 W6). It is `ShadowContact::shareable` — the
        // one place both share rules are written — asked here once so that a
        // platform with nothing to relay schedules no share at all, and asked
        // again per contact in [`Arena::share_pass`].
        self.contacts
            .of(observer)
            .any(|(_, held)| held.shareable(cutoff).is_some())
    }

    /// Steps 1 to 3 of doc 16 §3.6, over one snapshot of the units and the index.
    ///
    /// `None` when the observer runs no pass at all: it is not in this match, it is
    /// a wreck, it carries no sensor, or its pose is off the window.
    fn detections(
        &self,
        observer: ObjId,
        counters: &mut PerceptionCounters,
    ) -> Option<Vec<Detection>> {
        let watcher = self.unit(observer)?;
        if watcher.limits.wreck {
            return None;
        }
        let sensor = watcher.sensor.as_ref()?;
        let from = self.eye_of(watcher)?;
        let cell_cm = u64::from(self.definition.terrain.cell_cm());
        let max_range_cm = u64::from(sensor.max_range_cells).saturating_mul(cell_cm);
        // Step 1's cull radius: the sensor's reach at the *widest* signature factor
        // any target could present (W1). Every later test only ever shortens it, so
        // a candidate outside this circle cannot be detected by any of them.
        let cull_cm = max_range_cm
            .saturating_mul(u64::from(
                self.sensors_cfg.max_signature_factor_permille.max(1),
            ))
            .saturating_div(1_000);
        let cull = u32::try_from(cull_cm).unwrap_or(u32::MAX);
        // **16b cut X.4**: the pods this observer is standing among, collected once
        // for the pass (doc 16 §9 item 4). `None` — every match the game ships —
        // leaves both terms below untouched. The cull radius is deliberately *not*
        // shortened by the factor: doc 16 §3.2's rule is that the cull never decides
        // membership, and the jam culls where every other signature term culls, in
        // step 3.
        // **16b cut X.5**: and the observer's own filter takes its share back out of
        // that fold before either term is read (doc 16 §9 item 5). It is inside the
        // `map` and not a branch of its own, because a filter with no bubble on it
        // has nothing to subtract from: an observer standing outside every bubble is
        // the same observer with the filter and without it, to the centimetre.
        let jam = self
            .jamming_at(&watcher.pose, counters)
            .map(|jam| jam.recovered(watcher.eccm));
        let attenuation = jam.map_or(sensor.max_attenuation, |jam| {
            jam.budget(sensor.max_attenuation)
        });
        // **16b cut Q.2**: the fraction of a detection's own effective reach that
        // is near enough to read a target's bands off, read once for the pass.
        // Its validation is Q.7's, so it is taken `.min(1000)` here: the widest a
        // close look can be is *everything this platform can detect*.
        let close_permille = self.sensors_cfg.condition_reach_permille.min(1_000);
        let mut found: Vec<Detection> = Vec::new();
        for id in self.index.in_range(&watcher.pose, cull, &self.units) {
            if id == observer {
                continue;
            }
            let Some(target) = self.unit(id) else {
                continue;
            };
            if target.limits.wreck && !sensor.sees_wrecks {
                continue;
            }
            counters.candidates = counters.candidates.saturating_add(1);
            let Some(to) = self.eye_of(target) else {
                continue;
            };
            // Step 3 before step 2: see this module's head. One cell read and
            // integer arithmetic, and it is a conjunct of the same test.
            let class = self
                .definition
                .terrain
                .get(to.cell.0, to.cell.1)
                .map_or(TerrainClass::Void, |cell| cell.class);
            let factor = sensor.signature_permille(class, target.speed_cms != 0);
            let mut reach_cm = max_range_cm
                .saturating_mul(u64::from(factor))
                .saturating_div(1_000);
            // **16b cut X.3**: the thermal term, multiplied into the ground's
            // (doc 16 §9 item 3). It is inside the `Option` and not a multiply by
            // 1,000 on the outside of it, so a match of stock tanks runs the same
            // arithmetic it ran before this cut, to the centimetre.
            if let Some(thermal) = watcher.thermal {
                reach_cm = reach_cm
                    .saturating_mul(u64::from(thermal.permille(target.power, target.speed_cms)))
                    .saturating_div(1_000);
            }
            // **16b cut X.4**: and the jamming term, multiplied into both of them
            // (doc 16 §9 item 4, W1). Inside the `Option` for the same reason: a
            // field with no pod on it runs the multiplication it ran before.
            if let Some(jam) = jam {
                reach_cm = jam.scale(reach_cm);
            }
            let range = range_cm(&watcher.pose, &target.pose);
            if range > reach_cm {
                continue;
            }
            // Step 2: the atlas trace over the arena's own cell grid.
            let trace = trace_cells(&self.definition.terrain, from, to);
            counters.traces = counters.traces.saturating_add(1);
            counters.cell_probes = counters
                .cell_probes
                .saturating_add(u64::from(trace.visited));
            if !sees(&trace.sight, attenuation) {
                continue;
            }
            found.push(Detection {
                target: id,
                team: target.team,
                // 16b cut F.3: the plot, off the same snapshot every test above
                // was decided against.
                seen_at: SeenPose::of(target),
                // **16b cut Q.2**: the close look, against the reach the range
                // test above was decided with — see [`Detection::close`].
                close: range
                    <= reach_cm
                        .saturating_mul(u64::from(close_permille))
                        .saturating_div(1_000),
                condition: target.condition,
            });
        }
        Some(found)
    }

    /// **The jamming one observer at `pose` is standing in** (16b cut X.4; doc 16
    /// §9 item 4), or `None` when no live pod reaches it.
    ///
    /// The scan is one [`crate::arena::index::SpatialIndex`] query at
    /// `jam_radius_cm` — the widest radius any pod in this match declared, resolved
    /// at check-out — and then each candidate's own radius, so a pod with a short
    /// bubble is not widened by a long-legged one sharing the field. A match with
    /// no pod has `jam_radius_cm == 0` and returns here without touching the index
    /// or the counters, which is what makes the stock pass bit-identical.
    ///
    /// Three things stop a pod radiating, all read at scan time rather than baked
    /// in at check-out:
    ///
    /// - **distance**: the observer must be inside *that* pod's radius;
    /// - **a wreck**: `destroy_unit` revokes a unit's capabilities (doc 16 §3.3),
    ///   and a burnt-out hull's transmitter is one of them;
    /// - **power**: a pod is an *emitter* (W3's own distinction), so a hull that has
    ///   not started up jams nobody. This is the one place the pass reads a unit's
    ///   power for a reason other than the target's heat, and it is deliberate: the
    ///   sight of a cold tank still works, because optics are optics, and its pod
    ///   does not, because a transmitter is not.
    ///
    /// A pod is inside its own bubble at range zero, so a unit that carries one
    /// jams itself. There is no exception: the rule is about where the *looker* is.
    pub fn jamming_at(
        &self,
        pose: &mush_sim::data::Pose,
        counters: &mut PerceptionCounters,
    ) -> Option<Jamming> {
        if self.jam_radius_cm == 0 {
            return None;
        }
        let mut cost: u32 = 0;
        let mut permille = FULL_SIGNATURE_PERMILLE;
        let mut jammed = false;
        for id in self.index.in_range(pose, self.jam_radius_cm, &self.units) {
            counters.jam_checks = counters.jam_checks.saturating_add(1);
            let Some(pod) = self.unit(id) else {
                continue;
            };
            let Some(jammer) = pod.jammer else {
                continue;
            };
            if pod.limits.wreck || pod.power != Power::Online {
                continue;
            }
            if range_cm(pose, &pod.pose) > u64::from(jammer.radius_cm) {
                continue;
            }
            jammed = true;
            cost = cost.saturating_add(u32::from(jammer.attenuation_cost));
            permille = permille
                .saturating_mul(jammer.signature_permille)
                .saturating_div(FULL_SIGNATURE_PERMILLE);
        }
        jammed.then_some(Jamming {
            attenuation_cost: cost,
            signature_permille: permille,
        })
    }

    /// **One unit's contact share** (doc 16 §3.6 step 5, W6): its classified
    /// contacts to every teammate inside `share_range_cells` with line of sight.
    ///
    /// The mesh is line of sight and the same attenuation rule detection uses (W6:
    /// *tactical comms are LOS mesh jammed by the same ECM*), so a teammate behind
    /// a wall is told nothing.
    ///
    /// **16b cut Q.6 closed the rest of that W6 clause.** Until it, the ECM half
    /// of the sentence was unbuilt and doc 16 §3.6's own note recorded it as the
    /// one clause still open: a pod jammed every sight in its bubble and jammed no
    /// radio at all. Now a link is made only when **both ends** can carry it —
    /// `jamming_at` is folded at the sharer's pose and at the mate's, each with
    /// that unit's own filter recovered out of it, and the **tighter** of the two
    /// answers governs. The line must pass [`sees`] at the smaller
    /// [`Jamming::budget`], and the sharer-to-mate range must be inside the
    /// smaller [`Jamming::scale`] of the share reach. Two terms, both the sensor
    /// pass's own, so a pod cannot blind a tank and still let it phone home.
    ///
    /// The cull is untouched by all of it: the index is queried at the *unjammed*
    /// reach and jamming only ever shortens, which is doc 16 §3.2's rule that the
    /// cull never decides membership.
    ///
    /// A share **refreshes** what it hands over, which is
    /// what keeps a contact the mesh is still reporting from expiring in the
    /// teammate's own store (cut S's deviation (3), kept).
    ///
    /// **16b cut F.3 gave the refresh its provenance**, which is what stops it
    /// being unbounded. Two rules, and together they end the loop cut S left:
    ///
    /// 1. **Only a sighting travels**, and it travels whole. A platform shares a
    ///    contact only when its **own** sensors saw the target inside the time to
    ///    live, and what it hands over is that sighting's own stamp *and its own
    ///    plot* ([`ShadowContact::own_seen`]) — never the newer copy of somebody
    ///    else's that a teammate's share may have written over
    ///    [`ShadowContact::seen_at`]. A position relayed that way would be a
    ///    second hop in everything but the name on it, and doc 16 §2 W6 is that
    ///    shared contacts fan out only within comms range *of the platform that
    ///    sees*.
    /// 2. **A share never raises a receiver's stamp above the sharer's own
    ///    sighting.** So a mesh copy goes stale exactly when the sighting behind
    ///    it does, plus nothing.
    ///
    /// Before them, two teammates inside share range each refreshed the other's
    /// copy to *now* on every pass, so a contact never expired however far the
    /// target drove, however completely it broke line of sight, and even after it
    /// was a wreck. Entry zones put teammates in adjacent cells, so that held from
    /// the first pulse of every match with two or more tanks a side.
    pub fn share_pass(
        &mut self,
        sharer: ObjId,
        t_ms: i64,
        counters: &mut PerceptionCounters,
        journal: &mut Vec<Notice>,
    ) {
        let Some(sharing) = self.unit(sharer) else {
            return;
        };
        if sharing.limits.wreck {
            return;
        }
        let Some(sensor) = sharing.sensor.as_ref() else {
            return;
        };
        if sensor.share_range_cells == 0 {
            return;
        }
        let Some(from) = self.eye_of(sharing) else {
            return;
        };
        let attenuation = sensor.max_attenuation;
        let cell_cm = u64::from(self.definition.terrain.cell_cm());
        let reach = u32::try_from(u64::from(sensor.share_range_cells).saturating_mul(cell_cm))
            .unwrap_or(u32::MAX);
        // **16b cut Q.6**: the pods the *sharer* is standing among, folded once for
        // the whole pass and not once per mate — the observer's own fold in
        // `detections` is read the same way, and for the same reason. The sharer's
        // own filter takes its share back out before either term is read, exactly
        // as an observer's does. A match with no pod in it has `jam_radius_cm == 0`
        // and never reaches the index or a counter, so a stock share is
        // bit-identical to the one cut F.3 left.
        let sharer_jam = self
            .jamming_at(&sharing.pose, counters)
            .map(|jam| jam.recovered(sharing.eccm));
        let sharer_reach = sharer_jam.map_or(u64::from(reach), |jam| jam.scale(u64::from(reach)));
        // What there is to say: [`ShadowContact::shareable`], which is both of this
        // function's rules written once — classified, and seen by **this
        // platform's own** sensors inside the time to live, with that sighting's
        // own stamp and its own plot. The plot comes off `own_seen` and never off
        // `seen_at`, because `seen_at` may be a teammate's: relaying it would put
        // a position one hop further than the platform that took it, which is the
        // transitive relay doc 16 §2 W6 forbids, wearing this platform's name.
        let cutoff = t_ms.saturating_sub(self.contact_ttl_ms()).saturating_add(1);
        let mine: Vec<Shared> = self
            .contacts
            .of(sharer)
            .filter_map(|(_, held)| {
                let (team, own) = held.shareable(cutoff)?;
                Some(Shared {
                    target: held.target,
                    team,
                    own,
                })
            })
            .collect();
        if mine.is_empty() {
            return;
        }
        let side = sharing.team;
        // Who to say it to, decided against the same snapshot.
        let mut mates: Vec<ObjId> = Vec::new();
        for id in self.index.in_range(&sharing.pose, reach, &self.units) {
            if id == sharer {
                continue;
            }
            let Some(mate) = self.unit(id) else {
                continue;
            };
            if mate.team != side || mate.limits.wreck {
                continue;
            }
            let Some(to) = self.eye_of(mate) else {
                continue;
            };
            counters.share_ops = counters.share_ops.saturating_add(1);
            let trace = trace_cells(&self.definition.terrain, from, to);
            counters.traces = counters.traces.saturating_add(1);
            counters.cell_probes = counters
                .cell_probes
                .saturating_add(u64::from(trace.visited));
            // **16b cut Q.6**: and the pods the *receiver* is standing among, with
            // its own filter against them. A link has two ends and a pod sitting on
            // either one cuts it, so the **tighter** of the two answers governs:
            // the smaller attenuation budget for the line, and the shorter scaled
            // reach for the range. Doc 16 W6 — *tactical comms are LOS mesh jammed
            // by the same ECM* — is the whole rule, and "the same" is meant
            // literally: this is `jamming_at`, `recovered`, `budget` and `scale`,
            // the four the sensor pass uses, so detection and the mesh cannot jam
            // differently.
            let mate_jam = self
                .jamming_at(&mate.pose, counters)
                .map(|jam| jam.recovered(mate.eccm));
            let budget = sharer_jam
                .map_or(attenuation, |jam| jam.budget(attenuation))
                .min(mate_jam.map_or(attenuation, |jam| jam.budget(attenuation)));
            if !sees(&trace.sight, budget) {
                continue;
            }
            let link_reach = sharer_reach
                .min(mate_jam.map_or(u64::from(reach), |jam| jam.scale(u64::from(reach))));
            if range_cm(&sharing.pose, &mate.pose) > link_reach {
                continue;
            }
            mates.push(id);
        }
        let seed = self.seed;
        for mate in mates {
            for shared in &mine {
                let target = shared.target;
                let team = shared.team;
                // A unit is never handed a contact on itself: doc 16 §3.6's
                // contacts are what a platform holds on *something else*.
                if target == mate {
                    continue;
                }
                let id = draw_contact_id(seed, mate, target);
                match self.contacts.get_mut(mate, id) {
                    Some(held) => {
                        // **16b cut F.3**: the sharer's own sighting, and only
                        // when it is newer than what the receiver already has.
                        // Never `t_ms`: a share carries a sighting, not a moment
                        // of transmission, so a receiver's copy cannot outlive the
                        // sighting behind it. The plot travels with the stamp, so
                        // the two can never be a fresh time against an old place.
                        if shared.own.at_ms > held.last_seen_ms {
                            held.last_seen_ms = shared.own.at_ms;
                            held.seen_at = shared.own.plot;
                        }
                        // **16b cut Q.2**: the reading follows the same rule, and
                        // follows it **independently** of the plot — against its
                        // own stamp. A sharer whose sighting is older than the
                        // receiver's may still have had a closer look at some
                        // earlier moment, and a receiver that has only ever seen
                        // the target from 900 m holds no reading at all; the
                        // strictly-newer test is per reading, so neither of them
                        // can overwrite the other's fresher look. The receiver's
                        // own `own_seen.condition` is untouched, which is what
                        // stops a mesh reading being relayed a second hop.
                        //
                        // The one thing carried across the replacement is
                        // destruction: a section the receiver's **own gunner**
                        // watched come apart is written into the reading it
                        // holds with the old look's stamp (it is not a look), so
                        // a teammate's share taken between that look and the
                        // fire is stamped newer while knowing strictly less.
                        // `Condition::keeping_destroyed` is why `SCAN` cannot
                        // contradict the line the pilot was just sent, however
                        // the mesh is shaped around them.
                        if let Some(reading) = shared.own.condition
                            && held
                                .seen_condition
                                .is_none_or(|had| reading.at_ms > had.at_ms)
                        {
                            let bands = held.seen_condition.map_or(reading.condition, |had| {
                                reading.condition.keeping_destroyed(had.condition)
                            });
                            held.seen_condition = Some(crate::arena::condition::Reading {
                                condition: bands,
                                ..reading
                            });
                        }
                        if held.class == Classification::Unknown {
                            held.class = Classification::Classified { team };
                            journal.push((
                                t_ms,
                                EventKind::Classified {
                                    viewer: mate,
                                    contact: id,
                                    target,
                                    team,
                                },
                                // No draw: the classification was the sharer's,
                                // and its label is already in the ledger against
                                // that pass.
                                None,
                            ));
                            counters.classified = counters.classified.saturating_add(1);
                        }
                    }
                    None => {
                        self.contacts.insert(
                            mate,
                            id,
                            ShadowContact {
                                target,
                                class: Classification::Classified { team },
                                // The sighting's own stamp on both, so a copy that
                                // arrives already two passes old is two passes old
                                // and expires accordingly.
                                first_seen_ms: shared.own.at_ms,
                                last_seen_ms: shared.own.at_ms,
                                seen_at: shared.own.plot,
                                // Nothing of this platform's own saw it: the copy
                                // is a mesh copy, and cut F.3's first rule stops
                                // it being relayed on.
                                own_seen: None,
                                // **16b cut Q.2**: the sharer's own reading, as
                                // it stands — a fresh row has nothing to be
                                // newer than. It is `None` for a sharer that has
                                // never had a close look, which is what makes a
                                // mesh-only contact publish no chassis at all.
                                seen_condition: shared.own.condition,
                            },
                        );
                        // Two transitions in one moment, both true of the teammate:
                        // it now holds the contact, and it holds it classified.
                        journal.push((
                            t_ms,
                            EventKind::Detected {
                                viewer: mate,
                                contact: id,
                                target,
                            },
                            None,
                        ));
                        journal.push((
                            t_ms,
                            EventKind::Classified {
                                viewer: mate,
                                contact: id,
                                target,
                                team,
                            },
                            None,
                        ));
                        counters.detected = counters.detected.saturating_add(1);
                        counters.classified = counters.classified.saturating_add(1);
                    }
                }
            }
        }
    }

    /// **Doc 16 §3.1's `Alert` trigger**: some unit holds a contact whose Euclidean
    /// range is under `tempo.alert_range_cm` (which [`Arena::new`] derived when the
    /// key is zero).
    ///
    /// A derived range of zero — a match in which no unit carries a weapon — is
    /// never under itself, so nothing raises `Alert` on a contact alone. That is
    /// the honest answer: there is nothing to shoot with.
    ///
    /// **16b cut F.3** gave it the two things its siblings
    /// ([`Arena::hostile_in_weapons_range`], the reflex controller's target
    /// choice) already had and it lacked:
    ///
    /// * **the wreck filter** — a contact on a burnt-out hull held the whole arena
    ///   at the `Alert` cadence, which is exactly what `sees_wrecks: false` was
    ///   meant to let go of;
    /// * **the plot** — the range is to where the platform last *saw* the target,
    ///   not to where the target is. A tempo read off a position nobody holds
    ///   would be the one place in the arena where hidden truth reached a pilot,
    ///   because how often a pilot is sent a frame is something a pilot can count.
    #[must_use]
    pub fn contact_within_alert_range(&self) -> bool {
        let reach = u64::from(self.alert_range_cm);
        if reach == 0 {
            return false;
        }
        self.units.iter().any(|viewer| {
            self.contacts.of(viewer.id).any(|(_, held)| {
                self.unit(held.target)
                    .is_some_and(|target| !target.limits.wreck)
                    && held.seen_at.range_from(&viewer.pose) < reach
            })
        })
    }

    /// The first [`DueKind::SensorPass`] of **every unit**, at **`id % pass_ms`**
    /// of arena time (doc 16 §3.6: *staggered by unit id*).
    ///
    /// **16b cut F.3** widened it from the units that carry a sensor. The pass is
    /// also where a platform's contacts expire, and a platform with no sight of
    /// its own is exactly the platform a teammate can hand contacts to: it was
    /// never scheduled a pass, so nothing ever let go of what the mesh gave it. A
    /// unit that detects nothing pays one short event a cadence — the read half
    /// returns before the index is touched — which is the cheapest place to put
    /// an expiry that must run for everyone and must not become a per-pulse sweep.
    ///
    /// The stagger is the object number modulo the cadence, so it needs no state,
    /// it is the same on a replay, and two units never collide unless their ids do
    /// modulo `pass_ms`. It spreads a match's passes across the cadence rather than
    /// bunching every unit's on one substep boundary, which is what keeps the
    /// worst pulse's trace count near the average (doc 16 §2 W4).
    pub fn schedule_first_passes(&mut self) {
        let cadence = i64::from(self.sensors_cfg.pass_ms.max(1));
        let ids: Vec<ObjId> = self.units.iter().map(|unit| unit.id).collect();
        for id in ids {
            let offset = i64::from(id.0) % cadence;
            self.due.schedule(
                offset,
                DueKind::SensorPass {
                    unit: id,
                    pass_seq: 1,
                },
            );
        }
    }
}

/// Whether a line of sight is a detection at `max_attenuation` (doc 16 §3.6 step
/// 2): `Clear`, or `Partial` at or under the sensor's ceiling. Anything blocked is
/// nothing seen.
#[must_use]
pub fn sees(sight: &Sight, max_attenuation: u8) -> bool {
    match sight {
        Sight::Clear => true,
        Sight::Partial(pct) => *pct <= max_attenuation,
        Sight::BlockedBy(_) => false,
    }
}

/// **The one refusal for a contact id a platform does not hold** (doc 16 §3.6:
/// *unresolvable, unowned or expired* are one answer).
///
/// It is [`crate::arena::state::NO_SUCH_CONTACT`] itself, not a copy: the line a
/// pilot reads is K.2's and there is exactly one of it in the tree, so the three
/// cases cannot drift into three sentences.
#[must_use]
pub const fn contact_refusal() -> &'static str {
    crate::arena::state::NO_SUCH_CONTACT
}

/// The bearing from `from` to `to`, millidegrees clockwise from north, in
/// `0..360_000` (doc 16 §3.6 "What a pilot sees": *bearing*).
///
/// Floating point inside, quantised on the way out — doc 16 §3.1's determinism
/// rule exactly: *floating point is allowed inside a computation as long as every
/// stored result is quantised*. Two poses in one place answer `0`.
#[must_use]
pub fn bearing_mdeg(from: &mush_sim::data::Pose, to: &mush_sim::data::Pose) -> u32 {
    bearing_of(
        to.x_cm.saturating_sub(from.x_cm),
        to.y_cm.saturating_sub(from.y_cm),
    )
}

/// The same bearing, from a displacement rather than from two poses (16b cut
/// N.1).
///
/// [`bearing_mdeg`] is this function over two [`mush_sim::data::Pose`]s; the
/// reflex controller steers at *cells*, which have no pose and no opaque posture
/// blob, so it needs the arithmetic without one. One implementation, two callers,
/// and the quantisation lives in exactly one place.
#[must_use]
pub fn bearing_of(dx: i64, dy: i64) -> u32 {
    if dx == 0 && dy == 0 {
        return 0;
    }
    #[allow(clippy::cast_precision_loss)]
    let radians = (dx as f64).atan2(dy as f64);
    let degrees = radians.to_degrees();
    let mdeg = (degrees * 1_000.0).round();
    #[allow(clippy::cast_possible_truncation)]
    let whole = mdeg as i64;
    let wrapped = whole.rem_euclid(360_000);
    u32::try_from(wrapped).unwrap_or(0)
}

/// One contact as a pilot's frame carries it (doc 16 §3.6 "What a pilot sees";
/// S.2 renders it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameContact {
    /// The opaque id. **16b cut Q.1 stopped it being a pilot's handle**: it is
    /// the stable row key a panel re-identifies a row by across pulses, and
    /// nothing prints it or parses it any more — see
    /// [`FrameContact::designator`].
    pub id: ContactId,
    /// **The short name the pilot reads and types** (16b cut Q.1), read off this
    /// contact's class *now* through `Arena::designator_for`: the team's letter
    /// and the target's centre slot while the contact is classified, a
    /// viewer-scoped `X<n>` while it is not.
    pub designator: Designator,
    /// **The provisional number this platform gave this target**, where it gave
    /// one and the row is currently naming the target by its slot (16b cut Q.1).
    ///
    /// It is what makes a lock taken as `X2` still typable as `X2` after the
    /// rename, and what the `identified` line quotes. `None` when the row is
    /// already provisional (the number is the designator), and when no number was
    /// ever issued — a contact that arrived over the mesh.
    pub alias: Option<u16>,
    /// `unknown`, or the side the platform has resolved.
    pub class: Classification,
    /// Bearing from the platform, millidegrees clockwise from north.
    pub bearing_mdeg: u32,
    /// Range from the platform **to the plot**, centimetres (doc 16 §3.2's one
    /// range function).
    ///
    /// **Rendered coarsely** (16b cut F.3): the cockpit turns it into one of
    /// `crate::match_::cockpit::range_band`'s four words *and* into metres
    /// rounded to `crate::match_::cockpit::CONTACT_RANGE_STEP_M`. Cut S.2 kept the
    /// figure inside the arena entirely, on doc 16 §3.6's *range band* read
    /// strictly, and that left a pilot with no distance information at all; the
    /// rounding is the deliberate coarseness that replaces the silence, and
    /// `range_m_rounded` records it as the owner's to revisit.
    pub range_cm: u64,
    /// **How stale the plot is**, arena milliseconds (16b cut F.3): now less the
    /// sighting that last refreshed the contact. Cut S.2 carried the hold time
    /// since first detection, which told a pilot nothing about whether the plot
    /// in front of them was a live track or a dying one.
    pub age_ms: i64,
    /// **The cell the target was last seen in** (16b cut S.2), for the marker
    /// `VIEW` draws. `None` for a plot off the window.
    ///
    /// A cell and not a pose: the map's resolution is what the map may show, so a
    /// marker carries exactly the precision the window already prints and no more.
    pub cell: Option<(u16, u16)>,
    /// **What this platform has read off the target's hull**, and how long ago
    /// (16b cut Q.2), or `None` for a contact it has never had a close look at
    /// while it was classified.
    ///
    /// **The section vocabulary lives inside the `Option`.** A section id is a
    /// chassis tell — `left_track` against `left_leg` — so a row with no reading
    /// must be unable to publish one *by construction* rather than by a renderer
    /// remembering not to. [`FrameCondition::table`] indexes
    /// [`crate::arena::step::Frame::scan_tables`], which is built from the rows
    /// that carry a reading and from no others.
    pub condition: Option<FrameCondition>,
}

/// **One contact's reading, as a pilot's frame carries it** (16b cut Q.2; doc 16
/// §3.6 "What a pilot sees").
///
/// Three integers, `Copy` with the row it rides on. The bands are meaningless
/// without the section ids they are in the order of, which is why the table index
/// is in here: see [`FrameContact::condition`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameCondition {
    /// Which of [`crate::arena::step::Frame::scan_tables`] names these sections.
    pub table: u8,
    /// The bands, in that table's order.
    pub bands: crate::arena::condition::Condition,
    /// How old the **look** is, arena milliseconds — `now` less the reading's own
    /// stamp, which is older than the plot's whenever the platform has held the
    /// target since without getting close enough to read it again.
    pub age_ms: i64,
}

/// **A contact, as a notice names it** (16b cut Q.1): the stable id a panel keys
/// a row on, and the short name the sentence says.
///
/// Both, and not one or the other, because they answer different questions. The
/// id is what a renderer matches against `Frame::contacts` and what survives the
/// rename; the designator is what the pilot reads and types, and it is **captured
/// at staging**, off the live contact, so a notice says what the contact was
/// called at the moment the thing happened rather than what it is called by the
/// time the frame is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Named {
    /// The opaque contact id — the row key, never printed.
    pub id: ContactId,
    /// What the pilot reads it as.
    pub designator: Designator,
}

/// **Which transition a pilot is being told about** (16b cut S.2; doc 16 §3.6:
/// *notices fire once per transition (`acquired`, `classified`, `lost`)*, §3.4
/// step 4's `blocked`, §5's *`Notice` events for transitions and refusals*).
///
/// The first three words are the ledger's own `detected`, `classified` and `lost`
/// read as a pilot hears them — `acquired` rather than `detected`, because what
/// the pilot is told is that their platform now holds something.
///
/// **16b cut F.2** added the other eight: doc 16 §5 asks for a notice per
/// transition *and refusal*, and until that cut the vocabulary was the contact
/// lifecycle alone — so a pilot shot to pieces was told nothing, and every order
/// the arena refused at apply vanished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    /// The platform now holds a contact it did not hold before.
    Acquired,
    /// A contact it holds has been told apart.
    Classified,
    /// A contact it held has gone, and stayed gone (see [`FrameNotice`]).
    Lost,
    /// **The platform's lock broke** because the contact it was on stopped being
    /// held (16b cut W.1; doc 16 §3.5: a shot needs *a lock on a contact the platform
    /// holds*).
    ///
    /// Said **at once**, with none of [`FrameNotice`]'s hysteresis, and that is the
    /// one place the two rules part: a contact that flickers behind a treeline costs a
    /// pilot no `lost` line, but a *lock* that breaks has to be said the moment it
    /// breaks, because the pilot's next `FIRE` is going to be refused and the reason
    /// is this. The lock does not come back when the contact does — the platform
    /// re-acquires the contact and the pilot locks it again.
    LockLost,
    /// **The platform's lock resolved** (16b cut F.2; doc 16 §3.5 step 5: the
    /// percentage *on `LOCK` before any shot*), said at the arena timestamp the
    /// lock was taken at.
    Locked,
    /// **A round of this platform's landed** (16b cut F.2), said to the shooter.
    Hit,
    /// **A round of this platform's did not land**, with the ledger's own reason
    /// (16b cut F.2), said to the shooter.
    Miss,
    /// **A round of this platform's finished the target** (16b cut F.2), said to
    /// the shooter after its [`NoticeKind::Hit`].
    Kill,
    /// **This platform was hit** (16b cut F.2), said to the victim — the section
    /// and facet struck and what is left there, and nothing whatever about the
    /// shooter.
    Struck,
    /// **One of this platform's sections is gone** (16b cut F.2), with what that
    /// costs the tank.
    SectionLost,
    /// **A section of the target this platform is shooting at is gone** (16b cut
    /// Q.2; doc 16 §2 W5), said to the **shooter** and only for a contact it holds
    /// and has told apart.
    ///
    /// The owner's first hand-played match found the gap: an attacker read `Hit on
    /// …` and `Miss on …` over and over and then `… destroyed.`, with nothing in
    /// between to say the rounds were doing anything. This is the thing in
    /// between, and it is the *only* thing about the target's damage a gunner is
    /// ever told — a section word, never a facet and never a number.
    TargetSectionLost,
    /// **The arena refused an order at apply** (16b cut F.2; doc 16 §5's
    /// *refusals*), after the world had authorised it.
    Refused,
    /// **Motion stopped the platform** (16b cut F.2; doc 16 §3.4 step 4: *one
    /// `blocked` notice per stop*).
    Blocked,
    /// **The platform's power finished coming up** (16b cut Q.1; doc 16 §3.3
    /// "Startup"), said to that platform's own pilots at the arena timestamp the
    /// flip happened.
    ///
    /// The owner's first hand-played match found the gap: `STARTUP` said the
    /// sequence had begun and then nothing said it had finished, so the way to
    /// find out was to keep typing `HEADING` until one of them was not refused.
    PowerOnline,
}

impl NoticeKind {
    /// The wire word, which is the ledger's word for the same transition where
    /// the ledger has one.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            NoticeKind::Acquired => "acquired",
            NoticeKind::Classified => "classified",
            NoticeKind::Lost => "lost",
            NoticeKind::LockLost => "lock_lost",
            NoticeKind::Locked => "locked",
            NoticeKind::Hit => "hit",
            NoticeKind::Miss => "miss",
            NoticeKind::Kill => "kill",
            NoticeKind::Struck => "struck",
            NoticeKind::SectionLost => "section_lost",
            NoticeKind::TargetSectionLost => "target_section_lost",
            NoticeKind::Refused => "refused",
            NoticeKind::Blocked => "blocked",
            NoticeKind::PowerOnline => "power_online",
        }
    }
}

/// **The numbers a refusal's sentence needs** (16b cut F.2), so the arena's own
/// refusal renders the byte-identical line the cockpit's local pre-check prints
/// for the same condition.
///
/// Read at the moment the order was refused, because that is the only moment they
/// are true: the cycle's remainder is against the arena's `now_ms` and the heat is
/// the cell's, not the pilot's pulse-old mirror of either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalDetail {
    /// The refusal needs no number.
    None,
    /// [`crate::arena::intents::Refusal::Cycling`]: seconds left, rounded up.
    Cycling {
        /// Seconds until the weapon is ready.
        secs: i64,
    },
    /// [`crate::arena::intents::Refusal::Overheated`]: the platform's heat and
    /// its ceiling.
    Overheated {
        /// What it is carrying.
        heat: u32,
        /// The ceiling.
        max: u32,
    },
}

/// **One thing a pilot is told once** (16b cut S.2, extended by 16b cut F.2; doc
/// 16 §3.6 "What a pilot sees", §3.4 step 4, §5).
///
/// It rides [`crate::arena::step::Frame::notices`] and the bridge turns it into a
/// line through the one renderer, `crate::match_::cockpit::notice_line`.
///
/// **The observer rule is the shape of this type.** A contact notice carries the
/// row key, the short designator ([`Named`]) and the class, and **nothing else
/// about the target** — no object number, no unit name, no pose — because a
/// notice is the first thing a pilot is ever told about a contact. **16b cut
/// Q.1** is why the designator is not a leak: it is read off the contact's class
/// at the moment the notice is staged, so it is this platform's own `X<n>` until
/// the contact has been told apart, and the target's *public* callsign — team
/// letter and centre slot, which doc 16 §5 makes lobby knowledge — only after. A [`FrameNotice::Struck`] carries the victim's own section and facet
/// and **nothing about the shooter**: where a round came from is a thing only a
/// sensor may tell, so there is no field here for it to be leaked through. A
/// [`FrameNotice::Blocked`] carries a cause and never the cell's occupant, because
/// an occupied cell may hold a unit this platform has not detected.
///
/// **16b cut Q.2** is the other direction of the same rule. A gunner may be told
/// which section of the target a round found ([`FrameNotice::Hit`]'s `section`)
/// and which section came apart ([`FrameNotice::TargetSectionLost`]) — and only
/// for a contact their platform **holds and has told apart**, because a section
/// id is a chassis vocabulary and a section word on an unidentified blip is
/// identification by gunfire. It is never a facet and never a number: what the
/// round did inside that section is the victim's own gauges.
///
/// # The hysteresis rule, in one sentence
///
/// A `lost` notice is **held for one further pass** (`sensors.pass_ms`) before it
/// is said: if the same platform re-acquires the same contact inside that window
/// the held notice is cancelled and the re-acquisition is **not** announced, so a
/// contact that flickers behind a treeline costs the pilot no lines at all. A loss
/// that outlives the window is announced, one pass late. The ledger's `lost` event
/// is unaffected — it is written at the moment the contact expired, which is what
/// makes the black box the record and the notice only the telling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameNotice {
    /// A contact's lifecycle transition (16b cut S.2).
    Contact {
        /// Which transition.
        kind: NoticeKind,
        /// The contact it is about: the row key and the short name it carried at
        /// the moment of the transition (16b cut Q.1).
        contact: Named,
        /// What the platform knows of it at the moment of the notice.
        class: Classification,
        /// **What it used to be called** (16b cut Q.1), set only on
        /// [`NoticeKind::Classified`] and only when the platform had given the
        /// target a provisional number: the `identified` line says
        /// `Contact X2 identified: BA, blue.`, so a pilot who locked `X2` knows
        /// which return just acquired a name.
        ///
        /// `None` for a classification that was never provisional — a contact
        /// that arrived classified over the mesh, or a match run with
        /// `designate_on_detection` on — and then **no line is said at all**: the
        /// acquisition already read `New contact RB: …`.
        was: Option<Designator>,
    },
    /// **The platform's lock resolved** (16b cut F.2): the percentage rides the
    /// frame this notice arrives on, so the line is the same one `WEAPONSTATUS`
    /// prints and never a second estimate.
    Locked {
        /// The contact now locked.
        contact: Named,
    },
    /// **A round of this platform's landed** (16b cut F.2), named by the contact
    /// the shooter's platform holds the target under — `None` when it holds none
    /// any more, and then the pilot is told only that the round landed.
    Hit {
        /// The contact, where the platform still holds one.
        contact: Option<Named>,
        /// **Which section the round found** (16b cut Q.2), set only when the
        /// shooter holds the target **and that contact is classified at the
        /// moment this is staged**.
        ///
        /// The gate is classification and not merely holding, because a section
        /// id is a **chassis vocabulary**: `Hit on X2: left_track.` tells a gunner
        /// the blip is tracked rather than legged, which is identification by
        /// gunfire and is what doc 16 §2 W5 prices at `classify_after_ms` of
        /// looking. It is the ledger's own `hit` section — a struck-through engine
        /// says `engine`, because that is the section the round found — and never
        /// the facet and never a number.
        section: Option<SmolStr>,
    },
    /// **A round of this platform's went nowhere** (16b cut F.2), with the
    /// ledger's own reason word ([`crate::arena::damage::MISS_REASONS`]).
    Miss {
        /// The contact, where the platform still holds one.
        contact: Option<Named>,
        /// The reason, as the black box records it.
        reason: SmolStr,
    },
    /// **A round of this platform's finished the target** (16b cut F.2).
    Kill {
        /// The contact, where the platform still holds one.
        contact: Option<Named>,
    },
    /// **This platform was hit** (16b cut F.2): the pilot's own tank's section
    /// and facet, which is physical knowledge, and what is left of them.
    Struck {
        /// The section the round found.
        section: SmolStr,
        /// **The section whose armour it passed**, which is `section` itself
        /// unless that section is struck through another's (doc 16 §3.3: the
        /// engine through `hull.rear`). The armour numbers below are this
        /// section's cell, so the sentence names it when the two differ.
        armor_section: SmolStr,
        /// The facet whose armour it passed, on `armor_section`.
        facet: SmolStr,
        /// The armour left in that facet.
        armor_left: u32,
        /// What that facet holds when it is whole.
        armor_max: u32,
        /// The internals left in that section.
        internals_left: u32,
        /// What that section holds when it is whole.
        internals_max: u32,
    },
    /// **One of this platform's sections reached zero internals** (16b cut F.2),
    /// with the consequences that fired, by their authored words.
    SectionLost {
        /// The section.
        section: SmolStr,
        /// Its consequences, in authored order.
        consequences: Vec<SmolStr>,
    },
    /// **A section of the target went** (16b cut Q.2), said to the shooter under
    /// the same held-and-classified gate [`FrameNotice::Hit`]'s section word is.
    ///
    /// It carries the section and **nothing else about the target** — no
    /// consequence list, which is the victim's own physical knowledge, and no
    /// number. An unheld or an unidentified target earns no line at all rather
    /// than a line with the word left out: a gunner who is told *something* went
    /// has still been told the blip has sections.
    TargetSectionLost {
        /// The contact, as the shooter's platform names it.
        contact: Named,
        /// The section the ledger's `section_destroyed` names.
        section: SmolStr,
    },
    /// **The arena refused an order at apply** (16b cut F.2).
    Refused {
        /// The order's own word (`startup`, `lock`, `fire`).
        order: SmolStr,
        /// Why.
        refusal: crate::arena::intents::Refusal,
        /// The weapon it named, where it named one.
        weapon: Option<SmolStr>,
        /// The numbers that refusal's sentence carries.
        detail: RefusalDetail,
    },
    /// **Motion stopped the platform** (16b cut F.2; doc 16 §3.4 step 4).
    Blocked {
        /// Why it stopped.
        cause: crate::arena::motion::BlockedCause,
    },
    /// **Power finished coming up** (16b cut Q.1; doc 16 §3.3 "Startup"), said to
    /// the platform's own pilots. It carries nothing: the transition is the whole
    /// of it.
    PowerOnline,
}

impl FrameNotice {
    /// Which kind this is — the wire word every renderer keys on.
    #[must_use]
    pub const fn kind(&self) -> NoticeKind {
        match self {
            FrameNotice::Contact { kind, .. } => *kind,
            FrameNotice::Locked { .. } => NoticeKind::Locked,
            FrameNotice::Hit { .. } => NoticeKind::Hit,
            FrameNotice::Miss { .. } => NoticeKind::Miss,
            FrameNotice::Kill { .. } => NoticeKind::Kill,
            FrameNotice::Struck { .. } => NoticeKind::Struck,
            FrameNotice::SectionLost { .. } => NoticeKind::SectionLost,
            FrameNotice::TargetSectionLost { .. } => NoticeKind::TargetSectionLost,
            FrameNotice::Refused { .. } => NoticeKind::Refused,
            FrameNotice::Blocked { .. } => NoticeKind::Blocked,
            FrameNotice::PowerOnline => NoticeKind::PowerOnline,
        }
    }

    /// The contact this notice names, where it names one — the **row key**, which
    /// a panel matches against `Frame::contacts` and nothing prints (16b cut
    /// Q.1). What the pilot reads is [`FrameNotice::designator`].
    #[must_use]
    pub const fn contact(&self) -> Option<ContactId> {
        match self.named() {
            Some(named) => Some(named.id),
            None => None,
        }
    }

    /// **The short name this notice's sentence says** (16b cut Q.1), where it
    /// names a contact at all. Never a unit and never an object number.
    #[must_use]
    pub const fn designator(&self) -> Option<Designator> {
        match self.named() {
            Some(named) => Some(named.designator),
            None => None,
        }
    }

    /// **The section of the TARGET this notice names** (16b cut Q.2), where it
    /// names one at all.
    ///
    /// Exactly the two gunner's words: [`FrameNotice::Hit`]'s section, which is
    /// `Some` only under the held-and-classified gate, and
    /// [`FrameNotice::TargetSectionLost`]'s. It is **not** the victim's own
    /// [`FrameNotice::Struck`] or [`FrameNotice::SectionLost`]: those are the
    /// crew reading their own gauges and carry a facet and a consequence list
    /// that this key's readers — a panel highlighting the section a gunner just
    /// took off a target — must not be handed.
    ///
    /// A renderer therefore never has to parse a sentence to know which section a
    /// line is about, which is the one-owner-per-sentence rule this module's head
    /// states.
    #[must_use]
    pub fn section(&self) -> Option<&SmolStr> {
        match self {
            FrameNotice::Hit { section, .. } => section.as_ref(),
            FrameNotice::TargetSectionLost { section, .. } => Some(section),
            FrameNotice::Contact { .. }
            | FrameNotice::Locked { .. }
            | FrameNotice::Miss { .. }
            | FrameNotice::Kill { .. }
            | FrameNotice::Struck { .. }
            | FrameNotice::SectionLost { .. }
            | FrameNotice::Refused { .. }
            | FrameNotice::Blocked { .. }
            | FrameNotice::PowerOnline => None,
        }
    }

    /// The pair, where the notice carries one.
    #[must_use]
    pub const fn named(&self) -> Option<Named> {
        match self {
            FrameNotice::Contact { contact, .. }
            | FrameNotice::Locked { contact }
            | FrameNotice::TargetSectionLost { contact, .. } => Some(*contact),
            FrameNotice::Hit { contact, .. }
            | FrameNotice::Miss { contact, .. }
            | FrameNotice::Kill { contact } => *contact,
            FrameNotice::Struck { .. }
            | FrameNotice::SectionLost { .. }
            | FrameNotice::Refused { .. }
            | FrameNotice::Blocked { .. }
            | FrameNotice::PowerOnline => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::TerrainCell;
    use crate::arena::test_support::{arena_of, arena_painted, pose, tank, visual};

    #[test]
    fn the_signature_table_reads_a_row_by_ground_and_posture() {
        let sensor = visual();
        // The authored rows.
        assert_eq!(
            sensor.signature_permille(TerrainClass::WoodsHeavy, false),
            500
        );
        assert_eq!(
            sensor.signature_permille(TerrainClass::WoodsHeavy, true),
            800
        );
        assert_eq!(sensor.signature_permille(TerrainClass::Road, true), 1_000);
        assert_eq!(sensor.signature_permille(TerrainClass::Road, false), 1_000);
        // A class the table does not name hides nothing, in either posture.
        for moving in [true, false] {
            assert_eq!(
                sensor.signature_permille(TerrainClass::Water, moving),
                FULL_SIGNATURE_PERMILLE
            );
        }
        // And a row that somehow lengthened the range is clamped, whatever the
        // definition's own `check` did or did not catch.
        let wild = SensorProfile {
            signature: vec![SignatureFactor {
                class: TerrainClass::Ice,
                moving_permille: 9_000,
                stopped_permille: 9_000,
            }],
            ..visual()
        };
        assert_eq!(
            wild.signature_permille(TerrainClass::Ice, true),
            FULL_SIGNATURE_PERMILLE
        );
    }

    #[test]
    fn a_bearing_is_millidegrees_clockwise_from_north() {
        let origin = pose(1_000, 1_000);
        assert_eq!(bearing_mdeg(&origin, &pose(1_000, 2_000)), 0);
        assert_eq!(bearing_mdeg(&origin, &pose(2_000, 1_000)), 90_000);
        assert_eq!(bearing_mdeg(&origin, &pose(1_000, 0)), 180_000);
        assert_eq!(bearing_mdeg(&origin, &pose(0, 1_000)), 270_000);
        assert_eq!(bearing_mdeg(&origin, &pose(2_000, 2_000)), 45_000);
        assert_eq!(bearing_mdeg(&origin, &origin), 0);
    }

    #[test]
    fn the_time_to_live_is_the_cadence_times_the_passes() {
        let arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        // The shipped cadence and band: 1,000 ms, three passes.
        assert_eq!(arena.contact_ttl_ms(), 3_000);
    }

    #[test]
    fn a_blocked_line_is_never_a_detection_and_attenuation_has_a_ceiling() {
        assert!(sees(&Sight::Clear, 0));
        assert!(sees(&Sight::Partial(25), 40));
        assert!(sees(&Sight::Partial(40), 40));
        assert!(!sees(&Sight::Partial(41), 40));
        assert!(!sees(&Sight::BlockedBy(mush_atlas::Blocker::Edge), 100));
    }

    #[test]
    fn the_refusal_is_the_cockpits_own_line() {
        assert_eq!(contact_refusal(), "No such contact.");
        assert_eq!(contact_refusal(), crate::match_::cockpit::NO_SUCH_CONTACT);
    }

    #[test]
    fn a_wreck_observes_nothing_and_an_unsensored_unit_runs_no_pass() {
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 1_500)),
        ]);
        // No sensor at all: `test_support`'s tank carries none.
        let mut counters = PerceptionCounters::default();
        let mut journal: Vec<Notice> = Vec::new();
        assert!(!arena.sensor_pass(ObjId(4), 1, 0, &mut counters, &mut journal));
        assert_eq!(counters, PerceptionCounters::default());
        assert!(journal.is_empty());

        // With a sensor it sees; as a wreck it does not.
        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.sensor = Some(visual());
            unit.sensor_height_cm = 150;
        }
        if let Some(unit) = arena.unit_mut(ObjId(9)) {
            unit.sensor_height_cm = 150;
        }
        assert!(!arena.sensor_pass(ObjId(4), 1, 0, &mut counters, &mut journal));
        assert_eq!(counters.passes, 1);
        assert_eq!(counters.detected, 1);
        assert_eq!(arena.contacts.len(), 1);

        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.limits.wreck = true;
        }
        let mut after = PerceptionCounters::default();
        assert!(!arena.sensor_pass(ObjId(4), 2, 1_000, &mut after, &mut journal));
        assert_eq!(after, PerceptionCounters::default());
    }

    #[test]
    fn a_wreck_is_no_candidate_unless_the_sensor_sees_wrecks() {
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 1_500)),
        ]);
        for id in [ObjId(4), ObjId(9)] {
            if let Some(unit) = arena.unit_mut(id) {
                unit.sensor_height_cm = 150;
            }
        }
        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.sensor = Some(visual());
        }
        if let Some(unit) = arena.unit_mut(ObjId(9)) {
            unit.limits.wreck = true;
        }
        let mut counters = PerceptionCounters::default();
        let mut journal: Vec<Notice> = Vec::new();
        arena.sensor_pass(ObjId(4), 1, 0, &mut counters, &mut journal);
        assert_eq!(counters.candidates, 0, "a wreck was offered as a candidate");
        assert!(arena.contacts.is_empty());

        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.sensor = Some(SensorProfile {
                sees_wrecks: true,
                ..visual()
            });
        }
        let mut seeing = PerceptionCounters::default();
        arena.sensor_pass(ObjId(4), 2, 1_000, &mut seeing, &mut journal);
        assert_eq!(seeing.candidates, 1);
        assert_eq!(seeing.detected, 1);
    }

    #[test]
    fn the_stagger_is_the_object_number_modulo_the_cadence() {
        // Three ids whose remainders differ, so the schedule spreads.
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 1_500)),
        ]);
        for id in [ObjId(4), ObjId(9)] {
            if let Some(unit) = arena.unit_mut(id) {
                unit.sensor = Some(visual());
                unit.sensor_height_cm = 150;
            }
        }
        // 16b cut F.3: `Arena::new` already scheduled a pass for **every** unit,
        // so the explicit call cut S.1's version of this test needed (the fixture
        // mounts its sights afterwards) would now schedule a second one each.
        let scheduled: Vec<(i64, ObjId)> = arena
            .due
            .iter()
            .filter_map(|event| match event.kind {
                DueKind::SensorPass { unit, pass_seq: 1 } => Some((event.t_ms, unit)),
                _ => None,
            })
            .collect();
        assert_eq!(scheduled, vec![(4, ObjId(4)), (9, ObjId(9))]);
    }

    /// The signature table, end to end through one pass on the 8 by 8 window: the
    /// target sits three cells out, and a sensor whose reach is exactly three cells
    /// holds it on a road and loses it stopped in heavy woods, because the row
    /// halves the reach. The wider properties — half range, the gap rule, the TTL,
    /// the share — are `crates/tacsim/tests/perception.rs`', which has a window
    /// big enough to put a target at half of the shipped sensor's reach.
    #[test]
    fn a_signature_row_shortens_the_reach_and_a_road_does_not() {
        for (class, held) in [
            (TerrainClass::Road, true),
            (TerrainClass::WoodsHeavy, false),
        ] {
            let mut arena = arena_painted(
                vec![
                    tank(ObjId(4), Team::Red, pose(500, 500)),
                    tank(ObjId(9), Team::Blue, pose(3_500, 500)),
                ],
                // The cell the target stands in, and nothing else: the trace runs
                // over open ground either way, so the only thing that differs is
                // the signature row.
                &[(
                    3,
                    0,
                    TerrainCell {
                        class,
                        elev_cm: 0,
                        move_cost: 150,
                    },
                )],
            );
            if let Some(unit) = arena.unit_mut(ObjId(4)) {
                unit.sensor = Some(SensorProfile {
                    // Three cells of reach against a target 3,000 cm away: full
                    // signature holds it, and the heavy-woods row's 500 does not.
                    max_range_cells: 3,
                    ..visual()
                });
            }
            let mut counters = PerceptionCounters::default();
            let mut journal: Vec<Notice> = Vec::new();
            arena.sensor_pass(ObjId(4), 1, 0, &mut counters, &mut journal);
            assert_eq!(counters.candidates, 1, "{class:?}: no candidate at all");
            assert_eq!(
                arena.contacts.len(),
                usize::from(held),
                "{class:?} answered the wrong way"
            );
        }
    }
}
