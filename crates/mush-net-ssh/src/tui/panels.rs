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
/// **The arena cockpit** (doc 16 §5; 16b cut Q.3 named it): the per-pulse
/// keyframe a seated pilot is pushed, and the one feed the cockpit screen
/// reads. `seated: false` is the last frame a pilot gets, and it clears the
/// screen.
pub const COCKPIT_FEED: &str = "arena.cockpit";
/// **The arena's terrain** (16b cut Q.3): the whole ground, PULLED and never
/// pushed, which the map pane composes the cockpit frame's markers onto. A
/// resumed client holds none and must re-ask (see
/// [`super::app::MAP_HEAL_LATCH_S`]).
pub const ARENA_MAP_FEED: &str = "arena.map";
/// How long a resync request stays latched before another delta may ask
/// again. The SSH bridge drops a throttled query silently (no reply, not
/// even an error), so a latch cleared only by the replacement keyframe is
/// permanent for the session once a flood bucket is hit. Five whole seconds
/// is well outside a healthy round trip and still coalesces a damage burst.
pub const RESYNC_LATCH_S: i64 = 5;
/// **The most lines the cockpit's notices ring keeps** (16b cut B.3, `B-64`):
/// the web's `COCKPIT_NOTICES`, the same number so the two panes read alike.
pub const COCKPIT_NOTICES: usize = 50;
/// The most characters one reply line may spend on the notices ring (16b cut
/// B.3). A pane row wraps now, so this is not a width: it bounds what one
/// server line can cost the ring, the way every other TUI renderer bounds the
/// server text it keeps.
pub const COCKPIT_REPLY_CHARS: usize = 200;
/// **The span tag on the arena bridge's `Sys` copy of a frame's own notice**
/// (`tacsim::arena_bridge::NOTICE_SENTENCE_TAG`, 16b cut B.1). The ring composes
/// that notice from the frame's FIELDS, so ringing the sentence as a reply too
/// would say every notice twice; a line whose first span carries this tag is the
/// one `Sys` line [`PanelStore::push_cockpit_reply`]'s caller leaves out. This
/// crate takes no `tacsim` dependency, so the word is copied here and pinned to
/// the web's copy by a test.
pub const NOTICE_SENTENCE_TAG: &str = "arena-notice";

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
    /// **The cockpit's notices ring** (16b cut B.3, `B-64`), oldest first, at
    /// most [`COCKPIT_NOTICES`]: each seated frame's notices composed by
    /// [`super::render::notice_words`] from their fields, and — while a seated
    /// frame is held — every `Sys` line the pilot is sent, because on screen 0
    /// this pane is the only reply surface there is.
    cockpit_notices: VecDeque<String>,
    /// The match the ring's lines belong to (16b cut B.3, `B-80`'s reset on
    /// the TUI): a frame of another match empties it before its own lines land.
    cockpit_notices_for: Option<i64>,
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

    /// **Drop a feed, whole** (16b cut Q.4; 16b cut B.3, `T-64`): the arena map
    /// of a match that is over describes ground the pilot is no longer on, and
    /// a stale map under live markers is worse than no map — the pane says it
    /// is waiting and the self-heal asks for the new one.
    ///
    /// Whole means the three maps a feed lives in: its held frame (`feeds`),
    /// a view's gap cursor (`view_cursors`) and an outstanding resync latch
    /// (`resync_pending`). Dropping only the frame left a cursor that would
    /// judge the next keyframe against a stream that no longer exists, and a
    /// latch that would swallow the next resync for up to [`RESYNC_LATCH_S`].
    pub fn forget(&mut self, feed: &str) {
        self.feeds.remove(feed);
        self.view_cursors.remove(feed);
        self.resync_pending.remove(feed);
    }

    /// **Ring one seated `arena.cockpit` frame's notices** (16b cut B.3,
    /// `B-64`, Q20). A frame of another match than the ring's empties it
    /// first (`B-80`'s reset: match 7's last lines never sit above match 8's
    /// first); then each entry of `notices` is composed by
    /// [`super::render::notice_words`] — from its fields, never its `text` —
    /// and pushed when it says anything, the front dropped past
    /// [`COCKPIT_NOTICES`]. Per-pulse frames carry only that pulse's notices,
    /// and the copy a read verb re-emits carries `notices: []`, so nothing is
    /// rung twice.
    ///
    /// **16b cut Y.4**: a frame's `catchup` object — a reclaim's first frame
    /// carries one, every other `null` — rings first, one line each of
    /// [`super::render::catchup_words`], under the frame's own callsign
    /// (cleaned, cut to eight). A `catchup` that is not an object rings
    /// nothing.
    pub fn push_cockpit_notices(&mut self, frame: &Value) {
        let match_id = frame.get("match").and_then(Value::as_i64);
        if match_id != self.cockpit_notices_for {
            self.cockpit_notices.clear();
            self.cockpit_notices_for = match_id;
        }
        // **16b cut Y.4** (doc 07 §1.3 step 5): the catch-up report a reclaim's
        // first frame carries, before that frame's notices — it is about the
        // time before them. Composed from the object's fields; the line
        // client's copy is tagged as a frame's own sentence and not rung.
        if let Some(catchup) = frame.get("catchup").filter(|v| v.is_object()) {
            let callsign = frame
                .get("callsign")
                .and_then(Value::as_str)
                .map(|raw| super::render::truncate_chars(super::render::clean(raw).trim(), 8))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "?".to_string());
            for line in super::render::catchup_words(&callsign, catchup) {
                self.ring_cockpit_line(line);
            }
        }
        let Some(notices) = frame.get("notices").and_then(Value::as_array) else {
            return;
        };
        for notice in notices {
            let line = super::render::notice_words(notice);
            if !line.is_empty() {
                self.ring_cockpit_line(line);
            }
        }
    }

    /// **Ring one reply** (16b cut B.3, the TUI half of B.1's `pushReply`): the
    /// plain text of a `Sys` line, while a SEATED `arena.cockpit` frame is held,
    /// and nothing otherwise. On screen 0 this ring is the only reply surface —
    /// a pilot who typed `lock BA` there expects `Locking BA.` on the pane — and
    /// the scrollback on screen 1 keeps its own copy of the sentence (Q20).
    ///
    /// A reply of several lines (`help cockpit` answers its whole topic as one
    /// `Sys`) rings as several: it is split at its line breaks BEFORE the
    /// control characters are cleaned out, so two lines never run together —
    /// the web's pane keeps the break too (`white-space: pre`). Each line is
    /// cleaned, a blank one is skipped, and the reply as a whole spends at most
    /// [`COCKPIT_REPLY_CHARS`]; the caller leaves out the one `Sys` line tagged
    /// [`NOTICE_SENTENCE_TAG`].
    pub fn push_cockpit_reply(&mut self, text: &str) {
        let seated = self
            .get(COCKPIT_FEED)
            .is_some_and(|frame| frame.get("seated").and_then(Value::as_bool) != Some(false));
        if !seated {
            return;
        }
        let mut budget = COCKPIT_REPLY_CHARS;
        for raw in text.lines() {
            if budget == 0 {
                break;
            }
            let line: String = super::render::clean(raw).chars().take(budget).collect();
            if line.trim().is_empty() {
                continue;
            }
            budget = budget.saturating_sub(line.chars().count());
            self.ring_cockpit_line(line);
        }
    }

    /// Empty the cockpit's notices ring and forget its match (16b cut B.3): the
    /// cleared frame, the last one a pilot gets, leaves nothing of the seat.
    pub fn clear_cockpit_notices(&mut self) {
        self.cockpit_notices.clear();
        self.cockpit_notices_for = None;
    }

    /// The cockpit's notices ring, oldest first (16b cut B.3).
    #[must_use]
    pub fn cockpit_notices(&self) -> &VecDeque<String> {
        &self.cockpit_notices
    }

    fn ring_cockpit_line(&mut self, line: String) {
        self.cockpit_notices.push_back(line);
        while self.cockpit_notices.len() > COCKPIT_NOTICES {
            self.cockpit_notices.pop_front();
        }
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

    /// **16b cut B.3** (`T-64`): `forget` drops a feed from all three maps it
    /// lives in — the frame, a view's gap cursor and the resync latch — so a
    /// forgotten feed starts again exactly as one never seen.
    #[test]
    fn forget_forgets_the_cursor_and_the_latch() {
        let mut st = PanelStore::new();
        // A view's cursor: after seq 6, a keyframe at seq 2 is stale…
        let data = serde_json::json!([{ "presentation": { "verb": "brace" } }]);
        let fresh = ViewEnvelope::keyframe("legal.actions", 1, 7, 6, data.clone()).unwrap();
        let older = ViewEnvelope::keyframe("legal.actions", 1, 7, 2, data.clone()).unwrap();
        assert_eq!(st.apply_view(&fresh, 0), PanelOutcome::Stored);
        assert_eq!(st.apply_view(&older, 0), PanelOutcome::Ignored);
        // …and once forgotten there is no cursor to judge it against.
        st.forget("legal.actions");
        assert!(st.get("legal.actions").is_none());
        assert_eq!(st.apply_view(&older, 0), PanelOutcome::Stored);

        // A resync latch: the second delta inside the latch asks nothing…
        let f = SmolStr::new("intel");
        let delta = serde_json::json!({});
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &delta, 10),
            PanelOutcome::NeedsResync(f.clone())
        );
        assert!(st.resync_pending("intel"));
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &delta, 11),
            PanelOutcome::Ignored
        );
        // …and a forgotten feed asks at once, inside the same second.
        st.forget("intel");
        assert!(!st.resync_pending("intel"));
        assert_eq!(
            st.apply_panel(&f, PanelFrameKind::Delta, &delta, 11),
            PanelOutcome::NeedsResync(f)
        );
    }

    fn seated_frame(match_id: i64, notices: Value) -> Value {
        serde_json::json!({ "match": match_id, "notices": notices })
    }

    fn acquired(designator: &str) -> Value {
        serde_json::json!({ "kind": "acquired", "designator": designator, "friendly": false })
    }

    /// **16b cut B.3** (`B-64`, `B-80`): the ring keeps one match's lines, the
    /// newest [`COCKPIT_NOTICES`], and takes a reply only while seated.
    #[test]
    fn the_cockpit_ring_is_one_match_capped_and_replies_only_while_seated() {
        let mut st = PanelStore::new();
        st.push_cockpit_reply("Locking BA.");
        assert!(
            st.cockpit_notices().is_empty(),
            "no frame, no seat, no ring"
        );
        let feed = SmolStr::new_static(COCKPIT_FEED);
        let frame = seated_frame(7, serde_json::json!([acquired("X4")]));
        st.apply_panel(&feed, PanelFrameKind::Keyframe, &frame, 0);
        st.push_cockpit_notices(&frame);
        st.push_cockpit_reply("Locking \u{1b}[31mBA.");
        assert_eq!(
            st.cockpit_notices().iter().cloned().collect::<Vec<_>>(),
            ["X4 new contact", "Locking [31mBA."],
            "composed from fields, and the reply cleaned of its control byte"
        );
        for i in 0..COCKPIT_NOTICES {
            st.push_cockpit_reply(&format!("reply {i}"));
        }
        assert_eq!(st.cockpit_notices().len(), COCKPIT_NOTICES);
        assert_eq!(
            st.cockpit_notices().front().map(String::as_str),
            Some("reply 0"),
            "the oldest lines go first"
        );
        st.push_cockpit_reply(&"z".repeat(COCKPIT_REPLY_CHARS + 40));
        assert_eq!(
            st.cockpit_notices().back().map(|l| l.chars().count()),
            Some(COCKPIT_REPLY_CHARS)
        );
        // A frame of another match starts the ring again before its own lines.
        let next = seated_frame(8, serde_json::json!([acquired("X1")]));
        st.push_cockpit_notices(&next);
        assert_eq!(
            st.cockpit_notices().iter().cloned().collect::<Vec<_>>(),
            ["X1 new contact"]
        );
        // The cleared frame: nothing rings after it, and the ring is empty.
        let cleared = serde_json::json!({ "seated": false, "match": null, "notices": [] });
        st.apply_panel(&feed, PanelFrameKind::Keyframe, &cleared, 1);
        st.clear_cockpit_notices();
        st.push_cockpit_reply("You climb out.");
        assert!(st.cockpit_notices().is_empty());
    }

    /// **16b cut B.3**: a reply of several lines rings line by line — never
    /// glued into one run-on line by the control-character clean — blank lines
    /// are skipped, and the reply as a whole is bounded.
    #[test]
    fn a_reply_of_several_lines_rings_line_by_line() {
        let mut st = PanelStore::new();
        let feed = SmolStr::new_static(COCKPIT_FEED);
        let frame = seated_frame(7, serde_json::json!([]));
        st.apply_panel(&feed, PanelFrameKind::Keyframe, &frame, 0);
        st.push_cockpit_notices(&frame);
        st.push_cockpit_reply("COCKPIT\n\nThe end of a line.\r\nStart of the next.");
        assert_eq!(
            st.cockpit_notices().iter().cloned().collect::<Vec<_>>(),
            ["COCKPIT", "The end of a line.", "Start of the next."]
        );
        // The whole reply spends at most COCKPIT_REPLY_CHARS, however many
        // lines it is cut into.
        let long = vec!["y".repeat(150); 4].join("\n");
        st.push_cockpit_reply(&long);
        let spent: Vec<usize> = st
            .cockpit_notices()
            .iter()
            .skip(3)
            .map(|l| l.chars().count())
            .collect();
        assert_eq!(spent, [150, COCKPIT_REPLY_CHARS - 150]);
    }

    /// The span tag the ring's caller skips is the bridge's, and the web reads
    /// the same word (16b cut B.1's `NOTICE_SENTENCE_TAG` in `cockpit.ts`).
    ///
    /// All three copies are pinned here, the bridge's included: this crate
    /// takes no `tacsim` dependency, so the bridge's source is read instead. A
    /// renamed tag on the bridge alone would have both clients ring every
    /// per-pulse notice twice — once composed, once as a reply.
    #[test]
    fn the_notice_sentence_tag_is_the_web_s() {
        let web = include_str!("../../../../web/src/lib/cockpit.ts");
        assert!(
            web.contains(&format!(
                "export const NOTICE_SENTENCE_TAG = \"{NOTICE_SENTENCE_TAG}\";"
            )),
            "the TUI's and the web's notice-sentence tags have drifted"
        );
        let bridge = include_str!("../../../tacsim/src/arena_bridge.rs");
        assert!(
            bridge.contains(&format!(
                "pub const NOTICE_SENTENCE_TAG: &str = \"{NOTICE_SENTENCE_TAG}\";"
            )),
            "the TUI's and the arena bridge's notice-sentence tags have drifted"
        );
    }
}
