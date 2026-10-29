//! The owner-attributed read path. No World mirror of grant authority exists.
use std::sync::Arc;

use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind};
use mush_core::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use mush_core::{ModError, ObjId, StateFamilyHandle, World};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::families::UnitGrantRegistry;
use super::record::*;
use super::resolve::{authorize_unit, validate_unit_grant};
use super::runtime::{PendingIntent, UnitAuthorityWorldFacts};
use crate::wiring::WorldPredicates;
use crate::{OrgStore, PrincipalId};

/// Public selections only. The issuer, identity, status and creation time come
/// from the authenticated command/coordinator, never these fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitGrantDraft {
    pub grantee: UnitGrantee,
    pub capability: UnitCapability,
    pub scope: UnitScope,
    #[serde(deserialize_with = "required_option")]
    pub valid_until: Option<i64>,
    pub delegable: bool,
    pub recovery: RecoveryPolicy,
    #[serde(deserialize_with = "required_option")]
    pub parent: Option<UnitGrantParent>,
}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum UnitGrantMutation {
    Issue(UnitGrantDraft),
    Revoke { grant: UnitGrantId },
}

#[derive(Clone, Debug, Serialize)]
pub enum UnitAuthorityRequest {
    Authorize {
        unit: ObjId,
        capability: UnitCapability,
        delegate: bool,
        mission: Option<MissionCommand>,
    },
    /// Advisory crew check for a deployment principal. The authenticated caller
    /// must currently be allowed to deploy this unit before learning whether
    /// the named operator may crew it. This never grants either right.
    CrewUse {
        unit: ObjId,
        operator: ObjId,
    },
    List {
        after: u64,
    },
    /// Used only by Org's own mutation provider and checked against its current
    /// pending input. The result is a proposal, not a portable authority token.
    Prepare(PendingIntent),
}

#[derive(Clone, Debug, Serialize)]
pub enum UnitAuthorityResponse {
    Authority {
        allowed: bool,
    },
    Grants {
        grants: Vec<UnitGrant>,
        more: bool,
    },
    Prepared {
        registry_revision: u64,
        grant: UnitGrant,
    },
}

pub struct UnitAuthorityReader {
    pub(crate) registry: StateFamilyHandle<(), UnitGrantRegistry>,
    pub(crate) facts: Arc<dyn UnitAuthorityWorldFacts>,
}

pub fn definition_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new("unit_authority").map_err(|e| ModError::msg(e.to_string()))?, 1,
        json!({"format":1,"purpose":"unit_authority","actor":"authenticated", "family":super::families::FAMILY}),
    ).map_err(|e| ModError::msg(e.to_string()))
}

pub(crate) fn evaluate(
    world: &World,
    factory: &dyn UnitAuthorityWorldFacts,
    registry: &UnitGrantRegistry,
    actor: ObjId,
    intent: &PendingIntent,
) -> Result<UnitGrant, OwnerReadError> {
    registry.validate().map_err(|_| OwnerReadError::Integrity)?;
    if intent.registry_revision != registry.revision || !intent.is_current(world.now()) {
        return Err(OwnerReadError::Refused);
    }
    let store = world
        .module_state::<OrgStore>()
        .ok_or(OwnerReadError::Unavailable)?;
    let facts = factory.facts(world);
    if !facts.principal_exists(actor) {
        return Err(OwnerReadError::Refused);
    }
    let predicates = WorldPredicates {
        w: world,
        providers: store.providers(),
    };
    match &intent.mutation {
        UnitGrantMutation::Issue(draft) => {
            let parent = match &draft.parent {
                Some(parent) => parent.clone(),
                None => {
                    let UnitScope::Units { units } = &draft.scope else {
                        return Err(OwnerReadError::Refused);
                    };
                    if units.len() != 1 {
                        return Err(OwnerReadError::Refused);
                    }
                    let unit = *units.first().ok_or(OwnerReadError::Refused)?;
                    let asset = facts.asset(unit).ok_or(OwnerReadError::Refused)?;
                    if !asset.active
                        || asset.unit != unit
                        || asset.owner != PrincipalId::Object(actor)
                    {
                        return Err(OwnerReadError::Refused);
                    }
                    UnitGrantParent::PersonalTitle {
                        unit,
                        owner: actor,
                        lineage: asset.lineage,
                    }
                }
            };
            let inherited_until = match &parent {
                UnitGrantParent::UnitGrant { id } => {
                    registry.grants.get(id).and_then(|g| g.valid_until)
                }
                _ => None,
            };
            let grant = UnitGrant {
                id: UnitGrantId(registry.next_id),
                revision: 1,
                grantor: actor,
                grantee: draft.grantee.clone(),
                capability: draft.capability,
                scope: draft.scope.clone(),
                valid_from: Some(intent.created_at),
                valid_until: draft.valid_until.or(inherited_until),
                delegable: draft.delegable,
                recovery: draft.recovery,
                parent,
                status: UnitGrantStatus::Active,
            };
            validate_unit_grant(
                store.state(),
                &registry.grants,
                facts.as_ref(),
                &predicates,
                &grant,
                world.now(),
            )
            .map_err(|_| OwnerReadError::Refused)?;
            registry
                .with_grant(grant.clone())
                .map_err(|_| OwnerReadError::Limit)?;
            Ok(grant)
        }
        UnitGrantMutation::Revoke { grant } => {
            let mut row = registry
                .grants
                .get(grant)
                .cloned()
                .ok_or(OwnerReadError::Refused)?;
            if row.grantor != actor || row.status != UnitGrantStatus::Active {
                return Err(OwnerReadError::Refused);
            }
            row.revision = row.revision.checked_add(1).ok_or(OwnerReadError::Limit)?;
            row.status = UnitGrantStatus::Revoked;
            registry
                .with_grant(row.clone())
                .map_err(|_| OwnerReadError::Limit)?;
            Ok(row)
        }
    }
}

impl OwnerReadService for UnitAuthorityReader {
    type State = OrgStore;
    type Request = UnitAuthorityRequest;
    type Output = UnitAuthorityResponse;
    fn read(
        &self,
        store: &OrgStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        let registry = ctx
            .action_state(&self.registry, &())?
            .cloned()
            .unwrap_or_default();
        registry.validate().map_err(|_| OwnerReadError::Integrity)?;
        match request {
            UnitAuthorityRequest::Authorize {
                unit,
                capability,
                delegate,
                mission,
            } => {
                let facts = self.facts.facts(ctx.world());
                let predicates = WorldPredicates {
                    w: ctx.world(),
                    providers: store.providers(),
                };
                let request = UnitAuthRequest {
                    actor: ctx.actor(),
                    unit,
                    action: if delegate {
                        UnitAction::Delegate(capability)
                    } else {
                        UnitAction::Exercise(capability)
                    },
                    now: ctx.world().now(),
                    mission,
                };
                let allowed = authorize_unit(
                    store.state(),
                    &registry.grants,
                    facts.as_ref(),
                    &predicates,
                    &request,
                )
                .is_allow();
                Ok(UnitAuthorityResponse::Authority { allowed })
            }
            UnitAuthorityRequest::CrewUse { unit, operator } => {
                let facts = self.facts.facts(ctx.world());
                let predicates = WorldPredicates {
                    w: ctx.world(),
                    providers: store.providers(),
                };
                let allowed = |actor, capability| {
                    authorize_unit(
                        store.state(),
                        &registry.grants,
                        facts.as_ref(),
                        &predicates,
                        &UnitAuthRequest {
                            actor,
                            unit,
                            action: UnitAction::Exercise(capability),
                            now: ctx.world().now(),
                            mission: None,
                        },
                    )
                    .is_allow()
                };
                if !allowed(ctx.actor(), UnitCapability::Deploy) {
                    return Err(OwnerReadError::Authority);
                }
                Ok(UnitAuthorityResponse::Authority {
                    allowed: allowed(operator, UnitCapability::OperateCrew),
                })
            }
            UnitAuthorityRequest::List { after } => {
                let mut rows = registry.grants.values().filter(|g| {
                    g.id.0 > after
                        && (g.grantor == ctx.actor()
                            || g.grantee == UnitGrantee::Character(ctx.actor()))
                });
                let grants: Vec<_> = rows.by_ref().take(32).cloned().collect();
                Ok(UnitAuthorityResponse::Grants {
                    grants,
                    more: rows.next().is_some(),
                })
            }
            UnitAuthorityRequest::Prepare(intent) => {
                if ctx.requester().as_str() != crate::NS
                    || super::runtime::pending(ctx.world(), ctx.actor()) != Some(&intent)
                {
                    return Err(OwnerReadError::Authority);
                }
                let grant = evaluate(
                    ctx.world(),
                    self.facts.as_ref(),
                    &registry,
                    ctx.actor(),
                    &intent,
                )?;
                Ok(UnitAuthorityResponse::Prepared {
                    registry_revision: registry.revision,
                    grant,
                })
            }
        }
    }
}
