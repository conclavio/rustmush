//! The `draining-<gen>` marker (29a §8 C-1).
//!
//! A draining generation leaves a marker, so that nothing but the swap
//! script ever brings it back into service beside its successor: not
//! `Restart=on-failure` after a crash, not `systemctl restart` (needrestart),
//! not a reboot. The process never removes it. A start that finds it reports
//! `READY=1` and exits 0, which systemd counts as a clean run: an exit
//! *before* `READY=1` under `Type=notify` is a protocol failure, and would be
//! restarted into service. The swap script clears the marker right before it
//! starts that generation.
//!
//! Under systemd the marker lives in `$STATE_DIRECTORY` (`StateDirectory=`),
//! the directory the swap script reads; elsewhere, in the daemon's fallback
//! directory (the doorway's: beside its host key).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The rule for `--generation`: 1-16 lowercase letters or digits, so the
/// marker's name and the unit instance are the same safe token.
pub fn check_generation(generation: &str) -> Result<(), String> {
    if generation.is_empty()
        || generation.len() > 16
        || !generation
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err("--generation must be 1-16 lowercase letters or digits".into());
    }
    Ok(())
}

/// The marker's file name for one generation.
#[must_use]
pub fn file_name(generation: &str) -> String {
    format!("draining-{generation}")
}

/// The first `STATE_DIRECTORY` entry (systemd separates several with `:`),
/// else `fallback`.
#[must_use]
pub fn dir(state_directory: Option<&OsStr>, fallback: &Path) -> PathBuf {
    state_directory
        .map(|v| PathBuf::from(v.to_string_lossy().split(':').next().unwrap_or_default()))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| fallback.to_path_buf())
}

/// This generation's marker: in `$STATE_DIRECTORY` under systemd, else in
/// `fallback`.
#[must_use]
pub fn path(generation: &str, fallback: &Path) -> PathBuf {
    dir(std::env::var_os("STATE_DIRECTORY").as_deref(), fallback).join(file_name(generation))
}

/// Leave the marker (a drain has begun). Logged, never fatal: a marker that
/// cannot be written costs a crash-restarted generation its refusal, not the
/// drain.
pub fn leave(marker: &Path) {
    if let Err(e) = std::fs::write(marker, b"draining\n") {
        tracing::error!(error = %e, marker = %marker.display(), "cannot write the drain marker");
    }
}

/// The start-time rule. When the marker exists: log why, send `READY=1` (so
/// the exit that follows is a clean run in systemd's eyes, and
/// `Restart=on-failure` leaves it be) and return `true`, on which the caller
/// exits 0 without serving.
#[must_use]
pub fn found(marker: &Path) -> bool {
    if !marker.exists() {
        return false;
    }
    tracing::warn!(
        marker = %marker.display(),
        "this generation drained and was not swapped back in; not serving (swap.sh clears the marker)"
    );
    crate::notify::ready();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_lives_in_the_state_directory_when_systemd_gives_one() {
        let beside = Path::new("/etc/rustmush");
        assert_eq!(
            dir(Some(OsStr::new("/var/lib/sshdoorway")), beside),
            PathBuf::from("/var/lib/sshdoorway")
        );
        // Several directories: the first one.
        assert_eq!(
            dir(
                Some(OsStr::new("/var/lib/sshdoorway:/var/lib/other")),
                beside
            ),
            PathBuf::from("/var/lib/sshdoorway")
        );
    }

    #[test]
    fn the_marker_lives_in_the_fallback_otherwise() {
        let beside = Path::new("/etc/rustmush");
        assert_eq!(dir(None, beside), PathBuf::from("/etc/rustmush"));
        assert_eq!(
            dir(Some(OsStr::new("")), beside),
            PathBuf::from("/etc/rustmush")
        );
        assert_eq!(dir(None, Path::new("")), PathBuf::new());
    }

    #[test]
    fn the_marker_is_named_by_its_generation() {
        assert_eq!(file_name("a"), "draining-a");
        assert_eq!(
            dir(None, Path::new("/x")).join(file_name("b")),
            PathBuf::from("/x/draining-b")
        );
    }

    #[test]
    fn generation_names_are_short_safe_tokens() {
        for ok in ["a", "b", "gen2", "0123456789abcdef"] {
            assert!(check_generation(ok).is_ok(), "{ok:?}");
        }
        for bad in ["", "A", "a-b", "a b", "0123456789abcdefg", "é"] {
            assert!(check_generation(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_found_marker_refuses_and_is_never_removed() {
        let dir = std::env::temp_dir().join(format!("mush-drain-marker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join(file_name("a"));
        let _ = std::fs::remove_file(&marker);
        assert!(!found(&marker));
        leave(&marker);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "draining\n");
        assert!(found(&marker));
        // Still there: only the swap script clears it.
        assert!(marker.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
