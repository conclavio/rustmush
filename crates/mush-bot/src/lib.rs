//! An external player: production dependencies contain no game engine, store,
//! gateway, or privileged protocol. All world interaction is SSH terminal text.
//!
//! This agent intentionally develops independently from in-game NPCs and
//! `mush-llm`. Its model client, decisions, memory and runner belong to this tool;
//! shared behavior is compared through the game interface, not shared planning.
pub mod analysis;
pub mod attention;
pub mod budget;
pub mod cast_debug;
pub mod character;
pub mod comparison;
pub mod composition;
pub mod config;
pub mod contracts;
pub mod costs;
pub mod debugger;
pub mod experiment;
pub mod harness;
pub mod knowledge;
pub mod memory;
pub mod model;
pub mod models;
pub mod organization;
pub mod probe;
pub mod regression;
pub mod replay;
pub mod runner;
pub mod script;
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
