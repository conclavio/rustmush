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
//! Cases (1)-(4) boot [`mushd::boot::boot_all_in_one`] — the same root
//! `main.rs` calls — over the memory backend and a scratch data dir, and each
//! one fails on the pre-WS04-S0 root.
//!
//! WS04-S3 then deleted the transcription those four cases were written
//! against: `boot_all_in_one` is now three calls into `mush_server::boot`, the
//! shared root the split half runs. The four cases are unchanged apart from
//! where they read the booted world from, which is the point — they are the
//! proof that the deletion preserved the deployed path, and they prove it by
//! executing it rather than by reading its source. Case (6) carries the one
//! claim they cannot: that the delegation is a delegation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use mush_core::{AccountId, ObjId};
use mush_server::boot::{BootedEngine, OpenedStore};
use mushd::boot::boot_all_in_one;
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
async fn boot(config: &Config) -> Result<(OpenedStore, BootedEngine), String> {
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
    let (_opened, booted) = boot(&config).await.expect("the memory-backend boot");

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
    let (opened, booted) = boot(&config).await.expect("the memory-backend boot");

    let accounts = opened.store.list_accounts().await.expect("stored accounts");
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
    let (_opened, booted) = boot(&config).await.expect("the memory-backend boot");

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

/// (5) `--check-config` refuses a typo'd boot-fatal value, in the real binary,
/// on its real exit code — the preflight WS04-S3 exists to give an operator.
///
/// `receipt_policy` is the value it is written against because it is the one
/// that used to be accepted in silence: before WS04-S0 the all-in-one never
/// read the key at all, so a misspelling ran the box on the weaker `applied`
/// receipts. Now it refuses the boot, which is right — and this is how an
/// operator finds that out without the game going down to tell them.
#[test]
fn check_config_rejects_a_typod_receipt_policy() {
    let dir = scratch_dir("check-config");
    let bad = dir.join("bad.toml");
    std::fs::write(
        &bad,
        "[store]\nstore = \"memory\"\nreceipt_policy = \"hostpowerloss\"\n",
    )
    .unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mushd"))
        .args(["--check-config", "--config"])
        .arg(&bad)
        .output()
        .expect("running mushd --check-config");
    assert!(
        !out.status.success(),
        "a misspelt receipt policy must exit non-zero; got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("hostpowerloss"),
        "the refusal must name the offending value; got {stderr:?}"
    );

    // The shipped example passes too. Every deployed instance now renders the
    // shared template; provisioning_smoke covers that render over all binaries.
    let deployed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mushd.toml.example");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mushd"))
        .args(["--check-config", "--config"])
        .arg(&deployed)
        .output()
        .expect("running mushd --check-config");
    assert!(
        out.status.success(),
        "{} must preflight clean; stderr: {:?}",
        deployed.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// (6) The structural claim the four boot cases cannot carry: `mushd` DELEGATES
/// its composition root rather than transcribing it.
///
/// Two transcriptions of one world boot do not disagree loudly — they disagree
/// by omission, which is how the all-in-one came to run a world with no
/// decision key ring, an unwired account oracle and an ignored pulse cadence
/// while the split half ran one with all three. This tripwire is aimed at
/// `boot.rs` because that is where the second transcription lived: every needle
/// below is a step of the world boot that must be reached through
/// `mush_server::boot` and never re-stated here.
///
/// It carries ONLY the structural claim. Every behavioural claim is (1)-(5),
/// which execute the root.
#[test]
fn mushd_delegates_the_composition_root() {
    const ROOT: &str = include_str!("../src/boot.rs");
    for needle in [
        "compose_modules(",
        "server_modules_with(",
        "tacsim_module(",
        "LedgerArchive::open(",
        "set_offer_keys(",
        "RedisStore::connect_read_only(",
        "spawn_archive_job(",
        "panel_feeds()",
        "run_migrations(",
        "load_or_seed_world(",
        "hydrate_world_timeline(",
        "boot_modules(",
    ] {
        assert!(
            !ROOT.contains(needle),
            "crates/mushd/src/boot.rs re-states `{needle}`: the all-in-one is \
             transcribing the world boot again instead of calling the shared root, \
             and the two will drift by omission exactly as they did before WS04-S0"
        );
    }
    assert!(
        ROOT.contains("mush_server::boot::build_engine")
            && ROOT.contains("mush_server::boot::open_store"),
        "the all-in-one root must reach the world through mush_server::boot"
    );
}
