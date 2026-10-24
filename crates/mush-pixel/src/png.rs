//! A minimal PNG encoder: palette (colour type 3) when the frame has at most
//! 256 colours, truecolor (colour type 2) otherwise. Filter type 0 on every
//! row, zlib via `flate2` at a fast level — the frames are flat-shaded and
//! compress well whatever the level, and this runs twenty times a second.

use std::collections::HashMap;
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

/// The filtered, palette-indexed rows of `pixels` (`w` a row), filling
/// `palette` as colours appear — or `None` the moment a 257th colour does.
fn index_rows(pixels: &[[u8; 3]], w: usize, palette: &mut Vec<[u8; 3]>) -> Option<Vec<u8>> {
    let mut index: HashMap<[u8; 3], u8> = HashMap::new();
    let mut raw: Vec<u8> = Vec::with_capacity(pixels.len() / w.max(1) + pixels.len());
    let mut last: Option<([u8; 3], u8)> = None;
    for row in pixels.chunks_exact(w) {
        raw.push(0); // filter: none
        for &px in row {
            let i = match last {
                Some((colour, i)) if colour == px => i,
                _ => {
                    let i = match index.entry(px) {
                        std::collections::hash_map::Entry::Occupied(e) => *e.get(),
                        std::collections::hash_map::Entry::Vacant(e) => {
                            if palette.len() == 256 {
                                return None;
                            }
                            let i = u8::try_from(palette.len()).unwrap_or(255);
                            palette.push(px);
                            *e.insert(i)
                        }
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
}
