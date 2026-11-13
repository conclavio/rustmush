//! The cold restart (doc 46 cut CT.2): one engine's persisted documents handed
//! to a freshly composed engine, the way a boot hands them over.
//!
//! A store holds two kinds of module document. The **typed** rows (keys under
//! `action-state/`, [`is_action_state_persistence_key`]) are the kernel's:
//! they are hydrated into the engine's action state after composition and
//! **before any module's boot callback**. Everything else is a module's own
//! and reaches it through [`Engine::modules_loaded`]. A test that restarts
//! cold must keep that order, and until CT.2 each commerce test file kept it
//! by hand: the same partition loop in five files, and a hydrate-then-boot
//! pair in half a dozen more.
//!
//! What is here:
//!
//! - [`load`] is the boot itself, over a [`Boot`] map that may hold both
//!   kinds: the typed rows are taken out ([`take_typed_rows`]) and hydrated,
//!   then the modules load the rest. [`TestWorld::try_cold_loaded`] is the same
//!   on a [`TestWorld`].
//! - [`TestWorld::cold_reboot_into`] and [`TestWorld::cold_restart`] are the
//!   whole restart: a fresh registry and engine composed over the world's
//!   objects, with no session, then [`load`], then God reconnected. That is
//!   the sequence [`TestWorld::reboot_into`] and, with a Store behind it,
//!   `AcknowledgedWorld::reboot_into` run, and all three compose the engine
//!   through the same function. `cold_restart` leaves the running world as it
//!   was, so a test can compare the two.
//! - [`load_typed`] is the restart of a rig that persists typed rows only.
//! - [`fold_replay_writes_into`] is the other half: the replay writes of an
//!   installed authoritative action folded into a [`Boot`] map, as the store
//!   that applied the bundle would hold them.
//!
//! **Why not `AcknowledgedWorld` itself.** `AcknowledgedWorld` drives a real
//! in-memory Store and refuses a boot whose typed rows differ from what that
//! Store acknowledged. The commerce owner tests stage and install prepared
//! actions directly, and many of them hand the restart a document they have
//! damaged on purpose, to see the owner under test refuse it. That refusal
//! must come from the owner, not from the fixture, so they restart over a
//! [`Boot`] map with no Store behind it.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use mush_core::authoritative::ReplayWrite;
use mush_core::engine::Effects;
use mush_core::{
    ActionStateBootRecord, ActionStateError, Engine, GOD, GameModule, ModError, ModuleBootData,
    ModuleOp, ModuleWrite, Outbox, is_action_state_persistence_key,
};
use serde_json::Value;
use smol_str::SmolStr;

use crate::{TestWorld, fold_module_ops_into, recompose};

/// Every namespace's persisted documents, as a boot hands them to the engine.
pub type Boot = HashMap<SmolStr, ModuleBootData>;

/// Why a cold restart was refused: by the kernel's typed hydration, or by a
/// module's boot. It prints exactly what the refusing side printed.
#[derive(Debug)]
pub enum ColdError {
    /// The kernel refused the typed rows.
    Typed(ActionStateError),
    /// A module refused its boot data, or the composition collided.
    Module(ModError),
    /// A replay write that is not a module document (an object write).
    Write(&'static str),
}

impl fmt::Display for ColdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Typed(error) => error.fmt(f),
            Self::Module(error) => error.fmt(f),
            Self::Write(error) => f.write_str(error),
        }
    }
}

impl Error for ColdError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Typed(error) => Some(error),
            Self::Module(error) => Some(error),
            Self::Write(_) => None,
        }
    }
}

impl From<ActionStateError> for ColdError {
    fn from(error: ActionStateError) -> Self {
        Self::Typed(error)
    }
}

impl From<ModError> for ColdError {
    fn from(error: ModError) -> Self {
        Self::Module(error)
    }
}

/// Take every typed action-state row out of `boot`, leaving each namespace its
/// own documents.
pub fn take_typed_rows(boot: &mut Boot) -> Vec<ActionStateBootRecord> {
    let mut typed = Vec::new();
    for (namespace, data) in boot {
        data.kv.retain(|(key, value)| {
            if is_action_state_persistence_key(key) {
                typed.push(ActionStateBootRecord::new(
                    namespace.clone(),
                    key.clone(),
                    value.clone(),
                ));
                false
            } else {
                true
            }
        });
    }
    typed
}

/// Boot a composed engine from `boot`: the typed rows hydrated first, then
/// every module's boot callback over what is left. Output is discarded.
///
/// # Errors
///
/// The kernel's refusal of a typed row, or a module's refusal of its data.
pub fn load(engine: &mut Engine, mut boot: Boot) -> Result<(), ColdError> {
    let typed = take_typed_rows(&mut boot);
    engine.hydrate_action_state(typed)?;
    engine.modules_loaded(boot, &mut Outbox::new(), &mut Effects::new())?;
    Ok(())
}

/// Boot a rig that persists typed rows only: `rows` (namespace, key, value)
/// hydrated, then `module`'s boot callback with no document of its own.
///
/// # Errors
///
/// The kernel's refusal of a row, or the module's boot audit.
pub fn load_typed<'a>(
    engine: &mut Engine,
    module: &str,
    rows: impl IntoIterator<Item = (&'a str, &'a str, &'a Value)>,
) -> Result<(), ColdError> {
    engine.hydrate_action_state(rows.into_iter().map(|(namespace, key, value)| {
        ActionStateBootRecord::new(namespace, key, value.clone())
    }))?;
    engine.module_loaded(
        module,
        ModuleBootData::default(),
        &mut Outbox::new(),
        &mut Effects::new(),
    )?;
    Ok(())
}

/// The module documents `writes` set, as the ops a store applies. Only module
/// JSON and ledger chunks are documents.
///
/// # Errors
///
/// A write of another kind (an object, a delete): the fixtures that fold
/// replay writes never produce one, so it is a broken fixture.
pub fn replay_ops(writes: &[ReplayWrite]) -> Result<Vec<ModuleOp>, ColdError> {
    let mut ops = Vec::with_capacity(writes.len());
    for write in writes {
        ops.push(match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => ModuleOp {
                ns: namespace.as_str().into(),
                op: ModuleWrite::JsonSet {
                    key: key.as_str().into(),
                    value: value.clone(),
                },
            },
            ReplayWrite::LedgerChunkSet { write } => ModuleOp {
                ns: write.namespace().as_str().into(),
                op: ModuleWrite::JsonSet {
                    key: write.key().as_str().into(),
                    value: serde_json::to_value(write.chunk())
                        .map_err(|_| ColdError::Write("ledger chunk encoding"))?,
                },
            },
            _ => return Err(ColdError::Write("unexpected replay write")),
        });
    }
    Ok(ops)
}

/// Fold an installed action's replay writes into `boot`, as the store that
/// applied its bundle holds them ([`fold_module_ops_into`]).
///
/// # Errors
///
/// As [`replay_ops`].
pub fn fold_replay_writes_into(boot: &mut Boot, writes: &[ReplayWrite]) -> Result<(), ColdError> {
    fold_module_ops_into(boot, replay_ops(writes)?);
    Ok(())
}

impl TestWorld {
    /// [`load`] on this world's engine: a second boot of the same composition
    /// from `boot`, typed rows first. For a rig that rebuilds its fixture
    /// objects in a fresh world and then boots it from kept documents.
    ///
    /// # Errors
    ///
    /// As [`load`].
    pub fn try_cold_loaded(&mut self, boot: Boot) -> Result<(), ColdError> {
        load(self.engine_mut(), boot)
    }

    /// [`TestWorld::reboot_into`] for a store that holds typed rows: a fresh
    /// registry and engine composed from `modules` over this world's objects,
    /// every session dropped, then [`load`], then God reconnected.
    ///
    /// # Errors
    ///
    /// A registration collision in `modules`, or as [`load`].
    pub fn cold_reboot_into(
        self,
        modules: Vec<Box<dyn GameModule>>,
        boot: Boot,
    ) -> Result<TestWorld, ColdError> {
        let next_conn = self.next_conn;
        TestWorld::cold_over(self.engine.world, next_conn, modules, boot)
    }

    /// [`TestWorld::cold_reboot_into`] over a copy of this world's objects,
    /// clock and timeline: the restarted world to compare with this one, which
    /// goes on running untouched. Nothing of this engine's module or typed
    /// state crosses: the new engine installs its own slots and holds only
    /// what `boot` gives it.
    ///
    /// # Errors
    ///
    /// As [`TestWorld::cold_reboot_into`].
    pub fn cold_restart(
        &self,
        modules: Vec<Box<dyn GameModule>>,
        boot: Boot,
    ) -> Result<TestWorld, ColdError> {
        TestWorld::cold_over(self.world().clone(), self.next_conn, modules, boot)
    }

    fn cold_over(
        world: mush_core::World,
        next_conn: u64,
        modules: Vec<Box<dyn GameModule>>,
        boot: Boot,
    ) -> Result<TestWorld, ColdError> {
        let mut tw = TestWorld {
            engine: Box::new(recompose(world, modules)?),
            next_conn,
        };
        tw.try_cold_loaded(boot)?;
        tw.connect(GOD);
        Ok(tw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::Registrar;
    use serde_json::json;

    /// A module that keeps one document, `note`, and refuses a boot that
    /// carries a typed row (the kernel owns those: one reaching a module's
    /// callback is the bug this module is here to catch).
    struct Toy;
    #[derive(Clone, Default)]
    struct ToyNote(Option<Value>);

    impl GameModule for Toy {
        fn name(&self) -> &'static str {
            "toy"
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.state(ToyNote::default());
            Ok(())
        }
        fn on_world_loaded(
            &mut self,
            ctx: &mut mush_core::ModuleCtx<'_>,
            data: ModuleBootData,
        ) -> Result<(), ModError> {
            if data
                .kv
                .iter()
                .any(|(key, _)| is_action_state_persistence_key(key))
            {
                return Err(ModError::msg("a typed row reached a module's boot"));
            }
            let note = data
                .kv
                .iter()
                .find(|(key, _)| key == "note")
                .map(|(_, value)| value.clone());
            if note == Some(json!("damaged")) {
                return Err(ModError::msg("the note is damaged"));
            }
            if let Some(slot) = ctx.state_mut::<ToyNote>() {
                slot.0 = note;
            }
            Ok(())
        }
    }

    fn toy() -> Vec<Box<dyn GameModule>> {
        vec![Box::new(Toy)]
    }

    fn set(key: &str, value: Value) -> ModuleOp {
        ModuleOp {
            ns: "toy".into(),
            op: ModuleWrite::JsonSet {
                key: key.into(),
                value,
            },
        }
    }

    fn note(tw: &TestWorld) -> Option<Value> {
        tw.world()
            .module_state::<ToyNote>()
            .and_then(|slot| slot.0.clone())
    }

    #[test]
    fn typed_rows_are_taken_out_and_the_rest_is_left() {
        let mut boot = Boot::new();
        fold_module_ops_into(
            &mut boot,
            vec![
                set("note", json!("kept")),
                set("action-state/v1/family/key", json!({"row": 1})),
            ],
        );
        let typed = take_typed_rows(&mut boot);
        assert_eq!(typed.len(), 1);
        let row = typed.first().expect("one typed row");
        assert_eq!(row.namespace, "toy");
        assert_eq!(row.key, "action-state/v1/family/key");
        assert_eq!(row.value, json!({"row": 1}));
        let left = &boot.get("toy").expect("the namespace stays").kv;
        assert_eq!(left, &vec![("note".into(), json!("kept"))]);
    }

    #[test]
    fn a_cold_restart_holds_what_the_documents_hold_and_nothing_else() {
        let mut live = TestWorld::with_modules(toy());
        let player = live.spawn_player("Kept");
        live.world_mut().set_now(4_000);
        let mut boot = Boot::new();
        fold_module_ops_into(&mut boot, vec![set("note", json!("written"))]);

        // The live world never took the write; only the documents hold it.
        assert_eq!(note(&live), None);
        let cold = live
            .cold_restart(toy(), boot.clone())
            .expect("the restart loads");
        assert_eq!(note(&cold), Some(json!("written")));
        assert_eq!(note(&live), None, "the running world is left as it was");
        assert!(cold.world().get(player).is_some(), "the objects are kept");
        assert_eq!(cold.world().now(), 4_000, "and the clock");
        assert!(!cold.world().is_connected(player), "no session survives");
        assert!(cold.world().is_connected(GOD));

        // A write the documents do not hold is not in the restarted world,
        // even when the running world's own state holds it: the copy carries
        // objects, not a module's slot.
        if let Some(slot) = live.world_mut().module_state_mut::<ToyNote>() {
            slot.0 = Some(json!("only in memory"));
        }
        assert_eq!(note(&live), Some(json!("only in memory")));
        let cold = live
            .cold_restart(toy(), Boot::new())
            .expect("an empty boot");
        assert_eq!(note(&cold), None, "a dropped write is gone");
        assert_eq!(note(&live), Some(json!("only in memory")));

        // The consuming form is the same restart.
        let cold = live.cold_reboot_into(toy(), boot).expect("the restart");
        assert_eq!(note(&cold), Some(json!("written")));
    }

    #[test]
    fn a_module_refusal_is_the_restarts_error_as_the_module_worded_it() {
        let live = TestWorld::with_modules(toy());
        let mut boot = Boot::new();
        fold_module_ops_into(&mut boot, vec![set("note", json!("damaged"))]);
        let Err(error) = live.cold_restart(toy(), boot) else {
            panic!("a damaged document must refuse the boot");
        };
        assert!(matches!(error, ColdError::Module(_)));
        assert_eq!(error.to_string(), "the note is damaged");
    }

    #[test]
    fn a_typed_row_goes_to_the_kernel_and_never_to_a_module() {
        let live = TestWorld::with_modules(toy());
        let mut boot = Boot::new();
        fold_module_ops_into(
            &mut boot,
            vec![set("action-state/v1/unregistered/key", json!({"row": 1}))],
        );
        // No family is registered, so the kernel refuses the row. Had the row
        // reached the module instead, the module's own refusal would print.
        let Err(error) = live.cold_restart(toy(), boot) else {
            panic!("an unregistered typed row must refuse the boot");
        };
        assert!(matches!(error, ColdError::Typed(_)), "{error}");
    }

    #[test]
    fn replay_writes_fold_as_the_documents_they_set() {
        let namespace = mush_core::mechanics::OwnerId::new("toy").expect("owner");
        let key = mush_core::authoritative::AuthoritativeKey::parse_canonical("note").expect("key");
        let writes = vec![ReplayWrite::ModuleJsonSet {
            namespace: namespace.clone(),
            key: key.clone(),
            value: json!("written"),
        }];
        let mut boot = Boot::new();
        fold_replay_writes_into(&mut boot, &writes).expect("a document write folds");
        assert_eq!(
            boot.get("toy").expect("the namespace").kv,
            vec![("note".into(), json!("written"))]
        );
        let delete = [ReplayWrite::ModuleDelete { namespace, key }];
        assert!(matches!(
            fold_replay_writes_into(&mut boot, &delete),
            Err(ColdError::Write(_))
        ));
    }
}
