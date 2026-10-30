//! `sshdoorway --config <path> --generation <a|b> [--check]` (doc 29 §9-§10).
//!
//! | Signal | Effect |
//! |---|---|
//! | `SIGUSR1` | drain (swap.sh only): stop accepting, warn on schedule, close at the deadline |
//! | `SIGTERM`, `SIGINT` | finish now: stop accepting, one warning, close after the grace period |
//! | `SIGHUP` | reload routing and limits (`[listen]` needs a swap) |
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

/// The signal streams, installed before anything slow happens: a default
/// `SIGHUP` or `SIGUSR2` (logrotate's postrotate) would otherwise kill a
/// generation that is still starting.
#[cfg(unix)]
struct Signals {
    hup: tokio::signal::unix::Signal,
    usr1: tokio::signal::unix::Signal,
    usr2: tokio::signal::unix::Signal,
    term: tokio::signal::unix::Signal,
    int: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    fn install() -> Result<Signals, String> {
        use tokio::signal::unix::{SignalKind, signal};
        let sig = |k: SignalKind| signal(k).map_err(|e| format!("signal handler: {e}"));
        Ok(Signals {
            hup: sig(SignalKind::hangup())?,
            usr1: sig(SignalKind::user_defined1())?,
            usr2: sig(SignalKind::user_defined2())?,
            term: sig(SignalKind::terminate())?,
            int: sig(SignalKind::interrupt())?,
        })
    }
}

/// How long after the drain's close (or a finish's grace) the process exits
/// even if some session never finished closing.
#[cfg(unix)]
const HARD_EXIT_AFTER: Duration = Duration::from_secs(60);

/// The marker a draining generation leaves, so systemd restarting it after a
/// crash does not bring it back into service beside its successor. swap.sh
/// clears it before starting that generation fresh.
#[cfg(unix)]
fn draining_marker(cfg: &Config, generation: &str) -> PathBuf {
    let dir = cfg
        .listen
        .host_key
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    dir.join(format!("draining-{generation}"))
}

#[cfg(unix)]
async fn serve(args: Args, cfg: Config) -> Result<(), String> {
    let mut sigs = Signals::install()?;
    let log = logging::init(&cfg.log, &args.generation)?;
    let marker = draining_marker(&cfg, &args.generation);
    if marker.exists() {
        let _ = std::fs::remove_file(&marker);
        tracing::warn!(
            marker = %marker.display(),
            "this generation was draining when it stopped; not serving again (marker consumed)"
        );
        return Ok(());
    }
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

    let result = signals(&args, &door, &log, &mut sigs, &marker).await;
    let _ = std::fs::remove_file(&marker);
    tracing::warn!("sshdoorway exiting");
    result
}

/// Readiness (29a A-3): at least one service must answer `Ping` within 30 s;
/// a service that does not is logged, not fatal (an instance being down must
/// not stop players reaching the others).
#[cfg(unix)]
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
                let connector = door.connector();
                let mut any = false;
                for s in &cfg.services {
                    if mush_sshdoorway::upstream::ping(&*connector, s)
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

/// The lifecycle (doc 29 §9.3, as amended by the ops review):
///
/// | Signal | Effect |
/// |---|---|
/// | `SIGUSR1` | **drain** (swap.sh): stop accepting, warn on schedule, close at the deadline |
/// | `SIGTERM`, `SIGINT` | **finish**: stop accepting, one warning, close after the grace period |
/// | `SIGHUP` | reload routing, limits and menu |
/// | `SIGUSR2` | reopen the log file |
///
/// `SIGTERM` finishing quickly is what keeps `systemctl stop`/`restart`, a
/// reboot or needrestart from turning into a 24 h outage; the 24 h drain is
/// only ever asked for by swap.sh, after a successor is serving.
#[cfg(unix)]
async fn signals(
    args: &Args,
    door: &Arc<Doorway>,
    log: &logging::LogGuard,
    sigs: &mut Signals,
    marker: &std::path::Path,
) -> Result<(), String> {
    let mut work: Option<tokio::task::JoinHandle<()>> = None;
    let mut finishing = false;
    let mut hard_exit: Option<tokio::time::Instant> = None;
    loop {
        let stopping = work.is_some();
        let empty = async {
            if stopping {
                door.drain.wait_empty().await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        let work_done = async {
            match work.as_mut() {
                Some(task) if !task.is_finished() => {
                    let _ = task.await;
                }
                _ => std::future::pending::<()>().await,
            }
        };
        let hard = async {
            match hard_exit {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = sigs.hup.recv() => reload(args, door),
            _ = sigs.usr2.recv() => {
                if let Some(f) = &log.file
                    && let Err(e) = f.reopen()
                {
                    tracing::error!(error = %e, path = %f.path().display(), "log reopen failed");
                }
            }
            _ = sigs.usr1.recv() => {
                if work.is_none() {
                    if let Err(e) = std::fs::write(marker, b"draining\n") {
                        tracing::error!(error = %e, marker = %marker.display(), "cannot write the drain marker");
                    }
                    work = Some(start_drain(door));
                }
            }
            _ = sigs.term.recv() => finish(door, &mut work, &mut finishing),
            _ = sigs.int.recv() => finish(door, &mut work, &mut finishing),
            () = work_done => {
                // The deadline (or the grace) has closed everything it could;
                // whatever has not finished closing gets a minute.
                hard_exit = Some(tokio::time::Instant::now() + HARD_EXIT_AFTER);
            }
            () = empty => return Ok(()),
            () = hard => {
                tracing::warn!(sessions = door.drain.live(), "sessions still closing; exiting anyway");
                return Ok(());
            }
        }
    }
}

#[cfg(unix)]
fn reload(args: &Args, door: &Arc<Doorway>) {
    match Config::load(&args.config) {
        Ok(new) => {
            let log_changed = {
                let old = door.config();
                old.log.file_dir != new.log.file_dir
                    || old.log.file_level != new.log.file_level
                    || old.log.journal_level != new.log.journal_level
            };
            match door.reload(new) {
                Ok(()) => {
                    tracing::warn!("config reloaded");
                    if log_changed {
                        tracing::warn!("[log] changed: that takes effect at the next swap");
                    }
                }
                Err(e) => tracing::error!(error = %e, "reload refused; keeping the old config"),
            }
        }
        Err(e) => tracing::error!(error = %e, "reload refused; keeping the old config"),
    }
}

#[cfg(unix)]
fn start_drain(door: &Arc<Doorway>) -> tokio::task::JoinHandle<()> {
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

/// Finish now. A repeated signal does not restart the grace period.
#[cfg(unix)]
fn finish(
    door: &Arc<Doorway>,
    work: &mut Option<tokio::task::JoinHandle<()>>,
    finishing: &mut bool,
) {
    if *finishing {
        return;
    }
    *finishing = true;
    notify::notify("STOPPING=1");
    door.drain.begin();
    if let Some(task) = work.take() {
        task.abort();
    }
    let cfg = door.config();
    let grace = Duration::from_secs(cfg.limits.force_grace_secs);
    let message = cfg.limits.drain_message.clone();
    tracing::warn!(
        sessions = door.drain.live(),
        grace_secs = grace.as_secs(),
        "finishing now"
    );
    let door = Arc::clone(door);
    *work = Some(tokio::spawn(async move {
        door.drain.finish_now(grace, &message).await;
    }));
}
