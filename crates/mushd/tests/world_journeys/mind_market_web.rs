//! **The mind market's web twin** (17g cut C3c; the C3 plan §3.3 and §5): the browser's
//! public JSON -> the decode `mush-net-web`'s socket loop runs (`serde_json::from_str::<
//! ClientCommand>`, `ws.rs`) -> an authenticated `WebBridge` -> the real world task, on the
//! production module assembly.
//!
//! **The assembly is the composition root's own**: `mush_server::boot::build_engine` over a
//! default `Config` (an in-memory store, a scratch data dir), i.e. `boot::game_modules` on the
//! stock roster — the built-ins, worldgen, the NPC module, the pilot sheet, the TacSim layer
//! (composed whether or not `[tacsim].enabled`; its arena bridge registers the `mind` invoke
//! family and 17g C3a's two panel queries on every launch format) and content. The stock
//! roster composes no mind commerce, so the market answers closed: both panels say
//! `open: false` with the `NOT_OPEN` bytes, and a typed market action is refused `not_open`.
//! No entitlement, listing, rating or money is fabricated; this twin proves the transport —
//! the gateway's two `Query::MindMarket` arms, the forwarded typed invoke, and the layer that
//! refuses a malformed request — which C3a's mush-server tests could not reach.
//!
//! The rig is `market_original_minds_web.rs`'s (`WebBridge::with_link` over a `Link` that
//! records each forwarded query's name and arguments) joined to `legal_action_web.rs`'s (the
//! real `WorldHandle` behind it, and a count of the forwarded invokes).
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use mush_core::{ControlMsg, GOD, Inbound, ObjId, ObjKind};
use mush_gateway::{
    accounts::{AuthBackend, AuthOutcome, CharSummary, CreateErr},
    web_bridge::WebBridge,
};
use mush_ipc::{
    ActionSend, GatewayAction, GatewayLink, ServiceReq, ServiceResp, ToServer, WireInbound,
};
use mush_net_web::{NewWebConn, WebHooks};
use mush_proto::{ClientCommand, Envelope, InvokeResult, MindResponse, OutputEvent};
use mush_server::world_task::{
    PersistHandles, ServerDeps, WorldHandle, spawn_persist_with_gateway_production,
};
use mush_session::SessionManager;
use mush_store::{InMemoryStore, ObjectStore};
use mushd::config::Config;
use serde_json::{Value, json};
use tokio::sync::mpsc;

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Rx = mpsc::Receiver<Arc<Envelope>>;
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// 17g C2's `NOT_OPEN` (`tacsim::minds::market_commands::words`), the bytes the closed
/// market answers on every surface (C3a's SURFACE).
const NOT_OPEN: &str = "The mind market is not open here.";
/// The gateway's refusal of a request `InvokeRequest::validate` refuses, before forwarding
/// (`bridge_common.rs` `invoke`).
const GATEWAY_INVALID: &str = "Invalid, oversized or rate-limited request.";
/// The most a market keyframe may be (17g C3a `MAX_KEYFRAME_BYTES`).
const MAX_KEYFRAME_BYTES: usize = 32 * 1024;

struct Auth(Vec<CharSummary>);
impl AuthBackend for Auth {
    async fn verify(&self, account: &str, password: &str) -> AuthOutcome {
        if account == "shared" && password == "fixture-password" {
            AuthOutcome::Ok {
                account: account.into(),
                characters: self.0.clone(),
            }
        } else {
            AuthOutcome::BadCredentials
        }
    }
    async fn characters(&self, account: &str) -> Vec<CharSummary> {
        if account == "shared" {
            self.0.clone()
        } else {
            Vec::new()
        }
    }
    async fn create(&self, _: &str, _: &str, _: &str) -> Result<CharSummary, CreateErr> {
        Err(CreateErr::Io("unused transport fixture".into()))
    }
    async fn create_character(&self, _: &str, _: &str) -> Result<CharSummary, CreateErr> {
        Err(CreateErr::Io("unused transport fixture".into()))
    }
}

/// The gateway's link to the real world task, recording what crosses it: every forwarded
/// query's `(name, args)` and the number of forwarded invokes.
struct Link {
    world: WorldHandle,
    queries: Mutex<Vec<(String, Value)>>,
    forwarded_invokes: AtomicUsize,
}
impl GatewayLink for Link {
    fn to_server(&self, input: ToServer) -> BoxFuture<'_, bool> {
        match &input {
            ToServer::Inbound(WireInbound::Query { name, args, .. }) => {
                self.queries
                    .lock()
                    .expect("queries")
                    .push((name.to_string(), args.clone()));
            }
            ToServer::Inbound(WireInbound::Invoke { .. }) => {
                self.forwarded_invokes.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
        self.world.to_server(input)
    }
    fn request(&self, input: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
        self.world.request(input)
    }
    fn submit_action(&self, action: GatewayAction) -> BoxFuture<'_, ActionSend> {
        self.world.submit_action(action)
    }
}

impl Link {
    /// The forwarded market queries, in order (the attach pass forwards queries of its own).
    fn market_queries(&self) -> Vec<(String, Value)> {
        self.queries
            .lock()
            .expect("queries")
            .iter()
            .filter(|(name, _)| name == "mind.market" || name == "mind.holdings")
            .cloned()
            .collect()
    }
    fn invokes(&self) -> usize {
        self.forwarded_invokes.load(Ordering::SeqCst)
    }
}

struct Rig {
    bridge: WebBridge<Auth>,
    link: Arc<Link>,
    handles: PersistHandles,
    own: Rx,
    alt: Rx,
    dir: std::path::PathBuf,
}

impl Rig {
    /// Boot the composition root's engine, spawn the real world task behind the gateway, and
    /// attach two characters of one account over the web: God (connection 10) and a second
    /// pilot (connection 20), the bystander every reply must miss.
    async fn new(tag: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let dir = crate::support::scratch_dir(tag);
        let mut cfg = Config::default();
        cfg.store.store = "memory".into();
        cfg.server.data_dir = dir.clone();
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        let spool = mush_server::quarantine::QuarantineSpool::in_memory();
        let mut booted = mush_server::boot::build_engine(&cfg, &store, &spool).await?;
        let alt =
            booted
                .engine
                .world
                .create(ObjKind::Player, "Other pilot", GOD, Some(ObjId(0)))?;
        let sessions = Arc::new(SessionManager::default());
        // The production spawn (`spawn_persist_with_gateway_production`): the stock roster
        // composes the NPC module's decision source, which the plain spawn refuses.
        let deps = ServerDeps::production(
            &booted,
            store.clone(),
            Arc::new(mush_config::ipguard::IpGuard::new()),
            Arc::new(mush_config::metrics::Metrics::new()),
            spool,
            Vec::new(),
        );
        let llm = Some(Arc::clone(&booted.llm));
        let workers = booted.decision_workers;
        let sink_sessions = sessions.clone();
        let handles = spawn_persist_with_gateway_production(
            booted.engine,
            deps,
            llm,
            workers,
            move |weak| Arc::new(mushd::inprocess::InProcessSink::new(sink_sessions, weak)),
        )?;
        let link = Arc::new(Link {
            world: handles.handle.clone(),
            queries: Mutex::new(Vec::new()),
            forwarded_invokes: AtomicUsize::new(0),
        });
        let bridge = WebBridge::with_link(
            Arc::new(Auth(vec![
                CharSummary {
                    id: GOD,
                    name: "God".into(),
                },
                CharSummary {
                    id: alt,
                    name: "Other pilot".into(),
                },
            ])),
            sessions,
            link.clone(),
            Arc::new(mush_config::ipguard::IpGuard::new()),
            store,
            vec![],
        );
        let ip = "127.0.0.1".parse()?;
        let login = bridge
            .login("shared", "fixture-password", ip)
            .await
            .map_err(|_| "actual token login")?;
        let (tx, mut own) = mpsc::channel(256);
        bridge
            .connect_character(
                NewWebConn {
                    id: 10,
                    addr: ip,
                    tx,
                    closer: mush_net_web::ConnCloser::detached(),
                },
                &login.token,
                u64::from(GOD.0),
            )
            .await
            .map_err(|_| "actual owner attachment")?;
        let (tx, mut other) = mpsc::channel(256);
        bridge
            .connect_character(
                NewWebConn {
                    id: 20,
                    addr: ip,
                    tx,
                    closer: mush_net_web::ConnCloser::detached(),
                },
                &login.token,
                u64::from(alt.0),
            )
            .await
            .map_err(|_| "actual second attachment")?;
        handles.handle.barrier_result().await?;
        while own.try_recv().is_ok() {}
        while other.try_recv().is_ok() {}
        Ok(Self {
            bridge,
            link,
            handles,
            own,
            alt: other,
            dir,
        })
    }

    /// One browser frame, decoded the way `mush-net-web`'s socket loop decodes it and routed
    /// to the hook it routes to. `false` when the decode refuses the frame: nothing is routed.
    async fn send(&self, conn: u64, text: &str) -> Result<bool, Box<dyn std::error::Error>> {
        match serde_json::from_str::<ClientCommand>(text) {
            Ok(ClientCommand::Req { id, q }) => {
                tokio::time::timeout(Duration::from_secs(10), self.bridge.request(conn, id, q))
                    .await?;
                Ok(true)
            }
            Ok(ClientCommand::Invoke { id, request }) => {
                tokio::time::timeout(
                    Duration::from_secs(10),
                    self.bridge.invoke(conn, id, request),
                )
                .await?;
                Ok(true)
            }
            Ok(other) => Err(format!("not a request or an invoke: {other:?}").into()),
            Err(_) => Ok(false),
        }
    }

    /// **The bystander heard nothing of the market.** A correlated `who` on the alt connection
    /// (forwarded through the same world task, so its reply is queued behind anything the
    /// world sent the alt before it) flushes the alt's outbox; every envelope ahead of that
    /// reply is checked: no other reply or invoke result, no `mind.market` / `mind.holdings`
    /// panel in any shape (correlated or bare), and none of the market's or the gateway's
    /// refusal bytes anywhere in it.
    async fn bystander_heard_nothing(&mut self, id: u64) -> TestResult {
        let frame = format!(r#"{{"t":"req","d":{{"id":{id},"q":{{"t":"who"}}}}}}"#);
        assert!(self.send(20, &frame).await?, "the flush request decodes");
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(envelope) = self.alt.recv().await {
                if let OutputEvent::Reply { id: got, .. } = &envelope.event
                    && *got == id
                {
                    return Ok(());
                }
                market_free(&envelope);
            }
            Err(Box::<dyn std::error::Error>::from("connection closed"))
        })
        .await?
    }

    async fn shutdown(self) -> TestResult {
        if !self
            .handles
            .handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await
        {
            return Err("shutdown send".into());
        }
        tokio::time::timeout(Duration::from_secs(10), self.handles.world_join).await??;
        tokio::time::timeout(Duration::from_secs(10), self.handles.writer_join).await??;
        tokio::time::timeout(Duration::from_secs(10), self.handles.outbox_join).await??;
        let _ = std::fs::remove_dir_all(&self.dir);
        Ok(())
    }
}

/// The reply correlated to `id` on this connection, unwrapped.
async fn reply(rx: &mut Rx, id: u64) -> Result<OutputEvent, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(envelope) = rx.recv().await {
            if let OutputEvent::Reply { id: got, event } = &envelope.event {
                if *got != id {
                    return Err(Box::<dyn std::error::Error>::from("wrong correlation"));
                }
                return Ok((**event).clone());
            }
        }
        Err("connection closed".into())
    })
    .await?
}

/// The invoke result correlated to `id` on this connection.
async fn result(rx: &mut Rx, id: u64) -> Result<InvokeResult, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(envelope) = rx.recv().await {
            if let OutputEvent::InvokeResult { id: got, result } = &envelope.event {
                if *got != id {
                    return Err(Box::<dyn std::error::Error>::from("wrong correlation"));
                }
                return Ok(result.clone());
            }
        }
        Err("connection closed".into())
    })
    .await?
}

/// One envelope the bystander received carries nothing of the market: it is no reply or
/// invoke result (the flush's own reply is taken before this is called), no market panel,
/// and holds neither `NOT_OPEN` nor the gateway's invalid-request bytes.
fn market_free(envelope: &Envelope) {
    assert!(
        !matches!(
            &envelope.event,
            OutputEvent::Reply { .. } | OutputEvent::InvokeResult { .. }
        ),
        "a reply reached another connection: {:?}",
        envelope.event
    );
    if let OutputEvent::Panel { feed, .. } = &envelope.event {
        assert!(
            feed.as_str() != "mind.market" && feed.as_str() != "mind.holdings",
            "a market panel reached another connection: {:?}",
            envelope.event
        );
    }
    let text = serde_json::to_string(envelope).expect("an envelope serializes");
    assert!(
        !text.contains(NOT_OPEN) && !text.contains(GATEWAY_INVALID),
        "market bytes reached another connection: {text}"
    );
}

/// A market panel keyframe's data, with its feed and the closed keyframe's keys checked.
fn keyframe(event: OutputEvent, feed: &str, keys: &[&str]) -> Value {
    let OutputEvent::Panel {
        feed: got, data, ..
    } = event
    else {
        panic!("a market panel, not {event:?}");
    };
    assert_eq!(got.as_str(), feed);
    assert!(data.to_string().len() <= MAX_KEYFRAME_BYTES);
    assert_eq!(data["schema"], json!({ "id": feed, "version": 1 }));
    let object = data.as_object().expect("a keyframe object");
    let mut present: Vec<&str> = object.keys().map(String::as_str).collect();
    present.sort_unstable();
    let mut expected = keys.to_vec();
    expected.sort_unstable();
    assert_eq!(present, expected, "every key present, and no other");
    data
}

#[tokio::test]
async fn mind_market_listings_and_holdings_cross_the_web_bridge_as_closed_keyframes() -> TestResult
{
    let mut rig = Rig::new("mind-market-web-panels").await?;

    assert!(
        rig.send(
            10,
            r#"{"t":"req","d":{"id":41,"q":{"t":"mind_market","d":{"kind":"listings","after":null}}}}"#,
        )
        .await?
    );
    let market = keyframe(
        reply(&mut rig.own, 41).await?,
        "mind.market",
        &["schema", "open", "notice", "rows", "after", "own"],
    );
    assert_eq!(market["open"], false);
    assert_eq!(market["notice"], NOT_OPEN);
    assert_eq!(market["rows"], json!([]));
    assert_eq!(market["after"], Value::Null);
    assert_eq!(market["own"], json!({ "quotes": [], "waiting": [] }));

    assert!(
        rig.send(
            10,
            r#"{"t":"req","d":{"id":42,"q":{"t":"mind_market","d":{"kind":"holdings","section":"owner","after":null}}}}"#,
        )
        .await?
    );
    let holdings = keyframe(
        reply(&mut rig.own, 42).await?,
        "mind.holdings",
        &["schema", "open", "notice", "section", "rows", "after"],
    );
    assert_eq!(holdings["open"], false);
    assert_eq!(holdings["notice"], NOT_OPEN);
    assert_eq!(holdings["section"], "owner");
    assert_eq!(holdings["rows"], json!([]));
    assert_eq!(holdings["after"], Value::Null);

    // A holdings cursor is a number on the wire, and the paid section is asked by name.
    assert!(
        rig.send(
            10,
            r#"{"t":"req","d":{"id":43,"q":{"t":"mind_market","d":{"kind":"holdings","section":"paid","after":42}}}}"#,
        )
        .await?
    );
    let paid = keyframe(
        reply(&mut rig.own, 43).await?,
        "mind.holdings",
        &["schema", "open", "notice", "section", "rows", "after"],
    );
    assert_eq!(paid["section"], "paid");
    assert_eq!(paid["notice"], NOT_OPEN);

    // One forwarded query per request, named by the feed, carrying the closed request's own
    // fields and nothing else — no actor, no account.
    assert_eq!(
        rig.link.market_queries(),
        vec![
            ("mind.market".to_owned(), json!({ "after": null })),
            (
                "mind.holdings".to_owned(),
                json!({ "section": "owner", "after": null })
            ),
            (
                "mind.holdings".to_owned(),
                json!({ "section": "paid", "after": 42 })
            ),
        ]
    );
    assert_eq!(rig.link.invokes(), 0, "a page read is never an invoke");
    // The same account's other attached character received none of it.
    rig.bystander_heard_nothing(49).await?;
    rig.shutdown().await
}

#[tokio::test]
async fn mind_market_typed_invoke_is_forwarded_and_refused_not_open() -> TestResult {
    let mut rig = Rig::new("mind-market-web-invoke").await?;
    assert!(
        rig.send(
            10,
            r#"{"t":"invoke","d":{"id":51,"request":{"service":"mind","request":{"op":"market","action":{"act":"pay","code":"aaaaaaaa"}}}}}"#,
        )
        .await?
    );
    let answer = result(&mut rig.own, 51).await?;
    assert_eq!(
        answer,
        InvokeResult::Mind(MindResponse::Error {
            code: "not_open".into(),
            message: NOT_OPEN.into(),
        })
    );
    // Forwarded once to the server (the gateway validates and does not answer it locally),
    // and asked no page on the way.
    assert_eq!(rig.link.invokes(), 1);
    assert!(rig.link.market_queries().is_empty());
    rig.bystander_heard_nothing(59).await?;
    rig.shutdown().await
}

#[tokio::test]
async fn mind_market_malformed_requests_are_refused_before_forwarding() -> TestResult {
    let mut rig = Rig::new("mind-market-web-malformed").await?;

    // The closed page requests: an unknown key, a code that is not 8 characters of digits
    // and a to f, a holdings cursor that is not a number. Each fails the socket's decode and
    // nothing is forwarded.
    for (id, frame) in [
        (60, r#"{"kind":"listings","extra":1}"#),
        (61, r#"{"kind":"listings","after":"short"}"#),
        (62, r#"{"kind":"listings","after":"aaaaaaa"}"#),
        (63, r#"{"kind":"listings","after":"gggggggg"}"#),
        (64, r#"{"kind":"holdings","section":"owner","after":"42"}"#),
        (65, r#"{"kind":"holdings","section":"both","after":null}"#),
    ] {
        let text =
            format!(r#"{{"t":"req","d":{{"id":{id},"q":{{"t":"mind_market","d":{frame}}}}}}}"#);
        assert!(!rig.send(10, &text).await?, "decode refuses {frame}");
    }
    assert!(rig.link.market_queries().is_empty());

    // The typed action's closed code: 7 characters, and 8 that are not digits or a to f.
    for (id, code) in [(70, "aaaaaaa"), (71, "gggggggg"), (72, "AAAAAAAA")] {
        let text = format!(
            r#"{{"t":"invoke","d":{{"id":{id},"request":{{"service":"mind","request":{{"op":"market","action":{{"act":"pay","code":"{code}"}}}}}}}}}}"#
        );
        assert!(!rig.send(10, &text).await?, "decode refuses code {code}");
    }
    assert_eq!(rig.link.invokes(), 0);

    // A royalty past 300 decodes (it is a u16) and is refused by `InvokeRequest::validate`
    // AT THE GATEWAY (`bridge_common.rs` `invoke`), before forwarding: the gateway's own
    // invalid-request reply, never a C2 word.
    assert!(
        rig.send(
            10,
            r#"{"t":"invoke","d":{"id":80,"request":{"service":"mind","request":{"op":"market","action":{"act":"publish","filed":7,"form":"open","royalty":301,"transfer":null}}}}}"#,
        )
        .await?
    );
    let refused = result(&mut rig.own, 80).await?;
    assert_eq!(
        refused,
        InvokeResult::Mind(MindResponse::Error {
            code: "invalid".into(),
            message: GATEWAY_INVALID.into(),
        })
    );
    let InvokeResult::Mind(MindResponse::Error { message, .. }) = &refused else {
        return Err("a mind error".into());
    };
    assert_ne!(message, NOT_OPEN);
    assert!(!message.to_lowercase().contains("royalty"));
    assert_eq!(rig.link.invokes(), 0, "a refused invoke is never forwarded");

    // The same request at the bound is forwarded, and the closed market refuses it.
    assert!(
        rig.send(
            10,
            r#"{"t":"invoke","d":{"id":81,"request":{"service":"mind","request":{"op":"market","action":{"act":"publish","filed":7,"form":"open","royalty":300,"transfer":null}}}}}"#,
        )
        .await?
    );
    assert_eq!(
        result(&mut rig.own, 81).await?,
        InvokeResult::Mind(MindResponse::Error {
            code: "not_open".into(),
            message: NOT_OPEN.into(),
        })
    );
    assert_eq!(rig.link.invokes(), 1);
    assert!(rig.link.market_queries().is_empty());
    rig.bystander_heard_nothing(89).await?;
    rig.shutdown().await
}
