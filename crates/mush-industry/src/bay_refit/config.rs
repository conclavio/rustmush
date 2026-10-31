//! The trusted bay composition and the real acknowledged owner callbacks.
use super::records::{BayOffer, RefitEvidence};
use super::state::BayRefitState;
use super::{BayRefitError as Error, BayRefitPolicy, CADENCE};
use crate::workorder::WorkOrderId;
use mush_core::participant::{OwnerPrepareContext, ParticipantError};
use mush_core::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistry, DefinitionRegistryBuilder, ObjId, OwnerId, World,
};
use mush_economy::bay_refit::{
    BayOfferAttribution, BayOrderAttribution, BayOwnerAuthority, BayPaymentPolicy,
    BayRefitError as PayError, BaySubject,
};
use mush_inventory::{EndpointRef, Grade, bay_part::BayPartBinding};
use mush_tactics::component_refit::{RefitChoice, RefitPolicy};
use mush_tactics::unit::replacement::{
    ComponentState, RefitJournal, refit_component, registered_spec,
};
use mush_tactics::{CellId, ComponentId, TacticalUnit};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

fn bad<T>(_: T) -> Error {
    Error::Integrity
}

/// **The ruleset's part catalogue**: which Inventory commodity a component
/// definition is stocked as, and the grade a part taken out goes back at.
/// The ruleset owns both; the bay only joins a lot to a mount through them.
pub trait BayPartCatalog: Send + Sync + 'static {
    /// The commodity a part of `component` (a component definition) is.
    fn commodity(&self, component: &DefinitionRef) -> Option<CanonicalId>;
    /// The grade the part taken out returns to stores at.
    fn grade(&self, removed: &ComponentState) -> Option<Grade>;
}

/// **The composed bay refit**: the fee terms, the bays and their stores, the
/// refit family it may move hulls within, the ruleset's derivable cells and
/// part catalogue, the work each refit takes, and the Economy owner it pays
/// through.
#[derive(Clone)]
pub struct BayRefitConfig {
    pub(crate) payment: BayPaymentPolicy,
    pub(crate) policy: BayRefitPolicy,
    pub(crate) work_units: u64,
    pub(crate) stores: BTreeMap<ObjId, EndpointRef>,
    pub(crate) roots: BTreeSet<DefinitionRef>,
    pub(crate) derived: Vec<CellId>,
    pub(crate) parts: Arc<dyn BayPartCatalog>,
    pub(crate) economy_owner: DefinitionRef,
    pub(crate) definitions: Arc<OnceLock<DefinitionRegistry>>,
}
impl std::fmt::Debug for BayRefitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BayRefitConfig")
            .field("policy", &self.policy)
            .field("work_units", &self.work_units)
            .finish_non_exhaustive()
    }
}
impl BayRefitConfig {
    /// # Errors
    ///
    /// [`Error::Policy`] for a payment policy the kernel refuses, stores that
    /// are not exactly the policy's bays, fewer than two roots or more than
    /// 64, work outside `1..=`[`super::MAX_WORK_UNITS`], or an Economy owner
    /// other than `economy/initial_filing`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        payment: BayPaymentPolicy,
        work_units: u64,
        stores: BTreeMap<ObjId, EndpointRef>,
        roots: BTreeSet<DefinitionRef>,
        derived: Vec<CellId>,
        parts: Arc<dyn BayPartCatalog>,
        economy_owner: DefinitionRef,
    ) -> Result<Self, Error> {
        let policy_err = |s: &str| Error::Policy(s.to_owned());
        payment
            .validate()
            .map_err(|e| Error::Policy(e.to_string()))?;
        let facilities: Vec<ObjId> = payment.bays.iter().map(|b| b.facility).collect();
        let policy = BayRefitPolicy::new(payment.recipe.clone(), facilities.clone())?;
        if stores.keys().copied().collect::<Vec<_>>() != facilities
            || !(2..=mush_tactics::component_refit::MAX_REFIT_ROOTS).contains(&roots.len())
            || !(1..=super::MAX_WORK_UNITS).contains(&work_units)
            || economy_owner.key().owner.as_str() != "economy"
            || economy_owner.key().kind.as_str() != "owner_participant"
            || economy_owner.key().id.as_str() != "initial_filing"
        {
            return Err(policy_err("bay refit composition"));
        }
        Ok(Self {
            payment,
            policy,
            work_units,
            stores,
            roots,
            derived,
            parts,
            economy_owner,
            definitions: Arc::default(),
        })
    }
    /// The same composition with its own definitions cell, for one boot:
    /// Industry, Economy's callback and Tactics' policy built from one
    /// `fresh()` share the registry Industry captures at load.
    #[must_use]
    pub fn fresh(&self) -> Self {
        Self {
            definitions: Arc::default(),
            ..self.clone()
        }
    }
    pub fn policy(&self) -> &BayRefitPolicy {
        &self.policy
    }
    pub fn payment(&self) -> &BayPaymentPolicy {
        &self.payment
    }
    pub fn roots(&self) -> &BTreeSet<DefinitionRef> {
        &self.roots
    }
    pub fn derived(&self) -> &[CellId] {
        &self.derived
    }
    /// The exact reference of one of the six bay actions.
    ///
    /// # Errors
    ///
    /// The policy's, for an unknown action.
    pub fn action(&self, name: &str) -> Result<DefinitionRef, Error> {
        self.policy.action_ref(name)
    }
    /// Industry's bay owner participant, as the kernel pins it.
    ///
    /// # Errors
    ///
    /// The descriptor's.
    pub fn participant_descriptor(&self) -> Result<DefinitionDescriptor, Error> {
        let mut refs = super::ACTIONS
            .iter()
            .map(|a| self.action(a))
            .collect::<Result<Vec<_>, _>>()?;
        refs.push(self.policy.recipe().clone());
        refs.sort();
        refs.dedup();
        DefinitionDescriptor::new(
            DefinitionKind::new("owner_participant").map_err(bad)?,
            DefinitionId::new("bay_refit").map_err(bad)?,
            1,
            serde_json::json!({"format":1,"protocol":"bay_refit_owner_v1","index":super::records::INDEX_KEY,"policy":self.policy,"work_units":self.work_units,"max_orders":super::records::MAX_ORDERS}),
        )
        .map(|d| d.with_references(refs))
        .map_err(bad)
    }
    pub fn participant(&self) -> Result<DefinitionRef, Error> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(bad)?,
                self.participant_descriptor()?,
            )
            .map_err(bad)
    }
    /// Tactics' refit participant for this bay, as the kernel pins it.
    ///
    /// # Errors
    ///
    /// Tactics', for a root set it refuses.
    pub fn tactics_participant(&self) -> Result<DefinitionRef, Error> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tactics").map_err(bad)?,
                mush_tactics::component_refit::participant_descriptor(
                    &self.action(super::INSTALL)?,
                    &self.roots,
                    &self.derived,
                )
                .map_err(bad)?,
            )
            .map_err(bad)
    }
    /// Inventory's part custody binding: the three paid bay actions.
    ///
    /// # Errors
    ///
    /// Inventory's.
    pub fn part_binding(&self) -> Result<BayPartBinding, Error> {
        BayPartBinding::new(
            self.action(super::ACCEPT)?,
            self.action(super::INSTALL)?,
            self.action(super::CANCEL)?,
        )
        .map_err(bad)
    }
    /// The Economy fee leg, paying through Industry's bay owner.
    ///
    /// # Errors
    ///
    /// Economy's.
    pub fn economy_config(&self) -> Result<mush_economy::bay_refit::BayRefitConfig, Error> {
        mush_economy::bay_refit::BayRefitConfig::new(
            self.payment.clone(),
            self.participant()?,
            Arc::new(Acknowledged {
                config: self.clone(),
            }),
        )
        .map_err(bad)
    }
    /// The Tactics refit policy over this bay's acknowledged orders.
    #[must_use]
    pub fn tactics_policy(&self) -> BayTacticsPolicy {
        BayTacticsPolicy {
            config: self.clone(),
        }
    }
    pub(crate) fn registry(&self) -> Result<&DefinitionRegistry, Error> {
        self.definitions.get().ok_or(Error::Unavailable)
    }
}

/// What a refit to `target` from `current` moves: the components `current`
/// has and `target` lacks, and the reverse; at most one each way.
pub(crate) fn moves(
    registry: &DefinitionRegistry,
    current: &TacticalUnit,
    target: &DefinitionRef,
) -> Result<(Option<ComponentId>, Option<ComponentId>), Error> {
    let before = registered_spec(registry, current.schema()).map_err(bad)?;
    let after = registered_spec(registry, target).map_err(bad)?;
    let ids = |s: &mush_tactics::UnitSpec| -> Vec<ComponentId> {
        s.components.iter().map(|c| c.id.clone()).collect()
    };
    let (a, b) = (ids(&before), ids(&after));
    let out: Vec<_> = a.iter().filter(|c| !b.contains(c)).cloned().collect();
    let into: Vec<_> = b.iter().filter(|c| !a.contains(c)).cloned().collect();
    if out.len() > 1 || into.len() > 1 || (out.is_empty() && into.is_empty()) {
        return Err(Error::Unavailable);
    }
    Ok((out.first().cloned(), into.first().cloned()))
}

/// The part fitted fresh: intact, its cells at their starting values.
pub(crate) fn fresh(
    registry: &DefinitionRegistry,
    target: &DefinitionRef,
    hull: ObjId,
    component: &ComponentId,
) -> Result<ComponentState, Error> {
    let spec = registered_spec(registry, target).map_err(bad)?;
    let baseline = TacticalUnit::from_spec(&spec, hull).map_err(bad)?;
    ComponentState::capture(&baseline, component).map_err(bad)
}

/// The component definition `component` names in `unit`'s spec.
pub(crate) fn definition_of(
    registry: &DefinitionRegistry,
    unit: &DefinitionRef,
    component: &ComponentId,
) -> Result<DefinitionRef, Error> {
    registered_spec(registry, unit)
        .map_err(bad)?
        .components
        .iter()
        .find(|c| &c.id == component)
        .map(|c| c.definition.clone())
        .ok_or(Error::Integrity)
}

/// **The refit an offer makes of `current`**: the choice Tactics is handed
/// and the journal it will reproduce, checked against the offer's hull,
/// revision, roots and moves.
pub(crate) fn refit(
    registry: &DefinitionRegistry,
    config: &BayRefitConfig,
    offer: &BayOffer,
    current: &TacticalUnit,
    now: i64,
) -> Result<(RefitChoice, RefitJournal), Error> {
    if current.object() != offer.hull
        || current.revision() != offer.unit_revision
        || current.schema() != &offer.before
        || !config.roots.contains(&offer.before)
        || !config.roots.contains(&offer.target)
        || moves(registry, current, &offer.target)? != (offer.remove.clone(), offer.install.clone())
    {
        return Err(Error::Unavailable);
    }
    let install = offer
        .install
        .as_ref()
        .map(|c| fresh(registry, &offer.target, offer.hull, c).map(|s| (c.clone(), s)))
        .transpose()?;
    let (_, journal) = refit_component(
        registry,
        current,
        &offer.target,
        offer.remove.as_ref(),
        install.as_ref().map(|(c, s)| (c, s)),
        &config.derived,
        offer.unit_revision,
        now,
    )
    .map_err(|_| Error::Unavailable)?;
    Ok((
        RefitChoice::new(offer.target.clone(), offer.remove.clone(), install),
        journal,
    ))
}

/// Industry's bay state, owned by Industry and ready.
pub(crate) fn state(world: &World) -> Result<&BayRefitState, Error> {
    if world
        .module_state_owner(std::any::TypeId::of::<BayRefitState>())
        .is_none_or(|o| o.as_str() != crate::NS)
    {
        return Err(Error::Unavailable);
    }
    world
        .module_state::<BayRefitState>()
        .filter(|s| s.ready && s.index.is_some())
        .ok_or(Error::Unavailable)
}

/// **The Tactics policy**: authorize the refit of exactly the order's hull,
/// from its acknowledged, fully worked, unterminated order, to its target.
#[derive(Clone)]
pub struct BayTacticsPolicy {
    config: BayRefitConfig,
}
impl RefitPolicy for BayTacticsPolicy {
    type Request = WorkOrderId;
    fn authorize(
        &self,
        ctx: &OwnerPrepareContext<'_>,
        id: &WorkOrderId,
        current: &TacticalUnit,
    ) -> Result<RefitChoice, ParticipantError> {
        let refused = || ParticipantError::Authority;
        let inv = ctx.invocation();
        if ctx.action() != &self.config.action(super::INSTALL).map_err(|_| refused())?
            || inv.principal() != &(mush_core::PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &mush_core::PerceivedRef::SelfActor
            || !inv.targets().is_empty()
        {
            return Err(refused());
        }
        let s = state(ctx.world()).map_err(|_| refused())?;
        let row = s.orders.get(id).ok_or_else(refused)?;
        row.validate().map_err(|_| refused())?;
        if row.intent.offer.requester != inv.actor()
            || row.terminal.is_some()
            || !row.progress.production().is_complete()
        {
            return Err(refused());
        }
        super::authority::install(ctx.world(), &self.config, &row.intent).map_err(|_| refused())?;
        let registry = self.config.registry().map_err(|_| refused())?;
        refit(
            registry,
            &self.config,
            &row.intent.offer,
            current,
            ctx.world().now(),
        )
        .map(|(choice, _)| choice)
        .map_err(|_| refused())
    }
}

/// The Economy fee leg's view of Industry's acknowledged offers and orders.
struct Acknowledged {
    config: BayRefitConfig,
}
impl BayOwnerAuthority for Acknowledged {
    fn acknowledged_offer(
        &self,
        world: &World,
        subject: &BaySubject,
    ) -> Result<BayOfferAttribution, PayError> {
        let p = state(world).map_err(|_| PayError::Unavailable)?;
        let o = p.offers.get(&subject.offer).ok_or(PayError::Unavailable)?;
        o.validate().map_err(|_| PayError::Integrity)?;
        if o.policy != self.config.policy
            || o.digest().map_err(|_| PayError::Integrity)? != subject.offer_digest
            || o.requester != subject.requester
            || o.hull != subject.hull
            || o.unit_revision != subject.unit_revision
            || o.before != subject.before
            || o.target != subject.target
            || o.facility != subject.facility
            || o.fee_minor != subject.fee_minor
            || p.orders
                .values()
                .any(|v| v.intent.offer.operation == o.operation)
        {
            return Err(PayError::Unavailable);
        }
        Ok(BayOfferAttribution {
            operation: o.operation,
            digest: subject.offer_digest,
            action: o.action.clone(),
            operator: o.operator,
            observed_ic: o.observed_ic.clone(),
            observed_utc: o.observed_utc,
            expires_utc: o.expires_utc,
        })
    }
    fn acknowledged_ready_order(
        &self,
        world: &World,
        subject: &BaySubject,
    ) -> Result<BayOrderAttribution, PayError> {
        let state = state(world).map_err(|_| PayError::Unavailable)?;
        let row = state
            .orders
            .get(&WorkOrderId(subject.order))
            .ok_or(PayError::Unavailable)?;
        if row
            .intent
            .subject(&self.config.payment)
            .map_err(|_| PayError::Integrity)?
            != *subject
            || !row.progress.production().is_complete()
        {
            return Err(PayError::Unavailable);
        }
        let registry = self.config.registry().map_err(|_| PayError::Unavailable)?;
        let o = &row.intent.offer;
        let current =
            mush_tactics::component_replacement::current_refit_unit(world, o.requester, o.hull)
                .map_err(|_| PayError::Unavailable)?;
        let (_, journal) = refit(registry, &self.config, o, current, world.now())
            .map_err(|_| PayError::Unavailable)?;
        let digest = RefitEvidence::capture(&journal)
            .digest()
            .map_err(|_| PayError::Integrity)?;
        let required = o.recipe.work_units;
        let due_at = mush_core::ClockStamp {
            clock: subject.accepted_at.clock.clone(),
            at: subject
                .accepted_at
                .at
                .checked_add(
                    i64::try_from(required)
                        .map_err(|_| PayError::Invalid)?
                        .checked_mul(CADENCE)
                        .ok_or(PayError::Invalid)?,
                )
                .ok_or(PayError::Invalid)?,
        };
        Ok(BayOrderAttribution {
            acceptance: row.accepted,
            subject: subject.digest()?,
            owner_generation: state.generation,
            due_at,
            required_work: required,
            completed_work: required,
            refit: digest,
        })
    }
}
