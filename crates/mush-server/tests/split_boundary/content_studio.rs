//! End-to-end split-boundary and persistence tests for the Staff Content Studio
//! (`InvokeRequest::ContentStudio`: `Catalog` → `Read` → `Assist` → `Check` → `Publish`
//! with `relocate_npcs`, doc 34 cut CS.5c).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mush_config::config::Config;
use mush_core::{Audience, CanonicalId, Effects, GOD, Inbound, Outbox};
use mush_ipc::{
    FrameReader, GatewayInput, GatewayRequest, GatewayRequestDisposition, IPC_MIN_PROTO,
    IPC_VERSION, SendTarget, ToGateway, ToServer, WireInbound, write_frame,
};
use mush_proto::{
    ContentStudioRequest, ContentStudioResponse, InvokeRequest, InvokeResult, OutputEvent,
};
use mush_server::{boot, persist, quarantine::QuarantineSpool};
use mush_store::{InMemoryStore, ObjectStore};
use mush_testkit::TestWorld;
use serde_json::{Value, json};

fn scratch_dir(tag: &str) -> PathBuf {
    static NEXT_DIR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rustmush-content-studio-{tag}-{}-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_owned();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.server.help_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/help");
    cfg.ssh.listen = "127.0.0.1:0".to_owned();
    cfg.ipc.listen = "127.0.0.1:0".to_owned();
    cfg.tacsim.enabled = true;
    cfg.tacsim.geology.key_dir = Some(cfg.server.data_dir.join("geology-keys"));
    cfg
}

async fn boot_ipc_server(tag: &'static str) -> (mush_ipc::IpcEndpoint, PathBuf) {
    let (bound, bound_rx) = tokio::sync::oneshot::channel();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let config = test_config(tag);
    let data_dir = config.server.data_dir.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("server runtime");
        runtime
            .block_on(mush_server::serve_with_store(
                config,
                Some(bound),
                Some(store),
            ))
            .expect("server exits cleanly");
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(60), bound_rx)
        .await
        .expect("server bind timed out")
        .expect("server returned bound endpoint");
    (endpoint, data_dir)
}

async fn invoke_over_ipc(
    frames: &mut FrameReader<mush_ipc::BoxRead>,
    writer: &mut mush_ipc::BoxWrite,
    conn: u64,
    id: u64,
    req: ContentStudioRequest,
) -> ContentStudioResponse {
    let gw_req = GatewayRequest::new(
        77,
        id,
        GatewayInput::Invoke {
            conn,
            actor: GOD,
            id,
            request: InvokeRequest::ContentStudio(req),
        },
    );
    let uid = gw_req.uid;
    write_frame(writer, &ToServer::Input(gw_req))
        .await
        .expect("write Input(GatewayRequest::Invoke) frame");

    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let frame = frames
                .read::<ToGateway>()
                .await
                .expect("ipc frame")
                .expect("ipc link open");
            match frame {
                ToGateway::RequestDisposition {
                    request,
                    disposition: GatewayRequestDisposition::LegacyClaimed,
                } if request == uid => {
                    write_frame(writer, &ToServer::GatewayReleased { request: uid })
                        .await
                        .expect("write GatewayReleased frame");
                }
                ToGateway::Deliver { target, event } if target == SendTarget::Conn(conn) => {
                    if let OutputEvent::InvokeResult {
                        id: reply_id,
                        result: InvokeResult::ContentStudio(resp),
                    } = event
                        && reply_id == id
                    {
                        return resp;
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for ContentStudio InvokeResult over IPC")
}

fn step_inbound(tw: &mut TestWorld, inbound: Inbound) -> Vec<(Audience, OutputEvent)> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(inbound, &mut out, &mut fx);
    out.take()
}

#[tokio::test]
async fn content_studio_ipc_roundtrip_over_split_gateway_link() {
    let (endpoint, data_dir) = boot_ipc_server("ipc-roundtrip").await;
    let (reader, mut writer) = mush_ipc::connect(&endpoint).await.expect("connect");
    let mut frames = FrameReader::new(reader);

    assert!(matches!(
        frames.read::<ToGateway>().await.unwrap().unwrap(),
        ToGateway::Hello { .. }
    ));
    write_frame(
        &mut writer,
        &ToServer::Hello {
            proto: IPC_VERSION,
            min_proto: IPC_MIN_PROTO,
        },
    )
    .await
    .unwrap();

    let conn = 77_u64;
    write_frame(
        &mut writer,
        &ToServer::Inbound(WireInbound::ConnUp {
            conn,
            actor: GOD,
            caps_structured: true,
            from_linkdead: false,
        }),
    )
    .await
    .unwrap();

    // 1. Catalog over the split IPC boundary
    let catalog_resp = invoke_over_ipc(
        &mut frames,
        &mut writer,
        conn,
        1,
        ContentStudioRequest::Catalog {},
    )
    .await;
    let ContentStudioResponse::Catalog { catalog } = catalog_resp else {
        panic!("expected Catalog response, got {catalog_resp:?}");
    };
    let ports = catalog["ports"].as_array().expect("ports array");
    let npcs = catalog["npcs"].as_array().expect("npcs array");
    let goods = catalog["goods"].as_array().expect("goods array");
    // Doc 32 cut S.1: twelve with Urvara Reach.
    assert_eq!(ports.len(), 12, "expected 12 real corridor ports");
    assert_eq!(npcs.len(), 57, "expected 57 total seeded NPCs");
    assert_eq!(goods.len(), 12, "expected 12 goods from goods.json");

    // 2. Read tacsim.crew.ceres over the split IPC boundary
    let read_resp = invoke_over_ipc(
        &mut frames,
        &mut writer,
        conn,
        2,
        ContentStudioRequest::Read {
            id: "tacsim.crew.ceres".to_string(),
        },
    )
    .await;
    let ContentStudioResponse::Document {
        id,
        latched_version,
        mut content,
        ..
    } = read_resp
    else {
        panic!("expected Document response, got {read_resp:?}");
    };
    assert_eq!(id, "tacsim.crew.ceres");
    assert_eq!(latched_version, Some(2));

    // 3. Assist over the split IPC boundary
    let assist_resp = invoke_over_ipc(
        &mut frames,
        &mut writer,
        conn,
        3,
        ContentStudioRequest::Assist {
            family: "npc".to_string(),
            scope: "speech_style".to_string(),
            brief: "Ceres dockside maintenance specialist".to_string(),
            context: json!({
                "port": "ceres_port"
            }),
        },
    )
    .await;
    let ContentStudioResponse::Assisted { patch, .. } = assist_resp else {
        panic!("expected Assisted response, got {assist_resp:?}");
    };
    let assisted_speech = patch["speech_style"]
        .as_str()
        .expect("assisted speech_style")
        .to_string();
    assert!(!assisted_speech.is_empty());

    // 4. Bump version to 3, relocate npc.ilya_mendes to room.c15_maintenance, and Check
    content["manifest"]["version"] = json!(3);
    content["entries"]["npc.ilya_mendes"]["def"]["room"] = json!("room.c15_maintenance");
    content["entries"]["npc.ilya_mendes"]["def"]["profile"]["speech_style"] =
        Value::String(assisted_speech.clone());

    let check_resp = invoke_over_ipc(
        &mut frames,
        &mut writer,
        conn,
        4,
        ContentStudioRequest::Check {
            id: "tacsim.crew.ceres".to_string(),
            content,
            live_check: true,
        },
    )
    .await;
    let ContentStudioResponse::Checked {
        refusals,
        pack: Some(signed_doc),
        digest: Some(digest_hex),
        ..
    } = check_resp
    else {
        panic!("expected clean Checked response, got {check_resp:?}");
    };
    assert!(refusals.is_empty(), "unexpected refusals: {refusals:?}");
    assert_eq!(digest_hex.len(), 64);

    // 5. Publish with relocate_npcs over the split IPC boundary
    let pub_resp = invoke_over_ipc(
        &mut frames,
        &mut writer,
        conn,
        5,
        ContentStudioRequest::Publish {
            id: "tacsim.crew.ceres".to_string(),
            content: signed_doc,
            relocate_npcs: vec!["npc.ilya_mendes".to_string()],
        },
    )
    .await;
    let ContentStudioResponse::Published {
        version,
        refusals: pub_refusals,
        summary,
        ..
    } = pub_resp
    else {
        panic!("expected Published response, got {pub_resp:?}");
    };
    assert!(
        pub_refusals.is_empty(),
        "publish refusals: {pub_refusals:?}"
    );
    assert_eq!(version, 3);
    assert!(
        summary.contains("1 relocated"),
        "expected summary to report NPC relocation, got: {summary}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[tokio::test]
async fn content_studio_publish_and_npc_relocation_survive_restart() {
    let cfg = test_config("restart-persistence");
    let data_dir = cfg.server.data_dir.clone();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let spool = QuarantineSpool::open(cfg.server.data_dir.join("quarantine")).unwrap();

    let engine = boot::build_engine(&cfg, &store, &spool)
        .await
        .unwrap()
        .engine;
    let mut tw = TestWorld::from_engine(engine);
    let mut serial = 1_u64;

    // Flush initial boot state
    {
        let dirty = tw.world_mut().take_dirty();
        let mut batch = persist::batch_from_dirty(tw.world(), dirty, serial, true);
        batch.module_ops = tw.take_module_ops();
        store.write(batch).await.unwrap();
        serial += 1;
    }

    let conn = 99_u64;
    let _ = step_inbound(
        &mut tw,
        Inbound::ConnUp {
            conn,
            actor: GOD,
            caps_structured: true,
            from_linkdead: false,
        },
    );

    // Read tacsim.crew.ceres
    let events = step_inbound(
        &mut tw,
        Inbound::Invoke {
            conn,
            actor: GOD,
            id: 10,
            request: InvokeRequest::ContentStudio(ContentStudioRequest::Read {
                id: "tacsim.crew.ceres".to_string(),
            }),
        },
    );
    let mut doc = events
        .into_iter()
        .find_map(|(_, ev)| match ev {
            OutputEvent::InvokeResult {
                id: 10,
                result:
                    InvokeResult::ContentStudio(ContentStudioResponse::Document { content, .. }),
            } => Some(content),
            _ => None,
        })
        .expect("Read tacsim.crew.ceres");

    // Bump version to 2 and move npc.ilya_mendes from room.c10_slack_line to room.c15_maintenance
    doc["manifest"]["version"] = json!(3);
    doc["entries"]["npc.ilya_mendes"]["def"]["room"] = json!("room.c15_maintenance");
    doc["entries"]["npc.ilya_mendes"]["def"]["profile"]["speech_style"] =
        json!("Short, clipped maintenance shorthand.");

    let pub_events = step_inbound(
        &mut tw,
        Inbound::Invoke {
            conn,
            actor: GOD,
            id: 11,
            request: InvokeRequest::ContentStudio(ContentStudioRequest::Publish {
                id: "tacsim.crew.ceres".to_string(),
                content: doc,
                relocate_npcs: vec!["npc.ilya_mendes".to_string()],
            }),
        },
    );
    let pub_summary = pub_events
        .into_iter()
        .find_map(|(_, ev)| match ev {
            OutputEvent::InvokeResult {
                id: 11,
                result:
                    InvokeResult::ContentStudio(ContentStudioResponse::Published {
                        version,
                        refusals,
                        summary,
                        ..
                    }),
            } => {
                assert!(refusals.is_empty(), "publish refusals: {refusals:?}");
                assert_eq!(version, 3);
                Some(summary)
            }
            _ => None,
        })
        .expect("Publish tacsim.crew.ceres@3");
    assert!(pub_summary.contains("1 relocated"), "{pub_summary}");

    // Flush to persistent store
    {
        let dirty = tw.world_mut().take_dirty();
        let mut batch = persist::batch_from_dirty(tw.world(), dirty, serial, true);
        batch.module_ops = tw.take_module_ops();
        store.write(batch).await.unwrap();
    }

    // Restart from store
    let restarted = boot::build_engine(&cfg, &store, &spool)
        .await
        .expect("restart build_engine")
        .engine;
    assert!(
        restarted.world.refused_namespace_writes().is_empty(),
        "{:?}",
        restarted.world.refused_namespace_writes()
    );

    let cs = restarted
        .world
        .module_state::<mush_content::ContentState>()
        .expect("ContentState");
    let crew_cid = CanonicalId::new("tacsim.crew.ceres").unwrap();
    let applied = cs.latches.applied(&crew_cid).expect("crew latch");
    assert_eq!(
        applied.version(),
        3,
        "latched version 3 must survive restart without downgrade"
    );

    let ilya_cid = CanonicalId::new("npc.ilya_mendes").unwrap();
    let ilya_obj = applied
        .entries()
        .get(&ilya_cid)
        .and_then(mush_content::AppliedEntry::object)
        .expect("npc.ilya_mendes object");
    let ceres_cid = CanonicalId::new("neighborhood.ceres_freeport").unwrap();
    let maint_cid = CanonicalId::new("room.c15_maintenance").unwrap();
    let maint_room = cs
        .latches
        .applied(&ceres_cid)
        .and_then(|l| l.entries().get(&maint_cid))
        .and_then(mush_content::AppliedEntry::object)
        .expect("room.c15_maintenance object");

    assert_eq!(
        restarted.world.get(ilya_obj).unwrap().location,
        Some(maint_room),
        "relocated NPC room must survive restart"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}
