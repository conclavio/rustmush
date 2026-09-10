//! End-to-end `@mail` suite, driven through the real [`Engine`] with the
//! module registered exactly as `mushd` will register it. Persistence is
//! exercised by applying the module's write-through ops to a fake kv map
//! and rebooting a second engine from it (the same round-trip shape the
//! comsys/store suites use).

use std::collections::{BTreeMap, HashMap};

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::module::{ModuleBootData, ModuleOp, ModuleWrite};
use mush_core::{
    Flags, GOD, LIMBO, LockExpr, LockType, ObjId, ObjKind, Outbox, Registry, World,
    register_core_commands, register_engine_commands,
};
use mush_mail::{
    INBOX, MAX_MAIL_BODY_LINES, MAX_MAIL_RECIPIENTS, MailModule, MailState, MailStatus, SENT, TRASH,
};
use mush_proto::{InputMode, OutputEvent, PromptSpec};
use mush_testkit::assert_actor_saw;
use smol_str::SmolStr;

/// Fixed "now" for every test world: 2023-11-14 22:13:20 UTC → the list
/// date column reads "Nov 14".
const NOW: i64 = 1_700_000_000;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

fn engine_from(world: World) -> Engine {
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    Engine::with_modules(world, registry, vec![Box::new(MailModule)]).unwrap()
}

/// Engine + Alice and Bob (both world-side connected), booted with no
/// persisted mail data.
fn setup() -> (Engine, ObjId, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(NOW);
    let alice = world
        .create(ObjKind::Player, "Alice", GOD, Some(LIMBO))
        .unwrap();
    let bob = world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
    world.set_connected(alice, true);
    world.set_connected(bob, true);
    let mut engine = engine_from(world);
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();
    (engine, alice, bob)
}

fn run(engine: &mut Engine, actor: ObjId, line: &str) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor,
            line: line.into(),
        },
        &mut out,
        &mut fx,
    );
    out
}

fn mail(engine: &Engine) -> &MailState {
    engine
        .world
        .module_state::<MailState>()
        .expect("mail state")
}

fn sys_texts(out: &Outbox) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            _ => None,
        })
        .collect()
}

fn prompts(out: &Outbox) -> Vec<PromptSpec> {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Prompt(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

/// Apply drained module ops to a fake kv store the way mush-store would
/// (mail never uses streams).
fn apply_ops(kv: &mut BTreeMap<SmolStr, serde_json::Value>, ops: Vec<ModuleOp>) {
    for op in ops {
        assert_eq!(op.ns, "mail", "mail writes only its own namespace");
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.insert(key, value);
            }
            ModuleWrite::Del { key } => {
                kv.remove(&key);
            }
            ModuleWrite::StreamAppend { .. } => panic!("mail does not use streams"),
        }
    }
}

// ----------------------------------------------------------------------
// Send / receive / notices
// ----------------------------------------------------------------------

#[test]
fn quick_send_delivers_and_notifies() {
    let (mut engine, alice, bob) = setup();
    let out = run(&mut engine, alice, "@mail/quick Bob=Lunch/Meet at noon?");
    assert_actor_saw(out.events(), alice, "MAIL: Message 1 sent to Bob.");
    // Live MailNotice to the recipient, with their unread count.
    assert!(out.events().iter().any(|(aud, e)| {
        matches!(aud, mush_core::Audience::Actor(a) if *a == bob)
            && matches!(
                e,
                OutputEvent::MailNotice { from, subject, id: 1, unread: 1 }
                    if from.name == "Alice" && subject == "Lunch"
            )
    }));
    let st = mail(&engine);
    // The message document, stored once.
    let m = st.message(1).expect("message stored");
    assert_eq!(m.from, alice);
    assert_eq!(m.to, vec![bob]);
    assert_eq!(m.subject, "Lunch");
    assert_eq!(m.body, "Meet at noon?");
    assert_eq!(m.reply_to, None);
    assert_eq!(m.sent_at, NOW);
    // Bob's inbox entry is unread; Alice's copy sits pre-read in `sent`.
    let e = st.entry(bob, 1).expect("bob entry");
    assert_eq!((e.folder.as_str(), e.status), (INBOX, MailStatus::Unread));
    let s = st.entry(alice, 1).expect("alice sent copy");
    assert_eq!((s.folder.as_str(), s.status), (SENT, MailStatus::Read));
    assert_eq!(st.next_id(), 2);
}

#[test]
fn multi_recipient_fan_out_stores_once() {
    let (mut engine, alice, bob) = setup();
    let carol = engine
        .world
        .create(ObjKind::Player, "Carol", GOD, Some(LIMBO))
        .unwrap();
    let out = run(&mut engine, alice, "@mail/quick Bob, Carol=Hi/all of you");
    assert_actor_saw(out.events(), alice, "sent to Bob, Carol");
    let st = mail(&engine);
    assert_eq!(st.message(1).unwrap().to, vec![bob, carol]);
    assert!(st.entry(bob, 1).is_some());
    assert!(st.entry(carol, 1).is_some());
    assert_eq!(st.next_id(), 2, "one message, one id");
}

#[test]
fn self_send_keeps_the_inbox_entry() {
    let (mut engine, alice, _) = setup();
    run(&mut engine, alice, "@mail/quick Alice=Note/to self");
    let st = mail(&engine);
    let e = st.entry(alice, 1).expect("single entry");
    assert_eq!(e.folder.as_str(), INBOX, "inbox wins over the sent copy");
    assert_eq!(st.folder_ids(alice, SENT), Vec::<u64>::new());
}

#[test]
fn unknown_recipient_aborts_send() {
    let (mut engine, alice, _) = setup();
    let out = run(&mut engine, alice, "@mail/quick Nobody=Hi/there");
    assert_actor_saw(out.events(), alice, "No such player: Nobody");
    assert!(mail(&engine).message(1).is_none());
}

// ----------------------------------------------------------------------
// Compose input context
// ----------------------------------------------------------------------

#[test]
fn compose_multiline_preview_and_send() {
    let (mut engine, alice, bob) = setup();
    let out = run(&mut engine, alice, "@mail/send Bob=Lunch");
    assert_actor_saw(out.events(), alice, "Composing to Bob -- subject: Lunch");
    // The compose prompt is pushed.
    let ps = prompts(&out);
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].text.as_ref().unwrap().to_plain_text(), "Mail>");
    assert_eq!(engine.world.input_depth(alice), 1);
    // Body lines are captured before dispatch — even command-shaped ones.
    run(&mut engine, alice, "Meet at noon?");
    run(&mut engine, alice, "look");
    // ~p previews without sending.
    let out = run(&mut engine, alice, "~p");
    assert_actor_saw(out.events(), alice, "MAIL: To: Bob");
    assert_actor_saw(out.events(), alice, "MAIL: Subject: Lunch");
    assert_actor_saw(out.events(), alice, "Meet at noon?");
    assert_actor_saw(out.events(), alice, "2 line(s)");
    assert!(mail(&engine).message(1).is_none(), "preview does not send");
    // "--" alone sends and pops back to the normal prompt.
    let out = run(&mut engine, alice, "--");
    assert_actor_saw(out.events(), alice, "MAIL: Message 1 sent to Bob.");
    assert!(
        prompts(&out)
            .iter()
            .any(|p| p.mode == InputMode::Normal && p.text.is_none())
    );
    assert_eq!(engine.world.input_depth(alice), 0);
    let st = mail(&engine);
    assert_eq!(st.message(1).unwrap().body, "Meet at noon?\nlook");
    assert!(st.entry(bob, 1).is_some());
}

#[test]
fn compose_tilde_q_aborts_without_sending() {
    let (mut engine, alice, _) = setup();
    run(&mut engine, alice, "@mail/send Bob=Doomed");
    run(&mut engine, alice, "this will be lost");
    let out = run(&mut engine, alice, "~q");
    assert_actor_saw(out.events(), alice, "Aborted.");
    assert_eq!(engine.world.input_depth(alice), 0);
    assert!(mail(&engine).message(1).is_none(), "nothing was sent");
}

#[test]
fn compose_and_recipient_fanout_are_hard_bounded() {
    let (mut engine, alice, _bob) = setup();
    run(&mut engine, alice, "@mail/send Bob=Bounded");
    for _ in 0..MAX_MAIL_BODY_LINES {
        run(&mut engine, alice, "x");
    }
    let out = run(&mut engine, alice, "overflow");
    assert_actor_saw(out.events(), alice, "Message body is limited");
    run(&mut engine, alice, "--");
    assert_eq!(
        mail(&engine).message(1).unwrap().body.lines().count(),
        MAX_MAIL_BODY_LINES
    );

    let mut recipients = Vec::new();
    for n in 0..=MAX_MAIL_RECIPIENTS {
        let id = engine
            .world
            .create(ObjKind::Player, &format!("Recipient{n}"), GOD, Some(LIMBO))
            .unwrap();
        recipients.push(engine.world.get(id).unwrap().name.clone());
    }
    let out = run(
        &mut engine,
        alice,
        &format!("@mail/quick {}=Too many/nope", recipients.join(",")),
    );
    assert_actor_saw(out.events(), alice, "at most 32 recipients");
    assert!(mail(&engine).message(2).is_none());
}

// ----------------------------------------------------------------------
// Read / unread / listing
// ----------------------------------------------------------------------

#[test]
fn read_shows_headers_and_marks_read() {
    let (mut engine, alice, bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=Lunch/Meet at noon?");
    let _ = engine.take_module_ops();
    let out = run(&mut engine, bob, "@mail 1");
    for needle in [
        "MAIL: Message 1 in inbox (id 1)",
        "From:    Alice",
        "To:      Bob",
        "Date:    Nov 14 2023 22:13 UTC",
        "Subject: Lunch",
        "Meet at noon?",
    ] {
        assert_actor_saw(out.events(), bob, needle);
    }
    let e = mail(&engine).entry(bob, 1).unwrap();
    assert_eq!(e.status, MailStatus::Read);
    assert_eq!(e.read_at, Some(NOW));
    // The status flip wrote through.
    let ops = engine.take_module_ops();
    assert!(
        ops.iter().any(|op| matches!(
            &op.op,
            ModuleWrite::JsonSet { key, value }
                if key.as_str() == format!("box:{}:1", bob.0)
                    && value["status"] == serde_json::json!("read")
        )),
        "read status persisted: {ops:?}"
    );
    // Re-reading does not write again.
    run(&mut engine, bob, "@mail 1");
    assert!(engine.take_module_ops().is_empty());
}

#[test]
fn unread_flips_back() {
    let (mut engine, alice, bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=Lunch/x");
    run(&mut engine, bob, "@mail 1");
    let out = run(&mut engine, bob, "@mail/unread 1");
    assert_actor_saw(out.events(), bob, "MAIL: Message 1 marked unread.");
    let e = mail(&engine).entry(bob, 1).unwrap();
    assert_eq!(e.status, MailStatus::Unread);
    assert_eq!(e.read_at, None);
    // Already-unread is refused.
    let out = run(&mut engine, bob, "@mail/unread 1");
    assert_actor_saw(out.events(), bob, "already unread");
}

#[test]
fn listing_shows_classic_columns() {
    let (mut engine, alice, bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=First one/x");
    run(&mut engine, alice, "@mail/quick/urgent Bob=Second one/y");
    run(&mut engine, bob, "@mail 1"); // read the first
    let out = run(&mut engine, bob, "@mail");
    let texts = sys_texts(&out);
    assert!(
        texts.iter().any(|t| t.contains("--- MAIL: inbox ---")),
        "{texts:?}"
    );
    // num / flags / from / subject / date columns.
    assert!(
        texts.iter().any(|t| t.starts_with("  1 - Alice")
            && t.contains("First one")
            && t.contains("Nov 14")),
        "read row: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("  2 ! Alice") && t.contains("Second one")),
        "urgent row: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("2 messages, 1 unread")),
        "{texts:?}"
    );
}

#[test]
fn empty_folder_and_bad_numbers() {
    let (mut engine, _, bob) = setup();
    let out = run(&mut engine, bob, "@mail");
    assert_actor_saw(out.events(), bob, "MAIL: Folder inbox is empty.");
    let out = run(&mut engine, bob, "@mail 3");
    assert_actor_saw(out.events(), bob, "MAIL: No message 3 in folder inbox.");
    let out = run(&mut engine, bob, "@mail nonsense");
    assert_actor_saw(out.events(), bob, "Expected a message number");
}

// ----------------------------------------------------------------------
// Reply / replyall / forward
// ----------------------------------------------------------------------

#[test]
fn reply_builds_the_chain_without_re_stacking() {
    let (mut engine, alice, bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=Lunch/noon?");
    run(&mut engine, bob, "@mail/reply 1");
    run(&mut engine, bob, "works for me");
    run(&mut engine, bob, "--");
    let st = mail(&engine);
    let m2 = st.message(2).expect("reply stored");
    assert_eq!(m2.from, bob);
    assert_eq!(m2.to, vec![alice]);
    assert_eq!(m2.subject, "Re: Lunch");
    assert_eq!(m2.reply_to, Some(1));
    assert_eq!(m2.body, "works for me");
    assert!(st.entry(alice, 2).is_some());
    // Replying to the reply keeps one "Re:" and extends the chain.
    run(&mut engine, alice, "@mail/reply 1"); // alice's inbox message 1 == mail id 2
    run(&mut engine, alice, "great");
    run(&mut engine, alice, "--");
    let m3 = mail(&engine).message(3).expect("second reply");
    assert_eq!(m3.subject, "Re: Lunch");
    assert_eq!(m3.reply_to, Some(2));
    assert_eq!(m3.to, vec![bob]);
}

#[test]
fn replyall_targets_everyone_but_self() {
    let (mut engine, alice, bob) = setup();
    let carol = engine
        .world
        .create(ObjKind::Player, "Carol", GOD, Some(LIMBO))
        .unwrap();
    run(&mut engine, alice, "@mail/quick Bob,Carol=Plans/meet");
    run(&mut engine, bob, "@mail/replyall 1");
    run(&mut engine, bob, "in");
    run(&mut engine, bob, "--");
    let m2 = mail(&engine).message(2).unwrap();
    assert_eq!(m2.to, vec![alice, carol], "sender first, self excluded");
    assert_eq!(m2.reply_to, Some(1));
}

#[test]
fn forward_copies_the_body() {
    let (mut engine, alice, bob) = setup();
    let carol = engine
        .world
        .create(ObjKind::Player, "Carol", GOD, Some(LIMBO))
        .unwrap();
    run(&mut engine, alice, "@mail/quick Bob=Secret/the plans");
    let out = run(&mut engine, bob, "@mail/fwd 1=Carol");
    assert_actor_saw(out.events(), bob, "sent to Carol");
    let st = mail(&engine);
    let m2 = st.message(2).unwrap();
    assert_eq!(m2.subject, "Fwd: Secret");
    assert_eq!(m2.reply_to, None, "a forward starts a fresh chain");
    assert!(m2.body.contains("[Forwarded from Alice]"));
    assert!(m2.body.contains("the plans"));
    assert!(st.entry(carol, 2).is_some());
}

// ----------------------------------------------------------------------
// Folders
// ----------------------------------------------------------------------

#[test]
fn folders_create_file_switch_and_remove() {
    let (mut engine, alice, bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=Keep me/x");
    let out = run(&mut engine, bob, "@mail/newfolder Projects");
    assert_actor_saw(out.events(), bob, "MAIL: Folder projects created.");
    let out = run(&mut engine, bob, "@mail/file 1=projects");
    assert_actor_saw(out.events(), bob, "MAIL: Message 1 filed in projects.");
    // Inbox is now empty; the folder listing shows counts and the current
    // marker; trash stays hidden.
    let out = run(&mut engine, bob, "@mail");
    assert_actor_saw(out.events(), bob, "MAIL: Folder inbox is empty.");
    let out = run(&mut engine, bob, "@mail/folder");
    let texts = sys_texts(&out);
    assert!(
        texts.iter().any(|t| t.contains("* inbox (0 messages")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("projects (1 message,")),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("trash")),
        "trash hidden: {texts:?}"
    );
    // Switch and list from it.
    let out = run(&mut engine, bob, "@mail/folder projects");
    assert_actor_saw(out.events(), bob, "MAIL: Current folder is now projects.");
    let out = run(&mut engine, bob, "@mail");
    assert_actor_saw(out.events(), bob, "Keep me");
    // Removing a non-empty folder is refused; emptied it works, and the
    // current folder falls back to inbox.
    let out = run(&mut engine, bob, "@mail/rmfolder projects");
    assert_actor_saw(out.events(), bob, "MAIL: Folder projects is not empty.");
    run(&mut engine, bob, "@mail/file 1=inbox");
    let out = run(&mut engine, bob, "@mail/rmfolder projects");
    assert_actor_saw(out.events(), bob, "MAIL: Folder projects removed.");
    assert_eq!(mail(&engine).current_folder(bob), INBOX);
    assert!(mail(&engine).user_folders(bob).is_empty());
}

#[test]
fn folder_name_rules_and_cap() {
    let (mut engine, _, bob) = setup();
    for (input, complaint) in [
        ("bad name", "1-16 characters"),
        ("waaaaaaaaay-too-long-name", "1-16 characters"),
        ("", "1-16 characters"),
        ("inbox", "built-in folder"),
        ("TRASH", "built-in folder"),
    ] {
        let out = run(&mut engine, bob, &format!("@mail/newfolder {input}"));
        assert_actor_saw(out.events(), bob, complaint);
    }
    run(&mut engine, bob, "@mail/newfolder dup");
    let out = run(&mut engine, bob, "@mail/newfolder DUP");
    assert_actor_saw(out.events(), bob, "already exists");
    // Cap: 15 more fill the 16 slots; the 17th is refused.
    for i in 1..16 {
        let out = run(&mut engine, bob, &format!("@mail/newfolder f{i}"));
        assert_actor_saw(out.events(), bob, "created");
    }
    let out = run(&mut engine, bob, "@mail/newfolder overflow");
    assert_actor_saw(out.events(), bob, "at most 16 folders");
    let out = run(&mut engine, bob, "@mail/folder nosuch");
    assert_actor_saw(out.events(), bob, "MAIL: No folder named nosuch.");
}

// ----------------------------------------------------------------------
// Soft delete
// ----------------------------------------------------------------------

#[test]
fn soft_delete_moves_to_hidden_trash_and_back() {
    let (mut engine, alice, bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=Oops/x");
    let out = run(&mut engine, bob, "@mail/delete 1");
    assert_actor_saw(out.events(), bob, "MAIL: Message 1 moved to trash.");
    let st = mail(&engine);
    assert_eq!(st.entry(bob, 1).unwrap().folder.as_str(), TRASH);
    assert_eq!(
        st.message(1).unwrap().body,
        "x",
        "soft: the message survives"
    );
    assert_eq!(
        st.unread_counts(bob),
        (0, 0),
        "trash is excluded from unread"
    );
    let out = run(&mut engine, bob, "@mail");
    assert_actor_saw(out.events(), bob, "MAIL: Folder inbox is empty.");
    // The trash is reachable by name for review and un-deletion.
    run(&mut engine, bob, "@mail/folder trash");
    let out = run(&mut engine, bob, "@mail");
    assert_actor_saw(out.events(), bob, "Oops");
    let out = run(&mut engine, bob, "@mail/delete 1");
    assert_actor_saw(out.events(), bob, "already in the trash");
    run(&mut engine, bob, "@mail/file 1=inbox");
    assert_eq!(mail(&engine).entry(bob, 1).unwrap().folder.as_str(), INBOX);
    assert_eq!(mail(&engine).unread_counts(bob), (1, 0));
}

// ----------------------------------------------------------------------
// Quotas + mail lock
// ----------------------------------------------------------------------

#[test]
fn quota_bounces_with_notice_and_staff_are_exempt() {
    let (mut engine, alice, bob) = setup();
    engine
        .world
        .module_state_mut::<MailState>()
        .unwrap()
        .set_quota(1);
    run(&mut engine, alice, "@mail/quick Bob=One/fits");
    let sent_before = mail(&engine).folder_ids(alice, SENT).len();
    let out = run(&mut engine, alice, "@mail/quick Bob=Two/bounces");
    assert_actor_saw(out.events(), alice, "Bob's mailbox is full.");
    assert_actor_saw(out.events(), alice, "Nobody could receive your message.");
    let st = mail(&engine);
    assert_eq!(st.entry_count(bob), 1, "nothing delivered");
    assert!(st.message(2).is_none(), "bounced message is not stored");
    assert_eq!(
        st.folder_ids(alice, SENT).len(),
        sent_before,
        "no sent copy for a fully-bounced message"
    );
    // Staff are exempt: a wizard's mailbox never fills.
    engine.world.set_flag(bob, Flags::WIZARD, true).unwrap();
    let out = run(&mut engine, alice, "@mail/quick Bob=Three/lands");
    assert_actor_saw(out.events(), alice, "sent to Bob");
    assert_eq!(mail(&engine).entry_count(bob), 2);
}

#[test]
fn mail_lock_refusals_are_reported_per_recipient() {
    let (mut engine, alice, bob) = setup();
    let carol = engine
        .world
        .create(ObjKind::Player, "Carol", GOD, Some(LIMBO))
        .unwrap();
    // Bob only accepts mail from Carol.
    engine
        .world
        .get_mut(bob)
        .unwrap()
        .locks
        .insert(LockType::Mail, LockExpr::Is(carol));
    // Partial fan-out: Carol gets it, Bob's refusal is reported.
    let out = run(&mut engine, alice, "@mail/quick Bob,Carol=Hi/there");
    assert_actor_saw(out.events(), alice, "MAIL: Message 1 sent to Carol.");
    assert_actor_saw(out.events(), alice, "Bob is refusing your mail.");
    let st = mail(&engine);
    assert!(st.entry(bob, 1).is_none());
    assert!(st.entry(carol, 1).is_some());
    assert_eq!(st.message(1).unwrap().to, vec![carol]);
    // Carol passes the lock.
    let out = run(&mut engine, carol, "@mail/quick Bob=Ping/pong");
    assert_actor_saw(out.events(), carol, "sent to Bob");
}

// ----------------------------------------------------------------------
// Connect-time summary
// ----------------------------------------------------------------------

#[test]
fn connect_summary_reports_unread_and_urgent() {
    let (mut engine, alice, _) = setup();
    let carol = engine
        .world
        .create(ObjKind::Player, "Carol", GOD, Some(LIMBO))
        .unwrap();
    run(&mut engine, alice, "@mail/quick/urgent Carol=Now/hurry");
    run(&mut engine, alice, "@mail/quick Carol=Later/whenever");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnUp {
            conn: 9,
            actor: carol,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    assert_actor_saw(
        out.events(),
        carol,
        "MAIL: You have 2 unread messages (1 urgent).",
    );
    // A player with no unread mail gets no MAIL line.
    let dave = engine
        .world
        .create(ObjKind::Player, "Dave", GOD, Some(LIMBO))
        .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnUp {
            conn: 10,
            actor: dave,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    assert!(
        !sys_texts(&out).iter().any(|t| t.contains("MAIL:")),
        "no summary without unread mail"
    );
}

// ----------------------------------------------------------------------
// Persistence round trip
// ----------------------------------------------------------------------

#[test]
fn persistence_round_trip_rebuilds_state() {
    let (mut engine, alice, bob) = setup();
    // A little history: two messages, one read, one urgent; a user folder;
    // one filing; one soft delete.
    run(&mut engine, alice, "@mail/quick Bob=First/hello");
    run(&mut engine, alice, "@mail/quick/urgent Bob=Second/urgent!");
    run(&mut engine, bob, "@mail 1");
    run(&mut engine, bob, "@mail/newfolder keep");
    run(&mut engine, bob, "@mail/file 2=keep");
    run(&mut engine, bob, "@mail/delete 1");
    // Everything the module wrote through, applied to a fake store.
    let mut kv: BTreeMap<SmolStr, serde_json::Value> = BTreeMap::new();
    apply_ops(&mut kv, engine.take_module_ops());
    let snapshot = mail(&engine).clone();

    // Reboot: same world objects, fresh registry+module, state rebuilt
    // purely from the persisted documents.
    let mut engine2 = engine_from(engine.world.clone());
    let boot = HashMap::from([(
        SmolStr::new("mail"),
        ModuleBootData {
            kv: kv.into_iter().collect(),
            streams: HashMap::new(),
            ..Default::default()
        },
    )]);
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine2.modules_loaded(boot, &mut out, &mut fx).unwrap();

    let st = mail(&engine2);
    assert_eq!(st.next_id(), snapshot.next_id());
    for id in [1u64, 2] {
        assert_eq!(st.message(id), snapshot.message(id), "msg {id}");
        assert_eq!(st.entry(bob, id), snapshot.entry(bob, id), "bob entry {id}");
        assert_eq!(
            st.entry(alice, id),
            snapshot.entry(alice, id),
            "alice entry {id}"
        );
    }
    assert_eq!(st.user_folders(bob), snapshot.user_folders(bob));
    assert_eq!(st.entry(bob, 1).unwrap().folder.as_str(), TRASH);
    assert_eq!(st.entry(bob, 2).unwrap().status, MailStatus::Urgent);
    assert_eq!(
        st.current_folder(bob),
        INBOX,
        "current folder is transient and resets on boot"
    );
    // The rebooted world plays on: listing works, ids continue.
    run(&mut engine2, bob, "@mail/folder keep");
    let out = run(&mut engine2, bob, "@mail");
    assert_actor_saw(out.events(), bob, "Second");
    run(&mut engine2, bob, "@mail/quick Alice=Third/fresh id");
    assert_eq!(mail(&engine2).message(3).unwrap().subject, "Third");
}

#[test]
fn config_quota_loads_from_boot_data() {
    let (mut engine, ..) = setup();
    let boot = HashMap::from([(
        SmolStr::new("mail"),
        ModuleBootData {
            kv: vec![(SmolStr::new("config"), serde_json::json!({"quota": 7}))],
            streams: HashMap::new(),
            ..Default::default()
        },
    )]);
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.modules_loaded(boot, &mut out, &mut fx).unwrap();
    assert_eq!(mail(&engine).quota(), 7);
}

// ----------------------------------------------------------------------
// Review-fix regressions: purge, quota-vs-trash, reply-to-self, the
// durable-ack gate, destroyed-character cleanup
// ----------------------------------------------------------------------

/// The quota counts everything EXCEPT trash, and `@mail/purge` empties the
/// trash for good — so a full mailbox is always self-serviceable:
/// delete + purge frees space with no staff intervention.
#[test]
fn quota_excludes_trash_and_purge_frees_it() {
    let (mut engine, alice, bob) = setup();
    engine
        .world
        .module_state_mut::<MailState>()
        .unwrap()
        .set_quota(1);
    run(&mut engine, alice, "@mail/quick Bob=One/fills the box");
    let out = run(&mut engine, alice, "@mail/quick Bob=Two/bounces");
    assert_actor_saw(out.events(), alice, "Bob's mailbox is full.");
    // Bob trashes the message: it immediately stops counting.
    run(&mut engine, bob, "@mail/delete 1");
    assert_eq!(mail(&engine).entry_count(bob), 0, "trash is quota-free");
    let out = run(&mut engine, alice, "@mail/quick Bob=Three/lands now");
    assert_actor_saw(out.events(), alice, "sent to Bob");
    // Purge deletes the trash entry outright and drops the message doc
    // once nothing references it (Alice's sent copy still does here, so
    // the doc survives until she purges too).
    let _ = engine.take_module_ops();
    let out = run(&mut engine, bob, "@mail/purge");
    assert_actor_saw(out.events(), bob, "MAIL: 1 message purged from trash.");
    let st = mail(&engine);
    assert!(st.entry(bob, 1).is_none(), "purged entry is gone");
    assert!(
        st.message(1).is_some(),
        "the doc survives while Alice's sent copy references it"
    );
    let ops = engine.take_module_ops();
    assert!(ops.iter().any(|op| matches!(
        &op.op,
        ModuleWrite::Del { key } if key.as_str() == format!("box:{}:1", bob.0)
    )));
    // Purging an empty trash reports politely.
    let out = run(&mut engine, bob, "@mail/purge");
    assert_actor_saw(out.events(), bob, "MAIL: Your trash is already empty.");
    // When the LAST reference goes, the message document goes with it.
    run(&mut engine, alice, "@mail/folder sent");
    let n = mail(&engine)
        .folder_ids(alice, SENT)
        .iter()
        .position(|&id| id == 1)
        .expect("alice's sent copy")
        + 1;
    run(&mut engine, alice, &format!("@mail/delete {n}"));
    let _ = engine.take_module_ops();
    run(&mut engine, alice, "@mail/purge");
    assert!(mail(&engine).message(1).is_none(), "orphaned doc deleted");
    let ops = engine.take_module_ops();
    assert!(ops.iter().any(|op| matches!(
        &op.op,
        ModuleWrite::Del { key } if key.as_str() == "msg:1"
    )));
}

/// `/reply` on your own message goes back to its recipients (the code used
/// to mail yourself); replying to a self-addressed note still self-mails.
#[test]
fn reply_to_own_message_targets_its_recipients() {
    let (mut engine, alice, bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=Ping/are you there?");
    // Alice replies to her own sent copy.
    run(&mut engine, alice, "@mail/folder sent");
    run(&mut engine, alice, "@mail/reply 1");
    run(&mut engine, alice, "still me");
    run(&mut engine, alice, "--");
    let m2 = mail(&engine).message(2).expect("reply stored");
    assert_eq!(
        m2.to,
        vec![bob],
        "a reply to your own message goes to its recipients"
    );
    // A note to yourself replies to yourself.
    run(&mut engine, bob, "@mail/quick Bob=Note/self");
    let self_id = mail(&engine)
        .folder_ids(bob, INBOX)
        .last()
        .copied()
        .unwrap();
    run(&mut engine, bob, "@mail/folder inbox");
    let n = mail(&engine)
        .folder_ids(bob, INBOX)
        .iter()
        .position(|&id| id == self_id)
        .unwrap()
        + 1;
    run(&mut engine, bob, &format!("@mail/reply {n}"));
    run(&mut engine, bob, "me again");
    run(&mut engine, bob, "--");
    let m = mail(&engine)
        .message(mail(&engine).next_id() - 1)
        .expect("self reply");
    assert_eq!(m.to, vec![bob]);
}

/// Mail delivery requests the durable-ack output gate: the world task must
/// flush the send's batch `durable: true` and hold "MAIL: sent" until the
/// store acks (PLAN's durability-critical list names mail send).
#[test]
fn send_requests_the_durable_ack_gate() {
    let (mut engine, alice, _bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=Gate/check");
    assert_eq!(
        engine.take_durable_gate(),
        Some((alice, false)),
        "a successful send must gate its output on the store ack (non-strict)"
    );
    // A refused send gates nothing (the handler returned Err, and dispatch
    // discarded everything it deferred).
    let (mut engine, alice, _bob) = setup();
    run(&mut engine, alice, "@mail/quick Nobody=X/y");
    assert_eq!(engine.take_durable_gate(), None);
    assert!(engine.take_module_ops().is_empty());
}

/// Destroying a character removes their mailbox and folder documents;
/// message documents survive while another recipient still holds a copy.
#[test]
fn destroyed_character_mailbox_is_cleaned_up() {
    use mush_core::module::emit_world_event;
    use mush_core::{Deferred, WorldEvent};
    let (mut engine, alice, bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=Bye/so long");
    run(&mut engine, bob, "@mail/newfolder keep");
    let _ = engine.take_module_ops();
    let mut out = Outbox::new();
    emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &WorldEvent::ObjDestroyed { obj: bob },
        mush_core::TypedStateLens::Detached(mush_core::DetachedTypedState::Test),
    );
    let st = mail(&engine);
    assert!(st.entry(bob, 1).is_none(), "box entries removed");
    assert!(st.user_folders(bob).is_empty(), "folders removed");
    assert!(
        st.message(1).is_some(),
        "Alice's sent copy keeps the doc alive"
    );
    let dels: Vec<String> = out
        .take_deferred()
        .into_iter()
        .filter_map(|d| match d {
            Deferred::Persist(ModuleOp {
                op: ModuleWrite::Del { key },
                ..
            }) => Some(key.to_string()),
            _ => None,
        })
        .collect();
    assert!(dels.contains(&format!("box:{}:1", bob.0)));
    assert!(dels.contains(&format!("folders:{}", bob.0)));
}

// ----------------------------------------------------------------------
// Web query handler (M4): mail.list via Inbound::Query
// ----------------------------------------------------------------------

/// Step one `Inbound::Query` and unwrap the conn-scoped `Reply` payload.
fn web_query(
    engine: &mut Engine,
    actor: ObjId,
    name: &str,
    args: serde_json::Value,
) -> OutputEvent {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Query {
            conn: 77,
            actor,
            id: 5,
            name: SmolStr::new(name),
            args,
        },
        &mut out,
        &mut fx,
    );
    let mut events = out.take();
    assert_eq!(events.len(), 1, "exactly one reply: {events:?}");
    let (aud, ev) = events.remove(0);
    assert_eq!(
        aud,
        mush_core::Audience::Conn(77),
        "replies are conn-scoped"
    );
    let OutputEvent::Reply { id: 5, event } = ev else {
        panic!("expected Reply id 5, got {ev:?}");
    };
    *event
}

/// `mail.list` — folder summaries plus one folder's entries, as a `mail`
/// panel keyframe. Reading stays a Line command (it mutates); listing must
/// NOT change any entry's status.
#[test]
fn mail_list_query_reports_folders_and_entries() {
    let (mut engine, alice, bob) = setup();
    run(&mut engine, alice, "@mail/quick Bob=Lunch/Meet at noon?");
    run(&mut engine, alice, "@mail/quick/urgent Bob=Now/really");

    // Default folder (null) is the inbox.
    let ev = web_query(
        &mut engine,
        bob,
        "mail.list",
        serde_json::json!({"folder": null}),
    );
    let OutputEvent::Panel { feed, kind, data } = ev else {
        panic!("expected Panel, got {ev:?}");
    };
    assert_eq!(feed, "mail");
    assert_eq!(kind, mush_proto::PanelFrameKind::Keyframe);
    assert_eq!(data["folder"], "inbox");
    let folders = data["folders"].as_array().unwrap();
    let inbox = folders.iter().find(|f| f["name"] == "inbox").unwrap();
    assert_eq!(inbox["count"], 2);
    assert_eq!(inbox["unread"], 2);
    assert!(
        folders.iter().all(|f| f["name"] != "trash"),
        "trash stays hidden from summaries: {folders:?}"
    );
    let entries = data["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["index"], 1);
    assert_eq!(entries[0]["id"], 1);
    assert_eq!(entries[0]["from"]["name"], "Alice");
    assert_eq!(entries[0]["subject"], "Lunch");
    assert_eq!(entries[0]["status"], "unread");
    assert_eq!(entries[0]["sent_at"], NOW);
    assert_eq!(entries[1]["status"], "urgent");
    // Mortals see their quota.
    assert_eq!(data["quota"]["used"], 2);

    // Listing is read-only: nothing was marked read by the query.
    let st = mail(&engine);
    assert_eq!(st.entry(bob, 1).unwrap().status, MailStatus::Unread);

    // The sender's copy lands pre-read in `sent`.
    let ev = web_query(
        &mut engine,
        alice,
        "mail.list",
        serde_json::json!({"folder": "sent"}),
    );
    let OutputEvent::Panel { data, .. } = ev else {
        panic!("expected Panel, got {ev:?}");
    };
    assert_eq!(data["folder"], "sent");
    let entries = data["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["status"], "read");

    // Unknown folders refuse, correlated as a Sys reply.
    let ev = web_query(
        &mut engine,
        bob,
        "mail.list",
        serde_json::json!({"folder": "nope"}),
    );
    let OutputEvent::Sys(t) = ev else {
        panic!("expected Sys refusal, got {ev:?}");
    };
    assert!(t.to_plain_text().contains("No folder named nope"));
}
