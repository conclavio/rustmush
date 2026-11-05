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
//!
//! **Its carrier lifts** (doc 36 cut GC.0; Q-36-6, the host half): once every hull it launched
//! into a field is aboard again or is a casualty or a wreck, the host takes the carrier out of the
//! field by doc 32 F-6's departure and off the site's ground (`lift_check`, `lift`), so the
//! unarmed carrier no longer keeps the field engaged and nobody has to mark the raider Friendly to
//! end it. `@field/raiders/launch` lands a lifted carrier again at its authored cell.

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
    /// **Its crew member's name** (doc 42b cut SU.1, Q-42b-1): the NPC the seed seats in it;
    /// `None` names them "`<hull>` crew".
    #[serde(default)]
    pub crew: Option<String>,
    /// **Its crew's resolve**, per mille (Q-42b-6, Q-42b-7): `None` takes the raider's.
    #[serde(default)]
    pub resolve_permille: Option<u16>,
}

impl RaiderUnit {
    /// **The name the seed gives this hull's crew member** (Q-42b-1).
    #[must_use]
    pub fn crew_name(&self) -> String {
        self.crew
            .clone()
            .unwrap_or_else(|| format!("{} crew", self.name))
    }
}

/// **The authored carrier** (doc 32 cut CF.6b).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaiderCarrier {
    /// The unit design (`tacsim.unit.dray`).
    pub unit: CanonicalId,
    /// Its name.
    pub name: String,
    /// **Its cell, as an offset from the site's pad** (doc 32 cut S.1, ruling 4): cells east
    /// and north of the pad's cell (`crate::surface::excursion::site_pad_cell`).
    pub cell: [i64; 2],
    /// Its heading, millidegrees.
    pub heading_mdeg: u32,
    /// **Its ramp's cell, as an offset from the site's pad**: where its hulls launch and where
    /// they come home.
    pub ramp: [i64; 2],
}

impl RaiderCarrier {
    /// **The carrier's and its ramp's site cells** on a site whose pad is at `pad` (doc 32 cut
    /// S.1): each offset added to the pad's cell; `None` when one leaves `i64`.
    #[must_use]
    pub fn cells_at(&self, pad: (i64, i64)) -> Option<([i64; 2], [i64; 2])> {
        let at = |offset: [i64; 2]| -> Option<[i64; 2]> {
            Some([pad.0.checked_add(offset[0])?, pad.1.checked_add(offset[1])?])
        };
        Some((at(self.cell)?, at(self.ramp)?))
    }
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
    /// **The organization it flies for** (doc 36 cut GC.11; Q-36-55): registered in
    /// `mush-faction` at the seed with the leader as its principal, so standing written for its
    /// fire and for fire at it names somebody.
    pub organization: String,
    /// The carrier its hulls launch from.
    pub carrier: RaiderCarrier,
    /// Its hulls.
    pub units: Vec<RaiderUnit>,
    /// **The raid director's schedule for it** (doc 36 cut GC.11; Q-36-41), when the director
    /// runs it; `None` keeps it staff-run only.
    #[serde(default)]
    pub director: Option<RaidSchedule>,
    /// **Its crews' resolve**, per mille (doc 42b cut SU.1, Q-42b-6): each hull's crew takes it
    /// unless the hull names its own; `None` reads [`DEFAULT_RESOLVE_PERMILLE`]. The director
    /// never varies it.
    #[serde(default)]
    pub resolve_permille: Option<u16>,
}

impl RaiderDef {
    /// **The resolve the seed writes on `unit`'s crew record** (Q-42b-6, Q-42b-7): the hull's
    /// own, else the raider's, else [`DEFAULT_RESOLVE_PERMILLE`].
    #[must_use]
    pub fn resolve_of(&self, unit: &RaiderUnit) -> u16 {
        unit.resolve_permille
            .or(self.resolve_permille)
            .unwrap_or(DEFAULT_RESOLVE_PERMILLE)
    }
}

/// **The resolve a raider that authors none takes**, per mille (doc 42b Q-42b-6; provisional).
pub const DEFAULT_RESOLVE_PERMILLE: u16 = 500;
/// **The station the seed seats a raider's crew member in** (doc 42b cut SU.1, Q-42b-1): host
/// bookkeeping only, as a DT NPC pilot's seat is; the hull still flies its ROM seal.
pub const CREW_STATION: &str = "cockpit";

/// **When the raid director sends a raider, and with how much** (doc 36 cut GC.11; Q-36-41):
/// one per raider, so one per site. Every number is provisional.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaidSchedule {
    /// **Presence**: the fewest flights out on the site's ground (anyone's units but the raider's,
    /// carriers on their pads not counted) before it raids.
    pub presence_flights: u16,
    /// **Ore value**: the fewest kilograms of ore aboard the rigs on the site's ground before it
    /// raids.
    pub ore_kg: u64,
    /// **The cooldown**: IC seconds from one raid's landing to the next one's at the earliest.
    pub cooldown_ic_s: i64,
    /// **The threat budget**: the most hulls one raid puts on the ground (each hull costs one).
    pub threat: u16,
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

/// **The authored raiders**, checked: schema 5 (doc 32 cut S.1: the carrier's cells are offsets
/// from the pad; doc 36 cut GC.11: an organization each, and the director's schedule; doc 42b cut
/// SU.1: a crew's resolve within 0 to 1,000 and a crew name that is not blank), unique ids
/// and organizations, one to 22 hulls each beside the carrier (a party's two-letter
/// designators), the carrier and its ramp within the widest site's side of the pad, and the ramp
/// beside the carrier, never under it.
///
/// # Errors
///
/// The first broken rule, as a sentence.
pub fn raiders() -> Result<Vec<RaiderDef>, String> {
    parse(RAIDERS_JSON)
}

/// [`raiders`]' checks over any text.
fn parse(json: &str) -> Result<Vec<RaiderDef>, String> {
    let file: RaidersFile =
        serde_json::from_str(json).map_err(|error| format!("raiders: {error}"))?;
    if file.schema != 5 {
        return Err("raiders: schema 5 only".to_owned());
    }
    // Doc 32 cut S.1: an offset within the widest site's side of the pad here; `seed` holds each
    // cell to its own site's side.
    let side = i64::from(crate::surface::ground::MAX_SITE_CELLS);
    for (n, def) in file.raiders.iter().enumerate() {
        if file.raiders[..n].iter().any(|other| other.id == def.id) {
            return Err(format!("raiders: {} is authored twice", def.id));
        }
        if def.units.is_empty() || def.units.len() > 22 {
            return Err(format!("raiders: {} fields one to 22 hulls", def.id));
        }
        if def.leader.trim().is_empty()
            || def.label.trim().is_empty()
            || def.organization.trim().is_empty()
            || def.organization.len() > 40
        {
            return Err(format!(
                "raiders: {} names its leader, itself and its organization",
                def.id
            ));
        }
        if file.raiders[..n]
            .iter()
            .any(|other| other.organization == def.organization)
        {
            return Err(format!(
                "raiders: {}'s organization is another raider's",
                def.id
            ));
        }
        if let Some(schedule) = def.director
            && (schedule.threat == 0 || schedule.cooldown_ic_s < 0)
        {
            return Err(format!(
                "raiders: {}'s director sends at least one hull and waits no negative time",
                def.id
            ));
        }
        let carrier = &def.carrier;
        let apart = (carrier.cell[0] - carrier.ramp[0])
            .abs()
            .max((carrier.cell[1] - carrier.ramp[1]).abs());
        if carrier
            .cell
            .iter()
            .chain(&carrier.ramp)
            .any(|v| !(-side + 1..side).contains(v))
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
        // Doc 42b cut SU.1 (Q-42b-6): a resolve is per mille, and a named crew has a name.
        if def
            .units
            .iter()
            .filter_map(|unit| unit.resolve_permille)
            .chain(def.resolve_permille)
            .any(|resolve| resolve > 1_000)
            || def.units.iter().any(|unit| {
                unit.crew
                    .as_ref()
                    .is_some_and(|crew| crew.trim().is_empty())
            })
        {
            return Err(format!(
                "raiders: {}'s crews name themselves and hold a resolve of 0 to 1,000",
                def.id
            ));
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
pub(crate) fn mark_of(w: &World, unit: ObjId) -> Option<serde_json::Value> {
    w.attr(unit, RAIDER_ATTR)
        .filter(|attr| attr.owner == GOD)?
        .as_json()
        .cloned()
}

/// The raider a hull is seeded for, by id, whatever its leader's state.
pub(crate) fn raider_of(w: &World, unit: ObjId) -> Option<String> {
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
pub(crate) fn hulls_of(w: &World, raider: &str) -> Vec<ObjId> {
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
pub(crate) fn set_mark(
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
    // Doc 32 cut S.1 (ruling 4): the carrier's cells from the site's own pad.
    let off = || {
        format!(
            "{}'s carrier and its ramp stand off {}.",
            def.label, def.site
        )
    };
    let (cell, ramp) = def
        .carrier
        .cells_at(crate::surface::excursion::site_pad_cell(ctx.w, &def.site))
        .ok_or_else(off)?;
    if cell.iter().chain(&ramp).any(|v| !(0..side).contains(v)) {
        return Err(off());
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
    // Doc 36 cut GC.11 (Q-36-55): the leader is its organization's principal. A world with no
    // faction registry seeds the raider all the same.
    if ctx.w.module_state::<mush_faction::FactionState>().is_some() {
        mush_faction::register_content_faction(ctx, &def.organization, leader).map_err(
            |error| format!("{}'s organization was not registered ({error}).", def.label),
        )?;
    }
    let carrier = make(
        ctx,
        &def.carrier.name,
        &carrier_spec,
        serde_json::json!({
            "raider": def.id.as_str(),
            "leader": leader.0,
            "ramp": ramp,
        }),
    )?;
    let pose = crate::surface::excursion::cell_pose(cell, def.carrier.heading_mdeg)?;
    mush_sim::place(ctx, carrier, domain, pose)
        .map_err(|error| format!("{} was not placed ({error}).", def.carrier.name))?;
    for (unit, spec) in def.units.iter().zip(specs) {
        let hull = make(
            ctx,
            &unit.name,
            &spec,
            serde_json::json!({
                "raider": def.id.as_str(),
                "leader": leader.0,
                "carrier": carrier.0,
            }),
        )?;
        crew(ctx, hull, &unit.crew_name(), leader, def.resolve_of(unit))?;
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

/// **A hull's crew member** (doc 42b cut SU.1, Q-42b-1): a fresh NPC inside the hull, seated at
/// [`CREW_STATION`] as a hired NPC is seated (`super::parties::seated` reads it), and given a crew
/// record at once: in the unit, fit, the leader its organizer, the resolve authored
/// (Q-42b-5). A re-seed makes fresh crew; the seat changes nothing about the hull's doctrine.
fn crew(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    name: &str,
    leader: ObjId,
    resolve: u16,
) -> Result<ObjId, String> {
    let who = ctx
        .w
        .create(ObjKind::Npc, name, GOD, Some(hull))
        .map_err(|error| format!("{name} was not made ({error})."))?;
    let station =
        mush_sim::StationId::new(CREW_STATION).ok_or_else(|| "No crew station.".to_owned())?;
    mush_sim::declare_station(
        ctx,
        hull,
        &station,
        "pilot",
        &crate::match_::cockpit::cockpit_channels(),
        None,
        0,
    )
    .map_err(|error| format!("{name}'s station was not declared ({error})."))?;
    mush_sim::commands::assign_station(ctx, hull, &station, who)
        .map_err(|error| format!("{name} was not seated ({error})."))?;
    super::crew::put(
        ctx,
        super::crew::CrewRecord {
            format: super::crew::CREW_FORMAT,
            character: who,
            location: super::crew::CrewLocation::Unit { unit: hull },
            condition: super::crew::CrewCondition::Fit,
            at_ic: ctx.w.world_ic_now().at,
            organizer: Some(leader),
            extraction: None,
            treatment: None,
            resolve_permille: Some(resolve),
        },
    );
    Ok(who)
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
    let record = super::close::site_record(ctx, &def.site)
        .unwrap_or_else(|| crate::surface::site_record::fresh_record(ctx.w, &def.site));
    if record.promoted.is_some() {
        return Err(format!(
            "{} is in a field; a raider launches only onto open ground.",
            crate::surface::act::site_name(ctx.w, &def.site)
        ));
    }
    let (_, launched) = land_and_launch(ctx, def, usize::MAX, &record)?;
    Ok(format!(
        "{} launch from the {}: {} hulls on the ground beside its ramp.",
        def.label,
        def.carrier.name,
        launched.len()
    ))
}

/// **Land `def`'s carrier and put at most `cap` of its hulls aboard on the ground** ([`launch`];
/// doc 36 cut GC.11, the raid director's landing): a carrier that lifted ([`lift`]) lands again at
/// its authored cell first, then the hulls aboard stand beside its ramp, in unit order. `record`
/// is the site's record the ground is read from. Answers the carrier and the hulls put down.
///
/// # Errors
///
/// No carrier, the carrier on other ground, nobody aboard, no free ground beside the ramp.
pub(crate) fn land_and_launch(
    ctx: &mut Ctx<'_>,
    def: &RaiderDef,
    cap: usize,
    record: &crate::surface::site_record::SiteRecord,
) -> Result<(ObjId, Vec<ObjId>), String> {
    let domain = super::parties::site_domain(ctx.w, &def.site)
        .ok_or_else(|| format!("{} is not a lane site.", def.site))?;
    let hulls = hulls_of(ctx.w, def.id.as_str());
    let carrier = hulls
        .iter()
        .copied()
        .find(|hull| ramp_cell(ctx.w, *hull).is_some())
        .ok_or_else(|| format!("{} has no carrier to launch from.", def.label))?;
    let aboard: Vec<ObjId> = hulls
        .iter()
        .copied()
        .filter(|hull| carrier_of(ctx.w, *hull) == Some(carrier))
        .filter(|hull| mush_sim::pose_of(ctx.w, *hull).is_none())
        // Doc 36 cut GC.3 (Q-36-18): an irrecoverable hull is off the ground as remains, not
        // aboard.
        .filter(|hull| !super::close::is_irrecoverable(ctx.w, *hull))
        // Doc 40 (the ramp exit's boarding window): a hull lost with its carrier is a wreck in
        // the bay, and a wreck does not drive down a ramp.
        .filter(|hull| {
            !mush_tactics::module::unit_of(ctx.w, *hull).is_some_and(crate::arena::damage::is_wreck)
        })
        .take(cap)
        .collect();
    if aboard.is_empty() {
        return Err(format!("{} has nobody aboard to launch.", def.label));
    }
    land_carrier(ctx, def, carrier)?;
    let ground = super::ground::definition(ctx.w, &def.site, record, Vec::new())?;
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
    Ok((carrier, aboard))
}

/// **Land `def`'s carrier** ([`land_and_launch`]'s first step): doc 36 cut GC.0, a carrier that
/// lifted ([`lift`]) lands again at its authored cell; one on the site's ground stays where it is.
///
/// # Errors
///
/// Not a lane site, the carrier on other ground, its cell off the site, or the landing refused.
pub(crate) fn land_carrier(
    ctx: &mut Ctx<'_>,
    def: &RaiderDef,
    carrier: ObjId,
) -> Result<(), String> {
    let domain = super::parties::site_domain(ctx.w, &def.site)
        .ok_or_else(|| format!("{} is not a lane site.", def.site))?;
    match mush_sim::pose_of(ctx.w, carrier) {
        Some((on, _)) if on == domain => Ok(()),
        Some(_) => Err(format!("{}'s carrier stands on other ground.", def.label)),
        None => {
            let (cell, _) = def
                .carrier
                .cells_at(crate::surface::excursion::site_pad_cell(ctx.w, &def.site))
                .ok_or_else(|| format!("{}'s carrier stands off {}.", def.label, def.site))?;
            let pose = crate::surface::excursion::cell_pose(cell, def.carrier.heading_mdeg)?;
            mush_sim::place(ctx, carrier, domain, pose)
                .map_err(|error| format!("{} did not land ({error}).", def.carrier.name))
        }
    }
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
    order_withdraw(ctx, &in_field)?;
    Ok(format!(
        "{} withdraw: {} hulls make for the {}'s ramp.",
        def.label,
        in_field.len(),
        def.carrier.name
    ))
}

/// **The leader's WITHDRAW to `hulls`** ([`withdraw`]; doc 36 cut GC.11, the director's
/// withdrawal): each marked, so a restart keeps asking, then handed to the kernel.
///
/// # Errors
///
/// A hull's mark could not be written.
pub(crate) fn order_withdraw(ctx: &mut Ctx<'_>, hulls: &[ObjId]) -> Result<(), String> {
    for hull in hulls {
        set_mark(ctx, *hull, "withdraw", Some(serde_json::Value::Bool(true)))?;
        // The host's pass offers it again until the kernel holds it (`super::home`).
        let _ = order_home(ctx, *hull);
    }
    Ok(())
}

/// **The kernel's `Withdraw` for a raider's hull**, as a roster's order reaches it (the DT.1b
/// adapter's fence, then the command; `super::orders`), the hull its own issuer: the leader
/// commands from no unit in the field, and the kernel takes an order only from a unit of the
/// recipient's own party. One fence and one sequence for the raider's whole life, so offering
/// it again is a duplicate the kernel refuses harmlessly. `false` when the field would not take
/// it now.
pub(crate) fn order_home(ctx: &mut Ctx<'_>, hull: ObjId) -> bool {
    use crate::arena::detachment::DetachmentObjective;
    // A raider leaving runs for its ramp: its WITHDRAW holds fire, so a hull that is fired on
    // on the way does not stop to stand.
    let directive = DetachmentObjective {
        objective: crate::deliberation::order::Objective::Withdraw,
        constraints: crate::deliberation::order::Constraints {
            hold_fire: true,
            ..crate::deliberation::order::Constraints::default()
        },
    };
    offer_order(ctx, hull, WITHDRAW_SEQUENCE, Some(directive))
}

/// The sequence the leader's WITHDRAW stands under ([`order_home`]).
const WITHDRAW_SEQUENCE: u64 = 1;
/// **The sequence that lifts it** ([`strand`]): the one order after the WITHDRAW a raider's hull
/// is ever given, so offering it again is a duplicate too.
const STRAND_SEQUENCE: u64 = 2;

/// **One leader's order to `hull` in its field** ([`order_home`], [`strand`]): `directive`
/// under the raider's fence at `sequence`; `None` cancels what stands, so the doctrine fights
/// on its own. `false` when the field would not take it now.
fn offer_order(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    sequence: u64,
    directive: Option<crate::arena::detachment::DetachmentObjective>,
) -> bool {
    use crate::arena::detachment::{DetachmentAuthority, DetachmentCommand, DetachmentControl};
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
        sequence,
        issuer: leader,
        issuer_unit: hull,
        squad: crate::deliberation::SquadId {
            team: party,
            index: 1,
        },
        directive,
        expires_at_ms: None,
    };
    crate::arena_bridge::offer_field_detachment(ctx, task, control, command)
}

/// **A raider's hull is out of its field at the ramp**: aboard its carrier again, its WITHDRAW
/// spent. Doc 40 (the ramp exit's boarding window; the owner's ruling, 2026-11-04, "back in the
/// field"): run by `super::exit::on_withdrawn` in the batch that checks the hull in, so no
/// carrier can be wrecked between the release and the boarding; the kernel kept the hull had
/// its carrier been lost (`crate::arena::withdraw::WithdrawRefusal::CarrierLost`). Its carrier's
/// lift check follows once the record names the exit (`super::exit`'s continuation).
///
/// # Errors
///
/// The hull could not be taken off the ground.
pub(crate) fn board(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<(), String> {
    set_mark(ctx, hull, "withdraw", None)?;
    mush_sim::unplace(ctx, hull, "tacsim: aboard its raider carrier")?;
    Ok(())
}

/// **A raider's hull whose carrier is lost is stranded in the field** (doc 40, the ramp exit's
/// boarding window; the owner's ruling, 2026-11-04): the kernel kept it at the ramp
/// (`crate::arena::withdraw::WithdrawRefusal::CarrierLost`), so it never left and stays the
/// field's unit, on its row. The shape is the least the host can do: nothing to undo or admit
/// again, since the kernel refused the release on its own live facts at the pulse that would
/// have let it go. The leader's WITHDRAW is lifted in the kernel ([`STRAND_SEQUENCE`], no
/// directive), so its doctrine fights as it would, to the death (the owner's ruling on a raid
/// stranded by its carrier's wreck, 2026-11-04); once the field took that, the hull's mark is
/// spent, so the host stops asking it out. A busy field leaves the mark, and the next refusal
/// strands it again.
pub(crate) fn strand(ctx: &mut Ctx<'_>, hull: ObjId) {
    if !withdrawing(ctx.w, hull) {
        return;
    }
    if !offer_order(ctx, hull, STRAND_SEQUENCE, None) {
        return;
    }
    if let Err(error) = set_mark(ctx, hull, "withdraw", None) {
        tracing::warn!(hull = hull.0, %error, "tacsim: a stranded raider's WITHDRAW mark could not be spent");
        return;
    }
    tracing::info!(
        hull = hull.0,
        "tacsim: a raider's carrier is lost; its hull is stranded in the field"
    );
}

/// **Every raider's hull aboard `carrier` is lost with it** (doc 40, the ramp exit's boarding
/// window; the owner's ruling, 2026-11-04): when the fixture carrier checks in from a field a
/// wreck, each hull of its raider aboard it (off the ground, launched from it, in no field
/// among `units`, not remains) becomes a wreck on its own record — the arena's wreck mark,
/// [`crate::carrier_damage::wreck_plan`], as a programme carrier's bay rigs are
/// (`super::close`'s `wreck_bay_rigs`). It stays aboard: the bay is in the wreck, and nothing
/// deletes it. A wreck aboard never launches again ([`land_and_launch`]).
pub(crate) fn wreck_aboard(ctx: &mut Ctx<'_>, carrier: ObjId, units: &[ObjId]) {
    let Some(raider) = raider_of(ctx.w, carrier) else {
        return;
    };
    if ramp_cell(ctx.w, carrier).is_none()
        || !mush_tactics::module::unit_of(ctx.w, carrier)
            .is_some_and(crate::arena::damage::is_wreck)
    {
        return;
    }
    let aboard: Vec<ObjId> = hulls_of(ctx.w, &raider)
        .into_iter()
        .filter(|hull| carrier_of(ctx.w, *hull) == Some(carrier))
        .filter(|hull| !units.contains(hull))
        .filter(|hull| mush_sim::pose_of(ctx.w, *hull).is_none())
        .filter(|hull| !super::close::is_irrecoverable(ctx.w, *hull))
        .collect();
    for hull in aboard {
        let Some(plan) = mush_tactics::module::unit_of(ctx.w, hull).and_then(|cells| {
            crate::carrier_damage::wreck_plan(crate::carrier_damage::spec_of(ctx.w, cells), cells)
        }) else {
            continue;
        };
        match mush_tactics::module::apply_plan(ctx, hull, &plan) {
            Ok(_) => tracing::info!(
                carrier = carrier.0,
                hull = hull.0,
                "tacsim: a raider's hull aboard its wrecked carrier is a wreck"
            ),
            Err(error) => {
                tracing::warn!(carrier = carrier.0, hull = hull.0, %error, "tacsim: a raider's hull aboard its wrecked carrier could not be wrecked");
            }
        }
    }
}

/// **The dry mass of the raider's hulls aboard `carrier`, kilograms** (doc 40, the ramp exit's
/// boarding window; Q-41-29's laden count, `crate::surface::tow::carried_kg`): a fixture carrier
/// has no programme bay, so what it carries is its raider's hulls off the ground that launched
/// from it, wrecks included. Nothing for a unit that is no fixture carrier.
pub(crate) fn aboard_kg(w: &World, carrier: ObjId) -> u64 {
    let Some(raider) = raider_of(w, carrier).filter(|_| ramp_cell(w, carrier).is_some()) else {
        return 0;
    };
    hulls_of(w, &raider)
        .into_iter()
        .filter(|hull| carrier_of(w, *hull) == Some(carrier))
        .filter(|hull| mush_sim::pose_of(w, *hull).is_none())
        .filter(|hull| !super::close::is_irrecoverable(w, *hull))
        .filter_map(|hull| crate::carrier_damage::mass_kg(w, hull))
        .fold(0_u64, u64::saturating_add)
}

/// **How often the snapshots' lift check runs**, in arena milliseconds (doc 36 cut GC.0; a
/// provisional default). The check decodes the whole snapshot, about 300 ms of the world task at
/// 512 a side, and snapshots come every `[tacsim.arena].snapshot_ms` (500 ms in the harness): on
/// every one, the decodes held the world task most of the fight's time, and a slower runner's
/// world fell behind its field. A lift waits at most this long past its last hull going down; a
/// board checks at once.
pub const LIFT_CHECK_MS: i64 = 10_000;

/// **The snapshots' lift check, paced** (doc 36 cut GC.0): [`lift_check`] at the field's first
/// snapshot, then once each [`LIFT_CHECK_MS`] of arena time. A snapshot earlier than the last
/// check (a new field on the site, or a resume that rewound) checks again.
pub(crate) fn lift_check_paced(ctx: &mut Ctx<'_>, site: &CanonicalId, at_ms: i64) {
    let Some(book) = ctx.w.module_state_mut::<super::FieldBook>() else {
        return;
    };
    if !lift_due(book.lift_checked.get(site).copied(), at_ms) {
        return;
    }
    book.lift_checked.insert(site.clone(), at_ms);
    lift_check(ctx, site);
}

/// Whether a snapshot at `at_ms` is owed the lift check, the last having run at `last`.
fn lift_due(last: Option<i64>, at_ms: i64) -> bool {
    last.is_none_or(|last| at_ms < last || at_ms - last >= LIFT_CHECK_MS)
}

/// The line the site hears when a raider's carrier lifts (doc 36 cut GC.0).
pub const RAIDER_LIFTED: &str =
    "The {name} lifts from {site} with its hulls aboard, leaving what it lost where it fell.";

/// **A raider's carrier lifts once its fight is over** (doc 36 cut GC.0; Q-36-6, the host half):
/// once every hull it launched into the field on `site` is aboard again or is a casualty or a
/// wreck, the host asks the field to let the carrier go with no ramp (doc 32 F-6's departure,
/// `super::exit` with [`super::exit::Then::RaiderLift`]), as its leader. Out, it leaves the
/// site's ground ([`lift`]), and its party has nobody standing in the field to keep it engaged.
///
/// "A casualty or a wreck" is what demotion would write for the hull now
/// (`super::close::condition_now`), read off the last snapshot the host holds, so a hull still
/// fighting holds its carrier on the ground. A carrier that launched no hull into this field (none
/// admitted, none named on the open promotion's exits) has nothing to wait for and stays; a
/// director raid's (doc 36 cut GC.11), whose hulls the kernel refused at the join, waits for the
/// director to land them again and lifts once a withdrawal trigger holds. A carrier that is itself a wreck does not lift. The host runs this after each hull boards and,
/// paced, at the field's snapshots ([`lift_check_paced`]); a field holding no raider's carrier
/// decodes nothing.
pub(crate) fn lift_check(ctx: &mut Ctx<'_>, site: &CanonicalId) {
    let Some(live) = ctx
        .w
        .module_state::<super::FieldBook>()
        .and_then(|book| book.hosted(site))
        .map(|hosted| hosted.live.clone())
    else {
        return;
    };
    let carriers: Vec<ObjId> = live
        .units
        .iter()
        .map(|row| row.unit)
        .filter(|unit| ramp_cell(ctx.w, *unit).is_some())
        .filter(|unit| {
            !ctx.w
                .module_state::<super::exit::ExitBook>()
                .is_some_and(|book| book.0.contains_key(unit))
        })
        .collect();
    if carriers.is_empty() {
        return;
    }
    let record = super::close::site_record(ctx, site);
    // The last snapshot, decoded once for every carrier: its returns, and the arena itself for
    // the director's idle trigger (doc 36 cut GC.11).
    let mut decoded: Option<Option<(crate::arena::Arena, Vec<crate::arena::damage::UnitReturn>)>> =
        None;
    for carrier in carriers {
        let Some(raider) = raider_of(ctx.w, carrier) else {
            continue;
        };
        // Doc 42 cut CR.4 (Q-42-12, Draft 0.3 42-5): a pending eject capture naming this carrier
        // holds it on the ground until the captive is aboard (the extraction's due instant).
        if super::crew::capture_pending(ctx.w, carrier) {
            continue;
        }
        let launched: Vec<ObjId> = hulls_of(ctx.w, &raider)
            .into_iter()
            .filter(|hull| carrier_of(ctx.w, *hull) == Some(carrier))
            .collect();
        let in_field: Vec<ObjId> = launched
            .iter()
            .copied()
            .filter(|hull| live.units.iter().any(|row| row.unit == *hull))
            .collect();
        let exited = launched
            .iter()
            .filter(|hull| record.as_ref().is_some_and(|record| record.exited(**hull)))
            .count();
        // Doc 36 cut GC.11: a director raid's carrier whose hulls were all refused at the join
        // (each back aboard) waits while the director lands them again, and lifts once its
        // triggers say so.
        let refused = in_field.is_empty() && exited == 0;
        if refused && !crate::surface::raid::director_raid(ctx.w, carrier) {
            continue;
        }
        let decoded =
            decoded.get_or_insert_with(|| decode(ctx, site, live.homes.clone(), &live.held));
        let condition = |unit: ObjId| {
            decoded
                .as_ref()
                .and_then(|(_, returns)| returns.iter().find(|back| back.unit == unit))
                .and_then(|back| super::close::condition_now(ctx, back))
        };
        let lost: Vec<ObjId> = in_field
            .iter()
            .copied()
            .filter(|hull| condition(*hull).is_some())
            .collect();
        let down = lost.len() == in_field.len();
        let wrecked = matches!(
            condition(carrier),
            Some(crate::surface::site_record::LeftCondition::Wreck)
        );
        if wrecked {
            continue;
        }
        if refused {
            let reading = crate::surface::raid::RaidReading {
                ordinal: live.field.ordinal,
                at_ms: live.at_ms,
                launched: 0,
                lost: 0,
                in_reach: false,
            };
            if crate::surface::raid::withdrawal_due(ctx, carrier, &reading).is_none() {
                continue;
            }
        }
        if !down {
            // Doc 36 cut GC.11 (Q-36-55): a director raid turns for home on its triggers.
            let in_reach = decoded.as_ref().is_some_and(|(arena, _)| {
                live.party_of(carrier)
                    .is_some_and(|party| arena.party_holds_armed_hostile_in_reach(party))
            });
            let reading = crate::surface::raid::RaidReading {
                ordinal: live.field.ordinal,
                at_ms: live.at_ms,
                launched: in_field.len().saturating_add(exited),
                lost: lost.len(),
                in_reach,
            };
            if let Some(why) = crate::surface::raid::withdrawal_due(ctx, carrier, &reading) {
                let going: Vec<ObjId> = in_field
                    .iter()
                    .copied()
                    .filter(|hull| !lost.contains(hull) && !withdrawing(ctx.w, *hull))
                    .collect();
                if !going.is_empty() {
                    tracing::info!(site = %site, carrier = carrier.0, reason = why, hulls = going.len(), "tacsim: a director raid turns for home");
                    let _ = order_withdraw(ctx, &going);
                }
            }
            continue;
        }
        // Doc 36 cut GC.3 (Q-36-8): a barge whose drive is destroyed stays on the ground.
        let grounded = decoded
            .as_ref()
            .and_then(|(_, returns)| returns.iter().find(|back| back.unit == carrier))
            .and_then(|back| {
                crate::carrier_damage::spec_of(ctx.w, &back.cells)
                    .map(|spec| crate::carrier_damage::CarrierDamage::read(spec, &back.cells))
            })
            .is_some_and(|damage| damage.has(crate::carrier_damage::CarrierLoss::RefusesLift));
        if grounded {
            continue;
        }
        let Some(leader) = leader_of(ctx.w, carrier) else {
            continue;
        };
        let saved = ctx.actor;
        ctx.actor = leader;
        // A refusal (the field busy) is asked again at the next snapshot.
        let asked = super::exit::request(ctx, carrier, None, super::exit::Then::RaiderLift);
        ctx.actor = saved;
        if asked.is_ok() {
            tracing::info!(site = %site, carrier = carrier.0, "tacsim: a raider's carrier lifts out of an IC field");
        }
    }
}

/// **The last snapshot of `site`'s field, decoded** on its own ground (its `homes` and its `held`
/// walls from the live record; doc 36a §11 #1), with its units' returns, when it decodes:
/// `close::snapshot_returns`' read, keeping the arena.
fn decode(
    ctx: &Ctx<'_>,
    site: &CanonicalId,
    homes: Vec<crate::arena::state::EntryZone>,
    held: &[super::record::HeldWall],
) -> Option<(crate::arena::Arena, Vec<crate::arena::damage::UnitReturn>)> {
    let document = ctx
        .w
        .module_state::<super::FieldBook>()
        .and_then(|book| book.snapshot(site).cloned())?;
    let record = super::close::site_record(ctx, site)?;
    let ground = super::admit::ground_with_held(ctx.w, site, &record, homes, held).ok()?;
    let limit = ctx.w.module_state::<crate::store::TacsimStore>().map_or(
        crate::arena::snapshot_capacity::SNAPSHOT_MAX_BYTES,
        |store| store.config().arena.snapshot_max_bytes,
    );
    let arena = crate::arena::Snapshot::decode_with_limit(&document, ground, limit).ok()?;
    let returns = arena.returned_units();
    Some((arena, returns))
}

/// **A raider's carrier is out of its field** (`super::exit`'s continuation for
/// [`super::exit::Then::RaiderLift`]): it leaves the site's ground with its hulls aboard. A later
/// `@field/raiders/launch` lands it again at its authored cell.
///
/// # Errors
///
/// The carrier could not be taken off the ground.
pub(crate) fn lift(ctx: &mut Ctx<'_>, carrier: ObjId, site: &CanonicalId) -> Result<(), String> {
    // Doc 36a lane 3 (P1-11; doc 36 cut GC.3, Q-36-8): a barge whose drive is down stays on the
    // ground, whoever asks for the lift.
    if let Some(line) = crate::carrier_damage::lift_refusal(ctx.w, carrier) {
        return Err(line);
    }
    mush_sim::unplace(ctx, carrier, "tacsim: a raider's carrier lifts")?;
    // A carrier off the ground is no casualty on the site's index.
    crate::surface::site_record::take_off_index(ctx, site, carrier);
    let line = RAIDER_LIFTED
        .replace("{name}", &crate::port::coop::name_of(ctx.w, carrier))
        .replace("{site}", &crate::surface::act::site_name(ctx.w, site));
    super::tell_site(ctx, site, &line);
    Ok(())
}

/// **A director raid arrives in a running field** (doc 36 cut GC.11; Q-36-41, the kernel's join
/// of GC.1b): its carrier and the hulls just put down beside its ramp are each checked out under
/// the field's mark, given a row on the live record in the raider's party (lettered next when
/// the field has none), and sent to the kernel as a join at the pose they stand at. The party's
/// home is its carrier's ramp, laid on the field's ground first
/// ([`crate::arena_task::ArenaMsg::Home`]), so the group can go home up it. The party holds its
/// leader's standing Hostile mark on every other party, as at an admission
/// (`super::carrier::joining`). A hull the field will not take now goes back aboard.
///
/// # Errors
///
/// No field runs on the site, or the field would not take the carrier.
pub(crate) fn join_raid(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    carrier: ObjId,
    hulls: &[ObjId],
) -> Result<(), String> {
    let (task, mut live) = ctx
        .w
        .module_state::<super::FieldBook>()
        .and_then(|book| book.hosted(site))
        .map(|hosted| (hosted.task, hosted.live.clone()))
        .ok_or_else(|| "no field runs there".to_owned())?;
    let tasks = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .ok_or_else(|| "the field's task is gone".to_owned())?;
    let mark = mush_sim::deployed::DeployedMark::field(site.as_str(), live.field.ordinal);
    let mut joined = 0_usize;
    // A carrier already in the field (its hulls refused and landed again) is not joined twice.
    let carrier_in = live.units.iter().any(|row| row.unit == carrier);
    let joining = std::iter::once(carrier)
        .filter(|_| !carrier_in)
        .chain(hulls.iter().copied());
    for unit in joining {
        match join_one(ctx, site, carrier, unit, &mut live, &tasks, task, &mark) {
            Ok(()) => joined += 1,
            Err(why) => {
                tracing::warn!(site = %site, hull = unit.0, reason = %why, "tacsim: a raider could not be taken into the field; it goes back aboard");
                if unit != carrier {
                    let _ = mush_sim::unplace(ctx, unit, "tacsim: aboard its raider carrier");
                    continue;
                }
                // Nothing it carried can stand outside the field either.
                for hull in hulls {
                    let _ = mush_sim::unplace(ctx, *hull, "tacsim: aboard its raider carrier");
                }
                let _ = mush_sim::unplace(ctx, carrier, "tacsim: a raider's carrier lifts");
                return Err(why);
            }
        }
    }
    if let Some(hosted) = ctx
        .w
        .module_state_mut::<super::FieldBook>()
        .and_then(|book| book.live.get_mut(site))
    {
        hosted.live = live.clone();
    }
    super::record::write_live(ctx, &live);
    tracing::info!(site = %site, carrier = carrier.0, joined, task = task.0, "tacsim: a raid is arriving in the field");
    Ok(())
}

/// One raider's join ([`join_raid`]): its home laid when its party has none, its check-out, its
/// Join, and its row on `live`.
#[allow(clippy::too_many_arguments)] // one join's whole context, as `join_raid` holds it
fn join_one(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    carrier: ObjId,
    unit: ObjId,
    live: &mut super::FieldLive,
    tasks: &crate::arena_bridge::ArenaTasks,
    task: crate::arena_task::MatchId,
    mark: &mush_sim::deployed::DeployedMark,
) -> Result<(), String> {
    let (_, pose) = mush_sim::pose_of(ctx.w, unit)
        .ok_or_else(|| format!("#{} is not on the ground", unit.0))?;
    let mut joining = super::carrier::joining(ctx, site, live, unit, pose)?;
    let party = joining.live.party_of(unit).ok_or("the hull has no party")?;
    // The party's home: its carrier's ramp, laid once.
    if !joining.live.homes.iter().any(|home| home.team == party)
        && let Some([x, y]) = ramp_cell(ctx.w, carrier)
        && let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y))
    {
        let zone = crate::arena::state::EntryZone {
            team: party,
            x0: x,
            y0: y,
            x1: x,
            y1: y,
        };
        tasks
            .offer(task, crate::arena_task::ArenaMsg::Home { zone })
            .map_err(|_| "the field is busy".to_owned())?;
        joining.live.homes.push(zone);
    }
    // Doc 36a lane 3 (P0-8): the site's record must have a row for everyone on the ground.
    if let Some(domain) = super::parties::site_domain(ctx.w, site) {
        super::admit::index_room(ctx.w, domain)?;
    }
    mush_tactics::custody::check_out(ctx, unit, mark.clone())
        .map_err(|error| format!("#{} could not be taken into the field ({error})", unit.0))?;
    let controller = joining.unit.controller;
    let armed = !joining.unit.weapons.is_empty();
    let message = crate::arena_task::ArenaMsg::Join {
        unit: Box::new(joining.unit),
        pilot: joining.pilot,
        host: Box::new(joining.host),
        spread: LAUNCH_SPREAD_CELLS,
        // A raider's hull flies no claimed mind.
        source: None,
        carried: None,
    };
    if tasks.offer(task, message).is_err() {
        let _ = mush_tactics::custody::check_in(ctx, unit, None);
        return Err("the field is busy".to_owned());
    }
    *live = joining.live;
    if let Some(registry) = ctx
        .w
        .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
    {
        registry.bind_joining(unit, task, controller, armed);
    }
    Ok(())
}

/// **Whether `unit` is a raider's hull or carrier** (the bridge's branch for a join's answer).
#[must_use]
pub fn is_raider(w: &World, unit: ObjId) -> bool {
    mark_of(w, unit).is_some()
}

/// **The kernel's answer to a raider's join** (doc 36 cut GC.11; the bridge's branch): taken in,
/// it is logged (the site heard the landing); refused, it is checked back in as it went out, its
/// row leaves the live record, and it goes back aboard (a carrier lifts).
pub(crate) fn on_joined(
    ctx: &mut Ctx<'_>,
    task: crate::arena_task::MatchId,
    unit: ObjId,
    outcome: Result<crate::arena::state::Designator, String>,
) {
    let Some((site, live)) = ctx.w.module_state::<super::FieldBook>().and_then(|book| {
        let site = book.site_of_task(task)?.clone();
        let live = book.hosted(&site)?.live.clone();
        Some((site, live))
    }) else {
        return;
    };
    match outcome {
        Ok(name) => {
            tracing::info!(site = %site, hull = unit.0, name = ?name, "tacsim: a raider joined the field");
        }
        Err(why) => {
            tracing::warn!(site = %site, hull = unit.0, reason = %why, "tacsim: the field refused a raider; it goes back aboard");
            let _ = mush_tactics::custody::check_in(ctx, unit, None);
            let mut live = live;
            live.units.retain(|row| row.unit != unit);
            if let Some(hosted) = ctx
                .w
                .module_state_mut::<super::FieldBook>()
                .and_then(|book| book.live.get_mut(&site))
            {
                hosted.live = live.clone();
            }
            super::record::write_live(ctx, &live);
            if let Some(registry) = ctx
                .w
                .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
            {
                registry.unbind_joining(unit, task);
            }
            let _ = mush_sim::unplace(ctx, unit, "tacsim: aboard its raider carrier");
        }
    }
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

    /// **Doc 42b cut SU.1** (Q-42b-6, Q-42b-7): schema 5's shipped resolves, the fallbacks (the
    /// hull's, else the raider's, else 500), the crew's default name, and the refusals.
    #[test]
    fn su1_schema_5_authors_each_crews_resolve() {
        let defs = raiders().expect("the shipped raiders check");
        let resolve = |id: &str| {
            let def = defs.iter().find(|def| def.id.as_str() == id).expect(id);
            def.units
                .iter()
                .map(|unit| def.resolve_of(unit))
                .collect::<Vec<_>>()
        };
        assert_eq!(resolve("tacsim.raider.ahuna_skulks"), [400, 400]);
        assert_eq!(resolve("tacsim.raider.urvara_reavers"), [750, 750]);
        let skulk = &defs[0];
        assert_eq!(skulk.units[0].crew_name(), "Skulk One crew");
        let mut file: serde_json::Value = serde_json::from_str(RAIDERS_JSON).expect("json");
        let def = &mut file["raiders"][0];
        def["units"][0]["resolve_permille"] = serde_json::json!(0);
        def["units"][0]["crew"] = serde_json::json!("Ivo Brandt");
        def.as_object_mut()
            .expect("a raider")
            .remove("resolve_permille");
        let parsed = parse(&file.to_string()).expect("a hull's own resolve and crew");
        let def = &parsed[0];
        assert_eq!(def.resolve_of(&def.units[0]), 0);
        assert_eq!(def.resolve_of(&def.units[1]), DEFAULT_RESOLVE_PERMILLE);
        assert_eq!(def.units[0].crew_name(), "Ivo Brandt");
        let refused = |path: &dyn Fn(&mut serde_json::Value)| {
            let mut bad: serde_json::Value = serde_json::from_str(RAIDERS_JSON).expect("json");
            path(&mut bad);
            parse(&bad.to_string()).is_err()
        };
        assert!(refused(
            &|f| f["raiders"][0]["resolve_permille"] = serde_json::json!(1_001)
        ));
        assert!(refused(
            &|f| f["raiders"][0]["units"][1]["resolve_permille"] = serde_json::json!(1_001)
        ));
        assert!(refused(
            &|f| f["raiders"][0]["units"][0]["crew"] = serde_json::json!(" ")
        ));
        assert!(refused(&|f| f["schema"] = serde_json::json!(4)));
    }

    /// Doc 36 cut GC.0: the snapshots' lift check runs at the first, then once a pace, and again
    /// at a snapshot earlier than the last check.
    #[test]
    fn gc0_the_snapshots_lift_check_is_paced() {
        assert!(lift_due(None, 0));
        assert!(!lift_due(Some(0), 500));
        assert!(!lift_due(Some(0), LIFT_CHECK_MS - 1));
        assert!(lift_due(Some(0), LIFT_CHECK_MS));
        assert!(lift_due(Some(20_000), 5_000));
    }
}
