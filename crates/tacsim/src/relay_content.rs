//! Checked Luna star and narrow v1 migration. The sim owner retains topology.

use mush_atlas::record::RecordKey;
use mush_content::{EntryApplier, Pack, RawEntry};
use mush_core::{BehaviorBinding, BehaviorId, CanonicalId, Ctx, ObjId, ObjKind};
use mush_org::{
    OrgAction, OrgMutationOp, OrgMutationPlan, OrgScope, OrgStatus, OrgStore, Organization,
    RulesValue, ScopeStatus,
};
use mush_sim::relay::{
    ClassPolicy, HALF_BEHAVIOR, NODE_BEHAVIOR, RelayHalf, RelayNode, RelayPair, RelayRegistry,
    RelayStore,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub const JSON: &str = include_str!("../content/relays/luna_ceres.json");
pub const KIND: &str = "relay_network";

pub const HISTORICAL_JSON: &str = include_str!("../content/relays/luna_ceres-v1.json");
pub const CHARTER: &str = "Concord-chartered neutral common carrier under the 2312 Settlement. The Exchange staffs the Luna star, publishes tariffs and records a named principal and reason for every service restriction. Independent sanctions survive an arrears cure.";
pub const LEGACY_CHARTER: &str =
    "Concord-chartered relay operator; R1 installs topology, not paid service.";
const GEOGRAPHY: [&str; 6] = [
    include_str!("../content/atlas/region.kirkwood_depot.json"),
    include_str!("../content/atlas/region.mercury_relay.json"),
    include_str!("../content/atlas/neighborhood.malapert_exchange.json"),
    include_str!("../content/atlas/neighborhood.phobos_exchange.json"),
    include_str!("../content/atlas/neighborhood.kirkwood_depot.json"),
    include_str!("../content/atlas/neighborhood.mercury_relay.json"),
];
pub fn packs() -> Result<Vec<Pack>, String> {
    GEOGRAPHY
        .into_iter()
        .chain([JSON])
        .map(|json| mush_content::load_pack(json).map_err(|e| format!("relay pack: {e}")))
        .collect()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeDef {
    label: String,
    place: RecordKey,
    coverage: Vec<RecordKey>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HalfDef {
    id: CanonicalId,
    node: CanonicalId,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairDef {
    halves: [HalfDef; 2],
    capacity_units_per_ic_second: u32,
    class_policy: ClassPolicy,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkDef {
    operator_name: String,
    nodes: BTreeMap<CanonicalId, NodeDef>,
    pairs: BTreeMap<CanonicalId, PairDef>,
}

fn room(ctx: &Ctx<'_>, key: &RecordKey) -> Result<ObjId, String> {
    crate::port::apply::room_by_key(ctx.w, key)
        .ok_or_else(|| format!("relay: unresolved room {key}"))
}

fn definition(ctx: &Ctx<'_>, entry: &RawEntry) -> Result<NetworkDef, String> {
    let def: NetworkDef = entry.decode().map_err(|e| e.to_string())?;
    if def.operator_name.is_empty()
        || def.operator_name.len() > 80
        || def.nodes.is_empty()
        || def.nodes.len() > mush_sim::relay::MAX_NODES
        || def.pairs.is_empty()
        || def.pairs.len() > mush_sim::relay::MAX_PAIRS
    {
        return Err("relay: invalid operator or node/pair census".into());
    }
    let mut halves = BTreeSet::new();
    for node in def.nodes.values() {
        if node.label.is_empty()
            || node.label.len() > 128
            || node.coverage.is_empty()
            || node.coverage.len() > mush_sim::relay::MAX_COVERAGE
            || node.coverage.iter().collect::<BTreeSet<_>>().len() != node.coverage.len()
            || !node.coverage.contains(&node.place)
        {
            return Err("relay: invalid node label or coverage".into());
        }
        room(ctx, &node.place)?;
        for key in &node.coverage {
            room(ctx, key)?;
        }
    }
    for pair in def.pairs.values() {
        if pair.capacity_units_per_ic_second == 0
            || pair.capacity_units_per_ic_second > 1_000_000
            || pair.halves[0].node == pair.halves[1].node
        {
            return Err("relay: invalid capacity or identical endpoints".into());
        }
        for half in &pair.halves {
            if !def.nodes.contains_key(&half.node) || !halves.insert(&half.id) {
                return Err("relay: missing node or duplicate half identity".into());
            }
        }
    }
    Ok(def)
}

fn bindings(ctx: &Ctx<'_>, registry: &RelayRegistry, def: &NetworkDef) -> Result<(), String> {
    for (id, authored) in &def.nodes {
        let live = registry.nodes.get(id).ok_or("relay: node identity drift")?;
        let coverage: Result<Vec<_>, _> = authored.coverage.iter().map(|k| room(ctx, k)).collect();
        if live.operator != registry.operator
            || live.label != authored.label
            || live.place != room(ctx, &authored.place)?
            || live.coverage != coverage?
        {
            return Err(format!("relay: node {id} binding/coverage drift"));
        }
    }
    for (id, authored) in &def.pairs {
        let live = registry.pairs.get(id).ok_or("relay: pair identity drift")?;
        if live.capacity_units_per_ic_second != authored.capacity_units_per_ic_second
            || live.class_policy != authored.class_policy
            || live
                .halves
                .iter()
                .zip(&authored.halves)
                .any(|(a, b)| a.id != b.id || a.node != b.node)
        {
            return Err(format!("relay: pair {id} definition drift"));
        }
    }
    Ok(())
}

/// Recognize only the exact old authored slice or the complete new topology.
/// Private extras and physical loss are retained; partial public expansion refuses.
fn preflight(
    ctx: &Ctx<'_>,
    pack: &Pack,
    id: &CanonicalId,
    entry: &RawEntry,
) -> Result<(NetworkDef, bool), String> {
    mush_sim::relay::check_installer(ctx)?;
    let def = definition(ctx, entry)?;
    let store = ctx
        .w
        .module_state::<RelayStore>()
        .ok_or("relay: simulation not composed")?;
    let orgs = ctx
        .w
        .module_state::<OrgStore>()
        .ok_or("relay: organizations not composed")?;
    let mut upgrade = false;
    let mut objects = def.nodes.len() + 2 * def.pairs.len();
    if let Some(r) = store.registry() {
        r.reconcile(ctx.w)?;
        let org = orgs
            .state()
            .org(r.operator)
            .ok_or("relay: operator absent")?;
        if &r.source_pack != pack.manifest().id()
            || &r.source_entry != id
            || org.name != def.operator_name
        {
            return Err("relay: existing catalog provenance/operator differs".into());
        }
        if bindings(ctx, r, &def).is_ok() {
            return Ok((def, false));
        }
        if pack.manifest().version() != 2 {
            return Err("relay: authored topology drift; repair required".into());
        }
        let old = mush_content::load_pack(HISTORICAL_JSON).map_err(|e| e.to_string())?;
        let old = definition(
            ctx,
            old.entry(id).ok_or("relay: historical network absent")?,
        )?;
        bindings(ctx, r, &old)?;
        let added_nodes: Vec<_> = def
            .nodes
            .keys()
            .filter(|n| !old.nodes.contains_key(*n))
            .collect();
        let added_pairs: Vec<_> = def
            .pairs
            .keys()
            .filter(|p| !old.pairs.contains_key(*p))
            .collect();
        if added_nodes.iter().any(|n| r.nodes.contains_key(*n))
            || added_pairs.iter().any(|p| r.pairs.contains_key(*p))
            || r.nodes.len() + added_nodes.len() > mush_sim::relay::MAX_NODES
            || r.pairs.len() + added_pairs.len() > mush_sim::relay::MAX_PAIRS
        {
            return Err(
                "relay: partial/conflicting public expansion or full catalog; repair required"
                    .into(),
            );
        }
        let halves: BTreeSet<_> = r
            .pairs
            .values()
            .flat_map(|p| p.halves.iter().map(|h| &h.id))
            .collect();
        if added_pairs
            .iter()
            .any(|p| def.pairs[*p].halves.iter().any(|h| halves.contains(&h.id)))
        {
            return Err("relay: new half identity collides with existing machinery".into());
        }
        let catalog = crate::builtin_definitions()?;
        let spec = catalog
            .units
            .get(crate::relay_loss::STATION)
            .ok_or("relay station definition absent")?;
        for (id, n) in &r.nodes {
            let unit = mush_tactics::module::unit_of(ctx.w, n.object)
                .ok_or("relay: existing structure missing; repair required")?;
            if unit.schema() != &spec.definition
                || crate::relay_loss::internals(unit, "core").is_none()
                || crate::relay_loss::internals(unit, "power").is_none()
                || crate::relay_loss::internals(unit, "core") == Some(0)
                    && !r.losses.contains_key(id)
            {
                return Err(
                    "relay: existing structure/loss record differs; repair required".into(),
                );
            }
        }
        r.revision
            .checked_add(1)
            .ok_or("relay revisions exhausted")?;
        if org.charter == LEGACY_CHARTER {
            org.revision
                .checked_add(1)
                .ok_or("organization revisions exhausted")?;
        }
        objects = added_nodes.len() + 2 * added_pairs.len();
        upgrade = true;
    } else if orgs.state().orgs().any(|o| o.name == def.operator_name) {
        return Err("relay: operator exists without relay catalog; repair, do not reseed".into());
    }
    ctx.w
        .next_id()
        .checked_add(u32::try_from(objects).map_err(|e| e.to_string())?)
        .ok_or("relay: object ids exhausted")?;
    Ok((def, upgrade))
}

fn expand(ctx: &mut Ctx<'_>, mut r: RelayRegistry, def: &NetworkDef) -> Result<(), String> {
    let catalog = crate::builtin_definitions()?;
    let spec = catalog
        .units
        .get(crate::relay_loss::STATION)
        .ok_or("relay station definition absent")?;
    // Create only the genuinely new station structures. The old structural latch
    // stays set, so a missing old structure can never be silently re-created.
    for (id, authored) in &def.nodes {
        if r.nodes.contains_key(id) {
            continue;
        }
        let place = room(ctx, &authored.place)?;
        let coverage = authored
            .coverage
            .iter()
            .map(|k| room(ctx, k))
            .collect::<Result<_, _>>()?;
        let obj = object(ctx, &authored.label, place, NODE_BEHAVIOR)?;
        mush_tactics::module::create_unit(ctx, obj, spec)?;
        r.nodes.insert(
            id.clone(),
            RelayNode {
                object: obj,
                place,
                label: authored.label.clone(),
                operator: r.operator,
                coverage,
            },
        );
    }
    for (id, authored) in &def.nodes {
        let n = r.nodes.get_mut(id).ok_or("relay node absent")?;
        let place = room(ctx, &authored.place)?;
        if n.place != place {
            ctx.w.move_to(n.object, place).map_err(|e| e.to_string())?;
            n.place = place;
        }
        n.coverage = authored
            .coverage
            .iter()
            .map(|k| room(ctx, k))
            .collect::<Result<_, _>>()?;
    }
    for (id, authored) in &def.pairs {
        if r.pairs.contains_key(id) {
            continue;
        }
        let mut halves = Vec::new();
        for half in &authored.halves {
            let node = r.nodes.get(&half.node).ok_or("relay node absent")?;
            let obj = object(
                ctx,
                &format!("Relay half {}", half.id),
                node.object,
                HALF_BEHAVIOR,
            )?;
            let unit = mush_tactics::module::unit_of(ctx.w, node.object)
                .ok_or("relay structure absent")?;
            // A lost or unpowered hub does not acquire healthy new spokes during
            // the gap between content apply and the next boot reconciliation.
            let destroyed = r.losses.contains_key(&half.node)
                || crate::relay_loss::internals(unit, "core") == Some(0);
            let damaged = crate::relay_loss::internals(unit, "power") == Some(0);
            halves.push(RelayHalf {
                id: half.id.clone(),
                object: obj,
                node: half.node.clone(),
                enabled: true,
                powered: true,
                staffed: true,
                destroyed,
                damaged,
            });
        }
        r.pairs.insert(
            id.clone(),
            RelayPair {
                halves: halves
                    .try_into()
                    .map_err(|_| "relay pair needs two halves")?,
                capacity_units_per_ic_second: authored.capacity_units_per_ic_second,
                class_policy: authored.class_policy,
            },
        );
    }
    let org = ctx
        .w
        .module_state::<OrgStore>()
        .and_then(|s| s.state().org(r.operator))
        .ok_or("relay operator absent")?;
    if org.charter == LEGACY_CHARTER {
        mush_org::submit(
            ctx,
            &OrgMutationPlan::of(vec![OrgMutationOp::ReviseCharter {
                org: r.operator,
                expected: LEGACY_CHARTER.into(),
                charter: CHARTER.into(),
            }]),
            OrgAction {
                action: "org.charter.revise".into(),
                org: Some(r.operator),
                detail: serde_json::json!({"source":"relay.full_star.v2"}),
            },
        )
        .map_err(|e| e.to_string())?;
    }
    r.revision = r
        .revision
        .checked_add(1)
        .ok_or("relay revisions exhausted")?;
    mush_sim::relay::install(ctx, r)?;
    ctx.out.pin_deferred();
    Ok(())
}

fn object(ctx: &mut Ctx<'_>, label: &str, place: ObjId, behavior: &str) -> Result<ObjId, String> {
    let obj = ctx
        .w
        .create(ObjKind::Thing, label, mush_core::GOD, Some(place))
        .map_err(|e| e.to_string())?;
    let binding = BehaviorBinding {
        schema_version: 1,
        attached_by: ctx.actor,
        attached_at: ctx.w.now(),
    };
    ctx.w
        .attach_behavior(
            obj,
            BehaviorId::new(behavior).map_err(|e| e.to_string())?,
            binding,
        )
        .map_err(|e| e.to_string())?;
    Ok(obj)
}

pub struct RelayApplier;

impl RelayApplier {
    fn put(
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
    ) -> Result<Option<ObjId>, String> {
        let (def, upgrade) = preflight(ctx, pack, id, entry)?;
        if let Some(registry) = ctx
            .w
            .module_state::<RelayStore>()
            .and_then(RelayStore::registry)
            .cloned()
        {
            if upgrade {
                expand(ctx, registry, &def)?;
            }
            crate::relay_loss::ensure_units(ctx)?;
            return Ok(None);
        }
        let (operator, root_id) = {
            let store = ctx
                .w
                .module_state_mut::<OrgStore>()
                .ok_or("org store missing")?;
            (store.alloc_org(), store.alloc_scope())
        };
        let org = Organization {
            id: operator,
            name: def.operator_name.clone().into(),
            aliases: Vec::new(),
            status: OrgStatus::Active,
            hidden: false,
            charter: if pack.manifest().version() == 1 {
                LEGACY_CHARTER
            } else {
                CHARTER
            }
            .into(),
            root_scope: root_id,
            revision: 1,
            ruleset_ext: RulesValue::default(),
        };
        let root = OrgScope {
            id: root_id,
            org: operator,
            parent: None,
            name: "root".into(),
            status: ScopeStatus::Active,
            depth: 0,
            created_rev: 1,
            revision: 1,
        };
        mush_org::submit(ctx, &OrgMutationPlan::of(vec![OrgMutationOp::CreateOrg { org, root }]),
            OrgAction { action: "org.create".into(), org: Some(operator), detail: serde_json::json!({"relay_pack": pack.manifest().id(), "name": def.operator_name}) })
            .map_err(|e| e.to_string())?;
        let mut nodes = BTreeMap::new();
        for (node_id, authored) in &def.nodes {
            let place = room(ctx, &authored.place)?;
            let coverage: Result<Vec<_>, _> =
                authored.coverage.iter().map(|k| room(ctx, k)).collect();
            let object = object(ctx, &authored.label, place, NODE_BEHAVIOR)?;
            nodes.insert(
                node_id.clone(),
                RelayNode {
                    object,
                    place,
                    label: authored.label.clone(),
                    operator,
                    coverage: coverage?,
                },
            );
        }
        let mut pairs = BTreeMap::new();
        for (pair_id, authored) in &def.pairs {
            let mut halves = Vec::new();
            for half in &authored.halves {
                let node = nodes.get(&half.node).ok_or("missing node")?;
                let object = object(
                    ctx,
                    &format!("Relay half {}", half.id),
                    node.object,
                    HALF_BEHAVIOR,
                )?;
                halves.push(RelayHalf {
                    id: half.id.clone(),
                    object,
                    node: half.node.clone(),
                    enabled: true,
                    powered: true,
                    staffed: true,
                    destroyed: false,
                    damaged: false,
                });
            }
            let halves: [RelayHalf; 2] =
                halves.try_into().map_err(|_| "pair must have two halves")?;
            pairs.insert(
                pair_id.clone(),
                RelayPair {
                    halves,
                    capacity_units_per_ic_second: authored.capacity_units_per_ic_second,
                    class_policy: authored.class_policy,
                },
            );
        }
        mush_sim::relay::install(
            ctx,
            RelayRegistry {
                version: 1,
                revision: 1,
                source_pack: pack.manifest().id().clone(),
                source_entry: id.clone(),
                operator,
                nodes,
                pairs,
                losses: BTreeMap::new(),
                structural_version: 0,
            },
        )?;
        crate::relay_loss::ensure_units(ctx)?;
        Ok(None)
    }
}

impl EntryApplier for RelayApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        CanonicalId::new(KIND).into_iter().collect()
    }
    fn phase(&self) -> u8 {
        12
    }
    fn check(
        &self,
        ctx: &Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
    ) -> Result<(), String> {
        preflight(ctx, pack, id, entry).map(|_| ())
    }
    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        _: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        Self::put(ctx, pack, id, entry)
    }
    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        _: Option<ObjId>,
        _: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        Self::put(ctx, pack, id, entry)
    }
    fn retire_check(
        &self,
        _: &Ctx<'_>,
        _: &CanonicalId,
        _: &CanonicalId,
        _: Option<ObjId>,
        _: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        Err("Installed relay topology cannot be retired implicitly; disable its halves with @relay.".into())
    }
    fn retire(
        &self,
        _: &mut Ctx<'_>,
        _: &CanonicalId,
        _: &CanonicalId,
        _: Option<ObjId>,
    ) -> Result<(), String> {
        Err("Installed relay topology cannot be retired implicitly.".into())
    }
}
