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
//! ## The role (which half is talking)
//!
//! Under the forked split `mush-gateway` and `mush-server` are two processes
//! reading the *same* config with the *same* `data_dir`, and the child
//! inherits the parent's stdio. Without a discriminator both halves write one
//! undifferentiated stream into one file — two independently buffered
//! appenders on one path, uncoordinated at rotation, and named for a `mushd`
//! the deployment does not run.
//!
//! [`init`] therefore takes a caller-supplied **role**, and the role shows up
//! twice:
//!
//! - as the log **file name** — `gateway.log`, `server.log`, `mushd.log`
//!   (see [`log_file_name`]), so the two halves never share an appender; and
//! - as a **`half=` field on every event**, on stdout as well as in the file,
//!   so a merged journal stays readable.
//!
//! The all-in-one `mushd` passes [`DEFAULT_ROLE`], so nothing existing moves.
//!
//! ## Panics
//!
//! [`init`] also installs a process-wide [`std::panic`] hook. The default
//! hook writes to stderr only, so a panic on an `.await` path — exactly the
//! shape `catch_unwind` structurally cannot reach, and that a supervisor sees
//! only as an opaque `JoinError` — never reached the log file at all. The hook
//! logs thread, payload, location and a forced [`Backtrace`] at ERROR under
//! the fixed [`PANIC_MARKER`], then chains to the hook it replaced (so the
//! usual stderr message still appears). Note that the panic event goes through
//! the same env-filter as everything else: a directive that silences ERROR
//! (`RUST_LOG=off`) silences this too.
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
//!
//! A panic payload is the one new thing that reaches the log, and it is
//! whatever the panicking code passed to `panic!`. Do not format a secret into
//! a panic message.

use std::backtrace::Backtrace;
use std::path::Path;

use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::{FormatEvent, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt};

use crate::config::{LogCfg, LogFormat};

/// The role the all-in-one binary logs under. Keeping it `mushd` means the
/// single-process deployment's file name and fields do not move.
pub const DEFAULT_ROLE: &str = "mushd";

/// The fixed, greppable marker on the panic ERROR event installed by [`init`].
/// Ops and tests match on this exact string; do not reword it casually.
pub const PANIC_MARKER: &str = "MUSH-PANIC";

/// Keeps the non-blocking file appender's worker alive for the process
/// lifetime. Hold it in `main` until the very end; dropping it flushes and
/// stops the background writer. Empty (no guard) when file logging is off.
#[must_use = "dropping the log guard stops the file appender's background flush"]
pub struct LogGuards {
    _file: Option<WorkerGuard>,
}

/// Normalise a caller-supplied role into the token that appears in the file
/// name and in the `half=` field: lowercase ASCII letters, digits, `-` and `_`
/// only. Anything else is dropped, and an empty result falls back to
/// [`DEFAULT_ROLE`] — the role is a path component and a field value, so it is
/// never allowed to be attacker- or typo-shaped.
fn sanitize_role(role: &str) -> String {
    let cleaned: String = role
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if cleaned.is_empty() {
        DEFAULT_ROLE.to_string()
    } else {
        cleaned
    }
}

/// The rolling-appender file-name prefix for `role` — `gateway` becomes
/// `gateway.log`, and `tracing-appender` suffixes the date. Two halves sharing
/// one `data_dir` therefore get two files and two independent rotations.
#[must_use]
pub fn log_file_name(role: &str) -> String {
    format!("{}.log", sanitize_role(role))
}

/// Wraps an event formatter to stamp the process role onto every line.
///
/// `tracing-subscriber` has no constant-fields API, and a root span would only
/// cover the thread that entered it, so the stamp is applied at format time:
/// a `half=<role> ` prefix for the human format, and a `"half":"<role>"` first
/// member for JSON (spliced into the object the inner formatter produced, so
/// the line stays one valid JSON object per line).
struct WithRole<E> {
    inner: E,
    role: String,
    json: bool,
}

impl<S, N, E> FormatEvent<S, N> for WithRole<E>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    E: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        if self.json {
            let mut buf = String::new();
            self.inner.format_event(ctx, Writer::new(&mut buf), event)?;
            // The JSON formatter always emits exactly one object. If it ever
            // does not, pass the line through untouched rather than corrupt it.
            match buf.strip_prefix('{') {
                Some(rest) => write!(writer, "{{\"half\":\"{}\",{rest}", self.role),
                None => writer.write_str(&buf),
            }
        } else {
            // Written through the caller's writer, so ANSI stays intact.
            write!(writer, "half={} ", self.role)?;
            self.inner.format_event(ctx, writer, event)
        }
    }
}

/// Install the process-wide panic hook: log the panic at ERROR under
/// [`PANIC_MARKER`] with thread, payload, location and a forced backtrace,
/// then run whatever hook was in place before (normally the default stderr
/// one). Called by [`init`]; exposed for tests.
///
/// The backtrace is [`Backtrace::force_capture`] — `RUST_BACKTRACE` is not a
/// thing an operator remembers to set on the box where the crash happened, and
/// the cost is paid only on a panic.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload_as_str()
            .unwrap_or("<non-string panic payload>")
            .to_string();
        let location = info
            .location()
            .map_or_else(|| "<unknown>".to_string(), ToString::to_string);
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>").to_string();
        let backtrace = Backtrace::force_capture();
        tracing::error!(
            target: "mush::panic",
            thread = %thread_name,
            payload = %payload,
            location = %location,
            backtrace = %backtrace,
            "{PANIC_MARKER}: a thread panicked",
        );
        previous(info);
    }));
}

/// Install the process subscriber from `[log]`. `data_dir` roots the
/// `logs/` directory used when file output is enabled, and `role` names the
/// half of the split that is talking ([`DEFAULT_ROLE`] for the all-in-one) —
/// it becomes the log file's name and a `half=` field on every event.
///
/// Also installs the process panic hook (see [`install_panic_hook`]) so a
/// panic lands in the log file and not only on inherited stderr.
///
/// Never fails the boot on a logging problem: a bad env-filter directive
/// falls back to `info`, and a logs directory that cannot be created falls
/// back to stdout-only (both warned about through the installed subscriber).
pub fn init(log: &LogCfg, data_dir: &Path, role: &str) -> LogGuards {
    let filter = EnvFilter::try_new(log.directive()).unwrap_or_else(|_| EnvFilter::new("info"));
    let format = log.format();
    let role = sanitize_role(role);

    // stdout layer (always present).
    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = Vec::new();
    layers.push(match format {
        LogFormat::Pretty => fmt::layer()
            .event_format(WithRole {
                inner: fmt::format::Format::default(),
                role: role.clone(),
                json: false,
            })
            .with_writer(std::io::stdout)
            .boxed(),
        LogFormat::Json => fmt::layer()
            .json()
            .event_format(WithRole {
                inner: fmt::format::Format::default().json(),
                role: role.clone(),
                json: true,
            })
            .with_writer(std::io::stdout)
            .boxed(),
    });

    // Optional daily-rotating file layer, named for the role so the two halves
    // of the split never share one appender.
    let (file_layer, guard, file_err) = if log.file {
        let dir = data_dir.join("logs");
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {
                let appender = tracing_appender::rolling::daily(&dir, log_file_name(&role));
                let (nb, guard) = tracing_appender::non_blocking(appender);
                // No ANSI in files (color escapes are noise in a log file).
                let layer = match format {
                    LogFormat::Pretty => fmt::layer()
                        .with_ansi(false)
                        .event_format(WithRole {
                            inner: fmt::format::Format::default(),
                            role: role.clone(),
                            json: false,
                        })
                        .with_writer(nb)
                        .boxed(),
                    LogFormat::Json => fmt::layer()
                        .json()
                        .event_format(WithRole {
                            inner: fmt::format::Format::default().json(),
                            role: role.clone(),
                            json: true,
                        })
                        .with_writer(nb)
                        .boxed(),
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

    // The subscriber is live, so a panic now has somewhere to land.
    install_panic_hook();

    // Now that the subscriber is live, surface any deferred outcome through it
    // (so it lands in the same place as everything else).
    if let Some(e) = file_err {
        tracing::warn!(error = %e, "could not open log directory; logging to stdout only");
    } else if log.file {
        tracing::info!(
            dir = %data_dir.join("logs").display(),
            file = %log_file_name(&role),
            "file logging enabled (daily rotation)"
        );
    }

    LogGuards { _file: guard }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use super::*;
    use tracing_subscriber::fmt;
    use tracing_subscriber::fmt::MakeWriter;

    /// A `MakeWriter` that collects everything the layer writes, so a test can
    /// read back the exact bytes a real appender would have received.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// A role stamps the file name; two halves sharing a `data_dir` therefore
    /// get two files, and a junk role can never escape into the path.
    #[test]
    fn the_role_names_the_log_file_and_is_sanitized() {
        assert_eq!(log_file_name("gateway"), "gateway.log");
        assert_eq!(log_file_name("server"), "server.log");
        assert_eq!(log_file_name(DEFAULT_ROLE), "mushd.log");
        assert_ne!(log_file_name("gateway"), log_file_name("server"));
        // Path separators, traversal and case are not the caller's to choose.
        assert_eq!(log_file_name("../../etc/Passwd"), "etcpasswd.log");
        assert_eq!(log_file_name("   "), "mushd.log");
    }

    /// The role reaches every event as a `half=` field, in both formats — and
    /// the JSON line stays one valid object with `half` as a real member.
    #[test]
    fn the_role_is_a_half_field_on_every_event() {
        let pretty = Capture::default();
        let subscriber = Registry::default().with(
            fmt::layer()
                .with_ansi(false)
                .event_format(WithRole {
                    inner: fmt::format::Format::default(),
                    role: "gateway".to_string(),
                    json: false,
                })
                .with_writer(pretty.clone()),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(user = "nick", "login ok");
        });
        let body = pretty.text();
        assert!(body.contains("half=gateway"), "pretty body: {body}");
        assert!(body.contains("login ok"), "pretty body: {body}");

        let json = Capture::default();
        let subscriber = Registry::default().with(
            fmt::layer()
                .json()
                .event_format(WithRole {
                    inner: fmt::format::Format::default().json(),
                    role: "server".to_string(),
                    json: true,
                })
                .with_writer(json.clone()),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(user = "nick", "login ok");
        });
        let body = json.text();
        let line = body.lines().next().expect("a json line");
        assert!(
            line.starts_with("{\"half\":\"server\","),
            "json line: {line}"
        );
        // Still one object per line: balanced braces and a lone trailing one.
        assert!(line.ends_with('}'), "json line: {line}");
        assert!(line.contains("login ok"), "json line: {line}");
    }

    /// The failure the step names: a panic that `catch_unwind` catches must
    /// reach the *subscriber* (and so the log file), not only stderr — with
    /// the payload, a location and the marker — and the hook it replaced must
    /// still run.
    #[test]
    fn the_panic_hook_logs_the_panic_and_chains_to_the_previous_hook() {
        let captured = Capture::default();
        let previous_ran = Arc::new(Mutex::new(false));

        let flag = Arc::clone(&previous_ran);
        std::panic::set_hook(Box::new(move |_| {
            *flag.lock().unwrap() = true;
        }));
        install_panic_hook();

        let subscriber = Registry::default().with(
            fmt::layer()
                .with_ansi(false)
                .event_format(WithRole {
                    inner: fmt::format::Format::default(),
                    role: "server".to_string(),
                    json: false,
                })
                .with_writer(captured.clone()),
        );
        tracing::subscriber::with_default(subscriber, || {
            let outcome = std::panic::catch_unwind(|| {
                panic!("the-world-loop-fell-over");
            });
            assert!(outcome.is_err(), "the panic must have been caught");
        });

        // Restore the default hook so a later panic elsewhere is not routed
        // into this test's (by then dead) capture.
        let _ = std::panic::take_hook();

        let body = captured.text();
        let panic_lines: Vec<&str> = body.lines().filter(|l| l.contains(PANIC_MARKER)).collect();
        assert_eq!(panic_lines.len(), 1, "exactly one panic event: {body}");
        let line = panic_lines[0];
        assert!(line.contains("ERROR"), "at ERROR: {line}");
        assert!(
            line.contains("the-world-loop-fell-over"),
            "payload text: {line}"
        );
        assert!(line.contains("logging.rs:"), "a location: {line}");
        assert!(line.contains("half=server"), "the role: {line}");
        assert!(line.contains("backtrace="), "a backtrace: {line}");
        assert!(
            *previous_ran.lock().unwrap(),
            "the previously installed hook must still run"
        );
    }

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

        let appender = tracing_appender::rolling::daily(&logs, log_file_name(DEFAULT_ROLE));
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
