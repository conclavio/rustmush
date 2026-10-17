//! **Doc 18 cut IC9 acceptance — frozen leg times, `depart`, and the carrier
//! hooks** (doc 18 §4.2 "The helm", §4.4, §4.8; D-18-46; lead rulings L3, L8,
//! L28 and reviewer findings F2, F11 of the IC9 brief).
//!
//! A journey may carry the passage planner's laden seconds per leg
//! (`CarrierJourney::with_leg_times`): every boundary, the arrival and
//! `@journey/show` then read them in place of the legs' authored `base_s`, and the
//! strict wire re-runs the invariant (a stored count mismatch or a zero is HELD at
//! boot). `mush_sim::depart` is `@journey/depart`'s body after the route is
//! resolved, dated from a caller's `departed_at`; `depart_ineligible` is its own
//! first half, so a caller can dry-run every refusal before it spends anything.
//! `CarrierHooks` is the composition seam a game layer narrates and refuses
//! through: live boundaries and first arrivals only — never at boot, never on the
//! crash-gap heal re-apply.
//!
//! The rig is `members.rs`'s: the real `Engine` via `TestWorld` with the
//! `SimulationModule` (and scenes) installed, one ordered op log folded for a
//! reboot.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mush_core::{
    Audience, BehaviorBinding, BehaviorId, CanonicalId, Ctx, Deferred, GOD, LIMBO,
    ModuleActionState, ModuleBootData, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox, ParsedArgs,
    SwitchSet, World,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use mush_sim::{
    CarrierHooks, CarrierJourney, JourneyBook, LegDef, RouteDef, SimulationModule, SpatialParent,
    SpatialState, Transition,
};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- the rig -------------------------------------------------------------

/// What the test hooks saw and how they answer.
#[derive(Default)]
struct Probe {
    /// `boundary`/`arrived` answer this (suppress the generic line).
    suppress: AtomicBool,
    /// `refuse_manual_departure` refuses while set.
    refuse: AtomicBool,
    boundaries: AtomicUsize,
    arrivals: AtomicUsize,
    /// The route id `refuse_manual_departure` was handed.
    seen_route: Mutex<Option<String>>,
    /// The rooms the hooks narrate into.
    rooms: Mutex<Vec<ObjId>>,
}

struct Hooks(Arc<Probe>);

fn hook_line(ctx: &mut Ctx<'_>, rooms: &[ObjId], subject: ObjId, text: &str) {
    for room in rooms {
        ctx.out.to_room_from(
            *room,
            Vec::new(),
            subject,
            OutputEvent::Narrative {
                kind: NarrativeKind::Emit,
                actor: None,
                text: RichText::new().text(text.to_string()),
            },
        );
    }
}

impl CarrierHooks for Hooks {
    fn refuse_manual_departure(
        &self,
        _w: &World,
        _carrier: ObjId,
        route: &RouteDef,
    ) -> Option<String> {
        *self.0.seen_route.lock().expect("lock") = Some(route.id().to_string());
        self.0
            .refuse
            .load(Ordering::SeqCst)
            .then(|| format!("Hooks refuse {}.", route.id()))
    }

    fn boundary(&self, ctx: &mut Ctx<'_>, journey: &CarrierJourney, reached: u32) -> bool {
        self.0.boundaries.fetch_add(1, Ordering::SeqCst);
        let rooms = self.0.rooms.lock().expect("lock").clone();
        hook_line(
            ctx,
            &rooms,
            journey.root(),
            &format!("Hooked boundary {reached}."),
        );
        self.0.suppress.load(Ordering::SeqCst)
    }

    fn arrived(&self, ctx: &mut Ctx<'_>, journey: &CarrierJourney) -> bool {
        self.0.arrivals.fetch_add(1, Ordering::SeqCst);
        let rooms = self.0.rooms.lock().expect("lock").clone();
        hook_line(ctx, &rooms, journey.root(), "Hooked arrival.");
        self.0.suppress.load(Ordering::SeqCst)
    }
}

fn boot_with(hooks: Option<Arc<Probe>>) -> TestWorld {
    let sim = match hooks {
        Some(probe) => SimulationModule::new().with_carrier_hooks(Arc::new(Hooks(probe))),
        None => SimulationModule::new(),
    };
    TestWorld::with_modules(vec![
        Box::new(sim),
        Box::new(mush_scenes::ScenesModule::new()),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

fn ok(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    let t = texts(&events);
    assert!(!has_error(&events), "{line:?} was refused: {t}");
    t
}

fn refused(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    let t = texts(&events);
    assert!(has_error(&events), "{line:?} was accepted: {t}");
    t
}

fn bind(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new(behavior).unwrap(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
}

fn state(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim state")
}

fn book(tw: &TestWorld) -> &JourneyBook {
    tw.world()
        .module_state::<JourneyBook>()
        .expect("journey book")
}

fn journey(tw: &TestWorld, root: ObjId) -> CarrierJourney {
    book(tw).get(root).cloned().expect("a journey")
}

fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).expect("id")
}

/// The fixture's objects, created in a fixed order so a second `TestWorld`
/// allocates the same ids.
#[derive(Clone, Copy)]
struct Fx {
    dock: ObjId,
    outside: ObjId,
    hull: ObjId,
    lock: ObjId,
    spine: ObjId,
    deck: ObjId,
    mid: ObjId,
    far: ObjId,
    apron: ObjId,
    bay: ObjId,
}

/// Objects: domain hosts `Dock` (presenting in `Outside`) and `Far`
/// (presenting in `Apron`), a carrier Thing `Hull`, its interior `Lock` with
/// member rooms `Spine` and `Deck`, a waypoint `Mid`, and `Bay`, a fixed
/// interior of `Dock`'s domain.
fn objects(tw: &mut TestWorld) -> Fx {
    let room = |tw: &mut TestWorld, name: &str| {
        tw.world_mut()
            .create(ObjKind::Room, name, GOD, None)
            .unwrap()
    };
    let dock = room(tw, "Dock");
    let outside = room(tw, "Outside");
    let hull = tw
        .world_mut()
        .create(ObjKind::Thing, "Hull", GOD, Some(LIMBO))
        .unwrap();
    let lock = room(tw, "Lock");
    let spine = room(tw, "Spine");
    let deck = room(tw, "Deck");
    let mid = room(tw, "Mid");
    let far = room(tw, "Far");
    let apron = room(tw, "Apron");
    let bay = room(tw, "Bay");
    for (from, name, to) in [
        (lock, "spine", spine),
        (spine, "lock", lock),
        (spine, "deck", deck),
        (deck, "spine", spine),
    ] {
        let e = tw
            .world_mut()
            .create(ObjKind::Exit, name, GOD, Some(from))
            .unwrap();
        tw.world_mut().link_exit(e, Some(to)).unwrap();
    }
    bind(tw, dock, "sim.domain");
    bind(tw, far, "sim.domain");
    bind(tw, hull, "sim.participant");
    bind(tw, lock, "sim.interior");
    bind(tw, bay, "sim.interior");
    Fx {
        dock,
        outside,
        hull,
        lock,
        spine,
        deck,
        mid,
        far,
        apron,
        bay,
    }
}

/// The records: domain #0 on `Dock`, domain #1 on `Far`, `Hull` placed in #0,
/// `Lock` riding `Hull` with members `{Lock, Spine, Deck}`, `Bay` fixed in #0.
fn records(tw: &mut TestWorld, fx: &Fx, log: &mut Vec<ModuleOp>) {
    ok(
        tw,
        &format!(
            "@domain/create #{}=sim.passive,#{}",
            fx.dock.0, fx.outside.0
        ),
    );
    ok(
        tw,
        &format!("@domain/create #{}=sim.passive,#{}", fx.far.0, fx.apron.0),
    );
    ok(tw, &format!("@spatial/place/domain #{}=0", fx.hull.0));
    ok(
        tw,
        &format!("@spatial/space #{}=moving:#{}", fx.lock.0, fx.hull.0),
    );
    ok(tw, &format!("@spatial/space #{}=fixed:0", fx.bay.0));
    drain(tw, log);
    let mut st = std::mem::take(
        tw.world_mut()
            .module_state_mut::<SpatialState>()
            .expect("sim state"),
    );
    let mut out = Outbox::new();
    let members = Transition::Members {
        space: fx.lock,
        rooms: [fx.lock, fx.spine, fx.deck].into_iter().collect(),
    };
    let result = mush_sim::plan(&st, tw.world(), &members)
        .and_then(|plan| mush_sim::apply(&mut st, tw.world_mut(), &mut out, plan));
    *tw.world_mut()
        .module_state_mut::<SpatialState>()
        .expect("sim state") = st;
    drain_out(&mut out, log);
    result.expect("members");
}

/// A fresh world at `now` with the scene built; the op log so far.
fn scene(hooks: Option<Arc<Probe>>, now: i64) -> (TestWorld, Fx, Vec<ModuleOp>) {
    let mut tw = boot_with(hooks);
    tw.set_now(now);
    let fx = objects(&mut tw);
    let mut log = Vec::new();
    records(&mut tw, &fx, &mut log);
    (tw, fx, log)
}

/// The two-leg corridor `Dock → Mid → Far` at 100 + 200 authored seconds.
fn corridor(fx: &Fx) -> RouteDef {
    RouteDef::new(
        cid("corridor"),
        (fx.dock, fx.far),
        vec![
            LegDef::new(fx.mid, 100).expect("leg"),
            LegDef::new(fx.far, 200).expect("leg"),
        ],
        cid("manual"),
        0,
    )
    .expect("route")
}

fn drain(tw: &mut TestWorld, log: &mut Vec<ModuleOp>) {
    log.extend(tw.take_module_ops());
}

fn drain_out(out: &mut Outbox, log: &mut Vec<ModuleOp>) {
    for deferred in out.take_deferred() {
        if let Deferred::Persist(op) = deferred {
            log.push(op);
        }
    }
}

/// Run `f` with a staff command context over the engine, logging what it
/// persisted and returning what it narrated.
fn with_ctx<R>(
    tw: &mut TestWorld,
    log: &mut Vec<ModuleOp>,
    f: impl FnOnce(&mut Ctx<'_>) -> R,
) -> (R, Vec<(Audience, OutputEvent)>) {
    drain(tw, log);
    let mut out = Outbox::new();
    let engine = tw.engine_mut();
    let result = {
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new("sim")),
            typed_state: ModuleActionState::unavailable(),
        };
        f(&mut ctx)
    };
    drain_out(&mut out, log);
    (result, out.take())
}

fn fold_kv_ops(log: &[ModuleOp]) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in log {
        if op.ns != "sim" {
            continue;
        }
        match &op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| k != key);
                kv.push((key.clone(), value.clone()));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn doc_mut<'a>(kv: &'a mut [(SmolStr, serde_json::Value)], key: &str) -> &'a mut serde_json::Value {
    &mut kv
        .iter_mut()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("no {key} document"))
        .1
}

/// A fresh world over the same objects at `now`, booted from `kv`.
fn reboot(
    kv: Vec<(SmolStr, serde_json::Value)>,
    hooks: Option<Arc<Probe>>,
    now: i64,
) -> (TestWorld, Fx) {
    let mut tw = boot_with(hooks);
    tw.set_now(now);
    let fx = objects(&mut tw);
    tw.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    (tw, fx)
}

fn cjourney_ops(log: &[ModuleOp]) -> usize {
    log.iter()
        .filter(|op| match &op.op {
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => {
                key.starts_with("cjourney:")
            }
            ModuleWrite::StreamAppend { .. } => false,
        })
        .count()
}

/// A player standing in `room` (placed in the lock's space when aboard).
fn person(tw: &mut TestWorld, name: &str, room: ObjId, aboard: Option<ObjId>) -> ObjId {
    let who = tw.spawn_player(name);
    bind(tw, who, "sim.participant");
    match aboard {
        Some(space) => {
            ok(
                tw,
                &format!("@spatial/place/interior #{}=#{}", who.0, space.0),
            );
            if room != space {
                tw.world_mut().move_to(who, room).expect("walks");
            }
        }
        None => tw.world_mut().move_to(who, room).expect("stands"),
    }
    who
}

fn heard(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

// ---- 1. the boundaries are the frozen seconds ----------------------------

/// **A journey flown on leg times reaches each boundary at its own seconds**:
/// `leg_due_at`, the cursor, the arrival and `@journey/show` all read the
/// frozen times, never the authored `base_s`.
#[test]
fn ic9_a_journey_with_leg_times_reaches_each_boundary_at_its_own_seconds() {
    let (mut tw, fx, mut log) = scene(None, 1_000);
    let route = corridor(&fx);
    let (departure, _) = with_ctx(&mut tw, &mut log, |ctx| {
        mush_sim::depart(ctx, fx.hull, route, Some(vec![300, 500]), 1_000)
    });
    let departure = departure.expect("departs");
    assert_eq!(departure.arrival_at, Some(1_800));
    let j = journey(&tw, fx.hull);
    assert_eq!(j.leg_times(), Some(&[300u32, 500][..]));
    assert_eq!(j.leg_secs(0), Some(300));
    assert_eq!(j.leg_secs(1), Some(500));
    assert_eq!(j.leg_secs(2), None);
    assert_eq!(j.leg_due_at(0), Some(1_300));
    assert_eq!(j.leg_due_at(1), Some(1_800));
    assert_eq!(j.arrival_at(), Some(1_800));

    let show = ok(&mut tw, &format!("@journey/show #{}", fx.hull.0));
    assert!(show.contains("leg 1 → Mid (300s, IC 1300)"), "{show}");
    assert!(show.contains("leg 2 → Far (500s, IC 1800)"), "{show}");

    // The authored 100 s boundary passes nothing.
    tw.set_now(1_101);
    tw.fire_due(1_101 * 1000);
    assert_eq!(journey(&tw, fx.hull).cursor(), 0);
    tw.set_now(1_299);
    tw.fire_due(1_299 * 1000);
    assert_eq!(journey(&tw, fx.hull).cursor(), 0);
    tw.set_now(1_300);
    tw.fire_due(1_300 * 1000);
    assert_eq!(journey(&tw, fx.hull).cursor(), 1);
    tw.set_now(1_799);
    tw.fire_due(1_799 * 1000);
    assert!(journey(&tw, fx.hull).state().is_under_way());
    tw.set_now(1_800);
    tw.fire_due(1_800 * 1000);
    assert!(!journey(&tw, fx.hull).state().is_under_way());
    assert!(matches!(
        state(&tw).parent_of(fx.hull),
        SpatialParent::Direct { domain, .. } if domain.0 == 1
    ));
}

// ---- 2. the wire and a reboot ----------------------------------------------

/// **Leg times survive the wire and a reboot**; a stored count mismatch and a
/// zero are refused by `TryFrom` and HELD at boot (never deleted).
#[test]
fn ic9_leg_times_survive_the_wire_and_a_reboot() {
    let (mut tw, fx, mut log) = scene(None, 1_000);
    let route = corridor(&fx);
    let (departure, _) = with_ctx(&mut tw, &mut log, |ctx| {
        mush_sim::depart(ctx, fx.hull, route, Some(vec![300, 500]), 1_000)
    });
    departure.expect("departs");
    let j = journey(&tw, fx.hull);

    // The wire round-trips, and a journey without times carries no key.
    let encoded = serde_json::to_value(&j).expect("encodes");
    assert_eq!(encoded["leg_s"], serde_json::json!([300, 500]));
    let back: CarrierJourney = serde_json::from_value(encoded.clone()).expect("decodes");
    assert_eq!(back, j);
    let plain = CarrierJourney::new(
        j.id(),
        j.root(),
        j.route().clone(),
        j.destination().clone(),
        j.departed().clone(),
        j.advanced_at().clone(),
        0,
        j.state().clone(),
        None,
    )
    .expect("plain");
    let plain_wire = serde_json::to_value(&plain).expect("encodes");
    assert!(
        !plain_wire
            .as_object()
            .expect("object")
            .contains_key("leg_s")
    );

    // The constructor-side refusals, and the wire re-running them.
    assert_eq!(
        plain.clone().with_leg_times(vec![5]),
        Err(mush_sim::CarrierJourneyError::LegTimes { legs: 2, times: 1 })
    );
    assert_eq!(
        plain.clone().with_leg_times(vec![0, 5]),
        Err(mush_sim::CarrierJourneyError::ZeroLegTime)
    );
    let mut short = encoded.clone();
    short["leg_s"] = serde_json::json!([5]);
    let e = serde_json::from_value::<CarrierJourney>(short).expect_err("a count mismatch");
    assert!(
        e.to_string()
            .contains("a journey's leg times name every leg: 1 for 2"),
        "{e}"
    );
    let mut zero = encoded;
    zero["leg_s"] = serde_json::json!([0, 5]);
    let e = serde_json::from_value::<CarrierJourney>(zero).expect_err("a zero");
    assert!(
        e.to_string().contains("a journey leg time is positive"),
        "{e}"
    );

    // A reboot keeps them.
    drain(&mut tw, &mut log);
    let kv = fold_kv_ops(&log);
    let (tw2, fx2) = reboot(kv.clone(), None, 1_100);
    let again = journey(&tw2, fx2.hull);
    assert_eq!(again.leg_times(), Some(&[300u32, 500][..]));
    assert_eq!(again.leg_due_at(0), Some(1_300));
    assert_eq!(again.arrival_at(), Some(1_800));

    // A stored mismatch is HELD at boot, its key recorded, the raw value kept.
    let key = format!("cjourney:{}", fx.hull.0);
    for bad in [serde_json::json!([5]), serde_json::json!([0, 5])] {
        let mut kv_bad = kv.clone();
        doc_mut(&mut kv_bad, &key)["leg_s"] = bad.clone();
        let (tw3, fx3) = reboot(kv_bad, None, 1_100);
        assert!(
            book(&tw3).held.contains_key(key.as_str()),
            "{bad} was not held"
        );
        assert!(book(&tw3).get(fx3.hull).is_none());
    }
}

// ---- 3. depart dates the journey ------------------------------------------

/// **`depart` dates the journey from `departed_at`** on the current
/// generation; a future instant is refused; the authored arm's texts are
/// unchanged.
#[test]
fn ic9_depart_dates_the_journey_from_departed_at() {
    let (mut tw, fx, mut log) = scene(None, 1_000);
    let route = corridor(&fx);
    let (future, _) = with_ctx(&mut tw, &mut log, |ctx| {
        mush_sim::depart(ctx, fx.hull, route.clone(), None, 1_001)
    });
    assert_eq!(
        future.expect_err("the future").to_string(),
        "A journey cannot depart in the future."
    );
    assert!(book(&tw).get(fx.hull).is_none());
    assert_eq!(cjourney_ops(&log), 0);
    let (past, _) = with_ctx(&mut tw, &mut log, |ctx| {
        mush_sim::depart(ctx, fx.hull, route, None, 900)
    });
    let past = past.expect("a past instant departs");
    let j = journey(&tw, fx.hull);
    assert_eq!(j.departed().at, 900);
    assert_eq!(j.departed().clock, tw.world().world_ic_now().clock);
    assert_eq!(j.leg_due_at(0), Some(1_000));
    assert_eq!(past.arrival_at, Some(1_200));
    assert_eq!(past.journey, j.id());
    assert!(past.aboard >= 2, "the hull and its lock rode the move");

    // The manual staff arm on a second world: its info line is unchanged.
    let (mut tw2, fx2, _) = scene(None, 1_000);
    let line = ok(
        &mut tw2,
        &format!(
            "@journey/depart #{}=corridor #{}:100 #{}:200",
            fx2.hull.0, fx2.mid.0, fx2.far.0
        ),
    );
    assert!(
        line.contains("Hull departs on journey #0: 2 leg(s), 2 record(s) aboard, due at IC 1300."),
        "{line}"
    );
    assert_eq!(journey(&tw2, fx2.hull).departed().at, 1_000);
}

// ---- 4. depart_ineligible is depart's own refusal ---------------------------

/// **`depart_ineligible` answers exactly what `depart` refuses with** — a
/// paused destination domain, a carrier already under way and a wrong origin —
/// and `depart` then writes nothing.
#[test]
fn ic9_depart_ineligible_is_departs_own_refusal() {
    let (mut tw, fx, mut log) = scene(None, 1_000);
    let check = |tw: &mut TestWorld, log: &mut Vec<ModuleOp>, route: RouteDef, want: &str| {
        drain(tw, log);
        let before = book(tw).clone();
        let mark = log.len();
        let (pair, events) = with_ctx(tw, log, |ctx| {
            let dry = mush_sim::depart_ineligible(ctx, fx.hull, &route);
            let wet = mush_sim::depart(ctx, fx.hull, route.clone(), Some(vec![10, 20]), 1_000);
            (dry, wet)
        });
        let (dry, wet) = pair;
        assert_eq!(dry.as_deref(), Some(want));
        assert_eq!(wet.expect_err("refused").to_string(), want);
        assert_eq!(log.len(), mark, "a refusal persists nothing");
        assert!(events.is_empty(), "a refusal narrates nothing");
        assert_eq!(book(tw), &before, "a refusal mutates no journey state");
    };

    // A destination domain that is not Active.
    ok(&mut tw, "@domain/pause 1");
    check(
        &mut tw,
        &mut log,
        corridor(&fx),
        "Domain #1 is Paused and accepts no arrivals.",
    );
    ok(&mut tw, "@domain/resume 1");

    // A wrong origin: the route sets out from Far, the hull is at Dock.
    let backwards = RouteDef::new(
        cid("backwards"),
        (fx.far, fx.dock),
        vec![LegDef::new(fx.dock, 100).expect("leg")],
        cid("manual"),
        0,
    )
    .expect("route");
    check(&mut tw, &mut log, backwards, mush_sim::ROUTE_NOT_FROM_HERE);

    // Under way already.
    let (first, _) = with_ctx(&mut tw, &mut log, |ctx| {
        mush_sim::depart(ctx, fx.hull, corridor(&fx), None, 1_000)
    });
    first.expect("the first departure");
    check(
        &mut tw,
        &mut log,
        corridor(&fx),
        "Hull is already under way (@journey/divert to re-plan it).",
    );
    // Nothing refused eligibility answers None.
    let (tw2, fx2, _) = scene(None, 1_000);
    let mut tw2 = tw2;
    let mut log2 = Vec::new();
    let (none, _) = with_ctx(&mut tw2, &mut log2, |ctx| {
        mush_sim::depart_ineligible(ctx, fx2.hull, &corridor(&fx2))
    });
    assert_eq!(none, None);
}

// ---- 5. the carrier hooks --------------------------------------------------

/// **The carrier hooks narrate and refuse the manual departure**: answering
/// `false`, the hook's line and the generic aboard lines are both heard;
/// answering `true`, only the hook's, the outside arrival line still present;
/// `refuse_manual_departure` sees the resolved route and refuses GOD; a boot
/// catch-up never calls the hooks; a crash-gap heal re-apply does not call
/// `arrived`.
#[test]
fn ic9_carrier_hooks_narrate_and_refuse_manual_departure() {
    let depart_line = |fx: &Fx| {
        format!(
            "@journey/depart #{}=corridor #{}:100 #{}:200",
            fx.hull.0, fx.mid.0, fx.far.0
        )
    };

    // (a) Kept: both lines.
    let probe = Arc::new(Probe::default());
    let (mut tw, fx, _log) = scene(Some(probe.clone()), 1_000);
    *probe.rooms.lock().expect("lock") = vec![fx.lock, fx.spine, fx.deck];
    let ann = person(&mut tw, "Ann", fx.deck, Some(fx.lock));
    let bob = person(&mut tw, "Bob", fx.apron, None);
    ok(&mut tw, &depart_line(&fx));
    assert_eq!(
        probe.seen_route.lock().expect("lock").as_deref(),
        Some("corridor")
    );
    tw.set_now(1_100);
    let events = tw.fire_due(1_100 * 1000);
    let ann_heard = heard(&tw, &events, ann);
    assert!(ann_heard.contains("Hooked boundary 1."), "{ann_heard}");
    assert!(
        ann_heard.contains("Hull passes Mid. Leg 2 of 2."),
        "{ann_heard}"
    );
    assert_eq!(probe.boundaries.load(Ordering::SeqCst), 1);
    tw.set_now(1_300);
    let events = tw.fire_due(1_300 * 1000);
    let ann_heard = heard(&tw, &events, ann);
    assert!(ann_heard.contains("Hooked arrival."), "{ann_heard}");
    assert!(
        ann_heard.contains("Hull arrives, and the passage ends."),
        "{ann_heard}"
    );
    let bob_heard = heard(&tw, &events, bob);
    assert!(
        bob_heard.contains("Hull arrives, and the passage ends."),
        "{bob_heard}"
    );
    assert_eq!(probe.arrivals.load(Ordering::SeqCst), 1);

    // (e) The crash-gap heal re-apply (on a live tick) does not call `arrived`:
    // the carrier is relocated by staff and the arrived record's repair claim is
    // re-opened in RAM, so the next fire re-applies the arrival.
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", fx.hull.0));
    let reopened = journey(&tw, fx.hull).with_applied_revision(u64::MAX);
    tw.world_mut()
        .module_state_mut::<JourneyBook>()
        .expect("journey book")
        .journeys
        .insert(fx.hull, reopened);
    tw.set_now(1_301);
    let events = tw.fire_due(1_301 * 1000);
    assert!(
        matches!(
            state(&tw).parent_of(fx.hull),
            SpatialParent::Direct { domain, .. } if domain.0 == 1
        ),
        "the heal ran"
    );
    assert_eq!(
        probe.arrivals.load(Ordering::SeqCst),
        1,
        "the heal re-apply called arrived"
    );
    assert!(!heard(&tw, &events, ann).contains("Hooked arrival."));

    // (b) Suppressed: only the hook's line aboard; the outside line stays.
    let probe = Arc::new(Probe::default());
    probe.suppress.store(true, Ordering::SeqCst);
    let (mut tw, fx, _log) = scene(Some(probe.clone()), 1_000);
    *probe.rooms.lock().expect("lock") = vec![fx.lock, fx.spine, fx.deck];
    let ann = person(&mut tw, "Ann", fx.deck, Some(fx.lock));
    let bob = person(&mut tw, "Bob", fx.apron, None);
    ok(&mut tw, &depart_line(&fx));
    tw.set_now(1_100);
    let events = tw.fire_due(1_100 * 1000);
    let ann_heard = heard(&tw, &events, ann);
    assert!(ann_heard.contains("Hooked boundary 1."), "{ann_heard}");
    assert!(!ann_heard.contains("passes"), "{ann_heard}");
    tw.set_now(1_300);
    let events = tw.fire_due(1_300 * 1000);
    let ann_heard = heard(&tw, &events, ann);
    assert!(ann_heard.contains("Hooked arrival."), "{ann_heard}");
    assert!(!ann_heard.contains("the passage ends"), "{ann_heard}");
    let bob_heard = heard(&tw, &events, bob);
    assert!(
        bob_heard.contains("Hull arrives, and the passage ends."),
        "{bob_heard}"
    );

    // (c) Refused: the hook sees the resolved route and refuses GOD; nothing
    // is written.
    let probe = Arc::new(Probe::default());
    probe.refuse.store(true, Ordering::SeqCst);
    let (mut tw, fx, mut log) = scene(Some(probe.clone()), 1_000);
    drain(&mut tw, &mut log);
    let mark = log.len();
    let t = refused(&mut tw, &depart_line(&fx));
    assert!(t.contains("Hooks refuse corridor."), "{t}");
    assert_eq!(
        probe.seen_route.lock().expect("lock").as_deref(),
        Some("corridor")
    );
    drain(&mut tw, &mut log);
    assert_eq!(cjourney_ops(log.get(mark..).unwrap_or_default()), 0);
    assert!(book(&tw).get(fx.hull).is_none());
    // `depart` itself never consults the manual-departure hook.
    let (direct, _) = with_ctx(&mut tw, &mut log, |ctx| {
        mush_sim::depart(ctx, fx.hull, corridor(&fx), None, 1_000)
    });
    direct.expect("depart is the planned path; the hook guards only the manual arm");

    // (d) A boot catch-up uses the generic line and never calls the hooks.
    let probe = Arc::new(Probe::default());
    let (mut tw, fx, mut log) = scene(Some(probe.clone()), 1_000);
    ok(&mut tw, &depart_line(&fx));
    drain(&mut tw, &mut log);
    let kv = fold_kv_ops(&log);
    let probe2 = Arc::new(Probe::default());
    let (tw2, fx2) = reboot(kv, Some(probe2.clone()), 2_000);
    assert!(
        matches!(
            state(&tw2).parent_of(fx2.hull),
            SpatialParent::Direct { domain, .. } if domain.0 == 1
        ),
        "boot caught the passage up"
    );
    assert_eq!(probe2.boundaries.load(Ordering::SeqCst), 0);
    assert_eq!(probe2.arrivals.load(Ordering::SeqCst), 0);
}

// ---- 6. anchored_space_of ----------------------------------------------------

/// **`anchored_space_of` names moving interiors only**: a member room and the
/// space object of a space riding a hull answer that space; a fixed interior,
/// a domain host and a room in no space answer `None`.
#[test]
fn ic9_anchored_space_of_names_moving_interiors_only() {
    let (tw, fx, _log) = scene(None, 1_000);
    let w = tw.world();
    assert_eq!(mush_sim::anchored_space_of(w, fx.lock), Some(fx.lock));
    assert_eq!(mush_sim::anchored_space_of(w, fx.spine), Some(fx.lock));
    assert_eq!(mush_sim::anchored_space_of(w, fx.deck), Some(fx.lock));
    assert_eq!(mush_sim::anchored_space_of(w, fx.bay), None);
    assert_eq!(mush_sim::anchored_space_of(w, fx.dock), None);
    assert_eq!(mush_sim::anchored_space_of(w, fx.mid), None);
    assert_eq!(mush_sim::anchored_space_of(w, fx.outside), None);
}
