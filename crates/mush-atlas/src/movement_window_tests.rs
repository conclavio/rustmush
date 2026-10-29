#![cfg(test)]

use super::*;
use crate::cells::{Cell, Cells};
use crate::geom::{Point2, Point3, Polygon2, RectCm};
use crate::raster::Grid;
use crate::record::{Access, PortalMode, PortalRec, RoomKind, SurfaceRef, TimePolicy, Visibility};
use crate::terrain::TerrainClass;
use crate::window::{WindowEntry, WindowFrame};
use mush_core::{CanonicalId, GOD, LIMBO, ObjKind};
use mush_sim::{PlacementRecord, Pose, WindowOpener};
use serde_json::json;

struct Fixture {
    world: World,
    actor: ObjId,
    from: ObjId,
    to: ObjId,
}
fn key(name: &str) -> RecordKey {
    RecordKey::new(
        CanonicalId::new("test").expect("pack"),
        CanonicalId::new(name).expect("key"),
    )
}
fn fixture() -> Fixture {
    let mut world = World::new_minimal();
    let from = world
        .create(ObjKind::Room, "source", GOD, None)
        .expect("source");
    let to = world
        .create(ObjKind::Room, "destination", GOD, None)
        .expect("destination");
    let actor = world
        .create(ObjKind::Player, "walker", GOD, Some(from))
        .expect("actor");
    let frame = WindowFrame {
        region: key("region"),
        surfaces: vec![SurfaceRef::Ground],
        bbox: RectCm {
            min: PointCm { x: 0, y: 0 },
            max: PointCm { x: 300, y: 300 },
        },
        cell_cm: 100,
    };
    let surface = SurfaceComposite {
        surface: SurfaceRef::Ground,
        grid: Grid {
            origin: PointCm { x: 0, y: 0 },
            resolution_cm: 100,
            width: 3,
            height: 3,
        },
        cells: Cells::filled(
            3,
            3,
            Cell {
                class: TerrainClass::Paved,
                elev_cm: 0,
                cover: 0,
                room: 1,
            },
            9,
        )
        .expect("cells"),
        rooms: vec![to],
        features: vec![],
    };
    let mut state = AtlasState::default();
    state.records.roomgeo.insert(
        to,
        RoomGeo {
            room: to,
            key: key("room"),
            neighborhood: key("district"),
            site: None,
            surface: SurfaceRef::Ground,
            footprint: Footprint::Anchor {
                point: Point2(1.5, 1.5),
                radius_m: 1.0,
            },
            kind: RoomKind::Indoor,
            access: Access::Public,
            revision: 1,
        },
    );
    state.windows.open.insert(
        7,
        WindowEntry {
            domain: 7,
            frame: frame.clone(),
            anchors: vec![to],
            rooms: BTreeSet::from([to]),
            opened_by: WindowOpener::Staff { by: GOD },
            revision: 1,
            composite: Some(Composite {
                frame,
                surfaces: vec![surface],
                pins: vec![],
                features_seen: 0,
                digest: "not_authority".into(),
            }),
            pending: None,
        },
    );
    world.module_state_insert::<AtlasState>(state);
    Fixture {
        world,
        actor,
        from,
        to,
    }
}
impl Fixture {
    fn state(&mut self) -> &mut AtlasState {
        self.world
            .module_state_mut::<AtlasState>()
            .expect("actual Atlas owner")
    }
    fn entry(&mut self) -> &mut WindowEntry {
        self.state().windows.open.get_mut(&7).expect("window")
    }
    fn surface(&mut self) -> &mut SurfaceComposite {
        self.entry()
            .composite
            .as_mut()
            .expect("drawn")
            .surfaces
            .first_mut()
            .expect("surface")
    }
    fn plan(&self) -> WindowArrivalPlan {
        plan(&self.world, self.actor, Some(self.from), self.to, 1234).expect("plan")
    }
    fn place(&mut self, records: Vec<PlacementRecord>) {
        self.world
            .module_state_insert::<SpatialState>(SpatialState {
                placements: records.into_iter().map(|p| (p.object, p)).collect(),
                ..SpatialState::default()
            });
    }
}
fn placed(object: ObjId, domain: u64, x: i64, y: i64, z: i32) -> PlacementRecord {
    PlacementRecord {
        object,
        parent: SpatialParent::Direct {
            domain: DomainId(domain),
            pose: Pose::at(
                x,
                y,
                z,
                mush_sim::passive_rules_value(json!({})).expect("passive posture"),
            ),
        },
        revision: 3,
        unplaced: None,
    }
}
fn selected(plan: &WindowArrivalPlan) -> (usize, u16, u16, Option<ObjId>) {
    let EntryOutcome::Selected {
        surface,
        x,
        y,
        projected_room,
        ..
    } = plan.placement().expect("attempt").outcome()
    else {
        panic!("selected cell")
    };
    (*surface, *x, *y, *projected_room)
}
fn portal(name: &str, from: ObjId, to: ObjId, point: Point3) -> PortalRec {
    PortalRec {
        key: key(name),
        exits: vec![],
        from: Endpoint::Room(from),
        to: Endpoint::Room(to),
        geometry: PortalGeometry::Point(point),
        mode: PortalMode::Door,
        distance_m: 0,
        dz_m: 0.0,
        time: TimePolicy::Instant,
        visibility: Visibility::Hidden,
        revision: 1,
    }
}

#[test]
fn shares_portal_cell_and_projection_selectors_without_world_mutation() {
    let mut f = fixture();
    let p = portal("door", f.to, f.from, Point3(0.0, 1.0, 900.0));
    f.state().records.portals.insert(p.key.clone(), p);
    let _ = f.world.take_dirty();
    let captured = f.plan();
    assert_eq!(selected(&captured), (0, 0, 1, Some(f.to)));
    assert_eq!(
        captured.placement().expect("attempt").portal().selected(),
        Some(&key("door"))
    );
    let state = f.world.module_state::<AtlasState>().expect("owner");
    let composite = state
        .windows
        .open
        .get(&7)
        .and_then(|e| e.composite.as_ref())
        .expect("composite");
    let selected_portal = window::portal_between(&state.records, Some(f.from), f.to);
    assert_eq!(
        window::entry_cell(
            &f.world,
            DomainId(7),
            composite,
            &state.records,
            f.to,
            selected_portal.as_ref()
        ),
        Some((0, 0, 1))
    );
    let pose = window::pose_at(composite.surfaces.first().expect("surface"), 0, 1).expect("pose");
    assert_eq!(
        window::resolve_room(&f.world, DomainId(7), &pose),
        Some(f.to)
    );
    assert_eq!(pose.heading_mdeg, 0);
    assert!(state.travel.known.is_empty());
    assert!(f.world.take_dirty().is_empty());
    assert_eq!(f.world.get(f.actor).expect("actor").location, Some(f.from));
    captured.recheck(&f.world).expect("stable observations");
    assert!(captured.encode().expect("bounded encoding").len() < MAX_WINDOW_PLAN_BYTES);
}

#[test]
fn own_cell_and_other_height_occupancy_preserve_ties_and_arena_exclusion() {
    let mut f = fixture();
    let other = ObjId(999);
    f.place(vec![
        placed(f.actor, 7, 150, 150, 9999),
        placed(other, 7, 150, 50, -9999),
    ]);
    let before = f.plan();
    assert_eq!(
        selected(&before),
        (0, 0, 1, Some(f.to)),
        "own cell and a different floor still block xy; ties use y then x"
    );
    f.world
        .module_state_insert::<mush_sim::deployed::DeployedUnits>(
            mush_sim::deployed::DeployedUnits::new(),
        );
    mush_sim::deployed::mark(
        &mut f.world,
        other,
        mush_sim::deployed::DeployedMark::new(1, "arena"),
    );
    let after = f.plan();
    assert_eq!(selected(&after), (0, 1, 0, Some(f.to)));
    assert_eq!(before.recheck(&f.world), Err(CaptureError::Invalid));
    let state = f.world.module_state::<AtlasState>().expect("owner");
    let composite = state
        .windows
        .open
        .get(&7)
        .and_then(|e| e.composite.as_ref())
        .expect("drawn");
    assert_eq!(
        window::entry_cell(&f.world, DomainId(7), composite, &state.records, f.to, None),
        Some((0, 1, 0))
    );
}

#[test]
fn release_is_retained_when_target_has_no_free_cell_and_is_never_executed() {
    let mut f = fixture();
    f.place(vec![placed(f.actor, 8, 50, 50, 0)]);
    f.surface().cells = Cells::filled(
        3,
        3,
        Cell {
            class: TerrainClass::Void,
            ..Cell::default()
        },
        9,
    )
    .expect("void cells");
    let captured = f.plan();
    assert_eq!(captured.release(), Some(DomainId(8)));
    assert_eq!(
        captured.placement().expect("attempt").outcome(),
        &EntryOutcome::NoFreeCell
    );
    assert_eq!(
        mush_sim::pose_of(&f.world, f.actor).map(|(d, _)| d),
        Some(DomainId(8))
    );
    assert_eq!(f.world.get(f.actor).expect("actor").location, Some(f.from));
}

#[test]
fn first_covering_undrawn_window_masks_later_drawn_and_missing_slots_stay_visible() {
    let mut f = fixture();
    f.place(vec![placed(f.actor, 8, 50, 50, 0)]);
    let mut earlier = f.entry().clone();
    earlier.domain = 1;
    earlier.composite = None;
    f.state().windows.open.insert(1, earlier);
    let captured = f.plan();
    assert!(captured.placement().is_none());
    assert_eq!(captured.release(), Some(DomainId(8)));
    let mut absent = World::new_minimal();
    absent.module_state_insert::<SpatialState>(SpatialState {
        placements: [(GOD, placed(GOD, 8, 50, 50, 0))].into(),
        ..SpatialState::default()
    });
    let captured = plan(&absent, GOD, Some(LIMBO), LIMBO, 1).expect("absent Atlas");
    assert_eq!(
        captured.capture().atlas_slot(),
        crate::movement::SlotPresence::Absent
    );
    assert!(captured.placement().is_none());
    assert_eq!(captured.release(), Some(DomainId(8)));
}

#[test]
fn selected_projection_can_be_another_overlapping_surface_room() {
    let mut f = fixture();
    let floor = SurfaceRef::Floor {
        structure: key("building"),
        index: 1,
    };
    let to = f.to;
    let from = f.from;
    let mut upper = f.surface().clone();
    upper.surface = floor.clone();
    f.surface().rooms = vec![from];
    f.entry()
        .composite
        .as_mut()
        .expect("drawn")
        .surfaces
        .push(upper);
    f.state()
        .records
        .roomgeo
        .get_mut(&to)
        .expect("geometry")
        .surface = floor;
    assert_eq!(
        selected(&f.plan()),
        (1, 1, 1, Some(from)),
        "projection uses the actual first nearest-height surface, not an assumed destination"
    );
}

#[test]
fn selected_facts_are_rechecked_without_trusting_revision_or_cached_digest() {
    let mut f = fixture();
    let captured = f.plan();
    let mut cell = f.surface().cells.get(0, 0).expect("cell");
    cell.elev_cm = 8;
    assert!(f.surface().cells.set(0, 0, cell));
    assert_eq!(captured.recheck(&f.world), Err(CaptureError::Invalid));
    assert_eq!(f.entry().revision, 1);
    assert_eq!(
        f.entry().composite.as_ref().expect("drawn").digest,
        "not_authority"
    );
    let captured = f.plan();
    f.entry().composite.as_mut().expect("drawn").digest = "changed unused cache".into();
    captured.recheck(&f.world).expect("cache not consulted");
    let to = f.to;
    f.state()
        .records
        .roomgeo
        .get_mut(&to)
        .expect("geometry")
        .footprint = Footprint::Anchor {
        point: Point2(1.5, 1.5),
        radius_m: 2.0,
    };
    assert_eq!(captured.recheck(&f.world), Err(CaptureError::Invalid));
}

#[test]
fn original_portal_scan_and_second_lookup_are_preserved_including_negative_facts() {
    let mut f = fixture();
    let mut first = portal("lookup", f.from, f.to, Point3(2.0, 2.0, 0.0));
    first.revision = 10;
    f.state().records.portals.insert(key("a"), first);
    let second = portal("lookup", ObjId(777), f.to, Point3(0.0, 1.0, 0.0));
    f.state().records.portals.insert(key("lookup"), second);
    let captured = f.plan();
    assert_eq!(
        selected(&captured),
        (0, 0, 1, Some(f.to)),
        "the record key is looked up again, rather than using the selected row geometry"
    );
    let negative = portal("zero", ObjId(888), f.to, Point3(0.0, 0.0, 0.0));
    f.state().records.portals.insert(key("0"), negative);
    assert_eq!(captured.recheck(&f.world), Err(CaptureError::Invalid));
    let captured = f.plan();
    f.state()
        .records
        .portals
        .get_mut(&key("lookup"))
        .expect("lookup")
        .geometry = PortalGeometry::Point(Point3(f64::NAN, 0.0, 0.0));
    let fallback = f.plan();
    assert_eq!(selected(&fallback), (0, 1, 1, Some(f.to)));
    assert_eq!(captured.recheck(&f.world), Err(CaptureError::Invalid));
    f.state()
        .records
        .portals
        .get_mut(&key("lookup"))
        .expect("lookup")
        .geometry = PortalGeometry::Point(Point3(f64::INFINITY, 0.0, 0.0));
    assert_eq!(
        fallback.recheck(&f.world),
        Err(CaptureError::Invalid),
        "nonfinite input bits do not collapse into JSON null"
    );
}

#[test]
fn absent_geometry_and_inconsistent_window_keys_remain_distinct_attempts() {
    let mut f = fixture();
    let to = f.to;
    let geo = f.state().records.roomgeo.remove(&to).expect("room");
    assert_eq!(
        f.plan().placement().expect("attempt").outcome(),
        &EntryOutcome::NoRoomGeometry
    );
    f.state().records.roomgeo.insert(to, geo);
    f.state()
        .records
        .roomgeo
        .get_mut(&to)
        .expect("geometry")
        .surface = SurfaceRef::Floor {
        structure: key("absent"),
        index: 1,
    };
    assert_eq!(
        f.plan().placement().expect("attempt").outcome(),
        &EntryOutcome::NoSurface
    );
    f.state()
        .records
        .roomgeo
        .get_mut(&to)
        .expect("geometry")
        .surface = SurfaceRef::Ground;
    f.surface().rooms.clear();
    assert_eq!(
        f.plan().placement().expect("attempt").outcome(),
        &EntryOutcome::NoRoomIndex
    );
    f.entry().domain = 99;
    let captured = f.plan();
    assert_eq!(
        captured.placement().expect("attempt").domain(),
        DomainId(99)
    );
    assert_eq!(
        captured.placement().expect("attempt").outcome(),
        &EntryOutcome::MissingWindow
    );
    let mut undrawn = f.entry().clone();
    undrawn.rooms.clear();
    undrawn.composite = None;
    f.state().windows.open.insert(99, undrawn);
    assert_eq!(
        f.plan().placement().expect("attempt").outcome(),
        &EntryOutcome::Undrawn
    );
}

#[test]
fn bounds_precede_geometry_copies_and_occupancy_scan_and_portal_scan_is_a_prefix() {
    let mut f = fixture();
    f.surface().grid.resolution_cm = 0;
    assert_eq!(
        plan(&f.world, f.actor, None, f.to, 0),
        Err(CaptureError::Invalid)
    );
    f.surface().grid.resolution_cm = 100;
    let to = f.to;
    f.surface().rooms = vec![to; 4097];
    assert_eq!(
        plan(&f.world, f.actor, None, f.to, 0),
        Err(CaptureError::Limit)
    );
    f.surface().rooms.truncate(1);
    let surface = f.surface().clone();
    f.entry().composite.as_mut().expect("drawn").surfaces =
        vec![surface; MAX_WINDOW_PLAN_SURFACES + 1];
    assert_eq!(
        plan(&f.world, f.actor, None, f.to, 0),
        Err(CaptureError::Limit)
    );
    f.entry()
        .composite
        .as_mut()
        .expect("drawn")
        .surfaces
        .truncate(1);
    let to = f.to;
    f.state()
        .records
        .roomgeo
        .get_mut(&to)
        .expect("geo")
        .footprint = Footprint::Polygon(Polygon2(vec![
        Point2(0.0, 0.0);
        crate::geom::MAX_POLYGON_POINTS + 1
    ]));
    assert_eq!(
        plan(&f.world, f.actor, None, f.to, 0),
        Err(CaptureError::Limit)
    );
    f.state()
        .records
        .roomgeo
        .get_mut(&to)
        .expect("geo")
        .footprint = Footprint::Anchor {
        point: Point2(1.5, 1.5),
        radius_m: 1.0,
    };
    f.place(
        (0..=u32::try_from(MAX_WINDOW_PLAN_PLACEMENTS).expect("bound"))
            .map(|n| placed(ObjId(n), 88, 0, 0, 0))
            .collect(),
    );
    assert_eq!(
        plan(&f.world, f.actor, None, f.to, 0),
        Err(CaptureError::Limit)
    );
    f.place(vec![]);
    for i in 0..=MAX_WINDOW_PLAN_PORTAL_PROBES {
        let p = portal(&format!("p{i:05}"), ObjId(888), f.to, Point3(0.0, 0.0, 0.0));
        f.state().records.portals.insert(p.key.clone(), p);
    }
    assert_eq!(
        plan(&f.world, f.actor, Some(f.from), f.to, 0),
        Err(CaptureError::Limit)
    );
    plan(&f.world, f.actor, None, f.to, 0).expect("no source means no portal scan");
    let early = portal("a", f.to, f.from, Point3(0.0, 1.0, 0.0));
    f.state().records.portals.insert(early.key.clone(), early);
    assert_eq!(
        selected(&f.plan()),
        (0, 0, 1, Some(f.to)),
        "uninspected portal tails need not be copied"
    );
}
