//! `@macro` and the kernel's macro alias stage, end to end through the
//! public API (16c N14; 16b cut AD.1): a real `World`, the real `Registry`
//! with every core command, and the dispatch pipeline.
//!
//! The rule every test here serves: a macro never reaches a verb its player
//! could not type, and a refusal on a name never tells a player what exists.
//! Every word a test asserts answers `Huh?` is first asserted to prefix no
//! bare verb (`Registry::lookup` is `NotFound`), since the command table
//! prefix-resolves bare verbs (`pa` is `page`).

use std::sync::Arc;

use mush_core::commands::macros::{
    BAD_NAME, CAP, CHAIN, DOLLAR_VERB, EMPTY_LINE, EXPANSION_TOO_LONG, LIST_EMPTY, MACROS_ATTR,
    NO_ACCOUNT, NO_ARGS, NO_QUEUE, PREFIX_LINE, TAKEN, TOO_LONG, USAGE,
};
use mush_core::{
    AliasOutcome, AliasStage, Attr, AttrValue, Audience, CmdError, CommandGate, CommandSpec, Ctx,
    Flags, GOD, LIMBO, LockExpr, LookupErr, ObjId, ObjKind, Outbox, Registry, Role, World,
    dispatch, register_core_commands, register_engine_commands,
};
use mush_proto::{NarrativeKind, OutputEvent};

/// A store-less lens for this harness: it holds no engine.
const DETACHED: mush_core::TypedStateLens<'static> =
    mush_core::TypedStateLens::Detached(mush_core::DetachedTypedState::Test);

/// A world with the core commands, a wizardly God, a builder Bob, and a
/// plain player Ann — everyone starting in Limbo (`commands.rs`'s rig; an
/// integration test file is its own crate).
struct Game {
    w: World,
    reg: Registry,
    ann: ObjId,
    bob: ObjId,
}

impl Game {
    fn new() -> Game {
        let mut w = World::new_minimal();
        let ann = w.create(ObjKind::Player, "Ann", GOD, Some(LIMBO)).unwrap();
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();
        w.set_flag(bob, Flags::BUILDER, true).unwrap();
        let mut reg = Registry::new();
        register_core_commands(&mut reg).unwrap();
        Game { w, reg, ann, bob }
    }

    fn run(&mut self, actor: ObjId, line: &str) -> Outbox {
        let mut out = Outbox::new();
        dispatch(&self.reg, &mut self.w, actor, line, &mut out, DETACHED);
        out
    }

    /// What `actor` is told, directly, in reply to `line`.
    fn answer(&mut self, actor: ObjId, line: &str) -> Vec<String> {
        let out = self.run(actor, line);
        actor_texts(&out, actor)
    }

    fn loc(&self, id: ObjId) -> Option<ObjId> {
        self.w.get(id).and_then(|o| o.location)
    }

    /// Assert `word` resolves to no command at all, prefix included, so a
    /// `Huh?` for it is the macro's absence and not a verb.
    fn assert_no_verb(&self, word: &str) {
        assert!(
            matches!(self.reg.lookup(word), Err(LookupErr::NotFound)),
            "{word:?} must prefix no verb for this test to mean anything"
        );
    }
}

/// Plain-text projection of narrative-ish events.
fn plain(e: &OutputEvent) -> Option<String> {
    match e {
        OutputEvent::Sys(t) => Some(t.to_plain_text()),
        OutputEvent::Narrative { text, .. } => Some(text.to_plain_text()),
        _ => None,
    }
}

/// All Sys/Narrative texts addressed directly to `actor`.
fn actor_texts(out: &Outbox, actor: ObjId) -> Vec<String> {
    out.events()
        .iter()
        .filter(|(a, _)| *a == Audience::Actor(actor))
        .filter_map(|(_, e)| plain(e))
        .collect()
}

/// All Sys/Narrative texts addressed to a room audience on `room`.
fn room_texts(out: &Outbox, room: ObjId) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(a, e)| match a {
            Audience::Room { room: r, .. } if *r == room => plain(e),
            _ => None,
        })
        .collect()
}

/// The text of every `Say` narrative in `out`, whatever its audience.
fn says(out: &Outbox) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Narrative {
                kind: NarrativeKind::Say,
                text,
                ..
            } => Some(text.to_plain_text()),
            _ => None,
        })
        .collect()
}

fn is_huh(texts: &[String]) -> bool {
    texts.iter().any(|t| t.starts_with("Huh?"))
}

/// A do-nothing handler for a verb a test registers.
fn noop() -> Arc<dyn mush_core::CommandHandler> {
    Arc::new(|_ctx: &mut Ctx<'_>| -> Result<(), CmdError> { Ok(()) })
}

/// A test alias stage registered AFTER core (so after the macro stage): it
/// claims `zap` through `claims_word`, and fully handles any line whose head
/// is `zap`, queueing `ZAPPED`.
struct ZapStage;

impl AliasStage for ZapStage {
    fn try_alias(
        &self,
        _w: &mut World,
        actor: ObjId,
        line: &str,
        out: &mut Outbox,
    ) -> AliasOutcome {
        if line.split_whitespace().next() == Some("zap") {
            out.info(actor, "ZAPPED");
            AliasOutcome::Handled
        } else {
            AliasOutcome::NotMine
        }
    }

    fn claims_word(&self, _w: &World, _actor: ObjId, word: &str) -> bool {
        word.eq_ignore_ascii_case("zap")
    }
}

#[test]
fn a_macro_is_defined_listed_typed_and_deleted() {
    let mut g = Game::new();
    let ann = g.ann;
    assert_eq!(
        g.answer(ann, "@macro hey=say hello $1"),
        vec!["Macro hey set: say hello $1".to_string()]
    );
    assert_eq!(
        g.answer(ann, "@macro"),
        vec![
            "Your macros (1 of 20):".to_string(),
            "  hey               say hello $1".to_string(),
        ]
    );
    let out = g.run(ann, "hey Bob");
    assert_eq!(room_texts(&out, LIMBO), vec!["hello Bob".to_string()]);
    assert_eq!(says(&out), vec!["hello Bob".to_string()]);
    assert_eq!(
        g.answer(ann, "@macro/del hey"),
        vec!["Macro hey deleted.".to_string()]
    );
    g.assert_no_verb("hey");
    let out = g.run(ann, "hey Bob");
    assert!(is_huh(&actor_texts(&out, ann)), "{:?}", out.events());
    assert!(says(&out).is_empty());
    assert_eq!(g.answer(ann, "@macro"), vec![LIST_EMPTY.to_string()]);
    assert!(
        g.w.get(ann).unwrap().attrs.get(MACROS_ATTR).is_none(),
        "the last deletion removes the attribute"
    );
}

#[test]
fn the_document_is_one_json_attribute_written_on_the_ordinary_path() {
    let mut g = Game::new();
    let ann = g.ann;
    let _ = g.w.take_dirty();
    g.run(ann, "@macro LK=lock $1");
    assert!(
        g.w.take_dirty().contains(&ann),
        "a definition marks the player dirty: the store's ordinary snapshot path"
    );
    let attr = g.w.get(ann).unwrap().attrs.get("macros").unwrap().clone();
    let AttrValue::Json(doc) = &attr.value else {
        panic!("the macros attribute is JSON, got {:?}", attr.value);
    };
    let map = doc.as_object().expect("a JSON object");
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("lk").and_then(|v| v.as_str()), Some("lock $1"));
    assert_eq!(attr.owner, ann);
    assert!(attr.flags.is_empty());
}

#[test]
fn a_bare_taken_word_is_refused_the_same_way_whatever_it_is() {
    let mut g = Game::new();
    let ann = g.ann;
    // A bare staff verb a mortal cannot run, and a stage claiming `zap`.
    g.reg
        .register(
            CommandSpec {
                name: "zapwiz".into(),
                min_role: Role::Wizard,
                help: "test".into(),
                ..CommandSpec::default()
            },
            noop(),
        )
        .unwrap();
    g.reg.add_alias_stage(Arc::new(ZapStage));
    let words = [
        "look", "l", "LOOK", "home", "HOME", "quit", "respond", "zapwiz", "zap",
    ];
    let mut answers = Vec::new();
    for word in words {
        let answer = g.answer(ann, &format!("@macro {word}=say x"));
        assert_eq!(answer, vec![TAKEN.to_string()], "{word}");
        answers.push(answer);
    }
    for pair in answers.windows(2) {
        assert_eq!(pair[0], pair[1]);
    }
    for word in words {
        let folded = word.to_ascii_lowercase();
        assert_eq!(
            g.answer(ann, &format!("@macro/del {word}")),
            vec![format!("You have no macro named {folded}.")],
            "{word}"
        );
    }
    assert!(g.w.get(ann).unwrap().attrs.get(MACROS_ATTR).is_none());
}

#[test]
fn an_at_or_plus_word_is_refused_on_its_characters_alone() {
    let mut g = Game::new();
    let ann = g.ann;
    let mut words: Vec<String> = vec!["@dig".into(), "@zzqnotaverb".into(), "+x".into()];
    let gated: Vec<String> = g
        .reg
        .specs()
        .filter(|s| {
            s.min_role != Role::Player
                || s.power.is_some()
                || s.privilege.is_some()
                || s.lock.is_some()
        })
        .flat_map(|s| {
            std::iter::once(s.name.to_string()).chain(s.aliases.iter().map(ToString::to_string))
        })
        .collect();
    assert!(!gated.is_empty(), "the core registry has gated verbs");
    words.extend(gated);
    let mut answers = Vec::new();
    for word in &words {
        let answer = g.answer(ann, &format!("@macro {word}=say x"));
        assert_eq!(answer, vec![BAD_NAME.to_string()], "{word}");
        answers.push(answer);
    }
    for pair in answers.windows(2) {
        assert_eq!(pair[0], pair[1]);
    }
    // DELETE never checks the shape: it folds and looks up.
    assert_eq!(
        g.answer(ann, "@macro/del @dig"),
        vec!["You have no macro named @dig.".to_string()]
    );
}

#[test]
fn names_and_lines_are_checked() {
    let mut g = Game::new();
    let ann = g.ann;
    for name in ["a b", "$x", "abcdefghijklmnopq"] {
        assert_eq!(
            g.answer(ann, &format!("@macro {name}=say x")),
            vec![BAD_NAME.to_string()],
            "{name}"
        );
    }
    assert_eq!(g.answer(ann, "@macro x"), vec![USAGE.to_string()]);
    assert_eq!(g.answer(ann, "@macro/del"), vec![USAGE.to_string()]);
    assert_eq!(g.answer(ann, "@macro x="), vec![EMPTY_LINE.to_string()]);
    // Characters, not bytes: 201 characters (397 bytes) is too long, 200 is
    // set.
    let long = format!("say {}", "é".repeat(197));
    assert_eq!(long.chars().count(), 201);
    assert_eq!(
        g.answer(ann, &format!("@macro x={long}")),
        vec![TOO_LONG.to_string()]
    );
    let fits = format!("say {}", "é".repeat(196));
    assert_eq!(fits.chars().count(), 200);
    assert_eq!(
        g.answer(ann, &format!("@macro x={fits}")),
        vec![format!("Macro x set: {fits}")]
    );
    for line in ["\"hi", ":waves", ";s grin", "\\The wind"] {
        assert_eq!(
            g.answer(ann, &format!("@macro q={line}")),
            vec![PREFIX_LINE.to_string()],
            "{line}"
        );
    }
    for line in ["@macro z=@wait 0=z", "@macro z=@WAIT/x 1=say hi"] {
        assert_eq!(g.answer(ann, line), vec![NO_QUEUE.to_string()], "{line}");
    }
    for line in [
        "@macro k=@sshkey/add x",
        "@macro k=@newpassword $1=$2",
        "@macro k=@newchar Bo",
    ] {
        assert_eq!(g.answer(ann, line), vec![NO_ACCOUNT.to_string()], "{line}");
    }
    // The verb is folded as the command table folds it: the Kelvin sign is
    // a `k` to the registry, so it is one to this check too.
    assert_eq!(
        g.answer(ann, "@macro k=@ssh\u{212A}ey/add x"),
        vec![NO_ACCOUNT.to_string()]
    );
    // A `$` in the first word would make the verb typed text, past every
    // check above: refused whatever the rest of the line is.
    for line in [
        "@macro y=$1 $1",
        "@macro k=$1",
        "@macro q=@wa$1 0=q $1",
        "@macro k=@ssh$1",
        "@macro y=$$x hi",
    ] {
        assert_eq!(g.answer(ann, line), vec![DOLLAR_VERB.to_string()], "{line}");
    }
    // The case fold: stored, listed and matched as `hey`.
    assert_eq!(
        g.answer(ann, "@macro HeY=say hi $1"),
        vec!["Macro hey set: say hi $1".to_string()]
    );
    let doc =
        g.w.get(ann)
            .unwrap()
            .attrs
            .get(MACROS_ATTR)
            .and_then(Attr::as_json)
            .cloned()
            .unwrap();
    assert!(doc.get("hey").is_some() && doc.get("HeY").is_none());
    let out = g.run(ann, "HEY there");
    assert_eq!(says(&out), vec!["hi there".to_string()]);
}

#[test]
fn a_word_that_only_shortens_a_verb_may_be_a_macro() {
    let mut g = Game::new();
    let ann = g.ann;
    assert!(g.reg.spec_of("lo").is_none());
    assert!(matches!(
        g.reg.lookup("lo"),
        Ok(_) | Err(LookupErr::Ambiguous(_))
    ));
    assert_eq!(
        g.answer(ann, "@macro lo=say short"),
        vec!["Macro lo set: say short".to_string()]
    );
    let out = g.run(ann, "lo");
    assert_eq!(says(&out), vec!["short".to_string()]);
    assert!(
        !out.events()
            .iter()
            .any(|(_, e)| matches!(e, OutputEvent::Room(_))),
        "the macro wins over the prefix: no room is rendered"
    );
}

#[test]
fn a_macro_is_expanded_once_whatever_the_document_holds() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(ann, "@macro a=say one");
    assert_eq!(g.answer(ann, "@macro b=a two"), vec![CHAIN.to_string()]);
    assert_eq!(g.answer(ann, "@macro a=a"), vec![CHAIN.to_string()]);
    g.run(ann, "@macro c=say three");
    assert_eq!(g.answer(ann, "@macro d=c"), vec![CHAIN.to_string()]);
    // The reverse: a new name that another macro's line starts with.
    assert_eq!(
        g.answer(ann, "@macro x=y two"),
        vec!["Macro x set: y two".to_string()]
    );
    assert_eq!(g.answer(ann, "@macro y=say hi"), vec![CHAIN.to_string()]);
    assert_eq!(
        g.answer(ann, "@macro x=say other"),
        vec!["Macro x replaced: say other".to_string()]
    );
    assert_eq!(
        g.answer(ann, "@macro y=say hi"),
        vec!["Macro y set: say hi".to_string()]
    );
    // A document written by hand past the definition-time rules: `b` names
    // macro `a`, and is still expanded exactly once.
    g.w.set_attr(
        ann,
        "macros",
        Attr::json(serde_json::json!({"a": "say one", "b": "a two"}), ann),
    )
    .unwrap();
    g.assert_no_verb("a");
    g.assert_no_verb("b");
    let out = g.run(ann, "b");
    assert!(is_huh(&actor_texts(&out, ann)), "{:?}", out.events());
    assert!(
        says(&out).is_empty(),
        "b's line `a two` is never re-expanded"
    );
    let out = g.run(ann, "a");
    assert_eq!(says(&out), vec!["one".to_string()]);
}

#[test]
fn an_expansion_never_queues_or_touches_the_account_whatever_the_document_holds() {
    let mut g = Game::new();
    let ann = g.ann;
    // `@wait` is the engine's verb: register it, and prove a TYPED `@wait 0`
    // queues, so the absences below are the stage's refusal and not a
    // missing verb.
    register_engine_commands(&mut g.reg).unwrap();
    let out = g.run(ann, "@wait 0=say typed");
    assert_eq!(out.deferred().len(), 1, "a typed @wait queues its line");
    // The verb refuses these at definition (`DOLLAR_VERB`)…
    assert_eq!(
        g.answer(ann, "@macro y=$1 $1"),
        vec![DOLLAR_VERB.to_string()]
    );
    // …and a document written past the verb gets no further: `y @wait 0=y`
    // would queue `y @wait 0=y`, which re-arms itself on every fire.
    g.w.set_attr(
        ann,
        "macros",
        Attr::json(
            serde_json::json!({"y": "$1 $1", "k": "$1", "w": "@wa$1 0=say $2"}),
            ann,
        ),
    )
    .unwrap();
    for line in [
        "y @wait 0=y",
        "k @WAIT 0=say hi",
        "k @wait/x 0=y",
        "w it later",
    ] {
        let out = g.run(ann, line);
        assert_eq!(actor_texts(&out, ann), vec![NO_QUEUE.to_string()], "{line}");
        assert!(
            out.deferred().is_empty(),
            "`{line}` queued: {:?}",
            out.deferred()
        );
        assert!(says(&out).is_empty(), "{line}");
    }
    for line in [
        "k @sshkey/add ssh-ed25519 AAAA",
        "k @NewPassword a=b",
        "k @newchar Bo",
        "k @ssh\u{212A}ey/list",
    ] {
        let out = g.run(ann, line);
        assert_eq!(
            actor_texts(&out, ann),
            vec![NO_ACCOUNT.to_string()],
            "{line}"
        );
    }
    // Any other verb the document's typed text names still runs as typed.
    let out = g.run(ann, "k say hi");
    assert_eq!(says(&out), vec!["hi".to_string()]);
}

/// A handler that tells its actor `text`, so a test sees it ran.
fn tells(text: &'static str) -> Arc<dyn mush_core::CommandHandler> {
    Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
        ctx.out.info(ctx.actor, text);
        Ok(())
    })
}

#[test]
fn a_macro_reaches_no_verb_its_player_could_not_type() {
    let mut g = Game::new();
    let ann = g.ann;
    // A bare Wizard-only verb and a bare lock-gated one (only God passes),
    // beside the core's Builder-only `@dig`: role, lock and role again.
    g.reg
        .register(
            CommandSpec {
                name: "zapwiz".into(),
                min_role: Role::Wizard,
                help: "test".into(),
                ..CommandSpec::default()
            },
            tells("ZAPWIZ RAN"),
        )
        .unwrap();
    g.reg
        .register(
            CommandSpec {
                name: "sanctify".into(),
                lock: Some(LockExpr::Is(GOD)),
                help: "test".into(),
                ..CommandSpec::default()
            },
            tells("SANCTIFY RAN"),
        )
        .unwrap();
    // Typed, each is refused to Ann; through a macro, the refusal is the same
    // one, read as it would be had she typed the line, and nothing ran.
    let typed = g.answer(ann, "@dig Pit");
    assert_eq!(typed, vec!["Permission denied.".to_string()]);
    assert_eq!(
        g.answer(ann, "@macro dg=@dig $1"),
        vec!["Macro dg set: @dig $1".to_string()]
    );
    assert_eq!(g.answer(ann, "dg Pit"), typed);
    assert!(
        !g.w.iter().any(|o| o.name == "Pit"),
        "a mortal's macro dug no room"
    );
    for (name, verb) in [("zw", "zapwiz"), ("sf", "sanctify")] {
        let typed = g.answer(ann, verb);
        assert_eq!(typed, vec!["Permission denied.".to_string()], "{verb}");
        assert_eq!(
            g.answer(ann, &format!("@macro {name}={verb}")),
            vec![format!("Macro {name} set: {verb}")]
        );
        assert_eq!(g.answer(ann, name), typed, "{name}");
    }
    // The refusal is the typist's, not the macro's: God, who may type them,
    // reaches them through the same macros.
    for (name, verb, ran) in [
        ("zw", "zapwiz", "ZAPWIZ RAN"),
        ("sf", "sanctify", "SANCTIFY RAN"),
    ] {
        g.run(GOD, &format!("@macro {name}={verb}"));
        assert_eq!(g.answer(GOD, name), vec![ran.to_string()], "{name}");
    }
}

#[test]
fn arguments_are_substituted_and_the_last_takes_the_rest() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(ann, "@macro t=say [$1|$2|$3|$$|$4]");
    let out = g.run(ann, "t x y z w");
    assert_eq!(says(&out), vec!["[x|y|z w|$|$4]".to_string()]);
    let out = g.run(ann, "t x");
    assert_eq!(says(&out), vec!["[x|||$|$4]".to_string()]);
    g.run(ann, "@macro r=say $1");
    let out = g.run(ann, "r moving to B now");
    assert_eq!(says(&out), vec!["moving to B now".to_string()]);
    g.run(ann, "@macro k=say $1");
    assert_eq!(g.answer(ann, "k"), vec!["Say what?".to_string()]);
    g.run(ann, "@macro fc=say cannon");
    let out = g.run(ann, "fc BA");
    assert_eq!(actor_texts(&out, ann), vec![NO_ARGS.to_string()]);
    assert!(says(&out).is_empty());
    let out = g.run(ann, "fc");
    assert_eq!(says(&out), vec!["cannon".to_string()]);
    // An empty expansion is not special-cased: it goes on through dispatch
    // and answers `Huh?`. (`e` would prefix `examine`; `emp` prefixes nothing.)
    g.assert_no_verb("emp");
    g.run(ann, "@macro emp=$1");
    assert!(is_huh(&g.answer(ann, "emp")));
}

#[test]
fn an_expansion_is_capped_at_what_a_player_could_type() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(ann, "@macro big=say $1$1$1$1$1$1$1$1$1$1");
    let out = g.run(ann, &format!("big {}", "w".repeat(900)));
    assert_eq!(actor_texts(&out, ann), vec![EXPANSION_TOO_LONG.to_string()]);
    assert!(says(&out).is_empty());
    let out = g.run(ann, &format!("big {}", "w".repeat(100)));
    assert_eq!(says(&out), vec!["w".repeat(1000)]);
}

#[test]
fn twenty_is_the_cap() {
    let mut g = Game::new();
    let ann = g.ann;
    for i in 0..20 {
        assert_eq!(
            g.answer(ann, &format!("@macro m{i}=say {i}")),
            vec![format!("Macro m{i} set: say {i}")]
        );
    }
    assert_eq!(g.answer(ann, "@macro m20=say 20"), vec![CAP.to_string()]);
    assert_eq!(
        g.answer(ann, "@macro m5=say five"),
        vec!["Macro m5 replaced: say five".to_string()]
    );
    g.run(ann, "@macro/del m0");
    assert_eq!(
        g.answer(ann, "@macro m20=say 20"),
        vec!["Macro m20 set: say 20".to_string()]
    );
}

#[test]
fn a_hand_written_text_attribute_is_no_macro() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(ann, "@set me/macros=hey:say hi");
    assert!(
        g.w.get(ann)
            .unwrap()
            .attrs
            .get(MACROS_ATTR)
            .and_then(Attr::as_text)
            .is_some()
    );
    g.assert_no_verb("hey");
    let out = g.run(ann, "hey");
    assert!(is_huh(&actor_texts(&out, ann)), "{:?}", out.events());
    assert!(says(&out).is_empty());
    assert_eq!(g.answer(ann, "@macro"), vec![LIST_EMPTY.to_string()]);
    assert_eq!(
        g.answer(ann, "@macro hey=say hi"),
        vec!["Macro hey set: say hi".to_string()]
    );
    assert!(matches!(
        g.w.get(ann).unwrap().attrs.get(MACROS_ATTR).unwrap().value,
        AttrValue::Json(_)
    ));
    let out = g.run(ann, "hey");
    assert_eq!(says(&out), vec!["hi".to_string()]);
}

/// A gate refusing the verb `zap` to one actor.
struct NoZapFor(ObjId);

impl CommandGate for NoZapFor {
    fn check(
        &self,
        _w: &World,
        _reg: &Registry,
        actor: ObjId,
        verb: &str,
        _line: &str,
    ) -> Result<(), String> {
        if actor == self.0 && verb.eq_ignore_ascii_case("zap") {
            Err("The gate holds zap.".to_string())
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_rewrite_continues_to_the_later_stages_and_is_gated_first() {
    let mut g = Game::new();
    let ann = g.ann;
    g.reg.add_alias_stage(Arc::new(ZapStage));
    assert_eq!(g.answer(ann, "@macro zap=say x"), vec![TAKEN.to_string()]);
    assert_eq!(
        g.answer(ann, "@macro z=zap $1"),
        vec!["Macro z set: zap $1".to_string()]
    );
    // The macro stage rewrites `z now` to `zap now`, and the LATER stage sees
    // the rewrite (before D9 it fell past every stage to `Huh?`).
    assert_eq!(g.answer(ann, "z now"), vec!["ZAPPED".to_string()]);
    // With a gate on `zap`, the substitute is refused at the rewrite, before
    // the later stage ever sees it.
    g.reg.add_command_gate(Arc::new(NoZapFor(ann)));
    assert_eq!(
        g.answer(ann, "z now"),
        vec!["The gate holds zap.".to_string()]
    );
}

#[test]
fn an_exit_s_name_may_be_a_macro_but_the_exit_wins_on_the_whole_line() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(GOD, "@dig Pit=north;n");
    assert_eq!(
        g.answer(ann, "@macro north=say n"),
        vec![
            "Macro north set: say n".to_string(),
            "Note: north is also an exit here; typed alone, the exit wins.".to_string(),
        ]
    );
    // The bare key's line: the exit.
    let out = g.run(ann, "north");
    assert!(says(&out).is_empty());
    assert_ne!(g.loc(ann), Some(LIMBO), "north alone takes the exit");
    g.w.move_to(ann, LIMBO).unwrap();
    // Anything after the name: the macro.
    let out = g.run(ann, "north x");
    assert_eq!(g.loc(ann), Some(LIMBO), "north x runs the macro");
    assert!(says(&out).is_empty(), "say n takes no arguments");
    assert_eq!(actor_texts(&out, ann), vec![NO_ARGS.to_string()]);
    g.run(ann, "@macro north=say n $1");
    let out = g.run(ann, "north x");
    assert_eq!(g.loc(ann), Some(LIMBO));
    assert_eq!(says(&out), vec!["n x".to_string()]);
    // A multi-word exit: the button submits its whole name.
    assert_eq!(
        g.answer(ann, "@macro out=say o"),
        vec!["Macro out set: say o".to_string()]
    );
    g.run(GOD, "@dig Street=Out to the street;out");
    let street =
        g.w.iter()
            .find(|o| o.name == "Street")
            .map(|o| o.id)
            .unwrap();
    g.run(ann, "Out to the street");
    assert_eq!(g.loc(ann), Some(street));

    // The note tells only of an exit Ann's room view lists, or it would be a
    // silent probe for one she has not perceived. A DARK exit she does not
    // control: no note (God, who controls it, is told).
    g.w.move_to(ann, LIMBO).unwrap();
    g.run(GOD, "@dig Vault=trapdoor");
    let trapdoor =
        g.w.exits_of(LIMBO)
            .iter()
            .copied()
            .find(|&e| g.w.get(e).is_some_and(|o| o.name == "trapdoor"))
            .unwrap();
    g.w.set_flag(trapdoor, Flags::DARK, true).unwrap();
    assert_eq!(
        g.answer(ann, "@macro trapdoor=say t $1"),
        vec!["Macro trapdoor set: say t $1".to_string()]
    );
    assert_eq!(
        g.answer(GOD, "@macro trapdoor=say t $1"),
        vec![
            "Macro trapdoor set: say t $1".to_string(),
            "Note: trapdoor is also an exit here; typed alone, the exit wins.".to_string(),
        ]
    );
    // A global exit on Limbo, seen from another room: typed alone it would
    // still be taken, but the room view never lists it, so no note either.
    let pit = g.w.iter().find(|o| o.name == "Pit").map(|o| o.id).unwrap();
    g.w.move_to(ann, pit).unwrap();
    assert!(
        mush_core::commands::movement::find_exit(&g.w, ann, "out").is_some(),
        "the global exit answers `out` from the Pit"
    );
    assert!(g.w.exits_of(pit).iter().all(|&e| {
        g.w.get(e)
            .is_none_or(|o| !o.name.to_ascii_lowercase().contains("out"))
    }));
    assert_eq!(
        g.answer(ann, "@macro out=say o $1"),
        vec!["Macro out replaced: say o $1".to_string()]
    );
}

#[test]
fn a_parent_s_macros_are_not_the_child_s() {
    let mut g = Game::new();
    let (ann, bob) = (g.ann, g.bob);
    g.w.set_parent(ann, Some(bob)).unwrap();
    assert_eq!(
        g.answer(bob, "@macro zq=say parent"),
        vec!["Macro zq set: say parent".to_string()]
    );
    let out = g.run(bob, "zq");
    assert_eq!(says(&out), vec!["parent".to_string()]);
    g.assert_no_verb("zq");
    let out = g.run(ann, "zq");
    assert!(is_huh(&actor_texts(&out, ann)), "{:?}", out.events());
    assert!(says(&out).is_empty());
    assert_eq!(g.answer(ann, "@macro"), vec![LIST_EMPTY.to_string()]);
}

#[test]
fn the_list_marks_a_name_that_is_now_a_command() {
    let mut g = Game::new();
    let ann = g.ann;
    g.run(ann, "@macro zz=say hi");
    g.reg
        .register(
            CommandSpec {
                name: "zz".into(),
                help: "test".into(),
                ..CommandSpec::default()
            },
            noop(),
        )
        .unwrap();
    assert_eq!(
        g.answer(ann, "@macro"),
        vec![
            "Your macros (1 of 20):".to_string(),
            "  zz                say hi  (now a command; @macro/del it)".to_string(),
        ]
    );
    // The stage cannot see the registry: the macro still wins for its owner.
    let out = g.run(ann, "zz");
    assert_eq!(says(&out), vec!["hi".to_string()]);
}
