//! **`@registry/inspect <hull>`: the Registry reads a hull** (doc 17h cut M8a, A4; doc 17
//! §13; doc 23 §7). The tacsim half of
//! [`mush_roll::commerce::registry_inspection`]: the action and outcome definitions under
//! this crate's owner (the Roll's policy names them), the provider that reads the hull in
//! one transaction and lands the Roll's record through the writer the Roll installed, the
//! command that prints the same facts and submits the action, and the slot that holds the
//! registered writer for both.
//!
//! * [`crate::minds::registry_inspection::facts`] — the one read (`read`): the fitted core (`crate::arena::core::read_core`),
//!   the core's provenance ([`crate::minds::provenance`]), the open trip's seal facts (a
//!   rig's `RigRecord::sortie`, a ship's `tacsim.ship_epoch`) and the seal checked against
//!   the hardware as the IC audit checks it, as a read — and the reply lines over it.
//! * `provider` — `registry_mind_inspection`'s provider: parse `{"hull": <n>}`, read,
//!   record (M8a's A8: one write), answer `{"inspection", "hull"}`.
//! * [`crate::minds::registry_inspection::command`] — the command body `market_commands::registry` dispatches to.
//!
//! **Composition**: `MindCommerce::with_registry_inspection` makes the writer and hands the
//! Roll its policy; `configure_tacsim` hands the layer this [`crate::minds::registry_inspection::Composition`]; the layer's
//! `register` puts the two descriptors on the crate's one action and one outcome validator
//! (`crate::port_tick`) and registers the provider and the slot; its boot hook captures the
//! readiness. The named launch composes it beside recall and revocation
//! (`crate::launch::market`).

pub mod command;
pub mod facts;
mod provider;

use std::sync::{Arc, Mutex};

use mush_core::{DefinitionDescriptor, DefinitionRef, ModError, Registrar};
pub use mush_roll::commerce::registry_inspection::{
    HullClass, InspectionFacts, InspectionRecord, InspectionWriter, Provenance,
    REGISTRY_INSPECTION_CAPABILITY, RegisteredInspectionWriter, RegistryInspectionPolicy,
    SealCheck, UngovernedNote,
};

/// The inspection action's id (the Roll's policy's).
pub const INSPECTION_ACTION: &str = "registry_mind_inspection";
/// The inspection outcome's id.
pub const INSPECTION_OUTCOME: &str = "registry_mind_inspected";

/// What `MindCommerce` composed: the policy and the shared writer cell.
#[derive(Clone)]
pub struct Composition {
    pub(crate) policy: RegistryInspectionPolicy,
    pub(crate) writer: InspectionWriter,
}

impl Composition {
    /// The action and outcome descriptors, for the crate's validators and registration.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub(crate) fn descriptors(&self) -> Result<[DefinitionDescriptor; 2], ModError> {
        self.policy.tacsim_descriptors()
    }

    /// Register the provider over the registered `action` and `outcome`, and the slot.
    ///
    /// # Errors
    /// A registrar that is not this crate's, or a reference that is not the policy's.
    pub(crate) fn register(
        &self,
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        outcome: DefinitionRef,
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS
            || action != self.policy.action()?
            || outcome != self.policy.outcome()?
        {
            return Err(ModError::msg(
                "Registry inspection registration differs from its composition",
            ));
        }
        let writer = self.writer.for_action(reg, &action)?;
        reg.action_provider(
            action.clone(),
            provider::Provider::new(action, outcome, writer.clone())?,
        )?;
        reg.state(InspectionHost {
            writer,
            attempts: Arc::new(Mutex::new((i64::MIN, 0))),
        });
        Ok(())
    }
}

/// **The slot** this crate registers when the inspection composes: the writer bound to the
/// registered action (the command's offer test and submission, the boot's readiness
/// capture) and the command's idempotency attempts (`market_commands::next_key`'s
/// counter, RAM only).
#[derive(Clone)]
pub struct InspectionHost {
    pub(crate) writer: RegisteredInspectionWriter,
    pub(crate) attempts: Arc<Mutex<(i64, u64)>>,
}

impl InspectionHost {
    /// The slot in `world`, owner-checked (the command slot's rule).
    #[must_use]
    pub fn of(world: &mush_core::World) -> Option<&InspectionHost> {
        if world
            .module_state_owner(std::any::TypeId::of::<InspectionHost>())
            .map(|owner| owner.as_str())
            != Some(crate::NS)
        {
            return None;
        }
        world.module_state::<InspectionHost>()
    }

    /// The registered writer: the record family for a read of what an inspection wrote.
    #[must_use]
    pub fn writer(&self) -> &RegisteredInspectionWriter {
        &self.writer
    }
}

/// **The boot hook's half**: capture the readiness once the Roll, Org and Economy owners
/// are ready. A roster without the slot captures nothing.
///
/// # Errors
/// An owner not ready, or a second capture.
pub(crate) fn capture(ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
    if let Some(host) = ctx.state::<InspectionHost>() {
        host.writer.capture(ctx)?;
    }
    Ok(())
}
