//! Interface cut I1, the core half of the contracts (docs/interface/02 §3.1):
//! the viewer-filtered, bounded, deterministic `commands.list` table and the
//! bounded `RoomView.ambient` map with core's own `ic_time`.

use std::sync::Arc;

use mush_core::commands::info::{
    MAX_COMMAND_SUMMARY_BYTES, MAX_COMMANDS_LIST_BYTES, command_entries, commands_keyframe,
    split_help,
};
use mush_core::output::room_view;
use mush_core::{
    AMBIENT_IC_TIME_KEY, CmdError, CommandSpec, Ctx, EventHook, GOD, HookPoint, HookReadContext,
    LIMBO, MAX_AMBIENT_BYTES, MAX_AMBIENT_KEYS, ObjId, ObjKind, Powers, Registrar, Registry, Role,
    World, register_core_commands,
};
use mush_proto::SmolStr;

fn setup() -> (World, Registry, ObjId, ObjId) {
    let mut w = World::new_minimal();
    let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
    let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
    let mut reg = Registry::new();
    register_core_commands(&mut reg).unwrap();
    (w, reg, nick, hall)
}

fn noop() -> Arc<dyn mush_core::CommandHandler> {
    Arc::new(|_ctx: &mut Ctx<'_>| -> Result<(), CmdError> { Ok(()) })
}

// ----------------------------------------------------------------------
// commands.list (D-I-5)
// ----------------------------------------------------------------------

#[test]
fn split_help_uses_the_house_separator_then_the_first_line() {
    let (usage, summary) =
        split_help("WHO  --  List connected players with their idle and connected times.");
    assert_eq!(usage, "WHO");
    assert_eq!(
        summary,
        "List connected players with their idle and connected times."
    );
    let (usage, summary) = split_help(
        "@unit/create <obj>=<schema-id>   make an object a tactical unit\n@unit/show <obj>   inspect it",
    );
    assert_eq!(
        usage,
        "@unit/create <obj>=<schema-id> make an object a tactical unit"
    );
    assert_eq!(summary, "@unit/show <obj> inspect it");
    let (_, summary) = split_help(&format!("x  --  {}", "y".repeat(500)));
    assert!(
        summary.len() <= MAX_COMMAND_SUMMARY_BYTES && summary.ends_with('…'),
        "summary capped with an ellipsis: {} bytes",
        summary.len()
    );
}

#[test]
fn command_entries_are_viewer_filtered_sorted_and_deterministic() {
    let (w, mut reg, nick, _) = setup();
    reg.register(
        CommandSpec {
            name: "@wizonly".into(),
            min_role: Role::Wizard,
            help: "@wizonly  --  Staff only.".into(),
            ..CommandSpec::default()
        },
        noop(),
    )
    .unwrap();
    reg.register(
        CommandSpec {
            name: "@powered".into(),
            power: Some(Powers::SEE_HIDDEN),
            help: "@powered  --  Needs a power.".into(),
            ..CommandSpec::default()
        },
        noop(),
    )
    .unwrap();

    let player: Vec<String> = command_entries(&reg, &w, nick)
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(player.iter().any(|n| n == "WHO"), "{player:?}");
    assert!(
        !player.iter().any(|n| n == "@wizonly" || n == "@powered"),
        "gated verbs are absent for a player, not greyed: {player:?}"
    );
    let mut sorted = player.clone();
    sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(player, sorted, "byte-ordered by name");

    let god: Vec<String> = command_entries(&reg, &w, GOD)
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(
        god.iter().any(|n| n == "@wizonly") && god.iter().any(|n| n == "@powered"),
        "{god:?}"
    );

    let who = command_entries(&reg, &w, nick)
        .into_iter()
        .find(|e| e.name == "WHO")
        .expect("WHO listed");
    assert_eq!(who.usage, "WHO");
    assert!(who.summary.starts_with("List connected players"), "{who:?}");
    assert_eq!(
        command_entries(&reg, &w, nick),
        command_entries(&reg, &w, nick),
        "byte-identical across two calls"
    );
}

#[test]
fn commands_keyframe_stops_at_the_byte_cap_and_says_so() {
    let (w, mut reg, nick, _) = setup();
    for i in 0..400 {
        reg.register(
            CommandSpec {
                name: format!("+pad{i:03}").into(),
                help: format!("+pad{i:03} <arg>  --  {}", "padding ".repeat(40)),
                ..CommandSpec::default()
            },
            noop(),
        )
        .unwrap();
    }
    let frame = commands_keyframe(&reg, &w, nick);
    assert_eq!(frame["truncated"], serde_json::json!(true));
    let bytes = serde_json::to_vec(&frame).unwrap().len();
    assert!(
        bytes <= MAX_COMMANDS_LIST_BYTES + 64,
        "the keyframe is bounded: {bytes} bytes"
    );
    let listed = frame["commands"].as_array().unwrap().len();
    assert!(listed > 0 && listed < 400, "stopped, not split: {listed}");
}

// ----------------------------------------------------------------------
// RoomView.ambient (D-I-4)
// ----------------------------------------------------------------------

#[test]
fn room_view_carries_core_ic_time_and_nothing_else_by_default() {
    let (w, reg, nick, hall) = setup();
    let rv = room_view(&reg, &w, nick, hall).expect("room renders");
    let ic = rv
        .ambient
        .get(AMBIENT_IC_TIME_KEY)
        .expect("core contributes ic_time");
    assert!(ic["hour"].is_number() && ic["season"].is_string());
    assert_eq!(rv.ambient.len(), 1, "{:?}", rv.ambient);
}

struct Contributor {
    pairs: Vec<(SmolStr, serde_json::Value)>,
}

impl EventHook for Contributor {
    fn room_ambient(
        &self,
        _ctx: &HookReadContext<'_>,
        _looker: ObjId,
        _room: ObjId,
    ) -> Vec<(SmolStr, serde_json::Value)> {
        self.pairs.clone()
    }
}

#[test]
fn ambient_hooks_win_first_per_key_and_are_bounded() {
    let (w, mut reg, nick, hall) = setup();
    // A module claims the core key first (a ruleset calendar), a second key
    // of its own, one over the byte cap, and more keys than the cap.
    let mut pairs = vec![
        (
            SmolStr::new_static(AMBIENT_IC_TIME_KEY),
            serde_json::json!({ "label": "12 Brumaire" }),
        ),
        (
            SmolStr::new_static("weather"),
            serde_json::json!({ "state": "rain" }),
        ),
        (
            SmolStr::new_static("huge"),
            serde_json::json!("x".repeat(MAX_AMBIENT_BYTES)),
        ),
    ];
    for i in 0..(MAX_AMBIENT_KEYS + 4) {
        pairs.push((SmolStr::new(format!("k{i}")), serde_json::json!(i)));
    }
    Registrar::new(&mut reg, "mod").hook(HookPoint::RoomAmbient, Arc::new(Contributor { pairs }));
    // A second module offering the same key loses.
    Registrar::new(&mut reg, "late").hook(
        HookPoint::RoomAmbient,
        Arc::new(Contributor {
            pairs: vec![(
                SmolStr::new_static("weather"),
                serde_json::json!({ "state": "snow" }),
            )],
        }),
    );

    let rv = room_view(&reg, &w, nick, hall).expect("room renders");
    assert_eq!(rv.ambient[AMBIENT_IC_TIME_KEY]["label"], "12 Brumaire");
    assert_eq!(rv.ambient["weather"]["state"], "rain", "first writer wins");
    assert!(
        !rv.ambient.contains_key("huge"),
        "over the byte cap: dropped"
    );
    assert!(
        rv.ambient.len() <= MAX_AMBIENT_KEYS,
        "{:?}",
        rv.ambient.keys()
    );
    let bytes = serde_json::to_vec(&rv.ambient).unwrap().len();
    assert!(bytes <= MAX_AMBIENT_BYTES, "bounded: {bytes}");
}
