//! **The game's content, compiled in** (16b cut C.2; doc 16 §4 "Content
//! packs").
//!
//! Ten files under `crates/tacsim/content/` (the cores pack, 17b cut M3.1, the
//! tenth), `include_str!`ed here and pinned
//! by digest in `crates/tacsim/tests/content.rs`, in the shape
//! `mush_atlas::starter` already uses for the lot: **the game's own content
//! loads on every boot without an operator naming a path**.
//!
//! # Two routes, one loader, one place they meet
//!
//! `[tacsim].pack_paths` is the operator's route: the host reads the bytes and
//! [`crate::verify_sources`] parses and verifies them into the
//! [`crate::store::TacsimStore`]'s pack ledger, which `@tacsim/pack` reads. The
//! built-in route is the same bytes from `include_str!` through
//! [`sources`]. **Both go through one [`crate::pack::load_pack`]** — one strict
//! wire, one digest verification, one set of entry bounds — and both land in one
//! [`crate::definitions::Catalog::build_game`], which is what the composition
//! root's unit catalog, the module's registry descriptors and the arena
//! applier's bindings are all three readings of. There is no second loader and
//! no second validation, which is the only way two routes can be made to agree.
//!
//! What the built-in route deliberately does **not** enter is the operator's
//! pack *ledger*: no `tacsim:pack:` row, no `@tacsim/pack list` line, and
//! `[tacsim].enabled = false` stays E7 cut 1's promise that the layer is
//! bit-identical to one built before it existed. A compiled-in pack cannot
//! change under a running world, so the row that exists to catch a re-signed
//! file has nothing to catch; what a *changed* shipped definition does is move
//! its [`mush_core::mechanics::DefinitionRef`] digest, and a persisted unit
//! pointing at the old one then fails M32's own exact-definition check at boot —
//! which is the layer that owns the question. An operator who names one of these
//! files on `pack_paths` anyway gets it in the ledger **and** deduplicated out
//! of the catalog by `(id, version)`, so the definitions are registered once.
//!
//! # What ships, and through which latch
//!
//! | file | pack | kind | applied |
//! |---|---|---|---|
//! | `ammunition/he_105.json` | `tacsim.pack.he_105` | rules | never — a definition |
//! | `ammunition/ball_762.json` | `tacsim.pack.ball_762` | rules | never |
//! | `weapons/cannon_105.json` | `tacsim.pack.cannon_105` | rules | never |
//! | `weapons/mg_coax.json` | `tacsim.pack.mg_coax` | rules | never |
//! | `sensors/visual_mk1.json` | `tacsim.pack.visual_mk1` | rules | never |
//! | `cores/cores.json` | `tacsim.pack.cores` | rules | never (17b cut M3.1) |
//! | `units/line_tank.json` | `tacsim.pack.line_tank` | rules | never |
//! | `resolution.json` | `tacsim.pack.resolution` | rules | never |
//! | `atlas/proving_ground.json` | `tacsim.proving_ground` | atlas region | the content latch, at a first boot |
//! | `arenas/proving_ground.json` | `tacsim.pack.proving_ground` | seed | the content latch, after it |
//!
//! A *definition* pack is verified and registered and applies nothing: there is
//! no world state in a weapon. The last two **instantiate** — the region's tiles
//! and the arena's room, domain and twelve tanks — so they go through
//! `mush_content`'s latch as boot packs, in that order, exactly as the lot's
//! three starter packs do.
//!
//! `resources.json` is not a pack at all; see [`super::resources`].
//!
//! # And what ships beside it without being shipped
//!
//! **16b cut X.1** adds three more files under `content/` — the laser, its charge
//! and the tank that carries it — which are [`EXTENSIONS`] and deliberately not
//! [`SOURCES`]. See that constant for why. **16b cut X.2** adds the biped as
//! [`EXTENSIONS_BIPED`], a table of its own: see that constant for why *that*.
//! **16b cut X.3** adds the thermal sight and the tank that mounts it as
//! [`EXTENSIONS_THERMAL`], on the same rule. **16b cut X.4** adds the jamming pod
//! and the tank that carries it as [`EXTENSIONS_ECM`], on the same rule again.
//! **16b cut X.5** adds the counter-jamming filter and its tank as
//! [`EXTENSIONS_ECCM`], the last of doc 16 §9's five. **16b cut O.3** adds the
//! range's target as [`EXTENSIONS_TARGET`], a sixth table on the same rule.
//! **16b cut AE.1** adds three chassis — a wheeled scout car, a heavy tank and
//! a second biped — as [`EXTENSIONS_CHASSIS`], a seventh:
//!
//! | file | pack | kind | applied |
//! |---|---|---|---|
//! | `units/scout_mk1.json` | `tacsim.pack.scout_mk1` | rules | never |
//! | `units/heavy_mk1.json` | `tacsim.pack.heavy_mk1` | rules | never |
//! | `units/biped_mk2.json` | `tacsim.pack.biped_mk2` | rules | never |
//!
//! **16b cut AE.2** adds two weapons and their rounds — the scout car's 30 mm
//! autocannon and the heavy's missile rack — as [`EXTENSIONS_ORDNANCE`], an
//! eighth, and moves the scout and the heavy to version 2 to carry them:
//!
//! | file | pack | kind | applied |
//! |---|---|---|---|
//! | `ammunition/ap_30.json` | `tacsim.pack.ap_30` | rules | never |
//! | `weapons/autocannon_30.json` | `tacsim.pack.autocannon_30` | rules | never |
//! | `ammunition/missile_round.json` | `tacsim.pack.missile_round` | rules | never |
//! | `weapons/missile_lr.json` | `tacsim.pack.missile_lr` | rules | never |
//!
//! **16b cut AE.3** adds the ground drone — a small tracked sight with no gun,
//! the one unit of class `drone` (doc 16 §2 W2) — as [`EXTENSIONS_DRONE`], a
//! ninth, fielded last:
//!
//! | file | pack | kind | applied |
//! |---|---|---|---|
//! | `units/drone_mk1.json` | `tacsim.pack.drone_mk1` | rules | never |
//!
//! **Doc 18 cut IC2** adds the trade run's hull — the Dray, class `hauler`,
//! carrying `logistics` and `interior` (doc 18 §4.2, D-18-29) — as
//! [`EXTENSIONS_HAULER`], a tenth, fielded last and never fielded in an arena:
//!
//! | file | pack | kind | applied |
//! |---|---|---|---|
//! | `units/dray.json` | `tacsim.pack.dray` | rules | never |
//!
//! # The trade run's lane (doc 18 cut IC2)
//!
//! [`LANE_SOURCES`] is the 2350 game's one campaign-seed pack of its own:
//!
//! | file | pack | kind | applied |
//! |---|---|---|---|
//! | `lane/ceres_phobos.json` | `tacsim.lane.ceres_phobos` | seed | the content latch, after the stations |
//!
//! the `sol` region, Ceres, Mars, Pallas and Kirkwood at their true longitudes on
//! one real date (doc 18 §4.8, D-18-48; the numbers come from
//! `content/sky/fetch_horizons.py`), the three places and the two routes. It is
//! the one game pack that is both **loaded** into the [`crate::store::TacsimStore`]
//! (through [`game_seed_sources`], so its routes materialize) and **applied** by
//! the content latch (through [`lane_packs`], so its places are rooms), and both
//! are switched by the one `game_content` boolean the composition root passes
//! (lead ruling R2). `goods.json` beside it is a typed document, not a pack; see
//! [`super::goods`].
//!
//! # And what is **fielded**
//!
//! **16b cut Q.8** answers X.1 deviation 1's open question — *whether the
//! extensions join the shipped catalog* — with **yes for the game, and no for
//! the stock game**. [`fielded_sources`] is [`sources`] followed by all ten
//! extension tables in `requires` order — doc 16 §9's five, since 16b cut O.3
//! the range's target, since 16b cut AE.2 the two weapons, since 16b cut
//! AE.1 the three chassis, since 16b cut AE.3 the drone, and since doc 18 cut
//! IC2 the Dray (in the catalog, never in an arena) — and
//! [`super::Catalog::build_fielded`] is what the three production sites build
//! from, so `SIM UNIT` can offer a pilot a biped or a scout car, `@unit/create`
//! can name a laser tank on a shipped server, and `SIM RANGE` can field a
//! target.
//!
//! [`SOURCES`] and [`sources`] are untouched, and so is
//! [`super::Catalog::build_game`]: "the stock game" is a statement about the
//! packs in the table above — eight until **17b cut M3.1** added the cores pack
//! as a ninth, because every chassis fits a core from then on and a chassis
//! must find its core in the game it is loaded into — it is what every cut-X
//! acceptance compares
//! its own item against, and a cut that moved it would have rewritten seven
//! tests about lasers, bipeds and pods to say something about a lobby.
//!
//! Nothing here is **latched**. A unit, weapon, ammunition or sensor definition
//! applies no world state, so the catalog is rebuilt from these bytes at every
//! boot and [`boot_packs`] — the two packs that really do instantiate — is
//! unmoved. A live world takes the fielded catalog at its next restart and its
//! standing tanks are untouched until somebody picks a type.

use crate::config::PackSource;

/// `crates/tacsim/content/ammunition/he_105.json`.
pub const HE_105_JSON: &str = include_str!("../../content/ammunition/he_105.json");
/// `crates/tacsim/content/ammunition/ball_762.json`.
pub const BALL_762_JSON: &str = include_str!("../../content/ammunition/ball_762.json");
/// `crates/tacsim/content/weapons/cannon_105.json`.
pub const CANNON_105_JSON: &str = include_str!("../../content/weapons/cannon_105.json");
/// `crates/tacsim/content/weapons/mg_coax.json`.
pub const MG_COAX_JSON: &str = include_str!("../../content/weapons/mg_coax.json");
/// `crates/tacsim/content/sensors/visual_mk1.json`.
pub const VISUAL_MK1_JSON: &str = include_str!("../../content/sensors/visual_mk1.json");
/// `crates/tacsim/content/cores/cores.json` (17b cut M3.1).
pub const CORES_JSON: &str = include_str!("../../content/cores/cores.json");
/// `crates/tacsim/content/units/line_tank.json`.
pub const LINE_TANK_JSON: &str = include_str!("../../content/units/line_tank.json");
/// `crates/tacsim/content/resolution.json`.
pub const RESOLUTION_JSON: &str = include_str!("../../content/resolution.json");
/// `crates/tacsim/content/arenas/proving_ground.json`.
pub const PROVING_GROUND_JSON: &str = include_str!("../../content/arenas/proving_ground.json");
/// `crates/tacsim/content/atlas/proving_ground.json` — the arena's ground, as an
/// ordinary atlas region pack.
pub const PROVING_GROUND_ATLAS_JSON: &str = include_str!("../../content/atlas/proving_ground.json");
/// `crates/tacsim/content/resources.json`.
pub const RESOURCES_JSON: &str = include_str!("../../content/resources.json");

/// The atlas pack's id — the arena entry's `pack` field names it.
pub const PROVING_GROUND_ATLAS_PACK: &str = "tacsim.proving_ground";

/// The arena seed pack's id.
pub const PROVING_GROUND_PACK: &str = "tacsim.pack.proving_ground";

/// The arena definition the shipped content declares.
pub const PROVING_GROUND: &str = "tacsim.arena.proving_ground";

// ---------------------------------------------------------------------------
// The trade run's two IC stations (doc 18 §4.5, §4.7; cut IC1)
// ---------------------------------------------------------------------------

/// Ceres' region pack: the body, the 2350 base theme, the port locality.
pub const CERES_REGION_PACK: &str = "region.ceres_freeport";
/// Ceres Freeport: thirteen rooms on three decks of the Freeport Hub, the Limbo
/// door `exit.limbo_arrivals` into Port Watch Arrivals (D-18-3), the cast, the
/// lore pools and the ambience (18a §1).
pub const CERES_PACK: &str = "neighborhood.ceres_freeport";
/// Stickney's region pack (Phobos).
pub const STICKNEY_REGION_PACK: &str = "region.stickney_yards";
/// Stickney Yards: twelve rooms on two decks of the Yard Rim, no foot route —
/// its Visiting Berth Gallery carries `port.dock` (D-18-19), 18a §2.
pub const STICKNEY_PACK: &str = "neighborhood.stickney_yards";
/// `crates/tacsim/content/atlas/region.ceres_freeport.json`.
pub const CERES_REGION_JSON: &str = include_str!("../../content/atlas/region.ceres_freeport.json");
/// `crates/tacsim/content/atlas/neighborhood.ceres_freeport.json`.
pub const CERES_JSON: &str = include_str!("../../content/atlas/neighborhood.ceres_freeport.json");
/// `crates/tacsim/content/atlas/region.stickney_yards.json`.
pub const STICKNEY_REGION_JSON: &str =
    include_str!("../../content/atlas/region.stickney_yards.json");
/// `crates/tacsim/content/atlas/neighborhood.stickney_yards.json`.
pub const STICKNEY_JSON: &str =
    include_str!("../../content/atlas/neighborhood.stickney_yards.json");
/// The four station packs in `requires` order (doc 18 §4.5, §4.7): each region
/// before its neighborhood, Ceres before Stickney — Stickney's NPCs draw the
/// `corridor_trade` and `tycho` pools Ceres' pack authors (each lore tag is
/// owned by exactly one pack).
pub const STATION_SOURCES: [(&str, &str); 4] = [
    (CERES_REGION_PACK, CERES_REGION_JSON),
    (CERES_PACK, CERES_JSON),
    (STICKNEY_REGION_PACK, STICKNEY_REGION_JSON),
    (STICKNEY_PACK, STICKNEY_JSON),
];
/// The rooms Ceres Freeport's pack makes (18a §1.3).
pub const CERES_ROOMS: usize = 13;
/// The rooms Stickney Yards' pack makes (18a §2.3).
pub const STICKNEY_ROOMS: usize = 12;

/// The unit schema the shipped content declares.
pub const LINE_TANK: &str = "tacsim.unit.line_tank";

/// The resolution table the shipped content declares.
pub const RESOLUTION: &str = "tacsim.resolution.stock";

/// **The cores pack** (17b cut M3.1; doc 17 §9, D-UM-33): the doctrine core
/// (Class I), the line core (II) and the command core (III), which every
/// shipped chassis but the range's target fits one of.
pub const CORES: &str = "tacsim.pack.cores";

/// The Class I core (`tacsim.pack.cores`): the scout's and the drone's.
pub const CORE_DOCTRINE: &str = "tacsim.core.doctrine";

/// The Class II core: the line tanks' and the bipeds'.
pub const CORE_LINE: &str = "tacsim.core.line";

/// The Class III core: the heavy's.
pub const CORE_COMMAND: &str = "tacsim.core.command";

// ---------------------------------------------------------------------------
// Doc 16 §9's extensions
// ---------------------------------------------------------------------------

/// `crates/tacsim/content/ammunition/laser_charge.json` (16b cut X.1).
pub const LASER_CHARGE_JSON: &str = include_str!("../../content/ammunition/laser_charge.json");
/// `crates/tacsim/content/weapons/laser_mk1.json` (16b cut X.1).
pub const LASER_MK1_JSON: &str = include_str!("../../content/weapons/laser_mk1.json");
/// `crates/tacsim/content/units/line_tank_laser.json` (16b cut X.1).
pub const LINE_TANK_LASER_JSON: &str = include_str!("../../content/units/line_tank_laser.json");

/// The laser weapon doc 16 §9 item 1 asks for.
pub const LASER_MK1: &str = "tacsim.weapon.laser_mk1";

/// The unit that carries it.
pub const LINE_TANK_LASER: &str = "tacsim.unit.line_tank_laser";

/// **Doc 16 §9 item 1's content, in `requires` order** (16b cut X.1): the laser,
/// its charge and the tank that carries it. Every later item of §9 has a table of
/// its own beside this one — see [`EXTENSIONS_BIPED`] for why.
///
/// Content the game **can** load and does not ship. It goes through exactly the
/// route an operator's `[tacsim].pack_paths` takes — one
/// [`crate::pack::load_pack`], one [`super::Catalog::build_game`], one set of C1
/// descriptors — and is kept out of [`SOURCES`] because doc 16 §9's whole rule is
/// that an extension passes *without changing* what the stock game is: the shipped
/// catalog is two weapons, one unit and one arena, and
/// `crates/tacsim/tests/definitions.rs` says so. An extension that quietly made it
/// three weapons would be a diff in a test this cut may not touch, and — more to
/// the point — would put a laser in a lobby that doc 16 §9 has not yet asked to
/// field one.
///
/// `crates/tacsim/tests/extensions.rs` loads it, pins every digest, and drives it
/// end to end.
pub const EXTENSIONS: [(&str, &str); 3] = [
    (
        "tacsim/content/ammunition/laser_charge.json",
        LASER_CHARGE_JSON,
    ),
    ("tacsim/content/weapons/laser_mk1.json", LASER_MK1_JSON),
    (
        "tacsim/content/units/line_tank_laser.json",
        LINE_TANK_LASER_JSON,
    ),
];

/// `crates/tacsim/content/units/biped_mk1.json` (16b cut X.2).
pub const BIPED_MK1_JSON: &str = include_str!("../../content/units/biped_mk1.json");

/// The biped doc 16 §9 item 2 asks for.
pub const BIPED_MK1: &str = "tacsim.unit.biped_mk1";

/// **Doc 16 §9 item 2's content** (16b cut X.2): the biped, which mounts the
/// shipped cannon, coaxial and sight and so requires no other extension row.
///
/// A table of its **own**, beside [`EXTENSIONS`] rather than inside it, and the
/// reason is the rule of cut X itself: X.1's acceptance asserts that the catalog
/// built from the laser rows is the stock game *plus the laser* — three weapons
/// and two unit schemas, exactly — and that test is one this cut may not touch.
/// A biped in the same table would make it three schemas and fail an assertion
/// about lasers that has nothing to do with bipeds. One table per item keeps each
/// acceptance a statement about its own content; the route, the loader, the
/// validation and the C1 descriptors are the one set X.1 established, through
/// [`extension_sources_of`].
pub const EXTENSIONS_BIPED: [(&str, &str); 1] =
    [("tacsim/content/units/biped_mk1.json", BIPED_MK1_JSON)];

/// `crates/tacsim/content/sensors/thermal_mk1.json` (16b cut X.3).
pub const THERMAL_MK1_JSON: &str = include_str!("../../content/sensors/thermal_mk1.json");
/// `crates/tacsim/content/units/line_tank_thermal.json` (16b cut X.3).
pub const LINE_TANK_THERMAL_JSON: &str = include_str!("../../content/units/line_tank_thermal.json");

/// The thermal sight doc 16 §9 item 3 asks for.
pub const THERMAL_MK1: &str = "tacsim.sensor.thermal_mk1";

/// The unit that mounts it.
pub const LINE_TANK_THERMAL: &str = "tacsim.unit.line_tank_thermal";

/// **Doc 16 §9 item 3's content, in `requires` order** (16b cut X.3): the thermal
/// sight and the tank that mounts it.
///
/// A table of its **own** beside [`EXTENSIONS`] and [`EXTENSIONS_BIPED`], for the
/// reason that constant gives: each item's acceptance states the whole of what its
/// own content adds to the catalog — here one sensor and one unit schema, and not
/// one weapon — and no later item may move an earlier one's count.
pub const EXTENSIONS_THERMAL: [(&str, &str); 2] = [
    ("tacsim/content/sensors/thermal_mk1.json", THERMAL_MK1_JSON),
    (
        "tacsim/content/units/line_tank_thermal.json",
        LINE_TANK_THERMAL_JSON,
    ),
];

/// `crates/tacsim/content/sensors/ecm_mk1.json` (16b cut X.4).
pub const ECM_MK1_JSON: &str = include_str!("../../content/sensors/ecm_mk1.json");
/// `crates/tacsim/content/units/line_tank_ecm.json` (16b cut X.4).
pub const LINE_TANK_ECM_JSON: &str = include_str!("../../content/units/line_tank_ecm.json");

/// The jamming pod doc 16 §9 item 4 asks for — a `sensor` row whose `jamming`
/// block is non-null, which is the item's own wording.
pub const ECM_MK1: &str = "tacsim.sensor.ecm_mk1";

/// The unit that carries it.
pub const LINE_TANK_ECM: &str = "tacsim.unit.line_tank_ecm";

/// **Doc 16 §9 item 4's content, in `requires` order** (16b cut X.4): the jamming
/// pod and the tank that carries it.
///
/// A table of its **own** beside the three before it, for the reason
/// [`EXTENSIONS_BIPED`] gives: each item's acceptance states the whole of what its
/// own content adds to the catalog — here one sensor row and one unit schema, and
/// no weapon — and no later item may move an earlier one's count.
pub const EXTENSIONS_ECM: [(&str, &str); 2] = [
    ("tacsim/content/sensors/ecm_mk1.json", ECM_MK1_JSON),
    (
        "tacsim/content/units/line_tank_ecm.json",
        LINE_TANK_ECM_JSON,
    ),
];

/// `crates/tacsim/content/sensors/eccm_mk1.json` (16b cut X.5).
pub const ECCM_MK1_JSON: &str = include_str!("../../content/sensors/eccm_mk1.json");
/// `crates/tacsim/content/units/line_tank_eccm.json` (16b cut X.5).
pub const LINE_TANK_ECCM_JSON: &str = include_str!("../../content/units/line_tank_eccm.json");

/// The counter-jamming filter doc 16 §9 item 5 asks for — a `sensor` row whose
/// `emission` block is non-null, which is the hook doc 16 §3.6 names.
pub const ECCM_MK1: &str = "tacsim.sensor.eccm_mk1";

/// The unit that carries it.
pub const LINE_TANK_ECCM: &str = "tacsim.unit.line_tank_eccm";

/// **Doc 16 §9 item 5's content, in `requires` order** (16b cut X.5): the filter and
/// the tank that carries it.
///
/// A table of its **own** beside the four before it, for the reason
/// [`EXTENSIONS_BIPED`] gives: each item's acceptance states the whole of what its
/// own content adds to the catalog — here one sensor row and one unit schema, and no
/// weapon and no pod — and no later item may move an earlier one's count.
pub const EXTENSIONS_ECCM: [(&str, &str); 2] = [
    ("tacsim/content/sensors/eccm_mk1.json", ECCM_MK1_JSON),
    (
        "tacsim/content/units/line_tank_eccm.json",
        LINE_TANK_ECCM_JSON,
    ),
];

/// `crates/tacsim/content/units/target_mk1.json` (16b cut O.3).
pub const TARGET_MK1_JSON: &str = include_str!("../../content/units/target_mk1.json");

/// **The range's target** (16b cut O.3; 16c N13): the stock hull with no weapon,
/// no sensor and no speed — what `SIM RANGE <n>` fields on the target side's
/// pods 1 to `n`.
pub const TARGET_MK1: &str = "tacsim.unit.target_mk1";

/// **The range's content** (16b cut O.3; 16c N13): one unit schema, which mounts
/// no weapon and no sensor and so requires no other pack at all.
///
/// A table of its **own** beside doc 16 §9's five, for the reason
/// [`EXTENSIONS_BIPED`] gives: each table's acceptance states the whole of what
/// its own content adds to the catalog — here one unit schema, and no weapon, no
/// ammunition and no sensor row — and no later table may move an earlier one's
/// count. It is not a §9 item; it is the fielded catalog's sixth table because
/// the range needs something on the pods to shoot at that does not shoot back,
/// and the fielded catalog is the one route a pod's tank can be refitted from.
pub const EXTENSIONS_TARGET: [(&str, &str); 1] =
    [("tacsim/content/units/target_mk1.json", TARGET_MK1_JSON)];

/// `crates/tacsim/content/ammunition/ap_30.json` (16b cut AE.2).
pub const AP_30_JSON: &str = include_str!("../../content/ammunition/ap_30.json");
/// `crates/tacsim/content/weapons/autocannon_30.json` (16b cut AE.2).
pub const AUTOCANNON_30_JSON: &str = include_str!("../../content/weapons/autocannon_30.json");
/// `crates/tacsim/content/ammunition/missile_round.json` (16b cut AE.2).
pub const MISSILE_ROUND_JSON: &str = include_str!("../../content/ammunition/missile_round.json");
/// `crates/tacsim/content/weapons/missile_lr.json` (16b cut AE.2).
pub const MISSILE_LR_JSON: &str = include_str!("../../content/weapons/missile_lr.json");

/// **The 30 mm autocannon** (16b cut AE.2; 16c N15): three rounds a pull every
/// three seconds — the cannon's damage a second in bites of 20, each round its
/// own draw under `burst:<seq>`. The scout car's first weapon from its version
/// 2, so its bare `FIRE`.
pub const AUTOCANNON_30: &str = "tacsim.weapon.autocannon_30";

/// **The missile rack** (16b cut AE.2; 16c N15): six rounds, one every ten
/// seconds, at 250 m/s — a round with a flight time a moving target walks out
/// from under. Doc 16 §3.5's scheduled impact is the whole of its mechanism: a
/// longer schedule and a due event, and nothing new on the wire. The heavy's
/// third weapon from its version 2, named `FIRE missiles`.
pub const MISSILE_LR: &str = "tacsim.weapon.missile_lr";

/// **The two weapons of 16b cut AE.2 and their ammunition, in `requires`
/// order** (16c N15): the 30 mm AP round, the autocannon that fires it, the
/// missile round and the rack that fires it — two weapons and two ammunition
/// families, and no unit schema.
///
/// A table of its **own**, for the reason [`EXTENSIONS_BIPED`] gives: its
/// acceptance states the whole of what it adds and no earlier table's count
/// moves. It is fielded **before** [`EXTENSIONS_CHASSIS`], because the scout
/// car and the heavy tank require the autocannon and the rack from their
/// version 2 on, and after [`EXTENSIONS_TARGET`], where every earlier table
/// already is.
pub const EXTENSIONS_ORDNANCE: [(&str, &str); 4] = [
    ("tacsim/content/ammunition/ap_30.json", AP_30_JSON),
    (
        "tacsim/content/weapons/autocannon_30.json",
        AUTOCANNON_30_JSON,
    ),
    (
        "tacsim/content/ammunition/missile_round.json",
        MISSILE_ROUND_JSON,
    ),
    ("tacsim/content/weapons/missile_lr.json", MISSILE_LR_JSON),
];

/// `crates/tacsim/content/units/scout_mk1.json` (16b cut AE.1).
pub const SCOUT_MK1_JSON: &str = include_str!("../../content/units/scout_mk1.json");

/// **The scout car** (16b cut AE.1; 16c N15): the fielded catalog's one
/// `wheeled` unit — thin, quick, and stopped by ground a tank crosses. From its
/// version 2 (16b cut AE.2) its first weapon is [`AUTOCANNON_30`] and the
/// coaxial is slaved to it.
pub const SCOUT_MK1: &str = "tacsim.unit.scout_mk1";

/// `crates/tacsim/content/units/heavy_mk1.json` (16b cut AE.1).
pub const HEAVY_MK1_JSON: &str = include_str!("../../content/units/heavy_mk1.json");

/// **The heavy tank** (16b cut AE.1; 16c N15): the stock tank's five sections
/// at 1.75 times the armour and two thirds the speed. From its version 2 (16b
/// cut AE.2) it carries [`MISSILE_LR`] beside the cannon, which stays its bare
/// `FIRE`.
pub const HEAVY_MK1: &str = "tacsim.unit.heavy_mk1";

/// `crates/tacsim/content/units/biped_mk2.json` (16b cut AE.1).
pub const BIPED_MK2_JSON: &str = include_str!("../../content/units/biped_mk2.json");

/// **The second biped** (16b cut AE.1; 16c N15): the laser in an arm, the
/// thermal sight in the head, the heat of the laser tank.
pub const BIPED_MK2: &str = "tacsim.unit.biped_mk2";

/// **The three chassis of 16b cut AE.1** (16c N15): the scout car, the heavy
/// tank and the second biped, in that order — three unit schemas, and no
/// weapon, ammunition or sensor row of their own.
///
/// A table of its **own**, for the reason [`EXTENSIONS_BIPED`] gives: its
/// acceptance states the whole of what it adds (three unit schemas) and no
/// earlier table's count moves. It is fielded **after** [`EXTENSIONS_TARGET`]
/// because the Mk2 biped requires the laser of [`EXTENSIONS`] and the thermal
/// sight of [`EXTENSIONS_THERMAL`], so it has to follow both; and, since **16b
/// cut AE.2** moved the scout and the heavy to version 2, after
/// [`EXTENSIONS_ORDNANCE`] too, whose autocannon and rack they carry.
pub const EXTENSIONS_CHASSIS: [(&str, &str); 3] = [
    ("tacsim/content/units/scout_mk1.json", SCOUT_MK1_JSON),
    ("tacsim/content/units/heavy_mk1.json", HEAVY_MK1_JSON),
    ("tacsim/content/units/biped_mk2.json", BIPED_MK2_JSON),
];

/// `crates/tacsim/content/units/drone_mk1.json` (16b cut AE.3).
pub const DRONE_MK1_JSON: &str = include_str!("../../content/units/drone_mk1.json");

/// **The ground drone** (16b cut AE.3; 16c N15; doc 16 §2 W2): a small tracked
/// sensor platform on a 250 cm mast, one `body` section, **no gun** — what it
/// sees its side sees through the mesh, and a laser kills it. The one unit
/// that declares `"class": "drone"`, so `[tacsim.arena].max_units_by_class`
/// caps it at one a side by default, and the end rules read it as the machine
/// that decides nothing (`Arena::eliminated`, `Arena::hold_presence`).
pub const DRONE_MK1: &str = "tacsim.unit.drone_mk1";

/// **The drone of 16b cut AE.3**, one unit schema and no weapon, ammunition or
/// sensor row of its own.
///
/// A table of its **own**, for the reason [`EXTENSIONS_BIPED`] gives: its
/// acceptance states the whole of what it adds (one unit schema) and no earlier
/// table's count moves. It requires only the stock sight, so it could go
/// anywhere after [`SOURCES`]; it is fielded **last**, after
/// [`EXTENSIONS_CHASSIS`], so every earlier table keeps its place.
pub const EXTENSIONS_DRONE: [(&str, &str); 1] =
    [("tacsim/content/units/drone_mk1.json", DRONE_MK1_JSON)];

/// `crates/tacsim/content/units/dray.json` (doc 18 cut IC2).
pub const DRAY_JSON: &str = include_str!("../../content/units/dray.json");

/// The Dray's pack id.
pub const DRAY_PACK: &str = "tacsim.pack.dray";

/// **The Dray** (doc 18 cut IC2; doc 18 §4.2, D-18-29): the Stickney-built bulk
/// hauler a First Captain charters — 240 t dry, a 100 t / 120 m³ hold, an
/// 80 t tank, a 120 crew-day locker, two berths, a `coast_hauler` drive — class
/// `hauler`, so never fielded in an arena.
pub const DRAY: &str = "tacsim.unit.dray";

/// **The trade run's hull, one unit schema and no weapon, ammunition or sensor
/// row of its own** (doc 18 cut IC2; doc 18 §4.2, D-18-29).
///
/// A table of its **own**, for the reason [`EXTENSIONS_BIPED`] gives: its
/// acceptance states the whole of what it adds (one unit schema, class
/// `hauler`, carrying `logistics` and `interior`) and no earlier table's count
/// moves. It requires nothing, so it could go anywhere; it is fielded **last**,
/// after [`EXTENSIONS_DRONE`], so every earlier table keeps its place. It is
/// fielded into the catalog (the drive table, the port code and the hull
/// template read it there) and **never fielded in an arena**:
/// `crate::definitions::UnitClass::fieldable` is false for a hauler, so the
/// lobby never offers one, `Arena::new` refuses one and the class-cap config
/// refuses the word.
pub const EXTENSIONS_HAULER: [(&str, &str); 1] = [("tacsim/content/units/dray.json", DRAY_JSON)];

// ---------------------------------------------------------------------------
// The trade run's lane (doc 18 §4.4, §4.8; cut IC2)
// ---------------------------------------------------------------------------

/// **The lane pack's id** (doc 18 cut IC2; doc 18 §4.8, D-18-48).
///
/// **A published version is never mutated** (the store's rule,
/// `boot_reconcile`): IC2's v1 was published to every enabled world that booted
/// it, so cut IC3 publishes **v2** (the routes re-timed for the Dray's re-fitted
/// 226 km/s cruise, and the two `port` entries) and cut IC8 publishes v3 (the
/// vendors: v2's ten non-port entries and both ports' fields byte-for-byte, plus
/// each port's `vendors` array), and doc 19 cut A.1 publishes v4 (the astronomy
/// engine's sky: eleven real bodies and Kirkwood with semi-annual element tables
/// from JPL Horizons, 2340–2400; the two routes re-timed to the three-dimensional
/// chords at the epoch), and doc 19 cut A.4 publishes v5 (the two ports' `premium`
/// blocks). A world that latched v1, v2, v3 or v4 keeps that ledger
/// row and takes the shipped version by
/// `@atlas/pack plan tacsim.lane.ceres_phobos` / `@atlas/pack apply` — the boot
/// pass applies only never-latched packs (`mush-content/src/module.rs`). Address
/// lane symbols by ([`LANE_PACK`], version), never by bare id:
/// `TacsimStore::resolve` picks a bare id across loaded packs, and the lane
/// shares `sol`, `ceres`, `mars` and `ceres_freeport` with the corridor
/// fixtures. Only [`LANE_VERSION`] is a game source; v1's and v2's bytes are no longer
/// compiled in.
pub const LANE_PACK: &str = "tacsim.lane.ceres_phobos";
/// **The lane version this server ships** (doc 18 cut IC3; v3 since cut IC8; v4
/// since doc 19 cut A.1: the twelve bodies with element tables, the sky in three
/// dimensions, the routes re-timed to the epoch's 3D chords; **v5 since doc 19 cut
/// A.4**: both ports' `premium` blocks, the reach premium on demand-side bids).
/// The port readiness report (`crate::port::readiness`) reads the latched version
/// against it, and every test that names the lane's version reads it here.
pub const LANE_VERSION: u32 = 5;
/// `crates/tacsim/content/lane/ceres_phobos.json`.
pub const LANE_JSON: &str = include_str!("../../content/lane/ceres_phobos.json");
/// The lane as one source, in apply order.
pub const LANE_SOURCES: [(&str, &str); 1] = [("tacsim/content/lane/ceres_phobos.json", LANE_JSON)];
/// The outbound route: Ceres Freeport by Kirkwood Depot to Stickney Yards.
pub const LANE_ROUTE_OUT: &str = "ceres_stickney";
/// The return route: Stickney Yards by Kirkwood Depot to Ceres Freeport.
pub const LANE_ROUTE_BACK: &str = "stickney_ceres";

/// The lane pack, loaded and digest-verified (doc 18 cut IC2). The production
/// boot list appends it after [`station_packs`] when `[tacsim].enabled`, and the
/// atlas shelves it after them.
///
/// # Errors
///
/// A message naming the file when the shipped pack does not verify — a build
/// error, since `crates/tacsim/tests/lane.rs` pins its digest.
pub fn lane_packs() -> Result<Vec<mush_content::Pack>, String> {
    LANE_SOURCES
        .iter()
        .map(|(name, json)| {
            mush_content::load_pack(json).map_err(|error| format!("{name}: {error}"))
        })
        .collect()
}

/// **The game's own seed sources** (doc 18 cut IC2; lead ruling R2): the lane,
/// named `<built-in …>`, which [`crate::verify_sources_with`] reads ahead of the
/// operator's files when the game content is on, so the store loads the lane
/// whose places the content latch applies and its routes materialize.
#[must_use]
pub fn game_seed_sources() -> Vec<PackSource> {
    LANE_SOURCES
        .iter()
        .map(|(name, bytes)| PackSource {
            name: format!("<built-in {name}>"),
            bytes: (*bytes).to_string(),
        })
        .collect()
}

/// The extension packs as sources [`crate::verify_sources`] reads exactly as it
/// reads an operator's.
#[must_use]
pub fn extension_sources() -> Vec<PackSource> {
    extension_sources_of(&EXTENSIONS)
}

/// **One item's extension rows as operator sources** (16b cut X.2), the shape
/// every §9 item's acceptance loads its own content through.
#[must_use]
pub fn extension_sources_of(rows: &[(&str, &str)]) -> Vec<PackSource> {
    rows.iter()
        .map(|(name, bytes)| PackSource {
            name: format!("<extension {name}>"),
            bytes: (*bytes).to_string(),
        })
        .collect()
}

/// Every TacSim pack this crate ships, in `requires` order — a pack after every
/// pack it names.
///
/// **17b cut M3.1** puts the cores pack ([`CORES`]) ninth, before
/// `line_tank.json`, which requires it from its version 3 — and every chassis
/// of the extension tables after it does too, which this table being ahead of
/// all of them satisfies.
pub const SOURCES: [(&str, &str); 9] = [
    ("tacsim/content/ammunition/he_105.json", HE_105_JSON),
    ("tacsim/content/ammunition/ball_762.json", BALL_762_JSON),
    ("tacsim/content/weapons/cannon_105.json", CANNON_105_JSON),
    ("tacsim/content/weapons/mg_coax.json", MG_COAX_JSON),
    ("tacsim/content/sensors/visual_mk1.json", VISUAL_MK1_JSON),
    ("tacsim/content/cores/cores.json", CORES_JSON),
    ("tacsim/content/units/line_tank.json", LINE_TANK_JSON),
    ("tacsim/content/resolution.json", RESOLUTION_JSON),
    (
        "tacsim/content/arenas/proving_ground.json",
        PROVING_GROUND_JSON,
    ),
];

/// The shipped packs as sources [`crate::verify_sources`] reads exactly as it
/// reads an operator's.
#[must_use]
pub fn sources() -> Vec<PackSource> {
    SOURCES
        .iter()
        .map(|(name, bytes)| PackSource {
            name: format!("<built-in {name}>"),
            bytes: (*bytes).to_string(),
        })
        .collect()
}

/// **Every pack the fielded game loads** (16b cut Q.8): [`SOURCES`], then doc 16
/// §9's five extension items, then the range's target (16b cut O.3), in
/// `requires` order — a pack after every pack it names.
///
/// The six tables carry no dependency on one another: each armed unit requires
/// the stock cannon, coaxial and sight (plus, for the laser tank, the two laser
/// packs of its own table), the target requires nothing, and every sensor and
/// ammunition row requires nothing. So the order below is `EXTENSIONS`,
/// `EXTENSIONS_BIPED`, `EXTENSIONS_THERMAL`, `EXTENSIONS_ECM`, `EXTENSIONS_ECCM`,
/// which is the order doc 16 §9 numbers its items in, and then
/// `EXTENSIONS_TARGET`; `crates/tacsim/tests/content.rs` pins that every named
/// requirement is satisfied by a pack ahead of it.
///
/// **16b cut AE.1** appends a seventh, [`EXTENSIONS_CHASSIS`], after the
/// target: its Mk2 biped requires the laser and the thermal sight, which are
/// ahead of it here.
///
/// **16b cut AE.2** inserts an eighth, [`EXTENSIONS_ORDNANCE`], between the
/// target and the chassis: the scout car's version 2 requires the autocannon
/// and the heavy's the missile rack, so both weapons (and the two rounds they
/// fire, ahead of them in their own table) come first.
///
/// **16b cut AE.3** appends a ninth, [`EXTENSIONS_DRONE`], last: it requires
/// only the stock sight, and last moves no earlier table.
///
/// **Doc 18 cut IC2** appends a tenth, [`EXTENSIONS_HAULER`], last: the Dray
/// requires nothing, and last moves no earlier table. It is in the catalog and
/// never in an arena (class `hauler`; doc 18 §4.2, D-18-29).
///
/// Named `<built-in …>` like [`sources`], and not `<extension …>` like
/// [`extension_sources_of`]: through this route the ten are shipped content
/// and not an operator's file, and the name is what a refusal quotes.
#[must_use]
pub fn fielded_sources() -> Vec<PackSource> {
    let mut out = sources();
    for table in [
        EXTENSIONS.as_slice(),
        EXTENSIONS_BIPED.as_slice(),
        EXTENSIONS_THERMAL.as_slice(),
        EXTENSIONS_ECM.as_slice(),
        EXTENSIONS_ECCM.as_slice(),
        EXTENSIONS_TARGET.as_slice(),
        EXTENSIONS_ORDNANCE.as_slice(),
        EXTENSIONS_CHASSIS.as_slice(),
        EXTENSIONS_DRONE.as_slice(),
        EXTENSIONS_HAULER.as_slice(),
    ] {
        out.extend(table.iter().map(|(name, bytes)| PackSource {
            name: format!("<built-in {name}>"),
            bytes: (*bytes).to_string(),
        }));
    }
    out
}

/// The two packs the content latch applies at a first boot, in order: the
/// arena's ground, then the arena.
///
/// # Errors
///
/// A message naming the file when a shipped pack does not verify — a build
/// error, since `crates/tacsim/tests/content.rs` pins both digests.
pub fn boot_packs() -> Result<Vec<mush_content::Pack>, String> {
    [
        (
            "tacsim/content/atlas/proving_ground.json",
            PROVING_GROUND_ATLAS_JSON,
        ),
        (
            "tacsim/content/arenas/proving_ground.json",
            PROVING_GROUND_JSON,
        ),
    ]
    .iter()
    .map(|(name, json)| mush_content::load_pack(json).map_err(|error| format!("{name}: {error}")))
    .collect()
}

/// The station packs, loaded and digest-verified, in apply order
/// ([`STATION_SOURCES`]). They are the 2350 game's content: the production boot
/// list appends them after [`boot_packs`] when `[tacsim].enabled` (doc 18 §4.7),
/// and the atlas shelves them always. [`boot_packs`] itself stays the proving
/// ground's two, because the arena fixtures `pop()` the arena off its end.
///
/// # Errors
///
/// A message naming the pack when a shipped pack does not verify — a build error,
/// since `crates/tacsim/tests/stations.rs` pins all four digests.
pub fn station_packs() -> Result<Vec<mush_content::Pack>, String> {
    STATION_SOURCES
        .iter()
        .map(|(name, json)| {
            mush_content::load_pack(json).map_err(|error| format!("{name}: {error}"))
        })
        .collect()
}
