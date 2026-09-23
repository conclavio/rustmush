//! Offline first-staff creation, shared by `mushd` and `mush-server`.
//! Passwords arrive on redirected stdin only. No password or hash is accepted
//! on argv, printed, or logged, and no connection listener is started.

use std::io::{IsTerminal, Read};

use mush_config::accounts::{bootstrap_staff_account, validate_account_name, validate_password};
use mush_config::config::{Config, StoreBackend};
use mush_core::GOD;

use crate::boot;
use crate::quarantine::QuarantineSpool;

/// Run the offline bootstrap, holding the normal quarantine lifetime lock
/// through recovery, world boot, and the atomic account creation.
///
/// # Errors
///
/// Invalid config/credentials, terminal stdin, an in-memory store, an active
/// world, failed recovery/boot, or any existing account refuses the operation.
pub fn run(config: &Config, account: &str) -> Result<(), String> {
    validate_account_name(account).map_err(|e| e.to_string())?;
    if config.store_backend().map_err(|e| e.to_string())? != StoreBackend::Redis {
        return Err(
            "staff bootstrap requires the Redis store; a memory account would be lost on exit"
                .into(),
        );
    }
    let refusals = boot::check_config(config);
    if !refusals.is_empty() {
        return Err(format!(
            "staff bootstrap config refused: {}",
            refusals.join("; ")
        ));
    }
    if std::io::stdin().is_terminal() {
        return Err("staff bootstrap reads the password from redirected stdin; use a password manager or `read -rs` and pipe it (see deploy/README.md)".into());
    }
    let password = read_password(std::io::stdin().lock())?;
    let mut spool = QuarantineSpool::open(config.server.data_dir.join("quarantine"))
        .map_err(|e| format!("staff bootstrap needs the instance stopped: {e}"))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("bootstrap runtime: {e}"))?;
    runtime.block_on(async {
        crate::prepare_quarantine_for_writable_boot(config, &mut spool, None).await?;
        let opened = boot::open_store(config).await?;
        let result = async {
            if !opened
                .store
                .list_accounts()
                .await
                .map_err(|e| e.to_string())?
                .is_empty()
            {
                return Err("staff bootstrap refused: an account already exists".into());
            }
            // Ordinary boot preserves the import, K3 recovery and seed order.
            // An accounts.json import is also an existing-account refusal.
            let booted = boot::build_engine(config, &opened.store, &spool).await?;
            let god = booted
                .engine
                .world
                .get(GOD)
                .ok_or_else(|| "staff bootstrap refused: God #1 is missing".to_owned())?;
            bootstrap_staff_account(&*opened.store, account, &god.name, password).await?;
            opened.store.drain().await.map_err(|e| e.to_string())
        }
        .await;
        let closed = opened.store.close().await.map_err(|e| e.to_string());
        result.and(closed)
    })
}

fn read_password(input: impl Read) -> Result<String, String> {
    // 128 bytes plus CRLF and one sentinel byte: never allocate an unbounded
    // secret from a misdirected pipe. Only the final line ending is removed.
    let mut password = String::new();
    input
        .take(131)
        .read_to_string(&mut password)
        .map_err(|_| "cannot read a UTF-8 password from stdin".to_owned())?;
    if password.ends_with('\n') {
        password.pop();
        if password.ends_with('\r') {
            password.pop();
        }
    }
    if password.contains(['\r', '\n']) {
        return Err("stdin must contain exactly one password line".into());
    }
    validate_password(&password).map_err(|e| e.to_string())?;
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::read_password;

    #[test]
    fn password_pipe_is_bounded_and_preserves_spaces() {
        assert_eq!(read_password(&b"  secret  \r\n"[..]).unwrap(), "  secret  ");
        assert_eq!(
            read_password("x".repeat(128).as_bytes()).unwrap().len(),
            128
        );
        for bad in [
            "",
            "abc",
            "one\ntwo\n",
            &"x".repeat(129),
            &"x".repeat(100_000),
        ] {
            assert!(read_password(bad.as_bytes()).is_err());
        }
    }
}
