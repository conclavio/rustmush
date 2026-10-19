//! WS25-S3: the shipped instance template renders a WHOLE config, and the
//! shipped example cannot drift away from it.
//!
//! A missing section silently takes compiled-in defaults the operator never
//! saw. Every instance now renders `deploy/instance/mushd.toml.tmpl`, so this
//! gate holds that common deployment lane to its documented configuration:
//!
//! - every section in `REQUIRED_SECTIONS` is declared in the rendered file;
//!   deployment choices and live simulator keys are explicitly present;
//! - a `__PLACEHOLDER__` with no line in `instance.env` fails here rather than
//!   installing a config with `__START_ROOM__` in it;
//! - `[ipc].listen` cannot be the port `[store].redis_url` already holds — the
//!   WS08-S13 refusal, exercised over the real template rather than a fixture;
//! - each box's `[onboarding]` is the shape that box CHOSE, which is not the
//!   same shape for all of them (see [`Onboarding`]);
//! - the deploy-time `EXTERNAL_IP` marker is substituted before a render and is
//!   in nothing after one — and, so that rule is not a check on itself, the
//!   unresolved render really does produce listeners nothing can bind;
//! - and `mushd.toml.example` declares exactly the template's section set, so a
//!   section added to one and forgotten in the other is a red test.
//!
//! Cut L removed the unused hand-written dev2 lane. A second instance supplies
//! another `instance.env` and joins [`SHIPPED`] when committed.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use mush_config::config::{Config, ConfigError, LogFormat};

/// The ten sections absent from the box this template exists to fix.
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
    "tacsim.time",
    "tacsim.time.ephemeris",
    "tacsim.recovery",
    "tacsim.economy",
    // 16b cut G — the combat arena's five blocks, three of them with their own
    // sub-tables. Doc 16 §7's Configuration paragraph requires all of them in
    // the example and the instance template.
    "tacsim.arena",
    "tacsim.arena.tempo",
    "tacsim.arena.intents",
    "tacsim.arena.ledger",
    // 16b cut N.1 added the block and did not add the row; 16b cut F.4 did.
    "tacsim.arena.reflex",
    "tacsim.match",
    "tacsim.sensors",
    "tacsim.resolution",
    "tacsim.harness",
];

/// What a box's `[onboarding]` block is FOR. The shipped instances deliberately
/// differ here, so "the start rooms are not Limbo" cannot be one rule for all of
/// them — it is the REFERENCE instance's rule, and a box with the other shape
/// gets the assertion that matches its own design instead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Onboarding {
    /// The reference instance: chargen is gated behind staff approval, and the
    /// two start rooms are rooms the operator digs before opening. Neither may
    /// be Limbo — the bootstrap room seeded beside God, reserved from housing,
    /// and not a place to open a game — and they may not be the same room, or
    /// the gate holds unapproved characters nowhere.
    Gated,
    /// No gate at all: `chargen_required = false` with BOTH rooms Limbo. This
    /// is not a half-configured `Gated` — it is `mush_core::OnboardingConfig`'s
    /// all-default form, written up in `crates/mush-core/src/onboarding.rs` as
    /// "the gate always passes, the hook never fires, the spawn move is a no-op,
    /// and approval moves no one — exactly the pre-onboarding behaviour". It is
    /// the right answer on a box with no approval staff whose starter area hangs
    /// straight off Limbo, which is dev1.
    Open,
}

/// One shipped `instance.env`, and the claims that are ITS rather than the
/// template's. A knob a box is free to choose does not belong in a rule applied
/// to every box; it belongs here, next to the box that chose it.
struct ShippedInstance {
    /// The env file, relative to the repository root.
    env: &'static str,
    /// The `[log].format` this box renders, and it must be one
    /// `mush_config::config::LogCfg::format` actually knows: an unrecognised
    /// value degrades to `pretty` with a line on stderr rather than refusing the
    /// boot, so a typo here is a box that is not logging the way its operator
    /// believes. `json` is for a box with a log shipper; `pretty` is for a box
    /// whose operator reads `journalctl` by eye.
    log_format: &'static str,
    /// The onboarding shape this box chose.
    onboarding: Onboarding,
    /// The IC seconds per real second this box renders into `[game].ic_ratio`.
    /// A clock a box is free to choose belongs beside the box, not in a rule
    /// applied to every box; 16b cut AC.2 made it a key so the render stops
    /// reverting it.
    ic_ratio: u32,
    /// The IC date and time at unix 0 this box renders into `[game].ic_epoch`,
    /// as the parsed config carries it — the quotes are the template's.
    ic_epoch: &'static str,
}

/// The shipped parameterisations. 16b cut R-b: one game — the Shadowrun env left
/// with its ruleset, and the disabled-`[tacsim]` branch it used to exercise is
/// rendered by `mush-server`'s `provisioning_smoke` from the reference env with
/// `TACSIM_ENABLED` overridden. 16b cut F.5 added dev1, the first DEPLOYED box
/// to be a render rather than a hand-written file.
const SHIPPED: &[ShippedInstance] = &[
    ShippedInstance {
        env: "deploy/instance/instance.env.example",
        log_format: "json",
        onboarding: Onboarding::Gated,
        ic_ratio: 1,
        ic_epoch: "2020-01-01 00:00",
    },
    ShippedInstance {
        env: "deploy/dev1/instance.env",
        log_format: "pretty",
        onboarding: Onboarding::Open,
        // The owner's clock of 2026-10-18 (doc 18 R10, commit 29230ff5): at rate 24 from
        // this epoch the fresh store read 2351-05-13 06:00 at 2026-10-19T03:00Z — the lane's
        // epoch, one IC day per real hour. (Before: rate 4 from 2123-01-19 11:15, 16c T-88.)
        ic_ratio: 24,
        ic_epoch: "0987-04-20 06:00",
    },
];

/// The value `EXTERNAL_IP` carries in a committed `instance.env` that resolves
/// its address at deploy time. It is a MARKER, not an address: both dev1 deploy
/// scripts rewrite the whole line with the source IP of the box's default route
/// before they render, because the box's address is not a fact about the
/// repository and a committed address is one nobody notices has gone stale.
const DEPLOY_TIME_MARKER: &str = "auto";

/// What a test substitutes where the deploy substitutes the real address — RFC
/// 5737 TEST-NET-3, which is documentation-only and routes nowhere.
const DOCUMENTATION_IP: &str = "203.0.113.10";

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
/// before the first `#`. `grep -n '^[^#]*__'` in deploy/README.md, in the
/// template's own header and in `deploy/instance/render.sh` IS this function;
/// every placeholder rule below goes through it so the shell and the test cannot
/// drift apart.
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

/// Render one shipped instance and hand back the rendered text alongside the
/// parsed config, having first proved the render was complete.
fn rendered_instance(instance: &ShippedInstance) -> (String, Config) {
    let root = repo_root();
    let env_file = instance.env;
    let template = read(&root.join("deploy/instance/mushd.toml.tmpl"));
    let mut env = parse_env(&read(&root.join(env_file)));

    // The deploy's own first step, in one line: the address is resolved into
    // the installed copy of instance.env, and only then is anything rendered.
    for (key, value) in &mut env {
        if key == "EXTERNAL_IP" && value == DEPLOY_TIME_MARKER {
            *value = DOCUMENTATION_IP.to_string();
        }
    }
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

    // ...and the deploy-time marker is not a placeholder the grep above can
    // see, so it gets its own rule. `auto` reaching a rendered config means
    // `[ssh].listen = "auto:6789"`, which no preflight refuses — the symptom
    // would be a box that comes up and answers on no port at all.
    let stragglers: Vec<&str> = text
        .lines()
        .filter(|l| live(l).contains(DEPLOY_TIME_MARKER))
        .collect();
    assert!(
        stragglers.is_empty(),
        "{env_file}: the deploy-time marker {DEPLOY_TIME_MARKER:?} reached the \
         rendered config: {stragglers:?}"
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
        env_file.replace(['/', '\\', '.'], "_"),
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

#[test]
fn the_template_declares_every_section_the_deployed_boxes_lack() {
    for instance in SHIPPED {
        let env_file = instance.env;
        let (text, cfg) = rendered_instance(instance);
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
        assert_ne!(
            cfg.log.file, defaults.log.file,
            "{env_file}: [log] is the default block again"
        );
        // The FORMAT is the box's to choose — `json` for a box with a log
        // shipper, `pretty` for a box read by eye — so it is asserted against
        // that box's declared choice rather than against one global value. What
        // is NOT the box's to choose is whether the value means anything:
        // `LogCfg::format` recognises exactly two strings and degrades anything
        // else to `Pretty` with a line on stderr, which is a box logging in a
        // format its operator did not pick.
        assert_eq!(
            cfg.log.format, instance.log_format,
            "{env_file}: [log].format is not the format this box chose"
        );
        let intended = match instance.log_format {
            "json" => LogFormat::Json,
            "pretty" => LogFormat::Pretty,
            other => panic!("{env_file}: {other:?} is not a format LogCfg::format knows"),
        };
        assert_eq!(
            cfg.log.format(),
            intended,
            "{env_file}: [log].format does not survive LogCfg::format, so this \
             box is logging in a format nobody chose"
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

/// Live boot-read numbers must be written even when they equal the defaults.
/// Parsing alone cannot distinguish an omitted knob from an intentional value.
///
/// **16b cut Z.1** (16c N6) put `[tacsim.arena].squad_size` on this list: its
/// default is the shipped four, so the parsed-value assertion in
/// `q7_the_rendered_instances_field_machines_and_the_example_does_not` reads
/// four whether or not either file writes the key, and only the raw table can
/// tell an operator's written lance from a compiled-in one.
///
/// **16b cut AF.1** (16c N16) put `[tacsim.arena].snapshot_ms` here on the same
/// precedent: an integer knob with an operator meaning — `0` is the kill switch
/// that makes a restart void every running match — whose default is the shipped
/// five seconds, so only the raw table tells a written cadence from a compiled
/// one.
#[test]
fn live_ground_sky_and_path_keys_are_explicit_in_every_shipped_config() {
    let mut configs = vec![(
        "mushd.toml.example",
        read(&repo_root().join("mushd.toml.example")),
    )];
    for instance in SHIPPED {
        configs.push((instance.env, rendered_instance(instance).0));
    }
    for (name, text) in configs {
        let raw: toml::Value = toml::from_str(&text).expect("shipped config parses");
        for (path, expected) in [
            ("tacsim.ground_cell_cm", 1000),
            ("tacsim.time.ephemeris.epoch", 0),
            // Doc 19 cut A.6 (§8; doc 18 Q-18-37): the shipped sky moves at real
            // motion; the Rust default stays 0 for the tree's pinned figures.
            ("tacsim.time.ephemeris.rate", 1000),
            ("atlas.path_max_cells", 4096),
            ("tacsim.arena.squad_size", 4),
            // 16b cut AF.1: the snapshot cadence, and its kill switch at `0`.
            ("tacsim.arena.snapshot_ms", 5000),
            // 16b cut AE.3: `[tacsim.arena].max_units_by_class` is deliberately NOT
            // here — it is a map with a compiled default, like `[tacsim.arena.tempo]`'s
            // keys, not an integer knob, and the example writes it while the untouched
            // template does not; its parsed value is asserted beside `squad_size`'s.
        ] {
            let mut found = Some(&raw);
            for key in path.split('.') {
                found = found.and_then(|value| value.get(key));
            }
            assert_eq!(
                found.and_then(toml::Value::as_integer),
                Some(expected),
                "{name}: {path} must be explicit"
            );
        }
    }
}

/// The listeners a rendered instance advertises are ADDRESSES. This is the
/// other half of the deploy-time-marker rule: `rendered_instance` proves the
/// marker is gone, and this proves what replaced it is something a socket can
/// be bound to — neither `[ssh].listen` nor `[web].listen` is on any preflight
/// list, so a bad one is found by a box that comes up serving nothing.
#[test]
fn the_rendered_listeners_are_addresses() {
    for instance in SHIPPED {
        let env_file = instance.env;
        let (_, cfg) = rendered_instance(instance);
        cfg.ssh
            .listen
            .parse::<SocketAddr>()
            .unwrap_or_else(|e| panic!("{env_file}: [ssh].listen {:?}: {e}", cfg.ssh.listen));
        let web = cfg
            .web
            .get("listen")
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("{env_file}: [web].listen is missing"));
        web.parse::<SocketAddr>()
            .unwrap_or_else(|e| panic!("{env_file}: [web].listen {web:?}: {e}"));
    }
}

/// The marker rule inside [`rendered_instance`] is a check on a substitution
/// that has already happened, so on its own it can only ever say yes. This is
/// the negative control for it: render a committed env VERBATIM — the way a
/// hand re-render on the box does, with no deploy script resolving the line
/// first — and watch `auto` land in the listeners.
///
/// What that proves is why the marker is guarded in three places and not one.
/// `[ssh].listen` and `[web].listen` are on no `--check-config` list, so the
/// unresolved render passes every preflight and installs a box that comes up
/// and answers on no port. The gates that actually stop it are the deploy
/// scripts' `grep -q "^EXTERNAL_IP=$EXT_IP$"` over the env they just wrote and
/// `deploy/instance/render.sh`'s refusal of a live `auto` in what it rendered;
/// this test is what stops the marker quietly ceasing to be load-bearing (an
/// `EXTERNAL_IP` that stopped feeding the listeners would make all three checks
/// vacuous and nothing else would notice).
#[test]
fn the_unresolved_deploy_time_marker_reaches_the_listeners() {
    let root = repo_root();
    let template = read(&root.join("deploy/instance/mushd.toml.tmpl"));
    let mut boxes_with_a_marker = 0;

    for instance in SHIPPED {
        let env_file = instance.env;
        let env = parse_env(&read(&root.join(env_file)));
        if !env
            .iter()
            .any(|(k, v)| k == "EXTERNAL_IP" && v == DEPLOY_TIME_MARKER)
        {
            continue;
        }
        boxes_with_a_marker += 1;

        let text = render(&template, &env);
        let listeners: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| live(l).contains(DEPLOY_TIME_MARKER))
            .filter(|l| live(l).trim_start().starts_with("listen"))
            .collect();
        assert!(
            listeners.len() >= 2,
            "{env_file}: rendering the committed env unresolved should put \
             {DEPLOY_TIME_MARKER:?} in both listeners, so that the deploy's \
             substitution and render.sh's refusal have something to be about; \
             it put it in {listeners:?}"
        );
        for listener in listeners {
            let value = listener
                .split_once('=')
                .expect("a listen line is a key = value")
                .1
                .trim()
                .trim_matches('"');
            assert!(
                value.parse::<SocketAddr>().is_err(),
                "{env_file}: {value:?} binds, so the marker is an address after \
                 all and this control proves nothing"
            );
        }
    }

    assert!(
        boxes_with_a_marker > 0,
        "no shipped instance.env carries EXTERNAL_IP={DEPLOY_TIME_MARKER:?} any \
         more, so the marker rule in rendered_instance guards nothing — drop it \
         with this test or restore the box that used it"
    );
}

#[test]
fn the_template_never_points_the_boundary_at_the_store() {
    for instance in SHIPPED {
        let env_file = instance.env;
        let (_, cfg) = rendered_instance(instance);

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

/// Each box starts characters where it MEANT to, which is not the same place on
/// every box.
///
/// The reference instance gates chargen and starts characters in two dug rooms,
/// and for it "not Limbo" is the rule: Limbo is the bootstrap room seeded beside
/// God, reserved from housing, and not a place to open a game. dev1 (16b cut
/// F.5) is deliberately the other shape — no gate, both rooms Limbo — which is
/// NOT the rule waived but a different rule applied: that exact combination is
/// `mush_core::OnboardingConfig`'s documented all-default no-op, and it is what
/// a box with no approval staff wants when its starter area hangs straight off
/// Limbo (`#0` carries the `ooc` exit into the OOC Area, whose Welcome Hall has
/// the door into the Simulator Lobby). Asserting it here is what stops that
/// choice decaying into a half-configured gate: a `chargen_required = true` on
/// dev1 with both rooms still `0` would hold every new character in Limbo on
/// the chargen command set with nobody to approve them, and no other gate reads
/// these keys.
#[test]
fn the_shipped_instances_start_characters_where_they_meant_to() {
    for instance in SHIPPED {
        let env_file = instance.env;
        let (_, cfg) = rendered_instance(instance);
        match instance.onboarding {
            Onboarding::Gated => {
                assert!(
                    cfg.onboarding.chargen_required,
                    "{env_file}: a gated instance must actually gate"
                );
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
            Onboarding::Open => {
                assert!(
                    !cfg.onboarding.chargen_required,
                    "{env_file}: both start rooms are Limbo, so a gate here holds new \
                     characters in the bootstrap room with nobody to approve them"
                );
                assert_eq!(
                    cfg.onboarding.start_room_id(),
                    mush_core::LIMBO,
                    "{env_file}: an ungated box names Limbo deliberately; any other id \
                     is a room this world does not seed"
                );
                assert_eq!(
                    cfg.onboarding.start_room_unapproved_id(),
                    mush_core::LIMBO,
                    "{env_file}: the unapproved room must be Limbo too — the all-default \
                     no-op is both rooms, not one"
                );
            }
        }
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

    for instance in SHIPPED {
        let env_file = instance.env;
        let env = parse_env(&read(&root.join(env_file)));
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

/// **16b cut AC.2 (16c T-88): each shipped instance renders the IC clock it
/// chose.**
///
/// The template used to write `ic_ratio = 1` and `ic_epoch = "2020-01-01 00:00"`
/// as literals, so an operator's edit to a rendered `mushd.toml` was reverted by
/// the next deploy's render — and because the world timeline is seeded from
/// `[game]` only on an EMPTY store, a wipe after such a render froze the
/// template's clock into the new world. Now `IC_RATIO` and `IC_EPOCH` are
/// instance keys: this pins that each env's own values reach the PARSED config,
/// that the epoch parses (a hard boot error otherwise), and that the rendered
/// lines are exactly `ic_ratio = N` and `ic_epoch = "…"` — the value the env's,
/// the quotes the template's. `every_template_placeholder_is_a_shipped_instance_env_key`
/// covers the two tokens by construction; this is the pin on the values.
#[test]
fn t88_the_shipped_instances_render_the_clock_they_chose() {
    for instance in SHIPPED {
        let env_file = instance.env;
        let (text, cfg) = rendered_instance(instance);
        assert_eq!(
            cfg.game.ic_ratio, instance.ic_ratio,
            "{env_file}: IC_RATIO does not reach [game].ic_ratio"
        );
        assert_eq!(
            cfg.game.ic_epoch, instance.ic_epoch,
            "{env_file}: IC_EPOCH does not reach [game].ic_epoch"
        );
        assert!(
            cfg.game.game_clock().is_ok(),
            "{env_file}: the rendered epoch must parse: an unquoted render would not \
             even be a TOML string"
        );
        let ratio_line = format!("ic_ratio = {}", instance.ic_ratio);
        assert!(
            text.lines().any(|l| l == ratio_line),
            "{env_file}: the render has no line {ratio_line:?} (IC_RATIO)"
        );
        let epoch_line = format!("ic_epoch = \"{}\"", instance.ic_epoch);
        assert!(
            text.lines().any(|l| l == epoch_line),
            "{env_file}: the render has no line {epoch_line:?} (IC_EPOCH carries the \
             bare value; the quotes are the template's)"
        );
    }
}

/// **16b cut Q.7: the rendered instances turn the AI fill on, and the example
/// does not.**
///
/// The two are deliberately different. `mushd.toml.example` is the reference
/// file a reader learns the shape from, and doc 16 §1's acceptance story A1 is
/// two humans in two tanks; the boxes the owner actually plays on are where one
/// human against the machines is the point.
#[test]
fn q7_the_rendered_instances_field_machines_and_the_example_does_not() {
    let root = repo_root();
    for instance in SHIPPED {
        let env_file = instance.env;
        let (_, cfg) = rendered_instance(instance);
        assert!(
            cfg.tacsim.match_.ai_fill,
            "{env_file}: the rendered box should field machines for a lone pilot"
        );
        assert_eq!(
            cfg.tacsim.arena.reflex.rally_ms, 20_000,
            "{env_file}: 16b §9's rally lull"
        );
        // **16b cut E.1**: and half of `withdraw_permille`, written down rather
        // than defaulted, because a doctrine number an operator may want to move
        // is a number the file has to show them.
        assert_eq!(
            cfg.tacsim.arena.reflex.stand_min_permille, 175,
            "{env_file}: 16b cut E.1's en-route stand floor"
        );
        // **16b cut E.3**: and the traffic hold's horizon, for the same reason —
        // 0 is the pre-cut rule and a box that ran it by accident would stop every
        // follower in every column at the ford.
        assert_eq!(
            cfg.tacsim.arena.occupied_stop_ms, 2_000,
            "{env_file}: 16b cut E.3's traffic hold"
        );
        // **16b cut E.4**: and the two clocks that make a solo match end. A box
        // that ran the search or the hunt off by accident is the match that goes
        // quiet for ever, which is the whole of what the sub-cut was for.
        assert_eq!(
            cfg.tacsim.arena.reflex.search_ms, 60_000,
            "{env_file}: 16b cut E.4's search"
        );
        assert_eq!(
            cfg.tacsim.arena.reflex.hunt_after_ms, 90_000,
            "{env_file}: 16b cut E.4's hunt"
        );
        // **16b cut Z.1** (16c N6): and the lance, written down for the same
        // reason — a box on pairs would be twice the commanders for the same field.
        // The default is four too, so this reads the parsed value only; that the
        // key is *written* is `live_ground_sky_and_path_keys_are_explicit_…`'s.
        assert_eq!(
            cfg.tacsim.arena.squad_size, 4,
            "{env_file}: 16b cut Z.1's squad size"
        );
        // **16b cut AF.1** (16c N16): a snapshot every five seconds of arena time.
        // The default is five seconds too, so this reads the parsed value only;
        // that the key is *written* is `live_ground_sky_and_path_keys_are_explicit_…`'s.
        assert_eq!(
            cfg.tacsim.arena.snapshot_ms, 5_000,
            "{env_file}: 16b cut AF.1's snapshot cadence"
        );
        // **16b cut AF.2** (16c N16): the harness's snapshot bound, measured
        // before it was set. The template writes it with the other bounds under
        // a header that says a box never silently runs a compiled-in default;
        // this reads the rendered value (a harness key is not on the
        // explicit-key list, which is for the live arena's knobs).
        assert_eq!(
            cfg.tacsim.harness.max_snapshot_bytes, 7_340_032,
            "{env_file}: 16b cut AF.2's snapshot bound"
        );
        // **16b cut AI.1** (16c `B-84`): the harness's per-wake A* cell bound,
        // sized on the four-seed measurement, written by the template beside the
        // queries bound; this reads the rendered value.
        assert_eq!(
            cfg.tacsim.harness.max_path_cells_per_pulse, 48_128,
            "{env_file}: 16b cut AI.1's cell bound"
        );
        // **16b cut AE.3** (doc 16 §2 W2): one drone a side. The template writes
        // no key, so this is the compiled default read off the parsed struct.
        assert_eq!(
            cfg.tacsim.arena.max_units_by_class,
            BTreeMap::from([("drone".to_owned(), 1)]),
            "{env_file}: 16b cut AE.3's class caps"
        );
    }
    let example = Config::load(&root.join("mushd.toml.example"))
        .unwrap_or_else(|e| panic!("mushd.toml.example does not parse: {e}"));
    assert!(
        !example.tacsim.match_.ai_fill,
        "the reference example stays off: doc 16 §1's A1 is a two-tank story"
    );
    assert_eq!(example.tacsim.arena.reflex.rally_ms, 20_000);
    assert_eq!(example.tacsim.arena.reflex.stand_min_permille, 175);
    assert_eq!(example.tacsim.arena.occupied_stop_ms, 2_000);
    assert_eq!(example.tacsim.arena.reflex.search_ms, 60_000);
    assert_eq!(example.tacsim.arena.reflex.hunt_after_ms, 90_000);
    assert_eq!(example.tacsim.arena.squad_size, 4);
    // 16b cut AF.1: the example writes the snapshot cadence at its default.
    assert_eq!(example.tacsim.arena.snapshot_ms, 5_000);
    // 16b cut AF.2: and the harness's measured snapshot bound.
    assert_eq!(example.tacsim.harness.max_snapshot_bytes, 7_340_032);
    // 16b cut AI.1: and the harness's measured per-wake A* cell bound.
    assert_eq!(example.tacsim.harness.max_path_cells_per_pulse, 48_128);
    // 16b cut AE.3: the example writes `max_units_by_class = { drone = 1 }` inline
    // under `[tacsim.arena]` (no header of its own: the template declares none).
    assert_eq!(
        example.tacsim.arena.max_units_by_class,
        BTreeMap::from([("drone".to_owned(), 1)])
    );
}

/// **16b cut D.3 (`B-33`): every shipped configuration writes the unattended
/// key, and writes the shipped default.**
///
/// The key decides whether a match every pilot has left ever ends, so a box that
/// ran the arena on a compiled-in number nobody read is exactly the shape of the
/// `[tacsim]`-header defect the readiness review found: green everywhere, and
/// missing on the box the owner plays on.
#[test]
fn d3_every_shipped_configuration_declares_the_unattended_interval() {
    let root = repo_root();
    {
        let relative = "mushd.toml.example";
        let text = read(&root.join(relative));
        assert!(
            text.contains("unattended_ms"),
            "{relative}: [tacsim.match].unattended_ms is missing"
        );
        let cfg = Config::load(&root.join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"));
        assert_eq!(
            cfg.tacsim.match_.unattended_ms, 300_000,
            "{relative}: the shipped five minutes"
        );
    }
    // The rendered instances take it from the template, so they are read the way
    // every other rendered value is.
    for instance in SHIPPED {
        let (rendered, cfg) = rendered_instance(instance);
        assert!(
            rendered.contains("unattended_ms"),
            "{}: the template never writes the key",
            instance.env
        );
        assert_eq!(
            cfg.tacsim.match_.unattended_ms, 300_000,
            "{}: the shipped five minutes",
            instance.env
        );
    }
}

/// **16b cut M.4 (16c `B-55`, `T-26`): every shipped configuration carries a
/// signature factor of exactly 1.0**, the default beside it.
///
/// The factor also scales the perception pass's candidate cull; 1.0 is the
/// exact cull (no shipped signature row exceeds 1.0) and the host refuses
/// anything below it. The 1.5 every file carried until M.4 was a circle 2.25
/// times the area it needed.
#[test]
fn m4_every_shipped_configuration_carries_the_exact_signature_factor() {
    let exact = |factor: f64| (factor - 1.0).abs() < f64::EPSILON;
    assert!(
        exact(Config::default().tacsim.sensors.max_signature_factor),
        "the compiled-in default is 1.0"
    );
    let root = repo_root();
    let example = Config::load(&root.join("mushd.toml.example"))
        .unwrap_or_else(|e| panic!("mushd.toml.example does not parse: {e}"));
    assert!(
        exact(example.tacsim.sensors.max_signature_factor),
        "mushd.toml.example: [tacsim.sensors].max_signature_factor is 1.0"
    );
    for instance in SHIPPED {
        let env_file = instance.env;
        let (_, cfg) = rendered_instance(instance);
        assert!(
            exact(cfg.tacsim.sensors.max_signature_factor),
            "{env_file}: the rendered [tacsim.sensors].max_signature_factor is 1.0"
        );
    }
}
