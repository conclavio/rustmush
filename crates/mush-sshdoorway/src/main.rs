//! `sshdoorway --config <path> --generation <a|b> [--check]` (doc 29 §9-§10).
//!
//! | Signal | Effect |
//! |---|---|
//! | `SIGUSR1` | drain (swap.sh only): stop accepting, warn on schedule, close at the deadline |
//! | `SIGTERM`, `SIGINT` | finish now: stop accepting, one warning, close after the grace period |
//! | `SIGHUP` | reload routing and limits (`[listen]` needs a swap) |
//! | `SIGUSR2` | reopen the log file (logrotate) |
//!
//! The lifecycle itself is `mush_drain::lifecycle` (doc 33 §4.5, cut GG.3);
//! this binary parses its arguments, builds the doorway, binds, probes
//! readiness, and gives the lifecycle the doorway's reload, log reopen and
//! drain settings.

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

#[cfg(unix)]
use mush_drain::lifecycle::{DrainConfig, Host, Signals};
#[cfg(unix)]
use mush_drain::{listen, marker, notify};
use mush_sshdoorway::Config;
#[cfg(unix)]
use mush_sshdoorway::{Doorway, logging};

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
    mush_drain::marker::check_generation(&args.generation)?;
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

/// The doorway's side of the lifecycle: what `SIGHUP` reloads, what `SIGUSR2`
/// reopens, and the drain settings of `[limits]`, read when a drain or a
/// finish begins (so a reload counts).
#[cfg(unix)]
struct DoorwayHost<'a> {
    args: &'a Args,
    door: &'a Arc<Doorway>,
    log: &'a logging::LogGuard,
}

#[cfg(unix)]
impl Host for DoorwayHost<'_> {
    fn drain_config(&self) -> DrainConfig {
        let cfg = self.door.config();
        DrainConfig {
            deadline: Duration::from_secs(cfg.limits.drain_deadline_secs),
            warnings: cfg.warnings().unwrap_or_default(),
            message: cfg.limits.drain_message.clone(),
            grace: Duration::from_secs(cfg.limits.force_grace_secs),
        }
    }

    fn reload(&mut self) {
        reload(self.args, self.door);
    }

    fn reopen_logs(&mut self) {
        if let Some(f) = &self.log.file
            && let Err(e) = f.reopen()
        {
            tracing::error!(error = %e, path = %f.path().display(), "log reopen failed");
        }
    }
}

#[cfg(unix)]
async fn serve(args: Args, cfg: Config) -> Result<(), String> {
    // The signal streams first, before anything slow happens: a default
    // SIGHUP or SIGUSR2 (logrotate's postrotate) would otherwise kill a
    // generation that is still starting.
    let mut sigs = Signals::install()?;
    let log = logging::init(&cfg.log, &args.generation)?;
    // Under systemd the marker lives in $STATE_DIRECTORY (StateDirectory=,
    // /var/lib/sshdoorway), the directory swap.sh reads; elsewhere, beside
    // the host key. The doorway never removes it: see `mush_drain::marker`.
    let beside_key = cfg
        .listen
        .host_key
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let marker = marker::path(&args.generation, &beside_key);
    if marker::found(&marker) {
        // READY=1 has been sent: a clean run in systemd's eyes, so
        // Restart=on-failure leaves it be.
        return Ok(());
    }
    listen::warn_if_migrate_req_off();
    let connector: Arc<dyn mush_sshdoorway::upstream::Connector> =
        Arc::new(mush_sshdoorway::upstream::UnixConnector);
    let door = Doorway::new(cfg.clone(), connector)?;
    tracing::warn!(
        generation = %args.generation,
        fingerprint = %door.host_fingerprint,
        "sshdoorway starting"
    );

    // Accept from the moment the socket is bound: a bound listener that is
    // not accepting still takes its share of the SO_REUSEPORT group, and
    // every connection the kernel hands it would wait out the readiness
    // probe (up to 30 s) in its queue.
    for addr in &cfg.listen.addrs {
        let l = listen::bind(*addr).map_err(|e| format!("binding {addr}: {e}"))?;
        let door = Arc::clone(&door);
        tokio::spawn(async move {
            if let Err(e) = door.serve(l).await {
                tracing::error!(error = %e, "listener failed");
            }
        });
    }
    ready(&door).await?;
    tokio::spawn(Arc::clone(&door).summarize_forever());
    notify::ready();
    tracing::warn!("sshdoorway ready");

    let mut host = DoorwayHost {
        args: &args,
        door: &door,
        log: &log,
    };
    mush_drain::lifecycle::run(&mut sigs, &door.drain, &marker, &mut host).await;
    tracing::warn!("sshdoorway exiting");
    Ok(())
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

/// `SIGHUP`: reload routing, limits and menu. A reload that fails validation,
/// or that changes `[listen]`, keeps the old config.
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
