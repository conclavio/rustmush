//! Integration tests for the M4 web bridge (`mushd::web_bridge`), driven
//! hooks-level with no sockets: structured connections are plain mpsc
//! channels of stamped [`Envelope`]s, exactly as `mush-net-web`'s WS pump
//! would hold them.
//!
//! Covers: token mint/validate/slide, logout, register→create (through the
//! shared `bridge_common` flow into a live structured session), foreign
//! character/bad token refusals, resume replay + the too-old re-hydration
//! path, the `Req` → `Inbound::Query` → `Reply` mapping against the real
//! comsys/mail query handlers, and the `@newpassword` end-to-end flow
//! including logout-everywhere token invalidation.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{Engine, GOD, Inbound, ObjId};
use mush_net_web::{LoginErr, NewWebConn, ResumeErr, WebHooks};
use mush_proto::{Envelope, OutputEvent, Query};
use mush_session::{ConnId, SessionConfig, SessionManager};
use mush_store::{InMemoryStore, ObjectStore, TokenRecord};
use mush_testkit::text_of;
use tokio::sync::mpsc;

use mushd::accounts::{Accounts, AuthBackend, AuthOutcome};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::web_bridge::{TOKEN_TTL_S, WebBridge};
use mushd::world_task::{self, PersistHandles, ServerDeps};

type EnvRx = mpsc::Receiver<Arc<Envelope>>;

fn ip(last: u8) -> IpAddr {
    IpAddr::from([127, 0, 0, last])
}

/// The whole server wiring minus sockets: store-seeded world, the real
/// module roster (comsys/mail/jobs — their query handlers included), staff
/// commands, the persist world task, store-backed accounts, and the web
/// bridge with the same panel manifest `main.rs` builds.
struct Rig {
    store: Arc<dyn ObjectStore>,
    sessions: Arc<SessionManager>,
    guard: Arc<IpGuard>,
    accounts: Arc<Accounts>,
    bridge: WebBridge<Accounts>,
    handles: PersistHandles,
}

async fn rig_with(config: SessionConfig) -> Rig {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let world = load_or_seed_world(&*store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, modules::server_modules()).expect("modules");
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");
    let mut panels: Vec<String> = engine
        .registry
        .panel_feeds()
        .iter()
        .map(|(f, _)| f.to_string())
        .collect();
    panels.push("who".to_string());
    let sessions = Arc::new(SessionManager::with_config(config));
    let guard = Arc::new(IpGuard::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
            llm: None,
        },
    );
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 3));
    let bridge = WebBridge::new(
        Arc::clone(&accounts),
        Arc::clone(&sessions),
        handles.handle.clone(),
        Arc::clone(&guard),
        Arc::clone(&store),
        panels,
    );
    Rig {
        store,
        sessions,
        guard,
        accounts,
        bridge,
        handles,
    }
}

async fn rig() -> Rig {
    rig_with(SessionConfig::default()).await
}

/// Receive envelopes until one's event text contains `needle`.
async fn expect_env(rx: &mut EnvRx, needle: &str) -> Arc<Envelope> {
    let fut = async {
        loop {
            match rx.recv().await {
                Some(env) => {
                    if text_of(&env.event).contains(needle) {
                        return env;
                    }
                }
                None => panic!("envelope channel closed while waiting for {needle:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), fut)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"))
}

fn web_conn(id: u64, last_octet: u8) -> (NewWebConn, EnvRx) {
    let (tx, rx) = mpsc::channel(256);
    (
        NewWebConn {
            id,
            addr: ip(last_octet),
            tx,
        },
        rx,
    )
}

/// Register an account+character through the bridge, returning the token
/// and the character key.
async fn register(rig: &Rig, account: &str, password: &str, char_name: &str) -> (String, u64) {
    let ok = rig
        .bridge
        .register(account, password, char_name, ip(1))
        .await
        .expect("register");
    assert_eq!(ok.account, account);
    assert_eq!(ok.characters.len(), 1);
    assert_eq!(ok.characters[0].name, char_name);
    (ok.token, ok.characters[0].key)
}

// ---------------------------------------------------------------------------
// Tokens: mint / validate / slide / logout
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn login_mints_a_sliding_hashed_token() {
    let rig = rig().await;
    let (_token, _key) = register(&rig, "nick", "hunter42", "Nick").await;

    // A real login (argon2 verify) mints a fresh token.
    let ok = rig
        .bridge
        .login("NICK", "hunter42", ip(1))
        .await
        .expect("login");
    assert_eq!(ok.account, "nick", "canonical (registered) case");
    assert_eq!(ok.token.len(), 64);
    assert_eq!(ok.characters[0].name, "Nick");

    // The raw token is NOT the store key: only its SHA-256 is.
    assert!(
        rig.store.token_get(&ok.token).await.unwrap().is_none(),
        "raw token must not key the store"
    );

    // Shrink the stored expiry, then validate: the TTL must slide back out
    // to ~now + 7 days (sliding refresh happens inside validate_token).
    let hash = {
        // Locate the record by scanning is not possible through the trait;
        // recompute the hash the bridge used.
        mushd_token_hash(&ok.token)
    };
    let rec = rig.store.token_get(&hash).await.unwrap().expect("stored");
    assert_eq!(rec.account, "nick");
    let now = mushd::unix_now_s();
    assert!((rec.expires - (now + TOKEN_TTL_S)).abs() <= 5, "fresh TTL");
    rig.store
        .token_put(
            hash.clone(),
            TokenRecord {
                expires: now + 60,
                ..rec
            },
        )
        .await
        .unwrap();

    let identity = rig
        .bridge
        .validate_token(&ok.token)
        .await
        .expect("valid token");
    assert_eq!(identity.account, "nick");
    assert_eq!(identity.characters[0].name, "Nick");
    let slid = rig
        .store
        .token_get(&hash)
        .await
        .unwrap()
        .expect("still there");
    assert!(
        slid.expires >= now + TOKEN_TTL_S - 5,
        "validation must slide the TTL back out (got {})",
        slid.expires
    );

    // Garbage tokens validate to None.
    assert!(rig.bridge.validate_token("deadbeef").await.is_none());
}

/// The bridge's hash must match the store's key derivation; recompute it
/// the same way (sha256 hex) without reaching into private API.
fn mushd_token_hash(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(token.as_bytes());
    let mut out = String::new();
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn login_maps_bad_creds_bans_and_tempbans() {
    let rig = rig().await;
    register(&rig, "nick", "hunter42", "Nick").await;

    assert_eq!(
        rig.bridge.login("nick", "wrong", ip(1)).await.unwrap_err(),
        LoginErr::BadCreds
    );
    assert_eq!(
        rig.bridge
            .login("ghost", "hunter42", ip(1))
            .await
            .unwrap_err(),
        LoginErr::BadCreds,
        "unknown account is indistinguishable from wrong password"
    );

    // Tempban (auth-failure window) → RateLimited.
    for _ in 0..5 {
        rig.bridge.auth_failed(ip(7));
    }
    assert!(rig.bridge.is_ip_blocked(ip(7)));
    assert_eq!(
        rig.bridge
            .login("nick", "hunter42", ip(7))
            .await
            .unwrap_err(),
        LoginErr::RateLimited
    );

    // Siteban → Banned.
    rig.guard.set_siteban(ip(8), None);
    assert_eq!(
        rig.bridge
            .login("nick", "hunter42", ip(8))
            .await
            .unwrap_err(),
        LoginErr::Banned
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn logout_revokes_one_token_only() {
    let rig = rig().await;
    let (t1, _) = register(&rig, "nick", "hunter42", "Nick").await;
    let t2 = rig
        .bridge
        .login("nick", "hunter42", ip(1))
        .await
        .unwrap()
        .token;

    rig.bridge.logout(&t1).await;
    assert!(rig.bridge.validate_token(&t1).await.is_none());
    assert!(rig.bridge.validate_token(&t2).await.is_some());
    // Idempotent.
    rig.bridge.logout(&t1).await;
}

// ---------------------------------------------------------------------------
// register → connect (the shared bridge_common flow, structured wire)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn register_then_connect_streams_stamped_envelopes() {
    let rig = rig().await;
    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;

    // The account exists store-side with the character linked.
    let rec = rig
        .store
        .get_account("nick")
        .await
        .unwrap()
        .expect("account");
    assert_eq!(rec.characters.len(), 1);
    assert_eq!(rec.characters[0].name, "Nick");

    let (conn, mut rx) = web_conn(1_000, 1);
    let start = rig
        .bridge
        .connect_character(conn, &token, key)
        .await
        .expect("connect");
    assert_eq!(start.resume_from, 0, "fresh session starts at seq 0");

    // The entry output arrives as stamped envelopes: SessionInfo, then the
    // initial room render — with monotonic seqs starting at 1.
    let si = expect_env(&mut rx, "Connected as Nick (nick)").await;
    let room = expect_env(&mut rx, "Limbo(#0)").await;
    assert!(matches!(si.event, OutputEvent::SessionInfo { .. }));
    assert!(matches!(room.event, OutputEvent::Room(_)));
    assert!(si.seq >= 1);
    assert!(room.seq > si.seq, "seqs are monotonic per session");
    assert!(si.ts > 0);

    // Lines flow through the same shared line path as SSH.
    rig.bridge.line(1_000, "say over the web".to_string()).await;
    expect_env(&mut rx, "over the web").await;
}

/// MEDIUM finding: SSH's `LineEditor` never lets a submitted line exceed
/// `MAX_LINE_CHARS`, but the web path had no equivalent gate short of the
/// 64 KiB WS frame cap. `GameBridge::line` (shared by both transports) must
/// reject an over-long web line with a `Sys` refusal instead of forwarding
/// it to the world.
#[tokio::test(flavor = "multi_thread")]
async fn line_over_the_char_cap_is_rejected_before_reaching_the_world() {
    let rig = rig().await;
    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    let (conn, mut rx) = web_conn(1_050, 1);
    rig.bridge
        .connect_character(conn, &token, key)
        .await
        .expect("connect");
    expect_env(&mut rx, "Limbo(#0)").await;

    let huge = "a".repeat(mush_net_ssh::MAX_LINE_CHARS + 1);
    rig.bridge.line(1_050, huge).await;
    expect_env(&mut rx, "Line too long").await;

    // A line exactly at the cap is unaffected (still processed normally).
    rig.bridge.line(1_050, "say still alive".to_string()).await;
    expect_env(&mut rx, "still alive").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_rejects_bad_tokens_and_foreign_characters() {
    let rig = rig().await;
    let (token, _key) = register(&rig, "nick", "hunter42", "Nick").await;
    let (_zoe_token, zoe_key) = register(&rig, "zoe", "hunter42", "Zoe").await;

    let (conn, _rx) = web_conn(1_001, 1);
    assert!(
        rig.bridge
            .connect_character(conn, "not-a-token", zoe_key)
            .await
            .is_err()
    );
    let (conn, _rx) = web_conn(1_002, 1);
    assert!(
        rig.bridge
            .connect_character(conn, &token, zoe_key)
            .await
            .is_err(),
        "nick's token must not attach zoe's character"
    );
    assert_eq!(rig.sessions.connection_count(), 0);
}

// ---------------------------------------------------------------------------
// Resume
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn resume_replays_the_gap_in_order() {
    let rig = rig().await;
    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    let (conn, mut rx) = web_conn(1_100, 1);
    rig.bridge
        .connect_character(conn, &token, key)
        .await
        .expect("connect");
    let last_seen = expect_env(&mut rx, "Limbo(#0)").await.seq;

    // The link drops (linkdead grace holds the session), and output keeps
    // flowing into the resume ring while nobody is listening.
    rig.bridge.disconnected(1_100).await;
    drop(rx);
    for i in 0..3 {
        assert!(
            rig.handles
                .handle
                .inbound(Inbound::Cmd {
                    conn: 9_999, // no live conn: input path not under test
                    actor: ObjId(u32::try_from(key).unwrap()),
                    line: format!("think missed-{i}"),
                })
                .await
        );
    }
    rig.handles.handle.barrier().await;

    let (conn, mut rx2) = web_conn(1_101, 1);
    let ok = rig
        .bridge
        .resume(conn, &token, key, last_seen)
        .await
        .expect("resume");
    let texts: Vec<String> = ok.replayed.iter().map(|e| text_of(&e.event)).collect();
    for i in 0..3 {
        assert!(
            texts.iter().any(|t| t.contains(&format!("missed-{i}"))),
            "missed-{i} must replay; got {texts:?}"
        );
    }
    // Replay is ordered, gap-free from last_seen, and live_from points one
    // past the newest replayed seq.
    let seqs: Vec<u64> = ok.replayed.iter().map(|e| e.seq).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "ordered: {seqs:?}");
    assert!(seqs.first().is_some_and(|&s| s > last_seen));
    assert_eq!(ok.live_from, seqs.last().unwrap() + 1);

    // The revived session is live: new output arrives on the new conn.
    rig.bridge.line(1_101, "say back again".to_string()).await;
    expect_env(&mut rx2, "back again").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_too_old_and_reaped_sessions_rehydrate() {
    // Tiny ring: 4 envelopes, so a handful of lines evicts the gap.
    let rig = rig_with(SessionConfig {
        resume_ring: 4,
        ..SessionConfig::default()
    })
    .await;
    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    let (conn, mut rx) = web_conn(1_200, 1);
    rig.bridge
        .connect_character(conn, &token, key)
        .await
        .expect("connect");
    let last_seen = expect_env(&mut rx, "Limbo(#0)").await.seq;
    rig.bridge.disconnected(1_200).await;
    drop(rx);
    for i in 0..10 {
        rig.handles
            .handle
            .inbound(Inbound::Cmd {
                conn: 9_999,
                actor: ObjId(u32::try_from(key).unwrap()),
                line: format!("think flood-{i}"),
            })
            .await;
    }
    rig.handles.handle.barrier().await;

    // The gap outgrew the ring: TooOld, and nothing was attached — the
    // transport falls back to connect_character with the same conn id.
    let (conn, _rx) = web_conn(1_201, 1);
    assert!(matches!(
        rig.bridge.resume(conn, &token, key, last_seen).await,
        Err(ResumeErr::TooOld)
    ));
    assert_eq!(rig.sessions.connection_count(), 0, "TooOld must not attach");
    let (conn, mut rx) = web_conn(1_201, 1);
    rig.bridge
        .connect_character(conn, &token, key)
        .await
        .expect("re-hydration connect");
    expect_env(&mut rx, "Limbo(#0)").await;

    // A reaped session (no session at all) also reports TooOld — the same
    // graceful re-hydration path, not a hard close.
    rig.bridge.disconnected(1_201).await;
    drop(rx);
    for handle in rig
        .sessions
        .session_of_char(mushd::world_task::char_key(ObjId(
            u32::try_from(key).unwrap(),
        )))
        .map(|s| rig.sessions.close_session(s))
        .unwrap_or_default()
    {
        drop(handle);
    }
    let (conn, _rx) = web_conn(1_202, 1);
    assert!(matches!(
        rig.bridge.resume(conn, &token, key, 0).await,
        Err(ResumeErr::TooOld)
    ));

    // A bad token is BadSession, never TooOld.
    let (conn, _rx) = web_conn(1_203, 1);
    assert!(matches!(
        rig.bridge.resume(conn, "bogus", key, 0).await,
        Err(ResumeErr::BadSession)
    ));
}

// ---------------------------------------------------------------------------
// Req → Inbound::Query → Reply (against the real module query handlers)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn requests_round_trip_to_correlated_replies() {
    let rig = rig().await;
    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    let (conn, mut rx) = web_conn(1_300, 1);
    rig.bridge
        .connect_character(conn, &token, key)
        .await
        .expect("connect");
    expect_env(&mut rx, "Limbo(#0)").await;

    // Who → Reply { id, Who } with our character listed.
    rig.bridge.request(1_300, 7, Query::Who).await;
    let env = expect_env(&mut rx, "[reply 7]").await;
    let OutputEvent::Reply { id: 7, event } = &env.event else {
        panic!("expected Reply 7, got {:?}", env.event);
    };
    let OutputEvent::Who(entries) = event.as_ref() else {
        panic!("expected Who, got {event:?}");
    };
    assert!(entries.iter().any(|e| e.name == "Nick"));

    // MailList → Reply { Panel feed "mail" } with the folder summaries.
    rig.bridge
        .request(1_300, 8, Query::MailList { folder: None })
        .await;
    let env = expect_env(&mut rx, "[reply 8]").await;
    let OutputEvent::Reply { id: 8, event } = &env.event else {
        panic!("expected Reply 8, got {:?}", env.event);
    };
    let OutputEvent::Panel { feed, data, .. } = event.as_ref() else {
        panic!("expected Panel, got {event:?}");
    };
    assert_eq!(feed, "mail");
    assert_eq!(data["folder"], "inbox");
    assert!(
        data["folders"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "inbox")
    );

    // Speak on Public (auto-joined at creation), then pull the backlog.
    rig.bridge
        .line(1_300, "pub hello backlog".to_string())
        .await;
    expect_env(&mut rx, "hello backlog").await;
    rig.bridge
        .request(
            1_300,
            9,
            Query::ChannelBacklog {
                channel: "Public".into(),
                before_seq: None,
                limit: 10,
            },
        )
        .await;
    let env = expect_env(&mut rx, "[reply 9]").await;
    let OutputEvent::Reply { id: 9, event } = &env.event else {
        panic!("expected Reply 9, got {:?}", env.event);
    };
    let OutputEvent::Panel { feed, data, .. } = event.as_ref() else {
        panic!("expected Panel, got {event:?}");
    };
    assert_eq!(feed, "comsys");
    let messages = data["backlog"]["messages"].as_array().unwrap();
    assert!(
        messages
            .iter()
            .any(|m| m["text"].as_str().unwrap().contains("hello backlog")),
        "{messages:?}"
    );

    // PanelSubscribe("comsys") acks with the comsys.state keyframe.
    rig.bridge
        .request(
            1_300,
            10,
            Query::PanelSubscribe {
                feed: "comsys".into(),
            },
        )
        .await;
    let env = expect_env(&mut rx, "[reply 10]").await;
    let OutputEvent::Reply { id: 10, event } = &env.event else {
        panic!("expected Reply 10, got {:?}", env.event);
    };
    let OutputEvent::Panel { feed, data, .. } = event.as_ref() else {
        panic!("expected Panel keyframe, got {event:?}");
    };
    assert_eq!(feed, "comsys");
    let chans = data["channels"].as_array().unwrap();
    let public = chans
        .iter()
        .find(|c| c["name"] == "Public")
        .expect("joined Public");
    assert_eq!(public["alias"], "pub");
    assert_eq!(public["color"]["web_hex"], "#00ffff");
    assert_eq!(public["unread"], 0, "own message was delivered live");

    // An unknown panel feed refuses politely, correlated.
    rig.bridge
        .request(
            1_300,
            11,
            Query::PanelSubscribe {
                feed: "btech.map".into(),
            },
        )
        .await;
    let env = expect_env(&mut rx, "[reply 11]").await;
    assert!(text_of(&env.event).contains("No such panel feed"));

    // No ruleset is loaded in this bridge rig, so there is no `<ruleset>.sheet`
    // feed to route to; the sheet query degrades to a polite reply (with a
    // ruleset loaded it routes to the handler — see web_e2e's
    // `sheet_query_routes_to_the_ruleset_over_web`).
    rig.bridge.request(1_300, 12, Query::Sheet).await;
    let env = expect_env(&mut rx, "[reply 12]").await;
    assert!(text_of(&env.event).contains("not available here"));
}

// ---------------------------------------------------------------------------
// @newpassword end-to-end (staff command → store update → token revocation)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn newpassword_rehashes_and_revokes_web_sessions() {
    let rig = rig().await;
    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    let token2 = rig
        .bridge
        .login("nick", "hunter42", ip(1))
        .await
        .unwrap()
        .token;

    // Nick is connected over the web and will be told about the change.
    let (conn, mut nick_rx) = web_conn(1_400, 1);
    rig.bridge
        .connect_character(conn, &token, key)
        .await
        .expect("connect");
    expect_env(&mut nick_rx, "Limbo(#0)").await;

    // God runs @newpassword from a (structured, for convenience) conn,
    // attached directly — God has no account to log in with.
    let (god_tx, mut god_rx) = mpsc::channel(256);
    let handle = mush_session::ConnectionHandle::new_structured(
        ConnId(1_401),
        ip(2),
        mush_proto::ClientCaps {
            kind: mush_proto::ClientKind::Web,
            color: mush_proto::ColorDepth::TrueColor,
            width: 80,
            height: 24,
            utf8: true,
            structured: true,
        },
        god_tx,
    );
    rig.sessions
        .attach(handle, mushd::world_task::char_key(GOD), "god".to_string());
    assert!(
        rig.handles
            .handle
            .inbound(Inbound::ConnUp {
                conn: 1_401,
                actor: GOD,
                caps_structured: true,
                from_linkdead: false,
            })
            .await
    );

    // Bad target/password validation happens synchronously in the handler.
    rig.handles
        .handle
        .inbound(Inbound::Cmd {
            conn: 1_401,
            actor: GOD,
            line: "@newpassword Nobody=whatever".to_string(),
        })
        .await;
    expect_env(&mut god_rx, "I don't recognize that player.").await;
    rig.handles
        .handle
        .inbound(Inbound::Cmd {
            conn: 1_401,
            actor: GOD,
            line: "@newpassword Nick=abc".to_string(),
        })
        .await;
    expect_env(&mut god_rx, "passwords must be 4-128 characters").await;

    // The real change: ack-gated confirmation, target notification, both
    // web tokens revoked, old password dead, new password live.
    rig.handles
        .handle
        .inbound(Inbound::Cmd {
            conn: 1_401,
            actor: GOD,
            line: "@newpassword Nick=n3w-pass".to_string(),
        })
        .await;
    let confirm = expect_env(&mut god_rx, "Password for Nick (account nick) changed").await;
    assert!(
        text_of(&confirm.event).contains("2 web session tokens revoked"),
        "{}",
        text_of(&confirm.event)
    );
    expect_env(&mut nick_rx, "Your password has been changed by God.").await;

    assert!(rig.bridge.validate_token(&token).await.is_none());
    assert!(rig.bridge.validate_token(&token2).await.is_none());
    assert!(matches!(
        rig.accounts.verify("nick", "hunter42").await,
        AuthOutcome::BadCredentials
    ));
    assert!(matches!(
        rig.accounts.verify("nick", "n3w-pass").await,
        AuthOutcome::Ok { .. }
    ));
    // And the web login path works with the new password.
    assert!(rig.bridge.login("nick", "n3w-pass", ip(1)).await.is_ok());
}
