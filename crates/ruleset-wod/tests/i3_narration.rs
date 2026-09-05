//! Interface cut I3: a public roll reaches the room as one line — `roll  Nick
//! strength+brawl  8 6 4 3 5 = 2 successes  diff 6` — an `Emit` whose first
//! span is tagged `roll` in the layout `mush_ruleset::roll_narration` fixes,
//! while every record (the `.roll` panel, the private line) keeps the classic
//! sentence.

use mush_core::{Audience, GOD, ObjId};
use mush_jobs::JobsModule;
use mush_proto::{NarrativeKind, ObjRef, OutputEvent, RichText};
use mush_ruleset::{ROLL_TAG, is_roll_narration};
use mush_testkit::TestWorld;
use ruleset_wod::WodRuleset;

fn world() -> (TestWorld, ObjId) {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(12345)),
    ]);
    let nick = tw.spawn_player("Nick");
    tw.run(GOD, "@sheet/set Nick/strength=3");
    tw.run(GOD, "@sheet/set Nick/brawl=2");
    (tw, nick)
}

/// Every narrative the room received: `(kind, actor, text)`.
fn room_narratives(
    events: &[(Audience, OutputEvent)],
) -> Vec<(NarrativeKind, Option<ObjRef>, RichText)> {
    events
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (Audience::Room { .. }, OutputEvent::Narrative { kind, actor, text }) => {
                Some((*kind, actor.clone(), text.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The `summary` the roller's `wod.roll` panel keyframe carried.
fn panel_summary(events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    events
        .iter()
        .find_map(|(aud, ev)| match (aud, ev) {
            (Audience::Actor(a), OutputEvent::Panel { feed, data, .. })
                if *a == who && feed.as_str() == "wod.roll" =>
            {
                data["summary"].as_str().map(str::to_string)
            }
            _ => None,
        })
        .expect("a wod.roll keyframe to the roller")
}

#[test]
fn a_public_roll_reaches_the_room_as_a_roll_tagged_emit() {
    let (mut tw, nick) = world();
    let ev = tw.run(nick, "+roll strength+brawl");
    let lines = room_narratives(&ev);
    assert_eq!(lines.len(), 1, "{ev:?}");
    let (kind, actor, text) = &lines[0];
    assert_eq!(*kind, NarrativeKind::Emit);
    assert_eq!(actor.as_ref().map(|a| a.name.as_str()), Some("Nick"));
    assert!(is_roll_narration(text), "{text:?}");
    let spans = text.spans();
    assert_eq!(spans.len(), 4, "{spans:?}");
    assert_eq!(spans[0].tag.as_deref(), Some(ROLL_TAG));
    assert_eq!(spans[0].text, "roll");
    assert_eq!(spans[1].text, "  ");
    assert_eq!(spans[2].tag.as_deref(), Some("player"));
    assert_eq!(spans[2].text, "Nick");
    let body = &spans[3].text;
    assert!(body.starts_with("  strength+brawl  "), "{body}");
    assert!(body.contains(" = "), "{body}");
    assert!(body.ends_with("  diff 6"), "{body}");
    // The record keeps the classic sentence.
    assert!(
        panel_summary(&ev, nick)
            .starts_with("Nick rolls strength+brawl (5 dice, diff 6) and rolled "),
        "{}",
        panel_summary(&ev, nick)
    );
}

#[test]
fn a_private_roll_keeps_the_classic_sentence_and_sends_the_room_nothing() {
    let (mut tw, nick) = world();
    let ev = tw.run(nick, "+roll/priv strength+brawl");
    assert!(room_narratives(&ev).is_empty(), "{ev:?}");
    let seen = tw.seen_by(&ev, nick);
    assert!(
        seen.iter()
            .any(|l| { l.starts_with("Nick rolls strength+brawl (5 dice, diff 6) and rolled ") }),
        "{seen:?}"
    );
    assert!(!seen.iter().any(|l| l.starts_with("roll  ")), "{seen:?}");
}

#[test]
fn a_contested_roll_names_both_sides_and_the_verdict() {
    let (mut tw, nick) = world();
    tw.spawn_player("Bob");
    tw.run(GOD, "@sheet/set Bob/dexterity=2");
    let ev = tw.run(nick, "+roll strength+brawl contested Bob=dexterity");
    let lines = room_narratives(&ev);
    assert_eq!(lines.len(), 1, "{ev:?}");
    let text = &lines[0].2;
    assert!(is_roll_narration(text));
    let spans = text.spans();
    assert_eq!(spans.len(), 6, "{spans:?}");
    assert_eq!(spans[2].text, "Nick");
    assert_eq!(spans[4].tag.as_deref(), Some("player"));
    assert_eq!(spans[4].text, "Bob");
    let plain = text.to_plain_text();
    assert!(plain.starts_with("roll  Nick  strength+brawl  "), "{plain}");
    assert!(plain.contains("  vs  Bob  dexterity  "), "{plain}");
    assert!(plain.contains("  => "), "{plain}");
    assert!(plain.ends_with("  diff 6"), "{plain}");
}
