//! The comsys half of the deep-lock defect (WS01-S3B).
//!
//! `@channel/perm` is a lane from player text straight into a persisted
//! document: it parses a lock expression into [`ChannelPerms`] and mirrors
//! the whole channel through the **module-op** lane, which the object lane's
//! guards never see. Two fences hold that lane, one at each end.
//!
//! **In**: `parse_lock` bounds the height of the tree it will build, so an
//! over-deep key is refused at the one gate where a player can be told why,
//! and the `chan:{id}` document stays inside `mush_core::MAX_DOC_DEPTH`.
//!
//! **Out**: a channel document that will not decode is *skipped, counted and
//! logged by key* instead of failing the whole module boot. That used to be
//! the other way round, on the reasoning that a skipped channel's
//! memberships would be swept as orphans and deleted — so the skip holds an
//! unreadable channel's id, its memberships are left alone in the store, and
//! its id is not handed out again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mush_comsys::{Channel, ChannelId, ComsysModule, ComsysState, NS};
use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::module::ModuleBootData;
use mush_core::{
    GOD, LIMBO, ModuleWrite, ObjId, ObjKind, Outbox, Registry, World, register_core_commands,
    register_engine_commands,
};
use mush_proto::OutputEvent;
use smol_str::SmolStr;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

fn engine_with_players(names: &[&str]) -> (Engine, Vec<ObjId>) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let ids: Vec<ObjId> = names
        .iter()
        .map(|n| {
            world
                .create(ObjKind::Player, n, GOD, Some(LIMBO))
                .expect("create player")
        })
        .collect();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).expect("core commands");
    register_engine_commands(&mut registry).expect("engine commands");
    let engine = Engine::with_modules(world, registry, vec![Box::new(ComsysModule::new())])
        .expect("engine with comsys");
    (engine, ids)
}

/// Boot comsys over `kv`, returning the engine and everything the module
/// logged while doing it.
fn boot_over(
    kv: Vec<(SmolStr, serde_json::Value)>,
    players: &[&str],
) -> (Engine, Vec<ObjId>, String) {
    let (mut engine, ids) = engine_with_players(players);
    let capture = Capture::default();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        engine
            .modules_loaded(
                HashMap::from([(
                    SmolStr::new(NS),
                    ModuleBootData {
                        kv,
                        streams: HashMap::new(),
                        ..Default::default()
                    },
                )]),
                &mut out,
                &mut fx,
            )
            .expect("an undecodable channel doc must not fail the module boot");
    });
    (engine, ids, capture.text())
}

fn state(engine: &Engine) -> &ComsysState {
    engine
        .world
        .module_state::<ComsysState>()
        .expect("comsys state")
}

fn run(engine: &mut Engine, actor: ObjId, line: &str) -> Vec<String> {
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
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            _ => None,
        })
        .collect()
}

/// A `MakeWriter` collecting everything a `fmt` layer writes, so a test can
/// assert on the diagnostic an operator would actually read.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("capture buffer")).into_owned()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("capture buffer")
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// A channel document whose join lock is `levels` deep and, at the bottom,
/// not a lock at all — deep *and* undecodable, which is the pair that
/// matters: depth alone does not stop `serde_json::from_value`, which has no
/// recursion limit of its own (only the text parser does, at 128 levels, and
/// that one fires in the store before this document is ever handed over).
fn undecodable_channel_doc(id: u64, levels: usize) -> serde_json::Value {
    let mut lock = serde_json::json!({ "NotAVariant": null });
    for _ in 0..levels {
        lock = serde_json::json!({ "Not": lock });
    }
    serde_json::json!({
        "id": id, "name": "Rotten", "description": "", "seq": 0,
        "flags": {"loud": false, "no_titles": false, "logged": false, "ic": false, "relay": false},
        "history": {"persist": false, "cap": 200},
        "perms": {"join": lock, "speak": {"Const": true}, "see": {"Const": true},
                  "admin": {"Const": false}},
    })
}

fn sound_channel_doc(id: u64, name: &str) -> serde_json::Value {
    serde_json::to_value(Channel::new(ChannelId(id), name)).expect("a channel serializes")
}

fn membership_doc(player: u32, channel: u64, alias: &str) -> serde_json::Value {
    serde_json::json!({
        "player": player, "channel": channel, "alias": alias, "last_read": 0
    })
}

// ----------------------------------------------------------------------
// The way in: the parse gate
// ----------------------------------------------------------------------

/// (a) A flat `&` chain thousands of terms long fits in one input line and
/// evades any recursion counter, so `parse_lock` bounds the *tree* instead —
/// on all four gates, not just the one somebody remembered. The refusal
/// leaves `ChannelPerms` byte-identical and queues no module write, which is
/// the only reason the persisted document never has to be repaired.
#[test]
fn an_over_deep_lock_is_refused_on_every_channel_gate() {
    let (mut engine, _) = engine_with_players(&["Nick"]);
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .expect("first boot");
    let _ = engine.take_module_ops();

    let before = state(&engine)
        .channel_by_name("Public")
        .expect("first boot seeds Public")
        .clone();
    let key = format!("{}#1", "#1&".repeat(3_000));
    for gate in ["join", "speak", "see", "admin"] {
        let texts = run(
            &mut engine,
            GOD,
            &format!("@channel/perm Public={gate}={key}"),
        );
        assert!(
            texts.iter().any(|t| t.contains("nests too deeply")),
            "the {gate} gate must refuse an over-deep lock and say why: {texts:?}"
        );
        let after = state(&engine)
            .channel_by_name("Public")
            .expect("the channel survives a refusal");
        assert_eq!(
            after.perms, before.perms,
            "a refused {gate} lock must leave every gate untouched"
        );
        assert!(
            engine.take_module_ops().is_empty(),
            "a refused {gate} lock must queue no module write"
        );
    }
}

/// And the reason that gate is load-bearing for the *store*: the deepest key
/// it will accept still serializes a channel document well inside
/// `MAX_DOC_DEPTH`, which is the bound `validate_module_op` now refuses past.
/// If this ever stops holding, a legal lock would start latching integrity
/// on the write path instead of being refused with an explanation here.
#[test]
fn the_deepest_accepted_lock_keeps_the_channel_document_inside_the_store_bound() {
    let (mut engine, _) = engine_with_players(&["Nick"]);
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .expect("first boot");
    let _ = engine.take_module_ops();

    let terms = usize::try_from(mush_core::MAX_LOCK_EXPR_DEPTH).expect("the bound fits");
    let key = format!("{}#1", "#1&".repeat(terms - 1));
    let texts = run(
        &mut engine,
        GOD,
        &format!("@channel/perm Public=join={key}"),
    );
    assert!(
        !texts.iter().any(|t| t.contains("nests too deeply")),
        "a key at exactly the bound must be accepted: {texts:?}"
    );
    let chan = state(&engine)
        .channel_by_name("Public")
        .expect("the channel is still there")
        .clone();
    let doc = serde_json::to_value(&chan).expect("a channel serializes");
    let depth = mush_core::depth_of_value(&doc);
    assert!(
        depth <= mush_core::MAX_DOC_DEPTH,
        "the deepest lock the parser builds must persist: {depth} levels against a store \
         limit of {}",
        mush_core::MAX_DOC_DEPTH
    );
}

// ----------------------------------------------------------------------
// The way out: the boot policy
// ----------------------------------------------------------------------

/// (b) One undecodable channel document used to return `Err` for the whole
/// module boot, so comsys did not load at all — every channel, every
/// membership, every alias gone until an operator found the document by
/// hand. It is now skipped and named, and the sound channel next to it boots
/// normally.
#[test]
fn an_undecodable_channel_doc_is_skipped_and_the_module_still_boots() {
    let bad = undecodable_channel_doc(1, 200);
    // Prove the fixture: deep enough that no store would take it, and
    // genuinely undecodable — not merely deep.
    assert!(
        mush_core::depth_of_value(&bad) > mush_core::MAX_DOC_DEPTH,
        "the fixture must be past the store's bound"
    );
    serde_json::from_value::<Channel>(bad.clone())
        .expect_err("the fixture must not decode, or this test proves nothing");

    let (mut engine, ids, logged) = boot_over(
        vec![
            (SmolStr::new("chan:1"), bad),
            (SmolStr::new("chan:2"), sound_channel_doc(2, "Sound")),
            (SmolStr::new("member:2:1"), membership_doc(2, 1, "rot")),
            (SmolStr::new("member:2:2"), membership_doc(2, 2, "snd")),
            (SmolStr::new("meta"), serde_json::json!({"next_chan_id": 3})),
        ],
        &["Nick"],
    );
    let nick = ids[0];

    let st = state(&engine);
    assert!(
        st.channel_by_name("Sound").is_some(),
        "the sound channel next to a rotten one must load"
    );
    assert!(
        !st.channels.contains_key(&ChannelId(1)),
        "the unreadable channel is not in RAM"
    );
    assert!(
        st.members.contains_key(&(nick, ChannelId(2))),
        "the sound channel's membership loads"
    );
    assert!(
        !st.members.contains_key(&(nick, ChannelId(1))),
        "the unreadable channel's membership is not in RAM"
    );

    // The operator is told which key, and how many.
    assert!(
        logged.contains("chan:1") && logged.contains("bad channel doc skipped"),
        "the skip must name the key: {logged}"
    );
    assert!(
        logged.contains("skipped=1"),
        "the skips must be counted: {logged}"
    );

    // And nothing was deleted. The old policy failed the boot precisely to
    // avoid this sweep; skipping only works because the sweep skips too.
    let dels: Vec<String> = engine
        .take_module_ops()
        .iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::Del { key } => Some(key.to_string()),
            _ => None,
        })
        .collect();
    assert!(
        dels.is_empty(),
        "an unreadable channel's memberships must survive in the store: {dels:?}"
    );
}

/// The other half of skipping: an id whose document did not decode must not
/// be handed out again. `@channel/create` would `JsonSet` straight over the
/// document the operator is still trying to repair.
#[test]
fn an_unreadable_channel_id_is_never_reallocated() {
    // No `meta` doc, so the allocator falls back to the highest id seen —
    // and the highest id here is the one that would not decode.
    let (engine, _, _) = boot_over(
        vec![
            (SmolStr::new("chan:0"), sound_channel_doc(0, "Public")),
            (SmolStr::new("chan:5"), undecodable_channel_doc(5, 200)),
        ],
        &["Nick"],
    );
    assert_eq!(
        state(&engine).next_chan_id,
        6,
        "the allocator must count unreadable ids"
    );
}

/// A namespace that holds *only* unreadable documents is still not a first
/// boot: re-seeding `Public` and force-joining everyone on top of a store
/// full of channels nobody could read would be the worst possible recovery.
#[test]
fn a_namespace_of_unreadable_docs_is_not_mistaken_for_a_first_boot() {
    let (engine, _, _) = boot_over(
        vec![(SmolStr::new("chan:0"), undecodable_channel_doc(0, 200))],
        &["Nick"],
    );
    assert!(
        state(&engine).channels.is_empty(),
        "no channel loads, and none is seeded"
    );
}
