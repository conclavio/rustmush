//! Hydration (doc 14 §4.5; S3, 14-spatial-implementation-plan §3.4): a player asks
//! for a site, the world reserves what the request needs, the building agent plans
//! it, and `+build/go` activates it as a `site` pack under review.
//!
//! What this module owns:
//!
//! - **Standing** — `mod:atlas:standing:<char>` ([`StandingRec`]): the character's
//!   cap on outstanding requests (`None` is the configured default) and their
//!   strikes. At `hydration_strikes_to_zero` strikes the cap is zero until staff
//!   reset it (`@atlas/cap`). All of it is on the character; accounts are not
//!   principals (D-14-25).
//! - **Requests** — `mod:atlas:request:<id>` ([`RequestRec`]): one player's ask for
//!   one site, keyed by an [`IdempotencyKeyText`] derived from `(character,
//!   neighborhood, kind, footprint)`, so a retry, a second tab and a second ask for
//!   the same implied site converge on the first request rather than charging a
//!   second slot. A request reserves three things while it is outstanding: one slot
//!   of the character's cap, one unit of the neighborhood's `max_player_sites`, and
//!   its footprint, which no other request or site may overlap.
//! - **Activation** — [`activate`]: the planner's `site` pack is applied, its sites
//!   marked `origin: Hydrated`, `review: PendingReview`; a `BUILD` ticket carries the
//!   request, the pack digest and the neighborhood to the reviewers; a canon event
//!   records it. Playable at once.
//! - **Review** — the `build.review` payload handler on `+job/approve` (the site is
//!   kept and the slot released), the status observer on `+job/deny` (changes
//!   requested: the site stays live, the slot is still held), and `@atlas/strike`
//!   (the site's pack is retired and the character's strikes rise).
//! - **The hand-off to the planner** — [`AtlasHooks`]: `@atlas/propose` reaches the
//!   worldgen planner through a function it registers at boot, since the atlas has
//!   no LLM edge and never will.
//!
//! The reservations are the request records themselves, re-read on every check:
//! the plan named `ReservationBook` (C8) for the three quantities, and the deviation
//! is recorded in the plan's ledger — they are two small counts and one footprint,
//! all derived from the outstanding set, and the authoritative family arrives when
//! the atlas takes an authoritative action.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_canon::{CanonDraft, CanonScope};
use mush_content::{Pack, PackKind};
use mush_core::{CanonicalId, Ctx, DefinitionDigest, IdempotencyKeyText, ObjId, Role, World};
use mush_jobs::{Job, JobPayload, JobStatus, OpenJob};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::geom::{Point2, Polygon2, Rect2, Shape};
use crate::module::{AtlasState, del_doc, put_doc};
use crate::raster::overlap;
use crate::record::{KIND_SITE, RecordKey, ReviewState, SiteOrigin, record_key};

/// The review queue.
pub const BUILD_QUEUE: &str = "BUILD";
/// The payload handler `+job/approve` runs on a BUILD ticket.
pub const BUILD_HANDLER: &str = "build.review";
/// The status observer that learns of a deny.
pub const STATUS_OBSERVER: &str = "atlas.build";
/// The default cap on a character's outstanding requests (`[worldgen].hydration_cap`).
pub const DEFAULT_HYDRATION_CAP: u32 = 2;
/// Strikes at which the cap becomes zero (`[worldgen].hydration_strikes_to_zero`).
pub const DEFAULT_STRIKES_TO_ZERO: u32 = 3;
/// The most requests the world keeps.
pub const MAX_REQUESTS: usize = 4096;
/// The most bytes of prose a request carries.
pub const MAX_PROSE_BYTES: usize = 4096;
/// The side, metres, of the square a request without a footprint asks for.
pub const DEFAULT_SITE_M: f64 = 20.0;
/// The step, metres, the free-ground search moves by.
const SEARCH_STEP_M: f64 = 10.0;
/// The BUILD payload's schema version.
pub const REQUEST_SCHEMA_VERSION: u32 = 1;
/// The digest domain of a request id's footprint hash.
const FOOTPRINT_DOMAIN: &str = "mush.atlas.hydration.footprint.v1";
/// The canon source prefix.
const SOURCE_PREFIX: &str = "atlas:request:";

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// The key of a standing document.
#[must_use]
pub fn standing_key(who: ObjId) -> SmolStr {
    SmolStr::new(format!("standing:{}", who.0))
}

/// The key of a request document.
#[must_use]
pub fn request_key(id: &str) -> SmolStr {
    SmolStr::new(format!("request:{id}"))
}

/// Whether a persisted key is this module's.
#[must_use]
pub fn is_hydration_key(key: &str) -> bool {
    key.starts_with("standing:") || key.starts_with("request:")
}

/// A character's standing (doc 14 §4.5, D-14-25).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandingRec {
    /// The character.
    pub character: ObjId,
    /// The cap on outstanding requests; `None` is the configured default.
    #[serde(default)]
    pub cap: Option<u32>,
    /// Strikes taken.
    #[serde(default)]
    pub strikes: u32,
    /// Revision.
    pub revision: u64,
}

/// Where a request is.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RequestStatus {
    /// The planner is at work, or the player is answering it.
    Generating,
    /// A proposal stands, ready for `+build/go`.
    Proposed {
        /// The proposal.
        proposal: u64,
    },
    /// The site is live, pending review.
    Activated {
        /// The site.
        site: RecordKey,
        /// The BUILD ticket, when the jobs module is composed.
        #[serde(default)]
        job: Option<u64>,
    },
    /// Reviewed and kept; the slot is released.
    Approved {
        /// The site.
        site: RecordKey,
    },
    /// The reviewer asked for changes; the site stays live and the slot is held.
    NeedsChanges {
        /// The site.
        site: RecordKey,
        /// The BUILD ticket.
        #[serde(default)]
        job: Option<u64>,
    },
    /// Withdrawn before activation, or its pack retired.
    Withdrawn {
        /// Why.
        #[serde(default)]
        reason: String,
    },
    /// Struck: the site retired, the character's strikes raised.
    Struck {
        /// Why.
        reason: String,
    },
}

impl RequestStatus {
    /// Whether the request still holds its slot, its unit and its footprint.
    #[must_use]
    pub fn outstanding(&self) -> bool {
        matches!(
            self,
            RequestStatus::Generating
                | RequestStatus::Proposed { .. }
                | RequestStatus::Activated { .. }
                | RequestStatus::NeedsChanges { .. }
        )
    }

    /// Whether the request's site stands.
    #[must_use]
    pub fn site(&self) -> Option<&RecordKey> {
        match self {
            RequestStatus::Activated { site, .. }
            | RequestStatus::Approved { site }
            | RequestStatus::NeedsChanges { site, .. } => Some(site),
            _ => None,
        }
    }

    /// The word `+build/status` prints.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            RequestStatus::Generating => "generating",
            RequestStatus::Proposed { .. } => "proposed",
            RequestStatus::Activated { .. } => "pending review",
            RequestStatus::Approved { .. } => "approved",
            RequestStatus::NeedsChanges { .. } => "changes requested",
            RequestStatus::Withdrawn { .. } => "withdrawn",
            RequestStatus::Struck { .. } => "struck",
        }
    }
}

/// One player's request for one site.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestRec {
    /// The id (an idempotency key over character, neighborhood, kind and footprint).
    pub id: String,
    /// Who asked.
    pub character: ObjId,
    /// Where.
    pub neighborhood: RecordKey,
    /// The site kind asked for.
    pub kind: String,
    /// The footprint reserved, region metres.
    pub footprint: Polygon2,
    /// What they asked for.
    pub prose: String,
    /// Where it is.
    pub status: RequestStatus,
    /// The pack that activated it, if any.
    #[serde(default)]
    pub pack: Option<CanonicalId>,
    /// Unix seconds.
    pub created: i64,
    /// Unix seconds.
    pub updated: i64,
    /// Revision.
    pub revision: u64,
}

impl RequestRec {
    /// Re-check every bound.
    ///
    /// # Errors
    ///
    /// A message.
    pub fn validate(&self) -> Result<(), String> {
        IdempotencyKeyText::parse_canonical(&self.id).map_err(|e| format!("id: {e}"))?;
        if self.kind.trim().is_empty() || self.kind.len() > crate::record::MAX_WORD_BYTES {
            return Err("kind is empty or too long".to_string());
        }
        if self.prose.len() > MAX_PROSE_BYTES {
            return Err(format!("prose is over {MAX_PROSE_BYTES} bytes"));
        }
        self.footprint
            .to_cm()
            .map_err(|e| format!("footprint: {e}"))?;
        Ok(())
    }

    /// The id of the site pack that activates this request.
    #[must_use]
    pub fn pack_id(&self) -> CanonicalId {
        let tail: String = self
            .id
            .rsplit(':')
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(12)
            .collect();
        CanonicalId::new(format!("site.hyd_{}_{tail}", self.character.0))
            .or_else(|_| CanonicalId::new(format!("site.hyd_{}", self.character.0)))
            .unwrap_or_else(|_| self.neighborhood.pack.clone())
    }
}

/// What the world holds of hydration, in RAM, mirrored by the documents.
#[derive(Clone, Debug, Default)]
pub struct HydrationState {
    /// Standing, by character.
    pub standing: BTreeMap<ObjId, StandingRec>,
    /// Requests, by id.
    pub requests: BTreeMap<String, RequestRec>,
}

impl HydrationState {
    /// The character's outstanding requests.
    pub fn outstanding_of(&self, who: ObjId) -> impl Iterator<Item = &RequestRec> {
        self.requests
            .values()
            .filter(move |r| r.character == who && r.status.outstanding())
    }

    /// The outstanding requests in a neighborhood.
    pub fn outstanding_in<'a>(
        &'a self,
        neighborhood: &'a RecordKey,
    ) -> impl Iterator<Item = &'a RequestRec> + 'a {
        self.requests
            .values()
            .filter(move |r| &r.neighborhood == neighborhood && r.status.outstanding())
    }

    /// The request that activated `site`, if any.
    #[must_use]
    pub fn request_of_site(&self, site: &RecordKey) -> Option<&RequestRec> {
        self.requests
            .values()
            .find(|r| r.status.site() == Some(site))
    }
}

/// Rebuild the state from the persisted documents, dropping loudly what will not
/// load.
#[must_use]
pub fn hydrate(kv: &[(SmolStr, serde_json::Value)]) -> (HydrationState, Vec<SmolStr>) {
    let mut state = HydrationState::default();
    let mut dropped = Vec::new();
    for (key, value) in kv {
        if let Some(rest) = key.strip_prefix("standing:") {
            match serde_json::from_value::<StandingRec>(value.clone()) {
                Ok(rec) if rec.character.0.to_string() == rest => {
                    state.standing.insert(rec.character, rec);
                }
                Ok(_) => {
                    tracing::warn!(%key, "atlas: a standing document's key disagrees with its character; dropped");
                    dropped.push(key.clone());
                }
                Err(error) => {
                    tracing::warn!(%key, %error, "atlas: a standing document was dropped at boot");
                    dropped.push(key.clone());
                }
            }
        } else if let Some(rest) = key.strip_prefix("request:") {
            match serde_json::from_value::<RequestRec>(value.clone())
                .map_err(|e| e.to_string())
                .and_then(|rec| rec.validate().map(|()| rec))
            {
                Ok(rec) if rec.id == rest && state.requests.len() < MAX_REQUESTS => {
                    state.requests.insert(rec.id.clone(), rec);
                }
                Ok(_) => {
                    tracing::warn!(%key, "atlas: a request document's key disagrees with its id, or the book is full; dropped");
                    dropped.push(key.clone());
                }
                Err(error) => {
                    tracing::warn!(%key, %error, "atlas: a request document was dropped at boot");
                    dropped.push(key.clone());
                }
            }
        }
    }
    (state, dropped)
}

// ---------------------------------------------------------------------------
// Standing
// ---------------------------------------------------------------------------

fn state<'a>(ctx: &'a Ctx<'_>) -> Result<&'a AtlasState, String> {
    ctx.state::<AtlasState>()
        .ok_or_else(|| "The atlas is not composed on this server.".to_string())
}

fn state_mut<'a>(ctx: &'a mut Ctx<'_>) -> Result<&'a mut AtlasState, String> {
    ctx.state_mut::<AtlasState>()
        .ok_or_else(|| "The atlas is not composed on this server.".to_string())
}

/// The cap in force for `who`: their standing's, else the configured default; zero
/// past the strike line until staff reset it.
#[must_use]
pub fn cap_of(st: &AtlasState, who: ObjId) -> u32 {
    let standing = st.hydration.standing.get(&who);
    let strikes = standing.map_or(0, |s| s.strikes);
    if strikes >= st.limits.hydration_strikes_to_zero {
        return 0;
    }
    standing
        .and_then(|s| s.cap)
        .unwrap_or(st.limits.hydration_cap)
}

/// The standing as `+build/status` and `@atlas/standing` print it.
#[must_use]
pub fn standing_lines(st: &AtlasState, w: &World, who: ObjId) -> Vec<String> {
    let name = mush_core::output::display_name(w, who);
    let standing = st.hydration.standing.get(&who);
    let strikes = standing.map_or(0, |s| s.strikes);
    let cap = cap_of(st, who);
    let outstanding: Vec<&RequestRec> = st.hydration.outstanding_of(who).collect();
    let mut lines = vec![format!(
        "{name}: {} of {cap} request slot{} in use, {strikes} strike{}{}.",
        outstanding.len(),
        if cap == 1 { "" } else { "s" },
        if strikes == 1 { "" } else { "s" },
        if strikes >= st.limits.hydration_strikes_to_zero {
            " — no new requests until staff reset the cap"
        } else {
            ""
        }
    )];
    let mut all: Vec<&RequestRec> = st
        .hydration
        .requests
        .values()
        .filter(|r| r.character == who)
        .collect();
    all.sort_by_key(|r| std::cmp::Reverse(r.updated));
    for r in all.iter().take(12) {
        let nb = st
            .records
            .neighborhoods
            .get(&r.neighborhood)
            .map_or_else(|| r.neighborhood.to_string(), |n| n.name.clone());
        lines.push(format!(
            "  {} — {} in {nb}: {}{}",
            r.id,
            r.kind,
            r.status.label(),
            match &r.status {
                RequestStatus::Proposed { proposal } => format!(" (proposal {proposal})"),
                RequestStatus::Activated { job: Some(j), .. }
                | RequestStatus::NeedsChanges { job: Some(j), .. } => format!(" (job {j})"),
                RequestStatus::Struck { reason } | RequestStatus::Withdrawn { reason }
                    if !reason.is_empty() =>
                {
                    format!(": {reason}")
                }
                _ => String::new(),
            }
        ));
    }
    lines
}

/// Staff set a character's cap; the strikes are cleared with it, since a cap set by
/// hand is the reset D-14-25 names.
///
/// # Errors
///
/// A message.
pub fn set_cap(ctx: &mut Ctx<'_>, who: ObjId, cap: u32) -> Result<StandingRec, String> {
    let rec = {
        let st = state_mut(ctx)?;
        let entry = st
            .hydration
            .standing
            .entry(who)
            .or_insert_with(|| StandingRec {
                character: who,
                cap: None,
                strikes: 0,
                revision: 0,
            });
        entry.cap = Some(cap);
        entry.strikes = 0;
        entry.revision = entry.revision.saturating_add(1);
        entry.clone()
    };
    put_doc(ctx, standing_key(who), &rec);
    Ok(rec)
}

fn add_strike(ctx: &mut Ctx<'_>, who: ObjId) -> Result<StandingRec, String> {
    let rec = {
        let st = state_mut(ctx)?;
        let entry = st
            .hydration
            .standing
            .entry(who)
            .or_insert_with(|| StandingRec {
                character: who,
                cap: None,
                strikes: 0,
                revision: 0,
            });
        entry.strikes = entry.strikes.saturating_add(1);
        entry.revision = entry.revision.saturating_add(1);
        entry.clone()
    };
    put_doc(ctx, standing_key(who), &rec);
    Ok(rec)
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// What a player asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The site kind (`site.park`).
    pub kind: String,
    /// The footprint, or `None` for an unclaimed fitting one.
    pub footprint: Option<Polygon2>,
    /// The brief.
    pub prose: String,
}

/// What [`request`] came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Requested {
    /// A new request holds its slot, its unit and its footprint.
    New(RequestRec),
    /// An outstanding request of the same character for the same site: nothing was
    /// charged twice.
    Converged(RequestRec),
}

impl Requested {
    /// The request either way.
    #[must_use]
    pub fn record(&self) -> &RequestRec {
        match self {
            Requested::New(r) | Requested::Converged(r) => r,
        }
    }
}

/// The request id: an idempotency key over the character, the neighborhood pack,
/// the kind and a hash of the footprint.
///
/// # Errors
///
/// A message, when the parts will not key.
pub fn derive_id(
    who: ObjId,
    neighborhood: &CanonicalId,
    kind: &str,
    footprint: &Polygon2,
) -> Result<String, String> {
    let value = serde_json::to_value(footprint).map_err(|e| e.to_string())?;
    let digest =
        DefinitionDigest::for_canonical_value_with_limit(FOOTPRINT_DOMAIN, &value, 64 * 1024)
            .map_err(|e| e.to_string())?
            .to_hex();
    let short: String = digest.chars().take(12).collect();
    let kind: String = kind
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_')
        .collect();
    let id = format!("hyd:{}:{}:{kind}:{short}", who.0, neighborhood);
    IdempotencyKeyText::parse_canonical(&id)
        .map(|k| k.as_str().to_string())
        .map_err(|e| e.to_string())
}

/// Ask for a site. Reserves the character's slot, the neighborhood's unit and the
/// footprint, or says why not; converges on the same character's outstanding request
/// for the same site.
///
/// # Errors
///
/// The refusal, as the player reads it.
pub fn request(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    neighborhood: &RecordKey,
    placement: Placement,
) -> Result<Requested, String> {
    let now = ctx.w.now();
    let rec = {
        let st = state(ctx)?;
        let Some(nb) = st.records.neighborhoods.get(neighborhood) else {
            return Err("That neighborhood is not on the atlas.".to_string());
        };
        if !nb.hydration.enabled {
            return Err(format!(
                "{} does not take player-built sites; ask staff, or build where the policy allows it.",
                nb.name
            ));
        }
        let kind = placement.kind.trim().to_lowercase();
        if kind.is_empty() || kind.len() > crate::record::MAX_WORD_BYTES {
            return Err("Name a site kind, like site.park.".to_string());
        }
        if placement.prose.len() > MAX_PROSE_BYTES {
            return Err(format!("Keep the brief under {MAX_PROSE_BYTES} bytes."));
        }
        let footprint = match placement.footprint.clone() {
            Some(f) => f,
            None => free_ground(st, nb, DEFAULT_SITE_M).ok_or_else(|| {
                format!(
                    "No unclaimed ground of {DEFAULT_SITE_M} m square is left in {}; name a footprint, or build elsewhere.",
                    nb.name
                )
            })?,
        };
        let shape = Shape::Polygon(
            footprint
                .to_cm()
                .map_err(|e| format!("The footprint: {e}"))?,
        );
        let nb_poly = nb.footprint.to_cm().map_err(|e| e.to_string())?;
        if !shape.ring().points().iter().all(|p| nb_poly.contains(*p)) {
            return Err(format!("That footprint leaves {}.", nb.name));
        }
        let id = derive_id(who, &nb.key.pack, &kind, &footprint)?;
        if let Some(existing) = st.hydration.requests.get(&id)
            && existing.status.outstanding()
        {
            return Ok(Requested::Converged(existing.clone()));
        }
        // The character's slot.
        let cap = cap_of(st, who);
        let outstanding: Vec<String> = st
            .hydration
            .outstanding_of(who)
            .map(|r| r.id.clone())
            .collect();
        if outstanding.len() >= usize::try_from(cap).unwrap_or(usize::MAX) {
            return Err(if cap == 0 {
                "Your standing allows no requests until staff reset it. See +build/status."
                    .to_string()
            } else {
                format!(
                    "Your standing allows {cap} outstanding request{}; you have {} ({}). See +build/status.",
                    if cap == 1 { "" } else { "s" },
                    outstanding.len(),
                    outstanding.join(", ")
                )
            });
        }
        // The footprint: nothing standing and no other request under it.
        for other in st.hydration.outstanding_in(neighborhood) {
            if let Ok(poly) = other.footprint.to_cm()
                && overlap(&shape, &Shape::Polygon(poly), nb.resolution_cm)
            {
                if other.character == who && other.kind == kind {
                    return Ok(Requested::Converged(other.clone()));
                }
                return Err(format!(
                    "That ground is spoken for: request {} ({}) is already {} there.",
                    other.id,
                    mush_core::output::display_name(ctx.w, other.character),
                    other.status.label()
                ));
            }
        }
        for site in st.records.sites_in(neighborhood) {
            if let Ok(poly) = site.footprint.to_cm()
                && overlap(&shape, &Shape::Polygon(poly), nb.resolution_cm)
            {
                return Err(format!("That ground is {}'s ({}).", site.name, site.key));
            }
        }
        for room in st.records.rooms_in(neighborhood) {
            if let Ok(other) = room.footprint.shape()
                && overlap(&shape, &other, nb.resolution_cm)
            {
                return Err(format!(
                    "That ground is under {}.",
                    mush_core::output::display_name(ctx.w, room.room)
                ));
            }
        }
        // The neighborhood's unit.
        let standing_sites = st
            .records
            .sites_in(neighborhood)
            .filter(|s| matches!(s.origin, SiteOrigin::Hydrated { .. }))
            .count();
        let generating = st
            .hydration
            .outstanding_in(neighborhood)
            .filter(|r| r.status.site().is_none())
            .count();
        let allowed = usize::try_from(nb.hydration.max_player_sites).unwrap_or(usize::MAX);
        if standing_sites.saturating_add(generating) >= allowed {
            return Err(format!(
                "{} holds its allowance of {} player-built site{}; build elsewhere, or wait for a review.",
                nb.name,
                nb.hydration.max_player_sites,
                if nb.hydration.max_player_sites == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if st.hydration.requests.len() >= MAX_REQUESTS {
            return Err("The request book is full; ask staff.".to_string());
        }
        RequestRec {
            id,
            character: who,
            neighborhood: neighborhood.clone(),
            kind,
            footprint,
            prose: placement.prose.trim().to_string(),
            status: RequestStatus::Generating,
            pack: None,
            created: now,
            updated: now,
            revision: 1,
        }
    };
    put_doc(ctx, request_key(&rec.id), &rec);
    if let Ok(st) = state_mut(ctx) {
        st.hydration.requests.insert(rec.id.clone(), rec.clone());
    }
    Ok(Requested::New(rec))
}

/// An unclaimed square of `side` metres in the neighborhood: the first, scanning
/// from the south-west corner in [`SEARCH_STEP_M`] steps, that lies inside the
/// footprint and overlaps no site, room or outstanding request.
fn free_ground(
    st: &AtlasState,
    nb: &crate::record::NeighborhoodRec,
    side: f64,
) -> Option<Polygon2> {
    let poly = nb.footprint.to_cm().ok()?;
    let bbox = poly.bbox();
    let taken: Vec<Shape> = st
        .records
        .sites_in(&nb.key)
        .filter_map(|s| s.footprint.to_cm().ok().map(Shape::Polygon))
        .chain(
            st.records
                .rooms_in(&nb.key)
                .filter_map(|r| r.footprint.shape().ok()),
        )
        .chain(
            st.hydration
                .outstanding_in(&nb.key)
                .filter_map(|r| r.footprint.to_cm().ok().map(Shape::Polygon)),
        )
        .collect();
    let x0 = crate::geom::metres(bbox.min.x);
    let y0 = crate::geom::metres(bbox.min.y);
    let x1 = crate::geom::metres(bbox.max.x);
    let y1 = crate::geom::metres(bbox.max.y);
    let mut y = y0;
    while y + side <= y1 {
        let mut x = x0;
        while x + side <= x1 {
            let rect = Rect2(Point2(x, y), Point2(x + side, y + side));
            if let Ok(candidate) = rect.to_polygon().to_cm() {
                let inside = candidate.points().iter().all(|p| poly.contains(*p));
                let shape = Shape::Polygon(candidate);
                if inside && !taken.iter().any(|t| overlap(&shape, t, nb.resolution_cm)) {
                    return Some(rect.to_polygon());
                }
            }
            x += SEARCH_STEP_M;
        }
        y += SEARCH_STEP_M;
    }
    None
}

fn update_request(
    ctx: &mut Ctx<'_>,
    id: &str,
    mutate: impl FnOnce(&mut RequestRec),
) -> Result<RequestRec, String> {
    let now = ctx.w.now();
    let rec = {
        let st = state_mut(ctx)?;
        let rec = st
            .hydration
            .requests
            .get_mut(id)
            .ok_or_else(|| format!("No request {id}."))?;
        mutate(rec);
        rec.updated = now;
        rec.revision = rec.revision.saturating_add(1);
        rec.clone()
    };
    put_doc(ctx, request_key(id), &rec);
    Ok(rec)
}

/// The planner's proposal stands for this request.
///
/// # Errors
///
/// A message.
pub fn set_proposal(ctx: &mut Ctx<'_>, id: &str, proposal: u64) -> Result<RequestRec, String> {
    let current = state(ctx)?
        .hydration
        .requests
        .get(id)
        .ok_or_else(|| format!("No request {id}."))?
        .status
        .clone();
    if !matches!(
        current,
        RequestStatus::Generating | RequestStatus::Proposed { .. }
    ) {
        return Err(format!(
            "Request {id} is {}; it takes no proposal.",
            current.label()
        ));
    }
    update_request(ctx, id, |r| r.status = RequestStatus::Proposed { proposal })
}

/// Withdraw a request before it is activated: the character's own, or staff's call.
///
/// # Errors
///
/// A message.
pub fn withdraw(
    ctx: &mut Ctx<'_>,
    id: &str,
    who: ObjId,
    reason: &str,
) -> Result<RequestRec, String> {
    let (owner, current) = {
        let st = state(ctx)?;
        let rec = st
            .hydration
            .requests
            .get(id)
            .ok_or_else(|| format!("No request {id}."))?;
        (rec.character, rec.status.clone())
    };
    if owner != who && !crate::module::is_staff(ctx.w, who) {
        return Err(format!("Request {id} is not yours."));
    }
    if !matches!(
        current,
        RequestStatus::Generating | RequestStatus::Proposed { .. }
    ) {
        return Err(format!(
            "Request {id} is {}; a site that stands is reviewed, not withdrawn.",
            current.label()
        ));
    }
    update_request(ctx, id, |r| {
        r.status = RequestStatus::Withdrawn {
            reason: reason.trim().to_string(),
        };
    })
}

// ---------------------------------------------------------------------------
// Activation and review
// ---------------------------------------------------------------------------

/// What [`activate`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activation {
    /// The request as it now stands.
    pub request: String,
    /// The site.
    pub site: RecordKey,
    /// The BUILD ticket, when the jobs module is composed.
    pub job: Option<u64>,
    /// How many entries the pack made.
    pub created: usize,
}

/// Apply the planner's `site` pack for `id`: the sites are marked hydrated and
/// pending review, a BUILD ticket goes to the reviewers, a canon event records it.
///
/// # Errors
///
/// A message; when the apply stopped part way the pack is latched honestly and the
/// message says so.
pub fn activate(ctx: &mut Ctx<'_>, id: &str, pack: &Pack) -> Result<Activation, String> {
    let (rec, nb_name, reviewers) = {
        let st = state(ctx)?;
        let rec = st
            .hydration
            .requests
            .get(id)
            .ok_or_else(|| format!("No request {id}."))?
            .clone();
        let nb = st
            .records
            .neighborhoods
            .get(&rec.neighborhood)
            .ok_or_else(|| "The neighborhood is gone from the atlas.".to_string())?;
        (rec, nb.name.clone(), nb.reviewers.clone())
    };
    if !matches!(
        rec.status,
        RequestStatus::Generating | RequestStatus::Proposed { .. }
    ) {
        return Err(format!(
            "Request {id} is {}; it is not waiting to be built.",
            rec.status.label()
        ));
    }
    if pack.manifest().kind() != PackKind::Site {
        return Err("A hydration activates a site pack.".to_string());
    }
    if !pack
        .manifest()
        .requires()
        .iter()
        .any(|r| r.id == rec.neighborhood.pack)
    {
        return Err(format!(
            "The pack does not require {}; a site pack adds to the neighborhood it requires.",
            rec.neighborhood.pack
        ));
    }
    let (applied, report) =
        mush_content::module::apply_pack(ctx, pack).map_err(|e| e.to_string())?;
    if let Some(anomaly) = &report.anomaly {
        return Err(format!(
            "The site stopped part way ({anomaly}); what was made is latched under {}. Fix the pack and go again.",
            applied.pack()
        ));
    }
    // The sites the pack made: hydrated, pending review.
    let pack_id = pack.manifest().id().clone();
    let mut sites: Vec<RecordKey> = Vec::new();
    let docs: Vec<(SmolStr, crate::record::SiteRec)> = {
        let st = state_mut(ctx)?;
        let mut docs = Vec::new();
        for site in st.records.sites.values_mut() {
            if site.key.pack != pack_id {
                continue;
            }
            site.origin = SiteOrigin::Hydrated {
                request: id.to_string(),
            };
            site.review = ReviewState::PendingReview;
            site.revision = site.revision.saturating_add(1);
            sites.push(site.key.clone());
            docs.push((record_key(KIND_SITE, &site.key), site.clone()));
        }
        docs
    };
    for (key, site) in &docs {
        put_doc(ctx, key.clone(), site);
    }
    let site = sites
        .first()
        .cloned()
        .ok_or_else(|| "The pack made no site.".to_string())?;
    let site_name = state(ctx)?
        .records
        .sites
        .get(&site)
        .map_or_else(|| site.to_string(), |s| s.name.clone());
    let who_name = mush_core::output::display_name(ctx.w, rec.character);
    // The BUILD ticket, when there is a jobs module to file it in.
    let job = mush_jobs::open_job(
        ctx,
        OpenJob {
            queue: SmolStr::new_static(BUILD_QUEUE),
            title: format!("Review {site_name} by {who_name} in {nb_name}"),
            opener: rec.character,
            text: Some(format!(
                "{} asked for a {} in {nb_name}: {}",
                who_name,
                rec.kind,
                if rec.prose.is_empty() {
                    "(no brief)"
                } else {
                    rec.prose.as_str()
                }
            )),
            payload: Some(JobPayload {
                handler: SmolStr::new_static(BUILD_HANDLER),
                data: serde_json::json!({
                    "request": id,
                    "pack_digest": pack.manifest().digest().to_hex(),
                    "pack": pack_id.as_str(),
                    "neighborhood": rec.neighborhood.to_string(),
                    "site": site.to_string(),
                    "reviewers": reviewers.iter().map(|r| r.0).collect::<Vec<_>>(),
                    "schema_version": REQUEST_SCHEMA_VERSION,
                }),
            }),
            tags: vec![SmolStr::new_static("build")],
        },
    )
    .map_err(|e| {
        tracing::debug!(error = %e, "atlas: no BUILD ticket for the activation");
        e
    })
    .ok();
    let site_for_status = site.clone();
    update_request(ctx, id, |r| {
        r.status = RequestStatus::Activated {
            site: site_for_status,
            job,
        };
        r.pack = Some(pack_id.clone());
    })?;
    event(
        ctx,
        "atlas.site.activated",
        format!("{site_name} was built by {who_name} in {nb_name}, pending review"),
        vec![rec.character],
        id,
        serde_json::json!({ "site": site.to_string(), "pack": pack_id.as_str(), "job": job }),
    );
    if ctx.w.get(rec.character).is_some() {
        ctx.out.info(
            rec.character,
            format!(
                "{site_name} stands in {nb_name}, pending review{}.",
                job.map(|j| format!(" (job {j})")).unwrap_or_default()
            ),
        );
    }
    Ok(Activation {
        request: id.to_string(),
        site,
        job,
        created: report.created,
    })
}

/// The request a BUILD ticket carries.
fn request_of_job(job: &Job) -> Option<String> {
    let payload = job.payload.as_ref()?;
    if payload.handler.as_str() != BUILD_HANDLER {
        return None;
    }
    payload
        .data
        .get("request")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

fn set_site_review(ctx: &mut Ctx<'_>, site: &RecordKey, review: ReviewState) -> Option<String> {
    let doc = {
        let st = state_mut(ctx).ok()?;
        let rec = st.records.sites.get_mut(site)?;
        rec.review = review;
        rec.revision = rec.revision.saturating_add(1);
        (record_key(KIND_SITE, site), rec.clone())
    };
    let name = doc.1.name.clone();
    put_doc(ctx, doc.0, &doc.1);
    Some(name)
}

/// `+job/approve` on a BUILD ticket: the site is kept, the slot released.
///
/// # Errors
///
/// A message; the approval is left undone.
pub fn review_approve(ctx: &mut Ctx<'_>, job: &Job) -> Result<(), String> {
    let id = request_of_job(job).ok_or_else(|| "not a BUILD review".to_string())?;
    let rec = state(ctx)?
        .hydration
        .requests
        .get(&id)
        .ok_or_else(|| format!("request {id} is gone"))?
        .clone();
    let site = match &rec.status {
        RequestStatus::Activated { site, .. } | RequestStatus::NeedsChanges { site, .. } => {
            site.clone()
        }
        other => {
            return Err(format!(
                "request {id} is {}; there is nothing to approve",
                other.label()
            ));
        }
    };
    let name = set_site_review(ctx, &site, ReviewState::Approved)
        .ok_or_else(|| format!("site {site} is gone from the atlas"))?;
    let site_for_status = site.clone();
    update_request(ctx, &id, |r| {
        r.status = RequestStatus::Approved {
            site: site_for_status,
        };
    })?;
    let who = mush_core::output::display_name(ctx.w, ctx.actor);
    event(
        ctx,
        "atlas.site.approved",
        format!("{name} was approved by {who}"),
        vec![rec.character, ctx.actor],
        &id,
        serde_json::json!({ "site": site.to_string(), "job": job.id }),
    );
    if ctx.w.get(rec.character).is_some() && rec.character != ctx.actor {
        ctx.out
            .info(rec.character, format!("{name} was approved by {who}."));
    }
    Ok(())
}

/// The status observer: a BUILD ticket denied is changes requested — the site
/// stays live, the slot stays held.
pub fn on_job_status(ctx: &mut Ctx<'_>, job: &Job, previous: JobStatus) {
    let Some(id) = request_of_job(job) else {
        return;
    };
    if job.status != JobStatus::Denied || previous == JobStatus::Denied {
        return;
    }
    let Some(rec) = state(ctx)
        .ok()
        .and_then(|st| st.hydration.requests.get(&id).cloned())
    else {
        return;
    };
    let RequestStatus::Activated { site, .. } = &rec.status else {
        return;
    };
    let site = site.clone();
    let Some(name) = set_site_review(ctx, &site, ReviewState::NeedsChanges) else {
        return;
    };
    let job_id = job.id;
    let site_for_status = site.clone();
    if update_request(ctx, &id, |r| {
        r.status = RequestStatus::NeedsChanges {
            site: site_for_status,
            job: Some(job_id),
        };
    })
    .is_err()
    {
        return;
    }
    let who = mush_core::output::display_name(ctx.w, ctx.actor);
    let reason = job
        .comments
        .last()
        .map(|c| c.text.clone())
        .unwrap_or_default();
    event(
        ctx,
        "atlas.site.changes_requested",
        format!("{who} asked for changes to {name}"),
        vec![rec.character, ctx.actor],
        &id,
        serde_json::json!({ "site": site.to_string(), "job": job.id, "reason": reason }),
    );
    if ctx.w.get(rec.character).is_some() && rec.character != ctx.actor {
        ctx.out.info(
            rec.character,
            format!("{who} asked for changes to {name}: {reason}"),
        );
    }
}

/// What [`strike`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrikeReport {
    /// The request.
    pub request: String,
    /// The character.
    pub character: ObjId,
    /// Their strikes now.
    pub strikes: u32,
    /// Their cap now.
    pub cap: u32,
    /// The pack retired.
    pub pack: CanonicalId,
}

/// Strike a hydrated site: its pack is retired, the request struck, the character's
/// strikes raised (doc 14 §4.5 step 4).
///
/// # Errors
///
/// A message; a site that is not a hydration, or a pack that will not retire (a
/// room occupied, an exit from outside leading in), refuses with the reason.
pub fn strike(ctx: &mut Ctx<'_>, site: &RecordKey, reason: &str) -> Result<StrikeReport, String> {
    let (rec, name) = {
        let st = state(ctx)?;
        let site_rec = st
            .records
            .sites
            .get(site)
            .ok_or_else(|| format!("No site {site} on the atlas."))?;
        let SiteOrigin::Hydrated { request } = &site_rec.origin else {
            return Err(format!(
                "{} is staff-authored; retire its pack with @atlas/pack retire.",
                site_rec.name
            ));
        };
        let rec = st
            .hydration
            .requests
            .get(request)
            .ok_or_else(|| format!("The request behind {} is gone.", site_rec.name))?
            .clone();
        (rec, site_rec.name.clone())
    };
    let reason = reason.trim();
    if reason.is_empty() {
        return Err("A strike carries a reason.".to_string());
    }
    let pack = rec.pack.clone().unwrap_or_else(|| site.pack.clone());
    mush_content::module::retire_pack(ctx, &pack, &format!("struck: {reason}"))
        .map_err(|e| e.to_string())?;
    update_request(ctx, &rec.id, |r| {
        r.status = RequestStatus::Struck {
            reason: reason.to_string(),
        };
    })?;
    let standing = add_strike(ctx, rec.character)?;
    let cap = cap_of(state(ctx)?, rec.character);
    let who = mush_core::output::display_name(ctx.w, ctx.actor);
    event(
        ctx,
        "atlas.site.struck",
        format!("{name} was struck by {who}: {reason}"),
        vec![rec.character, ctx.actor],
        &rec.id,
        serde_json::json!({ "site": site.to_string(), "pack": pack.as_str(), "strikes": standing.strikes }),
    );
    event(
        ctx,
        "atlas.site.retired",
        format!("{name} is gone"),
        vec![rec.character],
        &rec.id,
        serde_json::json!({ "site": site.to_string(), "pack": pack.as_str(), "reason": reason }),
    );
    if ctx.w.get(rec.character).is_some() {
        ctx.out.info(
            rec.character,
            format!(
                "{name} was struck by {who}: {reason}. You have {} strike{}{}.",
                standing.strikes,
                if standing.strikes == 1 { "" } else { "s" },
                if cap == 0 {
                    "; no new requests until staff reset your cap"
                } else {
                    ""
                }
            ),
        );
    }
    Ok(StrikeReport {
        request: rec.id,
        character: rec.character,
        strikes: standing.strikes,
        cap,
        pack,
    })
}

/// A pack was retired by staff: the requests it stood for are withdrawn and the
/// retirement recorded.
pub fn note_pack_retired(ctx: &mut Ctx<'_>, pack: &CanonicalId, reason: &str) {
    let affected: Vec<RequestRec> = state(ctx)
        .map(|st| {
            st.hydration
                .requests
                .values()
                .filter(|r| r.pack.as_ref() == Some(pack) && r.status.outstanding())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for rec in affected {
        let site = rec.status.site().cloned();
        let _ = update_request(ctx, &rec.id, |r| {
            r.status = RequestStatus::Withdrawn {
                reason: format!("retired: {reason}"),
            };
        });
        event(
            ctx,
            "atlas.site.retired",
            format!(
                "a site built by {} was retired: {reason}",
                mush_core::output::display_name(ctx.w, rec.character)
            ),
            vec![rec.character],
            &rec.id,
            serde_json::json!({ "site": site.map(|s| s.to_string()), "pack": pack.as_str() }),
        );
    }
}

/// Record one canon event for a request, when the canon module is composed.
fn event(
    ctx: &mut Ctx<'_>,
    kind: &'static str,
    summary: String,
    entities: Vec<ObjId>,
    request: &str,
    detail: serde_json::Value,
) {
    let ic_time = ctx.w.world_ic_now().at;
    let _ = mush_canon::commit(
        ctx,
        CanonDraft {
            kind: SmolStr::new_static(kind),
            summary,
            entities,
            scope: CanonScope::Public,
            ic_time,
            source: format!("{SOURCE_PREFIX}{request}"),
            detail,
        },
    );
}

/// Register the BUILD payload handler and the status observer with the jobs module,
/// when it is composed. Called at the atlas's boot.
pub fn register_review(w: &mut World) {
    let handler: mush_jobs::PayloadHandlerFn = Arc::new(review_approve);
    if let Err(error) = mush_jobs::register_payload_handler(w, BUILD_HANDLER, handler) {
        tracing::debug!(%error, "atlas: no BUILD review handler");
    }
    let observer: mush_jobs::StatusObserverFn = Arc::new(on_job_status);
    if let Err(error) = mush_jobs::register_status_observer(w, STATUS_OBSERVER, observer) {
        tracing::debug!(%error, "atlas: no BUILD status observer");
    }
}

/// Whether `who` may review sites in `neighborhood`: a named reviewer, or a Builder.
#[must_use]
pub fn may_review(st: &AtlasState, w: &World, who: ObjId, neighborhood: &RecordKey) -> bool {
    st.records
        .neighborhoods
        .get(neighborhood)
        .is_some_and(|n| n.reviewers.contains(&who))
        || mush_core::perm::has_role(w, who, Role::Builder)
}

// ---------------------------------------------------------------------------
// The hand-off to the planner
// ---------------------------------------------------------------------------

/// The planner's entry, registered by the worldgen module at boot.
pub type ProposeFn =
    Arc<dyn Fn(&mut Ctx<'_>, ProposeRequest) -> Result<String, String> + Send + Sync>;

/// The functions other modules register with the atlas.
#[derive(Clone, Default)]
pub struct AtlasHooks {
    /// `@atlas/propose`'s planner.
    pub propose: Option<ProposeFn>,
}

impl std::fmt::Debug for AtlasHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtlasHooks")
            .field("propose", &self.propose.is_some())
            .finish()
    }
}

/// Where a proposal is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposeScope {
    /// The world: a region pack.
    World,
    /// A region: a neighborhood pack.
    Region(RecordKey),
    /// A neighborhood: sites, rooms, people.
    Neighborhood(RecordKey),
}

/// What a proposal starts from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposeSource {
    /// A brief.
    Prose(String),
    /// A pack on the shelf, reconceptualized (D-14-26).
    FromPack(CanonicalId),
    /// A player's request.
    Request(String),
}

/// A proposal, as `@atlas/propose` or `+build/place` hands it to the planner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposeRequest {
    /// Who asked.
    pub author: ObjId,
    /// Where.
    pub scope: ProposeScope,
    /// From what.
    pub source: ProposeSource,
}

/// Register the planner. Called from the worldgen module's boot.
///
/// # Errors
///
/// A message when the atlas is not composed or a planner is already registered.
pub fn register_proposer(w: &mut World, f: ProposeFn) -> Result<(), String> {
    let hooks = w
        .module_state_mut::<AtlasHooks>()
        .ok_or_else(|| "the atlas module is not loaded".to_string())?;
    if hooks.propose.is_some() {
        return Err("a planner is already registered".to_string());
    }
    hooks.propose = Some(f);
    Ok(())
}

/// Hand a proposal to the planner.
///
/// # Errors
///
/// The planner's refusal, or that none is composed.
pub fn propose(ctx: &mut Ctx<'_>, request: ProposeRequest) -> Result<String, String> {
    let planner = ctx
        .state::<AtlasHooks>()
        .and_then(|h| h.propose.clone())
        .ok_or_else(|| "No planner is composed on this server.".to_string())?;
    planner(ctx, request)
}

/// Every request, for `@atlas/standing` without a name and the planner's views.
#[must_use]
pub fn requests(st: &AtlasState) -> Vec<&RequestRec> {
    let mut all: Vec<&RequestRec> = st.hydration.requests.values().collect();
    all.sort_by_key(|r| std::cmp::Reverse(r.updated));
    all
}

/// Forget a request's document (tests and staff cleanup).
pub fn forget(ctx: &mut Ctx<'_>, id: &str) {
    if let Ok(st) = state_mut(ctx) {
        st.hydration.requests.remove(id);
    }
    del_doc(ctx, request_key(id));
}
