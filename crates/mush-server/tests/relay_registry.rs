//! R1's production boot, content and persistence boundary. Live Redis/SSH evidence
//! is recorded separately in doc 15b; these tests require no external service.
use mush_config::config::Config;
use mush_core::{CanonicalId, GOD, LIMBO, ModuleOp, ModuleWrite};
use mush_server::{boot, persist, quarantine::QuarantineSpool};
use mush_sim::relay::{
    KEY, LinkState, MAX_COVERAGE, MAX_NODES, MAX_PAIRS, RelayRegistry, RelayStore,
};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::TestWorld;
use std::{path::Path, sync::Arc};

struct Rig {
    cfg: Config,
    store: Arc<dyn ObjectStore>,
    spool: QuarantineSpool,
    tw: TestWorld,
    serial: u64,
}
impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.cfg.server.data_dir);
    }
}
impl Rig {
    async fn new(enabled: bool) -> Self {
        let mut cfg = Config::default();
        cfg.store.store = "memory".into();
        cfg.server.data_dir = std::env::temp_dir().join(format!(
            "mush-r1-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        cfg.server.help_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/help");
        cfg.tacsim.enabled = enabled;
        cfg.tacsim.geology.key_dir = Some(cfg.server.data_dir.join("geology-keys"));
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        let spool = QuarantineSpool::open(cfg.server.data_dir.join("quarantine")).unwrap();
        let engine = boot::build_engine(&cfg, &store, &spool)
            .await
            .unwrap()
            .engine;
        let mut rig = Self {
            cfg,
            store,
            spool,
            tw: TestWorld::from_engine(engine),
            serial: 1,
        };
        rig.flush().await;
        rig
    }
    fn registry(&self) -> RelayRegistry {
        self.tw
            .world()
            .module_state::<RelayStore>()
            .unwrap()
            .registry()
            .unwrap()
            .clone()
    }
    fn run(&mut self, actor: mush_core::ObjId, command: &str) -> String {
        let events = self.tw.run(actor, command);
        self.tw.seen_by(&events, actor).join("\n")
    }
    async fn flush(&mut self) {
        let dirty = self.tw.world_mut().take_dirty();
        let mut batch = persist::batch_from_dirty(self.tw.world(), dirty, self.serial, true);
        batch.module_ops = self.tw.take_module_ops();
        self.store.write(batch).await.unwrap();
        self.serial += 1;
    }
    async fn restart(&mut self) -> Result<(), String> {
        let engine = boot::build_engine(&self.cfg, &self.store, &self.spool)
            .await
            .map_err(|e| e.to_string())?
            .engine;
        self.tw = TestWorld::from_engine(engine);
        Ok(())
    }
    async fn replace_document(&mut self, value: serde_json::Value) {
        self.store
            .write(WriteBatch {
                serial: self.serial,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "sim".into(),
                    op: ModuleWrite::JsonSet {
                        key: KEY.into(),
                        value,
                    },
                }],
                ..WriteBatch::default()
            })
            .await
            .unwrap();
        self.serial += 1;
    }
}
fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).unwrap()
}

#[tokio::test]
async fn r1_real_seed_availability_and_reapply_survive_restart() {
    let mut rig = Rig::new(true).await;
    let original = rig.registry();
    assert_eq!((original.nodes.len(), original.pairs.len()), (2, 1));
    assert_eq!(original.pairs[&cid("luna_ceres")].state(), LinkState::Live);
    assert_eq!(original.nodes[&cid("malapert")].coverage.len(), 12);
    assert_eq!(original.nodes[&cid("ceres")].coverage.len(), 13);
    assert!(rig.run(GOD, "@relay/check").contains("Bindings OK"));
    assert!(
        rig.run(GOD, "@relay/show luna_ceres")
            .contains("luna_ceres.ceres")
    );
    assert!(
        rig.run(GOD, "@relay/disable luna_ceres.luna")
            .contains("revision 2")
    );
    assert_eq!(
        rig.registry().pairs[&cid("luna_ceres")].state(),
        LinkState::Degraded
    );
    assert!(
        rig.run(GOD, "@relay/disable luna_ceres.luna")
            .contains("no change")
    );
    rig.flush().await;
    let disabled = rig.registry();
    rig.restart().await.unwrap();
    assert_eq!(rig.registry(), disabled);
    let reply = rig.run(GOD, "@atlas/pack apply tacsim.relay.luna_ceres");
    assert!(!reply.contains("refused"), "{reply}");
    assert_eq!(rig.registry(), disabled);
    assert!(
        rig.run(GOD, "@relay/enable luna_ceres.luna")
            .contains("revision 3")
    );
    assert!(
        rig.run(GOD, "@relay/power luna_ceres.ceres=off")
            .contains("revision 4")
    );
    assert!(rig.run(GOD, "@relay/check").contains("Degraded"));
    assert!(
        rig.run(GOD, "@relay/power luna_ceres.ceres=on")
            .contains("revision 5")
    );
    assert!(
        rig.run(GOD, "@relay/staff luna_ceres.ceres=off")
            .contains("revision 6")
    );
    assert!(rig.run(GOD, "@relay/check").contains("Degraded"));
    assert!(
        rig.run(GOD, "@relay/staff luna_ceres.ceres=on")
            .contains("revision 7")
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    let mut restored = rig.registry();
    assert_eq!(restored.pairs[&cid("luna_ceres")].state(), LinkState::Live);
    restored.revision = original.revision;
    assert_eq!(restored, original);
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}

#[tokio::test]
async fn r1_existing_world_acquires_seed_once() {
    let mut rig = Rig::new(false).await;
    let old_ids: Vec<_> = rig.tw.world().iter().map(|o| o.id).collect();
    assert!(
        rig.tw
            .world()
            .module_state::<RelayStore>()
            .unwrap()
            .registry()
            .is_none()
    );
    rig.cfg.tacsim.enabled = true;
    rig.restart().await.unwrap();
    let original = rig.registry();
    for id in old_ids {
        assert!(rig.tw.world().get(id).is_some());
    }
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.registry(), original);
}

#[tokio::test]
async fn r1_permissions_and_installed_object_lifecycle() {
    let mut rig = Rig::new(true).await;
    let player = rig.tw.spawn_player("RelayVisitor");
    let original = rig.registry();
    let denied = rig.run(player, "@relay/show luna_ceres");
    assert!(denied.contains("Permission denied"), "{denied}");
    assert_eq!(denied, rig.run(player, "@relay/show nonexistent"));
    assert_eq!(denied, rig.run(player, "@relay/disable luna_ceres.luna"));
    assert_eq!(rig.registry(), original);
    let half = original.pairs[&cid("luna_ceres")].halves[0].object;
    let reply = rig.run(GOD, &format!("@teleport {half}={LIMBO}"));
    assert!(reply.contains("cannot be moved"), "{reply}");
    let reply = rig.run(GOD, &format!("@behavior/remove {half}=sim.relay_half"));
    assert!(reply.contains("still referenced"), "{reply}");
    let reply = rig.run(GOD, &format!("@destroy {half}"));
    assert!(
        reply.contains("refus") || reply.contains("behavior"),
        "{reply}"
    );
    original.reconcile(rig.tw.world()).unwrap();
}

#[tokio::test]
async fn r1_bad_documents_are_held_not_reseeded() {
    let mut rig = Rig::new(true).await;
    let original = rig.registry();
    let good = serde_json::to_value(&original).unwrap();
    let mut bad_cases = Vec::new();
    bad_cases.push(serde_json::json!({"version":999}));
    let mut duplicate = good.clone();
    duplicate["pairs"]["luna_ceres"]["halves"][1]["id"] =
        duplicate["pairs"]["luna_ceres"]["halves"][0]["id"].clone();
    bad_cases.push(duplicate);
    let mut conflict = good.clone();
    conflict["pairs"]["luna_ceres"]["halves"][0]["object"] =
        good["nodes"]["malapert"]["object"].clone();
    bad_cases.push(conflict);
    let mut location = good.clone();
    location["nodes"]["malapert"]["place"] = serde_json::json!(LIMBO.0);
    location["nodes"]["malapert"]["coverage"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(LIMBO.0));
    bad_cases.push(location);
    let mut huge = good.clone();
    huge["nodes"]["malapert"]["label"] = serde_json::json!("x".repeat(mush_sim::relay::MAX_BYTES));
    bad_cases.push(huge);
    for bad in bad_cases {
        rig.replace_document(bad.clone()).await;
        let error = rig.restart().await.unwrap_err();
        assert!(error.contains("relay:registry"), "{error}");
        assert_eq!(
            rig.store.module_json_get("sim", KEY).await.unwrap(),
            Some(bad)
        );
        rig.replace_document(good.clone()).await;
        rig.restart().await.unwrap();
        assert_eq!(rig.registry(), original);
    }
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            module_ops: vec![ModuleOp {
                ns: "sim".into(),
                op: ModuleWrite::Del { key: KEY.into() },
            }],
            ..WriteBatch::default()
        })
        .await
        .unwrap();
    let error = rig.restart().await.unwrap_err();
    assert!(error.contains("unrecorded binding"), "{error}");
    assert!(
        rig.store
            .module_json_get("sim", KEY)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn r1_bounds_and_terminal_state_are_enforced() {
    let rig = Rig::new(true).await;
    let original = rig.registry();
    for bound in ["nodes", "pairs", "coverage"] {
        let mut bad = original.clone();
        match bound {
            "nodes" => {
                for i in 0..MAX_NODES {
                    bad.nodes.insert(
                        cid(&format!("extra{i}")),
                        original.nodes[&cid("ceres")].clone(),
                    );
                }
            }
            "pairs" => {
                for i in 0..MAX_PAIRS {
                    bad.pairs.insert(
                        cid(&format!("extra{i}")),
                        original.pairs[&cid("luna_ceres")].clone(),
                    );
                }
            }
            _ => bad.nodes.get_mut(&cid("ceres")).unwrap().coverage = vec![LIMBO; MAX_COVERAGE + 1],
        }
        assert!(bad.validate().is_err(), "{bound}");
    }
    let mut pair = original.pairs[&cid("luna_ceres")].clone();
    pair.halves[0].destroyed = true;
    assert_eq!(pair.state(), LinkState::Dead);
    pair.halves[1].powered = false;
    assert_eq!(pair.state(), LinkState::Dead);
}
