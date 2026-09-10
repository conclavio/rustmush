//! [`PanelStore`] — the TUI's copy of every panel feed and semantic view it
//! has seen, bounded and keyed by name (interface plan I2).
//!
//! Keyframes replace. A delta applies through a per-feed merge function —
//! `comsys` folds a backlog batch into its per-channel ring, `weather`
//! shallow-merges — and a delta for any other feed asks for one resync (a
//! keyframe query under the feed's name) with the same storm guard the
//! legal-actions view already had: a second delta before the replacement
//! arrives asks nothing. The guard AGES ([`RESYNC_LATCH_S`]): the SSH query
//! seam answers a throttled query with no reply at all, so a latch that only
//! a keyframe could clear would freeze a pane for the rest of the session.
//! At most [`MAX_TUI_FEEDS`] feeds are kept, each at most
//! `MAX_VIEW_DATA_BYTES` serialized; an unknown feed is stored, bounded and
//! never painted.

use std::collections::{HashMap, VecDeque};

use mush_proto::{
    MAX_VIEW_DATA_BYTES, PanelFrameKind, SmolStr, ViewCursor, ViewEnvelope, ViewFrameDisposition,
    VitalsFrame,
};
use serde_json::Value;

use super::app::TuiManifest;

/// The most feeds the store keeps.
pub const MAX_TUI_FEEDS: usize = 32;
/// The most `.roll` frames the rolls ring keeps.
pub const ROLLS_RING: usize = 100;
/// The most messages kept per channel.
pub const BACKLOG_RING: usize = 200;
/// The most `tactics.unit` journal rows the ring keeps (interface plan I8;
/// the web client keeps the same number, so the two read alike).
pub const JOURNAL_RING: usize = 200;
/// The most rows one delta may contribute to the journal ring. A delta with
/// more entries than this contributes its first `JOURNAL_ROWS_PER_DELTA - 1`
/// and one `elided` row carrying the count omitted, so a 1024-entry burst can
/// never flood the ring or hide that it was cut.
pub const JOURNAL_ROWS_PER_DELTA: usize = 50;
/// The feed whose deltas are kept as an after-action journal.
pub const UNIT_FEED: &str = "tactics.unit";
/// The contacts table (interface plan I8, D-I8-1): a generic-view keyframe
/// the unit screen renders with the I9 renderer, subscribed only when the
/// manifest carries it.
pub const CONTACTS_FEED: &str = "tactics.contacts";
/// The tactical map (doc 14 §6, S4): a grid-shaped keyframe of the window
/// the viewer stands on, rendered by the same lines line mode prints.
pub const MAP_FEED: &str = "atlas.map";
/// How long a resync request stays latched before another delta may ask
/// again. The SSH bridge drops a throttled query silently (no reply, not
/// even an error), so a latch cleared only by the replacement keyframe is
/// permanent for the session once a flood bucket is hit. Five whole seconds
/// is well outside a healthy round trip and still coalesces a damage burst.
pub const RESYNC_LATCH_S: i64 = 5;

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
    /// Feeds with a keyframe request outstanding, and the whole second it
    /// was armed at — see [`RESYNC_LATCH_S`].
    resync_pending: HashMap<SmolStr, i64>,
    rolls: VecDeque<Value>,
    unit_journal: VecDeque<Value>,
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

    /// The `.unit` feed THIS world publishes, resolved by suffix from the
    /// manifest exactly as [`Self::sheet`] resolves `.sheet` — sorted first,
    /// so two of them pick deterministically.
    ///
    /// Look-back LB-12: `tactics.unit` is only the name the tacsim pack
    /// happens to use, and screens 8/9 promise to follow any `<pack>.unit`
    /// feed "without a client change" while the code hard-coded
    /// [`UNIT_FEED`]; the web already suffix-routed it. An empty manifest is a
    /// hello that has not landed, and the screens must still name a feed, so
    /// [`UNIT_FEED`] is the fallback.
    #[must_use]
    pub fn unit_feed(manifest: &TuiManifest) -> &str {
        let mut names: Vec<&str> = manifest
            .panels
            .iter()
            .map(String::as_str)
            .filter(|n| n.ends_with(".unit"))
            .collect();
        names.sort_unstable();
        names.first().copied().unwrap_or(UNIT_FEED)
    }

    /// The `.roll` frames, oldest first.
    #[must_use]
    pub fn rolls(&self) -> &VecDeque<Value> {
        &self.rolls
    }

    /// The `tactics.unit` journal ring, oldest first (interface plan I8).
    ///
    /// One row per projected delta entry:
    /// `{ object, rev, cause, at, entry }` — `object` is the delta's own
    /// subject id, which need not be the subject of the held keyframe (the
    /// resync path always asks for the viewer's default subject), so a
    /// renderer must read each row's `object` rather than assume it.
    #[must_use]
    pub fn unit_journal(&self) -> &VecDeque<Value> {
        &self.unit_journal
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
                if feed.ends_with(".unit") {
                    // A keyframe for another subject makes the journal rows of
                    // the one we held meaningless: drop them. "Held" is the
                    // STORED frame, not its object id — a `null` keyframe is
                    // a frame (the viewer lost the station), so null → #5
                    // clears just as #7 → #5 does, while null → null (a
                    // resync answered for a viewer with no station) keeps the
                    // rows the after-action panes render. Nothing STORED yet
                    // is not a change: the fan-out delivers the delta first
                    // and the keyframe only after the resync, so clearing
                    // there would throw away the very rows the before/after
                    // pane exists to show.
                    let stored = self.get(feed);
                    let changed =
                        stored.is_some() && stored.and_then(unit_object_id) != unit_object_id(data);
                    if changed {
                        self.unit_journal.clear();
                    }
                }
                self.resync_pending.remove(feed);
                self.store(feed, data.clone(), now)
            }
            PanelFrameKind::Delta if feed.ends_with(".sheet") => {
                self.apply_sheet_delta(feed, data, now)
            }
            // The after-action journal (interface plan I8), on any `.unit`
            // feed (look-back LB-12). The delta is the only place the
            // before/after sides are ever carried, so it is kept in the ring
            // BEFORE asking for a keyframe: the storm guard answers `Ignored`
            // for the 2nd..nth delta while one request is outstanding, and
            // every one of those still belongs in the ring.
            PanelFrameKind::Delta if feed.ends_with(".unit") => {
                self.push_unit_journal(data);
                self.resync(feed, now)
            }
            PanelFrameKind::Delta => match feed.as_str() {
                "comsys" => {
                    if let Some(backlog) = data.get("backlog") {
                        self.merge_backlog(backlog);
                        PanelOutcome::Stored
                    } else {
                        self.resync(feed, now)
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
                        _ => return self.resync(feed, now),
                    };
                    self.store(feed, merged, now)
                }
                _ => self.resync(feed, now),
            },
        }
    }

    /// Fold one projected `tactics.unit` delta into the journal ring: one row
    /// per entry, tagged with the delta's own subject, revision, cause and
    /// clock. A delta longer than [`JOURNAL_ROWS_PER_DELTA`] contributes its
    /// first `JOURNAL_ROWS_PER_DELTA - 1` entries and one `elided` row.
    fn push_unit_journal(&mut self, delta: &Value) {
        let object = delta
            .get("object")
            .and_then(|o| o.get("id"))
            .and_then(Value::as_str)
            .map_or(Value::Null, |id| Value::String(id.to_string()));
        let rev = delta
            .get("to_revision")
            .and_then(Value::as_u64)
            .map_or(Value::Null, Value::from);
        let cause = delta.get("cause").cloned().unwrap_or(Value::Null);
        let at = delta.get("now").cloned().unwrap_or(Value::Null);
        let empty = Vec::new();
        let entries = delta
            .get("entries")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let total = entries.len();
        let keep = if total > JOURNAL_ROWS_PER_DELTA {
            JOURNAL_ROWS_PER_DELTA - 1
        } else {
            total
        };
        let mut row = |entry: Value| {
            self.unit_journal.push_back(serde_json::json!({
                "object": object.clone(),
                "rev": rev.clone(),
                "cause": cause.clone(),
                "at": at.clone(),
                "entry": entry,
            }));
        };
        for entry in entries.iter().take(keep) {
            row(entry.clone());
        }
        if total > keep {
            row(serde_json::json!({ "kind": "elided", "omitted": total - keep }));
        }
        while self.unit_journal.len() > JOURNAL_RING {
            self.unit_journal.pop_front();
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
            return self.resync(feed, now);
        };
        // A delta for a different subject than the held keyframe (a future
        // view-another's-sheet reuse of this feed) must never merge.
        if let (Some(held), Some(mine)) = (
            base_frame.get("target").and_then(Value::as_str),
            data.get("target").and_then(Value::as_str),
        ) && held != mine
        {
            return PanelOutcome::Ignored;
        }
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
                _ => self.resync(feed, now),
            },
            // A gap (base != cur), or a malformed frame: rehydrate, once.
            _ => self.resync(feed, now),
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
                self.resync(&name, now)
            }
        }
    }

    /// Whether a resync is outstanding for `feed`.
    #[must_use]
    pub fn resync_pending(&self, feed: &str) -> bool {
        self.resync_pending.contains_key(feed)
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

    /// Ask for one keyframe, at most once per [`RESYNC_LATCH_S`]. A latch
    /// younger than that answers `Ignored` (the storm guard); an older one —
    /// or a clock that went backwards — re-arms and asks again, so a query
    /// the bridge threw away silently costs one stale window, not the
    /// session.
    fn resync(&mut self, feed: &SmolStr, now: i64) -> PanelOutcome {
        if let Some(&armed) = self.resync_pending.get(feed) {
            let age = now.saturating_sub(armed);
            if (0..RESYNC_LATCH_S).contains(&age) {
                return PanelOutcome::Ignored;
            }
        }
        self.resync_pending.insert(feed.clone(), now);
        PanelOutcome::NeedsResync(feed.clone())
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

/// The `object.id` of a unit frame, if it carries one. A `null` frame (the
/// viewer holds no station) has none, and that is a held frame, not an error.
fn unit_object_id(frame: &Value) -> Option<String> {
    frame
        .get("object")
        .and_then(|o| o.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
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

    /// The I8 contract's F3 delta: `n` cell entries about `object`.
    fn unit_delta(object: &str, to_revision: u64, n: usize) -> Value {
        let entries: Vec<Value> = (0..n)
            .map(|i| {
                serde_json::json!({ "kind": "cell", "addr": format!("unit/c{i}"),
                    "from": 6, "to": 1 })
            })
            .collect();
        serde_json::json!({
            "object": { "id": object, "name": "Atlas" },
            "from_revision": to_revision - 1, "to_revision": to_revision,
            "now": 1_757_000_000, "cause": "gauss", "entries": entries
        })
    }

    fn unit_keyframe(object: &str, revision: u64) -> Value {
        serde_json::json!({
            "object": { "id": object, "name": "Atlas" }, "revision": revision,
            "state": "active", "record": null, "mobility": null,
            "sections": [], "cells": [], "components": [], "capabilities": [],
            "counts": { "sections": 0, "cells": 0, "components": 0, "capabilities": 0 },
            "truncated": false
        })
    }

    /// I8: a `tactics.unit` delta is kept BEFORE the resync is asked for, so
    /// the storm guard (which answers `Ignored` for the 2nd..nth delta while
    /// one keyframe request is outstanding) never costs the ring a row. A
    /// delta longer than `JOURNAL_ROWS_PER_DELTA` says how much it cut.
    #[test]
    fn a_tactics_unit_delta_rings_at_most_fifty_rows_and_asks_for_exactly_one_keyframe() {
        let mut st = PanelStore::new();
        let f = SmolStr::new(UNIT_FEED);
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 4, 3), 0),
            PanelOutcome::NeedsResync(f.clone())
        );
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 5, 2), 0),
            PanelOutcome::Ignored,
            "one keyframe request outstanding, no storm"
        );
        assert_eq!(
            st.unit_journal().len(),
            5,
            "both deltas ringed, coalesced or not"
        );
        let row = st.unit_journal().front().unwrap();
        assert_eq!(row["object"], serde_json::json!("#7"));
        assert_eq!(row["rev"], serde_json::json!(4));
        assert_eq!(row["cause"], serde_json::json!("gauss"));
        assert_eq!(row["at"], serde_json::json!(1_757_000_000u64));
        assert_eq!(row["entry"]["kind"], serde_json::json!("cell"));

        let mut st = PanelStore::new();
        st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 4, 1024), 0);
        assert_eq!(st.unit_journal().len(), JOURNAL_ROWS_PER_DELTA);
        let last = st.unit_journal().back().unwrap();
        assert_eq!(last["entry"]["kind"], serde_json::json!("elided"));
        assert_eq!(
            last["entry"]["omitted"],
            serde_json::json!(1024 - (JOURNAL_ROWS_PER_DELTA - 1))
        );
    }

    /// I8: a keyframe clears the resync latch; the ring is capped; a keyframe
    /// for another subject drops rows that no longer describe what is held,
    /// while the FIRST keyframe after a delta keeps them (the fan-out sends
    /// the delta first, and those rows are the after-action report); and a
    /// `null` keyframe — the viewer holds no station — is a held frame.
    #[test]
    fn a_keyframe_clears_the_latch_and_the_ring_caps_at_two_hundred() {
        let mut st = PanelStore::new();
        let f = SmolStr::new(UNIT_FEED);
        st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 4, 3), 0);
        assert!(st.resync_pending(UNIT_FEED));
        st.apply_panel(&f, PanelFrameKind::Keyframe, &unit_keyframe("#7", 4), 1);
        assert!(!st.resync_pending(UNIT_FEED));
        assert_eq!(
            st.unit_journal().len(),
            3,
            "the first keyframe keeps the rows that provoked it"
        );

        for rev in 5..40 {
            st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", rev, 10), 1);
        }
        assert_eq!(st.unit_journal().len(), JOURNAL_RING);
        let oldest = st.unit_journal().front().unwrap()["rev"].as_u64().unwrap();
        assert!(oldest > 5, "the ring dropped from the front: {oldest}");

        // A keyframe for another subject clears it; so does losing the unit.
        st.apply_panel(&f, PanelFrameKind::Keyframe, &unit_keyframe("#8", 1), 2);
        assert!(st.unit_journal().is_empty());
        st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#8", 2, 2), 2);
        assert_eq!(st.unit_journal().len(), 2);
        st.apply_panel(&f, PanelFrameKind::Keyframe, &Value::Null, 3);
        assert!(st.unit_journal().is_empty(), "the unit went away");
        assert_eq!(
            st.get(UNIT_FEED),
            Some(&Value::Null),
            "a null keyframe is a held frame, not an error"
        );
        assert!(!st.resync_pending(UNIT_FEED));

        // "Held" is the STORED frame, not its object id. On a fresh store
        // nothing is stored, so neither a `null` keyframe nor the first real
        // one throws the fan-out's rows away; once a frame IS stored, every
        // change of subject clears — including null → #5, which the old
        // `Option<String>` test could not see.
        let mut st = PanelStore::new();
        st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 4, 6), 0);
        assert_eq!(st.unit_journal().len(), 6);
        st.apply_panel(&f, PanelFrameKind::Keyframe, &Value::Null, 0);
        assert_eq!(
            st.unit_journal().len(),
            6,
            "nothing was stored: the honest null keeps the rows the panes show"
        );
        st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 5, 6), 0);
        st.apply_panel(&f, PanelFrameKind::Keyframe, &Value::Null, 0);
        assert_eq!(
            st.unit_journal().len(),
            12,
            "null over null is not a change of subject"
        );
        st.apply_panel(&f, PanelFrameKind::Keyframe, &unit_keyframe("#5", 1), 0);
        assert!(
            st.unit_journal().is_empty(),
            "a frame was held (the null) and the subject changed"
        );

        // The first keyframe after a delta on a truly fresh store keeps them.
        let mut st = PanelStore::new();
        st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 4, 6), 0);
        st.apply_panel(&f, PanelFrameKind::Keyframe, &unit_keyframe("#7", 4), 0);
        assert_eq!(st.unit_journal().len(), 6);
    }

    /// I8 (cross must_fix 2): the storm guard ages. A throttled SSH query is
    /// dropped with no reply of any kind, so a latch only a keyframe can
    /// clear would freeze the section map for the session while the journal
    /// kept filling. After `RESYNC_LATCH_S` the next delta asks again.
    #[test]
    #[allow(non_snake_case, reason = "the constant is named in the test's name")]
    fn a_stale_resync_latch_re_arms_after_RESYNC_LATCH_S() {
        let mut st = PanelStore::new();
        let f = SmolStr::new(UNIT_FEED);
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 4, 1), 0),
            PanelOutcome::NeedsResync(f.clone())
        );
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 5, 1), 1),
            PanelOutcome::Ignored,
            "still one request outstanding"
        );
        assert_eq!(
            st.apply_panel(
                &f,
                PanelFrameKind::Delta,
                &unit_delta("#7", 6, 1),
                RESYNC_LATCH_S - 1
            ),
            PanelOutcome::Ignored
        );
        assert_eq!(
            st.apply_panel(
                &f,
                PanelFrameKind::Delta,
                &unit_delta("#7", 7, 1),
                RESYNC_LATCH_S
            ),
            PanelOutcome::NeedsResync(f.clone()),
            "the reply never came: ask again"
        );
        assert_eq!(
            st.unit_journal().len(),
            4,
            "every delta rings, latched or not"
        );
        assert!(st.resync_pending(UNIT_FEED));
        st.apply_panel(
            &f,
            PanelFrameKind::Keyframe,
            &unit_keyframe("#7", 7),
            RESYNC_LATCH_S,
        );
        assert!(!st.resync_pending(UNIT_FEED), "a keyframe clears the latch");
        // A clock that jumped backwards re-arms rather than latching forever.
        let mut st = PanelStore::new();
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 4, 1), 100),
            PanelOutcome::NeedsResync(f.clone())
        );
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &unit_delta("#7", 5, 1), 20),
            PanelOutcome::NeedsResync(f.clone())
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
