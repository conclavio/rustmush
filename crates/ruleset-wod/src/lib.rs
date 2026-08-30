//! # ruleset-wod — Classic World of Darkness 20th Anniversary (oWoD20)
//!
//! The **base engine** for the oWoD20 Storyteller System (V20 / M20 / W20
//! core), implemented as a [`Ruleset`] on the landed module + sheet framework.
//! This milestone (M6a) is the base **Mortal** template plus every shared
//! subsystem; the three splats (Vampire / Mage / Werewolf) are M6b sub-modules
//! that bolt on through the proven [`Template`](schema::Template) seam.
//!
//! ## What it provides
//!
//! - the classic sheet ([`schema`]): nine Attributes, Talents/Skills/Knowledges
//!   abilities with specialties, an open Backgrounds map, Willpower, the
//!   7-level Health track, Virtues, Merits/Flaws, Experience, Nature/Demeanor;
//! - `+roll` ([`roll`]) — the Storyteller pool, resolved against the sheet,
//!   with `/spec` `/wp` `/priv` `/nowound`, inline `vs`/`ext`/`job`/`contested`,
//!   auto wound penalty, a per-character roll-log ring, and the `wod.roll` panel;
//! - `+harm` / `+heal` / `+health` ([`damage`]) — the marking/upgrade/rollover
//!   Health rules;
//! - `+chargen` / `+stat` / `+spec` ([`chargen_cmds`]) — the priority point-buy
//!   with live budget tracking, finalizing into a `wod-chargen` approval job;
//! - `+xp` / `+xp/spend` / `+xp/award` ([`xp`]) — XP costs and atomic
//!   apply-on-approval through a `wod-xp` job;
//! - `+sheet` / `@sheet/set` and the web sheet query (reused from the framework).
//!
//! All costs and pools live in [`config::WodConfig`] so splats and house rules
//! adjust them; the dice RNG is seedable ([`WodRuleset::with_seed`]) for
//! deterministic tests.

pub mod action;
pub mod agent;
pub mod chargen;
pub mod chargen_cmds;
pub mod config;
pub mod damage;
pub mod dice;
pub mod health;
pub mod onboarding;
pub mod perception;
pub mod roll;
pub mod schema;
pub mod sheet_cmds;
pub mod splats;
pub mod st;
pub mod state;
pub mod wizard;
pub mod xp;

use std::sync::Arc;

use mush_core::{
    GameModule, HookPoint, JobQueueSpec, ModError, ModuleBootData, ModuleCtx, ObjId, Outbox,
    PanelFeedHandle, Registrar, Role, World, WorldEventKind,
};
use mush_jobs::register_payload_handler;
use mush_ruleset::chargen::ChargenFlow;
use mush_ruleset::schema::SheetSchema;
use mush_ruleset::{
    AbilityError, AbilityOutcome, AgentCapabilityProfile, Ruleset, SheetDoc, SheetStore,
};

use crate::chargen::WodChargen;
use crate::config::WodConfig;
use crate::onboarding::{OnboardingGate, OnboardingHook, OnboardingSpawn};
use crate::schema::Template;
use crate::splats::{
    DEFAULT_TEMPLATES, Dispatch, Splat, TemplateSchemas, all_splats, schema_only_splats,
};
use crate::state::WodState;

pub use crate::agent::{WOD_ROLL_ABILITY, agent_profile, wod_roll_tool_spec};
pub use crate::onboarding::OnboardingConfig;
pub use crate::st::{NS, run_st_async};

/// The shared bundle captured by every WoD command closure: the multi-template
/// [`Dispatch`] (per-template schemas + splat behaviours), the tunable config,
/// and the roll panel feed.
///
/// The dispatch helpers here are the M6b backbone — every sheet read, write,
/// render, roll and chargen step selects the character's **active schema/splat**
/// by their `template` field through one of these.
pub(crate) struct WodCmd {
    pub dispatch: Arc<Dispatch>,
    pub config: WodConfig,
    pub roll_feed: PanelFeedHandle,
}

impl WodCmd {
    /// The per-template schema table.
    pub(crate) fn schemas(&self) -> &TemplateSchemas {
        &self.dispatch.schemas
    }

    /// The default (fallback / mortal) schema.
    pub(crate) fn default_schema(&self) -> Arc<SheetSchema> {
        self.dispatch.schemas.default_schema()
    }

    /// A store over the default schema — reads never validate, so this is the
    /// schema-agnostic read path.
    pub(crate) fn default_store(&self) -> SheetStore {
        SheetStore::new(self.default_schema())
    }

    /// Read `obj`'s sheet (schema-agnostic; `None` if they have none).
    pub(crate) fn read(&self, w: &World, obj: ObjId) -> Option<SheetDoc> {
        self.default_store().get(w, obj)
    }

    /// Read `obj`'s sheet, or a fresh default (mortal) sheet if they have none.
    pub(crate) fn read_or_default(&self, w: &World, obj: ObjId) -> SheetDoc {
        self.read(w, obj)
            .unwrap_or_else(|| SheetDoc::new_default(&self.default_schema()))
    }

    /// The active schema for a sheet, selected by its `template` field.
    pub(crate) fn schema_for_doc(&self, doc: &SheetDoc) -> Arc<SheetSchema> {
        self.dispatch.schemas.schema_for_doc(doc)
    }

    /// A **validating** store over a sheet's active schema (the write path).
    pub(crate) fn store_for_doc(&self, doc: &SheetDoc) -> SheetStore {
        SheetStore::new(self.schema_for_doc(doc))
    }

    /// The splat active for a sheet, selected by its `template` field.
    pub(crate) fn splat_for_doc(&self, doc: &SheetDoc) -> Arc<dyn Splat> {
        self.dispatch.splats.for_doc(doc)
    }
}

/// The oWoD20 base ruleset (`"wod"`) with the M6b multi-template splats.
pub struct WodRuleset {
    dispatch: Arc<Dispatch>,
    config: WodConfig,
    chargen: WodChargen,
    seed: Option<u64>,
    onboarding: OnboardingConfig,
}

impl WodRuleset {
    /// The full game with the [`DEFAULT_TEMPLATES`] splat set and an OS-seeded
    /// RNG.
    #[must_use]
    pub fn new() -> WodRuleset {
        WodRuleset::with_splats(all_splats(DEFAULT_TEMPLATES), WodConfig::default(), None)
    }

    /// The full game with a fixed RNG seed (deterministic tests).
    #[must_use]
    pub fn with_seed(seed: u64) -> WodRuleset {
        WodRuleset::with_splats(
            all_splats(DEFAULT_TEMPLATES),
            WodConfig::default(),
            Some(seed),
        )
    }

    /// A ruleset composed from an explicit **schema-only** template set (the
    /// M6a entry point, kept for tests and custom schema splats). Each template
    /// gets per-template dispatch but no splat commands or chargen contribution.
    #[must_use]
    pub fn with_templates(
        templates: Vec<Box<dyn Template>>,
        config: WodConfig,
        seed: Option<u64>,
    ) -> WodRuleset {
        WodRuleset::with_splats(schema_only_splats(templates), config, seed)
    }

    /// The internal constructor: build the [`Dispatch`] from a splat set.
    fn with_splats(
        splats: Vec<Arc<dyn Splat>>,
        config: WodConfig,
        seed: Option<u64>,
    ) -> WodRuleset {
        let dispatch = Arc::new(Dispatch::build("wod", splats));
        let chargen = WodChargen::new(Arc::clone(&dispatch), config.clone());
        WodRuleset {
            dispatch,
            config,
            chargen,
            seed,
            onboarding: OnboardingConfig::default(),
        }
    }

    /// Set the onboarding configuration (the `chargen_required` gate + start
    /// rooms). Off by default, for back-compat. The driver builds this from
    /// `mushd.toml`'s `[onboarding]` section.
    #[must_use]
    pub fn onboarding(mut self, onboarding: OnboardingConfig) -> WodRuleset {
        self.onboarding = onboarding;
        self
    }

    /// A validating store over the default (mortal) schema (for tests / host
    /// code). Per-template writes go through [`WodCmd`]'s dispatch helpers.
    #[must_use]
    pub fn store(&self) -> SheetStore {
        SheetStore::new(self.dispatch.schemas.default_schema())
    }

    /// The active schema for a sheet, selected by its `template` field.
    #[must_use]
    pub fn schema_for(&self, doc: &SheetDoc) -> Arc<SheetSchema> {
        self.dispatch.schemas.schema_for_doc(doc)
    }
}

impl Default for WodRuleset {
    fn default() -> WodRuleset {
        WodRuleset::new()
    }
}

impl GameModule for WodRuleset {
    fn name(&self) -> &'static str {
        "wod"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // State slot (RNG + transients).
        let state = match self.seed {
            Some(s) => WodState::seeded(s),
            None => WodState::from_os(),
        };
        reg.state(state);

        // Approval queues.
        reg.job_queue(JobQueueSpec {
            name: chargen_cmds::CHARGEN_QUEUE.into(),
            title: "Character generation approvals".into(),
            min_role: Role::Royalty,
        })?;
        reg.job_queue(JobQueueSpec {
            name: xp::XP_QUEUE.into(),
            title: "XP spend approvals".into(),
            min_role: Role::Royalty,
        })?;

        // Roll panel feed, then the command bundle carrying dispatch.
        let roll_feed = reg.panel_feed("wod.roll")?;
        let cmd = Arc::new(WodCmd {
            dispatch: Arc::clone(&self.dispatch),
            config: self.config.clone(),
            roll_feed,
        });

        // Template-dispatching sheet surface (+sheet/@sheet/@sheet/set + the
        // web query), then the shared command sets.
        sheet_cmds::register(reg, Arc::clone(&cmd))?;
        roll::register(reg, Arc::clone(&cmd))?;
        action::register(reg, Arc::clone(&cmd))?;
        damage::register(reg, Arc::clone(&cmd))?;
        chargen_cmds::register(reg, Arc::clone(&cmd))?;
        xp::register(reg, Arc::clone(&cmd))?;

        // Every enabled splat registers its own commands (always active; each
        // self-gates on the actor's template).
        for splat in self.dispatch.splats.iter() {
            splat.register_commands(reg, Arc::clone(&cmd))?;
        }

        // M11 per-observer projection: the Obfuscate-vs-Auspex perception gate
        // (read-only Perceive hook + presence-tracking observer + +obfuscate).
        // Purely additive — with nobody cloaked it hides no one and rolls no
        // dice, so a game that never uses Obfuscate is byte-for-byte unchanged.
        perception::register(reg, Arc::clone(&cmd))?;

        // M13 Storyteller adjudication: the re-entry point for the two-phase
        // `+cast … proposing …` flow (ruling + interpretation). Purely additive
        // — nothing fires unless a mage proposes a working and the driver routes
        // the deferred ST call.
        reg.async_completion(Arc::new(st::WodCompletion::new(Arc::clone(&cmd))))?;

        // Onboarding (the `chargen_required` gate). All three seams are inert
        // unless the config turns gating on, so a default game is unaffected.
        reg.command_gate(Arc::new(OnboardingGate {
            cfg: self.onboarding.clone(),
            dispatch: Arc::clone(&self.dispatch),
        }));
        reg.hook(
            HookPoint::Connected,
            Arc::new(OnboardingHook {
                cfg: self.onboarding.clone(),
                cmd: Arc::clone(&cmd),
            }),
        );
        reg.subscribe(
            WorldEventKind::ObjCreated,
            Arc::new(OnboardingSpawn {
                cfg: self.onboarding.clone(),
            }),
        );
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        // M17 rehydrate: restore the scene-invisible subset of WodState from the
        // `mod:wod:*` docs, so a mid-scene hot restart is invisible (a cloaked
        // vampire stays cloaked, an accepted-but-unthrown ruling survives, an
        // in-progress extended roll keeps its accumulated successes). Runs BEFORE
        // the world task spawns and the gateway re-attaches, so the restored
        // state is in place before the first re-attached `look` renders. The RNG,
        // the roll-log ring, and chargen are NOT persisted (re-seed / transient /
        // re-derivable), so the slot is never replaced — only these fields are.
        let mut conceal_doc = None;
        let mut pending_doc = None;
        let mut extended_doc = None;
        for (key, value) in data.kv {
            match key.as_str() {
                "conceal" => conceal_doc = Some(value),
                "pending" => pending_doc = Some(value),
                "extended" => extended_doc = Some(value),
                // Tolerate unknown keys (forward schema).
                _ => {}
            }
        }
        let now = ctx.w.now();
        // Phase 1 — decode + validate against the live world (immutable borrow;
        // each decode borrow ends before the next statement).
        let conceal = match &conceal_doc {
            Some(v) => Some(WodState::decode_conceal(v, |id| ctx.w.get(id).is_some()).map_err(ModError::msg)?),
            None => None,
        };
        let pending = match &pending_doc {
            Some(v) => {
                Some(WodState::decode_pending(v, now, |id| ctx.w.get(id).is_some()).map_err(ModError::msg)?)
            }
            None => None,
        };
        let extended = match &extended_doc {
            Some(v) => {
                Some(WodState::decode_extended(v, |id| ctx.w.get(id).is_some()).map_err(ModError::msg)?)
            }
            None => None,
        };
        // Phase 2 — install into the slot (fields only) and note any doc that was
        // pruned so the store is re-written to match RAM.
        let mut fixups = Vec::new();
        if let Some(st) = ctx.w.module_state_mut::<WodState>() {
            if let Some((concealed, pierced, dropped)) = conceal {
                st.install_conceal(concealed, pierced);
                if dropped {
                    fixups.push(st.conceal_persist_op());
                }
            }
            if let Some((map, dropped)) = pending {
                st.install_pending(map);
                if dropped {
                    fixups.push(st.pending_persist_op());
                }
            }
            if let Some((map, dropped)) = extended {
                st.install_extended(map);
                if dropped {
                    fixups.push(st.extended_persist_op());
                }
            }
        }
        for op in fixups {
            ctx.persist(op);
        }

        // Chargen approval: re-validate the budget (template-aware), stamp
        // Approved, then lift the onboarding gate (move to the live start room).
        let dispatch = Arc::clone(&self.dispatch);
        let config = self.config.clone();
        let onboarding = self.onboarding.clone();
        register_payload_handler(
            ctx.w,
            chargen_cmds::CHARGEN_HANDLER,
            Arc::new(move |ctx, job| {
                chargen_cmds::chargen_approval(&config, &dispatch, &onboarding, ctx, job)
            }),
        )
        .map_err(ModError::msg)?;

        // XP spend: re-validate + apply atomically (template-aware).
        let dispatch = Arc::clone(&self.dispatch);
        let config = self.config.clone();
        register_payload_handler(
            ctx.w,
            xp::XP_HANDLER,
            Arc::new(move |ctx, job| xp::xp_approval(&config, &dispatch, ctx, job)),
        )
        .map_err(ModError::msg)?;
        Ok(())
    }
}

impl Ruleset for WodRuleset {
    fn display_name(&self) -> &str {
        "World of Darkness 20th Anniversary"
    }

    fn sheet_schema(&self) -> &SheetSchema {
        // The framework's single-schema hook returns the default (mortal)
        // schema; the live surfaces dispatch per character by `template`.
        self.dispatch.schemas.default_schema_ref()
    }

    fn chargen(&self) -> Option<&dyn ChargenFlow> {
        Some(&self.chargen)
    }

    fn agent_profile(&self, sheet: &SheetDoc) -> AgentCapabilityProfile {
        agent::agent_profile(&self.dispatch.schemas.schema_for_doc(sheet), sheet)
    }

    fn invoke_ability(
        &self,
        w: &mut World,
        out: &mut Outbox,
        npc: ObjId,
        name: &str,
        input: serde_json::Value,
    ) -> Result<AbilityOutcome, AbilityError> {
        agent::invoke(&self.dispatch, w, out, npc, name, input)
    }

    /// Give `npc` a fresh character sheet of `template` (default the empty
    /// string ⇒ `mortal`) through the same validated retemplate path
    /// `@sheet/set .../template=<id>` uses — the M8 `@agent/sheet` bootstrap so
    /// an NPC has real traits for `wod_roll`. An existing sheet is re-shaped
    /// (dots preserved where the templates share fields); a missing one starts
    /// from the mortal default. Written as `by`.
    fn init_sheet(
        &self,
        w: &mut World,
        npc: ObjId,
        template: &str,
        by: ObjId,
    ) -> Result<String, String> {
        let id = match template.trim().to_ascii_lowercase().as_str() {
            "" => "mortal".to_string(),
            other => other.to_string(),
        };
        if !self.dispatch.schemas.ids().iter().any(|t| t == &id) {
            return Err(format!(
                "unknown template {id:?}; choose one of: {}",
                self.dispatch.schemas.ids().join(", ")
            ));
        }
        let schema = self.dispatch.schemas.schema_for_id(&id);
        let default = self.dispatch.schemas.default_schema();
        let mut doc = SheetStore::new(default.clone())
            .get(w, npc)
            .unwrap_or_else(|| SheetDoc::new_default(&default));
        crate::splats::retemplate(&mut doc, &schema, &id);
        SheetStore::new(schema)
            .put_by(w, npc, &doc, by)
            .map_err(|es| {
                es.iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            })?;
        Ok(format!("Gave the NPC a {id} sheet."))
    }
}
