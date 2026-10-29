//! Current authority is read independently from real Org/Economy state.
//! Accepted work keeps supplier consent; only fresh Accept asks for the operator's
//! current grants. Completion cannot borrow a departed operator's dispatch.
use super::{
    CoreClass, CoreFundingSubject, CoreIntegrationError as Error, CorePaymentPolicy,
    ScopedCoreAccount,
};
use super::{
    consent::{CoreOfferAttribution, CoreOfferAuthority},
    plans::FundingIntent,
    subject::{CORE3_ENTITLEMENT, DiePayment},
};
use crate::{
    Account, AccountStatus, EconomyStore, InstrumentDef, InstrumentId, InstrumentKind,
    InstrumentStatus, Vault, VaultStatus,
};
use mush_core::{ClockStamp, DefinitionRef, Flags, ObjId, ObjKind, World};
use mush_org::{AuthRequest, CapabilityId, OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use std::{
    any::TypeId,
    sync::{Arc, OnceLock},
};

#[derive(Clone)]
pub struct CoreIntegrationConfig {
    pub(crate) policy: CorePaymentPolicy,
    pub(crate) offer_authority: Arc<dyn CoreOfferAuthority>,
    pub(crate) industry_owner: DefinitionRef,
    pub(crate) definitions: Arc<OnceLock<mush_core::DefinitionRegistry>>,
}
impl CoreIntegrationConfig {
    pub fn new(
        policy: CorePaymentPolicy,
        industry_owner: DefinitionRef,
        offer_authority: Arc<dyn CoreOfferAuthority>,
    ) -> Result<Self, Error> {
        policy.validate()?;
        if industry_owner.key().owner.as_str() != "industry"
            || industry_owner.key().kind.as_str() != "owner_participant"
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            policy,
            industry_owner,
            offer_authority,
            definitions: Arc::default(),
        })
    }
    pub fn policy(&self) -> &CorePaymentPolicy {
        &self.policy
    }
    pub(crate) fn ready(&self, world: &World) -> Result<(), Error> {
        self.cleanup_ready(world)?;
        let defs = self.definitions.get().ok_or(Error::Unavailable)?;
        defs.get_exact(&self.industry_owner)
            .map_err(|_| Error::Unavailable)?;
        defs.get_exact(&self.policy.recipe)
            .map_err(|_| Error::Unavailable)?;

        Ok(())
    }
    pub(crate) fn cleanup_ready(&self, world: &World) -> Result<(), Error> {
        let defs = self.definitions.get().ok_or(Error::Unavailable)?;
        // Cleanup depends on actual owners, not a now-retired recipe or supplier grant.
        defs.require_owner_ready(&mush_core::OwnerId::new("industry").map_err(|_| Error::Invalid)?)
            .map_err(|_| Error::Unavailable)?;
        for owner in ["economy", "org", "inventory"] {
            defs.require_owner_ready(&mush_core::OwnerId::new(owner).map_err(|_| Error::Invalid)?)
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
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CurrentPaymentFacts {
    pub accounts: Vec<Account>,
    pub instruments: Vec<InstrumentDef>,
    pub input: Option<Vault>,
    pub supplier: Option<CoreOfferAttribution>,
    pub at: ClockStamp,
    pub utc: i64,
}
fn owned<T: 'static>(world: &World, owner: &str) -> Result<(), Error> {
    if world
        .module_state_owner(TypeId::of::<T>())
        .is_none_or(|o| o.as_str() != owner)
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn personal(world: &World, actor: ObjId) -> Result<(), Error> {
    if world
        .get(actor)
        .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn organization(world: &World, account: &ScopedCoreAccount) -> Result<(), Error> {
    let orgs = world
        .module_state::<OrgStore>()
        .ok_or(Error::Unavailable)?
        .state();
    let org = orgs.org(account.organization).ok_or(Error::Unavailable)?;
    let scope = orgs.scope(account.scope).ok_or(Error::Unavailable)?;
    if org.status != OrgStatus::Active || scope.org != org.id || scope.status != ScopeStatus::Active
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn scoped(
    world: &World,
    actor: ObjId,
    account: &ScopedCoreAccount,
    cap: &str,
    amount: i64,
) -> Result<(), Error> {
    organization(world, account)?;
    if world
        .get(actor)
        .is_none_or(|o| o.flags.contains(Flags::GOING))
        || !mush_org::authorize_in_world(
            world,
            &AuthRequest {
                actor,
                capability: CapabilityId::new(cap).ok_or(Error::Invalid)?,
                scope: account.scope,
                now: world.now(),
                amount: Some(amount),
            },
        )
        .is_allow()
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn accounts(
    world: &World,
    state: &EconomyStore,
    policy: &CorePaymentPolicy,
    subject: &CoreFundingSubject,
) -> Result<(Vec<Account>, Vec<InstrumentDef>), Error> {
    personal(world, subject.requester)?;
    if !state.is_title_issuer(&policy.issuer) {
        return Err(Error::Unavailable);
    }
    let instrument = state
        .snapshot()
        .instrument(&policy.instrument)
        .ok_or(Error::Unavailable)?;
    if instrument.kind != InstrumentKind::Currency
        || instrument.scale != 2
        || !instrument.transferable
        || instrument.status != InstrumentStatus::Active
    {
        return Err(Error::Unavailable);
    }
    let mut accounts = Vec::with_capacity(3);
    let payer = state
        .snapshot()
        .account(subject.payer)
        .ok_or(Error::Unavailable)?;
    if payer.owner != PrincipalId::Object(subject.requester)
        || payer.scope.is_some()
        || payer.status != AccountStatus::Active
        || payer.instrument != policy.instrument
    {
        return Err(Error::Unavailable);
    }
    accounts.push(payer.clone());
    // A heirloom or grey die pays only the ungoverning site's integrator (doc 17h cut M8b-2,
    // H2); a certified one pays the supplier and the chartered integrator.
    let payees = if subject.source.is_certified() {
        vec![&policy.supplier, &policy.integrator]
    } else {
        vec![policy.site(false)?.integrator]
    };
    for expected in payees {
        organization(world, expected)?;
        let account = state
            .snapshot()
            .account(expected.account)
            .ok_or(Error::Unavailable)?;
        if account.owner != PrincipalId::Organization(expected.organization)
            || account.scope != Some(expected.scope)
            || account.instrument != policy.instrument
            || account.status != AccountStatus::Active
        {
            return Err(Error::Unavailable);
        }
        accounts.push(account.clone());
    }
    let mut instruments = vec![instrument.clone()];
    if subject.class == CoreClass::III {
        let id = InstrumentId::new(CORE3_ENTITLEMENT).ok_or(Error::Invalid)?;
        let entitlement = state.snapshot().instrument(&id).ok_or(Error::Unavailable)?;
        if entitlement.kind != InstrumentKind::Entitlement
            || entitlement.scale != 0
            || entitlement.transferable
            || entitlement.status != InstrumentStatus::Active
        {
            return Err(Error::Unavailable);
        }
        let principal = PrincipalId::Object(subject.requester);
        if !state.snapshot().accounts_of(&principal).any(|a| {
            a.instrument == id
                && a.scope.is_none()
                && a.status == AccountStatus::Active
                && a.posted_balance.minor() > 0
        }) {
            return Err(Error::Unavailable);
        }
        instruments.push(entitlement.clone());
    }
    Ok((accounts, instruments))
}
pub(super) fn funding(
    config: &CoreIntegrationConfig,
    state: &EconomyStore,
    world: &World,
    subject: &CoreFundingSubject,
) -> Result<(FundingIntent, CurrentPaymentFacts), Error> {
    funding_at(config, state, world, subject, true)
}
pub(super) fn funding_recheck(
    config: &CoreIntegrationConfig,
    state: &EconomyStore,
    world: &World,
    intent: &FundingIntent,
) -> Result<CurrentPaymentFacts, Error> {
    let (actual, facts) = funding_at(config, state, world, &intent.subject, false)?;
    if &actual != intent {
        return Err(Error::Integrity);
    }
    Ok(facts)
}
fn funding_at(
    config: &CoreIntegrationConfig,
    state: &EconomyStore,
    world: &World,
    subject: &CoreFundingSubject,
    initial: bool,
) -> Result<(FundingIntent, CurrentPaymentFacts), Error> {
    config.ready(world)?;
    subject.validate()?;
    let policy = &config.policy;
    if subject.policy != policy.digest()? || subject.recipe != policy.recipe {
        return Err(Error::Integrity);
    }
    let at = world.world_ic_now();
    let utc = world.now();
    if (initial && at != subject.accepted_at)
        || at.clock != subject.accepted_at.clock
        || at.at < subject.accepted_at.at
        || at.at >= subject.expires_at.at
    {
        return Err(Error::Unavailable);
    }
    let supplier = config.offer_authority.acknowledged_offer(world, subject)?;
    supplier.validate(policy, subject)?;
    if utc < supplier.observed_utc || utc >= supplier.expires_utc {
        return Err(Error::Unavailable);
    }
    let site = policy.site(subject.source.is_certified())?;
    scoped(
        world,
        supplier.operator,
        site.integrator,
        site.operator_capability,
        1,
    )?;
    let vault = state
        .snapshot()
        .vault(subject.input_vault)
        .ok_or(Error::Unavailable)?;
    if vault.status != VaultStatus::Active || !crate::endpoint_state(world, vault.id).is_live() {
        return Err(Error::Unavailable);
    }
    let payment = if !subject.source.is_certified() {
        // Doc 17h cut M8b-2 (H2): the requester's own heirloom or grey lot, never stock.
        if policy.supplier_stock.contains(&vault.id)
            || vault.owner != PrincipalId::Object(subject.requester)
            || vault.scope.is_some()
            || !crate::may_take_from_vault(world, subject.requester, vault.id)
        {
            return Err(Error::Unavailable);
        }
        DiePayment::Uncertified
    } else if policy.supplier_stock.contains(&vault.id) {
        if vault.owner != PrincipalId::Organization(policy.supplier.organization)
            || vault.scope != Some(policy.supplier.scope)
        {
            return Err(Error::Unavailable);
        }
        scoped(
            world,
            supplier.operator,
            &policy.supplier,
            "vault.withdraw",
            1,
        )?;
        if !crate::may_take_from_vault(world, supplier.operator, vault.id) {
            return Err(Error::Unavailable);
        }
        DiePayment::SupplierStock
    } else {
        if vault.owner != PrincipalId::Object(subject.requester)
            || vault.scope.is_some()
            || !crate::may_take_from_vault(world, subject.requester, vault.id)
        {
            return Err(Error::Unavailable);
        }
        DiePayment::RequesterSupplied
    };
    let (accounts, instruments) = accounts(world, state, policy, subject)?;
    let intent = FundingIntent {
        subject: subject.clone(),
        policy: policy.clone(),
        supplier: supplier.clone(),
        payment,
    };
    intent.validate()?;
    Ok((
        intent,
        CurrentPaymentFacts {
            accounts,
            instruments,
            input: Some(vault.clone()),
            supplier: Some(supplier),
            at,
            utc,
        },
    ))
}
pub(super) fn completion(
    config: &CoreIntegrationConfig,
    state: &EconomyStore,
    world: &World,
    intent: &FundingIntent,
) -> Result<CurrentPaymentFacts, Error> {
    config.ready(world)?;
    intent.validate()?;
    if config.policy != intent.policy {
        return Err(Error::Unavailable);
    }
    let at = world.world_ic_now();
    if at.clock != intent.subject.accepted_at.clock
        || at.at < intent.subject.accepted_at.at
        || at.at >= intent.subject.expires_at.at
    {
        return Err(Error::Unavailable);
    }
    let (accounts, instruments) = accounts(world, state, &intent.policy, &intent.subject)?;
    // Accepted fixed work does not consult the former operator's membership,
    // grant, offer expiry, or current availability of the reserved stock offer.
    Ok(CurrentPaymentFacts {
        accounts,
        instruments,
        input: None,
        supplier: None,
        at,
        utc: world.now(),
    })
}
