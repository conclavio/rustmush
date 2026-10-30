//! End to end: a real sshdoorway in front of a real instance-side trusted
//! ingress (`mush-net-ssh`'s `TrustedIngress`, in process over a duplex
//! stream), driven by a russh client on loopback TCP. Runs on every platform.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use std::time::Duration;

use russh::{ChannelMsg, client};
use support::*;

/// Refused outright, or refused with nothing left to try (the russh client
/// then declines to attempt another method).
fn refused(r: Result<client::AuthResult, russh::Error>) -> bool {
    r.map(|r| !r.success()).unwrap_or(true)
}

#[tokio::test]
async fn a_bare_service_name_reaches_the_welcome_screen() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("dev1").await.unwrap().success());
    let mut ch = shell(&h).await;
    let out = read_until(&mut ch, "connect <account> <password>").await;
    assert!(out.contains("rustmush test banner"), "{out}");
    // The instance saw the player's address, not "unspecified".
    assert_eq!(
        t.hooks.opened(),
        vec!["127.0.0.1".parse::<std::net::IpAddr>().unwrap()]
    );
    h.disconnect(russh::Disconnect::ByApplication, "", "")
        .await
        .ok();
}

#[tokio::test]
async fn password_login_with_a_character_reaches_the_game_and_lines_flow() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(
        !h.authenticate_none("dev1+faenix+Kira")
            .await
            .unwrap()
            .success()
    );
    assert!(
        h.authenticate_password("dev1+faenix+Kira", "pw")
            .await
            .unwrap()
            .success()
    );
    let ch = shell(&h).await;
    wait_for(|| t.hooks.connected() == vec![("faenix".to_owned(), 1)]).await;
    ch.data(&b"say hello\r"[..]).await.unwrap();
    wait_for(|| t.hooks.lines().iter().any(|l| l == "say hello")).await;
    // A resize, pixels included, reaches the instance's caps.
    ch.window_change(150, 50, 1500, 1000).await.unwrap();
    wait_for(|| t.hooks.widths().contains(&150)).await;
}

#[tokio::test]
async fn a_wrong_password_is_counted_against_the_players_address_only() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    let r = h
        .authenticate_password("dev1+faenix", "wrong")
        .await
        .unwrap();
    assert!(!r.success());
    assert_eq!(
        t.hooks.failures(),
        vec!["127.0.0.1".parse::<std::net::IpAddr>().unwrap()]
    );
    // The same connection can still log in.
    assert!(
        h.authenticate_password("dev1+faenix", "pw")
            .await
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn keyboard_interactive_relays_as_a_password() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    let r = h
        .authenticate_keyboard_interactive_start("dev1+faenix", None)
        .await
        .unwrap();
    let client::KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } = r else {
        panic!("expected a prompt, got {r:?}");
    };
    assert_eq!(prompts.len(), 1);
    let r = h
        .authenticate_keyboard_interactive_respond(vec!["pw".into()])
        .await
        .unwrap();
    assert!(matches!(
        r,
        client::KeyboardInteractiveAuthResponse::Success
    ));
}

#[tokio::test]
async fn a_registered_key_logs_in_and_permits_the_key_only_exec() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    // An unregistered key is refused, and is not a counted failure.
    let other = test_key(9);
    assert!(
        !h.authenticate_publickey("dev1+faenix", with_hash(other))
            .await
            .unwrap()
            .success()
    );
    assert!(t.hooks.failures().is_empty());
    assert!(
        h.authenticate_publickey("dev1+faenix", with_hash(test_key(1)))
            .await
            .unwrap()
            .success()
    );
    // `mind check` is gated on a public-key login at the instance: the
    // attested key made this one.
    let mut ch = h.channel_open_session().await.unwrap();
    ch.exec(true, "mind check").await.unwrap();
    let reply = next_reply(&mut ch).await;
    assert!(
        reply,
        "the instance refused the key-only exec through the doorway"
    );
}

#[tokio::test]
async fn the_menu_routes_a_generic_name_to_a_front_door() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("nick").await.unwrap().success());
    let mut ch = shell(&h).await;
    let menu = read_until(&mut ch, "Type a number or a name").await;
    assert!(menu.contains("dev1"), "{menu}");
    assert!(
        !menu.contains("hidden"),
        "an unlisted service is not shown: {menu}"
    );
    ch.data(&b"\x1b[Azzz\x7f\x7f\x7f1\r"[..]).await.unwrap();
    let out = read_until(&mut ch, "connect <account> <password>").await;
    assert!(
        out.contains("Next time: ssh dev1+<account>@test.host"),
        "{out}"
    );
}

#[tokio::test]
async fn the_menu_reaches_an_unlisted_service_by_name_and_rejects_nonsense() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("nick").await.unwrap().success());
    let mut ch = shell(&h).await;
    read_until(&mut ch, "Type a number").await;
    ch.data(&b"99\r"[..]).await.unwrap();
    read_until(&mut ch, "Unknown choice.").await;
    ch.data(&b"HIDDEN\r"[..]).await.unwrap();
    read_until(&mut ch, "connect <account> <password>").await;
}

#[tokio::test]
async fn the_menu_needs_a_pty_and_exec_is_refused() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("root").await.unwrap().success());
    let mut ch = h.channel_open_session().await.unwrap();
    ch.exec(true, "uname -a").await.unwrap();
    assert!(!next_reply(&mut ch).await);
    let out = read_to_close(&mut ch).await;
    assert!(out.contains("commands are not available"), "{out}");

    let mut h = t.client().await;
    assert!(h.authenticate_none("root").await.unwrap().success());
    let mut ch = h.channel_open_session().await.unwrap();
    ch.request_shell(true).await.unwrap();
    let out = read_to_close(&mut ch).await;
    assert!(out.contains("needs an interactive terminal"), "{out}");
}

#[tokio::test]
async fn generic_names_get_no_password_prompt_and_invalid_names_nothing() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    let r = h.authenticate_password("root", "123456").await.unwrap();
    assert!(
        matches!(&r, russh::client::AuthResult::Failure { remaining_methods, .. } if remaining_methods.is_empty()),
        "{r:?}"
    );
    for bad in ["dev1++x", "dev1+a\nb"] {
        let mut h = t.client().await;
        assert!(refused(h.authenticate_none(bad).await));
        assert!(refused(h.authenticate_password(bad, "pw").await));
    }
    assert!(t.hooks.opened().is_empty(), "no upstream work for scanners");
}

#[tokio::test]
async fn forwarding_and_a_third_channel_are_refused() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("dev1").await.unwrap().success());
    assert!(
        h.channel_open_direct_tcpip("127.0.0.1", 22, "127.0.0.1", 5000)
            .await
            .is_err()
    );
    let _one = shell(&h).await;
    // The instance itself only takes a second channel in game, so the
    // doorway's cap is checked by a third.
    let _two = h.channel_open_session().await;
    let three = h.channel_open_session().await;
    assert!(three.is_err());
    assert!(
        !h.tcpip_forward("127.0.0.1", 0).await.is_ok_and(|p| p > 0),
        "remote forwarding must be refused"
    );
}

#[tokio::test]
async fn a_blocked_address_is_accepted_and_told_why() {
    let t = Rig::start(Hooks {
        blocked: true,
        ..Hooks::default()
    })
    .await;
    let mut h = t.client().await;
    assert!(
        h.authenticate_password("dev1+faenix", "pw")
            .await
            .unwrap()
            .success()
    );
    let mut ch = shell(&h).await;
    let out = read_to_close(&mut ch).await;
    assert!(
        out.contains("Connections from your address are not permitted."),
        "{out}"
    );
}

#[tokio::test]
async fn an_instance_closing_the_session_closes_the_players() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(
        h.authenticate_password("dev1+faenix+Kira", "pw")
            .await
            .unwrap()
            .success()
    );
    let mut ch = shell(&h).await;
    wait_for(|| !t.hooks.connected().is_empty()).await;
    // `@boot`: the binary drops the session's output queue.
    t.hooks.boot_all();
    read_to_close(&mut ch).await;
}

#[tokio::test]
async fn an_unreachable_service_is_told_not_hung() {
    let t = Rig::start(Hooks::default()).await;
    t.connector.fail_next();
    let mut h = t.client().await;
    assert!(h.authenticate_none("dev1").await.unwrap().success());
    let mut ch = shell(&h).await;
    let out = read_to_close(&mut ch).await;
    assert!(out.contains("dev1 is not reachable right now"), "{out}");
}

#[tokio::test]
async fn draining_warns_on_schedule_and_closes_at_the_deadline() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("dev1").await.unwrap().success());
    let mut ch = shell(&h).await;
    read_until(&mut ch, "connect <account>").await;

    t.door.drain.begin();
    let warnings = vec![
        mush_sshdoorway::config::Warning {
            remaining: Duration::from_millis(1500),
            label: "2s".into(),
        },
        mush_sshdoorway::config::Warning {
            remaining: Duration::from_millis(700),
            label: "1s".into(),
        },
    ];
    let door = std::sync::Arc::clone(&t.door);
    let msg = mush_sshdoorway::config::DEFAULT_DRAIN_MESSAGE.to_owned();
    let run = tokio::spawn(async move {
        door.drain
            .run(Duration::from_millis(2500), &warnings, &msg)
            .await;
    });
    // New connections are no longer accepted.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(t.try_client().await.is_err());

    let out = read_to_close(&mut ch).await;
    let first = out
        .find("ATTENTION: Game Doorway updated and requires a restart, please find a safe place and reconnect within the next 2s.")
        .expect("the 2s warning");
    let second = out
        .find("reconnect within the next 1s.")
        .expect("the 1s warning");
    assert!(first < second, "{out}");
    tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
    wait_for(|| t.door.drain.live() == 0).await;
}

#[tokio::test]
async fn a_client_that_stops_reading_is_cut_off_and_memory_stays_bounded() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(
        h.authenticate_password("dev1+faenix+Kira", "pw")
            .await
            .unwrap()
            .success()
    );
    let ch = shell(&h).await;
    wait_for(|| !t.hooks.connected().is_empty()).await;
    // Flood output and never read it: the instance's 5 s write timeout or the
    // doorway's 10 s one ends the session either way.
    let flood = t.hooks.flood(64 * 1024, 4000);
    let started = std::time::Instant::now();
    let _ = tokio::time::timeout(Duration::from_secs(40), flood).await;
    drop(ch);
    wait_for_secs(30, || t.door.drain.live() == 0).await;
    assert!(started.elapsed() < Duration::from_secs(60));
}

#[tokio::test]
async fn the_per_ip_cap_is_told_with_a_banner_and_refused() {
    let t = Rig::start_with(Hooks::default(), |cfg| {
        cfg.limits.per_ip_connections = 1;
    })
    .await;
    let mut a = t.client().await;
    assert!(a.authenticate_none("dev1").await.unwrap().success());
    let mut b = t.client().await;
    assert!(refused(b.authenticate_none("dev1").await));
    assert!(refused(b.authenticate_password("dev1", "x").await));
    wait_for(|| {
        t.banners()
            .iter()
            .any(|m| m.contains("Too many connections"))
    })
    .await;
}

#[tokio::test]
async fn the_rate_limit_drops_tcp_before_ssh() {
    let t = Rig::start_with(Hooks::default(), |cfg| {
        cfg.limits.per_ip_new_per_minute = 3;
    })
    .await;
    for _ in 0..3 {
        drop(t.client().await);
    }
    assert!(t.try_client().await.is_err());
}

#[tokio::test]
async fn sighup_reload_takes_new_routes_and_refuses_listen_changes() {
    let t = Rig::start(Hooks::default()).await;
    let mut cfg = (*t.door.config()).clone();
    cfg.services.retain(|s| s.name != "hidden");
    t.door.reload(cfg.clone()).unwrap();
    assert!(t.door.config().service("hidden").is_none());
    let mut moved = cfg.clone();
    moved.listen.addrs = vec!["127.0.0.2:22".parse().unwrap()];
    assert!(t.door.reload(moved).is_err());
    let mut broken = cfg;
    broken.limits.drain_message = "no placeholder".into();
    assert!(t.door.reload(broken).is_err());
}

#[tokio::test]
async fn the_upstream_reply_is_honoured_for_pty_and_shell() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("dev1").await.unwrap().success());
    let mut ch = h.channel_open_session().await.unwrap();
    // OpenSSH pipelines these; every wanted reply must come back, in order.
    ch.request_pty(true, "xterm", 120, 40, 0, 0, &[])
        .await
        .unwrap();
    ch.set_env(false, "COLORTERM", "truecolor").await.unwrap();
    ch.set_env(false, "LANG", "en_US.UTF-8").await.unwrap();
    ch.request_shell(true).await.unwrap();
    let mut replies = Vec::new();
    while replies.len() < 2 {
        match tokio::time::timeout(Duration::from_secs(10), ch.wait())
            .await
            .unwrap()
        {
            Some(ChannelMsg::Success) => replies.push(true),
            Some(ChannelMsg::Failure) => replies.push(false),
            Some(_) => {}
            None => panic!("channel closed"),
        }
    }
    assert_eq!(replies, [true, true]);
}

#[tokio::test]
async fn an_authenticated_session_that_never_opens_a_channel_is_closed() {
    let t = Rig::start_with(Hooks::default(), |cfg| {
        cfg.limits.auth_deadline_secs = 1;
    })
    .await;
    let mut menu = t.client().await;
    assert!(menu.authenticate_none("nick").await.unwrap().success());
    let mut spliced = t.client().await;
    assert!(spliced.authenticate_none("dev1").await.unwrap().success());
    // Held with no channel: both are cut at the second deadline, so neither
    // a slot nor a drain is held forever.
    wait_for_secs(10, || t.door.drain.live() == 0).await;
    assert!(menu.is_closed() && spliced.is_closed());
}

#[tokio::test]
async fn a_menu_channel_that_never_asks_for_a_shell_times_out() {
    let t = Rig::start_with(Hooks::default(), |cfg| {
        cfg.limits.menu_idle_secs = 1;
    })
    .await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("nick").await.unwrap().success());
    let mut ch = h.channel_open_session().await.unwrap();
    let out = read_to_close(&mut ch).await;
    assert!(out.contains("Menu timed out."), "{out}");
}

#[tokio::test]
async fn an_open_close_loop_is_capped() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("dev1").await.unwrap().success());
    let mut refused = 0;
    for _ in 0..12 {
        match h.channel_open_session().await {
            Ok(ch) => {
                let _ = ch.close().await;
            }
            Err(_) => refused += 1,
        }
    }
    assert!(refused >= 4, "only {refused} of 12 opens refused");
}

#[tokio::test]
async fn switching_service_between_attempts_redials_and_cannot_borrow_a_login() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    // A failed password on one service, then a key for an account the key
    // is not registered to on another: neither may ride the first dial.
    assert!(
        !h.authenticate_password("dev1+faenix", "wrong")
            .await
            .unwrap()
            .success()
    );
    assert!(
        !h.authenticate_publickey("hidden+other", with_hash(test_key(1)))
            .await
            .unwrap()
            .success()
    );
    assert!(
        h.authenticate_password("hidden+faenix", "pw")
            .await
            .unwrap()
            .success()
    );
    // Two dials (dev1, then hidden), each with its own Hello.
    assert_eq!(t.hooks.opened().len(), 2);
}

#[tokio::test]
async fn the_menu_bounds_its_input_line() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("nick").await.unwrap().success());
    let mut ch = shell(&h).await;
    read_until(&mut ch, "Type a number").await;
    ch.data(&vec![b'9'; 100_000][..]).await.unwrap();
    ch.data(&b"\r"[..]).await.unwrap();
    let out = read_until(&mut ch, "Unknown choice.").await;
    // At most MAX_MENU_LINE characters were ever echoed back.
    assert!(
        out.matches('9').count() <= 64,
        "echoed {}",
        out.matches('9').count()
    );
}

/// Adversarial review, critical: the menu channel's unread queue (100) used
/// to wedge the session loop after ~100 keystrokes or resizes.
#[tokio::test]
async fn hundreds_of_resizes_and_keystrokes_at_the_menu_do_not_wedge_it() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("nick").await.unwrap().success());
    let mut ch = shell(&h).await;
    read_until(&mut ch, "Type a number").await;
    // Read the echoes as a terminal would, or this client's own queue fills.
    let (mut rx, tx) = ch.split();
    let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let sink = std::sync::Arc::clone(&out);
    let reader = tokio::spawn(async move {
        while let Some(msg) = rx.wait().await {
            if let ChannelMsg::Data { data } = msg {
                sink.lock().unwrap().extend_from_slice(&data);
            }
        }
    });
    for i in 0..300u32 {
        tx.window_change(100 + i % 50, 40, 0, 0).await.unwrap();
        tx.data(&b"x\x7f"[..]).await.unwrap();
    }
    tx.data(&b"1\r"[..]).await.unwrap();
    wait_for(|| {
        String::from_utf8_lossy(&out.lock().unwrap()).contains("connect <account> <password>")
    })
    .await;
    reader.abort();
}

/// Adversarial review F4: a front door with a character part.
#[tokio::test]
async fn a_front_door_with_a_character_part_still_reaches_the_welcome_screen() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(
        h.authenticate_none("dev1+play+Kira")
            .await
            .unwrap()
            .success()
    );
    let mut ch = shell(&h).await;
    read_until(&mut ch, "connect <account> <password>").await;
}

/// Regression test for F-3: a client that sends the `SSH-2.0-` banner (so
/// `run_stream` returns `Ok(RunningSession)` and enters KEX) and then stalls
/// forever without sending `KEXINIT` or closing its TCP socket must be killed
/// by the watchdog via `KillSwitch` and release its slot and drain guard.
#[tokio::test]
async fn kex_stall_after_ssh_banner_is_aborted_and_releases_slot() {
    use tokio::io::AsyncWriteExt;
    let t = Rig::start_with(Hooks::default(), |cfg| {
        cfg.limits.auth_deadline_secs = 1;
    })
    .await;
    let mut raw = tokio::net::TcpStream::connect(t.addr).await.unwrap();
    raw.write_all(b"SSH-2.0-OpenSSH_9.0\r\n").await.unwrap();
    raw.flush().await.unwrap();
    wait_for(|| t.door.limits.live() == 1 && t.door.drain.live() == 1).await;
    // Hold `raw` open so the client never sends EOF; the watchdog must abort
    // the underlying Killable<TcpStream> even while `kex.active()` is true.
    wait_for_secs(6, || t.door.limits.live() == 0 && t.door.drain.live() == 0).await;
    drop(raw);
}

/// Regression test for F-1: over-cap connections are capped at
/// `MAX_UNCOUNTED_PER_KEY` (2) simultaneous banner slots per IP key (further
/// concurrent connections are dropped at TCP accept without claiming global
/// slots), and half-open over-cap clients are forcibly aborted after refusal.
#[tokio::test]
async fn over_cap_banner_slots_are_bounded_and_aborted_when_half_open() {
    let t = Rig::start_with(Hooks::default(), |cfg| {
        cfg.limits.per_ip_connections = 1;
        cfg.limits.auth_deadline_secs = 2;
    })
    .await;
    let mut a = t.client().await;
    assert!(a.authenticate_none("dev1").await.unwrap().success());
    let _ch = shell(&a).await;

    // Two simultaneous over-cap connections are admitted to deliver the banner.
    let mut b = t.client().await;
    let _c = t.client().await;
    assert_eq!(t.door.limits.live(), 3);

    // A third simultaneous over-cap connection from the same IP is dropped at
    // TCP accept without claiming a global slot.
    assert!(t.try_client().await.is_err());
    assert_eq!(t.door.limits.live(), 3);

    // Refuse `b` and leave both `b` and `_c` open without disconnecting from
    // the client side; the doorway aborts both within a few seconds.
    assert!(refused(b.authenticate_none("dev1").await));
    wait_for_secs(8, || t.door.limits.live() == 1).await;
}

/// Regression test for F-2: a burst of invalid menu submissions (or a client
/// that never reads menu output) is cut off at `MAX_MENU_TRIES` and aborted.
#[tokio::test]
async fn menu_invalid_submission_flood_is_capped_and_closed() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("nick").await.unwrap().success());
    let mut ch = shell(&h).await;
    read_until(&mut ch, "Type a number").await;
    let payload = b"99\r".repeat(32);
    let _ = ch.data(&payload[..]).await;
    let out = read_to_close(&mut ch).await;
    assert!(out.contains("Too many invalid choices."), "{out}");
    wait_for_secs(6, || t.door.drain.live() == 0).await;
}

/// Regression test for F-4: repeated public-key offers against a blocked
/// service or alternating services cannot dial upstream more than `MAX_DIALS`
/// times on one connection.
#[tokio::test]
async fn publickey_probes_and_service_switches_cap_upstream_dials() {
    let t = Rig::start(Hooks {
        blocked: true,
        ..Hooks::default()
    })
    .await;
    let mut h = t.client().await;
    // Against a blocked service, the first key offer dials once and learns the
    // notice; subsequent offers before signing are rejected without re-dialing.
    assert!(
        h.authenticate_publickey("dev1+faenix", with_hash(test_key(1)))
            .await
            .unwrap()
            .success()
    );
    assert_eq!(t.hooks.opened().len(), 1);
    let mut ch = shell(&h).await;
    let out = read_to_close(&mut ch).await;
    assert!(
        out.contains("Connections from your address are not permitted."),
        "{out}"
    );

    // Alternating services on a single connection cannot exceed MAX_DIALS (4).
    let t2 = Rig::start(Hooks::default()).await;
    let mut h2 = t2.client().await;
    for i in 0..8u8 {
        let user = if i % 2 == 0 {
            "dev1+faenix"
        } else {
            "hidden+faenix"
        };
        let _ = h2
            .authenticate_publickey(user, with_hash(test_key(10 + i)))
            .await;
    }
    assert!(
        t2.hooks.opened().len() <= mush_sshdoorway::server::MAX_DIALS,
        "opened {} upstream connections, expected <= {}",
        t2.hooks.opened().len(),
        mush_sshdoorway::server::MAX_DIALS
    );
}

/// Regression test for F-6: a player whose `ssh-agent` offers 6 unregistered
/// keys before falling back to password can still log in with their password.
#[tokio::test]
async fn multi_key_ssh_agent_falls_through_to_password_without_lockout() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    for seed in 10..16 {
        assert!(
            !h.authenticate_publickey("dev1+faenix", with_hash(test_key(seed)))
                .await
                .unwrap()
                .success()
        );
    }
    assert!(
        h.authenticate_password("dev1+faenix", "pw")
            .await
            .unwrap()
            .success()
    );
}

/// Regression test for F-11: a `SIGHUP` config reload takes effect immediately
/// for a session that is already sitting at the menu.
#[tokio::test]
async fn sighup_reload_updates_an_open_menu_session() {
    let t = Rig::start(Hooks::default()).await;
    let mut h = t.client().await;
    assert!(h.authenticate_none("nick").await.unwrap().success());
    let mut ch = shell(&h).await;
    read_until(&mut ch, "Type a number").await;

    let mut cfg = (*t.door.config()).clone();
    cfg.services.push(mush_sshdoorway::config::ServiceCfg {
        name: "dev2".into(),
        title: "Development server 2".into(),
        socket: "unused".into(),
        listed: true,
    });
    t.door.reload(cfg).unwrap();

    ch.data(&b"dev2\r"[..]).await.unwrap();
    let out = read_until(&mut ch, "connect <account> <password>").await;
    assert!(
        out.contains("Next time: ssh dev2+<account>@test.host"),
        "{out}"
    );
}
