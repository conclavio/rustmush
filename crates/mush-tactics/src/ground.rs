//! **S7 (doc 14 §5.4) — the TacSim ground driver.**
//!
//! Ten-metre cells (configurable, `[tacsim].ground_cell_cm`) over the same
//! tiles a street fight is fought on at one metre; the ruleset's locomotions
//! — `legged`, `tracked`, `wheeled`, `hover`, `rotor` — costed from the
//! engine's locomotion table; direct and indirect fire read from the atlas's
//! traces over the engagement's window; the pulse clock from E7 cut 8, which
//! `@domain/promote` arms on the same domain.
//!
//! The driver is a [`mush_sim::DomainDriver`]: `@domain/create <host>=tactics.ground`
//! asks it for the domain, and the domain it makes **is the window** the
//! engagement is fought on — a window is a domain of its own on the host room,
//! so there is one record, one pulse clock and one map — anchored at the host
//! room with the room's occupants placed; `@domain/retire` closes it. Nothing
//! about a hex: the window is the atlas's square grid (D-14-4).

use std::sync::Arc;

use mush_atlas::cells::Cell;
use mush_atlas::physics::{Cost, Impassable, LocomotionTable, StepKind, move_cost};
use mush_core::{Ctx, DefinitionRef, ObjId, ObjKind, World};
use smol_str::SmolStr;

/// The driver's name, as `@domain/create` takes it.
pub const GROUND_DRIVER: &str = "tactics.ground";

/// The definition a ground domain records: kind, id, version.
pub const GROUND_DEFINITION_KIND: &str = "driver";
/// The definition's id.
pub const GROUND_DEFINITION_ID: &str = "tactics.ground";
/// The definition's version.
pub const GROUND_DEFINITION_VERSION: u32 = 1;

/// The ground driver's cell size when nothing configures one, centimetres.
pub const DEFAULT_GROUND_CELL_CM: u16 = 1_000;

/// The locomotions the ground driver moves, every one an engine row of the
/// locomotion table (doc 14 §5.1).
pub const GROUND_LOCOMOTIONS: [&str; 5] = ["legged", "tracked", "wheeled", "hover", "rotor"];

/// The apex of the driver's indirect fire, centimetres (the atlas's).
pub const INDIRECT_APEX_CM: i32 = mush_atlas::DEFAULT_ARC_APEX_CM;

/// The ground driver's live configuration, a module-state slot the
/// composition root fills after composition (`[tacsim].ground_cell_cm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroundConfig {
    /// The cell size the driver opens its windows at.
    pub cell_cm: u16,
}

impl Default for GroundConfig {
    fn default() -> GroundConfig {
        GroundConfig {
            cell_cm: DEFAULT_GROUND_CELL_CM,
        }
    }
}

/// Set the driver's cell size on a composed world. The composition root's
/// post-composition device, like the pulse cadence's.
///
/// # Errors
///
/// A world without the tactics module.
pub fn configure(w: &mut World, cell_cm: u16) -> Result<(), String> {
    let slot = w
        .module_state_mut::<GroundConfig>()
        .ok_or_else(|| "the tactics module is not composed on this world".to_string())?;
    slot.cell_cm = cell_cm;
    Ok(())
}

/// The driver's cell size on a world; the default off a world without the slot.
#[must_use]
pub fn cell_cm(w: &World) -> u16 {
    w.module_state::<GroundConfig>()
        .map_or(DEFAULT_GROUND_CELL_CM, |c| c.cell_cm)
}

/// The TacSim ground driver.
pub struct GroundDriver {
    definition: DefinitionRef,
}

impl GroundDriver {
    /// A driver recording `definition` on its domains.
    #[must_use]
    pub fn new(definition: DefinitionRef) -> GroundDriver {
        GroundDriver { definition }
    }

    /// The driver's definition descriptor, for the registrar.
    ///
    /// # Errors
    ///
    /// A message when the descriptor will not build (it always builds).
    pub fn descriptor() -> Result<mush_core::DefinitionDescriptor, String> {
        let kind = mush_core::DefinitionKind::new(GROUND_DEFINITION_KIND)
            .map_err(|error| error.to_string())?;
        let id = mush_core::DefinitionId::new(GROUND_DEFINITION_ID)
            .map_err(|error| error.to_string())?;
        mush_core::DefinitionDescriptor::new(
            kind,
            id,
            GROUND_DEFINITION_VERSION,
            serde_json::json!({ "instance": "opaque_json" }),
        )
        .map_err(|error| error.to_string())
    }

    /// A standalone definition for tests and tools that compose no registrar:
    /// the same descriptor the module registers, in a registry of its own.
    ///
    /// # Errors
    ///
    /// A message when the registry refuses the descriptor.
    pub fn standalone_definition() -> Result<DefinitionRef, String> {
        let owner =
            mush_core::OwnerId::new(crate::module::NS).map_err(|error| error.to_string())?;
        mush_core::DefinitionRegistryBuilder::new()
            .register(owner, Self::descriptor()?)
            .map_err(|error| error.to_string())
    }
}

/// The room a domain host stands for: the host itself when it is a room, else
/// where it is.
fn host_room(w: &World, host: ObjId) -> Option<ObjId> {
    let obj = w.get(host)?;
    if obj.kind == ObjKind::Room {
        Some(host)
    } else {
        obj.location
    }
}

impl mush_sim::DomainDriver for GroundDriver {
    fn name(&self) -> &str {
        GROUND_DRIVER
    }

    fn definition(&self) -> DefinitionRef {
        self.definition.clone()
    }

    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        host: ObjId,
        _presentation: ObjId,
    ) -> Result<mush_sim::DomainId, String> {
        let room = host_room(ctx.w, host)
            .ok_or_else(|| "the host is nowhere the atlas knows".to_string())?;
        // Everyone and everything standing in the host room fights on the
        // window: players, NPCs (S8.5, 14a #5) and things.
        let participants: Vec<ObjId> = ctx
            .w
            .get(room)
            .map(|r| {
                r.contents
                    .iter()
                    .copied()
                    .filter(|o| {
                        ctx.w
                            .get(*o)
                            .is_some_and(|x| x.kind.is_actor() || x.kind == ObjKind::Thing)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let cell = cell_cm(ctx.w);
        // The window is the domain: it is hosted on the room and records this
        // driver, and the pulse clock arms on it like any other.
        mush_atlas::window::open(
            ctx,
            mush_atlas::OpenRequest {
                anchors: vec![room],
                bbox: None,
                opened_by: mush_sim::WindowOpener::Driver {
                    driver: SmolStr::new_static(GROUND_DRIVER),
                },
                cell_cm: Some(cell),
                participants,
                driver: Some(self.definition.clone()),
            },
        )
    }

    fn closed(&self, ctx: &mut Ctx<'_>, domain: mush_sim::DomainId) -> bool {
        // Closing the window retires its domain (the simulation's own close).
        match mush_atlas::window::close(ctx, domain, "the domain retired") {
            Ok(_) => true,
            Err(error) => {
                tracing::debug!(%domain, %error, "tactics.ground: no window to close");
                false
            }
        }
    }
}

/// Register the driver on a composed world. Once, at the module's boot.
///
/// # Errors
///
/// The simulation's refusal.
pub fn register(w: &mut World, definition: DefinitionRef) -> Result<(), String> {
    mush_sim::register_domain_driver(w, Arc::new(GroundDriver::new(definition)))
}

/// One step's cost for a ground locomotion at the driver's cell size, from the
/// engine's table: the slope is what the two cells' elevations make of the cell
/// size, so a ten-metre cell and a one-metre cell read one relief alike.
///
/// # Errors
///
/// [`Impassable`] as the table says.
pub fn ground_move_cost(
    table: &LocomotionTable,
    from: Cell,
    to: Cell,
    kind: StepKind,
    cell_cm: u16,
    locomotion: &str,
) -> Result<Cost, Impassable> {
    move_cost(table, from, to, kind, u32::from(cell_cm), locomotion, &[])
}

/// The driver's fire read over two placed objects: whether a direct shot has
/// its line, whether an indirect one clears the ground, and the range in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineOfFire {
    /// A straight line of fire exists.
    pub direct: bool,
    /// An arc at [`INDIRECT_APEX_CM`] clears the ground between.
    pub indirect: bool,
    /// The range, cells.
    pub range_cells: u32,
}

/// The line of fire between two objects on one window; `None` when they share
/// none. Read at intent time by the pulse driver's fire intents.
#[must_use]
pub fn line_of_fire(w: &World, from: ObjId, to: ObjId) -> Option<LineOfFire> {
    let metric = mush_sim::metric_between(w, from, to)?;
    let indirect = mush_sim::arc_between(w, from, to).unwrap_or(false);
    Some(LineOfFire {
        direct: !metric.occluded,
        indirect,
        range_cells: metric.range_cells,
    })
}
