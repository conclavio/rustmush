//! Coordinator-private future-key accounting. Registry activation supplies one
//! closed, composition-checked protocol; modules cannot provide arbitrary target
//! vectors, byte ceilings or verifier callbacks. Unconfigured metadata refuses.

use super::*;

mod charter_pending;
mod core_integration;
mod core_refit;
mod entitlement;
mod house;
mod house_renewal;
mod ic_claim;
mod paid;
mod paid_resale;
mod registry;
mod renewal;
use crate::{CommitToken, DefinitionRef};
pub(super) use charter_pending::{Protocol as CharterPendingProtocol, charter_pending_change};
pub(super) use core_integration::{Protocol as CoreIntegrationProtocol, core_integration_change};
pub(super) use core_refit::{Protocol as CoreRefitProtocol, refit_change};
pub(super) use entitlement::{Protocol as EntitlementProtocol, entitlement_change};
pub(super) use house::{Protocol as HouseProtocol, house_change};
pub(super) use house_renewal::{
    Protocol as HouseRenewalProtocol, renewal_change as house_renewal_change,
};
pub(super) use ic_claim::{Protocol as IcClaimProtocol, ic_claim_change};
pub(super) use paid::{Protocol as PurchaseProtocol, purchase_change};
pub(super) use paid_resale::{Protocol as PaidResaleProtocol, paid_resale_change};
pub(super) use registry::registry_change;
pub(super) use registry::{Protocol, RegisteredTxnIdentity};
pub(super) use renewal::{Protocol as RenewalProtocol, renewal_change};

const PREFIX: &str = "action-state/capacity/v1/";
const MAX_TARGETS: usize = 128;

fn refused(reason: &'static str) -> ActionStateError {
    ActionStateError::TerminalCapacity(reason)
}

pub(super) fn is_capacity_key(key: &str) -> bool {
    key.starts_with("action-state/capacity/")
}

/// Strict persisted key data, not a state-family capability.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Key {
    owner: OwnerId,
    family: StateFamilyId,
    encoded: SmolStr,
}

impl Key {
    fn resolve(&self, registry: &StateFamilyRegistry) -> Result<StateKey, ActionStateError> {
        let family = registry
            .entries
            .get(&(self.owner.clone(), self.family.clone()))
            .ok_or_else(|| refused("unknown target family"))?;
        validate_encoded_state_key(
            self.owner.clone(),
            self.family.clone(),
            &self.encoded,
            family.descriptor().max_key_bytes(),
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SlotMode {
    Create,
    UpdateExisting,
    // Only the fixed Industry protocol may own an absent-then-created output.
    IndustryOutput,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Target {
    key: Key,
    schema: CanonicalId,
    schema_version: u16,
    ceiling: usize,
    mode: SlotMode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    format: u8,
    funding: CommitToken,
    effect: DefinitionRef,
    sponsor: Key,
    targets: Vec<Target>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    protocol_fingerprint: Option<DefinitionDigest>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ReservationId(OwnerId, String);

pub(super) struct StoredReservation {
    record: Reservation,
    canonical: Vec<u8>,
    document: Value,
    path: AuthoritativeKey,
}

// Only the fixed IC protocol has a Roll sponsor and Tacsim-owned metadata.
// Every other protocol keeps its exact historical namespace and identifier.
fn metadata_owner(record: &Reservation) -> &OwnerId {
    if ic_claim::metadata_shape(record) {
        &record.effect.key().owner
    } else {
        &record.sponsor.owner
    }
}

impl StoredReservation {
    pub(super) fn id(&self) -> ReservationId {
        ReservationId(
            metadata_owner(&self.record).clone(),
            self.record.funding.to_hex(),
        )
    }
    pub(super) fn len(&self) -> usize {
        self.canonical.len()
    }
}

pub(super) fn decode(raw: &ActionStateBootRecord) -> Result<StoredReservation, ActionStateError> {
    // Bound before clone/typed Vec decoding. Unknown versions never fall through
    // to a module loader, including when the composition has no verifier.
    let canonical = canonical_json_bytes_with_limit(&raw.value, MAX_ACTION_STATE_RECORD_BYTES)
        .map_err(|_| refused("metadata bound"))?;
    let record: Reservation =
        serde_json::from_value(raw.value.clone()).map_err(|_| refused("metadata encoding"))?;
    let stored = encode(record)?;
    if raw.namespace != metadata_owner(&stored.record).as_str()
        || raw.key.as_str() != stored.path.as_str()
        || stored.canonical != canonical
    {
        return Err(refused("metadata path or canonical identity"));
    }
    Ok(stored)
}

fn encode(record: Reservation) -> Result<StoredReservation, ActionStateError> {
    if record.format != 1
        || record.targets.is_empty()
        || record.targets.len() > MAX_TARGETS
        || (record.effect.key().owner != record.sponsor.owner && !ic_claim::metadata_shape(&record))
    {
        return Err(refused("metadata shape"));
    }
    let document = serde_json::to_value(&record).map_err(|_| refused("metadata encoding"))?;
    let canonical = canonical_json_bytes_with_limit(&document, MAX_ACTION_STATE_RECORD_BYTES)
        .map_err(|_| refused("metadata bound"))?;
    let path = AuthoritativeKey::parse_canonical(format!("{PREFIX}{}", record.funding.to_hex()))
        .map_err(|_| refused("metadata key"))?;
    Ok(StoredReservation {
        record,
        canonical,
        document,
        path,
    })
}

#[derive(Clone, Copy)]
enum Phase {
    Funding,
    Retained,
    Terminal,
}

/// The verifier table is core-private and derives solely from the fixed closed
/// composition declaration. Its Registry protocol also audits reverse completeness.
struct Verifier {
    effect: DefinitionRef,
    verify: for<'a> fn(Phase, &Reservation, &View<'a>) -> Result<(), ActionStateError>,
}

#[derive(Default)]
pub(super) struct Verifiers(Vec<Verifier>);

impl Verifiers {
    pub(super) fn for_registry(registry: &StateFamilyRegistry) -> Self {
        let mut rows = Vec::new();
        if let Some(protocol) = &registry.registry_rating {
            for action in protocol.admission_actions() {
                rows.push(Verifier {
                    effect: action.clone(),
                    verify: registry::verify,
                });
            }
        }
        if let Some(protocol) = &registry.original_purchase {
            rows.push(Verifier {
                effect: protocol.admission_action().clone(),
                verify: paid::verify,
            });
        }
        if let Some(protocol) = &registry.entitlement_purchase {
            for action in protocol.admission_actions() {
                rows.push(Verifier {
                    effect: action.clone(),
                    verify: entitlement::verify,
                });
            }
        }
        if let Some(protocol) = &registry.charter_pending {
            rows.push(Verifier {
                effect: protocol.admission_action().clone(),
                verify: charter_pending::verify,
            });
        }
        if let Some(protocol) = &registry.paid_resale {
            rows.push(Verifier {
                effect: protocol.admission_action().clone(),
                verify: paid_resale::verify,
            });
        }
        if let Some(protocol) = &registry.house_purchase {
            for action in protocol.admission_actions() {
                rows.push(Verifier {
                    effect: action.clone(),
                    verify: house::verify,
                });
            }
        }
        if let Some(protocol) = &registry.mind_renewal {
            rows.push(Verifier {
                effect: protocol.admission_action().clone(),
                verify: renewal::verify,
            });
        }
        if let Some(protocol) = &registry.house_mind_renewal {
            rows.push(Verifier {
                effect: protocol.admission_action().clone(),
                verify: house_renewal::verify,
            });
        }
        if let Some(protocol) = &registry.ic_mind_claim {
            rows.push(Verifier {
                effect: protocol.admission_action().clone(),
                verify: ic_claim::verify,
            });
        }
        if let Some(protocol) = &registry.core_integration {
            rows.push(Verifier {
                effect: protocol.admission_action().clone(),
                verify: core_integration::verify,
            });
        }
        if let Some(protocol) = &registry.core_refit {
            rows.push(Verifier {
                effect: protocol.admission_action().clone(),
                verify: core_refit::verify,
            });
        }
        Self(rows)
    }

    fn verify(
        &self,
        registry: &StateFamilyRegistry,
        phase: Phase,
        reservation: &Reservation,
        view: &View<'_>,
    ) -> Result<(), ActionStateError> {
        let verifier = self
            .0
            .iter()
            .find(|v| {
                v.effect == reservation.effect
                    // Selection is by the fixed Registry identity only; its
                    // verifier must reconstruct and compare the complete old
                    // reference from the exact persisted compatibility digest.
                    || registry.registry_rating.as_ref().is_some_and(|protocol| {
                        protocol.admission_actions().iter().any(|action| v.effect == **action && reservation.effect.locator() == action.locator())
                    })
                    // Paid recovery likewise selects only its fixed locator.
                    // The closed verifier then reconstructs the full funding
                    // identity from the frozen quote and participant refs.
                    || registry.original_purchase.as_ref().is_some_and(|protocol| {
                        v.effect == *protocol.admission_action()
                            && protocol.recognizes_historical(&reservation.effect)
                    })
                    || registry.entitlement_purchase.as_ref().is_some_and(|protocol| {
                        protocol.admission_actions().contains(&&v.effect)
                            && v.effect.key() == reservation.effect.key()
                            && protocol.recognizes_historical(&reservation.effect)
                    })
                    || registry.house_purchase.as_ref().is_some_and(|protocol| {
                        protocol.admission_actions().contains(&&v.effect)
                            && v.effect.key() == reservation.effect.key()
                            && protocol.recognizes_historical(&reservation.effect)
                    })
                    || registry.ic_mind_claim.as_ref().is_some_and(|protocol| {
                        v.effect == *protocol.admission_action()
                            && protocol.recognizes_historical(&reservation.effect)
                    })
                    || registry.core_integration.as_ref().is_some_and(|protocol| { v.effect == *protocol.admission_action() && protocol.recognizes_historical(&reservation.effect) })
                    || registry.core_refit.as_ref().is_some_and(|protocol| { v.effect == *protocol.admission_action() && protocol.recognizes_historical(&reservation.effect) })
                    || registry.mind_renewal.as_ref().is_some_and(|protocol| {
                        v.effect == *protocol.admission_action()
                            && protocol.recognizes_historical(&reservation.effect)
                    })
                    || registry.house_mind_renewal.as_ref().is_some_and(|protocol| {
                        v.effect == *protocol.admission_action()
                            && protocol.recognizes_historical(&reservation.effect)
                    })
            })
            .ok_or_else(|| refused("closed effect verifier unavailable"))?;
        registry
            .owner_readiness
            .require_ready(&reservation.sponsor.owner)
            .map_err(ActionStateError::OwnerNotReady)?;
        (verifier.verify)(phase, reservation, view)?;
        registry
            .owner_readiness
            .require_ready(&reservation.sponsor.owner)
            .map_err(ActionStateError::OwnerNotReady)
    }
}

struct View<'a> {
    registry: &'a StateFamilyRegistry,
    records: &'a BTreeMap<StateKey, StoredStateRecord>,
    mutations: Option<&'a BTreeMap<StateKey, TxnMutation>>,
}

impl View<'_> {
    fn bytes(&self, key: &StateKey) -> Option<usize> {
        match self.mutations.and_then(|m| m.get(key)) {
            Some(TxnMutation::Put { canonical, .. }) => Some(canonical.len()),
            Some(TxnMutation::Delete) => None,
            None => self.records.get(key).map(|r| r.canonical.len()),
        }
    }
}

#[derive(Default)]
struct Claims {
    create: Option<ReservationId>,
    existing: usize,
    ceiling: usize,
}

#[derive(Default)]
pub(super) struct CapacityState {
    records: BTreeMap<ReservationId, Arc<StoredReservation>>,
    targets: BTreeMap<StateKey, Claims>,
    sponsors: BTreeMap<StateKey, ReservationId>,
    metadata_bytes: usize,
}

impl CapacityState {
    pub(super) fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub(super) fn metadata_bytes(&self) -> usize {
        self.metadata_bytes
    }

    fn rebuild(
        registry: &StateFamilyRegistry,
        records: BTreeMap<ReservationId, Arc<StoredReservation>>,
        view: &View<'_>,
    ) -> Result<Self, ActionStateError> {
        let mut out = Self {
            records,
            ..Self::default()
        };
        for (id, stored) in &out.records {
            let reservation = &stored.record;
            let sponsor = reservation.sponsor.resolve(registry)?;
            if view.bytes(&sponsor).is_none()
                || out.sponsors.insert(sponsor.clone(), id.clone()).is_some()
            {
                return Err(refused("missing or reused operation backlink"));
            }
            out.metadata_bytes = add(out.metadata_bytes, stored.len())?;
            let mut previous = None;
            for target in &reservation.targets {
                let key = target.key.resolve(registry)?;
                if previous.as_ref().is_some_and(|last| last >= &key) {
                    return Err(refused("noncanonical target order"));
                }
                previous = Some(key.clone());
                let descriptor = registry
                    .entries
                    .get(&(key.owner.clone(), key.family.clone()))
                    .ok_or_else(|| refused("unknown target family"))?
                    .descriptor();
                if target.schema != *descriptor.value_schema()
                    || target.schema_version != descriptor.schema_version()
                    || target.ceiling == 0
                    || target.ceiling > descriptor.max_record_bytes()
                {
                    return Err(refused("target schema or ceiling"));
                }
                let claims = out.targets.entry(key.clone()).or_default();
                match target.mode {
                    SlotMode::Create => {
                        if claims.create.is_some()
                            || claims.existing != 0
                            || view.bytes(&key).is_some()
                        {
                            return Err(refused("exclusive creation key occupied"));
                        }
                        claims.create = Some(id.clone());
                    }
                    SlotMode::IndustryOutput => {
                        if !core_integration::prospective_output(reservation, &target.key)
                            || claims.create.is_some()
                            || claims.existing != 0
                        {
                            return Err(refused("fixed Industry output claim differs"));
                        }
                        claims.create = Some(id.clone());
                    }
                    SlotMode::UpdateExisting => {
                        if claims.create.is_some() || view.bytes(&key).is_none() {
                            return Err(refused("required existing key unavailable"));
                        }
                        claims.existing = add(claims.existing, 1)?;
                    }
                }
                claims.ceiling = claims.ceiling.max(target.ceiling);
            }
            if !reservation.targets.iter().any(|target| {
                target.key == reservation.sponsor && target.mode == SlotMode::UpdateExisting
            }) {
                return Err(refused("operation terminal image is not reserved"));
            }
        }
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Usage {
    pub(super) bytes: usize,
    pub(super) records: usize,
}

impl Usage {
    fn check(self) -> Result<Self, ActionStateError> {
        if self.records > MAX_ACTION_STATE_BOOT_RECORDS {
            return Err(ActionStateError::LiveRecordLimit {
                attempted: self.records,
                max: MAX_ACTION_STATE_BOOT_RECORDS,
            });
        }
        if self.bytes > MAX_ACTION_STATE_BOOT_BYTES {
            return Err(ActionStateError::LiveBytesExceeded {
                attempted: self.bytes,
                max: MAX_ACTION_STATE_BOOT_BYTES,
            });
        }
        Ok(self)
    }
}

fn add(a: usize, b: usize) -> Result<usize, ActionStateError> {
    a.checked_add(b).ok_or(ActionStateError::ArithmeticOverflow)
}

fn subtract(a: usize, b: usize) -> Result<usize, ActionStateError> {
    a.checked_sub(b).ok_or(ActionStateError::ArithmeticOverflow)
}

fn charged(
    state: &CapacityState,
    view: &View<'_>,
    typed_bytes: usize,
    typed_records: usize,
) -> Result<Usage, ActionStateError> {
    let mut usage = Usage {
        bytes: add(typed_bytes, state.metadata_bytes)?,
        records: add(typed_records, state.records.len())?,
    };
    for (key, claims) in &state.targets {
        let actual = view.bytes(key);
        usage.bytes = add(
            usage.bytes,
            claims.ceiling.saturating_sub(actual.unwrap_or(0)),
        )?;
        if actual.is_none() {
            usage.records = add(usage.records, 1)?;
        }
    }
    usage.check()
}

pub(super) fn hydrate(
    registry: &StateFamilyRegistry,
    verifiers: &Verifiers,
    records: BTreeMap<ReservationId, Arc<StoredReservation>>,
    typed: &BTreeMap<StateKey, StoredStateRecord>,
    total_bytes: usize,
) -> Result<(CapacityState, Usage), ActionStateError> {
    let view = View {
        registry,
        records: typed,
        mutations: None,
    };
    // Every row must have an exact verifier, even an apparently empty business
    // deployment. No capacity prefix is ignored or treated as ordinary state.
    for record in records.values() {
        verifiers.verify(registry, Phase::Retained, &record.record, &view)?;
    }
    registry::audit_boot(&view, &records)?;
    paid::audit_boot(&view, &records)?;
    entitlement::audit_boot(&view, &records)?;
    house::audit_boot(&view, &records)?;
    charter_pending::audit_boot(&view, &records)?;
    paid_resale::audit_boot(&view, &records)?;
    renewal::audit_boot(&view, &records)?;
    house_renewal::audit_boot(&view, &records)?;
    ic_claim::audit_boot(&view, &records)?;
    core_integration::audit_boot(&view, &records)?;
    core_refit::audit_boot(&view, &records)?;
    let state = CapacityState::rebuild(registry, records, &view)?;
    let usage = charged(
        &state,
        &view,
        subtract(total_bytes, state.metadata_bytes)?,
        typed.len(),
    )?;
    Ok((state, usage))
}

/// Only the closed PrepareCtx path may populate these changes, after checking
/// the actual provider, invocation, exact transaction and validated post-images.
#[derive(Default)]
pub(super) struct Changes {
    funding: Option<Reservation>,
    terminal: Option<ReservationId>,
    progress: Option<ReservationId>,
    // Only the fixed Industry Complete path can retain its admitted delivery subset.
    industry_delivery: Option<Reservation>,
}

pub(super) struct MetadataWrite {
    pub(super) module_op: ModuleOp,
    pub(super) replay: ReplayWrite,
}

pub(super) struct PreparedCapacity {
    pub(super) state: Arc<CapacityState>,
    pub(super) usage: Usage,
    pub(super) writes: Vec<MetadataWrite>,
    pub(super) write_bytes: usize,
}

pub(super) fn prepare(
    store: &ActionStateStore,
    owner: &OwnerId,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: Changes,
    identity: Option<&RegisteredTxnIdentity>,
    typed_bytes: usize,
    typed_records: usize,
) -> Result<PreparedCapacity, ActionStateError> {
    if usize::from(changes.funding.is_some())
        + usize::from(changes.terminal.is_some())
        + usize::from(changes.progress.is_some())
        + usize::from(changes.industry_delivery.is_some())
        > 1
    {
        return Err(refused("one closed effect per action"));
    }
    let view = View {
        registry: &store.registry,
        records: &store.records,
        mutations: Some(mutations),
    };
    registry::validate_changes(store, mutations, &changes, identity)?;
    paid::validate_changes(store, mutations, &changes, identity)?;
    entitlement::validate_changes(store, mutations, &changes, identity)?;
    house::validate_changes(store, mutations, &changes, identity)?;
    charter_pending::validate_changes(store, mutations, &changes, identity)?;
    paid_resale::validate_changes(store, mutations, &changes, identity)?;
    renewal::validate_changes(store, mutations, &changes, identity)?;
    house_renewal::validate_changes(store, mutations, &changes, identity)?;
    ic_claim::validate_changes(store, mutations, &changes, identity)?;
    core_integration::validate_changes(store, mutations, &changes, identity)?;
    core_refit::validate_changes(store, mutations, &changes, identity)?;
    let mut replacement = None;
    let mut writes = Vec::new();
    let mut write_bytes = 0;
    if let Some(id) = &changes.progress {
        let stored = store
            .capacity
            .records
            .get(id)
            .ok_or_else(|| refused("missing retained reservation"))?;
        if &stored.record.sponsor.owner != owner {
            return Err(refused("progress owner"));
        }
        let admitted: BTreeMap<_, _> = stored
            .record
            .targets
            .iter()
            .map(|target| Ok((target.key.resolve(&store.registry)?, target)))
            .collect::<Result<_, ActionStateError>>()?;
        for (key, mutation) in mutations {
            let target = admitted
                .get(key)
                .ok_or_else(|| refused("unreserved progress key"))?;
            if target.mode != SlotMode::UpdateExisting
                && !(target.mode == SlotMode::IndustryOutput
                    && core_integration::prospective_output(&stored.record, &target.key))
            {
                return Err(refused("progress cannot consume a future creation key"));
            }
            match mutation {
                TxnMutation::Put { canonical, .. } if canonical.len() <= target.ceiling => (),
                _ => return Err(refused("progress must retain bounded existing images")),
            }
        }
        store
            .capacity_verifiers
            .verify(&store.registry, Phase::Retained, &stored.record, &view)?;
    }
    if let Some(record) = &changes.industry_delivery {
        let id = ReservationId(record.sponsor.owner.clone(), record.funding.to_hex());
        let old = store
            .capacity
            .records
            .get(&id)
            .ok_or_else(|| refused("delivery handoff reservation absent"))?;
        if metadata_owner(record) != owner {
            return Err(refused("delivery handoff owner"));
        }
        core_integration::require_delivery_handoff(&old.record, record)?;
        let before = View {
            registry: &store.registry,
            records: &store.records,
            mutations: None,
        };
        store
            .capacity_verifiers
            .verify(&store.registry, Phase::Retained, &old.record, &before)?;
        let admitted = old
            .record
            .targets
            .iter()
            .map(|t| Ok((t.key.resolve(&store.registry)?, t)))
            .collect::<Result<BTreeMap<_, _>, ActionStateError>>()?;
        for (key, mutation) in mutations {
            let target = admitted
                .get(key)
                .ok_or_else(|| refused("unreserved delivery handoff key"))?;
            if let TxnMutation::Put { canonical, .. } = mutation {
                if canonical.len() > target.ceiling {
                    return Err(refused("delivery handoff ceiling"));
                }
            }
        }
        store
            .capacity_verifiers
            .verify(&store.registry, Phase::Retained, record, &view)?;
        let next = Arc::new(encode(record.clone())?);
        if next.id() != old.id() || next.len() > old.len() {
            return Err(refused("delivery handoff metadata must fit admission"));
        }
        let mut records = store.capacity.records.clone();
        records.insert(id, Arc::clone(&next));
        write_bytes = next.len();
        writes.push(MetadataWrite {
            module_op: ModuleOp {
                ns: SmolStr::new(owner.as_str()),
                op: ModuleWrite::JsonSet {
                    key: SmolStr::new(next.path.as_str()),
                    value: next.document.clone(),
                },
            },
            replay: ReplayWrite::ModuleJsonSet {
                namespace: owner.clone(),
                key: next.path.clone(),
                value: next.document.clone(),
            },
        });
        replacement = Some(records);
    }
    if let Some(id) = &changes.terminal {
        let stored = store
            .capacity
            .records
            .get(id)
            .ok_or_else(|| refused("missing reservation"))?;
        let terminal_metadata_owner = metadata_owner(&stored.record);
        if terminal_metadata_owner != owner
            && !paid_resale::compound_terminal_owner(
                store,
                mutations,
                &stored.record,
                identity,
                owner,
            )?
        {
            return Err(refused("terminal owner"));
        }
        let sponsor = stored.record.sponsor.resolve(&store.registry)?;
        if !matches!(mutations.get(&sponsor), Some(TxnMutation::Put { .. })) {
            return Err(refused("terminal operation post-image required"));
        }
        let admitted: BTreeMap<_, _> = stored
            .record
            .targets
            .iter()
            .map(|t| Ok((t.key.resolve(&store.registry)?, t)))
            .collect::<Result<_, ActionStateError>>()?;
        for (key, mutation) in mutations {
            let Some(target) = admitted.get(key) else {
                // Only the configured Registry compound refund may add its
                // two exact new cause/status rows. Ordinary typed accounting
                // still charges them before custody; they were never reserved.
                if charter_pending::extra_terminal_key(
                    store,
                    mutations,
                    &stored.record,
                    identity,
                    key,
                )? || paid_resale::extra_terminal_key(
                    store,
                    mutations,
                    &stored.record,
                    identity,
                    key,
                )? {
                    continue;
                }
                return Err(refused("unreserved terminal key"));
            };
            if let TxnMutation::Put { canonical, .. } = mutation {
                if canonical.len() > target.ceiling {
                    return Err(refused("terminal ceiling exceeded"));
                }
            }
        }
        store
            .capacity_verifiers
            .verify(&store.registry, Phase::Terminal, &stored.record, &view)?;
        let mut records = store.capacity.records.clone();
        records.remove(id);
        replacement = Some(records);
        writes.push(MetadataWrite {
            module_op: ModuleOp {
                ns: SmolStr::new(terminal_metadata_owner.as_str()),
                op: ModuleWrite::Del {
                    key: SmolStr::new(stored.path.as_str()),
                },
            },
            replay: ReplayWrite::ModuleDelete {
                namespace: terminal_metadata_owner.clone(),
                key: stored.path.clone(),
            },
        });
    }
    if let Some(record) = changes.funding {
        if metadata_owner(&record) != owner {
            return Err(refused("funding owner"));
        }
        let stored = Arc::new(encode(record)?);
        let sponsor = stored.record.sponsor.resolve(&store.registry)?;
        if !matches!(mutations.get(&sponsor), Some(TxnMutation::Put { .. })) {
            return Err(refused("funding operation post-image required"));
        }
        for target in &stored.record.targets {
            let key = target.key.resolve(&store.registry)?;
            if view.bytes(&key).is_some_and(|bytes| bytes > target.ceiling) {
                return Err(refused("funding ceiling below current image"));
            }
        }
        store
            .capacity_verifiers
            .verify(&store.registry, Phase::Funding, &stored.record, &view)?;
        let mut records = store.capacity.records.clone();
        if records.insert(stored.id(), Arc::clone(&stored)).is_some() {
            return Err(refused("funding identity reused"));
        }
        write_bytes = stored.len();
        writes.push(MetadataWrite {
            module_op: ModuleOp {
                ns: SmolStr::new(owner.as_str()),
                op: ModuleWrite::JsonSet {
                    key: SmolStr::new(stored.path.as_str()),
                    value: stored.document.clone(),
                },
            },
            replay: ReplayWrite::ModuleJsonSet {
                namespace: owner.clone(),
                key: stored.path.clone(),
                value: stored.document.clone(),
            },
        });
        replacement = Some(records);
    }
    // Validate against both the old and complete proposed reservation index.
    // Releasing metadata does not allow this terminal to consume a foreign key.
    let changed = replacement.is_some();
    let state = match replacement {
        Some(records) => Arc::new(CapacityState::rebuild(&store.registry, records, &view)?),
        None => Arc::clone(&store.capacity),
    };
    for (key, mutation) in mutations {
        if store
            .capacity
            .targets
            .get(key)
            .and_then(|c| c.create.as_ref())
            .is_some_and(|id| {
                changes.terminal.as_ref() != Some(id)
                    && !changes
                        .industry_delivery
                        .as_ref()
                        .is_some_and(|r| r.funding.to_hex() == id.1 && &r.sponsor.owner == &id.0)
                    && !(changes.progress.as_ref() == Some(id)
                        && store.capacity.records.get(id).is_some_and(|record| {
                            core_integration::prospective_output(
                                &record.record,
                                &Key {
                                    owner: key.owner.clone(),
                                    family: key.family.clone(),
                                    encoded: key.encoded.clone(),
                                },
                            )
                        }))
            })
        {
            return Err(refused("reserved creation key"));
        }
        if matches!(mutation, TxnMutation::Delete)
            && state.targets.get(key).is_some_and(|c| c.existing != 0)
        {
            return Err(refused("required existing key deletion"));
        }
        if let Some(id) = store.capacity.sponsors.get(key) {
            if changes.terminal.as_ref() != Some(id)
                && !changes
                    .industry_delivery
                    .as_ref()
                    .is_some_and(|r| r.funding.to_hex() == id.1 && &r.sponsor.owner == &id.0)
            {
                let record = store
                    .capacity
                    .records
                    .get(id)
                    .ok_or_else(|| refused("index integrity"))?;
                store.capacity_verifiers.verify(
                    &store.registry,
                    Phase::Retained,
                    &record.record,
                    &view,
                )?;
            }
        }
    }
    let usage = if changed || state.is_empty() {
        charged(&state, &view, typed_bytes, typed_records)?
    } else {
        // Ordinary writes are O(touched keys), not a scan of all reservations.
        let mut usage = store.capacity_usage;
        for key in mutations.keys() {
            let old = store.records.get(key).map(|r| r.canonical.len());
            let new = view.bytes(key);
            let reserved = state.targets.get(key).map(|c| c.ceiling);
            usage.bytes = add(
                subtract(usage.bytes, old.unwrap_or(0).max(reserved.unwrap_or(0)))?,
                new.unwrap_or(0).max(reserved.unwrap_or(0)),
            )?;
            usage.records = subtract(
                usage.records,
                usize::from(old.is_some() || reserved.is_some()),
            )?;
            usage.records = add(
                usage.records,
                usize::from(new.is_some() || reserved.is_some()),
            )?;
        }
        usage.check()?
    };
    Ok(PreparedCapacity {
        state,
        usage,
        writes,
        write_bytes,
    })
}

#[cfg(test)]
mod tests;
