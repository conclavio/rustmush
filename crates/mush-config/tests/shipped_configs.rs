//! WS25-S3: the shipped instance template renders a WHOLE config, and the
//! shipped example cannot drift away from it.
//!
//! The failure mode these prove is the quiet one. Neither deployed box
//! (`deploy/dev1`, `deploy/dev2`) writes `[log]`, `[store.archive]`, `[offers]`,
//! `[resolution]`, `[discord]`, `[economy]`, `[weather]`, `[worldgen]`,
//! `[onboarding]`, `[npc]` or `[tacsim]` — eleven
//! sections — so each of those runs on a compiled-in default the operator never
//! saw: no log files, no ledger archive tier, key directories nobody
//! provisioned, and every character starting in Limbo `#0`. Nothing failed;
//! nothing said so. `deploy/instance/mushd.toml.tmpl` is the answer, and it is
//! only an answer while it stays complete — hence:
//!
//! - every one of the eleven is declared in the rendered file, with a value
//!   that is demonstrably *not* the default it would have taken by omission;
//! - a `__PLACEHOLDER__` with no line in `instance.env` fails here rather than
//!   installing a config with `__START_ROOM__` in it;
//! - `[ipc].listen` cannot be the port `[store].redis_url` already holds — the
//!   WS08-S13 refusal, exercised over the real template rather than a fixture;
//! - `[onboarding].start_room` is a seeded room, never `mush_core::LIMBO`;
//! - and `mushd.toml.example` declares exactly the template's section set, so a
//!   section added to one and forgotten in the other is a red test.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use mush_config::config::{Config, ConfigError};

/// The eleven sections absent from both boxes this template exists to fix.
/// `[tacsim.recovery]` rides along as the sub-block `[tacsim]` gained after
/// those boxes were written, and which `mushd.toml.example` had already drifted
/// past; `[tacsim.economy]` and the eight combat blocks of 16b cut G are later
/// additions held to the same rule. The count in the prose above is about the
/// deployed boxes' history, not the length of this list.
const REQUIRED_SECTIONS: &[&str] = &[
    "log",
    "store.archive",
    "offers",
    "resolution",
    "discord",
    "economy",
    "weather",
    "worldgen",
    "onboarding",
    "npc",
    "tacsim",
    "tacsim.recovery",
    "tacsim.economy",
    // 16b cut G — the combat arena's five blocks, three of them with their own
    // sub-tables. Doc 16 §7's Configuration paragraph requires all of them in
    // the example, the instance template and both dev deployment files.
    "tacsim.arena",
    "tacsim.arena.tempo",
    "tacsim.arena.intents",
    "tacsim.arena.ledger",
    "tacsim.match",
    "tacsim.sensors",
    "tacsim.resolution",
    "tacsim.harness",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The uncommented TOML table headers a config declares, in file order.
/// `[[array.of.tables]]` and commented-out headers are deliberately excluded:
/// they are documentation, not a section the server reads.
fn declared_sections(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('[')?;
            if rest.starts_with('[') {
                return None;
            }
            let name = rest.split_once(']')?.0;
            Some(name.trim().to_string())
        })
        .collect()
}

/// The part of a line the deploy's completeness check looks at: everything
/// before the first `#`. `grep -n '^[^#]*__'` in deploy/README.md and in the
/// template's own header IS this function; every placeholder rule below goes
/// through it so the shell and the test cannot drift apart.
fn live(line: &str) -> &str {
    line.split('#').next().unwrap_or("")
}

/// Parse an `instance.env` into its `KEY=value` pairs.
fn parse_env(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// Render the template exactly as the deploy does: every `__KEY__` token
/// replaced by that key's value from `instance.env`, and nothing else.
fn render(template: &str, env: &[(String, String)]) -> String {
    let mut out = template.to_string();
    for (key, value) in env {
        out = out.replace(&format!("__{key}__"), value);
    }
    out
}

/// Render `instance` and hand back the rendered text alongside the parsed
/// config, having first proved the render was complete.
fn rendered_instance(env_file: &str) -> (String, Config) {
    let root = repo_root();
    let template = read(&root.join("deploy/instance/mushd.toml.tmpl"));
    let env = parse_env(&read(&root.join("deploy/instance").join(env_file)));
    let text = render(&template, &env);

    // A key in the template with no line in instance.env would otherwise
    // install a config containing the literal `__START_ROOM__`, which parses as
    // a string where an integer belongs — or, worse, as a plausible-looking
    // path. Name the offenders.
    //
    // The rule is deliberately the deploy's rule, character for character:
    // `grep -n '^[^#]*__'`, i.e. a token in the part of the line before any
    // `#`. The template's own header spells the mechanism out with `__KEY__`
    // tokens, so both sides must exempt comments — and they must exempt the
    // SAME lines, or the shell declares a good render incomplete (or worse,
    // passes a bad one) while this test disagrees.
    let leftovers: Vec<&str> = text.lines().filter(|l| live(l).contains("__")).collect();
    assert!(
        leftovers.is_empty(),
        "{env_file} leaves placeholders unrendered: {leftovers:?}"
    );

    // One directory per CALL, not per process: these tests run as parallel
    // threads in a single binary and three of them render the same env file, so
    // a path keyed only on the file name would have one thread deleting the
    // config another was about to load — and `Config::load` answers a missing
    // file with `Config::default()` (config.rs, the NotFound arm), which is the
    // silent-default failure this whole step exists to close. It must never be
    // the thing that answers here.
    static RENDER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mush-tmpl-{}-{}-{}",
        env_file.replace('.', "_"),
        std::process::id(),
        RENDER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mushd.toml");
    std::fs::write(&path, &text).unwrap();
    assert!(
        path.is_file(),
        "{env_file}: the rendered config vanished before it was loaded, so \
         Config::load would have answered with compiled-in defaults"
    );
    let cfg = Config::load(&path)
        .unwrap_or_else(|e| panic!("the rendered {env_file} config does not parse: {e}"));
    // Only what this call created.
    let _ = std::fs::remove_dir_all(&dir);

    (text, cfg)
}

/// The shipped parameterisation. 16b cut R-b: one game, one env — the Shadowrun
/// env left with its ruleset, and the disabled-`[tacsim]` branch it used to
/// exercise is rendered by `mush-server`'s `provisioning_smoke` from this one env
/// with `TACSIM_ENABLED` overridden.
const SHIPPED_ENVS: &[&str] = &["instance.env.example"];

#[test]
fn the_template_declares_every_section_the_deployed_boxes_lack() {
    for env_file in SHIPPED_ENVS {
        let (text, cfg) = rendered_instance(env_file);
        let declared: BTreeSet<String> = declared_sections(&text).into_iter().collect();
        for section in REQUIRED_SECTIONS {
            assert!(
                declared.contains(*section),
                "{env_file}: [{section}] is one of the sections the deployed \
                 boxes omit; the template must declare it, not default it"
            );
        }

        // Declared is not enough: a section written with the value it would
        // have taken by omission documents nothing. Each of these differs from
        // `Config::default()`, so the assertion fails if the block is ever
        // reduced to a header.
        let defaults = Config::default();
        assert!(
            cfg.log.file,
            "{env_file}: [log].file must be on (rotated copies survive a volatile journal)"
        );
        assert_eq!(cfg.log.format, "json", "{env_file}: [log].format");
        assert_ne!(
            cfg.log.format, defaults.log.format,
            "{env_file}: [log] is the default block again"
        );
        assert!(
            cfg.store.archive.enabled,
            "{env_file}: [store.archive] must be ENABLED — WS04-S1/S2's \
             LedgerArchive tier is opened by this key and nothing else"
        );
        assert!(
            !defaults.store.archive.enabled,
            "the archive default must stay off, or this assertion proves nothing"
        );

        // [offers]/[resolution] provision two DIFFERENT private key families
        // under the instance data root, rather than letting first boot create
        // them wherever the process happened to be.
        let offers = cfg.offers.key_dir.as_ref().expect("[offers].key_dir");
        let resolution = cfg
            .resolution
            .key_dir
            .as_ref()
            .expect("[resolution].key_dir");
        assert_ne!(
            offers, resolution,
            "{env_file}: the key families must not share a directory"
        );
        for dir in [offers, resolution] {
            assert!(
                dir.starts_with(&cfg.server.data_dir),
                "{env_file}: {} must live under [server].data_dir {}",
                dir.display(),
                cfg.server.data_dir.display()
            );
        }

        // The raw sections are opaque here, but "declared with content" is
        // exactly what distinguishes them from the empty-table default.
        for (name, raw) in [
            ("discord", &cfg.discord),
            ("web", &cfg.web),
            ("llm", &cfg.llm),
        ] {
            let table = raw
                .as_table()
                .unwrap_or_else(|| panic!("[{name}] is not a table"));
            assert!(
                !table.is_empty(),
                "{env_file}: [{name}] is the empty default"
            );
        }
    }
}

#[test]
fn the_template_never_points_the_boundary_at_the_store() {
    for env_file in SHIPPED_ENVS {
        let (_, cfg) = rendered_instance(env_file);

        // The shipped form is a per-instance Unix socket, which has no port to
        // collide with at all. Off Unix that path cannot even parse, and that
        // one refusal is tolerable — a collision, or a bad address, is not.
        match cfg.ipc_endpoint() {
            Ok(_) => {}
            Err(ConfigError::BadIpcListen { .. }) if !cfg!(unix) => {
                assert!(
                    cfg.ipc.listen.parse::<std::net::SocketAddr>().is_err(),
                    "{env_file}: a TCP [ipc].listen must validate on every \
                     platform, not be refused"
                );
            }
            Err(e) => panic!("{env_file}: the rendered boundary is not usable: {e}"),
        }

        // The bypass: put the boundary back on the store's own port, the way an
        // instance that never wrote an [ipc] section would have inherited it,
        // and the load path must refuse. This is the WS08-S13 refusal driven by
        // the template's OWN store URL rather than a fixture, so an instance.env
        // whose REDIS_PORT drifts onto the boundary is caught here.
        let authority = cfg
            .store
            .redis_url
            .rsplit_once("//")
            .expect("the template's redis_url is a URL")
            .1;
        let mut collided = cfg.clone();
        collided.ipc.listen = authority.to_string();
        let err = collided
            .ipc_endpoint()
            .expect_err("the boundary must never be allowed to bind the store's port");
        assert!(
            matches!(err, ConfigError::IpcStorePortCollision { .. }),
            "{env_file}: expected a port-collision refusal, got {err:?}"
        );
    }
}

#[test]
fn the_template_starts_characters_in_a_seeded_room_not_limbo() {
    for env_file in SHIPPED_ENVS {
        let (_, cfg) = rendered_instance(env_file);
        assert_ne!(
            cfg.onboarding.start_room_id(),
            mush_core::LIMBO,
            "{env_file}: [onboarding].start_room is Limbo — the bootstrap room \
             seeded beside God, reserved from housing, and not a place to open \
             a game"
        );
        assert_ne!(
            cfg.onboarding.start_room_unapproved_id(),
            mush_core::LIMBO,
            "{env_file}: [onboarding].start_room_unapproved falls back to Limbo"
        );
        assert_ne!(
            cfg.onboarding.start_room_unapproved_id(),
            cfg.onboarding.start_room_id(),
            "{env_file}: the chargen area and the live start room are the same \
             room, so the gate holds unapproved characters nowhere"
        );
    }
}

#[test]
fn the_shipped_example_and_the_deployed_template_declare_the_same_sections() {
    let root = repo_root();
    let example = declared_sections(&read(&root.join("mushd.toml.example")));
    let template = declared_sections(&read(&root.join("deploy/instance/mushd.toml.tmpl")));

    let example_set: BTreeSet<&String> = example.iter().collect();
    let template_set: BTreeSet<&String> = template.iter().collect();
    let only_example: Vec<_> = example_set.difference(&template_set).collect();
    let only_template: Vec<_> = template_set.difference(&example_set).collect();
    assert!(
        only_example.is_empty() && only_template.is_empty(),
        "mushd.toml.example and deploy/instance/mushd.toml.tmpl have drifted — \
         only in the example: {only_example:?}; only in the template: \
         {only_template:?}"
    );

    // The template is the thing the deployment installs, so it is the one that
    // must be complete; the example is the documentation of the same surface.
    for section in REQUIRED_SECTIONS {
        assert!(
            example_set.contains(&section.to_string()),
            "mushd.toml.example does not document [{section}]"
        );
    }
}

/// The instance parameters are a closed set: every `__KEY__` the template uses
/// has a line in every shipped `instance.env`, and no shipped env carries a key
/// the template does not use. The first half is what stops a half-rendered
/// install; the second is what stops an operator editing a knob that is wired
/// to nothing.
#[test]
fn every_template_placeholder_is_a_shipped_instance_env_key() {
    let root = repo_root();
    let template = read(&root.join("deploy/instance/mushd.toml.tmpl"));

    // Only the live lines: the header's own prose spells the mechanism out with
    // `__KEY__` and `__SOMETHING__`, which are examples, not parameters.
    let mut used = BTreeSet::new();
    for line in template.lines() {
        let mut rest = live(line);
        while let Some(start) = rest.find("__") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("__") else { break };
            let name = &after[..end];
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                used.insert(name.to_string());
            }
            rest = &after[end + 2..];
        }
    }
    assert!(!used.is_empty(), "the template is not parameterised at all");

    for env_file in SHIPPED_ENVS {
        let env = parse_env(&read(&root.join("deploy/instance").join(env_file)));
        let keys: BTreeSet<String> = env.into_iter().map(|(k, _)| k).collect();
        let missing: Vec<_> = used.difference(&keys).collect();
        let unused: Vec<_> = keys.difference(&used).collect();
        assert!(
            missing.is_empty(),
            "{env_file} is missing template keys: {missing:?}"
        );
        assert!(
            unused.is_empty(),
            "{env_file} declares keys the template never uses: {unused:?}"
        );
    }
}
