//! **A composed landing's writes, split the way the store holds them** (doc 23 cut S.1).
//!
//! The production world task prepares every authoritative act on the composed path
//! (`Engine::prepare_registered_composed_resolved_authoritative_action`), whose bundle is one
//! list of `ReplayWrite`s: the typed families' post-images and, since S.1, the black box's
//! `log:hull.<n>` chunk the finish's owner participant appends. A harness that lands acts that
//! way keeps the typed post-images for the reboot's `hydrate_action_state` and folds the ledger
//! chunks into the module store the reboot boots from — [`split`] is that split, the hardware
//! flow driver's conversion (`tacsim::hardware::flow::driver`), so no harness spells it twice.

use mush_core::authoritative::ReplayWrite;
use mush_core::{ModuleOp, ModuleWrite};

/// **`writes` as module ops**: `(typed, ledger)` — every typed post-image (`ModuleJsonSet` as a
/// `JsonSet`, `ModuleDelete` as a `Del`) and every ledger chunk (`LedgerChunkSet` as the
/// `JsonSet` of its `log:<id>:<chunk>` document). A surface act writes no object.
///
/// # Panics
///
/// On an object write, or a chunk that does not encode.
#[must_use]
pub fn split(writes: &[ReplayWrite]) -> (Vec<ModuleOp>, Vec<ModuleOp>) {
    let mut typed = Vec::new();
    let mut ledger = Vec::new();
    for write in writes {
        match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => typed.push(ModuleOp {
                ns: namespace.as_str().into(),
                op: ModuleWrite::JsonSet {
                    key: key.as_str().into(),
                    value: value.clone(),
                },
            }),
            ReplayWrite::ModuleDelete { namespace, key } => typed.push(ModuleOp {
                ns: namespace.as_str().into(),
                op: ModuleWrite::Del {
                    key: key.as_str().into(),
                },
            }),
            ReplayWrite::LedgerChunkSet { write } => ledger.push(ModuleOp {
                ns: write.namespace().as_str().into(),
                op: ModuleWrite::JsonSet {
                    key: write.key().as_str().into(),
                    value: serde_json::to_value(write.chunk()).expect("a ledger chunk encodes"),
                },
            }),
            ReplayWrite::ObjectSet { .. }
            | ReplayWrite::ObjectDelete { .. }
            | ReplayWrite::ObjectAllocatorSet { .. } => {
                panic!("a landed act wrote an object: {write:?}")
            }
        }
    }
    (typed, ledger)
}

/// Whether `write` is a black box chunk (`log:hull.<n>:<k>` in tacsim's namespace).
#[must_use]
pub fn is_black_box(write: &ReplayWrite) -> bool {
    matches!(
        write,
        ReplayWrite::LedgerChunkSet { write }
            if write.namespace().as_str() == tacsim::NS && write.log_id().starts_with("hull.")
    )
}
