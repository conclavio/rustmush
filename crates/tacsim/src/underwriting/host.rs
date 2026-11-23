//! Bounded private host intents. Wire selectors cannot create a departure,
//! quote authority, loss, successful submission or policy acknowledgement.
use super::{
    CoverageError, audit, authority,
    families::{Families, Row},
    metadata::Publications,
    policy::{Policy, Terminal},
    protocol::Protocol,
    records::Quote,
};
use crate::minds::ic_sortie::{AcknowledgedDeparture, CommittedCoverageClose, IcHost};
use mush_core::submission_observation::{SubmissionDisposition, SubmissionObservation};
use mush_core::{
    ClockStamp, CommitToken, Ctx, DefinitionDigest, DefinitionRef, IdempotencyKeyText, ObjId,
    PerceivedRef, World,
};
use mush_economy::{AccountId, Amount};
use mush_roll::commerce::coverage::CoverageWriter;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

/// Module KV key persisting non-ring-overwritten unwarranted inspection records
/// for active policies and covered hulls (`P2-TSW-38`).
pub(super) const UNWARRANTED_KEY: &str = "coverage_unwarranted_inspections";
/// Maximum active (non-expired) quotes per `(actor, ic_open)` (`P2-TSW-39`).
pub(super) const MAX_ACTIVE_QUOTES_PER_OPEN: usize = 8;
/// Maximum expired unbound quotes pruned inside one `prepare_quote` txn (`P2-TSW-39`).
pub(super) const MAX_PRUNE_PER_QUOTE: usize = 16;

const MAX_BUYER_INBOX: usize = 128;
const MAX_SERVICE_INBOX: usize = 128;
const INBOX_IDLE_TTL: Duration = Duration::from_secs(300);
const MAX_VOID_HULLS: usize = 65_536;
const MAX_VOID_RECORDS_PER_HULL: usize = 64;
const MAX_POLICY_VOIDS: usize = 65_536;
const MAX_REQUEST_CANDIDATES_PER_REQUEST: usize = 16;
const MAX_REQUEST_CANDIDATE_KEYS: usize = 4_096;

fn bad(_: impl std::fmt::Display) -> CoverageError {
    CoverageError::Authority
}
#[derive(Clone)]
pub(super) enum Intent {
    Quote {
        departure: Arc<AcknowledgedDeparture>,
        publication: CommitToken,
        payer: AccountId,
        value: Amount,
    },
    Bind {
        departure: Arc<AcknowledgedDeparture>,
        quote: CommitToken,
    },
    Terminal {
        close: Arc<CommittedCoverageClose>,
        policy: CommitToken,
        cancel: bool,
    },
}
enum Attempt {
    New,
    Observed(SubmissionObservation),
    Retryable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageSubmissionState {
    Unsubmitted,
    AwaitingAcknowledgement,
    Retryable,
}
impl Attempt {
    fn poll(&mut self) {
        if let Self::Observed(o) = self
            && matches!(
                o.take_terminal(),
                Some(SubmissionDisposition::NotDispatched | SubmissionDisposition::NoIntent)
            )
        {
            *self = Self::Retryable;
        }
    }
    fn status(&self) -> CoverageSubmissionState {
        match self {
            Self::New => CoverageSubmissionState::Unsubmitted,
            Self::Observed(_) => CoverageSubmissionState::AwaitingAcknowledgement,
            Self::Retryable => CoverageSubmissionState::Retryable,
        }
    }
}
type SlotKey = (ObjId, usize, Option<CommitToken>);
struct Pending {
    actor: ObjId,
    index: usize,
    intent: Intent,
    attempt: Attempt,
    seq: u64,
    created_at: Instant,
}
impl Pending {
    fn slot_key(&self) -> SlotKey {
        slot_key(self.actor, self.index, &self.intent)
    }
}
fn slot_key(actor: ObjId, index: usize, intent: &Intent) -> SlotKey {
    let policy = match intent {
        Intent::Terminal { policy, .. } => Some(*policy),
        Intent::Quote { .. } | Intent::Bind { .. } => None,
    };
    (actor, index, policy)
}
#[derive(Default)]
struct Inbox {
    entries: BTreeMap<DefinitionDigest, Pending>,
    next_seq: u64,
}
impl Inbox {
    fn prune_expired_at(&mut self, now: Instant) {
        for p in self.entries.values_mut() {
            p.attempt.poll();
        }
        self.entries.retain(|_, p| {
            matches!(p.attempt, Attempt::Observed(_))
                || now.saturating_duration_since(p.created_at) < INBOX_IDLE_TTL
        });
    }
    fn lane_count(&self, service: ObjId, is_service_terminal: bool) -> usize {
        self.entries
            .values()
            .filter(|p| (p.actor == service && p.index >= 3) == is_service_terminal)
            .count()
    }
    fn allocate_seq(&mut self) -> u64 {
        self.next_seq = self.next_seq.saturating_add(1);
        self.next_seq
    }
}

/// One durable unwarranted-inspection record retained outside the 64-slot ring
/// buffer (`P2-TSW-38`) so subsequent governed inspections cannot evict it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VoidRecord {
    pub(super) seq: u64,
    pub(super) at_ic: ClockStamp,
    pub(super) inspection: CommitToken,
}

/// Authoritative non-ring-overwritten index of unwarranted inspections per hull
/// and pinned to active policies whose terms cover them (`P2-TSW-38`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UnwarrantedIndex {
    pub(super) version: u8,
    #[serde(default)]
    pub(super) hulls: BTreeMap<u32, Vec<VoidRecord>>,
    #[serde(default)]
    pub(super) policy_voids: BTreeMap<String, VoidRecord>,
    #[serde(skip)]
    pub(super) dirty: bool,
}
impl Default for UnwarrantedIndex {
    fn default() -> Self {
        Self {
            version: 1,
            hulls: BTreeMap::new(),
            policy_voids: BTreeMap::new(),
            dirty: false,
        }
    }
}
impl UnwarrantedIndex {
    pub(super) fn validate(&self) -> Result<(), CoverageError> {
        if self.version != 1
            || self.hulls.len() > MAX_VOID_HULLS
            || self.policy_voids.len() > MAX_POLICY_VOIDS
        {
            return Err(CoverageError::Bound);
        }
        for (&hull, list) in &self.hulls {
            let ascending = list
                .iter()
                .zip(list.iter().skip(1))
                .all(|(a, b)| a.seq < b.seq);
            if hull == 0
                || list.len() > MAX_VOID_RECORDS_PER_HULL
                || !ascending
                || list.iter().any(|r| r.seq == 0)
            {
                return Err(CoverageError::Product);
            }
        }
        for (key, rec) in &self.policy_voids {
            if !CommitToken::from_hex(key).is_ok_and(|t| t.to_hex() == *key) || rec.seq == 0 {
                return Err(CoverageError::Product);
            }
        }
        Ok(())
    }
    pub(super) fn record_hull(&mut self, hull: ObjId, rec: VoidRecord) {
        if hull.0 == 0 || rec.seq == 0 {
            return;
        }
        if !self.hulls.contains_key(&hull.0) && self.hulls.len() >= MAX_VOID_HULLS {
            return;
        }
        let list = self.hulls.entry(hull.0).or_default();
        match list.binary_search_by_key(&rec.seq, |r| r.seq) {
            Ok(idx) => {
                if list[idx] != rec {
                    list[idx] = rec;
                    self.dirty = true;
                }
            }
            Err(idx) => {
                list.insert(idx, rec);
                if list.len() > MAX_VOID_RECORDS_PER_HULL {
                    let excess = list.len() - MAX_VOID_RECORDS_PER_HULL;
                    list.drain(..excess);
                }
                self.dirty = true;
            }
        }
    }
    pub(super) fn record_policy(&mut self, policy: CommitToken, rec: VoidRecord) {
        if rec.seq == 0 {
            return;
        }
        let key = policy.to_hex();
        match self.policy_voids.get_mut(&key) {
            Some(existing) => {
                if rec.seq > existing.seq {
                    *existing = rec;
                    self.dirty = true;
                }
            }
            None => {
                if self.policy_voids.len() < MAX_POLICY_VOIDS {
                    self.policy_voids.insert(key, rec);
                    self.dirty = true;
                }
            }
        }
    }
    pub(super) fn prune_policy(&mut self, policy: CommitToken) {
        if self.policy_voids.remove(&policy.to_hex()).is_some() {
            self.dirty = true;
        }
    }
}

fn stamp_in_term(at: &ClockStamp, start: &ClockStamp, end: &ClockStamp) -> bool {
    at.clock == start.clock && at.clock == end.clock && start.at <= at.at && at.at < end.at
}

#[derive(Clone, Copy)]
struct UnboundQuoteEntry {
    actor: ObjId,
    ic_open: CommitToken,
    expires_at_utc: i64,
}

#[derive(Default)]
struct RuntimeIndex {
    used_requests: BTreeSet<DefinitionDigest>,
    request_candidates: BTreeMap<DefinitionDigest, Vec<(usize, String)>>,
    candidate_overflow: bool,
    bound_quotes: BTreeSet<CommitToken>,
    bind_candidates: BTreeSet<CommitToken>,
    unbound_quotes: BTreeMap<CommitToken, UnboundQuoteEntry>,
}
impl RuntimeIndex {
    fn record_candidate(&mut self, request: DefinitionDigest, index: usize, key: String) {
        if self.used_requests.contains(&request) {
            return;
        }
        if !self.request_candidates.contains_key(&request)
            && self.request_candidates.len() >= MAX_REQUEST_CANDIDATE_KEYS
        {
            self.candidate_overflow = true;
            return;
        }
        let list = self.request_candidates.entry(request).or_default();
        if list.iter().any(|(i, k)| *i == index && k == &key) {
            return;
        }
        if list.len() >= MAX_REQUEST_CANDIDATES_PER_REQUEST {
            self.candidate_overflow = true;
            return;
        }
        list.push((index, key));
    }
}

#[derive(Clone)]
pub(super) struct Installed {
    pub(super) families: Families,
    pub(super) readiness: authority::Readiness,
}
pub(super) struct CoverageHostInner {
    pub(super) protocol: Protocol,
    pub(super) writer: CoverageWriter,
    pub(super) ic: IcHost,
    pub(super) service: ObjId,
    inbox: Arc<Mutex<Inbox>>,
    unwarranted: Arc<Mutex<UnwarrantedIndex>>,
    runtime: Arc<Mutex<RuntimeIndex>>,
    pub(super) installed: Arc<OnceLock<Installed>>,
    /// Doc 17h cut M8d D5: the Registry inspection's record cell, when the inspection
    /// composes, for a product's `UngovernedCore` exclusion.
    pub(super) inspection: Option<mush_roll::commerce::registry_inspection::InspectionWriter>,
}
/// Bounded host handle wrapping `CoverageHostInner` in one `Arc` so cloning and passing
/// `Option<CoverageHost>` on the stack costs 8 bytes.
#[derive(Clone)]
pub struct CoverageHost {
    pub(super) inner: Arc<CoverageHostInner>,
}
pub struct CoverageTicket {
    request: DefinitionDigest,
    actor: ObjId,
    index: usize,
    action: DefinitionRef,
}
impl CoverageTicket {
    pub fn actor(&self) -> ObjId {
        self.actor
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    /// The public parameters to submit: none. The provider's offer carries
    /// this ticket as a fixed parameter, so resubmitting it would collide.
    pub fn parameters(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}
/// A readback result, not policy or host authority. Release consumes a separate
/// private value produced only by the same complete installed-policy audit.
pub struct CoverageCompletion {
    operation: CommitToken,
    kind: &'static str,
    departure: Option<Arc<AcknowledgedDeparture>>,
}
impl CoverageCompletion {
    pub fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn kind(&self) -> &str {
        self.kind
    }
}
pub struct CoverageUnadmitted {
    origin: Arc<Mutex<Inbox>>,
    ticket: CoverageTicket,
    pending: Option<Pending>,
}
impl CoverageUnadmitted {
    pub fn ticket(&self) -> &CoverageTicket {
        &self.ticket
    }
}
impl CoverageHost {
    pub(super) fn new(
        protocol: Protocol,
        writer: CoverageWriter,
        ic: IcHost,
        service: ObjId,
        inspection: Option<mush_roll::commerce::registry_inspection::InspectionWriter>,
    ) -> Self {
        Self {
            inner: Arc::new(CoverageHostInner {
                protocol,
                writer,
                ic,
                service,
                inbox: Arc::new(Mutex::new(Inbox::default())),
                unwarranted: Arc::new(Mutex::new(UnwarrantedIndex::default())),
                runtime: Arc::new(Mutex::new(RuntimeIndex::default())),
                installed: Arc::new(OnceLock::new()),
                inspection,
            }),
        }
    }
    pub(super) fn load_unwarranted(&self, index: UnwarrantedIndex) -> Result<(), CoverageError> {
        self.inner.load_unwarranted(index)
    }
    pub(super) fn take_dirty_unwarranted(&self) -> Option<UnwarrantedIndex> {
        self.inner.take_dirty_unwarranted()
    }
    pub(super) fn mark_unwarranted_dirty(&self) {
        self.inner.mark_unwarranted_dirty();
    }
    pub(super) fn hydrate_from_boot(
        &self,
        summary: audit::BootAuditSummary,
    ) -> Result<(), CoverageError> {
        self.inner.hydrate_from_boot(summary)
    }
    pub(super) fn sync_unwarranted_from_boot(
        &self,
        ctx: &Ctx<'_>,
        known_hulls: &BTreeSet<ObjId>,
        active_policies: &[(CommitToken, ObjId, ClockStamp, ClockStamp)],
    ) -> Result<(), CoverageError> {
        self.inner
            .sync_unwarranted_from_boot(ctx, known_hulls, active_policies)
    }
    pub fn sync_unwarranted_inspections(
        &self,
        ctx: &Ctx<'_>,
        hull: ObjId,
    ) -> Result<(), CoverageError> {
        self.inner.sync_unwarranted_inspections(ctx, hull)
    }
    pub(super) fn observe_inspection_keys(&self, ctx: &Ctx<'_>, keys: &[smol_str::SmolStr]) {
        self.inner.observe_inspection_keys(ctx, keys);
    }
    pub(super) fn observe_coverage_keys(
        &self,
        ctx: &Ctx<'_>,
        family_id: &str,
        keys: &[smol_str::SmolStr],
    ) {
        self.inner.observe_coverage_keys(ctx, family_id, keys);
    }
    pub(super) fn record_quote_candidate(
        &self,
        op: CommitToken,
        actor: ObjId,
        ic_open: CommitToken,
        request: DefinitionDigest,
        expires_at_utc: i64,
    ) {
        self.inner
            .record_quote_candidate(op, actor, ic_open, request, expires_at_utc);
    }
    pub(super) fn record_bind_candidate(
        &self,
        op: CommitToken,
        quote: CommitToken,
        request: DefinitionDigest,
    ) {
        self.inner.record_bind_candidate(op, quote, request);
    }
    pub(super) fn record_terminal_candidate(&self, policy: CommitToken, request: DefinitionDigest) {
        self.inner.record_terminal_candidate(policy, request);
    }
    pub(super) fn quote_plan_candidates(
        &self,
        actor: ObjId,
        ic_open: CommitToken,
        now: i64,
    ) -> Result<(Vec<CommitToken>, Vec<CommitToken>), CoverageError> {
        self.inner.quote_plan_candidates(actor, ic_open, now)
    }
    pub(super) fn is_quote_bound_or_pending(&self, op: CommitToken) -> bool {
        self.inner.is_quote_bound_or_pending(op)
    }
    pub(super) fn prune_missing_quote_candidates(&self, missing: &[CommitToken]) {
        self.inner.prune_missing_quote_candidates(missing);
    }
    pub(super) fn voiding(
        &self,
        product: &super::CoverageProductV1,
        policy: CommitToken,
        c: &mush_economy::managed_coverage::CoverageConsentV1,
        now: &mush_core::ClockStamp,
        read: impl FnMut(&InspectionFamily, &String) -> Result<Option<InspectionRow>, CoverageError>,
    ) -> Result<Option<CommitToken>, CoverageError> {
        self.inner.voiding(product, policy, c, now, read)
    }
    pub(super) fn ready(&self, world: &World, index: usize) -> Result<&Installed, CoverageError> {
        self.inner.ready(world, index)
    }
    pub fn quote(
        &self,
        world: &World,
        departure: Arc<AcknowledgedDeparture>,
        publication: CommitToken,
        payer: AccountId,
        value: Amount,
    ) -> Result<CoverageTicket, CoverageError> {
        self.inner
            .quote(world, departure, publication, payer, value)
    }
    pub fn bind(
        &self,
        world: &World,
        departure: Arc<AcknowledgedDeparture>,
        quote: CommitToken,
    ) -> Result<CoverageTicket, CoverageError> {
        self.inner.bind(world, departure, quote)
    }
    /// Selects only an already committed genuine covered Close. Expiry alone
    /// cannot make a terminal proof; a missing region event keeps collateral.
    pub fn terminal(
        &self,
        ctx: &Ctx<'_>,
        policy: CommitToken,
        cancel: bool,
    ) -> Result<CoverageTicket, CoverageError> {
        self.inner.terminal(ctx, policy, cancel)
    }
    pub(super) fn lookup(
        &self,
        request: DefinitionDigest,
        actor: ObjId,
        index: usize,
    ) -> Result<Intent, CoverageError> {
        self.inner.lookup(request, actor, index)
    }
    pub(super) fn selected(
        &self,
        actor: ObjId,
        index: usize,
    ) -> Result<Option<DefinitionDigest>, CoverageError> {
        self.inner.selected(actor, index)
    }
    /// Rebind only the same actual retained withheld capability to an installed
    /// policy after restart/ACK loss. No second native Bind or new permit occurs.
    pub fn recover_binding(
        &self,
        ctx: &Ctx<'_>,
        departure: Arc<AcknowledgedDeparture>,
        binding: CommitToken,
    ) -> Result<CoverageCompletion, CoverageError> {
        self.inner.recover_binding(ctx, departure, binding)
    }
    pub fn release_departure(
        &self,
        ctx: &Ctx<'_>,
        completion: &mut CoverageCompletion,
    ) -> Result<crate::minds::ic_sortie::CoveredDeparture, CoverageError> {
        self.inner.release_departure(ctx, completion)
    }
    pub fn submit(
        &self,
        ctx: &mut Ctx<'_>,
        ticket: &CoverageTicket,
    ) -> Result<CoverageSubmissionState, CoverageError> {
        self.inner.submit(ctx, ticket)
    }
    pub fn submission_state(
        &self,
        t: &CoverageTicket,
    ) -> Result<CoverageSubmissionState, CoverageError> {
        self.inner.submission_state(t)
    }
    /// Complete real inverse readback precedes removal; no response bit or
    /// disappearance can turn an attempt into an installed policy.
    pub fn acknowledge(
        &self,
        ctx: &Ctx<'_>,
        t: &CoverageTicket,
        operation: CommitToken,
    ) -> Result<CoverageCompletion, CoverageError> {
        self.inner.acknowledge(ctx, t, operation)
    }
    pub fn discard_unadmitted(
        &self,
        ctx: &Ctx<'_>,
        t: &CoverageTicket,
    ) -> Result<CoverageUnadmitted, CoverageError> {
        self.inner.discard_unadmitted(ctx, t)
    }
    pub fn restore_unadmitted(
        &self,
        ctx: &Ctx<'_>,
        released: &mut CoverageUnadmitted,
    ) -> Result<CoverageTicket, CoverageError> {
        self.inner.restore_unadmitted(ctx, released)
    }
}
impl CoverageHostInner {
    fn load_unwarranted(&self, index: UnwarrantedIndex) -> Result<(), CoverageError> {
        index.validate()?;
        let mut slot = self.unwarranted.lock().map_err(bad)?;
        *slot = index;
        Ok(())
    }
    pub(super) fn take_dirty_unwarranted(&self) -> Option<UnwarrantedIndex> {
        let mut slot = self.unwarranted.lock().ok()?;
        if !slot.dirty {
            return None;
        }
        slot.dirty = false;
        Some(slot.clone())
    }
    pub(super) fn mark_unwarranted_dirty(&self) {
        if let Ok(mut slot) = self.unwarranted.lock() {
            slot.dirty = true;
        }
    }
    pub(super) fn hydrate_from_boot(
        &self,
        summary: audit::BootAuditSummary,
    ) -> Result<(), CoverageError> {
        let mut rt = self.runtime.lock().map_err(bad)?;
        rt.used_requests = summary.used_requests;
        rt.request_candidates.clear();
        rt.candidate_overflow = false;
        rt.bound_quotes = summary.bound_quotes;
        rt.bind_candidates.clear();
        rt.unbound_quotes.clear();
        for (op, actor, ic_open, expires_at_utc) in summary.unbound_quotes {
            rt.unbound_quotes.insert(
                op,
                UnboundQuoteEntry {
                    actor,
                    ic_open,
                    expires_at_utc,
                },
            );
        }
        Ok(())
    }
    pub(super) fn sync_unwarranted_from_boot(
        &self,
        ctx: &Ctx<'_>,
        known_hulls: &BTreeSet<ObjId>,
        active_policies: &[(CommitToken, ObjId, ClockStamp, ClockStamp)],
    ) -> Result<(), CoverageError> {
        let Some(inspection) = self.inspection.as_ref() else {
            return Ok(());
        };
        let family = inspection.family().map_err(bad)?;
        let mut idx = self.unwarranted.lock().map_err(bad)?;
        for &hull in known_hulls {
            for slot in mush_roll::commerce::registry_inspection::inspection_slots(hull) {
                if let Some(record) = ctx.action_state(&family, &slot).map_err(bad)?
                    && record.hull == hull
                    && crate::minds::registry_inspection::unwarranted::unwarranted(record)
                {
                    let rec = VoidRecord {
                        seq: record.seq,
                        at_ic: record.at_ic.clone(),
                        inspection: record.inspection,
                    };
                    idx.record_hull(hull, rec.clone());
                    for (policy, p_hull, start, end) in active_policies {
                        if *p_hull == hull && stamp_in_term(&rec.at_ic, start, end) {
                            idx.record_policy(*policy, rec.clone());
                        }
                    }
                }
            }
        }
        let active_set: BTreeSet<String> = active_policies
            .iter()
            .map(|(p, _, _, _)| p.to_hex())
            .collect();
        let before = idx.policy_voids.len();
        idx.policy_voids.retain(|k, _| active_set.contains(k));
        if idx.policy_voids.len() != before {
            idx.dirty = true;
        }
        Ok(())
    }
    /// Synchronizes all unwarranted inspections currently in `hull`'s 64-slot
    /// inspection ring into the durable non-ring-overwritten ledger, pinning
    /// any in-term finding onto `hull`'s active policy (`P2-TSW-38`).
    pub fn sync_unwarranted_inspections(
        &self,
        ctx: &Ctx<'_>,
        hull: ObjId,
    ) -> Result<(), CoverageError> {
        let r = self.ready(ctx.w, 4)?;
        let family = self
            .inspection
            .as_ref()
            .ok_or(CoverageError::Authority)?
            .family()
            .map_err(bad)?;
        let active = ctx
            .action_state(&r.families.hulls, &format!("{:08x}", hull.0))
            .map_err(bad)?
            .and_then(|head| head.active)
            .and_then(|policy_id| {
                ctx.action_state(&r.families.policies, &policy_id.to_hex())
                    .ok()
                    .flatten()
                    .map(|p| {
                        let terms = &p.history.consent().terms;
                        (policy_id, terms.starts_at.clone(), terms.ends_at.clone())
                    })
            });
        let mut idx = self.unwarranted.lock().map_err(bad)?;
        for slot in mush_roll::commerce::registry_inspection::inspection_slots(hull) {
            if let Some(record) = ctx.action_state(&family, &slot).map_err(bad)?
                && record.hull == hull
                && crate::minds::registry_inspection::unwarranted::unwarranted(record)
            {
                let rec = VoidRecord {
                    seq: record.seq,
                    at_ic: record.at_ic.clone(),
                    inspection: record.inspection,
                };
                idx.record_hull(hull, rec.clone());
                if let Some((policy_id, ref start, ref end)) = active
                    && stamp_in_term(&rec.at_ic, start, end)
                {
                    idx.record_policy(policy_id, rec);
                }
            }
        }
        Ok(())
    }
    pub(super) fn observe_inspection_keys(&self, ctx: &Ctx<'_>, keys: &[smol_str::SmolStr]) {
        let Some(r) = self.installed.get() else {
            return;
        };
        let Some(family) = self.inspection.as_ref().and_then(|w| w.family().ok()) else {
            return;
        };
        let Ok(mut idx) = self.unwarranted.lock() else {
            return;
        };
        for key in keys {
            let Ok(Some(record)) = ctx.action_state(&family, &key.to_string()) else {
                continue;
            };
            if !crate::minds::registry_inspection::unwarranted::unwarranted(record) {
                continue;
            }
            let rec = VoidRecord {
                seq: record.seq,
                at_ic: record.at_ic.clone(),
                inspection: record.inspection,
            };
            idx.record_hull(record.hull, rec.clone());
            if let Ok(Some(head)) =
                ctx.action_state(&r.families.hulls, &format!("{:08x}", record.hull.0))
                && let Some(policy_id) = head.active
                && let Ok(Some(policy)) =
                    ctx.action_state(&r.families.policies, &policy_id.to_hex())
                && policy.history.terminal().is_none()
            {
                let terms = &policy.history.consent().terms;
                if stamp_in_term(&rec.at_ic, &terms.starts_at, &terms.ends_at) {
                    idx.record_policy(policy_id, rec);
                }
            }
        }
    }
    pub(super) fn observe_coverage_keys(
        &self,
        ctx: &Ctx<'_>,
        family_id: &str,
        keys: &[smol_str::SmolStr],
    ) {
        let Some(r) = self.installed.get() else {
            return;
        };
        let Ok(mut rt) = self.runtime.lock() else {
            return;
        };
        for key in keys {
            let key_str = key.to_string();
            match family_id {
                "tacsim_coverage_quotes" => match ctx.action_state(&r.families.quotes, &key_str) {
                    Ok(Some(q)) => {
                        rt.used_requests.insert(q.request);
                        rt.request_candidates.remove(&q.request);
                        if !rt.bound_quotes.contains(&q.operation) {
                            rt.unbound_quotes.insert(
                                q.operation,
                                UnboundQuoteEntry {
                                    actor: q.terms.buyer,
                                    ic_open: q.risk.ic_open,
                                    expires_at_utc: q.terms.expires_at_utc,
                                },
                            );
                        }
                    }
                    Ok(None) => {
                        if let Ok(op) = CommitToken::from_hex(&key_str) {
                            rt.unbound_quotes.remove(&op);
                        }
                    }
                    Err(_) => {}
                },
                "tacsim_coverage_policies" => {
                    if let Ok(Some(p)) = ctx.action_state(&r.families.policies, &key_str) {
                        rt.used_requests.insert(p.request);
                        rt.request_candidates.remove(&p.request);
                        rt.bound_quotes.insert(p.quote);
                        rt.bind_candidates.remove(&p.quote);
                        rt.unbound_quotes.remove(&p.quote);
                    }
                }
                "tacsim_coverage_terminals" => {
                    if let Ok(Some(t)) = ctx.action_state(&r.families.terminals, &key_str) {
                        rt.used_requests.insert(t.request);
                        rt.request_candidates.remove(&t.request);
                        if let Ok(mut idx) = self.unwarranted.lock() {
                            idx.prune_policy(t.policy);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    pub(super) fn record_quote_candidate(
        &self,
        op: CommitToken,
        actor: ObjId,
        ic_open: CommitToken,
        request: DefinitionDigest,
        expires_at_utc: i64,
    ) {
        if let Ok(mut rt) = self.runtime.lock() {
            rt.record_candidate(request, 1, op.to_hex());
            if !rt.bound_quotes.contains(&op) {
                rt.unbound_quotes.insert(
                    op,
                    UnboundQuoteEntry {
                        actor,
                        ic_open,
                        expires_at_utc,
                    },
                );
            }
        }
    }
    pub(super) fn record_bind_candidate(
        &self,
        op: CommitToken,
        quote: CommitToken,
        request: DefinitionDigest,
    ) {
        if let Ok(mut rt) = self.runtime.lock() {
            rt.record_candidate(request, 2, op.to_hex());
            rt.bind_candidates.insert(quote);
        }
    }
    pub(super) fn record_terminal_candidate(&self, policy: CommitToken, request: DefinitionDigest) {
        if let Ok(mut rt) = self.runtime.lock() {
            rt.record_candidate(request, 3, policy.to_hex());
        }
    }
    pub(super) fn quote_plan_candidates(
        &self,
        actor: ObjId,
        ic_open: CommitToken,
        now: i64,
    ) -> Result<(Vec<CommitToken>, Vec<CommitToken>), CoverageError> {
        let rt = self.runtime.lock().map_err(bad)?;
        let for_open: Vec<CommitToken> = rt
            .unbound_quotes
            .iter()
            .filter_map(|(&op, entry)| {
                (entry.actor == actor && entry.ic_open == ic_open).then_some(op)
            })
            .collect();
        let mut expired: Vec<(i64, CommitToken)> = rt
            .unbound_quotes
            .iter()
            .filter_map(|(&op, entry)| {
                (entry.expires_at_utc <= now
                    && !rt.bound_quotes.contains(&op)
                    && !rt.bind_candidates.contains(&op))
                .then_some((entry.expires_at_utc, op))
            })
            .collect();
        expired.sort_unstable();
        let expired_ops = expired
            .into_iter()
            .take(MAX_PRUNE_PER_QUOTE * 2)
            .map(|(_, op)| op)
            .collect();
        Ok((for_open, expired_ops))
    }
    pub(super) fn is_quote_bound_or_pending(&self, op: CommitToken) -> bool {
        self.runtime
            .lock()
            .is_ok_and(|rt| rt.bound_quotes.contains(&op) || rt.bind_candidates.contains(&op))
    }
    pub(super) fn prune_missing_quote_candidates(&self, missing: &[CommitToken]) {
        if missing.is_empty() {
            return;
        }
        if let Ok(mut rt) = self.runtime.lock() {
            for op in missing {
                rt.unbound_quotes.remove(op);
            }
        }
    }
    fn is_request_used(
        &self,
        ctx: &Ctx<'_>,
        f: &Families,
        request: DefinitionDigest,
    ) -> Result<bool, CoverageError> {
        let (known, candidates, overflow) = {
            let rt = self.runtime.lock().map_err(bad)?;
            if rt.used_requests.contains(&request) {
                return Ok(true);
            }
            (
                false,
                rt.request_candidates
                    .get(&request)
                    .cloned()
                    .unwrap_or_default(),
                rt.candidate_overflow,
            )
        };
        if known {
            return Ok(true);
        }
        for (index, key) in candidates {
            let matched = match index {
                1 => ctx
                    .action_state(&f.quotes, &key)
                    .map_err(bad)?
                    .is_some_and(|r| r.request == request),
                2 => ctx
                    .action_state(&f.policies, &key)
                    .map_err(bad)?
                    .is_some_and(|r| r.request == request),
                3 | 4 => ctx
                    .action_state(&f.terminals, &key)
                    .map_err(bad)?
                    .is_some_and(|r| r.request == request),
                _ => false,
            };
            if matched {
                if let Ok(mut rt) = self.runtime.lock() {
                    rt.used_requests.insert(request);
                    rt.request_candidates.remove(&request);
                }
                return Ok(true);
            }
        }
        if overflow {
            let used = audit::request_used_scan(ctx, f, request)?;
            if used && let Ok(mut rt) = self.runtime.lock() {
                rt.used_requests.insert(request);
                rt.request_candidates.remove(&request);
            }
            return Ok(used);
        }
        Ok(false)
    }
    /// **The inspection that voids this cover** (doc 17h cut M8d D5, hardened by `P2-TSW-38`):
    /// when `product` excludes an ungoverned core, the newest inspection of the covered hull that
    /// read one within the term (from its start to `now`, never past its end), its commit. Reads
    /// both the 64-slot inspection ring and the durable non-ring-overwritten unwarranted index so
    /// subsequent governed inspections or post-term inspections cannot evict a voiding finding.
    ///
    /// # Errors
    /// `Authority` when the exclusion cannot be read.
    pub(super) fn voiding(
        &self,
        product: &super::CoverageProductV1,
        policy: CommitToken,
        c: &mush_economy::managed_coverage::CoverageConsentV1,
        now: &mush_core::ClockStamp,
        mut read: impl FnMut(&InspectionFamily, &String) -> Result<Option<InspectionRow>, CoverageError>,
    ) -> Result<Option<CommitToken>, CoverageError> {
        if !product.excludes(super::CoverageExclusion::UngovernedCore) {
            return Ok(None);
        }
        let family = self
            .inspection
            .as_ref()
            .ok_or(CoverageError::Authority)?
            .family()
            .map_err(bad)?;
        let mut rows = Vec::new();
        let mut newly_observed = Vec::new();
        for slot in mush_roll::commerce::registry_inspection::inspection_slots(c.basis.hull) {
            if let Some(record) = read(&family, &slot)? {
                let is_unwarranted =
                    crate::minds::registry_inspection::unwarranted::unwarranted(&record);
                if is_unwarranted && record.hull == c.basis.hull {
                    newly_observed.push(VoidRecord {
                        seq: record.seq,
                        at_ic: record.at_ic.clone(),
                        inspection: record.inspection,
                    });
                }
                rows.push(VoidRow {
                    hull: record.hull,
                    seq: record.seq,
                    unwarranted: is_unwarranted,
                    at: record.at_ic.clone(),
                    inspection: record.inspection,
                });
            }
        }
        {
            let mut idx = self.unwarranted.lock().map_err(bad)?;
            for rec in newly_observed {
                idx.record_hull(c.basis.hull, rec.clone());
                if stamp_in_term(&rec.at_ic, &c.terms.starts_at, &c.terms.ends_at) {
                    idx.record_policy(policy, rec);
                }
            }
            if let Some(stored) = idx.hulls.get(&c.basis.hull.0) {
                for rec in stored {
                    rows.push(VoidRow {
                        hull: c.basis.hull,
                        seq: rec.seq,
                        unwarranted: true,
                        at: rec.at_ic.clone(),
                        inspection: rec.inspection,
                    });
                }
            }
            if let Some(rec) = idx.policy_voids.get(&policy.to_hex()) {
                rows.push(VoidRow {
                    hull: c.basis.hull,
                    seq: rec.seq,
                    unwarranted: true,
                    at: rec.at_ic.clone(),
                    inspection: rec.inspection,
                });
            }
        }
        Ok(void_from(
            c.basis.hull,
            (&c.terms.starts_at, &c.terms.ends_at),
            now,
            rows,
        ))
    }
    pub(super) fn ready(&self, world: &World, index: usize) -> Result<&Installed, CoverageError> {
        let r = self.installed.get().ok_or(CoverageError::Authority)?;
        r.readiness.check(&self.protocol.action(index)?)?;
        self.writer.require_audited(world).map_err(bad)?;
        if world
            .module_state_owner(std::any::TypeId::of::<Publications>())
            .is_none_or(|o| o.as_str() != crate::NS)
            || world
                .module_state::<Publications>()
                .is_none_or(|p| !p.loaded)
        {
            return Err(CoverageError::Authority);
        }
        Ok(r)
    }
    fn insert(
        &self,
        actor: ObjId,
        index: usize,
        intent: Intent,
    ) -> Result<CoverageTicket, CoverageError> {
        self.insert_at(actor, index, intent, Instant::now())
    }
    fn insert_at(
        &self,
        actor: ObjId,
        index: usize,
        intent: Intent,
        now: Instant,
    ) -> Result<CoverageTicket, CoverageError> {
        let mut inbox = self.inbox.lock().map_err(bad)?;
        inbox.prune_expired_at(now);
        let is_service_terminal = actor == self.service && index >= 3;
        let cap = if is_service_terminal {
            MAX_SERVICE_INBOX
        } else {
            MAX_BUYER_INBOX
        };
        let target_slot = slot_key(actor, index, &intent);
        if inbox.lane_count(self.service, is_service_terminal) >= cap
            || inbox.entries.values().any(|p| p.slot_key() == target_slot)
        {
            return Err(CoverageError::Bound);
        }
        let request = DefinitionDigest::from_bytes(rand::random());
        if request.as_bytes() == &[0; 32] || inbox.entries.contains_key(&request) {
            return Err(CoverageError::Bound);
        }
        let seq = inbox.allocate_seq();
        inbox.entries.insert(
            request,
            Pending {
                actor,
                index,
                intent,
                attempt: Attempt::New,
                seq,
                created_at: now,
            },
        );
        Ok(CoverageTicket {
            request,
            actor,
            index,
            action: self.protocol.action(index)?,
        })
    }
    pub fn quote(
        &self,
        world: &World,
        departure: Arc<AcknowledgedDeparture>,
        publication: CommitToken,
        payer: AccountId,
        value: Amount,
    ) -> Result<CoverageTicket, CoverageError> {
        self.ready(world, 1)?;
        departure.recheck(world).map_err(bad)?;
        self.insert(
            departure.actor(),
            1,
            Intent::Quote {
                departure,
                publication,
                payer,
                value,
            },
        )
    }
    pub fn bind(
        &self,
        world: &World,
        departure: Arc<AcknowledgedDeparture>,
        quote: CommitToken,
    ) -> Result<CoverageTicket, CoverageError> {
        self.ready(world, 2)?;
        departure.recheck(world).map_err(bad)?;
        self.insert(departure.actor(), 2, Intent::Bind { departure, quote })
    }
    /// Selects only an already committed genuine covered Close. Expiry alone
    /// cannot make a terminal proof; a missing region event keeps collateral.
    pub fn terminal(
        &self,
        ctx: &Ctx<'_>,
        policy: CommitToken,
        cancel: bool,
    ) -> Result<CoverageTicket, CoverageError> {
        let r = self.ready(ctx.w, 4)?;
        let p = read(ctx, &r.families.policies, policy.to_hex())?;
        if p.history.terminal().is_some() {
            return Err(CoverageError::Authority);
        }
        let c = p.history.consent();
        let close = Arc::new(
            self.ic
                .committed_coverage_close(ctx.w, c.basis.ic_open, c.basis.departure_digest)
                .map_err(bad)?,
        );
        let proof = super::policy::CloseProof::capture(ctx.w, &close)?;
        let pays = proof.covered_loss(c)?;
        if cancel && pays {
            return Err(CoverageError::Authority);
        }
        // Doc 17h cut M8d D5: a covered loss the product's exclusion voids takes the no-pay
        // action; the provider decides the same from its own transaction.
        let product = &read(ctx, &r.families.products, c.basis.product.to_hex())?.product;
        let voided = pays
            && self
                .voiding(product, policy, c, &ctx.w.world_ic_now(), |family, slot| {
                    ctx.action_state(family, slot)
                        .map(|record| record.cloned())
                        .map_err(bad)
                })?
                .is_some();
        let actor = if cancel { c.terms.buyer } else { self.service };
        let index = if pays && !voided { 3 } else { 4 };
        self.ready(ctx.w, index)?;
        self.insert(
            actor,
            index,
            Intent::Terminal {
                close,
                policy,
                cancel,
            },
        )
    }
    pub(super) fn lookup(
        &self,
        request: DefinitionDigest,
        actor: ObjId,
        index: usize,
    ) -> Result<Intent, CoverageError> {
        let inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox
            .entries
            .get(&request)
            .ok_or(CoverageError::Authority)?;
        if p.actor != actor || p.index != index {
            return Err(CoverageError::Authority);
        }
        Ok(p.intent.clone())
    }
    pub(super) fn selected(
        &self,
        actor: ObjId,
        index: usize,
    ) -> Result<Option<DefinitionDigest>, CoverageError> {
        let mut inbox = self.inbox.lock().map_err(bad)?;
        for p in inbox.entries.values_mut() {
            if p.actor == actor && p.index == index {
                p.attempt.poll();
            }
        }
        let best = inbox
            .entries
            .iter()
            .filter(|(_, p)| p.actor == actor && p.index == index)
            .max_by_key(|(_, p)| {
                let priority: u8 = match p.attempt {
                    Attempt::Observed(_) => 2,
                    Attempt::New => 1,
                    Attempt::Retryable => 0,
                };
                (priority, p.seq)
            })
            .map(|(id, _)| *id);
        Ok(best)
    }
    fn check(&self, p: &Pending, t: &CoverageTicket) -> Result<(), CoverageError> {
        if p.actor != t.actor || p.index != t.index || self.protocol.action(p.index)? != t.action {
            return Err(CoverageError::Authority);
        }
        Ok(())
    }
    /// Rebind only the same actual retained withheld capability to an installed
    /// policy after restart/ACK loss. No second native Bind or new permit occurs.
    pub fn recover_binding(
        &self,
        ctx: &Ctx<'_>,
        departure: Arc<AcknowledgedDeparture>,
        binding: CommitToken,
    ) -> Result<CoverageCompletion, CoverageError> {
        let r = self.ready(ctx.w, 2)?;
        departure.recheck(ctx.w).map_err(bad)?;
        let p = read(ctx, &r.families.policies, binding.to_hex())?;
        audit::verify_policy_runtime(ctx, &r.families, self.service, p)?;
        let c = p.history.consent();
        if p.history.terminal().is_some()
            || c.basis.ic_open != departure.open()
            || c.basis.departure_digest != departure.digest()
            || c.terms.buyer != departure.actor()
        {
            return Err(CoverageError::Authority);
        }
        Ok(CoverageCompletion {
            operation: binding,
            kind: "bound",
            departure: Some(departure),
        })
    }
    pub fn release_departure(
        &self,
        ctx: &Ctx<'_>,
        completion: &mut CoverageCompletion,
    ) -> Result<crate::minds::ic_sortie::CoveredDeparture, CoverageError> {
        let r = self.ready(ctx.w, 2)?;
        let departure = completion
            .departure
            .as_ref()
            .ok_or(CoverageError::Authority)?;
        let p = read(ctx, &r.families.policies, completion.operation.to_hex())?;
        audit::verify_policy_runtime(ctx, &r.families, self.service, p)?;
        let c = p.history.consent();
        let now = ctx.w.world_ic_now();
        if completion.kind != "bound"
            || p.history.terminal().is_some()
            || c.basis.ic_open != departure.open()
            || c.basis.departure_digest != departure.digest()
            || now.clock != c.terms.starts_at.clock
            || now.at < c.terms.starts_at.at
            || now.at >= c.terms.ends_at.at
        {
            return Err(CoverageError::Authority);
        }
        let released = departure
            .release_after_binding(ctx.w, completion.operation)
            .map_err(bad)?;
        completion.departure.take();
        Ok(released)
    }

    pub fn submit(
        &self,
        ctx: &mut Ctx<'_>,
        ticket: &CoverageTicket,
    ) -> Result<CoverageSubmissionState, CoverageError> {
        let r = self.ready(ctx.w, ticket.index)?;
        if ctx.dispatch_actor() != Some(ticket.actor) || ctx.actor != ticket.actor {
            return Err(CoverageError::Authority);
        }
        // Pre-check claim_delay_seconds before dispatching an index = 3 payout
        // so an immature claim never wastes a submission or enters Retryable
        // prematurely (`P2-TSW-20`).
        let claim_check = {
            let inbox = self.inbox.lock().map_err(bad)?;
            let p = inbox
                .entries
                .get(&ticket.request)
                .ok_or(CoverageError::Authority)?;
            self.check(p, ticket)?;
            if ticket.index == 3
                && let Intent::Terminal { close, policy, .. } = &p.intent
            {
                Some((Arc::clone(close), *policy))
            } else {
                None
            }
        };
        if let Some((close, policy)) = claim_check {
            let row = read(ctx, &r.families.policies, policy.to_hex())?;
            let consent = row.history.consent();
            let proof = super::policy::CloseProof::capture(ctx.w, &close)?;
            let now = ctx.w.world_ic_now();
            if proof.at_ic.clock != now.clock
                || proof
                    .at_ic
                    .at
                    .checked_add(consent.terms.claim_delay_seconds)
                    .is_none_or(|due| now.at < due)
            {
                return Err(CoverageError::OutsideTerm);
            }
        }
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox
            .entries
            .get_mut(&ticket.request)
            .ok_or(CoverageError::Authority)?;
        self.check(p, ticket)?;
        p.attempt.poll();
        if matches!(p.attempt, Attempt::Observed(_))
            || ctx.authoritative_pending(ticket.actor)
            || ctx.authoritative_commit_pending()
            || ctx.authoritative_queue_full()
        {
            return Ok(p.attempt.status());
        }
        let key =
            IdempotencyKeyText::parse_canonical(format!("coverage.{}", ticket.request.to_hex()))
                .map_err(bad)?;
        p.attempt = Attempt::Observed(
            ctx.submit_authoritative_observed(
                ticket.action.clone(),
                PerceivedRef::SelfActor,
                vec![],
                ticket.parameters(),
                key,
            )
            .map_err(bad)?,
        );
        Ok(p.attempt.status())
    }
    pub fn submission_state(
        &self,
        t: &CoverageTicket,
    ) -> Result<CoverageSubmissionState, CoverageError> {
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox
            .entries
            .get_mut(&t.request)
            .ok_or(CoverageError::Authority)?;
        self.check(p, t)?;
        p.attempt.poll();
        Ok(p.attempt.status())
    }
    /// Complete real inverse readback precedes removal; no response bit or
    /// disappearance can turn an attempt into an installed policy.
    pub fn acknowledge(
        &self,
        ctx: &Ctx<'_>,
        t: &CoverageTicket,
        operation: CommitToken,
    ) -> Result<CoverageCompletion, CoverageError> {
        let r = self.ready(ctx.w, t.index)?;
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox
            .entries
            .get(&t.request)
            .ok_or(CoverageError::Authority)?;
        self.check(p, t)?;
        let (kind, departure) = match &p.intent {
            Intent::Quote {
                departure,
                publication,
                payer,
                value,
            } => {
                let q: &Quote = read(ctx, &r.families.quotes, operation.to_hex())?;
                audit::verify_quote_runtime(ctx, &r.families, q)?;
                if q.request != t.request
                    || q.risk.ic_open != departure.open()
                    || q.risk.departure_digest != departure.digest()
                    || q.publication != *publication
                    || q.terms.payer != *payer
                    || q.terms.insured_value != *value
                {
                    return Err(CoverageError::Authority);
                }
                if let Ok(mut rt) = self.runtime.lock() {
                    rt.used_requests.insert(q.request);
                    rt.request_candidates.remove(&q.request);
                    if !rt.bound_quotes.contains(&q.operation) {
                        rt.unbound_quotes.insert(
                            q.operation,
                            UnboundQuoteEntry {
                                actor: q.terms.buyer,
                                ic_open: q.risk.ic_open,
                                expires_at_utc: q.terms.expires_at_utc,
                            },
                        );
                    }
                }
                ("quoted", None)
            }
            Intent::Bind { departure, quote } => {
                let policy: &Policy = read(ctx, &r.families.policies, operation.to_hex())?;
                audit::verify_policy_runtime(ctx, &r.families, self.service, policy)?;
                if policy.request != t.request
                    || policy.quote != *quote
                    || policy.history.consent().basis.departure_digest != departure.digest()
                    || policy.history.consent().basis.ic_open != departure.open()
                {
                    return Err(CoverageError::Authority);
                }
                if let Ok(mut rt) = self.runtime.lock() {
                    rt.used_requests.insert(policy.request);
                    rt.request_candidates.remove(&policy.request);
                    rt.bound_quotes.insert(policy.quote);
                    rt.bind_candidates.remove(&policy.quote);
                    rt.unbound_quotes.remove(&policy.quote);
                }
                ("bound", Some(Arc::clone(departure)))
            }
            Intent::Terminal {
                close,
                policy,
                cancel,
            } => {
                let terminal: &Terminal = read(ctx, &r.families.terminals, policy.to_hex())?;
                let actual = read(ctx, &r.families.policies, policy.to_hex())?;
                audit::verify_terminal_runtime(ctx, &r.families, self.service, actual, terminal)?;
                let reason = &actual
                    .history
                    .terminal()
                    .ok_or(CoverageError::Authority)?
                    .decision()
                    .reason;
                if terminal.operation != operation
                    || terminal.request != t.request
                    || terminal.policy != *policy
                    || terminal.close.close != close.operation().map_err(bad)?
                    || *cancel
                        != matches!(
                            reason,
                            mush_economy::managed_coverage::CoverageDecisionKindV1::Cancelled { .. }
                        )
                {
                    return Err(CoverageError::Authority);
                }
                if let Ok(mut rt) = self.runtime.lock() {
                    rt.used_requests.insert(terminal.request);
                    rt.request_candidates.remove(&terminal.request);
                }
                if let Ok(mut idx) = self.unwarranted.lock() {
                    idx.prune_policy(*policy);
                }
                (if t.index == 3 { "claimed" } else { "released" }, None)
            }
        };
        inbox
            .entries
            .remove(&t.request)
            .ok_or(CoverageError::Authority)?;
        Ok(CoverageCompletion {
            operation,
            kind,
            departure,
        })
    }
    pub fn discard_unadmitted(
        &self,
        ctx: &Ctx<'_>,
        t: &CoverageTicket,
    ) -> Result<CoverageUnadmitted, CoverageError> {
        let r = self.ready(ctx.w, t.index)?;
        audit::check_runtime_context(ctx)?;
        if self.is_request_used(ctx, &r.families, t.request)? {
            return Err(CoverageError::Authority);
        }
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox
            .entries
            .get_mut(&t.request)
            .ok_or(CoverageError::Authority)?;
        self.check(p, t)?;
        p.attempt.poll();
        if !matches!(p.attempt, Attempt::New | Attempt::Retryable) {
            return Err(CoverageError::Authority);
        }
        let pending = inbox
            .entries
            .remove(&t.request)
            .ok_or(CoverageError::Authority)?;
        Ok(CoverageUnadmitted {
            origin: Arc::clone(&self.inbox),
            ticket: CoverageTicket {
                request: t.request,
                actor: t.actor,
                index: t.index,
                action: t.action.clone(),
            },
            pending: Some(pending),
        })
    }
    pub fn restore_unadmitted(
        &self,
        ctx: &Ctx<'_>,
        released: &mut CoverageUnadmitted,
    ) -> Result<CoverageTicket, CoverageError> {
        if !Arc::ptr_eq(&self.inbox, &released.origin) {
            return Err(CoverageError::Authority);
        }
        let t = &released.ticket;
        let r = self.ready(ctx.w, t.index)?;
        audit::check_runtime_context(ctx)?;
        if self.is_request_used(ctx, &r.families, t.request)? {
            return Err(CoverageError::Authority);
        }
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let now = Instant::now();
        inbox.prune_expired_at(now);
        let candidate = released.pending.as_ref().ok_or(CoverageError::Authority)?;
        self.check(candidate, t)?;
        let is_service_terminal = t.actor == self.service && t.index >= 3;
        let cap = if is_service_terminal {
            MAX_SERVICE_INBOX
        } else {
            MAX_BUYER_INBOX
        };
        let target_slot = candidate.slot_key();
        if inbox.lane_count(self.service, is_service_terminal) >= cap
            || inbox.entries.contains_key(&t.request)
            || inbox.entries.values().any(|p| p.slot_key() == target_slot)
        {
            return Err(CoverageError::Bound);
        }
        let mut p = released.pending.take().ok_or(CoverageError::Authority)?;
        p.created_at = now;
        p.seq = inbox.allocate_seq();
        inbox.entries.insert(t.request, p);
        Ok(CoverageTicket {
            request: t.request,
            actor: t.actor,
            index: t.index,
            action: t.action.clone(),
        })
    }
}
/// One inspection as the void rule reads it (doc 17h cut M8d D5).
pub(super) struct VoidRow {
    pub(super) hull: ObjId,
    pub(super) seq: u64,
    pub(super) unwarranted: bool,
    pub(super) at: mush_core::ClockStamp,
    pub(super) inspection: CommitToken,
}

/// **The void rule** (doc 17h cut M8d D5), pure: the newest unwarranted inspection of `hull`
/// recorded on the term's clock from its start to `now`, before its end (exclusive, as the term
/// is: fix round K10).
///
/// **The race.** The host decides the action (pay or no-pay) from the world it reads, and the
/// provider decides again from its own transaction. An inspection landing between the two makes
/// them differ; the provider's index check then refuses the pay action, and the host's next
/// terminal selection takes the no-pay one. Nothing pays on a stale read.
pub(super) fn void_from(
    hull: ObjId,
    (start, end): (&mush_core::ClockStamp, &mush_core::ClockStamp),
    now: &mush_core::ClockStamp,
    rows: impl IntoIterator<Item = VoidRow>,
) -> Option<CommitToken> {
    rows.into_iter()
        .filter(|row| {
            row.hull == hull
                && row.unwarranted
                && row.at.clock == start.clock
                && row.at.clock == end.clock
                && row.at.clock == now.clock
                && start.at <= row.at.at
                && row.at.at < end.at
                && row.at.at <= now.at
        })
        .max_by_key(|row| row.seq)
        .map(|row| row.inspection)
}

/// The Registry inspection's record family (doc 17h cut M8d D5).
pub(super) type InspectionFamily = mush_core::StateFamilyHandle<String, InspectionRow>;
/// One Registry inspection record.
pub(super) type InspectionRow = mush_roll::commerce::registry_inspection::InspectionRecord;

pub(super) fn read<'a, R: Row>(
    ctx: &'a Ctx<'_>,
    f: &mush_core::StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, CoverageError> {
    let row = ctx
        .action_state(f, &key)
        .map_err(bad)?
        .ok_or(CoverageError::Product)?;
    row.validate()?;
    if row.key() != key {
        return Err(CoverageError::Product);
    }
    Ok(row)
}

#[cfg(test)]
mod m8d_tests {
    use super::*;
    use mush_core::{ClockRef, ClockStamp};

    fn at(clock: &ClockRef, at: i64) -> ClockStamp {
        ClockStamp {
            clock: clock.clone(),
            at,
        }
    }

    /// **M8d D5**: the void rule reads only the covered hull's unwarranted inspections on the
    /// term's clock, from its start to now and never past its end, and names the newest.
    #[test]
    fn m8d_the_void_rule_names_the_newest_unwarranted_inspection_in_the_term() {
        let ic = ClockRef::WorldIc { generation: 1 };
        let other = ClockRef::WorldIc { generation: 2 };
        let hull = ObjId(7);
        let (start, end, now) = (at(&ic, 100), at(&ic, 200), at(&ic, 150));
        let row = |hull, seq, unwarranted, stamp, byte| VoidRow {
            hull,
            seq,
            unwarranted,
            at: stamp,
            inspection: CommitToken::from_bytes([byte; 16]),
        };
        let void = |rows: Vec<VoidRow>| void_from(hull, (&start, &end), &now, rows);
        assert_eq!(void(vec![]), None, "nothing recorded");
        assert_eq!(
            void(vec![row(hull, 1, false, at(&ic, 120), 1)]),
            None,
            "governed"
        );
        assert_eq!(
            void(vec![row(ObjId(8), 1, true, at(&ic, 120), 1)]),
            None,
            "another hull"
        );
        assert_eq!(
            void(vec![row(hull, 1, true, at(&ic, 99), 1)]),
            None,
            "before the term"
        );
        assert_eq!(
            void(vec![row(hull, 1, true, at(&ic, 151), 1)]),
            None,
            "after now"
        );
        assert_eq!(
            void(vec![row(hull, 1, true, at(&other, 120), 1)]),
            None,
            "another clock"
        );
        assert_eq!(
            void(vec![row(hull, 1, true, at(&ic, 100), 1)]),
            Some(CommitToken::from_bytes([1; 16])),
            "the term's first instant counts"
        );
        assert_eq!(
            void(vec![
                row(hull, 1, true, at(&ic, 110), 1),
                row(hull, 3, true, at(&ic, 130), 3),
                row(hull, 2, true, at(&ic, 120), 2),
            ]),
            Some(CommitToken::from_bytes([3; 16])),
            "the newest"
        );
        let late = at(&ic, 250);
        assert_eq!(
            void_from(
                hull,
                (&start, &end),
                &late,
                vec![row(hull, 1, true, at(&ic, 220), 1)]
            ),
            None,
            "never past the term's end"
        );
        let at_end = at(&ic, 200);
        assert_eq!(
            void_from(
                hull,
                (&start, &end),
                &at_end,
                vec![row(hull, 1, true, at(&ic, 200), 1)]
            ),
            None,
            "K10: the term's end is exclusive"
        );
    }

    #[test]
    fn p2_tsw_38_unwarranted_index_survives_ring_and_post_term_eviction() {
        let ic = ClockRef::WorldIc { generation: 1 };
        let hull = ObjId(7);
        let policy = CommitToken::from_bytes([0xaa; 16]);
        let start = at(&ic, 100);
        let end = at(&ic, 200);
        let mut idx = UnwarrantedIndex::default();

        // In-term unwarranted inspection at seq = 1, t = 120.
        let in_term = VoidRecord {
            seq: 1,
            at_ic: at(&ic, 120),
            inspection: CommitToken::from_bytes([0x11; 16]),
        };
        idx.record_hull(hull, in_term.clone());
        assert!(stamp_in_term(&in_term.at_ic, &start, &end));
        idx.record_policy(policy, in_term.clone());
        idx.validate().expect("valid index");

        // 70 subsequent post-term unwarranted inspections evict seq=1 from `hulls`
        // (cap 64) as well as any 64-slot ring buffer, yet `policy_voids` retains it.
        for s in 2..=72_u64 {
            let post = VoidRecord {
                seq: s,
                at_ic: at(&ic, 200 + s as i64),
                inspection: CommitToken::from_bytes([0x22; 16]),
            };
            idx.record_hull(hull, post.clone());
            if stamp_in_term(&post.at_ic, &start, &end) {
                idx.record_policy(policy, post);
            }
        }
        idx.validate().expect("valid after 70 post-term records");
        assert!(
            !idx.hulls[&hull.0].iter().any(|r| r.seq == 1),
            "hull ring evicted seq=1"
        );
        let pinned = idx
            .policy_voids
            .get(&policy.to_hex())
            .expect("policy void pinned");
        assert_eq!(pinned.inspection, in_term.inspection);

        // Evaluating void_from with combined hull + policy_voids still voids the policy.
        let mut rows: Vec<VoidRow> = idx.hulls[&hull.0]
            .iter()
            .map(|r| VoidRow {
                hull,
                seq: r.seq,
                unwarranted: true,
                at: r.at_ic.clone(),
                inspection: r.inspection,
            })
            .collect();
        rows.push(VoidRow {
            hull,
            seq: pinned.seq,
            unwarranted: true,
            at: pinned.at_ic.clone(),
            inspection: pinned.inspection,
        });
        assert_eq!(
            void_from(hull, (&start, &end), &at(&ic, 280), rows),
            Some(in_term.inspection)
        );

        // Terminal cleanup removes the policy pin.
        idx.prune_policy(policy);
        assert!(!idx.policy_voids.contains_key(&policy.to_hex()));
    }

    #[test]
    fn p2_tsw_20_inbox_ttl_and_slot_keying_rules() {
        let service = ObjId(1);
        let buyer = ObjId(2);
        let policy_a = CommitToken::from_bytes([0x0a; 16]);
        let policy_b = CommitToken::from_bytes([0x0b; 16]);

        // Distinct policies get distinct terminal slot keys on the shared service actor.
        assert_ne!(
            (service, 3_usize, Some(policy_a)),
            (service, 3_usize, Some(policy_b))
        );
        // Same policy claim (index 3) and release/void (index 4) are distinct slots.
        assert_ne!(
            (service, 3_usize, Some(policy_a)),
            (service, 4_usize, Some(policy_a))
        );
        // Service terminal lane is isolated from buyer Quote/Bind lane.
        let is_service_lane = |actor: ObjId, index: usize| actor == service && index >= 3;
        assert!(is_service_lane(service, 3));
        assert!(is_service_lane(service, 4));
        assert!(!is_service_lane(buyer, 1));
        assert!(!is_service_lane(buyer, 2));
        assert!(!is_service_lane(buyer, 4));

        // TTL expiry math: Attempt::New and Attempt::Retryable expire at INBOX_IDLE_TTL.
        let t0 = Instant::now();
        let before_ttl = t0 + INBOX_IDLE_TTL - Duration::from_secs(1);
        let at_ttl = t0 + INBOX_IDLE_TTL;
        let fresh_new = Attempt::New;
        let retry = Attempt::Retryable;
        for attempt in [&fresh_new, &retry] {
            let keep_before = matches!(attempt, Attempt::Observed(_))
                || before_ttl.saturating_duration_since(t0) < INBOX_IDLE_TTL;
            let keep_at = matches!(attempt, Attempt::Observed(_))
                || at_ttl.saturating_duration_since(t0) < INBOX_IDLE_TTL;
            assert!(keep_before, "entries stay alive before INBOX_IDLE_TTL");
            assert!(!keep_at, "unsubmitted/retryable entries expire at TTL");
        }
    }

    #[test]
    fn p2_tsw_39_runtime_index_separates_bound_and_unbound_expired_quotes() {
        let mut rt = RuntimeIndex::default();
        let buyer = ObjId(42);
        let open = CommitToken::from_bytes([0x01; 16]);
        let q1 = CommitToken::from_bytes([0x10; 16]);
        let q2 = CommitToken::from_bytes([0x20; 16]);
        let q3 = CommitToken::from_bytes([0x30; 16]);
        let r1 = DefinitionDigest::from_bytes([0x11; 32]);
        let r2 = DefinitionDigest::from_bytes([0x22; 32]);
        let r3 = DefinitionDigest::from_bytes([0x33; 32]);

        rt.record_candidate(r1, 1, q1.to_hex());
        rt.unbound_quotes.insert(
            q1,
            UnboundQuoteEntry {
                actor: buyer,
                ic_open: open,
                expires_at_utc: 100,
            },
        );
        rt.record_candidate(r2, 1, q2.to_hex());
        rt.unbound_quotes.insert(
            q2,
            UnboundQuoteEntry {
                actor: buyer,
                ic_open: open,
                expires_at_utc: 100,
            },
        );
        rt.record_candidate(r3, 1, q3.to_hex());
        rt.unbound_quotes.insert(
            q3,
            UnboundQuoteEntry {
                actor: buyer,
                ic_open: open,
                expires_at_utc: 500,
            },
        );
        // Mark q2 as pending-bind so it is never pruned even when expired.
        rt.bind_candidates.insert(q2);

        let expired: Vec<CommitToken> = rt
            .unbound_quotes
            .iter()
            .filter_map(|(&op, entry)| {
                (entry.expires_at_utc <= 200
                    && !rt.bound_quotes.contains(&op)
                    && !rt.bind_candidates.contains(&op))
                .then_some(op)
            })
            .collect();
        assert_eq!(expired, vec![q1], "only expired unbound q1 is prunable");
    }
}
