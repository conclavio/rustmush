//! `--check-config`: the boot refusals a binary can find without booting
//! (WS04-S3).
//!
//! Several `mushd.toml` values are boot-fatal by design — a misspelt
//! `[store].receipt_policy` refuses the boot rather than silently downgrading
//! to the weaker `applied` receipts, and a digest-mismatched TacSim pack
//! refuses it rather than composing a world nobody wrote. That is the right
//! behaviour and it is why this module exists: on the split deployment the
//! process a supervisor starts is `mush-gateway`, the world half is
//! `mush-server`, and a value either of them refuses is a box that does not
//! come up.
//!
//! So each binary can be asked the same question ahead of time. The flag loads
//! the config and runs the validations that need neither a socket, nor the
//! store, nor the quarantine spool's exclusive lifetime lock — which is what
//! makes it safe to run against a config a LIVE instance is already serving.
//!
//! What lives here is the part every binary can check, named rather than
//! implied: `[store].store`, `[store].receipt_policy` and `[game].ic_epoch`.
//! [`check_ipc`] adds `[ipc].listen` for the two SPLIT halves only — the
//! boundary is boot-fatal in `mush-server` and `mush-gateway` and does not
//! exist at all in the all-in-one, so folding it into [`check`] would make
//! `mushd --check-config` refuse a key `mushd` never reads (W1-B7-FIX).
//! The world half's extra checks (the TacSim pack block, the `[store.archive]`
//! window) need crates the gateway does not depend on, so they hang off
//! `mush_server::boot::check_config`, which calls [`check`] first — and NOT
//! [`check_ipc`], because `mushd` reaches the boot half's list too. One
//! [`verdict`] then turns either set into the process's ending, so the three
//! binaries cannot drift into reporting the same finding three different ways.
//!
//! The list is deliberately explicit, because the previous wording — "every
//! validation" — was a promise the code did not keep: `[game].ic_epoch` is
//! boot-fatal in `build_engine` and was not checked here at all (W1-B8-FIX),
//! and `[ipc].listen` is boot-fatal in both split halves and was not checked
//! here either (W1-B7-FIX). A preflight that claims completeness has to be
//! read as a claim about a specific set of keys; anything boot-fatal outside
//! that set is a bug in this list, and belongs on it.

use std::net::SocketAddr;
use std::path::Path;

use crate::config::{Config, ConfigError};
use crate::exit::ExitClass;
use crate::generation::Generation;

/// The boot refusals every binary can raise from the parsed config alone,
/// collected rather than short-circuited: an operator fixing a config wants the
/// whole list, not the first line of it.
///
/// Three keys, and the list is the contract:
///
/// - `[store].store`, via [`Config::store_backend`];
/// - `[store].receipt_policy`, via [`Config::store_receipt_policy`];
/// - `[game].ic_epoch`, via [`crate::config::GameCfg::game_clock`] — a
///   malformed epoch is boot-fatal in the world half's `build_engine` (a
///   silently-wrong calendar being worse than not starting), so an operator
///   must be able to find it here rather than from a supervisor watching a boot
///   loop (W1-B8-FIX).
///
/// None of the three opens anything — each is a pure reading of the parsed
/// stanza — so this is safe beside a running instance.
#[must_use]
pub fn check(config: &Config) -> Vec<String> {
    let mut refusals = Vec::new();
    if let Err(error) = config.store_backend() {
        refusals.push(error.to_string());
    }
    if let Err(error) = config.store_receipt_policy() {
        refusals.push(error.to_string());
    }
    if let Err(error) = config.game.game_clock() {
        refusals.push(error.to_string());
    }
    // Doc 29a §9: the SSH port's gates. A cap of zero or an unparsable
    // exempt network is a lockout the operator should hear about here, not
    // from the first player who cannot get in.
    if let Err(error) = config.ssh_auth_deadline() {
        refusals.push(error.to_string());
    }
    if let Err(error) = config.ipguard.guard_config() {
        refusals.push(error.to_string());
    }
    refusals
}

/// The one refusal only the SPLIT halves can raise: `[ipc].listen`, via
/// [`Config::ipc_endpoint`] (W1-B7-FIX).
///
/// `mush-server` and `mush-gateway` both build the boundary endpoint during
/// boot and both die on a bad one, so it belongs on the preflight list by the
/// module's own rule. It is not in [`check`] because the all-in-one never reads
/// `[ipc]` at all — a `mushd --check-config` that refused a config over the
/// boundary would be reporting a failure that binary cannot have. So the two
/// split `main`s call this beside [`check`] and `mushd` does not.
///
/// Like [`check`] this opens nothing: `ipc_endpoint` parses the stanza and, for
/// the TCP form, compares it against the store's own authority. (It also emits
/// the boundary's unauthenticated-TCP warn, which is the same thing an operator
/// would see at boot and is worth seeing early.)
///
/// # Platform tolerance
///
/// A Unix-socket path — the form every shipped template uses, and the one
/// `docs/ops.md` requires on Linux prod — cannot parse off Unix, where there
/// are no Unix domain sockets. That refusal is NOT reported: it says the host
/// running the preflight has no such sockets, not that the config is wrong, and
/// treating it as a refusal would have `--check-config` on Windows condemn the
/// very configs the deployment target runs. Everything else still refuses on
/// every platform — a port collision with the store, and the empty value, which
/// is the only string `IpcEndpoint::parse` rejects on Unix too.
#[must_use]
pub fn check_ipc(config: &Config) -> Vec<String> {
    match config.ipc_endpoint() {
        Ok(_) => Vec::new(),
        Err(ConfigError::BadIpcListen { .. })
            if !cfg!(unix)
                && !config.ipc.listen.is_empty()
                && config.ipc.listen.parse::<SocketAddr>().is_err() =>
        {
            Vec::new()
        }
        Err(error) => vec![error.to_string()],
    }
}

/// The gateway half's refusal of a trusted socket it must not bind (doc 33
/// §4.4): under the two-generation deploy, `[ssh].trusted_socket` is a
/// **symlink** the swap script re-points at the serving generation's own
/// `<stem>.<gen>.sock`, and no gateway binds the symlink name. A gateway
/// started without `--generation` would bind exactly that name (replacing the
/// link with a socket of its own and cutting the doorway off from the
/// generations), so `--check-config` refuses it. With a generation the
/// gateway derives its own path ([`Generation::socket_path`]) and the link is
/// not its business; without a trusted socket there is nothing to check. A
/// configured path that does not exist yet, or is a socket, is no refusal:
/// the bind probe at boot has the last word on those.
#[must_use]
pub fn check_trusted_socket_link(config: &Config, generation: Option<Generation>) -> Vec<String> {
    if generation.is_some() {
        return Vec::new();
    }
    let Some(path) = config.ssh.trusted_socket_path() else {
        return Vec::new();
    };
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => vec![format!(
            "[ssh].trusted_socket = {} is a symlink: the generations' handoff link (doc 33 \
             §4.4), which no gateway binds. Start this gateway as a generation \
             (--generation a|b) so it binds its own socket beside it",
            path.display()
        )],
        _ => Vec::new(),
    }
}

/// Spend a set of refusals as this process's ending: each one on stderr, the
/// verdict in the shared exit vocabulary.
///
/// A clean config says so on stdout — a preflight that printed nothing would be
/// indistinguishable from one that never ran.
#[must_use]
pub fn verdict(program: &str, path: &Path, refusals: &[String]) -> ExitClass {
    if refusals.is_empty() {
        println!("{program}: {} is usable", path.display());
        return ExitClass::Ok;
    }
    for refusal in refusals {
        eprintln!("{program}: {refusal}");
    }
    eprintln!(
        "{program}: {} would refuse the boot ({} problem{})",
        path.display(),
        refusals.len(),
        if refusals.len() == 1 { "" } else { "s" }
    );
    ExitClass::Failure
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{check, check_ipc, check_trusted_socket_link, verdict};
    use crate::config::Config;
    use crate::exit::ExitClass;
    use crate::generation::Generation;

    /// The failure `--check-config` exists for: a typo in a boot-fatal key.
    /// The refusal has to NAME the offending value, or a preflight that says
    /// "no" without saying which line is no better than the boot failure it
    /// replaces.
    #[test]
    fn a_typod_receipt_policy_is_named() {
        let mut config = Config::default();
        config.store.receipt_policy = "hostpowerloss".to_owned();
        let refusals = check(&config);
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert!(
            refusals.iter().any(|r| r.contains("hostpowerloss")),
            "{refusals:?}"
        );
        assert_eq!(
            verdict("mushd", std::path::Path::new("mushd.toml"), &refusals),
            ExitClass::Failure
        );
    }

    /// Both `[store]` readings are checked, not just the first: a config with
    /// two mistakes reports two.
    #[test]
    fn every_refusal_is_collected_not_just_the_first() {
        let mut config = Config::default();
        config.store.store = "postgres".to_owned();
        config.store.receipt_policy = "eventually".to_owned();
        let refusals = check(&config);
        assert_eq!(refusals.len(), 2, "{refusals:?}");
        assert!(refusals.iter().any(|r| r.contains("postgres")));
        assert!(refusals.iter().any(|r| r.contains("eventually")));
    }

    /// `[game].ic_epoch` is boot-fatal in `build_engine` — a malformed epoch
    /// refuses the boot rather than running a silently-wrong calendar — so a
    /// preflight that let it through would hand an operator a "usable" verdict
    /// for a config that cannot start (W1-B8-FIX). The refusal has to name the
    /// KEY as well as the value: `[store]` and `[game]` can both hold a string
    /// nobody recognises, and "invalid" alone does not say which line to edit.
    #[test]
    fn an_unparseable_ic_epoch_is_named_by_its_key() {
        let mut config = Config::default();
        config.game.ic_epoch = "yesterday".to_owned();
        let refusals = check(&config);
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert!(
            refusals
                .iter()
                .any(|r| r.contains("[game].ic_epoch") && r.contains("yesterday")),
            "{refusals:?}"
        );
        assert_eq!(
            verdict("mushd", std::path::Path::new("mushd.toml"), &refusals),
            ExitClass::Failure
        );
    }

    /// ...and the shipped defaults pass, so the flag is usable as a gate.
    #[test]
    fn the_defaults_are_usable() {
        let refusals = check(&Config::default());
        assert!(refusals.is_empty(), "{refusals:?}");
        assert_eq!(
            verdict("mushd", std::path::Path::new("mushd.toml"), &refusals),
            ExitClass::Ok
        );
    }

    /// `[ipc].listen` is boot-fatal in both split halves, so a value neither
    /// half could bind has to be findable here rather than from a supervisor
    /// watching a relaunch loop (W1-B7-FIX). The empty string is the one value
    /// `IpcEndpoint::parse` rejects on every platform, so this is the portable
    /// proof; the refusal names the key, because "invalid" alone does not say
    /// which line to edit.
    #[test]
    fn an_unparseable_ipc_listen_is_refused_by_name() {
        let mut config = Config::default();
        config.ipc.listen = String::new();
        let refusals = check_ipc(&config);
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert!(
            refusals.iter().any(|r| r.contains("[ipc].listen")),
            "{refusals:?}"
        );
        assert_eq!(
            verdict("mush-server", std::path::Path::new("mushd.toml"), &refusals),
            ExitClass::Failure
        );
    }

    /// The form every shipped template uses. On Unix it validates outright; off
    /// Unix `IpcEndpoint::parse` cannot take a socket path at all, and that
    /// refusal is about the HOST running the preflight, not the config — so it
    /// must not be reported, or `--check-config` on a Windows dev box would
    /// condemn the exact configs the Linux target runs.
    #[test]
    fn the_shipped_unix_socket_form_is_not_a_refusal_on_any_platform() {
        let mut config = Config::default();
        config.ipc.listen = "/run/dev1mush/ipc.sock".to_owned();
        let refusals = check_ipc(&config);
        assert!(refusals.is_empty(), "{refusals:?}");
    }

    /// The tolerance is only for the path form: a boundary pointed at the
    /// store's own port still refuses everywhere, because that config
    /// crash-loops on Linux whatever platform noticed it.
    #[test]
    fn a_boundary_on_the_stores_own_port_is_refused_on_every_platform() {
        let mut config = Config::default();
        config.store.store = "redis".to_owned();
        config.store.redis_url = "redis://127.0.0.1:6791".to_owned();
        config.ipc.listen = "127.0.0.1:6791".to_owned();
        let refusals = check_ipc(&config);
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert!(refusals.iter().any(|r| r.contains("6791")), "{refusals:?}");
    }

    /// ...and the all-in-one never reads `[ipc]`, so the shared list must stay
    /// silent about it: `mushd --check-config` reporting a boundary refusal
    /// would be naming a failure that binary cannot have.
    #[test]
    fn the_shared_check_ignores_the_boundary_the_all_in_one_never_reads() {
        let mut config = Config::default();
        config.ipc.listen = String::new();
        assert!(check(&config).is_empty(), "{:?}", check(&config));
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mush-preflight-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn with_trusted_socket(path: &Path) -> Config {
        let mut config = Config::default();
        config.ssh.trusted_socket = path.display().to_string();
        config
    }

    /// No trusted socket, a path that does not exist, or a plain file there:
    /// not this check's business, with or without a generation.
    #[test]
    fn a_trusted_socket_that_is_not_a_symlink_is_no_refusal() {
        let dir = scratch("plain");
        let none = Config::default();
        assert!(check_trusted_socket_link(&none, None).is_empty());
        let missing = with_trusted_socket(&dir.join("missing.sock"));
        assert!(check_trusted_socket_link(&missing, None).is_empty());
        let file = dir.join("file.sock");
        std::fs::write(&file, b"").expect("a file");
        let plain = with_trusted_socket(&file);
        assert!(check_trusted_socket_link(&plain, None).is_empty());
        assert!(check_trusted_socket_link(&plain, Some(Generation::A)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A symlinked trusted socket is refused without a generation, by its
    /// path, and accepted with one (the generation binds beside it).
    #[cfg(unix)]
    #[test]
    fn a_symlinked_trusted_socket_is_refused_without_a_generation() {
        let dir = scratch("link");
        let link = dir.join("dev1.sock");
        std::os::unix::fs::symlink(dir.join("dev1.a.sock"), &link).expect("a symlink");
        let config = with_trusted_socket(&link);
        let refusals = check_trusted_socket_link(&config, None);
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert!(
            refusals
                .first()
                .is_some_and(|r| r.contains("dev1.sock") && r.contains("symlink")),
            "{refusals:?}"
        );
        assert!(check_trusted_socket_link(&config, Some(Generation::B)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
