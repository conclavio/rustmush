//! The housing command surface (E5b, 08d §4's command list).
//!
//! ## Who may do what
//!
//! | verb | who |
//! |---|---|
//! | `@property/create|rooms|tier|feature|withdraw` | staff |
//! | `@property/show|list` | staff, a landlord officer holding `property.view` / `property.manage` on the compartment (or the landlord org's root scope), a tenant, or a guest (**reduced**) |
//! | `@tenancy/create|resolve` | **staff only** (08d §11 answer 3, binding: `property.manage` covers show/list and guest policy, never tenancy creation) |
//! | `@tenancy/show|list` | staff, a landlord officer in their own compartment, a tenant, or a guest (**reduced**) |
//! | `+home`, `+home/leave`, `+guest` | the tenant; `+home` also answers a guest, with their own standing and nothing else |
//!
//! ## Existence hiding (§4.5, D-E3-6, applied to homes)
//!
//! A property or tenancy the actor may not **view** renders as `No such property.` /
//! `No such tenancy.` — the same text a nonexistent one gets. `mush_economy`'s
//! `NamedResource` renderer is the model but its helpers are crate-private, so the
//! discipline is reimplemented here rather than approximated: every refusal that
//! touches a record goes through [`hidden`] first, so "denied" and "not there" are one
//! message and a probe cannot use the refusal to enumerate who lives where.
//!
//! Housing has a second reason to care that the economy does not. An account id is a
//! number; a residence is a **room somebody can stand in**, so the interesting probe
//! is not `@property/show 7` but the door itself — which is why the hook's refusal
//! text ([`crate::hooks::REFUSED`]) is the ordinary "You can't go that way." and says
//! nothing about tenancy at all.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, World, match_object};
use mush_economy::{Amount, CadenceId, ObligationTemplate};
use mush_org::{AuthRequest, CapabilityId, OrgScopeId, PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;

use crate::ids::{PropertyId, TenancyId};
use crate::module::{
    HousingStore, RoomEntry, is_character, is_live_room, is_staff, put_property, put_tenancy,
    render_principal,
};
use crate::quarantine::{QuarantineKey, under_review};
use crate::record::{
    AccessTerms, Funding, InspectionTerm, InspectionWhen, MAX_ROOMS_PER_PROPERTY, MAX_TENANTS,
    Property, PropertyStatus, PropertyTier, Resolution, StoryWindow, Tenancy, TenancyStatus,
};

/// Every refusal a property the actor may not see produces.
pub const NO_SUCH_PROPERTY: &str = "No such property.";
/// Every refusal a tenancy the actor may not see produces.
pub const NO_SUCH_TENANCY: &str = "No such tenancy.";
/// The row cap every `*/list` obeys (the economy's `LIST_CAP`).
pub const LIST_CAP: usize = 20;

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@property".into(),
            shape: ArgShape::EqPair,
            switches: ["create", "rooms", "tier", "feature", "site", "withdraw", "show", "list"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@property/create <name>=<anchor room>[,<room>…] tier=<0-5> \
                   landlord=<org:<id|name>|housing>\n\
                   \x20   [scope=<org scope id>]                              (staff)\n\
                   \x20                                      Mints the ANCHOR's title record to the\n\
                   \x20                                      landlord. The rooms' @owner stays the\n\
                   \x20                                      builder — title is not ownership of the\n\
                   \x20                                      object, and @chown needs a player.\n\
                   @property/rooms <id>=<room>[,…]         add rooms (staff)\n\
                   @property/rooms <id>=-<room>[,…]        remove them (never the anchor, and never\n\
                   \x20                                      while the property is LET)\n\
                   @property/tier <id>=<0-5>               the residence's physical band (staff)\n\
                   @property/feature <id>=<key>            record an approved feature (staff)\n\
                   @property/site <id>=<site>              link the atlas site it stands on (staff);\n\
                   \x20                                      refused naming any room outside the\n\
                   \x20                                      site's footprint; empty clears\n\
                   @property/withdraw <id>                 take it out of service (staff)\n\
                   @property/show <id>   @property/list [landlord=<org>]\n\
                   \x20  Who may read them: staff; an officer holding property.view or\n\
                   \x20  property.manage on the landlord's compartment (or, when the property\n\
                   \x20  named no compartment, the landlord org's root scope); a tenant; and a\n\
                   \x20  GUEST, who sees the property's name and their own standing and nothing\n\
                   \x20  else — never the landlord, tier, rooms, features, status or occupancy.\n\
                   \x20  A property you may not view answers \"No such property.\", and so does an\n\
                   \x20  unresolvable landlord= filter.\n\
                   A property is ORDINARY PRIVATE ROOMS a builder already dug (D-E5-12): never a\n\
                   sim space, a domain presentation room, the chargen start room, a room another\n\
                   property already holds, or a thoroughfare — a room reachable from outside the\n\
                   property by any exit other than into the anchor. A tenancy grants no @dig."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_property),
    )?;
    reg.command(
        CommandSpec {
            name: "@tenancy".into(),
            shape: ArgShape::EqPair,
            switches: ["create", "resolve", "show", "list"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@tenancy/create <property>=<tenant>[,<tenant>…] cadence=<id>\n\
                   \x20   [inspect=<who>:<always|delinquent>]\n\
                   \x20   [mode=abstract|exact rent=<minor units> from=<acct> to=<acct>\n\
                   \x20    every=<seconds>]                                              (staff)\n\
                   \x20                                      every= is the rent period (default one\n\
                   \x20                                        real day). from= must be an account a\n\
                   \x20                                        TENANT owns — never a bystander's and\n\
                   \x20                                        never a system faucet.\n\
                   \x20   Creating and resolving a tenancy is STAFF, even for a landlord's own\n\
                   \x20   officers: property.manage covers show/list and guest policy and never\n\
                   \x20   tenancy creation (§11 answer 3, the default the owner accepted).\n\
                   \x20                                      abstract: live immediately; the ruleset\n\
                   \x20                                        submits support plans and NOTHING posts.\n\
                   \x20                                      exact: offers a LEASE CONTRACT instead. The\n\
                   \x20                                        tenant's own @contract/accept is the\n\
                   \x20                                        consent; activation creates the rent\n\
                   \x20                                        obligation and the tenancy goes active.\n\
                   @tenancy/resolve <id>=extend|scene|end|waive [reason]        (staff)\n\
                   \x20  extend  leave it open and give the story window a fresh notice budget\n\
                   \x20  scene   start a scene on it and record it on the window\n\
                   \x20  end     end it: every homed tenant's home anchor is cleared\n\
                   \x20  waive   end it as forgiven — no fault recorded against anyone\n\
                   @tenancy/show <id>   @tenancy/list [property=<id>]\n\
                   \x20  Who may read them: staff; a landlord officer in their own compartment;\n\
                   \x20  a tenant; and a GUEST, who sees the property's name and their own\n\
                   \x20  standing and nothing else. Staff alone see the numeric support level\n\
                   \x20  and the plan count — everyone else reads the assessment word.\n\
                   Nothing ends a home automatically (D-E5-1). Automatic assessment stops at\n\
                   eviction-pending with a bounded story window: the tenant is notified at most 3\n\
                   times, then staff are paged, and the tenant keeps their key throughout."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_tenancy),
    )?;
    reg.command(
        CommandSpec {
            name: "+home".into(),
            shape: ArgShape::EqPair,
            switches: ["leave"].iter().copied().map(SmolStr::new).collect(),
            help: "+home                                   where you live, how it stands, who you\n\
                   \x20                                      have let in, and any notice waiting for you\n\
                   \x20                                      As a GUEST it shows the property's name and\n\
                   \x20                                      your own standing — never who lives there.\n\
                   +home/leave [<tenancy>]                 move out, deliberately and for good\n\
                   \x20                                      With more than one home it REFUSES and lists\n\
                   \x20                                      them: it never guesses which one to end.\n\
                   \x20                                      Your home anchor is cleared, the access hook\n\
                   \x20                                      stops admitting you, and it does not come\n\
                   \x20                                      back. This is the tenant's half of \"a home\n\
                   \x20                                      only ends by an explicit act\"."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_home),
    )?;
    reg.command(
        CommandSpec {
            name: "+guest".into(),
            shape: ArgShape::EqPair,
            switches: ["add", "remove"].iter().copied().map(SmolStr::new).collect(),
            help: "+guest                                  who you have let into your home\n\
                   +guest add <who>                        let somebody in (they may enter and carry)\n\
                   +guest remove <who>                     stop letting them in\n\
                   \x20  home=<tenancy> names which home, and is required when you have more\n\
                   \x20  than one.\n\
                   Guests are the TENANT's to manage, at any status: a delinquent tenant still\n\
                   invites whoever they like. A guest is not homed on the property — an invitation\n\
                   is not a residence."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_guest),
    )?;
    reg.command(
        CommandSpec {
            name: "@housing".into(),
            shape: ArgShape::EqPair,
            switches: ["quarantine"].iter().copied().map(SmolStr::new).collect(),
            help: "@housing/quarantine show                the fail-closed quarantine (staff)\n\
                   @housing/quarantine recheck <key>       re-run the check for one conflict\n\
                   \x20  A conflict between records — one room in two properties, two live\n\
                   \x20  tenancies on one property, a live tenancy on no property — is found at\n\
                   \x20  boot and after every write. Every room of every property involved\n\
                   \x20  refuses everyone but staff, the tick skips the tenancies, their rent is\n\
                   \x20  suspended, and one alert per conflict carries the repair. There is NO\n\
                   \x20  release verb: the quarantine lifts by itself the moment the repair holds\n\
                   \x20  (@property/rooms <id>=-<room>, @tenancy/resolve <id>=end,\n\
                   \x20  @property/withdraw <id>); recheck only re-runs the check after a repair\n\
                   \x20  made some other way, and refuses while the conflict still exists."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_housing),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// @housing/quarantine (look-back fold, D-LBK-1)
// ---------------------------------------------------------------------------

fn cmd_housing(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @housing/quarantine show  |  @housing/quarantine recheck <key>";
    require_staff(ctx)?;
    if !ctx.switches.has("quarantine") {
        return Err(CmdError::msg(USAGE));
    }
    let raw = ctx.args.raw().trim().to_string();
    let (verb, arg) = match raw.split_once(char::is_whitespace) {
        Some((v, r)) => (v.to_ascii_lowercase(), r.trim().to_string()),
        None => (raw.to_ascii_lowercase(), String::new()),
    };
    match verb.as_str() {
        "show" => {
            let rows: Vec<String> = ctx
                .w
                .module_state::<HousingStore>()
                .map(|s| s.quarantine().map(render_conflict).collect())
                .unwrap_or_default();
            if rows.is_empty() {
                info(ctx, "Nothing is under housing review.".to_string());
            }
            for row in rows {
                info(ctx, row);
            }
            Ok(())
        }
        "recheck" => {
            let key = QuarantineKey::parse(&arg).ok_or_else(|| {
                CmdError::msg("recheck takes a key as @housing/quarantine show prints it.")
            })?;
            let known = ctx
                .w
                .module_state::<HousingStore>()
                .is_some_and(|s| s.conflict(key).is_some());
            if !known {
                return Err(CmdError::msg(format!("Nothing is under review as {key}.")));
            }
            // The same check the write path runs; nothing here can release a conflict
            // that still exists.
            crate::quarantine::revalidate(ctx);
            match ctx
                .w
                .module_state::<HousingStore>()
                .and_then(|s| s.conflict(key).cloned())
            {
                Some(c) => Err(CmdError::msg(format!(
                    "{key} still exists: {}. Repair it — {} — and it lifts by itself.",
                    c.summary,
                    c.repair.join(" · ")
                ))),
                None => {
                    info(
                        ctx,
                        format!("{key}: the invariant holds again; the quarantine is lifted."),
                    );
                    ctx.gate_on_durable_ack();
                    Ok(())
                }
            }
        }
        _ => Err(CmdError::msg(USAGE)),
    }
}

fn render_conflict(c: &crate::quarantine::Conflict) -> String {
    let props: Vec<String> = c.properties.iter().map(|p| format!("#{p}")).collect();
    let tenancies: Vec<String> = c.tenancies.iter().map(|t| format!("#{t}")).collect();
    format!(
        "{} — {} [properties: {}; tenancies: {}] repair: {}",
        c.key,
        c.summary,
        if props.is_empty() {
            "none".to_string()
        } else {
            props.join(", ")
        },
        if tenancies.is_empty() {
            "none".to_string()
        } else {
            tenancies.join(", ")
        },
        c.repair.join(" · ")
    )
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "housing", msg.into());
}

fn require_staff(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_staff(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("You can't do that."))
    }
}

/// Whether `actor` holds `capability` on `scope`, live.
#[must_use]
pub fn holds(w: &World, actor: ObjId, capability: &str, scope: OrgScopeId) -> bool {
    let Some(capability) = CapabilityId::new(capability) else {
        return false;
    };
    let req = AuthRequest {
        actor,
        capability,
        scope,
        now: w.now(),
        amount: None,
    };
    mush_org::authorize_in_world(w, &req).is_allow()
}

/// The compartment an organization principal's authority is asked on for a housing
/// record: the hinted one when it really belongs to that org, else the org's **root
/// scope**.
///
/// One helper because there was one bug (verify fold, A4). `may_view_property` asked
/// only about `Property::landlord_scope` and the access hook asked about the root
/// scope as a fallback, so a property created without `scope=` — which the command
/// allows, and which is the obvious thing to type — locked every one of the
/// landlord's officers out of the record while still letting them inspect it. Two
/// answers to "is this person an officer here" is one too many.
#[must_use]
pub(crate) fn officer_scope(
    w: &World,
    principal: &PrincipalId,
    hint: Option<OrgScopeId>,
) -> Option<OrgScopeId> {
    match principal {
        PrincipalId::Organization(org) => {
            let store = w.module_state::<mush_org::OrgStore>()?;
            let state = store.state();
            hint.filter(|s| state.scope(*s).is_some_and(|sc| sc.org == *org))
                .or_else(|| state.org(*org).map(|o| o.root_scope))
        }
        // A property let by a person or a system principal has no org to fall back
        // to; it can still name a compartment its officers are read in.
        PrincipalId::Object(_) | PrincipalId::System(_) => hint,
    }
}

/// Whether `actor` is one of this property's **landlord officers** — the single
/// authority path (fold A4/A5).
///
/// Either capability is standing. §11 answer 3 makes `property.manage` the wider
/// right (it covers show/list *and* guest policy), so a manage-only officer who could
/// not read the property would hold a right they had no way to exercise — which is
/// exactly how an invitation used to demote one to "just a guest".
#[must_use]
pub fn officer_of(w: &World, actor: ObjId, p: &Property) -> bool {
    let Some(scope) = officer_scope(w, &p.landlord, p.landlord_scope) else {
        return false;
    };
    holds(w, actor, "property.view", scope) || holds(w, actor, "property.manage", scope)
}

/// §4.5 view standing over a property: staff, a landlord officer on its compartment,
/// or somebody who lives (or is invited) there.
#[must_use]
pub fn may_view_property(w: &World, actor: ObjId, p: &Property) -> bool {
    if is_staff(w, actor) {
        return true;
    }
    if officer_of(w, actor, p) {
        return true;
    }
    // A landlord who is a person sees their own.
    if p.landlord == PrincipalId::Object(actor) {
        return true;
    }
    w.module_state::<HousingStore>().is_some_and(|s| {
        s.tenancies()
            .filter(|t| t.property == p.id && t.status.is_live())
            .any(|t| t.is_tenant(actor) || t.is_guest(actor))
    })
}

/// Whether this actor's **only** standing on `p` is "somebody let me in" (fold A1).
///
/// A guest was already reduced on `+home` and `@tenancy/show`, and read the whole
/// record — landlord, tier, every room, every approved feature, the status and how
/// many people live there — off `@property/show`, `@property/list` and
/// `@tenancy/list`. An invitation is not a tenancy and it is certainly not a
/// landlord's books: a guest gets the name of the place and their own standing.
#[must_use]
fn guest_only_property(w: &World, actor: ObjId, p: &Property) -> bool {
    if is_staff(w, actor) || officer_of(w, actor, p) || p.landlord == PrincipalId::Object(actor) {
        return false;
    }
    w.module_state::<HousingStore>().is_some_and(|s| {
        let mut guest = false;
        for t in s
            .tenancies()
            .filter(|t| t.property == p.id && t.status.is_live())
        {
            if t.is_tenant(actor) {
                return false;
            }
            guest = guest || t.is_guest(actor);
        }
        guest
    })
}

/// The one line a guest reads about a place they were let into.
fn guest_line(name: &str) -> String {
    format!("{name}: you are a guest here.")
}

/// §4.5 view standing over a tenancy.
#[must_use]
pub fn may_view_tenancy(w: &World, actor: ObjId, t: &Tenancy) -> bool {
    if is_staff(w, actor) || t.is_tenant(actor) || t.is_guest(actor) {
        return true;
    }
    w.module_state::<HousingStore>()
        .and_then(|s| s.property(t.property).cloned())
        .is_some_and(|p| may_view_property(w, actor, &p))
}

/// Resolve a property the actor may view, or the one refusal text.
fn property_of(ctx: &Ctx<'_>, s: &str) -> Result<Property, CmdError> {
    let id = parse_id(s).map(PropertyId).ok_or_else(hidden_property)?;
    let p = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|st| st.property(id).cloned())
        .ok_or_else(hidden_property)?;
    if may_view_property(ctx.w, ctx.actor, &p) {
        Ok(p)
    } else {
        Err(hidden_property())
    }
}

/// Resolve a tenancy the actor may view, or the one refusal text.
fn tenancy_of(ctx: &Ctx<'_>, s: &str) -> Result<Tenancy, CmdError> {
    let id = parse_id(s).map(TenancyId).ok_or_else(hidden_tenancy)?;
    let t = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|st| st.tenancy(id).cloned())
        .ok_or_else(hidden_tenancy)?;
    if may_view_tenancy(ctx.w, ctx.actor, &t) {
        Ok(t)
    } else {
        Err(hidden_tenancy())
    }
}

fn hidden_property() -> CmdError {
    CmdError::msg(NO_SUCH_PROPERTY)
}
fn hidden_tenancy() -> CmdError {
    CmdError::msg(NO_SUCH_TENANCY)
}

/// **The existence-hiding rule, once.** Any refusal that names a record the actor may
/// not view renders as that record's "No such …" text; only somebody with standing
/// sees the real reason.
fn hidden(ctx: &Ctx<'_>, p: Option<&Property>, t: Option<&Tenancy>, detail: String) -> CmdError {
    if let Some(p) = p
        && !may_view_property(ctx.w, ctx.actor, p)
    {
        return hidden_property();
    }
    if let Some(t) = t
        && !may_view_tenancy(ctx.w, ctx.actor, t)
    {
        return hidden_tenancy();
    }
    CmdError::msg(detail)
}

fn parse_id(s: &str) -> Option<u64> {
    s.trim().trim_start_matches('#').parse::<u64>().ok()
}

/// Pull `key=value` options out of a free-form tail (the economy's `parse_opts`,
/// reimplemented because that one is crate-private).
fn parse_opts(s: &str, keys: &[&str]) -> (String, BTreeMap<String, String>) {
    let mut rest: Vec<&str> = Vec::new();
    let mut opts: BTreeMap<String, String> = BTreeMap::new();
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

fn emit_list(ctx: &mut Ctx<'_>, mut rows: Vec<String>, empty: &str) {
    if rows.is_empty() {
        info(ctx, empty.to_string());
        return;
    }
    let extra = rows.len().saturating_sub(LIST_CAP);
    rows.truncate(LIST_CAP);
    for r in rows {
        info(ctx, r);
    }
    if extra > 0 {
        info(ctx, format!("({extra} more)"));
    }
}

/// Resolve a room by `#id` or by name, refusing anything that is not a live room.
fn room_arg(ctx: &Ctx<'_>, s: &str) -> Result<ObjId, CmdError> {
    let id = match parse_id(s) {
        Some(n) => ObjId(u32::try_from(n).map_err(|_| CmdError::msg("No such room."))?),
        None => match_object(ctx.w, ctx.actor, s).map_err(|_| CmdError::msg("No such room."))?,
    };
    if is_live_room(ctx.w, id) {
        Ok(id)
    } else {
        Err(CmdError::msg("No such room."))
    }
}

/// `<principal>` = `#obj` / a matchable name | `org:<id|name>` | `sys:<name>` |
/// `housing`.
///
/// **`housing` resolves to this server's own wired system principal**, never to a
/// literal `wod.housing`: D-E5-5 makes this crate game-neutral, and a hard-coded
/// World-of-Darkness name in it would be the one place that stopped being true. The
/// principal the host registered as the title issuer *is* the generic system landlord
/// (owner answer 5's "plus a generic `system:wod.housing` principal"), so naming it
/// here keeps the two the same by construction rather than by convention.
fn parse_landlord(ctx: &Ctx<'_>, s: &str) -> Option<PrincipalId> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("housing") {
        return ctx
            .w
            .module_state::<HousingStore>()
            .and_then(|st| st.issuer().cloned())
            .map(PrincipalId::System);
    }
    if let Some(rest) = s.strip_prefix("sys:") {
        return SystemPrincipalId::new(rest).map(PrincipalId::System);
    }
    if let Some(rest) = s.strip_prefix("org:") {
        let store = ctx.w.module_state::<mush_org::OrgStore>()?;
        let state = store.state();
        let rest = rest.trim();
        if let Ok(n) = rest.trim_start_matches('#').parse::<u64>() {
            let id = mush_org::OrgId(n);
            return state.org(id).map(|_| PrincipalId::Organization(id));
        }
        return state
            .orgs()
            .find(|o| o.name.eq_ignore_ascii_case(rest))
            .map(|o| PrincipalId::Organization(o.id));
    }
    match_object(ctx.w, ctx.actor, s)
        .ok()
        .map(PrincipalId::Object)
}

// ---------------------------------------------------------------------------
// @property
// ---------------------------------------------------------------------------

fn cmd_property(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let sw = &ctx.switches;
    if sw.has("show") {
        return property_show(ctx);
    }
    if sw.has("list") {
        return property_list(ctx);
    }
    require_staff(ctx)?;
    if sw.has("create") {
        property_create(ctx)
    } else if sw.has("rooms") {
        property_rooms(ctx)
    } else if sw.has("tier") {
        property_tier(ctx)
    } else if sw.has("feature") {
        property_feature(ctx)
    } else if sw.has("site") {
        property_site(ctx)
    } else if sw.has("withdraw") {
        property_withdraw(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @property/create|rooms|tier|feature|site|withdraw|show|list",
        ))
    }
}

/// `@property/create <name>=<anchor>[,rooms…] tier=<n> landlord=<…> [scope=]`.
///
/// The **title** is minted here, through [`mush_economy::issue_title`] with the
/// module's wired issuer (08d §4, §2, D-E3-13): a residence the economy does not know
/// exists is a residence nothing can lease, encumber or transfer. `Obj.owner` of the
/// rooms is left alone — title is not object ownership, and `@chown` requires a
/// player, so pointing a room's owner at an organization is not even expressible.
fn property_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @property/create <name>=<anchor room>[,<room>…] tier=<0-5> \
                         landlord=<org:<id|name>|housing> [scope=<id>]";
    let name = ctx.args.lhs().trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let (rest, opts) = parse_opts(&rhs, &["tier", "landlord", "scope"]);
    let names: Vec<&str> = rest
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let Some(anchor_name) = names.first() else {
        return Err(CmdError::msg(USAGE));
    };
    let anchor = room_arg(ctx, anchor_name)?;
    let tier = PropertyTier::new(
        opts.get("tier")
            .and_then(|s| s.parse::<u8>().ok())
            .ok_or_else(|| CmdError::msg("tier= is 0 to 5."))?,
    );
    let landlord = opts
        .get("landlord")
        .and_then(|s| parse_landlord(ctx, s))
        .ok_or_else(|| CmdError::msg("No such landlord."))?;
    let scope = match opts.get("scope") {
        None => None,
        Some(s) => Some(
            parse_id(s)
                .map(OrgScopeId)
                .filter(|id| {
                    ctx.w
                        .module_state::<mush_org::OrgStore>()
                        .is_some_and(|st| st.state().scope(*id).is_some())
                })
                .ok_or_else(|| CmdError::msg("No such scope."))?,
        ),
    };
    let mut rooms: BTreeSet<ObjId> = BTreeSet::new();
    rooms.insert(anchor);
    for n in names.iter().skip(1) {
        rooms.insert(room_arg(ctx, n)?);
    }
    if rooms.len() > MAX_ROOMS_PER_PROPERTY {
        return Err(CmdError::msg(format!(
            "A property holds at most {MAX_ROOMS_PER_PROPERTY} rooms."
        )));
    }
    // Every D-E5-12 rule, before anything is allocated.
    for r in &rooms {
        check_room(ctx, *r, None, anchor, &rooms)?;
    }
    let issuer = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|s| s.issuer().cloned())
        .ok_or_else(|| {
            CmdError::msg(
                "This server wired no housing title issuer, so a property cannot be titled. \
                 (HousingModule::with_issuer + EconomyModule::with_title_issuers.)",
            )
        })?;
    let id = {
        let store = ctx
            .w
            .module_state_mut::<HousingStore>()
            .ok_or_else(|| CmdError::msg("No housing."))?;
        store
            .alloc_property()
            .ok_or_else(|| CmdError::msg("The property-id allocator is exhausted."))?
    };
    let mut p = Property::new(id, &name, anchor, rooms, tier, landlord.clone())
        .map_err(|e| CmdError::msg(e.to_string()))?;
    p.landlord_scope = scope;
    // **Title first**: it is the fallible half, and the module's rule is the
    // economy's — do the thing that can be refused before the thing that cannot.
    match mush_economy::issue_title(ctx, anchor, landlord.clone(), scope, issuer, None) {
        Ok(_) => {}
        Err(error) => {
            return Err(CmdError::msg(format!(
                "The anchor's title could not be minted, so nothing was created: {error}"
            )));
        }
    }
    let rendered = render_principal(ctx.w, &landlord);
    put_property(ctx, p);
    crate::module::persist_meta(ctx);
    info(
        ctx,
        format!(
            "Property #{id} created: \"{name}\", tier {}, let by {rendered}.",
            tier.get()
        ),
    );
    info(
        ctx,
        format!(
            "Anchor #{} now carries the title record. @property/show {id}",
            anchor.0
        ),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

/// **D-E5-12, enforced** (08d §4's room rule).
///
/// A property room is an ordinary private room a builder dug. Everything else is
/// refused *here*, at the only moment a human can still fix it — because a sim space,
/// a presentation room or the chargen start room reached by one of the paths that call
/// `World::move_to` directly (08d §1.1) would put somebody inside a home the access
/// hook was never consulted about.
fn check_room(
    ctx: &Ctx<'_>,
    room: ObjId,
    property: Option<PropertyId>,
    anchor: ObjId,
    proposed: &BTreeSet<ObjId>,
) -> Result<(), CmdError> {
    if !is_live_room(ctx.w, room) {
        return Err(CmdError::msg("No such room."));
    }
    if let Some(store) = ctx.w.module_state::<HousingStore>() {
        match store.room_entry(room) {
            Some(RoomEntry::Unique(other)) if Some(*other) != property => {
                // Existence hiding: naming somebody else's residence would confirm
                // it exists.
                let visible = store
                    .property(*other)
                    .is_some_and(|p| may_view_property(ctx.w, ctx.actor, p));
                return Err(CmdError::msg(if visible {
                    format!("#{} is already a room of property #{other}.", room.0)
                } else {
                    format!("#{} cannot be part of a property.", room.0)
                }));
            }
            // A room under review belongs to nobody until the review is over
            // (look-back fold, D-LBK-1); the actor is staff here (`require_staff`),
            // but the discipline holds for the record.
            Some(RoomEntry::Conflict(_)) => {
                return Err(CmdError::msg(if is_staff(ctx.w, ctx.actor) {
                    format!(
                        "#{} is under housing review (@housing/quarantine show); repair that first.",
                        room.0
                    )
                } else {
                    format!("#{} cannot be part of a property.", room.0)
                }));
            }
            Some(RoomEntry::Unique(_)) | None => {}
        }
    }
    if let Some(why) = reserved(ctx.w, room) {
        return Err(CmdError::msg(format!(
            "#{} is {why}; it is never a private residence (D-E5-12).",
            room.0
        )));
    }
    // **A room carrying a behavior binding belongs to another module** (verify fold,
    // H4). It is the sim-space / presentation-room / interior-anchor test without the
    // `mush-sim` dependency §2 forbids: a simulation domain, a tactics host and an
    // interior anchor are all *structural* rooms some module keeps state about, and
    // every one of them is marked by a `BehaviorBinding` on the object itself. The
    // thoroughfare rule catches them only when they are reachable from outside — and
    // the **anchor is exempt from the thoroughfare rule**, which left exactly one
    // hole: a sim domain named as a property's anchor passed every check there was.
    if ctx.w.get(room).is_some_and(|o| !o.behaviors.is_empty()) {
        return Err(CmdError::msg(format!(
            "#{} carries a behavior binding, so another module owns it as a structural room \
             (a simulation domain, an interior anchor, a presentation space). It is never a \
             private residence (D-E5-12).",
            room.0
        )));
    }
    // **The thoroughfare rule.** A room reachable by an exit from a room *outside* the
    // property is a public way, not a private residence — the one exception being the
    // anchor, which is the property's declared entrance and is meant to be reachable.
    //
    // It is also what keeps a sim space, a domain presentation room and the chargen
    // start room out without a `mush-sim` dependency housing must not have: every one
    // of them is somewhere a character arrives from outside, so every one of them
    // fails this test. **This is the only rule the anchor is exempt from**, and the
    // reserved list, the `no-property` marker and the behavior-binding test above are
    // what close the gap that exemption leaves.
    if room != anchor {
        let inside = |r: ObjId| proposed.contains(&r);
        let offending: Option<(ObjId, ObjId)> = ctx
            .w
            .iter()
            .filter(|o| o.kind == mush_core::ObjKind::Exit)
            .filter(|o| o.destination == Some(room))
            .find_map(|o| match o.location {
                Some(from) if !inside(from) => Some((o.id, from)),
                _ => None,
            });
        if let Some((exit, from)) = offending {
            return Err(CmdError::msg(format!(
                "#{} is a thoroughfare: exit #{} leads into it from #{}, which is outside the                  property. Only the anchor may be entered from outside.",
                room.0, exit.0, from.0
            )));
        }
    }
    Ok(())
}

/// Rooms that may **never** be property rooms, whatever else is true of them.
///
/// Three sources, and none of them is a `mush-sim` dependency (08d §2 fixes housing's
/// dependency set at `{core, org, economy, jobs}`):
///
/// - **Limbo**, always. It is where every homeless object lands, so a private
///   residence there would be a house the world drops strangers into.
/// - **The wiring-frozen reserved list** ([`crate::HousingModule::with_reserved_rooms`]),
///   which is how the host names the chargen start rooms it parses out of
///   `[onboarding]` — config housing itself must not read.
/// - **The `no-property` marker**, housing's own, so a builder can put a room out of
///   bounds without a config change.
fn reserved(w: &World, room: ObjId) -> Option<&'static str> {
    if room == mush_core::LIMBO {
        return Some("Limbo");
    }
    if w.has_marker(room, crate::module::NO_PROPERTY_MARKER) {
        return Some("marked as never-a-residence");
    }
    w.module_state::<HousingStore>()
        .filter(|s| s.is_reserved(room))
        .map(|_| "a room this server reserved (a chargen start room, or similar)")
}

fn property_rooms(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @property/rooms <id>=<room>[,…]  |  @property/rooms <id>=-<room>[,…]";
    let p = property_of(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let removing = rhs.trim_start().starts_with('-');
    let list = rhs.trim_start().trim_start_matches('-');
    let names: Vec<String> = list
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    let mut next = p.clone();
    for n in &names {
        let room = room_arg(ctx, n)?;
        if removing {
            next = next
                .remove_room(room)
                .map_err(|e| hidden(ctx, Some(&p), None, e.to_string()))?;
        } else {
            let mut proposed = next.rooms.clone();
            proposed.insert(room);
            check_room(ctx, room, Some(p.id), next.anchor, &proposed)?;
            next = next
                .add_room(room)
                .map_err(|e| hidden(ctx, Some(&p), None, e.to_string()))?;
        }
    }
    // Adding rooms back to a residence whose rooms went missing is exactly how a
    // withdrawn property is put back into service, so this is the one verb that
    // un-withdraws — and only when every room it now names is live.
    if next.status == PropertyStatus::Withdrawn
        && !removing
        && next.rooms.iter().all(|r| is_live_room(ctx.w, *r))
    {
        next.status = if ctx
            .w
            .module_state::<HousingStore>()
            .is_some_and(|s| s.has_live_tenancy(p.id))
        {
            PropertyStatus::Let
        } else {
            PropertyStatus::Vacant
        };
        next.revision = next.revision.saturating_add(1);
        info(ctx, format!("Property #{} is back in service.", p.id));
    }
    let n = next.rooms.len();
    put_property(ctx, next);
    info(ctx, format!("Property #{} now holds {n} room(s).", p.id));
    ctx.gate_on_durable_ack();
    Ok(())
}

fn property_tier(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let p = property_of(ctx, ctx.args.lhs())?;
    let tier = ctx
        .args
        .rhs()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .filter(|n| *n <= crate::MAX_LEVEL)
        .ok_or_else(|| CmdError::msg("Usage: @property/tier <id>=<0-5>"))?;
    let mut next = p.clone();
    next.tier = PropertyTier::new(tier);
    next.revision = next.revision.saturating_add(1);
    put_property(ctx, next);
    info(
        ctx,
        format!(
            "Property #{} is tier {tier}. The next assessed period uses it.",
            p.id
        ),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

fn property_feature(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let p = property_of(ctx, ctx.args.lhs())?;
    let key = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: @property/feature <id>=<key>"))?
        .to_string();
    let next = p
        .add_feature(&key, ctx.actor)
        .map_err(|e| hidden(ctx, Some(&p), None, e.to_string()))?;
    put_property(ctx, next);
    info(
        ctx,
        format!("Property #{}: feature \"{key}\" approved.", p.id),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@property/withdraw <id>` — out of service.
///
/// It does **not** touch the tenancies (D-E5-1): whoever lives there still lives
/// there, still gets in, and still shows on `+home`. What changes is that the destroy
/// gate stops protecting the rooms and the access hook stops answering for them, which
/// is the point — withdrawing is how staff clear the way to demolish something.
/// `@property/site <id>=<site>` (S2b): link the atlas site the residence stands on.
/// Every room of the property must lie within the site's footprint — by its room
/// geometry naming the site, or its footprint falling inside the site's; a room off
/// the map, or outside, is named in the refusal. An empty right-hand side clears.
fn property_site(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let p = property_of(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_string();
    let site = if rhs.is_empty() {
        None
    } else {
        let records = mush_atlas::place::records(ctx.w)
            .ok_or_else(|| CmdError::msg("The atlas is not composed on this server."))?;
        let key = match mush_atlas::place::find_place(records, &rhs) {
            Some(mush_atlas::PlaceRef::Site(key)) => key,
            Some(_) => {
                return Err(CmdError::msg(
                    "A property stands on a site, not a larger place.",
                ));
            }
            None => {
                return Err(CmdError::msg(format!(
                    "No site called {rhs:?} on the atlas (name it, or <pack>:<entry>)."
                )));
            }
        };
        let site = records
            .sites
            .get(&key)
            .ok_or_else(|| CmdError::msg("That site is gone."))?;
        let shape = site
            .footprint
            .to_cm()
            .map_err(|e| CmdError::msg(format!("The site's footprint will not measure: {e}")))?;
        let shape = mush_atlas::Shape::Polygon(shape);
        for room in &p.rooms {
            let Some(geo) = records.roomgeo.get(room) else {
                return Err(CmdError::msg(format!(
                    "#{} is on no map; a property links to a site only when every room has \
                     geometry.",
                    room.0
                )));
            };
            if geo.site.as_ref() == Some(&key) {
                continue;
            }
            let inside = geo.footprint.shape().is_ok_and(|room_shape| {
                let b = room_shape.bbox();
                [
                    mush_atlas::PointCm {
                        x: b.min.x,
                        y: b.min.y,
                    },
                    mush_atlas::PointCm {
                        x: b.max.x - 1,
                        y: b.min.y,
                    },
                    mush_atlas::PointCm {
                        x: b.min.x,
                        y: b.max.y - 1,
                    },
                    mush_atlas::PointCm {
                        x: b.max.x - 1,
                        y: b.max.y - 1,
                    },
                ]
                .iter()
                .all(|corner| shape.contains(*corner))
            });
            if !inside {
                return Err(CmdError::msg(format!(
                    "#{} lies outside {}'s footprint; a property stands on one site.",
                    room.0, site.name
                )));
            }
        }
        Some((key, site.name.clone()))
    };
    let mut next = p.clone();
    next.site = site.as_ref().map(|(k, _)| k.clone());
    next.revision = next.revision.saturating_add(1);
    put_property(ctx, next);
    info(
        ctx,
        match site {
            Some((key, name)) => format!(
                "Property #{} stands on site {name} ({}:{}).",
                p.id, key.pack, key.entry
            ),
            None => format!("Property #{} stands on no atlas site.", p.id),
        },
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

fn property_withdraw(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let p = property_of(ctx, ctx.args.lhs())?;
    if p.status == PropertyStatus::Withdrawn {
        return Err(hidden(
            ctx,
            Some(&p),
            None,
            format!("Property #{} is already withdrawn.", p.id),
        ));
    }
    let live = ctx
        .w
        .module_state::<HousingStore>()
        .is_some_and(|s| s.has_live_tenancy(p.id));
    let mut next = p.clone();
    next.status = PropertyStatus::Withdrawn;
    next.revision = next.revision.saturating_add(1);
    put_property(ctx, next);
    info(ctx, format!("Property #{} is withdrawn.", p.id));
    if live {
        info(
            ctx,
            "Its tenancy is UNCHANGED and its tenants still live there — withdrawing is not an \
             eviction. End it deliberately with @tenancy/resolve if that is what you mean."
                .to_string(),
        );
    }
    ctx.gate_on_durable_ack();
    Ok(())
}

fn property_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let p = property_of(ctx, ctx.args.lhs())?;
    // A1: the guest reduction `+home` and `@tenancy/show` already apply. One line,
    // and nothing about the landlord, the tier, the rooms, the features, the status
    // or who lives there.
    // Look-back fold, D-LBK-1: under review, members and officers read exactly that,
    // a guest — "everyone else" — reads the hidden text, and staff read the record
    // and the review.
    let review = review_lines_for_property(ctx.w, p.id);
    if !review.is_empty() && guest_only_property(ctx.w, ctx.actor, &p) {
        return Err(hidden_property());
    }
    if guest_only_property(ctx.w, ctx.actor, &p) {
        info(ctx, guest_line(&p.name));
        return Ok(());
    }
    if !review.is_empty() && !is_staff(ctx.w, ctx.actor) {
        info(
            ctx,
            under_review(&format!("Property #{} — \"{}\"", p.id, p.name)),
        );
        return Ok(());
    }
    let landlord = render_principal(ctx.w, &p.landlord);
    let rooms: Vec<String> = p.rooms.iter().map(|r| format!("#{}", r.0)).collect();
    info(
        ctx,
        format!("Property #{} — \"{}\" [{:?}]", p.id, p.name, p.status),
    );
    info(
        ctx,
        format!(
            "  landlord {landlord}  tier {}  anchor #{}",
            p.tier.get(),
            p.anchor.0
        ),
    );
    info(ctx, format!("  rooms: {}", rooms.join(" ")));
    if let Some(site) = &p.site {
        let name = mush_atlas::place::records(ctx.w)
            .and_then(|r| r.sites.get(site))
            .map_or_else(|| "a site that is gone".to_string(), |s| s.name.clone());
        info(
            ctx,
            format!("  site: {name} ({}:{})", site.pack, site.entry),
        );
    }
    for line in review {
        info(ctx, line);
    }
    if !p.features.is_empty() {
        let f: Vec<String> = p.features.iter().map(|f| f.key.to_string()).collect();
        info(ctx, format!("  features: {}", f.join(", ")));
    }
    let tenancies: Vec<String> = ctx
        .w
        .module_state::<HousingStore>()
        .map(|s| {
            s.tenancies()
                .filter(|t| t.property == p.id && !t.status.is_ended())
                .map(|t| format!("  tenancy #{} — {}", t.id, t.status.kind().label()))
                .collect()
        })
        .unwrap_or_default();
    for row in tenancies {
        info(ctx, row);
    }
    Ok(())
}

fn property_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (_, opts) = parse_opts(ctx.args.raw(), &["landlord"]);
    // **A3: `landlord=` never widens to "no filter".** An unresolvable value used to
    // fall through to an unfiltered list, so the *shape* of the answer told a probe
    // whether an organization exists: `landlord=org:Ironhold` listed nothing and
    // `landlord=org:Nowhere` listed everything they could see. It is now the same
    // refusal `@property/create` gives an unknown landlord, and it is the record's
    // own hidden text — so a typo, a nonexistent org and an org whose properties this
    // actor may not see are one message.
    let filter = match opts.get("landlord") {
        None => None,
        Some(s) => Some(parse_landlord(ctx, s).ok_or_else(hidden_property)?),
    };
    let rows: Vec<String> = ctx
        .w
        .module_state::<HousingStore>()
        .map(|s| {
            s.properties()
                .filter(|p| may_view_property(ctx.w, ctx.actor, p))
                .filter(|p| filter.as_ref().is_none_or(|f| p.landlord == *f))
                .map(|p| {
                    // A1: a guest's row is their own standing, never the books.
                    if guest_only_property(ctx.w, ctx.actor, p) {
                        return format!("#{} — {}", p.id, guest_line(&p.name));
                    }
                    format!(
                        "#{} \"{}\" — tier {} · {} room(s) · {:?}",
                        p.id,
                        p.name,
                        p.tier.get(),
                        p.rooms.len(),
                        p.status
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if filter.is_some() && rows.is_empty() {
        return Err(hidden_property());
    }
    emit_list(ctx, rows, "No properties.");
    Ok(())
}

// ---------------------------------------------------------------------------
// @tenancy
// ---------------------------------------------------------------------------

fn cmd_tenancy(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let sw = &ctx.switches;
    if sw.has("show") {
        return tenancy_show(ctx);
    }
    if sw.has("list") {
        return tenancy_list(ctx);
    }
    // §11 answer 3, binding: creating and resolving are **staff**, not
    // `property.manage`. An officer with the landlord's management right may read and
    // set guest policy; deciding who lives somewhere is a staff act in E5.
    require_staff(ctx)?;
    if sw.has("create") {
        tenancy_create(ctx)
    } else if sw.has("resolve") {
        tenancy_resolve(ctx)
    } else {
        Err(CmdError::msg("Usage: @tenancy/create|resolve|show|list"))
    }
}

fn tenancy_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @tenancy/create <property>=<tenant>[,…] cadence=<id> \
                         [inspect=<who>:<always|delinquent>] \
                         [mode=exact rent=<n> from=<acct> to=<acct>]";
    let p = property_of(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    // A7: `every=` was read by `offer_lease` and never collected here, so the key
    // fell through into the tenant list and the rent period silently stayed at the
    // default whatever anybody typed.
    let (rest, opts) = parse_opts(
        &rhs,
        &["cadence", "inspect", "mode", "rent", "from", "to", "every"],
    );
    if p.status == PropertyStatus::Withdrawn {
        return Err(hidden(
            ctx,
            Some(&p),
            None,
            format!(
                "Property #{} is withdrawn; put it back in service first.",
                p.id
            ),
        ));
    }
    // **One live tenancy per property** (E5a fold, handoff 1).
    if ctx
        .w
        .module_state::<HousingStore>()
        .is_some_and(|s| s.has_live_tenancy(p.id))
    {
        return Err(hidden(
            ctx,
            Some(&p),
            None,
            format!(
                "Property #{} already has a live tenancy. End it first (@tenancy/resolve) — a \
                 residence answers for exactly one.",
                p.id
            ),
        ));
    }
    let mut tenants: BTreeSet<ObjId> = BTreeSet::new();
    for n in rest.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let who = match parse_id(n) {
            Some(v) => ObjId(u32::try_from(v).map_err(|_| CmdError::msg("No such player."))?),
            None => {
                match_object(ctx.w, ctx.actor, n).map_err(|_| CmdError::msg("No such player."))?
            }
        };
        // E5a fold, handoff 2: the library takes any `ObjId`; a tenancy is for people.
        if !is_character(ctx.w, who) {
            return Err(CmdError::msg(format!("#{} is not a character.", who.0)));
        }
        tenants.insert(who);
    }
    if tenants.is_empty() || tenants.len() > MAX_TENANTS {
        return Err(CmdError::msg(format!("Name 1 to {MAX_TENANTS} tenants.")));
    }
    let cadence = opts
        .get("cadence")
        .and_then(|s| parse_id(s))
        .map(CadenceId)
        .filter(|id| {
            ctx.w
                .module_state::<mush_economy::EconomyStore>()
                .is_some_and(|e| e.snapshot().cadence(*id).is_some())
        })
        .ok_or_else(|| {
            CmdError::msg("cadence= must name a campaign clock (@econ/cadence list).")
        })?;
    let terms = match opts.get("inspect") {
        None => AccessTerms::default(),
        Some(spec) => AccessTerms {
            inspection: Some(parse_inspection(ctx, spec)?),
        },
    };
    let exact = opts
        .get("mode")
        .is_some_and(|m| m.eq_ignore_ascii_case("exact"));

    if exact {
        let id = {
            let store = ctx
                .w
                .module_state_mut::<HousingStore>()
                .ok_or_else(|| CmdError::msg("No housing."))?;
            store
                .alloc_tenancy()
                .ok_or_else(|| CmdError::msg("The tenancy-id allocator is exhausted."))?
        };
        let mut t = Tenancy::new(id, p.id, tenants.clone(), cadence)
            .map_err(|e| CmdError::msg(e.to_string()))?;
        t.terms = terms;
        let contract = offer_lease(ctx, &p, &t, &opts)?;
        t.funding = Funding::Exact {
            contract,
            obligation: crate::econ::PENDING_OBLIGATION,
        };
        crate::flow::seed_assessment(ctx, &t);
        put_tenancy(ctx, t);
        crate::module::persist_meta(ctx);
        info(
            ctx,
            format!("Tenancy #{id} drafted on lease contract #{contract}."),
        );
        info(
            ctx,
            format!(
                "It is PENDING until every tenant runs @contract/accept {contract} — their \
                 acceptance is the consent, and activation creates the rent obligation."
            ),
        );
        ctx.gate_on_durable_ack();
        return Ok(());
    }

    // Abstract: the staff act that creates it is both consents recorded at once —
    // there is no separate accept verb in E5 (08d §4's command list), and chargen's
    // approval step (§5.4) creates a live tenancy through the **same** entry.
    let id = crate::flow::create_tenancy(ctx, p.id, tenants.clone(), cadence, terms)
        .map_err(|e| hidden(ctx, Some(&p), None, e.to_string()))?;
    let who: Vec<String> = tenants.iter().map(|o| format!("#{}", o.0)).collect();
    info(
        ctx,
        format!(
            "Tenancy #{id} is active on property #{}: {}.",
            p.id,
            who.join(", ")
        ),
    );
    info(
        ctx,
        "Their home anchor now points at the property. Assessment runs on the named cadence; \
         supported periods say nothing at all."
            .to_string(),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

/// Draft and offer the lease contract an exact tenancy stands on
/// ([`mush_economy::offer_lease`], 08d §4's "exact mode").
fn offer_lease(
    ctx: &mut Ctx<'_>,
    p: &Property,
    t: &Tenancy,
    opts: &BTreeMap<String, String>,
) -> Result<mush_economy::ContractId, CmdError> {
    const USAGE: &str =
        "Exact mode needs rent=<minor units> from=<tenant's account> to=<landlord's account>.";
    let rent = opts
        .get("rent")
        .and_then(|s| s.parse::<i64>().ok())
        .map(Amount::from_minor)
        .filter(|a| !a.is_zero() && !a.is_negative())
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let debit = opts
        .get("from")
        .and_then(|s| parse_id(s))
        .map(mush_economy::AccountId)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let credit = opts
        .get("to")
        .and_then(|s| parse_id(s))
        .map(mush_economy::AccountId)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let every = opts
        .get("every")
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_RENT_PERIOD_SECS);
    let (debtor, creditor) = {
        let store = ctx
            .w
            .module_state::<mush_economy::EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let snap = store.snapshot();
        let (Some(d), Some(c)) = (snap.account(debit), snap.account(credit)) else {
            return Err(CmdError::msg("No such account."));
        };
        (d.owner.clone(), c.owner.clone())
    };
    // **T2: the rent debits a tenant, and only a tenant.**
    //
    // `from=` was any account id at all, so `@tenancy/create … mode=exact` could
    // point a standing per-period charge at somebody who is not on the lease, is not
    // a party to the contract, and is never asked — their `@contract/accept` is not
    // required because they are not a party, so the tenants' acceptance alone
    // activated a charge against a bystander's wallet. And `from=<faucet>` was worse
    // than that: a system source on the debit side is a standing mint, which is the
    // one thing `@obligation/create` refuses by name.
    //
    // The economy re-checks the same rule at `check_offerable` (the debtor must be a
    // party, and must not be a system principal), so this refusal is the readable
    // one rather than the only one.
    match &debtor {
        PrincipalId::Object(o) if t.tenants.contains(o) => {}
        PrincipalId::System(_) => {
            return Err(CmdError::msg(
                "A system source/sink cannot be the debit side of the rent: that would be a \
                 standing mint. Name a tenant's own account.",
            ));
        }
        _ => {
            return Err(CmdError::msg(
                "from= must be an account one of this tenancy's TENANTS owns: the rent \
                 obligation debits a party to the lease, never a bystander.",
            ));
        }
    }
    let now = ctx.w.now();
    let spec = mush_economy::LeaseSpec {
        name: SmolStr::new(format!("Lease of {}", p.name)),
        lessor: p.landlord.clone(),
        scope: p.landlord_scope,
        tenants: t.tenants.iter().map(|o| PrincipalId::Object(*o)).collect(),
        premises: p.anchor,
        rent: ObligationTemplate {
            debtor,
            creditor,
            debit_account: debit,
            credit_account: credit,
            amount: rent,
            schedule: mush_economy::EconomicSchedule::every(now.saturating_add(every), every),
            grace: mush_economy::GracePolicy::new(mush_economy::DEFAULT_GRACE_PERIODS),
            notify_on_success: false,
        },
    };
    mush_economy::offer_lease(ctx, spec).map_err(|e| {
        CmdError::msg(format!(
            "The lease could not be offered, so nothing was created: {e}"
        ))
    })
}

/// How often an exact lease charges rent when nobody said (one real day).
const DEFAULT_RENT_PERIOD_SECS: i64 = 86_400;

/// `inspect=<who>:<always|delinquent>` — the one **explicit** entry term (owner
/// answer 9). There is no implied landlord entry and no "delinquency unlocks the
/// door"; a term that does not say *when* is refused, because E5a deliberately gives
/// `InspectionWhen` no default (fold A6).
fn parse_inspection(ctx: &Ctx<'_>, spec: &str) -> Result<InspectionTerm, CmdError> {
    const USAGE: &str = "inspect=<#obj|org:<name>>:<always|delinquent>";
    let (who, when) = spec.rsplit_once(':').ok_or_else(|| CmdError::msg(USAGE))?;
    let who = parse_landlord(ctx, who).ok_or_else(|| CmdError::msg("No such principal."))?;
    let when = match when.trim().to_ascii_lowercase().as_str() {
        "always" => InspectionWhen::Always,
        "delinquent" | "whiledelinquent" => InspectionWhen::WhileDelinquent,
        _ => return Err(CmdError::msg(USAGE)),
    };
    Ok(InspectionTerm { who, when })
}

/// `@tenancy/resolve <id>=extend|end|waive [reason]` — the human decision the story
/// window exists to ask for.
fn tenancy_resolve(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @tenancy/resolve <id>=extend|scene|end|waive [reason]";
    let t = tenancy_of(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .trim()
        .to_string();
    let (verb, reason) = match rhs.split_once(char::is_whitespace) {
        Some((v, r)) => (v.to_ascii_lowercase(), r.trim().to_string()),
        None => (rhs.to_ascii_lowercase(), String::new()),
    };
    let now = crate::module::cadence_now(ctx.w, t.cadence).unwrap_or(0);
    let before = t.status.kind();
    match verb.as_str() {
        "extend" => {
            let TenancyStatus::EvictionPending { window } = t.status.clone() else {
                return Err(hidden(
                    ctx,
                    None,
                    Some(&t),
                    format!("Tenancy #{} has no open story window to extend.", t.id),
                ));
            };
            // A **module-level** window reset. E5a has no `extend` arrow — its window
            // functions only ever spend the budget — and inventing one there would
            // have meant a pure function that makes a window younger, which is
            // exactly the shape the decay rules exist to forbid. Here it is what it
            // is: a staffer said "keep this open and tell them again", so the notice
            // count starts over while the job and the page it already opened stay
            // pointed at the same records.
            //
            // **T3: the escalation id is NOT carried over.** It used to be, and that
            // made `extend` the quietest way to end a story: `window_due` answers
            // `NotifyEscalated` (weekly, forever) for any window that already has an
            // escalation, so a staffer who typed `extend` after the page had opened
            // re-armed three notices that could never be sent and guaranteed the
            // window would never ask for a person again. Dropping it is safe
            // precisely because the ladder is the **economy's**: paging again finds
            // the same subject and reopens the same record, id and history intact, so
            // "extend" costs a re-page rather than a second alert.
            let mut fresh = StoryWindow::opened_at(now);
            fresh.job = window.job;
            fresh.scene = window.scene;
            let mut next = t.clone();
            next.status = TenancyStatus::EvictionPending { window: fresh };
            next.revision = next.revision.saturating_add(1);
            put_tenancy(ctx, next.clone());
            crate::flow::notify_tenants(
                ctx,
                &next,
                &format!(
                    "HOME: staff have extended the window on tenancy #{}. Nothing has changed \
                     about where you live.",
                    next.id
                ),
            );
            info(
                ctx,
                format!("Tenancy #{}: the story window is extended.", t.id),
            );
        }
        "scene" => {
            // 08d §4's "optionally starts a scene, when the ST asks via the job".
            // The ST asks by typing this: there is no callback from a job comment
            // into a module, and inventing one would be a second, weaker dispatch
            // path into housing. The `SceneRef` recorded on the window is opaque and
            // boot-local by construction (E5a deviation 8) — a core `SceneId`
            // restarts at 0 every boot, so a persisted one would name a different
            // scene after a restart.
            let TenancyStatus::EvictionPending { window } = &t.status else {
                return Err(hidden(
                    ctx,
                    None,
                    Some(&t),
                    format!("Tenancy #{} has no open story window.", t.id),
                ));
            };
            // **T8: ask before starting one.** `attach_window` refuses to replace a
            // different scene id, but the scene was started *first* — so a second
            // `=scene` created a real core scene, pulled every tenant into it, and
            // then threw away the only reference to it. The refusal has to come
            // before the side effect.
            if window.scene.is_some() {
                return Err(hidden(
                    ctx,
                    None,
                    Some(&t),
                    format!(
                        "Tenancy #{}'s story window already has a scene. (A scene recorded \
                         before a reboot is cleared at boot — core scene ids restart — so this \
                         is only ever a scene running now.)",
                        t.id
                    ),
                ));
            }
            let mut members: BTreeSet<ObjId> = t.tenants.clone();
            members.insert(ctx.actor);
            let scene = ctx.start_scene("housing.story", members);
            match crate::attach_window(&t, None, Some(crate::SceneRef(scene.0))) {
                Ok(next) => {
                    put_tenancy(ctx, next);
                    info(ctx, format!("Scene {scene} started for tenancy #{}.", t.id));
                }
                Err(e) => return Err(hidden(ctx, None, Some(&t), e.to_string())),
            }
        }
        "end" | "waive" => {
            let how = if verb == "waive" {
                Resolution::Waived { by: ctx.actor }
            } else {
                Resolution::staff_ended(ctx.actor, &reason)
                    .map_err(|e| CmdError::msg(e.to_string()))?
            };
            let next = crate::resolve(&t, how, now)
                .map_err(|e| hidden(ctx, None, Some(&t), e.to_string()))?;
            crate::flow::notify_tenants(
                ctx,
                &next,
                &format!(
                    "HOME: tenancy #{} has ended ({}). Your home anchor there is cleared.",
                    next.id,
                    if verb == "waive" {
                        "waived by staff"
                    } else {
                        "ended by staff"
                    }
                ),
            );
            crate::flow::close_pending_job(ctx, &t, verb == "waive");
            crate::flow::commit(ctx, before, next);
            info(ctx, format!("Tenancy #{} ended.", t.id));
        }
        _ => return Err(CmdError::msg(USAGE)),
    }
    ctx.gate_on_durable_ack();
    Ok(())
}

fn tenancy_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let t = tenancy_of(ctx, ctx.args.lhs())?;
    // A **guest** sees the property's name and their own standing, and nothing about
    // who lives there (08d §4's per-role fields).
    // Look-back fold, D-LBK-1: under review, a member or an officer reads exactly
    // that, a guest — "everyone else" — reads the hidden text, and staff read the
    // record and the review.
    let review = review_lines_for_tenancy(ctx.w, t.id);
    if !review.is_empty() && guest_only(ctx.w, ctx.actor, &t) {
        return Err(hidden_tenancy());
    }
    if guest_only(ctx.w, ctx.actor, &t) {
        let name = property_name(ctx.w, t.property);
        info(ctx, guest_line(&name));
        return Ok(());
    }
    if !review.is_empty() && !is_staff(ctx.w, ctx.actor) {
        info(
            ctx,
            under_review(&format!(
                "Tenancy #{} — {}",
                t.id,
                property_name(ctx.w, t.property)
            )),
        );
        return Ok(());
    }
    for row in render_tenancy(ctx, &t) {
        info(ctx, row);
    }
    for line in review {
        info(ctx, line);
    }
    Ok(())
}

/// The review lines for a quarantined property (empty when it is not under review).
fn review_lines_for_property(w: &World, property: PropertyId) -> Vec<String> {
    w.module_state::<HousingStore>()
        .map(|s| {
            s.quarantine()
                .filter(|c| c.properties.contains(&property))
                .map(|c| {
                    format!(
                        "  UNDER REVIEW {}: {} — repair: {}",
                        c.key,
                        c.summary,
                        c.repair.join(" · ")
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The review lines for a quarantined tenancy (empty when it is not under review).
fn review_lines_for_tenancy(w: &World, tenancy: TenancyId) -> Vec<String> {
    w.module_state::<HousingStore>()
        .map(|s| {
            s.quarantine()
                .filter(|c| c.tenancies.contains(&tenancy))
                .map(|c| {
                    format!(
                        "  UNDER REVIEW {}: {} — repair: {}",
                        c.key,
                        c.summary,
                        c.repair.join(" · ")
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn tenancy_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (_, opts) = parse_opts(ctx.args.raw(), &["property"]);
    let filter = opts
        .get("property")
        .and_then(|s| parse_id(s))
        .map(PropertyId);
    let rows: Vec<String> = ctx
        .w
        .module_state::<HousingStore>()
        .map(|s| {
            s.tenancies()
                .filter(|t| may_view_tenancy(ctx.w, ctx.actor, t))
                .filter(|t| filter.is_none_or(|f| t.property == f))
                .map(|t| {
                    // A1: a guest reads their own standing here too — the status and
                    // the tenant count are exactly what an invitation does not buy.
                    if guest_only(ctx.w, ctx.actor, t) {
                        return format!(
                            "#{} — {}",
                            t.id,
                            guest_line(&property_name(ctx.w, t.property))
                        );
                    }
                    format!(
                        "#{} — property #{} · {} · {} tenant(s)",
                        t.id,
                        t.property,
                        t.status.kind().label(),
                        t.tenants.len()
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    emit_list(ctx, rows, "No tenancies.");
    Ok(())
}

/// Whether this actor's only standing on the tenancy is "guest".
fn guest_only(w: &World, actor: ObjId, t: &Tenancy) -> bool {
    if !t.is_guest(actor) || t.is_tenant(actor) || is_staff(w, actor) {
        return false;
    }
    w.module_state::<HousingStore>()
        .and_then(|s| s.property(t.property).cloned())
        // A landlord officer — of either capability, on the compartment or the org's
        // root scope — is not "just a guest" (fold A4/A5).
        .is_none_or(|p| !officer_of(w, actor, &p) && p.landlord != PrincipalId::Object(actor))
}

fn property_name(w: &World, property: PropertyId) -> String {
    w.module_state::<HousingStore>()
        .and_then(|s| s.property(property).map(|p| p.name.to_string()))
        .unwrap_or_else(|| format!("property #{property}"))
}

/// The **assessment word** for an abstract tenancy: what the ladder makes of the
/// residents' plans this period, and nothing about where the number came from
/// (fold A2).
fn assessment_word(
    w: &World,
    t: &Tenancy,
    plans: &BTreeMap<ObjId, crate::SupportPlan>,
) -> &'static str {
    let (tier, period) = w.module_state::<HousingStore>().map_or_else(
        || (crate::PropertyTier::default(), 0),
        |s| {
            (
                s.property(t.property)
                    .map_or_else(crate::PropertyTier::default, |p| p.tier),
                s.assess_state(t.id).period,
            )
        },
    );
    crate::assess_at(tier, plans, &t.tenants, period, crate::PLAN_MAX_AGE_PERIODS).label()
}

/// The full tenancy render (staff, landlord officer, tenant).
fn render_tenancy(ctx: &Ctx<'_>, t: &Tenancy) -> Vec<String> {
    let staff = is_staff(ctx.w, ctx.actor);
    let mut out = Vec::new();
    let name = property_name(ctx.w, t.property);
    out.push(format!(
        "Tenancy #{} — {name} (property #{}) · {}",
        t.id,
        t.property,
        t.status.kind().label()
    ));
    let who: Vec<String> = t
        .tenants
        .iter()
        .map(|o| {
            ctx.w
                .get(*o)
                .map_or_else(|| format!("#{}", o.0), |x| x.name.clone())
        })
        .collect();
    out.push(format!("  tenants: {}", who.join(", ")));
    if !t.guests.is_empty() {
        let g: Vec<String> = t
            .guests
            .iter()
            .map(|o| {
                ctx.w
                    .get(*o)
                    .map_or_else(|| format!("#{}", o.0), |x| x.name.clone())
            })
            .collect();
        out.push(format!("  guests: {}", g.join(", ")));
    }
    match &t.funding {
        // **A2: the assessment word to everyone, the number to staff alone.**
        //
        // A `SupportLevel` is the reading `ruleset-wod` derives from a *sheet*
        // (D-E5-5's whole point is that housing never sees one), and the best-of is
        // the **stronger** co-tenant's band — so printing it told a landlord's
        // officer, and every co-tenant, roughly how rich somebody's character is.
        // The number, how many residents have filed a plan, and which sources they
        // named are all staff-only; what a tenant or an officer needs is whether the
        // place is being carried, which is the word the ladder itself acts on.
        Funding::Abstract { plans } => {
            if staff {
                let best =
                    crate::best_level(plans).map_or("—".to_string(), |l| l.get().to_string());
                out.push(format!(
                    "  funding: abstract · {} plan(s) · best level {best}",
                    plans.len()
                ));
            } else {
                out.push(format!(
                    "  funding: abstract · {}",
                    assessment_word(ctx.w, t, plans)
                ));
            }
        }
        Funding::Exact {
            contract,
            obligation,
        } => {
            if crate::econ::is_pending_lease(&t.funding) {
                out.push(format!(
                    "  funding: exact · lease contract #{contract} (not yet accepted)"
                ));
            } else {
                out.push(format!(
                    "  funding: exact · contract #{contract} · rent obligation #{obligation}"
                ));
            }
        }
    }
    if let Some(term) = &t.terms.inspection {
        out.push(format!(
            "  inspection: {} ({})",
            render_principal(ctx.w, &term.who),
            match term.when {
                InspectionWhen::Always => "always",
                InspectionWhen::WhileDelinquent => "while delinquent",
            }
        ));
    }
    if let TenancyStatus::EvictionPending { window } = &t.status {
        out.push(format!(
            "  story window: opened {} · {} of {} notices · job {} · alert {}",
            window.opened,
            window.notified,
            crate::MAX_WINDOW_NOTICES,
            window
                .job
                .map_or_else(|| "—".to_string(), |j| format!("#{j}")),
            window
                .escalation
                .map_or_else(|| "—".to_string(), |e| format!("#{e}"))
        ));
        out.push(format!(
            "  @tenancy/resolve {}=extend | end | waive — nothing else will happen on its own",
            t.id
        ));
    }
    let state = ctx
        .w
        .module_state::<HousingStore>()
        .map(|s| s.assess_state(t.id))
        .unwrap_or_default();
    out.push(format!(
        "  cadence #{} · last assessed at position {} (period {})",
        t.cadence, state.position, state.period
    ));
    out
}

// ---------------------------------------------------------------------------
// +home / +guest
// ---------------------------------------------------------------------------

fn cmd_home(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("leave") {
        return home_leave(ctx);
    }
    let me = ctx.actor;
    let (tenant, guest): (Vec<Tenancy>, Vec<Tenancy>) = ctx
        .w
        .module_state::<HousingStore>()
        .map(|s| {
            (
                s.tenancies()
                    .filter(|t| t.status.is_live() && t.is_tenant(me))
                    .cloned()
                    .collect(),
                s.tenancies()
                    .filter(|t| t.status.is_live() && t.is_guest(me))
                    .cloned()
                    .collect(),
            )
        })
        .unwrap_or_default();
    if tenant.is_empty() && guest.is_empty() {
        info(
            ctx,
            "You have no home and no standing invitation.".to_string(),
        );
        return Ok(());
    }
    for t in &tenant {
        // Look-back fold, D-LBK-1: a home under review reads as exactly that.
        if ctx
            .w
            .module_state::<HousingStore>()
            .is_some_and(|s| s.is_quarantined_tenancy(t.id))
        {
            info(
                ctx,
                under_review(&format!(
                    "Tenancy #{} — {}",
                    t.id,
                    property_name(ctx.w, t.property)
                )),
            );
            continue;
        }
        for row in render_tenancy(ctx, t) {
            info(ctx, row);
        }
        // The pending notice, if one is still queued (a player who has not been on
        // since it was sent sees it here as well as at their next login).
        let pending = ctx
            .w
            .module_state::<HousingStore>()
            .and_then(|s| crate::notice::latest_pending(s, me).map(|n| n.text.to_string()));
        if let Some(text) = pending {
            info(ctx, format!("  waiting for you: {text}"));
        }
    }
    // A guest sees the property's name and their own standing, and nothing else —
    // and nothing at all about a home under review (D-LBK-1: the hidden text, which
    // on this list is no line).
    for t in &guest {
        if ctx
            .w
            .module_state::<HousingStore>()
            .is_some_and(|s| s.is_quarantined_tenancy(t.id))
        {
            continue;
        }
        let name = property_name(ctx.w, t.property);
        info(ctx, guest_line(&name));
    }
    Ok(())
}

/// **Which of the actor's own homes a `+home`/`+guest` verb acts on** (verify fold,
/// A6).
///
/// Both verbs used to take `live_tenancies_of_tenant(me).next()` — the **lowest id**,
/// silently. So a player on two leases who typed a bare `+home/leave` ended whichever
/// one they happened to have taken first, and `+guest add` let somebody into a house
/// they may not have meant. `+home/leave` is the one irreversible act in the whole
/// player surface (D-E5-1's tenant half), so with more than one and none named it
/// refuses and lists them rather than guessing.
fn my_tenancy(ctx: &Ctx<'_>, named: Option<&str>, none: &str) -> Result<Tenancy, CmdError> {
    let me = ctx.actor;
    let mine: Vec<Tenancy> = ctx
        .w
        .module_state::<HousingStore>()
        .map(|s| s.live_tenancies_of_tenant(me).cloned().collect())
        .unwrap_or_default();
    if let Some(arg) = named.map(str::trim).filter(|s| !s.is_empty()) {
        let id = parse_id(arg).map(TenancyId).ok_or_else(hidden_tenancy)?;
        // Existence hiding: naming somebody *else's* tenancy answers exactly as a
        // nonexistent one does.
        return mine
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(hidden_tenancy);
    }
    match mine.len() {
        0 => Err(CmdError::msg(none.to_string())),
        1 => mine.into_iter().next().ok_or_else(hidden_tenancy),
        _ => {
            let list: Vec<String> = mine
                .iter()
                .map(|t| format!("#{} ({})", t.id, property_name(ctx.w, t.property)))
                .collect();
            Err(CmdError::msg(format!(
                "You live in more than one place; say which one: {}",
                list.join(", ")
            )))
        }
    }
}

/// Clear a leaver's `Obj.home` **only when it points inside the property they are
/// leaving**, then re-home them onto another live lease (verify fold, H3).
///
/// The unconditional `set_home(me, None)` this replaces is `to_unhome_on_end`'s
/// carve-out inverted: a co-tenant of two leases whose home already pointed at the
/// *other* property had that pointer cleared by leaving this one, so core's destroy
/// dispersal — which honours no hook — would have dropped them in Limbo instead of
/// the house they still live in.
fn drop_home_here(ctx: &mut Ctx<'_>, who: ObjId, t: &Tenancy) {
    let Some(p) = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|s| s.property(t.property).cloned())
    else {
        return;
    };
    let rooms = crate::module::live_rooms(ctx.w, &p);
    let inside = ctx
        .w
        .get(who)
        .and_then(|o| o.home)
        .is_some_and(|h| h == p.anchor || rooms.contains(&h));
    if !inside {
        return;
    }
    // The invariant is "`Obj.home` follows *a* live tenancy", so a leaver who still
    // rents elsewhere is re-homed rather than un-homed (the same A5 rule `flow::end`
    // applies to the whole roster).
    let elsewhere = ctx.w.module_state::<HousingStore>().and_then(|s| {
        s.live_tenancies_of_tenant(who)
            .find(|other| other.id != t.id)
            .and_then(|other| s.property(other.property).map(|op| op.anchor))
    });
    if let Err(error) = ctx.w.set_home(who, elsewhere) {
        tracing::warn!(who = who.0, %error, "housing: could not move a leaver's home");
    }
}

/// `+home/leave [<tenancy>]` — the tenant's own move-out (D-E5-11's clearing half).
fn home_leave(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let named = ctx.args.lhs().to_string();
    let t = my_tenancy(ctx, Some(&named), "You have no home to leave.")?;
    let now = crate::module::cadence_now(ctx.w, t.cadence).unwrap_or(0);
    let before = t.status.kind();
    // A co-tenant leaving a shared lease is a roster change, not an ending: the
    // library refuses to remove the last tenant precisely so that "everybody left"
    // still goes through an explicit resolution.
    if t.tenants.len() > 1 {
        let next = t
            .remove_tenant(me, now)
            .map_err(|e| CmdError::msg(e.to_string()))?;
        put_tenancy(ctx, next);
        // Their own home pointer is theirs to drop — but only the one that points
        // *here* (H3); the tenancy's `homed` set keeps the record of what housing
        // set, and the ending sweep is what clears it (E5a fold A1).
        drop_home_here(ctx, me, &t);
        info(
            ctx,
            format!(
                "You have moved out of tenancy #{}. Your co-tenants stay.",
                t.id
            ),
        );
        ctx.gate_on_durable_ack();
        return Ok(());
    }
    let next = crate::resolve(&t, Resolution::TenantLeft, now)
        .map_err(|e| CmdError::msg(e.to_string()))?;
    crate::flow::close_pending_job(ctx, &t, false);
    crate::flow::commit(ctx, before, next);
    info(
        ctx,
        format!(
            "You have moved out. Tenancy #{} is ended and your home is cleared.",
            t.id
        ),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

fn cmd_guest(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: +guest [home=<tenancy>]  |  +guest add <who> [home=<tenancy>]  |  \
         +guest remove <who> [home=<tenancy>]";
    let me = ctx.actor;
    // Both spellings: the switch form and the bare word the help prints. `home=` is
    // A6's disambiguator — a player on two leases has to say which door.
    let (raw, opts) = parse_opts(ctx.args.raw().trim(), &["home"]);
    let t = my_tenancy(
        ctx,
        opts.get("home").map(String::as_str),
        "You have no home to invite anybody into.",
    )?;
    let (verb, target) = if ctx.switches.has("add") {
        ("add".to_string(), raw)
    } else if ctx.switches.has("remove") {
        ("remove".to_string(), raw)
    } else {
        match raw.split_once(char::is_whitespace) {
            Some((v, r)) => (v.to_ascii_lowercase(), r.trim().to_string()),
            None if raw.is_empty() => (String::new(), String::new()),
            None => (raw.to_ascii_lowercase(), String::new()),
        }
    };
    if verb.is_empty() {
        let g: Vec<String> = t
            .guests
            .iter()
            .map(|o| {
                ctx.w
                    .get(*o)
                    .map_or_else(|| format!("#{}", o.0), |x| x.name.clone())
            })
            .collect();
        emit_list(ctx, g, "You have let nobody in.");
        return Ok(());
    }
    let who =
        match_object(ctx.w, me, target.trim()).map_err(|_| CmdError::msg("No such player."))?;
    match verb.as_str() {
        "add" => {
            // E5a fold, handoff 2.
            if !is_character(ctx.w, who) {
                return Err(CmdError::msg("Only a character can be a guest."));
            }
            let next = t.add_guest(who).map_err(|e| CmdError::msg(e.to_string()))?;
            put_tenancy(ctx, next);
            info(ctx, format!("#{} may now come and go.", who.0));
            crate::notice::notify(
                ctx,
                who,
                format!(
                    "HOME: you have been made a guest of {}.",
                    property_name(ctx.w, t.property)
                ),
            );
        }
        "remove" => {
            let next = t.remove_guest(who);
            put_tenancy(ctx, next);
            info(ctx, format!("#{} is no longer a guest.", who.0));
        }
        _ => return Err(CmdError::msg(USAGE)),
    }
    ctx.gate_on_durable_ack();
    Ok(())
}
