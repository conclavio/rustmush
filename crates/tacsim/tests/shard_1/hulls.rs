//! **Doc 18 cut IC4 acceptance — walking aboard: the boundary rule and the
//! hull template** (doc 18 §4.2, §4.7 part (f); D-18-22, D-18-23, D-18-29,
//! D-18-34; 18a §3 and §4; lead rulings L1–L19 of the IC4 brief).
//!
//! Lane v2's Ceres programme lays up eight Drays in the Visiting Berth Gallery
//! at the lane's apply: each a Thing titled to the Co-op First Captain Trust
//! under `tacsim.registry`, carrying the Dray's unit, with seven rooms dressed
//! in 18a §3.1's text, five exit pairs, the store and the airlock spaces, the
//! airlock's six walkable member rooms and its two berths, placed `Direct` on
//! the Ceres port domain. A pilot placed in the berths walks all six rooms with
//! no placement write, is refused every other way off, and is still in the
//! Bridge after a reboot. Every hull is recorded in the fail-closed port map; a
//! re-apply makes nothing, a grown pool makes only the new hull, and drift is
//! refused before any write.
//!
//! Composed on the production roster (`tacsim::composition`, the shape
//! `mush_server::boot::game_modules` builds). No test here asserts a passage
//! duration (D-18-50).

use crate::support;

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{Audience, CanonicalId, GOD, ModuleOp, ModuleWrite, ObjId, ObjKind};
use mush_economy::{EconomyConfig, EconomyStore};
use mush_org::{OrgId, OrgScopeId, PrincipalId, SystemPrincipalId};
use mush_proto::OutputEvent;
use mush_sim::data::{SpaceAnchor, SpatialState};
use mush_testkit::{TestWorld, fold_module_ops};
use smol_str::SmolStr;
use tacsim::definitions::InteriorDef;
use tacsim::definitions::content::{DRAY, LANE_JSON, LANE_PACK, LANE_VERSION};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::hull::{HULL_DETAIL_PREFIX, REGISTRY_ISSUER, registry_mark};
use tacsim::port::{HullRow, PortDef, PortMap, PortMapState, room_key};
use tacsim::{TacsimConfig, TacsimStore};

use support::ground::{UNSIGNED, sign};

// ---------------------------------------------------------------------------
// The rig (ports.rs's, minimal)
// ---------------------------------------------------------------------------

fn config() -> TacsimConfig {
    TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    }
}

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
}

/// How a test roster is composed.
#[derive(Clone)]
struct Roster {
    /// Whether the pilot sheet composes (production does).
    sheet: bool,
    /// A shelf replacing the atlas module's compiled one (the v3 variants).
    shelf: Option<Vec<mush_content::Pack>>,
}

impl Roster {
    /// The production roster, the sheet included.
    fn production() -> Roster {
        Roster {
            sheet: true,
            shelf: None,
        }
    }

    /// `tests/lane.rs`'s shape: no sheet, so a spawned player walks.
    fn bare() -> Roster {
        Roster {
            sheet: false,
            ..Roster::production()
        }
    }

    fn with_shelf(mut self, shelf: Vec<mush_content::Pack>) -> Roster {
        self.shelf = Some(shelf);
        self
    }

    fn modules(&self) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let llm = Arc::new(
            mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
                .expect("a disabled LLM service composes"),
        );
        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig::default();
        let cfg = config();
        // Doc 18 cut IC5: the production built-ins, `tacsim` the third quantity
        // delegate, the inventory cell handed to the layer.
        let (mut mods, inventory) = comp::server_modules_with_tacsim(
            mush_weather::WeatherConfig::default(),
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &game,
            comp::recovery_policy(&cfg.recovery),
        );
        if let Some(shelf) = &self.shelf {
            let at = mods
                .iter()
                .position(|m| m.name() == mush_atlas::NS)
                .expect("the atlas composes");
            mods[at] = Box::new(
                mush_atlas::AtlasModule::with_config(mush_atlas::AtlasConfig::default())
                    .with_shelf(with_stations(shelf)),
            );
        }
        mods.push(comp::worldgen_module_with(
            Arc::clone(&llm),
            mush_worldgen::WorldgenConfig::default(),
        ));
        mods.push(comp::npc_module_with(
            comp::npc_agent_adapter(),
            &mush_config::config::NpcCfg::default(),
        ));
        if self.sheet {
            mods.extend(comp::sheet_modules(&onboarding, &game));
        }
        mods.push(comp::tacsim_module_wired(cfg, true, inventory));
        mods.push(comp::content_module_with(true, true).expect("the content module composes"));
        mods
    }
}

/// No module write was refused for its namespace.
fn no_refused_writes(tw: &TestWorld) {
    let refused = tw.world().refused_namespace_writes();
    assert!(refused.is_empty(), "refused namespace writes: {refused:?}");
}

fn boot(roster: &Roster) -> TestWorld {
    let tw = TestWorld::with_modules(roster.modules());
    no_refused_writes(&tw);
    tw
}

fn reboot(
    tw: TestWorld,
    roster: &Roster,
    store: std::collections::HashMap<SmolStr, mush_core::ModuleBootData>,
) -> TestWorld {
    let tw = tw
        .reboot_into(roster.modules(), store)
        .expect("the reboot boots");
    no_refused_writes(&tw);
    tw
}

fn said(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn run(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    said(tw, &events, who)
}

fn store(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("tacsim state")
}

fn economy(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy state")
}

fn spatial(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim state")
}

fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&lane_id(), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

fn ceres_map(tw: &TestWorld) -> PortMap {
    port_map(tw, "ceres_port")
}

/// Both port maps as they stand in RAM.
fn maps(tw: &TestWorld) -> Vec<PortMap> {
    store(tw).port_maps().cloned().collect()
}

/// The port-map documents a batch wrote, by key.
fn map_docs(ops: &[ModuleOp]) -> BTreeMap<String, serde_json::Value> {
    ops.iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, value }
                if key.starts_with(tacsim::port::PORT_MAP_PREFIX) =>
            {
                Some((key.to_string(), value.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The lane's Ceres port entry, decoded from the shipped bytes.
fn ceres_def() -> PortDef {
    let pack = load_pack(LANE_JSON).expect("the lane verifies");
    match pack.entry(&cid("ceres_port")) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("ceres_port is a port entry: {other:?}"),
    }
}

/// The Dray's interior template, decoded from the shipped bytes.
fn interior() -> InteriorDef {
    let pack = load_pack(tacsim::definitions::content::DRAY_JSON).expect("the Dray verifies");
    match pack.entry(&cid(DRAY)) {
        Some(PackEntry::Unit(spec)) => spec.interior.clone().expect("an interior"),
        other => panic!("the Dray's entry is a unit: {other:?}"),
    }
}

/// A Ceres room by its pack-local name (`room.c01_berth_gallery`).
fn ceres_room(tw: &TestWorld, local: &str) -> ObjId {
    let def = ceres_def();
    let key = room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    room_by_key(tw.world(), &key).expect("the room is on this world")
}

/// A hull row's rooms by template id.
fn hull_rooms(row: &HullRow) -> BTreeMap<&str, ObjId> {
    row.rooms
        .iter()
        .map(|(id, obj)| (id.as_str(), ObjId(*obj)))
        .collect()
}

/// The sim `placement:` writes among `ops`.
fn placement_writes(ops: &[ModuleOp]) -> usize {
    ops.iter()
        .filter(|op| {
            op.ns == mush_sim::data::NS
                && matches!(&op.op,
                    ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key }
                        if key.starts_with("placement:"))
        })
        .count()
}

/// The bare roster with a pilot placed in *Slack Water*'s berths: the pilot
/// and the first hull's row.
fn aboard(tw: &mut TestWorld) -> (ObjId, HullRow) {
    let pilot = tw.spawn_player("Pilot");
    let row = ceres_map(tw).hulls[0].clone();
    let airlock = hull_rooms(&row)["airlock"];
    let out = run(
        tw,
        GOD,
        &format!("@behavior/add #{}=sim.participant", pilot.0),
    );
    assert!(!out.contains("denied"), "{out}");
    let out = run(
        tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{},berths", pilot.0, airlock.0),
    );
    assert!(out.contains("Placed"), "{out}");
    assert_eq!(location(tw, pilot), Some(airlock));
    (pilot, row)
}

fn location(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}

fn attr_text(tw: &TestWorld, obj: ObjId, name: &str) -> Option<String> {
    tw.world()
        .attr(obj, name)
        .and_then(|a| a.as_text().map(str::to_owned))
}

/// The aboard refusal for `who` on *Slack Water*.
fn aboard_sentence(who: &str) -> String {
    format!(
        "{who} is aboard Slack Water; it moves only between Slack Water's rooms, or leaves \
         through the simulation or staff recovery (@spatial)."
    )
}

/// The lane's bytes as JSON, edited by `edit`, at `version`, re-signed.
fn lane_variant(version: u32, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["version"] = serde_json::json!(version);
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    edit(&mut doc);
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

/// The lane at the shipped version with test-only edits (`ports.rs`'s base edit plus
/// `edit`), re-signed — it was "one past the shipped v3" at doc 18 cut IC8 and has read
/// `LANE_VERSION` since doc 19 cut A.4 bumped the lane past it.
fn lane_v4(edit: impl FnOnce(&mut serde_json::Value)) -> mush_content::Pack {
    mush_content::load_pack(&lane_variant(LANE_VERSION, |doc| {
        doc["entries"]["ceres_port"]["def"]["programme"]["dormancy_real_days"] =
            serde_json::json!(8);
        doc["entries"]["stickney_port"]["def"]["orgs"][2]["faction"] =
            serde_json::json!("freeport");
        edit(doc);
    }))
    .expect("the variant verifies")
}

/// `@atlas/pack plan` then `@atlas/pack apply <fingerprint>` on the lane.
fn plan_and_apply(tw: &mut TestWorld) -> (String, String) {
    let plan = run(tw, GOD, &format!("@atlas/pack plan {LANE_PACK}"));
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .map(|rest| rest.split_whitespace().next().unwrap_or("").to_owned())
        .unwrap_or_default();
    let apply = run(tw, GOD, &format!("@atlas/pack apply {fingerprint}"));
    (plan, apply)
}

fn lane_latch(tw: &TestWorld) -> Option<u32> {
    tw.world()
        .module_state::<mush_content::ContentState>()
        .and_then(|c| c.latches.applied(&lane_id()))
        .map(mush_content::Applied::version)
}

/// A programmatic destroy and the event the command layer would fire.
fn destroy(tw: &mut TestWorld, obj: ObjId) {
    tw.world_mut().destroy(obj).expect("destroy");
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    tw.engine_mut().emit_world_event(
        &mush_core::WorldEvent::ObjDestroyed { obj },
        &mut out,
        &mut fx,
    );
}

/// The Trust's map row.
fn trust(map: &PortMap) -> (OrgId, OrgScopeId) {
    let row = map.orgs.get(&cid("coop_trust")).expect("the Trust's row");
    (OrgId(row.org), OrgScopeId(row.root_scope))
}

// ---------------------------------------------------------------------------
// 1. The eight Drays
// ---------------------------------------------------------------------------

/// **Boot lays up eight Drays at Ceres** (doc 18 §4.7 part f; lens 5).
#[test]
#[allow(clippy::too_many_lines)]
fn ic4_boot_lays_up_eight_drays_at_ceres() {
    let mut tw = boot(&Roster::production());
    let def = ceres_def();
    let programme = def.programme.clone().expect("a programme");
    let template = interior();
    let map = ceres_map(&tw);
    let gallery = ceres_room(&tw, "room.c01_berth_gallery");
    let domain = map.domain.expect("a domain");
    let (trust_org, trust_scope) = trust(&map);
    assert_eq!(map.hulls.len(), 8);
    let mut last: Option<u32> = None;
    for (i, row) in map.hulls.iter().enumerate() {
        let n = u32::try_from(i).expect("small");
        assert_eq!(row.n, n);
        assert_eq!(row.mark, format!("FPC-D-04{:02}", n + 1));
        assert!(last.is_none_or(|prev| prev < row.hull), "ids ascend with n");
        last = Some(row.hull);
        let hull = ObjId(row.hull);
        let obj = tw.world().get(hull).expect("the hull");
        assert_eq!(obj.kind, ObjKind::Thing);
        assert_eq!(obj.name, programme.hull_names[i]);
        assert_eq!(obj.owner, GOD, "the applying actor, never a pilot");
        assert!(obj.aliases.iter().any(|a| a == &row.mark));
        assert_eq!(obj.location, Some(gallery));
        let (on, _) = mush_sim::pose_of(tw.world(), hull).expect("placed");
        assert_eq!(on.0, domain);
        assert!(
            tw.world()
                .module_state::<mush_tactics::UnitStore>()
                .expect("units")
                .units
                .contains_key(&hull)
        );
        // Seven rooms named as the template, and since doc 20 cut SM.3a (lane v10,
        // step 6b) the refit's bay room on every hull (was 7).
        let rooms = hull_rooms(row);
        assert_eq!(rooms.len(), 8);
        let bay = rooms["bay"];
        assert_eq!(
            tw.world().get(bay).map(|o| o.name.clone()),
            Some("Vehicle Bay".to_owned())
        );
        for room in &template.rooms {
            let obj = rooms[room.id.as_str()];
            let o = tw.world().get(obj).expect("a room");
            assert_eq!(o.kind, ObjKind::Room);
            assert_eq!(o.name, room.name);
        }
        // Ten exits exactly as `interior.exits` lists them.
        assert_eq!(row.exits.len(), 10);
        for (p, pair) in template.exits.iter().enumerate() {
            for (side, (from, to)) in [(&pair.from, &pair.to), (&pair.to, &pair.from)]
                .into_iter()
                .enumerate()
            {
                let exit = tw
                    .world()
                    .get(ObjId(row.exits[2 * p + side]))
                    .expect("an exit");
                let mut segments = pair.names[side].split(';').map(str::trim);
                assert_eq!(exit.kind, ObjKind::Exit);
                assert_eq!(Some(exit.name.as_str()), segments.next());
                let aliases: Vec<&str> = exit.aliases.iter().map(SmolStr::as_str).collect();
                assert_eq!(aliases, segments.collect::<Vec<_>>());
                assert_eq!(exit.location, Some(rooms[from.as_str()]));
                assert_eq!(exit.destination, Some(rooms[to.as_str()]));
            }
        }
        // The spaces.
        let interior_behavior = mush_core::BehaviorId::new("sim.interior").expect("an id");
        let airlock = rooms["airlock"];
        let stowage = rooms["stowage"];
        for space in [airlock, stowage] {
            assert!(tw.world().has_behavior(space, &interior_behavior));
        }
        let st = spatial(&tw);
        let lock = &st.spaces[&airlock];
        assert_eq!(lock.anchor, SpaceAnchor::MovingObject { host: hull });
        let walkable: std::collections::BTreeSet<ObjId> = [
            "airlock",
            "passage",
            "bridge",
            "crew",
            "hold",
            "engineering",
        ]
        .iter()
        .map(|id| rooms[id])
        .collect();
        assert_eq!(lock.rooms, walkable);
        let berths = mush_sim::SlotId::new("berths").expect("a slot");
        assert_eq!(lock.compartments[&berths].maximum_items, Some(2));
        let store = &st.spaces[&stowage];
        assert_eq!(store.anchor, SpaceAnchor::MovingObject { host: hull });
        assert_eq!(store.rooms, std::iter::once(stowage).collect());
        // Doc 20 cut SM.3a: the bay is a space riding the hull, never a member of the
        // airlock, with its rigs compartment at two.
        let bay_space = &st.spaces[&bay];
        assert_eq!(bay_space.anchor, SpaceAnchor::MovingObject { host: hull });
        assert!(!lock.rooms.contains(&bay));
        let rigs = mush_sim::SlotId::new("rigs").expect("a slot");
        assert_eq!(bay_space.compartments[&rigs].maximum_items, Some(2));
        // The title.
        let asset = economy(&tw)
            .snapshot()
            .asset(hull)
            .cloned()
            .expect("titled");
        assert_eq!(asset.title_owner, PrincipalId::Organization(trust_org));
        assert_eq!(asset.scope, Some(trust_scope));
    }
    assert!(port_map(&tw, "stickney_port").hulls.is_empty());

    // A player standing in C01 sees all eight.
    let watcher = tw.spawn_player("Watcher");
    tw.world_mut().move_to(watcher, gallery).expect("moved");
    let out = run(&mut tw, watcher, "look");
    for name in &programme.hull_names {
        assert!(out.contains(name.as_str()), "{name}:\n{out}");
    }
}

// ---------------------------------------------------------------------------
// 2. Room text
// ---------------------------------------------------------------------------

/// **The room text is the template's, and `look` reads the details** from
/// tacsim's own shelf (L14).
#[test]
fn ic4_room_text_is_the_template_and_look_reads_the_details() {
    let mut tw = boot(&Roster::bare());
    let template = interior();
    let row = ceres_map(&tw).hulls[0].clone();
    let rooms = hull_rooms(&row);
    for room in &template.rooms {
        let obj = rooms[room.id.as_str()];
        // A blank template field is no attribute at all (the atlas's rule).
        let want = |text: &str| (!text.trim().is_empty()).then(|| text.to_owned());
        assert_eq!(attr_text(&tw, obj, "desc"), want(&room.desc), "{}", room.id);
        assert_eq!(
            attr_text(&tw, obj, mush_atlas::def::SHORT_DESC_ATTR),
            want(&room.short_desc),
            "{}",
            room.id
        );
        for (keyword, text) in &room.details {
            let attr = format!(
                "{HULL_DETAIL_PREFIX}{}",
                mush_core::fold_detail_keyword(keyword)
            );
            assert_eq!(attr_text(&tw, obj, &attr).as_deref(), Some(text.as_str()));
        }
    }
    let (pilot, _) = aboard(&mut tw);
    let detail = |room: &str, keyword: &str| -> String {
        template
            .room(&cid(room))
            .and_then(|r| r.details.get(keyword))
            .cloned()
            .expect("a detail")
    };
    let out = run(&mut tw, pilot, "look builders plate");
    let want = detail("airlock", "builders plate");
    let head: String = want.chars().take(40).collect();
    assert!(out.contains(&head), "{out}");
    assert!(!run(&mut tw, pilot, "in").is_empty());
    assert_eq!(location(&tw, pilot), Some(rooms["passage"]));
    let out = run(&mut tw, pilot, "look manifest slate");
    let want = detail("passage", "manifest slate");
    let head: String = want.chars().take(40).collect();
    assert!(out.contains(&head), "{out}");
}

// ---------------------------------------------------------------------------
// 3. The title
// ---------------------------------------------------------------------------

/// **`@asset/show` names the Trust** (lens 6).
#[test]
fn ic4_asset_show_names_the_trust() {
    let mut tw = boot(&Roster::production());
    let hull = ObjId(ceres_map(&tw).hulls[0].hull);
    let out = run(&mut tw, GOD, &format!("@asset/show #{}", hull.0));
    assert!(out.contains("title"), "{out}");
    assert!(out.contains("Co-op First Captain Trust"), "{out}");
}

// ---------------------------------------------------------------------------
// 4. Walking
// ---------------------------------------------------------------------------

/// **A pilot walks all six rooms with no placement write** (lens 1).
#[test]
fn ic4_a_pilot_walks_all_six_rooms_with_no_placement_write() {
    let mut tw = boot(&Roster::bare());
    let (pilot, row) = aboard(&mut tw);
    let rooms = hull_rooms(&row);
    let _ = tw.take_module_ops();
    let placement = spatial(&tw).placements[&pilot].clone();
    for (word, room) in [
        ("in", "passage"),
        ("up", "bridge"),
        ("down", "passage"),
        ("aft", "crew"),
        ("forward", "passage"),
        ("down", "hold"),
        ("down", "engineering"),
        ("up", "hold"),
        ("up", "passage"),
        ("forward", "airlock"),
    ] {
        let out = run(&mut tw, pilot, word);
        assert_eq!(location(&tw, pilot), Some(rooms[room]), "{word}: {out}");
        let ops = tw.take_module_ops();
        assert_eq!(placement_writes(&ops), 0, "{word} wrote a placement");
        assert_eq!(spatial(&tw).placements[&pilot], placement);
    }
    assert!(mush_sim::verify_projections(spatial(&tw), tw.world()).is_empty());
}

// ---------------------------------------------------------------------------
// 5. Every way off
// ---------------------------------------------------------------------------

/// **The boundary refuses every way off but staff recovery** (lens 2).
#[test]
fn ic4_the_boundary_refuses_every_way_off_but_staff_recovery() {
    let mut tw = boot(&Roster::bare());
    let (pilot, row) = aboard(&mut tw);
    let rooms = hull_rooms(&row);
    let bridge = rooms["bridge"];
    run(&mut tw, pilot, "in");
    run(&mut tw, pilot, "up");
    assert_eq!(location(&tw, pilot), Some(bridge));
    let c03 = ceres_room(&tw, "room.c03_arrivals");
    let out = run(&mut tw, pilot, "o");
    assert!(out.contains(&aboard_sentence("Pilot")), "{out}");
    assert_eq!(location(&tw, pilot), Some(bridge));
    for line in ["s".to_owned(), "c".to_owned(), "home".to_owned()] {
        let out = run(&mut tw, pilot, &line);
        assert_eq!(location(&tw, pilot), Some(bridge), "{line}: {out}");
    }
    // `+ic` reaches the gate with no exit (the Bridge is no port room) and
    // answers the aboard sentence as `Sys` (the IC1 carry-forward).
    let events = tw.run(pilot, "+ic");
    let sys = events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if mush_testkit::text_of(&OutputEvent::Sys(rt.clone()))
            .contains(&aboard_sentence("Pilot")))
    });
    assert!(sys, "{}", said(&tw, &events, pilot));
    assert_eq!(location(&tw, pilot), Some(bridge));
    // C03 made JUMP_OK so the pilot's own `@teleport` clears
    // `perm::can_teleport` and the refusal is the gate's (IC4 review T2).
    tw.world_mut()
        .set_flag(c03, mush_core::Flags::JUMP_OK, true)
        .expect("C03 takes JUMP_OK");
    let out = run(&mut tw, pilot, &format!("@teleport me=#{}", c03.0));
    assert!(out.contains(&aboard_sentence("Pilot")), "{out}");
    assert_eq!(location(&tw, pilot), Some(bridge), "{out}");
    let out = run(&mut tw, GOD, &format!("@teleport #{}=#{}", pilot.0, c03.0));
    assert!(out.contains(&aboard_sentence("Pilot")), "{out}");
    assert_eq!(location(&tw, pilot), Some(bridge));
    for word in ["egress", "disembark"] {
        let out = run(&mut tw, pilot, word);
        assert!(
            out.contains("You cannot egress from Slack Water; use +ship/leave in the Airlock."),
            "{word}: {out}"
        );
        assert_eq!(location(&tw, pilot), Some(bridge));
    }
}

// ---------------------------------------------------------------------------
// 6. A reboot
// ---------------------------------------------------------------------------

/// **A reboot keeps the walker in the Bridge** (lens 3).
#[test]
fn ic4_a_reboot_keeps_the_walker_in_the_bridge() {
    let roster = Roster::bare();
    let mut tw = boot(&roster);
    let (pilot, row) = aboard(&mut tw);
    let bridge = hull_rooms(&row)["bridge"];
    run(&mut tw, pilot, "in");
    run(&mut tw, pilot, "up");
    assert_eq!(location(&tw, pilot), Some(bridge));
    let placement = spatial(&tw).placements[&pilot].clone();
    let before_maps = maps(&tw);
    let objects = tw.world().iter().count();
    let folded = fold_module_ops(tw.take_module_ops());
    let mut tw = reboot(tw, &roster, folded);
    assert_eq!(location(&tw, pilot), Some(bridge));
    assert_eq!(spatial(&tw).placements[&pilot], placement);
    assert_eq!(maps(&tw), before_maps);
    assert_eq!(tw.world().iter().count(), objects, "no object was created");
    let ops = tw.take_module_ops();
    assert!(map_docs(&ops).is_empty(), "the reboot wrote a port map");
    assert_eq!(placement_writes(&ops), 0, "the reboot wrote a placement");
}

// ---------------------------------------------------------------------------
// 7. The berths
// ---------------------------------------------------------------------------

/// **The berths cap refuses a third placement.**
#[test]
fn ic4_the_berths_cap_refuses_a_third_placement() {
    let mut tw = boot(&Roster::bare());
    let row = ceres_map(&tw).hulls[0].clone();
    let airlock = hull_rooms(&row)["airlock"];
    let crew: Vec<ObjId> = ["Ann", "Bea", "Cid"]
        .iter()
        .map(|name| tw.spawn_player(name))
        .collect();
    for (i, who) in crew.iter().enumerate() {
        run(
            &mut tw,
            GOD,
            &format!("@behavior/add #{}=sim.participant", who.0),
        );
        let out = run(
            &mut tw,
            GOD,
            &format!("@spatial/place/interior #{}=#{},berths", who.0, airlock.0),
        );
        if i < 2 {
            assert!(out.contains("Placed"), "{out}");
        } else {
            assert!(out.contains("is full (2 max)"), "{out}");
            assert!(!spatial(&tw).placements.contains_key(who));
        }
    }
}

// ---------------------------------------------------------------------------
// 8. A member room
// ---------------------------------------------------------------------------

/// **A member room cannot be destroyed and cannot become a space** (lead note
/// §5). "Unchanged" is asserted on the records, not on behaviour bindings.
#[test]
fn ic4_a_member_cannot_be_destroyed_and_cannot_become_a_space() {
    let mut tw = boot(&Roster::bare());
    let row = ceres_map(&tw).hulls[0].clone();
    let rooms = hull_rooms(&row);
    let (bridge, airlock, stowage) = (rooms["bridge"], rooms["airlock"], rooms["stowage"]);
    let hull = ObjId(row.hull);
    let snapshot = |tw: &TestWorld| {
        let st = spatial(tw);
        (
            st.spaces[&airlock].clone(),
            st.spaces[&stowage].clone(),
            st.derived.member_of.clone(),
            st.placements[&hull].clone(),
        )
    };
    let before = snapshot(&tw);
    let out = run(&mut tw, GOD, &format!("@destroy #{}", bridge.0));
    assert!(
        out.contains(
            "Bridge is a room of the interior space Airlock; @spatial/unspace that space first."
        ),
        "{out}"
    );
    let out = run(
        &mut tw,
        GOD,
        &format!("@behavior/add #{}=sim.interior", bridge.0),
    );
    assert!(!out.to_ascii_lowercase().contains("denied"), "{out}");
    let out = run(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=moving:#{}", bridge.0, hull.0),
    );
    assert!(
        out.contains(&format!(
            "{bridge} is a room of interior space {airlock}; it cannot be a space itself"
        )),
        "{out}"
    );
    assert!(
        !tw.engine().has_authoritative_submit(),
        "a refused member-room request must not queue a physical transition"
    );
    let out = run(
        &mut tw,
        GOD,
        &format!("@behavior/remove #{}=sim.interior", bridge.0),
    );
    let interior_behavior = mush_core::BehaviorId::new("sim.interior").expect("an id");
    assert!(
        !tw.world().has_behavior(bridge, &interior_behavior),
        "{out}"
    );
    assert_eq!(snapshot(&tw), before);
    assert!(!spatial(&tw).spaces.contains_key(&bridge));
    assert!(tw.world().get(bridge).is_some(), "the Bridge stands");
}

// ---------------------------------------------------------------------------
// 9. Custody
// ---------------------------------------------------------------------------

/// **The stowage can back a vault and the airlock cannot** (doc 18 §4.2 (d)).
#[test]
fn ic4_the_stowage_can_back_a_vault_and_the_airlock_cannot() {
    let mut tw = boot(&Roster::production());
    let row = ceres_map(&tw).hulls[0].clone();
    let rooms = hull_rooms(&row);
    let vaults = |tw: &TestWorld| economy(tw).snapshot().vaults().count();
    let before = vaults(&tw);
    let out = run(
        &mut tw,
        GOD,
        &format!("@vault/create a=interior:#{}", rooms["airlock"].0),
    );
    assert_eq!(vaults(&tw), before, "{out}");
    let out = run(
        &mut tw,
        GOD,
        &format!("@vault/create b=interior:#{}", rooms["stowage"].0),
    );
    assert_eq!(vaults(&tw), before + 1, "{out}");
}

// ---------------------------------------------------------------------------
// 10. A second boot
// ---------------------------------------------------------------------------

/// **A second boot makes no hull** (doc 18 §4.7 "Idempotency").
#[test]
fn ic4_a_second_boot_makes_no_hull() {
    let roster = Roster::production();
    let mut tw = boot(&roster);
    let before_maps = maps(&tw);
    let objects = tw.world().iter().count();
    let journal = economy(&tw).journal_len();
    let assets = economy(&tw).snapshot().assets().count();
    let folded = fold_module_ops(tw.take_module_ops());
    let mut tw = reboot(tw, &roster, folded);
    assert_eq!(ceres_map(&tw).hulls.len(), 8);
    assert_eq!(maps(&tw), before_maps);
    assert_eq!(tw.world().iter().count(), objects, "no new object");
    assert_eq!(economy(&tw).journal_len(), journal, "no title posted");
    assert_eq!(economy(&tw).snapshot().assets().count(), assets);
    assert!(map_docs(&tw.take_module_ops()).is_empty());
}

// ---------------------------------------------------------------------------
// 11. A grown pool
// ---------------------------------------------------------------------------

/// **An update creates only the new hull** (L10): pool 9, a ninth name, a
/// smaller stake so the floor still fits.
#[test]
fn ic4_an_update_creates_only_the_new_hull() {
    let v3 = lane_v4(|doc| {
        let programme = &mut doc["entries"]["ceres_port"]["def"]["programme"];
        programme["pool"] = serde_json::json!(9);
        programme["stake_major"] = serde_json::json!(20_000);
        programme["hull_names"]
            .as_array_mut()
            .expect("names")
            .push(serde_json::json!("Ninth Wave"));
    });
    let roster = Roster::production().with_shelf(vec![v3]);
    let mut tw = boot(&roster);
    let before = ceres_map(&tw);
    let objects = tw.world().iter().count();
    let assets = economy(&tw).snapshot().assets().count();
    let placements: Vec<_> = before
        .hulls
        .iter()
        .map(|row| spatial(&tw).placements[&ObjId(row.hull)].clone())
        .collect();
    let titles: Vec<_> = before
        .hulls
        .iter()
        .map(|row| economy(&tw).snapshot().asset(ObjId(row.hull)).cloned())
        .collect();
    let (plan, apply) = plan_and_apply(&mut tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{LANE_VERSION}")),
        "{apply}\n{plan}"
    );
    let after = ceres_map(&tw);
    assert_eq!(after.hulls.len(), 9);
    assert_eq!(
        &after.hulls[..8],
        &before.hulls[..],
        "the earlier rows stand"
    );
    let ninth = &after.hulls[8];
    assert_eq!(ninth.n, 8);
    assert_eq!(ninth.mark, "FPC-D-0409");
    // Doc 20 cut SM.3a: the bay room too (was 7).
    assert_eq!(ninth.rooms.len(), 8);
    assert_eq!(ninth.exits.len(), 10);
    assert_eq!(
        tw.world().get(ObjId(ninth.hull)).map(|o| o.name.clone()),
        Some("Ninth Wave".to_owned())
    );
    assert_eq!(
        tw.world().iter().count(),
        objects + 19,
        "one Thing, 8 rooms (the bay among them since doc 20 cut SM.3a), 10 exits"
    );
    assert_eq!(
        economy(&tw).snapshot().assets().count(),
        assets + 1,
        "one title"
    );
    for (i, row) in before.hulls.iter().enumerate() {
        assert_eq!(spatial(&tw).placements[&ObjId(row.hull)], placements[i]);
        assert_eq!(
            economy(&tw).snapshot().asset(ObjId(row.hull)).cloned(),
            titles[i]
        );
    }
}

/// **A held hull document is skipped, never refused mid-pass** (L10; IC4
/// review correctness/F1): on a kept world one hull's `placement:` document
/// and another's `unit:` document no longer decode, so boot holds them. A
/// later version that grows the pool still makes the ninth hull; the held
/// hulls are neither re-placed nor given a second unit, and their documents
/// stay staff's to repair.
#[test]
fn ic4_a_held_hull_document_is_skipped_not_refused() {
    let v3 = lane_v4(|doc| {
        let programme = &mut doc["entries"]["ceres_port"]["def"]["programme"];
        programme["pool"] = serde_json::json!(9);
        programme["stake_major"] = serde_json::json!(20_000);
        programme["hull_names"]
            .as_array_mut()
            .expect("names")
            .push(serde_json::json!("Ninth Wave"));
    });
    let roster = Roster::production().with_shelf(vec![v3]);
    let mut tw = boot(&roster);
    let before = ceres_map(&tw);
    let (held_place, held_unit) = (ObjId(before.hulls[2].hull), ObjId(before.hulls[3].hull));
    let mut folded = fold_module_ops(tw.take_module_ops());
    let mut corrupt = |ns: &str, key: String| {
        let slot = folded
            .get_mut(ns)
            .and_then(|data| data.kv.iter_mut().find(|(k, _)| k.as_str() == key))
            .unwrap_or_else(|| panic!("{ns} {key}"));
        slot.1 = serde_json::json!({ "not": "a record" });
    };
    corrupt(
        mush_sim::data::NS,
        mush_sim::data::placement_key(held_place).to_string(),
    );
    corrupt(mush_tactics::module::NS, format!("unit:{}", held_unit.0));
    let mut tw = reboot(tw, &roster, folded);
    assert!(spatial(&tw).held_placement(held_place).is_some());
    assert!(!spatial(&tw).placements.contains_key(&held_place));
    let units = |tw: &TestWorld| {
        tw.world()
            .module_state::<mush_tactics::UnitStore>()
            .expect("units")
            .clone()
    };
    assert!(
        !units(&tw).units.contains_key(&held_unit),
        "the unit is held"
    );
    let held_record = units(&tw)
        .records
        .iter()
        .find(|r| r.key_object == Some(held_unit))
        .cloned()
        .expect("a held unit record");
    let (plan, apply) = plan_and_apply(&mut tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{LANE_VERSION}")),
        "{apply}\n{plan}"
    );
    let after = ceres_map(&tw);
    assert_eq!(after.hulls.len(), 9, "the ninth hull is made");
    assert_eq!(
        &after.hulls[..8],
        &before.hulls[..],
        "the earlier rows stand"
    );
    assert!(spatial(&tw).held_placement(held_place).is_some());
    assert!(
        !spatial(&tw).placements.contains_key(&held_place),
        "a held placement is never re-placed"
    );
    assert!(!units(&tw).units.contains_key(&held_unit), "no second unit");
    assert!(units(&tw).records.contains(&held_record));
    let ops = tw.take_module_ops();
    let placement_key = mush_sim::data::placement_key(held_place);
    let unit_key = format!("unit:{}", held_unit.0);
    assert!(
        !ops.iter().any(|op| matches!(&op.op,
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key }
                if *key == placement_key || key.as_str() == unit_key)),
        "the held documents are never rewritten"
    );
}

// ---------------------------------------------------------------------------
// 12. Drift
// ---------------------------------------------------------------------------

/// **Hull drift is refused before anything is made** (L10).
#[test]
fn ic4_hull_drift_is_refused_before_anything_is_made() {
    let shrink = lane_v4(|doc| {
        let programme = &mut doc["entries"]["ceres_port"]["def"]["programme"];
        programme["pool"] = serde_json::json!(7);
        programme["hull_names"]
            .as_array_mut()
            .expect("names")
            .truncate(7);
    });
    let reprefix = lane_v4(|doc| {
        doc["entries"]["ceres_port"]["def"]["programme"]["registry_prefix"] =
            serde_json::json!("FPC-D-05");
    });
    let plain = lane_v4(|_| {});
    type Prepare = Box<dyn Fn(&mut TestWorld)>;
    let cases: Vec<(&str, mush_content::Pack, Prepare, &str)> = vec![
        ("pool 7", shrink, Box::new(|_| {}), "never retired"),
        ("prefix 05", reprefix, Box::new(|_| {}), "never re-issued"),
        (
            "a hull destroyed",
            plain.clone(),
            Box::new(|tw| {
                let hull = ObjId(ceres_map(tw).hulls[3].hull);
                destroy(tw, hull);
            }),
            "never re-created",
        ),
    ];
    for (name, pack, prepare, words) in cases {
        let roster = Roster::production().with_shelf(vec![pack]);
        let mut tw = boot(&roster);
        prepare(&mut tw);
        let before_maps = maps(&tw);
        let objects = tw.world().iter().count();
        let journal = economy(&tw).journal_len();
        let (_plan, apply) = plan_and_apply(&mut tw);
        assert!(apply.contains(words), "{name}:\n{apply}");
        assert!(!apply.contains("Applied "), "{name}:\n{apply}");
        assert_eq!(lane_latch(&tw), Some(LANE_VERSION), "{name}");
        assert_eq!(maps(&tw), before_maps, "{name}");
        assert_eq!(tw.world().iter().count(), objects, "{name}");
        assert_eq!(economy(&tw).journal_len(), journal, "{name}");
    }

    // A map whose first row records an eleventh exit (a live object, so the
    // count is what is refused), edited in the folded store before a reboot
    // onto the v3 shelf (IC3's map-drift pattern).
    let roster = Roster::production().with_shelf(vec![plain]);
    let mut tw = boot(&roster);
    let extra = ceres_map(&tw).hulls[1].exits[0];
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port")).to_string();
    let mut folded = fold_module_ops(tw.take_module_ops());
    let slot = folded
        .get_mut(tacsim::NS)
        .and_then(|data| data.kv.iter_mut().find(|(k, _)| k.as_str() == key))
        .expect("the ceres map document");
    slot.1["hulls"][0]["exits"]
        .as_array_mut()
        .expect("exits")
        .push(serde_json::json!(extra));
    let mut tw = reboot(tw, &roster, folded);
    assert_eq!(ceres_map(&tw).hulls[0].exits.len(), 11);
    let before_maps = maps(&tw);
    let objects = tw.world().iter().count();
    let (_plan, apply) = plan_and_apply(&mut tw);
    assert!(apply.contains("exits are never retired"), "{apply}");
    assert!(!apply.contains("Applied "), "{apply}");
    assert_eq!(maps(&tw), before_maps);
    assert_eq!(tw.world().iter().count(), objects);
}

// ---------------------------------------------------------------------------
// 13. The readiness line
// ---------------------------------------------------------------------------

/// **The readiness report counts the hulls** (L17).
#[test]
fn ic4_the_readiness_report_counts_the_hulls() {
    let mut tw = boot(&Roster::production());
    let out = run(&mut tw, GOD, "@tacsim/port status");
    assert!(
        out.contains(
            "  ceres_port hulls: 8 of 8 made, titled to Co-op First Captain Trust and laid up. \
             [ready]"
        ),
        "{out}"
    );
    assert!(!out.contains("stickney_port hulls"), "{out}");
    let last = ObjId(ceres_map(&tw).hulls[7].hull);
    destroy(&mut tw, last);
    let out = run(&mut tw, GOD, "@tacsim/port status");
    assert!(
        out.contains(&format!(
            "  ceres_port hulls: FPC-D-0408 (#{}) does not exist. [missing]",
            last.0
        )),
        "{out}"
    );
}

// ---------------------------------------------------------------------------
// 14. The issuer
// ---------------------------------------------------------------------------

/// **The registry issuer is the programme's** (doc 18 §4.2 schema addition 3).
#[test]
fn ic4_the_registry_issuer_is_the_programmes() {
    let programme = ceres_def().programme.expect("a programme");
    assert_eq!(programme.issuer, REGISTRY_ISSUER);
    let tw = boot(&Roster::production());
    let issuer = SystemPrincipalId::new(REGISTRY_ISSUER).expect("a principal");
    assert!(economy(&tw).is_title_issuer(&issuer));
    assert_eq!(registry_mark("FPC-D-04", 0), "FPC-D-0401");
    assert_eq!(registry_mark("FPC-D-04", 7), "FPC-D-0408");
}

/// A test shelf with the four station packs ahead of it (doc 18 cut IC8): the
/// `PortApplier` resolves the vendors' storefront items and signatories from the
/// station packs on the atlas shelf (lead ruling L3), as the production shelf always
/// carries them.
fn with_stations(shelf: &[mush_content::Pack]) -> Vec<mush_content::Pack> {
    let mut full = tacsim::definitions::content::station_packs().expect("the station packs");
    full.extend(shelf.iter().cloned());
    full
}
