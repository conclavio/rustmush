//! Debug-build process-termination hooks for the K8 crash matrix.
//!
//! The hooks are inert unless both environment variables name an exact cut
//! and a one-shot marker path. They are compiled out of release builds. A
//! marker is created with `create_new`, so a supervised replacement process
//! can inherit the same environment without crashing a second time.

#[cfg(debug_assertions)]
use std::io::Write as _;
#[cfg(debug_assertions)]
use std::path::PathBuf;
#[cfg(debug_assertions)]
use std::time::Duration;

#[cfg(debug_assertions)]
const CUT_ENV: &str = "RUSTMUSH_K8_PROCESS_CUT";
#[cfg(debug_assertions)]
const MARKER_ENV: &str = "RUSTMUSH_K8_PROCESS_CUT_MARKER";
#[cfg(debug_assertions)]
const RELEASE_ENV: &str = "RUSTMUSH_K8_PROCESS_CUT_RELEASE";

/// Terminate this debug server process at one exact, externally armed cut.
///
/// When a release path is configured the hook first publishes the marker and
/// waits. That lets the parent prove an externally visible boundary (such as
/// the IPC `Owned` frame) before releasing the crash while the persistence
/// task remains stopped before its next operation.
pub(crate) async fn terminate_at(point: &'static str) {
    #[cfg(debug_assertions)]
    {
        if std::env::var(CUT_ENV).ok().as_deref() != Some(point) {
            return;
        }
        let Some(marker) = std::env::var_os(MARKER_ENV).map(PathBuf::from) else {
            return;
        };
        let Ok(mut file) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        else {
            return;
        };
        let _ = writeln!(file, "point={point} pid={}", std::process::id());
        let _ = file.sync_all();
        tracing::warn!(point, marker = %marker.display(), "K8 process cut armed");

        if let Some(release) = std::env::var_os(RELEASE_ENV).map(PathBuf::from) {
            while !release.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        std::process::abort();
    }

    #[cfg(not(debug_assertions))]
    let _ = point;
}
