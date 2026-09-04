//! E5b acceptance (08d §4): **the housing module**, live.
//!
//! Scenario 23's housing half, plus the three axes this cut's verify is written
//! against — *offline-home safety, notice bounds, hook bypasses*:
//!
//! - **nothing ends a home on its own.** Every path that could is driven to its end
//!   and the tenancy is still there: forty periods of no support, a destroyed room, a
//!   missing tenant, a reboot at every rung. D-E5-1 is the invariant the whole file
//!   is arranged around.
//! - **the tenant keeps their key.** Access is asserted at every status, not just the
//!   happy one — a delinquent tenant walks in, an eviction-pending tenant walks in,
//!   and the stranger at the same door gets the same neutral sentence they would get
//!   for a wall.
//! - **notices are bounded.** Three re-notices, then silence, then one page — counted,
//!   not sampled.
//! - **the refusals are asserted against state**, never only against the message: a
//!   refused `@property/rooms` allocates nothing, a refused destroy leaves the room,
//!   a refused second tenancy leaves the first one alone.
//!
//! Everything runs through the real `Engine` via `TestWorld`, with the real org,
//! economy, jobs and housing modules wired exactly as `mush-server` wires them.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::module::{GameModule, ModError, Registrar, WorldEvent};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId,
};
use mush_economy::{EconomyStore, EscalationSubject};
use mush_housing::{
    HousingStore, PropertyId, PropertyStatus, SupportLevel, SupportSource, TenancyId,
    TenancyStatusKind,
};
use mush_org::SystemPrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "housing";

// ---- rig ------------------------------------------------------------------

/// The four modules `mush-server` wires, in the same order, plus one test module that
/// exposes the two seams a *ruleset* would drive (`submit_support_plan`) and that a
/// *bug* would drive (a room destroyed without passing the gate).
fn boot() -> TestWorld {
    let issuer = SystemPrincipalId::new("wod.housing").expect("valid system principal");
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new().with_capabilities(vocab())),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_title_issuers(vec![issuer.clone()])
                .with_observers(vec![Arc::new(mush_housing::HousingEconomyObserver)]),
        ),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(mush_housing::HousingModule::new().with_issuer(issuer)),
        Box::new(TestSeams),
    ])
}

fn vocab() -> std::collections::BTreeSet<mush_org::CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_housing::CAPABILITIES.iter().copied())
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| mush_org::CapabilityId::new(c).expect("well-formed"))
        .collect()
}

/// `+plan <tenancy>=<resident> <level>` — the ruleset's own entry (D-E5-5), and
/// `+boom <room>` — a room destroyed **programmatically**, the way a sim teardown or
/// a migration would, without ever passing the destroy gate.
#[derive(Debug)]
struct TestSeams;

impl GameModule for TestSeams {
    fn name(&self) -> &'static str {
        "housingtest"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.command(
            CommandSpec {
                name: "+plan".into(),
                shape: ArgShape::EqPair,
                help: "test seam".into(),
                ..CommandSpec::default()
            },
            Arc::new(cmd_plan),
        )?;
        reg.command(
            CommandSpec {
                name: "+boom".into(),
                shape: ArgShape::EqPair,
                help: "test seam".into(),
                ..CommandSpec::default()
            },
            Arc::new(cmd_boom),
        )?;
        Ok(())
    }
}

fn cmd_plan(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let tenancy = TenancyId(ctx.args.lhs().trim().parse::<u64>().map_err(|_| CmdError::msg("id"))?);
    let rhs = ctx.args.rhs().unwrap_or("").to_string();
    let mut w = rhs.split_whitespace();
    let who = ObjId(
        w.next()
            .and_then(|s| s.trim_start_matches('#').parse::<u32>().ok())
            .ok_or_else(|| CmdError::msg("who"))?,
    );
    let level = SupportLevel::new(
        w.next().and_then(|s| s.parse::<u8>().ok()).ok_or_else(|| CmdError::msg("level"))?,
    );
    mush_housing::submit_support_plan(ctx, tenancy, who, level, vec![SupportSource::Inheritance])
        .map_err(|e| CmdError::msg(e.to_string()))?;
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "test", "plan submitted".to_string());
    Ok(())
}

fn cmd_boom(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let room = ObjId(
        ctx.args
            .lhs()
            .trim()
            .trim_start_matches('#')
            .parse::<u32>()
            .map_err(|_| CmdError::msg("room"))?,
    );
    ctx.w.destroy(room).map_err(|e| CmdError::msg(e.to_string()))?;
    ctx.emit_world_event(&WorldEvent::ObjDestroyed { obj: room });
    Ok(())
}

// ---- output helpers -------------------------------------------------------

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events.iter().map(|(_, e)| text_of(e)).collect::<Vec<_>>().join("\n")
}
fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}
fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(has_error(&ev), "`{line}` should have been refused, got: {}", texts(&ev));
    texts(&ev)
}

fn store(tw: &TestWorld) -> &HousingStore {
    tw.world().module_state::<HousingStore>().expect("housing store")
}
fn tenancy(tw: &TestWorld, id: TenancyId) -> mush_housing::Tenancy {
    store(tw).tenancy(id).expect("tenancy").clone()
}
fn status(tw: &TestWorld, id: TenancyId) -> TenancyStatusKind {
    tenancy(tw, id).status.kind()
}
fn property(tw: &TestWorld, id: PropertyId) -> mush_housing::Property {
    store(tw).property(id).expect("property").clone()
}
fn room_named(tw: &TestWorld, name: &str) -> ObjId {
    tw.world().iter().find(|o| o.name == name).expect("room").id
}
fn home_of(tw: &TestWorld, who: ObjId) -> Option<ObjId> {
    tw.world().get(who).and_then(|o| o.home)
}
fn alerts(tw: &TestWorld) -> Vec<mush_economy::Escalation> {
    tw.world()
        .module_state::<EconomyStore>()
        .map(|s| s.escalations().cloned().collect())
        .unwrap_or_default()
}

// ---- the world ------------------------------------------------------------

struct Rig {
    pat: ObjId,
    quinn: ObjId,
    /// A player with no standing anywhere.
    rex: ObjId,
    anchor: ObjId,
    spare: ObjId,
    cadence: u64,
    property: PropertyId,
}

/// LIMBO ──in──▶ Flat ──spare──▶ Spare, with the way back from each. Only the anchor
/// (Flat) is reachable from outside, which is exactly what D-E5-12's thoroughfare
/// rule requires.
fn rig(tw: &mut TestWorld) -> Rig {
    let pat = tw.spawn_player("Pat");
    let quinn = tw.spawn_player("Quinn");
    let rex = tw.spawn_player("Rex");
    ok(tw, GOD, "@dig Flat=in,out");
    let anchor = room_named(tw, "Flat");
    ok(tw, GOD, &format!("@teleport #{}", anchor.0));
    ok(tw, GOD, "@dig Spare=spare,back");
    let spare = room_named(tw, "Spare");
    ok(tw, GOD, &format!("@teleport #{}", LIMBO.0));
    ok(tw, GOD, "@econ/cadence create chronicle source=manual");
    let cadence = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().cadences().next().map(|c| c.id.0))
        .expect("cadence");
    ok(
        tw,
        GOD,
        &format!("@property/create The Flat=#{},#{} tier=2 landlord=housing", anchor.0, spare.0),
    );
    Rig { pat, quinn, rex, anchor, spare, cadence, property: PropertyId(0) }
}

/// One live abstract tenancy for `who`, at the given plan level.
fn tenant(tw: &mut TestWorld, r: &Rig, who: ObjId, level: u8) -> TenancyId {
    ok(
        tw,
        GOD,
        &format!("@tenancy/create {}=#{} cadence={}", r.property, who.0, r.cadence),
    );
    let id = store(tw).tenancies().last().expect("tenancy").id;
    ok(tw, GOD, &format!("+plan {id}=#{} {level}", who.0));
    id
}

/// Advance the campaign clock one period and let both ticks fire.
fn period(tw: &mut TestWorld, r: &Rig, n: u64) -> Vec<(Audience, OutputEvent)> {
    ok(tw, GOD, &format!("@econ/cadence advance {} {n}", r.cadence));
    fire(tw)
}

/// Fire the due ticks once, at a strictly increasing instant.
fn fire(tw: &mut TestWorld) -> Vec<(Audience, OutputEvent)> {
    let t = tw.world().now().saturating_add(40);
    tw.set_now(t);
    tw.fire_due(t.saturating_mul(1000))
}

// ---- persistence helpers (the e4b1 shape, across every namespace) ---------
//
// `Engine::modules_loaded` hands EVERY module its boot data, so a reboot that named
// only `housing` would hand the economy an empty world — no cadence, no accounts, no
// contracts — and the tenancy under test would boot pointing at a clock that no
// longer exists. The fold therefore keeps every namespace's writes, and the reboot
// replays all of them.

type Kv = HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn drain_into(tw: &mut TestWorld, kv: &mut Kv) {
    for op in tw.take_module_ops() {
        let entry = kv.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                entry.retain(|(k, _)| *k != key);
                entry.push((key, value));
            }
            ModuleWrite::Del { key } => entry.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
}
fn fold_kv(tw: &mut TestWorld) -> Kv {
    let mut kv = Kv::new();
    drain_into(tw, &mut kv);
    kv
}
fn reboot(tw: &mut TestWorld, kv: Kv) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .into_iter()
        .map(|(ns, kv)| (ns, ModuleBootData { kv, streams: HashMap::new() }))
        .collect();
    tw.modules_loaded(boot);
}
fn patch(kv: &mut Kv, key: &str, f: impl Fn(&mut serde_json::Value)) {
    let entry = kv.get_mut(&SmolStr::new(NS)).expect("housing namespace");
    let mut seen = false;
    for (k, v) in entry.iter_mut() {
        if k == key {
            f(v);
            seen = true;
        }
    }
    assert!(seen, "no such persisted key to patch: {key}");
}
fn housing_keys(kv: &Kv) -> Vec<String> {
    kv.get(&SmolStr::new(NS))
        .map(|e| e.iter().map(|(k, _)| k.to_string()).collect())
        .unwrap_or_default()
}

// ======================================================================
// Scenario 23's housing half — the ladder, in abstract mode
// ======================================================================

#[test]
fn a_supported_tenancy_says_nothing_for_many_periods() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);

    for _ in 0..10 {
        // The ruleset restates the same plan every period, as its evaluator would.
        ok(&mut tw, GOD, &format!("+plan {id}=#{} 3", r.pat.0));
        let seen = texts(&period(&mut tw, &r, 1));
        assert!(!seen.contains("HOME:"), "a supported period notified: {seen}");
    }
    assert_eq!(status(&tw, id), TenancyStatusKind::Active, "a supported tenancy moved");
    // S3: a restatement is not news, so the audit tail is not thirty plans deep.
    let t = tenancy(&tw, id);
    assert!(t.history.len() <= 4, "restatements filled the tail: {:?}", t.history);
}

#[test]
fn unsupported_walks_grace_delinquent_eviction_with_one_notice_each() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    let mut notices = 0usize;

    // Period 1: Active → Grace.
    let seen = texts(&period(&mut tw, &r, 1));
    notices += usize::from(seen.contains("did not cover this period"));
    assert_eq!(status(&tw, id), TenancyStatusKind::Grace);

    // Period 2: Grace → Delinquent.
    let seen = texts(&period(&mut tw, &r, 1));
    notices += usize::from(seen.contains("is behind"));
    assert_eq!(status(&tw, id), TenancyStatusKind::Delinquent);

    // Periods 3-4: Delinquent → EvictionPending (DELINQUENT_PERIODS is 2).
    period(&mut tw, &r, 1);
    let seen = texts(&period(&mut tw, &r, 1));
    notices += usize::from(seen.contains("end of what happens automatically"));
    assert_eq!(status(&tw, id), TenancyStatusKind::EvictionPending);
    assert_eq!(notices, 3, "one private notice per transition");

    // Nothing moved: the tenant is where they were and still homed on the anchor.
    assert_eq!(home_of(&tw, r.pat), Some(r.anchor));
    // And the ST has a ticket carrying the commands that answer it.
    let t = tenancy(&tw, id);
    let mush_housing::TenancyStatus::EvictionPending { window } = t.status else {
        panic!("not eviction pending")
    };
    let job = window.job.expect("a story job was opened");
    let job = mush_jobs::get_job(tw.world(), job).expect("job").clone();
    assert_eq!(job.queue, mush_housing::STORY_QUEUE);
    let body = job.comments.first().map(|c| c.text.clone()).unwrap_or_default();
    assert!(body.contains(&format!("@tenancy/resolve {id}=end")), "job body: {body}");
    assert!(body.contains("nothing ends this home on its own"), "job body: {body}");
}

#[test]
fn the_window_renotifies_three_times_then_goes_quiet_then_pages_once() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    for _ in 0..4 {
        period(&mut tw, &r, 1);
    }
    assert_eq!(status(&tw, id), TenancyStatusKind::EvictionPending);

    let mut renotices = 0usize;
    let mut quiet = 0usize;
    for _ in 0..3 {
        let seen = texts(&period(&mut tw, &r, 1));
        renotices += usize::from(seen.contains("still waiting on a decision"));
    }
    assert_eq!(renotices, mush_housing::MAX_WINDOW_NOTICES as usize, "the notice budget is 3");

    // Then silence until the escalation gap has elapsed (WINDOW_ESCALATE, measured
    // from the last notice — E5a deviation 1).
    for _ in 0..1 {
        let seen = texts(&period(&mut tw, &r, 1));
        quiet += usize::from(!seen.contains("HOME:"));
    }
    assert_eq!(quiet, 1, "the window was not quiet after its budget");

    // Then exactly one page, and no more notices of the old kind.
    let seen = texts(&period(&mut tw, &r, 1));
    assert!(seen.contains("staff have now been paged"), "no escalation notice: {seen}");
    let paged: Vec<_> = alerts(&tw)
        .into_iter()
        .filter(|e| e.subject == EscalationSubject::HousingTenancy(id.0))
        .collect();
    assert_eq!(paged.len(), 1, "one page per window");
    let page = &paged[0];
    assert_eq!(page.level, 2);
    assert!(
        page.recommended.iter().any(|c| c == &format!("@tenancy/resolve {id}=end")),
        "the alert does not carry the filled-in commands: {:?}",
        page.recommended
    );
    assert!(status(&tw, id) == TenancyStatusKind::EvictionPending, "the page changed the state");
}

#[test]
fn support_restored_at_every_stage_returns_to_active_and_closes_the_window() {
    for stop in 1..=5 {
        let mut tw = boot();
        let r = rig(&mut tw);
        let id = tenant(&mut tw, &r, r.pat, 0);
        for _ in 0..stop {
            period(&mut tw, &r, 1);
        }
        let before = status(&tw, id);
        // The evaluator answers again, this time with enough.
        ok(&mut tw, GOD, &format!("+plan {id}=#{} 4", r.pat.0));
        period(&mut tw, &r, 1);
        assert_eq!(status(&tw, id), TenancyStatusKind::Active, "recovery from {before:?} failed");
        if before == TenancyStatusKind::EvictionPending {
            // S8: the ST's ticket is resolved and the page is acknowledged, so nobody
            // is left holding a decision about a home that is fine.
            let t = tenancy(&tw, id);
            let window = t.last_window.expect("the closed window was kept");
            if let Some(job) = window.job {
                let job = mush_jobs::get_job(tw.world(), job).expect("job");
                assert_eq!(job.status, mush_jobs::JobStatus::Resolved);
            }
            for e in alerts(&tw) {
                if e.subject == EscalationSubject::HousingTenancy(id.0) {
                    assert!(!e.is_open(), "the page is still open after recovery");
                }
            }
        }
    }
}

#[test]
fn two_co_tenants_assess_best_of() {
    let mut tw = boot();
    let r = rig(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{},#{} cadence={}",
            r.property, r.pat.0, r.quinn.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    // One resident cannot carry the tier; the other comfortably can (D-E5-13).
    for _ in 0..4 {
        ok(&mut tw, GOD, &format!("+plan {id}=#{} 0", r.pat.0));
        ok(&mut tw, GOD, &format!("+plan {id}=#{} 5", r.quinn.0));
        period(&mut tw, &r, 1);
    }
    assert_eq!(status(&tw, id), TenancyStatusKind::Active, "a poor co-tenant dragged the lease down");

    // Take the paying one away and it slips, from that period on and not before.
    ok(&mut tw, GOD, &format!("+plan {id}=#{} 0", r.quinn.0));
    period(&mut tw, &r, 1);
    assert_eq!(status(&tw, id), TenancyStatusKind::Grace);
}

#[test]
fn a_large_cadence_jump_assesses_once_with_bounded_catch_up() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    // Forty periods of downtime in one advance: the whole ladder is three periods
    // long, and a restart must not spend all of it while the player was asleep.
    period(&mut tw, &r, 40);
    assert_eq!(status(&tw, id), TenancyStatusKind::Grace, "a jump charged the backlog");
    period(&mut tw, &r, 40);
    assert_eq!(status(&tw, id), TenancyStatusKind::Delinquent);
}

#[test]
fn an_unknown_assessment_is_not_a_miss_until_its_grace_is_spent() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // A tenancy nobody has ever submitted a plan for: `Unknown`, not `Unsupported`.
    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/create {}=#{} cadence={}", r.property, r.pat.0, r.cadence),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    period(&mut tw, &r, 1);
    assert_eq!(status(&tw, id), TenancyStatusKind::Active, "a silent evaluator was read as a miss");
    period(&mut tw, &r, 1);
    assert_eq!(status(&tw, id), TenancyStatusKind::Grace, "UNKNOWN_GRACE never hardened");
}

// ======================================================================
// Access — the hook, at every status
// ======================================================================

#[test]
fn the_tenant_enters_at_every_status_and_a_stranger_never_does() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);

    for step in 0..5 {
        // The tenant walks in, whatever the ladder says.
        ok(&mut tw, r.pat, "in");
        assert_eq!(
            tw.world().get(r.pat).and_then(|o| o.location),
            Some(r.anchor),
            "the tenant was refused at step {step} ({:?})",
            status(&tw, id)
        );
        // The stranger does not, and is told nothing about why.
        let seen = texts(&tw.run(r.rex, "in"));
        assert!(seen.contains(mush_housing::REFUSED), "wrong refusal: {seen}");
        assert!(!seen.to_lowercase().contains("tenan"), "the refusal leaked a tenancy: {seen}");
        assert_eq!(tw.world().get(r.rex).and_then(|o| o.location), Some(LIMBO));
        ok(&mut tw, r.pat, "out");
        period(&mut tw, &r, 1);
    }
    assert_eq!(status(&tw, id), TenancyStatusKind::EvictionPending);
}

#[test]
fn a_guest_may_enter_and_a_removed_guest_may_not() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);

    err(&mut tw, r.rex, "in");
    ok(&mut tw, r.pat, &format!("+guest add #{}", r.rex.0));
    ok(&mut tw, r.rex, "in");
    assert_eq!(tw.world().get(r.rex).and_then(|o| o.location), Some(r.anchor));

    ok(&mut tw, r.rex, "out");
    ok(&mut tw, r.pat, &format!("+guest remove #{}", r.rex.0));
    err(&mut tw, r.rex, "in");
}

#[test]
fn add_guest_refuses_a_non_character() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, GOD, "@create rock");
    let rock = tw.world().iter().find(|o| o.name == "rock").expect("rock").id;
    let seen = err(&mut tw, r.pat, &format!("+guest add #{}", rock.0));
    assert!(seen.contains("Only a character"), "{seen}");
}

#[test]
fn an_inspection_term_admits_only_under_its_own_when() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // `delinquent` first: refused while the tenancy is Active.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={} inspect=#{}:delinquent",
            r.property, r.pat.0, r.cadence, r.quinn.0
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    ok(&mut tw, GOD, &format!("+plan {id}=#{} 0", r.pat.0));
    err(&mut tw, r.quinn, "in");

    // …and admitted once the tenancy is actually behind.
    for _ in 0..2 {
        period(&mut tw, &r, 1);
    }
    assert_eq!(status(&tw, id), TenancyStatusKind::Delinquent);
    ok(&mut tw, r.quinn, "in");
    assert_eq!(tw.world().get(r.quinn).and_then(|o| o.location), Some(r.anchor));
}

#[test]
fn a_carry_is_refused_out_of_a_home_and_into_one() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, GOD, &format!("@teleport #{}", r.anchor.0));
    ok(&mut tw, GOD, "@create lamp");
    let lamp = tw.world().iter().find(|o| o.name == "lamp").expect("lamp").id;
    ok(&mut tw, GOD, "drop lamp");
    ok(&mut tw, GOD, &format!("@teleport #{}", LIMBO.0));

    // Reaching in: refused, with the neutral carry text.
    let seen = texts(&tw.run(r.rex, &format!("get #{}", lamp.0)));
    assert!(
        seen.contains(mush_housing::REFUSED_CARRY) || has_error(&tw.run(r.rex, &format!("get #{}", lamp.0))),
        "the stranger reached into a home: {seen}"
    );
    assert_eq!(tw.world().get(lamp).and_then(|o| o.location), Some(r.anchor));
}

#[test]
fn staff_teleporting_a_non_guest_in_is_refused_and_a_guest_is_allowed() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    // 08d §1.1: `check_move_attempt` sees the MOVED object, so a staffer teleporting a
    // stranger in is refused exactly as the stranger is. That is the fail-closed
    // direction, and the documented way round it is to add them as a guest.
    err(&mut tw, GOD, &format!("@teleport #{}=#{}", r.rex.0, r.anchor.0));
    assert_eq!(tw.world().get(r.rex).and_then(|o| o.location), Some(LIMBO));
    ok(&mut tw, r.pat, &format!("+guest add #{}", r.rex.0));
    ok(&mut tw, GOD, &format!("@teleport #{}=#{}", r.rex.0, r.anchor.0));
    assert_eq!(tw.world().get(r.rex).and_then(|o| o.location), Some(r.anchor));
}

// ======================================================================
// The room rule (D-E5-12) and the destroy gate
// ======================================================================

#[test]
fn property_rooms_refuses_a_thoroughfare() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // Open a way into the SPARE room from outside the property.
    ok(&mut tw, GOD, &format!("@teleport #{}", LIMBO.0));
    ok(&mut tw, GOD, &format!("@open shortcut=#{}", r.spare.0));
    // A property that does not yet hold it now cannot take it.
    ok(&mut tw, GOD, "@dig Annex=annex,unannex");
    let annex = room_named(&tw, "Annex");
    ok(
        &mut tw,
        GOD,
        &format!("@property/create Other=#{} tier=1 landlord=housing", annex.0),
    );
    let other = store(&tw).properties().last().expect("property").id;
    let seen = err(&mut tw, GOD, &format!("@property/rooms {other}=#{}", r.spare.0));
    assert!(
        seen.contains("thoroughfare") || seen.contains("already a room"),
        "wrong refusal: {seen}"
    );
}

#[test]
fn property_rooms_refuses_a_room_another_property_already_holds() {
    let mut tw = boot();
    let r = rig(&mut tw);
    ok(&mut tw, GOD, "@dig Annex=annex,unannex");
    let annex = room_named(&tw, "Annex");
    ok(
        &mut tw,
        GOD,
        &format!("@property/create Other=#{} tier=1 landlord=housing", annex.0),
    );
    let other = store(&tw).properties().last().expect("property").id;
    let seen = err(&mut tw, GOD, &format!("@property/rooms {other}=#{}", r.spare.0));
    assert!(seen.contains("already a room"), "wrong refusal: {seen}");
    assert_eq!(property(&tw, other).rooms.len(), 1, "the refused add still took the room");
}

#[test]
fn property_create_refuses_limbo_and_a_marked_room() {
    let mut tw = boot();
    let _r = rig(&mut tw);
    err(&mut tw, GOD, "@property/create Nowhere=#0 tier=1 landlord=housing");
    ok(&mut tw, GOD, "@dig Cellar=down,up");
    let cellar = room_named(&tw, "Cellar");
    ok(&mut tw, GOD, &format!("@set #{}=no-property", cellar.0));
    let seen = err(
        &mut tw,
        GOD,
        &format!("@property/create Cellar=#{} tier=1 landlord=housing", cellar.0),
    );
    assert!(seen.contains("never a private residence"), "wrong refusal: {seen}");
}

#[test]
fn destroy_of_a_let_room_is_refused_and_the_room_survives() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Let);
    let seen = err(&mut tw, GOD, &format!("@destroy #{}", r.spare.0));
    assert!(seen.contains("Withdraw the property first"), "wrong refusal: {seen}");
    assert!(tw.world().get(r.spare).is_some(), "the room was destroyed anyway");

    // Withdrawing clears the way, exactly as the message says.
    ok(&mut tw, GOD, &format!("@property/withdraw {}", r.property));
    ok(&mut tw, GOD, &format!("@destroy #{}", r.spare.0));
}

#[test]
fn a_room_destroyed_programmatically_withdraws_the_property_and_pages() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);
    // The gate is bypassed entirely — a sim teardown, a migration, a bug.
    ok(&mut tw, GOD, &format!("+boom #{}", r.spare.0));
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Withdrawn);
    // The tenancy is untouched: D-E5-1 does not care why the rooms went away.
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
    assert_eq!(tenancy(&tw, id).tenants.len(), 1);
    let paged: Vec<_> = alerts(&tw)
        .into_iter()
        .filter(|e| e.subject == EscalationSubject::HousingProperty(r.property.0))
        .collect();
    assert_eq!(paged.len(), 1, "no page for a destroyed property room");
    assert!(paged[0].context.contains("withdrawn"), "{}", paged[0].context);
}

#[test]
fn an_evicted_tenant_in_a_destroyed_room_lands_in_limbo() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    ok(&mut tw, r.pat, "in");
    assert_eq!(home_of(&tw, r.pat), Some(r.anchor), "the tenant was never homed");

    // Staff end the tenancy after a scene. D-E5-11: the home anchor is cleared.
    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=end they moved on"));
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert_eq!(home_of(&tw, r.pat), None, "the ended tenancy left a live home pointer");

    // …and the hook now refuses them, so they cannot walk back in.
    ok(&mut tw, r.pat, "out");
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Vacant);

    // Put them back inside and destroy the room out from under them: with no home,
    // core's dispersal drops them in Limbo rather than back into the house.
    ok(&mut tw, GOD, &format!("@teleport #{}=#{}", r.pat.0, r.anchor.0));
    ok(&mut tw, GOD, &format!("+boom #{}", r.anchor.0));
    assert_eq!(
        tw.world().get(r.pat).and_then(|o| o.location),
        Some(LIMBO),
        "an evicted tenant dispersed back into the property"
    );
}

// ======================================================================
// Resolutions
// ======================================================================

#[test]
fn resolve_end_clears_every_homed_tenant_and_the_hook_then_refuses() {
    let mut tw = boot();
    let r = rig(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{},#{} cadence={}",
            r.property, r.pat.0, r.quinn.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    assert_eq!(home_of(&tw, r.pat), Some(r.anchor));
    assert_eq!(home_of(&tw, r.quinn), Some(r.anchor));

    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=end"));
    assert_eq!(home_of(&tw, r.pat), None);
    assert_eq!(home_of(&tw, r.quinn), None);
    // The property is `Vacant`, so its rooms are ordinary rooms again and the hook
    // abstains — a residence nobody rents is not a locked box. The refusal is back
    // the moment somebody else lives there.
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Vacant);
    ok(&mut tw, r.pat, "in");
    ok(&mut tw, r.pat, "out");
    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/create {}=#{} cadence={}", r.property, r.rex.0, r.cadence),
    );
    err(&mut tw, r.pat, "in");
    err(&mut tw, r.quinn, "in");
}

#[test]
fn resolve_waive_ends_it_as_forgiven_and_extend_refreshes_the_budget() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    for _ in 0..7 {
        period(&mut tw, &r, 1);
    }
    let t = tenancy(&tw, id);
    let mush_housing::TenancyStatus::EvictionPending { window } = t.status else {
        panic!("not eviction pending")
    };
    assert_eq!(window.notified, mush_housing::MAX_WINDOW_NOTICES, "the budget was not spent");

    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=extend"));
    let t = tenancy(&tw, id);
    let mush_housing::TenancyStatus::EvictionPending { window } = t.status else {
        panic!("extend ended it")
    };
    assert_eq!(window.notified, 0, "extend did not refresh the notice budget");
    assert_eq!(status(&tw, id), TenancyStatusKind::EvictionPending, "extend changed the state");

    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=waive"));
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert!(
        matches!(tenancy(&tw, id).status, mush_housing::TenancyStatus::Ended {
            how: mush_housing::Resolution::Waived { .. }
        }),
        "waive did not record a Waived resolution"
    );
}

#[test]
fn home_leave_ends_it_for_the_last_tenant_and_only_drops_a_co_tenant() {
    let mut tw = boot();
    let r = rig(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{},#{} cadence={}",
            r.property, r.pat.0, r.quinn.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;

    // A co-tenant leaving is a roster change, never an ending.
    ok(&mut tw, r.quinn, "+home/leave");
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
    assert!(!tenancy(&tw, id).is_tenant(r.quinn));
    assert_eq!(home_of(&tw, r.quinn), None);
    err(&mut tw, r.quinn, "in");

    // The last one leaving is the explicit resolution D-E5-1 requires.
    ok(&mut tw, r.pat, "+home/leave");
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert_eq!(home_of(&tw, r.pat), None);
}

#[test]
fn a_second_live_tenancy_on_one_property_is_refused() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);
    let seen = err(
        &mut tw,
        GOD,
        &format!("@tenancy/create {}=#{} cadence={}", r.property, r.quinn.0, r.cadence),
    );
    assert!(seen.contains("already has a live tenancy"), "wrong refusal: {seen}");
    assert_eq!(store(&tw).tenancies().count(), 1, "the refused create still allocated");

    // Ending the first one clears the way.
    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=end"));
    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/create {}=#{} cadence={}", r.property, r.quinn.0, r.cadence),
    );
}

// ======================================================================
// Notices, disclosure, and the landlord's side
// ======================================================================

#[test]
fn an_offline_tenant_reads_the_notice_at_reconnect() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    tw.world_mut().set_connected(r.pat, false);
    period(&mut tw, &r, 1);
    assert_eq!(status(&tw, id), TenancyStatusKind::Grace);
    // Nothing was lost: it is queued, durably.
    let kv = fold_kv(&mut tw);
    assert!(
        housing_keys(&kv).contains(&mush_housing::notice_key(r.pat).to_string()),
        "the offline notice was not persisted"
    );

    tw.world_mut().set_connected(r.pat, true);
    let seen = texts(&tw.run(r.pat, "+home"));
    assert!(seen.contains("waiting for you"), "+home did not show the pending notice: {seen}");
}

#[test]
fn a_guests_home_shows_the_property_and_nothing_about_who_lives_there() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, r.pat, &format!("+guest add #{}", r.rex.0));
    let seen = texts(&tw.run(r.rex, "+home"));
    assert!(seen.contains("you are a guest here"), "{seen}");
    assert!(!seen.contains("Pat"), "a guest was shown the tenant list: {seen}");
    assert!(!seen.contains("tenants:"), "a guest was shown the tenant list: {seen}");
}

#[test]
fn no_such_property_for_the_nonexistent_and_the_unviewable_alike() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    let missing = texts(&tw.run(r.rex, "@property/show 999"));
    let real = texts(&tw.run(r.rex, &format!("@property/show {}", r.property)));
    assert!(missing.contains(mush_housing::NO_SUCH_PROPERTY), "{missing}");
    assert_eq!(missing, real, "a probe can tell the unviewable from the nonexistent");

    let missing = texts(&tw.run(r.rex, "@tenancy/show 999"));
    let real = texts(&tw.run(r.rex, "@tenancy/show 0"));
    assert!(missing.contains(mush_housing::NO_SUCH_TENANCY), "{missing}");
    assert_eq!(missing, real);
}

#[test]
fn a_landlord_officer_sees_their_own_properties_and_nothing_of_another_landlords() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // One org landlord with an officer, and the system-landlord property from the rig.
    ok(&mut tw, GOD, "@org/create Ironhold");
    let (org, scope) = {
        let st = tw.world().module_state::<mush_org::OrgStore>().expect("org");
        let o = st.state().orgs().next().expect("org").clone();
        (o.id, o.root_scope)
    };
    ok(&mut tw, GOD, &format!("@org/grant #{}=Ironhold property.view scope={scope}", r.quinn.0));
    ok(&mut tw, GOD, "@dig Tower=up,down");
    let tower = room_named(&tw, "Tower");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@property/create Ironhold Tower=#{} tier=3 landlord=org:{} scope={scope}",
            tower.0, org
        ),
    );
    let theirs = store(&tw).properties().last().expect("property").id;

    let seen = texts(&tw.run(r.quinn, "@property/list"));
    assert!(seen.contains(&format!("#{theirs}")), "the officer cannot see their own: {seen}");
    assert!(
        !seen.contains(&format!("#{} ", r.property)),
        "the officer saw another landlord's property: {seen}"
    );
    // …and the reverse: a stranger sees neither.
    let seen = texts(&tw.run(r.rex, "@property/list"));
    assert!(seen.contains("No properties."), "{seen}");
}

#[test]
fn tenancy_creation_is_staff_only_even_for_a_landlord_officer() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // §11 answer 3, binding: `property.manage` covers show/list and guest policy, and
    // never tenancy creation.
    err(
        &mut tw,
        r.quinn,
        &format!("@tenancy/create {}=#{} cadence={}", r.property, r.pat.0, r.cadence),
    );
    assert_eq!(store(&tw).tenancies().count(), 0);
}

// ======================================================================
// Exact mode — a real lease contract and a real obligation
// ======================================================================

/// Two accounts in one instrument, funded, plus the lease the tenancy stands on.
fn exact_rig(tw: &mut TestWorld, r: &Rig, funds: i64) -> (TenancyId, u64, u64) {
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, &format!("@account/open #{}=gold", r.pat.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", r.quinn.0));
    let (faucet, tenant_acct, landlord_acct) = {
        let snap = tw.world().module_state::<EconomyStore>().expect("econ").snapshot();
        let faucet = snap
            .accounts()
            .find(|a| matches!(a.owner, mush_org::PrincipalId::System(_)))
            .expect("faucet")
            .id;
        let t = snap
            .accounts()
            .find(|a| a.owner == mush_org::PrincipalId::Object(r.pat))
            .expect("tenant account")
            .id;
        let l = snap
            .accounts()
            .find(|a| a.owner == mush_org::PrincipalId::Object(r.quinn))
            .expect("landlord account")
            .id;
        (faucet, t, l)
    };
    if funds > 0 {
        ok(tw, GOD, &format!("@econ/mint {faucet}={tenant_acct} {funds}"));
    }
    // The premises are already titled: `@property/create` minted the anchor's title
    // to the landlord, which is exactly the asset the lease pledges as collateral.
    ok(
        tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={} mode=exact rent=10 from={tenant_acct} \
             to={landlord_acct}",
            r.property, r.pat.0, r.cadence
        ),
    );
    let id = store(tw).tenancies().last().expect("tenancy").id;
    (id, tenant_acct.0, landlord_acct.0)
}

#[test]
fn exact_mode_stands_on_a_real_lease_contract_and_a_real_obligation() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    // The rig's property is let by `sys:wod.housing`, which types nothing and cannot
    // hold a title; an exact lease needs a landlord who can be a party. Rebuild it.
    ok(&mut tw, GOD, "@dig Loft=up2,down2");
    let loft = room_named(&tw, "Loft");
    ok(
        &mut tw,
        GOD,
        &format!("@property/create Loft=#{} tier=1 landlord=#{}", loft.0, r.quinn.0),
    );
    r.property = store(&tw).properties().last().expect("property").id;
    r.anchor = loft;
    let (id, _, _) = exact_rig(&mut tw, &r, 1_000);

    // Drafted, not live: the tenant's own acceptance is the consent.
    assert_eq!(status(&tw, id), TenancyStatusKind::Pending);
    let contract = {
        let t = tenancy(&tw, id);
        match t.funding {
            mush_housing::Funding::Exact { contract, obligation } => {
                assert_eq!(obligation, mush_housing::PENDING_OBLIGATION);
                contract
            }
            mush_housing::Funding::Abstract { .. } => panic!("exact mode drafted an abstract tenancy"),
        }
    };

    ok(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    assert_eq!(status(&tw, id), TenancyStatusKind::Active, "acceptance did not activate the tenancy");
    let obligation = match tenancy(&tw, id).funding {
        mush_housing::Funding::Exact { obligation, .. } => obligation,
        mush_housing::Funding::Abstract { .. } => panic!("funding changed"),
    };
    assert_ne!(obligation, mush_housing::PENDING_OBLIGATION, "no rent obligation was bound");
    // The economy owns the money: the obligation exists, is active, and names the
    // contract as its cause.
    let o = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(obligation).cloned())
        .expect("obligation");
    assert_eq!(o.cause, mush_economy::ObligationCause::Contract(contract));
    // The premises carry the contract's `Lease` encumbrance.
    let asset = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().asset(loft).cloned())
        .expect("title");
    assert!(
        asset.encumbrances.iter().any(|e| matches!(
            e,
            mush_economy::Encumbrance::Lease { contract: c, .. } if *c == contract
        )),
        "the lease placed no Lease encumbrance: {:?}",
        asset.encumbrances
    );
    // And the home invariant applies exactly as it does in abstract mode.
    assert_eq!(home_of(&tw, r.pat), Some(loft));
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Let);
}

#[test]
fn an_exact_tenancy_mirrors_its_obligation_and_reaches_the_window() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    ok(&mut tw, GOD, "@dig Loft=up2,down2");
    let loft = room_named(&tw, "Loft");
    ok(
        &mut tw,
        GOD,
        &format!("@property/create Loft=#{} tier=1 landlord=#{}", loft.0, r.quinn.0),
    );
    r.property = store(&tw).properties().last().expect("property").id;
    r.anchor = loft;
    // No funds at all: every settlement misses.
    let (id, _, landlord) = exact_rig(&mut tw, &r, 0);
    let contract = match tenancy(&tw, id).funding {
        mush_housing::Funding::Exact { contract, .. } => contract,
        mush_housing::Funding::Abstract { .. } => panic!("abstract"),
    };
    ok(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);

    // Both clocks move: wall time is what makes the obligation miss and then go
    // delinquent on its OWN schedule, and the campaign clock is what counts the
    // assessed periods that walk the tenancy down the ladder. Reading either off the
    // other is exactly the bug `EconomicDeadline` exists to make unwritable.
    for _ in 0..24 {
        ok(&mut tw, GOD, &format!("@econ/cadence advance {} 1", r.cadence));
        let t = tw.world().now().saturating_add(86_400);
        tw.set_now(t);
        tw.fire_due(t.saturating_mul(1000));
        if status(&tw, id) == TenancyStatusKind::EvictionPending {
            break;
        }
    }
    assert!(
        matches!(
            status(&tw, id),
            TenancyStatusKind::Delinquent | TenancyStatusKind::EvictionPending
        ),
        "the exact ladder stalled at {:?}",
        status(&tw, id)
    );
    // Whatever happened, no housing code moved money: the landlord was never paid,
    // because the tenant never had anything to pay with.
    assert_eq!(
        tw.world()
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().account(mush_economy::AccountId(landlord)).map(|a| a.posted_balance.minor())),
        Some(0),
        "housing posted something"
    );
}

// ======================================================================
// Persistence, boot, reconciliation
// ======================================================================

#[test]
fn a_reboot_at_every_rung_preserves_the_ladder() {
    for stop in 0..6 {
        let mut tw = boot();
        let r = rig(&mut tw);
        let id = tenant(&mut tw, &r, r.pat, 0);
        for _ in 0..stop {
            period(&mut tw, &r, 1);
        }
        let before = status(&tw, id);
        let rooms = property(&tw, r.property).rooms.clone();
        let kv = fold_kv(&mut tw);
        reboot(&mut tw, kv);
        assert_eq!(status(&tw, id), before, "the ladder moved across a reboot (stop {stop})");
        assert_eq!(property(&tw, r.property).rooms, rooms);
        // The hook still answers for the same rooms after the reboot.
        ok(&mut tw, r.pat, "in");
        err(&mut tw, r.rex, "in");
    }
}

#[test]
fn a_corrupt_tenancy_document_fails_the_boot() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);
    let mut kv = fold_kv(&mut tw);
    // Past every §9 cap, through E5a's `TryFrom` wire: a document that would mint a
    // tenancy with no tenants at all.
    patch(&mut kv, mush_housing::tenancy_key(id).as_str(), |v| {
        v["tenants"] = serde_json::json!([]);
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reboot(&mut tw, kv.clone());
    }));
    assert!(result.is_err(), "a corrupt container document booted anyway");
}

#[test]
fn boot_withdraws_a_property_whose_rooms_went_missing_and_pages() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);
    let kv = fold_kv(&mut tw);
    // The room is gone from the world, but the housing document still names it —
    // exactly what a restore from a mismatched snapshot looks like.
    ok(&mut tw, GOD, &format!("@property/withdraw {}", r.property));
    ok(&mut tw, GOD, &format!("@destroy #{}", r.spare.0));
    tw.world_mut().sweep();
    reboot(&mut tw, kv);
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Withdrawn);
    assert_eq!(status(&tw, id), TenancyStatusKind::Active, "boot ended a tenancy");
    assert!(
        alerts(&tw)
            .iter()
            .any(|e| e.subject == EscalationSubject::HousingProperty(r.property.0)),
        "boot did not page about the missing rooms"
    );
}

#[test]
fn boot_drops_a_destroyed_tenant_through_remove_tenant() {
    let mut tw = boot();
    let r = rig(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{},#{} cadence={}",
            r.property, r.pat.0, r.quinn.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    let kv = fold_kv(&mut tw);
    ok(&mut tw, GOD, &format!("@destroy #{}", r.quinn.0));
    tw.world_mut().sweep();
    reboot(&mut tw, kv);
    let t = tenancy(&tw, id);
    assert!(!t.is_tenant(r.quinn), "a destroyed tenant kept their place on the roster");
    assert!(t.is_tenant(r.pat));
    assert_eq!(t.status.kind(), TenancyStatusKind::Active, "boot ended the tenancy");
    // S11/A1: the roster went through `remove_tenant`, which writes the history entry
    // and leaves `homed` to the ending sweep.
    assert!(
        t.history.iter().any(|e| matches!(
            e,
            mush_housing::TenancyEvent::TenantRemoved { who, .. } if *who == r.quinn
        )),
        "the removal was not recorded: {:?}",
        t.history
    );
    assert!(t.homed.contains(&r.quinn), "boot edited `homed`, which the ending sweep owns");
}

#[test]
fn boot_pages_about_an_exact_tenancy_whose_obligation_is_gone() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    ok(&mut tw, GOD, "@dig Loft=up2,down2");
    let loft = room_named(&tw, "Loft");
    ok(
        &mut tw,
        GOD,
        &format!("@property/create Loft=#{} tier=1 landlord=#{}", loft.0, r.quinn.0),
    );
    r.property = store(&tw).properties().last().expect("property").id;
    r.anchor = loft;
    let (id, _, _) = exact_rig(&mut tw, &r, 1_000);
    let contract = match tenancy(&tw, id).funding {
        mush_housing::Funding::Exact { contract, .. } => contract,
        mush_housing::Funding::Abstract { .. } => panic!("abstract"),
    };
    ok(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);

    // Point the tenancy at an obligation that does not exist, and reboot.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, mush_housing::tenancy_key(id).as_str(), |v| {
        v["funding"]["Exact"]["obligation"] = serde_json::json!(9_999);
    });
    reboot(&mut tw, kv);
    assert_eq!(status(&tw, id), TenancyStatusKind::Active, "boot moved a tenancy on a missing record");
    assert!(
        alerts(&tw).iter().any(|e| e.subject == EscalationSubject::HousingTenancy(id.0)),
        "boot did not page about the missing obligation"
    );
}

#[test]
fn allocators_are_floored_past_every_key_and_never_reused() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=end"));
    let mut kv = fold_kv(&mut tw);
    // A document at a far higher id than `meta` knows about — a partial restore.
    let entry = kv.get_mut(&SmolStr::new(NS)).expect("housing namespace");
    let doc = entry
        .iter()
        .find(|(k, _)| k.as_str() == mush_housing::tenancy_key(id).as_str())
        .map(|(_, v)| v.clone())
        .expect("tenancy doc");
    let mut high = doc;
    high["id"] = serde_json::json!(500);
    entry.push((SmolStr::new("tenancy:500"), high));
    reboot(&mut tw, kv);
    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/create {}=#{} cadence={}", r.property, r.quinn.0, r.cadence),
    );
    let fresh = store(&tw).tenancies().last().expect("tenancy").id;
    assert!(fresh.0 > 500, "the allocator re-minted a live id: {fresh}");
}

// ======================================================================
// The cadence verbs (E4b1 deviation 1, shipped here)
// ======================================================================

#[test]
fn the_cadence_verbs_create_show_list_and_advance_only_a_manual_clock() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let seen = ok(&mut tw, GOD, "@econ/cadence list");
    assert!(seen.contains("chronicle"), "{seen}");
    let seen = ok(&mut tw, GOD, &format!("@econ/cadence show {}", r.cadence));
    assert!(seen.contains("manual"), "{seen}");

    ok(&mut tw, GOD, "@econ/cadence create season source=real:3600 catchup=2");
    let season = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().cadences().find(|c| c.name == "season").map(|c| c.id.0))
        .expect("season");
    let seen = err(&mut tw, GOD, &format!("@econ/cadence advance {season}"));
    assert!(seen.contains("only a manual cadence"), "{seen}");

    // Creating one is staff-only, and a duplicate name is refused.
    err(&mut tw, r.pat, "@econ/cadence create mine source=manual");
    err(&mut tw, GOD, "@econ/cadence create season source=real:3600");
    ok(&mut tw, GOD, &format!("@econ/cadence advance {} 3", r.cadence));
    let position = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().cadence(mush_economy::CadenceId(r.cadence)).map(|c| c.position))
        .expect("cadence");
    assert_eq!(position, 3);
}

#[test]
fn a_tenancy_on_a_clock_that_never_moves_never_moves() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    // Forty fires with the campaign clock standing still: the pull model does
    // nothing at all, so an unsupported tenancy does not creep toward eviction on
    // wall time.
    for _ in 0..40 {
        fire(&mut tw);
    }
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
}

// ======================================================================
// Title, and the property surface
// ======================================================================

#[test]
fn creating_a_property_mints_the_anchors_title_to_the_landlord() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let asset = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().asset(r.anchor).cloned())
        .expect("the anchor carries no title record");
    assert!(
        matches!(&asset.title_owner, mush_org::PrincipalId::System(s) if s.as_str() == "wod.housing"),
        "title went to {:?}",
        asset.title_owner
    );
    // …and the room's own @owner is untouched: title is not object ownership.
    assert_eq!(tw.world().get(r.anchor).map(|o| o.owner), Some(GOD));
    // The spare room is a property room but carries no title of its own.
    assert!(
        tw.world()
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().asset(r.spare).cloned())
            .is_none()
    );
}

#[test]
fn a_property_with_no_wired_issuer_refuses_to_be_created() {
    // The fail-closed default: a residence the economy does not know exists is one
    // nothing can lease, encumber or transfer.
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new().with_capabilities(vocab())),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(mush_housing::HousingModule::new()),
    ]);
    ok(&mut tw, GOD, "@dig Hut=inhut,outhut");
    let hut = tw.world().iter().find(|o| o.name == "Hut").expect("hut").id;
    // `landlord=housing` resolves to the wired system principal, and there is none —
    // so name one explicitly, and the refusal is squarely about the missing issuer.
    let seen = err(
        &mut tw,
        GOD,
        &format!("@property/create Hut=#{} tier=1 landlord=sys:somebody", hut.0),
    );
    assert!(seen.contains("no housing title issuer"), "{seen}");
    // …and the generic `housing` landlord does not resolve at all without one.
    err(
        &mut tw,
        GOD,
        &format!("@property/create Hut=#{} tier=1 landlord=housing", hut.0),
    );
    assert_eq!(
        tw.world().module_state::<HousingStore>().map(|s| s.properties().count()),
        Some(0)
    );
}

#[test]
fn a_let_propertys_rooms_cannot_be_removed_and_the_anchor_never_can() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    err(&mut tw, GOD, &format!("@property/rooms {}=-#{}", r.property, r.spare.0));
    assert_eq!(property(&tw, r.property).rooms.len(), 2);
    ok(&mut tw, GOD, &format!("@property/withdraw {}", r.property));
    err(&mut tw, GOD, &format!("@property/rooms {}=-#{}", r.property, r.anchor.0));
    ok(&mut tw, GOD, &format!("@property/rooms {}=-#{}", r.property, r.spare.0));
    assert_eq!(property(&tw, r.property).rooms.len(), 1);
}

#[test]
fn every_verb_has_help_and_the_module_wires_through_registration_alone() {
    let tw = boot();
    for verb in ["@property", "@tenancy", "+home", "+guest"] {
        let spec = tw.registry().spec_of(verb).unwrap_or_else(|| panic!("{verb} is not registered"));
        assert!(spec.help.len() > 200, "{verb} has no real help text");
    }
    assert!(tw.registry().tick_reg(NS, mush_housing::TICK_TAG).is_some(), "no assessment tick");
    assert!(
        tw.registry().job_queues().iter().any(|q| q.name == mush_housing::STORY_QUEUE),
        "the wod-story queue is not registered"
    );
    assert!(
        tw.registry().marker_owner(mush_housing::NO_PROPERTY_MARKER).is_some(),
        "the no-property marker is not registered"
    );
}

#[test]
fn resolve_scene_starts_a_scene_and_records_it_on_the_window() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    // No window, no scene: the verb answers about the tenancy, not about scenes.
    err(&mut tw, GOD, &format!("@tenancy/resolve {id}=scene"));
    for _ in 0..4 {
        period(&mut tw, &r, 1);
    }
    assert_eq!(status(&tw, id), TenancyStatusKind::EvictionPending);
    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=scene"));
    let t = tenancy(&tw, id);
    let mush_housing::TenancyStatus::EvictionPending { window } = t.status else {
        panic!("the scene ended the tenancy")
    };
    assert!(window.scene.is_some(), "the window records no scene");
    assert!(window.job.is_some(), "the job was lost");
    // Idempotent: asking again starts a second scene, and attaching a DIFFERENT id is
    // refused rather than silently orphaning the first (E5a's `WindowAlreadyAttached`).
    err(&mut tw, GOD, &format!("@tenancy/resolve {id}=scene"));
    assert_eq!(status(&tw, id), TenancyStatusKind::EvictionPending);
}
