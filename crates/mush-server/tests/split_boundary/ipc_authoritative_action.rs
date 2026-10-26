//! End-to-end K2/K3 ownership transfer over the real split-process protocol.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures::future::BoxFuture;
use mush_config::config::Config;
use mush_core::{
    ActionOfferId, AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeSemanticEvent,
    CanonicalHash, CanonicalId, CommitToken, DefinitionDescriptor, DefinitionId, DefinitionKind,
    DefinitionRegistryBuilder, GOD, IdempotencyKey, IdempotencyScope, LIMBO, Obj, ObjId, ObjKind,
    OwnerId, PerceivedRef, PreparedCausalRef, PrincipalId, ReplayWrite, RequestUid, RulesValue,
    ScopedIdempotencyKey,
};
use mush_ipc::{
    FrameReader, GatewayAction, GatewayInput, GatewayRequest, GatewayRequestDisposition,
    ServiceReq, ServiceResp, ToGateway, ToServer, WireInbound, write_frame,
};
use mush_proto::ProjectedActionStatus;
use mush_server::action_coordinator::ActionCoordinator;
use mush_store::BoundIdempotencyStatus;
use mush_store::{
    AccountRec, AuthoritativeCheckpointReceipt, AuthoritativeDiscardReceipt,
    AuthoritativeEpochRetirement, AuthoritativeIntentPage, AuthoritativeOutboxAttemptResult,
    AuthoritativeOutboxRecord, AuthoritativeQuarantineReceipt, AuthoritativeReceipt,
    AuthoritativeRetentionUsage, AuthoritativeScanCursor, AuthoritativeStore, BanRec, BootSkip,
    BundleStatus, DiscordBanRec, DiscordCodeRecord, DiscordLinkRec, IdempotencyStatus,
    InMemoryStore, IntentReceipt, ModuleDocs, ObjectStore, PendingAuthoritativeIntent, SshKeyRec,
    StoreError, TokenRecord, WorldSnapshot, WriteBatch, WriteReceipt,
};

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rustmush-ipc-authoritative-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn test_config(tag: &str) -> Config {
    let mut config = Config::default();
    config.store.store = "memory".to_owned();
    config.server.data_dir = scratch_dir(tag);
    config.ssh.listen = "127.0.0.1:0".to_owned();
    config.ipc.listen = "127.0.0.1:0".to_owned();
    config
}

async fn boot_server(tag: &'static str) -> (mush_ipc::IpcEndpoint, Arc<InMemoryStore>) {
    let (bound, bound_rx) = tokio::sync::oneshot::channel();
    let store = Arc::new(InMemoryStore::new());
    let server_store: Arc<dyn ObjectStore> = store.clone();
    let config = test_config(tag);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(mush_server::serve_with_store(
                config,
                Some(bound),
                Some(server_store),
            ))
            .unwrap();
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(30), bound_rx)
        .await
        .expect("server bind timed out")
        .expect("server returned bound endpoint");
    (endpoint, store)
}

async fn hello_and_attach(
    endpoint: &mush_ipc::IpcEndpoint,
) -> (FrameReader<mush_ipc::BoxRead>, mush_ipc::BoxWrite) {
    let (reader, mut writer) = mush_ipc::connect(endpoint).await.unwrap();
    let mut frames = FrameReader::new(reader);
    assert!(matches!(
        frames.read::<ToGateway>().await.unwrap().unwrap(),
        ToGateway::Hello { .. }
    ));
    write_frame(
        &mut writer,
        &ToServer::Inbound(WireInbound::ConnUp {
            conn: 41,
            actor: GOD,
            caps_structured: true,
            from_linkdead: false,
        }),
    )
    .await
    .unwrap();
    (frames, writer)
}

fn action_request(
    sequence: u64,
    actor: mush_core::ObjId,
    offered_action: ActionOfferId,
) -> GatewayRequest {
    GatewayRequest::new(
        77,
        sequence,
        GatewayInput::Action(Box::new(GatewayAction {
            conn: 41,
            actor,
            offered_action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: serde_json::json!({}),
        })),
    )
}

async fn brace_offer(
    frames: &mut FrameReader<mush_ipc::BoxRead>,
    writer: &mut mush_ipc::BoxWrite,
) -> ActionOfferId {
    write_frame(
        writer,
        &ToServer::Request {
            id: 900,
            req: ServiceReq::LegalActions {
                conn: 41,
                actor: GOD,
                subject: PerceivedRef::SelfActor,
            },
        },
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let ToGateway::Reply {
                id: 900,
                resp: ServiceResp::LegalActions(Ok(offers)),
            } = frames.read::<ToGateway>().await.unwrap().unwrap()
            {
                let offer = offers
                    .iter()
                    .find(|offer| offer.presentation().verb == "brace")
                    .expect("tactics brace is publicly projected");
                break offer.offer();
            }
        }
    })
    .await
    .expect("legal-action projection timed out")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authenticated_action_transfers_ownership_only_after_exact_intent() {
    let (endpoint, store) = boot_server("owned-finished").await;
    let (mut frames, mut writer) = hello_and_attach(&endpoint).await;
    let offer = brace_offer(&mut frames, &mut writer).await;
    let request = action_request(1, GOD, offer);
    write_frame(&mut writer, &ToServer::Input(request.clone()))
        .await
        .unwrap();

    let (token, status, projected_receipt) = tokio::time::timeout(Duration::from_secs(15), async {
        let mut owned = None;
        loop {
            match frames.read::<ToGateway>().await.unwrap().unwrap() {
                ToGateway::RequestDisposition {
                    request: seen,
                    disposition: GatewayRequestDisposition::Busy,
                } if seen == request.uid => {
                    assert!(owned.is_none(), "Busy arrived after Owned");
                    // Busy proves no intent was reserved. Retry the exact
                    // request within the enclosing deadline; once Owned has
                    // arrived, only the original completion may satisfy us.
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    write_frame(&mut writer, &ToServer::Input(request.clone()))
                        .await
                        .unwrap();
                }
                ToGateway::RequestDisposition {
                    request: seen,
                    disposition: GatewayRequestDisposition::Owned { token, status },
                } if seen == request.uid => owned = Some((token, status)),
                ToGateway::RequestDisposition {
                    request: seen,
                    disposition: GatewayRequestDisposition::Finished { recipient, receipt },
                } if seen == request.uid => {
                    let (owned_token, status) = owned.expect("Finished arrived before Owned");
                    assert_eq!(recipient, GOD);
                    assert_eq!(receipt.status(), &status);
                    assert_eq!(receipt.commit(), owned_token.to_string());
                    break (owned_token, status, receipt);
                }
                ToGateway::RequestDisposition {
                    request: seen,
                    disposition,
                } if seen == request.uid => {
                    panic!("unexpected authoritative disposition: {disposition:?}");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("authoritative dispositions timed out");
    let projected = projected_receipt
        .result()
        .expect("production tactics owns the brace receipt projection");
    assert_eq!(projected.summary(), "Brace committed.");
    assert_eq!(projected.data()["braced"], true);
    assert_eq!(projected.data()["revision"], 1);
    let serialized = serde_json::to_value(&projected_receipt).unwrap();
    mush_contract_tests::assert_projection_json_clean(
        "split IPC action receipt",
        &serialized,
        &[
            "action",
            "actor",
            "bundle_hash",
            "definition",
            "eligible_witnesses",
            "guards",
            "ingress_hash",
            "outcome",
            "outcome_hash",
            "plan_hash",
            "provider",
            "request_hash",
            "resolution",
        ],
        &[],
    );

    let key = ActionCoordinator::status_key(&PrincipalId::Object { id: GOD }, &status)
        .expect("public status reconstructs under its authenticated principal");
    let BoundIdempotencyStatus::FinalizedExact(internal_receipt) = store
        .bound_authoritative_idempotency_status(&key)
        .await
        .unwrap()
    else {
        panic!("owned action did not finalize under its public status identity");
    };
    let bundle_hash = internal_receipt.bundle_hash();
    assert_eq!(internal_receipt.token(), token);
    assert!(matches!(
        store.authoritative_status(token, bundle_hash).await.unwrap(),
        BundleStatus::FinalizedExact(receipt)
            if receipt.token() == token && receipt.bundle_hash() == bundle_hash
    ));

    // The authenticated status service reproduces the same redacted result;
    // changing only the actor field cannot disclose it.
    write_frame(
        &mut writer,
        &ToServer::Request {
            id: 901,
            req: ServiceReq::ActionStatus {
                conn: 41,
                actor: GOD,
                status: status.clone(),
            },
        },
    )
    .await
    .unwrap();
    write_frame(
        &mut writer,
        &ToServer::Request {
            id: 902,
            req: ServiceReq::ActionStatus {
                conn: 41,
                actor: LIMBO,
                status: status.clone(),
            },
        },
    )
    .await
    .unwrap();
    let mut authorized = None;
    let mut spoofed = None;
    tokio::time::timeout(Duration::from_secs(10), async {
        while authorized.is_none() || spoofed.is_none() {
            match frames.read::<ToGateway>().await.unwrap().unwrap() {
                ToGateway::Reply {
                    id: 901,
                    resp: ServiceResp::ActionStatus(result),
                } => authorized = Some(result),
                ToGateway::Reply {
                    id: 902,
                    resp: ServiceResp::ActionStatus(result),
                } => spoofed = Some(result),
                _ => {}
            }
        }
    })
    .await
    .expect("action status replies timed out");
    assert_eq!(
        authorized,
        Some(ProjectedActionStatus::Committed(projected_receipt.clone()))
    );
    assert_eq!(spoofed, Some(ProjectedActionStatus::Unknown));

    // Model loss of the first ownership/terminal frames: replay the byte-exact
    // gateway request after the original connection has disappeared. The
    // durable binding is the proof this exact ingress was authenticated before
    // ownership; it answers from the compact receipt without requiring the old
    // transport to remain live. Provider planning does not run again and
    // stance revision stays 1.
    write_frame(
        &mut writer,
        &ToServer::Inbound(WireInbound::ConnDown {
            conn: 41,
            actor: GOD,
            reason: mush_core::DisconnectKind::Lost,
        }),
    )
    .await
    .unwrap();
    write_frame(&mut writer, &ToServer::Input(request.clone()))
        .await
        .unwrap();
    let replay = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let ToGateway::RequestDisposition {
                request: seen,
                disposition,
            } = frames.read::<ToGateway>().await.unwrap().unwrap()
                && seen == request.uid
            {
                break disposition;
            }
        }
    })
    .await
    .expect("exact replay disposition timed out");
    assert_eq!(
        replay,
        GatewayRequestDisposition::Finished {
            recipient: GOD,
            receipt: projected_receipt,
        },
        "an exact finalized replay must return the existing result"
    );
    let BundleStatus::FinalizedExact(receipt) = store
        .authoritative_status(token, bundle_hash)
        .await
        .unwrap()
    else {
        panic!("exact replay lost its finalized witness");
    };
    assert_eq!(receipt.outcome().data()["revision"], 1);

    write_frame(
        &mut writer,
        &ToServer::Control(mush_core::ControlMsg::Shutdown),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actor_field_cannot_spoof_the_authenticated_connection() {
    let (endpoint, _store) = boot_server("spoof-rejected").await;
    let (mut frames, mut writer) = hello_and_attach(&endpoint).await;
    let offer = brace_offer(&mut frames, &mut writer).await;
    let request = action_request(2, LIMBO, offer);
    write_frame(&mut writer, &ToServer::Input(request.clone()))
        .await
        .unwrap();

    let disposition = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let ToGateway::RequestDisposition {
                request: seen,
                disposition,
            } = frames.read::<ToGateway>().await.unwrap().unwrap()
                && seen == request.uid
            {
                if disposition == GatewayRequestDisposition::Busy {
                    // Admission can refuse before the actor check runs. Keep
                    // the spoofed request byte-identical and retry only this
                    // pre-ownership response inside the existing deadline.
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    write_frame(&mut writer, &ToServer::Input(request.clone()))
                        .await
                        .unwrap();
                    continue;
                }
                break disposition;
            }
        }
    })
    .await
    .expect("spoof rejection timed out");
    assert_eq!(
        disposition,
        GatewayRequestDisposition::Rejected { retryable: false }
    );

    write_frame(
        &mut writer,
        &ToServer::Control(mush_core::ControlMsg::Shutdown),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tampered_offer_tag_is_rejected_before_intent() {
    let (endpoint, _store) = boot_server("tampered-offer").await;
    let (mut frames, mut writer) = hello_and_attach(&endpoint).await;
    let offer = brace_offer(&mut frames, &mut writer).await;
    let mut tampered = *offer.0.as_bytes();
    tampered[31] ^= 0x01;
    let request = action_request(3, GOD, ActionOfferId(CanonicalHash::from_bytes(tampered)));
    write_frame(&mut writer, &ToServer::Input(request.clone()))
        .await
        .unwrap();

    let disposition = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let ToGateway::RequestDisposition {
                request: seen,
                disposition,
            } = frames.read::<ToGateway>().await.unwrap().unwrap()
                && seen == request.uid
            {
                if disposition == GatewayRequestDisposition::Busy {
                    // Admission may be busy before offer validation. Retry
                    // the same tampered input within the enclosing deadline,
                    // then still require the exact permanent rejection.
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    write_frame(&mut writer, &ToServer::Input(request.clone()))
                        .await
                        .unwrap();
                    continue;
                }
                break disposition;
            }
        }
    })
    .await
    .expect("tampered-offer rejection timed out");
    assert_eq!(
        disposition,
        GatewayRequestDisposition::Rejected { retryable: false }
    );

    write_frame(
        &mut writer,
        &ToServer::Control(mush_core::ControlMsg::Shutdown),
    )
    .await
    .unwrap();
}

/// A raw IPC peer can admit an authoritative action and then send
/// `GatewayReleased` naming the same uid — a frame *pair* no conforming gateway
/// produces, since the gateway releases only what the server answered
/// `LegacyClaimed`. The release path used to convert the released payload with
/// an infallible `From` whose action arm was an `unreachable!`, so that pair
/// panicked the IPC reader loop and, in split mode, aborted into the supervisor
/// relaunch loop with every session on the boundary dropped.
///
/// The boundary must survive the pair, and the coordinator must remain the sole
/// author of the request's outcome: after any pre-ownership Busy retries, exactly
/// one terminal disposition per uid, the coordinator's refusal of the spoofed
/// actor. The release races the in-flight handler, so the pair is driven
/// repeatedly — a single lost race is
/// what used to kill the process, and the fix must hold for every ordering.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_release_of_a_coordinator_action_cannot_kill_the_reader_loop() {
    let (endpoint, _store) = boot_server("action-release-pair").await;
    let (mut frames, mut writer) = hello_and_attach(&endpoint).await;
    let offer = brace_offer(&mut frames, &mut writer).await;

    for sequence in 10..22u64 {
        // A spoofed actor: the coordinator crosses the store and the world
        // before it can refuse, which is the window the release lands in.
        let request = action_request(sequence, LIMBO, offer);
        write_frame(&mut writer, &ToServer::Input(request.clone()))
            .await
            .unwrap();
        write_frame(
            &mut writer,
            &ToServer::GatewayReleased {
                request: request.uid,
            },
        )
        .await
        .unwrap();

        let disposition = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let ToGateway::RequestDisposition {
                    request: seen,
                    disposition,
                } = frames.read::<ToGateway>().await.unwrap().unwrap()
                {
                    assert_eq!(
                        seen, request.uid,
                        "no disposition may exist for a uid other than the one in flight"
                    );
                    if disposition == GatewayRequestDisposition::Busy {
                        // Nothing was owned, so retry the same frame pair.
                        // Releasing again is essential: the admitted attempt
                        // must still exercise the malicious-release race.
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        write_frame(&mut writer, &ToServer::Input(request.clone()))
                            .await
                            .unwrap();
                        write_frame(
                            &mut writer,
                            &ToServer::GatewayReleased {
                                request: request.uid,
                            },
                        )
                        .await
                        .unwrap();
                        continue;
                    }
                    break disposition;
                }
            }
        })
        .await
        .expect("the reader loop died on the release of a coordinator claim");
        assert_eq!(
            disposition,
            GatewayRequestDisposition::Rejected { retryable: false },
            "the coordinator alone decides an action's fate"
        );
    }

    // No second disposition for any of those uids: the peer's release published
    // nothing of its own and freed no uid for a duplicate run.
    write_frame(
        &mut writer,
        &ToServer::Request {
            id: 950,
            req: ServiceReq::Barrier,
        },
    )
    .await
    .unwrap();
    let probe = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match frames.read::<ToGateway>().await.unwrap().unwrap() {
                ToGateway::Reply { id: 950, resp } => break resp,
                ToGateway::RequestDisposition { request, .. } => {
                    panic!("extra disposition for {request} after the coordinator finished")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("boundary liveness probe timed out");
    assert_eq!(
        probe,
        ServiceResp::Barrier,
        "the boundary must still serve this connection after the refused releases"
    );

    write_frame(
        &mut writer,
        &ToServer::Control(mush_core::ControlMsg::Shutdown),
    )
    .await
    .unwrap();
}

/// WS04-S2: the boot `Hello` advertises the TacSim layer, because the split half
/// now composes it.
///
/// One feature id per composed module is what the gateway copies into the web
/// client's `hello` frame, so a module the forked half never composed was a
/// capability the split web client could not see. `[tacsim].enabled` is false on
/// this config and that is deliberate: the layer composes either way (its verbs
/// answer "not enabled" rather than vanishing), which is exactly what makes the
/// advertised feature list the same on both halves.
///
/// This file's own rig boots through `serve_with_store(Some(store))` — the
/// caller-supplied branch that bypasses `open_store` and therefore arms no
/// archive tier. The assertion is unaffected: `Hello` is built from the booted
/// engine, not from the store.
#[tokio::test]
async fn hello_advertises_the_tacsim_feature() {
    let (endpoint, _store) = boot_server("hello-features").await;
    let (reader, _writer) = mush_ipc::connect(&endpoint).await.unwrap();
    let mut frames = FrameReader::new(reader);
    let ToGateway::Hello { features, .. } = frames.read::<ToGateway>().await.unwrap().unwrap()
    else {
        panic!("the boundary opens with a Hello");
    };
    assert!(
        features.iter().any(|f| f == "tacsim"),
        "the split half must advertise the TacSim layer it now composes: {features:?}"
    );
    // The renaming the manifest has always done, asserted beside it so the two
    // halves of `build_hello_manifest` are pinned by one case.
    assert!(
        features.iter().any(|f| f == "channels"),
        "comsys still advertises as `channels`: {features:?}"
    );
}

// ---------------------------------------------------------------------------
// WS03-S4: the custody-leaking coordinator returns
// ---------------------------------------------------------------------------

/// Which answer the store gives the coordinator's read-only preflight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreflightFault {
    /// Answer honestly from the in-memory store.
    None,
    /// Answer with a *transient* error, forever. Nothing about the request's
    /// durable fate is ever proved, which is the state the retry loop used to
    /// spin on without a ceiling or a deadline.
    Transient,
    /// Answer that the exact request is quarantined: terminal evidence that it
    /// consumed its idempotency binding without committing.
    Quarantined,
}

/// The commit token the planted quarantine receipt carries, so the test can
/// prove the disposition names *that* durable record and not some other one.
const QUARANTINE_TOKEN: [u8; 16] = [0x5a; 16];

/// An [`ObjectStore`] whose authoritative *preflight lookup* can be made to
/// fail transiently forever, or to report a quarantined binding. Everything
/// else — the whole boot, the world's writes, the rest of the authoritative
/// protocol — is the ordinary in-memory store.
struct PreflightFaultStore {
    inner: InMemoryStore,
    fault: Mutex<PreflightFault>,
    quarantined: AuthoritativeQuarantineReceipt,
}

impl PreflightFaultStore {
    fn new() -> Arc<PreflightFaultStore> {
        Arc::new(PreflightFaultStore {
            inner: InMemoryStore::new(),
            fault: Mutex::new(PreflightFault::None),
            quarantined: quarantine_receipt(),
        })
    }

    fn arm(&self, fault: PreflightFault) {
        *self.fault.lock().unwrap() = fault;
    }

    fn fault(&self) -> PreflightFault {
        *self.fault.lock().unwrap()
    }

    fn authoritative(&self) -> &dyn AuthoritativeStore {
        self.inner
            .authoritative_store()
            .expect("the in-memory store is authoritative")
    }
}

/// A valid quarantine receipt to plant. Its own identity is arbitrary: the
/// coordinator reads only the token and the evidence hash off it, and the test
/// asserts the disposition carries exactly this token.
fn quarantine_receipt() -> AuthoritativeQuarantineReceipt {
    let owner = OwnerId::new("preflight_fault").expect("owner");
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").expect("kind"),
                DefinitionId::new("brace").expect("definition"),
                1,
                serde_json::json!({}),
            )
            .expect("action descriptor"),
        )
        .expect("register action");
    let outcome = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("outcome").expect("kind"),
                DefinitionId::new("braced").expect("definition"),
                1,
                serde_json::json!({}),
            )
            .expect("outcome descriptor"),
        )
        .expect("register outcome");
    let request = RequestUid::from_bytes([0x33; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let cause = PreparedCausalRef::new(request, None, 0).expect("root cause");
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: CommitToken::from_bytes(QUARANTINE_TOKEN),
        request,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: owner.clone(),
                namespace: CanonicalId::new("brace").expect("namespace"),
            },
            1,
            IdempotencyKey::Request(request),
        ),
        ingress_hash: CanonicalHash::from_bytes([9; 32]),
        request_hash: CanonicalHash::from_bytes([3; 32]),
        plan_hash: CanonicalHash::from_bytes([4; 32]),
        action: action.clone(),
        provider: owner.clone(),
        cause,
        definitions: vec![action.clone(), outcome.clone()],
        writes: vec![
            ReplayWrite::ObjectSet {
                object: Obj::new(ObjId(2), ObjKind::Thing, "quarantined", GOD, 0),
            },
            ReplayWrite::ObjectAllocatorSet { next_id: 3 },
        ],
        outcome: RulesValue::new(outcome.clone(), serde_json::json!({ "ok": true }))
            .expect("outcome value"),
        resolution: None,
        semantic_events: vec![
            AuthoritativeSemanticEvent::root(
                cause,
                &action,
                RulesValue::new(outcome, serde_json::json!({ "ok": true })).expect("event value"),
                vec![principal.clone()],
            )
            .expect("root event"),
        ],
        eligible_witnesses: vec![principal],
        durable_outbox: Vec::new(),
    })
    .expect("quarantined bundle");
    AuthoritativeQuarantineReceipt::new(&bundle, CanonicalHash::from_bytes([0xee; 32]))
}

impl AuthoritativeStore for PreflightFaultStore {
    fn authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
        ingress_hash: CanonicalHash,
    ) -> BoxFuture<'a, Result<IdempotencyStatus, StoreError>> {
        match self.fault() {
            PreflightFault::None => self
                .authoritative()
                .authoritative_idempotency_status(key, ingress_hash),
            // Transient by construction: `is_transient()` is true for it, so
            // the coordinator must read it as "unproven", never as "no intent
            // exists".
            PreflightFault::Transient => {
                Box::pin(async move { Err(StoreError::AuthoritativeEpochContention) })
            }
            PreflightFault::Quarantined => {
                let receipt = self.quarantined.clone();
                Box::pin(async move { Ok(IdempotencyStatus::QuarantinedExact(Box::new(receipt))) })
            }
        }
    }

    fn reserve_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>> {
        self.authoritative().reserve_authoritative_intent(bundle)
    }

    fn bound_authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
    ) -> BoxFuture<'a, Result<BoundIdempotencyStatus, StoreError>> {
        self.authoritative()
            .bound_authoritative_idempotency_status(key)
    }

    fn reconcile_authoritative(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeReceipt, StoreError>> {
        self.authoritative()
            .reconcile_authoritative(token, expected_bundle_hash)
    }

    fn checkpoint_authoritative(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeCheckpointReceipt, StoreError>> {
        self.authoritative()
            .checkpoint_authoritative(token, expected_bundle_hash)
    }

    fn discard_quarantined_authoritative_intent(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
        evidence_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeDiscardReceipt, StoreError>> {
        self.authoritative()
            .discard_quarantined_authoritative_intent(token, expected_bundle_hash, evidence_hash)
    }

    fn restore_quarantined_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
        evidence_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>> {
        self.authoritative()
            .restore_quarantined_authoritative_intent(bundle, evidence_hash)
    }

    fn pending_authoritative_outbox(
        &self,
    ) -> BoxFuture<'_, Result<Vec<AuthoritativeOutboxRecord>, StoreError>> {
        self.authoritative().pending_authoritative_outbox()
    }

    fn begin_authoritative_outbox_attempt(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_attempts: u32,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>> {
        self.authoritative().begin_authoritative_outbox_attempt(
            item_id,
            expected_payload_hash,
            expected_attempts,
        )
    }

    fn complete_authoritative_outbox_attempt(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_attempt: u32,
        result: AuthoritativeOutboxAttemptResult,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>> {
        self.authoritative().complete_authoritative_outbox_attempt(
            item_id,
            expected_payload_hash,
            expected_attempt,
            result,
        )
    }

    fn note_authoritative_outbox_unroutable(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_passes: u32,
        error: String,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>> {
        self.authoritative().note_authoritative_outbox_unroutable(
            item_id,
            expected_payload_hash,
            expected_passes,
            error,
        )
    }

    fn retire_authoritative_epochs<'a>(
        &'a self,
        scope: &'a IdempotencyScope,
        minimum_accepted_epoch: u64,
    ) -> BoxFuture<'a, Result<AuthoritativeEpochRetirement, StoreError>> {
        self.authoritative()
            .retire_authoritative_epochs(scope, minimum_accepted_epoch)
    }

    fn authoritative_status(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<BundleStatus, StoreError>> {
        self.authoritative()
            .authoritative_status(token, expected_bundle_hash)
    }

    fn pending_authoritative_intents(
        &self,
    ) -> BoxFuture<'_, Result<Vec<PendingAuthoritativeIntent>, StoreError>> {
        self.authoritative().pending_authoritative_intents()
    }

    fn scan_authoritative_intents(
        &self,
        cursor: Option<AuthoritativeScanCursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<AuthoritativeIntentPage, StoreError>> {
        self.authoritative()
            .scan_authoritative_intents(cursor, limit)
    }

    fn authoritative_retention_generation(&self) -> BoxFuture<'_, Result<u64, StoreError>> {
        self.authoritative().authoritative_retention_generation()
    }

    fn repair_authoritative_retention(
        &self,
        expected_generation: u64,
        usage: AuthoritativeRetentionUsage,
    ) -> BoxFuture<'_, Result<bool, StoreError>> {
        self.authoritative()
            .repair_authoritative_retention(expected_generation, usage)
    }
}

impl ObjectStore for PreflightFaultStore {
    fn authoritative_store(&self) -> Option<&dyn AuthoritativeStore> {
        Some(self)
    }

    fn bootstrap_writable(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.bootstrap_writable()
    }

    fn load_world(&self) -> BoxFuture<'_, Result<Option<WorldSnapshot>, StoreError>> {
        self.inner.load_world()
    }

    fn write(&self, batch: WriteBatch) -> BoxFuture<'_, Result<WriteReceipt, StoreError>> {
        self.inner.write(batch)
    }

    fn create_account(&self, rec: AccountRec) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.create_account(rec)
    }

    fn get_account<'a>(
        &'a self,
        account: &'a str,
    ) -> BoxFuture<'a, Result<Option<AccountRec>, StoreError>> {
        self.inner.get_account(account)
    }

    fn link_character<'a>(
        &'a self,
        account: &'a str,
        id: ObjId,
        name: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        self.inner.link_character(account, id, name)
    }

    fn list_accounts(&self) -> BoxFuture<'_, Result<Vec<AccountRec>, StoreError>> {
        self.inner.list_accounts()
    }

    fn set_account_password<'a>(
        &'a self,
        account: &'a str,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.set_account_password(account, hash)
    }

    fn set_account_ssh_keys<'a>(
        &'a self,
        account: &'a str,
        keys: &'a [SshKeyRec],
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.set_account_ssh_keys(account, keys)
    }

    fn set_account_discord<'a>(
        &'a self,
        account: &'a str,
        link: Option<&'a DiscordLinkRec>,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.set_account_discord(account, link)
    }

    fn discord_user_reserve<'a>(
        &'a self,
        user_id: u64,
        account: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.discord_user_reserve(user_id, account)
    }

    fn discord_user_release(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        self.inner.discord_user_release(user_id)
    }

    fn account_by_discord_user(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<AccountRec>, StoreError>> {
        self.inner.account_by_discord_user(user_id)
    }

    fn discord_code_put(
        &self,
        code: String,
        rec: DiscordCodeRecord,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.discord_code_put(code, rec)
    }

    fn discord_code_take<'a>(
        &'a self,
        code: &'a str,
    ) -> BoxFuture<'a, Result<Option<DiscordCodeRecord>, StoreError>> {
        self.inner.discord_code_take(code)
    }

    fn discord_ban_put(&self, rec: DiscordBanRec) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.discord_ban_put(rec)
    }

    fn discord_ban_get(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<DiscordBanRec>, StoreError>> {
        self.inner.discord_ban_get(user_id)
    }

    fn discord_ban_del(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        self.inner.discord_ban_del(user_id)
    }

    fn set_ban<'a>(
        &'a self,
        ip: &'a str,
        until_unix: Option<i64>,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        self.inner.set_ban(ip, until_unix, reason)
    }

    fn get_bans(&self) -> BoxFuture<'_, Result<Vec<BanRec>, StoreError>> {
        self.inner.get_bans()
    }

    fn clear_ban<'a>(&'a self, ip: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.clear_ban(ip)
    }

    fn put_tempban_lift(
        &self,
        lift: mush_store::TempbanLift,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.put_tempban_lift(lift)
    }
    fn get_tempban_lifts(&self) -> BoxFuture<'_, Result<Vec<mush_store::TempbanLift>, StoreError>> {
        self.inner.get_tempban_lifts()
    }

    fn token_put(&self, hash: String, rec: TokenRecord) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.token_put(hash, rec)
    }

    fn token_get<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<TokenRecord>, StoreError>> {
        self.inner.token_get(hash)
    }

    fn token_touch<'a>(
        &'a self,
        hash: &'a str,
        now: i64,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.token_touch(hash, now)
    }

    fn token_del<'a>(&'a self, hash: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.token_del(hash)
    }

    fn tokens_del_account<'a>(
        &'a self,
        account: &'a str,
    ) -> BoxFuture<'a, Result<u64, StoreError>> {
        self.inner.tokens_del_account(account)
    }

    fn module_json_get<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
        self.inner.module_json_get(ns, key)
    }

    fn module_json_list<'a>(
        &'a self,
        ns: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        self.inner.module_json_list(ns)
    }

    fn module_docs<'a>(&'a self, ns: &'a str) -> BoxFuture<'a, Result<ModuleDocs, StoreError>> {
        self.inner.module_docs(ns)
    }

    fn module_docs_from<'a>(
        &'a self,
        ns: &'a str,
        skip: &'a BootSkip,
    ) -> BoxFuture<'a, Result<ModuleDocs, StoreError>> {
        self.inner.module_docs_from(ns, skip)
    }

    fn module_keys<'a>(&'a self, ns: &'a str) -> BoxFuture<'a, Result<Vec<String>, StoreError>> {
        self.inner.module_keys(ns)
    }

    fn module_doc_text<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, StoreError>> {
        self.inner.module_doc_text(ns, key)
    }

    fn module_purge_archived<'a>(
        &'a self,
        ns: &'a str,
        keys: Vec<String>,
    ) -> BoxFuture<'a, Result<usize, StoreError>> {
        self.inner.module_purge_archived(ns, keys)
    }

    fn ledger_archive(&self) -> Option<&mush_store::LedgerArchive> {
        self.inner.ledger_archive()
    }

    fn stream_range<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
        count_latest: u32,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        self.inner.stream_range(ns, key, count_latest)
    }

    fn get_schema_version(&self) -> BoxFuture<'_, Result<Option<u32>, StoreError>> {
        self.inner.get_schema_version()
    }

    fn set_schema_version(&self, version: u32) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.set_schema_version(version)
    }

    fn bgsave(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.bgsave()
    }

    fn drain(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.drain()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.close()
    }
}

async fn boot_fault_server(tag: &'static str) -> (mush_ipc::IpcEndpoint, Arc<PreflightFaultStore>) {
    let (bound, bound_rx) = tokio::sync::oneshot::channel();
    let store = PreflightFaultStore::new();
    let server_store: Arc<dyn ObjectStore> = store.clone();
    let config = test_config(tag);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(mush_server::serve_with_store(
                config,
                Some(bound),
                Some(server_store),
            ))
            .unwrap();
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(30), bound_rx)
        .await
        .expect("server bind timed out")
        .expect("server returned bound endpoint");
    (endpoint, store)
}

/// The next disposition published for `uid`; a disposition for any other uid
/// is a bug this rig refuses to skip past.
async fn next_disposition(
    frames: &mut FrameReader<mush_ipc::BoxRead>,
    uid: mush_ipc::RequestUid,
    what: &str,
) -> GatewayRequestDisposition {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let ToGateway::RequestDisposition {
                request: seen,
                disposition,
            } = frames.read::<ToGateway>().await.unwrap().unwrap()
            {
                assert_eq!(seen, uid, "a disposition for an unexpected request");
                break disposition;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what} timed out"))
}

/// A quarantined preflight used to `return` with the claim still in the table
/// and nothing published: the gateway kept its replay copy forever, the server
/// kept its custody slot forever, and every replay of that uid was swallowed as
/// a `Duplicate`. One connection lost a custody slot per incident and the
/// player was told nothing at all.
///
/// Quarantine is terminal evidence, so the answer is terminal too: `Owned`
/// naming the quarantined record's own token — truthful (the intent IS durably
/// owned), terminal for the gateway, and needing no new wire variant. Custody
/// is surrendered on both sides, which the replay below proves: it is admitted
/// afresh and answered from the store again, without the action ever being
/// planned.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_quarantined_preflight_is_answered_and_releases_custody() {
    let (endpoint, store) = boot_fault_server("preflight-quarantined").await;
    let (mut frames, mut writer) = hello_and_attach(&endpoint).await;
    let offer = brace_offer(&mut frames, &mut writer).await;

    store.arm(PreflightFault::Quarantined);
    let request = action_request(30, GOD, offer);
    write_frame(&mut writer, &ToServer::Input(request.clone()))
        .await
        .unwrap();

    let first = next_disposition(&mut frames, request.uid, "quarantine disposition").await;
    let GatewayRequestDisposition::Owned { token, status } = first else {
        panic!("a quarantined request must be answered as durably owned: {first:?}");
    };
    assert_eq!(
        token,
        CommitToken::from_bytes(QUARANTINE_TOKEN),
        "the disposition must name the quarantined record's own token"
    );

    // Custody was surrendered on both sides, so the exact replay is a fresh
    // admission rather than a swallowed duplicate — and it is answered from
    // the durable record again, not planned.
    write_frame(&mut writer, &ToServer::Input(request.clone()))
        .await
        .unwrap();
    let replay = next_disposition(&mut frames, request.uid, "quarantine replay disposition").await;
    assert_eq!(
        replay,
        GatewayRequestDisposition::Owned { token, status },
        "the replay must be answered from the store, not re-planned"
    );

    // And nothing else was ever published for it.
    write_frame(
        &mut writer,
        &ToServer::Request {
            id: 960,
            req: ServiceReq::Barrier,
        },
    )
    .await
    .unwrap();
    let probe = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match frames.read::<ToGateway>().await.unwrap().unwrap() {
                ToGateway::Reply { id: 960, resp } => break resp,
                ToGateway::RequestDisposition { request, .. } => {
                    panic!("extra disposition for {request} after the quarantine answer")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("boundary liveness probe timed out");
    assert_eq!(probe, ServiceResp::Barrier);

    write_frame(
        &mut writer,
        &ToServer::Control(mush_core::ControlMsg::Shutdown),
    )
    .await
    .unwrap();
}

/// The transient-preflight loop had an exponential backoff capped at five
/// seconds and neither a retry ceiling nor a deadline: against a store that
/// stays transient it retried forever, holding a custody slot and a detached
/// task per request while publishing nothing.
///
/// It is bounded now. Giving up *retains* the claim rather than lying about the
/// request's fate, so nothing is published and the gateway keeps its replay
/// copy — and each exact replay re-arms the claim for one more attempt. That
/// re-arming is bounded too: after `MAX_CLAIM_REARMS` the server surrenders
/// custody and answers `Rejected { retryable: true }`, the vocabulary the
/// gateway already drops its own replay copy on. No uid can loop forever, and
/// the connection is not wedged: once the store heals, a fresh action commits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_permanently_transient_preflight_retains_and_then_stops_at_the_rearm_bound() {
    let (endpoint, store) = boot_fault_server("preflight-transient").await;
    let (mut frames, mut writer) = hello_and_attach(&endpoint).await;
    let offer = brace_offer(&mut frames, &mut writer).await;

    store.arm(PreflightFault::Transient);
    let request = action_request(40, GOD, offer);
    let uid = request.uid;
    write_frame(&mut writer, &ToServer::Input(request.clone()))
        .await
        .unwrap();

    // Read on its own task: cancelling a frame read to drive the replays would
    // risk losing a partially-read frame.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let reader = tokio::spawn(async move {
        loop {
            if let ToGateway::RequestDisposition {
                request: seen,
                disposition,
            } = frames.read::<ToGateway>().await.unwrap().unwrap()
            {
                assert_eq!(seen, uid, "a disposition for an unexpected request");
                let _ = tx.send(disposition);
                break;
            }
        }
        frames
    });

    let mut rx = rx;
    let disposition = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            tokio::select! {
                got = &mut rx => break got.expect("the reader task delivered the disposition"),
                () = tokio::time::sleep(Duration::from_millis(250)) => {
                    // The gateway's replay cadence, compressed: a replay that
                    // lands while the claim is retained re-arms it, and one
                    // that lands while a handler runs is an ordinary duplicate.
                    write_frame(&mut writer, &ToServer::Input(request.clone()))
                        .await
                        .unwrap();
                }
            }
        }
    })
    .await
    .expect("the retained claim never stopped re-arming");
    assert_eq!(
        disposition,
        GatewayRequestDisposition::Rejected { retryable: true },
        "the re-arm bound hands the request back to the gateway"
    );
    let mut frames = tokio::time::timeout(Duration::from_secs(10), reader)
        .await
        .expect("reader task")
        .expect("reader join");

    // The connection is not wedged: its custody budget came back with the
    // exhausted claim, so a fresh action owns once the store heals.
    store.arm(PreflightFault::None);
    let fresh = action_request(41, GOD, offer);
    write_frame(&mut writer, &ToServer::Input(fresh.clone()))
        .await
        .unwrap();
    let owned = next_disposition(&mut frames, fresh.uid, "healed action disposition").await;
    assert!(
        matches!(owned, GatewayRequestDisposition::Owned { .. }),
        "the connection must still be able to own an action: {owned:?}"
    );

    write_frame(
        &mut writer,
        &ToServer::Control(mush_core::ControlMsg::Shutdown),
    )
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// WS03-S15: the retained-claim reaper never outlives its reader loop
// ---------------------------------------------------------------------------

/// The world loop needs a sink; the reaper proof inspects no delivery.
struct BlackholeGateway;

impl mush_server::gateway_sink::GatewaySink for BlackholeGateway {
    fn transmit(&self, _sends: Vec<(mush_ipc::SendTarget, Arc<mush_proto::OutputEvent>)>) {}

    fn set_flood_exempt(&self, _char: u64, _exempt: bool) {}

    fn is_socket(&self) -> bool {
        false
    }

    fn close_session(&self, _char: u64, _cause: mush_ipc::DetachCause) {}

    fn close_web_sessions(
        &self,
        _chars: Vec<u64>,
        _notice: Option<mush_proto::OutputEvent>,
        _cause: mush_ipc::DetachCause,
    ) {
    }

    fn close_char_connections(&self, _char: u64) -> Vec<u64> {
        Vec::new()
    }

    fn structured_conns_of(&self, _chars: &[u64]) -> Vec<(u64, u64)> {
        Vec::new()
    }

    fn detach_conn_local(&self, _conn: u64) {}

    fn session_counts(&self) -> (i64, i64) {
        (0, 0)
    }
}

/// A live world task, so `reader_loop` is driven against the real handle it
/// takes in production. This proof never reaches the world: the frame it feeds
/// is refused by the reader's own guard first.
fn boot_world() -> mush_server::world_task::PersistHandles {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let engine = mush_core::Engine::with_modules(
        mush_core::World::new_minimal(),
        mush_server::world_task::base_registry(),
        Vec::new(),
    )
    .expect("the minimal engine composes");
    mush_server::world_task::spawn_persist_with_gateway(
        engine,
        mush_server::world_task::ServerDeps {
            store,
            guard: Arc::new(mush_config::ipguard::IpGuard::new()),
            staff: Arc::new(mush_server::staff::StaffQueue::new()),
            llm: None,
            metrics: Arc::new(mush_config::metrics::Metrics::new()),
            quarantine_spool: mush_server::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
        |_| -> mush_server::world_task::Gateway { Arc::new(BlackholeGateway) },
    )
}

/// Watch one runtime's live task count until `ready` accepts it. The reader
/// loop is the only thing on that runtime, so the count is exactly the reader
/// task plus whatever the reader task spawned.
fn wait_for_tasks(rt: &tokio::runtime::Runtime, ready: impl Fn(usize) -> bool, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let alive = rt.metrics().num_alive_tasks();
        if ready(alive) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what} (live tasks on the reader's runtime: {alive})"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The retained-claim reaper is spawned at the TOP of `reader_loop` and holds
/// an `Arc` to that boundary's claim table, so it may not survive the loop.
/// `reader_loop` does not merely fall off its end: it has several early
/// `return Err(..)` exits — the raw-input guard proven here, the
/// invalid/conflicting-identity arms, every `world_is_gone` — and `serve` can
/// drop the reader future outright. A reaper cancelled only at the tail of the
/// function would outlive every one of those, sleeping forever on a dead
/// boundary's custody.
///
/// The proof runs the real reader on a runtime of its own, so the live-task
/// count is unambiguous: it must rise (the reaper exists at all) and then fall
/// back to nothing once the guard's early return drops the abort-on-drop
/// handle. A leaked reaper never lets it fall.
#[test]
fn an_early_reader_loop_error_does_not_leave_a_reaper_running() {
    // The world lives on its own runtime, so the runtime under test carries
    // the reader loop and nothing else.
    let world_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (world, world_exit) = world_rt.block_on(async {
        let handles = boot_world();
        (handles.handle.clone(), handles.world_exit.clone())
    });

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    assert_eq!(rt.metrics().num_alive_tasks(), 0, "a runtime of its own");

    // A duplex whose peer end this test holds open, so the reader parks on the
    // socket instead of ending at EOF before anything can be observed.
    let (mut peer, wire) = tokio::io::duplex(64 * 1024);
    let reader = rt.spawn(async move {
        let (tx, _replies) = tokio::sync::mpsc::channel(64);
        let metrics = Arc::new(mush_config::metrics::Metrics::new());
        let sink = mush_server::gateway_sink::SocketSink::new(
            tx,
            Arc::clone(&metrics),
            Arc::new(tokio::sync::Notify::new()),
            64,
            90,
        );
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        mush_server::reader_loop(
            Box::new(wire),
            world,
            store,
            Arc::new(mush_core::OfferRouteTable::default()),
            sink,
            &metrics,
            world_exit,
        )
        .await
    });

    wait_for_tasks(
        &rt,
        |alive| alive >= 2,
        "the reader loop must spawn its retained-claim reaper",
    );

    // Raw player input that bypassed the custody envelope: an early
    // `return Err(..)`, nowhere near the end of the function.
    rt.block_on(write_frame(
        &mut peer,
        &ToServer::Inbound(WireInbound::Cmd {
            conn: 1,
            actor: GOD,
            line: "look".to_string(),
        }),
    ))
    .expect("the frame is written to the reader's socket");
    let error = rt
        .block_on(reader)
        .expect("the reader task itself must not panic")
        .expect_err("raw player input must end the boundary");
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidData,
        "the raw-input guard is the exit taken: {error}"
    );

    wait_for_tasks(
        &rt,
        |alive| alive == 0,
        "the reaper must die with the reader loop's early return, not outlive it",
    );
}
