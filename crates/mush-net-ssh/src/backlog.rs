//! The `narrative.backlog` page wire shape (interface plan I12).
//!
//! The gateway answers a `narrative.backlog` query with
//! `Reply{Panel{feed: "narrative", kind: Delta, data}}`, where `data` is
//! `{"backlog": {"before_seq": …, "more": …, "evicted": …, "lines": […]}}`
//! and each line is `{seq, ts, kind, actor, text}`
//! (`mush-gateway/src/bridge_common.rs`, `narrative_backlog_reply`). Both
//! client halves read it through this one module — the TUI's scrollback
//! prepend and line mode's `@history` — so the shape is pinned in a single
//! place and one round-trip test covers the pair.
//!
//! The page is **newest first**: the session ring is walked in reverse
//! (`mush-session/src/manager.rs`, `backlog_matching`), so `lines.last()` is
//! the oldest line and its `seq` is the next page's `before_seq`.
//!
//! Parsing is hand-rolled over [`serde_json::Value`] rather than derived:
//! this crate depends on `serde_json` but not on `serde` itself, and a page
//! is five fields. The behaviour matches a `#[serde(default)]` derive —
//! unknown fields are ignored (the gateway is free to add some), an absent
//! `more`/`evicted`/`ts` reads as its default — with one deliberate
//! strictness: a line that is not a well-formed narrative rejects the WHOLE
//! page. Both halves read `None` as "stop asking" rather than guessing.

use mush_proto::{NarrativeKind, ObjRef, RichText};
use serde_json::Value;

/// One narrative line of a backlog page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BacklogLine {
    /// The session ring's sequence number — the paging cursor.
    pub(crate) seq: u64,
    /// When the line was delivered, Unix **milliseconds** (`Envelope.ts`).
    /// Scrollback entries stamp Unix *seconds*, so dividing is not optional.
    pub(crate) ts: i64,
    /// Say / pose / emit / …, exactly as the live event carried it.
    pub(crate) kind: NarrativeKind,
    /// The speaker; `null` for an unattributed emit.
    pub(crate) actor: Option<ObjRef>,
    /// The line itself.
    pub(crate) text: RichText,
}

/// One page of narrative backlog, newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BacklogBody {
    /// The server holds further lines below this page.
    pub(crate) more: bool,
    /// Older history has rolled off the session ring.
    pub(crate) evicted: bool,
    /// The lines, newest first.
    pub(crate) lines: Vec<BacklogLine>,
}

impl BacklogBody {
    /// The oldest line's seq — the `before_seq` of the next page.
    pub(crate) fn oldest_seq(&self) -> Option<u64> {
        self.lines.last().map(|l| l.seq)
    }
}

/// Read the `data` of a `narrative.backlog` reply. `None` for anything that
/// is not a well-formed page.
pub(crate) fn parse(data: &Value) -> Option<BacklogBody> {
    let body = data.get("backlog")?;
    let lines = match body.get("lines") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().map(parse_line).collect::<Option<Vec<_>>>()?,
        Some(_) => return None,
    };
    Some(BacklogBody {
        more: body.get("more").and_then(Value::as_bool).unwrap_or(false),
        evicted: body
            .get("evicted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        lines,
    })
}

fn parse_line(v: &Value) -> Option<BacklogLine> {
    Some(BacklogLine {
        seq: v.get("seq")?.as_u64()?,
        ts: v.get("ts").and_then(Value::as_i64).unwrap_or(0),
        kind: serde_json::from_value(v.get("kind")?.clone()).ok()?,
        actor: match v.get("actor") {
            None | Some(Value::Null) => None,
            Some(a) => Some(serde_json::from_value(a.clone()).ok()?),
        },
        text: serde_json::from_value(v.get("text")?.clone()).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::Span;

    /// A literal copy of what `narrative_backlog_reply` builds, with every
    /// [`NarrativeKind`], a `null` actor, a tagged span and an unknown extra
    /// field. Nothing shares a type with the gateway, so this test is the
    /// only thing pinning the two crates' agreement on the shape.
    #[test]
    fn the_gateway_page_round_trips_including_a_null_actor_and_every_kind() {
        let kinds = [
            ("say", NarrativeKind::Say),
            ("pose", NarrativeKind::Pose),
            ("semi_pose", NarrativeKind::SemiPose),
            ("emit", NarrativeKind::Emit),
            ("page", NarrativeKind::Page),
            ("ooc", NarrativeKind::Ooc),
            ("spoof", NarrativeKind::Spoof),
            ("whisper", NarrativeKind::Whisper),
        ];
        let lines: Vec<Value> = kinds
            .iter()
            .enumerate()
            .map(|(i, (wire, _))| {
                let seq = 40 - i as u64;
                serde_json::json!({
                    "seq": seq,
                    "ts": 1_756_300_000_123i64,
                    "kind": wire,
                    // An unattributed emit really does ride with `actor: null`.
                    "actor": if i % 2 == 0 {
                        serde_json::json!({ "id": "#5", "name": "Mireille" })
                    } else {
                        Value::Null
                    },
                    "text": [ { "text": "the rain again" }, { "text": " north", "tag": "exit" } ],
                    // The gateway is free to grow the line; we must not care.
                    "unknown_future_field": 7,
                })
            })
            .collect();
        let data = serde_json::json!({
            "backlog": { "before_seq": Value::Null, "more": true, "evicted": false, "lines": lines }
        });

        let page = parse(&data).expect("the gateway's own page parses");
        assert!(page.more && !page.evicted);
        assert_eq!(page.lines.len(), 8);
        assert_eq!(page.oldest_seq(), Some(33), "newest first: last is oldest");
        for (i, (_, kind)) in kinds.iter().enumerate() {
            let line = &page.lines[i];
            assert_eq!(line.kind, *kind);
            assert_eq!(line.seq, 40 - i as u64);
            assert_eq!(line.ts, 1_756_300_000_123);
            assert_eq!(line.actor.is_some(), i % 2 == 0);
            assert_eq!(
                line.text,
                RichText(vec![
                    Span::plain("the rain again"),
                    Span::tagged("exit", " north"),
                ])
            );
        }
    }

    /// The empty page a reaped session yields (`backlog_matching` errors,
    /// the bridge `unwrap_or_default`s) is a valid terminal page, not an
    /// error — and `more`/`evicted` default when absent.
    #[test]
    fn an_empty_page_is_terminal_and_missing_flags_default() {
        let page = parse(&serde_json::json!({ "backlog": { "lines": [] } })).expect("valid");
        assert_eq!(page, BacklogBody::default());
        assert_eq!(page.oldest_seq(), None);
        let page = parse(&serde_json::json!({ "backlog": {} })).expect("no lines key at all");
        assert!(page.lines.is_empty());
    }

    /// A page whose shape is wrong anywhere is rejected whole: both halves
    /// stop the backlog rather than paint half a page or loop on a cursor
    /// they could not advance.
    #[test]
    fn a_malformed_page_is_rejected_whole() {
        for data in [
            serde_json::json!({}),
            serde_json::json!({ "backlog": { "lines": "nope" } }),
            serde_json::json!({ "backlog": { "lines": [ { "seq": "x" } ] } }),
            // A good line beside a bad one: still no page.
            serde_json::json!({ "backlog": { "lines": [
                { "seq": 9, "kind": "say", "text": [] },
                { "seq": 8, "kind": "not_a_kind", "text": [] },
            ] } }),
            serde_json::json!({ "backlog": { "lines": [ { "seq": 9, "kind": "say" } ] } }),
            serde_json::json!({ "backlog": { "lines": [
                { "seq": 9, "kind": "say", "text": [], "actor": 3 },
            ] } }),
        ] {
            assert_eq!(parse(&data), None, "{data}");
        }
    }
}
