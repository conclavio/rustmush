//! The shared E7 cut-9/10/11 acceptance rig for `mush-campaign`.
//!
//! Extracted verbatim from `e7c10.rs` by E7 cut 11 B0 so cut 11's driving-ingress
//! suite drives the SAME scene, the same NPCs and the same seat offers rather
//! than a second, subtly different world. Nothing here changed: only the
//! visibility, so a second test binary can reach it.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use mush_campaign::{CampaignStore, GrantState, OperationId, SeatOfferId};
use mush_core::{
    AccountId, AccountIdentity, Audience, BehaviorBinding, BehaviorId, ChannelId, GOD, LIMBO,
    ModuleBootData, ModuleWrite, ObjId, ObjKind, SeatAudit,
};
use mush_org::CapabilityId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

pub const NS: &str = "sim";

// ---- rig ------------------------------------------------------------------

/// A fixed in-memory account oracle: character → account id.
#[derive(Debug, Default)]
pub struct MapOracle(pub std::sync::Mutex<BTreeMap<ObjId, AccountId>>);

impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.lock().ok()?.get(&character).copied()
    }
}

/// The server's own `SeatAuditLog`, re-implemented here so this crate's tests
/// need no `mush-server` edge. Identical contract: process-local, boolean
/// answers, never a world document — and, after the E7 cut 10 verify fold
/// (K5/E7(a)), the OPAQUE MINT: the token is minted here, where the account row
/// lives, with a process nonce folded in, so the world cannot recompute it from
/// the seat's public coordinates.
pub static TEST_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Debug, Default)]
pub struct TestAudit(pub std::sync::Mutex<BTreeMap<String, AccountId>>);

impl SeatAudit for TestAudit {
    fn record(&self, seed: &str, account: AccountId) -> Option<String> {
        let n = TEST_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let token = format!("{n:016x}{:016x}", seed.len() as u64);
        let mut rows = self.0.lock().ok()?;
        rows.insert(token.clone(), account);
        Some(token)
    }
    fn knows(&self, token: &str) -> bool {
        self.0
            .lock()
            .map(|r| r.contains_key(token))
            .unwrap_or(false)
    }
    fn account_for(&self, token: &str) -> Option<AccountId> {
        self.0.lock().ok()?.get(token).copied()
    }
    fn forget(&self, token: &str) -> bool {
        self.0
            .lock()
            .map(|mut r| r.remove(token).is_some())
            .unwrap_or(false)
    }
}

/// An audit that refuses every write — the unwired/failed-store arm.
#[derive(Debug, Default)]
pub struct DeadAudit;

impl SeatAudit for DeadAudit {
    fn record(&self, _seed: &str, _account: AccountId) -> Option<String> {
        None
    }
    fn knows(&self, _token: &str) -> bool {
        false
    }
    fn account_for(&self, _token: &str) -> Option<AccountId> {
        None
    }
    fn forget(&self, _token: &str) -> bool {
        false
    }
}

pub fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_campaign::CAPABILITIES.iter().copied())
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

pub fn boot_with(audit: Arc<dyn SeatAudit>) -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(mush_campaign::CampaignModule::new().with_seat_audit(audit)),
    ])
}

pub fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

pub fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}

pub fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}

pub fn bind(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
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

pub fn camp(tw: &TestWorld) -> &CampaignStore {
    tw.world()
        .module_state::<CampaignStore>()
        .expect("campaign store")
}

/// The channels a resolver would actually see on `actor`'s grant.
pub fn granted(tw: &TestWorld, actor: ObjId, unit: ObjId, station: &str) -> Vec<String> {
    tw.world()
        .command_subjects_of(actor)
        .iter()
        .find(|g| g.subject == unit && g.station.as_str() == station)
        .map(|g| g.channels.iter().map(|c| c.as_str().to_owned()).collect())
        .unwrap_or_default()
}

pub fn epoch_of(tw: &TestWorld, actor: ObjId, unit: ObjId, station: &str) -> Option<u64> {
    tw.world()
        .command_subjects_of(actor)
        .iter()
        .find(|g| g.subject == unit && g.station.as_str() == station)
        .map(|g| g.epoch)
}

/// The kernel's own stale-intent predicate, per channel.
pub fn is_current(
    tw: &TestWorld,
    actor: ObjId,
    unit: ObjId,
    station: &str,
    epoch: u64,
    channel: &str,
) -> bool {
    tw.world().channel_grant_is_current(
        actor,
        unit,
        NS,
        station,
        epoch,
        &ChannelId::new(channel).unwrap(),
    )
}

pub struct Fx {
    pub field: ObjId,
    pub atlas: ObjId,
    pub locust: ObjId,
    /// The AI NPC that normally stands the Atlas pilot watch (also the seat's
    /// declared fallback).
    pub ace: ObjId,
    /// The AI NPC on the Locust.
    pub duce: ObjId,
    pub commander: ObjId,
    pub driver: ObjId,
    pub bystander: ObjId,
    pub oracle: Arc<MapOracle>,
}

/// A domain, two placed units with two-channel `pilot` stations, an AI NPC
/// billeted on each, a commander who owns them, a would-be proxy driver and a
/// bystander with no authority at all.
pub fn scene(audit: Arc<dyn SeatAudit>) -> (TestWorld, Fx) {
    let mut tw = boot_with(audit);
    let commander = tw.spawn_player("Commander");
    let driver = tw.spawn_player("Driver");
    let bystander = tw.spawn_player("Bystander");
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let atlas = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", commander, Some(LIMBO))
        .unwrap();
    let locust = tw
        .world_mut()
        .create(ObjKind::Thing, "Locust", commander, Some(LIMBO))
        .unwrap();
    let ace = tw
        .world_mut()
        .create(ObjKind::Npc, "Ace", commander, Some(LIMBO))
        .unwrap();
    let duce = tw
        .world_mut()
        .create(ObjKind::Npc, "Duce", commander, Some(LIMBO))
        .unwrap();
    bind(&mut tw, field, "sim.domain");
    bind(&mut tw, atlas, "sim.participant");
    bind(&mut tw, locust, "sim.participant");
    let oracle = Arc::new(MapOracle::default());
    if let Ok(mut map) = oracle.0.lock() {
        map.insert(commander, AccountId::mint("CommanderAcct", 1).unwrap());
        map.insert(driver, AccountId::mint("DriverAcct", 2).unwrap());
        map.insert(bystander, AccountId::mint("BystanderAcct", 3).unwrap());
    }
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .unwrap() = mush_sim::AccountOracle::new(oracle.clone());
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    );
    for unit in [atlas, locust] {
        ok(
            &mut tw,
            GOD,
            &format!("@spatial/place/domain #{}=0", unit.0),
        );
        ok(
            &mut tw,
            GOD,
            &format!(
                "@spatial/station #{}=pilot,pilot,channels:movement+fire",
                unit.0
            ),
        );
    }
    for (unit, npc) in [(atlas, ace), (locust, duce)] {
        ok(
            &mut tw,
            GOD,
            &format!("@spatial/assign #{}=pilot,#{}", unit.0, npc.0),
        );
        ok(
            &mut tw,
            GOD,
            &format!("@station/fallback #{}/pilot=#{}", unit.0, npc.0),
        );
        tw.world_mut().move_to(npc, field).unwrap();
    }
    for who in [commander, driver, bystander] {
        tw.world_mut().move_to(who, field).unwrap();
    }
    // **Warm the world-owned lease mirror** (fold B5 / decision E5(a)). Leg (2)
    // fails CLOSED on a mirror that has not yet seen the whole `control_leases`
    // family, because between boot and the first Coarse fire the mirror is empty
    // and self-declared incomplete while `control_leases` is durable K state that
    // survived the reboot — reading that absence as "nobody holds this" is the
    // fail-open cut 10 shipped. `mush-sim` refreshes it from its OWN owner-scoped
    // contexts, so one fire is what a live server does in its first 15 seconds.
    fire(&mut tw);
    (
        tw,
        Fx {
            field,
            atlas,
            locust,
            ace,
            duce,
            commander,
            driver,
            bystander,
            oracle,
        },
    )
}

/// Declare, make public, and commit both units — the operation every seat hangs
/// off. Returns its id.
pub fn operation(tw: &mut TestWorld, fx: &Fx) -> OperationId {
    let before: BTreeSet<u64> = camp(tw).operations().map(|d| d.id().0).collect();
    ok(tw, fx.commander, "@op/declare hold the refinery");
    let id = camp(tw)
        .operations()
        .map(|d| d.id())
        .find(|id| !before.contains(&id.0))
        .expect("one new operation");
    ok(tw, fx.commander, &format!("@op/terms {}/notice=24", id.0));
    ok(
        tw,
        fx.commander,
        &format!("@op/terms {}/visibility=public", id.0),
    );
    ok(
        tw,
        fx.commander,
        &format!("@op/join {}=#{}", id.0, fx.atlas.0),
    );
    ok(
        tw,
        fx.commander,
        &format!("@op/join {}=#{}", id.0, fx.locust.0),
    );
    id
}

/// Publish one seat on `unit`/`pilot` driven by `npc`, granting `channels`.
pub fn offer(
    tw: &mut TestWorld,
    fx: &Fx,
    op: OperationId,
    unit: ObjId,
    npc: ObjId,
    channels: &str,
) -> SeatOfferId {
    let before: BTreeSet<u64> = camp(tw).seat_offers().map(|o| o.id.0).collect();
    ok(
        tw,
        fx.commander,
        &format!(
            // A1: the seat's audience is the operation's own attacking side plus
            // whoever `to:` names. The driver is a third party with no unit and
            // no capability, so a seat they may take must SAY so.
            "+seat/offer {}/#{}/pilot=#{},{channels},to:#{}",
            op.0, unit.0, npc.0, fx.driver.0
        ),
    );
    camp(tw)
        .seat_offers()
        .map(|o| o.id)
        .find(|id| !before.contains(&id.0))
        .expect("one new seat offer")
}

pub fn grant_state(tw: &TestWorld, op: OperationId, id: SeatOfferId) -> Option<GrantState> {
    camp(tw).seat_grant(op, id).map(|g| g.state.clone())
}

/// Drain the engine's one-authoritative-submit-per-actor slot, exactly as the
/// world task's `CommandSubmitDriver` would between commands. Without it a
/// second seat act by the same NPC gets back-pressure ("your earlier action is
/// still committing"), which is correct behaviour and not what these tests are
/// about.
pub fn drain(tw: &mut TestWorld) -> Vec<mush_core::AuthoritativeSubmit> {
    let mut staged = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        staged.push(*submit);
    }
    staged
}

/// Drive the real disconnect ingress: the session layer's `SessionGone` after
/// `mush-session`'s linkdead grace, which is what fires `HookPoint::Disconnected`.
pub fn disconnect(tw: &mut TestWorld, actor: ObjId, conn: u64) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    // The transport drops (the actor goes LINK-DEAD and keeps the seat)...
    tw.engine_mut().step(
        mush_core::Inbound::ConnDown {
            conn,
            actor,
            reason: mush_core::DisconnectKind::Lost,
        },
        &mut out,
        &mut fx,
    );
    // ...and only when `linkdead_grace` expires does the session layer give up,
    // which is the message that fires `HookPoint::Disconnected`.
    tw.engine_mut()
        .step(mush_core::Inbound::SessionGone { actor }, &mut out, &mut fx);
}

pub fn fold_kv(tw: &mut TestWorld, kv: &mut BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>) {
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

pub fn reboot(tw: &mut TestWorld, kv: &BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, entries)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: entries.clone(),
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

/// One campaign tick fire (Coarse, so the clock must move a whole period).
pub fn fire(tw: &mut TestWorld) {
    let period = i64::try_from(mush_campaign::TICK_PERIOD_SECS).unwrap_or(15);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}
