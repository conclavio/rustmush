//! Integration tests for the mush-session public API: attach/detach,
//! multi-connect fan-out, linkdead lifecycle, flood control, slow-client
//! flagging, and WHO facts. No tokio runtime required — everything in this
//! crate is non-blocking by contract.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent, RichText};
use mush_session::{
    AttachOutcome, CharKey, ConnId, ConnectionHandle, DetachOutcome, DetachReason, FloodVerdict,
    SendToConn, SessionConfig, SessionManager, sanitize_line,
};
use tokio::sync::mpsc;

fn ssh_caps() -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color: ColorDepth::Ansi256,
        width: 80,
        height: 24,
        utf8: true,
        structured: false,
    }
}

fn addr(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, last))
}

/// A test connection: handle + the transport-side receiver.
fn conn_at(
    id: u64,
    ip: u8,
    buffer: usize,
    connected_at: Instant,
) -> (ConnectionHandle, mpsc::Receiver<Arc<OutputEvent>>) {
    let (tx, rx) = mpsc::channel(buffer);
    let handle = ConnectionHandle::new_at(ConnId(id), addr(ip), ssh_caps(), tx, connected_at);
    (handle, rx)
}

fn ev(text: &str) -> Arc<OutputEvent> {
    Arc::new(OutputEvent::Sys(RichText::plain(text)))
}

#[test]
fn attach_multiconnect_fanout_shares_the_arc() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();
    let (c1, mut rx1) = conn_at(1, 1, 8, t0);
    let (c2, mut rx2) = conn_at(2, 2, 8, t0);

    let (s1, out1) = mgr.attach(c1, CharKey(42), "nick".into());
    assert_eq!(out1, AttachOutcome::New);
    let (s2, out2) = mgr.attach(c2, CharKey(42), "nick".into());
    assert_eq!(s1, s2, "same character joins the same session");
    assert_eq!(
        out2,
        AttachOutcome::Rejoined {
            others: 1,
            from_linkdead: false
        }
    );

    let event = ev("You hear a distant hum.");
    let report = mgr.send_to_char(CharKey(42), Arc::clone(&event));
    assert_eq!(report.delivered, 2);
    assert!(report.dropped.is_empty());

    let got1 = rx1.try_recv().expect("conn 1 got the event");
    let got2 = rx2.try_recv().expect("conn 2 got the event");
    assert!(
        Arc::ptr_eq(&got1, &event),
        "fan-out clones the Arc, not the event"
    );
    assert!(Arc::ptr_eq(&got2, &event));

    // A character nobody plays delivers to nobody.
    let report = mgr.send_to_char(CharKey(999), ev("void"));
    assert_eq!(report.delivered, 0);
}

#[test]
fn detach_with_others_remaining() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();
    let (c1, _rx1) = conn_at(1, 1, 8, t0);
    let (c2, _rx2) = conn_at(2, 2, 8, t0);
    let (sess, _) = mgr.attach(c1, CharKey(7), "a".into());
    mgr.attach(c2, CharKey(7), "a".into());

    assert_eq!(
        mgr.detach_at(ConnId(1), DetachReason::Lost, t0),
        Some(DetachOutcome::OthersRemain)
    );
    assert!(!mgr.is_linkdead(sess));
    assert_eq!(mgr.connection_count(), 1);
    assert_eq!(mgr.session_count(), 1);
    // Detaching an unknown connection is None, and idempotent.
    assert_eq!(mgr.detach_at(ConnId(1), DetachReason::Lost, t0), None);
}

#[test]
fn linkdead_lifecycle_grace_expiry() {
    let mgr = SessionManager::new(); // default grace: 300 s
    let t0 = Instant::now();
    let (c1, _rx1) = conn_at(1, 1, 8, t0);
    let (sess, _) = mgr.attach(c1, CharKey(5), "kayla".into());

    let out = mgr.detach_at(ConnId(1), DetachReason::Lost, t0);
    assert_eq!(
        out,
        Some(DetachOutcome::Linkdead {
            grace: Duration::from_secs(300)
        })
    );
    assert!(mgr.is_linkdead(sess));
    assert_eq!(mgr.session_count(), 1, "linkdead session persists");
    assert_eq!(mgr.connection_count(), 0);
    assert_eq!(
        mgr.next_linkdead_expiry(),
        Some(t0 + Duration::from_secs(300))
    );

    // WHO still lists the linkdead character.
    let who = mgr.who();
    assert_eq!(who.len(), 1);
    assert!(who[0].linkdead);
    assert_eq!(who[0].conns, 0);

    // Before the grace elapses: nothing to reap.
    assert!(mgr.take_expired(t0 + Duration::from_secs(299)).is_empty());
    assert_eq!(mgr.session_count(), 1);

    // After the grace: reaped exactly once, with the facts the driver needs.
    let reaped = mgr.take_expired(t0 + Duration::from_secs(301));
    assert_eq!(reaped.len(), 1);
    assert_eq!(reaped[0].sess, sess);
    assert_eq!(reaped[0].character, CharKey(5));
    assert_eq!(reaped[0].account, "kayla");
    assert_eq!(mgr.session_count(), 0);
    assert!(mgr.who().is_empty());
    assert!(mgr.take_expired(t0 + Duration::from_secs(600)).is_empty());
    assert_eq!(mgr.next_linkdead_expiry(), None);

    // The character is free again: a new attach starts a fresh session.
    let (c2, _rx2) = conn_at(2, 1, 8, t0 + Duration::from_secs(400));
    let (sess2, out) = mgr.attach(c2, CharKey(5), "kayla".into());
    assert_eq!(out, AttachOutcome::New);
    assert_ne!(sess2, sess);
}

#[test]
fn reconnect_cancels_linkdead() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();
    let (c1, _rx1) = conn_at(1, 1, 8, t0);
    let (sess, _) = mgr.attach(c1, CharKey(5), "kayla".into());
    mgr.detach_at(ConnId(1), DetachReason::Lost, t0);
    assert!(mgr.is_linkdead(sess));

    let (c2, mut rx2) = conn_at(2, 3, 8, t0 + Duration::from_secs(30));
    let (sess2, out) = mgr.attach(c2, CharKey(5), "kayla".into());
    assert_eq!(sess2, sess, "reconnect revives the same session");
    assert_eq!(
        out,
        AttachOutcome::Rejoined {
            others: 0,
            from_linkdead: true
        }
    );
    assert!(!mgr.is_linkdead(sess));

    // Long after the original grace would have expired: nothing is reaped.
    assert!(mgr.take_expired(t0 + Duration::from_secs(3600)).is_empty());
    assert_eq!(mgr.session_count(), 1);
    let report = mgr.send_to_char(CharKey(5), ev("still here"));
    assert_eq!(report.delivered, 1);
    assert!(rx2.try_recv().is_ok());
}

#[test]
fn quit_and_boot_close_immediately() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();
    let (c1, _rx1) = conn_at(1, 1, 8, t0);
    let (sess, _) = mgr.attach(c1, CharKey(9), "q".into());

    assert_eq!(
        mgr.detach_at(ConnId(1), DetachReason::Quit, t0),
        Some(DetachOutcome::Closed)
    );
    assert_eq!(mgr.session_count(), 0);
    assert_eq!(mgr.session_of_char(CharKey(9)), None);
    assert!(!mgr.is_linkdead(sess));
    assert!(mgr.take_expired(t0 + Duration::from_secs(9999)).is_empty());

    let (c2, _rx2) = conn_at(2, 1, 8, t0);
    let (_, out) = mgr.attach(c2, CharKey(9), "q".into());
    assert_eq!(out, AttachOutcome::New);
    assert_eq!(
        mgr.detach_at(ConnId(2), DetachReason::Booted, t0),
        Some(DetachOutcome::Closed)
    );
    assert_eq!(mgr.session_count(), 0);
}

#[test]
fn close_session_returns_all_handles() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();
    let (c1, _rx1) = conn_at(1, 1, 8, t0);
    let (c2, _rx2) = conn_at(2, 2, 8, t0);
    let (sess, _) = mgr.attach(c1, CharKey(3), "s".into());
    mgr.attach(c2, CharKey(3), "s".into());

    let handles = mgr.close_session(sess);
    assert_eq!(handles.len(), 2);
    assert_eq!(mgr.session_count(), 0);
    assert_eq!(mgr.connection_count(), 0);
    assert_eq!(mgr.session_of_char(CharKey(3)), None);
    assert!(
        mgr.close_session(sess).is_empty(),
        "second close is a no-op"
    );
}

#[test]
fn flood_bucket_over_manager_with_fake_clock() {
    let mgr = SessionManager::with_config(SessionConfig {
        flood_rate: 2.0,
        flood_burst: 3.0,
        ..SessionConfig::default()
    });
    let t0 = Instant::now();
    let (c1, _rx1) = conn_at(1, 1, 8, t0);
    mgr.attach(c1, CharKey(1), "f".into());

    // Burst of 3, then throttled.
    for i in 0..3 {
        assert_eq!(
            mgr.check_input_at(ConnId(1), t0),
            FloodVerdict::Ok,
            "line {i}"
        );
    }
    assert_eq!(mgr.check_input_at(ConnId(1), t0), FloodVerdict::Throttled);
    // One second at 2/s: exactly two more lines pass.
    let t1 = t0 + Duration::from_secs(1);
    assert_eq!(mgr.check_input_at(ConnId(1), t1), FloodVerdict::Ok);
    assert_eq!(mgr.check_input_at(ConnId(1), t1), FloodVerdict::Ok);
    assert_eq!(mgr.check_input_at(ConnId(1), t1), FloodVerdict::Throttled);
    // Unknown connections are never throttled.
    assert_eq!(mgr.check_input_at(ConnId(99), t1), FloodVerdict::Ok);
}

/// A flood-exempt character (the world task marks actors inside an input
/// context — mail compose) bypasses the throttle entirely, on every one of
/// their connections, without draining their bucket; clearing the flag
/// restores normal throttling with the bucket intact.
#[test]
fn flood_exemption_bypasses_the_throttle_for_a_character() {
    let mgr = SessionManager::with_config(SessionConfig {
        flood_rate: 2.0,
        flood_burst: 3.0,
        ..SessionConfig::default()
    });
    let t0 = Instant::now();
    let (c1, _rx1) = conn_at(1, 1, 8, t0);
    mgr.attach(c1, CharKey(1), "f".into());
    let (c2, _rx2) = conn_at(2, 2, 8, t0);
    mgr.attach(c2, CharKey(2), "g".into());

    mgr.set_flood_exempt(CharKey(1), true);
    // A 40-line paste (way over burst 3) passes untouched.
    for i in 0..40 {
        assert_eq!(
            mgr.check_input_at(ConnId(1), t0),
            FloodVerdict::Ok,
            "exempt paste line {i}"
        );
    }
    // A different character is still throttled normally.
    for _ in 0..3 {
        assert_eq!(mgr.check_input_at(ConnId(2), t0), FloodVerdict::Ok);
    }
    assert_eq!(mgr.check_input_at(ConnId(2), t0), FloodVerdict::Throttled);
    // Clearing the exemption restores throttling — with the full burst
    // intact (the exempt lines never charged the bucket).
    mgr.set_flood_exempt(CharKey(1), false);
    for _ in 0..3 {
        assert_eq!(mgr.check_input_at(ConnId(1), t0), FloodVerdict::Ok);
    }
    assert_eq!(mgr.check_input_at(ConnId(1), t0), FloodVerdict::Throttled);
}

#[test]
fn slow_client_full_buffer_sets_dropped_flag() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();
    // Capacity 1 and nobody draining: the second event must overflow.
    let (c1, mut rx1) = conn_at(1, 1, 1, t0);
    mgr.attach(c1, CharKey(11), "slow".into());
    let handle = mgr.conn_handle(ConnId(1)).unwrap();

    let first = mgr.send_to_char(CharKey(11), ev("one"));
    assert_eq!(first.delivered, 1);
    assert!(!handle.output_dropped());

    let second = mgr.send_to_char(CharKey(11), ev("two"));
    assert_eq!(second.delivered, 0);
    assert_eq!(second.dropped, vec![ConnId(1)]);
    assert!(handle.output_dropped(), "overflow set the dropped flag");
    assert_eq!(
        mgr.send_to_conn(ConnId(1), ev("three")),
        SendToConn::Dropped
    );

    // Draining lets sends succeed again, but the flag is sticky until the
    // driver decides (disconnect or forgive).
    assert!(rx1.try_recv().is_ok());
    assert_eq!(
        mgr.send_to_conn(ConnId(1), ev("four")),
        SendToConn::Delivered
    );
    assert!(handle.output_dropped());
    handle.clear_output_dropped();
    assert!(!handle.output_dropped());

    assert_eq!(
        mgr.send_to_conn(ConnId(99), ev("nobody")),
        SendToConn::Unknown
    );
}

#[test]
fn broadcast_reaches_every_connection() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();
    let (c1, mut rx1) = conn_at(1, 1, 8, t0);
    let (c2, mut rx2) = conn_at(2, 2, 8, t0);
    let (c3, mut rx3) = conn_at(3, 3, 8, t0);
    mgr.attach(c1, CharKey(1), "a".into());
    mgr.attach(c2, CharKey(1), "a".into()); // second conn, same char
    mgr.attach(c3, CharKey(2), "b".into());

    let event = ev("The server is going down for maintenance.");
    let report = mgr.broadcast_all(Arc::clone(&event));
    assert_eq!(report.delivered, 3);
    for rx in [&mut rx1, &mut rx2, &mut rx3] {
        let got = rx.try_recv().expect("broadcast reached this conn");
        assert!(Arc::ptr_eq(&got, &event));
    }
}

#[test]
fn who_reports_session_facts() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();

    // Alice: two connections, input on the second one.
    let (a1, _rxa1) = conn_at(1, 10, 8, t0);
    let (a2, _rxa2) = conn_at(2, 11, 8, t0 + Duration::from_secs(60));
    mgr.attach(a1, CharKey(100), "alice".into());
    mgr.attach(a2, CharKey(100), "alice".into());
    mgr.record_input_at(ConnId(2), t0 + Duration::from_secs(90));

    // Bob: one connection, connected later, never typed.
    let (b1, _rxb1) = conn_at(3, 20, 8, t0 + Duration::from_secs(120));
    mgr.attach(b1, CharKey(200), "bob".into());

    let who = mgr.who();
    assert_eq!(who.len(), 2);
    // Ordered by connect time: alice first.
    let (alice, bob) = (&who[0], &who[1]);
    assert_eq!(alice.character, CharKey(100));
    assert_eq!(alice.account, "alice");
    assert_eq!(
        alice.addr,
        addr(10),
        "primary (earliest) connection's address"
    );
    assert_eq!(alice.connected_at, t0);
    assert_eq!(
        alice.last_input,
        t0 + Duration::from_secs(90),
        "max across conns"
    );
    assert_eq!(alice.conns, 2);
    assert!(!alice.linkdead);

    assert_eq!(bob.character, CharKey(200));
    assert_eq!(bob.account, "bob");
    assert_eq!(bob.addr, addr(20));
    assert_eq!(bob.connected_at, t0 + Duration::from_secs(120));
    assert_eq!(
        bob.last_input,
        t0 + Duration::from_secs(120),
        "no input yet: idle counts from connect"
    );
    assert_eq!(bob.conns, 1);

    // Idle math the WHO renderer will do.
    let now = t0 + Duration::from_secs(150);
    let handle = mgr.conn_handle(ConnId(2)).unwrap();
    assert_eq!(handle.idle_for(now), Duration::from_secs(60));

    // Alice loses her primary conn: WHO now shows the remaining conn's addr,
    // and her input high-water mark survives the detach.
    mgr.detach_at(ConnId(1), DetachReason::Lost, now);
    let who = mgr.who();
    assert_eq!(who[0].conns, 1);
    assert_eq!(who[0].addr, addr(11));
    assert_eq!(who[0].last_input, t0 + Duration::from_secs(90));
    assert_eq!(
        who[0].connected_at, t0,
        "session connect time is the first attach"
    );
}

#[test]
fn caps_update_round_trips() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();
    let (c1, _rx1) = conn_at(1, 1, 8, t0);
    mgr.attach(c1, CharKey(1), "c".into());

    let mut caps = mgr.caps(ConnId(1)).unwrap();
    assert_eq!(caps.width, 80);
    caps.width = 132;
    caps.height = 43;
    assert!(mgr.update_caps(ConnId(1), caps));
    let caps = mgr.caps(ConnId(1)).unwrap();
    assert_eq!((caps.width, caps.height), (132, 43));
    assert!(!mgr.update_caps(ConnId(9), caps));
}

#[test]
fn sanitize_is_exported_and_strips_esc() {
    assert_eq!(sanitize_line("say \x1b[2J\x1b[H boo"), "say [2J[H boo");
    assert_eq!(sanitize_line("keep\ttabs"), "keep\ttabs");
}

#[test]
fn conns_of_session_lists_handles_in_attach_order() {
    let mgr = SessionManager::new();
    let t0 = Instant::now();
    // One ANSI and one structured connection on the same session (an SSH
    // login plus a web tab), so a driver can act on a subset (e.g.
    // @newpassword boots only the structured one).
    let (ansi, _rx1) = conn_at(1, 1, 8, t0);
    let (env_tx, _env_rx) = mpsc::channel(8);
    let structured = ConnectionHandle::new_structured(
        ConnId(2),
        addr(2),
        ClientCaps {
            kind: ClientKind::Web,
            structured: true,
            ..ssh_caps()
        },
        env_tx,
    );
    let (sess, _) = mgr.attach(ansi, CharKey(7), "nick".into());
    mgr.attach(structured, CharKey(7), "nick".into());

    let handles = mgr.conns_of_session(sess);
    assert_eq!(
        handles.iter().map(|h| h.id).collect::<Vec<_>>(),
        vec![ConnId(1), ConnId(2)],
        "attach order"
    );
    assert!(!handles[0].is_structured());
    assert!(handles[1].is_structured());

    // Detaching one shrinks the list; an unknown session is empty.
    mgr.detach(ConnId(1), DetachReason::Quit);
    let handles = mgr.conns_of_session(sess);
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].id, ConnId(2));
    assert!(
        mgr.conns_of_session(mush_session::SessId(999)).is_empty(),
        "unknown session lists nothing"
    );
}
