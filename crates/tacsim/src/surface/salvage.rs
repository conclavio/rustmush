//! **Salvage on a later visit** (doc 36 cut GC.5; Q-36-16, Q-36-19, Q-36-22, Q-36-23, Q-36-29,
//! Q-36-52's non-friend half; doc 27 FO-1, FO-4, Q-S2, Q-S14, Q-R1).
//!
//! A party that never saw the field finds what it left with a scan, takes another party's
//! casualty into its own carrier's bay, and picks up the cargo that lies on the ground. Four
//! rules, each the host's:
//!
//! - **A scan reveals salvage** (Q-36-23). A scan that lands `Done` writes into its operator's own
//!   `known` record ([`SiteRecord::learn`], Q-CF-28) every casualty and wreck on the site's index,
//!   and every remains and crate on its ground, within [`SALVAGE_SCAN_CELLS`] of the stand. A
//!   remembered thing in reach the scan does not find again is forgotten, as at a field's close.
//!   The scan is the sensor sweep a rig already makes for the ground's chart: no line of sight is
//!   traced, and nothing about it reaches the kernel.
//! - **Custody** (Q-36-16, Q-S2, Q-S14). A casualty loaded into a bay that is not its own party's
//!   or a Friend's is in the custody of that carrier's operator ([`SalvageCustody`], on the
//!   site's record, where the claim is filed), with a claim owed ([`ClaimSeam::Owed`]). Custody is
//!   never title. GC.6 files the claim and moves the seam to [`ClaimSeam::Filed`]
//!   ([`crate::surface::claim`]).
//! - **A written-off lease rig is Occator's alone** (Q-36-22, Q-36-29). Anyone may tow it, but a
//!   load hands it back to Occator's shed with no custody and no claim
//!   (`crate::port::tv::hand_back`), and a custody record over a rig written off later is void
//!   ([`custody_of`]).
//! - **Cargo pickup** (Q-36-19, FO-4). `+tv/<n> pickup [n] [go]` takes a crate lying within a cell
//!   of the rig: the crate leaves the site's record and its manifest moves from the site's spoil
//!   into the rig's hopper by the inventory's own transfer, in one bundle. No goods are made: a
//!   crate whose goods the spoil does not hold is refused, and nothing moves. Doc 36a P2-2: a
//!   crate the hopper has no room for is refused at the command (the mining plan's
//!   merged-compartment rule) and again at the prepare, which re-sums the hopper's pools; the
//!   command says the pickup is queued, never done. On open ground anyone may. On owned ground
//!   anyone still can, physically; the pickup is lawful for the landholder and for the crate's
//!   own hull's holder, and the outcome records it unlawful for anyone else, naming the
//!   landholder (evidence for GC.6, never an omniscient prohibition).

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

use crate::surface::excursion::{cells_apart, site_cell};
use crate::surface::site_record::{GroundKind, KnownItem, KnownWhat, SiteRecord};

/// **A scan's salvage reach**, cells (Q-36-23): the presence picture's visual range
/// ([`crate::surface::presence::SIGHT_CELLS`]). Provisional.
pub const SALVAGE_SCAN_CELLS: u64 = crate::surface::presence::SIGHT_CELLS;
/// **A pickup's reach**, cells: a crate on the rig's own or a neighbouring cell, as the tow's
/// ([`crate::surface::tow::TOW_REACH_CM`]).
pub const PICKUP_REACH_CELLS: u64 = 1;
/// The most custody records one site's record keeps.
pub const MAX_CUSTODY: usize = 32;

/// The scan's line when it found salvage: `{n}`, `{what}`.
pub const SCAN_FOUND: &str = "Rig {n}'s scan also marks what lies on the ground in reach: {what}.";
/// The pickup's preview: `{n}`, `{what}`.
pub const PICKUP_PREVIEW: &str =
    "Rig {n} can take {what} aboard into its hopper. +tv/{n} pickup go";
/// The pickup queued (doc 36a P2-2: the goods move when the act lands, never at the command):
/// `{n}`, `{what}`.
pub const PICKED_UP: &str = "Rig {n} will take {what} aboard into its hopper: queued.";
/// The hopper has no room for the crate (doc 36a P2-2): `{n}`, `{what}`, `{free}`, `{kg}`.
pub const NO_ROOM: &str =
    "Rig {n}'s hopper has no room for {what}: {free} kg free, and the crate is {kg} kg.";
/// Nothing to pick up: `{n}`.
pub const NO_CRATE: &str = "No crate lies within a cell of rig {n}.";
/// The named crate is not in reach: `{n}`, `{id}`.
pub const CRATE_NOT_IN_REACH: &str = "Object {id} is not a crate within a cell of rig {n}.";
/// The rig has no hopper: `{n}`.
pub const NO_HOPPER: &str = "Rig {n} has no hopper to take cargo into.";
/// The spoil does not hold the crate's goods: `{n}`.
pub const CRATE_EMPTY: &str =
    "Rig {n} finds the crate empty: what its manifest names is not on the ground.";
/// `+tv/<n> pickup` usage.
pub const PICKUP_USAGE: &str = "Usage: +tv/<n> pickup [object number] [go].";

// ---- custody ------------------------------------------------------------------------------

/// **Where a salvage claim stands** (Q-36-16; GC.6's seam).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "claim", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimSeam {
    /// Owed: the custodian has a claim to file, and nobody has filed it yet.
    Owed,
    /// Filed as an economy contract (GC.6).
    Filed {
        /// The contract, `ContractId.0`.
        contract: u64,
    },
}

/// **A hull in a salvor's custody** (Q-36-16, Q-S14): loaded into a carrier's bay that is not its
/// own party's or a Friend's. Custody, never title.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SalvageCustody {
    /// The hull.
    pub hull: ObjId,
    /// Who holds it: the carrier's operator.
    pub holder: ObjId,
    /// The carrier whose bay it was loaded into.
    pub carrier: ObjId,
    /// The tug that brought it in.
    pub recovered_by: ObjId,
    /// The IC instant the load completed.
    pub recovered_ic: i64,
    /// The IC instant it was left on this ground (Q-36-22's clock), when the index knew it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_ic: Option<i64>,
    /// The claim.
    pub claim: ClaimSeam,
    /// **Doc 36 cut GC.6**: the tow operation the load closed, which a claim's evidence binds
    /// to. Omitted when unknown (a record written before GC.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<mush_tactics::field_ops::RecoveryOpId>,
    /// **Doc 36 cut GC.6** (Q-S3, Q-36-26): the tenure, title and witnesses pinned when the
    /// attach began, carried from the site's tows at the load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<crate::surface::claim::TenurePin>,
    /// **Doc 36 cut GC.6** (Q-36-24, Q-36-27): the claim as filed, kept by the library's own
    /// rules beside the seam's contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Box<mush_tactics::field_ops::SalvageClaim>>,
    /// **Doc 36 cut GC.6** (Q-36-25): who was told at filing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub told: Vec<ObjId>,
    /// **Doc 36 cut GC.6** (Q-36-28): evidence delivered as a report, or read from the black box
    /// with bay access, at most [`crate::surface::claim::MAX_EVIDENCE`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<mush_tactics::field_ops::SalvageEvidenceRecord>,
    /// **Doc 41 cut RS.3** (Q-41-20, Q-S14): repairs the custodian had made at their own risk
    /// before the claim was filed, carried into the filed claim's own list at filing; at most
    /// [`crate::surface::claim::MAX_AT_RISK`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub at_risk: Vec<mush_tactics::field_ops::AtRiskRepairEntry>,
}

/// **Put `custody` on `record`**, replacing any earlier one for the same hull; past
/// [`MAX_CUSTODY`] the oldest goes.
pub(crate) fn hold(record: &mut SiteRecord, custody: SalvageCustody) {
    record.custody.retain(|row| row.hull != custody.hull);
    let at = record
        .custody
        .partition_point(|row| row.hull < custody.hull);
    record.custody.insert(at, custody);
    while record.custody.len() > MAX_CUSTODY {
        let oldest = record
            .custody
            .iter()
            .enumerate()
            .min_by_key(|(_, row)| (row.recovered_ic, row.hull))
            .map(|(at, _)| at);
        match oldest {
            Some(at) => {
                record.custody.remove(at);
            }
            None => break,
        }
    }
}

/// **The custody record over `hull`**, read from every lane site's record (Q-S14): `None` when
/// nobody holds it, or when it is a lease rig written off since (Q-36-29: Occator's alone).
#[must_use]
pub fn custody_of(ctx: &Ctx<'_>, hull: ObjId) -> Option<SalvageCustody> {
    if crate::port::tv::written_off(ctx.w, hull) {
        return None;
    }
    if let Some(space_row) = ctx
        .w
        .module_state::<crate::space::tick::SpaceBubbleBook>()
        .and_then(|b| b.space_custody.get(&hull).cloned())
    {
        return Some(space_row);
    }
    let handles = ctx.w.module_state::<crate::port::families::PortHandles>()?;
    crate::surface::chart::lane_sites(ctx.w)
        .into_iter()
        .find_map(|site| {
            let key = crate::surface::site_record::SiteKey(site.entry);
            let record = ctx
                .action_state(&handles.families.sites, &key)
                .ok()
                .flatten()?;
            record.custody.iter().find(|row| row.hull == hull).cloned()
        })
}

// ---- the scan -----------------------------------------------------------------------------

/// **What a scan from `stand` finds** (Q-36-23): every indexed casualty and wreck but the scanner
/// itself, and every object on the ground, within `reach` cells.
#[must_use]
pub fn scan_finds(
    record: &SiteRecord,
    stand: (i64, i64),
    reach: u64,
    scanner: ObjId,
) -> Vec<KnownItem> {
    let mut found: Vec<KnownItem> = Vec::new();
    for left in record.units.iter().filter(|left| left.unit != scanner) {
        let cell = left.pose.cell();
        if cells_apart(stand, cell) <= reach {
            found.push(KnownItem(
                KnownWhat::of_unit(left.condition),
                cell.0,
                cell.1,
            ));
        }
    }
    for object in &record.objects {
        let cell = object.pose.cell();
        if cells_apart(stand, cell) <= reach.saturating_add(u64::from(object.footprint / 2)) {
            found.push(KnownItem(
                KnownWhat::of_object(&object.kind),
                cell.0,
                cell.1,
            ));
        }
    }
    found
}

/// **`principal` learns what a scan from `stand` finds** (Q-36-23), the reach swept: answers the
/// things found, and whether the record changed.
pub fn learn_from_scan(
    record: &mut SiteRecord,
    principal: ObjId,
    (stand, scanner): ((i64, i64), ObjId),
    at_ic: i64,
) -> (Vec<KnownItem>, bool) {
    let found = scan_finds(record, stand, SALVAGE_SCAN_CELLS, scanner);
    let before = record.known_by(principal).to_vec();
    record.learn(
        principal,
        &found,
        |cell| cells_apart(stand, cell) <= SALVAGE_SCAN_CELLS,
        at_ic,
    );
    let changed = record.known_by(principal) != before.as_slice();
    (found, changed)
}

/// The scan's line for what it found, `None` when nothing.
#[must_use]
pub fn found_words(found: &[KnownItem]) -> Option<String> {
    if found.is_empty() {
        return None;
    }
    let mut counts: BTreeMap<KnownWhat, usize> = BTreeMap::new();
    for item in found {
        *counts.entry(item.0).or_default() += 1;
    }
    Some(
        counts
            .into_iter()
            .map(|(what, n)| {
                if n == 1 {
                    format!("a {}", what.word())
                } else {
                    format!("{n} {}", plural(what))
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
    )
}

const fn plural(what: KnownWhat) -> &'static str {
    match what {
        KnownWhat::Casualty => "casualties",
        KnownWhat::Wreck => "wrecks",
        KnownWhat::Remains => "remains",
        KnownWhat::Crate => "crates",
    }
}

// ---- cargo pickup -------------------------------------------------------------------------

/// **A pickup's facts**, fixed by the server for the tow provider's prepare.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickupFacts {
    /// The site.
    pub site: CanonicalId,
    /// The crate's number on the site.
    pub object: u32,
    /// Its manifest, as the command read it.
    pub manifest: BTreeMap<CanonicalId, u64>,
    /// The site's spoil vault, `VaultId.0`: where a crate's goods lie.
    pub spoil: u64,
    /// The rig's hopper vault, `VaultId.0`.
    pub hopper: u64,
    /// Whether the pickup is lawful under FO-4.
    pub lawful: bool,
    /// **The hopper's limits** (doc 36a P2-2), measure → limit, every compartment merged: what
    /// the prepare's re-check holds the hopper to.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub limits: BTreeMap<CanonicalId, u64>,
    /// **The hopper's goods' figures** (doc 36a P2-2): each good the hopper accepts (the
    /// manifest's among them), measure → per unit, so the prepare can re-sum the hopper's pools
    /// without the goods table.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub figures: BTreeMap<CanonicalId, BTreeMap<CanonicalId, u64>>,
    /// **Whose ground the crate lay on** (doc 36a P2-2; Q-36-30): the landholder the evidence
    /// log names when the pickup is unlawful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landholder: Option<ObjId>,
}

/// The hopper's limits and its goods' figures, as [`hopper_room`] fixes them for the prepare.
type HopperFigures = (
    BTreeMap<CanonicalId, u64>,
    BTreeMap<CanonicalId, BTreeMap<CanonicalId, u64>>,
);

/// **The hopper's room against a crate** (doc 36a P2-2; the mining plan's merged-compartment
/// rule, `crate::surface::act::commands`): the rig's compartments merged into one, the hopper's
/// pools summed per measure, the crate's manifest added. `Ok((limits, figures))` when it fits;
/// `Err((free_kg, crate_kg))` when a measure would overrun, with the mass figures for the line;
/// `None` when the rig's unit or the goods table cannot be read (the command refuses as
/// [`NO_HOPPER`]).
fn hopper_room(
    ctx: &Ctx<'_>,
    tv: ObjId,
    manifest: &BTreeMap<CanonicalId, u64>,
) -> Option<Result<HopperFigures, (u64, u64)>> {
    use crate::port::capacity;
    let pool_unit = crate::port::tv::pool_unit(ctx.w);
    let goods = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .and_then(crate::store::TacsimStore::goods)?;
    let comps = crate::port::tv::hopper_compartments(ctx.w, tv, pool_unit.as_ref())?;
    let mut merged = capacity::Compartment {
        id: CanonicalId::new("hopper").ok()?,
        accepts: std::collections::BTreeSet::new(),
        limits: BTreeMap::new(),
    };
    for comp in comps {
        merged.accepts.extend(comp.accepts);
        for (measure, limit) in comp.limits {
            let slot = merged.limits.entry(measure).or_insert(0);
            *slot = slot.saturating_add(limit);
        }
    }
    let contents =
        crate::port::tv::hopper_contents_with(ctx.w, tv, pool_unit.as_ref(), &|endpoint, good| {
            crate::port_tick::read_pool(ctx, endpoint, good)
        })
        .ok()?;
    let held = capacity::load(goods, &merged, &contents).ok()?;
    let adds = capacity::load(goods, &merged, manifest).ok()?;
    let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).ok()?;
    let over = merged.limits.iter().any(|(measure, limit)| {
        held.get(measure)
            .copied()
            .unwrap_or(0)
            .saturating_add(adds.get(measure).copied().unwrap_or(0))
            > *limit
    });
    if over {
        let free = merged
            .limits
            .get(&mass)
            .copied()
            .unwrap_or(0)
            .saturating_sub(held.get(&mass).copied().unwrap_or(0));
        return Some(Err((free, adds.get(&mass).copied().unwrap_or(0))));
    }
    let (accepted, _) = crate::port::tv::hopper_goods_of(ctx.w, tv)?;
    let mut figures = BTreeMap::new();
    for good in accepted.iter().chain(manifest.keys()) {
        let def = goods.good(good)?;
        let per: BTreeMap<CanonicalId, u64> = merged
            .limits
            .keys()
            .filter_map(|measure| def.per_unit(measure).map(|per| (measure.clone(), per)))
            .collect();
        figures.insert(good.clone(), per);
    }
    Some(Ok((merged.limits, figures)))
}

/// **Whether `actor` may lawfully take what lies at `cell` on `site`** (FO-4): open ground, or
/// owned ground whose landholder is `actor`, or a crate whose own hull `actor` holds.
fn lawful(ctx: &Ctx<'_>, site: &CanonicalId, cell: (i64, i64), from: Option<ObjId>) -> bool {
    use crate::port::site_map::SurfaceReads;
    let actor = ctx.actor;
    if from.is_some_and(|hull| {
        crate::port::coop::helm_holder(ctx.w, hull) == Some(actor)
            || ctx
                .w
                .module_state::<crate::store::TacsimStore>()
                .and_then(|store| store.tv_lease_on(hull))
                .is_some_and(|lease| lease.pilot == actor)
    }) {
        return true;
    }
    let scheme = crate::surface::chart::site_scheme(ctx.w, site);
    let pose = mush_tactics::field_ops::FieldPose {
        site: ObjId(1),
        x_cm: cell.0.saturating_mul(1_000).saturating_add(500),
        y_cm: cell.1.saturating_mul(1_000).saturating_add(500),
    };
    let Some(letter) = crate::field_ops::block_letter_at_site_pose(pose, scheme) else {
        return true;
    };
    let now = ctx.w.world_ic_now().at;
    let claim = ctx
        .block_tenure(&crate::surface::claims::BlockKey {
            site: site.clone(),
            letter,
        })
        .ok()
        .flatten()
        .and_then(|record| record.claim)
        .filter(|claim| claim.live(now));
    match claim {
        None => true,
        Some(claim) => claim.holder == PrincipalId::Object(actor),
    }
}

/// The crate's words: `a crate of 2 metal_ore`.
fn crate_words(manifest: &BTreeMap<CanonicalId, u64>) -> String {
    crate::surface::site_record::object_words(&GroundKind::Cargo {
        manifest: manifest.clone(),
        from: None,
    })
}

/// **`+tv/<n> pickup [n] [go]`** (this module's head).
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn pickup_cmd(
    ctx: &mut Ctx<'_>,
    rig: &crate::surface::act::RigAt,
    named: Option<u32>,
    go: bool,
) -> Result<(), CmdError> {
    use crate::port::site_map::SurfaceReads;
    let n = rig.slot.to_string();
    let (site, pose) = crate::surface::tow::ground_pose(ctx.w, rig.tv)
        .ok_or_else(|| CmdError::msg(crate::surface::tow::NOT_ON_GROUND.replace("{n}", &n)))?;
    let here = site_cell(pose.x_cm, pose.y_cm);
    let record = crate::surface::tow::record_or_new(ctx, &site);
    let crates: Vec<&crate::surface::site_record::GroundObject> = record
        .objects
        .iter()
        .filter(|object| matches!(object.kind, GroundKind::Cargo { .. }))
        .filter(|object| cells_apart(here, object.pose.cell()) <= PICKUP_REACH_CELLS)
        .collect();
    let found = match named {
        Some(id) => crates
            .iter()
            .find(|object| object.id == id)
            .copied()
            .ok_or_else(|| {
                CmdError::msg(
                    CRATE_NOT_IN_REACH
                        .replace("{n}", &n)
                        .replace("{id}", &id.to_string()),
                )
            })?,
        None => crates
            .first()
            .copied()
            .ok_or_else(|| CmdError::msg(NO_CRATE.replace("{n}", &n)))?,
    };
    let GroundKind::Cargo { manifest, from } = &found.kind else {
        return Err(CmdError::msg(NO_CRATE.replace("{n}", &n)));
    };
    let hopper = crate::port::vehicle::hopper_of(ctx.w, rig.tv)
        .ok_or_else(|| CmdError::msg(NO_HOPPER.replace("{n}", &n)))?;
    let spoil = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|store| store.port_maps().find(|map| map.entry == site))
        .and_then(|map| crate::port::site::spoil_vault(ctx.w, map))
        .ok_or_else(|| CmdError::msg(CRATE_EMPTY.replace("{n}", &n)))?;
    let held = manifest.iter().all(|(good, qty)| {
        ctx.available(mush_inventory::EndpointRef(spoil), good)
            .is_ok_and(|have| have >= *qty)
    });
    if !held {
        return Err(CmdError::msg(CRATE_EMPTY.replace("{n}", &n)));
    }
    let what = crate_words(manifest);
    // Doc 36a P2-2: the hopper's room, at the command and again at the prepare.
    let (limits, figures) = match hopper_room(ctx, rig.tv, manifest) {
        None => return Err(CmdError::msg(NO_HOPPER.replace("{n}", &n))),
        Some(Err((free, kg))) => {
            return Err(CmdError::msg(
                NO_ROOM
                    .replace("{n}", &n)
                    .replace("{what}", &what)
                    .replace("{free}", &free.to_string())
                    .replace("{kg}", &kg.to_string()),
            ));
        }
        Some(Ok(room)) => room,
    };
    if !go {
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.tow",
            PICKUP_PREVIEW.replace("{n}", &n).replace("{what}", &what),
        );
        return Ok(());
    }
    let cell = found.pose.cell();
    let facts = PickupFacts {
        lawful: lawful(ctx, &site, cell, *from),
        landholder: match crate::surface::claim::tenure_at(ctx, &site, cell) {
            mush_tactics::field_ops::GroundTenure::Owned { landholder, .. } => Some(landholder),
            mush_tactics::field_ops::GroundTenure::Unclaimed => None,
        },
        site,
        object: found.id,
        manifest: manifest.clone(),
        spoil: spoil.0,
        hopper: hopper.0,
        limits,
        figures,
    };
    let actor = ctx.actor;
    if !crate::surface::tow::submit(
        ctx,
        actor,
        rig.tv,
        crate::surface::tow::TowOrder::Pickup {
            facts: Box::new(facts),
        },
        0,
    ) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(
        actor,
        "tacsim.tow",
        PICKED_UP.replace("{n}", &n).replace("{what}", &what),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::site_record::{LeftCondition, LeftUnit, SitePose};

    fn pose(x: i64, y: i64) -> SitePose {
        SitePose {
            x_cm: x * 1_000 + 500,
            y_cm: y * 1_000 + 500,
            heading_mdeg: 0,
        }
    }

    fn ground() -> SiteRecord {
        let mut record = SiteRecord::new(CanonicalId::new("tacsim.port.ahuna").expect("an id"));
        for (unit, x, condition) in [
            (7, 104, LeftCondition::Casualty),
            (8, 150, LeftCondition::Wreck),
        ] {
            record
                .leave_unit(LeftUnit {
                    unit: ObjId(unit),
                    pose: pose(x, 100),
                    condition,
                    epoch: None,
                    left_ic: 10,
                })
                .expect("left");
        }
        record
            .drop_object(
                pose(110, 100),
                1,
                GroundKind::Remains {
                    of: None,
                    mass_kg: 4_000,
                    merged: 1,
                },
                10,
            )
            .expect("dropped");
        record
    }

    /// **Q-36-23**: a scan finds the casualty and the remains in reach, never the scanner and
    /// never what lies past the reach, into the scanner's own record alone.
    #[test]
    fn gc5_a_scan_finds_the_salvage_in_reach_into_the_scanners_record() {
        let mut record = ground();
        let (found, changed) = learn_from_scan(&mut record, ObjId(42), ((100, 100), ObjId(9)), 20);
        assert!(changed);
        assert_eq!(
            found,
            vec![
                KnownItem(KnownWhat::Casualty, 104, 100),
                KnownItem(KnownWhat::Remains, 110, 100),
            ]
        );
        assert_eq!(record.known_by(ObjId(42)).len(), 2);
        assert!(record.known_by(ObjId(43)).is_empty(), "nobody else learns");
        // Scanned again with the casualty gone: forgotten; nothing changes on a third look.
        record.take_unit(ObjId(7)).expect("taken");
        let (_, changed) = learn_from_scan(&mut record, ObjId(42), ((100, 100), ObjId(9)), 30);
        assert!(changed);
        assert_eq!(
            record.known_by(ObjId(42)),
            &[KnownItem(KnownWhat::Remains, 110, 100)]
        );
        let (_, changed) = learn_from_scan(&mut record, ObjId(42), ((100, 100), ObjId(9)), 40);
        assert!(
            !changed,
            "a look that finds what it knew writes nothing new"
        );
        assert_eq!(
            found_words(&[
                KnownItem(KnownWhat::Casualty, 1, 1),
                KnownItem(KnownWhat::Crate, 2, 2),
                KnownItem(KnownWhat::Crate, 3, 3),
            ])
            .as_deref(),
            Some("a casualty, 2 crates")
        );
    }

    /// **Q-36-16**: custody is one record a hull, kept in order and bounded.
    #[test]
    fn gc5_custody_is_one_record_a_hull() {
        let mut record = ground();
        let custody = |hull: u32, at: i64| SalvageCustody {
            hull: ObjId(hull),
            holder: ObjId(2),
            carrier: ObjId(3),
            recovered_by: ObjId(4),
            recovered_ic: at,
            left_ic: Some(1),
            claim: ClaimSeam::Owed,
            operation: None,
            pin: None,
            file: None,
            told: Vec::new(),
            evidence: Vec::new(),
            at_risk: Vec::new(),
        };
        hold(&mut record, custody(9, 5));
        hold(&mut record, custody(7, 6));
        hold(&mut record, custody(9, 8));
        assert_eq!(
            record
                .custody
                .iter()
                .map(|row| (row.hull.0, row.recovered_ic))
                .collect::<Vec<_>>(),
            vec![(7, 6), (9, 8)]
        );
        record.check().expect("the record checks");
        for hull in 100..(100 + u32::try_from(MAX_CUSTODY).expect("small")) {
            hold(&mut record, custody(hull, i64::from(hull)));
        }
        assert_eq!(record.custody.len(), MAX_CUSTODY);
        assert!(record.custody.iter().all(|row| row.hull != ObjId(7)));
    }
}
