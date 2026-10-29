//! **The TacSim module state and its documents** (namespace `tacsim`), on
//! `ruleset-shadowrun/src/economy/store.rs`'s exact pattern.
//!
//! | doc | class | corrupt → |
//! |---|---|---|
//! | `tacsim:pack:{id}@{version}` | ledger | **skipped loudly**, and the world boots. The **key still counts**: a version whose row will not parse stays *published*, so it is never quietly treated as new. What is lost with the body is the **digest that version was first published under**, and there is no way to recover it — so boot repairs the row from the loaded pack and **pages level 2 naming the pack** (08c §4.6), because for that one version the in-place-re-publication gate below is disarmed until an operator re-checks it. A subject seeded under it still fails closed rather than floating onto a newer version |
//! | `tacsim:seed:{pack}@{version}` | latch | **skipped loudly**, and the world boots — but the **key still bars a re-seed**. This is the one place where losing the body must not lose the fact: "we cannot read the latch" has to mean *do not seed again*, never *seed again* (§3.1 invariant (ii)). The cost is that the seeded objects' pack-local names are gone, so the corridor's routes cannot be re-materialized until `@tacsim/seed` is re-run against a fresh store; the objects themselves are untouched |
//! | `tacsim:pin:{subject}` | pin | **skipped loudly**. A lost pin does **not** float its subject onto a newer version: resolution refuses, naming the pack, because §3.1 invariant (i)'s whole point is that a published contract's assumptions never shift underneath it |
//! | `tacsim:tutorial:{account}` | per-account | **skipped loudly**; that account reads its hints again (16b cut AH.1: `crate::match_::tutorial`) |
//! | `tacsim:mindsrc:{sha256}` | immutable source | **skipped loudly** when its `sha256` disagrees with its key or its bytes do not hash to it (17b cut M3.3; doc 17 §13). The **key still counts**: the boot never rewrites a document under a key it holds, so a tampered stock source stays unreadable rather than silently replaced, and a scripted seal naming it refuses its resume by name (`crate::match_::resume`) |
//! | `tacsim:port:{pack}/{entry}` | applier map (doc 18 cut IC3) | **skipped loudly, and the key still bars re-creation**: the entry's next apply refuses before any write, naming the key, and `@tacsim/port status` reports the map unreadable; repair is a staff act on the document (doc 18 §4.7) |
//! | `tacsim:notice:{obj}` | per-character queue (doc 18 cut IC5: `crate::port_tick`'s reconnect notices) | **dropped loudly** for that character; the key bars nothing, and the next notice rewrites it |
//! | `tacsim:coop:{account}` | per-gateway-account enrolment (doc 18 cut IC6: `crate::port::coop`) | **skipped loudly, and the key still bars re-enrolment**: that account's `+coop/enrol` refuses, and staff repair the document; the operate gate never reads it (it reads the live lease) |
//! | `tacsim:voyage:{hull}` | per-hull sailing plan (doc 18 cut IC9: `crate::port::voyage`) — the filed passage and the hull's burn sequence | **skipped loudly, and the key stays claimed**: that hull's `+ship/undock` refuses (its papers cannot be read) and the launch arm skips it, logged once; staff repair the document |
//! | `tacsim:orders:{hull}` | per-hull sailing plan (doc 18 cut IC11: `crate::port::orders`) — the queued orders, their log and the step in flight | **skipped loudly, and the key stays claimed**: that hull's `+ship/orders` and `+ship/order` refuse (its plan cannot be read), the order arm skips it and pages once a boot; staff repair the document by editing it (a deleted one restarts its order ids only past the hull's recent tickets) |
//! | `tacsim:tvlease:{lease}` | per-lease rig record (doc 20 cut SM.3a: `crate::port::tv`) — a terrain vehicle Occator leased into a Dray-P's bay, its slot, its deposit hold and its term | **skipped loudly, and the key stays claimed**: the record reads unreadable, which bars only its own lease — its pilot's `+tv` reads dark and the recall holds the hull for staff; staff repair the document |
//! | `tacsim:tv:{tv}` | per-rig projection (doc 20 cut SM.3b: `crate::surface::excursion::TvRec`) — where a rig is, its open excursion, the act in flight, what the tick applied and told, a sixteen-line log | **skipped loudly, read unreadable, and it bars nothing**: the projection decides nothing (every decision reads the typed `tacsim.excursion` family), and the `tacsim.surface` tick's next write replaces it |
//! | `tacsim:mined:{site}` | per-site ring of the last 64 mined cycles (doc 20 cut SM.7: `crate::surface::mined::MinedRing`) — the fact `tacsim.surface.mined`, written by the `tacsim.surface` tick's tell; read by nothing yet | **dropped loudly, and it bars nothing**: the ring reads unreadable, and the tick's next write replaces it |
//! | `tacsim:claimlog:{site}` | per-site claim log (doc 20 cut SM.10: `crate::surface::claims::ClaimLog`) — the lapses the site clerk chronicled, the contest refunds it paid, the expired fees it paged and the paper's adjudication contract, written only by the clerk's claim steps | **dropped loudly (paged once at load), and it bars nothing**: a re-chronicle is then guarded by `mush_canon::records_for_source`, a refund is re-attempted only under its own idempotency key, and the paper contract is re-found by its issuer and name before any re-file; the clerk's next write replaces it |
//! | `tacsim:tvpref:{character}` | per-character `+tv` preferences (doc 20 cut SM.4a: `crate::port::tv::TvPrefs`) — whether the character hears other rigs' soundings | **dropped loudly**: the character reads as not quiet (they hear soundings again) and the key bars nothing; the next `+tv/quiet` rewrites it |
//! | `tacsim:licence:{account}` | per-gateway-account prospecting licence (doc 20 cut SM.5: `crate::port::licence`) — its holder, entitlement account, expiry, grace and last fee contract | **skipped loudly, and the key still bars the account**: that account's licence reads unreadable, so `+licence/buy` and every licence-gated act (`+tv/lease` now) refuse, and staff repair the document |
//! | `tacsim:geology` | world pin (doc 20 cut SM.1; Q-20-12) — the geology key's fingerprint | **skipped loudly, and the key still bars a rewrite**: the geology layer reads disabled ("tacsim:geology cannot be read") on the readiness line; staff repair the document, after restoring the key directory |
//! | `tacsim:registry_unwarranted` | the unwarranted chronicle (doc 17h cut M8d D6: `crate::minds::registry_inspection::unwarranted`) — per hull, the newest inspection read and the findings kept | **skipped loudly, and the key still bars the chronicle**: nothing more is chronicled (a reset cursor would chronicle old findings twice and move the Red Dies clock twice), the public fact reads none, and staff repair the document |
//! | `tacsim:meta` | allocators | tolerated field by field, floors re-derived from every **parsed key**, loaded or not (E6b's R8 finding). Losing it costs nothing |
//!
//! Nothing **in this loader** fails the boot, and the direction of each loss is
//! deliberate. A malformed pack **file** is the opposite answer — it refuses the boot
//! and names the file — because a file is the operator's declared intent and a stored
//! document is this world's accumulated history: refusing to start over a typo an
//! operator can fix is right, and refusing to start over one row of history is not
//! (10b §4 rule 2).
//!
//! # Where a stored row DOES refuse the boot
//!
//! One place, and it is a statement about a **file**, not about the row: a ledger row
//! that parses and whose digest disagrees with the loaded pack for that exact
//! `(id, version)` means the operator re-signed a published version in place.
//! `crate::boot_reconcile` refuses the boot naming the file and both digests (the owner
//! decision of 2026-09-05) — never page-and-drop, never adopt — because a seeded world
//! depends on the exact bytes and a published version is never mutated: a change
//! publishes a **new version that coexists**. The row is history; the disagreement is
//! the typo.
//!
//! Both loaders apply the **identity rule** (DL-07): a document whose record disagrees
//! with its own key is dropped loudly, never trusted.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, ModuleOp, ModuleWrite, ObjId, Outbox, World};
use mush_sim::{DistanceLaw, DriveProfile, LegDef, RouteDef};

use crate::sky::Sky;
use smol_str::SmolStr;

use crate::config::{LoadedPack, TacsimConfig};
use crate::pack::{PackEntry, RouteEntry, RulesPack};
use crate::record::{PackRecord, PinRecord, SeedApplication};

/// The module namespace (the module's own name).
pub const NS: &str = "tacsim";

/// The allocator/watermark document.
pub const META_KEY: &str = "tacsim:meta";

/// The namespace's schema version.
pub const SCHEMA_VERSION: u16 = 1;

/// The most rows one `@tacsim` listing page renders — `pack list`, `pack show`,
/// `pack verify` and `pin list`, all four (independent verify B1/B3).
///
/// Bounded on records **visited**, not matched (D-LBK-5), so a viewer who may not see
/// most of them still pays one page of work.
pub const PACK_PAGE: usize = 32;

/// The most pins one world may carry (independent verify B4).
///
/// `tacsim:pin:{subject}` was the one document family in the crate with no cap, no
/// `Del` path and no verb to see it: the family grew monotonically, was reloaded in
/// full at every boot and was invisible from the game. 10b §7 gives a cap to every
/// other list in the programme; a pin is a per-subject migration decision, so this sits
/// with the programme's other per-subject caps.
///
/// Unlike pins after B4, the tutorial family (`tacsim:tutorial:{account}`, 16b cut
/// AH.1) deliberately has no cap and no `Del`, and answers B4 by construction: it is
/// bounded one-to-one by accounts, which have no deletion path of their own, each
/// document is one bool and at most six words (five until 17b cut M5.6's `mind`), and
/// every account can see its own from the game through `@tutorial`.
pub const MAX_PINS: usize = 128;

/// `tacsim:pack:{id}@{version}`.
#[must_use]
pub fn pack_key(id: &CanonicalId, version: u32) -> SmolStr {
    SmolStr::new(format!("tacsim:pack:{id}@{version}"))
}

/// `tacsim:seed:{pack}@{version}` — **the version is in the key** (OPEN 5, §4 rule 3),
/// because invariant (ii) is "at most once per `(pack, version)`".
#[must_use]
pub fn seed_key(pack: &CanonicalId, version: u32) -> SmolStr {
    SmolStr::new(format!("tacsim:seed:{pack}@{version}"))
}

/// `tacsim:pin:{subject}`.
#[must_use]
pub fn pin_key(subject: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("tacsim:pin:{subject}"))
}

/// The prefix of every sealed mind source's document (17b cut M3.3).
pub(crate) const MINDSRC_PREFIX: &str = "tacsim:mindsrc:";

/// **`tacsim:mindsrc:<sha256>`** (17b cut M3.3; doc 17 §13): the key a sealed
/// mind source is held under, its digest as the tree writes every digest (64
/// lowercase base-16 characters). Content-addressed: the key IS the bytes'
/// digest, so a source is written once and never rewritten.
#[must_use]
pub(crate) fn mindsrc_key(digest: &[u8; 32]) -> SmolStr {
    SmolStr::new(format!(
        "{MINDSRC_PREFIX}{}",
        crate::arena::digest_text(digest)
    ))
}

/// **One sealed mind source, whole** (17b cut M3.3; doc 17 §13): the document
/// at `mindsrc_key` — the bytes' SHA-256 (64 lowercase base-16 characters,
/// the key's own tail), the engine ABI it was first stored under, and the
/// source text. A bounded whole-document record, written once by
/// `put_mind_source` and never rewritten.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindSourceRecord {
    /// SHA-256 of `bytes`, as the key writes it.
    pub sha256: String,
    /// The `MIND_ABI` it was stored under.
    pub abi: String,
    /// The source.
    pub bytes: String,
}

/// Split an `{id}@{version}` key tail. `None` when either half is unusable, which is
/// what makes an unreadable key a *loud skip* rather than a silent zero.
fn split_versioned(rest: &str) -> Option<(CanonicalId, u32)> {
    let (id, version) = rest.rsplit_once('@')?;
    let id = CanonicalId::parse_canonical(id).ok()?;
    let version: u32 = version.parse().ok()?;
    (version > 0).then_some((id, version))
}

/// Why resolving a subject to a pack version refuses.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ResolveError {
    /// No loaded pack declares this subject.
    #[error("nothing loaded declares {subject}")]
    Unknown {
        /// The subject asked for.
        subject: String,
    },
    /// The version this subject resolves to is not loaded any more.
    ///
    /// **This is the fail-closed answer**, and it is deliberately not "use the newest
    /// one": §3.1 invariant (i) says an unpinned consumer of a retired version fails
    /// closed rather than silently upgrading, because a published hauling contract's
    /// route assumptions must not shift underneath it (T1).
    #[error(
        "{subject} resolves to {pack}@{version}, which this server no longer loads; it is {how} \
         and will not be silently moved to another version"
    )]
    Retired {
        /// The subject asked for.
        subject: String,
        /// The pack it resolves to.
        pack: String,
        /// The version it resolves to.
        version: u32,
        /// How it resolved — `pinned` or `seeded`.
        how: &'static str,
    },
}

/// The in-RAM TacSim slot — the single runtime source of truth.
#[derive(Clone, Debug, Default)]
pub struct TacsimStore {
    pub(crate) relay_mobile: Option<crate::relay_mobile::Programme>,
    pub(crate) relay_mobile_held: bool,
    pub(crate) relay_world: Option<crate::relay_world::Story>,
    pub(crate) relay_world_held: bool,
    /// Doc 17h cut M8d D6: the unwarranted chronicle (`tacsim:registry_unwarranted`).
    pub(crate) registry_unwarranted:
        Option<crate::minds::registry_inspection::unwarranted::Chronicle>,
    /// Whether a `tacsim:registry_unwarranted` key was read, loaded or not.
    pub(crate) registry_unwarranted_held: bool,
    pub(crate) relay_foundry: Option<crate::relay_foundry::Foundry>,
    pub(crate) relay_foundry_held: bool,
    pub(crate) relay_proposals: BTreeMap<ObjId, crate::relay_foundry::Proposal>,
    /// Frozen at wiring.
    pub(crate) cfg: TacsimConfig,
    /// The packs the host read and this module verified, keyed `(id, version)`.
    /// **Not persisted**: a pack is a file, and the ledger row below is what the
    /// world remembers about it.
    pub(crate) loaded: BTreeMap<(CanonicalId, u32), LoadedPack>,
    /// `tacsim:pack:{id}@{version}` — the published-version ledger.
    pub(crate) published: BTreeMap<(CanonicalId, u32), PackRecord>,
    /// Every `(id, version)` a `tacsim:pack:` **key** claimed, loaded or not (R8).
    pub(crate) published_keys: BTreeSet<(CanonicalId, u32)>,
    /// `tacsim:seed:{pack}@{version}` — the idempotency latches.
    pub(crate) seeded: BTreeMap<(CanonicalId, u32), SeedApplication>,
    /// Every `(pack, version)` a `tacsim:seed:` **key** claimed, loaded or not (R8).
    /// **This, not `seeded`, is what bars a second `commit`.**
    pub(crate) seeded_keys: BTreeSet<(CanonicalId, u32)>,
    /// **pack → applied version**, re-derived from `seeded_keys` on every write and
    /// at boot. Never authority in its own right; the keys are.
    pub(crate) applied: BTreeMap<CanonicalId, u32>,
    /// `tacsim:pin:{subject}`.
    pub(crate) pins: BTreeMap<CanonicalId, PinRecord>,
    /// The materialized routes, re-derived from the loaded packs and the seed
    /// latches' placement maps. A projection, never a document.
    ///
    /// Keyed by `(pack, version, route id)` and **not** by the route's pack-local id
    /// alone (independent verify S4): a route id is pack-local by definition, so a
    /// single-id key let a second seeded pack whose id merely sorts later silently
    /// replace the corridor's `ceres_phobos` — deterministic, so not a replay
    /// divergence, but a fail-open shadowing of the very definition T1 rests on, with
    /// no way to ask for a specific pack's route.
    pub(crate) routes: BTreeMap<(CanonicalId, u32, CanonicalId), RouteDef>,
    /// Per latch, the pack-local symbols whose placed object this world no longer has
    /// (independent verify S3). Re-derived at boot against the live world, never a
    /// document; a route through one of them is skipped rather than materialized over
    /// a tombstone.
    pub(crate) missing: BTreeMap<(CanonicalId, u32), BTreeSet<CanonicalId>>,
    /// The highest published version per pack, floored on every parsed key.
    pub(crate) watermark: BTreeMap<CanonicalId, u32>,
    /// **16b cut K.1** — `tacsim:arena:{entry}`: what each arena's applier made,
    /// slot id to object.
    ///
    /// Persisted, unlike `loaded` and `definitions` beside it, because it is
    /// **not** a function of the pack files: every tank is authored inside one
    /// arena entry, so the content latch — one entry, one object — remembers only
    /// the muster room, and which object is slot `red_3` is a fact about *this*
    /// world that only the apply that made it knows.
    pub(crate) arena_rosters: BTreeMap<CanonicalId, crate::match_::ArenaRoster>,
    /// **16b cut C.2** — the decoded, checked and bound game definitions
    /// ([`crate::definitions::Catalog`]), re-derived from `loaded` whenever the
    /// packs are installed.
    ///
    /// It lives here rather than in a slot of its own because the arena pack's
    /// applier runs under the **content** module's `Ctx` and reaches this crate
    /// only by type; one slot the whole layer already owns is one place for it
    /// to look. Like `loaded` it is **not persisted**: it is a pure function of
    /// the pack files, and a world remembers a pack by its ledger row.
    pub(crate) definitions: std::sync::Arc<crate::definitions::Catalog>,
    /// **16b cut AH.1** — `tacsim:tutorial:{account}`: each account's first-time
    /// hints, whether they are switched off and which points it has read
    /// ([`crate::match_::tutorial`]). Keyed by the immutable account id, never by a
    /// character: an alt is the same person.
    pub(crate) tutorials: BTreeMap<mush_core::AccountId, crate::match_::tutorial::TutorialRec>,
    /// **17b cut M3.3** — `tacsim:mindsrc:{sha256}`: every sealed mind source
    /// this world holds, by digest, each one checked against its key and its
    /// bytes at boot (doc 17 §13: *sources live immutably*). A seal names its
    /// source by digest alone ([`crate::arena::mind::MindSeal::source_sha256`]).
    pub(crate) mind_sources: BTreeMap<[u8; 32], String>,
    /// **Every digest a `tacsim:mindsrc:` key claimed**, readable or not (the R8
    /// rule this loader applies to packs and seeds): [`put_mind_source`] never
    /// writes under one of them, so an immutable document is never rewritten.
    pub(crate) mind_source_keys: BTreeSet<[u8; 32]>,
    /// **Doc 18 cut IC3** — `tacsim:port:{pack}/{entry}`: what each port
    /// entry's applier made ([`crate::port::PortMap`]), keyed `(pack, entry)`.
    /// Persisted, like the arena rosters, because which org and which
    /// obligations a port made is a fact about *this* world that only the apply
    /// that made them knows — and it is the sole dedup (the economy does not
    /// dedup obligations).
    pub(crate) port_maps: BTreeMap<(CanonicalId, CanonicalId), crate::port::PortMap>,
    /// **Every `(pack, entry)` a `tacsim:port:` key claimed**, readable or not
    /// (the R8 rule): a claimed key with no readable body is
    /// [`crate::port::PortMapState::Unreadable`] and is never re-created.
    pub(crate) port_map_keys: BTreeSet<(CanonicalId, CanonicalId)>,
    /// **Doc 18 cut IC3 (§4.7; lead ruling L15)** — whether this world is an
    /// in-process test engine's (`composition::tacsim_test_world_module`). RAM
    /// only: never persisted and never a config key, so a production world can
    /// never be one. It relaxes the readiness report's world-rate line and
    /// registers the port tick's probe (doc 18 cut IC5), and nothing else.
    pub(crate) test_world: bool,
    /// **Doc 18 cut IC5** (§4.4 "Failure handling"; lead ruling L10) —
    /// `tacsim:notice:{obj}`: each player's queued reconnect notices
    /// ([`crate::port_tick::NoticeQueue`]), rendered once on `Connected` and then
    /// deleted. Persisted on the IC3 pattern: one whole-document post-image per
    /// character through [`push_op`]; a corrupt document is dropped loudly for that
    /// character and bars nothing.
    pub(crate) notices: BTreeMap<ObjId, crate::port_tick::NoticeQueue>,
    /// **Doc 18 cut IC6** (§4.1, D-18-25; lead ruling L5) —
    /// `tacsim:coop:{account}`: each gateway account's enrolment in the First
    /// Captain programme ([`crate::port::coop::Enrolment`]), keyed by the
    /// immutable account id, never by a character: one enrolment per account,
    /// ever. The account ledger and the frozen terms — the operate gate never
    /// reads it (it reads the live lease, D-18-23).
    pub(crate) enrolments: BTreeMap<mush_core::AccountId, crate::port::coop::Enrolment>,
    /// **Every `tacsim:coop:` key seen at load or written**, readable or not
    /// (the R8 rule): a claimed key with no readable body is
    /// [`crate::port::coop::EnrolmentState::Unreadable`] and still bars that
    /// account's re-enrolment.
    pub(crate) enrolment_keys: BTreeSet<SmolStr>,
    /// **Doc 18 cut IC6** — the compiled-in goods table, parsed once by the
    /// layer (`TacsimRuleset`) and handed to the slot at `register` and again
    /// at `on_world_loaded`. RAM only, never persisted; `None` only on a build
    /// fault, and every gauge then reads dark.
    pub(crate) goods: Option<std::sync::Arc<crate::definitions::GoodsTable>>,
    /// **Doc 18 cut IC9** (§4.4's lifecycle "departing", lead ruling L4) —
    /// `tacsim:voyage:{hull}`: each programme hull's filed sailing plan
    /// ([`crate::port::voyage::Voyage`]): the pending passage `+ship/undock`
    /// filed, and the per-hull burn sequence it mints. Persisted on the
    /// enrolment pattern: one whole-document post-image per hull through
    /// [`put_voyage`].
    pub(crate) voyages: BTreeMap<ObjId, crate::port::voyage::Voyage>,
    /// **Every `tacsim:voyage:` key seen at load or written**, readable or not
    /// (the R8 rule): a claimed key with no readable body is
    /// [`crate::port::voyage::VoyageState::Unreadable`], which refuses that
    /// hull's undock and is skipped by the launch arm (fail closed).
    pub(crate) voyage_keys: BTreeSet<SmolStr>,
    /// **Doc 18 cut IC11** (§4.4 "The queued-orders model", D-18-31) —
    /// `tacsim:orders:{hull}`: each hull's queued sailing plan
    /// ([`crate::port::orders::Orders`]): the orders, their log, the hold and the
    /// step in flight. Persisted on the voyage pattern: one whole-document post-image
    /// per hull through [`put_orders`], which checks it first.
    pub(crate) orders: BTreeMap<ObjId, crate::port::orders::Orders>,
    /// **Every `tacsim:orders:` key seen at load or written**, readable or not (the
    /// R8 rule): a claimed key with no readable body is
    /// [`crate::port::orders::OrdersState::Unreadable`], which refuses that hull's
    /// `+ship/orders` and `+ship/order` and is skipped (and paged) by the order arm.
    pub(crate) orders_keys: BTreeSet<SmolStr>,
    /// **Doc 20 cut SM.1** (Q-20-12) — `tacsim:geology`: the geology key's
    /// pinned fingerprint ([`crate::surface::key`]). Absent until the first
    /// keyed boot pins it; an unreadable body is still a claimed key (the R8
    /// rule): the boot never rewrites it and the layer reads disabled.
    pub(crate) geology_pin: crate::surface::key::GeologyPinState,
    /// **Doc 20 cut SM.3a** (§4.3, Q-20-6) — `tacsim:tvlease:{lease}`: each rig
    /// lease's record ([`crate::port::tv::TvLease`]), keyed by the lease contract.
    /// Persisted on the enrolment pattern: one whole-document post-image per lease
    /// through [`put_tv_lease`]; an unreadable one bars only its own lease.
    pub(crate) tv_leases: BTreeMap<u64, crate::port::tv::TvLeaseState>,
    /// **Doc 20 cut SM.3b** (§8; lead ruling 1) — `tacsim:tv:{tv}`: each rig's
    /// normal-lane projection ([`crate::surface::excursion::TvRec`]), keyed by the
    /// terrain vehicle's id. Written only by the `tacsim.surface` tick through
    /// [`put_tv_rec`]; an unreadable body is [`crate::surface::excursion::TvRecState::Unreadable`]
    /// and bars nothing (the projection decides nothing; the tick's next write replaces it).
    pub(crate) tv_recs: BTreeMap<u32, crate::surface::excursion::TvRecState>,
    /// **Doc 20 cut SM.7** (§6) — `tacsim:mined:{site}`: each site's ring of the last 64 mined
    /// cycles ([`crate::surface::mined::MinedRing`]), keyed by the site entry. Written only by the
    /// `tacsim.surface` tick's tell through [`put_mined`]; an unreadable body is
    /// [`crate::surface::mined::MinedState::Unreadable`] and bars nothing.
    pub(crate) mined: BTreeMap<CanonicalId, crate::surface::mined::MinedState>,
    /// **Doc 20 cut SM.10** (§9) — `tacsim:claimlog:{site}`: each site's claim log
    /// ([`crate::surface::claims::ClaimLog`]), keyed by the site entry. Written only by the site
    /// clerk's claim steps through [`put_claim_log`]; an unreadable body is
    /// [`crate::surface::claims::ClaimLogState::Unreadable`] and bars nothing.
    pub(crate) claim_logs: BTreeMap<CanonicalId, crate::surface::claims::ClaimLogState>,
    /// **Doc 20 cut SM.4a** (D10) — `tacsim:tvpref:{character}`: each character's `+tv`
    /// preferences ([`crate::port::tv::TvPrefs`]), keyed by the character's id. Written only by
    /// `+tv/quiet` through [`put_tv_prefs`]; an unreadable body is logged and read as not quiet
    /// (it bars nothing, and the next `+tv/quiet` replaces it).
    pub(crate) tv_prefs: BTreeMap<u32, crate::port::tv::TvPrefs>,
    /// **Doc 20 cut SM.5** (§9.1, D-20-12) — `tacsim:licence:{account}`: each
    /// gateway account's prospecting licence ([`crate::port::licence::Licence`]),
    /// keyed by the immutable account id (one licence an account, the enrolment
    /// pattern). Persisted through [`put_licence`]; read at every licence-gated act,
    /// never written by a tick.
    pub(crate) licences: BTreeMap<mush_core::AccountId, crate::port::licence::Licence>,
    /// **Every `tacsim:licence:` key seen at load or written**, readable or not (the
    /// R8 rule): a claimed key with no readable body is
    /// [`crate::port::licence::LicenceState::Unreadable`] and bars that account.
    pub(crate) licence_keys: BTreeSet<SmolStr>,
    /// **Doc 23 cut S.1** (§7; the lead's L2) — `log:hull.<n>`: every hull's black box, one
    /// `mush_core::Ledger` log per hull ([`crate::surface::black_box::log_id`]), its open
    /// tails restored from history at boot ([`crate::surface::black_box::restore`]). Appended
    /// only through the black box's owner participant
    /// ([`crate::surface::black_box::BlackBoxParticipant`]), inside the authoritative act
    /// whose record move it records — the launch's finish, the recover's finish, a staff
    /// recovery — and installed when that act's composed commit installs.
    pub(crate) black_boxes: mush_core::Ledger,
    /// **Whether the boot could not restore the black boxes** (doc 23 cut S.1): a refused
    /// restore pages and leaves every black box dark — no append, so no chunk the corrupt
    /// history holds is ever overwritten by a fresh chunk 0 (a corrupt stored row fails closed,
    /// never re-creates). Staff repair the `log:hull.*` documents and restart.
    pub(crate) black_box_dark: bool,
}

impl TacsimStore {
    /// The frozen `[tacsim]` block.
    #[must_use]
    pub fn config(&self) -> &TacsimConfig {
        &self.cfg
    }

    /// **A hull's black box, its open tail** (doc 23 cut S.1, §7; the read seam an inspection
    /// uses): the entries of `log:hull.<n>`'s open (unsealed) chunk, in order — sealed history
    /// is on disk, read through [`crate::surface::black_box::history`] over the chunk
    /// documents. Empty for a hull that never sealed, or whose open chunk just rolled over.
    #[must_use]
    pub fn black_box(&self, hull: ObjId) -> &[mush_core::LedgerEntry] {
        self.black_boxes
            .open_entries(crate::surface::black_box::log_id(hull).as_str())
    }

    /// **A hull's black box, its open tail decoded** (doc 23 cut S.1): [`TacsimStore::black_box`]'s
    /// entries as `(seq, line)`, and how many entries did not decode as a
    /// [`crate::surface::black_box::BlackBoxLine`] (the S.1 fix round's F4: never dropped
    /// silently; the participant writes nothing else, so a non-zero count is a corrupt chunk).
    #[must_use]
    pub fn black_box_lines(
        &self,
        hull: ObjId,
    ) -> (Vec<(u64, crate::surface::black_box::BlackBoxLine)>, usize) {
        let mut lines = Vec::new();
        let mut skipped = 0_usize;
        for entry in self.black_box(hull) {
            match serde_json::from_value(entry.value.clone()) {
                Ok(line) => lines.push((entry.seq, line)),
                Err(_) => skipped = skipped.saturating_add(1),
            }
        }
        (lines, skipped)
    }

    /// **Lines ever written to a hull's black box** (doc 23 cut S.1): the log's next sequence
    /// number — what the line cap (`[tacsim.surface].black_box_max_lines`) is held against.
    #[must_use]
    pub fn black_box_len(&self, hull: ObjId) -> u64 {
        self.black_boxes
            .next_seq(crate::surface::black_box::log_id(hull).as_str())
    }

    /// Whether the boot left every black box dark (a refused restore; doc 23 cut S.1).
    #[must_use]
    pub fn black_box_dark(&self) -> bool {
        self.black_box_dark
    }

    /// **Test hook** (17b cut M1b.5): replace the `[tacsim.minds]` block this
    /// world reads, as a config reload between a check-out and a later verb
    /// would — so a test can prove a running match is held to the floor it was
    /// checked out with (`MatchMirror::odds_floor_permille`) and never to this
    /// block. Nothing in the shipped lane calls it.
    #[doc(hidden)]
    pub fn replace_minds_config_for_tests(&mut self, minds: crate::config::MindsConfig) {
        self.cfg.minds = minds;
    }

    /// **Test hook** (doc 20 cut SM.6, review F3): drop one org's row from every RAM port
    /// map of the lane — a port map that lost its issuer — so a test can prove a site whose
    /// issuer does not resolve opens nothing. Persists nothing; the count of rows dropped.
    /// Nothing in the shipped lane calls it.
    #[doc(hidden)]
    pub fn drop_lane_org_for_tests(&mut self, org: &CanonicalId) -> usize {
        let mut dropped = 0_usize;
        for map in self.port_maps.values_mut() {
            if map.pack.as_str() == crate::definitions::content::LANE_PACK
                && map.orgs.remove(org).is_some()
            {
                dropped = dropped.saturating_add(1);
            }
        }
        dropped
    }

    /// Whether the layer is enabled at all.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.cfg.enabled
    }

    /// **Whether this world is a test world** (doc 18 cut IC3; lead ruling L15):
    /// set only by `composition::tacsim_test_world_module` (and, since doc 18 cut
    /// IC5, `composition::tacsim_test_world_module_wired`), so false on every
    /// production roster. It relaxes the readiness report's world-rate line and
    /// registers the port tick's probe (doc 18 cut IC5), and nothing else.
    #[must_use]
    pub fn test_world(&self) -> bool {
        self.test_world
    }

    /// **One port entry's map** (doc 18 cut IC3): absent, readable, or a key
    /// whose body did not read — the last is never re-created.
    #[must_use]
    pub fn port_map(
        &self,
        pack: &CanonicalId,
        entry: &CanonicalId,
    ) -> crate::port::PortMapState<'_> {
        let key = (pack.clone(), entry.clone());
        match self.port_maps.get(&key) {
            Some(map) => crate::port::PortMapState::Present(map),
            None if self.port_map_keys.contains(&key) => crate::port::PortMapState::Unreadable,
            None => crate::port::PortMapState::Absent,
        }
    }

    /// Every readable port map, in `(pack, entry)` order.
    pub fn port_maps(&self) -> impl Iterator<Item = &crate::port::PortMap> {
        self.port_maps.values()
    }

    /// **One gateway account's enrolment** (doc 18 cut IC6; lead ruling L5):
    /// absent, readable, or a key whose body did not read — the last still bars
    /// that account's `+coop/enrol`.
    #[must_use]
    pub fn enrolment_state(
        &self,
        account: mush_core::AccountId,
    ) -> crate::port::coop::EnrolmentState<'_> {
        match self.enrolments.get(&account) {
            Some(e) => crate::port::coop::EnrolmentState::Present(e),
            None if self
                .enrolment_keys
                .contains(&crate::port::coop::enrolment_key(account)) =>
            {
                crate::port::coop::EnrolmentState::Unreadable
            }
            None => crate::port::coop::EnrolmentState::Absent,
        }
    }

    /// The readable enrolment whose `pilot` is `pilot` (a scan, bounded by the
    /// enrolments).
    #[must_use]
    pub fn enrolment_of(&self, pilot: ObjId) -> Option<&crate::port::coop::Enrolment> {
        self.enrolments.values().find(|e| e.pilot == pilot)
    }

    /// The readable enrolment whose lease is `lease` (`ContractId.0`).
    #[must_use]
    pub fn enrolment_for_lease(&self, lease: u64) -> Option<&crate::port::coop::Enrolment> {
        self.enrolments.values().find(|e| e.lease == lease)
    }

    /// The readable enrolment whose buyout contract is `contract` (`ContractId.0`;
    /// doc 18 cut IC13).
    #[must_use]
    pub fn enrolment_for_buyout(&self, contract: u64) -> Option<&crate::port::coop::Enrolment> {
        self.enrolments
            .values()
            .find(|e| e.buyout == Some(contract))
    }

    /// **The enrolment governing a hull** (doc 18 cut IC13): the one standing
    /// `Active`, `BuyoutPending` or `Graduated` for that hull, else the `Revoked`
    /// one with the greatest `ended_at`. One hull carries at most one non-Revoked
    /// enrolment and any number of Revoked ones (one per past tenant).
    #[must_use]
    pub fn enrolment_for_hull(&self, hull: ObjId) -> Option<&crate::port::coop::Enrolment> {
        use crate::port::coop::EnrolmentStatus;
        let mut on_hull = self.enrolments.values().filter(|e| e.hull == hull);
        let mut latest_revoked: Option<&crate::port::coop::Enrolment> = None;
        for e in on_hull.by_ref() {
            match e.status {
                EnrolmentStatus::Active
                | EnrolmentStatus::BuyoutPending
                | EnrolmentStatus::Graduated => return Some(e),
                EnrolmentStatus::Revoked => {
                    if latest_revoked.is_none_or(|best| e.ended_at > best.ended_at) {
                        latest_revoked = Some(e);
                    }
                }
            }
        }
        latest_revoked
    }

    /// Every readable enrolment, in account order.
    pub fn enrolments(&self) -> impl Iterator<Item = &crate::port::coop::Enrolment> {
        self.enrolments.values()
    }

    /// **One rig lease's record** (doc 20 cut SM.3a), as the store holds it;
    /// `None` when no key claims the lease.
    #[must_use]
    pub fn tv_lease(&self, lease: u64) -> Option<&crate::port::tv::TvLeaseState> {
        self.tv_leases.get(&lease)
    }

    /// **Every readable rig lease record of `pilot`** (doc 20 cut SM.3a), in lease
    /// order, any status.
    pub fn tv_leases_of(&self, pilot: ObjId) -> impl Iterator<Item = &crate::port::tv::TvLease> {
        self.tv_leases
            .values()
            .filter_map(move |state| match state {
                crate::port::tv::TvLeaseState::Present(record) if record.pilot == pilot => {
                    Some(record)
                }
                crate::port::tv::TvLeaseState::Present(_)
                | crate::port::tv::TvLeaseState::Unreadable { .. } => None,
            })
    }

    /// **The live rig lease record on `tv`** (doc 20 cut SM.3a; reviewer F2): the
    /// readable record naming the TV whose status is not `Returned` (Active or
    /// Forfeit). At most one by construction: a take refuses a rig with one.
    #[must_use]
    pub fn tv_lease_on(&self, tv: ObjId) -> Option<&crate::port::tv::TvLease> {
        self.tv_leases.values().find_map(|state| match state {
            crate::port::tv::TvLeaseState::Present(record) if record.tv == tv && record.live() => {
                Some(record)
            }
            crate::port::tv::TvLeaseState::Present(_)
            | crate::port::tv::TvLeaseState::Unreadable { .. } => None,
        })
    }

    /// **Whether an unreadable rig lease record names `who`** (doc 20 cut SM.3a)
    /// as its pilot, its hull or its TV, so far as its body could be read.
    #[must_use]
    pub fn tv_lease_unreadable_for(&self, who: ObjId) -> bool {
        self.tv_leases.values().any(|state| match state {
            crate::port::tv::TvLeaseState::Unreadable { pilot, hull, tv } => {
                [pilot, hull, tv].iter().any(|obj| **obj == Some(who))
            }
            crate::port::tv::TvLeaseState::Present(_) => false,
        })
    }

    /// **One gateway account's prospecting licence** (doc 20 cut SM.5): absent,
    /// readable, or a key whose body did not read — the last bars that account's
    /// `+licence/buy` and every licence-gated act.
    #[must_use]
    pub fn licence_state(
        &self,
        account: mush_core::AccountId,
    ) -> crate::port::licence::LicenceState<'_> {
        match self.licences.get(&account) {
            Some(record) => crate::port::licence::LicenceState::Present(record),
            None if self
                .licence_keys
                .contains(&crate::port::licence::licence_key(account)) =>
            {
                crate::port::licence::LicenceState::Unreadable
            }
            None => crate::port::licence::LicenceState::Absent,
        }
    }

    /// The readable licence whose `holder` is `pilot` (a scan, bounded by the
    /// licences): the fallback when no account oracle answers for the pilot.
    #[must_use]
    pub fn licence_of_holder(&self, pilot: ObjId) -> Option<&crate::port::licence::Licence> {
        self.licences.values().find(|record| record.holder == pilot)
    }

    /// **One rig's projection** (doc 20 cut SM.3b; §8): `tacsim:tv:{tv}` as the store
    /// holds it; `None` when no key claims it. It decides nothing.
    #[must_use]
    pub fn tv_rec(&self, tv: ObjId) -> Option<&crate::surface::excursion::TvRecState> {
        self.tv_recs.get(&tv.0)
    }

    /// **One site's mined ring** (doc 20 cut SM.7; §6): `tacsim:mined:{site}` as the store holds
    /// it; `None` when no key claims it. Read by nothing in the game yet.
    #[must_use]
    pub fn mined(&self, site: &CanonicalId) -> Option<&crate::surface::mined::MinedState> {
        self.mined.get(site)
    }

    /// **One site's claim log** (doc 20 cut SM.10): `tacsim:claimlog:{site}` as the store holds
    /// it; `None` when no key claims it.
    #[must_use]
    pub fn claim_log(&self, site: &CanonicalId) -> Option<&crate::surface::claims::ClaimLogState> {
        self.claim_logs.get(site)
    }

    /// **Whether `who` has asked not to hear other rigs' soundings** (doc 20 cut SM.4a; D10):
    /// their `tacsim:tvpref:` document's `quiet`; absent or unreadable reads `false` (the
    /// unreadable one was logged at load).
    #[must_use]
    pub fn tv_quiet(&self, who: ObjId) -> bool {
        self.tv_prefs.get(&who.0).is_some_and(|prefs| prefs.quiet)
    }

    /// **One hull's sailing plan** (doc 18 cut IC9): absent, readable, or a key
    /// whose body did not read — the last refuses that hull's undock.
    #[must_use]
    pub fn voyage_state(&self, hull: ObjId) -> crate::port::voyage::VoyageState<'_> {
        match self.voyages.get(&hull) {
            Some(voyage) => crate::port::voyage::VoyageState::Present(voyage),
            None if self
                .voyage_keys
                .contains(&crate::port::voyage::voyage_key(hull)) =>
            {
                crate::port::voyage::VoyageState::Unreadable
            }
            None => crate::port::voyage::VoyageState::Absent,
        }
    }

    /// Every readable sailing plan, in hull order.
    pub fn voyages(&self) -> impl Iterator<Item = &crate::port::voyage::Voyage> {
        self.voyages.values()
    }

    /// **One hull's queued sailing plan** (doc 18 cut IC11): absent, readable, or a
    /// key whose body did not read — the last refuses that hull's plan verbs.
    #[must_use]
    pub fn orders_state(&self, hull: ObjId) -> crate::port::orders::OrdersState<'_> {
        match self.orders.get(&hull) {
            Some(orders) => crate::port::orders::OrdersState::Present(orders),
            None if self
                .orders_keys
                .contains(&crate::port::orders::orders_key(hull)) =>
            {
                crate::port::orders::OrdersState::Unreadable
            }
            None => crate::port::orders::OrdersState::Absent,
        }
    }

    /// Every readable queued sailing plan, in hull order.
    pub fn orders_docs(&self) -> impl Iterator<Item = &crate::port::orders::Orders> {
        self.orders.values()
    }

    /// **The hulls whose sailing plan's key is claimed but whose body did not read**
    /// (doc 18 cut IC14 row 11; the [`Self::orders_unreadable`] precedent): the port
    /// tick pages each once a boot.
    pub fn voyages_unreadable(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.voyage_keys
            .iter()
            .filter_map(|key| crate::port::voyage::voyage_key_owner(key))
            .filter(|hull| !self.voyages.contains_key(hull))
    }

    /// Every hull whose queued sailing plan's key is claimed with no readable body,
    /// in key order (the order arm pages each once a boot).
    pub fn orders_unreadable(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.orders_keys
            .iter()
            .filter_map(|key| crate::port::orders::orders_key_owner(key))
            .filter(|hull| !self.orders.contains_key(hull))
    }

    /// **The goods table** (doc 18 cut IC6): the layer's parse, the one read the
    /// gauges and `+ship` make. `None` only on a build fault.
    #[must_use]
    pub fn goods(&self) -> Option<&crate::definitions::GoodsTable> {
        self.goods.as_deref()
    }

    /// Every `(pack, entry)` a port map key claims, readable or not.
    pub(crate) fn port_map_claims(&self) -> impl Iterator<Item = &(CanonicalId, CanonicalId)> {
        self.port_map_keys.iter()
    }

    /// **The sealed mind source under `digest`** (17b cut M3.3; doc 17 §13),
    /// or `None` when this world holds no readable document for it.
    #[must_use]
    pub fn mind_source(&self, digest: &[u8; 32]) -> Option<&str> {
        self.mind_sources.get(digest).map(String::as_str)
    }

    /// **Whether this world holds the sealed source under `digest`**, readable
    /// and checked — what a resume asks of a scripted seal.
    #[must_use]
    pub fn has_mind_source(&self, digest: &[u8; 32]) -> bool {
        self.mind_sources.contains_key(digest)
    }

    /// Every loaded pack, in `(id, version)` order.
    pub fn packs(&self) -> impl Iterator<Item = &LoadedPack> {
        self.loaded.values()
    }

    /// One loaded pack.
    #[must_use]
    pub fn pack(&self, id: &CanonicalId, version: u32) -> Option<&LoadedPack> {
        self.loaded.get(&(id.clone(), version))
    }

    /// Every loaded version of one pack, ascending.
    pub fn versions_of(&self, id: &CanonicalId) -> impl Iterator<Item = u32> + '_ {
        let id = id.clone();
        self.loaded
            .keys()
            .filter(move |(pack, _)| *pack == id)
            .map(|(_, version)| *version)
    }

    /// The highest loaded version of one pack.
    #[must_use]
    pub fn newest_loaded(&self, id: &CanonicalId) -> Option<u32> {
        self.versions_of(id).max()
    }

    /// The published-version ledger.
    pub fn published(&self) -> impl Iterator<Item = &PackRecord> {
        self.published.values()
    }

    /// **The digest this world published one version under**, if it can still read the
    /// row.
    ///
    /// The one read of `PackRecord::digest` that makes the ledger's digest field mean
    /// something (independent verify P1): `boot_reconcile` compares it against the
    /// loaded pack and refuses the boot on a disagreement, and `@tacsim/pack verify`
    /// reports the same fact rather than re-proving an equality `load_pack` already
    /// established.
    #[must_use]
    pub fn published_digest(
        &self,
        id: &CanonicalId,
        version: u32,
    ) -> Option<mush_core::DefinitionDigest> {
        self.published
            .get(&(id.clone(), version))
            .map(PackRecord::digest)
    }

    /// Resolve one recipe id in a published pack version to its **pack digest**
    /// and its authored definition (E7 cut 4's `RecipeAuthority` seam, read at the
    /// composition root). `None` if the pack version is not **published** (a
    /// retired version fails closed here) or the id is not a recipe — no second
    /// digest, no second registry (§9 q3): the pack's own published digest is the
    /// only one.
    #[must_use]
    pub fn recipe(
        &self,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<(mush_core::DefinitionDigest, crate::pack::RecipeDef)> {
        let digest = self.published_digest(pack, version)?;
        match self.pack(pack, version)?.pack.entry(id) {
            Some(crate::pack::PackEntry::Recipe(def)) => Some((digest, def.clone())),
            _ => None,
        }
    }

    /// **Every formation kind this world's PUBLISHED packs author**, in
    /// `(pack, version, id)` order (E7 cut 9's `FormationAuthority` seam, read at
    /// the composition root).
    ///
    /// A retired version contributes nothing — `published_digest` answers `None`
    /// for one — which is the same fail-closed rule [`TacsimStore::recipe`] uses.
    /// The walk is bounded by the pack cap on both axes.
    #[must_use]
    pub fn formation_kinds(&self) -> Vec<(CanonicalId, crate::pack::FormationKindDef)> {
        let mut out = Vec::new();
        for (pack, version) in self.published.keys() {
            if self.published_digest(pack, *version).is_none() {
                continue;
            }
            let Some(loaded) = self.pack(pack, *version) else {
                continue;
            };
            for (id, entry) in loaded.pack.entries() {
                if let crate::pack::PackEntry::Formation(def) = entry {
                    out.push((id.clone(), def.clone()));
                }
            }
        }
        out
    }

    /// The seed latch for one `(pack, version)`.
    #[must_use]
    pub fn seed(&self, pack: &CanonicalId, version: u32) -> Option<&SeedApplication> {
        self.seeded.get(&(pack.clone(), version))
    }

    /// **Whether this `(pack, version)` has already been applied.**
    ///
    /// Derived from the *keys*, so a latch whose body will not parse still bars a
    /// second `commit`. That is the only fail-closed direction: reading "cannot parse"
    /// as "not applied" would duplicate a mine.
    #[must_use]
    pub fn is_applied(&self, pack: &CanonicalId, version: u32) -> bool {
        self.seeded_keys.contains(&(pack.clone(), version))
    }

    /// The version of `pack` this world was seeded at, if any.
    #[must_use]
    pub fn applied_version(&self, pack: &CanonicalId) -> Option<u32> {
        self.applied.get(pack).copied()
    }

    /// One subject's pin.
    #[must_use]
    pub fn pin(&self, subject: &CanonicalId) -> Option<&PinRecord> {
        self.pins.get(subject)
    }

    /// Every pin, in subject order.
    pub fn pins(&self) -> impl Iterator<Item = &PinRecord> {
        self.pins.values()
    }

    /// **One materialized route, for the pack version that governs its id.**
    ///
    /// A route id is pack-local, so answering by id alone needs a rule for which pack's
    /// route that is: [`TacsimStore::resolve`]'s, the same rule every other subject
    /// goes through. Ask for a specific pack's route with [`TacsimStore::route_in`].
    #[must_use]
    pub fn route(&self, id: &CanonicalId) -> Option<&RouteDef> {
        let (pack, version) = self.resolve(id).ok()?;
        self.routes.get(&(pack, version, id.clone()))
    }

    /// One materialized route, named by the pack and version that declares it.
    #[must_use]
    pub fn route_in(
        &self,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<&RouteDef> {
        self.routes.get(&(pack.clone(), version, id.clone()))
    }

    /// Every materialized route, in `(pack, version, id)` order.
    pub fn routes(&self) -> impl Iterator<Item = (&CanonicalId, u32, &RouteDef)> {
        self.routes
            .iter()
            .map(|((pack, version, _), def)| (pack, *version, def))
    }

    /// Every seed latch, keyed `(pack, version)`.
    pub(crate) fn latches(&self) -> impl Iterator<Item = (&(CanonicalId, u32), &SeedApplication)> {
        self.seeded.iter()
    }

    /// Every object this world's seed latches created **and still has**, pack-local id
    /// to object.
    ///
    /// A placement whose object is gone is omitted rather than handed out (S3); it is
    /// reported by [`TacsimStore::missing_placements`], which boot has already paged.
    pub fn seeded_objects(&self) -> impl Iterator<Item = (&CanonicalId, ObjId)> {
        self.seeded.iter().flat_map(|(key, seed)| {
            let gone = self.missing.get(key);
            seed.placed()
                .iter()
                .filter(move |(id, _)| !gone.is_some_and(|set| set.contains(*id)))
                .map(|(id, obj)| (id, *obj))
        })
    }

    /// S9.1: the objects a pack's symbols name in this world — the content latch
    /// (the S9 seed: one row per pack id, its `placed` map) when it applied this
    /// version, else the legacy `tacsim:seed:` latch for a world seeded before.
    #[must_use]
    pub fn placed_in(
        &self,
        w: &World,
        pack: &CanonicalId,
        version: u32,
    ) -> Option<BTreeMap<CanonicalId, ObjId>> {
        if let Some(applied) = w
            .module_state::<mush_content::ContentState>()
            .and_then(|c| c.latches.applied(pack))
            .filter(|a| a.version() == version && !a.is_retired())
        {
            return Some(applied.placed());
        }
        self.seeded
            .get(&(pack.clone(), version))
            .map(|seed| seed.placed().clone())
    }

    /// S9.1: the version of `pack` this world has applied, through either latch.
    #[must_use]
    pub fn applied_in(&self, w: &World, pack: &CanonicalId) -> Option<u32> {
        w.module_state::<mush_content::ContentState>()
            .and_then(|c| c.latches.applied(pack))
            .filter(|a| !a.is_retired())
            .map(|a| a.version())
            .or_else(|| self.applied_version(pack))
    }

    /// S9.1: a route by id, materialized against whichever latch placed its
    /// pack's places in this world. The legacy index answers first; a pack the
    /// content latch applied is materialized here, the newest applied version.
    #[must_use]
    pub fn route_in_world(&self, w: &World, id: &CanonicalId) -> Option<RouteDef> {
        if let Some(def) = self.route(id) {
            return Some(def.clone());
        }
        self.latched_route(w, id).map(|(def, _, _)| def)
    }

    /// **A route by id at one pack version, from the content latch** (doc 18 cut
    /// IC14 row 47): the latched route only when this world applied exactly
    /// `version` of a pack defining it, never the newest-applied version
    /// [`TacsimStore::route_in_world`] falls back to. `@corridor/show <id>@<version>`
    /// reads it.
    #[must_use]
    pub fn route_in_world_at(&self, w: &World, id: &CanonicalId, version: u32) -> Option<RouteDef> {
        self.latched_route_at(w, id, Some(version))
            .map(|(def, _, _)| def)
    }

    /// The content-latch half of [`TacsimStore::route_in_world`]: the newest
    /// loaded version this world applied whose places the latch placed and whose
    /// route materializes, with the pack and the symbolic entry it came from. One
    /// candidate rule, so [`TacsimStore::route_source`] can never name a pack
    /// other than the one the route was materialized from (doc 18 cut IC2).
    fn latched_route(
        &self,
        w: &World,
        id: &CanonicalId,
    ) -> Option<(RouteDef, &RulesPack, &RouteEntry)> {
        self.latched_route_at(w, id, None)
    }

    /// [`TacsimStore::latched_route`] over the candidates of one pack `version`
    /// only, when one is named (doc 18 cut IC14 row 47).
    fn latched_route_at(
        &self,
        w: &World,
        id: &CanonicalId,
        version: Option<u32>,
    ) -> Option<(RouteDef, &RulesPack, &RouteEntry)> {
        let mut candidates: Vec<(&(CanonicalId, u32), &LoadedPack)> = self
            .loaded
            .iter()
            .filter(|((_, v), _)| version.is_none_or(|want| *v == want))
            .filter(|(_, loaded)| matches!(loaded.pack.entry(id), Some(PackEntry::Route(_))))
            .collect();
        candidates.sort_by_key(|(key, _)| std::cmp::Reverse(key.1));
        for ((pack, version), loaded) in candidates {
            if self.applied_in(w, pack) != Some(*version) {
                continue;
            }
            let Some(placed) = self.placed_in(w, pack, *version) else {
                continue;
            };
            let Some(PackEntry::Route(route)) = loaded.pack.entry(id) else {
                continue;
            };
            // Doc 18 cut IC2 (D-18-43): the pack's own sky, through the one seam.
            let sky = Sky::from_pack(&loaded.pack, &self.cfg.time.ephemeris);
            match materialize_route(id, route, &placed, None, &loaded.pack, &sky) {
                Ok(def) => return Some((def, &loaded.pack, route)),
                Err(detail) => {
                    tracing::warn!(route = %id, pack = %pack, version, %detail, "tacsim: a route could not be materialized against the content latch");
                }
            }
        }
        None
    }

    /// **Where a materialized route came from** (doc 18 cut IC2): the loaded pack
    /// and the symbolic route entry behind `def` — a legacy routes-index entry
    /// that holds exactly `def` (the version `resolve` picks first, then any
    /// other, so `@corridor/show <id> <version>` is answered from its own pack),
    /// else the content-latch candidate [`TacsimStore::route_in_world`] would
    /// materialize, and only when what it materializes is `def` itself. A pack
    /// that cannot have produced `def` is never named, so `@corridor/show` never
    /// annotates one pack's legs with another pack's sky.
    #[must_use]
    pub fn route_source(&self, w: &World, def: &RouteDef) -> Option<(&RulesPack, &RouteEntry)> {
        let id = def.id();
        let entry_of = |pack: &CanonicalId, version: u32| {
            let loaded = self.loaded.get(&(pack.clone(), version))?;
            match loaded.pack.entry(id) {
                Some(PackEntry::Route(route)) => Some((&loaded.pack, route)),
                _ => None,
            }
        };
        if let Ok((pack, version)) = self.resolve(id)
            && self.routes.get(&(pack.clone(), version, id.clone())) == Some(def)
            && let Some(found) = entry_of(&pack, version)
        {
            return Some(found);
        }
        if let Some(found) = self
            .routes
            .iter()
            .rev()
            .filter(|((_, _, route), held)| route == id && *held == def)
            .find_map(|((pack, version, _), _)| entry_of(pack, *version))
        {
            return Some(found);
        }
        self.latched_route(w, id)
            .filter(|(materialized, _, _)| materialized == def)
            .map(|(_, pack, route)| (pack, route))
    }

    /// Per latch, how many of its placed objects this world no longer has (S3).
    ///
    /// Empty on a healthy world. Every entry was paged at boot, level 2, naming the
    /// pack and the command that shows it.
    pub fn missing_placements(&self) -> impl Iterator<Item = (&CanonicalId, u32, usize)> {
        self.missing
            .iter()
            .map(|((pack, version), gone)| (pack, *version, gone.len()))
    }

    /// Record the placements boot found the world no longer has, and re-project.
    pub(crate) fn record_missing_placements(
        &mut self,
        missing: Vec<((CanonicalId, u32), CanonicalId)>,
    ) {
        for (key, symbol) in missing {
            self.missing.entry(key).or_default().insert(symbol);
        }
        self.reindex_routes();
    }

    /// **Resolve a subject to the exact pack version that governs it** (§3.1
    /// invariant (i)).
    ///
    /// A pin wins and is exact. Failing that, a pack this world was *seeded* at holds
    /// its subjects at that version. Only a subject with neither reaches "the newest
    /// loaded version", and a retired version on either of the first two paths
    /// **refuses** rather than upgrading.
    ///
    /// # Errors
    ///
    /// [`ResolveError::Unknown`] and [`ResolveError::Retired`].
    pub fn resolve(&self, subject: &CanonicalId) -> Result<(CanonicalId, u32), ResolveError> {
        if let Some(pin) = self.pins.get(subject) {
            let key = (pin.pack().clone(), pin.version());
            if self.loaded.contains_key(&key) {
                return Ok(key);
            }
            return Err(ResolveError::Retired {
                subject: subject.to_string(),
                pack: pin.pack().to_string(),
                version: pin.version(),
                how: "pinned",
            });
        }
        // Which packs could govern THIS subject at all? A subject nothing declares is
        // unknown; the answer must not depend on which version happened to load.
        //
        // Two sources, and both are about the subject (independent verify P2). A
        // loaded pack that declares it is the ordinary one. The second is a pack this
        // world was *seeded* at whose latch actually placed this symbol: that pack
        // governs what it made even when no loaded version declares it any more.
        //
        // What is deliberately NOT a source is `published_keys`. It carries a row for
        // every version any `tacsim:pack:` key ever claimed, so using it here made the
        // retired-seed arm true for any applied-but-unloaded pack **whatever was
        // asked** — dropping one already-applied campaign-seed file from `seed_paths`,
        // an ordinary operator action once the seed has run, made `resolve` refuse for
        // every subject in every still-loaded rules pack and misattribute each to the
        // unrelated seed pack. Invariant (i)'s rule is per-subject; that was a total
        // denial of the store's headline query. It may widen the *evidence* that a
        // candidate was retired; it may never make an unrelated pack a candidate.
        let mut candidates: BTreeSet<CanonicalId> = BTreeSet::new();
        for ((id, _), loaded) in &self.loaded {
            if loaded.pack.entry(subject).is_some() {
                candidates.insert(id.clone());
            }
        }
        for ((pack, version), seed) in &self.seeded {
            if seed.placed().contains_key(subject) && self.applied.get(pack) == Some(version) {
                candidates.insert(pack.clone());
            }
        }
        // A candidate this world was seeded at, whose version is gone, **refuses** —
        // fail closed, naming the pack that actually declares the subject asked for,
        // rather than silently re-pointing it at a newer version.
        for pack in &candidates {
            let Some(version) = self.applied.get(pack) else {
                continue;
            };
            if self.loaded.contains_key(&(pack.clone(), *version)) {
                continue;
            }
            return Err(ResolveError::Retired {
                subject: subject.to_string(),
                pack: pack.to_string(),
                version: *version,
                how: "seeded",
            });
        }
        for pack in &candidates {
            if let Some(version) = self.applied.get(pack) {
                let key = (pack.clone(), *version);
                if self.loaded.contains_key(&key) {
                    return Ok(key);
                }
            }
        }
        let mut best: Option<(CanonicalId, u32)> = None;
        for (id, version) in self.loaded.keys() {
            if !candidates.contains(id) {
                continue;
            }
            let better = best.as_ref().is_none_or(|(_, current)| version > current);
            if better {
                best = Some((id.clone(), *version));
            }
        }
        best.ok_or_else(|| ResolveError::Unknown {
            subject: subject.to_string(),
        })
    }

    /// Install the packs the host read and this module verified.
    pub(crate) fn install_packs(&mut self, packs: Vec<LoadedPack>) {
        self.loaded.clear();
        // 16b cut C.2: bind before installing, so the slot never holds packs
        // whose definitions did not resolve. A refusal here is already a boot
        // refusal (`TacsimRuleset::register` binds the same packs and returns
        // it), so this arm is the honest degradation rather than a second
        // policy: the packs are there and the definitions are empty.
        // **16b cut Q.8**: the FIELDED catalog — the stock content plus doc 16
        // §9's five extension items — which is what `SIM UNIT` lists, what the
        // arena applier binds through and what the stock-tank reset compares
        // against.
        match crate::definitions::Catalog::build_fielded(&packs, crate::NS) {
            Ok(catalog) => self.definitions = std::sync::Arc::new(catalog),
            Err(error) => {
                tracing::error!(%error, "tacsim: the loaded packs' definitions did not bind");
                self.definitions = std::sync::Arc::default();
            }
        }
        for loaded in packs {
            let key = (
                loaded.pack.manifest().id().clone(),
                loaded.pack.manifest().version(),
            );
            self.loaded.insert(key, loaded);
        }
    }

    /// **16b cut C.2** — the bound game definitions.
    #[must_use]
    pub fn definitions(&self) -> &crate::definitions::Catalog {
        &self.definitions
    }

    /// **16b cut K.1** — one arena's roster: the object behind every authored
    /// unit slot.
    #[must_use]
    pub fn arena_roster(&self, arena: &CanonicalId) -> Option<&crate::match_::ArenaRoster> {
        self.arena_rosters.get(arena)
    }

    /// Install one arena's roster in RAM — the applier's half of the write, so
    /// the boot that applies a pack can resolve its slots without a reboot.
    pub(crate) fn put_arena_roster(
        &mut self,
        arena: CanonicalId,
        roster: crate::match_::ArenaRoster,
    ) {
        self.arena_rosters.insert(arena, roster);
    }

    /// The allocator document's body.
    #[must_use]
    pub(crate) fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "schema": SCHEMA_VERSION,
            "watermark": self
                .watermark
                .iter()
                .map(|(id, version)| (id.to_string(), serde_json::json!(*version)))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        })
    }

    /// Re-derive every index from the records. Called on every write and at boot, so
    /// no index is ever authority in its own right.
    pub(crate) fn reindex(&mut self) {
        self.applied.clear();
        for (pack, version) in &self.seeded_keys {
            let entry = self.applied.entry(pack.clone()).or_insert(*version);
            *entry = (*entry).max(*version);
        }
        for (pack, version) in self.published_keys.iter().chain(self.seeded_keys.iter()) {
            let entry = self.watermark.entry(pack.clone()).or_insert(*version);
            *entry = (*entry).max(*version);
        }
        self.reindex_routes();
    }

    /// Re-materialize every route the loaded packs declare and the seed latches
    /// placed. A route whose places are not all placed is **skipped loudly**: it is a
    /// projection, and answering with a half-resolved route would be worse than
    /// answering with none.
    fn reindex_routes(&mut self) {
        self.routes.clear();
        let mut built: Vec<((CanonicalId, u32, CanonicalId), RouteDef)> = Vec::new();
        for ((pack, version), loaded) in &self.loaded {
            let key = (pack.clone(), *version);
            let Some(seed) = self.seeded.get(&key) else {
                continue;
            };
            let gone = self.missing.get(&key);
            // S7 (T17): the pack's own sky, under the campaign's epoch and rate —
            // through the sky seam since doc 18 cut IC2 (D-18-43).
            let sky = Sky::from_pack(&loaded.pack, &self.cfg.time.ephemeris);
            for (id, entry) in loaded.pack.entries() {
                let PackEntry::Route(route) = entry else {
                    continue;
                };
                match materialize_route(id, route, seed.placed(), gone, &loaded.pack, &sky) {
                    // Keyed by the pack that declares it: a pack-local id is not a
                    // global one, and a later-sorting pack must not shadow an earlier
                    // one's route (S4).
                    Ok(def) => built.push(((pack.clone(), *version, id.clone()), def)),
                    Err(detail) => tracing::warn!(
                        route = %id,
                        pack = %pack,
                        version,
                        %detail,
                        "tacsim: a pack route could not be materialized; skipped"
                    ),
                }
            }
        }
        for (key, def) in built {
            self.routes.insert(key, def);
        }
    }
}

/// Resolve one pack-authored route against a seed's placement map.
///
/// `gone` is the set of pack-local symbols this world no longer has an object for
/// (S3): a route through one of them is refused here rather than handed out over a
/// tombstone.
fn materialize_route(
    id: &CanonicalId,
    route: &RouteEntry,
    placed: &BTreeMap<CanonicalId, ObjId>,
    gone: Option<&BTreeSet<CanonicalId>>,
    pack: &RulesPack,
    sky: &Sky,
) -> Result<RouteDef, String> {
    let place = |symbol: &CanonicalId| -> Result<ObjId, String> {
        if gone.is_some_and(|set| set.contains(symbol)) {
            return Err(format!(
                "the object the seed placed for {symbol} is not in this world any more"
            ));
        }
        placed
            .get(symbol)
            .copied()
            .ok_or_else(|| format!("the seed placed no object for {symbol}"))
    };
    let from = place(&route.from)?;
    let to = place(&route.to)?;
    let mut legs = Vec::with_capacity(route.legs.len());
    for leg in &route.legs {
        let leg_to = place(&leg.to)?;
        // E7r1 R2: the pack's authored hazard band and interdiction threshold ride
        // the materialization. `LegDef::with_hazard` re-runs the bound, so a pack
        // past `MAX_HAZARD_BAND` is **refused on load, never clamped**.
        legs.push(
            LegDef::with_hazard(leg_to, leg.base_s, leg.hazard, leg.interdiction)
                .map_err(|e| e.to_string())?,
        );
    }
    let def = RouteDef::controlled(
        id.clone(),
        (from, to),
        legs,
        route.drive_class.clone(),
        route.propellant,
        route.controller.clone(),
    )
    .map_err(|e| e.to_string())?;
    // S7 (T17): a route between two bodies' places carries the distance the sky
    // pinned at the epoch; a route within one body's places carries none.
    //
    // Doc 18 cut IC2 (D-18-43, D-18-45: "the direct chord at the epoch"): the pin
    // asks the sky seam at the origin body's own epoch, both instants equal. A
    // body without `epoch_ic` falls back to the config epoch, so every fixture
    // pins bit-identically to the old `pinned_km`; a lane body pins at its own
    // epoch, the instant `@corridor/show` prints as the date beside it, so the km
    // and the date agree under any sky rate (pinning at the config epoch would,
    // at a non-zero rate, rewind a lane body to IC 0 from the date printed next
    // to the figure).
    let body_of = |symbol: &CanonicalId| match pack.entry(symbol) {
        Some(PackEntry::Location(place)) => place.body.clone(),
        _ => None,
    };
    if let (Some(a), Some(b)) = (body_of(&route.from), body_of(&route.to))
        && a != b
    {
        let e = sky.epoch_of(&a).unwrap_or_else(|| sky.config_epoch());
        if let Some(km) = sky.distance_km(&a, e, &b, e) {
            return Ok(def.pinned(km, (a, b)));
        }
    }
    Ok(def)
}

/// The drive profile a route's declared class is evaluated under.
///
/// Cut 1 authors no propulsion table — that is cut 11's full rules pack — so this is
/// the one nominal profile the fixture's `@tacsim/pack show` and the route tests use.
/// It is a **function of the class name only**, so it stays pure.
///
/// # Propellant is INSIDE `payload` (cut 6 reads this as the ruling)
///
/// Independent verify P7: the nominal capacity was 1 000 while the shipped corridor
/// declares `propellant: 1200`, so evaluating the fixture's one route **with its own
/// declared propellant load** was `RouteDefError::Overloaded` — the two numbers in the
/// same fixture contradicted each other, and nothing noticed because every test
/// evaluated at `payload = 0`. The two numbers are made consistent in the one place a
/// number is authored that does not move a published digest: the capacity, here. A
/// journey's propellant plan counts against the drive's payload capacity, so
/// `route_duration(route, nominal_drive(class), route.propellant())` is the empty-hold
/// case and any cargo is added to it. [`MAX_ROUTE_PROPELLANT`] bounds what a pack may
/// declare; this figure bounds what the one nominal drive will lift.
///
/// [`MAX_ROUTE_PROPELLANT`]: crate::pack::MAX_ROUTE_PROPELLANT
#[must_use]
pub fn nominal_drive(class: &CanonicalId) -> Option<DriveProfile> {
    // S7 (T17): the class's distance law. A torch accelerates the whole way (time
    // ∝ √distance); anything else coasts (time ∝ distance). Cut 11's propulsion
    // table will author this per class; until then the name says it.
    let law = if class.as_str().contains("torch") {
        DistanceLaw::Torch
    } else {
        DistanceLaw::Coast
    };
    DriveProfile::new(class.clone(), 1_000, 4_000)
        .ok()
        .map(|drive| drive.with_law(law))
}

/// Queue one module write.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Install a ledger row into RAM and queue its document — the single write path.
pub(crate) fn put_pack(ctx: &mut Ctx<'_>, record: PackRecord) {
    let key = pack_key(record.id(), record.version());
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        let index = (record.id().clone(), record.version());
        store.published_keys.insert(index.clone());
        store.published.insert(index, record.clone());
        store.reindex();
    }
    let Ok(value) = serde_json::to_value(&record) else {
        tracing::error!(%key, "tacsim: a pack ledger row did not serialize");
        return;
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// **Install one account's tutorial record into RAM and queue its document** — the
/// single write path (16b cut AH.1). A whole-document post-image on the module's own
/// lane, like every document here; there is no `Del` (see [`MAX_PINS`]).
pub(crate) fn put_tutorial(
    ctx: &mut Ctx<'_>,
    account: mush_core::AccountId,
    rec: crate::match_::tutorial::TutorialRec,
) {
    let value = rec.to_document(account);
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.tutorials.insert(account, rec);
    }
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: crate::match_::tutorial::tutorial_key(account),
            value,
        },
    );
}

/// Source-owner bound, separate from the authored-library 32 KiB limit. The
/// shipped open Standard source is larger than an authored library entry.
pub const MAX_STORED_MIND_SOURCE_BYTES: usize = 128 * 1024;
const MAX_SOURCE_ABI_BYTES: usize = 128;

/// An opaque source-owner publication prepared without mutating RAM. Its bytes,
/// expected key state and serialized operation cannot be edited by a caller.
#[derive(Debug)]
pub struct MindSourceToken {
    digest: [u8; 32],
    source: String,
    existed: bool,
    operations: Vec<ModuleOp>,
}

fn source_error(message: impl Into<String>) -> mush_roll::RollError {
    mush_roll::RollError::Invalid {
        field: "mind source".into(),
        message: message.into(),
    }
}

fn valid_source_shape(source: &str, abi: &str) -> bool {
    source.len() <= MAX_STORED_MIND_SOURCE_BYTES
        && !abi.is_empty()
        && abi.len() <= MAX_SOURCE_ABI_BYTES
        && !abi.chars().any(char::is_control)
}

impl mush_roll::SourceAdapter for TacsimStore {
    type Token = MindSourceToken;

    fn prepare_source(
        &self,
        source: &str,
        abi: &str,
    ) -> Result<mush_roll::PreparedSource<MindSourceToken>, mush_roll::RollError> {
        if !valid_source_shape(source, abi) {
            return Err(source_error(
                "source or ABI exceeds the source-owner bounds",
            ));
        }
        let digest = mush_roll::source_digest(source);
        let bytes = *digest.as_bytes();
        let existed = self.mind_source_keys.contains(&bytes);
        if existed && self.mind_source(&bytes) != Some(source) {
            return Err(mush_roll::RollError::Quarantined(
                mindsrc_key(&bytes).to_string(),
            ));
        }
        if !existed && self.mind_sources.contains_key(&bytes) {
            return Err(source_error(
                "source exists without its immutable key claim",
            ));
        }
        // Even the first publication is fully serialized before any RAM changes.
        let operations = if existed {
            Vec::new()
        } else {
            let record = MindSourceRecord {
                sha256: digest.to_hex(),
                abi: abi.to_owned(),
                bytes: source.to_owned(),
            };
            let value =
                serde_json::to_value(&record).map_err(|error| source_error(error.to_string()))?;
            vec![ModuleOp {
                ns: NS.into(),
                op: ModuleWrite::JsonSet {
                    key: mindsrc_key(&bytes),
                    value,
                },
            }]
        };
        Ok(mush_roll::PreparedSource {
            digest,
            token: MindSourceToken {
                digest: bytes,
                source: source.to_owned(),
                existed,
                operations: operations.clone(),
            },
            operations,
        })
    }

    fn read_source(
        &self,
        digest: mush_core::DefinitionDigest,
    ) -> Result<&str, mush_roll::RollError> {
        self.mind_source(digest.as_bytes()).ok_or_else(|| {
            mush_roll::RollError::Quarantined(mindsrc_key(digest.as_bytes()).to_string())
        })
    }
}

impl TacsimStore {
    /// Check this immediately before publication, while owning the synchronous
    /// world operation. A claimed unreadable document is never repaired here.
    pub(crate) fn check_mind_source(
        &self,
        token: &MindSourceToken,
    ) -> Result<(), mush_roll::RollError> {
        let claimed = self.mind_source_keys.contains(&token.digest);
        if claimed && self.mind_source(&token.digest) != Some(token.source.as_str()) {
            return Err(mush_roll::RollError::Quarantined(
                mindsrc_key(&token.digest).to_string(),
            ));
        }
        if claimed != token.existed || (!claimed && self.mind_sources.contains_key(&token.digest)) {
            return Err(mush_roll::RollError::StaleOperation);
        }
        Ok(())
    }

    /// Infallible half of an already checked publication. The caller owns the
    /// world operation: no other source operation can interleave after checking.
    fn apply_mind_source(&mut self, token: MindSourceToken) {
        if !token.existed {
            self.mind_source_keys.insert(token.digest);
            self.mind_sources.insert(token.digest, token.source);
        }
    }
}

/// Publish one fully prepared source + revision in the current world operation.
/// Both owners and persistence authority are checked before either RAM owner
/// changes. Roll's callback only copies serialized operations and cannot mutate
/// the source store, so the source token remains valid after Roll publication.
/// All work afterward is infallible application and queuing of the one batch.
/// This must be the command's final fallible action: an enclosing handler error
/// discards deferred writes but cannot roll back these already published owners.
pub(crate) fn publish_mind_revision(
    ctx: &mut Ctx<'_>,
    prepared: mush_roll::PreparedSave<MindSourceToken>,
) -> Result<mush_roll::RevisionRef, mush_roll::RollError> {
    if prepared.revision().key.character != ctx.actor {
        return Err(mush_roll::RollError::Missing);
    }
    if let Some(module) = &ctx.module
        && (module != NS || ctx.reg.persist_delegation(module, mush_roll::NS).is_none())
    {
        return Err(source_error(
            "combined save lacks tacsim-to-roll persistence authority",
        ));
    }
    let (change, source, revision) = prepared.into_parts();
    if source.digest.as_bytes() != &source.token.digest
        || source.operations != source.token.operations
    {
        return Err(source_error("prepared source operation changed"));
    }
    let (roll, sources) = ctx
        .w
        .module_state_pair_mut::<mush_roll::RollStore, TacsimStore>()
        .ok_or(mush_roll::RollError::Missing)?;
    roll.check_prepared(&change)?;
    sources.check_mind_source(&source.token)?;
    let mut operations = source.operations;
    roll.publish_with(change, |roll_ops| operations.extend_from_slice(roll_ops))?;
    sources.apply_mind_source(source.token);
    for operation in operations {
        ctx.persist(operation);
    }
    Ok(revision)
}

/// Audit after all owners hydrate and before commands can prune revisions.
/// Bad source/revision documents remain untouched and quarantine their library
/// references. Source bytes are deliberately not collected in M6.1: older kept
/// match results do not yet expose a complete source-reference index.
pub(crate) fn audit_mind_revisions(ctx: &mut Ctx<'_>) -> Result<(), mush_roll::RollError> {
    let Some(roll) = ctx.w.module_state::<mush_roll::RollStore>() else {
        // Small rigs may compose TacSim alone; production always composes Roll.
        return Ok(());
    };
    let sources = ctx
        .w
        .module_state::<TacsimStore>()
        .ok_or(mush_roll::RollError::Missing)?;
    // Archive validity is independent of today's executable ABI. Fresh saves
    // still use RevisionRules; a retained source needs a current certificate
    // before it may execute again under the new engine.
    let audit =
        roll.prepare_source_audit(&crate::minds::authoring::RetainedRevisionRules, sources)?;
    let roll = ctx
        .w
        .module_state_mut::<mush_roll::RollStore>()
        .ok_or(mush_roll::RollError::Missing)?;
    roll.apply_source_audit(audit)?;
    for (key, reason) in roll.quarantine() {
        tracing::error!(%key, %reason, "tacsim: mind revision quarantined; collection disabled");
    }
    Ok(())
}

/// Store an immutable stock source. Preparation and serialization precede RAM
/// publication; a corrupt claimed key is refused and never silently repaired.
/// Returns whether a new document was queued.
pub(crate) fn put_mind_source(ctx: &mut Ctx<'_>, source: &str) -> bool {
    use mush_roll::SourceAdapter as _;
    let Some(store) = ctx.w.module_state::<TacsimStore>() else {
        return false;
    };
    let prepared = match store.prepare_source(source, crate::minds::MIND_ABI) {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::error!(%error, "tacsim: immutable stock source refused");
            return false;
        }
    };
    if prepared.operations.is_empty() {
        return false;
    }
    let Some(store) = ctx.w.module_state_mut::<TacsimStore>() else {
        return false;
    };
    if let Err(error) = store.check_mind_source(&prepared.token) {
        tracing::error!(%error, "tacsim: stock source publication refused");
        return false;
    }
    store.apply_mind_source(prepared.token);
    for operation in prepared.operations {
        ctx.persist(operation);
    }
    true
}

/// RAM-only fixture source installation. Production writes always use the fully
/// serialized prepared-source path; the holdings fixture seam has no Ctx.
pub(crate) fn install_mind_source(store: &mut TacsimStore, source: &str) -> Option<[u8; 32]> {
    use mush_roll::SourceAdapter as _;
    let prepared = store.prepare_source(source, crate::minds::MIND_ABI).ok()?;
    if prepared.operations.is_empty() {
        return None;
    }
    store.check_mind_source(&prepared.token).ok()?;
    let digest = *prepared.digest.as_bytes();
    store.apply_mind_source(prepared.token);
    Some(digest)
}

/// **Install one port map into RAM and queue its whole document** — the single
/// write path of `tacsim:port:{pack}/{entry}` (doc 18 cut IC3; L3). The
/// `PortApplier` calls it immediately after every act, inside the apply's one
/// batch, so the map and the acts it names commit together.
///
/// Like [`put_seed`], the one `else` arm that must not drop the write: a map is
/// the sole dedup of what a port made, and a dropped map would let the next
/// apply mint the capital twice. The serialize failure is structurally
/// unreachable (ids, integers and maps keyed by canonical ids); if it ever
/// happened, it pages and writes a minimal `{schema, pack, entry}` body so **the
/// key still exists** — which loads as unreadable and bars re-creation.
pub(crate) fn put_port_map(ctx: &mut Ctx<'_>, map: &crate::port::PortMap) {
    let key = map.key();
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        let index = (map.pack.clone(), map.entry.clone());
        store.port_map_keys.insert(index.clone());
        store.port_maps.insert(index, map.clone());
    }
    let value = match serde_json::to_value(map) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                %key, %error,
                "tacsim: a port map did not serialize; a minimal document is written so the key \
                 still bars re-creating what this port made. Repair the document by hand."
            );
            serde_json::json!({
                "schema": map.schema,
                "pack": map.pack.to_string(),
                "entry": map.entry.to_string(),
            })
        }
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// **Install the geology pin into RAM and queue its document** (doc 20 cut SM.1;
/// Q-20-12) — [`put_port_map`]'s pattern: the RAM install, then one whole-document
/// `JsonSet`. Written only by the boot pass at the first keyed boot (a module
/// document, on [`put_mind_source`]'s precedent; never an authoritative submission).
/// The record is two plain fields, so its serialization cannot fail; if it ever
/// did, the same two fields are written by hand so the key still exists.
pub(crate) fn put_geology_pin(ctx: &mut Ctx<'_>, record: &crate::surface::GeologyPinRecord) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.geology_pin = crate::surface::key::GeologyPinState::Present(record.clone());
    }
    let value = serde_json::to_value(record).unwrap_or_else(|error| {
        tracing::error!(%error, "tacsim: the geology pin did not serialize; written by hand");
        serde_json::json!({
            "fingerprint": record.fingerprint,
            "key_version": record.key_version,
        })
    });
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(crate::surface::GEOLOGY_PIN_KEY),
            value,
        },
    );
}

/// **Install an enrolment into RAM and queue its document** (doc 18 cut IC6) —
/// the single write path, [`put_port_map`]'s exact pattern: the RAM install
/// (the record and its key), then one whole-document `JsonSet`. The serialize
/// failure is structurally unreachable; if it ever happened, it logs and writes
/// a minimal `{schema, account}` body so **the key still exists** — which loads
/// as unreadable and still bars that account's re-enrolment.
pub(crate) fn put_enrolment(ctx: &mut Ctx<'_>, e: &crate::port::coop::Enrolment) {
    let key = crate::port::coop::enrolment_key(e.account);
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.enrolment_keys.insert(key.clone());
        store.enrolments.insert(e.account, e.clone());
    }
    let value = match serde_json::to_value(e) {
        Ok(value) => value,
        Err(error) => {
            // Never the key: its tail is the account's canonical hash.
            tracing::error!(
                pilot = e.pilot.0, %error,
                "tacsim: an enrolment did not serialize; a minimal document is written so the key \
                 still bars re-enrolment. Repair the document by hand."
            );
            serde_json::json!({
                "schema": e.schema,
                "account": e.account,
            })
        }
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// **Install a prospecting licence into RAM and queue its document** (doc 20 cut
/// SM.5) — the single write path, [`put_enrolment`]'s exact pattern: the RAM
/// install (the record and its key), then one whole-document `JsonSet`. The
/// serialize failure is structurally unreachable; if it ever happened, it logs and
/// writes a minimal `{schema, account}` body so **the key still exists** — which
/// loads as unreadable and bars that account until staff repair it.
pub(crate) fn put_licence(ctx: &mut Ctx<'_>, record: &crate::port::licence::Licence) {
    let key = crate::port::licence::licence_key(record.account);
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.licence_keys.insert(key.clone());
        store.licences.insert(record.account, record.clone());
    }
    let value = match serde_json::to_value(record) {
        Ok(value) => value,
        Err(error) => {
            // Never the key: its tail is the account's canonical hash.
            tracing::error!(
                holder = record.holder.0, %error,
                "tacsim: a licence did not serialize; a minimal document is written so the key \
                 still bars the account. Repair the document by hand."
            );
            serde_json::json!({
                "schema": record.schema,
                "account": record.account,
            })
        }
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// **Install a rig lease record into RAM and queue its document** (doc 20 cut
/// SM.3a) — the single write path, [`put_enrolment`]'s exact pattern: the RAM
/// install, then one whole-document `JsonSet`. The serialize failure is
/// structurally unreachable; if it ever happened, it logs and writes a minimal
/// `{schema, lease}` body so **the key still exists** — which loads as unreadable
/// and bars only that lease until staff repair it.
pub(crate) fn put_tv_lease(ctx: &mut Ctx<'_>, record: &crate::port::tv::TvLease) {
    let key = crate::port::tv::tv_lease_key(record.lease);
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.tv_leases.insert(
            record.lease,
            crate::port::tv::TvLeaseState::Present(record.clone()),
        );
    }
    let value = match serde_json::to_value(record) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                lease = record.lease, %error,
                "tacsim: a rig lease record did not serialize; a minimal document is written so \
                 the key stays claimed. Repair the document by hand."
            );
            serde_json::json!({
                "schema": record.schema,
                "lease": record.lease,
            })
        }
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// **Install a rig's projection into RAM and queue its document** (doc 20 cut SM.3b; §8,
/// lead ruling 1) — the single write path, [`put_tv_lease`]'s shape: the RAM install, then
/// one whole-document `JsonSet`. Only the `tacsim.surface` tick calls it, in its own outbox
/// batch beside the placement. The serialize failure is structurally unreachable; if it ever
/// happened it logs and writes nothing (the projection decides nothing, and the tick's next
/// write replaces it). **Returns whether it wrote** (doc 20 cut SM.4a, D24: the tell's heard
/// lines leave only after the projection that advances `told` is written).
pub(crate) fn put_tv_rec(ctx: &mut Ctx<'_>, rec: &crate::surface::excursion::TvRec) -> bool {
    let value = match serde_json::to_value(rec) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                tv = rec.tv.0, %error,
                "tacsim: a rig projection did not serialize; nothing was written"
            );
            return false;
        }
    };
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.tv_recs.insert(
            rec.tv.0,
            crate::surface::excursion::TvRecState::Present(rec.clone()),
        );
    }
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: rec.key(),
            value,
        },
    );
    true
}

/// **Install a site's mined ring into RAM and queue its document** (doc 20 cut SM.7; §6) — the
/// single write path, [`put_tv_rec`]'s shape: the RAM install, then one whole-document `JsonSet`.
/// Only the `tacsim.surface` tick's tell calls it, after the projection's write landed. The
/// serialize failure is structurally unreachable; if it ever happened it logs and writes nothing
/// (the ring is read by nothing, and the next write replaces it). Returns whether it wrote.
pub(crate) fn put_mined(ctx: &mut Ctx<'_>, ring: &crate::surface::mined::MinedRing) -> bool {
    let value = match serde_json::to_value(ring) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                site = %ring.site, %error,
                "tacsim: a mined ring did not serialize; nothing was written"
            );
            return false;
        }
    };
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.mined.insert(
            ring.site.clone(),
            crate::surface::mined::MinedState::Present(ring.clone()),
        );
    }
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: ring.key(),
            value,
        },
    );
    true
}

/// **Install a site's claim log into RAM and queue its document** (doc 20 cut SM.10; §9) — the
/// single write path, [`put_mined`]'s shape: the check, the RAM install, then one whole-document
/// `JsonSet`. Only the site clerk's claim steps (`crate::port::site`) call it, in the same handler
/// as the act it records. A log that fails its check or does not serialize logs and writes
/// nothing. Returns whether it wrote.
pub(crate) fn put_claim_log(ctx: &mut Ctx<'_>, log: &crate::surface::claims::ClaimLog) -> bool {
    if let Err(error) = log.check() {
        tracing::error!(
            site = %log.site, %error,
            "tacsim: a claim log failed its own check; nothing was written"
        );
        return false;
    }
    let value = match serde_json::to_value(log) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                site = %log.site, %error,
                "tacsim: a claim log did not serialize; nothing was written"
            );
            return false;
        }
    };
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.claim_logs.insert(
            log.site.clone(),
            crate::surface::claims::ClaimLogState::Present(log.clone()),
        );
    }
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new(log.key()),
            value,
        },
    );
    true
}

/// **Install a character's `+tv` preferences into RAM and queue their document** (doc 20 cut
/// SM.4a; D10) — the single write path, [`put_tv_rec`]'s shape: the RAM install, then one
/// whole-document `JsonSet`, from `+tv/quiet`'s handler (the `Authoritative` bracket, `/lease`'s
/// precedent). The serialize failure is structurally unreachable; if it ever happened it logs and
/// writes nothing.
pub(crate) fn put_tv_prefs(ctx: &mut Ctx<'_>, prefs: &crate::port::tv::TvPrefs) {
    let value = match serde_json::to_value(prefs) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                character = prefs.character.0, %error,
                "tacsim: a +tv preference did not serialize; nothing was written"
            );
            return;
        }
    };
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.tv_prefs.insert(prefs.character.0, prefs.clone());
    }
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: prefs.key(),
            value,
        },
    );
}

/// **Install a sailing plan into RAM and queue its document** (doc 18 cut IC9) —
/// the single write path, [`put_enrolment`]'s exact pattern: the RAM install (the
/// record and its key), then one whole-document `JsonSet`. The serialize failure
/// is structurally unreachable; if it ever happened, it logs and writes a minimal
/// `{schema, hull}` body so **the key still exists** — which loads as unreadable
/// and refuses that hull's undock until staff repair it.
pub(crate) fn put_voyage(ctx: &mut Ctx<'_>, voyage: &crate::port::voyage::Voyage) {
    let key = crate::port::voyage::voyage_key(voyage.hull);
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.voyage_keys.insert(key.clone());
        store.voyages.insert(voyage.hull, voyage.clone());
    }
    let value = match serde_json::to_value(voyage) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                hull = voyage.hull.0, %error,
                "tacsim: a sailing plan did not serialize; a minimal document is written so the key stays claimed. Repair the document by hand."
            );
            serde_json::json!({
                "schema": voyage.schema,
                "hull": voyage.hull,
            })
        }
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// **Install a queued sailing plan into RAM and queue its document** (doc 18 cut
/// IC11) — the single write path, [`put_voyage`]'s pattern with one addition
/// (reviewer F2): **the document is checked first**. A document that breaks a rule
/// is paged (`tracing::error!` naming the hull and the rule), and nothing is
/// installed or written; the caller refuses (a verb answers `ORDERS_DARK`, the order
/// arm treats it as refused — and pre-checks before every closed call so it never
/// reaches this after staging). A serialize failure, structurally unreachable, logs
/// and writes a minimal `{schema, hull}` body so the key stays claimed.
///
/// # Errors
///
/// The broken rule, when [`crate::port::orders::Orders::check`] refuses.
pub(crate) fn put_orders(
    ctx: &mut Ctx<'_>,
    orders: &crate::port::orders::Orders,
) -> Result<(), String> {
    if let Err(error) = orders.check() {
        tracing::error!(
            hull = orders.hull.0, %error,
            "tacsim: a sailing plan was refused before it was written (it breaks a rule); nothing was written"
        );
        return Err(error);
    }
    let key = crate::port::orders::orders_key(orders.hull);
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.orders_keys.insert(key.clone());
        store.orders.insert(orders.hull, orders.clone());
    }
    let value = match serde_json::to_value(orders) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                hull = orders.hull.0, %error,
                "tacsim: a queued sailing plan did not serialize; a minimal document is written so the key stays claimed. Repair the document by hand."
            );
            serde_json::json!({
                "schema": orders.schema,
                "hull": orders.hull,
            })
        }
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
    Ok(())
}

/// Remove a pin from RAM and queue its document's deletion — the crate's one `Del`.
///
/// Safe because no index floor is derived from a `tacsim:pin:` key: `reindex` floors
/// the watermark on `published_keys` and `seeded_keys` only, so removing a pin removes
/// exactly the pin (independent verify B4).
pub(crate) fn del_pin(ctx: &mut Ctx<'_>, subject: &CanonicalId) {
    let key = pin_key(subject);
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.pins.remove(subject);
    }
    push_op(ctx.out, ModuleWrite::Del { key });
}

/// Install a pin into RAM and queue its document — the single write path.
pub(crate) fn put_pin(ctx: &mut Ctx<'_>, record: PinRecord) {
    let key = pin_key(record.subject());
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.pins.insert(record.subject().clone(), record.clone());
    }
    let Ok(value) = serde_json::to_value(&record) else {
        tracing::error!(%key, "tacsim: a pin did not serialize");
        return;
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// Install a seed latch into RAM and queue its document — the single write path, and
/// the one that must ride the **same batch** as the objects it names.
pub(crate) fn put_seed(ctx: &mut Ctx<'_>, record: SeedApplication) {
    let key = seed_key(record.pack(), record.version());
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        let index = (record.pack().clone(), record.version());
        store.seeded_keys.insert(index.clone());
        store.seeded.insert(index, record.clone());
        store.reindex();
    }
    // **The one `else` arm here that must not drop the write** (independent verify S5).
    // In `put_pack`/`put_pin` dropping is harmless; here the objects `seed::apply`
    // created are already dirty and are persisted by the very next `flush_dirty`, so
    // dropping the latch would leave a reboot with the rooms and no latch —
    // `is_applied` false, and a second `commit` duplicating the mine. That is the one
    // fail-open direction the design forbids, and it was silent by construction. It is
    // structurally unreachable (`SeedApplicationWire` is `CanonicalId`/`u32`/`String`/
    // `ClockStamp`/`ObjId`, and `CanonicalId::serialize` is `serialize_str`, so it is a
    // valid `serde_json` map key and `to_value` cannot fail — pinned by
    // `tests::s5_a_full_seed_latch_always_serializes`), so this is the shape being made
    // right rather than a live defect: page level 2, and latch a minimal document so
    // **the key still exists**, since the key is what bars a re-seed.
    let value = match serde_json::to_value(&record) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                %key, %error,
                objects = record.objects(),
                "tacsim: a seed latch did not serialize; {} object(s) were created and a re-seed \
                 would duplicate them. A minimal latch is written so the key still bars one. \
                 Recommended: `@tacsim/pack show {}@{}`, then reconcile by hand. Only this pack \
                 is stopped.",
                record.objects(),
                record.pack(),
                record.version()
            );
            let mut fallback = serde_json::json!({
                "pack": record.pack().to_string(),
                "version": record.version(),
                "digest": record.digest().to_hex(),
                "objects": 0,
                "placed": serde_json::Map::new(),
            });
            if let Ok(applied_at) = serde_json::to_value(record.applied_at()) {
                fallback["applied_at"] = applied_at;
            }
            fallback
        }
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// Queue the allocator document.
pub(crate) fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(meta) = ctx
        .w
        .module_state::<TacsimStore>()
        .map(TacsimStore::meta_doc)
    else {
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: meta,
        },
    );
}

/// Build the slot from the module's boot documents.
///
/// **Total**: every document it cannot use is skipped loudly and the world boots.
/// Every index floor is derived from the documents' **parsed keys**, loaded or not
/// (E6b's R8), which is what makes a corrupt seed latch still bar a re-seed.
#[must_use]
pub fn load(kv: &[(SmolStr, serde_json::Value)], cfg: TacsimConfig) -> TacsimStore {
    let mut store = TacsimStore {
        cfg,
        ..TacsimStore::default()
    };
    for (key, value) in kv {
        let key = key.as_str();
        if key == crate::relay_world::KEY {
            store.relay_world_held = true;
            store.relay_world = serde_json::from_value::<crate::relay_world::Story>(value.clone())
                .ok()
                .filter(|s| s.check().is_ok());
        } else if key == crate::minds::registry_inspection::unwarranted::KEY {
            store.registry_unwarranted_held = true;
            store.registry_unwarranted = serde_json::from_value::<
                crate::minds::registry_inspection::unwarranted::Chronicle,
            >(value.clone())
            .ok()
            .filter(|c| c.check().is_ok());
            if store.registry_unwarranted.is_none() {
                tracing::warn!(%key, "tacsim: the unwarranted chronicle is unreadable; skipped");
            }
        } else if key == crate::relay_mobile::KEY {
            store.relay_mobile_held = true;
            store.relay_mobile =
                serde_json::from_value::<crate::relay_mobile::Programme>(value.clone())
                    .ok()
                    .filter(|p| p.check().is_ok());
        } else if key == crate::relay_foundry::KEY {
            store.relay_foundry_held = true;
            store.relay_foundry =
                serde_json::from_value::<crate::relay_foundry::Foundry>(value.clone())
                    .ok()
                    .filter(|f| f.check().is_ok());
        } else if let Some(rest) = key.strip_prefix("tacsim:pack:") {
            let Some((id, version)) = split_versioned(rest) else {
                tracing::warn!(%key, "tacsim: a pack ledger row has an unreadable key; skipped");
                continue;
            };
            // The claim on the version is the KEY's, not the body's.
            store.published_keys.insert((id.clone(), version));
            load_pack_row(&mut store, key, &id, version, value);
        } else if let Some(rest) = key.strip_prefix("tacsim:seed:") {
            let Some((pack, version)) = split_versioned(rest) else {
                tracing::warn!(%key, "tacsim: a seed latch has an unreadable key; skipped");
                continue;
            };
            // **The latch is the key.** A body that will not parse costs the
            // placement map, never the bar on re-seeding.
            store.seeded_keys.insert((pack.clone(), version));
            load_seed_row(&mut store, key, &pack, version, value);
        } else if let Some(rest) = key.strip_prefix("tacsim:pin:") {
            let Ok(subject) = CanonicalId::parse_canonical(rest) else {
                tracing::warn!(%key, "tacsim: a pin has an unreadable key; skipped");
                continue;
            };
            load_pin_row(&mut store, key, &subject, value);
        } else if let Some(arena) = crate::match_::roster_id_of_key(key) {
            // 16b cut K.1. A roster that will not parse costs the slot ids of
            // one arena and nothing else: the objects stand, the pack stands,
            // and the next apply of that arena rewrites the row. So it is
            // skipped loudly, like every other body in this loader.
            match crate::match_::roster_from_document(&arena, value) {
                Some(roster) => {
                    store.arena_rosters.insert(arena, roster);
                }
                None => tracing::warn!(
                    %key,
                    "tacsim: an arena roster is unreadable; that arena's unit slots cannot be \
                     resolved until it is applied again"
                ),
            }
        } else if key.starts_with(crate::match_::tutorial::KEY_PREFIX) {
            // 16b cut AH.1. A record that will not parse, or that disagrees
            // with its own key, costs that account its hints — it reads them
            // again — and nothing else. **Neither warning names the key**: its
            // tail is the account's canonical hash, and `AccountId`'s contract
            // is that it never reaches a journal.
            match crate::match_::tutorial::TutorialRec::from_document(value) {
                Some((account, rec)) if crate::match_::tutorial::tutorial_key(account) == key => {
                    store.tutorials.insert(account, rec);
                }
                Some(_) => {
                    tracing::warn!("tacsim: a tutorial record disagrees with its own key; skipped");
                }
                None => tracing::warn!("tacsim: a tutorial record did not parse; skipped"),
            }
        } else if let Some(rest) = key.strip_prefix(MINDSRC_PREFIX) {
            // 17b cut M3.3. The claim is the KEY's (the R8 rule): a document
            // that will not read still bars a rewrite under its key.
            let Some(digest) = crate::arena::digest_from_text(rest) else {
                tracing::warn!(%key, "tacsim: a mind source has an unreadable key; skipped");
                continue;
            };
            store.mind_source_keys.insert(digest);
            load_mind_source(&mut store, key, rest, digest, value);
        } else if let Some(rest) = key.strip_prefix(crate::port::PORT_MAP_PREFIX) {
            // Doc 18 cut IC3. The claim is the KEY's (the R8 rule): a map that
            // will not read still bars re-creating what its port made.
            let Some((pack, entry)) = crate::port::map::split_key_tail(rest) else {
                tracing::warn!(%key, "tacsim: a port map has an unreadable key; skipped");
                continue;
            };
            store.port_map_keys.insert((pack.clone(), entry.clone()));
            load_port_map(&mut store, key, &pack, &entry, value);
        } else if key.starts_with(crate::port::voyage::VOYAGE_PREFIX) {
            // Doc 18 cut IC9. The claim is the KEY's (the R8 rule): a sailing plan
            // that will not read stays claimed, so that hull's undock refuses and
            // the launch arm skips it, until staff repair the document.
            let Some(hull) = crate::port::voyage::voyage_key_owner(key) else {
                tracing::warn!(%key, "tacsim: a sailing plan has an unreadable key; skipped");
                continue;
            };
            store.voyage_keys.insert(SmolStr::new(key));
            load_voyage(&mut store, key, hull, value);
        } else if key.starts_with(crate::port::orders::ORDERS_PREFIX) {
            // Doc 18 cut IC11. The claim is the KEY's (the R8 rule): a queued sailing
            // plan that will not read stays claimed, so that hull's plan verbs refuse
            // and the order arm skips it (paging once a boot), until staff repair it.
            let Some(hull) = crate::port::orders::orders_key_owner(key) else {
                tracing::warn!(%key, "tacsim: a queued sailing plan has an unreadable key; skipped");
                continue;
            };
            store.orders_keys.insert(SmolStr::new(key));
            load_orders(&mut store, key, hull, value);
        } else if key.starts_with(crate::port::coop::COOP_PREFIX) {
            // Doc 18 cut IC6 (lead ruling L5). The claim is the KEY's (the R8
            // rule): an enrolment that will not read still bars its account's
            // re-enrolment, and staff repair the document.
            store.enrolment_keys.insert(SmolStr::new(key));
            load_enrolment(&mut store, key, value);
        } else if key.starts_with(crate::port::licence::LICENCE_PREFIX) {
            // Doc 20 cut SM.5. The claim is the KEY's (the R8 rule): a licence that
            // will not read still bars its account (its licence reads unreadable), and
            // staff repair the document.
            store.licence_keys.insert(SmolStr::new(key));
            load_licence(&mut store, key, value);
        } else if let Some(rest) = key.strip_prefix(crate::port::tv::TV_LEASE_PREFIX) {
            // Doc 20 cut SM.3a. The claim is the KEY's (the R8 rule): a rig lease record
            // that will not read stays claimed as unreadable, which bars only its own
            // lease, until staff repair the document.
            let Ok(lease) = rest.parse::<u64>() else {
                tracing::warn!(%key, "tacsim: a rig lease record has an unreadable key; skipped");
                continue;
            };
            load_tv_lease(&mut store, key, lease, value);
        } else if key.starts_with(crate::port::tv::TV_PREF_PREFIX) {
            // Doc 20 cut SM.4a (D10). Before the `tacsim:tv:` arm, though the prefixes differ at
            // the first character after `tacsim:tv` and neither ever reads the other's keys. A
            // preference that will not read is logged and dropped: it bars nothing, reads as not
            // quiet, and the next `+tv/quiet` rewrites it.
            let Some(character) = crate::port::tv::tv_pref_key_owner(key) else {
                tracing::warn!(%key, "tacsim: a +tv preference has an unreadable key; skipped");
                continue;
            };
            load_tv_prefs(&mut store, key, character, value);
        } else if key.starts_with(crate::surface::excursion::TV_REC_PREFIX) {
            // Doc 20 cut SM.3b. After the `tacsim:tvlease:` arm, though the prefixes differ at
            // the colon and neither ever reads the other's keys. A projection that will not
            // read is claimed as unreadable and bars nothing: it decides nothing, and the
            // `tacsim.surface` tick's next write replaces it.
            let Some(tv) = crate::surface::excursion::tv_rec_key_owner(key) else {
                tracing::warn!(%key, "tacsim: a rig projection has an unreadable key; skipped");
                continue;
            };
            load_tv_rec(&mut store, key, tv, value);
        } else if key.starts_with(crate::surface::mined::MINED_PREFIX) {
            // Doc 20 cut SM.7 (§6). Never a prefix of `tacsim:mindsrc:`'s keys nor they of it
            // (they differ at the first character after `tacsim:min`). A ring that will not read
            // is logged and read as unreadable: it bars nothing (nothing reads it yet), and the
            // tick's next write replaces it.
            let Some(site) = crate::surface::mined::mined_key_site(key) else {
                tracing::warn!(%key, "tacsim: a mined ring has an unreadable key; skipped");
                continue;
            };
            load_mined(&mut store, key, site, value);
        } else if key.starts_with(crate::surface::claims::CLAIM_LOG_PREFIX) {
            // Doc 20 cut SM.10 (§9). A log that will not read is dropped loudly (paged once,
            // here) and read as unreadable: it bars nothing — the clerk's re-chronicle is
            // guarded by the chronicle itself, a refund by its idempotency key, the paper
            // contract by its re-find — and the clerk's next write replaces it.
            let Some(site) = crate::surface::claims::claim_log_key_site(key) else {
                tracing::warn!(%key, "tacsim: a claim log has an unreadable key; skipped");
                continue;
            };
            load_claim_log(&mut store, key, site, value);
        } else if key.starts_with(crate::port_tick::NOTICE_PREFIX) {
            // Doc 18 cut IC5 (lead ruling L10). A queue that will not read costs
            // that character its queued notices and nothing else: the key bars
            // nothing, the next `queue_notice` rewrites it, and boot writes nothing.
            let Some(who) = crate::port_tick::notice_key_owner(key) else {
                tracing::error!(%key, "tacsim: a reconnect-notice document has an unreadable key; skipped");
                continue;
            };
            load_notice_queue(&mut store, key, who, value);
        } else if key == crate::surface::GEOLOGY_PIN_KEY {
            // Doc 20 cut SM.1 (Q-20-12). The claim is the KEY's (the R8 rule): a pin
            // that will not read is `Unreadable`, which disables the geology layer and
            // is never rewritten by the boot.
            store.geology_pin = crate::surface::key::load_geology_pin(key, value);
        } else if key == META_KEY {
            load_meta(&mut store, value);
        }
        // Every other key belongs to a forward schema; tolerated in both directions.
    }
    store.reindex();
    store
}

/// One `tacsim:notice:{obj}` body (doc 18 cut IC5): decoded, checked, and held to
/// the identity rule (`queue.who` is the key's character). Any failure drops that
/// character's queue loudly and nothing else.
fn load_notice_queue(store: &mut TacsimStore, key: &str, who: ObjId, value: &serde_json::Value) {
    let queue = serde_json::from_value::<crate::port_tick::NoticeQueue>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|queue| queue.check().map(|()| queue))
        .and_then(|queue| {
            if queue.who == who {
                Ok(queue)
            } else {
                Err(format!(
                    "the queue names #{}, not its key's #{}",
                    queue.who.0, who.0
                ))
            }
        });
    match queue {
        Ok(queue) => {
            store.notices.insert(who, queue);
        }
        Err(error) => tracing::error!(
            %key, %error,
            "tacsim: a reconnect-notice document is unreadable; that character's queued notices \
             are dropped"
        ),
    }
}

fn load_pack_row(
    store: &mut TacsimStore,
    key: &str,
    id: &CanonicalId,
    version: u32,
    value: &serde_json::Value,
) {
    match serde_json::from_value::<PackRecord>(value.clone()) {
        Ok(record) if record.id() == id && record.version() == version => {
            store.published.insert((id.clone(), version), record);
        }
        Ok(record) => tracing::warn!(
            %key,
            record = %record.id(),
            "tacsim: a pack ledger row disagrees with its own key; skipped"
        ),
        Err(error) => {
            tracing::warn!(%key, %error, "tacsim: a pack ledger row did not parse; skipped");
        }
    }
}

fn load_seed_row(
    store: &mut TacsimStore,
    key: &str,
    pack: &CanonicalId,
    version: u32,
    value: &serde_json::Value,
) {
    match serde_json::from_value::<SeedApplication>(value.clone()) {
        Ok(record) if record.pack() == pack && record.version() == version => {
            store.seeded.insert((pack.clone(), version), record);
        }
        Ok(record) => tracing::warn!(
            %key,
            record = %record.pack(),
            "tacsim: a seed latch disagrees with its own key; skipped (the key still bars a re-seed)"
        ),
        Err(error) => tracing::warn!(
            %key, %error,
            "tacsim: a seed latch did not parse; skipped (the key still bars a re-seed)"
        ),
    }
}

fn load_pin_row(
    store: &mut TacsimStore,
    key: &str,
    subject: &CanonicalId,
    value: &serde_json::Value,
) {
    match serde_json::from_value::<PinRecord>(value.clone()) {
        Ok(record) if record.subject() == subject => {
            store.pins.insert(subject.clone(), record);
        }
        Ok(record) => tracing::warn!(
            %key,
            record = %record.subject(),
            "tacsim: a pin disagrees with its own key; skipped"
        ),
        Err(error) => tracing::warn!(%key, %error, "tacsim: a pin did not parse; skipped"),
    }
}

/// **One `tacsim:mindsrc:` document** (17b cut M3.3): held only when its
/// `sha256` is its key's tail and its bytes hash to it — anything else is
/// skipped loudly (the identity rule, DL-07, and the content address).
fn load_mind_source(
    store: &mut TacsimStore,
    key: &str,
    tail: &str,
    digest: [u8; 32],
    value: &serde_json::Value,
) {
    use sha2::{Digest as _, Sha256};
    // Inspect borrowed fields before copying any part of the document. This is
    // the same closed three-string shape as MindSourceRecord, but decoding an
    // owned record first would clone an unbounded corrupt source or unknown
    // subtree before the source-owner limits could reject it.
    let record = value.as_object().filter(|record| record.len() == 3);
    let fields = record.and_then(|record| {
        Some((
            record.get("sha256")?.as_str()?,
            record.get("abi")?.as_str()?,
            record.get("bytes")?.as_str()?,
        ))
    });
    let Some((sha256, abi, source)) = fields else {
        tracing::warn!(
            %key,
            "tacsim: a mind source did not parse; skipped (the key still bars a rewrite)"
        );
        return;
    };
    if !valid_source_shape(source, abi) {
        tracing::warn!(
            %key,
            "tacsim: mind source exceeds owner bounds; skipped (the key still bars a rewrite)"
        );
    } else if sha256 != tail {
        tracing::warn!(
            %key,
            "tacsim: a mind source disagrees with its own key; skipped (the key still bars a \
             rewrite)"
        );
    } else if <[u8; 32]>::from(Sha256::digest(source.as_bytes())) != digest {
        tracing::warn!(
            %key,
            "tacsim: a mind source's bytes do not hash to its key; skipped (the key still \
             bars a rewrite)"
        );
    } else {
        store.mind_sources.insert(digest, source.to_owned());
    }
}

/// **One `tacsim:port:` document** (doc 18 cut IC3): held only when it decodes
/// and its `pack` and `entry` are its key's (the identity rule, DL-07);
/// anything else is skipped loudly and the key stays claimed, so the port map
/// reads [`crate::port::PortMapState::Unreadable`].
fn load_port_map(
    store: &mut TacsimStore,
    key: &str,
    pack: &CanonicalId,
    entry: &CanonicalId,
    value: &serde_json::Value,
) {
    match serde_json::from_value::<crate::port::PortMap>(value.clone()) {
        Ok(map) if &map.pack == pack && &map.entry == entry => {
            store.port_maps.insert((pack.clone(), entry.clone()), map);
        }
        Ok(_) => tracing::error!(
            %key,
            "tacsim: a port map disagrees with its own key; skipped (the key still bars \
             re-creation; repair the document by hand)"
        ),
        Err(error) => tracing::error!(
            %key, %error,
            "tacsim: a port map did not parse; skipped (the key still bars re-creation; repair \
             the document by hand)"
        ),
    }
}

/// One `tacsim:coop:{account}` body (doc 18 cut IC6): decoded strictly, checked,
/// and held to the identity rule (`enrolment_key(e.account)` is its key). Any
/// failure skips it loudly; its key, already recorded, keeps it `Unreadable`.
/// **No warning names the key or the account**: the key's tail is the account's
/// canonical hash, and `AccountId`'s contract is that it never reaches a journal
/// (the `tacsim:tutorial:` loader's rule).
fn load_enrolment(store: &mut TacsimStore, key: &str, value: &serde_json::Value) {
    let enrolment = serde_json::from_value::<crate::port::coop::Enrolment>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|e| e.check().map(|()| e))
        .and_then(|e| {
            if crate::port::coop::enrolment_key(e.account) == key {
                Ok(e)
            } else {
                Err("the enrolment disagrees with its own key".to_owned())
            }
        });
    match enrolment {
        Ok(e) => {
            store.enrolments.insert(e.account, e);
        }
        Err(error) => tracing::warn!(
            %error,
            "tacsim: an enrolment document is unreadable; skipped (its key still bars that \
             account's re-enrolment; repair the document by hand)"
        ),
    }
}

/// One `tacsim:licence:{account}` body (doc 20 cut SM.5): decoded strictly, checked,
/// and held to the identity rule (`licence_key(record.account)` is its key). Any
/// failure skips it loudly; its key, already recorded, keeps that account's licence
/// `Unreadable`. **No warning names the key or the account** (the enrolment loader's
/// rule: the key's tail is the account's canonical hash).
fn load_licence(store: &mut TacsimStore, key: &str, value: &serde_json::Value) {
    let record = serde_json::from_value::<crate::port::licence::Licence>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|record| record.check().map(|()| record))
        .and_then(|record| {
            if crate::port::licence::licence_key(record.account) == key {
                Ok(record)
            } else {
                Err("the licence disagrees with its own key".to_owned())
            }
        });
    match record {
        Ok(record) => {
            store.licences.insert(record.account, record);
        }
        Err(error) => tracing::warn!(
            %error,
            "tacsim: a licence document is unreadable; skipped (its key still bars that \
             account; repair the document by hand)"
        ),
    }
}

/// One `tacsim:tvlease:{lease}` body (doc 20 cut SM.3a): decoded strictly, checked,
/// and held to the identity rule (`record.lease` is its key's). Any failure keeps the
/// key claimed as unreadable, with whatever of its pilot, hull and TV the raw body
/// still names (so the right pilot's `+tv` reads dark and the right hull is held).
fn load_tv_lease(store: &mut TacsimStore, key: &str, lease: u64, value: &serde_json::Value) {
    let record = serde_json::from_value::<crate::port::tv::TvLease>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|record| record.check().map(|()| record))
        .and_then(|record| {
            if record.lease == lease {
                Ok(record)
            } else {
                Err("the record disagrees with its own key".to_owned())
            }
        });
    match record {
        Ok(record) => {
            store
                .tv_leases
                .insert(lease, crate::port::tv::TvLeaseState::Present(record));
        }
        Err(error) => {
            tracing::error!(
                %key, %error,
                "tacsim: a rig lease record is unreadable; skipped (the key stays claimed and bars \
                 only its own lease; repair the document by hand)"
            );
            let obj = |field: &str| {
                value
                    .get(field)
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .map(ObjId)
            };
            store.tv_leases.insert(
                lease,
                crate::port::tv::TvLeaseState::Unreadable {
                    pilot: obj("pilot"),
                    hull: obj("hull"),
                    tv: obj("tv"),
                },
            );
        }
    }
}

/// One `tacsim:tvpref:{character}` body (doc 20 cut SM.4a): decoded strictly, its schema and
/// the identity rule (`prefs.character` is its key's) held. Any failure is logged and the
/// document dropped (absent reads as not quiet).
fn load_tv_prefs(store: &mut TacsimStore, key: &str, character: ObjId, value: &serde_json::Value) {
    match serde_json::from_value::<crate::port::tv::TvPrefs>(value.clone()) {
        Ok(prefs)
            if prefs.schema == crate::port::tv::TV_PREF_SCHEMA && prefs.character == character =>
        {
            store.tv_prefs.insert(character.0, prefs);
        }
        Ok(_) => tracing::error!(
            %key,
            "tacsim: a +tv preference disagrees with its key or schema; it reads as not quiet"
        ),
        Err(error) => tracing::error!(
            %key, %error,
            "tacsim: a +tv preference is unreadable; it reads as not quiet"
        ),
    }
}

/// One `tacsim:tv:{tv}` body (doc 20 cut SM.3b): decoded strictly, checked, and held to the
/// identity rule (`rec.tv` is its key's). Any failure is logged once and reads
/// [`crate::surface::excursion::TvRecState::Unreadable`], which bars nothing.
fn load_tv_rec(store: &mut TacsimStore, key: &str, tv: ObjId, value: &serde_json::Value) {
    let rec = serde_json::from_value::<crate::surface::excursion::TvRec>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|rec| rec.check().map(|()| rec))
        .and_then(|rec| {
            if rec.tv == tv {
                Ok(rec)
            } else {
                Err("the projection disagrees with its own key".to_owned())
            }
        });
    match rec {
        Ok(rec) => {
            store
                .tv_recs
                .insert(tv.0, crate::surface::excursion::TvRecState::Present(rec));
        }
        Err(error) => {
            tracing::error!(
                %key, %error,
                "tacsim: a rig projection is unreadable; it bars nothing and the surface tick's \
                 next write replaces it"
            );
            store
                .tv_recs
                .insert(tv.0, crate::surface::excursion::TvRecState::Unreadable);
        }
    }
}

/// One `tacsim:claimlog:{site}` body (doc 20 cut SM.10): decoded strictly, checked, and held to
/// the identity rule (`log.site` is its key's). Any failure is paged here, once a boot, and reads
/// [`crate::surface::claims::ClaimLogState::Unreadable`], which bars nothing.
fn load_claim_log(
    store: &mut TacsimStore,
    key: &str,
    site: CanonicalId,
    value: &serde_json::Value,
) {
    let log = serde_json::from_value::<crate::surface::claims::ClaimLog>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|log| log.check().map(|()| log))
        .and_then(|log| {
            if log.site == site {
                Ok(log)
            } else {
                Err("the log disagrees with its own key".to_owned())
            }
        });
    match log {
        Ok(log) => {
            store
                .claim_logs
                .insert(site, crate::surface::claims::ClaimLogState::Present(log));
        }
        Err(error) => {
            tracing::error!(
                %key, %error,
                runbook = "doc 20 cut SM.10: the site clerk re-finds what the log recorded (the \
                           chronicle, the refund's idempotency key, the paper contract by its \
                           issuer and name) and its next write replaces the log",
                "tacsim: a claim log is unreadable; it bars nothing"
            );
            store
                .claim_logs
                .insert(site, crate::surface::claims::ClaimLogState::Unreadable);
        }
    }
}

/// One `tacsim:mined:{site}` body (doc 20 cut SM.7): decoded strictly, checked, and held to the
/// identity rule (`ring.site` is its key's). Any failure is logged once and reads
/// [`crate::surface::mined::MinedState::Unreadable`], which bars nothing.
fn load_mined(store: &mut TacsimStore, key: &str, site: CanonicalId, value: &serde_json::Value) {
    let ring = serde_json::from_value::<crate::surface::mined::MinedRing>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|ring| ring.check().map(|()| ring))
        .and_then(|ring| {
            if ring.site == site {
                Ok(ring)
            } else {
                Err("the ring disagrees with its own key".to_owned())
            }
        });
    match ring {
        Ok(ring) => {
            store
                .mined
                .insert(site, crate::surface::mined::MinedState::Present(ring));
        }
        Err(error) => {
            tracing::error!(
                %key, %error,
                "tacsim: a mined ring is unreadable; it bars nothing and the surface tick's next \
                 write replaces it"
            );
            store
                .mined
                .insert(site, crate::surface::mined::MinedState::Unreadable);
        }
    }
}

/// One `tacsim:voyage:{hull}` body (doc 18 cut IC9): decoded strictly, checked,
/// and held to the identity rule (`voyage_key(v.hull)` is its key). Any failure
/// skips it loudly; its key, already recorded, keeps it `Unreadable`.
fn load_voyage(store: &mut TacsimStore, key: &str, hull: ObjId, value: &serde_json::Value) {
    let voyage = serde_json::from_value::<crate::port::voyage::Voyage>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|v| v.check().map(|()| v))
        .and_then(|v| {
            if crate::port::voyage::voyage_key(v.hull) == key {
                Ok(v)
            } else {
                Err("the sailing plan disagrees with its own key".to_owned())
            }
        });
    match voyage {
        Ok(v) => {
            store.voyages.insert(v.hull, v);
        }
        Err(error) => tracing::error!(
            hull = hull.0,
            %error,
            "tacsim: a sailing plan is unreadable; skipped (its key stays claimed: that hull's +ship/undock refuses and the launch arm skips it; repair the document by hand)"
        ),
    }
}

/// One `tacsim:orders:{hull}` body (doc 18 cut IC11): decoded strictly, checked, and
/// held to the identity rule (`orders_key(o.hull)` is its key). Any failure skips it
/// loudly — the page — and its key, already recorded, keeps it `Unreadable`.
fn load_orders(store: &mut TacsimStore, key: &str, hull: ObjId, value: &serde_json::Value) {
    let orders = serde_json::from_value::<crate::port::orders::Orders>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|o| o.check().map(|()| o))
        .and_then(|o| {
            if crate::port::orders::orders_key(o.hull) == key {
                Ok(o)
            } else {
                Err("the sailing plan disagrees with its own key".to_owned())
            }
        });
    match orders {
        Ok(o) => {
            store.orders.insert(o.hull, o);
        }
        Err(error) => tracing::error!(
            hull = hull.0,
            %error,
            "tacsim: a queued sailing plan is unreadable; skipped (its key stays claimed: that hull's +ship/orders and +ship/order refuse and the order arm skips it; repair the document by hand)"
        ),
    }
}

fn load_meta(store: &mut TacsimStore, value: &serde_json::Value) {
    // Tolerated field by field: a meta document that is half nonsense contributes
    // whatever floors it can and the parsed keys supply the rest.
    let Some(watermark) = value
        .get("watermark")
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    for (id, version) in watermark {
        let Ok(id) = CanonicalId::parse_canonical(id) else {
            continue;
        };
        let Some(version) = version.as_u64().and_then(|v| u32::try_from(v).ok()) else {
            continue;
        };
        let entry = store.watermark.entry(id).or_insert(version);
        *entry = (*entry).max(version);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- E7 cut 1 independent verify fold ----

    /// **S5 — the one write path that must not drop its document.**
    ///
    /// [`put_seed`]'s serialize-failure arm is structurally unreachable, and this pins
    /// that it stays so rather than leaving it assumed: a full latch — `MAX_SEED_OBJECTS`
    /// placements, a real `ClockStamp`, a real digest — round-trips through
    /// `serde_json::to_value` and back. `CanonicalId::serialize` is `serialize_str`, so
    /// every `placed` key is a valid JSON map key and there is no field in
    /// `SeedApplicationWire` that can refuse. The loud fallback in `put_seed` is the
    /// belt to this braces.
    #[test]
    fn s5_a_full_seed_latch_always_serializes() {
        let mut placed: BTreeMap<CanonicalId, ObjId> = BTreeMap::new();
        for i in 0..crate::pack::MAX_SEED_OBJECTS {
            let id = CanonicalId::new(format!("place_{i}")).expect("a canonical id");
            placed.insert(id, ObjId(u32::try_from(i).unwrap_or(0) + 2));
        }
        let record = SeedApplication::new(
            CanonicalId::new("t.big").expect("a canonical id"),
            1,
            mush_core::DefinitionDigest::from_bytes([7_u8; 32]),
            mush_core::ClockStamp {
                clock: mush_core::ClockRef::WorldIc { generation: 1 },
                at: 1_234,
            },
            placed,
        )
        .expect("a full latch builds");
        let value = serde_json::to_value(&record).expect("a full latch always serializes");
        let back: SeedApplication =
            serde_json::from_value(value).expect("and re-enters through its wire");
        assert_eq!(back, record);
        assert_eq!(back.objects() as usize, crate::pack::MAX_SEED_OBJECTS);
    }

    use mush_roll::SourceAdapter as _;

    #[test]
    fn m6_source_preparation_is_read_only_and_reboot_preserves_exact_bytes() {
        let mut store = TacsimStore::default();
        let source = "fn think() { `literal\r\nbytes` }\n";
        let prepared = store
            .prepare_source(source, crate::minds::MIND_ABI)
            .unwrap();
        assert!(store.mind_source_keys.is_empty());
        assert!(store.mind_sources.is_empty());
        assert_eq!(prepared.operations.len(), 1);
        let digest = *prepared.digest.as_bytes();
        let document = match &prepared.operations[0].op {
            ModuleWrite::JsonSet { key, value } => (key.clone(), value.clone()),
            _ => panic!("source preparation only writes one immutable document"),
        };
        store.check_mind_source(&prepared.token).unwrap();
        store.apply_mind_source(prepared.token);
        assert_eq!(store.mind_source(&digest), Some(source));
        let rebooted = load(&[document], TacsimConfig::default());
        assert_eq!(rebooted.mind_source(&digest), Some(source));
        assert!(
            rebooted
                .prepare_source(source, "later-abi")
                .unwrap()
                .operations
                .is_empty()
        );
    }

    #[test]
    fn m6_claimed_corrupt_source_refuses_without_repair_or_ram_publication() {
        let source = "fn think() { () }";
        let digest = mush_roll::source_digest(source);
        let key = mindsrc_key(digest.as_bytes());
        let bad = serde_json::json!({
            "sha256": digest.to_hex(), "abi": crate::minds::MIND_ABI,
            "bytes": "tampered bytes",
        });
        let store = load(&[(key.clone(), bad)], TacsimConfig::default());
        assert!(store.mind_source_keys.contains(digest.as_bytes()));
        assert!(!store.has_mind_source(digest.as_bytes()));
        assert!(
            matches!(store.prepare_source(source, crate::minds::MIND_ABI),
            Err(mush_roll::RollError::Quarantined(ref got)) if got == key.as_str())
        );
        assert!(store.mind_sources.is_empty());
    }

    #[test]
    fn m6_source_boot_rejects_oversized_or_malformed_documents_without_releasing_the_key() {
        let source = "fn think() { () }";
        let digest = mush_roll::source_digest(source);
        let key = mindsrc_key(digest.as_bytes());
        let valid = serde_json::json!({
            "sha256": digest.to_hex(), "abi": crate::minds::MIND_ABI,
            "bytes": source,
        });
        for (field, malformed) in [
            (
                "bytes",
                serde_json::json!("x".repeat(MAX_STORED_MIND_SOURCE_BYTES + 1)),
            ),
            (
                "abi",
                serde_json::json!("a".repeat(MAX_SOURCE_ABI_BYTES + 1)),
            ),
            ("abi", serde_json::json!("bad\nabi")),
            (
                "sha256",
                serde_json::json!("f".repeat(MAX_STORED_MIND_SOURCE_BYTES + 1)),
            ),
            // Both a wrong-type known field and an unknown subtree must be
            // rejected without copying the subtree into an owned record.
            (
                "bytes",
                serde_json::json!({"nested": "x".repeat(MAX_STORED_MIND_SOURCE_BYTES + 1)}),
            ),
            (
                "unknown",
                serde_json::json!({"nested": "x".repeat(MAX_STORED_MIND_SOURCE_BYTES + 1)}),
            ),
        ] {
            let mut document = valid.clone();
            document[field] = malformed;
            let store = load(&[(key.clone(), document)], TacsimConfig::default());
            assert!(
                store.mind_source_keys.contains(digest.as_bytes()),
                "{field}"
            );
            assert!(store.mind_sources.is_empty(), "{field}");
            assert!(
                matches!(
                    store.read_source(digest),
                    Err(mush_roll::RollError::Quarantined(_))
                ),
                "{field}"
            );
            assert!(
                matches!(
                    store.prepare_source(source, crate::minds::MIND_ABI),
                    Err(mush_roll::RollError::Quarantined(_))
                ),
                "{field}"
            );
        }
    }

    #[test]
    fn m6_source_boot_accepts_exact_byte_bounds_including_json_escaping() {
        // The source-owner limit is decoded UTF-8 bytes, not JSON's escaped
        // document size. A NUL needs six encoded bytes but one source byte.
        let source = "\0".repeat(MAX_STORED_MIND_SOURCE_BYTES);
        let abi = "a".repeat(MAX_SOURCE_ABI_BYTES);
        let prepared = TacsimStore::default()
            .prepare_source(&source, &abi)
            .unwrap();
        let digest = prepared.digest;
        let document = match &prepared.operations[0].op {
            ModuleWrite::JsonSet { key, value } => (key.clone(), value.clone()),
            _ => panic!("source preparation only writes one immutable document"),
        };
        let store = load(&[document], TacsimConfig::default());
        assert_eq!(store.read_source(digest).unwrap(), source);
        assert!(store.mind_source_keys.contains(digest.as_bytes()));
        assert!(
            store
                .prepare_source(&source, &abi)
                .unwrap()
                .operations
                .is_empty()
        );
    }

    #[test]
    fn m6_source_bound_admits_open_standard_and_rejects_oversize_before_hash_or_write() {
        let store = TacsimStore::default();
        assert!(crate::minds::content::STANDARD_RHAI.len() > mush_roll::MAX_SOURCE_BYTES);
        assert!(
            store
                .prepare_source(crate::minds::content::STANDARD_RHAI, crate::minds::MIND_ABI)
                .is_ok()
        );
        assert!(
            store
                .prepare_source(&"x".repeat(MAX_STORED_MIND_SOURCE_BYTES + 1), "abi")
                .is_err()
        );
        assert!(
            store
                .prepare_source("x", &"a".repeat(MAX_SOURCE_ABI_BYTES + 1))
                .is_err()
        );
        assert!(store.mind_sources.is_empty());
        assert!(store.mind_source_keys.is_empty());
    }

    #[test]
    fn m6_stale_source_token_cannot_rewrite_an_intervening_publication() {
        let mut store = TacsimStore::default();
        let stale = store.prepare_source("source", "abi").unwrap();
        let winner = store.prepare_source("source", "abi").unwrap();
        store.check_mind_source(&winner.token).unwrap();
        store.apply_mind_source(winner.token);
        assert_eq!(
            store.check_mind_source(&stale.token),
            Err(mush_roll::RollError::StaleOperation)
        );
        assert!(
            store
                .prepare_source("source", "abi")
                .unwrap()
                .operations
                .is_empty()
        );
    }

    /// A minimal source owner for the integration transaction tests. Semantic
    /// validation belongs to authoring::RevisionRules's independent tests.
    struct SourceModule;
    impl mush_core::GameModule for SourceModule {
        fn name(&self) -> &'static str {
            NS
        }
        fn register(
            &mut self,
            reg: &mut mush_core::Registrar<'_>,
        ) -> Result<(), mush_core::ModError> {
            reg.state(TacsimStore::default());
            Ok(())
        }
        fn on_world_loaded(
            &mut self,
            ctx: &mut mush_core::ModuleCtx<'_>,
            data: mush_core::ModuleBootData,
        ) -> Result<(), mush_core::ModError> {
            *ctx.state_mut::<TacsimStore>().expect("source owner slot") =
                load(&data.kv, TacsimConfig::default());
            Ok(())
        }
        fn on_all_loaded(
            &mut self,
            ctx: &mut mush_core::ModuleCtx<'_>,
        ) -> Result<(), mush_core::ModError> {
            audit_mind_revisions(ctx).map_err(|error| mush_core::ModError::msg(error.to_string()))
        }
    }
    fn transaction_engine(delegate: bool) -> mush_core::Engine {
        mush_core::Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![
                Box::new(SourceModule),
                Box::new(mush_roll::RollModule::with_writers(delegate.then_some(NS))),
            ],
        )
        .unwrap()
    }
    fn transaction_draft() -> mush_roll::WorkingCopy {
        mush_roll::WorkingCopy {
            source: "fn think(view, card) {}".into(),
            envelope: mush_roll::Envelope {
                schema: serde_json::json!({"sections": []}),
                presets: serde_json::json!({}),
                profiles: serde_json::json!({"Default": {}}),
                rules: serde_json::json!({}),
                declared_class: "I".into(),
                mind_abi: crate::minds::MIND_ABI.into(),
            },
        }
    }
    fn create_transaction_draft(engine: &mut mush_core::Engine, actor: ObjId) -> Vec<ModuleOp> {
        let roll = engine
            .world
            .module_state_mut::<mush_roll::RollStore>()
            .unwrap();
        let change = roll
            .prepare_create(actor, "test", transaction_draft())
            .unwrap();
        let mut ops = Vec::new();
        roll.publish_with(change, |batch| ops.extend_from_slice(batch))
            .unwrap();
        ops
    }
    fn prepare_transaction(
        engine: &mush_core::Engine,
        actor: ObjId,
    ) -> mush_roll::PreparedSave<MindSourceToken> {
        let roll = engine.world.module_state::<mush_roll::RollStore>().unwrap();
        roll.prepare_save(
            actor,
            "test",
            roll.mind(actor, "test").unwrap().base(),
            "notes",
            None,
            &crate::minds::authoring::RevisionRules,
            engine.world.module_state::<TacsimStore>().unwrap(),
        )
        .unwrap()
    }
    fn publish_transaction(
        engine: &mut mush_core::Engine,
        actor: ObjId,
        prepared: mush_roll::PreparedSave<MindSourceToken>,
    ) -> (Result<mush_roll::RevisionRef, mush_roll::RollError>, Outbox) {
        let mut out = Outbox::new();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let result = publish_mind_revision(&mut ctx, prepared);
        (result, out)
    }

    #[test]
    fn m6_combined_save_publishes_both_owners_in_one_batch_and_survives_reload() {
        let actor = ObjId(10);
        let mut engine = transaction_engine(true);
        let mut operations = create_transaction_draft(&mut engine, actor);
        let prepared = prepare_transaction(&engine, actor);
        let expected = prepared.revision();
        let source = prepared.source().digest;
        assert!(
            engine
                .world
                .module_state::<TacsimStore>()
                .unwrap()
                .mind_sources
                .is_empty()
        );
        assert!(
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .unwrap()
                .mind(actor, "test")
                .unwrap()
                .latest()
                .is_none()
        );
        let (saved, mut out) = publish_transaction(&mut engine, actor, prepared);
        let saved = saved.unwrap();
        assert_eq!(
            saved, expected,
            "the published revision keeps its prepared identity"
        );
        let batch: Vec<_> = out
            .take_deferred()
            .into_iter()
            .map(|deferred| match deferred {
                mush_core::Deferred::Persist(op) => op,
                _ => panic!("the transaction only queues durable writes"),
            })
            .collect();
        assert_eq!(batch.iter().filter(|op| op.ns == NS).count(), 1);
        assert!(batch.iter().any(|op| op.ns == mush_roll::NS));
        assert!(
            engine
                .world
                .module_state::<TacsimStore>()
                .unwrap()
                .has_mind_source(source.as_bytes())
        );
        assert_eq!(
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .unwrap()
                .revision(actor, saved.key)
                .unwrap()
                .source(),
            source,
            "the opaque save binds the revision to the prepared source"
        );
        operations.extend(batch);
        let mut disk: BTreeMap<(SmolStr, SmolStr), serde_json::Value> = BTreeMap::new();
        for operation in operations {
            match operation.op {
                ModuleWrite::JsonSet { key, value } => {
                    disk.insert((operation.ns, key), value);
                }
                _ => panic!("create/save need no removals"),
            }
        }
        let docs = |namespace: &str| -> Vec<_> {
            disk.iter()
                .filter(|((ns, _), _)| ns == namespace)
                .map(|((_, key), value)| (key.clone(), value.clone()))
                .collect()
        };
        let reloaded_sources = load(&docs(NS), TacsimConfig::default());
        let mut reloaded_roll = mush_roll::RollStore::load(&mush_core::ModuleBootData {
            kv: docs(mush_roll::NS),
            ..Default::default()
        })
        .unwrap();
        let audit = reloaded_roll
            .prepare_source_audit(&crate::minds::authoring::RevisionRules, &reloaded_sources)
            .unwrap();
        reloaded_roll.apply_source_audit(audit).unwrap();
        assert!(reloaded_roll.quarantine().is_empty());
        assert_eq!(
            reloaded_roll
                .revision(actor, saved.key)
                .unwrap()
                .reference(),
            saved
        );
        assert!(reloaded_roll.revision(ObjId(11), saved.key).is_err());
        assert_eq!(
            reloaded_sources.read_source(source).unwrap(),
            transaction_draft().source
        );

        // Real module hydration: the source owner runs before Roll's loader,
        // but on_all_loaded audits only after both have hydrated. A corrupt
        // immutable source quarantines its revision without repairing disk or
        // granting permission to prune references.
        for corrupt in [false, true] {
            let mut source_docs = docs(NS);
            if corrupt {
                source_docs[0].1["bytes"] = serde_json::json!("tampered");
            }
            let mut boot = std::collections::HashMap::new();
            boot.insert(
                SmolStr::new_static(NS),
                mush_core::ModuleBootData {
                    kv: source_docs.clone(),
                    ..Default::default()
                },
            );
            boot.insert(
                SmolStr::new_static(mush_roll::NS),
                mush_core::ModuleBootData {
                    kv: docs(mush_roll::NS),
                    ..Default::default()
                },
            );
            let mut engine = transaction_engine(true);
            engine
                .modules_loaded(boot, &mut Outbox::new(), &mut mush_core::Effects::default())
                .unwrap();
            let roll = engine.world.module_state::<mush_roll::RollStore>().unwrap();
            assert_eq!(roll.quarantine().is_empty(), !corrupt);
            assert_eq!(roll.revision(actor, saved.key).is_ok(), !corrupt);
            if corrupt {
                assert!(roll.prepare_prune(128).is_err());
            }
            assert!(
                engine.take_module_ops().is_empty(),
                "boot must not rewrite either owner"
            );
            let store = engine.world.module_state::<TacsimStore>().unwrap();
            assert!(store.mind_source_keys.contains(source.as_bytes()));
            assert_eq!(store.has_mind_source(source.as_bytes()), !corrupt);
        }
    }

    #[test]
    fn retained_api4_source_survives_actual_cold_audit_without_new_save_authority() {
        use mush_roll::RevisionAdapter as _;
        const OLD_ABI: &str = "rhai=1.26.0;api=4;stock=2";
        const SOURCE: &str = "const MIND = #{name: \"Retained\", api: 4, class: \"discretion\", role: \"combat\", core: \"I\"}; fn think(v,c) { this.marker = 73; }";
        assert_ne!(crate::minds::MIND_ABI, OLD_ABI);

        // Build the migration image through the actual Roll/source save union.
        // This test-only previous-ABI adapter validates the known-compatible
        // source in the real current sandbox; it issues no certificate or seal.
        struct PreviousAbiRules;
        impl mush_roll::RevisionAdapter for PreviousAbiRules {
            fn normalize_source(&self, source: &str) -> Result<String, mush_roll::RollError> {
                crate::minds::normalize::normalize_source(source)
            }
            fn validate(
                &self,
                source: &str,
                envelope: &mush_roll::Envelope,
            ) -> Result<(), mush_roll::RollError> {
                check_eq!(envelope.mind_abi, OLD_ABI);
                crate::minds::authoring::validate_retained_registry(source, envelope)
            }
        }
        let actor = ObjId(10);
        let mut draft = transaction_draft();
        draft.source = SOURCE.into();
        draft.envelope.mind_abi = OLD_ABI.into();
        assert!(
            crate::minds::authoring::RevisionRules
                .validate(SOURCE, &draft.envelope)
                .is_err()
        );
        let mut engine = Box::new(transaction_engine(true));
        let mut operations = Vec::new();
        {
            let roll = engine
                .world
                .module_state_mut::<mush_roll::RollStore>()
                .unwrap();
            let change = roll.prepare_create(actor, "retained", draft).unwrap();
            roll.publish_with(change, |batch| operations.extend_from_slice(batch))
                .unwrap();
        }
        let roll = engine.world.module_state::<mush_roll::RollStore>().unwrap();
        let base = roll.mind(actor, "retained").unwrap().base();
        let sources = engine.world.module_state::<TacsimStore>().unwrap();
        assert!(
            roll.prepare_save(
                actor,
                "retained",
                base,
                "",
                None,
                &crate::minds::authoring::RevisionRules,
                sources
            )
            .is_err()
        );
        let prepared = roll
            .prepare_save(
                actor,
                "retained",
                base,
                "",
                None,
                &PreviousAbiRules,
                sources,
            )
            .unwrap();
        let source = prepared.source().digest;
        let (saved, mut out) = publish_transaction(&mut engine, actor, prepared);
        let saved = saved.unwrap();
        let revision_bytes = serde_json::to_vec(
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .unwrap()
                .revision(actor, saved.key)
                .unwrap(),
        )
        .unwrap();
        operations.extend(
            out.take_deferred()
                .into_iter()
                .map(|deferred| match deferred {
                    mush_core::Deferred::Persist(op) => op,
                    _ => panic!("the save only queues durable writes"),
                }),
        );
        let mut disk = BTreeMap::new();
        for operation in operations {
            match operation.op {
                ModuleWrite::JsonSet { key, value } => {
                    disk.insert((operation.ns, key), value);
                }
                _ => panic!("create/save need no removals"),
            }
        }
        drop(engine);
        let docs = |namespace: &str| -> Vec<_> {
            disk.iter()
                .filter(|((ns, _), _)| ns == namespace)
                .map(|((_, key), value)| (key.clone(), value.clone()))
                .collect()
        };
        for corrupt in [false, true] {
            let mut source_docs = docs(NS);
            if corrupt {
                let (_, value) = source_docs.first_mut().expect("actual saved source");
                value["bytes"] = serde_json::json!("tampered");
            }
            let mut boot = std::collections::HashMap::new();
            boot.insert(
                SmolStr::new_static(NS),
                mush_core::ModuleBootData {
                    kv: source_docs,
                    ..Default::default()
                },
            );
            boot.insert(
                SmolStr::new_static(mush_roll::NS),
                mush_core::ModuleBootData {
                    kv: docs(mush_roll::NS),
                    ..Default::default()
                },
            );
            let mut cold = Box::new(transaction_engine(true));
            let mut boot_out = Outbox::new();
            cold.modules_loaded(boot, &mut boot_out, &mut mush_core::Effects::default())
                .unwrap();
            let roll = cold.world.module_state::<mush_roll::RollStore>().unwrap();
            assert_eq!(roll.quarantine().is_empty(), !corrupt);
            assert_eq!(roll.revision(actor, saved.key).is_ok(), !corrupt);
            if corrupt {
                assert!(roll.prepare_prune(128).is_err());
            } else {
                let revision = roll.revision(actor, saved.key).unwrap();
                assert_eq!(serde_json::to_vec(revision).unwrap(), revision_bytes);
                assert_eq!(revision.reference(), saved);
                let sources = cold.world.module_state::<TacsimStore>().unwrap();
                assert_eq!(sources.read_source(source).unwrap(), SOURCE);
                assert!(
                    roll.prepare_save(
                        actor,
                        "retained",
                        roll.mind(actor, "retained").unwrap().base(),
                        "",
                        None,
                        &crate::minds::authoring::RevisionRules,
                        sources
                    )
                    .is_err()
                );
            }
            assert!(
                cold.take_module_ops().is_empty(),
                "boot must not rewrite history"
            );
            assert!(
                boot_out.take_deferred().is_empty(),
                "audit is read-only on disk"
            );
        }
    }

    #[test]
    fn m6_combined_save_stale_roll_or_source_and_missing_authority_leave_both_owners_unchanged() {
        for failure in ["roll", "source", "authority", "actor"] {
            let actor = ObjId(10);
            let mut engine = transaction_engine(failure != "authority");
            create_transaction_draft(&mut engine, actor);
            let prepared = prepare_transaction(&engine, actor);
            let digest = prepared.source().digest;
            if failure == "roll" {
                let roll = engine
                    .world
                    .module_state_mut::<mush_roll::RollStore>()
                    .unwrap();
                let change = roll
                    .prepare_create(actor, "other", transaction_draft())
                    .unwrap();
                roll.publish_with(change, |_| {}).unwrap();
            } else if failure == "source" {
                // A source was independently published between prepare and commit.
                let store = engine.world.module_state_mut::<TacsimStore>().unwrap();
                install_mind_source(store, &transaction_draft().source).unwrap();
            }
            let source_before = engine
                .world
                .module_state::<TacsimStore>()
                .unwrap()
                .mind_sources
                .clone();
            let key_before = engine
                .world
                .module_state::<TacsimStore>()
                .unwrap()
                .mind_source_keys
                .clone();
            let publish_actor = if failure == "actor" { ObjId(11) } else { actor };
            let (result, out) = publish_transaction(&mut engine, publish_actor, prepared);
            assert!(result.is_err(), "{failure}");
            assert!(out.deferred().is_empty(), "{failure}");
            let store = engine.world.module_state::<TacsimStore>().unwrap();
            assert_eq!(store.mind_sources, source_before, "{failure}");
            assert_eq!(store.mind_source_keys, key_before, "{failure}");
            assert_eq!(
                store.has_mind_source(digest.as_bytes()),
                failure == "source"
            );
            assert!(
                engine
                    .world
                    .module_state::<mush_roll::RollStore>()
                    .unwrap()
                    .mind(actor, "test")
                    .unwrap()
                    .latest()
                    .is_none()
            );
        }
    }

    // ---- doc 18 cut IC14 row 36: `load`'s reconnect-notice branches ----

    /// A queue for `who` with one line, as its document.
    fn notice_doc(who: u32) -> serde_json::Value {
        let mut queue = crate::port_tick::NoticeQueue::new(ObjId(who));
        queue.push(1_000, "The Salt Moth has docked.");
        serde_json::to_value(&queue).expect("a queue serializes")
    }

    fn notice_key(who: u32) -> SmolStr {
        SmolStr::new(format!("{}{who}", crate::port_tick::NOTICE_PREFIX))
    }

    /// Load `bad` beside a good queue for #7; `#7`'s queue survives, and no queue is
    /// held for any character but #7.
    fn only_seven_survives(bad: (SmolStr, serde_json::Value)) {
        let kv = vec![(notice_key(7), notice_doc(7)), bad];
        let store = load(&kv, TacsimConfig::default());
        assert_eq!(
            store.notices.keys().copied().collect::<Vec<_>>(),
            vec![ObjId(7)],
            "every other queue intact, the bad one held for nobody"
        );
        assert_eq!(store.notices.get(&ObjId(7)).map(|q| q.lines.len()), Some(1));
    }

    /// Doc 18 cut IC14 row 36: a notice document whose key names no character is
    /// skipped, and nothing else is.
    #[test]
    fn ic14_a_notice_document_with_a_bad_key_is_skipped() {
        only_seven_survives((
            SmolStr::new(format!("{}abc", crate::port_tick::NOTICE_PREFIX)),
            notice_doc(9),
        ));
    }

    /// Doc 18 cut IC14 row 36: the identity rule — a queue whose body names another
    /// character than its key's is dropped.
    #[test]
    fn ic14_a_notice_document_naming_another_character_is_dropped() {
        only_seven_survives((notice_key(8), notice_doc(9)));
    }

    /// Doc 18 cut IC14 row 36: a queue that fails its own check (a wrong schema) is
    /// dropped.
    #[test]
    fn ic14_a_notice_document_failing_its_check_is_dropped() {
        let mut bad = notice_doc(8);
        bad["schema"] = serde_json::json!(99);
        only_seven_survives((notice_key(8), bad));
    }
    /// Doc 20 cut SM.3b: a `tacsim:tv:` projection and a `tacsim:tvlease:` record each load
    /// into their own map (the prefixes differ at the colon); a projection whose body
    /// disagrees with its key, or does not read, is claimed `Unreadable`.
    #[test]
    fn sm3b_the_projection_and_the_lease_record_load_into_their_own_maps() {
        let rec = crate::surface::excursion::TvRec::new(ObjId(5), 1);
        let lease = crate::port::tv::TvLease {
            schema: crate::port::tv::TV_LEASE_SCHEMA,
            lease: 7,
            tv: ObjId(5),
            mark: "OWP-TV-01".to_owned(),
            pilot: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            hold: 3,
            fee_minor: 140_000,
            deposit_minor: 500_000,
            term_secs: 86_400,
            leased_at_ic: 0,
            term_ends_ic: 86_400,
            terms_paid: 1,
            status: crate::port::tv::TvLeaseStatus::Active,
            ended_at_ic: None,
            repossessed: false,
        };
        let docs = vec![
            (
                crate::surface::excursion::tv_rec_key(ObjId(5)),
                serde_json::to_value(&rec).expect("serializes"),
            ),
            (
                crate::port::tv::tv_lease_key(7),
                serde_json::to_value(&lease).expect("serializes"),
            ),
            (
                crate::surface::excursion::tv_rec_key(ObjId(6)),
                serde_json::to_value(&rec).expect("serializes"),
            ),
            (
                crate::surface::excursion::tv_rec_key(ObjId(8)),
                serde_json::json!({ "schema": 1 }),
            ),
        ];
        let store = load(&docs, TacsimConfig::default());
        assert_eq!(
            store.tv_rec(ObjId(5)),
            Some(&crate::surface::excursion::TvRecState::Present(rec))
        );
        assert_eq!(
            store.tv_rec(ObjId(6)),
            Some(&crate::surface::excursion::TvRecState::Unreadable),
            "a body naming another rig"
        );
        assert_eq!(
            store.tv_rec(ObjId(8)),
            Some(&crate::surface::excursion::TvRecState::Unreadable),
            "a body that does not read"
        );
        assert_eq!(
            store.tv_recs.len(),
            3,
            "the lease record is not a projection"
        );
        assert!(matches!(
            store.tv_lease(7),
            Some(crate::port::tv::TvLeaseState::Present(_))
        ));
        assert_eq!(store.tv_leases.len(), 1, "no projection is a lease record");
    }
}
