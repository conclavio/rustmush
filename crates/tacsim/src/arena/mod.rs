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
//! `arena_task` is the thin wrapper that owns an [`state::Arena`], the bounded
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
//! ([`hash::state_hash`]) is a stable function of the state rather than of the
//! machine.
//!
//! # The two halves of cut A
//!
//! **A.1 (this commit)** is the data, the time, the inputs, the randomness, the
//! hashing and the ledger: [`state`], [`clock`], [`intents`], [`rng`], [`hash`],
//! [`ledger`], their unit tests, and the two gates doc 16 §8 names. Its public
//! surface is a contract.
//!
//! **A.2** adds `index.rs` (the uniform grid of cell buckets, rebuilt every
//! substep), `motion.rs` (doc 16 §3.4 per substep over `move_cost`) and `step.rs`
//! (the five pulse phases of doc 16 §3.1, in order, each reading what the
//! previous committed) on exactly the surface below, plus the cross-tempo
//! property test. **There is deliberately no `step` here**: A.1 builds
//! everything `step` needs and schedules nothing it does not own, so that the
//! seam between the two halves is a compile error rather than a judgement call.

pub mod clock;
pub mod hash;
pub mod intents;
pub mod ledger;
pub mod rng;
pub mod state;

pub use clock::{Clock, MIN_SUBSTEP_MS, Tempo, TempoSignals, substep_ms, substeps};
pub use hash::state_hash;
pub use intents::{
    Control, ControlState, Intent, IntentKind, IntentOutcome, Refusal, apply_control, apply_intent,
    drain_pending, resolve_startup,
};
pub use ledger::{CHUNK_BYTES, EventKind, Ledger, LedgerChunk, LedgerEvent, LedgerFull};
pub use rng::{DrawLabel, LABEL_KINDS, draw, draw_contact_id, draw_permille};
pub use state::{
    Arena, ArenaDefinition, ArenaError, ArenaTerrain, Channel, Classification, ContactId,
    CustodyOut, CustodyUnit, DueEvent, DueHeap, DueKind, EndReason, EntryZone, EpochTable,
    MatchEnd, MotionLimits, Pilot, Power, ShadowContact, ShadowContacts, Side, StopReason, Team,
    TerrainCell, TrackedRules, UnitState,
};

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

    use crate::arena::state::{
        Arena, ArenaDefinition, ArenaTerrain, CustodyOut, CustodyUnit, EntryZone, Pilot, Team,
        TerrainCell, TrackedRules,
    };
    use crate::config::{ArenaConfig, MatchConfig};

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
            rules: rules(),
        };
        TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
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
        }
    }

    /// A stock tank: 1,000 cm/s forward, 40% in reverse, 200/400 cm/s²,
    /// 30 degrees a second.
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
            unit: tactical_unit(id),
        }
    }

    /// A check-out message over the test window with seed `2350`.
    pub(crate) fn custody(units: Vec<CustodyUnit>, pilots: Vec<Pilot>) -> CustodyOut {
        CustodyOut {
            definition: definition(),
            units,
            pilots,
            seed: 2350,
        }
    }

    /// An arena of `units` with no pilots seated and default configuration.
    pub(crate) fn arena_of(units: Vec<CustodyUnit>) -> Arena {
        Arena::new(
            custody(units, Vec::new()),
            &ArenaConfig::default(),
            &MatchConfig::default(),
        )
        .expect("the test arena")
    }

    /// An arena of `units` whose every channel is at epoch `epoch`.
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
        )
        .expect("the test arena")
    }
}
