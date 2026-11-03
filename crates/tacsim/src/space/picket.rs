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
//!      toward the Belt Raider), flown by a compiled [`RhaiRunner`] executing `kirkwood_picket.rhai`.
//!    - [`BELT_RAIDER_PARTY`] (`PartyId::nth(2)` = `'C'`): **Belt Raider** (`SpaceRelation::Hostile`
//!      toward both the carrier and the Republic Customs Corvette), flown by a compiled
//!      [`RhaiRunner`] executing `belt_raider.rhai`.
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

/// Attribute name storing a space NPC hull's or collector drone's [`crate::arena::mind::MindSeal`] (`SF.R14`, `D-35-1`, `D-35-17`).
pub const SPACE_MIND_SEAL_ATTR: &str = "tacsim:mind_seal";

/// Construct a verified [`crate::arena::mind::MindSeal`] (`SealEngine::Scripted`) for a space NPC
/// hull or collector drone (`SF.R14`, `D-35-1`, `D-35-17`).
#[must_use]
pub fn build_space_mind_seal(
    unit: ObjId,
    sortie: crate::arena::mind::SortieId,
    mind_name: &str,
    source_sha256: [u8; 32],
) -> Option<crate::arena::mind::MindSeal> {
    use crate::arena::core::{ClassTable, CoreClass};
    use crate::arena::mind::{MindSeal, SealEngine};
    use crate::arena::profile_seal::{MindIdentity, MindOrigin, SealedCards};
    use crate::arena::runtime_card::{rom_definition, stock_definition};
    use crate::arena::state::Channel;
    use crate::config::ReflexConfig;

    let reflex = ReflexConfig::default();
    let line_card = crate::arena::card::line(&reflex);
    let channels = vec![Channel::Helm, Channel::Sensors];
    let weapons = Vec::new();
    let floor = 100;
    let classes = &ClassTable::STOCK;

    let identity = MindIdentity {
        mind: SmolStr::new(mind_name),
        origin: MindOrigin::Doctrine { revision: 1 },
        source_sha256,
    };
    let rom_identity = crate::match_::SealIdentity::doctrine();
    let rom = SealedCards::prepare(
        rom_definition(&reflex, floor).ok()?,
        &[],
        &classes.row(CoreClass::Zero),
        &weapons,
        &channels,
        floor,
    )
    .ok()?;
    let def = stock_definition(&[(SmolStr::new_static("Line"), line_card)], CoreClass::I).ok()?;
    let cards = SealedCards::prepare(
        def,
        &[SmolStr::new_static("Line")],
        &classes.row(CoreClass::I),
        &weapons,
        &channels,
        floor,
    )
    .ok()?;

    let seal = MindSeal {
        unit,
        sortie,
        core: Some(SmolStr::new_static("core.class_1")),
        core_class: CoreClass::I,
        class: CoreClass::I,
        governed: true,
        mind: identity.mind.clone(),
        origin: identity.origin.clone(),
        source_sha256,
        requested: identity,
        card: cards.opening_card.clone(),
        card_digest: cards.opening_digest,
        abi: crate::minds::MIND_ABI.into(),
        principal: None,
        channels,
        weapons: weapons.clone(),
        odds_floor_permille: floor,
        fallback: None,
        engine: SealEngine::Scripted,
        cards,
        rom_identity,
        rom,
    };
    seal.verify(classes, &weapons).ok()?;
    Some(seal)
}

/// Install space mind sources into [`crate::store::TacsimStore`] and attach a verified
/// [`crate::arena::mind::MindSeal`] onto `unit` (`SF.R14`, `D-35-1`, `D-35-17`).
pub fn seal_space_unit_in_world(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    sortie: crate::arena::mind::SortieId,
    mind_name: &str,
    source_sha256: [u8; 32],
) -> Option<crate::arena::mind::MindSeal> {
    if let Some(store) = ctx.w.module_state_mut::<crate::store::TacsimStore>() {
        crate::minds::content::ensure_space_mind_sources_installed(store);
    }
    let seal = build_space_mind_seal(unit, sortie, mind_name, source_sha256)?;
    if let Ok(val) = serde_json::to_value(&seal) {
        let _ = ctx.w.set_attr(
            unit,
            SPACE_MIND_SEAL_ATTR,
            mush_core::Attr::json(val, mush_core::GOD),
        );
    }
    Some(seal)
}

/// Read the [`crate::arena::mind::MindSeal`] attached to `unit` (`SF.R14`, `D-35-1`, `D-35-17`).
#[must_use]
pub fn space_mind_seal_of(w: &World, unit: ObjId) -> Option<crate::arena::mind::MindSeal> {
    let attr = w.get(unit)?.attrs.get(SPACE_MIND_SEAL_ATTR)?;
    let mush_core::AttrValue::Json(val) = &attr.value else {
        return None;
    };
    serde_json::from_value(val.clone()).ok()
}

/// Compact serde helper for [`Option<crate::arena::mind::MindSeal>`] on space state structs
/// (`SF.R14`, `35b` §5 #9): serializes only `(unit, sortie, mind, source_sha256)` (`~120 B`)
/// and rebuilds the verified [`crate::arena::mind::MindSeal`] on deserialize so `tacsim:space:state`
/// remains within its persistence budget while round-tripping losslessly.
pub mod compact_mind_seal {
    use mush_core::ObjId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use smol_str::SmolStr;

    use crate::arena::mind::{MindSeal, SortieId};

    #[derive(Serialize, Deserialize)]
    struct CompactMindSeal {
        unit: ObjId,
        sortie: SortieId,
        mind: SmolStr,
        source_sha256: [u8; 32],
    }

    /// Serialize `seal` in compact form.
    ///
    /// # Errors
    ///
    /// Propagates any serializer error.
    pub fn serialize<S>(seal: &Option<MindSeal>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let compact = seal.as_ref().map(|s| CompactMindSeal {
            unit: s.unit,
            sortie: s.sortie,
            mind: s.mind.clone(),
            source_sha256: s.source_sha256,
        });
        compact.serialize(serializer)
    }

    /// Deserialize `Option<MindSeal>` from compact form.
    ///
    /// # Errors
    ///
    /// Propagates any deserializer error.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<MindSeal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let compact = Option::<CompactMindSeal>::deserialize(deserializer)?;
        Ok(compact.and_then(|c| {
            super::build_space_mind_seal(c.unit, c.sortie, c.mind.as_str(), c.source_sha256)
        }))
    }
}

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
    /// Carrier submitted to inspection with unlicensed `strategic_ore` but lacked sufficient credit to pay the fine; contraband was seized and the ship is cleared (`SF.R12`).
    ClearedAfterSeizure,
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
    /// Carrier lacked credit to pay the customs fine; the unlicensed `strategic_ore` was seized
    /// from the hull's hold vault and [`mush_faction::CUSTOMS_ASSERTION_TITLE`] advanced (`SF.R12`).
    ContrabandSeized {
        /// Seized `strategic_ore` quantity in kilograms.
        seized_strategic_ore_kg: u64,
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
    /// Persistent Rhai memory bytes for the Republic Customs Corvette (`kirkwood_picket.rhai`).
    pub corvette_memory: Vec<u8>,
    /// Think sequence counter for the Republic Customs Corvette.
    pub corvette_think_seq: u64,
    /// Last trace lines emitted by the Republic Customs Corvette's `RhaiRunner`.
    pub corvette_last_trace: Vec<String>,
    /// Last station intents emitted by the Republic Customs Corvette's `RhaiRunner`.
    pub corvette_last_intents: Vec<IntentKind>,
    /// Sealed mind record for the Republic Customs Corvette (`SF.R14`, `D-35-1`, `D-35-17`).
    #[serde(default, with = "compact_mind_seal")]
    pub corvette_seal: Option<crate::arena::mind::MindSeal>,
    /// Persistent Rhai memory bytes for the Belt Raider (`belt_raider.rhai`).
    pub raider_memory: Vec<u8>,
    /// Think sequence counter for the Belt Raider.
    pub raider_think_seq: u64,
    /// Last trace lines emitted by the Belt Raider's `RhaiRunner`.
    pub raider_last_trace: Vec<String>,
    /// Last station intents emitted by the Belt Raider's `RhaiRunner`.
    pub raider_last_intents: Vec<IntentKind>,
    /// Sealed mind record for the Belt Raider (`SF.R14`, `D-35-1`, `D-35-17`).
    #[serde(default, with = "compact_mind_seal")]
    pub raider_seal: Option<crate::arena::mind::MindSeal>,
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
        let corvette_seal = build_space_mind_seal(
            corvette_hull,
            crate::arena::mind::SortieId::ic(corvette_hull, 1),
            "kirkwood_picket",
            crate::minds::content::kirkwood_picket_digest().0,
        );
        let raider_seal = build_space_mind_seal(
            raider_hull,
            crate::arena::mind::SortieId::ic(raider_hull, 1),
            "belt_raider",
            crate::minds::content::belt_raider_digest().0,
        );
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
            corvette_seal,
            raider_memory: Vec::new(),
            raider_think_seq: 0,
            raider_last_trace: Vec::new(),
            raider_last_intents: Vec::new(),
            raider_seal,
        }
    }

    /// Return whether the customs picket inspection is satisfied (`Cleared`, `ClearedAfterFine`, or `ClearedAfterSeizure`).
    #[must_use]
    pub const fn is_customs_cleared(&self) -> bool {
        matches!(
            self.inspection_status,
            PicketInspectionStatus::Cleared
                | PicketInspectionStatus::ClearedAfterFine
                | PicketInspectionStatus::ClearedAfterSeizure
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

/// Check whether `host` is a location entry authored with `kirkwood_picket: true` in an applied pack
/// (`SF.R14`, `D-35-11`, `D-35-17`).
#[must_use]
pub fn is_kirkwood_picket_host(w: &World, host: ObjId) -> bool {
    let Some(store) = w.module_state::<crate::store::TacsimStore>() else {
        return false;
    };
    for loaded in store.packs() {
        let manifest = loaded.pack.manifest();
        if store.applied_in(w, manifest.id()) != Some(manifest.version()) {
            continue;
        }
        let Some(placed) = store.placed_in(w, manifest.id(), manifest.version()) else {
            continue;
        };
        for (entry_id, obj) in placed {
            if obj == host
                && let Some(crate::pack::PackEntry::Location(loc)) = loaded.pack.entry(&entry_id)
                && loc.kirkwood_picket
            {
                return true;
            }
        }
    }
    false
}

/// Check whether a promoted carrier on `domain` should open a multi-party **Kirkwood Customs Picket**
/// bubble (`CARRIER_PARTY`, `CUSTOMS_CORVETTE_PARTY`, `BELT_RAIDER_PARTY`, `SF.R14`, `D-35-11`, `D-35-17`).
///
/// Returns `true` only when `domain` hosts a location authored with `kirkwood_picket: true`
/// (`SpaceBubbleBook::kirkwood_picket_domains` or [`is_kirkwood_picket_host`]).
#[must_use]
pub fn should_open_kirkwood_picket(w: &World, _hull: ObjId, domain: mush_sim::DomainId) -> bool {
    let Some(book) = w.module_state::<super::tick::SpaceBubbleBook>() else {
        return false;
    };
    if book.kirkwood_picket_domains.contains(&domain) {
        return true;
    }
    if let Some(sim) = w.module_state::<mush_sim::SpatialState>()
        && let Some(dom) = sim.domains.get(&domain)
        && is_kirkwood_picket_host(w, dom.host)
    {
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

/// Run one Rhai [`MindRunner::think`] step for a collector drone sortie (`drone_mining.rhai`, `SF.R14`, `D-35-13`).
#[must_use]
pub fn run_drone_rhai_think(
    drone_obj: ObjId,
    pos_m: [i64; 3],
    think_seq: u64,
    memory: &[u8],
) -> Option<ThinkResult> {
    let runner = cached_space_rhai_runner(crate::minds::content::drone_mining_source())?;
    let view = Arc::new(MindView {
        unit: drone_obj.0,
        callsign: Designator::Unit {
            party: CARRIER_PARTY.to_party_id(),
            letter: 'A',
        },
        party: CARRIER_PARTY.to_party_id(),
        cell: Some((8, 8)),
        mode: MatchMode::Skirmish,
        own_last_hit_ms: None,
        objective: None,
        motor: None,
        x_cm: pos_m[0].saturating_mul(100),
        y_cm: pos_m[1].saturating_mul(100),
        z_cm: pos_m[2].saturating_mul(100),
        heading_mdeg: 0,
        speed_cms: 0,
        power: Power::Online,
        held: false,
        wreck: false,
        immobile: false,
        sight_m: 10_000,
        ecm: Some(false),
        receiver: Some(FrameReceiver {
            budget: 100,
            budget_max: 100,
            reach_permille: 1_000,
        }),
        heat: None,
        sections: Vec::new(),
        lock: None,
        weapons: Vec::new(),
        rules: Vec::new(),
        order: None,
        t_ms: i64::try_from(think_seq.saturating_mul(1_000)).unwrap_or(0),
        notices: Vec::<FrameNotice>::new(),
        contacts: Vec::new(),
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
        unit: drone_obj,
        generation: 0,
        key: card.key(),
    };
    let map: Arc<ArenaDefinition> = CACHED_SPACE_ARENA_DEF.as_ref()?.clone();
    let t_ms = i64::try_from(think_seq.saturating_mul(1_000)).unwrap_or(0);
    let job = ThinkJob {
        unit: drone_obj,
        think_seq,
        t_ms,
        lands_ms: t_ms.saturating_add(500),
        seed: u64::from(drone_obj.0).wrapping_add(think_seq),
        dropped: false,
        profile: &core_profile,
        view: &view,
        memory,
        map: &map,
        card: &card,
        program: &program,
        profile_token,
    };
    Some(runner.think(&job))
}

fn sync_corvette_cleared_stage_in_memory(picket: &mut KirkwoodPicketState) {
    if !picket.is_customs_cleared() {
        return;
    }
    let mut mem = crate::arena::mind::decode_memory(&picket.corvette_memory);
    if !matches!(mem.get("stage"), Some(mush_mind::memory::Value::Int(3 | 5))) {
        mem.insert("stage".to_owned(), mush_mind::memory::Value::Int(5));
        picket.corvette_memory = crate::arena::mind::encode_memory(&mem);
    }
}

fn read_corvette_stage_from_memory(memory: &[u8]) -> Option<i64> {
    let mem = crate::arena::mind::decode_memory(memory);
    match mem.get("stage") {
        Some(mush_mind::memory::Value::Int(stage)) => Some(*stage),
        _ => None,
    }
}

fn step_corvette_mind_on_bubble(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    picket: &mut KirkwoodPicketState,
    carrier_ship_id: &str,
    emitted_events: &mut Vec<super::bubble::SpaceCombatEvent>,
) {
    sync_corvette_cleared_stage_in_memory(picket);
    let corvette_digest = picket.corvette_seal.as_ref().map_or_else(
        || crate::minds::content::kirkwood_picket_digest().0,
        |s| s.source_sha256,
    );
    let corvette_source = crate::minds::content::source_lookup(None, &corvette_digest)
        .unwrap_or_else(|| crate::minds::content::kirkwood_picket_source());

    if let Some(corvette) = bubble.ships.get(&picket.corvette_ship_id)
        && corvette.status == HullStatus::Operational
        && let Some((res, cmap)) = run_space_rhai_think(
            bubble,
            &picket.corvette_ship_id,
            picket.corvette_hull,
            corvette_source,
            picket.corvette_think_seq,
            &picket.corvette_memory,
        )
        && res.fault.is_none()
    {
        picket.corvette_think_seq = picket.corvette_think_seq.saturating_add(1);
        picket.corvette_memory = res.memory;
        picket.corvette_last_trace = res.trace;
        picket.corvette_last_intents = res.intents.clone();

        // Read `this.stage` from the Corvette's Rhai memory (`SF.R14`, `D-35-1`, `D-35-11`, `D-35-17`):
        if let Some(stage) = read_corvette_stage_from_memory(&picket.corvette_memory) {
            if stage == 4 && !picket.is_customs_cleared() {
                picket.inspection_status = PicketInspectionStatus::RanThePicket;
                bubble.set_party_relation(
                    CUSTOMS_CORVETTE_PARTY,
                    CARRIER_PARTY,
                    SpaceRelation::Hostile,
                );
            } else if stage == 2
                && picket.inspection_status == PicketInspectionStatus::AwaitingHeaveTo
            {
                picket.inspection_status = PicketInspectionStatus::HoveToReadyForInspection;
            }
        }

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

    // Perimeter backstop if an undetected (`Ghost`) carrier drifts past the 35 km picket boundary.
    if !picket.is_customs_cleared()
        && picket.inspection_status != PicketInspectionStatus::RanThePicket
        && let (Some(carrier), Some(corvette)) = (
            bubble.ships.get(carrier_ship_id),
            bubble.ships.get(&picket.corvette_ship_id),
        )
        && carrier.status == HullStatus::Operational
        && corvette.status == HullStatus::Operational
        && distance_3d_m(carrier.pos_m, corvette.pos_m) > PICKET_RUN_BOUNDARY_M
    {
        picket.inspection_status = PicketInspectionStatus::RanThePicket;
        bubble.set_party_relation(
            CUSTOMS_CORVETTE_PARTY,
            CARRIER_PARTY,
            SpaceRelation::Hostile,
        );
    }
}

fn step_raider_mind_on_bubble(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    picket: &mut KirkwoodPicketState,
    carrier_ship_id: &str,
    emitted_events: &mut Vec<super::bubble::SpaceCombatEvent>,
) {
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

    let raider_digest = picket.raider_seal.as_ref().map_or_else(
        || crate::minds::content::belt_raider_digest().0,
        |s| s.source_sha256,
    );
    let raider_source = crate::minds::content::source_lookup(None, &raider_digest)
        .unwrap_or_else(|| crate::minds::content::belt_raider_source());

    if let Some(raider) = bubble.ships.get(&picket.raider_ship_id)
        && raider.status == HullStatus::Operational
        && raider.emcon != EmconPosture::Ghost
        && let Some((res, cmap)) = run_space_rhai_think(
            bubble,
            &picket.raider_ship_id,
            picket.raider_hull,
            raider_source,
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
}

/// Step the Kirkwood Customs Picket `RhaiRunner` doctrines (`kirkwood_picket.rhai` and
/// `belt_raider.rhai`), reading `this.stage` from `picket.corvette_memory` (`SF.R14`, `D-35-11`, `D-35-17`).
pub fn step_kirkwood_picket(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    picket: &mut KirkwoodPicketState,
    carrier_ship_id: &str,
) -> Vec<super::bubble::SpaceCombatEvent> {
    let mut emitted_events = Vec::new();
    if picket.inspection_outcome.is_some() {
        step_raider_mind_on_bubble(bubble, tape, picket, carrier_ship_id, &mut emitted_events);
        step_corvette_mind_on_bubble(bubble, tape, picket, carrier_ship_id, &mut emitted_events);
    } else {
        step_corvette_mind_on_bubble(bubble, tape, picket, carrier_ship_id, &mut emitted_events);
        step_raider_mind_on_bubble(bubble, tape, picket, carrier_ship_id, &mut emitted_events);
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

/// Seize unlicensed `strategic_ore` from `hull`'s hold vault when `pilot` cannot pay the customs
/// fine, and advance [`mush_faction::CUSTOMS_ASSERTION_TITLE`] (`SF.R12`, `35b` §5 #3).
pub fn seize_contraband_and_advance_front(
    ctx: &mut Ctx<'_>,
    pilot: ObjId,
    hull: ObjId,
    unlicensed_strategic_ore_kg: u64,
) -> Option<FrontId> {
    let seize_t = u32::try_from(unlicensed_strategic_ore_kg.div_ceil(1_000).max(1)).unwrap_or(1);
    if let Some(view) = crate::port::ship::hull_view(ctx.w, hull)
        && let Some(hold_vault) = crate::port::ship::hold_vault(ctx.w, &view)
    {
        let _ = super::commands::submit_seized_contraband_burn(ctx, hull, hold_vault.0, seize_t);
    }

    let (registry_org, _, _) = resolve_registry_principal_and_account(ctx.w)?;
    let saved_actor = ctx.actor;
    ctx.actor = pilot;
    let front_id = mush_faction::record_customs_assertion(
        ctx,
        registry_org,
        mush_canon::CanonDraft {
            kind: "space.customs_seizure".into(),
            summary: format!(
                "Republic Customs Corvette seized {} kg unlicensed strategic_ore from hull #{} at the Kirkwood picket in lieu of unpaid fine; {} advances.",
                unlicensed_strategic_ore_kg,
                hull.0,
                mush_faction::CUSTOMS_ASSERTION_TITLE,
            ),
            entities: vec![pilot, hull],
            scope: mush_canon::CanonScope::Public,
            ic_time: ctx.w.world_ic_now().at,
            source: format!("space.picket.seize/{}/{}", hull.0, ctx.w.world_ic_now().at),
            detail: json!({
                "hull": hull.0,
                "pilot": pilot.0,
                "seized_strategic_ore_kg": unlicensed_strategic_ore_kg,
            }),
        },
    )
    .ok();
    ctx.actor = saved_actor;
    ctx.out.pin_deferred();
    front_id
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

fn configure_corvette_and_raider_ships(corvette: &mut SpaceShipState, raider: &mut SpaceShipState) {
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

    raider.max_accel_mps2 = 60;
    raider.delta_v_mps = 9_500;
    raider.torpedo_tubes = vec![super::weapons::TorpedoTube::standard_twin("torp_bow", 8)];
    raider.lasers = vec![
        super::weapons::LaserMount::standard_turret("laser_1"),
        super::weapons::LaserMount::standard_turret("laser_2"),
    ];
    raider.railguns = vec![super::weapons::RailgunMount::standard_spinal(
        "rail_spinal",
        24,
    )];
    raider.pdcs = vec![
        super::weapons::PdcMount::standard_rotary("pdc_port", 3_000),
        super::weapons::PdcMount::standard_rotary("pdc_stbd", 3_000),
    ];
    raider.decoys_remaining = 6;
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
}

/// Spawn real world `Thing` hulls and checked-out [`mush_tactics::TacticalUnit`]s for the
/// Republic Customs Corvette (`"tacsim.unit.corvette"`) and Belt Raider (from `raiders.json`),
/// and insert them alongside their 3-party relations into `bubble` (`SF.R12`, `D-35-1`, `D-35-11`).
pub fn spawn_kirkwood_picket_ships_in_world(
    ctx: &mut Ctx<'_>,
    bubble: &mut LocalSpaceBubble,
    domain: mush_sim::DomainId,
    leg_cursor: u32,
    _carrier_ship_id: &str,
) -> Option<KirkwoodPicketState> {
    let site_id = format!("domain.{}", domain.0);
    let corvette_ship_id = format!("customs.{}.{}", domain.0, leg_cursor);
    let raider_ship_id = format!("raider.{}.{}", domain.0, leg_cursor);

    let corvette_hull = ctx
        .w
        .create(
            mush_core::ObjKind::Thing,
            "Republic Customs Corvette",
            mush_core::GOD,
            None,
        )
        .ok()?;
    super::custody::create_stock_space_unit_in_world(
        ctx,
        corvette_hull,
        crate::definitions::content::CORVETTE,
    )
    .ok()?;
    let mut corvette = super::custody::check_out_space_unit(
        ctx,
        corvette_hull,
        &site_id,
        leg_cursor,
        &corvette_ship_id,
        "Republic Customs Corvette",
        CUSTOMS_CORVETTE_PARTY,
        [-CORVETTE_STANDOFF_M, 400, 0],
        [10_000, 0, 0],
    )
    .ok()?;

    let raider_defs = crate::field::raiders::raiders().ok()?;
    let raider_def = raider_defs.first()?;
    let leader = crate::field::raiders::hulls_of(ctx.w, raider_def.id.as_str())
        .into_iter()
        .find_map(|h| crate::field::raiders::leader_of(ctx.w, h))
        .or_else(|| {
            let l = ctx
                .w
                .create(
                    mush_core::ObjKind::Npc,
                    &raider_def.leader,
                    mush_core::GOD,
                    None,
                )
                .ok()?;
            if ctx.w.module_state::<mush_faction::FactionState>().is_some() {
                let _ = mush_faction::register_content_faction(ctx, &raider_def.organization, l);
            }
            Some(l)
        })?;

    let raider_hull = ctx
        .w
        .create(mush_core::ObjKind::Thing, "Belt Raider", leader, None)
        .ok()?;
    super::custody::create_stock_space_unit_in_world(
        ctx,
        raider_hull,
        raider_def.carrier.unit.as_str(),
    )
    .ok()?;
    let _ = ctx.w.set_attr(
        raider_hull,
        crate::field::raiders::RAIDER_ATTR,
        mush_core::Attr::json(
            serde_json::json!({
                "raider": raider_def.id.as_str(),
                "leader": leader.0,
            }),
            mush_core::GOD,
        ),
    );
    let mut raider = super::custody::check_out_space_unit(
        ctx,
        raider_hull,
        &site_id,
        leg_cursor,
        &raider_ship_id,
        "Belt Raider",
        BELT_RAIDER_PARTY,
        [RAIDER_AMBUSH_RANGE_M, 0, 0],
        [-20_000, 0, 0],
    )
    .ok()?;

    configure_corvette_and_raider_ships(&mut corvette, &mut raider);

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

    let now_ic = bubble.ic_timestamp_s.max(1);
    let corvette_seal = seal_space_unit_in_world(
        ctx,
        corvette_hull,
        crate::arena::mind::SortieId::ic(corvette_hull, now_ic),
        "kirkwood_picket",
        crate::minds::content::kirkwood_picket_digest().0,
    );
    let raider_seal = seal_space_unit_in_world(
        ctx,
        raider_hull,
        crate::arena::mind::SortieId::ic(raider_hull, now_ic),
        "belt_raider",
        crate::minds::content::belt_raider_digest().0,
    );

    let mut state =
        KirkwoodPicketState::new(corvette_hull, corvette_ship_id, raider_hull, raider_ship_id);
    if corvette_seal.is_some() {
        state.corvette_seal = corvette_seal;
    }
    if raider_seal.is_some() {
        state.raider_seal = raider_seal;
    }
    Some(state)
}

/// Populate the Republic Customs Corvette and Belt Raider ships for unit tests without a `World`.
#[cfg(test)]
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
    let mut raider = SpaceShipState::new_corvette(
        &raider_ship_id,
        "Belt Raider",
        BELT_RAIDER_PARTY,
        [RAIDER_AMBUSH_RANGE_M, 0, 0],
        [-20_000, 0, 0],
    );
    configure_corvette_and_raider_ships(&mut corvette, &mut raider);

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
