//! Client capability description, negotiated at connect time and updated on
//! resize.

use serde::{Deserialize, Serialize};

/// Which transport the client connected over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// SSH connection (ANSI-rendered text).
    Ssh,
    /// WebSocket connection (structured JSON events).
    Web,
}

/// How much color the connection can display; the ANSI renderer downgrades
/// [`Color`](crate::Color) values to fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ColorDepth {
    /// No color at all (`TERM=dumb`, `NOCOLOR`).
    None,
    /// Basic 16-color ANSI.
    Ansi16,
    /// xterm 256-color palette.
    Ansi256,
    /// 24-bit true color.
    TrueColor,
}

/// What one connection can display, detected at connect time and updated in
/// place on resize. SSH derives it from the PTY request / window-change
/// messages; the web client sends it explicitly via
/// [`ClientCommand::Caps`](crate::ClientCommand::Caps).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ClientCaps {
    /// Transport kind.
    pub kind: ClientKind,
    /// Color support.
    pub color: ColorDepth,
    /// Terminal/viewport width in columns.
    pub width: u16,
    /// Terminal/viewport height in rows.
    pub height: u16,
    /// Whether the client accepts UTF-8 output (else ASCII fallback art).
    pub utf8: bool,
    /// `true` when the client consumes structured JSON events (web);
    /// `false` when it gets ANSI-rendered text (SSH).
    pub structured: bool,
}
