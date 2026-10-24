//! Private topology knowledge, positive carriage grants, and explicit IC agency.
//! Service class, payment, membership and a right to inspect are not use grants.
use crate::{
    PropagationStore,
    service::{self, Principal},
};
use mush_core::{CanonicalId, CmdError, Ctx, ObjId, RelayRepresentation, TransportRefusal, World};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Right {
    Discover,
    Use,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub pair: CanonicalId,
    pub principal: Principal,
    pub rights: BTreeSet<Right>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub grants: Vec<Grant>,
    pub public_carriage: BTreeSet<CanonicalId>,
    pub represented: BTreeMap<ObjId, RelayRepresentation>,
}
impl Policy {
    pub fn validate(&self) -> Result<(), String> {
        let keys: BTreeSet<_> = self
            .grants
            .iter()
            .map(|g| (&g.pair, &g.principal))
            .collect();
        if self.grants.len() > 256
            || keys.len() != self.grants.len()
            || self.public_carriage.len() > 64
            || self.represented.len() > 128
            || self.grants.iter().any(|g| {
                g.rights.is_empty()
                    || match g.principal {
                        Principal::Object(id) => id.0 == 0,
                        Principal::Organization(_) => false,
                    }
            })
            || self.represented.keys().any(|a| a.0 == 0)
        {
            return Err("Invalid private relay authority table".into());
        }
        Ok(())
    }
}
fn policy(w: &World) -> Option<&Policy> {
    Some(
        &w.module_state::<PropagationStore>()?
            .transport
            .service
            .as_ref()?
            .access,
    )
}
/// Selection is only a preference; every source snapshots it after live authority
/// validation. Changing the preference cannot change already accepted traffic.
pub fn selected(w: &World, actor: ObjId) -> Result<Option<RelayRepresentation>, TransportRefusal> {
    let r = policy(w).and_then(|p| p.represented.get(&actor)).copied();
    if r.is_some_and(|r| {
        !service::integration(w).is_some_and(|api| api.representation_valid(w, actor, r))
    }) {
        return Err(TransportRefusal::Authority);
    }
    Ok(r)
}
pub fn principals(
    w: &World,
    actor: ObjId,
    representation: Option<RelayRepresentation>,
) -> Option<Vec<Principal>> {
    let api = service::integration(w)?;
    if let Some(r) = representation {
        if !api.representation_valid(w, actor, r) {
            return None;
        }
        Some(vec![
            Principal::Object(actor),
            Principal::Organization(r.organization),
        ])
    } else {
        Some(api.principals(w, actor))
    }
}
pub fn public(w: &World, id: &CanonicalId) -> bool {
    crate::routing::registry(w)
        .and_then(|r| r.pairs.get(id))
        .is_some_and(|p| {
            p.class_policy == mush_sim::relay::ClassPolicy::Exchange
                || policy(w).is_some_and(|a| a.public_carriage.contains(id))
        })
}
fn permitted(
    w: &World,
    id: &CanonicalId,
    actor: ObjId,
    representation: Option<RelayRepresentation>,
    right: Right,
) -> bool {
    let Some(api) = service::integration(w) else {
        return false;
    };
    let Some(principals) = principals(w, actor, representation) else {
        return false;
    };
    let capability = match right {
        Right::Discover => "relay.discover",
        Right::Use => "relay.use",
    };
    if api.authorized(w, actor, id, capability) {
        return true;
    }
    // Legal ownership is positive authority for the owning principal. A human
    // must explicitly and validly speak for that organization to exercise it.
    if api
        .operator(w, id)
        .is_some_and(|o| principals.contains(&Principal::Organization(o)))
    {
        return true;
    }
    policy(w).is_some_and(|p| {
        p.grants.iter().any(|g| {
            &g.pair == id && g.rights.contains(&right) && principals.contains(&g.principal)
        })
    })
}
pub fn discoverable(w: &World, actor: ObjId, id: &CanonicalId) -> bool {
    public(w, id)
        || selected(w, actor)
            .ok()
            .is_some_and(|r| permitted(w, id, actor, r, Right::Discover))
}
pub fn can_use(
    w: &World,
    id: &CanonicalId,
    actor: ObjId,
    representation: Option<RelayRepresentation>,
) -> bool {
    public(w, id) || permitted(w, id, actor, representation, Right::Use)
}
pub fn usable(
    w: &World,
    id: &CanonicalId,
    ends: (ObjId, ObjId),
    representation: Option<RelayRepresentation>,
) -> bool {
    public(w, id)
        || (permitted(w, id, ends.0, representation, Right::Use)
            && permitted(w, id, ends.1, None, Right::Use))
}
pub fn handles(ctx: &Ctx<'_>) -> bool {
    ["access", "network", "represent"]
        .iter()
        .any(|s| ctx.switches.has(s))
}
fn say(ctx: &mut Ctx<'_>, s: impl Into<String>) {
    ctx.out.sys_to_actor(ctx.actor, crate::NS, s.into());
}
pub fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let fail = |s: &str| CmdError::msg(s.to_owned());
    let api = service::integration(ctx.w).ok_or_else(|| fail("Relay service unavailable."))?;
    let mut state = service::current(ctx)
        .cloned()
        .ok_or_else(|| fail("Relay service unavailable."))?;
    let actor = ctx.actor;
    let at = ctx.w.world_ic_now().at;
    let left = ctx.args.lhs().trim().to_owned();
    if ctx.switches.has("represent") {
        if ctx.args.rhs().is_some() {
            return Err(fail("Usage: +relay/represent personal|org:id"));
        }
        if left.is_empty() {
            say(
                ctx,
                match state.access.represented.get(&actor) {
                    Some(r) => format!(
                        "Communication principal org:{}; scope {}. Authority is checked on every send and relay hop.",
                        r.organization, r.scope
                    ),
                    None => "Communication principal: personal.".into(),
                },
            );
            return Ok(());
        }
        if left == "personal" {
            state.access.represented.remove(&actor);
        } else {
            let org = left
                .strip_prefix("org:")
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| fail("Usage: +relay/represent personal|org:id"))?;
            let r = api
                .representation(ctx.w, actor, org)
                .ok_or_else(|| fail("Permission denied: relay.speak authority is required."))?;
            if state.access.represented.len() >= 128
                && !state.access.represented.contains_key(&actor)
            {
                return Err(fail("Representation table is full."));
            }
            state.access.represented.insert(actor, r);
        }
        // This actor's own selection is not part of any operator's audit view.
        service::save(ctx, state)?;
        say(
            ctx,
            "Communication principal selected; existing traffic is unchanged.",
        );
        return Ok(());
    }
    let (name, subject) = left
        .split_once('/')
        .map_or((left.as_str(), None), |(a, b)| (a, Some(b)));
    let id = service::pair(ctx, name)?;
    if !api.authorized(ctx.w, actor, &id, "relay.manage") {
        return Err(fail("Permission denied."));
    }
    if ctx.switches.has("network") {
        if subject.is_some() {
            return Err(fail("Usage: +relay/network pair=private|public"));
        }
        let public = match ctx.args.rhs().map(str::trim) {
            Some("public") => true,
            Some("private") => false,
            _ => return Err(fail("Usage: +relay/network pair=private|public")),
        };
        if crate::routing::registry(ctx.w)
            .and_then(|r| r.pairs.get(&id))
            .is_none_or(|p| p.class_policy != mush_sim::relay::ClassPolicy::Private)
        {
            return Err(fail(
                "Public Exchange links keep their chartered carriage policy.",
            ));
        }
        if public {
            if !api.authorized(ctx.w, actor, &id, "relay.publish") {
                return Err(fail(
                    "Permission denied: public carriage requires relay.publish authority.",
                ));
            }
            state.access.public_carriage.insert(id.clone());
        } else {
            state.access.public_carriage.remove(&id);
        }
        state.audit(
            actor,
            at,
            format!("network {id} {}", if public { "public" } else { "private" }),
        );
    } else {
        let principal = subject
            .and_then(Principal::parse)
            .filter(|p| api.valid_principal(ctx.w, p))
            .ok_or_else(|| {
                fail("Usage: +relay/access pair/#character|org:id=discover,use|discover|use|none")
            })?;
        let value = ctx.args.rhs().unwrap_or("").trim();
        let mut rights = BTreeSet::new();
        if value != "none" {
            for token in value.split(',') {
                rights.insert(match token.trim() {
                    "discover" => Right::Discover,
                    "use" => Right::Use,
                    _ => return Err(fail("Rights are discover, use, or none.")),
                });
            }
        }
        state
            .access
            .grants
            .retain(|g| g.pair != id || g.principal != principal);
        if !rights.is_empty() {
            if state.access.grants.len() >= 256 {
                return Err(fail("Private relay grant table is full."));
            }
            state.access.grants.push(Grant {
                pair: id.clone(),
                principal: principal.clone(),
                rights,
            });
        }
        state.audit(
            actor,
            at,
            format!("access {id} {} {value}", principal.label()),
        );
    }
    service::save(ctx, state)?;
    say(
        ctx,
        "Relay access policy recorded; queued traffic is rechecked before transmission.",
    );
    Ok(())
}
