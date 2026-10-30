//! **The bake matrix** (doc 22 D-22-10, cut U.6): a shipped unit's art as two
//! views (`fp` and `iso`) × eight facings × six poses × a size chain each,
//! reduced from one 384 px render of one named asset by one script, and
//! stored in a named-entry pack keyed by **model id** with its provenance
//! (doc 31 D-31-1: a look names the model it is drawn from).
//!
//! - **The views.** `fp` is baked at a 3° pitch (Q-22-3): the heightfield's
//!   camera is level, so the only pitch to a contact is its line's own
//!   elevation. `iso` is baked at the 2.5D's fixed 28°. The raycaster draws
//!   `fp`; the heightfield draws the chain whose pitch is nearer the line to
//!   the contact — `fp` within [`CROSSOVER_DEG`] of depression, `iso` beyond;
//!   the 2.5D draws `iso` ([`View::for_depression`]).
//! - **The chains.** `fp`: 96, 64, 48, 32, 24, 16, 12, 8 px ([`FP_SIZES`]);
//!   `iso`: 96, 80, 64, 56, 48, 40, 32, 24 px ([`ISO_SIZES`]). Every size is
//!   reduced from the 384 px render, never from a smaller size (rule 1;
//!   `bake::reduce`, the only resampler). A contact draws from the size
//!   nearest above its projected height ([`Chain::pick`]).
//! - **The pack** (format version [`FORMAT_VERSION`], magic [`FORMAT_MAGIC`]):
//!   named entries keyed by model id (`mc.st.tank_lvl2`, …; the catalogue is
//!   [`crate::model`]), each with its provenance (asset pack and version,
//!   prefab, clips, the bake and reduce commits, rule 2), its `size_cm`, and
//!   per view a frame span and the chain. A frame is its tight bounding box,
//!   its anchor, its height and its muzzles, and its pixels run-length coded
//!   (rule 7, [`encode_runs`]). A shipped model is **one file of one entry**
//!   (doc 31 D-31-2), `models/<id>.<sha-256 prefix>.mupk`.
//! - **The fallback** (rule 3): a look whose model is not installed draws its
//!   text sheet ([`Look::sheet`]: its own, else its model's), nearest-sampled
//!   — the raycaster's own sheet draw in the first-person pictures, and
//!   [`chain_from_sheet`] in the 2.5D.
//!
//! **Where the models come from** ([`model`], [`install`]). Native builds
//! embed every shipped model file (the build script lists `models/`), so the
//! gateway never waits on one (D-31-3); the web's WebAssembly build (no
//! `encode` feature) embeds none, and the page fetches each model's file the
//! first time a unit drawn from it is in a feed (D-31-11) and hands it to
//! [`install`], which adds it to the table the pictures read. A model that
//! arrives supersedes its fallback on the next frame.

use std::sync::{OnceLock, PoisonError, RwLock};

use crate::look::{self, Facing, Look, Material, Sheet};

#[cfg(feature = "encode")]
pub mod reduce;

/// The pack's first four bytes.
pub const FORMAT_MAGIC: [u8; 4] = *b"MUPK";
/// The pack format's version (version 1 was PR #154's positional `M25D`;
/// version 2, U.6's, keyed its entries by look id; version 3 by model id,
/// doc 31 D-31-2).
pub const FORMAT_VERSION: u16 = 3;
/// The side of the render every size is reduced from, pixels.
pub const RENDER_PX: u16 = 384;
/// The `fp` chain, largest first (Q-22-4: the floor is 8 px; under it the
/// bracket alone).
pub const FP_SIZES: [u16; 8] = [96, 64, 48, 32, 24, 16, 12, 8];
/// The `iso` chain, largest first: every step under 25 %, and 96/80/64/48
/// halve onto 48/40/32/24, so 0.5× zoom draws the same chain one octave down.
pub const ISO_SIZES: [u16; 8] = [96, 80, 64, 56, 48, 40, 32, 24];
/// The `fp` bake's pitch, degrees down (Q-22-3).
pub const FP_PITCH_DEG: f32 = 3.0;
/// The `iso` bake's pitch, degrees down: the 2.5D's fixed angle.
pub const ISO_PITCH_DEG: f32 = 28.0;
/// The depression, degrees, past which the heightfield draws a contact from
/// the `iso` chain rather than the `fp` one (about halfway between the two
/// pitches, doc 22 §5).
pub const CROSSOVER_DEG: f32 = 15.0;
/// The poses, in pack order: still, the four-frame moving cycle, the wreck.
pub const POSES: [&str; 6] = ["still", "moving0", "moving1", "moving2", "moving3", "wreck"];
/// The wreck's pose index.
pub const POSE_WRECK: usize = 5;
/// Frames a size: eight facings by six poses, facing-major.
pub const FRAMES: usize = 48;
/// Rule 7's budget, bytes: since doc 31 (D-31-12) a ceiling on **one
/// model's** file, which is what the web fetches.
pub const BUDGET_BYTES: usize = 512 * 1024;
/// The longest string the pack carries.
const MAX_STR: u64 = 4096;
/// The most clips, weapons or muzzles one entry carries.
const MAX_LIST: u64 = 64;
/// The most material codes a frame may hold (a 255 px square).
const MAX_PIXELS: usize = 255 * 255;

/// One of the bake's two views (doc 22 D-22-10; `top` is not baked, Q-22-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum View {
    /// The first-person view: a level eye, baked at [`FP_PITCH_DEG`].
    Fp,
    /// The 2.5D's view, baked at [`ISO_PITCH_DEG`].
    Iso,
}

impl View {
    /// Both views, in pack order.
    pub const ALL: [View; 2] = [View::Fp, View::Iso];

    /// The view's name in a pack, a bake's `meta.json` and a file name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            View::Fp => "fp",
            View::Iso => "iso",
        }
    }

    /// The view `word` names.
    #[must_use]
    pub fn named(word: &str) -> Option<View> {
        match word {
            "fp" => Some(View::Fp),
            "iso" => Some(View::Iso),
            _ => None,
        }
    }

    /// The view's size chain, largest first.
    #[must_use]
    pub const fn sizes(self) -> &'static [u16; 8] {
        match self {
            View::Fp => &FP_SIZES,
            View::Iso => &ISO_SIZES,
        }
    }

    /// The pitch the view is baked at, degrees down.
    #[must_use]
    pub const fn pitch_deg(self) -> f32 {
        match self {
            View::Fp => FP_PITCH_DEG,
            View::Iso => ISO_PITCH_DEG,
        }
    }

    /// **The chain pick** (doc 22 §5): the view whose pitch is nearer the line
    /// to a contact seen `depression_deg` below the eye's level — `fp` within
    /// [`CROSSOVER_DEG`], `iso` beyond, so looking down into a bowl from a hill
    /// shows the view the 2.5D bakes.
    #[must_use]
    pub fn for_depression(depression_deg: f32) -> View {
        if depression_deg > CROSSOVER_DEG {
            View::Iso
        } else {
            View::Fp
        }
    }

    const fn tag(self) -> u8 {
        match self {
            View::Fp => 0,
            View::Iso => 1,
        }
    }
}

/// The pose index a contact draws: its wreck, its moving cycle at `step`, or
/// still.
#[must_use]
pub fn pose_index(moving: bool, step: usize, wreck: bool) -> usize {
    if wreck {
        POSE_WRECK
    } else if moving {
        1 + step % 4
    } else {
        0
    }
}

/// The index of `facing` in a size's frames (`facing * 6 + pose`).
fn facing_index(facing: Facing) -> usize {
    Facing::ALL.iter().position(|&f| f == facing).unwrap_or(0)
}

/// What a pack's frames were made from (rule 2; the 2026-10-29 licensing
/// ruling's corollary: every baked artifact is reproducible from a committed
/// script and a named asset pack).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    /// The asset pack's name, as its store lists it.
    pub asset_pack: String,
    /// The asset pack's version.
    pub asset_version: String,
    /// The prefab the bake assembled (with its attachments, as the bake's
    /// config lists them).
    pub prefab: String,
    /// The animation clips the poses were sampled from.
    pub clips: Vec<String>,
    /// The commit `Bake.cs` ran at.
    pub bake_commit: String,
    /// The commit the reducer ran at.
    pub reduce_commit: String,
}

/// One baked frame: its tight bounding box inside the `size` square, the
/// anchor (the pixel that stands on the ground under the unit's centre), the
/// unit's height above the anchor, its muzzles, and its material codes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BakedFrame {
    /// The box's left column.
    pub x: u8,
    /// The box's top row.
    pub y: u8,
    /// The box's width (zero for an empty frame).
    pub w: u8,
    /// The box's height.
    pub h: u8,
    /// The anchor, frame pixels.
    pub anchor: (u8, u8),
    /// Rows from the box's top down to the anchor's, inclusive: what the
    /// 2.5D's depth sort and the lock box read.
    pub height: u8,
    /// Where each weapon's shot leaves, by weapon component id, frame pixels.
    pub muzzles: Vec<(String, (u8, u8))>,
    /// The box's material codes, row-major: `0` nothing, `1..=6` the
    /// materials ([`material_from_code`]).
    pub pixels: Vec<u8>,
}

impl BakedFrame {
    /// The material code at frame pixel `(x, y)`; `0` outside the box.
    #[must_use]
    pub fn code(&self, x: i64, y: i64) -> u8 {
        let (bx, by) = (x - i64::from(self.x), y - i64::from(self.y));
        if bx < 0 || by < 0 || bx >= i64::from(self.w) || by >= i64::from(self.h) {
            return 0;
        }
        let i = usize::try_from(by * i64::from(self.w) + bx).unwrap_or(usize::MAX);
        self.pixels.get(i).copied().unwrap_or(0)
    }

    /// A frame from a full `n` by `n` grid of codes, tightened to the box of
    /// its drawn pixels, with its anchor, muzzles and height.
    #[must_use]
    pub fn from_grid(
        grid: &[u8],
        n: usize,
        anchor: (u8, u8),
        muzzles: Vec<(String, (u8, u8))>,
    ) -> BakedFrame {
        let (mut x0, mut y0, mut x1, mut y1) = (n, n, 0usize, 0usize);
        for (i, &c) in grid.iter().enumerate().take(n * n) {
            if c != 0 {
                let (x, y) = (i % n.max(1), i / n.max(1));
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        if x0 > x1 || y0 > y1 {
            return BakedFrame {
                anchor,
                muzzles,
                ..BakedFrame::default()
            };
        }
        let mut pixels = Vec::with_capacity((x1 - x0 + 1) * (y1 - y0 + 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                pixels.push(grid.get(y * n + x).copied().unwrap_or(0));
            }
        }
        let byte = |v: usize| u8::try_from(v).unwrap_or(u8::MAX);
        let height = usize::from(anchor.1).saturating_add(1).saturating_sub(y0);
        BakedFrame {
            x: byte(x0),
            y: byte(y0),
            w: byte(x1 - x0 + 1),
            h: byte(y1 - y0 + 1),
            anchor,
            height: byte(height),
            muzzles,
            pixels,
        }
    }
}

/// One size of a chain: its side and its 48 frames, facing-major.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SizedFrames {
    /// The frame's side, pixels.
    pub size: u16,
    /// Eight facings by six poses ([`FRAMES`]), `facing * 6 + pose`.
    pub frames: Vec<BakedFrame>,
}

impl SizedFrames {
    /// The frame for `facing` in pose `pose` ([`pose_index`]).
    #[must_use]
    pub fn frame(&self, facing: Facing, pose: usize) -> Option<&BakedFrame> {
        self.frames.get(facing_index(facing) * POSES.len() + pose)
    }
}

/// One view's chain: the world span a frame's side covers and the sizes,
/// largest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    /// The view.
    pub view: View,
    /// How much of the world a frame's side spans, centimetres (the bake's
    /// orthographic window).
    pub span_cm: u32,
    /// The sizes, largest first.
    pub sizes: Vec<SizedFrames>,
}

impl Chain {
    /// The frame side, pixels, that draws the unit at `px_per_m` pixels a
    /// metre.
    #[must_use]
    pub fn needed_px(&self, px_per_m: f32) -> f32 {
        px_per_m * self.span_cm as f32 / 100.0
    }

    /// **The size pick** (doc 22 §5): the smallest size at or above the frame
    /// side that draws the unit at `px_per_m` — the size nearest above its
    /// projected height, since every frame of a chain spans the same world —
    /// or the largest when none is that large. Never a smaller size scaled
    /// up while a larger one exists (rule 1's other half). With its index.
    #[must_use]
    pub fn pick(&self, px_per_m: f32) -> Option<(usize, &SizedFrames)> {
        let needed = self.needed_px(px_per_m);
        self.sizes
            .iter()
            .enumerate()
            .filter(|(_, s)| f32::from(s.size) >= needed)
            .min_by_key(|(_, s)| s.size)
            .or_else(|| self.sizes.iter().enumerate().max_by_key(|(_, s)| s.size))
    }
}

/// One model's bake: its provenance, its size and both chains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelBake {
    /// The model's id (`mc.st.tank_lvl2`, …), which a look names.
    pub model: String,
    /// What it was made from.
    pub provenance: Provenance,
    /// How big the baked model stands, `(width, height)` centimetres: the
    /// model scaled to its look's height, its widest silhouette measured.
    pub size_cm: (u32, u32),
    /// The chains, in [`View::ALL`] order.
    pub chains: Vec<Chain>,
}

impl ModelBake {
    /// The chain for `view`.
    #[must_use]
    pub fn chain(&self, view: View) -> Option<&Chain> {
        self.chains.iter().find(|c| c.view == view)
    }
}

/// Why a pack was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackError(pub String);

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn refuse<T>(why: impl Into<String>) -> Result<T, PackError> {
    Err(PackError(why.into()))
}

/// Baked models, sorted by model id: a shipped model's file holds one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    /// One entry a baked model.
    pub entries: Vec<ModelBake>,
}

impl Pack {
    /// The entry for model `id`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ModelBake> {
        self.entries.iter().find(|e| e.model == id)
    }

    /// Put `entry` in, replacing the entry for the same model, kept sorted by
    /// model id (so a pack's bytes depend only on its entries).
    pub fn put(&mut self, entry: ModelBake) {
        self.entries.retain(|e| e.model != entry.model);
        self.entries.push(entry);
        self.entries.sort_by(|a, b| a.model.cmp(&b.model));
    }

    /// The pack as bytes (format [`FORMAT_VERSION`]).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&FORMAT_MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        put_varint(&mut out, self.entries.len() as u64);
        for e in &self.entries {
            put_str(&mut out, &e.model);
            let p = &e.provenance;
            put_str(&mut out, &p.asset_pack);
            put_str(&mut out, &p.asset_version);
            put_str(&mut out, &p.prefab);
            put_varint(&mut out, p.clips.len() as u64);
            for c in &p.clips {
                put_str(&mut out, c);
            }
            put_str(&mut out, &p.bake_commit);
            put_str(&mut out, &p.reduce_commit);
            put_varint(&mut out, u64::from(e.size_cm.0));
            put_varint(&mut out, u64::from(e.size_cm.1));
            // The muzzles' names once an entry; a frame's muzzle is an index.
            let mut names: Vec<&str> = e
                .chains
                .iter()
                .flat_map(|c| &c.sizes)
                .flat_map(|s| &s.frames)
                .flat_map(|f| f.muzzles.iter().map(|(n, _)| n.as_str()))
                .collect();
            names.sort_unstable();
            names.dedup();
            put_varint(&mut out, names.len() as u64);
            for n in &names {
                put_str(&mut out, n);
            }
            put_varint(&mut out, e.chains.len() as u64);
            for c in &e.chains {
                out.push(c.view.tag());
                put_varint(&mut out, u64::from(c.span_cm));
                put_varint(&mut out, c.sizes.len() as u64);
                for s in &c.sizes {
                    put_varint(&mut out, u64::from(s.size));
                    put_varint(&mut out, s.frames.len() as u64);
                    for f in &s.frames {
                        out.extend_from_slice(&[
                            f.x, f.y, f.w, f.h, f.anchor.0, f.anchor.1, f.height,
                        ]);
                        put_varint(&mut out, f.muzzles.len() as u64);
                        for (name, (mx, my)) in &f.muzzles {
                            let i = names.iter().position(|n| n == name).unwrap_or(0);
                            put_varint(&mut out, i as u64);
                            out.extend_from_slice(&[*mx, *my]);
                        }
                        let runs = encode_runs(&f.pixels);
                        put_varint(&mut out, runs.len() as u64);
                        out.extend_from_slice(&runs);
                    }
                }
            }
        }
        out
    }

    /// Read a pack, checking its structure: the magic and version, every
    /// count in bounds, every box inside its frame, every run filling its box
    /// exactly, nothing left over. (What the frames must hold — both chains at
    /// every size, the marks inside — is [`check_model`]'s.)
    ///
    /// # Errors
    ///
    /// [`PackError`] naming the first thing that does not hold.
    pub fn decode(bytes: &[u8]) -> Result<Pack, PackError> {
        let mut r = Reader { b: bytes, at: 0 };
        if r.take(4)? != FORMAT_MAGIC {
            return refuse("not a unit pack (no MUPK magic)");
        }
        let version = u16::from_le_bytes([r.u8()?, r.u8()?]);
        if version != FORMAT_VERSION {
            return refuse(format!(
                "unit pack version {version}; this build reads {FORMAT_VERSION}"
            ));
        }
        let count = r.count(u64::from(u16::MAX))?;
        let mut pack = Pack::default();
        for _ in 0..count {
            let look = r.str()?;
            let asset_pack = r.str()?;
            let asset_version = r.str()?;
            let prefab = r.str()?;
            let clips = (0..r.count(MAX_LIST)?)
                .map(|_| r.str())
                .collect::<Result<Vec<_>, _>>()?;
            let bake_commit = r.str()?;
            let reduce_commit = r.str()?;
            let size_cm = (r.u32()?, r.u32()?);
            let names = (0..r.count(MAX_LIST)?)
                .map(|_| r.str())
                .collect::<Result<Vec<_>, _>>()?;
            let mut chains = Vec::new();
            for _ in 0..r.count(2)? {
                let view = match r.u8()? {
                    0 => View::Fp,
                    1 => View::Iso,
                    t => return refuse(format!("{look}: view tag {t}")),
                };
                if chains.iter().any(|c: &Chain| c.view == view) {
                    return refuse(format!("{look}: two {} chains", view.name()));
                }
                let span_cm = r.u32()?;
                let mut sizes = Vec::new();
                for _ in 0..r.count(MAX_LIST)? {
                    let size = u16::try_from(r.count(255)?).unwrap_or(0);
                    if size == 0 {
                        return refuse(format!("{look}: a {} size of 0", view.name()));
                    }
                    let mut frames = Vec::new();
                    for _ in 0..r.count(FRAMES as u64)? {
                        frames.push(r.frame(&names, size, &look)?);
                    }
                    sizes.push(SizedFrames { size, frames });
                }
                chains.push(Chain {
                    view,
                    span_cm,
                    sizes,
                });
            }
            chains.sort_by_key(|c| c.view);
            pack.put(ModelBake {
                model: look,
                provenance: Provenance {
                    asset_pack,
                    asset_version,
                    prefab,
                    clips,
                    bake_commit,
                    reduce_commit,
                },
                size_cm,
                chains,
            });
        }
        if pack.entries.len() != count {
            return refuse("two entries for one model");
        }
        if r.at != bytes.len() {
            return refuse(format!("{} bytes after the last entry", bytes.len() - r.at));
        }
        Ok(pack)
    }
}

/// The bytes of a pack with no entries: what the tree carries until the
/// owner's bake lands.
#[must_use]
pub fn empty_pack_bytes() -> Vec<u8> {
    Pack::default().encode()
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], PackError> {
        let end = self.at.saturating_add(n);
        let Some(s) = self.b.get(self.at..end) else {
            return refuse("the pack ends early");
        };
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, PackError> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }

    fn varint(&mut self) -> Result<u64, PackError> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        refuse("a varint longer than 64 bits")
    }

    fn count(&mut self, max: u64) -> Result<usize, PackError> {
        let v = self.varint()?;
        if v > max {
            return refuse(format!("a count of {v}, more than {max}"));
        }
        Ok(usize::try_from(v).unwrap_or(0))
    }

    fn u32(&mut self) -> Result<u32, PackError> {
        let v = self.varint()?;
        u32::try_from(v).or_else(|_| refuse(format!("{v} does not fit 32 bits")))
    }

    fn str(&mut self) -> Result<String, PackError> {
        let n = self.count(MAX_STR)?;
        String::from_utf8(self.take(n)?.to_vec()).or_else(|_| refuse("a string is not UTF-8"))
    }

    fn frame(&mut self, names: &[String], size: u16, look: &str) -> Result<BakedFrame, PackError> {
        let head = self.take(7)?;
        let at = |i: usize| head.get(i).copied().unwrap_or(0);
        let (x, y, w, h) = (at(0), at(1), at(2), at(3));
        if u16::from(x) + u16::from(w) > size || u16::from(y) + u16::from(h) > size {
            return refuse(format!("{look}: a {size} px frame's box runs outside it"));
        }
        let anchor = (at(4), at(5));
        let height = at(6);
        let mut muzzles = Vec::new();
        for _ in 0..self.count(MAX_LIST)? {
            let i = self.count(MAX_LIST)?;
            let Some(name) = names.get(i) else {
                return refuse(format!("{look}: muzzle name {i} of {}", names.len()));
            };
            let (mx, my) = (self.u8()?, self.u8()?);
            muzzles.push((name.clone(), (mx, my)));
        }
        let len = self.count(MAX_PIXELS as u64 * 2)?;
        let pixels = decode_runs(self.take(len)?, usize::from(w) * usize::from(h))?;
        Ok(BakedFrame {
            x,
            y,
            w,
            h,
            anchor,
            height,
            muzzles,
            pixels,
        })
    }
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// **Rule 7's runs**: the box's codes, row-major, as runs, each a material
/// and a varint length. The material rides in the run's first byte — three
/// bits of material, the low four bits of the length less one, a continuation
/// bit — and any further length follows as LEB128 groups of seven: a run of up
/// to sixteen is one byte. (A whole byte for the material measured at twice
/// the size on the shipped art: 3.1 MB against 1.6 MB for six looks, doc 22
/// cut notes, U.6.)
#[must_use]
pub fn encode_runs(codes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&m) = codes.get(i) {
        let run = codes
            .get(i..)
            .map_or(1, |rest| rest.iter().take_while(|&&c| c == m).count());
        let more = run as u64 - 1;
        let first = (m & 0x07) | (((more & 0x0f) as u8) << 3);
        if more >> 4 == 0 {
            out.push(first);
        } else {
            out.push(first | 0x80);
            put_varint(&mut out, more >> 4);
        }
        i += run;
    }
    out
}

/// The codes [`encode_runs`] wrote, exactly `len` of them.
///
/// # Errors
///
/// [`PackError`] when the runs overrun or fall short of `len`, name a
/// material code past `6`, or end inside a run.
pub fn decode_runs(bytes: &[u8], len: usize) -> Result<Vec<u8>, PackError> {
    let mut out = Vec::with_capacity(len);
    let mut r = Reader { b: bytes, at: 0 };
    while r.at < bytes.len() {
        let b = r.u8()?;
        let m = b & 0x07;
        if m > 6 {
            return refuse(format!("material code {m}"));
        }
        let mut more = u64::from((b >> 3) & 0x0f);
        if b & 0x80 != 0 {
            more |= r.varint()? << 4;
        }
        let run = usize::try_from(more)
            .unwrap_or(usize::MAX)
            .saturating_add(1);
        if out.len().saturating_add(run) > len {
            return refuse("a frame's runs overrun its box");
        }
        out.resize(out.len() + run, m);
    }
    if out.len() != len {
        return refuse(format!("a frame's runs fill {} of {len}", out.len()));
    }
    Ok(out)
}

/// The material a pack code names: `1..=6` the six, anything else nothing.
#[must_use]
pub fn material_from_code(code: u8) -> Option<Material> {
    match code {
        1 => Some(Material::TeamDark),
        2 => Some(Material::Team),
        3 => Some(Material::TeamLight),
        4 => Some(Material::Black),
        5 => Some(Material::Steel),
        6 => Some(Material::Pale),
        _ => None,
    }
}

/// The pack code for `material` ([`material_from_code`]'s inverse).
#[must_use]
pub fn code_of(material: Material) -> u8 {
    match material {
        Material::TeamDark => 1,
        Material::Team => 2,
        Material::TeamLight => 3,
        Material::Black => 4,
        Material::Steel => 5,
        Material::Pale => 6,
    }
}

/// The models this build carries, decoded from the files it embeds on first
/// use, and every model [`install`]ed since: append-only, so a slot never
/// changes its model. Each entry is leaked once — a model is installed at
/// most once a process, and the pictures hand out `'static` chains.
static MODELS: RwLock<Vec<&'static ModelBake>> = RwLock::new(Vec::new());
static EMBEDDED_LOADED: OnceLock<()> = OnceLock::new();

fn load_embedded() {
    EMBEDDED_LOADED.get_or_init(|| {
        for (file, bytes) in crate::model::embedded_files() {
            // A shipped file that does not decode is a build defect the
            // catalogue test names; the pictures draw the fallback instead.
            if insert(bytes).is_err() {
                debug_assert!(false, "{file}: not a unit pack this build reads");
            }
        }
    });
}

fn insert(bytes: &[u8]) -> Result<bool, PackError> {
    let pack = Pack::decode(bytes)?;
    let mut table = MODELS.write().unwrap_or_else(PoisonError::into_inner);
    let mut added = false;
    for entry in pack.entries {
        if table.iter().all(|m| m.model != entry.model) {
            table.push(Box::leak(Box::new(entry)));
            added = true;
        }
    }
    Ok(added)
}

/// **Install a fetched model file** (the web's path, doc 31 D-31-11): every
/// entry it holds that the table lacks joins it, and the pictures draw from
/// it on their next frame; whether any did (`false` for a model already
/// installed — the files are immutable, so a second copy is the same model).
///
/// # Errors
///
/// [`PackError`] when the bytes are not a pack this build reads.
pub fn install(bytes: &[u8]) -> Result<bool, PackError> {
    load_embedded();
    insert(bytes)
}

/// The slot of model `id` in the table, if it is installed (or embedded).
#[must_use]
pub fn model_slot(id: &str) -> Option<usize> {
    load_embedded();
    MODELS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .position(|m| m.model == id)
}

/// The model in table slot `slot`.
#[must_use]
pub fn model_in_slot(slot: usize) -> Option<&'static ModelBake> {
    load_embedded();
    MODELS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(slot)
        .copied()
}

/// **The bake of model `id`**, when this build embeds it or a fetched file
/// installed it.
#[must_use]
pub fn model(id: &str) -> Option<&'static ModelBake> {
    model_slot(id).and_then(model_in_slot)
}

/// Every model the pictures can draw now, in table order.
#[must_use]
pub fn models() -> Vec<&'static ModelBake> {
    load_embedded();
    MODELS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The bake look `look` draws from, when its model is installed.
#[must_use]
pub fn bake_of_look(look: &Look) -> Option<&'static ModelBake> {
    look.model.as_deref().and_then(model)
}

/// **What a 2.5D unit's art is drawn from**: its model's installed bake, by
/// table slot, or its look's fallback sheet, by the look's index in
/// [`look::shipped`]. Two looks on one model draw one art — what the 2.5D's
/// kitty sheet cache keys on, so five tank variants upload one sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Art {
    /// A baked model, by its slot in the table ([`model_slot`]).
    Model(usize),
    /// A look's text sheet, by the look's index ([`look::by_index`]).
    Sheet(usize),
}

/// The art shipped look `index` draws in the 2.5D now: its model's bake when
/// installed, else its sheet ([`Look::sheet`]); `None` for a look with
/// neither.
#[must_use]
pub fn iso_art(index: usize) -> Option<Art> {
    let look = look::by_index(index)?;
    if let Some(slot) = look.model.as_deref().and_then(model_slot)
        && model_in_slot(slot).is_some_and(|m| m.chain(View::Iso).is_some())
    {
        return Some(Art::Model(slot));
    }
    look.sheet().map(|_| Art::Sheet(index))
}

/// The fallback chains, one a look ([`look::by_index`]), built on first use
/// and leaked once, so a look installed at run time gets one too.
static FALLBACK: std::sync::Mutex<std::collections::BTreeMap<usize, &'static Chain>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// **The `iso` chain `art` is drawn from**, with the height, centimetres,
/// the size pick reads: the model's own, or the look's sheet as a chain
/// ([`chain_from_sheet`]) at the look's `size_cm`.
#[must_use]
pub fn art_chain(art: Art) -> Option<(&'static Chain, u32)> {
    match art {
        Art::Model(slot) => {
            let bake = model_in_slot(slot)?;
            Some((bake.chain(View::Iso)?, bake.size_cm.1))
        }
        Art::Sheet(index) => {
            let look = look::by_index(index)?;
            let sheet = look.sheet()?;
            let mut cache = FALLBACK.lock().unwrap_or_else(PoisonError::into_inner);
            let chain: &'static Chain = match cache.get(&index) {
                Some(chain) => chain,
                None => {
                    let chain: &'static Chain =
                        Box::leak(Box::new(chain_from_sheet(sheet, look.size_cm, View::Iso)));
                    cache.insert(index, chain);
                    chain
                }
            };
            Some((chain, look.size_cm.1))
        }
    }
}

/// **The `iso` chain the 2.5D draws shipped look `index` from** now
/// ([`iso_art`], [`art_chain`]).
#[must_use]
pub fn iso_chain(index: usize) -> Option<(&'static Chain, u32)> {
    art_chain(iso_art(index)?)
}

/// **A look's text sheet as a chain** (rule 3's fallback, for the 2.5D):
/// every size of `view`'s chain nearest-sampled from the sheet, the unit
/// at `size_cm`, three-quarters of the frame's side at its widest, standing an eighth of
/// the side above the frame's foot.
#[must_use]
pub fn chain_from_sheet(sheet: &Sheet, size_cm: (u32, u32), view: View) -> Chain {
    let (wcm, hcm) = (size_cm.0.max(1), size_cm.1.max(1));
    let span_cm = wcm.max(hcm).saturating_mul(4) / 3;
    let sizes = view
        .sizes()
        .iter()
        .map(|&size| SizedFrames {
            size,
            frames: Facing::ALL
                .iter()
                .flat_map(|&facing| {
                    (0..POSES.len())
                        .map(move |pose| sheet_frame(sheet, size_cm, facing, pose, size, span_cm))
                })
                .collect(),
        })
        .collect();
    Chain {
        view,
        span_cm,
        sizes,
    }
}

fn sheet_frame(
    sheet: &Sheet,
    size_cm: (u32, u32),
    facing: Facing,
    pose: usize,
    size: u16,
    span_cm: u32,
) -> BakedFrame {
    let n = usize::from(size);
    let picked = match pose {
        0 => sheet.frame(facing, false, 0),
        POSE_WRECK => sheet
            .wreck_frame(facing)
            .map(|f| (f, false))
            .or_else(|| sheet.frame(facing, false, 0)),
        p => sheet.frame(facing, true, p - 1),
    };
    let (sw, sh) = (sheet.width.max(1), sheet.height.max(1));
    let scale = |cm: u32| ((cm as usize * n) / span_cm.max(1) as usize).clamp(1, n);
    let (dw, dh) = (scale(size_cm.0), scale(size_cm.1));
    let off_x = (n - dw) / 2;
    let off_y = n.saturating_sub(dh + n / 8);
    let mut grid = vec![0u8; n * n];
    let mut mirrored = false;
    if let Some((pixels, mirror)) = picked {
        mirrored = mirror;
        for dy in 0..dh {
            let sy = (dy * sh / dh).min(sh - 1);
            for dx in 0..dw {
                let sx = (dx * sw / dw).min(sw - 1);
                let sx = if mirror { sw - 1 - sx } else { sx };
                if let Some(Some(m)) = pixels.get(sy * sw + sx)
                    && let Some(cell) = grid.get_mut((off_y + dy) * n + off_x + dx)
                {
                    *cell = code_of(*m);
                }
            }
        }
    }
    let ax = if mirrored {
        sw - 1 - sheet.anchor.0.min(sw - 1)
    } else {
        sheet.anchor.0
    };
    let byte = |v: usize| u8::try_from(v.min(n - 1)).unwrap_or(u8::MAX);
    let anchor = (
        byte(off_x + (2 * ax + 1) * dw / (2 * sw)),
        byte(off_y + (2 * sheet.anchor.1 + 1) * dh / (2 * sh)),
    );
    BakedFrame::from_grid(&grid, n, anchor, Vec::new())
}

/// **The matrix check** (doc 22 §6, D-22-10; a model at a time since doc 31
/// D-31-2): what model bake `e` must hold against the `looks` that name it.
/// At least one look names it, and its provenance is complete; it has both
/// chains at exactly their sizes, 48 frames each; every frame's box, anchor,
/// height and muzzles lie inside it, and every muzzle names a weapon of a
/// look that names the model; its height is each such look's `size_cm`
/// height (within 2 %: the bake scales the model to it) and its widest
/// silhouette within half to twice the look's width (barrels and arms stand
/// past the sim's silhouette — the walker measures 9.6 m across its cannon
/// against its look's 5.2 m — so this catches a wrong model, not a long gun);
/// and every still frame's anchor stands under the unit's middle — in the
/// box's middle half across, and on its foot in the `fp` view. Returns what
/// does not hold, empty when everything does.
#[must_use]
pub fn check_model(e: &ModelBake, looks: &[Look]) -> Vec<String> {
    let mut bad = Vec::new();
    let naming: Vec<&Look> = looks
        .iter()
        .filter(|l| l.model.as_deref() == Some(e.model.as_str()))
        .collect();
    if naming.is_empty() {
        bad.push(format!("{}: no shipped look names the model", e.model));
    }
    let p = &e.provenance;
    for (what, v) in [
        ("asset pack", &p.asset_pack),
        ("asset version", &p.asset_version),
        ("prefab", &p.prefab),
        ("bake commit", &p.bake_commit),
        ("reduce commit", &p.reduce_commit),
    ] {
        if v.trim().is_empty() {
            bad.push(format!("{}: no {what} in its provenance", e.model));
        }
    }
    for look in &naming {
        let (lw, lh) = look.size_cm;
        if e.size_cm.1.abs_diff(lh) * 50 > lh {
            bad.push(format!(
                "{}: baked {} cm tall, look {} {lh} cm",
                e.model, e.size_cm.1, look.id
            ));
        }
        if e.size_cm.0 * 2 < lw || e.size_cm.0 > lw * 2 {
            bad.push(format!(
                "{}: baked {} cm wide, look {} {lw} cm",
                e.model, e.size_cm.0, look.id
            ));
        }
    }
    let weapons: Vec<&str> = naming
        .iter()
        .flat_map(|look| {
            look.weapons.keys().map(String::as_str).chain(
                look.sheet()
                    .into_iter()
                    .flat_map(|s| s.facings.values())
                    .flat_map(|f| f.muzzles.keys().map(String::as_str)),
            )
        })
        .collect();
    for view in View::ALL {
        let Some(chain) = e.chain(view) else {
            bad.push(format!("{}: no {} chain", e.model, view.name()));
            continue;
        };
        let sizes: Vec<u16> = chain.sizes.iter().map(|s| s.size).collect();
        if sizes != view.sizes() {
            bad.push(format!(
                "{} {}: sizes {sizes:?}, not {:?}",
                e.model,
                view.name(),
                view.sizes()
            ));
        }
        if chain.span_cm == 0 {
            bad.push(format!("{} {}: a span of 0", e.model, view.name()));
        }
        for s in &chain.sizes {
            check_size(&mut bad, &e.model, view, s, &weapons);
        }
    }
    bad
}

fn check_size(bad: &mut Vec<String>, model: &str, view: View, s: &SizedFrames, weapons: &[&str]) {
    let at = format!("{model} {} {}", view.name(), s.size);
    if s.frames.len() != FRAMES {
        bad.push(format!("{at}: {} frames, not {FRAMES}", s.frames.len()));
    }
    let n = s.size;
    let inside = |(x, y): (u8, u8)| u16::from(x) < n && u16::from(y) < n;
    for (i, f) in s.frames.iter().enumerate() {
        let (facing, pose) = (
            Facing::ALL
                .get(i / POSES.len())
                .copied()
                .unwrap_or(Facing::Front),
            POSES.get(i % POSES.len()).copied().unwrap_or("?"),
        );
        let at = format!("{at} {} {pose}", facing.as_str());
        if f.w == 0 || f.h == 0 {
            bad.push(format!("{at}: empty"));
            continue;
        }
        if !inside(f.anchor) {
            bad.push(format!("{at}: anchor {:?} outside the frame", f.anchor));
        }
        if f.height == 0 || u16::from(f.anchor.1) + 1 < u16::from(f.height) {
            bad.push(format!("{at}: height {} above the frame", f.height));
        }
        for (name, xy) in &f.muzzles {
            if !inside(*xy) {
                bad.push(format!("{at}: muzzle {name} {xy:?} outside the frame"));
            }
            if !weapons.contains(&name.as_str()) {
                bad.push(format!(
                    "{at}: muzzle {name} names no weapon of a look on the model"
                ));
            }
        }
        if i % POSES.len() == 0 {
            let (cx, half) = (2 * u16::from(f.x) + u16::from(f.w), u16::from(f.w));
            if (2 * u16::from(f.anchor.0) + 1).abs_diff(cx) > half / 2 + 2 {
                bad.push(format!(
                    "{at}: anchor x {} outside the box's middle half ({}..{})",
                    f.anchor.0,
                    f.x,
                    u16::from(f.x) + u16::from(f.w)
                ));
            }
            let foot = u16::from(f.y) + u16::from(f.h);
            if view == View::Fp && u16::from(f.anchor.1) + 2 + u16::from(f.h) / 8 < foot {
                bad.push(format!(
                    "{at}: anchor y {} above the box's foot {foot}",
                    f.anchor.1
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of `n` with a block of material 2 and a run of 3 across it,
    /// and a `cannon` muzzle.
    fn block_frame(n: usize) -> BakedFrame {
        let mut grid = vec![0u8; n * n];
        for y in n / 4..n * 3 / 4 {
            for x in n / 8..n * 7 / 8 {
                grid[y * n + x] = if y == n / 2 { 3 } else { 2 };
            }
        }
        let a = u8::try_from(n / 2).unwrap();
        let foot = u8::try_from(n * 3 / 4 - 1).unwrap();
        BakedFrame::from_grid(&grid, n, (a, foot), vec![("cannon".into(), (a, a))])
    }

    fn block_entry(model: &str, size_cm: (u32, u32)) -> ModelBake {
        ModelBake {
            model: model.into(),
            provenance: Provenance {
                asset_pack: "Test Pack".into(),
                asset_version: "1.0".into(),
                prefab: "Prefabs/Test.prefab".into(),
                clips: vec!["idle".into(), "walk".into()],
                bake_commit: "0123abcd".into(),
                reduce_commit: "4567ef01".into(),
            },
            size_cm,
            chains: View::ALL
                .iter()
                .map(|&view| Chain {
                    view,
                    span_cm: 1000,
                    sizes: view
                        .sizes()
                        .iter()
                        .map(|&size| SizedFrames {
                            size,
                            frames: vec![block_frame(usize::from(size)); FRAMES],
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn runs_round_trip_and_a_short_run_is_one_byte() {
        let codes: Vec<u8> = (0..500u32)
            .map(|i| match i {
                0..16 => 1,
                16..17 => 0,
                17..300 => 6,
                _ => u8::try_from(i % 7).unwrap(),
            })
            .collect();
        let runs = encode_runs(&codes);
        assert_eq!(decode_runs(&runs, codes.len()).unwrap(), codes);
        assert_eq!(runs[0], 1 | (15 << 3), "sixteen ones in one byte");
        assert_eq!(runs[1], 0, "one nothing in one byte");
        assert_eq!(runs[2] & 0x80, 0x80, "283 sixes need a second byte");
        assert!(decode_runs(&runs, codes.len() - 1).is_err(), "an overrun");
        assert!(decode_runs(&runs, codes.len() + 1).is_err(), "a short fill");
        assert!(decode_runs(&[7], 1).is_err(), "no material 7");
        assert!(decode_runs(&[0x80], 2).is_err(), "a run cut off");
        assert!(encode_runs(&[]).is_empty());
    }

    #[test]
    fn a_frame_is_its_tight_box() {
        let f = block_frame(32);
        assert_eq!((f.x, f.y, f.w, f.h), (4, 8, 24, 16));
        assert_eq!(f.anchor, (16, 23));
        assert_eq!(f.height, 16, "the box's top row to the anchor's");
        assert_eq!(f.code(4, 8), 2);
        assert_eq!(f.code(3, 8), 0, "outside the box");
        assert_eq!(f.code(10, 16), 3);
        assert_eq!(f.code(-1, -1), 0);
        let empty = BakedFrame::from_grid(&[0; 16], 4, (2, 3), Vec::new());
        assert_eq!((empty.w, empty.h, empty.code(2, 2)), (0, 0, 0));
    }

    #[test]
    fn a_pack_round_trips_and_refuses_what_is_not_one() {
        let mut pack = Pack::default();
        pack.put(block_entry("mc.st.tank_lvl2", (700, 250)));
        pack.put(block_entry("mc.lt.fox_biped", (520, 1100)));
        pack.put(block_entry("mc.lt.fox_biped", (520, 1100)));
        assert_eq!(pack.entries.len(), 2, "put replaces a model's entry");
        assert_eq!(
            pack.entries[0].model, "mc.lt.fox_biped",
            "sorted by model id"
        );
        let bytes = pack.encode();
        assert_eq!(&bytes[..4], b"MUPK");
        assert_eq!(Pack::decode(&bytes).unwrap(), pack);
        assert_eq!(Pack::decode(&empty_pack_bytes()).unwrap(), Pack::default());
        // Refused, never a panic: every truncation, a stray byte, another
        // version, a box outside its frame.
        // (Every cut in the header and the first frames, then a sample.)
        for cut in (0..400).chain((400..bytes.len()).step_by(211)) {
            assert!(Pack::decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut long = bytes.clone();
        long.push(0);
        assert!(Pack::decode(&long).is_err());
        let mut v2 = bytes.clone();
        v2[4] = 2;
        assert!(Pack::decode(&v2).unwrap_err().0.contains("version 2"));
        let mut wide = Pack::default();
        let mut e = block_entry("drone", (250, 100));
        e.chains[0].sizes[7].frames[0].w = 9;
        wide.put(e);
        assert!(
            Pack::decode(&wide.encode()).is_err(),
            "an 8 px frame 9 wide"
        );
    }

    #[test]
    fn the_pick_takes_the_size_nearest_above() {
        let e = block_entry("drone", (250, 100));
        let fp = e.chain(View::Fp).unwrap();
        // A 10 m span: at 3.3 px/m a frame side of 33 px is wanted → 48.
        assert_eq!(fp.pick(3.3).unwrap().1.size, 48);
        assert_eq!(fp.pick(3.2).unwrap().1.size, 32);
        assert_eq!(fp.pick(0.1).unwrap().1.size, 8, "the floor");
        assert_eq!(fp.pick(50.0).unwrap().1.size, 96, "past the largest");
        let iso = e.chain(View::Iso).unwrap();
        assert_eq!(iso.pick(7.0).unwrap().1.size, 80);
        assert_eq!(
            iso.pick(3.5).unwrap().1.size,
            40,
            "half the zoom, half the size"
        );
    }

    #[test]
    fn the_chain_pick_turns_at_fifteen_degrees() {
        assert_eq!(View::for_depression(0.0), View::Fp);
        assert_eq!(View::for_depression(-20.0), View::Fp, "looking up");
        assert_eq!(View::for_depression(15.0), View::Fp);
        assert_eq!(View::for_depression(15.5), View::Iso);
        assert_eq!(View::for_depression(60.0), View::Iso);
    }

    #[test]
    fn the_matrix_check_holds_a_model_against_the_looks_that_name_it() {
        let looks = look::shipped();
        let tank = looks
            .iter()
            .find(|l| l.id == "tracked_tank")
            .expect("the tank's look");
        let id = tank.model.as_deref().expect("the tank names a model");
        let good = block_entry(id, tank.size_cm);
        assert_eq!(check_model(&good, looks), Vec::<String>::new());
        let orphan = block_entry("mc.nobody", tank.size_cm);
        assert!(
            check_model(&orphan, looks)
                .iter()
                .any(|b| b.contains("no shipped look names"))
        );
        let mut tall = good.clone();
        tall.size_cm.1 *= 2;
        assert!(
            check_model(&tall, looks)
                .iter()
                .any(|b| b.contains("cm tall"))
        );
        let mut wide = good.clone();
        wide.size_cm.0 = wide.size_cm.0 * 18 / 10;
        assert!(
            check_model(&wide, looks).is_empty(),
            "a long gun is not a wrong model"
        );
        wide.size_cm.0 *= 2;
        assert!(
            check_model(&wide, looks)
                .iter()
                .any(|b| b.contains("cm wide"))
        );
        let mut chain = good.clone();
        chain.chains[1].sizes.pop();
        assert!(
            check_model(&chain, looks)
                .iter()
                .any(|b| b.contains("sizes"))
        );
        let mut anchor = good.clone();
        anchor.chains[0].sizes[0].frames[0].anchor = (2, 2);
        assert!(
            check_model(&anchor, looks)
                .iter()
                .any(|b| b.contains("anchor"))
        );
        let mut muzzle = good.clone();
        muzzle.chains[0].sizes[0].frames[0].muzzles[0].0 = "railgun".into();
        assert!(
            check_model(&muzzle, looks)
                .iter()
                .any(|b| b.contains("muzzle railgun"))
        );
        let mut prov = good;
        prov.provenance.asset_version.clear();
        assert!(
            check_model(&prov, looks)
                .iter()
                .any(|b| b.contains("asset version"))
        );
    }

    #[test]
    fn every_shipped_look_has_an_iso_chain() {
        for (i, l) in look::shipped().iter().enumerate() {
            let (chain, h) = iso_chain(i).unwrap_or_else(|| panic!("{}: no iso chain", l.id));
            assert!(h > 0);
            let sizes: Vec<u16> = chain.sizes.iter().map(|s| s.size).collect();
            assert_eq!(sizes, ISO_SIZES, "{}", l.id);
            for s in &chain.sizes {
                assert_eq!(s.frames.len(), FRAMES, "{}", l.id);
                let still = s.frame(Facing::Front, 0).unwrap();
                assert!(still.w > 0 && still.h > 0, "{} {}: drawn", l.id, s.size);
                assert!(u16::from(still.anchor.1) < s.size);
            }
        }
    }

    /// The fallback chain from a sheet stands every pose on its anchor, at
    /// every size of the view's chain.
    #[test]
    fn a_sheet_makes_a_whole_chain() {
        for l in look::shipped() {
            let sheet = l.sheet().unwrap_or_else(|| panic!("{}: a sheet", l.id));
            for view in View::ALL {
                let chain = chain_from_sheet(sheet, l.size_cm, view);
                let sizes: Vec<u16> = chain.sizes.iter().map(|s| s.size).collect();
                assert_eq!(sizes, view.sizes(), "{}", l.id);
                assert!(chain.sizes.iter().all(|s| s.frames.len() == FRAMES));
            }
        }
    }

    /// **The table grows a model at a time** (doc 31 D-31-2): a fetched
    /// file joins the table, a second copy adds nothing, a slot never moves,
    /// and what is not a pack is refused.
    #[test]
    fn the_model_table_grows() {
        let before = models().len();
        let mut one = Pack::default();
        one.put(block_entry("test.grows_a", (250, 100)));
        assert_eq!(install(&one.encode()), Ok(true));
        let slot = model_slot("test.grows_a").expect("installed");
        assert_eq!(install(&one.encode()), Ok(false), "the same model twice");
        let mut two = Pack::default();
        two.put(block_entry("test.grows_b", (250, 100)));
        assert_eq!(install(&two.encode()), Ok(true));
        assert_eq!(model_slot("test.grows_a"), Some(slot), "a slot never moves");
        assert!(models().len() >= before + 2);
        assert_eq!(model("test.grows_b").map(|m| m.size_cm), Some((250, 100)));
        assert!(install(b"not a pack").is_err());
    }
}
