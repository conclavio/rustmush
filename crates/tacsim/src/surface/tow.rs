//! **The tow** (doc 36 cut GC.4; Q-36-12, Q-36-13, Q-36-14, Q-36-15, Q-36-20, Q-36-21, Q-36-52's
//! friendly half, Q-36-54; doc 27 FO-1, FO-3, FO-13, Q-S5–Q-S9).
//!
//! Any mobile rig takes a casualty or a wreck lying within a cell of it in tow, drags it at the
//! tow rate, and loads it into a carrier's bay at the carrier's ramp. Three surface acts on the IC
//! tick, open to every duty (Q-36-14):
//!
//! - **`+tv/<n> tow [#unit] [go]`**: inspect and attach, one timed act of
//!   `[tacsim.recovery].inspect_s + attach_s` (Q-36-13). The preview is the inspection's report;
//!   the attachment lands at its due instant through the library's own rules
//!   ([`RecoveryBook::attach`]: the 2× tonnage cap, the reach, one tow a tug and one tug a
//!   casualty, the first attachment winning, Q-S9), and GC.3's carrier floor
//!   ([`crate::field::carrier::refuse_tow`]).
//! - **`+tv/<n> release [go]`**: let go, at once. The casualty lies where the tow stands. A
//!   release also interrupts an attach or a load in progress.
//! - **`+tv/<n> load [go]`**: from within a cell of a carrier's ramp, a timed act of
//!   `[tacsim.recovery].load_s`, checked against the bay's room: its item cap and its
//!   `si.mass_kg` and `si.volume_l` limits ([`BayRoom`], read fresh off the bay's contents; Q-S8,
//!   Q-36-15, Q-36-54). Into a carrier of the casualty's own party or a Friend's (its own bay, a
//!   party-mate's, a Friend's) it is a recovery; into anyone else's (doc 36 cut GC.5, Q-36-16) it
//!   puts the casualty in the carrier's operator's custody with a claim owed
//!   ([`crate::surface::salvage`]), and a written-off lease rig goes back to Occator's shed with
//!   no claim (Q-36-29).
//! - **`+tv/<n> pickup [n] [go]`** (doc 36 cut GC.5, Q-36-19): a crate within a cell into the
//!   rig's hopper, at once ([`crate::surface::salvage`]).
//!
//! **The coupled drive.** A tug in tow drives with the `drive` act it always had, at
//! [`crate::field_ops::coupled_drive_rates_for_tow`] (`crate::surface::act::rig_rates` reads the
//! coupling, so every leg, the reserve and the trip home are planned at the tow rate); the tick
//! that stands the tug at its leg's end stands its casualty on the same cell. A tug in tow does
//! nothing else but release or load: no scan, no sounding, no mining, no recover (the command
//! refuses, and the act's start refuses again in its own transaction).
//!
//! **Where it is kept.** On the site's own record (`tacsim.field`, [`SiteRecord::tows`]): the
//! library's [`RecoveryBook`] of the site's tows, and each tug's timed act in flight
//! ([`TowAct`]). One action, `tacsim_surface_tow`, writes it: a start (`attach`, `load`,
//! `release`) submitted by the command as the operator, and a finish (`attached`, `loaded`)
//! submitted by the surface tick as that operator once the act is due. The record is durable, so
//! **a restart while attached keeps the attachment and a restart while loading keeps the load**:
//! the tick finds the act due again and finishes it. The facts a prepare checks are fixed by the
//! offer from a server-written RAM order (`TowOrders`, the drive's `ActOrders` precedent), which
//! decides nothing the prepare does not re-check against the record in its own transaction.
//!
//! **Interruption** (Q-36-13): an attach or a load due on a promoted site lands as interrupted,
//! and a release interrupts either. A load's casualty is put in the bay by the tick only on an
//! unpromoted site, before the finish is submitted: the placement is the load's physical
//! completion, and the finish then records it whatever has happened since.
//!
//! **A tow in a promoted field** (Q-36-20) is admitted coupled (`crate::field::admit`): the tug
//! enters the kernel at the tow rate on wall time, and the casualty rides as its load, held off
//! the kernel's ground, so it is neither seen nor struck while carried. When the field settles,
//! the casualty stands at the tug's cell, still in tow (`crate::field::close`); a tug the field
//! left a casualty or a wreck ends the tow there ([`SiteRecord::demote`]). Releasing inside a
//! promoted field is not built: every surface act on a promoted site waits (Q-CF-30).
//!
//! **The load** closes the casualty's epoch `RecoveredToBase { carrier }`, naming the tug as its
//! recoverer, and takes it off the site's unit index (Q-36-17). Its licence and compute seats
//! stay with it until then (Q-36-21): nothing in the tow touches them. A completed load into a
//! carrier of the crew's own party or a Friend's sets every character still aboard the casualty
//! aboard the carrier, and cancels a pending extraction (Q-36-52's friendly half). A completed
//! load into anyone else's carrier abandons them: the extraction starts for each on the spot,
//! with no recovery-flight deferral (Q-36-52, doc 36 cut GC.5; `crate::field::crew`).

use std::collections::BTreeMap;

use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalId, CmdError, Ctx, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    IdempotencyKeyText, ModError, ObjId, PerceivedRef, ProjectedParameterKind, Registrar,
    RulesValue, StateFamilyHandle, World,
};
use mush_roll::commerce::ic_claims::IcCloseKind;
use mush_tactics::TacticalUnit;
use mush_tactics::field_ops::{
    AttachTowRequest, BayRoom, DEFAULT_MAX_ATTACH_DISTANCE_CM, FieldPose, FieldUnitAssessment,
    LoadIntoBayRequest, OperationalState, RecoveryBook, RecoveryError, RecoveryOperation,
    TowRigProfile, UnitMassProfile,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::port::families::PortHandles;
use crate::port_tick::port_error;
use crate::store::TacsimStore;
use crate::surface::act::{self, Where};
use crate::surface::excursion::{Charge, RigKey, RigRecord, cells_apart, site_cell};
use crate::surface::site_record::{SiteKey, SiteRecord};

/// The action's id.
pub const TOW_ACTION: &str = "tacsim_surface_tow";
/// Its outcome.
pub const TOW_OUTCOME: &str = "tacsim_surface_tow_result";
/// The most timed tow acts one site's record holds (one a tug).
pub const MAX_TOW_ACTS: usize = 64;
/// **A tow's reach**, centimetres: a casualty on a neighbouring cell, diagonals included (a cell
/// is 10 m, its diagonal 14.1 m).
pub const TOW_REACH_CM: u64 = 1_500;

/// The tow's verbs, for `+tv`'s parse.
pub const TOW_WORDS: [&str; 5] = ["load", "pickup", "release", "scrap", "tow"];
/// `+tv/<n> tow` with no rig of the actor's at hand.
pub const TOW_USAGE: &str = "Usage: +tv/<n> tow [#unit] [go], +tv/<n> release [go], +tv/<n> load [go], +tv/<n> pickup [n] [go] or +tv/<n> scrap [n] [go].";
/// The tow's preview (the inspection): `{n}`, `{name}`, `{t}`, `{state}`, `{speed}`, `{min}`.
pub const TOW_PREVIEW: &str = "Rig {n} looks the {name} over: {t} t, {state}. In tow it moves at {speed} m/s. Inspecting and making fast take about {min} IC minutes. +tv/{n} tow go";
/// The tow filed: `{n}`, `{name}`, `{min}`.
pub const TOW_FILED: &str =
    "Rig {n} inspects the {name} and makes fast; the tow is attached in about {min} IC minutes.";
/// The tow attached: `{n}`, `{name}`, `{speed}`.
pub const TOW_ATTACHED: &str = "Rig {n} has the {name} in tow, at {speed} m/s. Drive it to a carrier's ramp and +tv/{n} load go, or +tv/{n} release go.";
/// The tow refused at its landing: `{n}`, `{name}`, `{why}`.
pub const TOW_REFUSED: &str = "Rig {n} could not take the {name} in tow: {why}.";
/// An attach or a load interrupted: `{n}`, `{name}`.
pub const TOW_INTERRUPTED: &str =
    "Rig {n}'s work on the {name} was interrupted; nothing changed hands.";
/// The release: `{n}`, `{name}`.
pub const RELEASED: &str = "Rig {n} lets go of the {name}; it lies where it stands.";
/// The load's preview: `{n}`, `{name}`, `{carrier}`, `{min}`.
pub const LOAD_PREVIEW: &str = "Rig {n} can load the {name} into the {carrier}'s bay; it takes about {min} IC minutes. +tv/{n} load go";
/// The load filed: `{n}`, `{name}`, `{carrier}`, `{min}`.
pub const LOAD_FILED: &str =
    "Rig {n} hauls the {name} up the {carrier}'s ramp; it is aboard in about {min} IC minutes.";
/// The load done: `{name}`, `{carrier}`.
pub const LOADED: &str = "The {name} is aboard the {carrier}, in its bay.";
/// **Doc 36 cut GC.5**: the load's preview into a carrier not of the casualty's party or a
/// Friend's (Q-36-16): `{n}`, `{name}`, `{carrier}`, `{min}`.
pub const LOAD_PREVIEW_FOREIGN: &str = "Rig {n} can load the {name} into the {carrier}'s bay; it takes about {min} IC minutes. The {name} is not of the {carrier}'s party or a Friend's: aboard, it is in the {carrier}'s custody with a salvage claim owed, never its title, and anyone still inside it is abandoned to extraction. +tv/{n} load go";
/// **Doc 36 cut GC.5**: the load's preview of a written-off lease rig (Q-36-29): `{n}`,
/// `{name}`, `{carrier}`, `{min}`.
pub const LOAD_PREVIEW_HAND_BACK: &str = "Rig {n} can load the {name} into the {carrier}'s bay; it takes about {min} IC minutes. The {name} is an Occator write-off: loaded, it goes back to Occator, with no claim. +tv/{n} load go";
/// **Doc 36 cut GC.5**: the load done into a salvor's custody: `{name}`, `{carrier}`.
pub const LOADED_CUSTODY: &str =
    "The {name} is aboard the {carrier}, in its bay, held in custody pending a salvage claim.";
/// **Doc 36 cut GC.5**: a written-off lease rig handed back: `{name}`.
pub const HANDED_BACK: &str =
    "The {name} is an Occator write-off: it is handed back to Occator, with no claim.";
/// The crew aboard a loaded casualty (Q-36-52's friendly half): `{name}`, `{carrier}`.
pub const CREW_ABOARD: &str =
    "The {name} is hauled into the {carrier}'s bay with you in it: you are aboard the {carrier}.";
/// Nothing to tow: `{n}`.
pub const NOTHING_IN_REACH: &str =
    "Nothing rig {n} could tow lies within a cell of it: casualties and wrecks only.";
/// The named unit is not in reach: `{n}`, `{name}`.
pub const NOT_IN_REACH: &str = "The {name} is not a casualty or a wreck within a cell of rig {n}.";
/// Not towing: `{n}`.
pub const NOT_TOWING: &str = "Rig {n} has nothing in tow.";
/// Busy with a tow act: `{n}`.
pub const TOW_BUSY: &str =
    "Rig {n} is busy with its tow; wait for it, or +tv/{n} release go to break it off.";
/// A tug may only drive, release or load: `{n}`.
pub const IN_TOW: &str = "Rig {n} has a casualty in tow: it can drive, +tv/{n} load go at a ramp, or +tv/{n} release go.";
/// Not beside a ramp: `{n}`.
pub const NOT_AT_RAMP: &str =
    "Rig {n} is not within a cell of a carrier's ramp on this site: drive the tow there first.";
/// The bay has no room: `{carrier}`, `{name}`, `{why}`.
pub const BAY_NO_ROOM: &str = "The {carrier}'s bay has no room for the {name}: {why}.";
/// **Doc 36a lane 3 (P0-5's rider)**: the only carrier in reach is a stranger's. A tug loads
/// into its own carrier's bay or a Friend's, never a stranger's.
pub const LOAD_NOT_YOUR_CARRIER: &str =
    "Rig {n} may only load into its own carrier's bay or a Friend's; the {carrier} is neither.";
/// The rig is not on the ground: `{n}`.
pub const NOT_ON_GROUND: &str = "Rig {n} is not on a site's ground.";
/// **Doc 36a P1-4**: another tug's attach is in flight on the casualty: `{n}`, `{name}`.
pub const ATTACHING_ELSEWHERE: &str =
    "Another tug is making fast to the {name} already; rig {n} waits for that to land or let go.";
/// Its own state forbids towing: `{n}`.
pub const TUG_NOT_MOBILE: &str =
    "Rig {n} cannot tow: it must be whole enough to drive under its own power.";

// ---- the record ---------------------------------------------------------------------------

/// **What a timed tow act does at its due instant** (Q-36-13).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
pub enum TowStage {
    /// Inspect and attach to `casualty`.
    Attach {
        /// The casualty.
        casualty: ObjId,
    },
    /// Load the tow into `carrier`'s bay.
    Load {
        /// The carrier.
        carrier: ObjId,
    },
    /// **Doc 41 cut RS.1**: tend `target` ([`crate::surface::tender`]).
    Tend {
        /// The unit tended.
        target: ObjId,
        /// The work.
        work: crate::surface::tender::TendWork,
        /// IC seconds left when a field paused it (Q-41-3); `None` while it runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        paused: Option<i64>,
    },
}

/// **One tug's timed tow act in flight** (Q-36-13).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TowAct {
    /// Issued by the site's record ([`Tows::issued`]); the finish's replay guard.
    pub ordinal: u32,
    /// The tug.
    pub tug: ObjId,
    /// Who set it going; its finish is submitted as them.
    pub operator: ObjId,
    /// What it does.
    pub stage: TowStage,
    /// When it started, IC.
    pub started_ic: i64,
    /// When it lands, IC.
    pub due_ic: i64,
}

/// **A site's tows** ([`SiteRecord::tows`]): the library's book of the site's tows, each tug's
/// timed act in flight, and the last act ordinal issued.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tows {
    /// The site's tows: active ones only; a released or loaded tow is retired from it.
    #[serde(default)]
    pub book: RecoveryBook,
    /// The timed acts in flight, strictly increasing by tug, at most [`MAX_TOW_ACTS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acts: Vec<TowAct>,
    /// The last act ordinal issued (0: none).
    #[serde(default)]
    pub issued: u32,
    /// **Doc 36 cut GC.6** (Q-S3, Q-36-26): the tenure each attach pinned, one a casualty, in
    /// order, at most [`MAX_TOW_ACTS`] ([`crate::surface::claim::TenurePin`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<crate::surface::claim::TenurePin>,
}

impl Tows {
    /// Whether nothing was ever towed here, so the record omits it.
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        self.issued == 0 && self.acts.is_empty() && self.book.is_empty() && self.pins.is_empty()
    }

    /// The record's own invariants.
    ///
    /// # Errors
    ///
    /// The first broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.acts.len() > MAX_TOW_ACTS
            || self.acts.windows(2).any(|pair| match pair {
                [a, b] => a.tug >= b.tug,
                _ => false,
            })
        {
            return Err(format!(
                "a site's tow acts are at most {MAX_TOW_ACTS}, one a tug, in order"
            ));
        }
        if self
            .acts
            .iter()
            .any(|a| a.ordinal == 0 || a.ordinal > self.issued || a.due_ic < a.started_ic)
        {
            return Err("a tow act is one the record issued, due after it started".to_owned());
        }
        if self.pins.len() > MAX_TOW_ACTS
            || self.pins.windows(2).any(|pair| match pair {
                [a, b] => a.casualty >= b.casualty,
                _ => false,
            })
        {
            return Err(format!(
                "a site's tenure pins are at most {MAX_TOW_ACTS}, one a casualty, in order"
            ));
        }
        Ok(())
    }

    /// The timed act `tug` has in flight.
    #[must_use]
    pub fn act_of(&self, tug: ObjId) -> Option<&TowAct> {
        self.acts.iter().find(|a| a.tug == tug)
    }

    /// The tow `tug` holds.
    #[must_use]
    pub fn towing(&self, tug: ObjId) -> Option<&RecoveryOperation> {
        self.book.active_operation_for_tug(tug)
    }

    /// The tow that holds `casualty`.
    #[must_use]
    pub fn towed(&self, casualty: ObjId) -> Option<&RecoveryOperation> {
        self.book.active_operation_for_casualty(casualty)
    }

    /// Every active tow, in id order.
    pub fn active(&self) -> impl Iterator<Item = &RecoveryOperation> {
        self.book
            .operations()
            .map(|(_, op)| op)
            .filter(|op| op.stage().is_active())
    }

    /// **Whether another tug than `tug` has an attach in flight on `casualty`** (doc 36a
    /// P1-4): a second attach start waits for it, so the pin the first wrote is never replaced
    /// under it.
    #[must_use]
    pub fn attaching_elsewhere(&self, tug: ObjId, casualty: ObjId) -> bool {
        self.acts
            .iter()
            .any(|act| act.tug != tug && act.stage == TowStage::Attach { casualty })
    }

    /// Take `tug`'s act out.
    pub fn take_act(&mut self, tug: ObjId) -> Option<TowAct> {
        let at = self.acts.iter().position(|a| a.tug == tug)?;
        Some(self.acts.remove(at))
    }

    /// Put an act in, issuing its ordinal.
    ///
    /// # Errors
    ///
    /// The tug has an act already, the cap is reached, or the ordinals are spent.
    pub fn put_act(
        &mut self,
        tug: ObjId,
        operator: ObjId,
        stage: TowStage,
        (started_ic, due_ic): (i64, i64),
    ) -> Result<u32, &'static str> {
        if self.act_of(tug).is_some() {
            return Err("tow_busy");
        }
        if self.acts.len() >= MAX_TOW_ACTS {
            return Err("tow_full");
        }
        let ordinal = self.issued.checked_add(1).ok_or("tow_full")?;
        self.issued = ordinal;
        let at = self.acts.partition_point(|a| a.tug < tug);
        self.acts.insert(
            at,
            TowAct {
                ordinal,
                tug,
                operator,
                stage,
                started_ic,
                due_ic: due_ic.max(started_ic),
            },
        );
        Ok(ordinal)
    }

    /// **End every tow `unit` is in** (as tug or casualty) at `at_ic`: detached and retired, and
    /// any act it or its casualty has in flight dropped. Answers whether anything ended.
    pub fn end(&mut self, unit: ObjId, at_ic: i64) -> bool {
        let mut ended = false;
        let ops: Vec<(mush_tactics::field_ops::RecoveryOpId, u64, ObjId)> = self
            .active()
            .filter(|op| op.tug() == unit || op.casualty() == unit)
            .map(|op| (op.id(), op.revision(), op.tug()))
            .collect();
        for (id, revision, tug) in ops {
            let at = self
                .book
                .get(id)
                .map_or(at_ic, |op| op.updated_at_ic().max(at_ic));
            if self.book.detach(id, revision, at).is_ok() {
                let _ = self.book.retire(id);
                ended = true;
            }
            let _ = self.take_act(tug);
        }
        if self.take_act(unit).is_some() {
            ended = true;
        }
        let attaching: Vec<ObjId> = self
            .acts
            .iter()
            .filter(|a| a.stage == TowStage::Attach { casualty: unit })
            .map(|a| a.tug)
            .collect();
        for tug in attaching {
            ended |= self.take_act(tug).is_some();
        }
        ended
    }
}

// ---- the facts a prepare checks -----------------------------------------------------------

/// **The tug's facts** at the order's instant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TugFacts {
    /// Its operational state.
    pub state: OperationalState,
    /// Whether it moves under its own power.
    pub can_move: bool,
    /// Its uncoupled base speed, cm/s.
    pub base_speed_cms: u32,
    /// Its mass, kg.
    pub mass_kg: u64,
    /// Where it stands.
    pub pose: FieldPose,
    /// **Doc 41 cut RS.1** (Q-41-27): the tow coupling its definition authors
    /// (`logistics.recovery_profile`), which carries its cap.
    #[serde(default)]
    pub profile: TowRigProfile,
}

/// **An attach's facts**: the tug's and the casualty's, read off the world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachFacts {
    /// The site.
    pub site: CanonicalId,
    /// The tug.
    pub tug: TugFacts,
    /// The casualty.
    pub casualty: ObjId,
    /// Its assessment.
    pub assessment: FieldUnitAssessment,
    /// Its mass profile.
    pub mass: UnitMassProfile,
    /// Where it lies.
    pub pose: FieldPose,
    /// GC.3's carrier floor's line, when the casualty is a carrier the tug is too light for.
    pub carrier_refusal: Option<String>,
    /// The act's length, IC seconds (`[tacsim.recovery].inspect_s + attach_s`), fixed by the
    /// server: the prepare reads no configuration.
    pub secs: u32,
    /// **Doc 36 cut GC.6** (Q-S3): the tenure, title and witnesses the attach's start pins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<crate::surface::claim::TenurePin>,
}

/// **A load's facts**: the carrier's bay, the casualty's measures and where everything stands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadFacts {
    /// The site.
    pub site: CanonicalId,
    /// The carrier.
    pub carrier: ObjId,
    /// The casualty.
    pub casualty: ObjId,
    /// The bay's room, its contents counted but never the casualty.
    pub room: BayRoom,
    /// The casualty's stowage volume, litres (Q-36-54).
    pub volume_l: u64,
    /// The carrier's ramp.
    pub ramp: FieldPose,
    /// Where the tug stands.
    pub tug_pose: FieldPose,
    /// Whether the carrier is the casualty's own party's or a Friend's.
    pub friendly: bool,
    /// **Doc 36a lane 3 (P0-5's rider)**: whether the carrier is the tug's own hull or a Friend's
    /// of the tug's party — the command fixes it, the prepare refuses a load without it.
    #[serde(default)]
    pub own_carrier: bool,
    /// **Doc 36 cut GC.5**: who would hold the casualty in custody (the carrier's operator), when
    /// anyone can be named; the load's operator otherwise.
    pub custodian: Option<ObjId>,
    /// **Doc 36 cut GC.5**: the casualty is a written-off lease rig, Occator's alone (Q-36-29).
    pub written_off: bool,
    /// **Doc 36 cut GC.5**: a written-off casualty stands in Occator's shed (the finish's fact,
    /// as `in_bay` is for any other).
    pub handed_back: bool,
    /// The casualty's unit record now (the library pins its revision across the tow).
    pub casualty_unit: TacticalUnit,
    /// The bay's slot id.
    pub slot: String,
    /// Whether the casualty stands in the carrier's bay (the finish's fact).
    pub in_bay: bool,
    /// The act's length, IC seconds (`[tacsim.recovery].load_s`), fixed by the server.
    pub secs: u32,
}

/// **One server-written order** the offer fixes for its prepare (this module's head).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TowOrder {
    /// `+tv/<n> tow … go`.
    Attach {
        /// The facts.
        facts: Box<AttachFacts>,
    },
    /// The tick's finish of an attach.
    Attached {
        /// The act's ordinal.
        ordinal: u32,
        /// The site.
        site: CanonicalId,
        /// The facts now, `None` when either unit is gone.
        facts: Option<Box<AttachFacts>>,
    },
    /// `+tv/<n> release go`.
    Release {
        /// The site.
        site: CanonicalId,
    },
    /// `+tv/<n> load go`.
    Load {
        /// The facts.
        facts: Box<LoadFacts>,
    },
    /// **Doc 36 cut GC.5**: `+tv/<n> pickup … go`.
    Pickup {
        /// The facts.
        facts: Box<crate::surface::salvage::PickupFacts>,
    },
    /// The tick's finish of a load.
    Loaded {
        /// The act's ordinal.
        ordinal: u32,
        /// The site.
        site: CanonicalId,
        /// The facts now, `None` when a unit is gone.
        facts: Option<Box<LoadFacts>>,
    },
    /// **Doc 36 cut GC.6**: a salvage claim's act (`+salvage`, and the tick's settlement), keyed
    /// by the hull.
    Claim {
        /// The order.
        order: Box<crate::surface::claim::ClaimOrder>,
    },
    /// **Doc 36 cut GC.6**: `+tv/<n> scrap … go`, remains broken up where they lie.
    Scrap {
        /// The facts.
        facts: Box<crate::surface::claim::ScrapFacts>,
    },
    /// **Doc 41 cut RS.1**: `+tv/<n> unjam|stabilize|rearm … go`, a tend act's start.
    Tend {
        /// The facts.
        facts: Box<crate::surface::tender::TendFacts>,
    },
    /// **Doc 41 cut RS.1**: the tick's step on a tend act (its finish, a pause, a resume).
    Tended {
        /// The act's ordinal.
        ordinal: u32,
        /// The site.
        site: CanonicalId,
        /// The step.
        step: crate::surface::tender::TendStep,
    },
    /// **Doc 41 cut RS.1**: `+tv/<n> stores load|open … go`, on the rig's stores and bale; no
    /// site record is read or written.
    Stores {
        /// The facts.
        facts: Box<crate::surface::tender::StoresFacts>,
    },
}

impl TowOrder {
    /// Its `op` word.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Attach { .. } => "attach",
            Self::Attached { .. } => "attached",
            Self::Release { .. } => "release",
            Self::Load { .. } => "load",
            Self::Loaded { .. } => "loaded",
            Self::Pickup { .. } => "pickup",
            Self::Claim { .. } => "claim",
            Self::Scrap { .. } => "scrap",
            Self::Tend { .. } => "tend",
            Self::Tended { .. } => "tended",
            Self::Stores { .. } => "stores",
        }
    }
}

/// **The tow orders** (RAM, keyed by the actor and the tug; doc 36a P1-3): each actor's own
/// order on a rig, so one party's `+salvage/inspect` never clobbers another's queued contest on
/// the same hull. Past [`crate::port::tv::MAX_POINTERS`] rows the oldest written goes, never the
/// lowest id.
#[derive(Clone, Debug, Default)]
pub(crate) struct TowOrders {
    /// `(actor, tug)` → (when it was written, the order).
    rows: BTreeMap<(ObjId, ObjId), (u64, TowOrder)>,
    /// The last write's sequence.
    written: u64,
}

impl TowOrders {
    /// Write `order` as `actor`'s on `tug`, replacing that actor's earlier order on it.
    fn put(&mut self, actor: ObjId, tug: ObjId, order: TowOrder) {
        self.written = self.written.wrapping_add(1);
        self.rows.insert((actor, tug), (self.written, order));
        while self.rows.len() > crate::port::tv::MAX_POINTERS {
            let oldest = self
                .rows
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| *key);
            match oldest {
                Some(key) => {
                    self.rows.remove(&key);
                }
                None => break,
            }
        }
    }

    /// `actor`'s orders, by tug.
    fn of(&self, actor: ObjId) -> impl Iterator<Item = (ObjId, &TowOrder)> {
        self.rows
            .range((actor, ObjId(0))..=(actor, ObjId(u32::MAX)))
            .map(|((_, tug), (_, order))| (*tug, order))
    }
}

/// **The active tows, mirrored in RAM** (refreshed by the tow tick from every lane site's
/// record): tug → (site, its tow). A convenience for the drive's rates and the tick's placement,
/// never a fact a prepare relies on.
#[derive(Clone, Debug, Default)]
pub(crate) struct TowIndex(BTreeMap<ObjId, (CanonicalId, RecoveryOperation)>);

/// **The tows the tick finished and has not yet told** (RAM): tug → (operator, act).
#[derive(Clone, Debug, Default)]
pub(crate) struct TowWatch(BTreeMap<ObjId, (ObjId, TowAct)>);

/// The tow `tug` holds, as the RAM mirror last saw it.
#[must_use]
pub(crate) fn coupled(w: &World, tug: ObjId) -> Option<&RecoveryOperation> {
    w.module_state::<TowIndex>()?.0.get(&tug).map(|(_, op)| op)
}

// ---- world reads --------------------------------------------------------------------------

/// The site's record.
fn record_of(ctx: &Ctx<'_>, site: &CanonicalId) -> Option<SiteRecord> {
    let handles = ctx.w.module_state::<PortHandles>()?;
    let key = SiteKey(site.clone());
    ctx.action_state(&handles.families.sites, &key)
        .ok()
        .flatten()
        .cloned()
        .filter(|record| record.key() == key)
}

/// **Where `unit` stands on a site's ground**: the site, the domain and its pose.
fn on_ground(w: &World, unit: ObjId) -> Option<(CanonicalId, mush_sim::DomainId, FieldPose)> {
    ground_of_unit(w, unit)
}

/// **Where `unit` stands on a site's ground** (the salvage's reader): the site and its pose.
pub(crate) fn ground_pose(w: &World, unit: ObjId) -> Option<(CanonicalId, FieldPose)> {
    ground_of_unit(w, unit).map(|(site, _, pose)| (site, pose))
}

fn ground_of_unit(w: &World, unit: ObjId) -> Option<(CanonicalId, mush_sim::DomainId, FieldPose)> {
    let Where::Ground {
        domain, x_cm, y_cm, ..
    } = act::where_is(w, unit)
    else {
        return None;
    };
    let site = act::site_of_domain(w, domain)?;
    let pose = FieldPose {
        site: ObjId(u32::try_from(domain.0).ok().filter(|d| *d != 0)?),
        x_cm,
        y_cm,
    };
    Some((site, domain, pose))
}

/// A unit's spec, record, assessment and mass profile.
fn unit_facts(
    w: &World,
    unit: ObjId,
) -> Option<(TacticalUnit, FieldUnitAssessment, UnitMassProfile, u32, u64)> {
    let cells = mush_tactics::module::unit_of(w, unit)?.clone();
    let spec = crate::carrier_damage::spec_of(w, &cells)?;
    let assessment = crate::field_ops::assess_tacsim_unit(&cells, Some(spec)).ok()?;
    let mass = crate::field_ops::unit_mass_profile_for_spec(spec, 0, 0).ok()?;
    let speed = crate::field_ops::base_speed_cms_for_spec(spec);
    let volume = crate::field_ops::stowage_volume_l_for_spec(spec);
    Some((cells, assessment, mass, speed, volume))
}

/// **The tow coupling `unit`'s definition authors** (doc 41 cut RS.1, Q-41-27):
/// `logistics.recovery_profile`, `Standard` when it authors none.
#[must_use]
pub(crate) fn profile_of(w: &World, unit: ObjId) -> TowRigProfile {
    use crate::definitions::RecoveryProfile;
    let profile = mush_tactics::module::unit_of(w, unit)
        .and_then(|cells| crate::carrier_damage::spec_of(w, cells))
        .and_then(|spec| spec.logistics.as_ref())
        .map_or(RecoveryProfile::Standard, |l| l.recovery_profile);
    match profile {
        RecoveryProfile::Standard => TowRigProfile::Standard,
        RecoveryProfile::RecoveryOptimized => TowRigProfile::RecoveryOptimized,
        RecoveryProfile::Heavy => TowRigProfile::Heavy,
    }
}

/// **What `unit` carries, kg** (doc 41 cut RS.1, Q-41-29; doc 36's owed "the hopper's ore in a
/// casualty's laden mass"): a rig's hopper or stores; a hull's hold, tank and locker and every
/// rig in its bay, dry and laden, as the passage planner weighs her (`crate::port::voyage`).
/// What will not read counts nothing, logged: the tow is never refused for a gauge.
pub(crate) fn carried_kg(ctx: &Ctx<'_>, unit: ObjId) -> u64 {
    if let Some(view) = crate::port::ship::hull_view(ctx.w, unit) {
        return match crate::port::voyage::basis(ctx, &view, 0) {
            Ok(basis) => basis.payload,
            Err(error) => {
                tracing::warn!(unit = unit.0, %error, "tacsim: a casualty's hold did not weigh; the tow counts her dry");
                0
            }
        };
    }
    if crate::port::vehicle::hopper_of(ctx.w, unit).is_none() {
        return 0;
    }
    match crate::port::tv::hopper_load_of(ctx, unit) {
        Ok((kg, _)) => kg,
        Err(error) => {
            tracing::warn!(unit = unit.0, %error, "tacsim: a casualty's hopper did not weigh; the tow counts her dry");
            0
        }
    }
}

/// **`dry` laden with `cargo_kg`** (Q-41-29): the casualty's mass profile as the tow counts it.
fn laden(dry: UnitMassProfile, cargo_kg: u64) -> UnitMassProfile {
    UnitMassProfile::new(
        dry.dry_mass_kg(),
        cargo_kg,
        dry.cargo_volume_l(),
        dry.width_cm(),
        dry.height_cm(),
    )
    .unwrap_or(dry)
}

/// The tug's facts.
fn tug_facts(w: &World, tug: ObjId) -> Option<(CanonicalId, TugFacts)> {
    let (site, _, pose) = on_ground(w, tug)?;
    let (_, assessment, mass, speed, _) = unit_facts(w, tug)?;
    Some((
        site,
        TugFacts {
            state: assessment.state,
            can_move: assessment.can_move,
            base_speed_cms: speed,
            mass_kg: mass.total_mass_kg(),
            pose,
            profile: profile_of(w, tug),
        },
    ))
}

/// **An attach's facts** for `tug` and `casualty`, when both stand on the same site's ground:
/// the casualty counted laden (doc 41 cut RS.1, Q-41-29).
fn attach_facts(ctx: &Ctx<'_>, tug: ObjId, casualty: ObjId) -> Option<AttachFacts> {
    let w: &World = ctx.w;
    let (site, tug_facts) = tug_facts(w, tug)?;
    let (there, _, pose) = on_ground(w, casualty)?;
    if there != site {
        return None;
    }
    let (_, assessment, dry, _, _) = unit_facts(w, casualty)?;
    let mass = laden(dry, carried_kg(ctx, casualty));
    let cfg = recovery_config(w);
    Some(AttachFacts {
        secs: cfg.inspect_s.saturating_add(cfg.attach_s),
        site,
        tug: tug_facts,
        casualty,
        assessment,
        mass,
        pose,
        carrier_refusal: crate::field::carrier::refuse_tow(w, casualty, tug),
        pin: None,
    })
}

/// The library's attach request for `facts` at `now`.
fn attach_request(tug: ObjId, facts: &AttachFacts, now: i64) -> AttachTowRequest {
    AttachTowRequest {
        tug,
        tug_state: facts.tug.state,
        tug_can_move: facts.tug.can_move,
        tug_base_speed_cms: facts.tug.base_speed_cms,
        tug_mass_kg: facts.tug.mass_kg,
        rig_profile: facts.tug.profile,
        tug_pose: facts.tug.pose,
        casualty_assessment: facts.assessment.clone(),
        casualty_mass: facts.mass,
        casualty_pose: facts.pose,
        max_attach_distance_cm: TOW_REACH_CM.max(DEFAULT_MAX_ATTACH_DISTANCE_CM),
        at_ic: now,
    }
}

/// **The bay's load now**: its item cap and limits, and its contents' count, mass and volume,
/// `except` left out. `None` for a hull with no readable bay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BayLoad {
    /// The bay's room.
    pub room: ObjId,
    /// Its slot id.
    pub slot: mush_sim::SlotId,
    /// Its item cap.
    pub maximum_items: u32,
    /// Its mass limit, kg (`u64::MAX` when it declares none).
    pub mass_limit_kg: u64,
    /// Its volume limit, litres (`u64::MAX` when it declares none).
    pub volume_limit_l: u64,
    /// The units in it.
    pub items: u32,
    /// Their dry mass, kg.
    pub mass_kg: u64,
    /// Their stowage volume, litres.
    pub volume_l: u64,
}

impl BayLoad {
    /// The room left.
    #[must_use]
    pub(crate) fn free(&self) -> BayRoom {
        BayRoom {
            items_free: self.maximum_items.saturating_sub(self.items),
            mass_free_kg: self.mass_limit_kg.saturating_sub(self.mass_kg),
            volume_free_l: self.volume_limit_l.saturating_sub(self.volume_l),
        }
    }

    /// The first limit its contents pass, in words: `None` when within every one.
    #[must_use]
    pub(crate) fn over(&self) -> Option<String> {
        if self.mass_kg > self.mass_limit_kg {
            return Some(format!(
                "its bay holds {} t of hulls, over its {} t",
                self.mass_kg / 1_000,
                self.mass_limit_kg / 1_000
            ));
        }
        if self.volume_l > self.volume_limit_l {
            return Some(format!(
                "its bay holds {} m3 of hulls, over its {} m3",
                self.volume_l / 1_000,
                self.volume_limit_l / 1_000
            ));
        }
        None
    }
}

/// **`hull`'s bay load**, `except` one unit (a casualty being loaded counts as not yet in).
pub(crate) fn bay_load(w: &World, hull: ObjId, except: Option<ObjId>) -> Option<BayLoad> {
    let view = crate::port::ship::hull_view(w, hull)?;
    let (bay, room) = view.bay()?;
    let slot = mush_sim::SlotId::new(bay.slot.id.as_str())?;
    let limit = |measure: &str| {
        bay.slot
            .limits
            .iter()
            .find(|(id, _)| id.as_str() == measure)
            .map_or(u64::MAX, |(_, limit)| *limit)
    };
    let mut load = BayLoad {
        room,
        slot,
        maximum_items: bay.slot.maximum_items,
        mass_limit_kg: limit(crate::definitions::logistics::MASS_KG),
        volume_limit_l: limit(crate::definitions::logistics::VOLUME_L),
        items: 0,
        mass_kg: 0,
        volume_l: 0,
    };
    for rig in crate::port::vehicle::rigs_in_bay(w, &view) {
        if Some(rig.tv) == except {
            continue;
        }
        load.items = load.items.saturating_add(1);
        if let Some((_, _, mass, _, volume)) = unit_facts(w, rig.tv) {
            load.mass_kg = load.mass_kg.saturating_add(mass.dry_mass_kg());
            load.volume_l = load.volume_l.saturating_add(volume);
        }
    }
    Some(load)
}

/// **The departure's bay line** (doc 36 cut GC.4; Q-36-15): `Some(line)` when `hull`'s bay holds
/// more mass or volume of hulls than its limits — the voyage's bay gate counts mass as well as
/// rigs.
#[must_use]
pub(crate) fn bay_over_line(w: &World, hull: ObjId, name: &str) -> Option<String> {
    let over = bay_load(w, hull, None)?.over()?;
    Some(BAY_OVER.replace("{name}", name).replace("{over}", &over))
}

/// The departure refused for an overweight bay: `{name}`, `{over}`.
pub const BAY_OVER: &str = "The {name} cannot lift: {over}. Take a hull out of the bay first.";

/// **The carriers whose ramp is within a cell of `pose` on `site`**, nearest first.
fn carriers_at(w: &World, site: &CanonicalId, pose: &FieldPose) -> Vec<(ObjId, FieldPose)> {
    let mut found: Vec<(u64, ObjId, FieldPose)> = Vec::new();
    for hull in crate::surface::recovery::world_hulls(w) {
        let Some((entry, domain, ramp, _, _)) = crate::surface::tick::hull_view_berth(w, hull)
        else {
            continue;
        };
        if &entry != site || u32::try_from(domain.0).ok() != Some(pose.site.0) {
            continue;
        }
        let apart = cells_apart(
            site_cell(ramp.x_cm, ramp.y_cm),
            site_cell(pose.x_cm, pose.y_cm),
        );
        if apart <= 1 {
            found.push((
                apart,
                hull,
                FieldPose {
                    site: pose.site,
                    x_cm: ramp.x_cm,
                    y_cm: ramp.y_cm,
                },
            ));
        }
    }
    found.sort_by_key(|(apart, hull, _)| (*apart, *hull));
    found
        .into_iter()
        .map(|(_, hull, ramp)| (hull, ramp))
        .collect()
}

/// **Whether `carrier` is the casualty's own party's or a Friend's** (Q-36-52): the casualty's
/// own leased bay, or the field's own Friend rule (`crate::field::parties::friends`: one owner
/// behind both, or, since doc 36 cut GC.10, an organization they openly share).
fn friendly(ctx: &Ctx<'_>, site: &CanonicalId, casualty: ObjId, carrier: ObjId) -> bool {
    if ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_lease_on(casualty))
        .is_some_and(|lease| lease.hull == carrier)
    {
        return true;
    }
    // Doc 36 cut GC.10 (Q-36-10): the field's own Friend rule — one owner behind both, or an
    // organization their leaders or owners openly share.
    let entrants = crate::field::parties::entrants(ctx, site, &[casualty, carrier]);
    let [a, b] = entrants.as_slice() else {
        return false;
    };
    if a.key == b.key {
        return true;
    }
    let Ok((parties, _)) = crate::field::parties::letter(&entrants) else {
        return false;
    };
    let letter = |key| parties.iter().find(|p| p.key == key).map(|p| p.party);
    let (Some(x), Some(y)) = (letter(a.key), letter(b.key)) else {
        return false;
    };
    crate::field::parties::friends(&entrants, &parties)
        .into_iter()
        .any(|pair| pair == (x, y) || pair == (y, x))
}

/// **Whether `carrier` is `tug`'s own hull or a Friend's** (doc 36a lane 3, P0-5's rider): the
/// tug's own lease hull, else [`friendly`]'s Friend rule between the tug and the carrier. A tug
/// loads into no stranger's bay, whatever the casualty's party.
fn own_or_friends(ctx: &Ctx<'_>, site: &CanonicalId, tug: ObjId, carrier: ObjId) -> bool {
    friendly(ctx, site, tug, carrier)
}

/// **Who holds what a carrier takes into custody** (doc 36 cut GC.5; Q-36-16): its party's owner
/// as the field reads it, else its leader, else whoever holds its helm.
fn custodian_of(ctx: &Ctx<'_>, site: &CanonicalId, carrier: ObjId) -> Option<ObjId> {
    crate::field::parties::entrants(ctx, site, &[carrier])
        .first()
        .and_then(|entrant| entrant.owner.or(entrant.leader))
        .or_else(|| crate::port::coop::helm_holder(ctx.w, carrier))
}

/// **A load's facts** for `tug`'s tow into `carrier`.
fn load_facts(
    ctx: &Ctx<'_>,
    tug: ObjId,
    casualty: ObjId,
    carrier: ObjId,
    ramp: FieldPose,
) -> Option<LoadFacts> {
    let w: &World = ctx.w;
    let (site, _, tug_pose) = on_ground(w, tug)?;
    let (cells, _, _, _, volume) = unit_facts(w, casualty)?;
    let load = bay_load(w, carrier, Some(casualty))?;
    Some(LoadFacts {
        secs: recovery_config(w).load_s,
        friendly: friendly(ctx, &site, casualty, carrier),
        own_carrier: own_or_friends(ctx, &site, tug, carrier),
        custodian: custodian_of(ctx, &site, carrier),
        written_off: crate::port::tv::written_off(w, casualty),
        handed_back: crate::port::tv::in_shed(w, casualty),
        site,
        carrier,
        casualty,
        room: load.free(),
        volume_l: volume,
        ramp,
        tug_pose,
        casualty_unit: cells,
        slot: load.slot.as_str().to_owned(),
        in_bay: act::where_is(w, casualty) == (Where::Bay { hull: carrier }),
    })
}

/// A casualty's name as the lines print it.
fn name(w: &World, unit: ObjId) -> String {
    crate::port::coop::name_of(w, unit)
}

/// The state word a preview prints.
const fn state_word(state: OperationalState) -> &'static str {
    match state {
        OperationalState::Operational => "under its own power",
        OperationalState::Immobile => "immobile",
        OperationalState::Unarmed => "disarmed",
        OperationalState::Disabled => "disabled",
        OperationalState::Wreck => "a wreck",
        OperationalState::Irrecoverable => "beyond recovery",
    }
}

/// Seconds as whole IC minutes, rounded up.
fn minutes(seconds: u32) -> u32 {
    seconds.div_ceil(60)
}

/// cm/s as m/s with one decimal.
fn metres_s(cms: u32) -> String {
    format!("{}.{}", cms / 100, (cms % 100) / 10)
}

/// **The library's refusal in words** (the lines' `{why}`).
fn why(error: &RecoveryError) -> String {
    match error {
        RecoveryError::ExceedsTonnageCap {
            tug_mass_kg,
            casualty_mass_kg,
            max_casualty_mass_kg,
        } => format!(
            "at {} t laden it is more than the {} t a {} t tug can tow",
            casualty_mass_kg.div_ceil(1_000),
            max_casualty_mass_kg / 1_000,
            tug_mass_kg / 1_000
        ),
        RecoveryError::OutOfAttachRange { .. } | RecoveryError::SiteMismatch { .. } => {
            "it is not within a cell".to_owned()
        }
        RecoveryError::TugNotMobile { .. } => "the tug cannot drive".to_owned(),
        RecoveryError::TargetNotTowable { state, .. } => match state {
            OperationalState::Operational => "it moves under its own power".to_owned(),
            _ => "it cannot be towed".to_owned(),
        },
        RecoveryError::CasualtyAlreadyAttached { .. } => "another tug has it already".to_owned(),
        RecoveryError::TugAlreadyActive { .. } | RecoveryError::UnitCurrentlyTowed { .. } => {
            "the tug is in a tow already".to_owned()
        }
        RecoveryError::BaySlotFull => "the bay is full".to_owned(),
        RecoveryError::BayMassExceeded { mass_kg, free_kg } => format!(
            "at {} t it is more than the {} t the bay has left",
            mass_kg.div_ceil(1_000),
            free_kg / 1_000
        ),
        RecoveryError::BayVolumeExceeded { volume_l, free_l } => format!(
            "at {} m3 it is more than the {} m3 the bay has left",
            volume_l.div_ceil(1_000),
            free_l / 1_000
        ),
        other => other.to_string(),
    }
}

/// The record a prepare's refusal word stands for.
fn refused(word: &'static str) -> mush_core::ActionError {
    port_error(ActionErrorCode::ProviderRejected, word)
}

/// The integrity refusal.
fn integrity(message: &'static str) -> mush_core::ActionError {
    port_error(ActionErrorCode::Integrity, message)
}

/// The invalid request.
fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "the tow is self-scoped, takes no targets and names one rig and its step",
    )
}

/// The tow's static refusal word for a library error.
const fn code(error: &RecoveryError) -> &'static str {
    match error {
        RecoveryError::ExceedsTonnageCap { .. } => "tow_tonnage",
        RecoveryError::OutOfAttachRange { .. } | RecoveryError::SiteMismatch { .. } => "tow_reach",
        RecoveryError::TugNotMobile { .. } => "tow_tug_immobile",
        RecoveryError::TargetNotTowable { .. } => "tow_not_towable",
        RecoveryError::CasualtyAlreadyAttached { .. } => "tow_taken",
        RecoveryError::TugAlreadyActive { .. } | RecoveryError::UnitCurrentlyTowed { .. } => {
            "tow_busy"
        }
        RecoveryError::BaySlotFull
        | RecoveryError::BayMassExceeded { .. }
        | RecoveryError::BayVolumeExceeded { .. } => "tow_bay_room",
        _ => "tow_refused",
    }
}

// ---- the action ---------------------------------------------------------------------------

fn mod_error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(mod_error)?,
        DefinitionId::new(id).map_err(mod_error)?,
        1,
        payload,
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(mod_error)
}

/// The outcome's descriptor.
///
/// # Errors
///
/// A descriptor that does not build.
pub fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "outcome",
        TOW_OUTCOME,
        json!({"fields":["actor","tv","op","result"]}),
    )
}

/// The action's descriptor.
///
/// # Errors
///
/// The outcome unregistered, or a descriptor that does not build.
pub fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(TOW_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the tow's outcome did not register"))?;
    descriptor(
        "action",
        TOW_ACTION,
        json!({"subject":"self_actor","targets":0,"family":crate::surface::site_record::SITE_RECORD_FAMILY}),
    )
    .map(|d| d.with_references(vec![outcome]))
}

/// **`tacsim_surface_tow`** (this module's head).
pub(crate) struct TowProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub sites: StateFamilyHandle<SiteKey, SiteRecord>,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    /// **Doc 36 cut GC.5**: the inventory families a pickup moves a crate's goods through; `None`
    /// when the inventory cell is not filled, and then a pickup refuses.
    pub inventory: Option<mush_inventory::InventoryFamilies>,
    pub black_box: Option<
        mush_core::participant::ParticipantHandle<crate::surface::black_box::BlackBoxParticipant>,
    >,
}

/// **The tow's registered action** (module state, the bay's `BayActHandles` precedent: kept off
/// `PortHandles`, so the port's registration frame stays inside the M7 memory-size limits).
#[derive(Clone, Debug, Default)]
pub(crate) struct TowHandles {
    /// `tacsim_surface_tow`, when registered.
    pub action: Option<DefinitionRef>,
}

/// Register the provider, its handles and its RAM slots — out of line, so its temporaries are
/// never on the port registration's frame.
///
/// # Errors
///
/// Any registration the registrar refuses.
#[inline(never)]
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    families: &crate::port::families::PortFamilies,
    (inventory, black_box): (
        Option<&mush_inventory::InventoryFamilies>,
        Option<
            mush_core::participant::ParticipantHandle<
                crate::surface::black_box::BlackBoxParticipant,
            >,
        >,
    ),
) -> Result<(), ModError> {
    reg.state(TowOrders::default());
    reg.state(TowIndex::default());
    reg.state(TowWatch::default());
    reg.state(crate::surface::tender::TendApplied::default());
    reg.state(crate::surface::claim::ClaimMail::default());
    let (Some(action), Some(outcome)) = (actions.get(TOW_ACTION), outcomes.get(TOW_OUTCOME)) else {
        reg.state(TowHandles::default());
        return Ok(());
    };
    reg.action_provider(
        action.clone(),
        TowProvider {
            action: action.clone(),
            outcome: outcome.clone(),
            sites: families.sites.clone(),
            excursions: families.excursions.clone(),
            inventory: inventory.cloned(),
            black_box,
        },
    )?;
    reg.state(TowHandles {
        action: Some(action.clone()),
    });
    Ok(())
}

/// The action's `op` words.
const OPS: [&str; 11] = [
    "attach", "attached", "release", "load", "loaded", "pickup", "claim", "scrap", "tend",
    "tended", "stores",
];

fn parameters(
    invocation: &ActionInvocation,
) -> Result<(&Map<String, Value>, ObjId, &str), mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    let p = invocation.parameters().as_object().ok_or_else(invalid)?;
    let tv = p
        .get("tv")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .map(ObjId)
        .ok_or_else(invalid)?;
    let op = p
        .get("op")
        .and_then(Value::as_str)
        .filter(|op| OPS.contains(op))
        .ok_or_else(invalid)?;
    Ok((p, tv, op))
}

impl ActionProvider for TowProvider {
    fn validate_invocation(
        &self,
        invocation: &ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, mush_core::ActionError> {
        if !act::self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w = &engine.world;
        let Some(orders) = w.module_state::<TowOrders>() else {
            return Ok(None);
        };
        let mut rows = Map::new();
        for (tv, order) in orders.of(query.actor()) {
            if let Ok(value) = serde_json::to_value(order) {
                rows.insert(tv.0.to_string(), value);
            }
        }
        if rows.is_empty() {
            return Ok(None);
        }
        act::offer_draft(
            action,
            engine,
            json!({"orders": rows, "now": w.world_ic_now().at}),
            vec![
                act::parameter(
                    "tv",
                    true,
                    ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: i64::from(u32::MAX),
                    },
                )?,
                act::parameter(
                    "op",
                    true,
                    ProjectedParameterKind::Choice {
                        choices: OPS.map(str::to_owned).to_vec(),
                    },
                )?,
            ],
            (
                "tacsim-surface-tow",
                "Tow",
                "Take a casualty in tow, let it go, load it into a carrier's bay, or file and settle a salvage claim.",
            ),
        )
        .map(Some)
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        let (p, tv, op) = parameters(invocation)?;
        let now = p.get("now").and_then(Value::as_i64).ok_or_else(invalid)?;
        let order: TowOrder = p
            .get("orders")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(&tv.0.to_string()))
            .cloned()
            .and_then(|row| serde_json::from_value(row).ok())
            .ok_or_else(|| {
                port_error(
                    ActionErrorCode::Unauthorized,
                    "that rig has no tow order of yours",
                )
            })?;
        if order.word() != op {
            return Err(invalid());
        }
        let actor = invocation.actor();
        // Doc 41 cut RS.1: the stores ops write the rig's record and its stores, never a site's.
        if let TowOrder::Stores { facts } = &order {
            let result = self.stores(txn, tv, facts)?;
            let outcome = RulesValue::new(
                self.outcome.clone(),
                json!({"actor": actor.0, "tv": tv.0, "op": op, "result": result}),
            )
            .map_err(|_| integrity("the tow result is out of bounds"))?;
            return Ok(AuthoritativeActionOutput::new(outcome)
                .with_eligible_witnesses(vec![invocation.principal().clone()]));
        }
        let site = match &order {
            TowOrder::Tend { facts } => facts.site.clone(),
            TowOrder::Stores { .. } => return Err(invalid()),
            TowOrder::Attach { facts } => facts.site.clone(),
            TowOrder::Load { facts } => facts.site.clone(),
            TowOrder::Pickup { facts } => facts.site.clone(),
            TowOrder::Scrap { facts } => facts.site.clone(),
            TowOrder::Claim { order } => order.site().clone(),
            TowOrder::Attached { site, .. }
            | TowOrder::Release { site }
            | TowOrder::Loaded { site, .. }
            | TowOrder::Tended { site, .. } => site.clone(),
        };
        let key = SiteKey(site.clone());
        let mut record = txn
            .get(&self.sites, &key)
            .map_err(|_| integrity("the site records are unavailable"))?
            .cloned()
            .filter(|record| record.key() == key)
            .ok_or_else(|| refused("tow_no_ground"))?;
        let promoted = record.open_field().is_some();
        let mut closing: Option<(ObjId, crate::surface::black_box::BlackBoxLine)> = None;
        let result: String = match order {
            TowOrder::Attach { facts } => {
                if promoted {
                    return Err(refused(crate::surface::site_record::FIELD_OPEN_CODE));
                }
                if !record.units.iter().any(|left| left.unit == facts.casualty) {
                    return Err(refused("tow_not_lying"));
                }
                if facts.carrier_refusal.is_some() {
                    return Err(refused("tow_carrier_floor"));
                }
                // Doc 36a P1-4: one attach in flight a casualty, so the first tug's pin stands
                // until its attach lands or is let go.
                if record.tows.attaching_elsewhere(tv, facts.casualty) {
                    return Err(refused("tow_taken"));
                }
                let rig = act::read_record(txn, &self.excursions, tv)?;
                if rig
                    .as_ref()
                    .is_some_and(|r| r.act.is_some() || r.run.is_some())
                {
                    return Err(refused("tow_rig_busy"));
                }
                let mut probe = record.tows.book.clone();
                probe
                    .attach(attach_request(tv, &facts, now))
                    .map_err(|e| refused(code(&e)))?;
                let secs = i64::from(facts.secs);
                record
                    .tows
                    .put_act(
                        tv,
                        actor,
                        TowStage::Attach {
                            casualty: facts.casualty,
                        },
                        (now, now.saturating_add(secs)),
                    )
                    .map_err(refused)?;
                // Doc 36 cut GC.6 (Q-S3, Q-36-26): the tenure is pinned when the attach starts.
                if let Some(pin) = facts
                    .pin
                    .clone()
                    .filter(|pin| pin.casualty == facts.casualty)
                {
                    crate::surface::claim::pin(&mut record.tows.pins, pin);
                }
                "filed".to_owned()
            }
            TowOrder::Attached { ordinal, facts, .. } => {
                let act = record
                    .tows
                    .act_of(tv)
                    .filter(|a| a.ordinal == ordinal && a.operator == actor)
                    .cloned()
                    .ok_or_else(|| port_error(ActionErrorCode::Conflict, "that tow has landed"))?;
                let TowStage::Attach { casualty } = act.stage else {
                    return Err(invalid());
                };
                record.tows.take_act(tv);
                if promoted {
                    "interrupted".to_owned()
                } else {
                    match facts.filter(|f| f.casualty == casualty) {
                        None => "refused:it is no longer there".to_owned(),
                        Some(facts) if facts.carrier_refusal.is_some() => {
                            "refused:the tug is too light for a carrier".to_owned()
                        }
                        Some(facts) => {
                            match record.tows.book.attach(attach_request(tv, &facts, now)) {
                                Ok(_) => "attached".to_owned(),
                                Err(error) => format!("refused:{}", why(&error)),
                            }
                        }
                    }
                }
            }
            TowOrder::Release { .. } => {
                // Doc 36a P1-10 (Q-CF-30): a release draining after a hostile mark promoted the
                // site waits like every other surface act; otherwise the retired tow's casualty
                // is neither fielded nor carried, and the demotion drops it.
                if promoted {
                    return Err(refused(crate::surface::site_record::FIELD_OPEN_CODE));
                }
                // Doc 36 cut GC.6 (Q-36-26): a release starts the tenure again.
                let released = record
                    .tows
                    .towing(tv)
                    .map(RecoveryOperation::casualty)
                    .or_else(|| {
                        record.tows.act_of(tv).and_then(|a| match a.stage {
                            TowStage::Attach { casualty } => Some(casualty),
                            TowStage::Load { .. } | TowStage::Tend { .. } => None,
                        })
                    });
                record
                    .tows
                    .pins
                    .retain(|pin| !(Some(pin.casualty) == released && pin.tug == tv));
                let had_act = record.tows.take_act(tv).is_some();
                let tow = record.tows.towing(tv).map(|op| (op.id(), op.revision()));
                match tow {
                    Some((id, revision)) => {
                        let at = record
                            .tows
                            .book
                            .get(id)
                            .map_or(now, |op| op.updated_at_ic().max(now));
                        record
                            .tows
                            .book
                            .detach(id, revision, at)
                            .map_err(|e| refused(code(&e)))?;
                        record.tows.book.retire(id).map_err(|e| refused(code(&e)))?;
                        "released".to_owned()
                    }
                    None if had_act => "interrupted".to_owned(),
                    None => return Err(refused("tow_none")),
                }
            }
            TowOrder::Load { facts } => {
                if promoted {
                    return Err(refused(crate::surface::site_record::FIELD_OPEN_CODE));
                }
                let op = record
                    .tows
                    .towing(tv)
                    .cloned()
                    .ok_or_else(|| refused("tow_none"))?;
                if op.casualty() != facts.casualty {
                    return Err(refused("tow_changed"));
                }
                if record.tows.act_of(tv).is_some() {
                    return Err(refused("tow_busy"));
                }
                // Doc 36a lane 3 (P0-5's rider): never into a stranger's carrier.
                if !facts.own_carrier {
                    return Err(refused("tow_foreign_carrier"));
                }
                // Within a cell of the ramp, as the command and the tick count it.
                if facts.tug_pose.site != facts.ramp.site
                    || cells_apart(
                        site_cell(facts.tug_pose.x_cm, facts.tug_pose.y_cm),
                        site_cell(facts.ramp.x_cm, facts.ramp.y_cm),
                    ) > 1
                {
                    return Err(refused("tow_not_at_ramp"));
                }
                facts
                    .room
                    .check_can_load(op.casualty_mass().total_mass_kg(), facts.volume_l)
                    .map_err(|e| refused(code(&e)))?;
                record
                    .tows
                    .put_act(
                        tv,
                        actor,
                        TowStage::Load {
                            carrier: facts.carrier,
                        },
                        (now, now.saturating_add(i64::from(facts.secs))),
                    )
                    .map_err(refused)?;
                "filed".to_owned()
            }
            TowOrder::Loaded { ordinal, facts, .. } => {
                let act = record
                    .tows
                    .act_of(tv)
                    .filter(|a| a.ordinal == ordinal && a.operator == actor)
                    .cloned()
                    .ok_or_else(|| port_error(ActionErrorCode::Conflict, "that load has landed"))?;
                let TowStage::Load { carrier } = act.stage else {
                    return Err(invalid());
                };
                record.tows.take_act(tv);
                let op = record.tows.towing(tv).cloned();
                match (
                    op,
                    facts.filter(|f| f.carrier == carrier && (f.in_bay || f.handed_back)),
                ) {
                    (Some(op), Some(facts)) if op.casualty() == facts.casualty => {
                        self.complete_load(
                            txn,
                            &mut record,
                            (tv, actor, &op, &facts),
                            now,
                            &mut closing,
                        )?;
                        if facts.written_off {
                            "handed_back".to_owned()
                        } else if facts.friendly {
                            "loaded".to_owned()
                        } else {
                            "custody".to_owned()
                        }
                    }
                    _ if promoted => "interrupted".to_owned(),
                    _ => "refused:it never reached the bay".to_owned(),
                }
            }
            TowOrder::Pickup { facts } => {
                self.pick_up(txn, &mut record, (tv, actor), &facts, (promoted, now))?
            }
            TowOrder::Claim { order } => {
                if promoted {
                    return Err(refused(crate::surface::site_record::FIELD_OPEN_CODE));
                }
                if order.hull() != tv {
                    return Err(invalid());
                }
                crate::surface::claim::prepare(&mut record, actor, &order, now).map_err(refused)?
            }
            TowOrder::Scrap { facts } => {
                self.scrap(txn, &mut record, (tv, actor), &facts, (promoted, now))?
            }
            TowOrder::Tend { facts } => {
                self.tend(txn, &mut record, (tv, actor), &facts, (promoted, now))?
            }
            TowOrder::Tended { ordinal, step, .. } => {
                self.tended(txn, &mut record, (tv, actor, ordinal), &step, now)?
            }
            TowOrder::Stores { .. } => return Err(invalid()),
        };
        record
            .check()
            .map_err(|_| integrity("the site record is out of bounds"))?;
        txn.replace(&self.sites, &key, record)
            .map_err(|_| integrity("the site record could not be written"))?;
        if let Some((hull, line)) = closing {
            crate::surface::black_box::append(context, self.black_box.as_ref(), hull, line)?;
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"actor": actor.0, "tv": tv.0, "op": op, "result": result}),
        )
        .map_err(|_| integrity("the tow result is out of bounds"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// The recovery block (its stage timings), as the command and the tick read it.
fn recovery_config(w: &World) -> crate::config::TacsimRecoveryConfig {
    w.module_state::<TacsimStore>()
        .map(|store| store.config().recovery.clone())
        .unwrap_or_default()
}

impl TowProvider {
    /// **The load's completion** (this module's head): the library's load into the bay, the tow
    /// retired, the casualty off the site's index, and its epoch closed `RecoveredToBase`.
    fn complete_load(
        &self,
        txn: &mut ActionTxn<'_>,
        record: &mut SiteRecord,
        (tv, operator, op, facts): (ObjId, ObjId, &RecoveryOperation, &LoadFacts),
        now: i64,
        closing: &mut Option<(ObjId, crate::surface::black_box::BlackBoxLine)>,
    ) -> Result<(), mush_core::ActionError> {
        let at = now.max(op.updated_at_ic());
        let mut revision = op.revision();
        // The tow's last leg, to where the tug stands at the ramp (the library's speed check
        // over the IC time since its last stage); a leg the check refuses is logged and the
        // load still lands, since the casualty already stands in the bay.
        if op.stage().active_pose() != Some(facts.tug_pose) {
            let dt =
                u32::try_from(at.saturating_sub(op.updated_at_ic()).max(1)).unwrap_or(u32::MAX);
            match record
                .tows
                .book
                .advance_tow(op.id(), revision, facts.tug_pose, dt, at)
            {
                Ok(advanced) => revision = advanced.revision(),
                Err(error) => {
                    tracing::warn!(tug = tv.0, %error, "tacsim: a tow's last leg did not check; its load lands where the casualty stands");
                }
            }
        }
        let loaded = record.tows.book.load_into_bay(LoadIntoBayRequest {
            id: op.id(),
            expected_revision: revision,
            carrier: facts.carrier,
            slot: smol_str::SmolStr::new(&facts.slot),
            carrier_pose: facts.ramp,
            max_load_distance_cm: u64::MAX,
            bay_room: facts.room,
            casualty_stowage_volume_l: facts.volume_l,
            casualty_unit: &facts.casualty_unit,
            at_ic: at,
        });
        match loaded {
            Ok(_) => {
                let _ = record.tows.book.retire(op.id());
            }
            Err(error) => {
                tracing::warn!(tug = tv.0, casualty = facts.casualty.0, %error, "tacsim: a load the library refused stood in the bay already; its tow is released there");
                record.tows.end(facts.casualty, at);
            }
        }
        let left = record.take_unit(facts.casualty).ok();
        // Doc 36 cut GC.6 (Q-S3, Q-36-26): the tenure this tug pinned rides into the custody.
        let pin = record
            .tows
            .pins
            .iter()
            .position(|pin| pin.casualty == facts.casualty)
            .map(|at| record.tows.pins.remove(at))
            .filter(|pin| pin.tug == tv);
        // Doc 36 cut GC.5 (Q-36-16, Q-36-29): a load into a carrier that is not the casualty's own
        // party's or a Friend's puts it in that carrier's operator's custody with a claim owed;
        // a written-off lease rig is Occator's alone and is handed back with none.
        if facts.written_off || facts.friendly {
            record.custody.retain(|row| row.hull != facts.casualty);
        } else {
            let holder = facts.custodian.unwrap_or(operator);
            // Doc 36 cut GC.6 (Q-36-30): a recovery off owned ground with neither the title
            // holder's nor the landholder's authority is written to the evidence log; doc 41
            // Q-41-21: either authorization as the attach read it counts.
            if let Some(landholder) = pin
                .as_ref()
                .and_then(crate::surface::claim::TenurePin::landholder)
                && landholder != holder
                && pin.as_ref().and_then(|pin| pin.answering) != Some(holder)
                && !pin
                    .as_ref()
                    .is_some_and(|pin| pin.owner_grant.is_some() || pin.landholder_authorized)
            {
                crate::surface::claim::log_unlawful(
                    record,
                    crate::surface::claim::UnlawfulAct {
                        kind: crate::surface::claim::UnlawfulKind::Recovery,
                        actor: holder,
                        subject: facts.casualty.0,
                        landholder: Some(landholder),
                        at_ic: at,
                    },
                );
            }
            crate::surface::salvage::hold(
                record,
                crate::surface::salvage::SalvageCustody {
                    hull: facts.casualty,
                    holder,
                    carrier: facts.carrier,
                    recovered_by: tv,
                    recovered_ic: at,
                    left_ic: left.map(|row| row.left_ic),
                    claim: crate::surface::salvage::ClaimSeam::Owed,
                    operation: Some(op.id()),
                    pin,
                    file: None,
                    told: Vec::new(),
                    evidence: Vec::new(),
                    at_risk: Vec::new(),
                },
            );
        }
        // The casualty's own rig record: its epoch closed into the carrier, its surface work
        // cleared, its charge counted from the bay.
        if let Some(mut rig) = act::read_record(txn, &self.excursions, facts.casualty)? {
            if let Some(open) = rig.sortie.take() {
                *closing = Some((
                    facts.casualty,
                    crate::surface::black_box::BlackBoxLine::sortie_closed(
                        facts.casualty,
                        &open,
                        at,
                        IcCloseKind::RecoveredToBase {
                            base: facts.carrier,
                        },
                    )
                    .recovered_by(tv),
                ));
            }
            rig.act = None;
            rig.run = None;
            rig.queued = None;
            rig.plan = None;
            rig.excursion = None;
            rig.charge = Charge {
                value_cu: rig.charge.value_cu,
                at_ic: at,
                in_bay: true,
            };
            act::write_record(txn, &self.excursions, rig, true)?;
        }
        Ok(())
    }
}

impl TowProvider {
    /// **A crate picked up** (doc 36 cut GC.5; Q-36-19, FO-4): off the site's record, and its
    /// manifest from the site's spoil into the rig's hopper by the inventory's own transfer, in
    /// this bundle. No goods are made: a transfer the spoil cannot cover refuses the whole act.
    fn pick_up(
        &self,
        txn: &mut ActionTxn<'_>,
        record: &mut SiteRecord,
        (tv, actor): (ObjId, ObjId),
        facts: &crate::surface::salvage::PickupFacts,
        (promoted, now): (bool, i64),
    ) -> Result<String, mush_core::ActionError> {
        if promoted {
            return Err(refused(crate::surface::site_record::FIELD_OPEN_CODE));
        }
        if record.tows.act_of(tv).is_some() || record.tows.towing(tv).is_some() {
            return Err(refused("tow_busy"));
        }
        if act::read_record(txn, &self.excursions, tv)?
            .as_ref()
            .is_some_and(|r| r.act.is_some() || r.run.is_some())
        {
            return Err(refused("tow_rig_busy"));
        }
        let inventory = self
            .inventory
            .as_ref()
            .ok_or_else(|| refused("pickup_unwired"))?;
        let object = record
            .take_object(facts.object)
            .map_err(|_| refused("pickup_gone"))?;
        let crate::surface::site_record::GroundKind::Cargo { manifest, .. } = &object.kind else {
            return Err(refused("pickup_gone"));
        };
        if *manifest != facts.manifest {
            return Err(refused("pickup_changed"));
        }
        let moves: Vec<(CanonicalId, u64)> = manifest
            .iter()
            .filter(|(_, qty)| **qty > 0)
            .map(|(good, qty)| (good.clone(), *qty))
            .collect();
        let hopper = mush_inventory::EndpointRef(mush_economy::VaultId(facts.hopper));
        // Doc 36a P2-2: the hopper's room, re-read here against the facts' limits; what landed
        // in it since the command (a run's ore, another pickup) counts.
        let mut load: BTreeMap<CanonicalId, u64> = facts
            .limits
            .keys()
            .map(|measure| (measure.clone(), 0))
            .collect();
        for (good, per) in &facts.figures {
            let qty = inventory
                .available(txn, hopper, good)
                .map_err(|_| integrity("the hopper cannot be read"))?;
            for (measure, per_unit) in per {
                let slot = load.entry(measure.clone()).or_insert(0);
                *slot = slot.saturating_add(qty.saturating_mul(*per_unit));
            }
        }
        for (good, qty) in &moves {
            let Some(per) = facts.figures.get(good) else {
                if facts.figures.is_empty() {
                    break;
                }
                return Err(refused("pickup_no_stow"));
            };
            for (measure, per_unit) in per {
                let slot = load.entry(measure.clone()).or_insert(0);
                *slot = slot.saturating_add(qty.saturating_mul(*per_unit));
            }
        }
        if facts
            .limits
            .iter()
            .any(|(measure, limit)| load.get(measure).copied().unwrap_or(0) > *limit)
        {
            return Err(refused("pickup_no_room"));
        }
        inventory
            .transfer_many(
                txn,
                mush_inventory::EndpointRef(mush_economy::VaultId(facts.spoil)),
                hopper,
                &moves,
            )
            .map_err(|error| {
                tracing::warn!(tv = tv.0, object = facts.object, %error, "tacsim: a crate's goods would not move from the spoil; the pickup refuses");
                refused("pickup_empty")
            })?;
        if facts.lawful {
            return Ok("picked:lawful".to_owned());
        }
        // Doc 36 cut GC.6 (Q-36-30): evidence for a later crime system, never an enforcement.
        crate::surface::claim::log_unlawful(
            record,
            crate::surface::claim::UnlawfulAct {
                kind: crate::surface::claim::UnlawfulKind::Pickup,
                actor,
                subject: facts.object,
                landholder: facts.landholder,
                at_ic: now,
            },
        );
        Ok("picked:unlawful".to_owned())
    }

    /// **Remains broken up where they lie** (doc 36 cut GC.6, GC.3's follow-up; Q-36-39): off the
    /// site's record, and their structural mass minted as one `structural_alloys` lot into the
    /// carrier's hold, in this bundle. Remains that changed since the command read them refuse.
    fn scrap(
        &self,
        txn: &mut ActionTxn<'_>,
        record: &mut SiteRecord,
        (tv, actor): (ObjId, ObjId),
        facts: &crate::surface::claim::ScrapFacts,
        (promoted, now): (bool, i64),
    ) -> Result<String, mush_core::ActionError> {
        if promoted {
            return Err(refused(crate::surface::site_record::FIELD_OPEN_CODE));
        }
        if record.tows.act_of(tv).is_some() || record.tows.towing(tv).is_some() {
            return Err(refused("tow_busy"));
        }
        if act::read_record(txn, &self.excursions, tv)?
            .as_ref()
            .is_some_and(|r| r.act.is_some() || r.run.is_some())
        {
            return Err(refused("tow_rig_busy"));
        }
        let inventory = self
            .inventory
            .as_ref()
            .ok_or_else(|| refused("scrap_unwired"))?;
        let object = record
            .take_object(facts.object)
            .map_err(|_| refused("scrap_gone"))?;
        let crate::surface::site_record::GroundKind::Remains { of, mass_kg, .. } = &object.kind
        else {
            return Err(refused("scrap_gone"));
        };
        if *of != facts.of || *mass_kg != facts.mass_kg {
            return Err(refused("scrap_changed"));
        }
        // Doc 39 cut IN.1 (§4.10): scrap mints down to whole tonnes; under a tonne, on a site
        // with no facility to keep the fraction, nothing mints and the mass is a declared loss.
        let alloys_t = mass_kg / crate::port::bay_work::strip::SCRAP_KG_PER_T;
        if alloys_t != facts.alloys_t {
            return Err(refused("scrap_changed"));
        }
        let good = CanonicalId::new(crate::port::bay_work::strip::SCRAP_GOOD)
            .map_err(|_| integrity("the alloys' good is malformed"))?;
        let grade = mush_inventory::Grade::new(crate::port::bay_work::strip::SCRAP_GRADE)
            .map_err(|_| integrity("the alloys' grade is out of bounds"))?;
        if alloys_t > 0 {
            inventory
            .mint_origin(
                txn,
                mush_inventory::EndpointRef(mush_economy::VaultId(facts.hold)),
                &good,
                alloys_t,
                grade,
                mush_economy::ExternalRef {
                    provider: "tacsim".into(),
                    kind: "remains_scrap".into(),
                    id: format!("{}/{}", facts.site, facts.object).into(),
                    revision: None,
                },
                mush_inventory::LotId(facts.lot),
            )
            .map_err(|error| {
                tracing::warn!(tv = tv.0, object = facts.object, %error, "tacsim: remains' alloys would not mint; the scrap refuses");
                refused("scrap_unminted")
            })?;
        }
        if facts.lawful {
            return Ok("scrapped:lawful".to_owned());
        }
        crate::surface::claim::log_unlawful(
            record,
            crate::surface::claim::UnlawfulAct {
                kind: crate::surface::claim::UnlawfulKind::Scrap,
                actor,
                subject: facts.object,
                landholder: facts.landholder,
                at_ic: now,
            },
        );
        Ok("scrapped:unlawful".to_owned())
    }

    /// **A tend act's start** (doc 41 cut RS.1; [`crate::surface::tender`]): on open ground, the
    /// rig idle (no tow act, no tow, no surface act or run), the target on the same ground, not
    /// carried in tow, within a cell; one timed act on the site's record.
    fn tend(
        &self,
        txn: &mut ActionTxn<'_>,
        record: &mut SiteRecord,
        (tv, actor): (ObjId, ObjId),
        facts: &crate::surface::tender::TendFacts,
        (promoted, now): (bool, i64),
    ) -> Result<String, mush_core::ActionError> {
        if promoted {
            return Err(refused(crate::surface::site_record::FIELD_OPEN_CODE));
        }
        if record.tows.towing(tv).is_some() || record.tows.towed(tv).is_some() {
            return Err(refused("tow_busy"));
        }
        if record.tows.towed(facts.target).is_some() {
            return Err(refused("tend_in_tow"));
        }
        if facts.target == tv
            || facts.tender_pose.site != facts.target_pose.site
            || cells_apart(
                site_cell(facts.tender_pose.x_cm, facts.tender_pose.y_cm),
                site_cell(facts.target_pose.x_cm, facts.target_pose.y_cm),
            ) > 1
        {
            return Err(refused("tend_reach"));
        }
        if act::read_record(txn, &self.excursions, tv)?
            .as_ref()
            .is_some_and(|r| r.act.is_some() || r.run.is_some())
        {
            return Err(refused("tow_rig_busy"));
        }
        record
            .tows
            .put_act(
                tv,
                actor,
                TowStage::Tend {
                    target: facts.target,
                    work: facts.work.clone(),
                    paused: None,
                },
                (now, now.saturating_add(i64::from(facts.secs))),
            )
            .map_err(refused)?;
        Ok("filed".to_owned())
    }

    /// **The tick's step on a tend act** (doc 41 cut RS.1): a pause under a field, a resume on
    /// open ground, a finish that draws the stores and writes the bale, or a finish that came
    /// to nothing.
    fn tended(
        &self,
        txn: &mut ActionTxn<'_>,
        record: &mut SiteRecord,
        (tv, actor, ordinal): (ObjId, ObjId, u32),
        step: &crate::surface::tender::TendStep,
        now: i64,
    ) -> Result<String, mush_core::ActionError> {
        use crate::surface::tender::TendStep;
        let promoted = record.open_field().is_some();
        let act = record
            .tows
            .acts
            .iter_mut()
            .find(|a| a.tug == tv && a.ordinal == ordinal && a.operator == actor)
            .ok_or_else(|| port_error(ActionErrorCode::Conflict, "that tend act has landed"))?;
        let TowStage::Tend { paused, .. } = &mut act.stage else {
            return Err(invalid());
        };
        match step {
            TendStep::Pause => {
                if !promoted || paused.is_some() {
                    return Err(port_error(ActionErrorCode::Conflict, "no field to pause for"));
                }
                *paused = Some(act.due_ic.saturating_sub(now).max(1));
                return Ok("paused".to_owned());
            }
            TendStep::Resume => {
                let Some(left) = *paused else {
                    return Err(port_error(ActionErrorCode::Conflict, "the act is not paused"));
                };
                if promoted {
                    return Err(refused(crate::surface::site_record::FIELD_OPEN_CODE));
                }
                *paused = None;
                act.started_ic = now;
                act.due_ic = now.saturating_add(left);
                return Ok("resumed".to_owned());
            }
            TendStep::Lost { .. } | TendStep::Finish { .. } => {}
        }
        if promoted || paused.is_some() || act.due_ic > now {
            return Err(port_error(ActionErrorCode::Conflict, "the tend act is not due"));
        }
        record.tows.take_act(tv);
        let TendStep::Finish { draw } = step else {
            return Ok("lost".to_owned());
        };
        // The draw (Q-41-2): what the stores still hold of it, out of them; the bale written.
        // The work already landed on the unit (the load's precedent), so a short store never
        // undoes it: the finish takes what is there and says so.
        let mut short = false;
        if let Some(inventory) = self.inventory.as_ref() {
            let stores = mush_inventory::EndpointRef(mush_economy::VaultId(draw.stores));
            for (good, qty) in &draw.debit {
                let have = inventory
                    .available(txn, stores, good)
                    .map_err(|_| integrity("the stores cannot be read"))?;
                let take = have.min(*qty);
                short |= take < *qty;
                if take > 0 {
                    inventory
                        .debit(txn, stores, good, take)
                        .map_err(|_| integrity("the stores could not be drawn"))?;
                }
            }
        } else if !draw.debit.is_empty() {
            short = true;
        }
        if let Some(after) = draw.bale_after {
            let found = act::read_record(txn, &self.excursions, tv)?;
            let existed = found.is_some();
            let mut rig =
                found.unwrap_or_else(|| crate::surface::excursion::RigRecord::fresh_unread(tv));
            rig.bale = Some(after);
            act::write_record(txn, &self.excursions, rig, existed)?;
        }
        Ok(if short {
            "tended:short".to_owned()
        } else {
            "tended".to_owned()
        })
    }

    /// **A stores op** (doc 41 cut RS.1): a load from the carrier's hold into the stores,
    /// re-checked against the stores' room; or the bale opened, a tonne of each line out of the
    /// stores onto the rig's record.
    fn stores(
        &self,
        txn: &mut ActionTxn<'_>,
        tv: ObjId,
        facts: &crate::surface::tender::StoresFacts,
    ) -> Result<String, mush_core::ActionError> {
        use crate::surface::tender::StoresOp;
        let inventory = self
            .inventory
            .as_ref()
            .ok_or_else(|| refused("stores_unwired"))?;
        let stores = mush_inventory::EndpointRef(mush_economy::VaultId(facts.stores));
        let mut rig = act::read_record(txn, &self.excursions, tv)?;
        if rig
            .as_ref()
            .is_some_and(|r| r.act.is_some() || r.run.is_some() || r.excursion.is_some())
        {
            return Err(refused("tow_rig_busy"));
        }
        match &facts.op {
            StoresOp::Load {
                hold,
                good,
                qty,
                limits,
                figures,
            } => {
                let mut load: BTreeMap<CanonicalId, u64> = BTreeMap::new();
                for (held, per) in figures {
                    let mut q = inventory
                        .available(txn, stores, held)
                        .map_err(|_| integrity("the stores cannot be read"))?;
                    if held == good {
                        q = q.saturating_add(*qty);
                    }
                    for (measure, per_unit) in per {
                        *load.entry(measure.clone()).or_default() += q.saturating_mul(*per_unit);
                    }
                }
                if !figures.contains_key(good)
                    || limits
                        .iter()
                        .any(|(m, limit)| load.get(m).copied().unwrap_or(0) > *limit)
                {
                    return Err(refused("stores_no_room"));
                }
                inventory
                    .transfer_many(
                        txn,
                        mush_inventory::EndpointRef(mush_economy::VaultId(*hold)),
                        stores,
                        &[(good.clone(), *qty)],
                    )
                    .map_err(|_| refused("stores_hold_short"))?;
                Ok("loaded".to_owned())
            }
            StoresOp::Open => {
                if rig.as_ref().is_some_and(|r| r.bale.is_some()) {
                    return Err(refused("stores_bale_open"));
                }
                for good in [
                    crate::surface::tender::ALLOYS,
                    crate::surface::tender::FITTINGS,
                ] {
                    let good = CanonicalId::new(good).map_err(|_| invalid())?;
                    inventory
                        .debit(txn, stores, &good, 1)
                        .map_err(|_| refused("stores_no_tonne"))?;
                }
                let existed = rig.is_some();
                let mut record =
                    rig.take()
                        .unwrap_or_else(|| crate::surface::excursion::RigRecord::fresh_unread(tv));
                record.bale = Some(crate::port::bay_work::Bale {
                    alloys_kg: crate::port::bay_work::KG_PER_T,
                    fittings_kg: crate::port::bay_work::KG_PER_T,
                });
                act::write_record(txn, &self.excursions, record, existed)?;
                Ok("opened".to_owned())
            }
        }
    }
}

// ---- submission ---------------------------------------------------------------------------

/// Write `order` for `tug` as `actor`'s, and submit it as `actor`.
pub(crate) fn submit(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    tug: ObjId,
    order: TowOrder,
    tag: u32,
) -> bool {
    let Some(action) = ctx
        .w
        .module_state::<TowHandles>()
        .and_then(|handles| handles.action.clone())
    else {
        return false;
    };
    let op = order.word();
    let now = ctx.w.world_ic_now().at;
    let Ok(key) =
        IdempotencyKeyText::parse_canonical(format!("tacsim.tow/{}/{op}/{tag}/{now}", tug.0))
    else {
        return false;
    };
    if let Some(orders) = ctx.w.module_state_mut::<TowOrders>() {
        orders.put(actor, tug, order);
    }
    let saved = ctx.actor;
    ctx.actor = actor;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({"tv": tug.0, "op": op}),
        key,
    );
    ctx.actor = saved;
    true
}

// ---- the commands -------------------------------------------------------------------------

/// The rig's ground facts or the line that refuses.
fn ground_of(w: &World, rig: &act::RigAt) -> Result<(CanonicalId, TugFacts), CmdError> {
    let n = rig.slot.to_string();
    let (site, facts) =
        tug_facts(w, rig.tv).ok_or_else(|| CmdError::msg(NOT_ON_GROUND.replace("{n}", &n)))?;
    if !facts.state.is_operational() || !facts.can_move || facts.base_speed_cms == 0 {
        return Err(CmdError::msg(TUG_NOT_MOBILE.replace("{n}", &n)));
    }
    Ok((site, facts))
}

/// The site's record, or a fresh one (the salvage's reader).
pub(crate) fn record_or_new(ctx: &Ctx<'_>, site: &CanonicalId) -> SiteRecord {
    record_or_busy(ctx, site)
}

/// A record that will not read refuses as busy.
fn record_or_busy(ctx: &Ctx<'_>, site: &CanonicalId) -> SiteRecord {
    record_of(ctx, site).unwrap_or_else(|| SiteRecord::new(site.clone()))
}

/// **`+tv/<n> tow [#unit] [go]`** (this module's head).
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn tow_cmd(
    ctx: &mut Ctx<'_>,
    rig: &act::RigAt,
    named: Option<&str>,
    go: bool,
) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let (site, tug) = ground_of(ctx.w, rig)?;
    let record = record_or_busy(ctx, &site);
    if record.tows.act_of(rig.tv).is_some() {
        return Err(CmdError::msg(TOW_BUSY.replace("{n}", &n)));
    }
    if record.tows.towing(rig.tv).is_some() {
        return Err(CmdError::msg(IN_TOW.replace("{n}", &n)));
    }
    // The casualties on the index within reach, nearest first; a named one must be among them.
    let mut near: Vec<(u64, ObjId)> = record
        .units
        .iter()
        .filter(|left| left.unit != rig.tv)
        .filter_map(|left| {
            let (there, _, pose) = on_ground(ctx.w, left.unit)?;
            let d = (there == site)
                .then(|| tug.pose.distance_cm_to(&pose))
                .flatten()?;
            (d <= TOW_REACH_CM).then_some((d, left.unit))
        })
        .collect();
    near.sort_unstable();
    let casualty = match named {
        Some(word) => {
            let wanted = word.trim_start_matches('#');
            let found = near.iter().map(|(_, unit)| *unit).find(|unit| {
                unit.0.to_string() == wanted || name(ctx.w, *unit).eq_ignore_ascii_case(word)
            });
            found.ok_or_else(|| {
                CmdError::msg(NOT_IN_REACH.replace("{n}", &n).replace("{name}", word))
            })?
        }
        None => near
            .first()
            .map(|(_, unit)| *unit)
            .ok_or_else(|| CmdError::msg(NOTHING_IN_REACH.replace("{n}", &n)))?,
    };
    let mut facts = attach_facts(ctx, rig.tv, casualty)
        .ok_or_else(|| CmdError::msg(NOTHING_IN_REACH.replace("{n}", &n)))?;
    // Doc 36 cut GC.6 (Q-S3): the tenure, title and witnesses this attach's start pins.
    facts.pin = Some(crate::surface::claim::pin_for(
        ctx,
        &record,
        &site,
        (rig.tv, casualty),
        site_cell(facts.pose.x_cm, facts.pose.y_cm),
    ));
    let label = name(ctx.w, casualty);
    if let Some(line) = &facts.carrier_refusal {
        return Err(CmdError::msg(line.clone()));
    }
    // Doc 36a P1-4 (Q-36-26): a second tug's attach start while another's is in flight on the
    // casualty would replace the first's tenure pin; it waits instead.
    if record.tows.attaching_elsewhere(rig.tv, casualty) {
        return Err(CmdError::msg(
            ATTACHING_ELSEWHERE
                .replace("{n}", &n)
                .replace("{name}", &label),
        ));
    }
    let now = ctx.w.world_ic_now().at;
    let mut probe = record.tows.book.clone();
    let speed = match probe.attach(attach_request(rig.tv, &facts, now)) {
        Ok(op) => op.coupled_speed_cms(),
        Err(error) => {
            return Err(CmdError::msg(
                TOW_REFUSED
                    .replace("{n}", &n)
                    .replace("{name}", &label)
                    .replace("{why}", &why(&error)),
            ));
        }
    };
    let cfg = recovery_config(ctx.w);
    let min = minutes(cfg.inspect_s.saturating_add(cfg.attach_s)).to_string();
    if !go {
        let line = TOW_PREVIEW
            .replace("{n}", &n)
            .replace("{name}", &label)
            .replace("{t}", &(facts.mass.total_mass_kg() / 1_000).to_string())
            .replace("{state}", state_word(facts.assessment.state))
            .replace("{speed}", &metres_s(speed))
            .replace("{min}", &min);
        ctx.out.sys_to_actor(ctx.actor, "tacsim.tow", line);
        return Ok(());
    }
    let actor = ctx.actor;
    if !submit(
        ctx,
        actor,
        rig.tv,
        TowOrder::Attach {
            facts: Box::new(facts),
        },
        0,
    ) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(
        actor,
        "tacsim.tow",
        TOW_FILED
            .replace("{n}", &n)
            .replace("{name}", &label)
            .replace("{min}", &min),
    );
    Ok(())
}

/// **`+tv/<n> release [go]`** (this module's head).
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn release_cmd(ctx: &mut Ctx<'_>, rig: &act::RigAt, go: bool) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let (site, _, _) =
        on_ground(ctx.w, rig.tv).ok_or_else(|| CmdError::msg(NOT_ON_GROUND.replace("{n}", &n)))?;
    let record = record_or_busy(ctx, &site);
    let casualty = record
        .tows
        .towing(rig.tv)
        .map(RecoveryOperation::casualty)
        .or_else(|| {
            record.tows.act_of(rig.tv).map(|a| match &a.stage {
                TowStage::Attach { casualty } => *casualty,
                TowStage::Load { .. } => rig.tv,
                TowStage::Tend { target, .. } => *target,
            })
        })
        .ok_or_else(|| CmdError::msg(NOT_TOWING.replace("{n}", &n)))?;
    let line = RELEASED
        .replace("{n}", &n)
        .replace("{name}", &name(ctx.w, casualty));
    if !go {
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.tow",
            format!("{line} +tv/{n} release go"),
        );
        return Ok(());
    }
    let actor = ctx.actor;
    if !submit(ctx, actor, rig.tv, TowOrder::Release { site }, 0) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(actor, "tacsim.tow", line);
    Ok(())
}

/// **`+tv/<n> load [go]`** (this module's head).
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn load_cmd(ctx: &mut Ctx<'_>, rig: &act::RigAt, go: bool) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let (site, _, pose) =
        on_ground(ctx.w, rig.tv).ok_or_else(|| CmdError::msg(NOT_ON_GROUND.replace("{n}", &n)))?;
    let record = record_or_busy(ctx, &site);
    if record.tows.act_of(rig.tv).is_some() {
        return Err(CmdError::msg(TOW_BUSY.replace("{n}", &n)));
    }
    let op = record
        .tows
        .towing(rig.tv)
        .cloned()
        .ok_or_else(|| CmdError::msg(NOT_TOWING.replace("{n}", &n)))?;
    let casualty = op.casualty();
    let label = name(ctx.w, casualty);
    // The carrier: the rig's own hull when its ramp is in reach, else the nearest in reach
    // that is a Friend's (doc 36a lane 3, P0-5's rider: never a stranger's).
    let carriers = carriers_at(ctx.w, &site, &pose);
    let nearest = carriers
        .first()
        .cloned()
        .ok_or_else(|| CmdError::msg(NOT_AT_RAMP.replace("{n}", &n)))?;
    let (carrier, ramp) = carriers
        .iter()
        .find(|(hull, _)| *hull == rig.hull)
        .or_else(|| {
            carriers
                .iter()
                .find(|(hull, _)| own_or_friends(ctx, &site, rig.tv, *hull))
        })
        .cloned()
        .ok_or_else(|| {
            CmdError::msg(
                LOAD_NOT_YOUR_CARRIER
                    .replace("{n}", &n)
                    .replace("{carrier}", &name(ctx.w, nearest.0)),
            )
        })?;
    let carrier_name = name(ctx.w, carrier);
    let facts = load_facts(ctx, rig.tv, casualty, carrier, ramp)
        .ok_or_else(|| CmdError::msg(NOT_AT_RAMP.replace("{n}", &n)))?;
    if let Err(error) = facts
        .room
        .check_can_load(op.casualty_mass().total_mass_kg(), facts.volume_l)
    {
        return Err(CmdError::msg(
            BAY_NO_ROOM
                .replace("{carrier}", &carrier_name)
                .replace("{name}", &label)
                .replace("{why}", &why(&error)),
        ));
    }
    let cfg = recovery_config(ctx.w);
    let min = minutes(cfg.load_s).to_string();
    let fill = |line: &str| {
        line.replace("{n}", &n)
            .replace("{name}", &label)
            .replace("{carrier}", &carrier_name)
            .replace("{min}", &min)
    };
    if !go {
        let preview = if facts.written_off {
            LOAD_PREVIEW_HAND_BACK
        } else if facts.friendly {
            LOAD_PREVIEW
        } else {
            LOAD_PREVIEW_FOREIGN
        };
        ctx.out.sys_to_actor(ctx.actor, "tacsim.tow", fill(preview));
        return Ok(());
    }
    let actor = ctx.actor;
    if !submit(
        ctx,
        actor,
        rig.tv,
        TowOrder::Load {
            facts: Box::new(facts),
        },
        0,
    ) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(actor, "tacsim.tow", fill(LOAD_FILED));
    Ok(())
}

/// **The gate every other `+tv` act meets** (this module's head): a tug in tow or busy with a
/// tow act may only drive, release or load; a casualty in tow does nothing.
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn act_gate(ctx: &Ctx<'_>, rig: &act::RigAt, drive: bool) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let Some((site, _, _)) = on_ground(ctx.w, rig.tv) else {
        return Ok(());
    };
    let Some(record) = record_of(ctx, &site) else {
        return Ok(());
    };
    if record.tows.act_of(rig.tv).is_some() {
        return Err(CmdError::msg(TOW_BUSY.replace("{n}", &n)));
    }
    if record.tows.towed(rig.tv).is_some() {
        return Err(CmdError::msg(IN_TOW_CASUALTY.replace("{n}", &n)));
    }
    if record.tows.towing(rig.tv).is_some() && !drive {
        return Err(CmdError::msg(IN_TOW.replace("{n}", &n)));
    }
    Ok(())
}

/// A casualty in tow: `{n}`.
pub const IN_TOW_CASUALTY: &str =
    "Rig {n} is in tow: it goes where its tug takes it until it is loaded or let go.";

/// **Whether `tv` may start a surface act of `kind` in its own transaction** (the start's
/// prepare): a tug with a tow act in flight starts nothing; a tug in tow only drives; a casualty
/// in tow starts nothing.
pub(crate) fn start_refusal(
    record: &SiteRecord,
    tv: ObjId,
    kind: crate::surface::excursion::ActKind,
) -> Option<&'static str> {
    if record.tows.act_of(tv).is_some() {
        return Some("tow_busy");
    }
    if record.tows.towed(tv).is_some() {
        return Some("tow_in_tow");
    }
    if record.tows.towing(tv).is_some() && kind != crate::surface::excursion::ActKind::Drive {
        return Some("tow_in_tow");
    }
    None
}

// ---- the tick -----------------------------------------------------------------------------

/// **The tow's tick** (on the surface tick): refresh the RAM mirror, tell the finishes that
/// landed, and submit every act due.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    if ctx.w.module_state::<TowIndex>().is_none() {
        return;
    }
    let now = ctx.w.world_ic_now().at;
    let sites: Vec<CanonicalId> = crate::surface::chart::lane_sites(ctx.w)
        .into_iter()
        .map(|site| site.entry)
        .collect();
    let mut index: BTreeMap<ObjId, (CanonicalId, RecoveryOperation)> = BTreeMap::new();
    let mut due: Vec<(CanonicalId, bool, TowAct)> = Vec::new();
    let mut steps: Vec<(CanonicalId, TowAct, crate::surface::tender::TendStep)> = Vec::new();
    let mut acts_now: BTreeMap<ObjId, u32> = BTreeMap::new();
    for site in &sites {
        let Some(record) = record_of(ctx, site) else {
            continue;
        };
        for op in record.tows.active() {
            index.insert(op.tug(), (site.clone(), op.clone()));
        }
        let promoted = record.open_field().is_some();
        for act in &record.tows.acts {
            acts_now.insert(act.tug, act.ordinal);
            // Doc 41 cut RS.1 (Q-41-3): a tend act pauses under a field and resumes after it.
            if let TowStage::Tend { paused, .. } = &act.stage {
                use crate::surface::tender::TendStep;
                match (promoted, paused) {
                    (true, None) => steps.push((site.clone(), act.clone(), TendStep::Pause)),
                    (false, Some(_)) => steps.push((site.clone(), act.clone(), TendStep::Resume)),
                    (false, None) if act.due_ic <= now => {
                        due.push((site.clone(), false, act.clone()));
                    }
                    _ => {}
                }
                continue;
            }
            if act.due_ic <= now {
                due.push((site.clone(), promoted, act.clone()));
            }
        }
    }
    tell_landed(ctx, &index, &acts_now);
    if let Some(slot) = ctx.w.module_state_mut::<TowIndex>() {
        slot.0 = index;
    }
    for (site, act, step) in steps {
        if ctx.authoritative_pending(act.operator) {
            continue;
        }
        crate::surface::tender::step(ctx, &site, &act, step);
    }
    for (site, promoted, act) in due {
        if ctx.authoritative_pending(act.operator) {
            continue;
        }
        let order = match act.stage.clone() {
            TowStage::Tend { target, work, .. } => TowOrder::Tended {
                ordinal: act.ordinal,
                site: site.clone(),
                step: crate::surface::tender::due_step(ctx, &act, target, &work, now),
            },
            TowStage::Attach { casualty } => TowOrder::Attached {
                ordinal: act.ordinal,
                site: site.clone(),
                facts: attach_facts(ctx, act.tug, casualty).map(Box::new),
            },
            TowStage::Load { carrier } => {
                let casualty = ctx
                    .w
                    .module_state::<TowIndex>()
                    .and_then(|index| index.0.get(&act.tug))
                    .map(|(_, op)| op.casualty());
                if !promoted && let Some(casualty) = casualty {
                    place_in_bay(ctx, &site, act.tug, casualty, carrier);
                }
                let facts = casualty.and_then(|casualty| {
                    let ramp = crate::surface::tick::hull_view_berth(ctx.w, carrier)
                        .map(|(_, domain, ramp, _, _)| FieldPose {
                            site: ObjId(u32::try_from(domain.0).unwrap_or(0)),
                            x_cm: ramp.x_cm,
                            y_cm: ramp.y_cm,
                        })
                        .or_else(|| {
                            // Doc 36a P0-7: a casualty already in the carrier's bay when the
                            // carrier is no longer berthed here (a finish lost, the carrier
                            // lifted) still finishes its load: the facts read the bay, and the
                            // tug's own stand serves as the ramp.
                            in_bay_of(ctx.w, casualty, carrier)
                                .then(|| on_ground(ctx.w, act.tug).map(|(_, _, pose)| pose))
                                .flatten()
                        })?;
                    load_facts(ctx, act.tug, casualty, carrier, ramp)
                });
                TowOrder::Loaded {
                    ordinal: act.ordinal,
                    site: site.clone(),
                    facts: facts.map(Box::new),
                }
            }
        };
        if submit(ctx, act.operator, act.tug, order, act.ordinal)
            && let Some(watch) = ctx.w.module_state_mut::<TowWatch>()
        {
            watch.0.insert(act.tug, (act.operator, act));
        }
    }
    // Doc 36 cut GC.6 (Q-36-27): the salvage claims whose window has run, or which a contest
    // vacates.
    crate::surface::claim::tick(ctx);
}

/// **The load's physical step**: the casualty into `carrier`'s bay, when its tug stands within
/// a cell of the ramp and the bay has room for it, and the crew aboard it with it (Q-36-52's
/// friendly half) or abandoned to extraction (Q-36-52, a non-friend's bay; doc 36 cut GC.5). A
/// written-off lease rig goes back to Occator's shed instead (Q-36-29). Idempotent: a casualty in
/// the bay (or the shed) already is left there.
fn place_in_bay(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    tug: ObjId,
    casualty: ObjId,
    carrier: ObjId,
) {
    if act::where_is(ctx.w, casualty) == (Where::Bay { hull: carrier })
        || crate::port::tv::in_shed(ctx.w, casualty)
    {
        return;
    }
    let Some((entry, _, ramp, room, slot)) = crate::surface::tick::hull_view_berth(ctx.w, carrier)
    else {
        return;
    };
    let Some((there, _, pose)) = on_ground(ctx.w, tug) else {
        return;
    };
    if &entry != site
        || there != *site
        || cells_apart(
            site_cell(ramp.x_cm, ramp.y_cm),
            site_cell(pose.x_cm, pose.y_cm),
        ) > 1
    {
        return;
    }
    let Some(load) = bay_load(ctx.w, carrier, Some(casualty)) else {
        return;
    };
    let Some((_, _, mass, _, volume)) = unit_facts(ctx.w, casualty) else {
        return;
    };
    if load
        .free()
        .check_can_load(mass.total_mass_kg(), volume)
        .is_err()
    {
        return;
    }
    let friend = friendly(ctx, site, casualty, carrier);
    // Doc 36 cut GC.5 (Q-36-29): a written-off lease rig goes back to Occator's shed, not into
    // the bay, with no claim.
    let written_off = crate::port::tv::written_off(ctx.w, casualty);
    if written_off {
        if !crate::port::tv::hand_back(ctx, casualty) {
            return;
        }
    } else if let Err(error) = mush_sim::place_interior(ctx, casualty, room, Some(slot)) {
        tracing::warn!(casualty = casualty.0, carrier = carrier.0, %error, "tacsim: a tow's load could not be placed in the bay; its finish records it");
        return;
    }
    // Q-36-52: the crew of a friendly load are aboard the carrier; anyone else's are abandoned,
    // and their extraction starts now. A written-off rig is nobody's but Occator's, so whoever is
    // still inside it is abandoned too.
    if friend && !written_off {
        crate::field::crew::aboard_loaded(ctx, casualty, carrier);
    } else {
        crate::field::crew::abandoned_loaded(ctx, site, casualty, carrier);
    }
    let template = if written_off {
        HANDED_BACK
    } else if friend {
        LOADED
    } else {
        LOADED_CUSTODY
    };
    let line = template
        .replace("{name}", &name(ctx.w, casualty))
        .replace("{carrier}", &name(ctx.w, carrier));
    crate::field::tell_site(ctx, site, &line);
}

/// **Tell each tug's operator how its finished act landed**, once the record shows it gone.
fn tell_landed(
    ctx: &mut Ctx<'_>,
    index: &BTreeMap<ObjId, (CanonicalId, RecoveryOperation)>,
    acts_now: &BTreeMap<ObjId, u32>,
) {
    let watched: Vec<(ObjId, (ObjId, TowAct))> = ctx
        .w
        .module_state::<TowWatch>()
        .map(|watch| watch.0.iter().map(|(k, v)| (*k, v.clone())).collect())
        .unwrap_or_default();
    for (tug, (operator, act)) in watched {
        if acts_now.get(&tug) == Some(&act.ordinal) {
            continue;
        }
        if let Some(watch) = ctx.w.module_state_mut::<TowWatch>() {
            watch.0.remove(&tug);
        }
        let slot = ctx
            .w
            .module_state::<TacsimStore>()
            .and_then(|store| store.tv_lease_on(tug))
            .map_or_else(|| tug.0.to_string(), |lease| lease.slot.to_string());
        let line = match act.stage {
            TowStage::Tend { target, .. } => {
                crate::surface::tender::landed_line(ctx, (tug, act.ordinal), &slot, target)
            }
            TowStage::Attach { casualty } => {
                let label = name(ctx.w, casualty);
                match index.get(&tug) {
                    Some((_, op)) if op.casualty() == casualty => TOW_ATTACHED
                        .replace("{n}", &slot)
                        .replace("{name}", &label)
                        .replace("{speed}", &metres_s(op.coupled_speed_cms())),
                    _ => TOW_INTERRUPTED
                        .replace("{n}", &slot)
                        .replace("{name}", &label),
                }
            }
            TowStage::Load { carrier } => match index.get(&tug) {
                Some((_, op)) => TOW_INTERRUPTED
                    .replace("{n}", &slot)
                    .replace("{name}", &name(ctx.w, op.casualty())),
                None => format!(
                    "Rig {slot}'s tow is aboard the {}; its tow line is free.",
                    name(ctx.w, carrier)
                ),
            },
        };
        ctx.out.sys_to_actor(operator, "tacsim.tow", line);
    }
}

// ---- the coupled drive and the field ------------------------------------------------------

/// **Stand `tug`'s casualty on the tug's cell** (the drive's placement, the field's demotion).
pub(crate) fn follow(ctx: &mut Ctx<'_>, tug: ObjId) {
    let Some(casualty) = coupled(ctx.w, tug).map(RecoveryOperation::casualty) else {
        return;
    };
    let Some((domain, pose)) = mush_sim::pose_of(ctx.w, tug) else {
        return;
    };
    if mush_sim::pose_of(ctx.w, casualty)
        .is_some_and(|(on, at)| on == domain && at.x_cm == pose.x_cm && at.y_cm == pose.y_cm)
    {
        return;
    }
    if let Err(error) = mush_sim::place(ctx, casualty, domain, pose) {
        tracing::warn!(tug = tug.0, casualty = casualty.0, %error, "tacsim: a tow's casualty could not follow its tug");
    }
}

/// **The casualties a field carries as its tugs' loads** (Q-36-20): every casualty in an active
/// tow on `record`, by its tug, and the tug's coupled speed.
#[must_use]
pub(crate) fn carried(record: &SiteRecord) -> BTreeMap<ObjId, (ObjId, u32)> {
    record
        .tows
        .active()
        .map(|op| (op.casualty(), (op.tug(), op.coupled_speed_cms())))
        .collect()
}

/// **At a field's demotion, every carried casualty stands at its tug's cell**, indexed there as
/// it was before (Q-36-20): `left` gains its entry at the new pose. A tug that left the surface
/// (irrecoverable; doc 36a lane 3, P0-8) leaves its casualty indexed where it stands.
pub(crate) fn set_down(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    fielded: &[ObjId],
    left: &mut Vec<crate::surface::site_record::LeftUnit>,
    side: u16,
) {
    let Some(record) = record_of(ctx, site) else {
        return;
    };
    for (casualty, (tug, _)) in carried(&record) {
        if fielded.contains(&casualty) || left.iter().any(|row| row.unit == casualty) {
            continue;
        }
        let Some(prior) = record
            .units
            .iter()
            .find(|row| row.unit == casualty)
            .cloned()
        else {
            continue;
        };
        // Doc 36a lane 3 (P0-8): a tug that fell irrecoverable left the surface before this runs
        // (`close::demote_units`' remains), so its casualty stands where it was held out — at its
        // own pose, which the index keeps. The tow itself ends with the remains.
        let at_tug = mush_sim::pose_of(ctx.w, tug);
        let Some((domain, pose)) = at_tug
            .clone()
            .or_else(|| mush_sim::pose_of(ctx.w, casualty))
        else {
            continue;
        };
        if at_tug.is_some()
            && let Err(error) = mush_sim::place(ctx, casualty, domain, pose.clone())
        {
            tracing::warn!(tug = tug.0, casualty = casualty.0, %error, "tacsim: a carried casualty could not be set down at its tug");
        }
        let at = crate::surface::site_record::SitePose {
            x_cm: pose.x_cm,
            y_cm: pose.y_cm,
            heading_mdeg: pose.heading_mdeg,
        };
        let mut row = prior;
        if at.on_site(side) {
            row.pose = at;
        }
        left.push(row);
    }
}

/// **The speed limit a running field admitted `unit` at**, cm/s, read off the field's last
/// snapshot (Q-36-20): a tug in tow reads its tow rate. `None` with no field, no snapshot yet or
/// no such unit in it.
#[must_use]
pub fn admitted_speed_cms(w: &World, site: &CanonicalId, unit: ObjId) -> Option<u32> {
    let document = w
        .module_state::<crate::field::FieldBook>()?
        .snapshot(site)?;
    // The arena's state rides the document as one JSON string (`crate::arena::snapshot`).
    let state: Value = serde_json::from_str(document.get("state")?.as_str()?).ok()?;
    let row = state.get("units")?.as_array()?.iter().find(|row| {
        row.get("id")
            .and_then(Value::as_u64)
            .is_some_and(|id| id == u64::from(unit.0))
    })?;
    row.get("mobility")?
        .get("max_speed_cms")?
        .as_u64()
        .and_then(|cms| u32::try_from(cms).ok())
}

/// **The room `hull`'s bay has free now** (Q-S8, Q-36-15, Q-36-54), `except` one unit counted as
/// out: what a load is checked against. `None` for a hull with no readable bay.
#[must_use]
pub fn bay_room_of(w: &World, hull: ObjId, except: Option<ObjId>) -> Option<BayRoom> {
    bay_load(w, hull, except).map(|load| load.free())
}

/// **Whether `unit` stands in `hull`'s bay** (a reader for the gates).
#[must_use]
pub fn in_bay_of(w: &World, unit: ObjId, hull: ObjId) -> bool {
    act::where_is(w, unit) == (Where::Bay { hull })
}
