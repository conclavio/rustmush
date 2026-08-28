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
pub mod state;
pub mod xp;

use std::sync::Arc;

use mush_core::{
    GameModule, JobQueueSpec, ModError, ModuleBootData, ModuleCtx, PanelFeedHandle, Registrar, Role,
};
use mush_jobs::register_payload_handler;
use mush_ruleset::chargen::ChargenFlow;
use mush_ruleset::schema::SheetSchema;
use mush_ruleset::{
    AgentCapabilityProfile, AgentToolSpec, Ruleset, SheetDoc, SheetStore, register_sheet_commands,
};
use smol_str::SmolStr;

use crate::chargen::WodChargen;
use crate::config::WodConfig;
use crate::schema::{ATTRIBUTES, KNOWLEDGES, Mortal, SKILLS, TALENTS, Template, build_schema};
use crate::state::WodState;

/// The shared bundle captured by every WoD command closure: the composed
/// schema, the tunable config, the sheet store, and the roll panel feed.
pub(crate) struct WodCmd {
    pub schema: Arc<SheetSchema>,
    pub config: WodConfig,
    pub store: SheetStore,
    pub roll_feed: PanelFeedHandle,
}

/// The oWoD20 base ruleset (`"wod"`).
pub struct WodRuleset {
    schema: Arc<SheetSchema>,
    config: WodConfig,
    store: SheetStore,
    chargen: WodChargen,
    seed: Option<u64>,
}

impl WodRuleset {
    /// The base ruleset with the default (Mortal-only) template set and an
    /// OS-seeded RNG.
    #[must_use]
    pub fn new() -> WodRuleset {
        WodRuleset::with_templates(vec![Box::new(Mortal)], WodConfig::default(), None)
    }

    /// The base ruleset with a fixed RNG seed (deterministic tests).
    #[must_use]
    pub fn with_seed(seed: u64) -> WodRuleset {
        WodRuleset::with_templates(vec![Box::new(Mortal)], WodConfig::default(), Some(seed))
    }

    /// The ruleset composed from an explicit template set (the M6b splat entry
    /// point) with a config and optional RNG seed.
    #[must_use]
    pub fn with_templates(
        templates: Vec<Box<dyn Template>>,
        config: WodConfig,
        seed: Option<u64>,
    ) -> WodRuleset {
        let schema = Arc::new(build_schema("wod", &templates));
        let store = SheetStore::new(Arc::clone(&schema));
        let chargen = WodChargen::new(Arc::clone(&schema), config.clone());
        WodRuleset {
            schema,
            config,
            store,
            chargen,
            seed,
        }
    }

    /// The sheet store this ruleset uses (for tests / host code).
    #[must_use]
    pub fn store(&self) -> &SheetStore {
        &self.store
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

        // Generic sheet surface + web query + `sheet` namespace claim.
        register_sheet_commands(reg, Arc::clone(&self.schema), None)?;

        // Roll panel feed, then the command bundle.
        let roll_feed = reg.panel_feed("wod.roll")?;
        let cmd = Arc::new(WodCmd {
            schema: Arc::clone(&self.schema),
            config: self.config.clone(),
            store: self.store.clone(),
            roll_feed,
        });

        roll::register(reg, Arc::clone(&cmd))?;
        damage::register(reg, Arc::clone(&cmd))?;
        chargen_cmds::register(reg, Arc::clone(&cmd))?;
        xp::register(reg, Arc::clone(&cmd))?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        // Chargen approval: re-validate the budget, then stamp Approved.
        let store = self.store.clone();
        let config = self.config.clone();
        register_payload_handler(
            ctx.w,
            chargen_cmds::CHARGEN_HANDLER,
            Arc::new(move |ctx, job| chargen_cmds::chargen_approval(&config, &store, ctx, job)),
        )
        .map_err(ModError::msg)?;

        // XP spend: re-validate + apply atomically.
        let store = self.store.clone();
        let config = self.config.clone();
        register_payload_handler(
            ctx.w,
            xp::XP_HANDLER,
            Arc::new(move |ctx, job| xp::xp_approval(&config, &store, ctx, job)),
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
        &self.schema
    }

    fn chargen(&self) -> Option<&dyn ChargenFlow> {
        Some(&self.chargen)
    }

    fn agent_profile(&self, sheet: &SheetDoc) -> AgentCapabilityProfile {
        agent_profile(&self.schema, sheet)
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
