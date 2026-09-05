//! # ruleset-shadowrun — Shadowrun 5th Edition (SR5)
//!
//! The **second** game system on the `mush-ruleset` + `mush-dice` plugin API
//! (M10b): the deliberate proof that the framework — validated by
//! `ruleset-wod` — scales past oWoD's d10 Storyteller pool to SR5's d6
//! hit-counting pool, Limits, Edge, and the priority chargen system.
//!
//! ## What it provides
//!
//! - the flat SR5 sheet ([`schema`]): eight core Attributes + Edge/Essence/
//!   Magic/Resonance, active-skill / skill-group / knowledge maps, the two
//!   Condition Monitors, derived Limits / Initiative, magic/gear/economy;
//! - `+roll` ([`roll`]) — the d6 hit pool (5/6 = hit), resolved against the
//!   sheet, with the sensible Limit + wound modifier auto-applied, `/edge`
//!   push-the-limit (add Edge dice, waive the Limit, Rule of Six), glitch /
//!   critical-glitch surfacing, a per-character roll-log ring, and the `sr.roll`
//!   panel feed;
//! - `+damage` / `+heal` / `+condition` ([`condition`]) — the two Condition
//!   Monitors with Stun→Physical / Physical overflow and the wound modifier;
//! - `+init` ([`initiative`]) — Initiative = REA + INT + initiative dice;
//! - `+cast` / `+thread` / `+spells` / `+magic` ([`magic`]) — spellcasting +
//!   Drain, complex forms + Fading (sheet-tracked, pool-rolled);
//! - `+chargen` / `+stat` / `+spec` ([`chargen_cmds`], [`wizard`]) — the
//!   **priority** point-buy with a live budget, finalizing into a
//!   `shadowrun-chargen` approval job;
//! - `+karma` ([`karma`]) — the SR5 advancement currency (the XP analog), with
//!   costed spends applied atomically through a `shadowrun-karma` job;
//! - `+sheet` / `@sheet/set` and the schema-generic web sheet query (reused from
//!   the framework — the SR5 sheet renders with **no** web change).
//!
//! All tunable numbers ride [`data::Sr5Config`]; the dice RNG is seedable
//! ([`ShadowrunRuleset::with_seed`]) for deterministic tests.
//!
//! ## Per-server single ruleset
//!
//! `wod` and `shadowrun` both register `+roll`/`+sheet`/`+chargen`, so a server
//! runs **one** ruleset (`[rulesets].enabled = ["shadowrun"]`); enabling both
//! would collide command names. Per-character ruleset dispatch across systems is
//! a future concern.

pub mod agent;
pub mod chargen;
pub mod chargen_cmds;
pub mod condition;
pub mod data;
pub mod derive;
pub mod dice;
pub mod economy;
pub mod initiative;
pub mod karma;
pub mod magic;
pub mod roll;
pub mod schema;
pub mod sheet_cmds;
pub mod state;
pub mod wizard;

use std::sync::Arc;

use mush_core::{
    GameModule, JobQueueSpec, ModError, ModuleBootData, ModuleCtx, ObjId, Outbox, PanelFeedHandle,
    Registrar, Role, World,
};
use mush_jobs::register_payload_handler;
use mush_ruleset::chargen::ChargenFlow;
use mush_ruleset::schema::SheetSchema;
use mush_ruleset::{
    AbilityError, AbilityOutcome, AgentCapabilityProfile, Ruleset, SheetDoc, SheetStore,
};

pub use crate::agent::{SR_ROLL_ABILITY, agent_profile, sr_roll_tool_spec};
pub use crate::chargen::Sr5Chargen;
pub use crate::data::Sr5Config;
pub use crate::economy::{
    FENCE_DELAY_MAX_SECS, FENCE_DELAY_MIN_SECS, FENCE_QUOTE_TTL_SECS, FenceId, FenceQuote,
    FenceSettlement, InstalledAugmentation, LifestyleTier, MAX_AUGMENTATIONS,
    MAX_CATALOG_CODE_BYTES, MAX_FENCES, MAX_OPEN_QUOTES_PER_SELLER, MAX_QUOTES, NS, QuoteId,
    SR_CORE_PACK, SR_MONTH, SR_MONTH_CATCH_UP, SR_MONTH_PERIOD_SECS, SrAvailability,
    SrBoardObserver, SrCatalogSource, SrDelivery, SrEconomyConfig, SrEconomyStore, SrFence,
    default_lifestyle_costs, goods_instrument, load_store, quote_key,
};
pub use crate::state::Sr5State;

/// A second `on_world_loaded` over one live world is **not** a production path — a
/// restart composes a fresh engine — but the E6a boot reconciliation has to be
/// re-runnable to be testable, and `register_payload_handler` refuses a duplicate.
///
/// So a re-registration is tolerated: the handler already installed is the same
/// closure over the same wiring-frozen config, and the alternative — failing the
/// boot — would turn a re-entered load into a dead world. Every other registration
/// error still fails the boot.
fn tolerate_reregistration(r: Result<(), String>) -> Result<(), String> {
    match r {
        Err(e) if e.contains("already registered") => Ok(()),
        other => other,
    }
}

/// The shared bundle captured by every SR5 command closure: the (single) sheet
/// schema, the tunable config, and the roll panel feed.
pub(crate) struct Sr5Cmd {
    /// The one SR5 sheet schema (shared).
    pub schema: Arc<SheetSchema>,
    /// The tunable priority/Karma numbers.
    pub config: Sr5Config,
    /// The `sr.roll` panel feed.
    pub roll_feed: PanelFeedHandle,
}

impl Sr5Cmd {
    /// A validating store over the schema.
    pub(crate) fn store(&self) -> SheetStore {
        SheetStore::new(Arc::clone(&self.schema))
    }

    /// Read `obj`'s sheet (`None` if they have none).
    pub(crate) fn read(&self, w: &World, obj: ObjId) -> Option<SheetDoc> {
        self.store().get(w, obj)
    }

    /// Read `obj`'s sheet, or a fresh default sheet if they have none.
    pub(crate) fn read_or_default(&self, w: &World, obj: ObjId) -> SheetDoc {
        self.store().get_or_default(w, obj)
    }
}

/// The Shadowrun 5th Edition ruleset (`"shadowrun"`).
pub struct ShadowrunRuleset {
    schema: Arc<SheetSchema>,
    config: Sr5Config,
    chargen: Sr5Chargen,
    seed: Option<u64>,
    cmd: Option<Arc<Sr5Cmd>>,
    /// E6a's frozen `[shadowrun.economy]` (08f §9, answer 16), parsed host-side and
    /// handed in at wiring — never read from a document, never written by a command.
    econ: SrEconomyConfig,
}

impl ShadowrunRuleset {
    /// The ruleset with default config and an OS-seeded RNG.
    #[must_use]
    pub fn new() -> ShadowrunRuleset {
        ShadowrunRuleset::with_config(Sr5Config::default(), None)
    }

    /// The ruleset with a fixed RNG seed (deterministic tests).
    #[must_use]
    pub fn with_seed(seed: u64) -> ShadowrunRuleset {
        ShadowrunRuleset::with_config(Sr5Config::default(), Some(seed))
    }

    /// The ruleset with an explicit config and optional RNG seed.
    #[must_use]
    pub fn with_config(config: Sr5Config, seed: Option<u64>) -> ShadowrunRuleset {
        ShadowrunRuleset {
            schema: Arc::new(schema::build_schema("shadowrun")),
            chargen: Sr5Chargen::new(config.clone()),
            config,
            seed,
            cmd: None,
            econ: SrEconomyConfig::default(),
        }
    }

    /// Set the E6a `[shadowrun.economy]` configuration (08f §9, answer 16).
    ///
    /// The host parses and validates the block (`mush_server::config_ext`) and calls
    /// this at wiring; the default is `enabled = false`, which is exactly the
    /// pre-E6a server.
    #[must_use]
    pub fn economy(mut self, econ: SrEconomyConfig) -> ShadowrunRuleset {
        self.econ = econ;
        self
    }

    /// The frozen `[shadowrun.economy]` block.
    #[must_use]
    pub fn economy_config(&self) -> &SrEconomyConfig {
        &self.econ
    }

    /// The composed SR5 schema (shared).
    #[must_use]
    pub fn schema(&self) -> Arc<SheetSchema> {
        Arc::clone(&self.schema)
    }

    /// A validating [`SheetStore`] over the schema.
    #[must_use]
    pub fn store(&self) -> SheetStore {
        SheetStore::new(Arc::clone(&self.schema))
    }

    /// The tunable config (priority/Karma numbers).
    #[must_use]
    pub fn config(&self) -> &Sr5Config {
        &self.config
    }
}

impl Default for ShadowrunRuleset {
    fn default() -> ShadowrunRuleset {
        ShadowrunRuleset::new()
    }
}

impl GameModule for ShadowrunRuleset {
    fn name(&self) -> &'static str {
        "shadowrun"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // Seedable dice RNG + transient roll-log / chargen state.
        let state = match self.seed {
            Some(s) => Sr5State::seeded(s),
            None => Sr5State::from_os(),
        };
        reg.state(state);

        // Approval queues.
        reg.job_queue(JobQueueSpec {
            name: chargen_cmds::CHARGEN_QUEUE.into(),
            title: "Runner generation approvals".into(),
            min_role: Role::Royalty,
        })?;
        reg.job_queue(JobQueueSpec {
            name: karma::KARMA_QUEUE.into(),
            title: "Karma spend approvals".into(),
            min_role: Role::Royalty,
        })?;

        // The roll panel feed + the command bundle.
        let roll_feed = reg.panel_feed("sr.roll")?;
        let cmd = Arc::new(Sr5Cmd {
            schema: Arc::clone(&self.schema),
            config: self.config.clone(),
            roll_feed,
        });

        // The SR5 sheet surface (+sheet / @sheet / @sheet/set + the
        // `shadowrun.sheet` web query — schema-generic web payload, no web
        // change; @sheet/set recomputes derived stats).
        sheet_cmds::register(reg, Arc::clone(&cmd))?;

        roll::register(reg, Arc::clone(&cmd))?;
        condition::register(reg, Arc::clone(&cmd))?;
        initiative::register(reg, Arc::clone(&cmd))?;
        magic::register(reg, Arc::clone(&cmd))?;
        chargen_cmds::register(reg, Arc::clone(&cmd))?;
        karma::register(reg, Arc::clone(&cmd))?;

        // E6a: the run-and-pay layer. The state slot always exists (an
        // `enabled = false` server answers "not enabled" rather than hiding a verb —
        // E5c deviation 1), and the config is frozen into it at boot.
        reg.state(economy::SrEconomyStore::default());
        economy::commands::register(reg, Arc::clone(&cmd))?;

        self.cmd = Some(cmd);
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        // M17 rehydrate: restore the one persisted subsystem — in-progress
        // extended rolls — from the `mod:shadowrun:extended` doc, so a mid-scene hot
        // restart keeps an accumulating extended test's successes. Runs BEFORE
        // the world task spawns and the gateway re-attaches, so the restored
        // state is in place before the first re-attached `+roll` continues.
        // SR has NO fiction-critical scene state (no conceal/pending), so this
        // is the only rehydration; the RNG, roll-log ring, and chargen are NOT
        // persisted (re-seed / transient / re-derivable), so the slot is never
        // replaced — only the extended field is.
        //
        // E6a reads the same namespace: its two document families (`sr:profile:{obj}`,
        // `sr:run:{id}`) live alongside M17's one key, so the boot data is loaded once
        // and each half takes its own keys. `load` is total — it skips what it cannot
        // use and says so — so a corrupt profile or listing never fails the boot
        // (`economy::store`'s table says why).
        let econ_store = economy::load_store(&data.kv, self.econ.clone());
        let mut extended_doc = None;
        for (key, value) in data.kv {
            // The one persisted key; any other is tolerated (forward schema) — E6a's
            // included.
            if key == "extended" {
                extended_doc = Some(value);
            }
        }
        // Phase 1 — decode + validate against the live world (immutable borrow).
        let extended = match &extended_doc {
            Some(v) => Some(
                Sr5State::decode_extended(v, |id| ctx.w.get(id).is_some())
                    .map_err(ModError::msg)?,
            ),
            None => None,
        };
        // Phase 2 — install into the slot (field only) and note a pruned doc so
        // the store is re-written to match RAM.
        let mut fixups = Vec::new();
        if let Some(st) = ctx.w.module_state_mut::<Sr5State>()
            && let Some((map, dropped)) = extended
        {
            st.install_extended(map);
            if dropped {
                fixups.push(st.extended_persist_op());
            }
        }
        for op in fixups {
            ctx.persist(op);
        }

        // E6a: install the loaded slot, then run the layer's own boot — the
        // instrument, the three system accounts, the declared `sr.month` clock, and
        // the two reconciliation passes. It runs **after** the org, faction, economy
        // and party modules' own boots (the ruleset is last in the module list), so
        // every one of them is loaded before a question is asked of it.
        if let Some(slot) = ctx.w.module_state_mut::<economy::SrEconomyStore>() {
            *slot = econ_store;
        }
        let econ_cfg = self.econ.clone();
        economy::boot::run(ctx, &econ_cfg);

        let cmd = self
            .cmd
            .clone()
            .ok_or_else(|| ModError::msg("shadowrun register() did not run before load"))?;

        // Chargen approval: re-validate the budget, stamp Approved, then — E6a — open
        // the `sr.nuyen` account and mint the priority allotment. The mint is a
        // one-shot attributed post keyed `sr.chargen/{obj}` and **never fails the
        // approval** (08d deviation 11, M13).
        {
            let cmd = Arc::clone(&cmd);
            let config = self.config.clone();
            let econ = econ_cfg.clone();
            tolerate_reregistration(register_payload_handler(
                ctx.w,
                chargen_cmds::CHARGEN_HANDLER,
                Arc::new(move |ctx, job| {
                    chargen_cmds::chargen_approval(&config, &cmd, ctx, job)?;
                    chargen_cmds::chargen_economy(&econ, &config, &cmd, ctx, job);
                    Ok(())
                }),
            ))
            .map_err(ModError::msg)?;
        }
        // Karma spend: re-validate + apply atomically.
        {
            let cmd = Arc::clone(&cmd);
            let config = self.config.clone();
            tolerate_reregistration(register_payload_handler(
                ctx.w,
                karma::KARMA_HANDLER,
                Arc::new(move |ctx, job| karma::karma_approval(&config, &cmd, ctx, job)),
            ))
            .map_err(ModError::msg)?;
        }
        Ok(())
    }
}

impl Ruleset for ShadowrunRuleset {
    fn display_name(&self) -> &str {
        "Shadowrun 5th Edition"
    }

    fn sheet_schema(&self) -> &SheetSchema {
        self.schema.as_ref()
    }

    fn chargen(&self) -> Option<&dyn ChargenFlow> {
        Some(&self.chargen)
    }

    fn agent_profile(&self, sheet: &SheetDoc) -> AgentCapabilityProfile {
        agent::agent_profile(self.schema.as_ref(), sheet)
    }

    fn invoke_ability(
        &self,
        w: &mut World,
        out: &mut Outbox,
        npc: ObjId,
        name: &str,
        input: serde_json::Value,
    ) -> Result<AbilityOutcome, AbilityError> {
        agent::invoke(&self.schema, w, out, npc, name, input)
    }

    /// Give `npc` a fresh SR5 sheet so `sr_roll` has real traits to read — the
    /// `@agent/sheet` bootstrap. SR5 has a single flat sheet (no templates), so
    /// `template` is accepted for interface parity but ignored; a fresh default
    /// sheet is written (staff then set stats with `@sheet/set`). Written as `by`.
    fn init_sheet(
        &self,
        w: &mut World,
        npc: ObjId,
        template: &str,
        by: ObjId,
    ) -> Result<String, String> {
        let _ = template;
        let mut doc = SheetDoc::new_default(&self.schema);
        crate::derive::apply_derived(&mut doc);
        self.store().put_by(w, npc, &doc, by).map_err(|es| {
            es.iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        Ok("Gave the NPC a Shadowrun sheet.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_ruleset::validate;

    #[test]
    fn ruleset_identity_and_schema() {
        let rs = ShadowrunRuleset::new();
        assert_eq!(rs.name(), "shadowrun");
        assert_eq!(rs.id(), "shadowrun");
        assert_eq!(rs.display_name(), "Shadowrun 5th Edition");
        assert!(rs.chargen().is_some());
        assert!(rs.sheet_schema().field("body").is_some());
    }

    #[test]
    fn default_sheet_validates() {
        let rs = ShadowrunRuleset::new();
        let data = rs.sheet_schema().default_data();
        assert!(validate(rs.sheet_schema(), &data).is_ok());
    }

    #[test]
    fn agent_profile_reads_own_sheet() {
        let rs = ShadowrunRuleset::new();
        let mut doc = SheetDoc::new_default(&rs.schema());
        doc.set_text("metatype", "elf");
        doc.set_text("awakened_type", "magician");
        let prof = rs.agent_profile(&doc);
        assert!(prof.summary.contains("elf"));
        assert!(prof.tags.iter().any(|t| t == "magic:awakened"));
        assert!(prof.tools.iter().any(|t| t.name == SR_ROLL_ABILITY));
    }
}
