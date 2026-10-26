//! S9.1 (D-14-33): the corridor's places and routes are the atlas's sites and
//! connections, applied through the content latch — one row per pack id, a
//! newer version an update in place, rooms kept; an orbital site lies in no
//! neighborhood and rasterizes nothing; a corridor's length is the sky's pin;
//! and (doc 18 cut IC1, Q-18-6) no player verb travels a corridor connection —
//! its `spacecraft` mode is the atlas's projection of a carrier route.

use mush_atlas::record::RecordKey;
use mush_atlas::{AtlasModule, AtlasState};
use mush_content::{Appliers, ContentModule, ContentState};
use mush_core::{CanonicalId, GOD, ObjId, ObjKind};
use mush_sim::SimulationModule;
use mush_testkit::TestWorld;
use tacsim::{TacsimRuleset, TacsimStore, fixture};

/// The TacSim ruleset with both corridor versions loaded, the simulation, the
/// atlas and the content module with the atlas's and this crate's appliers.
fn lot() -> TestWorld {
    let mut cfg = fixture::config();
    cfg.seeds.push(tacsim::PackSource {
        name: "<built-in tacsim.corridor@2>".to_string(),
        bytes: fixture::CORRIDOR_V2_JSON.to_string(),
    });
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    tacsim::register_appliers(&mut appliers).expect("tacsim appliers");
    TestWorld::with_modules(vec![
        Box::new(TacsimRuleset::with_config(cfg)),
        Box::new(SimulationModule::new()),
        Box::new(AtlasModule::new()),
        Box::new(ContentModule::new().with_appliers(appliers)),
    ])
}

fn atlas(tw: &TestWorld) -> &AtlasState {
    tw.world()
        .module_state::<AtlasState>()
        .expect("atlas state")
}

fn store(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("tacsim state")
}

fn corridor() -> CanonicalId {
    CanonicalId::new(fixture::CORRIDOR_PACK).expect("a canonical id")
}

fn key(entry: &str) -> RecordKey {
    RecordKey::new(corridor(), CanonicalId::new(entry).expect("an id"))
}

fn seed(tw: &mut TestWorld, version: u32) -> String {
    let events = tw.run(
        GOD,
        &format!("@tacsim/seed tacsim.corridor@{version} commit"),
    );
    tw.seen_by(&events, GOD).join("\n")
}

fn placed(tw: &TestWorld, version: u32) -> std::collections::BTreeMap<CanonicalId, ObjId> {
    store(tw)
        .placed_in(tw.world(), &corridor(), version)
        .expect("the content latch placed the corridor")
}

/// The length of a connection's corridor polyline, kilometres, from gigametres.
fn corridor_km(points: &[mush_atlas::Point3]) -> f64 {
    points
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .sum::<f64>()
        / tacsim::GM_PER_KM
}

#[test]
fn the_corridor_seeds_through_the_content_latch_as_sites_and_connections() {
    let mut tw = lot();
    let out = seed(&mut tw, 1);
    assert!(out.contains("applied through the content latch"), "{out}");
    let latch = tw
        .world()
        .module_state::<ContentState>()
        .expect("content state")
        .latches
        .applied(&corridor())
        .cloned()
        .expect("the content latch holds the corridor");
    assert_eq!(latch.version(), 1);
    let placed = placed(&tw, 1);
    assert_eq!(placed.len(), 3, "{placed:?}");
    for room in placed.values() {
        assert_eq!(tw.world().get(*room).unwrap().kind, ObjKind::Room);
    }
    let st = atlas(&tw);
    let sol = key("sol");
    assert!(
        st.records.regions.contains_key(&sol),
        "the pack's system region"
    );
    for place in fixture::PLACES {
        let site = st.records.sites.get(&key(place)).expect(place);
        assert!(
            site.neighborhoods.is_empty(),
            "{place} lies in no neighborhood"
        );
        assert_eq!(site.region.as_ref(), Some(&sol));
        assert!(!site.tactical_ready);
    }
    let conn = st
        .records
        .connections
        .get(&key(fixture::ROUTE))
        .expect("the route is a connection");
    assert_eq!(
        conn.endpoints.0.room,
        placed
            .get(&CanonicalId::new("ceres_extraction").unwrap())
            .copied()
    );
    assert_eq!(
        conn.endpoints.1.room,
        placed
            .get(&CanonicalId::new("phobos_yard").unwrap())
            .copied()
    );
    assert!(conn.endpoints.0.neighborhood.is_none());
    // The legacy latch was not written.
    assert!(store(&tw).seed(&corridor(), 1).is_none());
}

#[test]
fn an_orbital_site_validates_and_rasterizes_nothing() {
    let mut tw = lot();
    seed(&mut tw, 1);
    let st = atlas(&tw);
    let sol = key("sol");
    assert!(
        st.records.tiles.values().all(|t| t.region != sol),
        "a system region has no ground tiles"
    );
    for place in fixture::PLACES {
        st.records
            .sites
            .get(&key(place))
            .expect(place)
            .validate()
            .expect("valid");
    }
    // The region view lists them under the region.
    let events = tw.run(GOD, "+atlas/region Sol");
    let text = tw.seen_by(&events, GOD).join("\n");
    assert!(text.contains("Phobos Yard"), "{text}");
}

#[test]
fn the_corridors_length_is_the_skys_pin() {
    let mut tw = lot();
    seed(&mut tw, 1);
    let conn = atlas(&tw)
        .records
        .connections
        .get(&key(fixture::ROUTE))
        .cloned()
        .expect("the connection");
    let route = store(&tw)
        .route_in_world(tw.world(), &CanonicalId::new(fixture::ROUTE).unwrap())
        .expect("the route materializes against the content latch");
    let pin = route.distance_km().expect("the S7 pin") as f64;
    let drawn = corridor_km(&conn.corridor);
    assert!(
        (drawn - pin).abs() <= 2.0,
        "the corridor is drawn between the epoch positions: {drawn} km against the pin {pin} km"
    );
}

#[test]
fn a_second_version_updates_in_place_and_keeps_the_rooms() {
    let mut tw = lot();
    seed(&mut tw, 1);
    let before = placed(&tw, 1);
    let revision_before = atlas(&tw)
        .records
        .connections
        .get(&key(fixture::ROUTE))
        .map(|c| c.revision)
        .expect("the connection");
    let dry = tw.run(GOD, "@tacsim/seed tacsim.corridor@2 dry");
    let dry = tw.seen_by(&dry, GOD).join("\n");
    assert!(
        dry.contains("would update the corridor applied at @1"),
        "{dry}"
    );
    let out = seed(&mut tw, 2);
    assert!(out.contains("applied through the content latch"), "{out}");
    let after = placed(&tw, 2);
    assert_eq!(after.len(), fixture::PLACES_V2.len(), "{after:?}");
    for (id, room) in &before {
        assert_eq!(after.get(id), Some(room), "{id} keeps its room");
        assert_eq!(tw.world().get(*room).unwrap().kind, ObjKind::Room);
    }
    let st = atlas(&tw);
    assert!(
        st.records
            .connections
            .contains_key(&key(fixture::ROUTE_MARS))
    );
    assert!(
        st.records
            .connections
            .get(&key(fixture::ROUTE))
            .map(|c| c.revision)
            > Some(revision_before),
        "the re-applied connection's revision moved up"
    );
    // A third commit of the same version is a HIT.
    let again = seed(&mut tw, 2);
    assert!(again.contains("already been applied"), "{again}");
}

/// **Doc 18 cut IC1** (Q-18-6, binding; D-18-49): no player verb flies the
/// atlas's drawn corridor. A corridor connection's mode is the atlas's projection
/// of a carrier route; travel between stations is `mush_sim::CarrierJourney`.
/// This replaces S9.1's atlas-journey test, which pinned exactly the path the
/// cut deletes (`options_from_corridor_room`), and is written against the
/// constant so renaming the mode cannot reopen it.
#[test]
fn no_player_verb_traverses_a_spacecraft_connection() {
    let mut tw = lot();
    seed(&mut tw, 1);
    let placed = placed(&tw, 1);
    let ceres = placed[&CanonicalId::new("ceres_extraction").unwrap()];
    let pilot = tw.spawn_player("Pilot");
    tw.world_mut().move_to(pilot, ceres).unwrap();
    tw.world_mut().move_to(GOD, ceres).unwrap();
    // A player and staff standing in the corridor's room are both refused, and no
    // atlas journey starts.
    for who in [pilot, GOD] {
        let events = tw.run(who, "+travel Phobos Yard");
        let out = tw.seen_by(&events, who).join("\n");
        assert!(
            out.contains("You know no way to \"Phobos Yard\" from here."),
            "{out}"
        );
        assert!(atlas(&tw).travel.journey_of(who).is_none(), "{out}");
        assert_eq!(tw.world().get(who).and_then(|o| o.location), Some(ceres));
    }
    // The mode is not a self mode, and no corridor connection offers a way out
    // of either of its endpoint rooms, to a player or to staff.
    assert!(!mush_atlas::travel::SELF_MODES.contains(&tacsim::CORRIDOR_MODE));
    let st = atlas(&tw);
    let corridors: Vec<_> = st
        .records
        .connections
        .values()
        .filter(|c| c.modes.iter().any(|m| m == tacsim::CORRIDOR_MODE))
        .collect();
    assert!(!corridors.is_empty(), "the seed drew its corridor");
    for conn in corridors {
        for room in [conn.endpoints.0.room, conn.endpoints.1.room]
            .into_iter()
            .flatten()
        {
            for (who, staff) in [(pilot, false), (GOD, true)] {
                let options = mush_atlas::travel::options_from(st, who, room, staff);
                assert!(
                    options.is_empty(),
                    "{} offers a way out of #{}: {:?}",
                    conn.key,
                    room.0,
                    options.iter().map(|o| &o.place_name).collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn a_carrier_journey_along_the_corridor_is_listed_by_the_atlas() {
    use mush_core::{ClockRef, ClockStamp};
    use mush_sim::{CarrierJourney, JourneyBook, JourneyId, JourneyState, SpatialParent};

    let mut tw = lot();
    seed(&mut tw, 1);
    let placed = placed(&tw, 1);
    let phobos = placed[&CanonicalId::new("phobos_yard").unwrap()];
    let hull = tw
        .world_mut()
        .create(ObjKind::Thing, "Hauler Kestrel", GOD, Some(phobos))
        .unwrap();
    let route = store(&tw)
        .route_in_world(tw.world(), &CanonicalId::new(fixture::ROUTE).unwrap())
        .expect("the route materializes against the content latch");
    let stamp = ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 0,
    };
    let journey = CarrierJourney::new(
        JourneyId(1),
        hull,
        route,
        SpatialParent::Interior {
            space: phobos,
            slot: None,
        },
        stamp.clone(),
        stamp,
        0,
        JourneyState::Cruising,
        None,
    )
    .expect("a coherent journey");
    tw.world_mut()
        .module_state_mut::<JourneyBook>()
        .expect("the sim's journey book")
        .journeys
        .insert(hull, journey);
    // S9.2: the atlas reads the simulation's carrier journeys — one view.
    assert_eq!(
        mush_atlas::travel::aboard_carrier(tw.world(), hull),
        Some(hull)
    );
    let events = tw.run(GOD, "@atlas/journey list");
    let text = tw.seen_by(&events, GOD).join("\n");
    assert!(text.contains("carrier Hauler Kestrel"), "{text}");
    assert!(
        text.contains(fixture::ROUTE) && text.contains("cruising"),
        "{text}"
    );
}
