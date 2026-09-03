//! Boot: rebuild the [`OrgStore`] from persisted docs, fail-closed.
//!
//! **Effect-based polarity** (the E1 review, R-B3). Org and scope docs are
//! *containers*: a corrupt one FAILS the boot — proceeding would classify that
//! org's memberships, grants, and policies as orphans and permanently delete
//! them, and corrupt data is an operational problem to fix, never a license for
//! destructive "recovery". Membership/role/grant/delegation/policy docs are
//! *leaves*: an unparseable one is skipped loudly; an orphan (owning org gone, or
//! a membership whose object was destroyed) is dropped and its doc deleted.
//!
//! Every authority the engine reads is re-derived from live records, so the safe
//! failure is always *less* authority: a dropped record simply stops authorizing,
//! and dangling references (a scope whose parent is missing, a delegation whose
//! source grant is gone) are already fail-closed by [`crate::authorize`]'s
//! depth-bounded, existence-checking walks. The allocator resume never hands out a
//! live (or previously seen) id: `meta` is authoritative but floored at one past
//! the max id actually parsed.

use std::collections::BTreeMap;

use mush_core::is_log_key;
use mush_core::module::{ModError, ModuleBootData, ModuleCtx};
use mush_core::{Flags, Ledger};
use smol_str::SmolStr;

use crate::api::{self, AUDIT_LOG};
use crate::ids::{OrgId, OrgScopeId, PrincipalId};
use crate::module::OrgStore;
use crate::record::{
    AccessPolicy, CapabilityGrant, Delegation, OrgMembership, OrgRole, OrgScope, Organization,
    PolicySubject,
};
use crate::state::OrgState;

/// Whether `obj` is a still-live object in the world (not missing, not GOING) —
/// the liveness predicate the boot reconciliation drops dead-object bindings by.
fn obj_live(ctx: &ModuleCtx<'_>, obj: mush_core::ObjId) -> bool {
    ctx.w.get(obj).is_some_and(|o| !o.flags.contains(Flags::GOING))
}

/// Rebuild the [`OrgStore`] (state + audit ledger + allocators) from boot data.
pub(crate) fn load_boot(ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<OrgStore, ModError> {
    // The audit ledger rehydrates its open tail directly from the log chunks.
    let mut audit = Ledger::new();
    audit
        .restore_from_kv(&data.kv)
        .map_err(|error| ModError::msg(format!("org: audit ledger restore refused: {error}")))?;

    let mut orgs: BTreeMap<OrgId, Organization> = BTreeMap::new();
    let mut scopes: BTreeMap<OrgScopeId, OrgScope> = BTreeMap::new();
    let mut roles: Vec<OrgRole> = Vec::new();
    let mut grants: Vec<CapabilityGrant> = Vec::new();
    let mut delegations: Vec<Delegation> = Vec::new();
    let mut policies: Vec<AccessPolicy> = Vec::new();
    let mut members: Vec<OrgMembership> = Vec::new();
    let mut meta: Option<serde_json::Value> = None;
    let mut dels: Vec<SmolStr> = Vec::new();

    for (key, value) in data.kv {
        if key == api::META_KEY {
            meta = Some(value);
        } else if is_log_key(&key) {
            // Audit chunk — already rehydrated by `restore_from_kv`.
        } else if let Some(ids) = key.strip_prefix("org:") {
            // Container: any defect FAILS the boot.
            let id = ids
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("org: bad org key {key:?}")))?;
            let o = serde_json::from_value::<Organization>(value)
                .map_err(|e| ModError::msg(format!("org: bad org doc {key:?}: {e}")))?;
            if o.id != OrgId(id) {
                return Err(ModError::msg(format!("org: doc {key:?} claims id {}", o.id)));
            }
            orgs.insert(o.id, o);
        } else if let Some(ids) = key.strip_prefix("scope:") {
            // Container: any defect FAILS the boot.
            let id = ids
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("org: bad scope key {key:?}")))?;
            let s = serde_json::from_value::<OrgScope>(value)
                .map_err(|e| ModError::msg(format!("org: bad scope doc {key:?}: {e}")))?;
            if s.id != OrgScopeId(id) {
                return Err(ModError::msg(format!("org: scope doc {key:?} claims id {}", s.id)));
            }
            scopes.insert(s.id, s);
        } else if key.strip_prefix("role:").is_some() {
            parse_leaf(&key, value, "role", role_ck, &mut roles, &mut dels);
        } else if key.strip_prefix("grant:").is_some() {
            parse_leaf(&key, value, "grant", grant_ck, &mut grants, &mut dels);
        } else if key.strip_prefix("delegation:").is_some() {
            parse_leaf(&key, value, "delegation", del_ck, &mut delegations, &mut dels);
        } else if key.strip_prefix("policy:").is_some() {
            parse_leaf(&key, value, "policy", policy_ck, &mut policies, &mut dels);
        } else if key.starts_with("member:") {
            // Leaf; composite key is re-derived from the doc, so no key check.
            match serde_json::from_value::<OrgMembership>(value) {
                Ok(m) => members.push(m),
                Err(e) => {
                    tracing::error!(%key, error = %e, "org: bad membership doc skipped");
                    dels.push(key);
                }
            }
        } else {
            tracing::warn!(%key, "org: unknown persisted key ignored");
        }
    }

    // Allocator floor: one past the max id ever parsed (kept records AND orphans
    // about to be dropped), so a dropped id is never re-minted.
    let max_org = orgs.keys().map(|i| i.0).max();
    let max_scope = scopes.keys().map(|i| i.0).max();
    let max_role = roles.iter().map(|r| r.id.0).max();
    let max_grant = grants.iter().map(|g| g.id.0).max();
    let max_del = delegations.iter().map(|d| d.id.0).max();
    let max_policy = policies.iter().map(|p| p.id.0).max();

    // Build the live state, dropping orphans (and deleting their docs). Containers
    // first, then leaves in dependency order (org must exist; a membership's object
    // must be live).
    let mut st = OrgState::new();
    for (_, o) in orgs {
        st.put_org(o);
    }
    for (_, s) in scopes {
        st.put_scope(s);
    }
    for r in roles {
        if st.org(r.org).is_some() {
            st.put_role(r);
        } else {
            tracing::warn!(role = r.id.0, org = r.org.0, "org: role for a missing org dropped");
            dels.push(api::role_key(r.id));
        }
    }
    // Grants: the owning org must exist AND the grantee object must be live. The
    // liveness drop is the boot half of the destroy scrub — the `ObjDestroyed`
    // observer is not guaranteed to have run (the module may have been unloaded, or
    // a programmatic `World::destroy` bypassed the event), so boot reconciles
    // against the live world exactly as the faction registry does.
    for g in grants {
        if st.org(g.org).is_some() && obj_live(ctx, g.grantee) {
            st.put_grant(g);
        } else {
            tracing::warn!(grant = g.id.0, org = g.org.0, "org: grant for a missing org or dead grantee dropped");
            dels.push(api::grant_key(g.id));
        }
    }
    // Delegations: org + source grant must exist AND both grantor and grantee must
    // be live. The grantor-liveness drop is load-bearing, not tidiness: a delegation
    // whose grantor was destroyed would otherwise reload with `src.grantee ==
    // d.grantor` (both the same dead id) and re-grant the *surviving* grantee the
    // delegated capability — authority that must have died with the grantor.
    for d in delegations {
        let anchored = st.org(d.org).is_some() && st.grant(d.source_grant).is_some();
        if anchored && obj_live(ctx, d.grantor) && obj_live(ctx, d.grantee) {
            st.put_delegation(d);
        } else {
            tracing::warn!(delegation = d.id.0, "org: delegation with a missing anchor or dead grantor/grantee dropped");
            dels.push(api::delegation_key(d.id));
        }
    }
    // Policies: the org must exist; a policy OWNED by a dead object is dropped, and
    // rules naming a dead object as subject are stripped (parity with `scrub_object`
    // so a boot-without-observer converges to the same state a live destroy would).
    for mut p in policies {
        if st.org(p.org).is_none() {
            tracing::warn!(policy = p.id.0, org = p.org.0, "org: policy for a missing org dropped");
            dels.push(api::policy_key(p.id));
            continue;
        }
        if matches!(&p.owner, PrincipalId::Object(o) if !obj_live(ctx, *o)) {
            tracing::warn!(policy = p.id.0, "org: policy owned by a dead object dropped");
            dels.push(api::policy_key(p.id));
            continue;
        }
        let before = p.rules.len();
        p.rules.retain(|r| !matches!(&r.subject, PolicySubject::Principal(PrincipalId::Object(o)) if !obj_live(ctx, *o)));
        if p.rules.len() != before {
            tracing::warn!(policy = p.id.0, "org: policy rules naming a dead object stripped");
            api::persist_policy(ctx.out, &p); // re-persist the shrunk rule list
        }
        st.put_policy(p);
    }
    for m in members {
        if obj_live(ctx, m.member) && st.org(m.org).is_some() {
            st.put_membership(m);
        } else {
            tracing::warn!(org = m.org.0, member = m.member.0, "org: orphan membership dropped");
            dels.push(api::member_key(m.org, m.member));
        }
    }

    // Resume the allocators: `meta` is authoritative (it commits in the same batch
    // as every mutation), floored at the parsed max so a stale/partial meta can
    // never re-mint a live id.
    let next_org = meta_u64(&meta, "next_org_id").max(next_after(max_org));
    let next_scope = meta_u64(&meta, "next_scope_id").max(next_after(max_scope));
    let next_role = meta_u64(&meta, "next_role_id").max(next_after(max_role));
    let next_grant = meta_u64(&meta, "next_grant_id").max(next_after(max_grant));
    let next_delegation = meta_u64(&meta, "next_delegation_id").max(next_after(max_del));
    let next_policy = meta_u64(&meta, "next_policy_id").max(next_after(max_policy));

    for key in dels {
        api::persist_del(ctx.out, key);
    }

    // Sanity: the audit log must have rehydrated coherently (a breach would have
    // been reported by `restore_from_kv`). Nothing further to fix here.
    let _ = audit.next_seq(AUDIT_LOG);

    Ok(OrgStore::from_parts(
        st,
        audit,
        next_org,
        next_scope,
        next_role,
        next_grant,
        next_delegation,
        next_policy,
    ))
}

/// Parse a numeric-keyed leaf doc into `sink`, skipping (and deleting) an
/// unparseable one; a doc whose canonical key differs from its raw key is loaded
/// under its true id and the stale raw key is deleted.
fn parse_leaf<T: serde::de::DeserializeOwned>(
    key: &SmolStr,
    value: serde_json::Value,
    kind: &str,
    canonical: fn(&T) -> SmolStr,
    sink: &mut Vec<T>,
    dels: &mut Vec<SmolStr>,
) {
    match serde_json::from_value::<T>(value) {
        Ok(rec) => {
            if canonical(&rec) != *key {
                tracing::error!(%key, kind, "org: leaf doc key mismatch; stale key deleted");
                dels.push(key.clone());
            }
            sink.push(rec);
        }
        Err(e) => {
            tracing::error!(%key, kind, error = %e, "org: unparseable leaf doc skipped");
            dels.push(key.clone());
        }
    }
}

fn role_ck(r: &OrgRole) -> SmolStr {
    api::role_key(r.id)
}
fn grant_ck(g: &CapabilityGrant) -> SmolStr {
    api::grant_key(g.id)
}
fn del_ck(d: &Delegation) -> SmolStr {
    api::delegation_key(d.id)
}
fn policy_ck(p: &AccessPolicy) -> SmolStr {
    api::policy_key(p.id)
}

fn meta_u64(meta: &Option<serde_json::Value>, field: &str) -> u64 {
    meta.as_ref().and_then(|m| m.get(field)).and_then(serde_json::Value::as_u64).unwrap_or(0)
}
fn next_after(max: Option<u64>) -> u64 {
    max.map_or(0, |m| m + 1)
}
