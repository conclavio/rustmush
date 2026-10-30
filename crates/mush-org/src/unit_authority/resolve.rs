//! Live, bounded proofs for new unit admissions. No answer here actuates a
//! station, ejects an occupant, or cancels an already executed instruction.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;

use super::record::*;
use crate::authorize::{explicitly_denied, scope_contains, selector_subsumes, time_valid};
use crate::{
    AuthRequest, GrantLimit, OrgId, OrgScopeId, OrgState, OrgStatus, PredicateEval, PrincipalId,
    ScopeSelector, ScopeStatus,
};

#[derive(Clone)]
struct RootProof {
    organization: OrgId,
    scope: ScopeSelector,
    from: Option<i64>,
    until: Option<i64>,
    source: UnitAuthoritySource,
}

#[derive(Clone, Copy)]
struct ControlProof {
    scope: OrgScopeId,
}

struct ControlRequest<'a> {
    organization: OrgId,
    unit: ObjId,
    capability: UnitCapability,
    delegate: bool,
    selector: &'a ScopeSelector,
    depth: usize,
    window: Option<(Option<i64>, Option<i64>)>,
    recovery: RecoveryPolicy,
}

struct Resolver<'a> {
    orgs: &'a OrgState,
    grants: &'a BTreeMap<UnitGrantId, UnitGrant>,
    facts: &'a dyn UnitAuthorityFacts,
    predicates: &'a dyn PredicateEval,
    now: i64,
    actors: Vec<ObjId>,
    path: BTreeSet<UnitGrantId>,
    visits: usize,
    sources: Vec<UnitAuthoritySource>,
    checking_exclusions: bool,
    // Declaration/root checks prove lending authority only. Exercising a role
    // always uses the ordinary resolver with actual mission-owner facts.
    mission_lineage_only: bool,
}

impl<'a> Resolver<'a> {
    fn new(
        orgs: &'a OrgState,
        grants: &'a BTreeMap<UnitGrantId, UnitGrant>,
        facts: &'a dyn UnitAuthorityFacts,
        predicates: &'a dyn PredicateEval,
        now: i64,
        actor: ObjId,
    ) -> Result<Self, UnitGrantError> {
        if grants.len() > MAX_UNIT_GRANTS {
            return Err(UnitGrantError::Capacity);
        }
        if !facts.principal_exists(actor) {
            return Err(UnitGrantError::NoAuthority);
        }
        Ok(Self {
            orgs,
            grants,
            facts,
            predicates,
            now,
            actors: vec![actor],
            path: BTreeSet::new(),
            visits: 0,
            sources: Vec::new(),
            checking_exclusions: false,
            mission_lineage_only: false,
        })
    }

    fn charge(&mut self) -> Result<(), UnitGrantError> {
        if self.visits >= MAX_UNIT_PROOF_VISITS {
            return Err(UnitGrantError::Capacity);
        }
        self.visits += 1;
        Ok(())
    }

    fn witness(self) -> UnitGrantWitness {
        UnitGrantWitness {
            at: self.now,
            sources: self.sources,
        }
    }

    fn source(&mut self, source: UnitAuthoritySource) {
        if !self.sources.contains(&source) {
            self.sources.push(source);
        }
    }

    fn scope_active(&self, organization: OrgId, scope: OrgScopeId) -> bool {
        self.orgs
            .org(organization)
            .is_some_and(|org| org.status == OrgStatus::Active)
            && self
                .orgs
                .scope(scope)
                .is_some_and(|row| row.org == organization && row.status == ScopeStatus::Active)
    }

    fn policy(
        &mut self,
        scope: OrgScopeId,
        capability: UnitCapability,
    ) -> Result<(), UnitGrantError> {
        let capability = capability
            .capability_id()
            .ok_or(UnitGrantError::NoAuthority)?;
        let orgs = self.orgs;
        let predicates = self.predicates;
        for actor in self.actors.clone() {
            if explicitly_denied(
                orgs,
                &AuthRequest {
                    actor,
                    capability: capability.clone(),
                    scope,
                    now: self.now,
                    amount: None,
                },
                predicates,
                || self.charge(),
            )? {
                return Err(UnitGrantError::Excluded);
            }
        }
        Ok(())
    }

    fn asset(&self, unit: ObjId) -> Result<UnitAssetFacts, UnitGrantError> {
        self.facts
            .asset(unit)
            .filter(|asset| asset.unit == unit && asset.active)
            .ok_or(UnitGrantError::NoAuthority)
    }

    fn root(
        &self,
        parent: &UnitGrantParent,
        actor: ObjId,
        capability: UnitCapability,
        delegate: bool,
    ) -> Result<RootProof, UnitGrantError> {
        let cap = capability
            .capability_id()
            .ok_or(UnitGrantError::NoAuthority)?;
        let proof = match parent {
            UnitGrantParent::OrganizationGrant {
                organization,
                grant,
            } => {
                let source = self.orgs.grant(*grant).ok_or(UnitGrantError::Stale)?;
                if source.org != *organization
                    || source.grantee != actor
                    || source.capability != cap
                    || (delegate && !source.delegable)
                    || !no_amount_limit(source.limit)
                    || !time_valid(source.valid_from, source.valid_until, self.now)
                {
                    return Err(UnitGrantError::NoAuthority);
                }
                RootProof {
                    organization: *organization,
                    scope: source.scope.clone(),
                    from: source.valid_from,
                    until: source.valid_until,
                    source: UnitAuthoritySource::OrganizationGrant {
                        grant: *grant,
                        revision: source.revision,
                    },
                }
            }
            UnitGrantParent::OrganizationRoleTemplate {
                organization,
                role,
                role_revision,
                template_index,
            } => {
                if !self.orgs.active_roles(*organization, actor).contains(role) {
                    return Err(UnitGrantError::NoAuthority);
                }
                let source = self.orgs.role(*role).ok_or(UnitGrantError::Stale)?;
                if source.org != *organization || source.revision != *role_revision {
                    return Err(UnitGrantError::Stale);
                }
                let template = source
                    .grants
                    .get(usize::from(*template_index))
                    .ok_or(UnitGrantError::Stale)?;
                if template.capability != cap
                    || (delegate && !template.delegable)
                    || !no_amount_limit(template.limit)
                {
                    return Err(UnitGrantError::NoAuthority);
                }
                RootProof {
                    organization: *organization,
                    scope: template.scope.clone(),
                    from: None,
                    until: None,
                    source: UnitAuthoritySource::OrganizationRole {
                        role: *role,
                        revision: *role_revision,
                        template_index: *template_index,
                    },
                }
            }
            _ => return Err(UnitGrantError::NoAuthority),
        };
        if !self.scope_active(proof.organization, proof.scope.root()) {
            return Err(UnitGrantError::Inactive);
        }
        Ok(proof)
    }

    fn grantee(&self, grant: &UnitGrant) -> Result<(), UnitGrantError> {
        match &grant.grantee {
            UnitGrantee::Character(actor) if self.facts.principal_exists(*actor) => Ok(()),
            UnitGrantee::Organization {
                organization,
                scope,
            } if self.scope_active(*organization, *scope) => Ok(()),
            UnitGrantee::CurrentLeader(binding) => {
                if self.mission_lineage_only {
                    return Ok(());
                }
                let mission = self.facts.mission(binding).ok_or(UnitGrantError::Stale)?;
                let UnitScope::Units { units } = &grant.scope else {
                    return Err(UnitGrantError::Widening);
                };
                if !mission.active
                    || mission.binding != *binding
                    || mission.units.len() > MAX_UNIT_SCOPE_ITEMS
                    || !units.is_subset(&mission.units)
                {
                    return Err(UnitGrantError::Stale);
                }
                Ok(())
            }
            _ => Err(UnitGrantError::NoAuthority),
        }
    }

    /// Structural narrowing is checked again on every use. Concrete membership
    /// in a wildcard is separately checked for the actual requested unit.
    fn shape(&mut self, grant: &UnitGrant, depth: usize) -> Result<(), UnitGrantError> {
        self.enter(grant, depth)?;
        self.actors.push(grant.grantor);
        let result = self.shape_inner(grant, depth);
        self.actors.pop();
        self.path.remove(&grant.id);
        result
    }

    fn enter(&mut self, grant: &UnitGrant, depth: usize) -> Result<(), UnitGrantError> {
        self.charge()?;
        if depth >= MAX_UNIT_GRANT_DEPTH {
            return Err(UnitGrantError::Capacity);
        }
        grant.validate()?;
        if grant.status != UnitGrantStatus::Active
            || !time_valid(grant.valid_from, grant.valid_until, self.now)
        {
            return Err(UnitGrantError::Inactive);
        }
        if !self.facts.principal_exists(grant.grantor) {
            return Err(UnitGrantError::NoAuthority);
        }
        self.grantee(grant)?;
        if !self.path.insert(grant.id) {
            return Err(UnitGrantError::Cycle);
        }
        Ok(())
    }

    fn shape_inner(&mut self, grant: &UnitGrant, depth: usize) -> Result<(), UnitGrantError> {
        match &grant.parent {
            UnitGrantParent::PersonalTitle {
                unit,
                owner,
                lineage,
            } => {
                let asset = self.asset(*unit)?;
                if grant.grantor != *owner
                    || asset.owner != PrincipalId::Object(*owner)
                    || asset.lineage != *lineage
                {
                    return Err(UnitGrantError::Stale);
                }
                if !matches!(&grant.scope, UnitScope::Units { units } if units.len() == 1 && units.contains(unit))
                {
                    return Err(UnitGrantError::Widening);
                }
                self.source(UnitAuthoritySource::Title {
                    unit: *unit,
                    lineage: *lineage,
                });
            }
            UnitGrantParent::UnitGrant { id } => {
                let parent = self
                    .grants
                    .get(id)
                    .filter(|parent| parent.id == *id)
                    .cloned()
                    .ok_or(UnitGrantError::Stale)?;
                if parent.grantee != UnitGrantee::Character(grant.grantor)
                    || !parent.delegable
                    || parent.capability != grant.capability
                    || !scope_narrows(self.orgs, &parent.scope, &grant.scope)
                    || !window_narrows(
                        parent.valid_from,
                        parent.valid_until,
                        grant.valid_from,
                        grant.valid_until,
                    )
                    || (grant.recovery == RecoveryPolicy::ReturnOnly
                        && parent.recovery != RecoveryPolicy::ReturnOnly)
                {
                    return Err(UnitGrantError::Widening);
                }
                self.shape(&parent, depth + 1)?;
            }
            source => {
                if grant.recovery == RecoveryPolicy::ReturnOnly
                    && grant.capability != UnitCapability::RecoverLoad
                {
                    return Err(UnitGrantError::Widening);
                }
                let root = self.root(source, grant.grantor, grant.capability, true)?;
                if !root_scope_narrows(self.orgs, &root, &grant.scope)
                    || !window_narrows(root.from, root.until, grant.valid_from, grant.valid_until)
                {
                    return Err(UnitGrantError::Widening);
                }
                self.policy(root.scope.root(), grant.capability)?;
                self.source(root.source);
            }
        }
        self.source(UnitAuthoritySource::Grant {
            id: grant.id,
            revision: grant.revision,
        });
        Ok(())
    }

    fn walk(
        &mut self,
        grant: &UnitGrant,
        unit: ObjId,
        delegate: bool,
        expected: &UnitGrantee,
        depth: usize,
    ) -> Result<(), UnitGrantError> {
        if grant.grantee != *expected || (delegate && !grant.delegable) {
            return Err(UnitGrantError::NoAuthority);
        }
        self.enter(grant, depth)?;
        self.actors.push(grant.grantor);
        let result = self.walk_inner(grant, unit, depth);
        self.actors.pop();
        self.path.remove(&grant.id);
        result
    }

    fn walk_inner(
        &mut self,
        grant: &UnitGrant,
        unit: ObjId,
        depth: usize,
    ) -> Result<(), UnitGrantError> {
        self.matches_scope(
            &grant.scope,
            &grant.grantee,
            unit,
            grant.capability,
            depth + 1,
        )?;
        match &grant.parent {
            UnitGrantParent::PersonalTitle {
                unit: root_unit,
                owner,
                lineage,
            } => {
                let asset = self.asset(unit)?;
                if *root_unit != unit
                    || grant.grantor != *owner
                    || asset.owner != PrincipalId::Object(*owner)
                    || asset.lineage != *lineage
                {
                    return Err(UnitGrantError::Stale);
                }
                if !matches!(&grant.scope, UnitScope::Units { units } if units.len() == 1 && units.contains(&unit))
                {
                    return Err(UnitGrantError::Widening);
                }
                self.source(UnitAuthoritySource::Title {
                    unit,
                    lineage: *lineage,
                });
            }
            UnitGrantParent::UnitGrant { id } => {
                let parent = self
                    .grants
                    .get(id)
                    .filter(|parent| parent.id == *id)
                    .cloned()
                    .ok_or(UnitGrantError::Stale)?;
                if parent.capability != grant.capability
                    || !scope_narrows(self.orgs, &parent.scope, &grant.scope)
                    || !window_narrows(
                        parent.valid_from,
                        parent.valid_until,
                        grant.valid_from,
                        grant.valid_until,
                    )
                    || (grant.recovery == RecoveryPolicy::ReturnOnly
                        && parent.recovery != RecoveryPolicy::ReturnOnly)
                {
                    return Err(UnitGrantError::Widening);
                }
                self.walk(
                    &parent,
                    unit,
                    true,
                    &UnitGrantee::Character(grant.grantor),
                    depth + 1,
                )?;
            }
            source => {
                if grant.recovery == RecoveryPolicy::ReturnOnly
                    && grant.capability != UnitCapability::RecoverLoad
                {
                    return Err(UnitGrantError::Widening);
                }
                let root = self.root(source, grant.grantor, grant.capability, true)?;
                if !root_scope_narrows(self.orgs, &root, &grant.scope)
                    || !window_narrows(root.from, root.until, grant.valid_from, grant.valid_until)
                {
                    return Err(UnitGrantError::Widening);
                }
                let control = self.control(ControlRequest {
                    organization: root.organization,
                    unit,
                    capability: grant.capability,
                    delegate: true,
                    selector: &root.scope,
                    depth: depth + 1,
                    window: Some((grant.valid_from, grant.valid_until)),
                    recovery: grant.recovery,
                })?;
                self.policy(control.scope, grant.capability)?;
                self.source(root.source);
            }
        }
        self.source(UnitAuthoritySource::Grant {
            id: grant.id,
            revision: grant.revision,
        });
        Ok(())
    }

    fn matches_scope(
        &mut self,
        scope: &UnitScope,
        grantee: &UnitGrantee,
        unit: ObjId,
        capability: UnitCapability,
        depth: usize,
    ) -> Result<(), UnitGrantError> {
        match scope {
            UnitScope::Units { units } => {
                if units.contains(&unit) {
                    Ok(())
                } else {
                    Err(UnitGrantError::NoAuthority)
                }
            }
            UnitScope::OrganizationControlled {
                organization,
                scope,
                excluded,
            } => {
                if excluded.contains(&unit)
                    || (!self.checking_exclusions
                        && self.wildcard_excluded(grantee, *organization, capability, unit)?)
                {
                    return Err(UnitGrantError::Excluded);
                }
                self.control(ControlRequest {
                    organization: *organization,
                    unit,
                    capability,
                    delegate: false,
                    selector: scope,
                    depth,
                    window: None,
                    recovery: RecoveryPolicy::None,
                })?;
                Ok(())
            }
        }
    }

    fn wildcard_excluded(
        &mut self,
        grantee: &UnitGrantee,
        organization: OrgId,
        capability: UnitCapability,
        unit: ObjId,
    ) -> Result<bool, UnitGrantError> {
        let candidates: Vec<_> = self.grants.values().filter(|grant| grant.grantee == *grantee && grant.capability == capability
            && grant.status == UnitGrantStatus::Active && grant.validate().is_ok()
            && time_valid(grant.valid_from, grant.valid_until, self.now)
            && matches!(&grant.scope, UnitScope::OrganizationControlled {
                organization: owner, excluded, .. } if *owner == organization && excluded.contains(&unit)))
            .cloned().collect();
        for grant in candidates {
            let mut proof = Resolver {
                orgs: self.orgs,
                grants: self.grants,
                facts: self.facts,
                predicates: self.predicates,
                now: self.now,
                actors: self.actors.clone(),
                path: BTreeSet::new(),
                visits: self.visits,
                sources: Vec::new(),
                checking_exclusions: true,
                mission_lineage_only: self.mission_lineage_only,
            };
            // An exclusion only vetoes overlapping current control. While
            // proving that overlap, still enforce each backing loan's own
            // exclusions, but do not recursively evaluate peer exclusions.
            let result = proof.shape(&grant, 0).and_then(|()| {
                let UnitScope::OrganizationControlled { scope, .. } = &grant.scope else {
                    return Err(UnitGrantError::NoAuthority);
                };
                proof.actors.push(grant.grantor);
                proof
                    .control(ControlRequest {
                        organization,
                        unit,
                        capability,
                        delegate: false,
                        selector: scope,
                        depth: 0,
                        window: None,
                        recovery: RecoveryPolicy::None,
                    })
                    .map(|_| ())
            });
            self.visits = proof.visits;
            match result {
                Ok(()) => return Ok(true),
                Err(UnitGrantError::Capacity) => return Err(UnitGrantError::Capacity),
                Err(_) => {}
            }
        }
        Ok(false)
    }

    fn control(&mut self, request: ControlRequest<'_>) -> Result<ControlProof, UnitGrantError> {
        let ControlRequest {
            organization,
            unit,
            capability,
            delegate,
            selector,
            depth,
            window,
            recovery,
        } = request;
        self.charge()?;
        if !self.scope_active(organization, selector.root()) {
            return Err(UnitGrantError::Inactive);
        }
        let asset = self.asset(unit)?;
        if asset.owner == PrincipalId::Organization(organization)
            && let Some(scope) = asset.scope
            && self.scope_active(organization, scope)
            && scope_contains(self.orgs, selector, scope)
        {
            self.policy(scope, capability)?;
            self.source(UnitAuthoritySource::Title {
                unit,
                lineage: asset.lineage,
            });
            return Ok(ControlProof { scope });
        }
        if let Some(control) =
            self.facts
                .organization_control(organization, unit, capability, self.now)
            && control.organization == organization
            && control.unit == unit
            && control.capability == capability
            && (!delegate || control.delegable)
            && control.source != 0
            && control.revision != 0
            && (recovery == RecoveryPolicy::None || control.recovery == RecoveryPolicy::ReturnOnly)
            && time_valid(control.valid_from, control.valid_until, self.now)
            && control
                .valid_from
                .zip(control.valid_until)
                .is_none_or(|(start, end)| start < end)
            && self.scope_active(organization, control.scope)
            && scope_contains(self.orgs, selector, control.scope)
            && window.is_none_or(|(from, until)| {
                window_narrows(control.valid_from, control.valid_until, from, until)
            })
        {
            self.policy(control.scope, capability)?;
            self.source(UnitAuthoritySource::OrganizationControl {
                organization,
                source: control.source,
                revision: control.revision,
            });
            return Ok(ControlProof {
                scope: control.scope,
            });
        }
        // Explicit organization loans are real parent proofs, not supplied
        // booleans. They share the same path and work budget as the outer grant.
        let candidates: Vec<_> = self.grants.values().filter(|grant| grant.capability == capability
            && matches!(grant.grantee, UnitGrantee::Organization { organization: org, scope }
                if org == organization && self.scope_active(org, scope) && scope_contains(self.orgs, selector, scope)))
            .cloned().collect();
        for grant in candidates {
            let UnitGrantee::Organization { scope, .. } = grant.grantee else {
                continue;
            };
            if window.is_some_and(|(from, until)| {
                !window_narrows(grant.valid_from, grant.valid_until, from, until)
            }) {
                continue;
            }
            if recovery == RecoveryPolicy::ReturnOnly
                && grant.recovery != RecoveryPolicy::ReturnOnly
            {
                continue;
            }
            let checkpoint = self.sources.len();
            match self.walk(
                &grant,
                unit,
                delegate,
                &UnitGrantee::Organization {
                    organization,
                    scope,
                },
                depth,
            ) {
                Ok(()) => {
                    self.policy(scope, capability)?;
                    return Ok(ControlProof { scope });
                }
                Err(UnitGrantError::Capacity) => return Err(UnitGrantError::Capacity),
                Err(_) => self.sources.truncate(checkpoint),
            }
        }
        Err(UnitGrantError::NoAuthority)
    }

    fn direct_org(&mut self, request: &UnitAuthRequest) -> Result<(), UnitGrantError> {
        let capability = request.action.capability();
        let mut roots = Vec::new();
        let direct: Vec<_> = self
            .orgs
            .grants_to(request.actor)
            .take(MAX_UNIT_PROOF_VISITS + 1)
            .cloned()
            .collect();
        for grant in direct {
            self.charge()?;
            if grant.capability.as_str() == capability.as_str() {
                roots.push(UnitGrantParent::OrganizationGrant {
                    organization: grant.org,
                    grant: grant.id,
                });
            }
        }
        let organizations: Vec<_> = self
            .orgs
            .member_orgs(request.actor)
            .take(MAX_UNIT_PROOF_VISITS + 1)
            .collect();
        for organization in organizations {
            self.charge()?;
            let roles: Vec<_> = self
                .orgs
                .active_roles(organization, request.actor)
                .iter()
                .take(MAX_UNIT_PROOF_VISITS + 1)
                .copied()
                .collect();
            for role_id in roles {
                self.charge()?;
                let Some((revision, templates)) = self.orgs.role(role_id).map(|role| {
                    (
                        role.revision,
                        role.grants
                            .iter()
                            .enumerate()
                            .take(MAX_UNIT_PROOF_VISITS + 1)
                            .map(|(index, template)| {
                                (index, template.capability.as_str() == capability.as_str())
                            })
                            .collect::<Vec<_>>(),
                    )
                }) else {
                    continue;
                };
                for (index, matches) in templates {
                    self.charge()?;
                    if matches && let Ok(template_index) = u16::try_from(index) {
                        roots.push(UnitGrantParent::OrganizationRoleTemplate {
                            organization,
                            role: role_id,
                            role_revision: revision,
                            template_index,
                        });
                    }
                }
            }
        }
        for parent in roots {
            let checkpoint = self.sources.len();
            let Ok(root) = self.root(
                &parent,
                request.actor,
                capability,
                request.action.requires_delegation(),
            ) else {
                continue;
            };
            match self.control(ControlRequest {
                organization: root.organization,
                unit: request.unit,
                capability,
                delegate: request.action.requires_delegation(),
                selector: &root.scope,
                depth: 0,
                window: None,
                recovery: RecoveryPolicy::None,
            }) {
                Ok(control) => {
                    self.policy(control.scope, capability)?;
                    self.source(root.source);
                    return Ok(());
                }
                Err(UnitGrantError::Capacity) => return Err(UnitGrantError::Capacity),
                Err(_) => self.sources.truncate(checkpoint),
            }
        }
        Err(UnitGrantError::NoAuthority)
    }
}

fn no_amount_limit(limit: Option<GrantLimit>) -> bool {
    limit.and_then(|limit| limit.max_amount).is_none()
}

fn window_narrows(
    outer_from: Option<i64>,
    outer_until: Option<i64>,
    inner_from: Option<i64>,
    inner_until: Option<i64>,
) -> bool {
    outer_from.is_none_or(|start| inner_from.is_some_and(|inner| inner >= start))
        && outer_until.is_none_or(|end| inner_until.is_some_and(|inner| inner <= end))
}

fn scope_narrows(orgs: &OrgState, outer: &UnitScope, inner: &UnitScope) -> bool {
    match (outer, inner) {
        (UnitScope::Units { units: outer }, UnitScope::Units { units: inner }) => {
            inner.is_subset(outer)
        }
        (UnitScope::OrganizationControlled { excluded, .. }, UnitScope::Units { units }) => {
            units.is_disjoint(excluded)
        }
        (
            UnitScope::OrganizationControlled {
                organization: a,
                scope: outer,
                excluded: old,
            },
            UnitScope::OrganizationControlled {
                organization: b,
                scope: inner,
                excluded: new,
            },
        ) => a == b && selector_subsumes(orgs, outer, inner) && old.is_subset(new),
        _ => false,
    }
}

fn root_scope_narrows(orgs: &OrgState, root: &RootProof, inner: &UnitScope) -> bool {
    match inner {
        UnitScope::Units { .. } => true, // concrete control scope checked on each unit
        UnitScope::OrganizationControlled {
            organization,
            scope,
            ..
        } => *organization == root.organization && selector_subsumes(orgs, &root.scope, scope),
    }
}

/// Authorize a new admission over current records. Witnesses are audit hints;
/// they must never replace re-resolution at a later final boundary.
#[must_use]
pub fn authorize_unit(
    orgs: &OrgState,
    grants: &BTreeMap<UnitGrantId, UnitGrant>,
    facts: &dyn UnitAuthorityFacts,
    predicates: &dyn PredicateEval,
    request: &UnitAuthRequest,
) -> UnitDecision {
    let mut resolver =
        match Resolver::new(orgs, grants, facts, predicates, request.now, request.actor) {
            Ok(resolver) => resolver,
            Err(error) => return UnitDecision::Deny(error),
        };
    let asset = match resolver.asset(request.unit) {
        Ok(asset) => asset,
        Err(error) => return UnitDecision::Deny(error),
    };
    if asset.owner == PrincipalId::Object(request.actor) {
        resolver.source(UnitAuthoritySource::Title {
            unit: request.unit,
            lineage: asset.lineage,
        });
        return UnitDecision::Allow(resolver.witness());
    }
    let mut near = UnitGrantError::NoAuthority;
    for (id, grant) in grants {
        if grant.id != *id || grant.capability != request.action.capability() {
            continue;
        }
        let matches = match &grant.grantee {
            UnitGrantee::Character(actor) => *actor == request.actor,
            UnitGrantee::CurrentLeader(binding) => {
                request.mission.as_ref().is_some_and(|command| {
                    command.binding == *binding
                        && facts.mission(binding).is_some_and(|mission| {
                            mission.binding == *binding
                                && mission.active
                                && mission.units.len() <= MAX_UNIT_SCOPE_ITEMS
                                && mission.units.contains(&request.unit)
                                && mission.term == command.term
                                && command.term != 0
                                && mission
                                    .leader
                                    .is_some_and(|leader| leader.character() == request.actor)
                        })
                })
            }
            UnitGrantee::Organization { .. } => false,
        };
        if !matches {
            continue;
        }
        let checkpoint = resolver.sources.len();
        match resolver.walk(
            grant,
            request.unit,
            request.action.requires_delegation(),
            &grant.grantee,
            0,
        ) {
            Ok(()) => {
                if let UnitGrantee::CurrentLeader(binding) = &grant.grantee
                    && let Some(command) = &request.mission
                {
                    resolver.source(UnitAuthoritySource::Mission {
                        binding: binding.clone(),
                        term: command.term,
                    });
                }
                return UnitDecision::Allow(resolver.witness());
            }
            Err(UnitGrantError::Capacity) => return UnitDecision::Deny(UnitGrantError::Capacity),
            Err(error) => {
                near = error;
                resolver.sources.truncate(checkpoint);
            }
        }
    }
    match resolver.direct_org(request) {
        Ok(()) => UnitDecision::Allow(resolver.witness()),
        Err(UnitGrantError::Capacity) => UnitDecision::Deny(UnitGrantError::Capacity),
        Err(_) => UnitDecision::Deny(near),
    }
}

/// Validate a proposed new grant. Finite scopes prove every unit now; wildcard
/// scopes prove structural org authority now and re-check actual control on use.
pub fn validate_unit_grant(
    orgs: &OrgState,
    grants: &BTreeMap<UnitGrantId, UnitGrant>,
    facts: &dyn UnitAuthorityFacts,
    predicates: &dyn PredicateEval,
    candidate: &UnitGrant,
    now: i64,
) -> Result<UnitGrantWitness, UnitGrantError> {
    candidate.validate()?;
    if grants.len() >= MAX_UNIT_GRANTS {
        return Err(UnitGrantError::Capacity);
    }
    if grants.contains_key(&candidate.id) {
        return Err(UnitGrantError::Malformed("grant identity already exists"));
    }
    let mut resolver = Resolver::new(orgs, grants, facts, predicates, now, candidate.grantor)?;
    if let UnitGrantee::Character(actor) = candidate.grantee {
        resolver.actors.push(actor);
    }
    resolver.shape(candidate, 0)?;
    if let UnitScope::Units { units } = &candidate.scope {
        for unit in units {
            resolver.asset(*unit)?;
            resolver.walk(candidate, *unit, false, &candidate.grantee, 0)?;
        }
    }
    Ok(resolver.witness())
}

/// A prospective role declaration grants nobody a current right. The actual
/// mission owner must supply the exact accepted binding, leader and term at use.
/// This proves the same finite lending lineage as an ordinary new grant.
pub fn validate_mission_declaration(
    orgs: &OrgState,
    grants: &BTreeMap<UnitGrantId, UnitGrant>,
    facts: &dyn UnitAuthorityFacts,
    predicates: &dyn PredicateEval,
    candidate: &UnitGrant,
    now: i64,
) -> Result<UnitGrantWitness, UnitGrantError> {
    candidate.validate()?;
    if !matches!(candidate.grantee, UnitGrantee::CurrentLeader(_)) {
        return Err(UnitGrantError::NoAuthority);
    }
    if grants.len() >= MAX_UNIT_GRANTS {
        return Err(UnitGrantError::Capacity);
    }
    if grants.contains_key(&candidate.id) {
        return Err(UnitGrantError::Malformed("grant identity already exists"));
    }
    let mut resolver = Resolver::new(orgs, grants, facts, predicates, now, candidate.grantor)?;
    resolver.mission_lineage_only = true;
    resolver.shape(candidate, 0)?;
    let UnitScope::Units { units } = &candidate.scope else {
        return Err(UnitGrantError::Widening);
    };
    for unit in units {
        resolver.asset(*unit)?;
        resolver.walk(candidate, *unit, false, &candidate.grantee, 0)?;
    }
    Ok(resolver.witness())
}

/// One half of final admission: current owner/loan/policy backing for an actual
/// issued role grant. This alone deliberately says nothing about who leads.
pub(super) fn mission_lineage(
    orgs: &OrgState,
    grants: &BTreeMap<UnitGrantId, UnitGrant>,
    facts: &dyn UnitAuthorityFacts,
    predicates: &dyn PredicateEval,
    coordinate: (ObjId, ObjId, UnitGrantId),
    now: i64,
) -> Result<(), UnitGrantError> {
    let (actor, unit, grant) = coordinate;
    let grant = grants.get(&grant).ok_or(UnitGrantError::NoAuthority)?;
    if !matches!(grant.grantee, UnitGrantee::CurrentLeader(_)) {
        return Err(UnitGrantError::NoAuthority);
    }
    let mut resolver = Resolver::new(orgs, grants, facts, predicates, now, actor)?;
    resolver.mission_lineage_only = true;
    resolver.asset(unit)?;
    resolver.walk(grant, unit, false, &grant.grantee, 0)
}

/// Narrow retained custody, independently authenticated by its owning host.
/// There is deliberately no fire, deployment, repair, title or station action.
#[must_use]
pub fn authorize_recovery(facts: &dyn UnitAuthorityFacts, request: &RecoveryRequest) -> bool {
    if request.hull_epoch == 0 || !facts.principal_exists(request.actor) {
        return false;
    }
    let Some(custody) = facts.recovery_custody(request.unit, request.actor) else {
        return false;
    };
    if !custody.active
        || custody.policy != RecoveryPolicy::ReturnOnly
        || custody.unit != request.unit
        || custody.operator != request.actor
        || custody.hull_epoch != request.hull_epoch
    {
        return false;
    }
    match request.action {
        RecoveryAction::SafeStop | RecoveryAction::Disengage => true,
        RecoveryAction::ReturnToRecordedOrigin { origin } => custody.origin == origin,
        RecoveryAction::RecoverIntoRecordedBerth { berth } => custody.berth == berth,
    }
}
