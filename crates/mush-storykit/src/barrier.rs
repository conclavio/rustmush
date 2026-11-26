//! **Barriers: wait for the server to say it is there** (doc 51 §3.3, rule C).
//!
//! A story that runs a real server cannot drive its clock, and a story that
//! sleeps to let the server get somewhere is guessing how fast the box is. A
//! barrier asks the server instead. Each one here types a line a staff
//! character may type, over the story's own session, and returns when the
//! answer shows the server has got there:
//!
//! | Barrier | Waits until | Reads |
//! |---|---|---|
//! | [`settled`] | the world has answered everything sent before it | the sentinel's refusal |
//! | [`arena_wake`] | the match has pulsed at least once more | `frames F` on `@arena/status` |
//! | [`arena_seconds`] | the arena's own clock has advanced `n` seconds | `clock m:ss` on `@arena/status` |
//! | [`at_one_wake`] | a wake the story has seen is just behind it | a staff floor, then [`arena_wake`] |
//!
//! **Absence.** "Nothing was told" cannot be waited for. An absence is
//! asserted between two barriers, never after a sleep: send, call
//! [`settled`], then assert on the transcript between the marks.
//!
//! The session is the story rig's own terminal, reached through [`Session`].
//! The kit's one `Term` arrives with doc 51's units FL.7 and FL.8; until then
//! each rig's terminal implements the trait.

use std::future::Future;

use crate::wait::{STEP_TIMEOUT, wait_for, wait_until};

// A whole reply whose try counts its answers is `crate::reply::ask` (unit
// FL.5). `ask_whole` below is the same read since unit FL.8, answering text.

/// A line the parser refuses, typed to mark the end of a reply. The `zzq-`
/// opening is the mushd rig's convention: its readers that walk a reply line by
/// line drop a line that opens so as the reader's own traffic, wherever the
/// terminal's echo of it lands.
pub const SENTINEL: &str = "zzq-sentinel";

/// The parser's refusal: the sentinel's answer.
pub const SENTINEL_REFUSAL: &str = "Huh?";

/// The last line of `@arena/status`: the readout is whole once it has come.
const STATUS_END: &str = "ledger writes awaiting";

/// `@arena/status` with nothing on the board, which has no last line.
const STATUS_EMPTY: &str = "No match is running.";

/// **One line-mode session of a story**: what a barrier types on and reads
/// from. A rig's terminal implements it over its own channel.
pub trait Session {
    /// The offset of "now" in what the session has printed.
    fn mark(&self) -> usize;

    /// Type one line.
    fn send_line(&mut self, line: &str) -> impl Future<Output = ()>;

    /// Wait until one of `needles` shows in what was printed after `mark`,
    /// with ANSI sequences dropped, and answer which. It fails the story when
    /// the rig's step limit passes first.
    fn expect_one_of_after(&mut self, mark: usize, needles: &[&str])
    -> impl Future<Output = usize>;

    /// What was printed after `mark`, with ANSI sequences dropped.
    fn seen_after(&self, mark: usize) -> String;
}

/// The needles of a sentinel read: its refusal first, then the busy answers.
fn sentinel_needles() -> Vec<&'static str> {
    // The kit's one table of busy sentences (unit FL.5, `crate::busy`).
    let mut needles = vec![SENTINEL_REFUSAL];
    needles.extend(crate::busy::busy_needles());
    needles
}

/// **Everything sent to `term` so far is in its scrollback.** The world
/// answers a session's lines in the order it took them, from one ordered
/// outbox, so once the sentinel's refusal has arrived nothing sent earlier is
/// still in flight. A busy answer is typed again.
///
/// It is the barrier for an absence: mark, act, `settled`, then assert that
/// the transcript after the mark does not hold what must not have been told.
pub async fn settled(term: &mut impl Session) {
    let needles = sentinel_needles();
    wait_until(STEP_TIMEOUT, "the sentinel to be refused", async || {
        let mark = term.mark();
        term.send_line(SENTINEL).await;
        term.expect_one_of_after(mark, &needles).await == 0
    })
    .await;
}

/// **A whole reply, never half of one**: `line`, then the sentinel, and
/// everything printed through the sentinel's refusal. Text is parsed only
/// from a read like this one (doc 51 rule S).
///
/// A line refused busy is typed again, and **its tries count their answers**
/// (unit FL.8, over FL.5's [`crate::ask`]): a try that was refused waits for
/// its sentinel's own answer before the line is typed again, so that answer
/// can never end the next try's read early. FL.3's first version shared that
/// hazard with mushd's twelve private copies.
pub async fn ask_whole(term: &mut impl Session, line: &str) -> String {
    crate::reply::whole_through(term, line).await
}

/// One match's row of `@arena/status`, with the terminal's wrapping undone:
/// from `match <id>:` to the row's `frames N`.
fn status_row(status: &str, id: u64) -> Option<String> {
    let flat = status.split_whitespace().collect::<Vec<&str>>().join(" ");
    let (_, row) = flat.split_once(&format!("match {id}: "))?;
    let row = row.split(" orders: ").next()?;
    Some(row.to_owned())
}

/// The digits a field of the row opens with.
fn leading_number(text: &str) -> Option<u64> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// **The matches an `@arena/status` readout has a row for**, in the readout's
/// order. A story with one match on the board reads its number here.
#[must_use]
pub fn status_matches(status: &str) -> Vec<u64> {
    let flat = status.split_whitespace().collect::<Vec<&str>>().join(" ");
    flat.split("match ")
        .skip(1)
        .filter_map(|after| {
            let (id, rest) = after.split_once(':')?;
            // A row's own opening: `match 7: running, tempo ...`.
            rest.contains(", tempo ").then(|| id.parse().ok())?
        })
        .collect()
}

/// **Match `id`'s frame count** off an `@arena/status` readout: it moves once
/// a pulse's frame has reached the world.
#[must_use]
pub fn status_frames(status: &str, id: u64) -> Option<u64> {
    let row = status_row(status, id)?;
    leading_number(row.split_once(" frames ")?.1)
}

/// **Match `id`'s own clock**, in whole seconds, off an `@arena/status`
/// readout's `clock m:ss`.
#[must_use]
pub fn status_clock_seconds(status: &str, id: u64) -> Option<u64> {
    let row = status_row(status, id)?;
    let clock = row.split_once(", clock ")?.1;
    let (minutes, seconds) = clock.split_once(':')?;
    Some(minutes.parse::<u64>().ok()? * 60 + leading_number(seconds)?)
}

/// **`@arena/status`, read whole**: the readout ends on a line of its own
/// (`ledger writes awaiting …`), so the read runs to that line and needs no
/// sentinel behind it. One line a read, which matters to a wait that asks
/// five times a second: the session's flood bucket refills at ten.
///
/// `Err` is what came instead: an empty board, or a busy answer.
async fn arena_status(staff: &mut impl Session) -> Result<String, String> {
    let mut needles = vec![STATUS_END, STATUS_EMPTY];
    needles.extend(crate::busy::busy_needles());
    let mark = staff.mark();
    staff.send_line("@arena/status").await;
    let answered = staff.expect_one_of_after(mark, &needles).await;
    let status = staff.seen_after(mark);
    if answered == 0 {
        Ok(status)
    } else {
        Err(status)
    }
}

/// One number off match `id`'s row of `@arena/status`, read whole and asked
/// again until the readout carries the row.
async fn status_number(
    staff: &mut impl Session,
    id: u64,
    what: &str,
    read: fn(&str, u64) -> Option<u64>,
) -> u64 {
    wait_for(
        STEP_TIMEOUT,
        &format!("`@arena/status` to give match {id}'s {what}"),
        async || {
            let status = arena_status(staff).await?;
            read(&status, id).ok_or(status)
        },
    )
    .await
}

/// **The one match on the board**: the number of `@arena/status`'s first row,
/// asked until the readout has a row. For a story that runs one match at a
/// time and did not start it by number.
///
/// # Panics
///
/// When no readout inside the step limit has a row.
pub async fn the_match(staff: &mut impl Session) -> u64 {
    wait_for(
        STEP_TIMEOUT,
        "`@arena/status` to have a match's row",
        async || {
            let status = arena_status(staff).await?;
            status_matches(&status).first().copied().ok_or(status)
        },
    )
    .await
}

/// **Match `id`'s frame count now**, read off the staff readout.
///
/// # Panics
///
/// When no readout inside the step limit carries the match's row.
pub async fn arena_frames(staff: &mut impl Session, id: u64) -> u64 {
    status_number(staff, id, "frame count", status_frames).await
}

/// **Wait until match `id` has pulsed at least once more**: its frame count
/// on `@arena/status` has moved past what it reads now. An order that had
/// reached the arena before the call has been taken by then, and what that
/// pulse told a cockpit is on its way.
pub async fn arena_wake(staff: &mut impl Session, id: u64) {
    let from = arena_frames(staff, id).await;
    wait_until(STEP_TIMEOUT, "the arena's next wake", async || {
        arena_frames(staff, id).await > from
    })
    .await;
}

/// **Wait until match `id`'s own clock has advanced `seconds`**. It waits for
/// simulation time, not for the wall: on a loaded box a wall-clock sleep of
/// the same length may schedule too few pulses.
pub async fn arena_seconds(staff: &mut impl Session, id: u64, seconds: u64) {
    let from = status_number(staff, id, "clock", status_clock_seconds).await;
    wait_until(STEP_TIMEOUT, "arena time to advance", async || {
        status_number(staff, id, "clock", status_clock_seconds).await >= from + seconds
    })
    .await;
}

/// **Send lines so that they land at one arena wake.** Two lines that must be
/// taken by the same pulse (a shot and the move that spoils it) cannot simply
/// be typed back to back: a wake that falls between their arrivals takes one
/// and not the other.
///
/// This reads the match's frame count, sets a staff floor of `floor_ms` on
/// it, which pulls the next wake in to now, waits until the frame count has
/// passed the reading, and then runs `send`. The wake the story has just seen
/// is behind it and the next is a whole floor away, so everything `send`
/// types waits in the task's inbox for that one. The floor stays on: lift it
/// with `@arena/tempo <id>=off` once the lines' wake has been seen.
///
/// `floor_ms` is a staff floor as `@arena/tempo` takes one: a whole multiple
/// of the story's engaged period. Make it long against a round trip of the
/// session: the lines are safe for the floor less the time it took to see
/// the wake and type them.
///
/// # Panics
///
/// When staff's `@arena/tempo` is refused.
pub async fn at_one_wake<S: Session>(
    staff: &mut S,
    id: u64,
    floor_ms: u32,
    send: impl AsyncFnOnce(&mut S),
) {
    let from = arena_frames(staff, id).await;
    set_floor(staff, id, floor_ms).await;
    wait_until(STEP_TIMEOUT, "the wake the floor pulled in", async || {
        arena_frames(staff, id).await > from
    })
    .await;
    send(staff).await;
}

/// **Put match `id` under a staff floor of `floor_ms`** (`@arena/tempo`): every
/// wake is then at least that far from the last. A busy answer is typed again.
///
/// # Panics
///
/// When the floor is refused: the failure prints what staff read.
pub async fn set_floor(staff: &mut impl Session, id: u64, floor_ms: u32) {
    let floor = format!("under a floor of {floor_ms} ms");
    let mut needles = vec![floor.as_str(), "Usage:", "is not running", "has ended"];
    needles.extend(crate::busy::busy_needles());
    wait_for(
        STEP_TIMEOUT,
        &format!("a floor of {floor_ms} ms on match {id}"),
        async || {
            let mark = staff.mark();
            staff
                .send_line(&format!("@arena/tempo {id}={floor_ms}"))
                .await;
            match staff.expect_one_of_after(mark, &needles).await {
                0 => Ok(()),
                _ => Err(staff.seen_after(mark)),
            }
        },
    )
    .await;
}

/// **Lift match `id`'s staff floor**: it runs at its own tempo again.
pub async fn lift_floor(staff: &mut impl Session, id: u64) {
    let mut needles = vec!["its own tempo again"];
    needles.extend(crate::busy::busy_needles());
    wait_for(
        STEP_TIMEOUT,
        &format!("match {id}'s floor to lift"),
        async || {
            let mark = staff.mark();
            staff.send_line(&format!("@arena/tempo {id}=off")).await;
            match staff.expect_one_of_after(mark, &needles).await {
                0 => Ok(()),
                _ => Err(staff.seen_after(mark)),
            }
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A scripted session: each typed line is answered from a queue, and the
    /// last answer again once the queue is empty.
    struct Script {
        seen: String,
        typed: Vec<String>,
        answers: VecDeque<String>,
        last: String,
    }

    impl Script {
        fn new(answers: &[&str]) -> Script {
            Script {
                seen: String::new(),
                typed: Vec::new(),
                answers: answers.iter().map(|text| (*text).to_owned()).collect(),
                last: String::new(),
            }
        }
    }

    impl Session for Script {
        fn mark(&self) -> usize {
            self.seen.len()
        }

        /// A queued answer that opens `busy:` is given to whatever line comes
        /// next; otherwise the sentinel is refused and any other line takes
        /// the next answer.
        async fn send_line(&mut self, line: &str) {
            self.typed.push(line.to_owned());
            let busy = self
                .answers
                .front()
                .is_some_and(|next| next.starts_with("busy:"));
            let answer = if busy {
                let busy = self.answers.pop_front().unwrap_or_default();
                busy.trim_start_matches("busy:").to_owned()
            } else if line == SENTINEL {
                format!("{SENTINEL_REFUSAL}  (Type \"help\" for help.)")
            } else {
                if let Some(next) = self.answers.pop_front() {
                    self.last = next;
                }
                self.last.clone()
            };
            self.seen.push_str(line);
            self.seen.push('\n');
            self.seen.push_str(&answer);
            self.seen.push('\n');
        }

        async fn expect_one_of_after(&mut self, mark: usize, needles: &[&str]) -> usize {
            let text = &self.seen[mark..];
            needles
                .iter()
                .enumerate()
                .filter_map(|(index, needle)| text.find(needle).map(|at| (at, index)))
                .min()
                .map(|(_, index)| index)
                .unwrap_or_else(|| panic!("none of {needles:?} in {text:?}"))
        }

        fn seen_after(&self, mark: usize) -> String {
            self.seen[mark..].to_owned()
        }
    }

    /// A readout as the server prints it at eighty columns: the row wrapped,
    /// its `orders` and `leaders` lines under it, the ledger line last.
    fn status(id: u64, clock: &str, frames: u64) -> String {
        format!(
            "match {id}: running, tempo engaged, arena 43101 ms, clock {clock}, lagged 0, chunks\n\
             3 appended / 0 lost, frames\n{frames}\n  orders: 0 by pilots, 0 by lance leaders\n  \
             leaders: none\nledger writes awaiting the store's acknowledgement: 0"
        )
    }

    #[test]
    fn a_row_is_read_across_the_terminals_wrapping() {
        let text = status(7, "1:05", 412);
        assert_eq!(status_frames(&text, 7), Some(412));
        assert_eq!(status_clock_seconds(&text, 7), Some(65));
        assert_eq!(status_frames(&text, 8), None, "another match's row");
        let two = format!("{}\n{}", status(1, "0:09", 90), status(2, "2:00", 5));
        assert_eq!(status_frames(&two, 1), Some(90));
        assert_eq!(status_clock_seconds(&two, 2), Some(120));
        assert_eq!(status_frames("No match is running.", 1), None);
        assert_eq!(status_matches(&two), [1, 2]);
        assert_eq!(status_matches(&text), [7]);
        assert!(status_matches("No match is running.").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn settled_types_the_sentinel_again_over_a_busy_answer() {
        let mut term = Script::new(&[
            "busy:The world is committing another action; please retry in a moment.",
        ]);
        settled(&mut term).await;
        assert_eq!(term.typed, [SENTINEL, SENTINEL]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_whole_reply_ends_at_the_sentinels_refusal() {
        let mut term = Script::new(&["a row\nanother row"]);
        let reply = ask_whole(&mut term, "CONTACTS").await;
        assert!(reply.contains("a row\nanother row"), "{reply}");
        assert!(reply.contains(SENTINEL_REFUSAL), "{reply}");
        assert_eq!(term.typed, ["CONTACTS", SENTINEL]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_wake_is_waited_for_on_the_frame_count() {
        let (one, two) = (status(1, "0:03", 30), status(1, "0:03", 31));
        let mut term = Script::new(&[&one, &one, &one, &two]);
        arena_wake(&mut term, 1).await;
        assert_eq!(
            term.typed
                .iter()
                .filter(|line| *line == "@arena/status")
                .count(),
            4,
            "asked until the count moved"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn arena_seconds_waits_on_the_arenas_own_clock() {
        let readouts = [
            status(1, "0:58", 1),
            status(1, "0:59", 2),
            status(1, "1:00", 3),
            status(1, "1:01", 4),
        ];
        let refs: Vec<&str> = readouts.iter().map(String::as_str).collect();
        let mut term = Script::new(&refs);
        arena_seconds(&mut term, 1, 3).await;
        assert!(term.answers.is_empty(), "every readout was needed");
    }

    #[tokio::test(start_paused = true)]
    async fn lines_for_one_wake_go_out_after_the_floors_own_wake() {
        let (before, after) = (status(4, "0:10", 100), status(4, "0:10", 101));
        let mut term = Script::new(&[
            &before,
            "Match 4 under a floor of 5000 ms: engaged 400 ms of arena time a wake every 5000 ms",
            &before,
            &after,
        ]);
        at_one_wake(&mut term, 4, 5_000, async |staff| {
            staff.send_line("@arena/place #9=1,1").await;
        })
        .await;
        assert_eq!(
            term.typed,
            [
                "@arena/status",
                "@arena/tempo 4=5000",
                "@arena/status",
                "@arena/status",
                "@arena/place #9=1,1",
            ],
            "one line a readout, and the story's line after the wake"
        );
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "to give match 3's frame count did not happen within 60s")]
    async fn a_match_with_no_row_fails_with_the_readout() {
        let mut term = Script::new(&[STATUS_EMPTY]);
        let _ = arena_frames(&mut term, 3).await;
    }
}
