//! E7c2 B5 — the prepared path: five action definitions, five providers.
//!
//! Shaped on `crates/ruleset-wod/src/authoritative.rs` (K8.4's migrated-action
//! pattern: state family + definition validator + definitions + `action_provider`)
//! and on `mush-tactics`' `brace` (idempotency + checked revision,
//! authoritative-only preparation).
//!
//! **Every quantity mutation is on the prepared path from the first commit**
//! (§4.1's cut-2 rows), so pools, reservations, lots and tranches are never
//! conventional documents and E7k has nothing of cut 2's to move (§5).
//!
//! **No ambient RNG anywhere in this file** — nothing here draws, and
//! `crates/mush-contract-tests/tests/provider_rng_policy.rs` scans it.
//!
//! # Where endpoint legality is enforced, and why there
//!
//! `prepare_authoritative` has an `ActionTxn` and no world, so a vault's *status*
//! cannot be read there. It is enforced instead where the tree already enforces
//! legality: [`ActionProvider::project_offer`] enumerates the actor's **live,
//! usable** endpoints as `ProjectedParameterKind::Choice` values, and
//! `Engine::resolve_legal_action` **regenerates and re-validates the offer
//! immediately before preparation**. An endpoint frozen, closed, destroyed or
//! contested between the command and the commit therefore drops out of the
//! regenerated choice list and the submission is refused — it is not a check that
//! can go stale, it is the same mechanism a station grant uses.

use std::collections::BTreeMap;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider,
    ActionStateError, ActionTxn, AuthoritativeActionOutput, CanonicalId, ClockStamp,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionValidationContext,
    Engine, LegalActionQuery, ModError, ObjId, PerceivedRef, PrincipalId, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    Registrar, RevisionSet, RulesValue, SubjectKey,
};
use mush_economy::{EconomyStore, EndpointState, VaultId, endpoint_state};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::pool::{InventoryError, InventoryFamilies};
use crate::record::{
    AppliedMarker, EndpointRef, FACT_LOT_CREATED, FACT_TRANSFERRED, Grade, LotId, MAX_GRADE,
    MAX_LOT_PARENTS, MAX_LOT_QUANTITY, TrancheId,
};

/// The five action ids, and the one outcome schema they share.
pub const ACTION_MOVE: &str = "inv_move";
/// Split one lot.
pub const ACTION_SPLIT: &str = "inv_split";
/// Merge several lots.
pub const ACTION_MERGE: &str = "inv_merge";
/// Pour a lot into its pool.
pub const ACTION_DISSOLVE: &str = "inv_dissolve";
/// Mint a lot out of a pool.
pub const ACTION_CRYSTALLIZE: &str = "inv_crystallize";
/// The one durable outcome schema all five reference.
pub const OUTCOME_ID: &str = "inv_result";

/// Largest endpoint choice list one offer publishes. Well inside K's
/// `MAX_PROJECTED_ACTION_CHOICES` (256): a projection nobody can read is not a
/// projection, and a hundred vaults is already an operator problem.
pub const MAX_ENDPOINT_CHOICES: usize = 64;
/// Largest commodity id one parameter may carry.
pub const MAX_COMMODITY_BYTES: usize = 64;
/// Largest lot-list text one merge may carry (`MAX_LOT_PARENTS` × 17 + slack).
pub const MAX_LOT_LIST_BYTES: usize = 512;

/// Which of the five this provider instance owns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryVerb {
    /// Pool quantity between two endpoints: one debit, one credit, one bundle.
    Move,
    /// Split one lot.
    Split,
    /// Merge several lots.
    Merge,
    /// Pour one lot into its endpoint's pool.
    Dissolve,
    /// Mint one lot out of a pool.
    Crystallize,
}

impl InventoryVerb {
    const fn id(self) -> &'static str {
        match self {
            Self::Move => ACTION_MOVE,
            Self::Split => ACTION_SPLIT,
            Self::Merge => ACTION_MERGE,
            Self::Dissolve => ACTION_DISSOLVE,
            Self::Crystallize => ACTION_CRYSTALLIZE,
        }
    }

    const fn verb(self) -> &'static str {
        match self {
            Self::Move => "inv-move",
            Self::Split => "inv-split",
            Self::Merge => "inv-merge",
            Self::Dissolve => "inv-dissolve",
            Self::Crystallize => "inv-crystallize",
        }
    }

    const fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::Move => ("Move stock", "Move pooled quantity between two endpoints."),
            Self::Split => ("Split a lot", "Split a quantity off one lot."),
            Self::Merge => ("Merge lots", "Merge lots of one commodity and grade."),
            Self::Dissolve => (
                "Dissolve a lot",
                "Pour one lot into its endpoint's pool, keeping its provenance.",
            ),
            Self::Crystallize => (
                "Crystallize stock",
                "Mint one lot out of a pool, parented to that pool's tranche.",
            ),
        }
    }
}

/// One inventory action's provider.
pub struct InventoryProvider {
    verb: InventoryVerb,
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: InventoryFamilies,
}

fn error(code: ActionErrorCode, message: impl Into<String>) -> ActionError {
    ActionError::new(code, false, message).expect("bounded inventory action error")
}

fn state_error(_source: &ActionStateError) -> ActionError {
    error(
        ActionErrorCode::Integrity,
        "inventory typed state is unavailable",
    )
}

fn refuse(source: &InventoryError) -> ActionError {
    match source {
        InventoryError::State | InventoryError::Identity | InventoryError::HalfPool => error(
            ActionErrorCode::Integrity,
            "That endpoint cannot be used right now.",
        ),
        other => error(ActionErrorCode::ProviderRejected, other.to_string()),
    }
}

/// Whether the actor may **take** quantity out of that endpoint.
///
/// E7c2 fold C8 / owner decision D4: this is E3's own vault-withdraw authority,
/// asked through `mush_economy`'s exported seam and rebuilt nowhere. Before the
/// fold the write gate and the read gate were the *same* predicate —
/// `has_role(Royalty) || acts_for(actor, &vault.owner)` — whose `Organization`
/// arm answers true on `holds_capability("economy.account.post", root)`, so an
/// org's **money-posting** grant read as authority to walk its physical stock
/// out. §3.2's invariant (3) says the custody rules are E3's, unchanged; this is
/// what makes that true, and it is the one `may_take` its custody test row names.
#[must_use]
pub fn may_take(world: &mush_core::World, actor: ObjId, endpoint: EndpointRef) -> bool {
    mush_economy::may_take_from_vault(world, actor, endpoint.vault())
}

/// Every endpoint the actor may **take from** and that is live right now.
///
/// Bounded on records visited, and total: an unreadable economy answers an empty
/// list, which projects no offer rather than an offer nobody can honour.
///
/// E7c2 fold C12: the matching set is collected, sorted and *then* truncated —
/// it used to `break` at [`MAX_ENDPOINT_CHOICES`] in snapshot order and sort
/// whatever survived, so past 64 vaults an actor silently lost endpoints they
/// hold. [`endpoints_truncated`] says whether the cap was reached, and the
/// command layer says so out loud rather than projecting a quiet omission.
#[must_use]
pub fn live_endpoints(world: &mush_core::World, actor: ObjId) -> Vec<VaultId> {
    collect_live_endpoints(world, actor).0
}

/// Whether the actor holds more live endpoints than one offer may publish.
#[must_use]
pub fn endpoints_truncated(world: &mush_core::World, actor: ObjId) -> bool {
    collect_live_endpoints(world, actor).1
}

fn collect_live_endpoints(world: &mush_core::World, actor: ObjId) -> (Vec<VaultId>, bool) {
    let Some(store) = world.module_state::<EconomyStore>() else {
        return (Vec::new(), false);
    };
    let mut out = Vec::new();
    for vault in store.snapshot().vaults() {
        // Bounded on records VISITED: one past the cap is all it takes to know
        // "there were more", and it is never an unbounded scan.
        if out.len() > MAX_ENDPOINT_CHOICES {
            break;
        }
        if !may_take(world, actor, EndpointRef(vault.id)) {
            continue;
        }
        if endpoint_state(world, vault.id).is_live() {
            out.push(vault.id);
        }
    }
    out.sort_unstable();
    let truncated = out.len() > MAX_ENDPOINT_CHOICES;
    out.truncate(MAX_ENDPOINT_CHOICES);
    (out, truncated)
}

fn endpoint_choices(endpoints: &[VaultId]) -> Vec<String> {
    endpoints.iter().map(|id| id.0.to_string()).collect()
}

fn parameter(
    id: &str,
    required: bool,
    kind: ProjectedParameterKind,
) -> Result<ProjectedParameter, ActionError> {
    Ok(ProjectedParameter {
        id: CanonicalId::new(id)
            .map_err(|_| error(ActionErrorCode::Integrity, "inventory parameter id"))?,
        required,
        kind,
    })
}

fn text_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, ActionError> {
    parameters
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                format!("inventory action needs a {key}"),
            )
        })
}

fn u64_field(parameters: &serde_json::Map<String, Value>, key: &str) -> Result<u64, ActionError> {
    parameters
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value >= 1 && *value <= MAX_LOT_QUANTITY)
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                format!("inventory action needs a {key} between 1 and {MAX_LOT_QUANTITY}"),
            )
        })
}

fn endpoint_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<EndpointRef, ActionError> {
    text_field(parameters, key)?
        .parse::<u64>()
        .ok()
        .map(|id| EndpointRef(VaultId(id)))
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                format!("inventory action needs an endpoint for {key}"),
            )
        })
}

fn commodity_field(
    parameters: &serde_json::Map<String, Value>,
) -> Result<CanonicalId, ActionError> {
    let raw = text_field(parameters, "commodity")?;
    if raw.len() > MAX_COMMODITY_BYTES {
        return Err(error(
            ActionErrorCode::InvalidRequest,
            "that commodity id is too long",
        ));
    }
    CanonicalId::new(&raw).map_err(|_| {
        error(
            ActionErrorCode::InvalidRequest,
            "that is not a commodity id",
        )
    })
}

fn lot_field(parameters: &serde_json::Map<String, Value>, key: &str) -> Result<LotId, ActionError> {
    LotId::parse(&text_field(parameters, key)?)
        .ok_or_else(|| error(ActionErrorCode::InvalidRequest, "that is not a lot id"))
}

fn lots_field(parameters: &serde_json::Map<String, Value>) -> Result<Vec<LotId>, ActionError> {
    let raw = text_field(parameters, "lots")?;
    if raw.len() > MAX_LOT_LIST_BYTES {
        return Err(error(
            ActionErrorCode::InvalidRequest,
            "that is too many lots",
        ));
    }
    let mut lots = Vec::new();
    for token in raw.split(',') {
        if lots.len() >= MAX_LOT_PARENTS {
            return Err(error(
                ActionErrorCode::InvalidRequest,
                format!("a merge names at most {MAX_LOT_PARENTS} lots"),
            ));
        }
        let lot = LotId::parse(token)
            .ok_or_else(|| error(ActionErrorCode::InvalidRequest, "that is not a lot id"))?;
        lots.push(lot);
    }
    Ok(lots)
}

/// The IC stamp a preparation uses. Deliberately **not** the wall clock: every
/// deadline comparison later is `ClockStamp::checked_cmp`, which refuses to
/// compare across generations rather than expiring a hold early.
fn prepare_now(parameters: &serde_json::Map<String, Value>) -> Result<ClockStamp, ActionError> {
    parameters
        .get("now")
        .and_then(|value| serde_json::from_value::<ClockStamp>(value.clone()).ok())
        .ok_or_else(|| {
            error(
                ActionErrorCode::Unavailable,
                "the world clock is unavailable",
            )
        })
}

impl InventoryProvider {
    fn parameters<'i>(
        &self,
        invocation: &'i ActionInvocation,
    ) -> Result<&'i serde_json::Map<String, Value>, ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(error(
                ActionErrorCode::InvalidRequest,
                "an inventory action is self-scoped and takes no targets",
            ));
        }
        invocation.parameters().as_object().ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                "inventory parameters must be an object",
            )
        })
    }

    /// The marker id one request's fact is stored and answered under.
    ///
    /// `{at}{request}`: the IC second the offer's FIXED parameters stamped,
    /// order-preserving in 16 hex bytes, then the request digest. E7c2 fold R3 —
    /// canonical key order is therefore **time** order, so the RAM mirror keeps
    /// the newest `MAX_APPLIED_MARKERS` rather than whichever request digests
    /// happened to sort first, and a committed transfer's fact stops answering
    /// unsatisfied for ever once the family passes its cap.
    ///
    /// It stays a pure function of durable causal input: the stamp rides the
    /// offer, so two replays of one request name the identical marker.
    fn marker_id(&self, invocation: &ActionInvocation, now: &ClockStamp) -> SmolStr {
        let ordered = (now.at as u64) ^ (1_u64 << 63);
        SmolStr::new(format!("{ordered:016x}{}", invocation.request().to_hex()))
    }

    #[allow(clippy::too_many_arguments)]
    fn write_marker(
        &self,
        txn: &mut ActionTxn<'_>,
        invocation: &ActionInvocation,
        kind: &str,
        from: Option<EndpointRef>,
        to: Option<EndpointRef>,
        commodity: &CanonicalId,
        quantity: u64,
        now: &ClockStamp,
    ) -> Result<(), ActionError> {
        let marker = AppliedMarker::new(
            kind,
            from,
            to,
            commodity.clone(),
            quantity,
            invocation.actor(),
            now.at,
        )
        .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        let id = self.marker_id(invocation, now);
        // E7c2 fold R1(a) / owner decision D1: **the marker IS the replay
        // guard**. It used to be `txn.replace`d, which made it a rewritable
        // mirror rather than evidence — and `InventoryVerb::Move` derives
        // nothing and consumes nothing, so a retry after a reboot (whose
        // `ScopedIdempotencyKey` carries a per-process random epoch and
        // therefore does not survive one) moved the quantity twice and rewrote
        // the same fact over it. The id is epoch-free and stable across a
        // reboot, so refusing an existing one closes the whole cut with one
        // branch. `Dissolve` alone writes none: its guard is the consumed lot.
        if txn
            .get(&self.families.applied, &id)
            .map_err(|error| state_error(&error))?
            .is_some()
        {
            return Err(error(
                ActionErrorCode::ProviderRejected,
                "that request has already been applied",
            ));
        }
        txn.insert(&self.families.applied, &id, marker)
            .map_err(|error| state_error(&error))
    }
}

impl ActionProvider for InventoryProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        self.parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let endpoints = live_endpoints(&engine.world, query.actor());
        if endpoints.is_empty() {
            // No live endpoint the actor may act on: no offer at all. A viewer
            // with no inventory sees exactly what a viewer with a frozen one
            // sees — nothing.
            return Ok(None);
        }
        let choices = endpoint_choices(&endpoints);
        let mut parameters = Vec::new();
        match self.verb {
            InventoryVerb::Move => {
                parameters.push(parameter(
                    "from",
                    true,
                    ProjectedParameterKind::Choice {
                        choices: choices.clone(),
                    },
                )?);
                parameters.push(parameter(
                    "to",
                    true,
                    ProjectedParameterKind::Choice { choices },
                )?);
                parameters.push(parameter(
                    "commodity",
                    true,
                    ProjectedParameterKind::Text {
                        maximum_bytes: MAX_COMMODITY_BYTES,
                    },
                )?);
                parameters.push(parameter(
                    "quantity",
                    true,
                    ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: i64::try_from(MAX_LOT_QUANTITY).unwrap_or(i64::MAX),
                    },
                )?);
            }
            InventoryVerb::Split => {
                parameters.push(parameter(
                    "endpoint",
                    true,
                    ProjectedParameterKind::Choice { choices },
                )?);
                parameters.push(parameter(
                    "lot",
                    true,
                    ProjectedParameterKind::Text { maximum_bytes: 32 },
                )?);
                parameters.push(parameter(
                    "quantity",
                    true,
                    ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: i64::try_from(MAX_LOT_QUANTITY).unwrap_or(i64::MAX),
                    },
                )?);
            }
            InventoryVerb::Merge => {
                parameters.push(parameter(
                    "endpoint",
                    true,
                    ProjectedParameterKind::Choice { choices },
                )?);
                parameters.push(parameter(
                    "lots",
                    true,
                    ProjectedParameterKind::Text {
                        maximum_bytes: MAX_LOT_LIST_BYTES,
                    },
                )?);
            }
            InventoryVerb::Dissolve => {
                parameters.push(parameter(
                    "endpoint",
                    true,
                    ProjectedParameterKind::Choice { choices },
                )?);
                parameters.push(parameter(
                    "lot",
                    true,
                    ProjectedParameterKind::Text { maximum_bytes: 32 },
                )?);
            }
            InventoryVerb::Crystallize => {
                parameters.push(parameter(
                    "endpoint",
                    true,
                    ProjectedParameterKind::Choice { choices },
                )?);
                parameters.push(parameter(
                    "commodity",
                    true,
                    ProjectedParameterKind::Text {
                        maximum_bytes: MAX_COMMODITY_BYTES,
                    },
                )?);
                parameters.push(parameter(
                    "quantity",
                    true,
                    ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: i64::try_from(MAX_LOT_QUANTITY).unwrap_or(i64::MAX),
                    },
                )?);
                parameters.push(parameter(
                    "grade",
                    true,
                    ProjectedParameterKind::Integer {
                        minimum: 0,
                        maximum: i64::from(MAX_GRADE),
                    },
                )?);
            }
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            error(
                ActionErrorCode::Integrity,
                "inventory definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "inventory guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "inventory definition guard is unavailable",
                )
            })?;
        let (label, help) = self.verb.label();
        Ok(Some(ProviderOfferDraft {
            // The IC stamp rides the offer's FIXED parameters, so preparation
            // never reaches for a clock it does not have and two replays of one
            // request stamp the identical marker.
            fixed_parameters: json!({ "now": engine.world.world_ic_now() }),
            parameters: ProjectedParameterSchema::new(parameters).map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "inventory parameter schema is invalid",
                )
            })?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(self.verb.verb(), label, help).map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "inventory presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "inventory offer guards are invalid",
                )
            })?,
        }))
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(error(
                ActionErrorCode::Integrity,
                "inventory receipt carries the wrong durable schema",
            ));
        }
        let actor = outcome
            .data()
            .get("actor")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .map(ObjId)
            .ok_or_else(|| {
                error(
                    ActionErrorCode::Integrity,
                    "inventory receipt names no actor",
                )
            })?;
        if viewer != &(PrincipalId::Object { id: actor }) {
            return Err(error(
                ActionErrorCode::Integrity,
                "inventory receipt disagrees with its eligible viewer",
            ));
        }
        let summary = outcome
            .data()
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("Done.")
            .to_owned();
        mush_proto::ProjectedActionResult::new(self.verb.id(), summary, outcome.data().clone())
            .map(Some)
            .map_err(|_| {
                error(
                    ActionErrorCode::BoundsExceeded,
                    "inventory receipt exceeds public bounds",
                )
            })
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters = self.parameters(invocation)?.clone();
        let now = prepare_now(&parameters)?;
        let actor = invocation.actor();
        let mut data = BTreeMap::<String, Value>::new();
        data.insert("actor".to_owned(), json!(actor.0));
        data.insert("verb".to_owned(), json!(self.verb.id()));

        let summary = match self.verb {
            InventoryVerb::Move => {
                let from = endpoint_field(&parameters, "from")?;
                let to = endpoint_field(&parameters, "to")?;
                let commodity = commodity_field(&parameters)?;
                let quantity = u64_field(&parameters, "quantity")?;
                let moved = self
                    .families
                    .transfer(txn, from, to, &commodity, quantity)
                    .map_err(|source| refuse(&source))?;
                // E7c2 fold C6: a same-endpoint move is a no-op HIT on purpose
                // (deviation (g)) and writes nothing — so it must publish no
                // `inventory.transferred` fact either. It used to commit a
                // marker naming a quantity that never moved, and
                // `FactProvider::resolve` returns only `{satisfied, actor, at}`,
                // so no consumer of D-E7-31's gate could tell the difference.
                if moved.moved > 0 {
                    self.write_marker(
                        txn,
                        invocation,
                        FACT_TRANSFERRED,
                        Some(from),
                        Some(to),
                        &commodity,
                        quantity,
                        &now,
                    )?;
                }
                data.insert("from".to_owned(), json!(from.vault().0));
                data.insert("to".to_owned(), json!(to.vault().0));
                data.insert("commodity".to_owned(), json!(commodity.as_str()));
                data.insert("quantity".to_owned(), json!(quantity));
                data.insert("balance".to_owned(), json!(moved.balance));
                data.insert("available".to_owned(), json!(moved.available));
                if from == to {
                    format!("{quantity} {commodity} was already there; nothing moved.")
                } else {
                    format!("Moved {quantity} {commodity} to {to}.")
                }
            }
            InventoryVerb::Split => {
                // E7c2 fold C1: the `endpoint` Choice the offer minted from the
                // actor's own live set is READ here, not decoration. Before the
                // fold the three lot arms never read it, so the list authorised
                // endpoint A while the mechanic mutated endpoint B.
                let endpoint = endpoint_field(&parameters, "endpoint")?;
                let lot = lot_field(&parameters, "lot")?;
                let quantity = u64_field(&parameters, "quantity")?;
                let child = crate::pool::derive_lot_id(invocation.request(), 0)
                    .map_err(|source| refuse(&source))?;
                let created = self
                    .families
                    .split(txn, endpoint, lot, quantity, child)
                    .map_err(|source| refuse(&source))?;
                // E7c2 fold C11: every verb that mints a lot publishes
                // `inventory.lot.created`, so a consumer waiting on a split-off
                // parcel does not wait for ever.
                self.write_marker(
                    txn,
                    invocation,
                    FACT_LOT_CREATED,
                    None,
                    Some(created.endpoint()),
                    created.commodity(),
                    created.quantity(),
                    &now,
                )?;
                data.insert("lot".to_owned(), json!(created.id().0));
                data.insert("parent".to_owned(), json!(lot.0));
                data.insert("quantity".to_owned(), json!(quantity));
                format!("Split {quantity} off {lot} into {}.", created.id())
            }
            InventoryVerb::Merge => {
                let endpoint = endpoint_field(&parameters, "endpoint")?;
                let lots = lots_field(&parameters)?;
                let child = crate::pool::derive_lot_id(invocation.request(), 0)
                    .map_err(|source| refuse(&source))?;
                let created = self
                    .families
                    .merge(txn, endpoint, &lots, child)
                    .map_err(|source| refuse(&source))?;
                self.write_marker(
                    txn,
                    invocation,
                    FACT_LOT_CREATED,
                    None,
                    Some(created.endpoint()),
                    created.commodity(),
                    created.quantity(),
                    &now,
                )?;
                data.insert("lot".to_owned(), json!(created.id().0));
                data.insert("quantity".to_owned(), json!(created.quantity()));
                data.insert(
                    "parents".to_owned(),
                    json!(lots.iter().map(|lot| lot.0).collect::<Vec<_>>()),
                );
                format!("Merged {} lots into {}.", lots.len(), created.id())
            }
            InventoryVerb::Dissolve => {
                let endpoint = endpoint_field(&parameters, "endpoint")?;
                let lot = lot_field(&parameters, "lot")?;
                let (tranche, moved) = self
                    .families
                    .dissolve(txn, endpoint, lot)
                    .map_err(|source| refuse(&source))?;
                data.insert("lot".to_owned(), json!(lot.0));
                data.insert("tranche".to_owned(), json!(tranche.0));
                data.insert("balance".to_owned(), json!(moved.balance));
                data.insert("available".to_owned(), json!(moved.available));
                format!("Dissolved {lot} into {tranche}.")
            }
            InventoryVerb::Crystallize => {
                let endpoint = endpoint_field(&parameters, "endpoint")?;
                let commodity = commodity_field(&parameters)?;
                let quantity = u64_field(&parameters, "quantity")?;
                let grade = parameters
                    .get("grade")
                    .and_then(Value::as_u64)
                    .and_then(|value| u8::try_from(value).ok())
                    .and_then(|value| Grade::new(value).ok())
                    .ok_or_else(|| {
                        error(
                            ActionErrorCode::InvalidRequest,
                            format!("a grade is 0 to {MAX_GRADE}"),
                        )
                    })?;
                let child = crate::pool::derive_lot_id(invocation.request(), 0)
                    .map_err(|source| refuse(&source))?;
                let (created, moved) = self
                    .families
                    .crystallize(txn, endpoint, &commodity, quantity, grade, child)
                    .map_err(|source| refuse(&source))?;
                self.write_marker(
                    txn,
                    invocation,
                    FACT_LOT_CREATED,
                    None,
                    Some(endpoint),
                    &commodity,
                    quantity,
                    &now,
                )?;
                data.insert("lot".to_owned(), json!(created.id().0));
                data.insert("endpoint".to_owned(), json!(endpoint.vault().0));
                data.insert("commodity".to_owned(), json!(commodity.as_str()));
                data.insert("quantity".to_owned(), json!(quantity));
                data.insert("balance".to_owned(), json!(moved.balance));
                data.insert("available".to_owned(), json!(moved.available));
                format!("Crystallized {quantity} {commodity} as {}.", created.id())
            }
        };
        data.insert("summary".to_owned(), json!(summary));
        let outcome = RulesValue::new(self.outcome.clone(), json!(data)).map_err(|_| {
            error(
                ActionErrorCode::BoundsExceeded,
                "inventory outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
        DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
        1,
        payload,
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(|error| ModError::msg(error.to_string()))
}

fn exact_validator(
    descriptors: &[DefinitionDescriptor],
) -> impl Fn(&DefinitionValidationContext<'_>) -> Result<(), String> + Send + Sync + 'static {
    let expected: BTreeMap<_, _> = descriptors
        .iter()
        .map(|descriptor| {
            let mut references = descriptor.references().to_vec();
            references.sort();
            (
                (descriptor.id().clone(), descriptor.version()),
                (
                    descriptor.payload().clone(),
                    references,
                    descriptor.is_current(),
                ),
            )
        })
        .collect();
    move |context| {
        let definition = context.definition();
        let key = (
            definition.reference().key().id.clone(),
            definition.reference().version(),
        );
        let Some((payload, references, current)) = expected.get(&key) else {
            return Err("unknown inventory definition".to_owned());
        };
        if definition.payload() != payload
            || definition.references() != references
            || definition.is_current() != *current
        {
            return Err("inventory definition shape mismatch".to_owned());
        }
        Ok(())
    }
}

/// Register the outcome, the five actions and their providers.
///
/// # Errors
/// [`ModError`] if any definition or provider registration is refused.
pub fn register(reg: &mut Registrar<'_>, families: &InventoryFamilies) -> Result<(), ModError> {
    let outcome_descriptor = descriptor(
        "outcome",
        OUTCOME_ID,
        json!({ "fields": ["actor", "verb", "summary"] }),
    )?;
    reg.definition_validator(
        DefinitionKind::new("outcome").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(std::slice::from_ref(&outcome_descriptor)),
    )?;
    let outcome = reg.definition(outcome_descriptor)?;

    let verbs = [
        InventoryVerb::Move,
        InventoryVerb::Split,
        InventoryVerb::Merge,
        InventoryVerb::Dissolve,
        InventoryVerb::Crystallize,
    ];
    let descriptors = verbs
        .iter()
        .map(|verb| {
            descriptor(
                "action",
                verb.id(),
                json!({ "subject": "self_actor", "targets": 0, "family": "inventory.pool" }),
            )
            .map(|descriptor| descriptor.with_references(vec![outcome.clone()]))
        })
        .collect::<Result<Vec<_>, ModError>>()?;
    reg.definition_validator(
        DefinitionKind::new("action").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(&descriptors),
    )?;
    for (verb, descriptor) in verbs.into_iter().zip(descriptors) {
        let action = reg.definition(descriptor)?;
        reg.action_provider(
            action.clone(),
            InventoryProvider {
                verb,
                action,
                outcome: outcome.clone(),
                families: families.clone(),
            },
        )?;
    }
    Ok(())
}

/// The registered `DefinitionRef` for one verb, rebuilt from the frozen
/// registry so a command handler can name it without holding a handle.
///
/// # Errors
/// `None` when the definition registry is not frozen or the id is unknown.
#[must_use]
pub fn action_ref(registry: &mush_core::Registry, verb: InventoryVerb) -> Option<DefinitionRef> {
    let definitions = registry.definitions()?;
    let id = DefinitionId::new(verb.id()).ok()?;
    let kind = DefinitionKind::new("action").ok()?;
    definitions
        .current(&mush_core::DefinitionKey {
            owner: crate::record::owner(),
            kind,
            id,
        })
        .cloned()
}

/// Whether an endpoint is one this actor may **read**, for the command layer's
/// **byte-identical** refusal (§4 rule 4): an endpoint the viewer may not read
/// and one that does not exist answer the same way.
///
/// E7c2 fold C8 / owner decision D4: this delegates to E3's own
/// `may_view_vault` — staff, the exact object-owner, or `vault.view` on the
/// vault's scope under `scope_coherent`. It used to be a predicate of cut 2's
/// own, which disagreed with `@vault/show` in both directions: a `vault.view`
/// grantee got `NO_SUCH_ENDPOINT` from `@inv/show`, and an org's money-posting
/// grantee got a read they had never been given.
///
/// The one addition is fold R7's: **staff see a quarantined endpoint** even when
/// the economy hydrated no vault for it, because otherwise the quantity stranded
/// there is invisible to the only people who could reconcile it.
#[must_use]
pub fn endpoint_visible(world: &mush_core::World, actor: ObjId, endpoint: EndpointRef) -> bool {
    if mush_economy::may_view_vault(world, actor, endpoint.vault()) {
        return true;
    }
    mush_core::perm::has_role(world, actor, mush_core::Role::Royalty)
        && world
            .module_state::<crate::store::InventoryStore>()
            .is_some_and(|store| store.is_quarantined(endpoint))
}

/// The endpoint's live state, for a command's own pre-check.
#[must_use]
pub fn endpoint_status(world: &mush_core::World, endpoint: EndpointRef) -> EndpointState {
    endpoint_state(world, endpoint.vault())
}

/// The tranche id one endpoint's commodity pools into at `generation`, for a
/// projection. The root is a sequence (fold B2), so a projection that wants the
/// current one asks `InventoryFamilies::tranche_for` inside a transaction.
///
/// # Errors
/// [`InventoryError::Identity`] if the canonical value cannot be hashed.
pub fn tranche_of(
    endpoint: EndpointRef,
    commodity: &CanonicalId,
    generation: u32,
) -> Result<TrancheId, InventoryError> {
    crate::pool::tranche_key(endpoint, commodity, generation)
}
