//! The M10 LLM flavor top-up (design 04 §10.4, PLAN "weather LLM flavor
//! pools"): asynchronously generate flavor-line **variants** that enrich the
//! DETERMINISTIC weather machine. The state machine stays 100% deterministic —
//! only the flavor *text* is generated; states, transitions, and selection
//! remain a pure cache read.
//!
//! ## Two halves
//!
//! - **On the world loop** ([`FlavorStore`]): the persisted, staff-reviewable
//!   pool of generated lines, keyed by `(climate, state, bucket)`. Generated
//!   lines are **pending** until a staffer approves them; only **approved**
//!   lines join the live selection pool (so no unreviewed LLM text ever reaches
//!   a player). The store lives in the [`WeatherState`](crate::WeatherState)
//!   slot and mirrors to `flavor:{climate}:{state}:{bucket}` documents via
//!   module ops (write-through, rebuilt at boot).
//! - **Off the world loop** ([`run_weather_async`]): the pure generator. It
//!   holds no world reference — it consumes a [`FlavorGenSnapshot`] (owned data
//!   snapshotted synchronously by `@weather/generate`) and returns a serialized
//!   [`FlavorGenOutcome`], carried back into the world as a typed
//!   [`WorldCommand::ModuleAsyncDone`](mush_core::engine::WorldCommand::ModuleAsyncDone).
//!   Every call is metered/audited/gated through
//!   [`LlmService`](mush_llm::LlmService) on the cheap
//!   [`LlmFeature::Ambience`](mush_llm::LlmFeature) route.
//!
//! When generation is unavailable or disabled, the hand-written lines carry the
//! weather unchanged (the M5 behavior) — the flavor store is simply empty.
//!
//! ## Known divergence from PLAN/§10.4: bucket granularity
//!
//! PLAN's Weather bullet and design 04 §10.4 specify a generated pool per
//! `(climate, state, season, time_of_day)` bucket. This implementation keys the
//! generated overlay by `(climate, state, bucket)` only — it is **not**
//! season/time-of-day scoped, so an approved `rain` ambient line is eligible in
//! summer and winter, day and night. (The *hand-written* pools in
//! [`climate`](crate::climate) ARE season/tod scoped via `ambient_season`/
//! `ambient_tod`; only the generated overlay is coarser.) The generator prompt
//! is season-neutral, so no mismatched line is produced today, but the finer
//! per-season tuning the design allows is not yet available. Extending this to
//! the full four-part key means widening [`FlavorKey`], [`FlavorGenSnapshot`],
//! the prompt, and the `flavor:{climate}:{state}:{season}:{tod}:{bucket}`
//! persistence key (a format change), plus deciding whether `@weather/generate`
//! fans out per season/tod or generates for the current bucket — deferred as a
//! scoped follow-up, not folded into M10c. Tracked in PLAN's M10 section.
//!
//! ## Live smoke (orchestrator, after the mock suite is green)
//!
//! Every test in this crate uses a `MockProvider` (no network). To smoke the
//! real path once, against a running `mushd` with `[llm].enabled = true` and a
//! real `[llm.routes.ambience]` model, a Wizard runs in-game:
//!
//! ```text
//! @weather/climate here=temperate-coastal    ; opt this zone in (if not already)
//! @weather/generate temperate-coastal rain    ; ONE cheap Ambience-route call
//! ; ...seconds later: "Weather flavor for temperate-coastal/rain: N transition
//! ;    + M ambient PENDING line(s) added (model ...)."
//! @weather/lines temperate-coastal rain        ; review [builtin] vs [PENDING #id]
//! @weather/lines/approve <id>                  ; promote one PENDING line
//! @weather #<zone>=rain                         ; pin + watch the pool in play
//! ```
//!
//! The generation is off-loop (never blocks the tick), metered/audited on the
//! `ambience` route (visible in `@llm/budget` and `@llm/log`), and nothing the
//! model produced is shown to players until `@weather/lines/approve`.

use std::collections::BTreeMap;

use mush_core::{ModuleOp, ModuleWrite, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::cell::NS;

/// Hard storage bound for one generated `(climate, state, bucket)` pool.
/// Approved lines count toward the bound and are never evicted implicitly;
/// staff must reject obsolete lines before more pending lines can be admitted.
pub const MAX_FLAVOR_LINES_PER_POOL: usize = 256;

/// Hard UTF-8 wire/storage bound for one generated flavor line.
pub const MAX_FLAVOR_LINE_BYTES: usize = 512;

/// Hard upper bound on variants requested from a provider for either bucket.
pub const MAX_FLAVOR_VARIANTS_PER_BUCKET: usize = 64;

/// Which kind of line a generated variant is, and which live selection tier it
/// joins once approved. The `bucket` component of the `flavor:{climate}:{state}:{bucket}`
/// persistence key.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlavorBucket {
    /// A "the weather shifts to `<state>`" line — merges into the generic
    /// destination transition pool ([`LinePools::transition`](crate::LinePools::transition)).
    Transition,
    /// An idle ambience line while in `<state>` — merges into the ambient pool
    /// ([`LinePools::ambient`](crate::LinePools::ambient)).
    Ambient,
}

impl FlavorBucket {
    /// Every bucket, for enumeration.
    pub const ALL: [FlavorBucket; 2] = [FlavorBucket::Transition, FlavorBucket::Ambient];

    /// The wire/key string (`"transition"` / `"ambient"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FlavorBucket::Transition => "transition",
            FlavorBucket::Ambient => "ambient",
        }
    }

    /// Parse a bucket from its key string.
    #[must_use]
    pub fn parse(s: &str) -> Option<FlavorBucket> {
        match s {
            "transition" => Some(FlavorBucket::Transition),
            "ambient" => Some(FlavorBucket::Ambient),
            _ => None,
        }
    }
}

/// The identity of one generated pool: a climate, a weather state, and a
/// [`FlavorBucket`]. Ordered so listings are stable.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FlavorKey {
    /// The climate id.
    pub climate: SmolStr,
    /// The weather state id (the destination state for transitions; the
    /// current state for ambient).
    pub state: SmolStr,
    /// The line kind.
    pub bucket: FlavorBucket,
}

impl FlavorKey {
    /// A key.
    #[must_use]
    pub fn new(
        climate: impl Into<SmolStr>,
        state: impl Into<SmolStr>,
        bucket: FlavorBucket,
    ) -> FlavorKey {
        FlavorKey {
            climate: climate.into(),
            state: state.into(),
            bucket,
        }
    }

    /// The kv key of this pool's persisted document
    /// (`flavor:{climate}:{state}:{bucket}`).
    #[must_use]
    pub fn doc_key(&self) -> SmolStr {
        SmolStr::new(format!(
            "flavor:{}:{}:{}",
            self.climate,
            self.state,
            self.bucket.as_str()
        ))
    }
}

/// One generated flavor line under review. `id` is a stable review handle
/// (`@weather/lines/approve <id>`); `approved` gates whether the line enters
/// the live selection pool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlavorLine {
    /// A stable review id (unique within a boot; recomputed as `max+1` on load).
    pub id: u64,
    /// The generated text.
    pub text: String,
    /// Whether staff approved it into the live pool (`false` = pending).
    #[serde(default)]
    pub approved: bool,
}

/// The persisted, staff-reviewable pool of generated flavor lines. Held in the
/// [`WeatherState`](crate::WeatherState) slot; each `(climate, state, bucket)`
/// pool mirrors to one `flavor:*` document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlavorStore {
    pools: BTreeMap<FlavorKey, Vec<FlavorLine>>,
    next_id: u64,
}

impl FlavorStore {
    /// The approved lines' text for `(climate, state, bucket)` — the overlay
    /// the machine merges into its hand-written selection pool. Pending lines
    /// are **never** returned, so the tick can only ever draw hand-written +
    /// approved text.
    #[must_use]
    pub fn approved_texts(&self, climate: &str, state: &str, bucket: FlavorBucket) -> Vec<String> {
        let key = FlavorKey::new(climate, state, bucket);
        self.pools
            .get(&key)
            .map(|lines| {
                lines
                    .iter()
                    .filter(|l| l.approved)
                    .map(|l| l.text.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The lines in one pool (all statuses), for listing.
    #[must_use]
    pub fn pool(&self, key: &FlavorKey) -> &[FlavorLine] {
        self.pools.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every `(key, lines)` pair, ordered — for listing and enumeration.
    pub fn iter(&self) -> impl Iterator<Item = (&FlavorKey, &[FlavorLine])> {
        self.pools.iter().map(|(k, v)| (k, v.as_slice()))
    }

    /// Whether the store holds no lines.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pools.values().all(Vec::is_empty)
    }

    /// Add generated lines to a bucket as **pending**. Blank lines and
    /// case-insensitive duplicates of a line already in the pool are dropped.
    /// Returns the number actually added.
    pub fn add_pending(
        &mut self,
        climate: &str,
        state: &str,
        bucket: FlavorBucket,
        texts: impl IntoIterator<Item = String>,
    ) -> usize {
        let key = FlavorKey::new(climate, state, bucket);
        let pool = self.pools.entry(key).or_default();
        let mut added = 0;
        for text in texts {
            if pool.len() >= MAX_FLAVOR_LINES_PER_POOL || self.next_id == u64::MAX {
                break;
            }
            let text = text.trim();
            if text.is_empty()
                || text.len() > MAX_FLAVOR_LINE_BYTES
                || pool.iter().any(|l| l.text.eq_ignore_ascii_case(text))
            {
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            pool.push(FlavorLine {
                id,
                text: text.to_string(),
                approved: false,
            });
            added += 1;
        }
        added
    }

    /// Approve one line by id (pending → live). Returns the pool key that
    /// changed, or `None` if no such id (or it was already approved).
    pub fn approve(&mut self, id: u64) -> Option<FlavorKey> {
        for (key, lines) in &mut self.pools {
            if let Some(line) = lines.iter_mut().find(|l| l.id == id) {
                if line.approved {
                    return None;
                }
                line.approved = true;
                return Some(key.clone());
            }
        }
        None
    }

    /// Reject (delete) one line by id. Returns the pool key that changed, or
    /// `None` if no such id.
    pub fn reject(&mut self, id: u64) -> Option<FlavorKey> {
        for (key, lines) in &mut self.pools {
            if let Some(pos) = lines.iter().position(|l| l.id == id) {
                lines.remove(pos);
                return Some(key.clone());
            }
        }
        None
    }

    /// Approve every pending line. Returns the set of changed pool keys.
    pub fn approve_all_pending(&mut self) -> Vec<FlavorKey> {
        let mut changed = Vec::new();
        for (key, lines) in &mut self.pools {
            let mut any = false;
            for line in lines.iter_mut() {
                if !line.approved {
                    line.approved = true;
                    any = true;
                }
            }
            if any {
                changed.push(key.clone());
            }
        }
        changed
    }

    /// Reject (delete) every pending line. Returns the set of changed pool keys.
    pub fn reject_all_pending(&mut self) -> Vec<FlavorKey> {
        let mut changed = Vec::new();
        for (key, lines) in &mut self.pools {
            let before = lines.len();
            lines.retain(|l| l.approved);
            if lines.len() != before {
                changed.push(key.clone());
            }
        }
        changed
    }

    /// Find one line (and its key) by id.
    #[must_use]
    pub fn find(&self, id: u64) -> Option<(&FlavorKey, &FlavorLine)> {
        self.pools
            .iter()
            .find_map(|(k, v)| v.iter().find(|l| l.id == id).map(|l| (k, l)))
    }

    /// Install a pool loaded from persistence (boot). Overwrites any existing
    /// pool for the key after validating the same hard bounds enforced for new
    /// lines. Invalid persisted content is an authoritative-state violation,
    /// so callers fail boot rather than silently truncate approved material.
    pub fn load_pool(&mut self, key: FlavorKey, lines: Vec<FlavorLine>) -> Result<(), String> {
        if lines.len() > MAX_FLAVOR_LINES_PER_POOL {
            return Err(format!(
                "pool contains {} lines; maximum is {MAX_FLAVOR_LINES_PER_POOL}",
                lines.len()
            ));
        }
        for (index, line) in lines.iter().enumerate() {
            if line.text.trim().is_empty() {
                return Err(format!("line {index} has empty text"));
            }
            if line.text.len() > MAX_FLAVOR_LINE_BYTES {
                return Err(format!(
                    "line {} is {} bytes; maximum is {MAX_FLAVOR_LINE_BYTES}",
                    line.id,
                    line.text.len()
                ));
            }
            if lines[..index].iter().any(|prior| prior.id == line.id)
                || self
                    .pools
                    .iter()
                    .filter(|(existing_key, _)| *existing_key != &key)
                    .flat_map(|(_, existing_lines)| existing_lines)
                    .any(|prior| prior.id == line.id)
            {
                return Err(format!("duplicate review id {}", line.id));
            }
        }
        self.pools.insert(key, lines);
        Ok(())
    }

    /// Recompute `next_id` as one past the highest id present. Called after a
    /// boot load so freshly-generated lines never collide with survivors.
    pub fn recompute_next_id(&mut self) -> Result<(), String> {
        let max = self
            .pools
            .values()
            .flat_map(|v| v.iter())
            .map(|l| l.id)
            .max();
        self.next_id = match max {
            Some(m) => m
                .checked_add(1)
                .ok_or_else(|| "review id space is exhausted".to_string())?,
            None => 0,
        };
        Ok(())
    }

    /// The module op that mirrors one pool's current contents: a `JsonSet` of
    /// the line vector, or a `Del` when the pool is empty/absent (so a
    /// fully-rejected pool leaves no orphan document).
    #[must_use]
    pub fn persist_op(&self, key: &FlavorKey) -> ModuleOp {
        let doc_key = key.doc_key();
        match self.pools.get(key) {
            Some(lines) if !lines.is_empty() => ModuleOp {
                ns: SmolStr::new_static(NS),
                op: ModuleWrite::JsonSet {
                    key: doc_key,
                    value: serde_json::to_value(lines).expect("flavor lines serialize"),
                },
            },
            _ => ModuleOp {
                ns: SmolStr::new_static(NS),
                op: ModuleWrite::Del { key: doc_key },
            },
        }
    }
}

/// Parse a persisted `flavor:{climate}:{state}:{bucket}` key into a
/// [`FlavorKey`]. Splits from the right so a climate id containing a colon
/// still resolves. Returns `None` for a non-`flavor:` key or a bad bucket.
#[must_use]
pub fn parse_flavor_key(key: &str) -> Option<FlavorKey> {
    let rest = key.strip_prefix("flavor:")?;
    // rsplitn yields [bucket, state, climate] (climate keeps any inner colons).
    let mut parts = rest.rsplitn(3, ':');
    let bucket = FlavorBucket::parse(parts.next()?)?;
    let state = parts.next()?;
    let climate = parts.next()?;
    if climate.is_empty() || state.is_empty() {
        return None;
    }
    Some(FlavorKey::new(climate, state, bucket))
}

// ----------------------------------------------------------------------
// The off-loop generator (no world reference) + its serde payloads
// ----------------------------------------------------------------------

use mush_llm::{
    CallCtx, ChatMessage, CompletionRequest, LlmError, LlmFeature, LlmService, Priority, Sampling,
    ToolChoice, ToolDef,
};

/// The forced structured-output tool name.
const TOOL_NAME: &str = "submit_flavor";

/// The synchronously-captured context handed to [`run_weather_async`] (the
/// [`Effect::ModuleAsync`](mush_core::engine::Effect::ModuleAsync) payload).
/// Owned data only — no world references (treaty ruling 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlavorGenSnapshot {
    /// The staffer who requested the generation (audit + notification).
    pub requester: ObjId,
    /// The climate id the lines are for.
    pub climate: SmolStr,
    /// The climate's human name.
    pub climate_name: String,
    /// The climate's prose style guide (grounds the model's voice).
    pub climate_style: String,
    /// The weather state the lines are for.
    pub state: SmolStr,
    /// A one-line physical description of the state (visibility, precip, wind,
    /// temperature) so the model grounds the flavor in the conditions.
    pub state_desc: String,
    /// The zone theme's style guide, if a zone in this climate carries a
    /// `worldgen.theme` — empty otherwise. Kept consistent with worldgen.
    #[serde(default)]
    pub theme_style: String,
    /// The zone theme's favored vocabulary (may be empty).
    #[serde(default)]
    pub theme_vocab: Vec<String>,
    /// The zone theme's banned words (may be empty).
    #[serde(default)]
    pub theme_banned: Vec<String>,
    /// How many variants of each bucket to request.
    pub variants: usize,
}

/// The generator's result (the
/// [`WorldCommand::ModuleAsyncDone`](mush_core::engine::WorldCommand::ModuleAsyncDone)
/// payload).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum FlavorGenOutcome {
    /// Variants were generated (stored as pending, pending staff review).
    Ok {
        /// The requesting staffer.
        requester: ObjId,
        /// The climate.
        climate: SmolStr,
        /// The state.
        state: SmolStr,
        /// Generated transition variants.
        transition: Vec<String>,
        /// Generated ambient variants.
        ambient: Vec<String>,
        /// The model that produced them.
        model: String,
    },
    /// Generation failed (disabled, budget, refusal, transport, malformed).
    Err {
        /// The requesting staffer.
        requester: ObjId,
        /// The climate.
        climate: SmolStr,
        /// The state.
        state: SmolStr,
        /// A staff-readable message.
        message: String,
    },
}

/// The tool output shape the model fills.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct FlavorToolOut {
    transition: Vec<String>,
    ambient: Vec<String>,
}

/// Run one flavor-generation request end-to-end and return the serialized
/// [`FlavorGenOutcome`]. The entry point the world task's `Effect::ModuleAsync`
/// driver invokes off the loop: `payload` is a serialized [`FlavorGenSnapshot`].
pub async fn run_weather_async(llm: &LlmService, payload: &str) -> String {
    let outcome = match serde_json::from_str::<FlavorGenSnapshot>(payload) {
        Ok(snap) => generate_flavor(llm, &snap).await,
        Err(e) => {
            tracing::error!(error = %e, "weather: undecodable flavor snapshot payload");
            FlavorGenOutcome::Err {
                requester: ObjId(0),
                climate: SmolStr::default(),
                state: SmolStr::default(),
                message: format!("internal error decoding flavor request: {e}"),
            }
        }
    };
    serde_json::to_string(&outcome).unwrap_or_else(|e| {
        tracing::error!(error = %e, "weather: unserializable flavor outcome");
        String::from(
            r#"{"status":"err","requester":0,"climate":"","state":"","message":"internal error"}"#,
        )
    })
}

async fn generate_flavor(llm: &LlmService, snap: &FlavorGenSnapshot) -> FlavorGenOutcome {
    let caps = llm.caps_for(LlmFeature::Ambience).unwrap_or_default();
    let force_tool = caps.tool_calling && caps.forced_tool_choice;
    let req = CompletionRequest {
        model: String::new(),
        system: Some(system_prompt()),
        messages: vec![ChatMessage::user(user_prompt(snap))],
        tools: vec![submit_flavor_tool()],
        tool_choice: if force_tool {
            ToolChoice::Tool(TOOL_NAME.to_string())
        } else {
            ToolChoice::Auto
        },
        // The route supplies model / max_tokens / reasoning; placeholders here.
        max_tokens: 512,
        sampling: Sampling::default(),
        stop_sequences: Vec::new(),
        reasoning: None,
    };
    let ctx = CallCtx::new(LlmFeature::Ambience, Priority::Background).with_actor(snap.requester);

    match llm.call(ctx, req).await {
        Ok(resp) => {
            let out = match parse_flavor(&resp) {
                Ok(o) => o,
                Err(message) => {
                    return FlavorGenOutcome::Err {
                        requester: snap.requester,
                        climate: snap.climate.clone(),
                        state: snap.state.clone(),
                        message: format!("The model returned unusable flavor output: {message}."),
                    };
                }
            };
            let transition = clean(out.transition, snap.variants);
            let ambient = clean(out.ambient, snap.variants);
            if transition.is_empty() && ambient.is_empty() {
                return FlavorGenOutcome::Err {
                    requester: snap.requester,
                    climate: snap.climate.clone(),
                    state: snap.state.clone(),
                    message: "The model returned no usable flavor lines.".to_string(),
                };
            }
            FlavorGenOutcome::Ok {
                requester: snap.requester,
                climate: snap.climate.clone(),
                state: snap.state.clone(),
                transition,
                ambient,
                model: resp.model.clone(),
            }
        }
        Err(LlmError::Disabled) => FlavorGenOutcome::Err {
            requester: snap.requester,
            climate: snap.climate.clone(),
            state: snap.state.clone(),
            message: "LLM features are disabled.".to_string(),
        },
        Err(LlmError::Refused { category }) => {
            let cat = category.map(|c| format!(" ({c})")).unwrap_or_default();
            FlavorGenOutcome::Err {
                requester: snap.requester,
                climate: snap.climate.clone(),
                state: snap.state.clone(),
                message: format!("The model declined to generate weather flavor{cat}."),
            }
        }
        Err(e) => FlavorGenOutcome::Err {
            requester: snap.requester,
            climate: snap.climate.clone(),
            state: snap.state.clone(),
            message: format!("Flavor generation failed: {e}."),
        },
    }
}

/// Trim, drop blanks, de-duplicate (case-insensitive), and cap to `n`.
fn clean(lines: Vec<String>, n: usize) -> Vec<String> {
    let limit = n.min(MAX_FLAVOR_VARIANTS_PER_BUCKET);
    if limit == 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        let l = l.trim();
        if l.is_empty()
            || l.len() > MAX_FLAVOR_LINE_BYTES
            || out.iter().any(|e| e.eq_ignore_ascii_case(l))
        {
            continue;
        }
        out.push(l.to_string());
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Extract the tool output: prefer the forced tool call; else a JSON object in
/// the text body.
fn parse_flavor(resp: &mush_llm::CompletionResponse) -> Result<FlavorToolOut, String> {
    if let Some((name, input)) = resp.first_tool_use() {
        if name != TOOL_NAME {
            return Err(format!("unexpected tool {name:?}; expected {TOOL_NAME:?}"));
        }
        return serde_json::from_value(input.clone())
            .map_err(|e| format!("tool input did not match the schema: {e}"));
    }
    let text = resp.text();
    let block =
        first_json_object(&text).ok_or_else(|| "no tool call and no JSON object".to_string())?;
    serde_json::from_str(block).map_err(|e| format!("embedded JSON did not parse: {e}"))
}

/// The first balanced top-level JSON object in `text` (tolerating prose or code
/// fences around it). Respects strings/escapes so a brace inside a value never
/// closes the object early.
fn first_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for (j, &c) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=j]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The stable, cache-friendly system prompt.
#[must_use]
pub fn system_prompt() -> String {
    "You write short atmospheric weather flavor lines for a text MUSH. Each line \
     is ONE sentence a player sees in their room: either a TRANSITION line (the \
     moment the weather changes into a state) or an AMBIENT line (idle drip while \
     the weather sits in a state). Rules:\n\
     - Present tense, third person, no second-person address, no names.\n\
     - One sentence each; vivid but concise; no markdown, no numbering.\n\
     - Stay true to the climate's style and the physical conditions given.\n\
     - Do NOT contradict the state (e.g. no sunshine during a storm).\n\
     - Return your answer by calling the `submit_flavor` tool with `transition` \
     and `ambient` string arrays. If tool calling is unavailable, reply with ONLY \
     a JSON object of the same shape."
        .to_string()
}

/// Assemble the user turn: climate style, the state's physical conditions, the
/// optional zone theme, and how many of each line to produce.
#[must_use]
pub fn user_prompt(snap: &FlavorGenSnapshot) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "== CLIMATE ==\n{} ({})\nStyle: {}\n\n",
        snap.climate_name,
        snap.climate,
        if snap.climate_style.trim().is_empty() {
            "(none given)"
        } else {
            snap.climate_style.trim()
        }
    ));
    s.push_str(&format!(
        "== WEATHER STATE ==\n{}\n{}\n\n",
        snap.state, snap.state_desc
    ));
    if !snap.theme_style.trim().is_empty()
        || !snap.theme_vocab.is_empty()
        || !snap.theme_banned.is_empty()
    {
        s.push_str("== ZONE THEME (match this setting) ==\n");
        if !snap.theme_style.trim().is_empty() {
            s.push_str(&format!("Style: {}\n", snap.theme_style.trim()));
        }
        if !snap.theme_vocab.is_empty() {
            s.push_str(&format!("Favor: {}\n", snap.theme_vocab.join(", ")));
        }
        if !snap.theme_banned.is_empty() {
            s.push_str(&format!("Never use: {}\n", snap.theme_banned.join(", ")));
        }
        s.push('\n');
    }
    s.push_str(&format!(
        "== TASK ==\nWrite {n} TRANSITION lines (entering the '{state}' state) and {n} \
         AMBIENT lines (idle, while it is '{state}'). Call submit_flavor with both arrays.",
        n = snap.variants,
        state = snap.state
    ));
    s
}

/// The `submit_flavor` tool schema.
#[must_use]
pub fn submit_flavor_tool() -> ToolDef {
    ToolDef {
        name: TOOL_NAME.to_string(),
        description: "Submit the generated weather flavor lines as two string arrays.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "transition": {
                    "type": "array",
                    "maxItems": MAX_FLAVOR_VARIANTS_PER_BUCKET,
                    "items": { "type": "string", "maxLength": MAX_FLAVOR_LINE_BYTES }
                },
                "ambient": {
                    "type": "array",
                    "maxItems": MAX_FLAVOR_VARIANTS_PER_BUCKET,
                    "items": { "type": "string", "maxLength": MAX_FLAVOR_LINE_BYTES }
                }
            }
        }),
        strict: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_round_trips() {
        for b in FlavorBucket::ALL {
            assert_eq!(FlavorBucket::parse(b.as_str()), Some(b));
        }
        assert_eq!(FlavorBucket::parse("nope"), None);
    }

    #[test]
    fn doc_key_and_parse_round_trip() {
        let key = FlavorKey::new("temperate-coastal", "rain", FlavorBucket::Ambient);
        assert_eq!(key.doc_key(), "flavor:temperate-coastal:rain:ambient");
        assert_eq!(parse_flavor_key(&key.doc_key()), Some(key));
        assert_eq!(parse_flavor_key("cell:5"), None);
    }

    #[test]
    fn pending_lines_never_enter_the_live_overlay() {
        let mut store = FlavorStore::default();
        let added = store.add_pending(
            "c",
            "rain",
            FlavorBucket::Ambient,
            ["Rain hammers the tin.".to_string()],
        );
        assert_eq!(added, 1);
        // Pending: the live overlay is empty.
        assert!(
            store
                .approved_texts("c", "rain", FlavorBucket::Ambient)
                .is_empty(),
            "a pending line must never reach the live pool"
        );
        // Approve it: now it is live.
        let id = store.find(0).map(|(_, l)| l.id).unwrap();
        assert!(store.approve(id).is_some());
        assert_eq!(
            store.approved_texts("c", "rain", FlavorBucket::Ambient),
            vec!["Rain hammers the tin.".to_string()]
        );
    }

    #[test]
    fn add_pending_dedups_and_skips_blanks() {
        let mut store = FlavorStore::default();
        let added = store.add_pending(
            "c",
            "clear",
            FlavorBucket::Transition,
            [
                "The sky clears.".to_string(),
                "  ".to_string(),
                "the sky clears.".to_string(), // dup (case-insensitive)
                "A hard light returns.".to_string(),
            ],
        );
        assert_eq!(added, 2);
    }

    #[test]
    fn pending_pool_is_hard_bounded_without_evicting_approved_lines() {
        let mut store = FlavorStore::default();
        let added = store.add_pending(
            "c",
            "clear",
            FlavorBucket::Ambient,
            (0..=MAX_FLAVOR_LINES_PER_POOL).map(|n| format!("line {n}")),
        );
        assert_eq!(added, MAX_FLAVOR_LINES_PER_POOL);
        let key = FlavorKey::new("c", "clear", FlavorBucket::Ambient);
        assert_eq!(store.pool(&key).len(), MAX_FLAVOR_LINES_PER_POOL);

        let first_id = store.pool(&key)[0].id;
        store.approve(first_id).unwrap();
        assert_eq!(
            store.add_pending("c", "clear", FlavorBucket::Ambient, ["new line".into()]),
            0
        );
        assert!(store.find(first_id).is_some_and(|(_, line)| line.approved));
    }

    #[test]
    fn generated_line_and_variant_bounds_are_enforced() {
        let overlong = "x".repeat(MAX_FLAVOR_LINE_BYTES + 1);
        assert!(clean(vec!["one".into()], 0).is_empty());
        assert!(clean(vec![overlong.clone()], 1).is_empty());
        assert_eq!(
            clean(
                (0..=MAX_FLAVOR_VARIANTS_PER_BUCKET)
                    .map(|n| format!("line {n}"))
                    .collect(),
                usize::MAX,
            )
            .len(),
            MAX_FLAVOR_VARIANTS_PER_BUCKET
        );

        let mut store = FlavorStore::default();
        assert_eq!(
            store.add_pending("c", "rain", FlavorBucket::Ambient, [overlong]),
            0
        );
    }

    #[test]
    fn persisted_pool_validation_fails_closed() {
        let key = FlavorKey::new("c", "fog", FlavorBucket::Ambient);
        let mut store = FlavorStore::default();
        let too_many = (0..=MAX_FLAVOR_LINES_PER_POOL)
            .map(|id| FlavorLine {
                id: id as u64,
                text: format!("line {id}"),
                approved: true,
            })
            .collect();
        assert!(store.load_pool(key.clone(), too_many).is_err());

        let duplicate_ids = vec![
            FlavorLine {
                id: 7,
                text: "one".into(),
                approved: false,
            },
            FlavorLine {
                id: 7,
                text: "two".into(),
                approved: true,
            },
        ];
        assert!(store.load_pool(key.clone(), duplicate_ids).is_err());

        store
            .load_pool(
                key,
                vec![FlavorLine {
                    id: u64::MAX,
                    text: "last id".into(),
                    approved: true,
                }],
            )
            .unwrap();
        assert!(store.recompute_next_id().is_err());
    }

    #[test]
    fn reject_removes_and_recompute_next_id_survives_reboot() {
        let mut store = FlavorStore::default();
        store.add_pending("c", "fog", FlavorBucket::Ambient, ["a".into(), "b".into()]);
        let id_b = store.find(1).map(|(_, l)| l.id).unwrap();
        assert_eq!(
            store.reject(id_b),
            Some(FlavorKey::new("c", "fog", FlavorBucket::Ambient))
        );
        assert!(store.find(id_b).is_none());
        // Simulate a reboot: a fresh store loaded with the surviving line.
        let mut booted = FlavorStore::default();
        booted
            .load_pool(
                FlavorKey::new("c", "fog", FlavorBucket::Ambient),
                vec![FlavorLine {
                    id: 0,
                    text: "a".into(),
                    approved: true,
                }],
            )
            .unwrap();
        booted.recompute_next_id().unwrap();
        // The next generated id is past the survivor's, never colliding.
        booted.add_pending("c", "fog", FlavorBucket::Ambient, ["c".into()]);
        let new_id = booted.find_by_text("c").unwrap();
        assert_eq!(new_id, 1);
    }

    #[test]
    fn empty_pool_persists_as_delete() {
        let store = FlavorStore::default();
        let key = FlavorKey::new("c", "rain", FlavorBucket::Ambient);
        assert!(matches!(store.persist_op(&key).op, ModuleWrite::Del { .. }));
    }

    #[test]
    fn prompt_carries_climate_style_and_theme() {
        let snap = FlavorGenSnapshot {
            requester: ObjId(1),
            climate: "temperate-coastal".into(),
            climate_name: "Temperate Coastal".into(),
            climate_style: "salt air and drifting sea fog".into(),
            state: "rain".into(),
            state_desc: "visibility 60%, precip 60%, wind moderate".into(),
            theme_style: "rain-slick neon streets".into(),
            theme_vocab: vec!["neon".into()],
            theme_banned: vec!["sunshine".into()],
            variants: 4,
        };
        let user = user_prompt(&snap);
        assert!(
            user.contains("salt air and drifting sea fog"),
            "climate style: {user}"
        );
        assert!(
            user.contains("rain-slick neon streets"),
            "theme style: {user}"
        );
        assert!(user.contains("neon") && user.contains("sunshine"));
    }

    // A tiny test helper: the id of the first line whose text matches.
    impl FlavorStore {
        fn find_by_text(&self, text: &str) -> Option<u64> {
            self.pools
                .values()
                .flat_map(|v| v.iter())
                .find(|l| l.text == text)
                .map(|l| l.id)
        }
    }
}
