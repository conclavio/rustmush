//! A scratch directory that no clock names (doc 51 rule T).
//!
//! A rig that boots a real server needs a data directory of its own. The
//! helper every suite copied named it by the process id and the wall clock's
//! nanoseconds, and that read of the wall clock is one the `wait_budget` gate
//! counts. The process id and a counter are enough: no two live processes
//! share an id, and one process counts its own directories.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// How many directories this process has asked for.
static MADE: AtomicU64 = AtomicU64::new(0);

/// **A fresh, empty directory** under the system's temporary directory, named
/// `<suite>-<tag>-<process id>-<count>`.
///
/// `suite` says whose it is (`mush-icslice`), and `tag` which rig's within
/// the suite. The name is this process's alone while it runs. A directory an
/// earlier process left under the same name is removed first, so a stale tree
/// never reaches a story. The caller removes the directory when its rig drops.
///
/// # Errors
///
/// When the directory cannot be created.
pub fn scratch_dir(suite: &str, tag: &str) -> std::io::Result<PathBuf> {
    let count = MADE.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{suite}-{tag}-{}-{count}", std::process::id()));
    // A leftover from a process that had this id before. Absent is the usual case.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_directories_of_one_tag_are_two_directories() {
        let first = scratch_dir("mush-storykit", "same").unwrap();
        let second = scratch_dir("mush-storykit", "same").unwrap();
        assert_ne!(first, second);
        assert!(first.is_dir() && second.is_dir());
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with(&format!("mush-storykit-same-{}-", std::process::id())),
            "{name}"
        );
        for dir in [first, second] {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn a_new_directory_is_empty() {
        let dir = scratch_dir("mush-storykit", "empty").unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
