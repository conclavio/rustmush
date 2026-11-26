//! **A whole reply, and what a story reads out of it** (doc 51 §3.4, rule S;
//! unit FL.5).
//!
//! Text is parsed only from a reply the reader knows is whole. [`ask`] types a
//! line and then the kit's [`SENTINEL`], and returns everything printed before
//! the sentinel's refusal: the world answers a session's lines in the order it
//! took them, so the refusal marks the end of the reply. [`Reply`] is that
//! text, with the readers a story needs, so a story does not slice its
//! scrollback by hand.
//!
//! **Answers are counted.** Every line typed gets exactly one answer, and a
//! busy refusal ([`crate::busy`]) can be the answer to either of the two. FL.3
//! left one hazard in the kit's first `ask_whole`: when the line was refused
//! busy, the sentinel's own refusal was still to come, and it ended the next
//! try's read before that try's answer had arrived. Here a try is over only
//! when both answers are in:
//!
//! - the refusal comes first: the line was answered, and nothing is owed;
//! - a busy answer comes first, after the line's own answer: the line ran and
//!   the sentinel was refused, and nothing is owed;
//! - a busy answer comes first, with nothing before it: the line was refused,
//!   and the sentinel's answer is owed. The try waits for it, then types the
//!   line again.
//!
//! The second and third are told apart by whether anything but the echoes of
//! the two typed lines came before the busy answer, so `ask` is for a line
//! whose answer prints something, which every read does. A line refused busy
//! is typed again, so it is also for a line that is safe to type twice.

use crate::barrier::{SENTINEL, SENTINEL_REFUSAL, Session};
use crate::busy::{busy_needles, is_busy};
use crate::wait::{STEP_TIMEOUT, wait_until};

/// **One whole reply**: what a server printed in answer to one typed line,
/// with the echoes of the line and the sentinel left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    asked: String,
    text: String,
}

impl Reply {
    /// The reply to `asked`, from the text printed after it was typed and
    /// before the sentinel's refusal.
    #[must_use]
    pub fn new(asked: &str, text: &str) -> Reply {
        Reply {
            asked: asked.to_owned(),
            text: text.to_owned(),
        }
    }

    /// The line this answers.
    #[must_use]
    pub fn asked(&self) -> &str {
        &self.asked
    }

    /// Everything printed, as it came, the echoes included.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The reply's lines, carriage returns dropped, with the echo of the line
    /// asked and of any sentinel (a line that opens `zzq-`) left out.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        let asked = self.asked.trim();
        self.text
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .filter(move |line| {
                let body = line.trim();
                !body.starts_with("zzq-") && (asked.is_empty() || body != asked)
            })
    }

    /// Whether any line holds `needle`.
    #[must_use]
    pub fn contains(&self, needle: &str) -> bool {
        self.lines().any(|line| line.contains(needle))
    }

    /// **The first line that opens with `prefix`**, leading spaces aside.
    #[must_use]
    pub fn row(&self, prefix: &str) -> Option<&str> {
        self.lines()
            .map(str::trim_start)
            .find(|line| line.starts_with(prefix))
    }

    /// **Every line that opens with `prefix`**, leading spaces aside.
    #[must_use]
    pub fn rows(&self, prefix: &str) -> Vec<&str> {
        self.lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with(prefix))
            .collect()
    }

    /// **The section headed `name`**: the line that opens with `name`
    /// (leading spaces aside), and the lines after it indented deeper than it,
    /// to the first that is not. A readout that prints a heading and its rows
    /// under it reads this way.
    #[must_use]
    pub fn section(&self, name: &str) -> Option<Reply> {
        let lines: Vec<&str> = self.lines().collect();
        let at = lines
            .iter()
            .position(|line| line.trim_start().starts_with(name))?;
        let head = lines.get(at)?;
        let depth = indent(head);
        let mut text = String::from(*head);
        for line in lines.iter().skip(at + 1) {
            if line.trim().is_empty() || indent(line) <= depth {
                break;
            }
            text.push('\n');
            text.push_str(line);
        }
        Some(Reply {
            asked: String::new(),
            text,
        })
    }

    /// **The first whole number after `label`**, in the first line that holds
    /// it: `number_after("fuel")` reads `40` from `fuel 40 of 60`. A leading
    /// `-` counts; thousands separators (`,`) inside the number are dropped.
    #[must_use]
    pub fn number_after(&self, label: &str) -> Option<i64> {
        let line = self.lines().find(|line| line.contains(label))?;
        let (_, rest) = line.split_once(label)?;
        let rest = rest.trim_start_matches(|c: char| !c.is_ascii_digit() && c != '-');
        let negative = rest.starts_with('-');
        let digits: String = rest
            .trim_start_matches('-')
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == ',')
            .filter(char::is_ascii_digit)
            .collect();
        let value: i64 = digits.parse().ok()?;
        Some(if negative { -value } else { value })
    }
}

impl std::fmt::Display for Reply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Where the first answer-shaped event lies in `text` from byte `from`: the
/// sentinel's refusal (`true`) or a busy line (`false`), and its byte offset.
fn next_answer(text: &str, from: usize) -> Option<(usize, bool)> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if start < from {
            continue;
        }
        let body = line.trim();
        if body == SENTINEL_REFUSAL || body.starts_with(&format!("{SENTINEL_REFUSAL} ")) {
            return Some((start, true));
        }
        if is_busy(body) {
            return Some((start, false));
        }
    }
    None
}

/// Whether anything but the echoes of `asked` and the sentinel came in `text`.
fn answered(asked: &str, text: &str) -> bool {
    text.lines().any(|line| {
        let body = line.trim();
        !body.is_empty() && !body.starts_with("zzq-") && body != asked.trim()
    })
}

/// How one try ended.
enum Try {
    /// The whole reply.
    Whole(String),
    /// The line was refused busy and both answers are in.
    Refused,
}

/// The needles that say an answer may have come: the refusal and the busy
/// sentences. Which it was is judged line by line.
fn answer_needles() -> Vec<&'static str> {
    let mut needles = vec![SENTINEL_REFUSAL];
    needles.extend(busy_needles());
    needles
}

async fn one_try(term: &mut impl Session, line: &str) -> Try {
    let needles = answer_needles();
    let mark = term.mark();
    term.send_line(line).await;
    term.send_line(SENTINEL).await;
    // The first answer-shaped line after the two were typed.
    let first = loop {
        term.expect_one_of_after(mark, &needles).await;
        let text = term.seen_after(mark);
        if let Some(found) = next_answer(&text, 0) {
            break (found, text);
        }
        // A needle inside a longer line that is not an answer: wait for more.
        let later = term.mark();
        term.expect_one_of_after(later, &needles).await;
    };
    let ((at, refusal), text) = first;
    let before = text.get(..at).unwrap_or("");
    if refusal || answered(line, before) {
        return Try::Whole(before.to_owned());
    }
    // Refused busy with nothing before it: the sentinel's answer is owed.
    let after = at
        + text
            .get(at..)
            .and_then(|t| t.find('\n'))
            .map_or(0, |n| n + 1);
    loop {
        let later = term.mark();
        let text = term.seen_after(mark);
        if next_answer(&text, after).is_some() {
            return Try::Refused;
        }
        term.expect_one_of_after(later, &needles).await;
    }
}

/// **A whole reply, never half of one**: `line`, then the sentinel, and
/// everything printed before the sentinel's refusal, as a [`Reply`]. A line
/// refused busy is typed again once both of its try's answers are in (the
/// module head), inside the rig's step limit.
pub async fn ask(term: &mut impl Session, line: &str) -> Reply {
    let mut reply = None;
    wait_until(
        STEP_TIMEOUT,
        &format!("`{line}` to be answered whole"),
        async || match one_try(term, line).await {
            Try::Whole(text) => {
                reply = Some(text);
                true
            }
            Try::Refused => false,
        },
    )
    .await;
    Reply::new(line, &reply.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A session that answers each typed line with the next scripted answer:
    /// `busy` is the world's busy refusal, `huh` the parser's, anything else
    /// is printed as it is. Every typed line is echoed first.
    struct Script {
        answers: VecDeque<&'static str>,
        seen: String,
        typed: Vec<String>,
    }

    impl Script {
        fn new(answers: &[&'static str]) -> Script {
            Script {
                answers: answers.iter().copied().collect(),
                seen: String::new(),
                typed: Vec::new(),
            }
        }
    }

    const BUSY_LINE: &str = "The world is committing another action; please retry in a moment.";

    impl Session for Script {
        fn mark(&self) -> usize {
            self.seen.len()
        }

        async fn send_line(&mut self, line: &str) {
            self.typed.push(line.to_owned());
            self.seen.push_str(line);
            self.seen.push_str("\r\n");
            let answer = self.answers.pop_front().unwrap_or("huh");
            let text = match answer {
                "busy" => BUSY_LINE,
                "huh" => SENTINEL_REFUSAL,
                other => other,
            };
            self.seen.push_str(text);
            self.seen.push_str("\r\n");
        }

        async fn expect_one_of_after(&mut self, mark: usize, needles: &[&str]) -> usize {
            let tail = self.seen.get(mark..).unwrap_or("");
            needles
                .iter()
                .position(|needle| tail.contains(needle))
                .unwrap_or_else(|| panic!("the script ran dry waiting for {needles:?}"))
        }

        fn seen_after(&self, mark: usize) -> String {
            self.seen.get(mark..).unwrap_or("").to_owned()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_reply_ends_at_the_sentinels_refusal() {
        let mut term = Script::new(&["Rig 1:\r\n  fuel 40 of 60", "huh"]);
        let reply = ask(&mut term, "+tv/1").await;
        assert_eq!(
            reply.lines().collect::<Vec<_>>(),
            ["Rig 1:", "  fuel 40 of 60"]
        );
        assert_eq!(reply.number_after("fuel"), Some(40));
        assert_eq!(term.typed, ["+tv/1", SENTINEL]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_line_is_typed_again_once_the_sentinel_has_answered() {
        // The line refused busy, then its sentinel's refusal: the next try
        // must not end on that refusal.
        let mut term = Script::new(&["busy", "huh", "Rig 1:\r\n  fuel 40 of 60", "huh"]);
        let reply = ask(&mut term, "+tv/1").await;
        assert_eq!(reply.row("fuel"), Some("fuel 40 of 60"));
        assert_eq!(term.typed, ["+tv/1", SENTINEL, "+tv/1", SENTINEL]);
    }

    #[tokio::test(start_paused = true)]
    async fn both_refused_is_one_refused_try() {
        let mut term = Script::new(&["busy", "busy", "Rig 1: idle", "huh"]);
        let reply = ask(&mut term, "+tv/1").await;
        assert!(reply.contains("Rig 1: idle"), "{reply}");
        assert_eq!(term.typed.len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_sentinel_after_an_answer_is_the_end_of_the_reply() {
        // The line ran; the sentinel was the one refused. Nothing is owed, and
        // the line is not typed again.
        let mut term = Script::new(&["Rig 1: idle", "busy"]);
        let reply = ask(&mut term, "+tv/1").await;
        assert!(reply.contains("Rig 1: idle"), "{reply}");
        assert_eq!(term.typed, ["+tv/1", SENTINEL]);
    }

    #[test]
    fn a_section_is_its_heading_and_the_rows_under_it() {
        let reply = Reply::new(
            "STATUS",
            "STATUS\r\nhull: 90 of 100\r\n  front 30\r\n  rear 20\r\nturret: 150 of 150\r\n  \
             inside 150\r\n",
        );
        let hull = reply.section("hull").expect("the hull section");
        assert_eq!(
            hull.lines().collect::<Vec<_>>(),
            ["hull: 90 of 100", "  front 30", "  rear 20"]
        );
        assert_eq!(reply.section("turret").map(|t| t.lines().count()), Some(2));
        assert_eq!(reply.section("engine"), None);
        assert_eq!(reply.number_after("hull:"), Some(90));
        assert_eq!(
            reply.rows("  ").len(),
            0,
            "rows are read with leading spaces aside"
        );
    }

    #[test]
    fn a_number_reads_its_sign_and_drops_its_separators() {
        let reply = Reply::new("", "Balance: -1,250 cr\r\nBid 7 of 9");
        assert_eq!(reply.number_after("Balance:"), Some(-1_250));
        assert_eq!(reply.number_after("Bid"), Some(7));
        assert_eq!(reply.number_after("Ask"), None);
    }
}
