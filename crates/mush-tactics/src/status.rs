//! # `unit.status` — a PROJECTION, never a document (E7 cut 8 B2; D-E7-37)
//!
//! 08i answer 7 and 10b §3.8 are explicit: the production unit family is
//! `TacticalUnit` **only**, and `unit.status` is *"composed at read time from the
//! unit family, cut 6's placement, tactics' `ContactState` and the authorized-
//! station data of the viewer's grant"*. It is **never stored**: there is no
//! `status` key, no `status` document, no `status` field on any record, and
//! nothing in this file writes anything.
//!
//! That is the whole point. A stored status would be a second truth about a unit
//! that could drift from the four things it is derived from, and §4.1's closing
//! rule ("an exempt record may not later take part in a prepared guard, an offer
//! or a legality check") is exactly the trap a stored status walks into.
//!
//! ## The four sources, and what each one is on this tree
//!
//! | source | read from | why not the obvious thing |
//! |---|---|---|
//! | the unit | [`UnitStore`](crate::UnitStore) | the typed family migration is E7k's (§3.8 As built) |
//! | placement | `Obj.location` | `Obj.location` **is** the maintained projection of placement (design D1); reading it needs no `mush-sim` edge, and cut 6's `in_transit_room`/`recovery_room` projections answer through it too |
//! | contact | [`ContactLedgerState`](mush_core::ContactLedgerState) | the leak-safe projection cut 7 publishes; the family itself is owner-fenced and a *viewer* is not the owner |
//! | authorized station | `World::channel_grant` | K5a's grant, checked for the exact viewer |
//!
//! ## What a viewer may not learn
//!
//! A viewer with **no** authorized station on the unit sees the same status a
//! viewer of a unit that does not exist sees: [`UnitStatus::unknown`], whose
//! rendering is byte-identical in both cases (§4 rule 4). Contact counts are the
//! *viewer's own* ledger rows, never the unit's, so one station's picture never
//! leaks through another's status line.

use mush_core::{ContactLedgerState, ObjId, OwnerId, PrincipalId, World};

use crate::UnitStore;
use crate::unit::TacticalUnit;

/// The projected status of one unit, for one viewer, at one moment.
///
/// Deliberately **not** `Serialize`: a projection that could be persisted would
/// eventually be persisted. Renders go through [`UnitStatus::render`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitStatus {
    /// Whether the viewer is authorized to see anything at all.
    pub authorized: bool,
    /// The unit's own revision, or `0` when unknown.
    pub revision: u64,
    /// How many sections the unit carries.
    pub sections: usize,
    /// How many installed components it carries.
    pub components: usize,
    /// The room the unit projects into — placement's own projection (design D1).
    pub location: Option<ObjId>,
    /// How many live contacts the **viewer** holds.
    pub contacts: usize,
}

impl UnitStatus {
    /// The status a viewer sees for a unit they hold no station on — and,
    /// byte-identically, for a unit that does not exist.
    #[must_use]
    pub const fn unknown() -> UnitStatus {
        UnitStatus {
            authorized: false,
            revision: 0,
            sections: 0,
            components: 0,
            location: None,
            contacts: 0,
        }
    }

    /// One line, for a panel or a command.
    #[must_use]
    pub fn render(&self) -> String {
        if !self.authorized {
            return "You have no station on that unit.".to_owned();
        }
        let location = self
            .location
            .map_or_else(|| "nowhere".to_owned(), |id| format!("#{}", id.0));
        format!(
            "rev {} — {} section(s), {} component(s), at {}; you hold {} contact(s).",
            self.revision, self.sections, self.components, location, self.contacts
        )
    }
}

/// Whether `viewer` holds an authorized station on `unit`.
///
/// K5a's `ChannelGrant` is per `(actor, subject, owner, station)`; a viewer with
/// **any** live grant on the unit under the `sim` owner is authorized to read
/// its status. There is no per-station `ProjectionScopeId` — 08i answer 11
/// forbids one — so the check is "does this viewer hold a station here", not
/// "which station".
#[must_use]
pub fn viewer_is_stationed(world: &World, viewer: ObjId, unit: ObjId) -> bool {
    world
        .command_subjects_of(viewer)
        .iter()
        .any(|grant| grant.subject == unit)
}

/// **Compose** the status. Reads four sources and writes nothing.
#[must_use]
pub fn unit_status(world: &World, viewer: ObjId, unit: ObjId) -> UnitStatus {
    if !viewer_is_stationed(world, viewer, unit) {
        return UnitStatus::unknown();
    }
    let Some(tactical) = world
        .module_state::<UnitStore>()
        .and_then(|store| store.units.get(&unit))
    else {
        // Authorized, but there is no unit there. Byte-identical to the
        // unauthorized case: a station grant on a husk must not confirm that
        // the husk was ever a unit.
        return UnitStatus::unknown();
    };
    compose(world, viewer, tactical)
}

/// The same composition over a unit the caller already holds — the arm a test
/// uses to prove the projection is not a document, by composing a status for a
/// `TacticalUnit` that was **never persisted** and finding it byte-identical.
#[must_use]
pub fn unit_status_of(world: &World, viewer: ObjId, unit: &TacticalUnit) -> UnitStatus {
    if !viewer_is_stationed(world, viewer, unit.object()) {
        return UnitStatus::unknown();
    }
    compose(world, viewer, unit)
}

fn compose(world: &World, viewer: ObjId, unit: &TacticalUnit) -> UnitStatus {
    let holder = PrincipalId::Object { id: viewer };
    let contacts = world
        .module_state::<ContactLedgerState>()
        .map_or(0, |ledger| {
            ledger
                .rows()
                .iter()
                .filter(|row| row.holder == holder && row.active)
                .count()
        });
    UnitStatus {
        authorized: true,
        revision: unit.revision(),
        sections: unit.sections().count(),
        components: unit.components().count(),
        // Placement's own projection. `mush-sim` maintains it; reading it here
        // needs no edge, and an under-way carrier or a quarantined unit answers
        // through cut 6's `in_transit_room` / `recovery_room` by construction.
        location: world.get(unit.object()).and_then(|obj| obj.location),
        contacts,
    }
}

/// This module's owner, for a caller that needs to name it.
///
/// # Errors
///
/// Never in practice — `NS` is a static canonical id.
pub fn owner() -> Result<OwnerId, mush_core::ModError> {
    OwnerId::new(crate::NS).map_err(|error| mush_core::ModError::msg(error.to_string()))
}
