//! The campaign store, its persistence and its boot (E7 cut 9).
//!
//! ## The document discipline (§4 rules 1 and 2)
//!
//! | doc | class | a refused one |
//! |---|---|---|
//! | `op:{id}` | container | **held, never deleted**, its subject marked and staff paged |
//! | `formation:{id}` | container | **held, never deleted**, its subject marked |
//! | `campaign_meta` | allocators + the tick cursor | tolerated field by field, floored by the keys |
//!
//! An operation is a container for the same reason a tenancy is: its state **is**
//! the record — and, unlike a tenancy, it may be holding a live money `Hold` whose
//! only statement of purpose is this document. Deleting it would strand the
//! attacker's funds with nothing left in the world to say what they were for. So a
//! refused document is **held** (cut 6's `JourneyBook::held` precedent), the world
//! still boots, and staff are paged with the decode error and the command.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{ModuleOp, ModuleWrite};
use mush_core::{Ctx, Outbox, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::formation::{FormationAuthority, FormationKind, FormationNode};
use crate::ids::{FormationId, OperationId, formation_key, operation_key};
use crate::record::{OperationDoc, UnitRef};

/// The module's persistence namespace and registered name.
pub const NS: &str = "campaign";
/// The allocator/cursor document.
pub const META_KEY: &str = "campaign_meta";
/// The largest number of operations one world may carry.
pub const MAX_OPERATIONS: usize = 512;
/// The largest number of formation nodes one world may carry.
pub const MAX_FORMATION_NODES: usize = 1_024;
/// The largest number of refused documents kept for `@op/held`-style inspection
/// (cut 7's `MAX_HELD_DOCUMENTS` shape).
pub const MAX_HELD_DOCUMENTS: usize = 64;

/// One document the boot could not decode, kept with its error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldDoc {
    /// The key it was stored at.
    pub key: SmolStr,
    /// Why it was refused.
    pub error: String,
}

/// What one tick fire did (the pin every budget test reads).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TickStats {
    /// How many operations were touched at all.
    pub visited: usize,
    /// How many were worked.
    pub worked: usize,
    /// How many consumers were advanced (at most one per operation per fire).
    pub consumers: usize,
    /// How many were paused this fire.
    pub paused: usize,
}

/// The world-attached campaign state.
#[derive(Clone, Default)]
pub struct CampaignStore {
    ops: BTreeMap<u64, OperationDoc>,
    formations: BTreeMap<u64, FormationNode>,
    /// Kinds, hydrated from the composition-frozen authority at boot. Empty when
    /// no authority is wired, which makes `@force/create` refuse — fail-closed.
    kinds: BTreeMap<SmolStr, FormationKind>,
    authority: Option<Arc<dyn FormationAuthority>>,
    held: Vec<HeldDoc>,
    next_operation: u64,
    next_formation: u64,
    /// The rotation cursor: the tick resumes **past the last visited id**, skipped
    /// ones included (housing's `tick.rs`, D-LBK-2).
    pub(crate) cursor: u64,
    /// The last fire's budget accounting.
    pub(crate) last_fire: TickStats,
    /// Subjects already paged, so a repeated failure pages once rather than once a
    /// fire (08c §4.6's dedupe-by-subject rule).
    pub(crate) paged: BTreeSet<SmolStr>,
}

impl std::fmt::Debug for CampaignStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CampaignStore")
            .field("operations", &self.ops.len())
            .field("formations", &self.formations.len())
            .field("kinds", &self.kinds.len())
            .field("held", &self.held.len())
            .field("cursor", &self.cursor)
            .finish()
    }
}

/// The `campaign_meta` document.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Meta {
    next_operation: u64,
    next_formation: u64,
    cursor: u64,
}

impl CampaignStore {
    /// One operation.
    #[must_use]
    pub fn operation(&self, id: OperationId) -> Option<&OperationDoc> {
        self.ops.get(&id.0)
    }

    /// Every operation, in id order.
    pub fn operations(&self) -> impl Iterator<Item = &OperationDoc> {
        self.ops.values()
    }

    /// Every operation id, in order — the tick's rotation reads this.
    #[must_use]
    pub fn operation_ids(&self) -> Vec<OperationId> {
        self.ops.keys().copied().map(OperationId).collect()
    }

    /// One formation node.
    #[must_use]
    pub fn formation(&self, id: FormationId) -> Option<&FormationNode> {
        self.formations.get(&id.0)
    }

    /// Every formation node, in id order.
    pub fn formations(&self) -> impl Iterator<Item = &FormationNode> {
        self.formations.values()
    }

    /// One formation kind, as the composition-frozen authority authored it.
    #[must_use]
    pub fn kind(&self, id: &str) -> Option<&FormationKind> {
        self.kinds.get(id)
    }

    /// Every known kind, in id order.
    pub fn kinds(&self) -> impl Iterator<Item = &FormationKind> {
        self.kinds.values()
    }

    /// The documents the boot refused, with their errors.
    #[must_use]
    pub fn held(&self) -> &[HeldDoc] {
        &self.held
    }

    /// The last fire's accounting.
    #[must_use]
    pub fn last_fire(&self) -> TickStats {
        self.last_fire
    }

    /// **The one-operation rule (R1)**: which operation, if any, already holds
    /// `unit`. A `Resolved` or `Aborted` operation holds nothing.
    #[must_use]
    pub fn operation_holding(&self, unit: UnitRef) -> Option<OperationId> {
        self.ops
            .values()
            .find(|doc| {
                !matches!(
                    doc.runtime.state,
                    crate::record::OperationState::Resolved
                        | crate::record::OperationState::Aborted
                ) && doc.runtime.holds(unit)
            })
            .map(OperationDoc::id)
    }

    fn meta_doc(&self) -> serde_json::Value {
        serde_json::to_value(Meta {
            next_operation: self.next_operation,
            next_formation: self.next_formation,
            cursor: self.cursor,
        })
        .unwrap_or(serde_json::Value::Null)
    }

    /// Mint the next operation id, or `None` past [`MAX_OPERATIONS`].
    pub(crate) fn mint_operation(&mut self) -> Option<OperationId> {
        if self.ops.len() >= MAX_OPERATIONS {
            return None;
        }
        let id = self.next_operation.max(1);
        self.next_operation = id.checked_add(1)?;
        Some(OperationId(id))
    }

    /// Mint the next formation id, or `None` past [`MAX_FORMATION_NODES`].
    pub(crate) fn mint_formation(&mut self) -> Option<FormationId> {
        if self.formations.len() >= MAX_FORMATION_NODES {
            return None;
        }
        let id = self.next_formation.max(1);
        self.next_formation = id.checked_add(1)?;
        Some(FormationId(id))
    }

    pub(crate) fn install_operation(&mut self, doc: OperationDoc) {
        self.next_operation = self.next_operation.max(doc.id().0.saturating_add(1));
        self.ops.insert(doc.id().0, doc);
    }

    pub(crate) fn install_formation(&mut self, node: FormationNode) {
        self.next_formation = self.next_formation.max(node.id.0.saturating_add(1));
        self.formations.insert(node.id.0, node);
    }

    pub(crate) fn hold(&mut self, key: SmolStr, error: String) {
        if self.held.len() >= MAX_HELD_DOCUMENTS {
            return;
        }
        self.held.push(HeldDoc { key, error });
    }

    pub(crate) fn set_authority(&mut self, authority: Option<Arc<dyn FormationAuthority>>) {
        self.authority = authority;
    }

    /// Re-read the composition-frozen authority. Called once at boot; the kinds are
    /// rules data and never change at runtime.
    pub(crate) fn hydrate_kinds(&mut self, world: &World) {
        let Some(authority) = self.authority.clone() else {
            return;
        };
        self.kinds = authority
            .kinds(world)
            .into_iter()
            .map(|k| (k.id.clone(), k))
            .collect();
    }
}

fn push(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Install an operation into RAM and queue its document. **The single write path**:
/// there is no route that writes one without the other.
pub(crate) fn put_operation(ctx: &mut Ctx<'_>, doc: OperationDoc) {
    let key = operation_key(doc.id());
    let value = serde_json::to_value(&doc).unwrap_or(serde_json::Value::Null);
    if let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
        store.install_operation(doc);
    }
    push(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// Install a formation node into RAM and queue its document.
pub(crate) fn put_formation(ctx: &mut Ctx<'_>, node: FormationNode) {
    let key = formation_key(node.id);
    let value = serde_json::to_value(&node).unwrap_or(serde_json::Value::Null);
    if let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
        store.install_formation(node);
    }
    push(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// Queue the allocator/cursor document.
pub(crate) fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(meta) = ctx
        .w
        .module_state::<CampaignStore>()
        .map(CampaignStore::meta_doc)
    else {
        return;
    };
    push(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: meta,
        },
    );
}

/// Load the boot documents. A refused container is **held**, never dropped and
/// never fatal; the allocators are floored by the keys that survived, so a stale or
/// missing meta can never re-issue a live id.
pub(crate) fn load(kv: Vec<(SmolStr, serde_json::Value)>) -> CampaignStore {
    let mut store = CampaignStore::default();
    for (key, value) in kv {
        if key == META_KEY {
            match serde_json::from_value::<Meta>(value) {
                Ok(meta) => {
                    store.next_operation = meta.next_operation;
                    store.next_formation = meta.next_formation;
                    store.cursor = meta.cursor;
                }
                Err(error) => {
                    tracing::warn!(%key, %error, "campaign: the meta document was ignored");
                }
            }
        } else if let Some(raw) = key.strip_prefix("op:") {
            let id = raw.parse::<u64>().unwrap_or(0);
            match serde_json::from_value::<OperationDoc>(value) {
                Ok(doc) if doc.id().0 == id && id != 0 => store.install_operation(doc),
                Ok(_) => {
                    store.next_operation = store.next_operation.max(id.saturating_add(1));
                    store.hold(key, "the document's id disagrees with its key".to_owned());
                }
                Err(error) => {
                    // The allocator still floors past a held id, so a refused
                    // operation can never have its id re-issued underneath it.
                    store.next_operation = store.next_operation.max(id.saturating_add(1));
                    store.hold(key, error.to_string());
                }
            }
        } else if let Some(raw) = key.strip_prefix("formation:") {
            let id = raw.parse::<u64>().unwrap_or(0);
            match serde_json::from_value::<FormationNode>(value) {
                Ok(node) if node.id.0 == id && id != 0 => store.install_formation(node),
                Ok(_) => {
                    store.next_formation = store.next_formation.max(id.saturating_add(1));
                    store.hold(key, "the document's id disagrees with its key".to_owned());
                }
                Err(error) => {
                    store.next_formation = store.next_formation.max(id.saturating_add(1));
                    store.hold(key, error.to_string());
                }
            }
        } else {
            tracing::warn!(%key, "campaign: unknown persisted key ignored");
        }
    }
    store
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::OperationId;
    use crate::record::OperationRuntime;
    use crate::terms::operation_consent_hash;
    use crate::testfix::terms_fixture;

    fn doc(id: OperationId) -> OperationDoc {
        let mut terms = terms_fixture(id);
        terms.consent_hash = operation_consent_hash(&terms);
        OperationDoc {
            terms,
            runtime: OperationRuntime::declared(id),
        }
    }

    #[test]
    fn e7c9_b2_a_refused_operation_document_is_held_and_its_id_is_never_reissued() {
        let good = serde_json::to_value(doc(OperationId(1))).expect("serializes");
        let mut bad = serde_json::to_value(doc(OperationId(7))).expect("serializes");
        bad["runtime"]["participants"] = serde_json::json!([
            { "unit": 1, "side": "a", "command_parent": null, "joined_at": 0 },
            { "unit": 1, "side": "a", "command_parent": null, "joined_at": 0 },
        ]);
        let store = load(vec![
            (SmolStr::new_static("op:1"), good),
            (SmolStr::new_static("op:7"), bad),
        ]);
        assert!(store.operation(OperationId(1)).is_some());
        assert!(store.operation(OperationId(7)).is_none(), "refused");
        assert_eq!(store.held().len(), 1);
        assert_eq!(store.held()[0].key, "op:7");
        // …and the allocator floored past the held id, so the next mint cannot
        // collide with a document staff have yet to repair.
        let mut store = store;
        assert_eq!(store.mint_operation(), Some(OperationId(8)));
    }

    #[test]
    fn e7c9_b2_a_document_whose_id_disagrees_with_its_key_is_held_not_installed() {
        let mut wrong = serde_json::to_value(doc(OperationId(3))).expect("serializes");
        wrong["terms"]["operation"] = serde_json::json!(3);
        let store = load(vec![(SmolStr::new_static("op:4"), wrong)]);
        assert!(store.operation(OperationId(3)).is_none());
        assert!(store.operation(OperationId(4)).is_none());
        assert_eq!(store.held().len(), 1);
    }
}
