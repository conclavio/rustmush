//! **The mind sealed aboard** (doc 17h cut M8e; doc 23 cut S.4a, §6 "What the host does at
//! each end"; D-UM-45, D-UM-41, D-UM-42, D-23-5, D-23-8).
//!
//! A rig whose unit carries a core is sealed at its launch's dispatch with the mind the operator
//! names (`+tv/<n> launch mind=<word> [org=<org>] go`): a filed licence (`filed:<n>`) the
//! operator holds — or, with `org=`, one an organisation holds and the operator acts for (M8c
//! B3) — or, on an ungoverned core alone, an unfiled personal draft (`draft:<name>`). The seal
//! is taken in the start's preparation, where the Roll's holding reads are in hand
//! (`crate::match_::original_access::DispatchReads`); the act carries it to the finish, whose
//! open line prints it (`crate::surface::black_box::epoch_facts`) and whose bundle writes the
//! licence's seal mark (M8c's `roll_licence_seal_marks`).
//!
//! The mind is sealed and **dormant** (doc 17h Q-M8-1, S.4a): the rig has no station it could
//! act on, its excursion runs exactly as a stock rig's, and no IC claim opens — the claim book's
//! Open and Close and the pool seat land with the moving-anchor custody design (S.4b).
//!
//! The licence's state is the holding's (doc 17h cut M8a, `licence_state_of` over the paid
//! view's phase, revoked bit and selling bit): `active` or `grace` seal the mind; a **lapsed**
//! or **revoked** licence seals the ROM — Standard Doctrine on the Line card — with the line's
//! `licence_state` the licence's own word, and the operator alone is told at the launch's end; a
//! **pending** transfer (the licence offered for resale) refuses the seal by name.

use mush_core::{ActionError, ActionErrorCode, ActionTxn, ObjId, PrepareCtx};
use mush_economy::Holder;
use mush_roll::commerce::{LicenceId, TermPhase};
use serde::{Deserialize, Serialize};

use crate::arena::mind::{MindSeal, SortieId};
use crate::deliberation::order::CharacterRef;
use crate::match_::holdings::HeldMind;
use crate::minds::licence_state::{SealLicenceState, licence_state_of};
use crate::port_tick::port_error;
use crate::store::TacsimStore;
use crate::surface::black_box::SealFacts;

/// **What a launch's dispatch sealed** (doc 17h cut M8e E4), fixed at its start and carried
/// on its act ([`crate::surface::excursion::Act::seal`]) to the finish, which prints it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchSeal {
    /// The seal's facts, the licence's state and holder among them.
    pub facts: SealFacts,
    /// **The licence flown under**, the seal mark's key: a filed mind's, or a lapsed or
    /// revoked one's that fell to the ROM; absent for a draft. Skipped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<u64>,
}

impl DispatchSeal {
    /// **The licence whose seal-mark ring this flight marks** (M8c B5; fix round 1, J3): the
    /// licence a mind flew under; `None` for a draft and for a seal that fell to the ROM, which
    /// flew no mind under its licence and is no evidence about it.
    #[must_use]
    pub fn mark_licence(&self) -> Option<u64> {
        self.licence.filter(|_| !self.fell_to_doctrine())
    }

    /// **Whether the seal fell to the ROM for its licence's state** (a lapsed or revoked
    /// licence): what the operator alone is told at the launch's end.
    #[must_use]
    pub fn fell_to_doctrine(&self) -> bool {
        self.licence.is_some()
            && matches!(
                self.facts.licence_state,
                Some(SealLicenceState::Lapsed | SealLicenceState::Revoked)
            )
    }
}

/// The longest `mind=` word a start carries (a draft's name is a Roll mind name).
pub const MAX_MIND_WORD_BYTES: usize = 96;

/// **The pending refusal** (doc 17h cut M8e E4; fix round 1, J4: doc 17 D-UM-40's sentence, its
/// "sortie" in the lead's wording, owed to the owner for approval): a licence whose transfer is
/// funded and settling (a paid resale's `selling`, an owner charter's pending sale) is not
/// sealed. `{mind}` is the mind's name.
pub const MIND_PENDING: &str =
    "{mind} is changing hands; it is not sealed aboard until the transfer settles.";
/// **A licence the operator does not hold** (or an organisation's they do not act for), read
/// exactly like a missing one. `{licence}`.
pub const MIND_NO_LICENCE: &str = "You hold no licence {licence}.";
/// **A draft on a governed core** (D-UM-41): only an ungoverned core seals an unfiled draft.
pub const MIND_GOVERNED_DRAFT: &str =
    "A governed core seals no unfiled draft; name a filed licence.";
/// **An organisation the operator does not act for** (or may not see): the `+minds` family's
/// own sentence (M8c B3), re-exported.
pub use crate::minds::market_commands::words::NOT_FOR_ORG as MIND_NOT_FOR_ORG;
/// **No such draft, or not the operator's.** `{word}`.
pub const MIND_NO_DRAFT: &str = "You have no draft {word}.";
/// **The rig carries no core**: nothing but the ROM is sealed aboard.
pub const MIND_NO_CORE: &str = "That rig carries no core; only Standard Doctrine flies it.";
/// **The holdings are not composed here** (a world with no Roll holdings seam).
pub const MIND_UNAVAILABLE: &str = "Minds cannot be sealed aboard here.";

/// The `mind=` word: `filed:<n>` or a draft (`draft:<name>`, or a bare name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MindWord {
    /// A filed licence.
    Filed(LicenceId),
    /// An unfiled personal draft, by the name the operator saved it under.
    Draft(String),
}

impl MindWord {
    /// The word, or `None` for a `filed:` word with no licence number.
    pub(crate) fn parse(word: &str) -> Option<MindWord> {
        match crate::match_::original_access::filed_licence(word) {
            Some(licence) => licence.map(MindWord::Filed),
            None => {
                let name = word
                    .get(..6)
                    .filter(|head| head.eq_ignore_ascii_case("draft:"))
                    .and_then(|_| word.get(6..))
                    .unwrap_or(word)
                    .trim();
                (!name.is_empty()).then(|| MindWord::Draft(name.to_owned()))
            }
        }
    }
}

/// What the dispatch resolved, before the seal.
enum Resolved {
    /// The mind flies: the held executable, its licence's state, the licence and its holder.
    Held {
        held: Box<HeldMind>,
        state: SealLicenceState,
        licence: Option<LicenceId>,
        holder: Option<Holder>,
    },
    /// The licence's state falls the hull to the ROM.
    Rom {
        state: SealLicenceState,
        licence: LicenceId,
        holder: Holder,
    },
}

fn refuse(line: String) -> ActionError {
    ActionError::new(ActionErrorCode::ProviderRejected, false, line).unwrap_or_else(|_| {
        port_error(
            ActionErrorCode::ProviderRejected,
            "the mind cannot be sealed aboard",
        )
    })
}

/// The paid view's phase as the Roll's `TermPhase` (`OwnerPhase` folds `NotEffective` into
/// `Active`, which is how the holdings verb reads it).
fn phase_of(phase: mush_roll::commerce::command_reader::OwnerPhase) -> TermPhase {
    use mush_roll::commerce::command_reader::OwnerPhase;
    match phase {
        OwnerPhase::Active { .. } => TermPhase::Active,
        OwnerPhase::Grace { .. } => TermPhase::Grace,
        OwnerPhase::Lapsed => TermPhase::Lapsed,
    }
}

/// **The licence's state at the dispatch** from a holdings row (a paid view's, or an owner
/// row's with `revoked` false): M8a's `licence_state_of` with no recall bit (neither row has
/// one).
pub(crate) fn state_of(
    phase: mush_roll::commerce::command_reader::OwnerPhase,
    revoked: bool,
    selling: bool,
) -> SealLicenceState {
    licence_state_of(phase_of(phase), revoked, false, selling)
}

/// **Resolve the operator's `mind=` word at the dispatch** (the start's preparation): a
/// filed licence through the Roll's paid view and its admitted executable, a draft through the
/// operator's own saved revisions. `governed` is the rig's core's bit.
///
/// # Errors
///
/// **The owner row first, the paid view only when no owner row names the licence** (doc 17h
/// cut M8g's finding). A refused owner read poisons the whole preparation
/// (`PrepareCtx::read_owner_preimage`): every later owner read in it answers `Unavailable`. The
/// paid view (`OwnPaid`) refuses an owner licence, so asked first it left the owner row
/// unreadable and every owner licence, a character's or an organisation's, read as missing.
/// The owner holdings page reads refuse nothing a launch could still pass: a character's own
/// pages always answer, and an organisation's refuse only a character acting for none of its
/// scopes.
fn owner_then_paid<S, V>(
    mut state: S,
    owner: impl FnOnce(&mut S) -> Option<V>,
    paid: impl FnOnce(&mut S) -> Option<V>,
) -> Option<V> {
    owner(&mut state).or_else(|| paid(&mut state))
}

/// The refusal lines above, said to the operator.
fn resolve(
    ctx: &mut PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    actor: ObjId,
    (word, org): (&MindWord, Option<mush_org::OrgId>),
    governed: bool,
) -> Result<Resolved, ActionError> {
    let world = ctx
        .world()
        .map_err(|_| refuse(MIND_UNAVAILABLE.to_owned()))?;
    match word {
        MindWord::Draft(name) => {
            if org.is_some() {
                return Err(refuse(MIND_NO_DRAFT.replace("{word}", name)));
            }
            if governed {
                return Err(refuse(MIND_GOVERNED_DRAFT.to_owned()));
            }
            let held =
                crate::match_::holdings::choose(world, actor, &format!("draft:{name}"), None)
                    .ok()
                    .filter(|held| {
                        held.saved.is_some()
                            && matches!(
                                held.identity.origin,
                                crate::arena::profile_seal::MindOrigin::Draft { .. }
                            )
                    })
                    .ok_or_else(|| refuse(MIND_NO_DRAFT.replace("{word}", name)))?;
            Ok(Resolved::Held {
                held: Box::new(held),
                state: SealLicenceState::Draft,
                licence: None,
                holder: None,
            })
        }
        MindWord::Filed(licence) => {
            let licence = *licence;
            let missing =
                || refuse(MIND_NO_LICENCE.replace("{licence}", &licence.get().to_string()));
            let reads = crate::match_::original_access::DispatchReads::of(world)
                .ok_or_else(|| refuse(MIND_UNAVAILABLE.to_owned()))?;
            // An owner licence's row of the owner projection (fix round 1, J2), else a paid
            // licence's view: the phase, the revoked bit (an owner row has none; a revoked owner
            // term is refused by the admission read below), the selling bit and the mind's name.
            let (phase, revoked, selling, name) = owner_then_paid(
                (&mut *ctx, &mut *txn),
                |(ctx, txn)| {
                    reads.owner(ctx, txn, licence, org).map(|row| {
                        (
                            row.owner_phase(),
                            false,
                            row.selling(),
                            row.name().to_owned(),
                        )
                    })
                },
                |(ctx, txn)| {
                    reads.paid(ctx, txn, licence, org).map(|view| {
                        (
                            view.phase(),
                            view.revoked(),
                            view.selling(),
                            view.name().to_owned(),
                        )
                    })
                },
            )
            .ok_or_else(missing)?;
            let holder = match org {
                Some(org) => Holder::Organization { org, acting: actor },
                None => Holder::Character(actor),
            };
            // Doc 17h cut M8a: recalled is on neither view; a recalled revision's holding is
            // refused by the admission read below and so reads as a missing licence.
            let state = state_of(phase, revoked, selling);
            match state {
                SealLicenceState::Pending => Err(refuse(MIND_PENDING.replace("{mind}", &name))),
                SealLicenceState::Lapsed | SealLicenceState::Revoked => Ok(Resolved::Rom {
                    state,
                    licence,
                    holder,
                }),
                SealLicenceState::Active | SealLicenceState::Grace => {
                    let held = reads
                        .executable(ctx, txn, actor, licence)
                        .ok_or_else(missing)?;
                    Ok(Resolved::Held {
                        held: Box::new(held),
                        state,
                        licence: Some(licence),
                        holder: Some(holder),
                    })
                }
                SealLicenceState::Doctrine
                | SealLicenceState::Draft
                | SealLicenceState::Recalled => Err(missing()),
            }
        }
    }
}

/// **Seal `held` over the rig's unit** (doc 23 §6; D-UM-45: at the dispatch), the IC
/// execution's policy (`crate::minds::ic_sortie`'s `Execution::capture`): its authored
/// definition on the Line card, rated as held, capped by `[tacsim.minds].max_class`, the
/// operator the warrant principal (D-23-5), no weapons (a rig carries none).
///
/// # Errors
///
/// The seal's own refusal (a governed core under the mind's class, a card under the floor).
pub(crate) fn seal_held(
    tv: ObjId,
    unit: &mush_tactics::TacticalUnit,
    sortie: SortieId,
    held: &HeldMind,
    operator: ObjId,
    store: &TacsimStore,
) -> Result<MindSeal, crate::match_::seal::SealError> {
    let principal = Some(CharacterRef::Character { id: operator });
    let card = crate::arena::card::line(&store.cfg.arena.reflex);
    let fallback = None;
    crate::match_::seal::seal_with_policy(
        tv,
        unit,
        &[],
        sortie,
        &crate::match_::seal::SealPolicy {
            authored: held.definition.as_deref(),
            selected_profiles: &[],
            classes: &store.cfg.minds.classes,
            rom_cfg: &store.cfg.arena.reflex,
            principal: &principal,
            card: &card,
            odds_floor_permille: store.cfg.minds.odds_floor_permille,
            engine_for_minds: held.engine,
            rated: held.rated,
            class_cap: store.cfg.minds.max_class,
            identity: &held.identity,
            profiles: &[],
            fallback: &fallback,
            impounded: false,
        },
    )
}

/// **The ROM seal over the rig's unit** (doc 23 cut S.1's `seal_rom_for`), for a licence whose
/// state falls the hull to doctrine, or (doc 17h cut M8d2) for a rig whose core is impounded,
/// sealed then as no core.
pub(crate) fn seal_rom(
    tv: ObjId,
    unit: &mush_tactics::TacticalUnit,
    sortie: SortieId,
    operator: ObjId,
    store: &TacsimStore,
    impounded: bool,
) -> Result<MindSeal, crate::match_::seal::SealError> {
    crate::match_::seal::seal_rom_for(
        tv,
        unit,
        &[],
        sortie,
        &crate::match_::seal::IcSealConfig {
            minds: &store.cfg.minds,
            reflex: &store.cfg.arena.reflex,
            principal: Some(CharacterRef::Character { id: operator }),
            impounded,
        },
    )
}

/// **The dispatch's seal** (doc 17h cut M8e E4, E5), in the launch start's preparation:
/// `None` with no `mind=` word; else the rig must carry a core, the word resolves
/// ([`resolve`]) and the mind (or, for a lapsed or revoked licence, the ROM) is sealed over the
/// rig's unit with `SortieId::ic(tv, launch_ic)`, the facts carrying the licence's state and
/// holder.
///
/// # Errors
///
/// The refusal lines above, or the seal's own refusal (said to the operator as the seal's
/// reason).
pub(crate) fn dispatch(
    ctx: &mut PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    (actor, tv, launch_ic): (ObjId, ObjId, i64),
    word: Option<(&MindWord, Option<mush_org::OrgId>)>,
) -> Result<Option<DispatchSeal>, ActionError> {
    let Some(word) = word else {
        return Ok(None);
    };
    let (unit, governed, impounded) = {
        let world = ctx
            .world()
            .map_err(|_| refuse(MIND_UNAVAILABLE.to_owned()))?;
        let unit = mush_tactics::module::unit_of(world, tv)
            .cloned()
            .ok_or_else(|| refuse(MIND_NO_CORE.to_owned()))?;
        let core =
            crate::arena::core::read_core(&unit).ok_or_else(|| refuse(MIND_NO_CORE.to_owned()))?;
        let impounded = crate::match_::seal::core_impounded(world, &unit);
        (unit, core.governed, impounded)
    };
    let resolved = resolve(ctx, txn, actor, word, governed)?;
    let world = ctx
        .world()
        .map_err(|_| refuse(MIND_UNAVAILABLE.to_owned()))?;
    let store = world
        .module_state::<TacsimStore>()
        .ok_or_else(|| refuse(MIND_UNAVAILABLE.to_owned()))?;
    let sortie = SortieId::ic(tv, launch_ic);
    // Doc 17h cut M8d2 ("impounded in place"): the word still resolves, so its refusals stand,
    // but a rig whose core the Registry holds flies the ROM — no core, Class 0, governed —
    // under `doctrine`, naming no licence and no holder (the licence takes no flight mark).
    // Nothing else is said: the holder reads the claim on the core's title.
    if impounded {
        let seal = seal_rom(tv, &unit, sortie, actor, store, true)
            .map_err(|error| refuse(error.to_string()))?;
        return Ok(Some(assemble(
            &seal,
            SealLicenceState::Doctrine,
            None,
            None,
        )));
    }
    let (seal, state, licence, holder) = match resolved {
        Resolved::Held {
            held,
            state,
            licence,
            holder,
        } => (
            seal_held(tv, &unit, sortie, &held, actor, store),
            state,
            licence,
            holder,
        ),
        Resolved::Rom {
            state,
            licence,
            holder,
        } => (
            seal_rom(tv, &unit, sortie, actor, store, false),
            state,
            Some(licence),
            Some(holder),
        ),
    };
    let seal = seal.map_err(|error| refuse(error.to_string()))?;
    Ok(Some(assemble(&seal, state, licence, holder)))
}

/// **The dispatch's record of `seal`** under the licence's `state`, `licence` and `holder`:
/// the seal's facts with the holder beside them and, for a licence that fell the hull to the
/// ROM, the licence named on the line in place of `doctrine` (an inspection reads which licence
/// lapsed; the seal and its digest are the ROM's).
pub(crate) fn assemble(
    seal: &MindSeal,
    state: SealLicenceState,
    licence: Option<LicenceId>,
    holder: Option<Holder>,
) -> DispatchSeal {
    let mut facts = SealFacts::of(seal, state);
    facts.holder = holder;
    if let (Some(licence), SealLicenceState::Lapsed | SealLicenceState::Revoked) = (licence, state)
    {
        facts.licence = licence.get().to_string().into();
    }
    DispatchSeal {
        facts,
        licence: licence.map(LicenceId::get),
    }
}

/// **The launch's end clause for a licence that fell the rig to doctrine** (doc 17h cut M8e
/// E4): `LAUNCHED_DOCTRINE` naming the licence and its state, for a seal taken under a licence
/// (a holder named) that was lapsed or revoked at the dispatch; `None` otherwise. Said in the
/// operator's own completion line alone (`crate::surface::tick`), never to anyone else: the
/// line and the inspection read the state, not a notice.
#[must_use]
pub(crate) fn doctrine_clause(facts: &SealFacts, n: &str) -> Option<String> {
    let state = facts.licence_state?;
    (facts.holder.is_some()
        && matches!(state, SealLicenceState::Lapsed | SealLicenceState::Revoked))
    .then(|| {
        crate::port::tv::LAUNCHED_DOCTRINE
            .replace("{licence}", &facts.licence)
            .replace("{state}", state.as_str())
            .replace("{n}", n)
    })
}

#[cfg(test)]
mod tests;
