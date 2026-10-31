//! Typed JSON -> authenticated WebBridge -> real WorldHandle/coordinator/writer.
//! The existing tactics Brace action isolates transport from commerce policy;
//! no certificate, commercial right, provider callback, or ACK is fabricated.
use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};
use std::time::Duration;

use mush_core::{
    ControlMsg, Engine, GOD, Inbound, ObjId, ObjKind, OfferKeyRing, OfferSecret, PerceivedRef,
    World,
};
use mush_gateway::{
    accounts::{AuthBackend, AuthOutcome, CharSummary, CreateErr},
    web_bridge::WebBridge,
};
use mush_ipc::{ActionSend, GatewayAction, GatewayLink, ServiceReq, ServiceResp, ToServer};
use mush_net_web::{NewWebConn, WebHooks};
use mush_proto::{
    ClientCommand, Envelope, InvokeRequest, InvokeResult, LegalActionResponse, OutputEvent,
};
use mush_server::world_task::{
    PersistHandles, ServerDeps, WorldHandle, base_registry, spawn_persist_with_gateway,
};
use mush_session::SessionManager;
use mush_store::{InMemoryStore, ObjectStore};
use serde_json::{Value, json};
use tokio::sync::mpsc;

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Rx = mpsc::Receiver<Arc<Envelope>>;
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

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

// Mode zero forwards to the real server. Other modes test only the documented
// GatewayLink admission vocabulary; they make no writer/Unknown proof claim.
struct Link {
    world: WorldHandle,
    mode: AtomicU8,
    submits: AtomicUsize,
    forwarded_invokes: AtomicUsize,
}
impl GatewayLink for Link {
    fn to_server(&self, input: ToServer) -> BoxFuture<'_, bool> {
        if matches!(
            &input,
            ToServer::Inbound(mush_ipc::WireInbound::Invoke { .. })
        ) {
            self.forwarded_invokes.fetch_add(1, Ordering::SeqCst);
        }
        self.world.to_server(input)
    }
    fn request(&self, input: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
        self.world.request(input)
    }
    fn submit_action(&self, action: GatewayAction) -> BoxFuture<'_, ActionSend> {
        self.submits.fetch_add(1, Ordering::SeqCst);
        match self.mode.load(Ordering::SeqCst) {
            1 => Box::pin(async { ActionSend::Accepted }),
            2 => Box::pin(async { ActionSend::Gone }),
            3 => Box::pin(async { ActionSend::Busy }),
            _ => self.world.submit_action(action),
        }
    }
}

struct Rig {
    bridge: WebBridge<Auth>,
    link: Arc<Link>,
    store: Arc<InMemoryStore>,
    handles: PersistHandles,
    own: Rx,
    alt: Rx,
    alt_actor: ObjId,
}
impl Rig {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut world = World::new_minimal();
        let alt = world.create(ObjKind::Player, "Other pilot", GOD, Some(ObjId(0)))?;
        let mut engine = Engine::with_modules(
            world,
            base_registry(),
            vec![Box::new(mush_tactics::TacticsModule::new())],
        )?;
        engine.set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x61; 32])?, [])?);
        let store = Arc::new(InMemoryStore::new());
        let sessions = Arc::new(SessionManager::default());
        let mut deps = ServerDeps::in_memory();
        deps.store = store.clone();
        let sink_sessions = sessions.clone();
        let handles = spawn_persist_with_gateway(engine, deps, move |weak| {
            Arc::new(mushd::inprocess::InProcessSink::new(sink_sessions, weak))
        });
        let link = Arc::new(Link {
            world: handles.handle.clone(),
            mode: AtomicU8::new(0),
            submits: AtomicUsize::new(0),
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
            store.clone(),
            vec![],
        );
        let ip = "127.0.0.1".parse()?;
        let login = bridge
            .login("shared", "fixture-password", ip)
            .await
            .map_err(|_| "actual token login")?;
        let (tx, mut own) = mpsc::channel(128);
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
        let (tx, mut other) = mpsc::channel(128);
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
            store,
            handles,
            own,
            alt: other,
            alt_actor: alt,
        })
    }
    async fn offer(&self, conn: u64, actor: ObjId) -> Result<String, Box<dyn std::error::Error>> {
        let offers = self
            .handles
            .handle
            .legal_actions(conn, actor, PerceivedRef::SelfActor)
            .await?;
        let offer = offers
            .iter()
            .find(|o| o.presentation().verb == "brace")
            .ok_or("brace offer")?;
        Ok(offer.offer().0.to_hex())
    }
    async fn shutdown(self) -> TestResult {
        assert_eq!(
            self.link.forwarded_invokes.load(Ordering::SeqCst),
            0,
            "legal actions must never enter a mutable module InvokeHandler"
        );
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
        Ok(())
    }
}

async fn invoke(bridge: &WebBridge<Auth>, conn: u64, id: u64, request: Value) -> TestResult {
    let command: ClientCommand = serde_json::from_value(
        json!({"t":"invoke","actor":999,"account":"outer-ignored","d":{"id":id,"actor":1,"account":"payload-ignored","request":{"service":"legal_action","request":request}}}),
    )?;
    let ClientCommand::Invoke {
        id,
        request: InvokeRequest::LegalAction(request),
    } = command
    else {
        return Err("typed action request".into());
    };
    tokio::time::timeout(
        Duration::from_secs(10),
        bridge.invoke(conn, id, InvokeRequest::LegalAction(request)),
    )
    .await?;
    Ok(())
}
async fn result(rx: &mut Rx, id: u64) -> Result<LegalActionResponse, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(event) = rx.recv().await {
            if let OutputEvent::InvokeResult {
                id: actual,
                result: InvokeResult::LegalAction(result),
            } = &event.event
            {
                if *actual != id {
                    return Err(Box::<dyn std::error::Error>::from("wrong correlation"));
                }
                return Ok(result.clone());
            }
        }
        Err("connection closed".into())
    })
    .await?
}
fn no_result(rx: &mut Rx) {
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(&event.event, OutputEvent::InvokeResult { .. }),
            "cross-connection invoke result"
        );
    }
}

#[tokio::test]
async fn actual_gateway_coordinator_commit_status_and_other_character_refusal() -> TestResult {
    let mut rig = Rig::new().await?;
    let offer = rig.offer(10, GOD).await?;
    // The same account's other attached character cannot spend this viewer's offer.
    invoke(
        &rig.bridge,
        20,
        40,
        json!({"op":"submit_self","offer":offer,"parameters":{}}),
    )
    .await?;
    assert_eq!(
        result(&mut rig.alt, 40).await?,
        LegalActionResponse::Rejected {}
    );
    assert!(
        rig.store
            .module_json_get("tactics", "action-state/v1/stance/1")
            .await?
            .is_none()
    );
    no_result(&mut rig.own);
    invoke(
        &rig.bridge,
        10,
        41,
        json!({"op":"submit_self","offer":offer,"parameters":{}}),
    )
    .await?;
    let reference = match result(&mut rig.own, 41).await? {
        LegalActionResponse::Committed { reference, .. }
        | LegalActionResponse::Owned { reference } => reference,
        other => return Err(format!("actual writer did not own the action: {other:?}").into()),
    };
    rig.handles.handle.barrier_result().await?;
    let stance = rig
        .store
        .module_json_get("tactics", "action-state/v1/stance/1")
        .await?
        .ok_or("durable stance")?;
    assert_eq!(stance["value"]["braced"], true);
    assert_eq!(stance["value"]["revision"], 1);
    invoke(
        &rig.bridge,
        10,
        42,
        json!({"op":"status","reference":reference.as_str()}),
    )
    .await?;
    assert!(
        matches!(result(&mut rig.own, 42).await?, LegalActionResponse::Committed { reference: actual, .. } if actual == reference)
    );
    no_result(&mut rig.alt);
    invoke(
        &rig.bridge,
        20,
        43,
        json!({"op":"status","reference":reference.as_str()}),
    )
    .await?;
    assert_eq!(
        result(&mut rig.alt, 43).await?,
        LegalActionResponse::Unknown {}
    );
    no_result(&mut rig.own);
    assert!(
        rig.store
            .module_json_get(
                "tactics",
                &format!("action-state/v1/stance/{}", rig.alt_actor.0)
            )
            .await?
            .is_none()
    );
    rig.shutdown().await
}

#[tokio::test]
async fn public_parameter_schema_and_offer_tag_still_reach_real_server_refusal() -> TestResult {
    let mut rig = Rig::new().await?;
    let offer = rig.offer(10, GOD).await?;
    invoke(&rig.bridge, 10, 50, json!({"op":"submit_self","offer":offer,"parameters":{"actor":{"kind":"integer","value":"1"}}})).await?;
    assert_eq!(
        result(&mut rig.own, 50).await?,
        LegalActionResponse::Rejected {}
    );
    let mut forged = offer.clone();
    forged.replace_range(63..64, if offer.ends_with('0') { "1" } else { "0" });
    invoke(
        &rig.bridge,
        10,
        51,
        json!({"op":"submit_self","offer":forged,"parameters":{}}),
    )
    .await?;
    assert_eq!(
        result(&mut rig.own, 51).await?,
        LegalActionResponse::Rejected {}
    );
    assert!(
        rig.store
            .module_json_get("tactics", "action-state/v1/stance/1")
            .await?
            .is_none()
    );
    no_result(&mut rig.alt);
    rig.shutdown().await
}

#[tokio::test]
async fn gateway_custody_outcomes_are_correlated_and_never_retried_implicitly() -> TestResult {
    let mut rig = Rig::new().await?;
    let offer = rig.offer(10, GOD).await?;
    invoke(
        &rig.bridge,
        10,
        mush_proto::MAX_VIEW_WIRE_INTEGER + 1,
        json!({"op":"submit_self","offer":offer,"parameters":{}}),
    )
    .await?;
    assert_eq!(rig.link.submits.load(Ordering::SeqCst), 0);
    no_result(&mut rig.own);
    for (mode, expected) in [
        (1, LegalActionResponse::Queued {}),
        (2, LegalActionResponse::Unavailable { reference: None }),
        (3, LegalActionResponse::Busy {}),
    ] {
        rig.link.mode.store(mode, Ordering::SeqCst);
        let before = rig.link.submits.load(Ordering::SeqCst);
        invoke(
            &rig.bridge,
            10,
            60 + u64::from(mode),
            json!({"op":"submit_self","offer":offer,"parameters":{}}),
        )
        .await?;
        assert_eq!(result(&mut rig.own, 60 + u64::from(mode)).await?, expected);
        assert_eq!(rig.link.submits.load(Ordering::SeqCst), before + 1);
        rig.handles.handle.barrier_result().await?;
        assert_eq!(rig.link.submits.load(Ordering::SeqCst), before + 1);
        no_result(&mut rig.alt);
    }
    assert!(
        rig.store
            .module_json_get("tactics", "action-state/v1/stance/1")
            .await?
            .is_none()
    );
    rig.shutdown().await
}
