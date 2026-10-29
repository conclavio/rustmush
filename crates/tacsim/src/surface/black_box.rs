//! **A hull's black box** (doc 23 cut S.1; §6 "What the host does at each end", §7 "Records",
//! D-23-1, D-23-2, D-23-5, D-23-7, D-23-8; the S.1 brief's lead rulings L2, L4, L6 and L7).
//!
//! Doc 17 §13 and doc 16 W16 ("a battle's canon is reconstructed from black boxes") want one per
//! hull, durable from the first IC epoch on: doc 20 §8's trip lines are the 32-line
//! `tacsim:notice` ring, which no inspection can read. Here it is **one `mush_core::Ledger` log
//! per hull** — log id `hull.<n>` ([`log_id`]), so its chunk documents are `log:hull.<n>:<k>` in
//! tacsim's namespace — held in the store's slot
//! (`crate::store::TacsimStore::black_boxes`) and restored from history at boot ([`restore`],
//! mush-canon's pattern).
//!
//! **Two lines an excursion in S.1** ([`BlackBoxLine`], strict, `kind`-tagged in the arena's flat
//! style): `mind_sealed` when the rig leaves the bay at its launch's end — the ROM seal over the
//! terrain vehicle (`crate::match_::seal`'s `seal_rom_for`), the hangar it left (D-23-2), the
//! warrant principal (D-23-5), the admission recorded and never refused (D-23-8) — and
//! `sortie_closed` when the epoch closes, with `IcCloseKind`'s own wire. Nothing in a line is a
//! word a player sees: no verb prints it in S.1.
//!
//! **Doc 23 cut S.2** gives the ship its own pair (`crate::port::epoch`): `mind_sealed` with
//! `host: port` and the hangar port it left ([`Hangar::Port`]) when the voyage's depart takes it
//! off a hangar port, `sortie_closed` `returned_to_base` when it docks at one again; a rig
//! launched from a ship under an open epoch names that epoch in its bay hangar (V7, D-23-2). The
//! ship's epoch is its typed `tacsim.ship_epoch` record (`crate::port::epoch`), written by the act
//! whose bundle carries the line through `append`, exactly as a rig's record is.
//!
//! **An append rides the authoritative act whose record move it records** (L4; the economy
//! journal's pattern, `mush_economy::core_integration`): the launch's finish and the recover's
//! finish (`tacsim_surface_done`) and a staff recovery (`tacsim_surface_recovery`) ask
//! `BlackBoxParticipant` — the store slot's owner participant — for one line; its prepare
//! builds the exact `Ledger::prepare_append` plan and the replay write
//! (`ReplayWrite::LedgerChunkSet`) that commits in the act's own bundle, and its bind installs
//! the plan when the composed commit installs (`Ledger::bind_prepared`). A re-run finds the
//! record already moved and asks for nothing, so idempotency is the record's (no RAM, no request
//! id). The surface tick never appends; the applier and the boot pass submit nothing.
//!
//! **The cap** (`[tacsim.surface].black_box_max_lines`, provisional 4096): `mush_core::Ledger`
//! has no retention primitive — no way to drop a sealed chunk — so an append past the cap is
//! refused with a page and the act completes without its line (the brief's L2 fallback; the
//! retention cut is owed). An open reserves the room for its close ([`admits`]): a
//! `mind_sealed` needs two lines free, a `sortie_closed` one, so an epoch whose open is written
//! always has its close written too. An epoch whose open line was not written carries
//! `unrecorded` on its record and on its close line.

use std::sync::Arc;

use mush_core::authoritative::ReplayWrite;
use mush_core::ledger::BoundLedgerAppend;
use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    CanonicalId, ChunkFloors, LedgerDoc, ModError, ObjId, OwnerId, PreparedLedgerAppend,
};
use mush_roll::commerce::ic_claims::IcCloseKind;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::core::CoreClass;
use crate::arena::mind::{MindSeal, SortieId};
use crate::arena::mind_seal::Admission;
use crate::definitions::UnitClass;
use crate::deliberation::order::CharacterRef;
use crate::minds::licence_state::SealLicenceState;
use crate::store::TacsimStore;

/// The black box logs' id prefix: a hull's log is `hull.<n>`.
pub const LOG_PREFIX: &str = "hull.";

/// **The black box participant's definition kind** (doc 23 cut S.1): its own kind with its own
/// validator, so it shares no validator with the Registry's conditional `owner_participant` one.
pub const PARTICIPANT_KIND: &str = "black_box_owner";

/// The black box participant's definition id.
pub const PARTICIPANT_ID: &str = "tacsim_black_box";

/// **A hull's black box log id**: `hull.<n>`, the hull's object number in decimal.
#[must_use]
pub fn log_id(hull: ObjId) -> SmolStr {
    SmolStr::new(format!("{LOG_PREFIX}{}", hull.0))
}

/// **The hull a black box log id names** — the inverse of [`log_id`], strict (canonical
/// decimal, no sign, no leading zero), `None` for any other log.
#[must_use]
pub fn hull_of_log(id: &str) -> Option<ObjId> {
    let digits = id.strip_prefix(LOG_PREFIX)?;
    crate::port::families::strict_decimal_u32(digits).map(ObjId)
}

/// **Which host opened or closed an epoch** (doc 23 §6). The surface excursion in S.1, the port
/// voyage in S.2; the drone phase (S.4) adds its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpochHost {
    /// The surface excursion (doc 20), the first host (D-23-7).
    Surface,
    /// **The port voyage** (doc 18; doc 23 cut S.2): a ship's epoch, opened when the launch arm's
    /// depart moves it off a hangar port and closed when it docks at one again.
    Port,
}

/// **The hangar a hull left** (D-23-2: epochs nest by record, not by object), in one of two
/// shapes, untagged so the bay's JSON is cut S.1's byte for byte:
///
/// - **a carrier's bay** (S.1): the carrier hull, the bay slot, and the carrier's own open epoch
///   — the ship's `Voyage::sortie` when it has one (cut S.2's V7), else `null`;
/// - **a hangar port** (cut S.2's V4): a port is not a hull — the port entry the ship left and
///   that port's host room.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Hangar {
    /// A carrier's bay (`{"hull":n,"slot":k,"sortie":…}`).
    Bay {
        /// The carrier hull (the Dray-P whose bay the rig left).
        hull: ObjId,
        /// The rig's bay slot.
        slot: u8,
        /// The carrier's open epoch, when it has one (cut S.2 on).
        sortie: Option<SortieId>,
    },
    /// A hangar port (`{"entry":"…","base":n}`; cut S.2).
    Port {
        /// The port entry left.
        entry: CanonicalId,
        /// That port's host room.
        base: ObjId,
    },
}

/// **The pool admission an IC dispatch records** (D-23-8: recorded, never refused): the hull's
/// unit class word as the catalog names it, the host, and how many surface epochs are open on
/// the site counting this one (the excursion cap's `counts.site`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Admitted {
    /// The unit class (`vehicle` for a terrain vehicle, `hauler` for a Dray); absent when no
    /// unit or catalog row names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<UnitClass>,
    /// The host.
    pub host: EpochHost,
    /// Open surface epochs on the site, this one included; `1` for a ship (one ship an epoch).
    pub count: u32,
}

/// **Whether the epoch opened under a seal** (cut S.1's L8): `sealed`, or `absent` when the hull
/// had no `TacticalUnit` to seal (a fixture, a content gap) — the epoch still opens, its seal
/// facts left out, so an inspection sees that it happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealWord {
    /// The seal facts follow.
    Sealed,
    /// The hull had nothing to seal.
    Absent,
}

/// **A seal's facts, as the arena's `mind_sealed` line carries them** (17b cut M3.3;
/// `crate::arena::mind::MindSeal::sealed_line`), and the seal's own digest — what the epoch's
/// record keeps ([`crate::surface::excursion::OpenSortie::seal`]) and the open line prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealFacts {
    /// The sealed mind's name (`Standard Doctrine`).
    pub mind: SmolStr,
    /// The first sixteen base-16 characters of the sealed source's SHA-256.
    pub source: SmolStr,
    /// All sixty-four of the opening card's digest.
    pub card_digest: SmolStr,
    /// The class the mind thinks at (0 for the ROM).
    pub class: CoreClass,
    /// Whether the core runs under the governor (a hull with no core is governed).
    pub governed: bool,
    /// The licence the core holds, or `doctrine`.
    pub licence: SmolStr,
    /// **The state that licence was read under at the seal** (doc 17h cut M8a; doc 17 §10):
    /// the line's parameter, never the seal's field, so `seal_digest` stands. Absent on a
    /// record written before M8a.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence_state: Option<SealLicenceState>,
    /// All sixty-four of the whole seal's digest (`MindSeal::digest`).
    pub seal_digest: SmolStr,
    /// Why the hull flies a mind other than the one chosen, from the start (`no core: the
    /// ROM` for a terrain vehicle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<SmolStr>,
    /// **The core component sealed** (the M8a fix round's F5; `MindSeal::core`): the
    /// component the hull's core capability named at the seal, absent for a hull with no core
    /// (every terrain vehicle, so no rig's record moved). Skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core: Option<SmolStr>,
    /// **That core's class at the seal** (`MindSeal::core_class`), present exactly when
    /// `core` is: what the inspection compares with the class the component reads now, as the
    /// IC audit does (`crate::minds::ic_sortie::audit`). Skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_class: Option<CoreClass>,
    /// **What an ungoverned seal admitted** (doc 17h cut M8b C4; D-UM-42): the seal line's
    /// field, evidence only. Absent for every governed seal, so no record before M8b moves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub admissions: Vec<Admission>,
    /// **The licence's holder** (doc 17h cut M8e E4; M8c B1): the operator, or the
    /// organisation that holds the licence with the operator acting for it, for a seal taken
    /// under a filed licence at the dispatch (`crate::surface::dispatch`). The warrant
    /// principal stays the operator (D-23-5). Absent for the ROM, a draft and every record
    /// before M8e. Skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<mush_economy::Holder>,
}

impl SealFacts {
    /// **The facts of `seal`**, through the arena's own line ([`MindSeal::sealed_line`]), so the
    /// two can never print a seal differently; `licence_state` is what the site that seals
    /// knows of the licence (doc 17h cut M8a: `Doctrine` for every ROM seal).
    #[must_use]
    pub fn of(seal: &MindSeal, licence_state: SealLicenceState) -> SealFacts {
        // `sealed_line` writes `Some(licence_state)`; the parameter is the same state.
        let fallback_state = licence_state;
        let (mind, source, card_digest, class, governed, licence, licence_state, admissions) =
            match seal.sealed_line(licence_state) {
                crate::arena::ledger::EventKind::MindSealed {
                    mind,
                    source,
                    card_digest,
                    class,
                    governed,
                    licence,
                    licence_state,
                    admissions,
                    ..
                } => (
                    mind,
                    source,
                    card_digest,
                    class,
                    governed,
                    licence,
                    licence_state.unwrap_or(fallback_state),
                    admissions,
                ),
                // `sealed_line` builds `MindSealed` and nothing else; the seal's own fields are
                // the same facts.
                _ => (
                    seal.mind.clone(),
                    SmolStr::default(),
                    SmolStr::default(),
                    seal.class,
                    seal.governed,
                    SmolStr::new(seal.origin.licence_label()),
                    fallback_state,
                    seal.admissions(fallback_state),
                ),
            };
        SealFacts {
            mind,
            source,
            card_digest,
            class,
            governed,
            licence,
            licence_state: Some(licence_state),
            seal_digest: SmolStr::new(crate::arena::snapshot::digest_text(&seal.digest())),
            fallback: seal.fallback.clone(),
            core: seal.core.clone(),
            core_class: seal.core.as_ref().map(|_| seal.core_class),
            admissions,
            holder: None,
        }
    }
}

/// **One black box line** (cut S.1's L7): a JSON object with a `kind` word in the arena's flat
/// style (`crate::arena::ledger::EventKind`'s words are the pattern, not the type). Strict.
// A line is built, encoded into the ledger and dropped; it is never held in bulk, so the open
// line's size beside the close line's buys nothing boxed.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BlackBoxLine {
    /// **An epoch opened under its seal** (doc 23 §6 "At the dispatch"): the hull left its
    /// hangar at `at_ic`.
    MindSealed {
        /// The hull whose epoch it is.
        unit: ObjId,
        /// Its epoch's id (the IC kind, [`SortieId::ic`]).
        sortie: SortieId,
        /// The IC instant the hull left the hangar (the launch's due instant).
        at_ic: i64,
        /// The host that opened it.
        host: EpochHost,
        /// The hangar it left (D-23-2).
        hangar: Hangar,
        /// The site port entry it went down onto (a rig); the port entry it left (a ship, cut
        /// S.2's V4).
        site: CanonicalId,
        /// The warrant principal (D-23-5): the actor of the plan the hull leaves under.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        principal: Option<CharacterRef>,
        /// Whether the seal facts follow.
        seal: SealWord,
        /// The sealed mind's name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mind: Option<SmolStr>,
        /// The sealed source's first sixteen base-16 characters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<SmolStr>,
        /// The opening card's digest.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        card_digest: Option<SmolStr>,
        /// The class the mind thinks at.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        class: Option<CoreClass>,
        /// Whether the core is governed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        governed: Option<bool>,
        /// The licence, or `doctrine`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        licence: Option<SmolStr>,
        /// **The state the licence was read under** (doc 17h cut M8a): `doctrine` on every ROM
        /// seal; a line written before M8a decodes with it absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        licence_state: Option<SealLicenceState>,
        /// The whole seal's digest.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seal_digest: Option<SmolStr>,
        /// Why the hull flies what it flies from the start.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<SmolStr>,
        /// The pool admission, recorded (D-23-8).
        admitted: Admitted,
        /// **What an ungoverned seal admitted** (doc 17h cut M8b C4; D-UM-42): `under_floor`,
        /// `unfiled`, `unlicensed`, `over_class` — a record field, never a notice. Absent for a
        /// governed seal and with no seal.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        admissions: Vec<Admission>,
        /// **The licence's holder** (doc 17h cut M8e E4): the seal's
        /// [`SealFacts::holder`] — an organisation and its acting member for an org-held
        /// licence. Absent for the ROM, a draft and every line before M8e.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        holder: Option<mush_economy::Holder>,
    },
    /// **An epoch closed** (doc 23 §6 "At the close"; D-23-1): the hull arrived in a hangar or
    /// was recovered.
    SortieClosed {
        /// The hull.
        unit: ObjId,
        /// Its epoch's id.
        sortie: SortieId,
        /// The IC instant the epoch closed.
        at_ic: i64,
        /// **How** — `IcCloseKind`'s own wire (`{"kind":"returned_to_base","base":<n>}`), under
        /// `close` because the line's own tag is `kind` (cut S.1's recorded deviation from L7's
        /// field name). S.1 writes `returned_to_base` and `recovered_to_base`;
        /// `powered_down_in_base` is unused (a rig's park is the bay) and `destroyed` is
        /// reserved (the surface has no destruction path).
        close: IcCloseKind,
        /// Whether staff closed the excursion with the rig out first (D-23-4).
        abandoned: bool,
        /// **When staff abandoned it** (doc 23 cut S.3, the lead's A1;
        /// [`crate::surface::excursion::OpenSortie::abandoned_ic`]): the instant of the staff
        /// `close` that first flagged the epoch. Skipped when `None`, so S.1's and S.2's lines
        /// print byte for byte as they did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        abandoned_ic: Option<i64>,
        /// **Who recovered it** (doc 23 cut S.3, the lead's A3): the staff actor whose home
        /// brought the rig in, set exactly when `close` is `recovered_to_base` on a surface
        /// epoch. A ship's recovery writes `None`: a placement by staff is a GOD write that
        /// leaves no actor on the hull, and no verb records one yet. Skipped when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recovered_by: Option<ObjId>,
        /// **Whether the epoch's open line was never written** (the S.1 fix round's F2;
        /// [`crate::surface::excursion::OpenSortie::unrecorded`]): the black box was dark or
        /// full, or the open was prepared off the composed path — so an inspection reading a
        /// close with no open before it tells "no line" from "no epoch". Skipped when `false`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        unrecorded: bool,
    },
}

impl BlackBoxLine {
    /// **The open line** (L7): `seal` is `None` when the hull had no `TacticalUnit`.
    #[must_use]
    pub fn mind_sealed(
        opened: &crate::surface::excursion::OpenSortie,
        tv: ObjId,
        principal: Option<CharacterRef>,
        admitted: Admitted,
    ) -> BlackBoxLine {
        sealed_line(
            (tv, opened.id, opened.opened_ic),
            (
                EpochHost::Surface,
                Hangar::Bay {
                    hull: opened.hull,
                    slot: opened.slot,
                    sortie: None,
                },
                opened.site.clone(),
            ),
            principal,
            opened.seal.as_ref(),
            admitted,
        )
    }

    /// **A rig's open line naming its carrier's open epoch** (doc 23 cut S.2, the lead's V7;
    /// D-23-2): a bay hangar's `sortie` set to `carrier`; any other line unchanged.
    #[must_use]
    pub fn with_carrier(mut self, carrier: Option<SortieId>) -> BlackBoxLine {
        if let BlackBoxLine::MindSealed {
            hangar: Hangar::Bay { sortie, .. },
            ..
        } = &mut self
        {
            *sortie = carrier;
        }
        self
    }

    /// **A ship's open line** (doc 23 cut S.2, the lead's V4): `host: port`, the hangar port it
    /// left as the hangar and the site, the filing's pilot as the warrant principal (D-23-5),
    /// the ROM seal over the ship's unit (or `absent`), and one ship admitted (D-23-8: recorded,
    /// never refused).
    #[must_use]
    pub fn ship_sealed(
        hull: ObjId,
        epoch: &crate::port::epoch::ShipEpoch,
        class: Option<UnitClass>,
    ) -> BlackBoxLine {
        sealed_line(
            (hull, epoch.id, epoch.opened_ic),
            (
                EpochHost::Port,
                Hangar::Port {
                    entry: epoch.from.clone(),
                    base: epoch.base,
                },
                epoch.from.clone(),
            ),
            Some(CharacterRef::Character {
                id: epoch.principal,
            }),
            epoch.seal.as_ref(),
            Admitted {
                class,
                host: EpochHost::Port,
                count: 1,
            },
        )
    }

    /// **A ship's close line** (doc 23 cut S.2, V4): closed `close` at `at_ic` — S.2 writes
    /// `returned_to_base` into the arrival port's host room, and **doc 23 cut S.3** (A5)
    /// `recovered_to_base` into a hangar port the ship's own journey did not bring it to;
    /// `destroyed` is reserved (no wreck path). A ship is never abandoned (there is no staff
    /// `close` for a ship: D-23-4's abandonment is the rig's) and its recovery names no
    /// recoverer (A3: a staff placement leaves no actor on the hull).
    #[must_use]
    pub fn ship_closed(
        hull: ObjId,
        epoch: &crate::port::epoch::ShipEpoch,
        at_ic: i64,
        close: IcCloseKind,
    ) -> BlackBoxLine {
        BlackBoxLine::SortieClosed {
            unit: hull,
            sortie: epoch.id,
            at_ic,
            close,
            abandoned: false,
            abandoned_ic: None,
            recovered_by: None,
            unrecorded: epoch.unrecorded,
        }
    }

    /// **The close line of `open`** (L6, L7): closed `close` at `at_ic`, carrying the epoch's
    /// `abandoned` (and, doc 23 cut S.3's A1, its instant) and (the S.1 fix round's F2)
    /// `unrecorded` flags; no recoverer ([`BlackBoxLine::recovered_by`] names one).
    #[must_use]
    pub fn sortie_closed(
        tv: ObjId,
        open: &crate::surface::excursion::OpenSortie,
        at_ic: i64,
        close: IcCloseKind,
    ) -> BlackBoxLine {
        BlackBoxLine::SortieClosed {
            unit: tv,
            sortie: open.id,
            at_ic,
            close,
            abandoned: open.abandoned,
            abandoned_ic: open.abandoned.then_some(open.abandoned_ic).flatten(),
            recovered_by: None,
            unrecorded: open.unrecorded,
        }
    }

    /// **A surface close naming its recoverer** (doc 23 cut S.3, the lead's A3): a
    /// `recovered_to_base` close's `recovered_by` set to `staff`, the actor whose home brought
    /// the rig in; any other line, or any other close kind, unchanged — the recoverer is written
    /// exactly when the close is a recovery.
    #[must_use]
    pub fn recovered_by(mut self, staff: ObjId) -> BlackBoxLine {
        if let BlackBoxLine::SortieClosed {
            close: IcCloseKind::RecoveredToBase { .. },
            recovered_by,
            ..
        } = &mut self
        {
            *recovered_by = Some(staff);
        }
        self
    }

    /// The hull the line is about.
    #[must_use]
    pub fn unit(&self) -> ObjId {
        match self {
            BlackBoxLine::MindSealed { unit, .. } | BlackBoxLine::SortieClosed { unit, .. } => {
                *unit
            }
        }
    }

    /// The epoch the line is about.
    #[must_use]
    pub fn sortie(&self) -> SortieId {
        match self {
            BlackBoxLine::MindSealed { sortie, .. } | BlackBoxLine::SortieClosed { sortie, .. } => {
                *sortie
            }
        }
    }

    /// **The line's own invariants** (the participant checks them before anything is
    /// prepared): it is `hull`'s, its epoch is an IC epoch of that hull, a sealed open line
    /// carries every seal fact and an absent one none, the hangar is another object, and a
    /// close names another object as its base.
    ///
    /// # Errors
    ///
    /// The first invariant broken, as a sentence.
    pub fn check(&self, hull: ObjId) -> Result<(), String> {
        if self.unit() != hull {
            return Err("a black box line is its own hull's".to_owned());
        }
        match self.sortie() {
            SortieId::Ic { hull: of, .. } if of == hull => {}
            SortieId::Ic { .. } | SortieId::Packed(_) => {
                return Err("a black box epoch is an IC epoch of its own hull".to_owned());
            }
        }
        match self {
            BlackBoxLine::MindSealed {
                hangar,
                seal,
                mind,
                source,
                card_digest,
                class,
                governed,
                licence,
                seal_digest,
                fallback,
                admissions,
                ..
            } => {
                // Doc 17h cut M8b: only an ungoverned seal admits anything.
                if !admissions.is_empty() && *governed != Some(false) {
                    return Err("only an ungoverned seal records admissions".to_owned());
                }
                match hangar {
                    Hangar::Bay { hull: of, slot, .. } => {
                        if *of == hull || of.0 == 0 || *slot == 0 {
                            return Err("a hull leaves another hull's bay slot".to_owned());
                        }
                    }
                    // Doc 23 cut S.2 (V4): a ship leaves a port, another object.
                    Hangar::Port { base, .. } => {
                        if *base == hull || base.0 == 0 {
                            return Err("a ship leaves another object's port".to_owned());
                        }
                    }
                }
                let all = mind.is_some()
                    && source.is_some()
                    && card_digest.is_some()
                    && class.is_some()
                    && governed.is_some()
                    && licence.is_some()
                    && seal_digest.is_some();
                let none = mind.is_none()
                    && source.is_none()
                    && card_digest.is_none()
                    && class.is_none()
                    && governed.is_none()
                    && licence.is_none()
                    && seal_digest.is_none()
                    && fallback.is_none();
                let shaped = match seal {
                    SealWord::Sealed => all,
                    SealWord::Absent => none,
                };
                if !shaped {
                    return Err(
                        "a sealed line carries every seal fact, an absent one none".to_owned()
                    );
                }
            }
            BlackBoxLine::SortieClosed {
                close,
                abandoned,
                abandoned_ic,
                recovered_by,
                ..
            } => {
                match close {
                    IcCloseKind::ReturnedToBase { base }
                    | IcCloseKind::PoweredDownInBase { base }
                    | IcCloseKind::RecoveredToBase { base } => {
                        if *base == hull || base.0 == 0 {
                            return Err("a hull closes into another hull".to_owned());
                        }
                    }
                    IcCloseKind::Destroyed => {}
                }
                // Doc 23 cut S.3 (A1, A3): the instant is an abandonment's, the recoverer a
                // recovery's.
                if abandoned_ic.is_some() && !*abandoned {
                    return Err(
                        "only an abandoned epoch closes with its abandonment's instant".to_owned(),
                    );
                }
                if recovered_by.is_some_and(|by| by.0 == 0 || by == hull)
                    || (recovered_by.is_some()
                        && !matches!(close, IcCloseKind::RecoveredToBase { .. }))
                {
                    return Err("only a recovery names its recoverer, another object".to_owned());
                }
            }
        }
        Ok(())
    }
}

/// **One open line** (cut S.1's L7, shared with cut S.2's ship line): the epoch `(unit, sortie,
/// at_ic)`, where it left `(host, hangar, site)`, the principal, the seal's facts (or `absent`)
/// and the admission.
fn sealed_line(
    (unit, sortie, at_ic): (ObjId, SortieId, i64),
    (host, hangar, site): (EpochHost, Hangar, CanonicalId),
    principal: Option<CharacterRef>,
    seal: Option<&SealFacts>,
    admitted: Admitted,
) -> BlackBoxLine {
    BlackBoxLine::MindSealed {
        unit,
        sortie,
        at_ic,
        host,
        hangar,
        site,
        principal,
        seal: if seal.is_some() {
            SealWord::Sealed
        } else {
            SealWord::Absent
        },
        mind: seal.map(|s| s.mind.clone()),
        source: seal.map(|s| s.source.clone()),
        card_digest: seal.map(|s| s.card_digest.clone()),
        class: seal.map(|s| s.class),
        governed: seal.map(|s| s.governed),
        licence: seal.map(|s| s.licence.clone()),
        licence_state: seal.and_then(|s| s.licence_state),
        seal_digest: seal.map(|s| s.seal_digest.clone()),
        fallback: seal.and_then(|s| s.fallback.clone()),
        admitted,
        admissions: seal.map(|s| s.admissions.clone()).unwrap_or_default(),
        holder: seal.and_then(|s| s.holder),
    }
}

/// **The black box participant's definition** (doc 23 cut S.1): its own kind
/// ([`PARTICIPANT_KIND`]) and the log shape it owns.
///
/// # Errors
///
/// A descriptor that does not build (never, for these constants).
pub fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    let bad = |e: &dyn std::fmt::Display| ModError::msg(e.to_string());
    DefinitionDescriptor::new(
        DefinitionKind::new(PARTICIPANT_KIND).map_err(|e| bad(&e))?,
        DefinitionId::new(PARTICIPANT_ID).map_err(|e| bad(&e))?,
        1,
        serde_json::json!({
            "format": 1,
            "protocol": "tacsim_black_box_v1",
            "log": "hull.<n>",
            "lines": ["mind_sealed", "sortie_closed"],
        }),
    )
    .map_err(|e| bad(&e))
}

/// **Restore every black box's open tail from the boot's chunk documents** (mush-canon's
/// pattern, over the `hull.*` logs alone): the documents of the other logs in tacsim's
/// namespace (the arena's `match.*` tapes) are not this ledger's and are never inspected here.
///
/// # Errors
///
/// The inspection's breaches, as one sentence; the caller pages and leaves the black boxes
/// dark (`crate::store::TacsimStore::black_box_dark`).
pub fn restore(docs: &[LedgerDoc], omitted: &ChunkFloors) -> Result<mush_core::Ledger, String> {
    let ours: Vec<LedgerDoc> = docs
        .iter()
        .filter(|(key, _)| {
            mush_core::parse_chunk_key(key).is_some_and(|(log, _)| hull_of_log(&log).is_some())
        })
        .cloned()
        .collect();
    let mut ledger = mush_core::Ledger::new();
    let floors: ChunkFloors = omitted
        .iter()
        .filter(|(log, _)| hull_of_log(log).is_some())
        .map(|(log, floor)| (log.clone(), *floor))
        .collect();
    let history = ledger.inspect_from(&ours, &floors);
    ledger
        .restore_from_history(&history)
        .map_err(|error| format!("the black boxes did not restore: {error}"))?;
    Ok(ledger)
}

/// **A hull's whole black box, from its chunk documents** (the history read an inspection uses
/// for the sealed chunks the store's RAM tail no longer holds; the chunk documents a store read
/// returns): every line of `log:hull.<n>`, in order.
///
/// # Errors
///
/// A broken history (a hole, a bad chunk) or a line that is no [`BlackBoxLine`].
pub fn history(docs: &[LedgerDoc], hull: ObjId) -> Result<Vec<(u64, BlackBoxLine)>, String> {
    let log = log_id(hull);
    let ours: Vec<LedgerDoc> = docs
        .iter()
        .filter(|(key, _)| {
            mush_core::parse_chunk_key(key).is_some_and(|(id, _)| id.as_str() == log.as_str())
        })
        .cloned()
        .collect();
    let inspected = mush_core::inspect_ledger(&ours, None);
    if !inspected.is_clean() {
        return Err(format!(
            "hull {}'s black box history has {} breach(es)",
            hull.0,
            inspected.total_breaches()
        ));
    }
    let mut lines = Vec::new();
    for (_, position) in inspected.chunk_docs(log.as_str()) {
        let (_, text) = ours
            .get(position)
            .ok_or_else(|| "a black box chunk went missing".to_owned())?;
        let entries = mush_core::typed_entries::<BlackBoxLine>(text)
            .map_err(|error| format!("a black box chunk does not decode: {error}"))?;
        lines.extend(entries);
    }
    Ok(lines)
}

// ---- the launch finish's offer facts ---------------------------------------------------------

/// **What a launching act's finish needs to open the epoch, fixed at its offer** (doc 23 cut
/// S.1, L3/L8, D-23-8): the prepare reads no World, so the finish's offer computes the ROM seal
/// over the rig's `TacticalUnit` (`crate::match_::seal`'s `seal_rom_for`), the unit's class
/// word and the site's open epochs, and the prepare builds the open line from them. `Some`
/// only for a rig whose act in flight leaves the bay (a launch, or a scan, a sounding or a
/// drive from the bay).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EpochFacts {
    /// The seal's facts; `None` when the rig has no `TacticalUnit`, or its seal refused (paged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) seal: Option<SealFacts>,
    /// The rig's unit class, as the catalog names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) class: Option<UnitClass>,
    /// Surface epochs open on the act's site on other rigs.
    pub(crate) site_epochs: u32,
    /// **The carrier's open epoch** (doc 23 cut S.2, the lead's V7; D-23-2): the hull whose bay
    /// the rig leaves, its `tacsim.ship_epoch` record's open id when one is open. Skipped when
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) carrier: Option<SortieId>,
}

/// **The offer's typed reads the epoch facts take** (doc 23 cut S.1, S.2): another rig's record,
/// and (S.2's V7) a carrier's open epoch id from its `tacsim.ship_epoch` record.
pub(crate) type EpochReads<'a> = (
    &'a dyn Fn(ObjId) -> Option<crate::surface::excursion::RigRecord>,
    &'a dyn Fn(ObjId) -> Option<SortieId>,
);

/// **The epoch facts of `record`'s launching act, at its finish's offer** (see
/// [`EpochFacts`]): `None` when nothing in flight leaves the bay. `read_rig` reads another rig's
/// record and `read_carrier` the carrier's open epoch (the offer's engine reads); the site's
/// count is over the world's rigs.
pub(crate) fn epoch_facts(
    w: &mush_core::World,
    (read_rig, read_carrier): EpochReads<'_>,
    record: &crate::surface::excursion::RigRecord,
) -> Option<EpochFacts> {
    let act = record.act.as_ref()?;
    if act.kind != crate::surface::excursion::ActKind::Launch && !act.params.launches() {
        return None;
    }
    let excursion = record.excursion.as_ref()?;
    let tv = record.tv;
    let store = w.module_state::<TacsimStore>()?;
    let unit = mush_tactics::module::unit_of(w, tv);
    // Doc 17h cut M8e E4 (doc 23 S.4a): a launch that named a mind sealed it at its dispatch;
    // the open line prints that seal, never a second one.
    let dispatched = act.seal.as_ref().map(|sealed| sealed.facts.clone());
    let seal = dispatched.or_else(|| {
        unit.and_then(|unit| {
            let sealed = crate::match_::seal::seal_rom_for(
                tv,
                unit,
                &[],
                SortieId::ic(tv, excursion.id.launch_ic),
                &crate::match_::seal::IcSealConfig {
                    minds: &store.cfg.minds,
                    reflex: &store.cfg.arena.reflex,
                    principal: Some(CharacterRef::Character {
                        id: excursion.operator,
                    }),
                    impounded: crate::match_::seal::core_impounded(w, unit),
                },
            );
            match sealed {
                // Doc 17h cut M8a: a ROM seal is read under `doctrine`.
                Ok(seal) => Some(SealFacts::of(&seal, SealLicenceState::Doctrine)),
                Err(error) => {
                    tracing::error!(
                        tv = tv.0,
                        %error,
                        "tacsim: rig #{}'s ROM seal refused; its epoch opens with the seal absent",
                        tv.0
                    );
                    None
                }
            }
        })
    });
    let class = unit.map(UnitClass::read).or_else(|| {
        let id = crate::port::tv::pool_unit(w)?;
        store
            .definitions()
            .specs
            .get(&id)
            .map(|spec| spec.class_of())
    });
    let site_epochs = crate::surface::act::world_rigs(w)
        .into_iter()
        .filter(|other| *other != tv)
        .filter_map(read_rig)
        .filter(|other| {
            other
                .sortie
                .as_ref()
                .is_some_and(|open| open.site == excursion.site)
        })
        .count();
    // Doc 23 cut S.2 (V7): the carrier's open epoch, read from its sailing plan.
    let carrier = read_carrier(excursion.hull);
    Some(EpochFacts {
        seal,
        class,
        site_epochs: u32::try_from(site_epochs).unwrap_or(u32::MAX),
        carrier,
    })
}

/// **Whether a black box holding `len` lines under `cap` takes `line`** (the S.1 fix round's
/// F1): an open reserves the room for its close — a `mind_sealed` needs two lines free
/// (`len + 2 <= cap`), a `sortie_closed` one (`len < cap`) — so a written open is never left
/// without room for its close.
#[must_use]
pub fn admits(line: &BlackBoxLine, len: u64, cap: u64) -> bool {
    let needs: u64 = match line {
        BlackBoxLine::MindSealed { .. } => 2,
        BlackBoxLine::SortieClosed { .. } => 1,
    };
    len.checked_add(needs).is_some_and(|after| after <= cap)
}

/// **The epoch an act opens when the rig leaves the bay, and its open line** (doc 23 §6; L3,
/// L5, L8; D-23-2, D-23-5, D-23-8), shared by the finish (`crate::surface::act`) and a staff
/// close that lands between the tick's placement and the finish (the S.1 fix round's F3):
/// `SortieId::ic(tv, launch_ic)`, the hangar the excursion left, opened at the act's due
/// instant, the seal fixed at the offer (absent, paged, when there was none), the excursion's
/// operator as the warrant principal and the site's admission count. `abandoned` and
/// `unrecorded` start `false`; the caller sets them. **Doc 23 cut S.2 (V7)**: the line's bay
/// hangar names the carrier's open epoch, fixed at the offer ([`EpochFacts::carrier`]), else
/// `null` as in S.1.
pub(crate) fn open_epoch(
    tv: ObjId,
    excursion: &crate::surface::excursion::Excursion,
    due_ic: i64,
    epoch: Option<&EpochFacts>,
) -> (crate::surface::excursion::OpenSortie, BlackBoxLine) {
    let seal = epoch.and_then(|facts| facts.seal.clone());
    if seal.is_none() {
        tracing::error!(
            tv = tv.0,
            "tacsim: rig #{} left the bay with nothing sealed (no TacticalUnit, or its seal \
             refused); its epoch opens with the seal absent",
            tv.0
        );
    }
    let opened = crate::surface::excursion::OpenSortie {
        id: SortieId::ic(tv, excursion.id.launch_ic),
        hull: excursion.hull,
        slot: excursion.slot,
        site: excursion.site.clone(),
        operator: excursion.operator,
        opened_ic: due_ic,
        abandoned: false,
        abandoned_ic: None,
        unrecorded: false,
        seal,
    };
    let line = BlackBoxLine::mind_sealed(
        &opened,
        tv,
        Some(CharacterRef::Character {
            id: excursion.operator,
        }),
        Admitted {
            class: epoch.and_then(|facts| facts.class),
            host: EpochHost::Surface,
            count: epoch.map_or(1, |facts| facts.site_epochs.saturating_add(1)),
        },
    )
    .with_carrier(epoch.and_then(|facts| facts.carrier));
    (opened, line)
}

/// **Append one line to `hull`'s black box inside the act being prepared** (L4), and say
/// whether it was written:
///
/// - **on the composed path** (production's world task) through the participant, the line's
///   foreseeable refusals checked first and paged — a dark black box, or one without the room
///   [`admits`] asks — so a refusal never poisons the act: the act completes, its record moves
///   and the line alone is lost (`Ok(false)`);
/// - **off it** (a typed-only preparation, which offers no owner participants), or with no
///   handle composed for the action, nothing is asked: a warning, and `Ok(false)`.
///
/// The caller records `Ok(false)` for an open on the epoch
/// ([`crate::surface::excursion::OpenSortie::unrecorded`]).
///
/// # Errors
///
/// The participant's own refusal of a line the checks passed (the act fails, nothing commits).
pub(crate) fn append(
    ctx: &mut mush_core::PrepareCtx<'_>,
    handle: Option<&mush_core::participant::ParticipantHandle<BlackBoxParticipant>>,
    hull: ObjId,
    line: BlackBoxLine,
) -> Result<bool, mush_core::ActionError> {
    let Some(handle) = handle else {
        tracing::warn!(
            hull = hull.0,
            "tacsim: no black box participant is composed for this act; its line is not written"
        );
        return Ok(false);
    };
    let Ok(world) = ctx.world() else {
        tracing::warn!(
            hull = hull.0,
            "tacsim: this act was prepared off the composed path (no owner participants); hull \
             #{}'s black box line is not written",
            hull.0
        );
        return Ok(false);
    };
    let Some(store) = world.module_state::<TacsimStore>() else {
        return Ok(false);
    };
    let cap = u64::from(store.cfg.surface.black_box_max_lines);
    let len = store.black_box_len(hull);
    if store.black_box_dark {
        tracing::error!(
            hull = hull.0,
            "tacsim: the black boxes are dark (their restore refused at boot); hull #{}'s line \
             is not written; staff repair the log:hull.* documents and restart",
            hull.0
        );
        return Ok(false);
    }
    if !admits(&line, len, cap) {
        tracing::error!(
            hull = hull.0,
            lines = len,
            cap,
            "tacsim: hull #{}'s black box is full ([tacsim.surface].black_box_max_lines; an open \
             keeps room for its close); the line is refused and not written (no retention drops \
             a sealed chunk yet)",
            hull.0
        );
        return Ok(false);
    }
    ctx.prepare_owner(handle, Append { hull, line })
        .map(|appended| {
            tracing::debug!(
                hull = hull.0,
                seq = appended.seq,
                "tacsim: a black box line rides this act"
            );
            true
        })
        .map_err(|error| {
            tracing::error!(hull = hull.0, %error, "tacsim: the black box refused a line");
            crate::port_tick::port_error(
                mush_core::ActionErrorCode::Integrity,
                "the black box did not take the line",
            )
        })
}

// ---- the participant -------------------------------------------------------------------------

/// **The black box's owner participant** (doc 23 cut S.1; the economy journal's pattern): one
/// line appended to one hull's log inside the authoritative act that asks for it. Registered on
/// the store slot for `tacsim_surface_done`, `tacsim_surface_recovery` and (doc 23 cut S.2) the
/// ship's `tacsim_voyage_epoch`.
pub(crate) struct BlackBoxParticipant;

/// One line for one hull.
pub(crate) struct Append {
    /// The hull whose log takes the line.
    pub(crate) hull: ObjId,
    /// The line.
    pub(crate) line: BlackBoxLine,
}

/// What the participant prepared: the line's sequence number in its log.
pub(crate) struct Appended {
    /// The sequence number reserved.
    pub(crate) seq: u64,
}

/// The prepared plan, held across the commit.
pub(crate) struct Patch {
    log: SmolStr,
    seq: u64,
    prepared: PreparedLedgerAppend,
}

/// The plan bound to its unchanged tail.
pub(crate) struct Bound<'a>(BoundLedgerAppend<'a>);

impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        // The replay write already carried the chunk to the store in the act's bundle; the
        // ModuleWrite the install hands back is that same chunk, so nothing more is queued.
        let (_seq, _write) = self.0.apply();
    }
}

fn refused(message: &'static str) -> ParticipantError {
    ParticipantError::refused(message)
}

impl OwnerParticipant for BlackBoxParticipant {
    type State = TacsimStore;
    type Request = Append;
    type Output = Appended;
    type Patch = Patch;
    type Bound<'a> = Bound<'a>;

    fn prepare(
        &self,
        state: &TacsimStore,
        _ctx: &OwnerPrepareContext<'_>,
        request: Append,
    ) -> Result<OwnerPreparation<Appended, Patch>, ParticipantError> {
        if state.black_box_dark {
            return Err(refused("the black boxes are dark"));
        }
        request
            .line
            .check(request.hull)
            .map_err(|_| refused("a black box line is out of bounds"))?;
        let log = log_id(request.hull);
        let seq = state.black_boxes.next_seq(log.as_str());
        if !admits(
            &request.line,
            seq,
            u64::from(state.cfg.surface.black_box_max_lines),
        ) {
            return Err(refused("the black box is full"));
        }
        let value = serde_json::to_value(&request.line).map_err(|_| ParticipantError::Encoding)?;
        let prepared = state
            .black_boxes
            .prepare_append(log.as_str(), value)
            .map_err(|_| refused("the black box append is unsafe"))?;
        let write = prepared
            .authoritative_replay_write(OwnerId::new(crate::NS).map_err(|_| refused("owner"))?)
            .map_err(|_| ParticipantError::Encoding)?;
        Ok(OwnerPreparation::new(
            Appended {
                seq: prepared.seq(),
            },
            Patch { log, seq, prepared },
            vec![write],
        ))
    }

    fn recheck(
        &self,
        state: &TacsimStore,
        _ctx: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        if state.black_box_dark || state.black_boxes.next_seq(patch.log.as_str()) != patch.seq {
            return Err(refused("the black box moved"));
        }
        Ok(())
    }

    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut TacsimStore,
        patch: Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        state
            .black_boxes
            .bind_prepared(patch.prepared)
            .map(Bound)
            .map_err(|_| refused("the black box moved"))
    }

    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        matches!(
            write,
            ReplayWrite::LedgerChunkSet { write }
                if write.namespace().as_str() == crate::NS && hull_of_log(write.log_id()).is_some()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The log id and its inverse** — `hull.<n>`, strict both ways.
    #[test]
    fn s1_a_black_box_log_is_its_hulls_alone() {
        assert_eq!(log_id(ObjId(42)).as_str(), "hull.42");
        assert_eq!(hull_of_log("hull.42"), Some(ObjId(42)));
        for other in [
            "hull.", "hull.042", "hull.+4", "hull.4x", "match.4", "hull4", "",
        ] {
            assert_eq!(hull_of_log(other), None, "{other}");
        }
    }

    fn opened(seal: Option<SealFacts>) -> crate::surface::excursion::OpenSortie {
        crate::surface::excursion::OpenSortie {
            id: SortieId::ic(ObjId(30), 500),
            hull: ObjId(20),
            slot: 1,
            site: CanonicalId::new("ahuna").expect("an id"),
            operator: ObjId(10),
            opened_ic: 800,
            abandoned: false,
            abandoned_ic: None,
            unrecorded: false,
            seal,
        }
    }

    fn facts() -> SealFacts {
        SealFacts {
            mind: "Standard Doctrine".into(),
            source: "0123456789abcdef".into(),
            card_digest: "c".repeat(64).into(),
            class: CoreClass::Zero,
            governed: true,
            licence: "doctrine".into(),
            licence_state: Some(SealLicenceState::Doctrine),
            seal_digest: "d".repeat(64).into(),
            fallback: Some(crate::match_::NO_CORE_FALLBACK.into()),
            core: None,
            core_class: None,
            admissions: Vec::new(),
            holder: None,
        }
    }

    /// **Doc 17h cut M8b G12**: only an ungoverned seal records admissions — a governed open
    /// line carrying one is refused by `check`; the same line ungoverned stands.
    #[test]
    fn m8b_a_governed_line_carries_no_admissions() {
        let admitted = Admitted {
            class: Some(UnitClass::Vehicle),
            host: EpochHost::Surface,
            count: 1,
        };
        let mut governed = facts();
        governed.admissions = vec![Admission::UnderFloor];
        let line =
            BlackBoxLine::mind_sealed(&opened(Some(governed.clone())), ObjId(30), None, admitted);
        assert!(line.check(ObjId(30)).is_err());
        let mut ungoverned = governed;
        ungoverned.governed = false;
        let line = BlackBoxLine::mind_sealed(&opened(Some(ungoverned)), ObjId(30), None, admitted);
        assert_eq!(line.check(ObjId(30)), Ok(()));
    }

    /// **The two line shapes** (L7): the open line is `kind: mind_sealed`, flat, with the hangar,
    /// the admission and the seal facts (or `seal: absent` and none of them); the close line is
    /// `kind: sortie_closed` with `IcCloseKind`'s own wire under `close`; both round-trip, an
    /// unknown field is refused, and `check` holds each to its hull.
    #[test]
    fn s1_the_two_line_shapes_are_strict() {
        let admitted = Admitted {
            class: Some(UnitClass::Vehicle),
            host: EpochHost::Surface,
            count: 1,
        };
        let principal = Some(CharacterRef::Character { id: ObjId(10) });
        let sealed =
            BlackBoxLine::mind_sealed(&opened(Some(facts())), ObjId(30), principal, admitted);
        let value = serde_json::to_value(&sealed).expect("encodes");
        assert_eq!(value["kind"], "mind_sealed");
        assert_eq!(
            value["sortie"],
            serde_json::json!({"ic": {"hull": 30, "at_ic": 500}})
        );
        assert_eq!(value["at_ic"], 800);
        assert_eq!(value["host"], "surface");
        assert_eq!(
            value["hangar"],
            serde_json::json!({"hull": 20, "slot": 1, "sortie": null})
        );
        assert_eq!(
            value["principal"],
            serde_json::json!({"kind": "character", "id": 10})
        );
        assert_eq!(value["seal"], "sealed");
        assert_eq!(value["fallback"], "no core: the ROM");
        assert_eq!(value["licence"], "doctrine");
        // Doc 17h cut M8a: the licence state beside the licence, and a line written before
        // it (no `licence_state`) decodes with the state absent.
        assert_eq!(value["licence_state"], "doctrine");
        assert_eq!(
            value["admitted"],
            serde_json::json!({"class": "vehicle", "host": "surface", "count": 1})
        );
        assert_eq!(
            serde_json::from_value::<BlackBoxLine>(value.clone()).ok(),
            Some(sealed.clone())
        );
        let mut before_m8a = value;
        before_m8a
            .as_object_mut()
            .expect("an object")
            .remove("licence_state");
        match serde_json::from_value::<BlackBoxLine>(before_m8a) {
            Ok(BlackBoxLine::MindSealed {
                licence_state,
                licence,
                ..
            }) => {
                assert_eq!(licence_state, None, "an old line's state is absent");
                assert_eq!(licence.as_deref(), Some("doctrine"));
            }
            other => panic!("an old open line decodes: {other:?}"),
        }
        assert_eq!(sealed.check(ObjId(30)), Ok(()));
        assert!(sealed.check(ObjId(31)).is_err(), "another hull's line");

        let absent = BlackBoxLine::mind_sealed(&opened(None), ObjId(30), None, admitted);
        let value = serde_json::to_value(&absent).expect("encodes");
        assert_eq!(value["seal"], "absent");
        assert!(value.get("mind").is_none() && value.get("principal").is_none());
        assert_eq!(absent.check(ObjId(30)), Ok(()));

        let closed = BlackBoxLine::sortie_closed(
            ObjId(30),
            &opened(None),
            4_000,
            IcCloseKind::ReturnedToBase { base: ObjId(20) },
        );
        let value = serde_json::to_value(&closed).expect("encodes");
        assert_eq!(
            value,
            serde_json::json!({
                "kind": "sortie_closed",
                "unit": 30,
                "sortie": {"ic": {"hull": 30, "at_ic": 500}},
                "at_ic": 4000,
                "close": {"kind": "returned_to_base", "base": 20},
                "abandoned": false,
            })
        );
        assert_eq!(
            serde_json::from_value::<BlackBoxLine>(value.clone()).ok(),
            Some(closed.clone())
        );
        assert_eq!(closed.check(ObjId(30)), Ok(()));
        let mut extra = value;
        extra["note"] = serde_json::json!("x");
        assert!(
            serde_json::from_value::<BlackBoxLine>(extra).is_err(),
            "strict"
        );
        // The fix round's F2: an epoch whose open line was not written says so on its close
        // (and on its record); `false` is not written.
        let quiet = crate::surface::excursion::OpenSortie {
            unrecorded: true,
            abandoned: true,
            ..opened(None)
        };
        let value = serde_json::to_value(&quiet).expect("encodes");
        assert_eq!(value["unrecorded"], true);
        assert!(
            serde_json::to_value(opened(None))
                .expect("encodes")
                .get("unrecorded")
                .is_none()
        );
        let unrecorded = BlackBoxLine::sortie_closed(
            ObjId(30),
            &quiet,
            4_000,
            IcCloseKind::RecoveredToBase { base: ObjId(20) },
        );
        let value = serde_json::to_value(&unrecorded).expect("encodes");
        assert_eq!(
            value,
            serde_json::json!({
                "kind": "sortie_closed",
                "unit": 30,
                "sortie": {"ic": {"hull": 30, "at_ic": 500}},
                "at_ic": 4000,
                "close": {"kind": "recovered_to_base", "base": 20},
                "abandoned": true,
                "unrecorded": true,
            })
        );
        assert_eq!(
            serde_json::from_value::<BlackBoxLine>(value).ok(),
            Some(unrecorded)
        );
        let close_line = |sortie: SortieId, base: ObjId| BlackBoxLine::SortieClosed {
            unit: ObjId(30),
            sortie,
            at_ic: 4_000,
            close: IcCloseKind::RecoveredToBase { base },
            abandoned: true,
            abandoned_ic: None,
            recovered_by: None,
            unrecorded: false,
        };
        assert!(
            close_line(SortieId::ic(ObjId(30), 500), ObjId(20))
                .check(ObjId(30))
                .is_ok()
        );
        assert!(
            close_line(SortieId::in_match(0, ObjId(30)), ObjId(20))
                .check(ObjId(30))
                .is_err(),
            "an arena id is no IC epoch"
        );
        assert!(
            close_line(SortieId::ic(ObjId(30), 500), ObjId(30))
                .check(ObjId(30))
                .is_err(),
            "a hull closes into another hull"
        );
        // Doc 23 cut S.2 (V4): a ship's hangar is a port, `{entry, base}`, strict; the bay's shape
        // above is S.1's byte for byte. (V7) A bay names its carrier's open epoch.
        let port = Hangar::Port {
            entry: CanonicalId::new("ceres_port").expect("an id"),
            base: ObjId(7),
        };
        let value = serde_json::to_value(&port).expect("encodes");
        assert_eq!(value, serde_json::json!({"entry": "ceres_port", "base": 7}));
        assert_eq!(serde_json::from_value::<Hangar>(value).ok(), Some(port));
        for strayed in [
            serde_json::json!({"entry": "ceres_port", "base": 7, "slot": 1}),
            serde_json::json!({"hull": 20, "slot": 1, "sortie": null, "base": 7}),
            serde_json::json!({"entry": "ceres_port"}),
        ] {
            assert!(
                serde_json::from_value::<Hangar>(strayed.clone()).is_err(),
                "strict: {strayed}"
            );
        }
        let carried = sealed.with_carrier(Some(SortieId::ic(ObjId(20), 100)));
        assert_eq!(
            serde_json::to_value(&carried).expect("encodes")["hangar"],
            serde_json::json!({"hull": 20, "slot": 1, "sortie": {"ic": {"hull": 20, "at_ic": 100}}})
        );
        assert_eq!(carried.check(ObjId(30)), Ok(()));
        // Doc 23 cut S.3 (A1, A3): the abandonment's instant and the recoverer, in their own
        // frame (the M7 stack-frame limit).
        s3_the_abandonment_and_the_recoverer_are_strict();
    }

    /// **Doc 23 cut S.3's two fields on the close line** (A1, A3): `abandoned_ic` beside
    /// `abandoned`, `recovered_by` on a recovery, each skipped when `None` (the cases above print
    /// byte for byte as S.1 and S.2 pinned them); `recovered_by` touches a recovery only;
    /// `check` refuses an instant on an epoch never abandoned and a recoverer on any other close.
    fn s3_the_abandonment_and_the_recoverer_are_strict() {
        let abandoned = crate::surface::excursion::OpenSortie {
            abandoned: true,
            abandoned_ic: Some(2_500),
            ..opened(None)
        };
        let value = serde_json::to_value(&abandoned).expect("encodes");
        assert_eq!(value["abandoned"], true);
        assert_eq!(value["abandoned_ic"], 2_500);
        assert!(
            serde_json::to_value(opened(None))
                .expect("encodes")
                .get("abandoned_ic")
                .is_none(),
            "skipped when none"
        );
        let recovered = BlackBoxLine::sortie_closed(
            ObjId(30),
            &abandoned,
            4_000,
            IcCloseKind::RecoveredToBase { base: ObjId(21) },
        )
        .recovered_by(ObjId(9));
        let value = serde_json::to_value(&recovered).expect("encodes");
        assert_eq!(
            value,
            serde_json::json!({
                "kind": "sortie_closed",
                "unit": 30,
                "sortie": {"ic": {"hull": 30, "at_ic": 500}},
                "at_ic": 4000,
                "close": {"kind": "recovered_to_base", "base": 21},
                "abandoned": true,
                "abandoned_ic": 2500,
                "recovered_by": 9,
            })
        );
        assert_eq!(
            serde_json::from_value::<BlackBoxLine>(value.clone()).ok(),
            Some(recovered.clone())
        );
        assert_eq!(recovered.check(ObjId(30)), Ok(()));
        let mut extra = value;
        extra["recoverer"] = serde_json::json!(9);
        assert!(
            serde_json::from_value::<BlackBoxLine>(extra).is_err(),
            "strict"
        );
        // A return is no recovery: `recovered_by` leaves it alone.
        let returned = BlackBoxLine::sortie_closed(
            ObjId(30),
            &abandoned,
            4_000,
            IcCloseKind::ReturnedToBase { base: ObjId(20) },
        );
        assert_eq!(returned.clone().recovered_by(ObjId(9)), returned);
        assert_eq!(
            serde_json::to_value(&returned).expect("encodes")["abandoned_ic"],
            2_500
        );
        let line = |close: IcCloseKind, abandoned: bool, at: Option<i64>, by: Option<ObjId>| {
            BlackBoxLine::SortieClosed {
                unit: ObjId(30),
                sortie: SortieId::ic(ObjId(30), 500),
                at_ic: 4_000,
                close,
                abandoned,
                abandoned_ic: at,
                recovered_by: by,
                unrecorded: false,
            }
        };
        let recovery = IcCloseKind::RecoveredToBase { base: ObjId(20) };
        let back = IcCloseKind::ReturnedToBase { base: ObjId(20) };
        assert!(
            line(recovery, true, Some(1), Some(ObjId(9)))
                .check(ObjId(30))
                .is_ok()
        );
        assert!(
            line(recovery, false, None, Some(ObjId(9)))
                .check(ObjId(30))
                .is_ok()
        );
        assert!(
            line(recovery, false, Some(1), None)
                .check(ObjId(30))
                .is_err(),
            "an instant with no abandonment"
        );
        assert!(
            line(back, true, Some(1), Some(ObjId(9)))
                .check(ObjId(30))
                .is_err(),
            "a recoverer on a return"
        );
        assert!(
            line(recovery, true, None, Some(ObjId(0)))
                .check(ObjId(30))
                .is_err(),
            "no one recovers"
        );
        // A ship's close never names one (A3, A5).
        let epoch = crate::port::epoch::ShipEpoch {
            id: SortieId::ic(ObjId(40), 900),
            from: CanonicalId::new("ceres_port").expect("an id"),
            base: ObjId(7),
            principal: ObjId(10),
            opened_ic: 900,
            unrecorded: false,
            seal: None,
        };
        let ship = BlackBoxLine::ship_closed(
            ObjId(40),
            &epoch,
            5_000,
            IcCloseKind::RecoveredToBase { base: ObjId(8) },
        );
        assert_eq!(
            serde_json::to_value(&ship).expect("encodes"),
            serde_json::json!({
                "kind": "sortie_closed",
                "unit": 40,
                "sortie": {"ic": {"hull": 40, "at_ic": 900}},
                "at_ic": 5000,
                "close": {"kind": "recovered_to_base", "base": 8},
                "abandoned": false,
            })
        );
        assert_eq!(ship.check(ObjId(40)), Ok(()));
    }

    /// **A rig's open line names its carrier's open epoch, or none** (doc 23 cut S.2, V7; the fix
    /// round's R7): `open_epoch` with no carrier epoch fixed at the offer writes the bay hangar's
    /// `sortie: null`, exactly S.1's line; with one it writes that id; with no offer facts at all
    /// it writes `null` too.
    #[test]
    fn s2_open_epoch_names_the_carriers_epoch_or_null() {
        let excursion = crate::surface::excursion::Excursion {
            id: crate::surface::excursion::ExcursionId {
                tv: ObjId(30),
                launch_ic: 500,
            },
            operator: ObjId(10),
            hull: ObjId(20),
            slot: 1,
            site: CanonicalId::new("ahuna_port").expect("an id"),
        };
        let facts_with = |carrier: Option<SortieId>| EpochFacts {
            seal: Some(facts()),
            class: Some(UnitClass::Vehicle),
            site_epochs: 0,
            carrier,
        };
        let hangar_of =
            |line: &BlackBoxLine| serde_json::to_value(line).expect("encodes")["hangar"].clone();
        let (_, null) = open_epoch(ObjId(30), &excursion, 800, Some(&facts_with(None)));
        assert_eq!(
            hangar_of(&null),
            serde_json::json!({"hull": 20, "slot": 1, "sortie": null}),
            "no carrier epoch: null, S.1's line"
        );
        let (_, bare) = open_epoch(ObjId(30), &excursion, 800, None);
        assert_eq!(
            hangar_of(&bare),
            serde_json::json!({"hull": 20, "slot": 1, "sortie": null})
        );
        let ship = SortieId::ic(ObjId(20), 100);
        let (opened, named) = open_epoch(ObjId(30), &excursion, 800, Some(&facts_with(Some(ship))));
        assert_eq!(
            hangar_of(&named),
            serde_json::json!({"hull": 20, "slot": 1, "sortie": {"ic": {"hull": 20, "at_ic": 100}}})
        );
        assert_eq!(opened.id, SortieId::ic(ObjId(30), 500));
        assert_eq!(named.check(ObjId(30)), Ok(()));
    }

    /// **A ship's two lines** (doc 23 cut S.2, V4): the open line is `host: port`, its hangar and
    /// its site the port entry left, the pilot the principal, one hauler admitted; the close is
    /// `returned_to_base` into the arrival port's host room, never abandoned. Both round-trip and
    /// `check` holds each to its hull and to another object's port.
    #[test]
    fn s2_a_ship_leaves_and_reaches_a_port() {
        let epoch = crate::port::epoch::ShipEpoch {
            id: SortieId::ic(ObjId(40), 900),
            from: CanonicalId::new("ceres_port").expect("an id"),
            base: ObjId(7),
            principal: ObjId(10),
            opened_ic: 900,
            unrecorded: false,
            seal: Some(facts()),
        };
        let open = BlackBoxLine::ship_sealed(ObjId(40), &epoch, Some(UnitClass::Hauler));
        let value = serde_json::to_value(&open).expect("encodes");
        assert_eq!(value["kind"], "mind_sealed");
        assert_eq!(value["host"], "port");
        assert_eq!(
            value["hangar"],
            serde_json::json!({"entry": "ceres_port", "base": 7})
        );
        assert_eq!(value["site"], "ceres_port");
        assert_eq!(value["at_ic"], 900);
        assert_eq!(
            value["principal"],
            serde_json::json!({"kind": "character", "id": 10})
        );
        assert_eq!(
            value["admitted"],
            serde_json::json!({"class": "hauler", "host": "port", "count": 1})
        );
        assert_eq!(value["seal"], "sealed");
        assert_eq!(
            serde_json::from_value::<BlackBoxLine>(value).ok(),
            Some(open.clone())
        );
        assert_eq!(open.check(ObjId(40)), Ok(()));
        let home = crate::port::epoch::ShipEpoch {
            base: ObjId(40),
            ..epoch.clone()
        };
        assert!(
            BlackBoxLine::ship_sealed(ObjId(40), &home, None)
                .check(ObjId(40))
                .is_err(),
            "a ship leaves another object's port"
        );
        let closed = BlackBoxLine::ship_closed(
            ObjId(40),
            &epoch,
            5_000,
            IcCloseKind::ReturnedToBase { base: ObjId(8) },
        );
        assert_eq!(
            serde_json::to_value(&closed).expect("encodes"),
            serde_json::json!({
                "kind": "sortie_closed",
                "unit": 40,
                "sortie": {"ic": {"hull": 40, "at_ic": 900}},
                "at_ic": 5000,
                "close": {"kind": "returned_to_base", "base": 8},
                "abandoned": false,
            })
        );
        assert_eq!(closed.check(ObjId(40)), Ok(()));
        // A ship's line never carries a carrier: `with_carrier` touches a bay hangar only.
        assert_eq!(
            open.clone().with_carrier(Some(SortieId::ic(ObjId(1), 1))),
            open
        );
    }

    /// **An open reserves the room for its close** (the S.1 fix round's F1): appended through a
    /// real `Ledger` while [`admits`] says so — under a cap of 3, one open, its close, and a
    /// second open refused (it would leave no room for its own close); under a cap of 2, the
    /// open and its close fit exactly, and nothing after.
    #[test]
    fn s1_an_open_keeps_room_for_its_close() {
        let open = BlackBoxLine::mind_sealed(
            &opened(None),
            ObjId(30),
            None,
            Admitted {
                class: None,
                host: EpochHost::Surface,
                count: 1,
            },
        );
        let close = BlackBoxLine::sortie_closed(
            ObjId(30),
            &opened(None),
            4_000,
            IcCloseKind::ReturnedToBase { base: ObjId(20) },
        );
        let log = log_id(ObjId(30));
        // Append `line` when admitted under `cap`; whether it was.
        let take = |ledger: &mut mush_core::Ledger, line: &BlackBoxLine, cap: u64| -> bool {
            if !admits(line, ledger.next_seq(log.as_str()), cap) {
                return false;
            }
            let prepared = ledger
                .prepare_append(log.as_str(), serde_json::to_value(line).expect("encodes"))
                .expect("prepares");
            ledger.apply_prepared(prepared).expect("applies");
            true
        };
        let mut three = mush_core::Ledger::new();
        assert!(take(&mut three, &open, 3), "the first open");
        assert!(take(&mut three, &close, 3), "its close");
        assert!(
            !take(&mut three, &open, 3),
            "a second open has no room for its close"
        );
        assert!(
            take(&mut three, &close, 3),
            "a close still fits in the last line"
        );
        assert_eq!(three.next_seq(log.as_str()), 3);
        let mut two = mush_core::Ledger::new();
        assert!(take(&mut two, &open, 2));
        assert!(take(&mut two, &close, 2));
        assert!(!take(&mut two, &close, 2), "past the cap nothing fits");
        assert!(!take(&mut two, &open, 2));
        assert_eq!(two.next_seq(log.as_str()), 2);
        assert!(!admits(&open, 0, 1), "a cap of one never takes an open");
        assert!(admits(&close, 0, 1));
        assert!(!admits(&open, u64::MAX, u64::MAX), "no overflow");
    }
}
