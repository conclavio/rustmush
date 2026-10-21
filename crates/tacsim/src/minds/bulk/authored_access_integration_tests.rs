//! Real paired fork + current paid parent, reached only after measured Registry
//! and genuine Grant. Uses production workshop line/Invoke routes. The only test
//! command deliberately holds actual observations to exercise stale final checks.
use super::*;
use mush_proto::{MindDraft, MindEdit};

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
    actor: ObjId,
    conn: u64,
    request: MindRequest,
) -> TestResult<MindResponse> {
    let events = raw(
        world,
        Inbound::Invoke {
            actor,
            conn,
            id: 975,
            request: InvokeRequest::Mind(request),
        },
    )?;
    let mut responses = events
        .into_iter()
        .filter_map(|(audience, event)| match event {
            OutputEvent::InvokeResult {
                id: 975,
                result: InvokeResult::Mind(response),
            } => Some((audience, response)),
            _ => None,
        });
    let (audience, response) = responses.next().ok_or("actual workshop response")?;
    assert_eq!(audience, Audience::Conn(conn));
    assert!(responses.next().is_none());
    Ok(response)
}
fn read(world: &mut PlayerWorld) -> TestResult<(MindDraft, MindEditBase)> {
    match invoke(
        world,
        BUYER,
        BUYER_CONN,
        MindRequest::Read {
            name: "child".into(),
            revision: None,
        },
    )? {
        MindResponse::Read { draft, base, .. } => Ok((*draft, base)),
        other => Err(format!("lawful read refused: {other:?}").into()),
    }
}
fn current_base(world: &PlayerWorld) -> TestResult<MindEditBase> {
    let base = roll(world)?.mind(BUYER, "child")?.base();
    Ok(MindEditBase {
        edit: base.edit,
        revision: base.revision,
    })
}
fn refused(response: MindResponse) {
    assert!(
        matches!(response, MindResponse::Error { .. }),
        "{response:?}"
    );
}
fn probe(world: &mut PlayerWorld, verb: &str, accepted: bool) -> TestResult {
    let events = line(world, &format!("@authored-probe {verb}"))?;
    let text = PlayerWorld::private_text(&events, BUYER);
    assert!(
        text.contains(if accepted {
            "probe accepted"
        } else {
            "probe refused"
        }),
        "{text}"
    );
    if !accepted {
        assert!(
            events
                .iter()
                .all(|(_, e)| !mush_testkit::text_of(e).contains("this.landed"))
        );
    }
    Ok(())
}
fn private_source(events: &Events, expected: &str) -> TestResult {
    let expected = crate::minds::workshop::source_lines(expected, 1, 200)?;
    let actual = events
        .iter()
        .filter_map(|(audience, event)| match event {
            OutputEvent::Sys(text) => Some((audience, text.to_plain_text())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), expected.len());
    for ((audience, line), expected) in actual.into_iter().zip(expected) {
        assert_eq!(
            *audience,
            Audience::Conn(BUYER_CONN),
            "no same-character actor/resume output"
        );
        assert_eq!(line, expected);
    }
    Ok(())
}
async fn working(f: &PurchaseFixture, saved: &Durable) -> TestResult<Durable> {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult<Durable> = async {
        let initial = roll(&world)?.mind(BUYER, "child")?.clone();
        let selector = one(rows(&disk(&world), "roll", "roll_mind_draft_parent"))?;
        let terms = rows(&disk(&world), "roll", "roll_mind_paid_term");
        world.step(Inbound::ConnUp {
            conn: 404,
            actor: BUYER,
            caps_structured: false,
            from_linkdead: false,
        })?;
        let (mut draft, base) = read(&mut world)?;
        assert_eq!(draft.source, SOURCE);
        private_source(&line(&mut world, "@mind/source child 1-200")?, SOURCE)?;
        for command in ["@mind/info child", "@mind/card child"] {
            let events = line(&mut world, command)?;
            assert!(!events.is_empty());
            assert!(
                events
                    .iter()
                    .all(|(audience, _)| *audience == Audience::Conn(BUYER_CONN)),
                "{events:?}"
            );
        }
        refused(invoke(
            &mut world,
            GOD,
            OBSERVER_CONN,
            MindRequest::Read {
                name: "child".into(),
                revision: None,
            },
        )?);
        refused(invoke(
            &mut world,
            BUYER,
            BUYER_CONN,
            MindRequest::Read {
                name: "child".into(),
                revision: Some("1".into()),
            },
        )?);
        // A same-account different character is not the selector's actor.
        let alt = world
            .engine
            .world
            .create(ObjKind::Player, "B1 account alt", GOD, Some(ROOM))?;
        {
            let mut accounts = f.accounts.values.lock().map_err(module_error)?;
            let account = *accounts.get(&BUYER).ok_or("buyer account")?;
            f.accounts.epoch.invalidate();
            accounts.insert(alt, account);
        }
        world.step(Inbound::ConnUp {
            conn: 405,
            actor: alt,
            caps_structured: false,
            from_linkdead: false,
        })?;
        refused(invoke(
            &mut world,
            alt,
            405,
            MindRequest::Read {
                name: "child".into(),
                revision: None,
            },
        )?);
        // The source operation cannot be spent as an edit, even while current.
        probe(&mut world, "capture-source", true)?;
        probe(&mut world, "publish", false)?;
        assert_eq!(roll(&world)?.mind(BUYER, "child")?, &initial);
        probe(&mut world, "capture-edit", true)?;
        let changed = line(&mut world, "@mind/insert child 1=// first working edit")?;
        assert!(PlayerWorld::private_text(&changed, BUYER).contains("Working source updated"));
        assert!(
            roll(&world)?
                .mind(BUYER, "child")?
                .draft()
                .source
                .starts_with("// first working edit")
        );
        let after_line = roll(&world)?.mind(BUYER, "child")?.clone();
        probe(&mut world, "publish", false)?; // actual captured EditBase became stale
        refused(invoke(
            &mut world,
            BUYER,
            BUYER_CONN,
            MindRequest::Replace {
                name: "child".into(),
                base,
                draft: Box::new(draft.clone()),
            },
        )?);
        assert_eq!(roll(&world)?.mind(BUYER, "child")?, &after_line);
        draft
            .source
            .push_str("\n// replacement remains a working draft\n");
        let base = current_base(&world)?;
        assert!(matches!(
            invoke(
                &mut world,
                BUYER,
                BUYER_CONN,
                MindRequest::Replace {
                    name: "child".into(),
                    base,
                    draft: Box::new(draft.clone())
                }
            )?,
            MindResponse::Edited { .. }
        ));
        let base = current_base(&world)?;
        assert!(matches!(
            invoke(
                &mut world,
                BUYER,
                BUYER_CONN,
                MindRequest::Edit {
                    name: "child".into(),
                    base,
                    edit: MindEdit::ProfileAdd {
                        name: "Working".into(),
                        from: Some("Default".into())
                    }
                }
            )?,
            MindResponse::Edited { .. }
        ));
        let (readback, _) = read(&mut world)?;
        assert_eq!(readback.source, draft.source);
        assert!(
            readback
                .profiles
                .as_object()
                .ok_or("profiles")?
                .contains_key("Working")
        );
        private_source(
            &line(&mut world, "@mind/source child 1-200")?,
            &draft.source,
        )?;
        // Output computed from the marked draft also remains connection-only.
        let diff = line(&mut world, "@mind/diff child=child")?;
        assert!(
            diff.iter().all(|(a, _)| *a == Audience::Conn(BUYER_CONN)),
            "{diff:?}"
        );
        let edited = roll(&world)?.mind(BUYER, "child")?.clone();
        assert!(edited.require_original_access().is_err());
        assert_eq!(
            serde_json::to_value(&edited)?.get("provenance"),
            serde_json::to_value(&initial)?.get("provenance")
        );
        assert_eq!(
            one(rows(&disk(&world), "roll", "roll_mind_draft_parent"))?,
            selector
        );
        assert_eq!(rows(&disk(&world), "roll", "roll_mind_paid_term"), terms);
        // Copy remains closed; removal of a draft is the authoritative draft
        // retirement (its own stories), queued rather than refused, and not
        // issued here because this child is used below. The separate B2 child
        // exercises actual current-rights Check/Save.
        for command in ["@mind/localcopy child=copy"] {
            let before = disk(&world);
            let events = line(&mut world, command)?;
            let text = PlayerWorld::private_text(&events, BUYER);
            assert!(text.contains("No such owned mind"), "{command}: {text}");
            assert!(!text.contains("queued"));
            assert_eq!(world.engine.world.input_depth(BUYER), 0);
            assert_eq!(disk(&world), before);
            assert_eq!(roll(&world)?.mind(BUYER, "child")?, &edited);
        }
        eprintln!("M7_STORY_DIAG b1 step=after_loop"); // TEMP M7 diag
        // Exact original helpers still deny, even after lawful working edits.
        let store = roll(&world)?;
        let source = world
            .engine
            .world
            .module_state::<crate::store::TacsimStore>()
            .ok_or("Tacsim")?;
        assert!(
            crate::minds::workshop::owned_copy(
                store,
                source,
                BUYER,
                &crate::minds::workshop::MindRef::parse("child")?
            )
            .is_err()
        );
        eprintln!("M7_STORY_DIAG b1 step=after_owned_copy"); // TEMP M7 diag
        // A real disconnect/reconnect changes the physical lease. A held source
        // observation cannot emit into the replacement session or its sibling.
        probe(&mut world, "capture-source", true)?;
        world.step(Inbound::ConnDown {
            conn: BUYER_CONN,
            actor: BUYER,
            reason: mush_core::DisconnectKind::Lost,
        })?;
        world.step(Inbound::ConnUp {
            conn: BUYER_CONN,
            actor: BUYER,
            caps_structured: false,
            from_linkdead: false,
        })?;
        probe(&mut world, "emit", false)?;
        assert_eq!(
            read(&mut world)?.0.source,
            draft.source,
            "fresh request can use the new session"
        );
        eprintln!("M7_STORY_DIAG b1 step=before_cold"); // TEMP M7 diag
        let edited_disk = disk(&world);
        let retained = capture(&world, &edited_disk);
        let cold = PlayerWorld::boot_forks(f, &retained, true)?;
        assert_eq!(roll(&cold)?.mind(BUYER, "child")?, &edited);
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        eprintln!("M7_STORY_DIAG b1 step=after_cold"); // TEMP M7 diag
        // Same timestamp, changed actual clock generation invalidates a held view.
        probe(&mut world, "capture-source", true)?;
        let before = world.engine.world.world_ic_now();
        let epoch = world.engine.world.next_world_timeline_epoch(2)?;
        eprintln!("M7_STORY_DIAG b1 step=epoch_next"); // TEMP M7 diag
        world.engine.world.append_world_timeline_epoch(epoch)?;
        eprintln!("M7_STORY_DIAG b1 step=epoch_appended"); // TEMP M7 diag
        assert_eq!(world.engine.world.world_ic_now().at, before.at);
        assert_ne!(world.engine.world.world_ic_now().clock, before.clock);
        probe(&mut world, "check", false)?;
        eprintln!("M7_STORY_DIAG b1 step=checked"); // TEMP M7 diag
        Ok(retained)
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}

pub(super) async fn exercise(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let edited = story_phase("access.working", || working(f, saved))?.await?;
    eprintln!("M7_STORY_DIAG exercise boot_forks begin"); // TEMP M7 diag
    let mut world = PlayerWorld::boot_forks(f, &edited, true)?;
    eprintln!("M7_STORY_DIAG exercise boot_forks done"); // TEMP M7 diag
    let tasks = world.tasks.clone();
    let result:TestResult=async {
        let term:PaidTermRecord=serde_json::from_value(one(rows(&disk(&world),"roll","roll_mind_paid_term"))?)?;
        let before=roll(&world)?.mind(BUYER,"child")?.clone();
        let licence:PersonalPaidLicence=serde_json::from_value(one(rows(&disk(&world),"roll","roll_mind_personal_paid_licence"))?)?;
        let shell=licence.charter().object();
        world.transfer_charter(shell, BUYER, 49_000_010)?;
        assert_eq!(world.engine.world.module_state::<EconomyStore>().ok_or("Economy")?.snapshot().asset(shell).ok_or("title")?.title_owner,EconomicOwner::Object(BUYER));
        world.engine.world.set_now(1000+TERM_SECONDS+GRACE_SECONDS);
        assert!(world.engine.world.now()<term.window().grace_end().get());
        assert_eq!(read(&mut world)?.0.source,before.draft().source,"issuer title/term loss is not loss of the already-paid parent grant");
        let edit=line(&mut world,"@mind/insert child 1=// issuer-independent edit")?;
        assert!(PlayerWorld::private_text(&edit,BUYER).contains("Working source updated"));
        let before=roll(&world)?.mind(BUYER,"child")?.clone();
        probe(&mut world,"capture-source",true)?;
        world.engine.world.set_now(term.window().grace_end().get());
        probe(&mut world,"emit",false)?;
        refused(invoke(&mut world,BUYER,BUYER_CONN,MindRequest::Read { name:"child".into(),revision:None })?);
        let refused_edit=line(&mut world,"@mind/insert child 1=// not admitted")?;
        assert!(PlayerWorld::private_text(&refused_edit,BUYER).contains("No such owned mind"));
        assert_eq!(roll(&world)?.mind(BUYER,"child")?,&before);
        // Original M6 authors keep their own library source after filed-title loss.
        assert!(matches!(invoke(&mut world,GOD,OBSERVER_CONN,MindRequest::Read { name:"Candidate".into(),revision:None })?,MindResponse::Read { draft,.. } if draft.source==SOURCE));
        let retained=capture(&world,&disk(&world));let cold=PlayerWorld::boot_forks(f,&retained,true)?;
        assert_eq!(roll(&cold)?.mind(BUYER,"child")?,&before,"valid lapse is loadable, not a reason to remove provenance");
        cold.tasks.shutdown(Duration::from_secs(5)).await;Ok(())
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result?;
    // A withdrawal or recall is a retained cause record (`withdrawal::audit`,
    // `registry_recall::audit`), so a status naming one that does not exist is
    // forged history and refuses at boot. The real withdrawal (paid Pins keep
    // reading) and recall (refused) flows are their own stories.
    for field in ["withdrawn_by", "recalled_by"] {
        let mut changed = edited.clone();
        change_one_family(&mut changed, "roll", "roll_mind_status", |row| {
            *row.get_mut(field).ok_or("status")? = json!(token(4_000_200));
            Ok(())
        })?;
        assert!(
            PlayerWorld::boot_forks(f, &changed, true).is_err(),
            "a forged {field} cause loaded"
        );
    }
    Ok(())
}
