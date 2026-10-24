//! R7: persistent tactical structure is the source of relay availability.
use mush_core::{Ctx, ModError, Registrar, WorldEvent, WorldEventKind};
use mush_sim::relay::{NodeLoss, RelayStore};
use mush_tactics::{CellAddr, CellId, SectionId};
use std::sync::Arc;

pub const STATION: &str = "tacsim.unit.relay_station";

/// R1 upgrades attach a checked station once. Existing or held records are never
/// replaced: a wrong definition is a repairable boot error, not a healthy reseed.
pub fn ensure_units(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let Some(r) = ctx
        .w
        .module_state::<RelayStore>()
        .and_then(RelayStore::registry)
    else {
        return Ok(());
    };
    let migrated = r.structural_version == 1;
    let objects: Vec<_> = r.nodes.values().map(|n| n.object).collect();
    let catalog = crate::builtin_definitions()?;
    let spec = catalog
        .units
        .get(STATION)
        .ok_or("relay station definition missing")?;
    for object in objects {
        if let Some(unit) = mush_tactics::module::unit_of(ctx.w, object) {
            if unit.schema() != &spec.definition {
                return Err(format!(
                    "relay node {object} has a different structural definition"
                ));
            }
        } else {
            if migrated {
                return Err(format!(
                    "relay node {object}: migrated structure missing or held; repair required"
                ));
            }
            mush_tactics::module::create_unit(ctx, object, spec)?;
        }
    }
    if !migrated {
        let mut r = ctx
            .w
            .module_state::<RelayStore>()
            .and_then(RelayStore::registry)
            .cloned()
            .ok_or("relay missing")?;
        r.structural_version = 1;
        mush_sim::relay::install(ctx, r)?;
    }
    Ok(())
}

fn internals(unit: &mush_tactics::TacticalUnit, section: &str) -> Option<i64> {
    unit.cell(&CellAddr::Section {
        section: SectionId::new(section).ok()?,
        cell: CellId::new("internals").ok()?,
    })
    .map(|c| c.current)
}

fn publish_pending(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let pending: Vec<_> = ctx
        .w
        .module_state::<RelayStore>()
        .and_then(RelayStore::registry)
        .into_iter()
        .flat_map(|r| &r.losses)
        .filter(|(_, l)| l.canon.is_none())
        .map(|(id, l)| (id.clone(), l.clone()))
        .collect();
    for (node, loss) in pending {
        // Consumers read the terminal incident already installed on this batch.
        let mut entities = loss.responsible.into_iter().collect::<Vec<_>>();
        entities.push(loss.object);
        mush_faction::condemn_relay_loss(ctx, loss.object, loss.responsible, &loss.source())?;
        let canon = mush_canon::commit(ctx, mush_canon::CanonDraft {
            kind: "relay.node_destroyed".into(),
            summary: format!("Relay node {node} destroyed; its remote halves are orphaned. {}",
                loss.responsible.map_or_else(|| "Attribution unresolved.".into(), |a| format!("{a} is condemned; the Exchange declares permanent anathema and Tycho voids cover."))),
            entities,
            scope: mush_canon::CanonScope::Public,
            ic_time: loss.at,
            source: loss.source(),
            detail: serde_json::json!({"node":node,"object":loss.object,"responsible":loss.responsible,"revision":loss.revision}),
        }).ok_or("relay loss canon unavailable; terminal outage retained for boot retry")?;
        let mut registry = ctx
            .w
            .module_state::<RelayStore>()
            .and_then(RelayStore::registry)
            .cloned()
            .ok_or("relay catalog missing")?;
        if let Some(record) = registry.losses.get_mut(&node) {
            record.canon = Some(canon.0);
        }
        mush_sim::relay::install(ctx, registry)?;
        ctx.out.pin_deferred();
    }
    Ok(())
}

fn changed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::TacticalUnitChanged {
        unit: object,
        revision,
        responsible,
    } = ev
    else {
        return;
    };
    if let Err(error) = reconcile(ctx, *object, *revision, *responsible) {
        tracing::error!(%error, %object, "relay structural reconciliation failed");
    }
}

fn reconcile(
    ctx: &mut Ctx<'_>,
    object: mush_core::ObjId,
    revision: u64,
    responsible: Option<mush_core::ObjId>,
) -> Result<(), String> {
    let Some(mut registry) = ctx
        .w
        .module_state::<RelayStore>()
        .and_then(RelayStore::registry)
        .cloned()
    else {
        return Ok(());
    };
    let Some(node) = registry
        .nodes
        .iter()
        .find(|(_, n)| n.object == object)
        .map(|(id, _)| id.clone())
    else {
        return Ok(());
    };
    let unit = mush_tactics::module::unit_of(ctx.w, object).ok_or("relay structure unavailable")?;
    if unit.revision() != revision {
        return Err("stale structural event".into());
    }
    let core = internals(unit, "core").ok_or("relay core missing")?;
    let power = internals(unit, "power").ok_or("relay power missing")?;
    let dead = core == 0 || registry.losses.contains_key(&node);
    let mut changed = false;
    for half in registry
        .pairs
        .values_mut()
        .flat_map(|p| &mut p.halves)
        .filter(|h| h.node == node)
    {
        let before = (half.destroyed, half.damaged);
        half.destroyed |= dead;
        half.damaged = power == 0;
        changed |= before != (half.destroyed, half.damaged);
    }
    if dead && !registry.losses.contains_key(&node) {
        let responsible = responsible.filter(|a| ctx.w.get(*a).is_some_and(|o| o.kind.is_actor()));
        registry.losses.insert(
            node,
            NodeLoss {
                object,
                revision,
                responsible,
                at: ctx.w.world_ic_now().at,
                canon: None,
            },
        );
        changed = true;
    }
    if changed {
        registry.revision = registry
            .revision
            .checked_add(1)
            .ok_or("relay revisions exhausted")?;
        mush_sim::relay::install(ctx, registry)?;
    }
    publish_pending(ctx)
}

pub fn boot(ctx: &mut Ctx<'_>) -> Result<(), String> {
    ensure_units(ctx)?;
    let objects: Vec<_> = ctx
        .w
        .module_state::<RelayStore>()
        .and_then(RelayStore::registry)
        .into_iter()
        .flat_map(|r| r.nodes.values())
        .map(|n| n.object)
        .collect();
    for object in objects {
        let revision = mush_tactics::module::unit_of(ctx.w, object)
            .ok_or("relay unit missing")?
            .revision();
        reconcile(ctx, object, revision, None)?;
    }
    publish_pending(ctx)
}

pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.persist_delegation(
        crate::NS,
        "sim",
        "R7 tactical post-images commit physical relay loss through its owner API",
    )?;
    reg.subscribe(WorldEventKind::TacticalUnitChanged, Arc::new(changed));
    Ok(())
}
