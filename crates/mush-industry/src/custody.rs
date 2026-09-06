//! E7c3 B1 — this crate's `CustodyProvider`, on E3's rules unchanged.
//!
//! A deposit **is** an inventory endpoint (OPEN 1): one `Vault` per ore body,
//! backed by `CustodyRef::Registered { provider: "industry", id: <deposit> }` —
//! the variant that exists precisely for *"a provider-registered backing with
//! no single world object"*. That backing is what lets the body's unextracted
//! commodity be cut 2's `Pool` under one owner, which is the whole of the
//! one-bundle extraction argument.
//!
//! # What this provider deliberately refuses
//!
//! An ore body holds **quantity, not objects**. You cannot put a crate into a
//! seam of ore, and `move_into`/`move_out` therefore refuse rather than
//! inventing a location: `CustodyRef::Registered::object()` is `None`, so the
//! pure engine has no `to`/`from` to name for it in the first place
//! (`crates/mush-economy/src/plan.rs`'s `BackingInvalid`). `contents` is always
//! empty and `holds` always false — both **`Some`**, because E3's contract is
//! that a `None` from any query is fail-closed drift, and a provider that
//! answers "I don't know" about its own backing is exactly that drift.
//!
//! # And what it deliberately does not depend on
//!
//! `backing_valid` answers `Some(true)` for every well-formed industry backing
//! **without reading industry state**. That is not laziness: `endpoint_state`
//! is called during the economy's own boot, and this crate composes *after* the
//! economy, so a provider that needed this crate's loaded state would answer
//! `false` for every deposit on every boot — the `mush-sim` interior ordering
//! hazard (`crates/mush-server/src/modules.rs`), with the modules in the order
//! the boot pin requires. A registered backing has nothing that can be
//! *unavailable*: it is an abstraction this crate names, and whether the
//! deposit behind it exists is answered by `industry.deposit`, which is where a
//! missing one quarantines.

use mush_core::{Ctx, ObjId, World};
use mush_economy::{CustodyError, CustodyProvider, CustodyRef};

use crate::record::{CUSTODY_PROVIDER, OreDepositId};

/// The custody provider for `CustodyRef::Registered { provider: "industry" }`.
#[derive(Clone, Copy, Debug, Default)]
pub struct IndustryCustodyProvider;

/// The backing one ore body's endpoint vault carries.
#[must_use]
pub fn backing(deposit: OreDepositId) -> CustodyRef {
    CustodyRef::Registered {
        provider: smol_str::SmolStr::new_static(CUSTODY_PROVIDER),
        id: deposit.0,
    }
}

/// The ore body one backing names, if it is one of ours.
#[must_use]
pub fn deposit_of(backing: &CustodyRef) -> Option<OreDepositId> {
    match backing {
        CustodyRef::Registered { provider, id } if provider.as_str() == CUSTODY_PROVIDER => {
            Some(OreDepositId(*id))
        }
        _ => None,
    }
}

impl CustodyProvider for IndustryCustodyProvider {
    fn provider(&self) -> &str {
        CUSTODY_PROVIDER
    }

    fn handles(&self, backing: &CustodyRef) -> bool {
        deposit_of(backing).is_some()
    }

    fn claims(&self, _w: &World, _obj: ObjId) -> bool {
        // This provider claims no world object at all: an ore body is not one.
        // E3's exactly-one-claiming-provider rule is therefore never contested
        // by us, in either direction.
        false
    }

    fn backing_valid(&self, _w: &World, backing: &CustodyRef) -> Option<bool> {
        // `Some`, never `None`: `None` is fail-closed drift, and this provider
        // knows the answer for its own backings without reading any state (see
        // the module note on boot ordering).
        Some(deposit_of(backing).is_some())
    }

    fn contents(&self, _w: &World, backing: &CustodyRef) -> Option<Vec<ObjId>> {
        deposit_of(backing).map(|_| Vec::new())
    }

    fn holds(&self, _w: &World, backing: &CustodyRef, _obj: ObjId) -> Option<bool> {
        deposit_of(backing).map(|_| false)
    }

    fn move_into(
        &self,
        _ctx: &mut Ctx<'_>,
        _actor: ObjId,
        _obj: ObjId,
        _backing: &CustodyRef,
    ) -> Result<(), CustodyError> {
        Err(CustodyError::Refused(
            "an ore body holds quantity, not objects".to_owned(),
        ))
    }

    fn move_out(
        &self,
        _ctx: &mut Ctx<'_>,
        _actor: ObjId,
        _obj: ObjId,
        _backing: &CustodyRef,
        _to: ObjId,
    ) -> Result<(), CustodyError> {
        Err(CustodyError::Refused(
            "an ore body holds quantity, not objects".to_owned(),
        ))
    }
}
