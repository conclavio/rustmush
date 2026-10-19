//! Real player-module source dispatch over the parent's actual paid history.
//! Requests are made through ordinary line and structured ingress. No custom
//! QueryHandler or caller-created paid authorization is used here.
use super::*;
use mush_proto::{InvokeRequest, InvokeResult, MindRequest, MindResponse};

type Events = Vec<(Audience, OutputEvent)>;
fn raw(world: &mut PlayerWorld, input: Inbound) -> TestResult<Events> {
    let mut out = Outbox::new();
    world
        .engine
        .step(input, &mut out, &mut mush_core::engine::Effects::new());
    let events = out.take();
    world.commit()?;
    Ok(events)
}
fn invocation(actor: ObjId, conn: u64, id: u64, request: MindRequest) -> Inbound {
    Inbound::Invoke {
        actor,
        conn,
        id,
        request: InvokeRequest::Mind(request),
    }
}
fn source_request() -> MindRequest {
    MindRequest::FiledSource {
        licence: "2".into(),
        first: 1,
        last: 200,
    }
}
fn assert_source(events: &Events, conn: u64, permitted: bool) -> TestResult {
    let expected = crate::minds::workshop::source_lines(SOURCE, 1, 200)?;
    let mut answers = 0;
    for (audience, event) in events {
        if let OutputEvent::InvokeResult {
            result: InvokeResult::Mind(response),
            ..
        } = event
        {
            answers += 1;
            assert_eq!(*audience, Audience::Conn(conn));
            match response {
                MindResponse::FiledSource {
                    licence,
                    source_sha256,
                    first,
                    lines,
                } => {
                    assert!(permitted, "source escaped a refused current read");
                    assert_eq!(licence, "2");
                    assert_eq!(*first, 1);
                    assert_eq!(lines, &expected);
                    assert_eq!(source_sha256, &mush_roll::source_digest(SOURCE).to_hex());
                }
                MindResponse::Error { .. } => assert!(!permitted, "lawful source read refused"),
                _ => return Err("unexpected source response".into()),
            }
        }
        if serde_json::to_string(event)?.contains("this.landed") {
            assert!(permitted);
            assert_eq!(
                *audience,
                Audience::Conn(conn),
                "source is never actor/resume/broadcast output"
            );
        }
    }
    assert_eq!(answers, 1);
    Ok(())
}
async fn exercise_async(f: &PurchaseFixture, saved: &Durable, permitted: bool) -> TestResult {
    eprintln!("M7_STORY phase=paid_source_player_boot event=begin");
    let mut world = PlayerWorld::boot(f, saved)?;
    eprintln!("M7_STORY phase=paid_source_player_boot event=complete");
    let tasks = world.tasks.clone();
    let outcome: TestResult = async {
        let before = f.rows("roll", "roll_mind_paid_term");
        let term: PaidTermRecord = serde_json::from_value(one(before.clone())?)?;
        let events = raw(&mut world, invocation(BUYER, BUYER_CONN, 901, source_request()))?;
        assert_source(&events, BUYER_CONN, permitted)?;
        let line = raw(&mut world, Inbound::Cmd { actor: BUYER, conn: BUYER_CONN, line: "@mind/source filed:2 1-200".into() })?;
        let mut source_lines = 0;
        for (audience, event) in &line {
            if serde_json::to_string(event)?.contains("this.landed") {
                source_lines += 1;
                assert_eq!(*audience, Audience::Conn(BUYER_CONN));
            }
        }
        assert_eq!(source_lines > 0, permitted);
        // Neither GOD nor an alternate principal may use the customer's ID.
        assert_source(&raw(&mut world, invocation(GOD, OBSERVER_CONN, 902, source_request()))?, OBSERVER_CONN, false)?;
        let alt = world.engine.world.create(ObjKind::Player, "Source account alt", GOD, Some(ROOM))?;
        {
            let mut accounts = f.accounts.values.lock().map_err(module_error)?;
            let account = *accounts.get(&BUYER).ok_or("buyer account")?;
            f.accounts.epoch.invalidate();
            accounts.insert(alt, account);
        }
        world.step(Inbound::ConnUp { conn: 403, actor: alt, caps_structured: false, from_linkdead: false })?;
        assert_source(&raw(&mut world, invocation(alt, 403, 903, source_request()))?, 403, false)?;
        // Actual title transfer and the real seventeen-second issuer/customer
        // term gap cannot revoke an already granted customer's source form.
        let licence: PersonalPaidLicence = serde_json::from_value(one(f.rows("roll", "roll_mind_personal_paid_licence"))?)?;
        let shell = licence.charter().object();
        world.say(GOD, &format!("@asset/transfer #{}=#{}", shell.0, BUYER.0))?;
        assert_eq!(world.engine.world.module_state::<EconomyStore>().ok_or("Economy")?.snapshot().asset(shell).ok_or("title")?.title_owner, EconomicOwner::Object(BUYER));
        world.engine.world.set_now(1000 + TERM_SECONDS + GRACE_SECONDS);
        assert!(world.engine.world.now() < term.window().grace_end().get());
        assert_source(&raw(&mut world, invocation(BUYER, BUYER_CONN, 904, source_request()))?, BUYER_CONN, permitted)?;
        // The author's original library remains original-author access. Filed
        // title or paid term checks must not retroactively gate this M6 route.
        let author = raw(&mut world, invocation(GOD, OBSERVER_CONN, 905, MindRequest::Read { name: "Candidate".into(), revision: None }))?;
        assert!(author.iter().any(|(a,e)| matches!((a,e), (Audience::Conn(OBSERVER_CONN), OutputEvent::InvokeResult { result: InvokeResult::Mind(MindResponse::Read { draft, .. }), .. }) if draft.source == SOURCE)));
        // Construct requests while valid, then change actual authority before
        // dispatch. The consumer must resolve the current state, not the time
        // the client built its JSON or last read successfully.
        let delayed = invocation(BUYER, BUYER_CONN, 906, source_request());
        world.engine.world.set_now(term.window().grace_end().get());
        assert_source(&raw(&mut world, delayed)?, BUYER_CONN, false)?;
        world.engine.world.set_now(1017);
        let rebound = invocation(BUYER, BUYER_CONN, 907, source_request());
        world.step(Inbound::ConnDown { conn: BUYER_CONN, actor: BUYER, reason: mush_core::DisconnectKind::Lost })?;
        world.step(Inbound::ConnUp { conn: BUYER_CONN, actor: GOD, caps_structured: false, from_linkdead: false })?;
        assert_eq!(world.engine.world.connection_binding(BUYER_CONN).map(|(actor,_)|actor), Some(GOD));
        let rejected = raw(&mut world, rebound)?;
        assert!(rejected.iter().all(|(_,e)| !serde_json::to_string(e).is_ok_and(|s| s.contains("this.landed"))));
        assert_eq!(f.rows("roll", "roll_mind_paid_term"), before, "reads never extend a term");
        Ok(())
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    outcome?;
    eprintln!("M7_STORY phase=paid_source_live_checks event=complete");
    // A changed durable status without its real cause receipt is corruption.
    // No withdrawal/recall producer exists yet, so full boot must reject both;
    // it cannot be used as a shortcut to manufacture a live status observation.
    for field in ["withdrawn_by", "recalled_by"] {
        let mut changed = saved.clone();
        change_one_family(&mut changed, "roll", "roll_mind_status", |row| {
            *row.get_mut(field).ok_or("status")? = json!(token(3_000_099));
            Ok(())
        })?;
        assert!(
            PlayerWorld::boot(f, &changed).is_err(),
            "a status field alone cannot supply a missing cause receipt"
        );
    }
    eprintln!("M7_STORY phase=paid_source_unbacked_status event=complete");
    Ok(())
}
pub(super) fn exercise(f: &PurchaseFixture, saved: &Durable, permitted: bool) -> TestResult {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(exercise_async(f, saved, permitted))
}
