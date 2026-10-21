//! `mush-pixel` — the pixel cockpit spike (2026-10-21).
//!
//! A first-person view of the arena streamed to a terminal that speaks the
//! kitty graphics protocol. Three pieces, each pure and testable on its own:
//!
//! - [`raycast`]: a Wolfenstein-style software raycaster over the arena's
//!   2D cell grid (open / wall / woods / water / road), with the other units
//!   as distance-scaled billboard sprites, drawing into an RGB [`Frame`].
//! - [`png`]: a minimal PNG encoder (palette when the frame has at most 256
//!   colours, which the raycaster guarantees by construction; truecolor
//!   otherwise). IDAT is zlib via `flate2`.
//! - [`kitty`]: the kitty graphics protocol framing — the capability probe,
//!   a PNG frame as chunked inline transmission with a fixed image id and
//!   placement, wrapped in synchronized output.
//!
//! Nothing here touches a terminal, a channel or the world: the SSH TUI owns
//! the camera clock, the probe's state machine and the write.
//!
//! Born with all six panic-shaped restriction lints denied (WS01-S12; see
//! `crates/mush-contract-tests/tests/restriction_lints.rs`): a frame that
//! cannot be drawn is a frame that is not sent, never a dead connection.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod frame;
pub mod kitty;
pub mod png;
pub mod raycast;

pub use frame::Frame;
