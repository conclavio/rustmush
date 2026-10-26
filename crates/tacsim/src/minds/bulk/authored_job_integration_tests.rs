//! Runs only after the real Registry, paid Grant and registered authored fork.
//! No fabricated grant, worker result or direct parent-read context is used.
use super::*;
#[path = "authored_history_integration_tests.rs"]
mod history_tests;
#[path = "marked_registry_integration_tests.rs"]
mod registry_tests;
#[path = "authored_retirement_integration_tests.rs"]
mod retirement_tests;
use crate::minds::workshop::authored_jobs::probe as jobs;

const REQUEST: u64 = 9_801;
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
fn line(world: &mut PlayerWorld, text: &str) -> TestResult<Events> {
    raw(
        world,
        Inbound::Cmd {
            actor: BUYER,
            conn: BUYER_CONN,
            line: text.into(),
        },
    )
}
fn invoke(
    world: &mut PlayerWorld,
    conn: u64,
    id: u64,
    request: MindRequest,
) -> TestResult<MindResponse> {
    let events = raw(
        world,
        Inbound::Invoke {
            actor: BUYER,
            conn,
            id,
            request: InvokeRequest::Mind(request),
        },
    )?;
    let mut found = events
        .into_iter()
        .filter_map(|(audience, event)| match event {
            OutputEvent::InvokeResult {
                id: got,
                result: InvokeResult::Mind(response),
            } if got == id => Some((audience, response)),
            _ => None,
        });
    let (audience, response) = found.next().ok_or("actual ingress response")?;
    check_eq!(audience, Audience::Conn(conn));
    check!(found.next().is_none());
    Ok(response)
}
fn request(world: &PlayerWorld, save: bool) -> TestResult<MindRequest> {
    let base = roll(world)?.mind(BUYER, "child")?.base();
    let base = MindEditBase {
        edit: base.edit,
        revision: base.revision,
    };
    Ok(if save {
        MindRequest::Save {
            name: "child".into(),
            base,
            notes: "actual worker save".into(),
            version: None,
        }
    } else {
        MindRequest::Check {
            name: "child".into(),
            base,
        }
    })
}
fn submit(world: &mut PlayerWorld, save: bool, web: bool) -> TestResult<u64> {
    if web {
        let request = request(world, save)?;
        return match invoke(world, BUYER_CONN, REQUEST, request)? {
            MindResponse::Queued { job } => Ok(job),
            other => Err(format!("not queued: {other:?}").into()),
        };
    }
    let events = line(
        world,
        if save {
            "@mind/save child=actual line worker save"
        } else {
            "@mind/check child"
        },
    )?;
    let mut jobs = events.iter().filter_map(|(audience, event)| {
        let text = mush_testkit::text_of(event);
        text.strip_prefix("Authoring job ")
            .and_then(|s| s.strip_suffix(" queued."))
            .map(|s| (audience, s.parse::<u64>()))
    });
    let (audience, number) = jobs.next().ok_or("actual line queued response")?;
    check_eq!(*audience, Audience::Conn(BUYER_CONN));
    check!(jobs.next().is_none());
    Ok(number?)
}
async fn ready(world: &PlayerWorld, job: u64) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !jobs::ready(&world.engine.world, job) {
        if Instant::now() >= deadline {
            return Err("real author worker did not finish".into());
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    Ok(())
}
fn fire(world: &mut PlayerWorld, when: i64) -> TestResult<Events> {
    let mut out = Outbox::new();
    // The driver's wall scheduling cannot lag the fixture's deliberate IC
    // advance by millions of one-second ticks. World::now is the real floor.
    let when = when.max(world.engine.world.now().saturating_mul(1000));
    world
        .engine
        .fire_due(when, &mut out, &mut mush_core::engine::Effects::new());
    let events = out.take();
    world.commit()?;
    Ok(events)
}
fn poll_once(world: &mut PlayerWorld) -> TestResult<Events> {
    let when = world
        .engine
        .sched
        .pending_entries()
        .entries()
        .iter()
        .find_map(|(when, kind)| {
            matches!(kind,mush_core::ScheduledKind::ModuleTick {module,tag}
            if module.as_str()==crate::NS && tag.as_str()=="mind-author-results")
            .then_some(*when)
        })
        .ok_or("actual registered author result tick")?;
    fire(world, when)
}
fn private(events: &Events) {
    for (audience, event) in events {
        if matches!(event, OutputEvent::InvokeResult { id: REQUEST, .. })
            || mush_testkit::text_of(event).contains("ops ")
            || mush_testkit::text_of(event).contains("Saved revision")
            || mush_testkit::text_of(event).contains("Authoring completion")
        {
            assert_eq!(*audience, Audience::Conn(BUYER_CONN), "{event:?}");
        }
    }
}
async fn drain(world: &mut PlayerWorld, job: u64) -> TestResult<Events> {
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while jobs::retained(&world.engine.world, job) {
        if Instant::now() >= deadline {
            return Err("automatic connection completion timed out".into());
        }
        let when = world
            .engine
            .sched
            .next_due()
            .ok_or("registered completion scheduling")?;
        events.extend(fire(world, when)?);
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    private(&events);
    Ok(events)
}
fn terminal(events: &Events) -> TestResult<MindResponse> {
    let mut found = events.iter().filter_map(|(_, event)| match event {
        OutputEvent::InvokeResult {
            id: REQUEST,
            result: InvokeResult::Mind(response),
        } => Some(response.clone()),
        _ => None,
    });
    let response = found
        .next()
        .ok_or("automatic original-request terminal response")?;
    check!(found.next().is_none());
    Ok(response)
}
fn no_private_result(events: &Events) {
    for (_, event) in events {
        assert!(!mush_testkit::text_of(event).contains("ops "));
        assert!(!mush_testkit::text_of(event).contains("Saved revision"));
        assert!(!serde_json::to_string(event).is_ok_and(|s| s.contains("this.landed")));
        assert!(!matches!(
            event,
            OutputEvent::InvokeResult {
                result: InvokeResult::Mind(
                    MindResponse::Checked { .. } | MindResponse::Saved { .. }
                ),
                ..
            }
        ));
    }
}
async fn success(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        raw(
            &mut world,
            Inbound::ConnUp {
                conn: 404,
                actor: BUYER,
                caps_structured: false,
                from_linkdead: false,
            },
        )?;
        let selector = one(rows(&disk(&world), "roll", "roll_mind_draft_parent"))?;
        let initial = roll(&world)?.mind(BUYER, "child")?.clone();
        for (save, web) in [(false, true), (true, true), (false, false), (true, false)] {
            let before = roll(&world)?.mind(BUYER, "child")?.clone();
            let job = submit(&mut world, save, web)?;
            ready(&world, job).await?;
            assert_eq!(
                roll(&world)?.mind(BUYER, "child")?,
                &before,
                "worker cannot publish"
            );
            // An idle ready result still occupies the real account/global slot.
            let duplicate = request(&world, save)?;
            assert!(matches!(
                invoke(&mut world, 404, REQUEST + 1, duplicate)?,
                MindResponse::Error { .. }
            ));
            let manual = line(&mut world, &format!("@mind-author-complete {job}"))?;
            no_private_result(&manual);
            assert!(jobs::ready(&world.engine.world, job));
            // Ordinary elapsed IC in the same own term does not invalidate work.
            world.engine.world.set_now(world.engine.world.now() + 1);
            let queued = poll_once(&mut world)?;
            no_private_result(&queued);
            assert!(
                jobs::ready(&world.engine.world, job),
                "queued completion did not take receipt"
            );
            let duplicate = request(&world, save)?;
            assert!(matches!(
                invoke(&mut world, 404, REQUEST + 1, duplicate)?,
                MindResponse::Error { .. }
            ));
            if web {
                assert!(matches!(
                    invoke(&mut world, 404, REQUEST + 2, MindRequest::Status { job })?,
                    MindResponse::Error { .. }
                ));
            }
            let events = drain(&mut world, job).await?;
            if web {
                match terminal(&events)? {
                    MindResponse::Checked {
                        reports,
                        diagnostics,
                        ..
                    } if !save => {
                        assert!(diagnostics.is_empty());
                        assert_eq!(reports.len(), 6);
                        for picture in ["empty", "hostile_450", "hurt_two"] {
                            for armed in [false, true] {
                                assert_eq!(
                                    reports
                                        .iter()
                                        .filter(|r| r.picture == picture && r.armed == armed)
                                        .count(),
                                    1
                                );
                            }
                        }
                        assert!(reports.iter().all(|r| r.operations > 0));
                    }
                    MindResponse::Saved { revision, .. } if save => {
                        assert_eq!(
                            roll(&world)?
                                .mind(BUYER, "child")?
                                .latest()
                                .map(|key| key.number),
                            Some(revision)
                        );
                    }
                    other => return Err(format!("unexpected real completion: {other:?}").into()),
                }
            } else {
                let text = PlayerWorld::private_text(&events, BUYER);
                if save {
                    assert!(text.contains("Saved revision"), "{text}");
                } else {
                    assert_eq!(
                        events
                            .iter()
                            .filter(|(_, e)| mush_testkit::text_of(e).contains(": ops "))
                            .count(),
                        6
                    );
                }
            }
            assert!(!jobs::ready(&world.engine.world, job));
            let replay = line(&mut world, &format!("@mind-author-complete {job}"))?;
            no_private_result(&replay);
            if save {
                assert!(roll(&world)?.mind(BUYER, "child")?.base().edit > before.base().edit);
            } else {
                assert_eq!(roll(&world)?.mind(BUYER, "child")?, &before);
            }
        }
        let child = roll(&world)?.mind(BUYER, "child")?.clone();
        assert_eq!(
            serde_json::to_value(&child)?.get("provenance"),
            serde_json::to_value(&initial)?.get("provenance")
        );
        assert!(child.require_original_access().is_err());
        assert_eq!(
            one(rows(&disk(&world), "roll", "roll_mind_draft_parent"))?,
            selector
        );
        let retained = capture(&world, &disk(&world));
        let cold = PlayerWorld::boot_forks(f, &retained, true)?;
        assert_eq!(roll(&cold)?.mind(BUYER, "child")?, &child);
        let revision = roll(&cold)?.revision(BUYER, child.latest().ok_or("saved revision")?)?;
        let normalized = mush_roll::RevisionAdapter::normalize_source(
            &crate::minds::authoring::RevisionRules,
            SOURCE,
        )?;
        assert_eq!(revision.source(), mush_roll::source_digest(&normalized));
        assert_eq!(
            cold.engine
                .world
                .module_state::<crate::store::TacsimStore>()
                .ok_or("sources")?
                .mind_source(revision.source().as_bytes()),
            Some(normalized.as_str())
        );
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        eprintln!("M7_STORY phase=marked_history event=begin");
        story_phase("jobs.history", || history_tests::exercise(f, &retained))?.await?;
        eprintln!("M7_STORY phase=marked_history event=complete");
        eprintln!("M7_STORY phase=authored_retirement event=begin");
        story_phase("jobs.retirement", || {
            retirement_tests::exercise(f, &retained)
        })?
        .await?;
        eprintln!("M7_STORY phase=authored_retirement event=complete");
        eprintln!("M7_STORY phase=marked_registry event=begin");
        story_phase("jobs.registry", || registry_tests::exercise(f, &retained))?.await?;
        eprintln!("M7_STORY phase=marked_registry event=complete");
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
async fn refusal(f: &PurchaseFixture, saved: &Durable, mode: &str) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result:TestResult=async {
        let job=submit(&mut world,true,true)?;ready(&world,job).await?;
        if mode=="queued_expiry" {
            no_private_result(&poll_once(&mut world)?);
            assert!(jobs::ready(&world.engine.world,job),"queued delivery retains the actual result slot");
            assert_eq!(world.engine.sched.pending_entries().entries().iter()
                .filter(|(_,kind)|matches!(kind,mush_core::ScheduledKind::ConnectionCompletion(_))).count(),1);
        }
        match mode {
            "edit"=>{let events=line(&mut world,"@mind/insert child 1=// newer authorized edit")?;
                assert!(PlayerWorld::private_text(&events,BUYER).contains("Working source updated"));}
            "expiry"|"queued_expiry"=>{let term:PaidTermRecord=serde_json::from_value(one(rows(&disk(&world),"roll","roll_mind_paid_term"))?)?;
                world.engine.world.set_now(term.window().grace_end().get());}
            "clock"=>{let stamp=world.engine.world.world_ic_now();let epoch=world.engine.world.next_world_timeline_epoch(2)?;
                world.engine.world.append_world_timeline_epoch(epoch)?;
                assert_eq!(stamp.at,world.engine.world.world_ic_now().at);assert_ne!(stamp.clock,world.engine.world.world_ic_now().clock);}
            "reconnect"=>{raw(&mut world,Inbound::ConnUp {conn:404,actor:BUYER,caps_structured:false,from_linkdead:false})?;
                raw(&mut world,Inbound::ConnDown {conn:BUYER_CONN,actor:BUYER,reason:mush_core::DisconnectKind::Lost})?;
                raw(&mut world,Inbound::ConnUp {conn:BUYER_CONN,actor:BUYER,caps_structured:false,from_linkdead:false})?;}
            "cancel"=>{assert!(matches!(invoke(&mut world,BUYER_CONN,REQUEST,MindRequest::Cancel {job})?,MindResponse::Cancelled {..}));}
            _=>return Err("unknown refusal fixture".into()),
        }
        let before=roll(&world)?.mind(BUYER,"child")?.clone();
        let retained=disk(&world);
        let events=drain(&mut world,job).await?;
        no_private_result(&events);
        assert_eq!(roll(&world)?.mind(BUYER,"child")?,&before);
        // Revisions and sources live in Roll's and Tacsim's rows; unrelated
        // world services (an atlas connection's schedule) may advance meanwhile.
        let minds=|d:&Disk|->Disk{d.iter().filter(|((owner,_),_)|owner=="roll"||owner=="tacsim").map(|(k,v)|(k.clone(),v.clone())).collect()};
        assert_eq!(minds(&disk(&world)),minds(&retained),"refused completion publishes no revision/source");
        if !matches!(mode,"reconnect"|"cancel") {
            assert!(matches!(terminal(&events)?,MindResponse::Error {message,..} if message=="Authoring completion is no longer available."));
        } else {assert!(!events.iter().any(|(_,e)|matches!(e,OutputEvent::InvokeResult {id:REQUEST,..})));}
        Ok(())
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
async fn issuer_independent(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let job = submit(&mut world, true, true)?;
        ready(&world, job).await?;
        let licence: PersonalPaidLicence = serde_json::from_value(one(rows(
            &disk(&world),
            "roll",
            "roll_mind_personal_paid_licence",
        ))?)?;
        let shell = licence.charter().object();
        world.transfer_charter(shell, BUYER, 49_000_040)?;
        assert_eq!(
            world
                .engine
                .world
                .module_state::<EconomyStore>()
                .ok_or("Economy")?
                .snapshot()
                .asset(shell)
                .ok_or("title")?
                .title_owner,
            EconomicOwner::Object(BUYER)
        );
        world
            .engine
            .world
            .set_now(1000 + TERM_SECONDS + GRACE_SECONDS);
        let term: PaidTermRecord =
            serde_json::from_value(one(rows(&disk(&world), "roll", "roll_mind_paid_term"))?)?;
        assert!(world.engine.world.now() < term.window().grace_end().get());
        let events = drain(&mut world, job).await?;
        assert!(
            matches!(terminal(&events)?, MindResponse::Saved { .. }),
            "own paid grant survives issuer title/term loss"
        );
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
async fn diagnostics(f: &PurchaseFixture, saved: &Durable, expire: bool) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let edit = line(
            &mut world,
            "@mind/insert child 1=@@@ invalid authored draft",
        )?;
        assert!(PlayerWorld::private_text(&edit, BUYER).contains("Working source updated"));
        let job = submit(&mut world, false, true)?;
        ready(&world, job).await?;
        let before = roll(&world)?.mind(BUYER, "child")?.clone();
        if expire {
            let term: PaidTermRecord =
                serde_json::from_value(one(rows(&disk(&world), "roll", "roll_mind_paid_term"))?)?;
            world.engine.world.set_now(term.window().grace_end().get());
        }
        let events = drain(&mut world, job).await?;
        match terminal(&events)? {
            MindResponse::Checked {
                reports,
                diagnostics,
                ..
            } if !expire => {
                assert!(reports.is_empty());
                assert!(!diagnostics.is_empty(), "real invalid source was checked");
            }
            MindResponse::Error { message, .. } if expire => {
                assert_eq!(message, "Authoring completion is no longer available.");
                no_private_result(&events);
            }
            other => return Err(format!("unexpected diagnostics boundary: {other:?}").into()),
        }
        assert_eq!(roll(&world)?.mind(BUYER, "child")?, &before);
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
pub(super) async fn exercise(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    story_phase("jobs.success", || success(f, saved))?.await?;
    for mode in [
        "edit",
        "expiry",
        "queued_expiry",
        "clock",
        "reconnect",
        "cancel",
    ] {
        story_phase("jobs.refusal", || refusal(f, saved, mode))?.await?;
    }
    story_phase("jobs.issuer", || issuer_independent(f, saved))?.await?;
    story_phase("jobs.diagnostics", || diagnostics(f, saved, false))?.await?;
    story_phase("jobs.diagnostics_expired", || diagnostics(f, saved, true))?.await
}
