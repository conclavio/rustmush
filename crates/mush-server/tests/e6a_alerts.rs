//! E6a step 1, S10 (DL-05): the ladder's pages reach connected wizards through
//! `mush_server::econ_alerts::StaffAlerts`, on the ladder's own re-notify schedule.
//!
//! What the tests hold to: an open pages every connected Wizard-or-better once and
//! nobody else; the next sweep — which dedupes the page to a standing record and
//! emits nothing — pages nobody; a reminder pages once per schedule step (1 h, then
//! daily) and never in between; an acknowledgement whose cause comes back reopens
//! the record and pages again.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::{Audience, Flags, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId, ObjKind};
use mush_economy::{AccountId, ContractId, ContractState, EconomyStore, EscalationSubject};
use mush_org::{CapabilityId, PrincipalId};
use mush_proto::OutputEvent;
use mush_server::econ_alerts::StaffAlerts;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

/// The configured stale age, well inside the 7-day default.
const STALE_AGE: i64 = 3_600;

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

fn boot() -> TestWorld {
    let config = mush_economy::EconomyConfig {
        stale_contract_age_secs: STALE_AGE,
        ..mush_economy::EconomyConfig::default()
    };
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_config(config)
                .with_observers(vec![Arc::new(StaffAlerts::default())]),
        ),
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
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}
fn account_of(tw: &TestWorld, owner: ObjId) -> AccountId {
    store(tw)
        .snapshot()
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(owner))
        .expect("account")
        .id
}
/// One sweep fire at wall-clock second `t`, returning what everyone was told.
fn sweep_at(tw: &mut TestWorld, t: i64) -> Vec<(Audience, OutputEvent)> {
    tw.set_now(t);
    tw.fire_due(t.saturating_mul(1000))
}
/// How many alert lines `viewer` was handed in `ev`.
fn alerts_seen(tw: &TestWorld, ev: &[(Audience, OutputEvent)], viewer: ObjId) -> Vec<String> {
    tw.seen_by(ev, viewer)
        .into_iter()
        .filter(|l| l.starts_with("ECONOMY ALERT"))
        .collect()
}

/// A wizard, a civilian, and a live contract with no condition progress — the
/// shape the stale-contract page is for.
struct Rig {
    wizard: ObjId,
    civilian: ObjId,
    contract: ContractId,
}

fn rig(tw: &mut TestWorld) -> Rig {
    tw.set_now(1_000);
    let wizard = tw.spawn_player("Wiz");
    tw.world_mut()
        .set_flag(wizard, Flags::WIZARD, true)
        .expect("a flag");
    let civilian = tw.spawn_player("Civ");
    let johnson = tw.spawn_player("Johnson");
    let runner = tw.spawn_player("Runner");
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, &format!("@account/open #{}=gold", johnson.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", runner.0));
    let faucet = store(tw)
        .snapshot()
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let j_acct = account_of(tw, johnson);
    let r_acct = account_of(tw, runner);
    ok(tw, GOD, &format!("@econ/mint {faucet}={j_acct} 1000"));
    ok(tw, johnson, "@contract/draft Stuck");
    let id = store(tw)
        .snapshot()
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("the draft");
    ok(tw, johnson, &format!("@contract/party {id}=#{}", runner.0));
    ok(tw, johnson, &format!("@contract/pay {id}={j_acct} 300"));
    ok(tw, johnson, &format!("@contract/split {id}={r_acct} 300"));
    ok(tw, johnson, &format!("@contract/remainder {id}={j_acct}"));
    ok(
        tw,
        johnson,
        &format!("@contract/condition {id}=attest staff"),
    );
    ok(tw, johnson, &format!("@contract/offer {id}"));
    ok(tw, johnson, &format!("@contract/accept {id}"));
    ok(tw, runner, &format!("@contract/accept {id}"));
    assert_eq!(
        store(tw).snapshot().contract(id).expect("contract").state,
        ContractState::Active
    );
    Rig {
        wizard,
        civilian,
        contract: id,
    }
}

fn the_alert(tw: &TestWorld, contract: ContractId) -> mush_economy::Escalation {
    store(tw)
        .escalations()
        .find(|e| e.subject == EscalationSubject::Contract(contract))
        .cloned()
        .expect("the stale-contract page")
}

#[test]
fn e6s1_s10_opening_a_page_reaches_every_connected_wizard_once_and_nobody_else() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // Inside the configured age: no page, no line.
    let ev = sweep_at(&mut tw, 1_000 + STALE_AGE - 600);
    assert!(alerts_seen(&tw, &ev, r.wizard).is_empty());
    // The open: one line each to the wizard and to God, none to the civilian, and
    // the line already carries the answer.
    let opened_at = 1_000 + STALE_AGE + 60;
    let ev = sweep_at(&mut tw, opened_at);
    let wiz = alerts_seen(&tw, &ev, r.wizard);
    assert_eq!(
        wiz.len(),
        1,
        "the open did not page the wizard once: {wiz:?}"
    );
    assert!(wiz[0].starts_with("ECONOMY ALERT (opened)"), "{}", wiz[0]);
    assert!(
        wiz[0].contains(&format!("contract #{}", r.contract)),
        "the line does not name its subject: {}",
        wiz[0]
    );
    assert_eq!(alerts_seen(&tw, &ev, GOD).len(), 1);
    assert!(
        alerts_seen(&tw, &ev, r.civilian).is_empty(),
        "a civilian was paged"
    );
    let follow = tw.seen_by(&ev, r.wizard);
    assert!(
        follow
            .iter()
            .any(|l| l.contains("@contract/show") && l.contains("@econ/alerts")),
        "no recommended command followed the alert: {follow:?}"
    );
    // The very next fire: the ladder dedupes the page to its standing record and
    // announces nothing — no storm.
    let ev = sweep_at(&mut tw, opened_at + 60);
    assert!(
        alerts_seen(&tw, &ev, r.wizard).is_empty(),
        "the next fire re-paged"
    );
    assert_eq!(store(&tw).escalations().count(), 1);
}

#[test]
fn e6s1_s10_reminders_follow_the_ladders_schedule_and_never_repeat_inside_a_step() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let opened_at = 1_000 + STALE_AGE + 60;
    let ev = sweep_at(&mut tw, opened_at);
    assert_eq!(alerts_seen(&tw, &ev, r.wizard).len(), 1);
    // Before the first reminder is due: silence.
    let ev = sweep_at(
        &mut tw,
        opened_at + mush_economy::ESCALATION_RENOTIFY_SECS - 120,
    );
    assert!(alerts_seen(&tw, &ev, r.wizard).is_empty());
    // The first reminder (1 h): once.
    let first = opened_at + mush_economy::ESCALATION_RENOTIFY_SECS + 60;
    let ev = sweep_at(&mut tw, first);
    let wiz = alerts_seen(&tw, &ev, r.wizard);
    assert_eq!(wiz.len(), 1, "{wiz:?}");
    assert!(
        wiz[0].starts_with("ECONOMY ALERT (still open)"),
        "{}",
        wiz[0]
    );
    assert_eq!(the_alert(&tw, r.contract).notifications, 1);
    // Inside the same step: nothing, fire after fire.
    for k in 1..=3 {
        let ev = sweep_at(&mut tw, first + 60 * k);
        assert!(
            alerts_seen(&tw, &ev, r.wizard).is_empty(),
            "fire {k} re-paged"
        );
    }
    // The second reminder is daily.
    let ev = sweep_at(&mut tw, first + 43_200);
    assert!(
        alerts_seen(&tw, &ev, r.wizard).is_empty(),
        "a reminder came early"
    );
    let ev = sweep_at(&mut tw, first + 86_400 + 60);
    assert_eq!(alerts_seen(&tw, &ev, r.wizard).len(), 1);
    assert_eq!(the_alert(&tw, r.contract).notifications, 2);
}

#[test]
fn e6s1_s10_a_reopen_pages_again() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let opened_at = 1_000 + STALE_AGE + 60;
    let ev = sweep_at(&mut tw, opened_at);
    assert_eq!(alerts_seen(&tw, &ev, r.wizard).len(), 1);
    let alert = the_alert(&tw, r.contract);
    // Acknowledged without the cause being fixed: quiet for the cool-off...
    tw.set_now(opened_at + 30);
    ok(&mut tw, GOD, &format!("@econ/ack #{}", alert.id));
    assert!(!the_alert(&tw, r.contract).is_open());
    let ev = sweep_at(&mut tw, opened_at + 600);
    assert!(
        alerts_seen(&tw, &ev, r.wizard).is_empty(),
        "an acked record paged"
    );
    // ...then the same record reopens — same id — and pages again, once.
    let reopened_at = opened_at + 30 + mush_economy::ESCALATION_RENOTIFY_SECS + 60;
    let ev = sweep_at(&mut tw, reopened_at);
    let wiz = alerts_seen(&tw, &ev, r.wizard);
    assert_eq!(wiz.len(), 1, "the reopen did not page: {wiz:?}");
    assert!(wiz[0].starts_with("ECONOMY ALERT (reopened)"), "{}", wiz[0]);
    let again = the_alert(&tw, r.contract);
    assert_eq!(again.id, alert.id, "the reopen minted a new record");
    assert_eq!(again.reopened, 1);
    assert!(again.is_open());
    assert_eq!(store(&tw).escalations().count(), 1);
    let ev = sweep_at(&mut tw, reopened_at + 60);
    assert!(
        alerts_seen(&tw, &ev, r.wizard).is_empty(),
        "the reopen re-paged"
    );
}

// ======================================================================
// ---- E6a step 1 verify fold ----
//
// Lens R and D on the staff channel. The shape all three share: a page nobody
// could read is not a page, and the dedupe must not consume one.
// ======================================================================

/// Nobody staff-shaped is on duty — three in the morning on a one-staffer game —
/// and a Wizard-or-better who has **never connected in this process**, so their
/// arrival is a real first connection and fires the `Connected` hook.
fn nobody_on_duty(tw: &mut TestWorld, r: &Rig) -> ObjId {
    for who in [r.wizard, GOD] {
        tw.world_mut().set_connected(who, false);
    }
    let night = tw
        .world_mut()
        .create(ObjKind::Player, "NightWiz", GOD, Some(LIMBO))
        .expect("create");
    tw.world_mut()
        .set_flag(night, Flags::WIZARD, true)
        .expect("a flag");
    assert!(
        crate_connected_wizards_is_empty(tw),
        "somebody staff-shaped is still on duty"
    );
    night
}

/// The audience `StaffAlerts` fans out to, read the way the observer reads it.
fn crate_connected_wizards_is_empty(tw: &TestWorld) -> bool {
    !tw.world().connected().any(|a| {
        tw.world()
            .get(a)
            .is_some_and(|o| o.role() >= mush_core::Role::Wizard)
    })
}

#[test]
fn vf_r4_a_page_opened_with_nobody_on_duty_is_delivered_at_the_first_connect() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let night = nobody_on_duty(&mut tw, &r);

    // The ladder opens the record with no wizard connected. The dedupe used to be
    // recorded BEFORE the fan-out, so the step was consumed by an audience of nobody
    // — and the next thing anyone saw was a reminder an hour later saying "still
    // open" about a record they had never been told had opened.
    let opened_at = 1_000 + STALE_AGE + 60;
    let ev = sweep_at(&mut tw, opened_at);
    assert!(
        alerts_seen(&tw, &ev, night).is_empty(),
        "a disconnected wizard was somehow written to"
    );
    let alert = the_alert(&tw, r.contract);
    assert_eq!(alert.notifications, 0);

    // The wizard comes on duty. The economy re-announces its open backlog, and the
    // fan-out — which recorded nothing — delivers it.
    let ev = tw.connect(night);
    let lines = alerts_seen(&tw, &ev, night);
    assert_eq!(
        lines.len(),
        1,
        "the backlog never reached the first staffer to arrive: {lines:?}"
    );
    assert!(
        lines[0].starts_with("ECONOMY ALERT (opened)"),
        "the first thing they read was not the open: {}",
        lines[0]
    );
    assert!(
        lines[0].contains(&format!("contract #{}", r.contract)),
        "{}",
        lines[0]
    );

    // ...once. The next fire dedupes to the standing record, and a second connect
    // says nothing new.
    let ev = sweep_at(&mut tw, opened_at + 60);
    assert!(
        alerts_seen(&tw, &ev, night).is_empty(),
        "the next fire re-paged"
    );
    let ev = tw.connect(night);
    assert!(
        alerts_seen(&tw, &ev, night).is_empty(),
        "a second connect repeated the backlog"
    );
}

#[test]
fn vf_r5_a_boot_opened_page_reaches_a_wizard_who_connects_after_boot() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let night = nobody_on_duty(&mut tw, &r);

    // Boot opens its own level-2 records — a quarantined contract, an obligation
    // whose account went away — and announced NONE of them: only the ladder's `page`
    // announces, and boot does not call it. So a reconciliation page existed in
    // `@econ/alerts` and nothing ever said it out loud.
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != "economy" {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    let sys = PrincipalId::System(mush_org::SystemPrincipalId::new("faucet").expect("id"));
    let patched = serde_json::to_value(&sys).expect("a principal serializes");
    for (k, v) in &mut kv {
        if *k == SmolStr::new(format!("contract:{}", r.contract)) {
            v["consent"]["parties"]
                .as_array_mut()
                .expect("parties")
                .push(patched.clone());
        }
    }
    tw.modules_loaded(std::collections::HashMap::from([(
        SmolStr::new("economy"),
        ModuleBootData {
            kv,
            streams: std::collections::HashMap::new(),
            ..Default::default()
        },
    )]));
    let opened = store(&tw)
        .escalations()
        .find(|e| e.subject == EscalationSubject::Contract(r.contract))
        .cloned()
        .expect("boot quarantined without opening a record");
    assert_eq!(opened.notifications, 0);

    // The first staffer to arrive hears it.
    let ev = tw.connect(night);
    let lines = alerts_seen(&tw, &ev, night);
    assert_eq!(
        lines.len(),
        1,
        "a boot reconciliation page reached nobody: {lines:?}"
    );
    assert!(
        lines[0].contains(&format!("contract #{}", r.contract)),
        "{}",
        lines[0]
    );
}

/// The alerts world again, but holding on to the observer so the D5 test can look at
/// the map it keeps.
fn boot_watched() -> (TestWorld, Arc<StaffAlerts>) {
    let config = mush_economy::EconomyConfig {
        stale_contract_age_secs: STALE_AGE,
        ..mush_economy::EconomyConfig::default()
    };
    let alerts = Arc::new(StaffAlerts::default());
    let tw = TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_config(config)
                .with_observers(vec![alerts.clone()]),
        ),
    ]);
    (tw, alerts)
}

#[test]
fn vf_d5_the_dedupe_map_forgets_a_record_the_ladder_collected() {
    let (mut tw, alerts) = boot_watched();
    let r = rig(&mut tw);
    assert_eq!(alerts.tracked(), 0);
    let opened_at = 1_000 + STALE_AGE + 60;
    sweep_at(&mut tw, opened_at);
    let alert = the_alert(&tw, r.contract);
    assert_eq!(
        alerts.tracked(),
        1,
        "the fan-out did not record the step it delivered"
    );

    // The map only ever grew: the ladder's own GC collects an acknowledged record
    // seven days after it is acked, and this map never heard about it — so it kept
    // one entry per escalation the process had ever seen, for the life of the
    // process. Acknowledge it and then actually resolve the contract, so nothing
    // re-opens the record out from under the collector.
    tw.set_now(opened_at + 30);
    ok(&mut tw, GOD, &format!("@econ/ack #{}", alert.id));
    ok(&mut tw, GOD, &format!("@contract/dispute {}", r.contract));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@contract/resolve {}=refund settled out of band",
            r.contract
        ),
    );
    assert!(
        store(&tw)
            .snapshot()
            .contract(r.contract)
            .is_some_and(|c| c.state.is_terminal()),
        "the contract is still live and will be paged again"
    );

    let collected = opened_at + 30 + mush_economy::ESCALATION_GC_AFTER_SECS + 600;
    sweep_at(&mut tw, collected);
    assert!(
        store(&tw).escalation(alert.id).is_none(),
        "the ladder did not collect the acknowledged record"
    );

    // The next economic event of any kind sweeps the map against the live ladder —
    // the GC runs on a sweep, and a sweep announces nothing, so waiting for another
    // page would have waited forever.
    let faucet = store(&tw)
        .snapshot()
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let wallet = store(&tw)
        .snapshot()
        .accounts()
        .find(|a| !matches!(a.owner, PrincipalId::System(_)))
        .expect("a wallet")
        .id;
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={wallet} 1"));
    assert_eq!(
        alerts.tracked(),
        0,
        "the dedupe map is still holding a record the ladder collected"
    );
}
