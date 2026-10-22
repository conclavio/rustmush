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

/// The capability probe: a 1x1 RGB query image under [`PROBE_IMAGE_ID`]
/// then `CSI c`.
#[must_use]
pub fn probe_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b_G");
    out.extend_from_slice(format!("i={PROBE_IMAGE_ID},s=1,v=1,a=q,t=d,f=24;").as_bytes());
    out.extend_from_slice(base64(&[0, 0, 0]).as_bytes());
    out.extend_from_slice(b"\x1b\\");
    out.extend_from_slice(b"\x1b[c");
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
    fn the_probe_is_a_query_then_da1_and_its_ok_is_recognised() {
        let s = String::from_utf8_lossy(&probe_bytes()).into_owned();
        assert_eq!(s, "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c");
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
}
