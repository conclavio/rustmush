//! Interface cut I3: a public roll reaches the room as one line — `roll  Nick
//! Agility+Pistols  5 6 3 1 = 2 hits  [Physical limit 6]` — an `Emit` whose
//! first span is tagged `roll` in the same layout the WoD ruleset uses, while
//! every record (the `.roll` panel, the private line) keeps the classic
//! sentence. `+init/roll` takes the same form.

use mush_core::{Audience, GOD, ObjId};
use mush_jobs::JobsModule;
use mush_proto::{NarrativeKind, ObjRef, OutputEvent, RichText};
use mush_ruleset::{ROLL_TAG, is_roll_narration};
use mush_testkit::TestWorld;
use ruleset_shadowrun::ShadowrunRuleset;

fn world() -> (TestWorld, ObjId) {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ShadowrunRuleset::with_seed(777)),
    ]);
    let nick = tw.spawn_player("Nick");
    for line in [
        "@sheet/set Nick/metatype=human",
        "@sheet/set Nick/body=5",
        "@sheet/set Nick/agility=6",
        "@sheet/set Nick/reaction=5",
        "@sheet/set Nick/intuition=3",
        "@sheet/set Nick/strength=4",
        "@sheet/set Nick/willpower=4",
        "@sheet/set Nick/active_skills:pistols=4",
    ] {
        tw.run(GOD, line);
    }
    (tw, nick)
}

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

fn panel_summary(events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    events
        .iter()
        .find_map(|(aud, ev)| match (aud, ev) {
            (Audience::Actor(a), OutputEvent::Panel { feed, data, .. })
                if *a == who && feed.as_str() == "sr.roll" =>
            {
                data["summary"].as_str().map(str::to_string)
            }
            _ => None,
        })
        .expect("an sr.roll keyframe to the roller")
}

#[test]
fn a_public_roll_reaches_the_room_as_a_roll_tagged_emit() {
    let (mut tw, nick) = world();
    let ev = tw.run(nick, "+roll agility+pistols");
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
    assert!(body.starts_with("  Agility+Pistols  "), "{body}");
    assert!(body.contains(" = "), "{body}");
    assert!(body.ends_with("  [Physical limit 6]"), "{body}");
    assert!(
        panel_summary(&ev, nick).starts_with("Nick rolls Agility+Pistols (10 dice) and rolled "),
        "{}",
        panel_summary(&ev, nick)
    );
}

#[test]
fn a_private_roll_keeps_the_classic_sentence_and_sends_the_room_nothing() {
    let (mut tw, nick) = world();
    let ev = tw.run(nick, "+roll/priv agility+pistols");
    assert!(room_narratives(&ev).is_empty(), "{ev:?}");
    let seen = tw.seen_by(&ev, nick);
    assert!(
        seen.iter()
            .any(|l| l.starts_with("Nick rolls Agility+Pistols (10 dice) and rolled ")),
        "{seen:?}"
    );
    assert!(!seen.iter().any(|l| l.starts_with("roll  ")), "{seen:?}");
}

#[test]
fn a_contested_roll_names_both_sides_and_the_verdict() {
    let (mut tw, nick) = world();
    tw.spawn_player("Vlad");
    tw.run(GOD, "@sheet/set Vlad/reaction=4");
    tw.run(GOD, "@sheet/set Vlad/intuition=3");
    let ev = tw.run(
        nick,
        "+roll agility+pistols contested Vlad=reaction+intuition",
    );
    let lines = room_narratives(&ev);
    assert_eq!(lines.len(), 1, "{ev:?}");
    let text = &lines[0].2;
    assert!(is_roll_narration(text));
    let spans = text.spans();
    assert_eq!(spans.len(), 6, "{spans:?}");
    assert_eq!(spans[4].tag.as_deref(), Some("player"));
    assert_eq!(spans[4].text, "Vlad");
    let plain = text.to_plain_text();
    assert!(
        plain.starts_with("roll  Nick  agility+pistols  "),
        "{plain}"
    );
    assert!(
        plain.contains("  vs  Vlad  reaction+intuition  "),
        "{plain}"
    );
    assert!(plain.contains("  => "), "{plain}");
}

#[test]
fn an_initiative_roll_takes_the_same_form() {
    let (mut tw, nick) = world();
    let ev = tw.run(nick, "+init/roll");
    let lines = room_narratives(&ev);
    assert_eq!(lines.len(), 1, "{ev:?}");
    let text = &lines[0].2;
    assert!(is_roll_narration(text));
    let plain = text.to_plain_text();
    assert!(plain.starts_with("roll  Nick  Initiative  "), "{plain}");
    assert!(plain.contains(" = "), "{plain}");
    assert!(plain.ends_with("D6"), "{plain}");
}

/// Look-back I6-C11: `+init/roll` reaches the `sr.roll` feed like every
/// other roll — with `who`, the classic `summary` and no `outcome`, since an
/// Initiative roll is dice plus a base and not a hit count.
#[test]
fn an_initiative_roll_reaches_the_rolls_feed_without_an_outcome() {
    let (mut tw, nick) = world();
    let ev = tw.run(nick, "+init/roll");
    let frame = ev
        .iter()
        .find_map(|(aud, ev)| match (aud, ev) {
            (Audience::Actor(a), OutputEvent::Panel { feed, data, .. })
                if *a == nick && feed.as_str() == "sr.roll" =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("an sr.roll keyframe to the roller");
    assert_eq!(frame["who"], nick.to_string());
    assert_eq!(frame["expr"], "Initiative");
    let summary = frame["summary"].as_str().expect("summary");
    // Reaction 5 + Intuition 3 = 8, one initiative die.
    assert!(
        summary.starts_with("Nick rolls Initiative: 8 + 1D6 [") && summary.contains("] = "),
        "{summary}"
    );
    assert!(
        frame.get("outcome").is_none(),
        "no hit-count outcome on an initiative roll: {frame}"
    );
}
