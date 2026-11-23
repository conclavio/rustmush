//! Occator explicitly lends operational rights through the real Org registry.
//! Economic lease consent is a condition on that loan, never permission itself.
use super::{PortMap, def::VehiclesDef};
use mush_core::{Ctx, ObjId, ObjKind};
use mush_org::unit_authority::reader::{
    UnitAuthorityRequest, UnitAuthorityResponse, UnitGrantDraft, UnitGrantMutation,
};
use mush_org::unit_authority::{
    RecoveryPolicy, UnitCapability, UnitGrant, UnitGrantParent, UnitGrantStatus, UnitGrantee,
    UnitLeaseBinding, UnitScope,
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

/// Minimum real-time duration for a rig operational loan (`P2-TSW-18`).
const MIN_LOAN_REAL_SECS: i64 = 60;
/// Maximum real-time duration for a single rig operational loan window (`P2-TSW-18`: 30 real days).
const MAX_LOAN_REAL_SECS: i64 = 30 * 86_400;

fn all_grants(ctx: &Ctx<'_>) -> Result<Vec<UnitGrant>, String> {
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
        out.extend(grants);
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

/// Maximum leases tracked in `TacsimStore::rig_loan_grants` (`P2-TSW-18`).
const MAX_TRACKED_LOAN_LEASES: usize = 512;
/// Maximum dead-lease grant revocations queued in `TacsimStore::pending_loan_revokes` (`P2-TSW-18`).
const MAX_PENDING_LOAN_REVOKES: usize = 1_024;

/// Record any active Occator rig operational loans visible to the current command caller into
/// `TacsimStore::rig_loan_grants`, queuing dead-lease grants into `pending_loan_revokes`
/// (`P2-TSW-18`).
pub(crate) fn observe_actor_loans(ctx: &mut Ctx<'_>) {
    let Some((_, organization)) = lessor_lender(ctx.w) else {
        return;
    };
    let Ok(grants) = all_grants(ctx) else {
        return;
    };
    let now = ctx.w.now();
    let mut alive: Vec<(u64, u64)> = Vec::new();
    let mut dead: Vec<u64> = Vec::new();
    for g in grants {
        if g.status != UnitGrantStatus::Active || g.valid_until.is_some_and(|end| now >= end) {
            continue;
        }
        if let UnitGrantParent::LeasedOrganizationGrant {
            organization: org,
            lease: bound,
            ..
        } = &g.parent
            && *org == organization
        {
            if lease_is_dead(ctx.w, organization, bound) {
                dead.push(g.id.0);
            } else {
                alive.push((bound.contract, g.id.0));
            }
        }
    }
    let Some(store) = ctx.w.module_state_mut::<crate::TacsimStore>() else {
        return;
    };
    for (lease_id, grant_id) in alive {
        if !store.rig_loan_grants.contains_key(&lease_id)
            && store.rig_loan_grants.len() >= MAX_TRACKED_LOAN_LEASES
        {
            continue;
        }
        let ids = store.rig_loan_grants.entry(lease_id).or_default();
        if !ids.contains(&grant_id) && ids.len() < 8 {
            ids.push(grant_id);
        }
    }
    for grant_id in dead {
        queue_revoke(store, grant_id);
    }
}

fn queue_revoke(store: &mut crate::TacsimStore, grant_id: u64) {
    if store.pending_loan_revokes.len() < MAX_PENDING_LOAN_REVOKES
        && !store.pending_loan_revokes.contains(&grant_id)
    {
        store.pending_loan_revokes.push_back(grant_id);
    }
}

fn drain_one_pending_revoke(ctx: &mut Ctx<'_>) {
    let Some((roots, _)) = lessor_lender(ctx.w) else {
        return;
    };
    if ctx.authoritative_pending(roots.agent)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return;
    }
    let Some(next) = ctx
        .w
        .module_state::<crate::TacsimStore>()
        .and_then(|store| store.pending_loan_revokes.front().copied())
    else {
        return;
    };
    match submit_revoke(
        ctx,
        roots.agent,
        mush_org::unit_authority::UnitGrantId(next),
    ) {
        Ok(()) => {
            if let Some(store) = ctx.w.module_state_mut::<crate::TacsimStore>() {
                let _ = store.pending_loan_revokes.pop_front();
            }
        }
        Err(error) => {
            if !error.contains("pending")
                && !error.contains("retry shortly")
                && let Some(store) = ctx.w.module_state_mut::<crate::TacsimStore>()
            {
                let _ = store.pending_loan_revokes.pop_front();
            }
            tracing::debug!(grant = next, %error, "tacsim: rig loan revoke deferred");
        }
    }
}

fn loans(ctx: &Ctx<'_>, lease: &UnitLeaseBinding) -> Result<Vec<UnitGrant>, String> {
    Ok(all_grants(ctx)?
        .into_iter()
        .filter(|g| matches!(&g.parent, UnitGrantParent::LeasedOrganizationGrant { lease: bound, .. } if bound == lease))
        .collect())
}

/// Compute a bounded real-UTC expiry (`valid_until`) matching the rig lease's current term end
/// (`P2-TSW-18`), so operational loans are always finite and prunable by `UnitGrantRegistry::pruned`.
fn loan_valid_until(w: &mush_core::World, rental: &super::tv::TvLease) -> i64 {
    let now_real = w.now();
    let now_ic = w.world_ic_now().at;
    let remaining_ic = rental
        .term_ends_ic
        .saturating_sub(now_ic)
        .max(rental.term_secs.max(1));
    let rate = i64::from(w.world_timeline().current().rate.max(1));
    let remaining_real = remaining_ic.saturating_add(rate - 1) / rate;
    let span = remaining_real.clamp(MIN_LOAN_REAL_SECS, MAX_LOAN_REAL_SECS);
    now_real.saturating_add(span)
}

/// Resolve Occator's equipment lender (`LoanRoots`) and lessor `OrgId` from the programme port.
fn lessor_lender(w: &mush_core::World) -> Option<(LoanRoots, OrgId)> {
    let port = super::coop::programme_port(w)?;
    let def = port
        .def
        .vehicles
        .as_deref()
        .filter(|d| d.id.as_str() == POOL)?;
    let roots = port.map.vehicles.as_ref()?.loan_roots.as_deref()?.clone();
    let org = port.map.orgs.get(&def.org)?;
    Some((roots, OrgId(org.org)))
}

/// Whether a `LeasedOrganizationGrant`'s economic/store rig lease is no longer active (`P2-TSW-18`).
fn lease_is_dead(w: &mush_core::World, organization: OrgId, bound: &UnitLeaseBinding) -> bool {
    let store_active = w
        .module_state::<crate::TacsimStore>()
        .and_then(|store| match store.tv_lease(bound.contract) {
            Some(super::tv::TvLeaseState::Present(r)) => Some(r),
            _ => None,
        })
        .is_some_and(|r| {
            r.status == super::tv::TvLeaseStatus::Active
                && r.tv == bound.unit
                && r.pilot == bound.tenant
        });
    if !store_active {
        return true;
    }
    use mush_org::unit_authority::UnitAuthorityFacts;
    !mush_economy::unit_authority::EconomyUnitAuthorityFacts::new(w)
        .equipment_lease(organization, bound)
}

fn submit_revoke(
    ctx: &mut Ctx<'_>,
    agent: ObjId,
    grant: mush_org::unit_authority::UnitGrantId,
) -> Result<(), String> {
    if ctx.authoritative_pending(agent) {
        return Err("Occator's lender has an action pending; retry shortly.".into());
    }
    let reader = reader(ctx)?;
    let mut fire = crate::port_tick::Fire::new(BTreeMap::new());
    fire.as_principal(ctx, agent, agent, |ctx| {
        mush_org::unit_authority::runtime::submit_from(
            ctx,
            &reader,
            UnitGrantMutation::Revoke { grant },
        )
    })
    .map_err(|_| "Occator's lender has an action pending; retry shortly.".to_owned())?
}

/// **Explicitly begin revoking operational loans for a terminated/forfeited rig lease**
/// (`P2-TSW-18`): observes any grants visible to the caller, queues all tracked grants for
/// `lease_id` into `pending_loan_revokes`, and submits revocation for the first immediately as
/// Occator's lender agent; remaining grants are drained one per tick by [`sweep_stale_loans`].
pub(crate) fn revoke_lease_loans(ctx: &mut Ctx<'_>, lease_id: u64, unit: ObjId, pilot: ObjId) {
    let Some((_, organization)) = lessor_lender(ctx.w) else {
        return;
    };
    observe_actor_loans(ctx);
    // Also capture any matching grants directly visible to the current caller (even if the
    // caller's lease record just transitioned to Returned/Forfeited).
    let mut found: Vec<u64> = Vec::new();
    if let Ok(grants) = all_grants(ctx) {
        let now = ctx.w.now();
        for g in grants {
            if g.status == UnitGrantStatus::Active
                && g.valid_until.is_none_or(|end| now < end)
                && matches!(
                    &g.parent,
                    UnitGrantParent::LeasedOrganizationGrant {
                        organization: org,
                        lease: bound,
                        ..
                    } if *org == organization
                        && bound.contract == lease_id
                        && bound.unit == unit
                        && bound.tenant == pilot
                )
            {
                found.push(g.id.0);
            }
        }
    }
    if let Some(store) = ctx.w.module_state_mut::<crate::TacsimStore>() {
        if let Some(tracked) = store.rig_loan_grants.remove(&lease_id) {
            for id in tracked {
                queue_revoke(store, id);
            }
        }
        for id in found {
            queue_revoke(store, id);
        }
    }
    drain_one_pending_revoke(ctx);
}

/// **Background sweep of stale rig operational loans** (`P2-TSW-18`), called on the surface tick:
/// moves tracked grants of any now-dead rig lease into `pending_loan_revokes` and submits one
/// revocation per tick as Occator's lender agent so terminated leases never exhaust
/// `MAX_UNIT_GRANTS = 1024`.
pub(crate) fn sweep_stale_loans(ctx: &mut Ctx<'_>) {
    let dead_leases: Vec<u64> = ctx
        .w
        .module_state::<crate::TacsimStore>()
        .map(|store| {
            store
                .rig_loan_grants
                .keys()
                .copied()
                .filter(|lease_id| {
                    !matches!(
                        store.tv_lease(*lease_id),
                        Some(super::tv::TvLeaseState::Present(r))
                            if r.status == super::tv::TvLeaseStatus::Active
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if !dead_leases.is_empty()
        && let Some(store) = ctx.w.module_state_mut::<crate::TacsimStore>()
    {
        for lease_id in dead_leases {
            if let Some(ids) = store.rig_loan_grants.remove(&lease_id) {
                for id in ids {
                    queue_revoke(store, id);
                }
            }
        }
    }
    drain_one_pending_revoke(ctx);
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
    observe_actor_loans(ctx);
    let lease = binding(ctx, &rental, organization)?;
    let now = ctx.w.now();
    // P2-TSW-18: expired loans from an earlier term of a renewed lease are pruned on re-issue,
    // while explicitly revoked loans on this lease remain visible and block self-renewal.
    let current: Vec<UnitGrant> = loans(ctx, &lease)?
        .into_iter()
        .filter(|g| {
            g.status == UnitGrantStatus::Revoked || g.valid_until.is_none_or(|end| now < end)
        })
        .collect();
    if !current.is_empty() {
        if !rental.loans_issued {
            let stamped = super::tv::TvLease {
                loans_issued: true,
                ..rental.clone()
            };
            crate::store::put_tv_lease(ctx, &stamped);
        }
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
    // P2-TSW-29: once operational loans have been issued for this lease, refuse re-issuance
    // independent of global `UnitGrantRegistry::pruned` removing revoked or expired grant rows.
    if rental.loans_issued {
        if go {
            return Err("The existing equipment loan needs lender review.".into());
        }
        super::coop::say(
            ctx,
            "The recorded equipment agreement is unchanged; revoked loans are not renewed here.",
        );
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
    if rental.loans_issued {
        return Err("The existing equipment loan needs lender review.".into());
    }
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
    // P2-TSW-18: prevent rapid lease/return cycles faster than the tick sweep from accumulating
    // unrevoked dead-lease loans. Observe any grants visible to this tenant, queue dead ones,
    // and revoke one immediately if any dead-lease revocation is queued.
    observe_actor_loans(ctx);
    let has_pending = ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_some_and(|store| !store.pending_loan_revokes.is_empty());
    if has_pending {
        drain_one_pending_revoke(ctx);
        return Err("Occator's lender is closing an earlier rig loan; retry shortly.".into());
    }
    let valid_until = Some(loan_valid_until(ctx.w, rental));
    let drafts = CAPS
        .into_iter()
        .map(|capability| {
            Ok(UnitGrantDraft {
                grantee: UnitGrantee::Character(lease.tenant),
                capability,
                scope: UnitScope::Units {
                    units: BTreeSet::from([lease.unit]),
                },
                valid_until,
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
    // P2-TSW-29: persist loan issuance on the lease record so revoked loans cannot be silently
    // re-issued if `UnitGrantRegistry::pruned` removes the revoked rows.
    let stamped = super::tv::TvLease {
        loans_issued: true,
        ..rental.clone()
    };
    crate::store::put_tv_lease(ctx, &stamped);
    super::coop::say(
        ctx,
        "Occator is recording the four operational loans. Use +tv/permissions to inspect the committed agreement.",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **P2-TSW-29**: once `TvLease::loans_issued` is persisted `true`, `issue()` unconditionally
    /// refuses re-issuance independent of whether `UnitGrantRegistry::pruned` removed all earlier
    /// revoked/expired grant records.
    #[test]
    fn p2_tsw_29_issue_refuses_once_loans_issued_is_set() {
        let mut tw = mush_testkit::TestWorld::new();
        let pilot = tw.spawn_player("RevokedLessee");
        let counter = tw
            .world_mut()
            .create(ObjKind::Room, "Counter", ObjId(1), None)
            .expect("counter");
        let agent = tw
            .world_mut()
            .create(ObjKind::Npc, "Lender", ObjId(1), None)
            .expect("agent");
        let rental = super::super::tv::TvLease {
            schema: super::super::tv::TV_LEASE_SCHEMA,
            lease: 41,
            tv: ObjId(900),
            mark: "OWP-ES-01".to_owned(),
            pilot,
            hull: ObjId(500),
            slot: 1,
            hold: 7,
            fee_minor: 250_000,
            deposit_minor: 1_500_000,
            term_secs: 168 * 86_400,
            leased_at_ic: 1_000,
            term_ends_ic: 1_000 + 168 * 86_400,
            terms_paid: 1,
            status: super::super::tv::TvLeaseStatus::Active,
            ended_at_ic: None,
            repossessed: false,
            written_off: false,
            wreck_aboard_ic: None,
            loans_issued: true,
        };
        let roots = LoanRoots {
            agent,
            grants: BTreeMap::new(),
        };
        let lease = UnitLeaseBinding {
            contract: rental.lease,
            unit: rental.tv,
            tenant: pilot,
            consent_hash: [0; 32],
        };
        let engine = tw.engine_mut();
        let mut out = mush_core::Outbox::default();
        let mut ctx = Ctx {
            w: &mut engine.world,
            reg: &engine.registry,
            out: &mut out,
            actor: pilot,
            args: Default::default(),
            switches: Default::default(),
            module: Some(crate::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let err = issue(&mut ctx, &rental, roots, OrgId(1), counter, lease)
            .expect_err("must refuse re-issuance after loans_issued is set");
        assert_eq!(err, "The existing equipment loan needs lender review.");
    }
}
