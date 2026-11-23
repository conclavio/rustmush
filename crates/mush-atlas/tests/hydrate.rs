//! Hydration (doc 14 §4.5; S3): a request reserves the character's slot, the
//! neighborhood's unit and its ground and converges on itself; activation marks the
//! site hydrated, files a BUILD ticket and records canon; review approves, asks for
//! changes or strikes; three strikes zero the cap until staff reset it; the planner
//! hand-off; and a reboot.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mush_atlas::hydrate::{self, Placement, RequestRec, RequestStatus, Requested};
use mush_atlas::{AtlasConfig, AtlasModule, AtlasState, Polygon2, RecordKey};
use mush_canon::CanonState;
use mush_content::{Appliers, ContentModule, ContentState, Pack, parse_pack};
use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CanonicalId, CmdError, CommandSpec, Ctx, GOD, GameModule, ModuleBootData,
    ModuleWrite, ObjId, ObjKind,
};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use serde_json::{Value, json};
use smol_str::SmolStr;

fn signed(mut doc: Value) -> Pack {
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    parse_pack(&serde_json::to_string(&doc).expect("json")).expect("parses")
}

fn region() -> Pack {
    signed(json!({
        "manifest": { "id": "region.town", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.town": { "kind": "region", "def": {
                "name": "The Town", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]], "datum_m": 0.0,
                "background": { "surface": "paved", "mix": [["grass", 8]], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 1000 } },
            "locality.town": { "kind": "locality", "def": {
                "region": "region.town", "name": "Town", "kind": "district",
                "footprint": [[0, 0], [400, 0], [400, 400], [0, 400]] } }
        }
    }))
}

/// The Block: a Green off Limbo, open to player sites.
fn block(enabled: bool, max_player_sites: u32) -> Pack {
    signed(json!({
        "manifest": { "id": "neighborhood.block", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "The Block",
                "setting": "suburban", "footprint": [[0, 0], [200, 0], [200, 200], [0, 200]],
                "room_budget": { "target": 3, "maximum": 12 },
                "hydration": { "enabled": enabled, "max_player_sites": max_player_sites } } },
            "room.green": { "kind": "room", "def": { "name": "The Green", "surface": "ground",
                "footprint": { "rect": [[10, 10], [60, 60]] }, "desc": "A wide green." } },
            "exit.limbo_green": { "kind": "exit", "def": { "from": "limbo", "to": "room.green",
                "names": ["Out <O>;o;out", "Limbo <L>;limbo"] } }
        }
    }))
}

/// A test module whose `+probe` command runs one queued closure inside a real
/// command context, since the hydration API takes a `Ctx` the way a module's own
/// commands would hand it one.
type ProbeJob = Box<dyn FnOnce(&mut Ctx<'_>) + Send>;

#[derive(Clone, Default)]
struct ProbeQueue(Arc<Mutex<Vec<ProbeJob>>>);

struct Probe;

impl GameModule for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(ProbeQueue::default());
        // WS03-A1: `+probe` runs the hydration API under THIS module's identity,
        // which is the point of the rig — it stands in for worldgen, whose
        // `+build` calls the same API the same way. A write's identity is
        // whoever's `Ctx` carries it, so the three namespaces that API reaches
        // are declared here, by the writer, exactly as a real caller declares
        // its own.
        for ns in ["atlas", "content", "jobs"] {
            reg.persist_delegation(
                "probe",
                ns,
                "the test rig calls the atlas's hydration API under its own identity,                  standing in for worldgen's +build",
            )?;
        }
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "+probe".into(),
                shape: ArgShape::Raw,
                help: "+probe -- test: run the queued closure in a command context.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let job = ctx
                    .state::<ProbeQueue>()
                    .map(|q| Arc::clone(&q.0))
                    .and_then(|q| q.lock().expect("lock").pop());
                if let Some(job) = job {
                    job(ctx);
                }
                Ok(())
            }),
        )
    }
}

fn modules(nb: Pack) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    vec![
        Box::new(Probe),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(AtlasModule::with_config(AtlasConfig {
            hydration_cap: 2,
            hydration_strikes_to_zero: 3,
            ..AtlasConfig::default()
        })),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(vec![region(), nb]),
        ),
    ]
}

fn town() -> TestWorld {
    TestWorld::with_modules(modules(block(true, 4)))
}

fn nb_key() -> RecordKey {
    RecordKey::new(
        CanonicalId::new("neighborhood.block").expect("id"),
        CanonicalId::new("neighborhood").expect("id"),
    )
}

fn square(x: f64, y: f64, side: f64) -> Polygon2 {
    Polygon2(vec![
        mush_atlas::Point2(x, y),
        mush_atlas::Point2(x + side, y),
        mush_atlas::Point2(x + side, y + side),
        mush_atlas::Point2(x, y + side),
    ])
}

fn placement(kind: &str, footprint: Option<Polygon2>) -> Placement {
    Placement {
        kind: kind.to_string(),
        footprint,
        prose: "A quiet place with a bench.".to_string(),
    }
}

/// Ask through a command context (the API takes a `Ctx`).
fn ask(tw: &mut TestWorld, who: ObjId, footprint: Option<Polygon2>) -> Result<Requested, String> {
    ask_kind(tw, who, "site.park", footprint)
}

fn ask_kind(
    tw: &mut TestWorld,
    who: ObjId,
    kind: &str,
    footprint: Option<Polygon2>,
) -> Result<Requested, String> {
    let kind = kind.to_string();
    with_ctx(tw, who, move |ctx| {
        hydrate::request(ctx, who, &nb_key(), placement(&kind, footprint))
    })
}

/// Run `f` inside a command context for `actor` and hand its value back.
fn with_ctx<T: Send + 'static>(
    tw: &mut TestWorld,
    actor: ObjId,
    f: impl FnOnce(&mut Ctx<'_>) -> T + Send + 'static,
) -> T {
    let slot: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&slot);
    let queue = tw
        .world()
        .module_state::<ProbeQueue>()
        .expect("the probe module")
        .clone();
    queue.0.lock().expect("lock").push(Box::new(move |ctx| {
        *sink.lock().expect("lock") = Some(f(ctx));
    }));
    tw.run(actor, "+probe");
    let value = slot.lock().expect("lock").take();
    value.expect("the probe ran")
}

fn text(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn atlas(tw: &TestWorld) -> &AtlasState {
    tw.world().module_state::<AtlasState>().expect("atlas")
}

fn request_of(tw: &TestWorld, id: &str) -> RequestRec {
    atlas(tw)
        .hydration
        .requests
        .get(id)
        .cloned()
        .expect("the request")
}

/// The planner's pack for a request: one site of the request's kind over its
/// footprint, one room on it, a walk from the Green.
fn site_pack(rec: &RequestRec, name: &str, version: u32) -> Pack {
    let bbox = rec.footprint.to_cm().expect("cm").bbox();
    let (x0, y0) = (
        mush_atlas::geom::metres(bbox.min.x),
        mush_atlas::geom::metres(bbox.min.y),
    );
    // `PolygonCm::bbox()` is max-exclusive (+1 cm); subtract 1 cm to match
    // `rec.footprint`'s actual upper corner so the room stays inside the site.
    let (x1, y1) = (
        mush_atlas::geom::metres(bbox.max.x - 1),
        mush_atlas::geom::metres(bbox.max.y - 1),
    );
    signed(json!({
        "manifest": { "id": rec.pack_id().as_str(), "version": version, "kind": "site", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 },
                                   { "id": "neighborhood.block", "version": 1 }] },
        "entries": {
            "site.plot": { "kind": "site", "def": { "name": name, "site_kind": rec.kind,
                "footprint": rec.footprint } },
            "room.plot": { "kind": "room", "def": { "name": name, "surface": "ground",
                "site": "site.plot", "footprint": { "rect": [[x0, y0], [x1, y1]] },
                "desc": "A bench under a tree." } },
            "exit.green_plot": { "kind": "exit", "def": { "from": "neighborhood.block.room.green",
                "to": "room.plot", "names": [format!("{name} <P>;plot"), "Green <G>;g"],
                "portal": { "distance_m": 40 } } }
        }
    }))
}

fn activate(
    tw: &mut TestWorld,
    who: ObjId,
    id: &str,
    pack: &Pack,
) -> Result<hydrate::Activation, String> {
    let id = id.to_string();
    let pack = pack.clone();
    with_ctx(tw, who, move |ctx| hydrate::activate(ctx, &id, &pack))
}

fn canon_kinds(tw: &TestWorld, request: &str) -> Vec<String> {
    let state = tw.world().module_state::<CanonState>().expect("canon");
    let mut kinds: Vec<String> =
        mush_canon::records_for_source(state, &format!("atlas:request:{request}"), 32)
            .into_iter()
            .map(|e| e.kind.to_string())
            .collect();
    kinds.sort();
    kinds
}

fn site_named(tw: &TestWorld, name: &str) -> Option<mush_atlas::SiteRec> {
    atlas(tw)
        .records
        .sites
        .values()
        .find(|s| s.name == name)
        .cloned()
}

#[test]
fn a_request_reserves_its_slot_its_unit_and_its_ground_and_converges_on_itself() {
    let mut tw = town();
    let rae = tw.spawn_player("Rae");
    let bob = tw.spawn_player("Bob");

    let a = ask(&mut tw, rae, Some(square(100.0, 100.0, 20.0))).expect("a park");
    let Requested::New(a) = a else {
        panic!("a new request: {a:?}");
    };
    assert_eq!(a.status, RequestStatus::Generating);
    assert!(a.id.starts_with("hyd:"), "{}", a.id);
    // Again, the same ask: the same request, nothing charged twice.
    let again = ask(&mut tw, rae, Some(square(100.0, 100.0, 20.0))).expect("converges");
    assert!(
        matches!(&again, Requested::Converged(r) if r.id == a.id),
        "{again:?}"
    );
    // A second site, then the cap: the third is refused naming the standing.
    let b = ask(&mut tw, rae, Some(square(150.0, 100.0, 20.0))).expect("a second");
    assert!(matches!(b, Requested::New(_)));
    let refused = ask(&mut tw, rae, Some(square(100.0, 150.0, 20.0))).expect_err("the cap");
    assert!(
        refused.contains("Your standing allows 2 outstanding requests"),
        "{refused}"
    );
    assert!(refused.contains(&a.id), "{refused}");
    // Bob may not build over Rae's ground; the reply names the request.
    let refused = ask(&mut tw, bob, Some(square(110.0, 110.0, 20.0))).expect_err("spoken for");
    assert!(
        refused.contains("spoken for") && refused.contains(&a.id),
        "{refused}"
    );
    // Nor over the Green.
    let refused = ask(&mut tw, bob, Some(square(20.0, 20.0, 20.0))).expect_err("the green");
    assert!(refused.contains("under The Green"), "{refused}");
    // Nor past the edge.
    let refused = ask(&mut tw, bob, Some(square(190.0, 190.0, 20.0))).expect_err("the edge");
    assert!(refused.contains("leaves The Block"), "{refused}");
    // Withdrawing the second frees a slot.
    let b_id = b.record().id.clone();
    let withdrawn = with_ctx(&mut tw, rae, {
        let b_id = b_id.clone();
        move |ctx| hydrate::withdraw(ctx, &b_id, rae, "changed my mind")
    });
    assert!(withdrawn.is_ok(), "{withdrawn:?}");
    assert!(matches!(
        request_of(&tw, &b_id).status,
        RequestStatus::Withdrawn { .. }
    ));
    let c = ask(&mut tw, rae, Some(square(100.0, 150.0, 20.0))).expect("room again");
    assert!(matches!(c, Requested::New(_)));
    // Bob may not withdraw Rae's request; staff may.
    let c_id = c.record().id.clone();
    let err = with_ctx(&mut tw, bob, {
        let c_id = c_id.clone();
        move |ctx| hydrate::withdraw(ctx, &c_id, bob, "")
    })
    .expect_err("not his");
    assert!(err.contains("not yours"), "{err}");
    let staff = with_ctx(&mut tw, GOD, {
        let c_id = c_id.clone();
        move |ctx| hydrate::withdraw(ctx, &c_id, GOD, "staff")
    });
    assert!(staff.is_ok(), "{staff:?}");
    // No footprint: unclaimed ground is found inside the block, clear of the Green
    // and of Rae's park.
    let free = ask(&mut tw, bob, None).expect("free ground");
    let Requested::New(free) = free else {
        panic!("new: {free:?}");
    };
    let bbox = free.footprint.to_cm().expect("cm").bbox();
    assert!(
        bbox.min.x >= 0 && bbox.max.x <= 20_000 && bbox.max.y <= 20_000,
        "{bbox:?}"
    );
    // The ids differ by character and footprint, and the documents are written.
    assert_ne!(free.id, a.id);
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(|op| matches!(&op.op, ModuleWrite::JsonSet { key, .. } if key.as_str() == format!("request:{}", a.id))));
}

#[test]
fn activation_marks_the_site_hydrated_files_a_build_ticket_and_review_settles_it() {
    let mut tw = town();
    let rae = tw.spawn_player("Rae");
    let a = ask(&mut tw, rae, Some(square(100.0, 100.0, 20.0))).expect("a park");
    let a = a.record().clone();
    // A pack of the wrong kind, or one that does not require the block, is refused.
    let wrong = block(true, 2);
    let err = activate(&mut tw, rae, &a.id, &wrong).expect_err("not a site pack");
    assert!(err.contains("site pack"), "{err}");

    let pack = site_pack(&a, "Rae's Park", 1);
    let made = activate(&mut tw, rae, &a.id, &pack).expect("activates");
    assert!(made.job.is_some(), "{made:?}");
    assert_eq!(made.created, 3);
    let site = site_named(&tw, "Rae's Park").expect("the site stands");
    assert_eq!(
        site.origin,
        mush_atlas::record::SiteOrigin::Hydrated {
            request: a.id.clone()
        }
    );
    assert_eq!(site.review, mush_atlas::record::ReviewState::PendingReview);
    assert!(
        tw.world()
            .iter()
            .any(|o| o.kind == ObjKind::Room && o.name == "Rae's Park"),
        "the room is playable at once"
    );
    let rec = request_of(&tw, &a.id);
    assert!(
        matches!(rec.status, RequestStatus::Activated { .. }),
        "{rec:?}"
    );
    assert_eq!(rec.pack, Some(a.pack_id()));
    assert_eq!(
        canon_kinds(&tw, &a.id),
        vec!["atlas.site.activated".to_string()]
    );
    // The ticket carries the request, the digest and the neighborhood.
    let job_id = made.job.expect("job");
    let job = mush_jobs::get_job(tw.world(), job_id)
        .expect("the ticket")
        .clone();
    assert_eq!(job.queue.as_str(), "BUILD");
    let payload = job.payload.expect("payload");
    assert_eq!(payload.handler.as_str(), "build.review");
    assert_eq!(payload.data["request"], json!(a.id));
    assert_eq!(
        payload.data["pack_digest"],
        json!(pack.manifest().digest().to_hex())
    );
    assert_eq!(payload.data["neighborhood"], json!(nb_key().to_string()));
    // The slot is still held: two more asks and the cap bites.
    let _b = ask(&mut tw, rae, Some(square(150.0, 100.0, 20.0))).expect("second");
    let refused = ask(&mut tw, rae, Some(square(100.0, 150.0, 20.0))).expect_err("cap");
    assert!(refused.contains("Your standing allows"), "{refused}");
    // Activating it twice is refused: it is not waiting to be built.
    let err = activate(&mut tw, rae, &a.id, &pack).expect_err("already built");
    assert!(err.contains("pending review"), "{err}");

    // Approval keeps the site and releases the slot.
    let out = tw.run(GOD, &format!("+job/approve {job_id}"));
    assert!(
        text(&tw, &out, GOD).contains("approved"),
        "{}",
        text(&tw, &out, GOD)
    );
    let site = site_named(&tw, "Rae's Park").expect("still stands");
    assert_eq!(site.review, mush_atlas::record::ReviewState::Approved);
    assert!(matches!(
        request_of(&tw, &a.id).status,
        RequestStatus::Approved { .. }
    ));
    // WS03-A1: the approval runs OUR handler under the JOBS module's identity,
    // so those two record writes cross a namespace. A refused write still
    // mutates RAM — the two assertions above would pass either way — and only a
    // reboot would show the site had never left PendingReview in the store.
    let refused: Vec<String> = tw
        .world()
        .refused_namespace_writes()
        .into_iter()
        .map(|(module, ns, _)| format!("{module} -> {ns}"))
        .collect();
    assert!(
        refused.is_empty(),
        "the BUILD route made cross-namespace write(s) that did not land: {refused:?}. \
         Declare the pair with Registrar::persist_delegation and pin its row in \
         module-ns-owners.toml."
    );
    assert_eq!(
        canon_kinds(&tw, &a.id),
        vec![
            "atlas.site.activated".to_string(),
            "atlas.site.approved".to_string()
        ]
    );
    let c = ask(&mut tw, rae, Some(square(100.0, 150.0, 20.0))).expect("a slot again");
    let c = c.record().clone();

    // Changes requested: the site stays live, the slot is held.
    let made = activate(&mut tw, rae, &c.id, &site_pack(&c, "Rae's Garden", 1)).expect("activates");
    let job_id = made.job.expect("job");
    let out = tw.run(GOD, &format!("+job/deny {job_id}=needs a gate"));
    assert!(
        text(&tw, &out, GOD).contains("denied"),
        "{}",
        text(&tw, &out, GOD)
    );
    let site = site_named(&tw, "Rae's Garden").expect("still stands");
    assert_eq!(site.review, mush_atlas::record::ReviewState::NeedsChanges);
    assert!(matches!(
        request_of(&tw, &c.id).status,
        RequestStatus::NeedsChanges { .. }
    ));
    assert_eq!(
        canon_kinds(&tw, &c.id),
        vec![
            "atlas.site.activated".to_string(),
            "atlas.site.changes_requested".to_string()
        ]
    );
    let refused = ask(&mut tw, rae, Some(square(150.0, 150.0, 20.0))).expect_err("cap holds");
    assert!(refused.contains("Your standing allows"), "{refused}");
    // The player's standing says so.
    let out = tw.run(GOD, "@atlas/standing Rae");
    let listing = text(&tw, &out, GOD);
    assert!(
        listing.contains("2 of 2 request slots in use, 0 strikes"),
        "{listing}"
    );
    assert!(listing.contains("changes requested"), "{listing}");
}

#[test]
fn three_strikes_zero_the_cap_until_staff_reset_it() {
    let mut tw = town();
    let rae = tw.spawn_player("Rae");
    let mut ids = Vec::new();
    for (n, x) in [100.0, 130.0, 160.0].into_iter().enumerate() {
        let r = ask(&mut tw, rae, Some(square(x, 100.0, 20.0))).expect("a park");
        let r = r.record().clone();
        let name = format!("Park {n}");
        activate(&mut tw, rae, &r.id, &site_pack(&r, &name, 1)).expect("activates");
        assert!(site_named(&tw, &name).is_some());
        let out = tw.run(GOD, &format!("@atlas/strike {name}=an eyesore"));
        let said = text(&tw, &out, GOD);
        assert!(said.contains("Struck"), "{said}");
        assert!(said.contains(&format!("{} strike", n + 1)), "{said}");
        assert!(site_named(&tw, &name).is_none(), "the pack was retired");
        // A destroyed object is a GOING husk until the world task's sweep.
        assert!(
            !tw.world().iter().any(|o| o.kind == ObjKind::Room
                && o.name == name
                && !o.flags.contains(mush_core::Flags::GOING)),
            "the room is gone"
        );
        assert!(matches!(
            request_of(&tw, &r.id).status,
            RequestStatus::Struck { .. }
        ));
        let kinds = canon_kinds(&tw, &r.id);
        assert!(
            kinds.contains(&"atlas.site.struck".to_string()),
            "{kinds:?}"
        );
        assert!(
            kinds.contains(&"atlas.site.retired".to_string()),
            "{kinds:?}"
        );
        ids.push(r.id);
    }
    let out = tw.run(GOD, "@atlas/standing Rae");
    let listing = text(&tw, &out, GOD);
    assert!(
        listing.contains("0 of 0 request slots in use, 3 strikes — no new requests"),
        "{listing}"
    );
    let refused = ask(&mut tw, rae, Some(square(100.0, 150.0, 20.0))).expect_err("zeroed");
    assert!(
        refused.contains("no requests until staff reset it"),
        "{refused}"
    );
    // Staff reset the cap.
    let out = tw.run(GOD, "@atlas/cap Rae=2");
    assert!(
        text(&tw, &out, GOD).contains("Rae may hold 2 outstanding requests"),
        "{}",
        text(&tw, &out, GOD)
    );
    let again = ask(&mut tw, rae, Some(square(100.0, 150.0, 20.0))).expect("allowed again");
    assert!(matches!(again, Requested::New(_)));
    // A staff-authored site cannot be struck; nor a site nobody built.
    let out = tw.run(GOD, "@atlas/strike The Block=nope");
    assert!(
        text(&tw, &out, GOD).contains("A strike names a site"),
        "{}",
        text(&tw, &out, GOD)
    );
    let out = tw.run(GOD, "@atlas/strike Nowhere=nope");
    assert!(
        text(&tw, &out, GOD).contains("No site called"),
        "{}",
        text(&tw, &out, GOD)
    );
}

#[test]
fn the_neighborhood_allowance_and_a_closed_policy_refuse() {
    let mut tw = TestWorld::with_modules(modules(block(true, 1)));
    let rae = tw.spawn_player("Rae");
    let bob = tw.spawn_player("Bob");
    let _a = ask(&mut tw, rae, Some(square(100.0, 100.0, 20.0))).expect("the one");
    let refused = ask(&mut tw, bob, Some(square(150.0, 100.0, 20.0))).expect_err("full");
    assert!(
        refused.contains("holds its allowance of 1 player-built site"),
        "{refused}"
    );

    let mut tw = TestWorld::with_modules(modules(block(false, 2)));
    let rae = tw.spawn_player("Rae");
    let refused = ask(&mut tw, rae, Some(square(100.0, 100.0, 20.0))).expect_err("closed");
    assert!(
        refused.contains("does not take player-built sites"),
        "{refused}"
    );
    let refused =
        ask_kind(&mut tw, rae, "", Some(square(100.0, 100.0, 20.0))).expect_err("no kind");
    assert!(
        refused.contains("does not take") || refused.contains("Name a site kind"),
        "{refused}"
    );
}

#[test]
fn propose_reaches_the_registered_planner_and_says_when_there_is_none() {
    let mut tw = town();
    let out = tw.run(GOD, "@atlas/propose A bakery on the corner.");
    assert!(
        text(&tw, &out, GOD).contains("No planner is composed"),
        "{}",
        text(&tw, &out, GOD)
    );
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    hydrate::register_proposer(
        tw.world_mut(),
        Arc::new(move |_ctx, req| {
            sink.lock()
                .expect("lock")
                .push(format!("{:?} {:?}", req.scope, req.source));
            Ok("Planning it.".to_string())
        }),
    )
    .expect("registers");
    let out = tw.run(GOD, "@atlas/propose The Block=A bakery on the corner.");
    assert!(
        text(&tw, &out, GOD).contains("Planning it."),
        "{}",
        text(&tw, &out, GOD)
    );
    let out = tw.run(GOD, "@atlas/propose --from neighborhood.block");
    assert!(
        text(&tw, &out, GOD).contains("is not on the shelf"),
        "{}",
        text(&tw, &out, GOD)
    );
    let calls = seen.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].contains("Neighborhood") && calls[0].contains("bakery"),
        "{calls:?}"
    );
    // A second planner is refused.
    assert!(
        hydrate::register_proposer(tw.world_mut(), Arc::new(|_, _| Ok(String::new()))).is_err()
    );
    // Not staff: refused.
    let rae = tw.spawn_player("Rae");
    let out = tw.run(rae, "@atlas/propose A bakery.");
    assert!(
        text(&tw, &out, rae).contains("Permission denied"),
        "{}",
        text(&tw, &out, rae)
    );
}

#[test]
fn requests_and_standing_survive_a_reboot() {
    let mut tw = town();
    let rae = tw.spawn_player("Rae");
    let a = ask(&mut tw, rae, Some(square(100.0, 100.0, 20.0))).expect("a park");
    let a = a.record().clone();
    activate(&mut tw, rae, &a.id, &site_pack(&a, "Rae's Park", 1)).expect("activates");
    tw.run(GOD, "@atlas/strike Rae's Park=an eyesore");
    let b = ask(&mut tw, rae, Some(square(150.0, 100.0, 20.0))).expect("another");
    let b = b.record().clone();

    // A restart: fold the persisted ops into boot data and hydrate every module.
    let ops = tw.take_module_ops();
    let mut boot: HashMap<SmolStr, ModuleBootData> = HashMap::new();
    for op in ops {
        let data = boot.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                data.kv.retain(|(k, _)| *k != key);
                data.kv.push((key, value));
            }
            ModuleWrite::Del { key } => data.kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    tw.modules_loaded(boot);
    let st = atlas(&tw);
    assert!(matches!(
        st.hydration.requests.get(&a.id).map(|r| &r.status),
        Some(RequestStatus::Struck { .. })
    ));
    assert_eq!(
        st.hydration.requests.get(&b.id).map(|r| &r.status),
        Some(&RequestStatus::Generating)
    );
    assert_eq!(st.hydration.standing.get(&rae).map(|s| s.strikes), Some(1));
    assert_eq!(hydrate::cap_of(st, rae), 2);
    // The content latch remembers the retired pack, so nothing comes back.
    let content = tw.world().module_state::<ContentState>().expect("content");
    assert!(
        content
            .latches
            .applied(&a.pack_id())
            .is_some_and(mush_content::Applied::is_retired)
    );
}

/// HIGH-39: `hydrate::activate` re-validates `rec.footprint` and full
/// `player_build` rules before touching the world via `apply_pack`.
#[test]
fn high39_activate_revalidates_footprint_and_player_build_rules() {
    let mut tw = town();
    let rae = tw.spawn_player("Rae");
    let rec = ask(&mut tw, rae, Some(square(100.0, 100.0, 20.0)))
        .expect("park")
        .record()
        .clone();

    // 1. Pack whose room/site escapes `rec.footprint` is refused.
    let oversized = signed(json!({
        "manifest": { "id": rec.pack_id().as_str(), "version": 1, "kind": "site", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 },
                                   { "id": "neighborhood.block", "version": 1 }] },
        "entries": {
            "site.plot": { "kind": "site", "def": { "name": "Too Big", "site_kind": rec.kind,
                "footprint": [[100, 100], [150, 100], [150, 150], [100, 150]] } },
            "room.plot": { "kind": "room", "def": { "name": "Too Big", "surface": "ground",
                "site": "site.plot", "footprint": { "rect": [[100, 100], [150, 150]] },
                "desc": "Sprawls past its lot." } },
            "exit.green_plot": { "kind": "exit", "def": { "from": "neighborhood.block.room.green",
                "to": "room.plot", "names": ["Big <B>;big", "Green <G>;g"],
                "portal": { "distance_m": 40 } } }
        }
    }));
    let err = activate(&mut tw, rae, &rec.id, &oversized).expect_err("oversized must fail");
    assert!(
        err.contains("leaves the reserved request footprint"),
        "{err}"
    );

    // 2. Pack carrying prohibited player_build features (Limbo exit, staff flag) is refused.
    let bad_limbo = signed(json!({
        "manifest": { "id": rec.pack_id().as_str(), "version": 1, "kind": "site", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 },
                                   { "id": "neighborhood.block", "version": 1 }] },
        "entries": {
            "site.plot": { "kind": "site", "def": { "name": "Sneaky", "site_kind": rec.kind,
                "footprint": rec.footprint } },
            "room.plot": { "kind": "room", "def": { "name": "Sneaky", "surface": "ground",
                "site": "site.plot", "footprint": { "rect": [[100, 100], [120, 120]] },
                "desc": "A room with staff flags.", "flags": ["safe"] } },
            "exit.green_plot": { "kind": "exit", "def": { "from": "neighborhood.block.room.green",
                "to": "room.plot", "names": ["Sneaky <S>;s", "Green <G>;g"],
                "portal": { "distance_m": 40 } } },
            "exit.limbo_plot": { "kind": "exit", "def": { "from": "limbo",
                "to": "room.plot", "names": ["Backdoor <B>;b", "Limbo <L>;l"] } }
        }
    }));
    let err = activate(&mut tw, rae, &rec.id, &bad_limbo).expect_err("bad player_build must fail");
    assert!(
        err.contains("player-build validation")
            && (err.contains("policy.limbo") || err.contains("policy.flag")),
        "{err}"
    );
    assert!(
        site_named(&tw, "Sneaky").is_none(),
        "refused pack must leave no sites in the world"
    );

    // 3. Valid pack still activates cleanly.
    let valid = site_pack(&rec, "Rae's Valid Park", 1);
    activate(&mut tw, rae, &rec.id, &valid).expect("valid pack activates");
    assert!(site_named(&tw, "Rae's Valid Park").is_some());
}

/// STRAG-M9 (atlas): withdrawn requests are pruned past
/// `MAX_RETAINED_TERMINAL_REQUESTS` (on live withdraw and at boot), and
/// non-staff players face sliding-window request/withdrawal rate limits and
/// withdrawal cooldowns across restarts.
#[test]
fn strag_m9_hydration_request_pruning_and_player_rate_limits() {
    let mut tw = town();
    // 1. Staff request+withdraw churn beyond MAX_RETAINED_TERMINAL_REQUESTS stays bounded.
    let total = mush_atlas::MAX_RETAINED_TERMINAL_REQUESTS + 20;
    for i in 0..total {
        let x = 100.0 + ((i % 4) as f64) * 22.0;
        let y = 100.0 + (((i / 4) % 4) as f64) * 22.0;
        let req = ask_kind(
            &mut tw,
            GOD,
            &format!("site.kind_{i}"),
            Some(square(x, y, 20.0)),
        )
        .expect("staff ask")
        .record()
        .clone();
        let id = req.id.clone();
        with_ctx(&mut tw, GOD, move |ctx| {
            hydrate::withdraw(ctx, &id, GOD, "testing prune")
        })
        .expect("staff withdraw");
    }
    assert!(
        atlas(&tw).hydration.requests.len() <= mush_atlas::MAX_RETAINED_TERMINAL_REQUESTS,
        "terminal requests should prune to <= {}, got {}",
        mush_atlas::MAX_RETAINED_TERMINAL_REQUESTS,
        atlas(&tw).hydration.requests.len()
    );

    // 2. Boot hydration with > MAX_REQUESTS withdrawn docs + 1 open doc keeps the open doc
    //    and drops excess withdrawn docs for disk deletion.
    let mut synthetic_kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    let sample = atlas(&tw)
        .hydration
        .requests
        .values()
        .next()
        .expect("sample")
        .clone();
    for i in 0..(mush_atlas::MAX_REQUESTS + 10) {
        let mut r = sample.clone();
        r.id = format!("withdrawn-{i:05}");
        r.updated = i64::try_from(i).unwrap_or(0);
        r.status = RequestStatus::Withdrawn {
            reason: "churn".to_string(),
        };
        synthetic_kv.push((
            hydrate::request_key(&r.id),
            serde_json::to_value(&r).expect("json"),
        ));
    }
    let mut open_rec = sample.clone();
    open_rec.id = "zzz-live-request".to_string();
    open_rec.status = RequestStatus::Generating;
    synthetic_kv.push((
        hydrate::request_key(&open_rec.id),
        serde_json::to_value(&open_rec).expect("json"),
    ));
    let (hydrated, dropped) = hydrate::hydrate(&synthetic_kv);
    assert!(
        hydrated.requests.contains_key("zzz-live-request"),
        "open request must survive > MAX_REQUESTS withdrawn records at boot"
    );
    assert_eq!(
        hydrated.requests.len(),
        mush_atlas::MAX_RETAINED_TERMINAL_REQUESTS + 1
    );
    assert_eq!(
        dropped.len(),
        mush_atlas::MAX_REQUESTS + 10 - mush_atlas::MAX_RETAINED_TERMINAL_REQUESTS
    );

    // 3. Non-staff player withdrawal cooldown + withdrawal cap + hourly request cap.
    let spammer = tw.spawn_player("Spammer");
    for i in 0..2 {
        let req = ask_kind(
            &mut tw,
            spammer,
            &format!("site.s{i}"),
            Some(square(100.0, 100.0, 20.0)),
        )
        .expect("ask")
        .record()
        .clone();
        let id = req.id.clone();
        with_ctx(&mut tw, spammer, move |ctx| {
            hydrate::withdraw(ctx, &id, spammer, "nope")
        })
        .expect("withdraw");
    }
    // After 2 withdrawals at t=0, immediate 3rd request hits WITHDRAW_COOLDOWN_SECS.
    let cooled = ask_kind(
        &mut tw,
        spammer,
        "site.s2",
        Some(square(100.0, 100.0, 20.0)),
    )
    .expect_err("withdrawal cooldown must fire");
    assert!(cooled.contains("after withdrawing"), "{cooled}");

    // Advance past cooldown for withdrawals 3 and 4 -> 5th withdrawal inside 1h is refused.
    let mut now = tw.world().now();
    for i in 2..mush_atlas::MAX_WITHDRAWS_PER_PLAYER_WINDOW {
        now += mush_atlas::WITHDRAW_COOLDOWN_SECS + 1;
        tw.set_now(now);
        let req = ask_kind(
            &mut tw,
            spammer,
            &format!("site.s{i}"),
            Some(square(100.0, 100.0, 20.0)),
        )
        .expect("ask after cooldown")
        .record()
        .clone();
        let id = req.id.clone();
        with_ctx(&mut tw, spammer, move |ctx| {
            hydrate::withdraw(ctx, &id, spammer, "nope")
        })
        .expect("withdraw");
    }
    now += mush_atlas::WITHDRAW_COOLDOWN_SECS + 1;
    tw.set_now(now);
    let extra = ask_kind(
        &mut tw,
        spammer,
        "site.extra",
        Some(square(100.0, 100.0, 20.0)),
    )
    .expect("ask")
    .record()
    .clone();
    let extra_id = extra.id.clone();
    let wd_cap = with_ctx(&mut tw, spammer, move |ctx| {
        hydrate::withdraw(ctx, &extra_id, spammer, "too many")
    })
    .expect_err("withdrawal hourly cap must fire");
    assert!(wd_cap.contains("withdrawn"), "{wd_cap}");
}

#[test]
fn ext_p2_15_concave_neighborhood_and_reservation_reject_spanning_buildings() {
    // 1. Concave L-shaped neighborhood: [0,200]x[0,80] U [0,80]x[80,200].
    let l_block = signed(json!({
        "manifest": { "id": "neighborhood.block", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "L Block",
                "setting": "suburban",
                "footprint": [[0, 0], [200, 0], [200, 80], [80, 80], [80, 200], [0, 200]],
                "room_budget": { "target": 3, "maximum": 12 },
                "hydration": { "enabled": true, "max_player_sites": 4 } } },
            "room.green": { "kind": "room", "def": { "name": "The Green", "surface": "ground",
                "footprint": { "rect": [[10, 10], [50, 50]] }, "desc": "A wide green." } },
            "exit.limbo_green": { "kind": "exit", "def": { "from": "limbo", "to": "room.green",
                "names": ["Out <O>;o;out", "Limbo <L>;limbo"] } }
        }
    }));

    let mut tw = TestWorld::with_modules(modules(l_block));
    let alice = tw.spawn_player("Alice");

    // A triangle whose vertices all sit inside L Block, but whose hypotenuse spans the exterior
    // notch (80..200, 80..200): must be rejected at request time (`hydrate.rs:764`)!
    let spanning_nb = Polygon2(vec![
        mush_atlas::Point2(60.0, 60.0),
        mush_atlas::Point2(180.0, 60.0),
        mush_atlas::Point2(60.0, 180.0),
    ]);
    let err = ask_kind(&mut tw, alice, "site.park", Some(spanning_nb))
        .expect_err("spanning footprint must not pass concave neighborhood containment");
    assert!(err.contains("leaves L Block"), "{err}");

    // 2. Valid U-shaped reservation strictly inside the horizontal arm [80,190]x[10,70].
    let u_reservation = Polygon2(vec![
        mush_atlas::Point2(90.0, 10.0),
        mush_atlas::Point2(190.0, 10.0),
        mush_atlas::Point2(190.0, 70.0),
        mush_atlas::Point2(160.0, 70.0),
        mush_atlas::Point2(160.0, 30.0),
        mush_atlas::Point2(120.0, 30.0),
        mush_atlas::Point2(120.0, 70.0),
        mush_atlas::Point2(90.0, 70.0),
    ]);
    let rec = ask_kind(&mut tw, alice, "site.park", Some(u_reservation.clone()))
        .expect("valid concave U reservation inside L Block succeeds")
        .record()
        .clone();

    // A hydration pack whose site footprint uses the U's bounding box (all 4 vertices on the
    // reservation boundary, bridging across the open mouth [120,160]x[30,70]) must be rejected!
    let bad_pack = signed(json!({
        "manifest": { "id": rec.pack_id().as_str(), "version": 1, "kind": "site", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 },
                                   { "id": "neighborhood.block", "version": 1 }] },
        "entries": {
            "site.plot": { "kind": "site", "def": {
                "name": "Spanning Park", "site_kind": rec.kind,
                "footprint": [[90, 10], [190, 10], [190, 70], [90, 70]] } },
            "room.lawn": { "kind": "room", "def": {
                "name": "Lawn", "surface": "ground", "site": "site.plot",
                "footprint": { "rect": [[95, 15], [115, 65]] }, "desc": "Grass." } },
            "exit.walk": { "kind": "exit", "def": { "from": "neighborhood.block.room.green",
                "to": "room.lawn", "names": ["Park <P>;park", "Green <G>;green"],
                "portal": { "distance_m": 40 } } }
        }
    }));
    let req_id = rec.id.clone();
    let bad_res = with_ctx(&mut tw, GOD, move |ctx| {
        hydrate::activate(ctx, &req_id, &bad_pack)
    });
    assert!(
        bad_res.is_err(),
        "building spanning concave reservation mouth must be rejected"
    );
    let bad_err = bad_res.unwrap_err();
    assert!(
        bad_err.contains("leaves the reserved request footprint"),
        "{bad_err}"
    );

    // A hydration pack whose site footprint stays within the U-shaped reservation (sharing
    // boundary walls!) succeeds!
    let good_pack = signed(json!({
        "manifest": { "id": rec.pack_id().as_str(), "version": 1, "kind": "site", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 },
                                   { "id": "neighborhood.block", "version": 1 }] },
        "entries": {
            "site.plot": { "kind": "site", "def": {
                "name": "Courtyard Park", "site_kind": rec.kind,
                "footprint": [[90, 10], [190, 10], [190, 70], [160, 70], [160, 30], [120, 30], [120, 70], [90, 70]] } },
            "room.lawn": { "kind": "room", "def": {
                "name": "West Wing", "surface": "ground", "site": "site.plot",
                "footprint": { "rect": [[90, 10], [120, 70]] }, "desc": "A courtyard wing." } },
            "exit.walk": { "kind": "exit", "def": { "from": "neighborhood.block.room.green",
                "to": "room.lawn", "names": ["Park <P>;park", "Green <G>;green"],
                "portal": { "distance_m": 40 } } }
        }
    }));
    let req_id2 = rec.id.clone();
    let good_res = with_ctx(&mut tw, GOD, move |ctx| {
        hydrate::activate(ctx, &req_id2, &good_pack)
    });
    assert!(
        good_res.is_ok(),
        "valid boundary-touching site inside concave reservation must succeed: {good_res:?}"
    );
}

/// P2-MOD-33 & P2-MOD-72:
/// - `Access::Restricted` rooms are masked in hydration overlap errors and failed
///   probes consume per-player request rate-limit budget.
/// - `hydrate::free_ground` validates full polygon containment against concave
///   neighborhoods rather than vertices only.
#[test]
fn p2_mod_33_and_72_restricted_room_masking_rate_limit_and_concave_free_ground() {
    // 1. Restricted room name masking + rate-limiting on failed overlap probes.
    let secret_block = signed(json!({
        "manifest": { "id": "neighborhood.block", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "Secret Block",
                "setting": "suburban",
                "footprint": [[0, 0], [200, 0], [200, 200], [0, 200]],
                "room_budget": { "target": 3, "maximum": 12 },
                "hydration": { "enabled": true, "max_player_sites": 4 } } },
            "room.green": { "kind": "room", "def": { "name": "The Green", "surface": "ground",
                "footprint": { "rect": [[10, 10], [40, 40]] }, "desc": "A wide green." } },
            "room.vault": { "kind": "room", "def": { "name": "Hidden Sanctum", "surface": "ground",
                "access": "restricted",
                "footprint": { "rect": [[60, 60], [100, 100]] }, "desc": "Secret vault." } },
            "exit.limbo_green": { "kind": "exit", "def": { "from": "limbo", "to": "room.green",
                "names": ["Out <O>;o;out", "Limbo <L>;limbo"] } }
        }
    }));
    let mut tw = TestWorld::with_modules(modules(secret_block));
    let probe = tw.spawn_player("Probe");
    tw.engine_mut().set_now(10_000);

    let err = ask(&mut tw, probe, Some(square(65.0, 65.0, 20.0)))
        .expect_err("overlapping restricted room must fail");
    assert!(
        !err.contains("Hidden Sanctum"),
        "restricted room name leaked in hydrate::request: {err}"
    );
    assert!(err.contains("not available"), "{err}");

    // Repeating failed overlap probes consumes the per-player rate limit.
    for i in 1..mush_atlas::MAX_REQUESTS_PER_PLAYER_WINDOW {
        let _ = ask(
            &mut tw,
            probe,
            Some(square(65.0 + f64::from(i as u32), 65.0, 10.0)),
        );
    }
    let throttled = ask(&mut tw, probe, Some(square(120.0, 120.0, 20.0)))
        .expect_err("rate limit must stop repeated probes");
    assert!(
        throttled.contains("limit"),
        "expected rate limit: {throttled}"
    );

    // 2. Concave notch neighborhood where the first SW grid square has all 4 corners
    //    inside the polygon but its middle crosses a concave notch: `free_ground` must
    //    skip it and select a genuinely contained square so auto-placement succeeds.
    // U-shaped bottom notch: outer [0, 80] x [0, 80] with a narrow notch cut up into
    // [10, 16] x [0, 30] from y=0..30; a 24m square starting at (0,0) has corners at
    // (0,0), (24,0), (24,24), (0,24) all inside/on the polygon while its bottom edge
    // bridges across the notch (10..16, 0..24)!
    let notched = signed(json!({
        "manifest": { "id": "neighborhood.block", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "Notched Block",
                "setting": "suburban",
                "footprint": [[0, 0], [10, 0], [10, 30], [16, 30], [16, 0], [80, 0], [80, 80], [0, 80]],
                "room_budget": { "target": 3, "maximum": 12 },
                "hydration": { "enabled": true, "max_player_sites": 4 } } },
            "room.green": { "kind": "room", "def": { "name": "The Green", "surface": "ground",
                "footprint": { "rect": [[0, 40], [20, 60]] }, "desc": "A wide green." } },
            "exit.limbo_green": { "kind": "exit", "def": { "from": "limbo", "to": "room.green",
                "names": ["Out <O>;o;out", "Limbo <L>;limbo"] } }
        }
    }));
    let mut tw2 = TestWorld::with_modules(modules(notched));
    let bob = tw2.spawn_player("Bob");
    let chosen = ask(&mut tw2, bob, None)
        .expect("free_ground must skip notch-bridging candidate and find valid ground");
    let fp = chosen.record().footprint.to_cm().expect("cm");
    let nb_poly = atlas(&tw2)
        .records
        .neighborhoods
        .get(&nb_key())
        .unwrap()
        .footprint
        .to_cm()
        .unwrap();
    // All 4 corners AND the centre of the selected square must lie inside the neighborhood.
    let bbox = fp.bbox();
    let centre = mush_atlas::PointCm {
        x: (bbox.min.x + bbox.max.x) / 2,
        y: (bbox.min.y + bbox.max.y) / 2,
    };
    assert!(
        nb_poly.contains(centre),
        "free_ground picked a notch-bridging square: {fp:?}"
    );
}
