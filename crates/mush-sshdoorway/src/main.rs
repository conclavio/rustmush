//! `sshdoorway --config <path> --generation <a|b> [--check]` (doc 29 §9-§10).
//!
//! | Signal | Effect |
//! |---|---|
//! | `SIGHUP` | reload routing and limits (`[listen]` needs a swap) |
//! | `SIGTERM` | drain: stop accepting, warn on schedule, exit at zero sessions or the deadline |
//! | second `SIGTERM`, `SIGUSR1` | finish now: one warning, a grace period, close everything |
//! | `SIGUSR2` | reopen the log file (logrotate) |

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

use std::path::PathBuf;
use std::process::ExitCode;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;

use mush_sshdoorway::Config;
#[cfg(unix)]
use mush_sshdoorway::{Doorway, listen, logging, notify};

struct Args {
    config: PathBuf,
    generation: String,
    check: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        config: PathBuf::from("/etc/rustmush/sshdoorway.toml"),
        generation: "a".to_owned(),
        check: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" => args.config = it.next().ok_or("--config needs a path")?.into(),
            "--generation" => args.generation = it.next().ok_or("--generation needs a name")?,
            "--check" => args.check = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let g = &args.generation;
    if g.is_empty()
        || g.len() > 16
        || !g
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err("--generation must be 1-16 lowercase letters or digits".into());
    }
    Ok(args)
}

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("sshdoorway: runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sshdoorway: {e}");
            tracing::error!(error = %e, "sshdoorway exiting");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    let cfg = Config::load(&args.config).map_err(|e| format!("{}: {e}", args.config.display()))?;
    if args.check {
        println!(
            "{}: ok ({} services)",
            args.config.display(),
            cfg.services.len()
        );
        return Ok(());
    }
    #[cfg(unix)]
    {
        serve(args, cfg).await
    }
    #[cfg(not(unix))]
    {
        let _ = cfg;
        Err(
            "sshdoorway serves on Linux only (Unix sockets, SO_REUSEPORT); --check works anywhere"
                .into(),
        )
    }
}

#[cfg(unix)]
async fn serve(args: Args, cfg: Config) -> Result<(), String> {
    let log = logging::init(&cfg.log, &args.generation)?;
    if listen::migrate_req_enabled() == Some(false) {
        tracing::warn!(
            "net.ipv4.tcp_migrate_req is 0: connections queued on a draining generation will be reset"
        );
    }
    let connector: Arc<dyn mush_sshdoorway::upstream::Connector> =
        Arc::new(mush_sshdoorway::upstream::UnixConnector);
    let door = Doorway::new(cfg.clone(), connector)?;
    tracing::warn!(
        generation = %args.generation,
        fingerprint = %door.host_fingerprint,
        "sshdoorway starting"
    );

    let mut listeners = Vec::new();
    for addr in &cfg.listen.addrs {
        let l = listen::bind(*addr).map_err(|e| format!("binding {addr}: {e}"))?;
        listeners.push(l);
    }
    ready(&door).await?;
    for l in listeners {
        let door = Arc::clone(&door);
        tokio::spawn(async move {
            if let Err(e) = door.serve(l).await {
                tracing::error!(error = %e, "listener failed");
            }
        });
    }
    tokio::spawn(Arc::clone(&door).summarize_forever());
    notify::notify("READY=1");
    tracing::warn!("sshdoorway ready");

    signals(&args, &door, &log).await?;
    tracing::warn!("sshdoorway drained; exiting");
    Ok(())
}

#[cfg(unix)]
/// Readiness (29a A-3): at least one service must answer `Ping` within 30 s;
/// a service that does not is logged, not fatal (an instance being down must
/// not stop players reaching the others).
async fn ready(door: &Arc<Doorway>) -> Result<(), String> {
    let cfg = door.config();
    if cfg.services.is_empty() {
        tracing::warn!("no services configured; every player gets an empty menu");
        return Ok(());
    }
    let start = tokio::time::Instant::now();
    loop {
        match door.ping_all().await {
            Ok(()) => return Ok(()),
            Err(e) if start.elapsed() < Duration::from_secs(30) => {
                tracing::info!(error = %e, "waiting for services");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                let mut any = false;
                for s in &cfg.services {
                    if mush_sshdoorway::upstream::ping(&*door_connector(door), s)
                        .await
                        .is_ok()
                    {
                        any = true;
                    }
                }
                if any {
                    tracing::warn!(error = %e, "a service is unreachable; serving the rest");
                    return Ok(());
                }
                return Err(format!("no service answered: {e}"));
            }
        }
    }
}

#[cfg(unix)]
fn door_connector(door: &Arc<Doorway>) -> Arc<dyn mush_sshdoorway::upstream::Connector> {
    door.connector()
}

#[cfg(unix)]
async fn signals(args: &Args, door: &Arc<Doorway>, log: &logging::LogGuard) -> Result<(), String> {
    use tokio::signal::unix::{SignalKind, signal};
    let sig = |k: SignalKind| signal(k).map_err(|e| format!("signal handler: {e}"));
    let mut hup = sig(SignalKind::hangup())?;
    let mut term = sig(SignalKind::terminate())?;
    let mut usr1 = sig(SignalKind::user_defined1())?;
    let mut usr2 = sig(SignalKind::user_defined2())?;
    let mut int = sig(SignalKind::interrupt())?;
    let mut draining: Option<tokio::task::JoinHandle<()>> = None;
    loop {
        let empty = async {
            if draining.is_some() {
                door.drain.wait_empty().await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            _ = hup.recv() => match Config::load(&args.config) {
                Ok(new) => match door.reload(new) {
                    Ok(()) => tracing::warn!("config reloaded"),
                    Err(e) => tracing::error!(error = %e, "reload refused; keeping the old config"),
                },
                Err(e) => tracing::error!(error = %e, "reload refused; keeping the old config"),
            },
            _ = usr2.recv() => {
                if let Some(f) = &log.file
                    && let Err(e) = f.reopen()
                {
                    tracing::error!(error = %e, path = %f.path().display(), "log reopen failed");
                }
            }
            _ = term.recv() => {
                if draining.is_none() {
                    draining = Some(start_drain(door));
                } else {
                    finish_now(door, &mut draining);
                }
            }
            _ = int.recv() => {
                if draining.is_none() {
                    draining = Some(start_drain(door));
                }
                finish_now(door, &mut draining);
            }
            _ = usr1.recv() => {
                if draining.is_none() {
                    draining = Some(start_drain(door));
                }
                finish_now(door, &mut draining);
            }
            () = empty => return Ok(()),
        }
    }
}

#[cfg(unix)]
fn start_drain(door: &Arc<Doorway>) -> tokio::task::JoinHandle<()> {
    notify::notify("STOPPING=1");
    door.drain.begin();
    let cfg = door.config();
    let deadline = Duration::from_secs(cfg.limits.drain_deadline_secs);
    let warnings = cfg.warnings().unwrap_or_default();
    let message = cfg.limits.drain_message.clone();
    tracing::warn!(
        sessions = door.drain.live(),
        deadline_secs = deadline.as_secs(),
        "draining: stopped accepting"
    );
    let door = Arc::clone(door);
    tokio::spawn(async move { door.drain.run(deadline, &warnings, &message).await })
}

#[cfg(unix)]
fn finish_now(door: &Arc<Doorway>, draining: &mut Option<tokio::task::JoinHandle<()>>) {
    if let Some(task) = draining.take() {
        task.abort();
    }
    let cfg = door.config();
    let grace = Duration::from_secs(cfg.limits.force_grace_secs);
    let message = cfg.limits.drain_message.clone();
    tracing::warn!(grace_secs = grace.as_secs(), "finishing now");
    let door = Arc::clone(door);
    *draining = Some(tokio::spawn(async move {
        door.drain.finish_now(grace, &message).await;
    }));
}
