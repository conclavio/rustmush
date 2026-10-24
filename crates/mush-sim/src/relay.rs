//! Bounded relay topology and operator availability. Propagation owns routing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::{
    ArgShape, BehaviorId, BehaviorSpec, CanonicalId, CmdError, CommandCompatibility, CommandSpec,
    Ctx, DestroyPolicy, DetachGate, EventHook, Flags, HookPoint, HookReadContext, ModError,
    ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, ObjId, ObjKind, Registrar, Registry, Role,
    World,
};
use mush_org::{OrgId, OrgStore};
use mush_proto::RichText;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// One bounded catalog, atomically persisted with its content-created objects.
pub const KEY: &str = "relay:registry";
/// Hard bounds apply at load, content installation and every mutation.
pub const MAX_NODES: usize = 32;
pub const MAX_PAIRS: usize = 64;
pub const MAX_COVERAGE: usize = 64;
pub const MAX_BYTES: usize = 128 * 1024;
pub const NODE_BEHAVIOR: &str = "sim.relay_node";
pub const HALF_BEHAVIOR: &str = "sim.relay_half";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayNode {
    pub object: ObjId,
    pub place: ObjId,
    pub label: String,
    pub operator: OrgId,
    pub coverage: Vec<ObjId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayHalf {
    pub id: CanonicalId,
    pub object: ObjId,
    pub node: CanonicalId,
    pub enabled: bool,
    pub powered: bool,
    pub staffed: bool,
    /// Reserved terminal physical state. R1 has no destruction command.
    pub destroyed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassPolicy {
    Exchange,
    Private,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayPair {
    pub halves: [RelayHalf; 2],
    pub capacity_units_per_ic_second: u32,
    pub class_policy: ClassPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkState {
    Live,
    Degraded,
    Dead,
}

impl RelayPair {
    /// Derived from the two halves; there is no competing link-state field.
    #[must_use]
    pub fn state(&self) -> LinkState {
        if self.halves.iter().any(|h| h.destroyed) {
            LinkState::Dead
        } else if self
            .halves
            .iter()
            .all(|h| h.enabled && h.powered && h.staffed)
        {
            LinkState::Live
        } else {
            LinkState::Degraded
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayRegistry {
    pub version: u32,
    pub revision: u64,
    pub source_pack: CanonicalId,
    pub source_entry: CanonicalId,
    pub operator: OrgId,
    pub nodes: BTreeMap<CanonicalId, RelayNode>,
    pub pairs: BTreeMap<CanonicalId, RelayPair>,
}

impl RelayRegistry {
    /// Shape checks run before any reference walk or insertion.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.revision == 0 {
            return Err("unsupported version or zero revision".into());
        }
        if self.nodes.is_empty()
            || self.nodes.len() > MAX_NODES
            || self.pairs.is_empty()
            || self.pairs.len() > MAX_PAIRS
        {
            return Err("relay node/pair census is outside its bounds".into());
        }
        let mut objects = BTreeSet::new();
        let mut halves = BTreeSet::new();
        for node in self.nodes.values() {
            if node.label.is_empty()
                || node.label.len() > 128
                || node.coverage.is_empty()
                || node.coverage.len() > MAX_COVERAGE
                || node.coverage.iter().collect::<BTreeSet<_>>().len() != node.coverage.len()
                || !node.coverage.contains(&node.place)
                || !objects.insert(node.object)
            {
                return Err(
                    "invalid node label/operator/coverage or duplicate object binding".into(),
                );
            }
        }
        for pair in self.pairs.values() {
            if pair.capacity_units_per_ic_second == 0
                || pair.capacity_units_per_ic_second > 1_000_000
                || pair.halves[0].node == pair.halves[1].node
            {
                return Err("invalid pair capacity or endpoints".into());
            }
            if self.nodes.get(&pair.halves[0].node).map(|n| n.operator)
                != self.nodes.get(&pair.halves[1].node).map(|n| n.operator)
            {
                return Err("pair halves must have one operator".into());
            }
            for half in &pair.halves {
                if !self.nodes.contains_key(&half.node)
                    || !halves.insert(&half.id)
                    || !objects.insert(half.object)
                {
                    return Err("missing node, duplicate half identity or object binding".into());
                }
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_BYTES {
            return Err("relay catalog exceeds byte bound".into());
        }
        Ok(())
    }

    /// The document and the installed objects must describe the same topology.
    pub fn reconcile(&self, world: &World) -> Result<(), String> {
        self.validate()?;
        if world
            .module_state::<OrgStore>()
            .and_then(|s| s.state().org(self.operator))
            .is_none()
        {
            return Err(format!("operator #{} is missing", self.operator.0));
        }
        for (id, node) in &self.nodes {
            if world
                .module_state::<OrgStore>()
                .and_then(|s| s.state().org(node.operator))
                .is_none()
            {
                return Err(format!("node {id} operator is missing"));
            }
            check_object(
                world,
                node.object,
                ObjKind::Thing,
                Some(node.place),
                Some(NODE_BEHAVIOR),
            )
            .map_err(|e| format!("node {id}: {e}"))?;
            for place in &node.coverage {
                check_object(world, *place, ObjKind::Room, None, None)
                    .map_err(|e| format!("node {id} coverage: {e}"))?;
            }
        }
        for (id, pair) in &self.pairs {
            for half in &pair.halves {
                let node = self.nodes.get(&half.node).ok_or("missing node")?;
                check_object(
                    world,
                    half.object,
                    ObjKind::Thing,
                    Some(node.object),
                    Some(HALF_BEHAVIOR),
                )
                .map_err(|e| format!("pair {id} half {}: {e}", half.id))?;
            }
        }
        Ok(())
    }
}

fn check_object(
    world: &World,
    id: ObjId,
    kind: ObjKind,
    location: Option<ObjId>,
    behavior: Option<&str>,
) -> Result<(), String> {
    let obj = world
        .get(id)
        .ok_or_else(|| format!("object {id} missing"))?;
    if obj.kind != kind
        || obj.flags.contains(Flags::GOING)
        || location.is_some_and(|p| obj.location != Some(p))
    {
        return Err(format!(
            "object {id} has contradictory kind/location/lifecycle"
        ));
    }
    if let Some(name) = behavior {
        let bid = BehaviorId::new(name).map_err(|e| e.to_string())?;
        if obj
            .behaviors
            .get(&bid)
            .is_none_or(|b| b.schema_version != 1)
            || [NODE_BEHAVIOR, HALF_BEHAVIOR]
                .iter()
                .filter(|n| obj.behaviors.keys().any(|b| b.as_str() == **n))
                .count()
                != 1
        {
            return Err(format!(
                "object {id} lacks a unique schema-1 {name} binding"
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct RelayStore {
    registry: Option<RelayRegistry>,
}

impl RelayStore {
    #[must_use]
    pub fn registry(&self) -> Option<&RelayRegistry> {
        self.registry.as_ref()
    }
}

/// Install only a reconciled post-image. Caller owns content creation/authority.
pub fn install(ctx: &mut Ctx<'_>, registry: RelayRegistry) -> Result<(), String> {
    check_installer(ctx)?;
    registry.reconcile(ctx.w)?;
    let value = serde_json::to_value(&registry).map_err(|e| e.to_string())?;
    let store = ctx
        .w
        .module_state_mut::<RelayStore>()
        .ok_or("relay store not composed")?;
    store.registry = Some(registry);
    ctx.persist(ModuleOp {
        ns: "sim".into(),
        op: ModuleWrite::JsonSet {
            key: KEY.into(),
            value,
        },
    });
    ctx.gate_on_durable_ack();
    Ok(())
}

pub(crate) fn load(ctx: &mut ModuleCtx<'_>, data: &mut ModuleBootData) -> Result<(), ModError> {
    let mut found = None;
    let mut others = Vec::new();
    for (key, value) in std::mem::take(&mut data.kv) {
        if key.as_str() == KEY {
            if found.is_some() {
                return Err(ModError::msg("sim relay: duplicate registry document"));
            }
            if serde_json::to_vec(&value)
                .map_err(|e| ModError::msg(e.to_string()))?
                .len()
                > MAX_BYTES
            {
                return Err(ModError::msg(
                    "sim relay: registry exceeds byte bound; repair relay:registry",
                ));
            }
            let registry: RelayRegistry = serde_json::from_value(value).map_err(|e| {
                ModError::msg(format!(
                    "sim relay: unreadable relay:registry (held, not reseeded): {e}"
                ))
            })?;
            registry
                .reconcile(ctx.w)
                .map_err(|e| ModError::msg(format!("sim relay: repair relay:registry: {e}")))?;
            found = Some(registry);
        } else {
            others.push((key, value));
        }
    }
    data.kv = others;
    // Missing documents with surviving bindings are not a new installation.
    let node = BehaviorId::new(NODE_BEHAVIOR).map_err(|e| ModError::msg(e.to_string()))?;
    let half = BehaviorId::new(HALF_BEHAVIOR).map_err(|e| ModError::msg(e.to_string()))?;
    let expected: BTreeSet<ObjId> = found.as_ref().map_or_else(BTreeSet::new, |r| {
        r.nodes
            .values()
            .map(|n| n.object)
            .chain(
                r.pairs
                    .values()
                    .flat_map(|p| p.halves.iter().map(|h| h.object)),
            )
            .collect()
    });
    // A single boot-time scan, never on a command or delivery tick.
    for obj in ctx.w.iter() {
        if (obj.behaviors.contains_key(&node) || obj.behaviors.contains_key(&half))
            && !expected.contains(&obj.id)
        {
            return Err(ModError::msg(format!(
                "sim relay: unrecorded binding on {}; repair relay:registry before boot",
                obj.id
            )));
        }
    }
    ctx.w
        .module_state_mut::<RelayStore>()
        .ok_or_else(|| ModError::msg("relay store missing"))?
        .registry = found;
    Ok(())
}

fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // Check before parsing identifiers or looking up records: no existence leak.
    if !crate::commands::is_admin(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let registry = ctx
        .w
        .module_state::<RelayStore>()
        .and_then(RelayStore::registry)
        .cloned();
    let Some(mut registry) = registry else {
        crate::commands::info(ctx, "No relay topology installed.");
        return Ok(());
    };
    registry.reconcile(ctx.w).map_err(CmdError::msg)?;
    let switches: Vec<_> = ctx.switches.iter().map(str::to_owned).collect();
    let switch = match switches.as_slice() {
        [] => "list",
        [s] => s.as_str(),
        _ => return Err(CmdError::msg("Use one @relay switch.")),
    };
    match switch {
        "list" | "check" => {
            crate::commands::info(
                ctx,
                format!(
                    "Relay registry v{} revision {}: {} nodes, {} pairs, {} halves; operator #{}. Bindings OK.",
                    registry.version,
                    registry.revision,
                    registry.nodes.len(),
                    registry.pairs.len(),
                    registry.pairs.len() * 2,
                    registry.operator.0
                ),
            );
            for (id, pair) in &registry.pairs {
                crate::commands::info(
                    ctx,
                    format!(
                        "{id}: {:?}; {} <-> {}; capacity {} units/IC-second; {:?}",
                        pair.state(),
                        pair.halves[0].node,
                        pair.halves[1].node,
                        pair.capacity_units_per_ic_second,
                        pair.class_policy
                    ),
                );
            }
        }
        "show" => {
            let id = CanonicalId::new(ctx.args.lhs().trim())
                .map_err(|e| CmdError::msg(e.to_string()))?;
            let pair = registry
                .pairs
                .get(&id)
                .ok_or_else(|| CmdError::msg("No such relay pair."))?;
            crate::commands::info(
                ctx,
                format!("{id}: {:?}; revision {}", pair.state(), registry.revision),
            );
            for half in &pair.halves {
                let node = registry
                    .nodes
                    .get(&half.node)
                    .ok_or_else(|| CmdError::msg("Missing node."))?;
                crate::commands::info(
                    ctx,
                    format!(
                        "half {} {} node {} {} ({}) place {} coverage {:?}; enabled={} powered={} staffed={} destroyed={}",
                        half.id,
                        half.object,
                        half.node,
                        node.object,
                        node.label,
                        node.place,
                        node.coverage,
                        half.enabled,
                        half.powered,
                        half.staffed,
                        half.destroyed
                    ),
                );
            }
        }
        "enable" | "disable" | "power" | "staff" => {
            let id = CanonicalId::new(ctx.args.lhs().trim())
                .map_err(|e| CmdError::msg(e.to_string()))?;
            let flag = match switch {
                "enable" => true,
                "disable" => false,
                _ => match ctx.args.rhs().map(str::trim) {
                    Some("on") => true,
                    Some("off") => false,
                    _ => return Err(CmdError::msg("Use @relay/power|staff <half>=on|off.")),
                },
            };
            let half = registry
                .pairs
                .values_mut()
                .flat_map(|p| p.halves.iter_mut())
                .find(|h| h.id == id)
                .ok_or_else(|| CmdError::msg("No such relay half."))?;
            if half.destroyed {
                return Err(CmdError::msg(
                    "A destroyed half cannot be repaired; manufacture a replacement pair.",
                ));
            }
            let field = match switch {
                "power" => &mut half.powered,
                "staff" => &mut half.staffed,
                _ => &mut half.enabled,
            };
            if *field == flag {
                crate::commands::info(ctx, "Relay availability already matches; no change.");
                return Ok(());
            }
            *field = flag;
            registry.revision = registry
                .revision
                .checked_add(1)
                .ok_or_else(|| CmdError::msg("Relay revision exhausted."))?;
            let revision = registry.revision;
            install(ctx, registry).map_err(CmdError::msg)?;
            crate::commands::info(
                ctx,
                format!("Relay half {id}: {switch}={flag}; revision {revision}."),
            );
        }
        _ => {
            return Err(CmdError::msg(
                "Use @relay/list|check|show|enable|disable|power|staff.",
            ));
        }
    }
    Ok(())
}

/// Installed hardware cannot be picked up, teleported or stripped of its binding.
/// Deployment/replacement is a later milestone, through its own lifecycle seam.
struct InstalledGate;

fn is_hardware(w: &World, obj: ObjId) -> bool {
    w.get(obj).is_some_and(|o| {
        o.behaviors
            .keys()
            .any(|b| matches!(b.as_str(), NODE_BEHAVIOR | HALF_BEHAVIOR))
    })
}

impl EventHook for InstalledGate {
    fn carry_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        _: ObjId,
        obj: ObjId,
        _: ObjId,
    ) -> Result<(), RichText> {
        if is_hardware(ctx.world(), obj) {
            Err(RichText::tag(
                "error",
                "Installed relay hardware cannot be moved.",
            ))
        } else {
            Ok(())
        }
    }
    fn move_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        obj: ObjId,
        _: Option<ObjId>,
        _: ObjId,
    ) -> Result<(), RichText> {
        if is_hardware(ctx.world(), obj) {
            Err(RichText::tag(
                "error",
                "Installed relay hardware cannot be moved.",
            ))
        } else {
            Ok(())
        }
    }
}

impl DetachGate for InstalledGate {
    fn check(
        &self,
        w: &World,
        _: &Registry,
        _: ObjId,
        obj: ObjId,
        behavior: &BehaviorId,
    ) -> Result<(), String> {
        if matches!(behavior.as_str(), NODE_BEHAVIOR | HALF_BEHAVIOR)
            && w.module_state::<RelayStore>()
                .and_then(RelayStore::registry)
                .is_some_and(|r| {
                    r.nodes.values().any(|n| n.object == obj)
                        || r.pairs
                            .values()
                            .any(|p| p.halves.iter().any(|h| h.object == obj))
                })
        {
            Err("Installed relay binding is still referenced by relay:registry.".into())
        } else {
            Ok(())
        }
    }
}

/// Check before a content applier creates anything, not after it has made objects.
pub fn check_installer(ctx: &Ctx<'_>) -> Result<(), String> {
    if ctx
        .module
        .as_ref()
        .is_some_and(|m| m != "sim" && ctx.reg.persist_delegation(m, "sim").is_none())
    {
        Err("relay: installer has no declared sim persistence delegation".into())
    } else {
        Ok(())
    }
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    for writer in ["content", "atlas"] {
        reg.persist_delegation(writer, "sim", "R1 relay content installs its bounded sim catalog during boot or staff @atlas/pack apply")?;
    }
    reg.state(RelayStore::default());
    reg.hook(HookPoint::CarryAttempt, Arc::new(InstalledGate));
    reg.hook(HookPoint::MoveAttempt, Arc::new(InstalledGate));
    reg.detach_gate(Arc::new(InstalledGate));
    for name in [NODE_BEHAVIOR, HALF_BEHAVIOR] {
        let mut spec =
            BehaviorSpec::new(BehaviorId::new(name).map_err(|e| ModError::msg(e.to_string()))?);
        spec.allowed_kinds = vec![ObjKind::Thing];
        spec.conflicts = vec![
            BehaviorId::new(if name == NODE_BEHAVIOR {
                HALF_BEHAVIOR
            } else {
                NODE_BEHAVIOR
            })
            .map_err(|e| ModError::msg(e.to_string()))?,
        ];
        spec.destroy = DestroyPolicy::Refuse;
        spec.min_role = Role::Wizard;
        spec.help = "Installed relay topology; managed by its content pack and @relay.".into();
        reg.behavior(spec)?;
    }
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    reg.command(CommandSpec {
        name: "@relay".into(), shape: ArgShape::EqPair,
        switches: ["list", "check", "show", "enable", "disable", "power", "staff"].into_iter().map(SmolStr::new).collect(),
        help: "@relay (staff): /list, /check, /show <pair>; /enable|disable <half>; /power|staff <half>=on|off. Route diagnostics: @prop/route <envelope id>.".into(),
        ..CommandSpec::default()
    }, Arc::new(command))
}
