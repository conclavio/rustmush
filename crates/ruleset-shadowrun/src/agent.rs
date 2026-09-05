//! NPC agent grounding for SR5 (treaty ruling 6; design 03 §6): render an NPC's
//! sheet into an [`AgentCapabilityProfile`] for the LLM system prompt, and
//! execute the validated **`sr_roll`** ability the agent may request.
//!
//! ## Security posture (design 03 §7)
//!
//! `sr_roll` can touch **only the NPC's own sheet**: it resolves the pool
//! against the traits recorded on it ([`resolve_sheet_pool`] rejects raw dice /
//! named bonuses, so a jailbroken model cannot manufacture a pool of 15 from an
//! Agility of 3), applies the NPC's own wound modifier + Limit, and rolls
//! through the **same server RNG and roll log** players use. The model only
//! *requests* the roll; the deterministic engine computes the outcome, which is
//! emitted to the room and the `sr.roll` panel like any player roll.

use std::sync::Arc;

use mush_core::{Audience, ObjId, Outbox, World, obj_ref, output::primary_name};
use mush_dice::RollCtx;
use mush_dice::pool::PoolMechanic;
use mush_dice::sr5::{Sr5Opts, Sr5Outcome, Sr5Pool, Sr5Result};
use mush_proto::{NarrativeKind, OutputEvent, PanelFrameKind, RichText};
use mush_ruleset::schema::SheetSchema;
use mush_ruleset::{
    AbilityError, AbilityOutcome, AgentCapabilityProfile, AgentToolSpec, SheetDoc, SheetStore,
};
use mush_ruleset::{RollSide, roll_narration};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use smol_str::SmolStr;

use crate::data;
use crate::dice::{LimitKind, ResolvedPool, resolve_sheet_pool};
use crate::schema::MAGIC;
use crate::state::{RollLogEntry, Sr5State};

/// The one ability name the SR5 NPC agent may invoke.
pub const SR_ROLL_ABILITY: &str = "sr_roll";

/// The `sr.roll` panel feed id (matches the `+roll` command's feed).
const ROLL_FEED: &str = "sr.roll";

/// The hard bound on the `modifier` argument.
const MODIFIER_BOUND: i32 = 4;

/// The stable, documented `sr_roll` tool manifest exposed to the LLM.
#[must_use]
pub fn sr_roll_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: SmolStr::new(SR_ROLL_ABILITY),
        description: "Roll one of YOUR OWN Shadowrun dice pools (an attribute + a skill \
             recorded on your sheet). 5s and 6s are hits; the engine applies your Limit and \
             wound modifier and rolls real dice — you never choose the result. Use it when the \
             story calls for a test of your character's capability."
            .to_string(),
        params_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["pool_expr"],
            "properties": {
                "pool_expr": {
                    "type": "string",
                    "description": "The trait pool, e.g. \"agility+pistols\" or \
                        \"reaction+sneaking\". Only attributes/skills on your own sheet are \
                        allowed; raw numbers and made-up bonuses are refused."
                },
                "threshold": {
                    "type": "integer", "minimum": 1, "maximum": 6, "default": 1,
                    "description": "Hits needed to succeed, default 1 (higher is harder)."
                },
                "modifier": {
                    "type": "integer", "minimum": -4, "maximum": 4, "default": 0,
                    "description": "A small situational dice modifier (-4..+4)."
                }
            }
        }),
    }
}

/// Dispatch a named NPC ability. Runs in the world task with the NPC's world +
/// outbox.
///
/// # Errors
///
/// [`AbilityError::Unknown`] for an unrecognized ability; [`AbilityError::Invalid`]
/// for bad input or an over-claimed pool.
pub(crate) fn invoke(
    schema: &Arc<SheetSchema>,
    w: &mut World,
    out: &mut Outbox,
    npc: ObjId,
    name: &str,
    input: Value,
) -> Result<AbilityOutcome, AbilityError> {
    match name {
        SR_ROLL_ABILITY => sr_roll(schema, w, out, npc, input),
        other => Err(AbilityError::Unknown(other.to_string())),
    }
}

/// The `sr_roll` argument shape.
#[derive(Debug, Deserialize)]
struct SrRollInput {
    #[serde(alias = "pool")]
    pool_expr: String,
    #[serde(default)]
    threshold: Option<u8>,
    #[serde(default)]
    modifier: i32,
}

fn sr_roll(
    schema: &Arc<SheetSchema>,
    w: &mut World,
    out: &mut Outbox,
    npc: ObjId,
    input: Value,
) -> Result<AbilityOutcome, AbilityError> {
    let args: SrRollInput = serde_json::from_value(input)
        .map_err(|e| AbilityError::Invalid(format!("sr_roll needs a pool_expr ({e})")))?;

    let doc = SheetStore::new(Arc::clone(schema))
        .get(w, npc)
        .ok_or_else(|| AbilityError::Invalid("this character has no sheet to roll".to_string()))?;

    // Pool: only real sheet traits (the over-claim fence).
    let resolved =
        resolve_sheet_pool(schema, &doc, &args.pool_expr).map_err(AbilityError::Invalid)?;

    let wound = crate::derive::wound_modifier(&doc) as i32;
    let modifier = args.modifier.clamp(-MODIFIER_BOUND, MODIFIER_BOUND);
    let threshold = args.threshold.unwrap_or(1).max(1);
    let limit = resolved
        .limit
        .map(|k| k.value(&doc).clamp(0, i64::from(u8::MAX)) as u8);

    let opts = Sr5Opts {
        threshold,
        limit,
        edge_push: false,
        edge_dice: 0,
        modifier: modifier + wound,
    };
    let pool = resolved.total;

    let now = w.now();
    let outcome = {
        let st = w.module_state_mut::<Sr5State>().ok_or_else(|| {
            AbilityError::Invalid("the Shadowrun ruleset is not loaded".to_string())
        })?;
        let outcome = {
            let mut rctx = RollCtx::new(st.rng());
            Sr5Pool.roll(pool, &opts, &mut rctx)
        };
        st.push_log(
            npc,
            RollLogEntry {
                who: npc,
                expr: format!("{SR_ROLL_ABILITY}: {}", args.pool_expr),
                summary: outcome.summary(),
                threshold,
                ts: now,
                private: false,
                outcome: outcome.clone(),
            },
        );
        outcome
    };

    let expr_label = pool_label(&resolved);
    let (line, narration) = render_line(w, npc, &expr_label, pool + opts.modifier, wound, &outcome);
    emit(w, out, npc, &line, &narration, &outcome);

    Ok(build_outcome(
        &args, threshold, pool, wound, modifier, limit, &resolved, &outcome,
    ))
}

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

/// The classic one-line roll record and, beside it, the one-line room
/// narration (interface plan I3) — the same pair a player `+roll` produces.
fn render_line(
    w: &World,
    npc: ObjId,
    expr_label: &str,
    dice: i32,
    wound: i32,
    outcome: &Sr5Outcome,
) -> (String, RichText) {
    let name = w
        .get(npc)
        .map_or_else(|| npc.to_string(), |o| primary_name(&o.name).to_string());
    let mut s = format!(
        "{name} rolls {expr_label} ({} dice) and {}",
        dice.max(0),
        outcome.summary()
    );
    let mut notes = Vec::new();
    if wound < 0 {
        s.push_str(&format!(" [wound {wound}]"));
        notes.push(format!("[wound {wound}]"));
    }
    if let Some(l) = outcome.limit_text() {
        notes.push(format!("[{l}]"));
    }
    let narration = roll_narration(
        &RollSide {
            name: &name,
            expr: expr_label,
            dice: &outcome.faces_text(),
            result: &outcome.result_text(),
        },
        &notes,
    );
    (s, narration)
}

fn emit(
    w: &World,
    out: &mut Outbox,
    npc: ObjId,
    line: &str,
    narration: &RichText,
    outcome: &Sr5Outcome,
) {
    if let Some(loc) = w.get(npc).and_then(|o| o.location) {
        out.to_room(
            loc,
            Vec::new(),
            OutputEvent::Narrative {
                kind: NarrativeKind::Emit,
                actor: Some(obj_ref(w, npc)),
                text: narration.clone(),
            },
        );
    }
    out.push(
        Audience::Actor(npc),
        OutputEvent::Panel {
            feed: SmolStr::new(ROLL_FEED),
            kind: PanelFrameKind::Keyframe,
            data: json!({ "who": npc.to_string(), "summary": line, "outcome": outcome }),
        },
    );
}

fn polarity(outcome: &Sr5Outcome) -> &'static str {
    match outcome.result {
        Sr5Result::Success(_) => "success",
        Sr5Result::Failure => "failure",
        Sr5Result::Glitch(_) => "glitch",
        Sr5Result::CriticalGlitch => "critical_glitch",
    }
}

fn narration_hint(expr_label: &str, threshold: u8, outcome: &Sr5Outcome) -> String {
    let verdict = match outcome.result {
        Sr5Result::Success(0) => "a bare success".to_string(),
        Sr5Result::Success(n) if n >= 4 => format!("a stunning success ({n} net hits)"),
        Sr5Result::Success(n) => format!("a success ({n} net hits)"),
        Sr5Result::Failure => "a failure".to_string(),
        Sr5Result::Glitch(_) => "a glitch — it works but something goes wrong".to_string(),
        Sr5Result::CriticalGlitch => "a critical glitch — disaster".to_string(),
    };
    format!(
        "{expr_label} (threshold {threshold}, {} hits): {verdict}. Narrate this outcome in \
         character; do not invent a different result.",
        outcome.hits()
    )
}

#[allow(clippy::too_many_arguments)]
fn build_outcome(
    args: &SrRollInput,
    threshold: u8,
    pool: i32,
    wound: i32,
    modifier: i32,
    limit: Option<u8>,
    resolved: &ResolvedPool,
    outcome: &Sr5Outcome,
) -> AbilityOutcome {
    let expr_label = pool_label(resolved);
    let terms: Vec<Value> = resolved
        .terms
        .iter()
        .map(|t| json!({ "label": t.label, "value": t.value }))
        .collect();
    AbilityOutcome {
        ok: matches!(outcome.result, Sr5Result::Success(_)),
        summary: narration_hint(&expr_label, threshold, outcome),
        data: json!({
            "ability": SR_ROLL_ABILITY,
            "pool_expr": args.pool_expr,
            "pool_size": (pool + modifier + wound).max(0),
            "threshold": threshold,
            "limit": limit,
            "wound_modifier": wound,
            "modifier": modifier,
            "polarity": polarity(outcome),
            "hits": outcome.hits(),
            "net_hits": outcome.net_hits,
            "glitch": outcome.glitch,
            "terms": terms,
            "outcome": outcome,
        }),
    }
}

// ======================================================================
// Capability profile
// ======================================================================

/// Ground a sheet into an NPC capability profile: a prose summary, notable
/// stats, capability tags, and the `sr_roll` tool manifest.
#[must_use]
pub fn agent_profile(schema: &SheetSchema, sheet: &SheetDoc) -> AgentCapabilityProfile {
    let _ = schema;
    let metatype = sheet.text("metatype").unwrap_or("human").to_string();
    let awakened = sheet.text("awakened_type").unwrap_or("mundane").to_string();

    let mut notable_attrs: Vec<(&str, i64)> = data::CORE_ATTRS
        .iter()
        .filter_map(|(id, label, _)| sheet.number(id).filter(|n| *n >= 4).map(|n| (*label, n)))
        .collect();
    notable_attrs.sort_by_key(|&(_, n)| std::cmp::Reverse(n));

    let mut notable_skills: Vec<(String, i64)> = sheet
        .map(crate::schema::ACTIVE_SKILLS_FIELD)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_i64().filter(|n| *n >= 3).map(|n| (label_skill(k), n)))
                .collect()
        })
        .unwrap_or_default();
    notable_skills.sort_by_key(|(_, n)| std::cmp::Reverse(*n));

    let lim = crate::derive::limits(sheet);

    // Prose summary.
    let mut summary = format!("A {metatype}");
    if awakened != "mundane" {
        summary.push_str(&format!(" {awakened}"));
    }
    summary.push('.');
    if let Some((label, n)) = notable_attrs.first() {
        summary.push_str(&format!(" Best attribute {label} {n}."));
    }
    if !notable_skills.is_empty() {
        let list: Vec<String> = notable_skills
            .iter()
            .take(3)
            .map(|(l, n)| format!("{l} {n}"))
            .collect();
        summary.push_str(&format!(" Skilled at {}.", list.join(", ")));
    }
    summary.push_str(&format!(
        " Limits P{}/M{}/S{}.",
        lim.physical, lim.mental, lim.social
    ));

    // Trimmed stats.
    let mut stats = Map::new();
    stats.insert("metatype".to_string(), json!(metatype));
    stats.insert("awakened_type".to_string(), json!(awakened));
    stats.insert("attributes".to_string(), attrs_value(&notable_attrs));
    stats.insert("skills".to_string(), skills_value(&notable_skills));
    stats.insert(
        "limits".to_string(),
        json!({ "physical": lim.physical, "mental": lim.mental, "social": lim.social }),
    );
    if sheet.number(MAGIC).unwrap_or(0) > 0 {
        stats.insert("magic".to_string(), json!(sheet.number(MAGIC).unwrap_or(0)));
    }

    // Tags.
    let mut tags: Vec<SmolStr> = vec![
        SmolStr::new(format!("metatype:{metatype}")),
        SmolStr::new(format!("awakened:{awakened}")),
    ];
    if awakened != "mundane" {
        tags.push(SmolStr::new("magic:awakened"));
    }
    if let Some(t) = domain_tag(
        "combat",
        sheet,
        &[
            "pistols",
            "automatics",
            "longarms",
            "blades",
            "clubs",
            "unarmed",
        ],
    ) {
        tags.push(t);
    }
    if let Some(t) = domain_tag("matrix", sheet, &["hacking", "cybercombat", "computer"]) {
        tags.push(t);
    }
    for (l, n) in notable_skills.iter().take(4) {
        if *n >= 4 {
            tags.push(SmolStr::new(format!("skill:{}", slug(l))));
        }
    }

    AgentCapabilityProfile {
        summary,
        stats: Value::Object(stats),
        tags,
        tools: vec![sr_roll_tool_spec()],
    }
}

/// A domain tag (`combat:pistols`, `combat:weak`): the strongest of `ids` at 4+
/// names the domain; nothing at 2+ leaves it untagged; else `weak`.
fn domain_tag(prefix: &str, sheet: &SheetDoc, ids: &[&str]) -> Option<SmolStr> {
    let best = ids
        .iter()
        .filter_map(|id| {
            sheet
                .map(crate::schema::ACTIVE_SKILLS_FIELD)
                .and_then(|m| m.get(*id))
                .and_then(serde_json::Value::as_i64)
                .map(|n| (*id, n))
        })
        .max_by_key(|&(_, n)| n);
    match best {
        Some((id, n)) if n >= 4 => Some(SmolStr::new(format!("{prefix}:{}", slug(id)))),
        Some((_, n)) if n >= 2 => None,
        _ => Some(SmolStr::new(format!("{prefix}:weak"))),
    }
}

fn label_skill(id: &str) -> String {
    data::active_skill(id).map_or_else(|| id.to_string(), |s| s.name.to_string())
}

fn attrs_value(pairs: &[(&str, i64)]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(l, n)| ((*l).to_string(), json!(n)))
            .collect(),
    )
}

fn skills_value(pairs: &[(String, i64)]) -> Value {
    Value::Object(pairs.iter().map(|(l, n)| (l.clone(), json!(n))).collect())
}

fn slug(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

/// A local LimitKind re-export guard (kept so the module compiles if `dice`
/// prunes the type in future refactors).
#[allow(dead_code)]
type _LimitKindGuard = LimitKind;
