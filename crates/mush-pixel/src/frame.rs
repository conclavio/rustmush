//! [`Frame`] — a packed RGB framebuffer, the one thing every encoder reads.

/// One RGB pixel.
pub type Rgb = [u8; 3];

/// A `width` by `height` framebuffer of packed 8-bit RGB, row-major, top row
/// first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Frame {
    /// A black frame of the given size.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Frame {
        Frame {
            width,
            height,
            rgb: vec![0; width * height * 3],
        }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    /// The packed RGB bytes, `width * height * 3` of them.
    #[must_use]
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    /// The packed RGB bytes for a producer that writes whole rows (the
    /// raycaster's sky and floor passes): `width * height * 3` of them.
    pub(crate) fn rgb_mut(&mut self) -> &mut [u8] {
        &mut self.rgb
    }

    /// Set one pixel; out-of-range coordinates are ignored.
    pub fn put(&mut self, x: usize, y: usize, px: Rgb) {
        if x < self.width && y < self.height {
            let i = (y * self.width + x) * 3;
            if let Some(slot) = self.rgb.get_mut(i..i + 3) {
                slot.copy_from_slice(&px);
            }
        }
    }

    /// Read one pixel; black when out of range.
    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> Rgb {
        if x < self.width && y < self.height {
            let i = (y * self.width + x) * 3;
            match self.rgb.get(i..i + 3) {
                Some([r, g, b]) => [*r, *g, *b],
                _ => [0, 0, 0],
            }
        } else {
            [0, 0, 0]
        }
    }

    /// Fill the whole frame with one colour.
    pub fn clear(&mut self, px: Rgb) {
        for chunk in self.rgb.as_chunks_mut::<3>().0 {
            *chunk = px;
        }
    }

    /// A 64-bit hash of the pixels: two frames with the same hash are (for
    /// the purposes of "did anything change") the same picture, so the
    /// sender can skip a frame identical to the last one it sent.
    ///
    /// Taken eight bytes at a time (FNV-1a's offset and prime, a rotate to
    /// spread each word): the frame clock hashes every tick, sent or not, and
    /// the byte-at-a-time FNV the spike used cost about a millisecond at
    /// 640x480. The value is only ever compared within one connection, so it
    /// need not be stable across versions.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let (words, tail) = self.rgb.as_chunks::<8>();
        for word in words {
            h = (h ^ u64::from_le_bytes(*word))
                .wrapping_mul(0x0100_0000_01b3)
                .rotate_left(29);
        }
        for &b in tail {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        h ^ ((self.width as u64) << 32 | self.height as u64)
    }

    /// How many distinct colours the frame uses, counting up to `limit + 1`
    /// (so a caller asking "at most 256?" stops early on a busy frame).
    #[must_use]
    pub fn count_colours(&self, limit: usize) -> usize {
        let mut seen = std::collections::HashSet::new();
        for chunk in self.rgb.as_chunks::<3>().0 {
            seen.insert(*chunk);
            if seen.len() > limit {
                break;
            }
        }
        seen.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_and_bounds() {
        let mut f = Frame::new(4, 3);
        f.put(1, 2, [9, 8, 7]);
        assert_eq!(f.get(1, 2), [9, 8, 7]);
        assert_eq!(f.get(4, 0), [0, 0, 0]);
        f.put(99, 99, [1, 1, 1]);
        assert_eq!(f.rgb().len(), 36);
    }

    #[test]
    fn hash_changes_with_a_pixel_and_colours_count() {
        let mut f = Frame::new(8, 8);
        let a = f.hash();
        f.put(3, 3, [255, 0, 0]);
        assert_ne!(a, f.hash());
        assert_eq!(f.count_colours(256), 2);
        assert_eq!(f.count_colours(1), 2, "stops at limit + 1");
    }

    /// Every pixel of a frame moves the hash, including one in the tail a
    /// whole number of words leaves over, and the size is part of it.
    #[test]
    fn hash_sees_every_pixel_and_the_size() {
        let mut f = Frame::new(5, 3); // 45 bytes: five words and a tail of 5
        let mut seen = std::collections::HashSet::new();
        assert!(seen.insert(f.hash()));
        for y in 0..3 {
            for x in 0..5 {
                let mut g = f.clone();
                g.put(x, y, [1, 0, 0]);
                assert!(seen.insert(g.hash()), "pixel {x},{y}");
            }
        }
        f.put(4, 2, [0, 0, 1]);
        assert!(seen.insert(f.hash()), "the last byte");
        assert_ne!(Frame::new(3, 5).hash(), Frame::new(5, 3).hash());
    }
}
