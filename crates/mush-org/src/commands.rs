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
use crate::ids::{CapabilityId, OrgId, OrgScopeId};
use crate::module::OrgStore;
use crate::plan::{OrgMutationOp, OrgMutationPlan};
use crate::record::{
    CapabilityGrant, GrantLimit, OrgScope, OrgStatus, Organization, RulesValue, ScopeSelector,
    ScopeStatus,
};

/// Register the `@org` command.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@org".into(),
            shape: ArgShape::EqPair,
            switches: ["create", "list", "show", "grant", "scope", "status", "audit"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@org/create <name>                 create an organization (+ its root scope)\n\
                   @org/list                          list organizations\n\
                   @org/show <id|name>                inspect one (scopes, roles, members, grants, policies)\n\
                   @org/scope create <org>=<name> [parent=<scope>]   add a compartment\n\
                   @org/scope retire <org>=<scope>    retire one (never a delete)\n\
                   @org/grant <obj>=<org> <cap> [scope=<id>] [limit=<n>]\n\
                   \x20                                 grant a capability; scope= binds ONE scope,\n\
                   \x20                                 limit= caps the per-operation amount\n\
                   @org/audit <id|name>               recent audit entries for an org\n\
                   A limit is a PER-OPERATION ceiling, never a budget: ten payments of 1000 all\n\
                   pass a limit of 10000. Spending accounting is the economy's (holds are E4)."
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
        if ctx
            .w
            .module_state::<OrgStore>()
            .is_some_and(|st| st.state().org(id).is_some())
        {
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
    } else if ctx.switches.has("scope") {
        org_scope(ctx)
    } else if ctx.switches.has("status") {
        org_status(ctx)
    } else if ctx.switches.has("audit") {
        org_audit(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @org/create|list|show|scope|grant|status|audit …",
        ))
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
    info(
        ctx,
        format!("Created org #{org_id} ({name}); root scope #{root_id}."),
    );
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
        let mut v = vec![format!(
            "#{} {} [{}] — root scope #{}",
            o.id,
            o.name,
            status_label(o.status),
            o.root_scope
        )];
        if !o.charter.trim().is_empty() {
            v.push(format!("  charter: {}", o.charter));
        }
        for s in st.scopes_in(org_id) {
            v.push(format!(
                "  scope #{} {} (depth {}, parent {}){}",
                s.id,
                s.name,
                s.depth,
                s.parent.map_or_else(|| "—".into(), |p| format!("#{p}")),
                if s.status == ScopeStatus::Retired {
                    " RETIRED"
                } else {
                    ""
                },
            ));
        }
        for r in st.roles_in(org_id) {
            v.push(format!(
                "  role #{} {} ({} grant template(s))",
                r.id,
                r.name,
                r.grants.len()
            ));
        }
        for m in st.members_in(org_id) {
            v.push(format!(
                "  member #{} ({} role(s), {:?})",
                m.member.0,
                m.roles.len(),
                m.status
            ));
        }
        for g in st.grants_in(org_id) {
            v.push(format!(
                "  grant #{} -> #{} {} on {:?}{}{}{}",
                g.id,
                g.grantee.0,
                g.capability.as_str(),
                g.scope,
                g.limit
                    .and_then(|l| l.max_amount)
                    .map_or_else(String::new, |m| format!(" limit {m}/op")),
                if g.delegable { " (delegable)" } else { "" },
                // §4.8.4: a stored grant whose capability has fallen outside the
                // wiring vocabulary authorizes nothing (equality match) — say so
                // rather than leaving staff to wonder why it never fires.
                if store.knows_capability(&g.capability) {
                    ""
                } else {
                    " [UNKNOWN CAPABILITY]"
                },
            ));
        }
        for d in st.delegations_in(org_id) {
            v.push(format!(
                "  delegation #{} #{}→#{} {} ({:?})",
                d.id,
                d.grantor.0,
                d.grantee.0,
                d.capability.as_str(),
                d.status
            ));
        }
        for p in st.policies_in(org_id) {
            v.push(format!(
                "  policy #{} ({} rule(s), default {:?})",
                p.id,
                p.rules.len(),
                p.default
            ));
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

/// The capabilities an [`AuthRequest`](crate::AuthRequest) carries an amount for:
/// the economy's posting, and (doc 17h §3 M8f P2a) the Roll's `roll.holder.org`,
/// whose buy steps that move money ask with their amount. A `GrantLimit` on anything else is inert-but-lethal: `limit_ok` answers
/// `AmountRequired` for every amount-free request, so the grant authorizes *nothing*,
/// forever (A13). `@org/grant` therefore refuses `limit=` outside this list rather
/// than minting a grant that silently never works.
const AMOUNT_CAPABILITIES: &[&str] = &["economy.account.post", "roll.holder.org"];

/// Pull `key=value` options out of a free-form tail, returning the remaining words
/// and the recognized options (lower-cased keys). An unrecognized `k=v` word stays
/// in the remainder rather than being silently eaten.
fn parse_opts(s: &str, keys: &[&str]) -> (String, std::collections::BTreeMap<String, String>) {
    let mut rest: Vec<&str> = Vec::new();
    let mut opts: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for tok in s.split_whitespace() {
        if let Some((k, v)) = tok.split_once('=') {
            let lk = k.to_ascii_lowercase();
            if keys.contains(&lk.as_str()) {
                opts.insert(lk, v.to_string());
                continue;
            }
        }
        rest.push(tok);
    }
    (rest.join(" "), opts)
}

fn parse_scope_id(s: &str) -> Option<OrgScopeId> {
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(OrgScopeId)
}

fn org_grant(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @org/grant <obj>=<org> <capability> [scope=<id>] [limit=<n>]";
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let (rest, opts) = parse_opts(rhs, &["scope", "limit"]);
    let mut it = rest.split_whitespace();
    let org_s = it.next().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let cap_s = it.next().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let org_id = resolve_org(ctx, &org_s).ok_or_else(|| CmdError::msg("No such org."))?;
    let cap = CapabilityId::new(&cap_s)
        .ok_or_else(|| CmdError::msg("Bad capability id (use [a-z0-9_.-])."))?;

    // §4.8.4: with a vocabulary configured, an unknown capability is a typo, not a
    // grant — refuse it loudly instead of storing something that authorizes nothing.
    {
        let store = ctx.w.module_state::<OrgStore>().expect("org store");
        if !store.knows_capability(&cap) {
            return Err(CmdError::msg(format!(
                "Unknown capability {}. This server recognizes: {}.",
                cap.as_str(),
                store
                    .capabilities()
                    .iter()
                    .map(CapabilityId::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }

    // A13: a limit on a capability whose requests never carry an amount denies
    // forever (`AmountRequired`). Refuse at the source.
    let limit = match opts.get("limit") {
        None => None,
        Some(s) => {
            if !AMOUNT_CAPABILITIES.contains(&cap.as_str()) {
                return Err(CmdError::msg(format!(
                    "limit= applies only to an amount-carrying capability ({}); a limit on {} \
                     would deny every request, because those requests carry no amount.",
                    AMOUNT_CAPABILITIES.join(", "),
                    cap.as_str()
                )));
            }
            let n: i64 = s
                .parse()
                .map_err(|_| CmdError::msg("limit= must be a whole number of minor units."))?;
            if n < 0 {
                return Err(CmdError::msg("limit= must not be negative."));
            }
            Some(GrantLimit {
                max_amount: Some(n),
            })
        }
    };

    // `scope=` narrows to exactly ONE scope (the per-resource policy anchor,
    // D-E3-5); the default stays the org's whole live scope tree.
    let selector = match opts.get("scope") {
        None => {
            let store = ctx.w.module_state::<OrgStore>().expect("org store");
            ScopeSelector::SubtreeLive(
                store
                    .state()
                    .org(org_id)
                    .expect("resolved org exists")
                    .root_scope,
            )
        }
        Some(s) => {
            let sid = parse_scope_id(s).ok_or_else(|| CmdError::msg("Bad scope id."))?;
            let store = ctx.w.module_state::<OrgStore>().expect("org store");
            let sc = store
                .state()
                .scope(sid)
                .ok_or_else(|| CmdError::msg("No such scope."))?;
            if sc.org != org_id {
                return Err(CmdError::msg("That scope belongs to another organization."));
            }
            ScopeSelector::Exact(sid)
        }
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
        scope: selector.clone(),
        limit,
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
            detail: serde_json::json!({
                "grantee": obj.0,
                "capability": cap.as_str(),
                "grant": grant_id.0,
                "scope": format!("{selector:?}"),
                "limit": limit.and_then(|l| l.max_amount),
            }),
        },
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    let who = display_name(ctx.w, obj);
    let where_ = match selector {
        ScopeSelector::Exact(s) => format!("scope #{s}"),
        _ => format!("org #{org_id}'s scope tree"),
    };
    let cap_str = cap.as_str().to_string();
    info(
        ctx,
        format!("Granted {cap_str} to {who} over {where_} (grant #{grant_id})."),
    );
    if let Some(max) = limit.and_then(|l| l.max_amount) {
        info(
            ctx,
            format!("  limit {max} per operation (a ceiling, not a budget)."),
        );
    }
    Ok(())
}

// ---- @org/scope -----------------------------------------------------------

/// `@org/scope create <org>=<name> [parent=<scope>]` / `@org/scope retire <org>=<scope>`.
///
/// The verb rides the LHS (`ArgShape::EqPair` splits at the first `=`, and both
/// forms need the org on the left of it).
fn org_scope(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @org/scope create <org>=<name> [parent=<scope>]   |   \
                         @org/scope retire <org>=<scope>";
    let lhs = ctx.args.lhs().trim().to_string();
    let (verb, org_s) = lhs
        .split_once(char::is_whitespace)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let org_id = resolve_org(ctx, org_s).ok_or_else(|| CmdError::msg("No such org."))?;
    match verb.to_ascii_lowercase().as_str() {
        "create" => scope_create(ctx, org_id),
        "retire" => scope_retire(ctx, org_id),
        _ => Err(CmdError::msg(USAGE)),
    }
}

fn scope_create(ctx: &mut Ctx<'_>, org_id: OrgId) -> Result<(), CmdError> {
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: @org/scope create <org>=<name> [parent=<scope>]"))?;
    let (rest, opts) = parse_opts(rhs, &["parent"]);
    let name = rest.trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg(
            "Usage: @org/scope create <org>=<name> [parent=<scope>]",
        ));
    }
    let (parent_id, depth, created_rev) = {
        let store = ctx.w.module_state::<OrgStore>().expect("org store");
        let st = store.state();
        let org = st.org(org_id).expect("resolved org exists");
        let parent_id = match opts.get("parent") {
            None => org.root_scope,
            Some(s) => parse_scope_id(s).ok_or_else(|| CmdError::msg("Bad parent scope id."))?,
        };
        let parent = st
            .scope(parent_id)
            .ok_or_else(|| CmdError::msg("No such parent scope."))?;
        if parent.org != org_id {
            return Err(CmdError::msg(
                "That parent scope belongs to another organization.",
            ));
        }
        if parent.status != ScopeStatus::Active {
            return Err(CmdError::msg("That parent scope is retired."));
        }
        // `created_rev` is what a `ScopeSelector::SubtreeFrozen { snapshot_revision }`
        // compares against, and the whole point of that variant is that it does NOT
        // widen to scopes created after its snapshot. A snapshot is taken as the org's
        // CURRENT revision, so a scope minted now must stamp **past** it — hence
        // `revision + 1`, not `revision`. (No mutation op moves `Organization.revision`
        // today, so stamping it verbatim would make every frozen subtree silently
        // swallow every scope this command ever creates.)
        (parent_id, parent.depth + 1, org.revision + 1)
    };
    let scope_id = {
        let store = ctx.w.module_state_mut::<OrgStore>().expect("org store");
        store.alloc_scope()
    };
    let scope = OrgScope {
        id: scope_id,
        org: org_id,
        parent: Some(parent_id),
        name: SmolStr::new(&name),
        status: ScopeStatus::Active,
        depth,
        created_rev,
        revision: 1,
    };
    let plan = OrgMutationPlan::of(vec![OrgMutationOp::CreateScope { scope }]);
    api::submit(
        ctx,
        &plan,
        OrgAction {
            action: SmolStr::new_static("scope.create"),
            org: Some(org_id),
            detail: serde_json::json!({ "scope": scope_id.0, "name": name, "parent": parent_id.0 }),
        },
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    info(
        ctx,
        format!("Created scope #{scope_id} ({name}) under #{parent_id} in org #{org_id}."),
    );
    Ok(())
}

fn scope_retire(ctx: &mut Ctx<'_>, org_id: OrgId) -> Result<(), CmdError> {
    let scope_id = ctx
        .args
        .rhs()
        .and_then(parse_scope_id)
        .ok_or_else(|| CmdError::msg("Usage: @org/scope retire <org>=<scope>"))?;
    {
        let store = ctx.w.module_state::<OrgStore>().expect("org store");
        let sc = store
            .state()
            .scope(scope_id)
            .ok_or_else(|| CmdError::msg("No such scope."))?;
        if sc.org != org_id {
            return Err(CmdError::msg("That scope belongs to another organization."));
        }
        if sc.parent.is_none() {
            return Err(CmdError::msg("A root scope cannot be retired."));
        }
    }
    // §4.8.2: `mush-org` cannot see an account/asset/vault, so the modules that BIND
    // scopes answer for themselves through the wiring-frozen binder table. Retiring
    // a bound compartment would strand the records pointing at it.
    let binders = ctx
        .w
        .module_state::<OrgStore>()
        .expect("org store")
        .scope_use();
    for b in binders.iter() {
        if let Some(reason) = b.scope_in_use(ctx.w, scope_id) {
            return Err(CmdError::msg(format!(
                "That scope is still in use by {}: {reason}",
                b.binder()
            )));
        }
    }
    let plan = OrgMutationPlan::of(vec![OrgMutationOp::RetireScope { scope: scope_id }]);
    api::submit(
        ctx,
        &plan,
        OrgAction {
            action: SmolStr::new_static("scope.retire"),
            org: Some(org_id),
            detail: serde_json::json!({ "scope": scope_id.0 }),
        },
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    info(
        ctx,
        format!("Retired scope #{scope_id}; it confers no authority and takes no new binding."),
    );
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
            v.push(format!(
                "No audit entries for org #{org_id} in the open ledger tail."
            ));
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

/// `@org/status <id|name>=<state> [hidden=yes|no]` (E6c B10, DL-06's minimal slice).
///
/// The first — and only — write path for `Organization.status` past `Active` and for
/// `Organization.hidden`, both of which have existed since E1a with no producer at
/// all. Deliberately narrow: it moves two fields and bumps the revision, and it
/// **never touches an account, an asset, a membership or a live contract**. A
/// suspended or dissolving organization stops being registrable as a trading vendor
/// (E6c B5/B10) and its live contracts settle through E4 exactly as before; the close
/// workflow that resolves what a `Dissolving` organization still holds is E7's.
fn org_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @org/status <id|name>=<active|suspended|dissolving|closed>                          [hidden=yes|no]";
    let Some(org) = resolve_org(ctx, ctx.args.lhs()) else {
        return Err(CmdError::msg("No such organization."));
    };
    let rhs = ctx.args.rhs().unwrap_or_default();
    let mut status: Option<OrgStatus> = None;
    let mut hidden: Option<bool> = None;
    for word in rhs.split_whitespace() {
        if let Some(v) = word.strip_prefix("hidden=") {
            hidden = match v.trim().to_ascii_lowercase().as_str() {
                "yes" | "true" | "on" | "1" => Some(true),
                "no" | "false" | "off" | "0" => Some(false),
                _ => return Err(CmdError::msg(USAGE)),
            };
        } else {
            status = match word.trim().to_ascii_lowercase().as_str() {
                "active" => Some(OrgStatus::Active),
                "suspended" => Some(OrgStatus::Suspended),
                "dissolving" => Some(OrgStatus::Dissolving),
                "closed" => Some(OrgStatus::Closed),
                _ => return Err(CmdError::msg(USAGE)),
            };
        }
    }
    if status.is_none() && hidden.is_none() {
        return Err(CmdError::msg(USAGE));
    }
    let plan = OrgMutationPlan::of(vec![OrgMutationOp::SetOrgStatus {
        org,
        status,
        hidden,
    }]);
    api::submit(
        ctx,
        &plan,
        OrgAction {
            action: SmolStr::new_static("org.status"),
            org: Some(org),
            detail: serde_json::json!({
                "status": status.map(status_label),
                "hidden": hidden,
            }),
        },
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    let said = match (status, hidden) {
        (Some(s), Some(h)) => format!("{} and {}", status_label(s), hidden_label(h)),
        (Some(s), None) => status_label(s).to_string(),
        (None, Some(h)) => hidden_label(h).to_string(),
        (None, None) => String::new(),
    };
    info(ctx, format!("Org #{org} is now {said}."));
    Ok(())
}

fn hidden_label(h: bool) -> &'static str {
    if h { "hidden" } else { "listed" }
}

fn status_label(s: OrgStatus) -> &'static str {
    match s {
        OrgStatus::Active => "active",
        OrgStatus::Suspended => "suspended",
        OrgStatus::Dissolving => "dissolving",
        OrgStatus::Closed => "closed",
    }
}
