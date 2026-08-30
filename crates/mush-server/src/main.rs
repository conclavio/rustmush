//! mush-server — the forked SERVER half of rustmush (M16b).
//!
//! Boots the world task + persistence + modules and serves the gateway over
//! the [`mush_ipc`] boundary socket. Normally spawned as a child of
//! `mush-gateway` (which passes `--config` and `--ipc <addr>`), but it also
//! runs standalone for testing.
//!
//! Usage: `mush-server [--config <path>] [--ipc <addr>]`.

use std::path::PathBuf;
use std::process::ExitCode;

use mushd::config::Config;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: mush-server [--config <path>] [--ipc <addr>]");
        return ExitCode::SUCCESS;
    }
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map_or_else(|| PathBuf::from("mushd.toml"), PathBuf::from);
    let mut config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mush-server: {e}");
            return ExitCode::FAILURE;
        }
    };
    // `--ipc <addr>` overrides `[ipc].listen` (the supervisor picks the socket).
    if let Some(addr) = args
        .iter()
        .position(|a| a == "--ipc")
        .and_then(|i| args.get(i + 1))
    {
        config.ipc.listen = addr.clone();
    }

    let _log_guards = mushd::logging::init(&config.log, &config.server.data_dir);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("mush-server: tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(mush_server::serve(config, None)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mush-server: {e}");
            ExitCode::FAILURE
        }
    }
}
