//! The M12 informant loop's intel model (design: NPC INFORMANT LOOP —
//! perceive → remember → report). An [`ObjKind::Npc`](mush_core::ObjKind)
//! informant relays what it *perceives* (M11) to a [`principal`] as
//! provenance-tagged [`IntelRecord`]s; the principal PULLS them with the
//! `+intel` query. Filing is gated by perception in the observer
//! (`crate::module::file_intel` runs strictly inside the `can_perceive`-passed
//! branch), so an informant can report only what its NPC actually saw.
//!
//! [`principal`]: crate::profile::AgentProfile::principal
//!
//! The principal is a [`Principal`] — the M21 generalization: a **character**
//! (the M12 original: domitor, employer) whose bucket only they read, or a
//! **faction**, whose pool is a clearance-gated shared store read by members
//! whose level passes the faction's `intel_level` (see `mush-faction`).
//!
//! This is the epistemic-engine ledger seed: every record carries a
//! `provenance` string ("how the NPC knows"), a `ts`, and the `subject_name` /
//! `room_name` as they read at witness time.

use std::collections::HashMap;

use mush_core::ObjId;
use mush_faction::{FactionId, Principal};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// Per-principal cap on retained records: the oldest are dropped past this so
/// the log (and its persisted doc) stays bounded regardless of activity.
pub const INTEL_CAP: usize = 50;

/// What kind of event an [`IntelRecord`] captures — a small closed set covering
/// the observer signals an informant can witness.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntelKind {
    /// The subject arrived in the room (movement or connect).
    Arrived,
    /// The subject left the room (movement or disconnect).
    Departed,
    /// The subject was present (a generic sighting).
    Present,
    /// The subject spoke where the informant could hear.
    Spoke,
    /// The informant pierced the subject's *active concealment* — a notable
    /// sighting of someone trying not to be seen.
    Concealed,
    /// The informant perceived the subject *using a supernatural power* (a
    /// Discipline / spell / Gift). Inherently notable — filed regardless of the
    /// watch list, like [`IntelKind::Concealed`].
    #[serde(rename = "poweruse")]
    PowerUse,
}

/// One filed piece of intel: an informant's provenance-tagged report to its
/// principal about a subject it perceived.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IntelRecord {
    /// The NPC that witnessed and filed this.
    pub informant: ObjId,
    /// Who it was filed for — a character (domitor) or a faction pool.
    /// Serde stays wire-compatible with the M12 form (a bare dbref number).
    pub principal: Principal,
    /// Who the intel is about.
    pub subject: ObjId,
    /// The subject's display name at witness time.
    pub subject_name: String,
    /// The event kind.
    pub kind: IntelKind,
    /// A short human detail ("arrived", "said: …", …).
    pub detail: String,
    /// The room the informant witnessed it in.
    pub room_name: String,
    /// How the NPC knows (the epistemic-engine ledger seed).
    pub provenance: String,
    /// When witnessed (unix seconds).
    pub ts: i64,
}

/// The persistence key one principal's bounded record vec lives under (one
/// module doc per principal, whole-doc rewritten on each file — like a memory
/// subject doc). Character principals keep the M12 key form `intel:{dbref}`
/// so existing docs load unchanged; faction pools live at
/// `intel:f:{faction id}`.
#[must_use]
pub fn intel_key(principal: Principal) -> SmolStr {
    match principal {
        Principal::Character(id) => SmolStr::new(format!("intel:{}", id.0)),
        Principal::Faction { faction } => SmolStr::new(format!("intel:f:{}", faction.0)),
    }
}

/// Parse the id part of a persisted intel key (after the `intel:` prefix)
/// back into its [`Principal`]. `None` for malformed ids.
#[must_use]
pub fn parse_intel_key_id(id: &str) -> Option<Principal> {
    if let Some(f) = id.strip_prefix("f:") {
        f.parse::<u64>()
            .ok()
            .map(|n| Principal::Faction { faction: FactionId(n) })
    } else {
        id.parse::<u32>().ok().map(|n| Principal::Character(ObjId(n)))
    }
}

/// The in-RAM intel log for one server: records grouped by principal, bounded
/// to [`INTEL_CAP`] per principal (oldest dropped). Held in
/// [`NpcHostState`](crate::runtime::NpcHostState) and mirrored by the persisted
/// `intel:*` documents; rebuilt from boot data on load.
#[derive(Clone, Debug, Default)]
pub struct IntelStore {
    by_principal: HashMap<Principal, Vec<IntelRecord>>,
}

impl IntelStore {
    /// File one record for its principal, enforcing the [`INTEL_CAP`] bound
    /// (oldest dropped). Returns a snapshot of the principal's bounded vec so
    /// the caller can persist the whole doc.
    pub fn file(&mut self, rec: IntelRecord) -> Vec<IntelRecord> {
        let principal = rec.principal;
        let bucket = self.by_principal.entry(principal).or_default();
        bucket.push(rec);
        if bucket.len() > INTEL_CAP {
            let overflow = bucket.len() - INTEL_CAP;
            bucket.drain(..overflow);
        }
        bucket.clone()
    }

    /// Load a principal's records from boot data (replacing any present).
    pub fn load(&mut self, principal: Principal, mut records: Vec<IntelRecord>) {
        if records.len() > INTEL_CAP {
            let overflow = records.len() - INTEL_CAP;
            records.drain(..overflow);
        }
        self.by_principal.insert(principal, records);
    }

    /// A principal's records, oldest-first (empty if none). Callers wanting
    /// newest-first reverse this.
    #[must_use]
    pub fn for_principal(&self, principal: Principal) -> &[IntelRecord] {
        self.by_principal
            .get(&principal)
            .map_or(&[], Vec::as_slice)
    }

    /// A principal's records about one subject, oldest-first.
    #[must_use]
    pub fn for_principal_subject(
        &self,
        principal: Principal,
        subject: ObjId,
    ) -> Vec<&IntelRecord> {
        self.for_principal(principal)
            .iter()
            .filter(|r| r.subject == subject)
            .collect()
    }

    /// How many records a given NPC has filed (across every principal). Backs
    /// the staff `@agent/show` count.
    #[must_use]
    pub fn filed_by(&self, informant: ObjId) -> usize {
        self.by_principal
            .values()
            .flat_map(|v| v.iter())
            .filter(|r| r.informant == informant)
            .count()
    }

    /// Clear a principal's records, returning how many were dropped.
    pub fn clear_for(&mut self, principal: Principal) -> usize {
        self.by_principal
            .remove(&principal)
            .map_or(0, |v| v.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(n: u32) -> Principal {
        Principal::Character(ObjId(n))
    }

    fn rec(informant: u32, principal: Principal, subject: u32, ts: i64) -> IntelRecord {
        IntelRecord {
            informant: ObjId(informant),
            principal,
            subject: ObjId(subject),
            subject_name: format!("subj{subject}"),
            kind: IntelKind::Present,
            detail: format!("seen at {ts}"),
            room_name: "Bar".into(),
            provenance: "witnessed directly by npc".into(),
            ts,
        }
    }

    #[test]
    fn file_groups_by_principal_and_returns_snapshot() {
        let mut s = IntelStore::default();
        let snap = s.file(rec(9, ch(1), 2, 100));
        assert_eq!(snap.len(), 1);
        s.file(rec(9, ch(1), 3, 200));
        s.file(rec(9, ch(2), 2, 300)); // a different principal
        assert_eq!(s.for_principal(ch(1)).len(), 2);
        assert_eq!(s.for_principal(ch(2)).len(), 1);
        // No cross-principal leak.
        assert!(s.for_principal(ch(1)).iter().all(|r| r.principal == ch(1)));
    }

    #[test]
    fn faction_pools_are_distinct_from_character_buckets() {
        let mut s = IntelStore::default();
        let pool = Principal::faction(FactionId(1));
        s.file(rec(9, ch(1), 2, 100));
        s.file(rec(9, pool, 3, 200));
        assert_eq!(s.for_principal(ch(1)).len(), 1);
        assert_eq!(s.for_principal(pool).len(), 1);
        // Character #1 and faction #1 are unrelated buckets.
        assert_eq!(s.for_principal(ch(1))[0].subject, ObjId(2));
        assert_eq!(s.for_principal(pool)[0].subject, ObjId(3));
    }

    #[test]
    fn file_is_bounded_dropping_oldest() {
        let mut s = IntelStore::default();
        for i in 0..(INTEL_CAP as i64 + 10) {
            s.file(rec(9, ch(1), 2, i));
        }
        let recs = s.for_principal(ch(1));
        assert_eq!(recs.len(), INTEL_CAP, "bounded to the cap");
        // The oldest were dropped; the newest survive (oldest-first order).
        assert_eq!(recs.first().unwrap().ts, 10);
        assert_eq!(recs.last().unwrap().ts, INTEL_CAP as i64 + 9);
    }

    #[test]
    fn for_principal_subject_filters() {
        let mut s = IntelStore::default();
        s.file(rec(9, ch(1), 2, 100));
        s.file(rec(9, ch(1), 3, 200));
        s.file(rec(9, ch(1), 2, 300));
        let about2 = s.for_principal_subject(ch(1), ObjId(2));
        assert_eq!(about2.len(), 2);
        assert!(about2.iter().all(|r| r.subject == ObjId(2)));
    }

    #[test]
    fn filed_by_counts_across_principals() {
        let mut s = IntelStore::default();
        s.file(rec(9, ch(1), 2, 100));
        s.file(rec(9, Principal::faction(FactionId(7)), 2, 200));
        s.file(rec(8, ch(1), 2, 300)); // a different informant
        assert_eq!(s.filed_by(ObjId(9)), 2);
        assert_eq!(s.filed_by(ObjId(8)), 1);
    }

    #[test]
    fn clear_for_drops_only_that_principal() {
        let mut s = IntelStore::default();
        s.file(rec(9, ch(1), 2, 100));
        s.file(rec(9, ch(1), 3, 200));
        s.file(rec(9, ch(2), 2, 300));
        assert_eq!(s.clear_for(ch(1)), 2);
        assert!(s.for_principal(ch(1)).is_empty());
        assert_eq!(s.for_principal(ch(2)).len(), 1);
        assert_eq!(s.clear_for(ch(1)), 0, "already clear");
    }

    #[test]
    fn load_replaces_and_bounds() {
        let mut s = IntelStore::default();
        let many: Vec<IntelRecord> = (0..(INTEL_CAP as i64 + 5))
            .map(|i| rec(9, ch(1), 2, i))
            .collect();
        s.load(ch(1), many);
        assert_eq!(s.for_principal(ch(1)).len(), INTEL_CAP);
        // Load replaces rather than appending.
        s.load(ch(1), vec![rec(9, ch(1), 2, 999)]);
        assert_eq!(s.for_principal(ch(1)).len(), 1);
        assert_eq!(s.for_principal(ch(1))[0].ts, 999);
    }

    #[test]
    fn intel_keys_round_trip_and_stay_wire_compatible() {
        // The M12 key form is unchanged for characters.
        assert_eq!(intel_key(ch(42)).as_str(), "intel:42");
        assert_eq!(
            intel_key(Principal::faction(FactionId(3))).as_str(),
            "intel:f:3"
        );
        assert_eq!(parse_intel_key_id("42"), Some(ch(42)));
        assert_eq!(
            parse_intel_key_id("f:3"),
            Some(Principal::faction(FactionId(3)))
        );
        assert_eq!(parse_intel_key_id("bogus"), None);
        assert_eq!(parse_intel_key_id("f:bogus"), None);
    }

    #[test]
    fn record_serde_is_backward_compatible_with_m12_docs() {
        // An M12-era persisted record carried `"principal": <dbref>`.
        let old = r#"{"informant":9,"principal":4,"subject":2,"subject_name":"X",
                      "kind":"spoke","detail":"said: hi","room_name":"Bar",
                      "provenance":"witnessed directly by npc","ts":100}"#;
        let rec: IntelRecord = serde_json::from_str(old).unwrap();
        assert_eq!(rec.principal, ch(4));
        // Faction records tag themselves.
        let json = serde_json::to_string(&IntelRecord {
            principal: Principal::faction(FactionId(6)),
            ..rec
        })
        .unwrap();
        assert!(json.contains(r#""principal":{"faction":6}"#));
    }
}
