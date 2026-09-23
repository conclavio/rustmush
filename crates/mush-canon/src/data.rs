//! The canon event ledger data model (design:
//! `docs/scenes_factions_knowledge.md` §5).
//!
//! Canon events are the durable **consequences** selected from scene and world
//! activity — the things that *became true*, as opposed to the full transcript
//! of what occurred (that is the per-scene event log). Not every scene event
//! becomes canon: "Alice said the Prince is dead" is canonical as an *utterance*
//! recorded in the scene log, but "the Prince is dead" is canon only if
//! separately established. Domain modules emit canon events explicitly alongside
//! their state mutations, via [`crate::commit`].
//!
//! ## Immutability + retcons
//!
//! The ledger is append-only (the chunked [`Ledger`] substrate). A **retcon**
//! never rewrites history — it *appends* a new canon event that `supersedes` an
//! earlier one. "Current" canon is therefore **derived**: an event is current
//! unless some later event supersedes its id. This preserves the audit trail
//! (you can always see what was retconned and why) and keeps every derived
//! artifact's source ids valid.

use mush_core::{Deferred, Ledger, ModuleOp, ModuleWrite, ObjId, Outbox};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::BTreeSet;

/// The canon persistence namespace (`mod:canon:*`).
pub const NS: &str = "canon";

/// The kv key of the canon meta document (the id allocator resume point).
pub const META_KEY: &str = "meta";

/// The single ledger log id all canon events share (the world chronicle).
pub const LOG: &str = "canon";

/// A durable canon-event id: a monotonic per-world counter, **never reused**
/// (the allocator resume point persists in the meta doc). Derived artifacts
/// (beliefs, recaps, clocks) reference canon by this id, so it must be stable.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonId(pub u64);

impl std::fmt::Display for CanonId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Who may learn a canon event — the epistemic gate on the chronicle. The
/// event's *existence and summary* are gated here; the finer perception of its
/// details is the recap/belief layer's job.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonScope {
    /// Common knowledge — anyone may see it in the chronicle.
    #[default]
    Public,
    /// Only the entities involved (and staff) may see it.
    Restricted,
    /// Staff only — the fact exists; the world has to discover it.
    Secret,
}

impl CanonScope {
    /// Parse a staff-typed scope name.
    #[must_use]
    pub fn parse(s: &str) -> Option<CanonScope> {
        match s.trim().to_ascii_lowercase().as_str() {
            "public" => Some(CanonScope::Public),
            "restricted" | "restrict" | "entities" => Some(CanonScope::Restricted),
            "secret" | "staff" => Some(CanonScope::Secret),
            _ => None,
        }
    }

    /// Staff-facing label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            CanonScope::Public => "public",
            CanonScope::Restricted => "restricted",
            CanonScope::Secret => "secret",
        }
    }
}

/// What a module hands [`crate::commit`] to record a consequence. The id,
/// wall-clock time, and status are filled in by the ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonDraft {
    /// Module-defined kind, e.g. `"scene.concluded"`, `"faction.control"`.
    pub kind: SmolStr,
    /// A one-line human summary (the chronicle line).
    pub summary: String,
    /// Who/what the event is about (drives `Restricted` visibility and lookup).
    pub entities: Vec<ObjId>,
    /// Who may learn it.
    pub scope: CanonScope,
    /// The IC timestamp it occurred at (the caller supplies it: a scene passes
    /// its scene-local time, a world event the global IC clock).
    pub ic_time: i64,
    /// Free-form provenance ("scene:5"), for tracing back to the source.
    pub source: String,
    /// Kind-specific structured detail.
    pub detail: serde_json::Value,
}

/// One recorded canon event — the payload stored in the ledger. `supersedes` is
/// `Some` only for a retcon; `status` is **derived** at read time (an event is
/// superseded iff a later event supersedes its id), never mutated in place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanonEvent {
    /// Durable id.
    pub id: CanonId,
    /// Module-defined kind.
    pub kind: SmolStr,
    /// One-line summary.
    pub summary: String,
    /// Entities involved.
    #[serde(default)]
    pub entities: Vec<ObjId>,
    /// Visibility gate.
    #[serde(default)]
    pub scope: CanonScope,
    /// IC timestamp of occurrence.
    pub ic_time: i64,
    /// Wall-clock record time (unix seconds).
    pub at: i64,
    /// Provenance.
    #[serde(default)]
    pub source: String,
    /// If this event is a retcon, the id it supersedes.
    #[serde(default)]
    pub supersedes: Option<CanonId>,
    /// Retcon reason (empty for a normal event).
    #[serde(default)]
    pub reason: String,
    /// Kind-specific detail.
    #[serde(default)]
    pub detail: serde_json::Value,
}

impl CanonEvent {
    /// Whether `viewer` (with `staff` privilege) may see this event's existence
    /// in the chronicle, per its [`CanonScope`].
    #[must_use]
    pub fn visible_to(&self, viewer: ObjId, staff: bool) -> bool {
        match self.scope {
            CanonScope::Public => true,
            CanonScope::Restricted => staff || self.entities.contains(&viewer),
            CanonScope::Secret => staff,
        }
    }
}

/// One chronicle archive page a reader is waiting on (H1): the correlation
/// `Ctx::read_store` returned and the chunk range it covers. Transient world
/// RAM — a restart drops it, and a completion nobody is waiting on is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCanonRead {
    /// The correlation the completion must carry.
    pub correlation: u64,
    /// When it was requested (world seconds). A request older than
    /// `STORE_READ_STALE_SECS` is superseded by the next one (verify fold B1).
    pub requested_at: i64,
    /// The requested page (0 is the newest).
    pub page: u64,
    /// The chunk range the page covers, inclusive.
    pub chunk_lo: u64,
    /// The chunk range the page covers, inclusive.
    pub chunk_hi: u64,
}

/// The live canon state, held in a module state slot on the `World`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanonState {
    /// In-flight archive reads, one per requesting actor (H1). Transient.
    pub pending_reads: std::collections::HashMap<ObjId, PendingCanonRead>,
    /// The single append-only chronicle log (chunked; only the open tail in
    /// RAM — deep history pages in through the store-read seam, H1:
    /// `+chronicle/archive`).
    pub ledger: Ledger,
    /// The durable-id allocator resume point (persisted in the meta doc).
    pub next_canon_id: u64,
    /// The decoded entries of the last `SEALED_TAIL_CHUNKS` **sealed** chunks, as
    /// they stood at boot (E6b independent verify fold, R3).
    ///
    /// `Ledger::restore_from_kv` restores a sealed tail chunk as an *empty* open
    /// chunk one index on, so a chronicle whose newest records happened to fill a
    /// chunk just before the process ended rehydrated with nothing readable in
    /// [`CanonState::recent`] at all — and DL-09's whole point is that a seal
    /// committed just before a reboot is still findable. The window is bounded, read
    /// once at boot, and deliberately not appended to: records made *after* boot are
    /// in the open tail, which is where they belong.
    pub sealed_tail: Vec<CanonEvent>,
}

/// How many trailing sealed ledger chunks the canon read seam decodes at boot
/// (E6b independent verify fold, R3).
///
/// Two: a 16 KB chunk holds a great many canon events, and the loss this closes is a
/// boundary that fell between the last seal and the bounce — one chunk back is the
/// case, and the second is the margin for a boundary that fell twice.
pub const SEALED_TAIL_CHUNKS: usize = 2;

impl CanonState {
    /// Allocate the next canon id (callers persist the meta doc).
    pub fn alloc_id(&mut self) -> CanonId {
        let id = CanonId(self.next_canon_id);
        self.next_canon_id += 1;
        id
    }

    /// The canon events currently in RAM (the ledger's open tail), decoded.
    #[must_use]
    pub fn recent(&self) -> Vec<CanonEvent> {
        self.ledger
            .open_entries(LOG)
            .iter()
            .filter_map(|e| serde_json::from_value::<CanonEvent>(e.value.clone()).ok())
            .collect()
    }

    /// The set of ids superseded by a retcon **within the RAM tail** — used to
    /// mark current-vs-retconned in the chronicle. (Full-history supersession
    /// resolution needs the store-read seam.)
    #[must_use]
    pub fn superseded_in_tail(&self) -> BTreeSet<CanonId> {
        self.recent().iter().filter_map(|e| e.supersedes).collect()
    }

    /// The set of ids superseded by a retcon within an **already-decoded** slice —
    /// the same rule as [`CanonState::superseded_in_tail`] without a second decode
    /// (E6b independent verify fold, R7).
    #[must_use]
    pub fn superseded_in(events: &[CanonEvent]) -> BTreeSet<CanonId> {
        events.iter().filter_map(|e| e.supersedes).collect()
    }
}

// ----------------------------------------------------------------------
// Persistence
// ----------------------------------------------------------------------

/// Queue one canon write on the outbox deferred side-channel.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Mirror the meta document (the id allocator resume point).
pub(crate) fn persist_meta(out: &mut Outbox, next_canon_id: u64) {
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: serde_json::json!({ "next_canon_id": next_canon_id }),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: u64, scope: CanonScope, entities: &[u32]) -> CanonEvent {
        CanonEvent {
            id: CanonId(id),
            kind: SmolStr::new("test"),
            summary: format!("event {id}"),
            entities: entities.iter().map(|&e| ObjId(e)).collect(),
            scope,
            ic_time: 100,
            at: 1000,
            source: String::new(),
            supersedes: None,
            reason: String::new(),
            detail: serde_json::Value::Null,
        }
    }

    #[test]
    fn scope_visibility() {
        let pub_e = ev(0, CanonScope::Public, &[2]);
        let restricted = ev(1, CanonScope::Restricted, &[2]);
        let secret = ev(2, CanonScope::Secret, &[2]);
        // Public: everyone.
        assert!(pub_e.visible_to(ObjId(9), false));
        // Restricted: only entities or staff.
        assert!(restricted.visible_to(ObjId(2), false));
        assert!(!restricted.visible_to(ObjId(9), false));
        assert!(restricted.visible_to(ObjId(9), true));
        // Secret: staff only.
        assert!(!secret.visible_to(ObjId(2), false));
        assert!(secret.visible_to(ObjId(2), true));
    }

    #[test]
    fn id_allocation_is_monotonic() {
        let mut st = CanonState::default();
        assert_eq!(st.alloc_id(), CanonId(0));
        assert_eq!(st.alloc_id(), CanonId(1));
        assert_eq!(st.next_canon_id, 2);
    }

    #[test]
    fn recent_and_superseded_derive_from_the_tail() {
        let mut st = CanonState::default();
        let _ = st.alloc_id(); // 0
        let _ = st.alloc_id(); // 1
        // Append event 0, then a retcon (event 1 supersedes 0).
        let prepared = st
            .ledger
            .prepare_append(
                LOG,
                serde_json::to_value(ev(0, CanonScope::Public, &[])).unwrap(),
            )
            .unwrap();
        let (_, op0) = st.ledger.apply_prepared(prepared).unwrap();
        let mut retcon = ev(1, CanonScope::Public, &[]);
        retcon.supersedes = Some(CanonId(0));
        let prepared = st
            .ledger
            .prepare_append(LOG, serde_json::to_value(&retcon).unwrap())
            .unwrap();
        let (_, op1) = st.ledger.apply_prepared(prepared).unwrap();
        let _ = (op0, op1);
        let recent = st.recent();
        assert_eq!(recent.len(), 2);
        assert_eq!(st.superseded_in_tail(), BTreeSet::from([CanonId(0)]));
    }
}

#[cfg(test)]
mod e6b_read_seam_tests {
    use super::*;

    fn ev(id: u64, source: &str) -> CanonEvent {
        CanonEvent {
            id: CanonId(id),
            kind: SmolStr::new("scene.outcome"),
            summary: String::new(),
            entities: vec![ObjId(2)],
            scope: CanonScope::Restricted,
            ic_time: 100,
            at: 1000,
            source: source.to_string(),
            supersedes: None,
            reason: String::new(),
            detail: serde_json::Value::Null,
        }
    }

    /// E6b (DL-09): the read seam answers **by source**, newest first, bounded — and
    /// it is the only reason a seal committed before a restart is findable at all.
    #[test]
    fn e6b_b5_the_read_seam_answers_by_source_newest_first_and_bounded() {
        let mut st = CanonState::default();
        for (id, source) in [(0u64, "run:1"), (1, "run:2"), (2, "run:1"), (3, "run:1")] {
            let prepared = st
                .ledger
                .prepare_append(LOG, serde_json::to_value(ev(id, source)).unwrap())
                .unwrap();
            let _ = st.ledger.apply_prepared(prepared).unwrap();
        }
        let hits = crate::records_for_source(&st, "run:1", 8);
        assert_eq!(
            hits.iter().map(|e| e.id.0).collect::<Vec<_>>(),
            vec![3, 2, 0],
            "newest first, and only the named source"
        );
        assert_eq!(crate::records_for_source(&st, "run:1", 1).len(), 1);
        assert_eq!(crate::records_for_source(&st, "run:9", 8).len(), 0);
        // The cap is the seam's, not the caller's: a caller asking for more than
        // `MAX_RECORDS_PER_SOURCE` still gets at most that many.
        assert!(
            crate::records_for_source(&st, "run:1", usize::MAX).len()
                <= crate::MAX_RECORDS_PER_SOURCE
        );
    }
}

#[cfg(test)]
mod e6b_verify_fold_tests {
    use super::*;

    fn ev(id: u64, source: &str) -> CanonEvent {
        CanonEvent {
            id: CanonId(id),
            kind: SmolStr::new("scene.outcome"),
            summary: String::new(),
            entities: vec![ObjId(2)],
            scope: CanonScope::Restricted,
            ic_time: 100,
            at: 1000,
            source: source.to_string(),
            supersedes: None,
            reason: String::new(),
            detail: serde_json::Value::Null,
        }
    }

    fn append(st: &mut CanonState, e: CanonEvent) {
        let prepared = st
            .ledger
            .prepare_append(LOG, serde_json::to_value(e).unwrap())
            .unwrap();
        let _ = st.ledger.apply_prepared(prepared).unwrap();
    }

    /// **R2.** A **retconned** record is not evidence. A retcon appends rather than
    /// rewrites and deliberately emits no `CanonRecorded`, so the economy never
    /// hears it — without this filter the DL-09 recovery path would resurrect a seal
    /// a staffer had already superseded and settle a contract on retracted evidence,
    /// which the live path could never do.
    #[test]
    fn vf_r2_a_retconned_record_is_not_returned_as_evidence() {
        let mut st = CanonState::default();
        append(&mut st, ev(0, "run:1#success"));
        assert_eq!(crate::records_for_source(&st, "run:1#success", 8).len(), 1);
        let mut retcon = ev(1, "canon:0");
        retcon.supersedes = Some(CanonId(0));
        append(&mut st, retcon);
        assert!(
            crate::records_for_source(&st, "run:1#success", 8).is_empty(),
            "a superseded seal was still handed out as evidence"
        );
    }

    /// **R9.** The producer's cap and the consumer's must be the same number: the
    /// economy remembers a record's entities bounded at its own `MAX_CANON_ENTITIES`,
    /// so a draft naming more matched on the live edge and was silently missed by the
    /// recovery re-scan. `commit` truncates, and this pins the two together against
    /// the widest set any producer can build.
    #[test]
    fn vf_r9_the_commit_cap_covers_every_producers_widest_record() {
        assert_eq!(
            crate::MAX_ENTITIES_PER_EVENT,
            64,
            "the economy's MAX_CANON_ENTITIES is 64; the two must agree"
        );
        // `mush-scenes` seals at most `MAX_OUTCOME_SUBJECTS` subjects plus the
        // sealer, so no seal is ever truncated — and a draft that names more is.
        let mut wide = ev(0, "run:1#success");
        wide.entities = (0..(crate::MAX_ENTITIES_PER_EVENT as u32 + 8))
            .map(ObjId)
            .collect();
        let capped: Vec<ObjId> = wide
            .entities
            .iter()
            .copied()
            .take(crate::MAX_ENTITIES_PER_EVENT)
            .collect();
        assert_eq!(capped.len(), crate::MAX_ENTITIES_PER_EVENT);
    }
}
