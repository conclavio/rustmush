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
//! whenever it replaces one. [`site_ground_sync`] is the synchronous door (staff and tests).
//!
//! **The revision is cached beside the ground** (doc 20 cut SM.4a's pre-commit round, item 1;
//! the SM.4a brief's K14 STOP: the warm peek re-hashed the whole input on every `+tv` status,
//! every scan command and every offer projection, 12 ms a peek in a debug build): each entry
//! keeps the captured input and the spec its revision was computed over, so a peek whose fresh
//! capture **equals** an entry's (structural equality, no serialise and no hash) is that entry —
//! the hash is paid once, when an input is first seen (the build, on the tick). Equality of the
//! whole input is the key rather than the atlas records' own `revision` stamps because those
//! restart at 1 when a pack is retired and applied again (`mush_atlas::apply`'s
//! `drop_pack_records`): a stamp list could match a changed ground. An input no entry captured
//! falls back to the full [`site_revision`] lookup (a cold site, or an entry inserted by
//! [`SiteGroundCache::insert`] without its capture).
//!
//! **Doc 20 cut SM.4a** (lead ruling 6, D11) keeps every build off the act's path rather than
//! off the loop: [`site_ground_cached`] is the **non-building peek** the command, the start's
//! offer and `+tv` status read (a miss answers "still being mapped" and asks for a warm), and
//! the `tacsim.surface` tick warms at most one wanted site a fire through
//! [`site_ground_sync`] (`crate::surface::warm_one`). The finish never needs the composite:
//! the start freezes the stand, the footing and the fields the mast sees (cells are never
//! edited, D-14-9). **Residual (SM.12's cost lane)**: a true off-loop build — a host-held keyed
//! runner behind `Effect::ModuleAsync`, the composite half on the atlas's own
//! `CompositeJob`/`run_async` — if the measured cold build asks for one.
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

/// **The shipped default** of a site's cells along one side (§3.1: 256 × 256 at 10 m, D-16-5's
/// arena window): every region shipped today is 2,560 m a side.
///
/// **Doc 32 cut CF.2** (Q-CF-51): the shipped default of CF.0's per-region window side
/// ([`crate::definitions::arena::DEFAULT_WINDOW_SIDE`]), not a side of its own. **Doc 32 cut
/// S.0** (§18): a site's own side is its region's extent ([`site_side_of_region`]), read where a
/// site is in reach ([`ground_side`], `crate::surface::chart::site_side`); this constant remains
/// only where cut S.1 has not yet scaled the scheme — the address blocks and pockets, the chart,
/// the geology table, the pad — and as the side of a record built without a site in reach.
pub const SITE_CELLS: u16 = crate::definitions::arena::DEFAULT_WINDOW_SIDE;
/// **The most cells along a site's side** (doc 32 cut S.0, §18): the arena's own cap
/// ([`crate::definitions::arena::MAX_ARENA_SIDE`]), which is the atlas composite's
/// ([`mush_atlas::MAX_COMPOSITE_SIDE`]).
pub const MAX_SITE_CELLS: u16 = crate::definitions::arena::MAX_ARENA_SIDE;
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

#[cfg(test)]
impl SiteGeology {
    /// **A synthetic table for unit tests** (doc 20 cut SM.6): every pocket barren and 0 t
    /// except the listed `(site-local pocket, family, class, tonnes)` slots; fields blank.
    /// What a pure consumer of the table (`crate::surface::opening::opening_plan`) reads,
    /// with no key and no body.
    pub(crate) fn for_test(ore: &[((i64, i64), OreFamily, Word, u8)]) -> Self {
        let pocket_count = usize::try_from(SITE_POCKETS * SITE_POCKETS).unwrap_or(0);
        let mut pockets = vec![[encode(Word::Barren, 0); 4]; pocket_count];
        for ((px, py), family, class, tonnes) in ore {
            let slot = OreFamily::ALL
                .iter()
                .position(|f| f == family)
                .unwrap_or_else(|| panic!("a family"));
            let index = local_index(*px, *py, SITE_POCKETS).unwrap_or_else(|| panic!("on site"));
            if let Some(code) = pockets.get_mut(index).and_then(|row| row.get_mut(slot)) {
                *code = encode(*class, *tonnes);
            }
        }
        Self {
            body: CanonicalId::new("ceres").unwrap_or_else(|_| panic!("an id")),
            origin: (0, 0),
            key_version: 1,
            shifts: [0; 16],
            fields: vec![
                FieldCode {
                    words: [Word::Barren; 4],
                    counts: [0; 4],
                };
                64
            ],
            pockets,
        }
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
/// The atlas's refusal, "a site's ground is 256 by 256 cells of 10 m" for any other window (the
/// side the region's: doc 32 cut S.0), a
/// region that is not a surface of the spec's body (doc 20 cut SM.2, the brief's lead override
/// 4: SM.1's residual SM1-A7), or the geology's refusal.
pub fn build_site_ground(
    input: &CompositeInput,
    spec: &SiteGeologySpec,
    body: &BodyGeology,
    mac: &GeologyMac,
    key_version: u32,
) -> Result<SiteGround, String> {
    let built = site_composite(input, &spec.body)?;
    let geology = site_geology(mac, key_version, body, spec).map_err(|error| error.to_string())?;
    Ok(SiteGround {
        composite: Arc::new(built),
        geology: Arc::new(geology),
    })
}

/// **A site's side from its region** (doc 32 cut S.0, §18): the region's extent in cells of
/// [`SITE_CELL_CM`] — square, a whole number of cells, `1..=`[`MAX_SITE_CELLS`].
///
/// # Errors
///
/// "a site's region is square, whole 10 m cells, at most 512 a side; …" for any other extent.
pub fn site_side_of_region(region: &mush_atlas::RegionRec) -> Result<u16, String> {
    let refused = |what: &str| {
        format!(
            "a site's region is square, whole 10 m cells, at most {MAX_SITE_CELLS} a side; {} {what}",
            region.key
        )
    };
    let extent = region
        .extent
        .to_cm()
        .map_err(|_| refused("has no extent"))?;
    let (w, h) = (
        extent.max.x.saturating_sub(extent.min.x),
        extent.max.y.saturating_sub(extent.min.y),
    );
    let cell = i64::from(SITE_CELL_CM);
    if w != h {
        return Err(refused("is not square"));
    }
    if w <= 0 || w % cell != 0 {
        return Err(refused("is not whole cells"));
    }
    u16::try_from(w / cell)
        .ok()
        .filter(|side| (1..=MAX_SITE_CELLS).contains(side))
        .ok_or_else(|| refused("is too wide"))
}

/// **A site composite's side** (doc 32 cut S.0, §18): its ground surface's cells along one side,
/// which `site_composite` holds to the region's square extent; [`SITE_CELLS`] for a composite
/// without a ground surface (none that `site_composite` passes).
#[must_use]
pub fn ground_side(built: &Composite) -> u16 {
    built
        .surface(&mush_atlas::SurfaceRef::Ground)
        .map_or(SITE_CELLS, |ground| ground.cells.width())
}

/// **A site's terrain composite, shape-checked** — the one rule [`build_site_ground`] and
/// [`site_terrain`] share (doc 20 cut SM.4b factored it out; one rule, never a copy): the
/// captured region a surface of `body` (cut SM.2's lead override 4), then `composite()`, then a
/// ground surface of side × side cells of 10 m, the side the region's extent
/// ([`site_side_of_region`]; doc 32 cut S.0: 256 for every shipped region).
///
/// # Errors
///
/// The region on another body or not a surface, its extent refused, the atlas's refusal, or
/// another shape.
fn site_composite(input: &CompositeInput, body: &CanonicalId) -> Result<Composite, String> {
    // Doc 20 cut SM.2 (lead override 4): the captured region stands on the spec's body, or
    // the site's ground would read one body's terrain over another's geology.
    match &input.region.kind {
        mush_atlas::record::RegionKind::Surface { body: on } if on.as_str() == body.as_str() => {}
        mush_atlas::record::RegionKind::Surface { body: on } => {
            return Err(format!(
                "the site names {body}, but region {} is on {on}",
                input.region.key
            ));
        }
        mush_atlas::record::RegionKind::Orbital { .. }
        | mush_atlas::record::RegionKind::System
        | mush_atlas::record::RegionKind::Interstellar => {
            return Err(format!(
                "the site names {body}, but region {} is not a surface",
                input.region.key
            ));
        }
    }
    let side = site_side_of_region(&input.region)?;
    let wrong_shape = || format!("a site's ground is {side} by {side} cells of 10 m");
    let built = composite(input)?;
    let ground = built
        .surface(&mush_atlas::SurfaceRef::Ground)
        .ok_or_else(wrong_shape)?;
    if built.frame.cell_cm != SITE_CELL_CM
        || ground.cells.width() != side
        || ground.cells.height() != side
    {
        return Err(wrong_shape());
    }
    Ok(built)
}

/// **A lane site's terrain, keyless** (doc 20 cut SM.4b; the brief's reviewer resolution R3):
/// the site's location through `crate::surface::chart::lane_sites` (no key read), its window
/// (`chart::site_frame`), [`capture_site`] and `site_composite`'s one rule — **no geology key
/// is read**, so a rig's way home is built even while the geology layer is dark. Its callers:
/// the home field's warm when the site reads dark (`crate::surface::warm_one`) and the freeze
/// census (`tests/driving.rs`). On-loop and as cheap as the warm's own composite half.
///
/// # Errors
///
/// Not a lane site, its window does not form, the capture's or the composite's refusal.
pub fn site_terrain(w: &World, entry: &CanonicalId) -> Result<Composite, String> {
    site_terrain_and_biome(w, entry).map(|(composite, _)| composite)
}

/// **[`site_terrain`] and its region's biome** (doc 32 cut CF.2): the field a site promotes into
/// is built from exactly this ground, on keyless terrain, so a dark geology layer never stops a
/// fight.
///
/// # Errors
///
/// As [`site_terrain`]'s.
pub fn site_terrain_and_biome(
    w: &World,
    entry: &CanonicalId,
) -> Result<(Composite, String), String> {
    let site = crate::surface::chart::lane_sites(w)
        .into_iter()
        .find(|site| &site.entry == entry)
        .ok_or_else(|| format!("{entry} is not a lane site"))?;
    let frame = crate::surface::chart::site_frame(w, &site.location)
        .map_err(|error| format!("{entry}'s window does not form: {error}"))?;
    let input = capture_site(w, &frame)?;
    let biome = input.region.biome_str().to_owned();
    site_composite(&input, &site.body).map(|composite| (composite, biome))
}

/// One cached site: its key, its ground, what its revision was computed over (the captured input
/// and the spec; `None` for an entry [`SiteGroundCache::insert`]ed without them) and its
/// last-use tick.
#[derive(Clone, Debug)]
struct CachedSite {
    key: SiteGroundKey,
    ground: Arc<SiteGround>,
    captured: Option<(Arc<CompositeInput>, SiteGeologySpec)>,
    used: u64,
}

impl CachedSite {
    /// Whether this entry's revision was computed over exactly `input` and `spec` under
    /// `key_version` (the pre-commit round's item 1: structural equality, no serialise, no hash).
    fn captured(&self, input: &CompositeInput, spec: &SiteGeologySpec, key_version: u32) -> bool {
        self.key.key_version == key_version
            && self.key.region == input.frame.region
            && self
                .captured
                .as_ref()
                .is_some_and(|(seen, seen_spec)| seen_spec == spec && **seen == *input)
    }
}

/// The site ground LRU: at most [`SITE_CACHE_CAPACITY`] entries, each with its last-use tick on
/// one `u64` clock and — since cut SM.4a's pre-commit round (item 1) — the captured input and
/// spec its [`site_revision`] was computed over, so a warm peek finds its entry by equality and
/// never re-hashes. RAM only.
#[derive(Clone, Debug, Default)]
pub struct SiteGroundCache {
    entries: Vec<CachedSite>,
    clock: u64,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's composite churn): the entries inserted
    /// under a key not already present. RAM only, never persisted, never shown to a player.
    builds: u64,
    /// **Doc 20 cut SM.12c**: the least-recently-used entries dropped at capacity. RAM only, never
    /// persisted, never shown to a player.
    evictions: u64,
}

/// **The site-ground LRU's counters** (doc 20 cut SM.12c, the cost lane; doc 20 §11's composite
/// churn): the entries it holds, the builds inserted under a new key and the evictions at
/// capacity, this boot. A present key replaced moves neither count, and a body's table replaced
/// (`forget_body`) drops entries without counting an eviction — a table replacement, not churn.
/// RAM only, never persisted, never shown to a player.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiteCacheCounters {
    /// The entries held now.
    pub entries: u32,
    /// The entries inserted under a key not already present.
    pub builds: u64,
    /// The least-recently-used entries dropped at capacity.
    pub evictions: u64,
}

impl SiteGroundCache {
    /// **The cache's counters** (doc 20 cut SM.12c, the cost lane): read only.
    #[must_use]
    pub fn counters(&self) -> SiteCacheCounters {
        SiteCacheCounters {
            entries: u32::try_from(self.entries.len()).unwrap_or(u32::MAX),
            builds: self.builds,
            evictions: self.evictions,
        }
    }

    /// The next tick.
    fn tick(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    /// **A hit without touching it** (doc 20 cut SM.4a, lead ruling 6): the read-only peek every
    /// player path takes, so a `&World` reader never needs `&mut` and never builds.
    #[must_use]
    pub fn peek(&self, key: &SiteGroundKey) -> Option<Arc<SiteGround>> {
        self.entries
            .iter()
            .find(|entry| entry.key == *key)
            .map(|entry| Arc::clone(&entry.ground))
    }

    /// **The warm peek by capture** (doc 20 cut SM.4a's pre-commit round, item 1): the entry
    /// whose revision was computed over exactly `input` and `spec` under `key_version`, without
    /// touching it — no serialise and no hash, one structural comparison an entry of the region.
    #[must_use]
    pub fn peek_captured(
        &self,
        input: &CompositeInput,
        spec: &SiteGeologySpec,
        key_version: u32,
    ) -> Option<Arc<SiteGround>> {
        self.entries
            .iter()
            .find(|entry| entry.captured(input, spec, key_version))
            .map(|entry| Arc::clone(&entry.ground))
    }

    /// **The revision of `input` and `spec`** (the pre-commit round's item 1): the cached one when
    /// an entry was built over exactly them under `key_version` — computed once, when the input
    /// was first seen — else [`site_revision`] in full.
    ///
    /// # Errors
    ///
    /// [`site_revision`]'s, on a miss.
    pub fn revision_of(
        &self,
        input: &CompositeInput,
        spec: &SiteGeologySpec,
        key_version: u32,
    ) -> Result<u64, String> {
        match self
            .entries
            .iter()
            .find(|entry| entry.captured(input, spec, key_version))
        {
            Some(entry) => Ok(entry.key.revision),
            None => site_revision(input, spec),
        }
    }

    /// A hit (and touch it), or `None`.
    pub fn get(&mut self, key: &SiteGroundKey) -> Option<Arc<SiteGround>> {
        let now = self.tick();
        let entry = self.entries.iter_mut().find(|entry| entry.key == *key)?;
        entry.used = now;
        Some(Arc::clone(&entry.ground))
    }

    /// [`SiteGroundCache::peek_captured`]'s hit, touched.
    fn get_captured(
        &mut self,
        input: &CompositeInput,
        spec: &SiteGeologySpec,
        key_version: u32,
    ) -> Option<Arc<SiteGround>> {
        let now = self.tick();
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.captured(input, spec, key_version))?;
        entry.used = now;
        Some(Arc::clone(&entry.ground))
    }

    /// Insert (or replace) an entry, evicting the least recently used at capacity. It remembers
    /// no capture: only a lookup by `key` finds it ([`SiteGroundCache::insert_captured`] is the
    /// build's).
    pub fn insert(&mut self, key: SiteGroundKey, ground: SiteGround) -> Arc<SiteGround> {
        self.put(key, ground, None)
    }

    /// **Insert the build of `input` and `spec`** (the pre-commit round's item 1): as
    /// [`SiteGroundCache::insert`], remembering what `key.revision` was computed over so the
    /// next peek of the same capture finds it by equality.
    pub fn insert_captured(
        &mut self,
        key: SiteGroundKey,
        ground: SiteGround,
        input: CompositeInput,
        spec: SiteGeologySpec,
    ) -> Arc<SiteGround> {
        self.put(key, ground, Some((Arc::new(input), spec)))
    }

    /// The one insert.
    fn put(
        &mut self,
        key: SiteGroundKey,
        ground: SiteGround,
        captured: Option<(Arc<CompositeInput>, SiteGeologySpec)>,
    ) -> Arc<SiteGround> {
        let now = self.tick();
        let ground = Arc::new(ground);
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.key == key) {
            entry.ground = Arc::clone(&ground);
            entry.captured = captured;
            entry.used = now;
            return ground;
        }
        if self.entries.len() >= SITE_CACHE_CAPACITY
            && let Some(oldest) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(at, _)| at)
        {
            self.entries.swap_remove(oldest);
            self.evictions = self.evictions.saturating_add(1);
        }
        self.builds = self.builds.saturating_add(1);
        self.entries.push(CachedSite {
            key,
            ground: Arc::clone(&ground),
            captured,
            used: now,
        });
        ground
    }

    /// Drop every entry built for `body`: its table (or radius) is not in the key, so the one
    /// writer of tables (`crate::surface::install_body_geology`) forgets the body's sites when it
    /// replaces them. Returns how many it dropped. **Doc 20 cut SM.12c**: a table replacement, not
    /// churn — the dropped entries are never counted as evictions.
    pub(crate) fn forget_body(&mut self, body: &CanonicalId) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.ground.geology.body() != body);
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

/// **The non-building peek** (doc 20 cut SM.4a; lead ruling 6, D11): capture, key, look up —
/// **never builds**. `Ok(None)` is a cold site (the caller asks for a warm,
/// `crate::surface::want_site_ground`). The command, the start's offer and `+tv` status read
/// the composite through this alone, at most once per site per call; the cost of the capture
/// is measured by `tests/knowing.rs` (K14). **Since the pre-commit round's item 1 a warm peek
/// pays the capture and one structural comparison, never [`site_revision`]'s serialise-and-hash**
/// ([`SiteGroundCache::peek_captured`]); only a capture no entry was built over (a cold site)
/// pays the full revision, to find an entry inserted without its capture.
///
/// # Errors
///
/// The disabled layer's reason, or the capture's refusal.
pub fn site_ground_cached(
    w: &World,
    frame: &WindowFrame,
    spec: &SiteGeologySpec,
) -> Result<Option<Arc<SiteGround>>, String> {
    let (_, key_version) = live(w).map_err(|disabled| disabled.reason())?;
    let input = capture_site(w, frame)?;
    let Some(state) = w.module_state::<GeologyState>() else {
        return Ok(None);
    };
    if let Some(hit) = state.sites.peek_captured(&input, spec, key_version) {
        return Ok(Some(hit));
    }
    let key = SiteGroundKey {
        region: frame.region.clone(),
        revision: site_revision(&input, spec)?,
        key_version,
    };
    Ok(state.sites.peek(&key))
}

/// **The synchronous entry point** (lead ruling 4): capture, key, hit or build-and-insert.
/// Since doc 20 cut SM.4a its only production caller is the surface tick's warm
/// (`crate::surface::warm_one`, one site a fire, never on an act's path); staff and tests
/// call it directly.
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
    let state = w
        .module_state_mut::<GeologyState>()
        .ok_or_else(|| "no geology state is composed".to_owned())?;
    // The pre-commit round's item 1: an entry built over this very capture is a hit without
    // the hash; otherwise the revision is computed once, here, and kept beside the build.
    if let Some(hit) = state.sites.get_captured(&input, spec, key_version) {
        return Ok(hit);
    }
    let key = SiteGroundKey {
        region: frame.region.clone(),
        revision: site_revision(&input, spec)?,
        key_version,
    };
    if let Some(hit) = state.sites.get(&key) {
        return Ok(hit);
    }
    let ground = build_site_ground(&input, spec, &body, &mac, key_version)?;
    Ok(state
        .sites
        .insert_captured(key, ground, input, spec.clone()))
}

#[cfg(test)]
mod s0_tests {
    use super::*;
    use mush_atlas::geom::{Point2, Rect2};
    use mush_atlas::record::{BackgroundRecipe, RegionKind};
    use mush_atlas::{RegionRec, TerrainClass};

    fn cid(s: &str) -> CanonicalId {
        CanonicalId::new(s).expect("an id")
    }

    /// A Ceres surface region `w × h` metres, Ahuna Flank's recipe in miniature.
    fn region(w: f64, h: f64) -> RegionRec {
        RegionRec {
            key: RecordKey::new(cid("s0"), cid("site")),
            name: "S.0 test site".to_owned(),
            kind: RegionKind::Surface {
                body: "ceres".to_owned(),
            },
            frame: None,
            parent: None,
            extent: Rect2(Point2(0.0, 0.0), Point2(w, h)),
            datum_cm: 0,
            background: BackgroundRecipe {
                surface: TerrainClass::Regolith,
                mix: vec![(TerrainClass::Ice, 20), (TerrainClass::Rock, 15)],
                relief_cm: 150,
                relief_period_cm: 1_000,
                water_cm: None,
                seed: 2026,
            },
            ground: Vec::new(),
            ground_resolution_cm: 1_000,
            theme: None,
            biome: None,
            revision: 1,
        }
    }

    /// The site window over `region` at `cells` a side, as `chart::site_frame` frames it.
    fn input(region: RegionRec, cells: i64) -> CompositeInput {
        let edge = cells * i64::from(SITE_CELL_CM);
        CompositeInput {
            frame: WindowFrame {
                region: region.key.clone(),
                surfaces: vec![mush_atlas::SurfaceRef::Ground],
                bbox: mush_atlas::RectCm {
                    min: mush_atlas::PointCm { x: 0, y: 0 },
                    max: mush_atlas::PointCm { x: edge, y: edge },
                },
                cell_cm: SITE_CELL_CM,
            },
            region,
            tiles: Vec::new(),
            features: Vec::new(),
            rooms: Vec::new(),
        }
    }

    /// **A 512 site composites** (doc 32 cut S.0, §18): a 5,120 m region is 512 × 512 cells of
    /// 10 m through `site_composite`, inside the composite's byte cap, and its side reads 512.
    #[test]
    fn s0_a_5120_m_region_composites_to_512() {
        let built = site_composite(&input(region(5_120.0, 5_120.0), 512), &cid("ceres"))
            .expect("a 512 site composites");
        let ground = built
            .surface(&mush_atlas::SurfaceRef::Ground)
            .expect("a ground surface");
        assert_eq!((ground.cells.width(), ground.cells.height()), (512, 512));
        assert_eq!(ground_side(&built), 512);
        // The composite crosses the async seam at 512 (the composite's own deserializer).
        let json = serde_json::to_string(&built).expect("encodes");
        let back: Composite = serde_json::from_str(&json).expect("decodes");
        assert_eq!(back, built);
        // The shipped 2,560 m region is still 256.
        let shipped = site_composite(&input(region(2_560.0, 2_560.0), 256), &cid("ceres"))
            .expect("a 256 site composites");
        assert_eq!(ground_side(&shipped), 256);
    }

    /// **The cap is enforced** (doc 32 cut S.0): a 5,130 m region (513 cells), a non-square one
    /// and one of part cells are refused; a window that is not the region's side keeps CF.7's
    /// refusal sentence.
    #[test]
    fn s0_an_extent_past_512_or_not_square_is_refused() {
        assert_eq!(site_side_of_region(&region(2_560.0, 2_560.0)), Ok(256));
        assert_eq!(site_side_of_region(&region(5_120.0, 5_120.0)), Ok(512));
        let refused = |w, h| site_side_of_region(&region(w, h)).expect_err("refused");
        assert!(refused(5_130.0, 5_130.0).contains("too wide"));
        assert!(refused(5_120.0, 2_560.0).contains("not square"));
        assert!(refused(2_565.0, 2_565.0).contains("whole cells"));
        assert!(
            site_composite(&input(region(5_130.0, 5_130.0), 513), &cid("ceres"))
                .expect_err("513 refused")
                .contains("at most 512")
        );
        assert_eq!(
            site_composite(&input(region(2_560.0, 2_560.0), 512), &cid("ceres")),
            Err("a site's ground is 256 by 256 cells of 10 m".to_owned())
        );
    }
}
