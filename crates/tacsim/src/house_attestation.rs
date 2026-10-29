//! **House owner attestation in live play** (doc 17h §3 M8h; the lead's rulings A1–A6 of the
//! M8h brief, 2026-10-29). Closes 17g §4's first deferral: M8c built the House arm of
//! `@mind/attest <filed> org=<House>`, but the launch's Houses are NPC organisations with no
//! curator, so every House filing's free owner licence lapsed after C0's 365-day runway. This
//! tick attests each due House filing unattended, as the House's agent, and the 365-day term
//! becomes a renewal cycle (A4).
//!
//! **The agent** (A1): each House's attesting character is its procurement policy's agent where
//! it has one, else the House's own `attestation_agent`
//! ([`crate::launch::LaunchDeployment::house_attestation_agents`]); a House with neither is not
//! visited. The agent attests through its `roll.holder.org` grant on the House's scope, the
//! scope M8c's House arm asks, asked as a read (the attestation charges nothing, so a limited
//! grant attests: M8f H7).
//!
//! **The tick** (`tacsim.house_attestation`, [`HOUSE_ATTESTATION_TICK`], A2): Coarse, GOD, and it
//! submits nothing authoritative itself. Each fire it walks the owner licences through the Roll's
//! GOD-only `HouseAttestationDue` read (at most [`MAX_READS_PER_FIRE`] reads, resuming on the
//! next fire), keeping the House licences whose current generation is due by the House
//! provider's own rule (the term ended, Grace has not). Then, for each House, it queues at most
//! ONE attestation: `@mind/attest`'s House action for the first due filing, with
//! `org=<House>`, submitted as the agent through the port tick's swap
//! ([`crate::port_tick::Fire`]). The handler is infallible: a refusal pages a log line, never a
//! player notice (A5), and the House moves on.
//! - An **Unattestable** generation (an ungoverned seal mark inside the term, M8c B5) is never
//!   submitted: it lapses, which is the consequence, and `tacsim.house_attestation.unattestable`
//!   logs once.
//! - A **revoked** generation is never submitted (the provider refuses it).
//! - A generation whose attestation was queued [`MAX_ATTEST_TRIES`] times and is still due is
//!   given up with one page.
//!
//! **The memory** (A2): none of its own. The next generation's existence is the record: a landed
//! attestation makes the generation not due, so a restart between due and attestation re-reads
//! the still-due row and attests once, and a restart after it reads nothing due. RAM
//! ([`HouseAttestationState`]) only paces: the walk's cursor, the due queue, the tries and the
//! pages (which a restart forgets, so a page is logged once per boot).
//!
//! **No double attestation** (A3): a human curator and the tick can both attest; the provider
//! refuses a second attestation of a generation (the first makes it not due), and a generation a
//! curator attested first is never due to the tick.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar, TickCadence};
use mush_core::{Ctx, Flags, IdempotencyKeyText, ObjId, ObjKind, PerceivedRef, SceneId, World};
use mush_org::OrgId;
use mush_roll::commerce::LicenceId;
use mush_roll::commerce::command_actions::RollCommandActions;
use mush_roll::commerce::command_reader::RollCommandReadRequest;
use mush_roll::commerce::holding_reader::OriginalReadRequest;
use mush_roll::commerce::house_attestation_read::HouseAttestationDue;
use serde_json::json;

use crate::port_tick::{Fire, Step};

/// The tick's tag.
pub const HOUSE_ATTESTATION_TICK: &str = "tacsim.house_attestation";
/// How often it fires (Coarse). A House generation is due for its whole Grace, so the cadence
/// only bounds how soon after its term ends a filing is attested.
pub const HOUSE_ATTESTATION_TICK_PERIOD: Duration = Duration::from_secs(5);
/// `HouseAttestationDue` reads one fire makes at most (each walks
/// `house_attestation_read::HOUSE_LICENCES_PER_READ` owner licences); the walk resumes on the
/// next fire.
pub const MAX_READS_PER_FIRE: usize = 4;
/// Times one generation's attestation is queued while it stays due before the tick gives it up
/// (until a restart) with one page.
pub const MAX_ATTEST_TRIES: u8 = 3;

/// **The tick's RAM slot**: pacing only (the module note's memory).
#[derive(Clone, Debug, Default)]
pub(crate) struct HouseAttestationState {
    /// Each House's attesting character, set once the launch composes
    /// (`crate::launch::registration::Registration::finalize`); empty, the tick visits nothing.
    agents: Vec<(OrgId, ObjId)>,
    /// Where the walk resumes; `None`, the next read starts a new walk.
    cursor: Option<LicenceId>,
    /// The due House licences the walk found and the tick has not yet handled.
    queue: BTreeMap<LicenceId, HouseAttestationDue>,
    /// Attestations queued per `(licence, generation)`.
    tries: BTreeMap<(LicenceId, u64), u8>,
    paged: BTreeSet<String>,
    /// `(IC second, ordinal)` of the submission keys.
    keys: (i64, u64),
}

impl HouseAttestationState {
    /// The launch's attesting characters (set at finalize; boot re-sets them).
    pub(crate) fn set_agents(&mut self, agents: Vec<(OrgId, ObjId)>) {
        self.agents = agents;
    }
    /// Test only: the attesting characters the launch handed over.
    #[cfg(test)]
    pub(crate) fn agents(&self) -> &[(OrgId, ObjId)] {
        &self.agents
    }
    /// Test only: the pages logged so far, and the tries per `licence/generation`.
    #[cfg(test)]
    pub(crate) fn describe(&self) -> String {
        format!(
            "paged={:?} tries={:?} queue={:?}",
            self.paged,
            self.tries
                .iter()
                .map(|((licence, generation), n)| format!("{}/{generation}={n}", licence.get()))
                .collect::<Vec<_>>(),
            self.queue.keys().map(|l| l.get()).collect::<Vec<_>>()
        )
    }
    /// Log `what` once (a staff log line, never a player notice).
    fn page(&mut self, org: OrgId, what: &str, detail: &str) {
        if self.paged.insert(format!("{}/{what}", org.0)) {
            tracing::warn!(
                log_id = "tacsim.house_attestation.refused",
                org = org.0,
                what,
                detail,
                "tacsim: a House's attestation is refused; the House moves on"
            );
        }
    }
    fn key(&mut self, at: i64, org: OrgId, subject: &str) -> Option<IdempotencyKeyText> {
        if self.keys.0 != at {
            self.keys = (at, 0);
        }
        let ordinal = self.keys.1;
        self.keys.1 = ordinal.checked_add(1)?;
        IdempotencyKeyText::parse_canonical(format!(
            "house_attestation.{}.{subject}.{at:x}.{ordinal:x}",
            org.0
        ))
        .ok()
    }
}

/// **Register the House attestation tick** (doc 17h §3 M8h): its RAM slot and the Coarse
/// `tacsim.house_attestation` tick, unconditionally (a world with no agent visits nothing).
/// Called from `crate::port_tick::register_with_coverage`, beside procurement.
///
/// # Errors
///
/// Any registration the registrar refuses.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(HouseAttestationState::default());
    reg.tick(
        HOUSE_ATTESTATION_TICK,
        TickCadence::Coarse(HOUSE_ATTESTATION_TICK_PERIOD),
        Arc::new(house_attestation_tick),
    )
}

/// **The tick** (the module note).
pub(crate) fn house_attestation_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    if !ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_some_and(crate::TacsimStore::enabled)
    {
        return;
    }
    let Some(mut state) = ctx.w.module_state::<HouseAttestationState>().cloned() else {
        return;
    };
    if state.agents.is_empty() {
        return;
    }
    walk(ctx, &mut state);
    let action = ctx
        .reg
        .definitions()
        .and_then(|definitions| RollCommandActions::capture(definitions).attest_house());
    let mut fire = Fire::new(BTreeMap::new());
    for (org, agent) in state.agents.clone() {
        let step = match &action {
            Some(action) => visit(ctx, &mut fire, &mut state, (org, agent), action),
            None => {
                state.page(
                    org,
                    "unwired",
                    "the House attestation action is not registered",
                );
                Step::Refused
            }
        };
        fire.settle(agent, step);
    }
    if let Some(slot) = ctx.state_mut::<HouseAttestationState>() {
        *slot = state;
    }
}

/// Advance the walk within this fire's read budget, queueing every due House licence it finds.
/// A walk that starts afresh drops the last walk's queue first, so a queued row is at most one
/// walk old. An unreadable page pages once and the walk resumes from the same place next fire.
fn walk(ctx: &Ctx<'_>, state: &mut HouseAttestationState) {
    let Some(reader) = crate::match_::original_access::reader(ctx.w) else {
        if let Some(&(org, _)) = state.agents.first() {
            state.page(
                org,
                "unwired",
                "the Roll's original-holdings read is not composed",
            );
        }
        return;
    };
    for _ in 0..MAX_READS_PER_FIRE {
        if state.cursor.is_none() {
            state.queue.clear();
        }
        let page = ctx
            .read_owner(
                &reader,
                OriginalReadRequest::command(RollCommandReadRequest::house_attestation_due(
                    state.cursor,
                )),
            )
            .ok()
            .and_then(mush_roll::commerce::holding_reader::OriginalReadResult::into_command)
            .and_then(
                mush_roll::commerce::command_reader::RollCommandReadResult::into_house_attestation_due,
            );
        let Some(page) = page else {
            if let Some(&(org, _)) = state.agents.first() {
                state.page(org, "walk", "the House owner licences are unreadable");
            }
            return;
        };
        for row in page.rows() {
            state.queue.insert(row.licence(), *row);
        }
        state.cursor = page.next();
        if state.cursor.is_none() {
            // The walk is complete; the next fire starts another.
            return;
        }
    }
}

/// Whether `agent` is a live, non-GOING Player-kind character.
fn live(w: &World, agent: ObjId) -> bool {
    w.get(agent)
        .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
}

/// One House's one step: the first due filing of the House's the tick may attest, attested as
/// the agent; the ones it must not attest are dropped with one page each.
fn visit(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    state: &mut HouseAttestationState,
    (org, agent): (OrgId, ObjId),
    action: &mush_core::DefinitionRef,
) -> Step {
    if ctx.authoritative_pending(agent) {
        // The agent's last step has not landed; read it next fire (D-18-32).
        return Step::Deferred;
    }
    let due: Vec<HouseAttestationDue> = state
        .queue
        .values()
        .filter(|row| row.organization() == org)
        .copied()
        .collect();
    for row in due {
        let (licence, generation) = (row.licence(), row.generation());
        let subject = format!("{}/{generation}", licence.get());
        let tries = state
            .tries
            .get(&(licence, generation))
            .copied()
            .unwrap_or(0);
        if row.unattestable() {
            state.queue.remove(&licence);
            if state
                .paged
                .insert(format!("{}/unattestable {subject}", org.0))
            {
                tracing::warn!(
                    log_id = "tacsim.house_attestation.unattestable",
                    org = org.0,
                    licence = licence.get(),
                    generation,
                    "tacsim: a House filing flew on an ungoverned core; its generation lapses"
                );
            }
            continue;
        }
        if row.revoked() {
            state.queue.remove(&licence);
            state.page(
                org,
                &format!("revoked {subject}"),
                "the Registry revoked the generation; it is not attested",
            );
            continue;
        }
        if tries >= MAX_ATTEST_TRIES {
            state.queue.remove(&licence);
            state.page(
                org,
                &format!("given up {subject}"),
                "the attestation never landed; the generation is left to lapse",
            );
            continue;
        }
        if !live(ctx.w, agent) {
            state.page(
                org,
                "agent",
                "the attestation agent is gone or not a character",
            );
            return Step::Refused;
        }
        // The grant the House arm asks, asked here first so an ungranted agent queues nothing.
        if mush_roll::commerce::org_holder::org_actor_amount(
            ctx.w,
            org,
            row.scope(),
            agent,
            mush_roll::commerce::org_holder::READ_AMOUNT,
        )
        .is_err()
        {
            state.page(
                org,
                "grant",
                "the attestation agent holds no roll.holder.org grant on the House's scope",
            );
            return Step::Refused;
        }
        let at = ctx.w.world_ic_now().at;
        let Some(key) = state.key(at, org, &format!("{}.{generation}", licence.get())) else {
            state.page(org, "key", "an attestation key did not form");
            return Step::Refused;
        };
        let parameters = json!({"owner_licence": licence.get(), "org": org.0});
        let step = crate::port::stock::swapped(fire.as_principal(ctx, agent, agent, |ctx| {
            ctx.submit_authoritative(
                action.clone(),
                PerceivedRef::SelfActor,
                Vec::new(),
                parameters,
                key,
            );
        }));
        if step == Step::Submitted {
            state.queue.remove(&licence);
            let count = state.tries.entry((licence, generation)).or_insert(0);
            *count = count.saturating_add(1);
        }
        return step;
    }
    Step::Idle
}
