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
//! The world half's extra checks (the TacSim pack block, the `[store.archive]`
//! window) need crates the gateway does not depend on, so they hang off
//! `mush_server::boot::check_config`, which calls [`check`] first. One
//! [`verdict`] then turns either set into the process's ending, so the three
//! binaries cannot drift into reporting the same finding three different ways.
//!
//! The list is deliberately explicit, because the previous wording — "every
//! validation" — was a promise the code did not keep: `[game].ic_epoch` is
//! boot-fatal in `build_engine` and was not checked here at all (W1-B8-FIX).
//! A preflight that claims completeness has to be read as a claim about a
//! specific set of keys; anything boot-fatal outside that set is a bug in this
//! list, and belongs on it.

use std::path::Path;

use crate::config::Config;
use crate::exit::ExitClass;

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
    refusals
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
    use super::{check, verdict};
    use crate::config::Config;
    use crate::exit::ExitClass;

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
}
