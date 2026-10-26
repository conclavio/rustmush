use super::{
    LaunchDeployment, LaunchRecipe,
    admission::CopratesAdmission,
    content::{self, bad},
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry, Flags, ModError,
    ModuleCtx, ObjId, ObjKind, Registrar, World,
};
use mush_economy::{EconomyStore, InstrumentId};
use mush_org::{AuthRequest, CapabilityId, OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use std::{
    any::TypeId,
    sync::{Arc, OnceLock},
};

#[derive(Clone)]
pub(crate) struct Registration {
    deployment: Arc<LaunchDeployment>,
    admission: CopratesAdmission,
    identity: DefinitionDescriptor,
    registry: Arc<OnceLock<DefinitionRegistry>>,
}
/// Runtime selector for the existing workshop, installed only by the real owner.
#[derive(Clone)]
pub(crate) struct LaunchService(pub(crate) Registration);
impl Registration {
    pub(super) fn new(
        deployment: LaunchDeployment,
        admission: CopratesAdmission,
    ) -> Result<Self, ModError> {
        let mut identity = serde_json::to_value(&deployment).map_err(bad)?;
        identity
            .as_object_mut()
            .ok_or_else(|| bad("launch identity"))?
            .remove("mode");
        let identity = DefinitionDescriptor::new(
            DefinitionKind::new("launch_deployment").map_err(bad)?,
            DefinitionId::new("named_mind_launch").map_err(bad)?,
            1,
            identity,
        )
        .map_err(bad)?;
        Ok(Self {
            deployment: Arc::new(deployment),
            admission,
            identity,
            registry: Arc::default(),
        })
    }
    pub(crate) fn register(&self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if reg.module() != crate::NS {
            return Err(bad("launch owner differs"));
        }
        for descriptor in [
            content::descriptor()?,
            self.identity.clone(),
            self.admission.descriptor().clone(),
        ] {
            reg.definition_validator(
                descriptor.kind().clone(),
                crate::definitions::exact_validator(vec![descriptor.clone()]),
            )?;
            reg.definition(descriptor)?;
        }
        reg.state(LaunchService(self.clone()));
        Ok(())
    }
    pub(crate) fn finalize(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        let definitions = ctx
            .reg
            .definitions()
            .ok_or_else(|| bad("launch definitions missing"))?;
        for owner in ["org", "economy", "faction"] {
            definitions
                .require_owner_ready(&mush_core::OwnerId::new(owner).map_err(bad)?)
                .map_err(bad)?;
        }
        self.actual_identities(ctx.w)?;
        self.admission.capture(definitions)?;
        self.registry
            .set(definitions.clone())
            .map_err(|_| bad("launch runtime already captured"))
    }
    fn actual_identities(&self, world: &World) -> Result<(), ModError> {
        if world
            .module_state_owner(TypeId::of::<OrgStore>())
            .map(|o| o.as_str())
            != Some("org")
            || world
                .module_state_owner(TypeId::of::<EconomyStore>())
                .map(|o| o.as_str())
                != Some("economy")
        {
            return Err(bad("launch owners missing"));
        }
        let orgs = world
            .module_state::<OrgStore>()
            .ok_or_else(|| bad("launch organization owner"))?;
        let economy = world
            .module_state::<EconomyStore>()
            .ok_or_else(|| bad("launch economy owner"))?;
        if economy.is_frozen() {
            return Err(bad("launch economy unavailable"));
        }
        let snapshot = economy.snapshot();
        let credit =
            InstrumentId::new(crate::economy::CREDIT).ok_or_else(|| bad("credit identity"))?;
        for house in [
            &self.deployment.houses.meridiani,
            &self.deployment.houses.coprates,
            &self.deployment.houses.tycho,
        ] {
            orgs.state()
                .org(house.organization)
                .ok_or_else(|| bad("named launch organization missing"))?;
            let scope = orgs
                .state()
                .scope(house.scope)
                .ok_or_else(|| bad("named launch scope missing"))?;
            let treasury = snapshot
                .account(house.treasury)
                .ok_or_else(|| bad("named launch treasury missing"))?;
            if scope.org != house.organization
                || treasury.owner != PrincipalId::Organization(house.organization)
                || treasury.scope != Some(house.scope)
                || treasury.instrument != credit
                || world
                    .get(house.publisher)
                    .is_none_or(|o| o.kind != ObjKind::Player)
            {
                return Err(bad("named launch identities disagree"));
            }
        }
        for recipient in [&self.deployment.registry, &self.deployment.venue.recipient] {
            let account = snapshot
                .account(recipient.account)
                .ok_or_else(|| bad("launch recipient missing"))?;
            if account.owner != recipient.owner || account.instrument != credit {
                return Err(bad("launch recipient disagrees"));
            }
        }
        if world
            .module_state_owner(TypeId::of::<mush_faction::FactionState>())
            .map(|o| o.as_str())
            != Some(mush_faction::NS)
            || world
                .module_state::<mush_faction::FactionState>()
                .is_none_or(|s| !s.factions.contains_key(&self.deployment.republic))
        {
            return Err(bad("launch Republic faction missing"));
        }
        if world
            .get(self.deployment.venue.room)
            .is_none_or(|o| o.kind != ObjKind::Room)
        {
            return Err(bad("launch venue missing"));
        }
        // 17g C1b: format 2's market identities, in the House check's shape:
        // existence, ownership and kind only. Status is deliberately NOT
        // checked here: the org's `OrgStatus`, the scope's `ScopeStatus`, an
        // operator's GOING flag and the operators' grants are runtime states,
        // re-checked by the recall and revocation providers on every offer and
        // invocation (`registry_recall/authority.rs::observe`,
        // `registry_revocation/authority.rs`). Suspending the Registry or
        // retiring its scope is the designed kill switch, and a boot refusal
        // would turn it into an outage. A destroyed clerk or operator does
        // refuse, as a destroyed House publisher does: edit the deployment.
        if let Some(market) = &self.deployment.market {
            let authority = &market.registry_authority;
            let player = |id: ObjId| world.get(id).is_some_and(|o| o.kind == ObjKind::Player);
            if orgs.state().org(authority.organization).is_none()
                || orgs
                    .state()
                    .scope(authority.scope)
                    .is_none_or(|s| s.org != authority.organization)
                || !player(market.clerk)
                || !authority.operators.iter().all(|operator| player(*operator))
            {
                return Err(bad("named launch market identities disagree"));
            }
        }
        Ok(())
    }
    /// 17g C1b: the decoded deployment this launch registered. 17g C1c's bare
    /// `@mind/launch` reads its format-2 market for the market and readiness
    /// lines.
    pub(crate) fn deployment(&self) -> &LaunchDeployment {
        &self.deployment
    }
    #[cfg(test)]
    pub(crate) fn check_buyer(
        &self,
        world: &World,
        issuer: mush_roll::commerce::house_catalogue::HouseIssuer,
        buyer: ObjId,
    ) -> Result<(), mush_roll::commerce::records::PublicationError> {
        use mush_roll::commerce::house_purchase::HouseBuyerAdmission;
        self.admission.check(world, issuer, buyer)
    }
    pub(crate) fn import(
        &self,
        world: &World,
        actor: ObjId,
        recipe: LaunchRecipe,
        name: &str,
    ) -> Result<mush_roll::PreparedChange, mush_roll::RollError> {
        fn refused<T>(_: T) -> mush_roll::RollError {
            mush_roll::RollError::Missing
        }
        let registry = self.registry.get().ok_or(mush_roll::RollError::Missing)?;
        for owner in ["org", "economy", crate::NS] {
            registry
                .require_owner_ready(&mush_core::OwnerId::new(owner).map_err(refused)?)
                .map_err(refused)?;
        }
        let house = match recipe {
            LaunchRecipe::Meridiani => &self.deployment.houses.meridiani,
            LaunchRecipe::Coprates => &self.deployment.houses.coprates,
            LaunchRecipe::SafeConduct => &self.deployment.houses.tycho,
        };
        if actor != house.publisher
            || name != house.source_name
            || world
                .get(actor)
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        {
            return Err(mush_roll::RollError::Missing);
        }
        let orgs = world
            .module_state::<OrgStore>()
            .ok_or(mush_roll::RollError::Missing)?;
        if orgs
            .state()
            .org(house.organization)
            .is_none_or(|o| o.status != OrgStatus::Active)
            || orgs
                .state()
                .scope(house.scope)
                .is_none_or(|s| s.org != house.organization || s.status != ScopeStatus::Active)
        {
            return Err(mush_roll::RollError::Missing);
        }
        let request = AuthRequest {
            actor,
            capability: CapabilityId::new(
                mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY,
            )
            .ok_or(mush_roll::RollError::Missing)?,
            scope: house.scope,
            now: world.now(),
            amount: None,
        };
        if !mush_org::authorize_in_world(world, &request).is_allow() {
            return Err(mush_roll::RollError::Missing);
        }
        world
            .module_state::<mush_roll::RollStore>()
            .ok_or(mush_roll::RollError::Missing)?
            .prepare_create(actor, name, recipe.draft().map_err(refused)?)
    }
}
