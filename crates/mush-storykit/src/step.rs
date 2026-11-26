//! One bound on one event, and the SSH round trip (doc 51 §3.3 and §3.7, unit
//! FL.9).
//!
//! The small crates' stories (mush-net-web, mush-net-ssh, mush-sshdoorway)
//! awaited a frame, a close or a task under a span chosen at the call: three
//! seconds here, five there, ten for a server's stop. Doc 51 §3.7 says a
//! bound on an event is the rig's one limit, so a slow runner is answered in
//! one place. [`within`] is that bound for a future, as [`crate::wait_until`]
//! is for a condition polled.
//!
//! [`at_most`] is for the one shape that is not an event: a future whose end
//! is not the point, such as a flood that runs until the server cuts its
//! writer. Its span is the caller's, and it says nothing when the span runs
//! out.
//!
//! [`round_trip`] is the barrier for an SSH server's session loop. A story
//! that sends a packet the server is to ignore ("data before the shell does
//! nothing") cannot wait for nothing to happen. It sends a request with a
//! reply after it. The session loop takes one connection's packets in the
//! order they came, so when the reply comes, every packet before it has been
//! handled, and an absence is asserted then: between the send and the reply.

use std::future::Future;
use std::time::Duration;

use crate::wait::STEP_TIMEOUT;

/// **`event` under the one step limit**, [`STEP_TIMEOUT`]: what it gave.
///
/// The limit is a failure bound, never a pace; the call returns when the
/// event does.
///
/// # Panics
///
/// When [`STEP_TIMEOUT`] passes first. The message names `label`.
pub async fn within<T>(label: &str, event: impl Future<Output = T>) -> T {
    within_for(STEP_TIMEOUT, label, event).await
}

/// [`within`] under `bound`: a [`crate::StoryBudget`]'s limit, or a span
/// derived from the figure the event is due by.
///
/// # Panics
///
/// When `bound` passes first. The message names `label` and the bound.
pub async fn within_for<T>(bound: Duration, label: &str, event: impl Future<Output = T>) -> T {
    match tokio::time::timeout(bound, event).await {
        Ok(value) => value,
        Err(_) => panic!("{label} did not happen within {bound:?}"),
    }
}

/// **`work` for at most `span`**: what it gave, or `None` when `span` ran out
/// first. For a future whose end is not what the story waits for; an event
/// the story needs goes through [`within`].
pub async fn at_most<T>(span: Duration, work: impl Future<Output = T>) -> Option<T> {
    tokio::time::timeout(span, work).await.ok()
}

/// **The server has handled everything this connection sent before now.**
///
/// It sends a global request that asks for a reply (`tcpip-forward`, which
/// no server of this workspace grants) and waits, under [`STEP_TIMEOUT`], for
/// the answer, a refusal as much as a grant. An SSH server's session loop
/// takes one connection's packets in order and awaits each handler before
/// the next packet, so the answer comes after the handlers of every packet
/// sent before the request.
///
/// It proves nothing about work a handler hands to another task: a line the
/// server queues for a worker may still be in that queue. For that, wait for
/// what the worker does.
///
/// # Panics
///
/// When the server does not answer within [`STEP_TIMEOUT`].
pub async fn round_trip<H: russh::client::Handler>(handle: &russh::client::Handle<H>) {
    // Granted or refused, the answer is the barrier. Port 0 asks for nothing
    // in particular, and a server that did grant it holds no listener a story
    // reads.
    let _answer = within(
        "the server's answer to a request sent after the packets before it",
        handle.tcpip_forward("127.0.0.1", 0),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn within_hands_back_what_the_event_gave() {
        let got = within("a ready value", async { 7 }).await;
        assert_eq!(got, 7);
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "the close frame did not happen within 60s")]
    async fn an_event_that_never_comes_fails_with_its_label_and_the_step_limit() {
        within("the close frame", std::future::pending::<()>()).await;
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "the stop did not happen within 5s")]
    async fn within_for_names_its_own_bound() {
        within_for(
            Duration::from_secs(5),
            "the stop",
            std::future::pending::<()>(),
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn at_most_says_none_and_does_not_fail_when_the_span_runs_out() {
        let started = tokio::time::Instant::now();
        let got = at_most(Duration::from_secs(3), std::future::pending::<()>()).await;
        assert_eq!(got, None);
        assert_eq!(started.elapsed(), Duration::from_secs(3));
        assert_eq!(at_most(Duration::from_secs(3), async { 1 }).await, Some(1));
    }
}
