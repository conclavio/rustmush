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
    HousingStore, LiveEntry, PropertyId, PropertyStatus, RoomEntry, SupportLevel, SupportSource,
    TenancyId, TenancyStatusKind,
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
    boot_with(Vec::new())
}

/// [`boot`] with a wiring-frozen reserved-room list — the host's `[onboarding]` start
/// rooms, which `mush-server` now passes for real (verify fold, H4).
fn boot_with(reserved: Vec<ObjId>) -> TestWorld {
    let issuer = SystemPrincipalId::new("wod.housing").expect("valid system principal");
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new().with_capabilities(vocab())),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_title_issuers(vec![issuer.clone()])
                .with_observers(vec![Arc::new(mush_housing::HousingEconomyObserver)]),
        ),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(
            mush_housing::HousingModule::new()
                .with_issuer(issuer)
                .with_reserved_rooms(reserved),
        ),
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
    let tenancy = TenancyId(
        ctx.args
            .lhs()
            .trim()
            .parse::<u64>()
            .map_err(|_| CmdError::msg("id"))?,
    );
    let rhs = ctx.args.rhs().unwrap_or("").to_string();
    let mut w = rhs.split_whitespace();
    let who = ObjId(
        w.next()
            .and_then(|s| s.trim_start_matches('#').parse::<u32>().ok())
            .ok_or_else(|| CmdError::msg("who"))?,
    );
    let level = SupportLevel::new(
        w.next()
            .and_then(|s| s.parse::<u8>().ok())
            .ok_or_else(|| CmdError::msg("level"))?,
    );
    mush_housing::submit_support_plan(ctx, tenancy, who, level, vec![SupportSource::Inheritance])
        .map_err(|e| CmdError::msg(e.to_string()))?;
    let actor = ctx.actor;
    ctx.out
        .sys_to_actor(actor, "test", "plan submitted".to_string());
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
    ctx.w
        .destroy(room)
        .map_err(|e| CmdError::msg(e.to_string()))?;
    ctx.emit_world_event(&WorldEvent::ObjDestroyed { obj: room });
    Ok(())
}

// ---- output helpers -------------------------------------------------------

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
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    // Look-back fold, D-LBK-4a: every mutation in this file leaves the projections
    // equal to a rebuild from the records.
    assert_projections_consistent(tw);
    texts(&ev)
}
fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    assert_projections_consistent(tw);
    texts(&ev)
}

/// The recomputation-equivalence assertion (D-LBK-4a): the maintained room and
/// live-tenancy projections equal a rebuild from the records.
fn assert_projections_consistent(tw: &TestWorld) {
    if let Some(s) = tw.world().module_state::<HousingStore>()
        && let Err(drift) = s.projections_consistent()
    {
        panic!("projections drifted from the records: {drift}");
    }
}

fn store(tw: &TestWorld) -> &HousingStore {
    tw.world()
        .module_state::<HousingStore>()
        .expect("housing store")
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
        &format!(
            "@property/create The Flat=#{},#{} tier=2 landlord=housing",
            anchor.0, spare.0
        ),
    );
    Rig {
        pat,
        quinn,
        rex,
        anchor,
        spare,
        cadence,
        property: PropertyId(0),
    }
}

/// One live abstract tenancy for `who`, at the given plan level.
fn tenant(tw: &mut TestWorld, r: &Rig, who: ObjId, level: u8) -> TenancyId {
    ok(
        tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={}",
            r.property, who.0, r.cadence
        ),
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
        .map(|(ns, kv)| {
            (
                ns,
                ModuleBootData {
                    kv,
                    streams: HashMap::new(),
                },
            )
        })
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
        assert!(
            !seen.contains("HOME:"),
            "a supported period notified: {seen}"
        );
    }
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "a supported tenancy moved"
    );
    // S3: a restatement is not news, so the audit tail is not thirty plans deep.
    let t = tenancy(&tw, id);
    assert!(
        t.history.len() <= 4,
        "restatements filled the tail: {:?}",
        t.history
    );
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
    let body = job
        .comments
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();
    assert!(
        body.contains(&format!("@tenancy/resolve {id}=end")),
        "job body: {body}"
    );
    assert!(
        body.contains("nothing ends this home on its own"),
        "job body: {body}"
    );
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
    assert_eq!(
        renotices,
        mush_housing::MAX_WINDOW_NOTICES as usize,
        "the notice budget is 3"
    );

    // Then silence until the escalation gap has elapsed (WINDOW_ESCALATE, measured
    // from the last notice — E5a deviation 1).
    for _ in 0..1 {
        let seen = texts(&period(&mut tw, &r, 1));
        quiet += usize::from(!seen.contains("HOME:"));
    }
    assert_eq!(quiet, 1, "the window was not quiet after its budget");

    // Then exactly one page, and no more notices of the old kind.
    let seen = texts(&period(&mut tw, &r, 1));
    assert!(
        seen.contains("staff have now been paged"),
        "no escalation notice: {seen}"
    );
    let paged: Vec<_> = alerts(&tw)
        .into_iter()
        .filter(|e| e.subject == EscalationSubject::HousingTenancy(id.0))
        .collect();
    assert_eq!(paged.len(), 1, "one page per window");
    let page = &paged[0];
    assert_eq!(page.level, 2);
    assert!(
        page.recommended
            .iter()
            .any(|c| c == &format!("@tenancy/resolve {id}=end")),
        "the alert does not carry the filled-in commands: {:?}",
        page.recommended
    );
    assert!(
        status(&tw, id) == TenancyStatusKind::EvictionPending,
        "the page changed the state"
    );
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
        assert_eq!(
            status(&tw, id),
            TenancyStatusKind::Active,
            "recovery from {before:?} failed"
        );
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
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "a poor co-tenant dragged the lease down"
    );

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
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Grace,
        "a jump charged the backlog"
    );
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
        &format!(
            "@tenancy/create {}=#{} cadence={}",
            r.property, r.pat.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    period(&mut tw, &r, 1);
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "a silent evaluator was read as a miss"
    );
    period(&mut tw, &r, 1);
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Grace,
        "UNKNOWN_GRACE never hardened"
    );
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
        assert!(
            seen.contains(mush_housing::REFUSED),
            "wrong refusal: {seen}"
        );
        assert!(
            !seen.to_lowercase().contains("tenan"),
            "the refusal leaked a tenancy: {seen}"
        );
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
    assert_eq!(
        tw.world().get(r.rex).and_then(|o| o.location),
        Some(r.anchor)
    );

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
    let rock = tw
        .world()
        .iter()
        .find(|o| o.name == "rock")
        .expect("rock")
        .id;
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
    assert_eq!(
        tw.world().get(r.quinn).and_then(|o| o.location),
        Some(r.anchor)
    );
}

#[test]
fn a_carry_is_refused_out_of_a_home_and_into_one() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, GOD, &format!("@teleport #{}", r.anchor.0));
    ok(&mut tw, GOD, "@create lamp");
    let lamp = tw
        .world()
        .iter()
        .find(|o| o.name == "lamp")
        .expect("lamp")
        .id;
    ok(&mut tw, GOD, "drop lamp");
    ok(&mut tw, GOD, &format!("@teleport #{}", LIMBO.0));

    // Reaching in: refused, with the neutral carry text.
    let seen = texts(&tw.run(r.rex, &format!("get #{}", lamp.0)));
    assert!(
        seen.contains(mush_housing::REFUSED_CARRY)
            || has_error(&tw.run(r.rex, &format!("get #{}", lamp.0))),
        "the stranger reached into a home: {seen}"
    );
    assert_eq!(
        tw.world().get(lamp).and_then(|o| o.location),
        Some(r.anchor)
    );
}

#[test]
fn staff_teleporting_a_non_guest_in_is_refused_and_a_guest_is_allowed() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    // 08d §1.1: `check_move_attempt` sees the MOVED object, so a staffer teleporting a
    // stranger in is refused exactly as the stranger is. That is the fail-closed
    // direction, and the documented way round it is to add them as a guest.
    err(
        &mut tw,
        GOD,
        &format!("@teleport #{}=#{}", r.rex.0, r.anchor.0),
    );
    assert_eq!(tw.world().get(r.rex).and_then(|o| o.location), Some(LIMBO));
    ok(&mut tw, r.pat, &format!("+guest add #{}", r.rex.0));
    ok(
        &mut tw,
        GOD,
        &format!("@teleport #{}=#{}", r.rex.0, r.anchor.0),
    );
    assert_eq!(
        tw.world().get(r.rex).and_then(|o| o.location),
        Some(r.anchor)
    );
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
        &format!(
            "@property/create Other=#{} tier=1 landlord=housing",
            annex.0
        ),
    );
    let other = store(&tw).properties().last().expect("property").id;
    let seen = err(
        &mut tw,
        GOD,
        &format!("@property/rooms {other}=#{}", r.spare.0),
    );
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
        &format!(
            "@property/create Other=#{} tier=1 landlord=housing",
            annex.0
        ),
    );
    let other = store(&tw).properties().last().expect("property").id;
    let seen = err(
        &mut tw,
        GOD,
        &format!("@property/rooms {other}=#{}", r.spare.0),
    );
    assert!(seen.contains("already a room"), "wrong refusal: {seen}");
    assert_eq!(
        property(&tw, other).rooms.len(),
        1,
        "the refused add still took the room"
    );
}

#[test]
fn property_create_refuses_limbo_and_a_marked_room() {
    let mut tw = boot();
    let _r = rig(&mut tw);
    err(
        &mut tw,
        GOD,
        "@property/create Nowhere=#0 tier=1 landlord=housing",
    );
    ok(&mut tw, GOD, "@dig Cellar=down,up");
    let cellar = room_named(&tw, "Cellar");
    ok(&mut tw, GOD, &format!("@set #{}=no-property", cellar.0));
    let seen = err(
        &mut tw,
        GOD,
        &format!(
            "@property/create Cellar=#{} tier=1 landlord=housing",
            cellar.0
        ),
    );
    assert!(
        seen.contains("never a private residence"),
        "wrong refusal: {seen}"
    );
}

#[test]
fn destroy_of_a_let_room_is_refused_and_the_room_survives() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Let);
    let seen = err(&mut tw, GOD, &format!("@destroy #{}", r.spare.0));
    assert!(
        seen.contains("Withdraw the property first"),
        "wrong refusal: {seen}"
    );
    assert!(
        tw.world().get(r.spare).is_some(),
        "the room was destroyed anyway"
    );

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
    assert!(
        paged[0].context.contains("withdrawn"),
        "{}",
        paged[0].context
    );
}

#[test]
fn an_evicted_tenant_in_a_destroyed_room_lands_in_limbo() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    ok(&mut tw, r.pat, "in");
    assert_eq!(
        home_of(&tw, r.pat),
        Some(r.anchor),
        "the tenant was never homed"
    );

    // Staff end the tenancy after a scene. D-E5-11: the home anchor is cleared.
    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/resolve {id}=end they moved on"),
    );
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert_eq!(
        home_of(&tw, r.pat),
        None,
        "the ended tenancy left a live home pointer"
    );

    // …and the hook now refuses them, so they cannot walk back in.
    ok(&mut tw, r.pat, "out");
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Vacant);

    // Put them back inside and destroy the room out from under them: with no home,
    // core's dispersal drops them in Limbo rather than back into the house.
    ok(
        &mut tw,
        GOD,
        &format!("@teleport #{}=#{}", r.pat.0, r.anchor.0),
    );
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
        &format!(
            "@tenancy/create {}=#{} cadence={}",
            r.property, r.rex.0, r.cadence
        ),
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
    assert_eq!(
        window.notified,
        mush_housing::MAX_WINDOW_NOTICES,
        "the budget was not spent"
    );

    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=extend"));
    let t = tenancy(&tw, id);
    let mush_housing::TenancyStatus::EvictionPending { window } = t.status else {
        panic!("extend ended it")
    };
    assert_eq!(
        window.notified, 0,
        "extend did not refresh the notice budget"
    );
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::EvictionPending,
        "extend changed the state"
    );

    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=waive"));
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert!(
        matches!(
            tenancy(&tw, id).status,
            mush_housing::TenancyStatus::Ended {
                how: mush_housing::Resolution::Waived { .. }
            }
        ),
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
        &format!(
            "@tenancy/create {}=#{} cadence={}",
            r.property, r.quinn.0, r.cadence
        ),
    );
    assert!(
        seen.contains("already has a live tenancy"),
        "wrong refusal: {seen}"
    );
    assert_eq!(
        store(&tw).tenancies().count(),
        1,
        "the refused create still allocated"
    );

    // Ending the first one clears the way.
    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=end"));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={}",
            r.property, r.quinn.0, r.cadence
        ),
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
    assert!(
        seen.contains("waiting for you"),
        "+home did not show the pending notice: {seen}"
    );
}

#[test]
fn a_guests_home_shows_the_property_and_nothing_about_who_lives_there() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, r.pat, &format!("+guest add #{}", r.rex.0));
    let seen = texts(&tw.run(r.rex, "+home"));
    assert!(seen.contains("you are a guest here"), "{seen}");
    assert!(
        !seen.contains("Pat"),
        "a guest was shown the tenant list: {seen}"
    );
    assert!(
        !seen.contains("tenants:"),
        "a guest was shown the tenant list: {seen}"
    );
}

#[test]
fn no_such_property_for_the_nonexistent_and_the_unviewable_alike() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    let missing = texts(&tw.run(r.rex, "@property/show 999"));
    let real = texts(&tw.run(r.rex, &format!("@property/show {}", r.property)));
    assert!(
        missing.contains(mush_housing::NO_SUCH_PROPERTY),
        "{missing}"
    );
    assert_eq!(
        missing, real,
        "a probe can tell the unviewable from the nonexistent"
    );

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
        let st = tw
            .world()
            .module_state::<mush_org::OrgStore>()
            .expect("org");
        let o = st.state().orgs().next().expect("org").clone();
        (o.id, o.root_scope)
    };
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold property.view scope={scope}",
            r.quinn.0
        ),
    );
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
    assert!(
        seen.contains(&format!("#{theirs}")),
        "the officer cannot see their own: {seen}"
    );
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
        &format!(
            "@tenancy/create {}=#{} cadence={}",
            r.property, r.pat.0, r.cadence
        ),
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
        let snap = tw
            .world()
            .module_state::<EconomyStore>()
            .expect("econ")
            .snapshot();
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
        ok(
            tw,
            GOD,
            &format!("@econ/mint {faucet}={tenant_acct} {funds}"),
        );
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
        &format!(
            "@property/create Loft=#{} tier=1 landlord=#{}",
            loft.0, r.quinn.0
        ),
    );
    r.property = store(&tw).properties().last().expect("property").id;
    r.anchor = loft;
    let (id, _, _) = exact_rig(&mut tw, &r, 1_000);

    // Drafted, not live: the tenant's own acceptance is the consent.
    assert_eq!(status(&tw, id), TenancyStatusKind::Pending);
    let contract = {
        let t = tenancy(&tw, id);
        match t.funding {
            mush_housing::Funding::Exact {
                contract,
                obligation,
            } => {
                assert_eq!(obligation, mush_housing::PENDING_OBLIGATION);
                contract
            }
            mush_housing::Funding::Abstract { .. } => {
                panic!("exact mode drafted an abstract tenancy")
            }
        }
    };

    ok(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "acceptance did not activate the tenancy"
    );
    let obligation = match tenancy(&tw, id).funding {
        mush_housing::Funding::Exact { obligation, .. } => obligation,
        mush_housing::Funding::Abstract { .. } => panic!("funding changed"),
    };
    assert_ne!(
        obligation,
        mush_housing::PENDING_OBLIGATION,
        "no rent obligation was bound"
    );
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
        &format!(
            "@property/create Loft=#{} tier=1 landlord=#{}",
            loft.0, r.quinn.0
        ),
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
        ok(
            &mut tw,
            GOD,
            &format!("@econ/cadence advance {} 1", r.cadence),
        );
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
        tw.world().module_state::<EconomyStore>().and_then(|s| s
            .snapshot()
            .account(mush_economy::AccountId(landlord))
            .map(|a| a.posted_balance.minor())),
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
        assert_eq!(
            status(&tw, id),
            before,
            "the ladder moved across a reboot (stop {stop})"
        );
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
    assert!(
        result.is_err(),
        "a corrupt container document booted anyway"
    );
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
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "boot ended a tenancy"
    );
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
    assert!(
        !t.is_tenant(r.quinn),
        "a destroyed tenant kept their place on the roster"
    );
    assert!(t.is_tenant(r.pat));
    assert_eq!(
        t.status.kind(),
        TenancyStatusKind::Active,
        "boot ended the tenancy"
    );
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
    assert!(
        t.homed.contains(&r.quinn),
        "boot edited `homed`, which the ending sweep owns"
    );
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
        &format!(
            "@property/create Loft=#{} tier=1 landlord=#{}",
            loft.0, r.quinn.0
        ),
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
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "boot moved a tenancy on a missing record"
    );
    assert!(
        alerts(&tw)
            .iter()
            .any(|e| e.subject == EscalationSubject::HousingTenancy(id.0)),
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
        &format!(
            "@tenancy/create {}=#{} cadence={}",
            r.property, r.quinn.0, r.cadence
        ),
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

    ok(
        &mut tw,
        GOD,
        "@econ/cadence create season source=real:3600 catchup=2",
    );
    let season = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .cadences()
                .find(|c| c.name == "season")
                .map(|c| c.id.0)
        })
        .expect("season");
    let seen = err(&mut tw, GOD, &format!("@econ/cadence advance {season}"));
    assert!(seen.contains("only a manual cadence"), "{seen}");

    // Creating one is staff-only, and a duplicate name is refused.
    err(&mut tw, r.pat, "@econ/cadence create mine source=manual");
    err(&mut tw, GOD, "@econ/cadence create season source=real:3600");
    ok(
        &mut tw,
        GOD,
        &format!("@econ/cadence advance {} 3", r.cadence),
    );
    let position = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .cadence(mush_economy::CadenceId(r.cadence))
                .map(|c| c.position)
        })
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
        &format!(
            "@property/create Hut=#{} tier=1 landlord=sys:somebody",
            hut.0
        ),
    );
    assert!(seen.contains("no housing title issuer"), "{seen}");
    // …and the generic `housing` landlord does not resolve at all without one.
    err(
        &mut tw,
        GOD,
        &format!("@property/create Hut=#{} tier=1 landlord=housing", hut.0),
    );
    assert_eq!(
        tw.world()
            .module_state::<HousingStore>()
            .map(|s| s.properties().count()),
        Some(0)
    );
}

#[test]
fn a_let_propertys_rooms_cannot_be_removed_and_the_anchor_never_can() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    err(
        &mut tw,
        GOD,
        &format!("@property/rooms {}=-#{}", r.property, r.spare.0),
    );
    assert_eq!(property(&tw, r.property).rooms.len(), 2);
    ok(&mut tw, GOD, &format!("@property/withdraw {}", r.property));
    err(
        &mut tw,
        GOD,
        &format!("@property/rooms {}=-#{}", r.property, r.anchor.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@property/rooms {}=-#{}", r.property, r.spare.0),
    );
    assert_eq!(property(&tw, r.property).rooms.len(), 1);
}

#[test]
fn every_verb_has_help_and_the_module_wires_through_registration_alone() {
    let tw = boot();
    for verb in ["@property", "@tenancy", "+home", "+guest"] {
        let spec = tw
            .registry()
            .spec_of(verb)
            .unwrap_or_else(|| panic!("{verb} is not registered"));
        assert!(spec.help.len() > 200, "{verb} has no real help text");
    }
    assert!(
        tw.registry().tick_reg(NS, mush_housing::TICK_TAG).is_some(),
        "no assessment tick"
    );
    assert!(
        tw.registry()
            .job_queues()
            .iter()
            .any(|q| q.name == mush_housing::STORY_QUEUE),
        "the wod-story queue is not registered"
    );
    assert!(
        tw.registry()
            .marker_owner(mush_housing::NO_PROPERTY_MARKER)
            .is_some(),
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

// ======================================================================
// ---- E5b verify fold ----
// ======================================================================
//
// Three adversarial lenses over the shipped module, one test per confirmed finding:
// **A** (command authority and existence hiding), **H** (hooks and home safety),
// **T** (tick, ladder, exact mode, durability). Every one of them is a regression
// test in the strict sense — it fails on the code as it was shipped.

// ---- fold helpers ---------------------------------------------------------

fn window_of(tw: &TestWorld, id: TenancyId) -> mush_housing::StoryWindow {
    match tenancy(tw, id).status {
        mush_housing::TenancyStatus::EvictionPending { window } => window,
        other => panic!("tenancy #{id} is {other:?}, not eviction-pending"),
    }
}

fn tenancy_alerts(tw: &TestWorld, id: TenancyId) -> Vec<mush_economy::Escalation> {
    alerts(tw)
        .into_iter()
        .filter(|e| e.subject == EscalationSubject::HousingTenancy(id.0))
        .collect()
}

fn property_alerts(tw: &TestWorld, id: PropertyId) -> Vec<mush_economy::Escalation> {
    alerts(tw)
        .into_iter()
        .filter(|e| e.subject == EscalationSubject::HousingProperty(id.0))
        .collect()
}

/// Skip WALL time without moving the campaign clock — how a test reaches the
/// economy ladder's own cool-off (`ESCALATION_RENOTIFY_SECS`, one hour) between an
/// acknowledgement and the page that reopens it.
fn skip_wall(tw: &mut TestWorld, secs: i64) {
    let t = tw.world().now().saturating_add(secs);
    tw.set_now(t);
}

/// An org-let property with one officer holding `capability`. `scoped` chooses
/// whether `@property/create` names the compartment at all — the A4 case is the one
/// that does not.
fn org_property(
    tw: &mut TestWorld,
    officer: ObjId,
    capability: &str,
    scoped: bool,
    room: &str,
) -> PropertyId {
    ok(tw, GOD, "@org/create Ironhold");
    let (org, scope) = {
        let st = tw
            .world()
            .module_state::<mush_org::OrgStore>()
            .expect("org");
        let o = st
            .state()
            .orgs()
            .find(|o| o.name == "Ironhold")
            .expect("org")
            .clone();
        (o.id, o.root_scope)
    };
    ok(
        tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold {capability} scope={scope}",
            officer.0
        ),
    );
    ok(tw, GOD, &format!("@dig {room}=in{room},out{room}"));
    let id = room_named(tw, room);
    let scope_arg = if scoped {
        format!(" scope={scope}")
    } else {
        String::new()
    };
    ok(
        tw,
        GOD,
        &format!(
            "@property/create {room}=#{} tier=3 landlord=org:{org}{scope_arg}",
            id.0
        ),
    );
    store(tw).properties().last().expect("property").id
}

/// The rig's property is let by `sys:wod.housing`, which types nothing and cannot be
/// a contract party. Exact mode needs a landlord who can be one: rebuild it under
/// Quinn, in a room of its own.
fn loft(tw: &mut TestWorld, r: &mut Rig) {
    ok(tw, GOD, "@dig Loft=uploft,downloft");
    let loft = room_named(tw, "Loft");
    ok(
        tw,
        GOD,
        &format!(
            "@property/create Loft=#{} tier=1 landlord=#{}",
            loft.0, r.quinn.0
        ),
    );
    r.property = store(tw).properties().last().expect("property").id;
    r.anchor = loft;
}

/// The money half of `exact_rig` without the tenancy: the faucet, and one account
/// each for Pat (funded), Quinn and Rex. Returns `(faucet, pat, quinn, rex)`.
fn money(tw: &mut TestWorld, r: &Rig, funds: i64) -> (u64, u64, u64, u64) {
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
    for who in [r.pat, r.quinn, r.rex] {
        ok(tw, GOD, &format!("@account/open #{}=gold", who.0));
    }
    let (faucet, pat, quinn, rex) = {
        let snap = tw
            .world()
            .module_state::<EconomyStore>()
            .expect("econ")
            .snapshot();
        let find = |who: ObjId| {
            snap.accounts()
                .find(|a| a.owner == mush_org::PrincipalId::Object(who))
                .expect("account")
                .id
                .0
        };
        let faucet = snap
            .accounts()
            .find(|a| matches!(a.owner, mush_org::PrincipalId::System(_)))
            .expect("faucet")
            .id
            .0;
        (faucet, find(r.pat), find(r.quinn), find(r.rex))
    };
    if funds > 0 {
        ok(tw, GOD, &format!("@econ/mint {faucet}={pat} {funds}"));
    }
    (faucet, pat, quinn, rex)
}

fn contract_of(tw: &TestWorld, id: TenancyId) -> mush_economy::ContractId {
    match tenancy(tw, id).funding {
        mush_housing::Funding::Exact { contract, .. } => contract,
        mush_housing::Funding::Abstract { .. } => panic!("tenancy #{id} is not exact"),
    }
}

fn obligation_of(tw: &TestWorld, id: TenancyId) -> mush_economy::ObligationId {
    match tenancy(tw, id).funding {
        mush_housing::Funding::Exact { obligation, .. } => obligation,
        mush_housing::Funding::Abstract { .. } => panic!("tenancy #{id} is not exact"),
    }
}

/// Drop every persisted document in `ns` whose key starts with `prefix` — the shape
/// of a restore whose two halves came from different snapshots.
fn forget(kv: &mut Kv, ns: &str, prefix: &str) {
    let entry = kv.get_mut(&SmolStr::new(ns)).expect("namespace");
    let before = entry.len();
    entry.retain(|(k, _)| !k.starts_with(prefix));
    assert!(entry.len() < before, "nothing matched {ns}:{prefix}");
}

// ---- Lens A: command authority and existence hiding ------------------------

/// A1. `@property/show|list` and `@tenancy/list` handed a guest the whole record —
/// landlord, tier, every room, the status and the occupancy — while `+home` and
/// `@tenancy/show` reduced for the same person.
#[test]
fn vf_a1_a_guests_four_read_verbs_all_reduce() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, r.pat, &format!("+guest add #{}", r.rex.0));

    let mut all = String::new();
    for line in [
        format!("@property/show {}", r.property),
        "@property/list".to_string(),
        format!("@tenancy/show {id}"),
        "@tenancy/list".to_string(),
    ] {
        let out = texts(&tw.run(r.rex, &line));
        assert!(
            out.contains("you are a guest here"),
            "`{line}` did not reduce: {out}"
        );
        all.push_str(&out);
        all.push('\n');
    }
    for leak in [
        "tier",
        "landlord",
        "rooms:",
        "tenants:",
        "Pat",
        "anchor",
        "tenant(s)",
        "active",
    ] {
        assert!(!all.contains(leak), "a guest was shown {leak:?}:\n{all}");
    }
    // The tenant still reads everything a tenant reads.
    let mine = ok(&mut tw, r.pat, &format!("@property/show {}", r.property));
    assert!(
        mine.contains("tier 2"),
        "the tenant lost their own record: {mine}"
    );
}

/// A2. The numeric `SupportLevel` is the reading `ruleset-wod` derives from a sheet,
/// and best-of publishes the *stronger* co-tenant's band. It was rendered to
/// tenants and to landlord officers alike.
#[test]
fn vf_a2_the_support_level_is_staff_only_and_the_assessment_word_is_not() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let p = org_property(&mut tw, r.quinn, "property.view", true, "Tower");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {p}=#{},#{} cadence={}",
            r.pat.0, r.rex.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    // Rex carries it and Pat does not, so Pat's line would describe Rex's sheet.
    ok(&mut tw, GOD, &format!("+plan {id}=#{} 4", r.rex.0));

    let staff = ok(&mut tw, GOD, &format!("@tenancy/show {id}"));
    assert!(
        staff.contains("best level 4"),
        "staff lost the number: {staff}"
    );
    assert!(
        staff.contains("plan(s)"),
        "staff lost the plan count: {staff}"
    );

    for (who, label) in [
        (r.pat, "the co-tenant"),
        (r.quinn, "the landlord's officer"),
    ] {
        let seen = ok(&mut tw, who, &format!("@tenancy/show {id}"));
        assert!(
            seen.contains("supported"),
            "{label} got no assessment at all: {seen}"
        );
        assert!(
            !seen.contains("best level"),
            "{label} read the support level: {seen}"
        );
        assert!(
            !seen.contains("plan(s)"),
            "{label} read the plan count: {seen}"
        );
    }
}

/// A3. `landlord=` fell through to an unfiltered list when it did not resolve, so
/// the *shape* of the answer said whether an organization exists.
#[test]
fn vf_a3_the_landlord_filter_is_not_an_org_existence_oracle() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, GOD, "@org/create Ironhold");

    let nowhere = texts(&tw.run(r.pat, "@property/list landlord=org:Nowhere"));
    let typo = texts(&tw.run(r.pat, "@property/list landlord=org:Ironhld"));
    let real = texts(&tw.run(r.pat, "@property/list landlord=org:Ironhold"));
    assert!(
        nowhere.contains(mush_housing::NO_SUCH_PROPERTY),
        "{nowhere}"
    );
    assert_eq!(
        nowhere, typo,
        "a typo is distinguishable from a nonexistent org"
    );
    assert_eq!(
        nowhere, real,
        "a real org is distinguishable from a nonexistent one"
    );
    // And it never widens: Pat's own home is in none of those answers.
    assert!(
        !real.contains("The Flat"),
        "the filter widened to everything: {real}"
    );
    assert!(ok(&mut tw, r.pat, "@property/list").contains("The Flat"));
}

/// A4. `may_view_property` asked only about `Property::landlord_scope`, so a
/// property created without `scope=` — the obvious thing to type — locked every one
/// of the landlord's officers out of the record while still letting them inspect it.
#[test]
fn vf_a4_an_officer_of_a_scope_less_property_reads_and_inspects_it() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let p = org_property(&mut tw, r.quinn, "property.manage", false, "Tower");
    assert!(
        store(&tw)
            .property(p)
            .expect("property")
            .landlord_scope
            .is_none()
    );

    let seen = ok(&mut tw, r.quinn, &format!("@property/show {p}"));
    assert!(
        seen.contains("tier 3"),
        "the officer cannot read their own property: {seen}"
    );
    let listed = ok(&mut tw, r.quinn, "@property/list landlord=org:Ironhold");
    assert!(
        listed.contains(&format!("#{p} ")),
        "the officer's own list is empty: {listed}"
    );

    // One authority path: the same compartment answers the inspection term.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {p}=#{} cadence={} inspect=org:Ironhold:always",
            r.pat.0, r.cadence
        ),
    );
    let tower = room_named(&tw, "Tower");
    ok(&mut tw, r.quinn, "inTower");
    assert_eq!(
        tw.world().get(r.quinn).and_then(|o| o.location),
        Some(tower)
    );
    err(&mut tw, r.rex, "inTower");
}

/// A5. `guest_only` asked only about `property.view` on the compartment, so
/// inviting a manage-only officer in demoted them to "just a guest".
#[test]
fn vf_a5_an_invitation_does_not_demote_a_manage_only_officer() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let p = org_property(&mut tw, r.quinn, "property.manage", true, "Tower");
    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/create {p}=#{} cadence={}", r.pat.0, r.cadence),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;

    let before = ok(&mut tw, r.quinn, &format!("@tenancy/show {id}"));
    assert!(
        before.contains("tenants:"),
        "the officer could not read it at all: {before}"
    );
    // A courtesy, not a demotion.
    ok(&mut tw, r.pat, &format!("+guest add #{}", r.quinn.0));
    let after = ok(&mut tw, r.quinn, &format!("@tenancy/show {id}"));
    assert!(
        after.contains("tenants:"),
        "an invitation demoted the officer: {after}"
    );
    assert!(!after.contains("you are a guest here"), "{after}");
}

/// A6. Both player verbs took the lowest-id live tenancy, silently — and
/// `+home/leave` is the one irreversible act in the whole player surface.
#[test]
fn vf_a6_home_leave_and_guest_never_guess_which_home() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let first = tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, GOD, "@dig Loft=uploft,downloft");
    let loft = room_named(&tw, "Loft");
    ok(
        &mut tw,
        GOD,
        &format!("@property/create Loft=#{} tier=1 landlord=housing", loft.0),
    );
    let other = store(&tw).properties().last().expect("property").id;
    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/create {other}=#{} cadence={}", r.pat.0, r.cadence),
    );
    let second = store(&tw).tenancies().last().expect("tenancy").id;

    // `+guest` refuses, and `home=` disambiguates.
    let seen = err(&mut tw, r.pat, &format!("+guest add #{}", r.rex.0));
    assert!(seen.contains("more than one place"), "{seen}");
    ok(
        &mut tw,
        r.pat,
        &format!("+guest add #{} home={first}", r.rex.0),
    );
    assert!(tenancy(&tw, first).is_guest(r.rex));
    assert!(!tenancy(&tw, second).is_guest(r.rex));

    // `+home/leave` refuses, lists both, and ends nothing.
    let seen = err(&mut tw, r.pat, "+home/leave");
    assert!(
        seen.contains(&format!("#{first}")) && seen.contains(&format!("#{second}")),
        "{seen}"
    );
    assert_eq!(status(&tw, first), TenancyStatusKind::Active);
    assert_eq!(status(&tw, second), TenancyStatusKind::Active);

    // Naming one ends exactly that one, and re-homes them on the other (E5a fold A5).
    ok(&mut tw, r.pat, &format!("+home/leave {second}"));
    assert_eq!(status(&tw, second), TenancyStatusKind::Ended);
    assert_eq!(status(&tw, first), TenancyStatusKind::Active);
    assert_eq!(home_of(&tw, r.pat), Some(r.anchor));
    // Somebody else's tenancy is hidden, not refused for a different reason.
    let mine = err(&mut tw, r.rex, &format!("+home/leave {first}"));
    assert!(
        mine.contains("You have no home to leave.") || mine.contains(mush_housing::NO_SUCH_TENANCY),
        "{mine}"
    );
}

/// A7. `offer_lease` read `every=` and `tenancy_create` never collected it, so the
/// key fell through into the tenant list and the rent period silently stayed at the
/// default whatever anybody typed.
#[test]
fn vf_a7_every_reaches_the_lease_as_the_rent_period() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    loft(&mut tw, &mut r);
    let (_faucet, pat_acct, quinn_acct, _rex) = money(&mut tw, &r, 1_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={} mode=exact rent=10 from={pat_acct} \
             to={quinn_acct} every=600",
            r.property, r.pat.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    let contract = contract_of(&tw, id);
    ok(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    let o = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(obligation_of(&tw, id)).cloned())
        .expect("the rent obligation");
    assert_eq!(
        o.schedule.cadence,
        Some(600),
        "every= never reached the lease"
    );
    let help = tw
        .registry()
        .spec_of("@tenancy")
        .expect("registered")
        .help
        .to_string();
    assert!(
        help.contains("every= is the rent period"),
        "the key is undocumented"
    );
}

/// A8. The help said nothing about who may read the show/list verbs, and
/// `@econ/cadence`'s doc comment claimed a gate the code has never had.
#[test]
fn vf_a8_the_help_states_the_read_audience() {
    let tw = boot();
    let help = |verb: &str| {
        tw.registry()
            .spec_of(verb)
            .expect("registered")
            .help
            .to_string()
    };

    let property = help("@property");
    assert!(
        property.contains("GUEST"),
        "@property/show|list does not state the guest audience"
    );
    assert!(
        property.contains("root scope"),
        "@property help omits the officer fallback"
    );
    let tenancy = help("@tenancy");
    assert!(
        tenancy.contains("§11 answer 3"),
        "@tenancy help omits the staff-only default"
    );
    assert!(
        tenancy.contains("assessment word"),
        "@tenancy help omits the staff/other split"
    );
    assert!(
        tenancy.contains("GUEST"),
        "@tenancy/show|list does not state the guest audience"
    );
    assert!(
        help("+home").contains("REFUSES and lists"),
        "+home/leave help hides the rule"
    );
    assert!(
        help("+guest").contains("home=<tenancy>"),
        "+guest help hides its disambiguator"
    );
}

// ---- Lens H: hooks and home safety -----------------------------------------

/// H1. The access hook's first question was the `Let` latch, so `Withdrawn` was a
/// master key: a spare room falling down — or a deliberate withdrawal — cancelled
/// every access rule on a home somebody still lived in.
#[test]
fn vf_h1_withdrawal_never_unlocks_a_home_somebody_lives_in() {
    for how in ["a room fell down", "staff withdrew it"] {
        let mut tw = boot();
        let r = rig(&mut tw);
        // A lamp inside, placed while the property was still vacant.
        ok(&mut tw, GOD, &format!("@teleport #{}", r.anchor.0));
        ok(&mut tw, GOD, "@create lamp");
        let lamp = tw
            .world()
            .iter()
            .find(|o| o.name == "lamp")
            .expect("lamp")
            .id;
        ok(&mut tw, GOD, "drop lamp");
        ok(&mut tw, GOD, &format!("@teleport #{}", LIMBO.0));
        let id = tenant(&mut tw, &r, r.pat, 3);

        if how == "a room fell down" {
            ok(&mut tw, GOD, &format!("+boom #{}", r.spare.0));
        } else {
            ok(&mut tw, GOD, &format!("@property/withdraw {}", r.property));
        }
        assert_eq!(
            property(&tw, r.property).status,
            PropertyStatus::Withdrawn,
            "{how}"
        );
        assert_eq!(
            status(&tw, id),
            TenancyStatusKind::Active,
            "{how} ended a home"
        );

        // Twice: live, and after a reboot — the latch is what boot re-derives.
        let mut kv = fold_kv(&mut tw);
        for round in ["live", "rebooted"] {
            if round == "rebooted" {
                reboot(&mut tw, kv.clone());
                assert_eq!(property(&tw, r.property).status, PropertyStatus::Withdrawn);
            }
            err(&mut tw, r.rex, "in");
            assert_eq!(
                tw.world().get(r.rex).and_then(|o| o.location),
                Some(LIMBO),
                "{how}/{round}: a stranger walked into a withdrawn home"
            );
            err(&mut tw, r.rex, &format!("get #{}", lamp.0));
            assert_eq!(
                tw.world().get(lamp).and_then(|o| o.location),
                Some(r.anchor),
                "{how}/{round}: a stranger reached into a withdrawn home"
            );
            // …and the tenant still has their key.
            ok(&mut tw, r.pat, "in");
            ok(&mut tw, r.pat, "out");
            drain_into(&mut tw, &mut kv);
        }
    }
}

/// H2. `move_attempt` only ever asked about the destination, so `@teleport` walked
/// a thing straight out of a home nobody was allowed to reach into — the hole
/// `carry_attempt` closes for `get`, reopened by a different verb.
#[test]
fn vf_h2_teleport_cannot_walk_a_thing_out_of_an_occupied_home() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // Rex's own yard and his own lamp: every part of this move is his to make.
    ok(&mut tw, GOD, "@dig Yard=toyard,fromyard");
    let yard = room_named(&tw, "Yard");
    ok(&mut tw, GOD, &format!("@chown #{}=Rex", yard.0));
    ok(&mut tw, GOD, "@create lamp");
    let lamp = tw
        .world()
        .iter()
        .find(|o| o.name == "lamp")
        .expect("lamp")
        .id;
    ok(&mut tw, GOD, "drop lamp");
    ok(&mut tw, GOD, &format!("@chown #{}=Rex", lamp.0));

    // The control: the same actor, lamp and destination, allowed while the lamp is
    // standing nowhere in particular.
    ok(
        &mut tw,
        r.rex,
        &format!("@teleport #{}=#{}", lamp.0, yard.0),
    );
    assert_eq!(tw.world().get(lamp).and_then(|o| o.location), Some(yard));
    // Into the still-vacant property, and then somebody moves in.
    ok(
        &mut tw,
        GOD,
        &format!("@teleport #{}=#{}", lamp.0, r.anchor.0),
    );
    tenant(&mut tw, &r, r.pat, 3);

    let seen = err(
        &mut tw,
        r.rex,
        &format!("@teleport #{}=#{}", lamp.0, yard.0),
    );
    assert!(
        seen.contains(mush_housing::REFUSED_CARRY),
        "wrong refusal: {seen}"
    );
    assert_eq!(
        tw.world().get(lamp).and_then(|o| o.location),
        Some(r.anchor),
        "a thing was teleported out of an occupied home"
    );
    // A person is never trapped by the same rule.
    ok(&mut tw, r.pat, "in");
    ok(&mut tw, r.pat, "out");
}

/// H3. `+home/leave` cleared the leaver's `Obj.home` unconditionally, so a
/// co-tenant of two leases who left one lost the pointer to the other.
#[test]
fn vf_h3_leaving_one_of_two_leases_keeps_the_other_home() {
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
    let first = store(&tw).tenancies().last().expect("tenancy").id;
    ok(&mut tw, GOD, "@dig Loft=uploft,downloft");
    let loft = room_named(&tw, "Loft");
    ok(
        &mut tw,
        GOD,
        &format!("@property/create Loft=#{} tier=1 landlord=housing", loft.0),
    );
    let other = store(&tw).properties().last().expect("property").id;
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {other}=#{},#{} cadence={}",
            r.pat.0, r.quinn.0, r.cadence
        ),
    );
    let second = store(&tw).tenancies().last().expect("tenancy").id;
    assert_eq!(
        home_of(&tw, r.pat),
        Some(loft),
        "the later activation owns the pointer"
    );

    // Pat leaves the FLAT — the lease their home does not point at.
    ok(&mut tw, r.pat, &format!("+home/leave {first}"));
    assert!(!tenancy(&tw, first).is_tenant(r.pat));
    assert_eq!(
        status(&tw, first),
        TenancyStatusKind::Active,
        "a co-tenant leaving ended a lease"
    );
    assert_eq!(
        home_of(&tw, r.pat),
        Some(loft),
        "leaving one lease cleared the other one's home"
    );

    // …and leaving the one it does point at clears it, because there is no other.
    ok(&mut tw, r.pat, &format!("+home/leave {second}"));
    assert_eq!(home_of(&tw, r.pat), None);
    assert_eq!(
        home_of(&tw, r.quinn),
        Some(loft),
        "a co-tenant's own home was collateral damage"
    );
}

/// H4. The anchor is exempt from the **thoroughfare** rule and nothing else — so a
/// room another module structurally owns has to be refused there too, and the
/// reserved list has to actually be wired.
#[test]
fn vf_h4_the_anchor_is_exempt_from_the_thoroughfare_rule_and_nothing_else() {
    // (a) a behavior-bound room — a simulation domain, an interior anchor, a
    // presentation space — refused in the one position the thoroughfare rule exempts.
    let mut tw = boot();
    let _r = rig(&mut tw);
    ok(&mut tw, GOD, "@dig Domain=indomain,outdomain");
    let domain = room_named(&tw, "Domain");
    tw.world_mut()
        .attach_behavior(
            domain,
            mush_core::behavior::BehaviorId::new("sim.domain").expect("well-formed"),
            mush_core::behavior::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("attached");
    let before = store(&tw).properties().count();
    let seen = err(
        &mut tw,
        GOD,
        &format!(
            "@property/create Domain=#{} tier=1 landlord=housing",
            domain.0
        ),
    );
    assert!(seen.contains("behavior binding"), "wrong refusal: {seen}");
    assert_eq!(
        store(&tw).properties().count(),
        before,
        "a refused create allocated a property"
    );

    // (b) a wiring-frozen reserved room — what `mush-server` now passes out of
    // `[onboarding]`. Learn the id the rig mints, then boot a world reserving it.
    let start = {
        let mut probe = boot();
        let _ = rig(&mut probe);
        ok(&mut probe, GOD, "@dig Start=instart,outstart");
        room_named(&probe, "Start")
    };
    let mut tw = boot_with(vec![start]);
    let _r = rig(&mut tw);
    ok(&mut tw, GOD, "@dig Start=instart,outstart");
    assert_eq!(
        room_named(&tw, "Start"),
        start,
        "the rig does not allocate ids deterministically"
    );
    let seen = err(
        &mut tw,
        GOD,
        &format!(
            "@property/create Start=#{} tier=1 landlord=housing",
            start.0
        ),
    );
    assert!(seen.contains("reserved"), "wrong refusal: {seen}");
}

/// H5. The ending sweep looked one level down, at `contents_of(room)` and no
/// further — so anything a tenant was carrying, and anything homed inside but
/// standing elsewhere, kept pointing into a house that was no longer theirs.
#[test]
fn vf_h5_the_end_sweep_finds_what_is_carried_and_what_is_homed_from_outside() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);
    ok(&mut tw, r.pat, "in");

    // A lamp homed at the anchor, in Pat's pocket: one level below the room.
    ok(&mut tw, GOD, "@create lamp");
    let lamp = tw
        .world()
        .iter()
        .find(|o| o.name == "lamp")
        .expect("lamp")
        .id;
    ok(&mut tw, GOD, &format!("@link #{}=#{}", lamp.0, r.anchor.0));
    tw.world_mut()
        .move_to(lamp, r.pat)
        .expect("into the pocket");
    // …and a crate homed in the spare room, standing nowhere near it.
    ok(&mut tw, GOD, "@create crate");
    let boxed = tw
        .world()
        .iter()
        .find(|o| o.name == "crate")
        .expect("crate")
        .id;
    ok(&mut tw, GOD, &format!("@link #{}=#{}", boxed.0, r.spare.0));
    ok(&mut tw, GOD, "drop crate");
    assert_eq!(tw.world().get(boxed).and_then(|o| o.location), Some(LIMBO));

    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/resolve {id}=end they moved out"),
    );
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert_eq!(home_of(&tw, r.pat), None);
    assert_eq!(
        home_of(&tw, lamp),
        None,
        "a carried, homed item still points into the house"
    );
    assert_eq!(
        home_of(&tw, boxed),
        None,
        "an item homed inside but standing out still points in"
    );
}

/// H6. The `ObjDestroyed` backstop and boot's first pass both skipped a property
/// that was already `Withdrawn` — so the second room of somebody's home, and the
/// anchor after it, fell down in silence.
#[test]
fn vf_h6_a_second_destruction_on_a_withdrawn_home_still_pages() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);

    ok(&mut tw, GOD, &format!("+boom #{}", r.spare.0));
    let first = property_alerts(&tw, r.property);
    assert_eq!(first.len(), 1, "the first destruction did not page");
    let alert = first[0].id;

    // A staffer acknowledges it without fixing anything, and the anchor falls down.
    ok(&mut tw, GOD, &format!("@econ/ack {alert}"));
    skip_wall(&mut tw, 7_200);
    ok(&mut tw, GOD, &format!("+boom #{}", r.anchor.0));
    let again = property_alerts(&tw, r.property);
    assert_eq!(again.len(), 1, "one property, one decision, one record");
    assert!(again[0].reopened > 0, "the second destruction was silent");
    assert!(again[0].is_open());
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "a destroyed room ended a home"
    );

    // …and a reboot re-pages a live tenancy on a property with nothing left of it.
    ok(&mut tw, GOD, &format!("@econ/ack {alert}"));
    skip_wall(&mut tw, 7_200);
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "boot ended a home"
    );
    assert!(
        property_alerts(&tw, r.property)
            .iter()
            .any(mush_economy::Escalation::is_open),
        "boot did not re-page a live tenancy on a fully destroyed property"
    );
}

// ---- Lens T: tick, ladder, exact mode, durability --------------------------

/// T1. The relapse carry-over inherited the previous window's escalation id, and
/// `window_due` answers `NotifyEscalated` forever for any window that has one — so
/// a home that recovered and relapsed could never ask for a person again.
#[test]
fn vf_t1_a_relapsed_window_can_still_reach_a_person() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    for _ in 0..9 {
        period(&mut tw, &r, 1);
    }
    let first = tenancy_alerts(&tw, id);
    assert_eq!(first.len(), 1, "the window never paged");
    let alert = first[0].id;

    // Recovery acknowledges the page and parks the window (E5a fold, S8).
    ok(&mut tw, GOD, &format!("+plan {id}=#{} 4", r.pat.0));
    period(&mut tw, &r, 1);
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
    assert!(
        !alerts(&tw)
            .into_iter()
            .find(|e| e.id == alert)
            .expect("record")
            .is_open(),
        "recovery did not acknowledge the page"
    );
    skip_wall(&mut tw, 7_200);

    // A relapse inside the hysteresis: the same run of trouble, not a new one.
    ok(&mut tw, GOD, &format!("+plan {id}=#{} 0", r.pat.0));
    for _ in 0..4 {
        period(&mut tw, &r, 1);
    }
    assert_eq!(status(&tw, id), TenancyStatusKind::EvictionPending);
    let window = window_of(&tw, id);
    assert_eq!(
        window.notified,
        mush_housing::MAX_WINDOW_NOTICES,
        "the notice budget was re-armed"
    );
    assert!(
        window.escalation.is_none(),
        "the relapsed window inherited a dead escalation id"
    );

    // Two more periods spend WINDOW_ESCALATE and it pages again — the SAME record,
    // reopened, never a second alert about one home.
    for _ in 0..2 {
        period(&mut tw, &r, 1);
    }
    let again = tenancy_alerts(&tw, id);
    assert_eq!(again.len(), 1, "re-paging minted a second alert");
    assert_eq!(
        again[0].id, alert,
        "re-paging replaced the record instead of reopening it"
    );
    assert!(
        again[0].reopened > 0,
        "the relapsed window never paged again"
    );
    assert!(again[0].is_open());
}

/// T2. `from=` was any account id at all, so an exact tenancy could point a standing
/// per-period charge at a bystander who is not a party and is never asked — or at a
/// faucet, which is a standing mint.
#[test]
fn vf_t2_the_rent_obligation_debits_a_tenant_and_nobody_else() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    loft(&mut tw, &mut r);
    let (faucet, pat_acct, quinn_acct, rex_acct) = money(&mut tw, &r, 1_000);
    let before = store(&tw).tenancies().count();
    let make = |from: u64| {
        format!(
            "@tenancy/create {}=#{} cadence={} mode=exact rent=10 from={from} to={quinn_acct}",
            r.property, r.pat.0, r.cadence
        )
    };

    let seen = err(&mut tw, GOD, &make(rex_acct));
    assert!(
        seen.contains("TENANTS"),
        "wrong refusal for a bystander: {seen}"
    );
    let seen = err(&mut tw, GOD, &make(faucet));
    assert!(
        seen.contains("standing mint"),
        "wrong refusal for a faucet: {seen}"
    );
    assert_eq!(
        store(&tw).tenancies().count(),
        before,
        "a refused exact tenancy was written"
    );
    assert_eq!(
        tw.world()
            .module_state::<EconomyStore>()
            .map(|s| s.snapshot().contracts().count()),
        Some(0),
        "a refused exact tenancy left a lease behind"
    );
    // The legitimate one still builds.
    ok(&mut tw, GOD, &make(pat_acct));
    assert_eq!(store(&tw).tenancies().count(), before + 1);
}

/// T3. `extend` carried the escalation id into the fresh window, which silenced it
/// for good: three notices are advertised and none of them can be sent, and
/// `window_due` never answers `Escalate` again.
#[test]
fn vf_t3_extend_after_the_escalation_still_reaches_a_person() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    for _ in 0..9 {
        period(&mut tw, &r, 1);
    }
    let alert = tenancy_alerts(&tw, id)
        .first()
        .expect("the window paged")
        .id;
    // A staffer takes the alert and buys the tenant more time. It stays staff-only.
    ok(&mut tw, GOD, &format!("@econ/ack {alert}"));
    skip_wall(&mut tw, 7_200);
    err(&mut tw, r.pat, &format!("@tenancy/resolve {id}=extend"));
    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=extend"));

    let window = window_of(&tw, id);
    assert_eq!(
        window.notified, 0,
        "extend did not re-arm the notice budget"
    );
    assert!(window.job.is_some(), "extend lost the ST's ticket");
    assert!(
        window.escalation.is_none(),
        "extend carried a dead escalation into the fresh window"
    );

    // The three advertised notices actually arrive …
    let mut renotices = 0usize;
    for _ in 0..3 {
        let seen = texts(&period(&mut tw, &r, 1));
        renotices += usize::from(seen.contains("still waiting on a decision"));
    }
    assert_eq!(
        renotices,
        mush_housing::MAX_WINDOW_NOTICES as usize,
        "extend silenced the window"
    );
    // … and then it asks for a person again, on the same record.
    for _ in 0..2 {
        period(&mut tw, &r, 1);
    }
    let again = tenancy_alerts(&tw, id);
    assert_eq!(again.len(), 1, "extend minted a second alert");
    assert_eq!(again[0].id, alert);
    assert!(
        again[0].reopened > 0,
        "an extended window never paged again"
    );
}

/// T4. A recovery caused by a person — the rent obligation's creditor reinstating
/// it — acknowledged the page as `GOD`, so `@econ/alerts` said the game itself had
/// handled a private landlord's decision.
#[test]
fn vf_t4_a_landlords_reinstate_credits_the_ack_to_the_landlord() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    loft(&mut tw, &mut r);
    let (_faucet, pat_acct, quinn_acct, _rex) = money(&mut tw, &r, 0);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={} mode=exact rent=10 from={pat_acct} \
             to={quinn_acct} every=600",
            r.property, r.pat.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    let contract = contract_of(&tw, id);
    ok(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    let obligation = obligation_of(&tw, id);

    // No money at all: the rent misses on its own clock while the tenancy walks the
    // ladder on the campaign clock.
    for _ in 0..30 {
        ok(
            &mut tw,
            GOD,
            &format!("@econ/cadence advance {} 1", r.cadence),
        );
        let t = tw.world().now().saturating_add(3_600);
        tw.set_now(t);
        tw.fire_due(t.saturating_mul(1000));
        if status(&tw, id) == TenancyStatusKind::EvictionPending {
            break;
        }
    }
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::EvictionPending,
        "the exact ladder stalled"
    );
    // Then the window's own decay, on cadence periods alone.
    for _ in 0..8 {
        if !tenancy_alerts(&tw, id).is_empty() {
            break;
        }
        period(&mut tw, &r, 1);
    }
    let page = tenancy_alerts(&tw, id);
    assert_eq!(page.len(), 1, "the window never paged");
    assert!(page[0].is_open());

    // The landlord — the rent's creditor — puts it back on its feet themselves. That
    // is a recovery, not an ending, so it stays allowed; it is the *credit* that was
    // wrong.
    ok(
        &mut tw,
        r.quinn,
        &format!("@obligation/reinstate {obligation}"),
    );
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "the landlord's recovery was refused"
    );
    let acked = alerts(&tw)
        .into_iter()
        .find(|e| e.id == page[0].id)
        .expect("record");
    assert_eq!(
        acked.acked_by,
        Some(r.quinn),
        "the ack was credited to somebody else"
    );
}

/// T5. Boot's exact pass inspects only *live* tenancies, so a `Pending` draft whose
/// lease can never activate was invisible: it waits forever, and nothing says so.
#[test]
fn vf_t5_boot_pages_about_a_pending_tenancy_whose_lease_is_gone() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    loft(&mut tw, &mut r);
    let (_faucet, pat_acct, quinn_acct, _rex) = money(&mut tw, &r, 1_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={} mode=exact rent=10 from={pat_acct} to={quinn_acct}",
            r.property, r.pat.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    assert_eq!(status(&tw, id), TenancyStatusKind::Pending);

    // The two halves of the snapshot disagree: housing's draft outlived the lease.
    let mut kv = fold_kv(&mut tw);
    forget(&mut kv, "economy", "contract:");
    reboot(&mut tw, kv);

    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Pending,
        "boot ended a pending tenancy"
    );
    let paged = tenancy_alerts(&tw, id);
    assert_eq!(
        paged.len(),
        1,
        "boot did not page about a draft that can never activate"
    );
    assert!(
        paged[0]
            .recommended
            .iter()
            .any(|c| c == &format!("@tenancy/resolve {id}=end")),
        "the page does not carry the verb that answers it: {:?}",
        paged[0].recommended
    );
}

/// T6. "An unresolvable clock never makes anything happen" was the right rule with
/// no output at all: the ladder, the notices and the story window all stopped, and
/// nothing anywhere said why.
#[test]
fn vf_t6_a_tenancy_whose_clock_vanished_freezes_and_says_so() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    let mut kv = fold_kv(&mut tw);
    forget(&mut kv, "economy", "cadence:");
    reboot(&mut tw, kv);

    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "a missing clock moved a home"
    );
    let paged = tenancy_alerts(&tw, id);
    assert_eq!(
        paged.len(),
        1,
        "the frozen tenancy paged {} times",
        paged.len()
    );
    assert!(
        paged[0]
            .context
            .contains(&format!("cadence #{}", r.cadence)),
        "the page does not name the clock: {}",
        paged[0].context
    );
    // The tick asks again on every fire, and the ladder still shows one decision.
    for _ in 0..5 {
        fire(&mut tw);
    }
    assert_eq!(
        tenancy_alerts(&tw, id).len(),
        1,
        "the frozen tenancy paged once a fire"
    );
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
}

/// T7. The tick cloned every tenancy in the store on every fire — roster, guest
/// list, plans and a 32-entry history tail each — to discover that almost all of
/// them had ended. Ended tenancies are kept forever, so on a long-running world
/// they are most of the store.
#[test]
fn vf_t7_the_tick_walks_only_the_tenancies_that_have_not_ended() {
    let mut tw = boot();
    let r = rig(&mut tw);
    for _ in 0..500 {
        ok(
            &mut tw,
            GOD,
            &format!(
                "@tenancy/create {}=#{} cadence={}",
                r.property, r.pat.0, r.cadence
            ),
        );
        let id = store(&tw).tenancies().last().expect("tenancy").id;
        ok(
            &mut tw,
            GOD,
            &format!("@tenancy/resolve {id}=end they moved on"),
        );
    }
    let live = tenant(&mut tw, &r, r.pat, 3);
    assert_eq!(
        store(&tw).tenancies().count(),
        501,
        "the ended records were not kept"
    );
    let walked: Vec<TenancyId> = store(&tw).open_tenancy_ids().collect();
    assert_eq!(
        walked,
        vec![live],
        "the tick's working set is not just the live tenancy"
    );

    // …and it still assesses the one that matters, across a reboot: the index is
    // re-derived from the loaded statuses, never persisted.
    period(&mut tw, &r, 1);
    assert_eq!(status(&tw, live), TenancyStatusKind::Active);
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert_eq!(store(&tw).tenancies().count(), 501);
    assert_eq!(
        store(&tw).open_tenancy_ids().collect::<Vec<_>>(),
        vec![live]
    );
    period(&mut tw, &r, 1);
    assert_eq!(status(&tw, live), TenancyStatusKind::Active);
}

/// T8. `=scene` started a real core scene and *then* asked `attach_window`, which
/// refuses a second one — so the scene was created, joined and abandoned. And a
/// `SceneRef` persisted across a reboot names an id core has since reused, which
/// made the window unattachable for good.
#[test]
fn vf_t8_a_second_scene_is_refused_and_a_reboot_lets_one_attach_again() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    for _ in 0..4 {
        period(&mut tw, &r, 1);
    }
    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=scene"));
    let first = window_of(&tw, id).scene.expect("a scene was recorded");
    let scenes = tw.world().scenes().count();

    let seen = err(&mut tw, GOD, &format!("@tenancy/resolve {id}=scene"));
    assert!(
        seen.contains("already has a scene"),
        "wrong refusal: {seen}"
    );
    assert_eq!(
        window_of(&tw, id).scene,
        Some(first),
        "the window's scene changed"
    );
    assert_eq!(
        tw.world().scenes().count(),
        scenes,
        "a second scene was started and abandoned"
    );

    // A reboot clears the boot-local handle, so the ST can start one again.
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert_eq!(
        window_of(&tw, id).scene,
        None,
        "a boot-local scene ref survived the reboot"
    );
    ok(&mut tw, GOD, &format!("@tenancy/resolve {id}=scene"));
    assert!(
        window_of(&tw, id).scene.is_some(),
        "the window could not attach a scene again"
    );
    assert_eq!(status(&tw, id), TenancyStatusKind::EvictionPending);
}

/// T9. "The lease has activated but its rent obligation does not exist yet" is the
/// **expected** answer on `ContractChanged { Active }` — the contract's state write
/// happens before the economy creates its deliverables (08d §4 deviation 8) — so
/// logging it at `error!` meant every correct activation wrote a line that reads like
/// a broken one, which is how a real one stops being noticed.
///
/// The severity is a property of the *edge*, so that is what carries it: a log level
/// is not observable from a world, and `BindEdge` makes the rule assertable instead
/// of leaving it to a code review.
#[test]
fn vf_t9_the_contract_active_edge_is_the_happy_path_not_a_fault() {
    // The edge that fires before the economy has created anything: expected.
    assert!(
        !mush_housing::BindEdge::ContractActivated.missing_obligation_is_a_fault(),
        "the happy path is still logged as a fault"
    );
    // The obligation's own edge, and the tick's restart-safe catch-up: binding is
    // due by then, so a missing rent record is genuinely wrong and stays loud.
    assert!(mush_housing::BindEdge::BindingDue.missing_obligation_is_a_fault());

    // …and the path itself still works end to end: the quiet edge finds nothing, the
    // obligation's own edge binds it, and the tenancy goes live with a real id.
    let mut tw = boot();
    let mut r = rig(&mut tw);
    loft(&mut tw, &mut r);
    let (_faucet, pat_acct, quinn_acct, _rex) = money(&mut tw, &r, 1_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={} mode=exact rent=10 from={pat_acct} to={quinn_acct}",
            r.property, r.pat.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    let contract = contract_of(&tw, id);
    ok(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "the happy path did not activate"
    );
    assert_ne!(
        obligation_of(&tw, id),
        mush_housing::PENDING_OBLIGATION,
        "the sentinel survived a normal activation"
    );
}

// ======================================================================
// ---- look-back fold: D-LBK-1 + D-LBK-4a ----
// ======================================================================
//
// 08h F1/F4. Boot validated one document at a time, so a room in two properties, two
// live tenancies on one property, and a live tenancy on no property all booted — and
// the first two failed **open** (`property_of_room` answered the lowest id; if that
// property had no live tenancy the hook abstained at somebody's door). Pass 0 now
// quarantines every record involved: staff-only at the door, the destroy gate
// refuses, the tick skips, the rent is suspended through the economy's own seam,
// one page per conflict carries the repair, and nothing but the repair releases it.

fn quarantine_keys(tw: &TestWorld) -> Vec<String> {
    store(tw).quarantine().map(|c| c.key.to_string()).collect()
}

fn obligation_status(
    tw: &TestWorld,
    id: mush_economy::ObligationId,
) -> mush_economy::ObligationStatus {
    tw.world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(id).map(|o| o.status.clone()))
        .expect("the obligation record")
}

fn balance(tw: &TestWorld, acct: u64) -> i64 {
    tw.world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .account(mush_economy::AccountId(acct))
                .map(|a| a.posted_balance.minor())
        })
        .expect("the account")
}

/// A second property (Vacant, `housing`-let) in its own room, dug from Limbo.
fn second_property(tw: &mut TestWorld, name: &str) -> (PropertyId, ObjId) {
    ok(tw, GOD, &format!("@dig {name}=in{name},out{name}"));
    let room = room_named(tw, name);
    ok(
        tw,
        GOD,
        &format!(
            "@property/create {name}=#{} tier=1 landlord=housing",
            room.0
        ),
    );
    (store(tw).properties().last().expect("property").id, room)
}

/// (a) Two persisted properties sharing a room: both quarantined; a stranger AND the
/// tenant are refused at both anchors, staff admitted; `@destroy` of the room
/// refused; one page per conflict with the repair in the shipped syntax; and the
/// repair — `@property/rooms <id>=-<room>` — lifts it by itself.
#[test]
fn lb_dlbk1_two_properties_sharing_a_room_are_quarantined_fail_closed() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 3);
    let (other, loft) = second_property(&mut tw, "Loft");
    let mut kv = fold_kv(&mut tw);
    // A restore from two half-matching snapshots: the Loft's document also names the
    // Flat's spare room.
    patch(&mut kv, mush_housing::property_key(other).as_str(), |v| {
        v["rooms"] = serde_json::json!([loft.0, r.spare.0]);
    });
    reboot(&mut tw, kv);
    assert_projections_consistent(&tw);

    let key = format!("housing.corrupt:room:{}", r.spare.0);
    assert_eq!(
        quarantine_keys(&tw),
        vec![key.clone()],
        "the overlap was not quarantined"
    );
    assert_eq!(
        store(&tw).room_entry(r.spare),
        Some(&RoomEntry::Conflict(
            [r.property, other].into_iter().collect()
        ))
    );
    assert!(
        store(&tw).is_quarantined_property(r.property) && store(&tw).is_quarantined_property(other)
    );
    assert!(store(&tw).is_quarantined_tenancy(id));
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Active,
        "a quarantine moved the ladder"
    );

    // Fail closed at every room of every property involved: the stranger and the
    // TENANT alike, with the neutral text; staff walk in.
    for (who, label) in [(r.rex, "the stranger"), (r.pat, "the tenant")] {
        for (exit, room) in [("in", r.anchor), ("inLoft", loft)] {
            let seen = texts(&tw.run(who, exit));
            assert!(
                seen.contains(mush_housing::REFUSED),
                "{label} at {exit}: {seen}"
            );
            assert_ne!(
                tw.world().get(who).and_then(|o| o.location),
                Some(room),
                "{label} entered {exit}"
            );
        }
    }
    ok(&mut tw, GOD, "in");
    assert_eq!(
        tw.world().get(GOD).and_then(|o| o.location),
        Some(r.anchor),
        "staff were refused"
    );
    ok(&mut tw, GOD, "out");
    // The destroy gate refuses the room under review, and says why to a viewer.
    let seen = err(&mut tw, GOD, &format!("@destroy #{}", r.spare.0));
    assert!(
        seen.contains("under housing review"),
        "wrong refusal: {seen}"
    );
    assert!(
        tw.world().get(r.spare).is_some(),
        "the room was destroyed anyway"
    );
    // One page per conflict, the repair filled in with the shipped syntax.
    let pages: Vec<_> = alerts(&tw)
        .into_iter()
        .filter(|e| e.subject == EscalationSubject::HousingRoom(r.spare.0))
        .collect();
    assert_eq!(pages.len(), 1, "one conflict, one page: {pages:?}");
    assert!(pages[0].is_open());
    assert!(
        pages[0].context.contains(&key),
        "the page does not name the key: {}",
        pages[0].context
    );
    assert!(
        pages[0]
            .recommended
            .iter()
            .any(|c| c == &format!("@property/rooms {other}=-#{}", r.spare.0)),
        "the repair is not in the shipped syntax: {:?}",
        pages[0].recommended
    );
    // The staff verb lists it; `recheck` on the unrepaired conflict refuses and
    // keeps it.
    let seen = ok(&mut tw, GOD, "@housing/quarantine show");
    assert!(seen.contains(&key) && seen.contains("repair:"), "{seen}");
    let seen = err(&mut tw, GOD, &format!("@housing/quarantine recheck {key}"));
    assert!(seen.contains("still exists"), "{seen}");
    assert_eq!(
        quarantine_keys(&tw),
        vec![key.clone()],
        "recheck released an unrepaired conflict"
    );

    // Release only by re-validation: the Loft gives the room up, and the quarantine
    // lifts on that write with nothing else typed.
    ok(
        &mut tw,
        GOD,
        &format!("@property/rooms {other}=-#{}", r.spare.0),
    );
    assert!(
        quarantine_keys(&tw).is_empty(),
        "the repair did not lift the quarantine"
    );
    assert_eq!(
        store(&tw).room_entry(r.spare),
        Some(&RoomEntry::Unique(r.property))
    );
    ok(&mut tw, r.pat, "in");
    ok(&mut tw, r.pat, "out");
    err(&mut tw, r.rex, "in");
}

/// (b) Two live tenancies on one property at boot: both quarantined, the tick skips
/// both (positions unchanged after a cadence advance), no tenant notice; members read
/// "(under review)" and strangers the hidden text; ending one lifts it.
#[test]
fn lb_dlbk1_two_live_tenancies_on_one_property_are_skipped_by_the_tick() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let first = tenant(&mut tw, &r, r.pat, 0);
    let (other, _loft) = second_property(&mut tw, "Loft");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {other}=#{} cadence={}",
            r.quinn.0, r.cadence
        ),
    );
    let second = store(&tw).tenancies().last().expect("tenancy").id;
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, mush_housing::tenancy_key(second).as_str(), |v| {
        v["property"] = serde_json::json!(r.property.0);
    });
    reboot(&mut tw, kv);
    assert_projections_consistent(&tw);

    let key = format!("housing.corrupt:tenancy:{}", r.property);
    assert_eq!(quarantine_keys(&tw), vec![key.clone()]);
    assert_eq!(
        store(&tw).live_entry(r.property),
        Some(&LiveEntry::Conflict([first, second].into_iter().collect()))
    );
    assert!(
        store(&tw).live_tenancy_of(r.property).is_none(),
        "a conflict answered for a door"
    );
    let before = (
        store(&tw).assess_state(first),
        store(&tw).assess_state(second),
    );

    // A cadence advance assesses neither: no position moves, no status moves, and
    // nobody is told anything.
    let seen = texts(&period(&mut tw, &r, 1));
    assert!(
        !seen.contains("HOME:"),
        "a quarantined tenancy was notified: {seen}"
    );
    assert_eq!(
        (
            store(&tw).assess_state(first),
            store(&tw).assess_state(second)
        ),
        before,
        "the tick assessed a quarantined tenancy"
    );
    assert_eq!(
        status(&tw, first),
        TenancyStatusKind::Active,
        "a level-0 plan walked a quarantined tenancy"
    );
    assert_eq!(status(&tw, second), TenancyStatusKind::Active);

    // The renders: a member reads "(under review)"; a stranger the hidden text;
    // staff the record and the review.
    let mine = ok(&mut tw, r.pat, &format!("@tenancy/show {first}"));
    assert!(
        mine.contains("(under review)") && !mine.contains("tenants:"),
        "{mine}"
    );
    let home = ok(&mut tw, r.pat, "+home");
    assert!(
        home.contains("(under review)") && !home.contains("cadence #"),
        "{home}"
    );
    let prop = ok(&mut tw, r.pat, &format!("@property/show {}", r.property));
    assert!(
        prop.contains("(under review)") && !prop.contains("rooms:"),
        "{prop}"
    );
    let hidden = texts(&tw.run(r.rex, &format!("@tenancy/show {first}")));
    assert!(hidden.contains(mush_housing::NO_SUCH_TENANCY), "{hidden}");
    // A guest is "everyone else" while the record is under review: the hidden text,
    // not the guest line, and no `+home` line at all.
    ok(
        &mut tw,
        r.pat,
        &format!("+guest add #{} home={first}", r.rex.0),
    );
    let guest = texts(&tw.run(r.rex, &format!("@tenancy/show {first}")));
    assert!(
        guest.contains(mush_housing::NO_SUCH_TENANCY),
        "a guest read a record under review: {guest}"
    );
    let guest = texts(&tw.run(r.rex, &format!("@property/show {}", r.property)));
    assert!(guest.contains(mush_housing::NO_SUCH_PROPERTY), "{guest}");
    let guest = texts(&tw.run(r.rex, "+home"));
    assert!(!guest.contains("guest here"), "{guest}");
    let staff = ok(&mut tw, GOD, &format!("@tenancy/show {first}"));
    assert!(
        staff.contains("tenants:") && staff.contains("UNDER REVIEW"),
        "{staff}"
    );
    // One page, on the property, naming both endings.
    let pages: Vec<_> = alerts(&tw)
        .into_iter()
        .filter(|e| e.subject == EscalationSubject::HousingProperty(r.property.0))
        .collect();
    assert_eq!(pages.len(), 1);
    for t in [first, second] {
        assert!(
            pages[0]
                .recommended
                .iter()
                .any(|c| c == &format!("@tenancy/resolve {t}=end")),
            "{:?}",
            pages[0].recommended
        );
    }

    // The repair: end the second; the first is assessed on the next period.
    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/resolve {second}=end restored from a bad snapshot"),
    );
    assert!(
        quarantine_keys(&tw).is_empty(),
        "ending the duplicate did not lift the quarantine"
    );
    assert_eq!(
        store(&tw).live_entry(r.property),
        Some(&LiveEntry::Live(first))
    );
    period(&mut tw, &r, 1);
    assert_ne!(
        store(&tw).assess_state(first).position,
        before.0.position,
        "the lifted tenancy was not assessed"
    );
}

/// (c) A live tenancy on a missing property: quarantined, never assessed at tier 0,
/// paged with the ending; a reboot recomputes the same set from the records alone.
#[test]
fn lb_dlbk1_a_live_tenancy_on_a_missing_property_is_quarantined_and_paged() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, mush_housing::tenancy_key(id).as_str(), |v| {
        v["property"] = serde_json::json!(999);
    });
    reboot(&mut tw, kv.clone());
    assert_projections_consistent(&tw);

    let key = format!("housing.corrupt:orphan:{id}");
    assert_eq!(quarantine_keys(&tw), vec![key.clone()]);
    let before = store(&tw).assess_state(id);
    period(&mut tw, &r, 1);
    assert_eq!(
        store(&tw).assess_state(id),
        before,
        "a tier-0 assessment ran on a missing property"
    );
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
    let pages: Vec<_> = alerts(&tw)
        .into_iter()
        .filter(|e| e.subject == EscalationSubject::HousingTenancy(id.0))
        .collect();
    assert_eq!(pages.len(), 1);
    assert!(
        pages[0]
            .recommended
            .iter()
            .any(|c| c == &format!("@tenancy/resolve {id}=end")),
        "{:?}",
        pages[0].recommended
    );

    // Nothing is persisted: a second reboot recomputes exactly the same set.
    drain_into(&mut tw, &mut kv);
    assert!(
        housing_keys(&kv).iter().all(|k| !k.contains("quarantine")),
        "the quarantine was persisted: {:?}",
        housing_keys(&kv)
    );
    reboot(&mut tw, kv);
    assert_eq!(quarantine_keys(&tw), vec![key]);
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
}

/// Rent is **suspended, never merely paged**: an exact-mode quarantined tenancy's
/// obligation is held under the conflict's key at boot; a rent period that falls due
/// debits nothing and is recorded `Forgiven { Suspension }`; the repair resumes it,
/// and the next period debits.
#[test]
fn lb_dlbk1_a_quarantined_exact_tenancys_rent_is_suspended_and_resumes_on_repair() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    loft(&mut tw, &mut r);
    let (_faucet, pat_acct, quinn_acct, _rex) = money(&mut tw, &r, 1_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={} mode=exact rent=10 from={pat_acct} to={quinn_acct}",
            r.property, r.pat.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    let contract = contract_of(&tw, id);
    ok(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    let obligation = obligation_of(&tw, id);
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
    let (other, _annex) = second_property(&mut tw, "Annex");

    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, mush_housing::property_key(other).as_str(), |v| {
        let mut rooms = v["rooms"].as_array().cloned().unwrap_or_default();
        rooms.push(serde_json::json!(r.anchor.0));
        v["rooms"] = serde_json::Value::Array(rooms);
    });
    reboot(&mut tw, kv);
    assert_projections_consistent(&tw);

    let key = format!("housing.corrupt:room:{}", r.anchor.0);
    assert_eq!(quarantine_keys(&tw), vec![key.clone()]);
    let hold = mush_economy::SuspensionKey::new(&key).expect("a valid key");
    match obligation_status(&tw, obligation) {
        mush_economy::ObligationStatus::Suspended {
            reason: mush_economy::SuspendReason::Held { key: k },
        } => {
            assert_eq!(k, hold, "held under the wrong key");
        }
        other => panic!("the rent was not suspended at boot: {other:?}"),
    }
    // A rent period falls due (wall time; the rent is on the real clock) and a
    // campaign period passes: nothing is debited, the period is forgiven and says
    // why.
    skip_wall(&mut tw, 86_400);
    period(&mut tw, &r, 1);
    assert_eq!(
        balance(&tw, pat_acct),
        1_000,
        "a quarantined tenancy was charged rent"
    );
    let o = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(obligation).cloned())
        .expect("obligation");
    assert!(
        o.history.iter().any(|e| matches!(
            e,
            mush_economy::ObligationEvent::Forgiven {
                cause: mush_economy::ForgiveCause::Suspension,
                ..
            }
        )),
        "the suspended period was not forgiven with its cause: {:?}",
        o.history
    );
    assert!(o.held_by(&hold));

    // The repair lifts the quarantine and releases the hold; the next period debits.
    ok(
        &mut tw,
        GOD,
        &format!("@property/rooms {other}=-#{}", r.anchor.0),
    );
    assert!(quarantine_keys(&tw).is_empty());
    assert_eq!(
        obligation_status(&tw, obligation),
        mush_economy::ObligationStatus::Active,
        "the rent did not resume"
    );
    skip_wall(&mut tw, 86_400);
    period(&mut tw, &r, 1);
    assert_eq!(
        balance(&tw, pat_acct),
        990,
        "the resumed rent did not debit the next period"
    );
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
}

/// D-LBK-4a — the projections after every mutator equal a rebuild from the records,
/// and their entries say exactly what the records say (Unique/Live/Vacant).
#[test]
fn lb_dlbk4a_projections_match_a_rebuild_after_every_mutation() {
    let mut tw = boot();
    let r = rig(&mut tw);
    assert_eq!(
        store(&tw).room_entry(r.anchor),
        Some(&RoomEntry::Unique(r.property))
    );
    assert_eq!(store(&tw).live_entry(r.property), Some(&LiveEntry::Vacant));
    let id = tenant(&mut tw, &r, r.pat, 3);
    assert_eq!(
        store(&tw).live_entry(r.property),
        Some(&LiveEntry::Live(id))
    );
    ok(&mut tw, r.pat, &format!("+guest add #{}", r.rex.0));
    ok(&mut tw, r.pat, &format!("+guest remove #{}", r.rex.0));
    // Dug from inside the property, so it is a private room and not a thoroughfare.
    ok(&mut tw, GOD, &format!("@teleport #{}", r.anchor.0));
    ok(&mut tw, GOD, "@dig Annex=annex,back");
    ok(&mut tw, GOD, &format!("@teleport #{}", LIMBO.0));
    let annex = room_named(&tw, "Annex");
    // Adding a room, removing it again (after a withdrawal), withdrawing, and
    // putting the property back in service.
    ok(&mut tw, GOD, &format!("@property/withdraw {}", r.property));
    ok(
        &mut tw,
        GOD,
        &format!("@property/rooms {}=#{}", r.property, annex.0),
    );
    assert_eq!(
        store(&tw).room_entry(annex),
        Some(&RoomEntry::Unique(r.property))
    );
    ok(&mut tw, GOD, &format!("@property/withdraw {}", r.property));
    ok(
        &mut tw,
        GOD,
        &format!("@property/rooms {}=-#{}", r.property, annex.0),
    );
    assert_eq!(store(&tw).room_entry(annex), None);
    // Re-adding a room the property already holds is the no-op that puts a
    // withdrawn property whose rooms are all live back in service.
    ok(
        &mut tw,
        GOD,
        &format!("@property/rooms {}=#{}", r.property, r.spare.0),
    );
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Let);
    // The tenant leaving, a second tenancy, and a resolution.
    ok(&mut tw, r.pat, "+home/leave");
    assert_eq!(store(&tw).live_entry(r.property), Some(&LiveEntry::Vacant));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={}",
            r.property, r.quinn.0, r.cadence
        ),
    );
    let second = store(&tw).tenancies().last().expect("tenancy").id;
    assert_eq!(
        store(&tw).live_entry(r.property),
        Some(&LiveEntry::Live(second))
    );
    ok(&mut tw, GOD, &format!("@tenancy/resolve {second}=end"));
    assert_eq!(store(&tw).live_entry(r.property), Some(&LiveEntry::Vacant));
    // A programmatic destruction, last: the dead room stays a room of the record
    // (D-E5-1), so the projection keeps it too.
    ok(&mut tw, GOD, &format!("+boom #{}", r.spare.0));
    assert_projections_consistent(&tw);
    assert_eq!(
        store(&tw).room_entry(r.spare),
        Some(&RoomEntry::Unique(r.property))
    );
    // Every property has a row; every room of every property has one; nothing else.
    assert_eq!(store(&tw).room_entry(LIMBO), None);
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert_projections_consistent(&tw);
}

// ======================================================================
// ---- look-back fold: D-LBK-2 ----
// ======================================================================

/// `n` properties in `n` fresh rooms, each with one live tenancy of Pat's, seeded at
/// the clock's current position — injected as documents and booted, so a population
/// of a thousand costs a reboot rather than three thousand commands.
fn population(tw: &mut TestWorld, r: &Rig, n: u64) -> Vec<TenancyId> {
    use mush_core::ObjKind;
    let now = i64::try_from(
        tw.world()
            .module_state::<EconomyStore>()
            .and_then(|s| {
                s.snapshot()
                    .cadence(mush_economy::CadenceId(r.cadence))
                    .map(|c| c.position)
            })
            .expect("cadence"),
    )
    .unwrap_or(0);
    let landlord =
        mush_org::PrincipalId::System(SystemPrincipalId::new("wod.housing").expect("valid"));
    let mut kv = fold_kv(tw);
    let base_p = store(tw).properties().last().map_or(0, |p| p.id.0 + 1);
    let base_t = store(tw).tenancies().last().map_or(0, |t| t.id.0 + 1);
    let mut ids = Vec::new();
    for i in 0..n {
        let room = tw
            .world_mut()
            .create(ObjKind::Room, &format!("Cell {i}"), GOD, None)
            .expect("a room");
        let pid = PropertyId(base_p + i);
        let tid = TenancyId(base_t + i);
        let p = mush_housing::Property::new(
            pid,
            format!("Cell {i}"),
            room,
            [room].into_iter().collect(),
            mush_housing::PropertyTier::new(2),
            landlord.clone(),
        )
        .expect("a property");
        let t = mush_housing::Tenancy::new(
            tid,
            pid,
            [r.pat].into_iter().collect(),
            mush_economy::CadenceId(r.cadence),
        )
        .expect("a tenancy");
        let t =
            mush_housing::accept(&t, true, &[r.pat].into_iter().collect(), now).expect("accepted");
        let entry = kv.entry(SmolStr::new(NS)).or_default();
        entry.push((
            mush_housing::property_key(pid),
            serde_json::to_value(&p).expect("serializes"),
        ));
        entry.push((
            mush_housing::tenancy_key(tid),
            serde_json::to_value(&t).expect("serializes"),
        ));
        entry.push((
            mush_housing::assess_key(tid),
            serde_json::json!({ "position": now, "period": 0, "unknown_periods": 0, "seeded": true }),
        ));
        ids.push(tid);
    }
    reboot(tw, kv);
    assert_projections_consistent(tw);
    ids
}

/// A thousand open tenancies, none due: one fire visits at most 256 and advances the
/// cursor past the last **visited** id; the next fire visits the next block; after a
/// complete sweep with nothing due a fire visits nothing at all until the cadence
/// advances or a tenancy is created; at a period boundary a fire works at most 128
/// and later fires work the rest; and no tenancy is cloned before the due check.
#[test]
fn lb_dlbk2_a_fire_visits_at_most_256_works_at_most_128_and_idles_between_periods() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let ids = population(&mut tw, &r, 1_000);
    assert_eq!(store(&tw).open_tenancy_ids().count(), 1_000);

    // Nothing is due: four fires walk the population 256 at a time, cloning nothing.
    let mut cursors = Vec::new();
    for _ in 0..4 {
        fire(&mut tw);
        let f = store(&tw).last_fire();
        assert!(
            f.visited <= mush_housing::MAX_VISITED_PER_FIRE,
            "visited {}",
            f.visited
        );
        assert!(f.visited > 0, "an idle fire before the sweep was complete");
        assert_eq!(f.worked, 0);
        assert_eq!(f.cloned, 0, "a tenancy was cloned before the due check");
        cursors.push(store(&tw).cursor());
    }
    assert_eq!(
        cursors[0],
        ids[255].0 + 1,
        "the cursor did not advance past the last visited id"
    );
    assert_eq!(
        cursors[1],
        ids[511].0 + 1,
        "the second fire did not visit the next block"
    );
    // The sweep is complete and found nothing due: the next fire skips the scan.
    fire(&mut tw);
    let f = store(&tw).last_fire();
    assert_eq!(f.visited, 0, "a fire scanned after a complete idle sweep");
    assert!(f.short_circuited);
    // …until a tenancy is created, which bumps the population version…
    ok(&mut tw, GOD, "@dig Extra=inExtra,outExtra");
    let extra = room_named(&tw, "Extra");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@property/create Extra=#{} tier=1 landlord=housing",
            extra.0
        ),
    );
    let p = store(&tw).properties().last().expect("property").id;
    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/create {p}=#{} cadence={}", r.quinn.0, r.cadence),
    );
    fire(&mut tw);
    assert!(
        store(&tw).last_fire().visited > 0,
        "a new tenancy did not wake the scan"
    );
    for _ in 0..4 {
        fire(&mut tw);
    }
    assert!(
        store(&tw).last_fire().short_circuited,
        "the scan never went idle again"
    );

    // …or the cadence advances: now everything is due, and one fire works at most
    // 128 while later fires work the rest.
    ok(
        &mut tw,
        GOD,
        &format!("@econ/cadence advance {} 1", r.cadence),
    );
    let mut worked_total = 0usize;
    for _ in 0..12 {
        fire(&mut tw);
        let f = store(&tw).last_fire();
        assert!(
            f.worked <= mush_housing::MAX_WORKED_PER_FIRE,
            "worked {}",
            f.worked
        );
        assert!(f.visited <= mush_housing::MAX_VISITED_PER_FIRE);
        worked_total += f.worked;
    }
    assert_eq!(
        worked_total, 1_001,
        "the period was not spread over later fires: {worked_total}"
    );
}

// ======================================================================
// ---- look-back fold: D-LBK-3 (the notice half) ----
// ======================================================================

/// A 241-byte pending notice is dropped at boot with a log line; a 240-byte one is
/// kept and rendered at reconnect.
#[test]
fn lb_dlbk3_an_oversized_pending_notice_is_dropped_on_load_and_a_240_byte_one_kept() {
    let mut tw = boot();
    let r = rig(&mut tw);
    tenant(&mut tw, &r, r.pat, 0);
    tw.world_mut().set_connected(r.pat, false);
    period(&mut tw, &r, 1);
    let mut kv = fold_kv(&mut tw);
    let kept = "K".repeat(mush_housing::MAX_NOTICE_BYTES);
    let dropped = "D".repeat(mush_housing::MAX_NOTICE_BYTES + 1);
    patch(&mut kv, mush_housing::notice_key(r.pat).as_str(), |v| {
        let mut list = v.as_array().cloned().unwrap_or_default();
        list.push(serde_json::json!({ "at": 1, "text": dropped }));
        list.push(serde_json::json!({ "at": 2, "text": kept }));
        *v = serde_json::Value::Array(list);
    });
    reboot(&mut tw, kv);
    let queued = store(&tw).pending_notices(r.pat);
    assert_eq!(
        queued.len(),
        2,
        "the oversized notice was not dropped on its own: {queued:?}"
    );
    assert!(
        queued
            .iter()
            .all(|n| n.text.len() <= mush_housing::MAX_NOTICE_BYTES)
    );
    assert!(
        queued.iter().any(|n| n.text == kept),
        "the 240-byte notice was lost: {queued:?}"
    );
    // …and it is the one `+home` shows as waiting.
    let seen = ok(&mut tw, r.pat, "+home");
    assert!(
        seen.contains(&kept),
        "the kept notice is not rendered: {seen}"
    );
    assert!(
        !seen.contains(&dropped),
        "a 241-byte notice reached the render: {seen}"
    );
}

// ======================================================================
// ---- look-back fold: D-LBK-6 (housing's callers) ----
// ======================================================================

/// Housing closes its story job through the plain-job seam, naming its own queue:
/// the recovered ticket is `Resolved` with the closing note, in `wod-story`.
#[test]
fn lb_dlbk6_housing_resolves_its_story_job_through_the_plain_seam() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = tenant(&mut tw, &r, r.pat, 0);
    for _ in 0..4 {
        period(&mut tw, &r, 1);
    }
    let job = window_of(&tw, id).job.expect("the story job");
    assert_eq!(
        mush_jobs::get_job(tw.world(), job).expect("job").queue,
        mush_housing::STORY_QUEUE
    );
    ok(&mut tw, GOD, &format!("+plan {id}=#{} 4", r.pat.0));
    period(&mut tw, &r, 1);
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
    let job = mush_jobs::get_job(tw.world(), job).expect("job").clone();
    assert_eq!(job.status, mush_jobs::JobStatus::Resolved);
    assert!(job.payload.is_none());
    assert!(
        job.comments
            .iter()
            .any(|c| c.text.starts_with("Recovered:")),
        "the closing note is missing: {:?}",
        job.comments
    );
}

// ======================================================================
// ---- look-back fold: E5b-D34 ----
// ======================================================================

/// `Property.upkeep` is gone: an old document carrying it still loads, the render
/// has no upkeep line, `@property/create` does not accept the key, and the help no
/// longer advertises it.
#[test]
fn lb_e5bd34_an_old_document_carrying_upkeep_loads_and_is_not_rendered() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let mut kv = fold_kv(&mut tw);
    patch(
        &mut kv,
        mush_housing::property_key(r.property).as_str(),
        |v| {
            v["upkeep"] = serde_json::json!(500);
        },
    );
    reboot(&mut tw, kv);
    let seen = ok(&mut tw, GOD, &format!("@property/show {}", r.property));
    assert!(
        !seen.contains("upkeep"),
        "the dropped field was rendered: {seen}"
    );
    assert!(seen.contains("tier 2"), "{seen}");
    // Re-persisted, the field is gone for good.
    ok(&mut tw, GOD, &format!("@property/tier {}=3", r.property));
    let kv = fold_kv(&mut tw);
    let doc = kv
        .get(&SmolStr::new(NS))
        .and_then(|e| {
            e.iter()
                .find(|(k, _)| k.as_str() == mush_housing::property_key(r.property).as_str())
        })
        .map(|(_, v)| v.clone())
        .expect("the property doc");
    assert!(
        doc.get("upkeep").is_none(),
        "upkeep was written back: {doc}"
    );
    // Not accepted at creation: the token falls through as a room name and is refused.
    ok(&mut tw, GOD, "@dig Hut=inHut,outHut");
    let hut = room_named(&tw, "Hut");
    let seen = err(
        &mut tw,
        GOD,
        &format!(
            "@property/create Hut=#{} tier=1 landlord=housing upkeep=5",
            hut.0
        ),
    );
    assert!(seen.contains("No such room."), "{seen}");
    let help = tw
        .registry()
        .spec_of("@property")
        .expect("registered")
        .help
        .to_string();
    assert!(!help.contains("upkeep"), "the help still advertises upkeep");
}

// ======================================================================
// ---- look-back fold: rent-after-end ----
// ======================================================================
//
// 08g §13 deviation 8. Ending an exact tenancy cleared the homes and the latch and
// left the lease contract `Active`, so its rent obligation kept falling due and the
// former tenant was debited for a home they no longer had — the probe this section
// began as watched Pat go 1000 → 990 → 980 across `@tenancy/resolve <id>=end`. The
// end of a tenancy now ends the lease through the economy's own seam
// (`mush_economy::end_lease`), in the same step, on every path into `flow::end`; and
// boot's pass 8 catches a world that ended one before that was true.

fn contract_state(
    tw: &TestWorld,
    id: mush_economy::ContractId,
) -> Option<mush_economy::ContractState> {
    tw.world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(id).map(|c| c.state))
}

fn encumbrances_of(tw: &TestWorld, obj: ObjId) -> Vec<mush_economy::Encumbrance> {
    tw.world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().asset(obj).map(|a| a.encumbrances.clone()))
        .unwrap_or_default()
}

fn held_of(tw: &TestWorld, acct: u64) -> i64 {
    tw.world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .account(mush_economy::AccountId(acct))
                .map(|a| a.held_amount.minor())
        })
        .expect("the account")
}

/// How many journal transactions carry this contract's refund key (generation 0).
fn refund_posts(tw: &TestWorld, id: mush_economy::ContractId) -> usize {
    let key = mush_economy::refund_key(id);
    tw.world()
        .module_state::<EconomyStore>()
        .map(|s| s.transactions().filter(|t| t.idempotency == key).count())
        .unwrap_or(0)
}

fn tenancy_pages(tw: &TestWorld, id: TenancyId) -> Vec<mush_economy::Escalation> {
    alerts(tw)
        .into_iter()
        .filter(|e| e.subject == EscalationSubject::HousingTenancy(id.0))
        .collect()
}

/// One **bound** exact tenancy of `tenants` on the Loft (Quinn's), rent 10 a day from
/// Pat's account to Quinn's, with one period already paid (Pat 1000 → 990).
/// Returns `(tenancy, pat_acct, quinn_acct)`.
fn bound_lease(tw: &mut TestWorld, r: &mut Rig, tenants: &[ObjId]) -> (TenancyId, u64, u64) {
    loft(tw, r);
    let (_faucet, pat_acct, quinn_acct, _rex) = money(tw, r, 1_000);
    let roster: Vec<String> = tenants.iter().map(|o| format!("#{}", o.0)).collect();
    ok(
        tw,
        GOD,
        &format!(
            "@tenancy/create {}={} cadence={} mode=exact rent=10 from={pat_acct} to={quinn_acct}",
            r.property,
            roster.join(","),
            r.cadence
        ),
    );
    let id = store(tw).tenancies().last().expect("tenancy").id;
    let contract = contract_of(tw, id);
    for who in tenants {
        ok(tw, *who, &format!("@contract/accept {contract}"));
    }
    assert_eq!(
        status(tw, id),
        TenancyStatusKind::Active,
        "the lease did not bind"
    );
    skip_wall(tw, 86_400);
    period(tw, r, 1);
    assert_eq!(
        balance(tw, pat_acct),
        990,
        "one period of rent while the tenancy is live"
    );
    (id, pat_acct, quinn_acct)
}

/// After `@tenancy/resolve <id>=end` on a bound exact tenancy: the lease is
/// `Cancelled`, its rent `Cancelled`, the `Lease` pledge is off the anchor, nothing is
/// held against the tenant, the unwind posted exactly once under the contract's
/// refund key — and two more rent periods debit **nothing**.
#[test]
fn lb_rent_after_end_resolve_end_stops_the_rent_and_releases_the_pledge() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    let pat = r.pat;
    let (id, pat_acct, _quinn_acct) = bound_lease(&mut tw, &mut r, &[pat]);
    let contract = contract_of(&tw, id);
    let obligation = obligation_of(&tw, id);
    assert!(
        encumbrances_of(&tw, r.anchor).iter().any(
            |e| matches!(e, mush_economy::Encumbrance::Lease { contract: c, .. } if *c == contract)
        ),
        "the lease claim was never placed"
    );

    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/resolve {id}=end moved out"),
    );
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert_eq!(
        contract_state(&tw, contract),
        Some(mush_economy::ContractState::Cancelled),
        "the lease outlived the tenancy"
    );
    assert_eq!(
        obligation_status(&tw, obligation),
        mush_economy::ObligationStatus::Cancelled,
        "the rent outlived the tenancy"
    );
    assert!(
        encumbrances_of(&tw, r.anchor).is_empty(),
        "the lease claim stranded on the anchor: {:?}",
        encumbrances_of(&tw, r.anchor)
    );
    assert_eq!(held_of(&tw, pat_acct), 0);
    assert_eq!(
        refund_posts(&tw, contract),
        1,
        "the unwind did not post under the refund key exactly once"
    );
    assert!(
        tenancy_pages(&tw, id).is_empty(),
        "a clean ending paged: {:?}",
        tenancy_pages(&tw, id)
    );

    for _ in 0..2 {
        skip_wall(&mut tw, 86_400);
        period(&mut tw, &r, 1);
    }
    assert_eq!(
        balance(&tw, pat_acct),
        990,
        "the former tenant was debited after the end"
    );
    assert_eq!(
        obligation_status(&tw, obligation),
        mush_economy::ObligationStatus::Cancelled
    );
    assert_projections_consistent(&tw);
}

/// `+home/leave`: a co-tenant leaving is a roster change and the lease (and its
/// rent) runs on; the **last** tenant leaving ends the lease the same way a staff
/// resolution does.
#[test]
fn lb_rent_after_end_the_last_tenant_leaving_ends_the_lease_and_a_co_tenant_leaving_does_not() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    let (pat, rex) = (r.pat, r.rex);
    let (id, pat_acct, _quinn_acct) = bound_lease(&mut tw, &mut r, &[pat, rex]);
    let contract = contract_of(&tw, id);
    let obligation = obligation_of(&tw, id);

    ok(&mut tw, r.rex, "+home/leave");
    assert_eq!(status(&tw, id), TenancyStatusKind::Active);
    assert!(!tenancy(&tw, id).is_tenant(r.rex));
    assert_eq!(
        contract_state(&tw, contract),
        Some(mush_economy::ContractState::Active),
        "a co-tenant leaving ended the lease"
    );
    assert_eq!(
        obligation_status(&tw, obligation),
        mush_economy::ObligationStatus::Active
    );
    skip_wall(&mut tw, 86_400);
    period(&mut tw, &r, 1);
    assert_eq!(
        balance(&tw, pat_acct),
        980,
        "the live lease stopped charging when a co-tenant left"
    );

    ok(&mut tw, r.pat, "+home/leave");
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert_eq!(
        contract_state(&tw, contract),
        Some(mush_economy::ContractState::Cancelled)
    );
    assert_eq!(
        obligation_status(&tw, obligation),
        mush_economy::ObligationStatus::Cancelled
    );
    assert!(encumbrances_of(&tw, r.anchor).is_empty());
    skip_wall(&mut tw, 86_400);
    period(&mut tw, &r, 1);
    assert_eq!(
        balance(&tw, pat_acct),
        980,
        "the last tenant was debited after moving out"
    );
    assert_eq!(refund_posts(&tw, contract), 1);
}

/// An eviction decided at the story window (`@tenancy/resolve <id>=end` on an
/// `EvictionPending` exact tenancy) ends the lease too — and a former tenant who
/// later comes into money is charged nothing for it.
#[test]
fn lb_rent_after_end_an_eviction_end_ends_the_lease() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    loft(&mut tw, &mut r);
    // No funds at all: every settlement misses, and the ladder walks to the window.
    let (faucet, pat_acct, quinn_acct, _rex) = money(&mut tw, &r, 0);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={} mode=exact rent=10 from={pat_acct} to={quinn_acct}",
            r.property, r.pat.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    let contract = contract_of(&tw, id);
    ok(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    let obligation = obligation_of(&tw, id);
    for _ in 0..24 {
        ok(
            &mut tw,
            GOD,
            &format!("@econ/cadence advance {} 1", r.cadence),
        );
        let t = tw.world().now().saturating_add(86_400);
        tw.set_now(t);
        tw.fire_due(t.saturating_mul(1000));
        if status(&tw, id) == TenancyStatusKind::EvictionPending {
            break;
        }
    }
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::EvictionPending,
        "the exact ladder did not reach the window"
    );
    assert_eq!(
        contract_state(&tw, contract),
        Some(mush_economy::ContractState::Active)
    );

    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/resolve {id}=end no support was found"),
    );
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert_eq!(
        contract_state(&tw, contract),
        Some(mush_economy::ContractState::Cancelled),
        "the eviction left the lease live"
    );
    assert_eq!(
        obligation_status(&tw, obligation),
        mush_economy::ObligationStatus::Cancelled
    );
    assert!(encumbrances_of(&tw, r.anchor).is_empty());
    // Money arrives afterwards: none of it is rent for a home that is gone.
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={pat_acct} 1000"),
    );
    for _ in 0..2 {
        skip_wall(&mut tw, 86_400);
        period(&mut tw, &r, 1);
    }
    assert_eq!(
        balance(&tw, pat_acct),
        1_000,
        "an evicted tenant was charged rent afterwards"
    );
    assert_eq!(balance(&tw, quinn_acct), 0);
}

/// Boot, pass 8: an **ended** exact tenancy whose lease is still live (a world that
/// ended one before this fold, restored) — the lease is ended through the same seam
/// and the tenancy is paged once; the next rent period debits nothing.
#[test]
fn lb_rent_after_end_boot_ends_a_live_lease_behind_an_ended_tenancy_and_pages() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    let pat = r.pat;
    let (id, pat_acct, _quinn_acct) = bound_lease(&mut tw, &mut r, &[pat]);
    let contract = contract_of(&tw, id);
    let obligation = obligation_of(&tw, id);

    // The pre-fold shape: the tenancy document says Ended, and nothing was ever done
    // to the lease.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, mush_housing::tenancy_key(id).as_str(), |v| {
        v["status"] = serde_json::json!({ "Ended": { "how": "TenantLeft" } });
    });
    reboot(&mut tw, kv.clone());
    assert_projections_consistent(&tw);
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert_eq!(
        contract_state(&tw, contract),
        Some(mush_economy::ContractState::Cancelled),
        "boot left the lease live"
    );
    assert_eq!(
        obligation_status(&tw, obligation),
        mush_economy::ObligationStatus::Cancelled,
        "boot left the rent running"
    );
    assert!(encumbrances_of(&tw, r.anchor).is_empty());
    assert_eq!(refund_posts(&tw, contract), 1);
    assert_eq!(property(&tw, r.property).status, PropertyStatus::Vacant);
    let pages = tenancy_pages(&tw, id);
    assert_eq!(pages.len(), 1, "boot did not page exactly once: {pages:?}");
    assert_eq!(
        pages[0].kind,
        mush_economy::EscalationKind::BootReconciliation
    );
    assert!(pages[0].is_open());
    assert!(
        pages[0].context.contains("still live at boot")
            && pages[0].context.contains(&format!("#{contract}")),
        "{}",
        pages[0].context
    );
    assert!(
        pages[0]
            .recommended
            .iter()
            .any(|c| c == &format!("@obligation/show #{obligation}")),
        "{:?}",
        pages[0].recommended
    );

    skip_wall(&mut tw, 86_400);
    period(&mut tw, &r, 1);
    assert_eq!(
        balance(&tw, pat_acct),
        990,
        "the former tenant was debited after the boot repair"
    );
    // A second boot finds nothing: the lease is terminal, so pass 8 has no finding.
    drain_into(&mut tw, &mut kv);
    reboot(&mut tw, kv);
    assert_eq!(
        tenancy_pages(&tw, id).len(),
        1,
        "a terminal lease was paged again"
    );
    assert_eq!(refund_posts(&tw, contract), 1);
}

/// A lease that is already terminal is a no-op: the observer's own ending (staff
/// `@contract/cancel` → `Resolution::Settled`) reaches `flow::end` with a cancelled
/// contract, and nothing is posted twice, paged, or refused — across a reboot too.
#[test]
fn lb_rent_after_end_a_terminal_lease_is_a_no_op() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    let pat = r.pat;
    let (id, pat_acct, _quinn_acct) = bound_lease(&mut tw, &mut r, &[pat]);
    let contract = contract_of(&tw, id);
    let obligation = obligation_of(&tw, id);

    ok(&mut tw, GOD, &format!("@contract/cancel #{contract}"));
    assert_eq!(
        contract_state(&tw, contract),
        Some(mush_economy::ContractState::Cancelled)
    );
    assert_eq!(
        status(&tw, id),
        TenancyStatusKind::Ended,
        "the observer did not end the tenancy"
    );
    assert!(matches!(
        tenancy(&tw, id).status,
        mush_housing::TenancyStatus::Ended {
            how: mush_housing::Resolution::Settled
        }
    ));
    assert_eq!(
        obligation_status(&tw, obligation),
        mush_economy::ObligationStatus::Cancelled
    );
    assert_eq!(
        refund_posts(&tw, contract),
        1,
        "the ending sweep posted a second unwind"
    );
    assert!(
        tenancy_pages(&tw, id).is_empty(),
        "a terminal lease was paged: {:?}",
        tenancy_pages(&tw, id)
    );
    assert_eq!(home_of(&tw, r.pat), None);

    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert!(
        tenancy_pages(&tw, id).is_empty(),
        "boot paged about a terminal lease"
    );
    assert_eq!(refund_posts(&tw, contract), 1);
    skip_wall(&mut tw, 86_400);
    period(&mut tw, &r, 1);
    assert_eq!(balance(&tw, pat_acct), 990);
}

/// Ending a **pending** exact tenancy (the tenant never accepted — pass 4's own
/// recommended repair) ends the `Offered` lease with an empty plan, so a later
/// `@contract/accept` cannot activate a lease whose tenancy is gone and mint rent for
/// it.
#[test]
fn lb_rent_after_end_ending_a_pending_tenancy_ends_the_offered_lease() {
    let mut tw = boot();
    let mut r = rig(&mut tw);
    loft(&mut tw, &mut r);
    let (_faucet, pat_acct, quinn_acct, _rex) = money(&mut tw, &r, 1_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@tenancy/create {}=#{} cadence={} mode=exact rent=10 from={pat_acct} to={quinn_acct}",
            r.property, r.pat.0, r.cadence
        ),
    );
    let id = store(&tw).tenancies().last().expect("tenancy").id;
    let contract = contract_of(&tw, id);
    assert_eq!(status(&tw, id), TenancyStatusKind::Pending);
    assert_eq!(
        contract_state(&tw, contract),
        Some(mush_economy::ContractState::Offered)
    );

    ok(
        &mut tw,
        GOD,
        &format!("@tenancy/resolve {id}=end withdrawn"),
    );
    assert_eq!(status(&tw, id), TenancyStatusKind::Ended);
    assert_eq!(
        contract_state(&tw, contract),
        Some(mush_economy::ContractState::Cancelled),
        "the offered lease outlived its tenancy"
    );
    // Nothing was ever placed or escrowed, so nothing posted: the state write alone.
    assert_eq!(refund_posts(&tw, contract), 0);
    err(&mut tw, r.pat, &format!("@contract/accept {contract}"));
    let minted = tw
        .world()
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .obligations()
                .filter(|o| o.cause == mush_economy::ObligationCause::Contract(contract))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(minted, 0, "a cancelled lease minted rent");
    skip_wall(&mut tw, 86_400);
    period(&mut tw, &r, 1);
    assert_eq!(balance(&tw, pat_acct), 1_000);
}
