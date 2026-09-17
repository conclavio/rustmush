//! **The one game sheet** (doc 16 §7; 16b cut R-b, R4; 16a Q35 A).
//!
//! `ruleset-wod` and `ruleset-shadowrun` between them were about 20k lines of
//! sheet: eleven splat schemas, two dice mechanics, XP, health, damage, a
//! wizard, specialties, priorities. This module is what the 2350 game keeps of
//! that shape — **one** schema, **one** mechanic, **one** approval queue — built
//! on exactly the platform half [`mush_ruleset`] already provides:
//! [`SheetSchema`](mush_ruleset::SheetSchema), [`SheetDoc`],
//! [`SheetStore`], the [`ChargenFlow`] budget accountant and
//! [`register_sheet_commands`](mush_ruleset::register_sheet_commands).
//!
//! | File | What it is |
//! |---|---|
//! | [`schema`] | the three sections and eight fields, and nothing else |
//! | [`chargen`] | three stages over one 6-point pool, `+chargen` and `+stat` |
//! | [`roll`] | `+roll <expr>` over [`mush_dice`]'s expression evaluator |
//! | [`agent`] | the single NPC ability, `check`, and the grounding profile |
//! | [`onboarding`] | the `chargen_required` gate, ported from the WoD crate |
//!
//! # The two names, and why they differ
//!
//! The sheet's **ruleset id** is [`schema::RULESET`] — `"tacsim"` — because that
//! is what names the storage and the wire: [`SheetStore::attr_name`] is
//! `sheet:<schema.ruleset>` and `register_sheet_commands` reserves the panel
//! feed `<schema.ruleset>.sheet`. Neither reads the module's name.
//!
//! The sheet **module** is [`NS`] — `"pilot"` — because a
//! [`GameModule`]'s name *is* its persistence namespace and no two modules in one
//! composition may share one, and `"tacsim"` is taken: [`crate::TacsimRuleset`],
//! the E7 packs-and-seed layer, is named after [`crate::NS`]. So the sheet takes
//! a second name, and `"pilot"` rather than `"tacsim_sheet"` because that name is
//! also the job-queue prefix ([`chargen::CHARGEN_QUEUE`]) and the payload-handler
//! prefix ([`chargen::CHARGEN_HANDLER`]) — `pilot-chargen` and `pilot.chargen`
//! read like every other namespace in the composition, and `tacsim_sheet-chargen`
//! does not. [`Ruleset::id`] defaults to the module name and is read only by the
//! default `migrate_sheet` refusal, so nothing pairs the two strings.
//!
//! # No vitals, no second authority
//!
//! [`register_vitals`](mush_ruleset::register_vitals) is not called and no field
//! declares a [`VitalRole`](mush_proto::VitalRole): doc 16 §3.3 makes M32's
//! `TacticalUnit` cells the only damage authority, and a health track here would
//! be a second one for the same hit. See [`schema`] for the rest of the
//! deliberate absences.

pub mod agent;
pub mod chargen;
pub mod onboarding;
pub mod roll;
pub mod schema;

use std::sync::Arc;

use mush_core::{
    GameModule, HookPoint, JobQueueSpec, ModError, ModuleBootData, ModuleCtx, ObjId,
    OnboardingConfig, Outbox, Registrar, Role, World, WorldEventKind,
};
use mush_ruleset::chargen::ChargenFlow;
use mush_ruleset::schema::SheetSchema;
use mush_ruleset::{
    AbilityError, AbilityOutcome, AgentCapabilityProfile, Ruleset, SheetDoc, SheetStore,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

pub use crate::sheet::chargen::TacsimChargen;

/// The sheet module's name — and so its persistence namespace, its job-queue
/// prefix and its payload-handler prefix. Distinct from the sheet's **ruleset
/// id** ([`schema::RULESET`]); the module docs say why.
pub const NS: &str = "pilot";

/// The three pilot ratings, read off a sheet and clamped into range — the one
/// seam doc 16 §3.5's resolution mechanic has onto the character sheet.
///
/// Every stock rating is 1 ([`schema::RATING_MIN`]), which is what
/// [`PilotProfile::default`] answers for a character with no sheet at all: an
/// unsheeted NPC in a tank rolls like a green pilot rather than refusing to
/// resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PilotProfile {
    /// Gunnery, 1–5.
    pub gunnery: u8,
    /// Piloting, 1–5.
    pub piloting: u8,
    /// Sensors, 1–5.
    pub sensors: u8,
}

impl Default for PilotProfile {
    fn default() -> PilotProfile {
        PilotProfile {
            gunnery: schema::RATING_MIN,
            piloting: schema::RATING_MIN,
            sensors: schema::RATING_MIN,
        }
    }
}

impl PilotProfile {
    /// Read a profile off a sheet document. A missing, non-numeric or
    /// out-of-range rating clamps into `1..=5` rather than refusing: the
    /// resolution mechanic runs inside an arena pulse, where there is nobody to
    /// refuse to.
    #[must_use]
    pub fn from_sheet(doc: &SheetDoc) -> PilotProfile {
        let read = |field: &str| -> u8 {
            doc.number(field).map_or(schema::RATING_MIN, |n| {
                n.clamp(i64::from(schema::RATING_MIN), i64::from(schema::RATING_MAX)) as u8
            })
        };
        PilotProfile {
            gunnery: read(schema::FIELD_GUNNERY),
            piloting: read(schema::FIELD_PILOTING),
            sensors: read(schema::FIELD_SENSORS),
        }
    }

    /// One rating by field id (case-insensitive), or `None` for anything that
    /// is not one of [`schema::RATINGS`].
    #[must_use]
    pub fn rating(&self, field: &str) -> Option<u8> {
        if field.eq_ignore_ascii_case(schema::FIELD_GUNNERY) {
            Some(self.gunnery)
        } else if field.eq_ignore_ascii_case(schema::FIELD_PILOTING) {
            Some(self.piloting)
        } else if field.eq_ignore_ascii_case(schema::FIELD_SENSORS) {
            Some(self.sensors)
        } else {
            None
        }
    }
}

/// The sheet module's state slot: the dice RNG `+roll` and the `check` ability
/// draw from.
///
/// Not persisted and never rehydrated — a re-seed on boot is the whole point
/// (`ruleset-wod`'s slot said the same). One slot, one field: nothing else about
/// this sheet is transient.
#[derive(Clone, Debug)]
pub struct PilotSheetState {
    rng: ChaCha8Rng,
}

impl PilotSheetState {
    /// A slot seeded from the OS (production).
    #[must_use]
    pub fn from_os() -> PilotSheetState {
        PilotSheetState {
            rng: ChaCha8Rng::from_os_rng(),
        }
    }

    /// A slot with a fixed seed (tests, and `--seed` fixtures).
    #[must_use]
    pub fn seeded(seed: u64) -> PilotSheetState {
        PilotSheetState {
            rng: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// The RNG, for the roll paths.
    pub fn rng(&mut self) -> &mut ChaCha8Rng {
        &mut self.rng
    }
}

impl Default for PilotSheetState {
    fn default() -> PilotSheetState {
        PilotSheetState::from_os()
    }
}

/// The game's character-sheet ruleset: one schema, one chargen flow, one
/// approval queue, one `+roll`, one NPC ability.
pub struct PilotRuleset {
    schema: Arc<SheetSchema>,
    chargen: TacsimChargen,
    seed: Option<u64>,
    onboarding: OnboardingConfig,
    economy: crate::economy::GameEconomyConfig,
}

impl PilotRuleset {
    /// The ruleset with an OS-seeded RNG and onboarding off.
    #[must_use]
    pub fn new() -> PilotRuleset {
        let schema = Arc::new(schema::sheet_schema());
        PilotRuleset {
            chargen: TacsimChargen::new(Arc::clone(&schema)),
            schema,
            seed: None,
            onboarding: OnboardingConfig::default(),
            economy: crate::economy::GameEconomyConfig::default(),
        }
    }

    /// The ruleset with a fixed dice seed — deterministic `+roll` and `check`.
    #[must_use]
    pub fn with_seed(seed: u64) -> PilotRuleset {
        PilotRuleset {
            seed: Some(seed),
            ..PilotRuleset::new()
        }
    }

    /// Set the onboarding configuration (the `chargen_required` gate and the two
    /// start rooms). Off by default; the host builds this from `[onboarding]`.
    #[must_use]
    pub fn onboarding(mut self, onboarding: OnboardingConfig) -> PilotRuleset {
        self.onboarding = onboarding;
        self
    }

    /// Set the game's economy wiring (`[tacsim.economy]`): the settlement currency
    /// this module registers at boot. The host builds it from config.
    #[must_use]
    pub fn economy(mut self, economy: crate::economy::GameEconomyConfig) -> PilotRuleset {
        self.economy = economy;
        self
    }

    /// The shared schema handle.
    #[must_use]
    pub fn schema(&self) -> Arc<SheetSchema> {
        Arc::clone(&self.schema)
    }

    /// A validating store over the schema — the one sanctioned sheet writer.
    #[must_use]
    pub fn store(&self) -> SheetStore {
        SheetStore::new(Arc::clone(&self.schema))
    }
}

impl Default for PilotRuleset {
    fn default() -> PilotRuleset {
        PilotRuleset::new()
    }
}

impl GameModule for PilotRuleset {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // K8.5: every command below is a bounded whole-document post-image on the
        // compatibility lane, exactly as both deleted rulesets' were. It must come
        // before the first `reg.command`.
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        // WS03-A1, both directions of the jobs delegation. Out: `+chargen/submit`
        // files an approval job through `mush_jobs::open_job`, whose write carries
        // the caller's identity. In: `+job/approve` dispatches the payload handler
        // registered in `on_world_loaded` under the JOBS module's identity, and it
        // writes this module's namespace.
        reg.persist_delegation(
            NS,
            "jobs",
            "sheet::chargen files the one approval job through mush_jobs::open_job, \
             whose write carries the caller's identity",
        )?;
        reg.persist_delegation(
            "jobs",
            NS,
            "`+job/approve` dispatches this module's chargen payload handler under the \
             jobs module's identity, and it writes pilot-namespace documents",
        )?;
        reg.state(match self.seed {
            Some(seed) => PilotSheetState::seeded(seed),
            None => PilotSheetState::from_os(),
        });
        reg.job_queue(JobQueueSpec {
            name: chargen::CHARGEN_QUEUE.into(),
            title: "Pilot approvals".into(),
            min_role: Role::Royalty,
        })?;
        // The platform's own `+sheet`, `@sheet`, `@sheet/set`, the `tacsim.sheet`
        // query handler and its panel feed. `None`: the schema is at version 1 and
        // there is nothing to migrate from.
        mush_ruleset::register_sheet_commands(reg, Arc::clone(&self.schema), None)?;
        chargen::register(reg, Arc::clone(&self.schema))?;
        roll::register(reg)?;
        // Onboarding: all three seams are inert unless `chargen_required` is on.
        reg.command_gate(Arc::new(onboarding::OnboardingGate::new(
            self.onboarding.clone(),
            Arc::clone(&self.schema),
        )));
        reg.hook(
            HookPoint::Connected,
            Arc::new(onboarding::OnboardingHook::new(
                self.onboarding.clone(),
                Arc::clone(&self.schema),
            )),
        );
        reg.subscribe(
            WorldEventKind::ObjCreated,
            Arc::new(onboarding::OnboardingSpawn::new(self.onboarding.clone())),
        );
        Ok(())
    }

    /// Register the one job payload handler.
    ///
    /// It lands here rather than in [`GameModule::register`] because
    /// [`mush_jobs::register_payload_handler`] writes the jobs module's **state
    /// slot**, which exists only once the engine is built — the same reason
    /// `ruleset-wod` registered its two here. The sheet stores nothing of its own
    /// at boot: a sheet rides the object attribute write-through (treaty ruling
    /// 7), so `data` is ignored.
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        let schema = Arc::clone(&self.schema);
        let onboarding = self.onboarding.clone();
        mush_jobs::register_payload_handler(
            ctx.w,
            chargen::CHARGEN_HANDLER,
            Arc::new(move |ctx, job| chargen::chargen_approval(&schema, &onboarding, ctx, job)),
        )
        .map_err(ModError::msg)?;
        // 16b cut R-b: the settlement currency (doc 11 §2, `tacsim.credit`), create-only,
        // exactly where the WoD ruleset registered `wod.usd`. This module is composed
        // after the economy module, so its store is loaded by the time this runs; a
        // refusal is logged, not fatal (`@instrument/register` is the repair path).
        // 16b cut C.2 (C4): and the ten commodity families of doc 10 S4A beside
        // it, on the same rule, from `content/resources.json`.
        crate::economy::ensure_instruments(ctx, &self.economy);
        Ok(())
    }
}

impl Ruleset for PilotRuleset {
    fn display_name(&self) -> &str {
        "TacSim"
    }

    fn sheet_schema(&self) -> &SheetSchema {
        &self.schema
    }

    fn chargen(&self) -> Option<&dyn ChargenFlow> {
        Some(&self.chargen)
    }

    fn agent_profile(&self, sheet: &SheetDoc) -> AgentCapabilityProfile {
        agent::agent_profile(sheet)
    }

    /// Exactly one ability, [`agent::CHECK_ABILITY`]. Everything else is
    /// [`AbilityError::Unknown`] — R7 deletes `wod_roll` and `sr_roll` and this
    /// is what replaces both.
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

    /// Give `npc` a default sheet through the validated write path, so an NPC has
    /// real ratings for `check` to read. `template` is ignored: there is one
    /// sheet. An existing sheet is left alone rather than reset — `@agent/sheet`
    /// is a bootstrap, not an eraser.
    fn init_sheet(
        &self,
        w: &mut World,
        npc: ObjId,
        _template: &str,
        by: ObjId,
    ) -> Result<String, String> {
        let store = self.store();
        if store.get(w, npc).is_some() {
            return Ok("The NPC already has a sheet.".to_string());
        }
        let doc = SheetDoc::new_default(&self.schema);
        store.put_by(w, npc, &doc, by).map_err(|errors| {
            errors
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        Ok("Gave the NPC a pilot sheet.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_sheet_reads_as_a_green_pilot() {
        let doc = SheetDoc::new_default(&schema::sheet_schema());
        assert_eq!(PilotProfile::from_sheet(&doc), PilotProfile::default());
    }

    #[test]
    fn ratings_clamp_into_range_rather_than_refusing() {
        let mut doc = SheetDoc::new_default(&schema::sheet_schema());
        doc.set(schema::FIELD_GUNNERY, serde_json::json!(99));
        doc.set(schema::FIELD_PILOTING, serde_json::json!(-4));
        doc.set(schema::FIELD_SENSORS, serde_json::json!("four"));
        let profile = PilotProfile::from_sheet(&doc);
        assert_eq!(profile.gunnery, schema::RATING_MAX);
        assert_eq!(profile.piloting, schema::RATING_MIN);
        assert_eq!(profile.sensors, schema::RATING_MIN);
    }

    #[test]
    fn rating_resolves_only_the_three() {
        let profile = PilotProfile {
            gunnery: 4,
            piloting: 3,
            sensors: 2,
        };
        assert_eq!(profile.rating("GUNNERY"), Some(4));
        assert_eq!(profile.rating("piloting"), Some(3));
        assert_eq!(profile.rating("sensors"), Some(2));
        assert_eq!(profile.rating(schema::FIELD_REPUTATION), None);
    }

    /// The two names are deliberately different, and the packs module's name is
    /// the reason. A future rename that collapsed them would make the composition
    /// refuse at boot; this says so at test time instead.
    #[test]
    fn the_module_name_does_not_collide_with_the_packs_module() {
        assert_ne!(NS, crate::NS);
        assert_eq!(schema::RULESET, crate::NS);
        assert_eq!(PilotRuleset::new().id().as_str(), NS);
    }
}
