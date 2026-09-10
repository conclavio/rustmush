//! WS25-S6: a new instance is provisioned by RENDERING the shipped template,
//! and the rendered thing has to be a config a server will actually boot.
//!
//! WS25-S3 shipped `deploy/instance/mushd.toml.tmpl` and the two `instance.env`
//! parameterisations, and `crates/mush-config/tests/shipped_configs.rs` proves
//! the render is *complete* — every section declared, no placeholder left, the
//! start rooms not Limbo, the boundary not on the store's port. That is a
//! statement about the TEXT. It says nothing about whether the config that text
//! parses into is one the world half accepts, and everything WS04 added since
//! (`[tacsim]`'s pack block, `[store.archive]`'s window, `[game].ic_epoch`) is
//! boot-fatal by design: a value either binary refuses is a box that does not
//! come up, and the operator finds out from a supervisor watching a boot loop.
//!
//! WS04-S3 built the flag that answers that question without a live box —
//! `--check-config`, over `mush_config::preflight::check` (the set all three
//! binaries share) and `mush_server::boot::check_config` (that set plus the two
//! only the world half can find). This file points it at the shipped templates,
//! and then goes one step further than a preflight can: it BOOTS one rendered
//! instance and asserts the world it composed is the one the `instance.env`
//! asked for.
//!
//! The four claims:
//!
//! 1. both shipped envs render through the deploy's own rule — the `sed` loop
//!    documented at `deploy/instance/mushd.toml.tmpl:14-40` and in
//!    `deploy/README.md`, never `source` — and both rendered configs preflight
//!    clean under BOTH check sets;
//! 2. the real `mush-server` binary agrees, on its real exit code — the
//!    library functions and the process cannot drift into different verdicts;
//! 3. the negative control: a single typo'd `receipt_policy` in an otherwise
//!    good render is REFUSED, by name, at both levels. Without this the job
//!    above is a check that always says yes;
//! 4. and the rendered WoD instance boots: `open_store` opens the J5 archive
//!    tier the template enables, `spawn_ledger_archive` binds its handle rather
//!    than leaving the nightly pass unscheduled, `TACSIM_ENABLED=true` arrives
//!    as an ENABLED `[tacsim]` block on the loaded config (which is where that
//!    key is decided — see the boot case), and `RULESET=wod` comes out the
//!    other end as `wod` in the `hello` manifest the gateway sends a web
//!    client, with the TacSim module composed into it.
//!
//! # What is overridden, and why
//!
//! The shipped envs describe a real box: `DATA_DIR`/`BUILD_DIR`/`TLS_DIR` under
//! `/opt/rustmush/dev3`, and `[store].store = "redis"` on this instance's own
//! port with `receipt_policy = "applied"`. Neither exists here.
//!
//! - The PREFLIGHT cases render the shipped values verbatim. Nothing in either
//!   check set opens a path or a socket — that is the property that makes the
//!   flag safe to run beside a live instance — so the `/opt` paths are exactly
//!   as checkable here as they are on the box.
//! - The BOOT case renders with the three path keys pointed at a scratch
//!   directory (still through the same substitution: they are `__KEY__` tokens
//!   like any other) and then sets `[store].store = "memory"` on the parsed
//!   config, which `boot::open_store` allows and warns about. That is the one
//!   deviation from the shipped text, it is one line, and it is what lets this
//!   run on every push rather than only where a Redis 8 is up: the real receipt
//!   policy against a live server is the `redis-faults` job's business.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mush_config::config::Config;
use mush_config::metrics::Metrics;
use mush_server::boot;
use mush_server::quarantine::QuarantineSpool;

// ---- the render, which is the deploy's render -------------------------------

/// Both shipped parameterisations: the WoD instance that wants the TacSim
/// layer, and the Shadowrun one that does not.
const SHIPPED_ENVS: &[&str] = &["instance.env.example", "instance.env.shadowrun.example"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("crates/mush-server has a workspace root two levels up")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The part of a line the deploy's completeness check looks at: everything
/// before the first `#`. `grep -n '^[^#]*__'` in `deploy/README.md`, in the
/// template's own header, and in this step's CI job IS this function.
fn live(line: &str) -> &str {
    line.split('#').next().unwrap_or("")
}

/// Parse an `instance.env` into its `KEY=value` pairs, the way the deploy's
/// `while IFS='=' read -r k v` loop does: comments and blanks skipped, the line
/// READ rather than evaluated. `instance.env` is not a shell script — the
/// shipped `INSTANCE_TITLE=rustmush dev3 (oWoD)` is a good line here and a
/// syntax error to `.`, which is why nothing anywhere sources it.
fn parse_env(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// Substitute every `__KEY__` token with that key's value and nothing else —
/// the `s|__%s__|%s|g` sed script, in Rust.
fn substitute(template: &str, env: &[(String, String)]) -> String {
    let mut out = template.to_string();
    for (key, value) in env {
        out = out.replace(&format!("__{key}__"), value);
    }
    out
}

/// A scratch directory of this test binary's own, removed by the caller.
fn scratch_dir(tag: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mush-provisioning-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// TOML string values are escaped, so a Windows `\` in a substituted path would
/// render `data_dir = "C:\Users\..."` and refuse to parse. Forward slashes are
/// accepted by every path API on both platforms, and are what the shipped envs
/// carry anyway.
fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Render one shipped `instance.env` against the shipped template, apply the
/// deploy's completeness `grep`, write the result where a config lives, and
/// hand back `(path, text)`.
///
/// `overrides` replaces the value of a key before substitution — the boot case
/// re-points the three path keys at a scratch tree. It is applied to the ENV,
/// not to the rendered text, so the substitution under test is still the only
/// mechanism that fills the template.
fn render_instance(env_file: &str, into: &Path, overrides: &[(&str, String)]) -> (PathBuf, String) {
    let root = repo_root();
    let template = read(&root.join("deploy/instance/mushd.toml.tmpl"));
    let mut env = parse_env(&read(&root.join("deploy/instance").join(env_file)));
    for (key, value) in overrides {
        let slot = env
            .iter_mut()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("{env_file} has no {key} line to override"));
        slot.1 = value.clone();
    }
    let text = substitute(&template, &env);

    // `grep -n '^[^#]*__'`: a live line still carrying a token means a key with
    // no line in the env file, and the config it would install holds the
    // literal `__START_ROOM__`.
    let leftovers: Vec<&str> = text.lines().filter(|l| live(l).contains("__")).collect();
    assert!(
        leftovers.is_empty(),
        "{env_file} leaves placeholders unrendered: {leftovers:?}"
    );

    let path = into.join("mushd.toml");
    std::fs::write(&path, &text).expect("writing the rendered config");
    (path, text)
}

/// Load a rendered config through the real `Config::load`, which is what every
/// binary's `--config` does.
fn load(path: &Path) -> Config {
    Config::load(path).unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()))
}

// ---- 1: both shipped instances preflight clean ------------------------------

/// The rendered shipped instances are configs a server accepts — under the set
/// every binary shares AND under the world half's superset.
///
/// Both are asserted for both envs on purpose. The gateway is the process a
/// supervisor starts on the split deploy and it reads only the shared set; the
/// world half is the process that dies on `[tacsim]`, and it is the one nobody
/// starts by hand. A template that passes one and not the other is a box that
/// comes up and then loses its world.
#[test]
fn both_shipped_instances_preflight_clean_under_both_check_sets() {
    let dir = scratch_dir("preflight");
    for env_file in SHIPPED_ENVS {
        let (path, _) = render_instance(env_file, &dir, &[]);
        let cfg = load(&path);

        let shared = mush_config::preflight::check(&cfg);
        assert!(
            shared.is_empty(),
            "{env_file}: the shared preflight (mush-gateway's whole set) refuses \
             the shipped render: {shared:?}"
        );
        let world = boot::check_config(&cfg);
        assert!(
            world.is_empty(),
            "{env_file}: the world half's preflight refuses the shipped render: \
             {world:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- 2 and 3: the binary agrees, and it is not vacuous ----------------------

/// `mush-server --check-config`, as a process, over each rendered instance —
/// and then the negative control, which is the whole reason case 1 above is
/// worth running.
///
/// `receipt_policy` is the value the control is written against because it is
/// the one that used to be accepted in silence: before WS04-S0 the all-in-one
/// never read the key, so a misspelling ran the box on the weaker `applied`
/// receipts. The template writes `"applied"` deliberately; a single letter
/// dropped from it must be refused BY NAME, or `--check-config` over a rendered
/// instance is a step that can only ever pass.
///
/// Only `mush-server` runs here: `CARGO_BIN_EXE_*` covers this package alone.
/// The CI job this step adds builds all three and runs the same two invocations
/// against `mush-gateway` and `mushd` beside it.
#[test]
fn the_server_binary_preflights_the_render_and_refuses_a_typod_receipt_policy() {
    let dir = scratch_dir("binary");
    for env_file in SHIPPED_ENVS {
        let (path, text) = render_instance(env_file, &dir, &[]);

        let out = std::process::Command::new(env!("CARGO_BIN_EXE_mush-server"))
            .args(["--check-config", "--config"])
            .arg(&path)
            .output()
            .expect("running mush-server --check-config");
        assert!(
            out.status.success(),
            "{env_file}: the rendered instance must preflight clean in the real \
             binary; status {:?}, stderr {:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );

        // The negative control. One letter, in the one key whose misspelling
        // used to be silent.
        let broken = text.replace(
            "receipt_policy = \"applied\"",
            "receipt_policy = \"aplied\"",
        );
        assert_ne!(
            broken, text,
            "{env_file}: the template no longer writes [store].receipt_policy = \
             \"applied\", so this control tampers with nothing — re-point it at \
             the key that is boot-fatal now"
        );
        let broken_path = dir.join("broken.toml");
        std::fs::write(&broken_path, &broken).expect("writing the tampered config");

        // At the library level first, so a failure says which layer let it
        // through...
        let refusals = boot::check_config(&load(&broken_path));
        assert!(
            refusals.iter().any(|r| r.contains("aplied")),
            "{env_file}: the preflight must name the offending value: {refusals:?}"
        );

        // ...and then in the process, on its exit code, which is what the CI
        // job spends.
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_mush-server"))
            .args(["--check-config", "--config"])
            .arg(&broken_path)
            .output()
            .expect("running mush-server --check-config");
        assert!(
            !out.status.success(),
            "{env_file}: a typo'd receipt policy must exit non-zero; got {:?}",
            out.status
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("aplied"),
            "{env_file}: the refusal must name the offending value; got {stderr:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- 4: the rendered WoD instance boots the world it describes --------------

/// The one full boot. `instance.env.example` is the reference instance — WoD
/// with the TacSim layer on — and this asserts that the three things the
/// template decides and nothing else checks actually reach a booted world:
///
/// - `[store.archive].enabled = true` opens the J5 tier (`open_store`), which
///   is the half of the ledger's durability story config owns: with it off,
///   sealed chunks accumulate in the store forever;
/// - `[store.archive].window` schedules the nightly pass and its handle is
///   BOUND (`spawn_ledger_archive` -> `Some`), not dropped detached;
/// - `TACSIM_ENABLED=true` reaches the ENABLEDNESS of the layer, which is where
///   that key is decided: `cfg.tacsim.enabled` and the typed `[tacsim]` block
///   `config_ext::tacsim_config` parses out of it, the same pair the Shadowrun
///   case asserts the `false` side of. It is deliberately NOT read off the
///   manifest: `boot::game_modules` pushes `modules::tacsim_module(tacsim)`
///   unconditionally (boot.rs — the `if tacsim.enabled` above it only logs),
///   so `tacsim` is in the feature list of every config and an assertion there
///   would pass with the key flipped;
/// - `RULESET=wod` comes out the other end as `wod` in the `hello` manifest the
///   gateway sends a web client, with `shadowrun` absent and the WoD panel feeds
///   riding along — and the TacSim module composed into that manifest at all.
///
/// `[store].store` is set to `"memory"` after loading — see this file's header.
#[tokio::test]
async fn the_rendered_wod_instance_boots_and_advertises_its_layers() {
    let dir = scratch_dir("boot");
    let data = dir.join("data");
    std::fs::create_dir_all(&data).expect("the scratch data dir");
    let (path, _) = render_instance(
        "instance.env.example",
        &dir,
        &[
            ("DATA_DIR", toml_path(&data)),
            ("BUILD_DIR", toml_path(&dir.join("build"))),
            ("TLS_DIR", toml_path(&dir.join("tls"))),
        ],
    );

    let mut cfg = load(&path);
    assert_eq!(
        cfg.server.data_dir, data,
        "the path overrides go through the same substitution as every other key"
    );
    // `TACSIM_ENABLED=true`, asserted where the key is actually decided. The
    // manifest cannot carry this claim (see the doc above); the typed block is
    // what `game_modules` hands `modules::tacsim_module`, and its `enabled` is
    // the flag every later cut reads. (The shipped template renders
    // `pack_paths = []` — an instance turns the layer on before it has content
    // to put in it — so there is nothing to assert about the pack list here;
    // the Shadowrun case owns the `false` side of the same pair.)
    assert!(
        cfg.tacsim.enabled,
        "TACSIM_ENABLED=true must render an enabled [tacsim] block"
    );
    let tacsim = mush_server::config_ext::tacsim_config(&cfg.tacsim)
        .expect("the rendered [tacsim] block parses");
    assert!(
        tacsim.enabled,
        "the typed [tacsim] block the boot composes with must be enabled too"
    );

    // The one deviation from the shipped text: a throwaway backend, so this
    // case runs on every push. `open_store` allows it and says so in a warn.
    cfg.store.store = "memory".to_string();

    let opened = boot::open_store(&cfg)
        .await
        .expect("the rendered instance opens its store");
    assert!(
        opened.archive.is_some(),
        "[store.archive].enabled = true in the template, so the J5 tier must be \
         open and attached to the backend"
    );

    let spool = QuarantineSpool::open(cfg.server.data_dir.join("quarantine"))
        .expect("a fresh spool opens under the rendered data_dir");
    let booted = boot::build_engine(&cfg, &opened.store, &spool)
        .await
        .expect("the rendered instance boots its world");

    // The nightly pass is scheduled, and its handle comes back to be bound into
    // a shutdown tail rather than detached.
    let metrics = Arc::new(Metrics::new());
    let job = boot::spawn_ledger_archive(
        &cfg,
        &opened.store,
        opened.archive.clone(),
        booted.archive_namespaces.clone(),
        &metrics,
    )
    .expect("[store.archive].window parses, so the pass is schedulable");
    assert!(
        job.is_some(),
        "the archive job handle must be returned, not dropped: a detached pass \
         is one the shutdown tail cannot stop"
    );

    let (features, panels) = boot::build_hello_manifest(&booted.engine);
    // Not a statement about TACSIM_ENABLED — see the doc: the module composes
    // either way. It is a statement that the layer is in the roster the manifest
    // is built from at all, i.e. that this boot composed the TacSim module.
    assert!(
        features.iter().any(|f| f == "tacsim"),
        "the TacSim module must compose into the hello manifest: {features:?}"
    );
    assert!(
        features.iter().any(|f| f == "wod"),
        "RULESET=wod must reach the hello manifest: {features:?}"
    );
    assert!(
        !features.iter().any(|f| f == "shadowrun"),
        "the WoD instance must not compose the Shadowrun ruleset: {features:?}"
    );
    assert!(
        panels.iter().any(|p| p == "wod.sheet"),
        "the selected ruleset's panel feeds ride the same manifest: {panels:?}"
    );

    boot::stop_ledger_archive(job).await;
    drop(booted);
    drop(spool);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other parameterisation, without a second full boot: the Shadowrun env
/// selects its own ruleset and turns the TacSim layer OFF, and those are the
/// two keys that differ. A template only ever exercised one way has an untested
/// branch — and `[tacsim].enabled` is the one that decides whether any pack
/// file is read at all.
#[test]
fn the_shadowrun_instance_selects_its_own_ruleset_and_leaves_tacsim_off() {
    let dir = scratch_dir("shadowrun");
    let (path, _) = render_instance("instance.env.shadowrun.example", &dir, &[]);
    let cfg = load(&path);

    assert_eq!(
        cfg.rulesets.enabled,
        vec!["shadowrun".to_string()],
        "RULESET=shadowrun must select exactly the Shadowrun ruleset"
    );
    assert!(
        !cfg.tacsim.enabled,
        "TACSIM_ENABLED=false must leave the layer off"
    );
    let typed = mush_server::config_ext::tacsim_config(&cfg.tacsim)
        .expect("a disabled [tacsim] block still parses");
    assert!(!typed.enabled);
    assert!(
        typed.packs.is_empty(),
        "a disabled layer reads no pack file at all"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
