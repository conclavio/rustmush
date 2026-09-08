//! ---- E7r2: P2 — salvage: the stranded lot, recovered by its own custodian ----
//!
//! 10b §15.1 item 4, doc 10 T3/§10. Cut 6 already quarantines a `Failed` or
//! `Captured` carrier root into the recovery room and writes an anomaly, so a lost
//! hull is *recoverable today* — **by staff**, which is exactly what doc 10 §11's
//! "no staff step" forbids. This file is what turns "recoverable by staff" into
//! "recoverable by the owner".
//!
//! # The three things it deliberately is not
//!
//! * **Not an [`InventoryVerb`](crate::InventoryVerb).** The five stay five. This is a
//!   separate action with its own provider and its own gate, registered beside them —
//!   the absence of a player-reachable reserve/release/relocate verb is E7 residue
//!   pass 2's structural lock on a market listing's hold (§15.5's constraint (3)) and
//!   nothing here weakens it. `@inv/move` still cannot touch a lot in a wreck the
//!   actor may not take from, and it still cannot touch a *reserved* one at all.
//! * **Not a mint.** Recovery calls [`Lot::checked_relocate`](crate::record::Lot) —
//!   whose **first caller in the entire workspace** this is — through
//!   [`InventoryFamilies::relocate`](crate::InventoryFamilies). *A recovered lot is the
//!   SAME lot with its provenance intact*: `id`, `commodity`, `quantity`, `grade`,
//!   `lineage` and `depth` are carried unchanged and only `endpoint` and `revision`
//!   move. `mint_origin` is called from nowhere on this path and a source probe pins
//!   it.
//! * **Not a title move.** Doc 10 T3: capture changes **custody or possession, not
//!   title**, and title moves only through T3's closed list. A salvage *claim* is an
//!   E4b2 contract with a `Before` contest deadline and a named adjudicator, and it is
//!   explicitly **not built by this pass**. What this file moves is matter, under E3's
//!   own `may_take_from_vault` custody gate and nothing else.
//!
//! # Where the wreck comes from, and why the seam is at the composition root
//!
//! `mush-inventory → {mush-core, mush-economy}` and that is pinned by
//! `tests/e7c2.rs`. Whether a root is a wreck is `mush-sim`'s fact, so this crate
//! **cannot** ask it: [`SalvageSource`] is a trait declared here and implemented in
//! `mush-server`, the one crate with edges to both. That is why the seam exists and
//! why no `mush-inventory → mush-sim` edge appears.
//!
//! **Unwired is fail-closed.** A composition that names no source registers no salvage
//! action and projects no offer, so `@salvage/recover` on a real wreck reads exactly
//! what `@salvage/recover` on a nonexistent one reads.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalHash, CanonicalId, ClockStamp, DefinitionRef, Engine,
    IdempotencyKey, IdempotencyKeyText, IdempotencyScope, LegalActionQuery, ObjId, PerceivedRef,
    PrincipalId as CorePrincipalId, ProjectedParameter, ProjectedParameterKind,
    ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft, RevisionSet, RulesValue,
    ScopedIdempotencyKey, SubjectKey, World,
};
use mush_economy::VaultId;
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::pool::InventoryFamilies;
use crate::provider::may_take;
use crate::record::{AppliedMarker, EndpointRef, FACT_SALVAGE_RECOVERED, LotId};

/// The one salvage action this crate registers. It is **not** an
/// [`InventoryVerb`](crate::InventoryVerb) and never becomes one.
pub const ACTION_SALVAGE: &str = "inv_salvage";

/// The most wrecks one `@salvage/status` page or one offer's `Choice` list carries.
/// Bounded on records **visited** (D-LBK-5); the walk says so out loud when it stops.
pub const MAX_SALVAGE_WRECKS: usize = 32;

/// The most endpoints one wreck reports. A hull with more holds than this is an
/// operator problem long before it is a salvage problem.
pub const MAX_SALVAGE_ENDPOINTS: usize = 16;

/// The most `<wreck>/<endpoint>` sites one offer publishes — the product of the two
/// caps above would be unreadable, so the offer's own list is capped in its own right.
pub const MAX_SALVAGE_SITES: usize = 64;

/// **One refusal for every reason a salvage can fail** (§4 rule 4).
///
/// A captor, a stranger, an actor naming a wreck that does not exist and an actor
/// naming a lot that is not in one all read this line. The prepared path is therefore
/// not an existence oracle for another principal's losses.
pub const NO_SUCH_WRECK: &str = "There is no such wreck.";

/// What the composition root knows about one recoverable wreck.
///
/// Bounded by construction: [`WreckReport::new`] truncates nothing — it **refuses** an
/// endpoint list past [`MAX_SALVAGE_ENDPOINTS`], because a silently dropped endpoint is
/// stock its owner is told does not exist.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WreckReport {
    root: ObjId,
    captured_by: Option<ObjId>,
    site: ObjId,
    endpoints: Vec<EndpointRef>,
}

impl WreckReport {
    /// Build one report.
    ///
    /// # Errors
    /// `None` for an endpoint list past [`MAX_SALVAGE_ENDPOINTS`], or a zero root.
    #[must_use]
    pub fn new(
        root: ObjId,
        captured_by: Option<ObjId>,
        site: ObjId,
        mut endpoints: Vec<EndpointRef>,
    ) -> Option<WreckReport> {
        if root.0 == 0 || endpoints.len() > MAX_SALVAGE_ENDPOINTS {
            return None;
        }
        endpoints.sort_unstable();
        endpoints.dedup();
        Some(WreckReport {
            root,
            captured_by,
            site,
            endpoints,
        })
    }

    /// The carrier root that was lost.
    #[must_use]
    pub const fn root(&self) -> ObjId {
        self.root
    }
    /// Who took it, when it was taken rather than merely lost. **Never rendered to a
    /// principal who is not the wreck's own custodian**, and never a gate: a captor's
    /// authority over the cargo is `may_take_from_vault`'s answer and nothing else.
    #[must_use]
    pub const fn captured_by(&self) -> Option<ObjId> {
        self.captured_by
    }
    /// Where the hull is now — cut 6's `recovery_room`.
    #[must_use]
    pub const fn site(&self) -> ObjId {
        self.site
    }
    /// The inventory endpoints that belong to it.
    #[must_use]
    pub fn endpoints(&self) -> &[EndpointRef] {
        &self.endpoints
    }
}

/// **What `mush-server` tells this crate about wrecks** (E7r2 P2).
///
/// Implemented at the composition root because answering it needs `mush-sim`'s
/// `JourneyBook` and `SpatialState`, and this crate has no edge to either — see the
/// module docs. Every method is a pure read over `&World`: no clock, no RNG, no
/// mutation, and a source that cannot answer answers `None`/empty rather than
/// guessing, which is the fail-closed direction.
pub trait SalvageSource: Send + Sync + std::fmt::Debug {
    /// Every recoverable wreck in the world, in object order, at most `budget`.
    fn wrecks(&self, world: &World, budget: usize) -> Vec<ObjId>;

    /// One wreck's report, or `None` when `root` is not a recoverable wreck.
    fn wreck(&self, world: &World, root: ObjId) -> Option<WreckReport>;
}

/// The composed source, published in module state so a command can reach it
/// synchronously. `None` is the fail-closed composition.
#[derive(Clone, Default)]
pub struct SalvageHandle(pub Option<Arc<dyn SalvageSource>>);

impl std::fmt::Debug for SalvageHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SalvageHandle")
            .field("wired", &self.0.is_some())
            .finish()
    }
}

/// The `salvage/{root}` holder text, asserted once to be inside K's
/// `MAX_IDEMPOTENCY_KEY_BYTES` (128).
#[must_use]
pub fn salvage_holder_text(root: ObjId) -> String {
    format!("salvage/{}", root.0)
}

/// The **deterministic holder key** every recovery from one wreck carries.
///
/// `mush_industry::wo_holder` / `mush_market::listing_holder`'s shape exactly — this is
/// the pattern's third consumer, and it copies rather than inventing a third key
/// shape: a `PrincipalId::Scoped` under this crate's own `OwnerId`, class `"salvage"`,
/// id `salvage{root:08x}`, with a deterministic `CanonicalHash` over `{"root": root}`.
///
/// # Errors
/// [`crate::InventoryError::Identity`] if the key cannot be built.
pub fn salvage_holder(root: ObjId) -> Result<ScopedIdempotencyKey, crate::InventoryError> {
    let id = IdempotencyKeyText::parse_canonical(format!("salvage{:08x}", root.0))
        .map_err(|_| crate::InventoryError::Identity)?;
    let principal = CorePrincipalId::Scoped {
        owner: crate::record::owner(),
        class: CanonicalId::new("salvage").map_err(|_| crate::InventoryError::Identity)?,
        id,
    };
    let scope = IdempotencyScope {
        principal,
        owner: crate::record::owner(),
        namespace: CanonicalId::new(crate::record::NS)
            .map_err(|_| crate::InventoryError::Identity)?,
    };
    let key = CanonicalHash::for_canonical_value(
        "inventory.salvage_holder.v1",
        1,
        &json!({ "root": root.0 }),
        1024,
    )
    .map_err(|_| crate::InventoryError::Identity)?;
    Ok(ScopedIdempotencyKey::new(
        scope,
        0,
        IdempotencyKey::Deterministic(key),
    ))
}

/// **The one authority answer, and every reader asks this** (§4 rule 4's other half:
/// one internal view feeds every projection).
///
/// The `(wreck, endpoint)` sites this actor may recover from: an endpoint that belongs
/// to a recoverable wreck **and** that `mush_economy::may_take_from_vault` answers true
/// for — the owner via `acts_for`, staff, or a `vault.withdraw` capability holder in a
/// coherent scope. Nothing else is consulted.
///
/// **A captor gets nothing from this list**, and that is the whole of OPEN 4's answer:
/// capture moves custody or possession, never title (doc 10 T3), so until custody moves
/// through E3's own path the captor is not `may_take_from_vault` over the cargo's
/// vault and the wreck is, to them, byte-identically absent.
///
/// Bounded on records **visited**: at most [`MAX_SALVAGE_WRECKS`] wrecks, each
/// reporting at most [`MAX_SALVAGE_ENDPOINTS`] endpoints, and the result truncated at
/// [`MAX_SALVAGE_SITES`] — the second return says whether it was.
#[must_use]
pub fn sites(
    world: &World,
    source: &dyn SalvageSource,
    actor: ObjId,
) -> (Vec<(ObjId, EndpointRef)>, bool) {
    let mut out = Vec::new();
    let mut truncated = false;
    for root in source.wrecks(world, MAX_SALVAGE_WRECKS) {
        let Some(report) = source.wreck(world, root) else {
            continue;
        };
        for endpoint in report.endpoints() {
            if !may_take(world, actor, *endpoint) {
                continue;
            }
            if out.len() >= MAX_SALVAGE_SITES {
                truncated = true;
                return (out, truncated);
            }
            out.push((root, *endpoint));
        }
    }
    (out, truncated)
}

/// The `<wreck>/<endpoint>` token one site is named by — one parameter carrying both
/// halves, so a request cannot name a wreck and an endpoint that disagree.
#[must_use]
pub fn site_token(root: ObjId, endpoint: EndpointRef) -> String {
    format!("{}/{}", root.0, endpoint.vault().0)
}

/// [`site_token`] read back.
#[must_use]
pub fn parse_site(text: &str) -> Option<(ObjId, EndpointRef)> {
    let (root, endpoint) = text.trim().split_once('/')?;
    Some((
        ObjId(root.parse::<u32>().ok()?),
        EndpointRef(VaultId(endpoint.parse::<u64>().ok()?)),
    ))
}

// ---- the prepared path -----------------------------------------------------

fn error(code: ActionErrorCode, message: impl Into<String>) -> ActionError {
    ActionError::new(code, false, message).expect("bounded salvage action error")
}

fn refuse() -> ActionError {
    error(ActionErrorCode::ProviderRejected, NO_SUCH_WRECK)
}

fn parameter(
    id: &str,
    required: bool,
    kind: ProjectedParameterKind,
) -> Result<ProjectedParameter, ActionError> {
    Ok(ProjectedParameter {
        id: CanonicalId::new(id)
            .map_err(|_| error(ActionErrorCode::Integrity, "salvage parameter id"))?,
        required,
        kind,
    })
}

fn text_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, ActionError> {
    parameters
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| error(ActionErrorCode::InvalidRequest, NO_SUCH_WRECK))
}

/// The IC stamp the preparation uses, off the offer's own FIXED parameters — never a
/// wall clock, so two replays of one request stamp the identical marker.
fn prepare_now(parameters: &serde_json::Map<String, Value>) -> Result<ClockStamp, ActionError> {
    parameters
        .get("now")
        .and_then(|value| serde_json::from_value::<ClockStamp>(value.clone()).ok())
        .ok_or_else(|| {
            error(
                ActionErrorCode::Unavailable,
                "the world clock is unavailable",
            )
        })
}

/// The salvage action's provider. It holds the [`SalvageSource`] because **authority
/// lives in `project_offer`** — the seam with an `&Engine` — exactly as
/// `mush_industry::provider`'s `ClaimContest` and `mush_market::lot` do:
/// `prepare_authoritative` gets no world and can read no wreck.
pub(crate) struct SalvageProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: InventoryFamilies,
    source: Arc<dyn SalvageSource>,
}

impl SalvageProvider {
    fn parameters<'i>(
        &self,
        invocation: &'i ActionInvocation,
    ) -> Result<&'i serde_json::Map<String, Value>, ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(error(ActionErrorCode::InvalidRequest, NO_SUCH_WRECK));
        }
        invocation
            .parameters()
            .as_object()
            .ok_or_else(|| error(ActionErrorCode::InvalidRequest, NO_SUCH_WRECK))
    }
}

impl ActionProvider for SalvageProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = self.parameters(invocation)?;
        parse_site(&text_field(parameters, "site")?)
            .map(|_| ())
            .ok_or_else(|| error(ActionErrorCode::InvalidRequest, NO_SUCH_WRECK))
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        // **The whole authority of the lane, in one list.** An actor who may take from
        // no wreck endpoint gets **no offer at all** — so a captor, a stranger and an
        // actor naming a wreck that never existed are refused identically, and the
        // refusal is byte-identical before and after a capture because capture does
        // not change `may_take_from_vault`'s answer for anybody.
        let (sites, _) = sites(&engine.world, self.source.as_ref(), query.actor());
        if sites.is_empty() {
            return Ok(None);
        }
        // The destination is one of the actor's OWN live endpoints. Recovery moves the
        // lot out of the wreck and into somewhere the same principal already holds; it
        // never creates a vault and never touches title.
        let destinations = crate::provider::live_endpoints(&engine.world, query.actor());
        if destinations.is_empty() {
            return Ok(None);
        }
        let parameters = vec![
            parameter(
                "site",
                true,
                ProjectedParameterKind::Choice {
                    choices: sites
                        .iter()
                        .map(|(root, endpoint)| site_token(*root, *endpoint))
                        .collect(),
                },
            )?,
            parameter(
                "to",
                true,
                ProjectedParameterKind::Choice {
                    choices: destinations.iter().map(|id| id.0.to_string()).collect(),
                },
            )?,
            parameter(
                "lot",
                true,
                ProjectedParameterKind::Text { maximum_bytes: 32 },
            )?,
        ];
        let definitions = engine.registry.definitions().ok_or_else(|| {
            error(
                ActionErrorCode::Integrity,
                "inventory definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "inventory guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "inventory definition guard is unavailable",
                )
            })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({ "now": engine.world.world_ic_now() }),
            parameters: ProjectedParameterSchema::new(parameters).map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "salvage parameter schema is invalid",
                )
            })?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "inv-salvage",
                "Recover stranded stock",
                "Recover a lot stranded in a wreck you may draw from.",
            )
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "salvage presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "salvage offer guards are invalid",
                )
            })?,
        }))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters = self.parameters(invocation)?.clone();
        let now = prepare_now(&parameters)?;
        let (root, from) = parse_site(&text_field(&parameters, "site")?).ok_or_else(refuse)?;
        let to = text_field(&parameters, "to")?
            .parse::<u64>()
            .map(|id| EndpointRef(VaultId(id)))
            .map_err(|_| refuse())?;
        let lot = LotId::parse(&text_field(&parameters, "lot")?).ok_or_else(refuse)?;
        // **The replay guard comes FIRST**, before the one irreversible mutation, and
        // it is the marker rather than an idempotency epoch: a `ScopedIdempotencyKey`
        // carries a per-process random epoch and does not survive a reboot, so a
        // retried recovery after one would otherwise be a second, genuinely new
        // request. The marker id is a pure function of the request and the stamp.
        let marker_id = marker_id(invocation, &now);
        if txn
            .get(&self.families.applied, &marker_id)
            .map_err(|_| error(ActionErrorCode::Integrity, NO_SUCH_WRECK))?
            .is_some()
        {
            return Err(error(
                ActionErrorCode::ProviderRejected,
                "that request has already been applied",
            ));
        }
        // *A recovered lot is the SAME lot.* `relocate` is `Lot::checked_relocate`'s
        // one caller: `id`, `commodity`, `quantity`, `grade`, `lineage` and `depth` are
        // carried through untouched and only `endpoint` and `revision` move. Nothing
        // here mints, splits or merges, so a second recovery of the same lot finds it
        // no longer at `from` and refuses.
        let moved = self
            .families
            .relocate(txn, lot, from, to)
            .map_err(|_| refuse())?;
        // ---- E7r2: P4 — `inventory.salvage.recovered`, inside this bundle ----
        //
        // The third `AppliedMarker` kind, written by the provider that did the work and
        // never from a status (§10's E4c finding). A bundle that did not commit leaves
        // no marker, so the fact answers `None`, which is unsatisfied.
        let marker = AppliedMarker::new(
            FACT_SALVAGE_RECOVERED,
            Some(from),
            Some(to),
            moved.commodity().clone(),
            moved.quantity(),
            invocation.actor(),
            now.at,
        )
        .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        txn.insert(&self.families.applied, &marker_id, marker)
            .map_err(|_| error(ActionErrorCode::Integrity, NO_SUCH_WRECK))?;

        let mut data = BTreeMap::<String, Value>::new();
        data.insert("actor".to_owned(), json!(invocation.actor().0));
        data.insert("verb".to_owned(), json!(ACTION_SALVAGE));
        data.insert("wreck".to_owned(), json!(root.0));
        data.insert("from".to_owned(), json!(from.vault().0));
        data.insert("to".to_owned(), json!(to.vault().0));
        data.insert("lot".to_owned(), json!(moved.id().0));
        data.insert("quantity".to_owned(), json!(moved.quantity()));
        data.insert(
            "summary".to_owned(),
            json!(format!(
                "Recovered {} {} from wreck #{} as the same lot {}.",
                moved.quantity(),
                moved.commodity().as_str(),
                root.0,
                moved.id()
            )),
        );
        let outcome = RulesValue::new(self.outcome.clone(), json!(data)).map_err(|_| {
            error(
                ActionErrorCode::BoundsExceeded,
                "salvage outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// `{at}{request}` — `InventoryProvider::marker_id`'s shape exactly, so canonical key
/// order is time order and the RAM mirror keeps the newest.
fn marker_id(invocation: &ActionInvocation, now: &ClockStamp) -> SmolStr {
    let ordered = (now.at as u64) ^ (1_u64 << 63);
    SmolStr::new(format!("{ordered:016x}{}", invocation.request().to_hex()))
}

/// Build the provider for the composed source, or `None` when nothing was composed.
pub(crate) fn provider_for(
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: &InventoryFamilies,
    source: Option<&Arc<dyn SalvageSource>>,
) -> Option<SalvageProvider> {
    source.map(|source| SalvageProvider {
        action,
        outcome,
        families: families.clone(),
        source: Arc::clone(source),
    })
}

/// The registered `DefinitionRef` for the salvage action, rebuilt from the frozen
/// registry so a command handler can name it without holding a handle. `None` on a
/// composition that wired no source — the fail-closed shape.
#[must_use]
pub fn action_ref(registry: &mush_core::Registry) -> Option<DefinitionRef> {
    let definitions = registry.definitions()?;
    definitions
        .current(&mush_core::DefinitionKey {
            owner: crate::record::owner(),
            kind: mush_core::DefinitionKind::new("action").ok()?,
            id: mush_core::DefinitionId::new(ACTION_SALVAGE).ok()?,
        })
        .cloned()
}
