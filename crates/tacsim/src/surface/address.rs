//! **A site's names and the instruments' reach** (doc 20 cut SM.4a; §3.1, §5, §11's
//! Instruments row; the SM.4a brief's 3.1 and decisions D6 and D19).
//!
//! Pure integer arithmetic over the site-local grid, origin at the **south-west**, `x` east and
//! `y` north — the way `crate::port::def::SiteDef::shifts`, `crate::surface::excursion::ramp_pose`
//! and the atlas composite all read it (the composite's cell `(x, y)` is the site cell
//! `(x_cm / 1000, y_cm / 1000)` of a `Direct` pose on the site's passive domain:
//! `mush_atlas::raster::Grid::cell_of` over the frame's bbox from `(0, 0)`, pinned below).
//!
//! - a **cell** is 10 m, 256 a side; a **pocket** 4 × 4 cells, 64 a side; a **field** 8 × 8
//!   pockets (32 cells), 8 a side; a **block** 2 × 2 fields (64 cells), 4 a side;
//! - a block's **letter** reads row by row from the north-west (`BLOCK_LETTERS`: `A`–`D` the
//!   northern row … `M`–`P` the southern); a pocket's **address** is its block letter and its
//!   number `1..=256` row-major from the block's north-west (`C117`); a field's **name** is its
//!   block letter and its quadrant (`C-NW`);
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
use crate::surface::ground::{SITE_BLOCKS, SITE_CELLS, SITE_FIELDS, SITE_POCKETS};

/// Cells along a pocket's side (§3.1: 40 m pockets of 10 m cells).
pub const CELLS_PER_POCKET: i64 = 4;
/// Cells along a field's side (§3.1: 320 m fields).
pub const CELLS_PER_FIELD: i64 = 32;
/// Cells along a block's side (§3.1: 640 m blocks).
pub const CELLS_PER_BLOCK: i64 = 64;
/// Pockets along a block's side.
pub const POCKETS_PER_BLOCK: i64 = 16;
/// Pockets in one block: an address's number runs `1..=256`.
pub const POCKETS_IN_BLOCK: i64 = POCKETS_PER_BLOCK * POCKETS_PER_BLOCK;

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

/// Whether a site cell lies inside the address scheme's blocks: [`SITE_CELLS`] a side until
/// doc 32 cut S.1 scales the scheme with the site.
fn on_scheme(cell: (i64, i64)) -> bool {
    cell_on_site(cell, SITE_CELLS)
}

/// The pocket a site cell lies in, `None` off the site.
#[must_use]
pub fn pocket_of_cell(cell: (i64, i64)) -> Option<(i64, i64)> {
    on_scheme(cell).then(|| {
        (
            cell.0.div_euclid(CELLS_PER_POCKET),
            cell.1.div_euclid(CELLS_PER_POCKET),
        )
    })
}

/// The field a site cell lies in, `None` off the site.
#[must_use]
pub fn field_of_cell(cell: (i64, i64)) -> Option<(i64, i64)> {
    on_scheme(cell).then(|| {
        (
            cell.0.div_euclid(CELLS_PER_FIELD),
            cell.1.div_euclid(CELLS_PER_FIELD),
        )
    })
}

/// The field a pocket lies in, `None` off the site.
#[must_use]
pub fn field_of_pocket(pocket: (i64, i64)) -> Option<(i64, i64)> {
    pocket_on_site(pocket).then(|| {
        let side = crate::surface::geology::POCKETS_PER_FIELD_SIDE;
        (pocket.0.div_euclid(side), pocket.1.div_euclid(side))
    })
}

/// Whether a pocket lies on the site.
fn pocket_on_site(pocket: (i64, i64)) -> bool {
    (0..SITE_POCKETS).contains(&pocket.0) && (0..SITE_POCKETS).contains(&pocket.1)
}

/// Whether a field lies on the site.
fn field_on_site(field: (i64, i64)) -> bool {
    (0..SITE_FIELDS).contains(&field.0) && (0..SITE_FIELDS).contains(&field.1)
}

/// **A block's letter by its south-west-based index** `(bx, by)`, `0..4` each: the letter at
/// `BLOCK_LETTERS[(3 − by) · 4 + bx]` (read from the north-west).
#[must_use]
pub fn letter_of_block(bx: i64, by: i64) -> Option<char> {
    if !(0..SITE_BLOCKS).contains(&bx) || !(0..SITE_BLOCKS).contains(&by) {
        return None;
    }
    let index = (SITE_BLOCKS - 1 - by)
        .checked_mul(SITE_BLOCKS)?
        .checked_add(bx)?;
    BLOCK_LETTERS.get(usize::try_from(index).ok()?).copied()
}

/// **The block letter of a pocket** (§3.1): `BLOCK_LETTERS[(3 − py/16)·4 + px/16]`.
#[must_use]
pub fn block_letter(px: i64, py: i64) -> Option<char> {
    if !pocket_on_site((px, py)) {
        return None;
    }
    letter_of_block(
        px.div_euclid(POCKETS_PER_BLOCK),
        py.div_euclid(POCKETS_PER_BLOCK),
    )
}

/// **A pocket's address** (§3.1, §10's `C117`): its block letter and `n = (15 − py%16)·16 +
/// px%16 + 1`, row-major from the block's north-west, `1..=256`.
#[must_use]
pub fn pocket_address(px: i64, py: i64) -> Option<String> {
    let letter = block_letter(px, py)?;
    let n = (POCKETS_PER_BLOCK - 1 - py.rem_euclid(POCKETS_PER_BLOCK))
        .checked_mul(POCKETS_PER_BLOCK)?
        .checked_add(px.rem_euclid(POCKETS_PER_BLOCK))?
        .checked_add(1)?;
    Some(format!("{letter}{n}"))
}

/// **A pocket's index in its block's cell string** (`n − 1`, `0..256`), with its block's
/// letter: the chart's `PocketBlock.cells` byte for that pocket.
#[must_use]
pub fn pocket_in_block(px: i64, py: i64) -> Option<(char, usize)> {
    let letter = block_letter(px, py)?;
    let n = (POCKETS_PER_BLOCK - 1 - py.rem_euclid(POCKETS_PER_BLOCK))
        .checked_mul(POCKETS_PER_BLOCK)?
        .checked_add(px.rem_euclid(POCKETS_PER_BLOCK))?;
    Some((letter, usize::try_from(n).ok()?))
}

/// **A pocket's number in its block** (doc 20 cut SM.6: the pocket cursor's key): the number
/// part of [`pocket_address`], `1..=256` row-major from the block's north-west.
#[must_use]
pub fn pocket_number(px: i64, py: i64) -> Option<u16> {
    let (_, index) = pocket_in_block(px, py)?;
    u16::try_from(index.checked_add(1)?).ok()
}

/// The inverse of [`pocket_in_block`]: the site pocket of `letter`'s `index`th cell.
#[must_use]
pub fn pocket_of_block_index(letter: char, index: usize) -> Option<(i64, i64)> {
    let at = BLOCK_LETTERS.iter().position(|l| *l == letter)?;
    let at = i64::try_from(at).ok()?;
    let index = i64::try_from(index).ok()?;
    if !(0..POCKETS_IN_BLOCK).contains(&index) {
        return None;
    }
    let bx = at % SITE_BLOCKS;
    let by = SITE_BLOCKS - 1 - at / SITE_BLOCKS;
    let px = bx * POCKETS_PER_BLOCK + index % POCKETS_PER_BLOCK;
    let py = by * POCKETS_PER_BLOCK + (POCKETS_PER_BLOCK - 1 - index / POCKETS_PER_BLOCK);
    Some((px, py))
}

/// **A field's name** (§10's `C-NW`): its block letter and its quadrant in the block — `N` or
/// `S` by `fy % 2`, `W` or `E` by `fx % 2`.
#[must_use]
pub fn field_name(fx: i64, fy: i64) -> Option<String> {
    if !field_on_site((fx, fy)) {
        return None;
    }
    let letter = letter_of_block(fx.div_euclid(2), fy.div_euclid(2))?;
    let ns = if fy.rem_euclid(2) == 1 { 'N' } else { 'S' };
    let we = if fx.rem_euclid(2) == 1 { 'E' } else { 'W' };
    Some(format!("{letter}-{ns}{we}"))
}

/// **The block of a letter** (doc 20 cut SM.4b): the inverse of [`letter_of_block`], `(bx, by)`
/// from the south-west, case-insensitive; `None` for anything but `A`–`P`.
#[must_use]
pub fn block_of_letter(letter: char) -> Option<(i64, i64)> {
    let upper = letter.to_ascii_uppercase();
    let at = i64::try_from(BLOCK_LETTERS.iter().position(|l| *l == upper)?).ok()?;
    Some((at % SITE_BLOCKS, SITE_BLOCKS - 1 - at / SITE_BLOCKS))
}

/// **A pocket address's pocket** (doc 20 cut SM.4b): the inverse of [`pocket_address`] —
/// `C117`, case-insensitive, strict: a block letter `A`–`P` and a number `1..=256` with no sign
/// and no leading zero. `@geology`'s site form and `+tv/drive` read it.
#[must_use]
pub fn parse_pocket_address(text: &str) -> Option<(i64, i64)> {
    let mut chars = text.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let number = chars.as_str();
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) || number.starts_with('0') {
        return None;
    }
    let n: usize = number.parse().ok()?;
    if n == 0 || i64::try_from(n).ok()? > POCKETS_IN_BLOCK {
        return None;
    }
    pocket_of_block_index(letter, n.checked_sub(1)?)
}

/// **A field name's field** (doc 20 cut SM.4b): the inverse of [`field_name`] — `C-NW`,
/// case-insensitive: a block letter, a hyphen, `N` or `S`, then `W` or `E`.
#[must_use]
pub fn parse_field_name(text: &str) -> Option<(i64, i64)> {
    let upper = text.to_ascii_uppercase();
    let mut chars = upper.chars();
    let (Some(letter), Some('-'), Some(ns), Some(we), None) = (
        chars.next(),
        chars.next(),
        chars.next(),
        chars.next(),
        chars.next(),
    ) else {
        return None;
    };
    let (bx, by) = block_of_letter(letter)?;
    let north = match ns {
        'N' => 1,
        'S' => 0,
        _ => return None,
    };
    let east = match we {
        'E' => 1,
        'W' => 0,
        _ => return None,
    };
    Some((
        bx.checked_mul(2)?.checked_add(east)?,
        by.checked_mul(2)?.checked_add(north)?,
    ))
}

/// **A field's index on the site** (`fy · 8 + fx`, `0..64`): the chart's field-row key.
#[must_use]
pub fn field_index(fx: i64, fy: i64) -> Option<u8> {
    if !field_on_site((fx, fy)) {
        return None;
    }
    u8::try_from(fy.checked_mul(SITE_FIELDS)?.checked_add(fx)?).ok()
}

/// The inverse of [`field_index`].
#[must_use]
pub fn field_of_index(index: u8) -> Option<(i64, i64)> {
    let index = i64::from(index);
    let field = (index % SITE_FIELDS, index / SITE_FIELDS);
    field_on_site(field).then_some(field)
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

/// **The fields in reach** of `stand` at `radius` cells, on the site, north-west first (rows
/// from the north, each west to east).
#[must_use]
pub fn fields_in_reach(stand: (i64, i64), radius: i64) -> Vec<(i64, i64)> {
    in_reach(stand, radius, CELLS_PER_FIELD, SITE_FIELDS)
}

/// **The pockets in reach** of `stand` at `radius` cells, on the site, north-west first.
#[must_use]
pub fn pockets_in_reach(stand: (i64, i64), radius: i64) -> Vec<(i64, i64)> {
    in_reach(stand, radius, CELLS_PER_POCKET, SITE_POCKETS)
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
