//! One negotiated side channel belongs to one authenticated SSH connection.
use super::*;
use crate::resume::{self, Failure, Hint, MAX_FRAME, Reply, Request};
use zeroize::Zeroizing;

#[derive(Default)]
pub(super) struct Cockpit {
    pub hint: Option<Hint>,
    pub channel: Option<ChannelId>,
    pub started: bool,
    pub pending: Option<Request>,
    /// None means the SSH or welcome account has not authenticated yet.
    /// A requested name that is absent from its owned roster is refused.
    pub authenticated: Option<(String, Result<Option<u64>, Failure>)>,
    fallback: Option<Fallback>,
    used: bool,
    require_playing: bool,
    prompted: bool,
    bytes: Zeroizing<Vec<u8>>,
    timer: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Cockpit {
    fn drop(&mut self) {
        self.cancel_timer();
        self.stop_fallback();
    }
}

/// Bounds how long a hint alone can defer an ordinary zero-step login.
const REDEEM_WAIT: Duration = Duration::from_secs(3);
struct Fallback {
    waiting: Arc<Mutex<bool>>,
    task: tokio::task::JoinHandle<()>,
}

impl Cockpit {
    pub fn can_open(&self, playing: bool) -> bool {
        self.hint.is_some()
            && self.channel.is_none()
            && !self.used
            && (!self.require_playing || playing)
    }
    pub fn start_subsystem(&mut self) -> bool {
        if self.started {
            return false;
        }
        self.started = true;
        // A partial ticket must never be left in a freed allocation on growth.
        self.bytes.reserve_exact(4 + MAX_FRAME);
        true
    }
    pub(super) fn cancel_fallback(&mut self) {
        if let Some(fallback) = &self.fallback {
            let mut waiting = fallback.waiting.lock().expect("login timer poisoned");
            if *waiting {
                *waiting = false;
                fallback.task.abort();
            }
        }
    }

    pub(super) fn stop_fallback(&mut self) {
        self.cancel_fallback();
        if let Some(fallback) = self.fallback.take() {
            fallback.task.abort();
        }
    }

    /// A failed or abandoned redemption may be followed by one fresh issue.
    fn allow_issue(&mut self, require_playing: bool) {
        if let Some(hint) = &mut self.hint
            && hint.mode == resume::Mode::Redeem
        {
            hint.mode = resume::Mode::Issue;
            self.require_playing = require_playing;
            self.used = false;
            self.started = false;
            self.prompted = false;
        }
    }

    pub fn waiting_for_redeem(&self) -> bool {
        self.hint
            .as_ref()
            .is_some_and(|h| h.mode == resume::Mode::Redeem)
    }
    pub fn open(&mut self, id: ChannelId, handle: russh::server::Handle) {
        self.channel = Some(id);
        self.timer = Some(tokio::spawn(async move {
            tokio::time::sleep(SFTP_HANDSHAKE_TIMEOUT).await;
            let _ = tokio::time::timeout(WRITE_TIMEOUT, handle.close(id)).await;
        }));
    }
    /// Two auxiliary channels can be opened before either subsystem is
    /// requested. Their requests assign the roles; open order is immaterial.
    pub(super) fn match_pending(
        &mut self,
        id: ChannelId,
        name: &str,
        sftp: &mut Option<ChannelId>,
        sftp_timer: &mut Option<tokio::task::JoinHandle<()>>,
    ) {
        if !self.started
            && self.channel.is_some()
            && sftp.is_some()
            && sftp_timer.is_some()
            && ((name == resume::SUBSYSTEM && *sftp == Some(id))
                || (name == "sftp" && self.channel == Some(id)))
        {
            std::mem::swap(&mut self.channel, sftp);
            std::mem::swap(&mut self.timer, sftp_timer);
        }
    }
    fn cancel_timer(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.abort();
        }
    }
    pub fn release_candidate(&mut self) {
        self.channel = None;
        self.cancel_timer();
    }
    pub fn finish(&mut self) {
        self.release_candidate();
        self.pending = None;
        self.bytes = Zeroizing::new(Vec::new());
        self.used = true;
    }
    pub fn abandon_closed(&mut self) {
        self.finish();
        self.allow_issue(true);
    }
    pub fn abandon(&mut self, session: &mut Session) {
        self.close(session);
        self.allow_issue(true);
    }
    pub fn close(&mut self, session: &mut Session) {
        if let Some(channel) = self.channel {
            let _ = session.close(channel);
        }
        self.finish();
    }
    fn feed(&mut self, part: &[u8]) -> Result<bool, ()> {
        if !self.started
            || self.pending.is_some()
            || self.bytes.len().saturating_add(part.len()) > 4 + MAX_FRAME
        {
            return Err(());
        }
        self.bytes.extend_from_slice(part);
        let Some(header) = self.bytes.get(..4) else {
            return Ok(false);
        };
        let n = u32::from_be_bytes(header.try_into().map_err(|_| ())?) as usize;
        if n == 0 || n > MAX_FRAME || self.bytes.len() > 4 + n {
            return Err(());
        }
        if self.bytes.len() < 4 + n {
            return Ok(false);
        }
        let payload = self.bytes.get(4..).ok_or(())?;
        let request = resume::decode_request(payload).map_err(|_| ())?;
        self.bytes = Zeroizing::new(Vec::new());
        self.pending = Some(request);
        self.cancel_timer();
        Ok(true)
    }
}

pub(super) struct OutputQueues {
    pub events: mpsc::Receiver<Arc<OutputEvent>>,
    pub stamped: mpsc::Receiver<Arc<mush_proto::Envelope>>,
}

impl ConnHandler {
    pub(super) fn defer_login(
        &mut self,
        session: &Session,
        account: String,
        character: CharSummary,
    ) -> Result<(), russh::Error> {
        self.cockpit.stop_fallback();
        let player = self.player(session)?;
        let (mut conn, queues) = self.output_queues();
        let waiting = Arc::new(Mutex::new(true));
        let gate = Arc::clone(&waiting);
        let task = tokio::spawn(async move {
            tokio::time::sleep(REDEEM_WAIT).await;
            {
                // Cancellation and claiming connected are one operation with
                // respect to handler teardown or a complete sideband request.
                let mut waiting = gate.lock().expect("login timer poisoned");
                if !std::mem::take(&mut *waiting)
                    || player
                        .connected
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_err()
                {
                    return;
                }
            }
            conn.caps = player.worker.lock().caps;
            match player
                .worker
                .hooks
                .connect_character(conn, &account, character.key)
                .await
            {
                Ok(()) => {
                    let _ = player.activate(&account, &character, queues, None, |_, _| Ok(()));
                }
                Err(why) => {
                    player.connected.store(false, Ordering::SeqCst);
                    let _ = player
                        .worker
                        .write_sys(&format!("Connection failed: {why}"))
                        .await;
                }
            }
        });
        self.cockpit.fallback = Some(Fallback { waiting, task });
        Ok(())
    }

    pub(super) fn output_queues(&self) -> (NewConn, OutputQueues) {
        let (tx, events) = mpsc::channel(OUTPUT_BUFFER);
        let (stamped_tx, stamped) = mpsc::channel(OUTPUT_BUFFER);
        (
            NewConn {
                id: self.conn_id,
                addr: self.addr,
                caps: self.lock().caps,
                tx,
                stamped_tx: Some(stamped_tx),
            },
            OutputQueues { events, stamped },
        )
    }

    fn cockpit_reply(
        &mut self,
        session: &mut Session,
        reply: Reply,
        final_reply: bool,
    ) -> Result<(), russh::Error> {
        if let Some(channel) = self.cockpit.channel {
            let bytes = resume::encode_reply(&reply).map_err(|_| russh::Error::Disconnect)?;
            session.data(channel, bytes::Bytes::from_owner(bytes))?;
            if final_reply {
                session.eof(channel)?;
                self.cockpit.close(session);
            }
        }
        Ok(())
    }

    pub(super) async fn cockpit_data(
        &mut self,
        session: &mut Session,
        part: &[u8],
    ) -> Result<(), russh::Error> {
        match self.cockpit.feed(part) {
            Ok(false) => Ok(()),
            Ok(true) => self.try_cockpit(session).await,
            Err(()) => {
                self.cockpit_reply(session, Reply::failure(Failure::Denied), true)?;
                self.cockpit.allow_issue(true);
                Ok(())
            }
        }
    }

    pub(super) async fn try_cockpit(&mut self, session: &mut Session) -> Result<(), russh::Error> {
        let Some(request) = self.cockpit.pending.take() else {
            return Ok(());
        };
        let Some(hint) = self.cockpit.hint.as_ref() else {
            return Ok(());
        };
        let playing = self.lock().playing.clone();
        // A hint can expire without a request. A player admitted by the
        // ordinary path can now ask for a first lease on the same pane.
        let issue_after_login = playing.is_some() && request.mode == resume::Mode::Issue;
        if request.pane != hint.pane || (request.mode != hint.mode && !issue_after_login) {
            self.cockpit_reply(session, Reply::failure(Failure::Denied), true)?;
            self.cockpit.allow_issue(true);
            return Ok(());
        }
        let mut allow_issue = false;
        let reply = match request.mode {
            resume::Mode::Issue => {
                if let Some((account, ch, _)) = &playing {
                    Some(
                        match self
                            .hooks
                            .cockpit_issue(self.conn_id, &request.pane, account, *ch)
                            .await
                        {
                            Ok((ticket, grace)) => {
                                self.hooks
                                    .cockpit_transport(self.conn_id, Arc::clone(&self.kill));
                                self.lock().record_view(self.hooks.as_ref(), self.conn_id);
                                Reply::issued(ticket, grace)
                            }
                            Err(e) => Reply::failure(e),
                        },
                    )
                } else {
                    None
                }
            }
            resume::Mode::Redeem => {
                self.cockpit.cancel_fallback();
                if playing.is_some() {
                    allow_issue = true;
                    Some(Reply::failure(Failure::Denied))
                } else if let Some((account, selected)) = self.cockpit.authenticated.clone() {
                    let result = if let (Ok(selected), Some(ticket)) = (selected, &request.ticket) {
                        if self
                            .connected
                            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                            .is_err()
                        {
                            Err(Failure::Busy)
                        } else {
                            let (conn, queues) = self.output_queues();
                            match self
                                .hooks
                                .cockpit_redeem(conn, &request.pane, &account, selected, ticket)
                                .await
                            {
                                Ok((ch, ticket, grace, view)) => {
                                    self.hooks
                                        .cockpit_transport(self.conn_id, Arc::clone(&self.kill));
                                    let character = match &self.lock().mode {
                                        Mode::CharSelect { chars, .. } => {
                                            chars.iter().find(|c| c.key == ch).cloned()
                                        }
                                        _ => None,
                                    }
                                    .unwrap_or_else(|| CharSummary {
                                        key: ch,
                                        name: ch.to_string(),
                                    });
                                    self.player(session)?.activate(
                                        &account,
                                        &character,
                                        queues,
                                        Some(view),
                                        |id, bytes| session.data(id, bytes),
                                    )?;
                                    Ok(Reply::redeemed(ticket, grace))
                                }
                                Err(error) => {
                                    self.connected.store(false, Ordering::SeqCst);
                                    Err(error)
                                }
                            }
                        }
                    } else {
                        Err(Failure::Denied)
                    };
                    allow_issue = result.is_err();
                    Some(result.unwrap_or_else(Reply::failure))
                } else {
                    None
                }
            }
        };
        if let Some(reply) = reply {
            self.cockpit_reply(session, reply, true)?;
            if allow_issue {
                // A client may park its issue request while the user chooses a character.
                self.cockpit.allow_issue(false);
            }
            return Ok(());
        }
        self.cockpit.pending = Some(request);
        if !self.cockpit.prompted {
            self.cockpit.prompted = true;
            self.cockpit_reply(session, Reply::auth_required(), false)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started() -> Cockpit {
        let mut state = Cockpit::default();
        assert!(state.start_subsystem());
        state
    }

    #[test]
    fn an_incomplete_or_second_frame_cannot_be_dispatched() {
        let mut state = started();
        let body = br#"{"v":1,"op":"issue","pane":"0123456789abcdef0123456789abcdef"}"#;
        let header = (body.len() as u32).to_be_bytes();
        assert_eq!(state.feed(&header), Ok(false));
        let allocation = state.bytes.as_ptr();
        for byte in body.iter().take(body.len() - 1) {
            assert_eq!(state.feed(std::slice::from_ref(byte)), Ok(false));
            assert!(state.pending.is_none());
            assert_eq!(state.bytes.as_ptr(), allocation);
        }
        assert_eq!(state.feed(b"}"), Ok(true));
        assert!(state.pending.is_some());
        assert_eq!(state.feed(&header), Err(()));
    }

    #[test]
    fn oversized_or_trailing_bytes_are_rejected_before_dispatch() {
        let mut state = started();
        assert_eq!(state.feed(&((MAX_FRAME + 1) as u32).to_be_bytes()), Err(()));
        assert!(state.pending.is_none());
        let mut state = started();
        assert_eq!(state.feed(&[0, 0, 0, 1, b'{', b'}']), Err(()));
        assert!(state.pending.is_none());
    }
}
