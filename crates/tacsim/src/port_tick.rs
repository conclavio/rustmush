//! **The port tick** (doc 18 cut IC5; §4.3 "Finite seeded stock", §4.4 "The
//! queued-orders model" and "Offline execution", D-18-32, D-18-33).
//!
//! `tacsim.port` is the one pull tick every port arm runs on (Coarse, 5 s). It
//! obeys the tick-cadence rule (`mush-campaign/src/tick.rs:28-37`): a tick runs
//! as GOD and submits nothing authoritative itself, because two GOD submissions
//! in one drain void each other. So every authoritative act this tick makes is
//! made by `Fire::as_principal`, which swaps `ctx.actor` to the principal an
//! agent acts for (the `mush-npc` precedent) for ONE closed call, and counts that
//! call as the principal's one act this fire (D-18-32: at most one authoritative
//! submission per actor per fire). **Nothing in the tick calls
//! `ctx.submit_authoritative` outside a closure passed to `as_principal`.** GOD,
//! the tick's own actor, therefore never submits.
//!
//! The walk is the campaign rotation (`mush-campaign/src/tick.rs:94-157`): a
//! cursor over agent ids, at most [`crate::port_tick::MAX_AGENTS_VISITED_PER_FIRE`] visits and
//! [`crate::port_tick::MAX_AGENTS_WORKED_PER_FIRE`] units of work per fire, the cursor moved past
//! the last visited agent. Fairness between agents sharing one principal is a
//! RAM debt per principal (lead ruling L8, `arbitrate`).
//!
//! The arms it carries, in dispatch order ahead of the probe:
//!
//! - IC8 — **the stock arm** (each port's stock clerk: `tacsim_port_stock` mint +
//!   receipt, the dissolve into the storefront pool, the Draw listing) and **the
//!   lapsed-hold release** (a player's lapsed lot listing released as its holder,
//!   at most one per fire, the holder told once after the pull verification):
//!   [`crate::port::stock`];
//! - IC9 — **the launch arm** (the ticket's consumer): one agent per hull with a
//!   filed passage, acting as nobody — the launch is a direct `mush_sim::depart`
//!   placement write, no submission and no swap — with its pull verification and
//!   the held-launch backoff ([`crate::port::voyage`]); the dock is mush-sim's own
//!   arrival (`Direct` on the port's passive domain, D-18-18);
//! - IC11 — **the order arm** (the queue's offline execution, D-18-32): one agent
//!   per hull whose queued sailing plan reads and is due, acting as the plan's
//!   author for ONE closed typed call per step (`mush_market::quote` +
//!   `accept_at`, `sell_into_bid`, the leg re-stages, or the `tacsim_port_burn`
//!   submission), with its pull verification at the top of every fire
//!   ([`crate::port::executor`]); a hull with a filed passage is the launch arm's.
//! - doc 23 cut S.2 — **the epoch arm** (the ship's seal epoch, a pull reconciliation): one
//!   agent per programme hull whose epoch open or close is due (agent = the hull, principal =
//!   the epoch's warrant principal), submitting ONE `tacsim_voyage_epoch` as that principal
//!   ([`crate::port::epoch`]); for the fire it takes the hull from the launch, recall and order
//!   arms, so a close lands before the hull can leave again.
//!
//! **Doc 20 cut SM.3b** registers the surface layer's two providers here too, beside the
//! burn and under the crate's one `outcome` and one `action` validator:
//! `tacsim_surface_act` (every rig act's start) and `tacsim_surface_done` (every act's
//! finish, submitted by the `tacsim.surface` tick as the operator), each only when the
//! inventory cell is filled and the tools table built (`crate::surface::act`); and, at the
//! end of [`crate::port_tick::register`], the surface tick itself (`crate::surface::tick`).
//! **Doc 20 cut SM.4a** hands both the chart family and adds `+tv/forget`'s own action,
//! `tacsim_chart_forget` (`crate::surface::chart`), on every roster.
//!
//! **The probe** (lead ruling L3) is registered only in a test world: an agent
//! that acts as its principal once per remaining count, submitting
//! `tacsim_port_probe`, whose provider writes one mark into the test-only family
//! `tacsim.port_probe`. Production composes the families, the stock provider and
//! its definitions, the tick and the notice hook (lead rulings L4, L12).
//!
//! **Reconnect notices** live here too (§4.4 "Failure handling", lead ruling
//! L10): [`crate::port_tick::notify`] tells a connected player at once and otherwise queues a line
//! in a bounded per-player document (`tacsim:notice:{obj}`), and
//! [`crate::port_tick::ReconnectNotices`] renders the queue once on `Connected` under
//! [`crate::port_tick::NOTICE_HEADING`], then deletes it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar, TickCadence};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalId, Ctx, DefinitionDescriptor, DefinitionId,
    DefinitionKind, DefinitionRef, EventHook, Flags, GOD, HookPoint, IdempotencyKeyText,
    ModuleActionStateReadError, ObjId, ObjKind, PerceivedRef, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    RevisionSet, RulesValue, SceneId, StateFamilyHandle, SubjectKey, World,
};
use mush_inventory::{InventoryFamilies, InventoryHandleCell};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::port::families::{self, PortFamilies, PortHandles};
use crate::store::{TacsimStore, push_op};

/// The tick's tag.
pub const PORT_TICK: &str = "tacsim.port";
/// How often it fires (Coarse).
pub const PORT_TICK_PERIOD: Duration = Duration::from_secs(5);
/// Most agents one fire visits (the campaign visit budget).
pub const MAX_AGENTS_VISITED_PER_FIRE: usize = 64;
/// Most agents one fire works (the campaign work budget).
pub const MAX_AGENTS_WORKED_PER_FIRE: usize = 16;
/// Most probe agents a test world may enlist.
pub const MAX_PROBE_AGENTS: usize = 256;

/// The probe's action id (test world only).
pub const PROBE_ACTION: &str = "tacsim_port_probe";
/// The probe's outcome id (test world only).
pub const PROBE_OUTCOME: &str = "tacsim_port_probe_result";
/// The probe's family (test world only).
pub const PROBE_FAMILY: &str = "tacsim.port_probe";

/// The reconnect-notice document prefix (`tacsim:notice:{obj.0}`).
pub const NOTICE_PREFIX: &str = "tacsim:notice:";
/// The reconnect-notice document's schema.
pub const NOTICE_SCHEMA: u16 = 1;
/// Most lines one character's queue keeps (oldest dropped).
pub const MAX_NOTICE_LINES: usize = 32;
/// Most bytes one queued line keeps (cut on a char boundary).
pub const MAX_NOTICE_BYTES: usize = 240;
/// The line a reconnect render opens with.
pub const NOTICE_HEADING: &str = "While you were off watch:";

// ---- registration ------------------------------------------------------------

/// Register the port tick (doc 18 cut IC5): the port's typed families, the crate's
/// one `outcome` and one `action` validator, the `tacsim_port_stock` provider (doc 18
/// cut IC8), the `tacsim_port_burn` provider (cut IC9) and doc 20 cut SM.12a's
/// `tacsim_port_mill` provider — each when the inventory cell is filled (and the
/// burn's drive table built), their definitions regardless —
/// in a test world the probe (family, definitions and provider), [`PortHandles`],
/// [`PortTickState`], the `tacsim.port` tick and the [`ReconnectNotices`] hook. Cut
/// IC11's order arm registers no provider of its own: it runs the burn and the
/// market's closed calls as the plan's author.
///
/// The order is the inventory provider's interleaved one
/// (`mush-inventory/src/provider.rs:891-966`): an action descriptor must
/// reference its outcome's exact `DefinitionRef`, which exists only once the
/// outcome is registered.
///
/// # Errors
///
/// Any registration the registrar refuses.
pub fn register(
    reg: &mut Registrar<'_>,
    test_world: bool,
    inventory: Option<&InventoryHandleCell>,
    burn: Option<crate::port::burn::BurnTable>,
) -> Result<(), ModError> {
    register_with_ic(reg, test_world, inventory, burn, None)
}
pub(crate) fn register_with_ic(
    reg: &mut Registrar<'_>,
    test_world: bool,
    inventory: Option<&InventoryHandleCell>,
    burn: Option<crate::port::burn::BurnTable>,
    ic: Option<&crate::minds::ic_sortie::registration::Registration>,
) -> Result<(), ModError> {
    register_with_coverage(reg, test_world, inventory, burn, ic, None, None, false)
}
// One optional composition each (the IC sortie, the coverage, the Registry inspection) and the
// unit-authority switch: the merge of doc 17h cut M8a's inspection with main's detachments
// brought the count to eight; bundling them would only move the list.
#[allow(clippy::too_many_arguments)]
pub(crate) fn register_with_coverage(
    reg: &mut Registrar<'_>,
    test_world: bool,
    inventory: Option<&InventoryHandleCell>,
    burn: Option<crate::port::burn::BurnTable>,
    ic: Option<&crate::minds::ic_sortie::registration::Registration>,
    coverage: Option<&crate::underwriting::registration::Registration>,
    inspection: Option<&crate::minds::registry_inspection::Composition>,
    unit_authority: bool,
) -> Result<(), ModError> {
    let port_families = families::register(reg)?;
    reg.revision_validator(
        crate::surface::recovery::revision_kind()
            .map_err(|error| ModError::msg(error.to_string()))?,
        crate::surface::recovery::RecoveryRevisionValidator {
            excursions: port_families.excursions.clone(),
        },
    )?;
    crate::surface::survey::register(reg, port_families.surveys.clone())?;
    crate::surface::survey_facts::register(
        reg,
        port_families.surveys.clone(),
        inventory.is_some(),
    )?;
    crate::port::passenger::store::register(
        reg,
        port_families.passengers.clone(),
        port_families.passenger_index.clone(),
    )?;
    crate::port::passenger::facts::register(
        reg,
        port_families.passengers.clone(),
        inventory.is_some(),
    )?;
    crate::quest::store::register(
        reg,
        port_families.quests.clone(),
        port_families.quest_index.clone(),
        port_families.passengers.clone(),
        port_families.passenger_index.clone(),
        port_families.quest_survey_evidence.clone(),
        port_families.surveys.clone(),
    )?;
    let probe = if test_world {
        Some(reg.state_family::<ProbeKey, ProbeMark, _, _, _, _>(
            families::family_descriptor(
                PROBE_FAMILY,
                "tacsim.port_probe_key.v1",
                "tacsim.port_probe",
            )?,
            |key: &ProbeKey| Ok(key.key_text()),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &ProbeMark| value.check(),
            |value: &ProbeMark| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?)
    } else {
        None
    };
    let outcome_refs = register_outcomes(reg, test_world, ic, coverage, inspection)?;
    let probe_outcome = outcome_refs.get(PROBE_OUTCOME).cloned();
    let stock_outcome = outcome_refs
        .get(crate::port::stock::STOCK_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the stock outcome did not register"))?;
    // Doc 20 cut SM.12a: the mill's batch.
    let mill_outcome = outcome_refs
        .get(crate::port::mill::MILL_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the mill outcome did not register"))?;
    let burn_outcome = outcome_refs
        .get(crate::port::burn::BURN_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the burn outcome did not register"))?;
    // One `action` validator for the whole crate, likewise.
    let surface_act_outcome = outcome_refs
        .get(crate::surface::act::SURFACE_ACT_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the surface act's outcome did not register"))?;
    let surface_done_outcome = outcome_refs
        .get(crate::surface::act::SURFACE_DONE_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the surface finish's outcome did not register"))?;
    let recovery_outcome = outcome_refs
        .get(crate::surface::recovery::RECOVERY_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the staff recovery outcome did not register"))?;
    let chart_forget_outcome = outcome_refs
        .get(crate::surface::chart::CHART_FORGET_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the chart forget's outcome did not register"))?;
    let site_work_outcome = outcome_refs
        .get(crate::port::site::SITE_WORK_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the site work's outcome did not register"))?;
    // Doc 20 cut SM.12b: the yard's survey and delivery.
    let dispatch_outcome = outcome_refs
        .get(crate::port::yard::DISPATCH_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the dispatch outcome did not register"))?;
    let relay_outcome = outcome_refs
        .get(crate::relay_foundry::OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("relay fabrication outcome missing"))?;
    let survey_outcome = outcome_refs
        .get(crate::surface::survey::SURVEY_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("survey acceptance outcome missing"))?;
    let survey_cancel_outcome = outcome_refs
        .get(crate::surface::survey::SURVEY_CANCEL_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("survey cancellation outcome missing"))?;
    // Doc 23 cut S.2: the ship's epoch.
    let epoch_outcome = outcome_refs
        .get(crate::port::epoch::EPOCH_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the ship's epoch outcome did not register"))?;
    let passenger_outcome = outcome_refs
        .get(crate::port::passenger::store::OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("passenger outcome missing"))?;
    let mut actions = action_descriptors(
        (&stock_outcome, &mill_outcome),
        &relay_outcome,
        &burn_outcome,
        (&surface_act_outcome, &surface_done_outcome),
        &chart_forget_outcome,
        (&site_work_outcome, &dispatch_outcome),
        probe_outcome.as_ref(),
    )?;
    append_receipt_actions(
        &mut actions,
        &recovery_outcome,
        &survey_outcome,
        &survey_cancel_outcome,
        (&passenger_outcome, &epoch_outcome),
    )?;
    actions.push(crate::quest::registration::action(&outcome_refs)?);
    actions.push(crate::detachment::planning::action_descriptor(
        &outcome_refs,
    )?);
    actions.push(crate::procurement::action_descriptor(&outcome_refs)?);
    if let Some(ic) = ic {
        actions.extend(ic.actions()?);
    }
    if let Some(coverage) = coverage {
        actions.extend(coverage.descriptors()?);
    }
    // Doc 17h cut M8a: the Registry inspection's action, under this crate's owner (and the
    // fine's, the principal revocation's and the quirk claim's when they compose).
    if let Some(inspection) = inspection {
        inspection_descriptors(inspection, 0, &mut actions)?;
    }
    reg.definition_validator(
        kind("action")?,
        crate::definitions::exact_validator(actions.clone()),
    )?;
    let mut action_refs: BTreeMap<String, DefinitionRef> = BTreeMap::new();
    for descriptor in actions {
        let id = descriptor.id().as_str().to_owned();
        action_refs.insert(id, reg.definition(descriptor)?);
    }
    // Doc 17h cut M8a: the Registry inspection's provider and slot over the registered
    // action and outcome (`minds::registry_inspection`); doc 17h cut M8d D3 the fine's beside
    // them, in a frame of their own.
    if let Some(inspection) = inspection {
        register_inspection(reg, inspection, &action_refs, &outcome_refs)?;
    }
    let probe_action = action_refs.get(PROBE_ACTION).cloned();
    if let (Some(action), Some(outcome), Some(marks)) =
        (probe_action.clone(), probe_outcome, probe.clone())
    {
        reg.action_provider(
            action.clone(),
            ProbeProvider {
                action,
                outcome,
                marks,
            },
        )?;
    }
    // Inventory registers before this layer on every roster that hands a cell,
    // so the cell is filled here; an unfilled cell gives `None` and every read
    // then refuses (`PoolReadError::Unwired`).
    let inventory_families = inventory.and_then(|cell| cell.get().cloned());
    let AgreementActions {
        passenger,
        survey_accept,
        survey_cancel,
    } = register_agreement_providers(
        reg,
        &action_refs,
        (passenger_outcome, survey_outcome, survey_cancel_outcome),
        &port_families,
    )?;
    // Doc 18 cut IC8: `tacsim_port_stock`, tacsim's first production provider —
    // registered only when the inventory cell is filled (its prepare mints through
    // the delegated inventory families); the definitions register regardless.
    let stock_action = action_refs.get(crate::port::stock::STOCK_ACTION).cloned();
    register_stock(
        reg,
        stock_action.as_ref(),
        stock_outcome,
        &port_families,
        inventory_families.as_ref(),
    )?;
    // Doc 18 cut IC9: `tacsim_port_burn` — registered only when the inventory cell
    // is filled (its prepare debits through the delegated families) AND the drive
    // table built (the stock precedent); the definitions register regardless.
    let burn_registered = register_burn(
        reg,
        &action_refs,
        burn_outcome,
        &port_families,
        (inventory_families.as_ref(), burn),
    )?;
    // Doc 20 cut SM.3b (finding F5): the one tools table, built once — never a `ModError`
    // (the burn precedent: a bad table leaves one feature unwired, it does not fail the
    // layer's registration).
    let surface_tools = match crate::definitions::tools::tools_table() {
        Ok(table) => Some(Arc::new(table)),
        Err(error) => {
            tracing::error!(
                %error,
                "tacsim: the tools table does not build; the rig acts are not registered (every \
                 +tv act answers TV_UNWIRED on this server)"
            );
            None
        }
    };
    // Doc 20 cut SM.3b: `tacsim_surface_act` and `tacsim_surface_done` — registered only when
    // the inventory cell is filled (the dump moves pools through the delegated families) and
    // the tools table built; the definitions register regardless.
    // Doc 23 cut S.1 (§7, L2): the black box's owner participant, for the finish and the staff
    // recovery the two calls below register.
    // Doc 23 cut S.2: and for the ship's epoch, on every roster.
    let black_box = register_black_box(
        reg,
        &action_refs,
        inventory_families.is_some(),
        surface_tools.is_some(),
    )?;
    let (surface_act, surface_done) = register_surface_acts(
        reg,
        &action_refs,
        (surface_act_outcome, surface_done_outcome),
        &port_families,
        inventory_families.as_ref(),
        (surface_tools.as_ref(), black_box.done),
    )?;
    // Staff recovery is a separate capability from the player's surface acts.
    let surface_recovery = register_surface_recovery(
        reg,
        &action_refs,
        recovery_outcome,
        &port_families,
        (inventory_families.as_ref(), black_box.recovery),
    )?;
    // Doc 23 cut S.2 (V1): `tacsim_voyage_epoch`, the ship's epoch — its record's family, no
    // inventory; the black box's participant appends the line.
    let epoch_action = register_epoch(
        reg,
        &action_refs,
        epoch_outcome,
        &port_families,
        black_box.epoch,
    )?;
    // Doc 20 cut SM.4a (D9): `+tv/forget` needs only the chart family.
    let chart_forget = match action_refs
        .get(crate::surface::chart::CHART_FORGET_ACTION)
        .cloned()
    {
        Some(action) => {
            reg.action_provider(
                action.clone(),
                crate::surface::chart::ChartForgetProvider {
                    action: action.clone(),
                    outcome: chart_forget_outcome,
                    charts: port_families.charts.clone(),
                },
            )?;
            Some(action)
        }
        None => None,
    };
    // Doc 20 cut SM.6 (§3.4): `tacsim_site_work`, the site clerk's worked transfer —
    // registered when the inventory cell is filled (the transfer rides the delegated
    // families; the stock provider's rule); the definitions register regardless.
    let site_work = register_site_work(
        reg,
        &action_refs,
        site_work_outcome,
        &port_families,
        inventory_families.as_ref(),
    )?;
    let relay_fabricate = register_relay(
        reg,
        &action_refs,
        relay_outcome,
        &port_families,
        inventory_families.as_ref(),
    )?;
    // Doc 20 cut SM.12a: `tacsim_port_mill`, the mill's batch — registered only when the
    // inventory cell is filled (its prepare debits and credits through the delegated
    // families; the stock provider's rule); the definitions register regardless.
    let mill_action = register_mill(
        reg,
        &action_refs,
        mill_outcome,
        &port_families,
        inventory_families.as_ref(),
    )?;
    // Doc 20 cut SM.12b (§12.3): `tacsim_dispatch`, the house's survey and the yard's delivery —
    // registered only when the inventory cell is filled (the delivery's transfer rides the
    // delegated families; the stock provider's rule); the definitions register regardless.
    let dispatch_action = register_dispatch(
        reg,
        &action_refs,
        dispatch_outcome,
        &port_families,
        inventory_families.as_ref(),
    )?;
    let quest =
        crate::quest::registration::provider(reg, &action_refs, &outcome_refs, &port_families)?;
    crate::detachment::planning::register(reg, &action_refs, &outcome_refs, unit_authority)?;
    // Doc 17h cut M8f: the procurement ledger, its record action and the `tacsim.procurement`
    // tick, unconditionally (no policy, no visit).
    crate::procurement::register(reg, &action_refs, &outcome_refs)?;
    // Doc 17h cut M8h: the `tacsim.house_attestation` tick and its RAM slot, unconditionally
    // (no agent, no visit).
    crate::house_attestation::register(reg)?;
    reg.state(PortHandles {
        relay_fabricate,
        families: port_families,
        inventory: inventory_families,
        probe,
        burn: burn_registered,
        surface_act,
        surface_done,
        surface_recovery,
        surface_tools,
        chart_forget,
        survey_accept,
        survey_cancel,
        passenger,
        quest,
        site_work,
        dispatch: dispatch_action.clone(),
    });
    reg.state(PortTickState::new(
        probe_action,
        stock_action,
        (mill_action, epoch_action),
        dispatch_action,
    ));
    reg.tick(
        PORT_TICK,
        TickCadence::Coarse(PORT_TICK_PERIOD),
        Arc::new(port_tick),
    )?;
    reg.hook(HookPoint::Connected, Arc::new(ReconnectNotices));
    // Doc 20 cut SM.3b: the `tacsim.surface` tick and its slots.
    crate::surface::tick::register(reg)?;
    Ok(())
}

/// Build each outcome once for validation and registration, keeping descriptor
/// construction temporaries out of the main port registration's stack frame.
fn register_outcomes(
    reg: &mut Registrar<'_>,
    test_world: bool,
    ic: Option<&crate::minds::ic_sortie::registration::Registration>,
    coverage: Option<&crate::underwriting::registration::Registration>,
    inspection: Option<&crate::minds::registry_inspection::Composition>,
) -> Result<BTreeMap<String, DefinitionRef>, ModError> {
    let mut outcomes = outcome_descriptors(test_world)?;
    outcomes.push(crate::detachment::planning::outcome_descriptor()?);
    outcomes.push(crate::procurement::outcome_descriptor()?);
    if let Some(ic) = ic {
        outcomes.push(ic.outcome()?);
    }
    if let Some(coverage) = coverage {
        outcomes.push(coverage.outcome()?);
    }
    // Doc 17h cut M8a: the Registry inspection's outcome (and the fine's, the principal
    // revocation's and the quirk claim's when they compose, doc 17h cut M8g G3).
    if let Some(inspection) = inspection {
        inspection_descriptors(inspection, 1, &mut outcomes)?;
    }
    reg.definition_validator(
        kind("outcome")?,
        crate::definitions::exact_validator(outcomes.clone()),
    )?;
    let mut outcome_refs = BTreeMap::new();
    for descriptor in outcomes {
        let id = descriptor.id().as_str().to_owned();
        outcome_refs.insert(id, reg.definition(descriptor)?);
    }
    Ok(outcome_refs)
}

/// Keep descriptor construction temporaries out of the main registration frame.
/// These receipts follow the base actions and precede IC/coverage extensions.
fn append_receipt_actions(
    actions: &mut Vec<DefinitionDescriptor>,
    recovery: &DefinitionRef,
    survey_accept: &DefinitionRef,
    survey_cancel: &DefinitionRef,
    (passenger, epoch): (&DefinitionRef, &DefinitionRef),
) -> Result<(), ModError> {
    actions.push(
        descriptor(
            "action",
            crate::surface::recovery::RECOVERY_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": crate::surface::excursion::EXCURSION_FAMILY,
            }),
        )?
        .with_references(vec![recovery.clone()]),
    );
    actions.push(descriptor(
        "action",
        crate::surface::survey::SURVEY_ACTION,
        json!({"subject":"self_actor","targets":0,"family":crate::surface::survey::SURVEY_FAMILY}),
    )?.with_references(vec![survey_accept.clone()]));
    actions.push(descriptor(
        "action",
        crate::surface::survey::SURVEY_CANCEL_ACTION,
        json!({"subject":"self_actor","targets":0,"family":crate::surface::survey::SURVEY_FAMILY}),
    )?.with_references(vec![survey_cancel.clone()]));
    actions.push(descriptor("action", crate::port::passenger::store::ACTION,
        json!({"subject":"self_actor","targets":0,"family":crate::port::passenger::store::FAMILY}))?.with_references(vec![passenger.clone()]));
    // Doc 23 cut S.2: the ship's epoch writes its `tacsim.ship_epoch` record; its black box line
    // rides the black box's participant.
    actions.push(
        descriptor(
            "action",
            crate::port::epoch::EPOCH_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": crate::port::epoch::EPOCH_FAMILY,
            }),
        )?
        .with_references(vec![epoch.clone()]),
    );
    Ok(())
}

struct AgreementActions {
    passenger: Option<DefinitionRef>,
    survey_accept: Option<DefinitionRef>,
    survey_cancel: Option<DefinitionRef>,
}

/// Register agreement providers in their original passenger/accept/cancel order,
/// constructing each provider in this frame rather than retaining its temporaries
/// beside every other port provider.
fn register_agreement_providers(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: (DefinitionRef, DefinitionRef, DefinitionRef),
    families: &PortFamilies,
) -> Result<AgreementActions, ModError> {
    let passenger = actions.get(crate::port::passenger::store::ACTION).cloned();
    if let Some(action) = &passenger {
        reg.action_provider(
            action.clone(),
            crate::port::passenger::store::Provider {
                action: action.clone(),
                outcome: outcomes.0,
                bookings: families.passengers.clone(),
                index: families.passenger_index.clone(),
            },
        )?;
    }
    let survey_accept = actions.get(crate::surface::survey::SURVEY_ACTION).cloned();
    if let Some(action) = &survey_accept {
        reg.action_provider(
            action.clone(),
            crate::surface::survey::SurveyProvider {
                action: action.clone(),
                outcome: outcomes.1,
                surveys: families.surveys.clone(),
            },
        )?;
    }
    let survey_cancel = actions
        .get(crate::surface::survey::SURVEY_CANCEL_ACTION)
        .cloned();
    if let Some(action) = &survey_cancel {
        reg.action_provider(
            action.clone(),
            crate::surface::survey::CancelProvider {
                action: action.clone(),
                outcome: outcomes.2,
                surveys: families.surveys.clone(),
            },
        )?;
    }
    Ok(AgreementActions {
        passenger,
        survey_accept,
        survey_cancel,
    })
}

/// Build the paired providers in their own frame: each carries several typed
/// family handles, which must not accumulate on the main registration stack.
fn register_surface_acts(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: (DefinitionRef, DefinitionRef),
    families: &PortFamilies,
    inventory: Option<&InventoryFamilies>,
    (tools, black_box): (Option<&SurfaceTools>, Option<BlackBoxHandle>),
) -> Result<(Option<DefinitionRef>, Option<DefinitionRef>), ModError> {
    let (Some(act), Some(done), Some(inventory), Some(tools)) = (
        actions.get(crate::surface::act::SURFACE_ACT_ACTION),
        actions.get(crate::surface::act::SURFACE_DONE_ACTION),
        inventory,
        tools,
    ) else {
        return Ok((None, None));
    };
    reg.action_provider(
        act.clone(),
        crate::surface::act::SurfaceActProvider {
            action: act.clone(),
            outcome: outcomes.0,
            excursions: families.excursions.clone(),
            charts: families.charts.clone(),
            pockets: families.pockets.clone(),
            inventory: inventory.clone(),
            tools: Arc::clone(tools),
        },
    )?;
    reg.action_provider(
        done.clone(),
        crate::surface::act::SurfaceDoneProvider {
            action: done.clone(),
            outcome: outcomes.1,
            excursions: families.excursions.clone(),
            charts: families.charts.clone(),
            surveys: families.surveys.clone(),
            quest_survey_evidence: families.quest_survey_evidence.clone(),
            pockets: families.pockets.clone(),
            inventory: inventory.clone(),
            tools: Arc::clone(tools),
            black_box,
        },
    )?;
    Ok((Some(act.clone()), Some(done.clone())))
}

/// The players' surface tools table the finish reads.
type SurfaceTools = Arc<BTreeMap<CanonicalId, crate::definitions::tools::ToolDef>>;

/// The black box participant's handle for one action (doc 23 cut S.1).
type BlackBoxHandle =
    mush_core::participant::ParticipantHandle<crate::surface::black_box::BlackBoxParticipant>;

/// The black box participant's handles, one an action that asks for it.
struct BlackBoxHandles {
    /// `tacsim_surface_done`'s.
    done: Option<BlackBoxHandle>,
    /// `tacsim_surface_recovery`'s.
    recovery: Option<BlackBoxHandle>,
    /// Doc 23 cut S.2: `tacsim_voyage_epoch`'s.
    epoch: Option<BlackBoxHandle>,
}

/// **The black box's owner participant** (doc 23 cut S.1; §7; the lead's L2): its definition
/// and its kind's one validator, registered unconditionally (a definition is a fact about the
/// crate); the participant itself, on the store slot, for exactly the actions the calls after
/// it register — the finish (`tacsim_surface_done`: the inventory cell and the tools table
/// filled), the staff recovery (the inventory cell filled) and, doc 23 cut S.2, the ship's epoch
/// (`tacsim_voyage_epoch`, every roster) — and one handle each.
fn register_black_box(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    inventory: bool,
    tools: bool,
) -> Result<BlackBoxHandles, ModError> {
    use crate::surface::black_box::{
        BlackBoxParticipant, PARTICIPANT_KIND, participant_descriptor,
    };
    let descriptor = participant_descriptor()?;
    reg.definition_validator(
        kind(PARTICIPANT_KIND)?,
        crate::definitions::exact_validator(vec![descriptor.clone()]),
    )?;
    let definition = reg.definition(descriptor)?;
    let done = actions
        .get(crate::surface::act::SURFACE_DONE_ACTION)
        .filter(|_| {
            inventory && tools && actions.contains_key(crate::surface::act::SURFACE_ACT_ACTION)
        })
        .cloned();
    let recovery = actions
        .get(crate::surface::recovery::RECOVERY_ACTION)
        .filter(|_| inventory)
        .cloned();
    let epoch = actions.get(crate::port::epoch::EPOCH_ACTION).cloned();
    let allowed: BTreeSet<DefinitionRef> = done
        .iter()
        .chain(recovery.iter())
        .chain(epoch.iter())
        .cloned()
        .collect();
    if allowed.is_empty() {
        return Ok(BlackBoxHandles {
            done: None,
            recovery: None,
            epoch: None,
        });
    }
    reg.owner_participant(definition, allowed, BlackBoxParticipant)?;
    let mut request = |action: Option<DefinitionRef>| {
        action
            .map(|action| reg.request_owner_participant::<BlackBoxParticipant>(action))
            .transpose()
    };
    Ok(BlackBoxHandles {
        done: request(done)?,
        recovery: request(recovery)?,
        epoch: request(epoch)?,
    })
}

/// **The ship's epoch action** (doc 23 cut S.2, V1): `tacsim_voyage_epoch` over the epoch
/// records' family, the black box's participant appending the line; registered only with the
/// action in the roster and the participant's handle for it. Its own function so the provider's
/// construction is not one more temporary on [`register_with_coverage`]'s frame (the M7
/// memory-size limits).
fn register_epoch(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcome: DefinitionRef,
    families: &PortFamilies,
    black_box: Option<BlackBoxHandle>,
) -> Result<Option<DefinitionRef>, ModError> {
    let (Some(action), Some(handle)) = (actions.get(crate::port::epoch::EPOCH_ACTION), black_box)
    else {
        return Ok(None);
    };
    reg.action_provider(
        action.clone(),
        crate::port::epoch::EpochProvider {
            action: action.clone(),
            outcome,
            records: families.epochs.clone(),
            black_box: handle,
        },
    )?;
    Ok(Some(action.clone()))
}

// The six providers below each carry the delegated inventory families (a handful of typed
// handles) beside their own: built in their own frames, as `register_surface_acts` is, so the
// clones never accumulate on `register_with_coverage`'s stack (the M7 memory-size limits; doc 23
// cut S.2's epoch arm was the last straw).

/// Doc 18 cut IC8: `tacsim_port_stock`, tacsim's first production provider — registered only
/// when the inventory cell is filled (its prepare mints through the delegated inventory
/// families); the definition registers regardless.
fn register_stock(
    reg: &mut Registrar<'_>,
    action: Option<&DefinitionRef>,
    outcome: DefinitionRef,
    families: &PortFamilies,
    inventory: Option<&InventoryFamilies>,
) -> Result<(), ModError> {
    let (Some(action), Some(inventory)) = (action, inventory) else {
        return Ok(());
    };
    reg.action_provider(
        action.clone(),
        crate::port::stock::StockProvider {
            action: action.clone(),
            outcome,
            receipts: families.port_stock.clone(),
            inventory: inventory.clone(),
        },
    )
}

/// Doc 18 cut IC9: `tacsim_port_burn` — registered only when the inventory cell is filled (its
/// prepare debits through the delegated families) AND the drive table built (the stock
/// precedent); the definition registers regardless. The action, when registered.
fn register_burn(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcome: DefinitionRef,
    families: &PortFamilies,
    (inventory, table): (
        Option<&InventoryFamilies>,
        Option<crate::port::burn::BurnTable>,
    ),
) -> Result<Option<DefinitionRef>, ModError> {
    let (Some(action), Some(inventory), Some(table)) = (
        actions.get(crate::port::burn::BURN_ACTION),
        inventory,
        table,
    ) else {
        return Ok(None);
    };
    reg.action_provider(
        action.clone(),
        crate::port::burn::BurnProvider {
            action: action.clone(),
            outcome,
            tickets: families.tickets.clone(),
            inventory: inventory.clone(),
            table,
        },
    )?;
    Ok(Some(action.clone()))
}

/// Doc 20 cut SM.6 (§3.4): `tacsim_site_work`, the site clerk's worked transfer — registered
/// when the inventory cell is filled (the transfer rides the delegated families; the stock
/// provider's rule); the definition registers regardless. The action, when registered.
fn register_site_work(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcome: DefinitionRef,
    families: &PortFamilies,
    inventory: Option<&InventoryFamilies>,
) -> Result<Option<DefinitionRef>, ModError> {
    let (Some(action), Some(inventory)) =
        (actions.get(crate::port::site::SITE_WORK_ACTION), inventory)
    else {
        return Ok(None);
    };
    reg.action_provider(
        action.clone(),
        crate::port::site::SiteWorkProvider {
            action: action.clone(),
            outcome,
            pockets: families.pockets.clone(),
            inventory: inventory.clone(),
            warned: std::sync::Mutex::new(BTreeSet::new()),
        },
    )?;
    Ok(Some(action.clone()))
}

/// The relay foundry's fabrication — registered when the inventory cell is filled; the action,
/// when registered.
fn register_relay(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcome: DefinitionRef,
    families: &PortFamilies,
    inventory: Option<&InventoryFamilies>,
) -> Result<Option<DefinitionRef>, ModError> {
    let (Some(action), Some(inventory)) = (actions.get(crate::relay_foundry::ACTION), inventory)
    else {
        return Ok(None);
    };
    reg.action_provider(
        action.clone(),
        crate::relay_foundry::Provider {
            action: action.clone(),
            outcome,
            inventory: inventory.clone(),
            book: families.relay_fabrications.clone(),
        },
    )?;
    Ok(Some(action.clone()))
}

/// Doc 20 cut SM.12a: `tacsim_port_mill`, the mill's batch — registered only when the inventory
/// cell is filled (its prepare debits and credits through the delegated families; the stock
/// provider's rule); the definition registers regardless. The action, when registered.
fn register_mill(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcome: DefinitionRef,
    families: &PortFamilies,
    inventory: Option<&InventoryFamilies>,
) -> Result<Option<DefinitionRef>, ModError> {
    let (Some(action), Some(inventory)) = (actions.get(crate::port::mill::MILL_ACTION), inventory)
    else {
        return Ok(None);
    };
    reg.action_provider(
        action.clone(),
        crate::port::mill::MillProvider {
            action: action.clone(),
            outcome,
            mills: families.mills.clone(),
            inventory: inventory.clone(),
        },
    )?;
    Ok(Some(action.clone()))
}

/// Doc 20 cut SM.12b (§12.3): `tacsim_dispatch`, the house's survey and the yard's delivery —
/// registered only when the inventory cell is filled (the delivery's transfer rides the
/// delegated families; the stock provider's rule); the definition registers regardless. The
/// action, when registered.
fn register_dispatch(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcome: DefinitionRef,
    families: &PortFamilies,
    inventory: Option<&InventoryFamilies>,
) -> Result<Option<DefinitionRef>, ModError> {
    let (Some(action), Some(inventory)) =
        (actions.get(crate::port::yard::DISPATCH_ACTION), inventory)
    else {
        return Ok(None);
    };
    reg.action_provider(
        action.clone(),
        crate::port::yard::DispatchProvider {
            action: action.clone(),
            outcome,
            dispatch: families.dispatch.clone(),
            charts: families.charts.clone(),
            claims: families.claims.clone(),
            inventory: inventory.clone(),
        },
    )?;
    Ok(Some(action.clone()))
}

/// Recovery needs inventory but does not depend on the player's tools table.
fn register_surface_recovery(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcome: DefinitionRef,
    families: &PortFamilies,
    (inventory, black_box): (Option<&InventoryFamilies>, Option<BlackBoxHandle>),
) -> Result<Option<DefinitionRef>, ModError> {
    let (Some(action), Some(inventory)) = (
        actions.get(crate::surface::recovery::RECOVERY_ACTION),
        inventory,
    ) else {
        return Ok(None);
    };
    reg.action_provider(
        action.clone(),
        crate::surface::recovery::RecoveryProvider {
            action: action.clone(),
            outcome,
            excursions: families.excursions.clone(),
            pockets: families.pockets.clone(),
            inventory: inventory.clone(),
            black_box,
        },
    )?;
    Ok(Some(action.clone()))
}

fn kind(word: &str) -> Result<DefinitionKind, ModError> {
    DefinitionKind::new(word).map_err(|error| ModError::msg(error.to_string()))
}

/// One current version-1 definition descriptor (`b1_seam`'s helper).
fn descriptor(kind_word: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        kind(kind_word)?,
        DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
        1,
        payload,
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(|error| ModError::msg(error.to_string()))
}

/// **Every `outcome` definition this crate registers** — doc 18 cut IC8's
/// `tacsim_port_stock_result`, doc 20 cut SM.12a's `tacsim_port_mill_result`, cut IC9's
/// `tacsim_port_burn_result` and doc 20 cut SM.3b's
/// `tacsim_surface_act_result` and `tacsim_surface_done_result` on every roster, and the
/// probe's in a test world: a second `definition_validator` call
/// for one kind is a registration conflict, so there is exactly one validator per
/// kind for the crate.
fn outcome_descriptors(test_world: bool) -> Result<Vec<DefinitionDescriptor>, ModError> {
    let mut descriptors = vec![
        descriptor(
            "outcome",
            crate::quest::store::OUTCOME,
            json!({"fields":["actor","npc","source","booking","revision","checked","completed"]}),
        )?,
        descriptor(
            "outcome",
            crate::port::passenger::store::OUTCOME,
            json!({"fields":["actor","booking","phase"]}),
        )?,
        descriptor(
            "outcome",
            crate::surface::survey::SURVEY_OUTCOME,
            json!({"fields":["actor","offer","contract"]}),
        )?,
        descriptor(
            "outcome",
            crate::surface::survey::SURVEY_CANCEL_OUTCOME,
            json!({"fields":["actor","offer","contract"]}),
        )?,
        descriptor(
            "outcome",
            crate::relay_foundry::OUTCOME,
            json!({"fields":["actor","order","op"]}),
        )?,
        descriptor(
            "outcome",
            crate::port::stock::STOCK_OUTCOME,
            json!({ "fields": ["actor", "issue", "lot", "qty", "vault"] }),
        )?,
        // Doc 20 cut SM.12a: the mill's batch.
        descriptor(
            "outcome",
            crate::port::mill::MILL_OUTCOME,
            json!({ "fields": ["actor", "mill", "ordinal", "ore_t", "product_t"] }),
        )?,
        descriptor(
            "outcome",
            crate::port::burn::BURN_OUTCOME,
            json!({ "fields": ["actor", "burn_t", "crew_days", "hull", "seq"] }),
        )?,
        // Doc 20 cut SM.3b: the rig act's start and finish.
        descriptor(
            "outcome",
            crate::surface::act::SURFACE_ACT_OUTCOME,
            json!({ "fields": ["act", "actor", "due_ic", "ordinal", "tv"] }),
        )?,
        descriptor(
            "outcome",
            crate::surface::act::SURFACE_DONE_OUTCOME,
            json!({ "fields": ["actor", "ordinal", "result", "tv"] }),
        )?,
        descriptor(
            "outcome",
            crate::surface::recovery::RECOVERY_OUTCOME,
            json!({ "fields": ["actor", "tv", "ordinal", "operation", "pending", "moved"] }),
        )?,
        // Doc 20 cut SM.4a (D9): `+tv/forget`'s own action.
        descriptor(
            "outcome",
            crate::surface::chart::CHART_FORGET_OUTCOME,
            json!({ "fields": ["actor", "rows"] }),
        )?,
        // Doc 20 cut SM.6 (§3.4): the site clerk's worked transfer — never a tonne.
        descriptor(
            "outcome",
            crate::port::site::SITE_WORK_OUTCOME,
            json!({ "fields": ["actor", "deposit", "receipt"] }),
        )?,
        // Doc 20 cut SM.12b (§12.3): a house survey (`t` 0) or a yard's delivery — never a
        // reserve or a class.
        descriptor(
            "outcome",
            crate::port::yard::DISPATCH_OUTCOME,
            json!({ "fields": ["act", "actor", "receipt", "site", "t"] }),
        )?,
        // Doc 23 cut S.2: a ship's epoch opened or closed, and whether its line was written.
        descriptor(
            "outcome",
            crate::port::epoch::EPOCH_OUTCOME,
            json!({ "fields": ["actor", "hull", "op", "written"] }),
        )?,
    ];
    if test_world {
        descriptors.push(descriptor(
            "outcome",
            PROBE_OUTCOME,
            json!({ "fields": ["actor", "agent", "issue", "seen"] }),
        )?);
    }
    Ok(descriptors)
}

/// **Every `action` definition this crate registers**, each referencing its
/// outcome's exact `DefinitionRef` — doc 18 cut IC8's `tacsim_port_stock`, doc 20 cut
/// SM.12a's `tacsim_port_mill` (its outcome paired with the stock's in
/// `stock_outcomes`, so the list stays under clippy's argument bound), cut IC9's
/// `tacsim_port_burn` and doc 20 cut SM.3b's `tacsim_surface_act` and
/// `tacsim_surface_done` on every roster, doc 20 cut SM.6's `tacsim_site_work` and doc 20 cut
/// SM.12b's `tacsim_dispatch` (their outcomes paired in `site_outcomes`, the list kept at seven
/// arguments), and the probe's when its outcome is registered (a test world); one validator per
/// kind for the crate.
fn action_descriptors(
    stock_outcomes: (&DefinitionRef, &DefinitionRef),
    relay_outcome: &DefinitionRef,
    burn_outcome: &DefinitionRef,
    surface_outcomes: (&DefinitionRef, &DefinitionRef),
    chart_forget_outcome: &DefinitionRef,
    site_outcomes: (&DefinitionRef, &DefinitionRef),
    probe_outcome: Option<&DefinitionRef>,
) -> Result<Vec<DefinitionDescriptor>, ModError> {
    let mut descriptors = vec![
        descriptor(
            "action",
            crate::relay_foundry::ACTION,
            json!({"subject":"self_actor","targets":0,"family":crate::relay_foundry::FAMILY}),
        )?
        .with_references(vec![relay_outcome.clone()]),
        descriptor(
            "action",
            crate::port::stock::STOCK_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": families::PORT_STOCK_FAMILY,
            }),
        )?
        .with_references(vec![stock_outcomes.0.clone()]),
        // Doc 20 cut SM.12a: the mill's batch writes the mill's `tacsim.port_mill` ledger.
        descriptor(
            "action",
            crate::port::mill::MILL_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": crate::port::mill::MILL_FAMILY,
            }),
        )?
        .with_references(vec![stock_outcomes.1.clone()]),
        descriptor(
            "action",
            crate::port::burn::BURN_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": families::TICKET_FAMILY,
            }),
        )?
        .with_references(vec![burn_outcome.clone()]),
        // Doc 20 cut SM.3b: every rig act's start and its finish, both writing the rig's
        // `tacsim.excursion` record.
        descriptor(
            "action",
            crate::surface::act::SURFACE_ACT_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": crate::surface::excursion::EXCURSION_FAMILY,
            }),
        )?
        .with_references(vec![surface_outcomes.0.clone()]),
        descriptor(
            "action",
            crate::surface::act::SURFACE_DONE_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": crate::surface::excursion::EXCURSION_FAMILY,
            }),
        )?
        .with_references(vec![surface_outcomes.1.clone()]),
        // Doc 20 cut SM.4a (D9): `+tv/forget` strikes the actor's `tacsim.chart` rows.
        descriptor(
            "action",
            crate::surface::chart::CHART_FORGET_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": crate::surface::chart::CHART_FAMILY,
            }),
        )?
        .with_references(vec![chart_forget_outcome.clone()]),
        // Doc 20 cut SM.6 (§3.4): the site clerk's worked transfer writes the deposit's
        // `tacsim.pockets` cursor.
        descriptor(
            "action",
            crate::port::site::SITE_WORK_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": crate::surface::cursor::POCKETS_FAMILY,
            }),
        )?
        .with_references(vec![site_outcomes.0.clone()]),
        // Doc 20 cut SM.12b (§12.3): the house survey writes the dispatcher's chart row, the
        // delivery the yard's `tacsim.dispatch` ledger; the family named is the ledger's.
        descriptor(
            "action",
            crate::port::yard::DISPATCH_ACTION,
            json!({
                "subject": "self_actor",
                "targets": 0,
                "family": crate::port::yard::DISPATCH_FAMILY,
            }),
        )?
        .with_references(vec![site_outcomes.1.clone()]),
    ];
    if let Some(outcome) = probe_outcome {
        descriptors.push(
            descriptor(
                "action",
                PROBE_ACTION,
                json!({
                    "subject": "self_actor",
                    "targets": 0,
                    "family": PROBE_FAMILY,
                    "test_only": true,
                }),
            )?
            .with_references(vec![outcome.clone()]),
        );
    }
    Ok(descriptors)
}

// ---- the state slot ----------------------------------------------------------

/// The tick's RAM slot (lead ruling L9): never a document, and its own slot, so
/// `on_world_loaded`'s wholesale `TacsimStore` replacement never touches it. A
/// restart begins the walk again at the head, which costs one fire.
#[derive(Clone, Debug, Default)]
pub struct PortTickState {
    /// The rotation cursor, an agent id (the campaign rule).
    cursor: u32,
    last_fire: Option<PortFireStats>,
    /// Round-robin between agents sharing a principal (lead ruling L8): principal -> the agent
    /// it owes its next act to.
    owed: BTreeMap<ObjId, ObjId>,
    /// Agents already logged as `Refused` this boot (each is logged once, not every 5 s).
    warned: BTreeSet<ObjId>,
    /// `Some` only in a test world.
    probe_action: Option<DefinitionRef>,
    /// Test world only.
    probes: BTreeMap<ObjId, ProbeAgent>,
    /// Doc 18 cut IC8: the registered `tacsim_port_stock` action, set at `register`.
    pub(crate) stock_action: Option<DefinitionRef>,
    /// Doc 18 cut IC8: the staged lapsed-hold releases awaiting their pull
    /// verification, by `ListingId.0`, at most `crate::port::stock::MAX_RELEASING`.
    pub(crate) releasing: BTreeMap<u64, crate::port::stock::Releasing>,
    /// Doc 18 cut IC8: fires that found the layer on, this boot.
    pub(crate) fires: u64,
    /// Doc 18 cut IC8 (lead ruling L19): the drift pages raised this boot, by
    /// `(vault, good)` — each pool pages once per boot.
    pub(crate) paged: BTreeSet<(u64, CanonicalId)>,
    /// Doc 18 cut IC12: the moving-sky pages raised this boot — doc 18 §4.8 "pages
    /// if the sky rate is non-zero while any lane body lacks `epoch_ic`". A count
    /// since cut IC14 (row 39), so a test proves one page, not only the latch; it
    /// never exceeds one a boot.
    pub(crate) sky_pages: u32,
    /// Doc 18 §9 risk 19, cut IC14: the marker-headroom pages raised this boot —
    /// the `Markers` readiness line Flagged; at most one a boot.
    pub(crate) marker_pages: u32,
    /// Doc 18 Q-18-60 (R10), cut P-B: the unfunded-house pages raised this boot — a
    /// standing bid whose house cannot fund one unit, which the `Bids` readiness line
    /// reads Pending and never gates on; at most one a boot.
    pub(crate) funds_pages: u32,
    /// Doc 18 cut IC14 row 11: the hulls whose unreadable sailing plan has paged this
    /// boot (once each; bounded by the voyage keys, one per hull).
    pub(crate) voyage_warned: BTreeSet<u32>,
    /// Doc 18 cut IC14 row 11: the unreadable-sailing-plan pages raised this boot.
    pub(crate) voyage_pages: u32,
    /// Doc 18 cut IC14 row 29 (lead ruling L24): stock rows the clerk skips this boot,
    /// by issue key text — a mint submitted `MAX_STOCK_ATTEMPTS` times with no receipt.
    /// Bounded by `crate::port::stock::MAX_CLERK_ROWS`.
    pub(crate) stock_skipped: BTreeSet<String>,
    /// Doc 18 cut IC14 row 29: mints submitted with no receipt yet, by issue key text.
    /// Bounded by `crate::port::stock::MAX_CLERK_ROWS`.
    pub(crate) stock_attempts: BTreeMap<String, u8>,
    /// Doc 18 cut IC14 row 29: the ports whose map records rows past the clerk's bound,
    /// warned once a boot each (bounded by the lane's ports).
    pub(crate) stock_uncarried: BTreeSet<CanonicalId>,
    /// Doc 18 cut IC8: listings the release arm has already logged a skip or a
    /// refusal for this boot (each is logged once).
    pub(crate) release_warned: BTreeSet<u64>,
    /// Doc 18 cut IC8: listings whose release step was refused, by `ListingId.0` ->
    /// the fire it was refused on. The release agent passes over each for
    /// `crate::port::stock::RELEASE_GRACE_FIRES`, so one listing that is always refused
    /// can never hold the head of the line; entries past their grace are pruned every
    /// fire, so the map holds at most one entry per fire of grace.
    pub(crate) release_backoff: BTreeMap<u64, u64>,
    /// Doc 18 cut IC9: the launches awaiting their pull verification, by hull
    /// (`ObjId.0`) — `crate::port::voyage::launch_verify` tells the pilot once.
    pub(crate) launching: BTreeMap<u32, crate::port::voyage::Launching>,
    /// Doc 18 cut IC9 (lead ruling L29): held launches, by hull -> the fire the
    /// hold was raised on; the launch agent passes over each for
    /// `crate::port::voyage::LAUNCH_GRACE_FIRES`, then retries.
    pub(crate) launch_backoff: BTreeMap<u32, u64>,
    /// Doc 18 cut IC9: hulls whose held launch (or unreadable ticket) has paged
    /// staff and told the pilot this boot — once each.
    pub(crate) launch_warned: BTreeSet<u32>,
    /// Doc 18 cut IC9 (lead ruling L26): the fire a pending passage with no ticket
    /// was first visited on, by hull; the grace restarts after a reboot, which is
    /// harmless.
    pub(crate) filed_seen: BTreeMap<u32, u64>,
    /// Doc 18 cut IC10: the bid re-posts the market refused this boot, by `(vault,
    /// good)` — each is logged once.
    pub(crate) bid_warned: BTreeSet<(u64, CanonicalId)>,
    /// Doc 20 cut SM.5 (§7.3; reviewer F10): the intake drip's warnings this boot, by
    /// `(vault, good, kind)` — keyed apart by [`crate::port::stock::DripWarn`], so a
    /// first warning never hides the second; each is logged once.
    pub(crate) drip_warned: BTreeSet<(u64, CanonicalId, crate::port::stock::DripWarn)>,
    /// **Doc 20 cut SM.12a**: the registered `tacsim_port_mill` action, set at `register`;
    /// `Some` exactly when its provider registered (the inventory cell filled).
    pub(crate) mill_action: Option<DefinitionRef>,
    /// **Doc 20 cut SM.12a** (D8, lead ruling L24's rule as the drip applies it): the mill
    /// acts refused today, by `(storefront vault, the mill's ore, which act)`, with the IC
    /// day of that refusal. The clerk passes such a mill's act over for the rest of that
    /// IC day, so one bad mill never stalls the other mill, the drip or the bids. Bounded
    /// by `crate::port::stock::MAX_CLERK_ROWS`; stale days are pruned at each insert.
    pub(crate) mill_refused: BTreeMap<(u64, CanonicalId, crate::port::mill::MillAct), i64>,
    /// **Doc 20 cut SM.12a** (the stock rule's `stock_attempts`, lead ruling L24): a mill's
    /// batch submitted with its ledger not yet advanced, by `(storefront vault, the mill's
    /// ore)` -> `(the ordinal, the submissions)`. Bounded by
    /// `crate::port::stock::MAX_CLERK_ROWS`.
    pub(crate) mill_attempts: BTreeMap<(u64, CanonicalId), (u64, u8)>,
    /// **Doc 20 cut SM.12a**: the mill's warnings this boot, by `(storefront vault, the
    /// mill's ore, kind)` — `crate::port::mill::MillWarn::Underflow`, which no single mill
    /// owns, by the ask's good instead; each is logged once. Bounded by
    /// `crate::port::stock::MAX_CLERK_ROWS`.
    pub(crate) mill_warned: BTreeSet<(u64, CanonicalId, crate::port::mill::MillWarn)>,
    /// Doc 20 cut SM.5 (lead ruling L24's rule, applied to the drip): the refresh rows
    /// whose drip act the market refused having moved nothing, by `(vault, good)`, with
    /// the IC day of that refusal. The drip passes over such a row for the rest of that
    /// IC day, so one row's standing refusal (a suspended house, a listing cap) never
    /// stalls the other intakes or the lapsed-bid re-post behind it. Bounded by
    /// `crate::port::stock::MAX_CLERK_ROWS`.
    pub(crate) drip_refused: BTreeMap<(u64, CanonicalId), i64>,
    /// **Doc 20 cut SM.12a** (D8; lead ruling L24's rule, as the drip applies it): the asks
    /// whose listing the market refused, by `(storefront vault, good)`, with the IC day of
    /// that refusal. The list step passes over such an ask for the rest of that IC day, so
    /// a pool its house cannot list (a suspended or retired house; a mill makes such a pool
    /// out of a sold-out ask) never spends the clerk's every act and stalls the drip, the
    /// mills and the bids behind it. Bounded by `crate::port::stock::MAX_CLERK_ROWS`; stale
    /// days are pruned at each insert.
    pub(crate) list_refused: BTreeMap<(u64, CanonicalId), i64>,
    /// Doc 19 cut A.4b: when each port's emergency reading was last taken with
    /// nothing to do, by entry id (real seconds); the clerk reads it again
    /// `crate::port::price::EMERGENCY_CHECK_SECS` later.
    pub(crate) emergency_checked: BTreeMap<CanonicalId, i64>,
    /// Doc 18 cut IC11: hulls whose unreadable queued sailing plan has paged this
    /// boot (once each).
    pub(crate) orders_warned: BTreeSet<u32>,
    /// Doc 18 cut IC11: hulls whose queued depart met a dark plan this boot (paged
    /// once each).
    pub(crate) depart_dark_warned: BTreeSet<u32>,
    /// Doc 18 cut IC11 (reviewer F12): the order verification's rotation cursor, a
    /// hull id — the window of [`crate::port::executor::MAX_ORDER_VERIFY_PER_FIRE`]
    /// starts here, so no hull is starved.
    pub(crate) order_verify_after: u32,
    /// Doc 18 cut IC13: hulls the recall arm has paged this boot (a dormant hull that
    /// cannot be recalled where it is, a revoked hull not back in the pool, or an
    /// unreadable dormancy read) — once each.
    pub(crate) recall_warned: BTreeSet<u32>,
    /// Doc 18 cut IC13: blocked recalls, by hull -> the fire it was blocked on; the
    /// recall agent passes over each for `crate::port::recall::RECALL_RECHECK_FIRES`.
    pub(crate) recall_backoff: BTreeMap<u32, u64>,
    /// Doc 18 cut IC14 row 12 (lead ruling L11): a refunded buyout's re-charters, by
    /// hull -> (attempts made, the fire the next is due). RAM only, so a reboot resets
    /// the count (bounded by reboots); retained each fire to hulls whose governing
    /// enrolment still waits on a re-charter, so bounded by the pool of eight.
    pub(crate) relet_attempts: BTreeMap<ObjId, (u8, u64)>,
    /// **Doc 20 cut SM.12b**: the registered `tacsim_dispatch` action, set at `register`; `Some`
    /// exactly when its provider registered (the inventory cell filled).
    pub(crate) dispatch_action: Option<DefinitionRef>,
    /// **Doc 20 cut SM.12b** (SM.12a's mill rule): a yard's delivery submitted with its ledger not
    /// yet advanced, by dispatcher -> `(the ordinal, the submissions)`. Bounded by
    /// `crate::port::stock::MAX_CLERK_ROWS`.
    pub(crate) dispatch_attempts: BTreeMap<ObjId, (u64, u8)>,
    /// **Doc 20 cut SM.12b**: a dispatcher whose delivery is passed over for the rest of the IC
    /// day, with that day (a ledger that will not read, or an ordinal that never lands). Bounded
    /// by `crate::port::stock::MAX_CLERK_ROWS`; stale days are pruned at each insert.
    pub(crate) dispatch_refused: BTreeMap<ObjId, i64>,
    /// **Doc 20 cut SM.12b** (D21): a yard rig whose plan compile was refused, by rig -> the IC
    /// instant its next compile may run (one re-arm interval on), so a standing refusal never
    /// re-runs the compile every fire. Bounded by `crate::port::stock::MAX_CLERK_ROWS`; past
    /// instants are pruned at each insert.
    pub(crate) dispatch_file_after: BTreeMap<ObjId, i64>,
    /// **Doc 20 cut SM.12b**: the dispatchers' warnings this boot, by `(dispatcher, kind, what)`;
    /// each is logged once. Bounded by `crate::port::stock::MAX_CLERK_ROWS`.
    pub(crate) dispatch_warned:
        BTreeSet<(ObjId, crate::port::yard::DispatchWarn, smol_str::SmolStr)>,
    /// **Doc 23 cut S.2**: the registered `tacsim_voyage_epoch` action, set at `register`.
    pub(crate) epoch_action: Option<DefinitionRef>,
    /// **Doc 23 cut S.2**: each hull's due epoch open or close the arm submitted this boot, by
    /// hull (`ObjId.0`); dropped once the target is no longer due. Bounded by the programme's
    /// hulls.
    pub(crate) epoch_tries: BTreeMap<u32, crate::port::epoch::EpochTry>,
    /// **Doc 23 cut S.2**: the epoch arm's pages this boot, by `(hull, why)` — each once.
    pub(crate) epoch_warned: BTreeSet<(u32, crate::port::epoch::EpochWarn)>,
    /// **Doc 23 cut S.2**: the epoch arm's pages this boot (a count, for tests).
    pub(crate) epoch_pages: u32,
}

impl PortTickState {
    fn new(
        probe_action: Option<DefinitionRef>,
        stock_action: Option<DefinitionRef>,
        (mill_action, epoch_action): (Option<DefinitionRef>, Option<DefinitionRef>),
        dispatch_action: Option<DefinitionRef>,
    ) -> PortTickState {
        PortTickState {
            probe_action,
            stock_action,
            mill_action,
            dispatch_action,
            epoch_action,
            ..PortTickState::default()
        }
    }
}

/// What one fire did, for tests and status renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PortFireStats {
    /// Agents visited (each counts against [`MAX_AGENTS_VISITED_PER_FIRE`]).
    pub visited: u32,
    /// Agents worked: `Worked` and `Submitted` (against [`MAX_AGENTS_WORKED_PER_FIRE`]).
    pub worked: u32,
    /// Submissions deferred to the engine's drain.
    pub submitted: u32,
    /// Agents whose principal already acted, or owed its act to another agent.
    pub deferred: u32,
    /// Agents refused (gone, GOD or gone principal, unreadable read).
    pub refused: u32,
    /// Doc 18 cut IC14 row 20 (lead ruling L16): agents waiting on staff or a backoff
    /// that have already paged — counted, never the generic refusal warning.
    pub held: u32,
    /// **Doc 20 cut SM.12c** (the cost lane; doc 20 §11's Tick row): the fire's own wall time in
    /// microseconds — the tick's body after the layer check, the pages and the walk included.
    /// RAM only, never persisted, never shown to a player.
    pub elapsed_us: u64,
    /// **Doc 20 cut SM.12c**: the stock arm's agents this fire — **every latched lane port map's
    /// stock clerk** (five vendor ports on the shipped lane), never one clerk's figure. RAM only,
    /// never persisted, never shown to a player.
    pub stock: ArmFireStats,
    /// **Doc 20 cut SM.12c**: the site clerks' agents this fire (`Arm::Site`). RAM only, never
    /// persisted, never shown to a player.
    pub site: ArmFireStats,
    /// **Doc 20 cut SM.12c**: the yard dispatchers' agents this fire (`Arm::Dispatch`). RAM only,
    /// never persisted, never shown to a player.
    pub dispatch: ArmFireStats,
    /// **Doc 23 cut S.2**: the epoch arm's agents this fire (`Arm::Epoch`). RAM only, never
    /// persisted, never shown to a player.
    pub epoch: ArmFireStats,
}

/// **One arm's share of a port fire** (doc 20 cut SM.12c, the cost lane; doc 20a §5.6's "by
/// arm"): counted in the walk beside [`PortFireStats`]' totals for the stock, site and dispatch
/// arms and, doc 23 cut S.2, the epoch arm (the other arms count only in the totals). `deferred` counts the `Fire::as_principal`
/// refusals that defer — the principal already acted this fire, owed its act to another agent,
/// or has a submission still in flight (`Pending`). RAM only, never persisted, never shown to a
/// player.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArmFireStats {
    /// Agents of the arm visited.
    pub visited: u32,
    /// Agents of the arm worked (`Worked` and `Submitted`).
    pub worked: u32,
    /// Agents of the arm deferred.
    pub deferred: u32,
    /// Agents of the arm held (waiting on staff or a backoff, already paged).
    pub held: u32,
    /// Agents of the arm refused.
    pub refused: u32,
}

impl ArmFireStats {
    /// Count one visited agent's step (doc 20 cut SM.12c).
    fn count(&mut self, step: Step) {
        self.visited = self.visited.saturating_add(1);
        match step {
            Step::Idle => {}
            Step::Worked | Step::Submitted => self.worked = self.worked.saturating_add(1),
            Step::Deferred => self.deferred = self.deferred.saturating_add(1),
            Step::Held => self.held = self.held.saturating_add(1),
            Step::Refused => self.refused = self.refused.saturating_add(1),
        }
    }
}

/// The last fire's stats, `None` before the first fire that found the layer on.
#[must_use]
pub fn last_fire(w: &World) -> Option<PortFireStats> {
    w.module_state::<PortTickState>()
        .and_then(|state| state.last_fire)
}

/// **The drift pages the stock arm raised this boot** (doc 18 cut IC8, lead ruling
/// L19), as `(storefront vault, good)` in order — a status read for tests and later
/// renders; each pool pages at most once a boot.
#[must_use]
pub fn drift_pages(w: &World) -> Vec<(u64, CanonicalId)> {
    w.module_state::<PortTickState>()
        .map(|state| state.paged.iter().cloned().collect())
        .unwrap_or_default()
}

/// **How many times the port tick has paged the moving sky this boot** (doc 18 cut
/// IC12; §4.8; a count since cut IC14, row 39): a status read for tests, beside
/// [`drift_pages`]. One once a fire with the layer on found a non-zero sky rate and
/// a lane body without `epoch_ic` (`crate::port::readiness::body_without_epoch`);
/// the page is raised once a boot, so it never exceeds one.
#[must_use]
pub fn sky_pages(w: &World) -> u32 {
    w.module_state::<PortTickState>()
        .map_or(0, |state| state.sky_pages)
}

/// **The fire a hull's blocked recall was last backed off on, this boot** (doc 18 cut
/// IC14 row 33): a status read for tests beside [`recall_pages`] — a blocked recall
/// re-arms it each time the arm re-enters after `RECALL_RECHECK_FIRES`.
#[must_use]
pub fn recall_blocked_at(w: &World, hull: u32) -> Option<u64> {
    w.module_state::<PortTickState>()
        .and_then(|state| state.recall_backoff.get(&hull).copied())
}

/// **A refunded buyout's re-charter attempts so far and the fire the next is due**
/// (doc 18 cut IC14 row 12; proposed Q-18-71): a status read for tests beside
/// [`recall_pages`]. `None` before the recall arm first sees the hull and again once
/// the re-charter landed or the arm gave up.
#[must_use]
pub fn relet_attempts(w: &World, hull: ObjId) -> Option<(u8, u64)> {
    w.module_state::<PortTickState>()
        .and_then(|state| state.relet_attempts.get(&hull).copied())
}

/// **How many fires found the layer on this boot** (doc 18 cut IC14): a status read
/// for tests that count fires against a backoff.
#[must_use]
pub fn fires(w: &World) -> u64 {
    w.module_state::<PortTickState>()
        .map_or(0, |state| state.fires)
}

/// **How many times the port tick has paged the marker headroom this boot** (doc 18
/// §9 risk 19, cut IC14): one once a fire found the `Markers` readiness line Flagged
/// (more than three quarters of `mush_inventory::MAX_APPLIED_MARKERS` held); at most
/// one a boot.
#[must_use]
pub fn marker_pages(w: &World) -> u32 {
    w.module_state::<PortTickState>()
        .map_or(0, |state| state.marker_pages)
}

/// **How many times the port tick has paged a house that cannot fund a tonne this
/// boot** (doc 18 Q-18-60, the owner's R10; cut P-B): one once a fire found a
/// standing bid whose house's capital has settled and whose available balance is
/// still under one unit's price (`crate::port::readiness::unfunded_bids`; a fresh
/// world's unsettled window never pages); at most one a boot. The `Bids` line reads
/// Pending on it and never closes `+coop/enrol`; the sales against the house skip
/// with the executor's `HOUSE_CANNOT_PAY_SKIP` until it is paid.
#[must_use]
pub fn funds_pages(w: &World) -> u32 {
    w.module_state::<PortTickState>()
        .map_or(0, |state| state.funds_pages)
}

/// **The hulls the recall arm has paged this boot** (doc 18 cut IC13), in order — a
/// status read for tests beside [`sky_pages`].
#[must_use]
pub fn recall_pages(w: &World) -> Vec<u32> {
    w.module_state::<PortTickState>()
        .map(|state| state.recall_warned.iter().copied().collect())
        .unwrap_or_default()
}

/// Doc 18 cut IC12 (§4.8; lead ruling L13): the moving-sky page, once a boot. The
/// readiness report flags the Sky line on the same condition and stays a pure read;
/// the page lives here, on the fire, counted in [`PortTickState::sky_pages`] (at
/// most one a boot). It is never a gate (D-18-50).
fn page_moving_sky(w: &World, state: &mut PortTickState) {
    if state.sky_pages > 0 {
        return;
    }
    let Some(store) = w.module_state::<TacsimStore>() else {
        return;
    };
    let ephemeris = &store.config().time.ephemeris;
    let (Some(pack), _) = crate::port::readiness::report_ports(store, w) else {
        return;
    };
    if let Some(body) = crate::port::readiness::body_without_epoch(pack, ephemeris) {
        tracing::error!(
            runbook = "doc 18 §4.8: give every lane body an epoch_ic or set the sky rate to 0",
            body = %body,
            rate = ephemeris.rate,
            "tacsim: the sky moves but a lane body has no epoch_ic"
        );
        state.sky_pages = state.sky_pages.saturating_add(1);
    }
}

/// Doc 18 cut IC14 row 11 (lead ruling L17): each unreadable sailing plan pages once a
/// boot, beyond the boot log line — every gate treats it as filed meanwhile.
fn page_dark_voyages(w: &World, state: &mut PortTickState) {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return;
    };
    for hull in store.voyages_unreadable() {
        if state.voyage_warned.insert(hull.0) {
            tracing::error!(
                hull = hull.0,
                runbook = "repair the tacsim:voyage:<hull> document; every gate treats the \
                           hull as cleared until it reads",
                "tacsim: a hull's sailing plan does not read; it is held out of the pool"
            );
            state.voyage_pages = state.voyage_pages.saturating_add(1);
        }
    }
}

/// **How many unreadable sailing plans the port tick has paged this boot** (doc 18
/// cut IC14 row 11): a status read for tests; each hull at most once a boot.
#[must_use]
pub fn voyage_pages(w: &World) -> u32 {
    w.module_state::<PortTickState>()
        .map_or(0, |state| state.voyage_pages)
}

/// Doc 18 §9 risk 19, cut IC14: the marker-headroom page, once a boot, in the
/// [`page_moving_sky`] manner. The readiness report flags the `Markers` line on the
/// same condition ([`crate::port::readiness::markers_line`]) and stays a pure read;
/// the page lives here, counted in [`PortTickState::marker_pages`]. Never a gate
/// (proposed Q-18-72): it is the operator's signal that the inventory mirror nears
/// the cap past which every inventory fact goes dark.
fn page_marker_headroom(w: &World, state: &mut PortTickState) {
    if state.marker_pages > 0 {
        return;
    }
    let Some(inventory) = w.module_state::<mush_inventory::InventoryStore>() else {
        return;
    };
    let line = crate::port::readiness::markers_line(
        Some(inventory.markers()),
        mush_inventory::MAX_APPLIED_MARKERS,
    );
    if line.state == crate::port::readiness::LineState::Flagged {
        tracing::error!(
            runbook = "docs/ops.md \"Applied-marker retention (doc 18 cut IC14)\"",
            held = inventory.markers(),
            max = mush_inventory::MAX_APPLIED_MARKERS,
            "tacsim: the inventory mirror holds more than three quarters of its applied-marker \
             cap; past it every inventory fact goes dark"
        );
        state.marker_pages = state.marker_pages.saturating_add(1);
    }
}

/// Doc 18 Q-18-60 (the owner's R10 of 2026-10-18), cut P-B: the unfunded-house page,
/// once a boot, in the [`page_moving_sky`] manner. The readiness report reads the
/// `Bids` line Pending on the same condition ("not buying until its capital
/// settles"; `crate::port::readiness::unfunded_bids`, which also asks that the
/// house's capital has settled, so a fresh world's first fires never page) and,
/// since this cut, never gates on it; the page lives here, counted in
/// [`PortTickState::funds_pages`], so staff hear once that a house's sales are
/// skipping while enrolment stays open.
fn page_unfunded_houses(w: &World, state: &mut PortTickState) {
    if state.funds_pages > 0 {
        return;
    }
    let unfunded = crate::port::readiness::unfunded_bids(w);
    let Some((entry, org, good)) = unfunded.first() else {
        return;
    };
    tracing::error!(
        runbook = "doc 18 Q-18-60 (cut P-B): pay the house (@econ/pay #<treasury account>=#<house \
                   account> <amount>) or let players buy from it; enrolment stays open and the \
                   sales against it skip meanwhile",
        port = %entry,
        house = %org,
        good = %good,
        bids = unfunded.len(),
        "tacsim: a house cannot fund a tonne of its bid; sales against it skip until it is paid"
    );
    state.funds_pages = state.funds_pages.saturating_add(1);
}

// ---- the fire ----------------------------------------------------------------

/// One agent's turn this fire (D-18-32: one step per agent per fire).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Nothing due.
    Idle,
    /// A unit of work that staged nothing authoritative: doc 18 cut IC8's stock arm
    /// publishing a Draw listing (a normal-lane write made as the clerk). It counts as
    /// worked and as the principal's one act this fire.
    Worked,
    /// The swapped call DEFERRED one submission to the engine's drain. This is NOT "admitted":
    /// the drain may still drop it as back-pressure when the principal already has a pending
    /// submission (`engine.rs:4066-4074`), or void it as a fan-out when another submission by
    /// the same principal from outside this tick reached the same drain ([`Fire`]). A real arm
    /// (IC8/IC9/IC11) verifies completion by pull on a later fire (D-18-32) and never treats
    /// `Submitted` as done.
    Submitted,
    /// Its principal already acted this fire, or owes its act to another of its agents.
    Deferred,
    /// Agent gone, principal GOD or gone, or an unreadable read; logged once per boot.
    Refused,
    /// Doc 18 cut IC14 row 20 (lead ruling L16): a step that is waiting on staff or a
    /// backoff and has already paged (a held launch); counted, never the generic
    /// warning.
    Held,
}

/// The arms, in dispatch order (one step per agent per fire, D-18-32).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arm {
    /// Doc 18 cut IC8: a port's stock clerk — mint, dissolve, list
    /// ([`crate::port::stock::stock_step`]).
    Stock,
    /// Doc 18 cut IC8: the release of one lapsed player listing's hold, as the
    /// holder ([`crate::port::stock::release_step`]); at most one per fire.
    Release {
        /// The lapsed listing, `ListingId.0`.
        listing: u64,
    },
    /// Doc 18 cut IC9: one hull's filed passage — find its burned ticket and
    /// launch it, lapse it, or hold it ([`crate::port::voyage::launch_step`]).
    Launch {
        /// The hull, `ObjId.0`.
        hull: u32,
    },
    /// Doc 18 cut IC13: one chartered hull gone a week without a departure, or one
    /// whose charter ended while she is not back in the pool — recall or return it,
    /// or hold its plan and page ([`crate::port::recall::recall_step`]).
    Recall {
        /// The hull, `ObjId.0`.
        hull: u32,
    },
    /// Doc 18 cut IC11: one hull's queued sailing plan — re-stage its market leg, or
    /// run its cursor order as the plan's author
    /// ([`crate::port::executor::order_step`]).
    Order {
        /// The hull, `ObjId.0`.
        hull: u32,
    },
    /// Doc 20 cut SM.6 (§3.4, D-20-5): a latched lane site's clerk — open its next deposit or
    /// move its next worked share ([`crate::port::site::site_step`]); agent and principal the
    /// clerk.
    Site,
    /// Doc 20 cut SM.12b: a yard's dispatcher — deliver the yard hold's ore, or file one rig's
    /// next plan ([`crate::port::yard::dispatch_step`]); agent and principal the dispatcher.
    Dispatch,
    /// Doc 23 cut S.2: one hull's due epoch open or close, as its warrant principal
    /// ([`crate::port::epoch::epoch_step`]).
    Epoch,
    /// A persistent passenger agreement, acting as its captain.
    Passenger { booking: u64 },
    /// The test world's probe (lead ruling L3).
    Probe,
}

/// The `tacsim.port` fire.
fn port_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    // Lead ruling L12: the layer off visits nothing, moves no cursor and leaves
    // `last_fire` untouched.
    if !ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled)
    {
        return;
    }
    // Doc 20 cut SM.12c (the cost lane): the fire's own clock, the pages and the walk included.
    let started = std::time::Instant::now();
    if let Err(error) = crate::port::relationships::reconcile(ctx) {
        tracing::warn!(%error, "public desk affiliation reconciliation failed");
    }
    crate::port::survey::tick(ctx);
    crate::port::passenger::tick(ctx);
    crate::quest::tick(ctx);
    let Some(slot) = ctx.w.module_state_mut::<PortTickState>() else {
        return;
    };
    // The slot is taken for the fire and written back whole at its end; nothing
    // the fire calls reads it (the probe's offer is projected at landing, after).
    let mut state = std::mem::take(slot);
    state.fires = state.fires.saturating_add(1);
    page_moving_sky(ctx.w, &mut state);
    page_marker_headroom(ctx.w, &mut state);
    page_unfunded_houses(ctx.w, &mut state);
    page_dark_voyages(ctx.w, &mut state);
    // Doc 18 cut IC14 row 21: the first fire after a boot tells the grants the RAM
    // verification lost.
    if state.fires == 1 {
        crate::port::voyage::launch_retell(ctx, &state);
    }
    let mut stats = walk(ctx, &mut state);
    stats.elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    state.last_fire = Some(stats);
    if let Some(slot) = ctx.w.module_state_mut::<PortTickState>() {
        *slot = state;
    }
}

/// Collect, order and walk the agents; returns the fire's stats and leaves the
/// cursor, the debts and the warned set updated in `state`.
fn walk(ctx: &mut Ctx<'_>, state: &mut PortTickState) -> PortFireStats {
    // Doc 18 cut IC8 (D-18-32 step 5): the release arm's pull verification runs
    // first on every fire, whether or not any agent is offered. A notice pass: no
    // submission and no swap.
    crate::port::stock::release_verify(ctx, state);
    // Doc 18 cut IC9: the launch arm's pull verification, likewise first (no
    // submission, no swap), then its RAM pruned of every hull whose sailing plan
    // has no pending passage any more (a launch, a lapse or a staff clear).
    crate::port::voyage::launch_verify(ctx, state);
    crate::port::voyage::launch_prune(ctx.w, state);
    // Doc 18 cut IC11: the order arm's pull verification, likewise (no swap): every
    // step in flight judged Landed, Dropped or still pending, unreadable plans paged.
    crate::port::executor::order_verify(ctx, state);
    // Doc 18 cut IC8: the stock clerks of every present lane port map whose lane
    // is latched (a retired lane seeds nothing), then at most one release agent.
    // Doc 18 cut IC14 row 31 (lead ruling L25): latched per ENTRY — a port entry a
    // later lane version dropped keeps its map, and its clerk is not offered.
    let clerks: Vec<ObjId> = ctx
        .w
        .module_state::<TacsimStore>()
        .map(|store| {
            store
                .port_maps()
                .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
                .filter(|map| crate::port::stock::entry_latched(ctx.w, &map.entry))
                .filter_map(|map| map.clerk.as_ref().map(|clerk| ObjId(clerk.thing)))
                .collect()
        })
        .unwrap_or_default();
    let site_clerks = crate::port::site::site_clerks(ctx.w);
    let dispatchers = crate::port::yard::dispatchers(ctx.w);
    let release = crate::port::stock::release_agent(ctx.w, state);
    let launches = crate::port::voyage::launch_agents(ctx.w, state);
    // Doc 18 cut IC13 (lead ruling L12): the recall agents read tickets (`&Ctx`), and
    // their hulls are dropped from the order agents before either is offered, so a
    // dormant hull with a waiting plan is recalled rather than offered to the order
    // arm every fire, and the duplicate rule never logs.
    let mut recalls = crate::port::recall::recall_agents(ctx, state);
    let mut orders = crate::port::executor::order_agents(ctx.w, state);
    orders.retain(|(hull, _)| !recalls.iter().any(|(recalled, _)| recalled == hull));
    // Doc 23 cut S.2 (V1; the fix round's R4): the epoch agents — and every hull whose epoch
    // open or close is due, offered this fire or spacing out its retry, is withheld from the
    // launch, recall and order arms (one arm an agent), so a close lands before the hull can leave
    // again and an open before it moves on; they wait. A held or unwritable epoch withholds
    // nothing (R2: it never wedges its hull).
    let (epochs, withheld) = crate::port::epoch::epoch_agents(ctx, state);
    let mut launches = launches;
    launches.retain(|(hull, _)| !withheld.contains(hull));
    recalls.retain(|(hull, _)| !withheld.contains(hull));
    orders.retain(|(hull, _)| !withheld.contains(hull));
    // The agents: the union over the arms, agent -> (principal, arm). An agent
    // id belongs to one arm (hull ids and clerk ids are disjoint objects); a
    // duplicate offered by a later arm is refused and logged.
    let mut agents: BTreeMap<ObjId, (ObjId, Arm)> = BTreeMap::new();
    let mut offer = |agent: ObjId, principal: ObjId, arm: Arm| {
        if let Some((_, first)) = agents.get(&agent) {
            tracing::error!(
                agent = agent.0,
                ?first,
                ?arm,
                "tacsim: a port agent was offered by two arms; the first keeps it"
            );
        } else {
            agents.insert(agent, (principal, arm));
        }
    };
    for clerk in clerks {
        offer(clerk, clerk, Arm::Stock);
    }
    // Doc 20 cut SM.6 (§3.4): every latched lane site's live clerk, agent and principal both.
    for clerk in site_clerks {
        offer(clerk, clerk, Arm::Site);
    }
    // Doc 20 cut SM.12b (§12.3): every yard's live dispatcher, agent and principal both. The house
    // is its own principal, so its act never spends the Ceres stock clerk's (the emergency bids,
    // the list step, the drip, the mills, the bids) nor the site clerk's; a fire's sixteen-agent
    // budget gains one agent a yard.
    for dispatcher in dispatchers {
        offer(dispatcher, dispatcher, Arm::Dispatch);
    }
    // One per fire: a holder already offered this fire is refused by the
    // duplicate rule above and waits for the next.
    if let Some((holder, listing)) = release {
        offer(holder, holder, Arm::Release { listing });
    }
    // Doc 18 cut IC9: one launch agent per hull with a filed passage (agent = the
    // hull, principal = the filing pilot), hulls in their backoff skipped.
    for (hull, pilot) in launches {
        offer(hull, pilot, Arm::Launch { hull: hull.0 });
    }
    // Doc 18 cut IC13: one recall agent per dormant or not-yet-returned hull (agent =
    // the hull, principal = the enrolment's pilot), after the launches and before the
    // orders.
    for (hull, pilot) in recalls {
        offer(hull, pilot, Arm::Recall { hull: hull.0 });
    }
    // Doc 18 cut IC11: one order agent per hull whose queued plan is due and has no
    // filed passage (agent = the hull, principal = the plan's author, L15).
    for (hull, author) in orders {
        offer(hull, author, Arm::Order { hull: hull.0 });
    }
    // Doc 23 cut S.2: one epoch agent per hull whose epoch open or close is due (agent = the
    // hull, principal = the epoch's warrant principal).
    for (hull, principal) in epochs {
        offer(hull, principal, Arm::Epoch);
    }
    for (npc, captain, booking) in crate::port::passenger::agents(ctx) {
        offer(npc, captain, Arm::Passenger { booking });
    }
    if state.probe_action.is_some() {
        for (agent, probe) in &state.probes {
            offer(*agent, probe.principal, Arm::Probe);
        }
    }
    state.owed.retain(|principal, agent| {
        agents
            .get(agent)
            .is_some_and(|(owner, _)| owner == principal)
    });
    state.warned.retain(|agent| agents.contains_key(agent));
    if agents.is_empty() {
        return PortFireStats::default();
    }
    let order = rotation(agents.keys().copied(), state.cursor);
    let mut fire = Fire::new(std::mem::take(&mut state.owed));
    let mut last_visited = None;
    for agent in order {
        if usize::try_from(fire.stats.visited).unwrap_or(usize::MAX) >= MAX_AGENTS_VISITED_PER_FIRE
            || usize::try_from(fire.stats.worked).unwrap_or(usize::MAX)
                >= MAX_AGENTS_WORKED_PER_FIRE
        {
            break;
        }
        let Some(&(principal, arm)) = agents.get(&agent) else {
            continue;
        };
        fire.stats.visited = fire.stats.visited.saturating_add(1);
        last_visited = Some(agent);
        let step = step(ctx, state, &mut fire, agent, principal, arm);
        fire.settle(agent, step);
        // Doc 20 cut SM.12c (the cost lane): the stock, site and dispatch arms' own shares.
        match arm {
            Arm::Stock => fire.stats.stock.count(step),
            Arm::Site => fire.stats.site.count(step),
            Arm::Dispatch => fire.stats.dispatch.count(step),
            Arm::Epoch => fire.stats.epoch.count(step),
            Arm::Release { .. }
            | Arm::Launch { .. }
            | Arm::Recall { .. }
            | Arm::Order { .. }
            | Arm::Passenger { .. }
            | Arm::Probe => {}
        }
        match step {
            Step::Idle => {}
            Step::Worked => fire.stats.worked = fire.stats.worked.saturating_add(1),
            Step::Submitted => {
                fire.stats.worked = fire.stats.worked.saturating_add(1);
                fire.stats.submitted = fire.stats.submitted.saturating_add(1);
            }
            Step::Deferred => fire.stats.deferred = fire.stats.deferred.saturating_add(1),
            Step::Held => fire.stats.held = fire.stats.held.saturating_add(1),
            Step::Refused => {
                fire.stats.refused = fire.stats.refused.saturating_add(1);
                if state.warned.insert(agent) {
                    tracing::warn!(
                        agent = agent.0,
                        principal = principal.0,
                        "tacsim: a port agent was refused (gone, a GOD or gone principal, or an \
                         unreadable read); it is logged once this boot"
                    );
                }
            }
        }
    }
    state.cursor = next_cursor(last_visited, state.cursor);
    state.owed = fire.owed;
    fire.stats
}

/// The visit order (pure): the first agent with `id.0 >= cursor`, then the rest,
/// then a wrap to the head, once (campaign `tick.rs:120-126`).
fn rotation(agents: impl Iterator<Item = ObjId>, cursor: u32) -> Vec<ObjId> {
    let (before, mut from): (Vec<ObjId>, Vec<ObjId>) = agents.partition(|id| id.0 < cursor);
    from.sort_unstable();
    let mut before = before;
    before.sort_unstable();
    from.extend(before);
    from
}

/// The next cursor (pure): past the last visited agent (saturating), or unchanged
/// when nothing was visited. **A deferral never moves the cursor back.**
fn next_cursor(last_visited: Option<ObjId>, current: u32) -> u32 {
    last_visited.map_or(current, |agent| agent.0.saturating_add(1))
}

/// One agent's step: the arms in a fixed order, exhaustively (D-18-32: one step
/// per agent per fire) — cut IC8's stock and release arms, cut IC9's launch arm, cut
/// IC13's recall arm, cut IC11's order arm, then the probe. The probe arm runs only when
/// `PortTickState.probe_action` is `Some`.
fn step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    agent: ObjId,
    principal: ObjId,
    arm: Arm,
) -> Step {
    match arm {
        Arm::Stock => crate::port::stock::stock_step(ctx, state, fire, agent),
        Arm::Release { listing } => {
            crate::port::stock::release_step(ctx, state, fire, principal, listing)
        }
        Arm::Launch { hull } => crate::port::voyage::launch_step(ctx, state, hull),
        Arm::Recall { hull } => crate::port::recall::recall_step(ctx, state, ObjId(hull)),
        Arm::Order { hull } => {
            crate::port::executor::order_step(ctx, state, fire, ObjId(hull), principal)
        }
        Arm::Site => crate::port::site::site_step(ctx, fire, agent),
        Arm::Dispatch => crate::port::yard::dispatch_step(ctx, state, fire, agent),
        Arm::Epoch => crate::port::epoch::epoch_step(ctx, state, fire, agent, principal),
        Arm::Passenger { booking } => crate::port::passenger::step(ctx, fire, booking),
        Arm::Probe => match state.probe_action.clone() {
            Some(action) => probe_step(ctx, state, fire, &action, agent, principal),
            None => Step::Idle,
        },
    }
}

/// Whether `id` names no live object (absent, or marked `GOING`).
pub(crate) fn gone(w: &World, id: ObjId) -> bool {
    w.get(id).is_none_or(|obj| obj.flags.contains(Flags::GOING))
}

// ---- the swap and the turn rule --------------------------------------------

/// One fire's ledger of who has acted (D-18-32: at most one authoritative submission per actor per
/// fire) and of the round-robin debts between agents sharing a principal. A swapped call counts as
/// its principal's ONE act this fire whether or not it submitted, so a closed call that submits
/// internally (IC11's `trade::quote` + `accept_into`) can never fan out with a second submission
/// FROM THIS TICK in the same drain (`engine.rs:4043-4064`).
///
/// The guarantee is per tacsim fire, NOT per drain. `Engine::fire_due` dispatches every due
/// `@wait` line as its actor and runs every other due module tick into the same outbox before
/// its ONE `drain_deferred`. **Doc 18 cut IC8's core seam** (`Ctx::authoritative_pending`, lead
/// 2c-i) closes two of the cases: [`Fire::as_principal`] asks it first and answers
/// [`SwapRefused::Pending`] — a deferral, no act recorded, no debt — when the principal already
/// has a submission queued in the engine from an earlier drain (which would draw the
/// back-pressure refusal) or deferred in this outbox earlier in the same `fire_due` slice (an
/// earlier `@wait` line, or another module's swapped tick, which would fan out with it). **The
/// residual:** a principal's own `@wait` line that fires LATER in the same slice than this tick
/// still fans out with the arm's submission — the engine voids both and tells the principal
/// "That command tried to submit more than one authoritative action; nothing was staged." An
/// arm's step recovers by pull on a later fire (D-18-32), exactly as for back-pressure
/// ([`Step::Submitted`]); the player's own `@wait` command is the collateral.
///
/// *Why the debt rule is fair* (lead ruling L8): the cursor sweeps the agent set
/// continuously and every fire visits at least `min(N, 16)` agents (work never
/// exceeds visits), so every agent is visited within `ceil(N/16) + 1` fires and a
/// principal's agents are visited in cyclic id order. An agent that defers because
/// its principal already acted becomes that principal's debt if none is recorded;
/// every other agent of the principal then defers until the debtor is visited,
/// and the debtor's act, idle or refusal drops the debt ([`Fire::settle`]). So
/// service passes around a principal's due agents in cyclic id order and no agent
/// waits more than `k` sweeps (`k` the principal's agent count); and since a
/// deferral never pulls the cursor back, a principal with many agents cannot pin
/// the walk.
pub(crate) struct Fire {
    acted: BTreeSet<ObjId>,
    owed: BTreeMap<ObjId, ObjId>,
    stats: PortFireStats,
}

/// Why a swap was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SwapRefused {
    /// The principal is GOD.
    God,
    /// The agent or the principal is absent or `GOING`.
    Gone,
    /// The principal already acted this fire.
    AlreadyActed,
    /// The principal owes its act to another of its agents.
    Owed,
    /// Doc 18 cut IC8: the principal already has an authoritative submission in flight
    /// (queued in the engine, or deferred in this outbox): a deferral that records no
    /// act and no debt.
    Pending,
}

impl Fire {
    /// A fire's ledger, starting from `owed` (doc 20 cut SM.3b: the surface tick starts
    /// every fire from an empty map, finding F8).
    pub(crate) fn new(owed: BTreeMap<ObjId, ObjId>) -> Fire {
        Fire {
            acted: BTreeSet::new(),
            owed,
            stats: PortFireStats::default(),
        }
    }

    /// The agent's turn check, without acting: `God`/`Gone` from the world, then [`arbitrate`].
    pub(crate) fn turn(
        &mut self,
        w: &World,
        agent: ObjId,
        principal: ObjId,
    ) -> Result<(), SwapRefused> {
        if principal == GOD {
            return Err(SwapRefused::God);
        }
        if gone(w, principal) || gone(w, agent) {
            return Err(SwapRefused::Gone);
        }
        arbitrate(&self.acted, &mut self.owed, agent, principal)
    }

    /// Run ONE closed call with `ctx.actor = principal`, restoring the previous actor after it
    /// (the `mush-npc/src/module.rs:1499-1502` and `seed.rs:385-398` precedent). Asks the core
    /// seam first (doc 18 cut IC8: [`SwapRefused::Pending`] without touching `acted` or the
    /// debts), then [`Fire::turn`]; on success inserts `principal` into `acted` before the call.
    pub(crate) fn as_principal<R>(
        &mut self,
        ctx: &mut Ctx<'_>,
        agent: ObjId,
        principal: ObjId,
        call: impl FnOnce(&mut Ctx<'_>) -> R,
    ) -> Result<R, SwapRefused> {
        if ctx.authoritative_pending(principal) {
            return Err(SwapRefused::Pending);
        }
        self.turn(ctx.w, agent, principal)?;
        self.acted.insert(principal);
        let saved = ctx.actor;
        ctx.actor = principal;
        let result = call(ctx);
        ctx.actor = saved;
        Ok(result)
    }

    /// After every visit: unless `step` is `Deferred`, drop any debt owed to `agent`
    /// (`owed.retain(|_, a| *a != agent)`), so an agent that acted, idled or was refused can
    /// never hold its principal's other agents back.
    pub(crate) fn settle(&mut self, agent: ObjId, step: Step) {
        if step != Step::Deferred {
            settle_debts(&mut self.owed, agent);
        }
    }
}

/// The debt half of [`Fire::settle`] (pure).
fn settle_debts(owed: &mut BTreeMap<ObjId, ObjId>, agent: ObjId) {
    owed.retain(|_, owing| *owing != agent);
}

/// The pure turn rule (lead ruling L8):
/// - `principal` already acted this fire -> record `owed[principal] = agent` if no debt is
///   recorded yet, and `Err(AlreadyActed)`;
/// - else `owed[principal]` names another agent -> `Err(Owed)`;
/// - else `Ok(())`.
pub(crate) fn arbitrate(
    acted: &BTreeSet<ObjId>,
    owed: &mut BTreeMap<ObjId, ObjId>,
    agent: ObjId,
    principal: ObjId,
) -> Result<(), SwapRefused> {
    if acted.contains(&principal) {
        owed.entry(principal).or_insert(agent);
        return Err(SwapRefused::AlreadyActed);
    }
    match owed.get(&principal) {
        Some(debtor) if *debtor != agent => Err(SwapRefused::Owed),
        _ => Ok(()),
    }
}

// ---- reading inventory pools --------------------------------------------------

/// Why a pool could not be read. Never a zero balance (doc 18 §9 risk 17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoolReadError {
    /// The layer was handed no inventory cell (an undelegated roster).
    Unwired,
    /// The pool key could not be derived.
    Key(String),
    /// The module-scoped lens refused the read.
    Unreadable(String),
    /// The scalar does not fit `u64`.
    Corrupt,
}

/// One pool's balance, read ONLY through this module's own typed-state capability
/// (`ctx.action_state(&inventory.pools, &key)`, which the module-scoped lens admits because of the
/// delegation `server_modules_with_tacsim` declares; never through an owner-stamped transaction or
/// the engine's store). An absent pool is 0; anything that cannot be read is an error, never 0
/// (doc 18 §9 risk 17; IC7's `read_pool` rule).
///
/// # Errors
///
/// [`PoolReadError`], as its variants say.
pub fn read_pool(
    ctx: &Ctx<'_>,
    endpoint: mush_inventory::record::EndpointRef,
    commodity: &CanonicalId,
) -> Result<u64, PoolReadError> {
    let Some(inventory) = ctx
        .w
        .module_state::<PortHandles>()
        .and_then(|handles| handles.inventory.as_ref())
    else {
        return Err(PoolReadError::Unwired);
    };
    let key = mush_inventory::pool_key(endpoint, commodity)
        .map_err(|error| PoolReadError::Key(error.to_string()))?;
    pool_value(ctx.action_state(&inventory.pools, &key))
}

/// **One pool's availability** — its balance less every live hold, the figure
/// `InventoryFamilies::pool`'s `available` gives a prepare — read through the same lens and
/// rule as [`read_pool`] (doc 20 cut SM.3b; the fixer round's item 9: the dump's preview
/// names exactly what its start freezes). An absent pool is 0; a scalar without its book, a
/// book without its scalar (the half pool `InventoryFamilies::pool` refuses; the round's
/// review F10), or a book that refuses the arithmetic, is `Corrupt`; a refusal is never 0.
///
/// # Errors
///
/// [`PoolReadError`], as its variants say.
pub fn read_available(
    ctx: &Ctx<'_>,
    endpoint: mush_inventory::record::EndpointRef,
    commodity: &CanonicalId,
) -> Result<u64, PoolReadError> {
    let Some(inventory) = ctx
        .w
        .module_state::<PortHandles>()
        .and_then(|handles| handles.inventory.as_ref())
    else {
        return Err(PoolReadError::Unwired);
    };
    let key = mush_inventory::pool_key(endpoint, commodity)
        .map_err(|error| PoolReadError::Key(error.to_string()))?;
    let scalar = ctx
        .action_state(&inventory.pools, &key)
        .map_err(|error| pool_read_error(&error))?;
    let book = ctx
        .action_state(&inventory.books, &key)
        .map_err(|error| pool_read_error(&error))?;
    match (scalar, book) {
        (None, None) => Ok(0),
        (None, Some(_)) => Err(PoolReadError::Corrupt),
        (Some(scalar), Some(book)) => book
            .checked_available(*scalar)
            .map_err(|_| PoolReadError::Corrupt),
        (Some(_), None) => Err(PoolReadError::Corrupt),
    }
}

/// **One pool's balance, read from a hook** (doc 18 cut IC6: the ship gauges'
/// `look_rendered`): [`read_pool`]'s rule exactly — [`PortHandles`] from the
/// hook's world (`Unwired` when absent), the pool key, and this module's own
/// typed-state lens (`HookReadContext::action_state`, stamped with the hook's
/// module). An absent pool is 0; a refusal is never 0.
///
/// # Errors
///
/// [`PoolReadError`], as its variants say.
pub fn read_pool_in(
    hook: &mush_core::HookReadContext<'_>,
    endpoint: mush_inventory::record::EndpointRef,
    commodity: &CanonicalId,
) -> Result<u64, PoolReadError> {
    let Some(inventory) = hook
        .world()
        .module_state::<PortHandles>()
        .and_then(|handles| handles.inventory.as_ref())
    else {
        return Err(PoolReadError::Unwired);
    };
    let key = mush_inventory::pool_key(endpoint, commodity)
        .map_err(|error| PoolReadError::Key(error.to_string()))?;
    pool_value(hook.action_state(&inventory.pools, &key))
}

/// The lens's answer as a balance: absent is 0, a scalar that does not fit
/// `u64` is corrupt, and a refusal is mapped, never 0.
fn pool_value(
    found: Result<Option<&mush_core::BoundedScalar>, ModuleActionStateReadError>,
) -> Result<u64, PoolReadError> {
    match found {
        Ok(None) => Ok(0),
        Ok(Some(scalar)) => u64::try_from(scalar.current()).map_err(|_| PoolReadError::Corrupt),
        Err(error) => Err(pool_read_error(&error)),
    }
}

/// The total map from the lens's refusal to this module's (pure; unit-tested).
fn pool_read_error(error: &ModuleActionStateReadError) -> PoolReadError {
    match error {
        ModuleActionStateReadError::Unavailable
        | ModuleActionStateReadError::UnownedContext
        | ModuleActionStateReadError::ForeignOwner { .. }
        | ModuleActionStateReadError::State(_) => PoolReadError::Unreadable(error.to_string()),
    }
}

// ---- the probe (test world only; lead ruling L3) ----------------------------

/// A test world's probe agent: acts as `principal`, once per remaining count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeAgent {
    /// Whom the agent acts as.
    pub principal: ObjId,
    /// A pool to read before each act (the delegation's proof); `None` reads nothing.
    pub pool: Option<(mush_inventory::record::EndpointRef, CanonicalId)>,
    /// Acts left.
    pub remaining: u32,
    /// The next act's issue number.
    pub next_issue: u32,
}

/// Enlist (or replace) one probe agent. Refused outside a test world (`TacsimStore::test_world()`),
/// when no probe action is registered, or past `MAX_PROBE_AGENTS`.
///
/// # Errors
///
/// A short reason, as above.
pub fn enlist_probe(w: &mut World, agent: ObjId, probe: ProbeAgent) -> Result<(), &'static str> {
    if !w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::test_world)
    {
        return Err("the port-tick probe is a test world's");
    }
    let Some(state) = w.module_state_mut::<PortTickState>() else {
        return Err("the port tick is not composed");
    };
    if state.probe_action.is_none() {
        return Err("no probe action is registered");
    }
    if !state.probes.contains_key(&agent) && state.probes.len() >= MAX_PROBE_AGENTS {
        return Err("the probe roster is full");
    }
    state.probes.insert(agent, probe);
    Ok(())
}

/// One probe mark's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProbeKey {
    /// The agent that acted.
    pub agent: ObjId,
    /// Its issue number.
    pub issue: u32,
}

impl ProbeKey {
    /// `{agent.0}:{issue}`.
    #[must_use]
    pub fn key_text(&self) -> String {
        format!("{}:{}", self.agent.0, self.issue)
    }
}

/// One probe act's mark: who acted, as whom, and the balance it read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeMark {
    /// The agent.
    pub agent: ObjId,
    /// The principal the provider saw as the invocation's actor.
    pub principal: ObjId,
    /// The issue number.
    pub issue: u32,
    /// The pool balance the tick read (0 without a pool).
    pub seen: u64,
}

impl ProbeMark {
    /// `seen` within a pool's ceiling.
    ///
    /// # Errors
    ///
    /// A message when it is not.
    pub fn check(&self) -> Result<(), String> {
        if i64::try_from(self.seen)
            .is_ok_and(|seen| seen <= mush_inventory::record::MAX_POOL_QUANTITY)
        {
            Ok(())
        } else {
            Err("a probe mark reads at most a pool's ceiling".to_owned())
        }
    }
}

/// The probe's idempotency key: `tacsim.port.probe/{agent.0}/{issue}`.
fn probe_key_text(agent: ObjId, issue: u32) -> Option<IdempotencyKeyText> {
    IdempotencyKeyText::parse_canonical(format!("tacsim.port.probe/{}/{issue}", agent.0)).ok()
}

/// The probe step for one agent (lead ruling L3).
fn probe_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    action: &DefinitionRef,
    agent: ObjId,
    principal: ObjId,
) -> Step {
    if gone(ctx.w, agent) {
        return Step::Refused;
    }
    let Some(record) = state.probes.get(&agent).cloned() else {
        return Step::Idle;
    };
    if record.remaining == 0 {
        return Step::Idle;
    }
    match fire.turn(ctx.w, agent, principal) {
        Ok(()) => {}
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => {
            return Step::Deferred;
        }
        Err(SwapRefused::God | SwapRefused::Gone) => return Step::Refused,
    }
    let seen = match &record.pool {
        Some((endpoint, commodity)) => match read_pool(ctx, *endpoint, commodity) {
            Ok(seen) => seen,
            Err(error) => {
                tracing::debug!(
                    agent = agent.0,
                    ?error,
                    "tacsim: the probe's pool read refused"
                );
                return Step::Refused;
            }
        },
        None => 0,
    };
    let issue = record.next_issue;
    let Some(key) = probe_key_text(agent, issue) else {
        tracing::error!(
            agent = agent.0,
            issue,
            "tacsim: the probe's idempotency key did not form"
        );
        return Step::Refused;
    };
    let parameters = json!({ "agent": agent.0, "issue": issue, "seen": seen });
    let submitted = fire.as_principal(ctx, agent, principal, |ctx| {
        ctx.submit_authoritative(
            action.clone(),
            PerceivedRef::SelfActor,
            Vec::new(),
            parameters,
            key,
        );
    });
    match submitted {
        Ok(()) => {}
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => {
            return Step::Deferred;
        }
        Err(SwapRefused::God | SwapRefused::Gone) => return Step::Refused,
    }
    if let Some(entry) = state.probes.get_mut(&agent) {
        match entry.next_issue.checked_add(1) {
            Some(next) => {
                entry.next_issue = next;
                entry.remaining = entry.remaining.saturating_sub(1);
            }
            None => entry.remaining = 0,
        }
    }
    notify(
        ctx,
        principal,
        &format!("Probe {issue} for #{} was staged.", agent.0),
    );
    Step::Submitted
}

/// The probe's provider (test world only): one mark per `(agent, issue)`, the
/// replay guard IC8's receipt copies.
struct ProbeProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    marks: StateFamilyHandle<ProbeKey, ProbeMark>,
}

/// The crate's one `ActionError` builder and its only `.expect` (the bounded
/// static-message idiom of every provider in the tree, `mush-sim/src/handoff.rs`),
/// shared by the probe and doc 18 cut IC8's stock provider
/// (`crate::port::stock`) so the panic budget does not move.
pub(crate) fn port_error(code: ActionErrorCode, message: &'static str) -> ActionError {
    ActionError::new(code, false, message).expect("static port error is bounded")
}

/// The probe invocation's three parameters, checked.
fn probe_parameters(invocation: &ActionInvocation) -> Result<(ObjId, u32, u64), ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(port_error(
            ActionErrorCode::InvalidRequest,
            "the port probe is self-scoped and takes no targets",
        ));
    }
    let invalid = || {
        port_error(
            ActionErrorCode::InvalidRequest,
            "the port probe takes exactly an agent, an issue and a reading",
        )
    };
    let parameters = invocation
        .parameters()
        .as_object()
        .filter(|parameters| parameters.len() == 3)
        .ok_or_else(invalid)?;
    let agent = parameters
        .get("agent")
        .and_then(Value::as_u64)
        .and_then(|agent| u32::try_from(agent).ok())
        .filter(|agent| *agent >= 1)
        .ok_or_else(invalid)?;
    let issue = parameters
        .get("issue")
        .and_then(Value::as_u64)
        .and_then(|issue| u32::try_from(issue).ok())
        .ok_or_else(invalid)?;
    let seen = parameters
        .get("seen")
        .and_then(Value::as_u64)
        .filter(|seen| {
            i64::try_from(*seen).is_ok_and(|seen| seen <= mush_inventory::record::MAX_POOL_QUANTITY)
        })
        .ok_or_else(invalid)?;
    Ok((ObjId(agent), issue, seen))
}

fn integer_parameter(
    id: &str,
    minimum: i64,
    maximum: i64,
) -> Result<ProjectedParameter, ActionError> {
    Ok(ProjectedParameter {
        id: CanonicalId::new(id).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the port probe's parameter id is invalid",
            )
        })?,
        required: true,
        kind: ProjectedParameterKind::Integer { minimum, maximum },
    })
}

impl ActionProvider for ProbeProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        probe_parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
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
        let enlisted = engine
            .world
            .module_state::<PortTickState>()
            .is_some_and(|state| {
                state
                    .probes
                    .values()
                    .any(|probe| probe.principal == query.actor())
            });
        if !enlisted {
            return Ok(None);
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            port_error(
                ActionErrorCode::Integrity,
                "the port probe's definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the port probe's guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the port probe's definition guard is unavailable",
                )
            })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: ProjectedParameterSchema::new(vec![
                integer_parameter("agent", 1, i64::from(u32::MAX))?,
                integer_parameter("issue", 0, i64::from(u32::MAX))?,
                integer_parameter("seen", 0, mush_inventory::record::MAX_POOL_QUANTITY)?,
            ])
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the port probe's parameter schema is invalid",
                )
            })?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-port-probe",
                "Probe",
                "A test world's port-tick probe.",
            )
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the port probe's presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the port probe's offer guards are invalid",
                )
            })?,
        }))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let (agent, issue, seen) = probe_parameters(invocation)?;
        let key = ProbeKey { agent, issue };
        let prior = txn.get(&self.marks, &key).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the port probe's marks are unavailable",
            )
        })?;
        if prior.is_some() {
            // The replay guard IC8's receipt copies: one mark per (agent, issue).
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that probe issue has already landed",
            ));
        }
        let actor = invocation.actor();
        txn.insert(
            &self.marks,
            &key,
            ProbeMark {
                agent,
                principal: actor,
                issue,
                seen,
            },
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the port probe's mark did not stage",
            )
        })?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "actor": actor.0, "agent": agent.0, "issue": issue, "seen": seen }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the port probe's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

// ---- reconnect notices (§4.4 "Failure handling"; lead ruling L10) -----------

/// `format!("{NOTICE_PREFIX}{}", who.0)` — the numeric id, never `ObjId`'s `#`-prefixed Display,
/// so the key round-trips through [`notice_key_owner`] and `store::load`.
#[must_use]
pub fn notice_key(who: ObjId) -> SmolStr {
    SmolStr::new(format!("{NOTICE_PREFIX}{}", who.0))
}

/// The inverse: the tail after `NOTICE_PREFIX` as a strict decimal `u32` (no sign, no `#`,
/// no leading zero but `0`); `None` otherwise. `store::load` uses exactly this.
#[must_use]
pub fn notice_key_owner(key: &str) -> Option<ObjId> {
    key.strip_prefix(NOTICE_PREFIX)
        .and_then(families::strict_decimal_u32)
        .map(ObjId)
}

/// One queued line.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeLine {
    /// When it was queued (`ctx.w.now()`, unix seconds).
    pub at: i64,
    /// The line, one row of at most [`MAX_NOTICE_BYTES`].
    pub text: String,
}

/// One player's queued notices: the whole `tacsim:notice:{obj}` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeQueue {
    /// [`NOTICE_SCHEMA`].
    pub schema: u16,
    /// The character the queue is for (the identity rule: its key's).
    pub who: ObjId,
    /// Oldest first, at most [`MAX_NOTICE_LINES`].
    pub lines: Vec<NoticeLine>,
}

impl NoticeQueue {
    /// An empty queue for `who`.
    #[must_use]
    pub fn new(who: ObjId) -> NoticeQueue {
        NoticeQueue {
            schema: NOTICE_SCHEMA,
            who,
            lines: Vec::new(),
        }
    }

    /// Append one line: newlines/carriage returns become spaces, the text is cut to
    /// MAX_NOTICE_BYTES on a char boundary (housing's `clamp`), and the oldest line is dropped
    /// while the queue is full (FIFO).
    pub fn push(&mut self, at: i64, text: &str) {
        let flat: String = text
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        let text = clamp(flat);
        while self.lines.len() >= MAX_NOTICE_LINES {
            self.lines.remove(0);
        }
        self.lines.push(NoticeLine { at, text });
    }

    /// Schema 1, at most [`MAX_NOTICE_LINES`] lines, each at most
    /// [`MAX_NOTICE_BYTES`] and on one row.
    ///
    /// # Errors
    ///
    /// A message naming the first bound the queue breaks.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != NOTICE_SCHEMA {
            return Err(format!("a notice queue is schema {NOTICE_SCHEMA}"));
        }
        if self.lines.len() > MAX_NOTICE_LINES {
            return Err(format!(
                "a notice queue holds at most {MAX_NOTICE_LINES} lines"
            ));
        }
        for line in &self.lines {
            if line.text.len() > MAX_NOTICE_BYTES {
                return Err(format!("a notice line is at most {MAX_NOTICE_BYTES} bytes"));
            }
            if line.text.contains(['\n', '\r']) {
                return Err("a notice line is one row".to_owned());
            }
        }
        Ok(())
    }
}

/// Cut a line to [`MAX_NOTICE_BYTES`] on a char boundary (housing's `clamp`).
fn clamp(mut text: String) -> String {
    if text.len() > MAX_NOTICE_BYTES {
        let mut cut = MAX_NOTICE_BYTES;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        text.truncate(cut);
    }
    text
}

/// Tell `who` over a live transport, else queue it (including linkdead grace).
pub fn notify(ctx: &mut Ctx<'_>, who: ObjId, text: &str) {
    if ctx.w.is_connected(who) && ctx.w.has_live_conns(who) {
        ctx.out.sys_to_actor(who, "tacsim", text);
    } else {
        queue_notice(ctx, who, text);
    }
}

/// **How a surface line rides the reconnect queue** (doc 20 cut SM.11; the brief's lead ruling 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    /// A trip-level line — an unload, a plan's parking or end, an idle return, a stranded rig, a
    /// refused act: queued for an offline operator.
    Trip,
    /// Every other surface line — a cycle, a reading, a drive, a launch, a recover: said to a
    /// connected operator, never queued.
    Live,
}

/// **Tell `who` one surface line under the notice budget** (doc 20 §8 "Reconnect notices: at most
/// one a trip plus a summary", §11 Caps "≤ 1 notice a trip"; cut SM.11 lead ruling 7): connected,
/// the line is said, as [`notify`] does; offline it is queued only when it is a trip-level line —
/// an unload, a plan's parking or end, an idle return, a stranded rig, a refused act — never a
/// cycle, a reading, a drive, a launch or a recover. The ring's cap ([`MAX_NOTICE_LINES`]) stands.
pub fn notify_budget(ctx: &mut Ctx<'_>, who: ObjId, text: &str, level: NoticeLevel) {
    match delivery(ctx.w.is_connected(who) && ctx.w.has_live_conns(who), level) {
        Delivery::Say => ctx.out.sys_to_actor(who, "tacsim", text),
        Delivery::Queue => queue_notice(ctx, who, text),
        Delivery::Drop => {}
    }
}

/// What [`notify_budget`] does with one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivery {
    /// Said now.
    Say,
    /// Queued for the next login.
    Queue,
    /// Neither: an offline operator hears no live line.
    Drop,
}

/// [`notify_budget`]'s rule, pure: connected, every line is said; offline, a trip-level line is
/// queued and every other dropped.
fn delivery(connected: bool, level: NoticeLevel) -> Delivery {
    match (connected, level) {
        (true, _) => Delivery::Say,
        (false, NoticeLevel::Trip) => Delivery::Queue,
        (false, NoticeLevel::Live) => Delivery::Drop,
    }
}

/// Queue a line for `who`'s next login, writing the whole document. PLAYERS ONLY: a non-player
/// (a Thing principal such as IC8's clerk, which never connects) or a gone object gets nothing
/// queued, with a `tracing::debug!`. A serialization failure is a `tracing::error!` and writes
/// nothing (no `.expect`: the file's only `.expect` is `probe_error`'s).
pub fn queue_notice(ctx: &mut Ctx<'_>, who: ObjId, text: &str) {
    let player = ctx
        .w
        .get(who)
        .is_some_and(|obj| obj.kind == ObjKind::Player && !obj.flags.contains(Flags::GOING));
    if !player {
        tracing::debug!(
            who = who.0,
            "tacsim: a notice for a non-player or gone object is not queued"
        );
        return;
    }
    let at = ctx.w.now();
    let Some(store) = ctx.w.module_state_mut::<TacsimStore>() else {
        return;
    };
    let mut queue = store
        .notices
        .get(&who)
        .cloned()
        .unwrap_or_else(|| NoticeQueue::new(who));
    queue.push(at, text);
    let value = match serde_json::to_value(&queue) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                who = who.0, %error,
                "tacsim: a reconnect-notice queue did not serialize; nothing was queued"
            );
            return;
        }
    };
    store.notices.insert(who, queue);
    push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: notice_key(who),
            value,
        },
    );
}

/// The queued lines' texts, oldest first (tests and later status renders).
#[must_use]
pub fn pending_notices(w: &World, who: ObjId) -> Vec<String> {
    w.module_state::<TacsimStore>()
        .and_then(|store| store.notices.get(&who))
        .map(|queue| queue.lines.iter().map(|line| line.text.clone()).collect())
        .unwrap_or_default()
}

/// Renders a character's queued notices once on `Connected`, then deletes the
/// document. It renders whether or not `[tacsim].enabled` is on: the lines were
/// written by an enabled layer and are the player's own.
#[derive(Debug)]
pub struct ReconnectNotices;

impl EventHook for ReconnectNotices {
    // A linkdead session has no output recipient. Its trip notices are queued
    // just like a fully disconnected pilot's and consumed on the first revival.
    fn revived(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        self.connected(ctx, actor);
    }

    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        let queue = {
            let Some(store) = ctx.w.module_state_mut::<TacsimStore>() else {
                return;
            };
            match store.notices.remove(&actor) {
                Some(queue) if !queue.lines.is_empty() => queue,
                _ => return,
            }
        };
        ctx.out.sys_to_actor(actor, "tacsim", NOTICE_HEADING);
        for line in queue.lines {
            ctx.out.sys_to_actor(actor, "tacsim", line.text);
        }
        push_op(
            ctx.out,
            mush_core::ModuleWrite::Del {
                key: notice_key(actor),
            },
        );
    }
}

/// **The Registry inspection's descriptors of one kind** (`which`: 0 the actions, 1 the
/// outcomes): the inspection's own, and the fine's (doc 17h cut M8d D3), the principal
/// revocation's (D4), the quirk claim's (cut M8g G3) and the seizure's and forfeit's (cut
/// M8d2) when they compose. Its own frame, so
/// `register_with_coverage` stays under the M7 memory-size bound.
#[inline(never)]
fn inspection_descriptors(
    inspection: &crate::minds::registry_inspection::Composition,
    which: usize,
    into: &mut Vec<mush_core::DefinitionDescriptor>,
) -> Result<(), ModError> {
    let mut pairs = vec![inspection.descriptors()?];
    pairs.extend(inspection.fine_descriptors()?);
    pairs.extend(inspection.principal_descriptors()?);
    pairs.extend(inspection.claim_descriptors()?);
    for pair in pairs {
        let [action, outcome] = pair;
        into.push(if which == 0 { action } else { outcome });
    }
    // Doc 17h cut M8d2: the seizure's and the forfeit's, when they compose.
    if let Some([seize, seized, forfeit, forfeited]) = inspection.seizure_descriptors()? {
        into.push(if which == 0 { seize } else { seized });
        into.push(if which == 0 { forfeit } else { forfeited });
    }
    Ok(())
}

/// The Registry inspection's provider and slot (doc 17h cut M8a), and the fine's when it
/// composes (doc 17h cut M8d D3), over the registered references. Its own frame, so
/// `register_with_coverage` stays under the M7 memory-size bound.
#[inline(never)]
fn register_inspection(
    reg: &mut Registrar<'_>,
    inspection: &crate::minds::registry_inspection::Composition,
    action_refs: &BTreeMap<String, DefinitionRef>,
    outcome_refs: &BTreeMap<String, DefinitionRef>,
) -> Result<(), ModError> {
    let action = action_refs
        .get(crate::minds::registry_inspection::INSPECTION_ACTION)
        .cloned()
        .ok_or_else(|| ModError::msg("Registry inspection action unregistered"))?;
    let outcome = outcome_refs
        .get(crate::minds::registry_inspection::INSPECTION_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("Registry inspection outcome unregistered"))?;
    let fine = match (
        action_refs
            .get(crate::minds::registry_inspection::fine::FINE_ACTION)
            .cloned(),
        outcome_refs
            .get(crate::minds::registry_inspection::fine::FINE_OUTCOME)
            .cloned(),
    ) {
        (Some(action), Some(outcome)) => Some((action, outcome)),
        _ => None,
    };
    let principal = match (
        action_refs
            .get(crate::minds::registry_inspection::principal::PRINCIPAL_ACTION)
            .cloned(),
        outcome_refs
            .get(crate::minds::registry_inspection::principal::PRINCIPAL_OUTCOME)
            .cloned(),
    ) {
        (Some(action), Some(outcome)) => Some((action, outcome)),
        _ => None,
    };
    let claim = match (
        action_refs
            .get(crate::minds::registry_inspection::claims::CLAIM_ACTION)
            .cloned(),
        outcome_refs
            .get(crate::minds::registry_inspection::claims::CLAIM_OUTCOME)
            .cloned(),
    ) {
        (Some(action), Some(outcome)) => Some((action, outcome)),
        _ => None,
    };
    // Doc 17h cut M8d2: the seizure's four references, all or none.
    use crate::minds::registry_inspection::seizure as s;
    let seizure = match (
        action_refs.get(s::SEIZE_ACTION).cloned(),
        outcome_refs.get(s::SEIZE_OUTCOME).cloned(),
        action_refs.get(s::FORFEIT_ACTION).cloned(),
        outcome_refs.get(s::FORFEIT_OUTCOME).cloned(),
    ) {
        (Some(a), Some(b), Some(c), Some(d)) => Some([a, b, c, d]),
        _ => None,
    };
    inspection.register(reg, action, outcome, fine, principal, claim, seizure)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[u32]) -> Vec<ObjId> {
        list.iter().copied().map(ObjId).collect()
    }

    #[test]
    fn ic5_rotation_starts_at_the_cursor_and_wraps_once() {
        assert_eq!(rotation(ids(&[3, 5, 9]).into_iter(), 4), ids(&[5, 9, 3]));
        assert_eq!(rotation(ids(&[3, 5, 9]).into_iter(), 10), ids(&[3, 5, 9]));
        assert_eq!(rotation(ids(&[9, 3, 5]).into_iter(), 0), ids(&[3, 5, 9]));
    }

    #[test]
    fn ic5_the_cursor_moves_past_the_last_visited_agent() {
        assert_eq!(next_cursor(Some(ObjId(9)), 0), 10);
        assert_eq!(next_cursor(None, 7), 7);
        assert_eq!(next_cursor(Some(ObjId(u32::MAX)), 0), u32::MAX);
    }

    /// One simulated fire over `agents` (all due), each visit through
    /// [`arbitrate`] and the settle rule; returns who acted, in order.
    fn simulate(owed: &mut BTreeMap<ObjId, ObjId>, agents: &[(ObjId, ObjId, bool)]) -> Vec<ObjId> {
        let mut acted = BTreeSet::new();
        let mut actors = Vec::new();
        for (agent, principal, due) in agents {
            let step = if *due {
                match arbitrate(&acted, owed, *agent, *principal) {
                    Ok(()) => {
                        acted.insert(*principal);
                        actors.push(*agent);
                        Step::Submitted
                    }
                    Err(_) => Step::Deferred,
                }
            } else {
                Step::Idle
            };
            if step != Step::Deferred {
                settle_debts(owed, *agent);
            }
        }
        actors
    }

    #[test]
    fn ic5_agents_sharing_a_principal_take_turns() {
        let (a, b, c, p) = (ObjId(10), ObjId(11), ObjId(12), ObjId(2));
        let mut owed = BTreeMap::new();
        let mut order = Vec::new();
        for _ in 0..6 {
            order.extend(simulate(
                &mut owed,
                &[(a, p, true), (b, p, true), (c, p, true)],
            ));
        }
        assert_eq!(order, vec![a, b, c, a, b, c]);

        // A debt held by an agent that then idles is dropped, and the next due
        // agent acts.
        let mut owed = BTreeMap::new();
        assert_eq!(simulate(&mut owed, &[(a, p, true), (b, p, true)]), vec![a]);
        assert_eq!(owed.get(&p), Some(&b), "b holds the debt");
        assert_eq!(
            simulate(&mut owed, &[(a, p, true), (b, p, false), (c, p, true)]),
            vec![c],
            "a defers to b's debt, b idles and drops it, c acts"
        );
        assert!(owed.is_empty());

        // Another principal's agent is never deferred by P's debt.
        let (x, q) = (ObjId(20), ObjId(3));
        let mut owed = BTreeMap::new();
        assert_eq!(
            simulate(&mut owed, &[(a, p, true), (b, p, true), (x, q, true)]),
            vec![a, x]
        );
        assert_eq!(
            simulate(&mut owed, &[(a, p, true), (b, p, true), (x, q, true)]),
            vec![b, x]
        );
    }

    #[test]
    fn ic5_a_foreign_owner_read_maps_to_unreadable() {
        let foreign = ModuleActionStateReadError::ForeignOwner {
            context: mush_core::OwnerId::new("tacsim").unwrap(),
            family: mush_core::OwnerId::new("inventory").unwrap(),
        };
        assert!(matches!(
            pool_read_error(&foreign),
            PoolReadError::Unreadable(_)
        ));
        assert!(matches!(
            pool_read_error(&ModuleActionStateReadError::Unavailable),
            PoolReadError::Unreadable(_)
        ));
    }

    #[test]
    fn ic5_notice_key_round_trips() {
        assert_eq!(notice_key(ObjId(5)), "tacsim:notice:5");
        for x in [0, 5, u32::MAX] {
            assert_eq!(notice_key_owner(&notice_key(ObjId(x))), Some(ObjId(x)));
        }
        for bad in [
            "tacsim:notice:#5",
            "tacsim:notice:05",
            "tacsim:notice:",
            "tacsim:notice:-1",
        ] {
            assert_eq!(notice_key_owner(bad), None, "{bad:?}");
        }
        let key = ProbeKey {
            agent: ObjId(5),
            issue: 0,
        };
        assert_eq!(key.key_text(), "5:0");
        assert!(probe_key_text(key.agent, key.issue).is_some());
        assert!(IdempotencyKeyText::parse_canonical("tacsim.port.probe/5/0").is_ok());
    }

    #[test]
    fn ic5_notice_queue_keeps_the_newest_32() {
        let mut queue = NoticeQueue::new(ObjId(4));
        for n in 1..=40 {
            queue.push(n, &format!("line {n}"));
        }
        let texts: Vec<String> = queue.lines.iter().map(|l| l.text.clone()).collect();
        let expected: Vec<String> = (9..=40).map(|n| format!("line {n}")).collect();
        assert_eq!(texts, expected);
        queue.check().unwrap();
    }

    #[test]
    fn ic5_notice_lines_are_clamped_on_a_char_boundary() {
        // 239 ASCII bytes, then a 3-byte char straddling byte 240, then more.
        let mut text = "a".repeat(239);
        text.push('€');
        text.push_str(&"b".repeat(58));
        assert!(text.len() >= 300);
        let mut queue = NoticeQueue::new(ObjId(4));
        queue.push(0, &text);
        let line = &queue.lines[0].text;
        assert!(line.len() <= MAX_NOTICE_BYTES);
        assert_eq!(line.len(), 239, "the straddling char is dropped whole");
        assert!(std::str::from_utf8(line.as_bytes()).is_ok());
        queue.push(0, "a\nb");
        assert_eq!(queue.lines[1].text, "a b");
    }

    /// **The budget queues a trip, not a cycle** (doc 20 cut SM.11; lead ruling 7): offline a
    /// trip-level line is queued and a live line is not; connected both are said. The world proof
    /// is `tests/plans.rs`'s P13.
    #[test]
    fn sm11_the_budget_queues_a_trip_not_a_cycle() {
        assert_eq!(delivery(false, NoticeLevel::Trip), Delivery::Queue);
        assert_eq!(delivery(false, NoticeLevel::Live), Delivery::Drop);
        assert_eq!(delivery(true, NoticeLevel::Trip), Delivery::Say);
        assert_eq!(delivery(true, NoticeLevel::Live), Delivery::Say);
    }

    #[test]
    fn ic5_notice_document_wire_is_strict() {
        let good = json!({ "schema": 1, "who": 4, "lines": [{ "at": 0, "text": "ok" }] });
        let queue: NoticeQueue = serde_json::from_value(good).unwrap();
        queue.check().unwrap();
        let refused = |value: Value| {
            serde_json::from_value::<NoticeQueue>(value)
                .map_err(|e| e.to_string())
                .and_then(|q| q.check())
                .is_err()
        };
        assert!(refused(
            json!({ "schema": 1, "who": 4, "lines": [], "extra": 1 })
        ));
        assert!(refused(json!({ "schema": 2, "who": 4, "lines": [] })));
        let many: Vec<Value> = (0..33).map(|_| json!({ "at": 0, "text": "x" })).collect();
        assert!(refused(json!({ "schema": 1, "who": 4, "lines": many })));
        assert!(refused(
            json!({ "schema": 1, "who": 4, "lines": [{ "at": 0, "text": "x".repeat(241) }] })
        ));
        assert!(refused(
            json!({ "schema": 1, "who": 4, "lines": [{ "at": 0, "text": "a\nb" }] })
        ));
    }
}
