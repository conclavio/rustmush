//! Actual marked paste runs only after the genuine measured certificate,
//! paid Grant and acknowledged fork. No source/grant/currentness is invented.
use super::*;
use mush_proto::MindEdit;

type Events = Vec<(Audience, OutputEvent)>;
const CANARY: &str = "// PASTE_CUSTODY_CANARY";
fn raw(world: &mut PlayerWorld, input: Inbound) -> TestResult<Events> {
    let mut out = Outbox::new();
    world
        .engine
        .step(input, &mut out, &mut mush_core::engine::Effects::new());
    let events = out.take();
    world.commit()?;
    Ok(events)
}
fn feed(world: &mut PlayerWorld, conn: u64, text: &str) -> TestResult<Events> {
    raw(
        world,
        Inbound::Cmd {
            actor: BUYER,
            conn,
            line: text.into(),
        },
    )
}
fn no_content(events: &Events) {
    assert!(
        events.iter().all(|(_, event)| {
            let text = mush_testkit::text_of(event);
            !text.contains("PASTE_CUSTODY_CANARY") && !text.contains("this.landed")
        }),
        "paste output must not reveal source or accumulated replacement"
    );
}
fn start(world: &mut PlayerWorld) -> TestResult {
    no_content(&feed(world, BUYER_CONN, "@mind/paste child")?);
    check_eq!(
        world.engine.world.input_depth(BUYER),
        1,
        "actual paste context"
    );
    no_content(&feed(world, BUYER_CONN, CANARY)?);
    Ok(())
}
fn provenance(mind: &mush_roll::Mind) -> TestResult<Value> {
    serde_json::to_value(mind)?
        .get("provenance")
        .cloned()
        .ok_or_else(|| "actual marked provenance".into())
}
fn assert_same(mind: &mush_roll::Mind, other: &mush_roll::Mind) {
    assert!(
        mind == other,
        "refused paste preserves the complete actual Mind"
    );
}
async fn success(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let initial = roll(&world)?.mind(BUYER, "child")?.clone();
        let before = disk(&world);
        let selector = one(rows(&before, "roll", "roll_mind_draft_parent"))?;
        let paid = rows(&before, "roll", "roll_mind_paid_term");
        let term: PaidTermRecord = serde_json::from_value(one(paid.clone())?)?;
        let licence: PersonalPaidLicence = serde_json::from_value(one(rows(
            &before,
            "roll",
            "roll_mind_personal_paid_licence",
        ))?)?;
        start(&mut world)?;
        // The issuer loses both title and its own original term while this
        // editor is open. The customer's independent paid term still governs.
        world.transfer_charter(licence.charter().object(), BUYER, 49_000_050)?;
        assert!(
            world
                .engine
                .world
                .module_state::<EconomyStore>()
                .ok_or("economy")?
                .snapshot()
                .asset(licence.charter().object())
                .ok_or("actual title")?
                .title_owner
                == EconomicOwner::Object(BUYER)
        );
        world
            .engine
            .world
            .set_now(1000 + TERM_SECONDS + GRACE_SECONDS);
        assert!(world.engine.world.now() < term.window().grace_end().get());
        let lines = SOURCE
            .lines()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>();
        for line in &lines {
            no_content(&feed(&mut world, BUYER_CONN, line)?);
        }
        let done = feed(&mut world, BUYER_CONN, ".")?;
        no_content(&done);
        assert!(
            done.iter()
                .any(|(audience, event)| *audience == Audience::Conn(BUYER_CONN)
                    && mush_testkit::text_of(event).contains("Working source replaced"))
        );
        assert!(
            done.iter()
                .filter(
                    |(_, event)| mush_testkit::text_of(event).contains("Working source replaced")
                )
                .all(|(audience, _)| *audience == Audience::Conn(BUYER_CONN))
        );
        assert_eq!(world.engine.world.input_depth(BUYER), 0);
        let edited = roll(&world)?.mind(BUYER, "child")?.clone();
        let expected = format!(
            "{CANARY}\n{}\n",
            lines
                .iter()
                .map(|s| s.trim())
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(
            edited.draft().source == expected,
            "complete actual replacement"
        );
        assert_eq!(edited.base().edit, initial.base().edit + 1);
        assert!(provenance(&edited)? == provenance(&initial)?);
        assert!(edited.draft().envelope == initial.draft().envelope);
        assert!(edited.require_original_access().is_err());
        assert!(one(rows(&disk(&world), "roll", "roll_mind_draft_parent"))? == selector);
        assert!(rows(&disk(&world), "roll", "roll_mind_paid_term") == paid);
        let cold = PlayerWorld::boot_forks(f, &capture(&world, &disk(&world)), true)?;
        let cold_same = roll(&cold)?.mind(BUYER, "child")? == &edited;
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        assert!(cold_same, "real cold audit preserves marked replacement");
        // Ordinary original M6 paste keeps its old path, including after the
        // author's title loss. This does not grant access to a marked draft.
        world.say(GOD, "@mind/paste Candidate")?;
        assert_eq!(world.engine.world.input_depth(GOD), 1);
        world.say(GOD, "fn think(v,c) { speed(0); }")?;
        world.say(GOD, ".")?;
        let original = roll(&world)?.mind(GOD, "Candidate")?;
        original.require_original_access()?;
        assert!(original.draft().source == "fn think(v,c) { speed(0); }\n");
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
#[derive(Clone, Copy)]
enum Stop {
    OwnExpiryLine,
    OwnExpiryFinal,
    CompetingEdit,
    Clock,
    Rollback,
    Sibling,
    Reconnect,
    Cancel,
    Nul,
    Capacity,
}
async fn refusal(f: &PurchaseFixture, saved: &Durable, why: Stop) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let later = world.engine.world.now().checked_add(10).ok_or("clock")?;
        world.engine.world.set_now(later);
        let initial = roll(&world)?.mind(BUYER, "child")?.clone();
        let selector = one(rows(&disk(&world), "roll", "roll_mind_draft_parent"))?;
        let mut expected = initial.clone();
        start(&mut world)?;
        let events = match why {
            Stop::OwnExpiryLine | Stop::OwnExpiryFinal => {
                let term: PaidTermRecord = serde_json::from_value(one(rows(
                    &disk(&world),
                    "roll",
                    "roll_mind_paid_term",
                ))?)?;
                world.engine.world.set_now(term.window().grace_end().get());
                feed(
                    &mut world,
                    BUYER_CONN,
                    if matches!(why, Stop::OwnExpiryFinal) {
                        "."
                    } else {
                        "// forbidden later input"
                    },
                )?
            }
            Stop::CompetingEdit => {
                let base = initial.base();
                let edited = raw(
                    &mut world,
                    Inbound::Invoke {
                        actor: BUYER,
                        conn: BUYER_CONN,
                        id: 9961,
                        request: InvokeRequest::Mind(MindRequest::Edit {
                            name: "child".into(),
                            base: MindEditBase {
                                edit: base.edit,
                                revision: base.revision,
                            },
                            edit: MindEdit::ProfileAdd {
                                name: "Competing".into(),
                                from: Some("Default".into()),
                            },
                        }),
                    },
                )?;
                assert!(edited.iter().any(|(_, event)| matches!(
                    event,
                    OutputEvent::InvokeResult {
                        id: 9961,
                        result: InvokeResult::Mind(MindResponse::Edited { .. })
                    }
                )));
                expected = roll(&world)?.mind(BUYER, "child")?.clone();
                assert!(expected != initial, "real structured edit changed CAS");
                feed(&mut world, BUYER_CONN, ".")?
            }
            Stop::Clock => {
                let before = world.engine.world.world_ic_now();
                let epoch = world.engine.world.next_world_timeline_epoch(2)?;
                world.engine.world.append_world_timeline_epoch(epoch)?;
                assert!(world.engine.world.world_ic_now().clock != before.clock);
                assert_eq!(world.engine.world.world_ic_now().at, before.at);
                feed(&mut world, BUYER_CONN, ".")?
            }
            Stop::Rollback => {
                world.engine.world.set_now(later - 1);
                feed(&mut world, BUYER_CONN, ".")?
            }
            Stop::Sibling => {
                world.step(Inbound::ConnUp {
                    conn: 404,
                    actor: BUYER,
                    caps_structured: false,
                    from_linkdead: false,
                })?;
                feed(&mut world, 404, ".")?
            }
            Stop::Reconnect => {
                world.step(Inbound::ConnDown {
                    conn: BUYER_CONN,
                    actor: BUYER,
                    reason: mush_core::DisconnectKind::Lost,
                })?;
                assert_eq!(
                    world.engine.world.input_depth(BUYER),
                    0,
                    "real disconnect clears paste"
                );
                world.step(Inbound::ConnUp {
                    conn: BUYER_CONN,
                    actor: BUYER,
                    caps_structured: false,
                    from_linkdead: false,
                })?;
                feed(&mut world, BUYER_CONN, ".")?
            }
            Stop::Cancel => feed(&mut world, BUYER_CONN, "~q")?,
            Stop::Nul => feed(&mut world, BUYER_CONN, "invalid\0line")?,
            Stop::Capacity => {
                let chunk = "x".repeat(1024);
                let mut events = Vec::new();
                for _ in 0..33 {
                    events.extend(feed(&mut world, BUYER_CONN, &chunk)?);
                    if world.engine.world.input_depth(BUYER) == 0 {
                        break;
                    }
                }
                assert!(
                    events
                        .iter()
                        .any(|(_, event)| mush_testkit::text_of(event).contains("maximum 32 KiB")),
                    "actual bound refusal"
                );
                events
            }
        };
        no_content(&events);
        assert_eq!(
            world.engine.world.input_depth(BUYER),
            0,
            "all stopped editors release context"
        );
        assert!(!events.iter().any(|(_, event)| {
            mush_testkit::text_of(event).contains("Working source replaced")
        }));
        assert_same(roll(&world)?.mind(BUYER, "child")?, &expected);
        assert!(provenance(&expected)? == provenance(&initial)?);
        assert!(one(rows(&disk(&world), "roll", "roll_mind_draft_parent"))? == selector);
        if matches!(
            why,
            Stop::Cancel
                | Stop::Nul
                | Stop::Capacity
                | Stop::Sibling
                | Stop::CompetingEdit
                | Stop::Reconnect
        ) {
            // Drop cleanup must not leave a permanently busy editor slot.
            feed(&mut world, BUYER_CONN, "@mind/paste child")?;
            assert_eq!(world.engine.world.input_depth(BUYER), 1);
            feed(&mut world, BUYER_CONN, "~q")?;
        }
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
pub(super) async fn exercise(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    story_phase("paste.success", || success(f, saved))?.await?;
    for why in [
        Stop::OwnExpiryLine,
        Stop::OwnExpiryFinal,
        Stop::CompetingEdit,
        Stop::Clock,
        Stop::Rollback,
        Stop::Sibling,
        Stop::Reconnect,
        Stop::Cancel,
        Stop::Nul,
        Stop::Capacity,
    ] {
        story_phase("paste.refusal", || refusal(f, saved, why))?.await?;
    }
    Ok(())
}
