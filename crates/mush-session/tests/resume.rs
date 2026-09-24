//! M4 structured-delivery + resume tests: envelope stamping (seq/ts),
//! ANSI-vs-structured fan-out, ring wraparound + `TooOld`, seq monotonicity
//! across reconnects, and replay ordering. All clocks are manual — no
//! tokio runtime, no wall time.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use mush_proto::{
    ClientCaps, ClientKind, ColorDepth, Envelope, NarrativeKind, OutputEvent, ProjectedCommitToken,
    RichText,
};
use mush_session::{
    AttachOutcome, CharKey, ConnId, ConnectionHandle, DetachReason, ReplayError, SendToConn,
    SessId, SessionConfig, SessionManager,
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

#[test]
fn semantic_receipts_are_bounded_acknowledged_and_redelivered() {
    let mgr = manager_with_ticking_clock(SessionConfig {
        receipt_retention: 2,
        ..SessionConfig::default()
    });
    let (first, mut first_rx) = web_conn(1, 8);
    let (sess, _) = mgr.attach(first, CharKey(42), "nick".into());
    let token = |digit: char| {
        ProjectedCommitToken::new(std::iter::repeat_n(digit, 32).collect::<String>()).unwrap()
    };

    for digit in ['1', '2', '3'] {
        assert_eq!(
            mgr.send_acknowledgeable_to_conn(
                ConnId(1),
                token(digit),
                Arc::new(sys(&format!("receipt {digit}"))),
            ),
            SendToConn::Delivered
        );
    }
    assert_eq!(mgr.unacknowledged_receipt_count(sess), Some(2));
    let delivered: Vec<_> = std::iter::from_fn(|| first_rx.try_recv().ok()).collect();
    assert_eq!(delivered.len(), 3);
    assert_eq!(delivered.last().map(|env| env.seq), Some(3));

    // Re-sending an existing receipt does not consume a second slot, and an
    // exact ACK retires only its named token.
    assert_eq!(
        mgr.send_acknowledgeable_to_conn(ConnId(1), token('3'), Arc::new(sys("receipt 3")),),
        SendToConn::Delivered
    );
    assert_eq!(mgr.unacknowledged_receipt_count(sess), Some(2));
    assert!(mgr.acknowledge_receipt(ConnId(1), &token('2')));
    assert!(!mgr.acknowledge_receipt(ConnId(1), &token('2')));
    assert_eq!(mgr.unacknowledged_receipt_count(sess), Some(1));

    mgr.detach(ConnId(1), DetachReason::Lost);
    assert!(mgr.retain_acknowledgeable_for_char(
        CharKey(42),
        token('4'),
        Arc::new(sys("receipt 4")),
    ));
    assert_eq!(mgr.unacknowledged_receipt_count(sess), Some(2));
    let (second, mut second_rx) = web_conn(2, 8);
    let (same_sess, outcome) = mgr.attach(second, CharKey(42), "nick".into());
    assert_eq!(same_sess, sess);
    assert!(matches!(
        outcome,
        AttachOutcome::Rejoined {
            from_linkdead: true,
            ..
        }
    ));
    let replay = mgr.redeliver_unacknowledged(ConnId(2));
    assert_eq!(replay.delivered, 2);
    assert!(replay.dropped.is_empty());
    let first = second_rx.try_recv().expect("first receipt redelivered");
    let second = second_rx.try_recv().expect("second receipt redelivered");
    assert_eq!(first.seq, 5);
    assert_eq!(first.event, sys("receipt 3"));
    assert_eq!(second.seq, 6);
    assert_eq!(second.event, sys("receipt 4"));
    assert!(mgr.acknowledge_receipt(ConnId(2), &token('3')));
    assert!(mgr.acknowledge_receipt(ConnId(2), &token('4')));
    assert_eq!(mgr.unacknowledged_receipt_count(sess), Some(0));
}

// ---- I12 narrative backlog (backlog_matching) -------------------------------

fn narr(text: &str) -> OutputEvent {
    OutputEvent::Narrative {
        kind: NarrativeKind::Say,
        actor: None,
        text: RichText::plain(text),
    }
}

#[test]
fn backlog_matching_is_newest_first_narrative_only_and_pages_by_before_seq() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 32);
    let (sess, _) = mgr.attach(web, CharKey(1), "a".into());

    // Interleave Sys and Narrative: seqs 1..=5 = s1, n1, s2, n2, n3.
    mgr.send_event_to_char(CharKey(1), sys("s1")); // seq 1
    mgr.send_event_to_char(CharKey(1), narr("n1")); // seq 2
    mgr.send_event_to_char(CharKey(1), sys("s2")); // seq 3
    mgr.send_event_to_char(CharKey(1), narr("n2")); // seq 4
    mgr.send_event_to_char(CharKey(1), narr("n3")); // seq 5

    let keep = |ev: &OutputEvent| matches!(ev, OutputEvent::Narrative { .. });

    // Whole backlog: narrative only, newest-first; Sys excluded.
    let bl = mgr.backlog_matching(sess, None, 10, keep).expect("session");
    let seqs: Vec<u64> = bl.lines.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [5, 4, 2], "narrative seqs newest-first, Sys skipped");
    assert!(!bl.more && !bl.evicted);

    // Limit trips `more`.
    let bl = mgr.backlog_matching(sess, None, 2, keep).expect("session");
    assert_eq!(bl.lines.iter().map(|e| e.seq).collect::<Vec<_>>(), [5, 4]);
    assert!(bl.more, "a further narrative match exists below the page");

    // Paging with a strict-below cursor.
    let bl = mgr
        .backlog_matching(sess, Some(4), 10, keep)
        .expect("session");
    assert_eq!(
        bl.lines.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [2],
        "before_seq=4 excludes seq 4 (strict <) and returns the older n1"
    );
    assert!(!bl.more);
}

#[test]
fn backlog_matching_reports_evicted_once_the_ring_rolls() {
    let mgr = manager_with_ticking_clock(SessionConfig {
        resume_ring: 3,
        ..SessionConfig::default()
    });
    let (web, _rx) = web_conn(1, 32);
    let (sess, _) = mgr.attach(web, CharKey(1), "a".into());

    // Five narratives into a 3-deep ring: seqs 1,2 evicted, ring holds 3,4,5.
    for i in 1..=5 {
        mgr.send_event_to_char(CharKey(1), narr(&format!("n{i}")));
    }
    let keep = |ev: &OutputEvent| matches!(ev, OutputEvent::Narrative { .. });
    let bl = mgr.backlog_matching(sess, None, 10, keep).expect("session");
    assert_eq!(
        bl.lines.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [5, 4, 3]
    );
    assert!(!bl.more, "no further match — older seqs were evicted");
    assert!(bl.evicted, "the ring rolled, so older history may be gone");
}

// ---- 16b cut Q.3 / B.5: `schema.retain == "latest"` and the latest slot ------
//
// The arena's cockpit feed sends a whole-state keyframe per pilot per pulse —
// two a second at `Engaged` tempo — and a 512-entry ring holds about four
// minutes of that and nothing else. A keyframe whose payload declares
// `schema.retain == "latest"` says "the newest of me is the whole truth". Cut
// Q.3 dropped that feed's earlier marked entries from the ring as it recorded
// the new one (a scan of the ring per frame); **16b cut B.5** (`T-31`, `T-61`)
// moved the property to a per-feed latest slot beside the ring: the keyframe
// replaces its feed's slot entry and never enters the ring, `replay_since`
// merges the slot with the ring by seq, and only a `Keyframe` qualifies. The
// ring's other entries are untouched, and `evicted_seq` never moves, so
// `replay_since` cannot answer `TooOld` because of a frame.

/// The marker, spelled out. `mush_session::RETAIN_LATEST` is the thing under
/// test, and a suite that builds its payloads from it would stay green if its
/// value were renamed — while `tacsim`, which has no dependency on this crate
/// and spells the same string in `match_::cockpit`, went on sending the old one
/// and the ring quietly stopped coalescing. The literal is the contract;
/// `crates/mushd/tests/observer_rule.rs` is where the two constants are held to
/// each other.
const LATEST: &str = "latest";

/// A panel keyframe that asks to be retained latest-only.
fn retained(feed: &str, n: u64) -> OutputEvent {
    OutputEvent::Panel {
        feed: feed.into(),
        kind: mush_proto::PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "schema": { "id": feed, "version": 1, "retain": LATEST },
            "n": n,
        }),
    }
}

/// The same feed, without the marker.
fn unmarked(feed: &str, n: u64) -> OutputEvent {
    OutputEvent::Panel {
        feed: feed.into(),
        kind: mush_proto::PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "schema": { "id": feed, "version": 1 },
            "n": n,
        }),
    }
}

/// The `VIEW` case: a grid-shaped frame of the same feed, marked. It must
/// coalesce with the shapeless pulse frames, or a pilot who types `VIEW` twice a
/// second re-creates the flood one frame at a time.
fn retained_grid(feed: &str, n: u64) -> OutputEvent {
    OutputEvent::Panel {
        feed: feed.into(),
        kind: mush_proto::PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "schema": {
                "id": feed,
                "version": 1,
                "shape": "grid",
                "retain": LATEST,
            },
            "n": n,
        }),
    }
}

/// Everything a client that has seen nothing is replayed, oldest first — the
/// ring and the latest slot merged by seq, which is the read a reconnecting
/// client makes.
fn ring_of(mgr: &SessionManager, sess: SessId) -> Vec<Arc<Envelope>> {
    mgr.replay_since(sess, 0).expect("the ring is readable")
}

/// The RING alone, oldest first: `backlog_matching` reads the ring and never
/// the latest slot (16b cut B.5), so keeping everything shows exactly what the
/// ring holds.
fn ring_only(mgr: &SessionManager, sess: SessId) -> Vec<Arc<Envelope>> {
    let mut page = mgr
        .backlog_matching(sess, None, 100_000, |_| true)
        .expect("the ring is readable")
        .lines;
    page.reverse();
    page
}

/// Whether any envelope is a panel of `feed`.
fn holds_panel(envelopes: &[Arc<Envelope>], feed: &str) -> bool {
    envelopes
        .iter()
        .any(|e| matches!(&e.event, OutputEvent::Panel { feed: held, .. } if held.as_str() == feed))
}

/// One ring entry's `n`, for a panel.
fn panel_n(event: &OutputEvent) -> Option<u64> {
    match event {
        OutputEvent::Panel { data, .. } => data.get("n").and_then(serde_json::Value::as_u64),
        _ => None,
    }
}

#[test]
fn a_thousand_retained_frames_leave_one_slot_entry_and_disturb_nothing_else() {
    assert_eq!(
        mush_session::RETAIN_LATEST,
        LATEST,
        "the exported marker is the string every producer spells"
    );
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, mut rx) = web_conn(1, 4_096);
    let (sess, _) = mgr.attach(web, CharKey(1), "a".into());

    // Three ordinary lines, then a thousand cockpit frames, then one more line.
    for i in 1..=3 {
        mgr.send_event_to_char(CharKey(1), sys(&format!("before {i}")));
    }
    for n in 0..1_000 {
        mgr.send_event_to_char(CharKey(1), retained("arena.cockpit", n));
    }
    mgr.send_event_to_char(CharKey(1), sys("after"));

    let ring = ring_of(&mgr, sess);
    let events: Vec<OutputEvent> = ring.iter().map(|e| e.event.clone()).collect();
    assert_eq!(
        events.len(),
        5,
        "one frame survives, and the four lines stay"
    );
    assert_eq!(
        events[..3].to_vec(),
        vec![sys("before 1"), sys("before 2"), sys("before 3")],
        "in order, and ahead of the frame"
    );
    assert_eq!(events[4], sys("after"), "and the line after it is after it");
    // The survivor is the LAST frame, not the first.
    assert_eq!(panel_n(&events[3]), Some(999));
    // **16b cut B.5**: and it lives in the latest slot — the ring itself holds
    // the four lines and not one frame.
    let ring = ring_only(&mgr, sess);
    assert_eq!(
        ring.iter().map(|e| e.event.clone()).collect::<Vec<_>>(),
        vec![
            sys("before 1"),
            sys("before 2"),
            sys("before 3"),
            sys("after")
        ],
        "the ring holds no marked keyframe"
    );
    // Nothing was evicted: the ring never came near its bound, and a client
    // resuming from anywhere is answered rather than told to re-hydrate.
    assert_eq!(mgr.replay_since(sess, 0).expect("from zero").len(), 5);
    assert_eq!(
        mgr.replay_since(sess, 500).expect("from the middle").len(),
        2
    );
    assert!(
        mgr.replay_since(sess, 1_004).expect("caught up").is_empty(),
        "every one of the 1,004 sends still consumed a seq"
    );
    // And the connection was sent all 1,004: the slot is a RESUME rule and
    // never a delivery rule.
    let mut delivered = 0;
    while rx.try_recv().is_ok() {
        delivered += 1;
    }
    assert_eq!(delivered, 1_004);
}

#[test]
fn a_panel_without_the_marker_is_never_coalesced() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 64);
    let (sess, _) = mgr.attach(web, CharKey(2), "b".into());

    for n in 0..5 {
        mgr.send_event_to_char(CharKey(2), unmarked("arena.cockpit", n));
    }
    assert_eq!(ring_of(&mgr, sess).len(), 5, "an unmarked panel is history");

    // A marked frame of the same feed arrives: it replaces only its own kind in
    // the slot, and the five unmarked ones stay exactly where they were.
    mgr.send_event_to_char(CharKey(2), retained("arena.cockpit", 100));
    mgr.send_event_to_char(CharKey(2), retained("arena.cockpit", 101));
    let ring = ring_of(&mgr, sess);
    assert_eq!(ring.len(), 6);
    let ns: Vec<u64> = ring.iter().filter_map(|e| panel_n(&e.event)).collect();
    assert_eq!(ns, [0, 1, 2, 3, 4, 101]);
    // 16b cut B.5: the five unmarked frames are ring entries; the marked one is
    // not.
    let ns: Vec<u64> = ring_only(&mgr, sess)
        .iter()
        .filter_map(|e| panel_n(&e.event))
        .collect();
    assert_eq!(
        ns,
        [0, 1, 2, 3, 4],
        "only the unmarked frames are in the ring"
    );
}

#[test]
fn a_typed_view_coalesces_with_the_pulse_frames_of_its_own_feed() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 64);
    let (sess, _) = mgr.attach(web, CharKey(3), "c".into());

    mgr.send_event_to_char(CharKey(3), retained("arena.cockpit", 1));
    mgr.send_event_to_char(CharKey(3), retained_grid("arena.cockpit", 2));
    mgr.send_event_to_char(CharKey(3), retained("arena.cockpit", 3));
    mgr.send_event_to_char(CharKey(3), retained_grid("arena.cockpit", 4));

    let ring = ring_of(&mgr, sess);
    assert_eq!(ring.len(), 1, "a shape is not a second feed");
    let OutputEvent::Panel { data, .. } = &ring[0].event else {
        panic!("a panel");
    };
    assert_eq!(data.get("n").and_then(serde_json::Value::as_u64), Some(4));
    assert_eq!(
        data.get("schema").and_then(|s| s.get("shape")),
        Some(&serde_json::json!("grid")),
        "and the survivor is the last one sent, whatever its shape"
    );
    assert!(
        ring_only(&mgr, sess).is_empty(),
        "16b cut B.5: the survivor is the slot's, and the ring is empty"
    );
}

#[test]
fn two_retained_feeds_coalesce_independently() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 64);
    let (sess, _) = mgr.attach(web, CharKey(4), "d".into());

    for n in 0..4 {
        mgr.send_event_to_char(CharKey(4), retained("arena.cockpit", n));
        mgr.send_event_to_char(CharKey(4), retained("arena.map", n));
    }
    let ring = ring_of(&mgr, sess);
    assert_eq!(ring.len(), 2, "one survivor per feed");
    let rows: Vec<(String, u64)> = ring
        .iter()
        .filter_map(|e| match &e.event {
            OutputEvent::Panel { feed, .. } => Some((feed.to_string(), panel_n(&e.event)?)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rows,
        vec![("arena.cockpit".to_owned(), 3), ("arena.map".to_owned(), 3)],
        "and each survivor is its own feed's newest, in send order"
    );
    assert!(
        ring_only(&mgr, sess).is_empty(),
        "16b cut B.5: one slot entry per feed, and nothing in the ring"
    );
}

#[test]
fn coalescing_never_moves_the_too_old_boundary_and_leaves_the_backlog_alone() {
    let mgr = manager_with_ticking_clock(SessionConfig {
        resume_ring: 8,
        ..SessionConfig::default()
    });
    let (web, _rx) = web_conn(1, 256);
    let (sess, _) = mgr.attach(web, CharKey(5), "e".into());

    // Four narrative lines, then far more cockpit frames than the ring is deep.
    for i in 1..=4 {
        mgr.send_event_to_char(CharKey(5), narr(&format!("n{i}")));
    }
    for n in 0..50 {
        mgr.send_event_to_char(CharKey(5), retained("arena.cockpit", n));
    }

    // **The point of the whole idiom**: the four lines are still there. Before
    // it, fifty frames into an eight-deep ring left nothing else in it.
    let keep = |ev: &OutputEvent| matches!(ev, OutputEvent::Narrative { .. });
    let backlog = mgr.backlog_matching(sess, None, 10, keep).expect("session");
    assert_eq!(
        backlog
            .lines
            .iter()
            .map(|e| e.event.clone())
            .collect::<Vec<OutputEvent>>(),
        vec![narr("n4"), narr("n3"), narr("n2"), narr("n1")],
        "the narrative backlog is untouched by coalescing"
    );
    assert!(!backlog.evicted, "nothing rolled off the ring");
    assert!(!backlog.more);

    // And a client that has seen nothing at all is still answered, with the one
    // surviving frame in it — never `TooOld`, because `evicted_seq` never moved.
    let replayed = mgr.replay_since(sess, 0).expect("never TooOld");
    assert_eq!(replayed.len(), 5);
    assert_eq!(
        panel_n(&replayed[4].event),
        Some(49),
        "the survivor is last"
    );
    assert_eq!(
        replayed.iter().map(|e| e.seq).collect::<Vec<u64>>(),
        [1, 2, 3, 4, 54],
        "the gap the slot left is the harmless kind a conn-scoped send leaves"
    );
    // 16b cut B.5: fifty frames and an eight-deep ring — and the ring holds the
    // four lines alone, because not one frame ever entered it.
    assert_eq!(
        ring_only(&mgr, sess)
            .iter()
            .map(|e| e.seq)
            .collect::<Vec<u64>>(),
        [1, 2, 3, 4]
    );
}

/// A `Delta` of a retained feed (16b cut B.5 `T-61`): the marker rides the
/// payload, the kind says it is history.
fn retained_delta(feed: &str, n: u64) -> OutputEvent {
    OutputEvent::Panel {
        feed: feed.into(),
        kind: mush_proto::PanelFrameKind::Delta,
        data: serde_json::json!({
            "schema": { "id": feed, "version": 1, "retain": LATEST },
            "n": n,
        }),
    }
}

/// **16b cut B.5 (`T-31`, `T-61`): the latest slot, end to end.** Replay after
/// any `last_seq` is the ring's entries and the slot's newest keyframe of each
/// marked feed, merged by seq; the ring holds no marked keyframe; a `Delta` of
/// the same feed, marker and all, sits in the ring and replays; `evicted_seq`
/// never moves because of a keyframe; and a resume across a slot replacement
/// is never `TooOld`.
#[test]
fn the_latest_slot_replays_each_feeds_newest_keyframe_merged_by_seq() {
    let mgr = manager_with_ticking_clock(SessionConfig {
        resume_ring: 4,
        ..SessionConfig::default()
    });
    let (web, _rx) = web_conn(1, 256);
    let (sess, _) = mgr.attach(web, CharKey(6), "f".into());

    mgr.send_event_to_char(CharKey(6), sys("one")); // seq 1
    mgr.send_event_to_char(CharKey(6), retained("arena.cockpit", 10)); // seq 2
    mgr.send_event_to_char(CharKey(6), retained("arena.map", 20)); // seq 3
    mgr.send_event_to_char(CharKey(6), retained_delta("arena.cockpit", 11)); // seq 4
    mgr.send_event_to_char(CharKey(6), retained("arena.cockpit", 12)); // seq 5
    mgr.send_event_to_char(CharKey(6), sys("two")); // seq 6

    // The ring: the two lines and the delta, never a marked keyframe.
    let ring = ring_only(&mgr, sess);
    assert_eq!(
        ring.iter().map(|e| e.seq).collect::<Vec<u64>>(),
        [1, 4, 6],
        "the lines and the delta are ring entries"
    );
    assert_eq!(panel_n(&ring[1].event), Some(11), "the delta is history");

    // From zero: everything, in seq order, the cockpit slot holding its newest.
    let from_zero = mgr.replay_since(sess, 0).expect("never TooOld");
    assert_eq!(
        from_zero.iter().map(|e| e.seq).collect::<Vec<u64>>(),
        [1, 3, 4, 5, 6],
        "the map's keyframe, the delta, the cockpit's newest keyframe, merged by seq"
    );
    assert_eq!(panel_n(&from_zero[1].event), Some(20));
    assert_eq!(panel_n(&from_zero[3].event), Some(12));

    // After the map's keyframe: the map is not replayed again; the cockpit's
    // replacement is — a resume across a slot replacement, never `TooOld`.
    let after_map = mgr.replay_since(sess, 3).expect("never TooOld");
    assert_eq!(
        after_map.iter().map(|e| e.seq).collect::<Vec<u64>>(),
        [4, 5, 6]
    );
    // A client that saw seq 2 (the replaced keyframe) resumes too.
    assert_eq!(
        mgr.replay_since(sess, 2)
            .expect("the replaced keyframe's seq is a harmless gap")
            .iter()
            .map(|e| e.seq)
            .collect::<Vec<u64>>(),
        [3, 4, 5, 6]
    );
    // Caught up: nothing.
    assert!(mgr.replay_since(sess, 6).expect("caught up").is_empty());

    // A hundred more keyframes of both feeds into a four-deep ring: nothing is
    // evicted, so `evicted_seq` never moved and every `last_seq` is answered.
    for n in 0..100 {
        mgr.send_event_to_char(CharKey(6), retained("arena.cockpit", 100 + n));
        mgr.send_event_to_char(CharKey(6), retained("arena.map", 200 + n));
    }
    for last_seq in [0, 1, 2, 5, 6, 100, 205] {
        let replayed = mgr
            .replay_since(sess, last_seq)
            .unwrap_or_else(|e| panic!("resume from {last_seq} answered {e:?}"));
        let seqs: Vec<u64> = replayed.iter().map(|e| e.seq).collect();
        assert!(
            seqs.windows(2).all(|pair| pair[0] < pair[1]),
            "merged by seq from {last_seq}: {seqs:?}"
        );
    }
    let latest = mgr.replay_since(sess, 6).expect("never TooOld");
    assert_eq!(
        latest
            .iter()
            .map(|e| (e.seq, panel_n(&e.event)))
            .collect::<Vec<_>>(),
        vec![(205, Some(199)), (206, Some(299))],
        "one keyframe per feed after seq 6, each its feed's newest"
    );
    assert!(!holds_panel(&ring_only(&mgr, sess), "arena.map"));
    let backlog = mgr
        .backlog_matching(sess, None, 10, |_| true)
        .expect("session");
    assert!(!backlog.evicted, "a keyframe never evicts a ring entry");

    // Now the ring does roll (five lines into four places): `evicted_seq`
    // moves for the LINES alone. The slot's keyframes are never evicted — each
    // stays its feed's whole truth however far the ring rolls past its seq —
    // and since the unchanged gate refuses any `last_seq` below `evicted_seq`,
    // one is replayed exactly when it is newer than the client.
    for i in 0..5 {
        mgr.send_event_to_char(CharKey(6), sys(&format!("roll {i}")));
    }
    assert_eq!(
        mgr.replay_since(sess, 6),
        Err(ReplayError::TooOld),
        "the ring rolled past seq 6's successors, so seq 6 must re-hydrate"
    );
    let after_roll = mgr.replay_since(sess, 207).expect("inside the ring");
    assert_eq!(
        after_roll.iter().map(|e| e.seq).collect::<Vec<u64>>(),
        [208, 209, 210, 211],
        "the keyframes (seq 205, 206) are older than the client, so not replayed"
    );
}

/// **16b cut V.1: the latest slot carries the committed `VIEW` payload
/// unchanged.** The canonical `view_frame.json` — the window whose `units` now
/// open with the named spots' markers and whose `legend` ends with a spot's
/// name — is sent as a retain-latest `Keyframe` on `arena.cockpit` to one
/// character, and a resume from nothing replays it with its `data` equal to the
/// fixture, value for value. The slot is shape-blind, and this pins that the
/// payload's new shape crosses it whole.
#[test]
fn the_latest_slot_replays_the_committed_view_frame_unchanged() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../web/src/lib/fixtures/cockpit/view_frame.json"
    ))
    .expect("the committed fixture is JSON");
    assert_eq!(
        fixture.pointer("/schema/retain"),
        Some(&serde_json::json!(LATEST)),
        "the fixture asks to be retained latest-only"
    );
    assert_eq!(
        fixture
            .pointer("/units/0/spot")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "the fixture's first marker is a named spot (16b cut V.1)"
    );
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 64);
    let (sess, _) = mgr.attach(web, CharKey(7), "g".into());
    mgr.send_event_to_char(
        CharKey(7),
        OutputEvent::Panel {
            feed: "arena.cockpit".into(),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: fixture.clone(),
        },
    );
    let replayed = mgr.replay_since(sess, 0).expect("never TooOld");
    assert_eq!(replayed.len(), 1, "the one keyframe, from the slot");
    let OutputEvent::Panel { feed, kind, data } = &replayed[0].event else {
        panic!("the replayed event is the panel: {:?}", replayed[0].event);
    };
    assert_eq!(feed.as_str(), "arena.cockpit");
    assert_eq!(*kind, mush_proto::PanelFrameKind::Keyframe);
    assert_eq!(data, &fixture, "the payload crosses the slot unchanged");
    assert!(
        ring_only(&mgr, sess).is_empty(),
        "and it lives in the slot, not the ring"
    );
}

// ---- I12 leak tripwire: the ring's non-Char writers refuse a Narrative ------
//
// The narrative backlog's leak proof rests on one invariant: a `Narrative`
// reaches the resume ring only through the M11-gated resolved-Char fan-out
// (`send_to_char`), never a session- or broadcast-scoped writer. These pin
// the `debug_assert` on each of the three other `deliver_to_session` callers.

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "Narrative must reach the resume ring")]
fn send_to_session_refuses_a_narrative() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 8);
    let (sess, _) = mgr.attach(web, CharKey(1), "a".into());
    mgr.send_to_session(sess, Arc::new(narr("leak")));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "Narrative must reach the resume ring")]
fn send_event_to_session_refuses_a_narrative() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 8);
    let (sess, _) = mgr.attach(web, CharKey(1), "a".into());
    mgr.send_event_to_session(sess, narr("leak"));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "Narrative must reach the resume ring")]
fn broadcast_all_refuses_a_narrative() {
    let mgr = manager_with_ticking_clock(SessionConfig::default());
    let (web, _rx) = web_conn(1, 8);
    mgr.attach(web, CharKey(1), "a".into());
    mgr.broadcast_all(Arc::new(narr("leak")));
}
