//! An external player: production dependencies contain no game engine, store,
//! gateway, or privileged protocol. All world interaction is SSH terminal text.
//!
//! This agent intentionally develops independently from in-game NPCs and
//! `mush-llm`. Its model client, decisions, memory and runner belong to this tool;
//! shared behavior is compared through the game interface, not shared planning.
pub mod activity;
pub mod analysis;
pub mod arena;
pub mod attention;
pub mod blind_review;
pub mod budget;
pub mod campaign;
pub mod campaign_cli;
pub mod cast_debug;
pub mod character;
pub mod checkpoint;
pub mod comparison;
pub mod composition;
pub mod config;
pub mod contracts;
pub mod conversation;
pub mod costs;
pub mod debugger;
pub mod evidence;
pub mod experiment;
pub mod floor;
pub mod harness;
pub mod knowledge;
pub mod lab_cli;
pub mod learning;
pub mod memory;
pub mod mine;
pub mod mission;
pub mod model;
pub mod models;
pub mod navigation;
pub mod organization;
pub mod playtest;
pub mod probe;
pub mod regression;
pub mod replay;
pub mod reproduction;
pub mod runner;
pub mod scene;
pub mod script;
pub mod social;
pub mod studio;
pub mod studio_runtime;
pub mod studio_tui;
pub mod tape;
pub mod terminal;
pub mod trace;
pub mod trace_tui;
pub mod transport;
pub mod tui;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

/// Keep the newest UTF-8 text without cutting a character in half.
pub fn tail(text: &str, limit: usize) -> &str {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Create a directory (non-recursive) with restrictive owner-only permissions
/// (`0o700` on Unix) so local bot state and logs are not world-readable (P2-TSC-30).
pub fn create_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::DirBuilder::new().create(path)
    }
}

/// Recursively create directories with restrictive owner-only permissions
/// (`0o700` on Unix) for any newly created segments (P2-TSC-30).
pub fn create_private_dir_all(path: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Return [`std::fs::OpenOptions`] preconfigured with owner-only creation mode
/// (`0o600` on Unix) for private journals, transcripts, and reports (P2-TSC-30).
pub fn private_open_options() -> std::fs::OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.mode(0o600);
        opts
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
    }
}

/// Atomically create a brand-new file (`create_new(true)`) with owner-only
/// permissions (`0o600` on Unix; P2-TSC-30).
pub fn create_new_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    private_open_options()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Create or truncate a file with owner-only permissions (`0o600` on Unix; P2-TSC-30).
pub fn create_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    private_open_options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_fs_helpers_create_dirs_and_files_safely() {
        let base = std::env::temp_dir().join(format!(
            "mush-bot-privfs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let nested = base.join("a/b");
        create_private_dir_all(&nested).expect("create_private_dir_all");
        let child_dir = base.join("single");
        create_private_dir(&child_dir).expect("create_private_dir");
        let new_file = child_dir.join("report.json");
        create_new_private_file(&new_file).expect("create_new_private_file");
        assert!(create_new_private_file(&new_file).is_err());
        let trunc_file = child_dir.join("stderr.txt");
        create_private_file(&trunc_file).expect("create_private_file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&child_dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&nested).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&new_file).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&trunc_file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
