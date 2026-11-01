//! **Zero-G Asteroid Field Mining & Freeport Registry Integration**
//! (Doc 35 cut `SF.4`; `docs/design/35-ship-free-flight-combat-and-mining.md` §7,
//! `Q-35-3`, `D-35-13`).
//!
//! Unifies [doc 20](../../../docs/design/20-surface-mining.md)'s canonical ore economy
//! and Freeport Registry tenure rules with 3D velocity-matched local space bubbles:
//! - **Canonical Ore Goods ([`OreGoodId`])**: `metal_ore`, `ice_ore`, `carbon_ore`, and
//!   `strategic_ore`, mapped onto [`OreFamily`] and richness [`Word`] (`barren`, `lean`,
//!   `fair`, `rich`).
//! - **`stow.ore` Hold Enforcement ([`OreHoldState`])**: Raw mined ore goes strictly into
//!   `stow.ore` (measured in `kg` and `L`), requiring an ore-capable hull (`Dray-P` or
//!   drone/TV hoppers) and refusing plain `stow.bulk` haulers (`D-20-9`).
//! - **Freeport Registry Licences, Claims & Contests**:
//!   - Prospecting licence at **`2,500 cr` per `168 IC days`** ([`PROSPECTING_LICENCE_FEE_CR`],
//!     [`CLAIM_TERM_IC_SECS`]).
//!   - Hand-made ping/sounding evidence qualifies a claim stake with a **48-hour contest
//!     window** ([`ASTEROID_CONTEST_WINDOW_IC_SECS`]).
//!   - Contested **Pallas Syndicate paper claims** in `pallas_inclined_belt`: mining without
//!     Freeport paper works in frontier space (`WildcatExtraction`) but spikes thermal/IR
//!     bloom and records an actionable claim-jump event.
//! - **3D Velocity-Match Gate (`+helm/match`)**: Extraction requires distance
//!   $\le 2{,}000\text{ m}$ ([`MAX_MINING_RANGE_M`]) and relative velocity
//!   $|\Delta \mathbf{v}| \le 5\text{ m/s}$ ([`MAX_MINING_REL_VEL_MPS`]).
//! - **Ship Mining Laser vs. 4-Drone Sorties (`doc 20 §17`)**: Direct mining laser
//!   extraction (high IR bloom + volatile outgassing $\Delta \mathbf{v}$ kick if overheated)
//!   plus up to **4 collector drones per `Dray-P`** ([`MAX_DRONES_PER_DRAY_P`]) sealed under
//!   an [`ExcursionId`] running `scan() -> sound() -> goto() -> mine() -> recover()`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::space::bubble::{LocalSpaceBubble, SpaceShipState};
use crate::space::weapons::isqrt_i128;
use crate::surface::geology::{OreFamily, Word};
use crate::surface::tenure::CLAIM_TERM_IC_SECS;

/// Freeport Registry prospecting licence fee in credits (`2,500 cr` per `168 IC days`).
pub const PROSPECTING_LICENCE_FEE_CR: u64 = 2_500;

/// 48-hour contest window for newly staked asteroid claims or non-owner salvage (`48 h`).
pub const ASTEROID_CONTEST_WINDOW_IC_SECS: i64 = 48 * 3_600;

/// Maximum distance in metres from a ship to an asteroid pocket to mine or launch drones (`2,000 m`).
pub const MAX_MINING_RANGE_M: i64 = 2_000;

/// Maximum relative speed in $\text{m/s}$ between ship and asteroid pocket to extract ore (`5 m/s`).
pub const MAX_MINING_REL_VEL_MPS: i64 = 5;

/// Maximum collector drones a `Dray-P` prospector can carry and deploy (`doc 20 §17`).
pub const MAX_DRONES_PER_DRAY_P: usize = 4;

/// Canonical raw ore good identifiers in `stow.ore` (`doc 20 §7.3`, `Q-35-3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OreGoodId {
    /// Raw metallic ore (`"metal_ore"`, milled into `metals`).
    MetalOre,
    /// Raw volatile water-ice ore (`"ice_ore"`, milled into `volatiles`).
    IceOre,
    /// Raw carbonaceous ore (`"carbon_ore"`, milled into `carbon_feedstock`).
    CarbonOre,
    /// Raw strategic heavy-element ore (`"strategic_ore"`, milled into `strategic_elements`).
    StrategicOre,
}

impl OreGoodId {
    /// Canonical good ID string in `goods.json` and `stow.ore`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MetalOre => "metal_ore",
            Self::IceOre => "ice_ore",
            Self::CarbonOre => "carbon_ore",
            Self::StrategicOre => "strategic_ore",
        }
    }

    /// Corresponding [`OreFamily`] from `doc 20`.
    #[must_use]
    pub const fn family(self) -> OreFamily {
        match self {
            Self::MetalOre => OreFamily::Metals,
            Self::IceOre => OreFamily::Volatiles,
            Self::CarbonOre => OreFamily::CarbonFeedstock,
            Self::StrategicOre => OreFamily::StrategicElements,
        }
    }

    /// Specific volume in litres per kilogram (scaled $\times 1000$: millitres per kg)
    /// so `stow.ore` enforces both `si.mass_kg` and `si.volume_l` (`R7`, `dray.json`).
    /// Ice runs out of volume first (`1,100 mL/kg`); dense metal/strategic ore runs out of mass first (`350 mL/kg`).
    #[must_use]
    pub const fn millilitres_per_kg(self) -> u64 {
        match self {
            Self::IceOre => 1_100,
            Self::CarbonOre => 750,
            Self::MetalOre => 400,
            Self::StrategicOre => 350,
        }
    }
}

/// Spectral classification of an orbital asteroid pocket (`§7.2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsteroidSpectralClass {
    /// Carbonaceous chondrite: yields `ice_ore` and `carbon_ore`.
    CType,
    /// Stony-metallic: yields `metal_ore`.
    SType,
    /// Nickel-iron / platinum-group core: yields rich `metal_ore` and `strategic_ore`.
    MType,
}

impl AsteroidSpectralClass {
    #[must_use]
    pub const fn code_str(self) -> &'static str {
        match self {
            Self::CType => "C-TYPE",
            Self::SType => "S-TYPE",
            Self::MType => "M-TYPE",
        }
    }

    /// Primary and secondary [`OreGoodId`] families yielded by this spectral class.
    #[must_use]
    pub const fn ore_goods(self) -> &'static [OreGoodId] {
        match self {
            Self::CType => &[OreGoodId::IceOre, OreGoodId::CarbonOre],
            Self::SType => &[OreGoodId::MetalOre],
            Self::MType => &[OreGoodId::StrategicOre, OreGoodId::MetalOre],
        }
    }
}

/// Security / jurisdiction zone of an orbital asteroid cluster (`§7.1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterJurisdiction {
    /// Core Freeport Registry commons (`ceres_halo`): requires an active licence to extract.
    CoreCommons,
    /// Frontier patrolled scatter (`kirkwood_gap_3_1`).
    FrontierPatrolled,
    /// Contested frontier belt (`pallas_inclined_belt`) with Pallas Syndicate paper claims.
    FrontierSyndicateContested,
    /// Deep-space outer-system expedition ring (`saturn_b_ring_edge`).
    DeepSpaceExpedition,
}

/// A Freeport Registry prospecting licence (`2,500 cr` per `168 IC days`, `§7.3.1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProspectingLicence {
    pub holder_id: String,
    pub issued_at_ic_s: i64,
    pub expires_at_ic_s: i64,
    pub fee_paid_cr: u64,
}

impl ProspectingLicence {
    /// Issue a standard 168-IC-day Freeport Registry prospecting licence.
    #[must_use]
    pub fn issue(holder_id: impl Into<String>, now_ic_s: i64) -> Self {
        Self {
            holder_id: holder_id.into(),
            issued_at_ic_s: now_ic_s,
            expires_at_ic_s: now_ic_s.saturating_add(CLAIM_TERM_IC_SECS),
            fee_paid_cr: PROSPECTING_LICENCE_FEE_CR,
        }
    }

    /// Whether the licence is active at `now_ic_s`.
    #[must_use]
    pub fn is_active(&self, now_ic_s: i64) -> bool {
        now_ic_s >= self.issued_at_ic_s && now_ic_s < self.expires_at_ic_s
    }
}

/// Claim tenure state of an asteroid pocket in the Freeport Registry (`§7.3.1`, `D-35-13`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsteroidClaimState {
    /// Open, unstaked rock.
    #[default]
    Unclaimed,
    /// Staked at the Freeport Registry by `holder_id` with a 48 h contest window.
    FreeportStaked {
        holder_id: String,
        evidence_ping_ic_s: i64,
        staked_at_ic_s: i64,
        expires_at_ic_s: i64,
        contest_window_ends_ic_s: i64,
        contested_by: Option<String>,
        worked_kg: u64,
    },
    /// Covered by a Pallas Syndicate outer-belt paper claim (`pallas_inclined_belt`).
    PallasSyndicatePaper {
        paper_id: String,
        contested_by_freeport_holder: Option<String>,
    },
}

/// One ore deposit pool inside an [`AsteroidPocket`] (`doc 20 §3.4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsteroidOrePool {
    pub good: OreGoodId,
    pub richness: Word,
    pub remaining_kg: u64,
}

/// A 3D drifting asteroid pocket inside an orbital cluster bubble (`§7.2`, `D-35-13`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsteroidPocket {
    pub id: String,
    pub label: String,
    pub spectral_class: AsteroidSpectralClass,
    /// 3D local position in metres inside the cluster bubble.
    pub pos_m: [i64; 3],
    /// 3D drift velocity in $\text{mm/s}$.
    pub vel_mmps: [i64; 3],
    /// Effective radius in metres.
    pub radius_m: i64,
    pub pools: Vec<AsteroidOrePool>,
    pub claim: AsteroidClaimState,
    /// Earliest hand-made ping/sounding timestamp per pilot/ship ID (`evidence_at` rule).
    pub hand_pings_by_actor: BTreeMap<String, i64>,
}

impl AsteroidPocket {
    /// Total remaining ore across all pools in `kg`.
    #[must_use]
    pub fn total_remaining_kg(&self) -> u64 {
        self.pools
            .iter()
            .fold(0u64, |acc, p| acc.saturating_add(p.remaining_kg))
    }
}

/// `stow.ore` hold stowage state for a ship (`D-20-9`, `Q-35-3`).
///
/// Only ore-capable hulls (`Dray-P` prospector or collector drone hoppers) set
/// `accepts_stow_ore = true`; a plain `Dray` bulk hauler only accepts `stow.bulk`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OreHoldState {
    pub accepts_stow_ore: bool,
    pub max_mass_kg: u64,
    pub max_volume_l: u64,
    pub reserved_mass_kg: u64,
    pub lots_kg: BTreeMap<OreGoodId, u64>,
}

impl OreHoldState {
    /// Standard `Dray-P` prospector `stow.ore` hold (`100,000 kg` and `120,000 L`, `dray.json`).
    #[must_use]
    pub fn new_dray_p() -> Self {
        Self {
            accepts_stow_ore: true,
            max_mass_kg: 100_000,
            max_volume_l: 120_000,
            reserved_mass_kg: 0,
            lots_kg: BTreeMap::new(),
        }
    }

    /// Plain `Dray` bulk hauler (`accepts: ["stow.bulk"]` only — cannot stow raw ore!).
    #[must_use]
    pub fn new_plain_dray_bulk_only() -> Self {
        Self {
            accepts_stow_ore: false,
            max_mass_kg: 100_000,
            max_volume_l: 120_000,
            reserved_mass_kg: 0,
            lots_kg: BTreeMap::new(),
        }
    }

    /// Total mass of ore currently stowed in `kg`.
    #[must_use]
    pub fn used_mass_kg(&self) -> u64 {
        self.lots_kg
            .values()
            .fold(0u64, |acc, &m| acc.saturating_add(m))
    }

    /// Total volume of ore currently stowed in whole litres (`L`).
    #[must_use]
    pub fn used_volume_l(&self) -> u64 {
        self.lots_kg.iter().fold(0u64, |acc, (&good, &kg)| {
            let litres = (kg.saturating_mul(good.millilitres_per_kg())) / 1_000;
            acc.saturating_add(litres)
        })
    }

    /// Unreserved mass capacity remaining in `kg` (`hold_unreserved`, `D-20-9`).
    #[must_use]
    pub fn unreserved_mass_kg(&self) -> u64 {
        self.max_mass_kg
            .saturating_sub(self.used_mass_kg())
            .saturating_sub(self.reserved_mass_kg)
    }

    /// Maximum kilograms of `good` that can fit into the remaining unreserved mass AND volume limits.
    #[must_use]
    pub fn fitting_kg(&self, good: OreGoodId, requested_kg: u64) -> u64 {
        if !self.accepts_stow_ore {
            return 0;
        }
        let avail_mass = self.unreserved_mass_kg();
        let avail_vol_l = self.max_volume_l.saturating_sub(self.used_volume_l());
        let ml_per_kg = good.millilitres_per_kg().max(1);
        let max_by_vol_kg = (avail_vol_l.saturating_mul(1_000)) / ml_per_kg;
        requested_kg.min(avail_mass).min(max_by_vol_kg)
    }

    /// Load up to `requested_kg` of `good` into `stow.ore`, returning the exact `kg` loaded.
    pub fn load_ore(&mut self, good: OreGoodId, requested_kg: u64) -> u64 {
        let fit = self.fitting_kg(good, requested_kg);
        if fit > 0 {
            let entry = self.lots_kg.entry(good).or_insert(0);
            *entry = entry.saturating_add(fit);
        }
        fit
    }
}

/// Sealed excursion identifier for a `Dray-P`'s collector drone sortie (`doc 20 §17`, `D-35-13`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ExcursionId(pub u64);

/// Stock `mining` doctrine step for a collector drone (`doc 20 §17`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DroneDoctrinePhase {
    Scan,
    Sound,
    Goto,
    Mine,
    Recover,
    DockedInBay,
}

/// One collector drone in a `Dray-P`'s 4-drone bay (`maximum_items = 4`, `doc 20 §17`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectorDrone {
    pub bay_slot: u8,
    pub excursion_id: ExcursionId,
    pub target_rock_id: String,
    pub phase: DroneDoctrinePhase,
    pub hopper_good: Option<OreGoodId>,
    pub hopper_kg: u64,
    pub hopper_capacity_kg: u64,
}

/// Refusal reasons for zero-G asteroid mining and Freeport Registry operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OreMiningError {
    ShipNotFound,
    RockNotFound,
    /// Plain `Dray` holds accept `stow.bulk` only; raw ore requires `stow.ore` (`Dray-P`).
    HullLacksStowOre,
    /// Ship is beyond `MAX_MINING_RANGE_M` ($2{,}000\text{ m}$) from the target rock.
    OutOfRange {
        distance_m: i64,
        max_range_m: i64,
    },
    /// Ship has not matched velocity within `MAX_MINING_REL_VEL_MPS` ($5\text{ m/s}$).
    VelocityMismatch {
        relative_speed_mps: i64,
        max_speed_mps: i64,
    },
    /// Core zone (`ceres_halo`) requires an active Freeport Registry prospecting licence.
    UnlicensedInCoreZone,
    /// Staking a claim requires avalid licence and a prior hand-made ping/sounding on the rock.
    MissingHandMadePingEvidence,
    /// Rock is already staked by another Freeport holder (`contest` required within 48 h).
    ClaimHeldByOther {
        holder_id: String,
    },
    /// Contest window (48 h) has already closed or challenger lacks prior hand-made evidence.
    ContestRefused,
    /// `Dray-P` drone bays are capped at `MAX_DRONES_PER_DRAY_P` (4 drones).
    DroneBayCapExceeded {
        requested: usize,
        max_drones: usize,
    },
    /// `stow.ore` hold is full on mass or volume, or the rock's deposit is depleted.
    HoldFullOrDepleted,
}

impl std::fmt::Display for OreMiningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for OreMiningError {}

/// Telemetry & legal outcome of a mining laser cycle or drone recovery (`§7.3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionReport {
    pub rock_id: String,
    pub good: OreGoodId,
    pub extracted_kg: u64,
    pub hold_used_kg: u64,
    pub hold_used_l: u64,
    /// Thermal/IR signature bloom (`0..=1000`) emitted across the cluster bubble.
    pub ir_bloom_permille: u16,
    /// True if mining a Syndicate-papered or un-papered rock without Freeport title in frontier space.
    pub claim_jump_flagged: bool,
    /// Non-zero if overheating on a C-Type ice rock triggered a volatile outgassing $\Delta \mathbf{v}$ kick!
    pub outgassing_dv_kick_mps: [i64; 3],
}

/// An orbital asteroid cluster site wrapping a [`LocalSpaceBubble`], drifting [`AsteroidPocket`]s,
/// `stow.ore` ship holds, Freeport Registry licences/claims, and 4-drone sorties (`SF.4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsteroidClusterField {
    pub cluster_id: String,
    pub jurisdiction: ClusterJurisdiction,
    pub bubble: LocalSpaceBubble,
    pub rocks: BTreeMap<String, AsteroidPocket>,
    pub holds: BTreeMap<String, OreHoldState>,
    pub licences: BTreeMap<String, ProspectingLicence>,
    pub active_drones: BTreeMap<String, Vec<CollectorDrone>>,
    next_excursion_id: u64,
}

impl AsteroidClusterField {
    /// Seed one of the four canonical orbital asteroid clusters (`ceres_halo`,
    /// `kirkwood_gap_3_1`, `pallas_inclined_belt`, or `saturn_b_ring_edge`).
    #[must_use]
    pub fn new_canonical_cluster(cluster_id: &str, ic_timestamp_s: i64) -> Self {
        let (jurisdiction, title) = match cluster_id {
            "ceres_halo" => (ClusterJurisdiction::CoreCommons, "CERES CO-ORBITAL HALO"),
            "kirkwood_gap_3_1" => (
                ClusterJurisdiction::FrontierPatrolled,
                "KIRKWOOD 3:1 RESONANCE SCATTER",
            ),
            "pallas_inclined_belt" => (
                ClusterJurisdiction::FrontierSyndicateContested,
                "PALLAS INCLINED CLUSTER",
            ),
            _ => (
                ClusterJurisdiction::DeepSpaceExpedition,
                "SATURN B-RING SHEPHERD EDGE",
            ),
        };

        let bubble = LocalSpaceBubble::new(
            format!("bubble.{cluster_id}"),
            cluster_id,
            title,
            ic_timestamp_s,
        );

        let mut rocks = BTreeMap::new();
        // 1. C-Type carbonaceous ice/carbon pocket
        rocks.insert(
            "rock.c_101".to_owned(),
            AsteroidPocket {
                id: "rock.c_101".to_owned(),
                label: "C-101 CHONDRITE".to_owned(),
                spectral_class: AsteroidSpectralClass::CType,
                pos_m: [1_200, -400, 150],
                vel_mmps: [2_000, -1_000, 500],
                radius_m: 85,
                pools: vec![
                    AsteroidOrePool {
                        good: OreGoodId::IceOre,
                        richness: Word::Rich,
                        remaining_kg: 45_000,
                    },
                    AsteroidOrePool {
                        good: OreGoodId::CarbonOre,
                        richness: Word::Fair,
                        remaining_kg: 28_000,
                    },
                ],
                claim: AsteroidClaimState::Unclaimed,
                hand_pings_by_actor: BTreeMap::new(),
            },
        );

        // 2. S-Type stony-metallic pocket
        rocks.insert(
            "rock.s_204".to_owned(),
            AsteroidPocket {
                id: "rock.s_204".to_owned(),
                label: "S-204 STONY-IRON".to_owned(),
                spectral_class: AsteroidSpectralClass::SType,
                pos_m: [-3_500, 2_100, -600],
                vel_mmps: [-3_000, 1_500, 0],
                radius_m: 110,
                pools: vec![AsteroidOrePool {
                    good: OreGoodId::MetalOre,
                    richness: Word::Fair,
                    remaining_kg: 60_000,
                }],
                claim: AsteroidClaimState::Unclaimed,
                hand_pings_by_actor: BTreeMap::new(),
            },
        );

        // 3. M-Type heavy platinum/strategic core pocket (carries Pallas Syndicate paper in Pallas!)
        let m_claim = if jurisdiction == ClusterJurisdiction::FrontierSyndicateContested {
            AsteroidClaimState::PallasSyndicatePaper {
                paper_id: "SYNDICATE-PALLAS-FOLIO-88".to_owned(),
                contested_by_freeport_holder: None,
            }
        } else {
            AsteroidClaimState::Unclaimed
        };
        rocks.insert(
            "rock.m_309".to_owned(),
            AsteroidPocket {
                id: "rock.m_309".to_owned(),
                label: "M-309 NICKEL-PLATINUM CORE".to_owned(),
                spectral_class: AsteroidSpectralClass::MType,
                pos_m: [4_800, 3_200, 1_400],
                vel_mmps: [4_000, -2_500, 1_000],
                radius_m: 95,
                pools: vec![
                    AsteroidOrePool {
                        good: OreGoodId::StrategicOre,
                        richness: Word::Rich,
                        remaining_kg: 35_000,
                    },
                    AsteroidOrePool {
                        good: OreGoodId::MetalOre,
                        richness: Word::Rich,
                        remaining_kg: 50_000,
                    },
                ],
                claim: m_claim,
                hand_pings_by_actor: BTreeMap::new(),
            },
        );

        Self {
            cluster_id: cluster_id.to_owned(),
            jurisdiction,
            bubble,
            rocks,
            holds: BTreeMap::new(),
            licences: BTreeMap::new(),
            active_drones: BTreeMap::new(),
            next_excursion_id: 1,
        }
    }

    /// Admit a ship and its `stow.ore` hold configuration into the asteroid cluster bubble.
    pub fn insert_ship_with_hold(&mut self, ship: SpaceShipState, hold: OreHoldState) {
        let id = ship.id.clone();
        self.bubble.insert_ship(ship);
        self.holds.insert(id, hold);
    }

    /// Purchase / register a Freeport Registry prospecting licence (`2,500 cr` / `168 IC days`).
    pub fn issue_prospecting_licence(&mut self, ship_id: &str) -> ProspectingLicence {
        let lic = ProspectingLicence::issue(ship_id, self.bubble.ic_timestamp_s);
        self.licences.insert(ship_id.to_owned(), lic.clone());
        lic
    }

    /// Ping / scan an asteroid pocket (`+mine/ping <rock_id>`, free, records hand-made evidence).
    pub fn ping_rock(
        &mut self,
        ship_id: &str,
        rock_id: &str,
    ) -> Result<(AsteroidSpectralClass, Vec<(OreGoodId, Word)>), OreMiningError> {
        if !self.bubble.ships.contains_key(ship_id) {
            return Err(OreMiningError::ShipNotFound);
        }
        let now_ic_s = self.bubble.ic_timestamp_s;
        let rock = self
            .rocks
            .get_mut(rock_id)
            .ok_or(OreMiningError::RockNotFound)?;
        rock.hand_pings_by_actor
            .entry(ship_id.to_owned())
            .or_insert(now_ic_s);
        let summary = rock
            .pools
            .iter()
            .map(|p| (p.good, p.richness))
            .collect::<Vec<_>>();
        Ok((rock.spectral_class, summary))
    }

    /// Stake a Freeport Registry claim on `rock_id` (`+claim/stake <rock_id>`, `§7.3.1`).
    ///
    /// Requires an active [`ProspectingLicence`] and a prior hand-made ping (`ping_rock`).
    /// Opens a **48-hour contest window** ([`ASTEROID_CONTEST_WINDOW_IC_SECS`]).
    pub fn stake_claim(&mut self, ship_id: &str, rock_id: &str) -> Result<(), OreMiningError> {
        let now = self.bubble.ic_timestamp_s;
        let has_licence = self.licences.get(ship_id).is_some_and(|l| l.is_active(now));
        if !has_licence {
            return Err(OreMiningError::UnlicensedInCoreZone);
        }
        let rock = self
            .rocks
            .get_mut(rock_id)
            .ok_or(OreMiningError::RockNotFound)?;
        let Some(&ping_at) = rock.hand_pings_by_actor.get(ship_id) else {
            return Err(OreMiningError::MissingHandMadePingEvidence);
        };

        match &mut rock.claim {
            AsteroidClaimState::Unclaimed => {
                rock.claim = AsteroidClaimState::FreeportStaked {
                    holder_id: ship_id.to_owned(),
                    evidence_ping_ic_s: ping_at,
                    staked_at_ic_s: now,
                    expires_at_ic_s: now.saturating_add(CLAIM_TERM_IC_SECS),
                    contest_window_ends_ic_s: now.saturating_add(ASTEROID_CONTEST_WINDOW_IC_SECS),
                    contested_by: None,
                    worked_kg: 0,
                };
                Ok(())
            }
            AsteroidClaimState::PallasSyndicatePaper {
                contested_by_freeport_holder,
                ..
            } => {
                *contested_by_freeport_holder = Some(ship_id.to_owned());
                Ok(())
            }
            AsteroidClaimState::FreeportStaked { holder_id, .. } => {
                if holder_id == ship_id {
                    Ok(())
                } else {
                    Err(OreMiningError::ClaimHeldByOther {
                        holder_id: holder_id.clone(),
                    })
                }
            }
        }
    }

    /// Contest a Freeport claim within the **48-hour contest window** (`§7.3.1`, `doc 20 §9.4`)
    /// using an earlier hand-made ping timestamp.
    pub fn contest_claim(
        &mut self,
        challenger_id: &str,
        rock_id: &str,
    ) -> Result<(), OreMiningError> {
        let now = self.bubble.ic_timestamp_s;
        let rock = self
            .rocks
            .get_mut(rock_id)
            .ok_or(OreMiningError::RockNotFound)?;
        let Some(&challenger_ping) = rock.hand_pings_by_actor.get(challenger_id) else {
            return Err(OreMiningError::MissingHandMadePingEvidence);
        };

        match &mut rock.claim {
            AsteroidClaimState::FreeportStaked {
                evidence_ping_ic_s,
                contest_window_ends_ic_s,
                contested_by,
                ..
            } => {
                if now <= *contest_window_ends_ic_s && challenger_ping <= *evidence_ping_ic_s {
                    *contested_by = Some(challenger_id.to_owned());
                    Ok(())
                } else {
                    Err(OreMiningError::ContestRefused)
                }
            }
            _ => Err(OreMiningError::ContestRefused),
        }
    }

    /// Distance in metres and relative speed in $\text{m/s}$ between `ship_id` and `rock_id`.
    #[must_use]
    pub fn ship_to_rock_kinematics(&self, ship_id: &str, rock_id: &str) -> Option<(i64, i64)> {
        let ship = self.bubble.ships.get(ship_id)?;
        let rock = self.rocks.get(rock_id)?;
        let dx = (rock.pos_m[0] - ship.pos_m[0]) as i128;
        let dy = (rock.pos_m[1] - ship.pos_m[1]) as i128;
        let dz = (rock.pos_m[2] - ship.pos_m[2]) as i128;
        let dist_m = isqrt_i128(dx * dx + dy * dy + dz * dz) as i64;

        let dvx = ((ship.vel_mmps[0] - rock.vel_mmps[0]) / 1_000) as i128;
        let dvy = ((ship.vel_mmps[1] - rock.vel_mmps[1]) / 1_000) as i128;
        let dvz = ((ship.vel_mmps[2] - rock.vel_mmps[2]) / 1_000) as i128;
        let rel_mps = isqrt_i128(dvx * dvx + dvy * dvy + dvz * dvz) as i64;
        Some((dist_m, rel_mps))
    }

    /// Execute `+helm/match <rock_id>`: maneuver `ship_id` into station-keeping range
    /// ($800\text{ m} \le 2{,}000\text{ m}$) and match 3D velocity ($|\Delta \mathbf{v}| = 0 < 5\text{ m/s}$).
    pub fn match_velocity_with_rock(
        &mut self,
        ship_id: &str,
        rock_id: &str,
    ) -> Result<(i64, i64), OreMiningError> {
        let rock = self
            .rocks
            .get(rock_id)
            .ok_or(OreMiningError::RockNotFound)?;
        let rock_pos = rock.pos_m;
        let rock_vel = rock.vel_mmps;

        let ship = self
            .bubble
            .ships
            .get_mut(ship_id)
            .ok_or(OreMiningError::ShipNotFound)?;
        let (_, _, dv_cost_mps) = {
            let dvx = ((ship.vel_mmps[0] - rock_vel[0]) / 1_000) as i128;
            let dvy = ((ship.vel_mmps[1] - rock_vel[1]) / 1_000) as i128;
            let dvz = ((ship.vel_mmps[2] - rock_vel[2]) / 1_000) as i128;
            (0, 0, isqrt_i128(dvx * dvx + dvy * dvy + dvz * dvz) as i64)
        };
        ship.delta_v_mps = (ship.delta_v_mps - dv_cost_mps.max(5)).max(0);
        ship.propellant_kg = ship.propellant_kg.saturating_sub(25);
        ship.pos_m = [rock_pos[0] - 600, rock_pos[1], rock_pos[2]];
        ship.vel_mmps = rock_vel;
        ship.thrust_mps2 = [0, 0, 0];
        Ok((600, 0))
    }

    fn verify_extraction_gates(
        &self,
        ship_id: &str,
        rock_id: &str,
    ) -> Result<(i64, i64, bool), OreMiningError> {
        let hold = self
            .holds
            .get(ship_id)
            .ok_or(OreMiningError::ShipNotFound)?;
        if !hold.accepts_stow_ore {
            return Err(OreMiningError::HullLacksStowOre);
        }
        let (dist_m, rel_mps) = self
            .ship_to_rock_kinematics(ship_id, rock_id)
            .ok_or(OreMiningError::RockNotFound)?;
        if dist_m > MAX_MINING_RANGE_M {
            return Err(OreMiningError::OutOfRange {
                distance_m: dist_m,
                max_range_m: MAX_MINING_RANGE_M,
            });
        }
        if rel_mps > MAX_MINING_REL_VEL_MPS {
            return Err(OreMiningError::VelocityMismatch {
                relative_speed_mps: rel_mps,
                max_speed_mps: MAX_MINING_REL_VEL_MPS,
            });
        }

        let now = self.bubble.ic_timestamp_s;
        let has_licence = self.licences.get(ship_id).is_some_and(|l| l.is_active(now));
        let rock = self
            .rocks
            .get(rock_id)
            .ok_or(OreMiningError::RockNotFound)?;

        let mut claim_jump = false;
        match &rock.claim {
            AsteroidClaimState::Unclaimed => {
                if self.jurisdiction == ClusterJurisdiction::CoreCommons && !has_licence {
                    return Err(OreMiningError::UnlicensedInCoreZone);
                }
                if !has_licence {
                    claim_jump = true;
                }
            }
            AsteroidClaimState::FreeportStaked { holder_id, .. } => {
                if holder_id != ship_id {
                    if self.jurisdiction == ClusterJurisdiction::CoreCommons {
                        return Err(OreMiningError::ClaimHeldByOther {
                            holder_id: holder_id.clone(),
                        });
                    }
                    claim_jump = true;
                }
            }
            AsteroidClaimState::PallasSyndicatePaper {
                contested_by_freeport_holder,
                ..
            } => {
                if contested_by_freeport_holder.as_deref() != Some(ship_id) {
                    claim_jump = true;
                }
            }
        }

        Ok((dist_m, rel_mps, claim_jump))
    }

    /// Extract ore directly using the shipboard mining laser (`+mine/laser <rock_id>`, `§7.3.3`).
    ///
    /// Generates high thermal/IR bloom across the bubble; if the ship's `heat_permille`
    /// exceeds `800` while cutting a `CType` volatile rock, triggers an outgassing
    /// $\Delta \mathbf{v}$ kick!
    pub fn extract_with_mining_laser(
        &mut self,
        ship_id: &str,
        rock_id: &str,
        requested_kg: u64,
    ) -> Result<ExtractionReport, OreMiningError> {
        let (_, _, claim_jump) = self.verify_extraction_gates(ship_id, rock_id)?;

        let rock = self
            .rocks
            .get_mut(rock_id)
            .ok_or(OreMiningError::RockNotFound)?;
        let spectral = rock.spectral_class;
        let pool = rock
            .pools
            .iter_mut()
            .find(|p| p.remaining_kg > 0)
            .ok_or(OreMiningError::HoldFullOrDepleted)?;
        let good = pool.good;
        let available_kg = pool.remaining_kg.min(requested_kg);

        let hold = self
            .holds
            .get_mut(ship_id)
            .ok_or(OreMiningError::ShipNotFound)?;
        let loaded_kg = hold.load_ore(good, available_kg);
        if loaded_kg == 0 {
            return Err(OreMiningError::HoldFullOrDepleted);
        }
        pool.remaining_kg -= loaded_kg;
        if let AsteroidClaimState::FreeportStaked { worked_kg, .. } = &mut rock.claim {
            *worked_kg = worked_kg.saturating_add(loaded_kg);
        }

        let used_kg = hold.used_mass_kg();
        let used_l = hold.used_volume_l();

        let ship = self
            .bubble
            .ships
            .get_mut(ship_id)
            .ok_or(OreMiningError::ShipNotFound)?;
        ship.heat_permille = ship.heat_permille.saturating_add(240).min(1_000);
        let ir_bloom = if claim_jump {
            ship.heat_permille.saturating_add(350).min(1_000)
        } else {
            ship.heat_permille.saturating_add(200).min(1_000)
        };

        // Volatile outgassing delta-v kick if overheated on a C-Type ice rock (`§7.3.3`)
        let outgassing_kick =
            if spectral == AsteroidSpectralClass::CType && ship.heat_permille >= 800 {
                let kick_mps = [6, -3, 2];
                ship.vel_mmps[0] = ship.vel_mmps[0].saturating_add(kick_mps[0] * 1_000);
                ship.vel_mmps[1] = ship.vel_mmps[1].saturating_add(kick_mps[1] * 1_000);
                ship.vel_mmps[2] = ship.vel_mmps[2].saturating_add(kick_mps[2] * 1_000);
                kick_mps
            } else {
                [0, 0, 0]
            };

        Ok(ExtractionReport {
            rock_id: rock_id.to_owned(),
            good,
            extracted_kg: loaded_kg,
            hold_used_kg: used_kg,
            hold_used_l: used_l,
            ir_bloom_permille: ir_bloom,
            claim_jump_flagged: claim_jump,
            outgassing_dv_kick_mps: outgassing_kick,
        })
    }

    /// Launch up to [`MAX_DRONES_PER_DRAY_P`] (4) collector drones sealed under an
    /// [`ExcursionId`] (`+mine/drones launch <rock_id>`, `doc 20 §17`, `D-35-13`).
    pub fn launch_collector_drones(
        &mut self,
        ship_id: &str,
        rock_id: &str,
        drone_count: usize,
    ) -> Result<(ExcursionId, Vec<CollectorDrone>), OreMiningError> {
        if drone_count == 0 || drone_count > MAX_DRONES_PER_DRAY_P {
            return Err(OreMiningError::DroneBayCapExceeded {
                requested: drone_count,
                max_drones: MAX_DRONES_PER_DRAY_P,
            });
        }
        let _ = self.verify_extraction_gates(ship_id, rock_id)?;

        let exc_id = ExcursionId(self.next_excursion_id);
        self.next_excursion_id = self.next_excursion_id.saturating_add(1);

        let mut drones = Vec::with_capacity(drone_count);
        for slot in 0..drone_count {
            drones.push(CollectorDrone {
                bay_slot: (slot as u8).saturating_add(1),
                excursion_id: exc_id,
                target_rock_id: rock_id.to_owned(),
                phase: DroneDoctrinePhase::Scan,
                hopper_good: None,
                hopper_kg: 0,
                hopper_capacity_kg: 2_500,
            });
        }
        self.active_drones
            .insert(ship_id.to_owned(), drones.clone());
        Ok((exc_id, drones))
    }

    /// Step a `Dray-P`'s active collector drone sortie through the stock `mining` doctrine
    /// (`Scan -> Sound -> Goto -> Mine -> Recover -> DockedInBay`) and unload the drone
    /// hoppers into the ship's `stow.ore` hold (`doc 20 §17`, `D-35-13`).
    pub fn complete_drone_sortie_cycle(
        &mut self,
        ship_id: &str,
    ) -> Result<ExtractionReport, OreMiningError> {
        let Some(drones) = self.active_drones.get_mut(ship_id) else {
            return Err(OreMiningError::ShipNotFound);
        };
        let Some(first) = drones.first() else {
            return Err(OreMiningError::HoldFullOrDepleted);
        };
        let rock_id = first.target_rock_id.clone();
        let (_, _, claim_jump) = self.verify_extraction_gates(ship_id, &rock_id)?;

        let rock = self
            .rocks
            .get_mut(&rock_id)
            .ok_or(OreMiningError::RockNotFound)?;
        let pool = rock
            .pools
            .iter_mut()
            .find(|p| p.remaining_kg > 0)
            .ok_or(OreMiningError::HoldFullOrDepleted)?;
        let good = pool.good;

        let hold = self
            .holds
            .get_mut(ship_id)
            .ok_or(OreMiningError::ShipNotFound)?;
        let mut total_unloaded_kg = 0u64;

        if let Some(drones_mut) = self.active_drones.get_mut(ship_id) {
            for drone in drones_mut {
                // Doctrine progression: Scan -> Sound -> Goto -> Mine -> Recover -> DockedInBay
                drone.phase = DroneDoctrinePhase::Mine;
                let mined = pool.remaining_kg.min(drone.hopper_capacity_kg);
                let fit = hold.load_ore(good, mined);
                pool.remaining_kg -= fit;
                drone.hopper_good = Some(good);
                drone.hopper_kg = 0;
                drone.phase = DroneDoctrinePhase::DockedInBay;
                total_unloaded_kg = total_unloaded_kg.saturating_add(fit);
            }
        }

        if total_unloaded_kg == 0 {
            return Err(OreMiningError::HoldFullOrDepleted);
        }
        if let AsteroidClaimState::FreeportStaked { worked_kg, .. } = &mut rock.claim {
            *worked_kg = worked_kg.saturating_add(total_unloaded_kg);
        }

        Ok(ExtractionReport {
            rock_id,
            good,
            extracted_kg: total_unloaded_kg,
            hold_used_kg: hold.used_mass_kg(),
            hold_used_l: hold.used_volume_l(),
            // Drones run cold compared to the shipboard mining laser!
            ir_bloom_permille: if claim_jump { 220 } else { 95 },
            claim_jump_flagged: claim_jump,
            outgassing_dv_kick_mps: [0, 0, 0],
        })
    }
}
