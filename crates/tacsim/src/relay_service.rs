//! R3 composition: scoped IC operator authority and evidence publication.
use mush_core::{CanonicalId, Ctx, ObjId, Role, World, perm};
use mush_propagation::service::{Config, Evidence, Integration, Principal};
use serde::Deserialize;
pub const JSON: &str = include_str!("../content/relays/service-v1.json");
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Seed {
    policy: Config,
    scandal_title: String,
    scandal_summary: String,
}
fn seed() -> Result<Seed, String> {
    serde_json::from_str(JSON).map_err(|e| e.to_string())
}
#[derive(Debug)]
pub struct RelayService;
fn operator(world: &World, pair: &CanonicalId) -> Option<mush_org::OrgId> {
    let registry = mush_propagation::routing::registry(world)?;
    let link = registry.pairs.get(pair)?;
    let half = link.halves.first()?;
    Some(registry.nodes.get(&half.node)?.operator)
}
fn provenance(e: &Evidence) -> String {
    format!("relay.service.v1:{}:{}", e.receipt.pair, e.receipt.envelope)
}
impl Integration for RelayService {
    fn operator(&self, world: &World, pair: &CanonicalId) -> Option<u64> {
        operator(world, pair).map(|o| o.0)
    }
    fn sanction(&self, w: &World, operator: u64, principals: &[Principal]) -> Option<String> {
        let registry = mush_propagation::routing::registry(w)?;
        if operator != registry.operator.0 {
            return None;
        }
        registry
            .losses
            .values()
            .find(|loss| {
                loss.responsible
                    .is_some_and(|a| principals.contains(&Principal::Object(a)))
            })
            .map(|loss| {
                format!(
                    "{}: permanent Exchange anathema; Tycho cover voided (IC {}).",
                    loss.source(),
                    loss.at
                )
            })
    }
    fn valid_principal(&self, w: &World, p: &Principal) -> bool {
        match p {
            Principal::Object(id) => w
                .get(*id)
                .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING) && o.kind.is_actor()),
            Principal::Organization(id) => w
                .module_state::<mush_org::OrgStore>()
                .and_then(|s| s.state().org(mush_org::OrgId(*id)))
                .is_some(),
        }
    }
    fn principals(&self, w: &World, endpoint: ObjId) -> Vec<Principal> {
        let mut result = vec![Principal::Object(endpoint)];
        // A character speaks personally. A titled unit speaks for its legal owner;
        // ordinary membership, a builder's dbref owner, custody or a paying account
        // never permits claiming a different organization.
        if w.get(endpoint).is_some_and(|o| !o.kind.is_actor())
            && let Some(asset) = w
                .module_state::<mush_economy::EconomyStore>()
                .and_then(|s| s.snapshot().asset(endpoint))
        {
            let owner = match asset.title_owner {
                mush_org::PrincipalId::Organization(id) => Some(Principal::Organization(id.0)),
                mush_org::PrincipalId::Object(id) => Some(Principal::Object(id)),
                _ => None,
            };
            if let Some(owner) = owner
                && !result.contains(&owner)
            {
                result.push(owner);
            }
        }
        result
    }

    fn seed(&self) -> Result<Config, String> {
        Ok(seed()?.policy)
    }
    fn visible(&self, w: &World, actor: ObjId, r: &mush_propagation::service::Receipt) -> bool {
        use mush_propagation::record::DispatchKind;
        if r.recipient != actor || r.arrived.is_none() {
            return false;
        }
        let parts: Vec<_> = r.reference.as_deref().unwrap_or("").split(':').collect();
        match r.kind {
            DispatchKind::Mail => {
                let Some(id) = parts.get(2).and_then(|s| s.parse::<u64>().ok()) else {
                    return false;
                };
                let Some(mail) = w.module_state::<mush_mail::MailState>() else {
                    return false;
                };
                if parts.get(4) == Some(&"return") {
                    mail.message(id).is_some_and(|m| {
                        m.from == actor
                            && m.delivery
                                .as_ref()
                                .is_some_and(|d| d.contains_key(&r.issuer))
                    })
                } else {
                    mail.entry(actor, id).is_some()
                }
            }
            DispatchKind::Channel => {
                let Some(channel) = parts
                    .get(2)
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(mush_comsys::ChannelId)
                else {
                    return false;
                };
                let Some(seq) = parts.get(3).and_then(|s| s.parse::<u64>().ok()) else {
                    return false;
                };
                let Some(channels) = w.module_state::<mush_comsys::ComsysState>() else {
                    return false;
                };
                channels
                    .channels
                    .get(&channel)
                    .is_some_and(|c| c.hears(w, actor))
                    && channels
                        .memberships_of(actor)
                        .find(|m| m.channel == channel)
                        .and_then(|m| m.ic.as_ref())
                        .is_some_and(|k| k.received.iter().any(|r| r.message == seq))
            }
            _ => true,
        }
    }
    fn authorized(&self, w: &World, actor: ObjId, pair: &CanonicalId, action: &str) -> bool {
        let Some(org) = operator(w, pair) else {
            return false;
        };
        if perm::has_role(w, actor, Role::Wizard) {
            return true;
        }
        let Some(scope) = w
            .module_state::<mush_org::OrgStore>()
            .and_then(|s| s.state().org(org))
            .map(|o| o.root_scope)
        else {
            return false;
        };
        let Some(capability) = mush_org::CapabilityId::new(action) else {
            return false;
        };
        mush_org::authorize_in_world(
            w,
            &mush_org::AuthRequest {
                actor,
                capability,
                scope,
                now: w.now(),
                amount: None,
            },
        )
        .is_allow()
    }
    fn discover(&self, ctx: &mut Ctx<'_>, e: &Evidence) -> Result<(), String> {
        if e.holder != ctx.actor
            || !e.receipt.proof()
            || !self.authorized(ctx.w, ctx.actor, &e.receipt.pair, "relay.inspect")
        {
            return Err("Permission denied.".into());
        }
        if ctx
            .w
            .module_state::<mush_npc::NpcHostState>()
            .is_some_and(|s| {
                s.intel
                    .for_principal(mush_faction::Principal::Character(ctx.actor))
                    .iter()
                    .any(|r| {
                        r.kind == mush_npc::IntelKind::ServiceEvidence
                            && r.provenance == provenance(e)
                    })
            })
        {
            return Ok(());
        }
        let registry = mush_propagation::routing::registry(ctx.w).ok_or("Relay unavailable.")?;
        let node = registry
            .pairs
            .get(&e.receipt.pair)
            .and_then(|p| p.halves.first())
            .and_then(|h| registry.nodes.get(&h.node))
            .ok_or("Relay unavailable.")?;
        mush_npc::file_observation(
            ctx,
            mush_npc::IntelRecord {
                informant: ctx.actor,
                principal: mush_faction::Principal::Character(ctx.actor),
                subject: node.object,
                subject_name: node.label.clone(),
                kind: mush_npc::IntelKind::ServiceEvidence,
                detail: format!(
                    "{}: Standard queued at {}, free capacity observed at {}, transmitted at {}; declared wait {} IC seconds. No message content inspected.",
                    e.receipt.pair,
                    e.receipt.queued,
                    e.receipt.ticket.actual_ready.unwrap_or(e.receipt.queued),
                    e.receipt.transmitted,
                    e.receipt.ticket.declared_s
                ),
                room_name: "Operator service ledger".into(),
                provenance: provenance(e),
                ts: ctx.w.now(),
            },
        )
    }
    fn publish(&self, ctx: &mut Ctx<'_>, e: &Evidence) -> Result<u64, String> {
        if e.holder != ctx.actor
            || !e.receipt.proof()
            || !self.authorized(ctx.w, ctx.actor, &e.receipt.pair, "relay.publish")
        {
            return Err("Permission denied.".into());
        }
        let held = ctx
            .w
            .module_state::<mush_npc::NpcHostState>()
            .is_some_and(|s| {
                s.intel
                    .for_principal(mush_faction::Principal::Character(ctx.actor))
                    .iter()
                    .any(|r| {
                        r.kind == mush_npc::IntelKind::ServiceEvidence
                            && r.provenance == provenance(e)
                    })
            });
        if !held {
            return Err("The evidence is no longer in your knowledge ledger.".into());
        }
        let seed = seed()?;
        let org = operator(ctx.w, &e.receipt.pair).ok_or("Operator unavailable.")?;
        let draft = mush_canon::CanonDraft {
            kind: "relay.congestion_published".into(),
            summary: seed.scandal_summary,
            entities: vec![ctx.actor],
            scope: mush_canon::CanonScope::Public,
            ic_time: ctx.w.world_ic_now().at,
            source: format!("relay:congestion:{}", org.0),
            detail: serde_json::json!({"pair":e.receipt.pair,"queued":e.receipt.queued,"transmitted":e.receipt.transmitted,"capacity_available":e.receipt.ticket.actual_ready,"declared_wait":e.receipt.ticket.declared_s}),
        };
        mush_faction::publish_front(ctx, org, &seed.scandal_title, draft).map(|f| f.0)
    }
}
