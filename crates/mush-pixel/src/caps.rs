//! **What the terminal proved it can show** (doc 21 §4.4, D-21-5; cut G.1):
//! the primary device attributes reply and the graphics record the encoder
//! choice reads.
//!
//! The `@tui` probe sends a kitty query image and then `CSI c`. A kitty OK
//! that arrives before the DA1 reply means kitty graphics; the DA1 reply
//! itself lists attribute `4` on a terminal that does sixel, so sixel
//! detection costs nothing beyond the reply the probe already elicits.

/// A primary device attributes reply (`CSI ? <level> ; <attr> ; … c`): the
/// conformance level and the attributes below 64 (every attribute a
/// terminal lists today is: 1 132 columns, 4 sixel, 6 selective erase, 22
/// colour, 28 rectangular editing, 52 OSC 52 clipboard).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceAttributes {
    level: u16,
    attrs: u64,
}

impl DeviceAttributes {
    /// The reply's parameters as the key parser holds them, `"?62;4;22"`;
    /// `None` unless they start with `?`. A parameter that is not a number
    /// is skipped, and one of 64 or more is not recorded.
    #[must_use]
    pub fn parse(params: &str) -> Option<DeviceAttributes> {
        let mut fields = params.strip_prefix('?')?.split(';');
        let level = fields.next().and_then(|f| f.parse().ok()).unwrap_or(0);
        let attrs = fields
            .filter_map(|f| f.parse::<u32>().ok())
            .filter(|&n| n < 64)
            .fold(0u64, |mask, n| mask | 1 << n);
        Some(DeviceAttributes { level, attrs })
    }

    /// The conformance level (`62` VT220, `64` VT420, `65` VT510; `1` from
    /// a terminal that answers as a VT100).
    #[must_use]
    pub fn level(&self) -> u16 {
        self.level
    }

    /// Whether attribute `n` was listed.
    #[must_use]
    pub fn has(&self, n: u32) -> bool {
        n < 64 && self.attrs & (1 << n) != 0
    }

    /// Attribute `4`: the terminal draws sixel.
    #[must_use]
    pub fn sixel(&self) -> bool {
        self.has(4)
    }
}

/// What the probe found, once it is over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Graphics {
    /// The kitty query was answered OK before the DA1 reply.
    pub kitty: bool,
    /// The DA1 reply listed sixel.
    pub sixel: bool,
}

/// The encoder a connection's pictures go out in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoder {
    /// The kitty graphics protocol: inline PNG under one image id.
    Kitty,
}

impl Graphics {
    /// The encoder to use (D-21-5): kitty if the probe said OK, else none —
    /// the TUI's text panes stay the cockpit. Sixel, second in that order,
    /// is recorded here and chosen once its encoder lands (G.2).
    #[must_use]
    pub fn encoder(&self) -> Option<Encoder> {
        self.kitty.then_some(Encoder::Kitty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_da1_reply_reads_its_level_and_attributes() {
        let wt = DeviceAttributes::parse("?61;4;6;7;14;21;22;23;24;28;32;42");
        assert_eq!(
            wt.map(|d| (d.level(), d.sixel(), d.has(22))),
            Some((61, true, true))
        );
        let kitty = DeviceAttributes::parse("?62;c");
        assert_eq!(kitty.map(|d| (d.level(), d.sixel())), Some((62, false)));
        let bare = DeviceAttributes::parse("?1;2");
        assert_eq!(bare.map(|d| (d.level(), d.has(2))), Some((1, true)));
        // Not a DA1 reply; a parameter past 63; junk between.
        assert_eq!(DeviceAttributes::parse("62;4"), None);
        let odd = DeviceAttributes::parse("?64;;x;70;4");
        assert_eq!(odd.map(|d| (d.sixel(), d.has(70))), Some((true, false)));
        assert!(!DeviceAttributes::default().has(64));
    }

    #[test]
    fn kitty_is_chosen_and_sixel_waits_for_its_encoder() {
        let both = Graphics {
            kitty: true,
            sixel: true,
        };
        assert_eq!(both.encoder(), Some(Encoder::Kitty));
        let sixel = Graphics {
            kitty: false,
            sixel: true,
        };
        assert_eq!(sixel.encoder(), None);
        assert_eq!(Graphics::default().encoder(), None);
    }
}
