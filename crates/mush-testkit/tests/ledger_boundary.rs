//! The ledger-substrate boundary rig, exercised end to end through the real
//! dispatch + persistence path with a toy module that holds a [`Ledger`].
//!
//! Two invariants the substrate exists to guarantee:
//! 1. a command's world mutation and its ledger append land in ONE write batch
//!    (so an event can never be recorded without its mutation, or vice versa);
//! 2. the open tail rehydrates from persisted chunks on reboot, resuming the
//!    sequence exactly.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar};
use mush_core::{ArgShape, Attr, CmdError, CommandSpec, Ctx, Ledger, ModuleOp, ModuleWrite};
use smol_str::SmolStr;

const NS: &str = "toy";

/// The toy module's state: a ledger plus nothing else. One log id, `"main"`.
#[derive(Clone, Default)]
struct ToyState {
    ledger: Ledger,
}

struct ToyModule;

impl GameModule for ToyModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(ToyState::default());
        reg.command(
            CommandSpec {
                name: "+toylog".into(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(toylog),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        boot: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut ledger = Ledger::new();
        ledger
            .restore_from_kv(&boot.kv)
            .map_err(|error| ModError::msg(format!("toy ledger restore failed: {error}")))?;
        if let Some(st) = ctx.state_mut::<ToyState>() {
            st.ledger = ledger;
        }
        Ok(())
    }
}

/// `+toylog <text>` — records an event AND mutates the actor, the two halves a
/// ledger consumer must keep atomic. Append is on the success path (after the
/// command has committed to succeeding), per the ledger transaction rule.
fn toylog(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let text = ctx.args.raw().trim().to_string();
    if text.is_empty() {
        return Err(CmdError::msg("Say something."));
    }
    let actor = ctx.actor;
    // 1. Prepare the ledger append without changing it.
    let prepared = {
        let st = ctx
            .w
            .module_state::<ToyState>()
            .expect("toy state installed at registration");
        st.ledger
            .prepare_append("main", serde_json::json!({ "text": text, "by": actor.0 }))
            .map_err(|error| CmdError::msg(format!("ledger append refused: {error}")))?
    };
    // 2. Mutate world state (dirties the actor).
    ctx.w
        .set_attr(
            actor,
            "toy:last",
            Attr::json(serde_json::json!(text), actor),
        )
        .map_err(|e| CmdError::msg(e.to_string()))?;
    // 3. Applying a current prepared plan is infallible. Queue its write in
    // the SAME command as the object mutation.
    let (_seq, op) = ctx
        .w
        .module_state_mut::<ToyState>()
        .expect("toy state installed at registration")
        .ledger
        .apply_prepared(prepared)
        .expect("toy ledger did not change between prepare and apply");
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op,
    });
    Ok(())
}

use mush_testkit::TestWorld;

fn toy_world() -> (TestWorld, mush_core::ObjId) {
    let mut tw = TestWorld::with_modules(vec![Box::new(ToyModule)]);
    let alice = tw.spawn_player("Alice");
    // Discard the setup batch (spawn dirties the player) so each test observes
    // only the batch its command-under-test produces.
    tw.drain_write_batch();
    (tw, alice)
}

#[test]
fn mutation_and_ledger_append_land_in_one_batch() {
    let (mut tw, alice) = toy_world();
    tw.run(alice, "+toylog the prince is dead");
    let batch = tw.drain_write_batch();
    // The boundary invariant: the actor mutation and the ledger append are in
    // the same drained batch, so they commit atomically.
    batch.asserts_atomic_with_ledger(alice, NS, "main");
    // The appended chunk carries our one entry.
    let appends = batch.ledger_appends();
    assert_eq!(appends.len(), 1);
    let (ns, log_id, chunk) = &appends[0];
    assert_eq!(ns.as_str(), NS);
    assert_eq!(log_id.as_str(), "main");
    assert_eq!(chunk.entries.len(), 1);
    assert_eq!(chunk.entries[0].seq, 0);
    assert_eq!(chunk.entries[0].value["text"], "the prince is dead");
}

#[test]
fn a_failed_command_records_nothing() {
    let (mut tw, alice) = toy_world();
    // Empty arg → the handler errors before appending; nothing persists and
    // the actor is not dirtied.
    tw.run(alice, "+toylog   ");
    let batch = tw.drain_write_batch();
    assert!(
        !batch.appended_to(NS, "main"),
        "a command that errored before the append must record nothing"
    );
    assert!(!batch.dirty.contains(&alice));
}

#[test]
fn ledger_tail_rehydrates_on_reboot() {
    let (mut tw, alice) = toy_world();
    for line in ["one", "two", "three"] {
        tw.run(alice, &format!("+toylog {line}"));
    }
    // Fold every persisted JsonSet into a kv map (later writes win), as the
    // store would hold it, then boot a fresh world's toy module from it.
    let mut kv: HashMap<SmolStr, serde_json::Value> = HashMap::new();
    for op in tw.take_module_ops() {
        if op.ns.as_str() == NS {
            match op.op {
                ModuleWrite::JsonSet { key, value } => {
                    kv.insert(key, value);
                }
                ModuleWrite::Del { key } => {
                    kv.remove(&key);
                }
                ModuleWrite::StreamAppend { .. } => {}
            }
        }
    }
    let boot = ModuleBootData {
        kv: kv.into_iter().collect(),
        streams: HashMap::new(),
    };

    let mut tw2 = TestWorld::with_modules(vec![Box::new(ToyModule)]);
    tw2.spawn_player("Alice");
    tw2.modules_loaded(HashMap::from([(SmolStr::new(NS), boot)]));

    let ledger = &tw2
        .world()
        .module_state::<ToyState>()
        .expect("toy state after reboot")
        .ledger;
    assert_eq!(
        ledger.next_seq("main"),
        3,
        "the sequence resumes past all three"
    );
    // The next append continues the sequence and rewrites the same open chunk.
    let alice2 = tw2
        .world()
        .resolve_player("Alice")
        .expect("alice after reboot");
    tw2.run(alice2, "+toylog four");
    let ledger = &tw2.world().module_state::<ToyState>().unwrap().ledger;
    assert_eq!(ledger.next_seq("main"), 4);
    assert_eq!(ledger.open_entries("main").last().unwrap().seq, 3);
}
