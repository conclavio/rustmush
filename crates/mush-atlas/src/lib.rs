//! # mush-atlas
//!
//! The spatial records (doc 14 §2), the rasterizer (§3.2), the `+atlas` text views
//! (§6) and `@atlas/pack` (§3.1), plus the starter area as shipped content (§2.6).
//! Plan §3.1, cut S1.
//!
//! Rooms stay primary: a room is an object with geometry attached ([`RoomGeo`]), and
//! every other record — region, locality, neighborhood, site, structure, portal,
//! connection, feature, tile — is the substrate that geometry sits in. Nothing here
//! knows who is in a room or what they are doing there.

// WS01-S12: a new crate is born with every panic-shaped restriction lint denied,
// and `crates/mush-contract-tests/tests/restriction_lints.rs` holds it there.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod apply;
pub mod background;
pub mod cells;
pub mod commands;
pub mod def;
pub mod frame;
pub mod geom;
pub mod hydrate;
pub mod hydrate_cmd;
pub mod interface;
pub mod map_cmd;
pub mod module;
pub mod physics;
pub mod place;
pub mod raster;
pub mod record;
pub mod starter;
pub mod terrain;
pub mod theme;
pub mod theme_cmd;
pub mod trace;
pub mod travel;
pub mod travel_cmd;
pub mod validate;
pub mod view;
pub mod window;
pub mod window_cmd;

pub use apply::{KINDS, register_appliers};
pub use background::{Ground, background};
pub use cells::{Cell, Cells, CellsError, MAX_TILE_CELLS, MAX_TILE_DOC_BYTES, MAX_TILE_SIDE};
pub use frame::{MAP_FEED, MapFeed, delta_for, empty_frame, frame_for, surface_label};
pub use geom::{GeomError, Point2, Point3, PointCm, Polygon2, Rect2, RectCm, Shape};
pub use hydrate::{
    Activation, AtlasHooks, BUILD_HANDLER, BUILD_QUEUE, DEFAULT_HYDRATION_CAP,
    DEFAULT_STRIKES_TO_ZERO, HydrationState, Placement, ProposeFn, ProposeRequest, ProposeScope,
    ProposeSource, RequestRec, RequestStatus, Requested, StandingRec, StrikeReport,
};
pub use module::{
    ATTR_NS, AtlasConfig, AtlasLimits, AtlasModule, AtlasState, MAX_CONFIGURED_PACKS, NS,
    PackSource, is_staff, verify_sources,
};
pub use physics::{
    BASE_LOCOMOTIONS, BASE_MEDIA, BASE_TAGS, BIPED, Cost, Cover, DEFAULT_PATH_MAX_CELLS,
    Impassable, LOCOMOTION_JSON, LocomotionTable, Opacity, Path, PathOutcome, PhysicsError,
    StepKind, SurfaceProps, move_cost, path, props, slope_pct,
};
pub use place::{PlaceRef, chain_for_room, find_place};
pub use record::{
    Access, Footprint, MAX_KNOWN_PLACES, NeighborhoodRec, PortalRec, RecordError, RecordKey,
    Records, RegionRec, RoomGeo, RoomKind, SiteRec, StructureRec, SurfaceRef, TileRec,
};
pub use terrain::TerrainClass;
pub use theme::{DescLength, Theme, resolve_place_theme, resolve_theme, world_theme};
pub use trace::{
    Blocker, Crossed, EYE_HEIGHT_CM, Eye, Hearing, Sight, Trace, beam_path, direct_fire_path,
    hearing, indirect_fire_path, line_cells, line_of_sight,
};
pub use travel::{
    DEFAULT_DWELL_FLOOR_S, DEFAULT_REAL_CAP_S, End, Heading, HoldReason, JourneyId, JourneyRec,
    JourneyState, MAX_JOURNEYS, Node, RESUME_GRACE_S, TICK_TAG, TravelState, Traveler, Way,
    plan_legs,
};
pub use validate::{
    Finding, Severity, ValidateContext, ValidateScope, blocking, check_pack, validate,
};
pub use window::{
    Compass, Composite, CompositeInput, CompositeJob, CompositeOutcome, DEFAULT_CELL_SIZES,
    DEFAULT_WINDOW_MARGIN_M, DEFAULT_WINDOW_MAX_CELLS, FeatureOverlay, OpenRequest, STREET_CELL_CM,
    StepReport, StepTarget, SurfaceComposite, WindowEntry, WindowFrame, Windows, capture,
    choose_cell, composite, register_cell_size, resolve_room, run_async,
};
