//! **What the terminal proved it can show** (doc 21 §4.4, D-21-5; cut G.1):
//! the primary device attributes reply and the graphics record the encoder
//! choice reads.
//!
//! The `@tui` probe ([`probe_bytes`]) sends a kitty query image, `CSI 16 t`
//! and then `CSI c`. A kitty OK that arrives before the DA1 reply means kitty
//! graphics; the DA1 reply itself lists attribute `4` on a terminal that does
//! sixel, so sixel detection costs nothing beyond the reply the probe already
//! elicits; the `CSI 6 ; h ; w t` reply is the size of a cell in the pixels a
//! sixel image is drawn in (G.2), which a terminal that does not answer is
//! assumed to be [`CellSize::DEFAULT`].

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

/// The whole capability probe, sent once on entering `@tui`: the kitty
/// query, the cell-size report request (`CSI 16 t`), then `CSI c`, which
/// every terminal answers last.
#[must_use]
pub fn probe_bytes() -> Vec<u8> {
    let mut out = crate::kitty::query_bytes();
    out.extend_from_slice(b"\x1b[16t");
    out.extend_from_slice(b"\x1b[c");
    out
}

/// The size of one character cell in the pixels a sixel image is drawn in:
/// the terminal's answer to `CSI 16 t`, `CSI 6 ; <height> ; <width> t`.
///
/// It is what sizes a sixel picture's cell box, because a sixel image is not
/// scaled into cells the way a kitty image is. Windows Terminal draws sixel
/// against a virtual cell of 10 by 20 whatever its font and scales the image
/// to the real cell, which is why that is the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSize {
    /// Pixels across.
    pub width: u16,
    /// Pixels down.
    pub height: u16,
}

impl CellSize {
    /// The VT340's cell, and Windows Terminal's virtual one: what a terminal
    /// that did not answer is taken to have.
    pub const DEFAULT: CellSize = CellSize {
        width: 10,
        height: 20,
    };

    /// The report's parameters as the key parser holds them, `"6;20;10"`;
    /// `None` for any other `CSI … t` report or a zero side.
    #[must_use]
    pub fn parse(params: &str) -> Option<CellSize> {
        let mut fields = params.split(';');
        if fields.next() != Some("6") {
            return None;
        }
        let height: u16 = fields.next()?.parse().ok()?;
        let width: u16 = fields.next()?.parse().ok()?;
        (width > 0 && height > 0 && fields.next().is_none()).then_some(CellSize { width, height })
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
    /// DEC sixel, drawn into the cell grid at the cursor (G.2).
    Sixel,
}

impl Encoder {
    /// The encoder's name, as `@pixel/encoder` and the HUD say it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Encoder::Kitty => "kitty",
            Encoder::Sixel => "sixel",
        }
    }
}

impl Graphics {
    /// The encoder to use (D-21-5): kitty if the probe said OK, else sixel if
    /// the DA1 reply listed it, else none — the TUI's text panes stay the
    /// cockpit.
    #[must_use]
    pub fn encoder(&self) -> Option<Encoder> {
        if self.kitty {
            Some(Encoder::Kitty)
        } else if self.sixel {
            Some(Encoder::Sixel)
        } else {
            None
        }
    }
}

/// What a player set with `@pixel/encoder` for one connection (Q-21-2,
/// D-21-17; per connection until a per-account store exists): the probe's
/// choice, one encoder forced whatever the probe found (for a terminal that
/// answers for another, as tmux does), or no picture at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncoderChoice {
    /// The probe decides (D-21-5).
    #[default]
    Auto,
    /// This encoder, without waiting on the probe.
    Forced(Encoder),
    /// No picture.
    Off,
}

impl EncoderChoice {
    /// `auto`, `kitty`, `sixel` or `off`, case-folded.
    #[must_use]
    pub fn parse(word: &str) -> Option<EncoderChoice> {
        Some(match word.to_ascii_lowercase().as_str() {
            "auto" => EncoderChoice::Auto,
            "kitty" => EncoderChoice::Forced(Encoder::Kitty),
            "sixel" => EncoderChoice::Forced(Encoder::Sixel),
            "off" => EncoderChoice::Off,
            _ => return None,
        })
    }

    /// The word [`parse`](Self::parse) reads.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            EncoderChoice::Auto => "auto",
            EncoderChoice::Forced(e) => e.name(),
            EncoderChoice::Off => "off",
        }
    }

    /// The encoder this choice yields, given what the probe found once it is
    /// over (`None` while it runs): a forced encoder does not wait for it.
    #[must_use]
    pub fn encoder(self, probed: Option<Graphics>) -> Option<Encoder> {
        match self {
            EncoderChoice::Auto => probed.and_then(|g| g.encoder()),
            EncoderChoice::Forced(e) => Some(e),
            EncoderChoice::Off => None,
        }
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
    fn kitty_then_sixel_then_nothing() {
        let both = Graphics {
            kitty: true,
            sixel: true,
        };
        assert_eq!(both.encoder(), Some(Encoder::Kitty));
        let sixel = Graphics {
            kitty: false,
            sixel: true,
        };
        assert_eq!(sixel.encoder(), Some(Encoder::Sixel));
        assert_eq!(Graphics::default().encoder(), None);
    }

    #[test]
    fn a_choice_forces_or_defers_to_the_probe() {
        let sixel = Graphics {
            kitty: false,
            sixel: true,
        };
        assert_eq!(
            EncoderChoice::Auto.encoder(Some(sixel)),
            Some(Encoder::Sixel)
        );
        assert_eq!(
            EncoderChoice::Auto.encoder(None),
            None,
            "the probe is still out"
        );
        let kitty = EncoderChoice::Forced(Encoder::Kitty);
        assert_eq!(
            kitty.encoder(None),
            Some(Encoder::Kitty),
            "forced does not wait"
        );
        assert_eq!(kitty.encoder(Some(sixel)), Some(Encoder::Kitty));
        assert_eq!(EncoderChoice::Off.encoder(Some(sixel)), None);
        for word in ["auto", "KITTY", "Sixel", "off"] {
            let choice = EncoderChoice::parse(word);
            assert_eq!(
                choice.map(EncoderChoice::name),
                Some(word.to_ascii_lowercase().as_str())
            );
        }
        assert_eq!(EncoderChoice::parse("png"), None);
        assert_eq!(EncoderChoice::default(), EncoderChoice::Auto);
    }

    #[test]
    fn the_cell_size_report_and_the_probe() {
        assert_eq!(
            CellSize::parse("6;20;10"),
            Some(CellSize {
                width: 10,
                height: 20
            })
        );
        assert_eq!(CellSize::parse("6;0;10"), None);
        assert_eq!(
            CellSize::parse("4;480;640"),
            None,
            "the text area, not a cell"
        );
        assert_eq!(CellSize::parse("6;20"), None);
        assert_eq!(CellSize::parse("6;20;10;1"), None);
        let probe = String::from_utf8_lossy(&probe_bytes()).into_owned();
        assert!(probe.starts_with("\x1b_Gi=31,"), "{probe:?}");
        assert!(probe.ends_with("\x1b\\\x1b[16t\x1b[c"), "{probe:?}");
    }
}
