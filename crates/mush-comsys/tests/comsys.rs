//! End-to-end comsys tests through the real [`Engine`] (the ToyModule
//! harness pattern): join/speak/pose forms, titles, gag, recall, all four
//! lock gates, the alias-shadow warning, the LOUD login notice, the
//! ObjCreated/ObjDestroyed observers, and the full persistence round trip
//! (drain module ops → `InMemoryStore` → fresh boot → identical state).

use std::collections::HashMap;

use mush_comsys::{
    Channel, ChannelId, ComsysModule, ComsysState, DEFAULT_CHANNEL, MAX_HISTORY_CAP, Membership, NS,
};
use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::module::{ModuleBootData, emit_world_event};
use mush_core::{
    Audience, Deferred, DisconnectKind, GOD, GameModule, LIMBO, ModuleOp, ModuleWrite, ObjId,
    ObjKind, Outbox, Registry, World, WorldEvent, register_core_commands, register_engine_commands,
};
use mush_proto::{NarrativeKind, OutputEvent};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use smol_str::SmolStr;

/// A store-less lens for this harness: it drives the engine's public
/// event fan-out with no live typed-state snapshot and says so.
const DETACHED: mush_core::TypedStateLens<'static> =
    mush_core::TypedStateLens::Detached(mush_core::DetachedTypedState::Test);

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

/// Engine with core + engine commands and the comsys module, now = 1000s,
/// players Nick and Ann in Limbo, everyone connected, `modules_loaded`
/// already called with an empty store (first boot). Module ops are NOT
/// drained — [`setup`] does that.
fn boot() -> (Engine, ObjId, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let ann = world
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();
    world.set_connected(GOD, true);
    world.set_connected(nick, true);
    world.set_connected(ann, true);
    // Delivery (and cursor advance) requires live transport connections,
    // not just the world-side connected flag — mirror one live conn each,
    // as the engine does on ConnUp.
    world.set_live_conns(GOD, 1);
    world.set_live_conns(nick, 1);
    world.set_live_conns(ann, 1);
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, vec![Box::new(ComsysModule::new())]).unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();
    (engine, nick, ann)
}

/// [`boot`] with the first-boot seeding ops drained away.
fn setup() -> (Engine, ObjId, ObjId) {
    let (mut engine, nick, ann) = boot();
    let _ = engine.take_module_ops();
    (engine, nick, ann)
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

fn state(engine: &Engine) -> &ComsysState {
    engine
        .world
        .module_state::<ComsysState>()
        .expect("comsys state")
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

/// One received channel frame, flattened for assertions.
#[derive(Debug, Clone, PartialEq)]
struct ChanEv {
    to: ObjId,
    channel: String,
    seq: u64,
    actor: String,
    title: Option<String>,
    kind: NarrativeKind,
    text: String,
    ts: i64,
}

fn chan_events(out: &Outbox) -> Vec<ChanEv> {
    out.events()
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (
                Audience::Actor(a),
                OutputEvent::Channel {
                    channel,
                    seq,
                    actor,
                    title,
                    kind,
                    text,
                    ts,
                },
            ) => Some(ChanEv {
                to: *a,
                channel: channel.to_string(),
                seq: *seq,
                actor: actor.name.clone(),
                title: title.clone(),
                kind: *kind,
                text: text.to_plain_text(),
                ts: *ts,
            }),
            _ => None,
        })
        .collect()
}

fn recipients(out: &Outbox) -> Vec<ObjId> {
    let mut v: Vec<ObjId> = chan_events(out).iter().map(|e| e.to).collect();
    v.sort();
    v
}

// ----------------------------------------------------------------------
// First boot + auto-join
// ----------------------------------------------------------------------

#[test]
fn first_boot_seeds_public_and_autojoins_everyone() {
    let (mut engine, nick, ann) = boot();
    let st = state(&engine);
    assert_eq!(st.channels.len(), 1);
    let public = st.channel_by_name(DEFAULT_CHANNEL).expect("Public exists");
    assert!(public.flags.loud && public.flags.logged && !public.flags.no_titles);
    assert!(public.history.persist);
    assert_eq!(public.seq, 0);
    let id = public.id;
    // God, Nick, and Ann are all members under the alias "pub".
    for who in [GOD, nick, ann] {
        let m = st.members.get(&(who, id)).expect("auto-joined");
        assert_eq!(m.alias, "pub");
        assert!(!m.gagged);
        assert_eq!(m.last_read, 0);
    }
    assert_eq!(st.next_chan_id, 1);
    // The seed mirrored everything: channel doc, meta, one doc per member.
    let ops = engine.take_module_ops();
    let keys: Vec<String> = ops
        .iter()
        .map(|op| match &op.op {
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => key.to_string(),
            ModuleWrite::StreamAppend { key, .. } => key.to_string(),
        })
        .collect();
    assert!(ops.iter().all(|op| op.ns == NS));
    assert!(keys.contains(&format!("chan:{id}")));
    assert!(keys.contains(&"meta".to_string()));
    for who in [GOD, nick, ann] {
        assert!(keys.contains(&format!("member:{}:{id}", who.0)));
    }
}

#[test]
fn obj_created_observer_autojoins_new_players_only() {
    let (mut engine, _, _) = setup();
    let bella = engine
        .world
        .create(ObjKind::Player, "Bella", GOD, Some(LIMBO))
        .unwrap();
    let ball = engine
        .world
        .create(ObjKind::Thing, "ball", GOD, Some(LIMBO))
        .unwrap();
    let mut out = Outbox::new();
    emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &WorldEvent::ObjCreated {
            obj: bella,
            kind: ObjKind::Player,
            creator: GOD,
        },
        DETACHED,
    );
    emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &WorldEvent::ObjCreated {
            obj: ball,
            kind: ObjKind::Thing,
            creator: GOD,
        },
        DETACHED,
    );
    let st = state(&engine);
    let public = st.channel_by_name(DEFAULT_CHANNEL).unwrap().id;
    assert!(st.members.contains_key(&(bella, public)), "player joined");
    assert!(!st.members.contains_key(&(ball, public)), "things do not");
    // The join was mirrored on the deferred side-channel.
    assert!(out.deferred().iter().any(|d| matches!(
        d,
        Deferred::Persist(ModuleOp { ns, op: ModuleWrite::JsonSet { key, .. } })
            if ns == NS && key.as_str() == format!("member:{}:{public}", bella.0)
    )));
}

#[test]
fn obj_destroyed_observer_drops_memberships() {
    let (mut engine, _, ann) = setup();
    let public = state(&engine).channel_by_name(DEFAULT_CHANNEL).unwrap().id;
    let mut out = Outbox::new();
    emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &WorldEvent::ObjDestroyed { obj: ann },
        DETACHED,
    );
    assert!(!state(&engine).members.contains_key(&(ann, public)));
    assert!(out.deferred().iter().any(|d| matches!(
        d,
        Deferred::Persist(ModuleOp { op: ModuleWrite::Del { key }, .. })
            if key.as_str() == format!("member:{}:{public}", ann.0)
    )));
}

// ----------------------------------------------------------------------
// Speaking: say / pose / semipose, delivery, cursors
// ----------------------------------------------------------------------

#[test]
fn speaking_delivers_all_three_pose_forms() {
    let (mut engine, nick, ann) = setup();
    let out = run(&mut engine, nick, "pub hello world");
    let evs = chan_events(&out);
    assert_eq!(evs.len(), 3, "God, Nick, Ann each get one frame");
    assert_eq!(recipients(&out), vec![GOD, nick, ann]);
    let ev = evs.iter().find(|e| e.to == ann).unwrap();
    assert_eq!(ev.channel, "Public");
    assert_eq!(ev.seq, 1);
    assert_eq!(ev.actor, "Nick");
    assert_eq!(ev.title, None);
    assert_eq!(ev.kind, NarrativeKind::Say);
    assert_eq!(ev.text, "hello world");
    assert_eq!(ev.ts, 1_000_000);

    let out = run(&mut engine, nick, "pub :waves.");
    let ev = &chan_events(&out)[0];
    assert_eq!(
        (ev.kind, ev.text.as_str(), ev.seq),
        (NarrativeKind::Pose, "waves.", 2)
    );

    let out = run(&mut engine, nick, "pub ;'s eyes gleam.");
    let ev = &chan_events(&out)[0];
    assert_eq!(
        (ev.kind, ev.text.as_str(), ev.seq),
        (NarrativeKind::SemiPose, "'s eyes gleam.", 3)
    );

    // Delivery advanced (and will mirror) everyone's unread cursor.
    let st = state(&engine);
    let public = st.channel_by_name(DEFAULT_CHANNEL).unwrap().id;
    for who in [GOD, nick, ann] {
        assert_eq!(st.members.get(&(who, public)).unwrap().last_read, 3);
    }
    assert_eq!(st.channels.get(&public).unwrap().seq, 3);
}

#[test]
fn alias_is_case_insensitive_and_bare_alias_prompts() {
    let (mut engine, nick, _) = setup();
    let out = run(&mut engine, nick, "PUB shouted");
    assert_eq!(chan_events(&out).len(), 3);
    // A bare alias is claimed by the stage (no Huh) and prompts.
    let out = run(&mut engine, nick, "pub");
    assert!(chan_events(&out).is_empty());
    let texts = sys_texts(&out);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("What do you want to say on Public?")),
        "got: {texts:?}"
    );
    assert!(!texts.iter().any(|t| t.contains("Huh?")));
    // An unknown alias is not ours: normal dispatch (Huh).
    let out = run(&mut engine, nick, "zz hi");
    assert!(sys_texts(&out).iter().any(|t| t.contains("Huh?")));
}

#[test]
fn speaking_marks_message_and_mirrors_docs() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, nick, "pub logged line");
    let ops = engine.take_module_ops();
    let public = state(&engine).channel_by_name(DEFAULT_CHANNEL).unwrap().id;
    // Channel doc (seq bump), three member docs (cursors), one stream append.
    assert!(ops.iter().any(|op| matches!(
        &op.op,
        ModuleWrite::JsonSet { key, value }
            if key.as_str() == format!("chan:{public}") && value["seq"] == 1
    )));
    assert!(ops.iter().any(|op| matches!(
        &op.op,
        ModuleWrite::StreamAppend { key, value, maxlen }
            if key.as_str() == format!("hist:{public}")
                && value["text"] == "logged line"
                && *maxlen == 200
    )));
    let member_docs = ops
        .iter()
        .filter(
            |op| matches!(&op.op, ModuleWrite::JsonSet { key, .. } if key.starts_with("member:")),
        )
        .count();
    assert_eq!(member_docs, 3);
}

#[test]
fn speaking_with_many_offline_members_only_updates_the_live_audience() {
    const OFFLINE_MEMBERS: u32 = 10_000;

    let (mut engine, nick, ann) = setup();
    let public = state(&engine).channel_by_name(DEFAULT_CHANNEL).unwrap().id;
    engine.world.set_live_conns(ann, 0);
    engine.world.set_connected(ann, false);
    {
        let st = engine
            .world
            .module_state_mut::<ComsysState>()
            .expect("comsys state");
        for raw in 10_000..10_000 + OFFLINE_MEMBERS {
            let player = ObjId(raw);
            st.members.insert(
                (player, public),
                Membership {
                    player,
                    channel: public,
                    alias: "pub".into(),
                    title: None,
                    gagged: false,
                    last_read: 0,
                },
            );
        }
    }

    let out = run(&mut engine, nick, "pub bounded fanout");

    assert_eq!(recipients(&out), vec![GOD, nick]);
    let st = state(&engine);
    assert_eq!(st.members.get(&(GOD, public)).unwrap().last_read, 1);
    assert_eq!(st.members.get(&(nick, public)).unwrap().last_read, 1);
    assert_eq!(st.members.get(&(ann, public)).unwrap().last_read, 0);
    assert_eq!(
        st.members
            .range((ObjId(10_000), public)..=(ObjId(19_999), public))
            .filter(|(_, membership)| membership.channel == public)
            .filter(|(_, membership)| membership.last_read != 0)
            .count(),
        0,
        "offline cursors must not advance"
    );

    let member_docs = engine
        .take_module_ops()
        .into_iter()
        .filter(|op| {
            matches!(
                &op.op,
                ModuleWrite::JsonSet { key, .. } if key.starts_with("member:")
            )
        })
        .count();
    assert_eq!(member_docs, 2, "only delivered cursors are mirrored");
}

// ----------------------------------------------------------------------
// Titles
// ----------------------------------------------------------------------

#[test]
fn comtitle_shows_on_messages_and_no_titles_suppresses() {
    let (mut engine, nick, _) = setup();
    let out = run(&mut engine, nick, "comtitle pub=the Bold");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Title on Public set"))
    );
    let out = run(&mut engine, nick, "pub titled");
    assert_eq!(chan_events(&out)[0].title.as_deref(), Some("the Bold"));

    // NO_TITLES suppresses existing titles and refuses new ones.
    run(&mut engine, GOD, "@channel/set Public=no_titles");
    let out = run(&mut engine, nick, "pub untitled");
    assert_eq!(chan_events(&out)[0].title, None);
    let out = run(&mut engine, nick, "comtitle pub=sneaky");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Titles are disabled on Public."))
    );
    // Clearing still works, and an over-long title is refused.
    let out = run(&mut engine, nick, "comtitle pub=");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Title on Public cleared."))
    );
    let long = "x".repeat(61);
    let out = run(&mut engine, nick, &format!("comtitle pub={long}"));
    assert!(sys_texts(&out).iter().any(|t| t.contains("too long")));
}

// ----------------------------------------------------------------------
// Gagging
// ----------------------------------------------------------------------

#[test]
fn gag_suppresses_delivery_and_speaking_until_ungag() {
    let (mut engine, nick, ann) = setup();
    let out = run(&mut engine, ann, "@channel/gag pub");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("You gag Public."))
    );
    let out = run(&mut engine, nick, "pub psst");
    assert_eq!(recipients(&out), vec![GOD, nick], "Ann hears nothing");
    // Ann's cursor did not advance: the message stays unread for her.
    let public = state(&engine).channel_by_name(DEFAULT_CHANNEL).unwrap().id;
    assert_eq!(
        state(&engine)
            .members
            .get(&(ann, public))
            .unwrap()
            .last_read,
        0
    );
    // Gagged members may not speak.
    let out = run(&mut engine, ann, "pub hello?");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("You have Public gagged."))
    );
    assert!(chan_events(&out).is_empty());
    // Ungag restores delivery.
    run(&mut engine, ann, "@channel/ungag pub");
    let out = run(&mut engine, nick, "pub again");
    assert_eq!(recipients(&out), vec![GOD, nick, ann]);
}

// ----------------------------------------------------------------------
// Locks: join / speak / see
// ----------------------------------------------------------------------

#[test]
fn join_lock_refuses_addcom() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, GOD, "@channel/create Staff=Staff chatter");
    run(&mut engine, GOD, "@channel/perm Staff=join=flag^WIZARD");
    let out = run(&mut engine, nick, "addcom st=Staff");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Channel Staff does not admit you.")),
        "got: {:?}",
        sys_texts(&out)
    );
    assert!(state(&engine).membership_by_alias(nick, "st").is_none());
    // A wizard passes the lock.
    let out = run(&mut engine, GOD, "addcom stf=Staff");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Channel Staff added"))
    );
}

#[test]
fn speak_lock_blocks_speaking_but_not_hearing() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, GOD, "@channel/perm Public=speak=flag^WIZARD");
    let out = run(&mut engine, nick, "pub can I talk?");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("You cannot speak on Public."))
    );
    assert!(chan_events(&out).is_empty());
    // God can, and Nick still hears it (see lock is open).
    let out = run(&mut engine, GOD, "pub yes I can");
    assert!(recipients(&out).contains(&nick));
}

#[test]
fn see_lock_hides_from_clist_who_and_delivery() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, GOD, "@channel/create Sekrit=Hidden staff talk");
    run(&mut engine, nick, "addcom sk=Sekrit");
    run(&mut engine, GOD, "addcom sek=Sekrit");
    run(&mut engine, GOD, "@channel/perm Sekrit=see=flag^WIZARD");
    // Delivery: only God passes the see lock now.
    let out = run(&mut engine, GOD, "sek quiet words");
    assert_eq!(recipients(&out), vec![GOD], "Nick is hidden from delivery");
    // clist: hidden from Nick, visible to God (admin bypass).
    let out = run(&mut engine, nick, "clist");
    assert!(!sys_texts(&out).iter().any(|t| t.contains("Sekrit")));
    assert!(sys_texts(&out).iter().any(|t| t.contains("Public")));
    let out = run(&mut engine, GOD, "clist");
    assert!(sys_texts(&out).iter().any(|t| t.contains("Sekrit")));
    // who: reads as missing to Nick.
    let out = run(&mut engine, nick, "@channel/who Sekrit");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("No such channel."))
    );
    let out = run(&mut engine, GOD, "@channel/who Sekrit");
    assert!(sys_texts(&out).iter().any(|t| t.contains("Channel Sekrit")));
}

// ----------------------------------------------------------------------
// Alias shadowing
// ----------------------------------------------------------------------

#[test]
fn addcom_warns_when_alias_shadows_command_or_exit_and_alias_wins() {
    let (mut engine, nick, _) = setup();
    // An exit "down" in Limbo.
    run(&mut engine, GOD, "@dig Den=down;d,up;u");
    let out = run(&mut engine, GOD, "addcom down=Public");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Warning: 'down' also matches")),
        "got: {:?}",
        sys_texts(&out)
    );
    // A command name warns too.
    run(&mut engine, GOD, "@channel/create Foo");
    let out = run(&mut engine, nick, "addcom say=Foo");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Warning: 'say' also matches"))
    );
    // A clean alias does not warn.
    let out = run(&mut engine, nick, "addcom zz=Foo");
    assert!(!sys_texts(&out).iter().any(|t| t.contains("Warning:")));
    // Dispatch order: the alias wins over the exit — God speaks, stays put.
    let out = run(&mut engine, GOD, "down told you");
    assert_eq!(chan_events(&out)[0].text, "told you");
    assert_eq!(engine.world.get(GOD).unwrap().location, Some(LIMBO));
}

// ----------------------------------------------------------------------
// LOUD login notice
// ----------------------------------------------------------------------

#[test]
fn loud_channels_announce_unread_counts_at_login() {
    let (mut engine, nick, ann) = setup();
    engine.world.set_connected(ann, false);
    for i in 0..3 {
        run(&mut engine, nick, &format!("pub missed {i}"));
    }
    // Ann connects: three unread on the LOUD channel.
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnUp {
            conn: 50,
            actor: ann,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t == "[Public] 3 messages since you last read."),
        "got: {:?}",
        sys_texts(&out)
    );
    // She hears one live message (cursor catches up), disconnects, misses one.
    run(&mut engine, nick, "pub caught live");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnDown {
            conn: 50,
            actor: ann,
            reason: DisconnectKind::Quit,
        },
        &mut out,
        &mut fx,
    );
    run(&mut engine, nick, "pub missed again");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnUp {
            conn: 51,
            actor: ann,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t == "[Public] 1 message since you last read."),
        "singular form: {:?}",
        sys_texts(&out)
    );
    // Fully read: no notice. And gagged memberships never notice.
    run(&mut engine, GOD, "pub read this");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnDown {
            conn: 51,
            actor: ann,
            reason: DisconnectKind::Quit,
        },
        &mut out,
        &mut fx,
    );
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnUp {
            conn: 52,
            actor: ann,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    assert!(
        !sys_texts(&out)
            .iter()
            .any(|t| t.contains("since you last read")),
        "no unread, no notice: {:?}",
        sys_texts(&out)
    );
}

// ----------------------------------------------------------------------
// History: recall + caps
// ----------------------------------------------------------------------

#[test]
fn recall_replays_history_and_honors_cap() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, GOD, "@channel/set Public=cap=3");
    for i in 1..=5 {
        run(&mut engine, nick, &format!("pub message {i}"));
    }
    // The live ring is capped at 3.
    let public = state(&engine).channel_by_name(DEFAULT_CHANNEL).unwrap().id;
    assert_eq!(state(&engine).rings.get(&public).unwrap().len(), 3);
    let out = run(&mut engine, GOD, "@channel/recall pub");
    let texts = sys_texts(&out);
    assert!(texts.iter().any(|t| t.contains("last 3 messages")));
    assert!(texts.iter().any(|t| t.contains("Nick: message 3")));
    assert!(texts.iter().any(|t| t.contains("Nick: message 5")));
    assert!(!texts.iter().any(|t| t.contains("message 2")));
    // Explicit depth.
    let out = run(&mut engine, GOD, "@channel/recall pub=2");
    let texts = sys_texts(&out);
    assert!(texts.iter().any(|t| t.contains("last 2 messages")));
    assert!(!texts.iter().any(|t| t.contains("message 3")));
    // Bad depth.
    let out = run(&mut engine, GOD, "@channel/recall pub=zero");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Recall how many?"))
    );
    // Unlogged channels refuse.
    run(&mut engine, GOD, "@channel/create Quiet");
    run(&mut engine, GOD, "addcom q=Quiet");
    let out = run(&mut engine, GOD, "@channel/recall q");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("does not log history"))
    );
}

#[test]
fn recall_renders_pose_forms_and_titles() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, nick, "comtitle pub=the Bold");
    run(&mut engine, nick, "pub :waves.");
    let out = run(&mut engine, nick, "@channel/recall pub");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("[Public] the Bold Nick waves.")),
        "got: {:?}",
        sys_texts(&out)
    );
}

// ----------------------------------------------------------------------
// Admin surface
// ----------------------------------------------------------------------

/// `@channel/perm` is the comsys's own `parse_lock` entry point, so it has
/// to refuse an over-deep key the same way `@lock` does: a message to the
/// admin, the channel's existing permission untouched, and — because a
/// module op is what would reach Redis — nothing queued to persist.
#[test]
fn over_deep_channel_perm_is_refused_and_mutates_nothing() {
    let (mut engine, _nick, _ann) = setup();
    let before = state(&engine).channel_by_name("Public").unwrap().clone();
    for key in [
        "(".repeat(8192),
        "!".repeat(8192),
        format!("{}#1", "#1&".repeat(2730)),
        format!("{}#1", "#1|".repeat(2730)),
    ] {
        let out = run(
            &mut engine,
            GOD,
            &format!("@channel/perm Public=speak={key}"),
        );
        assert!(
            sys_texts(&out)
                .iter()
                .any(|t| t.contains("I don't understand that lock.")),
            "over-deep key should be refused: {:?}",
            sys_texts(&out)
        );
        let after = state(&engine).channel_by_name("Public").unwrap();
        assert_eq!(after.perms.speak, before.perms.speak);
        assert_eq!(after.seq, before.seq, "a refused perm must not bump seq");
        assert!(
            engine.take_module_ops().is_empty(),
            "a refused perm must queue no module write"
        );
    }
    // The refusal names the depth limit rather than some other parse error.
    let out = run(
        &mut engine,
        GOD,
        &format!("@channel/perm Public=speak={}#1", "#1&".repeat(2730)),
    );
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("nests too deeply")),
        "{:?}",
        sys_texts(&out)
    );
}

#[test]
fn admin_commands_are_gated() {
    let (mut engine, nick, _) = setup();
    for line in [
        "@channel/create X",
        "@channel/destroy Public",
        "@channel/perm Public=speak=#1",
        "@channel/color Public=red,#ff0000",
        "@channel/set Public=loud",
        "@channel/list",
    ] {
        let out = run(&mut engine, nick, line);
        assert!(
            sys_texts(&out)
                .iter()
                .any(|t| t.contains("Permission denied.")),
            "{line} should be denied: {:?}",
            sys_texts(&out)
        );
    }
    // Exactly one switch is required.
    let out = run(&mut engine, GOD, "@channel Public");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Pick exactly one switch"))
    );
    // The admin lock delegates to non-staff.
    run(&mut engine, GOD, "@channel/create Club");
    run(&mut engine, GOD, "@channel/perm Club=admin=*Nick");
    let out = run(&mut engine, nick, "@channel/set Club=loud");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Flag loud set on Club.")),
        "admin lock should delegate: {:?}",
        sys_texts(&out)
    );
}

#[test]
fn create_color_perm_validation_and_destroy_cleanup() {
    let (mut engine, nick, _) = setup();
    // Validation.
    let out = run(&mut engine, GOD, "@channel/create Bad Name");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("not a usable channel name"))
    );
    let out = run(&mut engine, GOD, "@channel/create Public");
    assert!(sys_texts(&out).iter().any(|t| t.contains("already exists")));
    let out = run(&mut engine, GOD, "@channel/color Public=red,notahex");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("not a hex color"))
    );
    let out = run(&mut engine, GOD, "@channel/perm Public=speak=((");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("I don't understand that lock."))
    );
    let out = run(&mut engine, GOD, "@channel/perm Public=bogus=#1");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Usage: @channel/perm"))
    );
    // Color happy path.
    run(&mut engine, GOD, "@channel/color Public=red,#FF0000");
    let public = state(&engine).channel_by_name("Public").unwrap().clone();
    assert_eq!(public.color.ansi, "red");
    assert_eq!(public.color.web_hex, "#ff0000");
    // Destroy: state gone, members dropped, docs and stream deleted.
    run(&mut engine, GOD, "@channel/create Temp");
    run(&mut engine, nick, "addcom t=Temp");
    let temp = state(&engine).channel_by_name("Temp").unwrap().id;
    let _ = engine.take_module_ops();
    let out = run(&mut engine, GOD, "@channel/destroy Temp");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Channel Temp destroyed."))
    );
    // Nick (a connected member) was told.
    assert!(out.events().iter().any(|(aud, e)| matches!(
        (aud, e),
        (Audience::Actor(a), OutputEvent::Sys(t))
            if *a == nick && t.to_plain_text().contains("[Temp] Channel destroyed by God.")
    )));
    assert!(state(&engine).channel_by_name("Temp").is_none());
    assert!(!state(&engine).members.contains_key(&(nick, temp)));
    let ops = engine.take_module_ops();
    let dels: Vec<String> = ops
        .iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::Del { key } => Some(key.to_string()),
            _ => None,
        })
        .collect();
    assert!(dels.contains(&format!("chan:{temp}")));
    assert!(dels.contains(&format!("hist:{temp}")));
    assert!(dels.contains(&format!("member:{}:{temp}", nick.0)));
}

#[test]
fn comlist_and_delcom_lifecycle() {
    let (mut engine, nick, _) = setup();
    let out = run(&mut engine, nick, "comlist");
    let texts = sys_texts(&out);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("pub") && t.contains("Public"))
    );
    assert!(texts.iter().any(|t| t.contains("-- 1 channel --")));
    // Leave.
    let out = run(&mut engine, nick, "delcom pub");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Channel Public removed"))
    );
    let public = state(&engine).channel_by_name("Public").unwrap().id;
    assert!(!state(&engine).members.contains_key(&(nick, public)));
    let ops = engine.take_module_ops();
    assert!(ops.iter().any(|op| matches!(
        &op.op,
        ModuleWrite::Del { key } if key.as_str() == format!("member:{}:{public}", nick.0)
    )));
    // The alias no longer speaks: normal dispatch takes over.
    let out = run(&mut engine, nick, "pub anyone?");
    assert!(sys_texts(&out).iter().any(|t| t.contains("Huh?")));
    // And an empty comlist says so.
    let out = run(&mut engine, nick, "comlist");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("You are on no channels."))
    );
}

#[test]
fn addcom_realias_and_duplicate_alias_refusal() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, GOD, "@channel/create Foo");
    run(&mut engine, nick, "addcom foo=Foo");
    // Same alias for a different channel: refused.
    let out = run(&mut engine, nick, "addcom foo=Public");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("already use the alias 'foo'"))
    );
    // Re-aliasing the same channel: fine.
    let out = run(&mut engine, nick, "addcom f=Foo");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Alias for channel Foo is now 'f'."))
    );
    assert!(state(&engine).membership_by_alias(nick, "f").is_some());
    assert!(state(&engine).membership_by_alias(nick, "foo").is_none());
}

// ----------------------------------------------------------------------
// Persistence round trip
// ----------------------------------------------------------------------

/// Build the second engine over an identical world, boot it from the boot
/// data read back out of the store, and return it.
fn reboot_from_store(store: &InMemoryStore) -> Engine {
    let (kv, streams) = futures::executor::block_on(async {
        let kv = store.module_json_list(NS).await.unwrap();
        // The driver recipe: every `chan:{id}` doc names its history
        // stream `hist:{id}`; load each with the doc's own cap.
        let mut streams = HashMap::new();
        for (key, value) in &kv {
            if let Some(id) = key.strip_prefix("chan:") {
                let cap = value["history"]["cap"].as_u64().unwrap_or(200);
                let sk = format!("hist:{id}");
                let entries = store
                    .stream_range(NS, &sk, u32::try_from(cap).unwrap_or(200))
                    .await
                    .unwrap();
                if !entries.is_empty() {
                    streams.insert(SmolStr::new(&sk), entries);
                }
            }
        }
        (kv, streams)
    });
    let boot_data = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams,
        ..Default::default()
    };
    let mut world = World::new_minimal();
    world.set_now(1_000);
    world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    world
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, vec![Box::new(ComsysModule::new())]).unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(
            HashMap::from([(SmolStr::new(NS), boot_data)]),
            &mut out,
            &mut fx,
        )
        .unwrap();
    engine
}

#[test]
fn persistence_round_trip_rebuilds_identical_state() {
    let (mut engine, nick, ann) = boot();
    let mut ops: Vec<ModuleOp> = engine.take_module_ops();
    for (actor, line) in [
        (GOD, "@channel/create Staff=Senior staff"),
        (GOD, "@channel/perm Staff=join=flag^WIZARD"),
        (GOD, "@channel/perm Staff=see=flag^WIZARD"),
        (GOD, "@channel/color Staff=magenta,#ff00ff"),
        (GOD, "@channel/set Staff=logged"),
        (GOD, "@channel/set Staff=persist"),
        (GOD, "addcom st=Staff"),
        (nick, "comtitle pub=the Bold"),
        (ann, "@channel/gag pub"),
        (nick, "pub hello there"),
        (nick, "pub :waves to everyone."),
        (GOD, "st wizards only"),
        (GOD, "pub message three"),
    ] {
        let out = run(&mut engine, actor, line);
        assert!(
            !sys_texts(&out).iter().any(|t| t.contains("Huh?")),
            "{line} failed: {:?}",
            sys_texts(&out)
        );
        ops.extend(engine.take_module_ops());
    }
    let store = InMemoryStore::new();
    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
    let engine2 = reboot_from_store(&store);
    assert_eq!(
        state(&engine),
        state(&engine2),
        "channels, memberships, rings, and the id allocator all round-trip"
    );
    // Sanity on the rebuilt side: seq continues, titles/gags survive.
    let st2 = state(&engine2);
    let public = st2.channel_by_name("Public").unwrap();
    assert_eq!(public.seq, 3);
    assert_eq!(
        st2.members
            .get(&(nick, public.id))
            .unwrap()
            .title
            .as_deref(),
        Some("the Bold")
    );
    assert!(st2.members.get(&(ann, public.id)).unwrap().gagged);
    assert_eq!(st2.rings.get(&public.id).unwrap().len(), 3);
    assert!(st2.channel_by_name("Staff").is_some());
}

#[test]
fn history_cap_survives_round_trip_exactly() {
    let (mut engine, nick, _) = boot();
    let mut ops: Vec<ModuleOp> = engine.take_module_ops();
    ops.extend({
        run(&mut engine, GOD, "@channel/set Public=cap=3");
        engine.take_module_ops()
    });
    for i in 1..=5 {
        run(&mut engine, nick, &format!("pub capped {i}"));
        ops.extend(engine.take_module_ops());
    }
    let store = InMemoryStore::new();
    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
    let engine2 = reboot_from_store(&store);
    let public = state(&engine2).channel_by_name("Public").unwrap().id;
    let ring: Vec<String> = state(&engine2)
        .rings
        .get(&public)
        .unwrap()
        .iter()
        .map(|m| m.text.clone())
        .collect();
    assert_eq!(ring, vec!["capped 3", "capped 4", "capped 5"]);
    assert_eq!(state(&engine), state(&engine2));
}

#[test]
fn boot_drops_orphaned_membership_docs() {
    // Hand-build boot data with a membership for a player that no longer
    // exists and one for a channel that no longer exists.
    let chan = serde_json::json!({
        "id": 0, "name": "Public", "description": "", "seq": 4,
        "flags": {"loud": true, "logged": true},
        "history": {"persist": true, "cap": 200}
    });
    let good = serde_json::json!({
        "player": 2, "channel": 0, "alias": "pub", "last_read": 4
    });
    let ghost_player = serde_json::json!({
        "player": 99, "channel": 0, "alias": "pub"
    });
    let ghost_chan = serde_json::json!({
        "player": 2, "channel": 7, "alias": "x"
    });
    let boot_data = ModuleBootData {
        kv: vec![
            (SmolStr::new("chan:0"), chan),
            (SmolStr::new("member:2:0"), good),
            (SmolStr::new("member:99:0"), ghost_player),
            (SmolStr::new("member:2:7"), ghost_chan),
            (SmolStr::new("meta"), serde_json::json!({"next_chan_id": 1})),
        ],
        streams: HashMap::new(),
        ..Default::default()
    };
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, vec![Box::new(ComsysModule::new())]).unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(
            HashMap::from([(SmolStr::new(NS), boot_data)]),
            &mut out,
            &mut fx,
        )
        .unwrap();
    let st = state(&engine);
    assert_eq!(st.channels.len(), 1, "not a first boot: no re-seed");
    assert_eq!(st.members.len(), 1);
    assert!(st.members.contains_key(&(nick, ChannelId(0))));
    assert_eq!(st.next_chan_id, 1);
    // The orphans' docs were queued for deletion.
    let dels: Vec<String> = engine
        .take_module_ops()
        .iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::Del { key } => Some(key.to_string()),
            _ => None,
        })
        .collect();
    assert!(dels.contains(&"member:99:0".to_string()));
    assert!(dels.contains(&"member:2:7".to_string()));
}

// ----------------------------------------------------------------------
// Review-fix regressions: prefix immunity, corrupt-doc boot, no-reseed,
// linkdead cursors, recall see lock, @ignore on channels
// ----------------------------------------------------------------------

/// `addcom say=Public` re-aliases the channel onto the verb `say`. A
/// *typed* `say hi` then routes to the channel (typed alias wins, per
/// treaty 5) — but `"hi` stays private room speech: prefix-expanded lines
/// never reach the alias stage.
#[test]
fn prefix_expanded_speech_never_routes_to_a_channel() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, nick, "addcom say=Public");
    // Typed verb: captured by the channel alias.
    let out = run(&mut engine, nick, "say we kill the prince at dawn");
    assert!(
        !chan_events(&out).is_empty(),
        "typed alias speaks on channel"
    );
    // Prefix form: an ordinary room say — NO channel frame.
    let out = run(&mut engine, nick, "\"we kill the prince at dawn");
    assert!(
        chan_events(&out).is_empty(),
        "prefix speech must never broadcast on a channel"
    );
    assert!(out.events().iter().any(|(_, e)| matches!(
        e,
        OutputEvent::Narrative { kind: NarrativeKind::Say, text, .. }
            if text.to_plain_text() == "we kill the prince at dawn"
    )));
    // Pose and semipose prefixes likewise.
    for line in [":whispers.", ";grins."] {
        let out = run(&mut engine, nick, line);
        assert!(chan_events(&out).is_empty(), "{line} leaked to a channel");
    }
}

/// A corrupt CHANNEL document is skipped, not fatal — and its memberships
/// are still not deleted as "orphans".
///
/// Both halves matter and they used to be one: the boot failed *in order to*
/// avoid the destructive cleanup. WS01-S3B separates them, because the way to
/// write a corrupt channel document is to type it (`@channel/perm` puts a
/// player's lock tree in there), and one such document must not take the
/// whole channel system off the game.
#[test]
fn corrupt_channel_doc_is_skipped_and_preserves_memberships() {
    let member = serde_json::json!({
        "player": 2, "channel": 0, "alias": "pub", "last_read": 4
    });
    let boot_data = ModuleBootData {
        kv: vec![
            (SmolStr::new("chan:0"), serde_json::json!({"rot": true})),
            (SmolStr::new("member:2:0"), member),
            (SmolStr::new("meta"), serde_json::json!({"next_chan_id": 1})),
        ],
        streams: HashMap::new(),
        ..Default::default()
    };
    let mut world = World::new_minimal();
    world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, vec![Box::new(ComsysModule::new())]).unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(
            HashMap::from([(SmolStr::new(NS), boot_data)]),
            &mut out,
            &mut fx,
        )
        .expect("a corrupt channel doc must not fail the module boot");
    // The channel is absent from RAM, and this is not mistaken for a first
    // boot (which would re-seed Public and force-join everyone).
    let st = state(&engine);
    assert!(
        st.channels.is_empty(),
        "the unreadable channel is not loaded"
    );
    assert!(st.members.is_empty(), "and neither is its membership");
    // Crucially: NO destructive cleanup was queued — the membership doc
    // survives in the store for the repaired boot.
    assert!(
        !engine.take_module_ops().iter().any(|op| matches!(
            &op.op,
            ModuleWrite::Del { key } if key.starts_with("member:")
        )),
        "corrupt-channel recovery must never delete membership docs"
    );
}

#[test]
fn persisted_history_cap_boundaries_are_safe_stream_requests() {
    let module = ComsysModule::new();
    for cap in [1, MAX_HISTORY_CAP] {
        let mut channel = Channel::new(ChannelId(7), "Boundary");
        channel.history.cap = cap;
        let kv = vec![(
            SmolStr::new("chan:7"),
            serde_json::to_value(channel).expect("channel serializes"),
        )];
        assert_eq!(
            module.boot_stream_requests(&kv).unwrap(),
            vec![(SmolStr::new("hist:7"), cap)]
        );
    }
}

#[test]
fn unsafe_persisted_history_cap_fails_before_stream_request_and_at_module_boot() {
    let module = ComsysModule::new();
    for cap in [0, MAX_HISTORY_CAP + 1] {
        let mut channel = Channel::new(ChannelId(7), "Unsafe");
        channel.history.cap = cap;
        let value = serde_json::to_value(&channel).expect("channel serializes");
        let kv = vec![(SmolStr::new("chan:7"), value.clone())];
        let err = module
            .boot_stream_requests(&kv)
            .expect_err("unsafe cap must not produce a store request");
        assert!(
            err.to_string()
                .contains(&format!("outside 1-{MAX_HISTORY_CAP}")),
            "{err}"
        );

        // Defend the direct/in-process module boot seam too: callers that
        // already assembled ModuleBootData cannot bypass the same bound.
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(ComsysModule::new())],
        )
        .unwrap();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        let err = engine
            .modules_loaded(
                HashMap::from([(
                    SmolStr::new(NS),
                    ModuleBootData {
                        kv: vec![(SmolStr::new("chan:7"), value)],
                        streams: HashMap::new(),
                        ..Default::default()
                    },
                )]),
                &mut out,
                &mut fx,
            )
            .expect_err("unsafe cap must fail direct module boot");
        assert!(
            err.to_string()
                .contains(&format!("outside 1-{MAX_HISTORY_CAP}")),
            "{err}"
        );
    }

    let too_wide = vec![(
        SmolStr::new("chan:8"),
        serde_json::json!({"history": {"cap": u64::from(u32::MAX) + 1}}),
    )];
    assert!(
        module.boot_stream_requests(&too_wide).is_err(),
        "a cap that cannot fit the store API's u32 count must fail closed"
    );
}

/// Staff destroying the LAST channel must not resurrect Public on the next
/// boot: first-boot seeding keys on a completely empty namespace, and the
/// meta doc survives the destroy.
#[test]
fn destroying_last_channel_does_not_reseed_on_reboot() {
    let (mut engine, _nick, _ann) = boot();
    let mut ops: Vec<ModuleOp> = engine.take_module_ops();
    run(&mut engine, GOD, "@channel/destroy Public");
    ops.extend(engine.take_module_ops());
    let store = InMemoryStore::new();
    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
    let engine2 = reboot_from_store(&store);
    assert!(
        state(&engine2).channels.is_empty(),
        "reboot after destroying the last channel must NOT reseed Public"
    );
    assert!(state(&engine2).members.is_empty());
}

/// A link-dead member (world-connected, zero live connections) receives
/// nothing — and their unread cursor must not move, so the messages they
/// missed still count at reconnect.
#[test]
fn linkdead_member_keeps_unread_cursor() {
    let (mut engine, nick, ann) = setup();
    // Ann goes link-dead: still connected world-side, no live conns.
    engine.world.set_live_conns(ann, 0);
    let out = run(&mut engine, nick, "pub anyone there?");
    assert_eq!(
        recipients(&out),
        vec![GOD, nick],
        "a link-dead member receives nothing"
    );
    let public = state(&engine).channel_by_name(DEFAULT_CHANNEL).unwrap().id;
    assert_eq!(
        state(&engine)
            .members
            .get(&(ann, public))
            .unwrap()
            .last_read,
        0,
        "an undelivered message must stay unread"
    );
    // Back live: delivery (and the cursor) resumes.
    engine.world.set_live_conns(ann, 1);
    let out = run(&mut engine, nick, "pub welcome back");
    assert_eq!(recipients(&out), vec![GOD, nick, ann]);
    assert_eq!(
        state(&engine)
            .members
            .get(&(ann, public))
            .unwrap()
            .last_read,
        2
    );
}

/// `@channel/recall` is delivery: the see lock gates it with no admin
/// bypass — a member barred from hearing the channel cannot read its
/// retained history either.
#[test]
fn recall_honors_the_see_lock() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, nick, "pub before the lock");
    run(&mut engine, GOD, "@channel/perm Public=see=flag^WIZARD");
    let out = run(&mut engine, nick, "@channel/recall pub");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("You cannot listen to Public.")),
        "got: {:?}",
        sys_texts(&out)
    );
    // Someone passing the lock still recalls.
    let out = run(&mut engine, GOD, "@channel/recall pub");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("before the lock"))
    );
}

/// `@ignore` applies to channel traffic: an ignored speaker's messages and
/// join/leave notices are filtered for the ignoring member only.
#[test]
fn ignore_filters_channel_messages_and_notices() {
    let (mut engine, nick, ann) = setup();
    let out = run(&mut engine, nick, "@ignore Ann");
    assert!(
        sys_texts(&out).iter().any(|t| t.contains("Ann")),
        "got: {:?}",
        sys_texts(&out)
    );
    // Ann speaks: Nick's frame is filtered, others still hear.
    let out = run(&mut engine, ann, "pub am I muted?");
    assert_eq!(
        recipients(&out),
        vec![GOD, ann],
        "the ignoring member must not receive the ignored speaker"
    );
    // Nick's cursor did not advance for a message never delivered to him.
    let public = state(&engine).channel_by_name(DEFAULT_CHANNEL).unwrap().id;
    assert_eq!(
        state(&engine)
            .members
            .get(&(nick, public))
            .unwrap()
            .last_read,
        0
    );
    // Ann's leave/join notices skip Nick too.
    let out = run(&mut engine, ann, "delcom pub");
    assert!(
        !out.events().iter().any(|(aud, e)| matches!(
            (aud, e),
            (Audience::Actor(a), OutputEvent::Sys(t))
                if *a == nick && t.to_plain_text().contains("has left this channel")
        )),
        "join/leave notices about an ignored player must be filtered"
    );
    assert!(
        out.events().iter().any(|(aud, e)| matches!(
            (aud, e),
            (Audience::Actor(a), OutputEvent::Sys(t))
                if *a == GOD && t.to_plain_text().contains("has left this channel")
        )),
        "other members still see the notice"
    );
}

// ----------------------------------------------------------------------
// Web query handlers (M4): comsys.state / comsys.backlog via Inbound::Query
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
    assert_eq!(aud, Audience::Conn(77), "replies are conn-scoped");
    let OutputEvent::Reply { id: 5, event } = ev else {
        panic!("expected Reply id 5, got {ev:?}");
    };
    *event
}

fn panel_data(ev: OutputEvent, want_feed: &str) -> serde_json::Value {
    let OutputEvent::Panel { feed, data, .. } = ev else {
        panic!("expected Panel {want_feed}, got {ev:?}");
    };
    assert_eq!(feed, want_feed);
    data
}

/// `comsys.state` reports joined channels with alias, colors (web hex
/// included), and the unread count derived from seq vs. last_read; a
/// channel whose see lock the member fails is omitted entirely.
#[test]
fn state_query_reports_channels_colors_and_unread() {
    let (mut engine, nick, ann) = setup();
    // Ann loses her live conns (linkdead): messages stop advancing her
    // cursor, so unread grows.
    engine.world.set_live_conns(ann, 0);
    run(&mut engine, nick, "pub one");
    run(&mut engine, nick, "pub two");

    let data = panel_data(
        web_query(&mut engine, ann, "comsys.state", serde_json::json!({})),
        "comsys",
    );
    let chans = data["channels"].as_array().unwrap();
    let public = chans
        .iter()
        .find(|c| c["name"] == "Public")
        .expect("Public listed");
    assert_eq!(public["alias"], "pub");
    assert_eq!(public["color"]["ansi"], "cyan");
    assert_eq!(public["color"]["web_hex"], "#00ffff");
    assert_eq!(public["seq"], 2);
    assert_eq!(public["last_read"], 0);
    assert_eq!(public["unread"], 2);
    assert_eq!(public["gagged"], false);
    assert_eq!(public["loud"], true);
    assert_eq!(public["logged"], true);
    assert_eq!(public["ic"], false, "OOC unless staff say otherwise");

    // See-locked away: the channel vanishes from the member's state.
    run(&mut engine, GOD, "@channel/perm Public=see=flag^WIZARD");
    let data = panel_data(
        web_query(&mut engine, ann, "comsys.state", serde_json::json!({})),
        "comsys",
    );
    assert!(
        data["channels"].as_array().unwrap().is_empty(),
        "see-locked channels must not appear: {data:?}"
    );
}

/// `comsys.backlog` returns the newest-first slice with `before_seq`
/// paging, resolves by channel name or personal alias, and requires
/// membership.
#[test]
fn backlog_query_slices_newest_first() {
    let (mut engine, nick, _) = setup();
    for i in 1..=5 {
        run(&mut engine, nick, &format!("pub msg-{i}"));
    }
    let data = panel_data(
        web_query(
            &mut engine,
            nick,
            "comsys.backlog",
            serde_json::json!({"channel": "Public", "limit": 2}),
        ),
        "comsys",
    );
    let texts: Vec<&str> = data["backlog"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["msg-5", "msg-4"], "newest first");
    assert_eq!(data["backlog"]["channel"], "Public");

    // Paging: strictly below before_seq, resolved via the personal alias.
    let data = panel_data(
        web_query(
            &mut engine,
            nick,
            "comsys.backlog",
            serde_json::json!({"channel": "pub", "before_seq": 4, "limit": 10}),
        ),
        "comsys",
    );
    let seqs: Vec<u64> = data["backlog"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs, [3, 2, 1]);
    // Message shape matches the live Channel frame fields.
    let top = &data["backlog"]["messages"][0];
    assert_eq!(top["actor"]["name"], "Nick");
    assert_eq!(top["kind"], "say");
    assert!(top["ts"].as_i64().unwrap() > 0);

    // Not a member (God delcom'd? no — use a channel Nick never joined).
    run(&mut engine, GOD, "@channel/create Private");
    let ev = web_query(
        &mut engine,
        nick,
        "comsys.backlog",
        serde_json::json!({"channel": "Private", "limit": 5}),
    );
    let OutputEvent::Sys(t) = ev else {
        panic!("expected Sys refusal, got {ev:?}");
    };
    assert!(t.to_plain_text().contains("You are not on that channel."));
}

/// The see lock gates backlog hydration exactly like recall and live
/// delivery — no admin bypass for plain members.
#[test]
fn backlog_query_honors_the_see_lock() {
    let (mut engine, nick, _) = setup();
    run(&mut engine, nick, "pub before the lock");
    run(&mut engine, GOD, "@channel/perm Public=see=flag^WIZARD");
    let ev = web_query(
        &mut engine,
        nick,
        "comsys.backlog",
        serde_json::json!({"channel": "Public", "limit": 10}),
    );
    let OutputEvent::Sys(t) = ev else {
        panic!("expected Sys refusal, got {ev:?}");
    };
    assert_eq!(t.to_plain_text(), "You cannot listen to Public.");

    // A member passing the lock still hydrates.
    let data = panel_data(
        web_query(
            &mut engine,
            GOD,
            "comsys.backlog",
            serde_json::json!({"channel": "Public", "limit": 10}),
        ),
        "comsys",
    );
    let texts: Vec<&str> = data["backlog"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["before the lock"]);
}

/// The IC flag (interface plan I7): `@channel/set <c>=ic` marks a channel
/// in-character, `clist` shows it as `I`, `comsys.state` reports it, and
/// `!ic` clears it. Channel admins only.
#[test]
fn ic_flag_sets_lists_and_reports() {
    let (mut engine, nick, _) = setup();
    let out = run(&mut engine, nick, "@channel/set Public=ic");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Permission denied")),
        "a member is not a channel admin: {out:?}"
    );
    run(&mut engine, GOD, "@channel/set Public=ic");
    assert!(state(&engine).channel_by_name("Public").unwrap().flags.ic);

    let out = run(&mut engine, nick, "clist");
    let listing = sys_texts(&out).join("\n");
    assert!(listing.contains('I'), "clist shows the I flag: {listing}");

    let data = panel_data(
        web_query(&mut engine, nick, "comsys.state", serde_json::json!({})),
        "comsys",
    );
    let public = data["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Public")
        .expect("Public listed");
    assert_eq!(public["ic"], true);

    run(&mut engine, GOD, "@channel/set Public=!ic");
    assert!(!state(&engine).channel_by_name("Public").unwrap().flags.ic);
    let out = run(&mut engine, GOD, "@channel/set Public=bogus");
    assert!(
        sys_texts(&out).iter().any(|t| t.contains("|ic")),
        "usage names the flag: {out:?}"
    );
}

// ---- Interface plan I11: the channel-mention producer ---------------------

/// The `mention` notices in `out`, as `(to, text, prefill)`.
fn mention_notices(out: &Outbox) -> Vec<(ObjId, String, Option<String>)> {
    out.events()
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (
                Audience::Actor(a),
                OutputEvent::Notice {
                    category,
                    text,
                    action,
                    ..
                },
            ) if category.as_str() == "mention" => Some((
                *a,
                text.to_plain_text(),
                action.as_ref().map(|x| x.prefill.clone()),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn a_channel_line_naming_a_member_lands_a_mention_notice_on_them_alone() {
    let (mut engine, nick, ann) = setup();
    // Ann is named as a whole word, case-insensitively: she alone is notified,
    // with her own alias for the channel ready as the reply.
    let out = run(&mut engine, nick, "pub hey ANN, look at this");
    let got = mention_notices(&out);
    assert_eq!(got.len(), 1, "{got:?}");
    let (to, text, prefill) = &got[0];
    assert_eq!(*to, ann);
    assert_eq!(text, "Nick on pub: hey ANN, look at this");
    assert_eq!(prefill.as_deref(), Some("pub "));
    // A line naming nobody notifies nobody; a speaker naming themselves is
    // never their own mention; a substring ("Annex") is not a whole-word name.
    assert!(mention_notices(&run(&mut engine, nick, "pub nothing to see")).is_empty());
    assert!(mention_notices(&run(&mut engine, nick, "pub Nick was here")).is_empty());
    assert!(mention_notices(&run(&mut engine, nick, "pub the Annex is closed")).is_empty());
}

/// Programme D4: a channel flagged `relay` publishes each spoken line to the
/// named relay sink exactly once (beside the members' copies); an unflagged
/// channel never does; a line that *came in* from the relay is spoken to the
/// members but never mirrored back (the loop guard) and carries its origin in
/// history; and the relay speaker refuses exactly what the alias stage would.
#[test]
fn d4_relay_flag_mirrors_lines_once_and_the_relay_speaker_keeps_the_gates() {
    use mush_core::RelayRefusal;
    use mush_core::output::Audience;
    use mush_proto::NarrativeKind;

    fn relay_sends(out: &Outbox) -> Vec<(String, String)> {
        out.events()
            .iter()
            .filter_map(|(aud, e)| match (aud, e) {
                (Audience::Relay(name), OutputEvent::Channel { text, .. }) => {
                    Some((name.to_string(), text.to_plain_text()))
                }
                _ => None,
            })
            .collect()
    }
    fn member_copies(out: &Outbox) -> usize {
        out.events()
            .iter()
            .filter(|(aud, e)| {
                matches!(aud, Audience::Actor(_)) && matches!(e, OutputEvent::Channel { .. })
            })
            .count()
    }

    let (mut engine, nick, _ann) = setup();
    // Unflagged: members hear it, the relay does not.
    let out = run(&mut engine, nick, "pub hello");
    assert!(member_copies(&out) >= 1);
    assert!(
        relay_sends(&out).is_empty(),
        "an unflagged channel is never relayed"
    );
    // Staff flag it: one relay copy per line, beside the members' copies.
    let out = run(&mut engine, GOD, "@channel/set Public=relay");
    assert!(
        sys_texts(&out).iter().any(|t| t.contains("relay set")),
        "{:?}",
        sys_texts(&out)
    );
    assert!(
        state(&engine)
            .channel_by_name("Public")
            .unwrap()
            .flags
            .relay
    );
    let out = run(&mut engine, nick, "pub hello again");
    assert_eq!(
        relay_sends(&out),
        vec![("Public".to_string(), "hello again".to_string())]
    );
    assert!(member_copies(&out) >= 1);

    // The relay speaker: spoken to members, origin marked, never mirrored back.
    let (mut out, _fx) = (Outbox::new(), Effects::new());
    assert_eq!(
        engine.relay_speak(
            nick,
            "Public",
            NarrativeKind::Say,
            "  from discord  ",
            &mut out
        ),
        Ok(())
    );
    assert!(member_copies(&out) >= 1, "members hear a relayed line");
    assert!(
        relay_sends(&out).is_empty(),
        "a relayed line is never mirrored back out"
    );
    let public = state(&engine).channel_by_name("Public").unwrap().id;
    let last = state(&engine).rings[&public]
        .back()
        .cloned()
        .expect("logged");
    assert_eq!(last.text, "from discord");
    assert_eq!(last.origin.as_deref(), Some("discord"));
    let mut sink = Outbox::new();
    // Refusals mirror the alias stage's.
    assert_eq!(
        engine.relay_speak(nick, "Nowhere", NarrativeKind::Say, "x", &mut sink),
        Err(RelayRefusal::NoSuchChannel)
    );
    assert_eq!(
        engine.relay_speak(nick, "Public", NarrativeKind::Say, "   ", &mut sink),
        Err(RelayRefusal::Empty)
    );
    run(&mut engine, nick, "@channel/gag pub");
    assert_eq!(
        engine.relay_speak(nick, "Public", NarrativeKind::Say, "muffled", &mut sink),
        Err(RelayRefusal::Gagged)
    );
    run(&mut engine, nick, "@channel/ungag pub");
    run(&mut engine, GOD, "@channel/create Private");
    run(&mut engine, GOD, "@channel/set Private=relay");
    assert_eq!(
        engine.relay_speak(nick, "Private", NarrativeKind::Say, "psst", &mut sink),
        Err(RelayRefusal::NotMember)
    );
    run(&mut engine, GOD, "@channel/set Public=!relay");
    assert_eq!(
        engine.relay_speak(nick, "Public", NarrativeKind::Say, "quiet now", &mut sink),
        Err(RelayRefusal::NotRelay)
    );
    let out = run(&mut engine, nick, "pub unflagged again");
    assert!(relay_sends(&out).is_empty());
}

/// Look-back LB-25 (I7): the `ic` flag survives a restart — it round-trips
/// through the persisted JSON, and a channel document written before I7 (no
/// `ic` key) reads back as OOC, which is what `#[serde(default)]` promises and
/// nothing pinned.
#[test]
fn the_ic_flag_round_trips_and_a_pre_i7_document_reads_as_ooc() {
    let flags = mush_comsys::ChannelFlags {
        ic: true,
        ..mush_comsys::ChannelFlags::default()
    };
    let json = serde_json::to_string(&flags).unwrap();
    assert!(json.contains("\"ic\":true"), "{json}");
    assert_eq!(
        serde_json::from_str::<mush_comsys::ChannelFlags>(&json).unwrap(),
        flags
    );
    let legacy: mush_comsys::ChannelFlags = serde_json::from_str("{}").unwrap();
    assert!(!legacy.ic, "a document without the key is OOC");
    assert_eq!(legacy, mush_comsys::ChannelFlags::default());
}

// ----------------------------------------------------------------------
// 16b cut Z.4 (16c N8): the module-driven make-once API and the closed flag
// ----------------------------------------------------------------------

/// Run one closure against a real `Ctx` the way a driving module (tacsim's
/// boot, its lobby) calls comsys's API, and hand back what it emitted and the
/// module writes it deferred.
fn with_api<T>(
    engine: &mut Engine,
    f: impl FnOnce(&mut mush_core::Ctx<'_>) -> T,
) -> (T, Vec<(Audience, OutputEvent)>, Vec<ModuleOp>) {
    let mut out = Outbox::new();
    let answer = {
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: None,
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        f(&mut ctx)
    };
    let ops = out
        .take_deferred()
        .into_iter()
        .filter_map(|d| match d {
            Deferred::Persist(op) => Some(op),
            _ => None,
        })
        .collect();
    (answer, out.take(), ops)
}

/// The keys of every `JsonSet` in `ops`, in order.
fn set_keys(ops: &[ModuleOp]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, .. } => Some(key.to_string()),
            _ => None,
        })
        .collect()
}

/// A side net's locks, as tacsim makes one: every lock shut but speak.
fn net_perms() -> mush_comsys::ChannelPerms {
    mush_comsys::ChannelPerms {
        join: mush_core::LockExpr::Const(false),
        speak: mush_core::LockExpr::Const(true),
        see: mush_core::LockExpr::Const(false),
        admin: mush_core::LockExpr::Const(false),
    }
}

/// A side net's flags: closed, and nothing else.
fn closed() -> mush_comsys::ChannelFlags {
    mush_comsys::ChannelFlags {
        closed: true,
        ..mush_comsys::ChannelFlags::default()
    }
}

/// The plain lines one actor was sent.
fn lines_to(events: &[(Audience, OutputEvent)], who: ObjId) -> Vec<String> {
    events
        .iter()
        .filter(|(aud, _)| *aud == Audience::Actor(who))
        .map(|(_, e)| mush_testkit::text_of(e))
        .collect()
}

/// `ensure_channel` twice makes one channel, with one `chan:` document and one
/// meta write between the two calls; a third call after `@channel/perm` opened
/// the join lock shuts it again with one channel write and nothing else, and
/// keeps what the module does not own (the colour a staffer set).
#[test]
fn z4_ensure_channel_makes_one_channel_once_and_reasserts_its_locks() {
    let (mut engine, _, _) = setup();
    let (first, _, ops_one) = with_api(&mut engine, |ctx| {
        mush_comsys::api::ensure_channel(ctx, "RedNet", "The red net.", closed(), net_perms())
    });
    let (second, _, ops_two) = with_api(&mut engine, |ctx| {
        mush_comsys::api::ensure_channel(ctx, "rednet", "Another text.", closed(), net_perms())
    });
    let id = first.expect("a usable name on a composed comsys makes a channel");
    assert_eq!(
        second,
        Some(id),
        "the same channel, by name, case-insensitively"
    );
    let keys: Vec<String> = set_keys(&ops_one)
        .into_iter()
        .chain(set_keys(&ops_two))
        .collect();
    assert_eq!(
        keys,
        vec![format!("chan:{}", id.0), "meta".to_string()],
        "one channel document and one meta write across both calls"
    );
    let st = state(&engine);
    assert_eq!(st.channels.len(), 2, "Public and the one net");
    let c = st.channels.get(&id).expect("the net");
    assert_eq!(c.name, "RedNet");
    assert_eq!(
        c.description, "The red net.",
        "the description is the first call's"
    );
    assert!(c.flags.closed);

    // Staff open the join lock and recolour it; the next boot's call shuts the
    // lock again and leaves the colour.
    run(&mut engine, GOD, "@channel/perm RedNet=join=#1");
    run(&mut engine, GOD, "@channel/color RedNet=red,#ff0000");
    let _ = engine.take_module_ops();
    assert_ne!(
        state(&engine)
            .channels
            .get(&id)
            .map(|c| c.perms.join.clone()),
        Some(mush_core::LockExpr::Const(false)),
        "the staffer opened it"
    );
    let (third, _, ops_three) = with_api(&mut engine, |ctx| {
        mush_comsys::api::ensure_channel(ctx, "RedNet", "The red net.", closed(), net_perms())
    });
    assert_eq!(third, Some(id));
    assert_eq!(set_keys(&ops_three), vec![format!("chan:{}", id.0)]);
    let c = state(&engine).channels.get(&id).expect("the net").clone();
    assert_eq!(c.perms, net_perms(), "the module's locks, re-asserted");
    assert_eq!(c.color.ansi, "red", "a recoloured net keeps its colour");
}

/// A name `@channel/create` would refuse is refused here too, and nothing is
/// written.
#[test]
fn z4_ensure_channel_answers_none_for_an_unusable_name_and_writes_nothing() {
    let (mut engine, _, _) = setup();
    let long = "N".repeat(33);
    for bad in ["", "Red Net", "Net!", long.as_str()] {
        let (answer, _, ops) = with_api(&mut engine, |ctx| {
            mush_comsys::api::ensure_channel(ctx, bad, "x", closed(), net_perms())
        });
        assert_eq!(answer, None, "{bad:?}");
        assert!(ops.is_empty(), "{bad:?} wrote {ops:?}");
    }
    assert_eq!(state(&engine).channels.len(), 1, "only Public");
}

/// `set_title` persists the membership, the next line carries the title, and it
/// refuses a non-member and an over-long title.
#[test]
fn z4_set_title_persists_and_the_next_line_carries_it() {
    let (mut engine, nick, ann) = setup();
    let (id, _, _) = with_api(&mut engine, |ctx| {
        let id = mush_comsys::api::ensure_channel(
            ctx,
            "Ops",
            "",
            mush_comsys::ChannelFlags::default(),
            mush_comsys::ChannelPerms::default(),
        )
        .expect("made");
        assert!(mush_comsys::api::force_join(ctx, nick, id, "ops"));
        id
    });
    let (set, _, ops) = with_api(&mut engine, |ctx| {
        mush_comsys::api::set_title(ctx, nick, id, Some("RA"))
    });
    assert!(set);
    let member_key = format!("member:{}:{}", nick.0, id.0);
    let doc = ops
        .iter()
        .find_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, value } if key.as_str() == member_key => {
                Some(value.clone())
            }
            _ => None,
        })
        .expect("the membership document is written");
    assert_eq!(doc["title"], "RA");
    // The same title again writes nothing.
    let (again, _, ops) = with_api(&mut engine, |ctx| {
        mush_comsys::api::set_title(ctx, nick, id, Some("RA"))
    });
    assert!(again);
    assert!(ops.is_empty(), "{ops:?}");

    let out = run(&mut engine, nick, "ops contact east");
    let evs = chan_events(&out);
    assert_eq!(evs.len(), 1, "{evs:?}");
    assert_eq!(evs[0].title.as_deref(), Some("RA"));
    assert_eq!(evs[0].text, "contact east");

    // Not a member; too long.
    let (refused, _, ops) = with_api(&mut engine, |ctx| {
        mush_comsys::api::set_title(ctx, ann, id, Some("BA"))
    });
    assert!(!refused);
    assert!(ops.is_empty());
    // One past `comtitle`'s sixty characters.
    let long = "x".repeat(61);
    let (refused, _, ops) = with_api(&mut engine, |ctx| {
        mush_comsys::api::set_title(ctx, nick, id, Some(&long))
    });
    assert!(!refused);
    assert!(ops.is_empty());
    assert_eq!(
        state(&engine)
            .members
            .get(&(nick, id))
            .and_then(|m| m.title.clone())
            .as_deref(),
        Some("RA")
    );
}

/// A `#false` join lock refuses `addcom` — even on an open channel a player can
/// see — while the module's forced join still joins.
#[test]
fn z4_a_shut_join_lock_refuses_addcom_while_force_join_joins() {
    let (mut engine, nick, ann) = setup();
    let perms = mush_comsys::ChannelPerms {
        join: mush_core::LockExpr::Const(false),
        ..mush_comsys::ChannelPerms::default()
    };
    let (id, _, _) = with_api(&mut engine, |ctx| {
        mush_comsys::api::ensure_channel(
            ctx,
            "Shut",
            "",
            mush_comsys::ChannelFlags::default(),
            perms,
        )
    });
    let id = id.expect("made");
    let out = run(&mut engine, nick, "addcom sh=Shut");
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Channel Shut does not admit you.")),
        "{:?}",
        sys_texts(&out)
    );
    assert!(!state(&engine).members.contains_key(&(nick, id)));
    let (joined, events, _) = with_api(&mut engine, |ctx| {
        mush_comsys::api::force_join(ctx, ann, id, "sh")
    });
    assert!(joined);
    assert!(state(&engine).members.contains_key(&(ann, id)));
    assert_eq!(
        lines_to(&events, ann),
        vec!["You have been joined to channel Shut (alias: sh).".to_string()]
    );
}

/// When the alias asked for already names another channel of the member's, the
/// join line says what they got and what the one they asked for means.
#[test]
fn z4_a_forced_join_says_what_an_alias_it_could_not_take_already_means() {
    let (mut engine, nick, _) = setup();
    let (_, events, _) = with_api(&mut engine, |ctx| {
        let id = mush_comsys::api::ensure_channel(ctx, "RedNet", "", closed(), net_perms())
            .expect("made");
        assert!(mush_comsys::api::force_join(ctx, nick, id, "pub"));
    });
    assert_eq!(
        lines_to(&events, nick),
        vec![
            "You have been joined to channel RedNet (alias: pub2; pub is your alias for Public)."
                .to_string()
        ]
    );
}

/// **A closed channel is its members' alone**: with a `#false` see lock it is
/// delivered to its members, is absent from a non-member's `clist` and present
/// (flag `C`) in a member's, `@channel/who` and `addcom` answer `No such
/// channel.` to a non-member and the roster to a member, and a member's web
/// hydration lists it; `members_of` reads the roster.
#[test]
fn z4_a_closed_channel_is_delivered_to_its_members_and_is_nothing_to_anybody_else() {
    let (mut engine, nick, ann) = setup();
    let (id, _, _) = with_api(&mut engine, |ctx| {
        let id =
            mush_comsys::api::ensure_channel(ctx, "RedNet", "The red net.", closed(), net_perms())
                .expect("made");
        assert!(mush_comsys::api::force_join(ctx, nick, id, "ra"));
        id
    });
    assert_eq!(mush_comsys::api::members_of(&engine.world, id), vec![nick]);

    // Delivery: the see lock is shut, and the member still hears it.
    let out = run(&mut engine, nick, "ra contact east");
    assert_eq!(
        recipients(&out),
        vec![nick],
        "the member alone; God is not on it"
    );

    // clist: absent for Ann, present for Nick with the closed letter.
    let out = run(&mut engine, ann, "clist");
    assert!(
        !sys_texts(&out).iter().any(|t| t.contains("RedNet")),
        "{:?}",
        sys_texts(&out)
    );
    let out = run(&mut engine, nick, "clist");
    let row = sys_texts(&out)
        .into_iter()
        .find(|t| t.contains("RedNet"))
        .expect("the member's clist lists the net");
    assert!(row.contains(" C "), "the closed letter: {row}");

    // who and addcom: no such channel to Ann, the roster to Nick.
    for line in ["@channel/who RedNet", "addcom rn=RedNet"] {
        let out = run(&mut engine, ann, line);
        assert_eq!(
            sys_texts(&out),
            vec!["No such channel.".to_string()],
            "{line}"
        );
    }
    let out = run(&mut engine, nick, "@channel/who RedNet");
    assert_eq!(
        sys_texts(&out),
        vec![
            "-- Channel RedNet: 1 member --".to_string(),
            "  Nick [connected]".to_string()
        ]
    );

    // The member's channel tabs hydrate it; the non-member's do not.
    let data = panel_data(
        web_query(&mut engine, nick, "comsys.state", serde_json::json!({})),
        "comsys",
    );
    assert!(
        data["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "RedNet" && c["alias"] == "ra"),
        "{data:?}"
    );
    let data = panel_data(
        web_query(&mut engine, ann, "comsys.state", serde_json::json!({})),
        "comsys",
    );
    assert!(
        !data["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "RedNet"),
        "{data:?}"
    );

    // A second member: both hear it, and the roster reads both.
    with_api(&mut engine, |ctx| {
        assert!(mush_comsys::api::force_join(ctx, ann, id, "ra"));
    });
    let out = run(&mut engine, ann, "ra copy");
    assert_eq!(recipients(&out), vec![nick, ann]);
    assert_eq!(
        mush_comsys::api::members_of(&engine.world, id),
        vec![nick, ann]
    );
    let (left, events, _) = with_api(&mut engine, |ctx| {
        mush_comsys::api::force_leave(ctx, nick, id)
    });
    assert!(left);
    assert_eq!(
        lines_to(&events, nick),
        vec!["You have been removed from channel RedNet.".to_string()]
    );
    assert_eq!(mush_comsys::api::members_of(&engine.world, id), vec![ann]);
}

/// **A closed channel's notices and LOUD count ignore its see lock too** (16b
/// cut Z.4): the brief names every per-member read of delivery, not only live
/// speech. With a `#false` see lock, an admin's `addcom` (which passes the shut
/// join lock) and their `delcom` still tell the members, and a member who was
/// away is counted what they missed at login.
#[test]
fn z4_a_closed_channels_notices_and_loud_count_ignore_its_see_lock() {
    let (mut engine, nick, ann) = setup();
    let loud_closed = mush_comsys::ChannelFlags {
        loud: true,
        ..closed()
    };
    let (id, _, _) = with_api(&mut engine, |ctx| {
        let id = mush_comsys::api::ensure_channel(ctx, "RedNet", "", loud_closed, net_perms())
            .expect("made");
        assert!(mush_comsys::api::force_join(ctx, nick, id, "ra"));
        assert!(mush_comsys::api::force_join(ctx, ann, id, "ra"));
        id
    });

    // The join and leave notices reach the members.
    let out = run(&mut engine, GOD, "addcom rn=RedNet");
    assert!(state(&engine).members.contains_key(&(GOD, id)));
    for who in [nick, ann] {
        assert_eq!(
            lines_to(out.events(), who),
            vec!["[RedNet] God has joined this channel.".to_string()],
            "{who:?}"
        );
    }
    let out = run(&mut engine, GOD, "delcom rn");
    assert!(!state(&engine).members.contains_key(&(GOD, id)));
    for who in [nick, ann] {
        assert_eq!(
            lines_to(out.events(), who),
            vec!["[RedNet] God has left this channel.".to_string()],
            "{who:?}"
        );
    }

    // Ann is away while Nick speaks twice; at login she is told the count.
    engine.world.set_connected(ann, false);
    run(&mut engine, nick, "ra missed one");
    run(&mut engine, nick, "ra missed two");
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::ConnUp {
            conn: 60,
            actor: ann,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t == "[RedNet] 2 messages since you last read."),
        "got: {:?}",
        sys_texts(&out)
    );
}
