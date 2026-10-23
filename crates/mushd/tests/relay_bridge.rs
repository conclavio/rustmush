//! R6b: the real bridge, world loop, commit writer and observer DM renderer.
//! Only the external Discord REST API is controlled; no network credentials.
use mush_core::{GOD, Inbound, ObjId};
use mush_gateway::{accounts::Accounts, controls::GatewayControls, discord_bridge::DiscordBridge};
use mush_net_discord::{
    DiscordConfig, DiscordHooks, FollowOutcome, Interaction, InteractionReply, LineOutcome,
    RelayLineOutcome, RelayMap,
    testing::{Call, FakeApi},
};
use mush_store::{AccountRec, CharacterRef, DiscordLinkRec, InMemoryStore, ObjectStore};
use mushd::{
    config::Config,
    ipguard::IpGuard,
    world_task::{self, ServerDeps},
};
use std::{sync::Arc, time::Duration};

async fn account(store: &Arc<dyn ObjectStore>, name: &str, actor: ObjId, user: u64) {
    store
        .create_account(AccountRec {
            id: None,
            name: name.into(),
            argon2_hash: "test-not-used".into(),
            created: 1,
            characters: vec![CharacterRef {
                id: actor,
                name: name.into(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .unwrap();
    assert!(store.discord_user_reserve(user, name).await.unwrap());
    assert!(
        store
            .set_account_discord(
                name,
                Some(&DiscordLinkRec {
                    user_id: user,
                    linked_at: 1,
                    relay_character: None,
                    auto_follow: false,
                    dm_channel: None,
                    quiet: false
                })
            )
            .await
            .unwrap()
    );
}
fn dm_contains(api: &FakeApi, channel: u64, text: &str) -> bool {
    api.calls().iter().any(|c|matches!(c,Call::Send{channel_id,msg} if *channel_id==channel&&msg.content.contains(text)))
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r6b_real_bridge_delivers_only_to_arrived_observers_and_deduplicates_provider_retries() {
    tokio::time::timeout(Duration::from_secs(90),async {
        let dir=std::env::temp_dir().join(format!("mush-r6b-bridge-{}",std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg=Config::default();cfg.store.store="memory".into();cfg.server.data_dir=dir.clone();cfg.tacsim.enabled=true;
        cfg.tacsim.geology.key_dir=Some(dir.join("geology-keys"));
        cfg.server.help_dir=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/help");
        let store:Arc<dyn ObjectStore>=Arc::new(InMemoryStore::new());
        let spool=mush_server::quarantine::QuarantineSpool::in_memory();
        let boot=mush_server::boot::build_engine(&cfg,&store,&spool).await.unwrap();
        let mut tw=mush_testkit::TestWorld::from_engine(boot.engine);
        let bob=tw.spawn_player("BridgeBob");
        // Use the authored relay's actual coverage, not a fabricated distance.
        let luna=tw.world().module_state::<mush_sim::relay::RelayStore>().unwrap().registry().unwrap().nodes[&mush_core::CanonicalId::new("malapert").unwrap()].coverage[0];
        let ceres=tw.world().module_state::<mush_sim::relay::RelayStore>().unwrap().registry().unwrap().nodes[&mush_core::CanonicalId::new("ceres").unwrap()].coverage[0];
        for line in [format!("@teleport me={luna}"),format!("@teleport {bob}={ceres}"),"@tacsim/time rate=24".into(),"@channel/create Radio".into()] {tw.run(GOD,&line);}
        for flag in ["ic","logged","persist","relay"]{tw.run(GOD,&format!("@channel/set Radio={flag}"));}
        tw.run(GOD,"addcom radio=Radio");tw.run(bob,"addcom radio=Radio");tw.run(GOD,"@channel/set Public=relay");
        let channel=tw.world().module_state::<mush_comsys::ComsysState>().unwrap().channel_by_name("Radio").unwrap().id;
        let dirty=tw.world_mut().take_dirty();
        let mut batch=mush_server::persist::batch_from_dirty(tw.world(),dirty,1,true);batch.module_ops=tw.take_module_ops();store.write(batch).await.unwrap();
        account(&store,"BridgeGod",GOD,42).await;account(&store,"BridgeBob",bob,43).await;
        let boot=mush_server::boot::build_engine(&cfg,&store,&spool).await.unwrap();
        let sessions=Arc::new(mush_session::SessionManager::with_config(Default::default()));
        let guard=Arc::new(IpGuard::new());let controls=Arc::new(GatewayControls::default());
        let deps=ServerDeps::production(&boot,store.clone(),guard.clone(),Arc::new(mush_config::metrics::Metrics::new()),spool,vec![]);
        let handles=world_task::spawn_persist_with_decision_workers(boot.engine,sessions.clone(),deps,mush_server::decision_dispatch::DEFAULT_DECISION_PROVIDER_WORKERS,controls.clone()).unwrap();
        let api=FakeApi::new();
        let config=DiscordConfig{dm_coalesce_ms:50,relay:vec![RelayMap{channel:"Radio".into(),discord_channel_id:4242,..Default::default()},RelayMap{channel:"Public".into(),discord_channel_id:4243,..Default::default()}],..Default::default()};
        let accounts=Arc::new(Accounts::new(store.clone(),handles.handle.clone(),3));
        let bridge=Arc::new(DiscordBridge::with_link(accounts,sessions,Arc::new(handles.handle.clone()),guard,store.clone(),api.clone(),&config));bridge.install_controls(&controls);
        assert!(matches!(bridge.follow(42,None).await,FollowOutcome::Following{..}));
        eprintln!("God follow attached");
        let god_dm=loop {
            if let Some(id)=api.calls().iter().find_map(|c| match c {Call::Send{channel_id,..}=>Some(*channel_id),_=>None}) { break id; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(matches!(bridge.follow(43,None).await,FollowOutcome::Following{..}));
        eprintln!("Bob follow attached; God DM {god_dm}");
        let bob_dm=loop {
            if let Some(id)=api.calls().iter().find_map(|c| match c {Call::Send{channel_id,..} if *channel_id!=god_dm=>Some(*channel_id),_=>None}) { break id; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        eprintln!("Bob DM {bob_dm}");
        assert_eq!(bridge.relay_message(42,4242,900,"provider private line").await,RelayLineOutcome::Accepted);
        // Acceptance returned only after the writer committed the source and
        // provider identity, with no observer delivery yet.
        let doc=store.module_json_get("comsys",&format!("ic:msg:{}:1",channel.0)).await.unwrap().unwrap();
        assert_eq!(doc["msg"]["text"],"provider private line");
        assert!(!dm_contains(&api,bob_dm,"provider private line"));
        assert_eq!(bridge.relay_message(42,4242,900,"provider private line").await,RelayLineOutcome::Accepted);
        assert_eq!(store.module_json_get("comsys",&format!("chan:{}",channel.0)).await.unwrap().unwrap()["seq"],1);
        assert_eq!(bridge.dm_message(42,901,"radio private alias line").await,LineOutcome::Accepted);
        assert_eq!(bridge.dm_message(42,901,"radio private alias line").await,LineOutcome::Accepted);
        assert_eq!(store.module_json_get("comsys",&format!("chan:{}",channel.0)).await.unwrap().unwrap()["seq"],2);
        assert!(!dm_contains(&api,bob_dm,"private alias line"));
        let modal=Interaction {interaction_id:903,custom_id:"d:actm:1".into(),values:vec![],fields:vec![("line".into(),"radio private modal line".into())],channel_id:None,message_id:None};
        for _ in 0..2 { assert!(matches!(bridge.interaction(42,modal.clone()).await,InteractionReply::Ephemeral(text) if text=="Sent.")); }
        assert_eq!(store.module_json_get("comsys",&format!("chan:{}",channel.0)).await.unwrap().unwrap()["seq"],3);
        bridge.dm_message(42,902,"pub OOC bridge works").await;
        handles.handle.barrier_result().await.unwrap();
        eprintln!("IC accepted, waiting for observer arrival");
        let deadline=tokio::time::Instant::now()+Duration::from_secs(60);
        while !dm_contains(&api,bob_dm,"provider private line") || !dm_contains(&api,bob_dm,"private alias line") || !dm_contains(&api,bob_dm,"private modal line") {
            if tokio::time::Instant::now()>deadline {
                eprintln!("source {:?}",store.module_json_get("comsys",&format!("ic:msg:{}:1",channel.0)).await);
                eprintln!("calls {:?}",api.calls());
                panic!("observer delivery timed out");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let calls=api.calls();
        assert!(!calls.iter().any(|c|matches!(c,Call::WebhookSend{channel_id:4242,..}|Call::Send{channel_id:4242,..})),"shared IC mirror stays disabled");
        assert!(calls.iter().any(|c|matches!(c,Call::WebhookSend{channel_id:4243,content,..} if content.contains("OOC bridge works"))),"OOC relay still mirrors");
        let count=|text:&str|calls.iter().filter_map(|c|match c {Call::Send{channel_id,msg} if *channel_id==bob_dm=>Some(msg.content.matches(text).count()),_=>None}).sum::<usize>();
        assert_eq!(count("provider private line"),1);assert_eq!(count("private alias line"),1);assert_eq!(count("private modal line"),1);
        bridge.detach(42).await;bridge.detach(43).await;
        handles.handle.inbound(Inbound::Control(mush_core::ControlMsg::Shutdown)).await;
        handles.world_join.await.unwrap();handles.writer_join.await.unwrap();handles.outbox_join.abort();
        std::fs::remove_dir_all(&dir).unwrap();
    }).await.expect("real bridge gate completed within its bound");
}
