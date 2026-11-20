use super::*;
use russh::Channel;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

const PANE: &str = "0123456789abcdeffedcba9876543210";
async fn game(h: &client::Handle<Cli>, mode: &str) -> Channel<client::Msg> {
    let mut ch = h.channel_open_session().await.unwrap();
    ch.request_pty(true, "xterm-256color", 120, 40, 1200, 800, &[])
        .await
        .unwrap();
    assert!(next_reply(&mut ch).await);
    ch.set_env(
        true,
        mush_net_ssh::resume::ENV_NAME,
        format!("{mode}:{PANE}"),
    )
    .await
    .unwrap();
    assert!(next_reply(&mut ch).await);
    ch.request_shell(true).await.unwrap();
    assert!(next_reply(&mut ch).await);
    ch
}
async fn side(h: &client::Handle<Cli>, mode: &str, ticket: Option<&str>) -> Channel<client::Msg> {
    let mut ch = h.channel_open_session().await.unwrap();
    ch.request_subsystem(true, mush_net_ssh::resume::SUBSYSTEM)
        .await
        .unwrap();
    assert!(next_reply(&mut ch).await);
    let mut request = json!({"v":1,"op":mode,"pane":PANE});
    if let Some(ticket) = ticket {
        request["ticket"] = json!(ticket);
    }
    let body = serde_json::to_vec(&request).unwrap();
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    for part in bytes.chunks(11) {
        ch.data(part).await.unwrap();
    }
    ch
}
async fn reply(ch: &mut Channel<client::Msg>) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut reader = ch.make_reader();
        let size = reader.read_u32().await.unwrap() as usize;
        assert!((1..=1024).contains(&size));
        let mut body = vec![0; size];
        reader.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    })
    .await
    .expect("sideband reply")
}
async fn verify_path(issue_routed: bool) {
    let rig = Rig::start(Hooks::default()).await;
    let direct = rig.direct_listener().await;
    let first = if issue_routed { rig.addr } else { direct };
    let second = if issue_routed { direct } else { rig.addr };
    let (mut initial, cut) = rig.droppable_client(first).await;
    let welcome = if issue_routed { "dev1" } else { "play" };
    assert!(initial.authenticate_none(welcome).await.unwrap().success());
    let mut shell = game(&initial, "issue").await;
    let mut exchange = side(&initial, "issue", None).await;
    assert_eq!(reply(&mut exchange).await["status"], "auth_required");
    shell.data(&b"connect faenix pw\r"[..]).await.unwrap();
    let result = reply(&mut exchange).await;
    assert_eq!(result["status"], "issued");
    let ticket = result["ticket"].as_str().unwrap();
    assert_eq!(ticket.len(), 43);
    assert_eq!(rig.hooks.connected().len(), 1);
    shell.data(&b"@sounds off\r"[..]).await.unwrap();
    read_until(&mut shell, "Sounds are off").await;
    shell.data(&b"@pixel/encoder off\r"[..]).await.unwrap();
    read_until(&mut shell, "Picture encoder for this connection: off").await;
    if issue_routed {
        shell.data(&b"@tui\r"[..]).await.unwrap();
        read_until(&mut shell, "\x1b[?1049h").await;
    }
    // Drop the TCP transport without sending shell EOF/CLOSE.
    cut.shutdown(std::net::Shutdown::Both).unwrap();
    wait_for(|| rig.hooks.state.lock().unwrap().disconnected.len() == 1).await;
    let (mut next, _cut) = rig.droppable_client(second).await;
    if issue_routed {
        assert!(next.authenticate_none("play").await.unwrap().success());
    } else {
        assert!(
            next.authenticate_password("dev1+faenix+Kira", "pw")
                .await
                .unwrap()
                .success()
        );
    }
    let mut shell = game(&next, "redeem").await;
    let mut exchange = side(&next, "redeem", Some(ticket)).await;
    if issue_routed {
        assert_eq!(reply(&mut exchange).await["status"], "auth_required");
        shell.data(&b"connect faenix pw\r"[..]).await.unwrap();
    }
    let restored = reply(&mut exchange).await;
    assert_eq!(restored["status"], "redeemed");
    assert!(
        restored["ticket"]
            .as_str()
            .is_some_and(|new| new.len() == 43 && new != ticket)
    );
    let mut vt = String::new();
    if issue_routed {
        vt += &read_until(&mut shell, "\x1b[?1049h").await;
        shell.data(&b"\x1d"[..]).await.unwrap();
        vt += &read_until(&mut shell, "\x1b[?1049l").await;
    }
    shell.data(&b"@sounds\r"[..]).await.unwrap();
    vt += &read_until(&mut shell, "Sounds are off").await;
    shell.data(&b"@pixel/encoder\r"[..]).await.unwrap();
    vt += &read_until(&mut shell, "Picture encoder: off").await;
    if !issue_routed {
        assert!(
            !vt.contains("\x1b[?1049h"),
            "a line player stays in line mode"
        );
    }
    assert!(!vt.contains(ticket));
    assert!(!vt.contains(restored["ticket"].as_str().unwrap()));
    assert_eq!(
        rig.hooks.connected().len(),
        1,
        "redeem must skip normal character entry"
    );
    assert!(
        rig.hooks.lines().is_empty(),
        "restoration must not issue game commands"
    );
    shell.eof().await.unwrap();
    wait_for(|| rig.hooks.state.lock().unwrap().disconnected.len() == 2).await;
    let (mut after_close, _cut) = rig.droppable_client(direct).await;
    assert!(
        after_close
            .authenticate_password("faenix+Kira", "pw")
            .await
            .unwrap()
            .success()
    );
    let _shell = game(&after_close, "redeem").await;
    let mut revoked = side(&after_close, "redeem", restored["ticket"].as_str()).await;
    assert!(matches!(
        reply(&mut revoked).await["status"].as_str(),
        Some("denied" | "expired")
    ));
}
#[tokio::test]
async fn cockpit_issue_direct_redeem_via_doorway() {
    verify_path(false).await;
}
#[tokio::test]
async fn cockpit_issue_via_doorway_redeem_direct() {
    verify_path(true).await;
}

#[tokio::test]
async fn refused_resume_reissues_on_the_same_direct_connection() {
    verify_reissue_after_refusal(false).await;
}
#[tokio::test]
async fn refused_resume_reissues_on_the_same_doorway_connection() {
    verify_reissue_after_refusal(true).await;
}

async fn verify_reissue_after_refusal(routed: bool) {
    let rig = Rig::start(Hooks::default()).await;
    let (addr, user) = if routed {
        (rig.addr, "dev1+faenix+Kira")
    } else {
        (rig.direct_listener().await, "faenix+Kira")
    };
    let (mut handle, _cut) = rig.droppable_client(addr).await;
    assert!(
        handle
            .authenticate_password(user, "pw")
            .await
            .unwrap()
            .success()
    );
    let shell = game(&handle, "redeem").await;
    let mut exchange = side(&handle, "redeem", Some(&"A".repeat(43))).await;
    let response = reply(&mut exchange).await;
    assert!(matches!(
        response["status"].as_str(),
        Some("denied" | "expired")
    ));
    assert!(rig.hooks.connected().is_empty());
    // The doorway permits a shell and one auxiliary channel. Queue CLOSE
    // on this connection before opening the fresh Issue subsystem.
    exchange.close().await.unwrap();
    let mut fresh = side(&handle, "issue", None).await;
    assert_eq!(reply(&mut fresh).await["status"], "auth_required");
    shell.data(&b"1\r"[..]).await.unwrap();
    wait_for(|| rig.hooks.connected().len() == 1).await;
    assert_eq!(reply(&mut fresh).await["status"], "issued");
    assert!(rig.hooks.lines().is_empty());
}

/// No packet follows the shell/partial request until the server has attached.
async fn fallback_without_more_input(user: &str, incomplete: bool) {
    let rig = Rig::start(Hooks::default()).await;
    let (mut h, _cut) = rig.droppable_client(rig.direct_listener().await).await;
    assert!(h.authenticate_password(user, "pw").await.unwrap().success());
    let shell = game(&h, "redeem").await;
    let mut pending = None;
    if incomplete {
        let mut ch = h.channel_open_session().await.unwrap();
        ch.request_subsystem(true, mush_net_ssh::resume::SUBSYSTEM)
            .await
            .unwrap();
        assert!(next_reply(&mut ch).await);
        ch.data(&[0u8, 0][..]).await.unwrap();
        pending = Some(ch);
    }
    wait_for(|| rig.hooks.connected().len() == 1).await;
    assert_eq!(rig.hooks.connected(), vec![("faenix".into(), 1)]);
    shell.data(&b"look\r"[..]).await.unwrap();
    wait_for(|| rig.hooks.lines() == ["look"]).await;
    if let Some(ch) = pending {
        ch.close().await.unwrap();
    }
    let mut fresh = side(&h, "issue", None).await;
    assert_eq!(reply(&mut fresh).await["status"], "issued");
}

#[tokio::test]
async fn redeem_hint_alone_preserves_requested_character_login() {
    fallback_without_more_input("faenix+Kira", false).await;
}
#[tokio::test]
async fn unfinished_redeem_frame_preserves_single_character_login() {
    fallback_without_more_input("faenix", true).await;
}

#[tokio::test]
async fn cockpit_and_sftp_role_assignment_follows_subsystem_requests() {
    let rig = Rig::start(Hooks::default()).await;
    let (mut h, _cut) = rig.droppable_client(rig.direct_listener().await).await;
    assert!(
        h.authenticate_password("faenix+Kira", "pw")
            .await
            .unwrap()
            .success()
    );
    let _shell = game(&h, "issue").await;
    let mut first = h.channel_open_session().await.unwrap();
    let mut second = h.channel_open_session().await.unwrap();
    second
        .request_subsystem(true, mush_net_ssh::resume::SUBSYSTEM)
        .await
        .unwrap();
    assert!(next_reply(&mut second).await);
    first.request_subsystem(true, "sftp").await.unwrap();
    assert!(next_reply(&mut first).await);
    assert!(h.channel_open_session().await.is_err());
}
