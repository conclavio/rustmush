//! **The customs picket's ruling, as one submitted action** (doc 35 cut SF.R26c; doc 35d §9a,
//! Q-35d-6 as ruled).
//!
//! `+helm/submit` and `+helm/run` used to write their durable effects in the handler: the
//! fine's contract, the faction front, the seizure's draw on the hold (a `tacsim_port_burn`
//! submitted after clearance was already granted), and the picket's verdict in the bubble.
//! Each now ends in **one** submitted action, [`CUSTOMS_ACTION`]:
//!
//! 1. The handler checks the seat, the bubble and the heave-to, reads the manifest, and files
//!    a [`PendingCustomsRuling`] on the space book: what customs found, and what it will do
//!    about it. It grants nothing.
//! 2. The action's prepare writes the ruling into the hull's row of
//!    [`CUSTOMS_RULING_FAMILY`], and for a seizure debits the ore from the hold **in the same
//!    transaction**, so a recorded seizure and an emptied hold are one fact. A sequence the
//!    row already holds is refused, so a ruling lands once.
//! 3. The space tick (`reconcile_customs_rulings`) reads the committed row and acts on it:
//!    it levies the fine, advances the front, and only then sets the picket's verdict and the
//!    corvette's relation in the bubble. A ruling whose submission was turned away (the engine
//!    holds one queued submission an actor, and a restart empties its queue) is submitted
//!    again after [`CUSTOMS_RETRY_AFTER_TICKS`], and given up after [`CUSTOMS_MAX_ATTEMPTS`]:
//!    the ship is then not cleared, and her crew are told.
//!
//! The economy's contracts and the faction's fronts are not action-state families, so the
//! action cannot write them; the tick does, on the committed row. The Registry's mind fine
//! (`crate::minds::registry_inspection::fine`) set the pattern.

use std::collections::BTreeMap;

use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalId, CmdError, Ctx, DefinitionRef, ModError, ObjId, PerceivedRef,
    ProjectedParameterKind, ProviderOfferDraft, Registrar, RulesValue, StateFamilyHandle,
};
use mush_inventory::InventoryFamilies;
use mush_inventory::record::EndpointRef;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::picket::{PicketInspectionOutcome, PicketInspectionStatus};
use super::tick::SpaceBubbleBook;
use crate::arena::party::Mark;
use crate::port_tick::port_error;

/// Authoritative action id of the customs ruling.
pub const CUSTOMS_ACTION: &str = "tacsim_space_customs";

/// Authoritative outcome id of [`CUSTOMS_ACTION`].
pub const CUSTOMS_OUTCOME: &str = "tacsim_space_customs_ruled";

/// Typed state family of landed customs rulings, one row a hull.
pub const CUSTOMS_RULING_FAMILY: &str = "tacsim.space_customs_ruling";

/// Prefix of a ruling's idempotency key: `tacsim.space.customs/{hull}/{seq}`.
pub const CUSTOMS_KEY_PREFIX: &str = "tacsim.space.customs/";

/// How many landed rulings a hull's row keeps.
pub const MAX_CUSTOMS_RULINGS: usize = 16;

/// How many rulings may be filed and not yet applied, over all hulls.
pub const MAX_PENDING_CUSTOMS_RULINGS: usize = 32;

/// Ticks a filed ruling may sit with no row and nothing in flight before the tick submits it
/// again.
pub const CUSTOMS_RETRY_AFTER_TICKS: u8 = 3;

/// Times the tick submits a filed ruling again before it gives the ruling up.
pub const CUSTOMS_MAX_ATTEMPTS: u8 = 3;

/// `+helm/submit`'s and `+helm/run`'s refusal while an earlier ruling on the ship is not yet
/// applied.
pub const CUSTOMS_RULING_PENDING: &str =
    "Republic Customs is still recording its last ruling on this ship; wait for it.";

/// The handlers' refusal when the ruling's action is not registered.
pub const CUSTOMS_UNWIRED: &str = "Republic Customs cannot record a ruling here.";

/// The crew's line when a filed ruling never landed and is given up.
pub const CUSTOMS_RULING_LAPSED: &str = "[PICKET] Republic Customs could not record its ruling on this ship, so nothing is settled and she is not cleared. Heave to and run +helm/submit again.";

/// The crew's line when a landed fine could not be collected; `{reason}` is the economy's.
pub const CUSTOMS_FINE_UNCOLLECTED: &str = "[PICKET] Republic Customs could not collect the contraband fine ({reason}), so this ship is not cleared. Heave to and run +helm/submit again.";

/// What customs found, and what follows from it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CustomsFinding {
    /// Nothing unlicensed aboard: the ship is cleared.
    Cleared {
        /// `strategic_ore` in the hold, kilograms.
        strategic_ore_kg: u64,
    },
    /// Unlicensed ore, and the hull's principal can pay: a fine is levied, then she is cleared.
    Fined {
        /// Unlicensed `strategic_ore`, kilograms.
        unlicensed_strategic_ore_kg: u64,
        /// The fine, minor credit units.
        fine_minor: i64,
    },
    /// Unlicensed ore, and the principal cannot pay: the ore is taken, then she is cleared.
    Seized {
        /// Unlicensed `strategic_ore`, kilograms.
        unlicensed_strategic_ore_kg: u64,
        /// Whole tonnes to take from the hold. On a landed ruling, what was taken.
        seize_t: u32,
        /// The hold vault the ore is taken from (`VaultId.0`).
        vault: u64,
    },
    /// The ship refused inspection (`+helm/run`).
    Ran,
}

impl CustomsFinding {
    /// The outcome's word for this finding.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Cleared { .. } => "cleared",
            Self::Fined { .. } => "fined",
            Self::Seized { .. } => "seized",
            Self::Ran => "ran",
        }
    }
}

/// One landed customs ruling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomsRuling {
    /// The ruling's sequence on its hull.
    pub seq: u32,
    /// Who typed `+helm/submit` or `+helm/run`.
    pub actor: ObjId,
    /// Who answers for the hull's manifest and fines.
    pub principal: ObjId,
    /// World IC time the ruling was filed.
    pub at_ic: i64,
    /// What customs found, as it landed.
    pub finding: CustomsFinding,
}

/// **The landed customs rulings of one hull** ([`CUSTOMS_RULING_FAMILY`]).
///
/// Written by [`CUSTOMS_ACTION`]'s prepare, in the same transaction as a seizure's debit.
/// The space tick reads it to apply a ruling, and the prepare reads it to refuse one that has
/// already landed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceCustomsRulings {
    /// The hull ruled on; the row's key.
    pub hull: ObjId,
    /// Landed rulings, ascending by sequence, at most [`MAX_CUSTOMS_RULINGS`].
    pub landed: Vec<CustomsRuling>,
}

impl SpaceCustomsRulings {
    /// The record invariants of [`CUSTOMS_RULING_FAMILY`].
    ///
    /// # Errors
    ///
    /// The row names no hull, holds too many rulings, or holds them out of order.
    pub fn check(&self) -> Result<(), String> {
        if self.hull.0 == 0 {
            return Err("a customs ruling row names its hull".to_owned());
        }
        if self.landed.len() > MAX_CUSTOMS_RULINGS {
            return Err("a customs ruling row holds a bounded number of rulings".to_owned());
        }
        if !self.landed.windows(2).all(|pair| pair[0].seq < pair[1].seq) {
            return Err("a customs ruling row holds its rulings ascending".to_owned());
        }
        Ok(())
    }

    /// The landed ruling `seq`, while the row still keeps it.
    #[must_use]
    pub fn ruling(&self, seq: u32) -> Option<&CustomsRuling> {
        self.landed.iter().find(|ruling| ruling.seq == seq)
    }

    /// Whether the ruling `seq` has landed. A sequence below the lowest one a full row keeps
    /// has been dropped from it, and counts as landed.
    #[must_use]
    pub fn has_landed(&self, seq: u32) -> bool {
        self.ruling(seq).is_some()
            || (self.landed.len() >= MAX_CUSTOMS_RULINGS
                && self.landed.first().is_some_and(|lowest| seq < lowest.seq))
    }

    /// The highest landed sequence, 0 when none has.
    #[must_use]
    pub fn high_water(&self) -> u32 {
        self.landed.last().map_or(0, |ruling| ruling.seq)
    }

    /// Record `ruling` as landed, dropping the lowest sequence when the row is full.
    pub fn record(&mut self, ruling: CustomsRuling) {
        if let Err(at) = self
            .landed
            .binary_search_by_key(&ruling.seq, |held| held.seq)
        {
            self.landed.insert(at, ruling);
        }
        if self.landed.len() > MAX_CUSTOMS_RULINGS {
            let excess = self.landed.len() - MAX_CUSTOMS_RULINGS;
            self.landed.drain(0..excess);
        }
    }
}

/// **A ruling filed by `+helm/submit` or `+helm/run` and not yet applied**: on the space book
/// (so it survives a restart), fixed into [`CUSTOMS_ACTION`]'s offer, and settled by
/// `reconcile_customs_rulings`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingCustomsRuling {
    /// The ruling's sequence on its hull.
    pub seq: u32,
    /// The hull ruled on.
    pub hull: ObjId,
    /// Who typed the command; the action is theirs.
    pub actor: ObjId,
    /// Who answers for the hull's manifest and fines.
    pub principal: ObjId,
    /// World IC time the ruling was filed.
    pub at_ic: i64,
    /// What customs found.
    pub finding: CustomsFinding,
    /// The corvette's ship id in the bubble, for the crew's lines.
    pub corvette: String,
    /// The hull's own ship id in the bubble, for the crew's lines.
    pub carrier: String,
    /// How many times the space tick has submitted this ruling again.
    #[serde(default)]
    pub attempts: u8,
    /// Consecutive ticks on which the ruling had neither landed nor been in flight.
    #[serde(default)]
    pub idle_ticks: u8,
}

/// World-state handles of the customs ruling.
#[derive(Clone)]
pub struct CustomsHandles {
    /// The registered [`CUSTOMS_ACTION`], when its definition is on the roll.
    pub action: Option<DefinitionRef>,
    /// [`CUSTOMS_RULING_FAMILY`].
    pub rulings: StateFamilyHandle<ObjId, SpaceCustomsRulings>,
}

/// Outcome descriptor for [`CUSTOMS_OUTCOME`].
pub(crate) fn outcome_descriptor() -> Result<mush_core::DefinitionDescriptor, ModError> {
    crate::port_tick::descriptor(
        "outcome",
        CUSTOMS_OUTCOME,
        json!({"format":1,"fields":["actor","hull","ruling","seq"]}),
    )
}

/// Action descriptor for [`CUSTOMS_ACTION`].
pub(crate) fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<mush_core::DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(CUSTOMS_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the space customs outcome did not register"))?;
    crate::port_tick::descriptor(
        "action",
        CUSTOMS_ACTION,
        json!({"format":1,"subject":"self_actor","targets":0,"family":CUSTOMS_RULING_FAMILY}),
    )
    .map(|descriptor| descriptor.with_references(vec![outcome]))
}

/// Register [`CUSTOMS_RULING_FAMILY`], the provider of [`CUSTOMS_ACTION`] and
/// [`CustomsHandles`]. The family and the handles register regardless; the provider when the
/// action's definition is on the roll. Without the inventory cell a seizure is refused.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    inventory: Option<&InventoryFamilies>,
) -> Result<(), ModError> {
    let rulings = reg.state_family::<ObjId, SpaceCustomsRulings, _, _, _, _>(
        crate::port::families::family_descriptor(
            CUSTOMS_RULING_FAMILY,
            "tacsim.space_customs_ruling_key.v1",
            "tacsim.space_customs_ruling",
        )?,
        |key: &ObjId| Ok(key.0.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &SpaceCustomsRulings| value.check(),
        |value: &SpaceCustomsRulings| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let action = if let Some(action) = actions.get(CUSTOMS_ACTION).cloned() {
        let outcome = outcomes
            .get(CUSTOMS_OUTCOME)
            .cloned()
            .ok_or_else(|| ModError::msg("the space customs outcome did not register"))?;
        reg.action_provider(
            action.clone(),
            CustomsProvider {
                action: action.clone(),
                outcome,
                rulings: rulings.clone(),
                inventory: inventory.cloned(),
            },
        )?;
        Some(action)
    } else {
        None
    };
    reg.state(CustomsHandles { action, rulings });
    Ok(())
}

// ---- the provider ------------------------------------------------------------------

/// **`tacsim_space_customs`**: the picket's ruling on one hull.
pub(crate) struct CustomsProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub rulings: StateFamilyHandle<ObjId, SpaceCustomsRulings>,
    pub inventory: Option<InventoryFamilies>,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "a customs ruling is self-scoped, takes no targets and names one filed ruling",
    )
}

/// The invocation's `seq`, checked for shape: self-scoped, no targets, decimal text.
fn seq_of(invocation: &ActionInvocation) -> Result<u32, mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    invocation
        .parameters()
        .as_object()
        .and_then(|parameters| parameters.get("seq"))
        .and_then(Value::as_str)
        .and_then(crate::port::families::strict_decimal_u32)
        .ok_or_else(invalid)
}

/// The rulings `actor` has filed on the hull they are aboard, read from the world at the
/// offer.
fn filed_by(w: &mush_core::World, actor: ObjId) -> Vec<PendingCustomsRuling> {
    let Some(hull) = super::commands::hull_aboard_for_actor(w, actor) else {
        return Vec::new();
    };
    w.module_state::<SpaceBubbleBook>()
        .map(|book| {
            book.pending_customs_rulings
                .iter()
                .filter(|ruling| ruling.actor == actor && ruling.hull == hull)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

impl ActionProvider for CustomsProvider {
    fn validate_invocation(
        &self,
        invocation: &ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        seq_of(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, mush_core::ActionError> {
        if !crate::surface::act::self_query(action, &self.action, query) {
            return Ok(None);
        }
        let filed = filed_by(&engine.world, query.actor());
        if filed.is_empty() {
            return Ok(None);
        }
        let choices: Vec<String> = filed.iter().map(|ruling| ruling.seq.to_string()).collect();
        let rulings = serde_json::to_value(&filed).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the filed customs ruling does not encode",
            )
        })?;
        let seq = crate::surface::act::parameter(
            "seq",
            true,
            ProjectedParameterKind::Choice { choices },
        )?;
        crate::surface::act::offer_draft(
            action,
            engine,
            json!({ "rulings": rulings }),
            vec![seq],
            (
                "tacsim-space-customs",
                "Record the customs ruling",
                "Record what the customs picket found aboard, and settle it.",
            ),
        )
        .map(Some)
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        let seq = seq_of(invocation)?;
        // 1. The server-authenticated filing.
        let filed: Vec<PendingCustomsRuling> = invocation
            .parameters()
            .get("rulings")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(invalid)?;
        let Some(pending) = filed.into_iter().find(|ruling| ruling.seq == seq) else {
            return Err(port_error(
                ActionErrorCode::InvalidRequest,
                "that customs ruling is not the one on file",
            ));
        };
        let actor = invocation.actor();
        if actor != pending.actor {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that customs ruling is another officer's",
            ));
        }
        // 2. The row is the replay guard: the tick submits a ruling again when it has no row
        //    and nothing is in flight, and one that did land is refused here.
        let prior = txn
            .get(&self.rulings, &pending.hull)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the customs rulings are unavailable",
                )
            })?
            .cloned();
        if prior.as_ref().is_some_and(|row| row.has_landed(seq)) {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "that customs ruling has already landed",
            ));
        }
        // 3. A seizure takes the ore in this transaction.
        let finding = match pending.finding {
            CustomsFinding::Seized {
                unlicensed_strategic_ore_kg,
                seize_t,
                vault,
            } => {
                let Some(inventory) = self.inventory.as_ref() else {
                    return Err(port_error(
                        ActionErrorCode::Integrity,
                        "the hold cannot be drawn on here",
                    ));
                };
                let ore = CanonicalId::new("strategic_ore").map_err(|_| invalid())?;
                let endpoint = EndpointRef(mush_economy::VaultId(vault));
                let free = inventory
                    .pool(txn, endpoint, &ore)
                    .map_err(|_| port_error(ActionErrorCode::Integrity, "the hold cannot be read"))?
                    .map_or(0, |pool| pool.available);
                let taken = u64::from(seize_t).min(free);
                if taken == 0 {
                    return Err(port_error(
                        ActionErrorCode::ProviderRejected,
                        "the hold holds no strategic ore to seize",
                    ));
                }
                inventory.debit(txn, endpoint, &ore, taken).map_err(|_| {
                    port_error(
                        ActionErrorCode::ProviderRejected,
                        "the hold could not be drawn down",
                    )
                })?;
                CustomsFinding::Seized {
                    unlicensed_strategic_ore_kg,
                    seize_t: u32::try_from(taken).unwrap_or(seize_t),
                    vault,
                }
            }
            other => other,
        };
        let word = finding.word();
        let mut row = prior.clone().unwrap_or(SpaceCustomsRulings {
            hull: pending.hull,
            landed: Vec::new(),
        });
        row.record(CustomsRuling {
            seq,
            actor,
            principal: pending.principal,
            at_ic: pending.at_ic,
            finding,
        });
        let staged = if prior.is_some() {
            txn.replace(&self.rulings, &pending.hull, row)
        } else {
            txn.insert(&self.rulings, &pending.hull, row)
        };
        staged.map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the customs ruling did not stage",
            )
        })?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": actor.0,
                "hull": pending.hull.0,
                "ruling": word,
                "seq": seq,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the customs ruling's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

// ---- the handlers' half ------------------------------------------------------------

/// A hull's landed rulings, read outside an action.
///
/// # Errors
///
/// The family is unwired or does not read.
pub(crate) fn rulings_of(ctx: &Ctx<'_>, hull: ObjId) -> Result<Option<SpaceCustomsRulings>, ()> {
    let handles = ctx.w.module_state::<CustomsHandles>().ok_or(())?;
    ctx.action_state(&handles.rulings, &hull)
        .map(|row| row.cloned())
        .map_err(|_| ())
}

/// Whether a ruling on `hull` is filed and not yet applied.
#[must_use]
pub(crate) fn ruling_pending(w: &mush_core::World, hull: ObjId) -> bool {
    w.module_state::<SpaceBubbleBook>().is_some_and(|book| {
        book.pending_customs_rulings
            .iter()
            .any(|ruling| ruling.hull == hull)
    })
}

/// **File a ruling on `hull` and submit it as the acting officer**: the whole durable effect
/// of `+helm/submit` and `+helm/run`. Nothing is granted here; the space tick applies the
/// ruling once it has landed. Answers the ruling's sequence.
///
/// # Errors
///
/// The action is unwired, a ruling on the hull is still pending, or too many are.
pub(crate) fn file_ruling(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    principal: ObjId,
    finding: CustomsFinding,
    (corvette, carrier): (String, String),
) -> Result<u32, CmdError> {
    if ctx
        .w
        .module_state::<CustomsHandles>()
        .and_then(|handles| handles.action.clone())
        .is_none()
    {
        return Err(CmdError::msg(CUSTOMS_UNWIRED));
    }
    if ruling_pending(ctx.w, hull) {
        return Err(CmdError::msg(CUSTOMS_RULING_PENDING));
    }
    // A sequence the row already holds would read as landed.
    let landed_high = rulings_of(ctx, hull)
        .map_err(|()| CmdError::msg(CUSTOMS_UNWIRED))?
        .map_or(0, |row| row.high_water());
    let actor = ctx.actor;
    let at_ic = ctx.w.world_ic_now().at;
    let pending = {
        let book = ctx
            .w
            .module_state_mut::<SpaceBubbleBook>()
            .ok_or_else(|| CmdError::msg("Space bubble state is not registered."))?;
        if book.pending_customs_rulings.len() >= MAX_PENDING_CUSTOMS_RULINGS {
            return Err(CmdError::msg(CUSTOMS_RULING_PENDING));
        }
        book.next_customs_ruling_seq = book
            .next_customs_ruling_seq
            .max(landed_high)
            .saturating_add(1);
        let pending = PendingCustomsRuling {
            seq: book.next_customs_ruling_seq,
            hull,
            actor,
            principal,
            at_ic,
            finding,
            corvette,
            carrier,
            attempts: 0,
            idle_ticks: 0,
        };
        book.pending_customs_rulings.push(pending.clone());
        pending
    };
    submit_ruling(ctx, &pending, 0);
    Ok(pending.seq)
}

/// Submit the filed ruling as its officer. `attempt` is 0 for the handler's own submission
/// and counts the tick's resubmissions, each under its own idempotency key; the row, not the
/// key, is what keeps a ruling from landing twice.
fn submit_ruling(ctx: &mut Ctx<'_>, pending: &PendingCustomsRuling, attempt: u8) {
    let Some(action) = ctx
        .w
        .module_state::<CustomsHandles>()
        .and_then(|handles| handles.action.clone())
    else {
        return;
    };
    let key_text = if attempt == 0 {
        format!("{CUSTOMS_KEY_PREFIX}{}/{}", pending.hull.0, pending.seq)
    } else {
        format!(
            "{CUSTOMS_KEY_PREFIX}{}/{}/retry{attempt}",
            pending.hull.0, pending.seq
        )
    };
    let Ok(key) = mush_core::IdempotencyKeyText::parse_canonical(key_text) else {
        return;
    };
    let parameters = json!({ "seq": pending.seq.to_string() });
    if ctx.actor == pending.actor {
        ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
    } else {
        let mut fire = crate::port_tick::Fire::new(BTreeMap::new());
        let _ = fire.as_principal(ctx, pending.actor, pending.actor, |ctx| {
            ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
        });
    }
}

// ---- the tick's half ---------------------------------------------------------------

/// **Settle the filed rulings against the landed ones** (the space tick).
///
/// A ruling whose row holds it has landed: it is applied ([`apply_landed`]) and leaves the
/// book. A ruling with no row whose officer has nothing in flight was turned away: after
/// [`CUSTOMS_RETRY_AFTER_TICKS`] it is submitted again, one ruling an officer a tick, oldest
/// first. After [`CUSTOMS_MAX_ATTEMPTS`] it is given up, and the ship is left as she was.
pub(crate) fn reconcile_customs_rulings(ctx: &mut Ctx<'_>) {
    let filed: Vec<PendingCustomsRuling> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|book| book.pending_customs_rulings.clone())
        .unwrap_or_default();
    if filed.is_empty() {
        return;
    }
    let commit_pending = ctx.authoritative_commit_pending();
    let mut landed: Vec<(PendingCustomsRuling, CustomsRuling)> = Vec::new();
    let mut waited = std::collections::BTreeSet::new();
    let mut retried: Vec<PendingCustomsRuling> = Vec::new();
    let mut lapsed: Vec<PendingCustomsRuling> = Vec::new();
    let mut visited = std::collections::BTreeSet::new();
    for pending in &filed {
        // An unreadable row holds the ruling where it is.
        let Ok(row) = rulings_of(ctx, pending.hull) else {
            continue;
        };
        if let Some(row) = row.as_ref()
            && row.has_landed(pending.seq)
        {
            // A ruling the full row has already dropped cannot be read back, so it cannot be
            // applied: it is given up like one that never landed, and the ship submits again.
            if let Some(ruling) = row.ruling(pending.seq) {
                landed.push((pending.clone(), ruling.clone()));
            } else {
                lapsed.push(pending.clone());
            }
            continue;
        }
        if commit_pending
            || ctx.authoritative_pending(pending.actor)
            || !visited.insert(pending.actor)
        {
            continue;
        }
        if pending.idle_ticks.saturating_add(1) < CUSTOMS_RETRY_AFTER_TICKS {
            waited.insert((pending.hull, pending.seq));
        } else if pending.attempts >= CUSTOMS_MAX_ATTEMPTS {
            lapsed.push(pending.clone());
        } else {
            retried.push(pending.clone());
        }
    }
    if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
        book.pending_customs_rulings.retain(|pending| {
            !landed
                .iter()
                .map(|(gone, _)| gone)
                .chain(lapsed.iter())
                .any(|gone| gone.hull == pending.hull && gone.seq == pending.seq)
        });
        for pending in &mut book.pending_customs_rulings {
            if waited.contains(&(pending.hull, pending.seq)) {
                pending.idle_ticks = pending.idle_ticks.saturating_add(1);
            } else if retried
                .iter()
                .any(|again| again.hull == pending.hull && again.seq == pending.seq)
            {
                pending.attempts = pending.attempts.saturating_add(1);
                pending.idle_ticks = 0;
            }
        }
    }
    for (pending, ruling) in landed {
        apply_landed(ctx, &pending, &ruling);
    }
    for pending in retried {
        submit_ruling(ctx, &pending, pending.attempts.saturating_add(1));
    }
    for pending in lapsed {
        tracing::warn!(
            hull = pending.hull.0,
            seq = pending.seq,
            ruling = pending.finding.word(),
            "tacsim: a filed customs ruling never landed and is given up"
        );
        say(ctx, &pending, CUSTOMS_RULING_LAPSED);
    }
}

/// Tell the hull's crew, and the officer who filed the ruling.
fn say(ctx: &mut Ctx<'_>, pending: &PendingCustomsRuling, line: &str) {
    crate::port::voyage::narrate_hull(ctx, pending.hull, line);
    ctx.out
        .sys_to_actor(pending.actor, "tacsim.space", line.to_owned());
}

/// Set the picket's verdict on `hull` in her bubble, when she is still in one with a picket
/// that has not already written her off as a runner. Answers whether it was set.
fn set_verdict(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    relation: Mark,
    status: PicketInspectionStatus,
    outcome: PicketInspectionOutcome,
) -> bool {
    let Some(active) = ctx
        .w
        .module_state_mut::<SpaceBubbleBook>()
        .and_then(|book| book.active_for_hull_mut(hull))
    else {
        return false;
    };
    let ran = active.picket.as_ref().is_some_and(|picket| {
        picket.inspection_status_for(hull) == PicketInspectionStatus::RanThePicket
    });
    if ran && status != PicketInspectionStatus::RanThePicket {
        return false;
    }
    let Some(picket) = active.picket.as_mut() else {
        return false;
    };
    picket.set_inspection_status_for(hull, status);
    picket.set_inspection_outcome_for(hull, outcome);
    let carrier_party = active.carrier_party_for(hull);
    active.bubble.set_party_relation(
        super::picket::CUSTOMS_CORVETTE_PARTY,
        carrier_party,
        Some(relation),
    );
    true
}

/// **Act on one landed ruling**: the fine, the front, then the verdict in the bubble.
fn apply_landed(ctx: &mut Ctx<'_>, pending: &PendingCustomsRuling, ruling: &CustomsRuling) {
    let hull = pending.hull;
    let (corvette, carrier) = (&pending.corvette, &pending.carrier);
    let front_title = mush_faction::CUSTOMS_ASSERTION_TITLE;
    let line = match ruling.finding {
        CustomsFinding::Cleared { strategic_ore_kg } => {
            set_verdict(
                ctx,
                hull,
                Mark::Friendly,
                PicketInspectionStatus::Cleared,
                PicketInspectionOutcome::Cleared { strategic_ore_kg },
            );
            format!(
                "[PICKET] Republic Customs Corvette `{corvette}` inspected `{carrier}` manifest ({strategic_ore_kg} kg `strategic_ore`, licensed/compliant): customs clearance granted."
            )
        }
        CustomsFinding::Fined {
            unlicensed_strategic_ore_kg,
            fine_minor,
        } => {
            match super::picket::levy_customs_fine_and_advance_front(
                ctx,
                ruling.principal,
                hull,
                unlicensed_strategic_ore_kg,
                fine_minor,
            ) {
                Ok((contract, front)) => {
                    let front_raw = front.map(|f| f.0);
                    set_verdict(
                        ctx,
                        hull,
                        Mark::Friendly,
                        PicketInspectionStatus::ClearedAfterFine,
                        PicketInspectionOutcome::Fined {
                            contract_id: contract.0,
                            fine_minor,
                            unlicensed_strategic_ore_kg,
                            front_id: front_raw,
                        },
                    );
                    let fine = crate::port::coop::money_minor(ctx, fine_minor);
                    format!(
                        "[PICKET] Republic Customs Corvette `{corvette}` inspected `{carrier}` manifest: {unlicensed_strategic_ore_kg} kg unlicensed `strategic_ore` detected; assessed {fine} contraband fine (Ledger contract #{}, campaign front `{front_title}` #{}) and granted customs clearance.",
                        contract.0,
                        front_raw.unwrap_or(0)
                    )
                }
                Err(reason) => {
                    tracing::warn!(
                        hull = hull.0,
                        seq = ruling.seq,
                        %reason,
                        "tacsim: a landed customs fine was not collected"
                    );
                    CUSTOMS_FINE_UNCOLLECTED.replace("{reason}", &reason)
                }
            }
        }
        CustomsFinding::Seized {
            unlicensed_strategic_ore_kg,
            seize_t,
            ..
        } => {
            let seized_kg = u64::from(seize_t).saturating_mul(1_000);
            let front_raw =
                super::picket::record_customs_seizure(ctx, ruling.principal, hull, seized_kg)
                    .map(|f| f.0);
            if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                book.mined_strategic_blocks.remove(&hull);
            }
            set_verdict(
                ctx,
                hull,
                Mark::Friendly,
                PicketInspectionStatus::ClearedAfterSeizure,
                PicketInspectionOutcome::ContrabandSeized {
                    seized_strategic_ore_kg: seized_kg,
                    front_id: front_raw,
                },
            );
            format!(
                "[PICKET] Republic Customs Corvette `{corvette}` inspected `{carrier}` manifest: {unlicensed_strategic_ore_kg} kg unlicensed `strategic_ore` found, {seize_t} t confiscated in lieu of fine (insufficient credit; campaign front `{front_title}` #{}) and customs clearance granted.",
                front_raw.unwrap_or(0)
            )
        }
        CustomsFinding::Ran => {
            let front_raw =
                super::picket::record_ran_the_picket_incident(ctx, ruling.principal, hull)
                    .map(|f| f.0);
            set_verdict(
                ctx,
                hull,
                Mark::Hostile,
                PicketInspectionStatus::RanThePicket,
                PicketInspectionOutcome::RanThePicket {
                    front_id: front_raw,
                },
            );
            format!(
                "[PICKET] You refuse inspection and run the Republic Customs picket! Corvette transitions to Hostile; campaign front `{front_title}` #{} advanced.",
                front_raw.unwrap_or(0)
            )
        }
    };
    say(ctx, pending, &line);
    super::commands::push_space_panels(ctx, hull);
}
