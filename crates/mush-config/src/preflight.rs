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
//! the config and runs every validation that needs neither a socket, nor the
//! store, nor the quarantine spool's exclusive lifetime lock — which is what
//! makes it safe to run against a config a LIVE instance is already serving.
//!
//! What lives here is the part every binary can check: the `[store]` stanza,
//! which all three read. The world half's extra checks (the TacSim pack block,
//! the `[store.archive]` window) need crates the gateway does not depend on, so
//! they hang off `mush_server::boot::check_config`, which calls [`check`] first.
//! One [`verdict`] then turns either set into the process's ending, so the three
//! binaries cannot drift into reporting the same finding three different ways.

use std::path::Path;

use crate::config::Config;
use crate::exit::ExitClass;

/// Every `[store]` refusal a boot would raise, collected rather than
/// short-circuited: an operator fixing a config wants the whole list, not the
/// first line of it.
///
/// Neither call opens anything — [`Config::store_backend`] and
/// [`Config::store_receipt_policy`] are pure readings of the parsed stanza — so
/// this is safe beside a running instance.
#[must_use]
pub fn check(config: &Config) -> Vec<String> {
    let mut refusals = Vec::new();
    if let Err(error) = config.store_backend() {
        refusals.push(error.to_string());
    }
    if let Err(error) = config.store_receipt_policy() {
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
