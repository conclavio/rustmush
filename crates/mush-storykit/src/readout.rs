//! **What mushd's stories read beyond the gateway's** (doc 51 §3.3 and §3.4,
//! unit FL.8): the rest of mushd's own terminal, moved onto the kit's
//! [`Term`], and the world-tick barrier.
//!
//! Before FL.8 mushd's `tests/support` kept its own `Term`, and three story
//! files a further private one each; fifteen `sim_lenses` files carried their
//! own `say`, twelve their own `ask` and ten their own `ask_whole`. Those are
//! the kit's now. What mushd's terminal had that the gateway's did not is
//! here:
//!
//! - [`Term::ask_settled`]: a needle read made whole. It is what a story's
//!   `ask(term, line, needle)` becomes when the text it returns is parsed.
//! - [`Term::expect_squeezed_after`] and [`Term::resize`], for the `@tui`
//!   screen, which paints only the cells that change.
//! - [`Term::sent_at`], the instants the session typed its lines, from which
//!   mushd's rig reads its flood bucket's room on the server's own type.
//! - [`after_ticks`]: the world's scheduler has fired a coarse tick `n` more
//!   times.

use std::time::{Duration, Instant};

use russh::ChannelMsg;

use crate::barrier::{SENTINEL, SENTINEL_REFUSAL, Session, ask_whole, settled};
use crate::busy::busy_needles;
use crate::term::{Term, strip_ansi};
use crate::wait::{STEP_TIMEOUT, loaded};

/// `text` without what [`settled`] added to a read: every echo of the
/// sentinel, the busy or flood answers of a sentinel typed again, and the
/// last refusal. Only lines after the first echo can be the sentinel's
/// answers, since the line itself was answered (its needle came) before the
/// sentinel was typed.
fn without_sentinel(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let first = lines.iter().position(|line| line.trim() == SENTINEL);
    let after = |at: usize| first.is_some_and(|first| at > first);
    let refusal = lines
        .iter()
        .enumerate()
        .rposition(|(at, line)| after(at) && line.trim().starts_with(SENTINEL_REFUSAL));
    lines
        .iter()
        .enumerate()
        .filter(|(at, line)| {
            line.trim() != SENTINEL
                && Some(*at) != refusal
                && !(after(*at) && crate::busy::is_busy(line))
        })
        .map(|(_, line)| *line)
        .collect()
}

/// **The session's flood bucket refused the line** (unit FL.8): a server
/// drops a line typed faster than the session's rate allows, and says this.
/// It is a refusal for a moment, like [`crate::BUSY_SENTENCES`], and the
/// kit's readers type the line again after it, a token's refill later.
///
/// It is not a row of that table: the table is also the bot's list of
/// sentences it types a line again after (mush-bot's
/// `the_bots_busy_sentences_are_the_story_kits`), and the bot's readers do
/// not meet the bucket at its pace. A whole read types two lines where a
/// needle read typed one, which is how mushd's stories met it.
pub const FLOODED: crate::BusySentence = crate::BusySentence {
    ends: "You're doing that too fast",
    sources: &[
        "mush-net-ssh/src/server.rs",
        "mush-gateway/src/bridge_common.rs",
    ],
};

/// What [`after_ticks`] has staff `think` from the world's scheduler. It opens
/// `zzq-` like the kit's sentinel, so a rig's line readers drop it the same
/// way.
pub const TICK_MARK: &str = "zzq-tick";

impl Term {
    /// **Type `line`, wait for `needle`, and answer the whole of what came**
    /// (doc 51 rule S): [`Term::say`], then [`settled`], and everything
    /// printed from the line on, ANSI dropped, through the sentinel's refusal.
    ///
    /// [`Term::ask`] stops at the needle, so its text may be half a reply.
    /// Here the sentinel is typed only once the needle has come, and the
    /// world answers a session's lines in the order it took them, so the
    /// line's own reply is all in. The needle may be an event that follows
    /// the line (a pulse's notice) as well as a row of its reply, which is
    /// what [`crate::ask_whole`] alone cannot wait for.
    ///
    /// The text is the line's echo and its reply: the sentinel's own echo and
    /// its refusal are left out, wherever the echo landed among the reply's
    /// lines.
    pub async fn ask_settled(&mut self, line: &str, needle: &str) -> String {
        let mark = self.say_from(line, needle).await;
        settled(self).await;
        without_sentinel(&self.since(mark))
    }

    /// Wait for `needle` after `mark` in the text read the way a full-screen
    /// paint has to be read: ANSI dropped and every whitespace character
    /// squeezed out, so `needle` is written without spaces (`Nolock`).
    ///
    /// The `@tui` screen paints only the cells that change, so a run of blanks
    /// that was blank before goes out as a cursor jump: `No lock` can reach the
    /// wire as `No`, a jump, `lock`, which [`Term::expect_after`] never sees.
    /// Squeezed, the two spellings are one. Under [`STEP_TIMEOUT`].
    pub async fn expect_squeezed_after(&mut self, mark: usize, needle: &str) {
        let squeezed = |raw: &str| -> String {
            strip_ansi(raw)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect()
        };
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        while !squeezed(self.seen.get(mark..).unwrap_or("")).contains(needle) {
            match tokio::time::timeout_at(deadline, self.channel.wait()).await {
                Err(_) => panic!(
                    "timeout waiting for {needle:?} squeezed; seen: {:?}",
                    self.seen
                ),
                Ok(None | Some(ChannelMsg::Close)) => panic!(
                    "channel closed waiting for {needle:?} squeezed; seen: {:?}",
                    self.seen
                ),
                Ok(Some(ChannelMsg::Data { data })) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Ok(Some(_)) => {}
            }
        }
    }

    /// Resize the pty to `cols` by `rows`: the SSH window-change a terminal
    /// sends when its window is resized (16b cut I.2). In `@tui` the server
    /// clears and repaints every cell at the new size, which is how a story
    /// reads a whole frame.
    pub async fn resize(&mut self, cols: u32, rows: u32) {
        if let Err(e) = self.channel.window_change(cols, rows, 0, 0).await {
            panic!("window change to {cols}x{rows}: {e}");
        }
    }

    /// The instants this terminal typed its lines, in order. A rig replays
    /// them through the server's own flood bucket to know how much room its
    /// session has before a battery of lines, in place of a sleep.
    #[must_use]
    pub fn sent_at(&self) -> &[Instant] {
        &self.sent
    }
}

/// **The world's scheduler has fired a coarse tick of `period` at least
/// `ticks` more times** (doc 51 §3.3, `after_tick`).
///
/// `staff` queues `think` [`TICK_MARK`] with `@wait` for one whole second more
/// than `ticks` periods, and waits for the thought. The scheduler fires its
/// entries in the order they fall due, a module's tick among them, so by the
/// time the queued line is answered every tick due before it has fired: at
/// least `ticks` of them, wherever in its period the tick stood when the line
/// was queued. Then it is a barrier for "nothing more was said after two
/// more ticks", where a story slept the span.
///
/// Doc 51 sketched this barrier over `@ps`'s next-fire readout. That readout
/// prints whole seconds, and a one-second tick (tacsim's boarding tick) reads
/// `next in 0s` at every instant, so it cannot tell one fire from the next.
/// The scheduler's own order can, with no product change (Q-51-11).
///
/// The queued line is a `think`, not a nonsense word: a line the scheduler
/// runs has no connection to answer, so a parser refusal of it reaches no
/// one, while a thought is told to the actor's every connection.
pub async fn after_ticks(staff: &mut impl Session, period: Duration, ticks: u32) {
    let span = period.saturating_mul(ticks);
    let whole = span.as_secs() + u64::from(span.subsec_nanos() > 0) + 1;
    // Queued whole, so a busy refusal of the `@wait` is typed again. Its echo
    // is in that read, so from the mark on the mark's word is the thought.
    ask_whole(staff, &format!("@wait {whole}=think {TICK_MARK}")).await;
    let mark = staff.mark();
    let mut needles = vec![TICK_MARK];
    needles.extend(busy_needles());
    // The thought, or a busy answer to the queued line, which says as much
    // (the scheduler reached it).
    let limit = loaded(Duration::from_secs(whole));
    assert!(
        limit <= STEP_TIMEOUT,
        "after_ticks waits {limit:?}, more than one step ({STEP_TIMEOUT:?}); wait fewer ticks"
    );
    staff.expect_one_of_after(mark, &needles).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session whose scheduler is a list of lines it answers later.
    #[derive(Default)]
    struct Staff {
        seen: String,
        typed: Vec<String>,
        queued: Vec<String>,
    }

    impl Session for Staff {
        fn mark(&self) -> usize {
            self.seen.len()
        }

        async fn send_line(&mut self, line: &str) {
            self.typed.push(line.to_owned());
            self.seen.push_str(line);
            self.seen.push_str("\r\n");
            if let Some((_, queued)) = line.split_once('=') {
                self.queued
                    .push(queued.trim_start_matches("think ").to_owned());
            } else {
                self.seen.push_str(SENTINEL_REFUSAL);
                self.seen.push_str("\r\n");
            }
        }

        async fn expect_one_of_after(&mut self, mark: usize, needles: &[&str]) -> usize {
            if self.seen.get(mark..).is_some_and(|t| !t.contains("Huh?")) {
                // The scheduler reaches the queued line.
                for thought in self.queued.drain(..) {
                    self.seen.push_str(&thought);
                    self.seen.push_str("\r\n");
                }
            }
            let tail = self.seen.get(mark..).unwrap_or("");
            needles
                .iter()
                .position(|needle| tail.contains(needle))
                .unwrap_or_else(|| panic!("nothing answered {needles:?}"))
        }

        fn seen_after(&self, mark: usize) -> String {
            self.seen.get(mark..).unwrap_or("").to_owned()
        }
    }

    #[test]
    fn a_settled_read_leaves_out_the_sentinel_wherever_its_echo_landed() {
        let text =
            "LOCK ZZ\r\nzzq-sentinel\r\nNo such contact.\r\nHuh?  (Type \"help\" for help.)\r\n";
        assert_eq!(without_sentinel(text), "LOCK ZZ\r\nNo such contact.\r\n");
        // A refusal that answered the line itself is kept.
        let text = "zz\r\nHuh?\r\nzzq-sentinel\r\nHuh?\r\n";
        assert_eq!(without_sentinel(text), "zz\r\nHuh?\r\n");
        // A sentinel the flood bucket refused, then the one that was answered.
        let text = "ORDER x\r\nNo such squad.\r\nzzq-sentinel\r\nYou're doing that too fast.\r\n\
                    zzq-sentinel\r\nHuh?\r\n";
        assert_eq!(without_sentinel(text), "ORDER x\r\nNo such squad.\r\n");
    }

    #[test]
    fn the_flood_sentence_is_still_written_where_it_says() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        for source in FLOODED.sources {
            let text = std::fs::read_to_string(crates.join(source)).unwrap_or_default();
            assert!(text.contains(FLOODED.ends), "{source}: {:?}", FLOODED.ends);
        }
        assert!(crate::is_busy("You're doing that too fast."));
        assert!(crate::busy_needles().contains(&FLOODED.ends));
    }

    #[tokio::test(start_paused = true)]
    async fn two_one_second_ticks_queue_a_line_three_seconds_out() {
        let mut staff = Staff::default();
        after_ticks(&mut staff, Duration::from_secs(1), 2).await;
        assert_eq!(
            staff.typed.first().map(String::as_str),
            Some("@wait 3=think zzq-tick")
        );
        assert!(staff.queued.is_empty(), "the queued line was answered");
    }

    #[tokio::test(start_paused = true)]
    async fn a_part_second_rounds_up_to_the_next_whole_second() {
        let mut staff = Staff::default();
        after_ticks(&mut staff, Duration::from_millis(250), 3).await;
        assert_eq!(
            staff.typed.first().map(String::as_str),
            Some("@wait 2=think zzq-tick")
        );
    }
}
