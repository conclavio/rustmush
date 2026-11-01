//! **Live MUSH spaceflight, combat, zero-G mining, and 6-DOF docking commands (`SF.9`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §5–§8, `D-35-1`..`D-35-16`).
//!
//! Registers the [`SpaceBook`] module state slot, the three 256-colour space panel feeds
//! (`space.holotank`, `space.cockpit`, `space.docking`) with pull [`QueryHandler`]s and push
//! keyframe helpers, and the three player verbs:
//! - **`+helm`** (`+helm/enter`, `+helm/burn`, `+helm/roll`, `+helm/match`, `+helm/decouple`,
//!   `+helm/droptanks`, `+helm/expedition`, `+helm/autopilot`, `+helm/dock`, `+helm/tow`)
//! - **`+tac`** (`+tac/pdc`, `+tac/laser`, `+tac/torp`, `+tac/rail`, `+tac/beam`, `+tac/step`)
//! - **`+mine`** (`+mine/ping`, `+mine/licence`, `+mine/stake`, `+mine/contest`, `+mine/laser`,
//!   `+mine/drones`)

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, CanonicalId, CmdError, CommandCompatibility, CommandSpec, Ctx, ObjId, QueryHandler,
    World,
};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::commands::NOT_ENABLED;
use crate::sky::Sky;
use crate::store::TacsimStore;

use super::bubble::{LocalSpaceBubble, ShipSectionId, SpacePartyId, SpaceRelation, SpaceShipState};
use super::docking::{
    DockingControlInput, DockingOutcome, DockingPose6Dof, DockingSession, DockingSessionState,
    SalvageTowRecord,
};
use super::flight::SpaceFlightRecord;
use super::mining::{AsteroidClusterField, OreHoldState};
use super::picket::{HAULER_SHIP_ID, HaulerManifest, KirkwoodPicketEncounter, PicketPhase};
use super::weapons::{LaserMode, LaserMount};

/// The helm primary verb.
pub const HELM_VERB: &str = "+helm";
/// The tactical primary verb.
pub const TAC_VERB: &str = "+tac";
/// The zero-G orbital asteroid mining primary verb.
pub const MINE_VERB: &str = "+mine";

/// Panel feed IDs matching `mush-pixel`.
pub const SPACE_HOLOTANK_FEED: &str = "space.holotank";
/// First-person bridge canopy feed ID.
pub const SPACE_COCKPIT_FEED: &str = "space.cockpit";
/// Out-of-time-loop 6-DOF bow docking camera feed ID.
pub const SPACE_DOCKING_FEED: &str = "space.docking";

/// Switches for `+helm`.
pub const HELM_SWITCHES: [&str; 10] = [
    "enter",
    "burn",
    "roll",
    "match",
    "decouple",
    "droptanks",
    "expedition",
    "autopilot",
    "dock",
    "tow",
];

/// Switches for `+tac`.
pub const TAC_SWITCHES: [&str; 6] = ["pdc", "laser", "torp", "rail", "beam", "step"];

/// Switches for `+mine`.
pub const MINE_SWITCHES: [&str; 6] = ["ping", "licence", "stake", "contest", "laser", "drones"];

/// `+helm` help text.
pub const HELM_HELP: &str = "\
+helm                                    current spaceflight mode, 3D pose, velocity, attitude & delta-v reserve.\n\
+helm/enter <site_or_picket>             promote into a site bubble (kirkwood_picket, pallas_inclined_belt, ceres_halo, kirkwood_gap_3_1, saturn_b_ring_edge).\n\
+helm/burn <ax> <ay> <az> [force]        command a 3D main-drive burn (m/s^2 in bubble or m/s dv in free flight; force overrides RETURN_RESERVE).\n\
+helm/roll <yaw> <pitch> <roll>          rotate hull attitude (degrees) to present fresh armour facets.\n\
+helm/match <rock_id>                    close within 2,000 m and match 3D velocity (<= 5 m/s) with an asteroid pocket.\n\
+helm/decouple [<from_body> <to_body>]   decouple from a filed passage leg into Tier-1 3D System Free Flight.\n\
+helm/droptanks                          fit external long-range expedition drop tanks (+22,000 m/s delta-v).\n\
+helm/expedition <body_id>               burn to an outer-system body (saturn, uranus, neptune, triton) and project the bridge canopy.\n\
+helm/autopilot <port_body>              solve a from-point two-burn flip-and-burn passage back onto the port network.\n\
+helm/dock <port|derelict> <target_id>   enter or step an out-of-time-loop 6-DOF docking session (or +helm/dock trim|pulse|coast|abort).\n\
+helm/tow                                rig a salvage tow line (doc 27a 2x tonnage cap, 48 h contest) to a clamped derelict.";

/// `+tac` help text.
pub const TAC_HELP: &str = "\
+tac                                     tactical status: hull facets, heat, PDC posture, ordnance & bubble contacts.\n\
+tac/pdc <auto|off>                      toggle automatic rotary point-defense interception (Q-35-4).\n\
+tac/laser <offense|defense>             set laser turrets to offensive beam fire or defensive anti-torpedo screen.\n\
+tac/torp <target_id> [<salvo>]          launch a high-G guided torpedo salvo inheriting launcher velocity.\n\
+tac/rail <target_id>                    fire an unguided high-velocity tungsten railgun slug along the target lead vector.\n\
+tac/beam <target_id>                    fire offensive lasers with inverse-square diffraction falloff.\n\
+tac/step [<wall_ms>]                    advance the engaged 1:1 wall-time space bubble (default 1000 ms).";

/// `+mine` help text.
pub const MINE_HELP: &str = "\
+mine                                    orbital stow.ore hold status, Freeport Registry licence & cluster rocks.\n\
+mine/ping <rock_id>                     free active radar/spectrometer ping (spectral class & ore richness words).\n\
+mine/licence                            issue or renew a Freeport Registry prospecting licence (2500 cr / 168 IC days).\n\
+mine/stake <rock_id>                    stake a Freeport Registry claim on a pinged rock in the cluster.\n\
+mine/contest <rock_id>                  contest a rival's Freeport claim within the 48 h window using prior ping evidence.\n\
+mine/laser <rock_id> [<kg>]             extract canonical ore into stow.ore with the ship mining laser (emits IR bloom).\n\
+mine/drones <rock_id> [<count>]         launch up to 4 sealed-mind collector drones (ExcursionId) and run a sortie cycle.";

/// Where a ship is currently hosted inside [`SpaceBook`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActiveSpaceContext {
    /// Promoted into the Kirkwood Customs Picket encounter (`Story 1`).
    Picket(String),
    /// Operating inside one of the 4 canonical orbital asteroid clusters (`SF.4`).
    Cluster(String),
    /// Inside a standalone 1:1 wall-time [`LocalSpaceBubble`].
    Bubble(String),
    /// Flying in Tier-1 heliocentric System Free Flight (`SF.6`).
    FreeFlight(String),
}

/// Module-owned runtime state for active space bubbles, asteroid clusters, customs pickets,
/// Tier-1 system free flight records, and 6-DOF docking sessions (`SF.9`).
#[derive(Clone, Debug, Default)]
pub struct SpaceBook {
    /// Standalone 1:1 wall-time combat/encounter bubbles keyed by `bubble_id`.
    pub bubbles: BTreeMap<String, LocalSpaceBubble>,
    /// Orbital asteroid cluster fields (`ceres_halo`, `kirkwood_gap_3_1`, `pallas_inclined_belt`,
    /// `saturn_b_ring_edge`) keyed by `cluster_id`.
    pub clusters: BTreeMap<String, AsteroidClusterField>,
    /// Active Kirkwood Customs Picket encounters keyed by `ship_id`.
    pub pickets: BTreeMap<String, KirkwoodPicketEncounter>,
    /// Active Tier-1 System Free Flight records keyed by `ship_id`.
    pub flights: BTreeMap<String, SpaceFlightRecord>,
    /// Active out-of-time-loop 6-DOF docking sessions keyed by `ship_id`.
    pub dockings: BTreeMap<String, DockingSession>,
    /// Active salvage tow records (`doc 27a`) keyed by towing `ship_id`.
    pub tows: BTreeMap<String, SalvageTowRecord>,
    /// Maps `ship_id` to its active [`ActiveSpaceContext`].
    pub ship_context: BTreeMap<String, ActiveSpaceContext>,
    /// Explicit actor -> `ship_id` binding for actors operating a space hull.
    pub actor_ships: BTreeMap<ObjId, String>,
}

impl SpaceBook {
    /// Resolve the ship ID operated by `actor`, checking [`Self::actor_ships`] first, then
    /// the actor's live charter or owned hull (`helm_of`), and falling back to `ship.<actor.0>`.
    #[must_use]
    pub fn ship_id_for_actor(&self, w: &World, actor: ObjId) -> String {
        if let Some(bound) = self.actor_ships.get(&actor) {
            return bound.clone();
        }
        if let Some(helm) = crate::port::coop::helm_of(w, actor) {
            return format!("hull.{}", helm.hull.0);
        }
        format!("ship.{}", actor.0)
    }

    /// Look up the [`LocalSpaceBubble`] containing `ship_id` (whether standalone, a cluster, or a picket).
    #[must_use]
    pub fn bubble_for_ship<'a>(
        &'a self,
        ship_id: &'a str,
    ) -> Option<(&'a LocalSpaceBubble, &'a str)> {
        match self.ship_context.get(ship_id)? {
            ActiveSpaceContext::Picket(key) => {
                let enc = self.pickets.get(key)?;
                Some((&enc.bubble, HAULER_SHIP_ID))
            }
            ActiveSpaceContext::Cluster(cid) => {
                let cluster = self.clusters.get(cid)?;
                Some((&cluster.bubble, ship_id))
            }
            ActiveSpaceContext::Bubble(bid) => {
                let bubble = self.bubbles.get(bid)?;
                Some((bubble, ship_id))
            }
            ActiveSpaceContext::FreeFlight(_) => None,
        }
    }

    /// Mutable lookup for the [`LocalSpaceBubble`] containing `ship_id`.
    pub fn bubble_for_ship_mut(
        &mut self,
        ship_id: &str,
    ) -> Option<(&mut LocalSpaceBubble, String)> {
        match self.ship_context.get(ship_id)?.clone() {
            ActiveSpaceContext::Picket(key) => {
                let enc = self.pickets.get_mut(&key)?;
                Some((&mut enc.bubble, HAULER_SHIP_ID.to_owned()))
            }
            ActiveSpaceContext::Cluster(cid) => {
                let cluster = self.clusters.get_mut(&cid)?;
                Some((&mut cluster.bubble, ship_id.to_owned()))
            }
            ActiveSpaceContext::Bubble(bid) => {
                let bubble = self.bubbles.get_mut(&bid)?;
                Some((bubble, ship_id.to_owned()))
            }
            ActiveSpaceContext::FreeFlight(_) => None,
        }
    }
}

/// Register the [`SpaceBook`] state slot, the three space panel feeds (`space.holotank`,
/// `space.cockpit`, `space.docking`), and the `+helm`, `+tac`, and `+mine` player verbs.
///
/// # Errors
///
/// [`ModError`] if a feed, query handler, or command name is already registered.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(SpaceBook::default());

    drop(reg.panel_feed(SPACE_HOLOTANK_FEED)?);
    reg.query_handler(SPACE_HOLOTANK_FEED, Arc::new(HolotankQuery))?;

    drop(reg.panel_feed(SPACE_COCKPIT_FEED)?);
    reg.query_handler(SPACE_COCKPIT_FEED, Arc::new(SpaceCockpitQuery))?;

    drop(reg.panel_feed(SPACE_DOCKING_FEED)?);
    reg.query_handler(SPACE_DOCKING_FEED, Arc::new(SpaceDockingQuery))?;

    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: HELM_VERB.into(),
            switches: HELM_SWITCHES.iter().map(|s| (*s).into()).collect(),
            shape: ArgShape::Raw,
            help: HELM_HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(helm_cmd),
    )?;
    reg.command(
        CommandSpec {
            name: TAC_VERB.into(),
            switches: TAC_SWITCHES.iter().map(|s| (*s).into()).collect(),
            shape: ArgShape::Raw,
            help: TAC_HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(tac_cmd),
    )?;
    reg.command(
        CommandSpec {
            name: MINE_VERB.into(),
            switches: MINE_SWITCHES.iter().map(|s| (*s).into()).collect(),
            shape: ArgShape::Raw,
            help: MINE_HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(mine_cmd),
    )?;
    Ok(())
}

struct HolotankQuery;

impl QueryHandler for HolotankQuery {
    fn query(&self, w: &World, actor: ObjId, _params: &Value) -> Result<OutputEvent, RichText> {
        let data = build_holotank_keyframe(w, actor);
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(SPACE_HOLOTANK_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}

struct SpaceCockpitQuery;

impl QueryHandler for SpaceCockpitQuery {
    fn query(&self, w: &World, actor: ObjId, _params: &Value) -> Result<OutputEvent, RichText> {
        let data = build_cockpit_keyframe(w, actor);
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(SPACE_COCKPIT_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}

struct SpaceDockingQuery;

impl QueryHandler for SpaceDockingQuery {
    fn query(&self, w: &World, actor: ObjId, _params: &Value) -> Result<OutputEvent, RichText> {
        let data = build_docking_keyframe(w, actor);
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(SPACE_DOCKING_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}

fn build_holotank_keyframe(w: &World, actor: ObjId) -> Value {
    if let Some(book) = w.module_state::<SpaceBook>() {
        let ship_id = book.ship_id_for_actor(w, actor);
        if let Some((bubble, resolved_id)) = book.bubble_for_ship(&ship_id) {
            return bubble.holotank_feed(resolved_id);
        }
    }
    json!({
        "schema": { "id": SPACE_HOLOTANK_FEED, "version": 1 },
        "title": "HOLO-TANK STANDBY",
        "scale_km": 1000,
        "contacts": [],
    })
}

fn build_cockpit_keyframe(w: &World, actor: ObjId) -> Value {
    if let Some(book) = w.module_state::<SpaceBook>() {
        let ship_id = book.ship_id_for_actor(w, actor);
        if let Some((bubble, resolved_id)) = book.bubble_for_ship(&ship_id) {
            return bubble.cockpit_feed(resolved_id);
        }
        if let Some(flight) = book.flights.get(&ship_id)
            && let Some(sky) = current_sky(w)
        {
            let bodies = default_outer_bodies();
            return flight.canopy_feed(&sky, &bodies);
        }
    }
    json!({
        "schema": { "id": SPACE_COCKPIT_FEED, "version": 1 },
        "callsign": "BRIDGE CANOPY",
        "bodies": [],
        "contacts": [],
    })
}

fn build_docking_keyframe(w: &World, actor: ObjId) -> Value {
    if let Some(book) = w.module_state::<SpaceBook>() {
        let ship_id = book.ship_id_for_actor(w, actor);
        if let Some(session) = book.dockings.get(&ship_id) {
            return session.docking_feed();
        }
    }
    json!({
        "schema": { "id": SPACE_DOCKING_FEED, "version": 1 },
        "target_name": "NO COLLAR TARGET",
        "status": "STANDBY",
        "dx_mm": 0,
        "dy_mm": 0,
        "dz_mm": 2500,
        "pitch_mdeg": 0,
        "yaw_mdeg": 0,
        "roll_mdeg": 0,
        "vz_mmps": 0,
        "rcs_dampeners": true,
    })
}

fn push_actor_space_feeds(ctx: &mut Ctx<'_>, actor: ObjId) {
    let holo = build_holotank_keyframe(ctx.w, actor);
    ctx.out.push(
        mush_core::Audience::Actor(actor),
        OutputEvent::Panel {
            feed: SmolStr::new_static(SPACE_HOLOTANK_FEED),
            kind: PanelFrameKind::Keyframe,
            data: holo,
        },
    );
    let cockpit = build_cockpit_keyframe(ctx.w, actor);
    ctx.out.push(
        mush_core::Audience::Actor(actor),
        OutputEvent::Panel {
            feed: SmolStr::new_static(SPACE_COCKPIT_FEED),
            kind: PanelFrameKind::Keyframe,
            data: cockpit,
        },
    );
    let has_docking = ctx.w.module_state::<SpaceBook>().is_some_and(|b| {
        let ship_id = b.ship_id_for_actor(ctx.w, actor);
        b.dockings.contains_key(&ship_id)
    });
    if has_docking {
        let dock = build_docking_keyframe(ctx.w, actor);
        ctx.out.push(
            mush_core::Audience::Actor(actor),
            OutputEvent::Panel {
                feed: SmolStr::new_static(SPACE_DOCKING_FEED),
                kind: PanelFrameKind::Keyframe,
                data: dock,
            },
        );
    }
}

fn require_enabled(w: &World) -> Result<(), CmdError> {
    if w.module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled)
    {
        Ok(())
    } else {
        Err(CmdError::msg(NOT_ENABLED))
    }
}

fn current_sky(w: &World) -> Option<Sky> {
    if let Some(store) = w.module_state::<TacsimStore>() {
        let (lane, _) = crate::port::readiness::report_ports(store, w);
        if let Some(lane) = lane {
            return Some(Sky::from_pack(lane, &store.config().time.ephemeris));
        }
    }
    fallback_outer_sky()
}

fn fallback_outer_sky() -> Option<Sky> {
    use crate::config::EphemerisConfig;
    use crate::ephemeris::{Body, Ephemeris};
    use crate::kepler::ElementRow;
    use crate::sky::KeplerSky;

    let epoch = 74_152_540_800_i64;
    let ceres = CanonicalId::new("ceres").ok()?;
    let mars = CanonicalId::new("mars").ok()?;
    let jupiter = CanonicalId::new("jupiter").ok()?;
    let saturn = CanonicalId::new("saturn").ok()?;

    let circular = Ephemeris::new(&EphemerisConfig {
        epoch: 0,
        rate: 1_000,
    })
    .with_body(
        ceres.clone(),
        Body {
            orbit_uau: 2_866_568,
            period_mdays: 1_681_861,
            phase_mdeg: 281_654,
            epoch_ic: Some(epoch),
            parent: None,
        },
    )
    .with_body(
        mars,
        Body {
            orbit_uau: 1_445_443,
            period_mdays: 687_034,
            phase_mdeg: 276_240,
            epoch_ic: Some(epoch),
            parent: None,
        },
    )
    .with_body(
        jupiter,
        Body {
            orbit_uau: 5_204_400,
            period_mdays: 4_332_589,
            phase_mdeg: 112_000,
            epoch_ic: Some(epoch),
            parent: None,
        },
    )
    .with_body(
        saturn,
        Body {
            orbit_uau: 9_582_600,
            period_mdays: 10_759_220,
            phase_mdeg: 45_000,
            epoch_ic: Some(epoch),
            parent: None,
        },
    );

    let ceres_row = ElementRow::from_authored(&[
        74_152_540_800.0,
        2.767_755_414,
        0.075_862_114,
        10.550_296,
        75.280_015,
        84.918_514,
        114.238_750,
        0.214_048_659,
    ])
    .ok()?;
    let mut tables = BTreeMap::new();
    tables.insert(ceres, vec![ceres_row]);
    Some(Sky::Kepler(KeplerSky::new(circular, tables)))
}

fn active_switch<'a>(ctx: &Ctx<'_>, known: &[&'a str]) -> Option<&'a str> {
    known.iter().copied().find(|s| ctx.switches.has(s))
}

fn default_outer_bodies() -> Vec<CanonicalId> {
    [
        "ceres", "mars", "jupiter", "saturn", "uranus", "neptune", "triton",
    ]
    .iter()
    .filter_map(|id| CanonicalId::new(*id).ok())
    .collect()
}

// ---------------------------------------------------------------------------
// `+helm` command implementation
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn helm_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_enabled(ctx.w)?;
    let actor = ctx.actor;
    let switch = active_switch(ctx, &HELM_SWITCHES);
    let raw = ctx.args.raw().trim().to_owned();

    let ship_id = ctx
        .w
        .module_state::<SpaceBook>()
        .map(|b| b.ship_id_for_actor(ctx.w, actor))
        .unwrap_or_else(|| format!("ship.{}", actor.0));

    match switch {
        None => {
            let summary = format_helm_status(ctx.w, actor, &ship_id);
            ctx.out.info(actor, summary);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("enter") => {
            if raw.is_empty() {
                return Err(CmdError::msg(
                    "Usage: +helm/enter <kirkwood_picket|pallas_inclined_belt|ceres_halo|kirkwood_gap_3_1|saturn_b_ring_edge>",
                ));
            }
            let now_ic = ctx.w.world_ic_now().at;
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            book.actor_ships.insert(actor, ship_id.clone());

            let msg = if raw == "kirkwood_picket" {
                let enc = KirkwoodPicketEncounter::promote_interception(
                    now_ic,
                    HaulerManifest {
                        strategic_ore_kg: 18_000,
                        metal_ore_kg: 12_000,
                        valid_registry_claim: true,
                        contraband_aboard: false,
                    },
                );
                book.pickets.insert(ship_id.clone(), enc);
                book.ship_context
                    .insert(ship_id.clone(), ActiveSpaceContext::Picket(ship_id));
                "Interdiction triggered at Kirkwood 3:1 Picket: CarrierJourney paused (Promoted); Republic Customs Corvette hails you to heave-to (<= 10 m/s).".to_owned()
            } else if matches!(
                raw.as_str(),
                "ceres_halo" | "kirkwood_gap_3_1" | "pallas_inclined_belt" | "saturn_b_ring_edge"
            ) {
                let field = book
                    .clusters
                    .entry(raw.clone())
                    .or_insert_with(|| AsteroidClusterField::new_canonical_cluster(&raw, now_ic));
                let mut dray_p = SpaceShipState::new_dray(
                    &ship_id,
                    "UNS-PROSPECTOR",
                    SpacePartyId(1),
                    [0, 0, 0],
                    [0, 0, 0],
                );
                dray_p
                    .lasers
                    .push(LaserMount::standard_turret("turret_laser_1"));
                field.insert_ship_with_hold(dray_p, OreHoldState::new_dray_p());
                book.ship_context
                    .insert(ship_id, ActiveSpaceContext::Cluster(raw.clone()));
                format!(
                    "Entered orbital asteroid cluster '{raw}' on 1:1 wall-clock time; Dray-P stow.ore bay online."
                )
            } else {
                let bubble = LocalSpaceBubble::new(
                    format!("bubble.{raw}"),
                    &raw,
                    raw.to_uppercase(),
                    now_ic,
                );
                let mut dray = SpaceShipState::new_dray(
                    &ship_id,
                    "DRAY HAULER",
                    SpacePartyId(1),
                    [0, 0, 0],
                    [0, 0, 0],
                );
                dray.lasers
                    .push(LaserMount::standard_turret("turret_laser_1"));
                let entry = book.bubbles.entry(raw.clone()).or_insert(bubble);
                entry.insert_ship(dray);
                book.ship_context
                    .insert(ship_id, ActiveSpaceContext::Bubble(raw.clone()));
                format!("Entered local space bubble '{raw}' on 1:1 wall-clock time.")
            };

            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("burn") => {
            let parts: Vec<&str> = raw.split_whitespace().collect();
            if parts.len() < 3 {
                return Err(CmdError::msg("Usage: +helm/burn <ax> <ay> <az> [force]"));
            }
            let ax: i64 = parts
                .first()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| CmdError::msg("Invalid X burn component."))?;
            let ay: i64 = parts
                .get(1)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| CmdError::msg("Invalid Y burn component."))?;
            let az: i64 = parts
                .get(2)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| CmdError::msg("Invalid Z burn component."))?;
            let force = parts
                .get(3)
                .is_some_and(|w| w.eq_ignore_ascii_case("force"));

            let sky_opt = current_sky(ctx.w);
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;

            if let Some((bubble, resolved_id)) = book.bubble_for_ship_mut(&ship_id) {
                match bubble.command_burn(&resolved_id, [ax, ay, az], force) {
                    Ok(()) => {
                        let msg = format!(
                            "Main drive thrust set to [{ax}, {ay}, {az}] m/s^2 (force={force})."
                        );
                        ctx.out.info(actor, msg);
                        push_actor_space_feeds(ctx, actor);
                        return Ok(());
                    }
                    Err(warn) => {
                        return Err(CmdError::msg(format!(
                            "TANK_RESERVE_WARN: burn dips into RETURN_RESERVE ({warn:?}); pass 'force' to override."
                        )));
                    }
                }
            }
            if let Some(flight) = book.flights.get_mut(&ship_id) {
                let sky = sky_opt.ok_or_else(|| CmdError::msg("Sky ephemeris unavailable."))?;
                let mag_sq = ax * ax + ay * ay + az * az;
                let dv_mag = (mag_sq as f64).sqrt().round() as i64;
                let heading = if dv_mag > 0 {
                    [
                        ((ax * 1_000) / dv_mag) as i32,
                        ((ay * 1_000) / dv_mag) as i32,
                        ((az * 1_000) / dv_mag) as i32,
                    ]
                } else {
                    [1_000, 0, 0]
                };
                let thrust = flight.max_accel_mps2.max(1);
                let dur_s = (dv_mag.max(0) as u64).div_ceil(thrust as u64).max(1);
                flight
                    .command_burn_dv(heading, thrust, dur_s, force)
                    .map_err(|e| CmdError::msg(format!("Free-flight burn refused: {e}")))?;
                flight.step_ic_seconds(&sky, dur_s);
                let msg = format!(
                    "Executed free-flight burn [{ax}, {ay}, {az}] m/s (remaining_dv={} m/s, propellant={} kg).",
                    flight.delta_v_mps, flight.propellant_kg
                );
                ctx.out.info(actor, msg);
                push_actor_space_feeds(ctx, actor);
                return Ok(());
            }
            Err(CmdError::msg(
                "Your ship is neither in an active space bubble nor decoupled in System Free Flight.",
            ))
        }
        Some("roll") => {
            let parts: Vec<i32> = raw
                .split_whitespace()
                .filter_map(|s| s.parse::<i32>().ok())
                .collect();
            let attitude = match parts.as_slice() {
                [roll] => (0, 0, *roll),
                [yaw, pitch, roll, ..] => (*yaw, *pitch, *roll),
                _ => {
                    return Err(CmdError::msg(
                        "Usage: +helm/roll <roll_deg> | +helm/roll <yaw_deg> <pitch_deg> <roll_deg>",
                    ));
                }
            };
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            if let Some((bubble, resolved_id)) = book.bubble_for_ship_mut(&ship_id)
                && let Some(ship) = bubble.ships.get_mut(&resolved_id)
            {
                ship.attitude_deg = attitude;
                let msg = format!(
                    "Hull attitude trimmed to yaw={} deg, pitch={} deg, roll={} deg.",
                    attitude.0, attitude.1, attitude.2
                );
                ctx.out.info(actor, msg);
                push_actor_space_feeds(ctx, actor);
                return Ok(());
            }
            Err(CmdError::msg("No ship in an active space bubble."))
        }
        Some("match") => {
            if raw.is_empty() {
                return Err(CmdError::msg("Usage: +helm/match <rock_id>"));
            }
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let Some(ActiveSpaceContext::Cluster(cid)) = book.ship_context.get(&ship_id).cloned()
            else {
                return Err(CmdError::msg(
                    "You must be inside an orbital asteroid cluster (+helm/enter <cluster>) to match velocity with a rock.",
                ));
            };
            let cluster = book
                .clusters
                .get_mut(&cid)
                .ok_or_else(|| CmdError::msg("Cluster not found."))?;
            let (dist_m, rel_mps) = cluster
                .match_velocity_with_rock(&ship_id, &raw)
                .map_err(|e| CmdError::msg(format!("Velocity match refused: {e}")))?;
            let msg = format!(
                "Matched 3D velocity with '{raw}': standoff distance {dist_m} m, relative speed {rel_mps} m/s (within <= 5 m/s mining gate)."
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("decouple") => {
            let parts: Vec<&str> = raw.split_whitespace().collect();
            let from_str = parts.first().copied().unwrap_or("ceres");
            let to_str = parts.get(1).copied().unwrap_or("mars");
            let from_body = CanonicalId::new(from_str)
                .map_err(|_| CmdError::msg("Invalid departure body ID."))?;
            let to_body = CanonicalId::new(to_str)
                .map_err(|_| CmdError::msg("Invalid destination body ID."))?;
            let sky =
                current_sky(ctx.w).ok_or_else(|| CmdError::msg("Sky ephemeris unavailable."))?;
            let epoch = 74_152_540_800_i64;
            let flight = SpaceFlightRecord::decouple_from_leg(
                &sky,
                &ship_id,
                "UNS-EXPLORER",
                "dray_p",
                SpacePartyId(1),
                &from_body,
                &to_body,
                epoch,
                epoch + 30_000,
                epoch + 10_000,
                24_000,
                24_000,
            )
            .map_err(|e| CmdError::msg(format!("Decouple failed: {e}")))?;
            let pos_km = flight.pos_km();
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            book.actor_ships.insert(actor, ship_id.clone());
            book.flights.insert(ship_id.clone(), flight);
            book.ship_context
                .insert(ship_id.clone(), ActiveSpaceContext::FreeFlight(ship_id));
            let msg = format!(
                "Decoupled from '{from_str} -> {to_str}' transit leg into Tier-1 3D System Free Flight at [{}, {}, {}] km.",
                pos_km.x, pos_km.y, pos_km.z
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("droptanks") => {
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let flight = book.flights.get_mut(&ship_id).ok_or_else(|| {
                CmdError::msg("Decouple into System Free Flight (+helm/decouple) first.")
            })?;
            flight.fit_long_range_drop_tanks();
            let msg = format!(
                "Fitted long-range external expedition drop tanks: remaining_dv={} m/s, propellant={} kg.",
                flight.delta_v_mps, flight.propellant_kg
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("expedition") => {
            if raw.is_empty() {
                return Err(CmdError::msg(
                    "Usage: +helm/expedition <saturn|uranus|neptune|triton|jupiter>",
                ));
            }
            let target_body = CanonicalId::new(&raw)
                .map_err(|_| CmdError::msg("Invalid outer-system body ID."))?;
            let sky =
                current_sky(ctx.w).ok_or_else(|| CmdError::msg("Sky ephemeris unavailable."))?;
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let flight = book.flights.get_mut(&ship_id).ok_or_else(|| {
                CmdError::msg("Decouple into System Free Flight (+helm/decouple) first.")
            })?;
            let flyby = flight
                .fly_expedition_to_body(&sky, &target_body, 90_000, true)
                .map_err(|e| CmdError::msg(format!("Expedition burn refused: {e}")))?;
            let ceres_id = CanonicalId::new("ceres").unwrap_or_else(|_| target_body.clone());
            let lag_s = flight.light_lag_to_body_s(&sky, &ceres_id).unwrap_or(0);
            let msg = format!(
                "Outer-system expedition arrived at '{}': transit distance {} km, one-way radio light-lag {} s ({} min).",
                flyby.dest_body,
                flyby.distance_km,
                lag_s,
                lag_s / 60
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("autopilot") => {
            let dest_str = if raw.is_empty() {
                "ceres"
            } else {
                raw.as_str()
            };
            let dest_body = CanonicalId::new(dest_str)
                .map_err(|_| CmdError::msg("Invalid destination body ID."))?;
            let sky =
                current_sky(ctx.w).ok_or_else(|| CmdError::msg("Sky ephemeris unavailable."))?;
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let flight = book.flights.get_mut(&ship_id).ok_or_else(|| {
                CmdError::msg("Your ship is not currently in Tier-1 System Free Flight.")
            })?;
            let ticket = flight
                .recouple_autopilot(&sky, &dest_body, false)
                .map_err(|e| CmdError::msg(format!("Autopilot recouple refused: {e}")))?;
            let msg = format!(
                "Autopilot recoupled to '{}': distance {} km, debited dv {} m/s, transit {} s (arrive_ic={}).",
                ticket.dest_body,
                ticket.distance_km,
                ticket.debited_dv_mps,
                ticket.arrive_ic - ticket.depart_ic,
                ticket.arrive_ic
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("dock") => helm_dock_subcmd(ctx, actor, &ship_id, &raw),
        Some("tow") => {
            let now_ic = ctx.w.world_ic_now().at.max(0) as u64;
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let session = book.dockings.get(&ship_id).ok_or_else(|| {
                CmdError::msg(
                    "No clamped derelict docking session found (+helm/dock derelict <id>).",
                )
            })?;
            let tow_cap = session.ship_tonnage_t * 2;
            let tow = session
                .attach_salvage_tow(now_ic)
                .map_err(|e| CmdError::msg(format!("Salvage tow refused: {e}")))?;
            let msg = format!(
                "Salvage tow line attached to derelict '{}' ({} t <= {} t 2x cap); 48 h Freeport contest window open until IC {} s.",
                tow.derelict_ship_id, tow.derelict_tonnage_t, tow_cap, tow.contest_expires_ic_secs
            );
            book.tows.insert(ship_id, tow);
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some(other) => Err(CmdError::msg(format!("Unknown +helm switch '{other}'."))),
    }
}

fn helm_dock_subcmd(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    ship_id: &str,
    raw: &str,
) -> Result<(), CmdError> {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    let sub = parts.first().copied().unwrap_or("");
    let book = ctx
        .w
        .module_state_mut::<SpaceBook>()
        .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;

    match sub {
        "port" => {
            let berth = parts.get(1).copied().unwrap_or("CERES-BERTH-04");
            let session = DockingSession::start_port_approach(
                format!("dock.{ship_id}"),
                ship_id,
                SpacePartyId(1),
                400,
                "ceres_port",
                berth,
                DockingPose6Dof {
                    dx_mm: 80,
                    dy_mm: -60,
                    dz_mm: 800,
                    pitch_mdeg: 250,
                    yaw_mdeg: -200,
                    roll_mdeg: 300,
                    vx_mmps: 0,
                    vy_mmps: 0,
                    vz_mmps: 450,
                    vpitch_mdeg_s: 0,
                    vyaw_mdeg_s: 0,
                    vroll_mdeg_s: 0,
                },
            );
            book.dockings.insert(ship_id.to_owned(), session);
            ctx.out.info(
                actor,
                format!("Entered out-of-time-loop 6-DOF port docking session at '{berth}'."),
            );
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        "derelict" => {
            let target_id = parts
                .get(1)
                .copied()
                .ok_or_else(|| CmdError::msg("Usage: +helm/dock derelict <target_ship_id>"))?;
            let (bubble, resolved_id) = book.bubble_for_ship(ship_id).ok_or_else(|| {
                CmdError::msg("You must be in a space bubble containing the derelict.")
            })?;
            let session = DockingSession::start_derelict_docking(
                bubble,
                format!("dock.{ship_id}"),
                resolved_id,
                target_id,
                400,
                550,
                DockingPose6Dof {
                    dx_mm: 90,
                    dy_mm: -70,
                    dz_mm: 800,
                    pitch_mdeg: 300,
                    yaw_mdeg: -250,
                    roll_mdeg: 400,
                    vx_mmps: 0,
                    vy_mmps: 0,
                    vz_mmps: 450,
                    vpitch_mdeg_s: 0,
                    vyaw_mdeg_s: 0,
                    vroll_mdeg_s: 0,
                },
            )
            .map_err(|e| CmdError::msg(format!("Derelict docking refused: {e}")))?;
            book.dockings.insert(ship_id.to_owned(), session);
            ctx.out.info(
                actor,
                format!(
                    "Entered mandatory out-of-time-loop 6-DOF hard-docking session with derelict '{target_id}'."
                ),
            );
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        "coast" | "clamp" => {
            let session = book.dockings.get_mut(ship_id).ok_or_else(|| {
                CmdError::msg("No active 6-DOF docking session (+helm/dock port|derelict ...).")
            })?;
            for _ in 0..200 {
                let st = session.step_frame_60fps(DockingControlInput::default());
                if matches!(st, DockingSessionState::Completed(_)) {
                    break;
                }
            }
            let summary = match &session.state {
                DockingSessionState::Completed(DockingOutcome::Clamped {
                    latch_closure_mmps,
                    saved_delta_v_mps,
                    saved_approach_ic_secs,
                    ..
                }) => format!(
                    "HARD-DOCK CLAMPED at {latch_closure_mmps} mm/s (saved_dv={saved_delta_v_mps} m/s, saved_time={saved_approach_ic_secs} IC s)."
                ),
                other => format!("Docking session state: {other:?}"),
            };
            ctx.out.info(actor, summary);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        "abort" => {
            let session = book
                .dockings
                .get_mut(ship_id)
                .ok_or_else(|| CmdError::msg("No active 6-DOF docking session."))?;
            let _ = session.step_frame_60fps(DockingControlInput {
                abort: true,
                ..DockingControlInput::default()
            });
            ctx.out
                .info(actor, "6-DOF docking session aborted; backing clear.");
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        _ => Err(CmdError::msg(
            "Usage: +helm/dock port [<berth>] | +helm/dock derelict <target_id> | +helm/dock clamp | +helm/dock abort",
        )),
    }
}

fn format_helm_status(w: &World, actor: ObjId, ship_id: &str) -> String {
    let Some(book) = w.module_state::<SpaceBook>() else {
        return "Spaceflight telemetry unavailable.".to_owned();
    };
    if let Some((bubble, resolved_id)) = book.bubble_for_ship(ship_id)
        && let Some(ship) = bubble.ships.get(resolved_id)
    {
        return format!(
            "Helm [{ship_id}] in bubble '{}': pos=[{}, {}, {}] m, vel=[{}, {}, {}] mm/s, attitude={:?} deg, dv={} m/s (reserve={} m/s).",
            bubble.title,
            ship.pos_m[0],
            ship.pos_m[1],
            ship.pos_m[2],
            ship.vel_mmps[0],
            ship.vel_mmps[1],
            ship.vel_mmps[2],
            ship.attitude_deg,
            ship.delta_v_mps,
            ship.return_reserve_dv_mps
        );
    }
    if let Some(flight) = book.flights.get(ship_id) {
        let p = flight.pos_km();
        return format!(
            "Helm [{ship_id}] in Tier-1 System Free Flight: pos=[{}, {}, {}] km, dv={} m/s (reserve={} m/s), drop_tanks={}.",
            p.x,
            p.y,
            p.z,
            flight.delta_v_mps,
            flight.return_reserve_dv_mps,
            flight.drop_tanks_fitted
        );
    }
    format!(
        "Helm [{ship_id}] (actor #{}): standing by in port or on filed autopilot. Use +helm/enter or +helm/decouple.",
        actor.0
    )
}

// ---------------------------------------------------------------------------
// `+tac` command implementation
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn tac_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_enabled(ctx.w)?;
    let actor = ctx.actor;
    let switch = active_switch(ctx, &TAC_SWITCHES);
    let raw = ctx.args.raw().trim().to_owned();

    let ship_id = ctx
        .w
        .module_state::<SpaceBook>()
        .map(|b| b.ship_id_for_actor(ctx.w, actor))
        .unwrap_or_else(|| format!("ship.{}", actor.0));

    match switch {
        None => {
            let book = ctx
                .w
                .module_state::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let (bubble, resolved_id) = book
                .bubble_for_ship(&ship_id)
                .ok_or_else(|| CmdError::msg("No active space bubble (+helm/enter <site>)."))?;
            let ship = bubble
                .ships
                .get(resolved_id)
                .ok_or_else(|| CmdError::msg("Ship not found in bubble."))?;
            let msg = format!(
                "Tactical [{}]: status={:?}, heat={} permille, pdc_auto={}, torps_in_flight={}, slugs_in_flight={}, contacts={}.",
                ship.callsign,
                ship.status,
                ship.heat_permille,
                ship.pdc_auto,
                bubble.torpedoes.len(),
                bubble.railgun_slugs.len(),
                bubble.ships.len().saturating_sub(1)
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("pdc") => {
            let on = match raw.as_str() {
                "auto" | "on" => true,
                "off" | "manual" => false,
                _ => return Err(CmdError::msg("Usage: +tac/pdc <auto|off>")),
            };
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let (bubble, resolved_id) = book
                .bubble_for_ship_mut(&ship_id)
                .ok_or_else(|| CmdError::msg("No active space bubble."))?;
            let ship = bubble
                .ships
                .get_mut(&resolved_id)
                .ok_or_else(|| CmdError::msg("Ship not found in bubble."))?;
            ship.pdc_auto = on;
            ctx.out.info(
                actor,
                format!("Rotary PDC point-defense network set to auto={on}."),
            );
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("laser") => {
            let mode = match raw.as_str() {
                "offense" => LaserMode::Offense,
                "defense" => LaserMode::Defense,
                _ => return Err(CmdError::msg("Usage: +tac/laser <offense|defense>")),
            };
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let (bubble, resolved_id) = book
                .bubble_for_ship_mut(&ship_id)
                .ok_or_else(|| CmdError::msg("No active space bubble."))?;
            let ship = bubble
                .ships
                .get_mut(&resolved_id)
                .ok_or_else(|| CmdError::msg("Ship not found in bubble."))?;
            for mount in &mut ship.lasers {
                mount.mode = mode;
            }
            ctx.out
                .info(actor, format!("Laser turrets switched to {mode:?} mode."));
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("torp") => {
            let parts: Vec<&str> = raw.split_whitespace().collect();
            let target_id = parts
                .first()
                .copied()
                .ok_or_else(|| CmdError::msg("Usage: +tac/torp <target_id> [<salvo_count>]"))?;
            let salvo: u8 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(2);
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let (bubble, resolved_id) = book
                .bubble_for_ship_mut(&ship_id)
                .ok_or_else(|| CmdError::msg("No active space bubble."))?;
            let events = bubble.fire_torpedo_salvo(&resolved_id, target_id, salvo);
            ctx.out.info(
                actor,
                format!(
                    "Launched {} guided torpedo(es) at '{target_id}'.",
                    events.len()
                ),
            );
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("rail") => {
            if raw.is_empty() {
                return Err(CmdError::msg("Usage: +tac/rail <target_id>"));
            }
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let (bubble, resolved_id) = book
                .bubble_for_ship_mut(&ship_id)
                .ok_or_else(|| CmdError::msg("No active space bubble."))?;
            let events = bubble.fire_railgun(&resolved_id, &raw);
            ctx.out.info(
                actor,
                format!("Fired {} railgun slug(s) at '{raw}'.", events.len()),
            );
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("beam") => {
            if raw.is_empty() {
                return Err(CmdError::msg("Usage: +tac/beam <target_id>"));
            }
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            let (bubble, resolved_id) = book
                .bubble_for_ship_mut(&ship_id)
                .ok_or_else(|| CmdError::msg("No active space bubble."))?;
            let events = bubble.fire_offensive_lasers(&resolved_id, &raw);
            ctx.out.info(
                actor,
                format!(
                    "Fired offensive laser pulse(s) at '{raw}' ({} hit events).",
                    events.len()
                ),
            );
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("step") => {
            let wall_ms: u32 = if raw.is_empty() {
                1_000
            } else {
                raw.parse()
                    .map_err(|_| CmdError::msg("Usage: +tac/step [<wall_ms>]"))?
            };
            let book = ctx
                .w
                .module_state_mut::<SpaceBook>()
                .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
            if let Some(ActiveSpaceContext::Picket(key)) = book.ship_context.get(&ship_id).cloned()
                && let Some(enc) = book.pickets.get_mut(&key)
            {
                if matches!(enc.phase, PicketPhase::HeavedToInspecting { .. }) {
                    let _ = enc.trigger_raider_ambush();
                }
                let rep = enc.step_wall_ms(wall_ms);
                let msg = format!(
                    "Stepped Kirkwood Picket bubble +{} ms: phase={:?}, events={}.",
                    rep.wall_advanced_ms,
                    enc.phase,
                    rep.events.len()
                );
                ctx.out.info(actor, msg);
                push_actor_space_feeds(ctx, actor);
                return Ok(());
            }
            let (bubble, _) = book
                .bubble_for_ship_mut(&ship_id)
                .ok_or_else(|| CmdError::msg("No active space bubble."))?;
            let rep = bubble.step_wall_ms(wall_ms);
            let msg = format!(
                "Stepped 1:1 space bubble +{} ms (substeps={}, events={}).",
                rep.wall_advanced_ms,
                rep.substeps_executed,
                rep.events.len()
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some(other) => Err(CmdError::msg(format!("Unknown +tac switch '{other}'."))),
    }
}

// ---------------------------------------------------------------------------
// `+mine` command implementation
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn mine_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_enabled(ctx.w)?;
    let actor = ctx.actor;
    let switch = active_switch(ctx, &MINE_SWITCHES);
    let raw = ctx.args.raw().trim().to_owned();

    let ship_id = ctx
        .w
        .module_state::<SpaceBook>()
        .map(|b| b.ship_id_for_actor(ctx.w, actor))
        .unwrap_or_else(|| format!("ship.{}", actor.0));

    let book = ctx
        .w
        .module_state_mut::<SpaceBook>()
        .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
    let Some(ActiveSpaceContext::Cluster(cid)) = book.ship_context.get(&ship_id).cloned() else {
        return Err(CmdError::msg(
            "You must be inside an orbital asteroid cluster (+helm/enter <cluster>) to use +mine.",
        ));
    };
    let field = book
        .clusters
        .get_mut(&cid)
        .ok_or_else(|| CmdError::msg("Cluster not found."))?;

    match switch {
        None => {
            let hold = field
                .holds
                .get(&ship_id)
                .ok_or_else(|| CmdError::msg("Ship hold not found in cluster."))?;
            let msg = format!(
                "Asteroid Cluster '{}' ({:?}): stow.ore={}/{} kg ({}/{} L), rocks={}.",
                field.cluster_id,
                field.jurisdiction,
                hold.used_mass_kg(),
                hold.max_mass_kg,
                hold.used_volume_l(),
                hold.max_volume_l,
                field.rocks.len()
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("ping") => {
            if raw.is_empty() {
                return Err(CmdError::msg("Usage: +mine/ping <rock_id>"));
            }
            let (spec, pools) = field
                .ping_rock(&ship_id, &raw)
                .map_err(|e| CmdError::msg(format!("Ping refused: {e}")))?;
            let pool_desc: Vec<String> = pools
                .iter()
                .map(|(g, w)| format!("{}:{w:?}", g.as_str()))
                .collect();
            let msg = format!(
                "Pinged rock '{raw}': class={}, deposits=[{}].",
                spec.code_str(),
                pool_desc.join(", ")
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("licence") => {
            let lic = field.issue_prospecting_licence(&ship_id);
            let msg = format!(
                "Issued Freeport Registry prospecting licence for '{}': fee={} cr, expires IC {} s.",
                lic.holder_id, lic.fee_paid_cr, lic.expires_at_ic_s
            );
            ctx.out.info(actor, msg);
            Ok(())
        }
        Some("stake") => {
            if raw.is_empty() {
                return Err(CmdError::msg("Usage: +mine/stake <rock_id>"));
            }
            field
                .stake_claim(&ship_id, &raw)
                .map_err(|e| CmdError::msg(format!("Claim stake refused: {e}")))?;
            ctx.out.info(
                actor,
                format!(
                    "Staked Freeport Registry claim on rock '{raw}' (48 h contest window open)."
                ),
            );
            Ok(())
        }
        Some("contest") => {
            if raw.is_empty() {
                return Err(CmdError::msg("Usage: +mine/contest <rock_id>"));
            }
            field
                .contest_claim(&ship_id, &raw)
                .map_err(|e| CmdError::msg(format!("Claim contest refused: {e}")))?;
            ctx.out.info(
                actor,
                format!("Filed Freeport Registry claim contest on rock '{raw}' using prior hand-made ping evidence."),
            );
            Ok(())
        }
        Some("laser") => {
            let parts: Vec<&str> = raw.split_whitespace().collect();
            let rock_id = parts
                .first()
                .copied()
                .ok_or_else(|| CmdError::msg("Usage: +mine/laser <rock_id> [<requested_kg>]"))?;
            let req_kg: u64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(4_000);
            let rep = field
                .extract_with_mining_laser(&ship_id, rock_id, req_kg)
                .map_err(|e| CmdError::msg(format!("Mining laser extraction refused: {e}")))?;
            // If claim_jump_flagged in frontier space and no jumper is present yet, spawn rival cutter
            if rep.claim_jump_flagged && !field.bubble.ships.contains_key("claim_jumper") {
                let mut jumper = SpaceShipState::new_corvette(
                    "claim_jumper",
                    "SYNDICATE-CUTTER",
                    SpacePartyId(2),
                    [35_000, 0, 0],
                    [0, 0, 0],
                );
                jumper.attitude_deg = (0, 0, 0);
                if let Some(drive) = jumper.sections.get_mut(&ShipSectionId::Drive) {
                    drive.facets.insert(super::bubble::FacetBand::Rear, 0);
                    drive.internals = 10;
                }
                if let Some(ship) = field.bubble.ships.get_mut(&ship_id) {
                    ship.pos_m = [34_750, 0, 0];
                    ship.vel_mmps = [0, 0, 0];
                }
                field.bubble.insert_ship(jumper);
                field.bubble.set_party_relation(
                    SpacePartyId(1),
                    SpacePartyId(2),
                    SpaceRelation::Hostile,
                );
            }
            let msg = format!(
                "Extracted {} kg of '{}' from '{}' into stow.ore (hold={}/100000 kg, ir_bloom={} permille, claim_jump={}).",
                rep.extracted_kg,
                rep.good.as_str(),
                rep.rock_id,
                rep.hold_used_kg,
                rep.ir_bloom_permille,
                rep.claim_jump_flagged
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some("drones") => {
            let parts: Vec<&str> = raw.split_whitespace().collect();
            let rock_id = parts
                .first()
                .copied()
                .ok_or_else(|| CmdError::msg("Usage: +mine/drones <rock_id> [<count_1_to_4>]"))?;
            let count: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(4);
            let (exc_id, drones) = field
                .launch_collector_drones(&ship_id, rock_id, count)
                .map_err(|e| CmdError::msg(format!("Drone launch refused: {e}")))?;
            let rep = field
                .complete_drone_sortie_cycle(&ship_id)
                .map_err(|e| CmdError::msg(format!("Drone sortie recovery failed: {e}")))?;
            let msg = format!(
                "Collector drone sortie (ExcursionId={}, {} drones) recovered {} kg of '{}' from '{}' into stow.ore.",
                exc_id.0,
                drones.len(),
                rep.extracted_kg,
                rep.good.as_str(),
                rep.rock_id
            );
            ctx.out.info(actor, msg);
            push_actor_space_feeds(ctx, actor);
            Ok(())
        }
        Some(other) => Err(CmdError::msg(format!("Unknown +mine switch '{other}'."))),
    }
}
