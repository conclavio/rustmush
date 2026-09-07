//! The campaign store, its persistence and its boot (E7 cut 9).
//!
//! ## The document discipline (§4 rules 1 and 2)
//!
//! | doc | class | a refused one |
//! |---|---|---|
//! | `op:{id}` | container | **held, never deleted**, its subject marked and staff paged |
//! | `formation:{id}` | container | **held, never deleted**, its subject marked |
//! | `campaign_meta` | allocators + the tick cursor | tolerated field by field, floored by the keys |
//!
//! An operation is a container for the same reason a tenancy is: its state **is**
//! the record — and, unlike a tenancy, it may be holding a live money `Hold` whose
//! only statement of purpose is this document. Deleting it would strand the
//! attacker's funds with nothing left in the world to say what they were for. So a
//! refused document is **held** (cut 6's `JourneyBook::held` precedent), the world
//! still boots, and staff are paged with the decode error and the command.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{ModuleOp, ModuleWrite};
use mush_core::{Ctx, Outbox, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::formation::{FormationAuthority, FormationKind, FormationNode};
use crate::ids::{FormationId, OperationId, formation_key, operation_key};
use crate::record::{OperationDoc, UnitRef};
use crate::seat::{
    CrewComplement, MAX_SEAT_OFFERS_PER_OPERATION, MusterReport, SeatGrant, SeatOffer, SeatOfferId,
    crew_key, muster_key, seat_grant_key, seat_offer_key,
};

/// The module's persistence namespace and registered name.
pub const NS: &str = "campaign";
/// The allocator/cursor document.
pub const META_KEY: &str = "campaign_meta";
/// The largest number of operations one world may carry.
pub const MAX_OPERATIONS: usize = 512;
/// The largest number of **live** operations one issuer may carry at once (E7 cut 9
/// independent verify fold, B8; decision E5(a)).
///
/// `@op/declare` asked for no capability, no bond, no scope and no rate limit, and
/// nothing anywhere removes an `op:<id>` document — so the only bound was the global
/// [`MAX_OPERATIONS`], and one unprivileged player exhausted the world's allocator
/// permanently and starved the tick's rotation besides (512 documents against
/// `MAX_VISITED_PER_FIRE` 64 is eight fires per round). Counted over
/// `terms.attacker` for states that are not terminal, so resolving or abandoning one
/// frees the slot; staff are not counted.
pub const MAX_OPERATIONS_PER_ISSUER: usize = 32;
/// The largest number of formation nodes one world may carry.
pub const MAX_FORMATION_NODES: usize = 1_024;
/// The largest number of refused documents kept for `@op/held`-style inspection
/// (cut 7's `MAX_HELD_DOCUMENTS` shape).
pub const MAX_HELD_DOCUMENTS: usize = 64;

/// One document the boot could not decode, kept with its error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldDoc {
    /// The key it was stored at.
    pub key: SmolStr,
    /// Why it was refused.
    pub error: String,
}

/// What one tick fire did (the pin every budget test reads).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TickStats {
    /// How many operations were touched at all.
    pub visited: usize,
    /// How many were worked.
    pub worked: usize,
    /// How many consumers were advanced (at most one per operation per fire).
    pub consumers: usize,
    /// How many were paused this fire.
    pub paused: usize,
}

/// The world-attached campaign state.
#[derive(Clone, Default)]
pub struct CampaignStore {
    ops: BTreeMap<u64, OperationDoc>,
    formations: BTreeMap<u64, FormationNode>,
    /// Kinds, hydrated from the composition-frozen authority at boot. Empty when
    /// no authority is wired, which makes `@force/create` refuse — fail-closed.
    kinds: BTreeMap<SmolStr, FormationKind>,
    authority: Option<Arc<dyn FormationAuthority>>,
    /// E7c10 B2/B3/B7: the seat lane's own documents, keyed exactly as they
    /// persist. All four are EXEMPT workflow records — no legality check reads
    /// one (a probe pins it), and cut 10 persists no lease-like record at all.
    seat_offers: BTreeMap<(u64, u64), SeatOffer>,
    seat_grants: BTreeMap<(u64, u64), SeatGrant>,
    crews: BTreeMap<u32, CrewComplement>,
    musters: BTreeMap<(u64, u64), MusterReport>,
    /// The next seat-offer id per operation, floored past every id on disk so a
    /// held document's id can never be re-issued underneath it.
    next_seat_offer: BTreeMap<u64, u64>,
    /// The muster sweep's own monotonic cycle (the tick arm's cursor).
    pub(crate) muster_cycle: u64,
    held: Vec<HeldDoc>,
    /// How many refused documents did not fit in `held` (E7 cut 9 fold, B14). Past
    /// [`MAX_HELD_DOCUMENTS`] the 65th was dropped from the list with no counter and
    /// no log, so staff were told about 64 of them and silently not about the rest —
    /// the exact failure mode that made a bond-bearing document invisible.
    held_overflow: usize,
    next_operation: u64,
    next_formation: u64,
    /// The rotation cursor: the tick resumes **past the last visited id**, skipped
    /// ones included (housing's `tick.rs`, D-LBK-2).
    pub(crate) cursor: u64,
    /// The last fire's budget accounting.
    pub(crate) last_fire: TickStats,
    /// Subjects already paged, so a repeated failure pages once rather than once a
    /// fire (08c §4.6's dedupe-by-subject rule).
    pub(crate) paged: BTreeSet<SmolStr>,
}

impl std::fmt::Debug for CampaignStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CampaignStore")
            .field("operations", &self.ops.len())
            .field("formations", &self.formations.len())
            .field("kinds", &self.kinds.len())
            .field("seat_offers", &self.seat_offers.len())
            .field("seat_grants", &self.seat_grants.len())
            .field("crews", &self.crews.len())
            .field("held", &self.held.len())
            .field("cursor", &self.cursor)
            .finish()
    }
}

/// The `campaign_meta` document.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Meta {
    next_operation: u64,
    next_formation: u64,
    cursor: u64,
    /// E7c10 B7: the muster sweep's monotonic cycle, so a reboot re-running the
    /// sweep is a no-op past the applied point rather than a re-write.
    #[serde(default)]
    muster_cycle: u64,
}

impl CampaignStore {
    /// One operation.
    #[must_use]
    pub fn operation(&self, id: OperationId) -> Option<&OperationDoc> {
        self.ops.get(&id.0)
    }

    /// Every operation, in id order.
    pub fn operations(&self) -> impl Iterator<Item = &OperationDoc> {
        self.ops.values()
    }

    /// Every operation id, in order — the tick's rotation reads this.
    #[must_use]
    pub fn operation_ids(&self) -> Vec<OperationId> {
        self.ops.keys().copied().map(OperationId).collect()
    }

    /// One formation node.
    #[must_use]
    pub fn formation(&self, id: FormationId) -> Option<&FormationNode> {
        self.formations.get(&id.0)
    }

    /// Every formation node, in id order.
    pub fn formations(&self) -> impl Iterator<Item = &FormationNode> {
        self.formations.values()
    }

    /// One formation kind, as the composition-frozen authority authored it.
    #[must_use]
    pub fn kind(&self, id: &str) -> Option<&FormationKind> {
        self.kinds.get(id)
    }

    /// Every known kind, in id order.
    pub fn kinds(&self) -> impl Iterator<Item = &FormationKind> {
        self.kinds.values()
    }

    /// The documents the boot refused, with their errors.
    #[must_use]
    pub fn held(&self) -> &[HeldDoc] {
        &self.held
    }

    /// Whether the boot refused (and is holding) this operation's document. A held
    /// operation is **not** in `ops`, so every `@op` verb answers the
    /// existence-hiding refusal — and its bond, if it had one, is a live reservation
    /// no campaign command can otherwise reach (E7 cut 9 fold, C3).
    #[must_use]
    pub fn is_held(&self, id: OperationId) -> bool {
        let key = operation_key(id);
        self.held.iter().any(|doc| doc.key == key)
    }

    /// How many further documents the boot refused past [`MAX_HELD_DOCUMENTS`].
    #[must_use]
    pub fn held_overflow(&self) -> usize {
        self.held_overflow
    }

    /// How many **live** operations `issuer` is carrying — the per-issuer cap's own
    /// count (B8). A terminal operation is not one.
    #[must_use]
    pub fn live_operations_for(&self, issuer: &mush_org::PrincipalId) -> usize {
        self.ops
            .values()
            .filter(|doc| !doc.runtime.state.is_terminal() && doc.terms.attacker == *issuer)
            .count()
    }

    /// The last fire's accounting.
    #[must_use]
    pub fn last_fire(&self) -> TickStats {
        self.last_fire
    }

    /// **The one-operation rule (R1)**: which operation, if any, already holds
    /// `unit`. A `Resolved` or `Aborted` operation holds nothing.
    #[must_use]
    pub fn operation_holding(&self, unit: UnitRef) -> Option<OperationId> {
        self.ops
            .values()
            .find(|doc| {
                !matches!(
                    doc.runtime.state,
                    crate::record::OperationState::Resolved
                        | crate::record::OperationState::Aborted
                ) && doc.runtime.holds(unit)
            })
            .map(OperationDoc::id)
    }

    // ---- E7c10: the seat lane's accessors and installers ----

    /// One live seat offer.
    #[must_use]
    pub fn seat_offer(&self, operation: OperationId, id: SeatOfferId) -> Option<&SeatOffer> {
        self.seat_offers.get(&(operation.0, id.0))
    }

    /// Every seat offer, in (operation, offer) order.
    pub fn seat_offers(&self) -> impl Iterator<Item = &SeatOffer> {
        self.seat_offers.values()
    }

    /// How many offers one operation carries - the cap's own count.
    #[must_use]
    pub fn seat_offer_count(&self, operation: OperationId) -> usize {
        self.seat_offers
            .range((operation.0, 0)..=(operation.0, u64::MAX))
            .count()
    }

    /// One seat grant.
    #[must_use]
    pub fn seat_grant(&self, operation: OperationId, id: SeatOfferId) -> Option<&SeatGrant> {
        self.seat_grants.get(&(operation.0, id.0))
    }

    /// Every seat grant, in (operation, offer) order.
    /// How many grants one operation carries (B10's counted cap).
    #[must_use]
    pub fn seat_grant_count(&self, operation: OperationId) -> usize {
        self.seat_grants
            .range((operation.0, 0)..=(operation.0, u64::MAX))
            .count()
    }

    pub fn seat_grants(&self) -> impl Iterator<Item = &SeatGrant> {
        self.seat_grants.values()
    }

    /// One unit's crew complement.
    #[must_use]
    pub fn crew(&self, unit: UnitRef) -> Option<&CrewComplement> {
        self.crews.get(&unit.0.0)
    }

    /// Every crew complement, in unit order.
    pub fn crews(&self) -> impl Iterator<Item = &CrewComplement> {
        self.crews.values()
    }

    /// The newest muster report for an operation, if any.
    #[must_use]
    pub fn latest_muster(&self, operation: OperationId) -> Option<&MusterReport> {
        self.musters
            .range((operation.0, 0)..=(operation.0, u64::MAX))
            .next_back()
            .map(|(_, report)| report)
    }

    /// Mint the next seat-offer id for an operation, or `None` past
    /// [`crate::seat::MAX_SEAT_OFFERS_PER_OPERATION`].
    pub(crate) fn mint_seat_offer(&mut self, operation: OperationId) -> Option<SeatOfferId> {
        if self.seat_offer_count(operation) >= MAX_SEAT_OFFERS_PER_OPERATION {
            return None;
        }
        let slot = self.next_seat_offer.entry(operation.0).or_insert(1);
        let id = (*slot).max(1);
        *slot = id.checked_add(1)?;
        Some(SeatOfferId(id))
    }

    pub(crate) fn install_seat_offer(&mut self, offer: SeatOffer) {
        let slot = self.next_seat_offer.entry(offer.operation.0).or_insert(1);
        *slot = (*slot).max(offer.id.0.saturating_add(1));
        self.seat_offers
            .insert((offer.operation.0, offer.id.0), offer);
    }

    pub(crate) fn install_seat_grant(&mut self, grant: SeatGrant) {
        self.seat_grants
            .insert((grant.operation.0, grant.offer.0), grant);
    }

    pub(crate) fn install_crew(&mut self, crew: CrewComplement) {
        self.crews.insert(crew.unit.0.0, crew);
    }

    /// **The muster ring** (E7 cut 10 fold, B1): keep this operation's newest
    /// [`MAX_RETAINED_MUSTERS`] reports and return the cycles evicted, so the
    /// caller can `Del` their documents in the same bundle. Only
    /// [`CampaignStore::latest_muster`] reads the collection, so no history is
    /// needed at all.
    pub(crate) fn prune_musters(&mut self, operation: OperationId) -> Vec<u64> {
        let doomed: Vec<u64> = self
            .musters
            .range((operation.0, 0)..=(operation.0, u64::MAX))
            .rev()
            .skip(crate::seat::MAX_RETAINED_MUSTERS)
            .map(|((_, cycle), _)| *cycle)
            .collect();
        for cycle in &doomed {
            self.musters.remove(&(operation.0, *cycle));
        }
        doomed
    }

    pub(crate) fn install_muster(&mut self, report: MusterReport) {
        self.muster_cycle = self.muster_cycle.max(report.cycle);
        self.musters
            .insert((report.operation.0, report.cycle), report);
    }

    fn meta_doc(&self) -> serde_json::Value {
        serde_json::to_value(Meta {
            next_operation: self.next_operation,
            next_formation: self.next_formation,
            cursor: self.cursor,
            muster_cycle: self.muster_cycle,
        })
        .unwrap_or(serde_json::Value::Null)
    }

    /// Mint the next operation id, or `None` past [`MAX_OPERATIONS`].
    pub(crate) fn mint_operation(&mut self) -> Option<OperationId> {
        if self.ops.len() >= MAX_OPERATIONS {
            return None;
        }
        let id = self.next_operation.max(1);
        self.next_operation = id.checked_add(1)?;
        Some(OperationId(id))
    }

    /// Mint the next formation id, or `None` past [`MAX_FORMATION_NODES`].
    pub(crate) fn mint_formation(&mut self) -> Option<FormationId> {
        if self.formations.len() >= MAX_FORMATION_NODES {
            return None;
        }
        let id = self.next_formation.max(1);
        self.next_formation = id.checked_add(1)?;
        Some(FormationId(id))
    }

    pub(crate) fn install_operation(&mut self, doc: OperationDoc) {
        self.next_operation = self.next_operation.max(doc.id().0.saturating_add(1));
        self.ops.insert(doc.id().0, doc);
    }

    pub(crate) fn install_formation(&mut self, node: FormationNode) {
        self.next_formation = self.next_formation.max(node.id.0.saturating_add(1));
        self.formations.insert(node.id.0, node);
    }

    pub(crate) fn hold(&mut self, key: SmolStr, error: String) {
        if self.held.len() >= MAX_HELD_DOCUMENTS {
            // Counted, not silently dropped: the boot pages the overflow once.
            self.held_overflow = self.held_overflow.saturating_add(1);
            return;
        }
        self.held.push(HeldDoc { key, error });
    }

    pub(crate) fn set_authority(&mut self, authority: Option<Arc<dyn FormationAuthority>>) {
        self.authority = authority;
    }

    /// Re-read the composition-frozen authority. Called once at boot; the kinds are
    /// rules data and never change at runtime.
    pub(crate) fn hydrate_kinds(&mut self, world: &World) {
        let Some(authority) = self.authority.clone() else {
            return;
        };
        self.kinds = authority
            .kinds(world)
            .into_iter()
            .map(|k| (k.id.clone(), k))
            .collect();
    }
}

fn push(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Install an operation into RAM and queue its document. **The single write path**:
/// there is no route that writes one without the other.
/// **The write path is the gate** (E7 cut 9 independent verify fold, C3).
///
/// §3.9's rule is "a stored set over any cap refuses on load, never truncates", and
/// the refused document is *held* — but `@op/withdraw` and the `ForceRetirement`
/// consumer wrote without ever re-running the constructor, so ordinary play could
/// store a document the next boot refused. The operation then left the store,
/// `store.operation(id)` answered `None`, every `@op` verb answered the
/// existence-hiding `No such operation.`, and the live E4 `Hold` stood against the
/// attacker's account with no campaign path left to it at all.
///
/// So the write re-runs both constructors. A document that would not load is **not
/// written** — the caller's own validation should have caught it, and reaching here
/// is a level-2 page with the document left exactly as it was.
pub(crate) fn put_operation(ctx: &mut Ctx<'_>, doc: OperationDoc) -> bool {
    if let Err(error) = doc.terms.check() {
        page_refused_write(ctx, doc.id(), &error.to_string());
        return false;
    }
    if let Err(error) = doc.runtime.check() {
        page_refused_write(ctx, doc.id(), &error.to_string());
        return false;
    }
    let key = operation_key(doc.id());
    let value = serde_json::to_value(&doc).unwrap_or(serde_json::Value::Null);
    if let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
        store.install_operation(doc);
    }
    push(ctx.out, ModuleWrite::JsonSet { key, value });
    true
}

fn page_refused_write(ctx: &mut Ctx<'_>, id: crate::ids::OperationId, error: &str) {
    crate::settle::page(
        ctx,
        SmolStr::new(format!("op:{}:write", id.0)),
        &format!("a write to operation {id} was refused before it was stored ({error})"),
        &format!(
            "operation {id} was about to be written in a shape the next boot would refuse and \
             hold; the write was dropped and the stored document is unchanged. Inspect it with \
             `@op/show/full {id}` and, if it cannot go on, abandon it with `@op/resolve \
             {id}=abort`, which releases its bond"
        ),
    );
}

/// Install a formation node into RAM and queue its document.
pub(crate) fn put_formation(ctx: &mut Ctx<'_>, node: FormationNode) {
    let key = formation_key(node.id);
    let value = serde_json::to_value(&node).unwrap_or(serde_json::Value::Null);
    if let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
        store.install_formation(node);
    }
    push(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// Install a seat offer into RAM and queue its document - **the single write
/// path**, re-running the constructor so ordinary play can never store a
/// document the next boot would refuse and hold (cut 9 fold C3's discipline).
pub(crate) fn put_seat_offer(ctx: &mut Ctx<'_>, offer: SeatOffer) -> bool {
    if offer.check().is_err() {
        return false;
    }
    let key = seat_offer_key(offer.operation, offer.id);
    let value = serde_json::to_value(&offer).unwrap_or(serde_json::Value::Null);
    if let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
        store.install_seat_offer(offer);
    }
    push(ctx.out, ModuleWrite::JsonSet { key, value });
    true
}

/// Install a seat grant into RAM and queue its document (the same discipline).
///
/// **B10: `MAX_SEAT_GRANTS_PER_OPERATION` has a refusal now.** It was declared,
/// re-exported and enforced nowhere — the bound was achieved structurally (a
/// grant is keyed `(operation, offer)` and offers are capped at mint), which is
/// exactly the "documentation only" shape cut 9's fold struck. A NEW grant past
/// the cap is refused here, at the one write path.
pub(crate) fn put_seat_grant(ctx: &mut Ctx<'_>, grant: SeatGrant) -> bool {
    if grant.check().is_err() {
        return false;
    }
    if let Some(store) = ctx.w.module_state::<CampaignStore>()
        && store.seat_grant(grant.operation, grant.offer).is_none()
        && store.seat_grant_count(grant.operation) >= crate::seat::MAX_SEAT_GRANTS_PER_OPERATION
    {
        return false;
    }
    let key = seat_grant_key(grant.operation, grant.offer);
    let value = serde_json::to_value(&grant).unwrap_or(serde_json::Value::Null);
    if let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
        store.install_seat_grant(grant);
    }
    push(ctx.out, ModuleWrite::JsonSet { key, value });
    true
}

/// Install a crew complement into RAM and queue its document.
pub(crate) fn put_crew(ctx: &mut Ctx<'_>, crew: CrewComplement) -> bool {
    if crew.check().is_err() {
        return false;
    }
    let key = crew_key(crew.unit);
    let value = serde_json::to_value(&crew).unwrap_or(serde_json::Value::Null);
    if let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
        store.install_crew(crew);
    }
    push(ctx.out, ModuleWrite::JsonSet { key, value });
    true
}

/// Install a muster report into RAM and queue its document, **evicting this
/// operation's oldest reports past [`MAX_RETAINED_MUSTERS`] in the same bundle**
/// (E7 cut 10 fold, B1). A superseded report is `Del`eted by policy — it is not
/// a stored set truncated on load, which §4 rule 1 forbids.
pub(crate) fn put_muster(ctx: &mut Ctx<'_>, report: MusterReport) -> bool {
    if report.check().is_err() {
        return false;
    }
    let operation = report.operation;
    let key = muster_key(report.operation, report.cycle);
    let value = serde_json::to_value(&report).unwrap_or(serde_json::Value::Null);
    let evicted = match ctx.w.module_state_mut::<CampaignStore>() {
        Some(store) => {
            store.install_muster(report);
            store.prune_musters(operation)
        }
        None => Vec::new(),
    };
    push(ctx.out, ModuleWrite::JsonSet { key, value });
    for cycle in evicted {
        push(
            ctx.out,
            ModuleWrite::Del {
                key: muster_key(operation, cycle),
            },
        );
    }
    true
}

/// Queue the allocator/cursor document.
pub(crate) fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(meta) = ctx
        .w
        .module_state::<CampaignStore>()
        .map(CampaignStore::meta_doc)
    else {
        return;
    };
    push(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: meta,
        },
    );
}

/// Load the boot documents. A refused container is **held**, never dropped and
/// never fatal; the allocators are floored by the keys that survived, so a stale or
/// missing meta can never re-issue a live id.
pub(crate) fn load(kv: Vec<(SmolStr, serde_json::Value)>) -> CampaignStore {
    let mut store = CampaignStore::default();
    for (key, value) in kv {
        if key == META_KEY {
            match serde_json::from_value::<Meta>(value) {
                Ok(meta) => {
                    store.next_operation = meta.next_operation;
                    store.next_formation = meta.next_formation;
                    store.cursor = meta.cursor;
                    store.muster_cycle = meta.muster_cycle;
                }
                Err(error) => {
                    tracing::warn!(%key, %error, "campaign: the meta document was ignored");
                }
            }
        } else if let Some(raw) = key.strip_prefix("op:") {
            let id = raw.parse::<u64>().unwrap_or(0);
            match serde_json::from_value::<OperationDoc>(value) {
                Ok(doc) if doc.id().0 == id && id != 0 => store.install_operation(doc),
                Ok(_) => {
                    store.next_operation = store.next_operation.max(id.saturating_add(1));
                    store.hold(key, "the document's id disagrees with its key".to_owned());
                }
                Err(error) => {
                    // The allocator still floors past a held id, so a refused
                    // operation can never have its id re-issued underneath it.
                    store.next_operation = store.next_operation.max(id.saturating_add(1));
                    store.hold(key, error.to_string());
                }
            }
        } else if let Some(raw) = key.strip_prefix("formation:") {
            let id = raw.parse::<u64>().unwrap_or(0);
            match serde_json::from_value::<FormationNode>(value) {
                Ok(node) if node.id.0 == id && id != 0 => store.install_formation(node),
                Ok(_) => {
                    store.next_formation = store.next_formation.max(id.saturating_add(1));
                    store.hold(key, "the document's id disagrees with its key".to_owned());
                }
                Err(error) => {
                    store.next_formation = store.next_formation.max(id.saturating_add(1));
                    store.hold(key, error.to_string());
                }
            }
        } else if key.starts_with("seat_offer:") {
            // E7c10 B2: an over-cap or malformed offer is HELD, never deleted -
            // a seat grant may be live against it - and its id is floored so it
            // can never be re-issued underneath a document staff must repair.
            match serde_json::from_value::<SeatOffer>(value) {
                Ok(offer) if seat_offer_key(offer.operation, offer.id) == key => {
                    store.install_seat_offer(offer)
                }
                Ok(_) => store.hold(key, "the offer's id disagrees with its key".to_owned()),
                Err(error) => store.hold(key, error.to_string()),
            }
        } else if key.starts_with("seat_grant:") {
            match serde_json::from_value::<SeatGrant>(value) {
                Ok(grant) if seat_grant_key(grant.operation, grant.offer) == key => {
                    store.install_seat_grant(grant)
                }
                Ok(_) => store.hold(key, "the grant's id disagrees with its key".to_owned()),
                Err(error) => store.hold(key, error.to_string()),
            }
        } else if key.starts_with("crew:") {
            match serde_json::from_value::<CrewComplement>(value) {
                Ok(crew) if crew_key(crew.unit) == key => store.install_crew(crew),
                Ok(_) => store.hold(key, "the crew's unit disagrees with its key".to_owned()),
                Err(error) => store.hold(key, error.to_string()),
            }
        } else if key.starts_with("muster:") {
            match serde_json::from_value::<MusterReport>(value) {
                Ok(report) if muster_key(report.operation, report.cycle) == key => {
                    store.install_muster(report)
                }
                Ok(_) => store.hold(key, "the report's key disagrees with itself".to_owned()),
                Err(error) => store.hold(key, error.to_string()),
            }
        } else {
            tracing::warn!(%key, "campaign: unknown persisted key ignored");
        }
    }
    // **B1: the boot hydration is bounded too.** A store written by a build
    // without the ring can carry an unbounded muster book, and cut 10 installed
    // every one it found — so the growth cost boot-time RAM as well as disk.
    let operations: Vec<OperationId> = store
        .musters
        .keys()
        .map(|(operation, _)| OperationId(*operation))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    for operation in operations {
        let dropped = store.prune_musters(operation).len();
        if dropped > 0 {
            tracing::warn!(
                %operation,
                dropped,
                "campaign: muster book over the retention ring; the oldest reports were left on \
                 disk and not hydrated"
            );
        }
    }
    store
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::OperationId;
    use crate::record::OperationRuntime;
    use crate::terms::operation_consent_hash;
    use crate::testfix::terms_fixture;

    fn doc(id: OperationId) -> OperationDoc {
        let mut terms = terms_fixture(id);
        terms.consent_hash = operation_consent_hash(&terms);
        OperationDoc {
            terms,
            runtime: OperationRuntime::declared(id),
        }
    }

    #[test]
    fn e7c9_b2_a_refused_operation_document_is_held_and_its_id_is_never_reissued() {
        let good = serde_json::to_value(doc(OperationId(1))).expect("serializes");
        let mut bad = serde_json::to_value(doc(OperationId(7))).expect("serializes");
        bad["runtime"]["participants"] = serde_json::json!([
            { "unit": 1, "side": "a", "command_parent": null, "joined_at": 0 },
            { "unit": 1, "side": "a", "command_parent": null, "joined_at": 0 },
        ]);
        let store = load(vec![
            (SmolStr::new_static("op:1"), good),
            (SmolStr::new_static("op:7"), bad),
        ]);
        assert!(store.operation(OperationId(1)).is_some());
        assert!(store.operation(OperationId(7)).is_none(), "refused");
        assert_eq!(store.held().len(), 1);
        assert_eq!(store.held()[0].key, "op:7");
        // …and the allocator floored past the held id, so the next mint cannot
        // collide with a document staff have yet to repair.
        let mut store = store;
        assert_eq!(store.mint_operation(), Some(OperationId(8)));
    }

    #[test]
    fn e7c9_b2_a_document_whose_id_disagrees_with_its_key_is_held_not_installed() {
        let mut wrong = serde_json::to_value(doc(OperationId(3))).expect("serializes");
        wrong["terms"]["operation"] = serde_json::json!(3);
        let store = load(vec![(SmolStr::new_static("op:4"), wrong)]);
        assert!(store.operation(OperationId(3)).is_none());
        assert!(store.operation(OperationId(4)).is_none());
        assert_eq!(store.held().len(), 1);
    }

    // ---- E7 cut 9 independent verify fold ----

    /// **B14. A boot with more refused documents than the held list carries counts
    /// the overflow.** Past `MAX_HELD_DOCUMENTS` cut 9 dropped the 65th with no
    /// counter and no log, so staff were told about 64 of them and silently not about
    /// the rest — while every one of them was still on disk and still floored the
    /// allocator, which is exactly what makes the silence dangerous.
    #[test]
    fn vf_b14a_a_boot_with_sixty_seven_refused_documents_counts_the_overflow() {
        let mut kv = Vec::new();
        for n in 1..=67u64 {
            let mut bad = serde_json::to_value(doc(OperationId(n))).expect("serializes");
            bad["runtime"]["participants"] = serde_json::json!([
                { "unit": 1, "side": "a", "command_parent": null, "joined_at": 0 },
                { "unit": 1, "side": "a", "command_parent": null, "joined_at": 0 },
            ]);
            kv.push((SmolStr::new(format!("op:{n}")), bad));
        }
        let mut store = load(kv);
        assert_eq!(store.held().len(), MAX_HELD_DOCUMENTS);
        assert_eq!(store.held_overflow(), 67 - MAX_HELD_DOCUMENTS);
        // Nothing was installed, nothing was deleted, and the allocator is floored
        // past every one of them — including the ones the list could not carry.
        assert_eq!(store.operations().count(), 0);
        assert_eq!(store.mint_operation(), Some(OperationId(68)));
    }

    /// **B8. The per-issuer count is over live operations only.**
    #[test]
    fn vf_b8b_the_per_issuer_count_ignores_terminal_operations() {
        let mut store = CampaignStore::default();
        let issuer = mush_org::PrincipalId::Object(mush_core::ObjId(100));
        for (n, state) in [
            crate::record::OperationState::Declared,
            crate::record::OperationState::Activated,
            crate::record::OperationState::Running,
            crate::record::OperationState::Resolved,
            crate::record::OperationState::Aborted,
        ]
        .into_iter()
        .enumerate()
        {
            let id = OperationId(n as u64 + 1);
            let mut d = doc(id);
            d.runtime.state = state;
            store.install_operation(d);
        }
        assert_eq!(store.live_operations_for(&issuer), 3);
        assert_eq!(
            store.live_operations_for(&mush_org::PrincipalId::Object(mush_core::ObjId(101))),
            0
        );
    }
}
