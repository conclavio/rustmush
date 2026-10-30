//! `mush-pixel` — the graphics engine (doc 21), grown from the pixel cockpit
//! spike of 2026-10-21.
//!
//! One [`Frame`], drawn by a [`picture::Picture`] from the per-observer
//! feeds a renderer already holds, and shown where the client runs: streamed
//! to an SSH terminal over the kitty graphics protocol or as DEC sixel, sent
//! as a PNG over the web gateway's socket fallback (G.3a/G.3c), or drawn
//! locally into an HTML canvas by the browser's WebAssembly module
//! (`mush-pixel-wasm`, G.3b). Both bucket-A cameras (first-person cockpit and
//! overhead) and bucket-B diagrams (orbital chart, armour doll, match replay)
//! share the same [`picture`] seam. The pieces, each pure and testable on its
//! own:
//!
//! - [`picture`]: the seam a renderer drives — a producer with its
//!   instruments and its clock, reading feeds by name.
//! - [`caps`]: what the terminal proved (the DA1 attributes, the graphics
//!   record) and the encoder that follows from it.
//! - [`heightfield`]: the voxel-space producer for ground with relief
//!   (G.7a/G.7b, the cockpit's default where the map carries heights), with
//!   the other units as distance-scaled billboard sprites.
//! - [`raycast`]: a Wolfenstein-style software raycaster over a flat 2D
//!   arena grid (open / wall / woods / water / road) and the shared billboard
//!   sprite renderer, drawing into an RGB [`Frame`].
//! - [`png`]: a minimal PNG encoder (palette when the frame has at most 256
//!   colours, which the raycaster guarantees by construction; truecolor
//!   otherwise). IDAT is zlib via `flate2`.
//! - [`heightfield`]: the voxel-space producer for ground with relief
//!   (G.7a), beside the raycaster.
//! - [`chart`]: the orbital view, `sky.chart` drawn as a scene (G.5b).
//! - [`doll`]: the armour doll as a page with a legend (G.6a).
//! - [`replay`]: a finished match's `arena.replay` drawn as a scene over its
//!   ground, with a timeline (G.8).
//! - [`overhead`]: the cockpit's second camera, the arena from above
//!   north up around the pilot's own tank (G.12b), and the same scene
//!   small as the flight HUD's inset, the picture's heading up (G.13).
//! - [`scene`]: bucket B's retained 2D scene and its one rasteriser (G.5).
//! - [`sixel`]: the same frame as a DEC sixel stream (G.2), for the
//!   terminals that draw sixel and not kitty — Windows Terminal above all.
//! - [`kitty`]: the kitty graphics protocol framing — the capability probe,
//!   a PNG frame as chunked inline transmission with a fixed image id and
//!   placement, wrapped in synchronized output.
//! - [`look`]: a unit type's look (doc 22): its name, its doll grid, its
//!   sprite sheet, kept apart from its rules.
//! - [`bake`]: the bake matrix (doc 22 D-22-10): each shipped look's `fp`
//!   and `iso` size chains, reduced from one 384 px render, in a named-entry
//!   pack with its provenance; the size and chain picks; the text-sheet
//!   fallback.
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

pub mod bake;
#[cfg(feature = "encode")]
pub mod caps;
pub mod chart;
pub mod cockpit;
pub mod design;
pub mod doll;
pub mod frame;
pub mod heightfield;
pub mod iso25d;
#[cfg(feature = "encode")]
pub mod kitty;
pub mod look;
pub mod math;
pub mod model;
pub mod overhead;
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
