//! The packed cell grid of a tile (doc 14 §2.2): per cell `class u8, elev_cm i16,
//! cover u8, room u16` — six bytes — run-length coded over identical neighbours and
//! carried in the document as base64.
//!
//! The encoding is what bounds a tile against the store: a 256×256 tile is 393 216
//! bytes raw, and after runs and base64 a flat lot is a few hundred bytes while a
//! pathological every-cell-different grid is 700 KiB. The rasterizer keeps every tile
//! under [`MAX_TILE_DOC_BYTES`] by coarsening its resolution (`raster`), so the worst
//! case is a bound and not a fact — `plan §7` records the measurement
//! (`tile_encoding_sizes_are_measured`).

use serde::{Deserialize, Serialize};

use crate::terrain::TerrainClass;

/// The most cells along one side of a tile.
pub const MAX_TILE_SIDE: u16 = 256;

/// The absolute cell cap of one tile — the config's `tile_max_cells` may lower it.
pub const MAX_TILE_CELLS: u32 = (MAX_TILE_SIDE as u32) * (MAX_TILE_SIDE as u32);

/// The most cells along one side of a composite (`window::composite`): a window
/// stitched from tiles may be wider than any one tile. Equal to tacsim's
/// `definitions::arena::MAX_ARENA_SIDE` (doc 30 WG.6), which pins the two together; an IC
/// site's side is capped by it too (doc 32 §18, cut S.0).
pub const MAX_COMPOSITE_SIDE: u16 = 512;

/// The absolute cell cap of one composite surface.
pub const MAX_COMPOSITE_CELLS: u32 = (MAX_COMPOSITE_SIDE as u32) * (MAX_COMPOSITE_SIDE as u32);

/// The most bytes one tile's encoded cells may take in its document.
pub const MAX_TILE_DOC_BYTES: usize = 512 * 1024;

/// One cell.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Cell {
    /// The ground class.
    pub class: TerrainClass,
    /// Elevation above the region datum, centimetres.
    pub elev_cm: i16,
    /// Cover, 0–255 (S5 gives it meaning; S1 writes 0).
    pub cover: u8,
    /// `0` for no room, else one more than the index into the tile's room table.
    pub room: u16,
}

/// Why a cell grid was refused.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CellsError {
    /// Zero or over-cap dimensions.
    #[error("a tile is 1 to {side} cells a side and at most {max} cells; {width}×{height} is not")]
    Size {
        /// Width.
        width: u16,
        /// Height.
        height: u16,
        /// The side cap in force: [`MAX_TILE_SIDE`] for a tile, [`MAX_COMPOSITE_SIDE`]
        /// for a composite.
        side: u16,
        /// The cap in force.
        max: u32,
    },
    /// The encoded text is not base64, or the runs do not add up.
    #[error("tile cells would not decode: {0}")]
    Encoding(String),
}

/// A row-major grid of cells.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cells {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Cells {
    /// A grid filled with one cell.
    ///
    /// # Errors
    ///
    /// [`CellsError::Size`].
    pub fn filled(
        width: u16,
        height: u16,
        cell: Cell,
        max_cells: u32,
    ) -> Result<Cells, CellsError> {
        let count = check_size(width, height, max_cells, MAX_TILE_SIDE, MAX_TILE_CELLS)?;
        Ok(Cells {
            width,
            height,
            cells: vec![cell; count],
        })
    }

    /// A composite surface's grid filled with one cell: up to [`MAX_COMPOSITE_SIDE`]
    /// a side, where a tile stops at [`MAX_TILE_SIDE`].
    ///
    /// # Errors
    ///
    /// [`CellsError::Size`].
    pub fn filled_composite(width: u16, height: u16, cell: Cell) -> Result<Cells, CellsError> {
        let count = check_size(
            width,
            height,
            MAX_COMPOSITE_CELLS,
            MAX_COMPOSITE_SIDE,
            MAX_COMPOSITE_CELLS,
        )?;
        Ok(Cells {
            width,
            height,
            cells: vec![cell; count],
        })
    }

    /// Width in cells.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Height in cells.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Cell count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether the grid is empty (never, once built).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(usize::from(y) * usize::from(self.width) + usize::from(x))
    }

    /// The cell at `(x, y)`.
    #[must_use]
    pub fn get(&self, x: u16, y: u16) -> Option<Cell> {
        self.index(x, y).and_then(|i| self.cells.get(i).copied())
    }

    /// Replace the cell at `(x, y)`; `false` when out of range.
    pub fn set(&mut self, x: u16, y: u16, cell: Cell) -> bool {
        match self.index(x, y) {
            Some(i) => match self.cells.get_mut(i) {
                Some(slot) => {
                    *slot = cell;
                    true
                }
                None => false,
            },
            None => false,
        }
    }

    /// Edit the cell at `(x, y)` in place; `false` when out of range.
    pub fn update(&mut self, x: u16, y: u16, f: impl FnOnce(&mut Cell)) -> bool {
        match self.index(x, y).and_then(|i| self.cells.get_mut(i)) {
            Some(slot) => {
                f(slot);
                true
            }
            None => false,
        }
    }

    /// Every cell, row-major from the south-west (cut AP: the path search's once-per-search class
    /// scan for its heuristic's floor,
    /// [`LocomotionCosts::cheapest_on`](crate::physics::LocomotionCosts::cheapest_on), reads the
    /// grid without a bounds check per cell; the search's inner loop still reads through
    /// [`Cells::get`]).
    #[must_use]
    pub fn as_slice(&self) -> &[Cell] {
        &self.cells
    }

    /// Every cell, row-major, with its coordinates.
    pub fn iter(&self) -> impl Iterator<Item = (u16, u16, Cell)> + '_ {
        let width = usize::from(self.width);
        self.cells.iter().enumerate().map(move |(i, c)| {
            #[allow(clippy::cast_possible_truncation)]
            let (x, y) = ((i % width) as u16, (i / width) as u16);
            (x, y, *c)
        })
    }

    /// How many runs the encoding has.
    #[must_use]
    pub fn runs(&self) -> usize {
        let mut runs = 0_usize;
        let mut prev: Option<Cell> = None;
        let mut len: u32 = 0;
        for c in &self.cells {
            if prev == Some(*c) && len < u32::from(u16::MAX) {
                len = len.saturating_add(1);
            } else {
                runs = runs.saturating_add(1);
                prev = Some(*c);
                len = 1;
            }
        }
        runs
    }

    /// The document form: base64 over `[w u16][h u16]` and `[count u16][class][elev
    /// i16][cover][room u16]` per run, all little-endian.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut bytes: Vec<u8> = Vec::with_capacity(4 + self.runs().saturating_mul(8));
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        let mut run: Option<(Cell, u16)> = None;
        let flush = |bytes: &mut Vec<u8>, run: Option<(Cell, u16)>| {
            if let Some((c, n)) = run {
                bytes.extend_from_slice(&n.to_le_bytes());
                bytes.push(c.class.code());
                bytes.extend_from_slice(&c.elev_cm.to_le_bytes());
                bytes.push(c.cover);
                bytes.extend_from_slice(&c.room.to_le_bytes());
            }
        };
        for c in &self.cells {
            match run {
                Some((prev, n)) if prev == *c && n < u16::MAX => run = Some((prev, n + 1)),
                other => {
                    flush(&mut bytes, other);
                    run = Some((*c, 1));
                }
            }
        }
        flush(&mut bytes, run);
        base64_encode(&bytes)
    }

    /// Decode the document form, checking the dimensions against `max_cells` and the
    /// runs against the dimensions.
    ///
    /// # Errors
    ///
    /// [`CellsError`].
    pub fn decode(text: &str, max_cells: u32) -> Result<Cells, CellsError> {
        Self::decode_capped(text, max_cells, MAX_TILE_SIDE, MAX_TILE_CELLS)
    }

    /// Decode a composite surface's document form: [`Cells::decode`] with the
    /// composite's caps ([`MAX_COMPOSITE_SIDE`]) in place of a tile's.
    ///
    /// # Errors
    ///
    /// [`CellsError`].
    pub fn decode_composite(text: &str) -> Result<Cells, CellsError> {
        Self::decode_capped(
            text,
            MAX_COMPOSITE_CELLS,
            MAX_COMPOSITE_SIDE,
            MAX_COMPOSITE_CELLS,
        )
    }

    fn decode_capped(
        text: &str,
        max_cells: u32,
        side_cap: u16,
        cells_cap: u32,
    ) -> Result<Cells, CellsError> {
        let bytes = base64_decode(text).ok_or_else(|| CellsError::Encoding("not base64".into()))?;
        let mut cursor = bytes.as_slice();
        let width = take_u16(&mut cursor)?;
        let height = take_u16(&mut cursor)?;
        let count = check_size(width, height, max_cells, side_cap, cells_cap)?;
        let mut cells: Vec<Cell> = Vec::with_capacity(count);
        while !cursor.is_empty() {
            let n = usize::from(take_u16(&mut cursor)?);
            let class_code = take_u8(&mut cursor)?;
            let class = TerrainClass::from_code(class_code)
                .ok_or_else(|| CellsError::Encoding(format!("unknown class code {class_code}")))?;
            let elev_cm = take_i16(&mut cursor)?;
            let cover = take_u8(&mut cursor)?;
            let room = take_u16(&mut cursor)?;
            if n == 0 || cells.len().saturating_add(n) > count {
                return Err(CellsError::Encoding("runs exceed the grid".into()));
            }
            cells.extend(std::iter::repeat_n(
                Cell {
                    class,
                    elev_cm,
                    cover,
                    room,
                },
                n,
            ));
        }
        if cells.len() != count {
            return Err(CellsError::Encoding(format!(
                "runs cover {} of {count} cells",
                cells.len()
            )));
        }
        Ok(Cells {
            width,
            height,
            cells,
        })
    }
}

fn check_size(
    width: u16,
    height: u16,
    max_cells: u32,
    side_cap: u16,
    cells_cap: u32,
) -> Result<usize, CellsError> {
    let count = u32::from(width) * u32::from(height);
    let cap = max_cells.min(cells_cap);
    if width == 0 || height == 0 || width > side_cap || height > side_cap || count > cap {
        return Err(CellsError::Size {
            width,
            height,
            side: side_cap,
            max: cap,
        });
    }
    usize::try_from(count).map_err(|_| CellsError::Size {
        width,
        height,
        side: side_cap,
        max: cap,
    })
}

fn take_u8(cursor: &mut &[u8]) -> Result<u8, CellsError> {
    let (first, rest) = cursor
        .split_first()
        .ok_or_else(|| CellsError::Encoding("truncated".into()))?;
    *cursor = rest;
    Ok(*first)
}

fn take_u16(cursor: &mut &[u8]) -> Result<u16, CellsError> {
    let lo = take_u8(cursor)?;
    let hi = take_u8(cursor)?;
    Ok(u16::from_le_bytes([lo, hi]))
}

fn take_i16(cursor: &mut &[u8]) -> Result<i16, CellsError> {
    let lo = take_u8(cursor)?;
    let hi = take_u8(cursor)?;
    Ok(i16::from_le_bytes([lo, hi]))
}

/// The wire form of [`Cells`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellsWire {
    /// Width in cells.
    pub width: u16,
    /// Height in cells.
    pub height: u16,
    /// The base64 runs.
    pub data: String,
}

impl Serialize for Cells {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        CellsWire {
            width: self.width,
            height: self.height,
            data: self.encode(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Cells {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = CellsWire::deserialize(deserializer)?;
        let cells = Cells::decode(&wire.data, MAX_TILE_CELLS).map_err(serde::de::Error::custom)?;
        wire_agrees(&wire, cells)
    }
}

/// Deserialize a composite surface's [`Cells`] (`window::SurfaceComposite`): the wire
/// form a tile has, decoded with the composite's caps so a 512-a-side window crosses
/// the async seam; a tile's own `Deserialize` keeps the tile's.
///
/// # Errors
///
/// The deserializer's, or the decode's.
pub fn deserialize_composite<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Cells, D::Error> {
    let wire = CellsWire::deserialize(deserializer)?;
    let cells = Cells::decode_composite(&wire.data).map_err(serde::de::Error::custom)?;
    wire_agrees(&wire, cells)
}

fn wire_agrees<E: serde::de::Error>(wire: &CellsWire, cells: Cells) -> Result<Cells, E> {
    if cells.width != wire.width || cells.height != wire.height {
        return Err(E::custom(
            "tile cells: the declared size disagrees with the encoded size",
        ));
    }
    Ok(cells)
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
#[must_use]
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let sym = |i: u32| -> char { char::from(BASE64.get(i as usize).copied().unwrap_or(b'=')) };
    for chunk in bytes.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied();
        let b2 = chunk.get(2).copied();
        let n =
            (u32::from(b0) << 16) | (u32::from(b1.unwrap_or(0)) << 8) | u32::from(b2.unwrap_or(0));
        out.push(sym((n >> 18) & 63));
        out.push(sym((n >> 12) & 63));
        out.push(if b1.is_some() {
            sym((n >> 6) & 63)
        } else {
            '='
        });
        out.push(if b2.is_some() { sym(n & 63) } else { '=' });
    }
    out
}

/// Standard base64 with padding; `None` on any malformed input.
#[must_use]
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let value = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let total = bytes.len() / 4;
    for (i, chunk) in bytes.chunks(4).enumerate() {
        let c0 = chunk.first().copied()?;
        let c1 = chunk.get(1).copied()?;
        let c2 = chunk.get(2).copied()?;
        let c3 = chunk.get(3).copied()?;
        let last = i + 1 == total;
        let pad = usize::from(c3 == b'=') + usize::from(c2 == b'=');
        if pad > 0 && !last {
            return None;
        }
        if c2 == b'=' && c3 != b'=' {
            return None;
        }
        let n = (value(c0)? << 18)
            | (value(c1)? << 12)
            | (if c2 == b'=' { 0 } else { value(c2)? << 6 })
            | (if c3 == b'=' { 0 } else { value(c3)? });
        #[allow(clippy::cast_possible_truncation)]
        {
            out.push(((n >> 16) & 0xff) as u8);
            if pad < 2 {
                out.push(((n >> 8) & 0xff) as u8);
            }
            if pad < 1 {
                out.push((n & 0xff) as u8);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_every_length() {
        for len in 0..40_usize {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 % 251) as u8).collect();
            let text = base64_encode(&bytes);
            assert_eq!(
                base64_decode(&text).as_deref(),
                Some(bytes.as_slice()),
                "len {len}"
            );
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert!(base64_decode("TQ=").is_none());
        assert!(base64_decode("T@==").is_none());
        assert!(base64_decode("TQ==TQ==").is_none(), "padding mid-stream");
    }

    #[test]
    fn cells_round_trip_through_the_document_form() {
        let mut cells = Cells::filled(7, 5, Cell::default(), MAX_TILE_CELLS).expect("grid");
        assert!(cells.set(
            3,
            2,
            Cell {
                class: TerrainClass::Floor,
                elev_cm: -120,
                cover: 9,
                room: 2,
            }
        ));
        assert!(!cells.set(7, 0, Cell::default()));
        let text = cells.encode();
        let back = Cells::decode(&text, MAX_TILE_CELLS).expect("decodes");
        assert_eq!(back, cells);
        assert_eq!(back.get(3, 2).map(|c| c.elev_cm), Some(-120));
        assert_eq!(back.runs(), 3);
        let json = serde_json::to_string(&cells).expect("json");
        let via: Cells = serde_json::from_str(&json).expect("wire");
        assert_eq!(via, cells);
    }

    #[test]
    fn a_composite_grid_reaches_512_a_side_and_no_further() {
        let tacsim_arena_cap: u16 = 512;
        assert_eq!(MAX_COMPOSITE_SIDE, tacsim_arena_cap);
        let big = Cells::filled_composite(512, 512, Cell::default()).expect("512²");
        assert_eq!(big.len(), 512 * 512);
        assert!(matches!(
            Cells::filled_composite(513, 512, Cell::default()),
            Err(CellsError::Size { side: 512, .. })
        ));
        // A tile stays at 256, by either door.
        assert!(Cells::filled(512, 512, Cell::default(), MAX_COMPOSITE_CELLS).is_err());
        let text = big.encode();
        assert!(Cells::decode(&text, MAX_COMPOSITE_CELLS).is_err());
        assert_eq!(Cells::decode_composite(&text).expect("decodes"), big);
        // The composite wire form round-trips through serde.
        #[derive(serde::Deserialize)]
        struct Wrap {
            #[serde(deserialize_with = "deserialize_composite")]
            cells: Cells,
        }
        let json = format!(
            "{{\"cells\":{}}}",
            serde_json::to_string(&big).expect("json")
        );
        let via: Wrap = serde_json::from_str(&json).expect("wire");
        assert_eq!(via.cells, big);
        assert!(
            serde_json::from_str::<Cells>(&serde_json::to_string(&big).expect("json")).is_err()
        );
    }

    #[test]
    fn a_grid_over_the_cap_is_refused_and_bad_runs_do_not_decode() {
        assert!(matches!(
            Cells::filled(300, 1, Cell::default(), MAX_TILE_CELLS),
            Err(CellsError::Size { .. })
        ));
        assert!(matches!(
            Cells::filled(256, 256, Cell::default(), 1024),
            Err(CellsError::Size { max: 1024, .. })
        ));
        let cells = Cells::filled(4, 4, Cell::default(), MAX_TILE_CELLS).expect("grid");
        let text = cells.encode();
        // Same bytes, declared as 4×3: the runs overflow.
        let mut bytes = base64_decode(&text).expect("bytes");
        if let Some(b) = bytes.get_mut(2) {
            *b = 3;
        }
        assert!(Cells::decode(&base64_encode(&bytes), MAX_TILE_CELLS).is_err());
        assert!(Cells::decode("", MAX_TILE_CELLS).is_err());
    }

    /// Plan §8's open question, answered by measurement: the flat lot is tiny, an
    /// 8 m relief lattice keeps a full outdoor tile far under the document cap, and
    /// only a grid where no cell equals its neighbour reaches the pathological size —
    /// which the rasterizer's coarsening keeps out of the store.
    #[test]
    fn tile_encoding_sizes_are_measured() {
        let flat = Cells::filled(256, 256, Cell::default(), MAX_TILE_CELLS).expect("grid");
        let flat_len = flat.encode().len();
        assert!(
            flat_len < 64,
            "a flat 256×256 tile encodes in {flat_len} bytes"
        );

        // An 8-cell lattice of varying elevation (the background's relief period at
        // 1 m cells): runs of eight.
        let mut lattice = flat.clone();
        for y in 0..256_u16 {
            for x in 0..256_u16 {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let elev = ((x / 8 + y / 8) % 7) as i16 * 10;
                lattice.update(x, y, |c| c.elev_cm = elev);
            }
        }
        let lattice_len = lattice.encode().len();
        assert!(
            lattice_len < MAX_TILE_DOC_BYTES / 4,
            "a full relief lattice encodes in {lattice_len} bytes"
        );

        let mut worst = flat;
        for y in 0..256_u16 {
            for x in 0..256_u16 {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let elev = (x ^ y) as i16;
                worst.update(x, y, |c| c.elev_cm = elev);
            }
        }
        let worst_len = worst.encode().len();
        assert!(
            worst_len > MAX_TILE_DOC_BYTES,
            "the every-cell-different case ({worst_len} bytes) is what the coarsening rule exists for"
        );
        eprintln!("tile encoding: flat {flat_len} B, lattice {lattice_len} B, worst {worst_len} B");
    }
}
