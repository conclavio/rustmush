//! Interface cut I6: the web roll builder's line — `+roll[/spec] <a>+<b> vs
//! <d>` — round-trips through the WoD `+roll` command as typed, so a click
//! on the builder is exactly a line in the box. (`/wp` is not built: the
//! command routes it to the authoritative action surface and refuses the
//! switch, which the last test pins so the builder never grows it back.)

use mush_core::{Audience, GOD, ObjId};
use mush_jobs::JobsModule;
use mush_proto::{NarrativeKind, OutputEvent};
use mush_ruleset::is_roll_narration;
use mush_testkit::TestWorld;
use ruleset_wod::WodRuleset;

fn world() -> (TestWorld, ObjId) {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(4242)),
    ]);
    let nick = tw.spawn_player("Nick");
    tw.run(GOD, "@sheet/set Nick/dexterity=3");
    tw.run(GOD, "@sheet/set Nick/brawl=2");
    tw.run(GOD, "@sheet/set Nick/willpower=5");
    (tw, nick)
}

/// The room's roll lines, plain.
fn roll_lines(events: &[(Audience, OutputEvent)]) -> Vec<String> {
    events
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (Audience::Room { .. }, OutputEvent::Narrative { kind, text, .. })
                if *kind == NarrativeKind::Emit && is_roll_narration(text) =>
            {
                Some(text.to_plain_text())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_builders_plain_line_rolls_the_pool_at_the_difficulty() {
    let (mut tw, nick) = world();
    let ev = tw.run(nick, "+roll dexterity+brawl vs 7");
    let lines = roll_lines(&ev);
    assert_eq!(lines.len(), 1, "{ev:?}");
    assert!(
        lines[0].starts_with("roll  Nick  dexterity+brawl  "),
        "{}",
        lines[0]
    );
    assert!(lines[0].ends_with("  diff 7"), "{}", lines[0]);
}

#[test]
fn the_builders_spec_switch_round_trips_and_wp_is_refused() {
    let (mut tw, nick) = world();
    // `/spec` needs a recorded specialty on an ability in the pool.
    tw.run(nick, "+spec brawl=Kickboxing");
    let ev = tw.run(nick, "+roll/spec dexterity+brawl vs 6");
    let lines = roll_lines(&ev);
    assert_eq!(lines.len(), 1, "{ev:?}");
    assert!(
        lines[0].starts_with("roll  Nick  dexterity+brawl  "),
        "{}",
        lines[0]
    );
    assert!(lines[0].ends_with("  diff 6"), "{}", lines[0]);
    let ev = tw.run(nick, "+roll/wp dexterity+brawl vs 6");
    assert!(roll_lines(&ev).is_empty(), "{ev:?}");
    let seen = tw.seen_by(&ev, nick);
    assert!(
        seen.iter()
            .any(|l| l.contains("authoritative action surface")),
        "{seen:?}"
    );
}

#[test]
fn a_single_trait_and_the_band_edges_parse() {
    let (mut tw, nick) = world();
    for line in ["+roll dexterity vs 3", "+roll dexterity vs 9"] {
        let ev = tw.run(nick, line);
        assert_eq!(roll_lines(&ev).len(), 1, "{line}: {ev:?}");
    }
}
