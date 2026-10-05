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
//!    **The factor is at least 1.0 and ships at exactly 1.0** (16b cut M.4; 16c
//!    `B-55`, `T-26`): every signature factor step 3 can read is at most 1.0, so
//!    a cull at 1.0 is exact — it holds every candidate the range test can pass
//!    and no more — and a cull below it would drop contacts the sensor can see,
//!    which the host refuses at boot and the pass floors here. The 1.5 shipped
//!    until M.4 was a circle 2.25 times the area it needed.
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
//! one cell read while the trace walks up to the first blocker (16b cut J.1).
//! Doc 16 §2 W4 bounds *traces and cell probes per pulse*; running the cheap
//! half of the conjunction first is how that bound is met without weakening it.
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
//! **The pod has a switch** (16b cut O.2, 16c N12): a pilot's `EMCON off`
//! ([`crate::arena::intents::resolve_emcon`]) silences it, and a silent pod is
//! skipped by [`Arena::jamming_at`] beside a wrecked or unpowered one — so the
//! carrier's own sight clears and its side's mesh closes again, at the price of
//! hiding nobody. Every pod checks out radiating ([`UnitState::ecm_on`]).
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
use mush_atlas::trace::{Eye, Sight};
use mush_core::ObjId;
use mush_tactics::TacticalUnit;
use serde::{Deserialize, Serialize};
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
/// ([`crate::match_::sensor_profile`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// **The sensor component's own id** on the M32 unit (16b cut M.4, 16c
    /// `B-53`) — the component this profile was decoded from
    /// ([`crate::match_::sensor_profile`] sets it), e.g. `visual_sensor` on the
    /// stock tank. The gunnery cone's `sensor_condition` term reads **this**
    /// component's `condition` cell, so on a tank that also carries a pod or a
    /// filter (each with a `condition` cell of its own) the term reads the sight
    /// and not whichever component sorts first.
    pub component: SmolStr,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// unit that somehow mounted two pods radiates the first in component order
    /// that is still there.
    ///
    /// **A pod must still be there to radiate** (16b cut M.4, 16c `B-54`): the
    /// same component's [`CONDITION_CELL`] has to be above zero — the guard
    /// [`Eccm::read`] has always had, now on both. Before M.4 a pod shot to
    /// condition `0` on the record still jammed. **A known limit, named**: the
    /// condition is read **at check-out only** (this runs from
    /// `Arena::new`), and is not re-read after damage inside a match, so a pod
    /// hit to zero mid-match keeps radiating until the hull is a wreck; the
    /// ECM tank mounts its pod on the hull for exactly that reason (the pack's
    /// own note).
    ///
    /// [`JAM_RADIUS_CELL`]: crate::definitions::unit::JAM_RADIUS_CELL
    /// [`CONDITION_CELL`]: crate::definitions::unit::CONDITION_CELL
    #[must_use]
    pub fn read(unit: &TacticalUnit, cell_cm: u16) -> Option<Jammer> {
        let radius = mush_tactics::CellId::new(crate::definitions::unit::JAM_RADIUS_CELL).ok()?;
        let cost =
            mush_tactics::CellId::new(crate::definitions::unit::JAM_ATTENUATION_CELL).ok()?;
        let signature =
            mush_tactics::CellId::new(crate::definitions::unit::JAM_SIGNATURE_CELL).ok()?;
        let condition = mush_tactics::CellId::new(crate::definitions::unit::CONDITION_CELL).ok()?;
        unit.components().find_map(|(component, _)| {
            let at = |cell: &mush_tactics::CellId| {
                let addr = mush_tactics::CellAddr::Component {
                    component: component.clone(),
                    cell: cell.clone(),
                };
                u32::try_from(unit.cell(&addr)?.current.max(0)).ok()
            };
            let radius_cells = at(&radius)?;
            // A pod that has been shot off its mount radiates nothing — the same
            // guard, on the same cell, as `Eccm::read`'s.
            if at(&condition)? == 0 {
                return None;
            }
            Some(Jammer {
                radius_cm: radius_cells.saturating_mul(u32::from(cell_cm)),
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// **A known limit, named** (16b cut M.4, 16c `B-54`): the condition is read
    /// **at check-out only** (this runs from `Arena::new`), exactly as
    /// [`Jammer::read`]'s is, and is not re-read after damage inside a match — a
    /// filter hit to zero mid-match keeps filtering until the match ends.
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

/// **What one platform's own receiver is running under** (16b cut D.4, `B-22`;
/// doc 16 §9 items 4 and 5; 16c §5 Q17: *it knows its own receiver is degraded; it
/// does not know who or from where*).
///
/// It is the [`Jamming`] fold `Arena::detections` already made for the pass —
/// the observer's own filter recovered out of it — read as two numbers about the
/// platform's own machine and nothing else. There is **no source, no bearing, no
/// team, no object number and no designator** on this type, so there is nothing
/// for a later renderer to leak: a pilot is told their sight is being interfered
/// with and is told nothing whatever about the pod.
///
/// `None` on [`UnitState::receiver`] means one of exactly three things, and the
/// pilot is told nothing in any of them beyond the one `Receiver clear.` the
/// transition owes:
///
/// 1. no pod reached the platform at its last pass;
/// 2. a pod reached it and the platform's own filter recovered the whole fold, so
///    the pass ran the arithmetic an unjammed pass runs, to the centimetre;
/// 3. the platform ran no pass at all — it is a wreck, it carries no sight, it is
///    not in the match, or its pose is off the window. **The receiver is off, not
///    clear**, which is why that case journals nothing.
///
/// # The two declared limits (16c §5 Q17, put to the owner)
///
/// The three integers are an instrument reading, and an instrument reading is
/// something a pilot can act on:
///
/// * **the edge is walkable** — a pilot who drives until the line flips has found
///   the boundary of a bubble, which is a fact about the ground rather than about
///   the pod;
/// * **a deeper degradation is readable** — two stacked pods read `0 of 40` where
///   one reads `15 of 40`, so a pilot can tell that more than one emitter is
///   folded at their own cell.
///
/// Neither says **whose** the emitters are or **where** they stand, which is the
/// whole of Q17's sentence. The cheap tightening, if the owner wants it, is to
/// print a thresholded word (`degraded` / `severely degraded`) on `STATUS` and on
/// the frame and to keep the integers off the wire; it is deliberately not built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receiver {
    /// The attenuation budget the last pass actually ran with —
    /// [`Jamming::budget`] of the sight's own `max_attenuation`.
    pub budget: u8,
    /// What that budget is when nothing is jamming: the sight's own
    /// `max_attenuation`.
    pub budget_max: u8,
    /// What is left of every target's signature factor, per mille — the fold's
    /// own [`Jamming::signature_permille`], after the filter.
    pub reach_permille: u16,
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
    /// Cells read by those traces, hit or miss — **since 16b cut J.1 the
    /// interior cells up to and including the first blocker**
    /// ([`mush_atlas::sight_only`] stops there and never reads an endpoint),
    /// and **since 16b cut J.2 only the cells the horizon index did not prove
    /// past**: a walked run's cells up to the blocker, and the one cell a
    /// blocked proof reads.
    pub cell_probes: u64,
    /// **Block reads the horizon index made** (16b cut J.2): one per block
    /// read, whichever rule settled the run it covers — proved clear, proved
    /// blocked, descended, or walked at the finest level; the cells a walked or
    /// blocked run reads go to `cell_probes`. Zero on a terrain without levels.
    pub horizon_probes: u64,
    /// `(sharer, teammate)` pairs considered.
    pub share_ops: u32,
    /// **Index candidates the jammer scan looked at** (16b cut X.4), one per unit
    /// the spatial index offered inside the match's widest pod radius, pod or
    /// not — **once per unit between two clears of [`Arena::jam_fold`]** (16b
    /// cut U.2, 16c `T-29`): the fold is memoised there and counted on a miss
    /// only.
    ///
    /// **The clears are per event, not per pod.** The memo is emptied at the
    /// substep's two index rebuilds and after **every** `StartupComplete` and
    /// **every** `Impact` the due loop applies, whoever started up or was hit,
    /// because either can move a pod's power or wreck and the memo does not ask
    /// which. So a unit is folded at most once per substep only while the
    /// substep resolves neither; under fire the count grows with the impacts in
    /// the substep times the units asked again after each, and a regression in
    /// it can be the fight getting hotter as well as the memo getting worse.
    ///
    /// Zero for every match in which nothing carries a pod, because the scan does
    /// not run at all then — which is the cost claim this counter exists to make
    /// checkable rather than assumed. `harness.max_jam_checks_per_pulse` is
    /// asserted against the worst wake of it.
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
        self.horizon_probes = self.horizon_probes.saturating_add(other.horizon_probes);
        self.share_ops = self.share_ops.saturating_add(other.share_ops);
        self.jam_checks = self.jam_checks.saturating_add(other.jam_checks);
        self.detected = self.detected.saturating_add(other.detected);
        self.classified = self.classified.saturating_add(other.classified);
        self.lost = self.lost.saturating_add(other.lost);
    }
}

/// One journalled event of a substep: its arena timestamp, what happened, and
/// every draw it made, in the order it made them — empty for most events, one
/// for a sensor classification or a dispersion miss, two for a hit and for a
/// `refused` miss written after the facet draw (16b cut M.3, 16c `T-07`:
/// [`crate::arena::ledger::LedgerEvent::draws`] has the whole list).
pub type Notice = (i64, EventKind, Vec<(String, u64)>);

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

/// **What steps 1 to 3 of one pass produced, and the fold they were decided
/// under** (16b cut D.4, `B-22`).
///
/// [`Arena::detections`] made the jamming fold for its own two terms and then
/// dropped it, so [`Arena::sensor_pass`] had no way to say whether the platform's
/// receiver was degraded except by calling [`Arena::jamming_at`] a **second**
/// time — which would re-run the index query and bump `jam_checks` once per unit
/// in range, doubling a perception counter the harness prints. So the fold is
/// carried out of the function that already made it, and no counter moves.
///
/// **16b cut U.2** moved the making of it up a level: `sensor_pass` takes the
/// fold through [`Arena::jam_fold_of`] and hands it to `detections`, which
/// recovers it with the observer's filter; this is still where the recovered
/// value leaves.
struct Sweep {
    /// The candidates that passed every test.
    found: Vec<Detection>,
    /// The jamming the observer was standing in, **recovered** by its own filter —
    /// the value [`Jamming::budget`] and [`Jamming::scale`] were read from.
    /// `None` is no live pod in reach.
    jam: Option<Jamming>,
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
        //
        // **16b cut U.2** (16c `T-29`): the observer's jamming fold comes out of
        // the per-substep memo, and it is taken only once `detections` is certain
        // to run — the same four conditions it returns early on, checked first
        // here. A fold taken for a wreck or a sightless hull would count
        // `jam_checks` that no pass before this cut ever counted, and would
        // fill the memo for a pass that never reads it.
        let fold = if self.runs_a_sweep(observer) {
            self.jam_fold_of(observer, counters)
        } else {
            None
        };
        let sweep = self.detections(observer, fold, counters);

        // ---- the platform's own receiver (16b cut D.4, `B-22`) ----
        //
        // Read off the fold `detections` has just handed back, and never by asking
        // `jamming_at` again: a second call would re-run the index query and count
        // `jam_checks` twice. See [`Sweep`].
        //
        // A pass that did not run at all writes `None` with **no event and no
        // notice**: a wreck's receiver is off rather than clear, and a wreck is
        // told nothing (doc 16 §3.3). That is a silent move of hashed state, which
        // is exactly why [`UnitState::receiver`] is folded into the state hash.
        let next_receiver = sweep.as_ref().and_then(|sweep| {
            let max = self.unit(observer)?.sensor.as_ref()?.max_attenuation;
            let jam = sweep.jam?;
            let budget = jam.budget(max);
            // A filter that recovered the whole bubble leaves a pass that is the
            // unjammed pass to the centimetre, so the receiver is not degraded:
            // saying otherwise would be telling a pilot about a pod their own
            // machine has entirely answered.
            if budget == max && jam.signature_permille == FULL_SIGNATURE_PERMILLE {
                return None;
            }
            Some(Receiver {
                budget,
                budget_max: max,
                reach_permille: u16::try_from(jam.signature_permille).unwrap_or(u16::MAX),
            })
        });
        let had_receiver = self.unit(observer).and_then(|state| state.receiver);
        if had_receiver != next_receiver {
            if let Some(state) = self.unit_mut(observer) {
                state.receiver = next_receiver;
            }
            // The **transition** and not the numbers: a pilot driving deeper into
            // one bubble is already degraded and has nothing new to be told.
            if sweep.is_some() && had_receiver.is_some() != next_receiver.is_some() {
                journal.push((
                    t_ms,
                    EventKind::Receiver {
                        unit: observer,
                        degraded: next_receiver.is_some(),
                    },
                    Vec::new(),
                ));
            }
        }
        let found = sweep.map(|sweep| sweep.found);

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
                            vec![(label.encode(), value)],
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
                    //
                    // **16b cut D.4** (`B-21`): and the one site
                    // [`ShadowContact::numbered`] is set. It records that **this
                    // hold** was opened under a provisional number, which is what
                    // the `identified` line's `was` has to be read off: the book
                    // is permanent and a target this platform numbered in an
                    // earlier hold and now received over the mesh is not a rename
                    // of anything the pilot is looking at.
                    let mut opened =
                        ShadowContact::sighted(target, Classification::Unknown, t_ms, seen_at);
                    if !self.sensors_cfg.designate_on_detection {
                        self.designators.assign(observer, target);
                        opened.numbered = true;
                    }
                    self.contacts.insert(observer, id, opened);
                    journal.push((
                        t_ms,
                        EventKind::Detected {
                            viewer: observer,
                            contact: id,
                            target,
                        },
                        Vec::new(),
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
                Vec::new(),
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

    /// **The one place the arena's perception traces** (16b cut J.1; doc 16 §3.2
    /// "Line of sight", §3.6 step 2): detection's step 2 in `detections` and the
    /// mesh's line in [`Arena::share_pass`], each one trace and its cells counted
    /// on `counters` (doc 16 §2 W4).
    ///
    /// It answers [`mush_atlas::sight_only`]'s sight and does not walk
    /// [`mush_atlas::trace_cells`]: the answer is the same [`Sight`] to the byte — the same cells in the same order, the same
    /// height at every step and the same per-cell rule, drawn from the line's
    /// canonical end — but with nothing allocated and nothing read past the first
    /// blocker, and the only thing perception reads off a trace is its sight and
    /// its cell count. The early stop is the O lens's one exception, taken because
    /// the pass's cost already depends on the hidden field before any line is
    /// walked (the index cull and the thermal cull decide which lines are traced
    /// at all) and no reader of the arena can time one line; see the atlas
    /// module's head. So [`PerceptionCounters::cell_probes`] counts the interior
    /// cells read up to the blocker, and every outcome is what it was.
    ///
    /// The fire path in `damage.rs` is the arena's other trace and stays on
    /// `trace_cells`, because its readers — the shot, the reflex's engage check
    /// and the cover readings — want `target_cover` and the attenuation from the
    /// shooter's own end, which `sight_only` does not answer.
    ///
    /// **16b cut J.2**: the trace goes through the terrain's own horizon index
    /// ([`crate::arena::state::ArenaTerrain::sight`]), which proves a run of the
    /// line clear without reading it, proves it blocked by reading one cell, and
    /// walks the rest under the same per-cell rule — `sight_only`'s answer to the
    /// byte, with fewer cells read. The cells it read go to
    /// [`PerceptionCounters::cell_probes`] and the blocks to
    /// [`PerceptionCounters::horizon_probes`]. The index caches terrain and
    /// nothing observed, so doc 16 §3.2's *no cross-observer cache* still holds.
    pub fn sight(&self, from: Eye, to: Eye, counters: &mut PerceptionCounters) -> Sight {
        let seen = self.definition.terrain.sight(from, to);
        counters.traces = counters.traces.saturating_add(1);
        counters.cell_probes = counters
            .cell_probes
            .saturating_add(u64::from(seen.cell_probes));
        counters.horizon_probes = counters
            .horizon_probes
            .saturating_add(u64::from(seen.horizon_probes));
        seen.sight
    }

    /// Steps 1 to 3 of doc 16 §3.6, over one snapshot of the units and the index.
    ///
    /// `None` when the observer runs no pass at all: it is not in this match, it is
    /// a wreck, it carries no sensor, or its pose is off the window.
    ///
    /// **16b cut D.4** made the answer a [`Sweep`] rather than the vector alone, so
    /// that the jamming fold this function makes for its own two terms leaves with
    /// it. Nothing else changed: the fold is the same one value, made once, and
    /// [`Arena::jamming_at`] is still called exactly once a pass.
    ///
    /// **16b cut U.2** (16c `T-29`) hands the fold in: `fold` is the observer's
    /// raw [`Jamming`] off [`Arena::jam_fold_of`] — `None` for no live pod in
    /// reach — and this function recovers it with the observer's own filter
    /// exactly as it did when it made the fold itself. The caller takes it only
    /// when [`Arena::runs_a_sweep`] says this function will not return early, so
    /// the scan is counted where it was counted before.
    fn detections(
        &self,
        observer: ObjId,
        fold: Option<Jamming>,
        counters: &mut PerceptionCounters,
    ) -> Option<Sweep> {
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
        // **16b cut M.4** (`B-55`, `T-26`): at the shipped 1.0 the circle is
        // exact, because no signature factor exceeds 1.0 (`signature_permille`
        // caps it there); and it is floored at 1.0 here as well as refused below
        // it at boot, so the cull can never be shorter than the range test.
        let cull_cm = max_range_cm
            .saturating_mul(u64::from(
                self.sensors_cfg
                    .max_signature_factor_permille
                    .max(FULL_SIGNATURE_PERMILLE),
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
        // **16b cut U.2**: the fold is the caller's, out of the per-substep memo.
        let jam = fold.map(|jam| jam.recovered(watcher.eccm));
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
            // Step 2: the atlas trace over the arena's own cell grid, through
            // the arena's one door ([`Arena::sight`], 16b cut J.1).
            let sight = self.sight(from, to, counters);
            if !sees(&sight, attenuation) {
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
        Some(Sweep { found, jam })
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
    /// Four things stop a pod radiating, all read at scan time rather than baked
    /// in at check-out:
    ///
    /// - **distance**: the observer must be inside *that* pod's radius;
    /// - **a wreck**: `destroy_unit` revokes a unit's capabilities (doc 16 §3.3),
    ///   and a burnt-out hull's transmitter is one of them;
    /// - **power**: a pod is an *emitter* (W3's own distinction), so a hull that has
    ///   not started up jams nobody. This is the one place the pass reads a unit's
    ///   power for a reason other than the target's heat, and it is deliberate: the
    ///   sight of a cold tank still works, because optics are optics, and its pod
    ///   does not, because a transmitter is not;
    /// - **the switch** (16b cut O.2, 16c N12): a pod its pilot has put to `EMCON
    ///   off` ([`UnitState::ecm_on`] false) radiates nothing. **This one clause
    ///   governs detection and both ends of the mesh at once**, because every
    ///   pass and every share reads the jamming through [`Arena::jam_fold_of`]
    ///   (16b cut U.2) and that memo is this function's: a silent pod can neither
    ///   blind a tank nor stop it phoning home, and nothing else in the crate
    ///   needs to know the switch exists.
    ///
    /// A pod is inside its own bubble at range zero, so a unit that carries one
    /// jams itself. There is no exception: the rule is about where the *looker* is
    /// — which is why the switch exists at all.
    ///
    /// **Who calls it directly** (16b cut U.2, 16c `T-29`). It stays the pure
    /// primitive — a pose in, a fold out, the scan counted — and exactly two
    /// kinds of caller use it: [`Arena::jam_fold_of`], the substep's memo
    /// every pass and share reads through, and tests that want the arithmetic at
    /// a pose no unit stands on. A pass that called it directly would scan again
    /// what the memo already holds and count `jam_checks` the harness bounds.
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
            if pod.limits.wreck || pod.power != Power::Online || !pod.ecm_on {
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

    /// **The jamming one unit is standing in, folded once between two clears**
    /// (16b cut U.2, 16c `T-29`; doc 16 §9 item 4): [`Arena::jam_fold`]'s entry
    /// for `unit` when it has one, and otherwise [`Arena::jamming_at`] of the
    /// unit's pose, computed once and stored. The clears are the substep's two
    /// index rebuilds, every `StartupComplete` and `Impact` the due loop
    /// applies, and every flip of a pod's switch
    /// ([`crate::arena::intents::resolve_emcon`], 16b cut O.2; see
    /// [`Arena::jam_fold`]).
    ///
    /// The answer is the **raw** fold — the pods alone, no filter — and each
    /// caller applies [`Jamming::recovered`] with its own unit's filter, exactly
    /// as it did before the memo existed; the memo is a cache of a pure function
    /// of the state it is keyed on, so it changes no answer anywhere.
    ///
    /// `jam_checks` is counted on a miss only, which is the whole of what the memo
    /// buys: a unit folded by its own pass, by its own share and as a mate in every
    /// teammate's share inside one substep scans its bubble once. A match with no
    /// pod (`jam_radius_cm == 0`, every match the game ships) returns `None`
    /// before the memo is touched, so a stock match is bit-identical to the one
    /// cut X.4 left in every count. A unit that is not in the match is `None` and
    /// is not stored.
    pub fn jam_fold_of(
        &mut self,
        unit: ObjId,
        counters: &mut PerceptionCounters,
    ) -> Option<Jamming> {
        if self.jam_radius_cm == 0 {
            return None;
        }
        if let Some(fold) = self.jam_fold.get(&unit) {
            return *fold;
        }
        let fold = {
            let state = self.unit(unit)?;
            self.jamming_at(&state.pose, counters)
        };
        self.jam_fold.insert(unit, fold);
        fold
    }

    /// **Whether [`Arena::detections`] runs for `observer` at all** (16b cut
    /// U.2): it is in the match, it is not a wreck, it carries a sight and its
    /// pose is on the window — the four early returns `detections` makes, asked
    /// first so the caller folds the observer's jamming only for a pass that
    /// reads it.
    fn runs_a_sweep(&self, observer: ObjId) -> bool {
        self.unit(observer).is_some_and(|watcher| {
            !watcher.limits.wreck && watcher.sensor.is_some() && self.eye_of(watcher).is_some()
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
    ///
    /// **17b cut M1b.1 — the answer** (doc 17 §5's *every sensor pass that
    /// refreshes the conditioned contact's plot*): the `(receiver, contact)`
    /// pairs this share inserted or refreshed — the plot written, or the class
    /// told apart — in the order it wrote them, **collected only for a receiver
    /// that holds a condition slot** (`Arena::conditions`). A share into a side
    /// with no slot answers an empty vector and allocates nothing for it; the
    /// due loop's `Share` arm evaluates each receiver whose lock is among them.
    pub fn share_pass(
        &mut self,
        sharer: ObjId,
        t_ms: i64,
        counters: &mut PerceptionCounters,
        journal: &mut Vec<Notice>,
    ) -> Vec<(ObjId, ContactId)> {
        let Some(sharing) = self.unit(sharer) else {
            return Vec::new();
        };
        if sharing.limits.wreck {
            return Vec::new();
        }
        let Some(sensor) = sharing.sensor.as_ref() else {
            return Vec::new();
        };
        if sensor.share_range_cells == 0 {
            return Vec::new();
        }
        let Some(from) = self.eye_of(sharing) else {
            return Vec::new();
        };
        let attenuation = sensor.max_attenuation;
        let cell_cm = u64::from(self.definition.terrain.cell_cm());
        let reach = u32::try_from(u64::from(sensor.share_range_cells).saturating_mul(cell_cm))
            .unwrap_or(u32::MAX);
        // Copied out of the borrow: the folds below go through the memo, which
        // takes the arena mutably.
        let side = sharing.team;
        let sharer_eccm = sharing.eccm;
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
            return Vec::new();
        }
        // **16b cut Q.6**: the pods the *sharer* is standing among, folded once for
        // the whole pass and not once per mate — the observer's own fold in
        // `detections` is read the same way, and for the same reason. The sharer's
        // own filter takes its share back out before either term is read, exactly
        // as an observer's does. A match with no pod in it has `jam_radius_cm == 0`
        // and never reaches the index or a counter, so a stock share is
        // bit-identical to the one cut F.3 left.
        //
        // **16b cut U.2** (16c `T-29`): through the per-substep memo, and below the
        // return above rather than over it — a sharer with nothing to say scans
        // nothing. The share is applied in the same substep as the pass that
        // scheduled it, so the fold is almost always the one that pass took.
        let sharer_jam = self
            .jam_fold_of(sharer, counters)
            .map(|jam| jam.recovered(sharer_eccm));
        let sharer_reach = sharer_jam.map_or(u64::from(reach), |jam| jam.scale(u64::from(reach)));
        // Who to say it to, decided against the same snapshot: the index query and
        // the side and wreck tests, collected first (16b cut U.2) because each
        // mate's fold below goes through the memo, which takes the arena mutably.
        let candidates: Vec<ObjId> = self
            .unit(sharer)
            .map(|sharing| {
                self.index
                    .in_range(&sharing.pose, reach, &self.units)
                    .into_iter()
                    .filter(|id| *id != sharer)
                    .filter(|id| {
                        self.unit(*id)
                            .is_some_and(|mate| mate.team == side && !mate.limits.wreck)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut mates: Vec<ObjId> = Vec::new();
        for id in candidates {
            let Some((to, mate_eccm)) = self
                .unit(id)
                .and_then(|mate| Some((self.eye_of(mate)?, mate.eccm)))
            else {
                continue;
            };
            counters.share_ops = counters.share_ops.saturating_add(1);
            // The mesh's line, through the same door as detection's (16b cut
            // J.1), and before the mate's fold as the trace always was.
            let sight = self.sight(from, to, counters);
            // **16b cut Q.6**: and the pods the *receiver* is standing among, with
            // its own filter against them. A link has two ends and a pod sitting on
            // either one cuts it, so the **tighter** of the two answers governs:
            // the smaller attenuation budget for the line, and the shorter scaled
            // reach for the range. Doc 16 W6 — *tactical comms are LOS mesh jammed
            // by the same ECM* — is the whole rule, and "the same" is meant
            // literally: this is `jamming_at`, `recovered`, `budget` and `scale`,
            // the four the sensor pass uses, so detection and the mesh cannot jam
            // differently. **16b cut U.2**: the mate's fold through the memo, in
            // the same place — after the trace, because the budget it feeds is the
            // line test's.
            let mate_jam = self
                .jam_fold_of(id, counters)
                .map(|jam| jam.recovered(mate_eccm));
            let budget = sharer_jam
                .map_or(attenuation, |jam| jam.budget(attenuation))
                .min(mate_jam.map_or(attenuation, |jam| jam.budget(attenuation)));
            if !sees(&sight, budget) {
                continue;
            }
            let link_reach = sharer_reach
                .min(mate_jam.map_or(u64::from(reach), |jam| jam.scale(u64::from(reach))));
            let apart = match (self.unit(sharer), self.unit(id)) {
                (Some(sharing), Some(mate)) => range_cm(&sharing.pose, &mate.pose),
                _ => continue,
            };
            if apart > link_reach {
                continue;
            }
            mates.push(id);
        }
        let seed = self.seed;
        // **17b cut M1b.1** (doc 17 §5; 17b §1(c), (d)): the (receiver, contact)
        // pairs this share inserted or refreshed — collected ONLY for a receiver
        // holding a condition slot, so a production share (no slot anywhere)
        // pays one map miss per receiver and allocates nothing for it. The due
        // loop's `Share` arm evaluates each receiver whose LOCK is among them.
        let mut refreshed: Vec<(ObjId, ContactId)> = Vec::new();
        for mate in mates {
            let watched = !self.conditions.is_empty_for(mate);
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
                        // 17b cut M1b.1: whether this share wrote the row a
                        // slot's warrant reads — its plot or its class.
                        let mut wrote = false;
                        if shared.own.at_ms > held.last_seen_ms {
                            wrote = true;
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
                                Vec::new(),
                            ));
                            counters.classified = counters.classified.saturating_add(1);
                            wrote = true;
                        }
                        if watched && wrote {
                            refreshed.push((mate, id));
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
                                // **16b cut D.4** (`B-21`): a mesh arrival was
                                // never a number on this platform's scope, so
                                // its classification is not a rename and the
                                // `identified` line is not said.
                                numbered: false,
                                // **16b cut E.2** (`B-45`): a share carries a
                                // sighting and never a kill. The sharer may have
                                // watched the target die; that is one fact its
                                // own gunner read, and doc 16 §2 W6 lets the
                                // mesh carry what its SENSORS took. The receiver
                                // holds the track until the track expires.
                                known_dead: false,
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
                            Vec::new(),
                        ));
                        journal.push((
                            t_ms,
                            EventKind::Classified {
                                viewer: mate,
                                contact: id,
                                target,
                                team,
                            },
                            Vec::new(),
                        ));
                        counters.detected = counters.detected.saturating_add(1);
                        counters.classified = counters.classified.saturating_add(1);
                        if watched {
                            refreshed.push((mate, id));
                        }
                    }
                }
            }
        }
        refreshed
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
    ///
    /// **16b cut E.2 (16c `B-45`)** replaced that wreck filter with
    /// [`Arena::contact_reads_dead`], which is the same sentence read the way
    /// every other reader of this store reads it: the live flag on the target is
    /// hidden truth, and a tempo that dropped the moment a hull burned — through
    /// a wall, for a platform whose track is three seconds stale — would be
    /// exactly the leak this function's own third bullet exists to close. A
    /// contact whose **unit has left the match** is still dropped at once: there
    /// is no unit for it to be a fact about.
    #[must_use]
    pub fn contact_within_alert_range(&self) -> bool {
        let reach = u64::from(self.alert_range_cm);
        if reach == 0 {
            return false;
        }
        self.units.iter().any(|viewer| {
            self.contacts.of(viewer.id).any(|(_, held)| {
                self.unit(held.target).is_some()
                    && !self.contact_reads_dead(held)
                    // 16b cut U.1 (16c `T-24`): `range < reach` squared, as
                    // `range <= reach − 1`; `reach` is at least 1 here, the
                    // nought having answered `false` above.
                    && held.seen_at.within_cm(&viewer.pose, reach - 1)
            })
        })
    }

    /// **Whether the platform holding this contact has any reason to believe its
    /// target is dead** (16b cut E.2, `B-45`; 16c §5's Q11), which is the one
    /// question every reader that used to look at `target.limits.wreck` was
    /// really asking.
    ///
    /// Two ways, and they are the only two a platform can have earned:
    ///
    /// * [`ShadowContact::known_dead`] — this platform's own gunner killed it,
    ///   which is the same fact its pilot is sent the `Kill` line about;
    /// * its **reading** shows [`crate::arena::condition::ConditionBand::Destroyed`]
    ///   on a section that kills the target when it goes
    ///   ([`crate::arena::state::UnitState::fatal_sections`]) — a close look, or
    ///   a section its own gunner watched come apart, or a teammate's share of
    ///   either.
    ///
    /// Reading the **target's** static fit is not a leak: a platform that holds
    /// a reading at all has already been sent that target's section vocabulary
    /// (`Frame::scan_tables`), so nothing here is a word the holder was not
    /// given.
    ///
    /// **A target that has left the match answers `false`, whichever way the
    /// hold would otherwise have read** — the lookup is the first line of the
    /// body and not the second, so the sentence is true of `known_dead` too.
    /// There is no unit left for the fact to be about, and every caller tests
    /// `self.unit(target)` separately anyway, because a unit that is *gone* is a
    /// different fact from a unit that is *dead*: *gone* drops a target at once
    /// and *dead* is what this answers.
    ///
    /// **What it costs.** A machine may hold a dead target for up to
    /// `sensors.lost_after_passes × sensors.pass_ms` (three seconds as shipped)
    /// plus the notice hysteresis, and may spend a burst on it — it fires, the
    /// round is re-validated against the wreck and it reads a miss. That is what
    /// a stale track costs, and it costs a pilot in the same seat exactly the
    /// same thing: the row stays on `CONTACTS` until it expires. Doc 16 §1 and
    /// §5 do not make an exception for a reader that happens to be a machine.
    #[must_use]
    pub fn contact_reads_dead(&self, held: &ShadowContact) -> bool {
        // *Gone* first: a contact whose unit has left the match is not a fact
        // about anything, so neither the kill this platform watched nor the
        // reading it holds is a thing to answer with. It is the doc's sentence
        // in the order the doc says it.
        let Some(target) = self.unit(held.target) else {
            return false;
        };
        held.known_dead
            || held.seen_condition.is_some_and(|reading| {
                reading.condition.destroyed_mask() & target.fatal_sections != 0
            })
    }

    /// The first [`DueKind::SensorPass`] of **every unit**, at
    /// **`stagger_ms(seed, id, pass_ms)`** of arena time (doc 16 §3.6: *staggered
    /// by unit id*).
    ///
    /// **16b cut F.3** widened it from the units that carry a sensor. The pass is
    /// also where a platform's contacts expire, and a platform with no sight of
    /// its own is exactly the platform a teammate can hand contacts to: it was
    /// never scheduled a pass, so nothing ever let go of what the mesh gave it. A
    /// unit that detects nothing pays one short event a cadence — the read half
    /// returns before the index is touched — which is the cheapest place to put
    /// an expiry that must run for everyone and must not become a per-pulse sweep.
    ///
    /// **16b cut U.1** (16c `T-25`): the stagger is
    /// [`crate::arena::rng::stagger_ms`], the unit's number mixed with the match
    /// seed modulo the cadence. It was the object number modulo the cadence, and a
    /// match's numbers are contiguous, so 300 units put every first pass inside
    /// the first 300 ms of each second and the worst pulse carried every pass. The
    /// mixed offset still needs no state and is the same on a replay and at every
    /// tempo (the cadence is `pass_ms`, never the period), and it spreads a
    /// match's passes across the whole cadence, which is what keeps the worst
    /// pulse's trace count near the average (doc 16 §2 W4). Two units may share
    /// an offset; nothing depends on their not doing so — same-millisecond passes
    /// apply in event-sequence order like any other due events.
    pub fn schedule_first_passes(&mut self) {
        let cadence = i64::from(self.sensors_cfg.pass_ms.max(1));
        let ids: Vec<ObjId> = self.units.iter().map(|unit| unit.id).collect();
        for id in ids {
            let offset = crate::arena::rng::stagger_ms(self.seed, id, cadence);
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
///
/// **17b cut M1a.3** made it `Clone` and not `Copy`: [`FrameContact::fatal`] is a
/// list of section words.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// `crate::match_::cockpit::contact_band`'s four words *and* into metres
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
    /// **Which face the contact was showing when it was last seen** (17b cut
    /// M1a.3; doc 17 §3, D-UM-5): [`crate::arena::resolution::facet_band_seen`]
    /// off the contact's own **plot** and the platform's own pose — advice
    /// from the plot, as the row's range and bearing are, never the target's
    /// live heading. On every row of every hull, armed or not.
    pub aspect: Aspect,
    /// **Whether this platform has any reason to believe the contact is dead**
    /// (17b cut M1a.3; doc 17 §3, D-UM-5), or `None` on a hull with **no
    /// weapons station** — the field is absent there, not `false` (doc 17 §3:
    /// *weapons station only*; §22's italic line).
    ///
    /// Two ways, and they are the only two a platform can have earned: this
    /// platform's own gunner's kill ([`ShadowContact::known_dead`]), or a held
    /// reading that shows destroyed a section the **target chassis's
    /// definition** marks fatal — content a pilot can read off
    /// [`FrameContact::fatal`], never the target's live wreck flag. It is not a
    /// call of [`Arena::contact_reads_dead`] (D-UM-5), and it is
    /// value-identical to it wherever a reading is held: a test pins the
    /// equality, so doc 17 §17's divergence (4) is measured honestly in 17b cut
    /// M1a.6 and is expected to count nought — declared, not hidden.
    pub reads_dead: Option<bool>,
    /// **The section words of the target's chassis whose destruction kills
    /// it**, in section order (17b cut M1a.3; departure (j)), present only
    /// beside a held reading ([`FrameContact::condition`] is `Some`) and
    /// `None` without one.
    ///
    /// It is what lets a pilot read the same rule [`FrameContact::reads_dead`]
    /// applies: which sections a reading would have to show destroyed. It is
    /// published only where the row already carries the chassis's section
    /// vocabulary ([`crate::arena::step::Frame::scan_tables`]), so it names no
    /// word a pilot was not given — and it is read off the chassis's static
    /// definition and profile, the same content
    /// [`crate::arena::state::UnitState::fatal_sections`] is built from.
    pub fatal: Option<Vec<SmolStr>>,
}

/// **Which face a contact was presenting when it was last seen** (17b cut M1a.3;
/// doc 17 §3, D-UM-5, D-UM-18's vocabulary).
///
/// One of [`crate::arena::resolution::DIRECT_BANDS`]: the band
/// [`crate::arena::resolution::facet_band_seen`] answers for the contact's
/// plot and the viewer's own pose. **Two words for one value, on purpose.** The
/// keyframe carries the **facet** word ([`Aspect::as_str`]: `front`, `left`,
/// `right`, `rear`) because that is what a mind reads and what 17b cut M1b's
/// `aspect = side,rear` mask tests against; the `CONTACTS` row prints the
/// **side** word ([`Aspect::side_word`]: `front`, `side`, `side`, `rear`)
/// because which flank is a fact a gunner does not need on a 120-column row.
///
/// **17b cut M1b.1**: serde-able as its facet word ([`Aspect::as_str`]), for
/// the slot notices a mind's buffer carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aspect {
    /// The contact's front was toward the platform.
    Front,
    /// Its left flank.
    Left,
    /// Its right flank.
    Right,
    /// Its rear.
    Rear,
}

impl Aspect {
    /// The facet word — what the keyframe carries and a mind reads.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Aspect::Front => "front",
            Aspect::Left => "left",
            Aspect::Right => "right",
            Aspect::Rear => "rear",
        }
    }

    /// One byte: `0` front, `1` left, `2` right, `3` rear.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Aspect::Front => 0,
            Aspect::Left => 1,
            Aspect::Right => 2,
            Aspect::Rear => 3,
        }
    }

    /// **The aspect a band word names** — the word
    /// [`crate::arena::resolution::facet_band_seen`] answers, which is always one
    /// of [`crate::arena::resolution::DIRECT_BANDS`]. Anything else answers
    /// [`Aspect::Front`], and is unreachable: the sector rule cannot produce
    /// another word, and `front` is the same fallback that rule itself takes.
    #[must_use]
    pub fn from_band(word: &str) -> Aspect {
        match word {
            "left" => Aspect::Left,
            "right" => Aspect::Right,
            "rear" => Aspect::Rear,
            _ => Aspect::Front,
        }
    }

    /// The side word the `CONTACTS` row prints: `front`, `side`, `side`, `rear`.
    #[must_use]
    pub const fn side_word(&self) -> &'static str {
        match self {
            Aspect::Front => "front",
            Aspect::Left | Aspect::Right => "side",
            Aspect::Rear => "rear",
        }
    }
}

/// **One weapon's advertised chance against one contact** (17b cut M1a.3; doc 17
/// §3, D-UM-5), per mille — [`Arena::resolve_seen`] over
/// [`Arena::aim_at`], so advice off the plot exactly as a lock's percentage is.
///
/// Built only for a **classified** contact (the silhouette table
/// `resolve_seen` reads is the target's, and an unclassified contact must not
/// leak it) and only on a hull with a weapons station; see
/// `Arena::frame_odds` for which contacts and in what order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameOdds {
    /// The row it is about.
    pub contact: ContactId,
    /// The weapon's id.
    pub weapon: SmolStr,
    /// The chance, per mille.
    pub p_permille: u32,
}

/// **A standing order's objective, as a hull's own frame carries it** (17b cut
/// M1a.3; doc 17 §3 "Two more fields", D-UM-15).
///
/// [`crate::deliberation::order::Objective`] with every contact **re-keyed to
/// the hull's own** [`ContactId`] — the row it holds the named target under, or
/// `None` when it holds none — so a mind reading its order can only ever name a
/// row of its own scope (`Arena::rekey_order`). The words and the bytes are
/// `Objective`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "objective")]
pub enum MindObjective {
    /// Stop and hold.
    Hold,
    /// Make for this window cell.
    Advance {
        /// `(column, row)`.
        to: (u16, u16),
    },
    /// Fight, preferring this row of the hull's own scope.
    Engage {
        /// The hull's own row for the ordered target, or `None`.
        target: Option<ContactId>,
    },
    /// Break off toward the own entry zone.
    Withdraw,
    /// Hold and face this bearing.
    Screen {
        /// Millidegrees clockwise from north.
        facing_mdeg: u32,
    },
    /// Keep station on a friend, by the hull's own row for it.
    Support {
        /// The hull's own row for the charge, or `None` (never the hull itself).
        of: Option<ContactId>,
    },
}

impl MindObjective {
    /// The wire word — [`crate::deliberation::order::Objective::as_str`]'s.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            MindObjective::Hold => "hold",
            MindObjective::Advance { .. } => "advance",
            MindObjective::Engage { .. } => "engage",
            MindObjective::Withdraw => "withdraw",
            MindObjective::Screen { .. } => "screen",
            MindObjective::Support { .. } => "support",
        }
    }

    /// The byte — [`crate::deliberation::order::Objective::code`]'s.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            MindObjective::Hold => 1,
            MindObjective::Advance { .. } => 2,
            MindObjective::Engage { .. } => 3,
            MindObjective::Withdraw => 4,
            MindObjective::Screen { .. } => 5,
            MindObjective::Support { .. } => 6,
        }
    }
}

/// **The standing order a hull is under, as its own frame carries it** (17b cut
/// M1a.3; doc 17 §3 "Two more fields", D-UM-15 as decided).
///
/// Re-keyed by `Arena::rekey_order`; read by `Arena::frame_order`, which
/// answers `None` for a piloted hull — so every frame a pilot is sent today
/// carries `"order": null`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameOrder {
    /// The squad it was written for, rendered (`red-1`).
    pub squad: SmolStr,
    /// What the squad is told to do, re-keyed to the hull's own rows.
    pub objective: MindObjective,
    /// The order forbids firing.
    pub hold_fire: bool,
    /// The furthest band the order lets the hull shoot from.
    pub max_range_band: Option<crate::deliberation::order::RangeBand>,
    /// The arena millisecond the order stops standing at.
    pub expires_ms: i64,
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

impl FrameCondition {
    /// **Whether this reading would print the same as `other`** (16b cut U.2,
    /// 16c `T-30`): the table and the bands equal, and the look's age equal at
    /// every resolution a surface derives from it — [`age_key`] at `pass_ms`.
    ///
    /// The age is carried on the keyframe to the millisecond whatever this
    /// answers; what it decides is whether a frame is *sent*, never what a pilot
    /// may read off the one they hold.
    ///
    /// **Both sides are destructured with no `..`**, so a field added to this
    /// struct breaks the build here until someone decides how it is compared:
    /// a field this method forgot would be a figure a parked pilot is never
    /// sent again, with no test to notice (the stale-frame lie `T-30` is
    /// careful to avoid).
    #[must_use]
    pub fn same_as(&self, other: &FrameCondition, pass_ms: i64) -> bool {
        let FrameCondition {
            table,
            bands,
            age_ms,
        } = self;
        let FrameCondition {
            table: their_table,
            bands: their_bands,
            age_ms: their_age_ms,
        } = other;
        table == their_table
            && bands == their_bands
            && age_key(*age_ms, pass_ms) == age_key(*their_age_ms, pass_ms)
    }
}

impl FrameContact {
    /// **Whether this row would print the same as `other`** (16b cut U.2, 16c
    /// `T-30`; doc 16 §3.1's latest-wins slot, 16c Q27).
    ///
    /// Every field equal — the id, the designator and its alias, the class, the
    /// bearing, the range and the cell — except the plot's age, which moves every
    /// millisecond a contact is held and is compared at the resolutions a surface
    /// derives from it instead: [`age_key`] at `pass_ms` (the whole second every
    /// panel prints, and the line client's stale clause), and, on the row the
    /// platform is **locked** on, [`lock_range_step_m`] as well — the lock's
    /// published metres, which move below a second (at 648 ms and about every
    /// 590 ms after). Comparing at whole seconds alone would let a parked, locked
    /// pilot keep reading a 10 m step on a plot 900 ms old, which is the invented
    /// digit `T-20` took out.
    ///
    /// The reading rides the row and is compared by
    /// [`FrameCondition::same_as`] under the same rule.
    ///
    /// **Both sides are destructured with no `..`**, as there: a field added to
    /// the row breaks the build here until its comparison is decided, where the
    /// derived `PartialEq` this replaced was total by construction.
    ///
    /// **17b cut M1a.3**: the aspect, `reads_dead` and `fatal` are compared
    /// exactly. The aspect is a word the row prints and the other two are what
    /// a mind reads; all three move only on a sensor pass, a share or the own
    /// kill, so comparing them costs no frame anybody would not already have
    /// been sent.
    #[must_use]
    pub fn same_as(&self, other: &FrameContact, pass_ms: i64, locked: bool) -> bool {
        let FrameContact {
            id,
            designator,
            alias,
            class,
            bearing_mdeg,
            range_cm,
            age_ms,
            cell,
            condition,
            aspect,
            reads_dead,
            fatal,
        } = self;
        let FrameContact {
            id: their_id,
            designator: their_designator,
            alias: their_alias,
            class: their_class,
            bearing_mdeg: their_bearing_mdeg,
            range_cm: their_range_cm,
            age_ms: their_age_ms,
            cell: their_cell,
            condition: their_condition,
            aspect: their_aspect,
            reads_dead: their_reads_dead,
            fatal: their_fatal,
        } = other;
        id == their_id
            && designator == their_designator
            && alias == their_alias
            && class == their_class
            && bearing_mdeg == their_bearing_mdeg
            && range_cm == their_range_cm
            && cell == their_cell
            && aspect == their_aspect
            && reads_dead == their_reads_dead
            && fatal == their_fatal
            && age_key(*age_ms, pass_ms) == age_key(*their_age_ms, pass_ms)
            && (!locked || lock_range_step_m(*age_ms) == lock_range_step_m(*their_age_ms))
            && match (condition, their_condition) {
                (Some(mine), Some(theirs)) => mine.same_as(theirs, pass_ms),
                (None, None) => true,
                _ => false,
            }
    }
}

/// **The age of a plot or a look, as the surfaces that print it read it** (16b
/// cut U.2, 16c `T-30`, Q27): `(age_ms / 1000, age_ms > pass_ms)`.
///
/// This is the list of every surface that derives something from an age, and
/// the next surface that does is added here, so a frame comparison cannot fall
/// behind it:
///
/// 1. **the whole second** — the web's and the TUI's `age_s`, the line client's
///    `CONTACTS` column and the seconds `stale_clause` prints;
/// 2. **the stale clause** — `crate::match_::cockpit::stale_clause` says `,
///    plotted N s ago` once the age is past one sensor pass, `pass_ms` read as
///    at least one exactly as that function reads it.
///
/// The locked row's third surface, the lock's range step, is
/// [`lock_range_step_m`]'s own and is compared beside this by
/// [`FrameContact::same_as`], because it applies to one row and not to every
/// age.
#[must_use]
pub fn age_key(age_ms: i64, pass_ms: i64) -> (i64, bool) {
    (age_ms / 1_000, age_ms > pass_ms.max(1))
}

/// **How coarsely a contact's range is reported**, metres (16b cut F.3; moved
/// here from `crate::match_::cockpit`, which re-exports it, by 16b cut U.2).
///
/// Doc 16 §3.6 gives `CONTACTS` a *range band* and no range, and cut S.2 read
/// that strictly. In play it left a pilot with no distance information whatever:
/// the band could only ever read `close` (see
/// `crate::match_::cockpit::contact_band`), so from acquisition to kill every row
/// said the same word whether the other tank was 600 m away or 40 m away.
///
/// A passive sight giving a coarse range is a **playtest-usability decision**,
/// recorded here for the owner to revisit: it departs from doc 16 §3.6 in the
/// direction of a pilot being able to act. The rounding is the whole of the
/// coarseness and it lives in this one constant, so tightening or loosening it is
/// one number.
pub const CONTACT_RANGE_STEP_M: u64 = 50;

/// **The finest a *locked* contact's range is ever reported**, metres (16b cut
/// Q.1; 16b cut D.4 made it the floor rather than the step).
///
/// A lock is a fire-control solution: the platform is tracking one return
/// continuously and resolving a hit chance against it every pulse, so the range
/// it can quote about that one contact is better than the range it quotes about
/// the rest of the scope. It is a second step past doc 16 §3.6's band-only
/// wording, recorded here beside the first for the owner to revisit, and it is
/// what makes `RANGE` worth typing after `LOCK`.
///
/// [`lock_range_step_m`] is what a lock's range is actually rounded to: this is
/// the floor it clamps up from, which is what a plot taken this instant is worth.
pub const LOCK_RANGE_STEP_M: u64 = 10;

/// **The fastest thing the catalog fields**, metres a second, rounded up (16b cut
/// D.4, `T-20`).
///
/// 17 m/s is about 60 kph, which is over the stock tank's 54 and over every other
/// hull the game ships. It is the drift term of [`lock_range_step_m`]: the
/// furthest a plot's subject could have got since the plot was taken.
pub const PLOT_DRIFT_M_PER_S: i64 = 17;

/// **How finely a locked contact's range may honestly be reported**, metres,
/// given a plot `age_ms` old (16b cut D.4, `T-20`; 16c §5 Q28: *derived from the
/// plot's age, floor 10 m*).
///
/// **16b cut U.2** moved it here from `crate::match_::cockpit`, which re-exports
/// it: [`FrameContact::same_as`] compares a locked row at this step, and the
/// library does not reach up into the cockpit. Pure integer arithmetic, no I/O;
/// there is still exactly one of it.
///
/// `LOCK_RANGE_STEP_M` was a fixed 10 m, which is finer than a plot one pass old
/// can carry: a target making 60 kph has moved 17 m in the second since the
/// sighting, so a 10 m figure quoted about it is three significant digits of
/// which the last one is invented.
///
/// The step is the drift, `age_ms × `[`PLOT_DRIFT_M_PER_S`]` / 1000`, rounded
/// **up** to a multiple of ten and clamped into
/// `LOCK_RANGE_STEP_M..=CONTACT_RANGE_STEP_M`: a plot up to 647 ms old keeps
/// 10 m, a one-second plot gets 20 m, two seconds gets 40 m, and about two and a
/// half seconds and older reads at the ordinary scope step, because a lock on a
/// plot that stale is not a better fire-control solution than any other row.
///
/// **It publishes nothing new.** The one number it is a function of — the
/// `age_ms` on the contact's own row — is already on the frame the pilot holds,
/// and is the same `1 s` `CONTACTS` prints. It is deliberately **not** priced off
/// `[tacsim.sensors].pass_ms`, which a pilot has never been sent.
///
/// The **metres** are a different matter and a client must not compute them: a
/// contact row publishes `range_m` already rounded to [`CONTACT_RANGE_STEP_M`],
/// so re-rounding that to this step gives the coarse figure back (450 m where the
/// lock reads 440). The lock's own metres are published once, as the frame's
/// `lock_range_m`, and `RANGE` prints the same call of the same function — which
/// is the whole of `T-20`'s *one function* rule.
#[must_use]
pub fn lock_range_step_m(age_ms: i64) -> u64 {
    let drift_m = age_ms.max(0).saturating_mul(PLOT_DRIFT_M_PER_S) / 1_000;
    let step = drift_m.saturating_add(9) / 10 * 10;
    u64::try_from(step)
        .unwrap_or(CONTACT_RANGE_STEP_M)
        .clamp(LOCK_RANGE_STEP_M, CONTACT_RANGE_STEP_M)
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
///
/// **17b cut M1a.5**: serde-able, with every payload of [`FrameNotice`], so a
/// mind's notice buffer is folded as canonical bytes and carried by a snapshot
/// (`crate::arena::mind::MindState::notices`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    /// The platform now holds a contact it did not hold before.
    Acquired,
    /// A contact it holds has been told apart.
    Classified,
    /// A contact it held has gone, and stayed gone (see [`FrameNotice`]).
    Lost,
    /// **A contact the platform had told apart has gone back to being a number**
    /// (16b cut D.4, `B-20`; 16c §5 Q16), said when a loss cancelled inside the
    /// hysteresis window re-inserted the row under a *different* name.
    ///
    /// The flicker itself is still silent — a contact that goes behind a treeline
    /// and comes straight back costs the pilot no lines — but a flicker that costs
    /// the platform its **classification** is not the same event: `LOCK BA` starts
    /// answering `No such contact.` and nothing else would say why.
    ///
    /// The old name is quoted and is **never** made an alias: accepting `BA` for
    /// the row would tell the pilot BA is still out there, which doc 16 §3.6's one
    /// byte-identical refusal exists to hide (16c §6).
    Unidentified,
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
    /// **A round arrived on a section of this platform that was already
    /// destroyed** (16b cut D.2, `B-15`), said to the victim in place of
    /// [`NoticeKind::Struck`] and **at most once every
    /// [`crate::arena::state::SPENT_NOTICE_MS`] per section**.
    ///
    /// The round is journalled exactly as it always was — it is a `hit` with two
    /// noughts — but six identical `armour gone, 0 of 0 inside.` lines in ten
    /// seconds told the pilot nothing the first one had not. The pilot's own
    /// tank is outside the observer rule, so the section is named.
    StruckSpent,
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
    /// **This platform is a wreck** (16b cut D.2, `B-03`), said to its own pilots
    /// off the ledger's `unit_destroyed`.
    ///
    /// Until this cut a pilot whose tank died read the gunner's `… destroyed.`
    /// only if they were the gunner: their own scope simply stopped answering and
    /// every order they typed vanished. It names the pilot's own tank and nothing
    /// else, so it is outside the observer rule entirely.
    OwnDestroyed,
    /// **This platform can neither move nor shoot** (16b cut D.2, `B-03`), said to
    /// its own pilots off the ledger's `unit_disabled`.
    OwnDisabled,
    /// **This platform's own receiver is being interfered with** (16b cut D.4,
    /// `B-22`), said to its own pilots off the ledger's `receiver`.
    ///
    /// The platform's own machine, and nothing about the pod: see [`Receiver`].
    ReceiverDegraded,
    /// **This platform's own receiver is running clean again** (16b cut D.4,
    /// `B-22`).
    ReceiverClear,
    /// **This platform's own jamming pod was switched on** (16b cut O.2, 16c
    /// N12), said to its own pilots off the ledger's `emcon`.
    EmconOn,
    /// **This platform's own jamming pod was switched off** (16b cut O.2, 16c
    /// N12), with what that costs said in the line.
    EmconOff,
    /// **The pilot's `DRIVE` arrived** (17b cut M1a.4; doc 17 §4 D-UM-10): the
    /// motor brought the hull within `ARRIVE_CELLS` of its goal and let go of
    /// it. Said once — the goal is dropped, so the motor's tick finds none.
    DriveArrived,
    /// **No route reaches the pilot's `DRIVE` goal from where the hull stands**
    /// (17b cut M1a.4): the motor dropped the goal and stopped the hull.
    DriveNoRoute,
    /// **A condition slot fired** (17b cut M1b.1 defines it; 17b cut M1b.4
    /// stages it): `Cannon fires at BA by S1 (62%, side).`
    SlotFired,
    /// **A condition slot held a shot** (17b cut M1b.1 defines it; 17b cut M1b.4
    /// stages it): `S1 held the cannon: 54% under your 60%.`
    SlotHeld,
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
            NoticeKind::Unidentified => "unidentified",
            NoticeKind::LockLost => "lock_lost",
            NoticeKind::Locked => "locked",
            NoticeKind::Hit => "hit",
            NoticeKind::Miss => "miss",
            NoticeKind::Kill => "kill",
            NoticeKind::Struck => "struck",
            NoticeKind::StruckSpent => "struck_spent",
            NoticeKind::SectionLost => "section_lost",
            NoticeKind::TargetSectionLost => "target_section_lost",
            NoticeKind::Refused => "refused",
            NoticeKind::Blocked => "blocked",
            NoticeKind::PowerOnline => "power_online",
            NoticeKind::OwnDestroyed => "destroyed",
            NoticeKind::OwnDisabled => "disabled",
            // **16b cut D.4**: the wire words are chosen so that the web ring's
            // fallback — the wire word with its underscores opened — reads
            // correctly with no client change.
            NoticeKind::ReceiverDegraded => "receiver_degraded",
            NoticeKind::ReceiverClear => "receiver_clear",
            // **16b cut O.2**: the pod's switch, one word for each way.
            NoticeKind::EmconOn => "emcon_on",
            NoticeKind::EmconOff => "emcon_off",
            // **17b cut M1a.4**: the motor's two answers to a `DRIVE`.
            NoticeKind::DriveArrived => "drive_arrived",
            NoticeKind::DriveNoRoute => "drive_no_route",
            // **17b cut M1b.1**: a slot's two answers.
            NoticeKind::SlotFired => "slot_fired",
            NoticeKind::SlotHeld => "slot_held",
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
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
    /// [`crate::arena::intents::Refusal::Unlawful`] (17b cut M1b.1; doc 17 §5,
    /// D-UM-17 as amended): the match's Compact floor, as the arena captured it
    /// at check-out (`Arena::odds_floor_permille`), so the sentence names the
    /// floor the condition was held to.
    Floor {
        /// The floor, per mille.
        permille: u32,
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
///
/// **16b cut M.2** (16c `T-83`) put the end of the window on arena time: a
/// `DueKind::LossSettles` at the deadline says the held loss, at the first
/// substep boundary at or after it, unless the platform re-detected the contact
/// at or before it — so the story is the same at every cadence, where the
/// pulse-end sweep it replaced said a late re-detection as a cancel at one tempo
/// and as a loss and a new contact at another. And a cancel whose row came back
/// **renamed and told apart** (a mesh copy inside the window) is said as the
/// identification it is (16c `B-74`).
///
/// **17b cut M1a.5** (doc 17 §3, D-UM-8): serde-able, every payload with it,
/// because a mind's notice buffer is hashed as each notice's canonical
/// `serde_json` bytes — every field, so two buffers that would drive two
/// different decisions (a `Miss` whose reason differs) never fold the same —
/// and carried by a snapshot. No wire reads these derives: the keyframe's
/// notice objects are `crate::match_::cockpit`'s own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
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
        /// **The round arrived on a section that was already destroyed** (16b cut
        /// D.2, `B-15`), so the gunner's line says so.
        ///
        /// `true` only on a notice that carries a `section`, which is to say only
        /// for the gunner who **holds the target and has told it apart**: a gunner
        /// watching an unidentified blip cannot tell a spent section from a live
        /// one and is told exactly what he was told before this cut, every round.
        /// See [`crate::arena::state::Arena::spent_said`] for why that is a
        /// decision and not an oversight.
        spent: bool,
    },
    /// **A round of this platform's went nowhere** (16b cut F.2), with the
    /// ledger's own reason word ([`crate::arena::damage::MISS_REASONS`]) while
    /// the platform holds the target, and **no reason at all** once it does not
    /// (16b cut AE.2): why a round missed — the target moved, the line was
    /// blocked, the ground covered it — is a position and a cover check the
    /// sensors did not earn. `contact: None` with an empty `reason` renders
    /// `Miss.`.
    Miss {
        /// The contact, where the platform still holds one.
        contact: Option<Named>,
        /// The reason, as the black box records it, when `contact` is `Some`;
        /// empty when the platform no longer holds the target (16b cut AE.2),
        /// which is not one of [`crate::arena::damage::MISS_REASONS`].
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
    /// **A round arrived on a section of this platform that was already
    /// destroyed** (16b cut D.2, `B-15`), staged to the victim in place of
    /// [`FrameNotice::Struck`] and rate-limited per section.
    ///
    /// It carries the section and nothing else: there are no numbers left to
    /// report — the armour and the internals are both nought, which is what made
    /// the `Struck` line six identical sentences — and nothing about the shooter,
    /// for the reason `Struck` carries nothing about one.
    StruckSpent {
        /// The section the round found, which is the pilot's own.
        section: SmolStr,
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
    /// **This platform is a wreck** (16b cut D.2, `B-03`), said to its own pilots.
    /// It carries nothing: the transition is the whole of it, and naming anything
    /// else would name the shooter.
    OwnDestroyed,
    /// **This platform can neither move nor shoot** (16b cut D.2, `B-03`), said to
    /// its own pilots. It carries nothing, for the reason
    /// [`FrameNotice::OwnDestroyed`] does.
    OwnDisabled,
    /// **This platform's own receiver crossed into or out of interference** (16b
    /// cut D.4, `B-22`), said to its own pilots.
    ///
    /// One `bool` and nothing else: the numbers ride [`crate::arena::step::Frame`]
    /// where the pilot's other instruments do, and there is no field here a
    /// source, a bearing, a side or an object number could be carried in.
    Receiver {
        /// Whether the receiver is now degraded.
        degraded: bool,
    },
    /// **This platform's own jamming pod was switched** (16b cut O.2, 16c N12),
    /// said to its own pilots at the arena timestamp the switch resolved at.
    ///
    /// The pilot's own tank and nothing else: the radius is the pod's own
    /// authored bubble, so the line can say how far the sights it blinds reach,
    /// and there is no field here for anybody else's sight to be named in. The
    /// sights inside the bubble learn of it only through their own receivers.
    Emcon {
        /// Whether the pod now radiates.
        on: bool,
        /// The pod's bubble, whole metres (`jammer.radius_cm / 100`).
        radius_m: u32,
    },
    /// **What the motor made of the pilot's `DRIVE`** (17b cut M1a.4; doc 17 §4
    /// D-UM-10's pilot feedback), said to the hull's own pilots at the motor
    /// step that found it — once each, because either answer drops the goal.
    ///
    /// The pilot's own drive and nothing else: no cell, no route, no other
    /// hull — the fan that may have moved the goal off a teammate's arrival
    /// square is not said, and a stop against something in the way is the
    /// `blocked` notice's.
    Drive {
        /// Arrived, or no route.
        outcome: DriveOutcome,
    },
    /// **A condition slot fired** (17b cut M1b.1 defines it; 17b cut M1b.4
    /// stages it; doc 17 §5: *every slot shot is announced once*), said on the
    /// shooter's own hull: its own slot, its own weapon, the contact its own
    /// platform holds the lock under, and the chance and the aspect that row
    /// already earned — nothing the platform has not detected.
    SlotFired {
        /// The slot, 1-based.
        slot: u8,
        /// The weapon's component id.
        weapon: SmolStr,
        /// The lock, named, while the platform still holds it.
        contact: Option<Named>,
        /// The chance the plot answered, per mille.
        p_permille: u32,
        /// The aspect the plot showed.
        aspect: Aspect,
    },
    /// **A condition slot held a shot** (17b cut M1b.1 defines it; 17b cut
    /// M1b.4 stages it, at most once per `(slot, contact)` per
    /// `crate::arena::conditions::HELD_SAID_MS`), with the term that failed.
    SlotHeld {
        /// The slot, 1-based.
        slot: u8,
        /// The weapon's component id.
        weapon: SmolStr,
        /// Why.
        why: crate::arena::conditions::HeldWhy,
    },
}

/// **The two answers the motor gives a `DRIVE`** (17b cut M1a.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveOutcome {
    /// Within `crate::arena::motor::ARRIVE_CELLS` of the goal, and stopped.
    Arrived,
    /// No route from where the hull stands; the goal is dropped and the hull
    /// holds.
    NoRoute,
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
            FrameNotice::StruckSpent { .. } => NoticeKind::StruckSpent,
            FrameNotice::SectionLost { .. } => NoticeKind::SectionLost,
            FrameNotice::TargetSectionLost { .. } => NoticeKind::TargetSectionLost,
            FrameNotice::Refused { .. } => NoticeKind::Refused,
            FrameNotice::Blocked { .. } => NoticeKind::Blocked,
            FrameNotice::PowerOnline => NoticeKind::PowerOnline,
            FrameNotice::OwnDestroyed => NoticeKind::OwnDestroyed,
            FrameNotice::OwnDisabled => NoticeKind::OwnDisabled,
            // 16b cut D.4: one transition, two words.
            FrameNotice::Receiver { degraded: true } => NoticeKind::ReceiverDegraded,
            FrameNotice::Receiver { degraded: false } => NoticeKind::ReceiverClear,
            // 16b cut O.2: one switch, two words.
            FrameNotice::Emcon { on: true, .. } => NoticeKind::EmconOn,
            FrameNotice::Emcon { on: false, .. } => NoticeKind::EmconOff,
            // 17b cut M1a.4: one drive, two words.
            FrameNotice::Drive {
                outcome: DriveOutcome::Arrived,
            } => NoticeKind::DriveArrived,
            FrameNotice::Drive {
                outcome: DriveOutcome::NoRoute,
            } => NoticeKind::DriveNoRoute,
            // 17b cut M1b.1: a slot's two answers.
            FrameNotice::SlotFired { .. } => NoticeKind::SlotFired,
            FrameNotice::SlotHeld { .. } => NoticeKind::SlotHeld,
        }
    }

    /// **Why motion stopped the platform** (16b cut D.2, `B-07`), where that is
    /// what this notice is about.
    ///
    /// The cut F.2 notice object carried five keys and the whole of the sentence
    /// lived in `text`, which a panel correctly never reads — so the web cockpit
    /// printed a bare `held` with no cause. This and the four below are the
    /// sentence's own parts, so every renderer composes the same line from the
    /// same fields and nobody has to parse prose back out.
    #[must_use]
    pub const fn cause(&self) -> Option<crate::arena::motion::BlockedCause> {
        match self {
            FrameNotice::Blocked { cause } => Some(*cause),
            _ => None,
        }
    }

    /// **Why the order was refused** (16b cut D.2, `B-07`), on a
    /// [`FrameNotice::Refused`] and nowhere else.
    #[must_use]
    pub const fn refusal(&self) -> Option<crate::arena::intents::Refusal> {
        match self {
            FrameNotice::Refused { refusal, .. } => Some(*refusal),
            _ => None,
        }
    }

    /// **The word the refused order is named by** (16b cut D.2, `B-07`) —
    /// [`crate::arena::intents::IntentKind::as_str`]'s, as the staging put it
    /// there.
    #[must_use]
    pub fn order(&self) -> Option<&SmolStr> {
        match self {
            FrameNotice::Refused { order, .. } => Some(order),
            _ => None,
        }
    }

    /// **The weapon the refused order named**, where it named one (16b cut D.2,
    /// `B-07`) — and, since **17b cut M1b.1**, the weapon a slot notice is about.
    #[must_use]
    pub fn weapon(&self) -> Option<&SmolStr> {
        match self {
            FrameNotice::Refused { weapon, .. } => weapon.as_ref(),
            FrameNotice::SlotFired { weapon, .. } | FrameNotice::SlotHeld { weapon, .. } => {
                Some(weapon)
            }
            _ => None,
        }
    }

    /// **The numbers the refusal's sentence carries** (16b cut D.2, `B-07`), or
    /// [`RefusalDetail::None`] on every other notice.
    #[must_use]
    pub const fn detail(&self) -> RefusalDetail {
        match self {
            FrameNotice::Refused { detail, .. } => *detail,
            _ => RefusalDetail::None,
        }
    }

    /// **Whether this notice is about a round that found a section already
    /// destroyed** (16b cut D.2, `B-15`) — the gunner's `Hit` with `spent` set,
    /// and the victim's [`FrameNotice::StruckSpent`].
    #[must_use]
    pub const fn spent(&self) -> bool {
        match self {
            FrameNotice::Hit { spent, .. } => *spent,
            FrameNotice::StruckSpent { .. } => true,
            _ => false,
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

    /// **What the contact this notice names used to be called** (16b cut B.1,
    /// `T-86`), where the sentence quotes an old name.
    ///
    /// The `was` of [`FrameNotice::Contact`], which the staging sites set only
    /// on [`NoticeKind::Classified`] (`X2 identified: BA`) and
    /// [`NoticeKind::Unidentified`] (`BA unidentified again: X2`); `None` on
    /// every other variant. Until this accessor the old name lived only in the
    /// sentence, so the web ring printed a regression as `X2 unidentified` and
    /// could not say which name the pilot had lost. The old name is a
    /// designator this platform itself issued or read, never an object number.
    #[must_use]
    pub const fn was(&self) -> Option<Designator> {
        match self {
            FrameNotice::Contact { was, .. } => *was,
            _ => None,
        }
    }

    /// **The section this notice is about** (16b cut Q.2, widened by 16b cut
    /// B.1, `T-86`) — the target's on the gunner's kinds, this platform's own on
    /// the victim's — so a panel never regexes a section out of `text`; `kind`
    /// says whose it is.
    ///
    /// The gunner's two: [`FrameNotice::Hit`]'s section, which is `Some` only
    /// under the held-and-classified gate, and
    /// [`FrameNotice::TargetSectionLost`]'s. The victim's three:
    /// [`FrameNotice::Struck`]'s, [`FrameNotice::StruckSpent`]'s and
    /// [`FrameNotice::SectionLost`]'s — the pilot's own tank, which is outside
    /// the observer rule (doc 16 §3.6). Only the section word: a `Struck`'s
    /// facet and numbers and a `SectionLost`'s consequence list stay in the
    /// sentence. No reader highlights by this key alone; a panel that marks the
    /// section a gunner just took off a target reads `kind` first.
    ///
    /// A renderer therefore never has to parse a sentence to know which section a
    /// line is about, which is the one-owner-per-sentence rule this module's head
    /// states.
    #[must_use]
    pub fn section(&self) -> Option<&SmolStr> {
        match self {
            FrameNotice::Hit { section, .. } => section.as_ref(),
            FrameNotice::TargetSectionLost { section, .. }
            | FrameNotice::Struck { section, .. }
            | FrameNotice::StruckSpent { section }
            | FrameNotice::SectionLost { section, .. } => Some(section),
            FrameNotice::Contact { .. }
            | FrameNotice::Locked { .. }
            | FrameNotice::Miss { .. }
            | FrameNotice::Kill { .. }
            | FrameNotice::Refused { .. }
            | FrameNotice::Blocked { .. }
            | FrameNotice::PowerOnline
            | FrameNotice::OwnDestroyed
            | FrameNotice::OwnDisabled
            // 16b cut D.4: the pilot's own receiver is not a section of anything.
            | FrameNotice::Receiver { .. }
            // 16b cut O.2: nor is the pilot's own pod's switch.
            | FrameNotice::Emcon { .. }
            // 17b cut M1a.4: nor is the pilot's own drive.
            | FrameNotice::Drive { .. }
            // 17b cut M1b.1: nor is a slot's shot or its held one.
            | FrameNotice::SlotFired { .. }
            | FrameNotice::SlotHeld { .. } => None,
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
            | FrameNotice::StruckSpent { .. }
            | FrameNotice::SectionLost { .. }
            | FrameNotice::Refused { .. }
            | FrameNotice::Blocked { .. }
            | FrameNotice::PowerOnline
            | FrameNotice::OwnDestroyed
            | FrameNotice::OwnDisabled
            // **16b cut D.4**: the receiver notice names no contact, by
            // construction — there is no field on it for one.
            | FrameNotice::Receiver { .. }
            // **16b cut O.2**: nor does the pod's switch, for the same reason.
            | FrameNotice::Emcon { .. }
            // **17b cut M1a.4**: nor does the motor's answer to a `DRIVE`.
            | FrameNotice::Drive { .. } => None,
            // **17b cut M1b.1**: a slot shot names its lock while the platform
            // holds it, and a held one the lock its aspect or range was read
            // off; a held shot for odds or rounds names nobody.
            FrameNotice::SlotFired { contact, .. } => *contact,
            FrameNotice::SlotHeld { why, .. } => match why {
                crate::arena::conditions::HeldWhy::Aspect { contact, .. }
                | crate::arena::conditions::HeldWhy::Range { contact, .. } => Some(*contact),
                crate::arena::conditions::HeldWhy::Odds { .. }
                | crate::arena::conditions::HeldWhy::Rounds { .. } => None,
            },
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

    /// **16b cut U.1** (16c `T-25`): every unit's first pass is at
    /// `stagger_ms(seed, id, pass_ms)` — keyed on the seed as well as the id, so
    /// two seeds spread the same ids differently — and two arenas with the same
    /// seed and ids schedule the same passes at every tempo, because the offset is
    /// taken against `pass_ms` and never the period.
    #[test]
    fn the_stagger_is_seeded_and_the_same_at_every_cadence() {
        use crate::arena::rng::stagger_ms;
        let ids: Vec<ObjId> = (100_000..100_024).map(ObjId).collect();
        let arena_at = |seed: u64, quiet_ms: u32| -> Arena {
            let units = ids
                .iter()
                .enumerate()
                .map(|(n, id)| {
                    let x = i64::try_from(n % 8).unwrap_or(0) * 1_000 + 500;
                    let y = i64::try_from(n / 8).unwrap_or(0) * 1_000 + 500;
                    tank(*id, Team::Red, pose(x, y))
                })
                .collect();
            let mut cfg = crate::config::ArenaConfig::default();
            cfg.tempo.quiet_ms = quiet_ms;
            Arena::new(
                crate::arena::state::CustodyOut {
                    seed,
                    ..crate::arena::test_support::custody(units, Vec::new())
                },
                &cfg,
                &crate::config::MatchConfig::default(),
                &crate::config::SensorsConfig::default(),
            )
            .expect("the stagger arena")
        };
        let first_passes = |arena: &Arena| -> Vec<(ObjId, i64)> {
            let mut passes: Vec<(ObjId, i64)> = arena
                .due
                .iter()
                .filter_map(|event| match event.kind {
                    DueKind::SensorPass { unit, pass_seq: 1 } => Some((unit, event.t_ms)),
                    _ => None,
                })
                .collect();
            passes.sort_unstable();
            passes
        };
        let arena = arena_at(2_350, 5_000);
        let cadence = i64::from(arena.sensors_cfg.pass_ms);
        let passes = first_passes(&arena);
        assert_eq!(passes.len(), ids.len(), "one first pass a unit");
        for (id, t_ms) in &passes {
            assert_eq!(*t_ms, stagger_ms(2_350, *id, cadence), "#{}", id.0);
            assert!((0..cadence).contains(t_ms));
        }
        // Contiguous ids no longer bunch into the first `units` milliseconds.
        assert!(
            passes.iter().any(|(_, t_ms)| *t_ms >= 24),
            "the stagger is still the id modulo the cadence: {passes:?}"
        );
        // Another seed spreads the same ids differently.
        assert_ne!(first_passes(&arena_at(2_351, 5_000)), passes);
        // The same seed and ids agree, at every tempo's period.
        for quiet_ms in [5_000, 1_000, 500] {
            assert_eq!(first_passes(&arena_at(2_350, quiet_ms)), passes);
        }
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

    // -----------------------------------------------------------------------
    // 16b cut U.2 — one jamming fold per unit per substep (16c `T-29`)
    // -----------------------------------------------------------------------

    const U2_RED: ObjId = ObjId(4);
    const U2_MATE: ObjId = ObjId(5);
    const U2_BLUE: ObjId = ObjId(9);
    const U2_POD: ObjId = ObjId(13);
    /// A second blue hull outside every bubble, so a wrecked pod carrier is
    /// not the end of its side.
    const U2_SPARE: ObjId = ObjId(21);

    /// A pod with round numbers: 30 m of bubble on the 8 by 8 window, a
    /// quarter of the Mk1 sight's budget, 150 per mille of a signature left.
    fn u2_pod() -> Jammer {
        Jammer {
            radius_cm: 3_000,
            attenuation_cost: 25,
            signature_permille: 150,
        }
    }

    /// Fit `carrier` with the pod and give the match its bubble radius, which
    /// `Arena::new` would have resolved from the check-out.
    fn u2_fit_pod(arena: &mut Arena, carrier: ObjId, power: Power) {
        if let Some(state) = arena.unit_mut(carrier) {
            state.jammer = Some(u2_pod());
            state.power = power;
        }
        arena.jam_radius_cm = u2_pod().radius_cm;
    }

    /// Two red line tanks a cell apart, a blue one four cells east and a blue
    /// pod carrier on the diagonal between: every one of them inside the pod's
    /// 30 m. Powered up; the index built over the opening poses and the memo
    /// empty, as phase 3(b) leaves them.
    fn u2_field() -> Arena {
        use crate::arena::test_support::line_tank;
        let mut arena = arena_of(vec![
            line_tank(U2_RED, Team::Red, pose(1_500, 1_500)),
            line_tank(U2_MATE, Team::Red, pose(1_500, 2_500)),
            line_tank(U2_BLUE, Team::Blue, pose(5_500, 1_500)),
            line_tank(U2_POD, Team::Blue, pose(3_500, 3_500)),
        ]);
        for id in [U2_RED, U2_MATE, U2_BLUE, U2_POD] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        u2_fit_pod(&mut arena, U2_POD, Power::Online);
        let units = arena.units.clone();
        arena.index.rebuild(&arena.definition.terrain, &units);
        arena.jam_fold.clear();
        arena
    }

    /// Where one unit of the fixture stands.
    fn u2_pose_of(arena: &Arena, id: ObjId) -> mush_sim::data::Pose {
        arena
            .unit(id)
            .map(|state| state.pose.clone())
            .unwrap_or_else(|| panic!("#{} is in the match", id.0))
    }

    /// **The memo answers what `jamming_at` answers, for every unit** — with
    /// the pod on and with it off — and a miss scans exactly what the
    /// primitive scans while a hit scans nothing.
    #[test]
    fn u2_the_memo_is_the_primitive_folded_once() {
        for power in [Power::Online, Power::Off] {
            let mut arena = u2_field();
            u2_fit_pod(&mut arena, U2_POD, power);
            for id in [U2_RED, U2_MATE, U2_BLUE, U2_POD] {
                let at = u2_pose_of(&arena, id);
                let mut direct = PerceptionCounters::default();
                let expected = arena.jamming_at(&at, &mut direct);
                assert_eq!(
                    expected.is_some(),
                    power == Power::Online,
                    "#{}: the fixture's pod reaches every unit when it is on",
                    id.0
                );
                let mut missed = PerceptionCounters::default();
                assert_eq!(arena.jam_fold_of(id, &mut missed), expected, "#{}", id.0);
                assert_eq!(
                    missed.jam_checks, direct.jam_checks,
                    "#{}: a miss scans what the primitive scans",
                    id.0
                );
                assert!(missed.jam_checks > 0);
                let mut hit = PerceptionCounters::default();
                assert_eq!(arena.jam_fold_of(id, &mut hit), expected, "#{}", id.0);
                assert_eq!(hit.jam_checks, 0, "#{}: a hit scans nothing", id.0);
            }
            assert_eq!(arena.jam_fold.len(), 4, "one row a unit");
            // A unit not in the match is no fold and no row.
            let mut none = PerceptionCounters::default();
            assert_eq!(arena.jam_fold_of(ObjId(999), &mut none), None);
            assert_eq!(none.jam_checks, 0);
            assert_eq!(arena.jam_fold.len(), 4);
        }
    }

    /// **Every unit's pass and share in one substep scans each unit's bubble
    /// once** when no pod's state moves: a unit folded by its own pass, by its
    /// own share and as a mate in a teammate's share, twice over, is counted
    /// once — exactly the sum of one primitive scan per unit.
    #[test]
    fn u2_a_substep_of_passes_and_shares_scans_each_unit_once() {
        let mut arena = u2_field();
        // Every unit holds its enemy classified, as its own sighting, so every
        // share has something to say and every teammate is a mate.
        for (viewer, target, team) in [
            (U2_RED, U2_BLUE, Team::Blue),
            (U2_MATE, U2_BLUE, Team::Blue),
            (U2_BLUE, U2_RED, Team::Red),
            (U2_POD, U2_RED, Team::Red),
        ] {
            let plot = arena
                .unit(target)
                .map(SeenPose::of)
                .unwrap_or_else(|| panic!("#{} is in the match", target.0));
            let id = crate::arena::rng::draw_contact_id(arena.seed, viewer, target);
            arena.contacts.insert(
                viewer,
                id,
                crate::arena::state::ShadowContact::sighted(
                    target,
                    Classification::Classified { team },
                    0,
                    plot,
                ),
            );
        }
        let mut expected = 0_u32;
        for id in [U2_RED, U2_MATE, U2_BLUE, U2_POD] {
            let at = u2_pose_of(&arena, id);
            let mut one = PerceptionCounters::default();
            let _ = arena.jamming_at(&at, &mut one);
            expected += one.jam_checks;
        }
        let mut counters = PerceptionCounters::default();
        let mut journal: Vec<Notice> = Vec::new();
        for (pass_seq, t_ms) in [(1_u64, 100_i64), (2, 200)] {
            for id in [U2_RED, U2_MATE, U2_BLUE, U2_POD] {
                arena.sensor_pass(id, pass_seq, t_ms, &mut counters, &mut journal);
                arena.share_pass(id, t_ms, &mut counters, &mut journal);
            }
        }
        assert!(
            counters.share_ops > 0,
            "the mesh ran, so mates were folded too: {counters:?}"
        );
        assert_eq!(
            counters.jam_checks, expected,
            "each unit's bubble was scanned once in the substep: {counters:?}"
        );
        assert_eq!(arena.jam_fold.len(), 4);
    }

    /// An arena of `units` over the test window whose sensor pass runs every
    /// 100 ms, so one unit passes two or three times inside one substep.
    fn u2_fast_passes(units: Vec<crate::arena::state::CustodyUnit>) -> Arena {
        let sensors = crate::config::SensorsConfig {
            pass_ms: 100,
            ..crate::config::SensorsConfig::default()
        };
        Arena::new(
            crate::arena::test_support::custody(units, Vec::new()),
            &crate::config::ArenaConfig::default(),
            &crate::config::MatchConfig::default(),
            &sensors,
        )
        .expect("the fast-pass arena")
    }

    /// `U2_RED`'s first pass and the substep it falls in, with the
    /// precondition both proofs below stand on asserted: the second pass, 100 ms
    /// after the first, is inside the same first substep.
    fn u2_first_pass(arena: &Arena) -> (i64, i64) {
        let first = crate::arena::rng::stagger_ms(arena.seed, U2_RED, 100);
        let substep = arena.substep_len_ms();
        assert!(
            first + 100 <= substep,
            "the fixture wants two passes of #{} in the first {substep} ms substep; the first is \
             at {first}",
            U2_RED.0
        );
        (first, substep)
    }

    /// **The exactness proof, first half**: a pod that comes online between two
    /// folds in one substep is seen by the second. The `StartupComplete` due
    /// between the observer's two passes clears the memo, so the second pass
    /// scans again and its receiver reads the bubble; without the clear it would
    /// have read the first fold's `None` for the rest of the substep.
    #[test]
    fn u2_a_pod_that_comes_online_between_two_folds_is_seen_by_the_second() {
        use crate::arena::test_support::line_tank;
        let mut arena = u2_fast_passes(vec![
            line_tank(U2_RED, Team::Red, pose(1_500, 1_500)),
            tank(U2_POD, Team::Blue, pose(3_500, 3_500)),
        ]);
        u2_fit_pod(&mut arena, U2_POD, Power::Starting);
        let (first, substep) = u2_first_pass(&arena);
        arena
            .due
            .schedule(first + 50, DueKind::StartupComplete { unit: U2_POD });
        let at = u2_pose_of(&arena, U2_RED);
        let mut one = PerceptionCounters::default();
        let _ = arena.jamming_at(&at, &mut one);

        arena.step(0, &[], &[]);
        let out = arena.step(substep, &[], &[]);
        assert_eq!(out.substeps, 1, "one substep, holding both passes");
        assert_eq!(
            arena.unit(U2_POD).map(|state| state.power),
            Some(Power::Online)
        );
        assert!(
            matches!(arena.jam_fold.get(&U2_RED), Some(Some(_))),
            "the fold after the startup is the online pod's: {:?}",
            arena.jam_fold
        );
        assert!(
            arena
                .unit(U2_RED)
                .and_then(|state| state.receiver)
                .is_some(),
            "the second pass ran under the pod that had just come up"
        );
        assert_eq!(
            out.perception.jam_checks,
            2 * one.jam_checks,
            "one scan before the startup and one after it, and none for a third pass"
        );
    }

    /// **The exactness proof, second half**: a pod wrecked by an `Impact` between
    /// two folds in one substep is not seen by the second. The round finishes the
    /// carrier's hull (`destroy_unit` turns its power off), the arm clears the
    /// memo, and the observer's next pass reads a clear receiver.
    #[test]
    fn u2_a_pod_wrecked_by_an_impact_between_two_folds_is_not_seen_by_the_second() {
        use crate::arena::test_support::line_tank;
        let mut arena = u2_fast_passes(vec![
            line_tank(U2_RED, Team::Red, pose(1_500, 1_500)),
            line_tank(U2_POD, Team::Blue, pose(3_500, 3_500)),
            tank(U2_SPARE, Team::Blue, pose(7_500, 7_500)),
        ]);
        u2_fit_pod(&mut arena, U2_POD, Power::Online);
        // The carrier sees nothing (so it folds nothing of its own) and every
        // round that reaches it goes into the hull, whose loss wrecks the tank.
        if let Some(state) = arena.unit_mut(U2_POD) {
            state.sensor = None;
            for band in &mut state.profile.bands {
                band.sections = vec![(SmolStr::new("hull"), 1)];
            }
        }
        let (first, substep) = u2_first_pass(&arena);
        let band = match (arena.unit(U2_POD), arena.unit(U2_RED)) {
            (Some(target), Some(shooter)) => {
                crate::arena::resolution::facet_band(target, shooter).to_owned()
            }
            _ => panic!("both tanks are in the match"),
        };
        let aim_cell = arena
            .unit(U2_POD)
            .and_then(|state| arena.definition.terrain.cell_of(&state.pose));
        // 16b cut AE.2: the round leaves from where the red tank stands, at its
        // eye — what `resolve_fire` would have recorded.
        let (from_x_cm, from_y_cm, from_cell, from_height_cm) = arena
            .unit(U2_RED)
            .map(|state| {
                (
                    state.pose.x_cm,
                    state.pose.y_cm,
                    arena.definition.terrain.cell_of(&state.pose),
                    i32::try_from(state.sensor_height_cm).unwrap_or(i32::MAX),
                )
            })
            .unwrap_or_else(|| panic!("the red tank is in the match"));
        arena.shots.insert(
            1,
            crate::arena::weapons::Shot {
                seq: 1,
                shooter: U2_RED,
                weapon: SmolStr::new("cannon"),
                contact: crate::arena::state::ContactId(1),
                target: U2_POD,
                rounds: 1,
                fired_at_ms: 0,
                impact_at_ms: first + 50,
                range_cm: 2_828,
                aim_x_cm: 3_500,
                aim_y_cm: 3_500,
                aim_cell,
                from_x_cm,
                from_y_cm,
                from_cell,
                from_height_cm,
                damage: 1_000_000,
                resolution: crate::arena::resolution::Resolution {
                    p_permille: 1_000,
                    ..crate::arena::resolution::Resolution::blind(
                        &band,
                        3_000,
                        mush_atlas::Cover::None,
                    )
                },
            },
        );
        arena.next_shot_seq = 1;
        arena
            .due
            .schedule(first + 50, DueKind::Impact { shot_seq: 1 });

        arena.step(0, &[], &[]);
        let out = arena.step(substep, &[], &[]);
        assert_eq!(out.substeps, 1, "one substep, holding both passes");
        assert!(
            arena.unit(U2_POD).is_some_and(|state| state.limits.wreck),
            "the round finished the carrier"
        );
        assert_eq!(
            arena.jam_fold.get(&U2_RED),
            Some(&None),
            "the fold after the impact is a field with no live pod: {:?}",
            arena.jam_fold
        );
        assert_eq!(
            arena.unit(U2_RED).and_then(|state| state.receiver),
            None,
            "the second pass ran with the pod gone, and the receiver cleared"
        );
    }

    /// **A rebuild empties the memo**: filled by hand, it is empty after a
    /// substep in which nothing folded — the rebuilds cleared it and no pass
    /// refilled it.
    #[test]
    fn u2_the_memo_is_empty_after_a_rebuild() {
        let mut arena = arena_of(vec![
            tank(U2_RED, Team::Red, pose(1_500, 1_500)),
            tank(U2_POD, Team::Blue, pose(3_500, 3_500)),
        ]);
        u2_fit_pod(&mut arena, U2_POD, Power::Online);
        let mut counters = PerceptionCounters::default();
        assert!(arena.jam_fold_of(U2_RED, &mut counters).is_some());
        assert!(arena.jam_fold_of(U2_POD, &mut counters).is_some());
        assert_eq!(arena.jam_fold.len(), 2);
        // Neither tank carries a sight, so no pass in the substep folds.
        arena.step(0, &[], &[]);
        let substep = arena.substep_len_ms();
        let out = arena.step(substep, &[], &[]);
        assert!(out.substeps >= 1);
        assert!(arena.jam_fold.is_empty(), "{:?}", arena.jam_fold);
        assert_eq!(out.perception.jam_checks, 0);
    }

    /// **A stock match is bit-identical**: no pod, so `jam_radius_cm` is nought,
    /// the memo is never touched and `jam_checks` stays at zero however many
    /// passes and shares run.
    #[test]
    fn u2_a_stock_match_counts_no_jam_check_and_touches_no_memo() {
        use crate::arena::test_support::line_tank;
        let mut arena = arena_of(vec![
            line_tank(U2_RED, Team::Red, pose(1_500, 1_500)),
            line_tank(U2_MATE, Team::Red, pose(1_500, 2_500)),
            line_tank(U2_BLUE, Team::Blue, pose(5_500, 1_500)),
        ]);
        assert_eq!(arena.jam_radius_cm, 0);
        let mut counters = PerceptionCounters::default();
        assert_eq!(arena.jam_fold_of(U2_RED, &mut counters), None);
        let mut total = PerceptionCounters::default();
        arena.step(0, &[], &[]);
        for wall in 1..=24_i64 {
            let out = arena.step(wall * 500, &[], &[]);
            total.absorb(&out.perception);
        }
        assert!(total.passes > 0 && total.detected > 0, "{total:?}");
        assert_eq!(total.jam_checks + counters.jam_checks, 0);
        assert!(arena.jam_fold.is_empty());
    }

    // -----------------------------------------------------------------------
    // 16b cut O.2 — the pod's switch (16c N12)
    // -----------------------------------------------------------------------

    /// **`jamming_at` skips a silent pod**: the carrier standing at range zero
    /// of its own pod folds `None` once the switch is off, as does every unit
    /// the radiating pod reached — and the same arena with the switch back on
    /// folds what it folded before.
    #[test]
    fn o2_jamming_at_skips_a_silent_pod() {
        let mut arena = u2_field();
        let own = u2_pose_of(&arena, U2_POD);
        let near = u2_pose_of(&arena, U2_RED);
        let mut counters = PerceptionCounters::default();
        let radiating = arena.jamming_at(&own, &mut counters);
        assert!(radiating.is_some(), "a pod jams its own carrier");
        assert!(arena.jamming_at(&near, &mut counters).is_some());
        if let Some(state) = arena.unit_mut(U2_POD) {
            state.ecm_on = false;
        }
        assert_eq!(
            arena.jamming_at(&own, &mut counters),
            None,
            "the carrier's own sight clears"
        );
        assert_eq!(arena.jamming_at(&near, &mut counters), None);
        if let Some(state) = arena.unit_mut(U2_POD) {
            state.ecm_on = true;
        }
        assert_eq!(arena.jamming_at(&own, &mut counters), radiating);
    }

    /// **A mate behind a silent pod's bubble is reached by the share a
    /// radiating pod refused** (doc 16 W6, 16b cut Q.6's two-ended fold, 16b cut
    /// O.2's one clause): sixty metres between two teammates is inside the
    /// sight's 300 m share reach and outside the 45 m the pod leaves of it at
    /// the mate's end, so the link is cut by range while the pod radiates and
    /// made when it falls silent — the mesh reads the switch through the same
    /// memo detection does.
    #[test]
    fn o2_a_mate_behind_a_silent_pod_receives_the_share_a_radiating_pod_refused() {
        use crate::arena::test_support::line_tank;
        let field = |on: bool| -> bool {
            let mut arena = arena_of(vec![
                line_tank(U2_RED, Team::Red, pose(500, 500)),
                line_tank(U2_MATE, Team::Red, pose(6_500, 500)),
                tank(U2_BLUE, Team::Blue, pose(500, 5_500)),
                tank(U2_POD, Team::Blue, pose(7_500, 2_500)),
            ]);
            u2_fit_pod(&mut arena, U2_POD, Power::Online);
            if let Some(state) = arena.unit_mut(U2_POD) {
                state.ecm_on = on;
            }
            let plot = arena
                .unit(U2_BLUE)
                .map(SeenPose::of)
                .unwrap_or_else(|| panic!("the target is in the match"));
            let id = crate::arena::rng::draw_contact_id(arena.seed, U2_RED, U2_BLUE);
            arena.contacts.insert(
                U2_RED,
                id,
                crate::arena::state::ShadowContact::sighted(
                    U2_BLUE,
                    Classification::Classified { team: Team::Blue },
                    0,
                    plot,
                ),
            );
            let units = arena.units.clone();
            arena.index.rebuild(&arena.definition.terrain, &units);
            arena.jam_fold.clear();
            let mut counters = PerceptionCounters::default();
            let mut journal: Vec<Notice> = Vec::new();
            arena.share_pass(U2_RED, 0, &mut counters, &mut journal);
            assert!(counters.jam_checks > 0, "the share scanned for the pod");
            let theirs = crate::arena::rng::draw_contact_id(arena.seed, U2_MATE, U2_BLUE);
            arena.contacts.resolve(U2_MATE, theirs).is_some()
        };
        assert!(!field(true), "the radiating pod did not cut the link");
        assert!(field(false), "the silent pod still cut the link");
    }

    /// **A flip flood costs an observer at most one transition per pass** (the
    /// cut O plan's item 38, recorded and not floored): three flips of the pod
    /// inside one `pass_ms` of the observer's — off, on, off — journal three
    /// `emcon` lines at their own milliseconds and one `receiver` line, at the
    /// observer's next pass, for the state the flood ended in.
    #[test]
    fn o2_three_flips_inside_one_pass_give_an_observer_one_transition() {
        use crate::arena::intents::{Intent, IntentKind};
        use crate::arena::state::Channel;
        use crate::arena::test_support::{arena_at_epoch, line_tank};

        let mut arena = arena_at_epoch(
            vec![
                line_tank(U2_RED, Team::Red, pose(1_500, 1_500)),
                tank(U2_POD, Team::Blue, pose(3_500, 3_500)),
                tank(U2_SPARE, Team::Red, pose(7_500, 7_500)),
            ],
            1,
        );
        for id in [U2_RED, U2_SPARE] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        u2_fit_pod(&mut arena, U2_POD, Power::Online);
        let mut bytes: Vec<u8> = Vec::new();
        let mut step = |arena: &mut Arena, wall: i64, kinds: &[(IntentKind, u64)]| {
            let intents: Vec<Intent> = kinds
                .iter()
                .map(|(kind, seq)| Intent {
                    unit: U2_POD,
                    channel: Channel::Sensors,
                    epoch: 1,
                    kind: kind.clone(),
                    seq: *seq,
                })
                .collect();
            let out = arena.step(wall, &[], &intents);
            for chunk in &out.ledger_chunks {
                bytes.extend_from_slice(&chunk.bytes);
            }
            assert_eq!(arena.now_ms, wall, "the pulse reached its wall time");
        };
        step(&mut arena, 0, &[]);
        step(&mut arena, 2_000, &[]);
        // The observer's passes are `pass_ms` apart from its stagger.
        let pass_ms = i64::from(arena.sensors_cfg.pass_ms);
        let first = crate::arena::rng::stagger_ms(arena.seed, U2_RED, pass_ms);
        let base = first + 2 * pass_ms;
        assert!((2_000..3_000).contains(&base), "{base}");
        step(
            &mut arena,
            base + 100,
            &[(IntentKind::Emcon { on: false }, 1)],
        );
        step(
            &mut arena,
            base + 300,
            &[(IntentKind::Emcon { on: true }, 2)],
        );
        step(
            &mut arena,
            base + 500,
            &[(IntentKind::Emcon { on: false }, 3)],
        );
        step(&mut arena, base + pass_ms + 500, &[]);
        arena.ledger.flush();
        for chunk in arena.ledger.take_chunks() {
            bytes.extend_from_slice(&chunk.bytes);
        }
        let events: Vec<crate::arena::ledger::LedgerEvent> = String::from_utf8(bytes)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let emcon: Vec<(i64, EventKind)> = events
            .iter()
            .filter(|event| event.kind.as_str() == "emcon")
            .map(|event| (event.t_ms, event.kind.clone()))
            .collect();
        assert_eq!(
            emcon,
            vec![
                (
                    base + 100,
                    EventKind::Emcon {
                        unit: U2_POD,
                        on: false
                    }
                ),
                (
                    base + 300,
                    EventKind::Emcon {
                        unit: U2_POD,
                        on: true
                    }
                ),
                (
                    base + 500,
                    EventKind::Emcon {
                        unit: U2_POD,
                        on: false
                    }
                ),
            ]
        );
        let receiver: Vec<(i64, EventKind)> = events
            .iter()
            .filter(
                |event| matches!(event.kind, EventKind::Receiver { unit, .. } if unit == U2_RED),
            )
            .map(|event| (event.t_ms, event.kind.clone()))
            .collect();
        assert_eq!(
            receiver,
            vec![
                (
                    first,
                    EventKind::Receiver {
                        unit: U2_RED,
                        degraded: true
                    }
                ),
                (
                    base + pass_ms,
                    EventKind::Receiver {
                        unit: U2_RED,
                        degraded: false
                    }
                ),
            ],
            "one transition for the whole flood, at the observer's own next pass"
        );
    }

    /// **17b cut M1b.1**: `share_pass` answers the `(receiver, contact)` pairs it
    /// inserted or refreshed, and only for a receiver holding a condition slot —
    /// a share into a side with no slot answers an empty vector (no allocation:
    /// `Vec::new()` reserves nothing) and still writes the mate's row.
    #[test]
    fn m1b1_a_share_answers_its_refreshed_pairs_only_for_a_slot_holder() {
        use crate::arena::conditions::{ASPECTS_ANY, Arming, FireCondition, SlotTarget};
        use crate::arena::test_support::line_tank;
        const SHARER: ObjId = ObjId(4);
        const MATE: ObjId = ObjId(5);
        const TARGET: ObjId = ObjId(9);
        let mut arena = arena_of(vec![
            line_tank(SHARER, Team::Red, pose(1_500, 1_500)),
            line_tank(MATE, Team::Red, pose(1_500, 2_500)),
            line_tank(TARGET, Team::Blue, pose(5_500, 1_500)),
        ]);
        let plot = arena
            .unit(TARGET)
            .map(crate::arena::state::SeenPose::of)
            .unwrap_or_default();
        let sighted = |at: i64| {
            crate::arena::state::ShadowContact::sighted(
                TARGET,
                Classification::Classified { team: Team::Blue },
                at,
                plot,
            )
        };
        let theirs = crate::arena::rng::draw_contact_id(arena.seed, SHARER, TARGET);
        let mine = crate::arena::rng::draw_contact_id(arena.seed, MATE, TARGET);
        arena.contacts.insert(SHARER, theirs, sighted(100));
        let mut counters = PerceptionCounters::default();
        let mut journal: Vec<Notice> = Vec::new();
        let pairs = arena.share_pass(SHARER, 100, &mut counters, &mut journal);
        assert!(pairs.is_empty(), "no slot on the side: {pairs:?}");
        assert_eq!(pairs.capacity(), 0, "and nothing allocated for it");
        assert!(
            arena.contacts.get(MATE, mine).is_some(),
            "the share still landed"
        );
        // The mate arms a slot; a newer sighting refreshes its row.
        assert_eq!(
            arena.resolve_arming(
                MATE,
                &Arming::Arm(FireCondition {
                    weapon: smol_str::SmolStr::new("cannon"),
                    target: SlotTarget::Lock,
                    min_permille: 100,
                    aspects: ASPECTS_ANY,
                    min_rounds_left: 0,
                    min_range_m: 0,
                    max_range_m: u32::MAX,
                }),
                0
            ),
            Ok(Some(1))
        );
        arena.contacts.insert(SHARER, theirs, sighted(200));
        let pairs = arena.share_pass(SHARER, 200, &mut counters, &mut journal);
        assert_eq!(pairs, vec![(MATE, mine)]);
        // A share no newer than the mate's row refreshes nothing.
        let pairs = arena.share_pass(SHARER, 200, &mut counters, &mut journal);
        assert!(pairs.is_empty(), "{pairs:?}");
    }
}
