//! Read-only `Req` hydration for `+intel` (M15): the `"intel.list"` query
//! handler, registered via
//! [`Registrar::query_handler`](mush_core::Registrar::query_handler) and
//! invoked by the engine's `Inbound::Query` handling. It backs the SSH TUI's
//! +intel widget (and any future web panel) with the SAME data the `+intel`
//! command shows.
//!
//! **Strictly principal-scoped (no cross-actor leak, invariant T3 / M12 P5).**
//! The querying `actor` is the principal: the handler reads ONLY
//! [`IntelStore::for_principal`]`(actor)`, exactly like [`cmd_intel`], so one
//! player can never pull another principal's informant reports. Reading intel
//! does not mutate, so unlike `@mail` read this stays a query.
//!
//! [`cmd_intel`]: crate::module

use mush_core::{ObjId, QueryHandler, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use smol_str::SmolStr;

use crate::intel::IntelRecord;
use crate::module::intel_kind_label;
use crate::runtime::NpcHostState;

fn err(msg: impl Into<String>) -> RichText {
    RichText::tag("error", msg)
}

/// `"intel.list"` — the +intel panel keyframe: the querying actor's own
/// informant intel, **newest first** (the same order and fields `+intel`
/// prints).
///
/// Params: none (the whole log for the principal). Filtering by subject stays
/// in the `+intel <subject>` command, which resolves world objects; the panel
/// pulls the full log and lets the client narrow.
///
/// Reply payload: `Panel { feed: "intel", kind: Keyframe, data: {
///   "count": <n>,
///   "records": [ { subject, kind, room, detail, provenance, ts } ]  // newest first
/// } }`.
pub struct IntelListQuery;

impl QueryHandler for IntelListQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let st = w
            .module_state::<NpcHostState>()
            .ok_or_else(|| err("NPC agents are not loaded."))?;
        // Principal scope: ONLY the querying actor's own bucket. No parameter
        // can widen this to another principal (no cross-actor leak).
        let records = st.intel.for_principal(actor);
        let items: Vec<serde_json::Value> = records
            .iter()
            .rev() // newest first, matching `+intel`
            .map(|r: &IntelRecord| {
                serde_json::json!({
                    "subject": r.subject_name,
                    "kind": intel_kind_label(r.kind),
                    "room": r.room_name,
                    "detail": r.detail,
                    "provenance": r.provenance,
                    "ts": r.ts,
                })
            })
            .collect();

        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static("intel"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "count": records.len(),
                "records": items,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::{IntelKind, IntelRecord};
    use crate::runtime::NpcHostState;
    use mush_core::World;

    /// A world with an `NpcHostState` whose intel store holds `records`.
    fn world_with_intel(records: Vec<IntelRecord>) -> World {
        let mut w = World::new_minimal();
        let mut host = NpcHostState::headless();
        for r in records {
            host.intel.file(r);
        }
        w.module_state_insert(host);
        w
    }

    fn rec(principal: u32, subject: u32, ts: i64, name: &str) -> IntelRecord {
        IntelRecord {
            informant: ObjId(9),
            principal: ObjId(principal),
            subject: ObjId(subject),
            subject_name: name.to_string(),
            kind: IntelKind::Spoke,
            detail: format!("said something at {ts}"),
            room_name: "The Bar".to_string(),
            provenance: "witnessed directly by Snitch".to_string(),
            ts,
        }
    }

    fn records_of(ev: &OutputEvent) -> Vec<serde_json::Value> {
        match ev {
            OutputEvent::Panel { feed, data, .. } => {
                assert_eq!(feed.as_str(), "intel");
                data.get("records")
                    .and_then(|r| r.as_array())
                    .cloned()
                    .unwrap_or_default()
            }
            other => panic!("expected Panel, got {other:?}"),
        }
    }

    #[test]
    fn lists_only_the_querying_principals_records() {
        // Principal #1 has two records; principal #2 has one. Actor #1 must
        // see exactly its own two and never #2's (no cross-principal leak).
        let w = world_with_intel(vec![
            rec(1, 2, 100, "Mallory"),
            rec(2, 2, 150, "SecretGuy"),
            rec(1, 3, 200, "Trent"),
        ]);
        let ev = IntelListQuery
            .query(&w, ObjId(1), &serde_json::json!({}))
            .expect("intel.list ok");
        let items = records_of(&ev);
        assert_eq!(items.len(), 2, "only principal #1's two records");
        let subjects: Vec<&str> = items
            .iter()
            .filter_map(|r| r.get("subject").and_then(|s| s.as_str()))
            .collect();
        assert!(subjects.contains(&"Mallory"));
        assert!(subjects.contains(&"Trent"));
        assert!(
            !subjects.contains(&"SecretGuy"),
            "principal #2's subject must NOT leak to actor #1"
        );
    }

    #[test]
    fn records_are_newest_first() {
        // Filed in witness order (as the informant loop does); the panel
        // reverses that so the most-recently-filed shows first, exactly like
        // the `+intel` command's `.rev()`.
        let w = world_with_intel(vec![
            rec(1, 2, 100, "Old"),
            rec(1, 2, 200, "Mid"),
            rec(1, 2, 300, "New"),
        ]);
        let ev = IntelListQuery
            .query(&w, ObjId(1), &serde_json::json!({}))
            .expect("ok");
        let items = records_of(&ev);
        let ts: Vec<i64> = items
            .iter()
            .filter_map(|r| r.get("ts").and_then(serde_json::Value::as_i64))
            .collect();
        assert_eq!(ts, vec![300, 200, 100], "most-recently-filed first");
    }

    #[test]
    fn empty_for_a_principal_with_no_intel() {
        let w = world_with_intel(vec![rec(1, 2, 100, "Mallory")]);
        // Actor #7 has filed nothing: an empty (not error) panel.
        let ev = IntelListQuery
            .query(&w, ObjId(7), &serde_json::json!({}))
            .expect("ok even when empty");
        assert!(records_of(&ev).is_empty());
    }

    #[test]
    fn carries_kind_label_and_provenance() {
        let w = world_with_intel(vec![rec(1, 2, 100, "Mallory")]);
        let ev = IntelListQuery
            .query(&w, ObjId(1), &serde_json::json!({}))
            .expect("ok");
        let items = records_of(&ev);
        let r = &items[0];
        assert_eq!(r.get("kind").and_then(|k| k.as_str()), Some("spoke"));
        assert_eq!(r.get("room").and_then(|k| k.as_str()), Some("The Bar"));
        assert!(
            r.get("provenance")
                .and_then(|p| p.as_str())
                .is_some_and(|p| p.contains("Snitch"))
        );
    }
}
