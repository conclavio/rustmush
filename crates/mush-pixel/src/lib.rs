//! `mush-pixel` — the graphics engine (doc 21), grown from the pixel cockpit
//! spike of 2026-10-21.
//!
//! One [`Frame`], drawn by a [`picture::Picture`] from the per-observer
//! feeds a renderer already holds, and encoded for what the client proved it
//! can show. Today: a first-person view of the arena streamed to a terminal
//! that speaks the kitty graphics protocol or draws sixel. The pieces, each pure and
//! testable on its own:
//!
//! - [`picture`]: the seam a renderer drives — a producer with its
//!   instruments and its clock, reading feeds by name.
//! - [`caps`]: what the terminal proved (the DA1 attributes, the graphics
//!   record) and the encoder that follows from it.
//! - [`raycast`]: a Wolfenstein-style software raycaster over the arena's
//!   2D cell grid (open / wall / woods / water / road), with the other units
//!   as distance-scaled billboard sprites, drawing into an RGB [`Frame`].
//! - [`png`]: a minimal PNG encoder (palette when the frame has at most 256
//!   colours, which the raycaster guarantees by construction; truecolor
//!   otherwise). IDAT is zlib via `flate2`.
//! - [`heightfield`]: the voxel-space producer for ground with relief
//!   (G.7a), beside the raycaster.
//! - [`chart`]: the orbital view, `sky.chart` drawn as a scene (G.5b).
//! - [`doll`]: the armour doll as a page with a legend (G.6a).
//! - [`replay`]: a finished match's `arena.replay` drawn as a scene over its
//!   ground, with a timeline (G.8).
//! - [`scene`]: bucket B's retained 2D scene and its one rasteriser (G.5).
//! - [`sixel`]: the same frame as a DEC sixel stream (G.2), for the
//!   terminals that draw sixel and not kitty — Windows Terminal above all.
//! - [`kitty`]: the kitty graphics protocol framing — the capability probe,
//!   a PNG frame as chunked inline transmission with a fixed image id and
//!   placement, wrapped in synchronized output.
//! - [`look`]: a unit type's look (doc 22): its name, its doll grid, its
//!   sprite sheet, kept apart from its rules.
//! - [`overlay`]: the instruments drawn into the frame over the picture —
//!   minimap, heat gauge, weapons, the armour doll, the status strip — from
//!   a small fixed palette so the PNG stays indexed.
//! - [`cockpit`]: the cockpit [`picture::Picture`] — the `arena.cockpit`
//!   and `arena.map` keyframes read into the raycaster's map, camera and
//!   sprites and the instruments' inputs.
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

#[cfg(feature = "encode")]
pub mod caps;
pub mod chart;
pub mod cockpit;
pub mod doll;
pub mod frame;
pub mod heightfield;
#[cfg(feature = "encode")]
pub mod kitty;
pub mod look;
pub mod math;
pub mod overlay;
pub mod picture;
#[cfg(feature = "encode")]
pub mod png;
pub mod raycast;
pub mod replay;
pub mod scene;
#[cfg(feature = "encode")]
pub mod sixel;

pub use frame::Frame;
