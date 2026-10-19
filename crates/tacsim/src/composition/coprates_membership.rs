//! Explicit Coprates buyer membership policy. This does not publish a house,
//! catalogue entry, certificate, offer or paid right. A real product must attach
//! the registered guard as well as the preview rule before it can use this cut.
use std::any::TypeId;

use mush_core::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionValidationContext, OwnerId,
};
use mush_core::{
    ActionError, AuthorityGuard, AuthorityGuardContext, AuthorityGuardKind,
    AuthorityGuardValidator, Ctx, Flags, ModError, ObjId, ObjKind, Registrar, World,
};
use mush_economy::CatalogRef;
use mush_faction::{FactionId, FactionStandingProvider, FactionState};
use mush_market::{AvailabilityOutcome, AvailabilityProvider, AvailabilityRule};
use mush_org::OrgPredicateProvider;
use serde_json::json;

const OWNER: &str = "tacsim";
const KIND: &str = "buyer_eligibility";
const RULE: &str = "coprates_republic_member";
const MAX_CATALOG_COMPONENT: usize = 128;

fn error(_: impl std::fmt::Display) -> ModError {
    ModError::msg("Coprates membership configuration is invalid")
}

/// Trusted composition coordinates, never chosen by a buyer request. No default
/// Republic id or actual filed-product identity is invented here.
#[derive(Clone, Debug)]
pub struct CopratesMembership {
    republic: FactionId,
    catalog: CatalogRef,
}
impl CopratesMembership {
    pub fn new(republic: FactionId, catalog: CatalogRef) -> Result<Self, ModError> {
        for part in [&catalog.ruleset, &catalog.code] {
            if part.is_empty()
                || part.len() > MAX_CATALOG_COMPONENT
                || CanonicalId::new(part.as_str()).is_err()
            {
                return Err(error("catalogue"));
            }
        }
        Ok(Self { republic, catalog })
    }

    /// Exact test metadata for the selected catalogue. A recorded positive
    /// preview is not evidence that membership remains current at funding.
    pub fn availability_rule(&self) -> AvailabilityRule {
        AvailabilityRule::Test {
            skill: RULE.into(),
            threshold: 1,
            interval: 0,
        }
    }

    pub fn descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        DefinitionDescriptor::new(
            DefinitionKind::new(KIND).map_err(error)?,
            DefinitionId::new(RULE).map_err(error)?,
            1,
            json!({"format":1,"protocol":"coprates_membership_v1",
                "catalog":self.catalog,"standing":{"provider":"faction",
                "predicate":"member","faction":self.republic.0},
                "scope":"new_buyer_admission"}),
        )
        .map_err(error)
    }

    fn current(&self, world: &World, buyer: ObjId) -> bool {
        if world.get(buyer).is_none_or(|object| {
            object.kind != ObjKind::Player || object.flags.contains(Flags::GOING)
        }) || world
            .module_state_owner(TypeId::of::<FactionState>())
            .map(|owner| owner.as_str())
            != Some(mush_faction::NS)
        {
            return false;
        }
        FactionStandingProvider.eval(world, "member", &json!({"faction":self.republic.0}), buyer)
            == Some(true)
    }

    /// Explicit installation on the actual Tacsim registrar. Duplicate or
    /// conflicting capability registrations retain core's fail-closed rule.
    /// The returned handle alone does not grant an action or publish a product.
    pub fn register(&self, reg: &mut Registrar<'_>) -> Result<CopratesMembershipHandle, ModError> {
        if reg.module() != OWNER {
            return Err(error("owner"));
        }
        let descriptor = self.descriptor()?;
        let expected = descriptor.clone();
        reg.definition_validator(
            DefinitionKind::new(KIND).map_err(error)?,
            move |ctx: &DefinitionValidationContext<'_>| {
                let row = ctx.definition();
                if row.reference().key().owner.as_str() == OWNER
                    && row.reference().key().kind == *expected.kind()
                    && row.reference().key().id == *expected.id()
                    && row.reference().version() == expected.version()
                    && row.payload() == expected.payload()
                    && row.references().is_empty()
                    && !row.is_current()
                {
                    Ok(())
                } else {
                    Err("Coprates membership definition differs".into())
                }
            },
        )?;
        let definition = reg.definition(descriptor)?;
        let handle = CopratesMembershipHandle {
            policy: self.clone(),
            capability: CanonicalId::new(definition.digest().to_hex()).map_err(error)?,
            definition,
            owner: OwnerId::new(OWNER).map_err(error)?,
        };
        reg.authority_guard_validator(AuthorityGuardKind::Capability, handle.clone())?;
        Ok(handle)
    }
}
impl AvailabilityProvider for CopratesMembership {
    fn resolve(
        &self,
        ctx: &mut Ctx<'_>,
        rule: &AvailabilityRule,
        catalog: &CatalogRef,
        buyer: ObjId,
    ) -> Option<AvailabilityOutcome> {
        if catalog != &self.catalog {
            return None;
        }
        // Owning the catalogue but receiving different rule metadata is a
        // definite refusal, never a fall-through to another provider.
        let passed = rule == &self.availability_rule() && self.current(ctx.w, buyer);
        Some(AvailabilityOutcome {
            passed,
            hits: u8::from(passed),
            threshold: 1,
            at: ctx.w.now(),
            note: "Coprates buyer eligibility checked".into(),
        })
    }
}

/// Registry-created, non-serializable handle. The full policy definition digest
/// identifies the assertion; generation 1 is its protocol version, not an
/// invented faction epoch. Validation always reads actual current membership.
#[derive(Clone, Debug)]
pub struct CopratesMembershipHandle {
    policy: CopratesMembership,
    definition: DefinitionRef,
    capability: CanonicalId,
    owner: OwnerId,
}
impl CopratesMembershipHandle {
    pub fn definition(&self) -> &DefinitionRef {
        &self.definition
    }

    /// Providers must include this assertion and `definition()` in the actual
    /// server guard set for the exact configured product. This produces no
    /// caller-selected action or authority for another catalogue.
    pub fn guard(
        &self,
        world: &World,
        catalog: &CatalogRef,
        buyer: ObjId,
    ) -> Option<AuthorityGuard> {
        (catalog == &self.policy.catalog && self.policy.current(world, buyer)).then(|| {
            AuthorityGuard::Capability {
                provider: self.owner.clone(),
                actor: buyer,
                capability: self.capability.clone(),
                generation: 1,
            }
        })
    }
}
impl AuthorityGuardValidator for CopratesMembershipHandle {
    fn is_current(
        &self,
        context: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        let AuthorityGuard::Capability {
            provider,
            actor,
            capability,
            generation,
        } = guard
        else {
            return Ok(false);
        };
        Ok(provider == &self.owner
            && capability == &self.capability
            && *generation == 1
            && self.policy.current(context.world(), *actor))
    }
}

#[cfg(test)]
mod tests;
