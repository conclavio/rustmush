//! R1's small real content slice. The sim module owns the durable topology.

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

pub fn packs() -> Result<Vec<Pack>, String> {
    Ok(vec![
        mush_content::load_pack(JSON).map_err(|e| format!("relay pack: {e}"))?,
    ])
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

/// All content refusals run before objects or an organization are created.
fn preflight(
    ctx: &Ctx<'_>,
    pack: &Pack,
    id: &CanonicalId,
    entry: &RawEntry,
) -> Result<NetworkDef, String> {
    let def = definition(ctx, entry)?;
    let store = ctx
        .w
        .module_state::<RelayStore>()
        .ok_or("relay: simulation not composed")?;
    let orgs = ctx
        .w
        .module_state::<OrgStore>()
        .ok_or("relay: organizations not composed")?;
    if let Some(registry) = store.registry() {
        registry.reconcile(ctx.w)?;
        if &registry.source_pack != pack.manifest().id()
            || &registry.source_entry != id
            || registry.nodes.len() != def.nodes.len()
            || registry.pairs.len() != def.pairs.len()
            || orgs
                .state()
                .org(registry.operator)
                .is_none_or(|o| o.name != def.operator_name)
        {
            return Err("relay: existing catalog provenance/operator/topology differs; explicit migration required".into());
        }
        for (id, authored) in &def.nodes {
            let live = registry.nodes.get(id).ok_or("relay: node identity drift")?;
            let coverage: Result<Vec<_>, _> =
                authored.coverage.iter().map(|k| room(ctx, k)).collect();
            if live.label != authored.label
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
    } else {
        if orgs.state().orgs().any(|o| o.name == def.operator_name) {
            return Err(
                "relay: operator exists without relay catalog; repair, do not reseed".into(),
            );
        }
        let count =
            u32::try_from(def.nodes.len() + 2 * def.pairs.len()).map_err(|e| e.to_string())?;
        ctx.w
            .next_id()
            .checked_add(count)
            .ok_or("relay: object ids exhausted")?;
    }
    Ok(def)
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
        let def = preflight(ctx, pack, id, entry)?;
        if ctx
            .w
            .module_state::<RelayStore>()
            .and_then(RelayStore::registry)
            .is_some()
        {
            // Reapply never resets availability or creates a second identity.
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
            charter: "Concord-chartered relay operator; R1 installs topology, not paid service."
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
            },
        )?;
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
