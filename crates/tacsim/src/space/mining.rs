//! **Asteroid Field Mining on the Doc 20 Engine (`SF.R5`, `D-35-13`, `Q-35-3`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §7, §10.2,
//! `docs/design/35a-space-flight-review.md` §1, §3, §4, §5 #11–#13, §8.3).
//!
//! Connects orbital cluster mining (`+mine`, `+helm/cluster`, `+helm/approach`, `+helm/match`)
//! directly to the Doc 20 / Doc 18 industry and inventory engines:
//! 1. **Real `mush_industry::OreDeposit` pools & `tacsim.pockets` cursors (`D-35-13`)**:
//!    Each orbital cluster (`ceres_halo`, `kirkwood_gap_3_1`, `pallas_inclined_belt`,
//!    `saturn_b_ring_edge`) lays up real `DARK` block place `Thing`s (`A`, `B`, `C`), block
//!    `Vault`s owned by `System(tacsim.sites)` ([`crate::port::site::SITES_PRINCIPAL`]), and a
//!    cluster clerk `Thing`. On `space_tick`, the cluster clerk opens each rock's
//!    `mush_industry::OreDeposit` via `industry_deposit_open` ([`next_cluster_opening`] in
//!    [`crate::composition::industry_seams::SiteOpener`]), crediting conserved ore into the rock's
//!    deposit vault.
//! 2. **Freeport Registry prospecting licence & `tacsim.claims` tenure (`D-35-13`, `35a` §5 #12)**:
//!    Extraction and claim actions require a live 2,500 cr / 168 IC-day Freeport Registry
//!    prospecting licence via [`crate::port::licence::licence_gate`]. Staking (`+mine/stake`) and
//!    contesting (`+mine/contest`) submit `industry_claim_tenure` through
//!    [`crate::composition::industry_seams::SurfaceTenure`] ([`next_cluster_tenure`]), staging
//!    real [`crate::surface::claims::BlockTenure`] records in `tacsim.claims` with [`CLAIM_TERM_IC_SECS`] (`168` IC days)
//!    and [`CONTEST_WINDOW_IC_SECS`] (`30` IC days, `35a` §5 #12). `pallas_inclined_belt` blocks
//!    `A` and `B` are seeded with `pallas_syndicate` claims; unleased or Syndicate-held frontier
//!    extraction raises a thermal bloom (`+18` engineering heat) and records a claim-jump flag
//!    without conjuring any scripted rival inside `+mine/laser` (`35a` §3 #9).
//! 3. **Exact 3D Euclidean velocity-match gate (`35a` §4, §5 #11)**:
//!    `+helm/match <rock>` never teleports the ship. It requires 3D Euclidean range `<= 2,000 m`
//!    and 3D Euclidean relative speed `dvx² + dvy² + dvz² < 5,000²` in `(mm/s)²` (`|Δv| < 5.000 m/s`).
//! 4. **Timed laser & 4 collector drones under [`ExcursionId`] into `stow.ore` (`35a` §3 #6, §5 #13)**:
//!    `+mine/laser` and `+mine/drones` start timed cycles progressed by `space_tick` on
//!    `WorldTimeline`. Up to [`crate::surface::excursion::MAX_EXCURSIONS_PER_OPERATOR`] (`4`) collector drones per `Dray-P`
//!    run under real [`ExcursionId`]s. Completed cycles submit `tacsim_site_work`
//!    ([`next_space_mine_work_row`]) to transfer conserved ore (`metal_ore`, `ice_ore`,
//!    `carbon_ore`, `strategic_ore`) from the rock's deposit vault into the hull's hold vault
//!    (`stow.ore`), respecting `dray_p.json`'s `86,000 kg` (`86 t`) hold capacity and updating
//!    [`PocketCursor`] in `tacsim.pockets`.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ClockStamp, Ctx, Engine, Flags, ObjId, ObjKind, StateFamilyHandle};
use mush_economy::{CustodyRef, Vault, VaultId, VaultStatus};
use mush_industry::{
    ClaimRecord, ClaimRev, CurvePoint, OreDeposit, OreDepositId, SystemOpening, TenureAct,
    TenureKind,
};
use mush_org::{PrincipalId, SystemPrincipalId};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::port::claims::TenureGame;
use crate::port::def::Tenure;
use crate::port::site::{SITES_PRINCIPAL, org_principal};
use crate::surface::claims::BlockKey;
use crate::surface::cursor::{
    CursorRow, MAX_CURSOR_ROWS, MAX_POCKET_NUMBER, MAX_POCKET_TONNES, POCKET_CURSOR_SCHEMA,
    PocketCursor, cursor_key_text,
};
use crate::surface::excursion::ExcursionId;
use crate::surface::geology::OreFamily;
use crate::surface::opening::{
    ORE_DEPOSIT_ACCESSIBILITY, ORE_DEPOSIT_GRADE, SURFACE_BAND, SURFACE_PROVIDER, ore_good, receipt,
};
use crate::surface::tenure::{CLAIM_TERM_IC_SECS, CONTEST_WINDOW_IC_SECS, contest_open};

/// Maximum 3D Euclidean range in metres for velocity matching and mining (`2,000 m`, `doc 35` §7.3).
pub const MAX_MINING_MATCH_RANGE_M: i64 = 2_000;

/// Maximum 3D Euclidean relative speed in millimetres per second for velocity matching (`5,000 mm/s = 5.000 m/s`, `35a` §5 #11).
pub const MAX_VELOCITY_MATCH_SPEED_MMPS: i64 = 5_000;

/// `Dray-P` (`dray_p.json`) unreserved bulk/ore hold capacity in kilograms (`86,000 kg = 86 t`, `35a` §5 #13).
pub const DRAY_P_HOLD_CAPACITY_KG: u64 = 86_000;

/// `Dray-P` (`dray_p.json`) unreserved bulk/ore hold capacity in whole tonnes (`86 t`).
pub const DRAY_P_HOLD_CAPACITY_T: u64 = 86;

/// Timed mining-laser cycle duration in IC seconds (`60 s`).
pub const LASER_CYCLE_IC_SECS: i64 = 60;

/// Tonnes of ore cut per completed mining-laser cycle (`4 t`).
pub const LASER_CYCLE_YIELD_T: u64 = 4;

/// Timed collector-drone sortie duration in IC seconds (`120 s`).
pub const DRONE_SORTIE_IC_SECS: i64 = 120;

/// Tonnes of ore collected per completed collector-drone sortie (`4 t`, so 4 drones yield `16 t`).
pub const DRONE_SORTIE_YIELD_T: u64 = 4;

/// Engineering heat added per normal mining-laser cycle (`+6` heat).
pub const LASER_BASE_HEAT: u16 = 6;

/// Additional thermal bloom heat added when mining unleased or Syndicate-held frontier ground (`+18` heat).
pub const FRONTIER_LASER_BLOOM_HEAT: u16 = 18;

/// Specific volume in litres per tonne (`L/t`, equal to `mL/kg`) for each [`OreFamily`],
/// matching `goods.json` (`35a` §5 #13).
#[must_use]
pub const fn ore_specific_volume_l_per_t(family: OreFamily) -> u32 {
    match family {
        OreFamily::Volatiles => 1_000,       // ice_ore: 1,000 L/t
        OreFamily::CarbonFeedstock => 1_200, // carbon_ore: 1,200 L/t
        OreFamily::Metals => 500,            // metal_ore: 500 L/t
        OreFamily::StrategicElements => 600, // strategic_ore: 600 L/t
    }
}

/// Canonical orbital asteroid clusters (`doc 35` §7.1, `D-35-13`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrbitalClusterId {
    /// `ceres_halo` — Core commons around Ceres Freeport.
    CeresHalo,
    /// `kirkwood_gap_3_1` — Frontier 3:1 resonance gap at 2.50 AU.
    KirkwoodGap31,
    /// `pallas_inclined_belt` — Frontier inclined belt near Pallas; Pallas Syndicate claims.
    PallasInclinedBelt,
    /// `saturn_b_ring_edge` — Frontier dense ice/silicate ring edge near Saturn/Titan.
    SaturnBRingEdge,
}

/// Security and tenure zone of an [`OrbitalClusterId`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClusterZone {
    /// Core patrolled commons (`ceres_halo`).
    Core,
    /// Frontier claims and contested space (`kirkwood_gap_3_1`, `pallas_inclined_belt`, `saturn_b_ring_edge`).
    Frontier,
}

/// Asteroid spectral class (`doc 35` §7.2, `D-35-13`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SpectralClass {
    /// Carbonaceous (`C-TYPE`): `ice_ore` (`Volatiles`) + `carbon_ore` (`CarbonFeedstock`).
    CType,
    /// Silicaceous (`S-TYPE`): `metal_ore` (`Metals`).
    SType,
    /// Metallic (`M-TYPE`): `metal_ore` (`Metals`) + `strategic_ore` (`StrategicElements`).
    MType,
}

impl SpectralClass {
    /// Short code (`"C-TYPE"`, `"S-TYPE"`, `"M-TYPE"`).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::CType => "C-TYPE",
            Self::SType => "S-TYPE",
            Self::MType => "M-TYPE",
        }
    }

    /// Ore families yielded by this spectral class (`doc 35` §7.2).
    #[must_use]
    pub const fn families(self) -> &'static [OreFamily] {
        match self {
            Self::CType => &[OreFamily::Volatiles, OreFamily::CarbonFeedstock],
            Self::SType => &[OreFamily::Metals],
            Self::MType => &[OreFamily::Metals, OreFamily::StrategicElements],
        }
    }

    /// Primary default [`OreFamily`] for this spectral class when none is specified.
    #[must_use]
    pub const fn primary_family(self) -> OreFamily {
        match self {
            Self::CType => OreFamily::Volatiles,
            Self::SType | Self::MType => OreFamily::Metals,
        }
    }
}

/// Authored specification of one asteroid pocket (`A`, `B`, `C`) inside an [`OrbitalClusterId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClusterRockSpec {
    /// Block letter (`'A'`, `'B'`, `'C'`).
    pub letter: char,
    /// Alias token (`"rock_1"`, `"rock_2"`, `"rock_3"`).
    pub alias: &'static str,
    /// Spectral class (`C`, `S`, `M`).
    pub spectral: SpectralClass,
    /// Position in cluster bubble coordinates (`[x_m, y_m, z_m]`).
    pub pos_m: [i64; 3],
    /// Drift velocity in cluster bubble coordinates (`[vx_mmps, vy_mmps, vz_mmps]`).
    pub vel_mmps: [i64; 3],
    /// Initial conserved `OreDeposit` reserve per ore family in tonnes.
    pub gross_t: u64,
    /// Authored default tenure (`Commons`, `Open`, `Held`).
    pub tenure: Tenure,
    /// Authored holder org id (`Some("pallas_syndicate")` on Pallas blocks `A` and `B`).
    pub holder_org: Option<&'static str>,
}

const CERES_HALO_ROCKS: [ClusterRockSpec; 3] = [
    ClusterRockSpec {
        letter: 'A',
        alias: "rock_1",
        spectral: SpectralClass::CType,
        pos_m: [1_200, 0, 0],
        vel_mmps: [0, 1_000, 0],
        gross_t: 120,
        tenure: Tenure::Commons,
        holder_org: None,
    },
    ClusterRockSpec {
        letter: 'B',
        alias: "rock_2",
        spectral: SpectralClass::SType,
        pos_m: [-1_400, 900, 0],
        vel_mmps: [1_000, 0, 0],
        gross_t: 120,
        tenure: Tenure::Commons,
        holder_org: None,
    },
    ClusterRockSpec {
        letter: 'C',
        alias: "rock_3",
        spectral: SpectralClass::CType,
        pos_m: [600, -1_500, 300],
        vel_mmps: [0, -1_000, 0],
        gross_t: 120,
        tenure: Tenure::Commons,
        holder_org: None,
    },
];

const KIRKWOOD_GAP_ROCKS: [ClusterRockSpec; 3] = [
    ClusterRockSpec {
        letter: 'A',
        alias: "rock_1",
        spectral: SpectralClass::SType,
        pos_m: [1_000, 400, 0],
        vel_mmps: [2_000, 0, 0],
        gross_t: 120,
        tenure: Tenure::Open,
        holder_org: None,
    },
    ClusterRockSpec {
        letter: 'B',
        alias: "rock_2",
        spectral: SpectralClass::MType,
        pos_m: [-1_100, 1_200, 200],
        vel_mmps: [0, 2_000, 0],
        gross_t: 120,
        tenure: Tenure::Open,
        holder_org: None,
    },
    ClusterRockSpec {
        letter: 'C',
        alias: "rock_3",
        spectral: SpectralClass::CType,
        pos_m: [500, -1_300, -200],
        vel_mmps: [-1_000, 1_000, 0],
        gross_t: 120,
        tenure: Tenure::Open,
        holder_org: None,
    },
];

const PALLAS_BELT_ROCKS: [ClusterRockSpec; 3] = [
    ClusterRockSpec {
        letter: 'A',
        alias: "rock_1",
        spectral: SpectralClass::SType,
        pos_m: [1_200, 300, 0],
        vel_mmps: [2_000, 1_000, 0],
        gross_t: 120,
        tenure: Tenure::Held,
        holder_org: Some("pallas_syndicate"),
    },
    ClusterRockSpec {
        letter: 'B',
        alias: "rock_2",
        spectral: SpectralClass::MType,
        pos_m: [-900, 1_400, 300],
        vel_mmps: [-1_000, 2_000, 0],
        gross_t: 120,
        tenure: Tenure::Held,
        holder_org: Some("pallas_syndicate"),
    },
    ClusterRockSpec {
        letter: 'C',
        alias: "rock_3",
        spectral: SpectralClass::SType,
        pos_m: [800, -1_100, -200],
        vel_mmps: [1_000, -1_000, 0],
        gross_t: 120,
        tenure: Tenure::Open,
        holder_org: None,
    },
];

const SATURN_RING_ROCKS: [ClusterRockSpec; 3] = [
    ClusterRockSpec {
        letter: 'A',
        alias: "rock_1",
        spectral: SpectralClass::CType,
        pos_m: [900, 600, 0],
        vel_mmps: [3_000, 1_000, 0],
        gross_t: 120,
        tenure: Tenure::Open,
        holder_org: None,
    },
    ClusterRockSpec {
        letter: 'B',
        alias: "rock_2",
        spectral: SpectralClass::CType,
        pos_m: [-1_000, 800, 150],
        vel_mmps: [-2_000, 2_000, 0],
        gross_t: 120,
        tenure: Tenure::Open,
        holder_org: None,
    },
    ClusterRockSpec {
        letter: 'C',
        alias: "rock_3",
        spectral: SpectralClass::SType,
        pos_m: [700, -1_200, -150],
        vel_mmps: [1_000, -2_000, 0],
        gross_t: 120,
        tenure: Tenure::Open,
        holder_org: None,
    },
];

impl OrbitalClusterId {
    /// All four canonical orbital clusters (`doc 35` §7.1).
    pub const ALL: [Self; 4] = [
        Self::CeresHalo,
        Self::KirkwoodGap31,
        Self::PallasInclinedBelt,
        Self::SaturnBRingEdge,
    ];

    /// Canonical identifier string (`"ceres_halo"`, `"kirkwood_gap_3_1"`,
    /// `"pallas_inclined_belt"`, `"saturn_b_ring_edge"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CeresHalo => "ceres_halo",
            Self::KirkwoodGap31 => "kirkwood_gap_3_1",
            Self::PallasInclinedBelt => "pallas_inclined_belt",
            Self::SaturnBRingEdge => "saturn_b_ring_edge",
        }
    }

    /// Display name of the orbital cluster.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::CeresHalo => "Ceres Halo Commons",
            Self::KirkwoodGap31 => "3:1 Kirkwood Resonance Gap",
            Self::PallasInclinedBelt => "Pallas Inclined Belt",
            Self::SaturnBRingEdge => "Saturn B-Ring Edge",
        }
    }

    /// Security/tenure zone (`Core` vs `Frontier`).
    #[must_use]
    pub const fn zone(self) -> ClusterZone {
        match self {
            Self::CeresHalo => ClusterZone::Core,
            Self::KirkwoodGap31 | Self::PallasInclinedBelt | Self::SaturnBRingEdge => {
                ClusterZone::Frontier
            }
        }
    }

    /// Anchor ephemeris body id (`ceres`, `pallas`, `titan`).
    #[must_use]
    pub const fn anchor_body(self) -> &'static str {
        match self {
            Self::CeresHalo | Self::KirkwoodGap31 => "ceres",
            Self::PallasInclinedBelt => "pallas",
            Self::SaturnBRingEdge => "titan",
        }
    }

    /// Parse a cluster identifier or short alias.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "ceres_halo" | "ceres" | "halo" => Some(Self::CeresHalo),
            "kirkwood_gap_3_1" | "kirkwood" | "kirkwood_gap" => Some(Self::KirkwoodGap31),
            "pallas_inclined_belt" | "pallas" | "pallas_belt" => Some(Self::PallasInclinedBelt),
            "saturn_b_ring_edge" | "saturn" | "saturn_b_ring" | "b_ring" => {
                Some(Self::SaturnBRingEdge)
            }
            _ => None,
        }
    }

    /// Convert to a [`CanonicalId`].
    #[must_use]
    pub fn canonical_id(self) -> Option<CanonicalId> {
        CanonicalId::new(self.as_str()).ok()
    }

    /// Authored asteroid pockets (`A`, `B`, `C`) in this cluster.
    #[must_use]
    pub const fn rocks(self) -> &'static [ClusterRockSpec; 3] {
        match self {
            Self::CeresHalo => &CERES_HALO_ROCKS,
            Self::KirkwoodGap31 => &KIRKWOOD_GAP_ROCKS,
            Self::PallasInclinedBelt => &PALLAS_BELT_ROCKS,
            Self::SaturnBRingEdge => &SATURN_RING_ROCKS,
        }
    }

    /// Look up a rock in this cluster by block letter (`'A'`..`'C'`) or alias (`"rock_1"`..`"rock_3"`).
    #[must_use]
    pub fn rock(self, token: &str) -> Option<&'static ClusterRockSpec> {
        let trimmed = token.trim();
        self.rocks().iter().find(|spec| {
            spec.letter
                .eq_ignore_ascii_case(&trimmed.chars().next().unwrap_or('\0'))
                && trimmed.len() == 1
                || spec.alias.eq_ignore_ascii_case(trimmed)
        })
    }
}

/// Parse an [`OreFamily`] from a player token (`metals`/`metal_ore`, `volatiles`/`ice_ore`,
/// `carbon`/`carbon_ore`, `strategic`/`strategic_ore`).
#[must_use]
pub fn parse_ore_family(token: &str) -> Option<OreFamily> {
    match token.trim().to_ascii_lowercase().as_str() {
        "metals" | "metal" | "metal_ore" => Some(OreFamily::Metals),
        "volatiles" | "volatile" | "ice" | "ice_ore" => Some(OreFamily::Volatiles),
        "carbon_feedstock" | "carbon" | "carbon_ore" => Some(OreFamily::CarbonFeedstock),
        "strategic_elements" | "strategic" | "strategic_ore" => Some(OreFamily::StrategicElements),
        _ => None,
    }
}

/// Evaluate the exact 3D Euclidean range (`<= 2,000 m`) and 3D Euclidean relative velocity
/// (`|Δv| < 5,000 mm/s = 5.000 m/s`) required for `+helm/match` and mining operations
/// (`35a` §4, §5 #11).
///
/// Returns `Ok((dist_m, rel_speed_mmps))` when both gates hold, or an `Err` refusal string.
pub fn check_velocity_match_gate(
    ship_pos_m: [i64; 3],
    ship_vel_mmps: [i64; 3],
    rock_pos_m: [i64; 3],
    rock_vel_mmps: [i64; 3],
) -> Result<(i64, i64), &'static str> {
    let dx = i128::from(ship_pos_m[0]) - i128::from(rock_pos_m[0]);
    let dy = i128::from(ship_pos_m[1]) - i128::from(rock_pos_m[1]);
    let dz = i128::from(ship_pos_m[2]) - i128::from(rock_pos_m[2]);
    let dist_sq_m2 = dx * dx + dy * dy + dz * dz;
    let max_range_sq = i128::from(MAX_MINING_MATCH_RANGE_M) * i128::from(MAX_MINING_MATCH_RANGE_M);
    let dist_m = libm::sqrt(dist_sq_m2 as f64).round() as i64;
    if dist_sq_m2 > max_range_sq {
        return Err(
            "Target rock is outside the 2,000 m velocity-match envelope; close range first.",
        );
    }

    let dvx = i128::from(ship_vel_mmps[0]) - i128::from(rock_vel_mmps[0]);
    let dvy = i128::from(ship_vel_mmps[1]) - i128::from(rock_vel_mmps[1]);
    let dvz = i128::from(ship_vel_mmps[2]) - i128::from(rock_vel_mmps[2]);
    let dv_sq_mmps2 = dvx * dvx + dvy * dvy + dvz * dvz;
    let max_speed_sq =
        i128::from(MAX_VELOCITY_MATCH_SPEED_MMPS) * i128::from(MAX_VELOCITY_MATCH_SPEED_MMPS);
    let rel_speed_mmps = libm::sqrt(dv_sq_mmps2 as f64).round() as i64;
    if dv_sq_mmps2 >= max_speed_sq {
        return Err(
            "Relative velocity exceeds the 5.0 m/s match gate; brake relative speed first.",
        );
    }
    Ok((dist_m, rel_speed_mmps))
}

/// Laid-up world objects and vaults for an [`OrbitalClusterId`].
#[derive(Clone, Debug)]
pub struct ClusterSiteState {
    /// Cluster identifier.
    pub cluster: OrbitalClusterId,
    /// Host room holding the `DARK` block place `Thing`s and cluster clerk.
    pub host_room: ObjId,
    /// `DARK` clerk `Thing` that submits `DepositOpen`, `ClaimTenure` seeds, and `tacsim_site_work`.
    pub clerk: ObjId,
    /// Block letter (`'A'`..`'C'`) -> `DARK` block place `Thing`.
    pub blocks: BTreeMap<char, ObjId>,
    /// `(letter, family)` -> `DARK` deposit-opening clerk `Thing`.
    pub deposit_clerks: BTreeMap<(char, OreFamily), ObjId>,
    /// Block letter (`'A'`..`'C'`) -> deposit endpoint `VaultId` owned by `System(tacsim.sites)`.
    pub vaults: BTreeMap<char, VaultId>,
}

/// Timed mining-laser cycle in flight on a hull (`+mine/laser`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaserExtractionCycle {
    /// Target rock block letter (`'A'`, `'B'`, `'C'`).
    pub rock_letter: char,
    /// Target [`OreFamily`].
    pub family: OreFamily,
    /// W-clock IC timestamp when the laser cycle started.
    pub started_ic: i64,
    /// W-clock IC timestamp when the laser cycle completes.
    pub due_ic: i64,
    /// Tonnes of ore to extract on completion.
    pub yield_t: u64,
    /// Operator who started the cycle.
    pub operator: ObjId,
}

/// Active collector-drone sortie in flight on a `Dray-P` (`+mine/drones`, `D-35-13`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroneSortieState {
    /// Drone bay slot (`1..=4`).
    pub slot: u8,
    /// Canonical [`ExcursionId`] (`(drone_obj_id, launch_ic)` from `crate::surface::excursion`).
    pub excursion_id: ExcursionId,
    /// Target rock block letter (`'A'`, `'B'`, `'C'`).
    pub rock_letter: char,
    /// Target [`OreFamily`].
    pub family: OreFamily,
    /// W-clock IC timestamp when the sortie completes and unloads into `stow.ore`.
    pub due_ic: i64,
    /// Tonnes of ore collected on completion.
    pub yield_t: u64,
    /// Operator who launched the sortie.
    pub operator: ObjId,
}

/// Active orbital cluster session for a carrier hull (`+helm/cluster`).
#[derive(Clone, Debug)]
pub struct ActiveClusterMiningSession {
    /// Carrier hull [`ObjId`].
    pub hull: ObjId,
    /// Active orbital cluster.
    pub cluster: OrbitalClusterId,
    /// Ship's position in cluster coordinates (`[x_m, y_m, z_m]`).
    pub ship_pos_m: [i64; 3],
    /// Ship's velocity in cluster coordinates (`[vx_mmps, vy_mmps, vz_mmps]`).
    pub ship_vel_mmps: [i64; 3],
    /// Rock currently targeted for approach braking (`+helm/approach <rock>`), if any.
    pub approaching_rock: Option<char>,
    /// Rock currently locked in RCS velocity match (`+helm/match <rock>`), if any.
    pub matched_rock: Option<char>,
    /// Hand ping timestamps (`rock_letter -> first_ping_ic`) recorded by `(actor, rock_letter)`.
    pub pings: BTreeMap<(ObjId, char), i64>,
    /// Active timed mining-laser cycle, if any.
    pub active_laser: Option<LaserExtractionCycle>,
    /// Active collector-drone sorties (at most [`crate::surface::excursion::MAX_EXCURSIONS_PER_OPERATOR`] = 4).
    pub active_drones: Vec<DroneSortieState>,
    /// Allocated `DARK` collector-drone `Thing`s per bay slot (`1..=4`) for [`ExcursionId`].
    pub drone_things: BTreeMap<u8, ObjId>,
    /// Cumulative thermal bloom heat (`MW-s` / heat units) generated by unleased frontier mining.
    pub frontier_bloom_heat: u16,
    /// Whether this hull has been flagged for unleased / claim-jumped extraction in this cluster.
    pub claim_jump_flagged: bool,
    /// Last W-clock IC timestamp stepped.
    pub last_ic_s: i64,
}

/// Server-authenticated ore transfer queued for `tacsim_site_work` when a laser cycle or
/// collector-drone sortie finishes on `space_tick`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSpaceMineDraw {
    /// Unique receipt (`"{cluster}/{letter}/{family}/draw/{seq}"`).
    pub receipt: String,
    /// Cluster whose clerk submits the transfer.
    pub cluster: OrbitalClusterId,
    ///Cluster clerk [`ObjId`].
    pub clerk: ObjId,
    /// Carrier hull receiving the ore.
    pub hull: ObjId,
    /// Operator notified on completion.
    pub operator: ObjId,
    /// Deposit id (`derive_system_deposit_id("{cluster}/{letter}/{family}")`).
    pub deposit: OreDepositId,
    /// Ore commodity (`"metal_ore"`, `"ice_ore"`, `"carbon_ore"`, `"strategic_ore"`).
    pub commodity: CanonicalId,
    /// Source rock deposit `VaultId`.
    pub from_vault: VaultId,
    /// Destination hull hold `VaultId` (`stow.ore`).
    pub to_vault: VaultId,
    /// Whole tonnes to transfer.
    pub worked_t: u64,
    /// Expected cumulative drawn tonnes in `tacsim.pockets` once this draw lands.
    pub expected_total_t: u64,
    /// Optional [`ExcursionId`] when the draw came from a collector-drone sortie.
    pub excursion: Option<ExcursionId>,
}

/// Pending player claim stake or contest order in an orbital cluster (`+mine/stake`, `+mine/contest`).
#[derive(Clone, Debug)]
pub struct ClusterTenureOrder {
    /// Acting licensed pilot.
    pub actor: ObjId,
    /// Target orbital cluster.
    pub cluster: OrbitalClusterId,
    /// Target rock block letter (`'A'`, `'B'`, `'C'`).
    pub letter: char,
    /// Whether this is a contest (`true`) or a fresh stake (`false`).
    pub is_contest: bool,
    /// Declared [`OreFamily`].
    pub declared: Option<OreFamily>,
    /// Pilot's hand ping timestamp (`discovered_at`).
    pub discovered_ic: i64,
    /// W-clock IC timestamp when the order was created.
    pub made_ic: i64,
    /// Receipt for `industry_claim_tenure`.
    pub receipt: String,
}

/// Build an updated [`PocketCursor`] by adding `added_t` tonnes to `prior`, filling rows of at
/// most [`MAX_POCKET_TONNES`] (`24 t`) across strictly increasing pocket numbers `1..=64` so
/// [`PocketCursor::check`] and `SurfaceDraws::drawn` (`initial_reserve - pool == Σ drawn`) hold.
#[must_use]
pub fn build_updated_pocket_cursor(
    deposit: OreDepositId,
    prior: Option<&PocketCursor>,
    added_t: u64,
) -> Option<PocketCursor> {
    let prior_total = match prior {
        Some(cursor) => cursor.drawn_total()?,
        None => 0,
    };
    let target_total = prior_total.checked_add(added_t)?;
    if target_total == 0 {
        return None;
    }
    let max_per_row = u64::from(MAX_POCKET_TONNES);
    let mut remaining = target_total;
    let mut rows = Vec::new();
    let mut pocket: u16 = 1;
    while remaining > 0 {
        if rows.len() >= MAX_CURSOR_ROWS || pocket > MAX_POCKET_NUMBER {
            return None;
        }
        let chunk = remaining.min(max_per_row);
        let drawn_t = u32::try_from(chunk).ok()?;
        rows.push(CursorRow {
            pocket,
            drawn_t,
            lock: None,
        });
        remaining -= chunk;
        pocket = pocket.saturating_add(1);
    }
    let cursor = PocketCursor {
        schema: POCKET_CURSOR_SCHEMA,
        deposit,
        rows,
    };
    cursor.check().ok()?;
    Some(cursor)
}

/// Ensure the `DARK` block place `Thing`s (`A`, `B`, `C`), deposit `Vault`s (`SITES_PRINCIPAL`),
/// and cluster clerk `Thing` exist for `cluster`.
pub fn ensure_cluster_laid_up(
    ctx: &mut Ctx<'_>,
    cluster: OrbitalClusterId,
) -> Result<ClusterSiteState, String> {
    if let Some(existing) = ctx
        .w
        .module_state::<super::tick::SpaceBubbleBook>()
        .and_then(|book| book.cluster_sites.get(&cluster).cloned())
    {
        return Ok(existing);
    }

    let host_room = ctx
        .w
        .get(ctx.actor)
        .and_then(|obj| obj.location)
        .unwrap_or(ObjId(0));
    let cluster_cid = cluster
        .canonical_id()
        .ok_or_else(|| "invalid cluster canonical id".to_owned())?;
    let saved_actor = ctx.actor;
    ctx.actor = mush_core::GOD;
    let clerk_res = crate::port::site::dark_thing(
        ctx,
        &cluster_cid,
        &format!("{} cluster clerk", cluster.display_name()),
        host_room,
    );
    ctx.actor = saved_actor;
    let clerk = ObjId(clerk_res?);

    let owner = SystemPrincipalId::new(SITES_PRINCIPAL)
        .map(PrincipalId::System)
        .ok_or_else(|| format!("{SITES_PRINCIPAL} is not a valid system principal"))?;

    let mut blocks = BTreeMap::new();
    let mut deposit_clerks = BTreeMap::new();
    let mut vaults = BTreeMap::new();
    for spec in cluster.rocks() {
        let saved = ctx.actor;
        ctx.actor = mush_core::GOD;
        let place_res = crate::port::site::dark_thing(
            ctx,
            &cluster_cid,
            &format!("{} block {}", cluster.display_name(), spec.letter),
            host_room,
        );
        ctx.actor = saved;
        let place = ObjId(place_res?);
        let vault_id = crate::port::apply::alloc_vault_id(ctx)
            .map_err(|e| format!("{cluster_cid}: block {} vault: {e}", spec.letter))?;
        mush_economy::create_vault(
            ctx,
            Vault {
                id: vault_id,
                owner: owner.clone(),
                scope: None,
                name: SmolStr::new(format!(
                    "{} block {} ground",
                    cluster.display_name(),
                    spec.letter
                )),
                backing: CustodyRef::Container(place),
                status: VaultStatus::Active,
                revision: 1,
            },
        );
        for &family in spec.spectral.families() {
            let saved_dep = ctx.actor;
            ctx.actor = mush_core::GOD;
            let dep_clerk_res = crate::port::site::dark_thing(
                ctx,
                &cluster_cid,
                &format!(
                    "{} block {} {} opener",
                    cluster.display_name(),
                    spec.letter,
                    family.id()
                ),
                host_room,
            );
            ctx.actor = saved_dep;
            deposit_clerks.insert((spec.letter, family), ObjId(dep_clerk_res?));
        }
        blocks.insert(spec.letter, place);
        vaults.insert(spec.letter, vault_id);
    }

    let state = ClusterSiteState {
        cluster,
        host_room,
        clerk,
        blocks,
        deposit_clerks,
        vaults,
    };
    if let Some(book) = ctx.w.module_state_mut::<super::tick::SpaceBubbleBook>() {
        book.cluster_sites.insert(cluster, state.clone());
    }

    // Immediately submit `DepositOpen` for each `(letter, family)` via its dedicated opener clerk.
    if let Some(open_action) =
        mush_industry::action_ref(ctx.reg, mush_industry::IndustryVerb::DepositOpen)
    {
        let mut fire = crate::port_tick::Fire::new(BTreeMap::new());
        for (&(letter, family), &dep_clerk) in &state.deposit_clerks {
            let rec = receipt(&cluster_cid, letter, family);
            if let Ok(key) = mush_core::IdempotencyKeyText::parse_canonical(format!(
                "{}{rec}",
                crate::port::site::SITE_OPEN_KEY_PREFIX
            )) {
                let _ = fire.as_principal(ctx, dep_clerk, dep_clerk, |ctx| {
                    ctx.submit_authoritative(
                        open_action.clone(),
                        mush_core::PerceivedRef::SelfActor,
                        Vec::new(),
                        json!({ "receipt": rec }),
                        key,
                    );
                });
            }
        }
    }

    Ok(state)
}

/// Ensure a `DARK` collector-drone `Thing` exists for `(hull, slot)` so [`ExcursionId`] carries
/// a real `ObjId`.
pub fn ensure_drone_thing(ctx: &mut Ctx<'_>, hull: ObjId, slot: u8) -> Result<ObjId, String> {
    if let Some(existing) = ctx
        .w
        .module_state::<super::tick::SpaceBubbleBook>()
        .and_then(|book| book.cluster_sessions.get(&hull))
        .and_then(|s| s.drone_things.get(&slot).copied())
    {
        return Ok(existing);
    }
    let saved = ctx.actor;
    ctx.actor = mush_core::GOD;
    let created = ctx
        .w
        .create(
            ObjKind::Thing,
            &format!("Hull #{} Collector Drone {}", hull.0, slot),
            mush_core::GOD,
            Some(hull),
        )
        .map_err(|e| e.to_string());
    if let Ok(thing) = created {
        let _ = ctx.w.set_flag(thing, Flags::DARK, true);
    }
    ctx.actor = saved;
    let thing = created?;
    if let Some(session) = ctx
        .w
        .module_state_mut::<super::tick::SpaceBubbleBook>()
        .and_then(|book| book.cluster_sessions.get_mut(&hull))
    {
        session.drone_things.insert(slot, thing);
    }
    Ok(thing)
}

/// Compute the next unopened [`SystemOpening`] for an orbital cluster clerk (`SiteOpener::next_opening`).
#[must_use]
pub fn next_cluster_opening(
    engine: &Engine,
    deposits: &StateFamilyHandle<OreDepositId, OreDeposit>,
    actor: ObjId,
) -> Option<SystemOpening> {
    let w = &engine.world;
    let book = w.module_state::<super::tick::SpaceBubbleBook>()?;
    let issuer_cid = CanonicalId::new("freeport_registry").ok()?;
    let jurisdiction = org_principal(w, &issuer_cid)?;
    let state = engine.action_state();

    for site_state in book.cluster_sites.values() {
        if let Some((&(letter, family), _)) = site_state
            .deposit_clerks
            .iter()
            .find(|(_, id)| **id == actor)
        {
            let cluster_cid = site_state.cluster.canonical_id()?;
            let spec = site_state
                .cluster
                .rocks()
                .iter()
                .find(|r| r.letter == letter)?;
            let place = *site_state.blocks.get(&letter)?;
            let vault = *site_state.vaults.get(&letter)?;
            let rec = receipt(&cluster_cid, letter, family);
            let deposit_id = mush_industry::derive_system_deposit_id(&rec).ok()?;
            if matches!(state.get(deposits, &deposit_id), Ok(None)) {
                let good = CanonicalId::new(ore_good(family)).ok()?;
                return Some(SystemOpening {
                    receipt: SmolStr::new(&rec),
                    place,
                    commodity: good,
                    grade: ORE_DEPOSIT_GRADE,
                    accessibility: ORE_DEPOSIT_ACCESSIBILITY,
                    grade_class: SURFACE_BAND.to_owned(),
                    reserve_class: SURFACE_BAND.to_owned(),
                    curve: vec![CurvePoint {
                        cumulative: 0,
                        per_cycle: 1,
                    }],
                    endpoint: mush_inventory::EndpointRef(vault),
                    reserve: spec.gross_t,
                    external: SmolStr::new_static(SURFACE_PROVIDER),
                    jurisdiction: Some(jurisdiction),
                });
            }
            return None;
        }
    }

    let site_state = book.cluster_sites.values().find(|s| s.clerk == actor)?;
    let cluster_cid = site_state.cluster.canonical_id()?;

    for spec in site_state.cluster.rocks() {
        let place = *site_state.blocks.get(&spec.letter)?;
        let vault = *site_state.vaults.get(&spec.letter)?;
        for &family in spec.spectral.families() {
            let rec = receipt(&cluster_cid, spec.letter, family);
            let deposit_id = mush_industry::derive_system_deposit_id(&rec).ok()?;
            if matches!(state.get(deposits, &deposit_id), Ok(None)) {
                let good = CanonicalId::new(ore_good(family)).ok()?;
                return Some(SystemOpening {
                    receipt: SmolStr::new(&rec),
                    place,
                    commodity: good,
                    grade: ORE_DEPOSIT_GRADE,
                    accessibility: ORE_DEPOSIT_ACCESSIBILITY,
                    grade_class: SURFACE_BAND.to_owned(),
                    reserve_class: SURFACE_BAND.to_owned(),
                    curve: vec![CurvePoint {
                        cumulative: 0,
                        per_cycle: 1,
                    }],
                    endpoint: mush_inventory::EndpointRef(vault),
                    reserve: spec.gross_t,
                    external: SmolStr::new_static(SURFACE_PROVIDER),
                    jurisdiction: Some(jurisdiction),
                });
            }
        }
    }
    None
}

/// Compute the next [`TenureAct`] for an orbital cluster clerk (seeding `pallas_syndicate` claims)
/// or a licensed pilot with a pending [`ClusterTenureOrder`] (`+mine/stake`, `+mine/contest`).
#[must_use]
pub fn next_cluster_tenure(engine: &Engine, actor: ObjId) -> Option<TenureAct> {
    let w = &engine.world;
    let now = w.world_ic_now().at;
    let book = w.module_state::<super::tick::SpaceBubbleBook>()?;
    let handles = w.module_state::<crate::port::families::PortHandles>()?;
    let industry = w.module_state::<mush_industry::IndustryHandles>()?;
    let state = engine.action_state();
    let issuer_cid = CanonicalId::new("freeport_registry").ok()?;
    let issuer = org_principal(w, &issuer_cid)?;

    // 1. Cluster block place or cluster clerk seeding authored held blocks (`pallas_syndicate`).
    for site_state in book.cluster_sites.values() {
        let matched_letter = site_state
            .blocks
            .iter()
            .find(|(_, id)| **id == actor)
            .map(|(&l, _)| l);
        if matched_letter.is_none() && site_state.clerk != actor {
            continue;
        }
        let cluster_cid = site_state.cluster.canonical_id()?;
        let term_at = now.checked_add(crate::surface::tenure::SEEDED_TERM_IC_SECS)?;
        for spec in site_state.cluster.rocks() {
            if let Some(only_letter) = matched_letter
                && spec.letter != only_letter
            {
                continue;
            }
            let (Tenure::Held, Some(holder_str)) = (spec.tenure, spec.holder_org) else {
                continue;
            };
            let place = *site_state.blocks.get(&spec.letter)?;
            // Ensure all deposits on this rock are open before seeding tenure.
            let mut opened = Vec::new();
            for &family in spec.spectral.families() {
                let rec = receipt(&cluster_cid, spec.letter, family);
                let dep_id = mush_industry::derive_system_deposit_id(&rec).ok()?;
                let dep = state.get(&industry.0.deposits, &dep_id).ok()??;
                opened.push(dep.id());
            }
            let key = BlockKey {
                site: cluster_cid.clone(),
                letter: spec.letter,
            };
            let existing = state.get(&handles.families.claims, &key).ok()?.cloned();
            if existing.as_ref().is_some_and(|r| r.claim.is_some()) {
                continue;
            }
            let holder_cid = CanonicalId::new(holder_str).ok()?;
            let holder = org_principal(w, &holder_cid).unwrap_or_else(|| issuer.clone());
            let game = TenureGame::new(&cluster_cid, spec.letter, "seed_held");
            return Some(TenureAct {
                receipt: SmolStr::new(format!(
                    "{}{cluster_cid}/{}/{}",
                    crate::port::site::CLAIM_SEED_KEY_PREFIX,
                    spec.letter.to_ascii_lowercase(),
                    issuer_cid
                )),
                actor,
                place,
                kind: TenureKind::Seed {
                    issuer,
                    holder,
                    term_at,
                    deposits: opened,
                    contest: None,
                },
                summary: crate::port::claims::TENURE_SUMMARY.to_owned(),
                game: serde_json::to_value(&game).ok()?,
            });
        }
        return None;
    }

    // 2. Licensed player's pending stake or contest order (`+mine/stake`, `+mine/contest`).
    let order = book.cluster_tenure_orders.get(&actor)?;
    let site_state = book.cluster_sites.get(&order.cluster)?;
    let cluster_cid = order.cluster.canonical_id()?;
    let place = *site_state.blocks.get(&order.letter)?;
    let spec = order
        .cluster
        .rocks()
        .iter()
        .find(|r| r.letter == order.letter)?;
    let mut deposits = Vec::new();
    for &family in spec.spectral.families() {
        let rec = receipt(&cluster_cid, order.letter, family);
        let dep_id = mush_industry::derive_system_deposit_id(&rec).ok()?;
        if state.get(&industry.0.deposits, &dep_id).ok()?.is_some() {
            deposits.push(dep_id);
        }
    }
    let key = BlockKey {
        site: cluster_cid.clone(),
        letter: order.letter,
    };
    let existing = state.get(&handles.families.claims, &key).ok()?.cloned();
    let reads = crate::port::site_map::EngineReads(engine);
    let mut existing_claims: Vec<(OreDepositId, ClaimRecord)> = Vec::new();
    for &dep_id in &deposits {
        if let Ok(list) = crate::port::site_map::SurfaceReads::claims(&reads, dep_id) {
            for c in list {
                if c.issuer() == &issuer {
                    existing_claims.push((dep_id, c));
                }
            }
        }
    }

    let holder = PrincipalId::Object(actor);
    let account = crate::port::licence::gateway_account(w, actor);
    let term_at = now.checked_add(CLAIM_TERM_IC_SECS)?;

    let (kind, kind_word) = if order.is_contest {
        let record = existing?;
        let old_claim = record.claim?;
        if !old_claim.live(now) || !old_claim.player_held() || old_claim.holder == holder {
            return None;
        }
        if !contest_open(old_claim.registered_ic, now) {
            return None;
        }
        let revs: Vec<ClaimRev> = existing_claims
            .iter()
            .map(|(_, c)| ClaimRev {
                claim: c.id(),
                revision: c.revision(),
            })
            .collect();
        (
            TenureKind::Contest {
                claims: revs,
                to: holder,
                discovered_at: order.discovered_ic,
                term_at,
            },
            "contest",
        )
    } else {
        let reopen = existing
            .as_ref()
            .and_then(|r| r.claim.as_ref())
            .is_some_and(|c| c.lapsed_ic.is_some());
        if existing
            .as_ref()
            .and_then(|r| r.claim.as_ref())
            .is_some_and(|c| c.lapsed_ic.is_none())
        {
            return None;
        }
        let mut file = Vec::new();
        let mut take_over = Vec::new();
        for &dep_id in &deposits {
            if let Some((_, c)) = existing_claims.iter().find(|(d, _)| *d == dep_id) {
                take_over.push(ClaimRev {
                    claim: c.id(),
                    revision: c.revision(),
                });
            } else {
                file.push(dep_id);
            }
        }
        (
            TenureKind::Stake {
                issuer,
                holder,
                term_at,
                discovered_at: order.discovered_ic,
                reopen,
                file,
                take_over,
            },
            "stake",
        )
    };

    let mut game = TenureGame::new(&cluster_cid, order.letter, kind_word);
    game.declared = order.declared;
    game.account = account;
    Some(TenureAct {
        receipt: SmolStr::new(&order.receipt),
        actor,
        place,
        kind,
        summary: crate::port::claims::TENURE_SUMMARY.to_owned(),
        game: serde_json::to_value(&game).ok()?,
    })
}

/// Build the next `SiteWorkProvider` offer row for a completed space mining extraction cycle
/// (`tacsim_site_work`).
#[must_use]
pub fn next_space_mine_work_row(
    engine: &Engine,
    pockets: &StateFamilyHandle<OreDepositId, PocketCursor>,
    clerk: ObjId,
) -> Option<(String, Value)> {
    let w = &engine.world;
    let book = w.module_state::<super::tick::SpaceBubbleBook>()?;
    let state = engine.action_state();
    let draw = book.pending_space_mine_draws.iter().find(|d| {
        if d.clerk != clerk {
            return false;
        }
        let prior_drawn = state
            .get(pockets, &d.deposit)
            .ok()
            .flatten()
            .and_then(PocketCursor::drawn_total)
            .unwrap_or(0);
        prior_drawn < d.expected_total_t
    })?;
    let prior = state.get(pockets, &draw.deposit).ok()?.cloned();
    let updated = build_updated_pocket_cursor(draw.deposit, prior.as_ref(), draw.worked_t)?;
    let value = json!({
        "deposit": cursor_key_text(&draw.deposit),
        "commodity": draw.commodity.as_str(),
        "from": draw.from_vault.0,
        "to": draw.to_vault.0,
        "worked_t": draw.worked_t,
        "cursor": updated.rows,
        "clerk": draw.clerk.0,
        "space_mine": true,
    });
    Some((draw.receipt.clone(), value))
}

/// Read the total tonnes currently stored across all `stow.ore` and bulk goods in `hull`'s hold vault.
#[must_use]
pub fn hull_hold_used_tonnes(ctx: &Ctx<'_>, hull: ObjId) -> u64 {
    let Some(view) = crate::port::ship::hull_view(ctx.w, hull) else {
        return 0;
    };
    let Some(hold_vault) = crate::port::ship::hold_vault(ctx.w, &view) else {
        return 0;
    };
    let mut total = 0_u64;
    for family in OreFamily::ALL {
        if let Ok(good) = CanonicalId::new(ore_good(family))
            && let Ok(qty) = crate::port_tick::read_pool(
                ctx,
                mush_inventory::record::EndpointRef(hold_vault),
                &good,
            )
        {
            total = total.saturating_add(qty);
        }
    }
    total
}

/// Read the remaining `OreDeposit` pool tonnes in `rock_vault` for `family`.
#[must_use]
pub fn rock_pool_tonnes(ctx: &Ctx<'_>, rock_vault: VaultId, family: OreFamily) -> u64 {
    let Ok(good) = CanonicalId::new(ore_good(family)) else {
        return 0;
    };
    crate::port_tick::read_pool(ctx, mush_inventory::record::EndpointRef(rock_vault), &good)
        .unwrap_or(0)
}

/// Format the live tenure description of `(cluster, letter)` for `viewer`.
#[must_use]
pub fn cluster_rock_tenure_word(
    ctx: &Ctx<'_>,
    cluster: OrbitalClusterId,
    spec: &ClusterRockSpec,
    viewer: ObjId,
    now_ic: i64,
) -> String {
    if spec.tenure == Tenure::Commons {
        return "commons (Freeport Registry)".to_owned();
    }
    let Some(cluster_cid) = cluster.canonical_id() else {
        return "open".to_owned();
    };
    if let Some(handles) = ctx.w.module_state::<crate::port::families::PortHandles>() {
        let key = BlockKey {
            site: cluster_cid,
            letter: spec.letter,
        };
        if let Ok(Some(record)) = ctx.action_state(&handles.families.claims, &key)
            && let Some(claim) = record.claim.as_ref()
            && claim.live(now_ic)
        {
            if claim.holder == PrincipalId::Object(viewer) {
                return format!(
                    "your claim (until {}, contest window {}d)",
                    crate::port::licence::ic_stamp(claim.term_end_ic),
                    CONTEST_WINDOW_IC_SECS / 86_400
                );
            }
            return format!(
                "held by {}",
                crate::port::claims::holder_name(ctx.w, &claim.holder)
            );
        }
    }
    if let (Tenure::Held, Some(org)) = (spec.tenure, spec.holder_org) {
        return format!("held by {org}");
    }
    "open".to_owned()
}

/// Step all laid-up orbital clusters and active cluster mining sessions on `space_tick`.
pub fn step_clusters_on_tick(ctx: &mut Ctx<'_>, now: ClockStamp) {
    let Some(industry) = ctx
        .w
        .module_state::<mush_industry::IndustryHandles>()
        .map(|h| h.0.clone())
    else {
        return;
    };
    let Some(handles) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned()
    else {
        return;
    };
    let Some(pockets) = Some(handles.families.pockets.clone()) else {
        return;
    };

    // 1. Advance active cluster sessions (approach braking, timed laser cycles, and drone sorties)
    //    so any draws finishing on this tick are queued before the cluster clerks submit work.
    let hulls: Vec<ObjId> = ctx
        .w
        .module_state::<super::tick::SpaceBubbleBook>()
        .map(|b| b.cluster_sessions.keys().copied().collect())
        .unwrap_or_default();

    for hull in hulls {
        let Some((session_snapshot, site_snapshot)) = ctx
            .w
            .module_state::<super::tick::SpaceBubbleBook>()
            .and_then(|b| {
                let s = b.cluster_sessions.get(&hull)?.clone();
                let site = b.cluster_sites.get(&s.cluster)?.clone();
                Some((s, site))
            })
        else {
            continue;
        };

        let dt_ic_s = now.at.saturating_sub(session_snapshot.last_ic_s).max(1);
        let mut next_pos = session_snapshot.ship_pos_m;
        let mut next_vel = session_snapshot.ship_vel_mmps;
        let mut burn_dv_mps: u64 = 0;

        // If approaching a rock (`+helm/approach <rock>`), close to 600 m standoff and brake
        // relative velocity to `< 2,000 mm/s` (`2.0 m/s`, well inside the `< 5.0 m/s` gate).
        if let Some(letter) = session_snapshot.approaching_rock
            && let Some(spec) = session_snapshot
                .cluster
                .rocks()
                .iter()
                .find(|r| r.letter == letter)
        {
            let target_pos = [spec.pos_m[0] - 600, spec.pos_m[1], spec.pos_m[2]];
            let target_vel = spec.vel_mmps;
            let dvx = (next_vel[0] - target_vel[0]).abs();
            let dvy = (next_vel[1] - target_vel[1]).abs();
            let dvz = (next_vel[2] - target_vel[2]).abs();
            burn_dv_mps = ((dvx + dvy + dvz) as u64).div_ceil(1_000).max(2);
            next_pos = target_pos;
            next_vel = [target_vel[0] + 1_200, target_vel[1], target_vel[2]];
        } else if let Some(letter) = session_snapshot.matched_rock
            && let Some(spec) = session_snapshot
                .cluster
                .rocks()
                .iter()
                .find(|r| r.letter == letter)
        {
            next_vel = spec.vel_mmps;
        } else {
            for axis in 0..3 {
                let dpos = (i128::from(next_vel[axis]) * i128::from(dt_ic_s.min(10))) / 1_000;
                next_pos[axis] = next_pos[axis].saturating_add(dpos as i64);
            }
        }

        if burn_dv_mps > 0 {
            let _ = super::commands::debit_hull_propellant_for_dv(ctx, hull, burn_dv_mps, true);
        }

        let hold_used_t = hull_hold_used_tonnes(ctx, hull);
        let mut hold_room_t = DRAY_P_HOLD_CAPACITY_T.saturating_sub(hold_used_t);
        let hold_vault = crate::port::ship::hull_view(ctx.w, hull)
            .and_then(|v| crate::port::ship::hold_vault(ctx.w, &v));
        let cluster_cid = session_snapshot.cluster.canonical_id();

        let mut completed_draws: Vec<(char, OreFamily, u64, ObjId, Option<ExcursionId>)> =
            Vec::new();

        // Check active laser cycle completion.
        let mut remaining_laser = session_snapshot.active_laser.clone();
        if let Some(laser) = &session_snapshot.active_laser
            && now.at >= laser.due_ic
        {
            remaining_laser = None;
            if let Some(rock_vault) = site_snapshot.vaults.get(&laser.rock_letter).copied() {
                let avail_t = rock_pool_tonnes(ctx, rock_vault, laser.family);
                let take_t = laser.yield_t.min(hold_room_t).min(avail_t);
                if take_t > 0 {
                    hold_room_t = hold_room_t.saturating_sub(take_t);
                    completed_draws.push((
                        laser.rock_letter,
                        laser.family,
                        take_t,
                        laser.operator,
                        None,
                    ));
                }
            }
        }

        // Check active collector-drone sorties completion.
        let mut remaining_drones = Vec::new();
        for drone in &session_snapshot.active_drones {
            if now.at >= drone.due_ic {
                if let Some(rock_vault) = site_snapshot.vaults.get(&drone.rock_letter).copied() {
                    let avail_t = rock_pool_tonnes(ctx, rock_vault, drone.family);
                    let take_t = drone.yield_t.min(hold_room_t).min(avail_t);
                    if take_t > 0 {
                        hold_room_t = hold_room_t.saturating_sub(take_t);
                        completed_draws.push((
                            drone.rock_letter,
                            drone.family,
                            take_t,
                            drone.operator,
                            Some(drone.excursion_id),
                        ));
                    }
                }
            } else {
                remaining_drones.push(drone.clone());
            }
        }

        // Coalesce completed draws per `(rock_letter, family)` so one `tacsim_site_work` bundle
        // moves the combined tonnes cleanly!
        let mut coalesced: BTreeMap<(char, OreFamily), (u64, ObjId, Option<ExcursionId>)> =
            BTreeMap::new();
        for (letter, family, tonnes, op, exc) in completed_draws {
            let entry = coalesced.entry((letter, family)).or_insert((0, op, exc));
            entry.0 = entry.0.saturating_add(tonnes);
            if entry.2.is_none() && exc.is_some() {
                entry.2 = exc;
            }
        }

        if let (Some(hold_vault), Some(cluster_cid)) = (hold_vault, cluster_cid) {
            let mut new_draws = Vec::new();
            for ((letter, family), (worked_t, operator, excursion)) in coalesced {
                if worked_t == 0 {
                    continue;
                }
                let Some(&from_vault) = site_snapshot.vaults.get(&letter) else {
                    continue;
                };
                let rec_base = receipt(&cluster_cid, letter, family);
                let Ok(deposit) = mush_industry::derive_system_deposit_id(&rec_base) else {
                    continue;
                };
                let Ok(commodity) = CanonicalId::new(ore_good(family)) else {
                    continue;
                };
                let landed_drawn = ctx
                    .action_state(&pockets, &deposit)
                    .ok()
                    .flatten()
                    .and_then(PocketCursor::drawn_total)
                    .unwrap_or(0);
                let queued_drawn: u64 = ctx
                    .w
                    .module_state::<super::tick::SpaceBubbleBook>()
                    .map_or(0, |b| {
                        b.pending_space_mine_draws
                            .iter()
                            .filter(|d| d.deposit == deposit && d.expected_total_t > landed_drawn)
                            .map(|d| d.worked_t)
                            .sum()
                    });
                let expected_total_t = landed_drawn
                    .saturating_add(queued_drawn)
                    .saturating_add(worked_t);
                new_draws.push((
                    rec_base,
                    deposit,
                    commodity,
                    from_vault,
                    worked_t,
                    expected_total_t,
                    operator,
                    excursion,
                ));
            }
            if let Some(book) = ctx.w.module_state_mut::<super::tick::SpaceBubbleBook>() {
                for (
                    rec_base,
                    deposit,
                    commodity,
                    from_vault,
                    worked_t,
                    expected_total_t,
                    operator,
                    excursion,
                ) in new_draws
                {
                    book.next_space_mine_draw_seq = book.next_space_mine_draw_seq.saturating_add(1);
                    let seq = book.next_space_mine_draw_seq;
                    let draw_receipt = format!("{rec_base}/draw/{seq}");
                    book.pending_space_mine_draws.push(PendingSpaceMineDraw {
                        receipt: draw_receipt,
                        cluster: session_snapshot.cluster,
                        clerk: site_snapshot.clerk,
                        hull,
                        operator,
                        deposit,
                        commodity,
                        from_vault,
                        to_vault: hold_vault,
                        worked_t,
                        expected_total_t,
                        excursion,
                    });
                }
            }
        }

        if let Some(book) = ctx.w.module_state_mut::<super::tick::SpaceBubbleBook>()
            && let Some(session) = book.cluster_sessions.get_mut(&hull)
        {
            session.ship_pos_m = next_pos;
            session.ship_vel_mmps = next_vel;
            if session.approaching_rock.is_some() {
                session.approaching_rock = None;
            }
            session.active_laser = remaining_laser;
            session.active_drones = remaining_drones;
            session.last_ic_s = now.at;
        }
    }

    // 2. For each laid-up cluster, let its dedicated deposit openers open any unopened
    //    `OreDeposit`, let its block places seed authored held tenure (`pallas_syndicate`),
    //    and let its cluster clerk submit any pending `tacsim_site_work` draw.
    let sites: Vec<ClusterSiteState> = ctx
        .w
        .module_state::<super::tick::SpaceBubbleBook>()
        .map(|b| b.cluster_sites.values().cloned().collect())
        .unwrap_or_default();

    let mut fire = crate::port_tick::Fire::new(BTreeMap::new());
    for site in &sites {
        let Some(cluster_cid) = site.cluster.canonical_id() else {
            continue;
        };

        // 2a. Open any unopened deposits via their dedicated `deposit_clerks`.
        let mut opened_all = true;
        for (&(letter, family), &dep_clerk) in &site.deposit_clerks {
            let rec = receipt(&cluster_cid, letter, family);
            let Ok(dep_id) = mush_industry::derive_system_deposit_id(&rec) else {
                continue;
            };
            if matches!(ctx.action_state(&industry.deposits, &dep_id), Ok(None)) {
                opened_all = false;
                if !ctx.authoritative_pending(dep_clerk)
                    && let Some(action) =
                        mush_industry::action_ref(ctx.reg, mush_industry::IndustryVerb::DepositOpen)
                    && let Ok(key) = mush_core::IdempotencyKeyText::parse_canonical(format!(
                        "{}{rec}",
                        crate::port::site::SITE_OPEN_KEY_PREFIX
                    ))
                {
                    let _ = fire.as_principal(ctx, dep_clerk, dep_clerk, |ctx| {
                        ctx.submit_authoritative(
                            action,
                            mush_core::PerceivedRef::SelfActor,
                            Vec::new(),
                            json!({ "receipt": rec }),
                            key,
                        );
                    });
                }
            }
        }

        // 2b. Once deposits are open, seed authored held claims (`pallas_syndicate`) via each
        //     block's own `place` `Thing` (`site.blocks[&spec.letter]`).
        if opened_all {
            let issuer_cid = CanonicalId::new("freeport_registry").ok();
            for spec in site.cluster.rocks() {
                if !matches!((spec.tenure, spec.holder_org), (Tenure::Held, Some(_))) {
                    continue;
                }
                let Some(&block_place) = site.blocks.get(&spec.letter) else {
                    continue;
                };
                if ctx.authoritative_pending(block_place) {
                    continue;
                }
                let key = BlockKey {
                    site: cluster_cid.clone(),
                    letter: spec.letter,
                };
                if matches!(
                    ctx.action_state(&handles.families.claims, &key),
                    Ok(Some(record)) if record.claim.is_some()
                ) {
                    continue;
                }
                if let Some(issuer_cid) = &issuer_cid
                    && let Some(action) =
                        mush_industry::action_ref(ctx.reg, mush_industry::IndustryVerb::ClaimTenure)
                {
                    let seed_receipt = format!(
                        "{}{cluster_cid}/{}/{}",
                        crate::port::site::CLAIM_SEED_KEY_PREFIX,
                        spec.letter.to_ascii_lowercase(),
                        issuer_cid
                    );
                    if let Ok(idem_key) =
                        mush_core::IdempotencyKeyText::parse_canonical(seed_receipt.clone())
                    {
                        let _ = fire.as_principal(ctx, block_place, block_place, |ctx| {
                            ctx.submit_authoritative(
                                action,
                                mush_core::PerceivedRef::SelfActor,
                                Vec::new(),
                                json!({ "receipt": seed_receipt }),
                                idem_key,
                            );
                        });
                    }
                }
            }
        }

        // 2c. Prune any `pending_space_mine_draws` whose `PocketCursor` has already landed, and
        //     submit the next pending `tacsim_site_work` draw via `site.clerk`.
        let clerk = site.clerk;
        let pending_for_clerk: Vec<PendingSpaceMineDraw> = ctx
            .w
            .module_state::<super::tick::SpaceBubbleBook>()
            .map(|b| {
                b.pending_space_mine_draws
                    .iter()
                    .filter(|d| d.clerk == clerk)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut landed_receipts = Vec::new();
        let mut next_draw = None;
        for d in pending_for_clerk {
            let drawn = ctx
                .action_state(&pockets, &d.deposit)
                .ok()
                .flatten()
                .and_then(PocketCursor::drawn_total)
                .unwrap_or(0);
            if drawn >= d.expected_total_t {
                landed_receipts.push(d.receipt);
            } else if next_draw.is_none() {
                next_draw = Some(d);
            }
        }
        if !landed_receipts.is_empty()
            && let Some(book) = ctx.w.module_state_mut::<super::tick::SpaceBubbleBook>()
        {
            let mut idx = 0;
            while idx < book.pending_space_mine_draws.len() {
                if landed_receipts.contains(&book.pending_space_mine_draws[idx].receipt) {
                    let finished = book.pending_space_mine_draws.remove(idx);
                    book.completed_space_mine_draws.push(finished);
                } else {
                    idx += 1;
                }
            }
        }
        if ctx.authoritative_pending(clerk) {
            continue;
        }
        if let Some(draw) = next_draw
            && let Some(action) = handles.site_work.clone()
            && let Ok(key) = mush_core::IdempotencyKeyText::parse_canonical(format!(
                "{}{}",
                crate::port::site::SITE_WORK_KEY_PREFIX,
                draw.receipt
            ))
        {
            let receipt_str = draw.receipt.clone();
            let _ = fire.as_principal(ctx, clerk, clerk, |ctx| {
                ctx.submit_authoritative(
                    action,
                    mush_core::PerceivedRef::SelfActor,
                    Vec::new(),
                    json!({ "receipt": receipt_str }),
                    key,
                );
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn velocity_match_gate_uses_3d_euclidean_mmps_and_rejects_per_axis_truncation_exploit() {
        let rock_pos = [1_000, 0, 0];
        let rock_vel = [0, 0, 0];

        // 4.4 m/s per axis (`[4_400, 4_400, 4_400] mm/s`) has 3D speed ~7.62 m/s;
        // per-axis integer truncation to 4 m/s used to pass (`35a` §5 #11), must fail!
        let exploit_vel = [4_400, 4_400, 4_400];
        assert!(
            check_velocity_match_gate([500, 0, 0], exploit_vel, rock_pos, rock_vel).is_err(),
            "3D speed ~7.62 m/s must be rejected by the 5.0 m/s Euclidean gate"
        );

        // Outside 2,000 m range must fail.
        assert!(
            check_velocity_match_gate([3_500, 0, 0], [1_000, 0, 0], rock_pos, rock_vel).is_err(),
            "range > 2,000 m must be rejected"
        );

        // Within 2,000 m and 3D relative speed < 5,000 mm/s passes.
        let (dist_m, speed_mmps) =
            check_velocity_match_gate([400, 0, 0], [2_000, 2_000, 1_000], rock_pos, rock_vel)
                .unwrap();
        assert_eq!(dist_m, 600);
        assert_eq!(speed_mmps, 3_000);
    }

    #[test]
    fn ore_specific_volumes_and_dray_p_hold_match_goods_and_unit_spec() {
        assert_eq!(ore_specific_volume_l_per_t(OreFamily::Volatiles), 1_000);
        assert_eq!(
            ore_specific_volume_l_per_t(OreFamily::CarbonFeedstock),
            1_200
        );
        assert_eq!(ore_specific_volume_l_per_t(OreFamily::Metals), 500);
        assert_eq!(
            ore_specific_volume_l_per_t(OreFamily::StrategicElements),
            600
        );
        assert_eq!(DRAY_P_HOLD_CAPACITY_KG, 86_000);
        assert_eq!(DRAY_P_HOLD_CAPACITY_T, 86);
    }

    #[test]
    fn pocket_cursor_builder_packs_24t_rows_and_passes_check() {
        let dep = OreDepositId(0x1234_5678_9abc_def0);
        let c1 = build_updated_pocket_cursor(dep, None, 20).unwrap();
        assert_eq!(c1.drawn_total(), Some(20));
        assert_eq!(c1.rows.len(), 1);
        c1.check().unwrap();

        let c2 = build_updated_pocket_cursor(dep, Some(&c1), 15).unwrap();
        assert_eq!(c2.drawn_total(), Some(35));
        assert_eq!(c2.rows.len(), 2);
        assert_eq!(c2.rows[0].drawn_t, 24);
        assert_eq!(c2.rows[1].drawn_t, 11);
        c2.check().unwrap();
    }
}
