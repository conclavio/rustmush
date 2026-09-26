//! **The game's content, compiled in** (16b cut C.2; doc 16 §4 "Content
//! packs").
//!
//! Nine files under `crates/tacsim/content/`, `include_str!`ed here and pinned
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
//! [`EXTENSIONS_ECCM`], the last of doc 16 §9's five.
//!
//! # And what is **fielded**
//!
//! **16b cut Q.8** answers X.1 deviation 1's open question — *whether the
//! extensions join the shipped catalog* — with **yes for the game, and no for
//! the stock game**. [`fielded_sources`] is [`sources`] followed by all five
//! extension tables in `requires` order, and [`super::Catalog::build_fielded`]
//! is what the three production sites build from, so `SIM UNIT` can offer a
//! pilot a biped and `@unit/create` can name a laser tank on a shipped server.
//!
//! [`SOURCES`] and [`sources`] are untouched, and so is
//! [`super::Catalog::build_game`]: "the stock game" is a statement about the
//! eight packs in the table above, it is what every cut-X acceptance compares
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

/// The unit schema the shipped content declares.
pub const LINE_TANK: &str = "tacsim.unit.line_tank";

/// The resolution table the shipped content declares.
pub const RESOLUTION: &str = "tacsim.resolution.stock";

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
pub const SOURCES: [(&str, &str); 8] = [
    ("tacsim/content/ammunition/he_105.json", HE_105_JSON),
    ("tacsim/content/ammunition/ball_762.json", BALL_762_JSON),
    ("tacsim/content/weapons/cannon_105.json", CANNON_105_JSON),
    ("tacsim/content/weapons/mg_coax.json", MG_COAX_JSON),
    ("tacsim/content/sensors/visual_mk1.json", VISUAL_MK1_JSON),
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
/// §9's five extension items, in `requires` order — a pack after every pack it
/// names.
///
/// The five tables carry no dependency on one another: each unit requires the
/// stock cannon, coaxial and sight (plus, for the laser tank, the two laser
/// packs of its own table), and every sensor and ammunition row requires
/// nothing. So the order below is `EXTENSIONS`, `EXTENSIONS_BIPED`,
/// `EXTENSIONS_THERMAL`, `EXTENSIONS_ECM`, `EXTENSIONS_ECCM`, which is the order
/// doc 16 §9 numbers its items in, and `crates/tacsim/tests/content.rs` pins
/// that every named requirement is satisfied by a pack ahead of it.
///
/// Named `<built-in …>` like [`sources`], and not `<extension …>` like
/// [`extension_sources_of`]: through this route the five are shipped content and
/// not an operator's file, and the name is what a refusal quotes.
#[must_use]
pub fn fielded_sources() -> Vec<PackSource> {
    let mut out = sources();
    for table in [
        EXTENSIONS.as_slice(),
        EXTENSIONS_BIPED.as_slice(),
        EXTENSIONS_THERMAL.as_slice(),
        EXTENSIONS_ECM.as_slice(),
        EXTENSIONS_ECCM.as_slice(),
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
