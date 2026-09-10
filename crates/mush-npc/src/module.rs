//! The `npc` [`GameModule`]: world-event observers that build stimuli, the
//! coalescing/cooldown tick that fires one off-loop turn per ready NPC, the
//! [`AsyncCompletion`] that applies the returned typed actions (re-validated),
//! and the `@agent` staff surface.
//!
//! Everything the LLM produces re-enters here as a typed [`NpcAction`] and is
//! re-validated against current world state (treaty ruling 3). A `Move` through
//! a now-locked exit fails exactly as a player's would; nothing bypasses the
//! command layer, and there is no path to a raw command line.

use std::sync::Arc;

use mush_core::commands::movement::{find_exit, traverse_exit};
use mush_core::module::{AsyncCompletion, WorldEvent};
use mush_core::{
    ArgShape, Attr, CmdError, CommandSpec, Ctx, Deferred, Effect, GameModule, ModError,
    ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, ObjId, ObjKind, Powers, Registrar, Role,
    TickCadence, World, WorldEventKind, WorldObserver, obj_ref, perm,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use smol_str::SmolStr;

use mush_faction::{FactionId, FactionState, Principal};

use crate::action::NpcAction;
use crate::intel::{
    IntelKind, IntelRecord, IntelStore, MAX_INTEL_READABLE_POOLS, finalize_projection, intel_key,
    parse_intel_key_id, push_newest_projection,
};
use crate::knowledge::{
    Familiarity, KNOWLEDGE_ATTR, LoreEntry, LorePool, MAX_LORE_POOLS, MAX_LORE_TAG_BYTES,
    MAX_RESOLVED_LORE_BYTES, MAX_RESOLVED_LORE_ENTRIES, NpcKnowledge, Secrecy, familiarity_of,
    pool_key, validate_lore_tag,
};
use crate::memory::{MAX_MEMORY_SUBJECT_BYTES, MAX_MEMORY_SUMMARY_CHARS, MemoryRecord, record_key};
use crate::profile::{AgentProfile, NS, PROFILE_ATTR, SUMMARY_ATTR, THINKING_FEED};
use crate::runtime::NpcHostState;
use crate::snapshot::{
    AgentGrounding, AppliedAction, MAX_PRESENT_DESCRIPTION_BYTES, MAX_SCENE_DESCRIPTION_BYTES,
    MAX_SCENE_EXITS, MAX_SCENE_PRESENT, MAX_SCENE_TIME_BYTES, MAX_SCENE_WEATHER_BYTES,
    MAX_SNAPSHOT_NAME_BYTES, NoGrounding, PresentPlayer, RecalledMemory, SceneView, StimulusLine,
    TurnKind, TurnOutcome, TurnSnapshot, validate_agent_capability_profile,
};
use crate::stimulus::{SpeechKind, Stimulus};

/// The NPC agent module. Construct with an [`AgentGrounding`] (the driver wires
/// the loaded ruleset; [`NpcModule::new`] uses [`NoGrounding`] for
/// dialogue-only worlds and tests).
pub struct NpcModule {
    grounding: Arc<dyn AgentGrounding>,
    /// H2a: how long one provider lease on an NPC decision lasts, seconds.
    decision_lease_secs: i64,
    /// H2b: how long a scene turn waits for its decision, seconds.
    scene_turn_deadline_secs: i64,
}

impl NpcModule {
    /// A module with no ruleset grounding (dialogue only).
    #[must_use]
    pub fn new() -> NpcModule {
        NpcModule {
            grounding: Arc::new(NoGrounding),
            decision_lease_secs: crate::decision::NPC_DECISION_LEASE_SECS,
            scene_turn_deadline_secs: crate::runtime::DEFAULT_SCENE_TURN_DEADLINE_SECS,
        }
    }

    /// H2a: override the provider lease duration on NPC decisions (the
    /// restart tests use one second; production keeps
    /// [`NPC_DECISION_LEASE_SECS`](crate::decision::NPC_DECISION_LEASE_SECS)).
    #[must_use]
    pub fn with_decision_lease_secs(mut self, secs: i64) -> NpcModule {
        self.decision_lease_secs = secs.max(1);
        self
    }

    /// H2b: `[npc].scene_turn_deadline_secs` — how long an NPC's scene turn
    /// waits for its durable decision before `npc_idle` fires.
    #[must_use]
    pub fn with_scene_turn_deadline_secs(mut self, secs: i64) -> NpcModule {
        self.scene_turn_deadline_secs = secs.max(1);
        self
    }

    /// A module grounded by `grounding` (the driver's ruleset adapter).
    #[must_use]
    pub fn with_grounding(grounding: Arc<dyn AgentGrounding>) -> NpcModule {
        NpcModule {
            grounding,
            decision_lease_secs: crate::decision::NPC_DECISION_LEASE_SECS,
            scene_turn_deadline_secs: crate::runtime::DEFAULT_SCENE_TURN_DEADLINE_SECS,
        }
    }
}

impl Default for NpcModule {
    fn default() -> NpcModule {
        NpcModule::new()
    }
}

impl GameModule for NpcModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.attr_namespace("agent")?;
        let panel = reg.panel_feed(THINKING_FEED)?;
        // S6: the `help` interface the docent fronts.
        reg.interface_provider(
            crate::interface::HELP_INTERFACE,
            Arc::new(crate::interface::HelpInterface),
        )?;
        let mut state = NpcHostState::new(panel);
        state.config.scene_turn_deadline_secs = self.scene_turn_deadline_secs;
        let tick_period = state.config.tick;
        reg.state(state);
        // WS03-A1: `crate::content`'s entry appliers run under the CONTENT
        // module's identity — the content module's pack pass drives every
        // kind's applier itself — so the lore pools and people a pack places
        // carry `content` on the write and `npc` on the namespace. We own the
        // namespace and mush-content cannot name us, so the pair is declared
        // here.
        reg.persist_delegation(
            "content",
            crate::profile::NS,
            "the content module's pack pass drives this crate's entry appliers under              the content module's identity, and they write the lore pools and people              the pack describes",
        )?;
        // ...and the two other modules that drive the same appliers: whoever
        // calls `mush_content::apply_pack` runs EVERY kind's applier under
        // their own Ctx, so a pack with people in it reaches this crate under
        // the atlas's identity (`@atlas/pack apply`) or worldgen's (`+build/go`
        // and a request's activation). Neither crate can name us, so both pairs
        // are declared here beside the content one.
        reg.persist_delegation(
            "atlas",
            crate::profile::NS,
            "`@atlas/pack apply` commits through mush_content::apply_pack, which \
             drives this crate's appliers under the atlas module's identity",
        )?;
        reg.persist_delegation(
            "worldgen",
            crate::profile::NS,
            "the same route for a built pack: `+build/go` and a request's activation \
             apply the pack under the worldgen module's identity, and a site pack \
             carries the personas the planner wrote",
        )?;

        // Speech + movement + presence observers build stimuli.
        for kind in [
            NarrativeKind::Say,
            NarrativeKind::Pose,
            NarrativeKind::SemiPose,
            NarrativeKind::Emit,
        ] {
            reg.subscribe(WorldEventKind::Said { kind }, Arc::new(NpcObserver));
        }
        reg.subscribe(WorldEventKind::Moved, Arc::new(NpcObserver));
        reg.subscribe(WorldEventKind::Connected, Arc::new(NpcObserver));
        reg.subscribe(WorldEventKind::Disconnected, Arc::new(NpcObserver));
        // M14 informant tie-in: a witnessed power use (already projected
        // per-observer by the ruleset director) files intel for its witnesses.
        reg.subscribe(WorldEventKind::PowerUsed, Arc::new(NpcObserver));
        // H2b: a turn-based scene handing its turn to an NPC is a durable
        // decision request.
        reg.subscribe(WorldEventKind::SceneTurn, Arc::new(NpcObserver));

        // The coalescing/cooldown driver.
        reg.tick(
            "agent",
            TickCadence::Coarse(tick_period),
            Arc::new(NpcTick {
                grounding: Arc::clone(&self.grounding),
            }),
        )?;

        // The typed-action re-entry point (holds the grounding so a UseAbility
        // executes on the world loop).
        reg.async_completion(Arc::new(NpcCompletion {
            grounding: Arc::clone(&self.grounding),
        }))?;

        // Staff surface. The handler is grounding-aware so `@agent/sheet` can
        // give an NPC a ruleset character sheet through the loaded ruleset.
        let grounding = Arc::clone(&self.grounding);
        reg.command(
            CommandSpec {
                name: "@agent".into(),
                shape: ArgShape::EqPair,
                switches: vec![
                    "create".into(),
                    "set".into(),
                    "audit".into(),
                    "sheet".into(),
                    "lore".into(),
                    "show".into(),
                    "poke".into(),
                    "enable".into(),
                    "disable".into(),
                ],
                min_role: Role::Builder,
                help: "@agent/create <name> -- create an NPC agent here (a first-class \
                       ObjKind::Npc) with a default Minor profile; give it a persona with \
                       @agent/set and (for rolls) a sheet with @agent/sheet.\n\
                       @agent/set <npc>/<field>=<value> -- edit the profile. Fields: \
                       persona, background, speech, goals, fallback, knows, does_not_know, \
                       tier (major|minor|extra), cooldown (seconds), daily_tokens, enabled, \
                       principal (who this NPC reports intel to: a character name, or \
                       faction:<name> for a faction's shared pool — empty \
                       clears), loyal (yes|no; the willingness gate), watch (\";\"-separated \
                       names/topics of interest). \
                       List fields (goals/fallback/knows/does_not_know/watch) take \
                       \";\"-separated values; an empty value clears a field.\n\
                       @agent/sheet <npc>[=<template>] -- give it a ruleset sheet (default \
                       mortal) so it can roll; fill it in with @sheet/set afterwards.\n\
                       @agent/lore <npc> -- list this NPC's world knowledge; \
                       @agent/lore <npc>=<public|guarded|secret> [subject:] <fact> adds an \
                       entry; =tags: <t1>; <t2> sets shared-pool tags; =del <n> / =clear \
                       remove entries. See also @lore for the shared pools.\n\
                       @agent/show <npc> (or bare @agent <npc>) -- profile + runtime status.\n\
                       @agent/enable <npc> / @agent/disable <npc> -- wake or park the agent.\n\
                       @agent/poke <npc>=<note> -- prod the agent with an out-of-band note.\n\
                       Informant loop: set principal/loyal/watch (above) to have an NPC file \
                       intel its principal pulls with +intel; @agent/show reports its filed \
                       count. See help intel.\n\
                       See also @llm/kill <npc> (per-NPC LLM disable) and help npc."
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| cmd_agent(grounding.as_ref(), ctx)),
        )?;

        // Shared lore pools: authored once, drawn on by many NPCs via tags.
        reg.command(
            CommandSpec {
                name: "@lore".into(),
                shape: ArgShape::EqPair,
                min_role: Role::Builder,
                help: "@lore -- list every shared lore pool with its entry count.\n\
                       @lore <tag> -- list one pool's entries.\n\
                       @lore <tag>=<public|guarded|secret> [subject:] <fact> -- add an entry \
                       to the pool (creating it if new). A SECRET entry is a staff-only \
                       record NEVER sent to any NPC LLM.\n\
                       @lore <tag>=del <n> -- delete pool entry #n (1-based).\n\
                       @lore <tag>=clear -- delete the whole pool.\n\
                       NPCs draw on a pool with @agent/lore <npc>=tags: <tag>. See help npc."
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(cmd_lore),
        )?;

        // The M12 informant PULL query: a player debriefs their own assets.
        // Player-facing (min_role defaults to Player); shows ONLY the querying
        // actor's own principal records — no cross-principal leak.
        reg.command(
            CommandSpec {
                name: "+intel".into(),
                shape: ArgShape::Raw,
                switches: vec!["clear".into()],
                help: "+intel  --  Debrief your informants: list the intel your NPC assets have \
                       filed FOR YOU, newest first (subject, what happened, where, and how long \
                       ago), plus your factions' shared pools when your rank clears you to read \
                       them (those lines are tagged [to <faction>]). Nothing is pushed to you — \
                       you pull it here.\n\
                       +intel <subject>  --  Show only what has been filed about one subject \
                       (by name, or a dbref like #42).\n\
                       +intel/clear  --  Clear your own personal log (faction pools are shared \
                       and are not cleared).\n\
                       An asset reports only if staff set its principal (you, or a faction), it \
                       is willing (loyal) and active, and it actually PERCEIVED the subject. It \
                       files a watched subject's arrivals/departures and speech, and always \
                       flags anyone it catches trying to stay hidden. See help intel."
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(cmd_intel),
        )?;

        // M15: the read-only `intel.list` hydration query behind the SSH TUI's
        // +intel widget (and any future web panel). Same principal-scoped data
        // as the `+intel` command; reading intel does not mutate, so — unlike
        // `@mail` read — this stays a query. The "intel" panel feed names the
        // keyframe it answers with.
        reg.panel_feed("intel")?;
        reg.query_handler("intel.list", Arc::new(crate::query::IntelListQuery))?;

        // H2a: the `npc` decision source — families, definitions, providers,
        // the capture sink and the idle fallback (`crate::decision`).
        crate::decision::register(reg, self.decision_lease_secs)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        boot: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut store = crate::memory::LongTermStore::default();
        let mut pools = crate::knowledge::LorePools::default();
        let mut intel = IntelStore::default();
        let mut pool_docs = std::collections::BTreeMap::new();
        let mut dropped_pool_docs = 0_usize;
        for (key, value) in boot.kv {
            if key.starts_with("mem:") {
                match serde_json::from_value::<MemoryRecord>(value) {
                    Ok(rec)
                        if rec.subject.len() <= MAX_MEMORY_SUBJECT_BYTES
                            && record_key(rec.npc, &rec.subject) == key =>
                    {
                        match store.insert(rec) {
                            Ok(Some(evicted)) => tracing::warn!(
                                npc = evicted.npc.0,
                                subject = %evicted.subject,
                                "npc: boot memory subject cap evicted an older record"
                            ),
                            Ok(None) => {}
                            Err(error) => tracing::error!(
                                %key,
                                ?error,
                                "npc: invalid bounded memory record skipped"
                            ),
                        }
                    }
                    Ok(_) => tracing::error!(
                        %key,
                        "npc: memory document key does not match its record; skipped"
                    ),
                    Err(e) => tracing::error!(%key, error = %e, "npc: unparseable memory skipped"),
                }
            } else if key.starts_with("lore:") {
                match serde_json::from_value::<LorePool>(value) {
                    Ok(pool) => {
                        if pool.tag.len() > MAX_LORE_TAG_BYTES || pool_key(&pool.tag) != key {
                            tracing::error!(%key, "npc: lore pool key does not match its tag; skipped");
                        } else if let Err(error) = pool.validate() {
                            tracing::error!(%key, %error, "npc: invalid bounded lore pool skipped");
                        } else {
                            pool_docs.insert(key, pool);
                            if pool_docs.len() > MAX_LORE_POOLS && pool_docs.pop_last().is_some() {
                                dropped_pool_docs = dropped_pool_docs.saturating_add(1);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(%key, error = %e, "npc: unparseable lore pool skipped")
                    }
                }
            } else if let Some(id) = key.strip_prefix("intel:") {
                match (
                    parse_intel_key_id(id),
                    serde_json::from_value::<Vec<IntelRecord>>(value),
                ) {
                    (Some(principal), Ok(records)) => match intel.load(principal, records) {
                        Ok(dropped) if dropped != 0 => tracing::warn!(
                            %key,
                            dropped,
                            "npc: bounded oversized intel bucket during boot"
                        ),
                        Ok(_) => {}
                        Err(error) => {
                            tracing::error!(%key, %error, "npc: invalid intel bucket skipped")
                        }
                    },
                    (None, _) => {
                        tracing::error!(%key, "npc: bad intel principal key skipped")
                    }
                    (_, Err(e)) => {
                        tracing::error!(%key, error = %e, "npc: unparseable intel skipped")
                    }
                }
            } else {
                tracing::warn!(%key, "npc: unknown persisted key ignored");
            }
        }
        if dropped_pool_docs != 0 {
            tracing::error!(
                dropped = dropped_pool_docs,
                max = MAX_LORE_POOLS,
                "npc: lore pool hard count exceeded; lexically later pools skipped"
            );
        }
        for (key, pool) in pool_docs {
            if let Err(error) = pools.insert(pool) {
                tracing::error!(%key, %error, "npc: invalid bounded lore pool skipped");
            }
        }
        if let Some(h) = ctx.state_mut::<NpcHostState>() {
            h.long_term = store;
            h.lore_pools = pools;
            h.intel = intel;
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Observers: WorldEvent -> Stimulus
// ----------------------------------------------------------------------

struct NpcObserver;

impl WorldObserver for NpcObserver {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        match ev {
            WorldEvent::Said {
                who,
                room,
                kind,
                text,
            } => on_said(ctx, *who, *room, *kind, text),
            WorldEvent::Moved { who, from, to } => on_moved(ctx, *who, *from, *to),
            WorldEvent::Connected { who } => on_presence(ctx, *who, true),
            WorldEvent::Disconnected { who } => on_presence(ctx, *who, false),
            WorldEvent::PowerUsed {
                who,
                room,
                witnesses,
                detail,
            } => on_power_used(ctx, *who, *room, witnesses, detail),
            WorldEvent::SceneTurn {
                scene,
                actor,
                round,
            } => on_scene_turn(ctx, *scene, *actor, *round),
            _ => {}
        }
    }
}

fn on_said(ctx: &mut Ctx<'_>, who: ObjId, room: ObjId, kind: NarrativeKind, text: &str) {
    // No connected player in the room ⇒ no wakes, no tokens.
    if players_present(ctx.w, room).is_empty() {
        return;
    }
    let now = now_ms(ctx.w);
    let speaker = display_name(ctx.w, who);
    let line = format_speech(&speaker, kind, text);
    let sk = speech_kind(kind);
    // Loop-breaker (cost control): an NPC's own speech (re-emitted as a `Said`
    // event by `say_as_npc`) is recorded into other NPCs' memory buffers for
    // context, but it must **never wake** another NPC — otherwise two enabled
    // NPCs sharing a room ping-pong LLM turns forever while a single idle player
    // is merely present. Every NPC turn stays player-driven: only player-sourced
    // speech enqueues a waking stimulus.
    let speaker_is_npc = ctx.w.get(who).is_some_and(|o| o.kind == ObjKind::Npc);
    let targets = npcs_in(ctx.w, room);
    for npc in targets {
        if npc == who {
            continue;
        }
        // M11 per-observer projection: an NPC that cannot perceive the speaker
        // (a vampire Obfuscated past its Auspex) neither remembers nor wakes to
        // the speech — true silence, exactly as for a player who fails to
        // notice. Free pass with no concealment ruleset registered.
        if !ctx.can_perceive(npc, who) {
            continue;
        }
        let Some(profile) = read_profile(ctx.w, npc) else {
            continue;
        };
        if !profile.enabled {
            continue;
        }
        if speaker_is_npc {
            // Context only — no wake.
            record_self(ctx, npc, &line);
            continue;
        }
        let directed = mentions(ctx.w, npc, text);
        let stim = if directed {
            Stimulus::SpokenTo {
                by: who,
                text: text.to_string(),
                kind: sk,
            }
        } else {
            Stimulus::HeardInRoom {
                by: who,
                text: text.to_string(),
            }
        };
        // Informant loop (M12): the NPC perceived `who` (the can_perceive gate
        // above passed), so it may file — gated further by the willingness /
        // watch / notability rules inside `file_intel`.
        let short = crate::action::sanitize_line(text, INTEL_DETAIL_CAP);
        file_intel(ctx, npc, who, IntelKind::Spoke, &format!("said: {short}"));
        remember_and_enqueue(ctx, npc, stim, &line, now);
    }
}

fn on_moved(ctx: &mut Ctx<'_>, who: ObjId, from: Option<ObjId>, to: ObjId) {
    if !is_connected_player(ctx.w, who) {
        return;
    }
    let now = now_ms(ctx.w);
    let name = display_name(ctx.w, who);
    // Arrival in `to`.
    for npc in npcs_in(ctx.w, to) {
        if npc == who || !enabled_npc(ctx.w, npc) {
            continue;
        }
        // M11: an unseen (Obfuscated) arrival triggers no stimulus.
        if !ctx.can_perceive(npc, who) {
            continue;
        }
        file_intel(ctx, npc, who, IntelKind::Arrived, "arrived");
        remember_and_enqueue(
            ctx,
            npc,
            Stimulus::PlayerArrived { who },
            &format!("{name} arrives."),
            now,
        );
    }
    // Departure from `from`.
    if let Some(from) = from {
        for npc in npcs_in(ctx.w, from) {
            if npc == who || !enabled_npc(ctx.w, npc) {
                continue;
            }
            if !ctx.can_perceive(npc, who) {
                continue;
            }
            file_intel(ctx, npc, who, IntelKind::Departed, "left");
            remember_and_enqueue(
                ctx,
                npc,
                Stimulus::PlayerLeft { who },
                &format!("{name} leaves."),
                now,
            );
        }
    }
}

fn on_presence(ctx: &mut Ctx<'_>, who: ObjId, connected: bool) {
    let Some(room) = ctx.w.get(who).and_then(|o| o.location) else {
        return;
    };
    if ctx.w.get(who).is_none_or(|o| o.kind != ObjKind::Player) {
        return;
    }
    let now = now_ms(ctx.w);
    let name = display_name(ctx.w, who);
    let (stim_line, verb, kind, detail): (fn(ObjId) -> Stimulus, &str, IntelKind, &str) =
        if connected {
            (
                |who| Stimulus::PlayerArrived { who },
                "arrives",
                IntelKind::Arrived,
                "arrived",
            )
        } else {
            (
                |who| Stimulus::PlayerLeft { who },
                "leaves",
                IntelKind::Departed,
                "left",
            )
        };
    for npc in npcs_in(ctx.w, room) {
        if !enabled_npc(ctx.w, npc) {
            continue;
        }
        // M11: a connect/disconnect by an unseen character wakes no NPC.
        if !ctx.can_perceive(npc, who) {
            continue;
        }
        file_intel(ctx, npc, who, kind, detail);
        remember_and_enqueue(ctx, npc, stim_line(who), &format!("{name} {verb}."), now);
    }
}

/// M14 informant tie-in: file intel for every NPC that PERCEIVED a power use.
///
/// The ruleset's perception director already resolved `witnesses` (the
/// onlookers who noticed the act) on the seeded RNG, so — unlike the
/// movement/speech observers — there is deliberately **NO `can_perceive`
/// re-gate** here: re-rolling would double-count perception and could diverge
/// from what the director actually delivered to each witness. We file only for
/// the NPCs in the carried set. [`file_intel`] treats [`IntelKind::PowerUse`] as
/// inherently notable (filed regardless of the watch list, like a pierced
/// concealment), so a witnessed power always reaches the informant's principal.
/// `room` is carried for completeness; the room *name* an intel record stores is
/// read from the informant's own location inside [`file_intel`].
/// H2b: the bounds of one scene-turn context, sized so a full room stays far
/// under the kernel's `MAX_DECISION_CONTEXT_BYTES` (64 KiB): 32 names of 64
/// bytes twice over, an 8 KiB description and a 4 KiB persona is under 16 KiB
/// of JSON at the worst.
pub const MAX_TURN_NAME_BYTES: usize = 64;
/// How many present players a turn context names.
pub const MAX_TURN_PRESENT: usize = 32;
/// How many exits a turn context names.
pub const MAX_TURN_EXITS: usize = 32;
/// The room description's byte bound in a turn context.
pub const MAX_TURN_DESCRIPTION_BYTES: usize = 8 * 1024;
/// The persona's byte bound in a turn context.
pub const MAX_TURN_PERSONA_BYTES: usize = 4 * 1024;

/// H2b: the public-safe context an NPC's scene turn carries — the room, who
/// is present, the exits and the NPC's own persona, every field bounded. No
/// memories in v1, no truth ids: names only.
pub fn scene_turn_context(
    w: &World,
    profile: &AgentProfile,
    scene: u64,
    round: u32,
    actor: ObjId,
    room: ObjId,
) -> serde_json::Value {
    let present: Vec<String> = players_present(w, room)
        .into_iter()
        .filter(|&who| who != actor)
        .take(MAX_TURN_PRESENT)
        .map(|who| bounded_text(display_name(w, who), MAX_TURN_NAME_BYTES))
        .collect();
    let exits: Vec<String> = w
        .exits_of(room)
        .iter()
        .take(MAX_TURN_EXITS)
        .map(|&exit| bounded_text(display_name(w, exit), MAX_TURN_NAME_BYTES))
        .collect();
    serde_json::json!({
        "scene": scene,
        "round": round,
        "room": bounded_text(display_name(w, room), MAX_TURN_NAME_BYTES),
        "description": bounded_text(
            attr_text(w, room, "short_desc")
                .or_else(|| attr_text(w, room, "desc"))
                .unwrap_or_default(),
            MAX_TURN_DESCRIPTION_BYTES,
        ),
        "present": present,
        "exits": exits,
        "persona": bounded_text(profile.persona.clone(), MAX_TURN_PERSONA_BYTES),
    })
}

/// H2b: the scene handed its turn to `actor`. Queue one durable decision
/// for it — the actor is the NPC, never `ctx.actor` (the facilitator); the
/// cause instance keys on `(scene, round, actor)`, so a re-advance to the
/// same NPC in the same round is an identical retry, and the replaceable key
/// on `(scene, actor)` lets a new round displace the older pending record.
/// The context is a bounded scene view under `MAX_DECISION_CONTEXT_BYTES`.
fn on_scene_turn(ctx: &mut Ctx<'_>, scene: u64, actor: ObjId, round: u32) {
    let Some(profile) = read_profile(ctx.w, actor) else {
        return;
    };
    if !profile.enabled || ctx.w.get(actor).is_none_or(|o| o.kind != ObjKind::Npc) {
        return;
    }
    let Some(room) = ctx.w.get(actor).and_then(|o| o.location) else {
        return;
    };
    let context = scene_turn_context(ctx.w, &profile, scene, round, actor, room);
    let (Ok(cause_kind), Ok(instance), Ok(replaceable)) = (
        mush_core::CanonicalId::new("npc.scene_turn"),
        mush_core::IdempotencyKeyText::parse_canonical(format!(
            "scene-{scene}-round-{round}-npc-{}",
            actor.0
        )),
        mush_core::IdempotencyKeyText::parse_canonical(format!("scene-{scene}-npc-{}", actor.0)),
    ) else {
        return;
    };
    let deadline_secs = ctx
        .state::<NpcHostState>()
        .map_or(crate::runtime::DEFAULT_SCENE_TURN_DEADLINE_SECS, |h| {
            h.config.scene_turn_deadline_secs
        });
    let request = crate::decision::NpcDecisionRequest {
        npc: actor,
        cause_kind,
        instance: instance.clone(),
        replaceable_key: Some(replaceable),
        context,
        deadline_secs,
        key: instance,
    };
    let queued = ctx
        .w
        .module_state_mut::<NpcHostState>()
        .is_some_and(|h| h.request_decision(request));
    if !queued {
        tracing::warn!(npc = %actor, scene, round, "npc: the scene turn's decision request was dropped (queue full)");
    }
}

fn on_power_used(ctx: &mut Ctx<'_>, who: ObjId, _room: ObjId, witnesses: &[ObjId], detail: &str) {
    for &npc in witnesses {
        // Defensive: the director returns the NPC subset already, but never file
        // for a non-NPC even if one were ever carried.
        if ctx.w.get(npc).is_none_or(|o| o.kind != ObjKind::Npc) {
            continue;
        }
        file_intel(ctx, npc, who, IntelKind::PowerUse, detail);
    }
}

/// Push a transcript line into the NPC's buffer (flagging overflow) and enqueue
/// the stimulus.
fn remember_and_enqueue(ctx: &mut Ctx<'_>, npc: ObjId, stim: Stimulus, line: &str, now: i64) {
    let ts = ctx.w.now();
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        let rt = h.runtime_mut(npc);
        rt.memory.push(ts, line);
        if rt.memory.is_overflowing() {
            rt.summarize_pending = true;
        }
        h.enqueue(npc, stim, now);
    }
}

// ----------------------------------------------------------------------
// The informant loop (M12): file perceived intel to a principal
// ----------------------------------------------------------------------

/// Cap on the escaped speech snippet stored in an intel `detail` — short enough
/// to keep the log terse, sanitized like every other stored NPC-adjacent text.
const INTEL_DETAIL_CAP: usize = 80;

/// File one perceived event as intel for `npc`'s principal, **iff** the loop is
/// armed and the event is reportable (design M12 D4). This is only ever called
/// from *inside* an observer's `can_perceive`-passed branch, so the informant
/// has genuinely perceived `subject` (invariant P1): a subject the NPC failed
/// to perceive never reaches here.
///
/// The willingness gate (P2): a principal must be set, the NPC must be `loyal`
/// and `enabled`, and the subject must be neither the informant nor the
/// principal. Reportability: the subject is on the NPC's `watch` list, OR it is
/// under active concealment ([`is_concealed`] — a notable pierce, always filed
/// regardless of the watch list). With an empty watch list and a non-concealed
/// subject, nothing is filed (the log stays signal-rich).
fn file_intel(ctx: &mut Ctx<'_>, npc: ObjId, subject: ObjId, kind: IntelKind, detail: &str) {
    let Some(profile) = read_profile(ctx.w, npc) else {
        return;
    };
    let Some(principal) = profile.principal else {
        return; // no principal ⇒ files nothing
    };
    if !profile.loyal || !profile.enabled {
        return; // resentful or parked ⇒ files nothing
    }
    if subject == npc || principal == Principal::Character(subject) {
        return; // never report the informant or the (character) principal itself
    }
    // A faction principal must still exist: after @faction/destroy the pool
    // is readable by no one (can_read_intel fails for a missing faction), so
    // filing into it would only churn an orphaned doc forever.
    if let Principal::Faction { faction } = principal {
        let live = ctx
            .w
            .module_state::<FactionState>()
            .is_some_and(|st| st.factions.contains_key(&faction));
        if !live {
            tracing::debug!(
                npc = npc.0,
                faction = faction.0,
                "npc: intel principal faction gone; not filing"
            );
            return;
        }
    }
    // Notability: a pierced concealment and a witnessed power use are always
    // reported; otherwise the subject must be on the watch list.
    let concealed = ctx.is_concealed(subject);
    let inherently_notable = matches!(kind, IntelKind::PowerUse);
    if !concealed && !inherently_notable && !subject_on_watch(ctx.w, &profile, subject) {
        return;
    }
    // A pierced-concealment sighting is filed as `Concealed` with the base
    // detail annotated — the notable fact is that the informant saw someone
    // trying not to be seen. An inherently-notable kind (a witnessed power use)
    // keeps its own kind even when the subject is also concealed: the fact worth
    // reporting is the power, and its detail already names the act.
    let (kind, detail) = if concealed && !inherently_notable {
        (
            IntelKind::Concealed,
            format!("{detail} (though they were trying to stay hidden)"),
        )
    } else {
        (kind, detail.to_string())
    };
    let room_name = ctx
        .w
        .get(npc)
        .and_then(|o| o.location)
        .map(|r| display_name(ctx.w, r))
        .unwrap_or_default();
    let subject_name = display_name(ctx.w, subject);
    let informant_name = display_name(ctx.w, npc);
    let rec = IntelRecord {
        informant: npc,
        principal,
        subject,
        subject_name,
        kind,
        detail,
        room_name,
        provenance: format!("witnessed directly by {informant_name}"),
        ts: ctx.w.now(),
    };
    let records = ctx.state_mut::<NpcHostState>().map(|h| h.intel.file(rec));
    match records {
        Some(Ok(records)) => persist_intel(ctx, principal, &records),
        Some(Err(error)) => {
            tracing::warn!(npc = npc.0, %error, "npc: invalid intel record refused");
        }
        None => {}
    }
}

/// Whether `who` clears the faction registry's own staff bar (mirrors
/// `mush-faction`'s admin check): the `FACTION_ADMIN` power or Royalty+.
/// `@agent` is Builder-gated, which sits BELOW that bar — so every faction
/// name this module renders or resolves must re-apply the hidden-faction
/// projection, or `@agent` becomes an existence oracle the faction module
/// itself carefully refuses to be.
fn is_faction_staff(w: &World, who: ObjId) -> bool {
    perm::has_power(w, who, Powers::FACTION_ADMIN) || perm::has_role(w, who, Role::Royalty)
}

/// Label for a [`Principal`] as shown to `viewer`: the character's name and
/// dbref, or the faction's name — but a faction the viewer could not see in
/// `+factions` (hidden, and they are neither a member nor faction staff) is
/// rendered as "an undisclosed faction", and a dangling id shows its number
/// to faction staff only.
fn principal_label(w: &World, viewer: ObjId, p: Principal) -> String {
    match p {
        Principal::Character(id) => format!("{} (#{})", display_name(w, id), id.0),
        Principal::Faction { faction } => {
            let staff = is_faction_staff(w, viewer);
            match w
                .module_state::<FactionState>()
                .and_then(|st| st.factions.get(&faction).map(|f| (st, f)))
            {
                Some((st, f)) if st.faction_listed(viewer, staff, f) => {
                    format!("faction {}", f.name)
                }
                Some(_) => "an undisclosed faction".to_string(),
                None if staff => format!("faction #{faction} (no longer exists)"),
                None => "an undisclosed faction".to_string(),
            }
        }
    }
}

/// Whether `subject` (by primary name, dbref, or alias) is on `profile`'s
/// watch list. The watch entries are stored lowercased; comparisons are
/// case-insensitive. An empty watch list matches nothing.
fn subject_on_watch(w: &World, profile: &AgentProfile, subject: ObjId) -> bool {
    if profile.watch.is_empty() {
        return false;
    }
    let Some(o) = w.get(subject) else {
        return false;
    };
    let name = primary(&o.name).to_lowercase();
    let dbref = format!("#{}", subject.0);
    profile.watch.iter().any(|watched| {
        watched == &name
            || watched == &dbref
            || o.aliases.iter().any(|a| a.eq_ignore_ascii_case(watched))
    })
}

// ----------------------------------------------------------------------
// The pull query (M12 D5): +intel — a player debriefs their own assets
// ----------------------------------------------------------------------

/// `+intel` / `+intel <subject>` / `+intel/clear` — the player-facing PULL over
/// the intel their NPC informants have filed FOR THEM (design M12 D5). The
/// querying actor is the principal: it can only ever read its own bucket
/// ([`IntelStore::for_principal`]), so there is no cross-principal leak (P5).
/// Records are shown newest-first with the subject, kind, room, detail,
/// provenance, and a relative timestamp. Nothing is ever pushed — the player
/// pulls here.
fn cmd_intel(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;

    if ctx.switches.has("clear") {
        // Clears the PERSONAL bucket only. Faction pools are shared — one
        // member must not be able to wipe the whole faction's memory.
        let dropped = ctx
            .state_mut::<NpcHostState>()
            .map_or(0, |h| h.intel.clear_for(Principal::Character(actor)));
        clear_intel(ctx, actor);
        ctx.out.sys_to_actor(
            actor,
            "intel",
            format!(
                "Cleared {dropped} personal intel {} (faction pools are shared and stay).",
                records_word(dropped)
            ),
        );
        return Ok(());
    }

    let arg = ctx.args.raw().trim().to_string();
    let by_id = if arg.is_empty() {
        None
    } else {
        mush_core::match_object(ctx.w, actor, &arg).ok()
    };
    let needle = arg.to_lowercase();
    let matches_filter = |record: &IntelRecord| {
        arg.is_empty()
            || by_id == Some(record.subject)
            || record.subject_name.to_lowercase().contains(&needle)
    };

    // The faction pools this actor is cleared to read (M21: a member whose
    // level passes the faction's intel_level). Snapshot before touching the
    // NPC state so the borrows never overlap.
    let readable_pools: Vec<(FactionId, SmolStr)> = readable_intel_pools(ctx.w, actor);

    // Snapshot only the newest bounded matching records across the actor's own
    // bucket and every readable faction pool. The working set is count-bounded
    // while scanning, then byte-bounded before any output is constructed.
    let records: Vec<(IntelRecord, Option<SmolStr>)> = ctx
        .state::<NpcHostState>()
        .map(|h| {
            let mut v = Vec::new();
            for record in h.intel.for_principal(Principal::Character(actor)) {
                if matches_filter(record) {
                    push_newest_projection(&mut v, record.clone(), None);
                }
            }
            for (fid, fname) in &readable_pools {
                for record in h.intel.for_principal(Principal::faction(*fid)) {
                    if matches_filter(record) {
                        push_newest_projection(&mut v, record.clone(), Some(fname.clone()));
                    }
                }
            }
            finalize_projection(v, |pool| pool.as_ref().map_or(0, SmolStr::len))
        })
        .unwrap_or_default();
    let filter = (!arg.is_empty()).then_some(arg);

    if records.is_empty() {
        let msg = match &filter {
            Some(s) => format!("No intel on file about {s}."),
            None => "You have no intel on file. Your informants report here when they \
                     have something to tell you."
                .to_string(),
        };
        ctx.out.sys_to_actor(actor, "intel", msg);
        return Ok(());
    }

    let n = records.len();
    let header = match &filter {
        Some(s) => format!("Intel about {s} — {n} {}, newest first:", records_word(n)),
        None => format!("Your intel log — {n} {}, newest first:", records_word(n)),
    };
    ctx.out.sys_to_actor(actor, "intel", header);

    let now = ctx.w.now();
    for (r, pool) in &records {
        let room = if r.room_name.trim().is_empty() {
            String::new()
        } else {
            format!(" in {}", r.room_name)
        };
        let via = pool
            .as_ref()
            .map(|f| format!("  [to {f}]"))
            .unwrap_or_default();
        ctx.out.sys_to_actor(
            actor,
            "intel",
            format!(
                "  {:>4} ago  {} [{}]{}: {}  ({}){via}",
                fmt_ago(now, r.ts),
                r.subject_name,
                intel_kind_label(r.kind),
                room,
                r.detail,
                r.provenance,
            ),
        );
    }
    Ok(())
}

/// The faction intel pools `actor` may read: every faction where they hold a
/// membership whose level passes the faction's `intel_level`. Empty when the
/// faction module is not loaded.
fn readable_intel_pools(w: &World, actor: ObjId) -> Vec<(FactionId, SmolStr)> {
    w.module_state::<FactionState>()
        .map(|fs| {
            fs.memberships_of(actor)
                .filter(|m| fs.can_read_intel(actor, m.faction))
                .filter_map(|m| fs.factions.get(&m.faction).map(|f| (f.id, f.name.clone())))
                .take(MAX_INTEL_READABLE_POOLS)
                .collect()
        })
        .unwrap_or_default()
}

/// The human label for an [`IntelKind`] in the `+intel` render (shared with
/// the `intel.list` query handler so the command and the widget agree).
pub(crate) fn intel_kind_label(kind: IntelKind) -> &'static str {
    match kind {
        IntelKind::Arrived => "arrived",
        IntelKind::Departed => "departed",
        IntelKind::Present => "present",
        IntelKind::Spoke => "spoke",
        IntelKind::Concealed => "concealed",
        IntelKind::PowerUse => "poweruse",
    }
}

/// "record" / "records" for a count.
fn records_word(n: usize) -> &'static str {
    if n == 1 { "record" } else { "records" }
}

/// A compact "how long ago" for the `+intel` render (`42s`, `5m`, `3h`, `2d`),
/// mirroring the jobs surface's `fmt_ago` (kept local to avoid a cross-crate
/// dependency for one tiny helper).
fn fmt_ago(now: i64, ts: i64) -> String {
    let d = (now - ts).max(0);
    if d < 60 {
        format!("{d}s")
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86_400)
    }
}

/// Delete a principal's persisted intel doc (the whole bucket was already
/// cleared from the in-RAM [`IntelStore`] by the caller). Mirrors
/// [`delete_pool`]'s persistence shape.
fn clear_intel(ctx: &mut Ctx<'_>, principal: ObjId) {
    let principal = Principal::Character(principal);
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::Del {
            key: intel_key(principal),
        },
    });
}

// ----------------------------------------------------------------------
// Tick: fire ready NPCs
// ----------------------------------------------------------------------

struct NpcTick {
    grounding: Arc<dyn AgentGrounding>,
}

impl mush_core::module::TickHandler for NpcTick {
    fn tick(&self, ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::module::SceneId>) {
        // H2a: one durable-decision capture per tick step, oldest request
        // first. Two captures in one step would void both.
        let request = ctx
            .w
            .module_state_mut::<NpcHostState>()
            .and_then(|h| h.decision_requests.pop_front());
        if let Some(request) = request {
            issue_capture(ctx, request);
        }
        let now = now_ms(ctx.w);
        let pending = match ctx.state::<NpcHostState>() {
            Some(h) => h.pending(),
            None => return,
        };
        for npc in pending {
            self.maybe_fire(ctx, npc, now);
        }
    }
}

/// H2a: turn one queued request into the kernel's capture selector. The NPC
/// must still be an enabled agent; the fallback is always `npc_idle`; the
/// actor is the NPC, never the tick's `ctx.actor`.
fn issue_capture(ctx: &mut Ctx<'_>, request: crate::decision::NpcDecisionRequest) {
    let Some(handles) = ctx.state::<crate::decision::NpcDecisionHandles>().cloned() else {
        tracing::warn!("npc: a decision was requested but the source is not registered");
        return;
    };
    let is_agent = ctx
        .w
        .get(request.npc)
        .is_some_and(|o| o.kind == ObjKind::Npc)
        && read_profile(ctx.w, request.npc).is_some_and(|p| p.enabled);
    if !is_agent {
        tracing::info!(npc = %request.npc, "npc: dropped a decision request for a non-agent");
        return;
    }
    let (Ok(schema), Ok(controller_key)) = (
        mush_core::CanonicalId::new(crate::decision::NPC_CONTEXT_SCHEMA),
        mush_core::CanonicalId::new(crate::decision::NPC_CONTROLLER_KEY),
    ) else {
        return;
    };
    let context = match mush_core::ProjectedDecisionContext::new(schema, request.context) {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(npc = %request.npc, error = %error, "npc: decision context refused");
            return;
        }
    };
    let draft = mush_core::DecisionCaptureDraft {
        actor: request.npc,
        subject: mush_core::PerceivedRef::SelfActor,
        controller_key,
        cause: mush_core::DecisionCause {
            kind: request.cause_kind,
            instance: request.instance,
            replaceable_key: request.replaceable_key,
        },
        context,
        deadline_secs: request.deadline_secs,
        fallback: mush_core::FallbackPolicyRef(handles.idle.clone()),
        key: request.key,
    };
    if let Err(error) = ctx.capture_decision(draft) {
        tracing::warn!(npc = %request.npc, error = %error, "npc: decision capture not queued");
    }
}

impl NpcTick {
    fn maybe_fire(&self, ctx: &mut Ctx<'_>, npc: ObjId, now: i64) {
        let Some(profile) = read_profile(ctx.w, npc) else {
            drop_mailbox(ctx, npc);
            return;
        };
        if !profile.enabled {
            drop_mailbox(ctx, npc);
            return;
        }
        let Some(room) = ctx.w.get(npc).and_then(|o| o.location) else {
            return;
        };
        let present = players_present(ctx.w, room);

        // Decide the turn kind + readiness from the runtime.
        let (has_mailbox, summarize, ready) = {
            let Some(h) = ctx.state::<NpcHostState>() else {
                return;
            };
            let Some(rt) = h.runtime(npc) else {
                return;
            };
            let coalesce = h.config.coalesce;
            let has_mailbox = !rt.mailbox.is_empty();
            let window = rt.window_elapsed(now, coalesce);
            let cooldown = rt.cooldown_ok(now, profile.cooldown(), rt.has_direct());
            let ready = has_mailbox && window && cooldown;
            (has_mailbox, rt.summarize_pending, ready)
        };

        // A participant in an exclusive turn-based scene (a timestop) acts only
        // when handed its scene turn, never on free-running stimuli — otherwise
        // recorded scene speech would keep triggering autonomous dialogue turns
        // inside the frozen scene. Drop the pending dialogue batch; a cheap
        // background summarize may still fold memory.
        if ctx.w.in_exclusive_scene(npc) {
            drop_mailbox(ctx, npc);
            if summarize {
                self.fire_summarize(ctx, npc, &profile, room, &present, now);
            }
            return;
        }

        // No players present: never fire a paid *dialogue* turn into an empty
        // room. Drop the pending dialogue batch unconditionally (its stimuli are
        // stale — the players who caused them have left); only a cheap
        // background summarize may proceed, independent of occupancy. A pending
        // summarize must not smuggle a dialogue turn past this guard.
        if present.is_empty() {
            drop_mailbox(ctx, npc);
            if summarize {
                self.fire_summarize(ctx, npc, &profile, room, &present, now);
            }
            return;
        }

        if has_mailbox && ready {
            self.fire_dialogue(ctx, npc, &profile, room, &present, now);
        } else if summarize && !has_mailbox {
            self.fire_summarize(ctx, npc, &profile, room, &present, now);
        }
    }

    fn fire_dialogue(
        &self,
        ctx: &mut Ctx<'_>,
        npc: ObjId,
        profile: &AgentProfile,
        room: ObjId,
        present: &[ObjId],
        now: i64,
    ) {
        let correlation = ctx.w.next_correlation_id();
        let batch = match ctx.state_mut::<NpcHostState>() {
            Some(h) => h.begin_turn(npc, now, correlation),
            None => return,
        };
        let snap = build_dialogue_snapshot(
            ctx,
            self.grounding.as_ref(),
            npc,
            profile,
            room,
            present,
            &batch,
            correlation,
        );
        if defer_turn(ctx, npc, correlation, &snap) {
            emit_thinking(ctx, npc, room);
        }
    }

    fn fire_summarize(
        &self,
        ctx: &mut Ctx<'_>,
        npc: ObjId,
        profile: &AgentProfile,
        room: ObjId,
        present: &[ObjId],
        now: i64,
    ) {
        let _ = now;
        let correlation = ctx.w.next_correlation_id();
        let fold = match ctx.state_mut::<NpcHostState>() {
            Some(h) => h.begin_summarize(npc, correlation),
            None => return,
        };
        if fold.is_empty() {
            // Nothing to fold after all; clear in-flight.
            if let Some(h) = ctx.state_mut::<NpcHostState>() {
                h.finish_turn(npc, now);
            }
            return;
        }
        let snap = build_summarize_snapshot(ctx, npc, profile, room, present, fold, correlation);
        let _ = defer_turn(ctx, npc, correlation, &snap);
    }
}

// ----------------------------------------------------------------------
// Async completion: apply typed actions, re-validated
// ----------------------------------------------------------------------

struct NpcCompletion {
    grounding: Arc<dyn AgentGrounding>,
}

impl AsyncCompletion for NpcCompletion {
    fn on_complete(&self, ctx: &mut Ctx<'_>, correlation: u64, payload: &str) {
        // Match and clear in-flight on the framework-supplied NPC id
        // (`ctx.actor` is the effect's actor) BEFORE decoding, so a malformed
        // payload for the active turn cannot wedge it forever. The correlation
        // check is load-bearing: a delayed duplicate from an older turn must
        // neither clear a newer turn nor replay its stale actions, while an
        // idle NPC applies a completion it has not seen before (a finished
        // turn re-entered with nothing in flight). We do not trust
        // `outcome.npc()` for either identity.
        let npc = ctx.actor;
        let now = now_ms(ctx.w);
        let active = ctx
            .state_mut::<NpcHostState>()
            .is_some_and(|h| h.finish_turn_if_correlation(npc, correlation, now));
        if !active {
            tracing::debug!(
                npc = npc.0,
                correlation,
                "npc: stale async completion dropped"
            );
            return;
        }
        // The thinking window closed with this turn, whatever the payload
        // decodes to: end the room's `npc.thinking` pill now rather than
        // leaving a client to its own timer (interface plan I1, D-I-13).
        if let Some(room) = ctx.w.get(npc).and_then(|o| o.location) {
            emit_thinking_done(ctx, npc, room);
        }
        let outcome: TurnOutcome = match serde_json::from_str(payload) {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(correlation, error = %e, "npc: bad completion payload");
                return;
            }
        };
        // Re-validate against current state.
        if ctx.w.get(npc).is_none_or(|o| o.kind != ObjKind::Npc) {
            return;
        }
        let Some(room) = ctx.w.get(npc).and_then(|o| o.location) else {
            return;
        };

        match outcome {
            TurnOutcome::Nothing { .. } => {
                // Silent + already audited by the LlmService call.
            }
            TurnOutcome::Degraded { line, reason, .. } => {
                // Fallback lines are staff-authored, but hold them to the same
                // output-hygiene invariant as every other NPC-said line
                // (single line, no control/ANSI/bidi, length-capped).
                // S6 rule 4: an interface NPC's fallback names the command
                // that does the same thing.
                let commands = read_profile_raw(ctx.w, npc)
                    .map(|p| crate::interface::commands_for(ctx.reg, &p))
                    .unwrap_or_default();
                let line = crate::interface::with_command_hint(&line, &commands);
                let line = crate::action::sanitize_line(&line, crate::action::SAY_CAP);
                if !line.is_empty() {
                    say_as_npc(ctx, npc, room, NarrativeKind::Say, &line);
                }
                notify_staff_once(ctx, npc, &reason);
            }
            TurnOutcome::Summary { summary, .. } => {
                let summary = crate::action::sanitize_line(&summary, MAX_MEMORY_SUMMARY_CHARS);
                let _ = ctx.w.set_attr(npc, SUMMARY_ATTR, Attr::text(&summary, npc));
                if let Some(h) = ctx.state_mut::<NpcHostState>() {
                    h.runtime_mut(npc).memory.summary = summary;
                }
            }
            TurnOutcome::Ok { actions, asker, .. } => {
                for aa in actions {
                    apply_action(
                        self.grounding.as_ref(),
                        ctx,
                        npc,
                        room,
                        &aa,
                        correlation,
                        asker,
                    );
                }
            }
        }
    }
}

/// Apply one typed action, re-validated in the world task.
/// H2c: the deterministic idempotency discriminator of one NPC ability
/// submission — the NPC, the turn's correlation and the ability name — so a
/// duplicated completion is a HIT of the first, never a second roll.
#[must_use]
pub fn ability_submit_key(
    npc: ObjId,
    correlation: u64,
    name: &str,
) -> mush_core::IdempotencyKeyText {
    let mut ability: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(32)
        .collect();
    if ability.is_empty() {
        ability.push_str("ability");
    }
    mush_core::IdempotencyKeyText::parse_canonical(format!("npc-{}-{correlation}-{ability}", npc.0))
        .expect("a bounded lowercase key")
}

fn apply_action(
    grounding: &dyn AgentGrounding,
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    room: ObjId,
    aa: &AppliedAction,
    correlation: u64,
    asker: Option<ObjId>,
) {
    match &aa.action {
        NpcAction::Say { text } => say_as_npc(ctx, npc, room, NarrativeKind::Say, text),
        NpcAction::Pose { text } => say_as_npc(ctx, npc, room, NarrativeKind::Pose, text),
        NpcAction::Move { exit } => {
            // Re-validated exactly like a player's move: a locked / vanished
            // exit simply fails.
            if let Some(e) = find_exit(ctx.w, npc, exit) {
                let lens = ctx.typed_state.lens();
                traverse_exit(ctx.reg, ctx.w, npc, e, ctx.out, lens);
            }
            record_self(ctx, npc, &format!("(tries to leave via {exit})"));
        }
        NpcAction::LookAt { .. } => {
            // Resolved off-loop into a tool result; no world mutation on-loop.
        }
        NpcAction::Remember { subject, note } => {
            let now = ctx.w.now();
            let update = match ctx.state_mut::<NpcHostState>() {
                Some(h) => Some(h.long_term.remember(npc, subject, note, now)),
                None => None,
            };
            match update {
                Some(Ok(update)) => persist_memory(ctx, &update),
                Some(Err(error)) => {
                    tracing::warn!(npc = npc.0, ?error, "npc: invalid memory action refused");
                }
                None => {}
            }
        }
        NpcAction::UseAbility { name, input } => {
            // S6 (doc 14 §4.6): an interface ability runs as the asker, on
            // the world loop, through the provider the profile names; the
            // answer is what the NPC says, the refusal too (it is the asker's
            // own command's word). Audited as (npc, asker, ability, input).
            let fronted = read_profile_raw(ctx.w, npc)
                .and_then(|p| crate::interface::provider_for(ctx.reg, &p, name));
            if let Some((_provider_id, provider, _def)) = fronted {
                invoke_interface(ctx, npc, room, asker, provider.as_ref(), name, input);
                return;
            }
            // H2c: an ability with a projected legal action is an **immediate
            // deterministic controller** (K7.0): the model's choice already
            // happened off-loop, the mapping is deterministic, so it enters
            // the action coordinator as this NPC's own submission — the
            // ruleset's typed state, resolution and receipt, never a bespoke
            // mutation. Everything else is the residual, below.
            if let Some(projected) = grounding.projected_ability(ctx.w, npc, name, input) {
                let key = ability_submit_key(npc, correlation, name);
                ctx.submit_authoritative(
                    projected.action,
                    mush_core::PerceivedRef::SelfActor,
                    Vec::new(),
                    projected.parameters,
                    key,
                );
                record_self(
                    ctx,
                    npc,
                    &format!("(used {name} as an authoritative action)"),
                );
                return;
            }
            // Executed on the world loop by the ruleset: it reads the NPC's own
            // sheet, rolls on the shared RNG + roll log, and emits the result
            // to the room itself. The model only requested it.
            match grounding.invoke_ability(ctx.w, ctx.out, npc, name, input.clone()) {
                Ok(outcome) => {
                    // M14 D6 / V5: the ruleset's perception director already
                    // projected any power use per-observer and returned the NPC
                    // witness set in `outcome.data.power_use` (the registry-less
                    // ability seam cannot emit a `WorldEvent` itself). We hold a
                    // `Ctx`, so we announce it here — exactly as the player path
                    // does after `roll_expr` — driving the informant loop for the
                    // NPC-actor branch too (no NPC power use bypasses projection).
                    announce_power_use(ctx, npc, room, &outcome);
                    record_self(ctx, npc, &format!("(used {name}: {})", outcome.summary))
                }
                Err(e) => {
                    tracing::debug!(npc = npc.0, %name, error = %e, "npc ability refused");
                }
            }
        }
        NpcAction::DoNothing { .. } => {}
    }
}

/// S6: run one interface ability as `asker`, speak the answer, audit the call.
fn invoke_interface(
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    room: ObjId,
    asker: Option<ObjId>,
    provider: &dyn mush_core::InterfaceAbilityProvider,
    name: &str,
    input: &serde_json::Value,
) {
    let Some(asker) = asker else {
        // No one spoke: an ambient turn holds no one's authority.
        record_self(
            ctx,
            npc,
            &format!("(would have used {name}, but no one asked)"),
        );
        return;
    };
    let live_asker = ctx
        .w
        .get(asker)
        .is_some_and(|o| o.location == Some(room) && !o.flags.contains(mush_core::Flags::GOING));
    let result = if live_asker {
        // Rule 1: the ability runs as the asker — the command context's actor
        // is theirs for the call, so every read the provider makes through a
        // command path is filtered as their own command would be and every
        // write is theirs.
        let saved = ctx.actor;
        ctx.actor = asker;
        let r = provider.invoke(ctx, mush_core::AbilityCall { npc, asker }, name, input);
        ctx.actor = saved;
        r
    } else {
        Err(mush_core::InterfaceError::Refused(
            "the one who asked is not here any more".to_string(),
        ))
    };
    let at = ctx.w.now();
    let ok = result.is_ok();
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        h.audit_interface(crate::runtime::InterfaceAuditRow {
            at,
            npc,
            asker,
            ability: SmolStr::new(name),
            input: crate::interface::abbreviate(input),
            ok,
        });
    }
    match result {
        Ok(reply) => {
            let lines: Vec<String> = reply
                .text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| crate::action::sanitize_line(l, crate::action::SAY_CAP))
                .filter(|l| !l.is_empty())
                .collect();
            match reply.audience {
                mush_core::ReplyAudience::Room => {
                    for line in &lines {
                        say_as_npc(ctx, npc, room, NarrativeKind::Say, line);
                    }
                    record_self(ctx, npc, &format!("(answered with {name})"));
                }
                mush_core::ReplyAudience::Asker => {
                    // S8.2 (14a #2): the provider read as the asker, so the
                    // answer is theirs alone — spoken to them, not the room:
                    // no room narrative, no `Said` event for another NPC to
                    // overhear, and the memory keeps the ability, not the
                    // text, so a later public turn cannot repeat it. The room
                    // sees that a word was had.
                    for line in &lines {
                        speak_to(ctx, npc, asker, NarrativeKind::Say, line);
                    }
                    let asker_name = display_name(ctx.w, asker);
                    if !lines.is_empty() {
                        let aside = format!(
                            "{} speaks quietly with {asker_name}.",
                            display_name(ctx.w, npc)
                        );
                        ctx.out.to_room(
                            room,
                            vec![asker],
                            OutputEvent::Narrative {
                                kind: NarrativeKind::Emit,
                                actor: Some(obj_ref(ctx.w, npc)),
                                text: RichText::plain(aside),
                            },
                        );
                    }
                    record_self(
                        ctx,
                        npc,
                        &format!("(answered {asker_name} with {name}, privately)"),
                    );
                }
            }
        }
        Err(mush_core::InterfaceError::Refused(why)) => {
            let line = crate::action::sanitize_line(&why, crate::action::SAY_CAP);
            if !line.is_empty() {
                say_as_npc(ctx, npc, room, NarrativeKind::Say, &line);
            }
            record_self(ctx, npc, &format!("(could not: {name}: {why})"));
        }
        Err(error) => {
            tracing::debug!(npc = npc.0, %name, %error, "npc: interface ability refused");
            record_self(ctx, npc, &format!("(could not use {name})"));
        }
    }
}

/// If `outcome` declares a perceived power use (a ruleset ability that routed
/// through the M14 perception director sets `data.power_use = { witnesses,
/// detail }`), emit [`WorldEvent::PowerUsed`] carrying the director-resolved NPC
/// witness set so [`on_power_used`] files intel for exactly those NPCs. A plain
/// ability (a mundane roll, or a ruleset without the director) sets no such key
/// and emits nothing. This is the NPC-actor mirror of the player path's emit in
/// the ruleset's `roll_expr`.
fn announce_power_use(
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    room: ObjId,
    outcome: &mush_ruleset::AbilityOutcome,
) {
    let Some(pu) = outcome.data.get("power_use") else {
        return;
    };
    let detail = pu
        .get("detail")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("used a power")
        .to_string();
    let witnesses: Vec<ObjId> = pu
        .get("witnesses")
        .and_then(|w| serde_json::from_value(w.clone()).ok())
        .unwrap_or_default();
    ctx.emit_world_event(&WorldEvent::PowerUsed {
        who: npc,
        room,
        witnesses,
        detail,
    });
}

// ----------------------------------------------------------------------
// @agent staff command
// ----------------------------------------------------------------------

fn cmd_agent(grounding: &dyn AgentGrounding, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // `create` runs before target resolution (the target does not exist yet).
    // S6: the audit is a read over every NPC; it needs no target.
    if ctx.switches.has("audit") {
        return agent_audit(ctx);
    }
    if ctx.switches.has("create") {
        return agent_create(ctx);
    }

    // Every other switch names an existing NPC. For `@agent/set <npc>/<field>`
    // the target is the path segment before the first `/`.
    let lhs = ctx.args.lhs().to_string();
    let target = lhs.split('/').next().unwrap_or("").trim();
    if target.is_empty() {
        return Err(CmdError::msg("Usage: @agent <npc>  (see help @agent)."));
    }
    let npc = mush_core::match_object(ctx.w, ctx.actor, target)?;
    if ctx.w.get(npc).is_none_or(|o| o.kind != ObjKind::Npc) {
        return Err(CmdError::msg("That is not an NPC."));
    }

    if ctx.switches.has("set") {
        return agent_set(ctx, npc, &lhs);
    }

    if ctx.switches.has("sheet") {
        return agent_sheet(grounding, ctx, npc);
    }

    if ctx.switches.has("lore") {
        return agent_lore(ctx, npc);
    }

    if ctx.switches.has("poke") {
        let note = ctx
            .args
            .rhs()
            .map(str::to_string)
            .filter(|n| !n.trim().is_empty())
            .ok_or_else(|| CmdError::msg("Usage: @agent/poke <npc>=<note>."))?;
        let now = now_ms(ctx.w);
        let by = ctx.actor;
        if let Some(h) = ctx.state_mut::<NpcHostState>() {
            h.enqueue(npc, Stimulus::StaffPoke { by, note }, now);
        }
        ctx.out.info(ctx.actor, "Poked the agent.");
        return Ok(());
    }

    if ctx.switches.has("enable") || ctx.switches.has("disable") {
        let enable = ctx.switches.has("enable");
        let mut profile = read_profile_raw(ctx.w, npc).unwrap_or_default();
        profile.enabled = enable;
        store_profile(ctx, npc, &profile)?;
        ctx.out.info(
            ctx.actor,
            format!("Agent {}.", if enable { "enabled" } else { "disabled" }),
        );
        return Ok(());
    }

    // Default: show profile + status.
    let profile = read_profile_raw(ctx.w, npc).unwrap_or_default();
    let name = display_name(ctx.w, npc);
    ctx.out.info(
        ctx.actor,
        format!(
            "Agent {name} (#{}) — tier {:?}, {}.",
            npc.0,
            profile.tier,
            if profile.enabled {
                "enabled"
            } else {
                "disabled"
            }
        ),
    );
    if !profile.persona.trim().is_empty() {
        ctx.out
            .info(ctx.actor, format!("  Persona: {}", profile.persona));
    }
    if !profile.goals.is_empty() {
        ctx.out
            .info(ctx.actor, format!("  Goals: {}", profile.goals.join("; ")));
    }
    // Informant loop (M12): who this NPC reports to, whether it is willing, and
    // what it watches for.
    if let Some(principal) = profile.principal {
        let plabel = principal_label(ctx.w, ctx.actor, principal);
        let watch = if profile.watch.is_empty() {
            "only notable events (pierced concealment)".to_string()
        } else {
            profile.watch.join(", ")
        };
        ctx.out.info(
            ctx.actor,
            format!(
                "  Informant: reports to {plabel}; {}; watching {watch}.",
                if profile.loyal {
                    "loyal"
                } else {
                    "NOT loyal (files nothing)"
                }
            ),
        );
        let filed = ctx
            .state::<NpcHostState>()
            .map_or(0, |h| h.intel.filed_by(npc));
        ctx.out.info(
            ctx.actor,
            format!("  Intel filed: {filed} {}.", records_word(filed)),
        );
    }
    let status = ctx
        .state::<NpcHostState>()
        .and_then(|h| h.runtime(npc))
        .map(|rt| {
            format!(
                "  Runtime: {} pending, {}, {} buffered.",
                rt.mailbox.len(),
                if rt.in_flight { "in flight" } else { "idle" },
                rt.memory.events.len()
            )
        })
        .unwrap_or_else(|| "  Runtime: no activity yet.".to_string());
    ctx.out.info(ctx.actor, status);
    // Persisted memory (survives reboots): the rolling summary attribute + the
    // count of durable, subject-keyed memories reloaded into the long-term store.
    let summary = read_summary(ctx.w, npc);
    if !summary.trim().is_empty() {
        ctx.out
            .info(ctx.actor, format!("  Summary: {}", summary.trim()));
    }
    let mem_subjects = ctx
        .state::<NpcHostState>()
        .map_or(0, |h| h.long_term.subject_count(npc));
    if mem_subjects > 0 {
        let plural = if mem_subjects == 1 {
            "subject"
        } else {
            "subjects"
        };
        ctx.out.info(
            ctx.actor,
            format!("  Durable memories: {mem_subjects} {plural}."),
        );
    }
    // World / semantic knowledge (design: M10d): per-NPC lore + shared-pool tags.
    let knowledge = read_knowledge(ctx.w, npc);
    if !knowledge.lore.is_empty() || !knowledge.tags.is_empty() {
        let (mut public, mut guarded, mut secret) = (0usize, 0usize, 0usize);
        for e in &knowledge.lore {
            match e.secrecy {
                Secrecy::Public => public += 1,
                Secrecy::Guarded => guarded += 1,
                Secrecy::Secret => secret += 1,
            }
        }
        let total = knowledge.lore.len();
        let tags = if knowledge.tags.is_empty() {
            "none".to_string()
        } else {
            knowledge.tags.join(", ")
        };
        ctx.out.info(
            ctx.actor,
            format!(
                "  World knowledge: {total} {} ({public} public, {guarded} guarded, {secret} \
                 secret); tags: {tags}.",
                entries_word(total)
            ),
        );
        if !knowledge.tags.is_empty() {
            let pools: Vec<String> = ctx
                .state::<NpcHostState>()
                .map(|h| {
                    knowledge
                        .tags
                        .iter()
                        .filter_map(|t| {
                            h.lore_pools
                                .get(t)
                                .map(|p| format!("{t} ({})", p.entries.len()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !pools.is_empty() {
                ctx.out.info(
                    ctx.actor,
                    format!("  Shared lore: draws on pools {}.", pools.join(", ")),
                );
            }
        }
    }
    Ok(())
}

/// `@agent/create <name>` — create a first-class NPC object in the staffer's
/// room and seed a default (Minor, enabled) profile. The NPC does nothing until
/// staff give it a persona (`@agent/set`) and, for rolls, a sheet
/// (`@agent/sheet`).
fn agent_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let name = ctx.args.lhs().trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg("Usage: @agent/create <name>."));
    }
    if name.contains('/') || name.contains('=') {
        return Err(CmdError::msg(
            "An NPC name can't contain '/' or '='. Usage: @agent/create <name>.",
        ));
    }
    let room = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or_else(|| CmdError::msg("You are nowhere; go somewhere before creating an NPC."))?;
    let npc = ctx
        .w
        .create(ObjKind::Npc, &name, ctx.actor, Some(room))
        .map_err(|e| CmdError::msg(format!("Could not create the NPC: {e}")))?;
    let profile = AgentProfile::default();
    store_profile(ctx, npc, &profile)?;
    ctx.out.info(
        ctx.actor,
        format!(
            "Created NPC agent {name} (#{}). Next: @agent/set {name}/persona=..., \
             @agent/sheet {name} for a rollable sheet.",
            npc.0
        ),
    );
    Ok(())
}

/// `@agent/set <npc>/<field>=<value>` — validated edit of the staff-authored
/// [`AgentProfile`]. List fields split on `;`; an empty value clears a field.
/// `@agent/audit [<npc>]` (S6, rule 4): the last interface-ability calls —
/// who fronted, who asked, what — newest last.
fn agent_audit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let raw = ctx.args.raw().trim().to_string();
    let only = if raw.is_empty() {
        None
    } else {
        Some(
            mush_core::match_object(ctx.w, actor, &raw)
                .map_err(|e| CmdError::msg(e.to_string()))?,
        )
    };
    let rows: Vec<String> = ctx
        .state::<NpcHostState>()
        .map(|h| {
            h.interface_audit
                .iter()
                .filter(|r| only.is_none_or(|o| r.npc == o))
                .rev()
                .take(20)
                .map(|r| {
                    let ago = ctx.w.now().saturating_sub(r.at);
                    format!(
                        "{ago}s ago {} for {}: {} {} {}",
                        display_name(ctx.w, r.npc),
                        display_name(ctx.w, r.asker),
                        r.ability,
                        r.input,
                        if r.ok { "answered" } else { "refused" }
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        ctx.out.info(actor, "No interface calls recorded.");
    } else {
        for line in rows.into_iter().rev() {
            ctx.out.info(actor, line);
        }
    }
    Ok(())
}

fn agent_set(ctx: &mut Ctx<'_>, npc: ObjId, lhs: &str) -> Result<(), CmdError> {
    let field = lhs
        .split_once('/')
        .map(|(_, f)| f.trim())
        .filter(|f| !f.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: @agent/set <npc>/<field>=<value>."))?;
    let value = ctx.args.rhs().unwrap_or("").trim().to_string();
    let mut profile = read_profile_raw(ctx.w, npc).unwrap_or_default();
    let field_l = field.to_ascii_lowercase();
    match field_l.as_str() {
        "persona" => profile.persona = value.clone(),
        "background" => profile.background = value.clone(),
        "speech" | "speech_style" => profile.speech_style = value.clone(),
        "goals" => profile.goals = split_list(&value),
        "fallback" | "fallback_lines" => profile.fallback_lines = split_list(&value),
        "knows" => profile.knowledge.knows = split_list(&value),
        "does_not_know" | "unknown" => profile.knowledge.does_not_know = split_list(&value),
        "tier" => profile.tier = parse_tier(&value)?,
        "cooldown" | "cooldown_s" => profile.cooldown_s = parse_opt_u32(&value)?,
        "daily_tokens" | "budget" => profile.daily_tokens = parse_opt_u64(&value)?,
        "enabled" => profile.enabled = parse_bool(&value)?,
        // S6: the interface providers the NPC fronts; an id the server does
        // not compose is kept (content may run ahead of composition) and
        // simply offers nothing.
        "interfaces" => {
            profile.interfaces = split_list(&value)
                .into_iter()
                .map(|i| SmolStr::new(i.trim().to_ascii_lowercase()))
                .collect();
        }
        "principal" => {
            profile.principal = if value.is_empty() {
                None
            } else if let Some(fname) = strip_prefix_ci(&value, "faction:") {
                // A faction pool: intel lands in the faction's shared store,
                // read via +intel by members passing its intel_level.
                //
                // The lookup honors the hidden-faction projection: to a
                // setter who could not see the faction in +factions, it is
                // indistinguishable from a missing one — otherwise this
                // Builder-gated command is an existence oracle over the
                // registry's secret societies.
                let staff = is_faction_staff(ctx.w, ctx.actor);
                let fid = ctx
                    .w
                    .module_state::<FactionState>()
                    .and_then(|st| {
                        st.faction_by_name(fname.trim())
                            .filter(|f| st.faction_listed(ctx.actor, staff, f))
                            .map(|f| f.id)
                    })
                    .ok_or_else(|| {
                        CmdError::msg(
                            "No such faction. (To bind a character whose name begins with \
                             \"faction:\", use their #dbref.)",
                        )
                    })?;
                Some(Principal::faction(fid))
            } else {
                Some(Principal::Character(mush_core::match_object(
                    ctx.w, ctx.actor, &value,
                )?))
            };
        }
        "loyal" => profile.loyal = parse_bool(&value)?,
        "watch" => {
            profile.watch = split_list(&value)
                .iter()
                .map(|s| s.to_ascii_lowercase())
                .collect();
        }
        _ => {
            return Err(CmdError::msg(format!(
                "Unknown agent field {field:?}. Fields: persona, background, speech, goals, \
                 fallback, knows, does_not_know, tier, cooldown, daily_tokens, enabled, \
                 principal, loyal, watch, interfaces."
            )));
        }
    }
    let set_principal = profile.principal.filter(|_| field_l == "principal");
    store_profile(ctx, npc, &profile)?;
    let name = display_name(ctx.w, npc);
    ctx.out.info(
        ctx.actor,
        if value.is_empty() && is_clearable(&field_l) {
            format!("Cleared {field} on {name}.")
        } else if let Some(principal) = set_principal {
            // Echo the resolved target so staff can confirm the name bound to
            // the intended object (a mis-binding is otherwise silent).
            let plabel = principal_label(ctx.w, ctx.actor, principal);
            format!("Set {field} on {name} to {plabel}.")
        } else {
            format!("Set {field} on {name}.")
        },
    );
    Ok(())
}

/// `@agent/sheet <npc>[=<template>]` — give the NPC a ruleset character sheet
/// (default `mortal`) through the loaded ruleset's normal validated path, so
/// `wod_roll` (etc.) has real traits to read. Delegates to the driver's
/// grounding adapter; a dialogue-only world reports that no sheet ruleset is
/// loaded.
fn agent_sheet(
    grounding: &dyn AgentGrounding,
    ctx: &mut Ctx<'_>,
    npc: ObjId,
) -> Result<(), CmdError> {
    let template = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("mortal")
        .to_string();
    let by = ctx.actor;
    match grounding.init_sheet(ctx.w, npc, &template, by) {
        Ok(msg) => {
            let name = display_name(ctx.w, npc);
            ctx.out.info(
                ctx.actor,
                format!("{msg} Fill it in with @sheet/set {name}/<field>=<value>."),
            );
            Ok(())
        }
        Err(e) => Err(CmdError::msg(e)),
    }
}

// ----------------------------------------------------------------------
// @agent/lore + @lore staff surface (world / semantic knowledge, M10d)
// ----------------------------------------------------------------------

/// The staff-facing warning appended when a `SECRET` per-NPC entry is added:
/// a Secret is a staff-only record that never reaches the NPC LLM (the
/// load-bearing invariant), so it cannot influence the NPC on its own.
const SECRET_NPC_WARNING: &str = "  Note: SECRET entries are stored as a staff-only record and are \
     NEVER sent to the NPC LLM; this will not influence the NPC. Use scripted or staff events to \
     act on it.";

/// As [`SECRET_NPC_WARNING`], for a Secret entry added to a *shared* pool.
const SECRET_POOL_WARNING: &str = "  Note: SECRET pool entries are a staff-only record and are \
     NEVER sent to any NPC's LLM; they cannot influence an NPC. Use scripted or staff events to \
     act on it.";

/// `@agent/lore <npc>[=<op>]` — the per-NPC world-knowledge surface: list
/// entries, add one, set shared-pool tags, delete one, or clear all entries
/// (tags kept). Distinct from the terse `knows`/`does_not_know`
/// [`KnowledgeBounds`](crate::profile::KnowledgeBounds) — this is graded,
/// tiered, subject-keyed, provenance-tagged.
fn agent_lore(ctx: &mut Ctx<'_>, npc: ObjId) -> Result<(), CmdError> {
    let rhs = ctx.args.rhs().map(str::trim).unwrap_or("").to_string();
    if rhs.is_empty() {
        return agent_lore_list(ctx, npc);
    }
    if rhs.eq_ignore_ascii_case("clear") {
        return agent_lore_clear(ctx, npc);
    }
    if let Some(rest) = strip_prefix_ci(&rhs, "tags:") {
        return agent_lore_tags(ctx, npc, rest.trim());
    }
    if let Some(rest) = strip_word_ci(&rhs, "del") {
        return agent_lore_del(ctx, npc, rest.trim());
    }
    let entry = parse_lore_spec(&rhs)?;
    agent_lore_add(ctx, npc, entry)
}

/// List `npc`'s per-NPC world knowledge: numbered entries, its shared-pool
/// tags, and a one-line note of how many pool entries those tags resolve.
fn agent_lore_list(ctx: &mut Ctx<'_>, npc: ObjId) -> Result<(), CmdError> {
    let knowledge = read_knowledge(ctx.w, npc);
    let name = display_name(ctx.w, npc);
    if knowledge.lore.is_empty() && knowledge.tags.is_empty() {
        ctx.out.info(
            ctx.actor,
            format!(
                "{name} has no world knowledge yet. Add some with \
                 @agent/lore {name}=<tier> [subject:] <fact>."
            ),
        );
        return Ok(());
    }
    ctx.out
        .info(ctx.actor, format!("World knowledge for {name}:"));
    if knowledge.lore.is_empty() {
        ctx.out.info(ctx.actor, "  (no direct entries)");
    } else {
        for (i, e) in knowledge.lore.iter().enumerate() {
            ctx.out
                .info(ctx.actor, format!("  {}. {}", i + 1, render_lore_entry(e)));
        }
    }
    if knowledge.tags.is_empty() {
        ctx.out.info(ctx.actor, "  Tags: (none)");
    } else {
        ctx.out
            .info(ctx.actor, format!("  Tags: {}", knowledge.tags.join(", ")));
        let resolved = ctx
            .state::<NpcHostState>()
            .map(|h| h.lore_pools.resolve(&knowledge.tags).len())
            .unwrap_or(0);
        ctx.out.info(
            ctx.actor,
            format!(
                "  Shared lore: {resolved} {} resolved from tagged pools.",
                entries_word(resolved)
            ),
        );
    }
    Ok(())
}

/// Add one parsed [`LoreEntry`] to `npc`'s per-NPC knowledge, warning on a
/// `SECRET` add that it will never reach the LLM.
fn agent_lore_add(ctx: &mut Ctx<'_>, npc: ObjId, entry: LoreEntry) -> Result<(), CmdError> {
    let secret = entry.secrecy == Secrecy::Secret;
    let mut knowledge = read_knowledge(ctx.w, npc);
    knowledge.lore.push(entry);
    let idx = knowledge.lore.len();
    store_knowledge(ctx, npc, &knowledge)?;
    let name = display_name(ctx.w, npc);
    ctx.out.info(
        ctx.actor,
        format!("Added world-knowledge entry #{idx} to {name}."),
    );
    if secret {
        ctx.out.info(ctx.actor, SECRET_NPC_WARNING);
    }
    Ok(())
}

/// Set `npc`'s shared-pool tags (an empty list clears them), confirming which
/// tags resolve to a defined pool and warning about any that do not.
fn agent_lore_tags(ctx: &mut Ctx<'_>, npc: ObjId, list: &str) -> Result<(), CmdError> {
    let tags = split_list(list);
    let mut knowledge = read_knowledge(ctx.w, npc);
    knowledge.tags = tags.clone();
    store_knowledge(ctx, npc, &knowledge)?;
    let name = display_name(ctx.w, npc);
    if tags.is_empty() {
        ctx.out
            .info(ctx.actor, format!("Cleared shared-lore tags on {name}."));
        return Ok(());
    }
    ctx.out.info(
        ctx.actor,
        format!("Set shared-lore tags on {name}: {}.", tags.join(", ")),
    );
    let (mut defined, mut undefined) = (Vec::new(), Vec::new());
    if let Some(h) = ctx.state::<NpcHostState>() {
        for t in &tags {
            match h.lore_pools.get(t) {
                Some(p) => defined.push(format!("{t} ({})", p.entries.len())),
                None => undefined.push(t.clone()),
            }
        }
    } else {
        undefined = tags.clone();
    }
    if !defined.is_empty() {
        ctx.out.info(
            ctx.actor,
            format!("  Resolves pools: {}.", defined.join(", ")),
        );
    }
    if !undefined.is_empty() {
        ctx.out.info(
            ctx.actor,
            format!(
                "  No pool defined yet for: {}. Author with @lore <tag>=<tier> [subject:] <fact>.",
                undefined.join(", ")
            ),
        );
    }
    Ok(())
}

/// Delete `npc`'s per-NPC entry #n (1-based, as `@agent/lore <npc>` lists it).
fn agent_lore_del(ctx: &mut Ctx<'_>, npc: ObjId, arg: &str) -> Result<(), CmdError> {
    let n = parse_index(arg)?;
    let mut knowledge = read_knowledge(ctx.w, npc);
    let count = knowledge.lore.len();
    if n == 0 || n > count {
        return Err(CmdError::msg(format!(
            "No entry #{n}. {} has {count} {}.",
            display_name(ctx.w, npc),
            entries_word(count)
        )));
    }
    let removed = knowledge.lore.remove(n - 1);
    store_knowledge(ctx, npc, &knowledge)?;
    ctx.out.info(
        ctx.actor,
        format!(
            "Deleted entry #{n} ({}) from {}.",
            render_lore_entry(&removed),
            display_name(ctx.w, npc)
        ),
    );
    Ok(())
}

/// Clear all of `npc`'s per-NPC lore entries, leaving its shared-pool tags.
fn agent_lore_clear(ctx: &mut Ctx<'_>, npc: ObjId) -> Result<(), CmdError> {
    let mut knowledge = read_knowledge(ctx.w, npc);
    let count = knowledge.lore.len();
    knowledge.lore.clear();
    store_knowledge(ctx, npc, &knowledge)?;
    ctx.out.info(
        ctx.actor,
        format!(
            "Cleared {count} world-knowledge {} from {} (tags kept).",
            entries_word(count),
            display_name(ctx.w, npc)
        ),
    );
    Ok(())
}

/// `@lore [<tag>[=<op>]]` — the shared lore-pool surface: list all pools, list
/// one, add an entry (creating the pool), delete an entry, or delete the pool.
fn cmd_lore(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let tag = ctx.args.lhs().trim().to_string();
    if tag.is_empty() {
        return lore_list_all(ctx);
    }
    validate_lore_tag(&tag).map_err(|error| CmdError::msg(error.to_string()))?;
    let rhs = ctx.args.rhs().map(str::trim).unwrap_or("").to_string();
    if rhs.is_empty() {
        return lore_show(ctx, &tag);
    }
    if rhs.eq_ignore_ascii_case("clear") {
        return lore_clear(ctx, &tag);
    }
    if let Some(rest) = strip_word_ci(&rhs, "del") {
        return lore_del(ctx, &tag, rest.trim());
    }
    let entry = parse_lore_spec(&rhs)?;
    lore_add(ctx, &tag, entry)
}

/// List every shared lore pool with its entry count.
fn lore_list_all(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let pools: Vec<(String, usize)> = ctx
        .state::<NpcHostState>()
        .map(|h| {
            h.lore_pools
                .tags_sorted()
                .into_iter()
                .map(|t| {
                    let n = h.lore_pools.get(&t).map_or(0, |p| p.entries.len());
                    (t, n)
                })
                .collect()
        })
        .unwrap_or_default();
    if pools.is_empty() {
        ctx.out.info(
            ctx.actor,
            "No shared lore pools defined. Create one with @lore <tag>=<tier> [subject:] <fact>.",
        );
        return Ok(());
    }
    ctx.out.info(ctx.actor, "Shared lore pools:");
    for (t, n) in pools {
        ctx.out
            .info(ctx.actor, format!("  {t} — {n} {}.", entries_word(n)));
    }
    Ok(())
}

/// List one pool's entries (numbered, tiered, with subject/source).
fn lore_show(ctx: &mut Ctx<'_>, tag: &str) -> Result<(), CmdError> {
    let pool = ctx
        .state::<NpcHostState>()
        .and_then(|h| h.lore_pools.get(tag).cloned())
        .ok_or_else(|| {
            CmdError::msg(format!(
                "No lore pool tagged {tag:?}. Create it with @lore {tag}=<tier> [subject:] <fact>."
            ))
        })?;
    ctx.out.info(
        ctx.actor,
        format!(
            "Lore pool {} — {} {}:",
            pool.tag,
            pool.entries.len(),
            entries_word(pool.entries.len())
        ),
    );
    for (i, e) in pool.entries.iter().enumerate() {
        ctx.out
            .info(ctx.actor, format!("  {}. {}", i + 1, render_lore_entry(e)));
    }
    Ok(())
}

/// Add one parsed [`LoreEntry`] to the pool `tag`, creating the pool if new.
fn lore_add(ctx: &mut Ctx<'_>, tag: &str, entry: LoreEntry) -> Result<(), CmdError> {
    let secret = entry.secrecy == Secrecy::Secret;
    let mut pool = ctx
        .state::<NpcHostState>()
        .and_then(|h| h.lore_pools.get(tag).cloned())
        .unwrap_or_else(|| LorePool {
            tag: tag.to_string(),
            entries: Vec::new(),
        });
    pool.entries.push(entry);
    let idx = pool.entries.len();
    let pool_tag = pool.tag.clone();
    persist_pool(ctx, &pool)?;
    ctx.out.info(
        ctx.actor,
        format!("Added entry #{idx} to lore pool {pool_tag}."),
    );
    if secret {
        ctx.out.info(ctx.actor, SECRET_POOL_WARNING);
    }
    Ok(())
}

/// Delete pool `tag`'s entry #n (1-based).
fn lore_del(ctx: &mut Ctx<'_>, tag: &str, arg: &str) -> Result<(), CmdError> {
    let n = parse_index(arg)?;
    let mut pool = ctx
        .state::<NpcHostState>()
        .and_then(|h| h.lore_pools.get(tag).cloned())
        .ok_or_else(|| CmdError::msg(format!("No lore pool tagged {tag:?}.")))?;
    let count = pool.entries.len();
    if n == 0 || n > count {
        return Err(CmdError::msg(format!(
            "No entry #{n} in pool {} ({count} {}).",
            pool.tag,
            entries_word(count)
        )));
    }
    let removed = pool.entries.remove(n - 1);
    let pool_tag = pool.tag.clone();
    persist_pool(ctx, &pool)?;
    ctx.out.info(
        ctx.actor,
        format!(
            "Deleted entry #{n} ({}) from pool {pool_tag}.",
            render_lore_entry(&removed)
        ),
    );
    Ok(())
}

/// Delete the whole pool `tag` (persisted doc + in-RAM view).
fn lore_clear(ctx: &mut Ctx<'_>, tag: &str) -> Result<(), CmdError> {
    if delete_pool(ctx, tag) {
        ctx.out.info(
            ctx.actor,
            format!("Deleted lore pool {tag:?}. NPCs tagging it now resolve nothing for it."),
        );
    } else {
        ctx.out
            .info(ctx.actor, format!("No lore pool tagged {tag:?} to delete."));
    }
    Ok(())
}

/// Parse `@…/lore …=<tier> [subject:] <fact>` into a [`LoreEntry`]. `tier` is
/// `public` | `guarded` | `secret` (case-insensitive); an optional leading
/// `subject:` (text before the first colon) sets the subject and the rest is
/// the fact. Both are trimmed; an empty fact is rejected. `source` is left
/// empty (provenance is not set from this line). Shared by the per-NPC and pool
/// add paths.
fn parse_lore_spec(rhs: &str) -> Result<LoreEntry, CmdError> {
    let rhs = rhs.trim();
    let (tier_tok, rest) = rhs.split_once(char::is_whitespace).ok_or_else(|| {
        CmdError::msg("Usage: ...=<tier> [subject:] <fact>. Tier is public, guarded, or secret.")
    })?;
    let secrecy = parse_secrecy(tier_tok)?;
    let rest = rest.trim();
    let (subject, fact) = match rest.split_once(':') {
        Some((s, f)) => (s.trim().to_string(), f.trim().to_string()),
        None => (String::new(), rest.to_string()),
    };
    if fact.is_empty() {
        return Err(CmdError::msg("The fact can't be empty."));
    }
    let entry = LoreEntry {
        subject,
        fact,
        secrecy,
        source: String::new(),
    };
    entry
        .validate()
        .map_err(|error| CmdError::msg(error.to_string()))?;
    Ok(entry)
}

/// Parse a secrecy tier word (case-insensitive).
fn parse_secrecy(value: &str) -> Result<Secrecy, CmdError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "public" => Ok(Secrecy::Public),
        "guarded" => Ok(Secrecy::Guarded),
        "secret" => Ok(Secrecy::Secret),
        other => Err(CmdError::msg(format!(
            "Unknown tier {other:?}. Choose public, guarded, or secret."
        ))),
    }
}

/// A staff-facing one-line render of one [`LoreEntry`]: `[tier] subject: fact
/// (source: …)`, omitting an empty subject/source.
fn render_lore_entry(e: &LoreEntry) -> String {
    let mut s = format!("[{}]", secrecy_label(e.secrecy));
    let subject = e.subject.trim();
    if !subject.is_empty() {
        s.push_str(&format!(" {subject}:"));
    }
    s.push_str(&format!(" {}", e.fact.trim()));
    let source = e.source.trim();
    if !source.is_empty() {
        s.push_str(&format!(" (source: {source})"));
    }
    s
}

fn secrecy_label(s: Secrecy) -> &'static str {
    match s {
        Secrecy::Public => "public",
        Secrecy::Guarded => "guarded",
        Secrecy::Secret => "secret",
    }
}

/// "entry" / "entries" for a count.
fn entries_word(n: usize) -> &'static str {
    if n == 1 { "entry" } else { "entries" }
}

/// Parse a 1-based entry index from a delete argument.
fn parse_index(s: &str) -> Result<usize, CmdError> {
    s.trim()
        .parse::<usize>()
        .map_err(|_| CmdError::msg(format!("Expected an entry number, got {:?}.", s.trim())))
}

/// If `s` (after leading whitespace) begins with `prefix` case-insensitively,
/// return the remainder after it; else `None`. Safe on non-ASCII input.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let s = s.trim_start();
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// If `s` (after leading whitespace) begins with the whole word `word`
/// (case-insensitive), followed by whitespace or end-of-string, return the
/// remainder after the word; else `None`.
fn strip_word_ci<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let s = s.trim_start();
    let head = s.get(..word.len())?;
    if !head.eq_ignore_ascii_case(word) {
        return None;
    }
    let rest = &s[word.len()..];
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some(rest)
}

/// Split a `;`- (or newline-) separated list value, trimming and dropping
/// empties. An empty input yields an empty list (clears the field).
fn split_list(value: &str) -> Vec<String> {
    value
        .split([';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether a field is a list/text field that an empty value clears (for the
/// confirmation wording only).
fn is_clearable(field: &str) -> bool {
    matches!(
        field,
        "persona"
            | "background"
            | "speech"
            | "speech_style"
            | "goals"
            | "fallback"
            | "fallback_lines"
            | "knows"
            | "does_not_know"
            | "unknown"
            | "cooldown"
            | "cooldown_s"
            | "daily_tokens"
            | "budget"
            | "principal"
            | "watch"
    )
}

fn parse_tier(value: &str) -> Result<crate::profile::NpcTier, CmdError> {
    use crate::profile::NpcTier;
    match value.trim().to_ascii_lowercase().as_str() {
        "major" => Ok(NpcTier::Major),
        "minor" | "" => Ok(NpcTier::Minor),
        "extra" => Ok(NpcTier::Extra),
        other => Err(CmdError::msg(format!(
            "Unknown tier {other:?}. Choose major, minor, or extra."
        ))),
    }
}

fn parse_bool(value: &str) -> Result<bool, CmdError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" | "enabled" => Ok(true),
        "false" | "off" | "no" | "0" | "disabled" | "" => Ok(false),
        other => Err(CmdError::msg(format!(
            "Expected a yes/no value for enabled, got {other:?}."
        ))),
    }
}

/// Parse an optional u32: empty / `none` / `default` / `0` clears the override.
fn parse_opt_u32(value: &str) -> Result<Option<u32>, CmdError> {
    let v = value.trim().to_ascii_lowercase();
    if v.is_empty() || v == "none" || v == "default" || v == "0" {
        return Ok(None);
    }
    v.parse::<u32>().map(Some).map_err(|_| {
        CmdError::msg(format!(
            "Expected a whole number of seconds, got {value:?}."
        ))
    })
}

/// Parse an optional u64: empty / `none` / `default` / `0` clears the override.
fn parse_opt_u64(value: &str) -> Result<Option<u64>, CmdError> {
    let v = value.trim().to_ascii_lowercase();
    if v.is_empty() || v == "none" || v == "default" || v == "0" {
        return Ok(None);
    }
    v.parse::<u64>()
        .map(Some)
        .map_err(|_| CmdError::msg(format!("Expected a whole number of tokens, got {value:?}.")))
}

// ----------------------------------------------------------------------
// Snapshot assembly
// ----------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn build_dialogue_snapshot(
    ctx: &Ctx<'_>,
    grounding: &dyn AgentGrounding,
    npc: ObjId,
    profile: &AgentProfile,
    room: ObjId,
    present: &[ObjId],
    batch: &[Stimulus],
    correlation: u64,
) -> TurnSnapshot {
    // A crowded room is projected through a fixed-size view, but the players
    // responsible for this exact batch must win admission over unrelated
    // occupants. Otherwise crowding could make direct speech appear to come
    // from someone absent from the model's scene.
    let mut prioritized_present = Vec::with_capacity(MAX_SCENE_PRESENT);
    for subject in batch
        .iter()
        .filter_map(Stimulus::subject)
        .chain(present.iter().copied())
    {
        let is_present_player =
            ctx.w.get(subject).is_some_and(|object| {
                object.kind == ObjKind::Player && object.location == Some(room)
            }) && ctx.w.is_connected(subject);
        if is_present_player && !prioritized_present.contains(&subject) {
            prioritized_present.push(subject);
            if prioritized_present.len() == MAX_SCENE_PRESENT {
                break;
            }
        }
    }
    let mut scene = build_scene(ctx, npc, room, &prioritized_present);
    let agent_profile = grounding.profile(ctx.w, npc).and_then(|profile| {
        if let Err(error) = validate_agent_capability_profile(&profile) {
            tracing::error!(npc = npc.0, %error, "npc: invalid ruleset capability profile omitted");
            return None;
        }
        Some(profile)
    });
    let present_names: Vec<String> = scene
        .present
        .iter()
        .map(|player| player.name.clone())
        .collect();

    let (summary, transcript, recalled) = memory_context(ctx, npc, &present_names);
    let stimuli = batch.iter().map(|s| render_line(ctx.w, s)).collect();
    let actor = batch.iter().find_map(Stimulus::subject);
    // S6: the interface abilities the profile names, from the registry.
    let interfaces = crate::interface::tools_for(ctx.reg, profile);

    // World / semantic knowledge: per-NPC lore UNION resolved shared-pool
    // entries, filtered to shareable (Public + Guarded) — a Secret entry never
    // enters the snapshot (defense-in-depth layer (a)).
    let knowledge = resolve_shareable_knowledge(ctx, npc);

    // Per-asker Familiarity (volatile trust) is annotated onto each present
    // player from durable memory, so the volatile user prompt can frame guarded
    // sharing. `build_scene` has no state, so we set it here by name.
    if let Some(h) = ctx.state::<NpcHostState>() {
        for pp in &mut scene.present {
            pp.familiarity = familiarity_of(&h.long_term, npc, &pp.name);
        }
    }

    TurnSnapshot {
        kind: TurnKind::Dialogue,
        npc,
        npc_name: bounded_text(display_name(ctx.w, npc), MAX_SNAPSHOT_NAME_BYTES),
        profile: profile.clone(),
        agent_profile,
        scene,
        knowledge,
        summary,
        recalled,
        transcript,
        stimuli,
        actor,
        fold_events: Vec::new(),
        seed: correlation,
        interfaces,
    }
}

/// Resolve `npc`'s world knowledge for a turn: per-NPC [`LoreEntry`]s
/// ([`KNOWLEDGE_ATTR`]) unioned with the entries of every shared pool it tags,
/// then filtered to the **shareable** (Public + Guarded) entries. A
/// [`Secrecy::Secret`](crate::knowledge::Secrecy::Secret) fact is dropped here
/// so it never enters the [`TurnSnapshot`].
fn resolve_shareable_knowledge(ctx: &Ctx<'_>, npc: ObjId) -> Vec<crate::knowledge::LoreEntry> {
    let knowledge = read_knowledge(ctx.w, npc);
    let mut all = knowledge.lore;
    if let Some(h) = ctx.state::<NpcHostState>() {
        all.extend(h.lore_pools.resolve_shareable(&knowledge.tags));
    }
    let mut resolved = Vec::new();
    let mut bytes = 0_usize;
    for entry in all.into_iter().filter(LoreEntry::is_shareable) {
        if resolved.len() >= MAX_RESOLVED_LORE_ENTRIES {
            break;
        }
        let Ok(encoded) = serde_json::to_vec(&entry) else {
            continue;
        };
        let Some(next_bytes) = bytes.checked_add(encoded.len()) else {
            break;
        };
        if next_bytes > MAX_RESOLVED_LORE_BYTES {
            break;
        }
        bytes = next_bytes;
        resolved.push(entry);
    }
    resolved
}

fn build_summarize_snapshot(
    ctx: &Ctx<'_>,
    npc: ObjId,
    profile: &AgentProfile,
    room: ObjId,
    present: &[ObjId],
    fold: Vec<String>,
    correlation: u64,
) -> TurnSnapshot {
    let scene = build_scene(ctx, npc, room, present);
    let summary = read_summary(ctx.w, npc);
    TurnSnapshot {
        kind: TurnKind::Summarize,
        npc,
        npc_name: bounded_text(display_name(ctx.w, npc), MAX_SNAPSHOT_NAME_BYTES),
        profile: profile.clone(),
        agent_profile: None,
        scene,
        knowledge: Vec::new(),
        summary,
        recalled: Vec::new(),
        transcript: String::new(),
        stimuli: Vec::new(),
        actor: None,
        fold_events: fold,
        seed: correlation,
        interfaces: Vec::new(),
    }
}

fn build_scene(ctx: &Ctx<'_>, npc: ObjId, room: ObjId, present: &[ObjId]) -> SceneView {
    let room_name = bounded_text(display_name(ctx.w, room), MAX_SNAPSHOT_NAME_BYTES);
    let room_desc = bounded_text(
        attr_text(ctx.w, room, "short_desc")
            .or_else(|| attr_text(ctx.w, room, "desc"))
            .unwrap_or_default(),
        MAX_SCENE_DESCRIPTION_BYTES,
    );
    let present = present
        .iter()
        .filter(|&&p| p != npc)
        .take(MAX_SCENE_PRESENT)
        .map(|&p| PresentPlayer {
            name: bounded_text(display_name(ctx.w, p), MAX_SNAPSHOT_NAME_BYTES),
            short_desc: bounded_text(
                attr_text(ctx.w, p, "short_desc").unwrap_or_default(),
                MAX_PRESENT_DESCRIPTION_BYTES,
            ),
            // Volatile per-asker trust is filled in by the dialogue-snapshot
            // builder (which has host state); the summarize path leaves the
            // default (Stranger).
            familiarity: Familiarity::default(),
        })
        .collect();
    let exits = ctx
        .w
        .exits_of(room)
        .iter()
        .filter_map(|&e| {
            ctx.w
                .get(e)
                .map(|o| bounded_text(primary(&o.name).to_string(), MAX_SNAPSHOT_NAME_BYTES))
        })
        .take(MAX_SCENE_EXITS)
        .collect();
    let weather =
        weather_digest(ctx, room).map(|weather| bounded_text(weather, MAX_SCENE_WEATHER_BYTES));
    SceneView {
        room_name,
        room_desc,
        present,
        time: bounded_text(ctx.ic_datetime(), MAX_SCENE_TIME_BYTES),
        weather,
        exits,
    }
}

/// Read the summary + recent transcript + recalled memories for the prompt.
fn memory_context(
    ctx: &Ctx<'_>,
    npc: ObjId,
    present_names: &[String],
) -> (String, String, Vec<RecalledMemory>) {
    let attr_summary = read_summary(ctx.w, npc);
    let Some(h) = ctx.state::<NpcHostState>() else {
        return (attr_summary, String::new(), Vec::new());
    };
    let transcript = h
        .runtime(npc)
        .map(|rt| rt.memory.transcript(h.config.transcript_chars))
        .unwrap_or_default();
    let summary = h
        .runtime(npc)
        .map(|rt| rt.memory.summary.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(attr_summary);
    let recalled = h
        .long_term
        .recall(npc, present_names, h.config.recall_top_n)
        .into_iter()
        .map(|r| RecalledMemory {
            subject: r.subject,
            notes: r.notes,
        })
        .collect();
    (summary, transcript, recalled)
}

fn render_line(w: &World, s: &Stimulus) -> StimulusLine {
    match s {
        Stimulus::SpokenTo { by, text, kind } => StimulusLine {
            who: bounded_text(display_name(w, *by), MAX_SNAPSHOT_NAME_BYTES),
            text: text.clone(),
            directed: true,
            tag: speech_tag(*kind).to_string(),
        },
        Stimulus::HeardInRoom { by, text } => StimulusLine {
            who: bounded_text(display_name(w, *by), MAX_SNAPSHOT_NAME_BYTES),
            text: text.clone(),
            directed: false,
            tag: "say".to_string(),
        },
        Stimulus::PlayerArrived { who } => StimulusLine {
            who: bounded_text(display_name(w, *who), MAX_SNAPSHOT_NAME_BYTES),
            text: String::new(),
            directed: false,
            tag: "arrived".to_string(),
        },
        Stimulus::PlayerLeft { who } => StimulusLine {
            who: bounded_text(display_name(w, *who), MAX_SNAPSHOT_NAME_BYTES),
            text: String::new(),
            directed: false,
            tag: "left".to_string(),
        },
        Stimulus::AmbientTimer => StimulusLine {
            who: String::new(),
            text: String::new(),
            directed: false,
            tag: "ambient".to_string(),
        },
        Stimulus::GameEvent { digest } => StimulusLine {
            who: String::new(),
            text: digest.clone(),
            directed: false,
            tag: "event".to_string(),
        },
        Stimulus::StaffPoke { by, note } => StimulusLine {
            who: bounded_text(display_name(w, *by), MAX_SNAPSHOT_NAME_BYTES),
            text: note.clone(),
            directed: true,
            tag: "poke".to_string(),
        },
    }
}

// ----------------------------------------------------------------------
// Emission helpers (NPC speech through the normal formatter)
// ----------------------------------------------------------------------

/// Say/pose as the NPC: the normal `Narrative` event with the NPC's *true*
/// name (it cannot forge another speaker), plus the `Said` observer signal so
/// other agents / comsys logging hear it, plus a self-memory line.
fn say_as_npc(ctx: &mut Ctx<'_>, npc: ObjId, room: ObjId, kind: NarrativeKind, text: &str) {
    emit_room(ctx, room, kind, npc, text);
    ctx.emit_world_event(&WorldEvent::Said {
        who: npc,
        room,
        kind,
        text: text.to_string(),
    });
    let name = display_name(ctx.w, npc);
    record_self(ctx, npc, &format_speech(&name, kind, text));
}

/// One narrative line from `actor` to `to` alone (S8.2): the asker hears the
/// NPC speak; no one else does.
fn speak_to(ctx: &mut Ctx<'_>, actor: ObjId, to: ObjId, kind: NarrativeKind, text: &str) {
    let ev = OutputEvent::Narrative {
        kind,
        actor: Some(obj_ref(ctx.w, actor)),
        text: RichText::plain(text),
    };
    ctx.out.to_actor(to, ev);
}

fn emit_room(ctx: &mut Ctx<'_>, room: ObjId, kind: NarrativeKind, actor: ObjId, text: &str) {
    let ev = OutputEvent::Narrative {
        kind,
        actor: Some(obj_ref(ctx.w, actor)),
        text: RichText::plain(text),
    };
    ctx.out.to_room(room, Vec::new(), ev);
}

fn record_self(ctx: &mut Ctx<'_>, npc: ObjId, line: &str) {
    let ts = ctx.w.now();
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        let rt = h.runtime_mut(npc);
        rt.memory.push(ts, line);
        if rt.memory.is_overflowing() {
            rt.summarize_pending = true;
        }
    }
}

fn emit_thinking(ctx: &mut Ctx<'_>, npc: ObjId, room: ObjId) {
    emit_thinking_frame(ctx, npc, room, true);
}

/// The end of a `npc.thinking` window: the same frame with `active: false`,
/// so a client clears its pill on the turn's completion instead of waiting
/// out its own timer (interface plan I1, D-I-13).
fn emit_thinking_done(ctx: &mut Ctx<'_>, npc: ObjId, room: ObjId) {
    emit_thinking_frame(ctx, npc, room, false);
}

fn emit_thinking_frame(ctx: &mut Ctx<'_>, npc: ObjId, room: ObjId, active: bool) {
    let panel = ctx.state::<NpcHostState>().and_then(|h| h.thinking.clone());
    if let Some(panel) = panel {
        let data = serde_json::json!({
            "npc": npc.to_string(),
            "name": display_name(ctx.w, npc),
            "active": active,
        });
        let aud = mush_core::Audience::Room {
            room,
            exclude: Vec::new(),
            subject: None,
        };
        panel.keyframe_to(ctx, aud, data);
    }
}

fn notify_staff_once(ctx: &mut Ctx<'_>, npc: ObjId, reason: &str) {
    let already = ctx
        .state::<NpcHostState>()
        .and_then(|h| h.runtime(npc))
        .is_some_and(|rt| rt.degraded_notified);
    if already {
        return;
    }
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        h.runtime_mut(npc).degraded_notified = true;
    }
    let name = display_name(ctx.w, npc);
    let msg = format!(
        "NPC {name} (#{}) degraded to a fallback line: {reason}",
        npc.0
    );
    let staff: Vec<ObjId> = ctx
        .w
        .connected()
        .filter(|&a| perm::has_role(ctx.w, a, Role::Royalty))
        .collect();
    for s in staff {
        ctx.out.sys_to_actor(s, "npc", msg.clone());
    }
}

fn defer_turn(ctx: &mut Ctx<'_>, npc: ObjId, correlation: u64, snap: &TurnSnapshot) -> bool {
    if let Err(error) = snap.validate() {
        tracing::error!(npc = npc.0, %error, "npc: invalid bounded snapshot; turn dropped");
        let now = now_ms(ctx.w);
        if let Some(host) = ctx.state_mut::<NpcHostState>() {
            host.finish_turn(npc, now);
        }
        notify_staff_once(ctx, npc, &error.to_string());
        return false;
    }
    let payload = match serde_json::to_string(snap) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(npc = npc.0, error = %e, "npc: unserializable snapshot; turn dropped");
            let nm = now_ms(ctx.w);
            if let Some(h) = ctx.state_mut::<NpcHostState>() {
                h.finish_turn(npc, nm);
            }
            notify_staff_once(ctx, npc, "the turn snapshot could not be serialized");
            return false;
        }
    };
    ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
        module: SmolStr::new(NS),
        actor: npc,
        correlation,
        payload,
    }));
    true
}

fn persist_memory(ctx: &mut Ctx<'_>, update: &crate::memory::MemoryUpdate) {
    if let Some(evicted) = &update.evicted {
        ctx.persist(ModuleOp {
            ns: SmolStr::new(NS),
            op: ModuleWrite::Del {
                key: record_key(evicted.npc, &evicted.subject),
            },
        });
    }
    let value = match serde_json::to_value(&update.record) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "npc: could not serialize memory record");
            return;
        }
    };
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::JsonSet {
            key: record_key(update.record.npc, &update.record.subject),
            value,
        },
    });
}

/// Persist one principal's bounded intel vec as the module doc keyed by
/// [`intel_key`] (whole-doc `JsonSet` on each file, like a memory subject doc).
/// The in-RAM [`IntelStore`] was already updated by
/// [`IntelStore::file`](crate::intel::IntelStore::file); this mirrors it durably.
fn persist_intel(ctx: &mut Ctx<'_>, principal: Principal, records: &[IntelRecord]) {
    let value = match serde_json::to_value(records) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "npc: could not serialize intel");
            return;
        }
    };
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::JsonSet {
            key: intel_key(principal),
            value,
        },
    });
}

fn store_profile(ctx: &mut Ctx<'_>, npc: ObjId, profile: &AgentProfile) -> Result<(), CmdError> {
    profile
        .validate()
        .map_err(|error| CmdError::msg(error.to_string()))?;
    let value = serde_json::to_value(profile)
        .map_err(|e| CmdError::msg(format!("Could not serialize the profile: {e}")))?;
    ctx.w
        .set_attr(npc, PROFILE_ATTR, Attr::json(value, ctx.actor))?;
    Ok(())
}

/// Read `npc`'s per-NPC world knowledge from [`KNOWLEDGE_ATTR`], defaulting to
/// an empty document when absent or unparseable.
fn read_knowledge(w: &World, npc: ObjId) -> NpcKnowledge {
    w.attr(npc, KNOWLEDGE_ATTR)
        .and_then(|a| a.as_json())
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .filter(|knowledge: &NpcKnowledge| knowledge.validate().is_ok())
        .unwrap_or_default()
}

/// Persist `npc`'s per-NPC world knowledge to [`KNOWLEDGE_ATTR`] (JSON), like
/// [`store_profile`] writes the profile. Wired by the M10d staff commands
/// (`@agent/lore` add/tags/del/clear).
fn store_knowledge(
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    knowledge: &NpcKnowledge,
) -> Result<(), CmdError> {
    knowledge
        .validate()
        .map_err(|error| CmdError::msg(error.to_string()))?;
    let value = serde_json::to_value(knowledge)
        .map_err(|e| CmdError::msg(format!("Could not serialize the knowledge: {e}")))?;
    ctx.w
        .set_attr(npc, KNOWLEDGE_ATTR, Attr::json(value, ctx.actor))?;
    Ok(())
}

/// Persist a shared lore pool as the module doc keyed by [`pool_key`], AND
/// mirror it into the in-RAM [`LorePools`] so the resolved view matches the
/// write (the pattern `remember`/`persist_memory` follows for durable memory).
/// Wired by the `@lore` staff commands.
fn persist_pool(ctx: &mut Ctx<'_>, pool: &LorePool) -> Result<(), CmdError> {
    pool.validate()
        .map_err(|error| CmdError::msg(error.to_string()))?;
    let value = serde_json::to_value(pool)
        .map_err(|error| CmdError::msg(format!("Could not serialize the lore pool: {error}")))?;
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        h.lore_pools
            .set(pool.clone())
            .map_err(|error| CmdError::msg(error.to_string()))?;
    }
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::JsonSet {
            key: pool_key(&pool.tag),
            value,
        },
    });
    Ok(())
}

/// Delete a shared lore pool: remove the persisted doc AND drop it from the
/// in-RAM [`LorePools`]. Returns whether a pool was present in RAM. Wired by
/// `@lore <tag>=clear`.
fn delete_pool(ctx: &mut Ctx<'_>, tag: &str) -> bool {
    let removed = ctx
        .state_mut::<NpcHostState>()
        .is_some_and(|h| h.lore_pools.remove(tag));
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::Del { key: pool_key(tag) },
    });
    removed
}

// ----------------------------------------------------------------------
// small world helpers
// ----------------------------------------------------------------------

/// Milliseconds derived from the world's (second-granularity) clock — the one
/// time base both the observers and the tick use.
fn now_ms(w: &World) -> i64 {
    w.now().saturating_mul(1000)
}

fn drop_mailbox(ctx: &mut Ctx<'_>, npc: ObjId) {
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        h.drop_mailbox(npc);
    }
}

pub(crate) fn read_profile(w: &World, npc: ObjId) -> Option<AgentProfile> {
    read_profile_raw(w, npc)
}

fn read_profile_raw(w: &World, npc: ObjId) -> Option<AgentProfile> {
    let v = w.attr(npc, PROFILE_ATTR)?.as_json()?;
    serde_json::from_value(v.clone())
        .ok()
        .filter(|profile: &AgentProfile| profile.validate().is_ok())
}

fn read_summary(w: &World, npc: ObjId) -> String {
    let summary = w
        .attr(npc, SUMMARY_ATTR)
        .and_then(|a| a.as_text())
        .unwrap_or_default()
        .to_string();
    crate::action::sanitize_line(&summary, MAX_MEMORY_SUMMARY_CHARS)
}

fn enabled_npc(w: &World, npc: ObjId) -> bool {
    read_profile(w, npc).is_some_and(|p| p.enabled)
}

fn players_present(w: &World, room: ObjId) -> Vec<ObjId> {
    w.contents_of(room)
        .iter()
        .copied()
        .filter(|&o| w.get(o).is_some_and(|x| x.kind == ObjKind::Player) && w.is_connected(o))
        .take(MAX_SCENE_PRESENT)
        .collect()
}

fn bounded_text(mut value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut boundary = maximum_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

fn is_connected_player(w: &World, who: ObjId) -> bool {
    w.get(who).is_some_and(|o| o.kind == ObjKind::Player) && w.is_connected(who)
}

fn npcs_in(w: &World, room: ObjId) -> Vec<ObjId> {
    w.contents_of(room)
        .iter()
        .copied()
        .filter(|&o| w.get(o).is_some_and(|x| x.kind == ObjKind::Npc))
        .collect()
}

/// Whether `text` addresses `npc` by (the first token of) its name.
fn mentions(w: &World, npc: ObjId, text: &str) -> bool {
    let Some(o) = w.get(npc) else { return false };
    let name = primary(&o.name).to_lowercase();
    if name.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|tok| tok == name)
        || o.aliases.iter().any(|a| {
            let a = a.to_lowercase();
            !a.is_empty()
                && lower
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|tok| tok == a)
        })
}

fn display_name(w: &World, id: ObjId) -> String {
    w.get(id)
        .map(|o| primary(&o.name).to_string())
        .unwrap_or_else(|| format!("#{}", id.0))
}

fn primary(name: &str) -> &str {
    name.split(';').next().unwrap_or(name).trim()
}

fn attr_text(w: &World, obj: ObjId, name: &str) -> Option<String> {
    w.attr(obj, name)
        .and_then(|a| a.as_text())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

fn weather_digest(ctx: &Ctx<'_>, room: ObjId) -> Option<String> {
    let (_m, provider) = ctx.reg.weather_query()?;
    let wx = provider.current(ctx.w, room)?;
    Some(wx.state.to_string())
}

fn speech_kind(kind: NarrativeKind) -> SpeechKind {
    match kind {
        NarrativeKind::Pose => SpeechKind::Pose,
        NarrativeKind::SemiPose => SpeechKind::SemiPose,
        NarrativeKind::Emit | NarrativeKind::Spoof => SpeechKind::Emit,
        _ => SpeechKind::Say,
    }
}

fn speech_tag(kind: SpeechKind) -> &'static str {
    match kind {
        SpeechKind::Say => "say",
        SpeechKind::Pose => "pose",
        SpeechKind::SemiPose => "semipose",
        SpeechKind::Emit => "emit",
    }
}

/// A script-formatted transcript line (matches the plain-text render shapes).
fn format_speech(name: &str, kind: NarrativeKind, text: &str) -> String {
    match kind {
        NarrativeKind::Say => format!("{name} says, \"{text}\""),
        NarrativeKind::Pose => format!("{name} {text}"),
        NarrativeKind::SemiPose => format!("{name}{text}"),
        _ => text.to_string(),
    }
}
