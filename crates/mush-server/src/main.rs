//! mush-server — the forked SERVER half of rustmush (M16b).
//!
//! Boots the world task + persistence + modules and serves the gateway over
//! the [`mush_ipc`] boundary socket. In production it is its own systemd unit
//! beside `mush-gateway` (doc 33 GG.1): it binds `[ipc].listen` from the
//! shared `mushd.toml`, tells systemd `READY=1` once the socket is bound, and
//! the gateway dials the same address. `--ipc <addr>` overrides the listen
//! address for tests and hand runs. SIGTERM is an orderly stop.
//!
//! Usage: `mush-server [--check-config] [--ipc-version] [--config <path>] [--ipc <addr>]`.
//! `--ipc-version` prints the `IPC_VERSION` this binary speaks and exits (doc
//! 33 §4.7: `deploy.sh` compares the new binary's against the running one's
//! and refuses a bump while a gateway generation is draining).
//! `--check-config` loads the config, reports the refusals on the preflight
//! list — `mush_server::boot::check_config` (which is
//! `mush_config::preflight::check` plus the world half's own), and this
//! binary's `[ipc].listen` — and exits without opening the store or the
//! quarantine lifetime lock. Those two functions are the list of record; the
//! flag is not a claim that a boot raises nothing else. It is the one preflight
//! that covers the half an operator rarely starts by hand, and where a bad
//! `[store]`/`[tacsim]` value takes the world down.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(clippy::unwrap_used, clippy::panic_in_result_fn)]

use std::process::ExitCode;

use mush_config::cli::CliSpec;
use mush_config::exit::{ExitClass, ExitFailure};

/// This binary's half of the one command line the three binaries share
/// (WS22-S1). `--ipc <addr>` overrides `[ipc].listen` — a test harness picks
/// the socket — and is applied by `Cli::load_config`.
const CLI: CliSpec = CliSpec {
    program: "mush-server",
    usage: "Usage: mush-server [--check-config] [--ipc-version] [--bootstrap-staff <account>] [--config <path>] [--ipc <addr>]",
    ipc: true,
    repl: false,
    check_config: true,
    bootstrap_staff: true,
    generation: false,
    ipc_version: true,
};

/// How long a booting server waits for the quarantine spool's lifetime lock
/// while another process holds it: long enough for a predecessor to finish
/// exiting (a 900 MB world unmaps in well under a second), short enough that
/// a second live server is refused well inside the unit's start timeout
/// (doc 33 Q-33-5 keeps the 15 s; the gateway keeps dialing meanwhile).
const QUARANTINE_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// This binary's ending, in the one exit vocabulary both halves speak
/// (`mush_config::exit`): 0 clean, 1 a fatal error including a world fault, 3 a
/// world task that never joined. Split out from `main` so the mapping is
/// testable — `ExitCode` itself is opaque, and an exit code that silently
/// changed meaning between the split and the all-in-one would be found by an
/// operator, in an incident.
fn exit_class(result: &Result<(), ExitFailure>) -> ExitClass {
    match result {
        Ok(()) => ExitClass::Ok,
        Err(failure) => failure.class,
    }
}

fn main() -> ExitCode {
    let cli = match mush_config::cli::parse_env(&CLI) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{}: {e}", CLI.program);
            eprintln!("{}", CLI.usage);
            return ExitClass::Failure.exit_code();
        }
    };
    if cli.help {
        println!("{}", CLI.usage);
        return ExitClass::Ok.exit_code();
    }
    // Doc 33 §4.7: the boundary version this binary speaks, for deploy.sh's
    // "does this server upgrade bump IPC_VERSION" check. No config is read.
    if cli.ipc_version {
        println!("{}", mush_ipc::IPC_VERSION);
        return ExitClass::Ok.exit_code();
    }
    let config = match cli.load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", CLI.program);
            return ExitClass::Failure.exit_code();
        }
    };

    // `--check-config` (WS04-S3): the refusals named by
    // `mush_server::boot::check_config` — the list of record for this half —
    // plus `[ipc].listen`, which is boot-fatal here and in the gateway but does
    // not exist in the all-in-one, so it hangs off `preflight::check_ipc`
    // rather than the shared list `mushd` also reads (W1-B7-FIX). None of them
    // opens the store, a socket, or the quarantine spool's exclusive lifetime
    // lock, so it is safe to run beside the live instance it preflights.
    if cli.check_config {
        let mut refusals = mush_server::boot::check_config(&config);
        refusals.extend(mush_config::preflight::check_ipc(&config));
        return mush_config::preflight::verdict(CLI.program, &cli.config, &refusals).exit_code();
    }

    if let Some(account) = &cli.bootstrap_staff {
        return match mush_server::bootstrap::run(&config, account) {
            Ok(()) => {
                println!("Staff account {account} created and linked to God #1.");
                ExitClass::Ok.exit_code()
            }
            Err(e) => {
                eprintln!("mush-server: {e}");
                ExitClass::Failure.exit_code()
            }
        };
    }

    // Logging may create/append files, so integrity storage is structurally
    // validated first. Retain this exact instance (and its exclusive lifetime
    // lock) through the zero-write exact-target pass and into the writer.
    //
    // A predecessor still exiting may hold the spool's lifetime lock for a
    // moment (a unit restart whose old instance is still draining), so the
    // lock is waited for, bounded; a second live server is still refused.
    let quarantine_spool = match mush_server::quarantine::QuarantineSpool::open_waiting(
        config.server.data_dir.join("quarantine"),
        QUARANTINE_LOCK_WAIT,
        || {
            eprintln!(
                "mush-server: the quarantine spool's lock is held by another process; \
                 waiting up to {} s for it to exit",
                QUARANTINE_LOCK_WAIT.as_secs()
            );
        },
    ) {
        Ok(spool) => spool,
        Err(error) => {
            eprintln!("mush-server: quarantine spool preflight failed: {error}");
            return ExitClass::Failure.exit_code();
        }
    };

    let _log_guards = mush_config::logging::init(&config.log, &config.server.data_dir, "server");

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("mush-server: tokio runtime: {e}");
            return ExitClass::Failure.exit_code();
        }
    };
    let result = runtime.block_on(mush_server::serve_classified(
        config,
        None,
        None,
        quarantine_spool,
    ));
    if let Err(failure) = &result {
        eprintln!("mush-server: {failure}");
    }
    exit_class(&result).exit_code()
}

#[cfg(test)]
mod tests {
    use mush_config::exit::{ExitClass, ExitFailure};

    /// This binary's half of the shared exit table. The numbers are asserted as
    /// literals because they are what leaves the process: a systemd unit's
    /// `Restart=` policy encodes them out of tree, and the whole
    /// point of one vocabulary is that `mush-server` and `mushd` cannot drift
    /// into meaning different things by the same number.
    #[test]
    fn the_exit_codes_are_the_shared_vocabulary() {
        assert_eq!(super::exit_class(&Ok(())).code(), 0);
        assert_eq!(
            super::exit_class(&Err(ExitFailure::failure("store unreachable"))).code(),
            1
        );
        assert_eq!(
            super::exit_class(&Err(ExitFailure::new(
                ExitClass::WorldJoinTimeout,
                "world task did not join within 30s"
            )))
            .code(),
            3
        );
        // Reserved by WS22-S3, produced by WS22-S18.
        assert_eq!(ExitClass::QuarantineLockHeld.code(), 2);
    }

    /// A world fault is a `1`, not a class of its own: it shares the catch-all
    /// with every other fatal error and the log says which. The pin is that it
    /// is NOT the timeout code — that distinction is the reason the table
    /// exists.
    #[test]
    fn a_world_fault_is_the_catch_all_and_not_the_timeout() {
        let faulted = ExitFailure::failure("world task faulted at engine.step: the sky fell");
        assert_eq!(super::exit_class(&Err(faulted)), ExitClass::Failure);
        assert_ne!(
            ExitClass::Failure.code(),
            ExitClass::WorldJoinTimeout.code()
        );
    }
}
