//! Occator explicitly lends operational rights through the real Org registry.
//! Economic lease consent is a condition on that loan, never permission itself.
use super::{PortMap, def::VehiclesDef};
use mush_core::{Ctx, ObjId, ObjKind};
use mush_org::unit_authority::reader::{
    UnitAuthorityRequest, UnitAuthorityResponse, UnitGrantDraft, UnitGrantMutation,
};
use mush_org::unit_authority::{
    RecoveryPolicy, UnitCapability, UnitGrant, UnitGrantParent, UnitGrantee, UnitLeaseBinding,
    UnitScope,
};
use mush_org::{
    CapabilityGrant, CapabilityId, GrantId, OrgAction, OrgId, OrgMutationOp, OrgMutationPlan,
    OrgScopeId, ScopeSelector,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const POOL: &str = "occator.rig_lease";
const CAPS: [UnitCapability; 4] = [
    UnitCapability::OperateCrew,
    UnitCapability::Deploy,
    UnitCapability::CommandAssignedAi,
    UnitCapability::RecoverLoad,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoanRoots {
    pub agent: ObjId,
    pub grants: BTreeMap<UnitCapability, GrantId>,
}

/// Static lessor policy, authored like the port's existing vendor-clerk roots.
/// Recorded references are not repaired if staff later revoke/remove a grant.
pub(crate) fn bootstrap(
    ctx: &mut Ctx<'_>,
    map: &mut PortMap,
    def: &VehiclesDef,
) -> Result<(), String> {
    if def.id.as_str() != POOL {
        return Ok(());
    }
    let lessor = map
        .orgs
        .get(&def.org)
        .cloned()
        .ok_or("Rig lessor unavailable.")?;
    let agent = map
        .vendors
        .get(&def.org)
        .map(|v| ObjId(v.signatory))
        .ok_or("Rig lessor signatory unavailable.")?;
    if !ctx
        .w
        .get(agent)
        .is_some_and(|o| o.kind == ObjKind::Npc && !o.flags.contains(mush_core::Flags::GOING))
    {
        return Err("Rig lessor signatory unavailable.".into());
    }
    let mut roots = map
        .vehicles
        .as_ref()
        .and_then(|v| v.loan_roots.as_deref())
        .cloned()
        .unwrap_or(LoanRoots {
            agent,
            grants: BTreeMap::new(),
        });
    if roots.agent != agent
        || roots.grants.len() > CAPS.len()
        || roots.grants.keys().any(|c| !CAPS.contains(c))
    {
        return Err("Rig lessor loan references changed.".into());
    }
    for capability in CAPS {
        if roots.grants.contains_key(&capability) {
            continue;
        }
        let grant = super::apply::alloc_grant_id(ctx)?;
        let plan = OrgMutationPlan::of(vec![OrgMutationOp::PutGrant {
            grant: CapabilityGrant {
                id: grant,
                org: OrgId(lessor.org),
                grantee: agent,
                capability: CapabilityId::new(capability.as_str())
                    .ok_or("Invalid rig capability.")?,
                scope: ScopeSelector::Exact(OrgScopeId(lessor.root_scope)),
                limit: None,
                delegable: true,
                valid_from: None,
                valid_until: None,
                revision: 1,
            },
        }]);
        mush_org::submit(
            ctx,
            &plan,
            OrgAction {
                action: "equipment.lessor_policy".into(),
                org: Some(OrgId(lessor.org)),
                detail: serde_json::json!({"pool":POOL,"agent":agent,"capability":capability}),
            },
        )
        .map_err(|e| e.to_string())?;
        roots.grants.insert(capability, grant);
        map.vehicles
            .as_mut()
            .ok_or("Rig pool unavailable.")?
            .loan_roots = Some(Box::new(roots.clone()));
        crate::store::put_port_map(ctx, map);
    }
    Ok(())
}

pub(crate) fn boot(ctx: &mut Ctx<'_>) {
    let Some((mut map, def)) = super::coop::programme_port(ctx.w).and_then(|p| {
        p.def
            .vehicles
            .as_deref()
            .map(|v| (p.map.clone(), v.clone()))
    }) else {
        return;
    };
    if let Err(error) = bootstrap(ctx, &mut map, &def) {
        tracing::warn!(%error,"rig lessor loan policy unavailable");
    }
}

fn reader(
    ctx: &Ctx<'_>,
) -> Result<
    mush_core::read_service::OwnerReadHandle<mush_org::unit_authority::reader::UnitAuthorityReader>,
    String,
> {
    ctx.w
        .module_state::<crate::detachment::planning::PlanningHandles>()
        .and_then(|h| h.unit_authority_reader.clone())
        .ok_or_else(|| "Unit authority unavailable.".into())
}

fn loans(ctx: &Ctx<'_>, lease: &UnitLeaseBinding) -> Result<Vec<UnitGrant>, String> {
    let reader = reader(ctx)?;
    let mut out = Vec::new();
    let mut after = 0;
    // The registry itself holds at most 1,024 rows; each attributed page is 32.
    for _ in 0..32 {
        let UnitAuthorityResponse::Grants { grants, more } = ctx
            .read_owner(&reader, UnitAuthorityRequest::List { after })
            .map_err(|e| e.to_string())?
        else {
            return Err("Unit loans unavailable.".into());
        };
        let next = grants.last().map(|g| g.id.0);
        out.extend(grants.into_iter().filter(|g|matches!(&g.parent,UnitGrantParent::LeasedOrganizationGrant{lease:bound,..} if bound==lease)));
        if !more {
            return Ok(out);
        }
        let Some(next) = next.filter(|n| *n > after) else {
            return Err("Unit loan history unavailable.".into());
        };
        after = next;
    }
    Err("Unit loan history is too large.".into())
}

fn selected(
    ctx: &Ctx<'_>,
    named: Option<u8>,
) -> Result<(super::tv::TvLease, LoanRoots, OrgId, ObjId), String> {
    let selected = {
        let port = super::coop::programme_port(ctx.w).ok_or("Rig lessor unavailable.")?;
        let def = port
            .def
            .vehicles
            .as_deref()
            .filter(|d| d.id.as_str() == POOL)
            .ok_or("Rig lessor unavailable.")?;
        let store = ctx
            .w
            .module_state::<crate::TacsimStore>()
            .ok_or("Rig leases unavailable.")?;
        let leases = store
            .tv_leases_of(ctx.actor)
            .filter(|r| r.live() && named.is_none_or(|n| n == r.slot))
            .cloned()
            .collect::<Vec<_>>();
        let [rental] = leases.as_slice() else {
            return Err("Name one of your current rental rig slots.".into());
        };
        let roots = port
            .map
            .vehicles
            .as_ref()
            .and_then(|v| v.loan_roots.as_deref())
            .cloned()
            .ok_or("The lessor has not appointed an equipment lender.")?;
        let org = port
            .map
            .orgs
            .get(&def.org)
            .ok_or("Rig lessor unavailable.")?;
        let counter = super::vendor::role_room(ctx.w, port.def, &def.counter)
            .ok_or("Rig counter unavailable.")?;
        (rental.clone(), roots, OrgId(org.org), counter)
    };
    Ok(selected)
}

fn binding(
    ctx: &Ctx<'_>,
    rental: &super::tv::TvLease,
    organization: OrgId,
) -> Result<UnitLeaseBinding, String> {
    rental.check()?;
    let lease = {
        let economy = ctx
            .w
            .module_state::<mush_economy::EconomyStore>()
            .ok_or("Lease contract unavailable.")?;
        let contract = economy
            .snapshot()
            .contract(mush_economy::ContractId(rental.lease))
            .ok_or("Lease contract unavailable.")?;
        UnitLeaseBinding {
            contract: rental.lease,
            unit: rental.tv,
            tenant: ctx.actor,
            consent_hash: contract.consent_hash,
        }
    };
    use mush_org::unit_authority::UnitAuthorityFacts;
    if !mush_economy::unit_authority::EconomyUnitAuthorityFacts::new(ctx.w)
        .equipment_lease(organization, &lease)
    {
        return Err("The actual equipment lease is not active.".into());
    }
    Ok(lease)
}

pub(crate) fn command(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let usage = "Use +tv[/<n>]/permissions [go] at Occator's counter.";
    let go = match ctx.args.raw().trim() {
        "" => false,
        "go" => true,
        _ => return Err(usage.into()),
    };
    let mut named = None;
    for switch in ctx.switches.iter() {
        if switch == "permissions" {
            continue;
        }
        let n = switch
            .parse::<u8>()
            .ok()
            .filter(|n| *n >= 1 && u32::from(*n) <= crate::definitions::logistics::MAX_BAY_ITEMS)
            .ok_or(usage)?;
        if named.replace(n).is_some() {
            return Err(usage.into());
        }
    }
    let (rental, roots, organization, counter) = selected(ctx, named)?;
    let lease = binding(ctx, &rental, organization)?;
    let current = loans(ctx, &lease)?;
    if !current.is_empty() {
        for grant in &current {
            let UnitAuthorityResponse::Authority { allowed } = ctx
                .read_owner(
                    &reader(ctx)?,
                    UnitAuthorityRequest::Authorize {
                        unit: rental.tv,
                        capability: grant.capability,
                        delegate: false,
                        mission: None,
                    },
                )
                .map_err(|e| e.to_string())?
            else {
                return Err("Unit authority unavailable.".into());
            };
            super::coop::say(
                ctx,
                format!(
                    "Rig {} loan {}: {} {}; {}.",
                    rental.slot,
                    grant.id.0,
                    grant.capability.as_str(),
                    if allowed { "allowed now" } else { "inactive" },
                    if grant.delegable {
                        "delegable within this lease"
                    } else {
                        "no onward delegation"
                    }
                ),
            );
        }
        if go
            && (current.len() != CAPS.len()
                || current
                    .iter()
                    .map(|g| g.capability)
                    .collect::<BTreeSet<_>>()
                    != BTreeSet::from(CAPS))
        {
            return Err("The existing equipment loan needs lender review.".into());
        }
        if go {
            super::coop::say(
                ctx,
                "The recorded equipment agreement is unchanged; revoked loans are not renewed here.",
            );
        }
        return Ok(());
    }
    super::coop::say(
        ctx,
        format!(
            "Occator offers rig {} operational loans while this exact lease is active: operate/crew and command with onward delegation; deploy and recover/load without it. No title or refit right and no additional charge. +tv/{}/permissions go requests the agreement.",
            rental.slot, rental.slot
        ),
    );
    if !go {
        return Ok(());
    }
    issue(ctx, &rental, roots, organization, counter, lease)
}

fn issue(
    ctx: &mut Ctx<'_>,
    rental: &super::tv::TvLease,
    roots: LoanRoots,
    organization: OrgId,
    counter: ObjId,
    lease: UnitLeaseBinding,
) -> Result<(), String> {
    if rental.status != super::tv::TvLeaseStatus::Active
        || ctx.w.get(ctx.actor).and_then(|o| o.location) != Some(counter)
        || !ctx.w.get(roots.agent).is_some_and(|o| {
            o.kind == ObjKind::Npc
                && !o.flags.contains(mush_core::Flags::GOING)
                && o.location == Some(counter)
        })
    {
        return Err(
            "The current lessee and Occator's lender must be at the actual counter.".into(),
        );
    }
    let drafts = CAPS
        .into_iter()
        .map(|capability| {
            Ok(UnitGrantDraft {
                grantee: UnitGrantee::Character(lease.tenant),
                capability,
                scope: UnitScope::Units {
                    units: BTreeSet::from([lease.unit]),
                },
                valid_until: None,
                delegable: matches!(
                    capability,
                    UnitCapability::OperateCrew | UnitCapability::CommandAssignedAi
                ),
                recovery: RecoveryPolicy::None,
                parent: Some(UnitGrantParent::LeasedOrganizationGrant {
                    organization,
                    grant: *roots
                        .grants
                        .get(&capability)
                        .ok_or("The lessor's loan authority is incomplete.")?,
                    lease: lease.clone(),
                }),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let reader = reader(ctx)?;
    let mut fire = crate::port_tick::Fire::new(BTreeMap::new());
    fire.as_principal(ctx, roots.agent, roots.agent, |ctx| {
        mush_org::unit_authority::runtime::submit_from(
            ctx,
            &reader,
            UnitGrantMutation::IssueMany(drafts),
        )
    })
    .map_err(|_| "Occator's lender has an action pending; retry shortly.".to_owned())??;
    super::coop::say(
        ctx,
        "Occator is recording the four operational loans. Use +tv/permissions to inspect the committed agreement.",
    );
    Ok(())
}
