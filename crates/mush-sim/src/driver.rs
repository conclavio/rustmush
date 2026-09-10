//! **S7 (doc 14 §5.4; doc 06 §6.1) — domain drivers.**
//!
//! `@domain/create <host>=<driver>` names the driver a domain runs under. The
//! passive driver is the simulation's own and does nothing; any other is a
//! **registered** module's — the TacSim ground driver is `mush-tactics`' — and
//! is told when a domain of its kind opens and when it retires, so it can open
//! the window the engagement is fought on and close it after. The record's
//! `driver` is the driver's own definition, so a replayed record names the
//! module that owned it.
//!
//! Registration is once, at the driver's module's boot, into a slot beside the
//! room resolver and the window geometry; a name registered twice, or the
//! passive driver's, is refused.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use mush_core::{Ctx, DefinitionRef, ObjId, World};
use smol_str::SmolStr;

use crate::data::{DomainId, PASSIVE_DRIVER};

/// The most drivers one world registers.
pub const MAX_DOMAIN_DRIVERS: usize = 16;

/// A module's domain driver.
pub trait DomainDriver: Send + Sync {
    /// The word `@domain/create` accepts; lower-case.
    fn name(&self) -> &str;
    /// The definition a domain of this driver records.
    fn definition(&self) -> DefinitionRef;
    /// Create a domain of this driver on `host`, presenting in `presentation`,
    /// and answer its id. A driver whose domain is a window (the ground driver)
    /// opens the window here, since a window is a domain of its own on the host.
    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        host: ObjId,
        presentation: ObjId,
    ) -> Result<DomainId, String>;
    /// A domain of this driver is being retired. Answer `true` when the driver
    /// retired it itself (closing a window retires its domain), `false` to let
    /// `@domain/retire` run the transition.
    fn closed(&self, _ctx: &mut Ctx<'_>, _domain: DomainId) -> bool {
        false
    }
}

/// The registered drivers, by name.
#[derive(Clone, Default)]
pub struct DomainDrivers {
    drivers: BTreeMap<SmolStr, Arc<dyn DomainDriver>>,
}

impl fmt::Debug for DomainDrivers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DomainDrivers")
            .field("names", &self.drivers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Register a driver. Once, at its module's boot.
///
/// # Errors
///
/// An empty or passive name, a name already registered, a full slot, or a world
/// without the simulation.
pub fn register_domain_driver(w: &mut World, driver: Arc<dyn DomainDriver>) -> Result<(), String> {
    let name = SmolStr::new(driver.name().trim().to_ascii_lowercase());
    if name.is_empty() {
        return Err("a domain driver has a name".to_string());
    }
    if name == PASSIVE_DRIVER {
        return Err(format!("`{PASSIVE_DRIVER}` is the simulation's own driver"));
    }
    let slot = w
        .module_state_mut::<DomainDrivers>()
        .ok_or_else(|| "the simulation is not composed on this world".to_string())?;
    if slot.drivers.contains_key(&name) {
        return Err(format!("domain driver `{name}` is already registered"));
    }
    if slot.drivers.len() >= MAX_DOMAIN_DRIVERS {
        return Err(format!("no more than {MAX_DOMAIN_DRIVERS} domain drivers"));
    }
    slot.drivers.insert(name, driver);
    Ok(())
}

/// The driver of that name, if one is registered.
#[must_use]
pub fn driver_named(w: &World, name: &str) -> Option<Arc<dyn DomainDriver>> {
    let name = name.trim().to_ascii_lowercase();
    w.module_state::<DomainDrivers>()?
        .drivers
        .get(name.as_str())
        .cloned()
}

/// The driver whose definition a domain records, if one is registered.
#[must_use]
pub fn driver_of(w: &World, definition: &DefinitionRef) -> Option<Arc<dyn DomainDriver>> {
    w.module_state::<DomainDrivers>()?
        .drivers
        .values()
        .find(|d| &d.definition() == definition)
        .cloned()
}

/// Every registered driver's name, in order.
#[must_use]
pub fn driver_names(w: &World) -> Vec<String> {
    w.module_state::<DomainDrivers>()
        .map(|s| s.drivers.keys().map(ToString::to_string).collect())
        .unwrap_or_default()
}
