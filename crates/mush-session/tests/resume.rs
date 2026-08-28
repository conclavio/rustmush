//! M4 structured-delivery + resume tests: envelope stamping (seq/ts),
//! ANSI-vs-structured fan-out, ring wraparound + `TooOld`, seq monotonicity
//! across reconnects, and replay ordering. All clocks are manual — no
//! tokio runtime, no wall time.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use mush_proto::{ClientCaps, ClientKind, ColorDepth, Envelope, OutputEvent, RichText};
use mush_session::{
    AttachOutcome, CharKey, ConnId, ConnectionHandle, DetachReason, ReplayError, SendToConn,
    SessionConfig, SessionManager,
};
use tokio::sync::mpsc;

fn ansi_caps() -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color: ColorDepth::Ansi256,
        width: 80,
        height: 24,
        utf8: true,
        structured: false,
    }
}

fn web_caps() -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Web,
        color: ColorDepth::TrueColor,
        width: 0,
        height: 0,
        utf8: true,
        structured: true,
    }
}

fn addr(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, last))
}

/// An ANSI test connection: handle + the transport-side event receiver.
fn ansi_conn(id: u64, buffer: usize) -> (ConnectionHandle, mpsc::Receiver<Arc<OutputEvent>>) {
    let (tx, rx) = mpsc::channel(buffer);
    let handle = ConnectionHandle::new_at(
        ConnId(id),
        addr(u8::try_from(id).unwrap_or(1)),
        ansi_caps(),
        tx,
        Instant::now(),
    );
    (handle, rx)
}

/// A structured test connection: handle + the transport-side envelope
/// receiver.
fn web_conn(id: u64, buffer: usize) -> (ConnectionHandle, mpsc::Receiver<Arc<Envelope>>) {
    let (tx, rx) = mpsc::channel(buffer);
    let handle = ConnectionHandle::new_structured_at(
        ConnId(id),
        addr(u8::try_from(id).unwrap_or(1)),
        web_caps(),
        tx,
        Instant::now(),
    );
    (handle, rx)
}

fn sys(text: &str) -> OutputEvent {
    OutputEvent::Sys(RichText::plain(text))
}

/// A manager whose clock ticks forward 1 ms per stamp, starting at 1000.
fn manager_with_ticking_clock(config: SessionConfig) -> SessionManager {
    let ticks = AtomicI64::new(1000);
    SessionManager::with_config_and_clock(config, move || ticks.fetch_add(1, Ordering::Relaxed))
}

#[test]
fn structured_vs_ansi_fanout_one_event_two_renders() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (ssh, mut ssh_rx) = ansi_conn(1, 8);
    let (web, mut web_rx) = web_conn(2, 8);
    mgr.attach(ssh, CharKey(42), "nick".into());
    let (_, out) = mgr.attach(web, CharKey(42), "nick".into());
    assert!(matches!(out, AttachOutcome::Rejoined { others: 1, .. }));

    let report = mgr.send_event_to_char(CharKey(42), sys("You say, \"hello\""));
    assert_eq!(report.delivered, 2);
    assert!(report.dropped.is_empty());

    // The SSH connection receives the bare event.
    let plain = ssh_rx.try_recv().expect("ansi conn got the event");
    assert_eq!(*plain, sys("You say, \"hello\""));

    // The web connection receives the same event stamped in an envelope.
    let env = web_rx.try_recv().expect("structured conn got the envelope");
    assert_eq!(env.seq, 1, "first session event is seq 1");
    assert_eq!(env.ts, 1000, "ts comes from the injected clock");
    assert_eq!(env.event, *plain, "one semantic event, two renders");

    // A second event advances seq and ts.
    mgr.send_event_to_char(CharKey(42), sys("second"));
    let env = web_rx.try_recv().expect("second envelope");
    assert_eq!((env.seq, env.ts), (2, 1001));
    assert_eq!(*ssh_rx.try_recv().expect("second event"), sys("second"));
}

#[test]
fn legacy_arc_send_path_stamps_and_rings_identically() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, mut web_rx) = web_conn(1, 8);
    let (sess, _) = mgr.attach(web, CharKey(7), "a".into());

    // The pre-M4 Arc-taking path must route through the same stamping
    // machinery: envelopes on the wire, envelopes in the ring.
    let ev = Arc::new(sys("via legacy path"));
    let report = mgr.send_to_char(CharKey(7), Arc::clone(&ev));
    assert_eq!(report.delivered, 1);
    let env = web_rx.try_recv().expect("envelope");
    assert_eq!(env.seq, 1);
    assert_eq!(env.event, *ev);

    let replayed = mgr.replay_since(sess, 0).expect("replay");
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].seq, 1);
    assert_eq!(replayed[0].event, *ev);
}

#[test]
fn seq_stays_monotonic_across_reconnects_and_replay_fills_the_gap() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web1, mut rx1) = web_conn(1, 8);
    let (sess, _) = mgr.attach(web1, CharKey(5), "kayla".into());

    for i in 1..=3 {
        mgr.send_event_to_char(CharKey(5), sys(&format!("event {i}")));
    }
    let last_seen = rx1.try_recv().and_then(|_| rx1.try_recv()).expect("drain");
    assert_eq!(last_seen.seq, 2, "client saw up through seq 2");
    // seq 3 sits undelivered in the channel when the link "drops".
    mgr.detach_at(ConnId(1), DetachReason::Lost, Instant::now());
    assert!(mgr.is_linkdead(sess));

    // Output while linkdead is still stamped + rung.
    mgr.send_event_to_char(CharKey(5), sys("event 4"));

    // Reconnect: same session, seq continues; replay closes the gap.
    let (web2, mut rx2) = web_conn(2, 8);
    let (sess2, out) = mgr.attach(web2, CharKey(5), "kayla".into());
    assert_eq!(sess2, sess, "reconnect revives the same session");
    assert!(matches!(
        out,
        AttachOutcome::Rejoined {
            from_linkdead: true,
            ..
        }
    ));
    let replayed = mgr.replay_since(sess, last_seen.seq).expect("replay");
    let seqs: Vec<u64> = replayed.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [3, 4], "the missed envelopes, in order");
    assert_eq!(replayed[0].event, sys("event 3"));
    assert_eq!(replayed[1].event, sys("event 4"));

    // Live delivery resumes after the replayed range: seq 5, not 1.
    mgr.send_event_to_char(CharKey(5), sys("event 5"));
    let env = rx2.try_recv().expect("live envelope after resume");
    assert_eq!(env.seq, 5, "seq never restarts within a session");
    assert_eq!(mgr.last_seq(sess), Some(5));
}

#[test]
fn replay_since_rejects_a_seq_the_session_never_stamped() {
    // A fresh session (this incarnation) stamped up through seq N=3.
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 64);
    let (sess, _) = mgr.attach(web, CharKey(30), "reborn".into());
    for i in 1..=3 {
        mgr.send_event_to_char(CharKey(30), sys(&format!("event {i}")));
    }
    assert_eq!(mgr.last_seq(sess), Some(3));

    // A client resuming with a seq far beyond N — e.g. remembered from a
    // PRIOR incarnation of this character's session (that one was reaped;
    // this is a brand new session that happens to reuse the character) —
    // must not read as "already caught up". Silently returning Ok(empty)
    // here is exactly the bug: the client believes it is caught up and
    // loses every event this new session ever stamps.
    assert_eq!(
        mgr.replay_since(sess, 3 + 1_000_000),
        Err(ReplayError::TooOld),
        "a seq this session never stamped must never look like 'caught up'"
    );

    // Sanity: a seq within what was actually stamped is still fine.
    assert!(
        mgr.replay_since(sess, 3)
            .expect("truly caught up")
            .is_empty()
    );
    assert_eq!(mgr.replay_since(sess, 1).expect("replay").len(), 2);
}

#[test]
fn ring_wraparound_evicts_oldest_and_reports_too_old() {
    let mgr = manager_with_ticking_clock(SessionConfig {
        resume_ring: 4,
        ..SessionConfig::default()
    });
    let (web, _rx) = web_conn(1, 64);
    let (sess, _) = mgr.attach(web, CharKey(9), "w".into());

    for i in 1..=10 {
        mgr.send_event_to_char(CharKey(9), sys(&format!("event {i}")));
    }
    // Ring holds seqs 7..=10; 6 was the last evicted.
    let replayed = mgr.replay_since(sess, 6).expect("gap fits the ring");
    let seqs: Vec<u64> = replayed.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [7, 8, 9, 10]);

    assert_eq!(
        mgr.replay_since(sess, 5),
        Err(ReplayError::TooOld),
        "seq 6 was evicted: a client at 5 cannot be caught up"
    );
    assert_eq!(mgr.replay_since(sess, 0), Err(ReplayError::TooOld));

    // Fully caught up: nothing to replay.
    assert!(mgr.replay_since(sess, 10).expect("caught up").is_empty());
    // Claiming a seq beyond anything ever stamped is TooOld, not "caught
    // up" — this session never stamped 99 (HIGH finding: see
    // `replay_since_rejects_a_seq_the_session_never_stamped` for the full
    // prior-incarnation scenario this guards against).
    assert_eq!(mgr.replay_since(sess, 99), Err(ReplayError::TooOld));

    assert_eq!(
        mgr.replay_since(mush_session::SessId(999), 0),
        Err(ReplayError::UnknownSession)
    );
}

#[test]
fn replay_preserves_append_order_and_content() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 64);
    let (sess, _) = mgr.attach(web, CharKey(3), "o".into());

    let texts = ["alpha", "beta", "gamma", "delta", "epsilon"];
    for t in texts {
        mgr.send_event_to_char(CharKey(3), sys(t));
    }
    let replayed = mgr.replay_since(sess, 2).expect("replay");
    assert_eq!(replayed.len(), 3);
    for (i, env) in replayed.iter().enumerate() {
        let expected_seq = 3 + u64::try_from(i).unwrap();
        assert_eq!(env.seq, expected_seq, "strictly increasing seqs");
        assert_eq!(env.event, sys(texts[2 + i]), "append order preserved");
        assert_eq!(
            env.ts,
            1000 + i64::try_from(2 + i).unwrap(),
            "ts stamped per event by the injected clock"
        );
    }
}

#[test]
fn conn_scoped_sends_consume_seqs_but_never_replay() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web_a, mut rx_a) = web_conn(1, 8);
    let (web_b, mut rx_b) = web_conn(2, 8);
    let (sess, _) = mgr.attach(web_a, CharKey(4), "p".into());
    mgr.attach(web_b, CharKey(4), "p".into());

    mgr.send_event_to_char(CharKey(4), sys("to everyone")); // seq 1
    assert_eq!(
        mgr.send_to_conn(ConnId(1), Arc::new(sys("only for conn 1"))),
        SendToConn::Delivered
    );
    mgr.send_event_to_char(CharKey(4), sys("to everyone again")); // seq 3

    // Conn A saw all three, each enveloped with a distinct seq.
    let seqs_a: Vec<u64> = std::iter::from_fn(|| rx_a.try_recv().ok().map(|e| e.seq)).collect();
    assert_eq!(seqs_a, [1, 2, 3], "conn-scoped send consumed seq 2");
    // Conn B saw only the session-scoped events.
    let seqs_b: Vec<u64> = std::iter::from_fn(|| rx_b.try_recv().ok().map(|e| e.seq)).collect();
    assert_eq!(seqs_b, [1, 3], "the prompt was not fanned out");

    // The ring skips the conn-scoped envelope: B resuming from 1 gets only
    // seq 3 — never A's prompt.
    let replayed = mgr.replay_since(sess, 1).expect("replay");
    let seqs: Vec<u64> = replayed.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [3], "conn-scoped envelopes are not ring-recorded");
}

#[test]
fn send_to_conn_keeps_plain_events_for_ansi() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (ssh, mut rx) = ansi_conn(1, 8);
    let (sess, _) = mgr.attach(ssh, CharKey(6), "s".into());

    assert_eq!(
        mgr.send_to_conn(ConnId(1), Arc::new(sys("banner"))),
        SendToConn::Delivered
    );
    assert_eq!(*rx.try_recv().expect("plain event"), sys("banner"));
    // ANSI conn-scoped sends do not consume seqs (nothing was stamped).
    assert_eq!(mgr.last_seq(sess), Some(0));
    assert_eq!(
        mgr.send_to_conn(ConnId(99), Arc::new(sys("nobody"))),
        SendToConn::Unknown
    );
}

#[test]
fn broadcast_stamps_per_session_for_mixed_transports() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (ssh, mut ssh_rx) = ansi_conn(1, 8);
    let (web, mut web_rx) = web_conn(2, 8);
    mgr.attach(ssh, CharKey(1), "a".into());
    let (web_sess, _) = mgr.attach(web, CharKey(2), "b".into());

    let ev = Arc::new(sys("The server is going down."));
    let report = mgr.broadcast_all(Arc::clone(&ev));
    assert_eq!(report.delivered, 2);

    let got = ssh_rx.try_recv().expect("broadcast reached ssh");
    assert!(
        Arc::ptr_eq(&got, &ev),
        "ANSI fan-out still shares the one Arc"
    );
    let env = web_rx.try_recv().expect("broadcast reached web");
    assert_eq!(env.seq, 1, "seq is per-session, stamped fresh");
    assert_eq!(env.event, *ev);
    // And it is resumable.
    assert_eq!(mgr.replay_since(web_sess, 0).expect("replay").len(), 1);
}

#[test]
fn slow_structured_client_is_flagged_like_ansi() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    // Capacity 1 and nobody draining: the second envelope must overflow.
    let (web, mut rx) = web_conn(1, 1);
    mgr.attach(web, CharKey(11), "slow".into());
    let handle = mgr.conn_handle(ConnId(1)).unwrap();

    assert_eq!(mgr.send_event_to_char(CharKey(11), sys("one")).delivered, 1);
    assert!(!handle.output_dropped());
    let second = mgr.send_event_to_char(CharKey(11), sys("two"));
    assert_eq!(second.delivered, 0);
    assert_eq!(second.dropped, vec![ConnId(1)]);
    assert!(handle.output_dropped(), "overflow set the dropped flag");

    // The overflowed envelope is still in the ring: a reconnect (or the
    // driver forgiving the client) can recover it via replay.
    let sess = mgr.session_of_conn(ConnId(1)).unwrap();
    let first = rx.try_recv().expect("first envelope");
    let replayed = mgr.replay_since(sess, first.seq).expect("replay");
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].event, sys("two"));
}

#[test]
fn mistyped_sink_sends_are_refused_without_flagging() {
    // Direct handle-level contract: routing bugs must not read as slow
    // clients.
    let (web, _rx) = web_conn(1, 8);
    assert!(!web.try_send(Arc::new(sys("bare event to web"))));
    assert!(
        !web.output_dropped(),
        "mis-typed send must not set the flag"
    );

    let (ssh, _rx) = ansi_conn(2, 8);
    let env = Arc::new(Envelope::new(1, 0, sys("envelope to ssh")));
    assert!(!ssh.try_send_envelope(env));
    assert!(!ssh.output_dropped());
}

#[test]
fn sessions_have_independent_seq_streams() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web_a, mut rx_a) = web_conn(1, 8);
    let (web_b, mut rx_b) = web_conn(2, 8);
    mgr.attach(web_a, CharKey(1), "a".into());
    mgr.attach(web_b, CharKey(2), "b".into());

    mgr.send_event_to_char(CharKey(1), sys("a1"));
    mgr.send_event_to_char(CharKey(1), sys("a2"));
    mgr.send_event_to_char(CharKey(2), sys("b1"));

    let seqs_a: Vec<u64> = std::iter::from_fn(|| rx_a.try_recv().ok().map(|e| e.seq)).collect();
    let seqs_b: Vec<u64> = std::iter::from_fn(|| rx_b.try_recv().ok().map(|e| e.seq)).collect();
    assert_eq!(seqs_a, [1, 2]);
    assert_eq!(seqs_b, [1], "each session counts from 1 independently");
}
