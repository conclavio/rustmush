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
pub mod fine;
pub mod principal;
mod provider;
pub mod seizure;
pub mod unwarranted;

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
    /// Doc 17h cut M8d D3: the levy sink's account when `@registry/fine` composes
    /// (`MindCommerce::with_registry_fines`).
    pub(crate) fines: Option<mush_economy::AccountId>,
    /// Doc 17h cut M8d D4: the revocation term in IC days when `@registry/revoke principal=`
    /// composes (`MindCommerce::with_registry_principal_revocation`).
    pub(crate) principals: Option<u32>,
    /// Doc 17h cut M8d2 S4/S5: the seizure's terms when `@registry/seize` and
    /// `@registry/forfeit` compose (`MindCommerce::with_registry_seizure`).
    pub(crate) seizures: Option<seizure::SeizureTerms>,
}

impl Composition {
    /// Doc 17h cut M8d2: the four seizure descriptors, when the seizure composes.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub(crate) fn seizure_descriptors(
        &self,
    ) -> Result<Option<[DefinitionDescriptor; 4]>, ModError> {
        self.seizures
            .map(|_| self.policy.seizure_descriptors())
            .transpose()
    }
    /// Doc 17h cut M8d2: the economy's closed seizure plan's terms — the two actions and the
    /// Registry org — when the seizure composes.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub(crate) fn economy_seizure(
        &self,
    ) -> Result<Option<mush_economy::registry_filing::seizure::SeizureConfig>, ModError> {
        if self.seizures.is_none() {
            return Ok(None);
        }
        let [seize, _, forfeit, _] = self.policy.seizure_refs()?;
        Ok(Some(
            mush_economy::registry_filing::seizure::SeizureConfig {
                seize,
                forfeit,
                claimant: self.policy.organization(),
            },
        ))
    }
    /// The action and outcome descriptors, for the crate's validators and registration.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub(crate) fn descriptors(&self) -> Result<[DefinitionDescriptor; 2], ModError> {
        self.policy.tacsim_descriptors()
    }

    /// Doc 17h cut M8d D3: the fine's action and outcome descriptors, when it composes.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub(crate) fn principal_descriptors(
        &self,
    ) -> Result<Option<[DefinitionDescriptor; 2]>, ModError> {
        self.principals
            .map(|_| self.policy.principal_descriptors())
            .transpose()
    }

    /// Doc 17h cut M8d D3: the fine's action and outcome descriptors, when it composes.
    pub(crate) fn fine_descriptors(&self) -> Result<Option<[DefinitionDescriptor; 2]>, ModError> {
        self.fines
            .map(|_| self.policy.fine_descriptors())
            .transpose()
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
        fine: Option<(DefinitionRef, DefinitionRef)>,
        principal: Option<(DefinitionRef, DefinitionRef)>,
        seizure: Option<[DefinitionRef; 4]>,
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
        // Doc 17h cut M8d D6: the unwarranted chronicle's observer and tick, on the same slot.
        let pending = unwarranted::Pending::default();
        unwarranted::register(reg, pending.clone())?;
        // Doc 17h cut M8d D3: the fine's family, provider, observer and levy tick.
        let fines = match (fine, self.fines) {
            (Some((fine_action, fine_outcome)), Some(credit)) => {
                if fine_action != self.policy.fine_action()?
                    || fine_outcome != self.policy.fine_outcome()?
                {
                    return Err(ModError::msg(
                        "Registry fine registration differs from its composition",
                    ));
                }
                let family = fine::family(reg)?;
                reg.action_provider(
                    fine_action.clone(),
                    fine::Provider::new(
                        fine_action.clone(),
                        fine_outcome,
                        writer.clone(),
                        family.clone(),
                        credit,
                    )?,
                )?;
                let host = fine::FineHost {
                    action: fine_action,
                    family,
                    pending: fine::Pending::default(),
                    paged: Arc::default(),
                };
                fine::register_levy(reg, &host)?;
                Some(host)
            }
            (None, None) => None,
            _ => {
                return Err(ModError::msg(
                    "Registry fine registration differs from its composition",
                ));
            }
        };
        // Doc 17h cut M8d D4: the principal revocation's provider.
        let principals = match (principal, self.principals) {
            (Some((action, outcome)), Some(days)) => {
                if action != self.policy.principal_action()?
                    || outcome != self.policy.principal_outcome()?
                {
                    return Err(ModError::msg(
                        "Registry principal revocation differs from its composition",
                    ));
                }
                reg.action_provider(
                    action.clone(),
                    principal::Provider::new(action.clone(), outcome, writer.clone(), days)?,
                )?;
                let host = principal::PrincipalHost {
                    action,
                    days,
                    notices: principal::Notices::default(),
                };
                principal::register_notices(reg, &writer, &host.notices)?;
                Some(host)
            }
            (None, None) => None,
            _ => {
                return Err(ModError::msg(
                    "Registry principal revocation differs from its composition",
                ));
            }
        };
        // Doc 17h cut M8d2 S4/S5: the seizure's two providers, each binding the economy leg.
        let seizures = match (seizure, self.seizures) {
            (Some(refs), Some(terms)) => {
                if refs != self.policy.seizure_refs()? {
                    return Err(ModError::msg(
                        "Registry seizure registration differs from its composition",
                    ));
                }
                Some(seizure::register(reg, refs, &writer, terms)?)
            }
            (None, None) => None,
            _ => {
                return Err(ModError::msg(
                    "Registry seizure registration differs from its composition",
                ));
            }
        };
        reg.state(InspectionHost {
            writer,
            attempts: Arc::new(Mutex::new((i64::MIN, 0))),
            pending,
            fines,
            principals,
            seizures,
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
    /// Doc 17h cut M8d D6: the hulls whose inspections changed, not yet chronicled (RAM).
    pub(crate) pending: unwarranted::Pending,
    /// Doc 17h cut M8d D3: the fine, when it composes.
    pub(crate) fines: Option<fine::FineHost>,
    /// Doc 17h cut M8d D4: the principal revocation, when it composes.
    pub(crate) principals: Option<principal::PrincipalHost>,
    /// Doc 17h cut M8d2 S4/S5: the seizure, when it composes.
    pub(crate) seizures: Option<seizure::SeizureHost>,
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

    /// Doc 17h cut M8d D3: the fine, when it composes.
    #[must_use]
    pub fn fines(&self) -> Option<&fine::FineHost> {
        self.fines.as_ref()
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
