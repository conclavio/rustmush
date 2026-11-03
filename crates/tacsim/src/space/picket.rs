//! **Kirkwood Customs Picket — Rhai `MindRunner` Doctrines, Multi-Party Triad Combat,
//! Live `stow.ore` + Registry Inspection, Ledger Fine & Customs Assertion Front (`SF.R6`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §9 Story 1, §10.2 `SF.R6`,
//! `D-35-1`, `D-35-10`, `D-35-11`, `D-35-12`, `docs/design/35a-space-flight-review.md` §1, §4, §8.3).
//!
//! Connects a promoted Kirkwood interdiction [`LocalSpaceBubble`] to:
//! 1. **Three distinct [`crate::arena::party::PartyId`] parties**:
//!    - [`CARRIER_PARTY`] (`PartyId::nth(0)` = `'A'`): The player's `Dray-P` carrier.
//!    - [`CUSTOMS_CORVETTE_PARTY`] (`PartyId::nth(1)` = `'B'`): **Republic Customs Corvette**
//!      (`SpaceRelation::Inspecting` / `Relation::Neutral` toward the carrier; `SpaceRelation::Hostile`
//!      toward the Belt Raider), flown by a compiled [`RhaiRunner`] executing [`KIRKWOOD_PICKET_RHAI`].
//!    - [`BELT_RAIDER_PARTY`] (`PartyId::nth(2)` = `'C'`): **Belt Raider** (`SpaceRelation::Hostile`
//!      toward both the carrier and the Republic Customs Corvette), flown by a compiled
//!      [`RhaiRunner`] executing [`BELT_RAIDER_RHAI`].
//! 2. **Light-lagged radio hail (`D-35-10`, `35a` §4 row 2)**:
//!    - Computes light propagation delay via [`mush_propagation::resolve_delivery`] and emits
//!      both hull narration and a structured `"tacsim:notice"` event ordering the carrier to
//!      heave-to ($|\Delta \mathbf{v}| < 10\text{ m/s}$ = [`HEAVE_TO_REL_VEL_MMPS`]).
//! 3. **Live `stow.ore` & Freeport Registry inspection (`D-35-11`, `35a` §1)**:
//!    - Reads the carrier hull's actual `stow.ore` vault (`strategic_ore`, `metal_ore`, `ice_ore`,
//!      `carbon_ore`) and checks the pilot's live Freeport Registry prospecting licence
//!      ([`crate::port::licence::licence_gate`]) and `tacsim.claims` tenure.
//!    - Submitting to inspection (`+helm/submit`) while hove-to ($|\Delta \mathbf{v}| < 10\text{ m/s}$)
//!      with unlicensed `strategic_ore` assesses a real `mush-economy` Public E4 contract fine
//!      into `freeport_registry`'s account and advances the [`mush_faction::CUSTOMS_ASSERTION_TITLE`]
//!      campaign front via [`mush_faction::record_customs_assertion`].
//!    - Burning past [`PICKET_RUN_BOUNDARY_M`] (`35 km`) or running `+helm/run` without clearing
//!      inspection records [`PicketInspectionOutcome::RanThePicket`], escalates the Republic
//!      Customs Corvette to [`SpaceRelation::Hostile`], and advances the
//!      [`mush_faction::CUSTOMS_ASSERTION_TITLE`] front.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::{
    AcceptOutcome, AccountId, AccountStatus, Amount, ConsiderationSpec, ContractId, ContractRoles,
    ContractSpec, EconomicClock, EconomicDeadline, EconomicVisibility, EconomyStore, EvidenceKind,
    Expiry, OutcomeTerms, PayoutShare, Share, TypedCondition,
};
use mush_faction::FrontId;
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use smol_str::SmolStr;

use super::bubble::{
    BubbleInputCommand, BubbleInputTape, EmconPosture, HullStatus, LocalSpaceBubble, SpacePartyId,
    SpaceRelation, SpaceShipState,
};
use super::weapons::{LaserMode, isqrt_i128};
use crate::arena::card::line;
use crate::arena::core::{ClassTable, CoreClass};
use crate::arena::intents::{IntentKind, MotorGoal};
use crate::arena::mind::{MindRunner, MindView, ThinkJob, ThinkResult, ViewContact};
use crate::arena::perception::{Aspect, FrameCondition, FrameNotice, Receiver as FrameReceiver};
use crate::arena::profile_rules::BoundProgram;
use crate::arena::profile_runtime::ProfileToken;
use crate::arena::runtime_card::{CardCatalog, ProfileKey, stock_definition};
use crate::arena::state::{
    ArenaDefinition, ArenaTerrain, Classification, ContactId, Designator, MatchMode, Power,
    TerrainCell, Waypoints,
};
use crate::arena::step::FrameHeat;
use crate::arena::weapons::FrameWeapon;
use crate::config::{ArenaConfig, ReflexConfig};
use crate::minds::RhaiRunner;
use crate::port::apply::{credit_scale, minor_of};
use crate::port::licence::{LICENCE_SETTLE_WINDOW_SECS, licence_gate, licence_port};

/// Party A (`PartyId::nth(0)` = `'A'`): Promoted player carrier (`Dray-P`).
pub const CARRIER_PARTY: SpacePartyId = SpacePartyId(1);

/// Party B (`PartyId::nth(1)` = `'B'`): **Republic Customs Corvette** (`Neutral`/`Inspecting` to carrier, `Hostile` to raider).
pub const CUSTOMS_CORVETTE_PARTY: SpacePartyId = SpacePartyId(2);

/// Party C (`PartyId::nth(2)` = `'C'`): **Belt Raider** (`Hostile` to both carrier and customs corvette).
pub const BELT_RAIDER_PARTY: SpacePartyId = SpacePartyId(3);

/// Initial standoff distance ($6\text{ km} = 6{,}000\text{ m}$) of the Republic Customs Corvette
/// when opening a Kirkwood Customs Picket bubble (`doc 35` §9 Story 1 step 2).
pub const CORVETTE_STANDOFF_M: i64 = 6_000;

/// Maximum 3D Euclidean relative speed ($10.0\text{ m/s} = 10{,}000\text{ mm/s}$) required to
/// heave-to for Kirkwood customs inspection (`doc 35` §9 Story 1 step 2).
pub const HEAVE_TO_REL_VEL_MMPS: i64 = 10_000;

/// Distance boundary ($35\text{ km} = 35{,}000\text{ m}$) from the Republic Customs Corvette
/// beyond which an uninspected carrier is marked as having run the picket (`D-35-11`).
pub const PICKET_RUN_BOUNDARY_M: i64 = 35_000;

/// Initial standoff distance ($120\text{ km} = 120{,}000\text{ m}$) at which the lurking
/// Belt Raider opens its ambush railgun & torpedo attack (`doc 35` §9 Story 1 step 3).
pub const RAIDER_AMBUSH_RANGE_M: i64 = 120_000;

/// Customs fine in major credits (`cr`) per tonne of unlicensed `strategic_ore` (`D-35-11`).
pub const CUSTOMS_FINE_MAJOR_PER_TONNE: i64 = 500;

/// Minimum customs fine in major credits (`1,000 cr`) when contraband `strategic_ore` is found.
pub const CUSTOMS_FINE_MIN_MAJOR: i64 = 1_000;

/// Contract name for the Republic Customs contraband fine (`D-35-11`).
pub const CUSTOMS_FINE_CONTRACT_NAME: &str = "Republic Customs Contraband Fine";

/// Compiled Rhai doctrine script for the **Republic Customs Corvette** (`D-35-1`, `D-35-11`).
///
/// Executed through [`RhaiRunner`] (`MindRunner::think`) on every bubble step:
/// - Tracks `this.stage` in persistent Rhai memory (`0` = hail, `1` = await_heave_to,
///   `2` = inspecting, `3` = screen_defend, `4` = hostile_pursuit, `5` = cleared).
/// - Emits station intents (`lock`, `hold`, `emcon`, `fire`) and trace signals read by
///   the Kirkwood picket controller.
pub const KIRKWOOD_PICKET_RHAI: &str = r#"
fn think(v, c) {
    if !("stage" in this) {
        this.stage = 0;
    }
    let carrier_id = ();
    let carrier_range = 0;
    let carrier_moving = true;
    let hostile_id = ();
    let hostile_range = 0;
    for row in v.contacts {
        if row.relation == "hostile" && row.reads_dead != true {
            if hostile_id == () || row.range_m < hostile_range {
                hostile_id = row.id;
                hostile_range = row.range_m;
            }
        } else if row.relation == "neutral" && row.reads_dead != true {
            if carrier_id == () || row.range_m < carrier_range {
                carrier_id = row.id;
                carrier_range = row.range_m;
                carrier_moving = row.moving;
            }
        }
    }

    if hostile_id != () {
        this.stage = 3;
        emcon(true);
        lock(hostile_id);
        for w in v.weapons {
            if w.ready_in_ms == 0 && w.id == "laser_1" {
                fire(w.id, 1);
                break;
            }
        }
        trace("PICKET_SCREEN_ENGAGE_RAIDER");
        return;
    }

    if carrier_id != () {
        if carrier_range > 35000 && this.stage != 5 {
            this.stage = 4;
            emcon(true);
            lock(carrier_id);
            for w in v.weapons {
                if w.ready_in_ms == 0 {
                    fire(w.id, 1);
                    break;
                }
            }
            trace("PICKET_RAN_THE_PICKET");
            return;
        }
        if this.stage == 0 {
            lock(carrier_id);
            hold();
            this.stage = 1;
            trace("PICKET_HAIL_HEAVE_TO");
            return;
        }
        if !carrier_moving {
            hold();
            this.stage = 2;
            trace("PICKET_CARRIER_HOVE_TO_READY");
            return;
        }
        lock(carrier_id);
        hold();
        trace("PICKET_AWAIT_HEAVE_TO");
    }
}
"#;

/// Compiled Rhai doctrine script for the lurking **Belt Raider** (`D-35-1`, `D-35-11`).
///
/// Executed through [`RhaiRunner`] (`MindRunner::think`) on every bubble step:
/// - Waits in `0` (`lurk`) while the carrier decelerates for inspection, then lights its torch
///   (`emcon(true)`), locks the carrier, launches a 2-torpedo salvo and fires a 120 km
///   world-frame railgun shot (`"RAIDER_TORCH_AND_SALVO"`), and presses the attack (`2`).
pub const BELT_RAIDER_RHAI: &str = r#"
fn think(v, c) {
    if !("stage" in this) {
        this.stage = 0;
    }
    let target_id = ();
    let target_range = 0;
    for row in v.contacts {
        if row.relation == "hostile" && row.reads_dead != true {
            target_id = row.id;
            target_range = row.range_m;
            break;
        }
    }
    if target_id == () {
        return;
    }
    if this.stage == 0 {
        this.stage = 1;
        emcon(true);
        lock(target_id);
        speed(2500);
        for w in v.weapons {
            if w.ready_in_ms == 0 {
                fire(w.id, 1);
            }
        }
        trace("RAIDER_TORCH_AND_SALVO");
        return;
    }
    this.stage = 2;
    lock(target_id);
    trace("RAIDER_PRESS_ATTACK");
}
"#;

/// Live cargo & Registry claim manifest read from the carrier hull's actual `stow.ore` vault
/// and `tacsim.licence` / `tacsim.claims` records (`D-35-11`, `35a` §1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveCustomsManifest {
    /// Total `strategic_ore` in the hull's `stow.ore` vault in kilograms.
    pub strategic_ore_kg: u64,
    /// Total `metal_ore` in the hull's `stow.ore` vault in kilograms.
    pub metal_ore_kg: u64,
    /// Total `ice_ore` in the hull's `stow.ore` vault in kilograms.
    pub ice_ore_kg: u64,
    /// Total `carbon_ore` in the hull's `stow.ore` vault in kilograms.
    pub carbon_ore_kg: u64,
    /// Whether the pilot holds a valid Freeport Registry prospecting licence (`licence_gate`).
    pub licensed: bool,
    /// Whether the pilot holds an active `BlockTenure::Claimed` stake in `tacsim.claims`.
    pub has_registered_claim: bool,
    /// Quantity of `strategic_ore` (in kilograms) deemed unlicensed/unclaimed contraband.
    pub unlicensed_strategic_ore_kg: u64,
}

/// Current stage of the Kirkwood Customs Picket inspection (`D-35-11`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PicketInspectionStatus {
    /// Picket has hailed on light-lagged radio; waiting for the carrier to heave-to ($|\Delta \mathbf{v}| < 10\text{ m/s}$).
    #[default]
    AwaitingHeaveTo,
    /// Carrier has matched velocity within $|\Delta \mathbf{v}| < 10\text{ m/s}$ and may submit to inspection (`+helm/submit`).
    HoveToReadyForInspection,
    /// Carrier submitted to inspection with valid Registry papers (or zero contraband) and is cleared.
    Cleared,
    /// Carrier submitted to inspection with unlicensed `strategic_ore`, paid the Ledger fine, and is cleared.
    ClearedAfterFine,
    /// Carrier burned past the picket perimeter (`> 35 km`) or ran the picket without clearing inspection.
    RanThePicket,
}

/// Recorded outcome of a Kirkwood Customs Picket inspection or perimeter breach (`D-35-11`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PicketInspectionOutcome {
    /// Carrier cleared customs inspection with no contraband violation.
    Cleared {
        /// Inspected `strategic_ore` quantity in kilograms.
        strategic_ore_kg: u64,
    },
    /// Carrier fined via a real `mush-economy` Public E4 contract into `freeport_registry`'s
    /// account and recorded on the [`mush_faction::CUSTOMS_ASSERTION_TITLE`] campaign front.
    Fined {
        /// Activated `mush-economy` contract id.
        contract_id: u64,
        /// Fine amount in minor credit units.
        fine_minor: i64,
        /// Unlicensed `strategic_ore` quantity in kilograms that triggered the fine.
        unlicensed_strategic_ore_kg: u64,
        /// Campaign front id (`"Customs Assertion"`) opened or advanced in `mush-faction`.
        front_id: Option<u64>,
    },
    /// Carrier ran the picket without clearing inspection, escalating the Republic Customs
    /// Corvette to [`SpaceRelation::Hostile`] and advancing [`mush_faction::CUSTOMS_ASSERTION_TITLE`].
    RanThePicket {
        /// Campaign front id (`"Customs Assertion"`) opened or advanced in `mush-faction`.
        front_id: Option<u64>,
    },
}

/// Per-bubble state for the multi-party Kirkwood Customs Picket (`SF.R6`, `D-35-11`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KirkwoodPicketState {
    /// Republic Customs Corvette hull object id.
    pub corvette_hull: ObjId,
    /// Republic Customs Corvette ship id in [`LocalSpaceBubble::ships`].
    pub corvette_ship_id: String,
    /// Belt Raider hull object id.
    pub raider_hull: ObjId,
    /// Belt Raider ship id in [`LocalSpaceBubble::ships`].
    pub raider_ship_id: String,
    /// Whether the light-lagged radio hail has been transmitted to the carrier.
    pub hail_sent: bool,
    /// Light propagation delay of the opening radio hail in milliseconds.
    pub hail_light_lag_ms: u64,
    /// Current customs inspection status.
    pub inspection_status: PicketInspectionStatus,
    /// Final customs inspection outcome, once resolved (`Cleared`, `Fined`, or `RanThePicket`).
    pub inspection_outcome: Option<PicketInspectionOutcome>,
    /// Whether the Belt Raider has lit its torch and launched its opening 2-torpedo + 120 km railgun ambush.
    pub raider_ambush_launched: bool,
    /// Persistent Rhai memory bytes for the Republic Customs Corvette (`KIRKWOOD_PICKET_RHAI`).
    pub corvette_memory: Vec<u8>,
    /// Think sequence counter for the Republic Customs Corvette.
    pub corvette_think_seq: u64,
    /// Last trace lines emitted by the Republic Customs Corvette's `RhaiRunner`.
    pub corvette_last_trace: Vec<String>,
    /// Last station intents emitted by the Republic Customs Corvette's `RhaiRunner`.
    pub corvette_last_intents: Vec<IntentKind>,
    /// Persistent Rhai memory bytes for the Belt Raider (`BELT_RAIDER_RHAI`).
    pub raider_memory: Vec<u8>,
    /// Think sequence counter for the Belt Raider.
    pub raider_think_seq: u64,
    /// Last trace lines emitted by the Belt Raider's `RhaiRunner`.
    pub raider_last_trace: Vec<String>,
    /// Last station intents emitted by the Belt Raider's `RhaiRunner`.
    pub raider_last_intents: Vec<IntentKind>,
}

impl KirkwoodPicketState {
    /// Create a fresh [`KirkwoodPicketState`] for a promoted Kirkwood interdiction bubble.
    #[must_use]
    pub fn new(
        corvette_hull: ObjId,
        corvette_ship_id: String,
        raider_hull: ObjId,
        raider_ship_id: String,
    ) -> Self {
        Self {
            corvette_hull,
            corvette_ship_id,
            raider_hull,
            raider_ship_id,
            hail_sent: false,
            hail_light_lag_ms: 0,
            inspection_status: PicketInspectionStatus::AwaitingHeaveTo,
            inspection_outcome: None,
            raider_ambush_launched: false,
            corvette_memory: Vec::new(),
            corvette_think_seq: 0,
            corvette_last_trace: Vec::new(),
            corvette_last_intents: Vec::new(),
            raider_memory: Vec::new(),
            raider_think_seq: 0,
            raider_last_trace: Vec::new(),
            raider_last_intents: Vec::new(),
        }
    }

    /// Return whether the customs picket inspection is satisfied (`Cleared` or `ClearedAfterFine`).
    #[must_use]
    pub const fn is_customs_cleared(&self) -> bool {
        matches!(
            self.inspection_status,
            PicketInspectionStatus::Cleared | PicketInspectionStatus::ClearedAfterFine
        )
    }
}

/// Compute 3D Euclidean distance in metres between two positions `a` and `b`.
#[must_use]
pub fn distance_3d_m(a: [i64; 3], b: [i64; 3]) -> i64 {
    let dx = i128::from(a[0]) - i128::from(b[0]);
    let dy = i128::from(a[1]) - i128::from(b[1]);
    let dz = i128::from(a[2]) - i128::from(b[2]);
    isqrt_i128(dx * dx + dy * dy + dz * dz).min(i128::from(i64::MAX)) as i64
}

/// Compute 3D Euclidean relative speed in $\text{mm/s}$ between two velocity vectors `va` and `vb`.
#[must_use]
pub fn relative_speed_3d_mmps(va: [i64; 3], vb: [i64; 3]) -> i64 {
    let dvx = i128::from(va[0]) - i128::from(vb[0]);
    let dvy = i128::from(va[1]) - i128::from(vb[1]);
    let dvz = i128::from(va[2]) - i128::from(vb[2]);
    isqrt_i128(dvx * dvx + dvy * dvy + dvz * dvz).min(i128::from(i64::MAX)) as i64
}

/// Check whether a promoted carrier on `domain` should open a multi-party **Kirkwood Customs Picket**
/// bubble (`CARRIER_PARTY`, `CUSTOMS_CORVETTE_PARTY`, `BELT_RAIDER_PARTY`, `D-35-11`, `doc 35` §9 Story 1).
///
/// Returns `true` when:
/// - `domain` is explicitly registered in `SpaceBubbleBook::kirkwood_picket_domains`, OR
/// - `hull` is a `Dray-P` prospector (`tacsim.unit.dray_p` or `HullView::is_prospector`).
#[must_use]
pub fn should_open_kirkwood_picket(w: &World, hull: ObjId, domain: mush_sim::DomainId) -> bool {
    if w.module_state::<super::tick::SpaceBubbleBook>()
        .is_some_and(|book| book.kirkwood_picket_domains.contains(&domain))
    {
        return true;
    }
    if let Some(unit) = mush_tactics::module::unit_of(w, hull)
        && unit.schema().key().id.as_str() == "tacsim.unit.dray_p"
    {
        return true;
    }
    if crate::port::ship::hull_view(w, hull).is_some_and(|view| view.is_prospector()) {
        return true;
    }
    false
}

/// Inspect the carrier hull's live `stow.ore` vault and the pilot's Freeport Registry
/// prospecting licence (`licence_gate`) and `tacsim.claims` tenure (`D-35-11`, `35a` §1).
#[must_use]
pub fn inspect_hull_manifest(
    ctx: &Ctx<'_>,
    hull: ObjId,
    pilot: Option<ObjId>,
    now_ic: i64,
) -> LiveCustomsManifest {
    let mut manifest = LiveCustomsManifest::default();
    if let Some(view) = crate::port::ship::hull_view(ctx.w, hull)
        && let Some(hold_vault) = crate::port::ship::hold_vault(ctx.w, &view)
    {
        let ep = mush_inventory::record::EndpointRef(hold_vault);
        let read_kg = |good_str: &str| -> u64 {
            CanonicalId::new(good_str)
                .ok()
                .and_then(|good| crate::port_tick::read_pool(ctx, ep, &good).ok())
                .unwrap_or(0)
                .saturating_mul(1_000)
        };
        manifest.strategic_ore_kg = read_kg("strategic_ore");
        manifest.metal_ore_kg = read_kg("metal_ore");
        manifest.ice_ore_kg = read_kg("ice_ore");
        manifest.carbon_ore_kg = read_kg("carbon_ore");
    }
    if let Some(actor) = pilot {
        manifest.licensed = licence_gate(ctx.w, actor, now_ic).is_ok();
        let account_hash = crate::port::licence::gateway_account(ctx.w, actor)
            .map(mush_core::AccountId::canonical_hash);
        if let Some(handles) = ctx.w.module_state::<crate::port::families::PortHandles>() {
            'search: for cluster in super::mining::OrbitalClusterId::ALL {
                let Ok(site) = CanonicalId::new(cluster.as_str()) else {
                    continue;
                };
                for letter in ['A', 'B', 'C'] {
                    let key = crate::surface::claims::BlockKey {
                        site: site.clone(),
                        letter,
                    };
                    if let Ok(Some(tenure)) = ctx.action_state(&handles.families.claims, &key)
                        && let Some(claim) = &tenure.claim
                    {
                        let holds = claim.holder == PrincipalId::Object(actor)
                            || match (claim.account, account_hash) {
                                (Some(acc), Some(acc_hash)) => acc.canonical_hash() == acc_hash,
                                _ => false,
                            };
                        if holds {
                            manifest.has_registered_claim = true;
                            break 'search;
                        }
                    }
                }
            }
        }
    }
    if manifest.strategic_ore_kg > 0 && (!manifest.licensed || !manifest.has_registered_claim) {
        manifest.unlicensed_strategic_ore_kg = manifest.strategic_ore_kg;
    }
    manifest
}

/// Emit a light-lagged radio hail (`mush_propagation::light_delay_secs`) from the
/// Republic Customs Corvette to the carrier hull and publish a structured `"tacsim:notice"`
/// event to every bridge crew member aboard (`D-35-10`, `doc 35` §9 Story 1 step 2, `35a` §4).
pub fn emit_light_lagged_picket_hail(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    distance_m: i64,
) -> (u64, u64) {
    let dist_u64 = u64::try_from(distance_m.max(0)).unwrap_or(0);
    let lag_ms_u64 = super::commands::vacuum_light_lag_ms(dist_u64);
    let lag_s = lag_ms_u64 / 1_000;
    let text = format!(
        "[RADIO +{}.{:03}s light-lag ({} m)] Republic Customs Corvette: \"Heave-to under 10 m/s (|dv| < 10.0 m/s) and run +helm/submit for Freeport Registry manifest & claim inspection.\"",
        lag_ms_u64 / 1_000,
        lag_ms_u64 % 1_000,
        dist_u64,
    );
    crate::port::voyage::narrate_hull(ctx, hull, &text);

    if let Some(view) = crate::port::ship::hull_view(ctx.w, hull)
        && let Some(airlock) = view.airlock()
    {
        let crew: Vec<ObjId> = ctx
            .w
            .contents_of(airlock)
            .iter()
            .copied()
            .filter(|&id| {
                ctx.w
                    .get(id)
                    .is_some_and(|o| o.kind == mush_core::ObjKind::Player)
            })
            .collect();
        let payload = json!({
            "kind": "picket_hail",
            "sender": "Republic Customs Corvette",
            "distance_m": dist_u64,
            "light_lag_s": lag_s,
            "light_lag_ms": lag_ms_u64,
            "max_heave_to_mps": 10,
            "text": text,
        });
        for member in crew {
            ctx.out.push(
                mush_core::Audience::Actor(member),
                mush_proto::OutputEvent::Panel {
                    feed: "tacsim:notice".into(),
                    kind: mush_proto::PanelFrameKind::Delta,
                    data: payload.clone(),
                },
            );
        }
    }
    (lag_s, lag_ms_u64)
}

fn minimal_space_arena_definition() -> Option<ArenaDefinition> {
    let terrain = ArenaTerrain::uniform(
        16,
        16,
        1_000,
        TerrainCell {
            move_cost: 1_000,
            ..TerrainCell::default()
        },
    )
    .ok()?;
    Some(ArenaDefinition {
        id: SmolStr::new_static("tacsim.arena.space_vacuum"),
        terrain,
        entry_zones: Vec::new(),
        key_spots: Vec::new(),
        waypoints: Waypoints::default(),
        modes: Vec::new(),
        homes: Vec::new(),
    })
}

static CACHED_RHAI_RUNNERS: std::sync::LazyLock<
    std::sync::Mutex<BTreeMap<[u8; 32], Arc<RhaiRunner>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));

static CACHED_SPACE_ARENA_DEF: std::sync::LazyLock<Option<Arc<ArenaDefinition>>> =
    std::sync::LazyLock::new(|| minimal_space_arena_definition().map(Arc::new));

/// Compile or retrieve a cached [`RhaiRunner`] for `source` (`SF.R10`, `35b` §5 #9).
pub fn cached_space_rhai_runner(source: &str) -> Option<Arc<RhaiRunner>> {
    use sha2::{Digest as _, Sha256};

    let bytes: [u8; 32] = Sha256::digest(source.as_bytes()).into();
    if let Ok(guard) = CACHED_RHAI_RUNNERS.lock()
        && let Some(found) = guard.get(&bytes)
    {
        return Some(found.clone());
    }
    let cfg = ArenaConfig::default();
    let runner = Arc::new(RhaiRunner::new(&cfg, source, mush_mind::Digest(bytes)).ok()?);
    if let Ok(mut guard) = CACHED_RHAI_RUNNERS.lock() {
        guard.insert(bytes, runner.clone());
    }
    Some(runner)
}

/// Compile and run one Rhai [`MindRunner::think`] step for `ship_id` inside `bubble` (`D-35-1`, `D-35-11`).
///
/// Builds a propagation-gated [`MindView`] (`mush_propagation::resolve_detection`) so the AI mind
/// sees only contacts that pass inverse-square IR / EMCON detection, executes `source` on
/// [`RhaiRunner`], and returns the [`ThinkResult`] alongside a `(ContactId -> ship_id)` lookup table.
pub fn run_space_rhai_think(
    bubble: &LocalSpaceBubble,
    ship_id: &str,
    unit_obj: ObjId,
    source: &str,
    think_seq: u64,
    memory: &[u8],
) -> Option<(ThinkResult, BTreeMap<ContactId, String>)> {
    let observer = bubble.ships.get(ship_id)?;
    let runner = cached_space_rhai_runner(source)?;

    let mut contact_map = BTreeMap::new();
    let mut contacts = Vec::new();
    for (idx, other) in bubble.ships.values().enumerate() {
        if other.id == ship_id {
            continue;
        }
        let obs = super::commands::evaluate_space_contact_detection(
            observer,
            unit_obj,
            other,
            ObjId(u32::try_from(idx).unwrap_or(0).saturating_add(2)),
            None,
            bubble.ic_timestamp_s,
        );
        if !obs.outcome.is_detected() {
            continue;
        }
        let cid = ContactId((idx as u64).saturating_add(1));
        contact_map.insert(cid, other.id.clone());

        let rel = bubble
            .party_relation(observer.party, other.party)
            .to_arena_relation();
        let rel_speed_mmps = relative_speed_3d_mmps(observer.vel_mmps, other.vel_mmps);
        let moving = rel_speed_mmps >= HEAVE_TO_REL_VEL_MMPS;
        let reads_dead = other.status != HullStatus::Operational;
        let party_id = other.party.to_party_id();
        let letter = char::from(b'A'.saturating_add(u8::try_from(idx.min(25)).unwrap_or(0)));
        let desig = Designator::Unit {
            party: party_id,
            letter,
        };
        let range_m = u64::try_from(obs.dist_m.max(0)).unwrap_or(0);
        let dx = (other.pos_m[0] - observer.pos_m[0]) as f64;
        let dy = (other.pos_m[1] - observer.pos_m[1]) as f64;
        let bearing_deg = (libm::atan2(dy, dx).to_degrees().round() as i32).rem_euclid(360);
        let band = if range_m <= 15_000 {
            SmolStr::new_static("near")
        } else if range_m <= 60_000 {
            SmolStr::new_static("medium")
        } else {
            SmolStr::new_static("far")
        };
        contacts.push(ViewContact {
            id: cid,
            designator: desig,
            alias: None,
            class: Classification::Classified { party: party_id },
            friendly: rel.is_friendly(),
            relation: rel,
            moving,
            bearing_deg: bearing_deg as u32,
            relative_deg: (bearing_deg - observer.attitude_deg.0).rem_euclid(360) as u32,
            range_m,
            band,
            age_ms: 0,
            cell: Some((8, 8)),
            condition: None::<FrameCondition>,
            fatal: Some(Vec::new()),
            aspect: Aspect::Front,
            odds: Vec::new(),
            reads_dead: Some(reads_dead),
        });
    }
    contacts.sort_by_key(|c| (c.range_m, c.id));

    let mut weapons = Vec::new();
    if let Some(laser) = observer.lasers.first() {
        let id = SmolStr::new(&laser.id);
        weapons.push(FrameWeapon {
            id,
            ready: laser.cooldown_remaining_ms == 0,
            ready_in_ms: i64::from(laser.cooldown_remaining_ms),
            rounds: 100,
            rounds_max: 100,
            range_band: None,
            p_permille: Some(850),
            bands_cm: vec![25_000_000],
            heat_per_shot: 0,
            armed: None,
        });
    }
    if let Some(tube) = observer.torpedo_tubes.first() {
        let id = SmolStr::new(&tube.id);
        weapons.push(FrameWeapon {
            id,
            ready: tube.cooldown_remaining_ms == 0 && tube.magazine_remaining > 0,
            ready_in_ms: i64::from(tube.cooldown_remaining_ms),
            rounds: u32::from(tube.magazine_remaining),
            rounds_max: u32::from(tube.magazine_remaining.max(8)),
            range_band: None,
            p_permille: Some(850),
            bands_cm: vec![25_000_000],
            heat_per_shot: 0,
            armed: None,
        });
    }
    if let Some(rg) = observer.railguns.first() {
        let id = SmolStr::new(&rg.id);
        weapons.push(FrameWeapon {
            id,
            ready: rg.cooldown_remaining_ms == 0 && rg.rounds_remaining > 0,
            ready_in_ms: i64::from(rg.cooldown_remaining_ms),
            rounds: u32::from(rg.rounds_remaining),
            rounds_max: u32::from(rg.rounds_remaining.max(24)),
            range_band: None,
            p_permille: Some(850),
            bands_cm: vec![25_000_000],
            heat_per_shot: 0,
            armed: None,
        });
    }

    let t_ms = i64::try_from(bubble.wall_elapsed_ms).unwrap_or(i64::MAX);
    let view = Arc::new(MindView {
        unit: unit_obj.0,
        callsign: Designator::Unit {
            party: observer.party.to_party_id(),
            letter: 'A',
        },
        party: observer.party.to_party_id(),
        cell: Some((8, 8)),
        mode: MatchMode::Skirmish,
        own_last_hit_ms: None,
        objective: None,
        motor: None,
        x_cm: observer.pos_m[0].saturating_mul(100),
        y_cm: observer.pos_m[1].saturating_mul(100),
        z_cm: observer.pos_m[2].saturating_mul(100),
        heading_mdeg: (observer.attitude_deg.0.rem_euclid(360) as u32) * 1_000,
        speed_cms: (isqrt_i128(
            i128::from(observer.vel_mmps[0]).pow(2)
                + i128::from(observer.vel_mmps[1]).pow(2)
                + i128::from(observer.vel_mmps[2]).pow(2),
        ) / 10) as i32,
        power: Power::Online,
        held: false,
        wreck: observer.status != HullStatus::Operational,
        immobile: observer.status != HullStatus::Operational,
        sight_m: 250_000,
        ecm: Some(observer.active_decoy_ms > 0),
        receiver: Some(FrameReceiver {
            budget: 100,
            budget_max: 100,
            reach_permille: 1_000,
        }),
        heat: Some(FrameHeat {
            heat: u32::from(observer.heat_permille),
            max: 1_000,
        }),
        sections: Vec::new(),
        lock: contacts.first().map(|c| c.id),
        weapons,
        rules: Vec::new(),
        order: None,
        t_ms,
        notices: Vec::<FrameNotice>::new(),
        contacts,
        scan_tables: Vec::new(),
        odds_floor_permille: 0,
    });

    let card_def = stock_definition(
        &[(SmolStr::new_static("Line"), line(&ReflexConfig::default()))],
        CoreClass::I,
    )
    .ok()?;
    let mut catalog = CardCatalog::default();
    let def_digest = catalog.admit(card_def).ok()?;
    let card = catalog
        .resolve(ProfileKey {
            definition: def_digest,
            index: 0,
        })
        .ok()?;
    let core_profile = ClassTable::STOCK.row(CoreClass::I);
    let program = BoundProgram::bind(&[], &[], &[], &core_profile, 0).ok()?;
    let profile_token = ProfileToken {
        unit: unit_obj,
        generation: 0,
        key: card.key(),
    };

    let map: Arc<ArenaDefinition> = CACHED_SPACE_ARENA_DEF.as_ref()?.clone();
    let job = ThinkJob {
        unit: unit_obj,
        think_seq,
        t_ms,
        lands_ms: t_ms.saturating_add(500),
        seed: u64::from(unit_obj.0).wrapping_add(think_seq),
        dropped: false,
        profile: &core_profile,
        view: &view,
        memory,
        map: &map,
        card: &card,
        program: &program,
        profile_token,
    };

    let result = runner.think(&job);
    Some((result, contact_map))
}

/// Step the Kirkwood Customs Picket `RhaiRunner` doctrines (`KIRKWOOD_PICKET_RHAI` and
/// `BELT_RAIDER_RHAI`), heave-to velocity check, and perimeter check on `bubble` (`D-35-11`).
#[allow(clippy::too_many_lines)]
pub fn step_kirkwood_picket(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    picket: &mut KirkwoodPicketState,
    carrier_ship_id: &str,
) -> Vec<super::bubble::SpaceCombatEvent> {
    let mut emitted_events = Vec::new();

    // 1. Check carrier relative velocity & distance against the Republic Customs Corvette.
    if let (Some(carrier), Some(corvette)) = (
        bubble.ships.get(carrier_ship_id),
        bubble.ships.get(&picket.corvette_ship_id),
    ) && carrier.status == HullStatus::Operational
        && corvette.status == HullStatus::Operational
    {
        let dist_m = distance_3d_m(carrier.pos_m, corvette.pos_m);
        let rel_speed_mmps = relative_speed_3d_mmps(carrier.vel_mmps, corvette.vel_mmps);
        if dist_m > PICKET_RUN_BOUNDARY_M && !picket.is_customs_cleared() {
            picket.inspection_status = PicketInspectionStatus::RanThePicket;
            bubble.set_party_relation(
                CUSTOMS_CORVETTE_PARTY,
                CARRIER_PARTY,
                SpaceRelation::Hostile,
            );
        } else if picket.inspection_status == PicketInspectionStatus::AwaitingHeaveTo
            && rel_speed_mmps < HEAVE_TO_REL_VEL_MMPS
        {
            picket.inspection_status = PicketInspectionStatus::HoveToReadyForInspection;
        }
    }

    // 2. If the Belt Raider is ready to launch its ambush (once customs inspection is resolved,
    // if the carrier runs the picket, or after 30.0 wall seconds in the bubble), light its torch
    // (`EmconPosture::ActiveCombat`) so sensors and `BELT_RAIDER_RHAI` engage (`35a` §1 row `D-35-11`:
    // never uncloak before `+helm/submit` has had a chance to inspect while hove-to!).
    let should_uncloak_raider = !picket.raider_ambush_launched
        && (picket.inspection_outcome.is_some()
            || picket.inspection_status == PicketInspectionStatus::RanThePicket);
    if should_uncloak_raider
        && let Some(raider) = bubble.ships.get(&picket.raider_ship_id)
        && raider.status == HullStatus::Operational
        && raider.emcon == EmconPosture::Ghost
    {
        emitted_events.extend(tape.record_and_apply(
            bubble,
            BubbleInputCommand::SetEmcon {
                ship_id: picket.raider_ship_id.clone(),
                posture: EmconPosture::ActiveCombat,
            },
        ));
    }

    // 3. Run the Belt Raider's `RhaiRunner` (`BELT_RAIDER_RHAI`) if operational and active.
    if let Some(raider) = bubble.ships.get(&picket.raider_ship_id)
        && raider.status == HullStatus::Operational
        && raider.emcon != EmconPosture::Ghost
        && let Some((res, cmap)) = run_space_rhai_think(
            bubble,
            &picket.raider_ship_id,
            picket.raider_hull,
            BELT_RAIDER_RHAI,
            picket.raider_think_seq,
            &picket.raider_memory,
        )
        && res.fault.is_none()
    {
        picket.raider_think_seq = picket.raider_think_seq.saturating_add(1);
        picket.raider_memory = res.memory;
        picket.raider_last_trace = res.trace;
        picket.raider_last_intents = res.intents.clone();

        let mut locked_ship = bubble
            .ships
            .get(&picket.raider_ship_id)
            .and_then(|s| s.locked_target_id.clone())
            .unwrap_or_else(|| carrier_ship_id.to_owned());

        for intent in &res.intents {
            match intent {
                IntentKind::Emcon { on } => {
                    let posture = if *on {
                        EmconPosture::ActiveCombat
                    } else {
                        EmconPosture::Ghost
                    };
                    if bubble
                        .ships
                        .get(&picket.raider_ship_id)
                        .is_some_and(|s| s.emcon != posture)
                    {
                        emitted_events.extend(tape.record_and_apply(
                            bubble,
                            BubbleInputCommand::SetEmcon {
                                ship_id: picket.raider_ship_id.clone(),
                                posture,
                            },
                        ));
                    }
                }
                IntentKind::Lock { contact } => {
                    if let Some(target_id) = cmap.get(contact) {
                        locked_ship = target_id.clone();
                        if bubble
                            .ships
                            .get(&picket.raider_ship_id)
                            .is_some_and(|s| s.locked_target_id.as_deref() != Some(target_id))
                        {
                            emitted_events.extend(tape.record_and_apply(
                                bubble,
                                BubbleInputCommand::SetLockedTarget {
                                    ship_id: picket.raider_ship_id.clone(),
                                    target_id: Some(target_id.clone()),
                                },
                            ));
                        }
                    }
                }
                IntentKind::Speed { cms } => {
                    if *cms > 0
                        && bubble
                            .ships
                            .get(&picket.raider_ship_id)
                            .is_some_and(|s| s.thrust_mps2 != [-15, 0, 0])
                    {
                        emitted_events.extend(tape.record_and_apply(
                            bubble,
                            BubbleInputCommand::CommandBurn {
                                ship_id: picket.raider_ship_id.clone(),
                                thrust_mps2: [-15, 0, 0],
                                force_into_reserve: true,
                            },
                        ));
                    }
                }
                IntentKind::Fire { weapon, .. } => {
                    if weapon.as_str().starts_with("torp") {
                        picket.raider_ambush_launched = true;
                        emitted_events.extend(tape.record_and_apply(
                            bubble,
                            BubbleInputCommand::FireTorpedoSalvo {
                                shooter_id: picket.raider_ship_id.clone(),
                                target_id: locked_ship.clone(),
                                count: 2,
                            },
                        ));
                    } else if weapon.as_str().starts_with("rail") {
                        picket.raider_ambush_launched = true;
                        emitted_events.extend(tape.record_and_apply(
                            bubble,
                            BubbleInputCommand::FireRailgun {
                                shooter_id: picket.raider_ship_id.clone(),
                                target_id: locked_ship.clone(),
                            },
                        ));
                    } else if weapon.as_str().starts_with("laser") {
                        emitted_events.extend(tape.record_and_apply(
                            bubble,
                            BubbleInputCommand::FireOffensiveLasers {
                                shooter_id: picket.raider_ship_id.clone(),
                                target_id: locked_ship.clone(),
                            },
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    // 4. Run the Republic Customs Corvette's `RhaiRunner` (`KIRKWOOD_PICKET_RHAI`).
    if let Some(corvette) = bubble.ships.get(&picket.corvette_ship_id)
        && corvette.status == HullStatus::Operational
        && let Some((res, cmap)) = run_space_rhai_think(
            bubble,
            &picket.corvette_ship_id,
            picket.corvette_hull,
            KIRKWOOD_PICKET_RHAI,
            picket.corvette_think_seq,
            &picket.corvette_memory,
        )
        && res.fault.is_none()
    {
        picket.corvette_think_seq = picket.corvette_think_seq.saturating_add(1);
        picket.corvette_memory = res.memory;
        picket.corvette_last_trace = res.trace;
        picket.corvette_last_intents = res.intents.clone();

        let mut locked_ship = bubble
            .ships
            .get(&picket.corvette_ship_id)
            .and_then(|s| s.locked_target_id.clone())
            .unwrap_or_else(|| picket.raider_ship_id.clone());

        if bubble
            .ships
            .get(&picket.corvette_ship_id)
            .is_some_and(|s| s.heat_permille > 600)
        {
            emitted_events.extend(tape.record_and_apply(
                bubble,
                BubbleInputCommand::BlowdownCoolant {
                    ship_id: picket.corvette_ship_id.clone(),
                },
            ));
        }

        for intent in &res.intents {
            match intent {
                IntentKind::Emcon { on } => {
                    let posture = if *on {
                        EmconPosture::ActiveCombat
                    } else {
                        EmconPosture::Standard
                    };
                    if bubble
                        .ships
                        .get(&picket.corvette_ship_id)
                        .is_some_and(|s| s.emcon != posture)
                    {
                        emitted_events.extend(tape.record_and_apply(
                            bubble,
                            BubbleInputCommand::SetEmcon {
                                ship_id: picket.corvette_ship_id.clone(),
                                posture,
                            },
                        ));
                    }
                }
                IntentKind::Lock { contact } => {
                    if let Some(target_id) = cmap.get(contact) {
                        locked_ship = target_id.clone();
                        if bubble
                            .ships
                            .get(&picket.corvette_ship_id)
                            .is_some_and(|s| s.locked_target_id.as_deref() != Some(target_id))
                        {
                            emitted_events.extend(tape.record_and_apply(
                                bubble,
                                BubbleInputCommand::SetLockedTarget {
                                    ship_id: picket.corvette_ship_id.clone(),
                                    target_id: Some(target_id.clone()),
                                },
                            ));
                        }
                    }
                }
                IntentKind::Goal(MotorGoal::Hold) => {
                    if bubble
                        .ships
                        .get(&picket.corvette_ship_id)
                        .is_some_and(|s| s.thrust_mps2 != [0, 0, 0])
                    {
                        emitted_events.extend(tape.record_and_apply(
                            bubble,
                            BubbleInputCommand::CommandBurn {
                                ship_id: picket.corvette_ship_id.clone(),
                                thrust_mps2: [0, 0, 0],
                                force_into_reserve: false,
                            },
                        ));
                    }
                }
                IntentKind::Fire { weapon, .. } if weapon.as_str().starts_with("laser") => {
                    emitted_events.extend(tape.record_and_apply(
                        bubble,
                        BubbleInputCommand::FireOffensiveLasers {
                            shooter_id: picket.corvette_ship_id.clone(),
                            target_id: locked_ship.clone(),
                        },
                    ));
                }
                IntentKind::Fire { weapon, .. } if weapon.as_str().starts_with("rail") => {
                    emitted_events.extend(tape.record_and_apply(
                        bubble,
                        BubbleInputCommand::FireRailgun {
                            shooter_id: picket.corvette_ship_id.clone(),
                            target_id: locked_ship.clone(),
                        },
                    ));
                }
                _ => {}
            }
        }
    }

    emitted_events
}

/// Build the Public E4 contract specification for a Republic Customs contraband fine (`D-35-11`).
#[must_use]
pub fn customs_fine_spec(
    pilot_account: AccountId,
    registry: &PrincipalId,
    registry_account: AccountId,
    pilot: ObjId,
    fine: Amount,
    now: i64,
) -> ContractSpec {
    let deadline = EconomicDeadline {
        clock: EconomicClock::RealUtc,
        at: now.saturating_add(LICENCE_SETTLE_WINDOW_SECS),
    };
    let me = PrincipalId::Object(pilot);
    ContractSpec {
        name: SmolStr::new_static(CUSTOMS_FINE_CONTRACT_NAME),
        scope: None,
        parties: vec![registry.clone()],
        considerations: vec![ConsiderationSpec {
            account: pilot_account,
            amount: fine,
        }],
        outcomes: vec![OutcomeTerms {
            name: SmolStr::new_static("fined"),
            when: vec![0],
            payouts: vec![PayoutShare {
                to: registry_account,
                share: Share::Exact(fine),
            }],
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences: Vec::new(),
        }],
        remainder_to: pilot_account,
        conditions: vec![TypedCondition::Before { deadline }],
        adjudicator: None,
        evidence: vec![EvidenceKind::Before],
        deadline: Some(deadline),
        on_expiry: Expiry::Forfeit {
            to: registry_account,
        },
        visibility: EconomicVisibility::Public,
        roles: ContractRoles {
            author: Some(me.clone()),
            payer: Some(me),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables: Vec::new(),
    }
}

/// Resolve the Freeport Registry organization id, principal, and live account on `w`.
#[must_use]
pub fn resolve_registry_principal_and_account(
    w: &World,
) -> Option<(OrgId, PrincipalId, AccountId)> {
    let (_, map, licence) = licence_port(w)?;
    let row = map.orgs.get(&licence.org)?;
    let org_id = OrgId(row.org);
    let org_live = w
        .module_state::<mush_org::OrgStore>()
        .is_some_and(|orgs| orgs.state().org(org_id).is_some());
    if !org_live {
        return None;
    }
    let economy = w.module_state::<EconomyStore>()?;
    let account_id = row.account.map(AccountId).filter(|id| {
        economy
            .snapshot()
            .account(*id)
            .is_some_and(|a| a.status == AccountStatus::Active)
    })?;
    Some((org_id, PrincipalId::Organization(org_id), account_id))
}

/// Assess a real `mush-economy` Public E4 contract fine from `pilot`'s credit account into
/// `freeport_registry`'s account and advance the [`mush_faction::CUSTOMS_ASSERTION_TITLE`]
/// campaign front (`D-35-11`, `SF.R6`).
///
/// # Errors
///
/// Returns an error string if the pilot lacks a credit account or sufficient funds, or if
/// contract activation fails.
pub fn assess_customs_fine_and_advance_front(
    ctx: &mut Ctx<'_>,
    pilot: ObjId,
    hull: ObjId,
    unlicensed_strategic_ore_kg: u64,
) -> Result<(ContractId, i64, Option<FrontId>), String> {
    let tonnes = unlicensed_strategic_ore_kg.div_ceil(1_000).max(1);
    let fine_major_u64 = tonnes
        .saturating_mul(u64::try_from(CUSTOMS_FINE_MAJOR_PER_TONNE).unwrap_or(500))
        .max(u64::try_from(CUSTOMS_FINE_MIN_MAJOR).unwrap_or(1_000));
    let fine_minor = minor_of(fine_major_u64, credit_scale(ctx.w))
        .ok_or_else(|| "customs fine overflows credit scale".to_owned())?;

    let credit = crate::port::credit::own_account(ctx.w, pilot)
        .ok_or_else(|| "pilot holds no credit account for customs fine".to_owned())?;
    let pilot_account = credit.id;
    let available = credit.available().map_or(0, Amount::minor);
    if available < fine_minor {
        return Err(format!(
            "insufficient credit for customs fine (requires {} minor units, available {})",
            fine_minor, available
        ));
    }

    let (registry_org, registry_principal, registry_account) =
        resolve_registry_principal_and_account(ctx.w)
            .ok_or_else(|| "Freeport Registry account unavailable".to_owned())?;

    let saved_actor = ctx.actor;
    ctx.actor = pilot;
    let spec = customs_fine_spec(
        pilot_account,
        &registry_principal,
        registry_account,
        pilot,
        Amount::from_minor(fine_minor),
        ctx.w.now(),
    );
    let contract_id = match mush_economy::offer_contract(ctx, spec) {
        Ok(id) => id,
        Err(e) => {
            ctx.actor = saved_actor;
            return Err(format!("customs fine contract offer failed: {e}"));
        }
    };
    if !matches!(
        mush_economy::accept_contract(ctx, contract_id),
        Ok(AcceptOutcome::Accepted { .. })
    ) {
        let _ = mush_economy::cancel_contract(ctx, contract_id);
        ctx.actor = saved_actor;
        ctx.out.pin_deferred();
        return Err("customs fine escrow acceptance failed".to_owned());
    }
    if !matches!(
        mush_economy::accept_contract_as(ctx, contract_id, &registry_principal),
        Ok(AcceptOutcome::Accepted {
            activated: true,
            ..
        })
    ) {
        let _ = mush_economy::cancel_contract(ctx, contract_id);
        ctx.actor = saved_actor;
        ctx.out.pin_deferred();
        return Err("Registry customs fine acceptance failed".to_owned());
    }

    let front_id = mush_faction::record_customs_assertion(
        ctx,
        registry_org,
        mush_canon::CanonDraft {
            kind: "space.customs_fine".into(),
            summary: format!(
                "Republic Customs Corvette fined hull #{} ({} kg unlicensed strategic_ore) at the Kirkwood picket; {} advances.",
                hull.0,
                unlicensed_strategic_ore_kg,
                mush_faction::CUSTOMS_ASSERTION_TITLE,
            ),
            entities: vec![pilot, hull],
            scope: mush_canon::CanonScope::Public,
            ic_time: ctx.w.world_ic_now().at,
            source: format!("space.picket.fine/{}/{}", hull.0, contract_id.0),
            detail: json!({
                "hull": hull.0,
                "pilot": pilot.0,
                "contract": contract_id.0,
                "fine_minor": fine_minor,
                "unlicensed_strategic_ore_kg": unlicensed_strategic_ore_kg,
            }),
        },
    )
    .ok();

    ctx.actor = saved_actor;
    ctx.out.pin_deferred();
    Ok((contract_id, fine_minor, front_id))
}

/// Record a picket-running incident (`RanThePicket`), escalating the Republic Customs Corvette
/// to [`SpaceRelation::Hostile`] and advancing the [`mush_faction::CUSTOMS_ASSERTION_TITLE`]
/// campaign front (`D-35-11`, `SF.R6`).
pub fn record_ran_the_picket_incident(
    ctx: &mut Ctx<'_>,
    pilot: ObjId,
    hull: ObjId,
) -> Option<FrontId> {
    let (registry_org, _, _) = resolve_registry_principal_and_account(ctx.w)?;
    let saved_actor = ctx.actor;
    ctx.actor = pilot;
    let front_id = mush_faction::record_customs_assertion(
        ctx,
        registry_org,
        mush_canon::CanonDraft {
            kind: "space.ran_the_picket".into(),
            summary: format!(
                "Hull #{} ran the Kirkwood Customs Picket without inspection clearance; {} advances.",
                hull.0,
                mush_faction::CUSTOMS_ASSERTION_TITLE,
            ),
            entities: vec![pilot, hull],
            scope: mush_canon::CanonScope::Public,
            ic_time: ctx.w.world_ic_now().at,
            source: format!("space.picket.run/{}/{}", hull.0, ctx.w.world_ic_now().at),
            detail: json!({
                "hull": hull.0,
                "pilot": pilot.0,
            }),
        },
    )
    .ok();
    ctx.actor = saved_actor;
    front_id
}

/// Populate the Republic Customs Corvette and Belt Raider ships for a Kirkwood Customs Picket
/// bubble and insert them alongside their 3-party relations into `bubble` (`D-35-11`, `doc 35` §9 Story 1).
#[must_use]
pub fn build_kirkwood_picket_ships(
    bubble: &mut LocalSpaceBubble,
    domain: mush_sim::DomainId,
    leg_cursor: u32,
    _carrier_ship_id: &str,
) -> Option<KirkwoodPicketState> {
    let corvette_ship_id = format!("customs.{}.{}", domain.0, leg_cursor);
    let raider_ship_id = format!("raider.{}.{}", domain.0, leg_cursor);
    let corvette_hull = ObjId(
        910_000_u32
            .saturating_add(u32::try_from(domain.0).unwrap_or(0))
            .max(1),
    );
    let raider_hull = ObjId(
        920_000_u32
            .saturating_add(u32::try_from(domain.0).unwrap_or(0))
            .max(1),
    );

    let mut corvette = SpaceShipState::new_corvette(
        &corvette_ship_id,
        "Republic Customs Corvette",
        CUSTOMS_CORVETTE_PARTY,
        [-CORVETTE_STANDOFF_M, 400, 0],
        [10_000, 0, 0],
    );
    corvette.attitude_deg = (180, 0, 0);
    corvette.pdc_auto = true;
    corvette.emcon = EmconPosture::ActiveCombat;
    if let Some(off_laser) = corvette.lasers.first_mut() {
        off_laser.max_range_m = 160_000;
        off_laser.base_damage = 180;
    }
    if let Some(def_laser) = corvette.lasers.get_mut(1) {
        def_laser.mode = LaserMode::Defense;
        def_laser.defense_range_m = 45_000;
    }

    let mut raider = SpaceShipState::new_corvette(
        &raider_ship_id,
        "Belt Raider",
        BELT_RAIDER_PARTY,
        [RAIDER_AMBUSH_RANGE_M, 0, 0],
        [-20_000, 0, 0],
    );
    raider.attitude_deg = (180, 0, 0);
    raider.emcon = EmconPosture::Ghost;
    raider.heat_permille = 0;
    if let Some(rg) = raider.railguns.first_mut() {
        rg.max_range_m = 250_000;
        rg.muzzle_vel_mps = 12_000;
    }
    if let Some(tube) = raider.torpedo_tubes.first_mut() {
        tube.tubes = 2;
        tube.max_dv_mps = 12_000;
        tube.accel_mps2 = 250;
    }

    bubble.set_party_relation(
        CARRIER_PARTY,
        CUSTOMS_CORVETTE_PARTY,
        SpaceRelation::Inspecting,
    );
    bubble.set_party_relation(CARRIER_PARTY, BELT_RAIDER_PARTY, SpaceRelation::Hostile);
    bubble.set_party_relation(
        CUSTOMS_CORVETTE_PARTY,
        BELT_RAIDER_PARTY,
        SpaceRelation::Hostile,
    );

    bubble.insert_ship(corvette);
    bubble.insert_ship(raider);

    Some(KirkwoodPicketState::new(
        corvette_hull,
        corvette_ship_id,
        raider_hull,
        raider_ship_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kirkwood_picket_and_belt_raider_rhai_doctrines_run_on_rhai_runner_without_faults() {
        let mut bubble = LocalSpaceBubble::new(
            "bubble.test.kirkwood",
            "domain.7",
            "Kirkwood Customs Picket Bubble",
            1_000_000,
        );
        let carrier = SpaceShipState::new_dray(
            "hull.10",
            "Salt Moth",
            CARRIER_PARTY,
            [0, 0, 0],
            [25_000, 0, 0],
        );
        bubble.insert_ship(carrier);
        let mut picket =
            build_kirkwood_picket_ships(&mut bubble, mush_sim::DomainId(7), 1, "hull.10")
                .expect("kirkwood picket ships should build");
        let mut tape = BubbleInputTape::new(&bubble);

        // Step 1: Carrier is closing at 15 m/s relative speed (`[25_000, 0, 0]` vs `[10_000, 0, 0]`).
        // The lurking Belt Raider is in `Ghost` at 120 km (undetected by inverse-square IR).
        // Republic Customs Corvette's `RhaiRunner` (`KIRKWOOD_PICKET_RHAI`) hails and awaits heave-to.
        let _ = tape.record_and_apply(
            &mut bubble,
            BubbleInputCommand::StepWall {
                dt_wall_ms: 1_000,
                ic_now_s: 1_000_024,
            },
        );
        let _ = step_kirkwood_picket(&mut bubble, &mut tape, &mut picket, "hull.10");
        assert_eq!(
            picket.inspection_status,
            PicketInspectionStatus::AwaitingHeaveTo
        );
        assert_eq!(picket.corvette_think_seq, 1);
        assert_eq!(
            picket.corvette_last_trace,
            vec!["PICKET_HAIL_HEAVE_TO".to_owned()]
        );
        assert!(!picket.raider_ambush_launched);

        // Step 2: Carrier decelerates to `[14_000, 0, 0]` (relative speed 4 m/s < 10 m/s heave-to gate).
        // Corvette's `RhaiRunner` transitions to `"inspecting"` (`PICKET_CARRIER_HOVE_TO_READY`),
        // and the Raider does NOT prematurely uncloak before `+helm/submit`!
        if let Some(c) = bubble.ships.get_mut("hull.10") {
            c.vel_mmps = [14_000, 0, 0];
        }
        let _ = step_kirkwood_picket(&mut bubble, &mut tape, &mut picket, "hull.10");
        assert_eq!(
            picket.inspection_status,
            PicketInspectionStatus::HoveToReadyForInspection
        );
        assert_eq!(picket.corvette_think_seq, 2);
        assert_eq!(
            picket.corvette_last_trace,
            vec!["PICKET_CARRIER_HOVE_TO_READY".to_owned()]
        );
        assert!(!picket.raider_ambush_launched);

        // Step 3: Inspection outcome is recorded (`ClearedAfterFine`), and on the next step
        // the Belt Raider lights its torch (`ActiveCombat`), `BELT_RAIDER_RHAI` fires its
        // 2-torpedo salvo + 120 km railgun shot (`RAIDER_TORCH_AND_SALVO`), and the Corvette's
        // `RhaiRunner` (`KIRKWOOD_PICKET_RHAI`) transitions to `"screen_defend"` (`PICKET_SCREEN_ENGAGE_RAIDER`).
        picket.inspection_status = PicketInspectionStatus::ClearedAfterFine;
        picket.inspection_outcome = Some(PicketInspectionOutcome::Fined {
            contract_id: 42,
            fine_minor: 200_000,
            unlicensed_strategic_ore_kg: 4_000,
            front_id: Some(1),
        });
        bubble.set_party_relation(
            CUSTOMS_CORVETTE_PARTY,
            CARRIER_PARTY,
            SpaceRelation::Friendly,
        );

        let ambush_events = step_kirkwood_picket(&mut bubble, &mut tape, &mut picket, "hull.10");
        assert!(picket.raider_ambush_launched);
        assert_eq!(picket.raider_think_seq, 1);
        assert_eq!(
            picket.raider_last_trace,
            vec!["RAIDER_TORCH_AND_SALVO".to_owned()]
        );
        assert_eq!(bubble.torpedoes.len(), 2);
        assert_eq!(bubble.railgun_slugs.len(), 1);
        assert!(ambush_events.iter().any(|e| matches!(
            e,
            super::super::bubble::SpaceCombatEvent::TorpedoLaunched { .. }
        )));
        assert!(ambush_events.iter().any(|e| matches!(
            e,
            super::super::bubble::SpaceCombatEvent::RailgunFired { .. }
        )));
        assert_eq!(
            picket.corvette_last_trace,
            vec!["PICKET_SCREEN_ENGAGE_RAIDER".to_owned()]
        );
    }

    #[test]
    fn burning_past_picket_boundary_without_clearance_constructs_ran_the_picket_and_turns_corvette_hostile()
     {
        let mut bubble = LocalSpaceBubble::new(
            "bubble.test.run_picket",
            "domain.8",
            "Kirkwood Customs Picket Bubble",
            1_000_000,
        );
        let carrier = SpaceShipState::new_dray(
            "hull.11",
            "Salt Moth",
            CARRIER_PARTY,
            [40_000, 0, 0],
            [40_000, 0, 0],
        );
        bubble.insert_ship(carrier);
        let mut picket =
            build_kirkwood_picket_ships(&mut bubble, mush_sim::DomainId(8), 1, "hull.11")
                .expect("kirkwood picket ships should build");
        let mut tape = BubbleInputTape::new(&bubble);

        let _ = step_kirkwood_picket(&mut bubble, &mut tape, &mut picket, "hull.11");
        assert_eq!(
            picket.inspection_status,
            PicketInspectionStatus::RanThePicket
        );
        assert_eq!(
            bubble.party_relation(CUSTOMS_CORVETTE_PARTY, CARRIER_PARTY),
            SpaceRelation::Hostile
        );
    }
}
