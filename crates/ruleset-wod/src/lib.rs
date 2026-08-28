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

pub mod chargen;
pub mod chargen_cmds;
pub mod config;
pub mod damage;
pub mod dice;
pub mod health;
pub mod roll;
pub mod schema;
pub mod sheet_cmds;
pub mod splats;
pub mod state;
pub mod xp;

use std::sync::Arc;

use mush_core::{
    GameModule, JobQueueSpec, ModError, ModuleBootData, ModuleCtx, ObjId, PanelFeedHandle,
    Registrar, Role, World,
};
use mush_jobs::register_payload_handler;
use mush_ruleset::chargen::ChargenFlow;
use mush_ruleset::schema::SheetSchema;
use mush_ruleset::{AgentCapabilityProfile, AgentToolSpec, Ruleset, SheetDoc, SheetStore};
use smol_str::SmolStr;

use crate::chargen::WodChargen;
use crate::config::WodConfig;
use crate::schema::{ATTRIBUTES, KNOWLEDGES, SKILLS, TALENTS, Template};
use crate::splats::{
    DEFAULT_TEMPLATES, Dispatch, Splat, TemplateSchemas, all_splats, schema_only_splats,
};
use crate::state::WodState;

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
        }
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
        damage::register(reg, Arc::clone(&cmd))?;
        chargen_cmds::register(reg, Arc::clone(&cmd))?;
        xp::register(reg, Arc::clone(&cmd))?;

        // Every enabled splat registers its own commands (always active; each
        // self-gates on the actor's template).
        for splat in self.dispatch.splats.iter() {
            splat.register_commands(reg, Arc::clone(&cmd))?;
        }
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        // Chargen approval: re-validate the budget (template-aware), then
        // stamp Approved.
        let dispatch = Arc::clone(&self.dispatch);
        let config = self.config.clone();
        register_payload_handler(
            ctx.w,
            chargen_cmds::CHARGEN_HANDLER,
            Arc::new(move |ctx, job| chargen_cmds::chargen_approval(&config, &dispatch, ctx, job)),
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
        agent_profile(&self.dispatch.schemas.schema_for_doc(sheet), sheet)
    }
}

/// Ground a sheet into an NPC capability profile: prose summary, notable
/// stats, capability tags, and the `wod_roll` tool manifest. Real content —
/// M8 NPCs roll from this.
#[must_use]
pub fn agent_profile(schema: &SheetSchema, sheet: &SheetDoc) -> AgentCapabilityProfile {
    let template = sheet.text("template").unwrap_or("mortal").to_string();

    // Notable attributes (>= 3) and abilities (>= 2), highest first.
    let mut notable_attrs: Vec<(&str, i64)> = ATTRIBUTES
        .iter()
        .filter_map(|(id, label, _)| sheet.number(id).filter(|n| *n >= 3).map(|n| (*label, n)))
        .collect();
    notable_attrs.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    let mut notable_abils: Vec<(&str, i64)> = TALENTS
        .iter()
        .chain(SKILLS)
        .chain(KNOWLEDGES)
        .filter_map(|(id, label)| sheet.number(id).filter(|n| *n >= 2).map(|n| (*label, n)))
        .collect();
    notable_abils.sort_by_key(|&(_, n)| std::cmp::Reverse(n));

    let wp = sheet.number("willpower").unwrap_or(0);
    let health = health_word(schema, sheet);

    let mut summary = format!("A {template}.");
    if let Some((label, n)) = notable_attrs.first() {
        summary.push_str(&format!(" Best attribute {label} {n}."));
    }
    if !notable_abils.is_empty() {
        let list: Vec<String> = notable_abils
            .iter()
            .take(3)
            .map(|(l, n)| format!("{l} {n}"))
            .collect();
        summary.push_str(&format!(" Skilled at {}.", list.join(", ")));
    }
    summary.push_str(&format!(" Willpower {wp}. Currently {health}."));

    let to_map = |pairs: &[(&str, i64)]| -> serde_json::Map<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(l, n)| ((*l).to_string(), serde_json::json!(n)))
            .collect()
    };
    let stats = serde_json::json!({
        "template": template,
        "attributes": to_map(&notable_attrs),
        "abilities": to_map(&notable_abils),
        "willpower": wp,
        "health": health,
    });

    let mut tags: Vec<SmolStr> = vec![SmolStr::new(format!("template:{template}"))];
    for (label, n) in notable_abils.iter().take(4) {
        if *n >= 3 {
            tags.push(SmolStr::new(format!(
                "skill:{}",
                label.to_lowercase().replace(' ', "_")
            )));
        }
    }

    AgentCapabilityProfile {
        summary,
        stats,
        tags,
        tools: vec![AgentToolSpec {
            name: SmolStr::new("wod_roll"),
            description: "Roll a Storyteller-System dice pool (attribute + ability + modifier) \
                 against a difficulty. Returns successes / failure / botch."
                .to_string(),
            params_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pool": { "type": "string", "description": "e.g. strength+brawl" },
                    "difficulty": { "type": "integer", "minimum": 2, "maximum": 10 }
                },
                "required": ["pool"]
            }),
        }],
    }
}

/// The health status word for the deepest filled level.
fn health_word(schema: &SheetSchema, sheet: &SheetDoc) -> String {
    let marks = sheet.track("health").unwrap_or_default();
    let filled = marks.iter().filter(|m| m.is_some()).count();
    if filled == 0 {
        return "unhurt".to_string();
    }
    let levels = schema
        .field("health")
        .and_then(|f| match &f.ty {
            mush_ruleset::schema::FieldType::Track { levels, .. } => Some(levels.clone()),
            _ => None,
        })
        .unwrap_or_default();
    if levels.is_empty() {
        return "wounded".to_string();
    }
    let idx = (filled - 1).min(levels.len() - 1);
    levels[idx].label.to_lowercase()
}
