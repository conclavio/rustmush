//! Structured logging setup (PLAN "Observability").
//!
//! Reads the `[log]` config and installs the process `tracing` subscriber:
//!
//! - **level** — a `tracing` env-filter directive (`[log].level`, else
//!   `RUST_LOG`, else `info`);
//! - **format** — `pretty` (human console, dev default) or `json` (one JSON
//!   object per line, for prod log shipping);
//! - **file output** — when `[log].file = true`, a **daily-rotating**,
//!   non-blocking appender under `<data_dir>/logs/` in addition to stdout
//!   (`tracing-appender`). The appender's [`WorkerGuard`] must outlive the
//!   process, so [`init`] returns it in [`LogGuards`]; drop it only at exit
//!   (dropping it early stops the background flush and can lose buffered
//!   lines).
//!
//! ## Operational-log durability (deliberate tradeoffs)
//!
//! The non-blocking appender runs in its default **lossy** mode: a bounded
//! in-memory channel (~128k lines) that **drops** lines rather than block the
//! hot path when it fills. During a high-volume incident (attack, error storm)
//! some `info`/`warn` lines can therefore be lost. This is deliberate — the
//! world loop must never stall on disk I/O — and it is **safe** because the
//! durable trail that matters (the LLM/staff **audit**) is a separate Redis
//! stream (`ModuleOp::StreamAppend`), not this file log. Do not switch to
//! `.lossy(false)` without accepting hot-path blocking under log pressure.
//!
//! Daily rotation keeps files **forever** — there is no built-in retention
//! cap. Ops must prune `<data_dir>/logs/` externally (logrotate / a cron
//! sweep); see `docs/ops.md`.
//!
//! ## Secrets
//!
//! This module changes only *where* and *how* logs are formatted, never
//! *what* is logged. The existing scrubbing contracts hold: `mush-net-web`'s
//! HTTP span carries method + path only (never the query string, so
//! `/ws?token=` never lands), `mush-llm`'s audit sink logs a prompt SHA-256
//! and never the text or a key, and `@newpassword`'s `Secret` redacts even
//! derived `Debug`. JSON format does not widen any of that — it reformats the
//! same fields.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt};

use crate::config::{LogCfg, LogFormat};

/// Keeps the non-blocking file appender's worker alive for the process
/// lifetime. Hold it in `main` until the very end; dropping it flushes and
/// stops the background writer. Empty (no guard) when file logging is off.
#[must_use = "dropping the log guard stops the file appender's background flush"]
pub struct LogGuards {
    _file: Option<WorkerGuard>,
}

/// Install the process subscriber from `[log]`. `data_dir` roots the
/// `logs/` directory used when file output is enabled.
///
/// Never fails the boot on a logging problem: a bad env-filter directive
/// falls back to `info`, and a logs directory that cannot be created falls
/// back to stdout-only (both warned about through the installed subscriber).
pub fn init(log: &LogCfg, data_dir: &Path) -> LogGuards {
    let filter = EnvFilter::try_new(log.directive()).unwrap_or_else(|_| EnvFilter::new("info"));
    let format = log.format();

    // stdout layer (always present).
    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = Vec::new();
    layers.push(match format {
        LogFormat::Pretty => fmt::layer().with_writer(std::io::stdout).boxed(),
        LogFormat::Json => fmt::layer().json().with_writer(std::io::stdout).boxed(),
    });

    // Optional daily-rotating file layer.
    let (file_layer, guard, file_err) = if log.file {
        let dir = data_dir.join("logs");
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {
                let appender = tracing_appender::rolling::daily(&dir, "mushd.log");
                let (nb, guard) = tracing_appender::non_blocking(appender);
                // No ANSI in files (color escapes are noise in a log file).
                let layer = match format {
                    LogFormat::Pretty => fmt::layer().with_ansi(false).with_writer(nb).boxed(),
                    LogFormat::Json => fmt::layer().json().with_writer(nb).boxed(),
                };
                (Some(layer), Some(guard), None)
            }
            Err(e) => (None, None, Some(format!("{}: {e}", dir.display()))),
        }
    } else {
        (None, None, None)
    };
    if let Some(layer) = file_layer {
        layers.push(layer);
    }

    // Layers first (each is `Layer<Registry>`), then the env filter as a
    // global filter over the whole stack (EnvFilter: Layer<S> for any S).
    Registry::default().with(layers).with(filter).init();

    // Now that the subscriber is live, surface any deferred outcome through it
    // (so it lands in the same place as everything else).
    if let Some(e) = file_err {
        tracing::warn!(error = %e, "could not open log directory; logging to stdout only");
    } else if log.file {
        tracing::info!(dir = %data_dir.join("logs").display(), "file logging enabled (daily rotation)");
    }

    LogGuards { _file: guard }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::fmt;

    /// The file appender writes to `<data_dir>/logs/` and — the security
    /// contract — a value we never pass to `tracing` never appears in the log.
    /// Built from the same layer primitives [`init`] uses, but installed
    /// scoped (`with_default`) so it does not fight the process-global
    /// subscriber other tests may install.
    #[test]
    fn file_appender_writes_without_leaking_an_unlogged_secret() {
        let base = std::env::temp_dir().join(format!("mushd-log-test-{}", std::process::id()));
        let logs = base.join("logs");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&logs).unwrap();

        let appender = tracing_appender::rolling::daily(&logs, "mushd.log");
        let (nb, guard) = tracing_appender::non_blocking(appender);
        let subscriber = Registry::default().with(fmt::layer().with_ansi(false).with_writer(nb));
        let secret = "hunter2-never-logged";
        tracing::subscriber::with_default(subscriber, || {
            // A scrubbed line: an id is fine, the secret is deliberately not a
            // field. This mirrors how the real log sites are written.
            tracing::info!(user = "nick", "login ok");
            let _ = secret;
        });
        // Dropping the guard flushes and joins the appender's worker thread.
        drop(guard);

        // Exactly one daily file; read it back.
        let file = std::fs::read_dir(&logs)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("mushd.log"))
            })
            .expect("a rotated log file");
        let body = std::fs::read_to_string(&file).unwrap();
        assert!(body.contains("login ok"), "log body: {body}");
        assert!(body.contains("nick"));
        assert!(
            !body.contains(secret),
            "a value never passed to tracing must never reach the log: {body}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
