//! [`OrgState`] — the in-RAM authoritative snapshot the [`authorize`](crate::authorize)
//! engine reads and the [`OrgMutationPlan`](crate::OrgMutationPlan) writes.
//!
//! It keeps the primary record maps plus the derived reverse indexes the hot
//! authorization lookups need (`by_member`, grants/delegations by grantee,
//! policies by org) — the R-R1 review point, so a per-actor lookup stays `O(k)`
//! rather than scanning all records. The indexes are always rebuilt from the
//! primary maps at boot (E1b); here they are maintained by the mutation methods.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;

use crate::ids::{DelegationId, GrantId, OrgId, OrgScopeId, PolicyId, RoleId};
use crate::record::{
    AccessPolicy, CapabilityGrant, Delegation, MembershipStatus, OrgMembership, OrgRole, OrgScope,
    Organization,
};

/// The authoritative organization state. Read by `authorize`; mutated only via
/// [`OrgMutationPlan`](crate::OrgMutationPlan)`::apply`, which keeps the derived
/// indexes consistent.
#[derive(Clone, Debug, Default)]
pub struct OrgState {
    orgs: BTreeMap<OrgId, Organization>,
    scopes: BTreeMap<OrgScopeId, OrgScope>,
    memberships: BTreeMap<(OrgId, ObjId), OrgMembership>,
    roles: BTreeMap<RoleId, OrgRole>,
    grants: BTreeMap<GrantId, CapabilityGrant>,
    delegations: BTreeMap<DelegationId, Delegation>,
    policies: BTreeMap<PolicyId, AccessPolicy>,
    // Derived reverse indexes (R-R1).
    by_member: BTreeMap<ObjId, BTreeSet<OrgId>>,
    grants_by_grantee: BTreeMap<ObjId, BTreeSet<GrantId>>,
    delegations_by_grantee: BTreeMap<ObjId, BTreeSet<DelegationId>>,
    policies_by_org: BTreeMap<OrgId, BTreeSet<PolicyId>>,
    roles_by_org: BTreeMap<OrgId, BTreeSet<RoleId>>,
}

impl OrgState {
    /// An empty state.
    #[must_use]
    pub fn new() -> OrgState {
        OrgState::default()
    }

    // ---- read accessors ---------------------------------------------------

    /// Look up an organization.
    pub fn org(&self, id: OrgId) -> Option<&Organization> {
        self.orgs.get(&id)
    }
    /// Look up a scope.
    pub fn scope(&self, id: OrgScopeId) -> Option<&OrgScope> {
        self.scopes.get(&id)
    }
    /// Look up a member's membership in an org.
    pub fn membership(&self, org: OrgId, member: ObjId) -> Option<&OrgMembership> {
        self.memberships.get(&(org, member))
    }
    /// The orgs an object is a member of (`O(k)` via the reverse index).
    pub fn member_orgs(&self, member: ObjId) -> impl Iterator<Item = OrgId> + '_ {
        self.by_member.get(&member).into_iter().flatten().copied()
    }
    /// A role template.
    pub fn role(&self, id: RoleId) -> Option<&OrgRole> {
        self.roles.get(&id)
    }
    /// A grant.
    pub fn grant(&self, id: GrantId) -> Option<&CapabilityGrant> {
        self.grants.get(&id)
    }
    /// A policy.
    pub fn policy(&self, id: PolicyId) -> Option<&AccessPolicy> {
        self.policies.get(&id)
    }
    /// A delegation.
    pub fn delegation(&self, id: DelegationId) -> Option<&Delegation> {
        self.delegations.get(&id)
    }

    pub(crate) fn grants_to(&self, grantee: ObjId) -> impl Iterator<Item = &CapabilityGrant> {
        // The trailing filter re-confirms the primary record still names this
        // grantee — the reverse index is a hint, never trusted on the authorize
        // hot path (a stale entry from any source narrows to nothing, never opens).
        self.grants_by_grantee
            .get(&grantee)
            .into_iter()
            .flatten()
            .filter_map(|g| self.grants.get(g))
            .filter(move |g| g.grantee == grantee)
    }
    pub(crate) fn delegations_to(&self, grantee: ObjId) -> impl Iterator<Item = &Delegation> {
        self.delegations_by_grantee
            .get(&grantee)
            .into_iter()
            .flatten()
            .filter_map(|d| self.delegations.get(d))
            .filter(move |d| d.grantee == grantee)
    }
    pub(crate) fn policies_of(&self, org: OrgId) -> impl Iterator<Item = &AccessPolicy> {
        self.policies_by_org
            .get(&org)
            .into_iter()
            .flatten()
            .filter_map(|p| self.policies.get(p))
            .filter(move |p| p.org == org)
    }
    /// Active-membership roles of an actor in an org (empty if not an active member).
    pub(crate) fn active_roles(&self, org: OrgId, actor: ObjId) -> &BTreeSet<RoleId> {
        static EMPTY: BTreeSet<RoleId> = BTreeSet::new();
        match self.memberships.get(&(org, actor)) {
            Some(m) if m.status == MembershipStatus::Active => &m.roles,
            _ => &EMPTY,
        }
    }

    // ---- mutators (index-maintaining; used by OrgMutationPlan::apply) ------

    pub(crate) fn put_org(&mut self, o: Organization) {
        self.orgs.insert(o.id, o);
    }
    pub(crate) fn put_scope(&mut self, s: OrgScope) {
        self.scopes.insert(s.id, s);
    }
    pub(crate) fn put_role(&mut self, r: OrgRole) {
        self.roles_by_org.entry(r.org).or_default().insert(r.id);
        self.roles.insert(r.id, r);
    }
    pub(crate) fn put_membership(&mut self, m: OrgMembership) {
        self.by_member.entry(m.member).or_default().insert(m.org);
        self.memberships.insert((m.org, m.member), m);
    }
    pub(crate) fn remove_membership(&mut self, org: OrgId, member: ObjId) {
        self.memberships.remove(&(org, member));
        if let Some(set) = self.by_member.get_mut(&member) {
            set.remove(&org);
            if set.is_empty() {
                self.by_member.remove(&member);
            }
        }
    }
    pub(crate) fn put_grant(&mut self, g: CapabilityGrant) {
        // On a replace whose grantee changed, drop the stale reverse-index entry so
        // the previous grantee is not left indexed (and authorized) for this grant.
        if let Some(prev) = self.grants.get(&g.id)
            && prev.grantee != g.grantee
            && let Some(set) = self.grants_by_grantee.get_mut(&prev.grantee)
        {
            set.remove(&g.id);
            if set.is_empty() {
                self.grants_by_grantee.remove(&prev.grantee);
            }
        }
        self.grants_by_grantee.entry(g.grantee).or_default().insert(g.id);
        self.grants.insert(g.id, g);
    }
    pub(crate) fn remove_grant(&mut self, id: GrantId) {
        if let Some(g) = self.grants.remove(&id)
            && let Some(set) = self.grants_by_grantee.get_mut(&g.grantee)
        {
            set.remove(&id);
            if set.is_empty() {
                self.grants_by_grantee.remove(&g.grantee);
            }
        }
    }
    pub(crate) fn put_delegation(&mut self, d: Delegation) {
        self.delegations_by_grantee.entry(d.grantee).or_default().insert(d.id);
        self.delegations.insert(d.id, d);
    }
    pub(crate) fn put_policy(&mut self, p: AccessPolicy) {
        // On a replace whose org changed, drop the stale by-org index entry so the
        // policy is not still evaluated under its previous org.
        if let Some(prev) = self.policies.get(&p.id)
            && prev.org != p.org
            && let Some(set) = self.policies_by_org.get_mut(&prev.org)
        {
            set.remove(&p.id);
            if set.is_empty() {
                self.policies_by_org.remove(&prev.org);
            }
        }
        self.policies_by_org.entry(p.org).or_default().insert(p.id);
        self.policies.insert(p.id, p);
    }

    /// Scrub every org binding of a destroyed object — the PRIMARY guard (a
    /// destroyed object grants nothing because it has no live records; E1b calls
    /// this on `ObjDestroyed`). Removes memberships, direct grants, and
    /// delegations to/from the object; also strips `AccessPolicy` rules and
    /// ownership referencing it (R-R5).
    pub fn scrub_object(&mut self, obj: ObjId) {
        let orgs: Vec<OrgId> = self.member_orgs(obj).collect();
        for org in orgs {
            self.remove_membership(org, obj);
        }
        let grant_ids: Vec<GrantId> =
            self.grants_by_grantee.get(&obj).into_iter().flatten().copied().collect();
        for g in grant_ids {
            self.remove_grant(g);
        }
        // Delegations to or from the object.
        let del_ids: Vec<DelegationId> = self
            .delegations
            .values()
            .filter(|d| d.grantee == obj || d.grantor == obj)
            .map(|d| d.id)
            .collect();
        for id in del_ids {
            if let Some(d) = self.delegations.remove(&id)
                && let Some(set) = self.delegations_by_grantee.get_mut(&d.grantee)
            {
                set.remove(&id);
                if set.is_empty() {
                    self.delegations_by_grantee.remove(&d.grantee);
                }
            }
        }
        // Policy hygiene: drop rules whose subject is this principal, and drop a
        // policy owned by this object entirely (R-R5). Absence only narrows.
        let owned: Vec<PolicyId> = self
            .policies
            .values()
            .filter(|p| matches!(&p.owner, crate::PrincipalId::Object(o) if *o == obj))
            .map(|p| p.id)
            .collect();
        for pid in owned {
            if let Some(p) = self.policies.remove(&pid)
                && let Some(set) = self.policies_by_org.get_mut(&p.org)
            {
                set.remove(&pid);
            }
        }
        for p in self.policies.values_mut() {
            p.rules.retain(|r| {
                !matches!(&r.subject, crate::PolicySubject::Principal(crate::PrincipalId::Object(o)) if *o == obj)
            });
        }
    }
}
