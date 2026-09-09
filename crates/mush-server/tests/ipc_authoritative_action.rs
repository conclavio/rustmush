//! End-to-end K2/K3 ownership transfer over the real split-process protocol.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_config::config::Config;
use mush_core::{ActionOfferId, CanonicalHash, GOD, LIMBO, PerceivedRef, PrincipalId};
use mush_ipc::{
    FrameReader, GatewayAction, GatewayInput, GatewayRequest, GatewayRequestDisposition,
    ServiceReq, ServiceResp, ToGateway, ToServer, WireInbound, write_frame,
};
use mush_proto::ProjectedActionStatus;
use mush_server::action_coordinator::ActionCoordinator;
use mush_store::BoundIdempotencyStatus;
use mush_store::{AuthoritativeStore, BundleStatus, InMemoryStore, ObjectStore};

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
    config.rulesets.enabled = vec![];
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
/// author of the request's outcome: exactly one disposition per uid, and it is
/// the coordinator's own refusal of the spoofed actor. The release races the
/// in-flight handler, so the pair is driven repeatedly — a single lost race is
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
