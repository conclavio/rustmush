//! **NPC consumption** (doc 48 unit EX.9; Q-48-19, Q-48-36, Q-48-40, Q-48-41).
//!
//! A port house with `consume` rows ([`ConsumeDef`]) uses up what it buys: Syrtis Bonded and the
//! Selk Townships Board their structural alloys and instrument boards (off-map construction
//! demand), the Lunar Habitat Board its provisions (its farm's own product, and the volatiles and
//! carbon feedstock it buys are the farm's inputs, which its process mill already uses up). Once
//! an IC day a row, the port's stock clerk submits **`tacsim_port_consume`**
//! (`ConsumeProvider`): the row's `per_ic_day` units (what the storefront holds, when less) are
//! debited from the house's storefront and the row's ledger ([`ConsumeLedger`], family
//! `tacsim.port_consume`) advances, in one bundle. The matter census reads the ledgers as its
//! `consumed` term.
//!
//! **A good the house asks** (the Habitat Board's provisions) backs the house's own standing
//! drawn ask, so the clerk first lowers that ask by the day's units (`mush_market::shrink_ask`,
//! the own-stock mill's draw, `crate::port::mill`), one act, and consumes them on a later fire.
//! The drift check and the readiness report read those units as drawn (`drawn`), so the gap
//! between the draw and the act never pages.
//!
//! **The export faucet pays the consumer back** (Q-48-22, Q-48-40;
//! [`crate::port::upkeep`]'s faucet): for each act, the good's authored price for the units used,
//! at most the row's `export_major_per_ic_day`, from the System account `tacsim.export` to the
//! house's account. Money enters the lane only as goods are used up, so a consumer's treasury
//! holds steady at its rate of consumption, and the faucet's money reaches players only through
//! the consumers' bids.
//!
//! **The restart argument** is the mill's: the ledger's `day` is the once-a-day guard and its
//! ordinal the replay guard, both durable and written in the act's own bundle, and the drawn units
//! are read off durable state every fire, so a restart never consumes twice in a day or lowers an
//! ask twice. The RAM pass-over (`PortTickState::consume_refused`) passes a row over for the rest
//! of the IC day once its act is submitted or refused.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalHash, CanonicalId, Ctx, DefinitionRef, IdempotencyKeyText, ModError, ObjId,
    PerceivedRef, ProjectedParameter, ProjectedParameterKind, ProjectedParameterSchema,
    ProjectedTargetSchema, ProviderOfferDraft, Registrar, RevisionSet, RulesValue,
    StateFamilyHandle, SubjectKey, World,
};
use mush_economy::VaultId;
use mush_inventory::InventoryFamilies;
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::port::def::{ConsumeDef, PortDef, VendorDef};
use crate::port::map::PortMap;
use crate::port::stock::{IC_DAY, MAX_CLERK_ROWS, clerk_port, swapped};
use crate::port_tick::{Fire, PortTickState, Step, port_error};

/// The act's action id (doc 48 unit EX.9).
pub const CONSUME_ACTION: &str = "tacsim_port_consume";
/// The act's outcome id.
pub const CONSUME_OUTCOME: &str = "tacsim_port_consume_result";
/// The consumption-ledger family: one [`ConsumeLedger`] per row.
pub const CONSUME_FAMILY: &str = "tacsim.port_consume";
/// An act's idempotency-key prefix: `tacsim.port.consume/<key text>/<ordinal>`.
pub const CONSUME_KEY_PREFIX: &str = "tacsim.port.consume/";
/// How many of its last acts a ledger keeps, its durable audit tail (the mill's sixteen).
pub const MAX_RECENT_ACTS: usize = 16;
/// The consumption identity's hash domain.
const CONSUME_ID_DOMAIN: &str = "tacsim.consume_id.v1";
/// The canonical-bytes ceiling of one consumption identity's input.
const CONSUME_ID_MAX_BYTES: usize = 1024;

static KEY_MISMATCH_LOGGED: AtomicBool = AtomicBool::new(false);

// ---- the identity and the ledger ---------------------------------------------------

/// **A consumption row's identity**: `hash(pack, entry, org, good)`, the mill id's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConsumeId(CanonicalHash);

impl ConsumeId {
    /// `CanonicalHash::for_canonical_value("tacsim.consume_id.v1", 1, {pack, entry, org, good},
    /// 1024)`.
    ///
    /// # Errors
    ///
    /// The hash's own refusal, as text (it cannot refuse four canonical ids).
    pub fn derive(
        pack: &CanonicalId,
        entry: &CanonicalId,
        org: &CanonicalId,
        good: &CanonicalId,
    ) -> Result<ConsumeId, String> {
        CanonicalHash::for_canonical_value(
            CONSUME_ID_DOMAIN,
            1,
            &json!({
                "pack": pack.as_str(),
                "entry": entry.as_str(),
                "org": org.as_str(),
                "good": good.as_str(),
            }),
            CONSUME_ID_MAX_BYTES,
        )
        .map(ConsumeId)
        .map_err(|error| error.to_string())
    }

    /// The key text: 64 lowercase hexadecimal digits.
    #[must_use]
    pub fn key_text(&self) -> String {
        self.0.to_hex()
    }

    /// The strict inverse of [`ConsumeId::key_text`].
    #[must_use]
    pub fn parse(text: &str) -> Option<ConsumeId> {
        CanonicalHash::from_hex(text).ok().map(ConsumeId)
    }
}

/// **One act, as its ledger's audit tail keeps it.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumeAct {
    /// The act's ordinal, from 1.
    pub ordinal: u64,
    /// The IC instant it landed at.
    pub at_ic: i64,
    /// Units used up.
    pub units: u64,
}

/// **One consumption row's durable receipt**, replaced in every act's own bundle by
/// `tacsim_port_consume`: the total the census reads, the IC day of the last act (the
/// once-a-day guard) and the last sixteen acts (the faucet's refunds read them).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumeLedger {
    /// The pack the row was authored in.
    pub pack: CanonicalId,
    /// The port entry.
    pub entry: CanonicalId,
    /// The consuming house.
    pub org: CanonicalId,
    /// The good used up.
    pub good: CanonicalId,
    /// The storefront vault it is used up from.
    pub vault: VaultId,
    /// The stock clerk the tick acted as.
    pub clerk: ObjId,
    /// Acts landed, `>= 1`.
    pub acts: u64,
    /// Units used up in all.
    pub units: u64,
    /// The IC day of the last act.
    pub day: i64,
    /// The last acts, oldest first, at most [`MAX_RECENT_ACTS`].
    pub recent: Vec<ConsumeAct>,
}

impl ConsumeLedger {
    /// This ledger's id, re-derived from `(pack, entry, org, good)`.
    ///
    /// # Errors
    ///
    /// As [`ConsumeId::derive`].
    pub fn consume_id(&self) -> Result<ConsumeId, String> {
        ConsumeId::derive(&self.pack, &self.entry, &self.org, &self.good)
    }

    /// The ledger's invariants: at least one act; the tail non-empty and at most
    /// [`MAX_RECENT_ACTS`], its ordinals rising to `acts`, each act's units within the total; and
    /// `day` the last act's IC day.
    ///
    /// # Errors
    ///
    /// The first invariant broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.acts == 0 {
            return Err("a consumption ledger records at least one act".to_owned());
        }
        let (Some(first), Some(last)) = (self.recent.first(), self.recent.last()) else {
            return Err("a consumption ledger keeps its last act".to_owned());
        };
        if self.recent.len() > MAX_RECENT_ACTS {
            return Err(format!(
                "a consumption ledger keeps at most {MAX_RECENT_ACTS} acts"
            ));
        }
        if last.ordinal != self.acts || first.ordinal == 0 {
            return Err("a consumption ledger's tail ends at its last act".to_owned());
        }
        if self
            .recent
            .windows(2)
            .any(|pair| pair[1].ordinal != pair[0].ordinal.saturating_add(1))
        {
            return Err("a consumption ledger's tail runs in order".to_owned());
        }
        if self.recent.iter().any(|act| act.units > self.units) {
            return Err("a consumption ledger's act exceeds its total".to_owned());
        }
        if last.at_ic.div_euclid(IC_DAY) != self.day {
            return Err("a consumption ledger's day is its last act's".to_owned());
        }
        Ok(())
    }
}

/// **The module's handles**: the registered action, when its definition is on the roll, and the
/// ledger family.
#[derive(Clone)]
pub struct ConsumeHandles {
    /// The registered [`CONSUME_ACTION`].
    pub action: Option<DefinitionRef>,
    /// [`CONSUME_FAMILY`].
    pub ledgers: StateFamilyHandle<ConsumeId, ConsumeLedger>,
}

/// **One ledger, read through the family's typed-state lens**, its key re-checked. `Err` when the
/// lens refuses or the ledger names another row: fail closed, logged once a process.
pub(crate) fn ledger_at(
    ctx: &Ctx<'_>,
    handle: &StateFamilyHandle<ConsumeId, ConsumeLedger>,
    id: &ConsumeId,
) -> Result<Option<ConsumeLedger>, ()> {
    match ctx.action_state(handle, id).map_err(|_| ())? {
        None => Ok(None),
        Some(ledger) if ledger.consume_id().ok().as_ref() == Some(id) => Ok(Some(ledger.clone())),
        Some(_) => {
            if !KEY_MISMATCH_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::error!(
                    key = %id.key_text(),
                    "tacsim: a consumption ledger is stored under another row's key; it reads \
                     as unreadable (fail closed)"
                );
            }
            Err(())
        }
    }
}

/// **Every consumption row of the laid-up ports with its ledger**, for the census and the faucet:
/// `(map, vendor def, row, ledger)`, rows whose ledger cannot be read skipped.
pub(crate) fn ledgers(ctx: &Ctx<'_>) -> Vec<(PortMap, VendorDef, ConsumeDef, ConsumeLedger)> {
    let Some(handles) = ctx.w.module_state::<ConsumeHandles>().cloned() else {
        return Vec::new();
    };
    let ports: Vec<(PortDef, PortMap)> = crate::port::market::ports(ctx.w)
        .into_iter()
        .filter_map(|(_, def, map)| Some((def.clone(), map?.clone())))
        .collect();
    let mut out = Vec::new();
    for (def, map) in ports {
        for vendor in &def.vendors {
            for row in &vendor.consume {
                let Ok(id) = ConsumeId::derive(&map.pack, &map.entry, &vendor.org, &row.good)
                else {
                    continue;
                };
                if let Ok(Some(ledger)) = ledger_at(ctx, &handles.ledgers, &id) {
                    out.push((map.clone(), vendor.clone(), row.clone(), ledger));
                }
            }
        }
    }
    out
}

// ---- registration ---------------------------------------------------------------------

/// Outcome descriptor for [`CONSUME_OUTCOME`].
pub(crate) fn outcome_descriptor() -> Result<mush_core::DefinitionDescriptor, ModError> {
    crate::port_tick::descriptor(
        "outcome",
        CONSUME_OUTCOME,
        json!({"format":1,"fields":["actor","consume","ordinal","units"]}),
    )
}

/// Action descriptor for [`CONSUME_ACTION`].
pub(crate) fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<mush_core::DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(CONSUME_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the consumption outcome did not register"))?;
    crate::port_tick::descriptor(
        "action",
        CONSUME_ACTION,
        json!({"format":1,"subject":"self_actor","targets":0,"family":CONSUME_FAMILY}),
    )
    .map(|descriptor| descriptor.with_references(vec![outcome]))
}

/// Register [`CONSUME_FAMILY`], the provider of [`CONSUME_ACTION`] and [`ConsumeHandles`]. The
/// family and the handles register regardless; the provider when the action's definition is on
/// the roll and the inventory cell is filled (its prepare debits through the delegated families).
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    inventory: Option<&InventoryFamilies>,
) -> Result<(), ModError> {
    let ledgers = reg.state_family::<ConsumeId, ConsumeLedger, _, _, _, _>(
        crate::port::families::family_descriptor(
            CONSUME_FAMILY,
            "tacsim.port_consume_key.v1",
            "tacsim.port_consume",
        )?,
        |key: &ConsumeId| Ok(key.key_text()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &ConsumeLedger| value.check(),
        |value: &ConsumeLedger| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let action = match (actions.get(CONSUME_ACTION).cloned(), inventory) {
        (Some(action), Some(inventory)) => {
            let outcome = outcomes
                .get(CONSUME_OUTCOME)
                .cloned()
                .ok_or_else(|| ModError::msg("the consumption outcome did not register"))?;
            reg.action_provider(
                action.clone(),
                ConsumeProvider {
                    action: action.clone(),
                    outcome,
                    ledgers: ledgers.clone(),
                    inventory: inventory.clone(),
                },
            )?;
            Some(action)
        }
        _ => None,
    };
    reg.state(ConsumeHandles { action, ledgers });
    Ok(())
}

// ---- the provider -------------------------------------------------------------------

/// **`tacsim_port_consume`**: one IC day's use of one row's good, debited at the storefront, and
/// the row's ledger advanced exactly once per ordinal and once per IC day, in one bundle.
pub(crate) struct ConsumeProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub ledgers: StateFamilyHandle<ConsumeId, ConsumeLedger>,
    pub inventory: InventoryFamilies,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "consumption is self-scoped, takes no targets and names one row, its next act and the \
         units",
    )
}

fn text<'p>(entry: &'p Map<String, Value>, key: &str) -> Result<&'p str, mush_core::ActionError> {
    entry.get(key).and_then(Value::as_str).ok_or_else(invalid)
}

fn number(entry: &Map<String, Value>, key: &str) -> Result<u64, mush_core::ActionError> {
    entry.get(key).and_then(Value::as_u64).ok_or_else(invalid)
}

fn canonical(entry: &Map<String, Value>, key: &str) -> Result<CanonicalId, mush_core::ActionError> {
    CanonicalId::new(text(entry, key)?).map_err(|_| invalid())
}

/// The invocation's parameters, checked for shape: self-scoped, no targets, an object naming one
/// `consume` row, an `ordinal` and `units`, both at least 1.
fn parameters(
    invocation: &ActionInvocation,
) -> Result<&Map<String, Value>, mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    let parameters = invocation.parameters().as_object().ok_or_else(invalid)?;
    text(parameters, "consume")?;
    for key in ["ordinal", "units"] {
        if parameters
            .get(key)
            .and_then(Value::as_u64)
            .is_none_or(|n| n == 0)
        {
            return Err(invalid());
        }
    }
    Ok(parameters)
}

/// **The clerk's consumption rows as the offer's fixed parameters** (the stock offer's lead ruling
/// L4): every authored row of the clerk's port with a vendor map row, keyed by its [`ConsumeId`]
/// key text, at most [`MAX_CLERK_ROWS`]. Derived from authoritative state at projection.
fn consume_rows(w: &World, clerk: ObjId) -> Map<String, Value> {
    let mut rows: BTreeMap<String, Value> = BTreeMap::new();
    let Some((map, def)) = clerk_port(w, clerk) else {
        return Map::new();
    };
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        for consume in &vendor.consume {
            let Ok(id) = ConsumeId::derive(&map.pack, &map.entry, &vendor.org, &consume.good)
            else {
                continue;
            };
            rows.insert(
                id.key_text(),
                json!({
                    "pack": map.pack.as_str(),
                    "entry": map.entry.as_str(),
                    "org": vendor.org.as_str(),
                    "good": consume.good.as_str(),
                    "per_ic_day": consume.per_ic_day,
                    "vault": row.vault,
                    "clerk": clerk.0,
                }),
            );
        }
    }
    rows.into_iter().take(MAX_CLERK_ROWS).collect()
}

impl ActionProvider for ConsumeProvider {
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
    ) -> Result<Option<ProviderOfferDraft>, mush_core::ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let rows = consume_rows(&engine.world, query.actor());
        if rows.is_empty() {
            return Ok(None);
        }
        let choices: Vec<String> = rows.keys().cloned().collect();
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the consumption's definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the consumption's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the consumption's definition guard is unavailable"))?;
        let id = |name: &'static str| {
            CanonicalId::new(name)
                .map_err(|_| integrity("the consumption's parameter id is invalid"))
        };
        let count = |name: &'static str| -> Result<ProjectedParameter, mush_core::ActionError> {
            Ok(ProjectedParameter {
                id: id(name)?,
                required: true,
                kind: ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(u32::MAX),
                },
            })
        };
        let consume = ProjectedParameter {
            id: id("consume")?,
            required: true,
            kind: ProjectedParameterKind::Choice { choices },
        };
        Ok(Some(ProviderOfferDraft {
            // The prepare has no world, so the IC instant rides the server-authenticated offer
            // beside the rows (the mill's rule).
            fixed_parameters: json!({
                "rows": Value::Object(rows),
                "now": engine.world.world_ic_now().at,
            }),
            parameters: ProjectedParameterSchema::new(vec![
                consume,
                count("ordinal")?,
                count("units")?,
            ])
            .map_err(|_| integrity("the consumption's parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-port-consume",
                "Use up goods",
                "A port's stock clerk uses up one IC day of what its house consumes.",
            )
            .map_err(|_| integrity("the consumption's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the consumption's offer guards are invalid"))?,
        }))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        // 1. The parameters as validated.
        let parameters = parameters(invocation)?;
        let key = text(parameters, "consume")?;
        let ordinal = number(parameters, "ordinal")?;
        let asked = number(parameters, "units")?;
        let now = parameters
            .get("now")
            .and_then(Value::as_i64)
            .ok_or_else(invalid)?;
        // 2. The server-authenticated row; an absent one is not this clerk's.
        let row = parameters
            .get("rows")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(key))
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        let pack = canonical(row, "pack")?;
        let port = canonical(row, "entry")?;
        let org = canonical(row, "org")?;
        let good = canonical(row, "good")?;
        let per_ic_day = number(row, "per_ic_day")?;
        let vault = number(row, "vault")?;
        let clerk = u32::try_from(number(row, "clerk")?).map_err(|_| invalid())?;
        // 3. The row is the acting clerk's.
        let actor = invocation.actor();
        if clerk != actor.0 {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that consumption row is another clerk's",
            ));
        }
        // 4. The key re-derived from the row itself.
        let id = ConsumeId::derive(&pack, &port, &org, &good).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the consumption identity cannot be derived",
            )
        })?;
        if id.key_text() != key {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "the consumption row does not name its own row",
            ));
        }
        // 5. The ledger, its key re-checked.
        let prior = txn
            .get(&self.ledgers, &id)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the consumption ledgers are unavailable",
                )
            })?
            .cloned();
        if let Some(ledger) = &prior
            && ledger.consume_id().ok().as_ref() != Some(&id)
        {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "a consumption ledger is stored under another row's key",
            ));
        }
        // 6. The replay guard: only the ledger's next ordinal, and once an IC day.
        let next = prior.as_ref().map_or(Some(1), |l| l.acts.checked_add(1));
        if next != Some(ordinal) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that consumption is already landed",
            ));
        }
        let today = now.div_euclid(IC_DAY);
        if prior.as_ref().is_some_and(|ledger| ledger.day >= today) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that row has already consumed today",
            ));
        }
        // 7. The units: what was asked, at most the day's, at most what the storefront holds.
        let endpoint = EndpointRef(VaultId(vault));
        let available = self
            .inventory
            .available(txn, endpoint, &good)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::ProviderRejected,
                    "the consumed good cannot be read",
                )
            })?;
        let units = asked.min(per_ic_day).min(available);
        if units == 0 {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "nothing is there to consume",
            ));
        }
        // 8. The debit.
        self.inventory
            .debit(txn, endpoint, &good, units)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::ProviderRejected,
                    "the consumed good could not be debited",
                )
            })?;
        // 9. The next ledger.
        let overflow = || {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the consumption ledger's total overflows",
            )
        };
        let (total, mut recent) = match &prior {
            Some(ledger) => (
                ledger.units.checked_add(units).ok_or_else(overflow)?,
                ledger.recent.clone(),
            ),
            None => (units, Vec::new()),
        };
        recent.push(ConsumeAct {
            ordinal,
            at_ic: now,
            units,
        });
        let excess = recent.len().saturating_sub(MAX_RECENT_ACTS);
        recent.drain(..excess);
        let ledger = ConsumeLedger {
            pack,
            entry: port,
            org,
            good,
            vault: VaultId(vault),
            clerk: actor,
            acts: ordinal,
            units: total,
            day: today,
            recent,
        };
        ledger.check().map_err(|_| {
            port_error(
                ActionErrorCode::ProviderRejected,
                "the consumption ledger is out of bounds",
            )
        })?;
        let staged = if prior.is_some() {
            txn.replace(&self.ledgers, &id, ledger)
        } else {
            txn.insert(&self.ledgers, &id, ledger)
        };
        staged.map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the consumption ledger did not stage",
            )
        })?;
        // 10. The outcome, witnessed by the principal.
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "actor": actor.0, "consume": key, "ordinal": ordinal, "units": units }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the consumption's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

// ---- drawn and not yet consumed ------------------------------------------------------

/// **The units the drift check and the Listings line read as drawn for consumption**: for a good
/// `vendor` asks and consumes, the part of `net` (the storefront's units of it, less what its
/// mills made and the ask does not carry yet, less what its own-stock mills drew) over the
/// standing listing and its Draws not yet drawn, up to one IC day's use. `0` with no listing and
/// for any other good. Pure.
#[must_use]
pub(crate) fn drawn(
    vendor: &VendorDef,
    good: &CanonicalId,
    net: u64,
    listed: Option<u64>,
    undrawn: u64,
) -> u64 {
    let Some(listed) = listed else {
        return 0;
    };
    let allowance = vendor
        .consume
        .iter()
        .filter(|row| &row.good == good && vendor.asks.iter().any(|ask| ask.good == row.good))
        .map(|row| row.per_ic_day)
        .fold(0, u64::saturating_add);
    net.saturating_sub(listed.saturating_add(undrawn))
        .min(allowance)
}

// ---- the clerk's step ------------------------------------------------------------------

/// **Whether `(vault, good)`'s consumption was passed over today.**
fn refused_today(state: &PortTickState, vault: u64, good: &CanonicalId, today: i64) -> bool {
    state
        .consume_refused
        .get(&(vault, good.clone()))
        .is_some_and(|day| *day == today)
}

/// Pass `(vault, good)`'s consumption over for the rest of `today` (stale days pruned first, so
/// the bound holds the live refusals).
fn refuse_today(state: &mut PortTickState, vault: u64, good: &CanonicalId, today: i64) {
    state.consume_refused.retain(|_, day| *day == today);
    let key = (vault, good.clone());
    if state.consume_refused.contains_key(&key) || state.consume_refused.len() < MAX_CLERK_ROWS {
        state.consume_refused.insert(key, today);
    }
}

/// **What an asked good's consumption comes to this fire.**
enum Asked {
    /// The ask is down by these units, not yet consumed.
    Consume(u64),
    /// The clerk lowered the ask (or tried to): this fire's one act.
    Acted(Step),
    /// Nothing is due this fire.
    Wait,
}

/// **An asked good's draw** (the own-stock mill's, `crate::port::mill`): the house's units back
/// its own standing drawn ask, so the clerk first lowers the ask by what the day still needs and
/// consumes on a later fire. No listing, or a Draw quote unsettled on the pool: nothing (the list
/// step and the market's draw window are the mill's waits).
#[allow(clippy::too_many_arguments)]
fn asked_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    clerk: ObjId,
    map: &PortMap,
    vendor: &VendorDef,
    row: &crate::port::map::VendorRow,
    consume: &ConsumeDef,
    today: i64,
) -> Asked {
    let endpoint = EndpointRef(VaultId(row.vault));
    let Ok(available) = crate::port_tick::read_available(ctx, endpoint, &consume.good) else {
        return Asked::Wait;
    };
    let mills = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .map(|handles| handles.families.mills.clone());
    let unlisted = match mills {
        Some(mills) => {
            match crate::port::mill::unlisted(ctx, &mills, map, vendor, row, &consume.good) {
                Ok(unlisted) => unlisted,
                Err(_) => return Asked::Wait,
            }
        }
        None => 0,
    };
    let reading = ctx
        .w
        .module_state::<MarketStore>()
        .map(|market| crate::port::stock::ask_reading(market, row.vault, &consume.good))
        .unwrap_or_default();
    if reading.in_flight {
        return Asked::Wait;
    }
    let Some((listing, listed)) = reading.listing else {
        return Asked::Wait;
    };
    let net = available.saturating_sub(unlisted);
    let mill_drawn =
        crate::port::mill::drawn(vendor, &consume.good, net, Some(listed), reading.undrawn);
    let excess = net
        .saturating_sub(mill_drawn)
        .saturating_sub(listed.saturating_add(reading.undrawn));
    if excess >= consume.per_ic_day {
        return Asked::Consume(excess);
    }
    // Lower the ask by what the day still needs, leaving the listing standing with one unit at
    // least; an ask already down to one consumes what is drawn.
    let need = consume
        .per_ic_day
        .saturating_sub(excess)
        .min(listed.saturating_sub(1));
    if need == 0 {
        return if excess > 0 {
            Asked::Consume(excess)
        } else {
            Asked::Wait
        };
    }
    let live = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|market| market.listing(listing))
        .is_some_and(|standing| !mush_market::listing_lapsed(standing, ctx.w.now()));
    if !live {
        return Asked::Wait;
    }
    let spec = mush_market::RestockSpec {
        listing,
        good: consume.good.clone(),
        units: need,
    };
    Asked::Acted(
        match fire.as_principal(ctx, clerk, clerk, |ctx| mush_market::shrink_ask(ctx, spec)) {
            Ok(Ok(_)) => Step::Worked,
            Ok(Err(error)) => {
                tracing::warn!(
                    vault = row.vault, good = %consume.good, ?error,
                    "tacsim: the stock clerk could not draw a consumption off its house's ask"
                );
                refuse_today(state, row.vault, &consume.good, today);
                Step::Refused
            }
            Err(refused) => swapped(Err(refused)),
        },
    )
}

/// **The consumption's turn of the stock arm** (after the mill, before the bids): the first row
/// not yet consumed today whose storefront holds its good submits `tacsim_port_consume` as the
/// clerk for the ledger's next ordinal (an asked good is drawn off its ask first). One act a fire.
/// `None` when nothing is due.
pub(crate) fn consume_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    clerk: ObjId,
    map: &PortMap,
    def: &PortDef,
) -> Option<Step> {
    if def.vendors.iter().all(|vendor| vendor.consume.is_empty()) {
        return None;
    }
    let handles = ctx.w.module_state::<ConsumeHandles>().cloned()?;
    let action = handles.action.clone()?;
    let today = ctx.w.world_ic_now().at.div_euclid(IC_DAY);
    // A port stocked today stands at its authored stock until the day turns.
    if !crate::port::upkeep::consumes(ctx.w, &map.entry, today) {
        return None;
    }
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        for consume in &vendor.consume {
            if refused_today(state, row.vault, &consume.good, today) {
                continue;
            }
            let Ok(id) = ConsumeId::derive(&map.pack, &map.entry, &vendor.org, &consume.good)
            else {
                continue;
            };
            let Ok(ledger) = ledger_at(ctx, &handles.ledgers, &id) else {
                continue;
            };
            if ledger.as_ref().is_some_and(|ledger| ledger.day >= today) {
                continue;
            }
            let asked = vendor.asks.iter().any(|ask| ask.good == consume.good);
            let units = if asked {
                match asked_step(ctx, state, fire, clerk, map, vendor, row, consume, today) {
                    Asked::Consume(units) => units,
                    Asked::Acted(step) => return Some(step),
                    Asked::Wait => continue,
                }
            } else {
                match crate::port_tick::read_available(
                    ctx,
                    EndpointRef(VaultId(row.vault)),
                    &consume.good,
                ) {
                    Ok(units) if units > 0 => units,
                    _ => continue,
                }
            };
            let units = units.min(consume.per_ic_day);
            if ctx.authoritative_pending(clerk) {
                return Some(Step::Deferred);
            }
            let Some(ordinal) = ledger
                .as_ref()
                .map_or(Some(1), |ledger| ledger.acts.checked_add(1))
            else {
                continue;
            };
            let key_text = id.key_text();
            let Ok(key) = IdempotencyKeyText::parse_canonical(format!(
                "{CONSUME_KEY_PREFIX}{key_text}/{ordinal}"
            )) else {
                return Some(Step::Refused);
            };
            let parameters = json!({ "consume": key_text, "ordinal": ordinal, "units": units });
            let step = swapped(fire.as_principal(ctx, clerk, clerk, |ctx| {
                ctx.submit_authoritative(
                    action.clone(),
                    PerceivedRef::SelfActor,
                    Vec::new(),
                    parameters,
                    key,
                );
            }));
            // Submitted, the act lands or is refused; either way the row is done for the day, so a
            // refused act never takes the clerk's every fire.
            if matches!(step, Step::Submitted | Step::Refused) {
                refuse_today(state, row.vault, &consume.good, today);
            }
            return Some(step);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::def::AskDef;

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap()
    }

    fn ledger(acts: &[(u64, i64, u64)]) -> ConsumeLedger {
        let recent: Vec<ConsumeAct> = acts
            .iter()
            .map(|(ordinal, at_ic, units)| ConsumeAct {
                ordinal: *ordinal,
                at_ic: *at_ic,
                units: *units,
            })
            .collect();
        let last = recent.last().copied().unwrap();
        ConsumeLedger {
            pack: cid("tacsim.lane.ceres_phobos"),
            entry: cid("luna_port"),
            org: cid("habitat_board"),
            good: cid("provisions"),
            vault: VaultId(7),
            clerk: ObjId(9),
            acts: last.ordinal,
            units: recent.iter().map(|act| act.units).sum(),
            day: last.at_ic.div_euclid(IC_DAY),
            recent,
        }
    }

    /// The ledger's invariants hold for a run of days and refuse a broken tail.
    #[test]
    fn ex9_the_consumption_ledger_checks_its_tail() {
        let good = ledger(&[(1, 10, 16), (2, IC_DAY + 10, 16)]);
        assert_eq!(good.check(), Ok(()));
        assert_eq!(good.consume_id().unwrap(), good.consume_id().unwrap());
        let mut gap = good.clone();
        gap.recent[1].ordinal = 3;
        gap.acts = 3;
        assert!(gap.check().is_err());
        let mut day = good.clone();
        day.day = 0;
        assert!(day.check().is_err());
        let mut none = good;
        none.recent.clear();
        assert!(none.check().is_err());
        let wire = serde_json::to_value(ledger(&[(1, 10, 16)])).unwrap();
        let back: ConsumeLedger = serde_json::from_value(wire).unwrap();
        assert_eq!(back, ledger(&[(1, 10, 16)]));
    }

    /// The drawn allowance is one day's use of an asked good, over the listing and its Draws, and
    /// nothing for a good bought by a bid or with no listing.
    #[test]
    fn ex9_drawn_reads_one_days_use_of_an_asked_good() {
        let vendor = VendorDef {
            org: cid("habitat_board"),
            storefront: "neighborhood.aitken_highport.item.habitat_manifold".to_owned(),
            agent: cid("luna_port.clerk"),
            signatory: "neighborhood.aitken_highport.npc.albrecht_sato".to_owned(),
            stock: Vec::new(),
            asks: vec![AskDef {
                good: cid("provisions"),
                price_major: 10,
                boards: Vec::new(),
            }],
            bids: Vec::new(),
            mill: Vec::new(),
            consume: vec![
                ConsumeDef {
                    good: cid("provisions"),
                    per_ic_day: 16,
                    export_major_per_ic_day: 160,
                },
                ConsumeDef {
                    good: cid("volatiles"),
                    per_ic_day: 1,
                    export_major_per_ic_day: 0,
                },
            ],
        };
        let provisions = cid("provisions");
        assert_eq!(drawn(&vendor, &provisions, 1_000, Some(990), 0), 10);
        assert_eq!(drawn(&vendor, &provisions, 1_000, Some(900), 50), 16);
        assert_eq!(drawn(&vendor, &provisions, 1_000, None, 0), 0);
        assert_eq!(drawn(&vendor, &cid("volatiles"), 1_000, Some(900), 0), 0);
    }
}
