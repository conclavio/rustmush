//! Independent current bay, payer and hull-title checks. Supplier membership
//! is required for fresh acceptance, never inherited as the requester's
//! identity and never re-required for already accepted fixed work.
use super::plans::FundingIntent;
use super::{
    BayOfferAttribution, BayOrderAttribution, BayOwnerAuthority, BayPaymentPolicy,
    BayRefitError as Error, BaySubject,
};
use crate::core_integration::ScopedCoreAccount;
use crate::{
    Account, AccountStatus, AssetLifecycle, EconomicAsset, EconomyStore, InstrumentDef,
    InstrumentKind, InstrumentStatus,
};
use mush_core::{ClockStamp, CommitToken, DefinitionRef, Flags, ObjId, ObjKind, World};
use mush_org::{AuthRequest, CapabilityId, OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use std::{
    any::TypeId,
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};
/// **The composed bay fee leg**: the policy, the Industry bay owner it
/// serves, and the callback that reads Industry's acknowledged offers and
/// ready orders.
#[derive(Clone)]
pub struct BayRefitConfig {
    pub(crate) policy: BayPaymentPolicy,
    pub(crate) authority: Arc<dyn BayOwnerAuthority>,
    pub(crate) industry_owner: DefinitionRef,
    pub(crate) definitions: Arc<OnceLock<mush_core::DefinitionRegistry>>,
}
impl std::fmt::Debug for BayRefitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BayRefitConfig")
            .field("policy", &self.policy)
            .field("industry_owner", &self.industry_owner)
            .finish_non_exhaustive()
    }
}
impl BayRefitConfig {
    /// # Errors
    ///
    /// [`Error::Invalid`] for a policy that will not validate, or an owner
    /// other than Industry's `bay_refit` participant.
    pub fn new(
        policy: BayPaymentPolicy,
        industry_owner: DefinitionRef,
        authority: Arc<dyn BayOwnerAuthority>,
    ) -> Result<Self, Error> {
        policy.validate()?;
        if industry_owner.key().owner.as_str() != "industry"
            || industry_owner.key().kind.as_str() != "owner_participant"
            || industry_owner.key().id.as_str() != "bay_refit"
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            policy,
            authority,
            industry_owner,
            definitions: Arc::default(),
        })
    }
    pub fn policy(&self) -> &BayPaymentPolicy {
        &self.policy
    }
    pub(crate) fn cleanup_ready(&self, world: &World) -> Result<(), Error> {
        let d = self.definitions.get().ok_or(Error::Unavailable)?;
        for owner in ["economy", "industry", "inventory", "org", "tactics", "sim"] {
            d.require_owner_ready(&mush_core::OwnerId::new(owner).map_err(|_| Error::Invalid)?)
                .map_err(|_| Error::Unavailable)?;
        }
        owned::<EconomyStore>(world, "economy")?;
        owned::<OrgStore>(world, "org")?;
        if world
            .module_state::<EconomyStore>()
            .is_none_or(EconomyStore::is_frozen)
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
    pub(crate) fn ready(&self, world: &World) -> Result<(), Error> {
        self.cleanup_ready(world)?;
        let d = self.definitions.get().ok_or(Error::Unavailable)?;
        for r in [&self.industry_owner, &self.policy.recipe] {
            d.get_exact(r).map_err(|_| Error::Unavailable)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CurrentFacts {
    pub accounts: Vec<Account>,
    pub instruments: Vec<InstrumentDef>,
    pub titles: Vec<EconomicAsset>,
    pub supplier: Option<BayOfferAttribution>,
    pub order: Option<BayOrderAttribution>,
    pub at: ClockStamp,
    pub utc: i64,
}
fn owned<T: 'static>(w: &World, owner: &str) -> Result<(), Error> {
    if w.module_state_owner(TypeId::of::<T>())
        .is_none_or(|o| o.as_str() != owner)
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn live(w: &World, id: ObjId, kind: Option<ObjKind>) -> Result<(), Error> {
    if w.get(id)
        .is_none_or(|o| o.flags.contains(Flags::GOING) || kind.is_some_and(|k| o.kind != k))
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn organization(w: &World, p: &ScopedCoreAccount) -> Result<(), Error> {
    let state = w
        .module_state::<OrgStore>()
        .ok_or(Error::Unavailable)?
        .state();
    let org = state.org(p.organization).ok_or(Error::Unavailable)?;
    let scope = state.scope(p.scope).ok_or(Error::Unavailable)?;
    if org.status != OrgStatus::Active || scope.org != org.id || scope.status != ScopeStatus::Active
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn title(
    state: &EconomyStore,
    id: ObjId,
    owner: &PrincipalId,
    scope: Option<mush_org::OrgScopeId>,
) -> Result<EconomicAsset, Error> {
    let a = state.snapshot().asset(id).ok_or(Error::Unavailable)?;
    if &a.title_owner != owner
        || a.scope != scope
        || a.lifecycle != AssetLifecycle::Active
        || a.frozen
        || a.stolen.is_some()
        || !a.encumbrances.is_empty()
    {
        return Err(Error::Unavailable);
    }
    Ok(a.clone())
}
/// The current facts the fee rests on: the window, the requester and the
/// hull at the bay, the bay's organization, the currency, both accounts, the
/// hull's and the bay's titles, and no vault backing on either's chain.
fn current(
    config: &BayRefitConfig,
    state: &EconomyStore,
    w: &World,
    s: &BaySubject,
) -> Result<CurrentFacts, Error> {
    config.ready(w)?;
    s.matches_policy(&config.policy)?;
    let p = &config.policy;
    let site = p.site(s.facility)?;
    let at = w.world_ic_now();
    if at.clock != s.accepted_at.clock || at.at < s.accepted_at.at || at.at >= s.expires_at.at {
        return Err(Error::Unavailable);
    }
    live(w, s.requester, Some(ObjKind::Player))?;
    live(w, site.facility, None)?;
    live(w, s.hull, Some(ObjKind::Thing))?;
    if w.get(s.requester)
        .is_none_or(|o| o.location != Some(site.facility))
        || w.get(s.hull)
            .is_none_or(|o| o.location != Some(site.facility))
    {
        return Err(Error::Unavailable);
    }
    organization(w, &site.provider)?;
    let currency = state
        .snapshot()
        .instrument(&p.instrument)
        .ok_or(Error::Unavailable)?;
    if currency.kind != InstrumentKind::Currency
        || currency.scale != 2
        || !currency.transferable
        || currency.status != InstrumentStatus::Active
    {
        return Err(Error::Unavailable);
    }
    let payer = state
        .snapshot()
        .account(s.payer)
        .ok_or(Error::Unavailable)?;
    let provider = state
        .snapshot()
        .account(site.provider.account)
        .ok_or(Error::Unavailable)?;
    if payer.owner != PrincipalId::Object(s.requester)
        || payer.scope.is_some()
        || payer.status != AccountStatus::Active
        || payer.instrument != p.instrument
        || provider.owner != PrincipalId::Organization(site.provider.organization)
        || provider.scope != Some(site.provider.scope)
        || provider.status != AccountStatus::Active
        || provider.instrument != p.instrument
    {
        return Err(Error::Unavailable);
    }
    let titles = vec![
        title(state, s.hull, &PrincipalId::Object(s.requester), None)?,
        title(
            state,
            site.facility,
            &PrincipalId::Organization(site.provider.organization),
            Some(site.provider.scope),
        )?,
    ];
    for start in [s.requester, s.hull] {
        let mut seen = BTreeSet::new();
        let mut next = Some(start);
        while let Some(id) = next {
            if seen.len() >= mush_core::MAX_CONTAINMENT_DEPTH
                || !seen.insert(id)
                || state.snapshot().is_vault_backing(id)
            {
                return Err(Error::Unavailable);
            }
            let row = w
                .get(id)
                .filter(|o| !o.flags.contains(Flags::GOING))
                .ok_or(Error::Unavailable)?;
            next = row.location;
        }
    }
    Ok(CurrentFacts {
        accounts: vec![payer.clone(), provider.clone()],
        instruments: vec![currency.clone()],
        titles,
        supplier: None,
        order: None,
        at,
        utc: w.now(),
    })
}
pub(super) fn funding(
    config: &BayRefitConfig,
    state: &EconomyStore,
    w: &World,
    s: &BaySubject,
    initial: bool,
) -> Result<(FundingIntent, CurrentFacts), Error> {
    let mut facts = current(config, state, w, s)?;
    if initial && facts.at != s.accepted_at {
        return Err(Error::Integrity);
    }
    let supplier = config.authority.acknowledged_offer(w, s)?;
    supplier.validate(&config.policy, s)?;
    if facts.utc < supplier.observed_utc || facts.utc >= supplier.expires_utc {
        return Err(Error::Unavailable);
    }
    live(w, supplier.operator, None)?;
    if !mush_org::authorize_in_world(
        w,
        &AuthRequest {
            actor: supplier.operator,
            capability: CapabilityId::new(super::OPERATOR_CAPABILITY).ok_or(Error::Invalid)?,
            scope: config.policy.site(s.facility)?.provider.scope,
            now: w.now(),
            amount: Some(1),
        },
    )
    .is_allow()
    {
        return Err(Error::Unavailable);
    }
    facts.supplier = Some(supplier.clone());
    let intent = FundingIntent {
        subject: s.clone(),
        policy: config.policy.clone(),
        supplier,
    };
    intent.validate()?;
    Ok((intent, facts))
}
pub(super) fn installation(
    config: &BayRefitConfig,
    state: &EconomyStore,
    w: &World,
    intent: &FundingIntent,
    acceptance: CommitToken,
) -> Result<CurrentFacts, Error> {
    intent.validate()?;
    if config.policy != intent.policy {
        return Err(Error::Unavailable);
    }
    let mut facts = current(config, state, w, &intent.subject)?;
    let order = config
        .authority
        .acknowledged_ready_order(w, &intent.subject)?;
    if order.acceptance != acceptance
        || order.subject != intent.subject.digest()?
        || order.due_at.clock != facts.at.clock
        || order.due_at.at < intent.subject.accepted_at.at
        || order.due_at.at >= intent.subject.expires_at.at
        || facts.at.at < order.due_at.at
        || order.required_work == 0
        || order.completed_work != order.required_work
        || order.refit.as_bytes() == &[0; 32]
    {
        return Err(Error::Integrity);
    }
    facts.order = Some(order);
    Ok(facts)
}
