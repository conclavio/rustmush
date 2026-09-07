//! # mush-ruleset — the ruleset plugin framework
//!
//! The reusable machinery every game ruleset (oWoD20 next, TacSim later)
//! plugs into: a declarative, **composable** [`SheetSchema`], a validating
//! [`SheetStore`] over object attributes, a chargen state machine with
//! point-budget accounting, and generic `+sheet` / `@sheet/set` commands plus
//! a web query handler — all driven from the one landed module API
//! ([`mush_core::GameModule`] + [`mush_core::Registrar`]).
//!
//! A [`Ruleset`] **is** a [`GameModule`](mush_core::GameModule): it registers
//! its commands, hooks, ticks, and panels through the normal
//! [`Registrar`](mush_core::Registrar), and additionally answers
//! sheet-schema, chargen, migration, and NPC-grounding questions.
//!
//! ## What lives here
//!
//! | Module | Contents |
//! |---|---|
//! | [`schema`] | [`SheetSchema`], [`FieldType`], [`DerivedExpr`], and the composable [`SheetSchemaBuilder`] — the splat seam |
//! | [`doc`] | [`SheetDoc`] + [`ApprovalState`] with typed getters/setters |
//! | [`validate`] | [`validate`](validate::validate) + [`SheetError`] |
//! | [`store`] | [`SheetStore`] — the validating/auditing facade over `World` attrs (treaty ruling 7) |
//! | [`chargen`] | [`ChargenFlow`] + the [`BudgetPlan`](chargen::BudgetPlan) point accountant + the stage machine |
//! | [`render`] | the classic terminal `+sheet` renderer |
//! | [`commands`] | [`register_sheet_commands`](commands::register_sheet_commands) — reusable `+sheet`/`@sheet/set` + the web query |
//!
//! ## Sheet storage (treaty ruling 7)
//!
//! A character's sheet is a single versioned JSON document per (object,
//! ruleset), stored as [`AttrValue::Json`](mush_core::AttrValue) under
//! `sheet:<ruleset>`. [`SheetStore`] is the only sanctioned writer; it
//! validates against the schema before every write and marks the object dirty
//! for the write-through persistence layer.
//!
//! ## The splat seam (M6b depends on this)
//!
//! Schemas **compose**: a base oWoD sheet plus template extensions. A splat
//! sub-module seeds [`SheetSchemaBuilder::from_schema`] with the base and adds
//! its own sections/fields — a "Disciplines" map, a Blood Pool track, a
//! Generation background — **without editing the base**. See the [`schema`]
//! module docs for the worked example; the same pattern extends chargen
//! (add [`ChargenStage`](chargen::ChargenStage)s), abilities (extend a
//! section), and power pools (add fields).

pub mod chargen;
pub mod commands;
pub mod doc;
pub mod narration;
pub mod render;
pub mod schema;
pub mod store;
pub mod validate;
pub mod vitals;

use mush_core::{DefinitionRef, GameModule, ObjId, Outbox, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

pub use chargen::{
    BudgetLedger, BudgetPlan, ChargenFlow, ChargenIssue, ChargenProgress, ChargenStage, CostRule,
    FieldSelector, PoolDef, finalize as chargen_finalize, try_advance,
};
pub use commands::register_sheet_commands;
pub use doc::{ApprovalState, JobId, SheetDoc};
pub use narration::{
    ROLL_GAP, ROLL_TAG, RollSide, contested_narration, is_roll_narration, roll_narration,
};
pub use render::render_sheet;
pub use schema::{
    DamageLevelDef, DerivedExpr, FieldDef, FieldType, SchemaError, SectionLayout, SheetSchema,
    SheetSchemaBuilder, SheetSection, TrackSize,
};
pub use store::{MigrateFn, SHEET_ATTR_NS, SheetStore, sheet_attr_name};
pub use validate::{SheetError, validate};
pub use vitals::{
    SheetSchemaFps, SheetView, VitalsRevisions, VitalsSource, frame as vitals_frame, from_sheet,
    pool, pool_state, register_vitals, vitals_feed,
};

/// A ruleset identifier — the interned short id (`"wod"`, `"tacsim"`), also
/// the module name and the `sheet:<id>` storage suffix.
pub type RulesetId = SmolStr;

/// A ruleset plugin: a [`GameModule`] that additionally describes its
/// character sheet, drives chargen, migrates stored sheets, and grounds LLM
/// NPC agents (design doc 04 §3.2; treaty ruling 6).
///
/// A ruleset registers its commands/hooks/panels through the normal
/// [`Registrar`](mush_core::Registrar) in
/// [`GameModule::register`](mush_core::GameModule::register) — typically via
/// [`register_sheet_commands`](commands::register_sheet_commands) for the
/// generic sheet surface plus its own `+roll`/`+xp`/etc.
pub trait Ruleset: GameModule {
    /// The stable ruleset id (defaults to the module
    /// [`name`](mush_core::GameModule::name)).
    fn id(&self) -> RulesetId {
        RulesetId::new(self.name())
    }

    /// The human-facing ruleset name ("World of Darkness 20th Anniversary").
    fn display_name(&self) -> &str;

    /// The declarative sheet schema (single source of truth for `+sheet`,
    /// the web panel, `@sheet/set`, chargen, and LLM serialization).
    fn sheet_schema(&self) -> &SheetSchema;

    /// The chargen flow, if this ruleset guides character generation.
    fn chargen(&self) -> Option<&dyn ChargenFlow> {
        None
    }

    /// Migrate a sheet's `data` from `from_version` to the current schema
    /// version (the ruleset handles the whole chain in one call). The default
    /// accepts an already-current document and refuses anything older.
    ///
    /// # Errors
    ///
    /// Returns a [`SheetError`] if the data cannot be migrated.
    fn migrate_sheet(
        &self,
        from_version: u32,
        data: serde_json::Value,
    ) -> Result<serde_json::Value, SheetError> {
        let current = self.sheet_schema().version;
        if from_version == current {
            Ok(data)
        } else {
            Err(SheetError::msg(format!(
                "ruleset {:?} has no migration from sheet v{from_version} to v{current}",
                self.id()
            )))
        }
    }

    /// A [`MigrateFn`] closure delegating to [`Ruleset::migrate_sheet`], for
    /// handing to [`SheetStore::with_migration`] — but a `&self`-borrowing
    /// default cannot outlive `register`, so rulesets that need lazy
    /// migration build their own `MigrateFn` from a free function. This
    /// method exists as the documented shape; overriding it is optional.
    fn migrate_fn(&self) -> Option<MigrateFn> {
        None
    }

    /// Ground an NPC's sheet into a capability profile for the M8 LLM agent
    /// host (treaty ruling 6): a prose summary, trimmed stats, capability
    /// tags, and a tool manifest placeholder.
    fn agent_profile(&self, sheet: &SheetDoc) -> AgentCapabilityProfile;

    /// Invoke a named NPC ability the LLM agent requested (the validated
    /// `wod_roll` tool, real in M8 — treaty ruling 6).
    ///
    /// This runs **in the world task**, re-validated against current state
    /// after the agent's off-loop LLM turn (design 03 §6.4, §8): the LLM only
    /// *requests* the ability by typed name + input; the deterministic engine
    /// computes the outcome, never the model. The handler is handed the world
    /// and the outbox so a real ability (a dice roll) reads the NPC's own
    /// sheet, rolls through the shared server RNG, logs, and emits like a
    /// player action — the security fence being that it may only touch
    /// `npc`'s own sheet and never mutate other objects or bypass locks
    /// (design 03 §7).
    ///
    /// `w`/`out` are the same borrows a
    /// [`Ctx`](mush_core::Ctx) carries (`ctx.w`, `ctx.out`), so the M8 agent
    /// host / driver calls this straight from its
    /// [`AsyncCompletion`](mush_core::AsyncCompletion) re-entry. The default
    /// rejects every ability (mortal / stub rulesets).
    ///
    /// # Errors
    ///
    /// Returns an [`AbilityError`] if the ability is unknown or the input is
    /// invalid (e.g. an over-claimed pool referencing traits not on the sheet).
    fn invoke_ability(
        &self,
        w: &mut World,
        out: &mut Outbox,
        npc: ObjId,
        name: &str,
        input: serde_json::Value,
    ) -> Result<AbilityOutcome, AbilityError> {
        let _ = (w, out, npc, input);
        Err(AbilityError::Unknown(name.to_string()))
    }

    /// H2c: the projected legal action an NPC ability maps to, if it has one.
    ///
    /// Under K7.0's rule the LLM's choice already happened off-loop on the
    /// dialogue lane; the mapping from `(name, input)` to a registered action
    /// and its parameters is deterministic, so the completion enters the
    /// action coordinator directly with no pending decision. An ability with
    /// no projected offer returns `None` and stays on the legacy
    /// [`Self::invoke_ability`] path — the **residual**, pinned by
    /// `crates/mush-contract-tests/tests/npc_ability_residual.rs` so it can only
    /// shrink. The default maps nothing.
    fn projected_ability(
        &self,
        w: &World,
        npc: ObjId,
        name: &str,
        input: &serde_json::Value,
    ) -> Option<ProjectedAbility> {
        let _ = (w, npc, name, input);
        None
    }

    /// Give `npc` a fresh character sheet of `template` through this ruleset's
    /// validated sheet-write path, so an NPC agent has real traits for its
    /// abilities to read. Backs the `@agent/sheet` staff command via the
    /// driver's generic grounding adapter. `template` is ruleset-specific (a
    /// splat/template id where the ruleset has them; ignored by single-sheet
    /// rulesets). The default has no sheet bootstrap and errs.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the ruleset has no sheet bootstrap,
    /// the template id is unknown, or the validated write fails.
    fn init_sheet(
        &self,
        w: &mut World,
        npc: ObjId,
        template: &str,
        by: ObjId,
    ) -> Result<String, String> {
        let _ = (w, npc, template, by);
        Err("this ruleset has no character-sheet bootstrap".to_string())
    }
}

/// LLM NPC-agent grounding for one NPC (design doc 04 §5.7). The LLM layer
/// injects `summary` + `stats` into the agent's system prompt and exposes
/// `tools`; the ruleset validates any tool call against the NPC's own sheet.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentCapabilityProfile {
    /// Compact prose ("Strong brawler; poor social; Kindred of Clan Brujah").
    pub summary: String,
    /// Trimmed sheet: notable attributes/abilities/powers as JSON.
    pub stats: serde_json::Value,
    /// Capability tags ("combat:brawl", "social:weak", "vampire:brujah").
    pub tags: Vec<SmolStr>,
    /// The tool manifest exposed to the LLM (placeholder shape until M8).
    pub tools: Vec<AgentToolSpec>,
}

/// H2c: an NPC ability resolved to a registered action the coordinator
/// executes — the definition the actor's own projection must offer, and the
/// parameters in that offer's schema.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedAbility {
    /// The registered action definition.
    pub action: DefinitionRef,
    /// Its parameters, in the projected offer's schema.
    pub parameters: serde_json::Value,
}

/// One LLM tool the NPC may call (placeholder for the M8 provider layer).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentToolSpec {
    /// Tool name (`"wod_roll"`).
    pub name: SmolStr,
    /// Human description for the model.
    pub description: String,
    /// A JSON-Schema-ish parameter description.
    pub params_schema: serde_json::Value,
}

/// The structured result of an NPC ability invocation (stub for M6a; the real
/// `wod_roll` returns a dice outcome in M8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbilityOutcome {
    /// Whether the ability succeeded mechanically.
    pub ok: bool,
    /// A one-line human/LLM-facing summary.
    pub summary: String,
    /// Structured result data (the dice records, damage, etc.).
    pub data: serde_json::Value,
}

/// Why an NPC ability invocation failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AbilityError {
    /// No such ability.
    #[error("unknown ability: {0}")]
    Unknown(String),
    /// The ability exists but the input was invalid.
    #[error("invalid ability input: {0}")]
    Invalid(String),
    /// The ability is not supported yet (M8).
    #[error("ability not supported yet")]
    Unsupported,
}
