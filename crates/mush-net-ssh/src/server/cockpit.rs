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
    pub playing: Option<(String, u64)>,
    used: bool,
    prompted: bool,
    bytes: Zeroizing<Vec<u8>>,
    timer: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Cockpit {
    fn drop(&mut self) {
        self.cancel_timer();
    }
}

impl Cockpit {
    pub fn can_open(&self) -> bool {
        self.hint.is_some() && self.channel.is_none() && !self.used
    }
    pub fn start_subsystem(&mut self) -> bool {
        !std::mem::replace(&mut self.started, true)
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
            session.data(channel, bytes)?;
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
            Err(()) => self.cockpit_reply(session, Reply::failure(Failure::Denied), true),
        }
    }

    pub(super) async fn try_cockpit(&mut self, session: &mut Session) -> Result<(), russh::Error> {
        let Some(request) = self.cockpit.pending.take() else {
            return Ok(());
        };
        let Some(hint) = self.cockpit.hint.as_ref() else {
            return Ok(());
        };
        if request.mode != hint.mode || request.pane != hint.pane {
            return self.cockpit_reply(session, Reply::failure(Failure::Denied), true);
        }
        let reply = match request.mode {
            resume::Mode::Issue => {
                if let Some((account, ch)) = &self.cockpit.playing {
                    Some(
                        match self
                            .hooks
                            .cockpit_issue(self.conn_id, &request.pane, account, *ch)
                            .await
                        {
                            Ok((ticket, grace)) => Reply::issued(ticket, grace),
                            Err(e) => Reply::failure(e),
                        },
                    )
                } else {
                    None
                }
            }
            resume::Mode::Redeem => {
                if self.cockpit.playing.is_some() {
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
                                Ok((ch, ticket, grace)) => {
                                    self.activate_player(session, &account, ch, queues, true)?;
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
                    Some(result.unwrap_or_else(Reply::failure))
                } else {
                    None
                }
            }
        };
        if let Some(reply) = reply {
            return self.cockpit_reply(session, reply, true);
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
        for byte in body.iter().take(body.len() - 1) {
            assert_eq!(state.feed(std::slice::from_ref(byte)), Ok(false));
            assert!(state.pending.is_none());
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
