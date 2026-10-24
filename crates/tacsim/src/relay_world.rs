//! R10: the public star's two original fronts share the existing Exchange/NPC.
//! A Terms case records at most two typed facts about ONE principal. Its clock is
//! history, never a second billing engine or an independent service restriction.
use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_propagation::service::{Principal, Restriction, ServiceState};
use serde::{Deserialize, Serialize};
pub const KEY: &str = "tacsim:relay_world";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub restriction: Restriction,
    pub canon: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Story {
    pub version: u8,
    pub front: mush_faction::FrontId,
    pub postmaster: ObjId,
    pub facts: Vec<Fact>,
}
impl Story {
    pub fn progress(&self) -> u8 {
        self.facts.last().map_or(0, |f| {
            if f.restriction.state == ServiceState::Suspended {
                4
            } else {
                2
            }
        })
    }
    pub fn check(&self) -> Result<(), String> {
        if self.version != 1 || self.postmaster.0 == 0 || self.facts.len() > 2 {
            return Err("Invalid Terms receipt census".into());
        }
        for (i, f) in self.facts.iter().enumerate() {
            let r = &f.restriction;
            if f.canon == 0
                || r.operator == 0
                || r.state == ServiceState::Served
                || r.source.as_str().len() > 48
                || r.reason.trim().is_empty()
                || r.reason.len() > 160
                || r.reason.chars().any(char::is_control)
                || matches!(
                    r.principal,
                    Principal::Object(ObjId(0)) | Principal::Organization(0)
                )
                || i == 1
                    && (r.state != ServiceState::Suspended
                        || self.facts[0].restriction.state != ServiceState::Throttled
                        || r.principal != self.facts[0].restriction.principal
                        || r.operator != self.facts[0].restriction.operator
                        || f.canon == self.facts[0].canon)
            {
                return Err("Invalid Terms fact or case identity".into());
            }
        }
        Ok(())
    }
}
pub fn current(w: &World) -> Option<&Story> {
    w.module_state::<crate::TacsimStore>()?.relay_world.as_ref()
}
fn full_star(w: &World) -> bool {
    let Ok(id) = CanonicalId::new("tacsim.relay.luna_ceres") else {
        return false;
    };
    w.module_state::<mush_content::ContentState>()
        .and_then(|s| s.latches.applied(&id))
        .is_some_and(|a| a.version() == 2 && !a.is_retired() && a.anomaly().is_none())
}
fn clock_ok(w: &World, s: &Story) -> Result<(), String> {
    let sim = w
        .module_state::<mush_faction::SimState>()
        .ok_or("Fronts unavailable")?;
    let front = sim.fronts.get(&s.front).ok_or("Terms front missing")?;
    let clocks: Vec<_> = sim.clocks.values().filter(|c| c.front == s.front).collect();
    if front.title != "Terms of Service"
        || clocks.len() != 1
        || clocks[0].segments != 4
        || clocks[0].filled != s.progress()
        || clocks[0].completed != (s.progress() == 4)
        || clocks[0].advance_on.is_some()
    {
        return Err("Terms front differs from attributed service receipts; repair required".into());
    }
    Ok(())
}
fn save(ctx: &mut Ctx<'_>, s: Story) -> Result<(), String> {
    let value = serde_json::to_value(&s).map_err(|e| e.to_string())?;
    if let Some(store) = ctx.w.module_state_mut::<crate::TacsimStore>() {
        store.relay_world = Some(s);
    }
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: KEY.into(),
            value,
        },
    );
    ctx.out.pin_deferred();
    Ok(())
}
fn draft(ctx: &Ctx<'_>, summary: String, detail: serde_json::Value) -> mush_canon::CanonDraft {
    mush_canon::CanonDraft {
        kind: "relay.terms".into(),
        summary,
        entities: vec![],
        scope: mush_canon::CanonScope::Public,
        ic_time: ctx.w.world_ic_now().at,
        source: "tacsim.relay_world".into(),
        detail,
    }
}
pub fn boot(ctx: &mut Ctx<'_>) -> Result<(), String> {
    if !full_star(ctx.w) {
        if current(ctx.w).is_some() {
            return Err("Terms receipts require the full public star".into());
        }
        return Ok(());
    }
    let registry = mush_propagation::routing::registry(ctx.w).ok_or("Relay catalog missing")?;
    let operator = registry.operator;
    let mobile = crate::relay_mobile::current(ctx.w)
        .ok_or("Fleet/Postmaster receipt missing")?
        .clone();
    if let Some(s) = current(ctx.w) {
        s.check()?;
        clock_ok(ctx.w, s)?;
        if s.postmaster != mobile.postmaster
            || s.facts.iter().any(|f| f.restriction.operator != operator.0)
        {
            return Err("Terms programme identity drift".into());
        }
    } else {
        if ctx
            .w
            .module_state::<crate::TacsimStore>()
            .is_some_and(|s| s.relay_world_held)
            || ctx
                .w
                .module_state::<mush_faction::SimState>()
                .is_some_and(|s| s.fronts.values().any(|f| f.title == "Terms of Service"))
        {
            return Err(
                "Terms receipt missing/unreadable beside existing story; refusing reseed".into(),
            );
        }
        let office = crate::port::apply::room_by_key(
            ctx.w,
            &mush_atlas::record::RecordKey {
                pack: CanonicalId::new("neighborhood.malapert_exchange")
                    .map_err(|e| e.to_string())?,
                entry: CanonicalId::new("room.office").map_err(|e| e.to_string())?,
            },
        )
        .ok_or("Postmaster office missing")?;
        let old = crate::port::coop::programme_ports(ctx.w)
            .into_iter()
            .find(|p| p.programme.id.as_str() == "concord.relief")
            .and_then(|p| p.map.presentation)
            .map(ObjId);
        let front=mush_faction::seed_relay_terms_front(ctx,operator,draft(ctx,
            "Terms of Service: the Exchange's first named arrears or incident case awaits a real service decision.".into(),serde_json::json!({"operator":operator})))?;
        // Relocate once from the old authored gallery; preserve a deliberately moved NPC.
        if ctx.w.get(mobile.postmaster).and_then(|o| o.location) == old {
            ctx.w
                .move_to(mobile.postmaster, office)
                .map_err(|e| e.to_string())?;
        }
        save(
            ctx,
            Story {
                version: 1,
                front,
                postmaster: mobile.postmaster,
                facts: vec![],
            },
        )?;
    }
    // Existing authorized incident policies also survive an old-world upgrade.
    let facts = ctx
        .w
        .module_state::<mush_propagation::PropagationStore>()
        .and_then(|s| s.relay_transport().service.as_ref())
        .map(|s| s.restrictions.clone())
        .unwrap_or_default();
    for fact in facts {
        observe(ctx, &fact)?;
    }
    tracing::info!(
        "relay world ready: Luna star v2; five public nodes, four pairs; Terms of Service and A Half Afloat; +relay (board and tariffs), +clocks"
    );
    Ok(())
}
pub fn observe(ctx: &mut Ctx<'_>, r: &Restriction) -> Result<(), String> {
    let Some(mut s) = current(ctx.w).cloned() else {
        return Ok(());
    };
    let operator = mush_propagation::routing::registry(ctx.w)
        .ok_or("Relay catalog missing")?
        .operator
        .0;
    if r.operator != operator
        || r.state == ServiceState::Served
        || s.progress() == 4
        || s.facts.first().is_some_and(|f| {
            f.restriction.principal != r.principal || r.state == ServiceState::Throttled
        })
    {
        return Ok(());
    }
    s.check()?;
    clock_ok(ctx.w, &s)?;
    let expected = s.progress();
    s.facts.push(Fact {
        restriction: r.clone(),
        canon: u64::MAX,
    });
    s.check()?;
    let next = s.progress();
    let summary = format!(
        "Terms of Service: {} is {:?}; {}: {}.",
        r.principal.label(),
        r.state,
        r.source,
        r.reason
    );
    let canon = mush_faction::advance_relay_terms_front(
        ctx,
        s.front,
        expected,
        next,
        draft(ctx, summary, serde_json::json!(r)),
    )?;
    if let Some(fact) = s.facts.last_mut() {
        fact.canon = canon;
    }
    save(ctx, s)
}
pub fn subscription(
    ctx: &mut Ctx<'_>,
    s: &mush_propagation::billing::Subscription,
    v: &mush_propagation::billing::View,
) -> Result<(), String> {
    // Cancellation ends collection, not an existing service debt. Use the same
    // money-owned restriction projection as routing, including terminal debt.
    if s.obligation.is_none() {
        return Ok(());
    }
    if v.cured > v.missed {
        return Err("Invalid subscription cure census".into());
    }
    let Some(registry) = mush_propagation::routing::registry(ctx.w) else {
        return Ok(());
    };
    let fact = Restriction {
        operator: registry.operator.0,
        principal: s.principal.clone(),
        source: CanonicalId::new(format!("billing.subscription.{}", s.id))
            .map_err(|e| e.to_string())?,
        state: v.state(),
        reason: format!(
            "{} unpaid installment(s); obligation {}{}",
            v.missed - v.cured,
            s.obligation.unwrap_or(0),
            if v.paused { "; collection paused" } else { "" }
        ),
        actor: s.issuer,
        at: ctx.w.world_ic_now().at,
    };
    observe(ctx, &fact)
}
