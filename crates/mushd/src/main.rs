//! mushd — the rustmush server binary.
//!
//! M1: boots the single-writer world task, the session layer, the interim
//! account store, and the SSH bridge, then runs until Ctrl-C (or an in-game
//! `@shutdown` via [`ControlMsg::Shutdown`]). `--repl` preserves the M0
//! synchronous stdin REPL for offline poking.
//!
//! Usage: `mushd [--repl] [--config <path>]` (config defaults to
//! `mushd.toml` in the working directory; a missing file means defaults).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{ControlMsg, Inbound};
use mush_session::{SessionConfig, SessionManager};
use tracing_subscriber::EnvFilter;

use mushd::accounts::{AccountStore, Accounts};
use mushd::config::Config;
use mushd::ipguard::IpGuard;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::world_task;
use mushd::{repl, unix_now_s};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: mushd [--repl] [--config <path>]");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--repl") {
        repl::run();
        return ExitCode::SUCCESS;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map_or_else(|| PathBuf::from("mushd.toml"), PathBuf::from);
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mushd: {e}");
            return ExitCode::FAILURE;
        }
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    match runtime.block_on(serve(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mushd: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(config: Config) -> Result<(), String> {
    let listen = config.ssh_listen().map_err(|e| e.to_string())?;
    let data_dir = config.server.data_dir.clone();
    tracing::info!(
        name = %config.server.name,
        data_dir = %data_dir.display(),
        %listen,
        "mushd {} starting",
        env!("CARGO_PKG_VERSION"),
    );

    // Account store (M1-interim JSON file; Redis in M2).
    let store = AccountStore::open(&data_dir)
        .map_err(|e| format!("cannot open account store in {}: {e}", data_dir.display()))?;

    // Session layer.
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(config.game.linkdead_grace_s),
        ..SessionConfig::default()
    }));

    // The single-writer world task.
    let engine = world_task::boot_engine();
    let (world, mut world_join) = world_task::spawn(engine, Arc::clone(&sessions));

    // Auth + abuse protection + the SSH hooks bridge.
    let accounts = Arc::new(Accounts::new(store, world.clone()));
    let guard = Arc::new(IpGuard::new());
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        world.clone(),
        Arc::clone(&guard),
    ));
    let bound = start_listener(listen, &data_dir, banner(&config.server.name), bridge)
        .await
        .map_err(|e| format!("SSH listener on {listen}: {e}"))?;

    tracing::info!(ssh = %bound, "up; press Ctrl-C to shut down");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Ctrl-C: shutting down");
            // Announce + stop the loop; the world task delivers the
            // shutdown notice to every connection before exiting.
            world.inbound(Inbound::Control(ControlMsg::Shutdown)).await;
            let _ = world_join.await;
        }
        // The world task can also end itself (in-game @shutdown, M3).
        _ = &mut world_join => {
            tracing::info!("world task ended on its own");
        }
    }
    // Give transports a beat to flush the shutdown notice, then exit;
    // dropping the runtime tears the listener and connections down.
    tokio::time::sleep(Duration::from_millis(200)).await;
    tracing::info!("mushd exiting at {}", unix_now_s());
    Ok(())
}

/// The classic welcome-screen banner (the transport appends the command
/// help below it).
fn banner(name: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        "------------------------------------------------------------------\n\
         \x20  {name}\n\
         \x20  a rustmush server, v{version}\n\
         \n\
         \x20  Connect to an existing account, or create a new one:\n\
         ------------------------------------------------------------------"
    )
}
