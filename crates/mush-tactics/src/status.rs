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
//! | the unit | [`UnitStore`] | the typed family migration is E7k's (§3.8 As built) |
//! | placement | `Obj.location` | `Obj.location` **is** the maintained projection of placement (design D1); reading it needs no `mush-sim` edge, and cut 6's `in_transit_room`/`recovery_room` projections answer through it too |
//! | contact | [`ContactLedgerState`] | the leak-safe projection cut 7 publishes; the family itself is owner-fenced and a *viewer* is not the owner |
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
    /// **Whether a simulation authority holds the unit** (16b cut S.2; doc 16 §3.6
    /// "World-side exclusion": *a deployed unit's `unit.status` answers "deployed"
    /// to a viewer stationed on it and the existence-hiding line to anyone else*).
    ///
    /// When it is set, every other projected field is zero: the arena is the single
    /// writer for the unit's condition cells and its pose, so the world's revision,
    /// section and component counts and `Obj.location` are a *stale* answer, and a
    /// stale answer about a unit in a match is exactly the leak doc 16 §3.6 closes.
    /// The cockpit frame is the only live view, and the pilot in the seat has it.
    pub deployed: bool,
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
            deployed: false,
        }
    }

    /// **The status a stationed viewer sees while an arena holds the unit** (16b cut
    /// S.2): authorized, and nothing but the one word.
    #[must_use]
    pub const fn deployed() -> UnitStatus {
        UnitStatus {
            authorized: true,
            revision: 0,
            sections: 0,
            components: 0,
            location: None,
            contacts: 0,
            deployed: true,
        }
    }

    /// One line, for a panel or a command.
    #[must_use]
    pub fn render(&self) -> String {
        if !self.authorized {
            return UNAUTHORIZED.to_owned();
        }
        if self.deployed {
            return DEPLOYED.to_owned();
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

/// **The existence-hiding line** (§4 rule 4): what a viewer with no station on the
/// unit is told, and byte-identically what a viewer naming a unit that does not
/// exist is told.
///
/// Named as a constant since 16b cut S.2, because it is now the answer to *two*
/// questions — no station, and (for anyone but the pilot) a unit in a match — and
/// the two must not drift into two sentences.
pub const UNAUTHORIZED: &str = "You have no station on that unit.";

/// **The line a stationed viewer gets while an arena holds the unit** (16b cut S.2;
/// doc 16 §3.6: *answers "deployed"*).
///
/// One word of world-side truth and a pointer at the only live view. It names no
/// match and no arena: an admission refusal names those, because the staffer it
/// answers is trying to *write*, and a read has no business learning them.
pub const DEPLOYED: &str =
    "deployed — the arena holds this unit; the cockpit is the only view of it.";

/// Whether `viewer` holds an authorized station on `unit`.
///
/// K5a's `ChannelGrant` is per `(actor, subject, owner, station)`; a viewer with
/// **any** live grant on the unit under the `sim` owner is authorized to read
/// its status. There is no per-station `ProjectionScopeId` — 08i answer 11
/// forbids one — so the check is "does this viewer hold a station here", not
/// "which station".
/// **E7 cut 8 independent verify fold A12:** the owner fence is real now. This
/// used to be `grant.subject == unit` with no `grant.owner` filter, contrary to
/// its own doc two lines above — so any future module publishing a
/// `CommandSubjectLink` on that object would have silently conferred read access
/// to the unit's true revision, section and component counts and `Obj.location`.
/// `World::channel_grant` filters by owner; the two authorization paths now
/// agree.
#[must_use]
pub fn viewer_is_stationed(world: &World, viewer: ObjId, unit: ObjId) -> bool {
    world
        .command_subjects_of(viewer)
        .iter()
        .any(|grant| grant.subject == unit && grant.owner == SIM_OWNER)
}

/// The owner whose grants confer a station on a unit: cut 5's `sim` control
/// links. Named as a string rather than imported, which is how it was written when
/// `mush-tactics` had no `mush-sim` edge at all (deviation (a)).
///
/// **16b cut T.3 gave this crate that edge** — `crate::custody` publishes into
/// `mush_sim::deployed` and reads `mush_sim::pose_of`, because only the crate that
/// owns `TacticalUnit` can hold a check-out snapshot — and cut S.2 reads
/// `mush_sim::deployed::excluded` two functions below. The string stays a string
/// anyway: an owner id is a wire name, and spelling it here keeps the two
/// authorization paths comparable without importing a constant that is about
/// grants rather than about units.
pub const SIM_OWNER: &str = "sim";

/// **Compose** the status. Reads four sources and writes nothing.
#[must_use]
pub fn unit_status(world: &World, viewer: ObjId, unit: ObjId) -> UnitStatus {
    if !viewer_is_stationed(world, viewer, unit) {
        return UnitStatus::unknown();
    }
    // 16b cut S.2, doc 16 §3.6 "World-side exclusion", through the one shared
    // predicate. **After** the authorization check and not before it, so a stranger
    // naming a deployed unit still gets the existence-hiding line and learns
    // nothing — including that the unit is in a match.
    if mush_sim::deployed::excluded(world, mush_sim::deployed::Shown::Unit(unit)) {
        return UnitStatus::deployed();
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
    if mush_sim::deployed::excluded(world, mush_sim::deployed::Shown::Unit(unit.object())) {
        return UnitStatus::deployed();
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
        deployed: false,
    }
}

/// The viewer's highest-priority `sim` station whose subject is an ACTIVE
/// unit — the default subject of the `tactics.unit` and `tactics.vitals`
/// queries (interface plan I8) when the viewer's own object carries no unit.
/// K5a's grant list is already priority-descending, so the first hit is the
/// station a cut-5 focus would narrow to. `None` when the viewer holds none.
#[must_use]
pub fn primary_stationed_unit(world: &World, viewer: ObjId) -> Option<ObjId> {
    let store = world.module_state::<UnitStore>()?;
    world
        .command_subjects_of(viewer)
        .iter()
        .filter(|grant| grant.owner == SIM_OWNER)
        .map(|grant| grant.subject)
        .find(|subject| store.units.contains_key(subject))
}

/// Every CONNECTED actor holding a `sim` station on `unit`, in id order, at
/// most `cap` — the after-action audience of `@unit/damage` (interface plan
/// I8, D-I8-2). The world keeps no subject→actor index, so this is an honest
/// walk of the connected set; a per-tick caller would want a reverse accessor
/// instead of this.
#[must_use]
pub fn stationed_viewers(world: &World, unit: ObjId, cap: usize) -> Vec<ObjId> {
    let mut viewers: Vec<ObjId> = world
        .connected()
        .filter(|viewer| viewer_is_stationed(world, *viewer, unit))
        .collect();
    viewers.sort();
    if viewers.len() > cap {
        tracing::warn!(
            unit = unit.0,
            connected = viewers.len(),
            cap,
            "tactics: after-action audience truncated"
        );
        viewers.truncate(cap);
    }
    viewers
}

/// This module's owner, for a caller that needs to name it.
///
/// # Errors
///
/// Never in practice — `NS` is a static canonical id.
pub fn owner() -> Result<OwnerId, mush_core::ModError> {
    OwnerId::new(crate::NS).map_err(|error| mush_core::ModError::msg(error.to_string()))
}
