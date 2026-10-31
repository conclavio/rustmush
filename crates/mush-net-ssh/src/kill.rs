//! A stream that can be cut from outside.
//!
//! russh runs each session in a task of its own, and that task cannot be
//! aborted from outside: a client that stalls in the key exchange holds the
//! loop in a socket read, a client that stopped reading holds it in a socket
//! write, and a client that ignores the server's `SSH_MSG_DISCONNECT` holds
//! it in the read-until-EOF that russh does after the half-close
//! (`inactivity_timeout` is `None` by design: in-game idlers are furniture).
//! A polite disconnect queues behind the very thing that is stuck. The
//! deadlines and every server-initiated disconnect therefore cut the
//! **stream**: once killed, every read and write fails, the session task
//! ends, and the connection's handler (and with it its per-address slot and
//! its descriptor) is dropped.
//!
//! Copied from `mush-sshdoorway`'s `kill.rs` (doc 29a §8 F-1, F-3), where it
//! was written for the same russh.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The switch. Cheap to clone through an `Arc`.
#[derive(Debug, Default)]
pub struct KillSwitch {
    killed: AtomicBool,
    wakers: Mutex<[Option<Waker>; 2]>,
}

impl KillSwitch {
    /// A fresh switch.
    #[must_use]
    pub fn new() -> Arc<KillSwitch> {
        Arc::new(KillSwitch::default())
    }

    /// Cut the stream: pending and future reads and writes fail.
    pub fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        let wakers = std::mem::take(
            &mut *self
                .wakers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for w in wakers.into_iter().flatten() {
            w.wake();
        }
    }

    /// Cut the stream after `delay`, from a task of its own. Harmless when
    /// the stream is already gone (the task flips a switch nobody reads), so
    /// every server-initiated disconnect can arm it without checking. Outside
    /// a tokio runtime (a synchronous unit test) nothing is armed.
    pub fn arm(self: &Arc<Self>, delay: Duration) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let switch = Arc::clone(self);
        rt.spawn(async move {
            tokio::time::sleep(delay).await;
            switch.kill();
        });
    }

    /// Whether [`kill`](Self::kill) was called.
    #[must_use]
    pub fn is_killed(&self) -> bool {
        self.killed.load(Ordering::SeqCst)
    }

    fn check(&self, slot: usize, cx: &Context<'_>) -> std::io::Result<()> {
        if self.is_killed() {
            return Err(std::io::Error::from(std::io::ErrorKind::ConnectionAborted));
        }
        let mut w = self
            .wakers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = w.get_mut(slot) {
            *entry = Some(cx.waker().clone());
        }
        drop(w);
        // A kill between the first check and the registration.
        if self.is_killed() {
            return Err(std::io::Error::from(std::io::ErrorKind::ConnectionAborted));
        }
        Ok(())
    }
}

/// `S`, cut when its [`KillSwitch`] is.
#[derive(Debug)]
pub struct Killable<S> {
    inner: S,
    switch: Arc<KillSwitch>,
}

impl<S> Killable<S> {
    /// Wrap `inner`.
    pub fn new(inner: S, switch: Arc<KillSwitch>) -> Killable<S> {
        Killable { inner, switch }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Killable<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if let Err(e) = self.switch.check(0, cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Killable<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if let Err(e) = self.switch.check(1, cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if let Err(e) = self.switch.check(1, cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn a_killed_stream_fails_a_blocked_write() {
        // A tiny duplex nobody reads: the second write blocks.
        let (a, _b) = tokio::io::duplex(8);
        let switch = KillSwitch::new();
        let mut s = Killable::new(a, Arc::clone(&switch));
        s.write_all(b"12345678").await.unwrap();
        let blocked = tokio::spawn(async move { s.write_all(b"more").await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!blocked.is_finished());
        switch.kill();
        let r = tokio::time::timeout(Duration::from_secs(5), blocked)
            .await
            .unwrap()
            .unwrap();
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn a_killed_stream_fails_a_blocked_read() {
        let (a, _b) = tokio::io::duplex(8);
        let switch = KillSwitch::new();
        let mut s = Killable::new(a, Arc::clone(&switch));
        let blocked = tokio::spawn(async move {
            let mut buf = [0u8; 4];
            s.read(&mut buf).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        switch.kill();
        let r = tokio::time::timeout(Duration::from_secs(5), blocked)
            .await
            .unwrap()
            .unwrap();
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn an_armed_switch_kills_after_its_delay() {
        let (a, _b) = tokio::io::duplex(8);
        let switch = KillSwitch::new();
        let mut s = Killable::new(a, Arc::clone(&switch));
        switch.arm(Duration::from_millis(100));
        assert!(!switch.is_killed());
        let blocked = tokio::spawn(async move {
            let mut buf = [0u8; 4];
            s.read(&mut buf).await
        });
        let r = tokio::time::timeout(Duration::from_secs(5), blocked)
            .await
            .unwrap()
            .unwrap();
        assert!(r.is_err());
        assert!(switch.is_killed());
    }
}
