//! Doc 12 §5.6, J4 — the narrow fetch, end to end through the production
//! boot driver (`mush_server::modules::boot_modules`) over the in-memory
//! store: the economy's sweep writes a checkpoint and its boot manifest; the
//! next boot reads the manifest, fetches nothing below its floor, and resumes
//! from the checkpoint; when the checkpoint no longer holds, the module asks
//! for the whole namespace, the loader clears the manifest and delivers whole,
//! and the boot is a full replay — after which the next checkpoint writes a
//! fresh manifest and the boot after it is narrowed again.

use mush_core::{Audience, BOOT_MANIFEST_KEY, BootManifest, GOD};
use mush_economy::{AccountId, EconomyStore};
use mush_proto::OutputEvent;
use mush_server::modules::boot_modules;
use mush_store::{InMemoryStore, ModuleOp, ModuleWrite, ObjectStore, WriteBatch};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "economy";

fn econ_world() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new().with_config(mush_economy::EconomyConfig {
                tx_tail_max: 3,
                checkpoint_every_entries: 4,
                ..mush_economy::EconomyConfig::default()
            }),
        ),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}

fn bal(tw: &TestWorld, id: u64) -> i64 {
    store(tw)
        .snapshot()
        .account(AccountId(id))
        .expect("account")
        .posted_balance
        .minor()
}

fn mint_n(tw: &mut TestWorld, n: usize) {
    for i in 0..n {
        let ev = tw.run(GOD, "@econ/mint 0=1 1");
        assert!(!has_error(&ev), "mint {i}: {}", texts(&ev));
    }
}

fn sweep_at(tw: &mut TestWorld, t: i64) {
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

/// Persist every module op the world queued since the last drain.
async fn persist(tw: &mut TestWorld, db: &InMemoryStore, serial: u64) {
    let module_ops = tw.take_module_ops();
    db.write(WriteBatch {
        serial,
        durable: true,
        module_ops,
        ..WriteBatch::default()
    })
    .await
    .expect("the batch writes");
}

async fn manifest_floor(db: &InMemoryStore) -> Option<u64> {
    let doc = db
        .module_json_get(NS, BOOT_MANIFEST_KEY)
        .await
        .expect("get")?;
    let manifest: BootManifest = serde_json::from_value(doc).expect("a manifest");
    manifest.skip_log_chunks_below.get("journal").copied()
}

#[tokio::test]
async fn j4_the_driver_boots_narrowed_from_the_manifest_and_recovers_whole_when_it_must() {
    let db = InMemoryStore::new();
    let mut tw = econ_world();
    let ev = tw.run(GOD, "@instrument/register gold=2");
    assert!(!has_error(&ev), "instrument: {}", texts(&ev));
    let ev = tw.run(GOD, "@account/system faucet=gold");
    assert!(!has_error(&ev), "faucet: {}", texts(&ev));
    let p = tw.spawn_player("Pat");
    let ev = tw.run(GOD, &format!("@account/open #{}=gold", p.0));
    assert!(!has_error(&ev), "account: {}", texts(&ev));
    mint_n(&mut tw, 121); // seq 0..=120
    sweep_at(&mut tw, 100);
    let cp = store(&tw)
        .checkpoint()
        .cloned()
        .expect("a checkpoint at the cadence");
    assert_eq!(cp.covered_seq, 120);
    persist(&mut tw, &db, 1).await;
    let floor = manifest_floor(&db)
        .await
        .expect("the manifest was written beside the head");
    assert!(floor >= 1, "the journal reached a second chunk: {floor}");

    // 1. A narrowed boot through the driver.
    let mut narrowed = econ_world();
    boot_modules(narrowed.engine_mut(), &db)
        .await
        .expect("boot");
    {
        let s = store(&narrowed);
        assert!(!s.is_frozen());
        let r = s.boot_replay().expect("booted");
        assert_eq!(
            (r.from_checkpoint, r.replayed, r.entries),
            (Some(120), 0, 121)
        );
        assert_eq!((s.tail_len(), s.tail_floor_seq()), (3, Some(118)));
        assert_eq!(bal(&narrowed, 1), 121);
    }

    // 2. The checkpoint no longer holds: the module asks for the whole
    //    namespace, the loader clears the manifest and delivers whole, and
    //    the boot is a full replay with the money right.
    let head_key = cp.head_key.clone();
    let mut head = db
        .module_json_get(NS, &head_key)
        .await
        .expect("get")
        .expect("the head is stored");
    head["state_hash"] = serde_json::json!("00");
    db.write(WriteBatch {
        serial: 2,
        durable: true,
        module_ops: vec![ModuleOp {
            ns: SmolStr::new(NS),
            op: ModuleWrite::JsonSet {
                key: head_key.clone(),
                value: head,
            },
        }],
        ..WriteBatch::default()
    })
    .await
    .expect("the tamper writes");
    let mut recovered = econ_world();
    boot_modules(recovered.engine_mut(), &db)
        .await
        .expect("the retry boots");
    {
        let s = store(&recovered);
        assert!(!s.is_frozen());
        let r = s.boot_replay().expect("booted");
        assert_eq!((r.from_checkpoint, r.replayed, r.entries), (None, 121, 121));
        assert_eq!(bal(&recovered, 1), 121);
    }
    assert_eq!(
        manifest_floor(&db).await,
        None,
        "the loader cleared the manifest that stood for nothing"
    );

    // 3. The recovered world's next checkpoint writes a fresh manifest, and
    //    the boot after it is narrowed again.
    mint_n(&mut recovered, 4); // seq 121..=124: the cadence
    sweep_at(&mut recovered, 200);
    assert_eq!(
        store(&recovered).checkpoint().map(|c| c.covered_seq),
        Some(124),
        "a new head past the rejected one"
    );
    persist(&mut recovered, &db, 3).await;
    let floor = manifest_floor(&db).await.expect("a fresh manifest");
    assert!(floor >= 1);
    let mut again = econ_world();
    boot_modules(again.engine_mut(), &db).await.expect("boot");
    {
        let s = store(&again);
        assert!(!s.is_frozen());
        let r = s.boot_replay().expect("booted");
        assert_eq!(
            (r.from_checkpoint, r.replayed, r.entries),
            (Some(124), 0, 125)
        );
        assert_eq!(bal(&again, 1), 125);
    }
}
