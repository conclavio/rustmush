//! One owner-local bounded book. It never treats absence as a close event.
use super::protocol::{MAX_HISTORY, MAX_OPEN, MAX_TOTAL_BYTES};
use super::records::{
    CompactedRecord, HEAD_MAX_BYTES, Head, Record, StoredRecord, bad, pruned_chain_digest,
};
use mush_core::participant::ParticipantError;
use mush_core::{CommitToken, DefinitionDigest, ObjId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Maximum oldest terminal non-coverage rows pruned in a single Open (well under
/// `MAX_AUTHORITATIVE_REPLAY_WRITES = 1024`).
const MAX_PRUNE_PER_OPEN: usize = 640;

pub(super) enum IcSortieDelta {
    Open {
        row: Arc<StoredRecord>,
        pruned: Vec<CommitToken>,
        post_reserved_bytes: usize,
        post_next_transition: u64,
        post_pruned_count: u64,
        post_pruned_digest: Option<DefinitionDigest>,
    },
    Close {
        row: Arc<StoredRecord>,
        post_reserved_bytes: usize,
        post_next_transition: u64,
    },
}
impl IcSortieDelta {
    pub(super) fn row(&self) -> &StoredRecord {
        match self {
            Self::Open { row, .. } | Self::Close { row, .. } => row,
        }
    }
    pub(super) fn pruned(&self) -> &[CommitToken] {
        match self {
            Self::Open { pruned, .. } => pruned,
            Self::Close { .. } => &[],
        }
    }
}

#[derive(Clone, Default)]
pub struct IcSorties {
    pub(super) loaded: bool,
    pub(super) generation: u64,
    pub(super) next_transition: u64,
    pub(super) pruned_count: u64,
    pub(super) pruned_digest: Option<DefinitionDigest>,
    pub(super) reserved_bytes: usize,
    pub(super) rows: BTreeMap<CommitToken, Arc<StoredRecord>>,
    pub(super) active: BTreeMap<ObjId, CommitToken>,
    /// O(log N) lookup from Open/Close request digest -> (open token, is_close).
    pub(super) requests: BTreeMap<DefinitionDigest, (CommitToken, bool)>,
    /// O(log N) lookup for retained Close commit tokens.
    pub(super) close_operations: BTreeSet<CommitToken>,
    /// Prunable terminal non-coverage rows ordered strictly by ascending `closed.transition`.
    pub(super) prunable: BTreeMap<u64, CommitToken>,
}
impl IcSorties {
    pub(super) fn reconstruct(
        head: Head,
        rows: impl IntoIterator<Item = StoredRecord>,
    ) -> Result<Self, ParticipantError> {
        head.validate()?;
        let mut s = Self {
            loaded: true,
            next_transition: 1,
            pruned_count: head.pruned_count,
            pruned_digest: head.pruned_digest,
            reserved_bytes: HEAD_MAX_BYTES,
            ..Self::default()
        };
        let mut order = BTreeMap::new();
        let mut commits = BTreeSet::new();
        for r in rows {
            r.validate()?;
            let open = r.open_operation();
            // Doc 23 cut S.4b-1 (23-7): the book's transition order is the Open's own field;
            // the seal's sortie may be the epoch's IC id.
            let sortie = r.open_transition();
            let open_request = r.open_request();
            if s.rows.len() >= MAX_HISTORY
                || !commits.insert(open)
                || s.requests.insert(open_request, (open, false)).is_some()
                || order.insert(sortie, (open, false)).is_some()
            {
                return Err(bad("duplicate or excess history"));
            }
            if let Some(c) = r.closed() {
                if !commits.insert(c.operation)
                    || !s.close_operations.insert(c.operation)
                    || s.requests.insert(c.request, (open, true)).is_some()
                    || order.insert(c.transition, (open, true)).is_some()
                {
                    return Err(bad("crossed close"));
                }
                if r.is_prunable() {
                    s.prunable.insert(c.transition, open);
                }
            }
            s.reserved_bytes = s
                .reserved_bytes
                .checked_add(r.reserved_bytes()?)
                .ok_or_else(|| bad("bytes"))?;
            if s.reserved_bytes > MAX_TOTAL_BYTES {
                return Err(bad("total bytes"));
            }
            if s.rows.insert(open, Arc::new(r)).is_some() {
                return Err(bad("duplicate open"));
            }
        }
        let order_len = order.len();
        for (ordinal, (open, closing)) in order {
            if head.pruned_count == 0 {
                if ordinal != s.next_transition {
                    return Err(bad("missing transition"));
                }
            } else if ordinal == 0 || ordinal >= head.next_transition {
                return Err(bad("transition out of range"));
            }
            let row = s.rows.get(&open).ok_or_else(|| bad("transition"))?;
            let unit = row.unit();
            if closing {
                if s.active.remove(&unit) != Some(open) {
                    return Err(bad("crossed release"));
                }
            } else if s.active.len() >= MAX_OPEN || s.active.insert(unit, open).is_some() {
                return Err(bad("historical capacity"));
            }
            if head.pruned_count == 0 {
                s.next_transition = s
                    .next_transition
                    .checked_add(1)
                    .ok_or_else(|| bad("ordinal"))?;
            }
        }
        if head.pruned_count == 0 {
            if head.next_transition != s.next_transition {
                return Err(bad("head completeness"));
            }
        } else {
            let expected_transitions = u64::try_from(order_len)
                .ok()
                .and_then(|n| head.pruned_count.checked_mul(2)?.checked_add(n))
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| bad("head completeness"))?;
            if head.next_transition != expected_transitions {
                return Err(bad("head completeness"));
            }
            s.next_transition = head.next_transition;
        }
        Ok(s)
    }
    pub(super) fn request_used(&self, request: DefinitionDigest) -> bool {
        self.requests.contains_key(&request)
    }
    pub(super) fn ready(&self) -> Result<(), ParticipantError> {
        // Config::check also requires the actual paired Roll final audit.
        if !self.loaded {
            return Err(bad("four-owner audit incomplete"));
        }
        Ok(())
    }
    pub(super) fn prepare_open(&self, row: Record) -> Result<IcSortieDelta, ParticipantError> {
        self.ready()?;
        row.validate()?;
        if row.closed.is_some()
            || row.open.transition != self.next_transition
            || self.active.len() >= MAX_OPEN
            || self.request_used(row.open.request)
            || self.rows.contains_key(&row.open.operation)
            || self.active.contains_key(&row.open.execution.seal.unit)
            || self.close_operations.contains(&row.open.operation)
        {
            return Err(bad("open capacity/identity"));
        }
        let stored = StoredRecord::Full(Box::new(row));
        let added_bytes = stored.reserved_bytes()?;
        let mut retained_len = self.rows.len();
        let mut bytes = self.reserved_bytes;
        let mut pruned = Vec::new();
        let mut pruned_count = self.pruned_count;
        let mut pruned_digest = self.pruned_digest;
        for (&close_transition, &open_token) in &self.prunable {
            let fits_len = retained_len < MAX_HISTORY;
            let fits_bytes = bytes
                .checked_add(added_bytes)
                .is_some_and(|total| total <= MAX_TOTAL_BYTES);
            if fits_len && fits_bytes {
                break;
            }
            if pruned.len() >= MAX_PRUNE_PER_OPEN {
                break;
            }
            let victim = self
                .rows
                .get(&open_token)
                .ok_or_else(|| bad("prune target"))?;
            let closed = victim.closed().ok_or_else(|| bad("prune open row"))?;
            if closed.transition != close_transition || !victim.is_prunable() {
                return Err(bad("prune invariant"));
            }
            let victim_bytes = victim.reserved_bytes()?;
            bytes = bytes
                .checked_sub(victim_bytes)
                .ok_or_else(|| bad("prune bytes"))?;
            retained_len = retained_len
                .checked_sub(1)
                .ok_or_else(|| bad("prune count"))?;
            pruned_count = pruned_count
                .checked_add(1)
                .ok_or_else(|| bad("prune counter"))?;
            pruned_digest = Some(pruned_chain_digest(
                pruned_digest,
                victim.open_operation(),
                closed.operation,
                victim.open_transition(),
                closed.transition,
                victim.unit(),
                victim.seal_digest(),
                closed.unit_digest,
            )?);
            pruned.push(open_token);
        }
        if retained_len >= MAX_HISTORY {
            return Err(bad("open capacity/identity"));
        }
        let post_reserved_bytes = bytes.checked_add(added_bytes).ok_or_else(|| bad("bytes"))?;
        if post_reserved_bytes > MAX_TOTAL_BYTES {
            return Err(bad("total bytes"));
        }
        let post_next_transition = self
            .next_transition
            .checked_add(1)
            .ok_or_else(|| bad("ordinal"))?;
        self.generation
            .checked_add(1)
            .ok_or_else(|| bad("generation"))?;
        Ok(IcSortieDelta::Open {
            row: Arc::new(stored),
            pruned,
            post_reserved_bytes,
            post_next_transition,
            post_pruned_count: pruned_count,
            post_pruned_digest: pruned_digest,
        })
    }
    pub(super) fn prepare_close(&self, row: Record) -> Result<IcSortieDelta, ParticipantError> {
        self.ready()?;
        row.validate()?;
        let before = self
            .rows
            .get(&row.open.operation)
            .ok_or_else(|| bad("open missing"))?;
        let before_full = before.as_full().ok_or_else(|| bad("open already closed"))?;
        let c = row.closed.as_ref().ok_or_else(|| bad("close missing"))?;
        if before_full.open != row.open
            || before_full.closed.is_some()
            || c.transition != self.next_transition
            || self.active.get(&row.open.execution.seal.unit) != Some(&row.open.operation)
            || self.request_used(c.request)
            || self.rows.contains_key(&c.operation)
            || self.close_operations.contains(&c.operation)
            || before_full.reserved_bytes()? != row.reserved_bytes()?
        {
            return Err(bad("close identity"));
        }
        let before_bytes = before.reserved_bytes()?;
        let stored = if row.open.withheld.is_none() {
            StoredRecord::Compacted(Box::new(CompactedRecord::from_closed(&row)?))
        } else {
            StoredRecord::Full(Box::new(row))
        };
        let after_bytes = stored.reserved_bytes()?;
        let post_reserved_bytes = self
            .reserved_bytes
            .checked_sub(before_bytes)
            .and_then(|base| base.checked_add(after_bytes))
            .ok_or_else(|| bad("close bytes"))?;
        if post_reserved_bytes > MAX_TOTAL_BYTES {
            return Err(bad("total bytes"));
        }
        let post_next_transition = self
            .next_transition
            .checked_add(1)
            .ok_or_else(|| bad("ordinal"))?;
        self.generation
            .checked_add(1)
            .ok_or_else(|| bad("generation"))?;
        Ok(IcSortieDelta::Close {
            row: Arc::new(stored),
            post_reserved_bytes,
            post_next_transition,
        })
    }
    /// Apply a delta checked by [`Self::prepare_open`] or [`Self::prepare_close`] in O(log N)
    /// without cloning the historical `rows` map.
    pub(super) fn apply_delta(&mut self, delta: IcSortieDelta) {
        match delta {
            IcSortieDelta::Open {
                row,
                pruned,
                post_reserved_bytes,
                post_next_transition,
                post_pruned_count,
                post_pruned_digest,
            } => {
                for open_token in pruned {
                    if let Some(victim) = self.rows.remove(&open_token) {
                        self.requests.remove(&victim.open_request());
                        if let Some(c) = victim.closed() {
                            self.requests.remove(&c.request);
                            self.close_operations.remove(&c.operation);
                            self.prunable.remove(&c.transition);
                        }
                    }
                }
                let open_op = row.open_operation();
                self.active.insert(row.unit(), open_op);
                self.requests.insert(row.open_request(), (open_op, false));
                self.rows.insert(open_op, row);
                self.reserved_bytes = post_reserved_bytes;
                self.next_transition = post_next_transition;
                self.pruned_count = post_pruned_count;
                self.pruned_digest = post_pruned_digest;
                self.generation = self.generation.saturating_add(1);
            }
            IcSortieDelta::Close {
                row,
                post_reserved_bytes,
                post_next_transition,
            } => {
                let open_op = row.open_operation();
                self.active.remove(&row.unit());
                if let Some(c) = row.closed() {
                    self.requests.insert(c.request, (open_op, true));
                    self.close_operations.insert(c.operation);
                    if row.is_prunable() {
                        self.prunable.insert(c.transition, open_op);
                    }
                }
                self.rows.insert(open_op, row);
                self.reserved_bytes = post_reserved_bytes;
                self.next_transition = post_next_transition;
                self.generation = self.generation.saturating_add(1);
            }
        }
    }
    pub(super) fn head_for_delta(&self, delta: &IcSortieDelta) -> Head {
        match delta {
            IcSortieDelta::Open {
                post_next_transition,
                post_pruned_count,
                post_pruned_digest,
                ..
            } => Head {
                format: 1,
                next_transition: *post_next_transition,
                pruned_count: *post_pruned_count,
                pruned_digest: *post_pruned_digest,
            },
            IcSortieDelta::Close {
                post_next_transition,
                ..
            } => Head {
                format: 1,
                next_transition: *post_next_transition,
                pruned_count: self.pruned_count,
                pruned_digest: self.pruned_digest,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{
        Controller, PartyId,
        core::CoreClass,
        mind::{SealEngine, SortieId},
        profile_seal::{FiledCertificate, MindIdentity, MindOrigin},
        runtime_card::stock_definition,
        test_support::{pose, tank},
    };
    use crate::arena_task::MatchId;
    use crate::config::ArenaConfig;
    use crate::match_::SealTerms;
    use crate::minds::ic_sortie::{
        execution::Execution,
        records::{CapturedProtocol, Closing, EventKind, FORMAT, Opening},
    };
    use mush_core::mechanics::{DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId};
    use mush_core::{ClockRef, ClockStamp, DefinitionDescriptor};
    use mush_roll::commerce::ic_claims::IcClaimConfig;
    use sha2::{Digest as _, Sha256};

    const UNIT: ObjId = ObjId(41);
    const DISPATCHER: ObjId = ObjId(7);
    const BASE: ObjId = ObjId(99);

    fn protocol() -> CapturedProtocol {
        let manifest = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tacsim").expect("owner"),
                DefinitionDescriptor::new(
                    DefinitionKind::new("registry_manifest").expect("kind"),
                    DefinitionId::new("combat").expect("id"),
                    1,
                    serde_json::json!({"format": 1}),
                )
                .expect("manifest descriptor"),
            )
            .expect("manifest ref");
        let seal = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tacsim").expect("owner"),
                crate::minds::ic_sortie::protocol::participant_descriptor()
                    .expect("participant descriptor"),
            )
            .expect("seal ref");
        let cfg = IcClaimConfig::new(manifest, seal).expect("ic claim config");
        CapturedProtocol::capture(&cfg)
    }

    fn sample_open(seed: u8, transition: u64, at_ic: i64, source: &str) -> Record {
        let mut hull = tank(UNIT, PartyId::RED, pose(1500, 1500));
        hull.controller = Controller::Mind;
        let mut terms = SealTerms::fixture();
        terms.match_id = MatchId(7);
        terms.principal =
            Some(crate::deliberation::order::CharacterRef::Character { id: DISPATCHER });
        let definition =
            stock_definition(&[("Line".into(), terms.card.clone())], CoreClass::I).expect("def");
        let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        let certificate = FiledCertificate {
            revision_character: ObjId(77),
            revision_mind: 1,
            revision_number: 1,
            revision_digest: DefinitionDigest::from_bytes([1; 32]),
            source_digest: DefinitionDigest::from_bytes(digest),
            rating_job: CommitToken::from_bytes([4; 16]),
            rating_id: DefinitionDigest::from_bytes([2; 32]),
            rating_schema: 1,
            rating_digest: DefinitionDigest::from_bytes([3; 32]),
            compatibility: DefinitionDigest::from_bytes([4; 32]),
            definition: definition.validate().expect("valid"),
            declared: CoreClass::I,
            minimum: CoreClass::I,
        };
        terms.authored = Some(Arc::new(definition));
        terms.selected_profiles = vec!["Line".into()];
        terms.identity = MindIdentity {
            mind: "Compaction fixture".into(),
            origin: MindOrigin::Filed {
                charter: "1".into(),
                line: "1".into(),
                revision: 1,
                licence: "1".into(),
                certificate: Box::new(certificate),
            },
            source_sha256: digest,
        };
        terms.rated = CoreClass::I;
        terms.engine_for_minds = SealEngine::Scripted;
        hull.reseal(&terms).expect("reseal");
        hull.seal.sortie = SortieId::ic(UNIT, at_ic);
        let execution = Execution::from_parts(
            hull.seal.clone(),
            source.into(),
            ArenaConfig::default(),
            terms.classes,
        );
        let proto = protocol();
        let open_action = proto.config().expect("cfg").actions().expect("acts")[0].clone();
        Record {
            format: FORMAT,
            open: Opening {
                request: DefinitionDigest::from_bytes([seed; 32]),
                operation: CommitToken::from_bytes([seed; 16]),
                action: open_action,
                protocol: proto,
                dispatcher: DISPATCHER,
                transition,
                withheld: None,
                admission_digest: DefinitionDigest::from_bytes([seed.wrapping_add(100); 32]),
                at_ic: ClockStamp {
                    clock: ClockRef::WorldIc { generation: 1 },
                    at: at_ic,
                },
                unit: hull.unit,
                execution,
            },
            closed: None,
        }
    }

    fn close_of(open_row: &Record, seed: u8, close_transition: u64, at_ic: i64) -> Record {
        let proto = open_row.open.protocol.clone();
        let close_action = proto.config().expect("cfg").actions().expect("acts")[1].clone();
        let mut closed = Closing {
            request: DefinitionDigest::from_bytes([seed.wrapping_add(150); 32]),
            operation: CommitToken::from_bytes([seed.wrapping_add(150); 16]),
            action: close_action,
            protocol: proto,
            transition: close_transition,
            sequence: 1,
            kind: EventKind::ReturnedToBase { base: BASE },
            at_ic: ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: at_ic,
            },
            conditions_digest: DefinitionDigest::from_bytes([7; 32]),
            unit_digest: DefinitionDigest::from_bytes([seed.wrapping_add(200); 32]),
            event_digest: DefinitionDigest::from_bytes([0; 32]),
            coverage: None,
        };
        closed.event_digest = closed
            .event_digest_parts(
                open_row.open.operation,
                open_row.open.execution.seal.unit,
                open_row.open.execution.seal.sortie,
                open_row.open.execution.digest(),
            )
            .expect("digest");
        Record {
            format: FORMAT,
            open: open_row.open.clone(),
            closed: Some(closed),
        }
    }

    #[test]
    fn p2_tsc_6_compaction_on_close_reclaims_bytes_and_pruning_roundtrips() {
        let mut book = IcSorties::reconstruct(
            Head {
                format: 1,
                next_transition: 1,
                pruned_count: 0,
                pruned_digest: None,
            },
            Vec::new(),
        )
        .expect("fresh book");

        // Open and close a sortie with a bulky Rhai source.
        let bulky = format!("fn think(v,c) {{ let s = \"{}\"; }}", "a".repeat(20_000));
        let open1 = sample_open(1, 1, 100, &bulky);
        let open1_op = open1.open.operation;
        let open1_delta = book.prepare_open(open1.clone()).expect("open1");
        let open_bytes = open1_delta.row().reserved_bytes().expect("open bytes");
        book.apply_delta(open1_delta);
        assert!(open_bytes > 28_000);
        assert_eq!(book.reserved_bytes, HEAD_MAX_BYTES + open_bytes);

        let closed1 = close_of(&open1, 1, 2, 200);
        let close1_delta = book.prepare_close(closed1).expect("close1");
        let compacted_bytes = close1_delta.row().reserved_bytes().expect("compact bytes");
        assert!(
            matches!(close1_delta.row(), StoredRecord::Compacted(_)),
            "terminal non-coverage row must compact on close"
        );
        assert!(
            compacted_bytes < open_bytes / 5,
            "compaction must reclaim the vast majority of reserved_bytes ({compacted_bytes} vs {open_bytes})"
        );
        let head_after_close = book.head_for_delta(&close1_delta);
        book.apply_delta(close1_delta);
        assert_eq!(book.reserved_bytes, HEAD_MAX_BYTES + compacted_bytes);

        // Encoded compacted record round-trips through StoredRecord::decode and reconstruct.
        let wire = book
            .rows
            .get(&open1_op)
            .expect("row")
            .to_value()
            .expect("encode");
        let decoded = StoredRecord::decode(&wire).expect("decode compacted");
        let rebuilt = IcSorties::reconstruct(head_after_close, vec![decoded]).expect("reconstruct");
        assert_eq!(rebuilt.next_transition, 3);
        assert_eq!(rebuilt.reserved_bytes, book.reserved_bytes);
        assert_eq!(rebuilt.prunable.len(), 1);

        // Open and close a second sortie so we have 2 prunable closed rows.
        let open2 = sample_open(2, 3, 300, &bulky);
        let d = book.prepare_open(open2.clone()).expect("open2");
        book.apply_delta(d);
        let d = book
            .prepare_close(close_of(&open2, 2, 4, 400))
            .expect("close2");
        book.apply_delta(d);
        assert_eq!(book.prunable.len(), 2);

        // Simulate near-MAX_TOTAL_BYTES pressure so opening row 3 must prune the
        // oldest closed non-coverage row (open1) while keeping open2 and open3.
        let open3 = sample_open(3, 5, 500, &bulky);
        let open3_needed = StoredRecord::Full(Box::new(open3.clone()))
            .reserved_bytes()
            .expect("open3 bytes");
        let slack = compacted_bytes / 2;
        book.reserved_bytes = MAX_TOTAL_BYTES - open3_needed + slack;
        let open3_delta = book.prepare_open(open3).expect("open3 prunes oldest");
        assert_eq!(open3_delta.pruned(), &[open1_op]);
        let head_pruned = book.head_for_delta(&open3_delta);
        assert_eq!(head_pruned.pruned_count, 1);
        assert!(head_pruned.pruned_digest.is_some());
        assert_eq!(head_pruned.next_transition, 6);
        book.apply_delta(open3_delta);
        assert!(!book.rows.contains_key(&open1_op));
        assert_eq!(book.rows.len(), 2);

        // Reconstruct with pruned checkpoint succeeds and rejects tampered counts.
        let persisted_rows: Vec<StoredRecord> = book
            .rows
            .values()
            .map(|r| StoredRecord::decode(&r.to_value().expect("enc")).expect("dec"))
            .collect();
        let restored = IcSorties::reconstruct(head_pruned.clone(), persisted_rows.clone())
            .expect("reconstruct with pruned checkpoint");
        assert_eq!(restored.pruned_count, 1);
        assert_eq!(restored.pruned_digest, head_pruned.pruned_digest);
        assert_eq!(restored.next_transition, 6);

        let mut bad_head = head_pruned;
        bad_head.next_transition = 7;
        assert!(
            IcSorties::reconstruct(bad_head, persisted_rows).is_err(),
            "transition count mismatch with pruned_count must fail closed"
        );
    }
}
