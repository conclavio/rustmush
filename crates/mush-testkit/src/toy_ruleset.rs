//! # The `toy` test ruleset (16b cut R-b)
//!
//! A miniature, self-contained [`Ruleset`] whose only job is to carry the
//! **platform seams** the deleted game rulesets (`ruleset-wod`,
//! `ruleset-shadowrun`) used to carry for the engine's own tests. Nothing here
//! is a game: the numbers are arbitrary and deliberately small, so a platform
//! test asserts on the contract and never on a splat's arithmetic.
//!
//! ## What it carries
//!
//! | seam | surface |
//! |---|---|
//! | sheet + schema | [`toy_schema`], `+sheet` / `@sheet` / `@sheet/set`, the `toy.sheet` query (stored) and `toy.sheet_live` (overlay-aware) |
//! | vitals + I13 republish | [`ToyVitals`] through [`mush_ruleset::register_vitals`] (`toy.vitals`) |
//! | chargen + approval jobs | `+chargen`, `+chargen/status`, `+chargen/submit`, `+stat`, queue `toy-chargen`, payload handler `toy.chargen` |
//! | onboarding | the gate / `Connected` hook / `ObjCreated` spawn trio + [`on_approved`], driven by [`OnboardingConfig`] |
//! | NPC grounding | [`Ruleset::agent_profile`], the `toy_roll` ability, [`Ruleset::init_sheet`] |
//! | per-observer perception | `+toyhide on|off` + a `HookPoint::Perceive` hook (`awareness` vs `stealth`) |
//! | gated narration channel | `+toysubtle <target>=<text>`, delivered only to `awareness >= 3`, announced as [`WorldEvent::PowerUsed`] |
//! | authoritative action | [`authoritative`] — `toy-willpower-roll` / `toy-willpower-set` providers over a typed `roll_state` family |
//!
//! ## Two owners
//!
//! [`ToyRuleset::named`] takes the module name, and
//! [`ToyRuleset::state_only`] drops everything but the typed state family — so
//! two modules can own a family of the same name while only one publishes
//! vitals (the pattern
//! `crates/mush-ruleset/tests/i13_action_state_republish.rs` needs).
//!
//! ## Determinism
//!
//! [`ToyRuleset::with_seed`] seeds the module's RNG, so `toy_roll` is
//! reproducible. The perception gate and the narration channel are
//! **threshold** comparisons, not rolls: they consume no randomness at all,
//! which keeps a seeded test's dice stream independent of how many onlookers
//! are in the room.

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandGate, CommandSpec, Ctx, EventHook, GameModule, HookPoint,
    HookReadContext, JobQueueSpec, ModError, ModuleBootData, ModuleCtx, ObjId, ObjKind,
    OnboardingConfig, Outbox, Perceive, Registrar, Registry, Role, World, WorldEvent,
    WorldEventKind, WorldObserver, perm,
};
use mush_dice::owod20::{Owod20Opts, Owod20Outcome, Owod20Pool};
use mush_dice::{PoolMechanic, RollCtx};
use mush_jobs::register_payload_handler;
use mush_proto::{Vital, VitalRole};
use mush_ruleset::chargen::{
    BudgetPlan, ChargenFlow, ChargenIssue, ChargenStage, CostRule, FieldSelector, PoolDef,
};
use mush_ruleset::schema::{
    DamageLevelDef, FieldDef, SectionLayout, SheetSchema, SheetSchemaBuilder, SheetSection,
    TrackSize,
};
use mush_ruleset::{
    AbilityError, AbilityOutcome, AgentCapabilityProfile, AgentToolSpec, ApprovalState,
    ProjectedAbility, RollSide, Ruleset, SheetDoc, SheetStore, SheetView, VitalsSource,
    register_sheet_commands, register_vitals, roll_narration, sheet_attr_name,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::Deserialize;
use smol_str::SmolStr;

pub mod authoritative;

pub use authoritative::{ToyActionHandles, ToyRollState};

/// The default module / ruleset id.
pub const TOY: &str = "toy";

/// The one NPC ability name the toy exposes.
pub const TOY_ROLL_ABILITY: &str = "toy_roll";

/// The two template ids the `template` enum offers.
pub const TOY_TEMPLATES: [&str; 2] = ["alpha", "beta"];

/// The awareness rating an onlooker needs to receive a `+toysubtle` narration.
pub const SUBTLE_AWARENESS: i64 = 3;

/// The chargen budget: points spendable over the `physical` category.
pub const CHARGEN_POINTS: i32 = 5;

/// The chargen per-field cap inside that budget.
pub const CHARGEN_CAP: i32 = 4;

// ======================================================================
// Schema
// ======================================================================

/// The toy sheet: five attributes, three skills, a bio block with a two-option
/// `template` enum and one staff-only field, a Willpower pool (permanent
/// `willpower` + the `willpower_current` gauge the authoritative action owns),
/// a `hidden` flag, and a fixed seven-box health track.
///
/// `ruleset` is the module id, so two toys never collide on `sheet:<id>`.
#[must_use]
pub fn toy_schema(ruleset: &str) -> SheetSchema {
    SheetSchemaBuilder::new(ruleset, 1)
        .section(
            SheetSection::new("attributes", "Attributes", SectionLayout::grid(1, 5))
                .field(FieldDef::dots("strength", "Strength", 1, 5).category("physical"))
                .field(FieldDef::dots("dexterity", "Dexterity", 1, 5).category("physical"))
                .field(FieldDef::dots("stamina", "Stamina", 1, 5).category("physical"))
                .field(FieldDef::dots("perception", "Perception", 1, 5).category("mental"))
                .field(FieldDef::dots("wits", "Wits", 1, 5).category("mental")),
        )
        .expect("the toy attributes section is well formed")
        .section(
            SheetSection::new("skills", "Skills", SectionLayout::List)
                .field(FieldDef::dots("brawl", "Brawl", 0, 5).category("skills"))
                .field(FieldDef::dots("awareness", "Awareness", 0, 5).category("skills"))
                .field(FieldDef::dots("stealth", "Stealth", 0, 5).category("skills")),
        )
        .expect("the toy skills section is well formed")
        .section(
            SheetSection::new("bio", "Bio", SectionLayout::List)
                .field(FieldDef::text("concept", "Concept", 40))
                .field(FieldDef::enumeration(
                    "template",
                    "Template",
                    TOY_TEMPLATES.iter().map(SmolStr::new).collect(),
                ))
                .field(FieldDef::dots("secret", "ST Secret", 0, 5).staff_only()),
        )
        .expect("the toy bio section is well formed")
        .section(
            SheetSection::new("pools", "Pools", SectionLayout::List)
                .field(FieldDef::number("willpower", "Willpower", 0, 10))
                .field(
                    FieldDef::number("willpower_current", "Willpower Pool", 0, 10)
                        .vital(VitalRole::Pool),
                )
                // The `+toyhide` flag. Not a vital: it is a state bit, not a gauge.
                .field(FieldDef::number("hidden", "Hidden", 0, 1)),
        )
        .expect("the toy pools section is well formed")
        .section(
            SheetSection::new("health", "Health", SectionLayout::Tracks).field(
                FieldDef::track(
                    "health",
                    "Health",
                    TrackSize::Fixed(7),
                    vec![
                        DamageLevelDef::new("bruised", "Bruised", 0),
                        DamageLevelDef::new("hurt", "Hurt", -1),
                        DamageLevelDef::new("injured", "Injured", -1),
                        DamageLevelDef::new("wounded", "Wounded", -2),
                        DamageLevelDef::new("mauled", "Mauled", -2),
                        DamageLevelDef::new("crippled", "Crippled", -5),
                        DamageLevelDef::new("incap", "Incapacitated", -127),
                    ],
                )
                .vital(VitalRole::Track),
            ),
        )
        .expect("the toy health section is well formed")
        .build()
}

// ======================================================================
// Module state (the seeded RNG)
// ======================================================================

/// The toy's module-state slot: the shared dice RNG, seedable for determinism.
/// Nothing durable lives here — the authoritative roll's resource is a typed
/// action-state family, and the concealment flag is a sheet field.
#[derive(Clone, Debug)]
pub struct ToyState {
    rng: ChaCha8Rng,
}

impl ToyState {
    /// A slot with a fixed seed (deterministic tests).
    #[must_use]
    pub fn seeded(seed: u64) -> ToyState {
        ToyState {
            rng: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// A slot seeded from the OS.
    #[must_use]
    pub fn from_os() -> ToyState {
        ToyState {
            rng: ChaCha8Rng::from_os_rng(),
        }
    }

    /// The shared RNG (the legacy, non-authoritative lane — `toy_roll`).
    pub fn rng(&mut self) -> &mut ChaCha8Rng {
        &mut self.rng
    }
}

// ======================================================================
// Pool resolution
// ======================================================================

/// A pool expression resolved against a stored sheet: the trait fields it
/// named, their display labels, and the dice they add up to. The fence that
/// makes "the ruleset, not the model, sizes the pool" true — a term that is
/// not a numeric, non-staff field on the schema is refused, so raw numbers and
/// invented bonuses cannot inflate a pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPool {
    /// The summed dice.
    pub total: i64,
    /// The schema field ids, in the order named.
    pub fields: Vec<SmolStr>,
    /// Their display labels (`"Strength+Brawl"` is `labels.join("+")`).
    pub labels: Vec<String>,
}

impl ResolvedPool {
    /// `"Strength+Brawl"` — the label a roll line prints.
    #[must_use]
    pub fn label(&self) -> String {
        if self.labels.is_empty() {
            "(pool)".to_string()
        } else {
            self.labels.join("+")
        }
    }
}

/// Resolve `expr` (`"strength+brawl"`) against `doc` under `schema`.
///
/// # Errors
///
/// A term that is empty, not a schema field, staff-only, or not numeric on the
/// sheet is refused with a human-readable message.
pub fn resolve_pool(
    schema: &SheetSchema,
    doc: &SheetDoc,
    expr: &str,
) -> Result<ResolvedPool, String> {
    let mut resolved = ResolvedPool {
        total: 0,
        fields: Vec::new(),
        labels: Vec::new(),
    };
    for raw in expr.split('+') {
        let term = raw.trim().to_ascii_lowercase();
        if term.is_empty() {
            return Err("a pool term is empty".to_string());
        }
        let field = schema
            .field(&term)
            .ok_or_else(|| format!("{term:?} is not a trait on this sheet"))?;
        if field.staff_only {
            return Err(format!("{term:?} is not a trait you may roll"));
        }
        let value = doc
            .number(&term)
            .ok_or_else(|| format!("{term:?} has no rating on this sheet"))?;
        resolved.total += value;
        resolved.fields.push(SmolStr::new(&term));
        resolved.labels.push(field.label.clone());
    }
    if resolved.fields.is_empty() {
        return Err("that pool names no traits".to_string());
    }
    Ok(resolved)
}

// ======================================================================
// The ruleset
// ======================================================================

/// The shared bundle every toy command closure captures.
pub(crate) struct ToyCmd {
    pub(crate) name: &'static str,
    pub(crate) schema: Arc<SheetSchema>,
    pub(crate) roll_state: mush_core::StateFamilyHandle<ObjId, ToyRollState>,
}

impl ToyCmd {
    pub(crate) fn store(&self) -> SheetStore {
        SheetStore::new(Arc::clone(&self.schema))
    }

    /// Read `obj`'s stored sheet (no overlay).
    pub(crate) fn read(&self, w: &World, obj: ObjId) -> Option<SheetDoc> {
        self.store().get(w, obj)
    }

    /// Read `obj`'s sheet through the authoritative Willpower overlay — the
    /// typed record owns `willpower_current` once it exists.
    pub(crate) fn read_ctx(&self, ctx: &Ctx<'_>, obj: ObjId) -> Option<SheetDoc> {
        let mut doc = self.read(ctx.w, obj)?;
        if let Ok(Some(state)) = ctx.action_state(&self.roll_state, &obj) {
            overlay(&mut doc, state);
        }
        Some(doc)
    }
}

/// Apply the typed roll state onto a read sheet, clamped by the permanent
/// rating (the one place the overlay's arithmetic lives).
pub(crate) fn overlay(doc: &mut SheetDoc, state: &ToyRollState) {
    let permanent = doc.number("willpower").unwrap_or(0).clamp(0, 10);
    doc.set_number("willpower_current", state.willpower_current.min(permanent));
}

/// The toy ruleset module.
pub struct ToyRuleset {
    name: &'static str,
    schema: Arc<SheetSchema>,
    chargen: ToyChargen,
    seed: Option<u64>,
    onboarding: OnboardingConfig,
    state_only: bool,
}

impl ToyRuleset {
    /// The full toy under the default module id `"toy"`, OS-seeded.
    #[must_use]
    pub fn new() -> ToyRuleset {
        ToyRuleset::named(TOY)
    }

    /// The full toy under module id `name` (so two can coexist).
    #[must_use]
    pub fn named(name: &'static str) -> ToyRuleset {
        let schema = Arc::new(toy_schema(name));
        ToyRuleset {
            name,
            chargen: ToyChargen {
                schema: Arc::clone(&schema),
            },
            schema,
            seed: None,
            onboarding: OnboardingConfig::default(),
            state_only: false,
        }
    }

    /// The full toy with a fixed dice seed.
    #[must_use]
    pub fn with_seed(seed: u64) -> ToyRuleset {
        ToyRuleset::new().seed(seed)
    }

    /// Fix the dice seed (deterministic `toy_roll`).
    #[must_use]
    pub fn seed(mut self, seed: u64) -> ToyRuleset {
        self.seed = Some(seed);
        self
    }

    /// The onboarding configuration (the `chargen_required` gate + the two
    /// start rooms). Off by default, so an unconfigured toy gates nobody.
    #[must_use]
    pub fn onboarding(mut self, onboarding: OnboardingConfig) -> ToyRuleset {
        self.onboarding = onboarding;
        self
    }

    /// Register **only** the typed `roll_state` family: no commands, no
    /// vitals, no queues, no state slot. The second owner in the I13
    /// two-owner pattern, whose family shares a name with the publisher's and
    /// must still be filtered out of the republish.
    #[must_use]
    pub fn state_only(mut self) -> ToyRuleset {
        self.state_only = true;
        self
    }

    /// A validating store over the toy schema (for fixture setup).
    #[must_use]
    pub fn store(&self) -> SheetStore {
        SheetStore::new(Arc::clone(&self.schema))
    }

    /// The chargen queue this toy files approvals on (`"<id>-chargen"`).
    #[must_use]
    pub fn chargen_queue(&self) -> String {
        format!("{}-chargen", self.name)
    }

    /// The payload handler an approval job carries (`"<id>.chargen"`).
    #[must_use]
    pub fn chargen_handler(&self) -> String {
        format!("{}.chargen", self.name)
    }
}

impl Default for ToyRuleset {
    fn default() -> ToyRuleset {
        ToyRuleset::new()
    }
}

impl GameModule for ToyRuleset {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);

        // The typed action-state family is the one thing BOTH owners register.
        let roll_state = authoritative::register_family(reg, self.name)?;
        if self.state_only {
            return Ok(());
        }
        authoritative::register_actions(
            reg,
            self.name,
            Arc::clone(&self.schema),
            roll_state.clone(),
        )?;

        reg.state(match self.seed {
            Some(seed) => ToyState::seeded(seed),
            None => ToyState::from_os(),
        });

        // Both directions of the jobs delegation: chargen files a job through
        // the jobs API (writing as the caller), and `+job/approve` dispatches
        // this module's payload handler under the jobs module's identity.
        reg.persist_delegation(
            self.name,
            "jobs",
            "the toy chargen files its approval job through mush_jobs::open_job, whose \
             write carries the caller's identity",
        )?;
        reg.persist_delegation(
            "jobs",
            self.name,
            "`+job/approve` dispatches the toy chargen payload handler under the jobs \
             module's identity, and it writes toy-namespace sheets",
        )?;
        reg.job_queue(JobQueueSpec {
            name: SmolStr::new(format!("{}-chargen", self.name)),
            title: "Toy character generation approvals".into(),
            min_role: Role::Royalty,
        })?;

        // The generic sheet surface: `+sheet`, `@sheet`, `@sheet/set` and the
        // `<id>.sheet` query/feed.
        register_sheet_commands(reg, Arc::clone(&self.schema), None)?;

        let cmd = Arc::new(ToyCmd {
            name: self.name,
            schema: Arc::clone(&self.schema),
            roll_state,
        });

        // The `<id>.vitals` feed + the I13 republish-on-commit.
        register_vitals(
            reg,
            self.name,
            &sheet_attr_name(self.name),
            Arc::new(ToyVitals {
                cmd: Arc::clone(&cmd),
            }),
        )?;

        // The overlay-aware sheet query beside the generic (stored-only) one.
        let live_feed = SmolStr::new(format!("{}.sheet_live", self.name));
        reg.panel_feed(&live_feed)?;
        reg.query_handler(
            &live_feed,
            Arc::new(ToySheetLive {
                cmd: Arc::clone(&cmd),
                feed: live_feed.clone(),
            }),
        )?;

        commands::register(reg, Arc::clone(&cmd))?;

        // Per-observer perception: the concealment gate.
        reg.hook(
            HookPoint::Perceive,
            Arc::new(ToyPerception::new(Arc::clone(&self.schema))),
        );

        // Onboarding — all three seams inert unless `chargen_required` is on.
        reg.command_gate(Arc::new(OnboardingGate {
            cfg: self.onboarding.clone(),
            cmd: Arc::clone(&cmd),
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
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        if self.state_only {
            return Ok(());
        }
        let schema = Arc::clone(&self.schema);
        let chargen = ToyChargen {
            schema: Arc::clone(&self.schema),
        };
        let onboarding = self.onboarding.clone();
        let name = self.name;
        // Tolerant by design: a platform test that only wants the sheet, the
        // perception gate or the authoritative action composes the toy WITHOUT
        // `mush_jobs::JobsModule`, and must not be failed at boot for it.
        // `+chargen/submit` refuses on its own in that world (there is no queue
        // to file into), which is the honest failure.
        if ctx.w.module_state::<mush_jobs::JobsApi>().is_none() {
            return Ok(());
        }
        register_payload_handler(
            ctx.w,
            &format!("{name}.chargen"),
            Arc::new(move |ctx, job| {
                chargen_approval(&schema, &chargen, &onboarding, name, ctx, job)
            }),
        )
        .map_err(ModError::msg)?;
        Ok(())
    }
}

impl Ruleset for ToyRuleset {
    fn display_name(&self) -> &str {
        "Toy Ruleset"
    }

    fn sheet_schema(&self) -> &SheetSchema {
        &self.schema
    }

    fn chargen(&self) -> Option<&dyn ChargenFlow> {
        Some(&self.chargen)
    }

    /// Ground the stored sheet for an NPC's system prompt: the shape
    /// `ruleset-wod`'s `agent_profile` had (prose summary, trimmed stats,
    /// capability tags, the one tool manifest), with the toy's traits.
    fn agent_profile(&self, sheet: &SheetDoc) -> AgentCapabilityProfile {
        let template = sheet.text("template").unwrap_or("alpha").to_string();
        let mut notable: Vec<(String, i64)> = Vec::new();
        for id in ["strength", "dexterity", "stamina", "perception", "wits"] {
            if let Some(n) = sheet.number(id).filter(|n| *n >= 3) {
                notable.push((label_of(&self.schema, id), n));
            }
        }
        notable.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let mut skills: Vec<(String, i64)> = Vec::new();
        for id in ["brawl", "awareness", "stealth"] {
            if let Some(n) = sheet.number(id).filter(|n| *n >= 2) {
                skills.push((label_of(&self.schema, id), n));
            }
        }
        skills.sort_by_key(|(_, n)| std::cmp::Reverse(*n));

        let willpower = sheet.number("willpower").unwrap_or(0);
        let mut summary = format!("A {template}.");
        if let Some((label, n)) = notable.first() {
            summary.push_str(&format!(" Best attribute {label} {n}."));
        }
        if !skills.is_empty() {
            let list: Vec<String> = skills
                .iter()
                .take(3)
                .map(|(l, n)| format!("{l} {n}"))
                .collect();
            summary.push_str(&format!(" Skilled at {}.", list.join(", ")));
        }
        summary.push_str(&format!(" Willpower {willpower}."));

        let mut tags: Vec<SmolStr> = vec![SmolStr::new(format!("template:{template}"))];
        if sheet.number("brawl").unwrap_or(0) >= 3 {
            tags.push(SmolStr::new("combat:brawl"));
        } else {
            tags.push(SmolStr::new("combat:weak"));
        }
        if sheet.number("stealth").unwrap_or(0) >= 3 {
            tags.push(SmolStr::new("skill:stealth"));
        }

        AgentCapabilityProfile {
            summary,
            stats: serde_json::json!({
                "template": template,
                "attributes": notable.iter().map(|(l, n)| serde_json::json!([l, n])).collect::<Vec<_>>(),
                "skills": skills.iter().map(|(l, n)| serde_json::json!([l, n])).collect::<Vec<_>>(),
                "willpower": willpower,
            }),
            tags,
            tools: vec![toy_roll_tool_spec()],
        }
    }

    fn invoke_ability(
        &self,
        w: &mut World,
        out: &mut Outbox,
        npc: ObjId,
        name: &str,
        input: serde_json::Value,
    ) -> Result<AbilityOutcome, AbilityError> {
        match name {
            TOY_ROLL_ABILITY => toy_roll(&self.schema, w, out, npc, input),
            other => Err(AbilityError::Unknown(other.to_string())),
        }
    }

    /// A Willpower-backed `toy_roll` is the coordinator's own
    /// `<id>_willpower_roll`; every other ability stays on the legacy
    /// [`Ruleset::invoke_ability`] residual.
    fn projected_ability(
        &self,
        w: &World,
        _npc: ObjId,
        name: &str,
        input: &serde_json::Value,
    ) -> Option<ProjectedAbility> {
        if name != TOY_ROLL_ABILITY {
            return None;
        }
        let args: ToyRollInput = serde_json::from_value(input.clone()).ok()?;
        if !args.willpower {
            return None;
        }
        let handles = w.module_state::<ToyActionHandles>()?;
        Some(ProjectedAbility {
            action: handles.willpower_roll.clone(),
            parameters: serde_json::json!({
                "pool": args.pool_expr,
                "difficulty": u32::from(args.difficulty.unwrap_or(6).clamp(2, 10)),
                "modifier": args.modifier.clamp(-3, 3),
                "ignore_wounds": false,
            }),
        })
    }

    /// Give `npc` a fresh toy sheet of `template` (`""` ⇒ `alpha`) through the
    /// validated write path — the `@agent/sheet` bootstrap.
    fn init_sheet(
        &self,
        w: &mut World,
        npc: ObjId,
        template: &str,
        by: ObjId,
    ) -> Result<String, String> {
        let id = match template.trim().to_ascii_lowercase().as_str() {
            "" => "alpha".to_string(),
            other => other.to_string(),
        };
        if !TOY_TEMPLATES.iter().any(|t| *t == id) {
            return Err(format!(
                "unknown template {id:?}; choose one of: {}",
                TOY_TEMPLATES.join(", ")
            ));
        }
        let store = self.store();
        let mut doc = store
            .get(w, npc)
            .unwrap_or_else(|| SheetDoc::new_default(&self.schema));
        doc.set_text("template", &id);
        store.put_by(w, npc, &doc, by).map_err(|es| fmt_errs(&es))?;
        Ok(format!("Gave the NPC a {id} sheet."))
    }
}

/// A schema field's display label, falling back to its id.
fn label_of(schema: &SheetSchema, id: &str) -> String {
    schema
        .field(id)
        .map_or_else(|| id.to_string(), |f| f.label.clone())
}

/// Join a validation error list into one line.
pub(crate) fn fmt_errs(errors: &[mush_ruleset::SheetError]) -> String {
    errors
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

// ======================================================================
// Chargen
// ======================================================================

/// The toy chargen flow: one stage, [`CHARGEN_POINTS`] points over the
/// `physical` category, per-field cap [`CHARGEN_CAP`].
pub struct ToyChargen {
    schema: Arc<SheetSchema>,
}

impl ToyChargen {
    /// A flow over an already-built schema (the command path holds an `Arc`).
    #[must_use]
    pub fn for_schema(schema: Arc<SheetSchema>) -> ToyChargen {
        ToyChargen { schema }
    }

    fn plan() -> BudgetPlan {
        BudgetPlan::new()
            .pool(PoolDef::new("physical", "Physical", CHARGEN_POINTS))
            .rule(
                CostRule::new("physical", FieldSelector::Category("physical".into()), 1, 1)
                    .with_cap(CHARGEN_CAP),
            )
    }
}

impl ChargenFlow for ToyChargen {
    fn stages(&self) -> Vec<ChargenStage> {
        vec![
            ChargenStage::new("attrs", "Attributes", "Spend 5 physical points.")
                .with_budget(ToyChargen::plan()),
        ]
    }

    fn validate_stage(&self, sheet: &SheetDoc, stage: &SmolStr) -> Vec<ChargenIssue> {
        self.stages()
            .into_iter()
            .find(|s| &s.id == stage)
            .map(|s| s.assess(&self.schema, sheet))
            .unwrap_or_default()
    }

    fn finalize(&self, sheet: &mut SheetDoc) -> Result<(), ChargenIssue> {
        let issues = self.validate_stage(sheet, &SmolStr::new("attrs"));
        if let Some(first) = issues.into_iter().next() {
            return Err(first);
        }
        // Derived: Willpower = Stamina + 3, and the pool starts full.
        let willpower = sheet.number("stamina").unwrap_or(1) + 3;
        sheet.set_number("willpower", willpower);
        sheet.set_number("willpower_current", willpower);
        Ok(())
    }
}

/// The `<id>.chargen` payload handler: re-validate the budget, stamp the sheet
/// Approved, then lift the onboarding gate.
///
/// # Errors
///
/// A missing sheet, a sheet that no longer balances, or a rejected write.
pub(crate) fn chargen_approval(
    schema: &Arc<SheetSchema>,
    chargen: &ToyChargen,
    onboarding: &OnboardingConfig,
    _name: &str,
    ctx: &mut Ctx<'_>,
    job: &mush_jobs::Job,
) -> Result<(), String> {
    let target = job
        .payload
        .as_ref()
        .and_then(|p| p.data.get("target"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .map(ObjId)
        .ok_or_else(|| "job payload has no target".to_string())?;
    let store = SheetStore::new(Arc::clone(schema));
    let mut doc = store
        .get(ctx.w, target)
        .ok_or_else(|| "the character has no sheet".to_string())?;
    if let Some(issue) = chargen
        .validate_stage(&doc, &SmolStr::new("attrs"))
        .into_iter()
        .next()
    {
        return Err(format!(
            "the sheet no longer balances ({issue}); it cannot be approved as-is"
        ));
    }
    doc.approved = ApprovalState::Approved {
        by: ctx.actor,
        at: ctx.w.now(),
    };
    store
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| fmt_errs(&es))?;
    on_approved(onboarding, ctx, target);
    Ok(())
}

// ======================================================================
// Vitals
// ======================================================================

/// The toy [`VitalsSource`]: the schema's declared vitals and the sheet
/// keyframe, both read through the authoritative Willpower overlay, plus the
/// `roll_state` key codec so a committed spend republishes its subject.
pub(crate) struct ToyVitals {
    cmd: Arc<ToyCmd>,
}

impl ToyVitals {
    fn vitals_of(&self, doc: &SheetDoc) -> Vec<Vital> {
        mush_ruleset::from_sheet(&self.cmd.schema, doc, &self.cmd.store())
    }
}

impl VitalsSource for ToyVitals {
    fn vitals(&self, ctx: &HookReadContext<'_>, subject: ObjId) -> Option<Vec<Vital>> {
        let mut doc = self.cmd.read(ctx.world(), subject)?;
        if let Ok(Some(state)) = ctx.action_state(&self.cmd.roll_state, &subject) {
            overlay(&mut doc, state);
        }
        Some(self.vitals_of(&doc))
    }

    fn vitals_in(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<Vec<Vital>> {
        let doc = self.cmd.read_ctx(ctx, subject)?;
        Some(self.vitals_of(&doc))
    }

    fn sheet_view(&self, ctx: &Ctx<'_>, subject: ObjId) -> Option<SheetView> {
        let doc = self.cmd.read_ctx(ctx, subject)?;
        Some(SheetView {
            target: subject,
            schema: self.cmd.schema.describe(),
            approved: serde_json::json!({
                "state": doc.approved.label(),
                "approved": doc.approved.is_approved(),
            }),
            data: doc.data.clone(),
            show_staff: false,
        })
    }

    fn state_subject(&self, family: &mush_core::StateFamilyId, key: &str) -> Option<ObjId> {
        authoritative::state_subject(family, key)
    }
}

/// The **overlay-aware** sheet query, on the `<id>.sheet_live` feed.
///
/// `register_sheet_commands` owns `<id>.sheet` and answers it from the STORED
/// document — it implements `query` only, so no typed-state lens ever reaches
/// it and a committed `roll_state` record is invisible there. A game ruleset
/// solves that by registering its own dispatching sheet query; the toy keeps
/// the generic surface and adds this second feed beside it, which answers the
/// same shape through [`ToyCmd::read_ctx`]. Ask `<id>.sheet_live` (or
/// `<id>.vitals`) whenever the question is "what does the subject actually
/// have right now".
pub(crate) struct ToySheetLive {
    cmd: Arc<ToyCmd>,
    feed: SmolStr,
}

impl mush_core::QueryHandler for ToySheetLive {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let context = HookReadContext::without_typed_state(w, self.cmd.name);
        self.query_with_context(&context, actor, params)
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let w = context.world();
        let target = match params.get("target") {
            None | Some(serde_json::Value::Null) => actor,
            Some(value) => {
                let spec = value
                    .as_str()
                    .ok_or_else(|| mush_proto::RichText::tag("error", "bad target"))?;
                let found = mush_core::match_object(w, actor, spec)
                    .map_err(|e| mush_proto::RichText::tag("error", e.to_string()))?;
                if found != actor && !perm::can_examine(w, actor, found) {
                    return Err(mush_proto::RichText::tag(
                        "error",
                        "You can't examine that sheet.",
                    ));
                }
                found
            }
        };
        let mut doc = self
            .cmd
            .read(w, target)
            .ok_or_else(|| mush_proto::RichText::tag("error", "That character has no sheet."))?;
        if let Ok(Some(state)) = context.action_state(&self.cmd.roll_state, &target) {
            overlay(&mut doc, state);
        }
        let show_staff = perm::has_role(w, actor, Role::Royalty);
        let data = if show_staff {
            doc.data.clone()
        } else {
            strip_staff_only(&self.cmd.schema, &doc.data)
        };
        let revision = w
            .module_state::<mush_ruleset::VitalsRevisions>()
            .and_then(|r| r.0.get(&target).copied())
            .unwrap_or(0);
        Ok(SheetView {
            target,
            schema: self.cmd.schema.describe(),
            approved: serde_json::json!({
                "state": doc.approved.label(),
                "approved": doc.approved.is_approved(),
            }),
            data,
            show_staff,
        }
        .query_keyframe(w, &self.feed, revision))
    }
}

/// `data` with every staff-only field removed (the player projection).
fn strip_staff_only(schema: &SheetSchema, data: &serde_json::Value) -> serde_json::Value {
    let mut out = data.clone();
    if let Some(map) = out.as_object_mut() {
        for (_, field) in schema.fields() {
            if field.staff_only {
                map.remove(field.id.as_str());
            }
        }
    }
    out
}

// ======================================================================
// Perception (concealment) and the gated narration channel
// ======================================================================

/// The read-only per-observer gate. `+toyhide on` sets the `hidden` sheet
/// flag; an observer perceives the hider only when their `awareness` is at
/// least the hider's `stealth`. A pure threshold — no dice, no cache, so a
/// seeded test's dice stream is untouched by who is in the room.
pub struct ToyPerception {
    schema: Arc<SheetSchema>,
}

impl ToyPerception {
    /// A hook over `schema` (the registering module's own).
    #[must_use]
    pub fn new(schema: Arc<SheetSchema>) -> ToyPerception {
        ToyPerception { schema }
    }

    /// The stored sheet. Reads never validate, so this is the read path.
    fn sheet(&self, ctx: &HookReadContext<'_>, who: ObjId) -> Option<SheetDoc> {
        SheetStore::new(Arc::clone(&self.schema)).get(ctx.world(), who)
    }
}

impl EventHook for ToyPerception {
    fn perceives(&self, ctx: &HookReadContext<'_>, observer: ObjId, subject: ObjId) -> Perceive {
        if observer == subject {
            return Perceive::Abstain;
        }
        let Some(hider) = self.sheet(ctx, subject) else {
            return Perceive::Abstain;
        };
        if hider.number("hidden").unwrap_or(0) < 1 {
            return Perceive::Abstain;
        }
        let stealth = hider.number("stealth").unwrap_or(0);
        let awareness = self
            .sheet(ctx, observer)
            .and_then(|d| d.number("awareness"))
            .unwrap_or(0);
        if awareness >= stealth {
            Perceive::Abstain
        } else {
            Perceive::Conceal
        }
    }

    /// The ruleset-agnostic notability signal: `subject` is concealed exactly
    /// when its `hidden` flag is set.
    fn is_concealed(&self, ctx: &HookReadContext<'_>, subject: ObjId) -> bool {
        self.sheet(ctx, subject)
            .is_some_and(|d| d.number("hidden").unwrap_or(0) >= 1)
    }
}

/// Who, in `actor`'s room, is on the gated sense channel: a Player or NPC
/// other than `actor` whose `awareness` reaches [`SUBTLE_AWARENESS`].
pub(crate) fn subtle_witnesses(cmd: &ToyCmd, w: &World, actor: ObjId) -> Vec<ObjId> {
    let Some(room) = w.get(actor).and_then(|o| o.location) else {
        return Vec::new();
    };
    let ids: Vec<ObjId> = w.contents_of(room).to_vec();
    ids.into_iter()
        .filter(|&o| o != actor)
        .filter(|&o| {
            matches!(
                w.get(o).map(|x| x.kind),
                Some(ObjKind::Player | ObjKind::Npc)
            )
        })
        .filter(|&o| {
            cmd.read(w, o)
                .and_then(|d| d.number("awareness"))
                .unwrap_or(0)
                >= SUBTLE_AWARENESS
        })
        .collect()
}

// ======================================================================
// Onboarding (gate / hook / spawn / lift)
// ======================================================================

/// Whether `verb` is on the onboarding allowlist — what a gated, unapproved
/// character may still run.
#[must_use]
pub fn is_onboarding_verb(verb: &str) -> bool {
    const ALLOWED: &[&str] = &[
        "look", "l", "say", "pose", "semipose", "think", "help", "+help", "who", "quit",
        "+chargen", "+stat", "+sheet",
    ];
    let v = verb.to_ascii_lowercase();
    ALLOWED.iter().any(|a| *a == v)
}

fn refusal() -> String {
    "You must finish creating your character and have it approved before you can do that. \
     Type +chargen to keep building; look, say, pose, and help still work while you wait."
        .to_string()
}

/// Whether the onboarding restriction applies to `actor` right now.
fn gated(cfg: &OnboardingConfig, cmd: &ToyCmd, w: &World, actor: ObjId) -> bool {
    cfg.chargen_required
        && w.get(actor).is_some_and(|o| o.kind == ObjKind::Player)
        && !perm::has_role(w, actor, Role::Builder)
        && !cmd
            .read(w, actor)
            .is_some_and(|d| matches!(d.approved, ApprovalState::Approved { .. }))
}

/// The dispatch-level gate.
pub(crate) struct OnboardingGate {
    cfg: OnboardingConfig,
    cmd: Arc<ToyCmd>,
}

impl CommandGate for OnboardingGate {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        actor: ObjId,
        verb: &str,
        line: &str,
    ) -> Result<(), String> {
        if !gated(&self.cfg, &self.cmd, w, actor) {
            return Ok(());
        }
        // Movement onto the live grid is the central block: an exit whose name
        // collides with an allowlisted verb must not slip past.
        if mush_core::commands::movement::find_exit(w, actor, line).is_some() {
            return Err(refusal());
        }
        if !is_onboarding_verb(verb) {
            return Err(refusal());
        }
        Ok(())
    }
}

/// The `Connected` hook: greet a gated character and point them at chargen.
pub(crate) struct OnboardingHook {
    cfg: OnboardingConfig,
    cmd: Arc<ToyCmd>,
}

impl EventHook for OnboardingHook {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if !gated(&self.cfg, &self.cmd, ctx.w, actor) {
            return;
        }
        if ctx.w.input_depth(actor) > 0 {
            return;
        }
        if let Some(ApprovalState::Pending(job)) = self.cmd.read(ctx.w, actor).map(|d| d.approved) {
            ctx.out.info(
                actor,
                format!(
                    "Your character is submitted and awaiting approval (job {job}). \
                     A storyteller will review it soon."
                ),
            );
            return;
        }
        ctx.out.info(
            actor,
            "Welcome! Let's finish creating your character. Type +chargen to begin.",
        );
    }
}

/// The `ObjCreated` observer: place a new unapproved Player in the chargen area.
pub(crate) struct OnboardingSpawn {
    cfg: OnboardingConfig,
}

impl WorldObserver for OnboardingSpawn {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        if !self.cfg.chargen_required {
            return;
        }
        let WorldEvent::ObjCreated { obj, kind, .. } = ev else {
            return;
        };
        if *kind != ObjKind::Player {
            return;
        }
        let dest = self.cfg.start_room_unapproved;
        if ctx.w.get(dest).is_none() {
            return;
        }
        if ctx.w.get(*obj).and_then(|o| o.location) == Some(dest) {
            return;
        }
        let _ = ctx.w.move_to(*obj, dest);
    }
}

/// Lift the gate on approval: move `target` to the live start room and
/// re-render it. A no-op when `chargen_required` is off.
pub(crate) fn on_approved(cfg: &OnboardingConfig, ctx: &mut Ctx<'_>, target: ObjId) {
    if !cfg.chargen_required {
        return;
    }
    let dest = cfg.start_room;
    let at_dest = ctx.w.get(target).and_then(|o| o.location) == Some(dest);
    let dest_exists = ctx.w.get(dest).is_some();
    if !dest_exists && !at_dest {
        tracing_warn_missing_start_room(target, dest);
    }
    let moved = dest_exists && !at_dest && ctx.w.move_to(target, dest).is_ok();
    ctx.out.info(
        target,
        "Your character has been approved — welcome to the game! You now have full access.",
    );
    // Render the room directly rather than re-dispatching `look`: a
    // re-dispatch from inside a handler carries no live typed-state lens.
    if moved && ctx.w.input_depth(target) == 0 {
        ctx.show_room(target, dest);
    }
}

/// The misconfiguration note, factored out so the hot path stays readable.
fn tracing_warn_missing_start_room(target: ObjId, dest: ObjId) {
    // `mush-testkit` carries no tracing dependency; the toy's start rooms are
    // fixture-owned, so the note goes to the test's stderr instead.
    eprintln!(
        "toy onboarding: start_room {dest} does not exist; approved character {target} \
         was not relocated out of the chargen area"
    );
}

// ======================================================================
// The `toy_roll` NPC ability
// ======================================================================

/// The `toy_roll` tool manifest exposed to an LLM agent.
#[must_use]
pub fn toy_roll_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: SmolStr::new(TOY_ROLL_ABILITY),
        description: "Roll one of YOUR OWN dice pools (traits recorded on your sheet) against a \
             difficulty. You may only name traits you actually have; the engine rolls real dice \
             and returns the outcome — you never choose the result."
            .to_string(),
        params_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["pool_expr"],
            "properties": {
                "pool_expr": { "type": "string" },
                "difficulty": { "type": "integer", "minimum": 2, "maximum": 10, "default": 6 },
                "willpower": { "type": "boolean", "default": false },
                "modifier": { "type": "integer", "minimum": -3, "maximum": 3, "default": 0 }
            }
        }),
    }
}

/// The `toy_roll` input the model may send.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ToyRollInput {
    /// The trait pool (`"strength+brawl"`).
    pub pool_expr: String,
    /// The target number (2..=10, default 6).
    pub difficulty: Option<u8>,
    /// Whether the roll spends Willpower — the flag that re-routes it onto the
    /// authoritative coordinator (`projected_ability`).
    pub willpower: bool,
    /// A small situational modifier, clamped to ±3.
    pub modifier: i32,
}

/// Roll the NPC's own pool on the shared seeded RNG and narrate it to the room.
fn toy_roll(
    schema: &Arc<SheetSchema>,
    w: &mut World,
    out: &mut Outbox,
    npc: ObjId,
    input: serde_json::Value,
) -> Result<AbilityOutcome, AbilityError> {
    let args: ToyRollInput =
        serde_json::from_value(input).map_err(|e| AbilityError::Invalid(e.to_string()))?;
    let store = SheetStore::new(Arc::clone(schema));
    let doc = store
        .get(w, npc)
        .ok_or_else(|| AbilityError::Invalid("that NPC has no sheet".to_string()))?;
    let resolved = resolve_pool(schema, &doc, &args.pool_expr).map_err(AbilityError::Invalid)?;
    let modifier = args.modifier.clamp(-3, 3);
    let pool = i32::try_from(resolved.total)
        .unwrap_or(i32::MAX)
        .saturating_add(modifier)
        .max(1);
    let difficulty = args.difficulty.unwrap_or(6).clamp(2, 10);
    let opts = Owod20Opts {
        difficulty,
        specialty: false,
        willpower: false,
        extra_successes: 0,
        again_tens_double: false,
    };
    let outcome = {
        let state = w
            .module_state_mut::<ToyState>()
            .ok_or_else(|| AbilityError::Invalid("the toy ruleset is not loaded".to_string()))?;
        let mut rctx = RollCtx::new(state.rng());
        Owod20Pool.roll(pool, &opts, &mut rctx)
    };
    let name = w.get(npc).map_or_else(
        || npc.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    );
    narrate_roll(w, out, npc, &name, &resolved, &outcome);
    Ok(ability_outcome(
        &args, pool, difficulty, &resolved, &outcome,
    ))
}

/// The classic one-line roll record, to the room.
fn narrate_roll(
    w: &World,
    out: &mut Outbox,
    who: ObjId,
    name: &str,
    resolved: &ResolvedPool,
    outcome: &Owod20Outcome,
) {
    let label = resolved.label();
    let narration = roll_narration(
        &RollSide {
            name,
            expr: &label,
            dice: &outcome.dice_text(),
            result: &outcome.result_text(),
        },
        &[format!("diff {}", outcome.difficulty)],
    );
    let event = mush_proto::OutputEvent::Narrative {
        kind: mush_proto::NarrativeKind::Emit,
        actor: Some(mush_core::obj_ref(w, who)),
        text: narration,
    };
    match w.get(who).and_then(|o| o.location) {
        Some(room) => out.to_room_from(room, Vec::new(), who, event),
        None => out.to_actor(who, event),
    }
}

/// The structured outcome handed back to the NPC layer — the die **faces** are
/// in `data.dice`, so a test can prove the ruleset, not the model, sized the
/// pool from the stored sheet.
fn ability_outcome(
    args: &ToyRollInput,
    pool: i32,
    difficulty: u8,
    resolved: &ResolvedPool,
    outcome: &Owod20Outcome,
) -> AbilityOutcome {
    AbilityOutcome {
        ok: outcome.successes() > 0,
        summary: format!("{} ({pool} dice, diff {difficulty})", outcome.summary()),
        data: serde_json::json!({
            "pool_expr": args.pool_expr,
            "label": resolved.label(),
            "fields": resolved.fields,
            "pool": pool,
            "difficulty": difficulty,
            "dice": outcome.dice.iter().map(|d| d.face).collect::<Vec<u8>>(),
            "successes": outcome.successes(),
            "outcome": outcome,
        }),
    }
}

// ======================================================================
// Commands
// ======================================================================

mod commands {
    use super::{
        Arc, ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, Registrar, SmolStr, ToyChargen,
        ToyCmd, WorldEvent, fmt_errs, subtle_witnesses,
    };
    use mush_core::{ObjKind, match_object};
    use mush_jobs::{JobPayload, OpenJob, open_job};
    use mush_ruleset::chargen::ChargenFlow;
    use mush_ruleset::{ApprovalState, SheetDoc};

    pub(super) fn register(reg: &mut Registrar<'_>, cmd: Arc<ToyCmd>) -> Result<(), ModError> {
        reg.command(
            CommandSpec {
                name: "+chargen".into(),
                switches: vec!["status".into(), "submit".into()],
                shape: ArgShape::Raw,
                help: "+chargen[/status|/submit]  --  Toy character generation: spend 5 \
                       physical points (cap 4 each) with +stat, then submit for approval."
                    .into(),
                ..CommandSpec::default()
            },
            {
                let cmd = Arc::clone(&cmd);
                Arc::new(move |ctx: &mut Ctx<'_>| chargen_cmd(&cmd, ctx))
            },
        )?;
        reg.command(
            CommandSpec {
                name: "+stat".into(),
                shape: ArgShape::EqPair,
                help: "+stat <field>=<n>  --  Set one numeric field on YOUR OWN toy sheet \
                       while it is unapproved."
                    .into(),
                ..CommandSpec::default()
            },
            {
                let cmd = Arc::clone(&cmd);
                Arc::new(move |ctx: &mut Ctx<'_>| stat_cmd(&cmd, ctx))
            },
        )?;
        reg.command(
            CommandSpec {
                name: "+toyhide".into(),
                shape: ArgShape::Raw,
                help: "+toyhide on|off  --  Set or clear your concealment flag. An onlooker \
                       perceives you only if their Awareness reaches your Stealth."
                    .into(),
                ..CommandSpec::default()
            },
            {
                let cmd = Arc::clone(&cmd);
                Arc::new(move |ctx: &mut Ctx<'_>| hide_cmd(&cmd, ctx))
            },
        )?;
        reg.command(
            CommandSpec {
                name: "+toysubtle".into(),
                shape: ArgShape::EqPair,
                help: "+toysubtle <target>=<text>  --  A subtle act on a gated sense channel: \
                       only onlookers with Awareness 3+ perceive it at all."
                    .into(),
                ..CommandSpec::default()
            },
            {
                let cmd = Arc::clone(&cmd);
                Arc::new(move |ctx: &mut Ctx<'_>| subtle_cmd(&cmd, ctx))
            },
        )?;
        Ok(())
    }

    /// `+chargen` / `+chargen/status` / `+chargen/submit`.
    fn chargen_cmd(cmd: &ToyCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        if ctx.switches.has("submit") {
            return submit(cmd, ctx);
        }
        let actor = ctx.actor;
        let doc = cmd
            .read(ctx.w, actor)
            .unwrap_or_else(|| SheetDoc::new_default(&cmd.schema));
        let flow = ToyChargen::for_schema(Arc::clone(&cmd.schema));
        let issues = flow.validate_stage(&doc, &SmolStr::new("attrs"));
        if issues.is_empty() {
            ctx.out.info(
                actor,
                "Toy chargen: your 5 physical points are spent. Type +chargen/submit.",
            );
        } else {
            let list: Vec<String> = issues
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            ctx.out.info(
                actor,
                format!(
                    "Toy chargen: spend 5 physical points (cap 4 each) with +stat.\n  {}",
                    list.join("\n  ")
                ),
            );
        }
        Ok(())
    }

    /// `+chargen/submit` — finalize, file an approval job, flip to Pending.
    fn submit(cmd: &ToyCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        let actor = ctx.actor;
        let mut doc = cmd
            .read(ctx.w, actor)
            .ok_or_else(|| CmdError::msg("You have no sheet to submit."))?;
        match doc.approved {
            ApprovalState::Pending(job) => {
                return Err(CmdError::msg(format!(
                    "Your sheet is already submitted (job {job}); wait for a storyteller."
                )));
            }
            ApprovalState::Approved { .. } => {
                return Err(CmdError::msg("Your sheet is already approved."));
            }
            ApprovalState::Unapproved => {}
        }
        let flow = ToyChargen::for_schema(Arc::clone(&cmd.schema));
        flow.finalize(&mut doc)
            .map_err(|i| CmdError::msg(format!("Cannot submit: {i}")))?;
        // Validate the exact finalized document BEFORE allocating a job, so a
        // refused submit never commits an orphan.
        mush_ruleset::validate(&cmd.schema, &doc.data)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;

        let name = cmd.name;
        let who = ctx
            .w
            .get(actor)
            .map_or_else(|| actor.to_string(), |o| o.name.to_string());
        let job = open_job(
            ctx,
            OpenJob {
                queue: SmolStr::new(format!("{name}-chargen")),
                title: format!("Chargen approval: {who}"),
                opener: actor,
                text: Some("Character submitted for approval.".to_string()),
                payload: Some(JobPayload {
                    handler: SmolStr::new(format!("{name}.chargen")),
                    data: serde_json::json!({ "target": actor.0 }),
                }),
                tags: vec!["chargen".into()],
            },
        )
        .map_err(CmdError::msg)?;

        doc.approved = ApprovalState::Pending(job);
        cmd.store()
            .put_in(actor, &doc, actor, ctx)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
        ctx.out.info(
            actor,
            format!("Submitted for approval as job {job}. A storyteller will review it."),
        );
        Ok(())
    }

    /// `+stat <field>=<n>` on the actor's own, unapproved sheet.
    fn stat_cmd(cmd: &ToyCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        let actor = ctx.actor;
        let field = ctx.args.lhs().trim().to_ascii_lowercase();
        let raw = ctx
            .args
            .rhs()
            .ok_or_else(|| CmdError::msg("Usage: +stat <field>=<n>"))?;
        let value: i64 = raw
            .trim()
            .parse()
            .map_err(|_| CmdError::msg("That is not a number."))?;
        let def = cmd
            .schema
            .field(&field)
            .ok_or_else(|| CmdError::msg(format!("No such field: {field}.")))?;
        if def.staff_only {
            return Err(CmdError::msg("Permission denied."));
        }
        let mut doc = cmd
            .read(ctx.w, actor)
            .unwrap_or_else(|| SheetDoc::new_default(&cmd.schema));
        if matches!(doc.approved, ApprovalState::Approved { .. }) {
            return Err(CmdError::msg(
                "Your sheet is approved; ask a storyteller to change it.",
            ));
        }
        doc.set_number(&field, value);
        cmd.store()
            .put_in(actor, &doc, actor, ctx)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
        ctx.out.info(actor, format!("{field} = {value}."));
        Ok(())
    }

    /// `+toyhide on|off`.
    fn hide_cmd(cmd: &ToyCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        let actor = ctx.actor;
        let on = match ctx.args.raw().trim().to_ascii_lowercase().as_str() {
            "on" | "" => true,
            "off" => false,
            _ => return Err(CmdError::msg("Usage: +toyhide on|off")),
        };
        let mut doc = cmd
            .read(ctx.w, actor)
            .ok_or_else(|| CmdError::msg("You have no sheet to hide behind."))?;
        doc.set_number("hidden", i64::from(on));
        cmd.store()
            .put_in(actor, &doc, actor, ctx)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
        ctx.out.info(
            actor,
            if on {
                "You fade out of sight."
            } else {
                "You step back into view."
            },
        );
        Ok(())
    }

    /// `+toysubtle <target>=<text>` — a narration on the gated sense channel.
    fn subtle_cmd(cmd: &ToyCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        let actor = ctx.actor;
        let spec = ctx.args.lhs().trim().to_string();
        let text = ctx
            .args
            .rhs()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| CmdError::msg("Usage: +toysubtle <target>=<text>"))?
            .to_string();
        if spec.is_empty() {
            return Err(CmdError::msg("Usage: +toysubtle <target>=<text>"));
        }
        let target = match_object(ctx.w, actor, &spec).map_err(CmdError::from)?;
        let actor_name = display_name(ctx.w, actor);
        let target_name = display_name(ctx.w, target);

        // (1) The actor always perceives their own act, with the raw detail.
        ctx.out.sys_to_actor(
            actor,
            cmd.name,
            format!("You work subtly on {target_name}: {text}"),
        );

        // (2)+(3) Silent per-observer projection: only the gated channel's
        // listeners receive anything at all. A non-listener gets no event.
        let witnessed = format!("{actor_name} bends another's will.");
        let winners = subtle_witnesses(cmd, ctx.w, actor);
        for &who in &winners {
            ctx.out.sys_to_actor(who, cmd.name, witnessed.clone());
        }

        // (4) Announce the director-resolved NPC subset so a witnessing NPC
        // informant files intel, with NO perception re-gate.
        let npcs: Vec<ObjId> = winners
            .into_iter()
            .filter(|&o| matches!(ctx.w.get(o).map(|x| x.kind), Some(ObjKind::Npc)))
            .collect();
        if let Some(room) = ctx.w.get(actor).and_then(|o| o.location) {
            ctx.emit_world_event(&WorldEvent::PowerUsed {
                who: actor,
                room,
                witnesses: npcs,
                detail: format!("used a subtle power on {target_name}"),
            });
        }
        Ok(())
    }

    fn display_name(w: &mush_core::World, who: ObjId) -> String {
        w.get(who).map_or_else(
            || who.to_string(),
            |o| mush_core::output::primary_name(&o.name).to_string(),
        )
    }
}
