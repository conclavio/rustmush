//! The kitty graphics protocol framing (the APC `ESC _ G … ESC \` family).
//!
//! What the spike uses of it:
//!
//! - **The probe**: a one-pixel query (`a=q`) the terminal answers with
//!   `ESC _ G i=31;OK ESC \` when it speaks the protocol, followed by a
//!   primary device attributes request (`CSI c`) EVERY terminal answers —
//!   so an OK arriving before the DA1 reply is support, and the DA1 reply
//!   arriving alone is the absence of it, with no wait on a silent peer.
//! - **A frame**: the PNG (`f=100`) sent inline (`t=d`) in 4096-byte base64
//!   chunks (`m=1` on every chunk but the last) under one fixed image id and
//!   placement id, displayed at once (`a=T`) scaled into `cols` by `rows`
//!   cells, cursor left where it was (`C=1`), replies suppressed (`q=2`).
//!   The whole thing sits between `CSI ?2026h` and `CSI ?2026l`
//!   (synchronized output) so the HUD text and the picture land together.
//! - **A delete**: free the image (`a=d,d=i`) when the view closes.

/// The image id the probe transmits under; the reply names it.
pub const PROBE_IMAGE_ID: u32 = 31;

/// The largest payload one APC chunk may carry, per the protocol.
const CHUNK: usize = 4096;

/// Begin synchronized output.
pub const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
/// End synchronized output.
pub const SYNC_END: &[u8] = b"\x1b[?2026l";

/// The kitty half of the capability probe: a 1x1 RGB query image under
/// [`PROBE_IMAGE_ID`]. The whole probe, with the `CSI c` that ends it, is
/// [`crate::caps::probe_bytes`].
#[must_use]
pub fn query_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b_G");
    out.extend_from_slice(format!("i={PROBE_IMAGE_ID},s=1,v=1,a=q,t=d,f=24;").as_bytes());
    out.extend_from_slice(base64(&[0, 0, 0]).as_bytes());
    out.extend_from_slice(b"\x1b\\");
    out
}

/// Whether an APC payload (the bytes between `ESC _` and `ESC \`) is the
/// probe's OK.
#[must_use]
pub fn is_probe_ok(payload: &str) -> bool {
    let Some(rest) = payload.strip_prefix('G') else {
        return false;
    };
    let (keys, status) = rest.split_once(';').unwrap_or((rest, ""));
    keys.split(',')
        .any(|kv| kv == format!("i={PROBE_IMAGE_ID}"))
        && status.starts_with("OK")
}

/// If an APC payload (`Gi=<id>…;<status>`) is a terminal error response (`q=1`)
/// for a retained image other than [`PROBE_IMAGE_ID`], returns that `image_id`
/// so the caller can re-upload an evicted or missing image.
#[must_use]
pub fn error_image_id(payload: &str) -> Option<u32> {
    let rest = payload.strip_prefix('G')?;
    let (keys, status) = rest.split_once(';')?;
    if status.is_empty() || status.starts_with("OK") {
        return None;
    }
    for kv in keys.split(',') {
        if let Some(id_str) = kv.strip_prefix("i=")
            && let Ok(id) = id_str.parse::<u32>()
            && id != PROBE_IMAGE_ID
        {
            return Some(id);
        }
    }
    None
}

/// One frame: `png` placed with its top-left at cell (`col`, `row`),
/// 1-based as `CUP` counts, scaled into `cols` by `rows` cells.
#[must_use]
pub fn frame_bytes(png: &[u8], image_id: u32, col: u16, row: u16, cols: u16, rows: u16) -> Vec<u8> {
    let b64 = base64(png);
    let mut out = Vec::with_capacity(b64.len() + 256);
    out.extend_from_slice(SYNC_BEGIN);
    out.extend_from_slice(format!("\x1b[{row};{col}H").as_bytes());
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK).collect();
    let n = chunks.len();
    for (k, chunk) in chunks.iter().enumerate() {
        out.extend_from_slice(b"\x1b_G");
        let more = u8::from(k + 1 < n);
        if k == 0 {
            out.extend_from_slice(
                format!("a=T,f=100,t=d,i={image_id},p=1,q=2,C=1,c={cols},r={rows},m={more};")
                    .as_bytes(),
            );
        } else {
            out.extend_from_slice(format!("m={more};").as_bytes());
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out.extend_from_slice(SYNC_END);
    out
}

/// Upload a PNG (`f=100`) into terminal storage under `image_id` (`a=t,t=d`)
/// without creating a placement yet, with `q=1` so storage errors are reported.
pub fn transmit_bytes(out: &mut Vec<u8>, png: &[u8], image_id: u32) {
    let b64 = base64(png);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK).collect();
    let n = chunks.len();
    for (k, chunk) in chunks.iter().enumerate() {
        out.extend_from_slice(b"\x1b_G");
        let more = u8::from(k + 1 < n);
        if k == 0 {
            out.extend_from_slice(format!("a=t,f=100,t=d,i={image_id},q=1,m={more};").as_bytes());
        } else {
            out.extend_from_slice(format!("m={more};").as_bytes());
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

/// Patch a rectangular sub-region `(dst_x, dst_y)` of an already-uploaded
/// image `image_id` in place (`a=f,r=1,X=1,f=100`).
pub fn patch_frame_bytes(out: &mut Vec<u8>, png: &[u8], image_id: u32, dst_x: u32, dst_y: u32) {
    let b64 = base64(png);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK).collect();
    let n = chunks.len();
    for (k, chunk) in chunks.iter().enumerate() {
        out.extend_from_slice(b"\x1b_G");
        let more = u8::from(k + 1 < n);
        if k == 0 {
            out.extend_from_slice(
                format!("a=f,f=100,t=d,i={image_id},r=1,x={dst_x},y={dst_y},X=1,q=1,m={more};")
                    .as_bytes(),
            );
        } else {
            out.extend_from_slice(format!("m={more};").as_bytes());
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

/// Parameters for a retained Kitty placement (`a=p`).
///
/// Re-sending the same `(image_id, placement_id)` moves and updates the
/// existing placement in place without creating a duplicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Stored terminal image ID (`i=`).
    pub image_id: u32,
    /// Placement ID (`p=`).
    pub placement_id: u32,
    /// 1-based terminal column (`CUP`).
    pub col: u16,
    /// 1-based terminal row (`CUP`).
    pub row: u16,
    /// Target width in terminal columns (`c=`). When `0`, omitted.
    pub cols: u16,
    /// Target height in terminal rows (`r=`). When `0`, omitted.
    pub rows: u16,
    /// Sub-cell X pixel offset within the anchor cell (`X=`).
    pub cell_x: u16,
    /// Sub-cell Y pixel offset within the anchor cell (`Y=`).
    pub cell_y: u16,
    /// Source rectangle X offset in pixels within the stored image (`x=`).
    pub src_x: u32,
    /// Source rectangle Y offset in pixels within the stored image (`y=`).
    pub src_y: u32,
    /// Source rectangle width in pixels (`w=`).
    pub src_w: u32,
    /// Source rectangle height in pixels (`h=`).
    pub src_h: u32,
    /// Stacking order (`z=`); negative values draw under text, positive over.
    pub z: i32,
}

/// Append a cursor move + retained placement (`a=p`) command to `out`.
///
/// Uses `C=1` so the cursor does not move after placing, `q=1` so only errors
/// (such as an evicted image ID) are reported back by the terminal, and `p=<id>`
/// when `p.placement_id > 0` so re-sending the same `(image_id, placement_id)`
/// updates the placement in place per the Kitty graphics specification.
pub fn place_bytes(out: &mut Vec<u8>, p: &Placement) {
    let pid_part = if p.placement_id > 0 {
        format!(",p={}", p.placement_id)
    } else {
        String::new()
    };
    let dim_part = if p.cols > 0 && p.rows > 0 {
        format!(",c={},r={}", p.cols, p.rows)
    } else {
        String::new()
    };
    out.extend_from_slice(
        format!(
            "\x1b[{row};{col}H\x1b_Ga=p,i={i}{pid_part},x={sx},y={sy},w={sw},h={sh}{dim_part},X={cx},Y={cy},z={z},C=1,q=1;\x1b\\",
            row = p.row,
            col = p.col,
            i = p.image_id,
            sx = p.src_x,
            sy = p.src_y,
            sw = p.src_w,
            sh = p.src_h,
            cx = p.cell_x,
            cy = p.cell_y,
            z = p.z,
        )
        .as_bytes(),
    );
}

/// Delete a placement `(image_id, placement_id)` (`a=d,d=i`) while keeping the
/// underlying stored image `image_id` in terminal memory. When `placement_id == 0`,
/// `p=` is omitted so all placements of `image_id` are removed.
pub fn delete_placement_bytes(out: &mut Vec<u8>, image_id: u32, placement_id: u32) {
    if placement_id > 0 {
        out.extend_from_slice(
            format!("\x1b_Ga=d,d=i,i={image_id},p={placement_id},q=2;\x1b\\").as_bytes(),
        );
    } else {
        out.extend_from_slice(format!("\x1b_Ga=d,d=i,i={image_id},q=2;\x1b\\").as_bytes());
    }
}

/// Delete stored `image_id` and all of its placements (`a=d,d=I`), appending to `out`.
pub fn delete_image_bytes(out: &mut Vec<u8>, image_id: u32) {
    out.extend_from_slice(format!("\x1b_Ga=d,d=I,i={image_id},q=2;\x1b\\").as_bytes());
}

/// Free `image_id` and its placements.
#[must_use]
pub fn delete_bytes(image_id: u32) -> Vec<u8> {
    format!("\x1b_Ga=d,d=I,i={image_id},q=2;\x1b\\").into_bytes()
}

/// Standard base64 with padding.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let digit = |v: u32| T.get(v as usize & 63).map_or('=', |&c| c as char);
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let b0 = group.first().map_or(0, |&b| u32::from(b));
        let b1 = group.get(1).map_or(0, |&b| u32::from(b));
        let b2 = group.get(2).map_or(0, |&b| u32::from(b));
        let v = (b0 << 16) | (b1 << 8) | b2;
        out.push(digit(v >> 18));
        out.push(digit(v >> 12));
        out.push(if group.len() > 1 { digit(v >> 6) } else { '=' });
        out.push(if group.len() > 2 { digit(v) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn the_query_is_one_pixel_and_its_ok_is_recognised() {
        let s = String::from_utf8_lossy(&query_bytes()).into_owned();
        assert_eq!(s, "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
        assert!(is_probe_ok("Gi=31;OK"));
        assert!(is_probe_ok("Gi=31,I=0;OK"));
        assert!(!is_probe_ok("Gi=31;ENOENT:not found"));
        assert!(!is_probe_ok("Gi=7;OK"));
        assert!(!is_probe_ok("Xi=31;OK"));
    }

    #[test]
    fn a_frame_is_chunked_under_one_id_inside_synchronized_output() {
        let png = vec![7u8; 9000]; // base64 = 12000 chars: three chunks
        let s = String::from_utf8_lossy(&frame_bytes(&png, 1, 1, 2, 100, 40)).into_owned();
        assert!(s.starts_with(
            "\x1b[?2026h\x1b[2;1H\x1b_Ga=T,f=100,t=d,i=1,p=1,q=2,C=1,c=100,r=40,m=1;"
        ));
        assert!(s.ends_with("\x1b\\\x1b[?2026l"));
        assert_eq!(s.matches("\x1b_G").count(), 3);
        assert_eq!(s.matches("m=1;").count(), 2);
        assert_eq!(s.matches("m=0;").count(), 1);
        // Every chunk payload is at most 4096 bytes.
        for apc in s.split("\x1b_G").skip(1) {
            let payload = apc.split(';').nth(1).and_then(|p| p.split('\x1b').next());
            assert!(payload.is_some_and(|p| p.len() <= CHUNK));
        }
        assert_eq!(
            String::from_utf8_lossy(&delete_bytes(1)),
            "\x1b_Ga=d,d=I,i=1,q=2;\x1b\\"
        );
    }

    #[test]
    fn retained_transmit_place_patch_and_error_parsing_work() {
        assert_eq!(error_image_id("Gi=31;ENOENT:not found"), None);
        assert_eq!(error_image_id("Gi=42,p=7;OK"), None);
        assert_eq!(error_image_id("Gi=42,p=7;ENOENT:image not found"), Some(42));
        assert_eq!(error_image_id("Gp=7,i=105;ENOSPC:out of quota"), Some(105));

        let mut buf = Vec::new();
        transmit_bytes(&mut buf, b"foo", 7);
        assert_eq!(
            String::from_utf8_lossy(&buf),
            "\x1b_Ga=t,f=100,t=d,i=7,q=1,m=0;Zm9v\x1b\\"
        );

        buf.clear();
        place_bytes(
            &mut buf,
            &Placement {
                image_id: 7,
                placement_id: 12,
                col: 10,
                row: 5,
                cols: 12,
                rows: 6,
                cell_x: 0,
                cell_y: 0,
                src_x: 96,
                src_y: 192,
                src_w: 96,
                src_h: 96,
                z: -10,
            },
        );
        assert_eq!(
            String::from_utf8_lossy(&buf),
            "\x1b[5;10H\x1b_Ga=p,i=7,p=12,x=96,y=192,w=96,h=96,c=12,r=6,X=0,Y=0,z=-10,C=1,q=1;\x1b\\"
        );

        buf.clear();
        patch_frame_bytes(&mut buf, b"bar", 2, 64, 128);
        assert_eq!(
            String::from_utf8_lossy(&buf),
            "\x1b_Ga=f,f=100,t=d,i=2,r=1,x=64,y=128,X=1,q=1,m=0;YmFy\x1b\\"
        );

        buf.clear();
        delete_placement_bytes(&mut buf, 7, 12);
        assert_eq!(
            String::from_utf8_lossy(&buf),
            "\x1b_Ga=d,d=i,i=7,p=12,q=2;\x1b\\"
        );
    }
}
