//! **The arena core** (16b cut A; doc 16 §3.1–§3.4, §4 "Arena state" and
//! "Ledger events", §8; D-16-1..5, D-16-10) — the combat authority's state, its
//! clock, its inputs, its randomness, its state hash and its match ledger, as a
//! library with **no input and no output at all**.
//!
//! # What "no I/O" means here, and why a contract test scans for it
//!
//! Doc 16 §3.1 (D-16-1, D-16-3): while a match runs the arena is the single
//! writer for its own facts — poses, condition cells, contacts, orders,
//! scheduled impacts, the match ledger — and the world half remains the single
//! writer for everything else. The two share no state and no lock. The way that
//! is *enforced* rather than promised is that nothing under this directory can
//! reach the store, a task, a session or the world at all: no persistence crate,
//! no async runtime, no output sink, no batch, no world handle. Cut T's
//! `arena_task` is the thin wrapper that owns an [`crate::arena::state::Arena`], the bounded
//! inbox, the bounded egress, a timer driven by `next_due_ms`, a watchdog and
//! the unwind boundary; everything below is a pure function of state and inputs.
//! `crates/mush-contract-tests/tests/arena_library.rs` scans every file here for
//! the names that would break that, so the property is a gate and not a habit.
//!
//! # Everything is an integer
//!
//! Doc 16 §3.1 "Determinism" (D-16-4): every stored quantity below is an
//! integer — centimetres, millidegrees, milliseconds, cm/s, per-mille. Floating
//! point is allowed *inside* a computation as long as the stored result is
//! quantised, and nothing in cut A needs it. That is what lets the cross-tempo
//! property test (A7) assert byte-identical per-substep state hashes across the
//! 5,000 / 1,000 / 500 ms tempos, and it is why the substep hash
//! ([`crate::arena::hash::state_hash`]) is a stable function of the state rather than of the
//! machine.
//!
//! # The two halves of cut A
//!
//! **A.1** was the data, the time, the inputs, the randomness, the hashing and
//! the ledger: [`crate::arena::state`], [`crate::arena::clock`], [`crate::arena::intents`], [`crate::arena::rng`], [`crate::arena::hash`], [`crate::arena::ledger`],
//! their unit tests, and the two gates doc 16 §8 names. Its public surface is a
//! contract.
//!
//! **A.2 (this commit)** adds [`crate::arena::index`] (the uniform grid of cell buckets,
//! rebuilt every substep, and the one Euclidean range function), [`crate::arena::motion`]
//! (doc 16 §3.4 per substep over `move_cost`, with a fixed-point sine table so
//! that a stored position is an integer a replay reproduces) and [`crate::arena::step`] (the
//! five pulse phases of doc 16 §3.1, in order, each reading what the previous
//! committed) on exactly A.1's surface, plus three additions to it that the
//! pulse needs and A.1 could not have used: [`crate::arena::state::Arena::index`],
//! [`crate::arena::state::Arena::last_frames`] and [`crate::arena::state::Arena::substep_hashes`].
//!
//! The tests that pin the claim are `crates/tacsim/tests/arena.rs`: one scripted
//! control and intent log replayed at 5,000, 1,000 and 500 ms produces one
//! ordered event list and one per-substep hash sequence, crossing an impassable
//! band and an occupied cell.
//!
//! # Cut S.1: the sensor pass and the shadow contact store
//!
//! [`crate::arena::perception`] is doc 16 §3.6's five steps as a [`crate::arena::state::DueKind::SensorPass`]
//! per unit every `sensors.pass_ms`, staggered by unit id, plus the
//! [`crate::arena::state::DueKind::Share`] the comms mesh fans out on. It fills the store A.1
//! declared ([`crate::arena::state::ShadowContacts`]), the three ledger kinds A.1 reserved
//! ([`crate::arena::ledger::EventKind::Detected`], [`crate::arena::ledger::EventKind::Classified`],
//! [`crate::arena::ledger::EventKind::Lost`]), the `contact_within_alert_range` signal A.1's
//! clock left `false`, and [`crate::arena::step::Frame::contacts`], which cut S.2 renders.
//!
//! Its one dependency outside this tree is the atlas's line of sight over a plain
//! cell grid, which [`crate::arena::state::ArenaTerrain`] implements
//! (`mush_atlas::CellGrid`): `mush_atlas::sight_only` for perception (16b cut
//! J.1, through [`crate::arena::Arena::sight`]) and `mush_atlas::trace_cells` for
//! the fire path (16b cut W.2), two walkers over the same seam and the same
//! per-cell rule. Both are *trace* functions and carry no window, no
//! record and no store, so the no-I/O gate still holds; the alternative,
//! `mush_atlas::SurfaceComposite`, would have brought a surface reference, a room
//! table and a feature overlay with it.
//!
//! # Cut J.2: the horizon index
//!
//! [`crate::arena::horizon`] is 16c N5's second step: a two-level pyramid (16-
//! and 4-cell blocks) of each block's highest and lowest cell and whether any is
//! opaque or partial, which proves a run of a line clear without reading it and
//! blocked by reading one cell, and walks the rest. It is a private field of
//! [`crate::arena::state::ArenaTerrain`], built by its one constructor and queried
//! by [`crate::arena::state::ArenaTerrain::sight`], so a test that swaps the
//! definition or the terrain swaps the index with it and `Arena` holds no copy.
//! Every sight is `mush_atlas::sight_only`'s to the byte (the module head argues
//! it); what moves is the cost — `cell_probes` down, and a new
//! [`crate::arena::perception::PerceptionCounters::horizon_probes`]. A cache of
//! terrain, folded into no hash, and not doc 16 §3.2's cross-observer cache:
//! nothing observed is cached.
//!
//! # Cut W.1: weapons and the shot
//!
//! [`crate::arena::weapons`] is doc 16 §3.5's first half: the two live cells a weapon has on a
//! unit (`ready_at_ms` and `ammo`, both laid down complete at check-out), the bodies
//! of the two discrete actions phase 4 used to drop ([`crate::arena::Arena::resolve_lock`] and
//! [`crate::arena::Arena::resolve_fire`]), the [`crate::arena::weapons::Shot`] an accepted shot puts in
//! flight, and the [`crate::arena::state::DueKind::Impact`] and [`crate::arena::state::DueKind::WeaponReady`]
//! events it schedules. It fills four more of the ledger kinds A.1 reserved
//! ([`crate::arena::ledger::EventKind::Locked`], [`crate::arena::ledger::EventKind::Fired`],
//! [`crate::arena::ledger::EventKind::Impact`], [`crate::arena::ledger::EventKind::Miss`]), the two `Engaged`
//! tempo signals A.1's clock left `false`, and [`crate::arena::step::Frame::lock`] and
//! [`crate::arena::step::Frame::weapons`], which `WEAPONSTATUS` renders.
//!
//! # Cut W.2: the resolution, the damage, and the end of a match
//!
//! [`crate::arena::resolution`] is doc 16 §3.5's five steps — the silhouette the target presents
//! toward the shooter, the error cone the four weighted terms widen, the curve that
//! turns their ratio into a per-mille probability, the draw that compares against
//! it, and the number the cockpit shows before any of it. [`crate::arena::damage`] is what a round
//! does when it arrives: the four re-validations and their named reasons, the facet
//! draw, the armour and the overflow, the consequences, the wreck, and doc 16
//! §3.8's team elimination.
//!
//! Between them they fill the last of the ledger kinds A.1 reserved
//! ([`crate::arena::ledger::EventKind::Hit`], [`crate::arena::ledger::EventKind::SectionDestroyed`],
//! [`crate::arena::ledger::EventKind::UnitDisabled`], [`crate::arena::ledger::EventKind::UnitDestroyed`]), the
//! three draw labels doc 16 §3.5 pinned and nothing had yet read (`hit`, `facet`,
//! `burst`), and the per-unit half of doc 16 §3.1's seventh seam message
//! ([`crate::arena::damage::UnitReturn`]) that cut T.3 left empty.
//!
//! **Where a match ends.** Doc 16 §3.8's team elimination is settled inside the
//! **substep** the last unit of a side went out of action in ([`crate::arena::step`]'s own note
//! says why): a wreck is applied at a due event's own arena timestamp, so the moment
//! a side was eliminated is a fact of arena time, and a `match_ended` stamped at the
//! pulse boundary instead would carry a different timestamp at every cadence — which
//! is exactly what the cross-tempo property forbids. A substep that ends the match
//! ends the pulse with it, and phase 4 resolves nothing afterwards.
//!
//! Two dependencies outside this tree, both deliberate. `mush_tactics::apply` is the
//! **only** way a unit's cells move (D-16-6): one
//! [`mush_tactics::UnitMutationPlan`] per impact, refuse-never-clamp, and it is a
//! pure library call with no store, no task and no world — which is why the no-I/O
//! gate still holds. And `crate::definitions::resolution::ResolutionDef` crosses the
//! seam whole rather than in a copy of its own, because its `hit_permille` *is* doc
//! 16 §3.5 step 3's interpolation and a second copy of a curve walk is a second
//! answer waiting to happen.
//!
//! # Cut N.1: the reflex controller and the paths
//!
//! [`crate::arena::reflex`] is doc 16 §6's deterministic state machine — `Startup → Patrol →
//! Approach → Engage → Withdraw → Disabled`, with `blocked`, `lost`, `no line`
//! and `unreachable` as edges — and [`crate::arena::path`] is the bounded A\* over the arena's
//! own terrain copy that `Patrol`, `Approach` and `Withdraw` route on. The
//! machine gives its orders through [`crate::arena::intents::apply_intent`], the same door a
//! pilot's verbs come through, so it can do nothing a player could not.
//!
//! It fills the `controller` attribute A.1 had no reader for
//! ([`crate::arena::state::Controller`]), the two tempo signals W.1 left `false`
//! ([`crate::arena::clock::TempoSignals::ai_approaching`] and
//! [`crate::arena::clock::TempoSignals::hostile_in_weapons_range`]), and the waypoint graph and
//! key spots C.2 authored and nothing read ([`crate::arena::state::ArenaDefinition::waypoints`],
//! [`crate::arena::state::ArenaDefinition::key_spots`]).
//!
//! **Its tick is a [`crate::arena::state::DueKind::ReflexTick`] and not pulse phase 4**, which
//! is a deviation from 16b §8's cut N row with a reason the same row gives:
//! [`crate::arena::reflex`]'s module head has it in full.
//!
//! **17b cut M1a.1** split the driving half out (doc 17 §4, D-UM-3):
//! [`crate::arena::motor`] owns the route, the search and its budget, the
//! follower and the remembered stops, one [`crate::arena::motor::MotorRow`]
//! per steered hull on [`crate::arena::state::Arena::motors`], and the reflex
//! controller decides the goal it drives to.
//!
//! # Cut X.6: elevation, and the one slope rule
//!
//! [`crate::arena::slope`] is doc 16 §9 item 6's second half. The heightfield itself was
//! already here — `crate::definitions::arena::from_pack` copies `elev_cm`
//! verbatim, [`crate::arena::motion`] carries it onto the pose at a crossing, and doc 16
//! §3.2's line of sight has honoured it since cut S.1 — so what this cut adds is
//! the *edge*: the slope between two cells surcharges what it costs to cross
//! them, and a slope past the locomotion's limit is not an edge at all.
//!
//! One module, because the rule has to be one rule: [`crate::arena::motion`] drives over an
//! edge and [`crate::arena::path`] plans over it, and a route that promised an edge the tank
//! is then refused at is a tank driving at a bluff until the match ends. The
//! flat case is the identity (`cost × 100 / 100`), so every level window in this
//! crate runs exactly the arithmetic it ran before.
//!
//! Its one dependency outside this tree is `mush_atlas::physics::{slope_pct,
//! StepKind, LocomotionTable}` — arithmetic and a table, no window, no record and
//! no store, so the no-I/O gate holds for the reason cut S.1's `trace_cells` does.
//! `LocomotionTable::shipped()` is an `include_str!` parsed once behind a
//! `OnceLock`, and [`crate::arena::slope::SlopeLimit::read`] runs once per unit at check-out,
//! so no pulse ever reads it. `from_pack` already prices every cell off the same
//! table, which is what makes the cell's cost and the edge's surcharge one
//! answer rather than two.
//!
//! **Two deliberate divergences**, both argued in [`crate::arena::slope`]'s own head: the
//! limit is symmetric where the atlas's is uphill-only (an uphill-only limit
//! makes a bluff a one-way door), and `physics::move_cost`'s climb branch is not
//! applied (it models a kerb inside one cell, and the arena's cells are 10 m).
//!
//! # Cut Y.4: the catch-up report
//!
//! [`crate::arena::catchup`] is doc 07 §1.3 step 5: a snapshot of what a
//! platform held and was when its pilot lost sight of it, and the report a
//! reclaim builds against it, carried once on the pilot's next frame
//! ([`crate::arena::step::Frame::catchup`]). Both tables are views beside
//! `last_frames` and fold into no hash.
//!
//! # Cut I.4: the staff picture
//!
//! [`crate::arena::observer`] is 16c N10's third lever: [`crate::arena::observer::Observer`], the
//! whole field as staff see it — objective, every unit by object number — built
//! by [`crate::arena::state::Arena::observer`] and handed out by phase 5 on
//! [`crate::arena::step::StepOutput::observer`] at most once per pulse and at
//! most once an arena second while [`crate::arena::state::Arena::watched`] is
//! set. It is a type distinct from a pilot's [`crate::arena::step::Frame`] with no path into one
//! (doc 16 §1), and the two fields that gate it are views no hash folds.
//!
//! # Cut AF.1: the snapshot
//!
//! [`crate::arena::snapshot`] is 16c N16's first third: one running arena,
//! between two pulses, as one versioned document whose state is one digested
//! string, and the same arena back out of it on the ground and under the
//! locomotion table it was fought on — the decoded state hash equal to the
//! recorded one on every decode. Its head says which of [`crate::arena::state::Arena`]'s
//! fields it carries, re-derives and resets. It ships nothing and boots nothing:
//! the message is 16b cut AF.2's and the boot AF.3's.
//!
//! # 17b cut M1a.2: the view
//!
//! [`crate::arena::view`] is doc 17 §3's "one formatter" (D-UM-4): the pure
//! projections of a [`crate::arena::step::Frame`] — the contact rows
//! ([`crate::arena::view::ContactRow`], [`crate::arena::view::frame_contacts_held`]),
//! the range rounding and its memory, the band and scope words, the lock reads,
//! the weapon a bare `FIRE` fires and the `VIEW` glyphs — moved here from
//! `crate::match_::cockpit`, which re-exports them, so a pilot's `CONTACTS`
//! and a mind's view (17b cut M1a.5) read one function. Pure over a frame and a
//! range memory, so the no-I/O gate covers it as it covers every file here.
//!
//! What is still not here, and whose it is: deliberation, the order type, the
//! squad grouping and the cost run are cut N.2's, and the task, the inbox, the
//! egress, the timer and the unwind boundary are cut T's.

pub mod catchup;
pub mod clock;
pub mod command;
pub mod condition;
pub mod damage;
pub mod hash;
pub mod heat;
pub mod horizon;
pub mod index;
pub mod intents;
pub mod ledger;
pub mod motion;
pub mod motor;
pub mod observer;
pub mod path;
pub mod perception;
pub mod reflex;
pub mod resolution;
pub mod rng;
pub mod slope;
pub mod snapshot;
pub mod state;
pub mod step;
pub mod view;
pub mod weapons;

pub use catchup::{
    Catchup, MAX_CATCHUP_CONTACTS, MAX_CATCHUP_SECTIONS, MAX_CATCHUP_WEAPONS, MachineWords,
    ReleaseSnapshot, SectionWords,
};
pub use clock::{
    Clock, MAX_TEMPO_FLOOR_MS, MIN_SUBSTEP_MS, Tempo, TempoSignals, substep_ms, substeps,
};
pub use command::{
    BriefObjective, MAX_SIGNAL_CONTACTS, SignalContact, SignalReason, SquadDigest, SquadSignal,
    SquadWatch,
};
pub use condition::{Condition, ConditionBand, MAX_SCANNED_SECTIONS, Reading};
pub use damage::{
    BLOCKED, COVER, DISPERSION, HULL_SECTION, MISS_REASONS, MOVED, REFUSED, TARGET_GONE, WRECK,
    WRECK_CONDITION, is_wreck,
};
pub use hash::state_hash;
pub use heat::{COOL_CAUSE, MILLI, SHOT_CAUSE};
pub use horizon::{Horizon, HorizonSight};
pub use index::{SpatialIndex, range_cm};
pub use intents::{
    Control, ControlState, Intent, IntentKind, IntentOutcome, IntentSource, Refusal, apply_control,
    apply_intent, drain_pending, resolve_emcon, resolve_startup,
};
pub use ledger::{CHUNK_BYTES, EventKind, Ledger, LedgerChunk, LedgerEvent, LedgerFull};
pub use motion::{Held, MotionOutcome, cos_fx, integrate, sin_fx};
pub use motor::{Fallback, MotorOutcome, MotorPlan, MotorRow, MotorTable};
pub use observer::{Observer, ObserverUnit};
pub use path::{PathFailure, PathResult, PathScratch, min_move_cost, path_with};
pub use perception::{
    Aspect, FULL_SIGNATURE_PERMILLE, FrameCondition, FrameContact, FrameOdds, FrameOrder,
    MindObjective, Named, PerceptionCounters, SensorProfile, SignatureFactor, ThermalSignature,
    bearing_mdeg, bearing_of, contact_refusal, sees,
};
pub use reflex::{
    ARRIVE_CELLS, REFLEX_AVOID_MAX, REFLEX_AVOID_TTL_MS, REFLEX_STAND_TICKS, ReflexCounters,
    ReflexMemory, ReflexState, ReflexUnit,
};
pub use resolution::{
    BandProfile, Consequence, DIRECT_BANDS, Resolution, SectionProfile, UnitProfile, facet_band,
};
pub use rng::{DrawLabel, LABEL_KINDS, draw, draw_contact_id, draw_permille};
pub use slope::{
    SlopeLimit, driving_mode, effective_cost, grade_pct, steepness_pct, step_kind, too_steep,
};
pub use snapshot::{Snapshot, SnapshotError, SnapshotHead, digest_text};
pub use state::{
    Arena, ArenaDefinition, ArenaError, ArenaMode, ArenaTerrain, Channel, Classification,
    ContactId, Controller, CostRow, CustodyOut, CustodyUnit, DEFAULT_SENSOR_HEIGHT_CM, Designator,
    DesignatorBook, DueEvent, DueHeap, DueKind, EndReason, EntryZone, EpochTable, HeatState,
    HoldRule, HoldState, KeySpot, MatchEnd, MatchMode, MotionCarry, MotionLimits, NO_SUCH_CONTACT,
    PROVISIONAL_PREFIX, Pilot, Power, SPOT_LETTERS, ShadowContact, ShadowContacts, Side,
    StopReason, Team, TerrainCell, TrackedRules, UnitState, Waypoint, Waypoints, WeaponSummary,
    spot_letter,
};
pub use step::{Frame, FrameHeat, FrameSlots, STOCK_ODDS_ROWS, StepOutput};
pub use view::{
    ContactRow, OddsCell, contact_band, fired_by_default, frame_contacts, frame_contacts_held,
    frame_lock, held_range_m, lock_designator, own_scope, range_m_rounded, settle_ranges,
};
pub use weapons::{FrameBand, FrameWeapon, Shot, WeaponBand};

/// The fixtures every module's unit tests share: a window, a pose, a stock tank
/// and a one-call [`Arena`].
///
/// It is an **inline** `#[cfg(test)]` module rather than its own file on purpose.
/// `crates/mush-contract-tests/tests/panic_budget.rs` excludes brace-delimited
/// items carrying `#[cfg(test)]` but scans every other `.rs` file under a crate,
/// so a `test_support.rs` would put its `expect`s on tacsim's production panic
/// budget; the root `clippy.toml`'s `allow-*-in-tests` keys work the same way.
#[cfg(test)]
pub(crate) mod test_support {
    use mush_core::{
        DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId,
    };
    use mush_sim::data::Pose;
    use mush_tactics::{
        CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec,
        TacticalUnit, UnitSpec,
    };
    use smol_str::SmolStr;

    use crate::arena::perception::{SensorProfile, SignatureFactor};
    use crate::arena::resolution::{BandProfile, Consequence, SectionProfile, UnitProfile};
    use crate::arena::state::{
        Arena, ArenaDefinition, ArenaTerrain, CustodyOut, CustodyUnit, EntryZone, MatchMode, Pilot,
        Side, Team, TerrainCell, TrackedRules, WeaponSummary,
    };
    use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};
    use crate::definitions::resolution::ResolutionDef;

    /// The window every unit test lays down: 8 by 8 cells of 1,000 cm, all
    /// passable ground at cost 100.
    pub(crate) const TEST_WIDTH: u16 = 8;
    /// Rows of [`TEST_WIDTH`]'s window.
    pub(crate) const TEST_HEIGHT: u16 = 8;

    /// An exact definition identity with a zero digest — enough for
    /// [`TacticalUnit::from_spec`]'s kind and owner checks, which are all the
    /// arena ever exercises (it applies no plan).
    pub(crate) fn dref(kind: &str, id: &str) -> DefinitionRef {
        DefinitionRef::new(
            DefinitionKey::new(
                OwnerId::new("tacsim").expect("a canonical owner"),
                DefinitionKind::new(kind).expect("a canonical kind"),
                DefinitionId::new(id).expect("a canonical id"),
            ),
            1,
            DefinitionDigest::from_bytes([0_u8; 32]),
        )
        .expect("version 1 is not zero")
    }

    /// An empty opaque rules blob.
    pub(crate) fn rules() -> RulesValue {
        RulesValue::new(dref("schema", "tacsim.arena.test"), serde_json::json!({}))
            .expect("an empty rules blob")
    }

    /// The smallest M32 unit the arena can carry: one hull section, one front
    /// facet, one armor cell.
    pub(crate) fn tactical_unit(id: ObjId) -> TacticalUnit {
        tactical_unit_ruled(id, rules())
    }

    /// **[`tactical_unit`] with the rules blob `data`** (16b cut AE.3): the one
    /// thing [`crate::definitions::UnitClass::read`] reads a declared class
    /// off, so a fixture can field a drone without a catalog.
    pub(crate) fn tactical_unit_with(id: ObjId, data: serde_json::Value) -> TacticalUnit {
        tactical_unit_ruled(
            id,
            RulesValue::new(dref("schema", "tacsim.arena.test"), data).expect("a rules blob"),
        )
    }

    /// [`tactical_unit`]'s one shape, under any rules blob.
    fn tactical_unit_ruled(id: ObjId, rules: RulesValue) -> TacticalUnit {
        let spec = UnitSpec {
            definition: dref("unit", "tacsim.unit.line_tank"),
            sections: vec![SectionSpec {
                id: SectionId::new("hull").expect("a canonical section"),
                condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
                facets: vec![FacetSpec {
                    id: FacetId::new("front").expect("a canonical facet"),
                    cells: vec![CellSpec {
                        id: mush_tactics::CellId::new("armor").expect("a cell"),
                        min: 0,
                        max: 10,
                        init: 10,
                    }],
                }],
                slots: Vec::new(),
                cells: vec![CellSpec {
                    id: mush_tactics::CellId::new("internals").expect("a cell"),
                    min: 0,
                    max: 8,
                    init: 8,
                }],
            }],
            unit_cells: Vec::new(),
            components: Vec::new(),
            capabilities: Vec::new(),
            mobility: None,
            rules,
        };
        TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
    }

    /// **[`tactical_unit`] with one ground mode of `locomotion`** (16b cut AE.1)
    /// — the one thing [`crate::arena::state::CostRow::read`] and
    /// [`crate::arena::slope::SlopeLimit::read`] read off a unit, so a fixture can
    /// field a car or a walker without a catalog.
    pub(crate) fn tactical_unit_on(id: ObjId, locomotion: &str) -> TacticalUnit {
        use mush_tactics::{
            LocomotionId, MediumId, MobilityMode, MobilityModeId, MobilityProfile,
            MobilityVocabulary,
        };
        let mode_id =
            MobilityModeId::new(format!("ground.{locomotion}")).expect("a canonical mode id");
        let mode = MobilityMode::new(
            mode_id.clone(),
            [MediumId::new("ground").expect("a canonical medium")],
            LocomotionId::new(locomotion).expect("a canonical locomotion"),
            rules(),
        );
        let profile =
            MobilityProfile::new(vec![mode], mode_id, &MobilityVocabulary::base().freeze())
                .expect("a one-mode profile");
        let spec = UnitSpec {
            definition: dref("unit", "tacsim.unit.line_tank"),
            sections: vec![SectionSpec {
                id: SectionId::new("hull").expect("a canonical section"),
                condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
                facets: vec![FacetSpec {
                    id: FacetId::new("front").expect("a canonical facet"),
                    cells: vec![CellSpec {
                        id: mush_tactics::CellId::new("armor").expect("a cell"),
                        min: 0,
                        max: 10,
                        init: 10,
                    }],
                }],
                slots: Vec::new(),
                cells: vec![CellSpec {
                    id: mush_tactics::CellId::new("internals").expect("a cell"),
                    min: 0,
                    max: 8,
                    init: 8,
                }],
            }],
            unit_cells: Vec::new(),
            components: Vec::new(),
            capabilities: Vec::new(),
            mobility: Some(profile),
            rules: rules(),
        };
        TacticalUnit::from_spec(&spec, id).expect("a one-section unit with a mode")
    }

    /// **A cell of `class` priced as the loader prices it** (16b cut AE.1): the
    /// shipped table's `tracked` row as `move_cost`, `0` where it is `null`.
    pub(crate) fn priced_cell(class: mush_atlas::TerrainClass) -> TerrainCell {
        TerrainCell {
            class,
            elev_cm: 0,
            move_cost: mush_atlas::physics::LocomotionTable::shipped()
                .cost(class.label(), crate::definitions::unit::TRACKED, &[])
                .map_or(0, |cost| u16::try_from(cost).unwrap_or(u16::MAX)),
        }
    }

    /// **A window priced per row** (16b cut AE.1): `width` by `height` of
    /// `fill`, with `paint`ed classes, through [`ArenaTerrain::priced`] and the
    /// shipped table — the terrain `from_pack` would build over those classes.
    pub(crate) fn priced_terrain(
        width: u16,
        height: u16,
        fill: mush_atlas::TerrainClass,
        paint: &[((u16, u16), mush_atlas::TerrainClass)],
    ) -> ArenaTerrain {
        let mut cells = vec![priced_cell(fill); usize::from(width) * usize::from(height)];
        for ((x, y), class) in paint {
            if let Some(slot) =
                cells.get_mut(usize::from(*y) * usize::from(width) + usize::from(*x))
            {
                *slot = priced_cell(*class);
            }
        }
        ArenaTerrain::priced(
            width,
            height,
            1_000,
            cells,
            mush_atlas::physics::LocomotionTable::shipped(),
        )
        .expect("a priced window")
    }

    /// A passable ground cell at cost 100 (terrain factor 1).
    pub(crate) fn ground() -> TerrainCell {
        TerrainCell {
            class: mush_atlas::TerrainClass::Ground,
            elev_cm: 0,
            move_cost: 100,
        }
    }

    /// A pose at `(x_cm, y_cm)` facing north.
    pub(crate) fn pose(x_cm: i64, y_cm: i64) -> Pose {
        Pose::at(x_cm, y_cm, 0, rules())
    }

    /// The test window.
    ///
    /// **16b cut N.1 gave it a two-node patrol graph and one key spot**, which is
    /// the smallest graph `Patrol` can walk that is not a single point: the two
    /// nodes are the west and east edges of the middle row, joined, so a unit put
    /// down anywhere on the window has a nearest node and an onward one.
    pub(crate) fn definition() -> ArenaDefinition {
        ArenaDefinition {
            id: SmolStr::new("tacsim.arena.test"),
            terrain: ArenaTerrain::uniform(TEST_WIDTH, TEST_HEIGHT, 1_000, ground())
                .expect("the test window"),
            entry_zones: vec![
                EntryZone {
                    team: Team::Red,
                    x0: 0,
                    y0: 0,
                    x1: 1,
                    y1: 7,
                },
                EntryZone {
                    team: Team::Blue,
                    x0: 6,
                    y0: 0,
                    x1: 7,
                    y1: 7,
                },
            ],
            key_spots: vec![crate::arena::state::KeySpot {
                name: SmolStr::new("The Middle"),
                x: 4,
                y: 4,
            }],
            waypoints: crate::arena::state::Waypoints::new(
                vec![
                    crate::arena::state::Waypoint {
                        id: SmolStr::new("west"),
                        x: 1,
                        y: 4,
                    },
                    crate::arena::state::Waypoint {
                        id: SmolStr::new("east"),
                        x: 6,
                        y: 4,
                    },
                ],
                &[(SmolStr::new("west"), SmolStr::new("east"))],
            ),
            modes: Vec::new(),
        }
    }

    /// A stock tank: 1,000 cm/s forward, 40% in reverse, 200/400 cm/s²,
    /// 30 degrees a second. **No sensor and no weapon**, which is cut A's arena
    /// exactly: it runs no pass and derives an `Alert` range of zero.
    pub(crate) fn tank(id: ObjId, team: Team, pose: Pose) -> CustodyUnit {
        CustodyUnit {
            id,
            team,
            pose,
            mobility: TrackedRules {
                max_speed_cms: 1_000,
                accel_cms2: 200,
                brake_cms2: 400,
                turn_rate_mdeg_s: 30_000,
                reverse_factor_pct: 40,
            },
            sensor_height_cm: 150,
            sensor: None,
            weapons: Vec::new(),
            profile: UnitProfile::default(),
            gunnery: 0,
            // 16b cut N.1: a fixture tank is a pilot's until a test says otherwise,
            // which is the check-out's rule for a unit with a seat.
            controller: crate::arena::state::Controller::Pilot,
            // 16b cut Q.1: no lobby fielded it, so `Arena::new` gives it the
            // lowest free slot of its team in unit-id order.
            slot: 0,
            unit: tactical_unit(id),
        }
    }

    /// **A drone** (16b cut AE.3): [`tank`] — no sight, no weapon — whose rules
    /// blob declares `"class": "drone"`, so `Arena::new` reads it as
    /// [`crate::definitions::UnitClass::Drone`] and counts it against the cap.
    pub(crate) fn drone(id: ObjId, team: Team, pose: Pose) -> CustodyUnit {
        CustodyUnit {
            unit: tactical_unit_with(id, serde_json::json!({ "class": "drone" })),
            ..tank(id, team, pose)
        }
    }

    /// **The shipped sensor's numbers**, as the arena carries them
    /// (`content/sensors/visual_mk1.json`).
    pub(crate) fn visual() -> SensorProfile {
        SensorProfile {
            component: smol_str::SmolStr::new("visual_sensor"),
            max_range_cells: 60,
            max_attenuation: 40,
            classify_after_ms: 3_000,
            sees_wrecks: false,
            share_range_cells: 30,
            signature: vec![
                SignatureFactor {
                    class: mush_atlas::TerrainClass::Road,
                    moving_permille: 1_000,
                    stopped_permille: 1_000,
                },
                SignatureFactor {
                    class: mush_atlas::TerrainClass::Ground,
                    moving_permille: 1_000,
                    stopped_permille: 900,
                },
                SignatureFactor {
                    class: mush_atlas::TerrainClass::Grass,
                    moving_permille: 1_000,
                    stopped_permille: 800,
                },
                SignatureFactor {
                    class: mush_atlas::TerrainClass::Rubble,
                    moving_permille: 950,
                    stopped_permille: 700,
                },
                SignatureFactor {
                    class: mush_atlas::TerrainClass::WoodsLight,
                    moving_permille: 900,
                    stopped_permille: 650,
                },
                SignatureFactor {
                    class: mush_atlas::TerrainClass::WoodsHeavy,
                    moving_permille: 800,
                    stopped_permille: 500,
                },
            ],
        }
    }

    /// A cannon: 2,500 m of reach, two range bands, a four-second cycle and a
    /// forty-round load.
    ///
    /// **16b cut E.1**: twice that reach is 5,000 m, and the derived
    /// `tempo.alert_range_cm` is the lesser of it and the window's own diagonal —
    /// which on every window this crate builds is the diagonal (11,000 cm on the
    /// eight-cell fixture, 22,000 on the sixteen-cell one), never 5,000 m.
    ///
    /// Close to `content/weapons/cannon_105.json` and deliberately not equal to it:
    /// the authored numbers are cut C's and a unit test that asserted them would be
    /// asserting the content. What it is exact about is the *shape* — bands running
    /// outward with a dispersion each, a load, a burst of one, a muzzle speed — which
    /// is what cut W.1 reads.
    pub(crate) fn cannon() -> WeaponSummary {
        WeaponSummary {
            id: SmolStr::new("cannon"),
            max_range_cm: 250_000,
            cycle_ms: 4_000,
            ammunition: SmolStr::new("tacsim.ammunition.he_105"),
            burst_rounds: 1,
            muzzle_speed_cms: 145_000,
            bands: vec![
                crate::arena::weapons::WeaponBand {
                    max_range_cm: 150_000,
                    dispersion_mrad: 3,
                },
                crate::arena::weapons::WeaponBand {
                    max_range_cm: 250_000,
                    dispersion_mrad: 6,
                },
            ],
            count_per_load: 40,
            // 16b cut W.2: `content/ammunition/he_105.json`'s own figure, which is
            // what makes four rounds through a 400 cm facet and one more through
            // 200 internals the arithmetic every damage test reads.
            damage: 120,
            mount_section: SmolStr::new("turret"),
        }
    }

    /// A tank that sees and shoots: [`tank`] with the shipped sensor and the
    /// shipped cannon, and **no** silhouette or facet table — which is cut A's and
    /// cut S's arena exactly: it can fire, and every shot resolves at zero because
    /// there is nothing authored for a round to strike.
    pub(crate) fn scout(id: ObjId, team: Team, pose: Pose) -> CustodyUnit {
        CustodyUnit {
            sensor: Some(visual()),
            weapons: vec![cannon()],
            ..tank(id, team, pose)
        }
    }

    /// **The stock tank's structure**, as `content/units/line_tank.json` authors it
    /// (16b cut W.2): five sections with their armour and internals, the five facet
    /// bands with their silhouettes and weights, and the consequences.
    ///
    /// Close to the content and deliberately not loaded from it: the authored numbers
    /// are cut C's and a unit test that read the pack would be asserting the content
    /// rather than the arithmetic. What it is exact about is the *shape* — a
    /// front-heavy hull, two tracks with one facet each, an engine struck through the
    /// hull's rear, a turret whose loss takes the weapons — which is what damage
    /// reads. `crates/tacsim/tests/resolution.rs` pins the figures below equal to the
    /// shipped pack, so the two cannot drift apart unnoticed.
    pub(crate) fn line_tank_unit(id: ObjId) -> TacticalUnit {
        let facet = |name: &str, armor: i64| FacetSpec {
            id: FacetId::new(name).expect("a canonical facet"),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("armor").expect("a cell"),
                min: 0,
                max: armor,
                init: armor,
            }],
        };
        let section = |name: &str, facets: Vec<FacetSpec>, internals: i64| SectionSpec {
            id: SectionId::new(name).expect("a canonical section"),
            condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
            facets,
            slots: Vec::new(),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: internals,
                init: internals,
            }],
        };
        let component =
            |name: &str, section: &str, cells: Vec<CellSpec>| mush_tactics::ComponentSpec {
                id: mush_tactics::ComponentId::new(name).expect("a component"),
                definition: dref("component", &format!("tacsim.unit.line_tank.{name}")),
                install: mush_tactics::InstallPoint::Section(
                    SectionId::new(section).expect("a canonical section"),
                ),
                condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
                cells,
                asset: None,
            };
        let spec = UnitSpec {
            definition: dref("unit", "tacsim.unit.line_tank"),
            sections: vec![
                section(
                    "hull",
                    vec![
                        facet("front", 400),
                        facet("left", 250),
                        facet("right", 250),
                        facet("rear", 150),
                        facet("top", 100),
                    ],
                    200,
                ),
                section(
                    "turret",
                    vec![
                        facet("front", 350),
                        facet("left", 220),
                        facet("right", 220),
                        facet("rear", 140),
                    ],
                    150,
                ),
                section("left_track", vec![facet("left", 120)], 100),
                section("right_track", vec![facet("right", 120)], 100),
                section("engine", Vec::new(), 120),
            ],
            unit_cells: Vec::new(),
            components: vec![
                component(
                    "cannon",
                    "turret",
                    vec![
                        CellSpec {
                            id: mush_tactics::CellId::new("ammo").expect("a cell"),
                            min: 0,
                            max: 40,
                            init: 40,
                        },
                        CellSpec {
                            id: mush_tactics::CellId::new("ready_at_ms").expect("a cell"),
                            min: 0,
                            max: 3_600_000,
                            init: 0,
                        },
                    ],
                ),
                component(
                    "visual_sensor",
                    "turret",
                    vec![CellSpec {
                        id: mush_tactics::CellId::new("condition").expect("a cell"),
                        min: 0,
                        max: 100,
                        init: 100,
                    }],
                ),
            ],
            capabilities: vec![mush_tactics::CapabilityInit {
                key: mush_tactics::CapabilityKey {
                    id: mush_tactics::CapabilityId::new("tacsim.power").expect("a capability"),
                    source: None,
                },
                rules: rules(),
            }],
            mobility: None,
            rules: rules(),
        };
        TacticalUnit::from_spec(&spec, id).expect("the stock tank")
    }

    /// The stock tank's own facet table and silhouettes, as the arena carries them
    /// (`content/units/line_tank.json`).
    pub(crate) fn line_tank_profile() -> UnitProfile {
        let band = |name: &str, width: u32, height: u32, sections: &[(&str, u16)]| BandProfile {
            band: SmolStr::new(name),
            width_cm: width,
            height_cm: height,
            sections: sections
                .iter()
                .map(|(section, weight)| (SmolStr::new(*section), *weight))
                .collect(),
        };
        let section =
            |name: &str, facets: &[&str], consequences: Vec<Consequence>| SectionProfile {
                id: SmolStr::new(name),
                facets: facets.iter().map(|facet| SmolStr::new(*facet)).collect(),
                consequences,
                struck_through: None,
            };
        UnitProfile {
            bands: vec![
                band("front", 350, 250, &[("hull", 50), ("turret", 50)]),
                band(
                    "left",
                    700,
                    250,
                    &[("hull", 40), ("left_track", 30), ("turret", 30)],
                ),
                band(
                    "right",
                    700,
                    250,
                    &[("hull", 40), ("right_track", 30), ("turret", 30)],
                ),
                band(
                    "rear",
                    350,
                    250,
                    &[("hull", 40), ("engine", 30), ("turret", 30)],
                ),
                band("top", 700, 350, &[("hull", 50), ("turret", 50)]),
            ],
            sections: vec![
                section(
                    "hull",
                    &["front", "left", "right", "rear", "top"],
                    vec![Consequence::DestroyUnit],
                ),
                section(
                    "turret",
                    &["front", "left", "right", "rear"],
                    vec![Consequence::RemoveWeapons {
                        section: SmolStr::new("turret"),
                    }],
                ),
                // 16b cut M.4 (`B-47`): the pair rule, as the version-2 pack
                // authors it on both tracks.
                section(
                    "left_track",
                    &["left"],
                    vec![
                        Consequence::Slow { percent: 50 },
                        Consequence::NoTurnToward { side: Side::Left },
                        Consequence::DisableMotionWhenAllOf {
                            sections: vec![SmolStr::new("left_track"), SmolStr::new("right_track")],
                        },
                    ],
                ),
                section(
                    "right_track",
                    &["right"],
                    vec![
                        Consequence::Slow { percent: 50 },
                        Consequence::NoTurnToward { side: Side::Right },
                        Consequence::DisableMotionWhenAllOf {
                            sections: vec![SmolStr::new("left_track"), SmolStr::new("right_track")],
                        },
                    ],
                ),
                SectionProfile {
                    id: SmolStr::new("engine"),
                    facets: Vec::new(),
                    consequences: vec![Consequence::DisableMotion],
                    struck_through: Some((SmolStr::new("hull"), SmolStr::new("rear"))),
                },
            ],
        }
    }

    /// **The stock tank**, whole: the sight, the cannon, the structure and the facet
    /// table — the fixture every damage test fights with.
    pub(crate) fn line_tank(id: ObjId, team: Team, pose: Pose) -> CustodyUnit {
        CustodyUnit {
            sensor: Some(visual()),
            weapons: vec![cannon()],
            profile: line_tank_profile(),
            unit: line_tank_unit(id),
            ..tank(id, team, pose)
        }
    }

    /// **A laser** (16b cut X.1; doc 16 §9 item 1): a beam, so `muzzle_speed_cms`
    /// is zero and its impact is scheduled at `now`.
    ///
    /// Close to `content/weapons/laser_mk1.json` and deliberately not equal to it,
    /// for the reason [`cannon`] gives. What matters here is the shape: zero muzzle
    /// speed, and a shot that costs heat.
    pub(crate) fn laser() -> WeaponSummary {
        WeaponSummary {
            id: SmolStr::new("laser"),
            max_range_cm: 240_000,
            cycle_ms: 2_000,
            ammunition: SmolStr::new("tacsim.ammunition.laser_charge"),
            burst_rounds: 1,
            muzzle_speed_cms: 0,
            bands: vec![
                crate::arena::weapons::WeaponBand {
                    max_range_cm: 120_000,
                    dispersion_mrad: 2,
                },
                crate::arena::weapons::WeaponBand {
                    max_range_cm: 240_000,
                    dispersion_mrad: 5,
                },
            ],
            count_per_load: 500,
            damage: 25,
            mount_section: SmolStr::new("turret"),
        }
    }

    /// **The stock tank's structure with heat in it** (16b cut X.1): the same five
    /// sections, a `heat` and a `heat_sink` cell on the hull, and a laser in the
    /// turret whose component carries what one shot of it costs.
    ///
    /// The cells are laid down here by hand exactly as
    /// `crate::definitions::unit::bind` lays them down from
    /// `content/units/line_tank_laser.json`; `crates/tacsim/tests/extensions.rs`
    /// drives the authored route end to end, and this is the fixture the modules'
    /// own unit tests use without a pack loader.
    pub(crate) fn laser_tank_unit(id: ObjId) -> TacticalUnit {
        let facet = |name: &str, armor: i64| FacetSpec {
            id: FacetId::new(name).expect("a canonical facet"),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("armor").expect("a cell"),
                min: 0,
                max: armor,
                init: armor,
            }],
        };
        let cell = |name: &str, max: i64, init: i64| CellSpec {
            id: mush_tactics::CellId::new(name).expect("a cell"),
            min: 0,
            max,
            init,
        };
        let section = |name: &str, facets: Vec<FacetSpec>, internals: i64, cells: Vec<CellSpec>| {
            let mut all = vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: internals,
                init: internals,
            }];
            all.extend(cells);
            SectionSpec {
                id: SectionId::new(name).expect("a canonical section"),
                condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
                facets,
                slots: Vec::new(),
                cells: all,
            }
        };
        let component =
            |name: &str, section: &str, cells: Vec<CellSpec>| mush_tactics::ComponentSpec {
                id: mush_tactics::ComponentId::new(name).expect("a component"),
                definition: dref("component", &format!("tacsim.unit.line_tank_laser.{name}")),
                install: mush_tactics::InstallPoint::Section(
                    SectionId::new(section).expect("a canonical section"),
                ),
                condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
                cells,
                asset: None,
            };
        let spec = UnitSpec {
            definition: dref("unit", "tacsim.unit.line_tank_laser"),
            sections: vec![
                section(
                    "hull",
                    vec![
                        facet("front", 400),
                        facet("left", 250),
                        facet("right", 250),
                        facet("rear", 150),
                        facet("top", 100),
                    ],
                    200,
                    vec![
                        cell(crate::definitions::unit::HEAT_CELL, 60, 0),
                        cell(crate::definitions::unit::HEAT_SINK_CELL, 3, 3),
                    ],
                ),
                section(
                    "turret",
                    vec![
                        facet("front", 350),
                        facet("left", 220),
                        facet("right", 220),
                        facet("rear", 140),
                    ],
                    150,
                    Vec::new(),
                ),
                section("left_track", vec![facet("left", 120)], 100, Vec::new()),
                section("right_track", vec![facet("right", 120)], 100, Vec::new()),
                section("engine", Vec::new(), 120, Vec::new()),
            ],
            unit_cells: Vec::new(),
            components: vec![
                component(
                    "laser",
                    "turret",
                    vec![
                        cell("ammo", 500, 500),
                        cell("ready_at_ms", 3_600_000, 0),
                        cell(crate::definitions::unit::HEAT_PER_SHOT_CELL, 20, 20),
                    ],
                ),
                component("visual_sensor", "turret", vec![cell("condition", 100, 100)]),
            ],
            capabilities: vec![mush_tactics::CapabilityInit {
                key: mush_tactics::CapabilityKey {
                    id: mush_tactics::CapabilityId::new("tacsim.power").expect("a capability"),
                    source: None,
                },
                rules: rules(),
            }],
            mobility: None,
            rules: rules(),
        };
        TacticalUnit::from_spec(&spec, id).expect("the laser tank")
    }

    /// **The laser tank**, whole (16b cut X.1): the stock tank's structure and
    /// facet table, a beam in the turret and a heat cell on the hull.
    pub(crate) fn laser_tank(id: ObjId, team: Team, pose: Pose) -> CustodyUnit {
        CustodyUnit {
            sensor: Some(visual()),
            weapons: vec![laser()],
            profile: line_tank_profile(),
            unit: laser_tank_unit(id),
            ..tank(id, team, pose)
        }
    }

    /// **The shipped resolution table**, as the arena carries it
    /// (`content/resolution.json`, pinned equal to it by
    /// `crates/tacsim/tests/resolution.rs`).
    pub(crate) fn resolution() -> ResolutionDef {
        crate::match_::stock_resolution()
    }

    /// A check-out message over the test window with seed `2350`.
    pub(crate) fn custody(units: Vec<CustodyUnit>, pilots: Vec<Pilot>) -> CustodyOut {
        CustodyOut {
            definition: definition(),
            units,
            pilots,
            resolution: resolution(),
            seed: 2350,
            mode: MatchMode::Skirmish,
        }
    }

    /// An arena of `units` with no pilots seated and default configuration.
    pub(crate) fn arena_of(units: Vec<CustodyUnit>) -> Arena {
        Arena::new(
            custody(units, Vec::new()),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("the test arena")
    }

    /// The same, over a window with `paint`ed cells — the shape a perception test
    /// wants, because the property under test is *which ground the target stands
    /// on*.
    pub(crate) fn arena_painted(
        units: Vec<CustodyUnit>,
        paint: &[(u16, u16, TerrainCell)],
    ) -> Arena {
        let mut cells = vec![ground(); usize::from(TEST_WIDTH) * usize::from(TEST_HEIGHT)];
        for (x, y, cell) in paint {
            if let Some(slot) =
                cells.get_mut(usize::from(*y) * usize::from(TEST_WIDTH) + usize::from(*x))
            {
                *slot = *cell;
            }
        }
        let definition = ArenaDefinition {
            terrain: ArenaTerrain::new(TEST_WIDTH, TEST_HEIGHT, 1_000, cells)
                .expect("the painted window"),
            ..definition()
        };
        Arena::new(
            CustodyOut {
                definition,
                units,
                pilots: Vec::new(),
                resolution: resolution(),
                seed: 2_350,
                mode: MatchMode::Skirmish,
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("the painted test arena")
    }

    /// An arena of `units` whose every channel is at epoch `epoch`, with one
    /// seated pilot per unit (`#1000`, `#1001`, … in unit order).
    pub(crate) fn arena_at_epoch(units: Vec<CustodyUnit>, epoch: u64) -> Arena {
        let pilots = units
            .iter()
            .enumerate()
            .map(|(seat, unit)| Pilot {
                unit: unit.id,
                pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
                station: SmolStr::new("cockpit"),
                epoch,
            })
            .collect();
        Arena::new(
            custody(units, pilots),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("the test arena")
    }
}
