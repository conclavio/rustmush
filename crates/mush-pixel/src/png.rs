//! A minimal PNG encoder: palette (colour type 3) when the frame has at most
//! 256 colours, truecolor (colour type 2) otherwise. Filter type 0 on every
//! row, zlib via `flate2` at a fast level — the frames are flat-shaded and
//! compress well whatever the level, and this runs twenty times a second.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::Frame;

/// The zlib level: 3 is within a few percent of 6 on flat-shaded frames at a
/// third of the encode time.
const ZLIB_LEVEL: u32 = 3;

/// Encode `frame` as a PNG.
#[must_use]
pub fn encode(frame: &Frame) -> Vec<u8> {
    let (w, h) = (frame.width(), frame.height());
    let rgb = frame.rgb();
    // Palette when it fits: a quarter of the bytes before compression. The
    // pixels are long runs of one colour, so the last lookup is remembered
    // and the map is consulted only when the colour changes.
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let indexed_raw = index_rows(rgb.as_chunks::<3>().0, w, &mut palette);
    let indexed = indexed_raw.is_some();
    let raw: Vec<u8> = indexed_raw.unwrap_or_else(|| {
        let mut raw = Vec::with_capacity(h * (1 + w * 3));
        for row in rgb.chunks_exact(w * 3) {
            raw.push(0); // filter: none
            raw.extend_from_slice(row);
        }
        raw
    });
    let mut z = ZlibEncoder::new(Vec::new(), Compression::new(ZLIB_LEVEL));
    // Writing into a Vec cannot fail.
    let _ = z.write_all(&raw);
    let idat = z.finish().unwrap_or_default();

    let mut out = Vec::with_capacity(idat.len() + 64 + palette.len() * 3);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&u32::try_from(w).unwrap_or(u32::MAX).to_be_bytes());
    ihdr.extend_from_slice(&u32::try_from(h).unwrap_or(u32::MAX).to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(if indexed { 3 } else { 2 }); // colour type
    ihdr.extend_from_slice(&[0, 0, 0]); // compression, filter, interlace
    chunk(&mut out, b"IHDR", &ihdr);
    if indexed {
        let plte: Vec<u8> = palette.iter().flatten().copied().collect();
        chunk(&mut out, b"PLTE", &plte);
    }
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Encode `width` by `height` packed RGBA bytes (`width * height * 4`) as a
/// PNG: indexed with a `tRNS` alpha table (colour type 3) when at most 256
/// distinct `(r, g, b, a)` tuples appear, truecolor with alpha (colour type 6)
/// otherwise.
#[must_use]
pub fn encode_rgba(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let expected = width.saturating_mul(height).saturating_mul(4);
    let rgba = rgba.get(..expected).unwrap_or(&[]);
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let indexed_raw = index_rgba_rows(rgba.as_chunks::<4>().0, width, &mut palette);
    let indexed = indexed_raw.is_some();
    let raw: Vec<u8> = indexed_raw.unwrap_or_else(|| {
        let row_bytes = width.saturating_mul(4);
        let mut raw = Vec::with_capacity(height.saturating_mul(1 + row_bytes));
        if row_bytes > 0 {
            for row in rgba.chunks_exact(row_bytes) {
                raw.push(0); // filter: none
                raw.extend_from_slice(row);
            }
        }
        raw
    });
    let mut z = ZlibEncoder::new(Vec::new(), Compression::new(ZLIB_LEVEL));
    let _ = z.write_all(&raw);
    let idat = z.finish().unwrap_or_default();

    let mut out = Vec::with_capacity(idat.len() + 80 + palette.len() * 4);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&u32::try_from(width).unwrap_or(u32::MAX).to_be_bytes());
    ihdr.extend_from_slice(&u32::try_from(height).unwrap_or(u32::MAX).to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(if indexed { 3 } else { 6 }); // colour type: 3 = indexed, 6 = RGBA
    ihdr.extend_from_slice(&[0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    if indexed {
        let mut plte = Vec::with_capacity(palette.len() * 3);
        let mut trns = Vec::with_capacity(palette.len());
        let mut any_transparent = false;
        for &[r, g, b, a] in &palette {
            plte.extend_from_slice(&[r, g, b]);
            trns.push(a);
            if a != u8::MAX {
                any_transparent = true;
            }
        }
        chunk(&mut out, b"PLTE", &plte);
        if any_transparent {
            chunk(&mut out, b"tRNS", &trns);
        }
    }
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// **Decode a PNG to RGBA** (doc 22 cut U.6: the reducer reads the bake's
/// renders, which Unity writes as 8-bit RGBA): `(width, height, rgba)`, four
/// bytes a pixel. Reads 8-bit greyscale, RGB, palette (with `tRNS`), grey
/// with alpha and RGBA, non-interlaced, all five row filters; anything else
/// is refused with a reason, never a panic.
///
/// # Errors
///
/// What is not a PNG this reads, or is cut short.
pub fn decode_rgba(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    use std::io::Read;
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n".as_slice()) {
        return Err("not a PNG".into());
    }
    let be32 = |b: &[u8], i: usize| -> usize {
        let byte = |k: usize| u32::from(b.get(i + k).copied().unwrap_or(0));
        ((byte(0) << 24) | (byte(1) << 16) | (byte(2) << 8) | byte(3)) as usize
    };
    let (mut w, mut h, mut colour, mut ended) = (0usize, 0usize, 0u8, false);
    let (mut plte, mut trns, mut idat) = (Vec::new(), Vec::new(), Vec::new());
    let mut at = 8usize;
    while let Some(head) = bytes.get(at..at + 8) {
        let len = be32(head, 0);
        let kind = head.get(4..8).unwrap_or(&[]);
        // The chunk's data and its CRC's four bytes must all be there.
        let data = bytes
            .get(at + 8..(at + 12).saturating_add(len))
            .and_then(|d| d.get(..len))
            .ok_or("a chunk runs past the end")?;
        match kind {
            b"IHDR" => {
                w = be32(data, 0);
                h = be32(data, 4);
                colour = data.get(9).copied().unwrap_or(0);
                if data.get(8) != Some(&8) {
                    return Err("only 8-bit PNGs are read".into());
                }
                if data.get(12) != Some(&0) {
                    return Err("interlaced PNGs are not read".into());
                }
            }
            b"PLTE" => plte = data.to_vec(),
            b"tRNS" => trns = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => {
                ended = true;
                break;
            }
            _ => {}
        }
        at = at.saturating_add(12).saturating_add(len);
    }
    if !ended {
        return Err("no IEND: the PNG is cut short".into());
    }
    let channels = match colour {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        c => return Err(format!("colour type {c} is not read")),
    };
    if w == 0 || h == 0 || w > 16_384 || h > 16_384 {
        return Err(format!("a {w}x{h} image"));
    }
    let stride = w * channels;
    let mut raw = Vec::with_capacity(h * (stride + 1));
    flate2::read::ZlibDecoder::new(idat.as_slice())
        .read_to_end(&mut raw)
        .map_err(|e| format!("the image data does not inflate: {e}"))?;
    if raw.len() < h * (stride + 1) {
        return Err("the image data is short".into());
    }
    let mut prev = vec![0u8; stride];
    let mut row = vec![0u8; stride];
    let mut rgba = Vec::with_capacity(w * h * 4);
    for line in raw.chunks_exact(stride + 1).take(h) {
        let (filter, src) = line.split_first().ok_or("an empty row")?;
        for i in 0..stride {
            let x = src.get(i).copied().unwrap_or(0);
            let left = |r: &[u8]| {
                if i >= channels {
                    r.get(i - channels).copied().unwrap_or(0)
                } else {
                    0
                }
            };
            let (a, b, c) = (left(&row), prev.get(i).copied().unwrap_or(0), left(&prev));
            let v = match filter {
                0 => x,
                1 => x.wrapping_add(a),
                2 => x.wrapping_add(b),
                3 => x.wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
                4 => x.wrapping_add(paeth(a, b, c)),
                f => return Err(format!("row filter {f}")),
            };
            if let Some(slot) = row.get_mut(i) {
                *slot = v;
            }
        }
        for px in row.chunks_exact(channels) {
            let get = |i: usize| px.get(i).copied().unwrap_or(0);
            let out = match colour {
                0 => [get(0), get(0), get(0), 255],
                2 => [get(0), get(1), get(2), 255],
                3 => {
                    let i = usize::from(get(0));
                    let rgb = plte
                        .get(i * 3..i * 3 + 3)
                        .ok_or("a pixel outside the palette")?;
                    let at = |k: usize| rgb.get(k).copied().unwrap_or(0);
                    [at(0), at(1), at(2), trns.get(i).copied().unwrap_or(255)]
                }
                4 => [get(0), get(0), get(0), get(1)],
                _ => [get(0), get(1), get(2), get(3)],
            };
            rgba.extend_from_slice(&out);
        }
        std::mem::swap(&mut prev, &mut row);
    }
    Ok((w, h, rgba))
}

/// The Paeth predictor (PNG filter 4).
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// The filtered, palette-indexed rows of RGBA `pixels` (`w` a row), filling
/// `palette` as `(r, g, b, a)` tuples appear — or `None` the moment a 257th
/// tuple does.
fn index_rgba_rows(pixels: &[[u8; 4]], w: usize, palette: &mut Vec<[u8; 4]>) -> Option<Vec<u8>> {
    if w == 0 {
        return Some(Vec::new());
    }
    const TABLE_LEN: usize = 1024;
    const TABLE_MASK: usize = TABLE_LEN - 1;
    let mut table = [0u16; TABLE_LEN];
    let mut raw: Vec<u8> = Vec::with_capacity(pixels.len() / w + pixels.len());
    let mut last: Option<([u8; 4], u8)> = None;
    for row in pixels.chunks_exact(w) {
        raw.push(0); // filter: none
        for &px in row {
            let i = match last {
                Some((colour, i)) if colour == px => i,
                _ => {
                    let key = u32::from_le_bytes(px);
                    let mut slot = ((key.wrapping_mul(0x9E37_79B9)) >> 22) as usize & TABLE_MASK;
                    let i = loop {
                        let entry = table.get(slot).copied().unwrap_or(0);
                        if entry == 0 {
                            if palette.len() == 256 {
                                return None;
                            }
                            let i = u8::try_from(palette.len()).unwrap_or(255);
                            palette.push(px);
                            if let Some(cell) = table.get_mut(slot) {
                                *cell = u16::from(i) + 1;
                            }
                            break i;
                        }
                        let candidate = (entry - 1) as u8;
                        if palette.get(usize::from(candidate)) == Some(&px) {
                            break candidate;
                        }
                        slot = (slot + 1) & TABLE_MASK;
                    };
                    last = Some((px, i));
                    i
                }
            };
            raw.push(i);
        }
    }
    Some(raw)
}

/// The filtered, palette-indexed rows of `pixels` (`w` a row), filling
/// `palette` as colours appear — or `None` the moment a 257th colour does.
///
/// Uses a stack-allocated 1,024-slot open-addressed table (`0` empty, `i + 1`
/// the palette index) instead of a heap `HashMap` + SipHash per colour change
/// (doc 21a `W-01`). At most 256 slots are ever occupied, so the load factor
/// never exceeds 0.25.
fn index_rows(pixels: &[[u8; 3]], w: usize, palette: &mut Vec<[u8; 3]>) -> Option<Vec<u8>> {
    const TABLE_LEN: usize = 1024;
    const TABLE_MASK: usize = TABLE_LEN - 1;
    let mut table = [0u16; TABLE_LEN];
    let mut raw: Vec<u8> = Vec::with_capacity(pixels.len() / w.max(1) + pixels.len());
    let mut last: Option<([u8; 3], u8)> = None;
    for row in pixels.chunks_exact(w) {
        raw.push(0); // filter: none
        for &px in row {
            let i = match last {
                Some((colour, i)) if colour == px => i,
                _ => {
                    let [r, g, b] = px;
                    let key = u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16);
                    let mut slot = ((key.wrapping_mul(0x9E37_79B9)) >> 22) as usize & TABLE_MASK;
                    let i = loop {
                        let entry = table.get(slot).copied().unwrap_or(0);
                        if entry == 0 {
                            if palette.len() == 256 {
                                return None;
                            }
                            let i = u8::try_from(palette.len()).unwrap_or(255);
                            palette.push(px);
                            if let Some(cell) = table.get_mut(slot) {
                                *cell = u16::from(i) + 1;
                            }
                            break i;
                        }
                        let candidate = (entry - 1) as u8;
                        if palette.get(usize::from(candidate)) == Some(&px) {
                            break candidate;
                        }
                        slot = (slot + 1) & TABLE_MASK;
                    };
                    last = Some((px, i));
                    i
                }
            };
            raw.push(i);
        }
    }
    Some(raw)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(out.get(start..).unwrap_or(&[]));
    out.extend_from_slice(&crc.to_be_bytes());
}

/// CRC-32 (IEEE, as PNG uses it), table-driven; the table is built once.
fn crc32(bytes: &[u8]) -> u32 {
    static TABLE: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
        let mut table = [0u32; 256];
        for (n, slot) in table.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        table
    });
    let table = &*TABLE;
    let mut crc: u32 = 0xffff_ffff;
    for &b in bytes {
        let entry = table
            .get(((crc ^ u32::from(b)) & 0xff) as usize)
            .copied()
            .unwrap_or(0);
        crc = entry ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn decode_reads_back_what_encode_wrote() {
        // Indexed with tRNS (few colours) and RGBA (more than 256).
        let few: Vec<u8> = (0..7 * 5u32)
            .flat_map(|i| {
                [
                    (i % 3) as u8 * 80,
                    10,
                    200,
                    if i % 4 == 0 { 0 } else { 255 },
                ]
            })
            .collect();
        assert_eq!(decode_rgba(&encode_rgba(7, 5, &few)).unwrap(), (7, 5, few));
        let many: Vec<u8> = (0..40 * 30u32)
            .flat_map(|i| [(i % 251) as u8, (i / 7 % 256) as u8, (i % 13) as u8, 255])
            .collect();
        assert_eq!(
            decode_rgba(&encode_rgba(40, 30, &many)).unwrap(),
            (40, 30, many)
        );
        assert!(decode_rgba(b"not a png").is_err());
        let png = encode_rgba(40, 30, &[9; 40 * 30 * 4]);
        for cut in [9, 20, 33, png.len() - 13, png.len() - 1] {
            assert!(decode_rgba(&png[..cut]).is_err(), "cut at {cut}");
        }
    }

    /// Every row filter reconstructs (`encode_rgba` writes only filter 0, so
    /// the rows here are filtered by hand).
    #[test]
    fn decode_undoes_every_row_filter() {
        let (w, h) = (3usize, 5usize);
        let px: Vec<u8> = (0..w * h * 4).map(|i| (i * 37 % 256) as u8).collect();
        let stride = w * 4;
        let mut raw = Vec::new();
        for y in 0..h {
            let filter = y as u8; // 0..=4
            raw.push(filter);
            for i in 0..stride {
                let x = px[y * stride + i];
                let a = if i >= 4 { px[y * stride + i - 4] } else { 0 };
                let b = if y > 0 { px[(y - 1) * stride + i] } else { 0 };
                let c = if y > 0 && i >= 4 {
                    px[(y - 1) * stride + i - 4]
                } else {
                    0
                };
                let pred = match filter {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                    _ => paeth(a, b, c),
                };
                raw.push(x.wrapping_sub(pred));
            }
        }
        let mut z = ZlibEncoder::new(Vec::new(), Compression::new(6));
        z.write_all(&raw).unwrap();
        let idat = z.finish().unwrap();
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&3u32.to_be_bytes());
        ihdr.extend_from_slice(&5u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut png, b"IHDR", &ihdr);
        chunk(&mut png, b"IDAT", &idat);
        chunk(&mut png, b"IEND", &[]);
        assert_eq!(decode_rgba(&png).unwrap(), (w, h, px));
    }

    #[test]
    fn crc_matches_the_reference_vector() {
        // The canonical check value for CRC-32/ISO-HDLC.
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        // The IEND chunk's CRC is a well-known constant.
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    fn decode_idat(png: &[u8]) -> (u8, Vec<u8>, Vec<u8>) {
        // Walk the chunks: return (colour type, palette, inflated image data).
        let mut i = 8;
        let mut colour = 0;
        let mut plte = Vec::new();
        let mut idat = Vec::new();
        while i + 8 <= png.len() {
            let len = u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]) as usize;
            let kind = &png[i + 4..i + 8];
            let data = &png[i + 8..i + 8 + len];
            match kind {
                b"IHDR" => colour = data[9],
                b"PLTE" => plte = data.to_vec(),
                b"IDAT" => idat.extend_from_slice(data),
                _ => {}
            }
            i += 12 + len;
        }
        let mut raw = Vec::new();
        let inflated = flate2::read::ZlibDecoder::new(&idat[..]).read_to_end(&mut raw);
        assert!(inflated.is_ok(), "zlib: {inflated:?}");
        (colour, plte, raw)
    }

    #[test]
    fn a_flat_frame_is_a_palette_png_that_round_trips() {
        let mut f = Frame::new(5, 2);
        f.clear([10, 20, 30]);
        f.put(2, 1, [200, 100, 0]);
        let png = encode(&f);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let (colour, plte, raw) = decode_idat(&png);
        assert_eq!(colour, 3, "palette");
        assert_eq!(plte, vec![10, 20, 30, 200, 100, 0]);
        // Two rows, each a filter byte then five indices.
        assert_eq!(raw, vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0]);
    }

    #[test]
    fn a_busy_frame_falls_back_to_truecolor() {
        let mut f = Frame::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                f.put(x, y, [x as u8 * 8, y as u8 * 8, 7]);
            }
        }
        let (colour, plte, raw) = decode_idat(&encode(&f));
        assert_eq!(colour, 2);
        assert!(plte.is_empty());
        assert_eq!(raw.len(), 32 * (1 + 32 * 3));
    }

    #[test]
    fn encode_rgba_uses_palette_with_trns_and_falls_back_to_type_6() {
        // Two RGBA colours (one transparent, one opaque): colour type 3 + tRNS.
        let mut rgba = vec![0u8; 4 * 2 * 4];
        rgba[4..8].copy_from_slice(&[200, 100, 50, 255]);
        let png = encode_rgba(4, 2, &rgba);
        let (colour, plte, raw) = decode_idat(&png);
        assert_eq!(colour, 3);
        assert_eq!(plte, vec![0, 0, 0, 200, 100, 50]);
        assert_eq!(raw, vec![0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        // More than 256 RGBA colours: colour type 6 (RGBA truecolor).
        let mut busy = Vec::with_capacity(32 * 32 * 4);
        for y in 0..32u8 {
            for x in 0..32u8 {
                busy.extend_from_slice(&[x * 8, y * 8, 7, 200]);
            }
        }
        let (colour, plte, raw) = decode_idat(&encode_rgba(32, 32, &busy));
        assert_eq!(colour, 6);
        assert!(plte.is_empty());
        assert_eq!(raw.len(), 32 * (1 + 32 * 4));
    }
}
