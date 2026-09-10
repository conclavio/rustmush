//! mush-gateway — the forked GATEWAY half of rustmush (M16b).
//!
//! Spawns and supervises the `mush-server` child, then serves SSH connections,
//! bridging them to the server over the [`mush_ipc`] boundary socket.
//!
//! Usage: `mush-gateway [--check-config] [--config <path>]`.
//! `--check-config` loads the config, reports the refusals on the preflight
//! list — `mush_config::preflight::check`, plus `preflight::check_ipc` for the
//! boundary this half also dies on — and exits. Those functions are the list of
//! record; the flag is not a claim that a boot raises nothing else. This is the
//! process a supervisor actually starts, so it is where an operator's preflight
//! lands; the world-half validations it cannot run (the `[tacsim]` pack block,
//! the archive window) belong to `mush-server --check-config`, which reads the
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
use mush_gateway::{GatewayOptions, ServerSource, serve};

/// This binary's half of the one command line the three binaries share
/// (WS22-S1). The gateway takes neither `--ipc` (it reads `[ipc].listen` and
/// hands it to the child) nor `--repl`.
const CLI: CliSpec = CliSpec {
    program: "mush-gateway",
    usage: "Usage: mush-gateway [--check-config] [--config <path>]",
    ipc: false,
    repl: false,
    check_config: true,
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
    // Only an EXPLICIT `--config` crosses to the child. A gateway that booted
    // the implicit `mushd.toml` — present or absent — must let the child
    // resolve it the same way, or a defaults-booted gateway spawns a child
    // that refuses to start on a path nobody wrote.
    let child_config = cli.child_config_path().map(std::path::Path::to_path_buf);
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
        source: ServerSource::Spawn {
            config_path: child_config,
        },
        store: None,
        ssh_ready: None,
        web_ready: None,
        restart_ready: None,
        siteban_refresh: None,
    };
    match runtime.block_on(serve(config, opts)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mush-gateway: {e}");
            ExitCode::FAILURE
        }
    }
}
