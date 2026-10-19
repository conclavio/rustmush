//! Hooks-level public JSON -> authenticated WebBridge -> actual Market handler.
//! Empty registered families isolate this read route. Nonempty privacy/paging lives
//! in Market lifecycle tests; this test creates no certificate, offer, or paid authority.
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{Audience, Inbound, ObjKind};
use mush_core::{DefinitionRef, Engine, GameModule, ModError, ObjId, Outbox, Registrar};
use mush_economy::{AccountId, Amount, InstrumentId};
use mush_gateway::accounts::{AuthBackend, AuthOutcome, CharSummary, CreateErr};
use mush_gateway::web_bridge::WebBridge;
use mush_ipc::{GatewayLink, ServiceReq, ServiceResp, ToServer, WireInbound};
use mush_market::mind_trade::records::QuoteVenue;
use mush_market::mind_trade::{
    OriginalDuration, OriginalPrice, OriginalQuoteConfig, OriginalQuotePolicy, QUERY,
};
use mush_net_web::{NewWebConn, WebHooks};
use mush_proto::{ClientCommand, Envelope, OutputEvent};
use mush_roll::commerce::filing::InitialFilingTerms;
use mush_roll::commerce::module::{CommerceConfig, CommerceMode};
use mush_roll::commerce::rating::RatingCompatibility;
use mush_session::{ConnId, SessionManager};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::mpsc;

fn descriptor() -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("registry_manifest").expect("kind"),
        DefinitionId::new("combat").expect("id"),
        1,
        json!({"fixture":"public_read_registration_only"}),
    )
    .expect("descriptor")
    .as_current()
}
fn manifest() -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new("tacsim").expect("owner"), descriptor())
        .expect("fixture manifest")
}
struct Manifest;
impl GameModule for Manifest {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let expected = descriptor();
        reg.definition_validator(
            DefinitionKind::new("registry_manifest").expect("kind"),
            move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
                let definition = ctx.definition();
                if definition.reference() == &manifest()
                    && definition.payload() == expected.payload()
                    && definition.references().is_empty()
                {
                    Ok(())
                } else {
                    Err("fixture manifest differs".into())
                }
            },
        )?;
        reg.definition(descriptor())?;
        Ok(())
    }
}
#[derive(Debug)]
struct Venue;
impl mush_market::VenueGate for Venue {
    fn venue_of_room(&self, _: &mush_core::World, _: ObjId) -> Option<mush_market::MarketId> {
        mush_market::MarketId::new("registry")
    }
    fn endpoint_for(
        &self,
        _: &mush_core::World,
        _: ObjId,
        _: &mush_market::MarketId,
        _: mush_market::Presence,
    ) -> Option<mush_inventory::EndpointRef> {
        None
    }
    fn admit(
        &self,
        _: &mush_core::World,
        _: ObjId,
        _: &mush_market::MarketId,
        _: Option<mush_inventory::EndpointRef>,
        _: mush_market::Presence,
    ) -> Result<(), mush_market::VenueRefusal> {
        Ok(())
    }
    fn governed(&self, _: &mush_core::World, _: mush_inventory::EndpointRef) -> bool {
        false
    }
    fn port_seller(
        &self,
        _: &mush_core::World,
        _: &mush_market::MarketId,
        _: &mush_org::PrincipalId,
    ) -> bool {
        false
    }
}
fn market_engine(enabled: bool) -> Engine {
    if !enabled {
        return Engine::with_modules(
            mush_core::World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(mush_market::MarketModule::new())],
        )
        .expect("default Market");
    }
    let issuer = mush_org::SystemPrincipalId::new("roll.registry").expect("issuer");
    let roll = CommerceConfig::new(
        CommerceMode::Active,
        InitialFilingTerms::new(90 * 86400, 3 * 86400).expect("terms"),
        manifest(),
        RatingCompatibility::combat_v1(manifest().digest()),
    )
    .expect("composition")
    .with_original_market_quotes();
    let policy = mush_economy::registry_filing::InitialFilingPolicy::new(
        issuer.clone(),
        AccountId(11),
        Amount::from_minor(100),
        mush_economy::CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )
    .expect("filing configuration");
    let economy = mush_economy::EconomyModule::new()
        .with_title_issuers(vec![issuer.clone()])
        .with_initial_filing(policy, roll.initial_filing_action().expect("action"));
    let quotes = OriginalQuoteConfig::new(
        OriginalQuotePolicy::new(
            OriginalPrice::new(
                Amount::from_minor(400),
                InstrumentId::new("cr").expect("instrument"),
                issuer,
                AccountId(11),
                50,
            )
            .expect("price"),
            QuoteVenue::new(
                mush_market::MarketId::new("registry").expect("market"),
                mush_org::PrincipalId::System(
                    mush_org::SystemPrincipalId::new("registry.venue").expect("venue"),
                ),
                AccountId(12),
                20,
            )
            .expect("venue"),
            OriginalDuration::new(true, 90 * 86400, 3 * 86400, 60).expect("duration"),
        )
        .expect("policy"),
        manifest(),
    )
    .expect("quote configuration");
    Engine::with_modules(
        mush_core::World::new_minimal(),
        mush_core::Registry::new(),
        vec![
            Box::new(Manifest),
            Box::new(economy),
            Box::new(mush_roll::RollModule::with_writers(["tacsim"]).with_commerce(roll)),
            Box::new(
                mush_market::MarketModule::new()
                    .with_original_mind_quotes(quotes)
                    .with_venue_gate(Arc::new(Venue)),
            ),
        ],
    )
    .expect("actual registered Market handler and dependency graph")
}

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
            vec![]
        }
    }
    async fn create(&self, _: &str, _: &str, _: &str) -> Result<CharSummary, CreateErr> {
        Err(CreateErr::Io("unused read fixture".into()))
    }
    async fn create_character(&self, _: &str, _: &str) -> Result<CharSummary, CreateErr> {
        Err(CreateErr::Io("unused read fixture".into()))
    }
}

struct Link {
    engine: Mutex<Engine>,
    sessions: Arc<SessionManager>,
    queries: Mutex<Vec<(u64, ObjId, u64, Value)>>,
}
impl GatewayLink for Link {
    fn to_server(&self, msg: ToServer) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        if let ToServer::Inbound(wire) = msg {
            if let WireInbound::Query {
                conn,
                actor,
                id,
                name,
                args,
            } = &wire
            {
                if name == QUERY {
                    self.queries
                        .lock()
                        .expect("queries")
                        .push((*conn, *actor, *id, args.clone()));
                }
            }
            let mut engine = self.engine.lock().expect("engine");
            let mut out = Outbox::new();
            engine.step(
                Inbound::from(wire),
                &mut out,
                &mut mush_core::engine::Effects::new(),
            );
            // This is a read-only fixture, not a writer/ACK simulation.
            assert!(engine.take_module_ops().is_empty());
            for (audience, event) in out.events() {
                if let Audience::Conn(conn) = audience {
                    let _ = self
                        .sessions
                        .send_to_conn(ConnId(*conn), Arc::new(event.clone()));
                } else {
                    assert!(
                        !matches!(event, OutputEvent::Reply { .. }),
                        "queries must be connection-only"
                    );
                }
            }
        }
        Box::pin(async { true })
    }
    fn request(
        &self,
        request: ServiceReq,
    ) -> Pin<Box<dyn Future<Output = Option<ServiceResp>> + Send + '_>> {
        let response = match request {
            // Each input above has already run synchronously and was checked to emit no writes.
            ServiceReq::Barrier => Some(ServiceResp::Barrier),
            ServiceReq::ConnectionReady { conn, actor } => Some(ServiceResp::ConnectionReady(
                self.engine.lock().expect("engine").actor_of(conn) == Some(actor),
            )),
            // Leave optional attach-room rendering unavailable; it is outside this query test.
            _ => None,
        };
        Box::pin(async move { response })
    }
}

type Rx = mpsc::Receiver<Arc<Envelope>>;
fn empty(rx: &mut Rx) {
    while rx.try_recv().is_ok() {}
}
fn reply(rx: &mut Rx, id: u64) -> OutputEvent {
    let envelopes = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    let mut replies = envelopes.iter().filter_map(|row| match &row.event {
        OutputEvent::Reply { id: got, event } if *got == id => Some((**event).clone()),
        _ => None,
    });
    let result = replies.next().expect("actual connection-correlated reply");
    assert!(replies.next().is_none());
    result
}
async fn send(bridge: &WebBridge<Auth>, conn: u64, id: u64, kind: &str, after: Value) {
    let command: ClientCommand = serde_json::from_value(json!({
        "t":"req","d":{"id":id,"q":{"t":"market_original_minds","d":{"kind":kind,"after":after}}}
    }))
    .expect("actual public wire request");
    let ClientCommand::Req { id, q } = command else {
        panic!("request variant")
    };
    bridge.request(conn, id, q).await;
}
fn data(event: OutputEvent) -> Value {
    let OutputEvent::Panel { feed, data, .. } = event else {
        panic!("actual Market panel")
    };
    assert_eq!(feed, QUERY);
    assert!(data.to_string().len() <= 32 * 1024);
    data
}

#[tokio::test]
async fn public_query_reaches_real_market_with_character_routing_and_correlation() {
    let mut engine = market_engine(true);
    let alt = engine
        .world
        .create(
            ObjKind::Player,
            "Other pilot",
            mush_core::GOD,
            Some(ObjId(0)),
        )
        .expect("same-account other character");
    let sessions = Arc::new(SessionManager::default());
    let link = Arc::new(Link {
        engine: Mutex::new(engine),
        sessions: sessions.clone(),
        queries: Mutex::new(vec![]),
    });
    let bridge = WebBridge::with_link(
        Arc::new(Auth(vec![
            CharSummary {
                id: mush_core::GOD,
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
        Arc::new(mush_store::InMemoryStore::new()),
        vec![],
    );
    let ip = "127.0.0.1".parse().expect("loopback");
    let login = bridge
        .login("shared", "fixture-password", ip)
        .await
        .expect("actual token mint");
    let (tx, mut own_rx) = mpsc::channel(64);
    bridge
        .connect_character(
            NewWebConn {
                id: 10,
                addr: ip,
                tx,
            },
            &login.token,
            u64::from(mush_core::GOD.0),
        )
        .await
        .expect("authenticated owner connection");
    let (tx, mut alt_rx) = mpsc::channel(64);
    bridge
        .connect_character(
            NewWebConn {
                id: 20,
                addr: ip,
                tx,
            },
            &login.token,
            u64::from(alt.0),
        )
        .await
        .expect("authenticated same-account alt connection");
    empty(&mut own_rx);
    empty(&mut alt_rx);
    let before = link
        .engine
        .lock()
        .expect("engine")
        .action_state()
        .revision();

    send(&bridge, 10, 41, "own", Value::Null).await;
    let first = data(reply(&mut own_rx, 41));
    assert_eq!(
        first,
        json!({"kind":"own","listings":[],"quotes":[],"after":null,"funded":false})
    );
    assert!(
        alt_rx.try_recv().is_err(),
        "reply must not reach the same-account alt"
    );
    send(
        &bridge,
        10,
        42,
        "own",
        json!("00000000000000000000000000000010"),
    )
    .await;
    assert_eq!(data(reply(&mut own_rx, 42)), first);
    assert!(alt_rx.try_recv().is_err());
    send(&bridge, 20, 43, "own", Value::Null).await;
    assert_eq!(data(reply(&mut alt_rx, 43)), first);
    assert!(own_rx.try_recv().is_err());
    send(&bridge, 10, 44, "listings", Value::Null).await;
    let public = data(reply(&mut own_rx, 44));
    assert_eq!(
        public,
        json!({"kind":"listings","rows":[],"after":null,"funded":false})
    );
    assert!(alt_rx.try_recv().is_err());
    let expected = vec![
        (10, mush_core::GOD, 41, json!({"kind":"own"})),
        (
            10,
            mush_core::GOD,
            42,
            json!({"kind":"own", "after":"00000000000000000000000000000010"}),
        ),
        (20, alt, 43, json!({"kind":"own"})),
        (10, mush_core::GOD, 44, json!({"kind":"listings"})),
    ];
    assert_eq!(*link.queries.lock().expect("queries"), expected);
    send(&bridge, 999, 45, "own", Value::Null).await;
    assert_eq!(
        *link.queries.lock().expect("queries"),
        expected,
        "unattached requests never reach owner"
    );
    assert_eq!(
        link.engine
            .lock()
            .expect("engine")
            .action_state()
            .revision(),
        before
    );
    // No opt-in still means no Market handler, even though the transport understands the query.
    let mut inactive = market_engine(false);
    let mut output = Outbox::new();
    inactive.step(
        Inbound::Query {
            conn: 10,
            actor: mush_core::GOD,
            id: 46,
            name: QUERY.into(),
            args: json!({"kind":"own"}),
        },
        &mut output,
        &mut mush_core::engine::Effects::new(),
    );
    assert!(output.events().iter().any(|(_, event)| matches!(event,
        OutputEvent::Reply { id: 46, event } if matches!(event.as_ref(), OutputEvent::Sys(_)))));
    assert!(!output.events().iter().any(|(_, event)| matches!(event,
        OutputEvent::Reply { event, .. } if matches!(event.as_ref(), OutputEvent::Panel { .. }))));
}
