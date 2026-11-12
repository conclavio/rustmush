//! **Doc 45 cut AP.1 — an operation's notice is pushed to its eligible defenders** (Q-45-15,
//! Q-45-17).
//!
//! Until this cut an operation's notice was a lens: a defender saw the window only by typing
//! `@op/show`. These run the real commands and the real campaign tick over `TestWorld`, on
//! `e7c9.rs`'s composition, with the notice's composition seam
//! (`CampaignModule::with_defender_notice`) wired to a recorder, and assert what leaves through
//! it: who is told, when, how often and what.
//!
//! **What this does not pin.** The seam's production function
//! (`mush_composition::campaign_notice`) joins TacSim's reconnect queue and `mush-mail`; neither
//! is in this crate's reach, by design, so the queued line and the inbox are not read here.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_campaign::{
    CampaignStore, DEFENDER_NOTICE, DefenderLetter, OPERATION_NOTICE_MIN, OperationDoc, OperationId,
};
use mush_core::{Audience, ModuleBootData, ModuleWrite, ObjId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

/// What left through the seam: each defender and their letter, in order.
type Log = Arc<Mutex<Vec<(ObjId, DefenderLetter)>>>;

type Kv = BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

/// `e7c9.rs`'s composition (org, faction, canon, economy, campaign), the notice wired to `log`
/// when one is given.
fn boot(log: Option<&Log>) -> TestWorld {
    let campaign = mush_campaign::CampaignModule::new();
    let campaign = match log {
        Some(log) => {
            let log = Arc::clone(log);
            campaign.with_defender_notice(Arc::new(
                move |_ctx: &mut mush_core::Ctx<'_>, who: ObjId, letter: &DefenderLetter| {
                    if let Ok(mut rows) = log.lock() {
                        rows.push((who, letter.clone()));
                    }
                },
            ))
        }
        None => campaign,
    };
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![]).with_canon_outcome_kinds(vec![
                SmolStr::new_static(mush_campaign::KIND_OPERATION_RESOLVED),
            ]),
        ),
        Box::new(campaign),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, event)| text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `line` as `actor`, refusing an error.
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let events = tw.run(actor, line);
    let text = texts(&events);
    let refused = events.iter().any(|(_, event)| {
        matches!(event, OutputEvent::Sys(text)
            if text.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
    });
    assert!(!refused, "`{line}` was refused: {text}");
    text
}

/// One campaign tick fire, and what it said.
fn fire(tw: &mut TestWorld) -> String {
    let period = i64::try_from(mush_campaign::TICK_PERIOD_SECS).unwrap_or(15);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    texts(&tw.fire_due(t.saturating_mul(1_000)))
}

fn doc(tw: &TestWorld, id: OperationId) -> OperationDoc {
    tw.world()
        .module_state::<CampaignStore>()
        .expect("the campaign store")
        .operations()
        .find(|doc| doc.id() == id)
        .cloned()
        .expect("the operation")
}

/// Declare an operation as `who` and name `defenders` eligible, through the real commands.
fn declared(tw: &mut TestWorld, who: ObjId, defenders: &[ObjId], visibility: &str) -> OperationId {
    ok(tw, who, "@op/declare hold the refinery");
    let id = tw
        .world()
        .module_state::<CampaignStore>()
        .expect("the campaign store")
        .operations()
        .map(OperationDoc::id)
        .max()
        .expect("one operation");
    for defender in defenders {
        ok(
            tw,
            who,
            &format!("@op/terms {}/defender=#{}", id.0, defender.0),
        );
    }
    ok(
        tw,
        who,
        &format!("@op/terms {}/visibility={visibility}", id.0),
    );
    id
}

fn fold_into(tw: &mut TestWorld, kv: &mut Kv) {
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

fn reboot(tw: &mut TestWorld, kv: &Kv) {
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

/// **An eligible defender is told without typing `@op/show`: once, when the terms freeze, with
/// the whole notice period still ahead of them.** Nothing is pushed while the operation is only
/// declared. The issuer sits on the declaration for three IC days before activating: the notice
/// is counted from the activation, so the window the defenders are told of is still 28 IC days
/// off. Each named defender is told once; a bystander and the issuer are told nothing; ticks and
/// a restart tell nobody again; and the letter carries the window, never the plan.
#[test]
fn ap1_an_activated_operations_notice_reaches_each_eligible_defender_once() {
    let log: Log = Arc::default();
    let mut tw = boot(Some(&log));
    let mut kv = Kv::new();
    let attacker = tw.spawn_player("Attacker");
    let first = tw.spawn_player("First");
    let second = tw.spawn_player("Second");
    let bystander = tw.spawn_player("Bystander");
    let id = declared(&mut tw, attacker, &[first, second], "audience");

    // Declared, its terms still editable: nobody is told.
    for _ in 0..3 {
        let _ = fire(&mut tw);
    }
    assert!(
        log.lock().expect("the log").is_empty(),
        "nothing before the terms freeze"
    );
    assert!(!doc(&tw, id).runtime.noticed);

    // Three IC days later the issuer activates. The notice runs from now.
    let later = tw.world().now() + 3 * 86_400;
    tw.set_now(later);
    let said = ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    assert!(said.contains("Notice runs from now"), "{said}");
    let frozen = doc(&tw, id);
    let now_ic = tw.world().world_timeline().stamp_at(tw.world().now()).at;
    assert_eq!(
        frozen.terms.notice.at, now_ic,
        "the notice is stamped at the activation"
    );
    assert_eq!(
        frozen.terms.window.0.at - now_ic,
        OPERATION_NOTICE_MIN,
        "the whole notice period is still ahead"
    );
    assert!(frozen.consent_intact(), "the moved terms still hash");

    // The tick tells each named defender, once.
    let _ = fire(&mut tw);
    let told: Vec<ObjId> = log
        .lock()
        .expect("the log")
        .iter()
        .map(|row| row.0)
        .collect();
    assert_eq!(
        told,
        vec![first, second],
        "each eligible defender, in the terms' order"
    );
    assert!(!told.contains(&bystander) && !told.contains(&attacker));
    assert!(doc(&tw, id).runtime.noticed);
    let letter = log.lock().expect("the log")[0].1.clone();
    let head = DEFENDER_NOTICE
        .split("{days}")
        .next()
        .expect("the line's head")
        .replace("{id}", &id.to_string());
    assert!(letter.line.starts_with(&head), "{}", letter.line);
    assert!(
        letter.line.contains("27 IC days 23 hours"),
        "{}",
        letter.line
    );
    assert!(
        letter.body.contains("entitled to defend"),
        "{}",
        letter.body
    );
    for leak in [
        "hold the refinery",
        "Bond",
        "Participants",
        "State:",
        "declared by",
    ] {
        assert!(
            !letter.body.contains(leak),
            "the notice leaked {leak}: {}",
            letter.body
        );
    }

    // More ticks, and a restart: nobody is told again.
    for _ in 0..4 {
        let _ = fire(&mut tw);
    }
    fold_into(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    assert!(doc(&tw, id).runtime.noticed, "the mark is durable");
    for _ in 0..4 {
        let _ = fire(&mut tw);
    }
    assert_eq!(
        log.lock().expect("the log").len(),
        2,
        "told once each, ever"
    );
}

/// **A covert operation pushes nothing.** `visibility=covert` shows an eligible defender no
/// operation at all (`e7c9_x1`), so the push tells them nothing either: it never says more than
/// `@op/show` would.
#[test]
fn ap1_a_covert_operation_tells_nobody() {
    let log: Log = Arc::default();
    let mut tw = boot(Some(&log));
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    let id = declared(&mut tw, attacker, &[defender], "covert");
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    for _ in 0..3 {
        let _ = fire(&mut tw);
    }
    assert!(
        log.lock().expect("the log").is_empty(),
        "a covert operation is not announced"
    );
    assert!(
        doc(&tw, id).runtime.noticed,
        "and the tick does not ask again"
    );
}

/// **With no seam wired, a defender is still told the line.** A composition without mail or a
/// reconnect queue says it to the defender and keeps nothing.
#[test]
fn ap1_an_unwired_notice_is_still_said() {
    let mut tw = boot(None);
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    let id = declared(&mut tw, attacker, &[defender], "audience");
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    let said = fire(&mut tw);
    assert!(
        said.contains(&format!("Operation {id} names you an eligible defender")),
        "{said}"
    );
    assert!(!fire(&mut tw).contains("eligible defender"), "once");
}
