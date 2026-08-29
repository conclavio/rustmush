//! NPC agent grounding for oWoD20 (treaty ruling 6; design 03 §6): render an
//! NPC's real character sheet into an [`AgentCapabilityProfile`] for the LLM
//! system prompt, and execute the validated **`wod_roll`** ability the agent
//! may request.
//!
//! ## Security posture (design 03 §7)
//!
//! The action allowlist — not the prompt — is the security boundary. The only
//! ability exposed here, `wod_roll`, can touch **only the NPC's own sheet**:
//! it reads that sheet, resolves the pool against the traits actually recorded
//! on it ([`resolve_sheet_pool`] rejects raw dice / named bonuses, so a fully
//! jailbroken model cannot manufacture a pool of 9 from a Brawl of 2), spends
//! at most the NPC's own temporary Willpower, and rolls through the **same
//! server RNG and roll log** players use. The model only *requests* the roll;
//! the deterministic engine computes the outcome, which is emitted to the room
//! and the `wod.roll` panel like any player roll — it cannot be fabricated. The
//! structured outcome is returned so `mush-npc`'s validator can post-check the
//! model's in-fiction narration against the real success/failure polarity.

use mush_core::{Audience, ObjId, Outbox, World, obj_ref, output::primary_name};
use mush_dice::RollCtx;
use mush_dice::owod20::{Owod20Opts, Owod20Outcome, Owod20Pool, RollResult};
use mush_dice::pool::PoolMechanic;
use mush_proto::{NarrativeKind, OutputEvent, PanelFrameKind, RichText};
use mush_ruleset::schema::{FieldType, SheetSchema};
use mush_ruleset::{
    AbilityError, AbilityOutcome, AgentCapabilityProfile, AgentToolSpec, SheetDoc, SheetStore,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use smol_str::SmolStr;

use crate::dice::{ResolvedPool, resolve_sheet_pool};
use crate::health::HealthTrack;
use crate::roll::has_any_specialty;
use crate::schema::{ATTRIBUTES, KNOWLEDGES, SKILLS, TALENTS};
use crate::splats::Dispatch;
use crate::state::{RollLogEntry, WodState};

/// The one ability name the WoD NPC agent may invoke.
pub const WOD_ROLL_ABILITY: &str = "wod_roll";

/// The `wod.roll` panel feed id (matches the `+roll` command's feed).
const ROLL_FEED: &str = "wod.roll";

/// The hard bound on the `modifier` argument: an agent may nudge its pool by a
/// small situational amount, never inflate it.
const MODIFIER_BOUND: i32 = 3;

// ======================================================================
// Ability manifest + invocation
// ======================================================================

/// The stable, documented `wod_roll` tool manifest exposed to the LLM. Keep
/// this schema stable — `mush-npc` surfaces it to the provider.
#[must_use]
pub fn wod_roll_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: SmolStr::new(WOD_ROLL_ABILITY),
        description: "Roll one of YOUR OWN Storyteller-System dice pools (an attribute + an \
             ability recorded on your sheet) against a difficulty. You may only name traits you \
             actually have; the engine rolls real dice and returns the outcome — you never choose \
             the result. Use it when the story calls for a test of your character's capability."
            .to_string(),
        params_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["pool_expr"],
            "properties": {
                "pool_expr": {
                    "type": "string",
                    "description": "The trait pool, e.g. \"strength+brawl\" or \
                        \"dexterity+firearms\". Only attributes/abilities/powers on your own \
                        sheet are allowed; raw numbers and made-up bonuses are refused."
                },
                "difficulty": {
                    "type": "integer", "minimum": 3, "maximum": 9, "default": 6,
                    "description": "Target number, default 6 (higher is harder)."
                },
                "specialty": {
                    "type": "boolean", "default": false,
                    "description": "Apply a specialty (10s count twice). Only takes effect if you \
                        have a specialty recorded on one of the pool's abilities."
                },
                "willpower": {
                    "type": "boolean", "default": false,
                    "description": "Spend one temporary Willpower for +1 automatic success (no \
                        botch). Only takes effect if you have a temporary point to spend."
                },
                "modifier": {
                    "type": "integer", "minimum": -3, "maximum": 3, "default": 0,
                    "description": "A small situational dice modifier (-3..+3)."
                }
            }
        }),
    }
}

/// Dispatch a named NPC ability. Runs in the world task with the NPC's world +
/// outbox (see [`crate::WodRuleset::invoke_ability`]).
///
/// # Errors
///
/// [`AbilityError::Unknown`] for an unrecognized ability; [`AbilityError::Invalid`]
/// for bad input or an over-claimed pool.
pub(crate) fn invoke(
    dispatch: &Dispatch,
    w: &mut World,
    out: &mut Outbox,
    npc: ObjId,
    name: &str,
    input: Value,
) -> Result<AbilityOutcome, AbilityError> {
    match name {
        WOD_ROLL_ABILITY => wod_roll(dispatch, w, out, npc, input),
        other => Err(AbilityError::Unknown(other.to_string())),
    }
}

/// The `wod_roll` argument shape (the stable contract with `mush-npc`).
#[derive(Debug, Deserialize)]
struct WodRollInput {
    /// The trait pool expression (`"strength+brawl"`); `pool` accepted as an
    /// alias for lenient model output.
    #[serde(alias = "pool")]
    pool_expr: String,
    /// Target number (default 6, clamped to 3..=9).
    #[serde(default)]
    difficulty: Option<u8>,
    /// Request a specialty (only honored if one is recorded).
    #[serde(default)]
    specialty: bool,
    /// Request a Willpower spend (only honored if a temporary point exists).
    #[serde(default)]
    willpower: bool,
    /// A small situational modifier, clamped to ±[`MODIFIER_BOUND`].
    #[serde(default)]
    modifier: i32,
}

#[allow(clippy::too_many_lines)]
fn wod_roll(
    dispatch: &Dispatch,
    w: &mut World,
    out: &mut Outbox,
    npc: ObjId,
    input: Value,
) -> Result<AbilityOutcome, AbilityError> {
    let args: WodRollInput = serde_json::from_value(input)
        .map_err(|e| AbilityError::Invalid(format!("wod_roll needs a pool_expr ({e})")))?;

    // The NPC's own sheet (schema-agnostic read), then its active schema.
    let mut doc = SheetStore::new(dispatch.schemas.default_schema())
        .get(w, npc)
        .ok_or_else(|| AbilityError::Invalid("this character has no sheet to roll".to_string()))?;
    let schema = dispatch.schemas.schema_for_doc(&doc);

    // Pool: only real sheet traits (the over-claim fence).
    let resolved =
        resolve_sheet_pool(&schema, &doc, &args.pool_expr).map_err(AbilityError::Invalid)?;

    // Wound penalty + incapacitation from the NPC's own Health track.
    let track = HealthTrack::from_marks(&doc.track("health").unwrap_or_default());
    let incap = track.is_incapacitated();
    let wound = i32::from(SheetStore::new(schema.clone()).wound_penalty(&doc, "health"));

    // War-form Physical delta (0 for any non-Garou sheet).
    let form = crate::splats::werewolf::current_form_pool_delta(&doc, &resolved.trait_fields);

    // Bounded situational modifier — cannot be abused to over-claim.
    let modifier = args.modifier.clamp(-MODIFIER_BOUND, MODIFIER_BOUND);

    // Specialty only if actually recorded on one of the pool's abilities.
    let specialty = args.specialty && has_any_specialty(&doc, &resolved.trait_fields);

    // Willpower spend deducts one *temporary* point from the NPC's own sheet
    // (clamped to permanent, mirroring `+roll/wp`). Honored only if available.
    let willpower = if args.willpower {
        let permanent = doc.number("willpower").unwrap_or(1).max(0);
        let temp = doc
            .number("willpower_temp")
            .unwrap_or(0)
            .clamp(0, permanent);
        if temp >= 1 {
            doc.set_number("willpower_temp", temp - 1);
            SheetStore::new(schema.clone())
                .put_by(w, npc, &doc, npc)
                .map_err(|es| AbilityError::Invalid(fmt_errs(&es)))?;
            true
        } else {
            false
        }
    } else {
        false
    };

    let pool = (resolved.total + modifier + wound + form).max(1);
    let difficulty = args.difficulty.unwrap_or(6).clamp(3, 9);
    let opts = Owod20Opts {
        difficulty,
        specialty,
        willpower,
        extra_successes: 0,
        again_tens_double: false,
    };

    // Roll on the shared server RNG + record in the transient roll-log ring —
    // the same path `+roll` uses, so the NPC's roll is real and auditable.
    let now = w.now();
    let outcome = {
        let st = w
            .module_state_mut::<WodState>()
            .ok_or_else(|| AbilityError::Invalid("the WoD ruleset is not loaded".to_string()))?;
        let outcome = {
            let mut rctx = RollCtx::new(st.rng());
            Owod20Pool.roll(pool, &opts, &mut rctx)
        };
        st.push_log(
            npc,
            RollLogEntry {
                who: npc,
                expr: format!("{WOD_ROLL_ABILITY}: {}", args.pool_expr),
                summary: outcome.summary(),
                difficulty: outcome.difficulty,
                ts: now,
                private: false,
                outcome: outcome.clone(),
            },
        );
        outcome
    };

    // Emit to the room and the wod.roll panel, exactly like a player roll.
    let expr_label = pool_label(&resolved);
    let line = render_line(w, npc, &expr_label, pool, wound, incap, &outcome);
    emit(w, out, npc, &line, &outcome);

    Ok(build_outcome(
        &args, difficulty, pool, wound, modifier, specialty, willpower, &resolved, &outcome,
    ))
}

/// A human/LLM label for the pool: its resolved trait labels joined (`"Strength+Brawl"`).
fn pool_label(resolved: &ResolvedPool) -> String {
    if resolved.terms.is_empty() {
        return "(pool)".to_string();
    }
    resolved
        .terms
        .iter()
        .map(|t| t.label.clone())
        .collect::<Vec<_>>()
        .join("+")
}

/// The classic one-line roll render for the room.
fn render_line(
    w: &World,
    npc: ObjId,
    expr_label: &str,
    pool: i32,
    wound: i32,
    incap: bool,
    outcome: &Owod20Outcome,
) -> String {
    let name = w
        .get(npc)
        .map_or_else(|| npc.to_string(), |o| primary_name(&o.name).to_string());
    let mut s = format!(
        "{name} rolls {expr_label} ({pool} dice, diff {}) and {}",
        outcome.difficulty,
        outcome.summary()
    );
    if wound < 0 {
        s.push_str(&format!(" [wound {wound}]"));
    }
    if incap {
        s.push_str(" [incapacitated]");
    }
    s
}

/// Broadcast the roll line to the NPC's room and push a `wod.roll` keyframe.
fn emit(w: &World, out: &mut Outbox, npc: ObjId, line: &str, outcome: &Owod20Outcome) {
    if let Some(loc) = w.get(npc).and_then(|o| o.location) {
        out.to_room(
            loc,
            Vec::new(),
            OutputEvent::Narrative {
                kind: NarrativeKind::Emit,
                actor: Some(obj_ref(w, npc)),
                text: RichText::plain(line),
            },
        );
    }
    out.push(
        Audience::Actor(npc),
        OutputEvent::Panel {
            feed: SmolStr::new(ROLL_FEED),
            kind: PanelFrameKind::Keyframe,
            data: json!({
                "who": npc.to_string(),
                "summary": line,
                "outcome": outcome,
            }),
        },
    );
}

/// The polarity word for a roll result (drives `mush-npc`'s narration check).
fn polarity(outcome: &Owod20Outcome) -> &'static str {
    match outcome.result {
        RollResult::Success(_) => "success",
        RollResult::Failure => "failure",
        RollResult::Botch(_) => "botch",
    }
}

/// A short in-fiction narration hint for the model.
fn narration_hint(expr_label: &str, outcome: &Owod20Outcome) -> String {
    let verdict = match outcome.result {
        RollResult::Success(1) => "a marginal success".to_string(),
        RollResult::Success(n) if n >= 5 => format!("a spectacular success ({n})"),
        RollResult::Success(n) => format!("a solid success ({n})"),
        RollResult::Failure => "a failure".to_string(),
        RollResult::Botch(_) => "a botch — it goes badly wrong".to_string(),
    };
    format!(
        "{expr_label} vs difficulty {}: {verdict}. Narrate this outcome in character; do not \
         invent a different result.",
        outcome.difficulty
    )
}

/// Build the structured [`AbilityOutcome`] returned to the agent loop.
#[allow(clippy::too_many_arguments)]
fn build_outcome(
    args: &WodRollInput,
    difficulty: u8,
    pool: i32,
    wound: i32,
    modifier: i32,
    specialty: bool,
    willpower: bool,
    resolved: &ResolvedPool,
    outcome: &Owod20Outcome,
) -> AbilityOutcome {
    let expr_label = pool_label(resolved);
    let terms: Vec<Value> = resolved
        .terms
        .iter()
        .map(|t| json!({ "label": t.label, "value": t.value }))
        .collect();
    AbilityOutcome {
        ok: matches!(outcome.result, RollResult::Success(_)),
        summary: narration_hint(&expr_label, outcome),
        data: json!({
            "ability": WOD_ROLL_ABILITY,
            "pool_expr": args.pool_expr,
            "pool_size": pool,
            "difficulty": difficulty,
            "wound_penalty": wound,
            "modifier": modifier,
            "specialty_applied": specialty,
            "willpower_applied": willpower,
            "polarity": polarity(outcome),
            "successes": outcome.successes(),
            "botch": outcome.is_botch(),
            "terms": terms,
            "outcome": outcome,
        }),
    }
}

fn fmt_errs(errs: &[mush_ruleset::SheetError]) -> String {
    errs.iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

// ======================================================================
// Capability profile (system-prompt grounding)
// ======================================================================

/// Ground a sheet into an NPC capability profile: a splat-aware prose summary,
/// notable stats, capability tags, and the `wod_roll` tool manifest. This text
/// goes into the agent's system prompt so the model role-plays within the
/// character's real oWoD capabilities. `schema` is the character's active
/// (template-selected) schema.
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
    let splat = splat_bits(&template, sheet);

    // Prose summary.
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
    if !splat.summary.is_empty() {
        summary.push(' ');
        summary.push_str(&splat.summary);
    }

    // Trimmed stats JSON.
    let mut stats = Map::new();
    stats.insert("template".to_string(), json!(template));
    stats.insert("attributes".to_string(), pairs_to_value(&notable_attrs));
    stats.insert("abilities".to_string(), pairs_to_value(&notable_abils));
    stats.insert("willpower".to_string(), json!(wp));
    stats.insert("health".to_string(), json!(health));
    if !splat.stats.is_empty() {
        stats.insert("powers".to_string(), Value::Object(splat.stats));
    }

    // Capability tags.
    let mut tags: Vec<SmolStr> = vec![SmolStr::new(format!("template:{template}"))];
    for (label, n) in notable_abils.iter().take(4) {
        if *n >= 3 {
            tags.push(SmolStr::new(format!("skill:{}", tag_slug(label))));
        }
    }
    if let Some(t) = domain_tag("combat", sheet, &["brawl", "melee", "firearms"]) {
        tags.push(t);
    }
    if let Some(t) = domain_tag(
        "social",
        sheet,
        &[
            "subterfuge",
            "intimidation",
            "leadership",
            "empathy",
            "expression",
        ],
    ) {
        tags.push(t);
    }
    tags.extend(splat.tags);

    AgentCapabilityProfile {
        summary,
        stats: Value::Object(stats),
        tags,
        tools: vec![wod_roll_tool_spec()],
    }
}

/// The splat-specific slice of a profile: extra prose, tags, and stats.
#[derive(Default)]
struct SplatBits {
    summary: String,
    tags: Vec<SmolStr>,
    stats: Map<String, Value>,
}

/// Splat-aware grounding for vampire / mage / werewolf, keyed off the sheet's
/// `template`. A mortal contributes nothing.
fn splat_bits(template: &str, sheet: &SheetDoc) -> SplatBits {
    match template {
        "vampire" => vampire_bits(sheet),
        "mage" => mage_bits(sheet),
        "werewolf" => werewolf_bits(sheet),
        _ => SplatBits::default(),
    }
}

fn vampire_bits(sheet: &SheetDoc) -> SplatBits {
    let mut bits = SplatBits::default();
    let clan = sheet.text("clan").unwrap_or("").to_string();
    let generation = sheet.number("generation").unwrap_or(13);
    let disc = dots_map(sheet, "disciplines");
    let bp = sheet.number("bloodpool").unwrap_or(0);
    let humanity = sheet.number("humanity").unwrap_or(0);

    let mut s = if clan.is_empty() {
        "Kindred (a vampire).".to_string()
    } else {
        format!(
            "Kindred of Clan {} (Generation {generation}).",
            title_case(&clan)
        )
    };
    if !disc.is_empty() {
        s.push_str(&format!(" Disciplines: {}.", fmt_dots(&disc)));
    }
    s.push_str(&format!(" Blood pool {bp}, Humanity {humanity}."));
    bits.summary = s;

    if !clan.is_empty() {
        bits.tags
            .push(SmolStr::new(format!("vampire:{}", tag_slug(&clan))));
    }
    bits.tags.push(SmolStr::new("splat:vampire"));
    for (name, _) in &disc {
        bits.tags
            .push(SmolStr::new(format!("discipline:{}", tag_slug(name))));
    }

    bits.stats.insert("clan".to_string(), json!(clan));
    bits.stats
        .insert("generation".to_string(), json!(generation));
    bits.stats
        .insert("disciplines".to_string(), map_to_value(&disc));
    bits.stats.insert("bloodpool".to_string(), json!(bp));
    bits.stats.insert("humanity".to_string(), json!(humanity));
    bits
}

fn mage_bits(sheet: &SheetDoc) -> SplatBits {
    let mut bits = SplatBits::default();
    let affiliation = sheet.text("affiliation").unwrap_or("").to_string();
    let arete = sheet.number("arete").unwrap_or(1);
    let spheres = dots_map(sheet, "spheres");
    let quint = sheet.number("quintessence").unwrap_or(0);
    let paradox = sheet.number("paradox").unwrap_or(0);

    let mut s = if affiliation.is_empty() {
        format!("An awakened mage (Arete {arete}).")
    } else {
        format!(
            "Awakened mage of the {} (Arete {arete}).",
            title_case(&affiliation)
        )
    };
    if !spheres.is_empty() {
        s.push_str(&format!(" Spheres: {}.", fmt_dots(&spheres)));
    }
    s.push_str(&format!(" Quintessence {quint}, Paradox {paradox}."));
    bits.summary = s;

    if !affiliation.is_empty() {
        bits.tags
            .push(SmolStr::new(format!("mage:{}", tag_slug(&affiliation))));
    }
    bits.tags.push(SmolStr::new("splat:mage"));

    bits.stats
        .insert("affiliation".to_string(), json!(affiliation));
    bits.stats.insert("arete".to_string(), json!(arete));
    bits.stats
        .insert("spheres".to_string(), map_to_value(&spheres));
    bits.stats.insert("quintessence".to_string(), json!(quint));
    bits.stats.insert("paradox".to_string(), json!(paradox));
    bits
}

fn werewolf_bits(sheet: &SheetDoc) -> SplatBits {
    let mut bits = SplatBits::default();
    let breed = sheet.text("breed").unwrap_or("").to_string();
    let auspice = sheet.text("auspice").unwrap_or("").to_string();
    let tribe = sheet.text("tribe").unwrap_or("").to_string();
    let rank = sheet.number("rank").unwrap_or(0);
    let rage = sheet.number("rage").unwrap_or(0);
    let gnosis = sheet.number("gnosis").unwrap_or(0);
    let gifts = dots_map(sheet, "gifts");

    let mut s = "Garou (a werewolf)".to_string();
    let heritage: Vec<String> = [&breed, &auspice, &tribe]
        .iter()
        .filter(|v| !v.is_empty())
        .map(|v| title_case(v))
        .collect();
    if !heritage.is_empty() {
        s.push_str(&format!(": {}", heritage.join(" ")));
    }
    s.push_str(&format!(" (Rank {rank}). Rage {rage}, Gnosis {gnosis}."));
    if !gifts.is_empty() {
        s.push_str(&format!(" Gifts: {}.", fmt_dots(&gifts)));
    }
    bits.summary = s;

    if !tribe.is_empty() {
        bits.tags
            .push(SmolStr::new(format!("werewolf:{}", tag_slug(&tribe))));
    }
    bits.tags.push(SmolStr::new("splat:werewolf"));

    bits.stats.insert("breed".to_string(), json!(breed));
    bits.stats.insert("auspice".to_string(), json!(auspice));
    bits.stats.insert("tribe".to_string(), json!(tribe));
    bits.stats.insert("rank".to_string(), json!(rank));
    bits.stats.insert("rage".to_string(), json!(rage));
    bits.stats.insert("gnosis".to_string(), json!(gnosis));
    bits.stats.insert(
        "renown".to_string(),
        json!({
            "glory": sheet.number("glory_perm").unwrap_or(0),
            "honor": sheet.number("honor_perm").unwrap_or(0),
            "wisdom": sheet.number("wisdom_perm").unwrap_or(0),
        }),
    );
    if !gifts.is_empty() {
        bits.stats.insert("gifts".to_string(), map_to_value(&gifts));
    }
    bits
}

/// A domain capability tag (`"combat:brawl"`, `"social:weak"`): the strongest
/// of `ids` at 3+ names the domain; nothing at 2+ leaves the domain untagged;
/// otherwise the domain is `weak`.
fn domain_tag(prefix: &str, sheet: &SheetDoc, ids: &[&str]) -> Option<SmolStr> {
    let best = ids
        .iter()
        .filter_map(|id| sheet.number(id).map(|n| (*id, n)))
        .max_by_key(|&(_, n)| n);
    match best {
        Some((id, n)) if n >= 3 => Some(SmolStr::new(format!("{prefix}:{}", tag_slug(id)))),
        Some((_, n)) if n >= 2 => None,
        _ => Some(SmolStr::new(format!("{prefix}:weak"))),
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
            FieldType::Track { levels, .. } => Some(levels.clone()),
            _ => None,
        })
        .unwrap_or_default();
    if levels.is_empty() {
        return "wounded".to_string();
    }
    let idx = (filled - 1).min(levels.len() - 1);
    levels[idx].label.to_lowercase()
}

/// Read a dots-map field (`disciplines`, `spheres`, `gifts`) as `name -> dots`,
/// dropping zero/absent entries.
fn dots_map(sheet: &SheetDoc, field: &str) -> Vec<(String, i64)> {
    let Some(m) = sheet.map(field) else {
        return Vec::new();
    };
    let mut out: Vec<(String, i64)> = m
        .iter()
        .filter_map(|(k, v)| v.as_i64().filter(|n| *n > 0).map(|n| (k.clone(), n)))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// `"Celerity 2, Potence 1"` from a dots list.
fn fmt_dots(entries: &[(String, i64)]) -> String {
    entries
        .iter()
        .map(|(k, n)| format!("{} {n}", title_case(k)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn map_to_value(entries: &[(String, i64)]) -> Value {
    let m: Map<String, Value> = entries.iter().map(|(k, n)| (k.clone(), json!(n))).collect();
    Value::Object(m)
}

fn pairs_to_value(pairs: &[(&str, i64)]) -> Value {
    let m: Map<String, Value> = pairs
        .iter()
        .map(|(l, n)| ((*l).to_string(), json!(n)))
        .collect();
    Value::Object(m)
}

/// Lowercase, `_`-joined tag slug (`"Get of Fenris"` -> `"get_of_fenris"`).
fn tag_slug(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

/// Title-case a lowercase enum id / key for prose (`"brujah"` -> `"Brujah"`).
fn title_case(s: &str) -> String {
    s.split([' ', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
