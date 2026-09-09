//! mush-gateway — the forked GATEWAY half of rustmush (M16b).
//!
//! Spawns and supervises the `mush-server` child, then serves SSH connections,
//! bridging them to the server over the [`mush_ipc`] boundary socket.
//!
//! Usage: `mush-gateway [--config <path>]`.

use std::path::PathBuf;
use std::process::ExitCode;

use mush_config::config::Config;
use mush_gateway::{GatewayOptions, ServerSource, serve};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: mush-gateway [--config <path>]");
        return ExitCode::SUCCESS;
    }
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map_or_else(|| PathBuf::from("mushd.toml"), PathBuf::from);
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mush-gateway: {e}");
            return ExitCode::FAILURE;
        }
    };

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
        source: ServerSource::Spawn { config_path },
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
