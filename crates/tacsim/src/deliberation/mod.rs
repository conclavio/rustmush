//! **Deliberation** (16b cut N.2, N3; doc 16 §6 "Deliberation", "Provider
//! failure", "The cost run"; 16a Q9/Q9b; 16b §8's cut N row).
//!
//! Cut N.1 built the reflex controller: a deterministic machine that patrols,
//! closes, shoots and breaks off, and never asks anybody anything. This cut is the
//! layer above it — **a mind that tells a squad what the fight is for** — and it is
//! deliberately thin, because doc 16 §6 says so: *not in the story; the hook is
//! the order type and the squad grouping*.
//!
//! ```text
//!   arena  ── SquadSignal ─▶  world task ── Effect::ModuleAsync ─▶  LlmService
//!     ▲                           │                                     │
//!     │                           ◀── WorldCommand::ModuleAsyncDone ────┘
//!     └────────── IntentKind::Order ───────┘
//! ```
//!
//! Four pieces, in that order:
//!
//! 1. **[`crate::deliberation::order`]** — the [`crate::deliberation::Order`] type a squad is steered by. Plain data,
//!    imported *into* the arena, carried as an [`crate::arena::intents::Intent`]
//!    and obeyed until its time to live runs out in arena milliseconds.
//! 2. **[`crate::deliberation::squad`]** — the grouping the order is written for: a side's reflex units
//!    dealt into squads of at most `[tacsim.arena].squad_size` (16b cut Z.1), each with the
//!    lowest-id unit as its commander.
//! 3. **[`crate::deliberation::request`]** — one structured call whose forced tool's arguments are an
//!    order.
//! 4. **[`crate::deliberation::cost`]** — the table the owner's cost run prints, read off `mush-llm`'s
//!    own budget and audit records.
//!
//! # What runs where, and why nothing waits
//!
//! The **trigger** is the arena's, because doc 16 §6 puts it on the commander's
//! contact set and only the arena can see one; it is floored by
//! `[tacsim.arena].deliberate_min_ms` in arena time and reported on
//! [`crate::arena::step::StepOutput::deliberation`]
//! ([`crate::arena::command`] is the whole of it).
//!
//! The **request** is the world's. `crate::arena_bridge` turns each signal into
//! one [`mush_core::Effect::ModuleAsync`] — the M7 LLM re-entry seam that worldgen,
//! the NPC turn and the weather generator already run on — and the world task
//! spawns it off-loop against the shared [`mush_llm::LlmService`]. The completion
//! comes back as `WorldCommand::ModuleAsyncDone`, is routed to `tacsim`'s
//! registered `AsyncCompletion`, and becomes one ordinary
//! [`crate::arena::intents::IntentKind::Order`] per unit of the squad, stamped with
//! the unit's published `helm` epoch and offered to the same inbox a cockpit verb
//! offers to (`crate::arena_bridge::send_order`, which is `forward_intent` without
//! the four checks that are all about *a player's* authority over a tank — there is
//! no seat here. Since **17b cut L.1** (doc 17 §4, D-UM-52) the authority an order
//! needs is two: a character entitled to order the squad — here the squad's
//! lance-leader, named in [`crate::deliberation::Order::by`] and checked against
//! the match record by `crate::arena_bridge::admit_order` — and the unit on a
//! machine controller, which the arena checks itself at apply).
//!
//! So a pulse never waits for a provider, an arena never holds a service handle,
//! and doc 16 §6's *the arena only ever sees an intent or nothing* is literally
//! what the code does.
//!
//! # The deviation this cut records
//!
//! 16b §8's cut N row asks for the request to go *through the existing durable
//! decision pipeline* (`mush_core::decision`, the K7 kernel `mush-npc` composes).
//! It does not. It goes through the **LLM re-entry seam** instead, and uses
//! `LlmService`'s own admission, budget, kill switch, retry, breaker and audit —
//! the same ones every durable decision's provider goes through, at the same
//! `Priority::Background`. Three reasons, in order of weight:
//!
//! 1. **The cut's own two requirements are in tension.** The row also asks for the
//!    structured call hoisted in cut R-a (`mush_llm::call_structured`), and the
//!    durable pipeline's provider deliberately does **not** use it: it forces one
//!    tool chosen from N *projected offers* and takes no repair round trip,
//!    because a repair burns lease time (`mush-server/src/decision_llm.rs`). An
//!    order is not a projected offer — it is a typed record with six objectives
//!    and two constraints — so the offer protocol would have had to be bent around
//!    it, and the repair turn the row asks for thrown away.
//! 2. **Durability would preserve the wrong thing.** Since 16b cut AF a match
//!    survives a server-half restart: the arena resumes from its last snapshot
//!    and the world releases every seat to doctrine. What a restart does NOT
//!    preserve is a deliberation in flight — the `Effect::ModuleAsync` died with
//!    the process and its reply has nowhere to land. The arena carries its
//!    standing orders and its squad watch across the resume, so the machines
//!    keep obeying the last order and a squad is asked again at its next trigger
//!    after the floor — an expiry it had already flagged, or a change in what its
//!    commander holds — about the field as it then stands. A K7 decision would
//!    recover the old question into the resumed match and answer it late,
//!    against a field the snapshot has moved on from; for an order with a
//!    thirty-second floor and a time to live in seconds, re-asking is the right
//!    answer and durability is the wrong one.
//! 3. **The cost is a cut of its own.** A second decision source means a second
//!    owner, four kernel state families, three action definitions, three action
//!    providers, a fallback adapter, a wake adapter and an arm in
//!    `world_task::production_decision_providers` (whose absence *fails the boot*),
//!    and it moves the module-namespace and state-slot gates and the boot audit —
//!    which freezes an owner on a malformed pending record. That is not the *hook*
//!    doc 16 §6 asks this cut for.
//!
//! What is **not** given up: the floor, the time to live, the doctrine fallback,
//! the one in-world notice, the per-squad request, the bounded admission and the
//! metered budget. If a later cut wants the kernel's durability, the order type,
//! the squad grouping, the trigger and the cost table are all it would keep.

pub mod cost;
pub mod order;
pub mod request;
pub mod squad;

use serde::{Deserialize, Serialize};

pub use cost::{CostGuard, CostRow, CostTable, cost_table};
pub use order::{
    CharacterRef, Constraints, MAX_TTL_MS, MIN_TTL_MS, MIND_LINK_DROPPED, Objective, Order,
    OrderError, RangeBand, StandingOrder,
};
pub use request::{BriefMember, RefusedReason, SquadBrief, deliberate, order_tool};
pub use squad::{
    LanceLeader, SQUAD_SIZE_MAX, SquadId, SquadRoster, fill_leaders, leader_of, squads_of,
};

/// The most deliberation requests that may be in flight for one match at once.
///
/// One per squad and a little slack, capped well under the field: the real bound
/// is `[tacsim.arena].deliberate_min_ms` and `mush-llm`'s own background
/// admission, and this is the belt that stops a match whose provider has stopped
/// answering from queueing a request per pulse for ever.
pub const MAX_IN_FLIGHT_PER_MATCH: usize = 8;

/// **The job the world task runs off-loop** — the payload
/// [`mush_core::Effect::ModuleAsync`] carries, JSON as a string, exactly as
/// worldgen's and the NPC turn's do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "tacsim")]
pub enum DeliberationJob {
    /// Ask one squad's commander for an order.
    #[serde(rename = "deliberate")]
    Deliberate {
        /// What the commander can see.
        brief: SquadBrief,
    },
}

/// **What came back.** Either an order, or a named reason there is none — and a
/// named reason is doc 16 §6's fallback, which is the reflex doctrine and not
/// idle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "outcome", rename_all = "snake_case")]
pub enum DeliberationReply {
    /// One order for one squad.
    Ok {
        /// Which match it was asked for.
        match_id: u64,
        /// The order.
        order: Order,
        /// The model that wrote it, for the staff line and the cost table.
        model: String,
        /// The prompt hash, so a reader can find the order's audit row.
        prompt_hash: String,
    },
    /// No order, and why.
    Refused {
        /// Which match it was asked for.
        match_id: u64,
        /// Which squad, rendered.
        squad: String,
        /// One of [`RefusedReason`]'s words.
        reason: String,
        /// The prompt hash, so a reader can find the failure's audit row.
        prompt_hash: String,
    },
}

impl DeliberationReply {
    /// Which match this reply is about.
    #[must_use]
    pub const fn match_id(&self) -> u64 {
        match self {
            DeliberationReply::Ok { match_id, .. }
            | DeliberationReply::Refused { match_id, .. } => *match_id,
        }
    }
}

/// **Run one deliberation job off the world loop** — the entry point
/// `mush_server::world_task` routes `Effect::ModuleAsync { module: "tacsim" }` to,
/// in the shape `mush_worldgen::run_worldgen_async` and
/// `mush_npc::run_npc_turn_async` already have.
///
/// It never panics and never returns anything but a decodable
/// [`DeliberationReply`]: a payload this module serialized itself and cannot decode
/// is a bug, and it comes back as an `unavailable` refusal rather than as a
/// dropped completion, because a dropped completion would leave the world's
/// in-flight count wrong for the life of the match.
pub async fn run_deliberation_async(llm: &mush_llm::LlmService, payload: &str) -> String {
    let reply = match serde_json::from_str::<DeliberationJob>(payload) {
        Ok(DeliberationJob::Deliberate { brief }) => {
            let match_id = brief.match_id;
            let squad = brief.squad.to_string();
            match deliberate(llm, &brief).await {
                Ok(order) => DeliberationReply::Ok {
                    match_id,
                    order,
                    model: String::new(),
                    prompt_hash: String::new(),
                },
                Err(error) => DeliberationReply::Refused {
                    match_id,
                    squad,
                    reason: RefusedReason::of(&error).as_str().to_string(),
                    prompt_hash: error.prompt_hash().to_string(),
                },
            }
        }
        Err(error) => {
            tracing::error!(%error, "tacsim: undecodable deliberation payload");
            DeliberationReply::Refused {
                match_id: 0,
                squad: String::new(),
                reason: RefusedReason::Unavailable.as_str().to_string(),
                prompt_hash: String::new(),
            }
        }
    };
    // A `DeliberationReply` is four owned strings and an `Order` of integers; there
    // is no value of it that cannot be encoded. The fallback is a hand-written
    // refusal rather than an `expect`, because this crate's panic budget is pinned
    // and a serializer that has learned to fail should not take the world task
    // with it.
    serde_json::to_string(&reply).unwrap_or_else(|_| {
        "{\"outcome\":\"refused\",\"match_id\":0,\"squad\":\"\",\"reason\":\"unavailable\",\
         \"prompt_hash\":\"\"}"
            .to_string()
    })
}

/// **The refusal a world task with no `LlmService` re-enters** (16b cut N.2).
///
/// A dropped completion would leave the bridge's in-flight count one too high for
/// the life of the match, so a host that cannot make the call answers the same way
/// a disabled service does: `unavailable`, and the squad stays on doctrine. The
/// match id is recovered from the job so the reply reaches the right bridge row.
#[must_use]
pub fn unavailable_reply(payload: &str) -> String {
    let (match_id, squad) = serde_json::from_str::<DeliberationJob>(payload).map_or_else(
        |_| (0, String::new()),
        |DeliberationJob::Deliberate { brief }| (brief.match_id, brief.squad.to_string()),
    );
    serde_json::to_string(&DeliberationReply::Refused {
        match_id,
        squad,
        reason: RefusedReason::Unavailable.as_str().to_string(),
        prompt_hash: String::new(),
    })
    .unwrap_or_else(|_| {
        "{\"outcome\":\"refused\",\"match_id\":0,\"squad\":\"\",\"reason\":\"unavailable\",         \"prompt_hash\":\"\"}"
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::Team;

    #[test]
    fn a_job_and_a_reply_round_trip_their_wire_forms() {
        let brief = SquadBrief {
            squad: SquadId {
                team: Team::Red,
                index: 1,
            },
            match_id: 7,
            commander: 4,
            // 17b cut L.1: the squad's lance-leader, never in the prompt.
            leader: CharacterRef::Persona {
                squad: SquadId {
                    team: Team::Red,
                    index: 1,
                },
            },
            reason: "no_order".to_string(),
            units: 2,
            health_permille: 1_000,
            states: vec![("patrol".to_string(), 2)],
            cell: Some((1, 1)),
            window: (16, 16),
            // 16b cut E.2 (16c `T-56`).
            default_ttl_ms: 30_000,
            contacts: Vec::new(),
            // 16b cut Z.1: the squad's own units, as they cross the seam.
            members: vec![
                BriefMember {
                    unit: 4,
                    ordinal: 1,
                    state: "patrol".to_string(),
                    health_permille: 1_000,
                    band: "here".to_string(),
                    bearing_deg: 0,
                },
                BriefMember {
                    unit: 5,
                    ordinal: 2,
                    state: "patrol".to_string(),
                    health_permille: 1_000,
                    band: "close".to_string(),
                    bearing_deg: 270,
                },
            ],
            objective: None,
        };
        let job = DeliberationJob::Deliberate {
            brief: brief.clone(),
        };
        let text = serde_json::to_string(&job).expect("a job encodes");
        assert_eq!(
            serde_json::from_str::<DeliberationJob>(&text).expect("a job decodes"),
            job
        );

        let reply = DeliberationReply::Ok {
            match_id: 7,
            order: Order {
                squad: brief.squad,
                objective: Objective::Hold,
                target: None,
                horizon_ms: 30_000,
                constraints: Constraints::default(),
                ttl_ms: 30_000,
                issued_ms: 5,
                issuer: None,
                by: CharacterRef::Persona { squad: brief.squad },
                spot: None,
            },
            model: "m".to_string(),
            prompt_hash: "h".to_string(),
        };
        let text = serde_json::to_string(&reply).expect("a reply encodes");
        assert_eq!(
            serde_json::from_str::<DeliberationReply>(&text).expect("a reply decodes"),
            reply
        );
        assert_eq!(reply.match_id(), 7);
    }

    #[tokio::test]
    async fn an_undecodable_payload_comes_back_as_an_unavailable_refusal() {
        let llm = mush_llm::LlmService::builder()
            .global_enabled(false)
            .build();
        let text = run_deliberation_async(&llm, "not json").await;
        let reply: DeliberationReply = serde_json::from_str(&text).expect("a reply");
        assert!(matches!(
            reply,
            DeliberationReply::Refused { ref reason, .. } if reason == "unavailable"
        ));
    }

    #[tokio::test]
    async fn a_service_with_no_route_falls_back_to_doctrine() {
        // The shipped default: `[llm].enabled = false`, no route, no provider. Doc
        // 16 §6's fallback is the reflex doctrine, and the shape of that here is
        // one refusal and no order.
        let llm = mush_llm::LlmService::builder()
            .global_enabled(false)
            .build();
        let brief = SquadBrief {
            squad: SquadId {
                team: Team::Blue,
                index: 1,
            },
            match_id: 3,
            commander: 9,
            // 17b cut L.1.
            leader: CharacterRef::Persona {
                squad: SquadId {
                    team: Team::Blue,
                    index: 1,
                },
            },
            reason: "no_order".to_string(),
            units: 2,
            health_permille: 1_000,
            states: Vec::new(),
            cell: None,
            window: (16, 16),
            // 16b cut E.2 (16c `T-56`).
            default_ttl_ms: 30_000,
            contacts: Vec::new(),
            members: Vec::new(),
            objective: None,
        };
        let job = DeliberationJob::Deliberate { brief };
        let payload = serde_json::to_string(&job).expect("a job");
        let text = run_deliberation_async(&llm, &payload).await;
        let reply: DeliberationReply = serde_json::from_str(&text).expect("a reply");
        match reply {
            DeliberationReply::Refused {
                match_id,
                squad,
                reason,
                ..
            } => {
                assert_eq!(match_id, 3);
                assert_eq!(squad, "blue-1");
                assert_eq!(reason, "unavailable");
            }
            DeliberationReply::Ok { .. } => panic!("a disabled service issues no order"),
        }
    }
}
