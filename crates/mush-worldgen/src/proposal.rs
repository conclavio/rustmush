//! Proposals (doc 14 §4.1 "outputs"; S3, 14-spatial-implementation-plan §3.4): the
//! durable record a planning run reads from and writes to, the on-loop half of the
//! building agent.
//!
//! A [`ProposalRec`] at `mod:worldgen:proposal:<id>` holds who asked, where, from
//! what (prose, a shelf pack to reconceptualize, or a player's request), the pack
//! the agent last proposed, its findings, the questions it asked and the answers,
//! the author's feedback, the spend, and where it stands. Every run is started
//! here — a [`PlanSnapshot`] captured from the world, deferred as
//! `Effect::ModuleAsync` — and every outcome lands here through the module's
//! completion, dropped when its revision is stale (an answer or feedback since the
//! run began bumps the revision, so an old run cannot overwrite a new one).
//!
//! Nothing touches the world until [`go`]: a staff proposal applies live and files
//! a post-hoc BUILD ticket for the record; a player's request activates through
//! `mush_atlas::hydrate::activate`, pending review.

use std::collections::BTreeMap;

use mush_atlas::hydrate::{self, ProposeRequest, ProposeScope, ProposeSource, Requested};
use mush_atlas::place::PlaceRef;
use mush_atlas::{AtlasState, Finding, Polygon2, RecordKey, Theme, ValidateContext, ValidateScope};
use mush_content::{ContentState, Pack, PackKind, Step};
use mush_core::{CanonicalId, Ctx, Deferred, Effect, ModuleOp, ModuleWrite, ObjId, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::data::{NS, WorldgenReq, WorldgenState};
use crate::plan::PlanLimits;

/// The persisted key prefix.
pub const PROPOSAL_PREFIX: &str = "proposal:";
/// The id counter's key.
pub const PROPOSAL_META_KEY: &str = "proposal_meta";
/// The most proposals the world keeps.
pub const MAX_PROPOSALS: usize = 1024;
/// The most questions a proposal carries.
pub const MAX_QUESTIONS: usize = 32;
/// The most feedback lines a proposal carries.
pub const MAX_FEEDBACK: usize = 16;
/// The most bytes of a brief.
pub const MAX_BRIEF_BYTES: usize = 8192;
/// The most bytes of an answer or a feedback line.
pub const MAX_ANSWER_BYTES: usize = 1024;

/// The key of a proposal document.
#[must_use]
pub fn proposal_key(id: u64) -> SmolStr {
    SmolStr::new(format!("{PROPOSAL_PREFIX}{id}"))
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// What a proposal starts from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ProposalSource {
    /// A brief.
    Prose {
        /// The text.
        text: String,
    },
    /// A shelf pack, reconceptualized (D-14-26).
    Pack {
        /// The pack.
        id: CanonicalId,
        /// Its version on the shelf.
        version: u32,
    },
    /// A player's hydration request.
    Request {
        /// The request.
        id: String,
        /// The site kind asked for.
        kind: String,
        /// The brief.
        prose: String,
    },
}

/// Where a proposal is for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ProposalScope {
    /// The world: a region pack.
    World,
    /// A region: a neighborhood pack.
    Region {
        /// The region.
        key: RecordKey,
    },
    /// A neighborhood: a site pack.
    Neighborhood {
        /// The neighborhood.
        key: RecordKey,
    },
}

impl ProposalScope {
    /// The validator's scope.
    #[must_use]
    pub fn validate_scope(&self) -> ValidateScope {
        match self {
            ProposalScope::World => ValidateScope::World,
            ProposalScope::Region { key } => ValidateScope::Region { key: key.clone() },
            ProposalScope::Neighborhood { key } => ValidateScope::Neighborhood { key: key.clone() },
        }
    }

    fn from_propose(scope: ProposeScope) -> ProposalScope {
        match scope {
            ProposeScope::World => ProposalScope::World,
            ProposeScope::Region(key) => ProposalScope::Region { key },
            ProposeScope::Neighborhood(key) => ProposalScope::Neighborhood { key },
        }
    }
}

/// Where a proposal stands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProposalStatus {
    /// A run is under way.
    Planning,
    /// The agent asked; the author answers.
    NeedsAnswers,
    /// The pack does not validate yet; feedback steers another run.
    NeedsWork {
        /// What the run said.
        message: String,
    },
    /// The pack validates: `+build/go`.
    Ready,
    /// The run stopped at the proposal's cost cap.
    Capped,
    /// The run failed.
    Failed {
        /// Why.
        message: String,
        /// Whether feedback may run it again.
        retryable: bool,
    },
    /// Applied (live, or activated pending review).
    Applied {
        /// The pack.
        pack: CanonicalId,
        /// The version.
        version: u32,
    },
    /// Withdrawn.
    Withdrawn,
}

impl ProposalStatus {
    /// The word a listing prints.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            ProposalStatus::Planning => "planning",
            ProposalStatus::NeedsAnswers => "needs answers",
            ProposalStatus::NeedsWork { .. } => "needs work",
            ProposalStatus::Ready => "ready",
            ProposalStatus::Capped => "at its cost cap",
            ProposalStatus::Failed { .. } => "failed",
            ProposalStatus::Applied { .. } => "applied",
            ProposalStatus::Withdrawn => "withdrawn",
        }
    }

    /// Whether the proposal is still being worked.
    #[must_use]
    pub fn open(&self) -> bool {
        !matches!(
            self,
            ProposalStatus::Applied { .. } | ProposalStatus::Withdrawn
        )
    }
}

/// A question the agent asked, and the answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// 1-based.
    pub n: u32,
    /// The question.
    pub text: String,
    /// The answer, once given.
    #[serde(default)]
    pub answer: Option<String>,
}

/// One proposal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalRec {
    /// The id.
    pub id: u64,
    /// Who asked.
    pub author: ObjId,
    /// Whether the author was staff when they asked (a staff go applies live).
    pub staff: bool,
    /// Where.
    pub scope: ProposalScope,
    /// From what.
    pub source: ProposalSource,
    /// Bumped by every answer, feedback and run start; an outcome for an older
    /// revision is dropped.
    pub revision: u64,
    /// The pack the agent last proposed.
    #[serde(default)]
    pub pack: Option<Pack>,
    /// The validator's findings on it.
    #[serde(default)]
    pub findings: Vec<Finding>,
    /// The questions asked, with answers.
    #[serde(default)]
    pub questions: Vec<Question>,
    /// The author's feedback, in order.
    #[serde(default)]
    pub feedback: Vec<String>,
    /// Where it stands.
    pub status: ProposalStatus,
    /// Spent so far, USD.
    #[serde(default)]
    pub cost_usd: f64,
    /// Calls so far.
    #[serde(default)]
    pub rounds: u32,
    /// The model that last served it.
    #[serde(default)]
    pub model: String,
    /// Unix seconds.
    pub created: i64,
    /// Unix seconds.
    pub updated: i64,
}

impl ProposalRec {
    /// The request behind a player's proposal.
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        match &self.source {
            ProposalSource::Request { id, .. } => Some(id),
            _ => None,
        }
    }

    /// Questions still unanswered.
    #[must_use]
    pub fn unanswered(&self) -> usize {
        self.questions.iter().filter(|q| q.answer.is_none()).count()
    }
}

// ---------------------------------------------------------------------------
// The off-loop envelope
// ---------------------------------------------------------------------------

/// What a planning run is handed: owned data only (treaty ruling 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanSnapshot {
    /// The proposal.
    pub proposal: u64,
    /// The revision the run started from.
    pub revision: u64,
    /// The author.
    pub author: ObjId,
    /// Whether the author is staff (the neighborhood's player policy is not applied).
    pub staff: bool,
    /// Where.
    pub scope: ProposalScope,
    /// From what.
    pub source: ProposalSource,
    /// The line the prompt says the scope is.
    pub scope_line: String,
    /// The brief of a reconceptualized pack, when it carries one.
    #[serde(default)]
    pub brief: Option<String>,
    /// The pack being reconceptualized.
    #[serde(default)]
    pub seed_pack: Option<Pack>,
    /// The agent's previous proposal, to revise.
    #[serde(default)]
    pub prior_pack: Option<Pack>,
    /// Its findings.
    #[serde(default)]
    pub prior_findings: Vec<Finding>,
    /// The questions and answers so far.
    #[serde(default)]
    pub answers: Vec<Question>,
    /// The feedback so far.
    #[serde(default)]
    pub feedback: Vec<String>,
    /// The footprint a request reserved.
    #[serde(default)]
    pub footprint: Option<Polygon2>,
    /// The id the pack must carry.
    pub pack_id: CanonicalId,
    /// The kind it must be.
    pub pack_kind: PackKind,
    /// The version it must be.
    pub pack_version: u32,
    /// What it must require.
    pub requires: Vec<(CanonicalId, u32)>,
    /// The neighborhood, for a site pack.
    #[serde(default)]
    pub neighborhood: Option<RecordKey>,
    /// The world, as the validator needs it.
    pub context: ValidateContext,
    /// The theme in force.
    pub theme: Theme,
    /// The limits.
    pub limits: PlanLimits,
    /// Spent on this proposal before the run, USD.
    #[serde(default)]
    pub spent_usd: f64,
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PlanStatus {
    /// The pack validates.
    Ready,
    /// The agent asked.
    NeedsAnswers,
    /// Errors remain.
    NeedsWork {
        /// What the run said.
        message: String,
    },
    /// The cost cap stopped it.
    Capped,
    /// It failed.
    Failed {
        /// Why.
        message: String,
        /// Whether feedback may run it again.
        retryable: bool,
    },
}

/// What a run came to — the completion payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanOutcome {
    /// The proposal.
    pub proposal: u64,
    /// The revision the run started from.
    pub revision: u64,
    /// The pack, if one was proposed.
    #[serde(default)]
    pub pack: Option<Pack>,
    /// Its findings.
    #[serde(default)]
    pub findings: Vec<Finding>,
    /// New questions.
    #[serde(default)]
    pub questions: Vec<String>,
    /// The run's spend, USD.
    pub cost_usd: f64,
    /// The run's calls.
    pub rounds: u32,
    /// How it ended.
    pub status: PlanStatus,
    /// The model.
    #[serde(default)]
    pub model: String,
    /// SHA-256 of the prompt.
    #[serde(default)]
    pub prompt_hash: String,
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

/// Rebuild the proposals and the id counter from the persisted documents.
#[must_use]
pub fn hydrate_boot(
    kv: &[(SmolStr, serde_json::Value)],
) -> (BTreeMap<u64, ProposalRec>, u64, Vec<SmolStr>) {
    let mut proposals = BTreeMap::new();
    let mut next = 1_u64;
    let mut dropped = Vec::new();
    for (key, value) in kv {
        if key.as_str() == PROPOSAL_META_KEY {
            next = value
                .get("next")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(1)
                .max(1);
        } else if let Some(rest) = key.strip_prefix(PROPOSAL_PREFIX) {
            match serde_json::from_value::<ProposalRec>(value.clone()) {
                Ok(rec) if rec.id.to_string() == rest && proposals.len() < MAX_PROPOSALS => {
                    // A run in flight at the restart is lost: it re-enters as a
                    // status the author can steer from.
                    let mut rec = rec;
                    if rec.status == ProposalStatus::Planning {
                        rec.status = ProposalStatus::Failed {
                            message: "the server restarted during the run".to_string(),
                            retryable: true,
                        };
                    }
                    next = next.max(rec.id.saturating_add(1));
                    proposals.insert(rec.id, rec);
                }
                Ok(_) => {
                    tracing::warn!(%key, "worldgen: a proposal's key disagrees with its id, or the book is full; dropped");
                    dropped.push(key.clone());
                }
                Err(error) => {
                    tracing::warn!(%key, %error, "worldgen: a proposal document was dropped at boot");
                    dropped.push(key.clone());
                }
            }
        }
    }
    (proposals, next, dropped)
}

// ---------------------------------------------------------------------------
// On-loop operations
// ---------------------------------------------------------------------------

fn state<'a>(ctx: &'a Ctx<'_>) -> Result<&'a WorldgenState, String> {
    ctx.state::<WorldgenState>()
        .ok_or_else(|| "The worldgen module is not composed on this server.".to_string())
}

/// WS03-S14: `@atlas/propose` reaches the planner through the function this
/// crate registers with the atlas at boot, so a planning run happens under the
/// ATLAS module's identity and the checked spelling would refuse the write of
/// our own slot. Same crate, same module, only the context's name differs; the
/// unchecked world accessor is the right spelling and `state-slot-owners.toml`
/// wants no row for it.
fn state_mut<'a>(ctx: &'a mut Ctx<'_>) -> Result<&'a mut WorldgenState, String> {
    ctx.w
        .module_state_mut::<WorldgenState>()
        .ok_or_else(|| "The worldgen module is not composed on this server.".to_string())
}

/// Write a proposal through.
pub fn persist(ctx: &mut Ctx<'_>, rec: &ProposalRec) {
    match serde_json::to_value(rec) {
        Ok(value) => ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::JsonSet {
                key: proposal_key(rec.id),
                value,
            },
        }),
        Err(error) => tracing::error!(%error, "worldgen: a proposal would not serialize"),
    }
}

fn persist_meta(ctx: &mut Ctx<'_>, next: u64) {
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new_static(PROPOSAL_META_KEY),
            value: serde_json::json!({ "next": next }),
        },
    });
}

/// Make a proposal record (status `Planning`, nothing run yet).
///
/// # Errors
///
/// A message.
pub fn create(
    ctx: &mut Ctx<'_>,
    author: ObjId,
    staff: bool,
    scope: ProposalScope,
    source: ProposalSource,
) -> Result<ProposalRec, String> {
    let now = ctx.w.now();
    let (rec, next) = {
        let st = state_mut(ctx)?;
        if st.proposals.len() >= MAX_PROPOSALS {
            return Err("The proposal book is full; ask staff to clear old ones.".to_string());
        }
        let id = st.next_proposal.max(1);
        st.next_proposal = id.saturating_add(1);
        let rec = ProposalRec {
            id,
            author,
            staff,
            scope,
            source,
            revision: 0,
            pack: None,
            findings: Vec::new(),
            questions: Vec::new(),
            feedback: Vec::new(),
            status: ProposalStatus::Planning,
            cost_usd: 0.0,
            rounds: 0,
            model: String::new(),
            created: now,
            updated: now,
        };
        st.proposals.insert(id, rec.clone());
        (rec, st.next_proposal)
    };
    persist_meta(ctx, next);
    persist(ctx, &rec);
    Ok(rec)
}

fn update(
    ctx: &mut Ctx<'_>,
    id: u64,
    mutate: impl FnOnce(&mut ProposalRec),
) -> Result<ProposalRec, String> {
    let now = ctx.w.now();
    let rec = {
        let st = state_mut(ctx)?;
        let rec = st
            .proposals
            .get_mut(&id)
            .ok_or_else(|| format!("No proposal {id}."))?;
        mutate(rec);
        rec.updated = now;
        rec.clone()
    };
    persist(ctx, &rec);
    Ok(rec)
}

/// The proposal, or why not.
pub fn get(ctx: &Ctx<'_>, id: u64) -> Result<ProposalRec, String> {
    state(ctx)?
        .proposals
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("No proposal {id}."))
}

/// A slug of a brief's first words, for a pack id.
fn slug(text: &str) -> String {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .take(3)
        .collect();
    let joined = words.join("_");
    let short: String = joined.chars().take(20).collect();
    if short.is_empty() {
        "plan".to_string()
    } else {
        short
    }
}

fn applied_version(content: &ContentState, pack: &CanonicalId) -> Option<u32> {
    content
        .latches
        .applied(pack)
        .map(mush_content::Applied::version)
}

/// What pack the run must produce for `rec`: id, kind, version, requires, the
/// neighborhood (for a site pack) and the reserved footprint.
#[allow(clippy::type_complexity)]
fn target(
    rec: &ProposalRec,
    atlas: &AtlasState,
    content: &ContentState,
) -> Result<
    (
        CanonicalId,
        PackKind,
        u32,
        Vec<(CanonicalId, u32)>,
        Option<RecordKey>,
        Option<Polygon2>,
    ),
    String,
> {
    let requires_for_neighborhood = |key: &RecordKey| -> Result<Vec<(CanonicalId, u32)>, String> {
        let nb = atlas
            .records
            .neighborhoods
            .get(key)
            .ok_or_else(|| "The neighborhood is gone from the atlas.".to_string())?;
        let region_pack = nb.region.pack.clone();
        let mut requires = Vec::new();
        if let Some(v) = applied_version(content, &region_pack) {
            requires.push((region_pack, v));
        }
        let nb_version = applied_version(content, &nb.key.pack)
            .ok_or_else(|| "The neighborhood's pack is not latched.".to_string())?;
        requires.push((nb.key.pack.clone(), nb_version));
        Ok(requires)
    };
    match &rec.source {
        ProposalSource::Request { id, .. } => {
            let request = atlas
                .hydration
                .requests
                .get(id)
                .ok_or_else(|| format!("Request {id} is gone."))?;
            let requires = requires_for_neighborhood(&request.neighborhood)?;
            Ok((
                request.pack_id(),
                PackKind::Site,
                1,
                requires,
                Some(request.neighborhood.clone()),
                Some(request.footprint.clone()),
            ))
        }
        ProposalSource::Pack { id, version } => {
            let pack = atlas
                .shelf
                .get(id)
                .ok_or_else(|| format!("{id} is not on the shelf."))?;
            let latched = applied_version(content, id).unwrap_or(0);
            let next = (*version).max(latched).saturating_add(1);
            let requires = pack
                .manifest()
                .requires()
                .iter()
                .map(|r| (r.id.clone(), r.version))
                .collect();
            let neighborhood = match &rec.scope {
                ProposalScope::Neighborhood { key } => Some(key.clone()),
                _ => None,
            };
            Ok((
                id.clone(),
                pack.manifest().kind(),
                next,
                requires,
                neighborhood,
                None,
            ))
        }
        ProposalSource::Prose { text } => {
            let tag = format!("p{}_{}", rec.id, slug(text));
            match &rec.scope {
                ProposalScope::Neighborhood { key } => {
                    let requires = requires_for_neighborhood(key)?;
                    let id = CanonicalId::new(format!("site.{tag}")).map_err(|e| e.to_string())?;
                    Ok((id, PackKind::Site, 1, requires, Some(key.clone()), None))
                }
                ProposalScope::Region { key } => {
                    let version = applied_version(content, &key.pack)
                        .ok_or_else(|| "The region's pack is not latched.".to_string())?;
                    let id = CanonicalId::new(format!("neighborhood.{tag}"))
                        .map_err(|e| e.to_string())?;
                    Ok((
                        id,
                        PackKind::Neighborhood,
                        1,
                        vec![(key.pack.clone(), version)],
                        None,
                        None,
                    ))
                }
                ProposalScope::World => {
                    let id =
                        CanonicalId::new(format!("region.{tag}")).map_err(|e| e.to_string())?;
                    Ok((id, PackKind::Region, 1, Vec::new(), None, None))
                }
            }
        }
    }
}

fn scope_line(atlas: &AtlasState, scope: &ProposalScope) -> String {
    match scope {
        ProposalScope::World => "the world (a new region)".to_string(),
        ProposalScope::Region { key } => atlas.records.regions.get(key).map_or_else(
            || format!("region {key}"),
            |r| format!("region {} ({key}): a new neighborhood in it", r.name),
        ),
        ProposalScope::Neighborhood { key } => atlas.records.neighborhoods.get(key).map_or_else(
            || format!("neighborhood {key}"),
            |n| {
                format!(
                    "neighborhood {} ({key}), a {} place of {} m × {} m: a site in it",
                    n.name,
                    n.setting,
                    n.footprint
                        .to_cm()
                        .map(|p| p.bbox().width() / 100)
                        .unwrap_or(0),
                    n.footprint
                        .to_cm()
                        .map(|p| p.bbox().height() / 100)
                        .unwrap_or(0)
                )
            },
        ),
    }
}

fn theme_for(w: &World, scope: &ProposalScope) -> Theme {
    match scope {
        ProposalScope::World => mush_atlas::world_theme(w).unwrap_or_default(),
        ProposalScope::Region { key } => {
            mush_atlas::resolve_place_theme(w, &PlaceRef::Region(key.clone()))
        }
        ProposalScope::Neighborhood { key } => {
            mush_atlas::resolve_place_theme(w, &PlaceRef::Neighborhood(key.clone()))
        }
    }
}

/// Capture the snapshot a run of `rec` needs, from the world as it stands.
///
/// # Errors
///
/// A message.
pub fn snapshot_for(ctx: &Ctx<'_>, rec: &ProposalRec) -> Result<PlanSnapshot, String> {
    let atlas = ctx
        .state::<AtlasState>()
        .ok_or_else(|| "The atlas is not composed on this server.".to_string())?;
    let content = ctx
        .state::<ContentState>()
        .ok_or_else(|| "The content module is not composed on this server.".to_string())?;
    let limits = state(ctx)?.plan;
    let (pack_id, pack_kind, pack_version, requires, neighborhood, footprint) =
        target(rec, atlas, content)?;
    let context = ValidateContext::capture(ctx.w, atlas, content, &rec.scope.validate_scope());
    let (brief, seed_pack) = match &rec.source {
        ProposalSource::Pack { id, .. } => {
            let pack = atlas.shelf.get(id).cloned();
            let brief = pack
                .as_ref()
                .and_then(|p| p.brief().cloned())
                .map(|b| b.to_string());
            (brief, pack)
        }
        _ => (None, None),
    };
    Ok(PlanSnapshot {
        proposal: rec.id,
        revision: rec.revision,
        author: rec.author,
        staff: rec.staff,
        scope: rec.scope.clone(),
        source: rec.source.clone(),
        scope_line: scope_line(atlas, &rec.scope),
        brief,
        seed_pack,
        prior_pack: rec.pack.clone(),
        prior_findings: rec.findings.clone(),
        answers: rec.questions.clone(),
        feedback: rec.feedback.clone(),
        footprint,
        pack_id,
        pack_kind,
        pack_version,
        requires,
        neighborhood,
        context,
        theme: theme_for(ctx.w, &rec.scope),
        limits,
        spent_usd: rec.cost_usd,
    })
}

/// Start (or restart) a run: bump the revision, capture the snapshot, defer the
/// call. Refused when the proposal has spent its cap or is not open.
///
/// # Errors
///
/// A message.
pub fn start_run(ctx: &mut Ctx<'_>, id: u64) -> Result<ProposalRec, String> {
    let rec = get(ctx, id)?;
    if !rec.status.open() {
        return Err(format!("Proposal {id} is {}.", rec.status.label()));
    }
    let cap = state(ctx)?.plan.cap_usd;
    if rec.cost_usd >= cap {
        return Err(format!(
            "Proposal {id} has spent its cap (${:.2} of ${cap:.2}); staff can raise it or start another.",
            rec.cost_usd
        ));
    }
    let rec = update(ctx, id, |r| {
        r.revision = r.revision.saturating_add(1);
        r.status = ProposalStatus::Planning;
    })?;
    let snap = snapshot_for(ctx, &rec)?;
    let payload = serde_json::to_string(&WorldgenReq::Plan(Box::new(snap)))
        .map_err(|e| format!("Could not prepare the planning request: {e}"))?;
    let correlation = ctx.w.next_correlation_id();
    ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
        module: SmolStr::new_static(NS),
        actor: rec.author,
        correlation,
        payload,
    }));
    Ok(rec)
}

/// The completion: fold a run's outcome into its proposal, unless the proposal
/// moved on since the run began.
pub fn on_outcome(ctx: &mut Ctx<'_>, outcome: PlanOutcome) {
    let Ok(rec) = get(ctx, outcome.proposal) else {
        tracing::warn!(
            proposal = outcome.proposal,
            "worldgen: an outcome for a proposal that is gone"
        );
        return;
    };
    if outcome.revision != rec.revision {
        tracing::info!(
            proposal = rec.id,
            ran = outcome.revision,
            now = rec.revision,
            "worldgen: a stale planning outcome was dropped"
        );
        return;
    }
    if !rec.status.open() {
        return;
    }
    let mut new_questions = Vec::new();
    let status = match &outcome.status {
        PlanStatus::Ready => ProposalStatus::Ready,
        PlanStatus::NeedsAnswers => ProposalStatus::NeedsAnswers,
        PlanStatus::NeedsWork { message } => ProposalStatus::NeedsWork {
            message: message.clone(),
        },
        PlanStatus::Capped => ProposalStatus::Capped,
        PlanStatus::Failed { message, retryable } => ProposalStatus::Failed {
            message: message.clone(),
            retryable: *retryable,
        },
    };
    let Ok(rec) = update(ctx, rec.id, |r| {
        if outcome.pack.is_some() {
            r.pack = outcome.pack.clone();
            r.findings = outcome.findings.clone();
        } else if !outcome.findings.is_empty() {
            r.findings = outcome.findings.clone();
        }
        let mut n = u32::try_from(r.questions.len()).unwrap_or(u32::MAX);
        for q in &outcome.questions {
            if r.questions.len() >= MAX_QUESTIONS {
                break;
            }
            n = n.saturating_add(1);
            r.questions.push(Question {
                n,
                text: q.clone(),
                answer: None,
            });
            new_questions.push((n, q.clone()));
        }
        r.cost_usd += outcome.cost_usd;
        r.rounds = r.rounds.saturating_add(outcome.rounds);
        if !outcome.model.is_empty() {
            r.model = outcome.model.clone();
        }
        r.status = status;
    }) else {
        return;
    };
    // A player's request learns that a proposal stands.
    if let Some(request) = rec.request_id()
        && rec.status == ProposalStatus::Ready
    {
        let _ = hydrate::set_proposal(ctx, request, rec.id);
    }
    if ctx.w.get(rec.author).is_none() {
        return;
    }
    let id = rec.id;
    let line = match &rec.status {
        ProposalStatus::Ready => format!(
            "Proposal {id} is ready: {} ({} entries, ${:.2} spent). +build/show {id} to read it, +build/go {id} to build it.",
            rec.pack
                .as_ref()
                .map_or_else(|| "a pack".to_string(), |p| p.manifest().id().to_string()),
            rec.pack.as_ref().map_or(0, |p| p.entries().len()),
            rec.cost_usd
        ),
        ProposalStatus::NeedsAnswers => {
            let mut s = format!("Proposal {id} asks:");
            for (n, q) in &new_questions {
                s.push_str(&format!("\n  {n}. {q}"));
            }
            s.push_str(&format!(
                "\nAnswer with +build/answer {id}/<n>=<answer>; the planning goes on when every question has one."
            ));
            s
        }
        ProposalStatus::NeedsWork { message } => format!(
            "Proposal {id} needs work: {message}. +build/show {id} lists the findings; +build/feedback {id}=<text> steers another run.",
        ),
        ProposalStatus::Capped => format!(
            "Proposal {id} stopped at its cost cap (${:.2}); what it has is kept. Staff can raise the cap, or start over.",
            rec.cost_usd
        ),
        ProposalStatus::Failed { message, retryable } => format!(
            "Proposal {id} failed: {message}.{}",
            if *retryable {
                format!(" +build/feedback {id}=<text> runs it again.")
            } else {
                String::new()
            }
        ),
        other => format!("Proposal {id}: {}.", other.label()),
    };
    ctx.out.info(rec.author, line);
}

/// `+build/answer <id>/<n>=<text>`: record an answer; when none is left open,
/// run again.
///
/// # Errors
///
/// A message.
pub fn answer(
    ctx: &mut Ctx<'_>,
    id: u64,
    actor: ObjId,
    n: u32,
    text: &str,
) -> Result<String, String> {
    let rec = get(ctx, id)?;
    authorize(ctx, &rec, actor)?;
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_ANSWER_BYTES {
        return Err(format!("An answer is 1 to {MAX_ANSWER_BYTES} bytes."));
    }
    if !rec.questions.iter().any(|q| q.n == n) {
        return Err(format!("Proposal {id} has no question {n}."));
    }
    if rec.status == ProposalStatus::Planning {
        return Err(format!(
            "Proposal {id} is being planned; answer when it asks."
        ));
    }
    let text = text.to_string();
    let rec = update(ctx, id, move |r| {
        if let Some(q) = r.questions.iter_mut().find(|q| q.n == n) {
            q.answer = Some(text);
        }
    })?;
    if rec.unanswered() == 0 {
        start_run(ctx, id)?;
        Ok(format!("Answered; proposal {id} is being planned again."))
    } else {
        Ok(format!(
            "Answered; {} question(s) still open on proposal {id}.",
            rec.unanswered()
        ))
    }
}

/// `+build/feedback <id>=<text>`: steer another run.
///
/// # Errors
///
/// A message.
pub fn feedback(ctx: &mut Ctx<'_>, id: u64, actor: ObjId, text: &str) -> Result<String, String> {
    let rec = get(ctx, id)?;
    authorize(ctx, &rec, actor)?;
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_ANSWER_BYTES {
        return Err(format!("Feedback is 1 to {MAX_ANSWER_BYTES} bytes."));
    }
    if rec.status == ProposalStatus::Planning {
        return Err(format!("Proposal {id} is being planned; wait for it."));
    }
    if rec.feedback.len() >= MAX_FEEDBACK {
        return Err(format!(
            "Proposal {id} has had its {MAX_FEEDBACK} rounds of feedback; start another."
        ));
    }
    let text = text.to_string();
    update(ctx, id, move |r| r.feedback.push(text))?;
    start_run(ctx, id)?;
    Ok(format!("Noted; proposal {id} is being planned again."))
}

/// `+build/withdraw <id>`.
///
/// # Errors
///
/// A message.
pub fn withdraw(ctx: &mut Ctx<'_>, id: u64, actor: ObjId) -> Result<String, String> {
    let rec = get(ctx, id)?;
    authorize(ctx, &rec, actor)?;
    if !rec.status.open() {
        return Err(format!("Proposal {id} is {}.", rec.status.label()));
    }
    update(ctx, id, |r| r.status = ProposalStatus::Withdrawn)?;
    if let Some(request) = rec.request_id() {
        // The request's slot goes with it, when it is not already built.
        let _ = hydrate::withdraw(ctx, request, actor, "proposal withdrawn");
    }
    Ok(format!("Proposal {id} withdrawn."))
}

fn authorize(ctx: &Ctx<'_>, rec: &ProposalRec, actor: ObjId) -> Result<(), String> {
    if rec.author == actor || mush_atlas::is_staff(ctx.w, actor) {
        Ok(())
    } else {
        Err(format!("Proposal {} is not yours.", rec.id))
    }
}

/// `+build/go <id>`: build it. A staff proposal applies live and files a post-hoc
/// BUILD ticket; a player's request activates pending review.
///
/// # Errors
///
/// A message; the proposal is left as it was, or marked `NeedsWork` when the world
/// moved under it.
pub fn go(ctx: &mut Ctx<'_>, id: u64, actor: ObjId) -> Result<String, String> {
    let rec = get(ctx, id)?;
    authorize(ctx, &rec, actor)?;
    if rec.status != ProposalStatus::Ready {
        return Err(format!(
            "Proposal {id} is {}; only a ready proposal is built.",
            rec.status.label()
        ));
    }
    let pack = rec
        .pack
        .clone()
        .ok_or_else(|| format!("Proposal {id} carries no pack."))?;
    // Re-validate against the world as it stands now: the run's context was a
    // snapshot, and the ground may have moved.
    {
        let atlas = ctx
            .state::<AtlasState>()
            .ok_or_else(|| "The atlas is not composed on this server.".to_string())?;
        let content = ctx
            .state::<ContentState>()
            .ok_or_else(|| "The content module is not composed on this server.".to_string())?;
        let mut cx = ValidateContext::for_pack(ctx.w, atlas, content, &pack);
        cx.player_build = rec.request_id().is_some();
        let findings = mush_atlas::validate(&pack, &cx);
        if mush_atlas::blocking(&findings) > 0 {
            let first = findings
                .iter()
                .find(|f| f.blocks())
                .map(Finding::line)
                .unwrap_or_default();
            update(ctx, id, |r| {
                r.findings = findings.clone();
                r.status = ProposalStatus::NeedsWork {
                    message: "the world moved under the plan".to_string(),
                };
            })?;
            return Err(format!(
                "Proposal {id} no longer validates: {first}. +build/feedback {id}=<text> runs it again."
            ));
        }
    }
    let version = pack.manifest().version();
    let pack_id = pack.manifest().id().clone();
    let line = if let Some(request) = rec.request_id() {
        let made = hydrate::activate(ctx, request, &pack)?;
        format!(
            "Built: {} stands pending review{}.",
            made.site,
            made.job.map(|j| format!(" (job {j})")).unwrap_or_default()
        )
    } else if rec.staff {
        let (applied, report) =
            mush_content::module::apply_pack(ctx, &pack).map_err(|e| e.to_string())?;
        if let Some(anomaly) = &report.anomaly {
            return Err(format!(
                "The apply stopped part way ({anomaly}); what was made is latched under {}. Fix the pack and go again.",
                applied.pack()
            ));
        }
        let job = hydrate::record_staff_apply(ctx, &pack, actor);
        format!(
            "Applied {}@{}: {} created, {} updated, {} retired{}.",
            applied.pack(),
            applied.version(),
            report.created,
            report.updated,
            report.retired,
            job.map(|j| format!("; ticket {j} filed for the record"))
                .unwrap_or_default()
        )
    } else {
        return Err(
            "Only staff apply a proposal live; a player's site goes through a request.".to_string(),
        );
    };
    update(ctx, id, |r| {
        r.status = ProposalStatus::Applied {
            pack: pack_id,
            version,
        };
    })?;
    Ok(line)
}

/// `+build/place <kind> [<where>]=<prose>`: reserve the ground and start planning.
///
/// # Errors
///
/// The refusal, as the player reads it.
pub fn place(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    kind: &str,
    place: Option<&str>,
    prose: &str,
) -> Result<String, String> {
    let kind = kind.trim().to_lowercase();
    let kind = if kind.starts_with("site.") {
        kind
    } else {
        format!("site.{kind}")
    };
    let prose = prose.trim();
    if prose.is_empty() {
        return Err("Say what you want built, in a sentence or a paragraph.".to_string());
    }
    if prose.len() > MAX_BRIEF_BYTES {
        return Err(format!("Keep the brief under {MAX_BRIEF_BYTES} bytes."));
    }
    let neighborhood = {
        let atlas = ctx
            .state::<AtlasState>()
            .ok_or_else(|| "The atlas is not composed on this server.".to_string())?;
        match place
            .map(str::trim)
            .filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case("here"))
        {
            Some(name) => match mush_atlas::find_place(&atlas.records, name) {
                Some(PlaceRef::Neighborhood(key)) => key,
                Some(PlaceRef::Site(key)) => atlas
                    .records
                    .sites
                    .get(&key)
                    .and_then(|s| s.neighborhoods.first().cloned())
                    .ok_or_else(|| format!("{name} is on no neighborhood."))?,
                Some(_) => {
                    return Err("Name a neighborhood (or a site in one) to build in.".to_string());
                }
                None => return Err(format!("No place called {name:?}.")),
            },
            None => {
                let here = ctx.w.get(actor).and_then(|o| o.location);
                here.and_then(|room| atlas.records.neighborhood_of_room(room))
                    .map(|n| n.key.clone())
                    .ok_or_else(|| {
                        "You are on no neighborhood; name one to build in.".to_string()
                    })?
            }
        }
    };
    let requested = hydrate::request(
        ctx,
        actor,
        &neighborhood,
        hydrate::Placement {
            kind: kind.clone(),
            footprint: None,
            prose: prose.to_string(),
        },
    )?;
    let existing = match &requested {
        Requested::Converged(r) => state(ctx)?
            .proposals
            .values()
            .find(|p| p.request_id() == Some(r.id.as_str()) && p.status.open())
            .cloned(),
        Requested::New(_) => None,
    };
    if let Some(p) = existing {
        return Ok(format!(
            "You already asked for that: proposal {} ({}). +build/show {}.",
            p.id,
            p.status.label(),
            p.id
        ));
    }
    let request = requested.record().clone();
    let staff = mush_atlas::is_staff(ctx.w, actor);
    let rec = create(
        ctx,
        actor,
        staff,
        ProposalScope::Neighborhood { key: neighborhood },
        ProposalSource::Request {
            id: request.id.clone(),
            kind,
            prose: prose.to_string(),
        },
    )?;
    start_run(ctx, rec.id)?;
    Ok(format!(
        "Request {} reserved; proposal {} is being planned. The game never blocks; you will be told when it is ready or asks.",
        request.id, rec.id
    ))
}

/// The planner the atlas hands `@atlas/propose` to.
///
/// # Errors
///
/// A message.
pub fn propose_hook(ctx: &mut Ctx<'_>, req: ProposeRequest) -> Result<String, String> {
    let scope = ProposalScope::from_propose(req.scope);
    let source = match req.source {
        ProposeSource::Prose(text) => {
            if text.len() > MAX_BRIEF_BYTES {
                return Err(format!("Keep the brief under {MAX_BRIEF_BYTES} bytes."));
            }
            ProposalSource::Prose { text }
        }
        ProposeSource::FromPack(id) => {
            let version = ctx
                .state::<AtlasState>()
                .and_then(|a| a.shelf.get(&id))
                .map(|p| p.manifest().version())
                .ok_or_else(|| format!("{id} is not on the shelf."))?;
            ProposalSource::Pack { id, version }
        }
        ProposeSource::Request(id) => {
            let (kind, prose) = ctx
                .state::<AtlasState>()
                .and_then(|a| a.hydration.requests.get(&id))
                .map(|r| (r.kind.clone(), r.prose.clone()))
                .ok_or_else(|| format!("No request {id}."))?;
            ProposalSource::Request { id, kind, prose }
        }
    };
    let staff = mush_atlas::is_staff(ctx.w, req.author);
    let rec = create(ctx, req.author, staff, scope, source)?;
    start_run(ctx, rec.id)?;
    Ok(format!(
        "Proposal {} is being planned. The game never blocks; you will be told when it is ready or asks.",
        rec.id
    ))
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// `+build/show <id>`.
#[must_use]
pub fn show_lines(ctx: &Ctx<'_>, rec: &ProposalRec) -> Vec<String> {
    let mut lines = vec![format!(
        "Proposal {} — {} — by {} — ${:.2} over {} call(s){}",
        rec.id,
        rec.status.label(),
        mush_core::output::display_name(ctx.w, rec.author),
        rec.cost_usd,
        rec.rounds,
        if rec.model.is_empty() {
            String::new()
        } else {
            format!(" ({})", rec.model)
        }
    )];
    match &rec.source {
        ProposalSource::Prose { text } => lines.push(format!("  brief: {text}")),
        ProposalSource::Pack { id, version } => {
            lines.push(format!("  reconceptualizing {id}@{version}"))
        }
        ProposalSource::Request { id, kind, prose } => {
            lines.push(format!("  request {id}: a {kind} — {prose}"));
        }
    }
    if let ProposalStatus::NeedsWork { message } | ProposalStatus::Failed { message, .. } =
        &rec.status
    {
        lines.push(format!("  {message}"));
    }
    if let Some(pack) = &rec.pack {
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for entry in pack.entries().values() {
            *kinds.entry(entry.kind().to_string()).or_default() += 1;
        }
        lines.push(format!(
            "  pack {}@{} ({}): {}",
            pack.manifest().id(),
            pack.manifest().version(),
            pack.manifest().kind().label(),
            kinds
                .iter()
                .map(|(k, n)| format!("{n} {k}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (id, entry) in pack.entries() {
            let name = entry
                .def()
                .get("name")
                .and_then(|n| n.as_str())
                .map(|n| format!(" — {n}"))
                .unwrap_or_default();
            lines.push(format!("    {id} ({}){name}", entry.kind()));
        }
        // The diff against the latch, when the pack is a new version of one.
        if let Some(content) = ctx.state::<ContentState>()
            && let Some(previous) = content.latches.applied(pack.manifest().id())
            && let Ok(plan) = mush_content::plan(pack, Some(previous))
        {
            lines.push(format!(
                "  against {}@{}: {} create, {} update, {} retire, {} unchanged",
                previous.pack(),
                previous.version(),
                plan.count(Step::Create),
                plan.count(Step::Update),
                plan.count(Step::Retire),
                plan.count(Step::Unchanged)
            ));
        }
    }
    if !rec.findings.is_empty() {
        lines.push(format!(
            "  findings ({} blocking):",
            mush_atlas::blocking(&rec.findings)
        ));
        for f in rec.findings.iter().take(24) {
            lines.push(format!("    {}", f.line()));
        }
    }
    if !rec.questions.is_empty() {
        lines.push("  questions:".to_string());
        for q in &rec.questions {
            lines.push(format!(
                "    {}. {} — {}",
                q.n,
                q.text,
                q.answer.as_deref().unwrap_or("(unanswered)")
            ));
        }
    }
    if !rec.feedback.is_empty() {
        lines.push("  feedback:".to_string());
        for f in &rec.feedback {
            lines.push(format!("    - {f}"));
        }
    }
    let next = match &rec.status {
        ProposalStatus::Ready => format!("+build/go {}", rec.id),
        ProposalStatus::NeedsAnswers => format!("+build/answer {}/<n>=<answer>", rec.id),
        ProposalStatus::NeedsWork { .. }
        | ProposalStatus::Failed {
            retryable: true, ..
        } => {
            format!("+build/feedback {}=<text>", rec.id)
        }
        ProposalStatus::Planning => "wait; it is being planned".to_string(),
        _ => String::new(),
    };
    if !next.is_empty() {
        lines.push(format!("  next: {next}"));
    }
    lines
}

/// `+build/status`: the actor's proposals, then their standing.
#[must_use]
pub fn status_lines(ctx: &Ctx<'_>, actor: ObjId) -> Vec<String> {
    let mut lines = Vec::new();
    if let Ok(st) = state(ctx) {
        let mut mine: Vec<&ProposalRec> = st
            .proposals
            .values()
            .filter(|p| p.author == actor)
            .collect();
        mine.sort_by_key(|p| (!p.status.open(), std::cmp::Reverse(p.updated)));
        if mine.is_empty() {
            lines.push("No proposals of yours.".to_string());
        } else {
            lines.push(format!("{} proposal(s):", mine.len()));
            for p in mine.iter().take(20) {
                let what = match &p.source {
                    ProposalSource::Prose { text } => text.chars().take(40).collect::<String>(),
                    ProposalSource::Pack { id, .. } => format!("reconceptualize {id}"),
                    ProposalSource::Request { kind, .. } => format!("a {kind}"),
                };
                lines.push(format!("  {} — {} — {what}", p.id, p.status.label()));
            }
        }
    }
    if let Some(atlas) = ctx.state::<AtlasState>() {
        lines.extend(hydrate::standing_lines(atlas, ctx.w, actor));
    }
    lines
}
