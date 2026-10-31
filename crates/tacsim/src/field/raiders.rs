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
@field/raiders <id> seeds one on its named site: its leader and its armed hulls, placed on the \
site's ground. Refused while any hull of that raider still stands. Staff only.";

/// One authored hull.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaiderUnit {
    /// The unit design (`tacsim.unit.line_tank`).
    pub unit: CanonicalId,
    /// The hull's name.
    pub name: String,
    /// Its site cell.
    pub cell: [i64; 2],
    /// Its heading, millidegrees.
    pub heading_mdeg: u32,
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

/// **The authored raiders**, checked: schema 1, unique ids, one to 23 hulls each (a party's
/// two-letter designators), and cells on the site grid.
///
/// # Errors
///
/// The first broken rule, as a sentence.
pub fn raiders() -> Result<Vec<RaiderDef>, String> {
    let file: RaidersFile =
        serde_json::from_str(RAIDERS_JSON).map_err(|error| format!("raiders: {error}"))?;
    if file.schema != 1 {
        return Err("raiders: schema 1 only".to_owned());
    }
    let side = i64::from(crate::surface::ground::SITE_CELLS);
    for (n, def) in file.raiders.iter().enumerate() {
        if file.raiders[..n].iter().any(|other| other.id == def.id) {
            return Err(format!("raiders: {} is authored twice", def.id));
        }
        if def.units.is_empty() || def.units.len() > 23 {
            return Err(format!("raiders: {} fields one to 23 hulls", def.id));
        }
        if def.leader.trim().is_empty() || def.label.trim().is_empty() {
            return Err(format!("raiders: {} names its leader and itself", def.id));
        }
        for unit in &def.units {
            if unit.cell.iter().any(|v| !(0..side).contains(v)) || unit.heading_mdeg >= 360_000 {
                return Err(format!(
                    "raiders: {}'s {} stands off its site",
                    def.id, unit.name
                ));
            }
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

/// The raider a hull is seeded for, by id, whatever its leader's state.
fn raider_of(w: &World, unit: ObjId) -> Option<String> {
    w.attr(unit, RAIDER_ATTR)
        .filter(|attr| attr.owner == GOD)?
        .as_json()?
        .get("raider")?
        .as_str()
        .map(str::to_owned)
}

/// The verb's spec.
#[must_use]
pub fn spec() -> CommandSpec {
    CommandSpec {
        name: RAIDERS_VERB.into(),
        switches: vec!["raiders".into()],
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
    let line = seed(ctx, &def).map_err(CmdError::msg)?;
    ctx.out.sys_to_actor(ctx.actor, "tacsim.field", line);
    Ok(())
}

/// **Seed `def` on its site**: its leader, then each hull made, placed and marked.
///
/// # Errors
///
/// Why nothing was seeded (a standing hull of the same raider, an unknown site or design), or a
/// failure part-way, as a sentence.
pub fn seed(ctx: &mut Ctx<'_>, def: &RaiderDef) -> Result<String, String> {
    let standing = ctx
        .w
        .module_state::<mush_tactics::module::UnitStore>()
        .map(|store| {
            store
                .units
                .keys()
                .any(|unit| raider_of(ctx.w, *unit).as_deref() == Some(def.id.as_str()))
        })
        .unwrap_or(false);
    if standing {
        return Err(format!(
            "{} still has a hull standing; a raider is never respawned.",
            def.label
        ));
    }
    let domain = super::parties::site_domain(ctx.w, &def.site)
        .ok_or_else(|| format!("{} is not a lane site.", def.site))?;
    let specs = {
        let store = ctx
            .w
            .module_state::<crate::TacsimStore>()
            .ok_or("The tactical layer is not loaded.")?;
        def.units
            .iter()
            .map(|unit| {
                store
                    .definitions()
                    .bindings
                    .get(&unit.unit)
                    .map(|binding| binding.spec.clone())
                    .ok_or_else(|| format!("No unit design {} is bound.", unit.unit))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let leader = ctx
        .w
        .create(ObjKind::Npc, &def.leader, GOD, None)
        .map_err(|error| format!("The raider's leader was not made ({error})."))?;
    let mark = serde_json::json!({ "raider": def.id.as_str(), "leader": leader.0 });
    for (unit, spec) in def.units.iter().zip(specs) {
        let obj = ctx
            .w
            .create(ObjKind::Thing, &unit.name, GOD, None)
            .map_err(|error| format!("{} was not made ({error}).", unit.name))?;
        mush_tactics::module::create_unit(ctx, obj, &spec)?;
        let pose = crate::surface::excursion::cell_pose(unit.cell, unit.heading_mdeg)?;
        mush_sim::place(ctx, obj, domain, pose)
            .map_err(|error| format!("{} was not placed ({error}).", unit.name))?;
        ctx.w
            .set_attr(obj, RAIDER_ATTR, Attr::json(mark.clone(), GOD))
            .map_err(|error| format!("{} was not marked ({error}).", unit.name))?;
    }
    Ok(format!(
        "{} seeded at {}: {} hulls led by {} (#{}).",
        def.label,
        crate::surface::act::site_name(ctx.w, &def.site),
        def.units.len(),
        def.leader,
        leader.0
    ))
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
