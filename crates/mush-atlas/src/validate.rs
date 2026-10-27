//! The validator (doc 14 §4.3; S3, 14-spatial-implementation-plan §3.4): one
//! deterministic function over a content pack and a snapshot of the world, the same
//! for a human's pack and the building agent's.
//!
//! `@atlas/pack plan` and the agent's `pack.validate` tool both call [`validate`]
//! with a [`ValidateContext`] — everything the rules need from the world, captured
//! on the world loop by [`ValidateContext::capture`] and serializable, so the agent's
//! off-loop run validates against the world as it stood when the run began
//! (treaty ruling 3) and the plan validates against the world as it stands now.
//!
//! Findings are typed ([`Finding`]): a code, the entry, a [`Severity`], a message
//! and a hint. An `Error` refuses an apply; a `Warning` is printed and applied
//! through; an `Info` is a note (a staff-only secret, a staff-run service). The
//! eight rules of doc 14 §4.3, as codes:
//!
//! 1. **schema** — `schema.kind`, `schema.decode`, `schema.pack`, `schema.requires`,
//!    `schema.reference`, `schema.site_pack`, `schema.marker` (a room marker outside
//!    the closed registry, or one named twice; doc 18 D-18-19).
//! 2. **geometry** — `geometry.outside` (a footprint leaves its neighborhood, site or
//!    structure), `geometry.site_overlap`, `geometry.room_overlap`,
//!    `geometry.portal_gap`, `geometry.floors`, `geometry.feature`.
//! 3. **topology** — `topology.unreachable`, `topology.isolated`,
//!    `topology.private_ingress`, `topology.access_point`.
//! 4. **budgets** — `budget.rooms`, `budget.tiles`, `budget.features`, `budget.npcs`,
//!    `budget.lore`, `budget.text`.
//! 5. **policy** — `policy.hydration`, `policy.player_sites`, `policy.kind`,
//!    `policy.property`, `policy.shortcut`, `policy.banned_word`.
//! 6. **secrets** — `secrets.leak`, `secrets.staff_only`.
//! 7. **supported definitions** — `support.staff_run`.
//! 8. **diff safety** — `diff.downgrade`, `diff.live_reference`.
//!
//! The geometric refusals the appliers raise at apply time (an overlap, an exit whose
//! rooms do not touch) are raised here first with the same answers, so a pack the
//! validator passes applies; the appliers keep their checks as the last line.

use std::collections::{BTreeMap, BTreeSet};

use mush_content::{Appliers, ContentState, Pack, PackKind, check_requires};
use mush_core::{CanonicalId, Flags, ObjId, ObjKind, World};
use serde::{Deserialize, Serialize};

use crate::def::{
    CheckpointDef, ConnectionDef, ExitDef, FeatureDef, HookDef, ItemDef, LocalityDef,
    MAX_AMBIENCE_LINE_BYTES, MAX_AMBIENCE_LINES, MAX_SHORT_DESC_BYTES, NeighborhoodDef, RegionDef,
    RoomDef, SecretDef, SiteDef, StructureDef, SurfaceDef,
};
use crate::geom::{Point3, PointCm, Polygon2, PolygonCm, Rect2, Shape};
use crate::module::AtlasState;
use crate::raster::{overlap, shared_threshold};
use crate::record::{
    Access, Budget, Footprint, HydrationPolicy, MAX_FLOORS, MAX_SECRET_BYTES, MAX_TEXT_BYTES,
    RecordKey, Records, RoomKind, SiteOrigin, SurfaceRef,
};
use crate::theme::Theme;

/// The site-kind taxonomy offered as hints (doc 14 R1): a kind outside it is a
/// warning, never a refusal.
pub const SITE_KINDS: [&str; 22] = [
    "site.hall",
    "site.house",
    "site.apartment",
    "site.shop",
    "site.office",
    "site.cafe",
    "site.bar",
    "site.venue",
    "site.park",
    "site.civic",
    "site.school",
    "site.clinic",
    "site.church",
    "site.hotel",
    "site.industrial",
    "site.warehouse",
    "site.garage",
    "site.transit",
    "site.lot",
    "site.garden",
    "site.market",
    "site.studio",
];

/// The most features one room may carry.
pub const MAX_FEATURES_PER_ROOM: usize = 64;
/// The most NPC entries one pack may carry.
pub const MAX_NPCS_PER_PACK: usize = 32;
/// The most bytes one lore entry may serialize to.
pub const MAX_LORE_BYTES: usize = 16 * 1024;
/// The shortest secret that is searched for as a substring of a prompt field; a
/// shorter one must match a field whole.
pub const MIN_SECRET_NEEDLE: usize = 8;

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

/// How much a finding matters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A note.
    Info,
    /// Printed, applied through.
    Warning,
    /// Refuses the apply.
    Error,
}

impl Severity {
    /// The word a plan prints.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// One typed finding (doc 14 §4.3).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// The rule, `<rule>.<what>`.
    pub code: String,
    /// The pack-local entry it is about, if one.
    pub entry: Option<String>,
    /// How much it matters.
    pub severity: Severity,
    /// What is wrong.
    pub message: String,
    /// What would fix it.
    pub fix_hint: String,
}

impl Finding {
    fn new(
        severity: Severity,
        code: &str,
        entry: Option<&CanonicalId>,
        message: impl Into<String>,
        fix_hint: impl Into<String>,
    ) -> Finding {
        Finding {
            code: code.to_string(),
            entry: entry.map(ToString::to_string),
            severity,
            message: message.into(),
            fix_hint: fix_hint.into(),
        }
    }

    /// Whether this finding refuses an apply.
    #[must_use]
    pub fn blocks(&self) -> bool {
        self.severity == Severity::Error
    }

    /// The one-line form a plan prints.
    #[must_use]
    pub fn line(&self) -> String {
        let entry = self
            .entry
            .as_deref()
            .map(|e| format!("{e}: "))
            .unwrap_or_default();
        format!(
            "[{}] {}{} ({}) — {}",
            self.severity.label(),
            entry,
            self.message,
            self.code,
            self.fix_hint
        )
    }
}

/// How many findings refuse the apply.
#[must_use]
pub fn blocking(findings: &[Finding]) -> usize {
    findings.iter().filter(|f| f.blocks()).count()
}

// ---------------------------------------------------------------------------
// The context: what the rules need from the world
// ---------------------------------------------------------------------------

/// A region, as the validator sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionSummary {
    /// The record.
    pub key: RecordKey,
    /// The name.
    pub name: String,
    /// The extent, region metres.
    pub extent: Rect2,
}

/// A locality, as the validator sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalitySummary {
    /// The record.
    pub key: RecordKey,
    /// The region.
    pub region: RecordKey,
    /// The name.
    pub name: String,
    /// The footprint.
    pub footprint: Polygon2,
}

/// A neighborhood, as the validator sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NeighborhoodSummary {
    /// The record.
    pub key: RecordKey,
    /// The region.
    pub region: RecordKey,
    /// The locality.
    pub locality: RecordKey,
    /// The name.
    pub name: String,
    /// The setting word.
    pub setting: String,
    /// The footprint.
    pub footprint: Polygon2,
    /// The room budget.
    pub room_budget: Budget,
    /// The hydration policy.
    pub hydration: HydrationPolicy,
    /// The fine resolution, centimetres.
    pub resolution_cm: u16,
    /// Sites a player built here that stand (pending or approved).
    pub player_sites: u32,
}

/// A site, as the validator sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SiteSummary {
    /// The record.
    pub key: RecordKey,
    /// The neighborhood.
    pub neighborhood: RecordKey,
    /// The name.
    pub name: String,
    /// The kind word.
    pub kind: String,
    /// The footprint.
    pub footprint: Polygon2,
    /// Whether the housing layer holds a title on it.
    pub titled: bool,
    /// Whether a player's request made it.
    pub hydrated: bool,
}

/// A structure, as the validator sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructureSummary {
    /// The record.
    pub key: RecordKey,
    /// The site.
    pub site: RecordKey,
    /// The footprint.
    pub footprint: Polygon2,
    /// The floor indices.
    pub floors: Vec<i16>,
}

/// A room, as the validator sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    /// The record (the pack entry that made it).
    pub key: RecordKey,
    /// The object.
    pub room: ObjId,
    /// The neighborhood.
    pub neighborhood: RecordKey,
    /// The region.
    pub region: RecordKey,
    /// The site, if one.
    pub site: Option<RecordKey>,
    /// The surface.
    pub surface: SurfaceRef,
    /// The footprint.
    pub footprint: Footprint,
    /// What it is.
    pub kind: RoomKind,
    /// Who may enter.
    pub access: Access,
    /// Whether the site it stands on is titled.
    pub titled: bool,
    /// How many characters stand in it.
    pub occupants: u32,
    /// Where the exits into it from outside its own pack come from (a pack id, or
    /// `world` for one dug by hand).
    pub entered_from: Vec<String>,
}

/// One end of a connection, as the validator sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccessSummary {
    /// The neighborhood pack it serves.
    pub neighborhood: CanonicalId,
    /// Where it is.
    pub point: Point3,
    /// The connection.
    pub connection: RecordKey,
}

/// One latched entry, as the validator sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LatchedEntry {
    /// The kind at apply time.
    pub kind: String,
    /// The object it made, if any.
    pub object: Option<ObjId>,
}

/// One pack's latch, as the validator sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LatchSummary {
    /// The version applied.
    pub version: u32,
    /// Whether it is retired.
    pub retired: bool,
    /// Every entry, by pack-local id.
    pub entries: BTreeMap<String, LatchedEntry>,
}

/// One loaded pack, for `requires`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadedRef {
    /// The pack.
    pub id: CanonicalId,
    /// The version.
    pub version: u32,
    /// The digest, hex.
    pub digest: String,
}

/// The limits in force.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidateLimits {
    /// The most cells one tile may hold.
    pub tile_max_cells: u32,
}

impl Default for ValidateLimits {
    fn default() -> ValidateLimits {
        ValidateLimits {
            tile_max_cells: crate::cells::MAX_TILE_CELLS,
        }
    }
}

/// What the validator was asked to look at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ValidateScope {
    /// The world: regions, for a region pack.
    World,
    /// One region: its neighborhoods, sites and rooms.
    Region {
        /// The region.
        key: RecordKey,
    },
    /// One neighborhood, in its region.
    Neighborhood {
        /// The neighborhood.
        key: RecordKey,
    },
}

/// Everything the rules need from the world, captured on the world loop.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ValidateContext {
    /// What was looked at.
    pub scope: Option<ValidateScope>,
    /// The entry kinds some module on this server owns.
    pub kinds: Vec<String>,
    /// Every loaded pack, for `requires`.
    pub loaded: Vec<LoadedRef>,
    /// Every latch, by pack id.
    pub latches: BTreeMap<String, LatchSummary>,
    /// Regions.
    pub regions: Vec<RegionSummary>,
    /// Localities.
    pub localities: Vec<LocalitySummary>,
    /// Neighborhoods.
    pub neighborhoods: Vec<NeighborhoodSummary>,
    /// Sites.
    pub sites: Vec<SiteSummary>,
    /// Structures.
    pub structures: Vec<StructureSummary>,
    /// Rooms.
    pub rooms: Vec<RoomSummary>,
    /// Checkpoints.
    pub checkpoints: Vec<RecordKey>,
    /// Connections' ends.
    pub access_points: Vec<AccessSummary>,
    /// Exit entries applied (portal records).
    pub exits: Vec<RecordKey>,
    /// The theme in force over the scope.
    pub theme: Option<Theme>,
    /// The travel modes this server knows.
    pub modes: Vec<String>,
    /// Whether the pack is a player's hydration (doc 14 §4.5): the neighborhood's
    /// policy applies.
    pub player_build: bool,
    /// The limits.
    pub limits: ValidateLimits,
}

impl ValidateContext {
    /// A context that knows the owned kinds and nothing of the world: the schema
    /// rules run, the rest see an empty map.
    #[must_use]
    pub fn schema_only(appliers: &Appliers) -> ValidateContext {
        ValidateContext {
            kinds: appliers.kinds().kinds().map(ToString::to_string).collect(),
            modes: known_modes(),
            ..ValidateContext::default()
        }
    }

    /// The scope a pack is validated in: a region pack against the world, a
    /// neighborhood pack against its region, a site pack against the neighborhood it
    /// requires.
    #[must_use]
    pub fn scope_of(records: &Records, pack: &Pack) -> ValidateScope {
        match pack.manifest().kind() {
            PackKind::Neighborhood => {
                let kind = CanonicalId::new("neighborhood").ok();
                let def = kind.and_then(|k| {
                    pack.entries_of(&k)
                        .next()
                        .and_then(|(_, e)| e.decode::<NeighborhoodDef>().ok())
                });
                let region = def.and_then(|d| {
                    resolve_in_world(records, pack, &d.region)
                        .filter(|k| records.regions.contains_key(k))
                });
                match region {
                    Some(key) => ValidateScope::Region { key },
                    None => ValidateScope::World,
                }
            }
            PackKind::Site => {
                let nb = pack.manifest().requires().iter().find_map(|r| {
                    records
                        .neighborhoods
                        .values()
                        .find(|n| n.key.pack == r.id)
                        .map(|n| n.key.clone())
                });
                match nb {
                    Some(key) => ValidateScope::Neighborhood { key },
                    None => ValidateScope::World,
                }
            }
            PackKind::Region | PackKind::RulesDefinition | PackKind::CampaignSeed => {
                ValidateScope::World
            }
        }
    }

    /// Capture the world for `scope`.
    #[must_use]
    pub fn capture(
        w: &World,
        state: &AtlasState,
        content: &ContentState,
        scope: &ValidateScope,
    ) -> ValidateContext {
        let records = &state.records;
        let mut cx = ValidateContext {
            scope: Some(scope.clone()),
            kinds: content
                .appliers
                .kinds()
                .kinds()
                .map(ToString::to_string)
                .collect(),
            loaded: content
                .latches
                .loaded()
                .into_iter()
                .map(|((id, version), loaded)| LoadedRef {
                    id,
                    version,
                    digest: loaded.digest.to_hex(),
                })
                .collect(),
            latches: content
                .latches
                .all()
                .map(|a| {
                    (
                        a.pack().to_string(),
                        LatchSummary {
                            version: a.version(),
                            retired: a.is_retired(),
                            entries: a
                                .entries()
                                .iter()
                                .map(|(id, e)| {
                                    (
                                        id.to_string(),
                                        LatchedEntry {
                                            kind: e.kind().to_string(),
                                            object: e.object(),
                                        },
                                    )
                                })
                                .collect(),
                        },
                    )
                })
                .collect(),
            modes: known_modes(),
            limits: ValidateLimits {
                tile_max_cells: state.limits.tile_max_cells,
            },
            ..ValidateContext::default()
        };
        cx.regions = records
            .regions
            .values()
            .map(|r| RegionSummary {
                key: r.key.clone(),
                name: r.name.clone(),
                extent: r.extent,
            })
            .collect();
        cx.localities = records
            .localities
            .values()
            .map(|l| LocalitySummary {
                key: l.key.clone(),
                region: l.region.clone(),
                name: l.name.clone(),
                footprint: l.footprint.clone(),
            })
            .collect();
        let region = match scope {
            ValidateScope::World => None,
            ValidateScope::Region { key } => Some(key.clone()),
            ValidateScope::Neighborhood { key } => {
                records.neighborhoods.get(key).map(|n| n.region.clone())
            }
        };
        cx.neighborhoods = records
            .neighborhoods
            .values()
            .map(|n| NeighborhoodSummary {
                key: n.key.clone(),
                region: n.region.clone(),
                locality: n.locality.clone(),
                name: n.name.clone(),
                setting: n.setting.clone(),
                footprint: n.footprint.clone(),
                room_budget: n.room_budget,
                hydration: n.hydration,
                resolution_cm: n.resolution_cm,
                player_sites: records
                    .sites_in(&n.key)
                    .filter(|s| matches!(s.origin, SiteOrigin::Hydrated { .. }))
                    .count()
                    .try_into()
                    .unwrap_or(u32::MAX),
            })
            .collect();
        let Some(region) = region else {
            cx.theme = crate::theme::world_theme(w);
            return cx;
        };
        let in_region: BTreeSet<RecordKey> = records
            .neighborhoods_in(&region)
            .map(|n| n.key.clone())
            .collect();
        cx.sites = records
            .sites
            .values()
            .filter(|s| s.neighborhoods.iter().any(|n| in_region.contains(n)))
            .filter_map(|s| {
                Some(SiteSummary {
                    key: s.key.clone(),
                    neighborhood: s.neighborhoods.first()?.clone(),
                    name: s.name.clone(),
                    kind: s.kind.clone(),
                    footprint: s.footprint.clone(),
                    titled: s.property.is_some(),
                    hydrated: matches!(s.origin, SiteOrigin::Hydrated { .. }),
                })
            })
            .collect();
        let titled: BTreeSet<RecordKey> = cx
            .sites
            .iter()
            .filter(|s| s.titled)
            .map(|s| s.key.clone())
            .collect();
        cx.structures = records
            .structures
            .values()
            .filter(|s| cx.sites.iter().any(|site| site.key == s.site))
            .map(|s| StructureSummary {
                key: s.key.clone(),
                site: s.site.clone(),
                footprint: s.footprint.clone(),
                floors: s.floors.iter().map(|f| f.index).collect(),
            })
            .collect();
        // Which pack each exit object belongs to, for `entered_from`.
        let exit_pack: BTreeMap<ObjId, CanonicalId> = records
            .portals
            .values()
            .flat_map(|p| p.exits.iter().map(move |e| (*e, p.key.pack.clone())))
            .collect();
        cx.rooms = records
            .roomgeo
            .values()
            .filter(|g| in_region.contains(&g.neighborhood))
            .map(|g| {
                let occupants = w
                    .contents_of(g.room)
                    .iter()
                    .filter(|o| {
                        w.get(**o).is_some_and(|o| {
                            o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING)
                        })
                    })
                    .count()
                    .try_into()
                    .unwrap_or(u32::MAX);
                let entered_from: Vec<String> = w
                    .iter()
                    .filter(|o| {
                        o.kind == ObjKind::Exit
                            && o.destination == Some(g.room)
                            && !o.flags.contains(Flags::GOING)
                    })
                    .filter_map(|o| match exit_pack.get(&o.id) {
                        Some(pack) if *pack == g.key.pack => None,
                        Some(pack) => Some(pack.to_string()),
                        None => Some("world".to_string()),
                    })
                    .collect::<BTreeSet<String>>()
                    .into_iter()
                    .collect();
                RoomSummary {
                    key: g.key.clone(),
                    room: g.room,
                    neighborhood: g.neighborhood.clone(),
                    region: region.clone(),
                    site: g.site.clone(),
                    surface: g.surface.clone(),
                    footprint: g.footprint.clone(),
                    kind: g.kind,
                    access: g.access,
                    titled: g.site.as_ref().is_some_and(|s| titled.contains(s)),
                    occupants,
                    entered_from,
                }
            })
            .collect();
        cx.checkpoints = records
            .checkpoints
            .values()
            .filter(|c| c.region == region)
            .map(|c| c.key.clone())
            .collect();
        cx.access_points = records
            .connections_in(&region)
            .flat_map(|c| {
                [&c.endpoints.0, &c.endpoints.1]
                    .into_iter()
                    // S9.1: an endpoint in no neighborhood (an orbital site's
                    // room) is nothing a neighborhood's geometry checks.
                    .filter_map(move |end| {
                        Some(AccessSummary {
                            neighborhood: end.neighborhood.clone()?,
                            point: end.point,
                            connection: c.key.clone(),
                        })
                    })
            })
            .collect();
        cx.exits = records
            .portals
            .values()
            .filter(|p| {
                cx.rooms
                    .iter()
                    .any(|r| p.exits.iter().any(|_| r.key.pack == p.key.pack))
            })
            .map(|p| p.key.clone())
            .collect();
        cx.theme = Some(match scope {
            ValidateScope::Neighborhood { key } => crate::theme::resolve_place_theme(
                w,
                &crate::place::PlaceRef::Neighborhood(key.clone()),
            ),
            _ => crate::theme::resolve_place_theme(w, &crate::place::PlaceRef::Region(region)),
        });
        cx
    }

    /// [`ValidateContext::capture`] for the scope [`ValidateContext::scope_of`] gives
    /// `pack`.
    #[must_use]
    pub fn for_pack(w: &World, state: &AtlasState, content: &ContentState, pack: &Pack) -> Self {
        let scope = ValidateContext::scope_of(&state.records, pack);
        ValidateContext::capture(w, state, content, &scope)
    }

    fn neighborhood(&self, key: &RecordKey) -> Option<&NeighborhoodSummary> {
        self.neighborhoods.iter().find(|n| &n.key == key)
    }

    fn known_keys(&self) -> BTreeSet<RecordKey> {
        let mut known = BTreeSet::new();
        known.extend(self.regions.iter().map(|r| r.key.clone()));
        known.extend(self.localities.iter().map(|l| l.key.clone()));
        known.extend(self.neighborhoods.iter().map(|n| n.key.clone()));
        known.extend(self.sites.iter().map(|s| s.key.clone()));
        known.extend(self.structures.iter().map(|s| s.key.clone()));
        known.extend(self.rooms.iter().map(|r| r.key.clone()));
        known.extend(self.checkpoints.iter().cloned());
        known.extend(self.access_points.iter().map(|a| a.connection.clone()));
        known.extend(self.exits.iter().cloned());
        known
    }
}

/// The travel modes the atlas knows a speed for, plus the walk.
fn known_modes() -> Vec<String> {
    ["foot", "car", "transit", "bus", "ship", "shuttle", "torch"]
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// Resolve a reference the way the applier's resolver does, against the records:
/// this pack's own entries first, then the packs it requires.
fn resolve_in_world(records: &Records, pack: &Pack, reference: &str) -> Option<RecordKey> {
    let reference = reference.trim();
    let own = pack.manifest().id();
    if let Ok(id) = CanonicalId::new(reference)
        && pack.entry(&id).is_some()
    {
        return Some(RecordKey::new(own.clone(), id));
    }
    for requirement in pack.manifest().requires() {
        let prefixed = reference
            .strip_prefix(&format!("{}.", requirement.id))
            .and_then(|rest| CanonicalId::new(rest).ok());
        let bare = CanonicalId::new(reference).ok();
        for candidate in prefixed.into_iter().chain(bare) {
            let key = RecordKey::new(requirement.id.clone(), candidate);
            if records.regions.contains_key(&key)
                || records.localities.contains_key(&key)
                || records.neighborhoods.contains_key(&key)
                || records.sites.contains_key(&key)
            {
                return Some(key);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The compatibility entry point
// ---------------------------------------------------------------------------

/// Every refusal a schema-only validation finds, one line each. Empty is clean.
///
/// The S1 entry point, kept for callers with no world in hand; [`validate`] with a
/// captured [`ValidateContext`] is the whole validator.
#[must_use]
pub fn check_pack(pack: &Pack, appliers: &Appliers) -> Vec<String> {
    validate(pack, &ValidateContext::schema_only(appliers))
        .into_iter()
        .filter(Finding::blocks)
        .map(|f| match f.entry {
            Some(entry) => format!("{entry}: {}", f.message),
            None => f.message,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The validator
// ---------------------------------------------------------------------------

/// Validate `pack` against `cx`: every finding, errors first, in rule order.
#[must_use]
pub fn validate(pack: &Pack, cx: &ValidateContext) -> Vec<Finding> {
    let mut run = Run {
        pack,
        cx,
        known: cx.known_keys(),
        decoded: Decoded::default(),
        out: Vec::new(),
    };
    run.schema();
    run.geometry();
    run.topology();
    run.budgets();
    run.policy();
    run.secrets();
    run.supported();
    run.diff_safety();
    let mut out = run.out;
    out.sort_by_key(|f| std::cmp::Reverse(f.severity));
    out
}

/// The pack's entries, decoded once.
#[derive(Default)]
struct Decoded {
    region: Option<(CanonicalId, RegionDef)>,
    localities: BTreeMap<CanonicalId, LocalityDef>,
    checkpoints: BTreeMap<CanonicalId, CheckpointDef>,
    connections: BTreeMap<CanonicalId, ConnectionDef>,
    neighborhood: Option<(CanonicalId, NeighborhoodDef)>,
    sites: BTreeMap<CanonicalId, SiteDef>,
    structures: BTreeMap<CanonicalId, StructureDef>,
    rooms: BTreeMap<CanonicalId, RoomDef>,
    exits: BTreeMap<CanonicalId, ExitDef>,
    features: BTreeMap<CanonicalId, FeatureDef>,
    items: BTreeMap<CanonicalId, ItemDef>,
    hooks: BTreeMap<CanonicalId, HookDef>,
    secrets: BTreeMap<CanonicalId, SecretDef>,
    npcs: BTreeMap<CanonicalId, serde_json::Value>,
    lore: BTreeMap<CanonicalId, serde_json::Value>,
}

/// Where a reference points.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Target {
    /// An entry of this pack.
    Own(CanonicalId),
    /// A record of another pack.
    World(RecordKey),
    /// Limbo.
    Limbo,
}

/// A room, for the geometry and topology rules: one of this pack's or one of the
/// world's.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Node {
    Own(CanonicalId),
    World(RecordKey),
    Limbo,
}

struct Run<'a> {
    pack: &'a Pack,
    cx: &'a ValidateContext,
    known: BTreeSet<RecordKey>,
    decoded: Decoded,
    out: Vec<Finding>,
}

impl Run<'_> {
    fn error(
        &mut self,
        code: &str,
        entry: Option<&CanonicalId>,
        msg: impl Into<String>,
        hint: impl Into<String>,
    ) {
        self.out
            .push(Finding::new(Severity::Error, code, entry, msg, hint));
    }

    fn warning(
        &mut self,
        code: &str,
        entry: Option<&CanonicalId>,
        msg: impl Into<String>,
        hint: impl Into<String>,
    ) {
        self.out
            .push(Finding::new(Severity::Warning, code, entry, msg, hint));
    }

    fn info(
        &mut self,
        code: &str,
        entry: Option<&CanonicalId>,
        msg: impl Into<String>,
        hint: impl Into<String>,
    ) {
        self.out
            .push(Finding::new(Severity::Info, code, entry, msg, hint));
    }

    fn own_id(&self) -> &CanonicalId {
        self.pack.manifest().id()
    }

    fn own_key(&self, entry: &CanonicalId) -> RecordKey {
        RecordKey::new(self.own_id().clone(), entry.clone())
    }

    // ---- references ---------------------------------------------------------

    /// Resolve a reference as the applier's resolver would: `limbo`, this pack's
    /// entries (bare or prefixed with the pack id), then the required packs' records.
    fn resolve(&self, reference: &str) -> Option<Target> {
        let reference = reference.trim();
        if reference == "limbo" {
            return Some(Target::Limbo);
        }
        if let Ok(id) = CanonicalId::new(reference)
            && self.pack.entry(&id).is_some()
        {
            return Some(Target::Own(id));
        }
        let own = self.own_id().to_string();
        if let Some(rest) = reference.strip_prefix(&format!("{own}."))
            && let Ok(id) = CanonicalId::new(rest)
            && self.pack.entry(&id).is_some()
        {
            return Some(Target::Own(id));
        }
        for requirement in self.pack.manifest().requires() {
            let prefixed = reference
                .strip_prefix(&format!("{}.", requirement.id))
                .and_then(|rest| CanonicalId::new(rest).ok());
            let bare = CanonicalId::new(reference).ok();
            for candidate in prefixed.into_iter().chain(bare) {
                let key = RecordKey::new(requirement.id.clone(), candidate);
                if self.known.contains(&key) {
                    return Some(Target::World(key));
                }
            }
        }
        None
    }

    /// Resolve a reference that must be an entry of `kind` (this pack's, or applied).
    fn require(
        &mut self,
        entry: &CanonicalId,
        field: &str,
        reference: &str,
        kind: &str,
    ) -> Option<Target> {
        let target = self.resolve(reference);
        let ok = match &target {
            Some(Target::Limbo) => kind == "room",
            Some(Target::Own(id)) => self
                .pack
                .entry(id)
                .is_some_and(|e| e.kind().as_str() == kind),
            Some(Target::World(key)) => self.world_kind(key) == Some(kind),
            None => false,
        };
        if ok {
            return target;
        }
        let what = match target {
            Some(_) => format!("{field}: {reference:?} is not a {kind}"),
            None => format!(
                "{field}: {reference:?} names no entry of this pack or of a pack it requires"
            ),
        };
        self.error(
            "schema.reference",
            Some(entry),
            what,
            format!(
                "name a {kind} entry of this pack, or `<pack>.<entry>` of a pack in `requires`"
            ),
        );
        None
    }

    fn world_kind(&self, key: &RecordKey) -> Option<&'static str> {
        let cx = self.cx;
        if cx.regions.iter().any(|r| &r.key == key) {
            Some("region")
        } else if cx.localities.iter().any(|l| &l.key == key) {
            Some("locality")
        } else if cx.neighborhoods.iter().any(|n| &n.key == key) {
            Some("neighborhood")
        } else if cx.sites.iter().any(|s| &s.key == key) {
            Some("site")
        } else if cx.structures.iter().any(|s| &s.key == key) {
            Some("structure")
        } else if cx.rooms.iter().any(|r| &r.key == key) {
            Some("room")
        } else if cx.checkpoints.contains(key) {
            Some("checkpoint")
        } else if cx.exits.contains(key) {
            Some("exit")
        } else if cx.access_points.iter().any(|a| &a.connection == key) {
            Some("connection")
        } else {
            None
        }
    }

    /// NPC packs can explicitly name an applied room outside the geometry scope.
    /// Resolve only the named dependency's latch; the NPC applier additionally
    /// verifies its current health and the live room object before any write.
    fn require_npc_room(&mut self, entry: &CanonicalId, room: &str, room_pack: &serde_json::Value) {
        let Some(name) = room_pack.as_str() else {
            self.error(
                "schema.decode",
                Some(entry),
                "room_pack must be a pack id string",
                "name the explicitly required pack that owns the NPC's room",
            );
            return;
        };
        let found = CanonicalId::new(name.trim())
            .ok()
            .zip(CanonicalId::new(room.trim()).ok())
            .and_then(|(pack, room)| {
                let required = self
                    .pack
                    .manifest()
                    .requires()
                    .iter()
                    .find(|r| r.id == pack)?;
                let latch = self.cx.latches.get(pack.as_str())?;
                if latch.retired || latch.version != required.version {
                    return None;
                }
                latch.entries.get(room.as_str())
            })
            .is_some_and(|room| room.kind == "room" && room.object.is_some());
        if !found {
            self.error(
                "schema.reference",
                Some(entry),
                format!("room: {room:?} is not an applied room of the explicit room_pack dependency {name:?}"),
                "declare room_pack in requires and name its exact applied room entry",
            );
        }
    }

    // ---- 1. schema ------------------------------------------------------------

    fn schema(&mut self) {
        let pack = self.pack;
        let kinds: BTreeSet<&str> = self.cx.kinds.iter().map(String::as_str).collect();
        for (id, entry) in pack.entries() {
            let kind = entry.kind().as_str();
            if !kinds.is_empty() && !kinds.contains(kind) {
                self.error(
                    "schema.kind",
                    Some(id),
                    format!("nothing on this server owns kind {kind}"),
                    "use a kind the atlas or the NPC module owns",
                );
                continue;
            }
            let outcome: Result<(), String> = match kind {
                "region" => entry
                    .decode::<RegionDef>()
                    .map_err(|e| e.to_string())
                    .and_then(|d| d.region_kind().map(|_| d))
                    .map(|d| {
                        if self.decoded.region.is_some() {
                            // The second is a pack error, reported below.
                        }
                        self.decoded.region.get_or_insert((id.clone(), d));
                    }),
                "locality" => entry
                    .decode::<LocalityDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        self.decoded.localities.insert(id.clone(), d);
                    }),
                // S7: the grid's shape and every sample, so a bad field is refused
                // at plan and never half-imported. C.1: and the authored class
                // grid's shape and every code beside them.
                "heightfield" => entry
                    .decode::<crate::def::HeightfieldDef>()
                    .map_err(|e| e.to_string())
                    .and_then(|d| d.samples().map(|_| d))
                    .and_then(|d| d.class_grid().map(|_| ())),
                "checkpoint" => entry
                    .decode::<CheckpointDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        self.decoded.checkpoints.insert(id.clone(), d);
                    }),
                "connection" => entry
                    .decode::<ConnectionDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        self.decoded.connections.insert(id.clone(), d);
                    }),
                "neighborhood" => entry
                    .decode::<NeighborhoodDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        if self.decoded.neighborhood.is_none() {
                            self.decoded.neighborhood = Some((id.clone(), d));
                        }
                    }),
                "site" => entry
                    .decode::<SiteDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        self.decoded.sites.insert(id.clone(), d);
                    }),
                "structure" => entry
                    .decode::<StructureDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        self.decoded.structures.insert(id.clone(), d);
                    }),
                "room" => entry
                    .decode::<RoomDef>()
                    .map_err(|e| e.to_string())
                    .and_then(|d| {
                        d.footprint
                            .to_footprint()
                            .shape()
                            .map(|_| d)
                            .map_err(|e| format!("footprint: {e}"))
                    })
                    .map(|d| {
                        self.decoded.rooms.insert(id.clone(), d);
                    }),
                "feature" => entry
                    .decode::<FeatureDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        self.decoded.features.insert(id.clone(), d);
                    }),
                "item" => entry
                    .decode::<ItemDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        self.decoded.items.insert(id.clone(), d);
                    }),
                "exit" => entry
                    .decode::<ExitDef>()
                    .map_err(|e| e.to_string())
                    .and_then(|d| {
                        if d.names.is_empty() || d.names.len() > 2 {
                            Err("an exit row names one exit, or two".to_string())
                        } else {
                            Ok(d)
                        }
                    })
                    .map(|d| {
                        self.decoded.exits.insert(id.clone(), d);
                    }),
                "hook" => entry
                    .decode::<HookDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        self.decoded.hooks.insert(id.clone(), d);
                    }),
                "secret" => entry
                    .decode::<SecretDef>()
                    .map_err(|e| e.to_string())
                    .map(|d| {
                        self.decoded.secrets.insert(id.clone(), d);
                    }),
                "npc" => {
                    self.decoded.npcs.insert(id.clone(), entry.def_value());
                    Ok(())
                }
                "lore" => {
                    self.decoded.lore.insert(id.clone(), entry.def_value());
                    Ok(())
                }
                // Another owner's kind decodes at apply.
                _ => Ok(()),
            };
            if let Err(problem) = outcome {
                self.error(
                    "schema.decode",
                    Some(id),
                    problem,
                    "match the entry's definition shape (see help atlas)",
                );
            }
        }

        // The pack's kind and its cardinal entry.
        let manifest_kind = pack.manifest().kind();
        let neighborhoods = pack
            .entries()
            .values()
            .filter(|e| e.kind().as_str() == "neighborhood")
            .count();
        let regions = pack
            .entries()
            .values()
            .filter(|e| e.kind().as_str() == "region")
            .count();
        match manifest_kind {
            PackKind::Neighborhood if neighborhoods != 1 => self.error(
                "schema.pack",
                None,
                format!("a neighborhood pack has exactly one neighborhood entry; this one has {neighborhoods}"),
                "give the pack one `neighborhood` entry",
            ),
            PackKind::Region if regions != 1 => self.error(
                "schema.pack",
                None,
                format!("a region pack has exactly one region entry; this one has {regions}"),
                "give the pack one `region` entry",
            ),
            PackKind::Site => {
                if self.decoded.sites.is_empty() {
                    self.error(
                        "schema.site_pack",
                        None,
                        "a site pack adds at least one site",
                        "give the pack a `site` entry the rooms name",
                    );
                }
                if neighborhoods != 0 {
                    self.error(
                        "schema.site_pack",
                        None,
                        "a site pack adds to a neighborhood it requires; it does not carry one",
                        "move the neighborhood entry to a neighborhood pack",
                    );
                }
                let required_nb = self.site_pack_neighborhood();
                if required_nb.is_none() {
                    self.error(
                        "schema.site_pack",
                        None,
                        "a site pack requires exactly one applied neighborhood pack",
                        "add the neighborhood pack to `requires`",
                    );
                }
            }
            _ => {}
        }

        // `requires`, against what is loaded.
        let loaded: BTreeMap<(CanonicalId, u32), mush_content::LoadedPack> = self
            .cx
            .loaded
            .iter()
            .filter_map(|l| {
                let digest = mush_core::DefinitionDigest::from_hex(&l.digest).ok()?;
                Some((
                    (l.id.clone(), l.version),
                    mush_content::LoadedPack { digest },
                ))
            })
            .collect();
        if let Err(e) = check_requires(pack, &loaded) {
            self.error(
                "schema.requires",
                None,
                e.to_string(),
                "apply the required pack at that version first, or fix the requirement",
            );
        }

        // References.
        let rooms: Vec<(CanonicalId, RoomDef)> = self
            .decoded
            .rooms
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (id, room) in &rooms {
            if let Some(site) = &room.site {
                self.require(id, "site", site, "site");
            }
            if let SurfaceDef::Floor(structure, index) = &room.surface
                && let Some(target) = self.require(id, "surface", structure, "structure")
            {
                let has_floor = match &target {
                    Target::Own(s) => self
                        .decoded
                        .structures
                        .get(s)
                        .is_some_and(|d| d.floors.iter().any(|f| f.index == *index)),
                    Target::World(key) => self
                        .cx
                        .structures
                        .iter()
                        .any(|s| &s.key == key && s.floors.contains(index)),
                    Target::Limbo => false,
                };
                if !has_floor {
                    self.error(
                        "schema.reference",
                        Some(id),
                        format!("surface: {structure} has no floor {index}"),
                        "name a floor the structure declares",
                    );
                }
            }
            if let PackKind::Site = manifest_kind {
                let own_site = room
                    .site
                    .as_deref()
                    .and_then(|s| self.resolve(s))
                    .is_some_and(|t| matches!(t, Target::Own(_)));
                if !own_site {
                    self.error(
                        "schema.site_pack",
                        Some(id),
                        "a site pack's rooms stand on a site of the pack",
                        "set `site` to one of this pack's site entries",
                    );
                }
            }
        }
        // Doc 18 D-18-19: a room's markers come from the closed registry, once each —
        // the applier refuses the same lists with the same words.
        let marker_problems: Vec<(CanonicalId, String)> = rooms
            .iter()
            .filter_map(|(id, room)| {
                crate::def::check_room_markers(&room.markers)
                    .err()
                    .map(|e| (id.clone(), e))
            })
            .collect();
        for (id, problem) in marker_problems {
            self.error(
                "schema.marker",
                Some(&id),
                problem,
                format!("name each marker once, from {:?}", crate::def::ROOM_MARKERS),
            );
        }
        let structures: Vec<(CanonicalId, String)> = self
            .decoded
            .structures
            .iter()
            .map(|(k, v)| (k.clone(), v.site.clone()))
            .collect();
        for (id, site) in &structures {
            self.require(id, "site", site, "site");
        }
        let sites: Vec<(CanonicalId, Vec<String>)> = self
            .decoded
            .sites
            .iter()
            .map(|(k, v)| (k.clone(), v.entrances.clone()))
            .collect();
        for (id, entrances) in &sites {
            for entrance in entrances {
                let own_exit = CanonicalId::new(entrance)
                    .ok()
                    .and_then(|e| self.pack.entry(&e).map(|r| r.kind().as_str() == "exit"))
                    .unwrap_or(false);
                if !own_exit {
                    self.error(
                        "schema.reference",
                        Some(id),
                        format!("entrance {entrance:?} is not an exit entry of this pack"),
                        "name an `exit` entry of this pack",
                    );
                }
            }
        }
        let exits: Vec<(CanonicalId, String, String)> = self
            .decoded
            .exits
            .iter()
            .map(|(k, v)| (k.clone(), v.from.clone(), v.to.clone()))
            .collect();
        for (id, from, to) in &exits {
            let a = self.require(id, "from", from, "room");
            let b = self.require(id, "to", to, "room");
            if let (Some(a), Some(b)) = (a, b)
                && a == b
            {
                self.error(
                    "schema.reference",
                    Some(id),
                    "an exit joins two different rooms",
                    "give `from` and `to` different rooms",
                );
            }
        }
        let placed: Vec<(CanonicalId, Option<String>)> = self
            .decoded
            .features
            .iter()
            .map(|(k, v)| (k.clone(), v.room.clone()))
            .chain(
                self.decoded
                    .items
                    .iter()
                    .map(|(k, v)| (k.clone(), Some(v.room.clone()))),
            )
            .chain(
                self.decoded
                    .hooks
                    .iter()
                    .map(|(k, v)| (k.clone(), v.room.clone())),
            )
            .collect();
        for (id, room) in &placed {
            if let Some(room) = room {
                self.require(id, "room", room, "room");
            }
        }
        let npcs: Vec<(CanonicalId, Option<String>, Option<serde_json::Value>)> = self
            .decoded
            .npcs
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.get("room")
                        .and_then(|r| r.as_str())
                        .map(ToString::to_string),
                    v.get("room_pack").filter(|p| !p.is_null()).cloned(),
                )
            })
            .collect();
        for (id, room, room_pack) in &npcs {
            match room {
                Some(room) => {
                    if let Some(pack) = room_pack {
                        self.require_npc_room(id, room, pack);
                    } else {
                        self.require(id, "room", room, "room");
                    }
                }
                None => self.error(
                    "schema.decode",
                    Some(id),
                    "an npc stands in a room",
                    "give the npc a `room` entry of this pack",
                ),
            }
        }
        if let Some((id, nb)) = self.decoded.neighborhood.clone() {
            self.require(&id, "region", &nb.region, "region");
            self.require(&id, "locality", &nb.locality, "locality");
        }
        let localities: Vec<(CanonicalId, String)> = self
            .decoded
            .localities
            .iter()
            .map(|(k, v)| (k.clone(), v.region.clone()))
            .collect();
        for (id, region) in &localities {
            self.require(id, "region", region, "region");
        }
        let checkpoints: Vec<(CanonicalId, String)> = self
            .decoded
            .checkpoints
            .iter()
            .map(|(k, v)| (k.clone(), v.region.clone()))
            .collect();
        for (id, region) in &checkpoints {
            self.require(id, "region", region, "region");
        }
        let connections: Vec<(CanonicalId, ConnectionDef)> = self
            .decoded
            .connections
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (id, c) in &connections {
            self.require(id, "region", &c.region, "region");
            for (n, leg) in c.legs.iter().enumerate() {
                let last = n + 1 == c.legs.len();
                let own_end = CanonicalId::new(&leg.to).ok().as_ref() == Some(id);
                if last {
                    if !own_end {
                        self.error(
                            "schema.reference",
                            Some(id),
                            format!("leg {}: the last leg names the connection itself", n + 1),
                            "set the last leg's `to` to this entry's id",
                        );
                    }
                } else if !own_end {
                    self.require(id, &format!("leg {}", n + 1), &leg.to, "checkpoint");
                }
            }
            for end in [&c.from, &c.to] {
                let known = self
                    .cx
                    .neighborhoods
                    .iter()
                    .any(|n| n.key.pack.as_str() == end.neighborhood.as_str())
                    || self
                        .cx
                        .loaded
                        .iter()
                        .any(|l| l.id.as_str() == end.neighborhood.as_str());
                if !known {
                    self.info(
                        "schema.reference",
                        Some(id),
                        format!(
                            "an end serves neighborhood pack {:?}, which is not applied yet",
                            end.neighborhood
                        ),
                        "apply that neighborhood pack; the end resolves when it lands",
                    );
                }
            }
        }
        let secrets: Vec<(CanonicalId, Vec<String>)> = self
            .decoded
            .secrets
            .iter()
            .map(|(k, v)| (k.clone(), v.about.clone()))
            .collect();
        for (id, about) in &secrets {
            for a in about {
                if CanonicalId::new(a).is_err() {
                    self.error(
                        "schema.reference",
                        Some(id),
                        format!("about {a:?} is not a pack-local id"),
                        "name entries of this pack",
                    );
                }
            }
        }
    }

    /// The neighborhood a site pack adds to: the one applied neighborhood among the
    /// packs it requires.
    fn site_pack_neighborhood(&self) -> Option<RecordKey> {
        let mut found = self.pack.manifest().requires().iter().filter_map(|r| {
            self.cx
                .neighborhoods
                .iter()
                .find(|n| n.key.pack == r.id)
                .map(|n| n.key.clone())
        });
        let first = found.next()?;
        if found.next().is_some() {
            return None;
        }
        Some(first)
    }

    // ---- the neighborhood the pack works in ----------------------------------

    /// The neighborhood the pack's sites and rooms lie in: the pack's own entry for a
    /// neighborhood pack, the required one for a site pack.
    fn neighborhood(&self) -> Option<NeighborhoodShape> {
        match self.pack.manifest().kind() {
            PackKind::Neighborhood => {
                let (id, def) = self.decoded.neighborhood.as_ref()?;
                let key = self.own_key(id);
                let region = self.resolve(&def.region).and_then(|t| match t {
                    Target::World(k) => Some(k),
                    Target::Own(_) | Target::Limbo => None,
                });
                Some(NeighborhoodShape {
                    key,
                    region,
                    polygon: def.footprint.to_cm().ok()?,
                    resolution_cm: def.resolution_cm,
                    room_budget: Budget {
                        target: def.room_budget.target,
                        maximum: def.room_budget.maximum,
                    },
                    hydration: HydrationPolicy {
                        enabled: def.hydration.enabled,
                        max_player_sites: def.hydration.max_player_sites,
                    },
                    player_sites: self
                        .cx
                        .neighborhood(&self.own_key(id))
                        .map_or(0, |n| n.player_sites),
                })
            }
            PackKind::Site => {
                let key = self.site_pack_neighborhood()?;
                let nb = self.cx.neighborhood(&key)?;
                Some(NeighborhoodShape {
                    key: nb.key.clone(),
                    region: Some(nb.region.clone()),
                    polygon: nb.footprint.to_cm().ok()?,
                    resolution_cm: nb.resolution_cm,
                    room_budget: nb.room_budget,
                    hydration: nb.hydration,
                    player_sites: nb.player_sites,
                })
            }
            _ => None,
        }
    }

    /// This pack's rooms with their surfaces and shapes.
    fn pack_rooms(&self) -> Vec<(CanonicalId, SurfaceRef, Shape, Option<Target>)> {
        self.decoded
            .rooms
            .iter()
            .filter_map(|(id, def)| {
                let shape = def.footprint.to_footprint().shape().ok()?;
                let surface = match &def.surface {
                    SurfaceDef::Ground => SurfaceRef::Ground,
                    SurfaceDef::Floor(structure, index) => SurfaceRef::Floor {
                        structure: match self.resolve(structure)? {
                            Target::Own(s) => self.own_key(&s),
                            Target::World(k) => k,
                            Target::Limbo => return None,
                        },
                        index: *index,
                    },
                };
                let site = def.site.as_deref().and_then(|s| self.resolve(s));
                Some((id.clone(), surface, shape, site))
            })
            .collect()
    }

    /// The world's rooms in the region, other than this pack's (which it replaces).
    fn world_rooms<'b>(
        &'b self,
        region: Option<&'b RecordKey>,
    ) -> impl Iterator<Item = &'b RoomSummary> + 'b {
        let own = self.own_id().clone();
        self.cx
            .rooms
            .iter()
            .filter(move |r| r.key.pack != own && region.is_none_or(|g| &r.region == g))
    }

    // ---- 2. geometry ------------------------------------------------------------

    fn geometry(&mut self) {
        let Some(nb) = self.neighborhood() else {
            return;
        };
        let own = self.own_id().clone();

        // Sites: inside the neighborhood, disjoint from every other site in it.
        let sites: Vec<(CanonicalId, PolygonCm, Option<CanonicalId>)> = self
            .decoded
            .sites
            .iter()
            .filter_map(|(id, def)| Some((id.clone(), def.footprint.to_cm().ok()?, None)))
            .collect();
        for (id, poly, _) in &sites {
            if !inside(&nb.polygon, &Shape::Polygon(poly.clone())) {
                self.error(
                    "geometry.outside",
                    Some(id),
                    "its footprint leaves the neighborhood",
                    "keep every corner inside the neighborhood's footprint",
                );
            }
        }
        let world_sites: Vec<(RecordKey, String, PolygonCm, bool)> = self
            .cx
            .sites
            .iter()
            .filter(|s| s.neighborhood == nb.key && s.key.pack != own)
            .filter_map(|s| {
                Some((
                    s.key.clone(),
                    s.name.clone(),
                    s.footprint.to_cm().ok()?,
                    s.titled,
                ))
            })
            .collect();
        for (n, (id, poly, _)) in sites.iter().enumerate() {
            let shape = Shape::Polygon(poly.clone());
            // Within the pack, the later entry answers for the pair.
            for (other_id, other, _) in sites.iter().skip(n + 1) {
                if overlap(&shape, &Shape::Polygon(other.clone()), nb.resolution_cm) {
                    self.error(
                        "geometry.site_overlap",
                        Some(other_id),
                        format!("its footprint overlaps site {id}"),
                        "move or shrink one of the two footprints",
                    );
                }
            }
            for (key, name, other, titled) in &world_sites {
                if overlap(&shape, &Shape::Polygon(other.clone()), nb.resolution_cm) {
                    if *titled {
                        self.error(
                            "policy.property",
                            Some(id),
                            format!("its footprint overlaps {name} ({key}), a titled property"),
                            "a property's ground is its owner's; build beside it",
                        );
                    } else {
                        self.error(
                            "geometry.site_overlap",
                            Some(id),
                            format!("its footprint overlaps site {key} ({name})"),
                            "move or shrink the footprint",
                        );
                    }
                }
            }
        }

        // Structures: inside their site; floors consistent.
        let structures: Vec<(CanonicalId, StructureDef)> = self
            .decoded
            .structures
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (id, def) in &structures {
            if let Ok(poly) = def.footprint.to_cm() {
                let site_poly = match self.resolve(&def.site) {
                    Some(Target::Own(s)) => self
                        .decoded
                        .sites
                        .get(&s)
                        .and_then(|d| d.footprint.to_cm().ok()),
                    Some(Target::World(k)) => self
                        .cx
                        .sites
                        .iter()
                        .find(|s| s.key == k)
                        .and_then(|s| s.footprint.to_cm().ok()),
                    _ => None,
                };
                if let Some(site_poly) = site_poly
                    && !inside(&site_poly, &Shape::Polygon(poly))
                {
                    self.error(
                        "geometry.outside",
                        Some(id),
                        format!("its footprint leaves site {}", def.site),
                        "keep the structure inside its site's footprint",
                    );
                }
            }
            let mut seen: BTreeSet<i16> = BTreeSet::new();
            let mut last: Option<(i16, f64)> = None;
            let mut bad = None;
            if def.floors.is_empty() || def.floors.len() > MAX_FLOORS {
                bad = Some(format!(
                    "a structure has 1 to {MAX_FLOORS} floors, not {}",
                    def.floors.len()
                ));
            }
            for f in &def.floors {
                if !seen.insert(f.index) {
                    bad = Some(format!("floor {} is declared twice", f.index));
                }
                if f.height_m <= 0.0 || !f.height_m.is_finite() || !f.z_offset_m.is_finite() {
                    bad = Some(format!(
                        "floor {}: height and offset are finite, height positive",
                        f.index
                    ));
                }
                if let Some((index, z)) = last
                    && (f.index <= index || f.z_offset_m <= z)
                {
                    bad = Some(format!(
                        "floors go up in index order with rising offsets; floor {} follows floor {index}",
                        f.index
                    ));
                }
                last = Some((f.index, f.z_offset_m));
            }
            if let Some(bad) = bad {
                self.error(
                    "geometry.floors",
                    Some(id),
                    bad,
                    "list floors by rising index, each with a rising z_offset_m and a positive height",
                );
            }
        }

        // Rooms: inside the neighborhood, inside their site and structure, disjoint
        // per surface across the region.
        let rooms = self.pack_rooms();
        for (id, surface, shape, site) in &rooms {
            if !inside(&nb.polygon, shape) {
                self.error(
                    "geometry.outside",
                    Some(id),
                    "its footprint leaves the neighborhood",
                    "keep every corner inside the neighborhood's footprint",
                );
            }
            let site_poly = match site {
                Some(Target::Own(s)) => self
                    .decoded
                    .sites
                    .get(s)
                    .and_then(|d| d.footprint.to_cm().ok()),
                Some(Target::World(k)) => self
                    .cx
                    .sites
                    .iter()
                    .find(|s| &s.key == k)
                    .and_then(|s| s.footprint.to_cm().ok()),
                _ => None,
            };
            if let Some(site_poly) = site_poly
                && !inside(&site_poly, shape)
            {
                self.error(
                    "geometry.outside",
                    Some(id),
                    "its footprint leaves the site it names",
                    "keep the room inside its site, or take the site off it",
                );
            }
            if let SurfaceRef::Floor { structure, .. } = surface {
                let slab = if structure.pack == own {
                    self.decoded
                        .structures
                        .get(&structure.entry)
                        .and_then(|d| d.footprint.to_cm().ok())
                } else {
                    self.cx
                        .structures
                        .iter()
                        .find(|s| &s.key == structure)
                        .and_then(|s| s.footprint.to_cm().ok())
                };
                if let Some(slab) = slab
                    && !inside(&slab, shape)
                {
                    self.error(
                        "geometry.outside",
                        Some(id),
                        format!("its footprint leaves structure {structure}"),
                        "keep a room on a floor inside the structure's footprint",
                    );
                }
            }
        }
        for (n, (id, surface, shape, _)) in rooms.iter().enumerate() {
            // Within the pack, the later entry answers for the pair.
            for (other_id, other_surface, other, _) in rooms.iter().skip(n + 1) {
                if surface == other_surface && overlap(shape, other, nb.resolution_cm) {
                    self.error(
                        "geometry.room_overlap",
                        Some(other_id),
                        format!("its footprint overlaps room {id}"),
                        "rooms partition their surface: move or shrink one",
                    );
                }
            }
            let world: Vec<(RecordKey, Shape)> = self
                .world_rooms(nb.region.as_ref())
                .filter(|r| &r.surface == surface)
                .filter_map(|r| Some((r.key.clone(), r.footprint.shape().ok()?)))
                .collect();
            for (key, other) in &world {
                if overlap(shape, other, nb.resolution_cm) {
                    self.error(
                        "geometry.room_overlap",
                        Some(id),
                        format!("its footprint overlaps room {key}"),
                        "rooms partition their surface: move or shrink the footprint",
                    );
                }
            }
        }

        // Exits: rooms on one surface touch, or the portal says where and how far.
        let exits: Vec<(CanonicalId, ExitDef)> = self
            .decoded
            .exits
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (id, def) in &exits {
            if def.portal.at.is_some() {
                continue;
            }
            let a = self.room_geometry(&rooms, &def.from);
            let b = self.room_geometry(&rooms, &def.to);
            let (Some((sa, shape_a)), Some((sb, shape_b))) = (a, b) else {
                continue;
            };
            if sa != sb {
                self.error(
                    "geometry.portal_gap",
                    Some(id),
                    format!("{} and {} are on different surfaces", def.from, def.to),
                    "give `portal.at` a point or segment where the way between them is",
                );
                continue;
            }
            if shared_threshold(&shape_a, &shape_b).is_none() && def.portal.distance_m.is_none() {
                self.error(
                    "geometry.portal_gap",
                    Some(id),
                    format!("the footprints of {} and {} do not touch", def.from, def.to),
                    "make the footprints share an edge, or give `portal.distance_m` (a walk) or `portal.at`",
                );
            }
        }

        // Features: one geometry, inside the neighborhood (and the room they name).
        let features: Vec<(CanonicalId, FeatureDef)> = self
            .decoded
            .features
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (id, def) in &features {
            let given = usize::from(def.point.is_some())
                + usize::from(def.polyline.is_some())
                + usize::from(def.polygon.is_some());
            if given != 1 {
                self.error(
                    "geometry.feature",
                    Some(id),
                    "a feature has exactly one of `point`, `polyline` and `polygon`",
                    "give it one geometry",
                );
                continue;
            }
            let points: Vec<PointCm> = def
                .point
                .iter()
                .copied()
                .chain(def.polyline.iter().flatten().copied())
                .chain(def.polygon.iter().flat_map(|p| p.0.iter().copied()))
                .filter_map(|p| p.to_cm().ok())
                .collect();
            if points.iter().any(|p| !point_in_or_on(&nb.polygon, *p)) {
                self.error(
                    "geometry.outside",
                    Some(id),
                    "its geometry leaves the neighborhood",
                    "keep every point inside the neighborhood's footprint",
                );
            }
            if let Some(room) = &def.room
                && let Some((_, shape)) = self.room_geometry(&rooms, room)
                && points
                    .iter()
                    .any(|p| !shape.contains(*p) && !on_boundary(&shape.ring(), *p))
            {
                self.warning(
                    "geometry.feature",
                    Some(id),
                    format!("its geometry leaves room {room}"),
                    "keep the feature inside the room it belongs to",
                );
            }
        }
    }

    /// The surface and shape of a room reference: one of this pack's, or the world's.
    fn room_geometry(
        &self,
        rooms: &[(CanonicalId, SurfaceRef, Shape, Option<Target>)],
        reference: &str,
    ) -> Option<(SurfaceRef, Shape)> {
        match self.resolve(reference)? {
            Target::Own(id) => rooms
                .iter()
                .find(|(r, ..)| *r == id)
                .map(|(_, s, shape, _)| (s.clone(), shape.clone())),
            Target::World(key) => self
                .cx
                .rooms
                .iter()
                .find(|r| r.key == key)
                .and_then(|r| Some((r.surface.clone(), r.footprint.shape().ok()?))),
            Target::Limbo => None,
        }
    }

    // ---- 3. topology ------------------------------------------------------------

    fn topology(&mut self) {
        if self.decoded.rooms.is_empty() {
            return;
        }
        let nb = self.neighborhood();
        let rooms = self.pack_rooms();
        let own_public: BTreeSet<CanonicalId> = self
            .decoded
            .rooms
            .iter()
            .filter(|(_, d)| d.access == Access::Public)
            .map(|(id, _)| id.clone())
            .collect();

        // The graph: every exit is an edge `from -> to`, and back when it names a pair.
        // A locked exit is not a public way.
        let mut edges: BTreeMap<Node, Vec<(Node, bool)>> = BTreeMap::new();
        let mut touched: BTreeSet<Node> = BTreeSet::new();
        let mut ingress: Vec<(CanonicalId, Node, Node, bool)> = Vec::new();
        for (id, def) in &self.decoded.exits {
            let (Some(a), Some(b)) = (self.node(&def.from), self.node(&def.to)) else {
                continue;
            };
            let public = def.lock.is_none();
            touched.insert(a.clone());
            touched.insert(b.clone());
            edges
                .entry(a.clone())
                .or_default()
                .push((b.clone(), public));
            ingress.push((id.clone(), a.clone(), b.clone(), public));
            if def.names.len() == 2 {
                edges
                    .entry(b.clone())
                    .or_default()
                    .push((a.clone(), public));
                ingress.push((id.clone(), b, a, public));
            }
        }

        // Seeds: Limbo, the world's rooms (applied and already validated), and the
        // rooms at the neighborhood's access points, and the rooms marked `port.dock` —
        // reached by carrier, doc 18 D-18-19 (a station with no foot route, Stickney,
        // is reached by ship and never by an exit).
        let mut seeds: Vec<Node> = vec![Node::Limbo];
        seeds.extend(
            touched
                .iter()
                .filter(|n| matches!(n, Node::World(_)))
                .cloned(),
        );
        seeds.extend(
            self.decoded
                .rooms
                .iter()
                .filter(|(_, d)| d.markers.iter().any(|m| m == crate::def::PORT_DOCK_MARKER))
                .map(|(id, _)| Node::Own(id.clone())),
        );
        if let Some(nb) = &nb {
            let points: Vec<PointCm> = self
                .cx
                .access_points
                .iter()
                .filter(|a| a.neighborhood == nb.key.pack)
                .filter_map(|a| a.point.xy().to_cm().ok())
                .collect();
            for p in points {
                let at: Vec<CanonicalId> = rooms
                    .iter()
                    .filter(|(_, surface, shape, _)| {
                        *surface == SurfaceRef::Ground
                            && (shape.contains(p) || on_boundary(&shape.ring(), p))
                    })
                    .map(|(id, ..)| id.clone())
                    .collect();
                if at.is_empty() {
                    if self.pack.manifest().kind() == PackKind::Neighborhood {
                        self.warning(
                            "topology.access_point",
                            None,
                            format!(
                                "no room lies at the access point ({}, {}); travelers arriving there land nowhere",
                                crate::geom::metres(p.x),
                                crate::geom::metres(p.y)
                            ),
                            "put an outdoor room over the access point, or move the connection's end",
                        );
                    }
                } else {
                    seeds.extend(at.into_iter().map(Node::Own));
                }
            }
        }

        // Reach, over public ways.
        let mut seen: BTreeSet<Node> = seeds.iter().cloned().collect();
        let mut queue: Vec<Node> = seeds;
        while let Some(node) = queue.pop() {
            if let Some(next) = edges.get(&node) {
                for (to, public) in next {
                    if *public && seen.insert(to.clone()) {
                        queue.push(to.clone());
                    }
                }
            }
        }
        for id in &own_public {
            let node = Node::Own(id.clone());
            let anchored =
                self.decoded.rooms.get(id).is_some_and(|d| {
                    matches!(d.footprint, crate::def::FootprintDef::Anchor { .. })
                });
            if !touched.contains(&node) {
                if anchored || seen.contains(&node) {
                    continue;
                }
                self.error(
                    "topology.isolated",
                    Some(id),
                    "no exit leads in or out",
                    "give it an exit, put it at an access point, or make it an anchor (a landing)",
                );
            } else if !seen.contains(&node) {
                self.error(
                    "topology.unreachable",
                    Some(id),
                    "no public way reaches it from Limbo, an access point or the world",
                    "join it to a reachable room with an unlocked exit, mark it `port.dock` if \
                     ships reach it, or make it restricted",
                );
            }
        }

        // Private ingress: a way into a restricted room from public ground is locked.
        for (id, from, to, public) in ingress {
            let Node::Own(target) = &to else {
                continue;
            };
            let restricted = self
                .decoded
                .rooms
                .get(target)
                .is_some_and(|d| d.access == Access::Restricted);
            if !restricted || !public {
                continue;
            }
            let from_public = match &from {
                Node::Limbo => true,
                Node::Own(f) => own_public.contains(f),
                Node::World(k) => self
                    .cx
                    .rooms
                    .iter()
                    .find(|r| &r.key == k)
                    .is_none_or(|r| r.access == Access::Public),
            };
            if from_public {
                self.error(
                    "topology.private_ingress",
                    Some(&id),
                    format!("an unlocked way leads into restricted room {target}"),
                    "lock the exit (`\"lock\": \"staff\"` or an expression), or make the room public",
                );
            }
        }
    }

    fn node(&self, reference: &str) -> Option<Node> {
        match self.resolve(reference)? {
            Target::Own(id) => self
                .decoded
                .rooms
                .contains_key(&id)
                .then_some(Node::Own(id)),
            Target::World(key) => self
                .cx
                .rooms
                .iter()
                .any(|r| r.key == key)
                .then_some(Node::World(key)),
            Target::Limbo => Some(Node::Limbo),
        }
    }

    // ---- 4. budgets --------------------------------------------------------------

    fn budgets(&mut self) {
        if let Some(nb) = self.neighborhood() {
            let own = self.own_id().clone();
            let standing = self
                .cx
                .rooms
                .iter()
                .filter(|r| r.neighborhood == nb.key && r.key.pack != own)
                .count();
            let total = standing + self.decoded.rooms.len();
            let maximum = usize::try_from(nb.room_budget.maximum).unwrap_or(usize::MAX);
            if total > maximum {
                self.error(
                    "budget.rooms",
                    None,
                    format!(
                        "the neighborhood would hold {total} rooms; its budget allows {maximum}"
                    ),
                    "fewer rooms, or raise `room_budget.maximum` on the neighborhood",
                );
            }
            let bbox = nb.polygon.bbox();
            let res = i64::from(nb.resolution_cm.max(1));
            let cells = (bbox.width() / res)
                .max(1)
                .saturating_mul((bbox.height() / res).max(1));
            let per_tile = i64::from(self.cx.limits.tile_max_cells);
            if cells
                > per_tile.saturating_mul(
                    i64::try_from(crate::record::MAX_TILES_PER_NEIGHBORHOOD).unwrap_or(64),
                )
            {
                self.warning(
                    "budget.tiles",
                    None,
                    format!(
                        "the neighborhood needs {cells} cells at {} cm; it rasterizes coarser than asked",
                        nb.resolution_cm
                    ),
                    "a coarser `resolution_cm`, or a smaller footprint",
                );
            }
        }
        let mut per_room: BTreeMap<String, usize> = BTreeMap::new();
        for def in self.decoded.features.values() {
            if let Some(room) = &def.room {
                *per_room.entry(room.clone()).or_default() += 1;
            }
        }
        for (room, count) in per_room {
            if count > MAX_FEATURES_PER_ROOM {
                self.error(
                    "budget.features",
                    None,
                    format!(
                        "room {room} carries {count} features; at most {MAX_FEATURES_PER_ROOM}"
                    ),
                    "fewer features in the room",
                );
            }
        }
        if self.decoded.npcs.len() > MAX_NPCS_PER_PACK {
            self.error(
                "budget.npcs",
                None,
                format!(
                    "{} npc entries; a pack carries at most {MAX_NPCS_PER_PACK}",
                    self.decoded.npcs.len()
                ),
                "fewer people, or a second pack",
            );
        }
        let lore: Vec<(CanonicalId, usize)> = self
            .decoded
            .lore
            .iter()
            .map(|(id, v)| (id.clone(), v.to_string().len()))
            .collect();
        for (id, bytes) in lore {
            if bytes > MAX_LORE_BYTES {
                self.error(
                    "budget.lore",
                    Some(&id),
                    format!("{bytes} bytes of lore; at most {MAX_LORE_BYTES}"),
                    "shorter facts, or another pool",
                );
            }
        }
        let texts: Vec<(CanonicalId, &'static str, usize)> = self
            .decoded
            .rooms
            .iter()
            .map(|(id, d)| (id.clone(), "desc", d.desc.len()))
            .chain(
                self.decoded
                    .items
                    .iter()
                    .map(|(id, d)| (id.clone(), "desc", d.desc.len())),
            )
            .chain(
                self.decoded
                    .hooks
                    .iter()
                    .map(|(id, d)| (id.clone(), "text", d.text.len())),
            )
            .collect();
        for (id, field, bytes) in texts {
            if bytes > MAX_TEXT_BYTES {
                self.error(
                    "budget.text",
                    Some(&id),
                    format!("{field} is {bytes} bytes; at most {MAX_TEXT_BYTES}"),
                    "shorter prose",
                );
            }
        }
        // Doc 18 cut IC12 (carry-forward IC1 → IC12): the two text bounds the applier
        // enforced alone — a room's `short_desc` and a neighborhood's `ambience` — are
        // reported here too, so a pack fails validation, never the apply.
        let short: Vec<(CanonicalId, usize)> = self
            .decoded
            .rooms
            .iter()
            .map(|(id, d)| (id.clone(), d.short_desc.len()))
            .filter(|(_, bytes)| *bytes > MAX_SHORT_DESC_BYTES)
            .collect();
        for (id, bytes) in short {
            self.error(
                "budget.text",
                Some(&id),
                format!("short_desc is {bytes} bytes; at most {MAX_SHORT_DESC_BYTES}"),
                "one line",
            );
        }
        let ambience: Option<(CanonicalId, Vec<String>)> = self
            .decoded
            .neighborhood
            .as_ref()
            .map(|(id, def)| (id.clone(), def.ambience.clone()));
        if let Some((id, lines)) = ambience {
            if lines.len() > MAX_AMBIENCE_LINES {
                self.error(
                    "budget.text",
                    Some(&id),
                    format!(
                        "ambience has {} lines; at most {MAX_AMBIENCE_LINES}",
                        lines.len()
                    ),
                    "fewer ambient lines",
                );
            }
            for (n, line) in (1_usize..).zip(&lines) {
                if line.trim().is_empty() {
                    self.error(
                        "budget.text",
                        Some(&id),
                        format!("ambience line {n} is blank"),
                        "remove the blank line",
                    );
                } else if line.len() > MAX_AMBIENCE_LINE_BYTES {
                    self.error(
                        "budget.text",
                        Some(&id),
                        format!(
                            "ambience line {n} is {} bytes; at most {MAX_AMBIENCE_LINE_BYTES}",
                            line.len()
                        ),
                        "a shorter line",
                    );
                }
            }
        }
        let secrets: Vec<(CanonicalId, usize)> = self
            .decoded
            .secrets
            .iter()
            .map(|(id, d)| (id.clone(), d.truth.len()))
            .collect();
        for (id, bytes) in secrets {
            if bytes > MAX_SECRET_BYTES {
                self.error(
                    "budget.text",
                    Some(&id),
                    format!("truth is {bytes} bytes; at most {MAX_SECRET_BYTES}"),
                    "a shorter truth",
                );
            }
        }
    }

    // ---- 5. policy ----------------------------------------------------------------

    fn policy(&mut self) {
        let nb = self.neighborhood();
        // Site kinds: hints, never imposed.
        let sites: Vec<(CanonicalId, String)> = self
            .decoded
            .sites
            .iter()
            .map(|(id, d)| (id.clone(), d.site_kind.clone()))
            .collect();
        for (id, kind) in &sites {
            if !SITE_KINDS.contains(&kind.as_str()) {
                self.warning(
                    "policy.kind",
                    Some(id),
                    format!("site kind {kind:?} is not in the taxonomy"),
                    format!(
                        "one of {}; another word is allowed and staff-run",
                        SITE_KINDS.join(", ")
                    ),
                );
            }
        }
        if self.cx.player_build
            && let Some(nb) = &nb
        {
            if !nb.hydration.enabled {
                self.error(
                    "policy.hydration",
                    None,
                    "the neighborhood does not take player-built sites",
                    "build where the policy allows it, or ask staff",
                );
            }
            let adding = u32::try_from(self.decoded.sites.len()).unwrap_or(u32::MAX);
            if nb.player_sites.saturating_add(adding) > nb.hydration.max_player_sites {
                self.error(
                    "policy.player_sites",
                    None,
                    format!(
                        "the neighborhood holds {} player-built sites of {} allowed",
                        nb.player_sites, nb.hydration.max_player_sites
                    ),
                    "wait for a review to retire one, or build elsewhere",
                );
            }
            // No shortcut through private space: an exit from the new site into a
            // restricted room or a titled property of the world.
            let exits: Vec<(CanonicalId, String, String)> = self
                .decoded
                .exits
                .iter()
                .map(|(k, v)| (k.clone(), v.from.clone(), v.to.clone()))
                .collect();
            for (id, from, to) in &exits {
                for reference in [from, to] {
                    if let Some(Target::World(key)) = self.resolve(reference)
                        && let Some(room) = self.cx.rooms.iter().find(|r| r.key == key)
                        && (room.access == Access::Restricted || room.titled)
                    {
                        self.error(
                            "policy.shortcut",
                            Some(id),
                            format!(
                                "{reference} is private space; a player's site may not open into it"
                            ),
                            "join the site to a public room or the street",
                        );
                    }
                }
            }
        }
        // The theme's banned vocabulary.
        let banned: Vec<String> = self
            .cx
            .theme
            .as_ref()
            .map(|t| {
                t.banned
                    .iter()
                    .map(|w| w.trim().to_lowercase())
                    .filter(|w| !w.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if banned.is_empty() {
            return;
        }
        let mut prose: Vec<(CanonicalId, String, String)> = Vec::new();
        for (id, d) in &self.decoded.rooms {
            prose.push((id.clone(), "name".into(), d.name.clone()));
            prose.push((id.clone(), "desc".into(), d.desc.clone()));
            for (k, v) in &d.details {
                prose.push((id.clone(), format!("detail {k}"), v.clone()));
            }
        }
        for (id, d) in &self.decoded.items {
            prose.push((id.clone(), "desc".into(), d.desc.clone()));
        }
        for (id, d) in &self.decoded.hooks {
            prose.push((id.clone(), "text".into(), d.text.clone()));
        }
        for (id, field, text) in prose {
            let words: BTreeSet<String> = text
                .split(|c: char| !c.is_alphanumeric() && c != '\'')
                .map(|w| w.trim_matches('\'').to_lowercase())
                .filter(|w| !w.is_empty())
                .collect();
            let hits: Vec<&str> = banned
                .iter()
                .filter(|b| {
                    words.contains(*b)
                        || (b.contains(' ') && text.to_lowercase().contains(b.as_str()))
                })
                .map(String::as_str)
                .collect();
            if !hits.is_empty() {
                self.warning(
                    "policy.banned_word",
                    Some(&id),
                    format!("{field} uses {}, which the theme bans", hits.join(", ")),
                    "another word; see @atlas/theme/show",
                );
            }
        }
    }

    // ---- 6. secrets ---------------------------------------------------------------

    fn secrets(&mut self) {
        let secrets: Vec<(CanonicalId, String, crate::record::Reveal)> = self
            .decoded
            .secrets
            .iter()
            .map(|(id, d)| (id.clone(), d.truth.clone(), d.reveal))
            .collect();
        if secrets.is_empty() {
            return;
        }
        for (id, _, reveal) in &secrets {
            self.info(
                "secrets.staff_only",
                Some(id),
                format!("a secret ({reveal:?}): stored, never in any prompt"),
                "nothing to do; staff reveal it by hand",
            );
        }
        // Every prompt field of every npc, and every lore fact that is not itself
        // secret.
        let mut fields: Vec<(CanonicalId, String, String)> = Vec::new();
        for (id, npc) in &self.decoded.npcs {
            collect_prompt_fields(npc, "", &mut |path, text| {
                fields.push((id.clone(), path, text));
            });
        }
        for (id, lore) in &self.decoded.lore {
            if let Some(entries) = lore.get("entries").and_then(|e| e.as_array()) {
                for (n, e) in entries.iter().enumerate() {
                    let secret = e.get("tier").and_then(|t| t.as_str()) == Some("secret");
                    if secret {
                        continue;
                    }
                    if let Some(fact) = e.get("fact").and_then(|f| f.as_str()) {
                        fields.push((id.clone(), format!("entries[{n}].fact"), fact.to_string()));
                    }
                }
            }
        }
        for (secret_id, truth, _) in &secrets {
            let needle = normalize(truth);
            if needle.is_empty() {
                continue;
            }
            for (id, path, text) in &fields {
                let hay = normalize(text);
                let leaked = if needle.len() >= MIN_SECRET_NEEDLE {
                    hay.contains(&needle)
                } else {
                    hay == needle
                };
                if leaked {
                    self.error(
                        "secrets.leak",
                        Some(id),
                        format!("{path} carries the truth of secret {secret_id}"),
                        "a secret never enters a prompt field: take it out of persona, background, goals, knows and fallback, and out of public lore",
                    );
                }
            }
        }
    }

    // ---- 7. supported definitions -----------------------------------------------

    fn supported(&mut self) {
        let connections: Vec<ServiceCheck> = self
            .decoded
            .connections
            .iter()
            .map(|(id, c)| {
                (
                    id.clone(),
                    c.modes.clone(),
                    c.service
                        .as_ref()
                        .map(|s| (s.vehicle.pack.clone(), s.vehicle.entry.clone())),
                )
            })
            .collect();
        for (id, modes, service) in &connections {
            for mode in modes {
                if !self.cx.modes.iter().any(|m| m == mode) {
                    self.info(
                        "support.staff_run",
                        Some(id),
                        format!("travel mode {mode:?} is not one this server times; it walks at a walking pace"),
                        "one of the known modes, or leave it and staff run it",
                    );
                }
            }
            if let Some((pack, entry)) = service {
                let key = CanonicalId::new(pack)
                    .ok()
                    .zip(CanonicalId::new(entry).ok())
                    .map(|(p, e)| RecordKey::new(p, e));
                let own_room = key.as_ref().is_some_and(|k| {
                    k.pack == *self.own_id() && self.decoded.rooms.contains_key(&k.entry)
                });
                let world_room = key
                    .as_ref()
                    .is_some_and(|k| self.cx.rooms.iter().any(|r| &r.key == k));
                if !own_room && !world_room {
                    self.info(
                        "support.staff_run",
                        Some(id),
                        format!("service vehicle {pack}:{entry} is not an applied room; the service waits for it"),
                        "apply the pack that makes the vehicle room, or name one that exists",
                    );
                }
            }
        }
        // An NPC's abilities are the ruleset's; the atlas marks unknown ones staff-run
        // rather than inventing them.
        let abilities: Vec<(CanonicalId, Vec<String>)> = self
            .decoded
            .npcs
            .iter()
            .map(|(id, npc)| {
                (
                    id.clone(),
                    npc.get("abilities")
                        .and_then(|a| a.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(ToString::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            })
            .collect();
        for (id, names) in abilities {
            for name in names {
                self.info(
                    "support.staff_run",
                    Some(&id),
                    format!(
                        "ability {name:?} is the ruleset's to provide; until it does, staff run it"
                    ),
                    "nothing to do here",
                );
            }
        }
    }

    // ---- 8. diff safety -----------------------------------------------------------

    fn diff_safety(&mut self) {
        let own = self.own_id().to_string();
        let Some(latch) = self.cx.latches.get(&own).cloned() else {
            return;
        };
        if latch.retired {
            return;
        }
        let version = self.pack.manifest().version();
        if version < latch.version {
            self.error(
                "diff.downgrade",
                None,
                format!(
                    "version {version} is below the applied version {}",
                    latch.version
                ),
                "bump the pack's version",
            );
        }
        let retired: Vec<(String, LatchedEntry)> = latch
            .entries
            .iter()
            .filter(|(id, _)| {
                CanonicalId::new(id)
                    .ok()
                    .is_none_or(|i| self.pack.entry(&i).is_none())
            })
            .map(|(id, e)| (id.clone(), e.clone()))
            .collect();
        for (id, entry) in retired {
            if entry.kind != "room" {
                continue;
            }
            let Ok(entry_id) = CanonicalId::new(&id) else {
                continue;
            };
            let key = self.own_key(&entry_id);
            let Some(room) = self.cx.rooms.iter().find(|r| r.key == key).cloned() else {
                continue;
            };
            if room.occupants > 0 {
                self.error(
                    "diff.live_reference",
                    Some(&entry_id),
                    format!(
                        "retiring it would strand {} character(s) standing in it",
                        room.occupants
                    ),
                    "keep the room, or wait until it is empty",
                );
            }
            if !room.entered_from.is_empty() {
                self.error(
                    "diff.live_reference",
                    Some(&entry_id),
                    format!(
                        "an exit from outside the pack leads into it ({})",
                        room.entered_from.join(", ")
                    ),
                    "keep the room, or retire that exit first",
                );
            }
        }
    }
}

/// A connection's modes and service vehicle, for the supported-definitions rule.
type ServiceCheck = (CanonicalId, Vec<String>, Option<(String, String)>);

/// The neighborhood a pack works in, resolved.
struct NeighborhoodShape {
    key: RecordKey,
    region: Option<RecordKey>,
    polygon: PolygonCm,
    resolution_cm: u16,
    room_budget: Budget,
    hydration: HydrationPolicy,
    player_sites: u32,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Whether every point of `inner`'s ring lies inside or on `outer`.
fn inside(outer: &PolygonCm, inner: &Shape) -> bool {
    inner
        .ring()
        .points()
        .iter()
        .all(|p| point_in_or_on(outer, *p))
}

fn point_in_or_on(poly: &PolygonCm, p: PointCm) -> bool {
    poly.contains(p) || on_boundary(poly, p)
}

/// Whether `p` lies on an edge of `poly`.
fn on_boundary(poly: &PolygonCm, p: PointCm) -> bool {
    poly.edges().any(|(a, b)| {
        let cross = i128::from(b.x - a.x) * i128::from(p.y - a.y)
            - i128::from(b.y - a.y) * i128::from(p.x - a.x);
        cross == 0
            && p.x >= a.x.min(b.x)
            && p.x <= a.x.max(b.x)
            && p.y >= a.y.min(b.y)
            && p.y <= a.y.max(b.y)
    })
}

/// Lower-case, whitespace-collapsed text for the secret search.
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Every string under a prompt-bearing key of an NPC definition, at any depth:
/// `persona`, `background`, `goals`, `knows`, `fallback`, `speech_style`.
fn collect_prompt_fields(
    value: &serde_json::Value,
    path: &str,
    out: &mut dyn FnMut(String, String),
) {
    const PROMPT_KEYS: [&str; 7] = [
        "persona",
        "background",
        "goals",
        "knows",
        "fallback",
        "fallback_lines",
        "speech_style",
    ];
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let here = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                if PROMPT_KEYS.contains(&k.as_str()) {
                    collect_strings(v, &here, out);
                } else {
                    collect_prompt_fields(v, &here, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for (n, v) in items.iter().enumerate() {
                collect_prompt_fields(v, &format!("{path}[{n}]"), out);
            }
        }
        _ => {}
    }
}

fn collect_strings(value: &serde_json::Value, path: &str, out: &mut dyn FnMut(String, String)) {
    match value {
        serde_json::Value::String(s) => out(path.to_string(), s.clone()),
        serde_json::Value::Array(items) => {
            for (n, v) in items.iter().enumerate() {
                collect_strings(v, &format!("{path}[{n}]"), out);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                collect_strings(v, &format!("{path}.{k}"), out);
            }
        }
        _ => {}
    }
}
