//! **One terminal for a story** (doc 51 §3.4 and §3.5, unit FL.7): a
//! line-mode SSH session and everything it has printed.
//!
//! Before FL.7 the gateway's stories carried ten copies of one `Term` struct,
//! one a file, each with its own `strip_ansi`, its own busy list and its own
//! limits. This is the one copy. A rig opens a session with [`Term::play`]
//! (or wraps a channel it opened itself with [`Term::new`]) and the story
//! reads through it.
//!
//! Two kinds of read, as rule S has them:
//!
//! - **A needle read** ([`Term::expect_after`], [`Term::say`]) returns when
//!   one string has arrived. It is for waiting.
//! - **A whole read** ([`crate::ask_whole`], [`crate::settled`], which take a
//!   `Term` as their [`Session`]) returns when the reply is known to be over.
//!   Text is parsed only from one of these.
//!
//! And one helper of rule P, [`Term::across_restart`]: the held session's
//! side of a restart, bounded by the restarted half's own schedule rather
//! than a span chosen at the call.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use russh::ChannelMsg;

use crate::barrier::{BUSY, Session, settled};
use crate::wait::{STEP_TIMEOUT, loaded};

/// How many times [`Term::say`] types a line again on a busy answer before
/// the story fails. Each try waits for an answer, so this counts answers, not
/// time.
pub const BUSY_RETRIES: usize = 8;

/// What a gateway tells a held session when its server half comes back and
/// the session's input is safe again (`mush_gateway`'s reconnected notice).
pub const RECONNECTED: &str = "Reconnected";

/// What a gateway tells a held session when its server half goes away.
pub const RESTARTING: &str = "restarting";

/// The terminal size a story's session asks for: the owner's floor of 120
/// columns, never 80.
pub const COLS: u32 = 120;
/// Rows of [`COLS`]' terminal.
pub const ROWS: u32 = 40;

/// The SSH client handler of a story: it trusts any host key, since every
/// server a story dials is its own.
#[derive(Clone, Copy, Debug, Default)]
pub struct AcceptAll;

impl russh::client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// `s` with its ANSI CSI sequences (`ESC [ … final`) and lone escapes
/// dropped: what a player reads, not what the terminal was told.
#[must_use]
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// One player's terminal: an SSH channel and everything it has printed.
pub struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

impl std::fmt::Debug for Term {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Term")
            .field("seen_bytes", &self.seen.len())
            .finish_non_exhaustive()
    }
}

/// What ended a read early.
enum Stop {
    /// The limit passed.
    Timeout,
    /// The server closed the channel.
    Closed,
}

impl Term {
    /// A terminal over a channel the rig opened (a shell, a pty requested).
    #[must_use]
    pub fn new(channel: russh::Channel<russh::client::Msg>) -> Term {
        Term {
            channel,
            seen: String::new(),
        }
    }

    /// **A player at the door**: connect to `addr` as the passwordless
    /// `play` user, ask for a [`COLS`] by [`ROWS`] pty and a shell. The handle
    /// is the SSH connection; dropping it ends the session.
    pub async fn play(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
        Term::play_sized(addr, COLS, ROWS).await
    }

    /// [`Term::play`] with a terminal of `cols` by `rows`, for a story about
    /// the screen's size.
    pub async fn play_sized(
        addr: SocketAddr,
        cols: u32,
        rows: u32,
    ) -> (russh::client::Handle<AcceptAll>, Term) {
        let config = Arc::new(russh::client::Config::default());
        let mut handle = match russh::client::connect(config, addr, AcceptAll).await {
            Ok(handle) => handle,
            Err(e) => panic!("connect to {addr}: {e}"),
        };
        match handle.authenticate_none("play").await {
            Ok(auth) if auth.success() => {}
            Ok(_) => panic!("`play` must be accepted with no auth at {addr}"),
            Err(e) => panic!("auth-none at {addr}: {e}"),
        }
        let channel = match handle.channel_open_session().await {
            Ok(channel) => channel,
            Err(e) => panic!("open a session channel at {addr}: {e}"),
        };
        if let Err(e) = channel
            .request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
            .await
        {
            panic!("request a pty at {addr}: {e}");
        }
        if let Err(e) = channel.request_shell(true).await {
            panic!("request a shell at {addr}: {e}");
        }
        (handle, Term::new(channel))
    }

    /// The offset of "now" in the raw stream: a read after it sees only what
    /// arrives from here on.
    #[must_use]
    pub fn mark(&self) -> usize {
        self.seen.len()
    }

    /// Everything printed so far, raw (ANSI kept).
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.seen
    }

    /// Everything printed after `mark`, ANSI dropped.
    #[must_use]
    pub fn since(&self, mark: usize) -> String {
        strip_ansi(self.seen.get(mark..).unwrap_or(""))
    }

    /// Type one line.
    pub async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        if let Err(e) = self.channel.data(&bytes[..]).await {
            panic!("send {line:?}: {e}; seen: {:?}", self.seen);
        }
    }

    /// Read until one of `needles` shows after `mark` or `limit` passes.
    async fn read_until(
        &mut self,
        mark: usize,
        needles: &[&str],
        limit: Duration,
    ) -> Result<usize, Stop> {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let seen = self.since(mark);
            if let Some(index) = needles.iter().position(|needle| seen.contains(needle)) {
                return Ok(index);
            }
            match tokio::time::timeout_at(deadline, self.channel.wait()).await {
                Err(_) => return Err(Stop::Timeout),
                Ok(None | Some(ChannelMsg::Close)) => return Err(Stop::Closed),
                Ok(Some(ChannelMsg::Data { data })) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Ok(Some(_)) => {}
            }
        }
    }

    /// Wait until one of `needles` shows after `mark`, within `limit`, and
    /// answer which. The failure prints the whole transcript.
    pub async fn expect_one_of_within(
        &mut self,
        mark: usize,
        needles: &[&str],
        limit: Duration,
    ) -> usize {
        assert!(!needles.is_empty(), "at least one needle is required");
        match self.read_until(mark, needles, limit).await {
            Ok(index) => index,
            Err(Stop::Timeout) => panic!(
                "timeout waiting for any of {needles:?} within {limit:?}; seen: {:?}",
                self.seen
            ),
            Err(Stop::Closed) => panic!(
                "channel closed waiting for any of {needles:?}; seen: {:?}",
                self.seen
            ),
        }
    }

    /// Wait until one of `needles` shows after `mark`, under
    /// [`STEP_TIMEOUT`], and answer which.
    pub async fn expect_one_of_after(&mut self, mark: usize, needles: &[&str]) -> usize {
        self.expect_one_of_within(mark, needles, STEP_TIMEOUT).await
    }

    /// Wait until `needle` shows after `mark`, under [`STEP_TIMEOUT`].
    pub async fn expect_after(&mut self, mark: usize, needle: &str) {
        self.expect_one_of_within(mark, &[needle], STEP_TIMEOUT)
            .await;
    }

    /// Wait until `needle` shows anywhere in the transcript.
    pub async fn expect(&mut self, needle: &str) {
        self.expect_after(0, needle).await;
    }

    /// **Type `line` and wait for `needle`**, typing it again while the world
    /// gives a busy answer ([`BUSY`]): a commit in flight, or the control
    /// lease's 1 Hz mirror. Those are real answers a real client retries, not
    /// failures. A needle read: it waits, it returns nothing to parse.
    ///
    /// The needle must not be in the line itself, or the line's own echo
    /// would satisfy it before the world had seen the line.
    pub async fn say(&mut self, line: &str, needle: &str) {
        assert!(
            !line.contains(needle),
            "`{needle}` appears in `{line}`'s own echo; pick a needle from the answer"
        );
        let mut needles = vec![needle];
        needles.extend(BUSY);
        for _ in 0..BUSY_RETRIES {
            let mark = self.mark();
            self.send_line(line).await;
            if self.expect_one_of_after(mark, &needles).await == 0 {
                return;
            }
        }
        panic!(
            "`{line}` had a busy answer {BUSY_RETRIES} times running; seen: {:?}",
            self.seen
        );
    }

    /// [`Term::say`], answering what was printed from the line on, ANSI
    /// dropped. It stops at the needle, so the reply may not be whole: parse
    /// only what the needle itself proves has arrived, or read with
    /// [`crate::ask_whole`].
    pub async fn ask(&mut self, line: &str, needle: &str) -> String {
        let mark = self.mark();
        self.say(line, needle).await;
        self.since(mark)
    }

    /// Wait until the server closes the channel, under [`STEP_TIMEOUT`].
    pub async fn expect_close(&mut self) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            match tokio::time::timeout_at(deadline, self.channel.wait()).await {
                Err(_) => panic!(
                    "timeout waiting for the channel to close; seen: {:?}",
                    self.seen
                ),
                Ok(None | Some(ChannelMsg::Close)) => return,
                Ok(Some(ChannelMsg::Data { data })) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Ok(Some(_)) => {}
            }
        }
    }

    /// **The held session's side of a restart** (doc 51 §3.5, rule P).
    ///
    /// `restart` is the rig's: it stops the half and starts it again, and
    /// returns when the new one has bound (the rig's own bound event, under
    /// its own limit). From the moment it returns, the rest is the other
    /// half's to do on its own schedule: notice, redial, re-attach. `heal`
    /// is the longest that schedule takes once there is something to dial,
    /// read from the product (the gateway's `HEAL_WITHIN`), and the wait for
    /// [`RECONNECTED`] is that under [`LOAD_FACTOR`](crate::LOAD_FACTOR).
    /// Then [`settled`]: the session has had everything the fresh world
    /// owed it before the story types on.
    ///
    /// It also proves the session was told: [`RESTARTING`] must show after
    /// the mark taken before the restart began, and before [`RECONNECTED`].
    /// Answers the mark taken before the restart, for the story's own reads.
    pub async fn across_restart(
        &mut self,
        heal: Duration,
        restart: impl Future<Output = ()>,
    ) -> usize {
        let mark = self.mark();
        restart.await;
        self.healed_after(mark, heal).await;
        mark
    }

    /// The tail of [`Term::across_restart`] for a story whose restart is not
    /// one future (a stop, something in the gap, then a start): from `mark`,
    /// taken before the half went away, [`RESTARTING`] then [`RECONNECTED`]
    /// within `heal` under the load factor, then [`settled`]. Call it once
    /// the rig's start has returned.
    pub async fn healed_after(&mut self, mark: usize, heal: Duration) {
        self.reconnected_after(mark, heal).await;
        settled(self).await;
    }

    /// [`Term::healed_after`] without its closing [`settled`]: for a story
    /// whose next read must not hold the sentinel's answer (a session that
    /// frames every reply, say).
    pub async fn reconnected_after(&mut self, mark: usize, heal: Duration) {
        self.expect_after(mark, RESTARTING).await;
        let limit = loaded(heal);
        match self.read_until(mark, &[RECONNECTED], limit).await {
            Ok(_) => {}
            Err(Stop::Timeout) => panic!(
                "the session was not reconnected within {limit:?} of the restarted half \
                 binding (its heal schedule, {heal:?}, under the load factor); seen: {:?}",
                self.seen
            ),
            Err(Stop::Closed) => panic!(
                "the session was closed across the restart; seen: {:?}",
                self.seen
            ),
        }
        let since = self.since(mark);
        assert!(
            since.find(RESTARTING) < since.find(RECONNECTED),
            "the session was told {RESTARTING:?} before {RECONNECTED:?}; seen: {since:?}"
        );
    }
}

impl Session for Term {
    fn mark(&self) -> usize {
        Term::mark(self)
    }

    fn send_line(&mut self, line: &str) -> impl Future<Output = ()> {
        Term::send_line(self, line)
    }

    fn expect_one_of_after(
        &mut self,
        mark: usize,
        needles: &[&str],
    ) -> impl Future<Output = usize> {
        Term::expect_one_of_after(self, mark, needles)
    }

    fn seen_after(&self, mark: usize) -> String {
        self.since(mark)
    }
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn strip_ansi_drops_csi_sequences_and_lone_escapes() {
        assert_eq!(strip_ansi("\u{1b}[32mConnected\u{1b}[0m."), "Connected.");
        assert_eq!(strip_ansi("a\u{1b}[1;36mb\u{1b}[0mc"), "abc");
        assert_eq!(strip_ansi("lone\u{1b}x"), "lonex");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn strip_ansi_keeps_text_after_an_unterminated_sequence_out() {
        assert_eq!(strip_ansi("ok\u{1b}[12"), "ok");
    }
}
