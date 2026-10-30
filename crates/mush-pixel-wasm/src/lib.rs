//! **The pixel engine in the browser** (doc 21 cut G.3b; doc 28 P2):
//! `mush-pixel`'s pictures ([`CockpitPicture`], [`OverheadPicture`],
//! [`Iso25DPicture`], [`ChartPicture`], [`DollPicture`], [`ReplayPicture`])
//! built for `wasm32-unknown-unknown` with plain exports and no wasm-bindgen,
//! so the web client draws every picture from its own feeds and the gateway
//! draws nothing for it.
//!
//! **The ABI** (version [`ABI_VERSION`]; `web/src/lib/pixel/wasm.ts` is its
//! one caller). Every argument and result is a number; text crosses through
//! a buffer the module owns:
//!
//! - `mp_abi() -> u32` — the ABI version, checked before anything else.
//! - `mp_kind(kind) -> u32` — choose which picture `mp_render` draws
//!   ([`KIND_COCKPIT`], [`KIND_OVERHEAD`], [`KIND_CHART`], [`KIND_DOLL`],
//!   [`KIND_REPLAY`], [`KIND_ISO25D`]; [`KIND_COCKPIT`] by default); `1` kept,
//!   `0` refused.
//! - `mp_input(len) -> ptr` — a buffer of `len` bytes for the caller to write
//!   one feed's JSON into (it lives until the next `mp_input`).
//! - `mp_feed(feed, len) -> u32` — read the buffer's first `len` bytes as a
//!   keyframe of `feed` ([`FEED_COCKPIT`], [`FEED_MAP`], [`FEED_CHART`],
//!   [`FEED_REPLAY`], [`FEED_REPLAY_CURSOR`]); `1` kept, `0` refused (not
//!   JSON, no such feed). `len == 0` forgets the feed, and a cockpit keyframe
//!   that says `seated: false` forgets the cockpit.
//! - `mp_render(w, h, look_mdeg, hull_mdeg, ordered_mdeg, has_ordered) -> u32`
//!   — draw a `w` x `h` frame (at most [`MAX_SIDE`] a side) of the chosen
//!   picture for that look; `1` drawn, `0` nothing to draw (missing feed, a
//!   bad size).
//! - `mp_pixels() -> ptr` — the last frame as RGBA, `w * h * 4` bytes, for an
//!   `ImageData` over the module's memory without a copy.
//! - `mp_hash_hi() / mp_hash_lo() -> u32` — the last frame's
//!   [`mush_pixel::Frame::hash`], split so no `BigInt` crosses.
//! - `mp_reset()` — fresh pictures (their animation counters and ground caches
//!   from zero), the feeds and chosen kind kept: what the parity test draws
//!   each case from.
//! - `mp_units(len) -> u32` — read the input buffer's first `len` bytes as one
//!   model's file (doc 31 D-31-2: a one-entry unit pack, a baked model's size
//!   chains), which this module does not embed — the page fetches each model
//!   the first time a unit drawn from it is in a feed (D-31-11) — and add it
//!   to the models the pictures draw from; `1` added, `0` refused (not a
//!   pack this build reads, or a model already installed). Until a model
//!   arrives the units on it draw their look's text sheet.
//! - `mp_wanted() -> len` — the model files the held feeds need and the
//!   module lacks, one file name a line (`models/index.json`'s `file`), into
//!   the output buffer `mp_output()` points at; `0` when nothing is wanted.
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
use mush_pixel::chart::{ChartPicture, SKY_CHART_FEED};
use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture};
use mush_pixel::doll::DollPicture;
use mush_pixel::iso25d::Iso25DPicture;
use mush_pixel::overhead::OverheadPicture;
use mush_pixel::picture::{Look, Picture};
use mush_pixel::replay::{REPLAY_CURSOR_FEED, REPLAY_FEED, ReplayPicture};
use serde_json::Value;

/// The ABI's version: the caller refuses a module that answers another
/// (version 3, doc 31 cut UD.1: `mp_units` takes one model a call and
/// `mp_wanted` names the files to fetch).
pub const ABI_VERSION: u32 = 3;

/// `mp_feed`'s number for `arena.cockpit`.
pub const FEED_COCKPIT: u32 = 1;

/// `mp_feed`'s number for `arena.map`.
pub const FEED_MAP: u32 = 2;

/// `mp_feed`'s number for `sky.chart`.
pub const FEED_CHART: u32 = 3;

/// `mp_feed`'s number for `arena.replay`.
pub const FEED_REPLAY: u32 = 4;

/// `mp_feed`'s number for `replay.cursor`.
pub const FEED_REPLAY_CURSOR: u32 = 5;

/// `mp_kind`'s number for the first-person cockpit ([`CockpitPicture`]).
pub const KIND_COCKPIT: u32 = 1;

/// `mp_kind`'s number for the cockpit's overhead camera ([`OverheadPicture`]).
pub const KIND_OVERHEAD: u32 = 2;

/// `mp_kind`'s number for the orbital sky chart ([`ChartPicture`]).
pub const KIND_CHART: u32 = 3;

/// `mp_kind`'s number for the armour doll page ([`DollPicture`]).
pub const KIND_DOLL: u32 = 4;

/// `mp_kind`'s number for the match replay ([`ReplayPicture`]).
pub const KIND_REPLAY: u32 = 5;

/// `mp_kind`'s number for the cockpit's 2.5D tactical camera ([`Iso25DPicture`]).
pub const KIND_ISO25D: u32 = 6;

/// The largest frame side drawn, pixels.
pub const MAX_SIDE: u32 = 2048;

/// The module's state: the input buffer, the feeds, the pictures, the frame
/// and its RGBA.
struct State {
    input: Vec<u8>,
    feeds: HashMap<String, Value>,
    kind: u32,
    offset_cm: (i64, i64),
    zoom_permille: u16,
    cockpit: CockpitPicture,
    overhead: OverheadPicture,
    iso25d: Iso25DPicture,
    chart: ChartPicture,
    doll: DollPicture,
    replay: ReplayPicture,
    frame: Frame,
    rgba: Vec<u8>,
    output: Vec<u8>,
}

impl State {
    fn new() -> State {
        State {
            input: Vec::new(),
            feeds: HashMap::new(),
            kind: KIND_COCKPIT,
            offset_cm: (0, 0),
            zoom_permille: 1000,
            cockpit: CockpitPicture::new(),
            overhead: OverheadPicture::new(),
            iso25d: Iso25DPicture::new(),
            chart: ChartPicture::new(),
            doll: DollPicture::new(),
            replay: ReplayPicture::new(),
            frame: Frame::new(1, 1),
            rgba: Vec::new(),
            output: Vec::new(),
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
        FEED_CHART => Some(SKY_CHART_FEED),
        FEED_REPLAY => Some(REPLAY_FEED),
        FEED_REPLAY_CURSOR => Some(REPLAY_CURSOR_FEED),
        _ => None,
    }
}

fn valid_kind(kind: u32) -> bool {
    matches!(
        kind,
        KIND_COCKPIT | KIND_OVERHEAD | KIND_CHART | KIND_DOLL | KIND_REPLAY | KIND_ISO25D
    )
}

/// Choose which picture [`render`] draws (see the crate's ABI): whether `kind`
/// is a known picture kind.
#[must_use]
pub fn set_kind(kind: u32) -> bool {
    if !valid_kind(kind) {
        return false;
    }
    with(|s| s.kind = kind);
    true
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

/// Add the model file `pack` to the models drawn (see the crate's ABI,
/// `mp_units`): whether it added one.
#[must_use]
pub fn units(pack: &[u8]) -> bool {
    mush_pixel::bake::install(pack).unwrap_or(false)
}

/// The model files the held feeds need and the module lacks (see the
/// crate's ABI, `mp_wanted`), in the order first needed.
#[must_use]
pub fn wanted() -> Vec<&'static str> {
    with(|s| {
        let mut types = Vec::new();
        let mut names: Vec<&String> = s.feeds.keys().collect();
        names.sort();
        for name in names {
            if let Some(feed) = s.feeds.get(name) {
                mush_pixel::model::unit_types_in(feed, &mut types);
            }
        }
        mush_pixel::model::wanted_files(types)
    })
}

/// Draw a `w` x `h` frame of the chosen picture for the look (see the crate's
/// ABI): whether one was drawn.
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
            kind,
            cockpit,
            overhead,
            iso25d,
            chart,
            doll,
            replay,
            frame,
            rgba,
            ..
        } = s;
        let drawn = match *kind {
            KIND_COCKPIT => cockpit.draw(frame, feeds, &look),
            KIND_OVERHEAD => overhead.draw(frame, feeds, &look),
            KIND_ISO25D => iso25d.draw(frame, feeds, &look),
            KIND_CHART => chart.draw(frame, feeds, &look),
            KIND_DOLL => doll.draw(frame, feeds, &look),
            KIND_REPLAY => replay.draw(frame, feeds, &look),
            _ => false,
        };
        if !drawn {
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

/// Fresh pictures, the feeds and chosen kind kept.
pub fn reset() {
    with(|s| {
        s.offset_cm = (0, 0);
        s.zoom_permille = 1000;
        s.cockpit = CockpitPicture::new();
        s.overhead = OverheadPicture::new();
        s.iso25d = Iso25DPicture::new();
        s.chart = ChartPicture::new();
        s.doll = DollPicture::new();
        s.replay = ReplayPicture::new();
    });
}

// ---- the exports -------------------------------------------------------------

/// The ABI version.
#[unsafe(no_mangle)]
pub extern "C" fn mp_abi() -> u32 {
    ABI_VERSION
}

/// Choose which picture `mp_render` draws.
#[unsafe(no_mangle)]
pub extern "C" fn mp_kind(kind: u32) -> u32 {
    u32::from(set_kind(kind))
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

/// Add the input buffer's first `len` bytes as one model's file.
#[unsafe(no_mangle)]
pub extern "C" fn mp_units(len: u32) -> u32 {
    let pack = with(|s| s.input.get(..len as usize).map(<[u8]>::to_vec));
    u32::from(pack.is_some_and(|pack| units(&pack)))
}

/// Write the wanted model files, one a line, into the output buffer: its
/// length.
#[unsafe(no_mangle)]
pub extern "C" fn mp_wanted() -> u32 {
    let text = wanted().join("\n");
    with(|s| {
        s.output = text.into_bytes();
        u32::try_from(s.output.len()).unwrap_or(0)
    })
}

/// The output buffer `mp_wanted` wrote.
#[unsafe(no_mangle)]
pub extern "C" fn mp_output() -> *const u8 {
    with(|s| s.output.as_ptr())
}

/// Set the dead-reckoned ego position offset `(dx_cm, dy_cm)` from the held
/// keyframe's `(x_cm, y_cm)`.
#[unsafe(no_mangle)]
pub extern "C" fn mp_offset_cm(dx_cm: i32, dy_cm: i32) {
    with(|s| s.offset_cm = (i64::from(dx_cm), i64::from(dy_cm)));
}

/// Set the 2.5D tactical camera zoom in permille (`500..=2000`; `1000` = `1.0×`).
#[unsafe(no_mangle)]
pub extern "C" fn mp_zoom_permille(zoom_permille: u32) {
    let clamped = u16::try_from(zoom_permille.clamp(500, 2000)).unwrap_or(1000);
    with(|s| s.zoom_permille = clamped);
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
    let ((dx_cm, dy_cm), zoom_permille) = with(|s| (s.offset_cm, s.zoom_permille));
    let look = Look {
        offset_mdeg: i64::from(look_mdeg),
        hull_offset_mdeg: i64::from(hull_mdeg),
        ordered_mdeg: (has_ordered != 0).then_some(i64::from(ordered_mdeg)),
        dx_cm,
        dy_cm,
        zoom_permille,
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
        install_shipped_pack();
        let golden: Value = serde_json::from_str(include_str!(
            "../../../web/src/lib/fixtures/pixel/cockpit_hashes.json"
        ))
        .unwrap();
        assert!(set_kind(KIND_COCKPIT));
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

    /// Doc 28 P2: the module draws the overhead golden's eight frames hash for
    /// hash (`mush-pixel/tests/golden/overhead_hashes.json`).
    #[test]
    fn the_module_draws_the_overhead_golden() {
        install_shipped_pack();
        let golden: Value = serde_json::from_str(include_str!(
            "../../mush-pixel/tests/golden/overhead_hashes.json"
        ))
        .unwrap();
        assert!(set_kind(KIND_OVERHEAD));
        assert!(feed(FEED_COCKPIT, fixture("cockpit").as_bytes()));
        for case in golden["cases"].as_array().unwrap() {
            let map = match case["map"].as_str() {
                Some("relief") => fixture("relief").to_string(),
                _ => {
                    let mut flat: Value = serde_json::from_str(fixture("map")).unwrap();
                    flat.as_object_mut().unwrap().remove("elev");
                    flat.to_string()
                }
            };
            assert!(feed(FEED_MAP, map.as_bytes()));
            reset();
            let size = case["size"].as_array().unwrap();
            let (w, h) = (size[0].as_u64().unwrap(), size[1].as_u64().unwrap());
            let offset_mdeg = case["look_mdeg"].as_i64().unwrap();
            let look = if offset_mdeg == 0 {
                Look::default()
            } else {
                Look {
                    offset_mdeg,
                    hull_offset_mdeg: 10_000,
                    ordered_mdeg: Some(90_000),
                    ..Look::default()
                }
            };
            assert!(render(w as u32, h as u32, look));
            assert_eq!(format!("{:016x}", hash()), case["hash"].as_str().unwrap());
            let px = pixels();
            assert_eq!(px.len() as u64, w * h * 4);
            assert!(px.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "opaque");
        }
        assert!(set_kind(KIND_COCKPIT));
    }

    /// Doc 28 P2: the remaining bucket-B pictures (`ChartPicture`, `DollPicture`,
    /// `ReplayPicture`) draw through the WASM ABI and match native `mush-pixel`
    /// hash for hash.
    #[test]
    fn the_module_draws_the_bucket_b_pictures() {
        install_shipped_pack();
        // DollPicture over the committed cockpit fixture.
        assert!(set_kind(KIND_DOLL));
        assert!(feed(FEED_COCKPIT, fixture("cockpit").as_bytes()));
        assert!(render(640, 480, Look::default()));
        let mut native = Frame::new(640, 480);
        let feeds: HashMap<String, Value> = [(
            COCKPIT_FEED.to_string(),
            serde_json::from_str(fixture("cockpit")).unwrap(),
        )]
        .into_iter()
        .collect();
        assert!(DollPicture::new().draw(&mut native, &feeds, &Look::default()));
        assert_eq!(hash(), native.hash());

        // ChartPicture: requires `sky.chart`, and draws the refusal card when the
        // chart block is absent.
        assert!(set_kind(KIND_CHART));
        assert!(feed(FEED_CHART, b""));
        assert!(!render(640, 480, Look::default()), "no chart feed");
        assert!(feed(FEED_CHART, br#"{"refused": true}"#));
        assert!(render(640, 480, Look::default()));
        let chart_feeds: HashMap<String, Value> = [(
            SKY_CHART_FEED.to_string(),
            serde_json::json!({"refused": true}),
        )]
        .into_iter()
        .collect();
        let mut native_chart = Frame::new(640, 480);
        assert!(ChartPicture::new().draw(&mut native_chart, &chart_feeds, &Look::default()));
        assert_eq!(hash(), native_chart.hash());

        // ReplayPicture: draws its waiting card before a document arrives, and
        // accepts `arena.replay` + `replay.cursor`.
        assert!(set_kind(KIND_REPLAY));
        assert!(feed(FEED_REPLAY, b""));
        assert!(feed(
            FEED_REPLAY_CURSOR,
            br#"{"match": 1, "t_ms": 0, "focus": 0}"#
        ));
        assert!(render(640, 480, Look::default()));
        let replay_feeds: HashMap<String, Value> = [(
            REPLAY_CURSOR_FEED.to_string(),
            serde_json::json!({"match": 1, "t_ms": 0, "focus": 0}),
        )]
        .into_iter()
        .collect();
        let mut native_replay = Frame::new(640, 480);
        assert!(ReplayPicture::new().draw(&mut native_replay, &replay_feeds, &Look::default()));
        assert_eq!(hash(), native_replay.hash());

        // Iso25DPicture: draws the 2.5D tactical view and matches native `Iso25DPicture`.
        assert!(set_kind(KIND_ISO25D));
        assert!(feed(FEED_COCKPIT, fixture("cockpit").as_bytes()));
        assert!(feed(FEED_MAP, fixture("relief").as_bytes()));
        reset();
        assert!(render(640, 480, Look::default()));
        let iso_feeds: HashMap<String, Value> = [
            (
                COCKPIT_FEED.to_string(),
                serde_json::from_str(fixture("cockpit")).unwrap(),
            ),
            (
                ARENA_MAP_FEED.to_string(),
                serde_json::from_str(fixture("relief")).unwrap(),
            ),
        ]
        .into_iter()
        .collect();
        let mut native_iso = Frame::new(640, 480);
        assert!(Iso25DPicture::new().draw(&mut native_iso, &iso_feeds, &Look::default()));
        assert_eq!(hash(), native_iso.hash());

        assert!(set_kind(KIND_COCKPIT));
    }

    /// Refusals: no cockpit, a bad size, not JSON, no such feed, no such kind;
    /// an unseated cockpit forgets the held one.
    #[test]
    fn what_cannot_be_drawn_is_not() {
        assert!(!set_kind(0));
        assert!(!set_kind(99));
        assert_eq!(mp_kind(0), 0);
        assert_eq!(mp_kind(KIND_COCKPIT), 1);
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

    /// The model files the page fetches, read from the tree.
    fn shipped_models() -> Vec<Vec<u8>> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../mush-pixel/models");
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        files
            .iter()
            .filter(|p| p.extension().is_some_and(|x| x == "mupk"))
            .filter_map(|p| std::fs::read(p).ok())
            .collect()
    }

    /// Install every shipped model, as the page does as each is needed, so
    /// these tests draw what the native goldens draw (a host test run gets
    /// `encode` by feature unification and embeds them already).
    fn install_shipped_pack() {
        for model in shipped_models() {
            let _ = units(&model);
        }
    }

    /// The models are fetched, not embedded (D-31-3, D-31-11): the module
    /// refuses what is not a pack, adds a model once, and names the files the
    /// held feeds still need. The wasm32 build (mush-pixel without `encode`)
    /// embeds none; a host test run inside the workspace gets `encode` by
    /// feature unification and embeds them, so only the target build is held
    /// to that.
    #[test]
    fn the_models_are_installed_not_embedded() {
        if cfg!(target_arch = "wasm32") {
            assert!(
                mush_pixel::model::embedded_files().is_empty(),
                "the web build embeds no model"
            );
        }
        assert!(!units(b"not a pack"));
        let models = shipped_models();
        assert_eq!(models.len(), mush_pixel::model::index().len());
        install_shipped_pack();
        for model in &models {
            assert!(!units(model), "installed once");
        }
        assert!(feed(FEED_COCKPIT, fixture("cockpit").as_bytes()));
        assert!(wanted().is_empty(), "every model on the map is installed");
        assert_eq!(mp_wanted(), 0);
        assert_eq!(mp_units(u32::MAX), 0, "past the input buffer");
    }
}
