//! The one-line roll narration every ruleset shares (interface plan I3).
//!
//! A public roll reaches the room as `Narrative { kind: Emit }` whose spans
//! follow one fixed layout, so line mode prints it as it is and a screen
//! client (the TUI, the web) can lift the leading `roll` span into its
//! speaker gutter:
//!
//! ```text
//! roll  Mireille  perception+alertness  8 6 4 3 = 2 successes  diff 6
//! ```
//!
//! Spans, in order: the marker `roll` (tagged [`ROLL_TAG`]), a [`ROLL_GAP`],
//! the roller's name (tagged `player`), and one body span — `  <expr>
//! <dice> = <result>` followed by every note as `  <note>`. A contested
//! roll repeats the name and body for the opponent, joined by `vs`, and
//! ends with `=> <verdict>`. Names come from the world, so they are the only
//! spans a player can influence; the marker and the gaps are fixed, which
//! is what lets a client trust the first span.
//!
//! The classic sentence (`Name rolls expr (n dice, diff d) and rolled … ->
//! …`) is unchanged everywhere it is a record rather than a line to read:
//! the roll log, the `.roll` panel's `summary`, the scene ledger, a job
//! comment, and the private and silent deliveries to the roller.

use mush_proto::{RichText, Span};

/// The semantic tag on a roll line's first span.
pub const ROLL_TAG: &str = "roll";
/// The column gap inside a roll line: two spaces.
pub const ROLL_GAP: &str = "  ";

/// One side of a roll: who threw which pool, what came up, and what it
/// amounted to. Every field is display text the caller already has.
#[derive(Clone, Copy, Debug)]
pub struct RollSide<'a> {
    /// The roller's display name.
    pub name: &'a str,
    /// The pool as typed or resolved — `strength+brawl`, `Agility+Pistols`,
    /// `Celerity (dexterity+athletics)`, `5`.
    pub expr: &'a str,
    /// The dice as thrown — `8 6 4 3`, `7 7 +WP`.
    pub dice: &'a str,
    /// The classified result — `2 successes`, `botch`, `4 hits`.
    pub result: &'a str,
}

/// The narration for one roll: `roll  <name>  <expr>  <dice> = <result>`
/// then `  <note>` for each note (`diff 6`, `[wound -2]`, `[Physical limit
/// 5]`, …).
#[must_use]
pub fn roll_narration(side: &RollSide<'_>, notes: &[String]) -> RichText {
    let mut body = side_body(side);
    push_notes(&mut body, notes);
    RichText(vec![
        Span::tagged(ROLL_TAG, ROLL_TAG),
        Span::plain(ROLL_GAP),
        Span::tagged("player", side.name),
        Span::plain(body),
    ])
}

/// The narration for a contested roll: the actor's side, `vs`, the
/// opponent's side, then `=> <verdict>` and the notes.
#[must_use]
pub fn contested_narration(
    actor: &RollSide<'_>,
    opponent: &RollSide<'_>,
    verdict: &str,
    notes: &[String],
) -> RichText {
    let mut a_body = side_body(actor);
    a_body.push_str(ROLL_GAP);
    a_body.push_str("vs");
    a_body.push_str(ROLL_GAP);
    let mut b_body = side_body(opponent);
    b_body.push_str(ROLL_GAP);
    b_body.push_str("=> ");
    b_body.push_str(verdict);
    push_notes(&mut b_body, notes);
    RichText(vec![
        Span::tagged(ROLL_TAG, ROLL_TAG),
        Span::plain(ROLL_GAP),
        Span::tagged("player", actor.name),
        Span::plain(a_body),
        Span::tagged("player", opponent.name),
        Span::plain(b_body),
    ])
}

/// Whether `text` is a roll line: its first span carries [`ROLL_TAG`].
#[must_use]
pub fn is_roll_narration(text: &RichText) -> bool {
    text.spans()
        .first()
        .is_some_and(|s| s.tag.as_deref() == Some(ROLL_TAG))
}

fn side_body(side: &RollSide<'_>) -> String {
    format!(
        "{ROLL_GAP}{}{ROLL_GAP}{} = {}",
        side.expr, side.dice, side.result
    )
}

fn push_notes(body: &mut String, notes: &[String]) {
    for note in notes {
        body.push_str(ROLL_GAP);
        body.push_str(note);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mireille() -> RollSide<'static> {
        RollSide {
            name: "Mireille",
            expr: "perception+alertness",
            dice: "8 6 4 3",
            result: "2 successes",
        }
    }

    #[test]
    fn a_roll_line_has_the_fixed_span_layout() {
        let text = roll_narration(&mireille(), &["diff 6".to_string()]);
        let spans = text.spans();
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].tag.as_deref(), Some("roll"));
        assert_eq!(spans[0].text, "roll");
        assert_eq!(spans[1].text, "  ");
        assert_eq!(spans[2].tag.as_deref(), Some("player"));
        assert_eq!(spans[2].text, "Mireille");
        assert_eq!(
            spans[3].text,
            "  perception+alertness  8 6 4 3 = 2 successes  diff 6"
        );
        assert_eq!(
            text.to_plain_text(),
            "roll  Mireille  perception+alertness  8 6 4 3 = 2 successes  diff 6"
        );
        assert!(is_roll_narration(&text));
        assert!(!is_roll_narration(&RichText::plain("roll  fake")));
    }

    #[test]
    fn notes_follow_in_order_and_none_is_fine() {
        let text = roll_narration(&mireille(), &[]);
        assert_eq!(
            text.to_plain_text(),
            "roll  Mireille  perception+alertness  8 6 4 3 = 2 successes"
        );
        let text = roll_narration(
            &mireille(),
            &["diff 6".to_string(), "[wound -2]".to_string()],
        );
        assert!(
            text.to_plain_text()
                .ends_with("= 2 successes  diff 6  [wound -2]")
        );
    }

    #[test]
    fn a_contested_line_names_both_sides_and_the_verdict() {
        let vlad = RollSide {
            name: "Vlad",
            expr: "reaction+intuition",
            dice: "3 3 1",
            result: "failure",
        };
        let text = contested_narration(&mireille(), &vlad, "Mireille wins", &["diff 6".into()]);
        assert_eq!(
            text.to_plain_text(),
            "roll  Mireille  perception+alertness  8 6 4 3 = 2 successes  vs  \
             Vlad  reaction+intuition  3 3 1 = failure  => Mireille wins  diff 6"
        );
        let spans = text.spans();
        assert_eq!(spans.len(), 6);
        assert_eq!(spans[0].tag.as_deref(), Some("roll"));
        assert_eq!(spans[2].tag.as_deref(), Some("player"));
        assert_eq!(spans[4].tag.as_deref(), Some("player"));
        assert_eq!(spans[4].text, "Vlad");
        assert!(is_roll_narration(&text));
    }
}
