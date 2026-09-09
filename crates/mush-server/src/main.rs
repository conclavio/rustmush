//! mush-server — the forked SERVER half of rustmush (M16b).
//!
//! Boots the world task + persistence + modules and serves the gateway over
//! the [`mush_ipc`] boundary socket. Normally spawned as a child of
//! `mush-gateway` (which passes `--ipc <addr>`, and `--config` only when it was
//! given one itself), but it also runs standalone for testing.
//!
//! Usage: `mush-server [--config <path>] [--ipc <addr>]`.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(clippy::unwrap_used, clippy::panic_in_result_fn)]

use std::process::ExitCode;

use mush_config::cli::CliSpec;
use mush_config::exit::{ExitClass, ExitFailure};

/// This binary's half of the one command line the three binaries share
/// (WS22-S1). `--ipc <addr>` overrides `[ipc].listen` — the supervisor picks
/// the socket — and is applied by `Cli::load_config`.
const CLI: CliSpec = CliSpec {
    program: "mush-server",
    usage: "Usage: mush-server [--config <path>] [--ipc <addr>]",
    ipc: true,
    repl: false,
};

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
    let config = match cli.load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", CLI.program);
            return ExitClass::Failure.exit_code();
        }
    };

    // Logging may create/append files, so integrity storage is structurally
    // validated first. Retain this exact instance (and its exclusive lifetime
    // lock) through the zero-write exact-target pass and into the writer.
    let quarantine_spool = match mush_server::quarantine::QuarantineSpool::open(
        config.server.data_dir.join("quarantine"),
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
    /// literals because they are what leaves the process: a supervisor's
    /// classifier and a systemd unit encode them out of tree, and the whole
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
