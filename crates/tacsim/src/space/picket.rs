//! **Kirkwood Customs Picket — Rhai `MindRunner` Doctrines, Multi-Party Triad Combat,
//! Live `stow.ore` + Registry Inspection, Ledger Fine & Customs Assertion Front (`SF.R6`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §9 Story 1, §10.2 `SF.R6`,
//! `D-35-1`, `D-35-10`, `D-35-11`, `D-35-12`, `docs/design/35a-space-flight-review.md` §1, §4, §8.3).
//!
//! Connects a promoted Kirkwood interdiction [`LocalSpaceBubble`] to:
//! 1. **Three distinct [`crate::arena::party::PartyId`] parties**:
//!    - [`CARRIER_PARTY`] (`PartyId::nth(0)` = `'A'`): The player's `Dray-P` carrier.
//!    - [`CUSTOMS_CORVETTE_PARTY`] (`PartyId::nth(1)` = `'B'`): **Republic Customs Corvette**
//!      (`Relation::Neutral` toward the carrier under the bubble's inspecting flag, Q-40-22;
//!      `Relation::Hostile` toward the Belt Raider), flown by a compiled [`crate::minds::MindPool`] runner executing `kirkwood_picket.rhai`.
//!    - [`BELT_RAIDER_PARTY`] (`PartyId::nth(2)` = `'C'`): **Belt Raider** (`Relation::Hostile`
//!      toward both the carrier and the Republic Customs Corvette), flown by a compiled
//!      [`crate::minds::MindPool`] runner executing `belt_raider.rhai`.
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
//!      Customs Corvette to [`Relation::Hostile`](crate::arena::party::Relation::Hostile), and advances the
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

use super::applier::Seat;
use super::bubble::{
    BubbleInputTape, EmconPosture, HullStatus, LocalSpaceBubble, SpaceShipState, admitted_party,
};
use super::view::{ContactBook, Situation, Stop};
use super::weapons::isqrt_i128;
use crate::arena::card::line;
use crate::arena::core::{ClassTable, CoreClass};
use crate::arena::intents::IntentKind;
use crate::arena::mind::{MindView, ThinkResult};
use crate::arena::party::{Mark, PartyId};
use crate::arena::perception::{FrameNotice, Receiver as FrameReceiver};
use crate::arena::profile_rules::BoundProgram;
use crate::arena::profile_runtime::ProfileToken;
use crate::arena::runtime_card::{CardCatalog, ProfileKey, stock_definition};
use crate::arena::state::{
    ArenaDefinition, ArenaTerrain, ContactId, Designator, MatchMode, Power, TerrainCell, Waypoints,
};
use crate::config::{ArenaConfig, ReflexConfig};
use crate::port::apply::{credit_scale, minor_of};
use crate::port::licence::{LICENCE_SETTLE_WINDOW_SECS, licence_gate, licence_port};

/// Party A (`PartyId::nth(0)` = `'A'`): Promoted player carrier (`Dray-P`).
pub const CARRIER_PARTY: PartyId = admitted_party(0);

/// Party B (`PartyId::nth(1)` = `'B'`): **Republic Customs Corvette** (`Neutral` and inspecting to carrier, `Hostile` to raider).
pub const CUSTOMS_CORVETTE_PARTY: PartyId = admitted_party(1);

/// Party C (`PartyId::nth(2)` = `'C'`): **Belt Raider** (`Hostile` to both carrier and customs corvette).
pub const BELT_RAIDER_PARTY: PartyId = admitted_party(2);

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

/// **The doctrine the space cast's `actor` is to be sealed with now** (doc 44 unit MW.10;
/// Q-44-25): staff's selection, else the pack's, from the store by its digest
/// ([`crate::minds::content::cast::cast_doctrine`]). `None`, with an error line that names the
/// actor and the digest, when the store does not hold it: the caller opens no bubble for her.
///
/// The caller asks before anything of a bubble is built and before the carrier is checked
/// out, so a refusal leaves the world as it was.
pub fn cast_doctrine_in_world(
    ctx: &mut Ctx<'_>,
    actor: crate::minds::content::cast::CastActor,
) -> Option<crate::minds::content::cast::CastDoctrine> {
    if let Some(store) = ctx.w.module_state_mut::<crate::store::TacsimStore>() {
        crate::minds::content::ensure_space_mind_sources_installed(store);
    }
    let store = ctx.w.module_state::<crate::store::TacsimStore>();
    crate::minds::content::cast::cast_doctrine(store, actor)
        .map_err(|fault| tracing::error!("tacsim: {fault}; no bubble is opened"))
        .ok()
}

/// **The seal an open bubble's actor comes back with after a restart** (doc 44 unit MW.10).
///
/// `record` is the seal the bubble's own document rebuilt (`compact_mind_seal`: the unit, the
/// sortie, the doctrine's name and its digest). `attribute` is the whole seal read from the
/// hull. The attribute is taken only when it is the record's seal: the same unit, sortie and
/// digest. A hull is reused by name from one bubble to the next, so an attribute that
/// disagrees is some other bubble's, and the record stands. With no record the attribute is
/// all there is.
#[must_use]
pub(crate) fn reloaded_seal(
    record: Option<crate::arena::mind::MindSeal>,
    attribute: Option<crate::arena::mind::MindSeal>,
) -> Option<crate::arena::mind::MindSeal> {
    match (record, attribute) {
        (Some(record), Some(attribute))
            if attribute.unit == record.unit
                && attribute.sortie == record.sortie
                && attribute.source_sha256 == record.source_sha256 =>
        {
            Some(attribute)
        }
        (Some(record), _) => Some(record),
        (None, attribute) => attribute,
    }
}

/// **A cast actor's seal in a rig with no world** (doc 44 unit MW.10): the pack's doctrine,
/// by the pack's digest, resolved with no store. A world seals its actors itself
/// ([`cast_doctrine_in_world`]) and never calls this, so it is built for tests alone.
#[cfg(test)]
pub(crate) fn pack_cast_seal(
    actor: crate::minds::content::cast::CastActor,
    unit: ObjId,
) -> Option<crate::arena::mind::MindSeal> {
    let doctrine = crate::minds::content::cast::cast_doctrine(None, actor).ok()?;
    build_space_mind_seal(
        unit,
        crate::arena::mind::SortieId::ic(unit, 1),
        doctrine.name.as_str(),
        doctrine.digest,
    )
}

impl KirkwoodPicketState {
    /// This record with both actors sealed as [`pack_cast_seal`] seals them: for a rig with
    /// no world.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn sealed_by_pack(mut self) -> Self {
        use crate::minds::content::cast::CastActor;
        self.corvette_seal = pack_cast_seal(CastActor::Corvette, self.corvette_hull);
        self.raider_seal = pack_cast_seal(CastActor::Raider, self.raider_hull);
        self
    }
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
    /// Corvette to [`Relation::Hostile`](crate::arena::party::Relation::Hostile) and advancing [`mush_faction::CUSTOMS_ASSERTION_TITLE`].
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
    /// The corvette's own contact numbers (doc 44 unit MW.9; [`ContactBook`]).
    #[serde(default, skip_serializing_if = "ContactBook::is_empty")]
    pub corvette_contacts: ContactBook,
    /// The raider's own contact numbers (doc 44 unit MW.9; [`ContactBook`]).
    #[serde(default, skip_serializing_if = "ContactBook::is_empty")]
    pub raider_contacts: ContactBook,
    /// Per-carrier customs inspection status keyed by carrier hull [`ObjId`] (`SF.R18`).
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "super::bubble::vec_tuple_map"
    )]
    pub carrier_inspection_status: BTreeMap<ObjId, PicketInspectionStatus>,
    /// Per-carrier customs inspection outcome keyed by carrier hull [`ObjId`] (`SF.R18`).
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "super::bubble::vec_tuple_map"
    )]
    pub carrier_inspection_outcome: BTreeMap<ObjId, PicketInspectionOutcome>,
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
        // Doc 44 unit MW.10: a fresh record holds no seal. The spawn puts in the seals the
        // world took; nothing here names a doctrine.
        let (corvette_seal, raider_seal) = (None, None);
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
            corvette_contacts: ContactBook::default(),
            raider_contacts: ContactBook::default(),
            carrier_inspection_status: BTreeMap::new(),
            carrier_inspection_outcome: BTreeMap::new(),
        }
    }

    /// Return the customs inspection status for carrier `hull` (`SF.R18`).
    #[must_use]
    pub fn inspection_status_for(&self, hull: ObjId) -> PicketInspectionStatus {
        self.carrier_inspection_status
            .get(&hull)
            .copied()
            .unwrap_or(self.inspection_status)
    }

    /// Return the customs inspection outcome for carrier `hull` (`SF.R18`).
    #[must_use]
    pub fn inspection_outcome_for(&self, hull: ObjId) -> Option<&PicketInspectionOutcome> {
        self.carrier_inspection_outcome
            .get(&hull)
            .or(self.inspection_outcome.as_ref())
    }

    /// Record the customs inspection status for carrier `hull` (`SF.R18`).
    pub fn set_inspection_status_for(&mut self, hull: ObjId, status: PicketInspectionStatus) {
        self.inspection_status = status;
        self.carrier_inspection_status.insert(hull, status);
    }

    /// Record the customs inspection outcome for carrier `hull` (`SF.R18`).
    pub fn set_inspection_outcome_for(&mut self, hull: ObjId, outcome: PicketInspectionOutcome) {
        self.inspection_outcome = Some(outcome.clone());
        self.carrier_inspection_outcome.insert(hull, outcome);
    }

    /// Return whether carrier `hull` has satisfied the customs picket inspection (`SF.R18`).
    #[must_use]
    pub fn is_carrier_customs_cleared(&self, hull: ObjId) -> bool {
        matches!(
            self.inspection_status_for(hull),
            PicketInspectionStatus::Cleared
                | PicketInspectionStatus::ClearedAfterFine
                | PicketInspectionStatus::ClearedAfterSeizure
        )
    }

    /// Whether the Customs Corvette is still screening any carrier (hailed or hove-to, neither
    /// cleared nor run): the bubble's inspecting flag on its party (Q-40-22, `SF.R18`).
    #[must_use]
    pub fn is_screening(&self) -> bool {
        if self.carrier_inspection_status.is_empty() {
            matches!(
                self.inspection_status,
                PicketInspectionStatus::AwaitingHeaveTo
                    | PicketInspectionStatus::HoveToReadyForInspection
            )
        } else {
            self.carrier_inspection_status.values().any(|s| {
                matches!(
                    s,
                    PicketInspectionStatus::AwaitingHeaveTo
                        | PicketInspectionStatus::HoveToReadyForInspection
                )
            })
        }
    }

    /// Return whether the customs picket inspection is satisfied (`Cleared`, `ClearedAfterFine`, or `ClearedAfterSeizure`).
    #[must_use]
    pub fn is_customs_cleared(&self) -> bool {
        if self.carrier_inspection_status.is_empty() {
            matches!(
                self.inspection_status,
                PicketInspectionStatus::Cleared
                    | PicketInspectionStatus::ClearedAfterFine
                    | PicketInspectionStatus::ClearedAfterSeizure
            )
        } else {
            self.carrier_inspection_status
                .keys()
                .all(|&h| self.is_carrier_customs_cleared(h))
        }
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

/// Resolve the customs principal (`ObjId`) responsible for `hull`'s manifest and fines (`SF.R23`, `35b` §5 #15):
/// prefers the hull's live Co-op charter tenant or active `EconomyStore` asset title holder
/// (`crate::port::coop::helm_holder`), falling back to `fallback_actor`.
#[must_use]
pub fn resolve_hull_customs_principal(w: &World, hull: ObjId, fallback_actor: ObjId) -> ObjId {
    if let Some(holder) = crate::port::coop::helm_holder(w, hull) {
        return holder;
    }
    if let Some(PrincipalId::Object(owner)) = w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().asset(hull))
        .map(|a| a.title_owner.clone())
        && owner != hull
    {
        return owner;
    }
    fallback_actor
}

/// Inspect the carrier hull's live `stow.ore` vault and the hull principal's Freeport Registry
/// prospecting licence (`licence_gate`) and per-block `tacsim.claims` tenure (`D-35-11`, `SF.R23`, `35b` §5 #15).
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
    if let Some(raw_actor) = pilot {
        let principal_actor = resolve_hull_customs_principal(ctx.w, hull, raw_actor);
        manifest.licensed = licence_gate(ctx.w, principal_actor, now_ic).is_ok()
            || licence_gate(ctx.w, raw_actor, now_ic).is_ok();

        let mined_blocks = ctx
            .w
            .module_state::<super::tick::SpaceBubbleBook>()
            .and_then(|b| b.mined_strategic_blocks.get(&hull))
            .filter(|blocks| !blocks.is_empty())
            .cloned();

        if let Some(blocks) = mined_blocks {
            manifest.has_registered_claim = blocks.iter().all(|(site, letter)| {
                super::mining::actor_holds_cluster_block_claim(
                    ctx,
                    site,
                    *letter,
                    principal_actor,
                    now_ic,
                ) || super::mining::actor_holds_cluster_block_claim(
                    ctx, site, *letter, raw_actor, now_ic,
                )
            });
        } else {
            'search: for cluster in super::mining::all_orbital_clusters(ctx.w) {
                let site = cluster.id.clone();
                for rock in &cluster.rocks {
                    if !rock
                        .spectral
                        .families()
                        .contains(&crate::surface::OreFamily::StrategicElements)
                    {
                        continue;
                    }
                    if super::mining::actor_holds_cluster_block_claim(
                        ctx,
                        &site,
                        rock.letter,
                        principal_actor,
                        now_ic,
                    ) || super::mining::actor_holds_cluster_block_claim(
                        ctx,
                        &site,
                        rock.letter,
                        raw_actor,
                        now_ic,
                    ) {
                        manifest.has_registered_claim = true;
                        break 'search;
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

/// Queued light-lagged radio hail awaiting delivery at `deliver_at_real_s` (`now + d/c`, `D-35-10`, `SF.R23`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingLightLagHail {
    /// Destination hull [`ObjId`].
    pub hull: ObjId,
    /// Sender label (e.g., `"Republic Customs Corvette"`).
    pub sender: String,
    /// Hail body message.
    pub message: String,
    /// Distance in metres (`d`).
    pub distance_m: u64,
    /// Whole seconds of one-way light-lag (`d / c`).
    pub light_lag_s: u64,
    /// Milliseconds of one-way light-lag (`(d * 1000) / c`).
    pub light_lag_ms: u64,
    /// Wall-clock real second (`ctx.w.now()`) at or after which the hail arrives at `hull`.
    pub deliver_at_real_s: i64,
}

/// Deliver a [`PendingLightLagHail`] to `hail.hull` and its bridge crew on `"tacsim:notice"` (`D-35-10`, `SF.R23`).
pub fn deliver_light_lagged_hail_now(ctx: &mut Ctx<'_>, hail: &PendingLightLagHail) {
    let text = format!(
        "[RADIO +{}.{:03}s light-lag ({} m)] {}: \"{}\"",
        hail.light_lag_ms / 1_000,
        hail.light_lag_ms % 1_000,
        hail.distance_m,
        hail.sender,
        hail.message,
    );
    crate::port::voyage::narrate_hull(ctx, hail.hull, &text);

    if let Some(view) = crate::port::ship::hull_view(ctx.w, hail.hull)
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
            "sender": hail.sender,
            "distance_m": hail.distance_m,
            "light_lag_s": hail.light_lag_s,
            "light_lag_ms": hail.light_lag_ms,
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
}

/// Deliver all due [`PendingLightLagHail`]s whose `deliver_at_real_s <= ctx.w.now()` (`D-35-10`, `SF.R23`).
pub fn step_pending_light_lag_hails(ctx: &mut Ctx<'_>) {
    let now_real_s = ctx.w.now();
    let due_hails: Vec<PendingLightLagHail> = {
        let Some(book) = ctx.w.module_state_mut::<super::tick::SpaceBubbleBook>() else {
            return;
        };
        if book.pending_light_lag_hails.is_empty() {
            return;
        }
        let mut due = Vec::new();
        let mut remaining = Vec::new();
        for hail in book.pending_light_lag_hails.drain(..) {
            if now_real_s >= hail.deliver_at_real_s {
                due.push(hail);
            } else {
                remaining.push(hail);
            }
        }
        book.pending_light_lag_hails = remaining;
        due
    };
    for hail in &due_hails {
        deliver_light_lagged_hail_now(ctx, hail);
    }
}

/// Emit or queue a light-lagged radio hail (`mush_propagation::light_delay_secs`) from `sender`
/// to `hull` at range `distance_m` (`D-35-10`, `SF.R23`).
///
/// When `lag_s == 0` (`distance_m < LIGHT_SPEED_MPS`, e.g. 6 km standoff = 20 ms), the hail arrives
/// within the current 1 Hz tick and is delivered immediately. When `lag_s >= 1`, the hail is queued
/// in `SpaceBubbleBook::pending_light_lag_hails` and delivered on `space_tick` at `now + lag_s`.
pub fn queue_or_deliver_light_lagged_hail(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    sender: impl Into<String>,
    message: impl Into<String>,
    distance_m: i64,
) -> (u64, u64) {
    let dist_u64 = u64::try_from(distance_m.max(0)).unwrap_or(0);
    let lag_ms_u64 = super::commands::vacuum_light_lag_ms(dist_u64);
    let lag_s = lag_ms_u64 / 1_000;
    let hail = PendingLightLagHail {
        hull,
        sender: sender.into(),
        message: message.into(),
        distance_m: dist_u64,
        light_lag_s: lag_s,
        light_lag_ms: lag_ms_u64,
        deliver_at_real_s: ctx
            .w
            .now()
            .saturating_add(i64::try_from(lag_s).unwrap_or(0)),
    };
    if lag_s == 0 {
        deliver_light_lagged_hail_now(ctx, &hail);
    } else if let Some(book) = ctx.w.module_state_mut::<super::tick::SpaceBubbleBook>() {
        book.pending_light_lag_hails.push(hail);
    }
    (lag_s, lag_ms_u64)
}

/// Emit a light-lagged radio hail (`mush_propagation::light_delay_secs`) from the
/// Republic Customs Corvette to the carrier hull and publish a structured `"tacsim:notice"`
/// event to every bridge crew member aboard (`D-35-10`, `doc 35` §9 Story 1 step 2, `35a` §4, `SF.R23`).
pub fn emit_light_lagged_picket_hail(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    distance_m: i64,
) -> (u64, u64) {
    queue_or_deliver_light_lagged_hail(
        ctx,
        hull,
        "Republic Customs Corvette",
        "Heave-to under 10 m/s (|dv| < 10.0 m/s) and run +helm/submit for Freeport Registry manifest & claim inspection.",
        distance_m,
    )
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

fn build_minimal_sealed_space_arena() -> Option<crate::arena::state::Arena> {
    use crate::arena::mind::MindSeal;
    use crate::arena::party::FieldHost;
    use crate::arena::resolution::UnitProfile;
    use crate::arena::state::{Arena, Controller, CustodyOut, CustodyUnit, TrackedRules};
    use crate::config::{MatchConfig, SensorsConfig};
    use mush_sim::data::Pose;

    let definition = minimal_space_arena_definition()?;
    let (unit, _) = super::custody::instantiate_stock_space_unit(
        ObjId(1),
        crate::definitions::content::CORVETTE,
    )
    .ok()?;
    let seal = build_space_mind_seal(
        ObjId(1),
        crate::arena::mind::SortieId::ic(ObjId(1), 1),
        "kirkwood_picket",
        crate::minds::content::kirkwood_picket_digest().0,
    )
    .unwrap_or_else(MindSeal::placeholder);
    let pose = Pose::at(1_500, 1_500, 0, unit.rules().clone());
    let custody_unit = CustodyUnit {
        id: ObjId(1),
        team: PartyId::RED,
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
        controller: Controller::Mind,
        slot: 0,
        seal,
        unit,
    };
    let mut host = FieldHost::all_hostile();
    host.every_class = true;
    Arena::new(
        CustodyOut {
            definition,
            units: vec![custody_unit],
            pilots: Vec::new(),
            resolution: crate::match_::stock_resolution(),
            seed: 2_350,
            mode: MatchMode::Skirmish,
            host,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .ok()
}

static CACHED_SPACE_ARENA_DEF: std::sync::LazyLock<Option<Arc<ArenaDefinition>>> =
    std::sync::LazyLock::new(|| minimal_space_arena_definition().map(Arc::new));

static BASE_SEALED_SPACE_ARENA: std::sync::LazyLock<Option<crate::arena::state::Arena>> =
    std::sync::LazyLock::new(build_minimal_sealed_space_arena);

/// Install the unit's sealed scripted runner via [`crate::minds::sealed::install_sealed_runners`]
/// (resolving the Rhai source by seal digest from [`crate::store::TacsimStore`]) and execute
/// the think job on the process-wide [`crate::minds::MindPool`] (`SF.R22`, `D-35-1`, `D-35-17`).
#[allow(clippy::too_many_arguments)]
pub fn run_sealed_mind_on_pool(
    unit_obj: ObjId,
    seal: Option<&crate::arena::mind::MindSeal>,
    default_digest: [u8; 32],
    store: Option<&crate::store::TacsimStore>,
    think_seq: u64,
    t_ms: i64,
    view: Arc<MindView>,
    memory: &[u8],
) -> Option<ThinkResult> {
    use crate::arena::mind::{CollectedThink, SealEngine};

    let mut arena = BASE_SEALED_SPACE_ARENA.as_ref()?.clone();
    let digest = seal.map_or(default_digest, |s| s.source_sha256);
    if let Some(u) = arena.units.first_mut() {
        u.id = unit_obj;
        u.seal.engine = SealEngine::Scripted;
        u.seal.digest = digest;
    }
    let cfg = ArenaConfig::default();
    crate::minds::sealed::install_sealed_runners(&mut arena, &cfg, &|d| {
        crate::minds::content::source_lookup(store, d).map(str::to_owned)
    })
    .ok()?;
    let runner = arena.runners.remove(&unit_obj)?;

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
    let collected = CollectedThink {
        epochs: [0, 0, 0],
        unit: unit_obj,
        think_seq,
        t_ms,
        lands_ms: t_ms.saturating_add(500),
        seed: u64::from(unit_obj.0).wrapping_add(think_seq),
        dropped: false,
        profile: core_profile,
        view,
        memory: memory.to_vec(),
        map,
        wrecks: Vec::new(),
        card,
        program,
        profile_token,
        runner,
        substep_hash: None,
        ranges_before: BTreeMap::new(),
    };
    let mut answers = crate::minds::MindPool::shared(1)
        .submit(vec![collected])
        .wait()
        .ok()?;
    Some(answers.pop()?.result)
}

/// Run one think of the mind sealed on `ship_id` inside `bubble` (`D-35-1`, `D-35-11`,
/// `SF.R22`; doc 44 unit MW.9), on the one view every space mind reads
/// ([`super::view::space_mind_view`]): `book` is this observer's contact numbers and
/// `situation` what the host knows of the stops. Answers the think and the ship each contact
/// id of that view names, for [`super::applier::apply_intents`].
#[allow(clippy::too_many_arguments)]
pub fn run_space_rhai_think_with_seal(
    bubble: &LocalSpaceBubble,
    ship_id: &str,
    unit_obj: ObjId,
    seal: Option<&crate::arena::mind::MindSeal>,
    default_digest: [u8; 32],
    store: Option<&crate::store::TacsimStore>,
    think_seq: u64,
    memory: &[u8],
    book: &mut ContactBook,
    situation: &Situation,
) -> Option<(ThinkResult, BTreeMap<ContactId, String>)> {
    let (view, contact_map) =
        super::view::space_mind_view(bubble, ship_id, unit_obj, book, situation)?;
    let t_ms = view.t_ms;
    let result = run_sealed_mind_on_pool(
        unit_obj,
        seal,
        default_digest,
        store,
        think_seq,
        t_ms,
        Arc::new(view),
        memory,
    )?;
    Some((result, contact_map))
}

/// Run one Rhai [`crate::minds::MindPool`] think step for a collector drone sortie (`drone_mining.rhai`, `SF.R14`, `SF.R22`, `D-35-13`).
#[must_use]
pub fn run_drone_rhai_think(
    store: Option<&crate::store::TacsimStore>,
    drone_obj: ObjId,
    seal: Option<&crate::arena::mind::MindSeal>,
    pos_m: [i64; 3],
    think_seq: u64,
    memory: &[u8],
) -> Option<ThinkResult> {
    let t_ms = i64::try_from(think_seq.saturating_mul(1_000)).unwrap_or(0);
    let view = Arc::new(MindView {
        unit: drone_obj.0,
        callsign: Designator::Unit {
            party: CARRIER_PARTY,
            letter: 'A',
        },
        party: CARRIER_PARTY,
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
        t_ms,
        notices: Vec::<FrameNotice>::new(),
        contacts: Vec::new(),
        scan_tables: Vec::new(),
        odds_floor_permille: 0,
        space: None,
    });
    run_sealed_mind_on_pool(
        drone_obj,
        seal,
        crate::minds::content::drone_mining_digest().0,
        store,
        think_seq,
        t_ms,
        view,
        memory,
    )
}

/// **The stops the customs corvette has on the carriers of its bubble** (doc 44 unit MW.9):
/// what every mind in a picket bubble reads of customs, from the picket's own per-carrier
/// state. A hail stands only while the corvette is there to enforce it.
#[must_use]
pub fn picket_situation(
    bubble: &LocalSpaceBubble,
    picket: &KirkwoodPicketState,
    carriers: &[(ObjId, String, PartyId)],
    step_ms: u32,
) -> Situation {
    let corvette_stands = bubble
        .ships
        .get(&picket.corvette_ship_id)
        .is_some_and(|corvette| corvette.status == HullStatus::Operational);
    let mut situation = Situation::open(step_ms);
    situation.stopper = Some(picket.corvette_ship_id.clone());
    for (hull, ship_id, _) in carriers {
        let stop = match picket.inspection_status_for(*hull) {
            PicketInspectionStatus::AwaitingHeaveTo if corvette_stands => Stop::Hailed,
            PicketInspectionStatus::HoveToReadyForInspection if corvette_stands => Stop::HoveTo,
            PicketInspectionStatus::AwaitingHeaveTo
            | PicketInspectionStatus::HoveToReadyForInspection => Stop::None,
            PicketInspectionStatus::Cleared
            | PicketInspectionStatus::ClearedAfterFine
            | PicketInspectionStatus::ClearedAfterSeizure => Stop::Cleared,
            PicketInspectionStatus::RanThePicket => Stop::Ran,
        };
        if stop != Stop::None {
            situation.stops.insert(ship_id.clone(), stop);
        }
    }
    situation
}

/// One think of the corvette's mind, applied by the one applier.
fn step_corvette_mind_on_bubble(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    picket: &mut KirkwoodPicketState,
    situation: &Situation,
    store: Option<&crate::store::TacsimStore>,
    emitted_events: &mut Vec<super::bubble::SpaceCombatEvent>,
) {
    if bubble
        .ships
        .get(&picket.corvette_ship_id)
        .is_none_or(|corvette| corvette.status != HullStatus::Operational)
    {
        return;
    }
    // Doc 44 unit MW.10: she thinks with the mind she was sealed with when the bubble
    // opened, and with no other. With no seal she does not think.
    let Some(sealed) = picket.corvette_seal.as_ref().map(|seal| seal.source_sha256) else {
        return;
    };
    let Some((res, cmap)) = run_space_rhai_think_with_seal(
        bubble,
        &picket.corvette_ship_id,
        picket.corvette_hull,
        picket.corvette_seal.as_ref(),
        sealed,
        store,
        picket.corvette_think_seq,
        &picket.corvette_memory,
        &mut picket.corvette_contacts,
        situation,
    ) else {
        return;
    };
    if res.fault.is_some() {
        return;
    }
    picket.corvette_think_seq = picket.corvette_think_seq.saturating_add(1);
    picket.corvette_memory = res.memory;
    picket.corvette_last_trace = res.trace;
    let _ = super::applier::apply_intents(
        bubble,
        tape,
        &picket.corvette_ship_id,
        Seat::PATROL,
        &cmap,
        &res.intents,
        emitted_events,
    );
    picket.corvette_last_intents = res.intents;
}

/// **The picket's own rules, on state** (`SF.R18`; doc 44 unit MW.9): a carrier that is hailed
/// and comes under the heave-to speed against the corvette is hove to and ready; one that is
/// not cleared and passes the boundary has run, and the corvette's party holds hers hostile.
/// These are the host's, read off the bubble; no mind's memory is read or written for them.
fn settle_picket_stops(
    bubble: &mut LocalSpaceBubble,
    picket: &mut KirkwoodPicketState,
    carriers: &[(ObjId, String, PartyId)],
) {
    let Some((corvette_pos, corvette_vel)) = bubble
        .ships
        .get(&picket.corvette_ship_id)
        .filter(|corvette| corvette.status == HullStatus::Operational)
        .map(|corvette| (corvette.pos_m, corvette.vel_mmps))
    else {
        return;
    };
    for (chull, cship_id, cparty) in carriers {
        let Some(carrier) = bubble.ships.get(cship_id) else {
            continue;
        };
        if carrier.status != HullStatus::Operational {
            continue;
        }
        let (carrier_pos, carrier_vel) = (carrier.pos_m, carrier.vel_mmps);
        let cur_status = picket.inspection_status_for(*chull);
        if cur_status == PicketInspectionStatus::AwaitingHeaveTo
            && relative_speed_3d_mmps(carrier_vel, corvette_vel) < HEAVE_TO_REL_VEL_MMPS
        {
            picket
                .carrier_inspection_status
                .insert(*chull, PicketInspectionStatus::HoveToReadyForInspection);
            if picket.inspection_status == PicketInspectionStatus::AwaitingHeaveTo {
                picket.inspection_status = PicketInspectionStatus::HoveToReadyForInspection;
            }
        }
        if !picket.is_carrier_customs_cleared(*chull)
            && cur_status != PicketInspectionStatus::RanThePicket
            && distance_3d_m(carrier_pos, corvette_pos) > PICKET_RUN_BOUNDARY_M
        {
            picket.set_inspection_status_for(*chull, PicketInspectionStatus::RanThePicket);
            bubble.set_party_relation(CUSTOMS_CORVETTE_PARTY, *cparty, Some(Mark::Hostile));
        }
    }
}

/// One think of the raider's mind, applied by the one applier. She thinks while she lurks as
/// well: when to light her torch is her doctrine's to decide.
fn step_raider_mind_on_bubble(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    picket: &mut KirkwoodPicketState,
    situation: &Situation,
    store: Option<&crate::store::TacsimStore>,
    emitted_events: &mut Vec<super::bubble::SpaceCombatEvent>,
) {
    if bubble
        .ships
        .get(&picket.raider_ship_id)
        .is_none_or(|raider| raider.status != HullStatus::Operational)
    {
        return;
    }
    // Doc 44 unit MW.10: as the corvette, the mind she was sealed with and no other.
    let Some(sealed) = picket.raider_seal.as_ref().map(|seal| seal.source_sha256) else {
        return;
    };
    let Some((res, cmap)) = run_space_rhai_think_with_seal(
        bubble,
        &picket.raider_ship_id,
        picket.raider_hull,
        picket.raider_seal.as_ref(),
        sealed,
        store,
        picket.raider_think_seq,
        &picket.raider_memory,
        &mut picket.raider_contacts,
        situation,
    ) else {
        return;
    };
    if res.fault.is_some() {
        return;
    }
    picket.raider_think_seq = picket.raider_think_seq.saturating_add(1);
    picket.raider_memory = res.memory;
    picket.raider_last_trace = res.trace;
    let applied = super::applier::apply_intents(
        bubble,
        tape,
        &picket.raider_ship_id,
        Seat::RAIDER,
        &cmap,
        &res.intents,
        emitted_events,
    );
    if applied.fired_heavy {
        picket.raider_ambush_launched = true;
    }
    picket.raider_last_intents = res.intents;
}

/// The carrier a test names by its ship id alone, as the entry the picket steps by.
fn lone_carrier(bubble: &LocalSpaceBubble, carrier_ship_id: &str) -> Vec<(ObjId, String, PartyId)> {
    let hull = carrier_ship_id
        .strip_prefix("hull.")
        .and_then(|number| number.parse::<u32>().ok())
        .map_or(ObjId(0), ObjId);
    let party = bubble
        .ships
        .get(carrier_ship_id)
        .map_or(CARRIER_PARTY, |ship| ship.party);
    vec![(hull, carrier_ship_id.to_owned(), party)]
}

/// Step the Kirkwood Customs Picket's two minds for a bubble with the one carrier
/// `carrier_ship_id`, a second at a time and on the compiled doctrines.
pub fn step_kirkwood_picket(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    picket: &mut KirkwoodPicketState,
    carrier_ship_id: &str,
) -> Vec<super::bubble::SpaceCombatEvent> {
    let carriers = lone_carrier(bubble, carrier_ship_id);
    step_kirkwood_picket_for_carriers_with_store(bubble, tape, picket, &carriers, None, 1_000)
}

/// Step the Kirkwood Customs Picket's sealed minds across all participating carriers
/// (`SF.R18`, `SF.R22`; doc 44 unit MW.9): each mind thinks once on the one view and is applied
/// by the one applier, and the picket's own rules are settled on state between them. `store`
/// is where a seal's source is looked up; `step_ms` is what the bubble steps by.
pub fn step_kirkwood_picket_for_carriers_with_store(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    picket: &mut KirkwoodPicketState,
    carriers: &[(ObjId, String, PartyId)],
    store: Option<&crate::store::TacsimStore>,
    step_ms: u32,
) -> Vec<super::bubble::SpaceCombatEvent> {
    let mut emitted_events = Vec::new();
    bubble.set_inspecting(CUSTOMS_CORVETTE_PARTY, picket.is_screening());
    settle_picket_stops(bubble, picket, carriers);
    let situation = picket_situation(bubble, picket, carriers, step_ms);
    if picket.inspection_outcome.is_some() || !picket.carrier_inspection_outcome.is_empty() {
        step_raider_mind_on_bubble(bubble, tape, picket, &situation, store, &mut emitted_events);
        step_corvette_mind_on_bubble(bubble, tape, picket, &situation, store, &mut emitted_events);
    } else {
        step_corvette_mind_on_bubble(bubble, tape, picket, &situation, store, &mut emitted_events);
        step_raider_mind_on_bubble(bubble, tape, picket, &situation, store, &mut emitted_events);
    }
    bubble.set_inspecting(CUSTOMS_CORVETTE_PARTY, picket.is_screening());
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

/// The contraband fine for `unlicensed_strategic_ore_kg`, in minor credit units (`D-35-11`):
/// [`CUSTOMS_FINE_MAJOR_PER_TONNE`] a tonne begun, never under [`CUSTOMS_FINE_MIN_MAJOR`].
/// `None` when the amount overflows the credit scale.
#[must_use]
pub fn customs_fine_minor(w: &World, unlicensed_strategic_ore_kg: u64) -> Option<i64> {
    let tonnes = unlicensed_strategic_ore_kg.div_ceil(1_000).max(1);
    let fine_major = tonnes
        .saturating_mul(u64::try_from(CUSTOMS_FINE_MAJOR_PER_TONNE).unwrap_or(500))
        .max(u64::try_from(CUSTOMS_FINE_MIN_MAJOR).unwrap_or(1_000));
    minor_of(fine_major, credit_scale(w))
}

/// Whether `pilot` can pay a customs fine of `fine_minor` now, and the Registry can take it
/// (doc 35 cut SF.R26c): read by `+helm/submit` to choose between a fine and a seizure, and
/// again by the levy.
///
/// # Errors
///
/// The pilot holds no credit account or too little credit, or the Registry has no live
/// account.
pub fn customs_fine_payable(w: &World, pilot: ObjId, fine_minor: i64) -> Result<(), String> {
    let credit = crate::port::credit::own_account(w, pilot)
        .ok_or_else(|| "pilot holds no credit account for customs fine".to_owned())?;
    let available = credit.available().map_or(0, Amount::minor);
    if available < fine_minor {
        return Err(format!(
            "insufficient credit for customs fine (requires {fine_minor} minor units, available {available})"
        ));
    }
    resolve_registry_principal_and_account(w)
        .map(|_| ())
        .ok_or_else(|| "Freeport Registry account unavailable".to_owned())
}

/// Levy a customs fine of `fine_minor` as a real `mush-economy` Public E4 contract from
/// `pilot`'s credit account into `freeport_registry`'s account, and advance the
/// [`mush_faction::CUSTOMS_ASSERTION_TITLE`] campaign front (`D-35-11`, `SF.R6`).
///
/// Doc 35 cut SF.R26c: called by the space tick for a ruling that has landed
/// ([`super::customs`]), never by `+helm/submit` itself. The economy's contracts are not an
/// action-state family, so the ruling's action cannot write one.
///
/// # Errors
///
/// Returns an error string if the pilot lacks a credit account or sufficient funds, or if
/// contract activation fails. Nothing is left offered on an error.
pub fn levy_customs_fine_and_advance_front(
    ctx: &mut Ctx<'_>,
    pilot: ObjId,
    hull: ObjId,
    unlicensed_strategic_ore_kg: u64,
    fine_minor: i64,
) -> Result<(ContractId, Option<FrontId>), String> {
    customs_fine_payable(ctx.w, pilot, fine_minor)?;
    let pilot_account = crate::port::credit::own_account(ctx.w, pilot)
        .map(|credit| credit.id)
        .ok_or_else(|| "pilot holds no credit account for customs fine".to_owned())?;
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
    Ok((contract_id, front_id))
}

/// Record that unlicensed `strategic_ore` was seized from `hull` in lieu of a fine `pilot`
/// could not pay, and advance [`mush_faction::CUSTOMS_ASSERTION_TITLE`] (`SF.R12`, `35b` §5
/// #3).
///
/// Doc 35 cut SF.R26c: the ore itself leaves the hold in the ruling's own action
/// ([`super::customs`]); this is the incident, recorded by the space tick once that action
/// has landed. `seized_strategic_ore_kg` is what the action took.
pub fn record_customs_seizure(
    ctx: &mut Ctx<'_>,
    pilot: ObjId,
    hull: ObjId,
    seized_strategic_ore_kg: u64,
) -> Option<FrontId> {
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
                seized_strategic_ore_kg,
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
                "seized_strategic_ore_kg": seized_strategic_ore_kg,
            }),
        },
    )
    .ok();
    ctx.actor = saved_actor;
    ctx.out.pin_deferred();
    front_id
}

/// Record a picket-running incident (`RanThePicket`), escalating the Republic Customs Corvette
/// to [`Relation::Hostile`](crate::arena::party::Relation::Hostile) and advancing the [`mush_faction::CUSTOMS_ASSERTION_TITLE`]
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

/// Authored NPC cast entry from `content/space/picket_cast.json` (`SF.R22`, `D-35-17`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PicketCastEntry {
    pub commander: String,
    pub organization: Option<String>,
    pub hull_name: String,
    pub unit: String,
    pub overlay: String,
    /// The doctrine she flies, by name: her seal's `mind`.
    pub doctrine: String,
    /// That doctrine's digest: the SHA-256 of its source, 64 lowercase base-16 characters
    /// (doc 44 unit MW.10; Q-44-25). Her seal names the source the store holds under it.
    pub doctrine_sha256: String,
    pub pos_m: [i64; 3],
    pub vel_mmps: [i64; 3],
}

impl PicketCastEntry {
    /// [`Self::doctrine_sha256`] as bytes, when it is a digest.
    #[must_use]
    pub fn doctrine_digest(&self) -> Option<[u8; 32]> {
        crate::arena::digest_from_text(&self.doctrine_sha256)
    }
}

/// Authored NPC cast table for the Kirkwood Customs Picket and deep-space interdiction (`SF.R22`, `D-35-17`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PicketCastBook {
    pub corvette: PicketCastEntry,
    pub raider: PicketCastEntry,
    pub interdictor: PicketCastEntry,
}

const PICKET_CAST_JSON: &str = include_str!("../../content/space/picket_cast.json");

static PICKET_CAST: std::sync::LazyLock<Option<PicketCastBook>> = std::sync::LazyLock::new(|| {
    // `SF.R30` (doc 35d §6.6): a cast file that does not parse is said aloud. There is no
    // built-in cast behind it: every spawn asks for the book and opens nothing without one.
    let book: PicketCastBook = serde_json::from_str(PICKET_CAST_JSON)
        .map_err(|error| {
            tracing::error!(
                %error,
                "tacsim: content/space/picket_cast.json does not parse; no picket or interdictor sails"
            );
        })
        .ok()?;
    // Doc 44 unit MW.10: an actor is named with her doctrine's digest. One that is not a
    // digest is the same fault as a file that does not parse.
    for (actor, entry) in [
        ("corvette", &book.corvette),
        ("raider", &book.raider),
        ("interdictor", &book.interdictor),
    ] {
        if entry.doctrine_digest().is_none() {
            tracing::error!(
                actor,
                doctrine = %entry.doctrine,
                "tacsim: content/space/picket_cast.json names a doctrine_sha256 that is not 64 lowercase base-16 characters; no picket or interdictor sails"
            );
            return None;
        }
    }
    Some(book)
});

/// Return the authored [`PicketCastBook`] loaded from `content/space/picket_cast.json` (`SF.R22`, `D-35-17`).
#[must_use]
pub fn picket_cast_book() -> Option<&'static PicketCastBook> {
    PICKET_CAST.as_ref()
}

fn configure_corvette_and_raider_ships(corvette: &mut SpaceShipState, raider: &mut SpaceShipState) {
    let (corvette_overlay, raider_overlay) = if let Some(cast) = picket_cast_book() {
        (cast.corvette.overlay.as_str(), cast.raider.overlay.as_str())
    } else {
        ("tacsim.unit.corvette", "tacsim.unit.raider_heavy")
    };
    super::custody::apply_space_unit_overlay(
        corvette,
        &super::custody::space_unit_overlay_for_id(corvette_overlay),
    );
    corvette.attitude_deg = (180, 0, 0);
    corvette.pdc_auto = true;
    corvette.emcon = EmconPosture::ActiveCombat;

    super::custody::apply_space_unit_overlay(
        raider,
        &super::custody::space_unit_overlay_for_id(raider_overlay),
    );
    raider.attitude_deg = (180, 0, 0);
    raider.emcon = EmconPosture::Ghost;
    raider.heat_permille = 0;
}

/// Spawn real world `Thing` hulls and checked-out [`mush_tactics::TacticalUnit`]s for the
/// Republic Customs Corvette and Belt Raider from `content/space/picket_cast.json`, and insert
/// them alongside their 3-party relations into `bubble` (`SF.R12`, `SF.R20`, `SF.R22`, `D-35-1`, `D-35-11`, `D-35-17`).
pub fn spawn_kirkwood_picket_ships_in_world(
    ctx: &mut Ctx<'_>,
    bubble: &mut LocalSpaceBubble,
    domain: mush_sim::DomainId,
    leg_cursor: u32,
    corvette_doctrine: &crate::minds::content::cast::CastDoctrine,
    raider_doctrine: &crate::minds::content::cast::CastDoctrine,
) -> Option<KirkwoodPicketState> {
    // Doc 44 unit MW.10: the caller resolved both doctrines from the store before it checked
    // the carrier out ([`cast_doctrine_in_world`]); this seals with them and names no other.
    let cast = picket_cast_book()?;
    let site_id = format!("domain.{}", domain.0);
    let corvette_ship_id = format!("customs.{}.{}", domain.0, leg_cursor);
    let raider_ship_id = format!("raider.{}.{}", domain.0, leg_cursor);

    let corvette_commander = super::tick::reuse_or_create_npc_commander(
        ctx,
        &cast.corvette.commander,
        cast.corvette.organization.as_deref(),
    )
    .unwrap_or(mush_core::GOD);
    let corvette_hull =
        super::tick::reuse_or_create_npc_hull(ctx, &cast.corvette.hull_name, corvette_commander)?;
    super::custody::create_stock_space_unit_in_world(ctx, corvette_hull, &cast.corvette.unit)
        .ok()?;
    super::custody::ensure_space_hull_titled(ctx, corvette_hull, corvette_commander);
    let mut corvette = super::custody::check_out_space_unit(
        ctx,
        corvette_hull,
        &site_id,
        leg_cursor,
        &corvette_ship_id,
        &cast.corvette.hull_name,
        CUSTOMS_CORVETTE_PARTY,
        cast.corvette.pos_m,
        cast.corvette.vel_mmps,
    )
    .ok()?;

    let leader = super::tick::reuse_or_create_npc_commander(
        ctx,
        &cast.raider.commander,
        cast.raider.organization.as_deref(),
    )?;
    let raider_hull = super::tick::reuse_or_create_npc_hull(ctx, &cast.raider.hull_name, leader)?;
    super::custody::create_stock_space_unit_in_world(ctx, raider_hull, &cast.raider.unit).ok()?;
    super::custody::ensure_space_hull_titled(ctx, raider_hull, leader);
    let mut raider = super::custody::check_out_space_unit(
        ctx,
        raider_hull,
        &site_id,
        leg_cursor,
        &raider_ship_id,
        &cast.raider.hull_name,
        BELT_RAIDER_PARTY,
        cast.raider.pos_m,
        cast.raider.vel_mmps,
    )
    .ok()?;

    configure_corvette_and_raider_ships(&mut corvette, &mut raider);

    bubble.set_party_relation(CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY, None);
    bubble.set_inspecting(CUSTOMS_CORVETTE_PARTY, true);
    bubble.set_party_relation(CARRIER_PARTY, BELT_RAIDER_PARTY, Some(Mark::Hostile));
    bubble.set_party_relation(
        CUSTOMS_CORVETTE_PARTY,
        BELT_RAIDER_PARTY,
        Some(Mark::Hostile),
    );

    bubble.insert_ship(corvette);
    bubble.insert_ship(raider);

    let now_ic = bubble.ic_timestamp_s.max(1);
    let corvette_seal = seal_space_unit_in_world(
        ctx,
        corvette_hull,
        crate::arena::mind::SortieId::ic(corvette_hull, now_ic),
        corvette_doctrine.name.as_str(),
        corvette_doctrine.digest,
    );
    let raider_seal = seal_space_unit_in_world(
        ctx,
        raider_hull,
        crate::arena::mind::SortieId::ic(raider_hull, now_ic),
        raider_doctrine.name.as_str(),
        raider_doctrine.digest,
    );

    // The record holds the seals the world took, and no other: one that was not built
    // leaves its actor without a mind for the life of the bubble.
    let mut state =
        KirkwoodPicketState::new(corvette_hull, corvette_ship_id, raider_hull, raider_ship_id);
    state.corvette_seal = corvette_seal;
    state.raider_seal = raider_seal;
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
    let cast = picket_cast_book()?;
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
        &cast.corvette.hull_name,
        CUSTOMS_CORVETTE_PARTY,
        cast.corvette.pos_m,
        cast.corvette.vel_mmps,
    );
    let mut raider = SpaceShipState::new_corvette(
        &raider_ship_id,
        &cast.raider.hull_name,
        BELT_RAIDER_PARTY,
        cast.raider.pos_m,
        cast.raider.vel_mmps,
    );
    configure_corvette_and_raider_ships(&mut corvette, &mut raider);

    bubble.set_party_relation(CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY, None);
    bubble.set_inspecting(CUSTOMS_CORVETTE_PARTY, true);
    bubble.set_party_relation(CARRIER_PARTY, BELT_RAIDER_PARTY, Some(Mark::Hostile));
    bubble.set_party_relation(
        CUSTOMS_CORVETTE_PARTY,
        BELT_RAIDER_PARTY,
        Some(Mark::Hostile),
    );

    bubble.insert_ship(corvette);
    bubble.insert_ship(raider);

    Some(
        KirkwoodPicketState::new(corvette_hull, corvette_ship_id, raider_hull, raider_ship_id)
            .sealed_by_pack(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::party::Relation;
    use crate::space::bubble::BubbleInputCommand;

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

        // Step 1: the carrier is closing at 15 m/s against the corvette. The corvette's mind
        // hails her; the raider's mind thinks too, and lurks dark.
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
        assert_eq!(picket.raider_think_seq, 1);
        assert!(picket.raider_last_trace.is_empty());
        assert_eq!(
            bubble.ships.get(&picket.raider_ship_id).map(|s| s.emcon),
            Some(EmconPosture::Ghost)
        );
        assert!(!picket.raider_ambush_launched);

        // Step 2: the carrier slows to 4 m/s against the corvette. The picket's host reads her
        // hove to, on her state, and the corvette's mind reads that word from its view.
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

        // Step 3: customs fine her and clear her. The raider hears the clearance on the
        // picket's radio and lights her torch; the corvette, which now holds her, screens.
        picket.set_inspection_status_for(ObjId(10), PicketInspectionStatus::ClearedAfterFine);
        picket.set_inspection_outcome_for(
            ObjId(10),
            PicketInspectionOutcome::Fined {
                contract_id: 42,
                fine_minor: 200_000,
                unlicensed_strategic_ore_kg: 4_000,
                front_id: Some(1),
            },
        );
        bubble.set_party_relation(CUSTOMS_CORVETTE_PARTY, CARRIER_PARTY, Some(Mark::Friendly));
        let memory_before = picket.corvette_memory.clone();

        let _ = step_kirkwood_picket(&mut bubble, &mut tape, &mut picket, "hull.10");
        assert_eq!(
            picket.raider_last_trace,
            vec!["RAIDER_LIGHT_TORCH".to_owned()]
        );
        assert_eq!(
            bubble.ships.get(&picket.raider_ship_id).map(|s| s.emcon),
            Some(EmconPosture::ActiveCombat)
        );
        assert!(!picket.raider_ambush_launched);
        assert_eq!(
            picket.corvette_last_trace,
            vec!["PICKET_SCREEN_ENGAGE_RAIDER".to_owned()]
        );
        // The clearance reached the corvette's mind through its view: its memory is what its
        // own last think left, and nothing else has written it.
        assert_eq!(picket.corvette_memory, memory_before);

        // Step 4: with her torch lit the raider holds the carrier, and opens: two torpedoes
        // and a railgun round, at the lock her mind set, on the burn her mind asked.
        let ambush_events = step_kirkwood_picket(&mut bubble, &mut tape, &mut picket, "hull.10");
        assert!(picket.raider_ambush_launched);
        assert_eq!(
            picket.raider_last_trace,
            vec!["RAIDER_TORCH_AND_SALVO".to_owned()]
        );
        assert_eq!(
            bubble
                .ships
                .get(&picket.raider_ship_id)
                .and_then(|s| s.locked_target_id.clone())
                .as_deref(),
            Some("hull.10")
        );
        assert!(
            bubble
                .ships
                .get(&picket.raider_ship_id)
                .is_some_and(SpaceShipState::is_burning)
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
            Relation::Hostile
        );
    }
}
