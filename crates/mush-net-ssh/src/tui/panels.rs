//! [`PanelStore`] — the TUI's copy of every panel feed and semantic view it
//! has seen, bounded and keyed by name (interface plan I2).
//!
//! Keyframes replace. A delta applies through a per-feed merge function —
//! `comsys` folds a backlog batch into its per-channel ring, `weather`
//! shallow-merges — and a delta for any other feed asks for one resync (a
//! keyframe query under the feed's name) with the same storm guard the
//! legal-actions view already had: a second delta before the replacement
//! arrives asks nothing. At most [`MAX_TUI_FEEDS`] feeds are kept, each at
//! most `MAX_VIEW_DATA_BYTES` serialized; an unknown feed is stored,
//! bounded and never painted.

use std::collections::{HashMap, HashSet, VecDeque};

use mush_proto::{
    MAX_VIEW_DATA_BYTES, PanelFrameKind, SmolStr, ViewCursor, ViewEnvelope, ViewFrameDisposition,
    VitalsFrame,
};
use serde_json::Value;

/// The most feeds the store keeps.
pub const MAX_TUI_FEEDS: usize = 32;
/// The most `.roll` frames the rolls ring keeps.
pub const ROLLS_RING: usize = 100;
/// The most messages kept per channel.
pub const BACKLOG_RING: usize = 200;

/// One feed's latest state.
#[derive(Clone, Debug)]
pub struct PanelState {
    /// The latest data (a keyframe, or a keyframe with deltas merged).
    pub data: Value,
    /// Frames applied since the first.
    pub revision: u64,
    /// When the last frame was applied (Unix seconds).
    pub updated_at: i64,
}

/// What applying a frame asks of the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelOutcome {
    /// Stored (or merged); repaint.
    Stored,
    /// Dropped: over a cap, or a stale view frame.
    Ignored,
    /// A delta could not be applied; query the named feed for a keyframe.
    NeedsResync(SmolStr),
}

/// One channel message kept for the backlog pane, live or recalled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChanMsg {
    /// Channel sequence number.
    pub seq: u64,
    /// Speaker name.
    pub actor: String,
    /// The line as it reads (`Name: text`, `Name poses`, …).
    pub line: String,
}

/// The store.
#[derive(Debug, Default)]
pub struct PanelStore {
    feeds: HashMap<SmolStr, PanelState>,
    view_cursors: HashMap<SmolStr, ViewCursor>,
    resync_pending: HashSet<SmolStr>,
    rolls: VecDeque<Value>,
    backlog: HashMap<String, VecDeque<ChanMsg>>,
}

impl PanelStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The latest data of `feed`.
    #[must_use]
    pub fn get(&self, feed: &str) -> Option<&Value> {
        self.feeds.get(feed).map(|p| &p.data)
    }

    /// The latest state of `feed`.
    #[must_use]
    pub fn state(&self, feed: &str) -> Option<&PanelState> {
        self.feeds.get(feed)
    }

    /// Feed names held, sorted.
    #[must_use]
    pub fn feeds(&self) -> Vec<SmolStr> {
        let mut names: Vec<SmolStr> = self.feeds.keys().cloned().collect();
        names.sort();
        names
    }

    /// Every `.vitals` frame held, by feed name.
    #[must_use]
    pub fn vitals(&self) -> Vec<(SmolStr, VitalsFrame)> {
        let mut out: Vec<(SmolStr, VitalsFrame)> = self
            .feeds
            .iter()
            .filter(|(name, _)| name.ends_with(".vitals"))
            .filter_map(|(name, st)| {
                VitalsFrame::from_panel_data(&st.data)
                    .ok()
                    .map(|f| (name.clone(), f))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The first `.sheet` keyframe held.
    #[must_use]
    pub fn sheet(&self) -> Option<&Value> {
        let mut names: Vec<&SmolStr> = self
            .feeds
            .keys()
            .filter(|n| n.ends_with(".sheet"))
            .collect();
        names.sort();
        names
            .first()
            .and_then(|n| self.feeds.get(*n))
            .map(|p| &p.data)
    }

    /// The `.roll` frames, oldest first.
    #[must_use]
    pub fn rolls(&self) -> &VecDeque<Value> {
        &self.rolls
    }

    /// The messages kept for `channel`, oldest first.
    #[must_use]
    pub fn backlog(&self, channel: &str) -> Vec<&ChanMsg> {
        self.backlog
            .get(&channel.to_lowercase())
            .map(|ring| ring.iter().collect())
            .unwrap_or_default()
    }

    /// Fold a live channel line into the backlog ring.
    pub fn push_channel(&mut self, channel: &str, msg: ChanMsg) {
        let ring = self.backlog.entry(channel.to_lowercase()).or_default();
        if ring.iter().any(|m| m.seq == msg.seq) {
            return;
        }
        ring.push_back(msg);
        ring.make_contiguous().sort_by_key(|m| m.seq);
        while ring.len() > BACKLOG_RING {
            ring.pop_front();
        }
    }

    /// Apply a panel frame.
    pub fn apply_panel(
        &mut self,
        feed: &SmolStr,
        kind: PanelFrameKind,
        data: &Value,
        now: i64,
    ) -> PanelOutcome {
        if !within_bounds(data) {
            // An over-bounds keyframe must still clear a pending resync, or a
            // >1 MiB frame would silence recovery for the rest of the session
            // (interface plan I13 bounds hardening) — a later, smaller frame
            // can then re-seed.
            if kind == PanelFrameKind::Keyframe {
                self.resync_pending.remove(feed);
            }
            return PanelOutcome::Ignored;
        }
        if feed.ends_with(".roll") {
            if kind == PanelFrameKind::Keyframe {
                self.rolls.push_back(data.clone());
                while self.rolls.len() > ROLLS_RING {
                    self.rolls.pop_front();
                }
            }
            return self.store(feed, data.clone(), now);
        }
        match kind {
            PanelFrameKind::Keyframe => {
                self.resync_pending.remove(feed);
                self.store(feed, data.clone(), now)
            }
            PanelFrameKind::Delta if feed.ends_with(".sheet") => {
                self.apply_sheet_delta(feed, data, now)
            }
            PanelFrameKind::Delta => match feed.as_str() {
                "comsys" => {
                    if let Some(backlog) = data.get("backlog") {
                        self.merge_backlog(backlog);
                        PanelOutcome::Stored
                    } else {
                        self.resync(feed)
                    }
                }
                "weather" => {
                    let merged = match (self.feeds.get(feed).map(|p| &p.data), data) {
                        (Some(Value::Object(base)), Value::Object(patch)) => {
                            let mut m = base.clone();
                            for (k, v) in patch {
                                m.insert(k.clone(), v.clone());
                            }
                            Value::Object(m)
                        }
                        (None, Value::Object(_)) => data.clone(),
                        _ => return self.resync(feed),
                    };
                    self.store(feed, merged, now)
                }
                _ => self.resync(feed),
            },
        }
    }

    /// Apply a `<ruleset>.sheet` delta (interface plan I13): the delta carries
    /// the whole staff-filtered `data` plus `revision`/`base_revision`. It
    /// applies only when `base_revision` matches the held server `revision`
    /// (read from the held frame's own data, NOT the local `PanelState`
    /// counter); a stale delta is ignored and a gap self-heals with a keyframe
    /// re-request. The merge replaces `data`/`approved`/`revision` and leaves
    /// the omitted `schema` from the held keyframe intact.
    fn apply_sheet_delta(&mut self, feed: &SmolStr, data: &Value, now: i64) -> PanelOutcome {
        let Some(base_frame) = self.feeds.get(feed).map(|p| p.data.clone()) else {
            // No held keyframe to base the delta on: fail closed, rehydrate.
            return self.resync(feed);
        };
        let cur = base_frame
            .get("revision")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let rev = data.get("revision").and_then(Value::as_u64);
        let base = data.get("base_revision").and_then(Value::as_u64);
        match (rev, base) {
            (Some(rev), _) if rev <= cur => PanelOutcome::Ignored,
            (Some(_), Some(base)) if base == cur => match (base_frame, data) {
                (Value::Object(mut merged), Value::Object(patch)) => {
                    for (k, v) in patch {
                        merged.insert(k.clone(), v.clone());
                    }
                    self.store(feed, Value::Object(merged), now)
                }
                _ => self.resync(feed),
            },
            // A gap (base != cur), or a malformed frame: rehydrate, once.
            _ => self.resync(feed),
        }
    }

    /// Apply a semantic view frame (`OutputEvent::View`), with the
    /// gap-detecting disposition every view shares.
    pub fn apply_view(&mut self, view: &ViewEnvelope, now: i64) -> PanelOutcome {
        let name = SmolStr::new(view.view());
        let current = self.view_cursors.get(&name);
        match view.disposition(view.view(), 1, current) {
            ViewFrameDisposition::Replace => {
                if !within_bounds(view.data()) {
                    return PanelOutcome::Ignored;
                }
                self.view_cursors.insert(name.clone(), view.cursor());
                self.resync_pending.remove(&name);
                self.store(&name, view.data().clone(), now)
            }
            ViewFrameDisposition::IgnoreStale => PanelOutcome::Ignored,
            // No view registers a merge contract yet: fail closed and
            // rehydrate, once.
            ViewFrameDisposition::ApplyDelta | ViewFrameDisposition::ResyncRequired => {
                self.feeds.remove(&name);
                self.resync(&name)
            }
        }
    }

    /// Whether a resync is outstanding for `feed`.
    #[must_use]
    pub fn resync_pending(&self, feed: &str) -> bool {
        self.resync_pending.contains(feed)
    }

    fn store(&mut self, feed: &SmolStr, data: Value, now: i64) -> PanelOutcome {
        if let Some(st) = self.feeds.get_mut(feed) {
            st.data = data;
            st.revision += 1;
            st.updated_at = now;
            return PanelOutcome::Stored;
        }
        if self.feeds.len() >= MAX_TUI_FEEDS {
            return PanelOutcome::Ignored;
        }
        self.feeds.insert(
            feed.clone(),
            PanelState {
                data,
                revision: 1,
                updated_at: now,
            },
        );
        PanelOutcome::Stored
    }

    fn resync(&mut self, feed: &SmolStr) -> PanelOutcome {
        if self.resync_pending.insert(feed.clone()) {
            PanelOutcome::NeedsResync(feed.clone())
        } else {
            PanelOutcome::Ignored
        }
    }

    fn merge_backlog(&mut self, backlog: &Value) {
        let Some(channel) = backlog.get("channel").and_then(Value::as_str) else {
            return;
        };
        let Some(messages) = backlog.get("messages").and_then(Value::as_array) else {
            return;
        };
        for m in messages {
            let seq = m.get("seq").and_then(Value::as_u64).unwrap_or(0);
            let actor = m
                .get("actor")
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let kind = m.get("kind").and_then(Value::as_str).unwrap_or("say");
            let text = m.get("text").map(rich_plain).unwrap_or_default();
            let line = match kind {
                "pose" => format!("{actor} {text}"),
                "semi_pose" => format!("{actor}{text}"),
                "emit" | "spoof" => text,
                _ => format!("{actor}: {text}"),
            };
            self.push_channel(channel, ChanMsg { seq, actor, line });
        }
    }
}

/// The plain text of a serialized `RichText` (an array of spans) or a bare
/// string, control characters dropped.
#[must_use]
pub fn rich_plain(v: &Value) -> String {
    let raw = match v {
        Value::Array(spans) => spans
            .iter()
            .filter_map(|s| s.get("text").and_then(Value::as_str))
            .collect::<String>(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    };
    raw.chars().filter(|c| !c.is_control()).collect()
}

fn within_bounds(data: &Value) -> bool {
    serde_json::to_vec(data).is_ok_and(|b| b.len() <= MAX_VIEW_DATA_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyframes_replace_and_unknown_feeds_are_bounded() {
        let mut st = PanelStore::new();
        let f = SmolStr::new("mystery");
        assert_eq!(
            st.apply_panel(
                &f,
                PanelFrameKind::Keyframe,
                &serde_json::json!({"a": 1}),
                5
            ),
            PanelOutcome::Stored
        );
        assert_eq!(
            st.apply_panel(
                &f,
                PanelFrameKind::Keyframe,
                &serde_json::json!({"a": 2}),
                6
            ),
            PanelOutcome::Stored
        );
        assert_eq!(st.get("mystery"), Some(&serde_json::json!({"a": 2})));
        assert_eq!(st.state("mystery").unwrap().revision, 2);
        assert_eq!(st.state("mystery").unwrap().updated_at, 6);
        for i in 0..(MAX_TUI_FEEDS + 5) {
            st.apply_panel(
                &SmolStr::new(format!("feed{i}")),
                PanelFrameKind::Keyframe,
                &Value::Null,
                0,
            );
        }
        assert_eq!(st.feeds().len(), MAX_TUI_FEEDS);
        let huge = Value::String("x".repeat(MAX_VIEW_DATA_BYTES + 1));
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Keyframe, &huge, 0),
            PanelOutcome::Ignored
        );
    }

    #[test]
    fn a_delta_without_a_merge_function_asks_for_exactly_one_resync() {
        let mut st = PanelStore::new();
        let f = SmolStr::new("intel");
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &serde_json::json!({}), 0),
            PanelOutcome::NeedsResync(f.clone())
        );
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &serde_json::json!({}), 0),
            PanelOutcome::Ignored,
            "no storm"
        );
        assert!(st.resync_pending("intel"));
        st.apply_panel(
            &f,
            PanelFrameKind::Keyframe,
            &serde_json::json!({"records": []}),
            0,
        );
        assert!(!st.resync_pending("intel"));
    }

    #[test]
    fn comsys_backlog_and_weather_deltas_merge() {
        let mut st = PanelStore::new();
        let comsys = SmolStr::new("comsys");
        let delta = serde_json::json!({ "backlog": { "channel": "Public", "before_seq": null,
            "messages": [
                { "seq": 3, "actor": { "name": "Zoe" }, "kind": "say", "text": [ { "text": "later" } ] },
                { "seq": 2, "actor": { "name": "Al" }, "kind": "pose", "text": "waves" }
            ] } });
        assert_eq!(
            st.apply_panel(&comsys, PanelFrameKind::Delta, &delta, 0),
            PanelOutcome::Stored
        );
        st.push_channel(
            "public",
            ChanMsg {
                seq: 4,
                actor: "Zoe".into(),
                line: "Zoe: newest".into(),
            },
        );
        let lines: Vec<&str> = st
            .backlog("PUBLIC")
            .iter()
            .map(|m| m.line.as_str())
            .collect();
        assert_eq!(lines, vec!["Al waves", "Zoe: later", "Zoe: newest"]);
        let weather = SmolStr::new("weather");
        st.apply_panel(
            &weather,
            PanelFrameKind::Keyframe,
            &serde_json::json!({ "zone": "z", "state": "rain", "temp": 10 }),
            0,
        );
        st.apply_panel(
            &weather,
            PanelFrameKind::Delta,
            &serde_json::json!({ "state": "fog" }),
            1,
        );
        assert_eq!(
            st.get("weather"),
            Some(&serde_json::json!({ "zone": "z", "state": "fog", "temp": 10 }))
        );
    }

    #[test]
    fn rolls_ring_and_vitals_and_sheet_lookups() {
        let mut st = PanelStore::new();
        for i in 0..(ROLLS_RING + 3) {
            st.apply_panel(
                &SmolStr::new("wod.roll"),
                PanelFrameKind::Keyframe,
                &serde_json::json!({ "summary": format!("r{i}") }),
                0,
            );
        }
        assert_eq!(st.rolls().len(), ROLLS_RING);
        st.apply_panel(
            &SmolStr::new("wod.sheet"),
            PanelFrameKind::Keyframe,
            &serde_json::json!({ "schema": {} }),
            0,
        );
        assert!(st.sheet().is_some());
        st.apply_panel(
            &SmolStr::new("wod.vitals"),
            PanelFrameKind::Keyframe,
            &serde_json::json!({ "subject": { "id": "#5", "name": "N" }, "revision": 1, "vitals": [] }),
            0,
        );
        assert_eq!(st.vitals().len(), 1);
        st.apply_panel(
            &SmolStr::new("bad.vitals"),
            PanelFrameKind::Keyframe,
            &serde_json::json!({ "nope": 1 }),
            0,
        );
        assert_eq!(st.vitals().len(), 1, "a malformed frame is skipped");
    }

    #[test]
    fn views_follow_the_gap_detecting_disposition() {
        let mut st = PanelStore::new();
        let data = serde_json::json!([{ "presentation": { "verb": "brace" } }]);
        let kf = ViewEnvelope::keyframe("legal.actions", 1, 7, 3, data.clone()).unwrap();
        assert_eq!(st.apply_view(&kf, 0), PanelOutcome::Stored);
        assert_eq!(st.get("legal.actions"), Some(&data));
        let gap = ViewEnvelope::delta("legal.actions", 1, 7, 5, 4, serde_json::json!({})).unwrap();
        assert_eq!(
            st.apply_view(&gap, 0),
            PanelOutcome::NeedsResync(SmolStr::new("legal.actions"))
        );
        assert!(st.get("legal.actions").is_none(), "stale buttons vanish");
        assert_eq!(st.apply_view(&gap, 0), PanelOutcome::Ignored);
        let replacement = ViewEnvelope::keyframe("legal.actions", 1, 7, 6, data.clone()).unwrap();
        assert_eq!(st.apply_view(&replacement, 0), PanelOutcome::Stored);
        let stale = ViewEnvelope::keyframe("legal.actions", 1, 7, 2, data).unwrap();
        assert_eq!(st.apply_view(&stale, 0), PanelOutcome::Ignored);
    }

    #[test]
    fn a_sheet_delta_applies_on_a_matching_base_ignores_stale_and_self_heals_on_a_gap() {
        let mut st = PanelStore::new();
        let f = SmolStr::new("wod.sheet");
        // Seed a keyframe (revision 5, with schema).
        st.apply_panel(
            &f,
            PanelFrameKind::Keyframe,
            &serde_json::json!({ "schema": { "s": 1 }, "data": { "strength": 3 }, "revision": 5 }),
            0,
        );

        // A matching delta (base 5 → rev 6): data replaced, omitted schema kept.
        assert_eq!(
            st.apply_panel(
                &f,
                PanelFrameKind::Delta,
                &serde_json::json!({ "data": { "strength": 4 }, "revision": 6, "base_revision": 5 }),
                1,
            ),
            PanelOutcome::Stored
        );
        let held = st.get("wod.sheet").unwrap();
        assert_eq!(
            held.get("data").unwrap(),
            &serde_json::json!({ "strength": 4 })
        );
        assert_eq!(held.get("revision").and_then(Value::as_u64), Some(6));
        assert!(
            held.get("schema").is_some(),
            "the omitted schema is preserved"
        );

        // A stale delta (rev <= held) is ignored, data untouched.
        assert_eq!(
            st.apply_panel(
                &f,
                PanelFrameKind::Delta,
                &serde_json::json!({ "data": { "strength": 9 }, "revision": 6, "base_revision": 5 }),
                2,
            ),
            PanelOutcome::Ignored
        );
        assert_eq!(
            st.get("wod.sheet").unwrap().get("data").unwrap(),
            &serde_json::json!({ "strength": 4 })
        );

        // A gap (base 8 != held 6) self-heals with exactly one resync; data intact.
        assert_eq!(
            st.apply_panel(
                &f,
                PanelFrameKind::Delta,
                &serde_json::json!({ "data": { "strength": 1 }, "revision": 9, "base_revision": 8 }),
                3,
            ),
            PanelOutcome::NeedsResync(f.clone())
        );
        assert_eq!(
            st.get("wod.sheet").unwrap().get("data").unwrap(),
            &serde_json::json!({ "strength": 4 })
        );
    }

    #[test]
    fn a_sheet_delta_before_any_keyframe_asks_for_a_resync() {
        let mut st = PanelStore::new();
        let f = SmolStr::new("wod.sheet");
        assert_eq!(
            st.apply_panel(
                &f,
                PanelFrameKind::Delta,
                &serde_json::json!({ "data": { "a": 1 }, "revision": 2, "base_revision": 1 }),
                0,
            ),
            PanelOutcome::NeedsResync(f.clone())
        );
    }
}
