//! **The port tick** (doc 18 cut IC5; §4.3 "Finite seeded stock", §4.4 "The
//! queued-orders model" and "Offline execution", D-18-32, D-18-33).
//!
//! `tacsim.port` is the one pull tick every port arm runs on (Coarse, 5 s). It
//! obeys the tick-cadence rule (`mush-campaign/src/tick.rs:28-37`): a tick runs
//! as GOD and submits nothing authoritative itself, because two GOD submissions
//! in one drain void each other. So every authoritative act this tick makes is
//! made by [`Fire::as_principal`], which swaps `ctx.actor` to the principal an
//! agent acts for (the `mush-npc` precedent) for ONE closed call, and counts that
//! call as the principal's one act this fire (D-18-32: at most one authoritative
//! submission per actor per fire). **Nothing in the tick calls
//! `ctx.submit_authoritative` outside a closure passed to `as_principal`.** GOD,
//! the tick's own actor, therefore never submits.
//!
//! The walk is the campaign rotation (`mush-campaign/src/tick.rs:94-157`): a
//! cursor over agent ids, at most [`MAX_AGENTS_VISITED_PER_FIRE`] visits and
//! [`MAX_AGENTS_WORKED_PER_FIRE`] units of work per fire, the cursor moved past
//! the last visited agent. Fairness between agents sharing one principal is a
//! RAM debt per principal (lead ruling L8, [`arbitrate`]).
//!
//! The arms it will carry, in dispatch order ahead of the probe:
//!
//! - IC8 — the stock arm (`tacsim_port_stock`: mint + receipt) and the
//!   lapsed-hold release: **not yet**;
//! - IC9 — the launch and dock arms (the ticket's consumer): **not yet**;
//! - IC11 — the order arm (the queue's offline execution): **not yet**.
//!
//! Today it carries one arm, **the probe** (lead ruling L3), registered only in a
//! test world: an agent that acts as its principal once per remaining count,
//! submitting `tacsim_port_probe`, whose provider writes one mark into the
//! test-only family `tacsim.port_probe`. Production composes the families, the
//! empty validators, an idle tick and the notice hook (lead rulings L4, L12).
//!
//! **Reconnect notices** live here too (§4.4 "Failure handling", lead ruling
//! L10): [`notify`] tells a connected player at once and otherwise queues a line
//! in a bounded per-player document (`tacsim:notice:{obj}`), and
//! [`ReconnectNotices`] renders the queue once on `Connected` under
//! [`NOTICE_HEADING`], then deletes it.

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
use mush_inventory::InventoryHandleCell;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::port::families::{self, PortHandles};
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

/// Register the port tick (doc 18 cut IC5): the two typed families, the crate's
/// one `outcome` and one `action` validator, in a test world the probe (family,
/// definitions and provider), [`PortHandles`], [`PortTickState`], the
/// `tacsim.port` tick and the [`ReconnectNotices`] hook.
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
) -> Result<(), ModError> {
    let port_families = families::register(reg)?;
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
    // One `outcome` validator for the whole crate, over the very descriptors
    // registered next (built once, so the two cannot drift).
    let outcomes = outcome_descriptors(test_world)?;
    reg.definition_validator(
        kind("outcome")?,
        crate::definitions::exact_validator(outcomes.clone()),
    )?;
    let mut outcome_refs: BTreeMap<String, DefinitionRef> = BTreeMap::new();
    for descriptor in outcomes {
        let id = descriptor.id().as_str().to_owned();
        outcome_refs.insert(id, reg.definition(descriptor)?);
    }
    let probe_outcome = outcome_refs.get(PROBE_OUTCOME).cloned();
    // One `action` validator for the whole crate, likewise.
    let actions = action_descriptors(probe_outcome.as_ref())?;
    reg.definition_validator(
        kind("action")?,
        crate::definitions::exact_validator(actions.clone()),
    )?;
    let mut action_refs: BTreeMap<String, DefinitionRef> = BTreeMap::new();
    for descriptor in actions {
        let id = descriptor.id().as_str().to_owned();
        action_refs.insert(id, reg.definition(descriptor)?);
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
    reg.state(PortHandles {
        families: port_families,
        inventory: inventory.and_then(|cell| cell.get().cloned()),
        probe,
    });
    reg.state(PortTickState::new(probe_action));
    reg.tick(
        PORT_TICK,
        TickCadence::Coarse(PORT_TICK_PERIOD),
        Arc::new(port_tick),
    )?;
    reg.hook(HookPoint::Connected, Arc::new(ReconnectNotices));
    Ok(())
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

/// **Every `outcome` definition this crate registers** — empty in production; the
/// probe's in a test world. IC8's `tacsim_port_stock` and IC9's
/// `tacsim.port.burn` add their outcome descriptors to this list, in this same
/// order: a second `definition_validator` call for one kind is a registration
/// conflict, so there is exactly one validator per kind for the crate.
fn outcome_descriptors(test_world: bool) -> Result<Vec<DefinitionDescriptor>, ModError> {
    let mut descriptors = Vec::new();
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
/// outcome's exact `DefinitionRef` — empty when no probe outcome is registered
/// (production). IC8's `tacsim_port_stock` and IC9's `tacsim.port.burn` add
/// their action descriptors to this list, in this same order (one validator per
/// kind for the crate).
fn action_descriptors(
    probe_outcome: Option<&DefinitionRef>,
) -> Result<Vec<DefinitionDescriptor>, ModError> {
    let mut descriptors = Vec::new();
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
}

impl PortTickState {
    fn new(probe_action: Option<DefinitionRef>) -> PortTickState {
        PortTickState {
            probe_action,
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
}

/// The last fire's stats, `None` before the first fire that found the layer on.
#[must_use]
pub fn last_fire(w: &World) -> Option<PortFireStats> {
    w.module_state::<PortTickState>()
        .and_then(|state| state.last_fire)
}

// ---- the fire ----------------------------------------------------------------

/// One agent's turn this fire (D-18-32: one step per agent per fire).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Nothing due.
    Idle,
    /// A unit of work that staged nothing authoritative (later arms' pull verification).
    /// No IC5 arm returns it (the probe either submits or does not); the walk
    /// already accounts it, so IC8's first pull-verifying arm needs no change there.
    #[allow(dead_code)]
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
}

/// The arms, in dispatch order (one step per agent per fire, D-18-32). IC8, IC9
/// and IC11 add variants ahead of the probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arm {
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
    let Some(slot) = ctx.w.module_state_mut::<PortTickState>() else {
        return;
    };
    // The slot is taken for the fire and written back whole at its end; nothing
    // the fire calls reads it (the probe's offer is projected at landing, after).
    let mut state = std::mem::take(slot);
    let stats = walk(ctx, &mut state);
    state.last_fire = Some(stats);
    if let Some(slot) = ctx.w.module_state_mut::<PortTickState>() {
        *slot = state;
    }
}

/// Collect, order and walk the agents; returns the fire's stats and leaves the
/// cursor, the debts and the warned set updated in `state`.
fn walk(ctx: &mut Ctx<'_>, state: &mut PortTickState) -> PortFireStats {
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
        match step {
            Step::Idle => {}
            Step::Worked => fire.stats.worked = fire.stats.worked.saturating_add(1),
            Step::Submitted => {
                fire.stats.worked = fire.stats.worked.saturating_add(1);
                fire.stats.submitted = fire.stats.submitted.saturating_add(1);
            }
            Step::Deferred => fire.stats.deferred = fire.stats.deferred.saturating_add(1),
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
/// per agent per fire). IC8, IC9 and IC11 add variants ahead of the probe; the
/// probe arm runs only when `PortTickState.probe_action` is `Some`.
fn step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    agent: ObjId,
    principal: ObjId,
    arm: Arm,
) -> Step {
    match arm {
        Arm::Probe => match state.probe_action.clone() {
            Some(action) => probe_step(ctx, state, fire, &action, agent, principal),
            None => Step::Idle,
        },
    }
}

/// Whether `id` names no live object (absent, or marked `GOING`).
fn gone(w: &World, id: ObjId) -> bool {
    w.get(id).is_none_or(|obj| obj.flags.contains(Flags::GOING))
}

// ---- the swap and the turn rule --------------------------------------------

/// One fire's ledger of who has acted (D-18-32: at most one authoritative submission per actor per
/// fire) and of the round-robin debts between agents sharing a principal. A swapped call counts as
/// its principal's ONE act this fire whether or not it submitted, so a closed call that submits
/// internally (IC11's `trade::quote` + `accept_into`) can never fan out with a second submission
/// FROM THIS TICK in the same drain (`engine.rs:4043-4064`).
///
/// The guarantee is per tacsim fire, NOT per drain. `Engine::fire_due` (`engine.rs:3213-3263`)
/// dispatches every due `@wait` line as its actor and runs every other due module tick into the
/// same outbox before its ONE `drain_deferred`, and this ledger cannot see their deferred
/// submissions. A principal whose own `@wait` command (or another module's swapped tick) submits
/// in the same `fire_due` slice as an arm's swapped call fans out in that drain: the engine voids
/// both and tells the principal "That command tried to submit more than one authoritative action;
/// nothing was staged." An arm's step recovers by pull on a later fire (D-18-32), exactly as for
/// back-pressure ([`Step::Submitted`]); the player's own `@wait` command is the collateral. A
/// residual for IC8/IC9/IC11, which would need a core seam (the outbox asked whether an actor
/// already has a deferred submission) to close.
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
}

impl Fire {
    fn new(owed: BTreeMap<ObjId, ObjId>) -> Fire {
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
    /// (the `mush-npc/src/module.rs:1499-1502` and `seed.rs:385-398` precedent). Calls
    /// [`Fire::turn`] first; on success inserts `principal` into `acted` before the call.
    pub(crate) fn as_principal<R>(
        &mut self,
        ctx: &mut Ctx<'_>,
        agent: ObjId,
        principal: ObjId,
        call: impl FnOnce(&mut Ctx<'_>) -> R,
    ) -> Result<R, SwapRefused> {
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
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed) => return Step::Deferred,
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
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed) => return Step::Deferred,
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

/// The file's one `ActionError` builder and its only `.expect` (the bounded
/// static-message idiom of every provider in the tree, `mush-sim/src/handoff.rs`).
fn probe_error(code: ActionErrorCode, message: &'static str) -> ActionError {
    ActionError::new(code, false, message).expect("static port-probe error is bounded")
}

/// The probe invocation's three parameters, checked.
fn probe_parameters(invocation: &ActionInvocation) -> Result<(ObjId, u32, u64), ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(probe_error(
            ActionErrorCode::InvalidRequest,
            "the port probe is self-scoped and takes no targets",
        ));
    }
    let invalid = || {
        probe_error(
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
            probe_error(
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
            probe_error(
                ActionErrorCode::Integrity,
                "the port probe's definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                probe_error(
                    ActionErrorCode::Integrity,
                    "the port probe's guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                probe_error(
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
                probe_error(
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
                probe_error(
                    ActionErrorCode::Integrity,
                    "the port probe's presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                probe_error(
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
            probe_error(
                ActionErrorCode::Integrity,
                "the port probe's marks are unavailable",
            )
        })?;
        if prior.is_some() {
            // The replay guard IC8's receipt copies: one mark per (agent, issue).
            return Err(probe_error(
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
            probe_error(
                ActionErrorCode::Integrity,
                "the port probe's mark did not stage",
            )
        })?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "actor": actor.0, "agent": agent.0, "issue": issue, "seen": seen }),
        )
        .map_err(|_| {
            probe_error(
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

/// Tell `who` now if connected (one `sys_to_actor(who, "tacsim", text)`), else queue it.
pub fn notify(ctx: &mut Ctx<'_>, who: ObjId, text: &str) {
    if ctx.w.is_connected(who) {
        ctx.out.sys_to_actor(who, "tacsim", text);
    } else {
        queue_notice(ctx, who, text);
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
