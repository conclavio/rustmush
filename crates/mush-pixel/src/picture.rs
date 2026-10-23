//! **The picture seam** (doc 21 §3.2, D-21-7; cut G.1): what a renderer —
//! the SSH TUI today, the web client's socket frame (G.3) and its
//! WebAssembly render (G.3b) later — asks of the engine.
//!
//! A [`Picture`] is one producer with its instruments and its clock: it
//! reads the feeds it names from whatever holds them ([`Feeds`]), draws one
//! [`Frame`], and says how often it wants the chance. It never sees a
//! terminal, a channel, an encoder or a key: the renderer owns the probe,
//! the clock, the placement and the input, and turns keys into a [`Look`].
//!
//! Which bucket a picture is (D-21-6) shows only in its interval:
//! [`BUCKET_A_INTERVAL`] for a camera in the world, [`BUCKET_B_INTERVAL`]
//! for a diagram redrawn when its feed changes (the frame hash makes the
//! clock "on change").

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use crate::Frame;

/// Bucket A's clock: a camera in the world at 20 frames a second.
pub const BUCKET_A_INTERVAL: Duration = Duration::from_millis(50);
/// Bucket B's clock: a diagram, never faster than five frames a second.
pub const BUCKET_B_INTERVAL: Duration = Duration::from_millis(200);

/// The feeds a renderer holds, by name: the per-observer `Panel` keyframes
/// (already projected through the epistemic seam), so a picture can show
/// nothing its viewer's feeds do not carry (D-21-1).
pub trait Feeds {
    /// The held keyframe of `feed`, if any.
    fn feed(&self, feed: &str) -> Option<&Value>;
}

impl Feeds for HashMap<String, Value> {
    fn feed(&self, feed: &str) -> Option<&Value> {
        self.get(feed)
    }
}

/// What the viewer's own keys do to the camera, and nothing more: a look is
/// local and the sim is not told (D-21-14 — G.4 makes the turn a helm order
/// and keeps this as the offset between the two).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    /// The camera's heading offset from the sim's, millidegrees.
    pub offset_mdeg: i64,
    /// Of that offset, how far the hull itself has turned from the sim's
    /// heading as the viewer's helm predicts it between pulses (G.4); the
    /// rest is the look. The flight HUD marks the hull on its tape where the
    /// two part.
    pub hull_offset_mdeg: i64,
    /// A heading the viewer's helm has ordered and the hull has not reached,
    /// millidegrees (the HUD marks it on the tape).
    pub ordered_mdeg: Option<i64>,
}

/// One producer, its instruments and its clock.
pub trait Picture: std::fmt::Debug + Send {
    /// The feeds this picture draws from, for a renderer that subscribes
    /// or hydrates them.
    fn feeds(&self) -> &'static [&'static str];

    /// How often the renderer should offer it a frame.
    fn interval(&self) -> Duration;

    /// Draw into `frame` from `feeds`. Returns `false`, leaving `frame` as it
    /// was, when the feeds do not carry what it needs (no cockpit frame
    /// held): the renderer then sends nothing and says why in its own text.
    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, look: &Look) -> bool;
}
