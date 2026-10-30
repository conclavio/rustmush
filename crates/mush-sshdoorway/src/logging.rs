//! Logging (doc 29 §8.1, D-29-18).
//!
//! - **File.** `[log].file_level` and above to `<file_dir>/<generation>.log`:
//!   one stable name per generation, so logrotate can rename it, and
//!   reopened on `SIGUSR2` (logrotate's `postrotate`) so no line is lost to
//!   `copytruncate`. Written through tracing-appender's lossy non-blocking
//!   writer, so a full disk or a log storm drops lines instead of stalling
//!   connections.
//! - **journald.** `[log].journal_level` and above (default `warn`) to
//!   stderr, so `systemctl status` shows trouble without per-connection lines
//!   flooding the box's journal. The unit rate-limits it as well.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, fmt};

use crate::config::LogCfg;

/// A log file that can be reopened in place.
#[derive(Clone, Debug)]
pub struct ReopenableFile {
    path: PathBuf,
    file: Arc<Mutex<Option<File>>>,
}

fn open_append(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o640);
    }
    opts.open(path)
}

impl ReopenableFile {
    /// Open (or create) `path` for appending.
    pub fn open(path: &Path) -> std::io::Result<ReopenableFile> {
        Ok(ReopenableFile {
            path: path.to_owned(),
            file: Arc::new(Mutex::new(Some(open_append(path)?))),
        })
    }

    /// Close and reopen the file at the same path (after logrotate renamed
    /// it). On failure the writer keeps the old handle.
    pub fn reopen(&self) -> std::io::Result<()> {
        let fresh = open_append(&self.path)?;
        let mut slot = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(fresh);
        Ok(())
    }

    /// The path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Write for ReopenableFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut slot = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match slot.as_mut() {
            Some(f) => f.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut slot = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match slot.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

/// Keeps the non-blocking writer's worker alive; drop it last.
#[derive(Debug)]
pub struct LogGuard {
    _guard: Option<tracing_appender::non_blocking::WorkerGuard>,
    /// The file sink, for `SIGUSR2`.
    pub file: Option<ReopenableFile>,
}

/// Install the global subscriber.
pub fn init(cfg: &LogCfg, generation: &str) -> Result<LogGuard, String> {
    let journal_level = LevelFilter::from_str(&cfg.journal_level)
        .map_err(|e| format!("[log].journal_level: {e}"))?;
    let file_level =
        LevelFilter::from_str(&cfg.file_level).map_err(|e| format!("[log].file_level: {e}"))?;
    let journal = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_target(false)
        .with_filter(journal_level);
    let (file_layer, guard, file) = match &cfg.file_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = dir.join(format!("{generation}.log"));
            let file =
                ReopenableFile::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let (writer, guard) = tracing_appender::non_blocking(file.clone());
            let layer = fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
                .with_filter(file_level);
            (Some(layer), Some(guard), Some(file))
        }
        None => (None, None, None),
    };
    tracing_subscriber::registry()
        .with(journal)
        .with(file_layer)
        .try_init()
        .map_err(|e| e.to_string())?;
    Ok(LogGuard {
        _guard: guard,
        file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopen_follows_a_rename() {
        let dir = std::env::temp_dir().join(format!(
            "sshdoorway-log-{}-{}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.log");
        let mut w = ReopenableFile::open(&path).unwrap();
        w.write_all(b"one\n").unwrap();
        let rotated = dir.join("a.log.1");
        // Windows cannot rename an open file; logrotate runs on Linux.
        if std::fs::rename(&path, &rotated).is_ok() {
            w.reopen().unwrap();
            w.write_all(b"two\n").unwrap();
            w.flush().unwrap();
            assert_eq!(std::fs::read_to_string(&rotated).unwrap(), "one\n");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
