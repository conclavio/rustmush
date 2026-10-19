//! Genuine B2 Save supplies immutable revisions after actual Registry/paid/fork.
//! The test command retains real observations only to exercise final-use refusal.
use super::*;

fn no_source(events: &Events) {
    assert!(
        events.iter().all(|(_, event)| {
            let text = mush_testkit::text_of(event);
            !text.contains("this.landed")
                && !text.contains("HISTORICAL_SOURCE_CANARY")
                && !matches!(
                    event,
                    OutputEvent::InvokeResult {
                        result: InvokeResult::Mind(MindResponse::Read { .. }),
                        ..
                    }
                )
        }),
        "refusal must contain neither source nor derived output"
    );
}
fn read_saved(world: &mut PlayerWorld, selector: &str) -> TestResult<mush_proto::MindDraft> {
    match invoke(
        world,
        BUYER_CONN,
        REQUEST + 20,
        MindRequest::Read {
            name: "child".into(),
            revision: Some(selector.into()),
        },
    )? {
        MindResponse::Read { draft, .. } => Ok(*draft),
        _ => Err("lawful exact historical read refused".into()),
    }
}
fn private_lines(events: &Events, expected: &[String]) {
    let actual = events
        .iter()
        .filter_map(|(audience, event)| match event {
            OutputEvent::Sys(text) => Some((audience, text.to_plain_text())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), expected.len(), "exact bounded output count");
    for ((audience, actual), expected) in actual.into_iter().zip(expected) {
        assert!(
            *audience == Audience::Conn(BUYER_CONN),
            "history output is physical-connection only"
        );
        assert!(
            actual == *expected,
            "exact historical output, payload omitted"
        );
    }
}
fn probe(world: &mut PlayerWorld, conn: u64, verb: &str, accepted: bool) -> TestResult {
    let events = raw(
        world,
        Inbound::Cmd {
            actor: BUYER,
            conn,
            line: format!("@authored-probe {verb}"),
        },
    )?;
    assert!(
        events.iter().any(
            |(_, event)| mush_testkit::text_of(event).contains(if accepted {
                "probe accepted"
            } else {
                "probe refused"
            })
        ),
        "actual retained-observation check"
    );
    if !accepted {
        no_source(&events);
    }
    Ok(())
}
async fn success(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result:TestResult=async {
        let before=disk(&world);let initial=roll(&world)?.mind(BUYER,"child")?.clone();
        let normalized=mush_roll::RevisionAdapter::normalize_source(&crate::minds::authoring::RevisionRules,SOURCE)?;
        let exact=roll(&world)?.revision(BUYER,mush_roll::RevisionKey {character:BUYER,mind:initial.id(),number:1})?.reference();
        for selector in ["1".to_owned(),format!("sha:{}",exact.digest.to_hex())] {
            assert!(read_saved(&mut world,&selector)?.source==normalized,"exact stored source, not draft");
            let events=line(&mut world,&format!("@mind/source child@{selector} 1-200"))?;
            private_lines(&events,&crate::minds::workshop::source_lines(&normalized,1,200)?);
        }
        assert!(disk(&world)==before,"read-only public history did not publish");
        // A real versioned Save creates different immutable source under a named selector.
        let edit=line(&mut world,"@mind/insert child 1=// HISTORICAL_SOURCE_CANARY")?;
        no_source(&edit);
        let source=mush_roll::RevisionAdapter::normalize_source(&crate::minds::authoring::RevisionRules, &roll(&world)?.mind(BUYER,"child")?.draft().source)?;
        let base=roll(&world)?.mind(BUYER,"child")?.base();
        let job=match invoke(&mut world,BUYER_CONN,REQUEST,MindRequest::Save {name:"child".into(),base:MindEditBase {edit:base.edit,revision:base.revision},notes:"historical access fixture".into(),version:Some("history".into())})? {
            MindResponse::Queued {job}=>job,_=>return Err("actual named save not queued".into())
        };
        ready(&world,job).await?;let completed=drain(&mut world,job).await?;
        assert!(matches!(terminal(&completed)?,MindResponse::Saved {..}),"actual named saved revision");
        assert!(read_saved(&mut world,"history")?.source==source);
        let history=roll(&world)?.mind(BUYER,"child")?.clone();
        let saved_copy=history.draft().clone();
        no_source(&line(&mut world,"@mind/insert child 1=// current draft differs")?);
        let current=roll(&world)?.mind(BUYER,"child")?.draft().clone();
        assert!(current.source!=saved_copy.source);
        assert!(read_saved(&mut world,"history")?.source==saved_copy.source);
        for (command,left,right) in [
            ("@mind/diff child@history=child",&saved_copy,&current),
            ("@mind/diff child=child@history",&current,&saved_copy),
        ] {
            private_lines(&line(&mut world,command)?,&crate::minds::workshop::diff(left,right)?);
        }
        // Mixed original/marked operands must not fall back to actor broadcast.
        line(&mut world,"@mind/new Independent")?;
        let original=roll(&world)?.mind(BUYER,"Independent")?.draft().clone();
        for (command,left,right) in [
            ("@mind/diff Independent=child@history",&original,&saved_copy),
            ("@mind/diff child@history=Independent",&saved_copy,&original),
        ] {private_lines(&line(&mut world,command)?,&crate::minds::workshop::diff(left,right)?);}
        probe(&mut world,BUYER_CONN,"history-wrong-digest",false)?;
        probe(&mut world,BUYER_CONN,"history-foreign",false)?;
        for selector in ["999999999999","sha:00000000","missing"] {
            let events=line(&mut world,&format!("@mind/source child@{selector}"))?;no_source(&events);
            assert!(events.iter().any(|(_,event)|mush_testkit::text_of(event).contains("No such owned mind")));
        }
        let foreign=raw(&mut world,Inbound::Invoke {actor:GOD,conn:OBSERVER_CONN,id:REQUEST+21,
            request:InvokeRequest::Mind(MindRequest::Read {name:"child".into(),revision:Some("1".into())})})?;
        no_source(&foreign);
        assert!(foreign.iter().any(|(audience,event)|*audience==Audience::Conn(OBSERVER_CONN)
            && matches!(event,OutputEvent::InvokeResult {result:InvokeResult::Mind(MindResponse::Error {..}),..})));
        // Issuer title and original-owner term loss leave this own paid grant valid.
        let paid=rows(&disk(&world),"roll","roll_mind_paid_term");
        let term:PaidTermRecord=serde_json::from_value(one(paid.clone())?)?;
        let licence:PersonalPaidLicence=serde_json::from_value(one(rows(&disk(&world),"roll","roll_mind_personal_paid_licence"))?)?;
        world.transfer_charter(licence.charter().object(), BUYER, 49_000_030)?;
        assert!(world.engine.world.module_state::<EconomyStore>().ok_or("economy")?.snapshot()
            .asset(licence.charter().object()).ok_or("title")?.title_owner==EconomicOwner::Object(BUYER));
        world.engine.world.set_now(1000+TERM_SECONDS+GRACE_SECONDS);
        assert!(world.engine.world.now()<term.window().grace_end().get());
        assert!(read_saved(&mut world,"history")?.source==source);
        let retained=capture(&world,&disk(&world));
        let mut cold=PlayerWorld::boot_forks(f,&retained,true)?;let cold_tasks=cold.tasks.clone();
        let read=read_saved(&mut cold,"history");cold_tasks.shutdown(Duration::from_secs(5)).await;
        assert!(read?.source==source,"cold source/selector audit preserves exact history");
        assert!(rows(&disk(&world),"roll","roll_mind_paid_term")==paid,"historical reads never renew grants");
        probe(&mut world,BUYER_CONN,"capture-history",true)?;
        world.engine.world.set_now(term.window().grace_end().get());
        probe(&mut world,BUYER_CONN,"emit",false)?;
        let before=disk(&world);
        for command in ["@mind/source child@1","@mind/diff child@1=Independent","@mind/diff Independent=child@1"] {
            let events=line(&mut world,command)?;no_source(&events);
            assert!(events.iter().any(|(_,event)|mush_testkit::text_of(event).contains("No such owned mind")));
        }
        assert!(matches!(invoke(&mut world,BUYER_CONN,REQUEST+20,MindRequest::Read {name:"child".into(),revision:Some("1".into())})?,MindResponse::Error {..}));
        assert!(disk(&world)==before,"refused history retains all saved and business rows");
        // Original M6 saved source remains independent of this paid-source route.
        let events=raw(&mut world,Inbound::Invoke {actor:GOD,conn:OBSERVER_CONN,id:REQUEST+22,
            request:InvokeRequest::Mind(MindRequest::Read {name:"Candidate".into(),revision:Some("1".into())})})?;
        assert!(events.iter().any(|(audience,event)|*audience==Audience::Conn(OBSERVER_CONN)
            && matches!(event,OutputEvent::InvokeResult {result:InvokeResult::Mind(MindResponse::Read {draft,..}),..} if draft.source==normalized)));
        Ok(())
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
async fn held(f: &PurchaseFixture, saved: &Durable, mode: &str) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        probe(&mut world, BUYER_CONN, "capture-history", true)?;
        let mut emit_conn = BUYER_CONN;
        match mode {
            "edit" => {
                no_source(&line(
                    &mut world,
                    "@mind/insert child 1=// later working edit",
                )?);
            }
            "clock" => {
                let stamp = world.engine.world.world_ic_now();
                let epoch = world.engine.world.next_world_timeline_epoch(2)?;
                world.engine.world.append_world_timeline_epoch(epoch)?;
                assert!(
                    stamp.at == world.engine.world.world_ic_now().at
                        && stamp.clock != world.engine.world.world_ic_now().clock
                );
            }
            "rollback" => {
                world.engine.world.set_now(world.engine.world.now() - 1);
            }
            "sibling" => {
                raw(
                    &mut world,
                    Inbound::ConnUp {
                        conn: 404,
                        actor: BUYER,
                        caps_structured: false,
                        from_linkdead: false,
                    },
                )?;
                emit_conn = 404;
            }
            "reconnect" => {
                raw(
                    &mut world,
                    Inbound::ConnDown {
                        conn: BUYER_CONN,
                        actor: BUYER,
                        reason: mush_core::DisconnectKind::Lost,
                    },
                )?;
                raw(
                    &mut world,
                    Inbound::ConnUp {
                        conn: BUYER_CONN,
                        actor: BUYER,
                        caps_structured: false,
                        from_linkdead: false,
                    },
                )?;
            }
            _ => return Err("unknown historical fence".into()),
        }
        let before = disk(&world);
        probe(&mut world, emit_conn, "emit", false)?;
        assert!(disk(&world) == before, "held history refusal is inert");
        if mode == "edit" || mode == "reconnect" {
            assert!(
                read_saved(&mut world, "1")?.source.contains("this.landed"),
                "fresh request resolves new physical/CAS state"
            );
        }
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
pub(super) async fn exercise(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    story_phase("history.success", || success(f, saved))?.await?;
    for mode in ["edit", "clock", "rollback", "sibling", "reconnect"] {
        story_phase("history.held", || held(f, saved, mode))?.await?;
    }
    // Explicitly disabling fork support refuses retained marked records at the
    // real final owner audit; it cannot silently load them as original source.
    assert!(
        PlayerWorld::boot(f, saved).is_err(),
        "default composition cannot reinterpret marked records"
    );
    // A raw status bit without its actual acknowledged cause is corruption,
    // never a substitute for a real current-status transition fixture.
    for field in ["withdrawn_by", "recalled_by"] {
        let mut changed = saved.clone();
        change_one_family(&mut changed, "roll", "roll_mind_status", |row| {
            *row.get_mut(field).ok_or("status")? = json!(token(4_000_200));
            Ok(())
        })?;
        assert!(
            PlayerWorld::boot_forks(f, &changed, true).is_err(),
            "uncorroborated status must fail the actual cold audit"
        );
    }
    Ok(())
}
