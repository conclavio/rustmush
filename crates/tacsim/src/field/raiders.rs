//! **Fixture raiders** (doc 32 cut CF.6; Q-CF-44, Q-CF-23).
//!
//! An opposing force for test and for play, authored as content (`content/field/raiders.json`)
//! like the Slack Line crew: a party of armed hulls one NPC leader stands behind, at a named
//! frontier site. Staff seed it there (`@field/raiders <id>`); nothing else ever makes one — no
//! generator, no respawn, no patrol logic. Seeding is refused while any hull of that raider
//! still stands, so a destroyed raider stays destroyed until staff seed a fresh one.
//!
//! **What makes a hull a raider's** is one staff-owned attribute on it, [`RAIDER_ATTR`], naming
//! the raider and its leader. The field's admission keys every such hull into one party,
//! `PartyKey::Fixture { leader }`, led by that NPC (`super::parties`), and that party opens
//! holding a standing Hostile mark on every other party (`super::admit`): a raider fights
//! whoever it finds, with no player connected, and the field runs to its own demotion (Q-CF-23)
//! — by the raider's destruction, or by a quiet interval once nobody is left to fight. Each hull
//! flies its ROM seal (Standard Doctrine, the kernel's), never a second AI.
//!
//! **Doctrine never opens a field by itself** (doc 32 §4a): a seeded raider sits on the ground
//! until its leader marks a party there Hostile through the ordinary door (`+field/mark`), which
//! a test or a staff hand drives as the fixture actor.
//!
//! **Its carrier** (doc 32 cut CF.6b; Q-CF-44, Q-CF-47): a raider launches from a fixture
//! carrier on the site, as the escort flow has it. The carrier is a hull of the raider like the
//! others — the same attribute, so the same party — standing still at its authored cell (the
//! Dray's top speed is nil), unarmed, and admitted to a field as a unit like CF.3's; it is not a
//! programme hull, so it has no pad, no bay and no voyage, only the ramp its content names. The
//! seed puts the carrier on the ground and its hulls aboard (made, never placed);
//! `@field/raiders/launch <id>` puts every hull aboard on the ground beside the ramp, nearest
//! ring first, and is refused while the site holds a field (a mid-field arrival is its own cut).
//!
//! **Its withdrawal** (doc 32 cut CF.6b; Q-CF-18): `@field/raiders/withdraw <id>` is the
//! leader's WITHDRAW to every hull of the raider standing in a field, handed to the kernel as a
//! roster's order is (the DT.1b adapter), under the raider's own fence. Its hulls' doctrine
//! drives home — a field party's home is its own carrier's ramp
//! (`crate::arena::state::ArenaDefinition::homes`) — and the host asks the kernel to let each
//! one go at the ramp every pass until it can (`super::home`); once out it is aboard again. The
//! order is marked on the hull ([`RAIDER_ATTR`]'s `withdraw`), so a restart keeps asking.

use mush_core::{
    ArgShape, Attr, CanonicalId, CmdError, CommandSpec, Ctx, Flags, GOD, ObjId, ObjKind, World,
};
use serde::Deserialize;

/// The raiders the content authors.
pub const RAIDERS_JSON: &str = include_str!("../../content/field/raiders.json");
/// The staff verb that seeds one.
pub const RAIDERS_VERB: &str = "@field";
/// **The attribute that makes a hull a raider's**: `{"raider": <id>, "leader": <dbref>}`, owned
/// by `GOD` (a player cannot forge one).
pub const RAIDER_ATTR: &str = "TACSIM.FIELD.RAIDER";
/// The verb's help.
pub const HELP: &str = "@field/raiders lists the fixture raiders the content authors. \
@field/raiders <id> seeds one on its named site: its leader, its carrier on the site's ground and \
its armed hulls aboard. Refused while any hull of that raider still stands. \
@field/raiders/launch <id> puts the hulls aboard on the ground beside the carrier's ramp. \
@field/raiders/withdraw <id> orders every hull in a field home up the ramp. Staff only.";

/// **How far from the ramp a launch may stand a hull**, cells (Chebyshev).
pub const LAUNCH_SPREAD_CELLS: u16 = 4;
/// **How far from its carrier an authored ramp may be**, cells (Chebyshev).
pub const RAMP_REACH_CELLS: i64 = 4;
/// **The fence a raider's orders stand under**: this bit over the leader's id, so it never
/// meets a roster's detachment id.
pub const RAIDER_DETACHMENT: u64 = 1 << 62;

/// One authored hull, aboard until it is launched.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaiderUnit {
    /// The unit design (`tacsim.unit.line_tank`).
    pub unit: CanonicalId,
    /// The hull's name.
    pub name: String,
    /// Its heading once launched, millidegrees.
    pub heading_mdeg: u32,
}

/// **The authored carrier** (doc 32 cut CF.6b).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaiderCarrier {
    /// The unit design (`tacsim.unit.dray`).
    pub unit: CanonicalId,
    /// Its name.
    pub name: String,
    /// Its site cell.
    pub cell: [i64; 2],
    /// Its heading, millidegrees.
    pub heading_mdeg: u32,
    /// **Its ramp's site cell**: where its hulls launch and where they come home.
    pub ramp: [i64; 2],
}

/// One authored raider.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaiderDef {
    /// Its id.
    pub id: CanonicalId,
    /// What it is called.
    pub label: String,
    /// Its leader's name (an NPC made at the seed).
    pub leader: String,
    /// The site's port entry it is seeded on.
    pub site: CanonicalId,
    /// The carrier its hulls launch from.
    pub carrier: RaiderCarrier,
    /// Its hulls.
    pub units: Vec<RaiderUnit>,
}

/// The file.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RaidersFile {
    schema: u16,
    #[allow(dead_code)] // the authored note, read by people
    brief: String,
    raiders: Vec<RaiderDef>,
}

/// **The authored raiders**, checked: schema 2, unique ids, one to 22 hulls each beside the
/// carrier (a party's two-letter designators), the carrier and its ramp on the site grid, and
/// the ramp beside the carrier, never under it.
///
/// # Errors
///
/// The first broken rule, as a sentence.
pub fn raiders() -> Result<Vec<RaiderDef>, String> {
    let file: RaidersFile =
        serde_json::from_str(RAIDERS_JSON).map_err(|error| format!("raiders: {error}"))?;
    if file.schema != 2 {
        return Err("raiders: schema 2 only".to_owned());
    }
    // Doc 32 cut S.0: the widest site here; `seed` holds each to its own site's side.
    let side = i64::from(crate::surface::ground::MAX_SITE_CELLS);
    for (n, def) in file.raiders.iter().enumerate() {
        if file.raiders[..n].iter().any(|other| other.id == def.id) {
            return Err(format!("raiders: {} is authored twice", def.id));
        }
        if def.units.is_empty() || def.units.len() > 22 {
            return Err(format!("raiders: {} fields one to 22 hulls", def.id));
        }
        if def.leader.trim().is_empty() || def.label.trim().is_empty() {
            return Err(format!("raiders: {} names its leader and itself", def.id));
        }
        let carrier = &def.carrier;
        let apart = (carrier.cell[0] - carrier.ramp[0])
            .abs()
            .max((carrier.cell[1] - carrier.ramp[1]).abs());
        if carrier
            .cell
            .iter()
            .chain(&carrier.ramp)
            .any(|v| !(0..side).contains(v))
            || carrier.heading_mdeg >= 360_000
            || !(1..=RAMP_REACH_CELLS).contains(&apart)
        {
            return Err(format!(
                "raiders: {}'s carrier and its ramp stand on the site, the ramp beside it",
                def.id
            ));
        }
        if def.units.iter().any(|unit| unit.heading_mdeg >= 360_000) {
            return Err(format!("raiders: {} has a heading off the turn", def.id));
        }
    }
    Ok(file.raiders)
}

/// **The raider leader a hull stands behind**, when it is a seeded raider's and the leader is a
/// live NPC.
#[must_use]
pub fn leader_of(w: &World, unit: ObjId) -> Option<ObjId> {
    let attr = w.attr(unit, RAIDER_ATTR).filter(|attr| attr.owner == GOD)?;
    let leader = attr
        .as_json()?
        .get("leader")?
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .map(ObjId)?;
    w.get(leader)
        .filter(|object| object.kind == ObjKind::Npc && !object.flags.contains(Flags::GOING))
        .map(|_| leader)
}

/// The hull's raider attribute, when staff wrote it.
fn mark_of(w: &World, unit: ObjId) -> Option<serde_json::Value> {
    w.attr(unit, RAIDER_ATTR)
        .filter(|attr| attr.owner == GOD)?
        .as_json()
        .cloned()
}

/// The raider a hull is seeded for, by id, whatever its leader's state.
fn raider_of(w: &World, unit: ObjId) -> Option<String> {
    mark_of(w, unit)?.get("raider")?.as_str().map(str::to_owned)
}

/// **The fixture carrier a raider's hull launched from** (doc 32 cut CF.6b).
#[must_use]
pub fn carrier_of(w: &World, unit: ObjId) -> Option<ObjId> {
    mark_of(w, unit)?
        .get("carrier")?
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .map(ObjId)
}

/// **A fixture carrier's ramp, as a site cell**, when `carrier` is one.
#[must_use]
pub fn ramp_cell(w: &World, carrier: ObjId) -> Option<[i64; 2]> {
    let mark = mark_of(w, carrier)?;
    match mark.get("ramp")?.as_array()?.as_slice() {
        [x, y] => Some([x.as_i64()?, y.as_i64()?]),
        _ => None,
    }
}

/// **The ramp a raider's hull comes home to**, as a site position (cm): its carrier's ramp
/// cell's centre (doc 32 cut CF.6b).
#[must_use]
pub fn hull_ramp(w: &World, unit: ObjId) -> Option<(i64, i64)> {
    let cell = ramp_cell(w, carrier_of(w, unit)?)?;
    let pose = crate::surface::excursion::cell_pose(cell, 0).ok()?;
    Some((pose.x_cm, pose.y_cm))
}

/// Whether the raider's hull carries its leader's standing WITHDRAW.
#[must_use]
pub fn withdrawing(w: &World, unit: ObjId) -> bool {
    mark_of(w, unit)
        .and_then(|mark| mark.get("withdraw").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// Every hull seeded for `raider`, the carrier included, in unit order.
fn hulls_of(w: &World, raider: &str) -> Vec<ObjId> {
    w.module_state::<mush_tactics::module::UnitStore>()
        .map(|store| {
            store
                .units
                .keys()
                .copied()
                .filter(|unit| raider_of(w, *unit).as_deref() == Some(raider))
                .collect()
        })
        .unwrap_or_default()
}

/// Rewrite one key of a hull's raider attribute (`None` removes it).
fn set_mark(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    key: &str,
    value: Option<serde_json::Value>,
) -> Result<(), String> {
    let mut mark = mark_of(ctx.w, unit).ok_or("the hull is no raider's")?;
    let Some(map) = mark.as_object_mut() else {
        return Err("the hull's raider attribute is not an object".to_owned());
    };
    match value {
        Some(value) => {
            map.insert(key.to_owned(), value);
        }
        None => {
            map.remove(key);
        }
    }
    ctx.w
        .set_attr(unit, RAIDER_ATTR, Attr::json(mark, GOD))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The verb's spec.
#[must_use]
pub fn spec() -> CommandSpec {
    CommandSpec {
        name: RAIDERS_VERB.into(),
        switches: vec!["raiders".into(), "launch".into(), "withdraw".into()],
        shape: ArgShape::Raw,
        help: HELP.into(),
        ..CommandSpec::default()
    }
}

/// **`@field/raiders [<id>]`**.
///
/// # Errors
///
/// The refusal line.
pub fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !(mush_core::perm::has_power(ctx.w, ctx.actor, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(ctx.w, ctx.actor, mush_core::Role::Royalty))
    {
        return Err(CmdError::msg("Permission denied."));
    }
    if !ctx.switches.has("raiders") {
        return Err(CmdError::msg("Usage: @field/raiders [<id>]"));
    }
    let defs = raiders().map_err(CmdError::msg)?;
    let wanted = ctx.args.raw().trim().to_owned();
    if wanted.is_empty() {
        for def in &defs {
            let line = format!(
                "{}: {}, {} hulls led by {}, at {}.",
                def.id,
                def.label,
                def.units.len(),
                def.leader,
                crate::surface::act::site_name(ctx.w, &def.site)
            );
            ctx.out.sys_to_actor(ctx.actor, "tacsim.field", line);
        }
        return Ok(());
    }
    let def = defs
        .into_iter()
        .find(|def| def.id.as_str() == wanted)
        .ok_or_else(|| CmdError::msg("No raider is authored with that id."))?;
    let line = if ctx.switches.has("launch") {
        launch(ctx, &def)
    } else if ctx.switches.has("withdraw") {
        withdraw(ctx, &def)
    } else {
        seed(ctx, &def)
    }
    .map_err(CmdError::msg)?;
    ctx.out.sys_to_actor(ctx.actor, "tacsim.field", line);
    Ok(())
}

/// **Seed `def` on its site**: its leader, its carrier made, placed and marked, then each hull
/// made and marked aboard it (never placed: [`launch`] puts them on the ground).
///
/// # Errors
///
/// Why nothing was seeded (a standing hull of the same raider, an unknown site or design), or a
/// failure part-way, as a sentence.
pub fn seed(ctx: &mut Ctx<'_>, def: &RaiderDef) -> Result<String, String> {
    if !hulls_of(ctx.w, def.id.as_str()).is_empty() {
        return Err(format!(
            "{} still has a hull standing; a raider is never respawned.",
            def.label
        ));
    }
    let domain = super::parties::site_domain(ctx.w, &def.site)
        .ok_or_else(|| format!("{} is not a lane site.", def.site))?;
    let side = i64::from(crate::surface::site_record::site_side_or_default(
        ctx.w, &def.site,
    ));
    if def
        .carrier
        .cell
        .iter()
        .chain(&def.carrier.ramp)
        .any(|v| !(0..side).contains(v))
    {
        return Err(format!(
            "{}'s carrier and its ramp stand off {}.",
            def.label, def.site
        ));
    }
    let spec_of = |ctx: &Ctx<'_>, design: &CanonicalId| {
        ctx.w
            .module_state::<crate::TacsimStore>()
            .ok_or_else(|| "The tactical layer is not loaded.".to_owned())?
            .definitions()
            .bindings
            .get(design)
            .map(|binding| binding.spec.clone())
            .ok_or_else(|| format!("No unit design {design} is bound."))
    };
    let carrier_spec = spec_of(ctx, &def.carrier.unit)?;
    let specs = def
        .units
        .iter()
        .map(|unit| spec_of(ctx, &unit.unit))
        .collect::<Result<Vec<_>, _>>()?;
    let leader = ctx
        .w
        .create(ObjKind::Npc, &def.leader, GOD, None)
        .map_err(|error| format!("The raider's leader was not made ({error})."))?;
    let carrier = make(
        ctx,
        &def.carrier.name,
        &carrier_spec,
        serde_json::json!({
            "raider": def.id.as_str(),
            "leader": leader.0,
            "ramp": def.carrier.ramp,
        }),
    )?;
    let pose = crate::surface::excursion::cell_pose(def.carrier.cell, def.carrier.heading_mdeg)?;
    mush_sim::place(ctx, carrier, domain, pose)
        .map_err(|error| format!("{} was not placed ({error}).", def.carrier.name))?;
    for (unit, spec) in def.units.iter().zip(specs) {
        make(
            ctx,
            &unit.name,
            &spec,
            serde_json::json!({
                "raider": def.id.as_str(),
                "leader": leader.0,
                "carrier": carrier.0,
            }),
        )?;
    }
    Ok(format!(
        "{} seeded at {}: the {} on the ground with {} hulls aboard, led by {} (#{}).",
        def.label,
        crate::surface::act::site_name(ctx.w, &def.site),
        def.carrier.name,
        def.units.len(),
        def.leader,
        leader.0
    ))
}

/// One hull made, its unit record built and its raider attribute written; never placed.
fn make(
    ctx: &mut Ctx<'_>,
    name: &str,
    spec: &mush_tactics::UnitSpec,
    mark: serde_json::Value,
) -> Result<ObjId, String> {
    let obj = ctx
        .w
        .create(ObjKind::Thing, name, GOD, None)
        .map_err(|error| format!("{name} was not made ({error})."))?;
    mush_tactics::module::create_unit(ctx, obj, spec)?;
    ctx.w
        .set_attr(obj, RAIDER_ATTR, Attr::json(mark, GOD))
        .map_err(|error| format!("{name} was not marked ({error})."))?;
    Ok(obj)
}

/// **Launch `def`'s hulls aboard** (this module's head): each on the ground beside its carrier's
/// ramp, on the nearest free cell it can stand on, nearest ring first.
///
/// # Errors
///
/// Why nothing launched: no carrier on the site's ground, the site in a field, nobody aboard,
/// no free ground beside the ramp.
pub fn launch(ctx: &mut Ctx<'_>, def: &RaiderDef) -> Result<String, String> {
    let domain = super::parties::site_domain(ctx.w, &def.site)
        .ok_or_else(|| format!("{} is not a lane site.", def.site))?;
    let hulls = hulls_of(ctx.w, def.id.as_str());
    let carrier = hulls
        .iter()
        .copied()
        .find(|hull| ramp_cell(ctx.w, *hull).is_some())
        .filter(|carrier| mush_sim::pose_of(ctx.w, *carrier).is_some_and(|(on, _)| on == domain))
        .ok_or_else(|| format!("{} has no carrier on the ground to launch from.", def.label))?;
    let record = super::close::site_record(ctx, &def.site)
        .unwrap_or_else(|| crate::surface::site_record::fresh_record(ctx.w, &def.site));
    if record.promoted.is_some() {
        return Err(format!(
            "{} is in a field; a raider launches only onto open ground.",
            crate::surface::act::site_name(ctx.w, &def.site)
        ));
    }
    let aboard: Vec<ObjId> = hulls
        .iter()
        .copied()
        .filter(|hull| carrier_of(ctx.w, *hull) == Some(carrier))
        .filter(|hull| mush_sim::pose_of(ctx.w, *hull).is_none())
        .collect();
    if aboard.is_empty() {
        return Err(format!("{} has nobody aboard to launch.", def.label));
    }
    let ground = super::ground::definition(ctx.w, &def.site, &record, Vec::new())?;
    let terrain = &ground.terrain;
    let mut taken: std::collections::BTreeSet<(u16, u16)> =
        super::parties::on_ground(ctx.w, domain)
            .into_iter()
            .filter_map(|unit| mush_sim::pose_of(ctx.w, unit))
            .filter_map(|(_, pose)| terrain.cell_of(&pose))
            .collect();
    let ramp = ramp_cell(ctx.w, carrier).ok_or("the carrier names no ramp")?;
    let ramp = (
        u16::try_from(ramp[0]).map_err(|_| "the ramp is off the site")?,
        u16::try_from(ramp[1]).map_err(|_| "the ramp is off the site")?,
    );
    for hull in &aboard {
        let row = mush_tactics::module::unit_of(ctx.w, *hull)
            .map(crate::arena::state::CostRow::read)
            .ok_or_else(|| format!("#{} has no tactical unit record", hull.0))?;
        let cell = (1..=LAUNCH_SPREAD_CELLS)
            .flat_map(|ring| super::carrier::ring_cells(ramp, ring))
            .find(|cell| !taken.contains(cell) && terrain.passable_for(row, cell.0, cell.1))
            .ok_or_else(|| format!("The ground beside the {} is full.", def.carrier.name))?;
        taken.insert(cell);
        let name = crate::port::coop::name_of(ctx.w, *hull);
        let heading = def
            .units
            .iter()
            .find(|unit| unit.name == name)
            .map_or(def.carrier.heading_mdeg, |unit| unit.heading_mdeg);
        let pose =
            crate::surface::excursion::cell_pose([i64::from(cell.0), i64::from(cell.1)], heading)?;
        mush_sim::place(ctx, *hull, domain, pose)
            .map_err(|error| format!("#{} was not placed ({error}).", hull.0))?;
    }
    Ok(format!(
        "{} launch from the {}: {} hulls on the ground beside its ramp.",
        def.label,
        def.carrier.name,
        aboard.len()
    ))
}

/// **The leader's WITHDRAW to every hull of `def` in a field** (this module's head): marked on
/// the hull, then handed to the kernel under the raider's own fence.
///
/// # Errors
///
/// No hull of the raider stands in a field.
pub fn withdraw(ctx: &mut Ctx<'_>, def: &RaiderDef) -> Result<String, String> {
    let in_field: Vec<ObjId> = hulls_of(ctx.w, def.id.as_str())
        .into_iter()
        .filter(|hull| carrier_of(ctx.w, *hull).is_some())
        .filter(|hull| super::field_of_unit(ctx.w, *hull).is_some())
        .collect();
    if in_field.is_empty() {
        return Err(format!("No hull of {} is in a field.", def.label));
    }
    for hull in &in_field {
        set_mark(ctx, *hull, "withdraw", Some(serde_json::Value::Bool(true)))?;
        // The host's pass offers it again until the kernel holds it (`super::home`).
        let _ = order_home(ctx, *hull);
    }
    Ok(format!(
        "{} withdraw: {} hulls make for the {}'s ramp.",
        def.label,
        in_field.len(),
        def.carrier.name
    ))
}

/// **The kernel's `Withdraw` for a raider's hull**, as a roster's order reaches it (the DT.1b
/// adapter's fence, then the command; `super::orders`), the hull its own issuer: the leader
/// commands from no unit in the field, and the kernel takes an order only from a unit of the
/// recipient's own party. One fence and one sequence for the raider's whole life, so offering
/// it again is a duplicate the kernel refuses harmlessly. `false` when the field would not take
/// it now.
pub(crate) fn order_home(ctx: &mut Ctx<'_>, hull: ObjId) -> bool {
    use crate::arena::detachment::{
        DetachmentAuthority, DetachmentCommand, DetachmentControl, DetachmentObjective,
    };
    let Some(leader) = leader_of(ctx.w, hull) else {
        return false;
    };
    let Some(site) = super::field_of_unit(ctx.w, hull) else {
        return false;
    };
    let Some((task, party)) = ctx
        .w
        .module_state::<super::FieldBook>()
        .and_then(|book| book.hosted(&site))
        .and_then(|hosted| Some((hosted.task, hosted.live.party_of(hull)?)))
    else {
        return false;
    };
    let authority = DetachmentAuthority {
        detachment: RAIDER_DETACHMENT | u64::from(leader.0),
        revision: 1,
        term: 1,
    };
    let control = DetachmentControl {
        unit: hull,
        epoch: 1,
        authority: Some(authority),
    };
    let command = DetachmentCommand {
        authority,
        authority_epoch: 1,
        sequence: 1,
        issuer: leader,
        issuer_unit: hull,
        squad: crate::deliberation::SquadId {
            team: party,
            index: 1,
        },
        // A raider leaving runs for its ramp: its WITHDRAW holds fire, so a hull that is fired on
        // on the way does not stop to stand.
        directive: Some(DetachmentObjective {
            objective: crate::deliberation::order::Objective::Withdraw,
            constraints: crate::deliberation::order::Constraints {
                hold_fire: true,
                ..crate::deliberation::order::Constraints::default()
            },
        }),
        expires_at_ms: None,
    };
    crate::arena_bridge::offer_field_detachment(ctx, task, control, command)
}

/// **A raider's hull is out of its field at the ramp** (`super::exit`'s continuation): aboard
/// its carrier again, its WITHDRAW spent.
///
/// # Errors
///
/// The hull could not be taken off the ground.
pub(crate) fn board(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<(), String> {
    set_mark(ctx, hull, "withdraw", None)?;
    mush_sim::unplace(ctx, hull, "tacsim: aboard its raider carrier")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_authored_raiders_parse_and_check() {
        let defs = raiders().expect("the shipped raiders check");
        assert!(!defs.is_empty());
        assert!(defs.iter().all(|def| !def.units.is_empty()));
    }
}
