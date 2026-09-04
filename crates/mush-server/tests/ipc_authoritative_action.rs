//! End-to-end K2/K3 ownership transfer over the real split-process protocol.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_config::config::Config;
use mush_core::{ActionOfferId, CanonicalHash, GOD, LIMBO, PerceivedRef};
use mush_ipc::{
    FrameReader, GatewayAction, GatewayInput, GatewayRequest, GatewayRequestDisposition,
    ServiceReq, ServiceResp, ToGateway, ToServer, WireInbound, write_frame,
};
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

    let (token, bundle_hash) = tokio::time::timeout(Duration::from_secs(15), async {
        let mut owned = None;
        loop {
            match frames.read::<ToGateway>().await.unwrap().unwrap() {
                ToGateway::RequestDisposition {
                    request: seen,
                    disposition: GatewayRequestDisposition::Owned { token, bundle_hash },
                } if seen == request.uid => owned = Some((token, bundle_hash)),
                ToGateway::RequestDisposition {
                    request: seen,
                    disposition: GatewayRequestDisposition::Finished { token },
                } if seen == request.uid => {
                    let (owned_token, hash) = owned.expect("Finished arrived before Owned");
                    assert_eq!(token, owned_token);
                    break (token, hash);
                }
                _ => {}
            }
        }
    })
    .await
    .expect("authoritative dispositions timed out");

    assert!(matches!(
        store
            .authoritative_status(token, bundle_hash)
            .await
            .unwrap(),
        BundleStatus::FinalizedExact(receipt)
            if receipt.token() == token && receipt.bundle_hash() == bundle_hash
    ));

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
        GatewayRequestDisposition::Finished { token },
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
