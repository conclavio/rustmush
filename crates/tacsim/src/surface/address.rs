//! **A site's names and the instruments' reach** (doc 20 cut SM.4a; §3.1, §5, §11's
//! Instruments row; the SM.4a brief's 3.1 and decisions D6 and D19; doc 32 cut S.1, §19).
//!
//! Pure integer arithmetic over the site-local grid, origin at the **south-west**, `x` east and
//! `y` north — the way `crate::port::def::SiteDef::shifts`, `crate::surface::excursion::ramp_pose`
//! and the atlas composite all read it (the composite's cell `(x, y)` is the site cell
//! `(x_cm / 1000, y_cm / 1000)` of a `Direct` pose on the site's passive domain:
//! `mush_atlas::raster::Grid::cell_of` over the frame's bbox from `(0, 0)`, pinned below).
//!
//! **The scheme scales with the site** (doc 32 cut S.1, the owner's ruling 3): a site of `side`
//! cells ([`Scheme`]) is always a 4 × 4 grid of blocks lettered `A`–`P`, each `side / 4` cells;
//! pockets stay 4 cells and fields 32. At the shipped 256:
//!
//! - a **cell** is 10 m, 256 a side; a **pocket** 4 × 4 cells, 64 a side; a **field** 8 × 8
//!   pockets (32 cells), 8 a side; a **block** 2 × 2 fields (64 cells), 4 a side;
//! - a block's **letter** reads row by row from the north-west (`BLOCK_LETTERS`: `A`–`D` the
//!   northern row … `M`–`P` the southern); a pocket's **address** is its block letter and its
//!   number `1..=256` row-major from the block's north-west (`C117`); a field's **name** is its
//!   block letter and its quadrant (`C-NW`);
//!
//! and at 512 a block is 128 cells, 32 × 32 pockets (numbers `1..=1024`, so up to `C1024`) and
//! 4 × 4 fields, a field's name its block letter, its quadrant of the block and its quadrant of
//! that quadrant (`C-NW-SE`). The functions outside [`Scheme`] are the shipped scheme's.
//!
//! - **the reach rule** (D6): a unit (a pocket, a field) is in an instrument's reach when its
//!   centre lies within `radius` cells of the stand cell's centre on both axes — doc 20 §5's
//!   "every pocket within sixteen cells on each axis", a per-axis square — computed in doubled
//!   cells so nothing is fractional. By construction the scan (48) reaches the stand's own field
//!   and its eight neighbours from any cell, the sounding (16) exactly 64 pockets (a whole field
//!   from its middle) and the quiet sounding (8) exactly 16;
//! - the **octant** of a bearing by the integer 2:5 rule (D19), spelled as the atlas's
//!   `Compass` words.
//!
//! No RNG, no `f64`, nothing that can panic.

use std::ops::RangeInclusive;

use crate::port::def::BLOCK_LETTERS;
use crate::surface::ground::{MAX_SITE_CELLS, SITE_CELLS};

/// Cells along a pocket's side (§3.1: 40 m pockets of 10 m cells).
pub const CELLS_PER_POCKET: i64 = 4;
/// Cells along a field's side (§3.1: 320 m fields).
pub const CELLS_PER_FIELD: i64 = 32;
/// Cells along a block's side on the shipped site (§3.1: 640 m blocks; [`Scheme::block_cells`]).
pub const CELLS_PER_BLOCK: i64 = Scheme::SHIPPED.block_cells();
/// Pockets along a block's side on the shipped site ([`Scheme::block_pockets`]).
pub const POCKETS_PER_BLOCK: i64 = Scheme::SHIPPED.block_pockets();
/// Pockets in one block of the shipped site: an address's number runs `1..=256`
/// ([`Scheme::pockets_in_block`]).
pub const POCKETS_IN_BLOCK: i64 = Scheme::SHIPPED.pockets_in_block();
/// Blocks along a site's side, at every side (ruling 3: always `A`–`P`).
pub const BLOCKS: i64 = 4;
/// **The side a scheme is a multiple of** (doc 32 cut S.1): a block holds a whole number of
/// pairs of fields (`side / 4` cells, a multiple of 64), so its fields read as quadrants.
pub const SCHEME_STEP: u16 = SITE_CELLS;

/// **A site's address scheme** (doc 32 cut S.1, ruling 3): its side in cells, a multiple of
/// [`SCHEME_STEP`] up to [`MAX_SITE_CELLS`] — 256 or 512. Every name, index and reach on a site
/// is read through its scheme; [`Scheme::SHIPPED`] is the 256 every shipped site is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scheme {
    /// Cells along the side.
    side: u16,
}

impl Default for Scheme {
    fn default() -> Self {
        Scheme::SHIPPED
    }
}

impl Scheme {
    /// The shipped site's scheme: 256 a side.
    pub const SHIPPED: Scheme = Scheme { side: SITE_CELLS };
    /// The widest site's scheme: [`MAX_SITE_CELLS`] a side.
    pub const WIDEST: Scheme = Scheme {
        side: MAX_SITE_CELLS,
    };

    /// **The scheme of a site `side` cells a side**, `None` for a side that is not a positive
    /// multiple of [`SCHEME_STEP`] up to [`MAX_SITE_CELLS`]
    /// (`crate::surface::ground::site_side_of_region` refuses such a region).
    #[must_use]
    pub const fn of_side(side: u16) -> Option<Scheme> {
        if side == 0 || !side.is_multiple_of(SCHEME_STEP) || side > MAX_SITE_CELLS {
            return None;
        }
        Some(Scheme { side })
    }

    /// **The scheme of a side already held to a site's** — a record's own `side`, a composite's —
    /// or the shipped scheme for one no site has (none that the validators pass).
    #[must_use]
    pub const fn of_site(side: u16) -> Scheme {
        match Scheme::of_side(side) {
            Some(scheme) => scheme,
            None => Scheme::SHIPPED,
        }
    }

    /// Cells along the side.
    #[must_use]
    pub const fn side(self) -> u16 {
        self.side
    }

    /// Cells along the side, as the grid's integer.
    const fn cells(self) -> i64 {
        self.side as i64
    }

    /// Cells along a block's side: `side / 4` (64 at 256, 128 at 512).
    #[must_use]
    pub const fn block_cells(self) -> i64 {
        self.cells() / BLOCKS
    }

    /// Pockets along a block's side (16 at 256, 32 at 512).
    #[must_use]
    pub const fn block_pockets(self) -> i64 {
        self.block_cells() / CELLS_PER_POCKET
    }

    /// Pockets in one block, and so the highest pocket number (256 at 256, 1,024 at 512).
    #[must_use]
    pub const fn pockets_in_block(self) -> i64 {
        self.block_pockets() * self.block_pockets()
    }

    /// Fields along a block's side (2 at 256, 4 at 512).
    #[must_use]
    pub const fn block_fields(self) -> i64 {
        self.block_cells() / CELLS_PER_FIELD
    }

    /// Pockets along the site's side (64 at 256, 128 at 512).
    #[must_use]
    pub const fn pockets(self) -> i64 {
        self.cells() / CELLS_PER_POCKET
    }

    /// Fields along the site's side (8 at 256, 16 at 512).
    #[must_use]
    pub const fn fields(self) -> i64 {
        self.cells() / CELLS_PER_FIELD
    }

    /// Fields on the site (64 at 256, 256 at 512): a field index runs `0..field_count`.
    #[must_use]
    pub const fn field_count(self) -> i64 {
        self.fields() * self.fields()
    }

    /// Whether a site-local cell lies on the site.
    #[must_use]
    pub fn cell_on_site(self, cell: (i64, i64)) -> bool {
        cell_on_site(cell, self.side)
    }

    /// The pocket a site cell lies in, `None` off the site.
    #[must_use]
    pub fn pocket_of_cell(self, cell: (i64, i64)) -> Option<(i64, i64)> {
        self.cell_on_site(cell).then(|| {
            (
                cell.0.div_euclid(CELLS_PER_POCKET),
                cell.1.div_euclid(CELLS_PER_POCKET),
            )
        })
    }

    /// The field a site cell lies in, `None` off the site.
    #[must_use]
    pub fn field_of_cell(self, cell: (i64, i64)) -> Option<(i64, i64)> {
        self.cell_on_site(cell).then(|| {
            (
                cell.0.div_euclid(CELLS_PER_FIELD),
                cell.1.div_euclid(CELLS_PER_FIELD),
            )
        })
    }

    /// The field a pocket lies in, `None` off the site.
    #[must_use]
    pub fn field_of_pocket(self, pocket: (i64, i64)) -> Option<(i64, i64)> {
        self.pocket_on_site(pocket).then(|| {
            let side = crate::surface::geology::POCKETS_PER_FIELD_SIDE;
            (pocket.0.div_euclid(side), pocket.1.div_euclid(side))
        })
    }

    /// Whether a pocket lies on the site.
    #[must_use]
    pub fn pocket_on_site(self, pocket: (i64, i64)) -> bool {
        (0..self.pockets()).contains(&pocket.0) && (0..self.pockets()).contains(&pocket.1)
    }

    /// Whether a field lies on the site.
    #[must_use]
    pub fn field_on_site(self, field: (i64, i64)) -> bool {
        (0..self.fields()).contains(&field.0) && (0..self.fields()).contains(&field.1)
    }

    /// **The block a field lies in**, `(bx, by)` from the south-west, `None` off the site.
    #[must_use]
    pub fn block_of_field(self, field: (i64, i64)) -> Option<(i64, i64)> {
        self.field_on_site(field).then(|| {
            (
                field.0.div_euclid(self.block_fields()),
                field.1.div_euclid(self.block_fields()),
            )
        })
    }

    /// **A field's block's index from the south-west** (`by · 4 + bx`, `0..16`): the per-block
    /// geology shift's slot (`crate::surface::ground::SiteGeologySpec::shifts`).
    #[must_use]
    pub fn block_index_of_field(self, field: (i64, i64)) -> Option<usize> {
        let (bx, by) = self.block_of_field(field)?;
        usize::try_from(by.checked_mul(BLOCKS)?.checked_add(bx)?).ok()
    }

    /// **The fields of a block**, south-west first, row by row: `None` for anything but `A`–`P`.
    #[must_use]
    pub fn fields_of_block(self, letter: char) -> Option<Vec<(i64, i64)>> {
        let (bx, by) = block_of_letter(letter)?;
        let n = self.block_fields();
        Some(
            (0..n)
                .flat_map(|dy| (0..n).map(move |dx| (bx * n + dx, by * n + dy)))
                .collect(),
        )
    }

    /// **The block letter of a pocket** (§3.1): `BLOCK_LETTERS[(3 − py/n)·4 + px/n]`, `n` the
    /// pockets along a block's side.
    #[must_use]
    pub fn block_letter(self, px: i64, py: i64) -> Option<char> {
        if !self.pocket_on_site((px, py)) {
            return None;
        }
        letter_of_block(
            px.div_euclid(self.block_pockets()),
            py.div_euclid(self.block_pockets()),
        )
    }

    /// **A pocket's address** (§3.1, §10's `C117`): its block letter and `n = (b − 1 − py%b)·b
    /// + px%b + 1`, `b` the pockets along a block's side, row-major from the block's north-west.
    #[must_use]
    pub fn pocket_address(self, px: i64, py: i64) -> Option<String> {
        let (letter, index) = self.pocket_in_block(px, py)?;
        Some(format!("{letter}{}", index.checked_add(1)?))
    }

    /// **A pocket's index in its block's cell string** (`n − 1`, `0..pockets_in_block`), with
    /// its block's letter: the chart's `PocketBlock.cells` byte for that pocket.
    #[must_use]
    pub fn pocket_in_block(self, px: i64, py: i64) -> Option<(char, usize)> {
        let letter = self.block_letter(px, py)?;
        let b = self.block_pockets();
        let n = (b - 1 - py.rem_euclid(b))
            .checked_mul(b)?
            .checked_add(px.rem_euclid(b))?;
        Some((letter, usize::try_from(n).ok()?))
    }

    /// **A pocket's number in its block** (doc 20 cut SM.6: the pocket cursor's key): the number
    /// part of [`Scheme::pocket_address`], row-major from the block's north-west.
    #[must_use]
    pub fn pocket_number(self, px: i64, py: i64) -> Option<u16> {
        let (_, index) = self.pocket_in_block(px, py)?;
        u16::try_from(index.checked_add(1)?).ok()
    }

    /// The inverse of [`Scheme::pocket_in_block`]: the site pocket of `letter`'s `index`th cell.
    #[must_use]
    pub fn pocket_of_block_index(self, letter: char, index: usize) -> Option<(i64, i64)> {
        let at = BLOCK_LETTERS.iter().position(|l| *l == letter)?;
        let at = i64::try_from(at).ok()?;
        let index = i64::try_from(index).ok()?;
        if !(0..self.pockets_in_block()).contains(&index) {
            return None;
        }
        let b = self.block_pockets();
        let bx = at % BLOCKS;
        let by = BLOCKS - 1 - at / BLOCKS;
        let px = bx * b + index % b;
        let py = by * b + (b - 1 - index / b);
        Some((px, py))
    }

    /// **The pockets of a block**, in cell-string order (`n − 1` ascending): `None` for anything
    /// but `A`–`P`.
    #[must_use]
    pub fn pockets_of_block(self, letter: char) -> Option<Vec<(i64, i64)>> {
        let count = usize::try_from(self.pockets_in_block()).ok()?;
        (0..count)
            .map(|index| self.pocket_of_block_index(letter, index))
            .collect()
    }

    /// **A field's name** (§10's `C-NW`): its block letter, then its quadrant in the block — `N`
    /// or `S`, then `W` or `E` — and on a block of 4 × 4 fields its quadrant in that quadrant
    /// (`C-NW-SE`).
    #[must_use]
    pub fn field_name(self, fx: i64, fy: i64) -> Option<String> {
        let (bx, by) = self.block_of_field((fx, fy))?;
        let letter = letter_of_block(bx, by)?;
        let n = self.block_fields();
        let (mut lx, mut ly) = (fx.rem_euclid(n), fy.rem_euclid(n));
        let mut name = String::from(letter);
        let mut half = n / 2;
        while half > 0 {
            let north = ly >= half;
            let east = lx >= half;
            name.push('-');
            name.push(if north { 'N' } else { 'S' });
            name.push(if east { 'E' } else { 'W' });
            lx %= half;
            ly %= half;
            half /= 2;
        }
        Some(name)
    }

    /// **A pocket address's pocket** (doc 20 cut SM.4b): the inverse of
    /// [`Scheme::pocket_address`] — `C117`, case-insensitive, strict: a block letter `A`–`P` and
    /// a number `1..=pockets_in_block` with no sign and no leading zero.
    #[must_use]
    pub fn parse_pocket_address(self, text: &str) -> Option<(i64, i64)> {
        let mut chars = text.chars();
        let letter = chars.next()?.to_ascii_uppercase();
        let number = chars.as_str();
        if number.is_empty()
            || number.len() > 4
            || !number.bytes().all(|b| b.is_ascii_digit())
            || number.starts_with('0')
        {
            return None;
        }
        let n: usize = number.parse().ok()?;
        if n == 0 || i64::try_from(n).ok()? > self.pockets_in_block() {
            return None;
        }
        self.pocket_of_block_index(letter, n.checked_sub(1)?)
    }

    /// **A field name's field** (doc 20 cut SM.4b): the inverse of [`Scheme::field_name`] —
    /// `C-NW` (or `C-NW-SE` at 512), case-insensitive: a block letter, then one hyphen, `N` or
    /// `S` and `W` or `E` for each level of quadrant the block holds.
    #[must_use]
    pub fn parse_field_name(self, text: &str) -> Option<(i64, i64)> {
        let upper = text.to_ascii_uppercase();
        let mut parts = upper.split('-');
        let mut letter = parts.next()?.chars();
        let (Some(letter), None) = (letter.next(), letter.next()) else {
            return None;
        };
        let (bx, by) = block_of_letter(letter)?;
        let n = self.block_fields();
        let (mut lx, mut ly) = (0, 0);
        let mut half = n / 2;
        while half > 0 {
            let part = parts.next()?;
            let mut chars = part.chars();
            let (Some(ns), Some(we), None) = (chars.next(), chars.next(), chars.next()) else {
                return None;
            };
            ly += match ns {
                'N' => half,
                'S' => 0,
                _ => return None,
            };
            lx += match we {
                'E' => half,
                'W' => 0,
                _ => return None,
            };
            half /= 2;
        }
        if parts.next().is_some() {
            return None;
        }
        Some((
            bx.checked_mul(n)?.checked_add(lx)?,
            by.checked_mul(n)?.checked_add(ly)?,
        ))
    }

    /// **A field's index on the site** (`fy · fields + fx`; `0..64` at 256, `0..256` at 512):
    /// the chart's field-row key.
    #[must_use]
    pub fn field_index(self, fx: i64, fy: i64) -> Option<u8> {
        if !self.field_on_site((fx, fy)) {
            return None;
        }
        u8::try_from(fy.checked_mul(self.fields())?.checked_add(fx)?).ok()
    }

    /// The inverse of [`Scheme::field_index`].
    #[must_use]
    pub fn field_of_index(self, index: u8) -> Option<(i64, i64)> {
        let index = i64::from(index);
        let field = (index % self.fields(), index / self.fields());
        self.field_on_site(field).then_some(field)
    }

    /// **The fields in reach** of `stand` at `radius` cells, on the site, north-west first (rows
    /// from the north, each west to east).
    #[must_use]
    pub fn fields_in_reach(self, stand: (i64, i64), radius: i64) -> Vec<(i64, i64)> {
        in_reach(stand, radius, CELLS_PER_FIELD, self.fields())
    }

    /// **The pockets in reach** of `stand` at `radius` cells, on the site, north-west first.
    #[must_use]
    pub fn pockets_in_reach(self, stand: (i64, i64), radius: i64) -> Vec<(i64, i64)> {
        in_reach(stand, radius, CELLS_PER_POCKET, self.pockets())
    }
}

/// Whether a site-local cell lies on a site of `side` cells a side (doc 32 cut S.0: the site's
/// own side, [`crate::surface::ground::ground_side`] or `crate::surface::chart::site_side`).
#[must_use]
pub fn cell_on_site(cell: (i64, i64), side: u16) -> bool {
    let side = i64::from(side);
    (0..side).contains(&cell.0) && (0..side).contains(&cell.1)
}

/// **The composite's cell for a site cell** (the atlas grid's `(u16, u16)`), `None` off a site
/// of `side` cells a side. The two grids are one (origin south-west, `y` north), so this only
/// narrows the type.
#[must_use]
pub fn composite_cell(cell: (i64, i64), side: u16) -> Option<(u16, u16)> {
    if !cell_on_site(cell, side) {
        return None;
    }
    Some((u16::try_from(cell.0).ok()?, u16::try_from(cell.1).ok()?))
}

/// **A cell of `surface`'s own grid** for a site cell: [`composite_cell`] at the surface's side.
#[must_use]
pub fn surface_cell(
    surface: &mush_atlas::window::SurfaceComposite,
    cell: (i64, i64),
) -> Option<(u16, u16)> {
    composite_cell(cell, surface.cells.width())
}

/// The shipped scheme's [`Scheme::pocket_of_cell`].
#[must_use]
pub fn pocket_of_cell(cell: (i64, i64)) -> Option<(i64, i64)> {
    Scheme::SHIPPED.pocket_of_cell(cell)
}

/// The shipped scheme's [`Scheme::field_of_cell`].
#[must_use]
pub fn field_of_cell(cell: (i64, i64)) -> Option<(i64, i64)> {
    Scheme::SHIPPED.field_of_cell(cell)
}

/// The shipped scheme's [`Scheme::field_of_pocket`].
#[must_use]
pub fn field_of_pocket(pocket: (i64, i64)) -> Option<(i64, i64)> {
    Scheme::SHIPPED.field_of_pocket(pocket)
}

/// **A block's letter by its south-west-based index** `(bx, by)`, `0..4` each, at every side:
/// the letter at `BLOCK_LETTERS[(3 − by) · 4 + bx]` (read from the north-west).
#[must_use]
pub fn letter_of_block(bx: i64, by: i64) -> Option<char> {
    if !(0..BLOCKS).contains(&bx) || !(0..BLOCKS).contains(&by) {
        return None;
    }
    let index = (BLOCKS - 1 - by).checked_mul(BLOCKS)?.checked_add(bx)?;
    BLOCK_LETTERS.get(usize::try_from(index).ok()?).copied()
}

/// The shipped scheme's [`Scheme::block_letter`].
#[must_use]
pub fn block_letter(px: i64, py: i64) -> Option<char> {
    Scheme::SHIPPED.block_letter(px, py)
}

/// The shipped scheme's [`Scheme::pocket_address`].
#[must_use]
pub fn pocket_address(px: i64, py: i64) -> Option<String> {
    Scheme::SHIPPED.pocket_address(px, py)
}

/// The shipped scheme's [`Scheme::pocket_in_block`].
#[must_use]
pub fn pocket_in_block(px: i64, py: i64) -> Option<(char, usize)> {
    Scheme::SHIPPED.pocket_in_block(px, py)
}

/// The shipped scheme's [`Scheme::pocket_number`].
#[must_use]
pub fn pocket_number(px: i64, py: i64) -> Option<u16> {
    Scheme::SHIPPED.pocket_number(px, py)
}

/// The shipped scheme's [`Scheme::pocket_of_block_index`].
#[must_use]
pub fn pocket_of_block_index(letter: char, index: usize) -> Option<(i64, i64)> {
    Scheme::SHIPPED.pocket_of_block_index(letter, index)
}

/// The shipped scheme's [`Scheme::field_name`].
#[must_use]
pub fn field_name(fx: i64, fy: i64) -> Option<String> {
    Scheme::SHIPPED.field_name(fx, fy)
}

/// **The block of a letter** (doc 20 cut SM.4b), at every side: the inverse of
/// [`letter_of_block`], `(bx, by)` from the south-west, case-insensitive; `None` for anything but
/// `A`–`P`.
#[must_use]
pub fn block_of_letter(letter: char) -> Option<(i64, i64)> {
    let upper = letter.to_ascii_uppercase();
    let at = i64::try_from(BLOCK_LETTERS.iter().position(|l| *l == upper)?).ok()?;
    Some((at % BLOCKS, BLOCKS - 1 - at / BLOCKS))
}

/// The shipped scheme's [`Scheme::parse_pocket_address`].
#[must_use]
pub fn parse_pocket_address(text: &str) -> Option<(i64, i64)> {
    Scheme::SHIPPED.parse_pocket_address(text)
}

/// The shipped scheme's [`Scheme::parse_field_name`].
#[must_use]
pub fn parse_field_name(text: &str) -> Option<(i64, i64)> {
    Scheme::SHIPPED.parse_field_name(text)
}

/// The shipped scheme's [`Scheme::field_index`].
#[must_use]
pub fn field_index(fx: i64, fy: i64) -> Option<u8> {
    Scheme::SHIPPED.field_index(fx, fy)
}

/// The shipped scheme's [`Scheme::field_of_index`].
#[must_use]
pub fn field_of_index(index: u8) -> Option<(i64, i64)> {
    Scheme::SHIPPED.field_of_index(index)
}

/// **The units of size `unit` cells in an instrument's reach along one axis** from stand cell
/// `stand` (D6): unit `a` (cells `a·unit ..= a·unit + unit − 1`) is in reach when `|(2·stand +
/// 1) − (2·a·unit + unit)| ≤ 2·radius`. Not clipped to the site (the `*_in_reach` functions
/// clip). An empty range for a non-positive `unit` or a negative radius.
#[must_use]
pub fn reach(stand: i64, unit: i64, radius: i64) -> RangeInclusive<i64> {
    if unit <= 0 || radius < 0 {
        return RangeInclusive::new(1, 0);
    }
    let twice = |v: i64| v.saturating_mul(2);
    let centre = twice(stand).saturating_add(1);
    // 2·a·unit + unit ∈ [centre − 2r, centre + 2r]  ⇔  a ∈ [⌈(lo − unit) / 2u⌉, ⌊(hi − unit) / 2u⌋].
    let lo = centre.saturating_sub(twice(radius)).saturating_sub(unit);
    let hi = centre.saturating_add(twice(radius)).saturating_sub(unit);
    let step = twice(unit);
    let first = lo.saturating_add(step - 1).div_euclid(step);
    let last = hi.div_euclid(step);
    first..=last
}

/// The shipped scheme's [`Scheme::fields_in_reach`].
#[must_use]
pub fn fields_in_reach(stand: (i64, i64), radius: i64) -> Vec<(i64, i64)> {
    Scheme::SHIPPED.fields_in_reach(stand, radius)
}

/// The shipped scheme's [`Scheme::pockets_in_reach`].
#[must_use]
pub fn pockets_in_reach(stand: (i64, i64), radius: i64) -> Vec<(i64, i64)> {
    Scheme::SHIPPED.pockets_in_reach(stand, radius)
}

/// The units of `unit` cells in reach, clipped to `0..side`, north-west first.
fn in_reach(stand: (i64, i64), radius: i64, unit: i64, side: i64) -> Vec<(i64, i64)> {
    let xs = reach(stand.0, unit, radius);
    let ys = reach(stand.1, unit, radius);
    let mut out = Vec::new();
    for y in ys.rev() {
        if !(0..side).contains(&y) {
            continue;
        }
        for x in xs.clone() {
            if (0..side).contains(&x) {
                out.push((x, y));
            }
        }
    }
    out
}

/// **A compass octant** (D19): a bearing by the integer 2:5 rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Octant {
    /// North.
    N,
    /// North-east.
    NE,
    /// East.
    E,
    /// South-east.
    SE,
    /// South.
    S,
    /// South-west.
    SW,
    /// West.
    W,
    /// North-west.
    NW,
}

impl Octant {
    /// **The octant of `(dx, dy)`** (`dx` east, `dy` north): `None` for `(0, 0)`; `5·|dy| ≤
    /// 2·|dx|` → east or west; `5·|dx| ≤ 2·|dy|` → north or south; else the diagonal by signs.
    /// Integer and deterministic (about 21.8° either side of an axis is that axis).
    #[must_use]
    pub fn of(dx: i64, dy: i64) -> Option<Octant> {
        if dx == 0 && dy == 0 {
            return None;
        }
        let ax = i128::from(dx).abs();
        let ay = i128::from(dy).abs();
        Some(if 5 * ay <= 2 * ax {
            if dx > 0 { Octant::E } else { Octant::W }
        } else if 5 * ax <= 2 * ay {
            if dy > 0 { Octant::N } else { Octant::S }
        } else {
            match (dx > 0, dy > 0) {
                (true, true) => Octant::NE,
                (true, false) => Octant::SE,
                (false, true) => Octant::NW,
                (false, false) => Octant::SW,
            }
        })
    }

    /// The short form: `N NE E SE S SW W NW`.
    #[must_use]
    pub fn abbrev(self) -> &'static str {
        match self {
            Octant::N => "N",
            Octant::NE => "NE",
            Octant::E => "E",
            Octant::SE => "SE",
            Octant::S => "S",
            Octant::SW => "SW",
            Octant::W => "W",
            Octant::NW => "NW",
        }
    }

    /// The word (the atlas `Compass::word` spellings).
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Octant::N => "north",
            Octant::NE => "northeast",
            Octant::E => "east",
            Octant::SE => "southeast",
            Octant::S => "south",
            Octant::SW => "southwest",
            Octant::W => "west",
            Octant::NW => "northwest",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::ground::{SITE_BLOCKS, SITE_FIELDS, SITE_POCKETS};

    /// The names read from the north-west: pocket (0, 63) is `A1`, (63, 0) is `P256`; the
    /// Dray-P's ramp cell (128, 124) is pocket (32, 31), `K1` in field `K-NW`; `C117`
    /// round-trips; the field quadrants; every pocket's address is unique and round-trips
    /// through its block index.
    #[test]
    fn sm4a_addresses_read_from_the_north_west() {
        assert_eq!(pocket_address(0, 63).as_deref(), Some("A1"));
        assert_eq!(pocket_address(63, 0).as_deref(), Some("P256"));
        assert_eq!(pocket_address(15, 48).as_deref(), Some("A256"));
        assert_eq!(pocket_address(16, 63).as_deref(), Some("B1"));
        assert_eq!(pocket_address(64, 0), None);
        assert_eq!(pocket_address(-1, 0), None);
        let ramp = pocket_of_cell((128, 124)).expect("on the site");
        assert_eq!(ramp, (32, 31));
        assert_eq!(pocket_address(ramp.0, ramp.1).as_deref(), Some("K1"));
        let field = field_of_cell((128, 124)).expect("on the site");
        assert_eq!(field, (4, 3));
        assert_eq!(field_name(field.0, field.1).as_deref(), Some("K-NW"));
        assert_eq!(field_name(0, 7).as_deref(), Some("A-NW"));
        assert_eq!(field_name(1, 6).as_deref(), Some("A-SE"));
        assert_eq!(field_name(7, 0).as_deref(), Some("P-SE"));
        assert_eq!(field_name(8, 0), None);
        // C117: block C (bx 2, by 3), n = 117 → row 7 from the north, column 4.
        let c117 = pocket_of_block_index('C', 116).expect("C117");
        assert_eq!(c117, (36, 56));
        assert_eq!(pocket_address(c117.0, c117.1).as_deref(), Some("C117"));
        assert_eq!(block_letter(c117.0, c117.1), Some('C'));
        let mut seen = std::collections::BTreeSet::new();
        for py in 0..SITE_POCKETS {
            for px in 0..SITE_POCKETS {
                let address = pocket_address(px, py).expect("an address");
                assert!(seen.insert(address), "unique");
                let (letter, index) = pocket_in_block(px, py).expect("a block index");
                assert_eq!(pocket_of_block_index(letter, index), Some((px, py)));
            }
        }
        for index in 0..64_u8 {
            let (fx, fy) = field_of_index(index).expect("a field");
            assert_eq!(field_index(fx, fy), Some(index));
        }
        assert_eq!(field_of_index(64), None);
        // The composite's grid is the site grid (origin south-west, y north): the atlas
        // raster's own `cell_of` over the frame's bbox agrees with `site_cell` everywhere.
        let grid = mush_atlas::raster::Grid::over(
            mush_atlas::RectCm {
                min: mush_atlas::PointCm { x: 0, y: 0 },
                max: mush_atlas::PointCm {
                    x: 256_000,
                    y: 256_000,
                },
            },
            1_000,
            u32::MAX,
        )
        .expect("a 256 × 256 grid");
        for (x, y) in [(0, 0), (128_000, 124_500), (255_999, 1), (12_345, 250_001)] {
            let cell = crate::surface::excursion::site_cell(x, y);
            assert_eq!(
                grid.cell_of(mush_atlas::PointCm { x, y }),
                composite_cell(cell, SITE_CELLS),
                "({x}, {y})"
            );
        }
    }

    /// **The reach is exact from every cell** (D6): over every stand cell of one field away
    /// from the site's edge, the scan (48) reads exactly nine fields — the stand's own and its
    /// eight neighbours — the sounding (16) exactly 64 pockets and the quiet sounding (8)
    /// exactly 16; from the field's middle the sounding reads exactly that field.
    #[test]
    fn sm4a_the_reach_is_exact_from_every_cell() {
        for x in 96..128 {
            for y in 96..128 {
                let fields = fields_in_reach((x, y), 48);
                assert_eq!(fields.len(), 9, "({x}, {y})");
                let own = field_of_cell((x, y)).expect("on the site");
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        assert!(fields.contains(&(own.0 + dx, own.1 + dy)), "({x}, {y})");
                    }
                }
                assert_eq!(pockets_in_reach((x, y), 16).len(), 64, "({x}, {y})");
                assert_eq!(pockets_in_reach((x, y), 8).len(), 16, "({x}, {y})");
            }
        }
        // From the field's middle (local 15..=17), the sounding is exactly the field.
        for local in 15..=17 {
            let stand = (96 + local, 96 + local);
            let mut pockets = pockets_in_reach(stand, 16);
            pockets.sort_unstable();
            let mut field: Vec<(i64, i64)> = (24..32)
                .flat_map(|py| (24..32).map(move |px| (px, py)))
                .collect();
            field.sort_unstable();
            assert_eq!(pockets, field, "{local}");
        }
        // North-west first.
        let first = fields_in_reach((112, 112), 48);
        assert_eq!(first.first(), Some(&(2, 4)));
        assert_eq!(first.last(), Some(&(4, 2)));
        // Clipped at the edge.
        assert_eq!(fields_in_reach((0, 0), 48).len(), 4);
        assert_eq!(pockets_in_reach((0, 0), 16).len(), 16);
        assert!(reach(5, 0, 3).is_empty());
    }

    /// The octants by the 2:5 rule.
    #[test]
    fn sm4a_octants() {
        assert_eq!(Octant::of(0, 0), None);
        assert_eq!(Octant::of(0, 9), Some(Octant::N));
        assert_eq!(Octant::of(0, -9), Some(Octant::S));
        assert_eq!(Octant::of(9, 0), Some(Octant::E));
        assert_eq!(Octant::of(-9, 0), Some(Octant::W));
        assert_eq!(Octant::of(5, 5), Some(Octant::NE));
        assert_eq!(Octant::of(5, -5), Some(Octant::SE));
        assert_eq!(Octant::of(-5, 5), Some(Octant::NW));
        assert_eq!(Octant::of(-5, -5), Some(Octant::SW));
        // 5·2 ≤ 2·5: east; 5·3 > 2·5 and 5·5 > 2·3: the diagonal.
        assert_eq!(Octant::of(5, 2), Some(Octant::E));
        assert_eq!(Octant::of(5, 3), Some(Octant::NE));
        assert_eq!(Octant::of(2, 5), Some(Octant::N));
        assert_eq!(Octant::of(i64::MAX, i64::MIN), Some(Octant::SE));
        assert_eq!(Octant::NW.abbrev(), "NW");
        assert_eq!(Octant::SW.word(), "southwest");
    }

    /// **Doc 20 cut SM.4b: the three inverses round-trip** — every pocket's address, every
    /// field's name and every block's letter parse back (in either case) to where they came
    /// from; malformed text is refused.
    #[test]
    fn sm4b_the_inverses_round_trip() {
        for py in 0..SITE_POCKETS {
            for px in 0..SITE_POCKETS {
                let text = pocket_address(px, py).expect("an address");
                assert_eq!(parse_pocket_address(&text), Some((px, py)), "{text}");
                assert_eq!(
                    parse_pocket_address(&text.to_ascii_lowercase()),
                    Some((px, py))
                );
            }
        }
        for fy in 0..SITE_FIELDS {
            for fx in 0..SITE_FIELDS {
                let text = field_name(fx, fy).expect("a name");
                assert_eq!(parse_field_name(&text), Some((fx, fy)), "{text}");
                assert_eq!(parse_field_name(&text.to_ascii_lowercase()), Some((fx, fy)));
            }
        }
        for by in 0..SITE_BLOCKS {
            for bx in 0..SITE_BLOCKS {
                let letter = letter_of_block(bx, by).expect("a letter");
                assert_eq!(block_of_letter(letter), Some((bx, by)));
                assert_eq!(block_of_letter(letter.to_ascii_lowercase()), Some((bx, by)));
            }
        }
        for bad in [
            "", "C", "C0", "C257", "C01", "C+1", "Q1", "C1x", "1C", "C-1",
        ] {
            assert_eq!(parse_pocket_address(bad), None, "{bad:?}");
        }
        for bad in ["", "C", "C-", "C-N", "C-NWX", "C-WN", "C_NW", "Q-NW", "CNW"] {
            assert_eq!(parse_field_name(bad), None, "{bad:?}");
        }
        assert_eq!(block_of_letter('Q'), None);
        assert_eq!(block_of_letter('1'), None);
    }

    /// **Doc 32 cut S.1 (ruling 3): the scheme scales with the site.** At 256 it is the shipped
    /// scheme exactly; at 512 the blocks are 128 cells, 32 × 32 pockets numbered to `1024`
    /// (`A1` at the north-west, `P1024` at the south-east, `C1024` round-trips), 16 × 16 fields
    /// (indexes to 255) named by quadrant of quadrant (`K-NW-NW`), and every pocket's address
    /// and every field's name parse back — in either case — to where they came from.
    #[test]
    fn s1_addresses_round_trip_at_both_sides() {
        assert_eq!(Scheme::of_side(256), Some(Scheme::SHIPPED));
        assert_eq!(Scheme::of_side(512), Some(Scheme::WIDEST));
        for bad in [0, 128, 300, 384, 768] {
            assert_eq!(Scheme::of_side(bad), None, "{bad}");
        }
        assert_eq!(Scheme::of_site(300), Scheme::SHIPPED);
        let s = Scheme::SHIPPED;
        assert_eq!(
            (
                s.block_cells(),
                s.block_pockets(),
                s.pockets_in_block(),
                s.block_fields(),
                s.pockets(),
                s.fields(),
                s.field_count()
            ),
            (64, 16, 256, 2, 64, 8, 64)
        );
        let w = Scheme::WIDEST;
        assert_eq!(
            (
                w.block_cells(),
                w.block_pockets(),
                w.pockets_in_block(),
                w.block_fields(),
                w.pockets(),
                w.fields(),
                w.field_count()
            ),
            (128, 32, 1_024, 4, 128, 16, 256)
        );
        // The shipped scheme is the free functions' exactly.
        for py in 0..SITE_POCKETS {
            for px in 0..SITE_POCKETS {
                assert_eq!(s.pocket_address(px, py), pocket_address(px, py));
                assert_eq!(s.pocket_in_block(px, py), pocket_in_block(px, py));
            }
        }
        for fy in 0..SITE_FIELDS {
            for fx in 0..SITE_FIELDS {
                assert_eq!(s.field_name(fx, fy), field_name(fx, fy));
            }
        }
        // 512: the corners, C1024, and the pad's own pocket (cell (256, 256)).
        assert_eq!(w.pocket_address(0, 127).as_deref(), Some("A1"));
        assert_eq!(w.pocket_address(127, 0).as_deref(), Some("P1024"));
        assert_eq!(w.pocket_address(31, 96).as_deref(), Some("A1024"));
        assert_eq!(w.pocket_address(32, 127).as_deref(), Some("B1"));
        assert_eq!(w.pocket_address(128, 0), None);
        let c1024 = w
            .parse_pocket_address("C1024")
            .expect("C1024 on a 512 site");
        assert_eq!(c1024, (95, 96));
        assert_eq!(w.pocket_address(c1024.0, c1024.1).as_deref(), Some("C1024"));
        assert_eq!(
            s.parse_pocket_address("C1024"),
            None,
            "C1024 is off a 256 site"
        );
        assert_eq!(s.parse_pocket_address("C257"), None);
        assert_eq!(w.parse_pocket_address("C1025"), None);
        assert_eq!(w.parse_pocket_address("C01"), None);
        let pad = w.pocket_of_cell((256, 256)).expect("on the site");
        assert_eq!(pad, (64, 64));
        assert_eq!(w.pocket_address(pad.0, pad.1).as_deref(), Some("G993"));
        assert_eq!(w.block_letter(64, 63), Some('K'), "the ramp's pocket");
        assert_eq!(w.block_letter(63, 63), Some('J'));
        // The fields: quadrant of quadrant; 255 the north-east field.
        assert_eq!(w.field_name(0, 15).as_deref(), Some("A-NW-NW"));
        assert_eq!(w.field_name(3, 12).as_deref(), Some("A-SE-SE"));
        assert_eq!(w.field_name(15, 0).as_deref(), Some("P-SE-SE"));
        assert_eq!(w.field_of_cell((256, 252)), Some((8, 7)));
        assert_eq!(w.field_name(8, 7).as_deref(), Some("K-NW-NW"));
        assert_eq!(w.field_index(15, 15), Some(255));
        assert_eq!(w.field_of_index(255), Some((15, 15)));
        assert_eq!(w.field_name(16, 0), None);
        assert_eq!(
            w.parse_field_name("C-NW"),
            None,
            "a 512 field names two levels"
        );
        assert_eq!(s.parse_field_name("C-NW-SE"), None, "a 256 field names one");
        for bad in ["C-NW-", "C-NW-SEX", "C--NW", "C-NWSE", "Q-NW-NW"] {
            assert_eq!(w.parse_field_name(bad), None, "{bad:?}");
        }
        // Every pocket and every field round-trips at 512, through the block index too.
        let mut seen = std::collections::BTreeSet::new();
        for py in 0..w.pockets() {
            for px in 0..w.pockets() {
                let text = w.pocket_address(px, py).expect("an address");
                assert!(seen.insert(text.clone()), "unique: {text}");
                assert_eq!(w.parse_pocket_address(&text), Some((px, py)), "{text}");
                assert_eq!(
                    w.parse_pocket_address(&text.to_ascii_lowercase()),
                    Some((px, py))
                );
                let (letter, index) = w.pocket_in_block(px, py).expect("a block index");
                assert_eq!(w.pocket_of_block_index(letter, index), Some((px, py)));
                let number = w.pocket_number(px, py).expect("a number");
                assert!((1..=1_024).contains(&number));
            }
        }
        assert_eq!(seen.len(), 16_384);
        for fy in 0..w.fields() {
            for fx in 0..w.fields() {
                let text = w.field_name(fx, fy).expect("a name");
                assert_eq!(w.parse_field_name(&text), Some((fx, fy)), "{text}");
                assert_eq!(
                    w.parse_field_name(&text.to_ascii_lowercase()),
                    Some((fx, fy))
                );
                let index = w.field_index(fx, fy).expect("an index");
                assert_eq!(w.field_of_index(index), Some((fx, fy)));
                let block = w.block_of_field((fx, fy)).expect("a block");
                assert_eq!(block, (fx / 4, fy / 4));
            }
        }
        // A block's fields and pockets at each side.
        assert_eq!(s.fields_of_block('K').map(|f| f.len()), Some(4));
        assert_eq!(w.fields_of_block('K').map(|f| f.len()), Some(16));
        assert_eq!(w.pockets_of_block('G').map(|p| p.len()), Some(1_024));
        // The reach clips to the wider site.
        assert_eq!(w.fields_in_reach((511, 511), 48).len(), 4);
        assert_eq!(w.fields_in_reach((300, 300), 48).len(), 9);
        assert_eq!(s.fields_in_reach((300, 300), 48).len(), 0, "off a 256 site");
    }

    /// Doc 20 cut SM.6: `pocket_number` is `pocket_address`'s number part over the whole site.
    #[test]
    fn sm6_pocket_number_is_the_address_number() {
        for py in 0..SITE_POCKETS {
            for px in 0..SITE_POCKETS {
                let address = pocket_address(px, py).unwrap_or_default();
                let number = pocket_number(px, py).unwrap_or_default();
                assert_eq!(
                    address[1..].parse::<u16>().ok(),
                    Some(number),
                    "({px}, {py})"
                );
                assert!((1..=256).contains(&number));
            }
        }
        assert_eq!(pocket_number(64, 0), None);
        assert_eq!(pocket_number(-1, 0), None);
    }
}
