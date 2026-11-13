//! **A player's own mind sealed aboard a ship** (doc 44 unit MW.13; Q-44-26; doc 23 D-23-11,
//! D-23-12; doc 17 "Private filing and the owner's licence").
//!
//! `+ship/mind filed:<licence> [org=<organisation>] go`, typed at the helm of a hull docked at a
//! port, seals a filed mind its holder may fly on her. The mind has her helm and sensors, and
//! no fire channel (the ruling: "the fire channels wait for the mounts' redesign"). Its class
//! comes from the hull's core, as on the ground. It flies her only while her crew has no link
//! (`crate::space::backup`): a crew at its stations flies her itself.
//!
//! # The seal is the Roll's claim
//!
//! Nothing is sealed here. The verb files a claim cursor on the hull's epoch record
//! ([`ShipEpochRecord::claim`]); the port tick's epoch arm then opens the Roll's claim by the
//! path a rig's launch uses (doc 23 §17.3: the IC host, a `StartPermit` over the hull as it
//! stands at her berth, submitted as the operator), and the claim's own capture is the one seal
//! (D-23-14: `minds::ic_sortie`'s `Execution::capture`, through `seal_with_policy`). So the
//! seal, its class, its principal and its claim are the claim book's, which the Roll and the
//! four owners persist; a restart reads back the same seal with nothing in a bubble's record
//! to rebuild it from. This is D-23-12's "cut that cores a ship": D-23-11 holds unchanged, with
//! the port's berth as the hangar.
//!
//! # Its life
//!
//! | The cursor | Means | Moves when |
//! |---|---|---|
//! | `Opening` | the holder asked; the claim opens, then stands | the hull's epoch closes at a port (`Closing`, in the bundle that closes the epoch); `+ship/mind off` at her berth; the seal no longer fits her ([`fits`]) |
//! | `Closing` | the claim's Close is owed | the book holds the claim closed |
//! | `Refused` | the Registry did not admit the claim | the next `+ship/mind` |
//!
//! A ship's cursor is never `Open`: the book says whether the claim is open
//! ([`claim_of`]), and one fact is not kept in two places. The claim closes once, because one
//! record's one cursor names one open token and the book closes a token once.
//!
//! # A seal is checked again where it is used
//!
//! [`sealed`] is the only reader the space seat has, and it answers a seal only while the
//! hull still carries the core the seal was taken over and the character who sealed it still
//! has authority over the hull (`mush_roll::commerce::ic_claims::hull_authority`, the rule the
//! claim's Open asked). So no route by which a hull's core or title changes has to remember
//! the seal: a refit, a strip, a sale, a sweep, a repossession or a staff transfer each leave a
//! seal that [`sealed`] no longer answers, and the arm closes its claim on its next fire
//! (`void`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionTxn, AuthoritativeActionOutput, CmdError,
    CommitToken, Ctx, IdempotencyKeyText, ObjId, PerceivedRef, PrepareCtx, World,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use smol_str::SmolStr;

use crate::arena::mind::MindSeal;
use crate::minds::ic_sortie::{
    ClaimRead, IcHostTicket, SurfaceClose, active_under_licence, claim_closed, claim_execution,
    claim_of, claim_seal,
};
use crate::port::coop::{helm_of, name_of, say};
use crate::port::epoch::{EPOCH_KEY_PREFIX, EpochKey, EpochProvider, ShipEpochRecord};
use crate::port::ship::berth_of;
use crate::port_tick::{Fire, NoticeLevel, PortTickState, Step, SwapRefused, port_error};
use crate::surface::dispatch::{Dispatched, MindWord};
use crate::surface::excursion::{ClaimCloseKind, ClaimCursor};

/// The switch.
pub const MIND_SWITCH: &str = "mind";
/// The grammar.
pub const MIND_USAGE: &str = "Usage: +ship/mind | +ship/mind filed:<licence> [org=<organisation>] [go] | +ship/mind off [go]";
/// No mind aboard: `{name}`.
pub const MIND_NONE: &str =
    "No mind is sealed aboard the {name}. If you lose your link, the house default takes her home.";
/// A claim on its way: `{name}`, `{licence}`.
pub const MIND_ADMITTING: &str =
    "The Registry is admitting licence {licence}'s claim on the {name}.";
/// A mind aboard: `{mind}`, `{name}`, `{licence}`, `{class}`.
pub const MIND_ABOARD: &str = "{mind} is sealed aboard the {name} under licence {licence}, at Class {class}. It has her helm and sensors while her crew has no link, until she next docks at a port.";
/// A mind aboard that no longer fits her: `{mind}`, `{name}`.
pub const MIND_UNFIT: &str =
    "{mind} was sealed aboard the {name}, and no longer fits her; its claim is being closed.";
/// A claim closing: `{name}`.
pub const MIND_CLOSING: &str = "The claim on the mind last sealed aboard the {name} is closing.";
/// The last claim was refused: `{name}`.
pub const MIND_WAS_REFUSED: &str =
    "The Registry did not admit the last mind you asked to seal aboard the {name}.";
/// Not at a berth: `{name}`.
pub const MIND_BERTH: &str =
    "A mind is sealed aboard the {name} at her berth in a port, before she undocks.";
/// No core: `{name}`.
pub const MIND_NO_CORE: &str =
    "The {name} carries no core; her drive ROM flies a filed plan and decides nothing.";
/// Already sealed: `{name}`.
pub const MIND_ALREADY: &str =
    "A mind is already sealed aboard the {name}; +ship/mind off unseals it at her berth.";
/// The last passage's close has not landed: `{name}`.
pub const MIND_PASSAGE: &str =
    "The {name}'s last passage is still being written up; try again in a moment.";
/// A draft: no ship's core seals one yet.
pub const MIND_DRAFT: &str =
    "A ship's core seals a filed mind; an unfiled draft cannot be sealed aboard a ship.";
/// A lapsed or revoked licence: `{licence}`, `{state}`.
pub const MIND_LAPSED: &str = "Licence {licence} is {state}; nothing is sealed aboard.";
/// The preview: `{licence}`, `{name}`.
pub const MIND_PREVIEW: &str = "Seal licence {licence} aboard the {name}: its mind takes her helm and sensors while her crew has no link, and holds a seat of the licence until she next docks at a port. Add go to seal it.";
/// Nothing to unseal: `{name}`.
pub const MIND_NOT_SEALED: &str = "The {name} has no mind sealed aboard to unseal.";
/// Another's seal: `{name}`.
pub const MIND_NOT_YOURS: &str =
    "The mind aboard the {name} was sealed by someone else; they unseal it.";
/// The claim has not opened yet: `{name}`.
pub const MIND_NOT_YET: &str =
    "The Registry has not admitted the {name}'s mind claim yet; try again in a moment.";
/// The unseal's preview: `{name}`.
pub const MIND_OFF_PREVIEW: &str = "Unseal the mind aboard the {name}: its claim closes and the licence's seat is free. Add go to unseal it.";
/// The tick's word once the claim is open: `{mind}`, `{name}`, `{class}`.
pub const MIND_SEALED: &str =
    "The Registry admits the claim: {mind} is sealed aboard the {name} at Class {class}.";
/// The tick's word once the claim is refused: `{name}`.
pub const MIND_REFUSED: &str =
    "The Registry would not admit her mind's claim: nothing is sealed aboard the {name}.";
/// The tick's word once the claim is closed: `{name}`.
pub const MIND_UNSEALED: &str = "The mind aboard the {name} is unsealed and its claim is closed.";
/// The tick's word when a seal no longer fits: `{name}`, `{why}`.
pub const MIND_VOIDED: &str =
    "The mind aboard the {name} no longer fits her ({why}); its claim is being closed.";
/// A refit, a strip, a fitting or a scrapping asked of a hull with a mind sealed aboard:
/// `{name}`. The unit's owner holds a hull under a claim as she is (`mush_tactics`'
/// `ic_custody`), so the yard says why before it tries.
pub const MIND_HOLDS_HER: &str = "A mind is sealed aboard the {name}, and she is held as she is until it is unsealed: +ship/mind off, at her berth.";
/// The verb is not wired on this roster.
pub const MIND_UNWIRED: &str = "Minds cannot be sealed aboard a ship here.";

/// Submissions an Open is given before the arm gives the claim up (the Roll refused it on
/// each: a seat already held, a holding gone, a hull changed).
pub const MAX_OPEN_TRIES: u8 = 5;

/// **The four acts on a hull's mind claim**, each an `op` of the ship's epoch action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MindOp {
    /// `+ship/mind filed:<n> go`: file the cursor.
    Seal,
    /// `+ship/mind off go`: close the claim at her berth.
    Unseal,
    /// The arm's: close a claim whose seal no longer fits the hull.
    Void,
    /// The arm's: give up a claim the Roll would not open.
    Refuse,
}

impl MindOp {
    /// Every op.
    pub(crate) const ALL: [MindOp; 4] =
        [MindOp::Seal, MindOp::Unseal, MindOp::Void, MindOp::Refuse];

    /// The word.
    pub(crate) fn word(self) -> &'static str {
        match self {
            MindOp::Seal => "mind",
            MindOp::Unseal => "unmind",
            MindOp::Void => "void",
            MindOp::Refuse => "refuse",
        }
    }

    /// The op a word names.
    pub(crate) fn parse(word: &str) -> Option<MindOp> {
        match word {
            "mind" => Some(MindOp::Seal),
            "unmind" => Some(MindOp::Unseal),
            "void" => Some(MindOp::Void),
            "refuse" => Some(MindOp::Refuse),
            _ => None,
        }
    }
}

/// **What an actor may do to one hull's mind claim now**, as the epoch action's offer fixes
/// it. The prepare reads every fact again from the world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MindOffer {
    /// The op words offered.
    pub(crate) ops: Vec<SmolStr>,
}

/// **Why a seal no longer fits its hull.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unfit {
    /// The hull has no unit, or its unit carries no core.
    NoCore,
    /// The hull's core is not of the class, or not under the governor, the seal was taken over.
    Core,
    /// The character who sealed it no longer has authority over the hull.
    Holder,
}

impl Unfit {
    /// The clause [`MIND_VOIDED`] is filled with.
    pub(crate) fn clause(self) -> &'static str {
        match self {
            Unfit::NoCore => "she carries no core",
            Unfit::Core => "her core has changed",
            Unfit::Holder => "she is no longer yours to fly it on",
        }
    }
}

/// **Whether `seal`, taken for `dispatcher`, still fits `hull`** (the module note): the hull
/// carries a core of the class and governor the seal was taken over, and the dispatcher still
/// has authority over her by the Roll's rule.
///
/// # Errors
///
/// The first way it does not.
pub(crate) fn fits(
    w: &World,
    hull: ObjId,
    seal: &MindSeal,
    dispatcher: ObjId,
) -> Result<(), Unfit> {
    let core = mush_tactics::module::unit_of(w, hull)
        .and_then(crate::arena::core::read_core)
        .ok_or(Unfit::NoCore)?;
    if seal.core.is_none() || core.class != seal.core_class || core.governed != seal.governed {
        return Err(Unfit::Core);
    }
    use mush_roll::commerce::ic_claims::{HullRefusal, hull_authority};
    match hull_authority(w, hull, dispatcher) {
        // An economy that cannot be read is not a change of holder.
        Ok(_) | Err(HullRefusal::Unavailable) => Ok(()),
        Err(HullRefusal::Untitled | HullRefusal::Anothers { .. }) => Err(Unfit::Holder),
    }
}

/// **A mind sealed aboard a hull, as the space seat reads it**, borrowed from the claim book.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sealed<'w> {
    /// The claim's open token.
    pub(crate) open: CommitToken,
    /// Who sealed it: the claim's dispatcher, and the seal's principal.
    pub(crate) dispatcher: ObjId,
    /// The claim's own seal.
    pub(crate) seal: &'w MindSeal,
    /// The source the claim retains.
    pub(crate) source: &'w str,
}

/// **The mind sealed aboard `hull`, if one still stands** (the module note): the claim book's
/// active claim on the hull, its seal and its retained source, and only while the seal still
/// fits her ([`fits`]). The one reader the space seat has.
#[must_use]
pub(crate) fn sealed(w: &World, hull: ObjId) -> Option<Sealed<'_>> {
    let (open, dispatcher, seal, source) = claim_execution(w, hull)?;
    fits(w, hull, seal, dispatcher).ok()?;
    Some(Sealed {
        open,
        dispatcher,
        seal,
        source,
    })
}

/// **What is sealed aboard a hull, as a reader sees it** (a story; staff): the claim, who
/// sealed it, the mind and its licence, the class it thinks at and the core it was taken
/// over, its channels and how many rows its rule program binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aboard {
    /// The claim's open token.
    pub claim: CommitToken,
    /// Who sealed it: the claim's dispatcher.
    pub dispatcher: ObjId,
    /// The seal's principal, when it is a character.
    pub principal: Option<ObjId>,
    /// The mind's name.
    pub mind: SmolStr,
    /// Its licence, as its seal's origin labels it.
    pub licence: SmolStr,
    /// The class it thinks at.
    pub class: crate::arena::core::CoreClass,
    /// The class of the core it was sealed over.
    pub core_class: crate::arena::core::CoreClass,
    /// Whether that core is governed.
    pub governed: bool,
    /// The channels the seal carries.
    pub channels: Vec<crate::arena::state::Channel>,
    /// The rows its opening profile's rule program binds.
    pub rules: usize,
    /// The SHA-256 of its source.
    pub source_sha256: [u8; 32],
    /// The seal's own digest.
    pub seal_digest: [u8; 32],
}

/// **What is sealed aboard `hull`** ([`Aboard`]), if a mind still stands there ([`sealed`]).
#[must_use]
pub fn aboard(w: &World, hull: ObjId) -> Option<Aboard> {
    let sealed = sealed(w, hull)?;
    let seal = sealed.seal;
    Some(Aboard {
        claim: sealed.open,
        dispatcher: sealed.dispatcher,
        principal: match &seal.principal {
            Some(crate::deliberation::order::CharacterRef::Character { id }) => Some(*id),
            _ => None,
        },
        mind: seal.mind.clone(),
        licence: seal.requested.origin.licence_label().into(),
        class: seal.class,
        core_class: seal.core_class,
        governed: seal.governed,
        channels: seal.channels.clone(),
        rules: seal
            .cards
            .programs
            .first()
            .map_or(0, |program| program.rows.len()),
        source_sha256: seal.source_sha256,
        seal_digest: seal.digest(),
    })
}

/// **The mind sealed aboard `hull`, made ready to think on `w`'s host**, if one stands
/// ([`sealed`]): the space seat's first source (`crate::space::backup`).
#[must_use]
pub(crate) fn thinker(w: &World, hull: ObjId) -> Option<Arc<crate::space::thinks::SealedMind>> {
    #[cfg(test)]
    if let Some(mind) = test_support::aboard(hull) {
        return Some(mind);
    }
    let sealed = sealed(w, hull)?;
    let host = crate::space::thinks::host_of(w)?;
    let classes = w
        .module_state::<crate::store::TacsimStore>()
        .map_or(crate::arena::core::ClassTable::STOCK, |store| {
            store.cfg.minds.classes
        });
    host.sealed_mind(hull, sealed.seal, sealed.source, &classes)
}

/// **Whether a mind claim holds `hull` as she is** (doc 44 unit MW.13): the claim book holds
/// an active claim on her, or her cursor has asked for one that is on its way. While it does
/// the unit's owner refuses her every change of fit (a refit, a component's fitting or
/// stripping, a core's, a scrapping), so the routes a player reaches say so first
/// ([`MIND_HOLDS_HER`]).
#[must_use]
pub(crate) fn holds(ctx: &Ctx<'_>, hull: ObjId) -> bool {
    if claim_of(ctx.w, hull) != ClaimRead::None {
        return true;
    }
    matches!(
        crate::port::epoch::record_at(ctx, hull),
        Ok(Some(record)) if matches!(record.claim.as_deref(), Some(ClaimCursor::Opening { .. }))
    )
}

/// Every ship hull the world holds: the programme ports' rows, the yards' and the owned-hull
/// register's ships.
fn ship_hulls(w: &World) -> BTreeSet<ObjId> {
    let mut hulls: BTreeSet<ObjId> = crate::port::coop::programme_ports(w)
        .into_iter()
        .flat_map(|port| port.map.hulls.iter().map(|row| ObjId(row.hull)))
        .collect();
    hulls.extend(
        crate::port::yard::yards(w)
            .into_iter()
            .flat_map(|yard| yard.row.hulls.iter().map(|row| ObjId(row.hull))),
    );
    hulls.extend(crate::port::owned::ships(w));
    hulls
}

/// The hangar port's host room `hull` is docked at, if she is.
fn berth_host(w: &World, hull: ObjId) -> Option<ObjId> {
    berth_of(w, hull)
        .hangar()
        .and_then(|(map, _)| map.host)
        .map(ObjId)
}

// ---- the offer and the prepare -----------------------------------------------------------------

/// **The hulls whose mind claim `actor` may act on now**, for the epoch action's offer: the
/// hull at whose helm they stand (seal, unseal), and every hull whose cursor names them
/// (unseal, and the arm's `void` and `refuse`).
pub(crate) fn offers(
    engine: &mush_core::Engine,
    provider: &EpochProvider,
    actor: ObjId,
) -> BTreeMap<String, MindOffer> {
    let w: &World = &engine.world;
    let mut offers: BTreeMap<String, MindOffer> = BTreeMap::new();
    let helm = helm_of(w, actor).map(|helm| helm.hull);
    for hull in ship_hulls(w) {
        let record = engine
            .action_state()
            .get(&provider.records, &EpochKey(hull))
            .ok()
            .flatten();
        let cursor = record.and_then(|record| record.claim.as_deref());
        let mut ops: Vec<SmolStr> = Vec::new();
        let mine = matches!(
            cursor,
            Some(ClaimCursor::Opening { operator, .. }) if *operator == actor
        );
        if helm == Some(hull) && !cursor.is_some_and(ClaimCursor::is_live) {
            ops.push(MindOp::Seal.word().into());
        }
        if mine {
            ops.extend([
                SmolStr::new_static(MindOp::Unseal.word()),
                SmolStr::new_static(MindOp::Void.word()),
                SmolStr::new_static(MindOp::Refuse.word()),
            ]);
        }
        if !ops.is_empty() {
            offers.insert(hull.0.to_string(), MindOffer { ops });
        }
    }
    offers
}

fn refuse(line: impl Into<String>) -> ActionError {
    ActionError::new(ActionErrorCode::ProviderRejected, false, line.into()).unwrap_or_else(|_| {
        port_error(
            ActionErrorCode::ProviderRejected,
            "the mind cannot be sealed aboard",
        )
    })
}

fn conflict(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::Conflict, message)
}

/// The `mind` and `org` parameters of a seal.
fn mind_param(
    parameters: &serde_json::Map<String, serde_json::Value>,
) -> Result<(MindWord, Option<mush_org::OrgId>), ActionError> {
    let invalid = |message: &'static str| port_error(ActionErrorCode::InvalidRequest, message);
    let org = match parameters.get("org") {
        None => None,
        Some(value) => Some(mush_org::OrgId(
            value
                .as_u64()
                .ok_or_else(|| invalid("an organisation is its number"))?,
        )),
    };
    parameters
        .get("mind")
        .and_then(serde_json::Value::as_str)
        .filter(|word| word.len() <= crate::surface::dispatch::MAX_MIND_WORD_BYTES)
        .and_then(MindWord::parse)
        .map(|word| (word, org))
        .ok_or_else(|| invalid("a mind is filed:<n> or a draft's name"))
}

/// **The mind ops' preparation** (the module note), called by the epoch action's prepare for
/// an op that is not an end of the epoch. Every fact is read from the world in hand: off the
/// composed path nothing is sealed or closed.
///
/// # Errors
///
/// The verb's refusals, as the sentence the operator reads; a conflict when the world has
/// moved under an op the arm submitted.
#[allow(clippy::too_many_lines)] // one ordered refusal list for each of four small ops
pub(crate) fn prepare(
    provider: &EpochProvider,
    invocation: &ActionInvocation,
    txn: &mut ActionTxn<'_>,
    context: &mut PrepareCtx<'_>,
    hull: ObjId,
    op: MindOp,
) -> Result<AuthoritativeActionOutput, ActionError> {
    let actor = invocation.actor();
    let parameters = invocation.parameters().as_object().ok_or_else(|| {
        port_error(
            ActionErrorCode::InvalidRequest,
            "a ship's mind names a hull and an act",
        )
    })?;
    let offered: MindOffer = parameters
        .get("minds")
        .and_then(|minds| minds.get(hull.0.to_string()))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| {
            port_error(
                ActionErrorCode::Unauthorized,
                "that ship's mind is not yours to seal",
            )
        })?;
    if !offered.ops.iter().any(|word| word == op.word()) {
        return Err(port_error(
            ActionErrorCode::Unauthorized,
            "that ship's mind is not yours to seal",
        ));
    }
    let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
    let key = EpochKey(hull);
    let prior = txn
        .get(&provider.records, &key)
        .map_err(|_| integrity("a ship's epoch records are unavailable"))?
        .cloned();
    let existed = prior.is_some();
    let mut record = prior.unwrap_or_else(|| ShipEpochRecord::fresh(hull));
    record
        .check()
        .map_err(|_| integrity("a ship's epoch record cannot move until it is repaired"))?;
    let unavailable = || refuse(crate::surface::dispatch::MIND_UNAVAILABLE);
    match op {
        MindOp::Seal => {
            let (name, now) = {
                let world = context.world().map_err(|_| unavailable())?;
                let name = name_of(world, hull);
                if helm_of(world, actor).map(|helm| helm.hull) != Some(hull) {
                    return Err(port_error(
                        ActionErrorCode::Unauthorized,
                        "that ship's mind is not yours to seal",
                    ));
                }
                if record.claim.as_deref().is_some_and(ClaimCursor::is_live) {
                    return Err(refuse(MIND_ALREADY.replace("{name}", &name)));
                }
                if berth_host(world, hull).is_none() {
                    return Err(refuse(MIND_BERTH.replace("{name}", &name)));
                }
                if record.open.is_some() {
                    return Err(refuse(MIND_PASSAGE.replace("{name}", &name)));
                }
                if mush_tactics::module::unit_of(world, hull)
                    .and_then(crate::arena::core::read_core)
                    .is_none()
                {
                    return Err(refuse(MIND_NO_CORE.replace("{name}", &name)));
                }
                (name, world.world_ic_now().at)
            };
            let (word, org) = mind_param(parameters)?;
            let dispatched = crate::surface::dispatch::dispatch(
                context,
                txn,
                (actor, hull, now),
                Some((&word, org)),
            )?;
            let (licence, seats) = match dispatched {
                Some(Dispatched::Claim { licence, seats, .. }) => (licence, seats),
                Some(Dispatched::Sealed(sealed)) if sealed.fell_to_doctrine() => {
                    let state = sealed
                        .facts
                        .licence_state
                        .map_or("lapsed", |state| state.as_str());
                    return Err(refuse(
                        MIND_LAPSED
                            .replace("{licence}", &sealed.facts.licence)
                            .replace("{state}", state),
                    ));
                }
                // A draft on an ungoverned core is sealed by the dispatch itself and opens no
                // claim; nothing here would carry such a seal, so a ship takes none.
                Some(Dispatched::Sealed(sealed)) if sealed.licence.is_none() => {
                    return Err(refuse(MIND_DRAFT));
                }
                Some(Dispatched::Sealed(_)) | None => return Err(unavailable()),
            };
            if let Some(seats) = seats {
                let held = context
                    .world()
                    .map_or(0, |world| active_under_licence(world, licence));
                if held >= usize::from(seats) {
                    return Err(refuse(
                        crate::surface::dispatch::MIND_SEATS_HELD
                            .replace("{licence}", &licence.to_string())
                            .replace("{held}", &held.to_string())
                            .replace("{seats}", &seats.to_string()),
                    ));
                }
            }
            let _ = name;
            record.claim = Some(Box::new(ClaimCursor::Opening {
                operator: actor,
                licence,
                org: org.map(|org| org.0),
                launch_ic: now,
                tries: 0,
            }));
        }
        MindOp::Unseal | MindOp::Void | MindOp::Refuse => {
            let Some(ClaimCursor::Opening { operator, .. }) = record.claim.as_deref() else {
                return Err(conflict("that ship has no mind claim standing"));
            };
            let operator = *operator;
            if operator != actor {
                return Err(port_error(
                    ActionErrorCode::Unauthorized,
                    "that ship's mind is not yours to unseal",
                ));
            }
            let world = context.world().map_err(|_| unavailable())?;
            let name = name_of(world, hull);
            let read = claim_of(world, hull);
            let berth = berth_host(world, hull);
            let base = berth.or_else(|| record.open.as_ref().map(|open| open.base));
            let now = world.world_ic_now().at;
            record.claim = Some(Box::new(match (op, read) {
                (MindOp::Refuse, ClaimRead::None) => ClaimCursor::Refused {
                    reason: "the Registry did not admit the claim".into(),
                    at_ic: now,
                },
                (MindOp::Refuse, ClaimRead::Active { .. }) => {
                    return Err(conflict("that ship's mind claim has opened"));
                }
                (MindOp::Unseal, _) if berth.is_none() || record.open.is_some() => {
                    return Err(refuse(MIND_BERTH.replace("{name}", &name)));
                }
                (MindOp::Unseal | MindOp::Void, ClaimRead::None) => {
                    return Err(match op {
                        MindOp::Unseal => refuse(MIND_NOT_YET.replace("{name}", &name)),
                        _ => conflict("that ship's mind claim has not opened"),
                    });
                }
                (MindOp::Unseal | MindOp::Void, ClaimRead::Active { open, dispatcher }) => {
                    if dispatcher != operator {
                        return Err(conflict("that ship's mind claim is another's"));
                    }
                    if op == MindOp::Void && standing(world, hull, &record, open).is_ok() {
                        return Err(conflict("that ship's mind still stands"));
                    }
                    let Some(base) = base else {
                        return Err(conflict("that ship's mind claim has no base to close to"));
                    };
                    ClaimCursor::Closing {
                        open,
                        kind: ClaimCloseKind::PoweredDownInBase { base },
                        operator,
                    }
                }
                (MindOp::Seal, _) => return Err(conflict("that ship's mind is being sealed")),
            }));
        }
    }
    record
        .check()
        .map_err(|_| integrity("a ship's epoch record is out of bounds"))?;
    let staged = if existed {
        txn.replace(&provider.records, &key, record)
    } else {
        txn.insert(&provider.records, &key, record)
    };
    staged.map_err(|_| integrity("a ship's epoch record did not stage"))?;
    provider.output(invocation, (hull, op.word(), false))
}

/// **Why a standing claim's seal should stand no longer**, or `Ok`: the seal does not fit the
/// hull ([`fits`]), or the hull's epoch closed after the mind was sealed and the close did not
/// move the cursor (a close prepared with no world in hand).
fn standing(
    w: &World,
    hull: ObjId,
    record: &ShipEpochRecord,
    open: CommitToken,
) -> Result<(), &'static str> {
    let Some(ClaimCursor::Opening {
        operator,
        launch_ic,
        ..
    }) = record.claim.as_deref()
    else {
        return Ok(());
    };
    let Some(seal) = claim_seal(w, open) else {
        return Ok(());
    };
    fits(w, hull, &seal, *operator).map_err(Unfit::clause)?;
    let docked_since = record.open.is_none()
        && record.last.as_ref().is_some_and(|last| {
            last.op == crate::port::epoch::EpochOp::Close && last.at_ic > *launch_ic
        });
    if docked_since {
        return Err("her passage has ended");
    }
    Ok(())
}

/// **The epoch's close moves the cursor** (the module note), in the bundle that closes the
/// epoch: a claim that stands closes by the epoch's own kind; one the Roll had not yet
/// admitted is given up. With no world in hand nothing moves, and the arm closes the claim on
/// its next fire ([`standing`]).
pub(crate) fn close_with_epoch(
    world: Option<&World>,
    hull: ObjId,
    record: &mut ShipEpochRecord,
    kind: ClaimCloseKind,
    at_ic: i64,
) {
    let Some(ClaimCursor::Opening { operator, .. }) = record.claim.as_deref() else {
        return;
    };
    let operator = *operator;
    let Some(world) = world else {
        return;
    };
    record.claim = Some(Box::new(match claim_of(world, hull) {
        ClaimRead::Active { open, dispatcher } if dispatcher == operator => ClaimCursor::Closing {
            open,
            kind,
            operator,
        },
        ClaimRead::Active { .. } | ClaimRead::None => ClaimCursor::Refused {
            reason: "her passage ended before the Registry admitted the claim".into(),
            at_ic,
        },
    }));
}

// ---- the arm -----------------------------------------------------------------------------------

/// One transition the arm is trying this boot: what, and how often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MindTry {
    /// The cursor's own mark: an Opening's `launch_ic`, a Closing's open token's low word.
    mark: i64,
    submissions: u8,
    last_fire: u64,
}

/// **The arm's slot** (RAM; a reboot owes every transition again from the record): each
/// hull's ticket in flight, with the claim it is for, and its tries.
#[derive(Clone, Default)]
pub(crate) struct MindArm {
    /// An Open in flight, with the instant (`launch_ic`) its seal was asked.
    opens: BTreeMap<ObjId, (i64, Arc<IcHostTicket>)>,
    /// A Close in flight, with the open token it closes.
    closes: BTreeMap<ObjId, (CommitToken, Arc<IcHostTicket>)>,
    /// Submissions made this boot.
    tries: BTreeMap<ObjId, MindTry>,
}

impl std::fmt::Debug for MindArm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MindArm")
            .field("opens", &self.opens.keys().collect::<Vec<_>>())
            .field("closes", &self.closes.keys().collect::<Vec<_>>())
            .field("tries", &self.tries)
            .finish()
    }
}

/// What a hull's cursor owes on this fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owes {
    /// Submit the claim's Open.
    Open,
    /// The Open has landed: acknowledge it and tell the operator.
    Seen,
    /// The seal stands no longer: close its claim.
    Void(&'static str),
    /// The Roll would not open it: give it up.
    GiveUp,
    /// Submit the claim's Close.
    Close,
    /// The Close has landed: acknowledge it and tell the operator.
    Closed,
}

/// What `record`'s cursor owes now, and as whom; and whether the hull's launch waits for it.
fn owes(
    w: &World,
    state: &PortTickState,
    hull: ObjId,
    record: &ShipEpochRecord,
) -> (Option<(ObjId, Owes)>, bool) {
    let arm = &state.mind_arm;
    match record.claim.as_deref() {
        Some(ClaimCursor::Opening {
            operator,
            launch_ic,
            ..
        }) => match claim_of(w, hull) {
            ClaimRead::Active { open, dispatcher } if dispatcher == *operator => {
                let owed = match standing(w, hull, record, open) {
                    Err(why) => Some(Owes::Void(why)),
                    Ok(()) if arm.opens.contains_key(&hull) => Some(Owes::Seen),
                    Ok(()) => None,
                };
                (owed.map(|owed| (*operator, owed)), false)
            }
            // A claim on this hull that another character dispatched: nothing of this cursor's.
            ClaimRead::Active { .. } => (None, false),
            ClaimRead::None => {
                let tried = arm
                    .tries
                    .get(&hull)
                    .filter(|tried| tried.mark == *launch_ic);
                let owed = match tried {
                    Some(tried) if tried.submissions >= MAX_OPEN_TRIES => Some(Owes::GiveUp),
                    Some(tried) if state.fires <= tried.last_fire => None,
                    Some(_) | None => Some(Owes::Open),
                };
                (owed.map(|owed| (*operator, owed)), true)
            }
        },
        Some(ClaimCursor::Closing { open, operator, .. }) => {
            let owed = if claim_closed(w, *open).is_some() {
                arm.closes.contains_key(&hull).then_some(Owes::Closed)
            } else {
                Some(Owes::Close)
            };
            (owed.map(|owed| (*operator, owed)), false)
        }
        _ => (None, false),
    }
}

/// **The hulls whose mind claim owes a transition this fire** (the module note): `(offered,
/// withheld)`, as the epoch arm's own pair. A hull whose claim has not opened yet is withheld
/// from the launch arm, so the claim opens over the hull at her berth (doc 23 D-23-21: the
/// claim first).
pub(crate) fn agents(
    ctx: &Ctx<'_>,
    state: &PortTickState,
    skip: &BTreeSet<ObjId>,
) -> (Vec<(ObjId, ObjId)>, BTreeSet<ObjId>) {
    let mut offered = Vec::new();
    let mut withheld = BTreeSet::new();
    if state.epoch_action.is_none() || crate::surface::mind_claim::host_of(ctx.w).is_none() {
        return (offered, withheld);
    }
    for hull in ship_hulls(ctx.w) {
        if skip.contains(&hull) {
            continue;
        }
        let Ok(Some(record)) = crate::port::epoch::record_at(ctx, hull) else {
            continue;
        };
        let (owed, wait) = owes(ctx.w, state, hull, &record);
        if wait {
            withheld.insert(hull);
        }
        if let Some((operator, _)) = owed {
            offered.push((hull, operator));
        }
    }
    (offered, withheld)
}

/// Count one submission of `mark` for `hull` on this fire.
fn tried(state: &mut PortTickState, hull: ObjId, mark: i64) {
    let submissions = state
        .mind_arm
        .tries
        .get(&hull)
        .filter(|tried| tried.mark == mark)
        .map_or(0, |tried| tried.submissions)
        .saturating_add(1);
    state.mind_arm.tries.insert(
        hull,
        MindTry {
            mark,
            submissions,
            last_fire: state.fires,
        },
    );
}

fn close_of(kind: ClaimCloseKind) -> SurfaceClose {
    match kind {
        ClaimCloseKind::ReturnedToBase { base } => SurfaceClose::ReturnedToBase(base),
        ClaimCloseKind::PoweredDownInBase { base } => SurfaceClose::PoweredDownInBase(base),
        ClaimCloseKind::Destroyed => SurfaceClose::Destroyed,
        ClaimCloseKind::RecoveredToBase { base } => SurfaceClose::RecoveredToBase(base),
    }
}

/// Submit one mind op of the epoch action as `operator`.
fn submit_op(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    action: &mush_core::DefinitionRef,
    (hull, operator): (ObjId, ObjId),
    op: MindOp,
    mark: i64,
) -> Step {
    let Ok(key) = IdempotencyKeyText::parse_canonical(format!(
        "{EPOCH_KEY_PREFIX}{}/{mark}/{}/tick",
        hull.0,
        op.word()
    )) else {
        return Step::Refused;
    };
    let parameters = json!({ "hull": hull.0.to_string(), "op": op.word() });
    match fire.as_principal(ctx, hull, operator, |ctx| {
        ctx.submit_authoritative(
            action.clone(),
            PerceivedRef::SelfActor,
            Vec::new(),
            parameters,
            key,
        );
    }) {
        Ok(()) => Step::Submitted,
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
    }
}

/// Submit one host ticket as `operator`.
fn submit_ticket(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    host: &crate::minds::ic_sortie::IcHost,
    ticket: &IcHostTicket,
    (hull, operator): (ObjId, ObjId),
) -> Step {
    match fire.as_principal(ctx, hull, operator, |ctx| host.submit_as_host(ctx, ticket)) {
        Ok(Ok(())) => Step::Submitted,
        Ok(Err(error)) => {
            tracing::warn!(hull = hull.0, %error, "tacsim: a ship's mind claim was not submitted; it waits");
            Step::Refused
        }
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
    }
}

/// **One visit to a hull whose mind claim owes a transition**, as its operator (the module
/// note; `crate::surface::mind_claim` is the rig's arm, and the transitions are the same).
#[allow(clippy::too_many_lines)] // six small transitions, one a branch
pub(crate) fn step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    hull: ObjId,
    principal: ObjId,
) -> Step {
    let Some(action) = state.epoch_action.clone() else {
        return Step::Idle;
    };
    let Some(host) = crate::surface::mind_claim::host_of(ctx.w) else {
        return Step::Idle;
    };
    let Ok(Some(record)) = crate::port::epoch::record_at(ctx, hull) else {
        return Step::Idle;
    };
    let (Some((operator, owed)), _) = owes(ctx.w, state, hull, &record) else {
        return Step::Idle;
    };
    if operator != principal {
        return Step::Idle;
    }
    let name = name_of(ctx.w, hull);
    match (owed, record.claim.as_deref()) {
        (
            Owes::Open,
            Some(ClaimCursor::Opening {
                licence, launch_ic, ..
            }),
        ) => {
            let kept = state
                .mind_arm
                .opens
                .get(&hull)
                .filter(|(at, _)| at == launch_ic)
                .map(|(_, ticket)| Arc::clone(ticket));
            let ticket = match kept {
                Some(ticket) => ticket,
                None => {
                    if let Some((_, stale)) = state.mind_arm.opens.remove(&hull) {
                        let _ = host.acknowledge(ctx.w, &stale);
                    }
                    let minted = crate::minds::ic_sortie::surface::surface_start(
                        ctx.w, operator, hull, *launch_ic,
                    )
                    .and_then(|permit| {
                        let licence =
                            mush_roll::commerce::LicenceId::new(*licence).map_err(|_| {
                                mush_core::participant::ParticipantError::refused(
                                    "ship mind claim licence",
                                )
                            })?;
                        host.enqueue_start(&permit, licence, vec![])
                    });
                    match minted {
                        Ok(ticket) => {
                            let ticket = Arc::new(ticket);
                            state
                                .mind_arm
                                .opens
                                .insert(hull, (*launch_ic, Arc::clone(&ticket)));
                            ticket
                        }
                        Err(error) => {
                            tracing::warn!(hull = hull.0, %error, "tacsim: a ship's mind claim could not be enqueued; it waits");
                            tried(state, hull, *launch_ic);
                            return Step::Deferred;
                        }
                    }
                }
            };
            let step = submit_ticket(ctx, fire, &host, &ticket, (hull, operator));
            if matches!(step, Step::Submitted | Step::Refused) {
                tried(state, hull, *launch_ic);
            }
            step
        }
        (Owes::Seen, _) => {
            if let Some((_, ticket)) = state.mind_arm.opens.remove(&hull) {
                let _ = host.acknowledge(ctx.w, &ticket);
            }
            state.mind_arm.tries.remove(&hull);
            if let Some(sealed) = sealed(ctx.w, hull) {
                let line = MIND_SEALED
                    .replace("{mind}", sealed.seal.mind.as_str())
                    .replace("{name}", &name)
                    .replace("{class}", sealed.seal.class.as_str());
                crate::port_tick::notify_budget(ctx, operator, &line, NoticeLevel::Trip);
            }
            Step::Worked
        }
        (Owes::Void(why), Some(ClaimCursor::Opening { launch_ic, .. })) => {
            let step = submit_op(
                ctx,
                fire,
                &action,
                (hull, operator),
                MindOp::Void,
                *launch_ic,
            );
            if step == Step::Submitted {
                let line = MIND_VOIDED.replace("{name}", &name).replace("{why}", why);
                crate::port_tick::notify_budget(ctx, operator, &line, NoticeLevel::Trip);
            }
            step
        }
        (Owes::GiveUp, Some(ClaimCursor::Opening { launch_ic, .. })) => {
            let step = submit_op(
                ctx,
                fire,
                &action,
                (hull, operator),
                MindOp::Refuse,
                *launch_ic,
            );
            if step == Step::Submitted {
                if let Some((_, ticket)) = state.mind_arm.opens.remove(&hull) {
                    let _ = host.acknowledge(ctx.w, &ticket);
                }
                state.mind_arm.tries.remove(&hull);
                let line = MIND_REFUSED.replace("{name}", &name);
                crate::port_tick::notify_budget(ctx, operator, &line, NoticeLevel::Trip);
            }
            step
        }
        (Owes::Close, Some(ClaimCursor::Closing { open, kind, .. })) => {
            let kept = state
                .mind_arm
                .closes
                .get(&hull)
                .filter(|(of, _)| of == open)
                .map(|(_, ticket)| Arc::clone(ticket));
            let ticket = match kept {
                Some(ticket) => ticket,
                None => {
                    if let Some((_, stale)) = state.mind_arm.closes.remove(&hull) {
                        let _ = host.acknowledge(ctx.w, &stale);
                    }
                    let minted = crate::minds::ic_sortie::surface::surface_close(
                        ctx.w,
                        operator,
                        *open,
                        close_of(*kind),
                    )
                    .and_then(|event| host.enqueue_close(&event));
                    match minted {
                        Ok(ticket) => {
                            let ticket = Arc::new(ticket);
                            state
                                .mind_arm
                                .closes
                                .insert(hull, (*open, Arc::clone(&ticket)));
                            ticket
                        }
                        Err(error) => {
                            tracing::warn!(hull = hull.0, %error, "tacsim: a ship's mind claim's close could not be enqueued; it waits");
                            return Step::Deferred;
                        }
                    }
                }
            };
            submit_ticket(ctx, fire, &host, &ticket, (hull, operator))
        }
        (Owes::Closed, _) => {
            if let Some((_, ticket)) = state.mind_arm.closes.remove(&hull) {
                let _ = host.acknowledge(ctx.w, &ticket);
            }
            let line = MIND_UNSEALED.replace("{name}", &name);
            crate::port_tick::notify_budget(ctx, operator, &line, NoticeLevel::Trip);
            Step::Worked
        }
        _ => Step::Idle,
    }
}

// ---- the verb ----------------------------------------------------------------------------------

/// What `+ship/mind` reads of a hull's claim.
fn status_line(w: &World, hull: ObjId, record: Option<&ShipEpochRecord>) -> String {
    let name = name_of(w, hull);
    match record.and_then(|record| record.claim.as_deref()) {
        Some(ClaimCursor::Opening { licence, .. }) => match claim_of(w, hull) {
            ClaimRead::Active { open, dispatcher } => match claim_seal(w, open) {
                Some(seal) if fits(w, hull, &seal, dispatcher).is_ok() => MIND_ABOARD
                    .replace("{mind}", seal.mind.as_str())
                    .replace("{name}", &name)
                    .replace("{licence}", &licence.to_string())
                    .replace("{class}", seal.class.as_str()),
                Some(seal) => MIND_UNFIT
                    .replace("{mind}", seal.mind.as_str())
                    .replace("{name}", &name),
                None => MIND_NONE.replace("{name}", &name),
            },
            ClaimRead::None => MIND_ADMITTING
                .replace("{licence}", &licence.to_string())
                .replace("{name}", &name),
        },
        Some(ClaimCursor::Closing { open, .. }) if claim_closed(w, *open).is_none() => {
            MIND_CLOSING.replace("{name}", &name)
        }
        Some(ClaimCursor::Refused { .. }) => format!(
            "{} {}",
            MIND_WAS_REFUSED.replace("{name}", &name),
            MIND_NONE.replace("{name}", &name)
        ),
        _ => MIND_NONE.replace("{name}", &name),
    }
}

/// **`+ship/mind`** (the module note): the read, the seal and the unseal.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor; on a refusal nothing is written.
pub(crate) fn mind_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Some(helm) = helm_of(ctx.w, actor) else {
        return Err(crate::port::ship::no_charter(ctx.w, actor));
    };
    let hull = helm.hull;
    let name = name_of(ctx.w, hull);
    let record =
        crate::port::epoch::record_at(ctx, hull).map_err(|()| CmdError::msg(MIND_UNWIRED))?;
    let (raw, go) = crate::surface::retrieval::split_go(ctx.args.raw());
    if raw.is_empty() && !go {
        let line = status_line(ctx.w, hull, record.as_ref());
        say(ctx, line);
        return Ok(());
    }
    // Where the claim path is not composed nothing would carry a seal; said after the
    // refusals a read of the hull settles, so a coreless hull is told she has no core.
    let action = ctx
        .w
        .module_state::<PortTickState>()
        .and_then(|state| state.epoch_action.clone())
        .filter(|_| crate::minds::ic_sortie::claim_path_composed(ctx.w));
    let cursor = record.as_ref().and_then(|record| record.claim.as_deref());
    let live = cursor.is_some_and(ClaimCursor::is_live);
    let now = ctx.w.world_ic_now().at;
    let attempt = record.as_ref().map_or(0, |record| record.attempts);
    let key = |op: MindOp| {
        IdempotencyKeyText::parse_canonical(format!(
            "{EPOCH_KEY_PREFIX}{}/{now}/{}/{attempt}",
            hull.0,
            op.word()
        ))
        .map_err(|_| CmdError::msg(MIND_UNWIRED))
    };
    if raw.eq_ignore_ascii_case("off") {
        match cursor {
            Some(ClaimCursor::Opening { operator, .. }) if *operator != actor => {
                return Err(CmdError::msg(MIND_NOT_YOURS.replace("{name}", &name)));
            }
            Some(ClaimCursor::Opening { .. }) => {}
            _ => return Err(CmdError::msg(MIND_NOT_SEALED.replace("{name}", &name))),
        }
        if berth_host(ctx.w, hull).is_none()
            || record.as_ref().is_some_and(|record| record.open.is_some())
        {
            return Err(CmdError::msg(MIND_BERTH.replace("{name}", &name)));
        }
        if claim_of(ctx.w, hull) == ClaimRead::None {
            return Err(CmdError::msg(MIND_NOT_YET.replace("{name}", &name)));
        }
        if !go {
            say(ctx, MIND_OFF_PREVIEW.replace("{name}", &name));
            return Ok(());
        }
        let action = action.ok_or_else(|| CmdError::msg(MIND_UNWIRED))?;
        let key = key(MindOp::Unseal)?;
        ctx.submit_authoritative(
            action,
            PerceivedRef::SelfActor,
            Vec::new(),
            json!({ "hull": hull.0.to_string(), "op": MindOp::Unseal.word() }),
            key,
        );
        return Ok(());
    }
    // The seal: `<word> [org=<organisation>]`.
    let mut word: Option<&str> = None;
    let mut org: Option<&str> = None;
    for token in raw.split_whitespace() {
        match token.split_once('=') {
            Some((key, value)) if key.eq_ignore_ascii_case("org") && org.is_none() => {
                org = Some(value);
            }
            None if word.is_none() => word = Some(token),
            _ => return Err(CmdError::msg(MIND_USAGE)),
        }
    }
    let Some(word) = word else {
        return Err(CmdError::msg(MIND_USAGE));
    };
    if word.len() > crate::surface::dispatch::MAX_MIND_WORD_BYTES {
        return Err(CmdError::msg(MIND_USAGE));
    }
    let Some(parsed) = MindWord::parse(word) else {
        return Err(CmdError::msg(MIND_USAGE));
    };
    let org = match org {
        None => None,
        Some(text) => {
            let refuse = || CmdError::msg(crate::surface::dispatch::MIND_NOT_FOR_ORG);
            let org = mush_economy::resolve_org_visible(ctx, text).ok_or_else(refuse)?;
            mush_roll::commerce::org_holder::acts_for(ctx.w, org, actor).map_err(|_| refuse())?;
            Some(org)
        }
    };
    // The refusals a read of the world settles, in the prepare's own order; the licence's are
    // the prepare's, where the Roll's holding reads are in hand.
    if live {
        return Err(CmdError::msg(MIND_ALREADY.replace("{name}", &name)));
    }
    if berth_host(ctx.w, hull).is_none() {
        return Err(CmdError::msg(MIND_BERTH.replace("{name}", &name)));
    }
    if record.as_ref().is_some_and(|record| record.open.is_some()) {
        return Err(CmdError::msg(MIND_PASSAGE.replace("{name}", &name)));
    }
    let core = mush_tactics::module::unit_of(ctx.w, hull).and_then(crate::arena::core::read_core);
    let Some(core) = core else {
        return Err(CmdError::msg(MIND_NO_CORE.replace("{name}", &name)));
    };
    let licence = match &parsed {
        MindWord::Filed(licence) => licence.get(),
        MindWord::Draft(_) if core.governed => {
            return Err(CmdError::msg(crate::surface::dispatch::MIND_GOVERNED_DRAFT));
        }
        MindWord::Draft(_) => return Err(CmdError::msg(MIND_DRAFT)),
    };
    let action = action.ok_or_else(|| CmdError::msg(MIND_UNWIRED))?;
    if let Some(line) = crate::surface::dispatch::untitled_refusal(ctx.w, actor, hull) {
        return Err(CmdError::msg(line));
    }
    if !go {
        say(
            ctx,
            MIND_PREVIEW
                .replace("{licence}", &licence.to_string())
                .replace("{name}", &name),
        );
        return Ok(());
    }
    let key = key(MindOp::Seal)?;
    let mut parameters = json!({
        "hull": hull.0.to_string(),
        "op": MindOp::Seal.word(),
        "mind": word,
    });
    if let (Some(org), Some(object)) = (org, parameters.as_object_mut()) {
        object.insert("org".into(), json!(org.0));
    }
    ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A unit test's way to put a sealed mind aboard a hull of a rig that composes no claim
    //! path: the mind [`super::thinker`] answers for that hull, on this thread, until the
    //! guard is dropped. A world's sealed mind is always the claim book's.

    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use mush_core::ObjId;

    use crate::space::thinks::SealedMind;

    thread_local! {
        static ABOARD: RefCell<BTreeMap<ObjId, Arc<SealedMind>>> = const { RefCell::new(BTreeMap::new()) };
    }

    /// The sealed minds put aboard on this thread, taken off again when it is dropped.
    #[derive(Debug)]
    pub(crate) struct Aboard(Vec<ObjId>);

    impl Drop for Aboard {
        fn drop(&mut self) {
            ABOARD.with(|aboard| {
                let mut aboard = aboard.borrow_mut();
                for hull in &self.0 {
                    aboard.remove(hull);
                }
            });
        }
    }

    /// Put each mind aboard its hull for as long as the answer lives.
    pub(crate) fn seal_aboard(minds: Vec<(ObjId, Arc<SealedMind>)>) -> Aboard {
        let hulls = minds.iter().map(|(hull, _)| *hull).collect();
        ABOARD.with(|aboard| aboard.borrow_mut().extend(minds));
        Aboard(hulls)
    }

    pub(super) fn aboard(hull: ObjId) -> Option<Arc<SealedMind>> {
        ABOARD.with(|aboard| aboard.borrow().get(&hull).map(Arc::clone))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The op words round-trip, and none is an end of the epoch.
    #[test]
    fn mw13_the_mind_ops_have_their_own_words() {
        for op in MindOp::ALL {
            assert_eq!(MindOp::parse(op.word()), Some(op));
        }
        assert_eq!(MindOp::parse("open"), None);
        assert_eq!(MindOp::parse("close"), None);
    }

    /// **The epoch's close moves a standing cursor and nothing else** (doc 44 unit MW.13):
    /// with no world in hand the cursor is left for the arm; a record with no cursor, or one
    /// already closing, is untouched.
    #[test]
    fn mw13_the_epochs_close_leaves_a_cursor_it_cannot_read_for_the_arm() {
        let hull = ObjId(5011);
        let kind = ClaimCloseKind::ReturnedToBase { base: ObjId(7) };
        let mut bare = ShipEpochRecord::fresh(hull);
        close_with_epoch(None, hull, &mut bare, kind, 900);
        assert_eq!(bare, ShipEpochRecord::fresh(hull));
        let opening = ClaimCursor::Opening {
            operator: ObjId(10),
            licence: 4,
            org: None,
            launch_ic: 100,
            tries: 0,
        };
        let mut standing = ShipEpochRecord::fresh(hull);
        standing.claim = Some(Box::new(opening.clone()));
        close_with_epoch(None, hull, &mut standing, kind, 900);
        assert_eq!(standing.claim.as_deref(), Some(&opening));
        // The cursor rides the record's wire and a record without one reads as it always did.
        let wire = serde_json::to_value(&standing).expect("encodes");
        assert_eq!(wire["claim"]["state"], "opening");
        assert_eq!(
            serde_json::from_value::<ShipEpochRecord>(wire).expect("decodes"),
            standing
        );
        assert_eq!(
            serde_json::to_value(ShipEpochRecord::fresh(hull)).expect("encodes"),
            json!({"hull": 5011, "attempts": 0})
        );
    }
}
