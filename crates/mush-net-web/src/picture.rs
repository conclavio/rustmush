//! **The web client's picture** (doc 21 cut G.3, D-21-4): the gateway draws
//! the first-person cockpit for a web viewer exactly as it does for an SSH
//! one, and sends each changed frame as a PNG in a binary WebSocket message.
//!
//! The picture is drawn from **this connection's own feeds**: every envelope
//! bound for the socket passes [`WebPicture::observe`] on its way out, which
//! keeps the latest `arena.cockpit` and `arena.map` keyframes (pushed, or a
//! query's `Reply`). So a web viewer's picture shows what its own stream
//! carries and nothing more (D-21-1), and a delta, which these two feeds do
//! not carry today, simply waits for the keyframe the client asks for.
//!
//! The client asks with [`ClientCommand::Picture`](mush_proto::ClientCommand):
//! on, off, and the look (the camera's own offset; turning the tank is a
//! `HEADING` line like any other). While it is on, the socket's pump runs a
//! frame clock at the picture's interval and sends a frame only when its
//! pixels changed — an idle cockpit costs nothing on the wire.

use std::collections::HashMap;
use std::time::Duration;

use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture};
use mush_pixel::picture::{Look, Picture};
use mush_pixel::{Frame, png};
use mush_proto::{OutputEvent, PanelFrameKind};
use serde_json::Value;

/// The frame a web viewer gets: the target size (the owner's ruling of
/// 2026-10-21); the browser scales it into its canvas.
pub const FRAME_W: usize = 640;
/// See [`FRAME_W`].
pub const FRAME_H: usize = 480;

/// One web connection's picture state.
#[derive(Debug)]
pub(crate) struct WebPicture {
    on: bool,
    look_mdeg: i64,
    /// The latest keyframes of the two feeds the cockpit picture reads.
    feeds: HashMap<String, Value>,
    picture: CockpitPicture,
    frame: Frame,
    /// The hash of the last frame sent, so an unchanged one is not.
    last_sent: Option<u64>,
}

impl WebPicture {
    pub(crate) fn new() -> WebPicture {
        WebPicture {
            on: false,
            look_mdeg: 0,
            feeds: HashMap::new(),
            picture: CockpitPicture::new(),
            frame: Frame::new(FRAME_W, FRAME_H),
            last_sent: None,
        }
    }

    /// Whether the client wants frames.
    pub(crate) fn is_on(&self) -> bool {
        self.on
    }

    /// The frame clock's interval: the picture's own (20 Hz for the cockpit).
    pub(crate) fn interval(&self) -> Duration {
        self.picture.interval()
    }

    /// The client's `Picture` request. Turning it on (or changing the look)
    /// forgets the last frame sent, so the next tick sends one.
    pub(crate) fn request(&mut self, on: bool, look_mdeg: i64) {
        if on != self.on || look_mdeg != self.look_mdeg {
            self.last_sent = None;
        }
        self.on = on;
        self.look_mdeg = look_mdeg;
    }

    /// An event on its way to the socket: keep the two feeds' keyframes. A
    /// cockpit keyframe that says the viewer is not seated drops the held
    /// one, so a pilot who climbs out stops getting a cockpit.
    pub(crate) fn observe(&mut self, event: &OutputEvent) {
        match event {
            OutputEvent::Panel {
                feed,
                kind: PanelFrameKind::Keyframe,
                data,
            } if feed == COCKPIT_FEED || feed == ARENA_MAP_FEED => {
                if feed == COCKPIT_FEED
                    && data.get("seated").and_then(Value::as_bool) == Some(false)
                {
                    self.feeds.remove(COCKPIT_FEED);
                } else {
                    self.feeds.insert(feed.to_string(), data.clone());
                }
            }
            OutputEvent::Reply { event, .. } => self.observe(event),
            _ => {}
        }
    }

    /// One tick of the frame clock: the PNG to send, or nothing (off, no
    /// cockpit frame held, or the same picture as the last one sent).
    pub(crate) fn frame(&mut self) -> Option<Vec<u8>> {
        if !self.on {
            return None;
        }
        let look = Look {
            offset_mdeg: self.look_mdeg,
        };
        if !self.picture.draw(&mut self.frame, &self.feeds, &look) {
            return None;
        }
        let hash = self.frame.hash();
        if self.last_sent == Some(hash) {
            return None;
        }
        self.last_sent = Some(hash);
        Some(png::encode(&self.frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COCKPIT: &str = include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json");
    const MAP: &str = include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json");

    fn panel(feed: &str, kind: PanelFrameKind, data: Value) -> OutputEvent {
        OutputEvent::Panel {
            feed: feed.into(),
            kind,
            data,
        }
    }

    fn keyframes() -> (OutputEvent, OutputEvent) {
        (
            panel(
                COCKPIT_FEED,
                PanelFrameKind::Keyframe,
                serde_json::from_str(COCKPIT).unwrap(),
            ),
            panel(
                ARENA_MAP_FEED,
                PanelFrameKind::Keyframe,
                serde_json::from_str(MAP).unwrap(),
            ),
        )
    }

    /// Off, nothing; on without a cockpit, nothing; with one, a PNG, once,
    /// until the look or the frame changes.
    #[test]
    fn a_frame_goes_out_only_when_wanted_held_and_changed() {
        let mut p = WebPicture::new();
        let (cockpit, map) = keyframes();
        p.observe(&cockpit);
        p.observe(&map);
        assert!(p.frame().is_none(), "off");
        p.request(true, 0);
        let png = p.frame().expect("a frame");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(p.frame().is_none(), "unchanged: nothing");
        p.request(true, 30_000);
        assert!(p.frame().is_some(), "the look moved");
        assert!(p.frame().is_none());
        p.request(false, 30_000);
        assert!(p.frame().is_none(), "off again");
        p.request(true, 30_000);
        assert!(p.frame().is_some(), "on again sends the current picture");
    }

    /// The feeds come from the stream itself: a `Reply`-wrapped keyframe
    /// counts, a delta and other feeds do not, and an unseated cockpit drops
    /// the held one.
    #[test]
    fn the_feeds_are_the_connections_own_keyframes() {
        let mut p = WebPicture::new();
        p.request(true, 0);
        let (cockpit, map) = keyframes();
        p.observe(&panel(
            COCKPIT_FEED,
            PanelFrameKind::Delta,
            serde_json::json!({}),
        ));
        p.observe(&panel(
            "wod.sheet",
            PanelFrameKind::Keyframe,
            serde_json::json!({}),
        ));
        assert!(p.frame().is_none(), "no cockpit keyframe held");
        p.observe(&OutputEvent::Reply {
            id: 4,
            event: Box::new(cockpit),
        });
        p.observe(&map);
        assert!(p.frame().is_some(), "a reply's keyframe is held");
        p.observe(&panel(
            COCKPIT_FEED,
            PanelFrameKind::Keyframe,
            serde_json::json!({ "seated": false }),
        ));
        p.request(true, 1);
        assert!(p.frame().is_none(), "climbed out: no cockpit");
    }
}
