//! The `@org` staff command surface (E1b): create an organization, list/show the
//! registry, grant a capability, and read the audit trail. Richer administration
//! (scopes, roles, memberships, policies, delegations) is driven programmatically
//! through [`crate::submit`] by the modules that own those flows (E1c faction, E2
//! economy); the fuller admin command language is a later cut.
//!
//! Every subcommand is staff-only. Org creation is deliberately staff-gated in E1
//! (the reviewed decision): players do not spin up first-class owning entities yet.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ObjId, Powers, Role, World, match_object, perm,
};
use smol_str::SmolStr;

use crate::api::{self, OrgAction, OrgAuditEntry};
use crate::ids::{CapabilityId, OrgId};
use crate::module::OrgStore;
use crate::plan::{OrgMutationOp, OrgMutationPlan};
use crate::record::{
    CapabilityGrant, OrgScope, OrgStatus, Organization, RulesValue, ScopeSelector, ScopeStatus,
};

/// Register the `@org` command.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@org".into(),
            shape: ArgShape::EqPair,
            switches: ["create", "list", "show", "grant", "audit"].iter().copied().map(SmolStr::new).collect(),
            help: "@org/create <name>                 create an organization (+ its root scope)\n\
                   @org/list                          list organizations\n\
                   @org/show <id|name>                inspect one (scopes, roles, members, grants, policies)\n\
                   @org/grant <obj>=<orgid> <cap>     grant a capability over the org's scope tree\n\
                   @org/audit <id|name>               recent audit entries for an org"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_org),
    )?;
    Ok(())
}

// ---- gating + small helpers ----------------------------------------------

fn is_admin(w: &World, who: ObjId) -> bool {
    perm::has_power(w, who, Powers::SIM_ADMIN) || perm::has_role(w, who, Role::Royalty)
}
fn require_admin(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_admin(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("Permission denied."))
    }
}
fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "org", msg.into());
}

/// Resolve an org by `#id` / `id` or by (case-insensitive) name/alias.
fn resolve_org(ctx: &Ctx<'_>, s: &str) -> Option<OrgId> {
    let s = s.trim();
    if let Ok(n) = s.trim_start_matches('#').parse::<u64>() {
        let id = OrgId(n);
        if ctx.w.module_state::<OrgStore>().is_some_and(|st| st.state().org(id).is_some()) {
            return Some(id);
        }
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let store = ctx.w.module_state::<OrgStore>()?;
    store
        .state()
        .orgs()
        .find(|o| {
            o.name.to_ascii_lowercase() == lower
                || o.aliases.iter().any(|a| a.to_ascii_lowercase() == lower)
        })
        .map(|o| o.id)
}

// ---- dispatch -------------------------------------------------------------

fn cmd_org(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    if ctx.switches.has("create") {
        org_create(ctx)
    } else if ctx.switches.has("list") {
        org_list(ctx)
    } else if ctx.switches.has("show") {
        org_show(ctx)
    } else if ctx.switches.has("grant") {
        org_grant(ctx)
    } else if ctx.switches.has("audit") {
        org_audit(ctx)
    } else {
        Err(CmdError::msg("Usage: @org/create|list|show|grant|audit …"))
    }
}

fn org_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let name = ctx.args.lhs().trim();
    if name.is_empty() {
        return Err(CmdError::msg("Usage: @org/create <name>"));
    }
    let name = name.to_string();
    // Allocate the org + its root scope from the slot's monotonic allocators.
    let (org_id, root_id) = {
        let store = ctx.w.module_state_mut::<OrgStore>().expect("org store");
        (store.alloc_org(), store.alloc_scope())
    };
    let org = Organization {
        id: org_id,
        name: name.clone().into(),
        aliases: Vec::new(),
        status: OrgStatus::Active,
        hidden: false,
        charter: String::new(),
        root_scope: root_id,
        revision: 1,
        ruleset_ext: RulesValue::default(),
    };
    let root = OrgScope {
        id: root_id,
        org: org_id,
        parent: None,
        name: "root".into(),
        status: ScopeStatus::Active,
        depth: 0,
        created_rev: 1,
        revision: 1,
    };
    let plan = OrgMutationPlan::of(vec![OrgMutationOp::CreateOrg { org, root }]);
    api::submit(
        ctx,
        &plan,
        OrgAction {
            action: SmolStr::new_static("org.create"),
            org: Some(org_id),
            detail: serde_json::json!({ "name": name }),
        },
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    info(ctx, format!("Created org #{org_id} ({name}); root scope #{root_id}."));
    Ok(())
}

fn org_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<OrgStore>().expect("org store");
        let st = store.state();
        let mut v: Vec<String> = st
            .orgs()
            .map(|o| {
                let scopes = st.scopes_in(o.id).count();
                let members = st.members_in(o.id).count();
                format!(
                    "#{} {} [{}]{} — {scopes} scope(s), {members} member(s)",
                    o.id,
                    o.name,
                    status_label(o.status),
                    if o.hidden { " (hidden)" } else { "" },
                )
            })
            .collect();
        if v.is_empty() {
            v.push("No organizations.".into());
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn org_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let org_id = resolve_org(ctx, ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such org."))?;
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<OrgStore>().expect("org store");
        let st = store.state();
        let o = st.org(org_id).expect("resolved org exists");
        let mut v = vec![format!("#{} {} [{}] — root scope #{}", o.id, o.name, status_label(o.status), o.root_scope)];
        if !o.charter.trim().is_empty() {
            v.push(format!("  charter: {}", o.charter));
        }
        for s in st.scopes_in(org_id) {
            v.push(format!(
                "  scope #{} {} (depth {}, parent {})",
                s.id,
                s.name,
                s.depth,
                s.parent.map_or_else(|| "—".into(), |p| format!("#{p}")),
            ));
        }
        for r in st.roles_in(org_id) {
            v.push(format!("  role #{} {} ({} grant template(s))", r.id, r.name, r.grants.len()));
        }
        for m in st.members_in(org_id) {
            v.push(format!("  member #{} ({} role(s), {:?})", m.member.0, m.roles.len(), m.status));
        }
        for g in st.grants_in(org_id) {
            v.push(format!(
                "  grant #{} -> #{} {} on {:?}{}",
                g.id,
                g.grantee.0,
                g.capability.as_str(),
                g.scope,
                if g.delegable { " (delegable)" } else { "" },
            ));
        }
        for d in st.delegations_in(org_id) {
            v.push(format!("  delegation #{} #{}→#{} {} ({:?})", d.id, d.grantor.0, d.grantee.0, d.capability.as_str(), d.status));
        }
        for p in st.policies_in(org_id) {
            v.push(format!("  policy #{} ({} rule(s), default {:?})", p.id, p.rules.len(), p.default));
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn org_grant(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: @org/grant <obj>=<orgid> <capability>"))?;
    let mut it = rhs.split_whitespace();
    let org_s = it.next().ok_or_else(|| CmdError::msg("Usage: @org/grant <obj>=<orgid> <capability>"))?;
    let cap_s = it.next().ok_or_else(|| CmdError::msg("Usage: @org/grant <obj>=<orgid> <capability>"))?;
    let org_id = resolve_org(ctx, org_s).ok_or_else(|| CmdError::msg("No such org."))?;
    let cap = CapabilityId::new(cap_s).ok_or_else(|| CmdError::msg("Bad capability id (use [a-z0-9_.-])."))?;

    let root = {
        let store = ctx.w.module_state::<OrgStore>().expect("org store");
        store.state().org(org_id).expect("resolved org exists").root_scope
    };
    let grant_id = {
        let store = ctx.w.module_state_mut::<OrgStore>().expect("org store");
        store.alloc_grant()
    };
    let grant = CapabilityGrant {
        id: grant_id,
        org: org_id,
        grantee: obj,
        capability: cap.clone(),
        scope: ScopeSelector::SubtreeLive(root),
        limit: None,
        delegable: false,
        valid_from: None,
        valid_until: None,
        revision: 1,
    };
    let plan = OrgMutationPlan::of(vec![OrgMutationOp::PutGrant { grant }]);
    api::submit(
        ctx,
        &plan,
        OrgAction {
            action: SmolStr::new_static("grant.create"),
            org: Some(org_id),
            detail: serde_json::json!({ "grantee": obj.0, "capability": cap.as_str(), "grant": grant_id.0 }),
        },
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    let who = display_name(ctx.w, obj);
    info(ctx, format!("Granted {} to {who} over org #{org_id}'s scope tree (grant #{grant_id}).", cap.as_str()));
    Ok(())
}

fn org_audit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let org_id = resolve_org(ctx, ctx.args.lhs()).ok_or_else(|| CmdError::msg("No such org."))?;
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<OrgStore>().expect("org store");
        let mut v: Vec<String> = store
            .audit
            .open_entries(api::AUDIT_LOG)
            .iter()
            .filter_map(|e| serde_json::from_value::<OrgAuditEntry>(e.value.clone()).ok())
            .filter(|e| e.org == Some(org_id))
            .rev()
            .take(20)
            .map(|e| format!("  [{}] #{} {} {}", e.at, e.actor.0, e.action, e.detail))
            .collect();
        v.reverse();
        if v.is_empty() {
            v.push(format!("No audit entries for org #{org_id} in the open ledger tail."));
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn status_label(s: OrgStatus) -> &'static str {
    match s {
        OrgStatus::Active => "active",
        OrgStatus::Suspended => "suspended",
        OrgStatus::Dissolving => "dissolving",
        OrgStatus::Closed => "closed",
    }
}
