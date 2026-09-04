//! The E1b public seam + persistence layer.
//!
//! Two things other modules (E1c faction, E2 economy) consume:
//!
//! - [`authorize_in_world`] — *may this actor exercise this capability on this
//!   scope now?* over the live [`OrgStore`] slot, with the frozen predicate table
//!   bridged in. The pure engine ([`crate::authorize`]) does the work; this only
//!   supplies live state.
//! - [`submit`] — apply a validated [`OrgMutationPlan`] to the live state and
//!   persist exactly the touched docs + an audit entry + the allocator `meta`,
//!   all on the caller's write batch (the E0 atomic-durability discipline: the
//!   whole bundle commits together or not at all).
//!
//! Persistence layout (namespace [`NS`] = `org`): one doc per record —
//! `org:{id}`, `scope:{id}`, `role:{id}`, `grant:{id}`, `delegation:{id}`,
//! `policy:{id}`, `member:{org}:{obj}` — a `meta` doc for the id allocators, and
//! the append-only audit ledger under log [`AUDIT_LOG`].

use mush_core::{Ctx, Deferred, ModuleOp, ModuleWrite, ObjId, Outbox, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::authorize::{AuthRequest, Decision, DenyReason};
use crate::ids::{DelegationId, GrantId, OrgId, OrgScopeId, PolicyId, RoleId};
use crate::module::OrgStore;
use crate::plan::{OrgError, OrgMutationOp, OrgMutationPlan};
use crate::record::{AccessPolicy, CapabilityGrant, Delegation, OrgMembership, OrgRole, OrgScope, Organization};
use crate::wiring::WorldPredicates;

/// The org persistence namespace (`mod:org:*`).
pub const NS: &str = "org";
/// The kv key of the meta document (the id-allocator resume point).
pub const META_KEY: &str = "meta";
/// The single ledger log id for the org action audit trail.
pub const AUDIT_LOG: &str = "audit";

// ---- keys -----------------------------------------------------------------

/// `org:{id}`.
#[must_use]
pub fn org_key(id: OrgId) -> SmolStr {
    SmolStr::new(format!("org:{}", id.0))
}
/// `scope:{id}`.
#[must_use]
pub fn scope_key(id: OrgScopeId) -> SmolStr {
    SmolStr::new(format!("scope:{}", id.0))
}
/// `role:{id}`.
#[must_use]
pub fn role_key(id: RoleId) -> SmolStr {
    SmolStr::new(format!("role:{}", id.0))
}
/// `grant:{id}`.
#[must_use]
pub fn grant_key(id: GrantId) -> SmolStr {
    SmolStr::new(format!("grant:{}", id.0))
}
/// `delegation:{id}`.
#[must_use]
pub fn delegation_key(id: DelegationId) -> SmolStr {
    SmolStr::new(format!("delegation:{}", id.0))
}
/// `policy:{id}`.
#[must_use]
pub fn policy_key(id: PolicyId) -> SmolStr {
    SmolStr::new(format!("policy:{}", id.0))
}
/// `member:{org}:{obj}`.
#[must_use]
pub fn member_key(org: OrgId, obj: ObjId) -> SmolStr {
    SmolStr::new(format!("member:{}:{}", org.0, obj.0))
}

// ---- persist helpers ------------------------------------------------------

/// Queue one org write on the outbox deferred side-channel.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp { ns: SmolStr::new_static(NS), op }));
}
fn json_set(out: &mut Outbox, key: SmolStr, value: serde_json::Value) {
    push_op(out, ModuleWrite::JsonSet { key, value });
}
/// Delete a raw doc key.
pub(crate) fn persist_del(out: &mut Outbox, key: SmolStr) {
    push_op(out, ModuleWrite::Del { key });
}
pub(crate) fn persist_org(out: &mut Outbox, o: &Organization) {
    json_set(out, org_key(o.id), serde_json::to_value(o).expect("org serializes"));
}
pub(crate) fn persist_scope(out: &mut Outbox, s: &OrgScope) {
    json_set(out, scope_key(s.id), serde_json::to_value(s).expect("scope serializes"));
}
pub(crate) fn persist_role(out: &mut Outbox, r: &OrgRole) {
    json_set(out, role_key(r.id), serde_json::to_value(r).expect("role serializes"));
}
pub(crate) fn persist_membership(out: &mut Outbox, m: &OrgMembership) {
    json_set(out, member_key(m.org, m.member), serde_json::to_value(m).expect("membership serializes"));
}
pub(crate) fn persist_grant(out: &mut Outbox, g: &CapabilityGrant) {
    json_set(out, grant_key(g.id), serde_json::to_value(g).expect("grant serializes"));
}
pub(crate) fn persist_delegation(out: &mut Outbox, d: &Delegation) {
    json_set(out, delegation_key(d.id), serde_json::to_value(d).expect("delegation serializes"));
}
pub(crate) fn persist_policy(out: &mut Outbox, p: &AccessPolicy) {
    json_set(out, policy_key(p.id), serde_json::to_value(p).expect("policy serializes"));
}
/// Mirror the meta document (the id-allocator resume points).
pub(crate) fn persist_meta(out: &mut Outbox, meta: serde_json::Value) {
    json_set(out, SmolStr::new_static(META_KEY), meta);
}

// ---- audit ----------------------------------------------------------------

/// What an actor did, for the append-only audit ledger. Built by the command
/// layer and stamped with actor + wall-clock time in [`submit`].
#[derive(Clone, Debug, PartialEq)]
pub struct OrgAction {
    /// A stable action token, e.g. `"org.create"`, `"grant.create"`.
    pub action: SmolStr,
    /// The org the action touched, if any.
    pub org: Option<OrgId>,
    /// Structured, action-specific detail.
    pub detail: serde_json::Value,
}

/// One recorded org action — the payload stored in the audit ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrgAuditEntry {
    /// Wall-clock record time (unix seconds).
    pub at: i64,
    /// Who acted.
    pub actor: ObjId,
    /// The action token.
    pub action: SmolStr,
    /// The org touched, if any.
    #[serde(default)]
    pub org: Option<OrgId>,
    /// Action-specific detail.
    #[serde(default)]
    pub detail: serde_json::Value,
}

/// Append one audit entry **without** an org mutation (E3 leftover, closed in E4c;
/// 08c §6.1 item 4, review finding G1).
///
/// [`submit`] is the only way an action reached the ledger, and it needs an
/// [`OrgMutationPlan`] — so an act that is *authorized by* org state but changes
/// none of it had nowhere to be recorded. The economy's `@econ/approve` and
/// `@econ/deny` are exactly that shape: the decision that releases (or refuses) a
/// scoped payment is an org-authority act whose record lived only in the economy's
/// own approval document, invisible to `@org/audit`.
///
/// It is the tail of [`submit`] and nothing else: prepare the append, apply it,
/// queue the op on the caller's batch. There is no state install, so the
/// "terminal on the success path" rule is weaker here than for `submit` — a later
/// `Err` truncates the deferred write and the RAM tail is the only thing left ahead,
/// which the next boot re-derives. Call it after the act it records has committed.
///
/// # Errors
///
/// [`OrgError::Invalid`] if the org module is not loaded or the audit ledger refuses
/// the append.
pub fn audit(ctx: &mut Ctx<'_>, action: OrgAction) -> Result<(), OrgError> {
    let entry = OrgAuditEntry {
        at: ctx.w.now(),
        actor: ctx.actor,
        action: action.action,
        org: action.org,
        detail: action.detail,
    };
    let prepared = ctx
        .w
        .module_state::<OrgStore>()
        .ok_or_else(|| OrgError::Invalid("org module not loaded".into()))?
        .audit
        .prepare_append(AUDIT_LOG, serde_json::to_value(&entry).expect("audit entry serializes"))
        .map_err(|error| OrgError::Invalid(format!("audit append refused: {error}")))?;
    let op = {
        let store = ctx
            .w
            .module_state_mut::<OrgStore>()
            .ok_or_else(|| OrgError::Invalid("org module not loaded".into()))?;
        store
            .audit
            .apply_prepared(prepared)
            .map_err(|error| OrgError::Invalid(format!("audit append became stale: {error}")))?
            .1
    };
    push_op(ctx.out, op);
    Ok(())
}

// ---- the world authorization seam -----------------------------------------

/// Authorize (or refuse) a request against the live world's org state. The public
/// entry point E2 (and any capability check) calls. If the org module is not
/// loaded, this fails closed with [`DenyReason::NoAuthority`].
#[must_use]
pub fn authorize_in_world(w: &World, req: &AuthRequest) -> Decision {
    let Some(store) = w.module_state::<OrgStore>() else {
        return Decision::Deny(DenyReason::NoAuthority);
    };
    // Clone the frozen Arc so the bridge owns it (no second borrow of the slot);
    // `&store.state` and the bridge's `&World` are both shared borrows of `w`.
    let providers = store.providers();
    let bridge = WorldPredicates { w, providers };
    crate::authorize::authorize(store.state(), req, &bridge)
}

// ---- the mutation seam ----------------------------------------------------

/// Apply `plan` to the live org state and persist the result. Validation is the
/// pure [`OrgMutationPlan::apply`] (refuse-whole on any error, nothing persisted).
/// On success this installs the new state, writes exactly the touched docs, an
/// audit entry, and the refreshed `meta` — all on the caller's batch.
///
/// **Terminal on the success path** (the canon/scene discipline): call it as the
/// last mutating act, because a later `Err` truncates the deferred persists but
/// does not roll back the in-RAM state install.
pub fn submit(ctx: &mut Ctx<'_>, plan: &OrgMutationPlan, action: OrgAction) -> Result<(), OrgError> {
    // Validate + compute the post-image against the live state (no mutation yet).
    let post = {
        let store = ctx
            .w
            .module_state::<OrgStore>()
            .ok_or_else(|| OrgError::Invalid("org module not loaded".into()))?;
        plan.apply(store.state())?
    };
    let now = ctx.w.now();
    let actor = ctx.actor;
    let audit_entry = OrgAuditEntry {
        at: now,
        actor,
        action: action.action,
        org: action.org,
        detail: action.detail,
    };
    // Construct and bound the exact audit post-image before changing either
    // the audit tail or the live organization state.
    let prepared_audit = ctx
        .w
        .module_state::<OrgStore>()
        .expect("org store slot")
        .audit
        .prepare_append(
            AUDIT_LOG,
            serde_json::to_value(&audit_entry).expect("audit entry serializes"),
        )
        .map_err(|error| OrgError::Invalid(format!("audit append refused: {error}")))?;
    // Queue projections only after the append is known to be representable. If
    // installation becomes stale, the caller's output checkpoint discards them.
    persist_ops(ctx.out, &plan.ops, &post);
    let (audit_op, meta) = {
        let store = ctx.w.module_state_mut::<OrgStore>().expect("org store slot");
        let (_seq, audit_op) = store
            .audit
            .apply_prepared(prepared_audit)
            .map_err(|error| OrgError::Invalid(format!("audit append became stale: {error}")))?;
        store.install_state(post);
        (audit_op, store.meta_doc())
    };
    push_op(ctx.out, audit_op);
    persist_meta(ctx.out, meta);
    Ok(())
}

/// Persist the docs each op touched — the op payload carries the new record for a
/// create/put; a revoke/remove deletes by key; a delegation-revoke keeps the
/// record (status flipped) and re-persists it from the post-image.
fn persist_ops(out: &mut Outbox, ops: &[OrgMutationOp], post: &crate::OrgState) {
    for op in ops {
        match op {
            OrgMutationOp::CreateOrg { org, root } => {
                persist_org(out, org);
                persist_scope(out, root);
            }
            OrgMutationOp::CreateScope { scope } => persist_scope(out, scope),
            // Retire flips status on the LIVE record, so the doc comes from the
            // post-image (the op carries only an id).
            OrgMutationOp::RetireScope { scope } => {
                if let Some(s) = post.scope(*scope) {
                    persist_scope(out, s);
                }
            }
            OrgMutationOp::CreateRole { role } => persist_role(out, role),
            OrgMutationOp::PutMembership { membership } => persist_membership(out, membership),
            OrgMutationOp::RemoveMembership { org, member } => persist_del(out, member_key(*org, *member)),
            OrgMutationOp::PutGrant { grant } => persist_grant(out, grant),
            OrgMutationOp::RevokeGrant { grant } => persist_del(out, grant_key(*grant)),
            OrgMutationOp::Delegate { delegation } => persist_delegation(out, delegation),
            OrgMutationOp::RevokeDelegation { delegation } => {
                if let Some(d) = post.delegation(*delegation) {
                    persist_delegation(out, d);
                }
            }
            OrgMutationOp::PutPolicy { policy } => persist_policy(out, policy),
        }
    }
}
