//! **The tender** (doc 41 cut RS.1; Q-41-1..4, Q-38-16, Q-36-35, Q-36-36; §4.1).
//!
//! A unit whose definition carries `logistics.tender` is a support rig: its stores compartment
//! (`logistics.tender.stores`, `stow.bulk`) carries munitions, structural alloys and yard
//! fittings, and its rig record carries a **bale** of its own (`RigRecord.bale`, the twin of
//! `BayHull.bale`, doc 39 IN.1's accounting): whole tonnes debited from the stores, kilograms
//! consumed by the work. Its verbs, first words on `+tv` like the tow's:
//!
//! - **`+tv/<n> stores [load <qty> <good>|open] [go]`**: the stores' contents and the bale; a
//!   load from the carrier's hold while the rig stands in its bay; and the bale opened before
//!   launch, a whole tonne of each line debited from the stores into it.
//! - **`+tv/<n> unjam <unit>[=<part>] [go]`**, **`stabilize <unit>[=<section>] [<points>|all]
//!   [go]`**, **`rearm <unit>[=<part>] [go]`**: one timed act of the tow's action
//!   (`tacsim_surface_tow`, op `tend`) on a unit within the tow's reach (one cell, diagonals
//!   included), through mush-tactics' field-engineering library ([`FieldEngineeringBook`]), the
//!   tender registered from its compartment ([`register_tender`]).
//!
//! **Time** is the tariff's base for the same work at [`TENDER_TIME_FACTOR`] (the workshop 1, a
//! carrier bay `CARRIER_TIME_FACTOR` 2, the tender 3; Q-41-3): an unjam is a component repair, a
//! stabilize a structural point each, a rearm a component's rearm. **The draw** (Q-41-2): an
//! unjam the library's fittings, a stabilize the tariff's kilograms a point
//! ([`fittings_kg_per_point`], alloys none until doc 39 IN.8 gives each design its per-point
//! figures), both through the bale; a rearm whole units of the mount's munitions good from the
//! stores (`div_ceil`, the bay's rule). **No money** moves (Q-41-4): the library's
//! `billed_credits` is computed and dropped.
//!
//! **The field** (Q-CF-30, Q-41-3): no tend starts on a promoted site; an act in flight when a
//! field opens over its site is **paused** (its time left kept) and resumes on the tick that
//! finds the site settled. The tender drives or tends, never both (the act is a tow act: the
//! drive's gate refuses a tug with one in flight). How a coupled pair leaves a running field is
//! doc 40's Q-40-25 (FR.5's), which this module only reads.
//!
//! **The finish** is the load's precedent (`place_in_bay`): the tick, finding the act due on an
//! unpromoted site, runs the library on the target's unit and applies the change
//! (`mush_tactics::module::apply_plan`), then submits the finish, which debits the stores and
//! writes the bale in its own transaction. A target past the work by then (unjammed meanwhile,
//! gone) finishes with nothing drawn.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::VaultId;
use mush_inventory::EndpointRef;
use mush_tactics::TacticalUnit;
use mush_tactics::field_ops::{
    ComponentCondition, DEFAULT_UNJAM_SPARE_PARTS_KG, FieldEngineeringBook, FieldPose,
    FieldTenderState, OperationalState, SectionCondition, UnitAnatomyProfile,
};
use mush_tactics::unit::{CellAddr, CellId, ComponentId, SectionId};
use serde::{Deserialize, Serialize};

use crate::port::bay_work::{Bale, KG_PER_T};
use crate::store::TacsimStore;
use crate::surface::act::{self, Where};
use crate::surface::excursion::{cells_apart, site_cell};
use crate::surface::tow::{self, TowOrder, TowStage};

/// **The tender takes three times the workshop's time** for the same job (Q-41-3, provisional):
/// one scale across the three facility classes.
pub const TENDER_TIME_FACTOR: i64 = 3;

/// The stores' good for structural alloys.
pub const ALLOYS: &str = "structural_alloys";
/// The stores' good for yard fittings.
pub const FITTINGS: &str = "industrial_parts";

/// The `+tv` words the tender adds (first words, as the tow's).
pub const TENDER_WORDS: [&str; 4] = ["rearm", "stabilize", "stores", "unjam"];

/// The usage line.
pub const TEND_USAGE: &str = "Usage: +tv/<n> unjam <unit>[=<part>] [go], +tv/<n> stabilize <unit>[=<section>] [<points>|all] [go], +tv/<n> rearm <unit>[=<part>] [go], +tv/<n> stores [go], +tv/<n> stores load <qty> <good> [go] or +tv/<n> stores open [go].";

/// The rig is no tender: `{n}`.
pub const NOT_A_TENDER: &str =
    "Rig {n} is no tender: it carries no stores to tend from. Occator leases one: +tv/lease tender.";

/// Nothing in reach: `{n}`, `{name}`.
pub const TEND_NOT_IN_REACH: &str = "The {name} is not within a cell of rig {n}.";

/// The unit needs nothing of this kind: `{n}`, `{name}`, `{what}`.
pub const TEND_NOTHING: &str = "The {name} has {what}; rig {n} has nothing to do.";

/// The preview: `{n}`, `{name}`, `{work}`, `{draw}`, `{min}`, `{verb}`, `{arg}`.
pub const TEND_PREVIEW: &str = "Rig {n} can {work} on the {name}, drawing {draw}; it takes about {min} IC minutes. +tv/{n} {verb} {arg} go";

/// Filed: `{n}`, `{name}`, `{work}`, `{min}`.
pub const TEND_FILED: &str =
    "Rig {n} sets to work on the {name}: {work}, about {min} IC minutes. It does nothing else until it is done.";

/// The stores cannot cover the work: `{n}`, `{need}`, `{have}`.
pub const TEND_SHORT: &str =
    "Rig {n}'s stores cannot cover it: the work draws {need}, and the stores and the bale hold {have}.";

/// A rearm the stores hold no rounds for: `{n}`, `{part}`, `{good}`.
pub const NO_ROUNDS_ABOARD: &str = "Rig {n} has no rounds aboard for {part} ({good}).";

/// A target carried in tow: `{name}`.
pub const TEND_IN_TOW: &str = "The {name} is in tow; nobody tends a hull carried in tow.";

/// The library's refusal: `{n}`, `{name}`, `{why}`.
pub const TEND_REFUSED: &str = "Rig {n} cannot work on the {name}: {why}.";

/// Done: `{n}`, `{name}`, `{work}`.
pub const TENDED: &str = "Rig {n} has finished on the {name}: {work}.";

/// A field opened over the pair: `{n}`, `{name}`.
pub const TEND_PAUSED: &str = "Rig {n} stops work on the {name}: a field has opened over the site. It takes the work up again when the field settles.";

/// The field settled: `{n}`, `{name}`, `{min}`.
pub const TEND_RESUMED: &str =
    "Rig {n} takes up its work on the {name} again; about {min} IC minutes are left.";

/// The finish found nothing to do: `{n}`, `{name}`, `{why}`.
pub const TEND_LOST: &str = "Rig {n}'s work on the {name} came to nothing: {why}.";

/// The stores, read: `{n}`, `{load}`, `{kg}`, `{cap}`, `{bale}`.
pub const STORES_SHOW: &str = "Rig {n}'s stores hold {load} ({kg} of {cap} kg). {bale}";

/// The bale, open: `{alloys}`, `{fittings}`.
pub const BALE_OPEN: &str =
    "Its bale is open: {alloys} kg of structural alloys and {fittings} kg of yard fittings.";

/// The bale, unopened.
pub const BALE_SHUT: &str =
    "Its bale is unopened: +tv/<n> stores open takes a tonne of alloys and a tonne of fittings into it.";

/// The load preview: `{n}`, `{qty}`, `{good}`, `{carrier}`.
pub const STORES_LOAD_PREVIEW: &str =
    "Rig {n} can take {qty} {good} into its stores from the {carrier}'s hold. +tv/{n} stores load {qty} {good} go";

/// Loaded: `{n}`, `{qty}`, `{good}`.
pub const STORES_LOADED: &str = "Rig {n}'s stores take {qty} {good} from the hold.";

/// The open preview: `{n}`.
pub const STORES_OPEN_PREVIEW: &str = "Rig {n} can open its bale: a tonne of structural alloys and a tonne of yard fittings out of its stores, consumed by the kilogram. +tv/{n} stores open go";

/// Opened: `{n}`.
pub const STORES_OPENED: &str =
    "Rig {n}'s bale is open: a tonne of structural alloys and a tonne of yard fittings.";

/// The bale is open already: `{n}`.
pub const BALE_ALREADY: &str = "Rig {n}'s bale is open already; the work tops it up from the stores a tonne at a time.";

/// The stores lack a tonne of a line: `{n}`, `{good}`.
pub const STORES_NO_TONNE: &str = "Rig {n}'s stores hold no whole tonne of {good} to open.";

/// Not in its bay: `{n}`.
pub const STORES_NOT_IN_BAY: &str =
    "Rig {n} takes on stores only in its carrier's bay, from the carrier's hold.";

/// The stores have no room: `{n}`, `{qty}`, `{good}`.
pub const STORES_NO_ROOM: &str = "Rig {n}'s stores have no room for {qty} {good}.";

/// The hold has too little: `{qty}`, `{good}`, `{have}`.
pub const HOLD_SHORT: &str = "The hold carries {have} {good}, not {qty}.";

/// The stores take only bulk: `{good}`.
pub const STORES_NOT_BULK: &str = "The stores take munitions, alloys and fittings; not {good}.";

// ---- the work -----------------------------------------------------------------------------

/// **One tend act's work** (Q-38-16).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "work", rename_all = "snake_case", deny_unknown_fields)]
pub enum TendWork {
    /// Clear a jammed part.
    Unjam {
        /// The part.
        component: String,
    },
    /// Patch a section's internals.
    Stabilize {
        /// The section.
        section: String,
        /// The points asked for.
        points: i64,
    },
    /// Refill a mount's magazine from the stores.
    Rearm {
        /// The part.
        component: String,
        /// The munitions good that loads it.
        good: CanonicalId,
        /// Rounds the magazine is short.
        rounds: i64,
        /// Rounds one unit of the good loads.
        per_unit: u64,
    },
}

impl TendWork {
    /// The verb that files it.
    #[must_use]
    pub const fn verb(&self) -> &'static str {
        match self {
            Self::Unjam { .. } => "unjam",
            Self::Stabilize { .. } => "stabilize",
            Self::Rearm { .. } => "rearm",
        }
    }

    /// The work in words: `clear the jam in cannon`.
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            Self::Unjam { component } => format!("clear the jam in {component}"),
            Self::Stabilize { section, points } => {
                format!("patch {points} points of {section}")
            }
            Self::Rearm {
                component, rounds, ..
            } => format!("load {rounds} rounds into {component}"),
        }
    }

    /// **What it draws** (Q-41-2): the bale's kilograms, and units of a munitions good.
    #[must_use]
    pub fn need(&self) -> (Bale, Option<(CanonicalId, u64)>) {
        match self {
            Self::Unjam { .. } => (
                Bale {
                    alloys_kg: 0,
                    fittings_kg: DEFAULT_UNJAM_SPARE_PARTS_KG,
                },
                None,
            ),
            Self::Stabilize { points, .. } => (
                Bale {
                    alloys_kg: 0,
                    fittings_kg: u64::try_from(*points)
                        .unwrap_or(0)
                        .saturating_mul(fittings_kg_per_point()),
                },
                None,
            ),
            Self::Rearm {
                good,
                rounds,
                per_unit,
                ..
            } => (
                Bale::default(),
                Some((
                    good.clone(),
                    u64::try_from(*rounds)
                        .unwrap_or(0)
                        .div_ceil((*per_unit).max(1)),
                )),
            ),
        }
    }

    /// **Its time, IC seconds** (Q-41-3): the tariff's base at [`TENDER_TIME_FACTOR`].
    #[must_use]
    pub fn secs(&self) -> i64 {
        let Ok(tariff) = crate::definitions::workshop::workshop_tariff() else {
            return 0;
        };
        let base = match self {
            Self::Unjam { .. } => tariff.duration_ic_repair_component,
            Self::Stabilize { points, .. } => {
                (*points).saturating_mul(tariff.duration_ic_per_structural_point_repair)
            }
            Self::Rearm { .. } => tariff.duration_ic_rearm_component,
        };
        base.max(1).saturating_mul(TENDER_TIME_FACTOR)
    }
}

/// **Yard fittings a structural point consumes, kg** (Q-41-2): the tariff's figure
/// (`structural_points_per_industrial_parts_t`), the constant doc 41 names until doc 39's IN.8
/// gives each design its own `fittings_kg_per_point`.
#[must_use]
pub fn fittings_kg_per_point() -> u64 {
    crate::definitions::workshop::workshop_tariff()
        .map_or(10, |tariff| {
            KG_PER_T.div_ceil(tariff.structural_points_per_industrial_parts_t.max(1))
        })
}

/// **A tend act's start facts**, fixed by the command for the start's prepare.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TendFacts {
    /// The site.
    pub site: CanonicalId,
    /// The unit tended.
    pub target: ObjId,
    /// The work.
    pub work: TendWork,
    /// The act's length, IC seconds.
    pub secs: u32,
    /// Where the tender stands.
    pub tender_pose: FieldPose,
    /// Where the target stands.
    pub target_pose: FieldPose,
}

/// **A finish's draw**, fixed by the tick after the work landed on the unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TendDraw {
    /// The stores vault, `VaultId.0`.
    pub stores: u64,
    /// Goods out of the stores: the tonnes opened into the bale, and a rearm's rounds.
    pub debit: BTreeMap<CanonicalId, u64>,
    /// The bale after the work (`None`: the work touched no bale).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bale_after: Option<Bale>,
    /// The line the operator is told.
    pub line: String,
}

/// **The tick's step on a tend act**.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub enum TendStep {
    /// The work landed on the unit: draw it.
    Finish {
        /// The draw.
        draw: Box<TendDraw>,
    },
    /// The work came to nothing (the target past it, or gone).
    Lost {
        /// Why, in words.
        why: String,
    },
    /// A field opened over the site: keep the time left.
    Pause,
    /// The site settled: start the time left again.
    Resume,
}

impl TendStep {
    /// Its word, for the act key.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Finish { .. } => "finish",
            Self::Lost { .. } => "lost",
            Self::Pause => "pause",
            Self::Resume => "resume",
        }
    }
}

/// **A stores op**, fixed by the command: a load from the hold, or the bale opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stores", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoresOp {
    /// `+tv/<n> stores load <qty> <good> go`.
    Load {
        /// The carrier's hold, `VaultId.0`.
        hold: u64,
        /// The good.
        good: CanonicalId,
        /// How many units.
        qty: u64,
        /// The stores' limits, measure → limit.
        limits: BTreeMap<CanonicalId, u64>,
        /// Each good the stores accept, measure → per unit.
        figures: BTreeMap<CanonicalId, BTreeMap<CanonicalId, u64>>,
    },
    /// `+tv/<n> stores open go`.
    Open,
}

/// **A stores op's facts**.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoresFacts {
    /// The stores vault, `VaultId.0`.
    pub stores: u64,
    /// What it does.
    pub op: StoresOp,
}

// ---- reads --------------------------------------------------------------------------------

/// **`tv`'s tender role** (`logistics.tender`): its stores compartment and reach. `None` for a
/// rig that tends nothing.
#[must_use]
pub fn tender_def(w: &World, tv: ObjId) -> Option<crate::definitions::TenderDef> {
    let cells = mush_tactics::module::unit_of(w, tv)?;
    crate::carrier_damage::spec_of(w, cells)?
        .logistics
        .as_ref()?
        .tender
        .clone()
}

/// **The stores' contents**: every good they accept with a quantity, by good.
fn stores_contents(ctx: &Ctx<'_>, tv: ObjId) -> Option<(VaultId, BTreeMap<CanonicalId, u64>)> {
    let vault = crate::port::vehicle::hopper_of(ctx.w, tv)?;
    let (accepted, _) = crate::port::tv::hopper_goods_of(ctx.w, tv)?;
    let mut out = BTreeMap::new();
    for good in accepted {
        let qty = crate::port_tick::read_available(ctx, EndpointRef(vault), &good).ok()?;
        if qty > 0 {
            out.insert(good, qty);
        }
    }
    Some((vault, out))
}

/// **The rig's bale** off its record.
fn bale_of(ctx: &Ctx<'_>, tv: ObjId) -> Option<Bale> {
    <Ctx<'_> as crate::port::site_map::SurfaceReads>::rig(ctx, tv)
        .ok()
        .flatten()
        .and_then(|record| record.bale)
}

/// The unit's spec, unit record and anatomy.
fn unit_parts(
    w: &World,
    unit: ObjId,
) -> Option<(TacticalUnit, crate::definitions::UnitSpec, UnitAnatomyProfile)> {
    let cells = mush_tactics::module::unit_of(w, unit)?.clone();
    let spec = crate::carrier_damage::spec_of(w, &cells)?.clone();
    let anatomy = crate::field_ops::anatomy_for_unit_spec(&spec).ok()?;
    Some((cells, spec, anatomy))
}

/// The munitions good that loads `component`, and its rounds a unit.
fn munitions_of(
    w: &World,
    spec: &crate::definitions::UnitSpec,
    component: &str,
) -> Option<(CanonicalId, u64)> {
    let store = w.module_state::<TacsimStore>()?;
    let local = component.rsplit('.').next()?;
    let ammunition = spec
        .components
        .iter()
        .find(|c| c.id.as_str() == local || c.id.as_str() == component)
        .and_then(|c| match &c.role {
            crate::definitions::unit::ComponentRole::Weapon { weapon, .. } => {
                store.definitions().weapons.get(weapon)
            }
            _ => None,
        })
        .map(|weapon| weapon.ammunition.clone())?;
    let good = store.goods()?.munitions_for(ammunition.as_str())?;
    let per = good.munitions.as_ref()?.rounds_per_unit.max(1);
    Some((good.id.clone(), u64::from(per)))
}

/// A component id's last segment, as a player types it (`turret.cannon` → `cannon`).
fn local(id: &str) -> &str {
    id.rsplit('.').next().unwrap_or(id)
}

/// **The work a verb finds on `unit`** (the preview's list, and `go`'s pick): every jammed
/// part, every damaged section and its points missing, every mount short of rounds.
fn works(
    w: &World,
    verb: &str,
    unit: ObjId,
    named: Option<&str>,
    points: Option<Points>,
) -> Result<Vec<TendWork>, &'static str> {
    let (cells, spec, anatomy) = unit_parts(w, unit).ok_or("no unit the tender can read")?;
    let assessment = mush_tactics::field_ops::assess_tactical_unit(&cells, &anatomy);
    if assessment.state == OperationalState::Irrecoverable {
        return Err("beyond a tender's work");
    }
    let wanted = |id: &str| named.is_none_or(|n| n.eq_ignore_ascii_case(local(id)) || n == id);
    let mut out = Vec::new();
    match verb {
        "unjam" => {
            for (component, condition) in &assessment.component_conditions {
                if *condition == ComponentCondition::Jammed && wanted(component.as_str()) {
                    out.push(TendWork::Unjam {
                        component: component.as_str().to_owned(),
                    });
                }
            }
        }
        "stabilize" => {
            for (section, condition) in &assessment.section_conditions {
                if !matches!(
                    condition,
                    SectionCondition::Damaged | SectionCondition::Intact
                ) || !wanted(section.as_str())
                {
                    continue;
                }
                let missing = internals_missing(&cells, &anatomy, section);
                if missing <= 0 {
                    continue;
                }
                let points = match points {
                    Some(Points::Count(n)) => n.min(missing),
                    Some(Points::All) | None => missing,
                };
                out.push(TendWork::Stabilize {
                    section: section.as_str().to_owned(),
                    points,
                });
            }
        }
        "rearm" => {
            let Some(ammo) = &anatomy.weapon_ammo_cell else {
                return Ok(out);
            };
            for weapon in &anatomy.weapon_components {
                if !wanted(weapon.as_str()) {
                    continue;
                }
                let addr = CellAddr::Component {
                    component: weapon.clone(),
                    cell: ammo.clone(),
                };
                let Some(cell) = cells.cell(&addr) else {
                    continue;
                };
                let rounds = cell.max.saturating_sub(cell.current);
                if rounds <= 0 {
                    continue;
                }
                let Some((good, per_unit)) = munitions_of(w, &spec, weapon.as_str()) else {
                    continue;
                };
                out.push(TendWork::Rearm {
                    component: weapon.as_str().to_owned(),
                    good,
                    rounds,
                    per_unit,
                });
            }
        }
        _ => return Err("no such work"),
    }
    Ok(out)
}

/// A section's internals short of their maximum, points.
fn internals_missing(cells: &TacticalUnit, anatomy: &UnitAnatomyProfile, section: &SectionId) -> i64 {
    let Some(cell) = &anatomy.internals_cell else {
        return 0;
    };
    cells
        .cell(&CellAddr::Section {
            section: section.clone(),
            cell: cell.clone(),
        })
        .map_or(0, |c| c.max.saturating_sub(c.current))
}

/// **The library's tender state** for `tv` on `site`, registered from its compartment
/// ([`FieldEngineeringBook::register_tender`]; Q-41-2): the bale's fittings plus the stores'
/// whole tonnes as spare parts, the stores' units of `good` as rounds, no charge (no shipped
/// terrain vehicle carries a beam).
#[must_use]
pub fn register_tender(
    tv: ObjId,
    pose: FieldPose,
    reach_cm: u32,
    bale: Option<Bale>,
    stores: &BTreeMap<CanonicalId, u64>,
    rounds: Option<(&CanonicalId, u64)>,
) -> Option<FieldEngineeringBook> {
    let fittings_t = stores
        .iter()
        .find(|(good, _)| good.as_str() == FITTINGS)
        .map_or(0, |(_, qty)| *qty);
    let spare = bale
        .map_or(0, |b| b.fittings_kg)
        .saturating_add(fittings_t.saturating_mul(KG_PER_T));
    let ammo = rounds.map_or(0, |(good, per)| {
        stores.get(good).copied().unwrap_or(0).saturating_mul(per)
    });
    let mut book = FieldEngineeringBook::new();
    book.register_tender(FieldTenderState {
        tender_unit: tv,
        site_id: pose.site,
        pose,
        operational_state: OperationalState::Operational,
        spare_parts_kg: spare,
        ammo_supply_rounds: i64::try_from(ammo).unwrap_or(i64::MAX),
        charge_reserve_cu: 0,
        max_service_distance_cm: u64::from(reach_cm),
    })
    .ok()?;
    Some(book)
}

/// **The library's work on `target`**, the unit changed in place.
fn run_library(
    book: &mut FieldEngineeringBook,
    tv: ObjId,
    target: &mut TacticalUnit,
    (pose, anatomy): (FieldPose, &UnitAnatomyProfile),
    work: &TendWork,
    (secs, now): (i64, i64),
) -> Result<(), String> {
    let result = match work {
        TendWork::Unjam { component } => {
            let id = ComponentId::new(component).map_err(|e| e.to_string())?;
            book.field_unjam_component(
                tv,
                target,
                pose,
                anatomy,
                &id,
                anatomy.component_health_cell.as_ref(),
                DEFAULT_UNJAM_SPARE_PARTS_KG,
                false,
                false,
                secs,
                now,
            )
        }
        TendWork::Stabilize { section, points } => {
            let id = SectionId::new(section).map_err(|e| e.to_string())?;
            book.field_stabilize_damaged_section(
                tv,
                target,
                pose,
                anatomy,
                &id,
                *points,
                fittings_kg_per_point(),
                false,
                false,
                secs,
                now,
            )
        }
        TendWork::Rearm {
            component, rounds, ..
        } => {
            let id = ComponentId::new(component).map_err(|e| e.to_string())?;
            let cell: CellId = anatomy
                .weapon_ammo_cell
                .clone()
                .ok_or("the mount has no magazine")?;
            book.field_resupply_ammo_and_charge(
                tv,
                target,
                pose,
                anatomy,
                Some((&id, &cell, *rounds)),
                0,
                0,
                0,
                None,
                false,
                false,
                secs,
                now,
            )
        }
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}

/// What `need` takes out of the stores through `bale`: the bale after and the goods debited.
fn draw_of(
    bale: Option<Bale>,
    need: &(Bale, Option<(CanonicalId, u64)>),
) -> (Option<Bale>, BTreeMap<CanonicalId, u64>) {
    let mut debit: BTreeMap<CanonicalId, u64> = BTreeMap::new();
    let mut after = None;
    if let Some(draw) = Bale::draw(bale, need.0) {
        for (good, t) in [(ALLOYS, draw.alloys_t), (FITTINGS, draw.fittings_t)] {
            if t > 0
                && let Ok(good) = CanonicalId::new(good)
            {
                *debit.entry(good).or_default() += t;
            }
        }
        after = Some(draw.after);
    }
    if let Some((good, units)) = &need.1
        && *units > 0
    {
        *debit.entry(good.clone()).or_default() += *units;
    }
    (after, debit)
}

/// Goods as words: `2 structural_alloys and 30 ammo_he_105`; `nothing`.
fn goods_words(goods: &BTreeMap<CanonicalId, u64>) -> String {
    let words: Vec<String> = goods
        .iter()
        .map(|(good, qty)| format!("{qty} {good}"))
        .collect();
    if words.is_empty() {
        "nothing".to_owned()
    } else {
        words.join(" and ")
    }
}

/// The bale's kilograms a draw consumes, and its goods, as the preview prints them.
fn draw_words(need: &(Bale, Option<(CanonicalId, u64)>)) -> String {
    let mut parts = Vec::new();
    if need.0.alloys_kg > 0 {
        parts.push(format!("{} kg of structural alloys", need.0.alloys_kg));
    }
    if need.0.fittings_kg > 0 {
        parts.push(format!("{} kg of yard fittings", need.0.fittings_kg));
    }
    if let Some((good, units)) = &need.1 {
        parts.push(format!("{units} {good}"));
    }
    if parts.is_empty() {
        "nothing".to_owned()
    } else {
        parts.join(" and ")
    }
}

/// Whether the stores cover `debit`.
fn covers(stores: &BTreeMap<CanonicalId, u64>, debit: &BTreeMap<CanonicalId, u64>) -> bool {
    debit
        .iter()
        .all(|(good, qty)| stores.get(good).copied().unwrap_or(0) >= *qty)
}

// ---- the commands -------------------------------------------------------------------------

/// **A stabilize's points**: a count or `all`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Points {
    /// That many.
    Count(i64),
    /// To each section's maximum.
    All,
}

/// **The tender's parsed act** (`crate::port::tv`'s `Act::Tend`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TendAct {
    /// `unjam|stabilize|rearm <unit>[=<which>] [<points>|all] [go]`.
    Work {
        /// The verb.
        verb: &'static str,
        /// The unit, as typed.
        unit: String,
        /// The part or section, as typed.
        which: Option<String>,
        /// A stabilize's points.
        points: Option<Points>,
        /// Committed.
        go: bool,
    },
    /// `stores`.
    Show,
    /// `stores load <qty> <good> [go]`.
    Load {
        /// How many units.
        qty: u64,
        /// The good, as typed.
        good: String,
        /// Committed.
        go: bool,
    },
    /// `stores open [go]`.
    Open {
        /// Committed.
        go: bool,
    },
}

/// **Parse the words after a tender verb** (`words` lowercased, `verb` already taken).
///
/// # Errors
///
/// [`TEND_USAGE`].
pub fn parse(verb: &str, words: &[String]) -> Result<TendAct, CmdError> {
    let usage = || CmdError::msg(TEND_USAGE);
    let (go, rest) = match words.split_last() {
        Some((last, rest)) if last == "go" => (true, rest),
        _ => (false, words),
    };
    if rest.iter().any(|w| w == "go") {
        return Err(usage());
    }
    if verb == "stores" {
        return match rest {
            [] if !go => Ok(TendAct::Show),
            [open] if open == "open" => Ok(TendAct::Open { go }),
            [load, qty, good] if load == "load" => {
                let qty = crate::port::families::strict_decimal_u32(qty)
                    .filter(|q| *q > 0)
                    .ok_or_else(usage)?;
                Ok(TendAct::Load {
                    qty: u64::from(qty),
                    good: good.clone(),
                    go,
                })
            }
            _ => Err(usage()),
        };
    }
    let verb: &'static str = match verb {
        "unjam" => "unjam",
        "stabilize" => "stabilize",
        "rearm" => "rearm",
        _ => return Err(usage()),
    };
    let (first, more) = rest.split_first().ok_or_else(usage)?;
    let (unit, which) = match first.split_once('=') {
        Some((unit, which)) if !unit.is_empty() && !which.is_empty() => {
            (unit.to_owned(), Some(which.to_owned()))
        }
        Some(_) => return Err(usage()),
        None => (first.clone(), None),
    };
    let points = match (verb, more) {
        (_, []) => None,
        ("stabilize", [word]) if word == "all" => Some(Points::All),
        ("stabilize", [word]) => Some(Points::Count(
            crate::port::families::strict_decimal_u32(word)
                .filter(|n| *n > 0)
                .map(i64::from)
                .ok_or_else(usage)?,
        )),
        _ => return Err(usage()),
    };
    Ok(TendAct::Work {
        verb,
        unit,
        which,
        points,
        go,
    })
}

/// **`+tv/<n> unjam|stabilize|rearm|stores …`** (this module's head).
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn tend_cmd(ctx: &mut Ctx<'_>, rig: &act::RigAt, act: TendAct) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let def = tender_def(ctx.w, rig.tv).ok_or_else(|| CmdError::msg(NOT_A_TENDER.replace("{n}", &n)))?;
    match act {
        TendAct::Show => show(ctx, rig),
        TendAct::Load { qty, good, go } => load(ctx, rig, qty, &good, go),
        TendAct::Open { go } => open(ctx, rig, go),
        TendAct::Work {
            verb,
            unit,
            which,
            points,
            go,
        } => work(ctx, rig, &def, (verb, &unit, which.as_deref(), points), go),
    }
}

/// `+tv/<n> stores`.
fn show(ctx: &mut Ctx<'_>, rig: &act::RigAt) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let dark = || CmdError::msg(crate::port::tv::TV_DARK.replace("{name}", &rig.mark));
    let (_, contents) = stores_contents(ctx, rig.tv).ok_or_else(dark)?;
    let (kg, cap) = crate::port::tv::hopper_load_of(ctx, rig.tv).map_err(|_| dark())?;
    let bale = match bale_of(ctx, rig.tv) {
        Some(bale) => BALE_OPEN
            .replace("{alloys}", &bale.alloys_kg.to_string())
            .replace("{fittings}", &bale.fittings_kg.to_string()),
        None => BALE_SHUT.replace("<n>", &n),
    };
    let line = STORES_SHOW
        .replace("{n}", &n)
        .replace("{load}", &goods_words(&contents))
        .replace("{kg}", &kg.to_string())
        .replace("{cap}", &cap.to_string())
        .replace("{bale}", &bale);
    crate::port::coop::say(ctx, line);
    Ok(())
}

/// `+tv/<n> stores load <qty> <good> [go]`.
fn load(ctx: &mut Ctx<'_>, rig: &act::RigAt, qty: u64, good: &str, go: bool) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let w: &World = ctx.w;
    if act::where_is(w, rig.tv) != (Where::Bay { hull: rig.hull }) {
        return Err(CmdError::msg(STORES_NOT_IN_BAY.replace("{n}", &n)));
    }
    let dark = || CmdError::msg(crate::port::tv::TV_DARK.replace("{name}", &rig.mark));
    let good = CanonicalId::new(good)
        .map_err(|_| CmdError::msg(STORES_NOT_BULK.replace("{good}", good)))?;
    let (accepted, _) = crate::port::tv::hopper_goods_of(w, rig.tv).ok_or_else(dark)?;
    if !accepted.contains(&good) {
        return Err(CmdError::msg(STORES_NOT_BULK.replace("{good}", good.as_str())));
    }
    let view = crate::port::ship::hull_view(w, rig.hull).ok_or_else(dark)?;
    let hold = crate::port::ship::hold_vault(w, &view).ok_or_else(dark)?;
    let have = crate::port_tick::read_available(ctx, EndpointRef(hold), &good).map_err(|_| dark())?;
    if have < qty {
        return Err(CmdError::msg(
            HOLD_SHORT
                .replace("{have}", &have.to_string())
                .replace("{good}", good.as_str())
                .replace("{qty}", &qty.to_string()),
        ));
    }
    let manifest = BTreeMap::from([(good.clone(), qty)]);
    let (limits, figures) = match stores_room(ctx, rig.tv, &manifest) {
        Some(Ok(room)) => room,
        Some(Err(())) => {
            return Err(CmdError::msg(
                STORES_NO_ROOM
                    .replace("{n}", &n)
                    .replace("{qty}", &qty.to_string())
                    .replace("{good}", good.as_str()),
            ));
        }
        None => return Err(dark()),
    };
    let carrier = crate::port::coop::name_of(w, rig.hull);
    let fill = |line: &str| {
        line.replace("{n}", &n)
            .replace("{qty}", &qty.to_string())
            .replace("{good}", good.as_str())
            .replace("{carrier}", &carrier)
    };
    if !go {
        crate::port::coop::say(ctx, fill(STORES_LOAD_PREVIEW));
        return Ok(());
    }
    let stores = crate::port::vehicle::hopper_of(w, rig.tv).ok_or_else(dark)?;
    let order = TowOrder::Stores {
        facts: Box::new(StoresFacts {
            stores: stores.0,
            op: StoresOp::Load {
                hold: hold.0,
                good: good.clone(),
                qty,
                limits,
                figures,
            },
        }),
    };
    let actor = ctx.actor;
    if !tow::submit(ctx, actor, rig.tv, order, 0) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    crate::port::coop::say(ctx, fill(STORES_LOADED));
    Ok(())
}

/// The stores' limits and figures, when `manifest` fits on top of what they hold.
type Room = (
    BTreeMap<CanonicalId, u64>,
    BTreeMap<CanonicalId, BTreeMap<CanonicalId, u64>>,
);

/// **The stores' room against `manifest`** (the pickup's `hopper_room` rule): `Ok` the limits
/// and figures the prepare re-checks, `Err(())` when it does not fit, `None` unreadable.
fn stores_room(
    ctx: &Ctx<'_>,
    tv: ObjId,
    manifest: &BTreeMap<CanonicalId, u64>,
) -> Option<Result<Room, ()>> {
    let goods = ctx.w.module_state::<TacsimStore>()?.goods()?;
    let def = tender_def(ctx.w, tv)?;
    let cells = mush_tactics::module::unit_of(ctx.w, tv)?;
    let spec = crate::carrier_damage::spec_of(ctx.w, cells)?;
    let comp = spec.logistics.as_ref()?.stowage.get(&def.stores)?;
    let limits = comp.limits.clone();
    let (_, contents) = stores_contents(ctx, tv)?;
    let mut figures: BTreeMap<CanonicalId, BTreeMap<CanonicalId, u64>> = BTreeMap::new();
    let (accepted, _) = crate::port::tv::hopper_goods_of(ctx.w, tv)?;
    for good in &accepted {
        let row = goods.good(good)?;
        figures.insert(
            good.clone(),
            limits
                .keys()
                .filter_map(|m| row.per_unit(m).map(|per| (m.clone(), per)))
                .collect(),
        );
    }
    let mut load: BTreeMap<CanonicalId, u64> = BTreeMap::new();
    for (good, qty) in contents.iter().chain(manifest.iter()) {
        for (measure, per) in figures.get(good)? {
            *load.entry(measure.clone()).or_default() += qty.saturating_mul(*per);
        }
    }
    if limits
        .iter()
        .any(|(m, limit)| load.get(m).copied().unwrap_or(0) > *limit)
    {
        return Some(Err(()));
    }
    Some(Ok((limits, figures)))
}

/// `+tv/<n> stores open [go]`.
fn open(ctx: &mut Ctx<'_>, rig: &act::RigAt, go: bool) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let dark = || CmdError::msg(crate::port::tv::TV_DARK.replace("{name}", &rig.mark));
    if bale_of(ctx, rig.tv).is_some() {
        return Err(CmdError::msg(BALE_ALREADY.replace("{n}", &n)));
    }
    let (stores, contents) = stores_contents(ctx, rig.tv).ok_or_else(dark)?;
    for good in [ALLOYS, FITTINGS] {
        if contents
            .iter()
            .find(|(id, _)| id.as_str() == good)
            .map_or(0, |(_, qty)| *qty)
            == 0
        {
            return Err(CmdError::msg(
                STORES_NO_TONNE.replace("{n}", &n).replace("{good}", good),
            ));
        }
    }
    if !go {
        crate::port::coop::say(ctx, STORES_OPEN_PREVIEW.replace("{n}", &n));
        return Ok(());
    }
    let order = TowOrder::Stores {
        facts: Box::new(StoresFacts {
            stores: stores.0,
            op: StoresOp::Open,
        }),
    };
    let actor = ctx.actor;
    if !tow::submit(ctx, actor, rig.tv, order, 0) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    crate::port::coop::say(ctx, STORES_OPENED.replace("{n}", &n));
    Ok(())
}

/// `+tv/<n> unjam|stabilize|rearm <unit>[=<which>] … [go]`.
fn work(
    ctx: &mut Ctx<'_>,
    rig: &act::RigAt,
    def: &crate::definitions::TenderDef,
    (verb, unit_word, which, points): (&'static str, &str, Option<&str>, Option<Points>),
    go: bool,
) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let w: &World = ctx.w;
    let (site, tender_pose) = tow::ground_pose(w, rig.tv)
        .ok_or_else(|| CmdError::msg(tow::NOT_ON_GROUND.replace("{n}", &n)))?;
    let record = tow::record_or_new(ctx, &site);
    if record.tows.act_of(rig.tv).is_some() {
        return Err(CmdError::msg(tow::TOW_BUSY.replace("{n}", &n)));
    }
    if record.tows.towing(rig.tv).is_some() {
        return Err(CmdError::msg(tow::IN_TOW.replace("{n}", &n)));
    }
    // The unit named, on the same ground within reach (the tow's: a cell, diagonals included).
    let wanted = unit_word.trim_start_matches('#');
    let target = record
        .units
        .iter()
        .map(|left| left.unit)
        .chain(
            w.module_state::<TacsimStore>()
                .into_iter()
                .flat_map(TacsimStore::tv_leases_live)
                .map(|lease| lease.tv),
        )
        .find(|unit| {
            *unit != rig.tv
                && (unit.0.to_string() == wanted
                    || crate::port::coop::name_of(w, *unit).eq_ignore_ascii_case(unit_word))
        });
    let not_near = || {
        CmdError::msg(
            TEND_NOT_IN_REACH
                .replace("{n}", &n)
                .replace("{name}", unit_word),
        )
    };
    let target = target.ok_or_else(not_near)?;
    let label = crate::port::coop::name_of(w, target);
    let (there, target_pose) = tow::ground_pose(w, target).ok_or_else(not_near)?;
    let near = there == site
        && tender_pose.site == target_pose.site
        && cells_apart(
            site_cell(tender_pose.x_cm, tender_pose.y_cm),
            site_cell(target_pose.x_cm, target_pose.y_cm),
        ) <= 1
        && tender_pose
            .distance_cm_to(&target_pose)
            .is_some_and(|d| d <= u64::from(def.reach_cm).max(tow::TOW_REACH_CM));
    if !near {
        return Err(not_near());
    }
    if record.tows.towed(target).is_some() {
        return Err(CmdError::msg(TEND_IN_TOW.replace("{name}", &label)));
    }
    let found = works(w, verb, target, which, points).map_err(|why| {
        CmdError::msg(
            TEND_REFUSED
                .replace("{n}", &n)
                .replace("{name}", &label)
                .replace("{why}", why),
        )
    })?;
    let what = match verb {
        "unjam" => "no jammed part",
        "stabilize" => "no damaged section a patch can reach",
        _ => "no mount short of rounds",
    };
    let Some(job) = found.into_iter().next() else {
        return Err(CmdError::msg(
            TEND_NOTHING
                .replace("{n}", &n)
                .replace("{name}", &label)
                .replace("{what}", what),
        ));
    };
    // What the stores and the bale hold for it.
    let dark = || CmdError::msg(crate::port::tv::TV_DARK.replace("{name}", &rig.mark));
    let (_, contents) = stores_contents(ctx, rig.tv).ok_or_else(dark)?;
    let need = job.need();
    if let TendWork::Rearm {
        component, good, ..
    } = &job
        && contents.get(good).copied().unwrap_or(0) == 0
    {
        return Err(CmdError::msg(
            NO_ROUNDS_ABOARD
                .replace("{n}", &n)
                .replace("{part}", local(component))
                .replace("{good}", good.as_str()),
        ));
    }
    // A rearm loads what the stores hold, up to the magazine.
    let job = match job {
        TendWork::Rearm {
            component,
            good,
            rounds,
            per_unit,
        } => {
            let held = contents
                .get(&good)
                .copied()
                .unwrap_or(0)
                .saturating_mul(per_unit);
            TendWork::Rearm {
                component,
                rounds: rounds.min(i64::try_from(held).unwrap_or(i64::MAX)),
                good,
                per_unit,
            }
        }
        other => other,
    };
    let need = if matches!(job, TendWork::Rearm { .. }) {
        job.need()
    } else {
        need
    };
    let (_, debit) = draw_of(bale_of(ctx, rig.tv), &need);
    if !covers(&contents, &debit) {
        return Err(CmdError::msg(
            TEND_SHORT
                .replace("{n}", &n)
                .replace("{need}", &goods_words(&debit))
                .replace("{have}", &goods_words(&contents)),
        ));
    }
    let secs = u32::try_from(job.secs()).unwrap_or(u32::MAX);
    let min = secs.div_ceil(60).to_string();
    if !go {
        let arg = match (&job, which) {
            (TendWork::Stabilize { points, .. }, _) => format!(
                "{unit_word}={} {points}",
                which.unwrap_or_else(|| match &job {
                    TendWork::Stabilize { section, .. } => section.as_str(),
                    _ => "",
                })
            ),
            (_, Some(which)) => format!("{unit_word}={which}"),
            (_, None) => unit_word.to_owned(),
        };
        let line = TEND_PREVIEW
            .replace("{n}", &n)
            .replace("{name}", &label)
            .replace("{work}", &job.words())
            .replace("{draw}", &draw_words(&need))
            .replace("{min}", &min)
            .replace("{verb}", verb)
            .replace("{arg}", &arg);
        crate::port::coop::say(ctx, line);
        return Ok(());
    }
    let words = job.words();
    let facts = TendFacts {
        site,
        target,
        work: job,
        secs,
        tender_pose,
        target_pose,
    };
    let actor = ctx.actor;
    if !tow::submit(
        ctx,
        actor,
        rig.tv,
        TowOrder::Tend {
            facts: Box::new(facts),
        },
        0,
    ) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    crate::port::coop::say(
        ctx,
        TEND_FILED
            .replace("{n}", &n)
            .replace("{name}", &label)
            .replace("{work}", &words)
            .replace("{min}", &min),
    );
    Ok(())
}

// ---- the tick -----------------------------------------------------------------------------

/// **The tick's step on `tv`'s due tend act**: the library run on the target's unit, the change
/// applied, and the draw the finish takes; or what it came to when the work is past.
pub(crate) fn finish(
    ctx: &mut Ctx<'_>,
    tv: ObjId,
    target: ObjId,
    work: &TendWork,
    (secs, now): (i64, i64),
) -> TendStep {
    let lost = |why: &str| TendStep::Lost { why: why.to_owned() };
    let w: &World = ctx.w;
    let Some(def) = tender_def(w, tv) else {
        return lost("the rig is no tender");
    };
    let Some((site, pose)) = tow::ground_pose(w, tv) else {
        return lost("the rig left the ground");
    };
    let Some((there, target_pose)) = tow::ground_pose(w, target) else {
        return lost("it is no longer on the ground");
    };
    if there != site {
        return lost("it is no longer on the same ground");
    }
    let Some((before, _, anatomy)) = unit_parts(w, target) else {
        return lost("its unit cannot be read");
    };
    let Some((stores, contents)) = stores_contents(ctx, tv) else {
        return lost("the stores cannot be read");
    };
    let bale = bale_of(ctx, tv);
    let need = work.need();
    let (bale_after, debit) = draw_of(bale, &need);
    if !covers(&contents, &debit) {
        return lost("the stores ran short");
    }
    let rounds = match work {
        TendWork::Rearm { good, per_unit, .. } => Some((good, *per_unit)),
        _ => None,
    };
    let Some(mut book) = register_tender(tv, pose, def.reach_cm, bale, &contents, rounds) else {
        return lost("the tender cannot be registered");
    };
    let mut after = before.clone();
    if let Err(why) = run_library(
        &mut book,
        tv,
        &mut after,
        (target_pose, &anatomy),
        work,
        (secs, now),
    ) {
        return TendStep::Lost { why };
    }
    let plan = mush_tactics::field_ops::ic_condition_plan(
        &before,
        &after,
        None,
        Some(smol_str::SmolStr::new(format!("tacsim.tender.{}", work.verb()))),
    );
    match plan {
        Ok(Some(plan)) => {
            if let Err(error) = mush_tactics::module::apply_plan(ctx, target, &plan) {
                return TendStep::Lost { why: error };
            }
        }
        Ok(None) => return lost("there was nothing left to do"),
        Err(error) => return TendStep::Lost { why: error.to_string() },
    }
    TendStep::Finish {
        draw: Box::new(TendDraw {
            stores: stores.0,
            debit,
            bale_after,
            line: work.words(),
        }),
    }
}

/// **The finishes the tick has run on a unit and not yet seen land** (RAM): `(tender, ordinal)`
/// → the step it submitted, so a finish refused or lost in flight is resubmitted as it was and
/// the work never lands on the unit twice; and the line the operator is told when it lands.
#[derive(Clone, Debug, Default)]
pub(crate) struct TendApplied(BTreeMap<(ObjId, u32), TendStep>);

/// **The step for `act`, due now**: the one already run for it, else [`finish`]'s, kept.
pub(crate) fn due_step(
    ctx: &mut Ctx<'_>,
    act: &tow::TowAct,
    target: ObjId,
    work: &TendWork,
    now: i64,
) -> TendStep {
    let key = (act.tug, act.ordinal);
    if let Some(step) = ctx
        .w
        .module_state::<TendApplied>()
        .and_then(|applied| applied.0.get(&key).cloned())
    {
        return step;
    }
    let secs = act.due_ic.saturating_sub(act.started_ic);
    let step = finish(ctx, act.tug, target, work, (secs, now));
    if let Some(applied) = ctx.w.module_state_mut::<TendApplied>() {
        // Bounded as the tow's acts are: one a tug.
        applied.0.retain(|(tug, _), _| *tug != act.tug);
        applied.0.insert(key, step.clone());
    }
    step
}

/// **A pause or a resume** submitted for `act`, and the operator told.
pub(crate) fn step(ctx: &mut Ctx<'_>, site: &CanonicalId, act: &tow::TowAct, step: TendStep) {
    let TowStage::Tend {
        target, paused, ..
    } = &act.stage
    else {
        return;
    };
    let slot = slot_of(ctx.w, act.tug);
    let name = crate::port::coop::name_of(ctx.w, *target);
    let line = match &step {
        TendStep::Pause => TEND_PAUSED.replace("{n}", &slot).replace("{name}", &name),
        TendStep::Resume => TEND_RESUMED
            .replace("{n}", &slot)
            .replace("{name}", &name)
            .replace(
                "{min}",
                &(paused.unwrap_or(0).max(0).saturating_add(59) / 60).to_string(),
            ),
        TendStep::Finish { .. } | TendStep::Lost { .. } => return,
    };
    let tag = act.ordinal.saturating_mul(4).saturating_add(match step {
        TendStep::Pause => 1,
        _ => 2,
    });
    let order = TowOrder::Tended {
        ordinal: act.ordinal,
        site: site.clone(),
        step,
    };
    if tow::submit(ctx, act.operator, act.tug, order, tag) {
        ctx.out.sys_to_actor(act.operator, "tacsim.tow", line);
    }
}

/// The rig's slot word, as the lines print it.
fn slot_of(w: &World, tv: ObjId) -> String {
    w.module_state::<TacsimStore>()
        .and_then(|store| store.tv_lease_on(tv))
        .map_or_else(|| tv.0.to_string(), |lease| lease.slot.to_string())
}

/// **The line a landed tend act tells** (`crate::surface::tow`'s `tell_landed`).
pub(crate) fn landed_line(
    ctx: &mut Ctx<'_>,
    key: (ObjId, u32),
    slot: &str,
    target: ObjId,
) -> String {
    let name = crate::port::coop::name_of(ctx.w, target);
    let step = ctx
        .w
        .module_state_mut::<TendApplied>()
        .and_then(|applied| applied.0.remove(&key));
    match step {
        Some(TendStep::Finish { draw }) => TENDED
            .replace("{n}", slot)
            .replace("{name}", &name)
            .replace("{work}", &draw.line),
        Some(TendStep::Lost { why }) => TEND_LOST
            .replace("{n}", slot)
            .replace("{name}", &name)
            .replace("{why}", &why),
        _ => TEND_LOST
            .replace("{n}", slot)
            .replace("{name}", &name)
            .replace("{why}", "it was let go"),
    }
}
