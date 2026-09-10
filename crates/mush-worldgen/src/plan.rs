//! The building agent's **planning run** (doc 14 §4.1; S3, 14-spatial-implementation-plan
//! §3.4): the off-loop half. It holds no world reference — it consumes a
//! [`PlanSnapshot`] captured on the world loop and produces a [`PlanOutcome`] that
//! re-enters as the module's typed completion (treaty ruling 3).
//!
//! The run is one `mush_llm::agent` tool loop: the model reads the atlas
//! (`atlas.read`, `atlas.search_sites`), proposes a whole pack (`pack.propose`),
//! revises it (`pack.revise`), validates it (`pack.validate`, which runs
//! `mush_atlas::validate` against the snapshot's captured context — the same
//! function `@atlas/pack plan` runs), and asks the author what it cannot decide
//! (`ask`). No tool touches the world: the pack is bytes on the proposal record until
//! `+build/go`. The run stops early when the pack validates with no error finding,
//! when the model asks, at `plan_max_rounds` calls, or at the proposal's cost cap.

use mush_atlas::{Finding, Severity, ValidateContext, validate};
use mush_content::{Pack, PackKind, parse_pack};
use mush_llm::agent::{
    self, RunBudget, RunLimits, RunSpec, StopCause, TextVerdict, ToolHost, ToolReply,
};
use mush_llm::{
    CallCtx, CompletionResponse, LlmError, LlmFeature, LlmService, Priority, ToolChoice, ToolDef,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::proposal::{PlanOutcome, PlanSnapshot, PlanStatus, ProposalSource};

/// The most bytes a proposed pack may serialize to.
pub const MAX_PACK_JSON_BYTES: usize = 256 * 1024;
/// The most questions one run may ask.
pub const MAX_QUESTIONS_PER_RUN: usize = 8;
/// `max_tokens` asked for per call (the route overrides it).
const PLAN_MAX_TOKENS: u32 = 4096;

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------

/// The pack schema, as the model is told it.
pub const SCHEMA: &str = "\
A pack is JSON: {\"manifest\": {\"id\", \"version\", \"kind\", \"requires\": [{\"id\", \"version\"}]}, \
\"entries\": {\"<entry id>\": {\"kind\": \"<kind>\", \"def\": {...}}}}. Entry ids are [a-z0-9_.] \
(e.g. room.lobby, exit.lobby_street, site.cafe). Coordinates are region metres, [x, y]. Kinds:\n\
- site: {name, site_kind (site.park, site.shop, ...), footprint: [[x,y],...] (a polygon), entrances: [exit ids]}\n\
- room: {name, desc, surface: \"ground\" | {\"floor\": [structure id, index]}, footprint: {\"rect\": [[x0,y0],[x1,y1]]} \
| {\"polygon\": [[x,y],...]}, site?: site id, kind?: outdoor|indoor|circulation, access?: public|restricted, \
details?: {name: text}, flags?: [outdoors|sheltered|dark|safe]}\n\
- exit: {from: room id or \"limbo\", to: room id, names: [\"Name <A>;a;alias\", \"Back <B>;b\"] (one for a one-way, two \
for a pair), portal?: {distance_m?, at?, mode?, timed?: [modes]}, lock?: \"staff\" | expr, desc?}. Rooms whose footprints \
touch need no portal; rooms apart need portal.distance_m.\n\
- structure: {site: site id, footprint: polygon, floors: [{index, z_offset_m, height_m, name?}]}\n\
- feature: {feature_kind, surface, one of point|polyline|polygon, room?}\n\
- item: {name, desc, room: room id, details?, flags?}\n\
- hook: {name, text, room?}\n\
- secret: {truth, reveal?: staff|never, about?: [entry ids]} — stored, never in any prompt field\n\
- npc: {name, room: room id, tier?: major|minor|extra, profile: {persona, background, speech_style, goals: [], knows: [], \
does_not_know: [], fallback: []}, lore?: [{tier, subject, fact, source}], tags?: []}\n\
- lore: {tag, entries: [{tier: public|guarded|secret, subject, fact, source}]}\n\
A room outside another pack is referenced as <pack id>.<entry id>. Every public room must be reachable through unlocked \
exits from the street, an access point or an existing room. Rooms on one surface never overlap; sites in a neighborhood \
never overlap; a room that names a site lies inside it.";

/// The system prompt: the contract, the scope, the pack the run is to produce,
/// the budget, the schema and the theme.
#[must_use]
pub fn system_prompt(snap: &PlanSnapshot) -> String {
    let scope = &snap.scope_line;
    let theme = &snap.theme;
    let mut out = format!(
        "You are the building agent for a text game world. You turn a brief into a CONTENT PACK: typed \
         records the engine validates and applies. You never write commands, never speak to players, and \
         never invent mechanics; you emit records through the tools and nothing else.\n\n\
         Scope: {scope}\n\
         The pack you propose is `{}` — kind {}, version {} — requiring {}.\n\
         Budget: aim to finish under ${:.2}; the run stops at ${:.2} or after {} calls, whichever first. \
         Reads are cheap; each call costs.\n\n\
         How to work:\n\
         1. Read what you need with atlas.read (neighborhood, sites, rooms, connections, theme, schema) and \
         atlas.search_sites.\n\
         2. Propose the WHOLE pack with pack.propose — manifest and every entry.\n\
         3. Call pack.validate; fix every `error` finding with pack.revise, then validate again. Warnings are \
         notes; infos need nothing.\n\
         4. Use `ask` only for what you cannot decide from the brief; the run pauses for the author's answer.\n\
         5. Stop once pack.validate reports no errors.\n\n\
         Schema:\n{SCHEMA}\n",
        snap.pack_id,
        snap.pack_kind.label(),
        snap.pack_version,
        if snap.requires.is_empty() {
            "nothing".to_string()
        } else {
            snap.requires
                .iter()
                .map(|(id, v)| format!("{id}@{v}"))
                .collect::<Vec<_>>()
                .join(", ")
        },
        snap.limits.target_usd,
        snap.limits.cap_usd,
        snap.limits.max_rounds,
    );
    if !theme.is_empty() {
        out.push_str("\nTheme:\n");
        for line in mush_atlas::theme::render_theme_lines(theme) {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// The opening user turn: the brief, the seed pack when reconceptualizing, the
/// answers so far and the author's feedback.
#[must_use]
pub fn user_prompt(snap: &PlanSnapshot) -> String {
    let mut out = String::new();
    match &snap.source {
        ProposalSource::Prose { text } => {
            out.push_str("Brief:\n");
            out.push_str(text);
            out.push('\n');
        }
        ProposalSource::Request { kind, prose, .. } => {
            out.push_str(&format!(
                "A player asks for a {kind} on the ground reserved for it (the footprint in the pack must be \
                 the reserved one, given below). Their brief:\n{prose}\n"
            ));
            if let Some(fp) = &snap.footprint {
                out.push_str(&format!(
                    "Reserved footprint (region metres): {}\n",
                    serde_json::to_string(fp).unwrap_or_default()
                ));
            }
        }
        ProposalSource::Pack { id, version } => {
            out.push_str(&format!(
                "Reconceptualize pack {id}@{version}: keep what works — its access points, its exits to \
                 other packs and every entry the brief does not ask you to change keep their ids — and \
                 revise the rest as version {}.\n",
                snap.pack_version
            ));
            if let Some(seed) = &snap.seed_pack {
                out.push_str("Current pack:\n");
                out.push_str(&serde_json::to_string(seed).unwrap_or_default());
                out.push('\n');
            }
            if let Some(brief) = &snap.brief {
                out.push_str("Its brief:\n");
                out.push_str(brief);
                out.push('\n');
            }
        }
    }
    if !snap.answers.is_empty() {
        out.push_str("\nThe author answered:\n");
        for q in &snap.answers {
            out.push_str(&format!(
                "  Q{}: {}\n  A: {}\n",
                q.n,
                q.text,
                q.answer.as_deref().unwrap_or("(unanswered)")
            ));
        }
    }
    if !snap.feedback.is_empty() {
        out.push_str("\nThe author's feedback, latest last:\n");
        for f in &snap.feedback {
            out.push_str("  - ");
            out.push_str(f);
            out.push('\n');
        }
    }
    if let Some(prior) = &snap.prior_pack {
        out.push_str("\nYour previous proposal (revise it rather than starting over):\n");
        out.push_str(&serde_json::to_string(prior).unwrap_or_default());
        out.push('\n');
        if !snap.prior_findings.is_empty() {
            out.push_str("Its findings:\n");
            for f in &snap.prior_findings {
                out.push_str("  - ");
                out.push_str(&f.line());
                out.push('\n');
            }
        }
    }
    out
}

/// The six tools (doc 14 D-14-22).
#[must_use]
pub fn tools() -> Vec<ToolDef> {
    let tool = |name: &str, description: &str, schema: serde_json::Value| ToolDef {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: schema,
        strict: false,
    };
    vec![
        tool(
            "atlas.read",
            "Read the atlas: the region, the neighborhood (footprint, budget, policy), its sites, its rooms, the connections' access points, the theme, or the pack schema.",
            serde_json::json!({
                "type": "object", "additionalProperties": false, "required": ["what"],
                "properties": { "what": { "type": "string",
                    "enum": ["region", "neighborhood", "sites", "rooms", "connections", "theme", "schema", "taxonomy"] } }
            }),
        ),
        tool(
            "atlas.search_sites",
            "Find sites in the region by a word in their name or kind.",
            serde_json::json!({
                "type": "object", "additionalProperties": false, "required": ["query"],
                "properties": { "query": { "type": "string" } }
            }),
        ),
        tool(
            "pack.propose",
            "Propose the whole pack: {manifest, entries}. The manifest's id, kind, version and requires are fixed by the run; give the entries.",
            serde_json::json!({
                "type": "object", "additionalProperties": false, "required": ["pack"],
                "properties": { "pack": { "type": "object" } }
            }),
        ),
        tool(
            "pack.revise",
            "Change entries of the proposed pack: {entries: {<id>: <entry> | null}} — null removes one.",
            serde_json::json!({
                "type": "object", "additionalProperties": false, "required": ["entries"],
                "properties": { "entries": { "type": "object" } }
            }),
        ),
        tool(
            "pack.validate",
            "Validate the proposed pack against the world: returns typed findings. No errors means it will apply.",
            serde_json::json!({ "type": "object", "additionalProperties": false, "properties": {} }),
        ),
        tool(
            "ask",
            "Ask the author questions you cannot decide from the brief. The run pauses until they answer.",
            serde_json::json!({
                "type": "object", "additionalProperties": false, "required": ["questions"],
                "properties": { "questions": { "type": "array", "items": { "type": "string" } } }
            }),
        ),
    ]
}

// ---------------------------------------------------------------------------
// The host
// ---------------------------------------------------------------------------

struct PlanHost<'a> {
    snap: &'a PlanSnapshot,
    candidate: Option<serde_json::Value>,
    pack: Option<Pack>,
    findings: Vec<Finding>,
    validated: bool,
    clean: bool,
    questions: Vec<String>,
    asked: bool,
}

impl PlanHost<'_> {
    fn reply(content: impl Into<String>, more: bool) -> Option<ToolReply> {
        Some(ToolReply {
            content: content.into(),
            is_error: false,
            more,
        })
    }

    fn refuse(content: impl Into<String>) -> Option<ToolReply> {
        Some(ToolReply {
            content: content.into(),
            is_error: true,
            more: true,
        })
    }

    fn read(&self, what: &str) -> Option<ToolReply> {
        let cx = &self.snap.context;
        let value = match what {
            "region" => serde_json::to_value(&cx.regions).unwrap_or_default(),
            "neighborhood" => serde_json::to_value(
                cx.neighborhoods
                    .iter()
                    .find(|n| self.snap.neighborhood.as_ref() == Some(&n.key))
                    .or_else(|| cx.neighborhoods.first()),
            )
            .unwrap_or_default(),
            "sites" => serde_json::to_value(&cx.sites).unwrap_or_default(),
            "rooms" => serde_json::to_value(
                cx.rooms
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "key": r.key.to_string(), "neighborhood": r.neighborhood.to_string(),
                            "site": r.site.as_ref().map(ToString::to_string), "surface": r.surface,
                            "footprint": r.footprint, "kind": r.kind, "access": r.access,
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default(),
            "connections" => serde_json::to_value(&cx.access_points).unwrap_or_default(),
            "theme" => serde_json::to_value(&self.snap.theme).unwrap_or_default(),
            "schema" => serde_json::Value::String(SCHEMA.to_string()),
            "taxonomy" => {
                serde_json::to_value(mush_atlas::validate::SITE_KINDS).unwrap_or_default()
            }
            other => return Self::refuse(format!("atlas.read: nothing called {other:?}")),
        };
        Self::reply(bounded(&value.to_string()), true)
    }

    fn search(&self, query: &str) -> Option<ToolReply> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Self::refuse("atlas.search_sites: give a word");
        }
        let hits: Vec<serde_json::Value> = self
            .snap
            .context
            .sites
            .iter()
            .filter(|s| s.name.to_lowercase().contains(&needle) || s.kind.contains(&needle))
            .map(|s| serde_json::json!({ "key": s.key.to_string(), "name": s.name, "kind": s.kind, "footprint": s.footprint }))
            .collect();
        Self::reply(bounded(&serde_json::Value::Array(hits).to_string()), true)
    }

    /// Take a proposed pack: the manifest is the run's, the entries the model's.
    fn propose(&mut self, pack: &serde_json::Value) -> Option<ToolReply> {
        let Some(entries) = pack.get("entries").and_then(|e| e.as_object()) else {
            return Self::refuse("pack.propose: the pack needs an `entries` object");
        };
        let mut candidate = serde_json::json!({
            "manifest": self.manifest(),
            "entries": entries.clone(),
        });
        if let Some(brief) = pack.get("brief") {
            candidate["brief"] = brief.clone();
        }
        self.install(candidate)
    }

    fn revise(&mut self, entries: &serde_json::Value) -> Option<ToolReply> {
        let Some(changes) = entries.as_object() else {
            return Self::refuse("pack.revise: `entries` is an object of id → entry or null");
        };
        let Some(mut candidate) = self.candidate.clone() else {
            return Self::refuse("pack.revise: propose a pack first with pack.propose");
        };
        let Some(map) = candidate.get_mut("entries").and_then(|e| e.as_object_mut()) else {
            return Self::refuse("pack.revise: the proposed pack has no entries");
        };
        for (id, entry) in changes {
            if entry.is_null() {
                map.remove(id);
            } else {
                map.insert(id.clone(), entry.clone());
            }
        }
        self.install(candidate)
    }

    fn manifest(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.snap.pack_id,
            "version": self.snap.pack_version,
            "kind": self.snap.pack_kind.label(),
            "digest": "0".repeat(64),
            "requires": self.snap.requires.iter().map(|(id, v)| serde_json::json!({ "id": id, "version": v })).collect::<Vec<_>>(),
        })
    }

    /// Parse and digest the candidate; a pack that will not parse is refused with
    /// the reason, so the model can fix it.
    fn install(&mut self, mut candidate: serde_json::Value) -> Option<ToolReply> {
        candidate["manifest"] = self.manifest();
        let text = candidate.to_string();
        if text.len() > MAX_PACK_JSON_BYTES {
            return Self::refuse(format!(
                "the pack is {} bytes; at most {MAX_PACK_JSON_BYTES}",
                text.len()
            ));
        }
        match signed(&candidate) {
            Ok(pack) => {
                let n = pack.entries().len();
                self.candidate = Some(candidate);
                self.pack = Some(pack);
                self.validated = false;
                self.clean = false;
                Self::reply(format!("accepted: {n} entries. Call pack.validate."), true)
            }
            Err(e) => Self::refuse(format!("the pack does not parse: {e}")),
        }
    }

    fn validate_now(&mut self) -> Option<ToolReply> {
        let Some(pack) = &self.pack else {
            return Self::refuse("pack.validate: propose a pack first with pack.propose");
        };
        let mut cx = self.snap.context.clone();
        cx.player_build = !self.snap.staff;
        let findings = validate(pack, &cx);
        let errors = findings.iter().filter(|f| f.blocks()).count();
        self.validated = true;
        self.clean = errors == 0;
        let lines: Vec<String> = findings.iter().map(Finding::line).collect();
        self.findings = findings;
        if self.clean {
            Self::reply(
                format!(
                    "clean: no errors{}. The pack is ready; stop.",
                    if lines.is_empty() {
                        String::new()
                    } else {
                        format!(" ({} note(s):\n{})", lines.len(), lines.join("\n"))
                    }
                ),
                false,
            )
        } else {
            Self::reply(
                format!(
                    "{errors} error(s); fix them with pack.revise and validate again:\n{}",
                    lines.join("\n")
                ),
                true,
            )
        }
    }

    fn ask(&mut self, questions: &serde_json::Value) -> Option<ToolReply> {
        let Some(items) = questions.as_array() else {
            return Self::refuse("ask: `questions` is an array of strings");
        };
        let mut added = 0;
        for q in items {
            if let Some(text) = q.as_str().map(str::trim).filter(|t| !t.is_empty())
                && self.questions.len() < MAX_QUESTIONS_PER_RUN
            {
                self.questions.push(text.chars().take(512).collect());
                added += 1;
            }
        }
        if added == 0 {
            return Self::refuse("ask: no question was given");
        }
        self.asked = true;
        Self::reply(
            format!(
                "{added} question(s) recorded; the author will answer and the run resumes. Stop now."
            ),
            false,
        )
    }
}

impl ToolHost for PlanHost<'_> {
    fn call(&mut self, name: &str, input: &serde_json::Value) -> Option<ToolReply> {
        match name {
            "atlas.read" => self.read(input.get("what").and_then(|w| w.as_str()).unwrap_or("")),
            "atlas.search_sites" => {
                self.search(input.get("query").and_then(|q| q.as_str()).unwrap_or(""))
            }
            "pack.propose" => {
                let pack = input
                    .get("pack")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                self.propose(&pack)
            }
            "pack.revise" => {
                let entries = input
                    .get("entries")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                self.revise(&entries)
            }
            "pack.validate" => self.validate_now(),
            "ask" => {
                let questions = input
                    .get("questions")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                self.ask(&questions)
            }
            _ => None,
        }
    }

    fn on_text(&mut self, _reply: &CompletionResponse) -> TextVerdict {
        if self.pack.is_none() {
            TextVerdict::Repair(
                "Reply by calling a tool: atlas.read to look, pack.propose with the whole pack, \
                 pack.validate to check it, or ask for what you cannot decide."
                    .to_string(),
            )
        } else {
            TextVerdict::Done
        }
    }

    fn finished(&self) -> bool {
        self.clean || self.asked
    }
}

/// A candidate with its digest computed and set.
fn signed(candidate: &serde_json::Value) -> Result<Pack, String> {
    let mut doc = candidate.clone();
    doc["manifest"]["digest"] = serde_json::Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).map_err(|e| e.to_string())?;
    let digest = parse_pack(&unsigned)
        .map_err(|e| e.to_string())?
        .computed_digest()
        .map_err(|e| e.to_string())?
        .to_hex();
    doc["manifest"]["digest"] = serde_json::Value::String(digest);
    parse_pack(&serde_json::to_string(&doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Keep a tool result under a page.
fn bounded(text: &str) -> String {
    const MAX: usize = 24 * 1024;
    if text.len() <= MAX {
        return text.to_string();
    }
    let cut = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i < MAX)
        .last()
        .unwrap_or(0);
    format!("{}… (truncated; read a narrower slice)", &text[..cut])
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

/// Run one planning round off the world loop.
pub async fn run_plan(llm: &LlmService, snap: &PlanSnapshot) -> PlanOutcome {
    let system = system_prompt(snap);
    let opening = user_prompt(snap);
    let prompt_hash = {
        let mut h = Sha256::new();
        h.update(system.as_bytes());
        h.update(b"\n\n");
        h.update(opening.as_bytes());
        h.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let remaining = (snap.limits.cap_usd - snap.spent_usd).max(0.0);
    let spec = RunSpec {
        ctx: CallCtx::new(LlmFeature::WorldgenPlan, Priority::Background).with_actor(snap.author),
        system,
        opening,
        tools: tools(),
        tool_choice: ToolChoice::Auto,
        limits: RunLimits {
            max_rounds: snap.limits.max_rounds,
            budget: RunBudget {
                cap_usd: Some(remaining),
                target_usd: Some(snap.limits.target_usd),
            },
            max_tokens: PLAN_MAX_TOKENS,
        },
    };
    let mut host = PlanHost {
        snap,
        candidate: None,
        pack: None,
        findings: Vec::new(),
        validated: false,
        clean: false,
        questions: Vec::new(),
        asked: false,
    };
    let report = agent::run(llm, spec, &mut host).await;
    // A pack the model never validated is validated here, so the record says
    // where it stands.
    if host.pack.is_some() && !host.validated {
        host.validate_now();
    }
    let status = match report.stopped {
        StopCause::Done | StopCause::Rounds | StopCause::NoTool
            if host.asked && !host.questions.is_empty() =>
        {
            PlanStatus::NeedsAnswers
        }
        StopCause::Done if host.clean => PlanStatus::Ready,
        StopCause::Done | StopCause::Rounds if host.pack.is_some() => PlanStatus::NeedsWork {
            message: format!(
                "{} error(s) remain after {} call(s)",
                host.findings.iter().filter(|f| f.blocks()).count(),
                report.rounds
            ),
        },
        StopCause::Done | StopCause::Rounds | StopCause::NoTool => PlanStatus::Failed {
            message: "the model proposed nothing usable".to_string(),
            retryable: true,
        },
        StopCause::Cap => PlanStatus::Capped,
        StopCause::Error(LlmError::Disabled) => PlanStatus::Failed {
            message: "LLM features are disabled.".to_string(),
            retryable: false,
        },
        StopCause::Error(LlmError::Refused { category }) => PlanStatus::Failed {
            message: format!(
                "the model declined{}",
                category.map(|c| format!(" ({c})")).unwrap_or_default()
            ),
            retryable: false,
        },
        StopCause::Error(e) => PlanStatus::Failed {
            message: format!("planning failed: {e}"),
            retryable: e.is_retryable() || matches!(e, LlmError::BudgetExceeded { .. }),
        },
    };
    PlanOutcome {
        proposal: snap.proposal,
        revision: snap.revision,
        pack: host.pack,
        findings: host.findings,
        questions: host.questions,
        cost_usd: report.cost_usd,
        rounds: u32::try_from(report.rounds).unwrap_or(u32::MAX),
        status,
        model: report.model,
        prompt_hash,
    }
}

/// The severity a plan line is coloured by (kept here so the command module
/// needs no `Severity` import).
#[must_use]
pub fn severity_word(s: Severity) -> &'static str {
    s.label()
}

/// A pack's kind as the manifest word.
#[must_use]
pub fn kind_label(kind: PackKind) -> &'static str {
    kind.label()
}

/// Serializable mirror of the limits a run is given.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanLimits {
    /// The most calls.
    pub max_rounds: usize,
    /// The proposal's whole cap, USD.
    pub cap_usd: f64,
    /// The target told to the model, USD.
    pub target_usd: f64,
}

impl Default for PlanLimits {
    fn default() -> PlanLimits {
        PlanLimits {
            max_rounds: crate::DEFAULT_PLAN_MAX_ROUNDS,
            cap_usd: crate::DEFAULT_PLAN_COST_CAP_USD,
            target_usd: crate::DEFAULT_PLAN_COST_TARGET_USD,
        }
    }
}

/// Whether a context is worth handing to the model at all.
#[must_use]
pub fn context_is_empty(cx: &ValidateContext) -> bool {
    cx.regions.is_empty() && cx.neighborhoods.is_empty()
}
