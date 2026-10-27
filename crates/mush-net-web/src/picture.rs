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
//!
//! **The clock is the picture's rate limit** (the external report of
//! 2026-10-23, doc 21a `X-01`): its deadline ([`WebPicture::deadline`]) is
//! moved by its own tick alone ([`WebPicture::tick`]), never by a request —
//! a look change is drawn at the next scheduled tick, so a client sending
//! look updates as fast as it likes still gets at most one frame an
//! interval. Before, every request reset the deadline to now, and a loopback
//! probe drew twenty frames in 845 ms.

use std::collections::HashMap;
use std::time::Duration;

use mush_pixel::cockpit::{ARENA_MAP_FEED, COCKPIT_FEED, CockpitPicture};
use mush_pixel::overhead::OverheadPicture;
use mush_pixel::picture::{Look, Picture};
use mush_pixel::{Frame, png};
use mush_proto::{OutputEvent, PanelFrameKind, PictureKind};
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
    /// Which camera the client asked for (doc 21 cut G.12a, §7.4).
    kind: PictureKind,
    /// The latest keyframes of the two feeds the cockpit picture reads.
    feeds: HashMap<String, Value>,
    picture: CockpitPicture,
    /// The overhead (G.12b's scene), drawn from the same two feeds when
    /// `kind` is [`PictureKind::Overhead`] — the web's overhead is this
    /// socket frame until the wasm module carries the scene (§7.4).
    overhead: OverheadPicture,
    frame: Frame,
    /// The hash of the last frame sent, so an unchanged one is not.
    last_sent: Option<u64>,
    /// The frame clock's deadline: when the pump next draws. Only
    /// [`WebPicture::tick`] moves it.
    next_frame: tokio::time::Instant,
}

impl WebPicture {
    pub(crate) fn new() -> WebPicture {
        WebPicture {
            on: false,
            look_mdeg: 0,
            kind: PictureKind::Cockpit,
            feeds: HashMap::new(),
            picture: CockpitPicture::new(),
            overhead: OverheadPicture::new(),
            frame: Frame::new(FRAME_W, FRAME_H),
            last_sent: None,
            next_frame: tokio::time::Instant::now(),
        }
    }

    /// Whether the client wants frames.
    pub(crate) fn is_on(&self) -> bool {
        self.on
    }

    /// The frame clock's interval: the asked-for picture's own (20 Hz for
    /// the cockpit and the overhead alike).
    pub(crate) fn interval(&self) -> Duration {
        match self.kind {
            PictureKind::Cockpit => self.picture.interval(),
            PictureKind::Overhead => self.overhead.interval(),
        }
    }

    /// The frame clock's deadline: when the pump next draws (sleep until it,
    /// while the picture is on). Stale while the picture is off, so turning
    /// it on after a while draws at once; a request never moves it.
    pub(crate) fn deadline(&self) -> tokio::time::Instant {
        self.next_frame
    }

    /// One tick of the frame clock, at `now`: the next deadline is an
    /// interval on from this one (so the rate never drifts), or an interval
    /// from `now` when the clock is behind — the pump fell behind on a slow
    /// send, or the picture just came on after a while off — so missed ticks
    /// are skipped, never caught up in a burst.
    pub(crate) fn tick(&mut self, now: tokio::time::Instant) {
        let next = self.next_frame + self.interval();
        self.next_frame = if next > now {
            next
        } else {
            now + self.interval()
        };
    }

    /// The client's `Picture` request. Turning it on (or changing the look
    /// or the camera) forgets the last frame sent, so the next tick sends one
    /// — the next *scheduled* tick: the clock's deadline is not touched
    /// (`X-01`).
    pub(crate) fn request(&mut self, on: bool, look_mdeg: i64, kind: PictureKind) {
        if on != self.on || look_mdeg != self.look_mdeg || kind != self.kind {
            self.last_sent = None;
        }
        self.on = on;
        self.look_mdeg = look_mdeg;
        self.kind = kind;
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
            ..Look::default()
        };
        // Doc 21 cut G.12a: the camera the client asked for.
        let drawn = match self.kind {
            PictureKind::Cockpit => self.picture.draw(&mut self.frame, &self.feeds, &look),
            PictureKind::Overhead => self.overhead.draw(&mut self.frame, &self.feeds, &look),
        };
        if !drawn {
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
        p.request(true, 0, PictureKind::Cockpit);
        let png = p.frame().expect("a frame");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(p.frame().is_none(), "unchanged: nothing");
        p.request(true, 30_000, PictureKind::Cockpit);
        assert!(p.frame().is_some(), "the look moved");
        assert!(p.frame().is_none());
        p.request(false, 30_000, PictureKind::Cockpit);
        assert!(p.frame().is_none(), "off again");
        p.request(true, 30_000, PictureKind::Cockpit);
        assert!(p.frame().is_some(), "on again sends the current picture");
    }

    /// **Doc 21 cut G.12a** (§7.4): a `picture` command with `kind:
    /// "overhead"` draws the overhead from the same fixture feeds — a frame
    /// whose bytes differ from the cockpit kind's — and a command without
    /// `kind` is the cockpit; swapping the kind sends a frame at once.
    #[test]
    fn the_overhead_kind_draws_the_overhead_from_the_same_feeds() {
        use mush_proto::ClientCommand;
        let ask = |text: &str| match serde_json::from_str::<ClientCommand>(text) {
            Ok(ClientCommand::Picture {
                on,
                look_mdeg,
                kind,
            }) => (on, look_mdeg, kind),
            other => panic!("a picture command: {other:?}"),
        };
        let bare = ask(r#"{"t":"picture","d":{"on":true,"look_mdeg":0}}"#);
        assert_eq!(
            bare,
            (true, 0, PictureKind::Cockpit),
            "absent reads cockpit"
        );
        let overhead = ask(r#"{"t":"picture","d":{"on":true,"look_mdeg":0,"kind":"overhead"}}"#);
        assert_eq!(overhead, (true, 0, PictureKind::Overhead));
        let (cockpit, map) = keyframes();
        let draw = |(on, look_mdeg, kind): (bool, i64, PictureKind)| {
            let mut p = WebPicture::new();
            p.observe(&cockpit);
            p.observe(&map);
            p.request(on, look_mdeg, kind);
            p.frame().expect("a frame")
        };
        let first_person = draw(bare);
        let top_down = draw(overhead);
        assert!(top_down.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_ne!(top_down, first_person, "the overhead is its own picture");
        // One connection swapping cameras: a new frame at once, then quiet.
        let mut p = WebPicture::new();
        p.observe(&cockpit);
        p.observe(&map);
        p.request(true, 0, PictureKind::Cockpit);
        assert_eq!(p.frame().as_deref(), Some(first_person.as_slice()));
        p.request(true, 0, PictureKind::Overhead);
        assert_eq!(p.frame().as_deref(), Some(top_down.as_slice()));
        assert!(p.frame().is_none(), "unchanged: nothing");
        assert_eq!(p.interval(), Duration::from_millis(50), "the same clock");
    }

    /// Doc 21a `X-01`: the frame clock is the picture's rate limit. Its
    /// deadline moves by its own tick alone — an interval on from the last
    /// deadline, or from now when behind — and never by a request, so ten
    /// look changes inside one interval are drawn at the next tick, once.
    #[test]
    fn the_frame_clock_moves_by_its_ticks_alone() {
        let mut p = WebPicture::new();
        let interval = p.interval();
        assert_eq!(interval, Duration::from_millis(50), "the cockpit's 20 Hz");
        let t0 = p.deadline();
        p.tick(t0 + Duration::from_millis(1));
        assert_eq!(
            p.deadline(),
            t0 + interval,
            "from the deadline, not the tick"
        );
        p.tick(t0 + interval + Duration::from_millis(3));
        assert_eq!(p.deadline(), t0 + 2 * interval, "no drift");
        // Behind (a slow send, or on again after a while off): an interval
        // from now, the missed ticks skipped.
        let late = t0 + 10 * interval;
        p.tick(late);
        assert_eq!(p.deadline(), late + interval);
        p.tick(late + interval);
        assert_eq!(
            p.deadline(),
            late + 2 * interval,
            "exactly on time counts as on time"
        );

        // A request, on or any number of look changes, leaves the deadline
        // alone; the next tick draws the latest look, once.
        let (cockpit, map) = keyframes();
        p.observe(&cockpit);
        p.observe(&map);
        let before = p.deadline();
        p.request(true, 0, PictureKind::Cockpit);
        assert!(p.frame().is_some(), "on: the first frame");
        for look in 1..=10 {
            p.request(true, look * 1_000, PictureKind::Cockpit);
            assert_eq!(p.deadline(), before, "look {look}: the clock is not moved");
        }
        assert!(p.frame().is_some(), "the next tick draws the latest look");
        assert!(p.frame().is_none(), "once");
    }

    /// The feeds come from the stream itself: a `Reply`-wrapped keyframe
    /// counts, a delta and other feeds do not, and an unseated cockpit drops
    /// the held one.
    #[test]
    fn the_feeds_are_the_connections_own_keyframes() {
        let mut p = WebPicture::new();
        p.request(true, 0, PictureKind::Cockpit);
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
        p.request(true, 1, PictureKind::Cockpit);
        assert!(p.frame().is_none(), "climbed out: no cockpit");
    }
}
