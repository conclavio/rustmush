//! **The site ground: the geology table of one site and the composite cache** (doc 20 cut SM.1;
//! §3.1, §3.2; the SM.1 brief's lead ruling 4 and decision 6).
//!
//! A site is 256 × 256 cells of 10 m (§3.1; D-16-5's arena window): 64 × 64 pockets of 4 × 4
//! cells, 8 × 8 fields, 4 × 4 blocks of 2 × 2 fields. Its ground is two pure builds over one
//! captured input:
//!
//! - the atlas's terrain **composite** (`mush_atlas::window::composite`), and
//! - the **site geology table** ([`SiteGeology`]): every pocket's (class, tonnes) for the four
//!   families as two bytes each, and the 64 fields' words and counts — computed with
//!   [`field_ground`] under each block's authored shift, so **the site table is the body
//!   function** (a test pins it).
//!
//! Both are held in an in-RAM LRU of sixteen ([`SiteGroundCache`]) keyed by (region, revision,
//! key version), where "revision" is [`site_revision`], a digest of the **whole** captured
//! input and the geology spec, so no input either build reads can change without changing the
//! key — save the body's table and radius, which the one writer of tables
//! (`crate::surface::install_body_geology`) answers by dropping that body's cached sites
//! whenever it replaces one. [`site_ground_sync`] is the synchronous door (staff and tests); **the off-loop driver is
//! SM.4's** (decision 6 narrows doc 20 §13's SM.1 row): the composite half needs no key (the
//! atlas's own `CompositeJob`/`run_async` already runs it off-loop), the geology half needs the
//! key, which a host-held runner keyed in `build_engine` can hold, never the JSON payload.
//!
//! **Truth never serializes** (§16): [`SiteGeology`] and [`SiteGround`] derive no serde traits.

use std::sync::Arc;

use mush_atlas::RecordKey;
use mush_atlas::window::{Composite, CompositeInput, WindowFrame, capture, composite};
use mush_core::{CanonicalId, GeologyMac, World};
use sha2::{Digest, Sha256};

use super::geology::{
    BodyGeology, GeologyError, MAX_SHIFT, OreFamily, POCKETS_PER_FIELD_SIDE, PocketRow, Word,
    by_family, field_ground,
};
use super::{GeologyState, body_geology, live};
use crate::arena::digest_text;

/// A site's cells along one side (§3.1: 256 × 256 at 10 m, D-16-5's arena window).
pub const SITE_CELLS: u16 = 256;
/// A site cell's edge, centimetres.
pub const SITE_CELL_CM: u16 = 1_000;
/// Pockets along one side of a site: 64 × 64 pockets of 4 × 4 cells.
pub const SITE_POCKETS: i64 = 64;
/// Fields along one side of a site: 8 × 8 fields; 4 × 4 blocks of 2 × 2 fields.
pub const SITE_FIELDS: i64 = 8;
/// Blocks along one side of a site.
pub const SITE_BLOCKS: i64 = 4;
/// The site ground cache's capacity.
pub const SITE_CACHE_CAPACITY: usize = 16;

/// The digest domain of a site geology table.
const SITE_GEOLOGY_DOMAIN: &[u8] = b"tacsim.surface.site_geology.v1\0";
/// The digest domain of a site's revision.
const SITE_REVISION_DOMAIN: &[u8] = b"tacsim.surface.site_revision.v1\0";

/// What the geology table of one site is built from. SM.2 derives it from the site block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteGeologySpec {
    /// The body.
    pub body: CanonicalId,
    /// The site's south-west pocket in the body pocket lattice; both coordinates multiples of 8
    /// (a field corner), so the site's blocks hold whole fields.
    pub origin: (i64, i64),
    /// One authored class shift per block, -3..=3, indexed `by * 4 + bx` from the south-west
    /// (bx, by = field x / 2, field y / 2 inside the site). SM.2 maps the letters A–P onto it.
    pub shifts: [i8; 16],
}

/// One field's words and counts, packed in a site table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FieldCode {
    words: [Word; 4],
    counts: [u8; 4],
}

/// The site's geology: every pocket's four (class, tonnes) as two bytes each (tonnes in bits
/// 0..=7, class code in bits 8..=9), the 64 fields' words and counts, built once per key
/// version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteGeology {
    body: CanonicalId,
    origin: (i64, i64),
    key_version: u32,
    shifts: [i8; 16],
    /// 64, index `fy * 8 + fx`.
    fields: Vec<FieldCode>,
    /// 4,096, index `py * 64 + px`.
    pockets: Vec<[u16; 4]>,
}

/// A pocket's family slot as two bytes.
fn encode(class: Word, tonnes: u8) -> u16 {
    (u16::from(class.code()) << 8) | u16::from(tonnes)
}

/// Two bytes back to a class and tonnes.
fn decode(code: u16) -> (Word, u8) {
    let [tonnes, class] = code.to_le_bytes();
    (Word::of_code(class & 0b11).unwrap_or(Word::Barren), tonnes)
}

/// A site-local index, `None` outside `0..side` on either axis.
fn local_index(x: i64, y: i64, side: i64) -> Option<usize> {
    if !(0..side).contains(&x) || !(0..side).contains(&y) {
        return None;
    }
    usize::try_from(y.checked_mul(side)?.checked_add(x)?).ok()
}

impl SiteGeology {
    /// The body.
    #[must_use]
    pub fn body(&self) -> &CanonicalId {
        &self.body
    }

    /// The site's south-west pocket on the body lattice.
    #[must_use]
    pub fn origin(&self) -> (i64, i64) {
        self.origin
    }

    /// The key version it was built under.
    #[must_use]
    pub fn key_version(&self) -> u32 {
        self.key_version
    }

    /// The pocket at site-local `(px, py)`, 0..64 each.
    #[must_use]
    pub fn pocket(&self, px: i64, py: i64) -> Option<PocketRow> {
        let code = *self.pockets.get(local_index(px, py, SITE_POCKETS)?)?;
        let decoded = code.map(decode);
        Some(PocketRow {
            classes: decoded.map(|(class, _)| class),
            tonnes: decoded.map(|(_, tonnes)| tonnes),
        })
    }

    /// The field at site-local `(fx, fy)`, 0..8 each: its words and ore-pocket counts.
    #[must_use]
    pub fn field(&self, fx: i64, fy: i64) -> Option<([Word; 4], [u8; 4])> {
        let field = self.fields.get(local_index(fx, fy, SITE_FIELDS)?)?;
        Some((field.words, field.counts))
    }

    /// Per family, the site's ore pockets and their tonnes.
    #[must_use]
    pub fn totals(&self) -> [(u32, u32); 4] {
        let mut totals = [(0_u32, 0_u32); 4];
        for code in &self.pockets {
            for ((pockets, tonnes), slot) in totals.iter_mut().zip(code) {
                let (class, t) = decode(*slot);
                if class != Word::Barren {
                    *pockets = pockets.saturating_add(1);
                    *tonnes = tonnes.saturating_add(u32::from(t));
                }
            }
        }
        totals
    }

    /// SHA-256 over `b"tacsim.surface.site_geology.v1\0" || body || 0x00 || origin x, y (i64
    /// LE) || key_version (u32 LE) || 16 shift bytes || each field's 4 word codes and 4 counts
    /// || each pocket's 4 codes (u16 LE)`, in index order — as 64 lowercase base-16 characters.
    #[must_use]
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(SITE_GEOLOGY_DOMAIN);
        hasher.update(self.body.as_str().as_bytes());
        hasher.update([0_u8]);
        hasher.update(self.origin.0.to_le_bytes());
        hasher.update(self.origin.1.to_le_bytes());
        hasher.update(self.key_version.to_le_bytes());
        for shift in self.shifts {
            hasher.update(shift.to_le_bytes());
        }
        for field in &self.fields {
            hasher.update(field.words.map(Word::code));
            hasher.update(field.counts);
        }
        for pocket in &self.pockets {
            for code in pocket {
                hasher.update(code.to_le_bytes());
            }
        }
        digest_text(&hasher.finalize().into())
    }
}

/// Build one site's geology table: its 64 fields under their blocks' shifts, and their pockets.
///
/// # Errors
///
/// An origin that is not a field corner, a shift outside -3..=3, a spec for another body, or a
/// site whose rows run off the lattice.
pub fn site_geology(
    mac: &GeologyMac,
    key_version: u32,
    body: &BodyGeology,
    spec: &SiteGeologySpec,
) -> Result<SiteGeology, GeologyError> {
    if spec.body != body.body {
        return Err(GeologyError::BodyMismatch {
            spec: spec.body.to_string(),
            body: body.body.to_string(),
        });
    }
    if spec.origin.0.rem_euclid(POCKETS_PER_FIELD_SIDE) != 0
        || spec.origin.1.rem_euclid(POCKETS_PER_FIELD_SIDE) != 0
    {
        return Err(GeologyError::Origin);
    }
    if let Some(shift) = spec
        .shifts
        .iter()
        .find(|shift| !(-MAX_SHIFT..=MAX_SHIFT).contains(*shift))
    {
        return Err(GeologyError::Shift(*shift));
    }
    let lattice = body.lattice();
    let top = spec
        .origin
        .1
        .checked_add(SITE_POCKETS)
        .ok_or(GeologyError::OffLattice)?;
    if spec.origin.1 < lattice.south() || top > lattice.north() {
        return Err(GeologyError::OffLattice);
    }
    let field_origin = (
        spec.origin.0.div_euclid(POCKETS_PER_FIELD_SIDE),
        spec.origin.1.div_euclid(POCKETS_PER_FIELD_SIDE),
    );
    let pocket_count = usize::try_from(SITE_POCKETS * SITE_POCKETS).unwrap_or(0);
    let mut fields = Vec::with_capacity(64);
    let mut pockets = vec![[0_u16; 4]; pocket_count];
    for fy in 0..SITE_FIELDS {
        for fx in 0..SITE_FIELDS {
            let block = local_index(fx / 2, fy / 2, SITE_BLOCKS).unwrap_or(0);
            let shift = spec.shifts.get(block).copied().unwrap_or(0);
            let ground = field_ground(
                mac,
                body,
                (
                    field_origin.0.saturating_add(fx),
                    field_origin.1.saturating_add(fy),
                ),
                shift,
            )?;
            fields.push(FieldCode {
                words: ground.words,
                counts: ground.counts,
            });
            for ly in 0..POCKETS_PER_FIELD_SIDE {
                for lx in 0..POCKETS_PER_FIELD_SIDE {
                    let Some(row) = ground.pocket(lx, ly) else {
                        continue;
                    };
                    let px = fx * POCKETS_PER_FIELD_SIDE + lx;
                    let py = fy * POCKETS_PER_FIELD_SIDE + ly;
                    let Some(slot) =
                        local_index(px, py, SITE_POCKETS).and_then(|i| pockets.get_mut(i))
                    else {
                        continue;
                    };
                    *slot = OreFamily::ALL.map(|family| {
                        encode(
                            by_family(row.classes, family),
                            by_family(row.tonnes, family),
                        )
                    });
                }
            }
        }
    }
    Ok(SiteGeology {
        body: spec.body.clone(),
        origin: spec.origin,
        key_version,
        shifts: spec.shifts,
        fields,
        pockets,
    })
}

/// The cache key (the SM.1 brief's lead ruling 4): the region, the [`site_revision`] and the
/// key version.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SiteGroundKey {
    /// The atlas region.
    pub region: RecordKey,
    /// [`site_revision`] of the captured input and the spec.
    pub revision: u64,
    /// The geology key's version.
    pub key_version: u32,
}

/// One site's ground: its composite and its geology table.
#[derive(Clone, Debug, PartialEq)]
pub struct SiteGround {
    /// The terrain composite.
    pub composite: Arc<Composite>,
    /// The geology table.
    pub geology: Arc<SiteGeology>,
}

/// The first eight bytes (LE) of SHA-256 over `b"tacsim.surface.site_revision.v1\0" ||
/// serde_json::to_vec(input) || 0x00 || spec.body || 0x00 || origin x, y (i64 LE) || the 16
/// shift bytes`: "revision" in the cache key, so any input `composite()` reads, and every input
/// `site_geology()` reads from its spec — a moved room footprint, a resized bbox, two frames over
/// one region, a moved origin or shift — changes the key. The body's own table and radius are
/// NOT in it: they are the world's, written only by `crate::surface::install_body_geology`,
/// which drops that body's cached sites when it replaces a table (`SiteGroundCache::forget_body`).
/// `CompositeInput` holds no hash maps (its records use `Vec`/`BTreeMap`), so its JSON is
/// canonical for one build.
///
/// # Errors
///
/// A serialization error (not expected), as text; never a panic.
pub fn site_revision(input: &CompositeInput, spec: &SiteGeologySpec) -> Result<u64, String> {
    let json = serde_json::to_vec(input).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(SITE_REVISION_DOMAIN);
    hasher.update(&json);
    hasher.update([0_u8]);
    hasher.update(spec.body.as_str().as_bytes());
    hasher.update([0_u8]);
    hasher.update(spec.origin.0.to_le_bytes());
    hasher.update(spec.origin.1.to_le_bytes());
    for shift in spec.shifts {
        hasher.update(shift.to_le_bytes());
    }
    let digest: [u8; 32] = hasher.finalize().into();
    let mut head = [0_u8; 8];
    for (slot, byte) in head.iter_mut().zip(digest) {
        *slot = byte;
    }
    Ok(u64::from_le_bytes(head))
}

/// On-loop and cheap: the atlas's `capture()` over `AtlasState.records`.
///
/// # Errors
///
/// No atlas composed, or the atlas's own refusal (an unknown region).
pub fn capture_site(w: &World, frame: &WindowFrame) -> Result<CompositeInput, String> {
    let atlas = w
        .module_state::<mush_atlas::AtlasState>()
        .ok_or_else(|| "no atlas is composed".to_owned())?;
    capture(&atlas.records, frame)
}

/// Pure (off-loop-able): `composite()` then `site_geology()`.
///
/// # Errors
///
/// The atlas's refusal, "a site's ground is 256 by 256 cells of 10 m" for any other window, or
/// the geology's refusal.
pub fn build_site_ground(
    input: &CompositeInput,
    spec: &SiteGeologySpec,
    body: &BodyGeology,
    mac: &GeologyMac,
    key_version: u32,
) -> Result<SiteGround, String> {
    const WRONG_SHAPE: &str = "a site's ground is 256 by 256 cells of 10 m";
    let built = composite(input)?;
    let ground = built
        .surface(&mush_atlas::SurfaceRef::Ground)
        .ok_or_else(|| WRONG_SHAPE.to_owned())?;
    if built.frame.cell_cm != SITE_CELL_CM
        || ground.cells.width() != SITE_CELLS
        || ground.cells.height() != SITE_CELLS
    {
        return Err(WRONG_SHAPE.to_owned());
    }
    let geology = site_geology(mac, key_version, body, spec).map_err(|error| error.to_string())?;
    Ok(SiteGround {
        composite: Arc::new(built),
        geology: Arc::new(geology),
    })
}

/// The site ground LRU: at most [`SITE_CACHE_CAPACITY`] entries, each with its last-use tick on
/// one `u64` clock. RAM only.
#[derive(Clone, Debug, Default)]
pub struct SiteGroundCache {
    entries: Vec<(SiteGroundKey, Arc<SiteGround>, u64)>,
    clock: u64,
}

impl SiteGroundCache {
    /// The next tick.
    fn tick(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    /// A hit (and touch it), or `None`.
    pub fn get(&mut self, key: &SiteGroundKey) -> Option<Arc<SiteGround>> {
        let now = self.tick();
        let entry = self.entries.iter_mut().find(|(k, ..)| k == key)?;
        entry.2 = now;
        Some(Arc::clone(&entry.1))
    }

    /// Insert (or replace) an entry, evicting the least recently used at capacity.
    pub fn insert(&mut self, key: SiteGroundKey, ground: SiteGround) -> Arc<SiteGround> {
        let now = self.tick();
        let ground = Arc::new(ground);
        if let Some(entry) = self.entries.iter_mut().find(|(k, ..)| *k == key) {
            entry.1 = Arc::clone(&ground);
            entry.2 = now;
            return ground;
        }
        if self.entries.len() >= SITE_CACHE_CAPACITY
            && let Some(oldest) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, (.., used))| *used)
                .map(|(at, _)| at)
        {
            self.entries.swap_remove(oldest);
        }
        self.entries.push((key, Arc::clone(&ground), now));
        ground
    }

    /// Drop every entry built for `body`: its table (or radius) is not in the key, so the one
    /// writer of tables (`crate::surface::install_body_geology`) forgets the body's sites when it
    /// replaces them. Returns how many it dropped.
    pub(crate) fn forget_body(&mut self, body: &CanonicalId) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|(_, ground, _)| ground.geology.body() != body);
        before.saturating_sub(self.entries.len())
    }

    /// How many entries it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// **The synchronous entry point** (lead ruling 4): capture, key, hit or build-and-insert.
/// SM.4 moves player paths onto an off-loop driver; this stays the staff/test door.
///
/// # Errors
///
/// The disabled layer's reason, no table for the body, the capture's or the build's refusal.
pub fn site_ground_sync(
    w: &mut World,
    frame: &WindowFrame,
    spec: &SiteGeologySpec,
) -> Result<Arc<SiteGround>, String> {
    let (mac, key_version) = live(w).map_err(|disabled| disabled.reason())?;
    let body = body_geology(w, &spec.body)
        .ok_or_else(|| format!("No geology table is loaded for {}.", spec.body))?;
    let input = capture_site(w, frame)?;
    let key = SiteGroundKey {
        region: frame.region.clone(),
        revision: site_revision(&input, spec)?,
        key_version,
    };
    let state = w
        .module_state_mut::<GeologyState>()
        .ok_or_else(|| "no geology state is composed".to_owned())?;
    if let Some(hit) = state.sites.get(&key) {
        return Ok(hit);
    }
    let ground = build_site_ground(&input, spec, &body, &mac, key_version)?;
    Ok(state.sites.insert(key, ground))
}
