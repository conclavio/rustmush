//! mush-gateway — the forked GATEWAY half of rustmush (M16b).
//!
//! Serves the SSH and web connections, bridging them to the `mush-server`
//! unit over the [`mush_ipc`] boundary socket (doc 33 GG.1): it dials the
//! server at `[ipc].listen`, holds every connection while the server is away,
//! and re-attaches them when it answers. It never starts or stops the server.
//!
//! Usage: `mush-gateway [--check-config] [--ipc-version] [--generation a|b] [--config <path>]`.
//! `--ipc-version` prints the `IPC_VERSION` this binary speaks and exits (doc
//! 33 §4.7: `deploy.sh` compares the new server binary's against the running
//! one's before a server deploy, and refuses a bump while a gateway
//! generation is draining).
//! `--generation a|b` names this gateway's generation (doc 33 §4.3): a `b`
//! gateway sets bit 62 on every connection id it allocates, so the two
//! gateways of a swap never hand the server the same id, and the gateway runs
//! the two-generation lifecycle (doc 33 §4.5, cut GG.4): `READY=1` once it
//! serves, the `draining-<gen>` marker in `$STATE_DIRECTORY`, its own trusted
//! socket beside the configured one, `SIGUSR1` to drain (the swap script
//! only) and `SIGTERM` to finish. Optional; absent is `a` for the id bit and
//! the gateway stops on `SIGTERM` as it always did.
//! `--check-config` loads the config, reports the refusals on the preflight
//! list — `mush_config::preflight::check`, plus `preflight::check_ipc` for the
//! boundary this half also dies on — and exits. Those functions are the list of
//! record; the flag is not a claim that a boot raises nothing else. The
//! world-half validations it cannot run (the `[tacsim]` pack block, the
//! archive window) belong to `mush-server --check-config`, which reads the
//! same file.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::panic_in_result_fn
)]

use std::process::ExitCode;

use mush_config::cli::CliSpec;
use mush_gateway::{GatewayOptions, serve};

/// This binary's half of the one command line the three binaries share
/// (WS22-S1). The gateway takes neither `--ipc` (it reads `[ipc].listen` from
/// the config, the same value the server binds) nor `--repl`; `--generation`
/// is its alone (doc 33 §4.3).
const CLI: CliSpec = CliSpec {
    program: "mush-gateway",
    usage: "Usage: mush-gateway [--check-config] [--ipc-version] [--generation a|b] [--config <path>]",
    ipc: false,
    repl: false,
    check_config: true,
    bootstrap_staff: false,
    generation: true,
    ipc_version: true,
};

fn main() -> ExitCode {
    let cli = match mush_config::cli::parse_env(&CLI) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{}: {e}", CLI.program);
            eprintln!("{}", CLI.usage);
            return ExitCode::FAILURE;
        }
    };
    if cli.help {
        println!("{}", CLI.usage);
        return ExitCode::SUCCESS;
    }
    // Doc 33 §4.7: the boundary version this binary speaks, for deploy.sh's
    // "does this server upgrade bump IPC_VERSION" check. No config is read.
    if cli.ipc_version {
        println!("{}", mush_ipc::IPC_VERSION);
        return ExitCode::SUCCESS;
    }
    let config = match cli.load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", CLI.program);
            return ExitCode::FAILURE;
        }
    };

    // `--check-config` (WS04-S3), before logging installs a file appender:
    // a preflight must leave nothing behind on the box it is asked about.
    if cli.check_config {
        let mut refusals = mush_config::preflight::check(&config);
        refusals.extend(mush_config::preflight::check_ipc(&config));
        // Doc 33 §4.4: a symlinked trusted socket is the generations' link;
        // a gateway without `--generation` must not bind it.
        refusals.extend(mush_config::preflight::check_trusted_socket_link(
            &config,
            cli.generation,
        ));
        return mush_config::preflight::verdict(CLI.program, &cli.config, &refusals).exit_code();
    }

    let _log_guards = mush_config::logging::init(&config.log, &config.server.data_dir, "gateway");

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("mush-gateway: tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let opts = GatewayOptions {
        generation: cli.generation,
        ..GatewayOptions::default()
    };
    match runtime.block_on(serve(config, opts)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mush-gateway: {e}");
            ExitCode::FAILURE
        }
    }
}
