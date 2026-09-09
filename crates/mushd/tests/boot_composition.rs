//! WS04-S0: the all-in-one composition root builds what the forked server half
//! builds.
//!
//! `mushd` is the binary the integration suite drives and the box deploys, but
//! its composition root had drifted away from `mush-server`'s `build_engine` in
//! four places — the durable decision key ring, the boot-hydrated account
//! oracle, the `[tacsim]` pulse cadence, and a validated
//! `[store].receipt_policy`. Nothing failed loudly: the world booted with an
//! unkeyed decision runtime, an unwired oracle every consumer then failed
//! closed against, the default pulse cadence whatever `[tacsim]` said, and a
//! misspelt receipt policy silently downgraded to `applied`.
//!
//! Every case here boots [`mushd::boot::boot_all_in_one`] — the same root
//! `main.rs` calls — over the memory backend and a scratch data dir, and each
//! one fails on the pre-WS04-S0 root.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use mush_core::{AccountId, ObjId};
use mushd::boot::{Booted, boot_all_in_one};
use mushd::config::Config;
use mushd::quarantine::QuarantineSpool;

/// A unique scratch data dir per call.
fn scratch_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mushd-boot-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The deployed defaults, pointed at a throwaway store and data dir.
fn memory_config(dir: &Path) -> Config {
    let mut config = Config::default();
    config.server.data_dir = dir.to_path_buf();
    config.store.store = "memory".to_owned();
    config
}

/// Boot exactly what `main.rs` boots.
async fn boot(config: &Config) -> Result<Booted, String> {
    let mut spool = QuarantineSpool::open(config.server.data_dir.join("quarantine"))
        .map_err(|error| format!("quarantine spool: {error}"))?;
    boot_all_in_one(config, &mut spool).await
}

/// (1) The K7 delayed-decision key ring is provisioned on disk and installed on
/// the engine. Pre-change `has_decision_keys()` was false on every all-in-one
/// boot, and `world_task.rs`'s warn branch started the H2 decision runtime
/// unkeyed.
#[tokio::test]
async fn mushd_provisions_the_decision_key_ring() {
    let dir = scratch_dir("decision-keys");
    let config = memory_config(&dir);
    let booted = boot(&config).await.expect("the memory-backend boot");

    assert!(
        booted.engine.has_decision_keys(),
        "the all-in-one root must install the decision key ring; unkeyed here means \
         the deployed binary runs the H2 decision runtime without one"
    );
    assert!(
        dir.join("decision-keys").is_dir(),
        "the key ring must be provisioned under the data dir beside offer-keys and \
         resolution-keys, at {}",
        dir.join("decision-keys").display()
    );
    assert!(dir.join("offer-keys").is_dir());
    assert!(dir.join("resolution-keys").is_dir());
}

/// (2) The account-distinctness oracle is hydrated from the store and installed
/// on the engine. Pre-change `account_of` answered `None` for every character,
/// so `+focus` and the D4 relay's `(account, actor)` re-validation failed closed
/// on the deployed binary.
#[tokio::test]
async fn mushd_installs_the_account_oracle() {
    let dir = scratch_dir("account-oracle");
    // Seeded through the one-time `accounts.json` import the root already runs,
    // which is the only way to put a record in a memory store the root opens
    // itself.
    std::fs::write(
        dir.join("accounts.json"),
        r#"{"version":1,"accounts":[{"name":"warden",
            "hash":"$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA",
            "characters":[{"id":7,"name":"Vesper"}]}]}"#,
    )
    .unwrap();
    let config = memory_config(&dir);
    let booted = boot(&config).await.expect("the memory-backend boot");

    let accounts = booted.store.list_accounts().await.expect("stored accounts");
    let record = accounts
        .iter()
        .find(|a| a.name == "warden")
        .expect("the imported account");
    let expected = record
        .id
        .or_else(|| AccountId::mint(&record.name, record.created).ok())
        .expect("a mintable account id");

    assert_eq!(
        booted.engine.account_of(ObjId(7)),
        Some(expected),
        "the all-in-one root must hydrate the account oracle; `None` here means every \
         account-distinctness policy on the deployed binary refuses"
    );
    assert_eq!(booted.engine.account_of(ObjId(8)), None);
}

/// (3) `[store].receipt_policy` is validated at boot. Pre-change the all-in-one
/// never read the key: a typo was accepted in silence and the box ran on the
/// weaker `applied` receipts, and `host_power_loss` on a memory store was
/// accepted outright.
#[tokio::test]
async fn mushd_rejects_an_invalid_receipt_policy() {
    let dir = scratch_dir("bad-receipt-policy");
    let mut config = memory_config(&dir);
    config.store.receipt_policy = "hostpowerloss".to_owned();
    let Err(error) = boot(&config).await else {
        panic!("a misspelt receipt policy must refuse the boot")
    };
    assert!(
        error.contains("hostpowerloss"),
        "the refusal must name the offending value; got {error:?}"
    );

    let dir = scratch_dir("memory-receipt-policy");
    let mut config = memory_config(&dir);
    config.store.receipt_policy = "host_power_loss".to_owned();
    let Err(error) = boot(&config).await else {
        panic!("host_power_loss on the memory backend must refuse the boot")
    };
    assert!(
        error.contains("requires the Redis backend"),
        "the refusal must say the policy needs Redis; got {error:?}"
    );
}

/// (4) `[tacsim].ground_pulse_ms` / `space_pulse_ms` reach the sim's live
/// cadence. Pre-change the all-in-one left `PulseConfig` at its composed
/// defaults whatever the config said.
#[tokio::test]
async fn mushd_installs_the_pulse_config() {
    let dir = scratch_dir("pulse-config");
    let mut config = memory_config(&dir);
    config.tacsim.ground_pulse_ms = 5_000;
    config.tacsim.space_pulse_ms = 11_000;
    let booted = boot(&config).await.expect("the memory-backend boot");

    let pulse = booted
        .engine
        .world
        .module_state::<mush_sim::tick::PulseConfig>()
        .copied()
        .expect("the sim composes the pulse slot");
    assert_eq!(
        pulse.ground_ms, 5_000,
        "the all-in-one root must upgrade the pulse cadence from [tacsim]"
    );
    assert_eq!(pulse.space_ms, 11_000);
}
