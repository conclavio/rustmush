//! **The cockpit picture in the browser** (doc 21 cut G.3b): `mush-pixel`'s
//! `CockpitPicture` built for `wasm32-unknown-unknown` with plain exports and
//! no wasm-bindgen, so the web client draws the picture from its own feeds
//! and the gateway draws nothing for it.
//!
//! **The ABI** (version [`ABI_VERSION`]; `web/src/lib/pixel/wasm.ts` is its
//! one caller). Every argument and result is a number; text crosses through
//! a buffer the module owns:
//!
//! - `mp_abi() -> u32` — the ABI version, checked before anything else.
//! - `mp_input(len) -> ptr` — a buffer of `len` bytes for the caller to write
//!   one feed's JSON into (it lives until the next `mp_input`).
//! - `mp_feed(feed, len) -> u32` — read the buffer's first `len` bytes as a
//!   keyframe of `feed` ([`FEED_COCKPIT`], [`FEED_MAP`]); `1` kept, `0`
//!   refused (not JSON, no such feed). `len == 0` forgets the feed, and a
//!   cockpit keyframe that says `seated: false` forgets the cockpit, as the
//!   gateway's own picture does.
//! - `mp_render(w, h, look_mdeg, hull_mdeg, ordered_mdeg, has_ordered) -> u32`
//!   — draw a `w` x `h` frame (at most [`MAX_SIDE`] a side) for that look;
//!   `1` drawn, `0` nothing to draw (no cockpit held, a bad size).
//! - `mp_pixels() -> ptr` — the last frame as RGBA, `w * h * 4` bytes, for an
//!   `ImageData` over the module's memory without a copy.
//! - `mp_hash_hi() / mp_hash_lo() -> u32` — the last frame's
//!   [`mush_pixel::Frame::hash`], split so no `BigInt` crosses.
//! - `mp_reset()` — a fresh picture (its animation counter from zero), the
//!   feeds kept: what the parity test draws each case from.
//!
//! A buffer the module hands out moves when its memory grows, so the caller
//! takes a fresh view of `memory.buffer` after every call that may allocate.
//!
//! Nothing here dereferences a pointer: the caller writes into, and reads
//! from, `Vec`s this module owns. The `unsafe_code` allowance is for
//! `#[unsafe(no_mangle)]` alone, which the export names need.

#![allow(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

use std::cell::RefCell;
use std::collections::HashMap;

use mush_pixel::Frame;
use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture};
use mush_pixel::picture::{Look, Picture};
use serde_json::Value;

/// The ABI's version: the caller refuses a module that answers another.
pub const ABI_VERSION: u32 = 1;

/// `mp_feed`'s number for `arena.cockpit`.
pub const FEED_COCKPIT: u32 = 1;

/// `mp_feed`'s number for `arena.map`.
pub const FEED_MAP: u32 = 2;

/// The largest frame side drawn, pixels.
pub const MAX_SIDE: u32 = 2048;

/// The module's state: the input buffer, the feeds, the picture, the frame
/// and its RGBA.
struct State {
    input: Vec<u8>,
    feeds: HashMap<String, Value>,
    picture: CockpitPicture,
    frame: Frame,
    rgba: Vec<u8>,
}

impl State {
    fn new() -> State {
        State {
            input: Vec::new(),
            feeds: HashMap::new(),
            picture: CockpitPicture::new(),
            frame: Frame::new(1, 1),
            rgba: Vec::new(),
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn feed_name(feed: u32) -> Option<&'static str> {
    match feed {
        FEED_COCKPIT => Some(COCKPIT_FEED),
        FEED_MAP => Some(ARENA_MAP_FEED),
        _ => None,
    }
}

/// Keep `json` as `feed`'s keyframe (see the crate's ABI): whether it was.
#[must_use]
pub fn feed(feed: u32, json: &[u8]) -> bool {
    let Some(name) = feed_name(feed) else {
        return false;
    };
    with(|s| {
        if json.is_empty() {
            s.feeds.remove(name);
            return true;
        }
        let Ok(value) = serde_json::from_slice::<Value>(json) else {
            return false;
        };
        if name == COCKPIT_FEED && value.get("seated").and_then(Value::as_bool) == Some(false) {
            s.feeds.remove(name);
        } else {
            s.feeds.insert(name.to_string(), value);
        }
        true
    })
}

/// Draw a `w` x `h` frame for the look (see the crate's ABI): whether one
/// was drawn.
#[must_use]
pub fn render(w: u32, h: u32, look: Look) -> bool {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return false;
    }
    let (w, h) = (w as usize, h as usize);
    with(|s| {
        if (s.frame.width(), s.frame.height()) != (w, h) {
            s.frame = Frame::new(w, h);
        }
        let State {
            feeds,
            picture,
            frame,
            rgba,
            ..
        } = s;
        if !picture.draw(frame, feeds, &look) {
            return false;
        }
        rgba.clear();
        rgba.reserve(w * h * 4);
        for px in frame.rgb().as_chunks::<3>().0 {
            rgba.extend_from_slice(px);
            rgba.push(u8::MAX);
        }
        true
    })
}

/// The last frame's hash.
#[must_use]
pub fn hash() -> u64 {
    with(|s| s.frame.hash())
}

/// The last frame as RGBA.
#[must_use]
pub fn pixels() -> Vec<u8> {
    with(|s| s.rgba.clone())
}

/// A fresh picture, the feeds kept.
pub fn reset() {
    with(|s| s.picture = CockpitPicture::new());
}

// ---- the exports -------------------------------------------------------------

/// The ABI version.
#[unsafe(no_mangle)]
pub extern "C" fn mp_abi() -> u32 {
    ABI_VERSION
}

/// A buffer of `len` bytes for the caller to write a feed's JSON into.
#[unsafe(no_mangle)]
pub extern "C" fn mp_input(len: u32) -> *mut u8 {
    with(|s| {
        s.input.clear();
        s.input.resize(len as usize, 0);
        s.input.as_mut_ptr()
    })
}

/// Keep the input buffer's first `len` bytes as `feed`'s keyframe.
#[unsafe(no_mangle)]
pub extern "C" fn mp_feed(which: u32, len: u32) -> u32 {
    let json = with(|s| s.input.get(..len as usize).map(<[u8]>::to_vec));
    u32::from(json.is_some_and(|json| feed(which, &json)))
}

/// Draw a frame for the look.
#[unsafe(no_mangle)]
pub extern "C" fn mp_render(
    w: u32,
    h: u32,
    look_mdeg: i32,
    hull_mdeg: i32,
    ordered_mdeg: i32,
    has_ordered: u32,
) -> u32 {
    let look = Look {
        offset_mdeg: i64::from(look_mdeg),
        hull_offset_mdeg: i64::from(hull_mdeg),
        ordered_mdeg: (has_ordered != 0).then_some(i64::from(ordered_mdeg)),
    };
    u32::from(render(w, h, look))
}

/// The last frame's RGBA.
#[unsafe(no_mangle)]
pub extern "C" fn mp_pixels() -> *const u8 {
    with(|s| s.rgba.as_ptr())
}

/// The last frame's hash, high word.
#[unsafe(no_mangle)]
pub extern "C" fn mp_hash_hi() -> u32 {
    (hash() >> 32) as u32
}

/// The last frame's hash, low word.
#[unsafe(no_mangle)]
pub extern "C" fn mp_hash_lo() -> u32 {
    hash() as u32
}

/// A fresh picture.
#[unsafe(no_mangle)]
pub extern "C" fn mp_reset() {
    reset();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> &'static str {
        match name {
            "cockpit" => include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"),
            "relief" => include_str!("../../../web/src/lib/fixtures/pixel/arena_map_relief.json"),
            _ => include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json"),
        }
    }

    /// The module draws the web golden's frames, natively, through the same
    /// functions its exports call (the browser's half is `pixel.test.ts`).
    #[test]
    fn the_module_draws_the_golden() {
        let golden: Value = serde_json::from_str(include_str!(
            "../../../web/src/lib/fixtures/pixel/cockpit_hashes.json"
        ))
        .unwrap();
        assert!(feed(FEED_COCKPIT, fixture("cockpit").as_bytes()));
        let (w, h) = (
            golden["width"].as_u64().unwrap(),
            golden["height"].as_u64().unwrap(),
        );
        for case in golden["cases"].as_array().unwrap() {
            // Doc 21 cut G.7b: the relief cases draw the heightfield.
            let map = match case["map"].as_str() {
                Some("relief") => fixture("relief").to_string(),
                // The flat case is the raycaster's: no heights.
                _ => {
                    let mut flat: Value = serde_json::from_str(fixture("map")).unwrap();
                    flat.as_object_mut().unwrap().remove("elev");
                    flat.to_string()
                }
            };
            assert!(feed(FEED_MAP, map.as_bytes()));
            reset();
            let look = Look {
                offset_mdeg: case["look_mdeg"].as_i64().unwrap(),
                ..Look::default()
            };
            assert!(render(w as u32, h as u32, look));
            assert_eq!(format!("{:016x}", hash()), case["hash"].as_str().unwrap());
            let px = pixels();
            assert_eq!(px.len() as u64, w * h * 4);
            assert!(px.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "opaque");
        }
    }

    /// Refusals: no cockpit, a bad size, not JSON, no such feed; an unseated
    /// cockpit forgets the held one.
    #[test]
    fn what_cannot_be_drawn_is_not() {
        assert!(feed(FEED_COCKPIT, b""));
        assert!(!render(320, 220, Look::default()), "no cockpit");
        assert!(!feed(FEED_COCKPIT, b"{not json"));
        assert!(!feed(9, b"{}"));
        assert!(feed(FEED_COCKPIT, fixture("cockpit").as_bytes()));
        assert!(!render(0, 220, Look::default()));
        assert!(!render(MAX_SIDE + 1, 220, Look::default()));
        assert!(render(320, 220, Look::default()));
        assert!(feed(FEED_COCKPIT, br#"{"seated": false}"#));
        assert!(!render(320, 220, Look::default()), "the seat went");
        assert_eq!(mp_abi(), ABI_VERSION);
    }
}
