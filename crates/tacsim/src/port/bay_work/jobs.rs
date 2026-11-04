//! **The jobs, the facility classes and the quote** (doc 36 cut GC.8; Q-36-35, Q-36-40, §4a).
//!
//! Pure: a hull's assessment becomes a list of jobs, each priced from the workshop tariff and
//! naming the least facility class that may do it, and an accepted list becomes the hull's
//! post-image through `mush_tactics`' workshop engine. Nothing here reads or writes the world.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ObjId};
use mush_tactics::TacticalUnit;
use mush_tactics::field_ops::{
    ComponentCondition, CustodyTitleStatus, ReplacementPartSpec, SectionCondition,
    UnitAnatomyProfile, WorkshopBook, WorkshopFacilityTariff, assess_tactical_unit,
};
use mush_tactics::unit::{CellAddr, CellId, ComponentId, SectionId};
use serde::{Deserialize, Serialize};

use crate::definitions::unit::{ComponentRole, UnitSpec};
use crate::definitions::{Catalog, GoodsTable};

/// **Where the work is done** (Q-36-35 as ruled 2026-11-02).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacilityClass {
    /// A carrier's vehicle bay: minor work only, at [`CARRIER_TIME_FACTOR`] times the time.
    CarrierBay,
    /// A port workshop room (GC.7's `bay_refit` facilities): everything.
    Workshop,
}

impl FacilityClass {
    /// The word a quote prints.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::CarrierBay => "carrier bay",
            Self::Workshop => "workshop",
        }
    }
}

/// A carrier bay takes this many times the workshop's time for the same job, at the same cost
/// (Q-36-35, §4a: "doubles the time, not the cost"). Provisional.
pub const CARRIER_TIME_FACTOR: i64 = 2;

/// **One job on one hull** (Q-36-40): a change to cells and conditions only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "job", rename_all = "snake_case", deny_unknown_fields)]
pub enum BayJob {
    /// Patch a standing section's armour and internals back to full.
    Armor {
        /// The section.
        section: SectionId,
    },
    /// Rebuild a destroyed section's shell.
    Rebuild {
        /// The section.
        section: SectionId,
    },
    /// Repair a damaged part in place.
    Mend {
        /// The part.
        component: ComponentId,
    },
    /// Clear a jammed part.
    Unjam {
        /// The part.
        component: ComponentId,
    },
    /// Fit a part of the slot's own definition where one is destroyed or missing: a new one
    /// the workshop supplies, or a stripped part from a lot in the carrier's hold (GC.8b).
    Replace {
        /// The slot.
        component: ComponentId,
        /// The lot the part comes from, when it is fitted from the hold.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lot: Option<mush_inventory::LotId>,
    },
    /// Load rounds into a weapon's magazine, drawn from munitions goods (Q-36-36).
    Rearm {
        /// The weapon.
        component: ComponentId,
        /// Rounds the magazine is short.
        rounds: i64,
        /// The munitions good that loads it, or `None` for beam charge (a cell, not a good).
        good: Option<CanonicalId>,
        /// Units of `good` the rounds take (`ceil(rounds / rounds_per_unit)`), 0 without one.
        units: u64,
    },
}

impl BayJob {
    /// The least facility class that may do this job (Q-36-35): armour, mending, unjamming and
    /// rearming in a carrier bay; rebuilding and replacing in a workshop only.
    #[must_use]
    pub const fn needs(&self) -> FacilityClass {
        match self {
            Self::Armor { .. } | Self::Mend { .. } | Self::Unjam { .. } | Self::Rearm { .. } => {
                FacilityClass::CarrierBay
            }
            Self::Rebuild { .. } | Self::Replace { .. } => FacilityClass::Workshop,
        }
    }

    /// Whether this job is a repair (Q-36-33's "repair": everything but loading rounds).
    #[must_use]
    pub const fn is_repair(&self) -> bool {
        !matches!(self, Self::Rearm { .. })
    }

    /// The selector word that picks this job in `+bay/order` (`armor`, `rebuild`, `mend`,
    /// `unjam`, `replace`, `rearm`).
    #[must_use]
    pub const fn kind_word(&self) -> &'static str {
        match self {
            Self::Armor { .. } => "armor",
            Self::Rebuild { .. } => "rebuild",
            Self::Mend { .. } => "mend",
            Self::Unjam { .. } => "unjam",
            Self::Replace { .. } => "replace",
            Self::Rearm { .. } => "rearm",
        }
    }

    /// The section or part the job names.
    #[must_use]
    pub fn target(&self) -> &str {
        match self {
            Self::Armor { section } | Self::Rebuild { section } => section.as_str(),
            Self::Mend { component }
            | Self::Unjam { component }
            | Self::Replace { component, .. }
            | Self::Rearm { component, .. } => component.as_str(),
        }
    }

    /// One line for the quote: `unjam turret.cannon`, `rearm hull.cannon (10 rounds, 10 ammo_he_105)`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Rearm {
                rounds,
                good: Some(good),
                units,
                ..
            } => format!("rearm {} ({rounds} rounds, {units} {good})", self.target()),
            Self::Rearm { rounds, .. } => {
                format!("recharge {} ({rounds} rounds)", self.target())
            }
            Self::Replace { lot: Some(lot), .. } => format!("fit {} from {lot}", self.target()),
            _ => format!("{} {}", self.kind_word(), self.target()),
        }
    }
}

/// A job with its price (doc 36 Q-36-31's tariff at the operator's multiplier).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotedJob {
    /// The job.
    pub job: BayJob,
    /// Labour, minor units.
    pub cost_minor: u64,
    /// Workshop time, IC seconds (before a carrier bay's factor).
    pub secs: i64,
    /// Structural points the job restores.
    pub points: u64,
}

/// **A quote for one hull at one facility class** (D-31-13: every action produces a quote first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BayQuote {
    /// The hull.
    pub hull: ObjId,
    /// Where it stands.
    pub class: FacilityClass,
    /// The jobs this facility may do.
    pub jobs: Vec<QuotedJob>,
    /// The jobs that need a better facility than this one (a carrier bay's destroyed parts).
    pub elsewhere: Vec<QuotedJob>,
}

impl BayQuote {
    /// Labour over `jobs`, minor units.
    #[must_use]
    pub fn cost_minor(jobs: &[QuotedJob]) -> u64 {
        jobs.iter()
            .fold(0u64, |sum, job| sum.saturating_add(job.cost_minor))
    }

    /// Time over `jobs` at `class`, IC seconds: the workshop's, doubled in a carrier bay.
    #[must_use]
    pub fn secs(jobs: &[QuotedJob], class: FacilityClass) -> i64 {
        let base = jobs
            .iter()
            .fold(0i64, |sum, job| sum.saturating_add(job.secs.max(1)));
        match class {
            FacilityClass::Workshop => base,
            FacilityClass::CarrierBay => base.saturating_mul(CARRIER_TIME_FACTOR),
        }
    }

    /// **The kilograms the jobs' points consume** (Q-36-31; doc 39 cut IN.1, §4.10): yard
    /// fittings at the tariff's rate, now to the kilogram rather than rounded up to a tonne a
    /// job; no alloys until doc 39 cut IN.8 gives each design its own per-point figures. Drawn
    /// through the hull's bale ([`super::Bale::draw`]).
    #[must_use]
    pub fn bale_need(jobs: &[QuotedJob], tariff: &WorkshopFacilityTariff) -> super::Bale {
        let points = jobs
            .iter()
            .fold(0u64, |sum, job| sum.saturating_add(job.points));
        super::Bale {
            alloys_kg: 0,
            fittings_kg: points
                .saturating_mul(super::KG_PER_T)
                .div_ceil(tariff.structural_points_per_industrial_parts_t.max(1)),
        }
    }

    /// The jobs a selector picks: `all`, a kind word (`armor`, `unjam`, `rearm`, …) or a
    /// section or part id; several, comma-separated. `None` when a word picks nothing.
    #[must_use]
    pub fn select(&self, selector: &str) -> Option<Vec<QuotedJob>> {
        let mut picked: Vec<QuotedJob> = Vec::new();
        for word in selector
            .split(',')
            .map(str::trim)
            .filter(|word| !word.is_empty())
        {
            let before = picked.len();
            for job in &self.jobs {
                let hit = word.eq_ignore_ascii_case("all")
                    || word.eq_ignore_ascii_case(job.job.kind_word())
                    || word.eq_ignore_ascii_case(job.job.target());
                if hit && !picked.contains(job) {
                    picked.push(job.clone());
                }
            }
            if picked.len() == before {
                return None;
            }
        }
        (!picked.is_empty()).then_some(picked)
    }
}

fn section_points(unit: &TacticalUnit, section: &SectionId) -> u64 {
    unit.cells()
        .filter(|(addr, _)| match addr {
            CellAddr::Section { cell: cid, .. }
                if cid.as_str() == crate::definitions::unit::HEAT_CELL
                    || cid.as_str() == crate::definitions::unit::HEAT_SINK_CELL =>
            {
                false
            }
            CellAddr::Section { section: s, .. } | CellAddr::Facet { section: s, .. } => {
                s == section
            }
            _ => false,
        })
        .fold(0u64, |sum, (_, cell)| {
            sum.saturating_add(u64::try_from(cell.max.saturating_sub(cell.current)).unwrap_or(0))
        })
}

/// The ammunition a weapon part loads, through its spec row and the catalog's weapon table.
fn ammunition_of<'a>(
    spec: &UnitSpec,
    catalog: &'a Catalog,
    component: &ComponentId,
) -> Option<&'a CanonicalId> {
    let local = component.as_str().rsplit('.').next()?;
    spec.components
        .iter()
        .find(|c| c.id.as_str() == local || c.id.as_str() == component.as_str())
        .and_then(|c| match &c.role {
            ComponentRole::Weapon { weapon, .. } => catalog.weapons.get(weapon),
            _ => None,
        })
        .map(|w| &w.ammunition)
}

/// **Every job `unit` needs, priced** (Q-36-31, Q-36-35, Q-36-40), split between what `class`
/// may do and what needs a workshop. A section that stands but is down points is `armor`; a
/// destroyed or wrecked one `rebuild`. A damaged part is `mend`, a jammed one `unjam`, a
/// destroyed or missing one `replace` (GC.2's `Missing`). A weapon short of rounds whose part
/// is not being replaced is `rearm`, its rounds taken from the munitions good that loads its
/// ammunition (GC.7), or from the bay's own charge for beam charge.
///
/// # Errors
///
/// A hull past saving (irrecoverable): nothing to quote.
pub fn quote(
    unit: &TacticalUnit,
    spec: &UnitSpec,
    anatomy: &UnitAnatomyProfile,
    catalog: &Catalog,
    goods: &GoodsTable,
    tariff: &WorkshopFacilityTariff,
    class: FacilityClass,
) -> Result<BayQuote, String> {
    let assessment = assess_tactical_unit(unit, anatomy);
    if !assessment.state.retains_salvageable_hull() {
        return Err("That hull is past repair; only scrap is left of it.".to_owned());
    }
    let mut all: Vec<QuotedJob> = Vec::new();
    for (section, condition) in &assessment.section_conditions {
        let points = section_points(unit, section);
        let cost = points.saturating_mul(tariff.cost_per_structural_point_repair);
        let secs = i64::try_from(points)
            .unwrap_or(i64::MAX)
            .saturating_mul(tariff.duration_ic_per_structural_point_repair);
        match condition {
            SectionCondition::Intact | SectionCondition::Damaged if points > 0 => {
                all.push(QuotedJob {
                    job: BayJob::Armor {
                        section: section.clone(),
                    },
                    cost_minor: cost,
                    secs,
                    points,
                });
            }
            SectionCondition::Destroyed | SectionCondition::Wreck => all.push(QuotedJob {
                job: BayJob::Rebuild {
                    section: section.clone(),
                },
                cost_minor: tariff.base_cost_reconstruct_section.saturating_add(cost),
                secs: tariff.base_duration_ic_reconstruct_section,
                points,
            }),
            _ => {}
        }
    }
    let mut replaced = Vec::new();
    for (component, condition) in &assessment.component_conditions {
        let job = match condition {
            ComponentCondition::Damaged => BayJob::Mend {
                component: component.clone(),
            },
            ComponentCondition::Jammed => BayJob::Unjam {
                component: component.clone(),
            },
            ComponentCondition::Destroyed | ComponentCondition::Missing => {
                replaced.push(component.clone());
                BayJob::Replace {
                    component: component.clone(),
                    lot: None,
                }
            }
            ComponentCondition::Intact => continue,
        };
        let (cost_minor, secs) = if matches!(job, BayJob::Replace { .. }) {
            (
                tariff.labor_cost_replace_component,
                tariff.duration_ic_replace_component,
            )
        } else {
            (
                tariff.cost_repair_component,
                tariff.duration_ic_repair_component,
            )
        };
        all.push(QuotedJob {
            job,
            cost_minor,
            secs,
            points: 0,
        });
    }
    if let Some(ammo) = &anatomy.weapon_ammo_cell {
        for weapon in &anatomy.weapon_components {
            if replaced.contains(weapon) {
                continue;
            }
            let addr = CellAddr::Component {
                component: weapon.clone(),
                cell: ammo.clone(),
            };
            let Some(cell) = unit.cell(&addr) else {
                continue;
            };
            let rounds = cell.max.saturating_sub(cell.current);
            if rounds <= 0 {
                continue;
            }
            let link = ammunition_of(spec, catalog, weapon)
                .and_then(|ammunition| goods.munitions_for(ammunition.as_str()))
                .and_then(|good| {
                    good.munitions
                        .as_ref()
                        .map(|m| (good.id.clone(), u64::from(m.rounds_per_unit.max(1))))
                });
            let (good, units) = match link {
                Some((good, per)) => (Some(good), u64::try_from(rounds).unwrap_or(0).div_ceil(per)),
                None => (None, 0),
            };
            all.push(QuotedJob {
                job: BayJob::Rearm {
                    component: weapon.clone(),
                    rounds,
                    good,
                    units,
                },
                cost_minor: u64::try_from(rounds)
                    .unwrap_or(0)
                    .saturating_mul(tariff.cost_per_ammo_round),
                secs: tariff.duration_ic_rearm_component,
                points: 0,
            });
        }
    }
    let (jobs, elsewhere) = all.into_iter().partition(|job| job.job.needs() <= class);
    Ok(BayQuote {
        hull: unit.object(),
        class,
        jobs,
        elsewhere,
    })
}

/// The order the work runs in: shells before the parts they hold, parts before their rounds.
const fn rank(job: &BayJob) -> u8 {
    match job {
        BayJob::Rebuild { .. } => 0,
        BayJob::Armor { .. } => 1,
        BayJob::Replace { .. } => 2,
        BayJob::Mend { .. } | BayJob::Unjam { .. } => 3,
        BayJob::Rearm { .. } => 4,
    }
}

/// **The hull after the work** (Q-36-40): each job through `mush_tactics`' workshop engine on a
/// throwaway book, in rank order, so the engine's own refusals hold (a part in a section
/// that does not stand is refused). A replaced part is fitted intact with its cells full, but a
/// weapon's magazine empty: rounds are goods (Q-36-36). `loaded` caps each rearm at the rounds
/// actually drawn. A part fitted from a lot (GC.8b) comes in the condition its provenance
/// records (`fitted`), its cells full but a weapon's magazine empty. `custody` is the salvor's
/// pending claim when one does the work at their own risk (Q-S14), else the title.
///
/// # Errors
///
/// The engine's refusal, as a sentence.
#[allow(clippy::too_many_arguments)]
pub fn work(
    unit: &TacticalUnit,
    anatomy: &UnitAnatomyProfile,
    jobs: &[QuotedJob],
    loaded: &BTreeMap<ComponentId, i64>,
    fitted: &BTreeMap<ComponentId, ComponentCondition>,
    actor: ObjId,
    custody: &CustodyTitleStatus,
    at_ic: i64,
) -> Result<TacticalUnit, String> {
    let mut book = WorkshopBook::new();
    let mut ordered: Vec<&QuotedJob> = jobs.iter().collect();
    ordered.sort_by_key(|job| rank(&job.job));
    let mut now = unit.clone();
    let say = |e: mush_tactics::field_ops::WorkshopError| e.to_string();
    for quoted in ordered {
        let cost = quoted.cost_minor;
        let secs = quoted.secs.max(1);
        now = match &quoted.job {
            BayJob::Armor { section } => {
                book.repair_section(
                    &now, anatomy, section, actor, custody, false, cost, secs, None, at_ic,
                )
                .map_err(say)?
                .0
            }
            BayJob::Rebuild { section } => {
                book.reconstruct_section(
                    &now, anatomy, section, actor, custody, false, cost, secs, None, at_ic,
                )
                .map_err(say)?
                .0
            }
            BayJob::Mend { component } | BayJob::Unjam { component } => {
                book.repair_component(
                    &now, anatomy, component, actor, custody, false, cost, secs, None, at_ic,
                )
                .map_err(say)?
                .0
            }
            BayJob::Replace { component, lot } => {
                let installed = now
                    .component(component)
                    .ok_or_else(|| format!("{component} is not on this hull"))?;
                let mut cell_values = BTreeMap::new();
                if anatomy.weapon_components.contains(component)
                    && let Some(ammo) = &anatomy.weapon_ammo_cell
                {
                    cell_values.insert(ammo.clone(), 0);
                }
                let condition = match lot {
                    Some(_) => fitted
                        .get(component)
                        .copied()
                        .ok_or_else(|| format!("{component}'s fitted part is not traced"))?,
                    None => ComponentCondition::Intact,
                };
                let part = ReplacementPartSpec {
                    source_stripped_record_id: None,
                    definition: installed.definition.clone(),
                    condition,
                    cell_values,
                    asset: None,
                };
                book.replace_component_in_slot(
                    &now, anatomy, component, &part, actor, custody, false, cost, secs, None, at_ic,
                )
                .map_err(say)?
                .0
            }
            BayJob::Rearm {
                component, rounds, ..
            } => {
                let rounds = loaded.get(component).copied().unwrap_or(*rounds);
                if rounds <= 0 {
                    continue;
                }
                let cell: CellId = anatomy
                    .weapon_ammo_cell
                    .clone()
                    .ok_or_else(|| "this hull's weapons carry no magazine".to_owned())?;
                book.rearm_component(
                    &now, anatomy, component, &cell, rounds, actor, custody, false, cost, secs,
                    None, at_ic,
                )
                .map_err(say)?
                .0
            }
        };
    }

    if !jobs.is_empty() {
        let mut cleanup_ops = Vec::new();
        if let Ok(hull_sec) = SectionId::new("hull")
            && let Ok(sink_cell) = CellId::new(crate::definitions::unit::HEAT_SINK_CELL)
        {
            let sink_addr = CellAddr::Section {
                section: hull_sec,
                cell: sink_cell,
            };
            if let Some(cell) = now.cell(&sink_addr)
                && cell.current < cell.max
            {
                cleanup_ops.push(mush_tactics::UnitMutationOp::SetCell {
                    addr: sink_addr,
                    value: cell.max,
                });
            }
        }
        if let Ok(cap_id) =
            mush_tactics::CapabilityId::new(crate::space::custody::SPACE_SUBSYSTEMS_CAPABILITY)
        {
            let subsys_key = mush_tactics::CapabilityKey {
                id: cap_id,
                source: None,
            };
            if now.capability(&subsys_key).is_some() {
                cleanup_ops
                    .push(mush_tactics::UnitMutationOp::RevokeCapability { key: subsys_key });
            }
        }
        if !cleanup_ops.is_empty()
            && let Ok(plan) =
                mush_tactics::UnitMutationPlan::new(cleanup_ops, Some(now.revision()), None)
            && let Ok((updated, _)) = mush_tactics::apply(&now, &plan, at_ic)
        {
            now = updated;
        }
    }

    Ok(now)
}
