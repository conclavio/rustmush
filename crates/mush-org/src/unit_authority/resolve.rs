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
    control_path: BTreeSet<(OrgId, OrgScopeId)>,
    visits: usize,
    visit_limit: usize,
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
            control_path: BTreeSet::new(),
            visits: 0,
            visit_limit: MAX_UNIT_PROOF_VISITS,
            sources: Vec::new(),
            checking_exclusions: false,
            mission_lineage_only: false,
        })
    }

    fn charge(&mut self) -> Result<(), UnitGrantError> {
        if self.visits >= self.visit_limit {
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
            UnitGrantParent::LeasedOrganizationGrant {
                organization,
                grant,
                lease,
            } => {
                if !self.facts.equipment_lease(*organization, lease) {
                    return Err(UnitGrantError::Stale);
                }
                return self.root(
                    &UnitGrantParent::OrganizationGrant {
                        organization: *organization,
                        grant: *grant,
                    },
                    actor,
                    capability,
                    delegate,
                );
            }
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
        if is_self_loan(grant) {
            return Err(UnitGrantError::Cycle);
        }
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
                control_path: BTreeSet::new(),
                visits: self.visits,
                visit_limit: self.visit_limit,
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
        let key = (request.organization, request.selector.root());
        if !self.control_path.insert(key) {
            return Err(UnitGrantError::Cycle);
        }
        let result = self.control_inner(request);
        self.control_path.remove(&key);
        result
    }

    fn control_inner(
        &mut self,
        request: ControlRequest<'_>,
    ) -> Result<ControlProof, UnitGrantError> {
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
        // booleans. Pre-filter active, in-window, delegable, narrowing,
        // non-self-loan, non-cyclic, and scope-matching rows before charging
        // proof visits, prioritize directly-backed candidates ahead of indirect
        // loan chains, and isolate each candidate under a fair visit sub-budget
        // so an attacker's unsolicited loan bomb can never abort legitimate
        // higher-ID loans.
        let mut direct_candidates = Vec::new();
        let mut indirect_candidates = Vec::new();
        for (id, grant) in self.grants {
            if grant.id != *id
                || grant.status != UnitGrantStatus::Active
                || grant.capability != capability
                || (delegate && !grant.delegable)
                || is_self_loan(grant)
                || !time_valid(grant.valid_from, grant.valid_until, self.now)
                || !window.is_none_or(|(from, until)| {
                    window_narrows(grant.valid_from, grant.valid_until, from, until)
                })
                || (recovery == RecoveryPolicy::ReturnOnly
                    && grant.recovery != RecoveryPolicy::ReturnOnly)
            {
                continue;
            }
            let UnitGrantee::Organization {
                organization: grantee_org,
                scope: grantee_scope,
            } = grant.grantee
            else {
                continue;
            };
            if grantee_org != organization
                || !self.scope_active(grantee_org, grantee_scope)
                || !scope_contains(self.orgs, selector, grantee_scope)
            {
                continue;
            }
            match &grant.scope {
                UnitScope::Units { units } => {
                    if units.contains(&unit) {
                        direct_candidates.push(grant.clone());
                    }
                }
                UnitScope::OrganizationControlled {
                    organization: scope_org,
                    scope: scope_sel,
                    excluded,
                } => {
                    if excluded.contains(&unit)
                        || !self.scope_active(*scope_org, scope_sel.root())
                        || self.control_path.contains(&(*scope_org, scope_sel.root()))
                    {
                        continue;
                    }
                    let directly_backed = (asset.owner == PrincipalId::Organization(*scope_org)
                        && asset.scope.is_some_and(|s| {
                            self.scope_active(*scope_org, s)
                                && scope_contains(self.orgs, scope_sel, s)
                        }))
                        || self
                            .facts
                            .organization_control(*scope_org, unit, capability, self.now)
                            .is_some();
                    if directly_backed {
                        direct_candidates.push(grant.clone());
                    } else {
                        indirect_candidates.push(grant.clone());
                    }
                }
            }
        }
        direct_candidates.extend(indirect_candidates);
        let total = direct_candidates.len();
        let mut saw_capacity = false;
        for (idx, grant) in direct_candidates.into_iter().enumerate() {
            let UnitGrantee::Organization { scope, .. } = grant.grantee else {
                continue;
            };
            let remaining = self.visit_limit.saturating_sub(self.visits);
            if remaining == 0 {
                saw_capacity = true;
                break;
            }
            let left = total - idx;
            let share = (remaining / left).max(1);
            let saved_limit = self.visit_limit;
            self.visit_limit = self.visits.saturating_add(share).min(saved_limit);
            let inserted_grantee_scope = self.control_path.insert((organization, scope));
            let checkpoint = self.sources.len();
            let step = self.walk(
                &grant,
                unit,
                delegate,
                &UnitGrantee::Organization {
                    organization,
                    scope,
                },
                depth,
            );
            if inserted_grantee_scope {
                self.control_path.remove(&(organization, scope));
            }
            self.visit_limit = saved_limit;
            match step {
                Ok(()) => {
                    self.policy(scope, capability)?;
                    return Ok(ControlProof { scope });
                }
                Err(UnitGrantError::Capacity) => {
                    saw_capacity = true;
                    self.sources.truncate(checkpoint);
                }
                Err(_) => self.sources.truncate(checkpoint),
            }
        }
        if saw_capacity {
            Err(UnitGrantError::Capacity)
        } else {
            Err(UnitGrantError::NoAuthority)
        }
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
        // Isolate each org root on its own sub-budget so an unsolicited loan
        // bomb targeting one organization can never abort another org root's
        // proof with Capacity.
        let base_visits = self.visits;
        let per_root_limit = self.visit_limit.saturating_sub(base_visits);
        let mut saw_capacity = false;
        for parent in roots {
            let Ok(root) = self.root(
                &parent,
                request.actor,
                capability,
                request.action.requires_delegation(),
            ) else {
                continue;
            };
            let mut sub = Resolver {
                orgs: self.orgs,
                grants: self.grants,
                facts: self.facts,
                predicates: self.predicates,
                now: self.now,
                actors: vec![request.actor],
                path: BTreeSet::new(),
                control_path: BTreeSet::new(),
                visits: 0,
                visit_limit: per_root_limit,
                sources: Vec::new(),
                checking_exclusions: false,
                mission_lineage_only: self.mission_lineage_only,
            };
            let step = sub
                .control(ControlRequest {
                    organization: root.organization,
                    unit: request.unit,
                    capability,
                    delegate: request.action.requires_delegation(),
                    selector: &root.scope,
                    depth: 0,
                    window: None,
                    recovery: RecoveryPolicy::None,
                })
                .and_then(|control| sub.policy(control.scope, capability));
            match step {
                Ok(()) => {
                    for src in sub.sources {
                        self.source(src);
                    }
                    self.source(root.source);
                    return Ok(());
                }
                Err(UnitGrantError::Capacity) => {
                    saw_capacity = true;
                }
                Err(_) => {}
            }
        }
        if saw_capacity {
            Err(UnitGrantError::Capacity)
        } else {
            Err(UnitGrantError::NoAuthority)
        }
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

fn is_self_loan(grant: &UnitGrant) -> bool {
    matches!(
        (&grant.grantee, &grant.scope),
        (
            UnitGrantee::Organization {
                organization: grantee_org,
                ..
            },
            UnitScope::OrganizationControlled {
                organization: scope_org,
                ..
            },
        ) if grantee_org == scope_org
    )
}

fn active_loan_orgs(
    grants: &BTreeMap<UnitGrantId, UnitGrant>,
    capability: UnitCapability,
    unit: ObjId,
    now: i64,
) -> BTreeSet<OrgId> {
    grants
        .iter()
        .filter_map(|(id, grant)| {
            if grant.id == *id
                && grant.status == UnitGrantStatus::Active
                && grant.capability == capability
                && !is_self_loan(grant)
                && time_valid(grant.valid_from, grant.valid_until, now)
                && match &grant.scope {
                    UnitScope::Units { units } => units.contains(&unit),
                    UnitScope::OrganizationControlled { excluded, .. } => !excluded.contains(&unit),
                }
                && let UnitGrantee::Organization { organization, .. } = grant.grantee
            {
                Some(organization)
            } else {
                None
            }
        })
        .collect()
}

fn scope_may_match_unit(
    orgs: &OrgState,
    asset: &UnitAssetFacts,
    facts: &dyn UnitAuthorityFacts,
    loan_orgs: &BTreeSet<OrgId>,
    scope: &UnitScope,
    request: &UnitAuthRequest,
) -> bool {
    let unit = request.unit;
    let capability = request.action.capability();
    let now = request.now;
    match scope {
        UnitScope::Units { units } => units.contains(&unit),
        UnitScope::OrganizationControlled {
            organization,
            scope,
            excluded,
        } => {
            if excluded.contains(&unit)
                || !orgs
                    .org(*organization)
                    .is_some_and(|org| org.status == OrgStatus::Active)
                || !orgs.scope(scope.root()).is_some_and(|row| {
                    row.org == *organization && row.status == ScopeStatus::Active
                })
            {
                return false;
            }
            let owns_in_scope = asset.owner == PrincipalId::Organization(*organization)
                && asset.scope.is_some_and(|s| {
                    orgs.scope(s).is_some_and(|row| {
                        row.org == *organization && row.status == ScopeStatus::Active
                    }) && scope_contains(orgs, scope, s)
                });
            owns_in_scope
                || facts
                    .organization_control(*organization, unit, capability, now)
                    .is_some()
                || loan_orgs.contains(organization)
        }
    }
}

fn grant_is_live(grants: &BTreeMap<UnitGrantId, UnitGrant>, grant: &UnitGrant, now: i64) -> bool {
    let mut current = grant;
    for _ in 0..=MAX_UNIT_GRANT_DEPTH {
        if current.status != UnitGrantStatus::Active
            || current.valid_until.is_some_and(|end| now >= end)
        {
            return false;
        }
        match current.parent {
            UnitGrantParent::UnitGrant { id } => {
                let Some(parent) = grants.get(&id) else {
                    return false;
                };
                if parent.id >= current.id {
                    return false;
                }
                current = parent;
            }
            _ => return true,
        }
    }
    false
}

fn check_grant_quotas(
    grants: &BTreeMap<UnitGrantId, UnitGrant>,
    candidate: &UnitGrant,
    now: i64,
) -> Result<(), UnitGrantError> {
    let mut grantor_count = 0usize;
    for existing in grants.values() {
        if !grant_is_live(grants, existing, now) {
            continue;
        }
        if existing.grantor == candidate.grantor {
            grantor_count += 1;
            if grantor_count >= MAX_ACTIVE_GRANTS_PER_GRANTOR {
                return Err(UnitGrantError::Capacity);
            }
            if existing.grantee == candidate.grantee
                && existing.capability == candidate.capability
                && existing.scope == candidate.scope
                && existing.delegable == candidate.delegable
                && existing.recovery == candidate.recovery
                && existing.parent == candidate.parent
                && existing.valid_until.is_none_or(|existing_end| {
                    candidate
                        .valid_until
                        .is_some_and(|cand_end| cand_end <= existing_end)
                })
                && existing.valid_from.is_none_or(|existing_start| {
                    candidate
                        .valid_from
                        .is_some_and(|cand_start| cand_start >= existing_start)
                })
            {
                return Err(UnitGrantError::Malformed("duplicate active unit grant"));
            }
        }
    }
    if let UnitScope::Units { units } = &candidate.scope {
        for unit in units {
            let unit_count = grants
                .values()
                .filter(|existing| {
                    grant_is_live(grants, existing, now)
                        && matches!(
                            &existing.scope,
                            UnitScope::Units { units: u } if u.contains(unit)
                        )
                })
                .count();
            if unit_count >= MAX_ACTIVE_GRANTS_PER_UNIT {
                return Err(UnitGrantError::Capacity);
            }
        }
    }
    Ok(())
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
    let capability = request.action.capability();
    let requires_delegation = request.action.requires_delegation();
    let loan_orgs = active_loan_orgs(grants, capability, request.unit, request.now);
    let mut near = UnitGrantError::NoAuthority;
    let mut grant_capacity = false;
    for (id, grant) in grants {
        if grant.id != *id || grant.capability != capability {
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
        if requires_delegation && !grant.delegable {
            continue;
        }
        if !scope_may_match_unit(orgs, &asset, facts, &loan_orgs, &grant.scope, request) {
            if matches!(
                &grant.scope,
                UnitScope::OrganizationControlled { excluded, .. }
                    if excluded.contains(&request.unit)
            ) && near == UnitGrantError::NoAuthority
            {
                near = UnitGrantError::Excluded;
            }
            continue;
        }
        if grant.status != UnitGrantStatus::Active
            || !time_valid(grant.valid_from, grant.valid_until, request.now)
        {
            if near == UnitGrantError::NoAuthority {
                near = UnitGrantError::Inactive;
            }
            continue;
        }
        // Isolate every candidate grant on its own fresh proof budget so an
        // attacker's unsolicited grant bomb at a lower ID can never exhaust the
        // budget or abort legitimate higher-ID grants.
        let mut candidate_resolver =
            match Resolver::new(orgs, grants, facts, predicates, request.now, request.actor) {
                Ok(r) => r,
                Err(error) => return UnitDecision::Deny(error),
            };
        match candidate_resolver.walk(grant, request.unit, requires_delegation, &grant.grantee, 0) {
            Ok(()) => {
                if let UnitGrantee::CurrentLeader(binding) = &grant.grantee
                    && let Some(command) = &request.mission
                {
                    candidate_resolver.source(UnitAuthoritySource::Mission {
                        binding: binding.clone(),
                        term: command.term,
                    });
                }
                return UnitDecision::Allow(candidate_resolver.witness());
            }
            Err(UnitGrantError::Capacity) => {
                grant_capacity = true;
            }
            Err(error) => {
                near = error;
            }
        }
    }
    // Evaluate direct organization authority on its own fresh proof budget so
    // third-party grant floods can never starve direct org access into Capacity.
    let mut direct_resolver =
        match Resolver::new(orgs, grants, facts, predicates, request.now, request.actor) {
            Ok(resolver) => resolver,
            Err(error) => return UnitDecision::Deny(error),
        };
    match direct_resolver.direct_org(request) {
        Ok(()) => UnitDecision::Allow(direct_resolver.witness()),
        Err(UnitGrantError::Capacity) => UnitDecision::Deny(UnitGrantError::Capacity),
        Err(_) if grant_capacity => UnitDecision::Deny(UnitGrantError::Capacity),
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
    check_grant_quotas(grants, candidate, now)?;
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
    check_grant_quotas(grants, candidate, now)?;
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
